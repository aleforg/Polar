pub(crate) mod links;
mod search;
pub(crate) mod tag_icons;
pub(crate) mod tags;
pub(crate) use tags::tags;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Ref, RefCell},
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Note {
    pub(crate) id: u64,
    pub(crate) text: String,
    #[serde(skip)]
    pub(crate) tag_cache: RefCell<(String, BTreeSet<String>)>,
    #[serde(skip)]
    pub(crate) thumbnail_cache: RefCell<(String, PathBuf, Option<PathBuf>)>,
    #[serde(skip)]
    pub(crate) search_cache: RefCell<(String, search::Features)>,
    pub(crate) pinned: bool,
    pub(crate) trashed: bool,
    pub(crate) modified: u64,
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(default)]
    pub(crate) read_only: bool,
    #[serde(default)]
    pub(crate) created: Option<u64>,
}

impl Note {
    pub(crate) fn tags_cached(&self) -> bool {
        self.tag_cache.borrow().0 == self.text
    }

    pub(crate) fn tags(&self) -> Ref<'_, BTreeSet<String>> {
        if !self.tags_cached() {
            *self.tag_cache.borrow_mut() = (self.text.clone(), tags(&self.text));
        }
        Ref::map(self.tag_cache.borrow(), |cached| &cached.1)
    }

    pub(crate) fn thumbnail(&self, directory: &Path) -> Option<PathBuf> {
        let mut cached = self.thumbnail_cache.borrow_mut();
        if cached.0 != self.text || cached.1 != directory {
            *cached = (
                self.text.clone(),
                directory.to_owned(),
                crate::attachments::thumbnail(directory, &self.text),
            );
        }
        cached.2.clone()
    }

    pub(crate) fn title(&self) -> &str {
        let title = self.text.lines().next().unwrap_or("").trim();
        let title = title.strip_prefix("# ").unwrap_or(title);
        if title.is_empty() {
            "Una magnifica nuova nota"
        } else {
            title
        }
    }

    pub(crate) fn snippet(&self) -> String {
        let text = self
            .text
            .lines()
            .skip(1)
            .filter(|line| {
                let text = line.trim();
                let only_tag = tags::in_text(text)
                    .next()
                    .is_some_and(|tag| tag.range == (0..text.len()));
                let only_image = text.starts_with("![")
                    && text.ends_with(')')
                    && text.contains("](attachments/image-");
                !only_tag && !only_image
            })
            .map(|line| {
                line.strip_prefix("- [ ] ")
                    .or_else(|| line.strip_prefix("- [x] "))
                    .or_else(|| line.strip_prefix("- [X] "))
                    .unwrap_or(line)
                    .trim_start_matches('#')
                    .trim()
            })
            .filter(|line| !line.is_empty())
            .flat_map(|line| line.chars().chain(std::iter::once(' ')))
            .filter(|ch| !matches!(ch, '*' | '`'))
            .take(240)
            .collect::<String>();
        let text = text.trim_end().to_owned();
        if self.text.trim().is_empty() {
            "Mantieni la calma e scrivi qualcosa".into()
        } else {
            text
        }
    }

    pub(crate) fn can_edit(&self) -> bool {
        !self.trashed && !self.archived && !self.read_only
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Filter {
    Notes,
    Untagged,
    Todo,
    Today,
    Archive,
    Pinned,
    Trash,
    Tag(String),
}

pub(crate) fn all_tags<'a>(notes: impl IntoIterator<Item = &'a Note>) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for note in notes
        .into_iter()
        .filter(|note| !note.trashed && !note.archived)
    {
        tags.extend(note.tags().iter().cloned());
    }
    tags
}

pub(crate) fn visible<'a>(notes: &'a [Note], filter: &Filter, query: &str) -> Vec<&'a Note> {
    let query = search::Search::new(query);
    let today = Local::now().date_naive();
    let mut matches = notes
        .iter()
        .filter(|note| {
            let in_section = match filter {
                Filter::Trash => note.trashed,
                Filter::Archive => note.archived && !note.trashed,
                _ => !note.archived && !note.trashed,
            };
            in_section
                && match filter {
                    Filter::Notes | Filter::Trash | Filter::Archive => true,
                    Filter::Untagged => note.tags().is_empty(),
                    Filter::Todo => note.search_features().todo,
                    Filter::Today => {
                        let timestamp = note.created.unwrap_or(note.modified);
                        i64::try_from(timestamp)
                            .ok()
                            .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
                            .is_some_and(|created| {
                                created.with_timezone(&Local).date_naive() == today
                            })
                    }
                    Filter::Pinned => note.pinned,
                    Filter::Tag(tag) => note.tags().iter().any(|candidate| {
                        candidate == tag
                            || candidate
                                .strip_prefix(tag)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                    }),
                }
                && query.matches(note)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|note| std::cmp::Reverse((note.pinned, note.modified, note.id)));
    matches
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn initial_notes() -> Vec<Note> {
    [
        ("Appunti di viaggio\n\n## Itinerario\nUn paragrafo con **grassetto** e *corsivo*.\n\n- [ ] Preparare lo zaino\n- [x] Controllare gli orari\n\n#viaggi/idee", true),
        ("Idee per il fine settimana\n\nUna passeggiata in collina e una sosta in libreria.\n\n#viaggi", false),
        ("Letture da esplorare\n\nRaccogli qui i libri e gli articoli da leggere.\n\n#letture", false),
        ("Organizza le tue note\n\nScrivi un tag nel testo per ritrovarlo nella barra laterale.", false),
        ("Liste e promemoria\n\n- [ ] Aggiornare la lista\n- [x] Scrivere il primo appunto\n\n#idee", false),
        ("Un posto per le idee\n\nQuesta raccolta contiene solo note di esempio.", false),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (text, pinned))| Note {
        id: index as u64 + 1,
        text: text.into(),
        tag_cache: RefCell::default(),
        thumbnail_cache: RefCell::default(),
        search_cache: RefCell::default(),
        pinned,
        trashed: false,
        archived: false,
        read_only: false,
        created: None,
        modified: 1_763_660_400 - index as u64 * 86_400,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_cache_tracks_edits_and_is_not_persisted() {
        let mut note = initial_notes().remove(0);
        note.text = "#alpha".into();
        assert_eq!(*note.tags(), BTreeSet::from(["alpha".into()]));
        note.text = "#bravo".into();
        assert_eq!(*note.tags(), BTreeSet::from(["bravo".into()]));
        note.text.clear();
        assert!(note.tags().is_empty());
        let json = serde_json::to_value(&note).unwrap();
        assert!(json.get("tag_cache").is_none());
        let loaded: Note = serde_json::from_value(json).unwrap();
        assert!(loaded.tags().is_empty());
    }

    #[test]
    fn filtered_search_updates_after_typing_and_deletion() {
        let mut notes = initial_notes();
        notes.truncate(1);
        for (source, query) in [
            ("- [ ] Da fare", "@todo"),
            ("- [x] Fatto", "@done"),
            ("`codice`", "@code"),
            ("![Foto](a.png)", "@images"),
        ] {
            notes[0].text = source.into();
            assert_eq!(visible(&notes, &Filter::Notes, query).len(), 1);
            notes[0].text.clear();
            assert!(visible(&notes, &Filter::Notes, query).is_empty());
        }
    }

    #[test]
    fn thumbnail_cache_tracks_edits_and_attachment_directory() {
        let directory = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("image-a.png"), b"fixture").unwrap();
        std::fs::write(other.path().join("image-a.png"), b"fixture").unwrap();
        let mut note = initial_notes().remove(0);
        note.text = "![Immagine](attachments/image-a.png)".into();
        assert_eq!(
            note.thumbnail(directory.path()),
            Some(directory.path().join("image-a.png"))
        );
        assert_eq!(
            note.thumbnail(other.path()),
            Some(other.path().join("image-a.png"))
        );
        note.text = "Nessuna immagine".into();
        assert!(note.thumbnail(directory.path()).is_none());
        assert!(
            serde_json::to_value(&note)
                .unwrap()
                .get("thumbnail_cache")
                .is_none()
        );
    }

    #[test]
    fn snippet_is_bounded_without_truncating_the_note() {
        let mut note = initial_notes().remove(0);
        note.text = format!("# Titolo\n\n{}", "Testo 🐻 **formattato**. ".repeat(10000));
        let original = note.text.clone();
        let snippet = note.snippet();
        assert!(snippet.chars().count() <= 240);
        assert!(snippet.starts_with("Testo 🐻 formattato."));
        assert_eq!(note.text, original);
    }

    #[test]
    fn searches_within_sections_and_preserves_archived_state_after_trash() {
        let mut notes = initial_notes();
        assert_eq!(
            visible(
                &notes,
                &Filter::Notes,
                r#"@todo "Appunti di viaggio" -collina"#
            )[0]
            .id,
            1
        );
        assert!(visible(&notes, &Filter::Notes, "@title itinerario").is_empty());
        assert_eq!(visible(&notes, &Filter::Notes, "ITINERARIO ORARI")[0].id, 1);
        notes[0].archived = true;
        assert!(visible(&notes, &Filter::Notes, "itinerario").is_empty());
        assert_eq!(
            visible(&notes, &Filter::Archive, "@pinned itinerario")[0].id,
            1
        );
        notes[0].trashed = true;
        assert!(visible(&notes, &Filter::Archive, "").is_empty());
        assert_eq!(visible(&notes, &Filter::Trash, "@todo")[0].id, 1);
        notes[0].trashed = false;
        assert_eq!(visible(&notes, &Filter::Archive, "itinerario")[0].id, 1);
    }

    #[test]
    fn nested_tags_search_and_pinning() {
        let mut notes = initial_notes();
        notes[1].modified = now();
        assert_eq!(
            tags("## Titolo #Viaggi/Idee, #idee"),
            BTreeSet::from(["idee".into(), "viaggi/idee".into()])
        );
        let travel = Filter::Tag("viaggi".into());
        assert_eq!(
            visible(&notes, &travel, "")
                .iter()
                .map(|note| note.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(visible(&notes, &travel, "COLLINA")[0].id, 2);
        notes[0].trashed = true;
        assert_eq!(visible(&notes, &Filter::Trash, "")[0].id, 1);
        assert_eq!(visible(&notes, &travel, "").len(), 1);
    }
}

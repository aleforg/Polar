pub(crate) mod links;
mod search;
pub(crate) mod tag_icons;
pub(crate) mod tags;
pub(crate) use tags::tags;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Note {
    pub(crate) id: u64,
    text: Arc<String>,
    #[serde(skip)]
    tag_cache: OnceCell<BTreeSet<String>>,
    #[serde(skip)]
    thumbnail_cache: RefCell<Option<(PathBuf, Option<PathBuf>)>>,
    #[serde(skip)]
    search_cache: OnceCell<search::Features>,
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
    pub(crate) fn new(id: u64, text: String, modified: u64) -> Self {
        Self {
            id,
            text: Arc::new(text),
            tag_cache: OnceCell::new(),
            thumbnail_cache: RefCell::default(),
            search_cache: OnceCell::new(),
            pinned: false,
            trashed: false,
            modified,
            archived: false,
            read_only: false,
            created: None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn text_snapshot(&self) -> Arc<String> {
        Arc::clone(&self.text)
    }

    pub(crate) fn set_text(&mut self, text: String) {
        self.text = Arc::new(text);
        self.invalidate_metadata();
    }

    pub(crate) fn append_attachment(&mut self, markdown: &str) {
        let text = Arc::make_mut(&mut self.text);
        text.push_str("\n\n");
        text.push_str(markdown);
        self.invalidate_metadata();
    }

    fn invalidate_metadata(&mut self) {
        self.tag_cache.take();
        self.thumbnail_cache.get_mut().take();
        self.search_cache.take();
    }

    pub(crate) fn prime_tags(&self, tags: BTreeSet<String>) {
        let _ = self.tag_cache.set(tags);
    }

    pub(crate) fn tags_cached(&self) -> bool {
        self.tag_cache.get().is_some()
    }

    pub(crate) fn tags(&self) -> &BTreeSet<String> {
        self.tag_cache.get_or_init(|| tags(self.text()))
    }

    pub(crate) fn thumbnail(&self, directory: &Path) -> Option<PathBuf> {
        let mut cached = self.thumbnail_cache.borrow_mut();
        if let Some((cached_directory, path)) = &*cached
            && cached_directory == directory
        {
            return path.clone();
        }
        let path = crate::attachments::thumbnail(directory, self.text());
        *cached = Some((directory.to_owned(), path.clone()));
        path
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
        let mut text = self
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
        text.truncate(text.trim_end().len());
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

pub(crate) fn all_tags<'a>(notes: impl IntoIterator<Item = &'a Note>) -> BTreeSet<&'a str> {
    let mut tags = BTreeSet::new();
    for note in notes
        .into_iter()
        .filter(|note| !note.trashed && !note.archived)
    {
        tags.extend(note.tags().iter().map(String::as_str));
    }
    tags
}

pub(crate) fn visible_indices(notes: &[Note], filter: &Filter, query: &str) -> Vec<usize> {
    let query = search::Search::new(query);
    let today = (*filter == Filter::Today).then(|| Local::now().date_naive());
    let mut matches = notes
        .iter()
        .enumerate()
        .filter(|(_, note)| {
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
                                Some(created.with_timezone(&Local).date_naive()) == today
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
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|&index| {
        let note = &notes[index];
        std::cmp::Reverse((note.pinned, note.modified, note.id))
    });
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
    .map(|(index, (text, pinned))| {
        let mut note = Note::new(
            index as u64 + 1,
            text.into(),
            1_763_660_400 - index as u64 * 86_400,
        );
        note.pinned = pinned;
        note
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible<'a>(notes: &'a [Note], filter: &Filter, query: &str) -> Vec<&'a Note> {
        visible_indices(notes, filter, query)
            .into_iter()
            .map(|index| &notes[index])
            .collect()
    }

    #[test]
    fn tag_cache_tracks_edits_and_is_not_persisted() {
        let mut note = initial_notes().remove(0);
        note.set_text("#alpha".into());
        assert_eq!(*note.tags(), BTreeSet::from(["alpha".into()]));
        note.set_text("#bravo".into());
        assert_eq!(*note.tags(), BTreeSet::from(["bravo".into()]));
        note.set_text(String::new());
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
            notes[0].set_text(source.into());
            assert_eq!(visible(&notes, &Filter::Notes, query).len(), 1);
            notes[0].set_text(String::new());
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
        note.set_text("![Immagine](attachments/image-a.png)".into());
        assert_eq!(
            note.thumbnail(directory.path()),
            Some(directory.path().join("image-a.png"))
        );
        assert_eq!(
            note.thumbnail(other.path()),
            Some(other.path().join("image-a.png"))
        );
        note.set_text("Nessuna immagine".into());
        assert!(note.thumbnail(directory.path()).is_none());
        assert!(
            serde_json::to_value(&note)
                .unwrap()
                .get("thumbnail_cache")
                .is_none()
        );
    }

    #[test]
    fn appending_an_attachment_invalidates_cached_metadata_and_keeps_the_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image-a.png");
        std::fs::write(&path, b"fixture").unwrap();
        let mut notes = [Note::new(1, "Titolo\n\n#prima".into(), 0)];
        let snapshot = notes[0].text_snapshot();
        assert!(notes[0].tags().contains("prima"));
        assert!(notes[0].thumbnail(directory.path()).is_none());
        assert!(visible(&notes, &Filter::Notes, "@images").is_empty());

        notes[0].append_attachment("![Foto](attachments/image-a.png)\n\n#dopo");

        assert_eq!(snapshot.as_str(), "Titolo\n\n#prima");
        assert!(notes[0].tags().contains("dopo"));
        assert_eq!(notes[0].thumbnail(directory.path()), Some(path));
        assert_eq!(visible(&notes, &Filter::Notes, "@images").len(), 1);
    }

    #[test]
    fn snippet_is_bounded_without_truncating_the_note() {
        let mut note = initial_notes().remove(0);
        note.set_text(format!(
            "# Titolo\n\n{}",
            "Testo 🐻 **formattato**. ".repeat(10000)
        ));
        let original = note.text().to_owned();
        let snippet = note.snippet();
        assert!(snippet.chars().count() <= 240);
        assert!(snippet.starts_with("Testo 🐻 formattato."));
        assert_eq!(note.text(), original);
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

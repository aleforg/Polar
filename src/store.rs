use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::notes::{Note, initial_notes, tag_icons::TagIcons};

#[derive(Deserialize)]
pub(crate) struct Archive {
    version: u32,
    pub(crate) notes: Vec<Note>,
    #[serde(default)]
    pub(crate) tag_icons: TagIcons,
}

#[derive(Serialize)]
struct ArchiveRef<'a> {
    version: u32,
    notes: &'a [Note],
    #[serde(skip_serializing_if = "TagIcons::is_empty")]
    tag_icons: &'a TagIcons,
}

pub(crate) struct Store {
    path: PathBuf,
    _lock: File,
}

impl Store {
    pub(crate) fn open(directory: &Path) -> io::Result<(Self, Archive)> {
        fs::create_dir_all(directory)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("notes.lock"))?;
        lock.try_lock()?;
        let store = Self {
            path: directory.join("notes.json"),
            _lock: lock,
        };
        let archive = match fs::read(&store.path) {
            Ok(bytes) => {
                let archive: Archive = serde_json::from_slice(&bytes)?;
                let mut ids = BTreeSet::new();
                if !matches!(archive.version, 1 | 2)
                    || archive
                        .notes
                        .iter()
                        .any(|note| note.id == 0 || note.id == u64::MAX || !ids.insert(note.id))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Archivio Polar non valido",
                    ));
                }
                archive
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Archive {
                version: 2,
                notes: initial_notes(),
                tag_icons: TagIcons::new(),
            },
            Err(error) => return Err(error),
        };
        Ok((store, archive))
    }

    pub(crate) fn attachments(&self) -> PathBuf {
        self.path.with_file_name("attachments")
    }

    pub(crate) fn save(&self, notes: &[Note], tag_icons: &TagIcons) -> io::Result<()> {
        let _timing = crate::performance::Sample::start("save");
        let pending = self.path.with_extension("json.pending");
        let mut file = BufWriter::new(File::create(&pending)?);
        serde_json::to_writer(
            &mut file,
            &ArchiveRef {
                version: 2,
                notes,
                tag_icons,
            },
        )?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.get_ref().sync_all()?;
        fs::rename(pending, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_legacy_archives_and_saves_new_note_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.json");
        fs::write(&path, br#"{"version":1,"notes":[{"id":8,"text":"Legacy","pinned":true,"trashed":false,"modified":42}]}"#).unwrap();
        let (
            store,
            Archive {
                mut notes,
                mut tag_icons,
                ..
            },
        ) = Store::open(directory.path()).unwrap();
        assert!(tag_icons.is_empty());
        tag_icons.insert("viaggi".into(), crate::notes::tag_icons::TagIcon::Book);
        assert!(!notes[0].archived && !notes[0].read_only);
        assert_eq!(notes[0].created, None);
        assert_eq!(notes[0].modified, 42);
        notes[0].archived = true;
        notes[0].read_only = true;
        notes[0].trashed = true;
        store.save(&notes, &tag_icons).unwrap();
        let version: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(version["version"], 2);
        drop(store);
        let (_, saved) = Store::open(directory.path()).unwrap();
        assert_eq!(saved.tag_icons, tag_icons);
        let saved = saved.notes;
        assert!(saved[0].archived && saved[0].trashed && saved[0].pinned && saved[0].read_only);
        assert_eq!(saved[0].text, "Legacy");
    }

    #[test]
    fn save_reopen_lock_and_corruption() {
        let path = std::env::temp_dir().join(format!("polar-test-{}", std::process::id()));
        let (
            store,
            Archive {
                mut notes,
                tag_icons,
                ..
            },
        ) = Store::open(&path).unwrap();
        notes[0].text = "Test di persistenza 🐻".into();
        store.save(&notes, &tag_icons).unwrap();
        assert!(Store::open(&path).is_err());
        fs::create_dir(path.join("notes.json.pending")).unwrap();
        assert!(store.save(&[], &tag_icons).is_err());
        fs::remove_dir(path.join("notes.json.pending")).unwrap();
        drop(store);
        let (store, saved) = Store::open(&path).unwrap();
        assert_eq!(saved.notes[0].text, notes[0].text);
        drop(store);
        fs::write(path.join("notes.json"), b"invalid").unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(fs::read(path.join("notes.json")).unwrap(), b"invalid");
        fs::remove_dir_all(path).unwrap();
    }
}

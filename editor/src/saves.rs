//! Save slots under `<data folder>/saves/<game key>/slot_<n>.ron`.
//! Resolve `crate::store::data_dir()` after player setup and pass the folder to `SaveStore::new`,
//! or supply another folder. Slots are numbered from zero. Play owns when saves are loaded and written.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clockwork_core::save::SaveData;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub struct SaveStore {
    dir: PathBuf,
}

/// A failed slot reads as empty, with its error retained for the caller to report.
#[derive(Debug)]
pub struct Slots {
    pub slots: Vec<Option<SaveData>>,
    pub errors: Vec<(usize, String)>,
}

impl SaveStore {
    pub fn new(data_dir: &Path, game: &str) -> Self {
        Self {
            dir: data_dir.join("saves").join(game_key(game)),
        }
    }

    fn path(&self, slot: usize) -> PathBuf {
        self.dir.join(format!("slot_{slot}.ron"))
    }

    /// Returns exactly `count` slots, preserving gaps and reporting every failed read.
    pub fn list(&self, count: usize) -> Slots {
        let mut slots = Vec::with_capacity(count);
        let mut errors = Vec::new();
        for slot in 0..count {
            match self.load(slot) {
                Ok(save) => slots.push(save),
                Err(error) => {
                    slots.push(None);
                    errors.push((slot, error));
                }
            }
        }
        Slots { slots, errors }
    }

    /// Missing files are empty. Corrupt or unreadable files return an error and stay on disk;
    /// the caller can present an empty slot alongside that error (as `list` does).
    pub fn load(&self, slot: usize) -> Result<Option<SaveData>, String> {
        let path = self.path(slot);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
        };
        ron::from_str(&text)
            .map(Some)
            .map_err(|error| format!("Corrupt save {}: {error}", path.display()))
    }

    /// Replaces a slot only after its complete RON has been written and synced. A crash may
    /// leave a temporary file, which slot reads ignore, but never a partially written slot.
    pub fn save(&self, slot: usize, save: &SaveData) -> Result<(), String> {
        let path = self.path(slot);
        let text = ron::ser::to_string_pretty(save, Default::default())
            .map_err(|error| format!("Could not serialize {}: {error}", path.display()))?;
        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("Could not create {}: {error}", self.dir.display()))?;
        // Each writer owns its temporary file. Never truncate another writer's file or a
        // leftover from an interrupted save, including after a process id is reused.
        let (temp, mut file) = loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let temp = self
                .dir
                .join(format!(".slot_{slot}.{}.{id}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => break (temp, file),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("Could not create {}: {error}", temp.display()));
                }
            }
        };
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let result = written.and_then(|()| fs::rename(&temp, &path));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(|error| format!("Could not save {}: {error}", path.display()))
    }

    /// Erasing an already empty slot succeeds. Only an explicit erase removes its file.
    // The file select's erase and copy buttons come with their screens.
    #[allow(dead_code)]
    pub fn erase(&self, slot: usize) -> Result<(), String> {
        let path = self.path(slot);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not erase {}: {error}", path.display())),
        }
    }

    /// Copies a populated slot, replacing the destination atomically. An empty or corrupt
    /// source is an error and leaves the destination unchanged.
    // The file select's erase and copy buttons come with their screens.
    #[allow(dead_code)]
    pub fn copy(&self, from: usize, to: usize) -> Result<(), String> {
        let save = self
            .load(from)?
            .ok_or_else(|| format!("Save slot is empty: {}", self.path(from).display()))?;
        self.save(to, &save)
    }
}

/// Escape UTF-8 bytes rather than replacing punctuation, so different names stay distinct.
/// Uppercase is escaped too for case-insensitive filesystems; the prefix avoids device names.
fn game_key(name: &str) -> String {
    let mut key = String::from("game-");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in name.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_') {
            key.push(char::from(byte));
        } else {
            key.push('~');
            key.push(char::from(HEX[usize::from(byte >> 4)]));
            key.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            loop {
                let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("clockwork-saves-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("Could not create test folder: {error}"),
                }
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn progress() -> SaveData {
        SaveData {
            items: [("key".into(), 2)].into(),
            flags: ["door open".into()].into(),
            gone: ["boss".into()].into(),
            start: Some("checkpoint".into()),
            acts_done: [("course".into(), [1, 3].into())].into(),
            seconds: 123.5,
        }
    }

    #[test]
    fn round_trip_and_replace() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "adventure");
        let save = progress();
        store.save(0, &save).unwrap();
        let reopened = SaveStore::new(&dir.0, "adventure");
        assert_eq!(reopened.load(0).unwrap(), Some(save));
        assert!(dir.0.join("saves/game-adventure/slot_0.ron").is_file());
        store.save(0, &SaveData::default()).unwrap();
        assert_eq!(reopened.load(0).unwrap(), Some(SaveData::default()));
        assert_eq!(fs::read_dir(&store.dir).unwrap().count(), 1);
    }

    #[test]
    fn list_with_gaps() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "game");
        assert_eq!(store.list(3).slots, vec![None; 3]);
        assert!(!store.dir.exists());
        store.save(1, &progress()).unwrap();
        store.save(3, &SaveData::default()).unwrap();
        store.save(8, &progress()).unwrap();
        let list = store.list(5);
        assert_eq!(
            list.slots,
            vec![
                None,
                Some(progress()),
                None,
                Some(SaveData::default()),
                None
            ]
        );
        assert!(list.errors.is_empty());
        assert!(store.list(0).slots.is_empty());
    }

    #[test]
    fn erase_is_idempotent_and_only_removes_its_slot() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "game");
        store.erase(0).unwrap();
        store.save(0, &progress()).unwrap();
        store.save(1, &progress()).unwrap();
        store.erase(0).unwrap();
        store.erase(0).unwrap();
        assert_eq!(store.load(0).unwrap(), None);
        assert_eq!(store.load(1).unwrap(), Some(progress()));
    }

    #[test]
    fn copy_replaces_destination_and_preserves_source() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "game");
        store.save(0, &progress()).unwrap();
        store.copy(0, 1).unwrap();
        store.save(2, &SaveData::default()).unwrap();
        store.copy(0, 2).unwrap();
        store.copy(0, 0).unwrap();
        assert_eq!(store.list(3).slots, vec![Some(progress()); 3]);
        assert!(store.copy(9, 1).is_err());
        assert_eq!(store.load(1).unwrap(), Some(progress()));
    }

    #[test]
    fn corrupt_file_is_reported_as_empty_and_preserved() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "game");
        store.save(0, &progress()).unwrap();
        let corrupt = "(items: definitely not RON";
        fs::write(store.path(1), corrupt).unwrap();
        let error = store.load(1).unwrap_err();
        assert!(error.contains("Corrupt save"));
        assert!(error.contains("slot_1.ron"));
        let list = store.list(3);
        assert_eq!(list.slots, vec![Some(progress()), None, None]);
        assert_eq!(list.errors, vec![(1, error)]);
        assert!(store.copy(1, 0).is_err());
        assert_eq!(store.load(0).unwrap(), Some(progress()));
        assert_eq!(fs::read_to_string(store.path(1)).unwrap(), corrupt);
        store.erase(1).unwrap();
        assert_eq!(store.load(1).unwrap(), None);
    }

    #[test]
    fn odd_game_names_stay_in_distinct_safe_folders() {
        let dir = TestDir::new();
        let names = [
            "My game / ../other\\save. ",
            "",
            ".",
            "..",
            "CON",
            "con",
            "a/b",
            "a_b",
            "a~2fb",
            "GAME",
            "game",
            "Pokémon: 星?*<>|\"",
        ];
        let mut keys = std::collections::BTreeSet::new();
        for (slot, name) in names.iter().enumerate() {
            let key = game_key(name);
            assert!(key.bytes().all(|b| b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'-' | b'_' | b'~')));
            assert!(keys.insert(key));
            let store = SaveStore::new(&dir.0, name);
            assert_eq!(store.dir.parent(), Some(dir.0.join("saves").as_path()));
            store.save(slot, &progress()).unwrap();
            assert_eq!(
                SaveStore::new(&dir.0, name).load(slot).unwrap(),
                Some(progress())
            );
        }
        assert_eq!(
            fs::read_dir(dir.0.join("saves")).unwrap().count(),
            names.len()
        );
    }

    #[test]
    fn io_errors_are_values_and_failed_rename_cleans_up_temp() {
        let dir = TestDir::new();
        let blocked = dir.0.join("file");
        fs::write(&blocked, "not a folder").unwrap();
        assert!(
            SaveStore::new(&blocked, "game")
                .save(0, &progress())
                .is_err()
        );

        let store = SaveStore::new(&dir.0, "game");
        fs::create_dir_all(store.path(0)).unwrap();
        fs::write(store.path(0).join("keep"), "untouched").unwrap();
        assert!(store.load(0).is_err());
        assert!(store.save(0, &progress()).is_err());
        assert!(store.erase(0).is_err());
        assert_eq!(
            fs::read_to_string(store.path(0).join("keep")).unwrap(),
            "untouched"
        );
        assert_eq!(fs::read_dir(&store.dir).unwrap().count(), 1);
    }

    #[test]
    fn interrupted_temp_does_not_hide_or_replace_a_slot() {
        let dir = TestDir::new();
        let store = SaveStore::new(&dir.0, "game");
        store.save(0, &progress()).unwrap();
        let temp = store.dir.join(".slot_0.interrupted.tmp");
        fs::write(&temp, "partial RON").unwrap();
        assert_eq!(store.list(2).slots, vec![Some(progress()), None]);
        store.save(0, &SaveData::default()).unwrap();
        assert_eq!(store.load(0).unwrap(), Some(SaveData::default()));
        assert_eq!(fs::read_to_string(temp).unwrap(), "partial RON");
    }
}

//! Writing Clockwork's own files safely: a save never leaves half a file behind, and a file
//! that can't be read is set aside instead of being overwritten by a fresh default.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes `contents` to `path` through a temporary file in the same folder and a rename, so
/// readers (and a crash mid-write) see either the old file or the new one, never a mix.
/// Creates the parent folder if needed.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Each writer owns its temporary file, even with several writers in one folder.
    let (temp, mut file) = loop {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".{name}.{}.{id}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let written = file
        .write_all(contents.as_ref())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Renames a file that exists but could not be read or parsed to
/// `<name>.unreadable-<seconds>`, so starting over with defaults doesn't destroy it.
/// Returns where it went, or `None` if there was nothing to move or the move failed.
pub fn set_aside(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mut target = path.with_file_name(format!("{name}.unreadable-{seconds}"));
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{name}.unreadable-{seconds}-{n}"));
        n += 1;
    }
    fs::rename(path, &target).ok()?;
    Some(target)
}

/// Reads and parses a RON file. A missing file is `Ok(None)`. A file that exists but can't be
/// read or parsed is set aside (see [`set_aside`]) and reported as `Err`, with where it went.
pub fn load_ron<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    ron::from_str(&text).map(Some).map_err(|error| {
        let kept = match set_aside(path) {
            Some(kept) => format!("it was kept as {}", kept.display()),
            None => "it could not be moved aside".into(),
        };
        format!("Could not parse {} ({error}); {kept}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clockwork-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = temp_dir("atomic");
        let path = dir.join("nested").join("a.ron");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.ron"]);
    }

    #[test]
    fn unreadable_ron_is_set_aside_not_lost() {
        let dir = temp_dir("aside");
        let path = dir.join("library.ron");
        assert_eq!(load_ron::<Vec<u32>>(&path), Ok(None));
        fs::write(&path, "[1, 2").unwrap();
        let error = load_ron::<Vec<u32>>(&path).unwrap_err();
        assert!(error.contains("kept as"), "{error}");
        assert!(!path.exists());
        let kept: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(fs::read_to_string(&kept[0]).unwrap(), "[1, 2");
        write_atomic(&path, "[3]").unwrap();
        assert_eq!(load_ron::<Vec<u32>>(&path), Ok(Some(vec![3])));
    }
}

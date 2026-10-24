//! Importing games in the background: files are read and hashed on a worker thread (a disc
//! image can take seconds), folders are searched for game files there too, and what was read
//! is added to the Library a frame later (see [`crate::state::EditorState::poll_imports`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clockwork_library::{Identifier, LibraryEntry, scan_folder};

/// What was read: the file, and its entry or why it couldn't be imported.
pub type Imported = (PathBuf, Result<LibraryEntry, String>);

#[derive(Default)]
pub struct Imports {
    shared: Arc<Mutex<Queue>>,
}

#[derive(Default)]
struct Queue {
    /// Files and folders still to read.
    waiting: VecDeque<PathBuf>,
    /// Read, not yet taken.
    read: Vec<Imported>,
    /// How many files and folders this run of imports has had, and how many are done.
    total: usize,
    done: usize,
    /// Whether a worker is reading.
    running: bool,
}

impl Imports {
    /// Reads these files (and the game files in these folders), after any being read now.
    pub fn queue(&mut self, identifier: Identifier, paths: impl IntoIterator<Item = PathBuf>) {
        let mut queue = self.shared.lock().unwrap();
        let before = queue.waiting.len();
        queue.waiting.extend(paths);
        queue.total += queue.waiting.len() - before;
        if queue.running || queue.waiting.is_empty() {
            return;
        }
        queue.running = true;
        drop(queue);
        let shared = self.shared.clone();
        std::thread::spawn(move || work(&shared, &identifier));
    }

    /// What was read since the last call.
    pub fn take(&mut self) -> Vec<Imported> {
        let mut queue = self.shared.lock().unwrap();
        if !queue.running && queue.waiting.is_empty() {
            // The run is over once everything read is taken.
            queue.total = 0;
            queue.done = 0;
        }
        std::mem::take(&mut queue.read)
    }

    /// (done, total) while importing.
    pub fn progress(&self) -> Option<(usize, usize)> {
        let queue = self.shared.lock().unwrap();
        queue.running.then_some((queue.done, queue.total))
    }

    /// Drops what is still waiting; the file being read finishes.
    pub fn cancel(&mut self) {
        let mut queue = self.shared.lock().unwrap();
        let dropped = queue.waiting.len();
        queue.waiting.clear();
        queue.total -= dropped;
    }
}

fn work(shared: &Mutex<Queue>, identifier: &Identifier) {
    loop {
        let path = {
            let mut queue = shared.lock().unwrap();
            match queue.waiting.pop_front() {
                Some(path) => path,
                None => {
                    queue.running = false;
                    return;
                }
            }
        };
        if path.is_dir() {
            let found = scan_folder(&path);
            let mut queue = shared.lock().unwrap();
            queue.total += found.len();
            queue.done += 1;
            // Its files next, before what was queued after the folder.
            for file in found.into_iter().rev() {
                queue.waiting.push_front(file);
            }
            continue;
        }
        let result = identifier.read(&path).map_err(|e| e.to_string());
        let mut queue = shared.lock().unwrap();
        queue.read.push((path, result));
        queue.done += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_library::Library;

    #[test]
    fn folders_and_files_are_read_on_a_worker_and_taken_after() {
        let dir = std::env::temp_dir().join(format!("clockwork-imports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let mut rom = vec![0u8; 0xc0];
        rom[0xa0..0xa4].copy_from_slice(b"TEST");
        rom[0xb2] = 0x96;
        rom[0xbd] = rom[0xa0..0xbd]
            .iter()
            .fold(0u8, |sum, b| sum.wrapping_sub(*b))
            .wrapping_sub(0x19);
        std::fs::write(dir.join("sub/game.gba"), &rom).unwrap();
        std::fs::write(dir.join("broken.gba"), b"not a game").unwrap();

        let library = Library::new().with_platforms(clockwork_platforms::all());
        let mut imports = Imports::default();
        imports.queue(library.identifier(), [dir.clone()]);
        let mut read = Vec::new();
        for _ in 0..500 {
            read.extend(imports.take());
            if imports.progress().is_none() {
                read.extend(imports.take());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        read.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(read.len(), 2);
        assert!(read[0].1.is_err(), "broken.gba isn't a game");
        assert_eq!(read[1].1.as_ref().unwrap().info.title, "TEST");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

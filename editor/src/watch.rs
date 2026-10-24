//! Noticing plugin files that change on disk (the agent writing a recipe, a plugin copied in),
//! as the system reports them rather than by reading every folder again and again.

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        mpsc::{Receiver, channel},
    },
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// The files of a plugin folder whose changes matter.
const PLUGIN_FILES: [&str; 3] = ["clockwork.toml", "recipe.ron", "compose.ron"];

/// How long the files must be quiet before the plugins are read again, in seconds: an agent
/// writing several files, or a folder being copied in, is read once it is done.
const SETTLE: f64 = 0.3;

/// Watches the plugin folders for changes to plugins.
pub struct PluginWatcher {
    /// `None` when the system can't watch them; plugins are then looked over every few seconds.
    watcher: Option<(
        RecommendedWatcher,
        Mutex<Receiver<notify::Result<notify::Event>>>,
    )>,
    /// The folders being watched: those of the ones asked for that exist.
    dirs: Vec<PathBuf>,
    /// When the last change that matters came, until the plugins are read again.
    changed_at: Option<f64>,
}

impl PluginWatcher {
    /// Watches `dirs` (the ones that exist).
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        let dirs = existing(dirs);
        let (sender, events) = channel();
        let watcher = notify::recommended_watcher(sender)
            .ok()
            .and_then(|mut watcher| {
                let mut any = false;
                for dir in &dirs {
                    any |= watcher.watch(dir, RecursiveMode::Recursive).is_ok();
                }
                any.then_some((watcher, Mutex::new(events)))
            });
        Self {
            watcher,
            dirs,
            changed_at: None,
        }
    }

    /// Whether changes are reported by the system (or must be looked for).
    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Whether the folders it watches are no longer those of `dirs` that exist (a content
    /// folder was added or removed), so it must be made again.
    pub fn is_stale(&self, dirs: Vec<PathBuf>) -> bool {
        existing(dirs) != self.dirs
    }

    /// Takes the changes reported since the last call; true once plugins changed and have
    /// been quiet for a moment (`now` in seconds).
    pub fn poll(&mut self, now: f64) -> bool {
        if let Some((_, events)) = &self.watcher
            && let Ok(events) = events.lock()
        {
            for event in events.try_iter().flatten() {
                if matters(&event, &self.dirs) {
                    self.changed_at = Some(now);
                }
            }
        }
        match self.changed_at {
            Some(at) if now - at >= SETTLE => {
                self.changed_at = None;
                true
            }
            _ => false,
        }
    }
}

fn existing(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// Whether an event changes a plugin: one of its files, or a plugin folder added or removed.
fn matters(event: &notify::Event, dirs: &[PathBuf]) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|path| changes_a_plugin(path, dirs))
}

fn changes_a_plugin(path: &Path, dirs: &[PathBuf]) -> bool {
    let Some(inside) = dirs.iter().find_map(|d| path.strip_prefix(d).ok()) else {
        return false;
    };
    let parts: Vec<_> = inside.components().collect();
    match parts.as_slice() {
        // A plugin folder itself.
        [_] => true,
        [_, file] => PLUGIN_FILES.iter().any(|f| file.as_os_str() == *f),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plugin_files_and_folders_matter() {
        let dirs = vec![PathBuf::from("/content/plugins")];
        let at = |p: &str| changes_a_plugin(Path::new(p), &dirs);
        assert!(at("/content/plugins/mario"));
        assert!(at("/content/plugins/mario/recipe.ron"));
        assert!(at("/content/plugins/mario/compose.ron"));
        assert!(!at("/content/plugins/mario/notes.md"));
        assert!(!at("/content/plugins/mario/cache/recipe.ron"));
        assert!(!at("/elsewhere/mario/recipe.ron"));
    }

    #[test]
    fn a_change_is_reported_once_it_settles() {
        let dir = std::env::temp_dir().join(format!("clockwork-watch-{}", std::process::id()));
        let plugin = dir.join("demo");
        std::fs::create_dir_all(&plugin).unwrap();
        let mut watcher = PluginWatcher::new(vec![dir.clone()]);
        if !watcher.is_watching() {
            // Some systems (and sandboxes) can't watch folders; nothing to test there.
            return;
        }
        std::fs::write(plugin.join("recipe.ron"), "(steps: [], outputs: [])").unwrap();
        let mut reported = false;
        for tick in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if watcher.poll(tick as f64 * 0.02) {
                reported = true;
                break;
            }
        }
        std::fs::remove_dir_all(&dir).ok();
        assert!(reported);
    }
}

//! The Library's saved state (`library.ron` in Clockwork's data folder), shared by the editor
//! and the agent's MCP server.
//!
//! Only ROM paths are stored. ROMs are re-read and re-hashed on load, so the stored file never
//! contains game data and can't go stale.

use std::collections::BTreeMap;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub roms: Vec<PathBuf>,
    /// Look up games online: their full names, and box art for games without a cover.
    #[serde(default = "yes")]
    pub fetch_box_art: bool,
    /// No-Intro names of identified games, by SHA-1.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Decompilation repositories linked to games, by SHA-1.
    #[serde(default)]
    pub decomps: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            roms: Vec::new(),
            fetch_box_art: true,
            names: BTreeMap::new(),
            decomps: BTreeMap::new(),
        }
    }
}

/// Clockwork's per-user data folder. `CLOCKWORK_DATA_DIR` overrides it, e.g. for testing.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLOCKWORK_DATA_DIR") {
        return Some(dir.into());
    }
    ProjectDirs::from("", "", "Clockwork").map(|dirs| dirs.data_dir().to_path_buf())
}

fn store_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("library.ron"))
}

pub fn load() -> Stored {
    load_reporting().0
}

/// The saved Library, and why it couldn't be read if it couldn't. An unreadable file is set
/// aside rather than overwritten by the next save (see [`crate::files::load_ron`]).
pub fn load_reporting() -> (Stored, Option<String>) {
    let Some(path) = store_path() else {
        return (Stored::default(), None);
    };
    match crate::files::load_ron(&path) {
        Ok(stored) => (stored.unwrap_or_default(), None),
        Err(problem) => (Stored::default(), Some(problem)),
    }
}

pub fn save(stored: &Stored) -> Result<(), String> {
    let path = store_path().ok_or("no data directory for this user")?;
    let text = ron::ser::to_string_pretty(stored, Default::default()).map_err(|e| e.to_string())?;
    crate::files::write_atomic(&path, text).map_err(|e| e.to_string())
}

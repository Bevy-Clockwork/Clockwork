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
    store_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(stored: &Stored) -> Result<(), String> {
    let path = store_path().ok_or("no data directory for this user")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = ron::ser::to_string_pretty(stored, Default::default()).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

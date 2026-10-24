//! The Library's saved state (`library.ron` in Clockwork's data folder), shared by the editor
//! and the agent's MCP server.
//!
//! One record per game ([`crate::Game`]): where its file is, what was learned from it (its hash
//! and header, so unchanged files aren't re-read on every start; a file whose length or
//! modification time changed is read and hashed again), its full name and its decompilation.
//! A game whose file is gone keeps its record until the user forgets it. The stored file never
//! contains game data. Files written before records (lists of paths, and names, decomps and
//! what was known, each by path or hash) are read into records.

use std::collections::BTreeMap;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::Game;

#[derive(Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub games: Vec<Game>,
    /// Look up games online: their full names, and box art for games without a cover.
    #[serde(default = "yes")]
    pub fetch_box_art: bool,
    // The format before records, read and folded into `games` (see [`Stored::migrate`]).
    #[serde(default, skip_serializing)]
    roms: Vec<PathBuf>,
    #[serde(default, skip_serializing)]
    names: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    decomps: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    known: Vec<OldKnown>,
}

/// What the format before records remembered about a file.
#[derive(Serialize, Deserialize)]
struct OldKnown {
    path: PathBuf,
    file_len: u64,
    modified: (u64, u32),
    sha1: String,
    size: u64,
    platform: String,
    title: String,
    game_code: String,
}

fn yes() -> bool {
    true
}

impl Default for Stored {
    fn default() -> Self {
        Self::new(Vec::new(), true)
    }
}

impl Stored {
    pub fn new(games: Vec<Game>, fetch_box_art: bool) -> Self {
        Self {
            games,
            fetch_box_art,
            roms: Vec::new(),
            names: BTreeMap::new(),
            decomps: BTreeMap::new(),
            known: Vec::new(),
        }
    }

    /// Folds the format before records into `games`: a record per path, with what was known
    /// about its file and the name and decomp of its hash.
    fn migrate(&mut self) {
        let known = std::mem::take(&mut self.known);
        for path in std::mem::take(&mut self.roms) {
            if self.games.iter().any(|g| g.path == path) {
                continue;
            }
            let mut game = Game::at(path);
            if let Some(k) = known.iter().find(|k| k.path == game.path) {
                game.sha1 = k.sha1.clone();
                game.size = k.size;
                game.platform = k.platform.clone();
                game.title = k.title.clone();
                game.game_code = k.game_code.clone();
                game.file = Some((k.file_len, k.modified));
            }
            self.games.push(game);
        }
        let (names, decomps) = (
            std::mem::take(&mut self.names),
            std::mem::take(&mut self.decomps),
        );
        for game in &mut self.games {
            if game.name.is_none() {
                game.name = names.get(&game.sha1).cloned();
            }
            if game.decomp.is_none() {
                game.decomp = decomps.get(&game.sha1).cloned();
            }
        }
    }

    /// The record of the game with this hash.
    pub fn game(&self, sha1: &str) -> Option<&Game> {
        self.games
            .iter()
            .find(|g| !g.sha1.is_empty() && g.sha1 == sha1)
    }
}

/// Clockwork's per-user data folder. `CLOCKWORK_DATA_DIR` overrides it, e.g. for testing.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLOCKWORK_DATA_DIR") {
        return Some(dir.into());
    }
    ProjectDirs::from("", "", "Clockwork").map(|dirs| dirs.data_dir().to_path_buf())
}

pub(crate) fn store_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("library.ron"))
}

pub fn load() -> Stored {
    load_reporting().0
}

/// The saved Library, and why it couldn't be read if it couldn't. An unreadable file is set
/// aside rather than overwritten by the next save (see [`crate::files::load_ron`]).
pub fn load_reporting() -> (Stored, Option<String>) {
    match store_path() {
        Some(path) => load_at(&path),
        None => (Stored::default(), None),
    }
}

/// The Library saved at `path`, as [`load_reporting`] reads it.
pub fn load_at(path: &std::path::Path) -> (Stored, Option<String>) {
    match crate::files::load_ron::<Stored>(path) {
        Ok(stored) => {
            let mut stored = stored.unwrap_or_default();
            stored.migrate();
            (stored, None)
        }
        Err(problem) => (Stored::default(), Some(problem)),
    }
}

pub fn save(stored: &Stored) -> Result<(), String> {
    let path = store_path().ok_or("no data directory for this user")?;
    let text = ron::ser::to_string_pretty(stored, Default::default()).map_err(|e| e.to_string())?;
    crate::files::write_atomic(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_from_before_records_becomes_one_record_per_game() {
        let text = r#"(
            roms: ["/games/a.gba", "/games/b.gba"],
            names: {"aaaa": "Game A (USA)", "gone": "Removed game"},
            decomps: {"aaaa": "https://example.com/a"},
            known: [(path: "/games/a.gba", file_len: 10, modified: (1, 2), sha1: "aaaa",
                     size: 10, platform: "gba", title: "GAME A", game_code: "AAAE")],
        )"#;
        let mut stored: Stored = ron::from_str(text).unwrap();
        stored.migrate();
        assert_eq!(stored.games.len(), 2);
        let a = &stored.games[0];
        assert_eq!(a.sha1, "aaaa");
        assert_eq!(a.name.as_deref(), Some("Game A (USA)"));
        assert_eq!(a.decomp.as_deref(), Some("https://example.com/a"));
        assert_eq!(a.file, Some((10, (1, 2))));
        // Never read: only its path.
        assert_eq!(stored.games[1], Game::at("/games/b.gba".into()));

        let saved = ron::to_string(&stored).unwrap();
        assert!(!saved.contains("roms") && !saved.contains("known"));
        let again: Stored = ron::from_str(&saved).unwrap();
        assert_eq!(again.games, stored.games);
    }
}

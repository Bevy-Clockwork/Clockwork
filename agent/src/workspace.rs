//! The user's Library as the agent sees it, plus where plugins and knowledge packs live.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use clockwork_library::{Library, LibraryEntry, boxart::display_name, store};
use serde::Serialize;

pub struct Workspace {
    library: Library,
    stored: store::Stored,
    /// ROMs in canonical form, by SHA-1, read on first use.
    roms: HashMap<String, Arc<Vec<u8>>>,
    /// ROMs that failed to load, with why.
    pub problems: Vec<String>,
}

/// A Library game, as reported to the agent.
#[derive(Serialize)]
pub struct GameInfo {
    pub sha1: String,
    pub name: String,
    pub platform: &'static str,
    pub game_code: String,
    pub header_title: String,
    pub size: u64,
    pub decomp: Option<String>,
}

impl Workspace {
    pub fn load() -> Self {
        let stored = store::load();
        let mut library = Library::new().with_platforms(clockwork_platforms::all());
        let mut problems = Vec::new();
        for path in &stored.roms {
            if let Err(e) = library.import_known(path, &stored.known) {
                problems.push(e.to_string());
            }
        }
        Self {
            library,
            stored,
            roms: HashMap::new(),
            problems,
        }
    }

    pub fn games(&self) -> Vec<GameInfo> {
        self.library
            .entries()
            .iter()
            .map(|e| GameInfo {
                sha1: e.sha1.clone(),
                name: self.title(e),
                platform: e.info.platform,
                game_code: e.info.game_code.clone(),
                header_title: e.info.title.clone(),
                size: e.size,
                decomp: self.decomp(e),
            })
            .collect()
    }

    pub fn title(&self, entry: &LibraryEntry) -> String {
        match self.stored.names.get(&entry.sha1) {
            Some(name) => display_name(name),
            None => entry.info.title.clone(),
        }
    }

    pub fn decomp(&self, entry: &LibraryEntry) -> Option<String> {
        self.stored.decomps.get(&entry.sha1).cloned()
    }

    /// Finds a game by SHA-1 (or a prefix of it) or by part of its name.
    pub fn find(&self, query: &str) -> Result<LibraryEntry, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Err("Name a game (SHA-1 or part of its name); see list_games".into());
        }
        let matches: Vec<&LibraryEntry> = self
            .library
            .entries()
            .iter()
            .filter(|e| e.sha1.starts_with(&q) || self.title(e).to_lowercase().contains(&q))
            .collect();
        match matches.as_slice() {
            [entry] => Ok((*entry).clone()),
            [] => Err(format!(
                "No game in the Library matches \"{query}\"; see list_games"
            )),
            many => Err(format!(
                "\"{query}\" matches {} games ({}); use the SHA-1",
                many.len(),
                many.iter()
                    .map(|e| self.title(e))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The game's ROM in canonical form (cached).
    pub fn rom(&mut self, entry: &LibraryEntry) -> Result<Arc<Vec<u8>>, String> {
        if let Some(rom) = self.roms.get(&entry.sha1) {
            return Ok(rom.clone());
        }
        let rom = Arc::new(self.library.rom_bytes(entry).map_err(|e| e.to_string())?);
        self.roms.insert(entry.sha1.clone(), rom.clone());
        Ok(rom)
    }
}

/// Where plugins are read from, in order. New plugins are saved to the first.
pub fn plugin_dirs() -> Vec<PathBuf> {
    clockwork_library::content::plugin_dirs()
}

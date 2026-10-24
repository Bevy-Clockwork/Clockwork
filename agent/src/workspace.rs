//! The user's Library as the agent sees it (a [`Catalog`] kept current with the editor's
//! changes), plus where plugins and knowledge packs live.

use std::path::PathBuf;
use std::sync::Arc;

use clockwork_library::{Library, LibraryEntry, catalog::Catalog};
use serde::Serialize;

pub struct Workspace {
    catalog: Catalog,
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
        Self {
            catalog: Catalog::load(Library::new().with_platforms(clockwork_platforms::all())),
        }
    }

    /// Picks up what the editor changed in the Library since it was last read (games imported,
    /// named or forgotten); cheap when nothing changed.
    pub fn refresh(&mut self) {
        self.catalog.refresh();
    }

    /// Games that couldn't be read, with why.
    pub fn problems(&self) -> &[String] {
        &self.catalog.problems
    }

    pub fn games(&self) -> Vec<GameInfo> {
        self.catalog
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
        self.catalog.title(entry)
    }

    pub fn decomp(&self, entry: &LibraryEntry) -> Option<String> {
        self.catalog.decomp(entry)
    }

    /// Finds a game by SHA-1 (or a prefix of it) or by part of its name.
    pub fn find(&self, query: &str) -> Result<LibraryEntry, String> {
        self.catalog.find(query)
    }

    /// The game's ROM in canonical form (cached).
    pub fn rom(&mut self, entry: &LibraryEntry) -> Result<Arc<Vec<u8>>, String> {
        self.catalog.rom(entry)
    }
}

/// Where plugins are read from, in order. New plugins are saved to the first.
pub fn plugin_dirs() -> Vec<PathBuf> {
    clockwork_library::content::plugin_dirs()
}

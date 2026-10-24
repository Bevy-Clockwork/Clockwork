//! The Library as a service: the games of `library.ron` with their names, decomps and ROMs,
//! kept current while another program changes the file. The agent's MCP server keeps one for
//! every session; games the user imports, names or forgets in the editor show up in it at the
//! next call, without reading every ROM again (records of unchanged files are trusted, see
//! [`Library::import_game`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::{Library, LibraryEntry, boxart::display_name, store};

/// What a refresh found changed, by SHA-1.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// `library.ron` was read again (names and decomps may have changed too).
    pub reread: bool,
}

pub struct Catalog {
    library: Library,
    stored: store::Stored,
    /// Where the Library is saved, and its length and modification time when last read.
    path: Option<PathBuf>,
    seen: Option<(u64, SystemTime)>,
    /// ROMs in canonical form, by SHA-1, read on first use.
    roms: HashMap<String, Arc<Vec<u8>>>,
    /// Games that couldn't be read, with why.
    pub problems: Vec<String>,
    /// Counts the times the Library changed.
    revision: u64,
}

fn stamp(path: Option<&Path>) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path?).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

impl Catalog {
    /// Reads the saved Library; `platforms` is an empty [`Library`] with the platforms that
    /// identify games (`Library::new().with_platforms(clockwork_platforms::all())`).
    pub fn load(platforms: Library) -> Self {
        Self::load_from(store::store_path(), platforms)
    }

    /// Reads the Library saved at `path` (`None`: there is none).
    pub fn load_from(path: Option<PathBuf>, platforms: Library) -> Self {
        let mut catalog = Self {
            library: platforms,
            stored: store::Stored::default(),
            path,
            seen: None,
            roms: HashMap::new(),
            problems: Vec::new(),
            revision: 0,
        };
        catalog.read();
        catalog
    }

    fn read(&mut self) {
        self.seen = stamp(self.path.as_deref());
        self.stored = match &self.path {
            Some(path) => store::load_at(path).0,
            None => store::Stored::default(),
        };
        let mut library = self.library.without_games();
        self.problems.clear();
        for game in &self.stored.games {
            if let Err(e) = library.import_game(game) {
                self.problems.push(e.to_string());
            }
        }
        self.library = library;
        self.revision += 1;
    }

    /// Reads `library.ron` again if it changed since it was read, and says what changed.
    pub fn refresh(&mut self) -> Changes {
        if stamp(self.path.as_deref()) == self.seen {
            return Changes::default();
        }
        let before: Vec<String> = self.sha1s();
        self.read();
        let after = self.sha1s();
        self.roms.retain(|sha1, _| after.contains(sha1));
        Changes {
            added: after
                .iter()
                .filter(|s| !before.contains(s))
                .cloned()
                .collect(),
            removed: before.into_iter().filter(|s| !after.contains(s)).collect(),
            reread: true,
        }
    }

    fn sha1s(&self) -> Vec<String> {
        self.library
            .entries()
            .iter()
            .map(|e| e.sha1.clone())
            .collect()
    }

    /// Changes whenever the Library is read again.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        self.library.entries()
    }

    pub fn stored(&self) -> &store::Stored {
        &self.stored
    }

    /// The game's full name, if known, else its header title.
    pub fn title(&self, entry: &LibraryEntry) -> String {
        match self.stored.game(&entry.sha1).and_then(|g| g.name.as_ref()) {
            Some(name) => display_name(name),
            None => entry.info.title.clone(),
        }
    }

    pub fn decomp(&self, entry: &LibraryEntry) -> Option<String> {
        self.stored.game(&entry.sha1).and_then(|g| g.decomp.clone())
    }

    /// Finds a game by SHA-1 (or a prefix of it) or by part of its name.
    pub fn find(&self, query: &str) -> Result<LibraryEntry, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Err("Name a game (SHA-1 or part of its name); see list_games".into());
        }
        let matches: Vec<&LibraryEntry> = self
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

    /// The game's ROM in canonical form (kept once read).
    pub fn rom(&mut self, entry: &LibraryEntry) -> Result<Arc<Vec<u8>>, String> {
        if let Some(rom) = self.roms.get(&entry.sha1) {
            return Ok(rom.clone());
        }
        let rom = Arc::new(self.library.rom_bytes(entry).map_err(|e| e.to_string())?);
        self.roms.insert(entry.sha1.clone(), rom.clone());
        Ok(rom)
    }
}

#[cfg(test)]
mod tests {
    use clockwork_gba::Gba;

    use super::*;
    use crate::{Game, sha1_hex};

    fn gba(title: &[u8; 4]) -> Vec<u8> {
        let mut rom = vec![0u8; 0xc0];
        rom[0xa0..0xa4].copy_from_slice(title);
        rom[0xb2] = 0x96;
        rom[0xbd] = rom[0xa0..0xbd]
            .iter()
            .fold(0u8, |sum, b| sum.wrapping_sub(*b))
            .wrapping_sub(0x19);
        rom
    }

    fn save(path: &Path, games: Vec<Game>) {
        let stored = store::Stored::new(games, false);
        let text = ron::ser::to_string_pretty(&stored, Default::default()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn games_added_and_forgotten_elsewhere_show_up_at_the_next_refresh() {
        let dir = std::env::temp_dir().join(format!("clockwork-catalog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.gba"), dir.join("b.gba"));
        std::fs::write(&a, gba(b"AAAA")).unwrap();
        std::fs::write(&b, gba(b"BBBB")).unwrap();
        let saved = dir.join("library.ron");
        save(&saved, vec![Game::at(a.clone())]);

        let mut catalog =
            Catalog::load_from(Some(saved.clone()), Library::new().with_platform(Gba));
        assert_eq!(catalog.entries().len(), 1);
        assert_eq!(catalog.refresh(), Changes::default(), "nothing changed");
        let first = catalog.revision();

        let mut named = Game::at(b.clone());
        named.sha1 = sha1_hex(&gba(b"BBBB"));
        named.name = Some("Bee Game (USA)".into());
        // A different length, so the change shows however coarse file times are.
        save(&saved, vec![named]);
        let changes = catalog.refresh();
        assert_eq!(changes.added, vec![sha1_hex(&gba(b"BBBB"))]);
        assert_eq!(changes.removed, vec![sha1_hex(&gba(b"AAAA"))]);
        assert!(catalog.revision() > first);
        assert_eq!(catalog.find("bee").unwrap().path, b);
        assert!(catalog.find("AAAA").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

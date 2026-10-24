//! The Library: games the user has imported from their own ROMs (DESIGN.md §4).
//!
//! ROMs are read in place and never copied into plugins.

pub mod agents;
pub mod boxart;
pub mod content;
mod cue;
pub mod files;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clockwork_core::{Platform, RomInfo};
use sha1::{Digest, Sha1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEntry {
    pub path: PathBuf,
    /// Lowercase hex SHA-1 of the ROM in its platform's canonical form (see
    /// [`Platform::canonicalize`]), matched against plugin manifests and No-Intro databases.
    pub sha1: String,
    pub size: u64,
    pub info: RomInfo,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0} is not a supported game image or executable")]
    UnknownPlatform(PathBuf),
    #[error("could not read zip archive {path}: {message}")]
    Zip { path: PathBuf, message: String },
    #[error("could not read cue sheet {path}: {message}")]
    Cue { path: PathBuf, message: String },
}

/// Game file extensions shared by the editor, folder scanner and zip reader.
/// An extension only selects candidates; the platform module validates their contents.
pub const ROM_EXTENSIONS: [&str; 13] = [
    "gba", "z64", "n64", "v64", "sfc", "smc", "nds", "iso", "bin", "cue", "exe", "elf", "ps-exe",
];

/// Reads a game image/executable, a cue sheet's data file, or the first candidate in a zip.
/// Library import additionally validates archive members against its registered platforms.
pub fn read_rom_file(path: &Path) -> Result<Vec<u8>, ImportError> {
    read_rom_matching(path, |_| true)
}

fn read_rom_matching(path: &Path, accepts: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>, ImportError> {
    let io_error = |source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    };
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
    {
        return cue::read(path);
    }
    let is_zip = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !is_zip {
        return std::fs::read(path).map_err(io_error);
    }

    let zip_error = |e: zip::result::ZipError| ImportError::Zip {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let file = std::fs::File::open(path).map_err(io_error)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    for i in 0..archive.len() {
        let mut member = archive.by_index(i).map_err(zip_error)?;
        let is_rom = member.is_file()
            && member.name().is_ok_and(|name| {
                let name = name.to_ascii_lowercase();
                ROM_EXTENSIONS
                    .iter()
                    .any(|ext| *ext != "cue" && name.ends_with(&format!(".{ext}")))
            });
        if is_rom {
            // Never inflate past the size the archive declares: a member that claims a few
            // bytes but expands to gigabytes (a zip bomb) is cut off and rejected.
            let declared = member.size();
            let mut rom = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(&mut member, declared.saturating_add(1)),
                &mut rom,
            )
            .map_err(io_error)?;
            if rom.len() as u64 > declared {
                return Err(ImportError::Zip {
                    path: path.to_path_buf(),
                    message: format!("a member expands past its declared {declared} bytes"),
                });
            }
            if accepts(&rom) {
                return Ok(rom);
            }
        }
    }
    Err(ImportError::UnknownPlatform(path.to_path_buf()))
}

#[derive(Default)]
pub struct Library {
    platforms: Vec<Arc<dyn Platform>>,
    entries: Vec<LibraryEntry>,
}

/// Reads and identifies game files without the [`Library`], e.g. on another thread; the
/// entries it makes are added with [`Library::add`].
#[derive(Clone)]
pub struct Identifier {
    platforms: Vec<Arc<dyn Platform>>,
}

impl Identifier {
    /// Reads a game file (see [`read_rom_file`]) and identifies it.
    pub fn read(&self, path: &Path) -> Result<LibraryEntry, ImportError> {
        let rom = read_rom_matching(path, |rom| {
            self.platforms.iter().any(|p| p.identify(rom).is_some())
        })?;
        self.identify(path.to_path_buf(), &rom)
    }

    /// Identifies a game from its bytes: its platform, header and canonical hash.
    pub fn identify(&self, path: PathBuf, rom: &[u8]) -> Result<LibraryEntry, ImportError> {
        let (platform, mut info) = self
            .platforms
            .iter()
            .find_map(|p| p.identify(rom).map(|info| (p, info)))
            .ok_or_else(|| ImportError::UnknownPlatform(path.clone()))?;
        if info.title.is_empty() {
            info.title = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled game".into());
        }
        let sha1 = match platform.canonicalize(rom) {
            Some(canonical) => sha1_hex(&canonical),
            None => sha1_hex(rom),
        };
        Ok(LibraryEntry {
            path,
            sha1,
            size: rom.len() as u64,
            info,
        })
    }
}

/// The game files in a folder and the folders in it: files with a game extension or `.zip`,
/// leaving out the `.bin` files of folders that have a cue sheet (the sheet stands for them).
/// Hidden files and folders are skipped.
pub fn scan_folder(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(read) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut files = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => folders.push(path),
                Ok(t) if t.is_file() => files.push(path),
                _ => {}
            }
        }
        let extension = |p: &PathBuf| {
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        };
        let has_cue = files.iter().any(|f| extension(f) == "cue");
        files.retain(|f| {
            let ext = extension(f);
            (ext == "zip" || ROM_EXTENSIONS.contains(&ext.as_str())) && !(has_cue && ext == "bin")
        });
        files.sort();
        found.extend(files);
    }
    found
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_platform(mut self, platform: impl Platform + 'static) -> Self {
        self.platforms.push(Arc::new(platform));
        self
    }

    /// Adds several platform modules, e.g. `clockwork_platforms::all()`.
    pub fn with_platforms(
        mut self,
        platforms: impl IntoIterator<Item = Box<dyn Platform>>,
    ) -> Self {
        self.platforms.extend(platforms.into_iter().map(Arc::from));
        self
    }

    /// Something that reads and identifies files as this library does, for another thread.
    pub fn identifier(&self) -> Identifier {
        Identifier {
            platforms: self.platforms.clone(),
        }
    }

    /// Adds an entry made by an [`Identifier`]; one of the same game (by hash) already here is
    /// kept instead. Returns its index and whether it is new.
    pub fn add(&mut self, entry: LibraryEntry) -> (usize, bool) {
        if let Some(i) = self.entries.iter().position(|e| e.sha1 == entry.sha1) {
            return (i, false);
        }
        self.entries.push(entry);
        (self.entries.len() - 1, true)
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    pub fn import(&mut self, path: impl AsRef<Path>) -> Result<&LibraryEntry, ImportError> {
        let entry = self.identifier().read(path.as_ref())?;
        let (i, _) = self.add(entry);
        Ok(&self.entries[i])
    }

    /// Reads an entry's ROM in its platform's canonical form, ready for recipes.
    pub fn rom_bytes(&self, entry: &LibraryEntry) -> Result<Vec<u8>, ImportError> {
        let platform = self
            .platforms
            .iter()
            .find(|p| p.id() == entry.info.platform)
            .ok_or_else(|| ImportError::UnknownPlatform(entry.path.clone()))?;
        let rom = read_rom_matching(&entry.path, |rom| platform.identify(rom).is_some())?;
        Ok(platform.canonicalize(&rom).unwrap_or(rom))
    }

    pub fn import_bytes(
        &mut self,
        path: PathBuf,
        rom: &[u8],
    ) -> Result<&LibraryEntry, ImportError> {
        let entry = self.identifier().identify(path, rom)?;
        let (i, _) = self.add(entry);
        Ok(&self.entries[i])
    }

    pub fn remove(&mut self, index: usize) -> Option<LibraryEntry> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }
}

/// A game the Library remembers: one record of `library.ron`. Holds where its file is, what was
/// learned from the file when it was last read (its hash and header, trusted while the file's
/// length and modification time are unchanged) and what the user added: the game's full name
/// and its decompilation. Never game data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Game {
    pub path: PathBuf,
    /// Empty if the file was never read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha1: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_code: String,
    /// The file's length and modification time (seconds and nanoseconds since 1970) when it
    /// was read; none for a file that is always read again (a cue sheet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<(u64, (u64, u32))>,
    /// Its No-Intro name, once identified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The decompilation repository linked to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decomp: Option<String>,
    /// When it was added and last used, and how the user marked it.
    #[serde(default, skip_serializing_if = "GameNotes::is_empty")]
    pub notes: GameNotes,
}

/// What the user's use of a game adds to its record.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GameNotes {
    /// When it was added to the Library, in seconds since 1970; 0 if before this was kept.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub added: u64,
    /// When its ROM was last read for a plugin, in seconds since 1970; 0 if never.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub used: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favourite: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl GameNotes {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Game {
    /// A game known only by where its file is.
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            sha1: String::new(),
            size: 0,
            platform: String::new(),
            title: String::new(),
            game_code: String::new(),
            file: None,
            name: None,
            decomp: None,
            notes: GameNotes::default(),
        }
    }
}

/// The length and modification time of a file whose contents can be trusted to change only
/// when they do. Cue sheets point at another file, so they are always read.
fn stamp(path: &Path) -> Option<(u64, (u64, u32))> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
    {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some((meta.len(), (modified.as_secs(), modified.subsec_nanos())))
}

impl Library {
    /// The record of `entry` (without a name or decomp), with its file's stamp if its file can
    /// be recognized again without reading it.
    pub fn game(&self, entry: &LibraryEntry) -> Game {
        Game {
            path: entry.path.clone(),
            sha1: entry.sha1.clone(),
            size: entry.size,
            platform: entry.info.platform.to_string(),
            title: entry.info.title.clone(),
            game_code: entry.info.game_code.clone(),
            file: stamp(&entry.path),
            name: None,
            decomp: None,
            notes: GameNotes::default(),
        }
    }

    /// Imports a remembered game, trusting its record while its file's length and modification
    /// time are unchanged and its platform is registered; otherwise reads the file.
    pub fn import_game(&mut self, game: &Game) -> Result<&LibraryEntry, ImportError> {
        let path = game.path.as_path();
        let remembered = self
            .platforms
            .iter()
            .find(|p| !game.sha1.is_empty() && p.id() == game.platform)
            .filter(|_| game.file.is_some() && stamp(path) == game.file)
            .map(|platform| LibraryEntry {
                path: path.to_path_buf(),
                sha1: game.sha1.clone(),
                size: game.size,
                info: RomInfo {
                    platform: platform.id(),
                    title: game.title.clone(),
                    game_code: game.game_code.clone(),
                },
            });
        let Some(entry) = remembered else {
            return self.import(path);
        };
        if let Some(i) = self.entries.iter().position(|e| e.sha1 == entry.sha1) {
            return Ok(&self.entries[i]);
        }
        self.entries.push(entry);
        Ok(self.entries.last().unwrap())
    }
}

pub fn sha1_hex(data: &[u8]) -> String {
    Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_gba::Gba;

    #[test]
    fn unchanged_files_are_recalled_without_reading_them() {
        let dir = std::env::temp_dir().join(format!("clockwork-known-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.gba");
        let mut rom = vec![0u8; 0xc0];
        rom[0xa0..0xa4].copy_from_slice(b"TEST");
        rom[0xb2] = 0x96;
        rom[0xbd] = rom[0xa0..0xbd]
            .iter()
            .fold(0u8, |sum, b| sum.wrapping_sub(*b))
            .wrapping_sub(0x19);
        std::fs::write(&path, &rom).unwrap();

        let mut first = Library::new().with_platform(Gba);
        let entry = first.import(&path).unwrap().clone();
        let mut known = first.game(&entry);
        assert!(known.file.is_some());
        // A remembered hash that differs from the file's proves the file wasn't read.
        known.sha1 = "remembered".into();
        let mut second = Library::new().with_platform(Gba);
        assert_eq!(second.import_game(&known).unwrap().sha1, "remembered");

        // A changed file is read again.
        rom.extend([0; 0x40]);
        std::fs::write(&path, &rom).unwrap();
        let mut third = Library::new().with_platform(Gba);
        let reread = third.import_game(&known).unwrap();
        assert_eq!(reread.sha1, sha1_hex(&rom));
        assert_eq!(reread.size, rom.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_scan_finds_game_files_and_leaves_out_what_a_cue_sheet_stands_for() {
        let dir = std::env::temp_dir().join(format!("clockwork-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for file in [
            "a.gba",
            "notes.txt",
            "sub/b.Z64",
            "sub/c.zip",
            "disc/game.cue",
            "disc/game.bin",
            ".hidden/d.gba",
            "loose/e.bin",
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        let mut found: Vec<String> = scan_folder(&dir)
            .iter()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "a.gba",
                "disc/game.cue",
                "loose/e.bin",
                "sub/b.Z64",
                "sub/c.zip"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha1_of_empty_input() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn rejects_unknown_files() {
        let mut lib = Library::new().with_platform(Gba);
        let err = lib.import_bytes("notes.txt".into(), b"hello").unwrap_err();
        assert!(matches!(err, ImportError::UnknownPlatform(_)));
        assert!(lib.entries().is_empty());
    }

    #[test]
    fn n64_byte_orders_hash_the_same() {
        let mut z64 = vec![0x20u8; 0x40];
        z64[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        let v64: Vec<u8> = z64.chunks(2).flat_map(|c| [c[1], c[0]]).collect();

        let mut lib = Library::new().with_platform(clockwork_n64::N64);
        let a = lib.import_bytes("a.z64".into(), &z64).unwrap().sha1.clone();
        let b = lib.import_bytes("a.v64".into(), &v64).unwrap().sha1.clone();
        assert_eq!(a, b);
        assert_eq!(a, sha1_hex(&z64));
        assert_eq!(lib.entries().len(), 1);
    }
}

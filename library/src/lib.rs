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
    platforms: Vec<Box<dyn Platform>>,
    entries: Vec<LibraryEntry>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_platform(mut self, platform: impl Platform + 'static) -> Self {
        self.platforms.push(Box::new(platform));
        self
    }

    /// Adds several platform modules, e.g. `clockwork_platforms::all()`.
    pub fn with_platforms(
        mut self,
        platforms: impl IntoIterator<Item = Box<dyn Platform>>,
    ) -> Self {
        self.platforms.extend(platforms);
        self
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    pub fn import(&mut self, path: impl AsRef<Path>) -> Result<&LibraryEntry, ImportError> {
        let path = path.as_ref();
        let rom = read_rom_matching(path, |rom| {
            self.platforms.iter().any(|p| p.identify(rom).is_some())
        })?;
        self.import_bytes(path.to_path_buf(), &rom)
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

        if let Some(i) = self.entries.iter().position(|e| e.sha1 == sha1) {
            return Ok(&self.entries[i]);
        }
        self.entries.push(LibraryEntry {
            path,
            sha1,
            size: rom.len() as u64,
            info,
        });
        Ok(self.entries.last().unwrap())
    }

    pub fn remove(&mut self, index: usize) -> Option<LibraryEntry> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }
}

/// What the Library remembers about an imported file, so the next start can skip reading and
/// hashing it while the file is unchanged (same length and modification time). Holds the
/// header's title and code and the hash, never game data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KnownRom {
    pub path: PathBuf,
    /// The file's length and modification time (seconds and nanoseconds since 1970).
    pub file_len: u64,
    pub modified: (u64, u32),
    pub sha1: String,
    pub size: u64,
    pub platform: String,
    pub title: String,
    pub game_code: String,
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
    /// What to remember about `entry` for [`Library::import_known`], if its file can be
    /// recognized again without reading it.
    pub fn known(&self, entry: &LibraryEntry) -> Option<KnownRom> {
        let (file_len, modified) = stamp(&entry.path)?;
        Some(KnownRom {
            path: entry.path.clone(),
            file_len,
            modified,
            sha1: entry.sha1.clone(),
            size: entry.size,
            platform: entry.info.platform.to_string(),
            title: entry.info.title.clone(),
            game_code: entry.info.game_code.clone(),
        })
    }

    /// Imports `path`, trusting what `known` remembers about it while the file's length and
    /// modification time are unchanged and its platform is registered; otherwise reads it.
    pub fn import_known(
        &mut self,
        path: impl AsRef<Path>,
        known: &[KnownRom],
    ) -> Result<&LibraryEntry, ImportError> {
        let path = path.as_ref();
        let remembered = known.iter().find(|k| k.path == path).and_then(|k| {
            let platform = self.platforms.iter().find(|p| p.id() == k.platform)?;
            (stamp(path)? == (k.file_len, k.modified)).then(|| LibraryEntry {
                path: path.to_path_buf(),
                sha1: k.sha1.clone(),
                size: k.size,
                info: RomInfo {
                    platform: platform.id(),
                    title: k.title.clone(),
                    game_code: k.game_code.clone(),
                },
            })
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
        let mut known = first.known(&entry).unwrap();
        // A remembered hash that differs from the file's proves the file wasn't read.
        known.sha1 = "remembered".into();
        let mut second = Library::new().with_platform(Gba);
        assert_eq!(
            second.import_known(&path, &[known.clone()]).unwrap().sha1,
            "remembered"
        );

        // A changed file is read again.
        rom.extend([0; 0x40]);
        std::fs::write(&path, &rom).unwrap();
        let mut third = Library::new().with_platform(Gba);
        let reread = third.import_known(&path, &[known]).unwrap();
        assert_eq!(reread.sha1, sha1_hex(&rom));
        assert_eq!(reread.size, rom.len() as u64);
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

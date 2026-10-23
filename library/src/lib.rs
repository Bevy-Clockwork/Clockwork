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
            let mut rom = Vec::new();
            std::io::Read::read_to_end(&mut member, &mut rom).map_err(io_error)?;
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

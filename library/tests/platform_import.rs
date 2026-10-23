use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use clockwork_library::{Library, sha1_hex};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(
            "clockwork-platform-import-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // This is the unique directory successfully created above, never a user-supplied path.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn library() -> Library {
    Library::new().with_platforms(clockwork_platforms::all())
}

fn put16(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn windows_exe() -> Vec<u8> {
    let mut bytes = vec![0; 1024];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x84, 0x14c);
    put16(&mut bytes, 0x86, 1);
    put16(&mut bytes, 0x94, 224);
    put16(&mut bytes, 0x96, 2);
    put16(&mut bytes, 0x98, 0x10b);
    put32(&mut bytes, 0xa8, 0x1000);
    put32(&mut bytes, 0xd0, 0x2000);
    put32(&mut bytes, 0xd4, 512);
    put16(&mut bytes, 0xdc, 2);
    put32(&mut bytes, 0x178 + 8, 512);
    put32(&mut bytes, 0x178 + 12, 0x1000);
    put32(&mut bytes, 0x178 + 16, 512);
    put32(&mut bytes, 0x178 + 20, 512);
    put32(&mut bytes, 0x178 + 36, 0x6000_0020);
    bytes
}

fn snes_cartridge() -> Vec<u8> {
    let mut bytes = vec![0xea; 0x8000];
    let header = &mut bytes[0x7fc0..];
    header[..21].fill(b' ');
    header[..10].copy_from_slice(b"SNES QUEST");
    header[0x15] = 0x20;
    header[0x17] = 5;
    header[0x18] = 0;
    header[0x1a] = 1;
    header[0x1c..0x20].copy_from_slice(&[0xcb, 0xed, 0x34, 0x12]);
    header[0x3c..0x3e].copy_from_slice(&0x8000u16.to_le_bytes());
    bytes
}

fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for &(name, data) in members {
        archive.start_file(name, options).unwrap();
        archive.write_all(data).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn pc_import_uses_filename_and_preserves_executable_bytes() {
    let directory = TestDir::new();
    let bytes = windows_exe();
    let path = directory.write("Clockwork Quest.exe", &bytes);
    let mut library = library();
    let entry = library.import(&path).unwrap().clone();
    assert_eq!(entry.info.platform, "pc");
    assert_eq!(entry.info.title, "Clockwork Quest");
    assert!(entry.info.game_code.is_empty());
    assert_eq!(entry.path, path);
    assert_eq!(entry.size, bytes.len() as u64);
    assert_eq!(entry.sha1, sha1_hex(&bytes));
    assert_eq!(library.rom_bytes(&entry).unwrap(), bytes);
}

#[test]
fn snes_copier_header_deduplicates_and_reads_back_canonical_bytes() {
    let directory = TestDir::new();
    let cartridge = snes_cartridge();
    let mut headered = vec![0x57; 512];
    headered.extend_from_slice(&cartridge);
    let smc = directory.write("SNES Quest.smc", &headered);
    let sfc = directory.write("SNES Quest.sfc", &cartridge);
    let mut library = library();
    let first = library.import(&smc).unwrap().clone();
    let second = library.import(&sfc).unwrap().clone();
    assert_eq!(first.info.platform, "snes");
    assert_eq!(first.info.title, "SNES QUEST");
    assert_eq!(first.sha1, sha1_hex(&cartridge));
    assert_eq!(first, second);
    assert_eq!(library.entries().len(), 1);
    assert_eq!(library.rom_bytes(&first).unwrap(), cartridge);
}

#[test]
fn zip_skips_unrecognized_bin_and_reads_back_the_imported_executable() {
    let directory = TestDir::new();
    let bytes = windows_exe();
    let path = directory.0.join("Windows game.zip");
    write_zip(
        &path,
        &[
            ("assets.bin", b"not a game image"),
            ("game/play.exe", &bytes),
        ],
    );
    let mut library = library();
    let entry = library.import(&path).unwrap().clone();
    assert_eq!(entry.info.platform, "pc");
    assert_eq!(entry.path, path);
    assert_eq!(entry.sha1, sha1_hex(&bytes));
    assert_eq!(library.rom_bytes(&entry).unwrap(), bytes);
}

#[test]
fn cue_resolves_named_data_file_preserves_path_and_deduplicates() {
    let directory = TestDir::new();
    // A standalone PS-X executable exercises CUE file resolution independently from the
    // disc parser's ISO/raw-sector tests; CUE does not decide the underlying platform.
    let mut bytes = vec![0; 4096];
    bytes[..8].copy_from_slice(b"PS-X EXE");
    put32(&mut bytes, 0x1c, 2048);
    let bin = directory.write("data track.bin", &bytes);
    let cue = directory.write(
        "Disc One.cue",
        b"FILE \"data track.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n",
    );
    let mut library = library();
    let entry = library.import(&cue).unwrap().clone();
    assert_eq!(entry.info.platform, "ps1");
    assert_eq!(entry.info.title, "Disc One");
    assert_eq!(entry.path, cue);
    assert_eq!(entry.sha1, sha1_hex(&bytes));
    assert_eq!(library.rom_bytes(&entry).unwrap(), bytes);
    assert_eq!(library.import(&bin).unwrap(), &entry);
    assert_eq!(library.entries().len(), 1);
}

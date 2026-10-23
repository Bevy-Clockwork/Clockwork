use super::*;

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 2..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn record(name: &[u8], block: u32, size: u32, directory: bool) -> Vec<u8> {
    let mut out = vec![0; (33 + name.len()).next_multiple_of(2)];
    out[0] = out.len() as u8;
    put32(&mut out, 2, block);
    put32(&mut out, 10, size);
    out[25] = if directory { 2 } else { 0 };
    put16(&mut out, 28, 1);
    out[32] = name.len() as u8;
    out[33..33 + name.len()].copy_from_slice(name);
    out
}

fn psx_exe() -> Vec<u8> {
    let mut exe = vec![0; 2 * BLOCK];
    exe[..8].copy_from_slice(b"PS-X EXE");
    exe[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    exe[0x18..0x1c].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    exe[0x1c..0x20].copy_from_slice(&(BLOCK as u32).to_le_bytes());
    exe
}

fn ps2_elf() -> Vec<u8> {
    let mut elf = vec![0; BLOCK];
    elf[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    elf[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf[18..20].copy_from_slice(&8u16.to_le_bytes());
    elf[20..24].copy_from_slice(&1u32.to_le_bytes());
    elf[28..32].copy_from_slice(&52u32.to_le_bytes());
    elf[40..42].copy_from_slice(&52u16.to_le_bytes());
    elf[42..44].copy_from_slice(&32u16.to_le_bytes());
    elf[44..46].copy_from_slice(&1u16.to_le_bytes());
    elf[52..56].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    elf[56..60].copy_from_slice(&84u32.to_le_bytes());
    elf[68..72].copy_from_slice(&16u32.to_le_bytes());
    elf[72..76].copy_from_slice(&16u32.to_le_bytes());
    elf
}

/// A generated 64-KiB disc with real dual-endian ISO9660 directory records.
fn iso(ps2: bool, nested: bool) -> Vec<u8> {
    let mut iso = vec![0; 32 * BLOCK];
    let pvd = &mut iso[16 * BLOCK..17 * BLOCK];
    pvd[..7].copy_from_slice(b"\x01CD001\x01");
    pvd[8..40].fill(b' ');
    pvd[8..19].copy_from_slice(b"PLAYSTATION");
    pvd[40..72].fill(b' ');
    pvd[40..49].copy_from_slice(b"TEST_DISC");
    put32(pvd, 80, 32);
    put16(pvd, 128, BLOCK as u16);
    pvd[156..190].copy_from_slice(&record(&[0], 20, BLOCK as u32, true));
    iso[17 * BLOCK..17 * BLOCK + 7].copy_from_slice(b"\xffCD001\x01");

    let key = if ps2 { "BOOT2" } else { "BOOT" };
    let device = if ps2 { "cdrom0" } else { "cdrom" };
    let name = if ps2 {
        "SLUS_203.12;1"
    } else {
        "SCUS_941.63;1"
    };
    let prefix = if nested { "EXE\\" } else { "" };
    let config = format!("{key} = {device}:\\{prefix}{name}\r\nVER = 1.00\r\n");
    iso[21 * BLOCK..21 * BLOCK + config.len()].copy_from_slice(config.as_bytes());
    let exe = if ps2 { ps2_elf() } else { psx_exe() };
    iso[22 * BLOCK..22 * BLOCK + exe.len()].copy_from_slice(&exe);

    let mut root = Vec::new();
    root.extend(record(&[0], 20, BLOCK as u32, true));
    root.extend(record(&[1], 20, BLOCK as u32, true));
    root.extend(record(b"SYSTEM.CNF;1", 21, config.len() as u32, false));
    let executable = record(name.as_bytes(), 22, exe.len() as u32, false);
    if nested {
        root.extend(record(b"EXE", 24, BLOCK as u32, true));
        iso[24 * BLOCK..24 * BLOCK + executable.len()].copy_from_slice(&executable);
    } else {
        root.extend(executable);
    }
    iso[20 * BLOCK..20 * BLOCK + root.len()].copy_from_slice(&root);
    iso
}

fn raw(iso: &[u8], mode: u8, pregap: usize) -> Vec<u8> {
    let mut raw = vec![0; (iso.len() / BLOCK + pregap) * RAW_BLOCK];
    for (i, data) in iso.as_chunks::<BLOCK>().0.iter().enumerate() {
        let sector = &mut raw[(i + pregap) * RAW_BLOCK..(i + pregap + 1) * RAW_BLOCK];
        sector[..12].copy_from_slice(&SYNC);
        sector[15] = mode;
        let offset = if mode == 1 { 16 } else { 24 };
        sector[offset..offset + BLOCK].copy_from_slice(data);
    }
    raw
}

#[test]
fn identifies_psx_executable_and_rejects_truncation() {
    let exe = psx_exe();
    assert_eq!(Ps1.identify(&exe).unwrap().platform, "ps1");
    assert!(Ps2.identify(&exe).is_none());
    for length in [0, 7, BLOCK - 1, BLOCK, exe.len() - 1] {
        assert!(Ps1.identify(&exe[..length]).is_none());
    }
    let mut bad = exe;
    bad[0x1c..0x20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Ps1.identify(&bad).is_none());
}

#[test]
fn identifies_both_consoles_from_iso_boot_files() {
    for nested in [false, true] {
        let ps1 = iso(false, nested);
        let ps2 = iso(true, nested);
        let info = Ps1.identify(&ps1).unwrap();
        assert_eq!(info.title, "TEST_DISC");
        assert_eq!(info.game_code, "SCUS-94163");
        assert!(Ps2.identify(&ps1).is_none());
        let info = Ps2.identify(&ps2).unwrap();
        assert_eq!(info.title, "TEST_DISC");
        assert_eq!(info.game_code, "SLUS-20312");
        assert!(Ps1.identify(&ps2).is_none());
    }
}

#[test]
fn identifies_mode1_and_mode2_raw_tracks_with_optional_pregap() {
    for mode in [1, 2] {
        for pregap in [0, 150] {
            let ps1 = raw(&iso(false, false), mode, pregap);
            let ps2 = raw(&iso(true, false), mode, pregap);
            assert_eq!(Ps1.identify(&ps1).unwrap().game_code, "SCUS-94163");
            assert_eq!(Ps2.identify(&ps2).unwrap().game_code, "SLUS-20312");
        }
    }
}

#[test]
fn raw_image_is_preserved_even_with_xa_form2_sectors() {
    let mut bytes = raw(&iso(false, false), 2, 0);
    // This sector is not filesystem metadata or boot code; XA media is allowed.
    bytes[26 * RAW_BLOCK + 18] = 0x20;
    bytes[26 * RAW_BLOCK + 22] = 0x20;
    bytes[26 * RAW_BLOCK + 24..27 * RAW_BLOCK].fill(0xa5);
    assert!(Ps1.identify(&bytes).is_some());
    assert_eq!(Ps1.canonicalize(&bytes), None);
    assert_eq!(Ps2.canonicalize(&bytes), None);
}

#[test]
fn accepts_psx_exe_default_without_system_cnf() {
    let mut bytes = iso(false, false);
    bytes[20 * BLOCK..21 * BLOCK].fill(0);
    let file = record(b"PSX.EXE;1", 22, (2 * BLOCK) as u32, false);
    bytes[20 * BLOCK..20 * BLOCK + file.len()].copy_from_slice(&file);
    let info = Ps1.identify(&bytes).unwrap();
    assert_eq!(info.title, "TEST_DISC");
    assert!(info.game_code.is_empty());
    assert!(Ps2.identify(&bytes).is_none());
}

#[test]
fn boot_syntax_is_case_insensitive_and_boot2_wins() {
    assert_eq!(
        boot_path(b" boot = cdrom:scus_941.63;1 argument\r"),
        Some(("ps1", "scus_941.63;1".to_owned()))
    );
    assert_eq!(
        boot_path(b"BOOT = cdrom:\\PSX.EXE;1\nBOOT2=cdrom0:/SLUS_203.12;1\0"),
        Some(("ps2", "SLUS_203.12;1".to_owned()))
    );
    assert_eq!(boot_path(b"BOOT2 = host:game.elf"), None);
    assert_eq!(boot_path(b"BOOT2 = cdrom0:"), None);
    assert_eq!(serial("EXE/scus_941.63;1"), "SCUS-94163");
    assert_eq!(serial("MAIN.ELF;1"), "");
}

#[test]
fn rejects_plain_iso_or_marker_strings_without_boot_executable() {
    let mut bytes = iso(false, false);
    bytes[22 * BLOCK..24 * BLOCK].fill(0);
    assert!(Ps1.identify(&bytes).is_none());
    let mut bytes = iso(true, false);
    bytes[22 * BLOCK + 18..22 * BLOCK + 20].copy_from_slice(&62u16.to_le_bytes());
    assert!(Ps2.identify(&bytes).is_none());
    assert!(
        Ps1.identify(b"PLAYSTATION BOOT=cdrom:\\SCUS_941.63;1 PS-X EXE")
            .is_none()
    );
    assert!(Ps2.identify(&ps2_elf()).is_none());
}

#[test]
fn rejects_truncated_images_and_out_of_range_extents() {
    let original = iso(false, false);
    for length in [0, 16 * BLOCK, 17 * BLOCK, 21 * BLOCK, original.len() - 1] {
        assert!(Ps1.identify(&original[..length]).is_none());
    }
    let mut bad = original.clone();
    put32(&mut bad, 16 * BLOCK + 156 + 2, u32::MAX);
    assert!(Ps1.identify(&bad).is_none());
    let mut bad = original.clone();
    put32(&mut bad, 20 * BLOCK + 68 + 10, u32::MAX);
    assert!(Ps1.identify(&bad).is_none());
    let mut bad = original;
    // Mismatched little-endian and big-endian volume size.
    bad[16 * BLOCK + 84] = 1;
    assert!(Ps1.identify(&bad).is_none());
}

#[test]
fn rejects_malformed_directory_and_raw_sector_headers() {
    let mut bytes = iso(false, false);
    bytes[20 * BLOCK] = 1;
    assert!(Ps1.identify(&bytes).is_none());
    let mut bytes = raw(&iso(false, false), 2, 0);
    bytes[20 * RAW_BLOCK + 18] = 0x20;
    bytes[20 * RAW_BLOCK + 22] = 0x20;
    assert!(Ps1.identify(&bytes).is_none());
    let mut bytes = raw(&iso(false, false), 1, 0);
    bytes[16 * RAW_BLOCK] = 1;
    assert!(Ps1.identify(&bytes).is_none());
}

#[test]
fn rejects_ps2_program_data_beyond_file_size() {
    let mut bytes = iso(true, false);
    bytes[22 * BLOCK + 56..22 * BLOCK + 60].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Ps2.identify(&bytes).is_none());
}

#[test]
fn follows_directories_across_sector_padding() {
    let mut bytes = iso(false, false);
    let root = bytes[20 * BLOCK..20 * BLOCK + 68].to_vec();
    let files = bytes[20 * BLOCK + 68..20 * BLOCK + 160].to_vec();
    // Move root directory to blocks 28-29, with file records after a zero tail.
    put32(&mut bytes, 16 * BLOCK + 156 + 2, 28);
    put32(&mut bytes, 16 * BLOCK + 156 + 10, (2 * BLOCK) as u32);
    bytes[28 * BLOCK..28 * BLOCK + root.len()].copy_from_slice(&root);
    bytes[29 * BLOCK..29 * BLOCK + files.len()].copy_from_slice(&files);
    assert_eq!(Ps1.identify(&bytes).unwrap().game_code, "SCUS-94163");
}

#[test]
fn config_read_crosses_logical_sectors() {
    let mut bytes = iso(true, false);
    let config = [
        vec![b'\n'; BLOCK - 8],
        b"BOOT2 = cdrom0:\\SLUS_203.12;1\r\n".to_vec(),
    ]
    .concat();
    put32(&mut bytes, 20 * BLOCK + 68 + 2, 26);
    put32(&mut bytes, 20 * BLOCK + 68 + 10, config.len() as u32);
    bytes[26 * BLOCK..26 * BLOCK + config.len()].copy_from_slice(&config);
    assert!(Ps2.identify(&bytes).is_some());
    assert!(Ps2.identify(&raw(&bytes, 2, 0)).is_some());
}

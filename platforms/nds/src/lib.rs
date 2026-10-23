//! Nintendo DS cartridge identification.
//!
//! Reads retail `.nds` headers, including the DS portion of DSi-enhanced games.
//! DSi-exclusive software, secure-area decryption and banner extraction are outside
//! this module's scope. A valid header does not establish that the game will run.
//!
//! Format and CRC references:
//! <https://github.com/devkitPro/ndstool/blob/master/source/header.h>,
//! <https://github.com/devkitPro/ndstool/blob/master/source/header.cpp>.

use clockwork_core::platform::{Platform, RomInfo, header_text};

const HEADER_LEN: usize = 0x200;
const LOGO_CRC: usize = 0x15c;
const HEADER_CRC: usize = 0x15e;
const NINTENDO_LOGO_CRC: u16 = 0xcf56;

pub struct Nds;

impl Platform for Nds {
    fn id(&self) -> &'static str {
        "nds"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        if rom.len() < HEADER_LEN
            || !matches!(rom[0x12], 0 | 2)
            || crc16(&rom[..HEADER_CRC]) != read_u16(rom, HEADER_CRC)
            || read_u16(rom, LOGO_CRC) != NINTENDO_LOGO_CRC
            || crc16(&rom[0xc0..LOGO_CRC]) != NINTENDO_LOGO_CRC
        {
            return None;
        }
        if !rom[..12]
            .iter()
            .all(|b| *b == 0 || (0x20..=0x7e).contains(b))
            || !rom[12..16]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return None;
        }
        let title = header_text(&rom[..12]);
        if title.is_empty() {
            return None;
        }

        let used_size = read_u32(rom, 0x80) as usize;
        let header_size = read_u32(rom, 0x84) as usize;
        let capacity = 0x20000u64.checked_shl(u32::from(rom[0x14]))?;
        if header_size < HEADER_LEN
            || header_size > used_size
            || used_size > rom.len()
            || rom.len() as u64 > capacity
        {
            return None;
        }

        // ARM9/ARM7 are mandatory. Offsets and lengths describe ROM bytes, not RAM
        // addresses. Use checked ranges so malicious u32 offsets cannot wrap.
        for (offset_field, size_field) in [(0x20, 0x2c), (0x30, 0x3c)] {
            if !valid_region(rom, offset_field, size_field, header_size, used_size, false) {
                return None;
            }
        }
        // Filesystem and overlay tables may be omitted by small homebrew images.
        for (offset_field, size_field) in [(0x40, 0x44), (0x48, 0x4c), (0x50, 0x54), (0x58, 0x5c)] {
            if !valid_region(rom, offset_field, size_field, header_size, used_size, true) {
                return None;
            }
        }
        let fat_offset = read_u32(rom, 0x48) as usize;
        let fat_size = read_u32(rom, 0x4c) as usize;
        if !fat_size.is_multiple_of(8)
            || !read_u32(rom, 0x54).is_multiple_of(32)
            || !read_u32(rom, 0x5c).is_multiple_of(32)
        {
            return None;
        }
        for entry in rom[fat_offset..fat_offset + fat_size].as_chunks::<8>().0 {
            let start = read_u32(entry, 0) as usize;
            let end = read_u32(entry, 4) as usize;
            if start < header_size || end < start || end > used_size {
                return None;
            }
        }
        let banner_offset = read_u32(rom, 0x68) as usize;
        if banner_offset != 0
            && (banner_offset < header_size
                || banner_offset
                    .checked_add(0x840)
                    .is_none_or(|end| end > used_size))
        {
            return None;
        }

        Some(RomInfo {
            platform: self.id(),
            title,
            game_code: header_text(&rom[12..16]),
        })
    }
}

fn valid_region(
    rom: &[u8],
    offset_field: usize,
    size_field: usize,
    header_size: usize,
    used_size: usize,
    optional: bool,
) -> bool {
    let start = read_u32(rom, offset_field) as usize;
    let size = read_u32(rom, size_field) as usize;
    if size == 0 {
        return optional && (start == 0 || (header_size..=used_size).contains(&start));
    }
    start >= header_size
        && start.is_multiple_of(4)
        && start.checked_add(size).is_some_and(|end| end <= used_size)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// DS BIOS CRC-16: reflected polynomial 0xA001, initial value 0xFFFF.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for &byte in bytes {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u32(rom: &mut [u8], offset: usize, value: u32) {
        rom[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn update_header_crc(rom: &mut [u8]) {
        let crc = crc16(&rom[..HEADER_CRC]);
        rom[HEADER_CRC..HEADER_CRC + 2].copy_from_slice(&crc.to_le_bytes());
    }

    fn fake_rom() -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[..12].copy_from_slice(b"TEST GAME\0\0\0");
        rom[12..16].copy_from_slice(b"ABCE");
        rom[16..18].copy_from_slice(b"01");
        put_u32(&mut rom, 0x20, 0x4000);
        put_u32(&mut rom, 0x2c, 0x4000);
        put_u32(&mut rom, 0x30, 0x8000);
        put_u32(&mut rom, 0x3c, 0x1000);
        put_u32(&mut rom, 0x40, 0x9000);
        put_u32(&mut rom, 0x44, 8);
        put_u32(&mut rom, 0x48, 0x9200);
        put_u32(&mut rom, 0x4c, 8);
        put_u32(&mut rom, 0x9200, 0x9800);
        put_u32(&mut rom, 0x9204, 0x9900);
        put_u32(&mut rom, 0x80, 0x10000);
        put_u32(&mut rom, 0x84, 0x4000);
        // Synthetic logo data with the retail CRC avoids embedding Nintendo artwork.
        // The identifier checks the format CRC, not authenticity or signatures.
        rom[0x15a..0x15c].copy_from_slice(&[0x04, 0x68]);
        rom[LOGO_CRC..LOGO_CRC + 2].copy_from_slice(&NINTENDO_LOGO_CRC.to_le_bytes());
        update_header_crc(&mut rom);
        rom
    }

    #[test]
    fn crc_matches_standard_modbus_check_vector() {
        assert_eq!(crc16(b"123456789"), 0x4b37);
    }

    #[test]
    fn identifies_ds_and_ds_compatible_dsi_headers() {
        let mut rom = fake_rom();
        for unit in [0, 2] {
            rom[0x12] = unit;
            update_header_crc(&mut rom);
            let info = Nds.identify(&rom).unwrap();
            assert_eq!(info.platform, "nds");
            assert_eq!(info.title, "TEST GAME");
            assert_eq!(info.game_code, "ABCE");
        }
    }

    #[test]
    fn accepts_trimmed_and_capacity_padded_images() {
        let rom = fake_rom();
        let mut padded = rom.clone();
        padded.resize(0x20000, 0xff);
        assert_eq!(Nds.identify(&rom), Nds.identify(&padded));
        assert!(Nds.canonicalize(&rom).is_none());
    }

    #[test]
    fn rejects_crc_corruption() {
        let valid = fake_rom();
        let mut rom = valid.clone();
        rom[0] ^= 1;
        assert!(Nds.identify(&rom).is_none());
        let mut rom = valid;
        rom[0xc0] ^= 1;
        update_header_crc(&mut rom);
        assert!(Nds.identify(&rom).is_none());
    }

    #[test]
    fn rejects_out_of_bounds_sections_even_with_correct_crc() {
        let valid = fake_rom();
        for (field, value) in [
            (0x20, 0xfffffffc),
            (0x2c, 0xffffffff),
            (0x30, 0x100),
            (0x3c, 0),
            (0x40, 0xffff0000),
            (0x4c, 7),
            (0x50, 0xffffffff),
            (0x54, 1),
            (0x68, 0xfffc),
            (0x80, 0x10001),
            (0x84, 0x10001),
            (0x84, 0x100),
        ] {
            let mut rom = valid.clone();
            put_u32(&mut rom, field, value);
            update_header_crc(&mut rom);
            assert!(Nds.identify(&rom).is_none(), "field {field:x}");
        }
    }

    #[test]
    fn rejects_file_table_entries_outside_image() {
        let mut rom = fake_rom();
        put_u32(&mut rom, 0x9204, 0xffffffff);
        assert!(Nds.identify(&rom).is_none());
    }

    #[test]
    fn rejects_invalid_platform_capacity_and_truncation() {
        let valid = fake_rom();
        for (offset, value) in [(0x12, 3), (0x12, 1), (0x14, 0xff), (12, 0)] {
            let mut rom = valid.clone();
            rom[offset] = value;
            update_header_crc(&mut rom);
            assert!(Nds.identify(&rom).is_none());
        }
        assert!(Nds.identify(&valid[..valid.len() - 1]).is_none());
        assert!(Nds.identify(&valid[..0x100]).is_none());
        assert!(Nds.identify(&vec![0; 0x10000]).is_none());
        assert!(Nds.identify(&[]).is_none());
    }
}

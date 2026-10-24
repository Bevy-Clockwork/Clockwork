//! Game Boy Advance platform module.
//!
//! LZ77 decompression, palettes, 4bpp tiles, metatiles, OAM objects and m4a music.

pub mod gfx;
pub mod lz77;
pub mod m4a;
pub mod obj;
pub mod popuri;

use clockwork_core::platform::{Platform, RomInfo, header_text};

const HEADER_LEN: usize = 0xC0;
const TITLE: std::ops::Range<usize> = 0xA0..0xAC;
const GAME_CODE: std::ops::Range<usize> = 0xAC..0xB0;
/// Always 0x96 in a valid header.
const FIXED_VALUE: usize = 0xB2;
const COMPLEMENT_CHECK: usize = 0xBD;

pub struct Gba;

impl Platform for Gba {
    fn id(&self) -> &'static str {
        "gba"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        if rom.len() < HEADER_LEN || rom[FIXED_VALUE] != 0x96 {
            return None;
        }
        if header_checksum(rom) != rom[COMPLEMENT_CHECK] {
            return None;
        }
        Some(RomInfo {
            platform: self.id(),
            title: header_text(&rom[TITLE]),
            game_code: header_text(&rom[GAME_CODE]),
        })
    }
}

/// The header's complement check over bytes 0xA0..=0xBC.
fn header_checksum(rom: &[u8]) -> u8 {
    rom[0xA0..=0xBC]
        .iter()
        .fold(0u8, |sum, &b| sum.wrapping_sub(b))
        .wrapping_sub(0x19)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_rom(title: &[u8], code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[0xA0..0xA0 + title.len()].copy_from_slice(title);
        rom[0xAC..0xB0].copy_from_slice(code);
        rom[FIXED_VALUE] = 0x96;
        rom[COMPLEMENT_CHECK] = header_checksum(&rom);
        rom
    }

    #[test]
    fn identifies_valid_header() {
        let info = Gba.identify(&fake_rom(b"TESTGAME", b"ABCE")).unwrap();
        assert_eq!(info.title, "TESTGAME");
        assert_eq!(info.game_code, "ABCE");
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut rom = fake_rom(b"TESTGAME", b"ABCE");
        rom[COMPLEMENT_CHECK] ^= 0xFF;
        assert!(Gba.identify(&rom).is_none());
    }

    #[test]
    fn rejects_short_file() {
        assert!(Gba.identify(&[0u8; 16]).is_none());
    }
}

//! Super Nintendo cartridge identification and copier-header normalization.
//!
//! Supports linear LoROM, HiROM and ExHiROM dumps, including their common
//! coprocessor mapping modes. Interleaved and split copier dumps are not supported.
//! Header checks establish the format; they do not verify the entire ROM checksum.
//!
//! Header layout and copier handling are documented by the bsnes implementation:
//! <https://github.com/bsnes-emu/bsnes/blob/master/bsnes/heuristics/super-famicom.cpp>.

use clockwork_core::platform::{Platform, RomInfo, header_text};

const BANK_SIZE: usize = 0x8000;
const COPIER_HEADER_SIZE: usize = 512;

pub struct Snes;

impl Platform for Snes {
    fn id(&self) -> &'static str {
        "snes"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        let (_, rom) = cartridge_data(rom)?;
        let offset = find_header(rom)?;
        let header = &rom[offset..offset + 0x40];
        // Older cartridges have no game code; do not read arbitrary program bytes
        // as an identifier when the expanded header is absent.
        let code = &rom[offset - 14..offset - 10];
        let game_code = if header[0x1a] == 0x33
            && code
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            header_text(code)
        } else {
            String::new()
        };
        Some(RomInfo {
            platform: self.id(),
            title: title_text(&header[..21])?,
            game_code,
        })
    }

    fn canonicalize(&self, rom: &[u8]) -> Option<Vec<u8>> {
        let (has_copier_header, cartridge) = cartridge_data(rom)?;
        if has_copier_header && find_header(cartridge).is_some() {
            Some(cartridge.to_vec())
        } else {
            None
        }
    }
}

fn cartridge_data(rom: &[u8]) -> Option<(bool, &[u8])> {
    match rom.len() % BANK_SIZE {
        0 if rom.len() >= BANK_SIZE => Some((false, rom)),
        COPIER_HEADER_SIZE if rom.len() >= BANK_SIZE + COPIER_HEADER_SIZE => {
            Some((true, &rom[COPIER_HEADER_SIZE..]))
        }
        _ => None,
    }
}

fn find_header(rom: &[u8]) -> Option<usize> {
    // Extended images can also contain an older, valid header in the first 4 MiB.
    // Prefer the header used by the extended map when it is present.
    [0x40ffc0, 0x7fc0, 0xffc0]
        .into_iter()
        .find(|&offset| valid_header(rom, offset))
}

fn valid_header(rom: &[u8], offset: usize) -> bool {
    let Some(header) = rom.get(offset..offset + 0x40) else {
        return false;
    };
    let map_mode = header[0x15] & !0x10; // FastROM uses the same layout.
    let mapping_matches = match offset {
        0x7fc0 => matches!(map_mode, 0x20 | 0x22 | 0x23),
        0xffc0 => matches!(map_mode, 0x21 | 0x2a),
        0x40ffc0 => map_mode == 0x25,
        _ => false,
    };
    if !mapping_matches || title_text(&header[..21]).is_none() {
        return false;
    }
    let complement = u16::from_le_bytes([header[0x1c], header[0x1d]]);
    let checksum = u16::from_le_bytes([header[0x1e], header[0x1f]]);
    let reset_vector = u16::from_le_bytes([header[0x3c], header[0x3d]]);
    if complement ^ checksum != 0xffff || reset_vector < 0x8000 {
        return false;
    }
    // The declared size rounds up to a power of two; a 3 MiB dump declares 4 MiB.
    // Constrain the shift before evaluating it, and reject truncated full banks.
    let size_code = header[0x17];
    if !(5..=13).contains(&size_code) || header[0x18] > 9 {
        return false;
    }
    let declared_size = 1024usize << size_code;
    if rom.len() > declared_size || rom.len() <= declared_size / 2 {
        return false;
    }
    // Bank $00's reset address must resolve to an instruction in this image.
    let reset_offset = (offset & !0x7fff) | usize::from(reset_vector & 0x7fff);
    rom.get(reset_offset)
        .is_some_and(|opcode| !matches!(opcode, 0x00 | 0x02 | 0xdb | 0x42 | 0xff))
}

/// The cartridge title uses JIS X 0201: ASCII and half-width katakana.
fn title_text(bytes: &[u8]) -> Option<String> {
    let mut title = String::new();
    let mut ended = false;
    for &byte in bytes {
        if byte == 0 || byte == 0xff {
            ended = true;
        } else if ended {
            if byte != b' ' {
                return None;
            }
        } else if (0x20..=0x7e).contains(&byte) {
            title.push(char::from(byte));
        } else if (0xa1..=0xdf).contains(&byte) {
            title.push(char::from_u32(0xff61 + u32::from(byte - 0xa1))?);
        } else {
            return None;
        }
    }
    let title = title.trim().to_owned();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_rom(offset: usize, mode: u8, len: usize) -> Vec<u8> {
        let mut rom = vec![0xea; len];
        rom[offset - 14..offset - 10].copy_from_slice(b"ABCE");
        let header = &mut rom[offset..offset + 0x40];
        header[..21].fill(b' ');
        header[..9].copy_from_slice(b"TEST GAME");
        header[0x15] = mode;
        header[0x17] = (len.next_power_of_two() / 1024).ilog2() as u8;
        header[0x18] = 0;
        header[0x1a] = 0x33;
        header[0x1c..0x20].copy_from_slice(&[0xcb, 0xed, 0x34, 0x12]);
        header[0x3c..0x3e].copy_from_slice(&0x8000u16.to_le_bytes());
        rom
    }

    #[test]
    fn identifies_standard_and_coprocessor_mapping_modes() {
        for (offset, mode, len) in [
            (0x7fc0, 0x20, 0x8000),
            (0x7fc0, 0x30, 0x10000),
            (0x7fc0, 0x32, 0x10000),
            (0x7fc0, 0x23, 0x10000),
            (0xffc0, 0x21, 0x10000),
            (0xffc0, 0x31, 0x10000),
            (0xffc0, 0x3a, 0x300000),
            (0x40ffc0, 0x35, 0x600000),
        ] {
            let info = Snes.identify(&fake_rom(offset, mode, len)).unwrap();
            assert_eq!(info.platform, "snes");
            assert_eq!(info.title, "TEST GAME");
            assert_eq!(info.game_code, "ABCE");
        }
    }

    #[test]
    fn copier_header_normalizes_to_identical_cartridge_bytes() {
        let rom = fake_rom(0x7fc0, 0x20, 0x8000);
        let mut headered = vec![0x57; COPIER_HEADER_SIZE];
        headered.extend_from_slice(&rom);
        assert_eq!(Snes.identify(&headered), Snes.identify(&rom));
        assert_eq!(Snes.canonicalize(&headered), Some(rom.clone()));
        assert_eq!(Snes.canonicalize(&rom), None);
        assert!(Snes.canonicalize(&vec![0; 0x8200]).is_none());
    }

    #[test]
    fn old_header_has_no_fabricated_game_code() {
        let mut rom = fake_rom(0x7fc0, 0x20, 0x8000);
        rom[0x7fda] = 1;
        assert_eq!(Snes.identify(&rom).unwrap().game_code, "");
    }

    #[test]
    fn decodes_half_width_katakana_title() {
        let mut rom = fake_rom(0x7fc0, 0x20, 0x8000);
        rom[0x7fc0..0x7fd5].fill(b' ');
        rom[0x7fc0..0x7fc3].copy_from_slice(&[0xc3, 0xbd, 0xc4]);
        assert_eq!(Snes.identify(&rom).unwrap().title, "ﾃｽﾄ");
    }

    #[test]
    fn rejects_malformed_headers_and_truncated_images() {
        let valid = fake_rom(0x7fc0, 0x20, 0x10000);
        for (offset, byte) in [
            (0x7fd5, 0x21), // HiROM map at the LoROM location.
            (0x7fdc, 0),    // Invalid complement.
            (0x7ffd, 0),    // Reset points into RAM.
            (0x7fd7, 0xff), // Invalid size, must never cause a shift panic.
            (0x7fc0, 1),    // Non-text title.
            (0, 0),         // Reset instruction is BRK.
        ] {
            let mut rom = valid.clone();
            rom[offset] = byte;
            assert!(Snes.identify(&rom).is_none(), "offset {offset:x}");
        }
        assert!(Snes.identify(&valid[..0x8000]).is_none());
        assert!(Snes.identify(&valid[..valid.len() - 1]).is_none());
        assert!(Snes.identify(&[]).is_none());
        assert!(Snes.identify(&vec![0xff; 0x10000]).is_none());
    }
}

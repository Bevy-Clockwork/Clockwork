//! Nintendo 64 platform module.
//!
//! ROM byte order, MIO0 compression, texels, and Fast3D display lists.

pub mod f3d;
pub mod mio0;
pub mod texture;

use clockwork_core::platform::{Platform, RomInfo, header_text};

const HEADER_LEN: usize = 0x40;
const TITLE: std::ops::Range<usize> = 0x20..0x34;
const GAME_CODE: std::ops::Range<usize> = 0x3B..0x3F;

/// The three byte orders N64 ROM dumps come in, detected from the first four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `.z64`: native big-endian.
    BigEndian,
    /// `.v64`: 16-bit byte-swapped.
    ByteSwapped,
    /// `.n64`: 32-bit little-endian.
    LittleEndian,
}

impl ByteOrder {
    pub fn detect(rom: &[u8]) -> Option<Self> {
        match rom.get(..4)? {
            [0x80, 0x37, 0x12, 0x40] => Some(Self::BigEndian),
            [0x37, 0x80, 0x40, 0x12] => Some(Self::ByteSwapped),
            [0x40, 0x12, 0x37, 0x80] => Some(Self::LittleEndian),
            _ => None,
        }
    }
}

/// Converts a ROM of any byte order to big-endian. Trailing bytes that don't fill a word are kept.
pub fn to_big_endian(rom: &[u8], order: ByteOrder) -> Vec<u8> {
    let mut out = rom.to_vec();
    match order {
        ByteOrder::BigEndian => {}
        ByteOrder::ByteSwapped => out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .for_each(|c| c.swap(0, 1)),
        ByteOrder::LittleEndian => out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|c| c.reverse()),
    }
    out
}

pub struct N64;

impl Platform for N64 {
    fn id(&self) -> &'static str {
        "n64"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        if rom.len() < HEADER_LEN {
            return None;
        }
        let order = ByteOrder::detect(rom)?;
        let header = to_big_endian(&rom[..HEADER_LEN], order);
        Some(RomInfo {
            platform: self.id(),
            title: header_text(&header[TITLE]),
            game_code: header_text(&header[GAME_CODE]),
        })
    }

    fn canonicalize(&self, rom: &[u8]) -> Option<Vec<u8>> {
        match ByteOrder::detect(rom)? {
            ByteOrder::BigEndian => None,
            order => Some(to_big_endian(rom, order)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_z64() -> Vec<u8> {
        let mut rom = vec![0x20u8; 0x40];
        rom[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        rom[0x20..0x28].copy_from_slice(b"TESTGAME");
        rom[0x3B..0x3F].copy_from_slice(b"NABE");
        rom
    }

    #[test]
    fn identifies_all_byte_orders() {
        let z64 = fake_z64();
        let v64: Vec<u8> = z64.chunks(2).flat_map(|c| [c[1], c[0]]).collect();
        let n64: Vec<u8> = z64
            .chunks(4)
            .flat_map(|c| [c[3], c[2], c[1], c[0]])
            .collect();

        for rom in [&z64, &v64, &n64] {
            let info = N64.identify(rom).unwrap();
            assert_eq!(info.title, "TESTGAME");
            assert_eq!(info.game_code, "NABE");
        }
    }

    #[test]
    fn rejects_unknown_magic() {
        assert!(N64.identify(&[0u8; 0x40]).is_none());
    }
}

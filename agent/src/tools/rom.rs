//! Reading the ROM: hex dumps, byte searches and decompression.

use std::fmt::Write;

use clockwork_core::recipe::Codec;
use clockwork_gba::lz77;

/// Largest region `read` returns at once.
pub const MAX_READ: usize = 4096;

#[derive(Clone, Copy)]
pub enum View {
    Hex,
    U16,
    U32,
}

impl View {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("hex") {
            "hex" => Ok(View::Hex),
            "u16" => Ok(View::U16),
            "u32" => Ok(View::U32),
            other => Err(format!("Unknown view \"{other}\"; use hex, u16 or u32")),
        }
    }
}

/// A dump of `rom[offset..offset + length]`, 16 bytes per line with offsets.
pub fn read(rom: &[u8], offset: usize, length: usize, view: View) -> Result<String, String> {
    let length = length.min(MAX_READ);
    let data = rom
        .get(offset..offset.saturating_add(length).min(rom.len()))
        .filter(|d| !d.is_empty())
        .ok_or_else(|| {
            format!(
                "Offset {offset:#x} is outside the ROM ({:#x} bytes)",
                rom.len()
            )
        })?;
    let mut out = String::new();
    for (i, line) in data.chunks(16).enumerate() {
        let _ = write!(out, "{:08X}:", offset + i * 16);
        match view {
            View::Hex => line.iter().for_each(|b| {
                let _ = write!(out, " {b:02X}");
            }),
            View::U16 => line.chunks(2).for_each(|c| {
                let v = c.iter().rev().fold(0u32, |acc, &b| acc << 8 | b as u32);
                let _ = write!(out, " {v:04X}");
            }),
            View::U32 => line.chunks(4).for_each(|c| {
                let v = c.iter().rev().fold(0u32, |acc, &b| acc << 8 | b as u32);
                let _ = write!(out, " {v:08X}");
            }),
        }
        out.push('\n');
    }
    Ok(out)
}

/// Parses "10 0A ff" or "100aff" into bytes.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return Err("Give an even number of hex digits, e.g. \"80 37 12 40\"".into());
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Offsets where `needle` occurs, optionally only at offsets divisible by `align`.
pub fn search(rom: &[u8], needle: &[u8], align: usize, max: usize) -> Vec<usize> {
    let align = align.max(1);
    rom.windows(needle.len())
        .enumerate()
        .filter(|(i, w)| i % align == 0 && *w == needle)
        .map(|(i, _)| i)
        .take(max)
        .collect()
}

/// Decompresses at `offset` and describes the result.
pub fn decompress(rom: &[u8], offset: usize, codec: Codec) -> Result<String, String> {
    let data = rom
        .get(offset..)
        .ok_or_else(|| format!("Offset {offset:#x} is outside the ROM"))?;
    let out = match codec {
        Codec::GbaLz77 => lz77::decompress(data).map_err(|e| e.to_string())?,
        Codec::Popuri => clockwork_gba::popuri::decompress(data)?,
        Codec::Mio0 => clockwork_n64::mio0::decompress(data).map_err(|e| e.to_string())?,
        Codec::None => return Err("Codec None needs no decompression; use read_rom".into()),
    };
    let preview = read(&out, 0, 64, View::Hex).unwrap_or_default();
    Ok(format!(
        "{} bytes after decompression ({} 32-byte 4bpp tiles, {} u16 values).\nFirst bytes:\n{preview}",
        out.len(),
        out.len() / 32,
        out.len() / 2
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_and_searches() {
        let rom: Vec<u8> = (0u8..40).collect();
        assert!(
            read(&rom, 16, 4, View::Hex)
                .unwrap()
                .starts_with("00000010: 10 11 12 13")
        );
        assert!(read(&rom, 0, 4, View::U32).unwrap().contains("03020100"));
        assert_eq!(search(&rom, &[4, 5], 1, 10), [4]);
        assert_eq!(search(&rom, &[5, 6], 4, 10), Vec::<usize>::new());
        assert_eq!(parse_hex("10 0a FF").unwrap(), [0x10, 0x0A, 0xFF]);
        assert!(read(&rom, 100, 4, View::Hex).is_err());
    }
}

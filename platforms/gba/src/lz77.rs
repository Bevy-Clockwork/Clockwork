//! The GBA BIOS LZ77 format (`LZ77UnCompWram`, header byte 0x10), used for most compressed
//! graphics and maps.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Lz77Error {
    #[error("not LZ77 data (header byte {0:#04x}, expected 0x10)")]
    BadHeader(u8),
    #[error("LZ77 data ends early")]
    Truncated,
    #[error("LZ77 back-reference points before the start of the output")]
    BadReference,
}

/// Decompresses LZ77 data starting at the beginning of `data`. Trailing bytes are ignored.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, Lz77Error> {
    let header = data.get(..4).ok_or(Lz77Error::Truncated)?;
    if header[0] != 0x10 {
        return Err(Lz77Error::BadHeader(header[0]));
    }
    let size = u32::from_le_bytes([header[1], header[2], header[3], 0]) as usize;
    let mut out = Vec::with_capacity(size);
    let mut pos = 4;
    let mut next = || {
        let b = *data.get(pos).ok_or(Lz77Error::Truncated)?;
        pos += 1;
        Ok::<u8, Lz77Error>(b)
    };

    while out.len() < size {
        let flags = next()?;
        for bit in (0..8).rev() {
            if out.len() >= size {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(next()?);
            } else {
                let b0 = next()?;
                let b1 = next()?;
                let len = (b0 >> 4) as usize + 3;
                let distance = (((b0 & 0xF) as usize) << 8 | b1 as usize) + 1;
                let start = out
                    .len()
                    .checked_sub(distance)
                    .ok_or(Lz77Error::BadReference)?;
                // Copy byte by byte: the source may overlap what is being written.
                for i in 0..len.min(size - out.len()) {
                    out.push(out[start + i]);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_overlapping_copy() {
        // "ab" as literals, then copy 6 bytes from distance 2 -> "abababab".
        let data = [0x10, 8, 0, 0, 0b0010_0000, b'a', b'b', 0x30, 0x01];
        assert_eq!(decompress(&data).unwrap(), b"abababab");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            decompress(&[0x11, 1, 0, 0]),
            Err(Lz77Error::BadHeader(0x11))
        );
        assert_eq!(decompress(&[0x10, 4, 0, 0, 0]), Err(Lz77Error::Truncated));
        assert_eq!(
            decompress(&[0x10, 4, 0, 0, 0b1000_0000, 0x10, 0x05]),
            Err(Lz77Error::BadReference)
        );
    }
}

//! MIO0's separate layout, back-reference, and literal streams.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Mio0Error {
    #[error("not MIO0 data")]
    BadMagic,
    #[error("MIO0 data ends early")]
    Truncated,
    #[error("MIO0 back-reference points before the output")]
    BadReference,
}

/// Decompresses one block, ignoring trailing bytes and allowing overlapping copies.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, Mio0Error> {
    let magic = data.get(..4).ok_or(Mio0Error::Truncated)?;
    if magic != b"MIO0" {
        return Err(Mio0Error::BadMagic);
    }
    let word = |p: usize| -> Result<u32, Mio0Error> {
        Ok(u32::from_be_bytes(
            data.get(p..p + 4)
                .ok_or(Mio0Error::Truncated)?
                .try_into()
                .unwrap(),
        ))
    };
    let size = word(4)? as usize;
    let mut compressed = word(8)? as usize;
    let mut raw = word(12)? as usize;
    let mut layout = 16;
    let mut out = Vec::new();
    while out.len() < size {
        let flags = word(layout)?;
        layout += 4;
        for bit in (0..32).rev() {
            if out.len() == size {
                break;
            }
            if flags & (1 << bit) != 0 {
                out.push(*data.get(raw).ok_or(Mio0Error::Truncated)?);
                raw += 1;
            } else {
                let bytes = data
                    .get(compressed..compressed + 2)
                    .ok_or(Mio0Error::Truncated)?;
                compressed += 2;
                let v = u16::from_be_bytes(bytes.try_into().unwrap());
                let len = (v >> 12) as usize + 3;
                let start = out
                    .len()
                    .checked_sub((v & 0xfff) as usize + 1)
                    .ok_or(Mio0Error::BadReference)?;
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
    fn block(size: u32, flags: u32, refs: &[u8], raw: &[u8]) -> Vec<u8> {
        let mut v = b"MIO0".to_vec();
        for w in [size, 20, 20 + refs.len() as u32, flags] {
            v.extend(w.to_be_bytes());
        }
        v.extend(refs);
        v.extend(raw);
        v
    }
    #[test]
    fn overlap_and_output_limit() {
        assert_eq!(
            decompress(&block(8, 0xc0000000, &[0x30, 1], b"ab")).unwrap(),
            b"abababab"
        );
        assert_eq!(
            decompress(&block(4, 0x80000000, &[0xf0, 0], b"a")).unwrap(),
            b"aaaa"
        );
    }
    #[test]
    fn layout_words() {
        let mut v = block(33, u32::MAX, &0x80000000u32.to_be_bytes(), &[7; 33]);
        v[12..16].copy_from_slice(&24u32.to_be_bytes());
        assert_eq!(decompress(&v).unwrap(), vec![7; 33]);
    }
    #[test]
    fn malformed() {
        assert_eq!(decompress(b"NOPE"), Err(Mio0Error::BadMagic));
        assert_eq!(decompress(b"MIO0"), Err(Mio0Error::Truncated));
        assert_eq!(
            decompress(&block(1, 0, &[0, 0], &[])),
            Err(Mio0Error::BadReference)
        );
        assert_eq!(
            decompress(&block(1, 0x80000000, &[], &[])),
            Err(Mio0Error::Truncated)
        );
        assert_eq!(
            decompress(&block(1, 0, &[0], &[])),
            Err(Mio0Error::Truncated)
        );
    }
}

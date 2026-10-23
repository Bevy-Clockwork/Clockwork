//! Popuri 0x70 streams: MSB-first bits in LE words, optional Huffman atoms,
//! five window encodings, and an optional delta post-filter. No game offsets.

type Result<T> = std::result::Result<T, String>;

struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<usize> {
        let mut value = 0;
        for _ in 0..n {
            let word = self.bit / 32 * 4;
            let byte = word + 3 - (self.bit % 32) / 8;
            let b = *self.data.get(byte).ok_or("truncated Popuri bitstream")?;
            value = (value << 1) | ((b >> (7 - self.bit % 8)) & 1) as usize;
            self.bit += 1;
        }
        Ok(value)
    }
}

struct Atoms<'a> {
    bits: Bits<'a>,
    width: usize,
    tree: Vec<[usize; 2]>,
}
const LEAF: usize = 1 << 30;
impl<'a> Atoms<'a> {
    fn new(mut bits: Bits<'a>, format: usize) -> Result<Self> {
        let width = match format {
            0 => 0,
            1 => 4,
            2 => 8,
            _ => return Err("bad Popuri atom format".into()),
        };
        let mut tree = vec![[usize::MAX; 2]];
        if width != 0 {
            let (mut path, mut total) = (0usize, 0usize);
            for depth in 0..width * 2 {
                let count = bits.read(width)?;
                total += count;
                if total > 1 << width {
                    return Err("too many Popuri Huffman symbols".into());
                }
                path <<= 1;
                for _ in 0..count {
                    if path >= 1 << (depth + 1) {
                        return Err("oversubscribed Popuri Huffman tree".into());
                    }
                    let mut node = 0;
                    for bit in (1..=depth).rev() {
                        let side = (path >> bit) & 1;
                        if tree[node][side] == usize::MAX {
                            tree[node][side] = tree.len();
                            tree.push([usize::MAX; 2]);
                        }
                        node = tree[node][side];
                        if node & LEAF != 0 {
                            return Err("overlapping Popuri Huffman codes".into());
                        }
                    }
                    let side = path & 1;
                    if tree[node][side] != usize::MAX {
                        return Err("duplicate Popuri Huffman code".into());
                    }
                    tree[node][side] = LEAF | bits.read(width)?;
                    path += 1;
                }
            }
            if tree.iter().any(|node| node.contains(&usize::MAX)) {
                return Err("incomplete Popuri Huffman tree".into());
            }
        }
        Ok(Self { bits, width, tree })
    }
    fn symbol(&mut self) -> Result<u8> {
        let mut node = 0;
        for _ in 0..=16 {
            let side = self.bits.read(1)?;
            node = self.tree.get(node).ok_or("invalid Popuri Huffman node")?[side];
            if node & LEAF != 0 {
                return Ok((node & !LEAF) as u8);
            }
        }
        Err("excessive Popuri Huffman depth".into())
    }
    fn byte(&mut self) -> Result<u8> {
        match self.width {
            0 => Ok(self.bits.read(8)? as u8),
            4 => Ok((self.symbol()? << 4) | self.symbol()?),
            _ => self.symbol(),
        }
    }
    fn vli(&mut self, width: usize) -> Result<usize> {
        let mut value = 0usize;
        for _ in 0..16 {
            let atom = self.bits.read(width)?;
            value = value
                .checked_mul(1 << (width - 1))
                .and_then(|v| v.checked_add(atom >> 1))
                .ok_or("Popuri integer overflow")?;
            if atom & 1 == 0 {
                return Ok(value);
            }
        }
        Err("excessive Popuri variable integer".into())
    }
}

fn copy(out: &mut Vec<u8>, distance: usize, count: usize, size: usize) -> Result<()> {
    if distance == 0 || distance > out.len() {
        return Err("invalid Popuri back-reference".into());
    }
    if count > size - out.len() {
        return Err("Popuri output overflow".into());
    }
    for _ in 0..count {
        out.push(out[out.len() - distance]);
    }
    Ok(())
}

/// Decode a complete stream, rejecting truncation, invalid tables and size overruns.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let header = data.get(..4).ok_or("truncated Popuri header")?;
    if header[0] != 0x70 {
        return Err("not a Popuri 0x70 stream".into());
    }
    let size = (u32::from_le_bytes(header.try_into().unwrap()) >> 8) as usize;
    if size > 0x40000 {
        return Err("Popuri output exceeds 256 KiB".into());
    }
    let mut bits = Bits {
        data: &data[4..],
        bit: 0,
    };
    let format = bits.read(8)?;
    let (lz, atom, delta) = (format & 7, (format >> 3) & 3, format >> 5);
    if lz > 4 || delta > 4 {
        return Err("unsupported Popuri format".into());
    }
    let mut read = Atoms::new(bits, atom)?;
    let mut ladder = Vec::new();
    let mut base = 1;
    for _ in 0..[2, 4, 7, 3, 0][lz] {
        let width = read.bits.read(4)? + 1;
        ladder.push((base, width));
        base += 1 << width;
    }
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        if lz == 4 {
            out.push(read.byte()?);
            continue;
        }
        if lz == 0 {
            let method = read.bits.read(2)?;
            if method >= 2 {
                let count = read.bits.read(6)? + if method == 2 { 1 } else { 2 };
                if count > size - out.len() {
                    return Err("Popuri output overflow".into());
                }
                if method == 3 {
                    let value = read.bits.read(8)? as u8;
                    out.resize(out.len() + count, value);
                } else {
                    for _ in 0..count {
                        out.push(read.byte()?);
                    }
                }
            } else {
                let (base, width) = ladder[method];
                let distance = base + read.bits.read(width)?;
                let count = read.bits.read(6)? + 3;
                copy(&mut out, distance, count, size)?;
            }
            continue;
        }
        let unit = if lz == 3 { 2 } else { 1 };
        if read.bits.read(1)? == 0 {
            if unit > size - out.len() {
                return Err("Popuri output overflow".into());
            }
            for _ in 0..unit {
                out.push(read.byte()?);
            }
            continue;
        }
        let index_bits = if lz == 2 { 3 } else { 2 };
        let length_bits = if lz == 3 { 3 } else { 4 };
        let minimum = if lz == 3 { 2 } else { 3 };
        let mut index = read.bits.read(index_bits)?;
        let mut extra = 0;
        if index == ladder.len() {
            let count = read.vli(length_bits)?;
            if read.bits.read(1)? == 0 {
                let count = count
                    .checked_add(1)
                    .and_then(|n| n.checked_mul(unit))
                    .ok_or("Popuri count overflow")?;
                if count > size - out.len() {
                    return Err("Popuri output overflow".into());
                }
                for _ in 0..count {
                    out.push(read.byte()?);
                }
                continue;
            }
            index = read.bits.read(index_bits)?;
            extra = count
                .checked_mul(1 << length_bits)
                .ok_or("Popuri length overflow")?;
        }
        let &(base, width) = ladder.get(index).ok_or("invalid Popuri ladder index")?;
        let distance = (base + read.bits.read(width)?) * unit;
        let count = (minimum + read.bits.read(length_bits)?)
            .checked_add(extra)
            .and_then(|v| v.checked_mul(unit))
            .ok_or("Popuri length overflow")?;
        copy(&mut out, distance, count, size)?;
    }
    match delta {
        0 => (),
        1 => {
            let mut sum = 0u8;
            for byte in &mut out {
                sum = sum.wrapping_add(*byte >> 4) & 15;
                let hi = sum;
                sum = sum.wrapping_add(*byte & 15) & 15;
                *byte = (hi << 4) | sum;
            }
        }
        2 => {
            let mut sum = 0u8;
            for byte in &mut out {
                sum = sum.wrapping_add(*byte);
                *byte = sum;
            }
        }
        3 | 4 => {
            if out.len() % 2 != 0 {
                return Err("odd Popuri word-filter output".into());
            }
            let (mut sum, mut lo, mut hi) = (0u16, 0u8, 0u8);
            for pair in out.as_chunks_mut::<2>().0 {
                if delta == 3 {
                    sum = sum.wrapping_add(u16::from_le_bytes([pair[0], pair[1]]));
                    pair.copy_from_slice(&sum.to_le_bytes());
                } else {
                    lo = lo.wrapping_add(pair[0]);
                    hi = hi.wrapping_add(pair[1]);
                    pair[0] = lo;
                    pair[1] = hi;
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(delta: u8, bytes: &[u8]) -> Vec<u8> {
        let mut out = (((bytes.len() as u32) << 8) | 0x70).to_le_bytes().to_vec();
        let mut atoms = vec![(delta << 5) | 4];
        atoms.extend(bytes);
        while atoms.len() % 4 != 0 {
            atoms.push(0);
        }
        for word in atoms.chunks(4) {
            out.extend(word.iter().rev());
        }
        out
    }
    #[test]
    fn raw_and_delta_filters() {
        for (delta, expected) in [
            (0, vec![1, 2, 255, 1]),
            (1, vec![1, 19, 33, 18]),
            (2, vec![1, 3, 2, 3]),
            (3, vec![1, 2, 0, 4]),
            (4, vec![1, 2, 0, 3]),
        ] {
            assert_eq!(decompress(&raw(delta, &[1, 2, 255, 1])).unwrap(), expected);
        }
    }
    #[test]
    fn rejects_invalid_input() {
        assert!(decompress(&[]).is_err());
        assert!(decompress(&[0x10, 0, 0, 0]).is_err());
        let mut stream = raw(0, &[1, 2, 3, 4]);
        stream.truncate(7);
        assert!(decompress(&stream).is_err());
        assert!(decompress(&raw(3, &[1])).is_err());
    }
}

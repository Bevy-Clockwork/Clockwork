//! Packed N64 texels expanded to row-major RGBA8.

use clockwork_core::image::RgbaImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TexFormat {
    Rgba,
    Yuv,
    Ci,
    Ia,
    I,
}

impl TexFormat {
    /// Rejects reserved format fields instead of guessing a texel layout.
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0 => Some(Self::Rgba),
            1 => Some(Self::Yuv),
            2 => Some(Self::Ci),
            3 => Some(Self::Ia),
            4 => Some(Self::I),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TexSize {
    B4,
    B8,
    B16,
    B32,
}

impl TexSize {
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0 => Some(Self::B4),
            1 => Some(Self::B8),
            2 => Some(Self::B16),
            3 => Some(Self::B32),
            _ => None,
        }
    }
    pub(crate) fn bytes(self, pixels: usize) -> Option<usize> {
        match self {
            Self::B4 => Some(pixels.div_ceil(2)),
            Self::B8 => Some(pixels),
            Self::B16 => pixels.checked_mul(2),
            Self::B32 => pixels.checked_mul(4),
        }
    }
}

fn rgba16(bytes: &[u8]) -> [u8; 4] {
    let v = u16::from_be_bytes([bytes[0], bytes[1]]);
    let expand = |n: u16| ((n << 3) | (n >> 2)) as u8;
    [
        expand((v >> 11) & 31),
        expand((v >> 6) & 31),
        expand((v >> 1) & 31),
        if v & 1 != 0 { 255 } else { 0 },
    ]
}

/// Returns `None` for unsupported layouts, short data, or absent palette entries.
pub fn decode(
    fmt: TexFormat,
    siz: TexSize,
    data: &[u8],
    width: u32,
    height: u32,
    palette: Option<&[u8]>,
) -> Option<RgbaImage> {
    use TexFormat::*;
    use TexSize::*;
    if !matches!(
        (fmt, siz),
        (Rgba, B16 | B32) | (Ia, B4 | B8 | B16) | (I | Ci, B4 | B8)
    ) {
        return None;
    }
    let count = (width as usize).checked_mul(height as usize)?;
    data.get(..siz.bytes(count)?)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count.checked_mul(4)?).ok()?;
    for p in 0..count {
        let nibble = || (data[p / 2] >> (if p % 2 == 0 { 4 } else { 0 })) & 15;
        let rgba = match (fmt, siz) {
            (Rgba, B16) => rgba16(&data[p * 2..]),
            (Rgba, B32) => data[p * 4..p * 4 + 4].try_into().unwrap(),
            (Ia, B4) => {
                let n = nibble();
                let i = ((n >> 1) as u16) * 255u16 / 7;
                [i as u8, i as u8, i as u8, (n & 1) * 255]
            }
            (Ia, B8) => {
                let i = (data[p] >> 4) * 17;
                [i, i, i, (data[p] & 15) * 17]
            }
            (Ia, B16) => [data[p * 2], data[p * 2], data[p * 2], data[p * 2 + 1]],
            (I, B4) => [nibble() * 17; 4],
            (I, B8) => [data[p]; 4],
            (Ci, B4 | B8) => {
                let index = if siz == B4 { nibble() } else { data[p] } as usize;
                rgba16(palette?.get(index * 2..index * 2 + 2)?)
            }
            _ => return None,
        };
        pixels.extend(rgba);
    }
    Some(RgbaImage {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use TexFormat::*;
    use TexSize::*;
    #[test]
    fn all_formats() {
        let cases = [
            (
                Rgba,
                B16,
                vec![0xf8, 1, 7, 0xc0],
                vec![255, 0, 0, 255, 0, 255, 0, 0],
            ),
            (
                Rgba,
                B32,
                vec![1, 2, 3, 4, 5, 6, 7, 8],
                vec![1, 2, 3, 4, 5, 6, 7, 8],
            ),
            (
                Ia,
                B4,
                vec![0xf8],
                vec![255, 255, 255, 255, 145, 145, 145, 0],
            ),
            (
                Ia,
                B8,
                vec![0xf0, 0x8f],
                vec![255, 255, 255, 0, 136, 136, 136, 255],
            ),
            (
                Ia,
                B16,
                vec![42, 7, 255, 128],
                vec![42, 42, 42, 7, 255, 255, 255, 128],
            ),
            (I, B4, vec![0x3f], vec![51, 51, 51, 51, 255, 255, 255, 255]),
            (I, B8, vec![3, 128], vec![3, 3, 3, 3, 128, 128, 128, 128]),
            (Ci, B4, vec![0x01], vec![255, 0, 0, 255, 0, 255, 0, 0]),
            (Ci, B8, vec![1, 0], vec![0, 255, 0, 0, 255, 0, 0, 255]),
        ];
        for (fmt, siz, data, want) in cases {
            assert_eq!(
                decode(fmt, siz, &data, 2, 1, Some(&[0xf8, 1, 7, 0xc0]))
                    .unwrap()
                    .pixels,
                want
            );
        }
    }
    #[test]
    fn invalid_and_odd_size() {
        assert!(decode(Yuv, B16, &[0; 4], 2, 1, None).is_none());
        assert!(decode(Rgba, B16, &[0], 1, 1, None).is_none());
        assert!(decode(Ci, B4, &[0], 1, 1, None).is_none());
        assert!(decode(Ci, B8, &[1], 1, 1, Some(&[0, 0])).is_none());
        assert_eq!(
            decode(I, B4, &[0xf0], 1, 1, None).unwrap().pixels,
            vec![255; 4]
        );
    }
}

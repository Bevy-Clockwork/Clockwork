//! Collision triangles stored as a vertex list, then groups of triangles that share a code, read
//! as a [`TriangleLayout`] describes (see [`clockwork_core::recipe::Step::CollisionTriangles`]):
//!
//! ```text
//! [start]                                 optional marker
//! count, count x (x, y, z)                vertices (signed)
//! repeated: code, count, count x (a, b, c [, extra words])
//! end                                     end of the groups (anything after it is not read)
//! ```

use clockwork_core::recipe::TriangleLayout;
use clockwork_core::terrain::{Collision, CollisionTriangle};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CollisionError {
    #[error("collision at {at:#x} does not start with {start:#x}")]
    NotCollision { at: usize, start: u32 },
    #[error("collision data ends early (at byte {0:#x})")]
    Truncated(usize),
    #[error("triangle {triangle} uses vertex {vertex}, but there are only {count}")]
    BadVertex {
        triangle: usize,
        vertex: u32,
        count: usize,
    },
    #[error("code {0:#x} does not fit in 16 bits")]
    BigCode(u32),
}

struct Words<'a> {
    data: &'a [u8],
    at: usize,
    layout: &'a TriangleLayout,
}

impl Words<'_> {
    fn next(&mut self) -> Result<u32, CollisionError> {
        let len = self.layout.word.bytes();
        let bytes = self
            .data
            .get(self.at..self.at + len)
            .ok_or(CollisionError::Truncated(self.at))?;
        self.at += len;
        let fold = |raw: u32, b: &u8| (raw << 8) | *b as u32;
        Ok(if self.layout.big_endian {
            bytes.iter().fold(0, fold)
        } else {
            bytes.iter().rev().fold(0, fold)
        })
    }

    fn signed(&mut self) -> Result<f32, CollisionError> {
        let raw = self.next()?;
        let bits = 8 * self.layout.word.bytes() as u32;
        Ok(if bits < 32 && raw & (1 << (bits - 1)) != 0 {
            raw as i64 - (1i64 << bits)
        } else {
            raw as i32 as i64
        } as f32)
    }
}

/// Reads the vertices and triangles of the collision at byte `at` of `data`.
pub fn parse(data: &[u8], at: usize, layout: &TriangleLayout) -> Result<Collision, CollisionError> {
    let mut words = Words { data, at, layout };
    if let Some(start) = layout.start
        && words.next()? != start
    {
        return Err(CollisionError::NotCollision { at, start });
    }
    let count = words.next()? as usize;
    let mut collision = Collision::default();
    for _ in 0..count {
        let mut v = [0.0; 3];
        for c in &mut v {
            *c = words.signed()?;
        }
        collision.vertices.push(v);
    }
    loop {
        let code = words.next()?;
        if code == layout.end {
            break;
        }
        let short = u16::try_from(code).map_err(|_| CollisionError::BigCode(code))?;
        let triangles = words.next()?;
        let extra = layout.extra.get(&code).copied().unwrap_or(0);
        for _ in 0..triangles {
            let mut vertices = [0u32; 3];
            for v in &mut vertices {
                let index = words.next()?;
                if index as usize >= count {
                    return Err(CollisionError::BadVertex {
                        triangle: collision.triangles.len(),
                        vertex: index,
                        count,
                    });
                }
                *v = index;
            }
            for _ in 0..extra {
                words.next()?;
            }
            collision.triangles.push(CollisionTriangle {
                code: short,
                vertices,
            });
        }
    }
    Ok(collision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::recipe::FieldSize;

    fn bytes(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn layout() -> TriangleLayout {
        TriangleLayout {
            word: FieldSize::U16,
            big_endian: true,
            start: Some(0x0040),
            end: 0x0041,
            extra: [(0x000E, 1)].into(),
        }
    }

    #[test]
    fn reads_vertices_groups_and_extra_words() {
        let data = bytes(&[
            0x0040, 3, // start, three vertices
            0, 0, 0, 100, 0, 0, 0xFFFF, 5, 100, // (0,0,0) (100,0,0) (-1,5,100)
            0x0000, 1, 0, 1, 2, // one triangle of code 0
            0x000E, 1, 2, 1, 0, 7, // one triangle of code 0x0E, with its extra word
            0x0041, 0x0043, 0, // end of the groups, then data that is not read
        ]);
        let c = parse(&data, 0, &layout()).unwrap();
        assert_eq!(
            c.vertices,
            vec![[0.0; 3], [100.0, 0.0, 0.0], [-1.0, 5.0, 100.0]]
        );
        assert_eq!(c.triangles.len(), 2);
        assert_eq!(
            (c.triangles[0].code, c.triangles[0].vertices),
            (0, [0, 1, 2])
        );
        assert_eq!(
            (c.triangles[1].code, c.triangles[1].vertices),
            (0x0E, [2, 1, 0])
        );
    }

    #[test]
    fn little_endian_bytes_without_a_start() {
        let layout = TriangleLayout {
            word: FieldSize::U8,
            big_endian: false,
            start: None,
            end: 0xFF,
            extra: Default::default(),
        };
        let data = [3, 0, 0, 0, 1, 0, 0, 0xFE, 0, 1, 5, 1, 0, 1, 2, 0xFF];
        let c = parse(&data, 0, &layout).unwrap();
        assert_eq!(c.vertices[2], [-2.0, 0.0, 1.0]);
        assert_eq!(c.triangles[0].code, 5);
    }

    #[test]
    fn offsets_and_errors() {
        let mut data = vec![0xAA, 0xBB];
        data.extend(bytes(&[0x0040, 1, 0, 0, 0, 0x0000, 1, 0, 0, 3, 0x0041]));
        assert!(matches!(
            parse(&data, 2, &layout()),
            Err(CollisionError::BadVertex {
                vertex: 3,
                count: 1,
                ..
            })
        ));
        assert_eq!(
            parse(&data, 0, &layout()),
            Err(CollisionError::NotCollision { at: 0, start: 0x40 })
        );
        assert_eq!(
            parse(&bytes(&[0x0040, 1, 0]), 0, &layout()),
            Err(CollisionError::Truncated(6))
        );
    }
}

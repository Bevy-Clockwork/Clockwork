//! Terrain: collision for 3D areas, as triangles that carry surface properties.
//!
//! Games with 3D levels (collision triangles with surface types) describe the ground as a triangle
//! soup instead of a grid of cells. A platform decodes it into a [`Collision`] with the game's own
//! surface codes; the recipe says what the codes mean, exactly as for a cell-based
//! [`SurfaceMap`], and the result is a [`Terrain`] that Play mode can walk on.

use serde::{Deserialize, Serialize};

use crate::surface::{CodeMeaning, SurfaceError, SurfaceExtension, SurfaceMap, SurfaceProperty};

/// Triangles with the game's own surface codes, as a platform decoder reads them. Positions are
/// in the model's units; triangles wind counterclockwise seen from the side they face.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collision {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<CollisionTriangle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionTriangle {
    /// The game's surface code (often the surface type of the triangle's group).
    pub code: u16,
    pub vertices: [u32; 3],
}

/// Where a character starts in an area, in the area's model units and degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub position: [f32; 3],
    /// Facing, around the vertical axis. 0 faces +z and 90 faces +x.
    #[serde(default)]
    pub yaw: f32,
}

/// A triangle with a normal this steep (|y| at most this) is a wall; flatter ones are floors
/// (facing up) or ceilings (facing down).
pub const DEFAULT_FLOOR_NORMAL_Y: f32 = 0.01;

pub fn default_floor_normal_y() -> f32 {
    DEFAULT_FLOOR_NORMAL_Y
}

/// Which way a triangle faces, by its normal (see [`DEFAULT_FLOOR_NORMAL_Y`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Floor,
    Ceiling,
    Wall,
}

/// Collision triangles with their surface meaning, ready to be placed in a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// One surface "cell" per triangle, in triangle order: its kinds hold the properties.
    pub surfaces: SurfaceMap,
    /// See [`DEFAULT_FLOOR_NORMAL_Y`].
    pub floor_normal_y: f32,
    pub spawn: Option<Spawn>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TerrainError {
    #[error("{0}")]
    Surfaces(#[from] SurfaceError),
    #[error("triangle {triangle} uses vertex {vertex}, but there are only {count}")]
    BadVertex {
        triangle: usize,
        vertex: u32,
        count: usize,
    },
}

impl Terrain {
    /// Gives the codes of `collision` their meaning. Unmapped codes are an error (as for
    /// surface maps), so no triangle silently ends up with no meaning.
    pub fn build(
        collision: &Collision,
        meaning: &CodeMeaning,
        extensions: Vec<SurfaceExtension>,
        floor_normal_y: f32,
        spawn: Option<Spawn>,
    ) -> Result<Self, TerrainError> {
        let count = collision.vertices.len();
        for (i, t) in collision.triangles.iter().enumerate() {
            if let Some(&vertex) = t.vertices.iter().find(|&&v| v as usize >= count) {
                return Err(TerrainError::BadVertex {
                    triangle: i,
                    vertex,
                    count,
                });
            }
        }
        let codes: Vec<u16> = collision.triangles.iter().map(|t| t.code).collect();
        let width = (codes.len() as u32).max(1);
        let surfaces = SurfaceMap::build(width, 1, &[("collision", &codes, meaning)], extensions)?;
        Ok(Self {
            vertices: collision.vertices.clone(),
            triangles: collision.triangles.iter().map(|t| t.vertices).collect(),
            surfaces,
            floor_normal_y,
            spawn,
        })
    }

    /// The unit normal of triangle `index` (from its winding), or `None` for one without area.
    pub fn normal(&self, index: usize) -> Option<[f32; 3]> {
        let [a, b, c] = self
            .triangles
            .get(index)?
            .map(|i| self.vertices[i as usize]);
        let (u, v) = (sub(b, a), sub(c, a));
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        (length > 1e-9).then(|| n.map(|c| c / length))
    }

    /// Whether triangle `index` is a floor, ceiling or wall by its normal.
    pub fn facing(&self, index: usize) -> Option<Facing> {
        let y = self.normal(index)?[1];
        Some(if y > self.floor_normal_y {
            Facing::Floor
        } else if y < -self.floor_normal_y {
            Facing::Ceiling
        } else {
            Facing::Wall
        })
    }

    /// What triangle `index` is: its properties with extensions replaced by their fallbacks.
    pub fn properties(&self, index: usize) -> Vec<SurfaceProperty> {
        let kind = self.surfaces.cells.get(index).copied();
        kind.and_then(|k| self.surfaces.kinds.get(k as usize))
            .map(|k| self.surfaces.resolve(&k.properties))
            .unwrap_or_default()
    }

    /// The game-specific codes triangle `index` came from, for inspection: (grid, code).
    pub fn codes(&self, index: usize) -> Vec<(String, u16)> {
        let kind = self.surfaces.cells.get(index).copied();
        kind.and_then(|k| self.surfaces.kinds.get(k as usize))
            .map(|k| k.codes.clone())
            .unwrap_or_default()
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn collision() -> Collision {
        Collision {
            vertices: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            triangles: vec![
                CollisionTriangle {
                    code: 0,
                    vertices: [0, 1, 2],
                },
                CollisionTriangle {
                    code: 0x28,
                    vertices: [0, 1, 2],
                },
            ],
        }
    }

    fn meaning() -> CodeMeaning {
        CodeMeaning {
            codes: BTreeMap::from([
                (0, vec![SurfaceProperty::Walkable]),
                (0x28, vec![SurfaceProperty::Solid]),
            ]),
            other: None,
        }
    }

    #[test]
    fn triangles_get_their_codes_meaning() {
        let terrain = Terrain::build(&collision(), &meaning(), vec![], 0.01, None).unwrap();
        assert_eq!(terrain.properties(0), vec![SurfaceProperty::Walkable]);
        assert_eq!(terrain.properties(1), vec![SurfaceProperty::Solid]);
        assert_eq!(terrain.codes(1), vec![("collision".to_string(), 0x28)]);
    }

    #[test]
    fn facing_follows_the_winding() {
        let mut c = collision();
        // (0,0,0) (0,0,1) (1,0,0) seen from above is counterclockwise: facing up.
        c.triangles[1].vertices = [0, 2, 1];
        c.triangles[1].code = 0;
        let terrain = Terrain::build(&c, &meaning(), vec![], 0.01, None).unwrap();
        assert_eq!(terrain.facing(0), Some(Facing::Floor));
        assert_eq!(terrain.facing(1), Some(Facing::Ceiling));
        c.vertices[1] = [0.0, 1.0, 0.0];
        let terrain = Terrain::build(&c, &meaning(), vec![], 0.01, None).unwrap();
        assert_eq!(terrain.facing(0), Some(Facing::Wall));
        c.vertices[1] = [0.0, 0.0, 0.0];
        let terrain = Terrain::build(&c, &meaning(), vec![], 0.01, None).unwrap();
        assert_eq!(terrain.facing(0), None);
    }

    #[test]
    fn unmapped_codes_and_bad_vertices_are_errors() {
        let mut c = collision();
        c.triangles[0].code = 7;
        assert!(matches!(
            Terrain::build(&c, &meaning(), vec![], 0.01, None),
            Err(TerrainError::Surfaces(SurfaceError::Unmapped {
                code: 7,
                ..
            }))
        ));
        let mut c = collision();
        c.triangles[1].vertices = [0, 1, 9];
        assert!(matches!(
            Terrain::build(&c, &meaning(), vec![], 0.01, None),
            Err(TerrainError::BadVertex {
                triangle: 1,
                vertex: 9,
                count: 3
            })
        ));
    }
}

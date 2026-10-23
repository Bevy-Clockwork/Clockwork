//! Walking on triangle terrain (3D areas): floors under a point and walls around a body.
//!
//! A [`WorldTerrain`] is a [`Terrain`] placed in the world: its triangles in meters, sorted into
//! floors (facing up), ceilings and walls by their normal, like the game's own collision loader
//! does. Triangles wind counterclockwise seen from the side they face. Ceilings are not used yet.

use bevy::math::{Affine3A, Vec2, Vec3};
use clockwork_core::surface::SurfaceProperty;
use clockwork_core::terrain::{Facing, Spawn, Terrain};

use crate::controller::Ground;

/// A floor this far above a body's feet still counts as the ground under it (a body that is
/// falling must not tunnel through thin floors, and steps up are judged by the controller).
pub const FLOOR_REACH: f32 = 1.0;

/// Walls only matter between this height above the feet and the top of the body.
const WALL_LOW: f32 = 0.3;
const WALL_HIGH: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Floor,
    Ceiling,
    Wall,
}

/// What a collision triangle is to a character: for drawing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriangleKind {
    Floor,
    Ceiling,
    Wall,
}

#[derive(Clone, Debug)]
struct Triangle {
    v: [Vec3; 3],
    normal: Vec3,
    kind: Kind,
    /// Standing on it means falling out of the world.
    hole: bool,
    min: Vec3,
    max: Vec3,
}

#[derive(Clone, Debug)]
pub struct WorldTerrain {
    triangles: Vec<Triangle>,
    spawn: Option<(Vec3, f32)>,
}

impl WorldTerrain {
    /// Places `terrain` in the world with `to_world` (model units to meters, with the node's
    /// position, rotation and scale; it must not mirror).
    pub fn new(terrain: &Terrain, to_world: Affine3A) -> Self {
        let points: Vec<Vec3> = terrain
            .vertices
            .iter()
            .map(|v| to_world.transform_point3(Vec3::from(*v)))
            .collect();
        let mut triangles = Vec::with_capacity(terrain.triangles.len());
        for (index, t) in terrain.triangles.iter().enumerate() {
            let v = t.map(|i| points[i as usize]);
            let cross = (v[1] - v[0]).cross(v[2] - v[0]);
            let Some(normal) = cross.try_normalize() else {
                continue; // A sliver with no area is nothing to stand on or run into.
            };
            let properties = terrain.properties(index);
            let has = |p: SurfaceProperty| properties.contains(&p);
            let horizontal = Vec2::new(normal.x, normal.z).length();
            let kind = if has(SurfaceProperty::Solid) && horizontal > 0.1 {
                Kind::Wall
            } else {
                match terrain.facing(index) {
                    Some(Facing::Floor) => Kind::Floor,
                    Some(Facing::Ceiling) => Kind::Ceiling,
                    _ => Kind::Wall,
                }
            };
            triangles.push(Triangle {
                v,
                normal,
                kind,
                hole: has(SurfaceProperty::FallThrough),
                min: v[0].min(v[1]).min(v[2]),
                max: v[0].max(v[1]).max(v[2]),
            });
        }
        let spawn = terrain.spawn.map(|Spawn { position, yaw }| {
            (
                to_world.transform_point3(Vec3::from(position)),
                // Model space is rotated with the node: add the node's turn around y.
                yaw.to_radians() + node_yaw(to_world),
            )
        });
        Self { triangles, spawn }
    }

    /// Every triangle in the world, with what it is, for debug drawing.
    pub fn debug_triangles(&self) -> impl Iterator<Item = ([Vec3; 3], TriangleKind)> + '_ {
        self.triangles.iter().map(|t| {
            (
                t.v,
                match t.kind {
                    Kind::Floor => TriangleKind::Floor,
                    Kind::Ceiling => TriangleKind::Ceiling,
                    Kind::Wall => TriangleKind::Wall,
                },
            )
        })
    }

    /// Where a character starts (position in meters, yaw in radians), if the area says.
    pub fn spawn(&self) -> Option<(Vec3, f32)> {
        self.spawn
    }

    /// The ground under (x, z) for feet at `y`: the highest floor at most [`FLOOR_REACH`] above
    /// them. `None` when there is no floor there, so other areas can decide.
    pub fn floor(&self, x: f32, y: f32, z: f32) -> Option<Ground> {
        let limit = y + FLOOR_REACH;
        let mut best: Option<(f32, bool)> = None;
        for t in &self.triangles {
            if t.kind != Kind::Floor
                || x < t.min.x
                || x > t.max.x
                || z < t.min.z
                || z > t.max.z
                || t.min.y > limit
            {
                continue;
            }
            if !inside(Vec2::new(x, z), t) {
                continue;
            }
            let h =
                t.v[0].y - (t.normal.x * (x - t.v[0].x) + t.normal.z * (z - t.v[0].z)) / t.normal.y;
            if h <= limit && best.is_none_or(|(b, _)| h > b) {
                best = Some((h, t.hole));
            }
        }
        best.map(|(h, hole)| if hole { Ground::Hole } else { Ground::Floor(h) })
    }

    /// How far along the ray (`direction` unit length) the first floor facing it is, if any: for
    /// finding the ground under a cursor.
    pub fn raycast_floor(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let mut best: Option<f32> = None;
        for t in &self.triangles {
            if t.kind != Kind::Floor || t.normal.dot(direction) >= 0.0 {
                continue;
            }
            // Moller-Trumbore.
            let (e1, e2) = (t.v[1] - t.v[0], t.v[2] - t.v[0]);
            let p = direction.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = origin - t.v[0];
            let u = s.dot(p) / det;
            let q = s.cross(e1);
            let v = direction.dot(q) / det;
            if !(0.0..=1.0).contains(&u) || v < 0.0 || u + v > 1.0 {
                continue;
            }
            let distance = e2.dot(q) / det;
            if distance > 0.0 && best.is_none_or(|b| distance < b) {
                best = Some(distance);
            }
        }
        best
    }

    /// Pushes a body of `radius` standing at `position` out of walls: the corrected position.
    pub fn push_out(&self, mut position: Vec3, radius: f32) -> Vec3 {
        for _ in 0..2 {
            for t in &self.triangles {
                if t.kind != Kind::Wall
                    || t.max.y < position.y + WALL_LOW
                    || t.min.y > position.y + WALL_HIGH
                    || position.x < t.min.x - radius
                    || position.x > t.max.x + radius
                    || position.z < t.min.z - radius
                    || position.z > t.max.z + radius
                {
                    continue;
                }
                let p = Vec2::new(position.x, position.z);
                // Walls only push from the side they face, like the game's own.
                let side = Vec2::new(t.normal.x, t.normal.z).normalize_or_zero();
                let nearest = nearest_on_wall(p, t);
                if (p - nearest).dot(side) < -1e-5 {
                    continue;
                }
                let away = p - nearest;
                let distance = away.length();
                if distance >= radius {
                    continue;
                }
                let direction = if distance > 1e-5 {
                    away / distance
                } else {
                    side
                };
                let push = radius - distance;
                position.x += direction.x * push;
                position.z += direction.y * push;
            }
        }
        position
    }
}

/// The turn around the vertical axis that an affine applies (x axis seen from above).
fn node_yaw(to_world: Affine3A) -> f32 {
    let x = to_world.transform_vector3(Vec3::X);
    // Yaw 0 faces +z: the x axis turned by yaw ends up at (cos yaw, -sin yaw).
    (-x.z).atan2(x.x)
}

fn inside(p: Vec2, t: &Triangle) -> bool {
    let a = Vec2::new(t.v[0].x, t.v[0].z);
    let b = Vec2::new(t.v[1].x, t.v[1].z);
    let c = Vec2::new(t.v[2].x, t.v[2].z);
    let side = |p: Vec2, a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
    let (d1, d2, d3) = (side(p, a, b), side(p, b, c), side(p, c, a));
    let negative = d1 < -1e-6 || d2 < -1e-6 || d3 < -1e-6;
    let positive = d1 > 1e-6 || d2 > 1e-6 || d3 > 1e-6;
    !(negative && positive)
}

/// The point of a wall (seen from above) nearest to `p`. A wall seen from above is a segment:
/// its two farthest points.
fn nearest_on_wall(p: Vec2, t: &Triangle) -> Vec2 {
    let pts = t.v.map(|v| Vec2::new(v.x, v.z));
    let mut best = (pts[0], pts[1], 0.0);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        let d = pts[i].distance_squared(pts[j]);
        if d >= best.2 {
            best = (pts[i], pts[j], d);
        }
    }
    let (a, b, length) = best;
    if length < 1e-10 {
        return a;
    }
    let along = (p - a).dot(b - a) / length;
    a + (b - a) * along.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::surface::CodeMeaning;
    use clockwork_core::terrain::{Collision, CollisionTriangle};
    use std::collections::BTreeMap;

    fn terrain(vertices: Vec<[f32; 3]>, triangles: Vec<(u16, [u32; 3])>) -> Terrain {
        let collision = Collision {
            vertices,
            triangles: triangles
                .into_iter()
                .map(|(code, vertices)| CollisionTriangle { code, vertices })
                .collect(),
        };
        let meaning = CodeMeaning {
            codes: BTreeMap::from([
                (0, vec![SurfaceProperty::Walkable]),
                (1, vec![SurfaceProperty::FallThrough]),
            ]),
            other: Some(vec![SurfaceProperty::Solid]),
        };
        Terrain::build(&collision, &meaning, vec![], 0.01, None).unwrap()
    }

    /// A 10 x 10 floor at height 1 (facing up) and a wall at x = 5 from the floor up to 4.
    fn room() -> WorldTerrain {
        let vertices = vec![
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 10.0],
            [10.0, 1.0, 10.0],
            [10.0, 1.0, 0.0],
            [5.0, 1.0, 0.0],
            [5.0, 1.0, 10.0],
            [5.0, 4.0, 10.0],
            [5.0, 4.0, 0.0],
        ];
        let triangles = vec![
            (0, [0, 1, 2]),
            (0, [0, 2, 3]),
            // The wall faces +x.
            (2, [4, 6, 5]),
            (2, [4, 7, 6]),
        ];
        WorldTerrain::new(&terrain(vertices, triangles), Affine3A::IDENTITY)
    }

    #[test]
    fn floor_is_the_highest_one_within_reach() {
        let world = room();
        assert_eq!(world.floor(2.0, 1.0, 2.0), Some(Ground::Floor(1.0)));
        assert_eq!(world.floor(2.0, 0.2, 2.0), Some(Ground::Floor(1.0)));
        // Too far below the floor to reach it.
        assert_eq!(world.floor(2.0, -0.5, 2.0), None);
        assert_eq!(world.floor(20.0, 1.0, 2.0), None);
    }

    #[test]
    fn rays_hit_the_first_floor_facing_them() {
        let world = room();
        let down = Vec3::NEG_Y;
        let t = world.raycast_floor(Vec3::new(2.0, 5.0, 2.0), down).unwrap();
        assert!((t - 4.0).abs() < 1e-4);
        // From below the floor faces away: no hit. Outside the floor: no hit.
        assert_eq!(
            world.raycast_floor(Vec3::new(2.0, -5.0, 2.0), Vec3::Y),
            None
        );
        assert_eq!(world.raycast_floor(Vec3::new(20.0, 5.0, 2.0), down), None);
        // A slanted ray from the side reaches the floor farther along.
        let slant = Vec3::new(1.0, -1.0, 0.0).normalize();
        let t = world
            .raycast_floor(Vec3::new(1.0, 3.0, 2.0), slant)
            .unwrap();
        assert!(((Vec3::new(1.0, 3.0, 2.0) + slant * t).y - 1.0).abs() < 1e-4);
    }

    #[test]
    fn fall_through_floors_are_holes() {
        let vertices = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 4.0], [4.0, 0.0, 4.0]];
        let world = WorldTerrain::new(&terrain(vertices, vec![(1, [0, 1, 2])]), Affine3A::IDENTITY);
        assert_eq!(world.floor(1.0, 0.0, 3.0), Some(Ground::Hole));
    }

    #[test]
    fn walls_push_bodies_out_only_within_their_height_and_radius() {
        let world = room();
        let at = |x: f32, y: f32| world.push_out(Vec3::new(x, y, 5.0), 0.5);
        // Next to the wall on the side it faces: pushed back to the radius. Behind it: free.
        assert!((at(5.2, 1.0).x - 5.5).abs() < 1e-4);
        assert_eq!(at(4.8, 1.0), Vec3::new(4.8, 1.0, 5.0));
        assert_eq!(at(6.0, 1.0), Vec3::new(6.0, 1.0, 5.0));
        // Above the wall nothing blocks.
        assert_eq!(at(5.2, 4.5), Vec3::new(5.2, 4.5, 5.0));
        // Past the wall's end nothing blocks.
        assert_eq!(
            world.push_out(Vec3::new(5.2, 1.0, 11.0), 0.5),
            Vec3::new(5.2, 1.0, 11.0)
        );
    }

    #[test]
    fn placement_scales_and_turns_the_spawn() {
        let mut t = terrain(
            vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            vec![(0, [0, 1, 2])],
        );
        t.spawn = Some(Spawn {
            position: [100.0, 50.0, 0.0],
            yaw: 180.0,
        });
        let placed = WorldTerrain::new(&t, Affine3A::from_scale(Vec3::splat(0.01)));
        let (position, yaw) = placed.spawn().unwrap();
        assert!((position - Vec3::new(1.0, 0.5, 0.0)).length() < 1e-5);
        assert!((yaw - std::f32::consts::PI).abs() < 1e-5);
        // A node turned a quarter turn (yaw +90 degrees: x becomes -z) adds that to the facing.
        let turned = Affine3A::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let (_, yaw) = WorldTerrain::new(&t, turned).spawn().unwrap();
        assert!((yaw - (std::f32::consts::PI + std::f32::consts::FRAC_PI_2)).abs() < 1e-5);
    }
}

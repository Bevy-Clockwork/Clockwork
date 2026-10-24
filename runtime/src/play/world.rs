//! The world a played character moves in: the flat areas of a scene (images with surface maps),
//! its 3D terrains, and the obstacles its behaviors put in the way.

use bevy::math::{Affine3A, Vec2, Vec3};
use clockwork_core::surface::{SurfaceMap, SurfaceProperty};

use super::entities::Entities;
use crate::{
    controller::{Ground, World},
    terrain::WorldTerrain,
};

/// An area of the scene the character can stand on: its image plane and surface map.
pub struct Area<'a> {
    /// From the world to the area's own space, where its image lies centered on the ground.
    pub to_local: Affine3A,
    /// The image's size in meters.
    pub size: Vec2,
    pub height: f32,
    pub map: &'a SurfaceMap,
}

/// What a scene has to stand on: its flat areas and its 3D terrains.
#[derive(Default)]
pub struct Level<'a> {
    pub areas: Vec<Area<'a>>,
    pub terrains: Vec<&'a WorldTerrain>,
}

/// Everything in a scene the played character can stand on or bump into.
pub struct PlayWorld<'a> {
    pub level: &'a Level<'a>,
    /// Obstacles placed in the scene.
    pub entities: &'a Entities,
}

impl World for Level<'_> {
    fn ground(&self, x: f32, z: f32) -> Ground {
        // The highest area under the point decides; outside every area there is nothing.
        let mut best: Option<(f32, Ground)> = None;
        for area in &self.areas {
            let local = area.to_local.transform_point3(Vec3::new(x, area.height, z));
            let u = (local.x + area.size.x / 2.0) / area.size.x;
            let v = (local.z + area.size.y / 2.0) / area.size.y;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let map = area.map;
            let (fx, fy) = (u * map.width as f32, v * map.height as f32);
            let ground = match map.kind_at(fx as u32, fy as u32) {
                None => Ground::Hole,
                Some(kind) => {
                    cell_ground(map, &kind.properties, fx.fract(), fy.fract(), area.height)
                }
            };
            if best.as_ref().is_none_or(|(h, _)| area.height > *h) {
                best = Some((area.height, ground));
            }
        }
        best.map_or(Ground::Hole, |(_, g)| g)
    }

    fn ground_at(&self, x: f32, y: f32, z: f32) -> Ground {
        let flat = self.ground(x, z);
        // The highest floor of any 3D area at the body's height; holes only if there is no floor.
        let mut terrain: Option<Ground> = None;
        for found in self.terrains.iter().filter_map(|t| t.floor(x, y, z)) {
            terrain = match (terrain, found) {
                (Some(Ground::Floor(a)), Ground::Floor(b)) => Some(Ground::Floor(a.max(b))),
                (Some(Ground::Floor(a)), _) => Some(Ground::Floor(a)),
                (_, found) => Some(found),
            };
        }
        match (flat, terrain) {
            (Ground::Wall, _) => Ground::Wall,
            (Ground::Floor(a), Some(Ground::Floor(b))) => Ground::Floor(a.max(b)),
            (Ground::Floor(a), _) => Ground::Floor(a),
            (Ground::Hole, Some(found)) => found,
            (Ground::Hole, None) => Ground::Hole,
        }
    }

    fn push_out(&self, position: Vec3, radius: f32) -> Vec3 {
        self.terrains
            .iter()
            .fold(position, |p, t| t.push_out(p, radius))
    }
}

/// The level, with what the scene's behaviors put in the way.
impl World for PlayWorld<'_> {
    fn ground(&self, x: f32, z: f32) -> Ground {
        if self.entities.blocked(Vec2::new(x, z)) {
            return Ground::Wall;
        }
        self.level.ground(x, z)
    }

    fn ground_at(&self, x: f32, y: f32, z: f32) -> Ground {
        if self.entities.blocked(Vec2::new(x, z)) {
            return Ground::Wall;
        }
        self.level.ground_at(x, y, z)
    }

    fn push_out(&self, position: Vec3, radius: f32) -> Vec3 {
        self.level.push_out(position, radius)
    }
}

/// What a cell is at a point inside it (fx, fy in 0..1 across the cell).
fn cell_ground(
    map: &SurfaceMap,
    properties: &[SurfaceProperty],
    fx: f32,
    fy: f32,
    height: f32,
) -> Ground {
    let properties = map.resolve(properties);
    let mut ground = Ground::Floor(height);
    for p in &properties {
        match p {
            SurfaceProperty::Solid => return Ground::Wall,
            SurfaceProperty::SolidParts(mask) => {
                let bit = (fx >= 0.5) as u8 + 2 * (fy >= 0.5) as u8;
                if mask & (1 << bit) != 0 {
                    return Ground::Wall;
                }
            }
            SurfaceProperty::FallThrough => ground = Ground::Hole,
            _ => {}
        }
    }
    ground
}

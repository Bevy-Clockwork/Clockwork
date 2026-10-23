//! The scene's 3D areas placed in the world: what Play mode walks on, where dropped characters
//! land and which sprites stand up as billboards.

use bevy::{math::Affine3A, prelude::*};
use clockwork_runtime::{controller::Ground, terrain::WorldTerrain};

use crate::{scene::NodeId, state::EditorState, workshop::Extractions};

/// How far from the floor still counts as standing on it, in meters.
pub const STANDING: f32 = 0.5;

struct Entry {
    node: NodeId,
    plugin: String,
    output: String,
    revision: u64,
    to_world: Affine3A,
    terrain: WorldTerrain,
}

/// Every visible 3D area of the scene, kept up to date as nodes move and plugins reload.
#[derive(Resource, Default)]
pub struct SceneTerrains {
    entries: Vec<Entry>,
}

impl SceneTerrains {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn all(&self) -> Vec<&WorldTerrain> {
        self.entries.iter().map(|e| &e.terrain).collect()
    }

    /// The first area's start point (position in meters, yaw in radians), if it has one.
    pub fn spawn(&self) -> Option<(Vec3, f32)> {
        self.entries.iter().find_map(|e| e.terrain.spawn())
    }

    /// The highest floor under `at` (at most a meter above it), in any area.
    pub fn floor_below(&self, at: Vec3) -> Option<f32> {
        self.entries
            .iter()
            .filter_map(|e| match e.terrain.floor(at.x, at.y, at.z) {
                Some(Ground::Floor(h)) => Some(h),
                _ => None,
            })
            .reduce(f32::max)
    }

    /// Whether `at` is on a floor of a 3D area.
    pub fn standing_on(&self, at: Vec3) -> bool {
        self.floor_below(at)
            .is_some_and(|h| (h - at.y).abs() <= STANDING)
    }

    /// Where a ray first meets a floor of any area.
    pub fn raycast_floor(&self, ray: Ray3d) -> Option<Vec3> {
        let direction = Vec3::from(ray.direction);
        self.entries
            .iter()
            .filter_map(|e| e.terrain.raycast_floor(ray.origin, direction))
            .reduce(f32::min)
            .map(|distance| ray.origin + direction * distance)
    }

    /// The node of the area that has a start point, with its position in meters and yaw.
    pub fn spawn_of(&self, node: NodeId) -> Option<(Vec3, f32)> {
        self.entries
            .iter()
            .find(|e| e.node == node)
            .and_then(|e| e.terrain.spawn())
    }

    /// Rebuilds what changed since last time: nodes that appeared, moved or were reloaded.
    pub fn refresh(&mut self, state: &EditorState, extractions: &Extractions) {
        let scene = &state.workshop.scene;
        let mut wanted = Vec::new();
        for id in scene.in_tree_order() {
            let Some(node) = scene.get(id).filter(|n| n.visible) else {
                continue;
            };
            let Some(item) = node.item.as_ref() else {
                continue;
            };
            let Some(model) = extractions
                .get(&item.plugin)
                .and_then(|x| x.model(&item.output))
            else {
                continue;
            };
            if model.terrain.is_none() {
                continue;
            }
            let to_world = scene.world_transform(id).compute_affine()
                * Affine3A::from_scale(Vec3::splat(model.meters_per_unit));
            wanted.push((id, item, to_world));
        }
        let revision = extractions.revision();
        let same = wanted.len() == self.entries.len()
            && wanted
                .iter()
                .zip(&self.entries)
                .all(|((id, item, affine), e)| {
                    e.node == *id
                        && e.plugin == item.plugin
                        && e.output == item.output
                        && e.revision == revision
                        && e.to_world == *affine
                });
        if same {
            return;
        }
        self.entries = wanted
            .into_iter()
            .filter_map(|(node, item, to_world)| {
                let terrain = extractions
                    .get(&item.plugin)?
                    .model(&item.output)?
                    .terrain
                    .as_ref()?;
                Some(Entry {
                    node,
                    plugin: item.plugin.clone(),
                    output: item.output.clone(),
                    revision,
                    to_world,
                    terrain: WorldTerrain::new(terrain, to_world),
                })
            })
            .collect();
    }
}

pub fn refresh_system(
    state: Res<EditorState>,
    extractions: Res<Extractions>,
    mut terrains: ResMut<SceneTerrains>,
) {
    terrains.refresh(&state, &extractions);
}

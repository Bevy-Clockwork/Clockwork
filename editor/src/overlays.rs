//! What the viewport draws on top of the scene to help build and debug: rings for behaviors,
//! health bars while playing, and the collision triangles of 3D areas.

use bevy::prelude::*;
use clockwork_runtime::terrain::TriangleKind;

use crate::{
    entities::Behavior, play::PlaySession, state::EditorState, terrains::SceneTerrains,
    workshop::OrbitCamera,
};

/// Collision triangles farther than this from the camera's focus aren't drawn (meters).
const TERRAIN_RANGE: f32 = 60.0;

fn rgb([r, g, b]: [f32; 3], alpha: f32) -> Color {
    Color::srgba(r, g, b, alpha)
}

fn ring(gizmos: &mut Gizmos, at: Vec3, radius: f32, color: Color) {
    gizmos.circle(
        Isometry3d::new(
            at + Vec3::Y * 0.04,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        ),
        radius,
        color,
    );
}

pub fn overlay_system(
    state: Res<EditorState>,
    session: Res<PlaySession>,
    terrains: Res<SceneTerrains>,
    orbit: Res<OrbitCamera>,
    mut gizmos: Gizmos,
) {
    let ws = &state.workshop;
    let playing = ws.playing.is_some();

    if playing {
        let entities = session.entities();
        for e in &entities.live {
            if e.gone {
                continue;
            }
            let [r, g, b] = e.behavior.color();
            if ws.debug.colliders {
                // Faint while it waits on its condition, its node appearing, or its act.
                let alpha = if e.active { 0.8 } else { 0.2 };
                ring(
                    &mut gizmos,
                    e.at,
                    e.behavior.radius(),
                    rgb([r, g, b], alpha),
                );
            }
            if e.active && e.flash > 0.0 {
                ring(&mut gizmos, e.at, e.behavior.radius() * 1.2, Color::WHITE);
            }
            // A health bar over a target that has been hurt.
            if matches!(e.behavior, Behavior::Target { .. })
                && e.active
                && e.max_health > 0.0
                && e.health < e.max_health
            {
                let fraction = (e.health / e.max_health).clamp(0.0, 1.0);
                let left = e.at + Vec3::new(-0.5, 1.6, 0.0);
                gizmos.line(left, left + Vec3::X, Color::srgba(0.2, 0.2, 0.2, 0.9));
                gizmos.line(
                    left,
                    left + Vec3::X * fraction,
                    Color::srgb(1.0 - fraction, fraction, 0.2),
                );
            }
        }
    } else {
        // While building: every behavior, the selected ones brighter.
        for node in ws.scene.nodes.iter().filter(|n| ws.scene.is_shown(n.id)) {
            let Some(first) = node.behaviors.first() else {
                continue;
            };
            let at = ws.scene.world_transform(node.id).translation;
            let selected = ws.is_selected(node.id);
            for entry in &node.behaviors {
                ring(
                    &mut gizmos,
                    at,
                    entry.behavior.radius(),
                    rgb(entry.behavior.color(), if selected { 1.0 } else { 0.55 }),
                );
            }
            let behavior = &first.behavior;
            if node.item.is_none() {
                // A node that only has a behavior would otherwise be invisible.
                gizmos.line(at, at + Vec3::Y * 1.0, rgb(behavior.color(), 0.9));
                gizmos.cross(
                    Isometry3d::from_translation(at + Vec3::Y * 0.05),
                    0.2,
                    rgb(behavior.color(), 0.9),
                );
            }
        }
    }

    if ws.debug.terrain {
        for terrain in terrains.all() {
            for (triangle, kind) in terrain.debug_triangles() {
                let center = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
                if center.distance(orbit.focus) > TERRAIN_RANGE {
                    continue;
                }
                let color = match kind {
                    TriangleKind::Floor => Color::srgba(0.4, 0.85, 0.4, 0.5),
                    TriangleKind::Wall => Color::srgba(0.95, 0.4, 0.35, 0.6),
                    TriangleKind::Ceiling => Color::srgba(0.4, 0.55, 0.95, 0.4),
                };
                gizmos.linestrip([triangle[0], triangle[1], triangle[2], triangle[0]], color);
            }
        }
    }
}

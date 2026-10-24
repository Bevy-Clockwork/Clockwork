//! What Play draws of the played character's fighting: hit boxes, explosions, its guard and the
//! projectiles that have no sprite.

use bevy::prelude::*;
use clockwork_runtime::actions::Shape;

/// Draws a shape as lines on the ground at `height`.
pub fn draw_shape(gizmos: &mut Gizmos, shape: &Shape, height: f32, color: Color) {
    let at = |p: Vec2| Vec3::new(p.x, height + 0.05, p.y);
    match *shape {
        Shape::Circle { center, radius } => {
            let points = (0..=32).map(|i| {
                let a = std::f32::consts::TAU * i as f32 / 32.0;
                at(center + Vec2::new(a.cos(), a.sin()) * radius)
            });
            gizmos.linestrip(points, color);
        }
        Shape::Box {
            center,
            dir,
            half_along,
            half_across,
        } => {
            let (a, b) = (dir * half_along, dir.perp() * half_across);
            let corners = [
                center + a + b,
                center + a - b,
                center - a - b,
                center - a + b,
                center + a + b,
            ];
            gizmos.linestrip(corners.map(at), color);
        }
    }
}

/// Draws the played character's hit boxes, explosions and projectiles.
pub fn gizmo_system(session: Res<crate::play::PlaySession>, mut gizmos: Gizmos) {
    let Some(fighter) = session.fighter() else {
        return;
    };
    if fighter.hitboxes {
        for flash in &fighter.flashes {
            let [r, g, b] = flash.color;
            let fade = 1.0 - flash.age / flash.lifetime.max(f32::EPSILON);
            draw_shape(
                &mut gizmos,
                &flash.shape,
                flash.height,
                Color::srgba(r, g, b, fade.clamp(0.2, 1.0)),
            );
        }
        if let Some((dir, angle)) = fighter.guard
            && let Some(at) = session.position()
        {
            let spread = angle.to_radians();
            for sign in [-1.0f32, 1.0] {
                let edge = Vec2::from_angle(sign * spread).rotate(dir) * 1.0;
                gizmos.line(
                    Vec3::new(at.x, at.y + 0.05, at.z),
                    Vec3::new(at.x + edge.x, at.y + 0.05, at.z + edge.y),
                    Color::srgb(0.4, 0.7, 1.0),
                );
            }
        }
    }
    // Projectiles with a sprite are drawn by `sprites::projectile_system`.
    for p in fighter.projectiles.iter() {
        if fighter.look_of(&p.id).is_some() && fighter.plugin.is_some() {
            continue;
        }
        let color = fighter
            .color_of(&p.id)
            .map(|[r, g, b]| Color::srgb(r, g, b))
            .unwrap_or(Color::WHITE);
        let position = Vec3::new(p.position.x, p.height + 0.3, p.position.y);
        gizmos.sphere(Isometry3d::from_translation(position), 0.25, color);
    }
}

//! A generic top-down character controller for 2D characters: walks on the ground in the
//! directions the player points, faces the stronger axis, and picks the idle or walk animation
//! of its facing. Configured entirely by [`Character2d`]; pure (no Bevy systems) so it can be
//! tested without a window.
//!
//! The ground plane is x (east) by z (south), as in [`crate::controller`], and the same [`World`]
//! provides collision. Floors of any height are walkable, walls and holes block.
//!
//! [`Character2d`]: clockwork_core::character2d::Character2d

use bevy::math::{Vec2, Vec3};
use clockwork_core::character2d::{AnimationRef, Animations2d, Direction4, Movement2d};

use crate::actions::Modifiers;
use crate::controller::{Ground, World};

/// Points around the character's radius that must all be on the floor.
const PROBES: usize = 8;

/// A top-down character's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Body2d {
    /// x east, y up, z south.
    pub position: Vec3,
    pub facing: Direction4,
    /// Current speed along the input direction.
    pub speed: f32,
    /// Seconds since the current animation (state and facing) started.
    pub time: f32,
    /// Whether the character is walking (input held) rather than standing.
    pub walking: bool,
    /// The way it last walked on the ground plane (unit length), kept while it slows down.
    heading: Vec2,
    last: Option<(bool, Direction4)>,
}

impl Body2d {
    pub fn new(position: Vec3, facing: Direction4) -> Self {
        Self {
            position,
            facing,
            speed: 0.0,
            time: 0.0,
            walking: false,
            heading: Vec2::ZERO,
            last: None,
        }
    }
}

/// Advances the character by `dt` seconds. `direction` is the input on the ground plane (x east,
/// z south), at most unit length; zero means standing still.
pub fn step(
    body: &mut Body2d,
    direction: Vec2,
    movement: &Movement2d,
    world: &impl World,
    dt: f32,
) {
    step_with(body, direction, movement, world, dt, &Modifiers::default());
}

/// [`step`] while an action constrains the character (see [`Modifiers`]).
pub fn step_with(
    body: &mut Body2d,
    direction: Vec2,
    movement: &Movement2d,
    world: &impl World,
    dt: f32,
    modifiers: &Modifiers,
) {
    let input = if modifiers.locked {
        0.0
    } else {
        direction.length()
    };
    body.walking = input > 0.0;
    if let Some(facing) = modifiers.facing {
        body.facing = facing;
    }
    if body.walking {
        // North is up the map, which is -z.
        if modifiers.facing.is_none() {
            body.facing = Direction4::from_input(direction.x, -direction.y, body.facing);
        }
        body.heading = direction.normalize();
        let target = movement.speed * modifiers.speed_factor * input.min(1.0);
        body.speed = match movement.acceleration {
            None => target,
            Some(a) => (body.speed + a * dt).min(target),
        };
    } else {
        body.speed = match movement.acceleration {
            None => 0.0,
            Some(a) => (body.speed - a * dt).max(0.0),
        };
    }

    if let Some(velocity) = modifiers.slide {
        body.walking = false;
        body.speed = velocity.length();
        if let Some(heading) = velocity.try_normalize() {
            body.heading = heading;
        }
    }

    // While slowing down the character keeps its way; at a standstill there is nothing to move.
    if body.speed > 0.0 {
        let delta = body.heading * body.speed * dt;
        let mut position = Vec2::new(body.position.x, body.position.z);
        for attempt in [delta, Vec2::new(delta.x, 0.0), Vec2::new(0.0, delta.y)] {
            if attempt == Vec2::ZERO {
                continue;
            }
            if let Some(floor) = free(position + attempt, movement.radius, world) {
                position += attempt;
                body.position = Vec3::new(position.x, floor, position.y);
                break;
            }
        }
    }

    let key = (body.walking, body.facing);
    if body.last != Some(key) {
        body.last = Some(key);
        body.time = 0.0;
    }
    body.time += dt;
}

/// The floor height if the circle of `radius` around `at` is entirely on floors.
fn free(at: Vec2, radius: f32, world: &impl World) -> Option<f32> {
    let mut height = None;
    for i in 0..=PROBES {
        // The center first (and last, closing the ring), then points around it.
        let probe = if i == 0 {
            at
        } else {
            at + Vec2::from_angle(std::f32::consts::TAU * (i - 1) as f32 / PROBES as f32) * radius
        };
        match world.ground(probe.x, probe.y) {
            Ground::Floor(h) => {
                if i == 0 {
                    height = Some(h);
                }
            }
            Ground::Wall | Ground::Hole => return None,
        }
    }
    height
}

/// What the character plays now: the animation, and how long it has been playing.
pub fn pose<'a>(body: &Body2d, animations: &'a Animations2d) -> (&'a AnimationRef, f32) {
    let directional = if body.walking {
        &animations.walk
    } else {
        &animations.idle
    };
    (directional.get(body.facing), body.time)
}

#[cfg(test)]
mod tests {
    use clockwork_core::character2d::Directional;

    use super::*;

    /// Floor everywhere except a wall at x >= 2 and a hole at z <= -2.
    struct Room;

    impl World for Room {
        fn ground(&self, x: f32, z: f32) -> Ground {
            if x >= 2.0 {
                Ground::Wall
            } else if z <= -2.0 {
                Ground::Hole
            } else {
                Ground::Floor(0.5)
            }
        }
    }

    fn movement(acceleration: Option<f32>) -> Movement2d {
        Movement2d {
            speed: 4.0,
            acceleration,
            radius: 0.25,
            gravity: 9.8,
        }
    }

    fn at(x: f32, z: f32) -> Body2d {
        Body2d::new(Vec3::new(x, 0.0, z), Direction4::South)
    }

    fn walk(body: &mut Body2d, direction: Vec2, movement: &Movement2d, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            step(body, direction, movement, &Room, 1.0 / 60.0);
        }
    }

    #[test]
    fn walks_at_full_speed_without_acceleration() {
        let mut body = at(0.0, 0.0);
        walk(&mut body, Vec2::new(0.0, 1.0), &movement(None), 0.5);
        assert!((body.position.z - 2.0).abs() < 0.05, "{:?}", body.position);
        assert_eq!(body.position.y, 0.5);
        walk(&mut body, Vec2::ZERO, &movement(None), 0.1);
        let stopped = body.position;
        walk(&mut body, Vec2::ZERO, &movement(None), 0.5);
        assert_eq!(body.position, stopped);
        assert!(!body.walking);
    }

    #[test]
    fn accelerates_when_asked_to() {
        let mut body = at(0.0, 0.0);
        walk(&mut body, Vec2::new(0.0, 1.0), &movement(Some(8.0)), 0.25);
        assert!((body.speed - 2.0).abs() < 0.2, "{}", body.speed);
        walk(&mut body, Vec2::new(0.0, 1.0), &movement(Some(8.0)), 2.0);
        assert!((body.speed - 4.0).abs() < 1e-3);
    }

    #[test]
    fn walls_stop_and_slide() {
        // Walking east-south into the wall at x = 2: the east part is lost, the south part stays.
        let mut body = at(1.5, 0.0);
        walk(
            &mut body,
            Vec2::new(1.0, 1.0).normalize(),
            &movement(None),
            1.0,
        );
        assert!(body.position.x + 0.25 < 2.0 + 1e-3, "{:?}", body.position);
        assert!(body.position.z > 1.0, "{:?}", body.position);
    }

    #[test]
    fn holes_block_too() {
        let mut body = at(0.0, -1.5);
        walk(&mut body, Vec2::new(0.0, -1.0), &movement(None), 1.0);
        assert!(body.position.z - 0.25 > -2.0 - 1e-3, "{:?}", body.position);
    }

    #[test]
    fn actions_slide_lock_and_slow_the_character() {
        let m = movement(None);
        // Sliding east at 6 m/s ignores the input and stops at the wall at x = 2.
        let mut body = at(0.0, 0.0);
        let slide = Modifiers {
            locked: true,
            slide: Some(Vec2::new(6.0, 0.0)),
            facing: Some(Direction4::East),
            ..Modifiers::default()
        };
        for _ in 0..30 {
            step_with(
                &mut body,
                Vec2::new(0.0, 1.0),
                &m,
                &Room,
                1.0 / 60.0,
                &slide,
            );
        }
        assert!(body.position.z == 0.0, "{:?}", body.position);
        assert!(
            (body.position.x - (2.0 - 0.25)).abs() < 0.2,
            "{:?}",
            body.position
        );
        assert_eq!(body.facing, Direction4::East);
        // Locked: no walking. Slowed: half speed, and the facing stays.
        let mut body = at(0.0, 0.0);
        let locked = Modifiers {
            locked: true,
            ..Modifiers::default()
        };
        step_with(&mut body, Vec2::new(0.0, 1.0), &m, &Room, 0.5, &locked);
        assert_eq!(body.position, Vec3::new(0.0, 0.0, 0.0));
        let slow = Modifiers {
            speed_factor: 0.5,
            facing: Some(Direction4::North),
            ..Modifiers::default()
        };
        step_with(&mut body, Vec2::new(0.0, 1.0), &m, &Room, 0.5, &slow);
        assert!((body.position.z - 1.0).abs() < 1e-3, "{:?}", body.position);
        assert_eq!(body.facing, Direction4::North);
    }

    #[test]
    fn faces_and_animates() {
        let reference = |name: &str, flip_x| AnimationRef {
            name: name.into(),
            flip_x,
            flip_y: false,
        };
        let directional = |prefix: &str| Directional {
            north: reference(&format!("{prefix}_n"), false),
            east: reference(&format!("{prefix}_e"), false),
            south: reference(&format!("{prefix}_s"), false),
            west: reference(&format!("{prefix}_e"), true),
        };
        let animations = Animations2d {
            idle: directional("idle"),
            walk: directional("walk"),
        };
        let mut body = at(0.0, 0.0);
        assert_eq!(pose(&body, &animations).0.name, "idle_s");
        // West: -x on the ground plane.
        walk(&mut body, Vec2::new(-1.0, 0.0), &movement(None), 0.5);
        let (animation, time) = pose(&body, &animations);
        assert_eq!(
            (animation.name.as_str(), animation.flip_x),
            ("walk_e", true)
        );
        assert!((time - 0.5).abs() < 0.02);
        // North is -z.
        walk(&mut body, Vec2::new(0.0, -1.0), &movement(None), 0.1);
        let (animation, time) = pose(&body, &animations);
        assert_eq!(animation.name, "walk_n");
        // The animation restarted when the facing changed.
        assert!(time < 0.2);
        // Standing keeps the facing.
        walk(&mut body, Vec2::ZERO, &movement(None), 0.1);
        assert_eq!(pose(&body, &animations).0.name, "idle_n");
    }
}

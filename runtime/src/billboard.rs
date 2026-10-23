//! A 2D character in a 3D world (2.5D): an upright billboard that walks on the world's floors.
//!
//! It is the [`crate::topdown`] character with the third dimension added: it follows floors up
//! and down (small steps), is stopped by walls and steps too tall to climb, and falls off ledges
//! under gravity. The input is camera-relative as for 3D characters; its facing animation follows
//! the stick as seen on screen (up the screen is north), since the sprite always faces the camera.
//! Configured entirely by [`Movement2d`] and [`Animations2d`]; pure, so it can be tested without a
//! window.

use bevy::math::{Vec2, Vec3};
use clockwork_core::character2d::{AnimationRef, Animations2d, Direction4, Movement2d};

use crate::actions::Modifiers;
use crate::controller::{Ground, World};

/// The tallest step a character walks up without jumping, in meters.
const STEP_HEIGHT: f32 = 0.3;
/// Points around the character's radius that must be clear of tall steps and walls.
const PROBES: usize = 8;
/// A character this far below where it started has fallen out of the world.
const FALL_LIMIT: f32 = 30.0;
const MAX_FALL_SPEED: f32 = 30.0;

/// A billboard character's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Body25d {
    /// Feet position: x east, y up, z south.
    pub position: Vec3,
    pub vertical: f32,
    pub grounded: bool,
    pub facing: Direction4,
    /// Current speed along its heading.
    pub speed: f32,
    /// Seconds since the current animation (state and facing) started.
    pub time: f32,
    /// Whether input is held, rather than standing.
    pub walking: bool,
    /// Where it respawns after falling out of the world.
    pub start: Vec3,
    heading: Vec2,
    last: Option<(bool, Direction4)>,
}

impl Body25d {
    pub fn new(position: Vec3, facing: Direction4) -> Self {
        Self {
            position,
            vertical: 0.0,
            grounded: true,
            facing,
            speed: 0.0,
            time: 0.0,
            walking: false,
            start: position,
            heading: Vec2::ZERO,
            last: None,
        }
    }
}

/// Advances the character by `dt` seconds. `stick` is the input as seen on screen (x right, y up
/// the screen) and `direction` the same input on the ground (x east, z south): both at most unit
/// length, zero meaning standing still.
pub fn step(
    body: &mut Body25d,
    stick: Vec2,
    direction: Vec2,
    movement: &Movement2d,
    world: &impl World,
    dt: f32,
) {
    step_with(
        body,
        stick,
        direction,
        movement,
        world,
        dt,
        &Modifiers::default(),
    );
}

/// [`step`] while an action constrains the character (see [`Modifiers`]).
pub fn step_with(
    body: &mut Body25d,
    stick: Vec2,
    direction: Vec2,
    movement: &Movement2d,
    world: &impl World,
    dt: f32,
    modifiers: &Modifiers,
) {
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
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
        if modifiers.facing.is_none() {
            body.facing = Direction4::from_input(stick.x, stick.y, body.facing);
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

    if body.speed > 0.0 {
        let delta = body.heading * body.speed * dt;
        for attempt in [delta, Vec2::new(delta.x, 0.0), Vec2::new(0.0, delta.y)] {
            if attempt == Vec2::ZERO {
                continue;
            }
            let wanted = body.position + Vec3::new(attempt.x, 0.0, attempt.y);
            let pushed = world.push_out(wanted, movement.radius);
            if clear(pushed, movement.radius, world) {
                body.position = pushed;
                break;
            }
        }
    }

    let ground = world.ground_at(body.position.x, body.position.y, body.position.z);
    if body.grounded {
        match ground {
            Ground::Floor(h) if (h - body.position.y).abs() <= STEP_HEIGHT => {
                body.position.y = h;
                body.vertical = 0.0;
            }
            _ => {
                body.grounded = false;
                body.vertical = 0.0;
            }
        }
    } else {
        body.vertical = (body.vertical - movement.gravity * dt).max(-MAX_FALL_SPEED);
        body.position.y += body.vertical * dt;
        if let Ground::Floor(h) = ground
            && body.vertical <= 0.0
            && body.position.y <= h
        {
            body.position.y = h;
            body.vertical = 0.0;
            body.grounded = true;
        }
    }
    if body.position.y < body.start.y - FALL_LIMIT {
        let start = body.start;
        *body = Body25d::new(start, body.facing);
        return;
    }

    let key = (body.walking, body.facing);
    if body.last != Some(key) {
        body.last = Some(key);
        body.time = 0.0;
    }
    body.time += dt;
}

/// Whether the circle of `radius` around `at` is free of walls and steps too tall to climb.
/// Holes don't block: the character walks off the edge and falls.
fn clear(at: Vec3, radius: f32, world: &impl World) -> bool {
    let blocked = |x: f32, z: f32| match world.ground_at(x, at.y, z) {
        Ground::Wall => true,
        Ground::Floor(h) => h > at.y + STEP_HEIGHT,
        Ground::Hole => false,
    };
    !blocked(at.x, at.z)
        && (0..PROBES).all(|i| {
            let p = Vec2::from_angle(std::f32::consts::TAU * i as f32 / PROBES as f32) * radius;
            !blocked(at.x + p.x, at.z + p.y)
        })
}

/// What the character plays now: the animation, and how long it has been playing.
pub fn pose<'a>(body: &Body25d, animations: &'a Animations2d) -> (&'a AnimationRef, f32) {
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

    /// Floor at height 0 for z < 0, a ramp rising 0.1 per meter for 0 <= z < 20, a 1 m cliff
    /// face at x = 5 (floor 1 m higher for x >= 5), a wall at x = -5, and no floor for z >= 20.
    struct Land;

    impl World for Land {
        fn ground(&self, x: f32, z: f32) -> Ground {
            self.ground_at(x, 100.0, z)
        }

        fn ground_at(&self, x: f32, y: f32, z: f32) -> Ground {
            if z >= 20.0 {
                return Ground::Hole;
            }
            let base = if z < 0.0 { 0.0 } else { z * 0.1 };
            let h = base + if x >= 5.0 { 1.0 } else { 0.0 };
            if h > y + 1.0 {
                Ground::Hole
            } else {
                Ground::Floor(h)
            }
        }

        fn push_out(&self, mut position: Vec3, radius: f32) -> Vec3 {
            position.x = position.x.max(-5.0 + radius);
            position
        }
    }

    fn movement() -> Movement2d {
        Movement2d {
            speed: 4.0,
            acceleration: None,
            radius: 0.25,
            gravity: 9.8,
        }
    }

    fn walk(body: &mut Body25d, stick: Vec2, direction: Vec2, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            step(body, stick, direction, &movement(), &Land, 1.0 / 60.0);
        }
    }

    #[test]
    fn follows_a_ramp_up_and_stays_grounded() {
        let mut body = Body25d::new(Vec3::new(0.0, 0.0, -1.0), Direction4::South);
        walk(&mut body, Vec2::Y, Vec2::new(0.0, 1.0), 2.0);
        assert!(body.grounded);
        // About 7 m further south: on the ramp at 0.1 per meter.
        assert!((body.position.z - 7.0).abs() < 0.2, "{:?}", body.position);
        assert!((body.position.y - body.position.z * 0.1).abs() < 1e-3);
    }

    #[test]
    fn tall_steps_and_walls_block_and_slide() {
        // East into the 1 m step at x = 5: stopped at the radius, still free to go along it.
        let mut body = Body25d::new(Vec3::new(4.0, 0.0, -5.0), Direction4::South);
        walk(&mut body, Vec2::X, Vec2::new(1.0, 0.0), 1.0);
        assert!(body.position.x + 0.25 <= 5.0 + 1e-3, "{:?}", body.position);
        walk(&mut body, Vec2::ZERO, Vec2::new(1.0, 0.5).normalize(), 0.5);
        assert!(body.position.z > -5.0, "{:?}", body.position);
        // West into the wall: pushed back, at most to its radius.
        let mut body = Body25d::new(Vec3::new(-4.0, 0.0, -5.0), Direction4::South);
        walk(&mut body, -Vec2::X, Vec2::new(-1.0, 0.0), 1.0);
        assert!(body.position.x >= -5.0 + 0.25 - 1e-3, "{:?}", body.position);
    }

    #[test]
    fn walks_off_the_edge_falls_and_respawns() {
        // The ramp is 1.8 m high at z = 18.
        let start = Vec3::new(0.0, 1.8, 18.0);
        let mut body = Body25d::new(start, Direction4::South);
        walk(&mut body, Vec2::Y, Vec2::new(0.0, 1.0), 1.0);
        assert!(!body.grounded, "{:?}", body.position);
        assert!(body.position.z > 20.0 - 0.25);
        walk(&mut body, Vec2::ZERO, Vec2::ZERO, 4.0);
        assert!(
            (body.position - start).length() < 1e-4,
            "{:?}",
            body.position
        );
        assert!(body.grounded);
    }

    #[test]
    fn actions_slide_and_hold_the_facing() {
        let mut body = Body25d::new(Vec3::new(0.0, 0.0, -5.0), Direction4::South);
        let slide = Modifiers {
            locked: true,
            slide: Some(Vec2::new(0.0, -5.0)),
            facing: Some(Direction4::North),
            ..Modifiers::default()
        };
        for _ in 0..60 {
            step_with(
                &mut body,
                Vec2::X,
                Vec2::X,
                &movement(),
                &Land,
                1.0 / 60.0,
                &slide,
            );
        }
        assert_eq!(body.facing, Direction4::North);
        assert!((body.position.z - -10.0).abs() < 0.1, "{:?}", body.position);
        assert!(body.position.x.abs() < 1e-4);
        assert!(!body.walking);
    }

    #[test]
    fn faces_the_stick_as_seen_on_screen() {
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
        let mut body = Body25d::new(Vec3::new(0.0, 0.0, -5.0), Direction4::South);
        assert_eq!(pose(&body, &animations).0.name, "idle_s");
        // The camera may be turned: the stick says screen-left, the ground heading says north.
        walk(&mut body, -Vec2::X, Vec2::new(0.0, -1.0), 0.2);
        let (animation, time) = pose(&body, &animations);
        assert_eq!(
            (animation.name.as_str(), animation.flip_x),
            ("walk_e", true)
        );
        assert!(time < 0.25);
        walk(&mut body, Vec2::ZERO, Vec2::ZERO, 0.1);
        assert_eq!(pose(&body, &animations).0.name, "idle_e");
    }
}

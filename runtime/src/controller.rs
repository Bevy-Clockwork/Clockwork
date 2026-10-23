//! Character motion independent of scene storage and ECS scheduling.

use bevy::math::{Vec2, Vec3};
use clockwork_core::character::{Lean, Movement, StateAnimations};
use std::f32::consts::{FRAC_PI_4, PI, TAU};

/// What the world looks like at one point on the ground plane (x, z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ground {
    Floor(f32),
    Wall,
    Hole,
}

/// Lets scenes provide collision without exposing their storage.
pub trait World {
    fn ground(&self, x: f32, z: f32) -> Ground;

    /// The ground at (x, z) for feet at height `y`. Worlds with several levels (3D terrain: a
    /// bridge over a path) pick the one the body is on; the default is `ground`.
    fn ground_at(&self, x: f32, _y: f32, z: f32) -> Ground {
        self.ground(x, z)
    }

    /// `position` moved out of any walls a body of `radius` would be inside of.
    fn push_out(&self, position: Vec3, _radius: f32) -> Vec3 {
        position
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveState {
    Idle,
    Moving,
    SkidTurn,
    Turn,
    SkidStop,
    SkidStopped,
    Jump(usize),
    Fall,
    Land(usize),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// Direction on (x, z), with analog strength preserved up to length one.
    pub direction: Vec2,
    pub walk: bool,
    pub jump_pressed: bool,
    pub jump_held: bool,
}

#[derive(Clone, Debug)]
pub struct Body {
    /// Feet position, with y pointing up.
    pub position: Vec3,
    /// Radians, facing (sin yaw, cos yaw) on (x, z).
    pub yaw: f32,
    /// Forward speed; opposing air input can briefly make it negative.
    pub speed: f32,
    pub vertical: f32,
    pub grounded: bool,
    pub state: MoveState,
    /// Respawn position after falling out of the world.
    pub start: Vec3,
    /// Wrapped yaw change per second, for leaning.
    pub turn_rate: f32,
    pub state_time: f32,
    /// Last landed jump and elapsed time, retained after its landing pose ends.
    pub chain: Option<(usize, f32)>,
    pub running_anim: bool,
    pub idle_index: usize,
    /// Roll and pitch in degrees.
    pub lean: (f32, f32),
    /// Remembers the chosen clip so callers can restart only when needed.
    pub last_animation: String,
    /// The state the last pose was chosen for: "animation done" only counts for the animation
    /// of the state it was shown in, not the one before a state change.
    pub posed_state: Option<MoveState>,
}

impl Body {
    /// Starts at rest on the ground; the next step checks the floor.
    pub fn new(position: Vec3, yaw: f32) -> Self {
        Self {
            position,
            yaw,
            speed: 0.0,
            vertical: 0.0,
            grounded: true,
            state: MoveState::Idle,
            start: position,
            turn_rate: 0.0,
            state_time: 0.0,
            chain: None,
            running_anim: false,
            idle_index: 0,
            lean: (0.0, 0.0),
            last_animation: String::new(),
            posed_state: None,
        }
    }

    fn enter(&mut self, state: MoveState) {
        if self.state != state {
            self.state = state;
            self.state_time = 0.0;
        }
    }
}

const STEP_HEIGHT: f32 = 0.3;
const TURN_TIME: f32 = 0.5;
const SKID_STOPPED_TIME: f32 = 0.5;
const INPUT_EPSILON: f32 = 0.0001;

fn wrapped(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

fn approach(value: f32, target: f32, amount: f32) -> f32 {
    value + (target - value).clamp(-amount, amount)
}

fn moving_state(speed: f32, has_input: bool) -> MoveState {
    if speed > 0.05 || has_input {
        MoveState::Moving
    } else {
        MoveState::Idle
    }
}

fn turn(body: &mut Body, target: f32, movement: &Movement, dt: f32) {
    let difference = wrapped(target - body.yaw);
    let limit = movement.turn_speed.to_radians() * dt;
    body.yaw = wrapped(
        body.yaw
            + if difference.abs() > 3.0 * FRAC_PI_4 && body.speed.abs() < 1.0 {
                difference
            } else {
                difference.clamp(-limit, limit)
            },
    );
}

fn ground_speed(body: &mut Body, input: &Input, length: f32, movement: &Movement, dt: f32) {
    let target = length
        * if input.walk {
            movement.walk_speed
        } else {
            movement.run_speed
        };
    body.speed = if length > 0.0 && body.speed < target {
        (body.speed + (movement.acceleration - body.speed * movement.acceleration_falloff) * dt)
            .min(target)
    } else {
        (body.speed - movement.braking * dt).max(target)
    };
}

fn clear(position: Vec3, radius: f32, world: &impl World) -> bool {
    let blocked = |x, z| match world.ground_at(x, position.y, z) {
        Ground::Wall => true,
        Ground::Floor(height) => height > position.y + STEP_HEIGHT,
        Ground::Hole => false,
    };
    !blocked(position.x, position.z)
        && (0..8).all(|sample| {
            let angle = sample as f32 * FRAC_PI_4;
            !blocked(
                position.x + angle.sin() * radius,
                position.z + angle.cos() * radius,
            )
        })
}

/// Advances by seconds; nonpositive or nonfinite time leaves the body unchanged.
pub fn step(body: &mut Body, input: &Input, movement: &Movement, world: &impl World, dt: f32) {
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let old_yaw = body.yaw;
    body.state_time += dt;
    if let Some((_, time)) = &mut body.chain {
        *time += dt;
    }
    let length = input.direction.length().min(1.0);
    let has_input = length > INPUT_EPSILON;
    let length = if has_input { length } else { 0.0 };
    let target_yaw = input.direction.x.atan2(input.direction.y);
    let mut strafe = 0.0;

    if body.grounded {
        match body.state {
            MoveState::Turn if body.state_time >= TURN_TIME => body.enter(MoveState::Moving),
            MoveState::SkidStopped if has_input || body.state_time >= SKID_STOPPED_TIME => {
                body.enter(moving_state(body.speed, has_input));
            }
            MoveState::Land(_) if body.state_time >= movement.landing_time => {
                body.enter(moving_state(body.speed, has_input));
            }
            _ => {}
        }
        if let Some(skid) = &movement.skid {
            let backwards =
                has_input && wrapped(target_yaw - body.yaw).abs() > skid.reverse_angle.to_radians();
            if (body.state == MoveState::SkidTurn && !backwards)
                || (body.state == MoveState::SkidStop && has_input)
            {
                body.enter(MoveState::Moving);
            }
            if body.state == MoveState::Moving && body.speed >= skid.min_speed {
                if backwards {
                    body.enter(MoveState::SkidTurn);
                } else if !has_input {
                    body.enter(MoveState::SkidStop);
                }
            }
        }

        let mut skidding = false;
        if let Some(skid) = &movement.skid
            && matches!(body.state, MoveState::SkidTurn | MoveState::SkidStop)
        {
            skidding = true;
            body.speed = (body.speed - skid.braking * dt).max(0.0);
            if body.speed == 0.0 {
                if body.state == MoveState::SkidTurn {
                    body.yaw = target_yaw;
                    body.speed = skid.turn_kick_speed;
                    body.enter(MoveState::Turn);
                } else {
                    body.enter(MoveState::SkidStopped);
                }
            }
        }
        if !skidding {
            if has_input && body.state != MoveState::Turn {
                turn(body, target_yaw, movement, dt);
            }
            ground_speed(body, input, length, movement, dt);
        }
        if matches!(body.state, MoveState::Idle | MoveState::Moving) {
            body.enter(moving_state(body.speed, has_input));
        }

        if input.jump_pressed && !movement.jumps.is_empty() {
            let index = body
                .chain
                .and_then(|(i, time)| {
                    i.checked_add(1).filter(|&next| {
                        time <= movement.chain_window
                            && movement
                                .jumps
                                .get(next)
                                .is_some_and(|jump| body.speed >= jump.min_ground_speed)
                    })
                })
                .unwrap_or(0);
            let jump = &movement.jumps[index];
            body.vertical = jump.speed + jump.speed_from_ground * body.speed;
            body.speed *= jump.keep_ground_speed;
            body.grounded = false;
            body.chain = None;
            body.enter(MoveState::Jump(index));
        }
    } else {
        if !matches!(body.state, MoveState::Jump(_) | MoveState::Fall) {
            body.enter(MoveState::Fall);
            body.chain = None;
        }
        if has_input && movement.air.turn {
            turn(body, target_yaw, movement, dt);
        }
        body.speed = approach(body.speed, 0.0, movement.air.drag * dt);
        if has_input {
            let angle = wrapped(target_yaw - body.yaw);
            body.speed += movement.air.acceleration * length * angle.cos() * dt;
            strafe = movement.air.strafe_speed * length * angle.sin();
        }
        if body.speed > movement.air.max_speed {
            body.speed -= movement.braking * dt;
        }
    }

    if let MoveState::Jump(i) = body.state
        && movement.jumps.get(i).is_some_and(|jump| jump.controllable)
        && let Some(release) = &movement.jump_release
        && !input.jump_held
        && body.vertical > release.above
    {
        body.vertical *= release.factor.powf(release.rate * dt);
    }

    let forward = Vec3::new(body.yaw.sin(), 0.0, body.yaw.cos());
    let right = Vec3::new(body.yaw.cos(), 0.0, -body.yaw.sin());
    let displacement = (forward * body.speed + right * strafe) * dt;
    let mut destination = body.position + displacement;
    // Walls push back: the speed into them is lost, along them it is kept.
    let pushed = world.push_out(destination, movement.radius);
    if pushed != destination {
        let into = (destination - pushed).normalize_or_zero();
        let toward = forward.dot(into).max(0.0);
        body.speed *= 1.0 - toward.min(1.0);
        destination = pushed;
    }
    if clear(destination, movement.radius, world) {
        body.position = destination;
    } else {
        let along_x = body.position + Vec3::X * displacement.x;
        let along_z = body.position + Vec3::Z * displacement.z;
        // A zero-length axis is not an escape from a head-on collision.
        if displacement.x.abs() > f32::EPSILON && clear(along_x, movement.radius, world) {
            body.position = along_x;
        } else if displacement.z.abs() > f32::EPSILON && clear(along_z, movement.radius, world) {
            body.position = along_z;
        } else {
            body.speed = 0.0;
        }
    }

    let ground = world.ground_at(body.position.x, body.position.y, body.position.z);
    if body.grounded {
        match ground {
            Ground::Floor(height) if (height - body.position.y).abs() <= STEP_HEIGHT => {
                body.position.y = height;
                body.vertical = 0.0;
            }
            _ => {
                body.grounded = false;
                body.vertical = 0.0;
                body.chain = None;
                body.enter(MoveState::Fall);
            }
        }
    } else {
        body.vertical = (body.vertical - movement.gravity * dt).max(-movement.max_fall_speed);
        body.position.y += body.vertical * dt;
        if let Ground::Floor(height) = ground
            && body.vertical <= 0.0
            && body.position.y <= height
        {
            body.position.y = height;
            body.vertical = 0.0;
            body.grounded = true;
            if let MoveState::Jump(i) = body.state {
                body.chain = Some((i, 0.0));
                body.enter(MoveState::Land(i));
            } else {
                body.chain = None;
                body.enter(moving_state(body.speed, has_input));
            }
        }
    }

    if body.position.y < body.start.y - 30.0 {
        *body = Body::new(body.start, body.yaw);
        return;
    }
    body.turn_rate = wrapped(body.yaw - old_yaw) / dt;
}

/// A clip choice and the playback controls needed to display it.
pub struct Pose<'a> {
    pub animation: &'a str,
    pub rate: f32,
    pub restart: bool,
    pub flipped: bool,
}

fn moving_pose<'a>(body: &mut Body, anims: &'a StateAnimations) -> (&'a str, f32) {
    if body.speed > anims.run_above {
        body.running_anim = true;
    } else if body.speed < anims.walk_below {
        body.running_anim = false;
    }
    let animation = if body.running_anim {
        &anims.run
    } else {
        &anims.walk
    };
    (animation, (body.speed.abs() / anims.stride_speed).max(0.1))
}

/// Selects clips from data and lets completed turn and rest clips release their states.
pub fn pose<'a>(
    body: &mut Body,
    anims: &'a StateAnimations,
    movement: &'a Movement,
    animation_done: bool,
) -> Pose<'a> {
    let animation_done = animation_done && body.posed_state == Some(body.state);
    let idle_advanced = body.state == MoveState::Idle && animation_done;
    if animation_done {
        match body.state {
            MoveState::Turn => body.enter(MoveState::Moving),
            MoveState::SkidStopped => body.enter(MoveState::Idle),
            _ => {}
        }
    }
    if idle_advanced && !anims.idle.is_empty() {
        body.idle_index = (body.idle_index % anims.idle.len() + 1) % anims.idle.len();
    }
    let idle = anims.idle.first().map_or("", String::as_str);
    let mut flipped = false;
    let (animation, rate) = match body.state {
        MoveState::Idle => (
            anims
                .idle
                .get(body.idle_index.checked_rem(anims.idle.len()).unwrap_or(0))
                .map_or(idle, String::as_str),
            1.0,
        ),
        MoveState::Moving => moving_pose(body, anims),
        MoveState::SkidTurn => (anims.skid_turn.as_deref().unwrap_or(&anims.walk), 1.0),
        MoveState::Turn => {
            flipped = anims.turn.is_some() && anims.turn_faces_back;
            (anims.turn.as_deref().unwrap_or(&anims.walk), 1.0)
        }
        MoveState::SkidStop => (anims.skid_stop.as_deref().unwrap_or(idle), 1.0),
        MoveState::SkidStopped => (anims.skid_stopped.as_deref().unwrap_or(idle), 1.0),
        MoveState::Jump(i) => (
            movement.jumps.get(i).map_or(anims.fall.as_str(), |jump| {
                if body.vertical > 0.0 {
                    &jump.animation
                } else {
                    jump.fall_animation.as_deref().unwrap_or(&jump.animation)
                }
            }),
            1.0,
        ),
        MoveState::Fall => (anims.fall.as_str(), 1.0),
        MoveState::Land(i) => {
            if let Some(animation) = movement
                .jumps
                .get(i)
                .and_then(|jump| jump.land_animation.as_deref())
            {
                (animation, 1.0)
            } else if body.speed > 0.05 {
                moving_pose(body, anims)
            } else {
                (idle, 1.0)
            }
        }
    };
    let restart = idle_advanced || animation != body.last_animation;
    body.posed_state = Some(body.state);
    body.last_animation.clear();
    body.last_animation.push_str(animation);
    Pose {
        animation,
        rate,
        restart,
        flipped,
    }
}

/// Follows the moving lean target; other states immediately return the bone to rest.
pub fn lean(body: &mut Body, lean: &Lean, dt: f32) -> (f32, f32) {
    if body.state != MoveState::Moving {
        body.lean = (0.0, 0.0);
    } else if dt.is_finite() && dt > 0.0 {
        let roll = (body.turn_rate.to_degrees() * body.speed * lean.roll_per_turn)
            .clamp(-lean.max, lean.max);
        let pitch = (body.speed * lean.pitch_per_speed).clamp(-lean.max, lean.max);
        body.lean = (
            approach(body.lean.0, roll, lean.rate * dt),
            approach(body.lean.1, pitch, lean.rate * dt),
        );
    }
    body.lean
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_animation_does_not_end_the_next_state() {
        let anims = StateAnimations {
            idle: vec!["idle".into()],
            walk: "walk".into(),
            run: "run".into(),
            run_above: 6.0,
            walk_below: 5.0,
            stride_speed: 1.0,
            fall: "fall".into(),
            skid_turn: None,
            turn: None,
            turn_faces_back: false,
            skid_stop: Some("skid".into()),
            skid_stopped: Some("stopped".into()),
        };
        let movement = movement();
        let mut b = Body::new(Vec3::ZERO, 0.0);
        b.state = MoveState::SkidStop;
        assert_eq!(pose(&mut b, &anims, &movement, false).animation, "skid");
        // The skid animation finished just as the state changed.
        b.state = MoveState::SkidStopped;
        let p = pose(&mut b, &anims, &movement, true);
        assert_eq!((b.state, p.animation), (MoveState::SkidStopped, "stopped"));
        // Once its own animation finishes, it ends.
        pose(&mut b, &anims, &movement, true);
        assert_eq!(b.state, MoveState::Idle);
    }
    use clockwork_core::character::{AirControl, Jump, JumpRelease, Skid};

    struct TestWorld;
    impl World for TestWorld {
        fn ground(&self, x: f32, z: f32) -> Ground {
            if x.abs() >= 10.0 || !(-3.0..10.0).contains(&z) {
                Ground::Hole
            } else if x > 3.0 {
                Ground::Wall
            } else {
                Ground::Floor(0.0)
            }
        }
    }

    struct Flat;
    impl World for Flat {
        fn ground(&self, _: f32, _: f32) -> Ground {
            Ground::Floor(0.0)
        }
    }

    struct Ledge(f32);
    impl World for Ledge {
        fn ground(&self, _: f32, z: f32) -> Ground {
            Ground::Floor(if z > 1.0 { self.0 } else { 0.0 })
        }
    }

    fn movement() -> Movement {
        Movement {
            walk_speed: 2.0,
            run_speed: 6.0,
            acceleration: 12.0,
            acceleration_falloff: 0.0,
            braking: 16.0,
            turn_speed: 90.0,
            skid: Some(Skid {
                min_speed: 4.0,
                reverse_angle: 135.0,
                braking: 10.0,
                turn_kick_speed: 1.5,
            }),
            jumps: (0..3)
                .map(|i| Jump {
                    speed: 6.0 + i as f32,
                    speed_from_ground: 0.0,
                    keep_ground_speed: 1.0,
                    min_ground_speed: if i == 2 { 5.0 } else { 2.0 },
                    controllable: true,
                    animation: format!("jump{i}"),
                    fall_animation: Some(format!("fall{i}")),
                    land_animation: Some(format!("land{i}")),
                })
                .collect(),
            chain_window: 0.3,
            landing_time: 0.1,
            jump_release: Some(JumpRelease {
                above: 1.0,
                factor: 0.5,
                rate: 10.0,
            }),
            gravity: 20.0,
            max_fall_speed: 25.0,
            air: AirControl {
                turn: false,
                drag: 0.0,
                acceleration: 0.0,
                strafe_speed: 3.0,
                max_speed: 6.0,
            },
            radius: 0.3,
        }
    }

    fn anims() -> StateAnimations {
        StateAnimations {
            idle: vec!["idle0".into(), "idle1".into()],
            walk: "walk".into(),
            run: "run".into(),
            run_above: 3.0,
            walk_below: 2.0,
            stride_speed: 2.0,
            fall: "fall".into(),
            skid_turn: Some("skid_turn".into()),
            turn: Some("turn".into()),
            turn_faces_back: true,
            skid_stop: Some("skid_stop".into()),
            skid_stopped: Some("skid_stopped".into()),
        }
    }

    fn forward() -> Input {
        Input {
            direction: Vec2::Y,
            ..Input::default()
        }
    }

    fn running(speed: f32) -> Body {
        let mut body = Body::new(Vec3::ZERO, 0.0);
        body.speed = speed;
        body.state = MoveState::Moving;
        body
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn acceleration_curve_caps_and_brakes_to_analog_targets() {
        let mut m = movement();
        m.acceleration_falloff = 2.0;
        let mut b = Body::new(Vec3::ZERO, 0.0);
        let mut previous_gain = f32::MAX;
        for _ in 0..20 {
            let before = b.speed;
            step(&mut b, &forward(), &m, &Flat, 0.1);
            let gain = b.speed - before;
            assert!(gain > 0.0 && gain < previous_gain);
            assert!(b.speed < m.run_speed);
            previous_gain = gain;
        }
        assert!(b.speed > 5.9);
        m.acceleration_falloff = 1.0;
        step(&mut b, &forward(), &m, &Flat, 0.1);
        near(b.speed, 6.0);
        for (walk, length, target) in [(true, 1.0, 2.0), (false, 0.5, 3.0)] {
            let input = Input {
                direction: Vec2::Y * length,
                walk,
                ..Input::default()
            };
            for _ in 0..20 {
                step(&mut b, &input, &m, &Flat, 0.1);
            }
            near(b.speed, target);
        }
        for _ in 0..20 {
            step(&mut b, &Input::default(), &m, &Flat, 0.1);
        }
        near(b.speed, 0.0);
        assert_eq!(b.state, MoveState::Idle);
    }

    #[test]
    fn turns_wraps_and_snaps_at_low_speed() {
        let mut b = Body::new(Vec3::ZERO, 0.0);
        let mut input = Input {
            direction: Vec2::X,
            ..Input::default()
        };
        step(&mut b, &input, &movement(), &Flat, 0.1);
        near(b.yaw, 9_f32.to_radians());
        near(b.turn_rate, 90_f32.to_radians());
        b.yaw = 179_f32.to_radians();
        b.speed = 2.0;
        let target = -179_f32.to_radians();
        input.direction = Vec2::new(target.sin(), target.cos());
        step(&mut b, &input, &movement(), &Flat, 0.1);
        near(b.yaw, target);
        near(b.turn_rate, 20_f32.to_radians());
        b.yaw = 0.0;
        b.speed = 0.0;
        step(&mut b, &input, &movement(), &Flat, 0.01);
        near(b.yaw, target);
    }

    #[test]
    fn turnaround_skids_kicks_and_finishes_without_turning() {
        let m = movement();
        let mut b = running(6.0);
        let input = Input {
            direction: -Vec2::Y,
            ..Input::default()
        };
        step(&mut b, &input, &m, &Flat, 0.1);
        assert_eq!(b.state, MoveState::SkidTurn);
        near(b.speed, 5.0);
        near(b.yaw, 0.0);
        near(b.state_time, 0.0);
        for _ in 0..5 {
            step(&mut b, &input, &m, &Flat, 0.1);
        }
        assert_eq!(b.state, MoveState::Turn);
        near(wrapped(b.yaw - PI), 0.0);
        near(b.speed, m.skid.as_ref().unwrap().turn_kick_speed);
        let side = Input {
            direction: Vec2::X,
            ..input
        };
        step(&mut b, &side, &m, &Flat, 0.1);
        near(wrapped(b.yaw - PI), 0.0);
        assert!(b.speed > 1.5);
        for _ in 0..4 {
            step(&mut b, &input, &m, &Flat, 0.1);
        }
        assert_eq!(b.state, MoveState::Moving);
        near(b.state_time, 0.0);
        b.enter(MoveState::Turn);
        let a = anims();
        assert!(pose(&mut b, &a, &m, false).flipped);
        pose(&mut b, &a, &m, true);
        assert_eq!(b.state, MoveState::Moving);
        b = running(6.0);
        step(&mut b, &input, &m, &Flat, 0.1);
        step(&mut b, &forward(), &m, &Flat, 0.1);
        assert_eq!(b.state, MoveState::Moving);
    }

    #[test]
    fn skid_stop_rests_and_can_be_interrupted() {
        let m = movement();
        let mut b = running(6.0);
        step(&mut b, &Input::default(), &m, &Flat, 0.1);
        assert_eq!(b.state, MoveState::SkidStop);
        near(b.speed, 5.0);
        let mut interrupted = b.clone();
        step(&mut interrupted, &forward(), &m, &Flat, 0.1);
        assert_eq!(interrupted.state, MoveState::Moving);
        for _ in 0..5 {
            step(&mut b, &Input::default(), &m, &Flat, 0.1);
        }
        assert_eq!(b.state, MoveState::SkidStopped);
        let mut completed = b.clone();
        pose(&mut completed, &anims(), &m, false);
        pose(&mut completed, &anims(), &m, true);
        assert_eq!(completed.state, MoveState::Idle);
        assert_eq!(completed.idle_index, 0);
        let mut interrupted = b.clone();
        step(&mut interrupted, &forward(), &m, &Flat, 0.1);
        assert_eq!(interrupted.state, MoveState::Moving);
        for _ in 0..5 {
            step(&mut b, &Input::default(), &m, &Flat, 0.1);
        }
        assert_eq!(b.state, MoveState::Idle);
    }

    fn jump_and_land(b: &mut Body, m: &Movement, expected: usize) {
        let input = Input {
            jump_pressed: true,
            jump_held: true,
            ..forward()
        };
        step(b, &input, m, &Flat, 0.01);
        assert_eq!(b.state, MoveState::Jump(expected));
        let input = Input {
            jump_pressed: false,
            ..input
        };
        let mut falling = false;
        for _ in 0..200 {
            step(b, &input, m, &Flat, 0.01);
            if b.grounded {
                assert!(falling);
                assert_eq!(b.state, MoveState::Land(expected));
                assert_eq!(b.chain, Some((expected, 0.0)));
                near(b.position.y, 0.0);
                return;
            }
            falling |= b.vertical < 0.0;
            assert_eq!(b.state, MoveState::Jump(expected));
        }
        panic!("did not land");
    }

    #[test]
    fn jump_chain_uses_speed_and_time_since_landing() {
        let m = movement();
        let mut b = running(6.0);
        jump_and_land(&mut b, &m, 0);
        // The chain survives the landing pose ending.
        step(&mut b, &forward(), &m, &Flat, 0.15);
        assert_eq!(b.state, MoveState::Moving);
        near(b.chain.unwrap().1, 0.15);
        jump_and_land(&mut b, &m, 1);
        let mut slow = b.clone();
        slow.speed = 3.0;
        jump_and_land(&mut slow, &m, 0);
        jump_and_land(&mut b, &m, 2);
        jump_and_land(&mut b, &m, 0);
        step(&mut b, &forward(), &m, &Flat, 0.31);
        jump_and_land(&mut b, &m, 0);
    }

    #[test]
    fn jumps_interrupt_every_ground_state_and_apply_data() {
        let mut m = movement();
        m.jumps[0].speed_from_ground = 0.5;
        m.jumps[0].keep_ground_speed = 0.25;
        for state in [
            MoveState::Idle,
            MoveState::Moving,
            MoveState::SkidTurn,
            MoveState::Turn,
            MoveState::SkidStop,
            MoveState::SkidStopped,
            MoveState::Land(0),
        ] {
            let mut b = running(2.0);
            b.state = state;
            let input = Input {
                jump_pressed: true,
                jump_held: true,
                ..forward()
            };
            step(&mut b, &input, &m, &Flat, 0.01);
            assert_eq!(b.state, MoveState::Jump(0));
            near(b.speed, 2.12 * 0.25);
            near(b.vertical, 6.0 + 0.5 * 2.12 - 0.2);
            assert!(!b.grounded);
        }
        m.jumps.clear();
        let mut b = running(2.0);
        step(
            &mut b,
            &Input {
                jump_pressed: true,
                ..forward()
            },
            &m,
            &Flat,
            0.01,
        );
        assert!(b.grounded);
    }

    fn peak(held: bool, controllable: bool) -> f32 {
        let mut m = movement();
        m.jumps[0].controllable = controllable;
        let mut b = Body::new(Vec3::ZERO, 0.0);
        let mut input = Input {
            jump_pressed: true,
            jump_held: held,
            ..Input::default()
        };
        let mut peak: f32 = 0.0;
        for _ in 0..100 {
            step(&mut b, &input, &m, &Flat, 0.01);
            input.jump_pressed = false;
            peak = peak.max(b.position.y);
        }
        peak
    }

    #[test]
    fn release_lowers_only_controllable_jump_peak() {
        assert!(peak(false, true) < peak(true, true) * 0.5);
        near(peak(false, false), peak(true, true));
    }

    #[test]
    fn air_keeps_facing_strafes_and_controls_signed_speed() {
        let mut m = movement();
        let mut b = Body::new(Vec3::new(0.0, 50.0, 0.0), 0.0);
        b.grounded = false;
        b.state = MoveState::Fall;
        let input = Input {
            direction: Vec2::X,
            ..Input::default()
        };
        step(&mut b, &input, &m, &Flat, 0.1);
        near(b.yaw, 0.0);
        near(b.position.x, 0.3);
        near(b.position.z, 0.0);
        m.air.acceleration = 4.0;
        m.air.drag = 1.0;
        step(
            &mut b,
            &Input {
                direction: -Vec2::Y,
                ..input
            },
            &m,
            &Flat,
            0.1,
        );
        near(b.speed, -0.4);
        step(&mut b, &Input::default(), &m, &Flat, 0.1);
        near(b.speed, -0.3);
        step(&mut b, &Input::default(), &m, &Flat, 0.4);
        near(b.speed, 0.0);
        b.speed = 7.0;
        step(&mut b, &Input::default(), &m, &Flat, 0.1);
        near(b.speed, 5.3);
        m.air.turn = true;
        step(&mut b, &input, &m, &Flat, 0.1);
        near(b.yaw, 9_f32.to_radians());
    }

    #[test]
    fn walks_off_edge_with_zero_vertical_and_lands_without_pose() {
        let mut b = running(6.0);
        b.position.z = 0.99;
        b.chain = Some((1, 0.0));
        step(&mut b, &forward(), &movement(), &Ledge(-1.0), 0.02);
        assert_eq!(b.state, MoveState::Fall);
        near(b.vertical, 0.0);
        assert_eq!(b.chain, None);
        for _ in 0..100 {
            step(&mut b, &forward(), &movement(), &Ledge(-1.0), 0.02);
            assert!(!matches!(b.state, MoveState::Land(_)));
            if b.grounded {
                near(b.position.y, -1.0);
                assert_eq!(b.state, MoveState::Moving);
                return;
            }
        }
        panic!("did not land");
    }

    /// A floor at 0 with a platform at 3 above it for x > 10, and a wall at x = 5 that only
    /// `push_out` knows about (it stops bodies coming from the left).
    struct Stacked;

    impl World for Stacked {
        fn ground(&self, _x: f32, _z: f32) -> Ground {
            Ground::Floor(0.0)
        }

        fn ground_at(&self, x: f32, y: f32, _z: f32) -> Ground {
            Ground::Floor(if x > 10.0 && y >= 2.0 { 3.0 } else { 0.0 })
        }

        fn push_out(&self, mut position: Vec3, radius: f32) -> Vec3 {
            if position.x < 6.0 {
                position.x = position.x.min(5.0 - radius);
            }
            position
        }
    }

    #[test]
    fn pushed_out_of_walls_and_landing_on_the_level_below() {
        let m = movement();
        let mut b = Body::new(Vec3::new(4.0, 0.0, 0.0), PI / 2.0);
        b.speed = 6.0;
        let input = Input {
            direction: Vec2::X,
            ..Input::default()
        };
        for _ in 0..20 {
            step(&mut b, &input, &m, &Stacked, 0.02);
            assert!(b.position.x <= 5.0 - m.radius + 1e-4);
        }
        // Running into the wall head-on loses the speed.
        near(b.speed, 0.0);
        // Falling from above lands on the platform, not on the floor far below it.
        let mut b = Body::new(Vec3::new(12.0, 5.0, 0.0), 0.0);
        b.grounded = false;
        for _ in 0..200 {
            step(&mut b, &Input::default(), &m, &Stacked, 0.02);
        }
        assert!(b.grounded);
        near(b.position.y, 3.0);
    }

    #[test]
    fn slides_along_wall_and_stops_head_on() {
        let mut b = Body::new(Vec3::new(2.6, 0.0, 0.0), FRAC_PI_4);
        let input = Input {
            direction: Vec2::ONE.normalize(),
            ..Input::default()
        };
        for _ in 0..30 {
            step(&mut b, &input, &movement(), &TestWorld, 0.02);
            assert!(b.position.x + movement().radius <= 3.0);
        }
        assert!(b.position.z > 1.0 && b.speed > 0.0);
        b.yaw = PI / 2.0;
        b.speed = 6.0;
        step(
            &mut b,
            &Input {
                direction: Vec2::X,
                ..input
            },
            &movement(),
            &TestWorld,
            0.1,
        );
        near(b.speed, 0.0);
        // Air strafe uses the same radius checks.
        b.yaw = 0.0;
        b.grounded = false;
        b.position.y = 2.0;
        let x = b.position.x;
        step(
            &mut b,
            &Input {
                direction: Vec2::X,
                ..input
            },
            &movement(),
            &TestWorld,
            0.1,
        );
        near(b.position.x, x);
    }

    #[test]
    fn small_step_allowed_tall_step_blocked() {
        for height in [0.2, 1.0] {
            let mut b = Body::new(Vec3::ZERO, 0.0);
            for _ in 0..50 {
                step(&mut b, &forward(), &movement(), &Ledge(height), 0.02);
            }
            if height < STEP_HEIGHT {
                assert!(b.position.z > 1.0);
                near(b.position.y, height);
            } else {
                assert!(b.position.z + movement().radius <= 1.0);
                near(b.position.y, 0.0);
            }
        }
    }

    #[test]
    fn hole_falls_and_respawns() {
        let mut b = Body::new(Vec3::ZERO, PI);
        let input = Input {
            direction: -Vec2::Y,
            ..Input::default()
        };
        let mut fell = false;
        for _ in 0..500 {
            step(&mut b, &input, &movement(), &TestWorld, 0.02);
            fell |= b.state == MoveState::Fall;
            assert!(b.vertical >= -movement().max_fall_speed);
            if fell && b.position == b.start {
                near(b.speed, 0.0);
                near(b.vertical, 0.0);
                assert_eq!(b.state, MoveState::Idle);
                assert_eq!(b.chain, None);
                near(b.state_time, 0.0);
                return;
            }
        }
        panic!("did not respawn");
    }

    #[test]
    fn pose_hysteresis_rates_and_idle_cycles() {
        let a = anims();
        let m = movement();
        let mut b = running(3.0);
        for (speed, name, rate) in [
            (3.0, "walk", 1.5),
            (4.0, "run", 2.0),
            (2.0, "run", 1.0),
            (1.0, "walk", 0.5),
            (0.0, "walk", 0.1),
            (-1.0, "walk", 0.5),
        ] {
            b.speed = speed;
            let p = pose(&mut b, &a, &m, false);
            assert_eq!(p.animation, name);
            near(p.rate, rate);
        }
        b.enter(MoveState::Idle);
        assert!(pose(&mut b, &a, &m, false).restart);
        assert!(!pose(&mut b, &a, &m, false).restart);
        let p = pose(&mut b, &a, &m, true);
        assert_eq!(p.animation, "idle1");
        assert!(p.restart);
        assert_eq!(pose(&mut b, &a, &m, true).animation, "idle0");
        let mut a = a;
        a.idle.truncate(1);
        assert!(pose(&mut b, &a, &m, true).restart);
        a.idle.clear();
        assert_eq!(pose(&mut b, &a, &m, false).animation, "");
    }

    #[test]
    fn pose_state_choices_and_fallbacks() {
        let mut a = anims();
        let mut m = movement();
        let mut b = running(4.0);
        for (state, name) in [
            (MoveState::SkidTurn, "skid_turn"),
            (MoveState::Turn, "turn"),
            (MoveState::SkidStop, "skid_stop"),
            (MoveState::SkidStopped, "skid_stopped"),
            (MoveState::Jump(1), "fall1"),
            (MoveState::Fall, "fall"),
            (MoveState::Land(1), "land1"),
        ] {
            b.enter(state);
            let p = pose(&mut b, &a, &m, false);
            assert_eq!(p.animation, name);
            near(p.rate, 1.0);
            assert_eq!(p.flipped, state == MoveState::Turn);
        }
        b.enter(MoveState::Jump(1));
        b.vertical = 1.0;
        assert_eq!(pose(&mut b, &a, &m, false).animation, "jump1");
        b.vertical = -1.0;
        m.jumps[1].fall_animation = None;
        assert_eq!(pose(&mut b, &a, &m, false).animation, "jump1");
        b.enter(MoveState::Land(1));
        m.jumps[1].land_animation = None;
        assert_eq!(pose(&mut b, &a, &m, false).animation, "run");
        b.speed = 0.0;
        assert_eq!(pose(&mut b, &a, &m, false).animation, "idle0");
        a.skid_turn = None;
        a.turn = None;
        a.skid_stop = None;
        a.skid_stopped = None;
        for (state, name) in [
            (MoveState::SkidTurn, "walk"),
            (MoveState::Turn, "walk"),
            (MoveState::SkidStop, "idle0"),
            (MoveState::SkidStopped, "idle0"),
        ] {
            b.enter(state);
            let p = pose(&mut b, &a, &m, false);
            assert_eq!(p.animation, name);
            assert!(!p.flipped);
        }
    }

    #[test]
    fn lean_targets_clamps_and_snaps_outside_moving() {
        let l = Lean {
            bone: "root".into(),
            roll_axis: [0.0, 0.0, 1.0],
            pitch_axis: [1.0, 0.0, 0.0],
            roll_per_turn: 0.1,
            pitch_per_speed: 2.0,
            max: 10.0,
            rate: 20.0,
        };
        let mut b = running(2.0);
        b.turn_rate = 30_f32.to_radians();
        let (roll, pitch) = lean(&mut b, &l, 0.1);
        near(roll, 2.0);
        near(pitch, 2.0);
        let (roll, pitch) = lean(&mut b, &l, 1.0);
        near(roll, 6.0);
        near(pitch, 4.0);
        b.speed = 6.0;
        b.turn_rate = -90_f32.to_radians();
        let (roll, pitch) = lean(&mut b, &l, 1.0);
        near(roll, -10.0);
        near(pitch, 10.0);
        b.enter(MoveState::Land(0));
        assert_eq!(lean(&mut b, &l, 0.001), (0.0, 0.0));
    }

    #[test]
    fn invalid_time_does_not_change_body() {
        let mut b = running(6.0);
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            step(&mut b, &forward(), &movement(), &Flat, dt);
            near(b.speed, 6.0);
            near(b.state_time, 0.0);
            assert_eq!(b.position, Vec3::ZERO);
        }
    }
}

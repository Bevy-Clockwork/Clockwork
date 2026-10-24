//! Actors at work: each one's state machine (`clockwork_core::actor`) stepped against the
//! player and the level. What an actor's states mean is generic; the numbers and states are the
//! plugin's.

use bevy::math::{Vec2, Vec3};
use clockwork_core::{
    actor::{Actor, ActorState, Cue, Motion},
    behavior::Facts,
    character2d::Direction4,
};

use crate::controller::{Ground, World};

/// How fast a hit's push dies away, per second: a push of `knockback` meters starts at ten
/// times that speed.
const PUSH_DECAY: f32 = 10.0;
/// The highest step up an actor takes, in meters.
const STEP_UP: f32 = 0.5;
/// Close enough to a point to have arrived, in meters.
const ARRIVED: f32 = 0.1;

/// An actor's running state.
#[derive(Debug, Clone)]
pub struct Mind {
    /// Its current state, by index into the actor's states.
    pub state: usize,
    /// Seconds in the current state.
    pub time: f32,
    /// Where it was placed.
    pub home: Vec3,
    /// The way it faces, for poses with a facing.
    pub facing: Direction4,
    /// The point it walks to (wandering) or the way it rushes (charging).
    target: Option<Vec2>,
    /// Seconds left of a wander pause.
    waiting: f32,
    /// The patrol point it walks to next.
    patrol: usize,
    /// What happened since its transitions were last checked.
    hurt: bool,
    arrived: bool,
    blocked: bool,
    /// A hit's push, meters per second.
    push: Vec2,
    /// Its random numbers: the same every play from the same start.
    pub dice: Dice,
}

/// Random numbers that come out the same every time from the same seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dice(u64);

impl Dice {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// A number in 0..1 (xorshift64*).
    pub fn roll(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
}

impl Mind {
    pub fn new(home: Vec3, seed: u64) -> Self {
        Self {
            state: 0,
            time: 0.0,
            home,
            facing: Direction4::South,
            target: None,
            waiting: 0.0,
            patrol: 0,
            hurt: false,
            arrived: false,
            blocked: false,
            push: Vec2::ZERO,
            dice: Dice::new(seed),
        }
    }

    fn random(&mut self) -> f32 {
        self.dice.roll()
    }

    /// It was hit: it reacts at its next step, pushed `away` (a ground direction).
    pub fn hit(&mut self, actor: &Actor, away: Vec2) {
        self.hurt = true;
        self.push = away * actor.knockback * PUSH_DECAY;
    }
}

/// What an actor's cues see: where the player is, its health and the game's facts.
pub struct Seen<'a> {
    pub player: Vec3,
    pub health: f32,
    pub facts: &'a dyn Facts,
}

/// The state it goes to now, if a transition's cue holds. `roll` gives random numbers.
pub fn next_state(
    actor: &Actor,
    mind: &Mind,
    at: Vec3,
    seen: &Seen,
    dt: f32,
    roll: &mut dyn FnMut() -> f32,
) -> Option<usize> {
    let state = actor.states.get(mind.state)?;
    let distance = Vec2::new(seen.player.x - at.x, seen.player.z - at.z).length();
    state
        .next
        .iter()
        .find(|t| holds(&t.when, mind, distance, seen, dt, roll))
        .and_then(|t| actor.state(&t.to))
}

fn holds(
    cue: &Cue,
    mind: &Mind,
    distance: f32,
    seen: &Seen,
    dt: f32,
    roll: &mut dyn FnMut() -> f32,
) -> bool {
    match cue {
        Cue::Near(d) => distance <= *d,
        Cue::Far(d) => distance > *d,
        Cue::After(seconds) => mind.time >= *seconds,
        Cue::Hurt => mind.hurt,
        Cue::HealthBelow(h) => seen.health < *h,
        Cue::Arrived => mind.arrived,
        Cue::Blocked => mind.blocked,
        Cue::Chance(per_second) => roll() < per_second * dt,
        Cue::When(condition) => condition.holds(seen.facts),
        Cue::Not(inner) => !holds(inner, mind, distance, seen, dt, roll),
        Cue::All(all) => all.iter().all(|c| holds(c, mind, distance, seen, dt, roll)),
        Cue::Any(any) => any.iter().any(|c| holds(c, mind, distance, seen, dt, roll)),
    }
}

impl Mind {
    /// Clears what the transitions have seen, after they were checked.
    pub fn seen(&mut self) {
        self.hurt = false;
        self.arrived = false;
        self.blocked = false;
    }

    /// Goes into state `index`, facing what it begins with (a charge, the player).
    pub fn enter(&mut self, index: usize, state: &ActorState, at: Vec3, player: Vec3) {
        self.state = index;
        self.time = 0.0;
        self.target = None;
        self.waiting = 0.0;
        if state.motion == Motion::Charge {
            let dir = Vec2::new(player.x - at.x, player.z - at.z).normalize_or_zero();
            self.target = Some(dir);
        }
    }

    /// Moves it one step of `dt` seconds as its state says. Returns where it is now.
    pub fn step(
        &mut self,
        actor: &Actor,
        at: Vec3,
        player: Vec3,
        world: &impl World,
        dt: f32,
    ) -> Vec3 {
        self.time += dt;
        let Some(state) = actor.states.get(self.state) else {
            return at;
        };
        let here = Vec2::new(at.x, at.z);
        let home = Vec2::new(self.home.x, self.home.z);
        let toward = |p: Vec2| (p - here).normalize_or_zero();
        let arrive_at = |target: Vec2, arrived: &mut bool| {
            if here.distance(target) <= ARRIVED {
                *arrived = true;
                None
            } else {
                Some(toward(target))
            }
        };
        let player_ground = Vec2::new(player.x, player.z);
        let mut arrived = false;
        let dir: Option<Vec2> = match &state.motion {
            Motion::Still => None,
            Motion::Chase => {
                (here.distance(player_ground) > actor.radius).then(|| toward(player_ground))
            }
            Motion::Flee => Some(-toward(player_ground)),
            Motion::Charge => self.target,
            Motion::Home => arrive_at(home, &mut arrived),
            Motion::Wander { radius, pause } => {
                if self.waiting > 0.0 {
                    self.waiting -= dt;
                    None
                } else {
                    let target = match self.target {
                        Some(t) => t,
                        None => {
                            let angle = self.random() * std::f32::consts::TAU;
                            let reach = radius * self.random().sqrt();
                            let t = home + Vec2::from_angle(angle) * reach;
                            self.target = Some(t);
                            t
                        }
                    };
                    let dir = arrive_at(target, &mut arrived);
                    if arrived {
                        self.target = None;
                        self.waiting = *pause;
                    }
                    dir
                }
            }
            Motion::Patrol { points, looped } => match points.get(self.patrol) {
                Some(&(x, z)) => {
                    let dir = arrive_at(home + Vec2::new(x, z), &mut arrived);
                    if arrived {
                        if self.patrol + 1 < points.len() {
                            self.patrol += 1;
                            arrived = false;
                        } else if *looped {
                            self.patrol = 0;
                            arrived = false;
                        } else {
                            // At the last point: it stays, and has arrived.
                            self.patrol = points.len();
                        }
                    }
                    dir
                }
                None => None,
            },
        };
        self.arrived |= arrived;
        let speed = actor.speed * state.speed;
        let walk = dir.map_or(Vec2::ZERO, |d| d * speed);
        if let Some(d) = dir.filter(|d| d.length_squared() > 0.0) {
            self.facing = facing_of(d);
        }
        let velocity = walk + self.push;
        self.push *= (-PUSH_DECAY * dt).exp();
        if self.push.length() < 0.01 {
            self.push = Vec2::ZERO;
        }
        let delta = velocity * dt;
        if delta == Vec2::ZERO {
            return at;
        }
        // The whole way, else along one axis (sliding along walls), else stopped. The ground is
        // checked at its edge, a radius ahead.
        for try_delta in [delta, Vec2::new(delta.x, 0.0), Vec2::new(0.0, delta.y)] {
            if try_delta == Vec2::ZERO {
                continue;
            }
            let to = here + try_delta;
            let probe = to + try_delta.normalize() * actor.radius;
            let feet = match world.ground_at(probe.x, at.y, probe.y) {
                Ground::Floor(h) if h - at.y <= STEP_UP => h,
                _ => continue,
            };
            let height = match world.ground_at(to.x, at.y, to.y) {
                Ground::Floor(h) if h - at.y <= STEP_UP => h,
                _ => feet,
            };
            if try_delta != delta {
                self.blocked = true;
            }
            return Vec3::new(to.x, height, to.y);
        }
        self.blocked = true;
        at
    }
}

/// The facing nearest a ground direction (x east, z south).
pub fn facing_of(dir: Vec2) -> Direction4 {
    if dir.x.abs() > dir.y.abs() {
        if dir.x > 0.0 {
            Direction4::East
        } else {
            Direction4::West
        }
    } else if dir.y > 0.0 {
        Direction4::South
    } else {
        Direction4::North
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::actor::Transition;

    struct Open;
    impl World for Open {
        fn ground(&self, x: f32, _: f32) -> Ground {
            if x > 5.0 {
                Ground::Wall
            } else {
                Ground::Floor(0.0)
            }
        }
    }

    struct NoFacts;
    impl Facts for NoFacts {
        fn count(&self, _: &str) -> u32 {
            0
        }
        fn flag(&self, _: &str) -> bool {
            false
        }
        fn gone(&self, _: &str) -> bool {
            false
        }
        fn act(&self) -> Option<u8> {
            None
        }
    }

    fn state(name: &str, motion: Motion, next: Vec<(&str, Cue)>) -> ActorState {
        ActorState {
            name: name.into(),
            motion,
            animation: None,
            speed: 1.0,
            contact: None,
            invulnerable: false,
            enter: None,
            next: next
                .into_iter()
                .map(|(to, when)| Transition {
                    to: to.into(),
                    when,
                })
                .collect(),
        }
    }

    fn actor(states: Vec<ActorState>) -> Actor {
        Actor {
            states,
            speed: 2.0,
            ..Actor::example()
        }
    }

    /// Runs `seconds` of steps, checking transitions as Play does; returns where it ends.
    fn run(actor: &Actor, mind: &mut Mind, mut at: Vec3, player: Vec3, seconds: f32) -> Vec3 {
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt).round() as usize {
            let seen = Seen {
                player,
                health: 1.0,
                facts: &NoFacts,
            };
            let mut roll = || 0.5;
            if let Some(next) = next_state(actor, mind, at, &seen, dt, &mut roll) {
                mind.enter(next, &actor.states[next], at, player);
            }
            mind.seen();
            at = mind.step(actor, at, player, &Open, dt);
        }
        at
    }

    #[test]
    fn it_chases_the_player_once_near_and_gives_up_when_far() {
        let a = actor(vec![
            state("idle", Motion::Still, vec![("chase", Cue::Near(3.0))]),
            state("chase", Motion::Chase, vec![("idle", Cue::Far(4.0))]),
        ]);
        let mut mind = Mind::new(Vec3::ZERO, 1);
        let far = Vec3::new(0.0, 0.0, -10.0);
        assert_eq!(run(&a, &mut mind, Vec3::ZERO, far, 1.0), Vec3::ZERO);
        let near = Vec3::new(0.0, 0.0, -2.5);
        let at = run(&a, &mut mind, Vec3::ZERO, near, 0.5);
        assert_eq!(mind.state, 1);
        assert!(at.z < -0.5, "walked north towards the player: {at}");
        assert_eq!(mind.facing, Direction4::North);
    }

    #[test]
    fn a_charge_rushes_until_a_wall_stops_it() {
        let a = actor(vec![
            state("wait", Motion::Still, vec![("charge", Cue::Near(10.0))]),
            state("charge", Motion::Charge, vec![("dizzy", Cue::Blocked)]),
            state("dizzy", Motion::Still, vec![]),
        ]);
        let mut mind = Mind::new(Vec3::ZERO, 1);
        let at = run(&a, &mut mind, Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0), 5.0);
        assert_eq!(mind.state, 2, "stopped by the wall at x = 5");
        assert!(at.x < 5.0 && at.x > 4.0, "{at}");
    }

    #[test]
    fn a_patrol_walks_its_points_and_arrives() {
        let a = actor(vec![
            state(
                "patrol",
                Motion::Patrol {
                    points: vec![(1.0, 0.0), (1.0, 1.0)],
                    looped: false,
                },
                vec![("home", Cue::Arrived)],
            ),
            state("home", Motion::Home, vec![]),
        ]);
        let mut mind = Mind::new(Vec3::ZERO, 1);
        let far = Vec3::new(0.0, 0.0, -50.0);
        let at = run(&a, &mut mind, Vec3::ZERO, far, 1.2);
        assert_eq!(mind.state, 1, "arrived at the last point");
        let at = run(&a, &mut mind, at, far, 2.0);
        assert!(at.distance(Vec3::ZERO) <= ARRIVED + 0.05, "back home: {at}");
    }

    #[test]
    fn wandering_stays_near_home_and_is_the_same_every_time() {
        let a = actor(vec![state(
            "wander",
            Motion::Wander {
                radius: 2.0,
                pause: 0.2,
            },
            vec![],
        )]);
        let far = Vec3::new(0.0, 0.0, -50.0);
        let path = |seed| {
            let mut mind = Mind::new(Vec3::ZERO, seed);
            let mut at = Vec3::ZERO;
            let mut points = Vec::new();
            for _ in 0..20 {
                at = run(&a, &mut mind, at, far, 0.5);
                assert!(at.length() <= 2.0 + 0.1, "{at}");
                points.push(at);
            }
            points
        };
        assert_eq!(path(7), path(7));
        assert_ne!(path(7), path(8));
    }

    #[test]
    fn a_hit_pushes_it_away_and_is_seen_once() {
        let a = actor(vec![
            state("idle", Motion::Still, vec![("ouch", Cue::Hurt)]),
            state("ouch", Motion::Still, vec![("idle", Cue::After(0.1))]),
        ]);
        let mut mind = Mind::new(Vec3::ZERO, 1);
        mind.hit(&a, Vec2::X);
        let at = run(&a, &mut mind, Vec3::ZERO, Vec3::new(0.0, 0.0, -50.0), 0.05);
        assert_eq!(mind.state, 1);
        assert!(at.x > 0.3, "pushed east: {at}");
        run(&a, &mut mind, at, Vec3::new(0.0, 0.0, -50.0), 0.2);
        assert_eq!(mind.state, 0, "the hit isn't seen again");
    }
}

//! Runs a 2D character's actions (see [`clockwork_core::action`]): which animation plays, how the
//! character may move meanwhile, when its hits and projectiles happen, and what it does when a
//! held button is let go. Pure (no Bevy systems), so it can be tested without a window.
//!
//! The runner knows nothing of items or controls. The caller starts actions (a button was
//! pressed), tells it which buttons are still held, and applies the [`Frame`] it returns to the
//! movement controller ([`Modifiers`]) and the world (hits, projectiles).

use bevy::math::Vec2;
use clockwork_core::action::{Action, HitArea, Hold, Motion, Moves};
use clockwork_core::character2d::{AnimationRef, Direction4};
use clockwork_core::item::Slot;

/// What started an action: the button whose being held keeps a hold going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Dodge,
    Slot(Slot),
}

/// A place on the ground relative to the character's feet (x east, z south, meters).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A box: `half_along` either way of `center` along `dir` and `half_across` across it.
    Box {
        center: Vec2,
        dir: Vec2,
        half_along: f32,
        half_across: f32,
    },
    Circle {
        center: Vec2,
        radius: f32,
    },
}

impl Shape {
    /// The shape moved by `by`.
    pub fn translated(self, by: Vec2) -> Shape {
        match self {
            Shape::Box {
                center,
                dir,
                half_along,
                half_across,
            } => Shape::Box {
                center: center + by,
                dir,
                half_along,
                half_across,
            },
            Shape::Circle { center, radius } => Shape::Circle {
                center: center + by,
                radius,
            },
        }
    }

    /// Which way a hit by the shape pushes something at `point`: along a box's direction (a
    /// swing pushes the way it swings), out from a circle's center (an explosion).
    pub fn push_at(&self, point: Vec2) -> Vec2 {
        match *self {
            Shape::Box { dir, .. } => dir.normalize_or_zero(),
            Shape::Circle { center, .. } => (point - center).normalize_or_zero(),
        }
    }

    /// Whether a disc of `radius` at `point` touches the shape.
    pub fn touches(&self, point: Vec2, radius: f32) -> bool {
        match *self {
            Shape::Circle { center, radius: r } => center.distance(point) <= r + radius,
            Shape::Box {
                center,
                dir,
                half_along,
                half_across,
            } => {
                let d = point - center;
                let along = d.dot(dir).abs();
                let across = d.dot(dir.perp()).abs();
                along <= half_along + radius && across <= half_across + radius
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub action: String,
    /// Which of the action's hit windows.
    pub window: usize,
    /// The first update of the window: count a target once, here.
    pub first: bool,
    pub shape: Shape,
    pub damage: f32,
    pub knockback: f32,
    /// The way the character faces, for knockback.
    pub dir: Vec2,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started(String),
    Ended(String),
    Hit(Hit),
    /// Throw or place a projectile at `offset` from the feet, going along `dir`.
    Spawn {
        projectile: String,
        offset: Vec2,
        dir: Vec2,
    },
}

/// What an action does to the animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose<'a> {
    pub animation: &'a AnimationRef,
    /// Seconds since the pose started (the animation loops itself).
    pub time: f32,
}

/// How an action constrains the movement controller this update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modifiers {
    /// Fraction of the normal walking speed.
    pub speed_factor: f32,
    /// Input is ignored: the character stands still (or slides).
    pub locked: bool,
    /// Moves the character with this velocity (m/s, ground plane) whatever the input.
    pub slide: Option<Vec2>,
    /// The character keeps facing this way.
    pub facing: Option<Direction4>,
}

impl Default for Modifiers {
    fn default() -> Self {
        Self {
            speed_factor: 1.0,
            locked: false,
            slide: None,
            facing: None,
        }
    }
}

/// The result of one update.
#[derive(Debug, Default)]
pub struct Frame<'a> {
    /// The animation to show instead of idle and walk, if an action is going on.
    pub pose: Option<Pose<'a>>,
    pub modifiers: Modifiers,
    pub invulnerable: bool,
    /// Hits from this direction (ground plane, unit) within this half angle (degrees) are blocked.
    pub guard: Option<(Vec2, f32)>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Free,
    Acting {
        action: usize,
        time: f32,
        source: Source,
        facing: Direction4,
        dir: Vec2,
    },
    Holding {
        action: usize,
        time: f32,
        source: Source,
        facing: Direction4,
        dir: Vec2,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Runner {
    state: State,
}

impl Default for Runner {
    fn default() -> Self {
        Self { state: State::Free }
    }
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The id of the action going on, if any.
    pub fn current<'a>(&self, moves: &'a Moves) -> Option<&'a str> {
        match self.state {
            State::Free => None,
            State::Acting { action, .. } | State::Holding { action, .. } => {
                moves.actions.get(action).map(|a| a.id.as_str())
            }
        }
    }

    /// The action going on, the facing it started with, and the seconds its current phase (the
    /// action, then its hold) has lasted.
    pub fn active<'a>(&self, moves: &'a Moves) -> Option<(&'a Action, Direction4, f32)> {
        match self.state {
            State::Free => None,
            State::Acting {
                action,
                time,
                facing,
                ..
            }
            | State::Holding {
                action,
                time,
                facing,
                ..
            } => moves.actions.get(action).map(|a| (a, facing, time)),
        }
    }

    pub fn is_free(&self) -> bool {
        self.state == State::Free
    }

    /// Whether a new action may start now: nothing going on, a hold (which any action can end),
    /// or an action that allows being cancelled by now.
    pub fn can_start(&self, moves: &Moves) -> bool {
        match self.state {
            State::Free | State::Holding { .. } => true,
            State::Acting { action, time, .. } => moves
                .actions
                .get(action)
                .and_then(|a| a.cancel_from)
                .is_some_and(|from| time >= from),
        }
    }

    /// Starts `action` facing `facing`, going along `dir` (unit, ground plane). False if there is
    /// no such action or the character is busy.
    pub fn start(
        &mut self,
        moves: &Moves,
        action: &str,
        source: Source,
        facing: Direction4,
        dir: Vec2,
    ) -> bool {
        if !self.can_start(moves) {
            return false;
        }
        let Some(index) = moves.actions.iter().position(|a| a.id == action) else {
            return false;
        };
        self.state = State::Acting {
            action: index,
            time: 0.0,
            source,
            facing,
            dir: dir.try_normalize().unwrap_or(Vec2::NEG_Y),
        };
        true
    }

    pub fn cancel(&mut self) {
        self.state = State::Free;
    }

    /// Advances by `dt` seconds. `held` says whether a source's button is down; `walking` whether
    /// the character is trying to walk (it picks the walking pose of a hold).
    pub fn update<'a>(
        &mut self,
        moves: &'a Moves,
        dt: f32,
        held: impl Fn(Source) -> bool,
        walking: bool,
    ) -> Frame<'a> {
        let mut frame = Frame::default();
        // A release can start an action, whose first update follows at once.
        for _ in 0..2 {
            match self.state.clone() {
                State::Free => break,
                State::Acting {
                    action,
                    time,
                    source,
                    facing,
                    dir,
                } => {
                    let Some(a) = moves.actions.get(action) else {
                        self.state = State::Free;
                        break;
                    };
                    let next = time + dt;
                    if time == 0.0 {
                        frame.events.push(Event::Started(a.id.clone()));
                    }
                    acting(a, time, next, facing, dir, &mut frame);
                    if next >= a.duration {
                        frame.events.push(Event::Ended(a.id.clone()));
                        self.state = match &a.hold {
                            Some(_) if held(source) => State::Holding {
                                action,
                                time: 0.0,
                                source,
                                facing,
                                dir,
                            },
                            _ => State::Free,
                        };
                    } else {
                        self.state = State::Acting {
                            action,
                            time: next,
                            source,
                            facing,
                            dir,
                        };
                    }
                    break;
                }
                State::Holding {
                    action,
                    time,
                    source,
                    facing,
                    dir,
                } => {
                    let Some(hold) = moves.actions.get(action).and_then(|a| a.hold.as_ref()) else {
                        self.state = State::Free;
                        break;
                    };
                    if held(source) {
                        holding(hold, facing, dir, &mut frame);
                        self.state = State::Holding {
                            action,
                            time: time + dt,
                            source,
                            facing,
                            dir,
                        };
                        break;
                    }
                    // Let go: perform the release action, if the pose was held long enough.
                    self.state = State::Free;
                    let release = hold.release.as_ref().filter(|r| time >= r.min_time);
                    if let Some(r) = release {
                        self.start(moves, &r.action, source, facing, dir);
                        continue;
                    }
                    break;
                }
            }
        }
        frame.pose = self.pose(moves, walking);
        frame
    }
}

fn along(dir: Vec2, facing_offset: Vec2) -> Vec2 {
    // `facing_offset` is (forward, sideways).
    dir * facing_offset.x + dir.perp() * facing_offset.y
}

fn acting(a: &Action, from: f32, to: f32, facing: Direction4, dir: Vec2, frame: &mut Frame<'_>) {
    frame.modifiers.facing = Some(facing);
    match &a.motion {
        Motion::Rooted => frame.modifiers.locked = true,
        Motion::Walk(factor) => frame.modifiers.speed_factor = *factor,
        Motion::Slide(_) => {
            frame.modifiers.locked = true;
            let speed = a.slide_speed(from).unwrap_or(0.0);
            frame.modifiers.slide = Some(dir * speed);
        }
    }
    frame.invulnerable = a
        .invulnerable
        .is_some_and(|(start, end)| to > start && from < end);
    for (window, hit) in a.hits.iter().enumerate() {
        if to <= hit.from || from >= hit.until {
            continue;
        }
        let shape = match hit.area {
            HitArea::Front {
                near,
                far,
                half_width,
            } => Shape::Box {
                center: dir * ((near + far) / 2.0),
                dir,
                half_along: (far - near) / 2.0,
                half_across: half_width,
            },
            HitArea::Around { radius } => Shape::Circle {
                center: Vec2::ZERO,
                radius,
            },
        };
        frame.events.push(Event::Hit(Hit {
            action: a.id.clone(),
            window,
            first: from <= hit.from,
            shape,
            damage: hit.damage,
            knockback: hit.knockback,
            dir,
        }));
    }
    for spawn in &a.spawns {
        if from < spawn.at && spawn.at <= to || (spawn.at == 0.0 && from == 0.0) {
            frame.events.push(Event::Spawn {
                projectile: spawn.projectile.clone(),
                offset: along(dir, Vec2::new(spawn.ahead, 0.0)),
                dir,
            });
        }
    }
}

fn holding(hold: &Hold, facing: Direction4, dir: Vec2, frame: &mut Frame<'_>) {
    frame.modifiers.facing = Some(facing);
    frame.modifiers.speed_factor = hold.walk_factor;
    frame.modifiers.locked = hold.walk_factor <= 0.0;
    frame.guard = hold.guard.map(|angle| (dir, angle));
}

impl Runner {
    /// The pose to show now, if an action is going on.
    pub fn pose<'a>(&self, moves: &'a Moves, walking: bool) -> Option<Pose<'a>> {
        match self.state {
            State::Free => None,
            State::Acting {
                action,
                time,
                facing,
                ..
            } => {
                let a = moves.actions.get(action)?;
                Some(Pose {
                    animation: a.animation.get(facing),
                    time,
                })
            }
            State::Holding {
                action,
                time,
                facing,
                ..
            } => {
                let hold = moves.actions.get(action)?.hold.as_ref()?;
                let directional = match (&hold.walk_animation, walking) {
                    (Some(walk), true) => walk,
                    _ => &hold.animation,
                };
                Some(Pose {
                    animation: directional.get(facing),
                    time,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clockwork_core::action::{HitWindow, Release, Spawn};
    use clockwork_core::character2d::{Animations2d, Character2d, Directional, Movement2d};

    use super::*;

    fn directional(name: &str) -> Directional {
        let one = |flip_x| AnimationRef {
            name: name.into(),
            flip_x,
            flip_y: false,
        };
        Directional {
            north: one(false),
            east: one(false),
            south: one(false),
            west: one(true),
        }
    }

    fn action(id: &str, duration: f32) -> Action {
        Action {
            id: id.into(),
            animation: directional(id),
            duration,
            motion: Motion::Rooted,
            hits: vec![],
            invulnerable: None,
            spawns: vec![],
            hold: None,
            cancel_from: None,
            attachments: vec![],
        }
    }

    fn character() -> Character2d {
        let mut slash = action("slash", 0.25);
        slash.hits = vec![HitWindow {
            from: 0.05,
            until: 0.15,
            area: HitArea::Front {
                near: 0.2,
                far: 1.4,
                half_width: 0.7,
            },
            damage: 1.0,
            knockback: 0.5,
        }];
        slash.hold = Some(Hold {
            animation: directional("charge"),
            walk_animation: Some(directional("charge_walk")),
            walk_factor: 0.7,
            guard: None,
            release: Some(Release {
                action: "spin".into(),
                min_time: 1.0,
            }),
        });
        let mut dodge = action("dodge", 0.4);
        dodge.motion = Motion::Slide(vec![(0.0, 7.0), (0.3, 0.0)]);
        dodge.invulnerable = Some((0.0, 0.3));
        dodge.cancel_from = Some(0.35);
        let mut shoot = action("shoot", 0.2);
        shoot.spawns = vec![Spawn {
            at: 0.1,
            projectile: "arrow".into(),
            ahead: 0.5,
        }];
        let mut shield = action("shield", 0.1);
        shield.hold = Some(Hold {
            animation: directional("shield"),
            walk_animation: None,
            walk_factor: 0.6,
            guard: Some(60.0),
            release: None,
        });
        Character2d {
            movement: Movement2d {
                speed: 4.0,
                acceleration: None,
                radius: 0.4,
                gravity: 9.8,
            },
            animations: Animations2d {
                idle: directional("idle"),
                walk: directional("walk"),
            },
            camera: None,
            world_camera: None,
            kit: Default::default(),
            stats: None,
            inventory: None,
            actions: vec![slash, action("spin", 0.5), dodge, shoot, shield],
            projectiles: vec![],
            dodge: Some("dodge".into()),
        }
    }

    const DT: f32 = 1.0 / 60.0;

    fn run(runner: &mut Runner, c: &Moves, seconds: f32, held: bool) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(seconds / DT).round() as usize {
            events.extend(runner.update(c, DT, |_| held, false).events);
        }
        events
    }

    #[test]
    fn a_slash_hits_during_its_window_only_once_first() {
        let c = character().moves();
        let mut r = Runner::new();
        assert!(r.start(
            &c,
            "slash",
            Source::Slot(Slot::B),
            Direction4::East,
            Vec2::X
        ));
        let events = run(&mut r, &c, 0.3, false);
        let hits: Vec<&Hit> = events
            .iter()
            .filter_map(|e| match e {
                Event::Hit(h) => Some(h),
                _ => None,
            })
            .collect();
        // About six updates inside the 0.1 s window, the first one marked.
        assert!((5..=8).contains(&hits.len()), "{}", hits.len());
        assert_eq!(hits.iter().filter(|h| h.first).count(), 1);
        assert!(matches!(hits[0].shape, Shape::Box { .. }));
        assert!(hits[0].shape.touches(Vec2::new(1.0, 0.0), 0.2));
        assert!(!hits[0].shape.touches(Vec2::new(0.0, 1.0), 0.2));
        assert!(events.contains(&Event::Started("slash".into())));
        assert!(events.contains(&Event::Ended("slash".into())));
        assert!(r.is_free());
    }

    #[test]
    fn holding_charges_and_releasing_spins_only_when_charged() {
        let c = character().moves();
        // Released too early: back to normal, no spin.
        let mut r = Runner::new();
        r.start(
            &c,
            "slash",
            Source::Slot(Slot::B),
            Direction4::South,
            Vec2::Y,
        );
        run(&mut r, &c, 0.5, true);
        assert_eq!(r.current(&c), Some("slash"));
        assert!(r.pose(&c, false).unwrap().animation.name == "charge");
        assert!(r.pose(&c, true).unwrap().animation.name == "charge_walk");
        run(&mut r, &c, 0.1, false);
        assert!(r.is_free());
        // Held long enough: the spin follows the release.
        let mut r = Runner::new();
        r.start(
            &c,
            "slash",
            Source::Slot(Slot::B),
            Direction4::South,
            Vec2::Y,
        );
        run(&mut r, &c, 1.4, true);
        let frame = r.update(&c, DT, |_| false, false);
        assert!(frame.events.contains(&Event::Started("spin".into())));
        assert_eq!(r.current(&c), Some("spin"));
        run(&mut r, &c, 0.6, true);
        assert!(r.is_free());
    }

    #[test]
    fn rolling_slides_is_invulnerable_and_cancels_late() {
        let c = character().moves();
        let mut r = Runner::new();
        r.start(&c, "dodge", Source::Dodge, Direction4::East, Vec2::X);
        let f = r.update(&c, DT, |_| false, false);
        assert_eq!(f.modifiers.slide, Some(Vec2::new(7.0, 0.0)));
        assert!(f.modifiers.locked && f.invulnerable);
        assert!(!r.can_start(&c));
        run(&mut r, &c, 0.34, false);
        let f = r.update(&c, DT, |_| false, false);
        assert_eq!(f.modifiers.slide, Some(Vec2::ZERO));
        assert!(!f.invulnerable);
        assert!(r.can_start(&c));
        assert!(!r.start(&c, "nothing", Source::Dodge, Direction4::East, Vec2::X));
    }

    #[test]
    fn spawns_happen_once_at_their_time() {
        let c = character().moves();
        let mut r = Runner::new();
        r.start(
            &c,
            "shoot",
            Source::Slot(Slot::A),
            Direction4::North,
            Vec2::NEG_Y,
        );
        let events = run(&mut r, &c, 0.3, false);
        let spawns: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, Event::Spawn { .. }))
            .collect();
        assert_eq!(spawns.len(), 1);
        if let Event::Spawn { offset, dir, .. } = spawns[0] {
            assert!((offset - Vec2::new(0.0, -0.5)).length() < 1e-5);
            assert_eq!(*dir, Vec2::NEG_Y);
        }
    }

    #[test]
    fn a_raised_shield_guards_and_slows_while_held() {
        let c = character().moves();
        let mut r = Runner::new();
        r.start(
            &c,
            "shield",
            Source::Slot(Slot::B),
            Direction4::East,
            Vec2::X,
        );
        run(&mut r, &c, 0.2, true);
        let f = r.update(&c, DT, |_| true, true);
        assert_eq!(f.modifiers.speed_factor, 0.6);
        assert!(!f.modifiers.locked);
        assert_eq!(f.guard, Some((Vec2::X, 60.0)));
        assert_eq!(f.modifiers.facing, Some(Direction4::East));
        // Letting go lowers it.
        r.update(&c, DT, |_| false, false);
        assert!(r.is_free());
    }
}

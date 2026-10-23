//! Projectiles in flight (arrows, bombs, returning throws; see [`clockwork_core::action::Projectile`]):
//! motion, walls, returning, bursting. Pure, so it can be tested without a window.
//!
//! Positions are (x east, z south) on the ground plane in meters, plus a height read from the
//! world's floor so a projectile can be drawn on it.

use bevy::math::{Vec2, Vec3};
use clockwork_core::action::{Projectile, ProjectileMotion};

use crate::controller::{Ground, World};

#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub id: String,
    /// Tells projectiles of one kind apart (a projectile hurts something once).
    pub uid: u64,
    pub position: Vec2,
    /// The floor height under it.
    pub height: f32,
    pub age: f32,
    dir: Vec2,
    /// Distance flown outwards, for projectiles that return.
    flown: f32,
    returning: bool,
    /// The seconds the burst has lasted, once it has started.
    burst: Option<f32>,
}

impl Live {
    /// The way it was thrown (unit, ground plane), which it keeps facing even while returning.
    pub fn dir(&self) -> Vec2 {
        self.dir
    }

    /// Whether it is bursting now.
    pub fn is_bursting(&self) -> bool {
        self.burst.is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A projectile touches things on its way: damage in a disc.
    Hit {
        id: String,
        uid: u64,
        position: Vec2,
        radius: f32,
        damage: f32,
        dir: Vec2,
    },
    /// An explosion is going on (every update while it lasts; `first` once).
    Burst {
        id: String,
        position: Vec2,
        radius: f32,
        damage: f32,
        knockback: f32,
        first: bool,
    },
    /// It is gone (hit a wall, flew out, burst through, came back).
    Ended { id: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Projectiles {
    live: Vec<Live>,
    next_uid: u64,
}

impl Projectiles {
    pub fn iter(&self) -> impl Iterator<Item = &Live> {
        self.live.iter()
    }

    /// How many of `id` exist now.
    pub fn count(&self, id: &str) -> usize {
        self.live.iter().filter(|p| p.id == id).count()
    }

    pub fn clear(&mut self) {
        self.live.clear();
    }

    /// Starts a projectile of `spec` at `position`, going along `dir`. False if the spec's limit
    /// of simultaneous projectiles is reached.
    pub fn spawn(&mut self, spec: &Projectile, position: Vec2, height: f32, dir: Vec2) -> bool {
        if spec
            .limit
            .is_some_and(|limit| self.count(&spec.id) >= limit as usize)
        {
            return false;
        }
        self.next_uid += 1;
        self.live.push(Live {
            id: spec.id.clone(),
            uid: self.next_uid,
            position,
            height,
            age: 0.0,
            dir: dir.try_normalize().unwrap_or(Vec2::NEG_Y),
            flown: 0.0,
            returning: false,
            burst: None,
        });
        true
    }

    /// A projectile hit something: arrows stop, returning ones turn back, the rest carry on.
    pub fn on_hit(&mut self, specs: &[Projectile], uid: u64) {
        let Some(at) = self.live.iter().position(|p| p.uid == uid) else {
            return;
        };
        match specs
            .iter()
            .find(|s| s.id == self.live[at].id)
            .map(|s| &s.motion)
        {
            Some(ProjectileMotion::Return { .. }) => self.live[at].returning = true,
            Some(ProjectileMotion::Straight { .. }) => {
                self.live.remove(at);
            }
            _ => {}
        }
    }

    /// Advances every projectile by `dt`; `owner` is where returning projectiles go back to.
    pub fn update(
        &mut self,
        specs: &[Projectile],
        owner: Vec3,
        world: &impl World,
        dt: f32,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let owner_ground = Vec2::new(owner.x, owner.z);
        self.live.retain_mut(|p| {
            let Some(spec) = specs.iter().find(|s| s.id == p.id) else {
                return false;
            };
            // A burst only counts its time.
            if let Some(time) = &mut p.burst {
                let burst = spec
                    .burst
                    .as_ref()
                    .expect("bursting projectiles have a burst");
                let first = *time == 0.0;
                *time += dt;
                events.push(Event::Burst {
                    id: p.id.clone(),
                    position: p.position,
                    radius: burst.radius,
                    damage: burst.damage,
                    knockback: burst.knockback,
                    first,
                });
                if *time >= burst.duration {
                    events.push(Event::Ended { id: p.id.clone() });
                    return false;
                }
                return true;
            }
            p.age += dt;
            let mut ended = false;
            match spec.motion {
                ProjectileMotion::Stationary => {}
                ProjectileMotion::Straight { speed } => {
                    let step = p.dir * speed * dt;
                    if !advance(p, step, spec, world) {
                        ended = true;
                    }
                }
                ProjectileMotion::Return { speed, range } => {
                    if p.returning {
                        let to_owner = owner_ground - p.position;
                        let step = speed * dt;
                        if to_owner.length() <= step.max(spec.radius) {
                            ended = true;
                        } else {
                            p.position += to_owner.normalize() * step;
                        }
                    } else {
                        let step = p.dir * speed * dt;
                        if !advance(p, step, spec, world) {
                            p.returning = true;
                        }
                        p.flown += step.length();
                        if p.flown >= range {
                            p.returning = true;
                        }
                    }
                }
            }
            if let Ground::Floor(h) = world.ground_at(p.position.x, p.height + 0.5, p.position.y) {
                p.height = h;
            }
            if spec.damage > 0.0 && !ended {
                events.push(Event::Hit {
                    id: p.id.clone(),
                    uid: p.uid,
                    position: p.position,
                    radius: spec.radius,
                    damage: spec.damage,
                    dir: p.dir,
                });
            }
            let expired =
                p.age >= spec.lifetime && !matches!(spec.motion, ProjectileMotion::Return { .. });
            if ended || (expired && spec.burst.is_none()) {
                events.push(Event::Ended { id: p.id.clone() });
                return false;
            }
            if expired || (ended && spec.burst.is_some()) {
                p.burst = Some(0.0);
            }
            true
        });
        events
    }
}

/// Moves by `step` unless a wall is in the way; false when it stopped (at a wall, if the spec
/// says projectiles stop there).
fn advance(p: &mut Live, step: Vec2, spec: &Projectile, world: &impl World) -> bool {
    let next = p.position + step;
    if spec.stops_at_walls
        && matches!(
            world.ground_at(next.x, p.height + 0.5, next.y),
            Ground::Wall
        )
    {
        return false;
    }
    p.position = next;
    true
}

#[cfg(test)]
mod tests {
    use clockwork_core::action::Burst;

    use super::*;

    struct Field;

    impl World for Field {
        fn ground(&self, x: f32, _z: f32) -> Ground {
            if x >= 10.0 {
                Ground::Wall
            } else {
                Ground::Floor(0.0)
            }
        }
    }

    fn spec(id: &str, motion: ProjectileMotion) -> Projectile {
        Projectile {
            id: id.into(),
            motion,
            lifetime: 3.0,
            radius: 0.2,
            damage: 1.0,
            burst: None,
            stops_at_walls: true,
            limit: None,
            color: [1.0; 3],
            sprite: None,
        }
    }

    const DT: f32 = 1.0 / 60.0;

    fn run(p: &mut Projectiles, specs: &[Projectile], owner: Vec3, seconds: f32) -> Vec<Event> {
        let mut out = Vec::new();
        for _ in 0..(seconds / DT) as usize {
            out.extend(p.update(specs, owner, &Field, DT));
        }
        out
    }

    #[test]
    fn arrows_fly_hit_and_stop_at_walls() {
        let specs = [spec("arrow", ProjectileMotion::Straight { speed: 10.0 })];
        let mut p = Projectiles::default();
        assert!(p.spawn(&specs[0], Vec2::ZERO, 0.0, Vec2::X));
        let events = run(&mut p, &specs, Vec3::ZERO, 0.5);
        assert!(events.iter().any(|e| matches!(e, Event::Hit { .. })));
        assert!((p.iter().next().unwrap().position.x - 5.0).abs() < 0.2);
        let events = run(&mut p, &specs, Vec3::ZERO, 1.0);
        assert!(events.contains(&Event::Ended { id: "arrow".into() }));
        assert_eq!(p.count("arrow"), 0);
    }

    #[test]
    fn hitting_something_stops_arrows_and_turns_returning_ones() {
        let specs = [
            spec("arrow", ProjectileMotion::Straight { speed: 10.0 }),
            spec(
                "returning",
                ProjectileMotion::Return {
                    speed: 8.0,
                    range: 4.0,
                },
            ),
        ];
        let mut p = Projectiles::default();
        p.spawn(&specs[0], Vec2::ZERO, 0.0, Vec2::X);
        p.spawn(&specs[1], Vec2::ZERO, 0.0, Vec2::X);
        let uids: Vec<u64> = p.iter().map(|l| l.uid).collect();
        assert_ne!(uids[0], uids[1]);
        p.on_hit(&specs, uids[0]);
        assert_eq!(p.count("arrow"), 0);
        p.on_hit(&specs, uids[1]);
        let before = p.iter().next().unwrap().position.x;
        run(&mut p, &specs, Vec3::new(-5.0, 0.0, 0.0), 0.2);
        assert!(
            p.iter().next().unwrap().position.x < before,
            "it turned back"
        );
        p.on_hit(&specs, 999);
    }

    #[test]
    fn bombs_burst_after_their_fuse_and_respect_the_limit() {
        let mut bomb = spec("bomb", ProjectileMotion::Stationary);
        bomb.damage = 0.0;
        bomb.lifetime = 1.0;
        bomb.limit = Some(1);
        bomb.burst = Some(Burst {
            radius: 1.5,
            damage: 4.0,
            duration: 0.3,
            knockback: 1.0,
        });
        let specs = [bomb];
        let mut p = Projectiles::default();
        assert!(p.spawn(&specs[0], Vec2::new(2.0, 0.0), 0.0, Vec2::X));
        assert!(!p.spawn(&specs[0], Vec2::ZERO, 0.0, Vec2::X));
        let early = run(&mut p, &specs, Vec3::ZERO, 0.9);
        assert!(early.is_empty());
        let events = run(&mut p, &specs, Vec3::ZERO, 0.6);
        let bursts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Burst { first, damage, .. } => Some((*first, *damage)),
                _ => None,
            })
            .collect();
        assert!(bursts.len() >= 15, "{}", bursts.len());
        assert_eq!(bursts.iter().filter(|b| b.0).count(), 1);
        assert!(bursts.iter().all(|b| b.1 == 4.0));
        assert_eq!(p.count("bomb"), 0);
        // The limit is free again.
        assert!(p.spawn(&specs[0], Vec2::ZERO, 0.0, Vec2::X));
    }

    #[test]
    fn returning_projectiles_go_out_and_come_back_to_the_owner() {
        let specs = [spec(
            "returning",
            ProjectileMotion::Return {
                speed: 8.0,
                range: 4.0,
            },
        )];
        let mut p = Projectiles::default();
        p.spawn(&specs[0], Vec2::new(0.5, 0.0), 0.0, Vec2::X);
        run(&mut p, &specs, Vec3::ZERO, 0.4);
        let out = p.iter().next().unwrap().position.x;
        assert!(out > 3.0, "{out}");
        // The owner has moved: it follows.
        let events = run(&mut p, &specs, Vec3::new(0.0, 0.0, 2.0), 1.5);
        assert!(events.contains(&Event::Ended {
            id: "returning".into()
        }));
        assert_eq!(p.count("returning"), 0);
    }
}

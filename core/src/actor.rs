//! Actors: scene nodes that act on their own while played (enemies, NPCs, things that move). An
//! actor is a small state machine: each state says how it moves, what it shows and when it
//! changes to another state. Nothing in it names a game: a game's enemy is its numbers (speed,
//! health, how near the player must come) and the states its code goes through, written as data
//! (DESIGN.md §3, Tier 3).
//!
//! ```ron
//! Actor((
//!     health: Some(2.0), radius: 0.4, contact: 1.0, speed: 1.5,
//!     states: [
//!         (name: "wander", motion: Wander(radius: 3.0, pause: 1.0),
//!          next: [(to: "chase", when: Near(4.0))]),
//!         (name: "chase", motion: Chase, speed: 1.6, animation: Some(One("walk")),
//!          next: [(to: "wander", when: Far(6.0)), (to: "stunned", when: Hurt)]),
//!         (name: "stunned", contact: Some(0.0), next: [(to: "chase", when: After(0.6))]),
//!     ],
//! ))
//! ```

use serde::{Deserialize, Serialize};

use crate::{
    action::Projectile,
    behavior::{Condition, Effect},
    character2d::{Direction4, Directional},
};

/// A thing that acts on its own: it starts in its first state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    /// It can be hit and beaten with this much health; without, hits don't hurt it.
    #[serde(default)]
    pub health: Option<f32>,
    /// Its size on the ground, in meters: for hits, touching the player and blocking.
    pub radius: f32,
    /// The player can't walk through it.
    #[serde(default)]
    pub solid: bool,
    /// Damage done to the player by touching it (none when 0).
    #[serde(default)]
    pub contact: f32,
    /// Meters per second when a state moves it (times the state's own `speed`).
    #[serde(default = "one")]
    pub speed: f32,
    /// Meters it is pushed back by a hit.
    #[serde(default)]
    pub knockback: f32,
    /// What it leaves behind when beaten, handed straight to the player.
    #[serde(default)]
    pub drop: Option<(String, u32)>,
    pub states: Vec<ActorState>,
}

fn one() -> f32 {
    1.0
}

/// One thing an actor does until a transition takes it elsewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorState {
    pub name: String,
    #[serde(default)]
    pub motion: Motion,
    /// The animation its node shows (of the sprite or model it shows); its node's own without.
    #[serde(default)]
    pub animation: Option<Pose>,
    /// Times the actor's speed.
    #[serde(default = "one")]
    pub speed: f32,
    /// Contact damage in this state instead of the actor's (0: harmless while stunned).
    #[serde(default)]
    pub contact: Option<f32>,
    /// Hits don't hurt it in this state.
    #[serde(default)]
    pub invulnerable: bool,
    /// What happens on entering the state: a sound, a dialog, a flag (any trigger effect).
    #[serde(default)]
    pub enter: Option<Effect>,
    /// A projectile it fires in this state (a thrown rock, an arrow, a fireball).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoot: Option<Shot>,
    /// Where it goes next: the first transition whose cue holds wins, checked every step.
    #[serde(default)]
    pub next: Vec<Transition>,
}

/// A projectile an actor fires, once each time it is in the state `at` seconds. The projectile
/// is written like a character's ([`crate::action::Projectile`]); its damage hurts the player
/// and a burst hurts the player inside it. Its id names it across the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub projectile: Projectile,
    /// Seconds into the state.
    #[serde(default)]
    pub at: f32,
    /// Which way it flies.
    #[serde(default)]
    pub aim: Aim,
    /// Meters in front of the actor it starts.
    #[serde(default)]
    pub ahead: f32,
}

/// Which way an actor's shot flies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Aim {
    /// At where the player is when it fires.
    #[default]
    Player,
    /// The way the actor faces.
    Facing,
}

/// How an actor moves in a state. Positions are on the ground, in meters.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Motion {
    /// Stays where it is.
    #[default]
    Still,
    /// Walks to random points within `radius` of where it was placed, waiting `pause` seconds
    /// at each.
    Wander { radius: f32, pause: f32 },
    /// Walks towards the player.
    Chase,
    /// Walks away from the player.
    Flee,
    /// Rushes in a straight line the way the player was when the state began, until stopped.
    Charge,
    /// Walks back to where it was placed.
    Home,
    /// Walks through `points` (meters from where it was placed, x east and z south), then back
    /// to the first if `looped`.
    Patrol {
        points: Vec<(f32, f32)>,
        #[serde(default)]
        looped: bool,
    },
}

/// What an actor shows in a state: one animation, or one per facing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pose {
    One(String),
    Facing(Directional),
}

impl Pose {
    /// The animation's name and whether it is mirrored, facing `facing`.
    pub fn get(&self, facing: Direction4) -> (&str, bool) {
        match self {
            Pose::One(name) => (name, false),
            Pose::Facing(d) => {
                let r = d.get(facing);
                (&r.name, r.flip_x)
            }
        }
    }
}

/// Going to the state named `to` when `when` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub to: String,
    pub when: Cue,
}

/// What an actor reacts to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Cue {
    /// The player is within this many meters.
    Near(f32),
    /// The player is farther than this many meters.
    Far(f32),
    /// The state has lasted this many seconds.
    After(f32),
    /// It was hit since the last step.
    Hurt,
    /// Its health is below this.
    HealthBelow(f32),
    /// It got where its motion goes: a wander point, home, a patrol's last point.
    Arrived,
    /// A wall, a hole or the edge of the world stopped it.
    Blocked,
    /// At random, this often per second on average.
    Chance(f32),
    /// A condition of the game holds (items carried, flags, nodes gone, the act).
    When(Condition),
    Not(Box<Cue>),
    All(Vec<Cue>),
    Any(Vec<Cue>),
}

impl Actor {
    /// A wandering enemy that chases the player when near: the Inspector's starting point.
    pub fn example() -> Self {
        let state = |name: &str, motion, next: Vec<(&str, Cue)>| ActorState {
            name: name.into(),
            motion,
            animation: None,
            speed: 1.0,
            contact: None,
            invulnerable: false,
            enter: None,
            shoot: None,
            next: next
                .into_iter()
                .map(|(to, when)| Transition {
                    to: to.into(),
                    when,
                })
                .collect(),
        };
        Actor {
            health: Some(2.0),
            radius: 0.4,
            solid: true,
            contact: 1.0,
            speed: 1.2,
            knockback: 0.8,
            drop: None,
            states: vec![
                state(
                    "wander",
                    Motion::Wander {
                        radius: 2.5,
                        pause: 1.0,
                    },
                    vec![("chase", Cue::Near(3.5))],
                ),
                ActorState {
                    speed: 1.5,
                    ..state(
                        "chase",
                        Motion::Chase,
                        vec![("stunned", Cue::Hurt), ("wander", Cue::Far(6.0))],
                    )
                },
                ActorState {
                    contact: Some(0.0),
                    ..state("stunned", Motion::Still, vec![("chase", Cue::After(0.5))])
                },
            ],
        }
    }

    /// The state of this name.
    pub fn state(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    /// What is wrong with it: no states, or transitions to states it doesn't have.
    pub fn problems(&self) -> Vec<String> {
        if self.states.is_empty() {
            return vec!["an actor needs at least one state".into()];
        }
        let mut problems = Vec::new();
        for state in &self.states {
            for t in &state.next {
                if self.state(&t.to).is_none() {
                    problems.push(format!(
                        "state \"{}\" goes to \"{}\", which the actor doesn't have",
                        state.name, t.to
                    ));
                }
            }
            if let Some(shot) = &state.shoot {
                let p = &shot.projectile;
                if !(shot.at.is_finite() && shot.at >= 0.0) {
                    problems.push(format!(
                        "state \"{}\" shoots at {} s, not a time in the state",
                        state.name, shot.at
                    ));
                }
                if !(p.lifetime.is_finite() && p.lifetime > 0.0) {
                    problems.push(format!(
                        "state \"{}\" shoots \"{}\", which lasts no time",
                        state.name, p.id
                    ));
                }
            }
        }
        problems
    }

    /// The projectiles its states fire.
    pub fn shots(&self) -> impl Iterator<Item = &Shot> {
        self.states.iter().filter_map(|s| s.shoot.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_doc_example_reads_and_names_only_its_own_states() {
        let actor: Actor = ron::from_str(
            r#"(
                health: Some(2.0), radius: 0.4, contact: 1.0, speed: 1.5,
                states: [
                    (name: "wander", motion: Wander(radius: 3.0, pause: 1.0),
                     next: [(to: "chase", when: Near(4.0))]),
                    (name: "chase", motion: Chase, speed: 1.6, animation: Some(One("walk")),
                     next: [(to: "wander", when: Far(6.0)), (to: "stunned", when: Hurt)]),
                    (name: "stunned", contact: Some(0.0), next: [(to: "chase", when: After(0.6))]),
                ],
            )"#,
        )
        .unwrap();
        assert_eq!(actor.states.len(), 3);
        assert!(actor.problems().is_empty());
        assert!(Actor::example().problems().is_empty());
        assert_eq!(actor.states.len(), 3);
        assert!(actor.problems().is_empty());
        let mut broken = actor.clone();
        broken.states[0].next[0].to = "dance".into();
        assert_eq!(
            broken.problems(),
            ["state \"wander\" goes to \"dance\", which the actor doesn't have"]
        );
    }

    #[test]
    fn cues_and_motions_read_as_written() {
        let cue: Cue = ron::from_str(r#"All([Near(2.0), Not(When(Flag("asleep")))])"#).unwrap();
        assert_eq!(
            cue,
            Cue::All(vec![
                Cue::Near(2.0),
                Cue::Not(Box::new(Cue::When(Condition::Flag("asleep".into()))))
            ])
        );
        let motion: Motion = ron::from_str("Patrol(points: [(2.0, 0.0), (2.0, 2.0)])").unwrap();
        assert_eq!(
            motion,
            Motion::Patrol {
                points: vec![(2.0, 0.0), (2.0, 2.0)],
                looped: false
            }
        );
    }
}

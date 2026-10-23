//! Parts of characters that are assets of their own: how a character moves, which animations
//! play for what, and its stats. A character names them (or writes them in place); characters
//! drawn alike can share their animations, characters that should feel alike their movement.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    character::{Movement, StateAnimations},
    character2d::{Animations2d, Movement2d},
    recipe::Output,
};

/// How a character moves: walking on a map seen from above (also upright in 3D worlds), or
/// running and jumping as a 3D character.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnyMovement {
    TopDown(Movement2d),
    Platformer(Movement),
}

/// Which animations play for what: by facing for sprites, by state for 3D models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnyAnimations {
    Directional(Animations2d),
    States(StateAnimations),
}

/// A played character's health, and how long it can't be hurt again after a hit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    pub health: f32,
    /// Seconds.
    #[serde(default = "hurt_time")]
    pub hurt_time: f32,
}

fn hurt_time() -> f32 {
    1.0
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            health: 6.0,
            hurt_time: hurt_time(),
        }
    }
}

pub const STATS: &str = "stats";

pub fn builtin_stats() -> OutputRef {
    OutputRef::new(BUILTIN, STATS)
}

pub(crate) fn builtins() -> Vec<Output> {
    vec![Output::Stats {
        id: STATS.into(),
        stats: Slot::Inline(Stats::default()),
    }]
}

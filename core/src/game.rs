//! A game's own assets: its rules (what happens when the player goes down, lives, a time limit,
//! a goal), and game settings that bundle the assets a whole game uses unless a node says
//! otherwise: a scene names one as its preset.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    recipe::Output,
};

/// What happens when the played character runs out of health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WhenDown {
    /// Back at the start, healed.
    #[default]
    StartOver,
    /// Back at the last checkpoint touched (or the start), healed.
    LastCheckpoint,
    /// Play stops.
    StopPlaying,
}

/// What wins the game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    /// Carrying this many of an item.
    Collect { item: String, count: u32 },
    /// Reaching the node with this name.
    Reach(String),
    /// Beating every target, and every actor that can be beaten.
    DestroyAll,
    /// Anything a condition can say (see [`crate::behavior::Condition`]).
    When(crate::behavior::Condition),
}

/// The rules of a game.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    pub when_down: WhenDown,
    /// Times the player can go down before the game is over; without, endless.
    #[serde(default)]
    pub lives: Option<u32>,
    /// Seconds to win in; when they run out the player goes down.
    #[serde(default)]
    pub time_limit: Option<f32>,
    #[serde(default)]
    pub goal: Option<Goal>,
}

/// The assets a whole game uses, each `"plugin/output"`; a played node's own choices and then
/// its character's come after them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameSettings {
    #[serde(default)]
    pub rules: Option<String>,
    #[serde(default)]
    pub flow: Option<String>,
    #[serde(default)]
    pub camera: Option<String>,
    #[serde(default)]
    pub hud: Option<String>,
    #[serde(default)]
    pub ui: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub menus: Option<Vec<String>>,
    #[serde(default)]
    pub stats: Option<String>,
    #[serde(default)]
    pub inventory: Option<String>,
}

pub const RULES: &str = "rules";

pub fn builtin_rules() -> OutputRef {
    OutputRef::new(BUILTIN, RULES)
}

pub(crate) fn builtins() -> Vec<Output> {
    vec![Output::Rules {
        id: RULES.into(),
        rules: Slot::Inline(Rules::default()),
    }]
}

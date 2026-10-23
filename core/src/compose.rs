//! Composition plugins (`compose.ron`): plugins made of other plugins' outputs instead of a recipe.
//! They hold no game data and no offsets, only references and choices: "this runner, with that
//! hero's sword and bow; the swing is the runner's punch; a bit faster". The editor's plugin editor writes them.
//!
//! A composition plugin's manifest names no game (`[source] game = "composition"`, no hashes);
//! the plugins it uses need their own games in the Library.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ability::RoleBinding;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    #[serde(default)]
    pub characters: Vec<ComposedCharacter>,
}

/// The game a composition plugin's manifest names.
pub const COMPOSITION_GAME: &str = "composition";

/// An output of another plugin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    pub plugin: String,
    pub output: String,
}

/// A playable character made from another plugin's character, with abilities learned from any
/// plugin and its movement tuned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedCharacter {
    /// The output id the character gets in this plugin.
    pub id: String,
    /// A model or sprite output with a character.
    pub base: OutputRef,
    #[serde(default)]
    pub abilities: Vec<AbilityUse>,
    #[serde(default)]
    pub tuning: Tuning,
    /// The bone held things are drawn at, for 3D characters whose plugin names none.
    #[serde(default)]
    pub hand: Option<String>,
}

/// An ability a composed character learns, and what plays for each of its roles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityUse {
    pub ability: OutputRef,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleBinding>,
    /// Roles without a binding play the ability's own animations (for characters drawn like the
    /// ability's source); otherwise they play the idle pose.
    #[serde(default)]
    pub use_defaults: bool,
}

/// Changes to the base character's movement, as factors.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tuning {
    /// Walking and running speeds.
    #[serde(default = "one")]
    pub speed: f32,
    /// Jump heights (3D characters).
    #[serde(default = "one")]
    pub jump: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            speed: 1.0,
            jump: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ron() {
        let composition: Composition = ron::from_str(
            r#"(characters: [(
                id: "runner_with_sword",
                base: (plugin: "game-b-runner", output: "runner"),
                abilities: [(
                    ability: (plugin: "game-a-hero", output: "sword"),
                    roles: {"swing": Animation("punch"), "charge": Default},
                )],
                tuning: (speed: 1.2),
                hand: Some("right_hand"),
            )])"#,
        )
        .unwrap();
        let character = &composition.characters[0];
        assert_eq!(character.tuning.jump, 1.0);
        assert_eq!(
            character.abilities[0].roles["swing"],
            RoleBinding::Animation("punch".into())
        );
        assert!(!character.abilities[0].use_defaults);
    }
}

//! Behaviors that make scene nodes do things when played: targets that can be hit and destroyed,
//! pickups, triggers, checkpoints, solid obstacles, hazards and doors. A behavior is generic (it
//! knows nothing of any game); what a node looks like is whatever plugin output it shows.
//!
//! Behaviors are assets too: a node can name one ("builtin/spikes", a plugin's "rupee") so
//! every node of that kind changes with it.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    recipe::Output,
};

/// What a node does when played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Behavior {
    /// Can be hit (by melee hits, projectiles and bursts) and destroyed; blocks movement if `solid`.
    Target {
        health: f32,
        radius: f32,
        solid: bool,
        /// What it leaves behind when destroyed, handed straight to the player.
        drop: Option<(String, u32)>,
    },
    /// Walking into it gives the player an item (if the inventory has room).
    Pickup {
        item: String,
        count: u32,
        radius: f32,
    },
    /// Does something when the player walks into it.
    Trigger {
        radius: f32,
        effect: Effect,
        /// Only the first time; otherwise every time the player enters.
        once: bool,
    },
    /// The player comes back here after falling out of the world or going down.
    Checkpoint { radius: f32 },
    /// Blocks movement.
    Solid { radius: f32 },
    /// Hurts the player for as long as it stands in it (as often as its hurt time allows).
    Hazard { radius: f32, damage: f32 },
    /// Takes the player to the node called `to`, the screen fading out and in over `fade`
    /// seconds.
    Door { radius: f32, to: String, fade: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Shows a message.
    Message(String),
    /// Moves the player to the node with this name.
    Teleport(String),
    /// Gives items.
    Give(String, u32),
    /// Puts the player back at the start.
    Restart,
    /// Takes health from the player (not again until its hurt time is over).
    Hurt(f32),
    /// Gives health back.
    Heal(f32),
    /// Shows a dialog, written in place or a dialog asset's name.
    Say(Slot<crate::ui::Dialog>),
    /// Plays a sound asset, `"plugin/output"`.
    Sound(String),
    /// Raises a flag (see [`Condition::Flag`]); it stays up until play stops or it's cleared.
    Set(String),
    /// Lowers a flag.
    Clear(String),
    /// Does each of these, in order.
    All(Vec<Effect>),
}

/// Something true or not while playing: what the player carries, flags effects raise, nodes
/// destroyed or picked up, and the current act. Behaviors work only while theirs holds;
/// nodes appear once theirs does; a game can be won by one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// The player carries at least `count` of `item`.
    Has {
        item: String,
        count: u32,
    },
    /// A flag is up (see [`Effect::Set`]).
    Flag(String),
    /// The node of this name was destroyed or picked up.
    Gone(String),
    /// This act (numbered from 1) is being played; false when no act is chosen.
    Act(u8),
    Not(Box<Condition>),
    /// Every one of these holds (true when there are none).
    All(Vec<Condition>),
    /// At least one of these holds.
    Any(Vec<Condition>),
}

/// What conditions are judged by while playing.
pub trait Facts {
    /// How many of an item the player carries.
    fn count(&self, item: &str) -> u32;
    fn flag(&self, name: &str) -> bool;
    /// Whether the node of this name is gone.
    fn gone(&self, node: &str) -> bool;
    /// The current act, or no act chosen.
    fn act(&self) -> Option<u8>;
}

impl Condition {
    pub fn holds(&self, facts: &dyn Facts) -> bool {
        match self {
            Condition::Has { item, count } => facts.count(item) >= *count,
            Condition::Flag(name) => facts.flag(name),
            Condition::Gone(node) => facts.gone(node),
            Condition::Act(act) => facts.act() == Some(*act),
            Condition::Not(inner) => !inner.holds(facts),
            Condition::All(all) => all.iter().all(|c| c.holds(facts)),
            Condition::Any(any) => any.iter().any(|c| c.holds(facts)),
        }
    }
}

/// One of a scene node's behaviors: written in place or a copy of the behavior asset `from`
/// (kept up to date with it), working only while `when` holds. It reaches `height` meters above
/// and below the node (its radius, at least 2 m, without one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeBehavior {
    pub behavior: Behavior,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<OutputRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
}

impl NodeBehavior {
    pub fn new(behavior: Behavior) -> Self {
        Self {
            behavior,
            from: None,
            when: None,
            height: None,
        }
    }

    /// How far above and below the node it reaches.
    pub fn reach(&self) -> f32 {
        self.height.unwrap_or(self.behavior.radius().max(2.0))
    }
}

impl Behavior {
    pub const KINDS: [&'static str; 7] = [
        "Target",
        "Pickup",
        "Trigger",
        "Checkpoint",
        "Solid",
        "Hazard",
        "Door",
    ];

    /// A new behavior of a kind from [`Behavior::KINDS`], with sensible defaults.
    pub fn new(kind: &str) -> Option<Self> {
        Some(match kind {
            "Target" => Behavior::Target {
                health: 3.0,
                radius: 0.6,
                solid: true,
                drop: None,
            },
            "Pickup" => Behavior::Pickup {
                item: String::new(),
                count: 1,
                radius: 0.6,
            },
            "Trigger" => Behavior::Trigger {
                radius: 1.5,
                effect: Effect::Message("Hello!".into()),
                once: false,
            },
            "Checkpoint" => Behavior::Checkpoint { radius: 1.2 },
            "Solid" => Behavior::Solid { radius: 0.6 },
            "Hazard" => Behavior::Hazard {
                radius: 0.8,
                damage: 1.0,
            },
            "Door" => Behavior::Door {
                radius: 0.8,
                to: String::new(),
                fade: 0.6,
            },
            _ => return None,
        })
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Behavior::Target { .. } => "Target",
            Behavior::Pickup { .. } => "Pickup",
            Behavior::Trigger { .. } => "Trigger",
            Behavior::Checkpoint { .. } => "Checkpoint",
            Behavior::Solid { .. } => "Solid",
            Behavior::Hazard { .. } => "Hazard",
            Behavior::Door { .. } => "Door",
        }
    }

    pub fn radius(&self) -> f32 {
        match self {
            Behavior::Target { radius, .. }
            | Behavior::Pickup { radius, .. }
            | Behavior::Trigger { radius, .. }
            | Behavior::Checkpoint { radius }
            | Behavior::Solid { radius }
            | Behavior::Hazard { radius, .. }
            | Behavior::Door { radius, .. } => *radius,
        }
    }

    /// RGB for drawing it in the viewport.
    pub fn color(&self) -> [f32; 3] {
        match self {
            Behavior::Target { .. } => [0.95, 0.4, 0.35],
            Behavior::Pickup { .. } => [0.45, 0.85, 0.4],
            Behavior::Trigger { .. } => [0.4, 0.6, 0.95],
            Behavior::Checkpoint { .. } => [0.95, 0.8, 0.35],
            Behavior::Solid { .. } => [0.65, 0.65, 0.65],
            Behavior::Hazard { .. } => [0.9, 0.3, 0.75],
            Behavior::Door { .. } => [0.35, 0.85, 0.85],
        }
    }
}

pub fn builtin(id: &str) -> OutputRef {
    OutputRef::new(BUILTIN, id)
}

/// Generic behaviors to place as they are: a target, a checkpoint, a wall post, spikes and a
/// heart that heals.
pub(crate) fn builtins() -> Vec<Output> {
    let behavior = |id: &str, behavior: Behavior| Output::Behavior {
        id: id.into(),
        behavior: Slot::Inline(behavior),
    };
    vec![
        behavior("target", Behavior::new("Target").expect("a kind")),
        behavior("checkpoint", Behavior::new("Checkpoint").expect("a kind")),
        behavior("post", Behavior::new("Solid").expect("a kind")),
        behavior("spikes", Behavior::new("Hazard").expect("a kind")),
        behavior(
            "heart",
            Behavior::Trigger {
                radius: 0.6,
                effect: Effect::Heal(2.0),
                once: true,
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Playing(Option<u8>);

    impl Facts for Playing {
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
            self.0
        }
    }

    #[test]
    fn act_conditions_match_only_the_chosen_act() {
        let condition: Condition = ron::from_str("Act(3)").unwrap();
        assert_eq!(condition, Condition::Act(3));
        assert_eq!(ron::to_string(&condition).unwrap(), "Act(3)");
        assert!(!condition.holds(&Playing(None)));
        assert!(!condition.holds(&Playing(Some(1))));
        assert!(condition.holds(&Playing(Some(3))));
        assert!(
            Condition::All(vec![condition, Condition::Not(Box::new(Condition::Act(1))),])
                .holds(&Playing(Some(3)))
        );
    }
}

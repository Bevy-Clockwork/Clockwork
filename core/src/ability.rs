//! Abilities: moves packaged so that any character can learn them, whatever game it comes from.
//!
//! A plugin makes an ability out of some of its character's actions (with the projectiles they
//! throw, the items that start them and the sprites they show). The poses those actions play are
//! **roles**: named slots that the learning character fills with animations of its own (a
//! punch for another character's sword swing). The source character's animations stay as the roles' defaults,
//! so characters drawn the same way need no binding at all. A role nothing fills plays the
//! character's idle pose: the move still works (it hurts, throws, guards), it just doesn't show.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::action::Moves;
use crate::character2d::{AnimationRef, Directional};
use crate::item::ItemDef;

/// Moves any character can learn.
#[derive(Debug, Clone, PartialEq)]
pub struct Ability {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The actions, with every animation replaced by a role: all four facings name the role.
    pub moves: Moves,
    pub items: Vec<ItemDef>,
    pub roles: Vec<Role>,
}

/// A pose an ability's actions need.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub name: String,
    /// What it is, for whoever binds it ("swing the weapon", "hold it up").
    #[serde(default)]
    pub description: String,
    /// The source character's animations for it.
    pub default: Directional,
}

/// What a character plays for a role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoleBinding {
    /// One animation for every facing (3D characters, or 2D ones without facings).
    Animation(String),
    /// An animation per facing (2D characters).
    Directional(Directional),
    /// The ability's own animations: for characters drawn like the ability's source.
    Default,
}

impl Directional {
    /// The same animation for every facing.
    pub fn uniform(name: &str) -> Directional {
        let one = AnimationRef {
            name: name.to_string(),
            flip_x: false,
            flip_y: false,
        };
        Directional {
            north: one.clone(),
            east: one.clone(),
            south: one.clone(),
            west: one,
        }
    }

    /// The role this stands for in an ability's actions: the name all four facings share.
    pub fn role(&self) -> Option<&str> {
        let name = self.north.name.as_str();
        [&self.east, &self.south, &self.west]
            .iter()
            .all(|r| r.name == name)
            .then_some(name)
    }
}

impl Ability {
    pub fn role(&self, name: &str) -> Option<&Role> {
        self.roles.iter().find(|r| r.name == name)
    }

    /// The ability's moves for a character: every role becomes the bound animation, the role's
    /// default for `Default` (or when `use_defaults` and nothing is bound), or `idle`.
    pub fn moves_for(
        &self,
        bindings: &BTreeMap<String, RoleBinding>,
        use_defaults: bool,
        idle: &Directional,
    ) -> Moves {
        let resolve = |directional: &Directional| -> Directional {
            let Some(role) = directional.role().and_then(|name| self.role(name)) else {
                return directional.clone();
            };
            match bindings.get(&role.name) {
                Some(RoleBinding::Animation(name)) => Directional::uniform(name),
                Some(RoleBinding::Directional(d)) => d.clone(),
                Some(RoleBinding::Default) => role.default.clone(),
                None if use_defaults => role.default.clone(),
                None => idle.clone(),
            }
        };
        let mut moves = self.moves.clone();
        for action in &mut moves.actions {
            action.animation = resolve(&action.animation);
            if let Some(hold) = &mut action.hold {
                hold.animation = resolve(&hold.animation);
                if let Some(walk) = &mut hold.walk_animation {
                    *walk = resolve(walk);
                }
            }
        }
        moves
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Motion};

    fn ability() -> Ability {
        Ability {
            id: "sword".into(),
            name: "Sword".into(),
            description: String::new(),
            moves: Moves {
                actions: vec![Action {
                    id: "slash".into(),
                    animation: Directional::uniform("swing"),
                    duration: 0.25,
                    motion: Motion::Rooted,
                    hits: vec![],
                    invulnerable: None,
                    spawns: vec![],
                    hold: None,
                    cancel_from: None,
                    attachments: vec![],
                }],
                ..Default::default()
            },
            items: vec![],
            roles: vec![Role {
                name: "swing".into(),
                description: String::new(),
                default: Directional::uniform("sword_south"),
            }],
        }
    }

    #[test]
    fn roles_become_the_learners_animations() {
        let idle = Directional::uniform("idle");
        let animation = |bindings: BTreeMap<String, RoleBinding>, defaults| {
            ability().moves_for(&bindings, defaults, &idle).actions[0]
                .animation
                .north
                .name
                .clone()
        };
        let punch = BTreeMap::from([("swing".to_string(), RoleBinding::Animation("punch".into()))]);
        assert_eq!(animation(punch, false), "punch");
        assert_eq!(animation(BTreeMap::new(), true), "sword_south");
        assert_eq!(animation(BTreeMap::new(), false), "idle");
        let default = BTreeMap::from([("swing".to_string(), RoleBinding::Default)]);
        assert_eq!(animation(default, false), "sword_south");
    }

    #[test]
    fn mixed_facings_are_not_roles() {
        let mut d = Directional::uniform("a");
        assert_eq!(d.role(), Some("a"));
        d.west.name = "b".into();
        assert_eq!(d.role(), None);
    }
}

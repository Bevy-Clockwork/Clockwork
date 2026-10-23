//! Items: what a character can own, carry and use. A plugin lists a game's items (name, what they
//! do, how many fit); the runtime keeps an inventory and runs the item's action when it is used.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
    /// The action (of the playing character) performed when the item is used from a button.
    #[serde(default)]
    pub action: Option<String>,
    /// Ammunition spent each time the action starts.
    #[serde(default)]
    pub ammo: Option<Ammo>,
    /// The most that can be carried: 1 for things you have or don't, more for bombs and arrows.
    #[serde(default = "default_max")]
    pub max: u32,
    /// How many the character starts with.
    #[serde(default)]
    pub start: u32,
    /// The button the item is put on at the start, if any.
    #[serde(default)]
    pub slot: Option<Slot>,
    /// The picture shown for it in the inventory and on its button.
    #[serde(default)]
    pub icon: Option<Icon>,
}

/// An item's picture: a frame of a sprite output, the first picture of one of its animations, or
/// an image output, all of the same plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Icon {
    Frame {
        sprite: String,
        frame: usize,
        #[serde(default)]
        flip_x: bool,
    },
    Animation {
        sprite: String,
        animation: String,
    },
    Image(String),
}

fn default_max() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    /// Swords, bows: things to fight with.
    Weapon,
    /// Things for getting around and solving problems.
    Tool,
    /// Worn or held, such as shields.
    Equipment,
    /// Ammunition and things that get used up.
    Consumable,
    /// Story and collection items.
    Quest,
}

/// An item that is spent when another is used: arrows for the bow, bombs for themselves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ammo {
    /// The item that is spent (usually the item itself, for bombs).
    pub item: String,
    pub cost: u32,
}

/// The buttons items go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    pub const ALL: [Slot; 2] = [Slot::A, Slot::B];

    pub fn index(self) -> usize {
        match self {
            Slot::A => 0,
            Slot::B => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_defaults() {
        let item: ItemDef = ron::from_str(
            r#"(id: "bow", name: "Bow", description: "Shoots arrows.", category: Weapon,
                action: Some("draw_bow"), ammo: Some((item: "arrows", cost: 1)), start: 1,
                slot: Some(A))"#,
        )
        .unwrap();
        assert_eq!(item.max, 1);
        assert_eq!(item.ammo.unwrap().item, "arrows");
        assert_eq!(item.slot, Some(Slot::A));
        assert_eq!(Slot::B.index(), 1);
    }
}

//! A character's inventory: what it owns and how many, and which items are on the A and B
//! buttons. Pure data and rules, for any game's items ([`clockwork_core::item`]).

use std::collections::HashMap;

use clockwork_core::item::{ItemDef, Slot};

#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    items: Vec<ItemDef>,
    counts: HashMap<String, u32>,
    slots: [Option<String>; 2],
}

impl Inventory {
    /// The starting inventory of a game's items: each at its start count, and on the button
    /// it names (the first item to claim a button gets it).
    pub fn new(items: Vec<ItemDef>) -> Self {
        let mut inventory = Self {
            counts: items
                .iter()
                .map(|i| (i.id.clone(), i.start.min(i.max)))
                .collect(),
            slots: [None, None],
            items,
        };
        for index in 0..inventory.items.len() {
            let item = inventory.items[index].clone();
            if let Some(slot) = item.slot
                && inventory.slots[slot.index()].is_none()
                && inventory.owns(&item.id)
            {
                inventory.slots[slot.index()] = Some(item.id);
            }
        }
        inventory
    }

    /// Every item the game defines, in order.
    pub fn items(&self) -> &[ItemDef] {
        &self.items
    }

    pub fn item(&self, id: &str) -> Option<&ItemDef> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn count(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    pub fn owns(&self, id: &str) -> bool {
        self.count(id) > 0
    }

    /// Adds `n` of an item, up to what fits. Returns how many were added.
    pub fn add(&mut self, id: &str, n: u32) -> u32 {
        let Some(max) = self.item(id).map(|i| i.max) else {
            return 0;
        };
        let count = self.counts.entry(id.to_string()).or_default();
        let added = n.min(max.saturating_sub(*count));
        *count += added;
        added
    }

    /// The item on a button, if it is still owned.
    pub fn equipped(&self, slot: Slot) -> Option<&ItemDef> {
        self.slots[slot.index()]
            .as_deref()
            .filter(|id| self.owns(id))
            .and_then(|id| self.item(id))
    }

    /// Puts an owned item with an action on a button. An item already on the other button swaps
    /// places with this button's. False if the item isn't owned or can't be used.
    pub fn equip(&mut self, slot: Slot, id: &str) -> bool {
        if !self.owns(id) || self.item(id).is_none_or(|i| i.action.is_none()) {
            return false;
        }
        let other = Slot::ALL[1 - slot.index()];
        if self.slots[other.index()].as_deref() == Some(id) {
            self.slots.swap(0, 1);
        } else {
            self.slots[slot.index()] = Some(id.to_string());
        }
        true
    }

    pub fn unequip(&mut self, slot: Slot) {
        self.slots[slot.index()] = None;
    }

    /// Whether the item can be used now: owned, with an action, and enough ammunition.
    pub fn can_use(&self, id: &str) -> bool {
        let Some(item) = self.item(id) else {
            return false;
        };
        self.owns(id)
            && item.action.is_some()
            && item
                .ammo
                .as_ref()
                .is_none_or(|ammo| self.count(&ammo.item) >= ammo.cost)
    }

    /// Uses up the item's ammunition for one use. False (and nothing spent) if it can't be used.
    pub fn spend(&mut self, id: &str) -> bool {
        if !self.can_use(id) {
            return false;
        }
        if let Some(ammo) = self.item(id).and_then(|i| i.ammo.clone()) {
            let count = self.counts.entry(ammo.item).or_default();
            *count -= ammo.cost;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use clockwork_core::item::{Ammo, Category};

    use super::*;

    fn item(id: &str, action: Option<&str>, max: u32, start: u32, slot: Option<Slot>) -> ItemDef {
        ItemDef {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            category: Category::Weapon,
            action: action.map(Into::into),
            ammo: None,
            max,
            start,
            slot,
            icon: None,
        }
    }

    fn inventory() -> Inventory {
        let mut bow = item("bow", Some("draw"), 1, 1, Some(Slot::A));
        bow.ammo = Some(Ammo {
            item: "arrows".into(),
            cost: 1,
        });
        let mut bombs = item("bombs", Some("place"), 10, 3, None);
        bombs.ammo = Some(Ammo {
            item: "bombs".into(),
            cost: 1,
        });
        Inventory::new(vec![
            item("sword", Some("slash"), 1, 1, Some(Slot::B)),
            bow,
            bombs,
            item("arrows", None, 30, 2, None),
            item("lantern", Some("light"), 1, 0, None),
        ])
    }

    #[test]
    fn starts_with_the_games_defaults() {
        let i = inventory();
        assert_eq!(i.equipped(Slot::B).unwrap().id, "sword");
        assert_eq!(i.equipped(Slot::A).unwrap().id, "bow");
        assert!(i.owns("bombs") && !i.owns("lantern"));
        assert_eq!(i.count("arrows"), 2);
    }

    #[test]
    fn equipping_swaps_and_refuses_what_cant_be_used() {
        let mut i = inventory();
        assert!(i.equip(Slot::A, "bombs"));
        assert_eq!(i.equipped(Slot::A).unwrap().id, "bombs");
        // The sword is on B: putting it on A swaps the two.
        assert!(i.equip(Slot::A, "sword"));
        assert_eq!(i.equipped(Slot::A).unwrap().id, "sword");
        assert_eq!(i.equipped(Slot::B).unwrap().id, "bombs");
        assert!(!i.equip(Slot::B, "lantern"), "not owned");
        assert!(!i.equip(Slot::B, "arrows"), "no action");
        i.unequip(Slot::B);
        assert!(i.equipped(Slot::B).is_none());
    }

    #[test]
    fn using_spends_ammunition_and_capacity_limits_pickups() {
        let mut i = inventory();
        assert!(i.spend("bow") && i.spend("bow"));
        assert!(!i.can_use("bow"), "out of arrows");
        assert!(!i.spend("bow"));
        assert_eq!(i.count("arrows"), 0);
        assert!(i.spend("bombs"));
        assert_eq!(i.count("bombs"), 2);
        assert_eq!(i.add("bombs", 50), 8);
        assert_eq!(i.count("bombs"), 10);
        assert_eq!(i.add("lantern", 1), 1);
        assert!(i.can_use("lantern"));
        assert_eq!(i.add("unknown", 1), 0);
    }
}

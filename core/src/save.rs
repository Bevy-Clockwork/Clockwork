//! A save file: what a player has done in a game, kept between sessions. Play takes it from
//! and gives it to the running world: the items carried, the flags raised, the nodes gone
//! (picked up, destroyed), where to start again, and each course's acts done.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveData {
    /// Items carried, by id, with how many.
    pub items: BTreeMap<String, u32>,
    /// Flags up (see `behavior::Effect::Set`).
    pub flags: BTreeSet<String>,
    /// Names of nodes gone for good. Picked-up coins come back; what a game keeps gone is up
    /// to Play (stars, keys, beaten bosses).
    pub gone: BTreeSet<String>,
    /// The node to start from when the file is played again.
    pub start: Option<String>,
    /// Acts done, by course entry node: act numbers from 1.
    pub acts_done: BTreeMap<String, BTreeSet<u8>>,
    /// Seconds played.
    pub seconds: f64,
}

impl SaveData {
    /// Whether nothing has been done yet: a new file.
    pub fn is_new(&self) -> bool {
        *self == SaveData::default()
    }
}

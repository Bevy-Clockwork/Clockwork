//! Generic Bevy implementations of the core vocabulary, configured by plugin data (DESIGN.md §4.1).
//!
//! Includes a pure character controller; tileset and sprite rendering are planned for M1/M2.

pub mod actions;
pub mod billboard;
pub mod controller;
pub mod hud;
pub mod input;
pub mod inventory;
pub mod play;
pub mod projectiles;
pub mod retro_material;
pub mod terrain;
pub mod topdown;

use bevy::prelude::*;

pub struct ClockworkRuntimePlugin;

impl Plugin for ClockworkRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            input::InputPlugin,
            hud::HudPlugin,
            retro_material::RetroMaterialPlugin,
        ));
    }
}

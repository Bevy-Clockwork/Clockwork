//! The in-game HUD and menus, drawn in the game's own view with Bevy UI at a whole-number pixel
//! scale. What they show and how they look are assets (`clockwork_core::ui`): the HUD's widgets
//! and where they sit, the UI theme, and the menus and the buttons that open them.
//!
//! The game fills [`ShownHud`] every frame and answers the [`HudRequests`] the HUD makes. While a
//! menu is open ([`MenuState::open`]) the game stands still. The HUD is drawn on the camera
//! marked [`HudTarget`] and fits that camera's viewport.
//! [`ShownScreen`] replaces the HUD with a flow screen; its [`ScreenRequest`] messages go
//! back to Play, which owns focus and decides when to enter or leave the world.

mod menu;
mod screens;
mod view;

use std::sync::Arc;

use bevy::prelude::*;
use clockwork_core::{
    image::RgbaImage,
    item::{Category, Slot},
    ui::{HudLayout, Menu, Theme},
};

pub use menu::{MenuMove, MenuState};
pub use screens::{ScreenMove, ScreenRequest, ShownScreen};

/// What the HUD shows of a button's item.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotView {
    pub id: String,
    pub name: String,
    /// "x 5" for things with a count.
    pub count: Option<String>,
    pub usable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
    pub owned: bool,
    pub count: u32,
    pub max: u32,
    pub on: Option<Slot>,
    pub can_equip: bool,
}

/// Everything the HUD draws of the played character's fighting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub slots: [Option<SlotView>; 2],
    pub action: Option<String>,
    pub items: Vec<ItemView>,
    pub hitboxes: bool,
    pub has_dodge: bool,
    /// Item pictures by item id (shared, not copied every frame).
    pub icons: Arc<Vec<(String, RgbaImage)>>,
}

impl Hud {
    pub fn icon(&self, item: &str) -> Option<&RgbaImage> {
        self.icons.iter().find(|(id, _)| id == item).map(|(_, i)| i)
    }

    /// The inventory's items in the order the menu shows them: by category, then as listed.
    pub fn menu_items(&self) -> Vec<&ItemView> {
        let mut items: Vec<&ItemView> = self.items.iter().collect();
        items.sort_by_key(|i| menu::category_order(i.category));
        items
    }
}

/// The HUD's requests, answered by the game.
#[derive(Clone, Debug, PartialEq)]
pub enum HudRequest {
    Equip(Slot, String),
    Unequip(Slot),
    ToggleHitboxes,
    /// From the pause menu: back to the start point.
    Restart,
    /// From the pause menu: stop playing.
    Stop,
    /// The open dialog's next page (or closing it after the last).
    NextPage,
}

/// The look of the played character's screen: its HUD, theme and menus.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub layout: HudLayout,
    pub theme: Theme,
    pub menus: Vec<Menu>,
    /// Pictures the theme or title names (frame and image logo), by `plugin/output`.
    pub images: Arc<Vec<(String, RgbaImage)>>,
    /// The theme's font, if it names one.
    pub font: Option<Arc<clockwork_core::font::BitmapFont>>,
    /// The dialog's font, if the theme names one.
    pub dialog_font: Option<Arc<clockwork_core::font::BitmapFont>>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            layout: clockwork_core::ui::default_hud(),
            theme: clockwork_core::ui::default_theme(),
            menus: Vec::new(),
            images: Default::default(),
            font: None,
            dialog_font: None,
        }
    }
}

/// What the HUD shows now.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ShownHud {
    /// The played character's fighting, if it has any.
    pub hud: Option<Hud>,
    /// Health and most health, if the character has them.
    pub health: Option<(f32, f32)>,
    pub toast: Option<String>,
    /// How the HUD and flow screens look. Without this, flow screens use the default theme;
    /// the in-game HUD shows nothing but toasts.
    pub screen: Option<Screen>,
    /// Lives left and whole seconds left, when the rules count them.
    pub lives: Option<u32>,
    pub seconds: Option<u32>,
    /// How dark the screen is, 0 to 16 (a door's fade).
    pub fade: u8,
    /// Big text over the game: "You win!".
    pub banner: Option<String>,
    /// A dialog's page shown, while one is open.
    pub dialog: Option<DialogView>,
}

/// A dialog's page as the HUD shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogView {
    pub speaker: Option<String>,
    pub text: String,
    /// Whether more pages follow.
    pub more: bool,
}

/// What the HUD asked for since the game last took them.
#[derive(Resource, Debug, Default)]
pub struct HudRequests(pub Vec<HudRequest>);

/// The camera the HUD is drawn on.
#[derive(Component, Debug, Default)]
pub struct HudTarget;

/// The game height the HUD is laid out for, in game pixels: the viewport shows it at the
/// biggest whole-number scale that fits.
pub const BASE_HEIGHT: f32 = 240.0;

/// The whole-number scale for a viewport `height` logical pixels high.
pub fn pixel_scale(height: f32) -> f32 {
    (height / BASE_HEIGHT).floor().max(1.0)
}

/// Draws the HUD and runs its menus.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownHud>()
            .init_resource::<ShownScreen>()
            .add_message::<ScreenRequest>()
            .init_resource::<HudRequests>()
            .init_resource::<MenuState>()
            .add_systems(
                Update,
                (
                    screens::input_system,
                    menu::menu_system,
                    view::build_system,
                    view::screens::blink_system,
                )
                    .chain()
                    .in_set(HudSystems),
            );
    }
}

/// The game fills [`ShownHud`] and [`ShownScreen`] before these systems, then takes
/// [`HudRequests`] and reads [`ScreenRequest`] messages after them.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HudSystems;

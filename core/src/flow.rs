//! A game's flow outside its world: the title screen, the save files to choose from, the act
//! to play when entering a course, and what shows when the game is over. A `Flow` asset holds
//! it (see `Output::Flow`); a game names one in its settings.
//!
//! [`ScreenView`] is what the screens show at a moment, worked out by Play from the flow, the
//! save files and the player's choices, and drawn by the runtime's HUD.

use serde::{Deserialize, Serialize};

use crate::{asset::Slot, ui::Dialog};

/// Words shown on a screen: written in place, or a dialog asset whose first page they are
/// (`"plugin/output"`), so a game's own text can come from its ROM.
pub type Label = Slot<Dialog>;

/// The screens around a game's world, each optional: without a title screen Play starts in
/// the world, without save files nothing is kept, without act select courses have one act.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    #[serde(default)]
    pub title: Option<TitleScreen>,
    #[serde(default)]
    pub files: Option<FileSelect>,
    #[serde(default)]
    pub acts: Option<ActSelect>,
    #[serde(default)]
    pub game_over: Option<GameOver>,
    /// The node a game starts at once its file is chosen (or past the title without files),
    /// wherever the played character stands in the scene; where it starts over from too.
    #[serde(default)]
    pub start: Option<String>,
}

/// The first screen: a logo over a background until the player presses a button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleScreen {
    /// An image or model asset, `"plugin/output"`, shown in the middle.
    #[serde(default)]
    pub logo: Option<String>,
    /// The words asking for a button, e.g. "PRESS START".
    pub prompt: Label,
    /// The background colour.
    #[serde(default = "black")]
    pub background: crate::ui::Rgba,
    /// How the game itself frames a model logo; the logo then fills the screen as it drew it.
    /// Without one the logo is framed to fit above the prompt.
    #[serde(default)]
    pub logo_camera: Option<LogoCamera>,
}

/// A camera looking at a model logo's origin over the whole (4:3) screen, as a game's own
/// screen camera does (Super Mario 64: from (0, 0, 3200), 45°).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoCamera {
    /// Where it stands, in the model's units.
    pub from: [f32; 3],
    /// Its vertical field of view in degrees.
    pub fov: f32,
}

/// Choosing which save file to play: `slots` files, each new or carrying on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSelect {
    pub title: Label,
    #[serde(default = "four")]
    pub slots: u8,
    /// The item counted on each file's button (stars, hearts), if any.
    #[serde(default)]
    pub counts: Option<String>,
}

/// Choosing an act when the player arrives in a course: the course's acts, each one setting
/// which of the course's objects are there (see a node's `acts`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActSelect {
    pub courses: Vec<Course>,
}

/// A course: entered by arriving at node `entry` (where its painting or door leads).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Course {
    pub name: Label,
    pub entry: String,
    /// The acts' names, act 1 first.
    pub acts: Vec<Label>,
}

/// What shows when the player is out of lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameOver {
    pub text: Label,
}

fn black() -> crate::ui::Rgba {
    crate::ui::Rgba(0, 0, 0, 255)
}

fn four() -> u8 {
    4
}

/// What the screens show now; `None` in Play while the world is on screen.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenView {
    Title {
        logo: Option<String>,
        /// The logo fills the screen, drawn by this camera.
        logo_camera: Option<LogoCamera>,
        prompt: String,
        background: crate::ui::Rgba,
    },
    Files {
        title: String,
        slots: Vec<FileView>,
        /// The highlighted slot, from 0.
        focus: usize,
    },
    Acts {
        course: String,
        acts: Vec<ActView>,
        /// The highlighted act, from 0.
        focus: usize,
    },
    GameOver {
        text: String,
    },
}

/// One save file's button.
#[derive(Debug, Clone, PartialEq)]
pub struct FileView {
    /// "Mario A": the slot's name.
    pub name: String,
    /// Empty: a new game starts there.
    pub empty: bool,
    /// How many of the flow's counted item it holds.
    pub count: Option<u32>,
}

/// One act's line.
#[derive(Debug, Clone, PartialEq)]
pub struct ActView {
    pub name: String,
    /// Its goal was reached on this file (e.g. its star collected).
    pub done: bool,
}

/// What the player did on a screen, for Play to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAction {
    /// The title screen's button.
    Start,
    /// A save file chosen (from 0).
    File(usize),
    /// An act chosen (from 0).
    Act(usize),
    /// Back out of the screen (file select to title, act select to the castle).
    Back,
    /// Past the game over screen.
    Continue,
}

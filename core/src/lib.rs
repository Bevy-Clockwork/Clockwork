//! Clockwork's shared vocabulary.
//!
//! Every plugin is written against the types in this crate, so it must stay small and stable.
//! It deliberately has no Bevy dependency: Bevy integration lives in `clockwork_runtime`.
//! See `DESIGN.md` §4.2.

pub mod ability;
pub mod action;
pub mod asset;
pub mod behavior;
pub mod camera;
pub mod character;
pub mod character2d;
pub mod compose;
pub mod flow;
pub mod font;
pub mod game;
pub mod image;
pub mod input;
pub mod item;
pub mod manifest;
pub mod model;
pub mod parts;
pub mod platform;
pub mod recipe;
pub mod recipe_schema;
pub mod save;
pub mod sound;
pub mod sprite;
pub mod surface;
pub mod terrain;
pub mod ui;
pub mod units;

pub use manifest::PluginManifest;
pub use platform::{Platform, RomInfo};
pub use surface::{SurfaceExtension, SurfaceProperty};
pub use units::NativeScale;

/// The version of the shared vocabulary that plugins declare in `clockwork.toml` as `core`.
pub const CORE_VERSION: &str = "0.1";

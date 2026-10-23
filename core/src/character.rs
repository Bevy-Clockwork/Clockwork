//! Characters: how a model moves and which animation plays when. Plugins give the numbers and
//! choices ("feels like" a game's character); the runtime's generic controller uses them.
//!
//! All values are in meters, seconds and degrees. Games that count per frame convert with their
//! frame rate (a per-frame speed × fps, a per-frame change of speed × fps²).

use serde::{Deserialize, Serialize};

use crate::{
    action::{Action, Moves, Projectile},
    asset::{AssetSource, Layer, OutputRef, ResolveError, Slot},
    parts::Stats,
};

/// A 3D character as a plugin writes it: its movement and animations may name assets.
pub type CharacterSpec = CharacterOf<Slot<Movement>, Slot<StateAnimations>>;

/// A 3D character ready to play: its movement and animations in place.
pub type Character = CharacterOf<Movement, StateAnimations>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterOf<M, A> {
    pub movement: M,
    pub animations: A,
    /// Its health and how long it can't be hurt again after a hit; without, the built-in
    /// `builtin/stats`.
    #[serde(default)]
    pub stats: Option<Slot<Stats>>,
    /// The items it carries: an `Items` asset, `"plugin/output"`.
    #[serde(default, with = "crate::asset::as_path_opt")]
    pub inventory: Option<OutputRef>,
    /// Bending a bone into turns and with speed while moving on the ground.
    #[serde(default)]
    pub lean: Option<Lean>,
    /// How the camera follows the character in Play mode: written in place or a camera asset's
    /// name. Without one the camera stays where it is put.
    #[serde(default)]
    pub camera: Option<crate::asset::Slot<FollowCamera>>,
    /// Its HUD, UI, menus and input map; empty slots get the built-in ones.
    #[serde(default, skip_serializing_if = "crate::ui::Kit::is_empty")]
    pub kit: crate::ui::Kit,
    /// Things the character can do besides moving, as for 2D characters (see
    /// [`crate::action`]): an action's animation names the model animation to play (the same
    /// for every facing).
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub projectiles: Vec<Projectile>,
    #[serde(default, alias = "roll")]
    pub dodge: Option<String>,
    /// The bone things it holds are drawn at (a hand).
    #[serde(default)]
    pub hand: Option<String>,
}

impl<M: Clone, A: Clone> CharacterOf<M, A> {
    /// What the character can do besides moving.
    pub fn moves(&self) -> Moves {
        Moves {
            actions: self.actions.clone(),
            projectiles: self.projectiles.clone(),
            dodge: self.dodge.clone(),
        }
    }

    /// The character with other moves (its own and learned ones).
    pub fn with_moves(&self, moves: Moves) -> Self {
        Self {
            actions: moves.actions,
            projectiles: moves.projectiles,
            dodge: moves.dodge,
            ..self.clone()
        }
    }
}

impl CharacterSpec {
    /// The character with its movement and animations found where they are named.
    pub fn resolve(&self, source: &dyn AssetSource) -> Result<Character, ResolveError> {
        let movement = crate::asset::resolve(
            &[(Layer::Asset, Some(&self.movement))],
            source,
            "3D movement",
            crate::asset::movement_3d,
        )?
        .expect("the layer has a slot")
        .value;
        let animations = crate::asset::resolve(
            &[(Layer::Asset, Some(&self.animations))],
            source,
            "3D animations",
            crate::asset::animations_3d,
        )?
        .expect("the layer has a slot")
        .value;
        Ok(Character {
            movement,
            animations,
            stats: self.stats.clone(),
            inventory: self.inventory.clone(),
            lean: self.lean.clone(),
            camera: self.camera.clone(),
            kit: self.kit.clone(),
            actions: self.actions.clone(),
            projectiles: self.projectiles.clone(),
            dodge: self.dodge.clone(),
            hand: self.hand.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Movement {
    /// Top speed while walking (the walk button held).
    pub walk_speed: f32,
    /// Top speed otherwise.
    pub run_speed: f32,
    /// Speed gained per second from a standstill while there is input.
    pub acceleration: f32,
    /// Acceleration drops by speed × this, so speed approaches its top gradually (per second).
    #[serde(default)]
    pub acceleration_falloff: f32,
    /// Speed lost per second without input, or above the top speed.
    pub braking: f32,
    /// How fast the character turns towards the input on the ground, in degrees per second.
    pub turn_speed: f32,
    /// Skidding: turning around at speed, and stopping at speed.
    #[serde(default)]
    pub skid: Option<Skid>,
    /// Jumps in a chain: the first is the normal jump; jumping again soon after landing from
    /// jump `i` does jump `i + 1` if its conditions hold (double, triple jump).
    pub jumps: Vec<Jump>,
    /// How soon after landing a jump continues the chain.
    #[serde(default)]
    pub chain_window: f32,
    /// How long the landing pose holds before normal movement animations resume.
    #[serde(default)]
    pub landing_time: f32,
    /// Letting go of jump early makes a lower jump.
    #[serde(default)]
    pub jump_release: Option<JumpRelease>,
    /// Downward acceleration.
    pub gravity: f32,
    /// Fastest fall.
    pub max_fall_speed: f32,
    pub air: AirControl,
    /// Collision radius.
    #[serde(default = "default_radius")]
    pub radius: f32,
}

fn default_radius() -> f32 {
    0.3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skid {
    /// Skids only happen from at least this speed.
    pub min_speed: f32,
    /// Input pointing more than this far from the facing direction (degrees) means turn around.
    pub reverse_angle: f32,
    /// Speed lost per second while skidding.
    pub braking: f32,
    /// After a turnaround skid, the character faces the input and sets off at this speed.
    pub turn_kick_speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Jump {
    /// Upward speed at the start.
    pub speed: f32,
    /// Plus this share of the ground speed.
    #[serde(default)]
    pub speed_from_ground: f32,
    /// Ground speed is multiplied by this when the jump starts.
    #[serde(default = "one")]
    pub keep_ground_speed: f32,
    /// This jump in the chain needs at least this ground speed; otherwise the first jump is used.
    #[serde(default)]
    pub min_ground_speed: f32,
    /// Whether letting go of jump early cuts it short (see `Movement::jump_release`).
    #[serde(default = "yes")]
    pub controllable: bool,
    /// Animation while rising.
    pub animation: String,
    /// Animation while falling; the rising one if missing.
    #[serde(default)]
    pub fall_animation: Option<String>,
    /// Animation on landing; none means straight back to moving.
    #[serde(default)]
    pub land_animation: Option<String>,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JumpRelease {
    /// Applies while rising faster than this.
    pub above: f32,
    /// Upward speed is multiplied by this, `rate` times per second.
    pub factor: f32,
    pub rate: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirControl {
    /// Whether the character turns towards the input in the air (many games keep the facing).
    #[serde(default)]
    pub turn: bool,
    /// Forward speed lost per second in the air.
    pub drag: f32,
    /// Forward speed gained or lost per second with input along or against the facing.
    pub acceleration: f32,
    /// Sideways speed with full input across the facing (not kept between steps).
    pub strafe_speed: f32,
    /// Above this forward speed, the ground braking applies in the air too.
    pub max_speed: f32,
}

/// The model's animation for each movement state, by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateAnimations {
    /// Standing still. With several, they play one after another and repeat.
    pub idle: Vec<String>,
    pub walk: String,
    pub run: String,
    /// The run animation starts above this speed, and the walk animation returns below
    /// `walk_below` (a gap keeps them from flickering).
    pub run_above: f32,
    pub walk_below: f32,
    /// The ground speed at which walk and run play at their normal rate; they play faster or
    /// slower in proportion, so feet don't slide.
    pub stride_speed: f32,
    /// Falling without having jumped (walking off an edge).
    pub fall: String,
    /// Skidding into a turnaround, and the turn itself.
    #[serde(default)]
    pub skid_turn: Option<String>,
    #[serde(default)]
    pub turn: Option<String>,
    /// The turn animation shows the character facing back (it ends facing forward).
    #[serde(default)]
    pub turn_faces_back: bool,
    /// Skidding to a stop, and coming to rest after it.
    #[serde(default)]
    pub skid_stop: Option<String>,
    #[serde(default)]
    pub skid_stopped: Option<String>,
}

/// Bends a bone (relative to its parent) into turns and forward with speed, as many characters
/// do while running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lean {
    pub bone: String,
    /// Axes in the parent bone's space; the pitch is applied after the roll.
    pub roll_axis: [f32; 3],
    pub pitch_axis: [f32; 3],
    /// Roll in degrees per (turn rate in degrees per second × speed).
    pub roll_per_turn: f32,
    /// Pitch in degrees per unit of speed.
    pub pitch_per_speed: f32,
    /// Largest roll or pitch, in degrees.
    pub max: f32,
    /// How fast the lean follows its target, in degrees per second.
    pub rate: f32,
}

/// A camera behind the character that swings round as it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowCamera {
    pub distance: f32,
    /// Looking down at this angle, in degrees.
    pub pitch: f32,
    /// Looking at this point above the feet.
    pub height: f32,
    /// How fast the camera swings behind the character, per second at run speed.
    pub follow: f32,
}

//! Top-down 2D characters: how a sprite moves on the ground and which of its animations plays
//! for what it does. Plugins give the numbers and choices; the runtime's generic top-down
//! controller uses them (see [`crate::character`] for the 3D counterpart).
//!
//! In 3D worlds (areas with terrain) the same character plays as a 2.5D billboard: it stands
//! upright, always facing the camera, walks camera-relative on the terrain's floors and falls
//! off ledges; the facing animations follow the stick as seen on screen.
//!
//! All values are in meters, seconds and degrees. Games that count in pixels per frame convert
//! with the sprite's `pixels_per_meter` and their frame rate.

use serde::{Deserialize, Serialize};

use crate::{
    action::{Action, Moves, Projectile},
    asset::{AssetSource, Layer, OutputRef, ResolveError, Slot},
    parts::Stats,
};

/// A top-down character as a plugin writes it: its movement and animations may name assets.
pub type Character2dSpec = Character2dOf<Slot<Movement2d>, Slot<Animations2d>>;

/// A top-down character ready to play: its movement and animations in place.
pub type Character2d = Character2dOf<Movement2d, Animations2d>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character2dOf<M, A> {
    pub movement: M,
    pub animations: A,
    /// Its health and how long it can't be hurt again after a hit; without, the built-in
    /// `builtin/stats`.
    #[serde(default)]
    pub stats: Option<Slot<Stats>>,
    /// The items it carries: an `Items` asset, `"plugin/output"`. Without one, its plugin's
    /// first list of items.
    #[serde(default, with = "crate::asset::as_path_opt")]
    pub inventory: Option<OutputRef>,
    /// How the camera looks down at the character in Play mode: written in place or a camera
    /// asset's name. Without one, the built-in `builtin/top-down`.
    #[serde(default)]
    pub camera: Option<Slot<TopDownCamera>>,
    /// The camera in 3D worlds, where the character is an upright billboard: it needs a lower
    /// pitch than the top-down view. Without one, the built-in `builtin/billboard`.
    #[serde(default)]
    pub world_camera: Option<Slot<TopDownCamera>>,
    /// Its HUD, UI, menus and input map; empty slots get the built-in ones.
    #[serde(default, skip_serializing_if = "crate::ui::Kit::is_empty")]
    pub kit: crate::ui::Kit,
    /// Things the character can do besides walking (see [`crate::action`]).
    #[serde(default)]
    pub actions: Vec<Action>,
    /// What those actions throw or place.
    #[serde(default)]
    pub projectiles: Vec<Projectile>,
    /// The action of the dodge button, if the character has one.
    #[serde(default, alias = "roll")]
    pub dodge: Option<String>,
}

impl<M: Clone, A: Clone> Character2dOf<M, A> {
    /// What the character can do besides walking.
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

impl Character2dSpec {
    /// The character with its movement and animations found where they are named.
    pub fn resolve(&self, source: &dyn AssetSource) -> Result<Character2d, ResolveError> {
        let movement = crate::asset::resolve(
            &[(Layer::Asset, Some(&self.movement))],
            source,
            "top-down movement",
            crate::asset::movement_2d,
        )?
        .expect("the layer has a slot")
        .value;
        let animations = crate::asset::resolve(
            &[(Layer::Asset, Some(&self.animations))],
            source,
            "top-down animations",
            crate::asset::animations_2d,
        )?
        .expect("the layer has a slot")
        .value;
        Ok(Character2d {
            movement,
            animations,
            stats: self.stats.clone(),
            inventory: self.inventory.clone(),
            camera: self.camera.clone(),
            world_camera: self.world_camera.clone(),
            kit: self.kit.clone(),
            actions: self.actions.clone(),
            projectiles: self.projectiles.clone(),
            dodge: self.dodge.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Movement2d {
    /// Walking speed on the ground.
    pub speed: f32,
    /// Speed gained per second; without it the character is at full speed at once, as in many
    /// older games.
    #[serde(default)]
    pub acceleration: Option<f32>,
    /// Collision radius on the ground.
    pub radius: f32,
    /// Downward acceleration in 3D worlds, where it can fall off ledges (top-down maps have no
    /// height). The default is Earth's.
    #[serde(default = "default_gravity")]
    pub gravity: f32,
}

fn default_gravity() -> f32 {
    9.8
}

/// The sprite animation for each thing the character does, per facing direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animations2d {
    pub idle: Directional,
    pub walk: Directional,
}

/// One animation per facing direction. Games often draw only one side view and mirror it, so
/// each entry can say so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directional {
    pub north: AnimationRef,
    pub east: AnimationRef,
    pub south: AnimationRef,
    pub west: AnimationRef,
}

impl Directional {
    pub fn get(&self, direction: Direction4) -> &AnimationRef {
        match direction {
            Direction4::North => &self.north,
            Direction4::East => &self.east,
            Direction4::South => &self.south,
            Direction4::West => &self.west,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationRef {
    /// The sprite animation's name.
    pub name: String,
    /// Mirrors it left to right (on top of any mirroring its own steps have).
    #[serde(default)]
    pub flip_x: bool,
    /// Mirrors it top to bottom (on top of any mirroring its own steps have).
    #[serde(default)]
    pub flip_y: bool,
}

/// A camera above the character, north up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopDownCamera {
    pub distance: f32,
    /// Looking down at this angle in degrees; 90 is straight down.
    pub pitch: f32,
}

impl Default for TopDownCamera {
    fn default() -> Self {
        Self {
            distance: 9.0,
            pitch: 90.0,
        }
    }
}

/// The four ways a top-down character faces. North is the top of the map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction4 {
    North,
    East,
    #[default]
    South,
    West,
}

impl Direction4 {
    /// The facing for a stick direction (x east, y north): the stronger axis wins, and the
    /// current facing stays when the axes are equal or there is no input.
    pub fn from_input(x: f32, y: f32, current: Direction4) -> Direction4 {
        if x.abs() > y.abs() {
            if x > 0.0 {
                Direction4::East
            } else {
                Direction4::West
            }
        } else if y.abs() > x.abs() {
            if y > 0.0 {
                Direction4::North
            } else {
                Direction4::South
            }
        } else {
            current
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_follows_the_stronger_axis() {
        use Direction4::*;
        assert_eq!(Direction4::from_input(1.0, 0.2, South), East);
        assert_eq!(Direction4::from_input(-0.3, -0.9, East), South);
        assert_eq!(Direction4::from_input(0.0, 1.0, South), North);
        assert_eq!(Direction4::from_input(1.0, 1.0, West), West);
        assert_eq!(Direction4::from_input(0.0, 0.0, North), North);
    }

    #[test]
    fn parses_ron() {
        let character: Character2d = ron::from_str(
            r#"(
                movement: (speed: 4.0, radius: 0.4),
                animations: (
                    idle: (north: (name: "a"), east: (name: "b"), south: (name: "c"),
                           west: (name: "b", flip_x: true)),
                    walk: (north: (name: "d"), east: (name: "e"), south: (name: "f"),
                           west: (name: "e", flip_x: true)),
                ),
            )"#,
        )
        .unwrap();
        assert!(character.animations.idle.get(Direction4::West).flip_x);
        assert_eq!(character.movement.acceleration, None);
        assert_eq!(character.camera, None);
        assert_eq!(character.world_camera, None);
        assert_eq!(character.movement.gravity, 9.8);
    }

    #[test]
    fn older_names_still_load() {
        let character: Character2d = ron::from_str(
            r#"(
                movement: (speed: 4.0, radius: 0.4),
                animations: (
                    idle: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                    walk: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                ),
                projectiles: [(id: "p", motion: Boomerang(speed: 1.0, range: 2.0), lifetime: 1.0, radius: 0.1)],
                roll: Some("dash"),
            )"#,
        )
        .unwrap();
        assert_eq!(character.dodge.as_deref(), Some("dash"));
        assert_eq!(
            character.projectiles[0].motion,
            crate::action::ProjectileMotion::Return {
                speed: 1.0,
                range: 2.0
            }
        );
    }

    #[test]
    fn cameras_are_written_in_place_or_named() {
        let character: Character2d = ron::from_str(
            r#"(
                movement: (speed: 4.0, radius: 0.4),
                animations: (
                    idle: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                    walk: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                ),
                camera: Some((distance: 9.0, pitch: 70.0)),
                world_camera: Some("tmc/low-camera"),
            )"#,
        )
        .unwrap();
        assert_eq!(
            character.camera,
            Some(Slot::Inline(TopDownCamera {
                distance: 9.0,
                pitch: 70.0
            }))
        );
        assert_eq!(
            character.world_camera,
            Some(Slot::Ref(crate::asset::OutputRef::new("tmc", "low-camera")))
        );
    }
}

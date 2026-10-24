//! Assets: every plugin output is one, named `plugin/output` (an [`OutputRef`]). Assets are
//! made of other assets: a setting can be written in place or name the asset that holds it (a
//! [`Slot`]), and the [`resolve`]r follows those names, applying overrides in a fixed order.
//!
//! Each kind of asset (one per [`Output`] variant) is registered in [`KINDS`]: what it is called,
//! whether it can be placed in a scene, which assets it uses, and the generic built-in assets it
//! comes with. Built-ins belong to the [`BUILTIN`] plugin and hold no game data.

use std::{fmt, sync::LazyLock};

use serde::{Deserialize, Serialize};

pub use crate::compose::OutputRef;
use crate::{
    camera::CameraRig,
    character::{FollowCamera, Movement, StateAnimations},
    character2d::{Animations2d, Movement2d, TopDownCamera},
    input::InputMap,
    parts::{AnyAnimations, AnyMovement, Stats},
    recipe::{Kind, Output},
    ui::{HudLayout, Kit, Menu, Theme},
};

/// The plugin the generic built-in assets belong to.
pub const BUILTIN: &str = "builtin";

impl OutputRef {
    pub fn new(plugin: &str, output: &str) -> Self {
        Self {
            plugin: plugin.to_string(),
            output: output.to_string(),
        }
    }

    /// Reads `plugin/output`.
    pub fn parse(text: &str) -> Option<Self> {
        let (plugin, output) = text.split_once('/')?;
        (!plugin.is_empty() && !output.is_empty()).then(|| Self::new(plugin, output))
    }
}

impl fmt::Display for OutputRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.plugin, self.output)
    }
}

/// One asset exactly: its name and its registered kind. Names are unique per kind, not per
/// plugin (an ability and a sprite may both be `hero/sword`), so selections and previews keep
/// the kind too.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetId {
    pub at: OutputRef,
    /// The registered kind's name, as in [`KINDS`] ("Character" for composed characters).
    pub kind: &'static str,
}

impl AssetId {
    pub fn new(at: OutputRef, kind: &'static str) -> Self {
        Self { at, kind }
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.kind, self.at)
    }
}

/// A setting written in place, or the name of the asset that holds it: `"builtin/top-down"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Slot<T> {
    Ref(#[serde(with = "as_path")] OutputRef),
    Inline(T),
}

impl<T> From<OutputRef> for Slot<T> {
    fn from(at: OutputRef) -> Self {
        Slot::Ref(at)
    }
}

impl<T> Slot<T> {
    /// The asset it names, if it names one.
    pub fn target(&self) -> Option<&OutputRef> {
        match self {
            Slot::Ref(at) => Some(at),
            Slot::Inline(_) => None,
        }
    }
}

/// An optional [`OutputRef`] written as `"plugin/output"`.
pub mod as_path_opt {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    use super::OutputRef;

    pub fn serialize<S: Serializer>(at: &Option<OutputRef>, s: S) -> Result<S::Ok, S::Error> {
        match at {
            Some(at) => s.serialize_some(&at.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<OutputRef>, D::Error> {
        let Some(text) = Option::<String>::deserialize(d)? else {
            return Ok(None);
        };
        OutputRef::parse(&text)
            .map(Some)
            .ok_or_else(|| D::Error::custom(format!("\"{text}\" is not \"plugin/output\"")))
    }
}

/// An [`OutputRef`] written as `"plugin/output"`.
mod as_path {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    use super::OutputRef;

    pub fn serialize<S: Serializer>(at: &OutputRef, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&at.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OutputRef, D::Error> {
        let text = String::deserialize(d)?;
        OutputRef::parse(&text)
            .ok_or_else(|| D::Error::custom(format!("\"{text}\" is not \"plugin/output\"")))
    }
}

/// An asset another asset uses, and the slot it uses it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub slot: &'static str,
    pub asset: OutputRef,
}

/// A kind of asset, registered in [`KINDS`].
pub struct AssetKind {
    /// The [`Output`] variant, as [`Output::name`] gives it.
    pub name: &'static str,
    /// What the Editor's browser sorts it under, unless the output says.
    pub category: Option<Kind>,
    /// Whether it can be placed in a scene.
    pub placeable: bool,
    /// The assets an output of this kind (in plugin `plugin`) uses.
    pub uses: fn(plugin: &str, output: &Output) -> Vec<Use>,
    /// The generic assets of this kind that come with Clockwork.
    pub builtins: fn() -> Vec<Output>,
}

fn no_uses(_: &str, _: &Output) -> Vec<Use> {
    Vec::new()
}

fn no_builtins() -> Vec<Output> {
    Vec::new()
}

fn slot_use<T>(slot: &'static str, value: &Option<Slot<T>>) -> Option<Use> {
    let asset = value.as_ref()?.target()?.clone();
    Some(Use { slot, asset })
}

/// Every kind of asset.
pub static KINDS: &[AssetKind] = &[
    AssetKind {
        name: "Image",
        category: None,
        placeable: true,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Surfaces",
        category: None,
        placeable: false,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Scene",
        category: Some(Kind::Scene),
        placeable: true,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Model",
        category: None,
        placeable: true,
        uses: |_, output| match output {
            Output::Model {
                character: Some(c), ..
            } => [
                slot_use("movement", &Some(c.movement.clone())),
                slot_use("animations", &Some(c.animations.clone())),
                slot_use("stats", &c.stats),
                c.inventory.clone().map(|asset| Use {
                    slot: "inventory",
                    asset,
                }),
                slot_use("camera", &c.camera),
            ]
            .into_iter()
            .flatten()
            .chain(kit_uses(&c.kit))
            .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Items",
        category: None,
        placeable: false,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Ability",
        category: Some(Kind::Ability),
        placeable: false,
        uses: |plugin, output| match output {
            Output::Ability { from, .. } => vec![Use {
                slot: "character",
                asset: OutputRef::new(plugin, from),
            }],
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Sprite",
        category: None,
        placeable: true,
        uses: |_, output| match output {
            Output::Sprite {
                character: Some(c), ..
            } => [
                slot_use("movement", &Some(c.movement.clone())),
                slot_use("animations", &Some(c.animations.clone())),
                slot_use("stats", &c.stats),
                c.inventory.clone().map(|asset| Use {
                    slot: "inventory",
                    asset,
                }),
                slot_use("camera", &c.camera),
                slot_use("world camera", &c.world_camera),
            ]
            .into_iter()
            .flatten()
            .chain(kit_uses(&c.kit))
            .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Camera",
        category: Some(Kind::Camera),
        placeable: false,
        uses: |_, output| match output {
            Output::Camera { camera, .. } => slot_use("same as", &Some(camera.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: crate::camera::builtins,
    },
    AssetKind {
        name: "Hud",
        category: Some(Kind::Hud),
        placeable: false,
        uses: |_, output| match output {
            Output::Hud { hud, .. } => {
                let mut uses: Vec<Use> = slot_use("same as", &Some(hud.clone()))
                    .into_iter()
                    .collect();
                if let Slot::Inline(layout) = hud {
                    uses.extend(layout.widgets.iter().filter_map(|w| {
                        Some(Use {
                            slot: "icon",
                            asset: OutputRef::parse(w.icon.as_deref()?)?,
                        })
                    }));
                }
                uses
            }
            _ => Vec::new(),
        },
        builtins: hud_builtins,
    },
    AssetKind {
        name: "Ui",
        category: Some(Kind::Ui),
        placeable: false,
        uses: |_, output| match output {
            Output::Ui { ui, .. } => {
                let mut uses: Vec<Use> =
                    slot_use("same as", &Some(ui.clone())).into_iter().collect();
                if let Slot::Inline(theme) = ui {
                    let named = |slot, text: &Option<String>| {
                        Some(Use {
                            slot,
                            asset: OutputRef::parse(text.as_deref()?)?,
                        })
                    };
                    uses.extend(named("font", &theme.font));
                    uses.extend(named("dialog font", &theme.dialog_font));
                    uses.extend(named(
                        "frame",
                        &theme.frame.as_ref().map(|f| f.image.clone()),
                    ));
                }
                uses
            }
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Menu",
        category: Some(Kind::Menu),
        placeable: false,
        uses: |_, output| match output {
            Output::Menu { menu, .. } => slot_use("same as", &Some(menu.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Flow",
        category: Some(Kind::Flow),
        placeable: false,
        uses: |_, output| match output {
            Output::Flow { flow, .. } => {
                let mut uses = Vec::new();
                match flow {
                    Slot::Ref(asset) => uses.push(Use {
                        slot: "same as",
                        asset: asset.clone(),
                    }),
                    Slot::Inline(flow) => {
                        if let Some(title) = &flow.title {
                            uses.extend(title.logo.as_deref().and_then(OutputRef::parse).map(
                                |asset| Use {
                                    slot: "logo",
                                    asset,
                                },
                            ));
                        }
                        let mut label = |slot, value: &crate::flow::Label| {
                            uses.extend(value.target().map(|asset| Use {
                                slot,
                                asset: asset.clone(),
                            }));
                        };
                        if let Some(title) = &flow.title {
                            label("prompt", &title.prompt);
                        }
                        if let Some(files) = &flow.files {
                            label("file title", &files.title);
                        }
                        if let Some(acts) = &flow.acts {
                            for course in &acts.courses {
                                label("course name", &course.name);
                                for act in &course.acts {
                                    label("act name", act);
                                }
                            }
                        }
                        if let Some(game_over) = &flow.game_over {
                            label("game over", &game_over.text);
                        }
                    }
                }
                uses
            }
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "InputMap",
        category: Some(Kind::Input),
        placeable: false,
        uses: |_, output| match output {
            Output::InputMap { input, .. } => slot_use("same as", &Some(input.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: crate::input::builtins,
    },
    AssetKind {
        name: "Movement",
        category: Some(Kind::Movement),
        placeable: false,
        uses: |_, output| match output {
            Output::Movement { movement, .. } => slot_use("same as", &Some(movement.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Animations",
        category: Some(Kind::Animation),
        placeable: false,
        uses: |_, output| match output {
            Output::Animations { animations, .. } => slot_use("same as", &Some(animations.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Stats",
        category: Some(Kind::Stats),
        placeable: false,
        uses: |_, output| match output {
            Output::Stats { stats, .. } => slot_use("same as", &Some(stats.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: crate::parts::builtins,
    },
    AssetKind {
        name: "Behavior",
        category: Some(Kind::Behavior),
        placeable: false,
        uses: |_, output| match output {
            Output::Behavior { behavior, .. } => slot_use("same as", &Some(behavior.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: crate::behavior::builtins,
    },
    AssetKind {
        name: "Rules",
        category: Some(Kind::Rules),
        placeable: false,
        uses: |_, output| match output {
            Output::Rules { rules, .. } => slot_use("same as", &Some(rules.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: crate::game::builtins,
    },
    AssetKind {
        name: "Game",
        category: Some(Kind::Game),
        placeable: false,
        uses: |_, output| match output {
            Output::Game { game, .. } => {
                let mut uses: Vec<Use> = slot_use("same as", &Some(game.clone()))
                    .into_iter()
                    .collect();
                if let Slot::Inline(g) = game {
                    let named = [
                        ("rules", &g.rules),
                        ("flow", &g.flow),
                        ("camera", &g.camera),
                        ("HUD", &g.hud),
                        ("UI", &g.ui),
                        ("input", &g.input),
                        ("stats", &g.stats),
                        ("inventory", &g.inventory),
                    ];
                    for (slot, text) in named {
                        if let Some(asset) = text.as_deref().and_then(OutputRef::parse) {
                            uses.push(Use { slot, asset });
                        }
                    }
                    for menu in g.menus.iter().flatten() {
                        if let Some(asset) = OutputRef::parse(menu) {
                            uses.push(Use {
                                slot: "menu",
                                asset,
                            });
                        }
                    }
                }
                uses
            }
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Font",
        category: Some(Kind::Font),
        placeable: false,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Sound",
        category: Some(Kind::Audio),
        placeable: false,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Music",
        category: Some(Kind::Audio),
        placeable: false,
        uses: no_uses,
        builtins: no_builtins,
    },
    AssetKind {
        name: "Dialog",
        category: Some(Kind::Dialog),
        placeable: false,
        uses: |_, output| match output {
            Output::Dialog { dialog, .. } => slot_use("same as", &Some(dialog.clone()))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
    AssetKind {
        name: "Sounds",
        category: Some(Kind::Audio),
        placeable: false,
        uses: |_, output| match output {
            Output::Sounds { sounds, .. } => {
                let mut uses: Vec<Use> = slot_use("same as", &Some(sounds.clone()))
                    .into_iter()
                    .collect();
                if let Slot::Inline(set) = sounds {
                    for (slot, text) in [
                        ("hurt", &set.hurt),
                        ("pickup", &set.pickup),
                        ("door", &set.door),
                        ("menu", &set.menu),
                    ] {
                        if let Some(asset) = text.as_deref().and_then(OutputRef::parse) {
                            uses.push(Use { slot, asset });
                        }
                    }
                }
                uses
            }
            _ => Vec::new(),
        },
        builtins: no_builtins,
    },
];

/// Only the built-in assets: what a recipe sees when it runs on its own.
pub struct Builtins;

impl AssetSource for Builtins {
    fn outputs(&self, at: &OutputRef) -> Vec<&Output> {
        if at.plugin != BUILTIN {
            return Vec::new();
        }
        builtins().iter().filter(|o| o.id() == at.output).collect()
    }
}

/// The HUD, UI and menu built-ins (registered with the HUD kind).
fn hud_builtins() -> Vec<Output> {
    crate::ui::builtins()
}

/// What a character's kit names.
fn kit_uses(kit: &Kit) -> Vec<Use> {
    let mut uses: Vec<Use> = [
        slot_use("HUD", &kit.hud),
        slot_use("UI", &kit.ui),
        slot_use("input", &kit.input),
        slot_use("sounds", &kit.sounds),
    ]
    .into_iter()
    .flatten()
    .collect();
    for menu in kit.menus.iter().flatten() {
        uses.extend(menu.target().map(|asset| Use {
            slot: "menu",
            asset: asset.clone(),
        }));
    }
    uses
}

/// The registered kind of an output.
pub fn kind(output: &Output) -> &'static AssetKind {
    KINDS
        .iter()
        .find(|k| k.name == output.name())
        .expect("every output variant is registered in KINDS")
}

/// A new output of a kind (by [`Output::name`]) with id `id`, for editors to add: the kind's
/// first built-in asset, or one holding default values. `None` for kinds with nothing to start
/// from that a template couldn't say better (images, models: they need a step's result).
pub fn starter(name: &str, id: &str) -> Option<Output> {
    let mut output = builtins()
        .iter()
        .find(|o| o.name() == name)
        .cloned()
        .or_else(|| {
            let id = id.to_string();
            Some(match name {
                "Hud" => Output::Hud {
                    id,
                    hud: Slot::Inline(Default::default()),
                },
                "Flow" => Output::Flow {
                    id,
                    flow: Slot::Inline(Default::default()),
                },
                "InputMap" => Output::InputMap {
                    id,
                    input: Slot::Inline(Default::default()),
                },
                "Stats" => Output::Stats {
                    id,
                    stats: Slot::Inline(Default::default()),
                },
                "Rules" => Output::Rules {
                    id,
                    rules: Slot::Inline(Default::default()),
                },
                "Game" => Output::Game {
                    id,
                    game: Slot::Inline(Default::default()),
                },
                "Sounds" => Output::Sounds {
                    id,
                    sounds: Slot::Inline(Default::default()),
                },
                // A walk at a usual speed, and each direction's animations named after it.
                "Movement" => Output::Movement {
                    id,
                    movement: Slot::Inline(crate::parts::AnyMovement::TopDown(
                        crate::character2d::Movement2d {
                            speed: 4.0,
                            acceleration: None,
                            radius: 0.4,
                            gravity: 9.8,
                        },
                    )),
                },
                "Animations" => {
                    use crate::character2d::{AnimationRef, Animations2d, Directional};
                    let named = |name: String| AnimationRef {
                        name,
                        flip_x: false,
                        flip_y: false,
                    };
                    let set = |what: &str| Directional {
                        north: named(format!("{what}_north")),
                        east: named(format!("{what}_east")),
                        south: named(format!("{what}_south")),
                        west: named(format!("{what}_west")),
                    };
                    Output::Animations {
                        id,
                        animations: Slot::Inline(crate::parts::AnyAnimations::Directional(
                            Animations2d {
                                idle: set("idle"),
                                walk: set("walk"),
                            },
                        )),
                    }
                }
                _ => return None,
            })
        })?;
    output.set_id(id);
    Some(output)
}

#[cfg(test)]
mod starter_tests {
    #[test]
    fn starters_are_named_as_asked() {
        for name in ["Camera", "Hud", "Flow", "Movement", "Animations", "Sounds"] {
            let output = super::starter(name, "new_one").unwrap_or_else(|| panic!("{name}"));
            assert_eq!((output.name(), output.id()), (name, "new_one"));
        }
        assert!(super::starter("Image", "x").is_none());
    }
}

/// The built-in assets of every kind, all in the [`BUILTIN`] plugin.
pub fn builtins() -> &'static [Output] {
    static ALL: LazyLock<Vec<Output>> =
        LazyLock::new(|| KINDS.iter().flat_map(|k| (k.builtins)()).collect());
    &ALL
}

/// Where assets are looked up by name. Ids are unique per kind, not per plugin (an ability and
/// a sprite may both be `hero/sword`), so a name can stand for several outputs.
pub trait AssetSource {
    fn outputs(&self, at: &OutputRef) -> Vec<&Output>;
}

/// Where a setting came from, strongest first: a scene node's override, the game's, the asset's
/// own, or the kind's built-in default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Node,
    Game,
    Asset,
    Default,
}

/// A setting found by [`resolve`]: its value, the layer it came from and the assets passed on
/// the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved<T> {
    pub value: T,
    pub layer: Layer,
    pub via: Vec<OutputRef>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} doesn't exist")]
    Missing(OutputRef),
    #[error("{at} isn't a {expected}")]
    WrongKind {
        at: OutputRef,
        expected: &'static str,
    },
    #[error("these assets name each other in a loop: {}", list(.0))]
    Loop(Vec<OutputRef>),
}

fn list(assets: &[OutputRef]) -> String {
    assets
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(" → ")
}

/// Finds a setting: the strongest layer that has one wins, and names are followed until a value
/// written in place. `take` gives the setting an asset holds, or `None` when the asset is of
/// another kind than `expected`. `Ok(None)` when no layer has the setting.
pub fn resolve<T: Clone>(
    layers: &[(Layer, Option<&Slot<T>>)],
    source: &dyn AssetSource,
    expected: &'static str,
    take: impl Fn(&Output) -> Option<Slot<T>>,
) -> Result<Option<Resolved<T>>, ResolveError> {
    let Some((layer, slot)) = layers.iter().find_map(|(l, s)| s.map(|s| (*l, s))) else {
        return Ok(None);
    };
    let mut via: Vec<OutputRef> = Vec::new();
    let mut slot = slot.clone();
    loop {
        let at = match slot {
            Slot::Inline(value) => return Ok(Some(Resolved { value, layer, via })),
            Slot::Ref(at) => at,
        };
        if via.contains(&at) {
            via.push(at);
            return Err(ResolveError::Loop(via));
        }
        let outputs = source.outputs(&at);
        if outputs.is_empty() {
            return Err(ResolveError::Missing(at));
        }
        // The output of the kind wanted, among those of that name.
        slot = outputs
            .into_iter()
            .find_map(&take)
            .ok_or_else(|| ResolveError::WrongKind {
                at: at.clone(),
                expected,
            })?;
        via.push(at);
    }
}

/// The top-down camera a camera asset holds.
pub fn top_down_camera(output: &Output) -> Option<Slot<TopDownCamera>> {
    match output {
        Output::Camera { camera, .. } => match camera {
            Slot::Ref(at) => Some(Slot::Ref(at.clone())),
            Slot::Inline(CameraRig::TopDown(c)) => Some(Slot::Inline(*c)),
            Slot::Inline(CameraRig::Follow(_)) => None,
        },
        _ => None,
    }
}

/// The setting an asset of one kind holds, for [`resolve`]: `hud_of`, `ui_of`, `menu_of` and
/// `input_of` read HUD, UI, menu and input map assets.
macro_rules! holds {
    ($name:ident, $variant:ident, $field:ident, $t:ty) => {
        pub fn $name(output: &Output) -> Option<Slot<$t>> {
            match output {
                Output::$variant { $field, .. } => Some($field.clone()),
                _ => None,
            }
        }
    };
}

holds!(hud_of, Hud, hud, HudLayout);
holds!(ui_of, Ui, ui, Theme);
holds!(menu_of, Menu, menu, Menu);
holds!(flow_of, Flow, flow, crate::flow::Flow);
holds!(input_of, InputMap, input, InputMap);
holds!(stats_of, Stats, stats, Stats);
holds!(behavior_of, Behavior, behavior, crate::behavior::Behavior);
holds!(rules_of, Rules, rules, crate::game::Rules);
holds!(game_of, Game, game, crate::game::GameSettings);
holds!(dialog_of, Dialog, dialog, crate::ui::Dialog);
holds!(sounds_of, Sounds, sounds, crate::ui::SoundSet);

/// The setting of one kind an asset holding several kinds holds: `movement_2d`, `movement_3d`,
/// `animations_2d` and `animations_3d` read movement and animation assets.
macro_rules! holds_one {
    ($name:ident, $variant:ident, $field:ident, $kind:path, $t:ty) => {
        pub fn $name(output: &Output) -> Option<Slot<$t>> {
            match output {
                Output::$variant { $field, .. } => match $field {
                    Slot::Ref(at) => Some(Slot::Ref(at.clone())),
                    Slot::Inline($kind(value)) => Some(Slot::Inline(value.clone())),
                    Slot::Inline(_) => None,
                },
                _ => None,
            }
        }
    };
}

holds_one!(
    movement_2d,
    Movement,
    movement,
    AnyMovement::TopDown,
    Movement2d
);
holds_one!(
    movement_3d,
    Movement,
    movement,
    AnyMovement::Platformer,
    Movement
);
holds_one!(
    animations_2d,
    Animations,
    animations,
    AnyAnimations::Directional,
    Animations2d
);
holds_one!(
    animations_3d,
    Animations,
    animations,
    AnyAnimations::States,
    StateAnimations
);

/// The follow camera a camera asset holds.
pub fn follow_camera(output: &Output) -> Option<Slot<FollowCamera>> {
    match output {
        Output::Camera { camera, .. } => match camera {
            Slot::Ref(at) => Some(Slot::Ref(at.clone())),
            Slot::Inline(CameraRig::Follow(c)) => Some(Slot::Inline(c.clone())),
            Slot::Inline(CameraRig::TopDown(_)) => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn flows_track_logos_and_every_referenced_label() {
        let output: Output = ron::from_str(r#"Flow(id: "screens", flow: (
            title: Some((logo: Some("art/logo"), prompt: "words/start")),
            files: Some((title: "words/files", counts: Some("star"))),
            acts: Some((courses: [
                (name: "words/course", entry: "node name", acts: ["words/act", (pages: ["Inline"])]),
                (name: (pages: ["Inline"]), entry: "other", acts: ["words/other-act"]),
            ])),
            game_over: Some((text: "words/over")),
        ))"#).unwrap();
        let kind = kind(&output);
        assert_eq!(kind.category, Some(Kind::Flow));
        assert!(!kind.placeable);
        let uses = (kind.uses)("game", &output);
        assert_eq!(
            uses.iter().map(|u| u.asset.to_string()).collect::<Vec<_>>(),
            [
                "art/logo",
                "words/start",
                "words/files",
                "words/course",
                "words/act",
                "words/other-act",
                "words/over"
            ]
        );
        let alias = Output::Flow {
            id: "alias".into(),
            flow: Slot::Ref(OutputRef::new("game", "screens")),
        };
        assert_eq!(
            (kind.uses)("game", &alias),
            vec![Use {
                slot: "same as",
                asset: OutputRef::new("game", "screens"),
            }]
        );
        let source = Map(HashMap::from([
            (OutputRef::new("game", "screens"), output.clone()),
            (OutputRef::new("game", "alias"), alias),
        ]));
        let resolved = resolve(
            &[(
                Layer::Game,
                Some(&Slot::Ref(OutputRef::new("game", "alias"))),
            )],
            &source,
            "flow",
            flow_of,
        )
        .unwrap()
        .unwrap();
        assert_eq!(Some(Slot::Inline(resolved.value)), flow_of(&output));
    }

    #[test]
    fn themes_track_both_font_dependencies() {
        let output = Output::Ui {
            id: "theme".into(),
            ui: Slot::Inline(crate::ui::Theme {
                font: Some("sm64/hud".into()),
                dialog_font: Some("sm64/dialog".into()),
                ..crate::ui::default_theme()
            }),
        };
        let uses = (kind(&output).uses)("game", &output);
        assert!(
            uses.iter()
                .any(|u| u.asset == OutputRef::new("sm64", "hud"))
        );
        assert!(
            uses.iter()
                .any(|u| u.asset == OutputRef::new("sm64", "dialog"))
        );
    }

    struct Map(HashMap<OutputRef, Output>);

    impl AssetSource for Map {
        fn outputs(&self, at: &OutputRef) -> Vec<&Output> {
            self.0.get(at).into_iter().collect()
        }
    }

    fn camera(id: &str, camera: Slot<CameraRig>) -> (OutputRef, Output) {
        (
            OutputRef::new("p", id),
            Output::Camera {
                id: id.into(),
                camera,
            },
        )
    }

    fn top(pitch: f32) -> Slot<CameraRig> {
        Slot::Inline(CameraRig::TopDown(TopDownCamera {
            distance: 9.0,
            pitch,
        }))
    }

    fn named(id: &str) -> Slot<TopDownCamera> {
        Slot::Ref(OutputRef::new("p", id))
    }

    fn source() -> Map {
        Map(HashMap::from([
            camera("high", top(80.0)),
            camera("alias", Slot::Ref(OutputRef::new("p", "high"))),
            camera("a", Slot::Ref(OutputRef::new("p", "b"))),
            camera("b", Slot::Ref(OutputRef::new("p", "a"))),
            camera(
                "behind",
                Slot::Inline(CameraRig::Follow(FollowCamera {
                    distance: 5.0,
                    pitch: 20.0,
                    height: 1.0,
                    follow: 2.0,
                })),
            ),
        ]))
    }

    fn pitch(layers: &[(Layer, Option<&Slot<TopDownCamera>>)]) -> Result<f32, ResolveError> {
        resolve(layers, &source(), "top-down camera", top_down_camera)
            .map(|r| r.expect("a layer has it").value.pitch)
    }

    #[test]
    fn the_strongest_layer_with_a_setting_wins() {
        let own = named("high");
        let inline = Slot::Inline(TopDownCamera {
            distance: 1.0,
            pitch: 10.0,
        });
        assert_eq!(
            pitch(&[(Layer::Node, None), (Layer::Asset, Some(&own))]),
            Ok(80.0)
        );
        assert_eq!(
            pitch(&[(Layer::Node, Some(&inline)), (Layer::Asset, Some(&own))]),
            Ok(10.0)
        );
        let none: Result<_, _> =
            resolve::<TopDownCamera>(&[(Layer::Node, None)], &source(), "camera", top_down_camera);
        assert_eq!(none, Ok(None));
    }

    #[test]
    fn names_are_followed_and_remembered() {
        let r = resolve(
            &[(Layer::Asset, Some(&named("alias")))],
            &source(),
            "top-down camera",
            top_down_camera,
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.value.pitch, 80.0);
        assert_eq!(r.layer, Layer::Asset);
        assert_eq!(
            r.via,
            vec![OutputRef::new("p", "alias"), OutputRef::new("p", "high")]
        );
    }

    #[test]
    fn missing_wrong_and_looping_names_are_errors() {
        assert_eq!(
            pitch(&[(Layer::Asset, Some(&named("nowhere")))]),
            Err(ResolveError::Missing(OutputRef::new("p", "nowhere")))
        );
        assert!(matches!(
            pitch(&[(Layer::Asset, Some(&named("behind")))]),
            Err(ResolveError::WrongKind { .. })
        ));
        assert!(matches!(
            pitch(&[(Layer::Asset, Some(&named("a")))]),
            Err(ResolveError::Loop(v)) if v.len() == 3
        ));
    }

    #[test]
    fn a_slot_reads_a_name_or_a_value() {
        let named: Slot<TopDownCamera> = ron::from_str("\"builtin/top-down\"").unwrap();
        assert_eq!(named, Slot::Ref(OutputRef::new(BUILTIN, "top-down")));
        let inline: Slot<TopDownCamera> = ron::from_str("(distance: 8.0, pitch: 30.0)").unwrap();
        assert_eq!(
            inline,
            Slot::Inline(TopDownCamera {
                distance: 8.0,
                pitch: 30.0
            })
        );
        assert!(ron::from_str::<Slot<TopDownCamera>>("\"no-slash\"").is_err());
        assert_eq!(
            ron::to_string(&named).unwrap(),
            "\"builtin/top-down\"".to_string()
        );
    }

    #[test]
    fn a_character_finds_the_movement_it_names() {
        let walk = Output::Movement {
            id: "walk".into(),
            movement: Slot::Inline(AnyMovement::TopDown(
                ron::from_str("(speed: 5.0, radius: 0.3)").unwrap(),
            )),
        };
        let source = Map(HashMap::from([(OutputRef::new("p", "walk"), walk)]));
        let spec: crate::character2d::Character2dSpec = ron::from_str(
            r#"(
                movement: "p/walk",
                animations: (
                    idle: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                    walk: (north: (name: "a"), east: (name: "a"), south: (name: "a"), west: (name: "a")),
                ),
            )"#,
        )
        .unwrap();
        let character = spec.resolve(&source).unwrap();
        assert_eq!(character.movement.speed, 5.0);
        let wrong: crate::character2d::Character2dSpec = ron::from_str(
            &ron::to_string(&spec)
                .unwrap()
                .replace("p/walk", "p/nowhere"),
        )
        .unwrap();
        assert!(matches!(
            wrong.resolve(&source),
            Err(ResolveError::Missing(_))
        ));
    }

    #[test]
    fn every_output_kind_is_registered_and_builtins_are_found() {
        for output in crate::recipe_schema::OUTPUTS {
            assert!(
                KINDS.iter().any(|k| k.name == output.name),
                "{} is not in KINDS",
                output.name
            );
        }
        for b in builtins() {
            assert_eq!(kind(b).name, b.name());
        }
        assert!(builtins().iter().any(|b| b.id() == crate::camera::TOP_DOWN));
    }
}

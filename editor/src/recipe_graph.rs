//! An extraction plugin's recipe drawn as a node graph: one node per step and per output, wired
//! by the names steps make and use (buffers, palette banks, images, models, collisions, sprites).
//!
//! The graph is a view over `recipe.ron` as text. Every change (a number, a name, a wire, a new
//! or removed step) replaces a span of that text and is parsed again before it is kept, so the
//! agent's comments survive and nothing the format can't hold is ever written. Each change runs
//! the recipe on the user's ROM, and nodes show what their step made.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::Path,
    sync::Arc,
};

use bevy_egui::egui::{
    self, Color32, ColorImage, Pos2, Rect, RichText, Stroke, TextureHandle, TextureOptions, Ui,
    Vec2, pos2, vec2,
};
use clockwork_core::{
    asset::Slot,
    camera::CameraRig,
    image::RgbaImage,
    parts::{AnyAnimations, AnyMovement},
    recipe::{Access, NameKind, Output, Recipe, Seg, Step},
    recipe_schema::{self, FieldInfo, FieldType, VariantInfo},
};
use clockwork_recipe::{
    Made,
    text::{Doc, Kind, Node, format_int_like, parse_int, parse_string, string_literal},
};
use egui_snarl::{
    InPin, InPinId, NodeId, OutPin, OutPinId, Snarl,
    ui::{
        AnyPins, BackgroundPattern, NodeLayout, PinInfo, PinPlacement, SnarlPin, SnarlStyle,
        SnarlViewer,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    graph_view::{MAX_SCALE, MIN_SCALE, Navigator},
    kit, theme,
};

/// Where the nodes of a recipe sit, next to its `recipe.ron`.
pub const LAYOUT_FILE: &str = "graph.ron";

/// Distance between columns and rows before nodes are arranged by their sizes.
const COLUMN: f32 = 320.0;
const ROW: f32 = 160.0;
const GAP: f32 = 40.0;
/// Largest side of a kept image preview, in pixels.
const THUMB: u32 = 256;
/// Largest side of the preview drawn in a node, in points.
const NODE_THUMB: f32 = 72.0;
/// Changes kept for undo.
const UNDO_LIMIT: usize = 200;

/// A node: a step or an output, by its place in the recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RNode {
    Step(usize),
    Output(usize),
}

/// A name a node uses or makes (see [`clockwork_core::recipe::Port`]).
#[derive(Clone, Debug, PartialEq)]
struct OwnedPort {
    path: Vec<Seg>,
    name: String,
    kind: NameKind,
    access: Access,
}

impl OwnedPort {
    /// What the input or output is called on the node.
    fn label(&self) -> String {
        match self.path.as_slice() {
            [Seg::Field("segments"), Seg::Key(k)] => format!("segment {k:#04x}"),
            [Seg::Field("grids"), Seg::Index(i), ..] => format!("grid {i}"),
            [Seg::Field(outer), Seg::Field(inner), ..] if *inner != "from" => (*inner).into(),
            [Seg::Field(field), ..] => (*field).into(),
            _ => "?".into(),
        }
    }
}

fn kind_color(kind: NameKind) -> Color32 {
    match kind {
        NameKind::Buffer => Color32::from_rgb(0x7F, 0xA8, 0xB8),
        NameKind::Bank => Color32::from_rgb(0xB0, 0x8C, 0xD0),
        NameKind::Image => theme::BRASS_BRIGHT,
        NameKind::Model => Color32::from_rgb(0xD8, 0x8A, 0x50),
        NameKind::Collision => Color32::from_rgb(0xC0, 0x60, 0x58),
        NameKind::Sprite => Color32::from_rgb(0x8C, 0xC0, 0x70),
    }
}

fn ports_of(recipe: &Recipe, node: RNode) -> Vec<OwnedPort> {
    let ports = match node {
        RNode::Step(i) => recipe.steps.get(i).map(Step::ports),
        RNode::Output(i) => recipe.outputs.get(i).map(Output::ports),
    };
    ports
        .unwrap_or_default()
        .into_iter()
        .map(|p| OwnedPort {
            path: p.path,
            name: p.name.to_string(),
            kind: p.kind,
            access: p.access,
        })
        .collect()
}

/// The ports a node shows as inputs (those it reads), by port index.
fn inputs_of(ports: &[OwnedPort]) -> Vec<usize> {
    (0..ports.len())
        .filter(|&i| ports[i].access.reads())
        .collect()
}

/// The ports a node shows as outputs (those it makes or changes), by port index.
fn outputs_of(ports: &[OwnedPort]) -> Vec<usize> {
    (0..ports.len())
        .filter(|&i| ports[i].access.writes())
        .collect()
}

/// Which step makes what each input reads: wires from (node, port) to (node, port).
#[derive(Default, Debug, PartialEq)]
struct Resolution {
    wires: Vec<((RNode, usize), (RNode, usize))>,
    /// Inputs that need something no earlier step makes.
    missing: HashSet<(RNode, usize)>,
}

fn resolve(recipe: &Recipe, ports: &HashMap<RNode, Vec<OwnedPort>>) -> Resolution {
    let mut resolution = Resolution::default();
    let mut latest: HashMap<(NameKind, &str), (RNode, usize)> = HashMap::new();
    let nodes = (0..recipe.steps.len())
        .map(RNode::Step)
        .chain((0..recipe.outputs.len()).map(RNode::Output));
    for node in nodes {
        let node_ports = &ports[&node];
        for (i, port) in node_ports.iter().enumerate() {
            if !port.access.reads() {
                continue;
            }
            match latest.get(&(port.kind, port.name.as_str())) {
                Some(from) => resolution.wires.push((*from, (node, i))),
                None if port.access != Access::Grow => {
                    resolution.missing.insert((node, i));
                }
                None => {}
            }
        }
        for (i, port) in node_ports.iter().enumerate() {
            if port.access.writes() {
                latest.insert((port.kind, port.name.as_str()), (node, i));
            }
        }
    }
    resolution
}

/// What a step or output made, for its node.
#[derive(Clone, Debug, Default)]
struct StepResult {
    lines: Vec<String>,
    /// The first bytes of a buffer it made.
    bytes: Option<Vec<u8>>,
    image: Option<RgbaImage>,
}

#[derive(Default)]
struct Trace {
    /// By step; `None` for steps that didn't run.
    steps: Vec<Option<StepResult>>,
    /// By output, once the whole recipe ran.
    outputs: Vec<Option<StepResult>>,
    error: Option<String>,
    error_step: Option<usize>,
    /// The output the error is in.
    error_output: Option<usize>,
}

/// Bytes kept of a buffer, to show.
const PEEK: usize = 256;

/// Bytes as rows of 16 in hex, each after its offset.
fn hex_rows(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            format!("{:04X}  {}", row * 16, hex.join(" "))
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// What each output of a recipe that ran is, for its node.
fn summarize_outputs(
    recipe: &Recipe,
    extracted: &clockwork_recipe::Extracted,
) -> Vec<Option<StepResult>> {
    recipe
        .outputs
        .iter()
        .map(|output| {
            let id = output.id();
            let mut result = StepResult::default();
            match output {
                Output::Image { .. } => {
                    let (_, image) = extracted.images.iter().find(|(i, _)| i == id)?;
                    result
                        .lines
                        .push(format!("{}×{} pixels", image.width, image.height));
                    result.image = Some(shrink(image, THUMB));
                }
                Output::Surfaces { .. } => {
                    let (_, map) = extracted.surfaces.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{}×{} cells of {}, {} kinds of surface",
                        map.width,
                        map.height,
                        map.cell_size,
                        map.kinds.len()
                    ));
                }
                Output::Model { .. } => {
                    let m = extracted.models.iter().find(|m| m.id == id)?;
                    result.lines.push(format!(
                        "{} bones, {} meshes, {} animations",
                        m.model.bones.len(),
                        m.model.meshes.len(),
                        m.model.animations.len()
                    ));
                    if let Some(terrain) = &m.terrain {
                        result
                            .lines
                            .push(format!("walkable: {} triangles", terrain.triangles.len()));
                    }
                    if m.character.is_some() {
                        result.lines.push("playable".into());
                    }
                }
                Output::Sprite { .. } => {
                    let s = extracted.sprites.iter().find(|s| s.id == id)?;
                    result.lines.push(format!(
                        "{} frames, {} animations{}",
                        s.sprite.frames.len(),
                        s.sprite.animations.len(),
                        if s.character.is_some() {
                            ", playable"
                        } else {
                            ""
                        }
                    ));
                    result.image = s.sprite.frames.first().map(|f| shrink(&f.image, THUMB));
                }
                Output::Items { .. } => {
                    let (_, items) = extracted.items.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!("{} items", items.len()));
                }
                Output::Ability { .. } => {
                    let a = extracted.abilities.iter().find(|a| a.id == id)?;
                    result
                        .lines
                        .push(format!("{}: {} roles", a.name, a.roles.len()));
                }
                Output::Camera { camera, .. } => result.lines.push(match camera {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(CameraRig::Follow(c)) => {
                        format!("behind, {} m at {}°", c.distance, c.pitch)
                    }
                    Slot::Inline(CameraRig::TopDown(c)) => {
                        format!("above, {} m at {}°", c.distance, c.pitch)
                    }
                }),
                Output::Hud { hud, .. } => result.lines.push(match hud {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(h) => format!("{} widgets", h.widgets.len()),
                }),
                Output::Ui { ui, .. } => result.lines.push(match ui {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(t) => format!(
                        "{} font, {}",
                        t.font.as_deref().unwrap_or("default"),
                        if t.frame.is_some() {
                            "framed panels"
                        } else {
                            "plain panels"
                        }
                    ),
                }),
                Output::Menu { menu, .. } => result.lines.push(match menu {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(m) => format!("{}: opens with {}", m.title, m.opens_with),
                }),
                Output::InputMap { input, .. } => result.lines.push(match input {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(m) => format!("{} buttons bound", m.bindings.len()),
                }),
                Output::Movement { movement, .. } => result.lines.push(match movement {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(AnyMovement::TopDown(m)) => format!("walks at {} m/s", m.speed),
                    Slot::Inline(AnyMovement::Platformer(m)) => {
                        format!("runs at {} m/s", m.run_speed)
                    }
                }),
                Output::Animations { animations, .. } => result.lines.push(match animations {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(AnyAnimations::Directional(_)) => "by facing".to_string(),
                    Slot::Inline(AnyAnimations::States(_)) => "by state".to_string(),
                }),
                Output::Stats { stats, .. } => result.lines.push(match stats {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(s) => format!("{} health", s.health),
                }),
                Output::Behavior { behavior, .. } => result.lines.push(match behavior {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(b) => format!("{}, {} m around", b.kind(), b.radius()),
                }),
                Output::Rules { rules, .. } => result.lines.push(match rules {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(r) => format!(
                        "{:?}{}",
                        r.when_down,
                        r.lives.map_or(String::new(), |l| format!(", {l} lives"))
                    ),
                }),
                Output::Game { game, .. } => result.lines.push(match game {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(_) => "a game's assets".to_string(),
                }),
                Output::Flow { flow, .. } => result.lines.push(match flow {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(f) => {
                        let screens: Vec<&str> = [
                            f.title.as_ref().map(|_| "title"),
                            f.files.as_ref().map(|_| "files"),
                            f.acts.as_ref().map(|_| "acts"),
                            f.game_over.as_ref().map(|_| "game over"),
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        format!("screens: {}", screens.join(", "))
                    }
                }),
                Output::Font { chars, .. } => {
                    let (_, font) = extracted.fonts.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{} glyphs of {}×{}",
                        chars.chars().count(),
                        font.glyph.0,
                        font.glyph.1
                    ));
                    result.image = Some(shrink(&font.image, THUMB));
                }
                Output::Sound { .. } => {
                    let (_, pcm) = extracted.sounds.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{:.2} s at {} Hz",
                        pcm.samples.len() as f32 / pcm.rate.max(1) as f32,
                        pcm.rate
                    ));
                }
                Output::Dialog { dialog, .. } => result.lines.push(match dialog {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(d) => format!("{} pages", d.pages.len()),
                }),
                Output::Sounds { sounds, .. } => result.lines.push(match sounds {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(_) => "a character's sounds".to_string(),
                }),
            }
            Some(result)
        })
        .collect()
}

fn summarize(ports: &[OwnedPort], made: &Made) -> StepResult {
    let mut result = StepResult::default();
    for port in ports.iter().filter(|p| p.access.writes()) {
        let name = &port.name;
        let line = match port.kind {
            NameKind::Buffer => made.buffer(name).map(|b| {
                if result.bytes.is_none() {
                    result.bytes = Some(b[..b.len().min(PEEK)].to_vec());
                }
                format!("{name}: {} bytes", b.len())
            }),
            NameKind::Bank => made.has_bank(name).then(|| format!("{name}: palettes")),
            NameKind::Image => made.image(name).map(|image| {
                if result.image.is_none() {
                    result.image = Some(shrink(image, THUMB));
                }
                format!("{name}: {}×{}", image.width, image.height)
            }),
            NameKind::Model => made.model(name).map(|m| {
                format!(
                    "{name}: {} bones, {} meshes, {} animations",
                    m.bones.len(),
                    m.meshes.len(),
                    m.animations.len()
                )
            }),
            NameKind::Collision => made
                .collision(name)
                .map(|c| format!("{name}: {} triangles", c.triangles.len())),
            NameKind::Sprite => made.sprite(name).map(|s| {
                if result.image.is_none() {
                    result.image = s.frames.first().map(|f| shrink(&f.image, THUMB));
                }
                format!(
                    "{name}: {} frames, {} animations",
                    s.frames.len(),
                    s.animations.len()
                )
            }),
        };
        result.lines.extend(line);
    }
    result
}

/// `image` made at most `side` pixels on its longer side (nearest pixel), for previews.
fn shrink(image: &RgbaImage, side: u32) -> RgbaImage {
    let longest = image.width.max(image.height).max(1);
    if longest <= side {
        return image.clone();
    }
    let w = (image.width * side / longest).max(1);
    let h = (image.height * side / longest).max(1);
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.set(x, y, image.get(x * image.width / w, y * image.height / h));
        }
    }
    out
}

/// Where the nodes sit, saved as `graph.ron`: by place in the recipe, with each node's variant
/// so a recipe changed elsewhere doesn't put its nodes in the wrong places.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeLayout {
    #[serde(default)]
    pub steps: Vec<(String, (f32, f32))>,
    #[serde(default)]
    pub outputs: Vec<(String, (f32, f32))>,
}

pub fn load_layout(dir: &Path) -> RecipeLayout {
    std::fs::read_to_string(dir.join(LAYOUT_FILE))
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_layout(dir: &Path, layout: &RecipeLayout) -> Result<(), String> {
    let config = ron::ser::PrettyConfig::new().depth_limit(2);
    let body = ron::ser::to_string_pretty(layout, config).map_err(|e| e.to_string())?;
    let text = format!(
        "// Where the plugin editor draws this recipe's nodes. The plugin itself is recipe.ron.\n{body}\n"
    );
    std::fs::write(dir.join(LAYOUT_FILE), text).map_err(|e| e.to_string())
}

/// New steps and outputs, as written in RON. Names starting with `new_` get a number that makes
/// them unique.
pub const STEP_TEMPLATES: &[(&str, &str)] = &[
    (
        "Read",
        r#"Read(name: "new_buffer", offset: 0x0, size: Some(16))"#,
    ),
    ("Place", r#"Place(from: "", into: "new_buffer", at: 0x0)"#),
    ("Fill", r#"Fill(name: "new_buffer", size: 16)"#),
    ("Literal", r#"Literal(name: "new_buffer", hex: "00")"#),
    (
        "GbaPalettes",
        r#"GbaPalettes(from: "", bank: "bg", slot: 0)"#,
    ),
    (
        "GbaMetatiles",
        r#"GbaMetatiles(tiles: "", char_base: 0x0, metatiles: "", bank: "bg", columns: 16, into: "new_image")"#,
    ),
    (
        "GbaMetatileMap",
        r#"GbaMetatileMap(tiles: "", char_base: 0x0, metatiles: "", map: "", width: 16, bank: "bg", into: "new_image")"#,
    ),
    ("Overlay", r#"Overlay(from: "", into: "")"#),
    (
        "Crop",
        r#"Crop(from: "", region: (x: 0, y: 0, w: 16, h: 16), into: "new_image")"#,
    ),
    (
        "Text",
        r#"Text(from: "", at: 0x0, runs: [(0x00, "0123456789")], codes: {}, end: 0xFF, into: "text")"#,
    ),
    (
        "Canvas",
        r#"Canvas(name: "new_image", width: 64, height: 64)"#,
    ),
    ("Paste", r#"Paste(from: "", into: "", at: (0, 0))"#),
    (
        "N64Texture",
        r#"N64Texture(from: "", at: 0x0, format: Rgba16, width: 16, height: 16, into: "new_image")"#,
    ),
    (
        "Lookup",
        r#"Lookup(indices: "", table: "", into: "new_buffer")"#,
    ),
    (
        "N64Model",
        r#"N64Model(segments: {}, bones: [], into: "new_model")"#,
    ),
    (
        "CollisionTriangles",
        r#"CollisionTriangles(from: "", at: 0x0, layout: (word: U16, end: 0x0), into: "new_collision")"#,
    ),
    (
        "ChannelAnimation",
        r#"ChannelAnimation(from: "", name: "new_animation", index: 0x0, values: 0x0, frames: 1, into: "")"#,
    ),
    (
        "GbaObjectFrames",
        r#"GbaObjectFrames(tiles: "", bank: "", objects: "", frames: (lists: (buffer: "", offset: 0x0, stride: 4, count: 1, field: (at: 0))), into: "new_sprite")"#,
    ),
    ("MergeSprite", r#"MergeSprite(from: "", into: "")"#),
    (
        "KeyframeAnimation",
        r#"KeyframeAnimation(from: "", name: "new_animation", at: Offset(0x0), layout: (stride: 4, frame: (at: 0), ticks: (at: 1), end: (at: 2)), fps: 60.0, into: "")"#,
    ),
];

pub const OUTPUT_TEMPLATES: &[(&str, &str)] = &[
    ("Image", r#"Image(id: "new_image", from: "")"#),
    (
        "Surfaces",
        r#"Surfaces(id: "new_surfaces", width: 1, cell_size: 16, grids: [])"#,
    ),
    (
        "Model",
        r#"Model(id: "new_model", from: "", meters_per_unit: 0.01)"#,
    ),
    (
        "Sprite",
        r#"Sprite(id: "new_sprite", from: "", pixels_per_meter: 16.0)"#,
    ),
    ("Items", r#"Items(id: "new_items", items: [])"#),
    (
        "Ability",
        r#"Ability(id: "new_ability", name: "New ability", from: "", actions: [])"#,
    ),
];

/// The kinds a template's step or output reads, and the kinds it makes.
fn template_kinds(template: &str, step: bool) -> (Vec<NameKind>, Vec<NameKind>) {
    let ports: Vec<(NameKind, Access)> = if step {
        ron::from_str::<Step>(template)
            .map(|s| s.ports().iter().map(|p| (p.kind, p.access)).collect())
            .unwrap_or_default()
    } else {
        ron::from_str::<Output>(template)
            .map(|o| o.ports().iter().map(|p| (p.kind, p.access)).collect())
            .unwrap_or_default()
    };
    (
        ports
            .iter()
            .filter(|(_, a)| a.reads())
            .map(|(k, _)| *k)
            .collect(),
        ports
            .iter()
            .filter(|(_, a)| a.writes())
            .map(|(k, _)| *k)
            .collect(),
    )
}

/// A recipe change made by name, for scripts (the editor's remote control). Nodes are written
/// `"step 3"` or `"output 0"`.
#[derive(Debug, Clone, Deserialize)]
pub enum Edit {
    /// Sets a value of a node (by field name) to RON text: `Value("step 0", "offset", "0x40")`.
    Value(String, String, String),
    /// Wires what a node makes (by name) into another node's input (by its label):
    /// `Wire("step 0", "tiles", "step 4", "metatiles")`.
    Wire(String, String, String, String),
    /// Adds a step or output from its template, by variant name: `Add("Crop")`.
    Add(String),
    Remove(String),
    /// Runs a step later (`true`) or earlier; moves an output down or up.
    Move(String, bool),
    Duplicate(String),
}

fn node_named(text: &str) -> Result<RNode, String> {
    let (kind, index) = text
        .trim()
        .split_once(' ')
        .ok_or_else(|| format!("write a node as \"step 3\" or \"output 0\", not \"{text}\""))?;
    let index: usize = index
        .trim()
        .parse()
        .map_err(|_| format!("\"{index}\" isn't a number"))?;
    match kind {
        "step" => Ok(RNode::Step(index)),
        "output" => Ok(RNode::Output(index)),
        _ => Err(format!("\"{kind}\" isn't step or output")),
    }
}

/// A change asked for while drawing, made after.
#[derive(Clone, Debug)]
enum Change {
    /// New text for a value.
    Replace {
        span: Range<usize>,
        text: String,
        /// Changes to the same field one after another are undone together.
        field: Option<(RNode, String)>,
    },
    /// Wire an output into an input: the input uses the output's name.
    Wire {
        from: (RNode, usize),
        to: (RNode, usize),
    },
    /// Sets the name an input uses (`""` for none).
    Rename { at: (RNode, usize), name: String },
    /// Renames what a step makes, and every input that reads it.
    RenameMade { at: (RNode, usize), name: String },
    /// Writes a field that was left out (so it had its default).
    AddField {
        node: RNode,
        key: String,
        value: String,
    },
    Add {
        template: String,
        step: bool,
        at: Pos2,
        /// Its place in its list; the end without one.
        index: Option<usize>,
        /// Its first input of this kind reads this name.
        read: Option<(NameKind, String)>,
        /// Its first output of the input's kind feeds this input.
        feed: Option<(RNode, usize)>,
    },
    /// Removes these steps and outputs.
    Remove(Vec<RNode>),
    /// Swaps a step or output with the one after it (`later`) or before it.
    Move { node: RNode, later: bool },
    /// Moves a step or output to this place in its list.
    MoveTo { node: RNode, to: usize },
    /// Adds a copy after it, with the names it makes renamed so nothing else changes.
    Duplicate(RNode),
}

/// Where nodes are drawn and whether they are open, by node.
type Places = HashMap<RNode, (Pos2, bool)>;

/// Places with nodes renumbered; `None` drops a node.
fn remap(places: Places, f: impl Fn(RNode) -> Option<RNode>) -> Places {
    places
        .into_iter()
        .filter_map(|(n, p)| Some((f(n)?, p)))
        .collect()
}

/// What `node` is called once `new` is put in before it in the same list.
fn shifted(node: RNode, new: RNode) -> RNode {
    match (node, new) {
        (RNode::Step(i), RNode::Step(at)) if i >= at => RNode::Step(i + 1),
        (RNode::Output(i), RNode::Output(at)) if i >= at => RNode::Output(i + 1),
        _ => node,
    }
}

/// The node at place `i` of `node`'s list.
fn same_list(node: RNode, i: usize) -> RNode {
    match node {
        RNode::Step(_) => RNode::Step(i),
        RNode::Output(_) => RNode::Output(i),
    }
}

fn index(node: RNode) -> usize {
    match node {
        RNode::Step(i) | RNode::Output(i) => i,
    }
}

pub struct RecipeGraph {
    doc: Doc,
    recipe: Recipe,
    saved_text: String,
    ports: HashMap<RNode, Vec<OwnedPort>>,
    resolution: Resolution,
    snarl: Snarl<RNode>,
    rom: Option<Arc<Vec<u8>>>,
    trace: Trace,
    trace_dirty: bool,
    /// A run in the background, started after an edit, and which run it is.
    trace_job: Option<(u64, std::thread::JoinHandle<Trace>)>,
    /// Counts runs, so a background run that finishes after a newer one is dropped.
    trace_runs: u64,
    /// Bumped with every trace, so previews are made again.
    revision: u64,
    edits: HashMap<(RNode, String), String>,
    thumbs: HashMap<String, (u64, TextureHandle)>,
    undo: Vec<String>,
    redo: Vec<String>,
    last_field: Option<(RNode, String)>,
    /// Why the last change wasn't made, or what went wrong running the recipe.
    pub message: Option<String>,
    sizes: HashMap<NodeId, Vec2>,
    arrange_pending: bool,
    /// Finding nodes and moving the view.
    pub nav: Navigator,
    /// Every plugin's assets, where the recipe's characters find the parts they name.
    pub assets: Arc<clockwork_recipe::assets::OwnedAssets>,
    /// The plugin's name, for the parts it names of itself.
    pub plugin: String,
    /// The node the Inspector shows: the one last selected, added or moved.
    focus: Option<RNode>,
    /// The nodes selected as last drawn, to tell when the selection changes.
    selection: Vec<NodeId>,
    /// What is typed in the add menus.
    add_query: String,
    /// Where the add menu opened with Space is, on screen and in the graph.
    add_at: Option<(Pos2, Pos2)>,
    /// The problem the toolbar went to last.
    problem: usize,
}

/// What happened while the graph was drawn.
pub struct Shown {
    /// The nodes were arranged by themselves (a recipe without a saved layout).
    pub arranged: bool,
}

impl RecipeGraph {
    /// The graph of a recipe's text, with nodes where `layout` has them. `rom` (the plugin's
    /// game, canonical) lets nodes show what their step makes.
    pub fn new(
        text: String,
        layout: &RecipeLayout,
        rom: Option<Arc<Vec<u8>>>,
    ) -> Result<Self, String> {
        let (doc, recipe) = parse(&text)?;
        let mut places = Places::new();
        for (i, step) in recipe.steps.iter().enumerate() {
            if let Some((name, p)) = layout.steps.get(i)
                && name == step.name()
            {
                places.insert(RNode::Step(i), (pos2(p.0, p.1), true));
            }
        }
        for (i, output) in recipe.outputs.iter().enumerate() {
            if let Some((name, p)) = layout.outputs.get(i)
                && name == output.name()
            {
                places.insert(RNode::Output(i), (pos2(p.0, p.1), true));
            }
        }
        let complete = places.len() == recipe.steps.len() + recipe.outputs.len();
        let mut graph = Self {
            doc,
            recipe,
            saved_text: text,
            ports: HashMap::new(),
            resolution: Resolution::default(),
            snarl: Snarl::new(),
            rom,
            assets: Default::default(),
            plugin: String::new(),
            trace: Trace::default(),
            trace_dirty: true,
            trace_job: None,
            trace_runs: 0,
            revision: 0,
            edits: HashMap::new(),
            thumbs: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_field: None,
            message: None,
            sizes: HashMap::new(),
            arrange_pending: !complete,
            nav: Navigator::default(),
            focus: None,
            selection: Vec::new(),
            add_query: String::new(),
            add_at: None,
            problem: 0,
        };
        graph.nav.open_view();
        graph.refresh(places);
        graph.run_trace();
        Ok(graph)
    }

    pub fn load(dir: &Path, rom: Option<Arc<Vec<u8>>>) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join("recipe.ron")).map_err(|e| e.to_string())?;
        Self::new(text, &load_layout(dir), rom)
    }

    pub fn text(&self) -> &str {
        &self.doc.text
    }

    /// Whether the text differs from what was loaded or last saved.
    pub fn text_changed(&self) -> bool {
        self.doc.text != self.saved_text
    }

    /// The text as loaded or last saved.
    pub fn saved_text(&self) -> &str {
        &self.saved_text
    }

    pub fn mark_saved(&mut self) {
        self.saved_text = self.doc.text.clone();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(text) = self.undo.pop() {
            let current = self.doc.text.clone();
            if self.set_text(&text, self.places()).is_ok() {
                self.redo.push(current);
            }
        }
        self.last_field = None;
    }

    pub fn redo(&mut self) {
        if let Some(text) = self.redo.pop() {
            let current = self.doc.text.clone();
            if self.set_text(&text, self.places()).is_ok() {
                self.undo.push(current);
            }
        }
        self.last_field = None;
    }

    /// How many steps and outputs the recipe has, and the error of its last run, if any.
    pub fn status(&self) -> (usize, usize, Option<&str>) {
        (
            self.recipe.steps.len(),
            self.recipe.outputs.len(),
            self.trace.error.as_deref(),
        )
    }

    /// Where the nodes sit, to save next to the recipe.
    pub fn layout(&self) -> RecipeLayout {
        let places = self.places();
        let at = |node: RNode| {
            places
                .get(&node)
                .map_or((0.0, 0.0), |(p, _)| (p.x.round(), p.y.round()))
        };
        RecipeLayout {
            steps: self
                .recipe
                .steps
                .iter()
                .enumerate()
                .map(|(i, s)| (s.name().to_string(), at(RNode::Step(i))))
                .collect(),
            outputs: self
                .recipe
                .outputs
                .iter()
                .enumerate()
                .map(|(i, o)| (o.name().to_string(), at(RNode::Output(i))))
                .collect(),
        }
    }

    /// Where every node is drawn, and whether it is open.
    fn places(&self) -> Places {
        self.snarl
            .nodes_info()
            .map(|n| (n.value, (n.pos, n.open)))
            .collect()
    }

    /// Keeps a new text (already parsed) and rebuilds the graph from it.
    fn set_text(&mut self, text: &str, places: Places) -> Result<(), String> {
        let (doc, recipe) = parse(text)?;
        self.doc = doc;
        self.recipe = recipe;
        self.refresh(places);
        if self
            .focus
            .is_some_and(|node| !self.ports.contains_key(&node))
        {
            self.focus = None;
        }
        self.trace_dirty = true;
        Ok(())
    }

    fn refresh(&mut self, places: Places) {
        self.ports = (0..self.recipe.steps.len())
            .map(RNode::Step)
            .chain((0..self.recipe.outputs.len()).map(RNode::Output))
            .map(|node| (node, ports_of(&self.recipe, node)))
            .collect();
        self.resolution = resolve(&self.recipe, &self.ports);
        let depth = depths(&self.recipe, &self.resolution, &self.ports);
        let mut snarl = Snarl::new();
        let mut ids: HashMap<RNode, NodeId> = HashMap::new();
        let last = depth.values().copied().max().unwrap_or(0) + 1;
        for (row, node) in (0..self.recipe.steps.len())
            .map(RNode::Step)
            .chain((0..self.recipe.outputs.len()).map(RNode::Output))
            .enumerate()
        {
            let column = match node {
                RNode::Step(_) => depth.get(&node).copied().unwrap_or(0),
                RNode::Output(_) => last,
            };
            let (pos, open) = places
                .get(&node)
                .copied()
                .unwrap_or((pos2(column as f32 * COLUMN, row as f32 * ROW * 0.5), true));
            let id = snarl.insert_node(pos, node);
            if !open {
                snarl.open_node(id, false);
            }
            ids.insert(node, id);
        }
        for ((from, from_port), (to, to_port)) in &self.resolution.wires {
            let output = outputs_of(&self.ports[from])
                .iter()
                .position(|p| p == from_port);
            let input = inputs_of(&self.ports[to]).iter().position(|p| p == to_port);
            if let (Some(output), Some(input)) = (output, input) {
                snarl.connect(
                    OutPinId {
                        node: ids[from],
                        output,
                    },
                    InPinId {
                        node: ids[to],
                        input,
                    },
                );
            }
        }
        self.snarl = snarl;
        self.sizes.clear();
    }

    /// Runs the recipe on the ROM and keeps what every step made. Used where the result is
    /// needed right away (opening a plugin, scripted edits); edits in the graph run it in the
    /// background instead (see [`RecipeGraph::start_trace`]).
    fn run_trace(&mut self) {
        self.trace_dirty = false;
        self.trace_runs += 1;
        self.trace_job = None;
        self.trace = trace(
            &self.recipe,
            self.rom.as_deref().map(Vec::as_slice),
            &self.assets,
            &self.plugin,
            &self.ports,
        );
        self.revision += 1;
    }

    /// Starts running the recipe in the background; the nodes keep showing what the last run
    /// made until it is done, so a big recipe never holds up the editor.
    fn start_trace(&mut self) {
        self.trace_dirty = false;
        self.trace_runs += 1;
        let recipe = self.recipe.clone();
        let rom = self.rom.clone();
        let assets = self.assets.clone();
        let plugin = self.plugin.clone();
        let ports = self.ports.clone();
        let started = std::thread::Builder::new()
            .name("recipe trace".into())
            .spawn(move || {
                trace(
                    &recipe,
                    rom.as_deref().map(Vec::as_slice),
                    &assets,
                    &plugin,
                    &ports,
                )
            });
        match started {
            Ok(job) => self.trace_job = Some((self.trace_runs, job)),
            Err(_) => self.run_trace(),
        }
    }

    /// Keeps the background run's result once it is done, unless a newer run started since.
    fn poll_trace(&mut self) {
        if !self
            .trace_job
            .as_ref()
            .is_some_and(|(_, job)| job.is_finished())
        {
            return;
        }
        let Some((run, job)) = self.trace_job.take() else {
            return;
        };
        match job.join() {
            Ok(made) if run == self.trace_runs => {
                self.trace = made;
                self.revision += 1;
            }
            Ok(_) => {}
            Err(_) => {
                self.trace = Trace {
                    error: Some("Running the recipe failed unexpectedly".into()),
                    ..Trace::default()
                };
                self.revision += 1;
            }
        }
    }

    /// Whether the recipe is running again after a change.
    pub fn running(&self) -> bool {
        self.trace_job.is_some()
    }

    /// The text of the recipe's node for a step or output.
    fn node_doc(doc: &Doc, node: RNode) -> Option<&Node> {
        let (list, i) = match node {
            RNode::Step(i) => ("steps", i),
            RNode::Output(i) => ("outputs", i),
        };
        doc.root.field(list)?.items().get(i)
    }

    /// Makes a change; on success it can be undone.
    fn apply(&mut self, change: Change) -> Result<(), String> {
        let mut places = self.places();
        let field = match &change {
            Change::Replace { field, .. } => field.clone(),
            _ => None,
        };
        let mut focus = self.focus;
        let text = match change {
            Change::Replace { span, text, .. } => self.doc.replaced(span, &text),
            Change::Wire { from, to } => {
                let port = |(node, i): (RNode, usize)| {
                    self.ports
                        .get(&node)
                        .and_then(|ports| ports.get(i))
                        .ok_or("no such input or output")
                };
                let (from_port, to_port) = (port(from)?, port(to)?);
                if from_port.kind != to_port.kind {
                    return Err(format!(
                        "A {} can't go into a {} input",
                        from_port.kind.label(),
                        to_port.kind.label()
                    ));
                }
                let text = self.renamed(&self.doc, to, &from_port.name)?;
                // Steps run in order: the input must read what this output makes, not something
                // made under the same name later or earlier.
                let (_, recipe) = parse(&text)?;
                let ports = all_ports(&recipe);
                if !resolve(&recipe, &ports).wires.contains(&(from, to)) {
                    return Err(match (from.0, to.0) {
                        (RNode::Step(a), RNode::Step(b)) if a >= b => format!(
                            "Step {b} runs before step {a}, so it can't use what step {a} makes: \
                             move step {b} later first (Alt+↓, or its place in the Inspector)"
                        ),
                        _ => format!(
                            "Another step makes \"{}\" in between, so the input would read that",
                            from_port.name
                        ),
                    });
                }
                text
            }
            Change::Rename { at, name } => self.renamed(&self.doc, at, &name)?,
            Change::RenameMade { at, name } => {
                // Every input that reads it, and what reads the inputs that change it.
                let mut targets = vec![at];
                let mut i = 0;
                while i < targets.len() {
                    for (from, to) in &self.resolution.wires {
                        if *from == targets[i] && !targets.contains(to) {
                            targets.push(*to);
                        }
                    }
                    i += 1;
                }
                let mut doc = self.doc.clone();
                for (node, port) in targets {
                    let text = self.renamed_with(&doc, &self.ports[&node], node, port, &name)?;
                    doc = Doc::parse(text).map_err(|e| e.to_string())?;
                }
                doc.text
            }
            Change::AddField { node, key, value } => {
                let item = Self::node_doc(&self.doc, node).ok_or("no such node")?;
                self.doc
                    .field_added(item, &key, &value)
                    .ok_or("it isn't written as fields")?
            }
            Change::Add {
                template,
                step,
                at,
                index,
                read,
                feed,
            } => {
                let list = if step { "steps" } else { "outputs" };
                let seq = self
                    .doc
                    .root
                    .field(list)
                    .ok_or(format!("the recipe has no {list} list"))?;
                let len = if step {
                    self.recipe.steps.len()
                } else {
                    self.recipe.outputs.len()
                };
                let place = index.unwrap_or(len).min(len);
                let item = self.unique_names(&template);
                let text = self.doc.inserted(seq, place, &item);
                let node = if step {
                    RNode::Step(place)
                } else {
                    RNode::Output(place)
                };
                places = remap(places, |n| Some(shifted(n, node)));
                places.insert(node, (at, true));
                let (mut doc, mut recipe) = parse(&text)?;
                if let Some((kind, name)) = read {
                    let ports = ports_of(&recipe, node);
                    if let Some(i) = ports
                        .iter()
                        .position(|p| p.kind == kind && p.access.reads())
                    {
                        let text = self.renamed_with(&doc, &ports, node, i, &name)?;
                        (doc, recipe) = parse(&text)?;
                    }
                }
                if let Some(to) = feed {
                    let kind = self.ports[&to.0][to.1].kind;
                    let fed = shifted(to.0, node);
                    let ports = ports_of(&recipe, node);
                    if let Some(made) = ports.iter().find(|p| p.kind == kind && p.access.writes()) {
                        let to_ports = ports_of(&recipe, fed);
                        let text = self.renamed_with(&doc, &to_ports, fed, to.1, &made.name)?;
                        (doc, _) = parse(&text)?;
                    }
                }
                focus = Some(node);
                doc.text
            }
            Change::Remove(mut nodes) => {
                // Last first, so the places of the others don't change.
                nodes.sort_by_key(|n| std::cmp::Reverse(order(*n)));
                nodes.dedup();
                let mut doc = self.doc.clone();
                for node in nodes {
                    let item = Self::node_doc(&doc, node).ok_or("no such node")?;
                    doc = Doc::parse(doc.removed(item)).map_err(|e| e.to_string())?;
                    places = remap(places, |n| match (n, node) {
                        _ if n == node => None,
                        (RNode::Step(i), RNode::Step(r)) if i > r => Some(RNode::Step(i - 1)),
                        (RNode::Output(i), RNode::Output(r)) if i > r => Some(RNode::Output(i - 1)),
                        _ => Some(n),
                    });
                }
                doc.text
            }
            Change::Move { node, later } => {
                let len = self.list_len(node);
                let to = match (index(node), later) {
                    (i, true) if i + 1 < len => i + 1,
                    (i, false) if i > 0 => i - 1,
                    (_, true) => return Err("It's already last".into()),
                    (_, false) => return Err("It's already first".into()),
                };
                let text = self.moved(node, to, &mut places)?;
                focus = Some(same_list(node, to));
                text
            }
            Change::MoveTo { node, to } => {
                let to = to.min(self.list_len(node).saturating_sub(1));
                let text = self.moved(node, to, &mut places)?;
                focus = Some(same_list(node, to));
                text
            }
            Change::Duplicate(node) => {
                let item = Self::node_doc(&self.doc, node).ok_or("no such node")?;
                let list = match node {
                    RNode::Step(_) => "steps",
                    RNode::Output(_) => "outputs",
                };
                let seq = self.doc.root.field(list).ok_or("no such list")?;
                let copy = same_list(node, index(node) + 1);
                let mut doc =
                    Doc::parse(self.doc.inserted(seq, index(copy), self.doc.text_of(item)))
                        .map_err(|e| e.to_string())?;
                let mut taken = self.taken_names();
                let fresh = |name: &str, taken: &mut HashSet<String>| {
                    let name = (2..)
                        .map(|n| format!("{name}_{n}"))
                        .find(|n| !taken.contains(n))
                        .unwrap_or_default();
                    taken.insert(name.clone());
                    name
                };
                if let RNode::Step(_) = node {
                    // What the copy makes gets new names; what it changes stays.
                    let (_, recipe) = parse(&doc.text)?;
                    let ports = ports_of(&recipe, copy);
                    for (i, port) in ports.iter().enumerate() {
                        if port.access == Access::Write {
                            let name = fresh(&port.name, &mut taken);
                            let text = self.renamed_with(&doc, &ports, copy, i, &name)?;
                            doc = Doc::parse(text).map_err(|e| e.to_string())?;
                        }
                    }
                } else if let Some(id) = Self::node_doc(&doc, copy).and_then(|n| n.field("id")) {
                    let current = parse_string(doc.text_of(id)).unwrap_or_default();
                    let name = fresh(&current, &mut taken);
                    doc = Doc::parse(doc.replaced(id.span.clone(), &string_literal(&name)))
                        .map_err(|e| e.to_string())?;
                }
                let original = places.get(&node).copied();
                places = remap(places, |n| Some(shifted(n, copy)));
                if let Some((p, open)) = original {
                    places.insert(copy, (p + vec2(40.0, 40.0), open));
                }
                focus = Some(copy);
                doc.text
            }
        };
        let before = self.doc.text.clone();
        self.set_text(&text, places)?;
        self.focus = focus.filter(|node| self.ports.contains_key(node));
        let merge = field.is_some() && field == self.last_field;
        if !merge {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_field = field;
        Ok(())
    }

    fn list_len(&self, node: RNode) -> usize {
        match node {
            RNode::Step(_) => self.recipe.steps.len(),
            RNode::Output(_) => self.recipe.outputs.len(),
        }
    }

    /// The text with a step or output moved to place `to` of its list, one swap with a
    /// neighbour at a time so comments on their lines go with them; `places` follow.
    fn moved(&self, node: RNode, to: usize, places: &mut Places) -> Result<String, String> {
        let from = index(node);
        let mut doc = self.doc.clone();
        let mut at = from;
        while at != to {
            let next = if to > at { at + 1 } else { at - 1 };
            let (first, second) = (same_list(node, at.min(next)), same_list(node, at.max(next)));
            let a = Self::node_doc(&doc, first).ok_or("no such node")?;
            let b = Self::node_doc(&doc, second).ok_or("no such node")?;
            let text = doc
                .swapped(a, b)
                .ok_or("It shares a line with another step in recipe.ron, so it can't move")?;
            doc = Doc::parse(text).map_err(|e| e.to_string())?;
            at = next;
        }
        // The nodes stay where they are drawn.
        *places = remap(std::mem::take(places), |n| {
            let i = match (n, node) {
                (RNode::Step(i), RNode::Step(_)) | (RNode::Output(i), RNode::Output(_)) => i,
                _ => return Some(n),
            };
            Some(same_list(
                n,
                if i == from {
                    to
                } else if from < to && i > from && i <= to {
                    i - 1
                } else if to < from && i >= to && i < from {
                    i + 1
                } else {
                    i
                },
            ))
        });
        Ok(doc.text)
    }

    /// The text with a node's port renamed.
    fn renamed(&self, doc: &Doc, at: (RNode, usize), name: &str) -> Result<String, String> {
        self.renamed_with(doc, &self.ports[&at.0], at.0, at.1, name)
    }

    fn renamed_with(
        &self,
        doc: &Doc,
        ports: &[OwnedPort],
        node: RNode,
        port: usize,
        name: &str,
    ) -> Result<String, String> {
        let item = Self::node_doc(doc, node).ok_or("no such node")?;
        let path = &ports.get(port).ok_or("no such input or output")?.path;
        let value = item
            .at(path, &doc.text)
            .ok_or("can't find where that name is written")?;
        Ok(doc.replaced(value.span.clone(), &string_literal(name)))
    }

    /// Every name the recipe uses, and its output ids.
    fn taken_names(&self) -> HashSet<String> {
        self.ports
            .values()
            .flatten()
            .map(|p| p.name.clone())
            .chain(self.recipe.outputs.iter().map(|o| o.id().to_string()))
            .collect()
    }

    /// A template with its `new_…` names made unique in this recipe.
    fn unique_names(&self, template: &str) -> String {
        let taken = self.taken_names();
        let mut text = template.to_string();
        for placeholder in [
            "new_buffer",
            "new_image",
            "new_model",
            "new_collision",
            "new_sprite",
            "new_animation",
            "new_surfaces",
            "new_items",
            "new_ability",
        ] {
            if !text.contains(placeholder) {
                continue;
            }
            let base = placeholder.trim_start_matches("new_");
            let name = (1..)
                .map(|n| format!("{base}_{n}"))
                .find(|n| !taken.contains(n))
                .unwrap_or_default();
            text = text.replace(placeholder, &name);
        }
        text
    }

    /// Makes a change written by name (see [`Edit`]) and runs the recipe again.
    pub fn edit(&mut self, edit: &Edit) -> Result<(), String> {
        let change = match edit {
            Edit::Value(node, field, text) => {
                let node = node_named(node)?;
                let item = Self::node_doc(&self.doc, node).ok_or("no such node")?;
                let value = item
                    .field(field)
                    .ok_or_else(|| format!("it has no {field}"))?;
                Change::Replace {
                    span: value.span.clone(),
                    text: text.clone(),
                    field: None,
                }
            }
            Edit::Wire(from, name, to, input) => {
                let (from, to) = (node_named(from)?, node_named(to)?);
                let out = self.ports.get(&from).and_then(|ports| {
                    ports
                        .iter()
                        .position(|p| p.access.writes() && p.name == *name)
                });
                let inp = self.ports.get(&to).and_then(|ports| {
                    ports
                        .iter()
                        .position(|p| p.access.reads() && p.label() == *input)
                });
                let out = out.ok_or_else(|| format!("it makes nothing called {name}"))?;
                let inp = inp.ok_or_else(|| format!("it has no input called {input}"))?;
                Change::Wire {
                    from: (from, out),
                    to: (to, inp),
                }
            }
            Edit::Add(variant) => {
                let (template, step) = STEP_TEMPLATES
                    .iter()
                    .find(|(n, _)| n == variant)
                    .map(|(_, t)| (*t, true))
                    .or_else(|| {
                        OUTPUT_TEMPLATES
                            .iter()
                            .find(|(n, _)| n == variant)
                            .map(|(_, t)| (*t, false))
                    })
                    .ok_or_else(|| format!("there is no step or output called {variant}"))?;
                // Below everything drawn.
                let below = self
                    .snarl
                    .nodes_pos_ids()
                    .map(|(id, pos, _)| pos.y + self.sizes.get(&id).map_or(ROW, |size| size.y))
                    .fold(0.0, f32::max);
                Change::Add {
                    template: template.to_string(),
                    step,
                    at: pos2(0.0, below + GAP),
                    index: None,
                    read: None,
                    feed: None,
                }
            }
            Edit::Remove(node) => Change::Remove(vec![node_named(node)?]),
            Edit::Move(node, later) => Change::Move {
                node: node_named(node)?,
                later: *later,
            },
            Edit::Duplicate(node) => Change::Duplicate(node_named(node)?),
        };
        self.apply(change)?;
        self.run_trace();
        Ok(())
    }

    /// The recipe's size, wiring and last run, for reports.
    pub fn describe(&self) -> String {
        let made = self.trace.steps.iter().filter(|s| s.is_some()).count();
        format!(
            "recipe, {} steps ({} ran), {} outputs, {} wires, {} inputs missing, error {:?}",
            self.recipe.steps.len(),
            made,
            self.recipe.outputs.len(),
            self.resolution.wires.len(),
            self.resolution.missing.len(),
            self.trace.error
        )
    }

    /// What keeps the recipe from running or being whole: the error of its last run, then every
    /// input that reads nothing, with the node each is on.
    pub fn problems(&self) -> Vec<(Option<RNode>, String)> {
        let mut problems = Vec::new();
        if let Some(error) = &self.trace.error {
            let node = self
                .trace
                .error_step
                .map(RNode::Step)
                .or(self.trace.error_output.map(RNode::Output));
            let at = node.map_or(String::new(), |n| format!("{}: ", title(&self.doc, n)));
            problems.push((node, format!("{at}{error}")));
        }
        let mut missing: Vec<(RNode, usize)> = self.resolution.missing.iter().copied().collect();
        missing.sort_by_key(|(n, p)| (order(*n), *p));
        for (node, port) in missing {
            let port = &self.ports[&node][port];
            let what = if port.name.is_empty() {
                format!("its {} input reads no {}", port.label(), port.kind.label())
            } else {
                format!(
                    "its {} input reads the {} \"{}\", which no earlier step makes",
                    port.label(),
                    port.kind.label(),
                    port.name
                )
            };
            problems.push((Some(node), format!("{}: {what}", title(&self.doc, node))));
        }
        problems
    }

    /// Selects the next problem's node and moves the view to it.
    pub fn next_problem(&mut self) {
        let problems = self.problems();
        if problems.is_empty() {
            return;
        }
        self.problem = (self.problem + 1) % problems.len();
        if let Some(node) = problems[self.problem].0 {
            self.go_to(node);
        }
    }

    /// Shows a node in the Inspector and moves the view to it.
    pub fn go_to(&mut self, node: RNode) {
        self.focus = Some(node);
        if let Some((id, pos, _)) = self.snarl.nodes_pos_ids().find(|(_, _, n)| **n == node) {
            let size = self.sizes.get(&id).copied().unwrap_or(vec2(260.0, 120.0));
            self.nav.show(Rect::from_min_size(pos, size));
        }
    }

    /// Shows a node written as `"step 3"` or `"output 0"` in the Inspector, for scripts.
    pub fn inspect(&mut self, node: &str) -> Result<(), String> {
        let node = node_named(node)?;
        if !self.ports.contains_key(&node) {
            return Err("no such node".into());
        }
        self.go_to(node);
        Ok(())
    }

    /// Where a new step (or output) goes: after the one in the Inspector, else at the end.
    fn insert_index(&self, step: bool) -> Option<usize> {
        match (self.focus?, step) {
            (RNode::Step(i), true) | (RNode::Output(i), false) => Some(i + 1),
            _ => None,
        }
    }

    /// Draws the graph for editing, with the Inspector beside it, and makes the changes asked for.
    pub fn show(&mut self, ui: &mut Ui, id_salt: &str) -> Shown {
        let mut changes = Vec::new();
        egui::Panel::right(egui::Id::new(("recipe inspector", id_salt)))
            .resizable(true)
            .default_size(340.0)
            .size_range(260.0..=620.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| self.inspector(ui, &mut changes));
            });

        let doc = &self.doc;
        self.nav.update(
            &self.snarl,
            &self.sizes,
            ui.available_rect_before_wrap(),
            |_, node| {
                let text = Self::node_doc(doc, *node).map_or("", |n| doc.text_of(n));
                format!("{} {text}", title(doc, *node))
            },
        );
        let lineage = self.focus.map(|node| lineage(&self.resolution, node));
        let insert = (self.insert_index(true), self.insert_index(false));
        let mut viewer = Viewer {
            nav: &mut self.nav,
            doc: &self.doc,
            steps: self.recipe.steps.len(),
            ports: &self.ports,
            resolution: &self.resolution,
            trace: &self.trace,
            thumbs: &mut self.thumbs,
            revision: self.revision,
            focus: self.focus,
            lineage: lineage.as_ref(),
            add_query: &mut self.add_query,
            insert,
            changes: Vec::new(),
            sizes: HashMap::new(),
        };
        let widget = egui_snarl::ui::SnarlWidget::new()
            .id_salt(("recipe_graph", id_salt))
            .style(style())
            .min_size(ui.available_size());
        let response = widget.show(&mut self.snarl, &mut viewer, ui);
        changes.extend(std::mem::take(&mut viewer.changes));
        let sizes = std::mem::take(&mut viewer.sizes);

        // The Inspector follows the selection when it changes.
        let selected = widget.get_selected_nodes(ui);
        if selected != self.selection {
            self.focus = selected
                .last()
                .and_then(|id| self.snarl.get_node(*id))
                .copied();
            self.selection = selected;
            ui.ctx().request_repaint();
        }

        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            let delete = ui.input(|i| i.key_pressed(egui::Key::Delete));
            if delete {
                let selected: Vec<RNode> = self
                    .selection
                    .iter()
                    .filter_map(|id| self.snarl.get_node(*id).copied())
                    .collect();
                if !selected.is_empty() {
                    changes.insert(0, Change::Remove(selected));
                }
            }
            let hovered = ui.rect_contains_pointer(response.rect);
            if hovered
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
                && let Some(screen) = ui.input(|i| i.pointer.latest_pos())
            {
                self.add_at = Some((screen, self.nav.graph_pos(screen)));
                self.add_query.clear();
            }
            if let Some(node) = self.focus {
                let (earlier, later, duplicate, frame) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp),
                        i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown),
                        i.consume_key(egui::Modifiers::COMMAND, egui::Key::D),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::F),
                    )
                });
                if earlier || later {
                    changes.insert(0, Change::Move { node, later });
                }
                if duplicate {
                    changes.insert(0, Change::Duplicate(node));
                }
                if frame {
                    self.go_to(node);
                }
            }
        }
        if let Some((screen, at)) = self.add_at {
            let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let area = egui::Area::new(egui::Id::new(("recipe add", id_salt)))
                .order(egui::Order::Foreground)
                .fixed_pos(screen)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(320.0);
                        ui.label(kit::hint("Add a step or output"));
                        add_list(ui, &mut self.add_query, |_, _| true)
                    })
                });
            if let Some((template, step)) = area.inner.inner {
                changes.push(Change::Add {
                    template,
                    step,
                    at,
                    index: self.insert_index(step),
                    read: None,
                    feed: None,
                });
                close = true;
            }
            if close || area.response.clicked_elsewhere() {
                self.add_at = None;
            }
        }

        // Spans belong to the text as drawn: make the first change only.
        if let Some(change) = changes.into_iter().next() {
            match self.apply(change) {
                Ok(()) => self.message = None,
                Err(e) => self.message = Some(e),
            }
        }

        let settled = sizes == self.sizes
            && self
                .snarl
                .node_ids()
                .all(|(node, _)| self.sizes.contains_key(&node));
        self.sizes = sizes;
        let arranged = self.arrange_pending && settled;
        if arranged {
            self.arrange();
            self.arrange_pending = false;
            self.nav.open_view();
        }
        if self.arrange_pending {
            ui.ctx().request_repaint();
        }
        // Run the recipe again once a drag is over, in the background, one run at a time.
        self.poll_trace();
        if self.trace_dirty && self.trace_job.is_none() && !ui.input(|i| i.pointer.any_down()) {
            self.start_trace();
        }
        if self.trace_job.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        Shown { arranged }
    }

    /// The node in focus, every value of it and what it made; with nothing in focus, how to edit
    /// and what is wrong.
    fn inspector(&mut self, ui: &mut Ui, changes: &mut Vec<Change>) {
        let Some(node) = self.focus else {
            self.overview(ui);
            return;
        };
        let Self {
            doc,
            recipe,
            ports,
            resolution,
            trace,
            edits,
            thumbs,
            revision,
            ..
        } = self;
        let Some(item) = Self::node_doc(doc, node) else {
            return;
        };
        let info = variant_info(node, item.name().unwrap_or_default());
        ui.label(
            RichText::new(title(doc, node))
                .font(theme::display(18.0))
                .color(theme::PARCHMENT),
        );
        if let Some(info) = info {
            ui.label(kit::dim(info.about));
        }
        ui.add_space(6.0);

        // Where it runs, and moving it.
        let len = match node {
            RNode::Step(_) => recipe.steps.len(),
            RNode::Output(_) => recipe.outputs.len(),
        };
        ui.horizontal(|ui| {
            ui.label(kit::dim(match node {
                RNode::Step(_) => "Runs at place",
                RNode::Output(_) => "Output",
            }));
            // Typed or dragged; moved once the drag or the typing is over.
            let pending = egui::Id::new(("recipe place", node_key(node)));
            let mut place = ui
                .data(|d| d.get_temp::<usize>(pending))
                .unwrap_or(index(node));
            let response =
                ui.add(egui::DragValue::new(&mut place).range(0..=len.saturating_sub(1)));
            ui.label(kit::dim(format!("of {len}")));
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(pending, place));
            }
            let done = response.drag_stopped()
                || response.lost_focus()
                || (response.changed() && !response.dragged() && !response.has_focus());
            if done {
                ui.data_mut(|d| d.remove::<usize>(pending));
                if place != index(node) {
                    changes.push(Change::MoveTo { node, to: place });
                }
            }
            let (earlier, later) = match node {
                RNode::Step(_) => ("Run earlier  ·  Alt+↑", "Run later  ·  Alt+↓"),
                RNode::Output(_) => ("Move up  ·  Alt+↑", "Move down  ·  Alt+↓"),
            };
            if kit::icon_button(ui, "⏶", earlier).clicked() {
                changes.push(Change::Move { node, later: false });
            }
            if kit::icon_button(ui, "⏷", later).clicked() {
                changes.push(Change::Move { node, later: true });
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Duplicate")
                .on_hover_text(kit::tip(
                    "A copy right after it, making new names",
                    "Ctrl+D",
                ))
                .clicked()
            {
                changes.push(Change::Duplicate(node));
            }
            if ui
                .button("Remove")
                .on_hover_text(kit::tip("Remove it from the recipe", "Delete"))
                .clicked()
            {
                changes.push(Change::Remove(vec![node]));
            }
        });

        let node_ports = &ports[&node];
        let reads: Vec<usize> = inputs_of(node_ports);
        if !reads.is_empty() {
            kit::section(ui, "Uses");
            for i in reads {
                let port = &node_ports[i];
                let missing = resolution.missing.contains(&(node, i));
                let changes_it = port.access != Access::Read;
                let label = if changes_it {
                    format!("{} ✎", port.label())
                } else {
                    port.label()
                };
                kit::prop(ui, &label, |ui| {
                    let shown = if port.name.is_empty() {
                        "— none —".to_string()
                    } else {
                        port.name.clone()
                    };
                    let text = if missing {
                        RichText::new(shown).color(theme::ERROR)
                    } else {
                        RichText::new(shown)
                    };
                    let names = candidates(ports, recipe.steps.len(), node, port.kind);
                    egui::ComboBox::from_id_salt(("recipe input", node_key(node), i))
                        .selected_text(text)
                        .width(ui.available_width().min(220.0))
                        .show_ui(ui, |ui| {
                            if let Some(name) = name_menu(ui, &names, &port.name, port.kind) {
                                changes.push(Change::Rename {
                                    at: (node, i),
                                    name,
                                });
                            }
                        })
                        .response
                        .on_hover_text(format!(
                            "{}{}",
                            port.kind.label(),
                            if changes_it {
                                ": it changes this one (✎), so later steps read the changed one"
                            } else {
                                ""
                            }
                        ));
                });
            }
        }

        let makes: Vec<usize> = (0..node_ports.len())
            .filter(|&i| node_ports[i].access == Access::Write)
            .collect();
        if !makes.is_empty() {
            kit::section(ui, "Makes");
            for i in makes {
                let port = &node_ports[i];
                let readers = resolution
                    .wires
                    .iter()
                    .filter(|(from, _)| *from == (node, i))
                    .count();
                let tip = format!(
                    "A {}; renaming it renames it in the {readers} input(s) that read it",
                    port.kind.label()
                );
                kit::prop(ui, &port.label(), |ui| {
                    ui.label(kit::dim(format!("→{readers}")))
                        .on_hover_text(&tip);
                    let key = (node, format!("→{i}"));
                    let new = text_field(ui, edits, &key, &port.name, false, |t| {
                        let t = t.trim();
                        (!t.is_empty()).then(|| t.to_string())
                    });
                    if let Some(name) = new.filter(|n| *n != port.name) {
                        changes.push(Change::RenameMade {
                            at: (node, i),
                            name,
                        });
                    }
                });
            }
        }

        // Every value but the names it uses and makes, by what each holds.
        let names: Vec<&'static str> = node_ports
            .iter()
            .filter_map(|p| match p.path.as_slice() {
                [Seg::Field(f)] => Some(*f),
                _ => None,
            })
            .collect();
        if let Kind::Struct { fields, .. } = &item.kind {
            kit::section(ui, "Values");
            let written: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
            for field in fields.iter().filter(|f| !names.contains(&f.key.as_str())) {
                let text = doc.text_of(&field.value);
                let about = info.and_then(|i| i.field(&field.key));
                let ty = about.map_or_else(|| guess_type(text), |f| f.ty);
                let key = (node, field.key.clone());
                if let Some(new) = value_row(ui, edits, &key, about, ty, text)
                    && new != text
                {
                    changes.push(Change::Replace {
                        span: field.value.span.clone(),
                        text: new,
                        field: Some(key),
                    });
                }
            }
            // Fields left out have their defaults; changing one writes it.
            for about in info.map_or(&[][..], |i| i.fields) {
                if !about.optional
                    || written.contains(&about.key)
                    || names.contains(&about.key)
                    || about.ty == FieldType::Name
                {
                    continue;
                }
                let default = about.ty.default_text();
                let key = (node, about.key.to_string());
                if let Some(new) = value_row(ui, edits, &key, Some(about), about.ty, default)
                    && new != default
                {
                    changes.push(Change::AddField {
                        node,
                        key: about.key.to_string(),
                        value: new,
                    });
                }
            }
        }

        kit::section(ui, "Result");
        let failed = match node {
            RNode::Step(i) => trace.error_step == Some(i),
            RNode::Output(i) => trace.error_output == Some(i),
        };
        if failed {
            let error = trace.error.as_deref().unwrap_or_default();
            ui.label(RichText::new(error).color(theme::ERROR));
            return;
        }
        let result = match node {
            RNode::Step(i) => trace.steps.get(i),
            RNode::Output(i) => trace.outputs.get(i),
        };
        let Some(Some(result)) = result else {
            ui.label(kit::dim(if trace.error.is_some() {
                "Not run: an earlier step failed"
            } else {
                "Nothing yet"
            }));
            return;
        };
        for line in &result.lines {
            ui.label(RichText::new(line).color(theme::PARCHMENT_DIM));
        }
        if let Some(image) = &result.image {
            let texture = thumb(ui.ctx(), thumbs, *revision, node, image);
            let side = image.width.max(image.height).max(1) as f32;
            let scale = (ui.available_width() / side).clamp(0.1, 4.0);
            ui.image((
                texture.id(),
                vec2(image.width as f32, image.height as f32) * scale,
            ));
        }
        if let Some(bytes) = &result.bytes {
            ui.add_space(4.0);
            ui.label(
                RichText::new(hex_rows(bytes))
                    .monospace()
                    .small()
                    .color(theme::PARCHMENT_DIM),
            );
        }
    }

    /// The Inspector with nothing selected: how to edit, then what is wrong.
    fn overview(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Nothing selected")
                .font(theme::display(18.0))
                .color(theme::PARCHMENT),
        );
        ui.label(kit::dim(
            "Click a step to see and change everything about it here.",
        ));
        kit::section(ui, "How to edit");
        for (keys, what) in [
            ("Space", "add a step after the selected one (or an output)"),
            ("Drag pin → pin", "use what a step makes in another step"),
            ("Drag pin → empty", "add a step already wired to it"),
            ("Click an input", "pick what it reads"),
            ("Alt+↑", "run the selected step earlier"),
            ("Alt+↓", "run it later"),
            ("Ctrl+D", "duplicate"),
            ("Delete", "remove the selected nodes"),
            ("F", "frame the selected node"),
            ("Ctrl+F", "find a node"),
            ("Ctrl+Z", "undo"),
            ("Ctrl+Y", "redo"),
        ] {
            ui.horizontal(|ui| {
                kit::kbd(ui, keys);
                ui.label(kit::dim(what));
            });
        }
        let problems = self.problems();
        kit::section(ui, &format!("Problems · {}", problems.len()));
        if problems.is_empty() {
            ui.label(RichText::new("None: every step ran").color(theme::SUCCESS));
        }
        let mut go = None;
        for (node, problem) in problems {
            let text = RichText::new(format!("⚠ {problem}")).color(theme::WARN);
            match node {
                Some(node) => {
                    if ui.link(text).clicked() {
                        go = Some(node);
                    }
                }
                None => {
                    ui.label(text);
                }
            }
        }
        if let Some(node) = go {
            self.go_to(node);
        }
    }

    /// Lays the nodes out in columns by how far each step is from the ROM, outputs last, each
    /// column in recipe order.
    pub fn arrange(&mut self) {
        let depth = depths(&self.recipe, &self.resolution, &self.ports);
        let last = depth.values().copied().max().unwrap_or(0) + 1;
        let column = |node: RNode| match node {
            RNode::Step(_) => depth.get(&node).copied().unwrap_or(0),
            RNode::Output(_) => last,
        };
        let size = |id: NodeId| self.sizes.get(&id).copied().unwrap_or(vec2(260.0, 120.0));
        let mut width = vec![0.0f32; last + 1];
        for (id, node) in self.snarl.node_ids() {
            width[column(*node)] = width[column(*node)].max(size(id).x);
        }
        let mut x = vec![0.0f32; last + 1];
        for c in 1..=last {
            x[c] = x[c - 1] + width[c - 1] + GAP * 2.0;
        }
        let mut order: Vec<(RNode, NodeId)> =
            self.snarl.node_ids().map(|(id, n)| (*n, id)).collect();
        order.sort_by_key(|(n, _)| match n {
            RNode::Step(i) => (0, *i),
            RNode::Output(i) => (1, *i),
        });
        let mut bottom = vec![0.0f32; last + 1];
        let mut placed = Vec::new();
        for (node, id) in order {
            let c = column(node);
            placed.push((id, pos2(x[c], bottom[c])));
            bottom[c] += size(id).y + GAP;
        }
        for (id, pos) in placed {
            if let Some(info) = self.snarl.get_node_info_mut(id) {
                info.pos = pos;
            }
        }
    }
}

/// Runs a recipe on the ROM and keeps what every step made.
fn trace(
    recipe: &Recipe,
    rom: Option<&[u8]>,
    assets: &clockwork_recipe::assets::OwnedAssets,
    plugin: &str,
    ports: &HashMap<RNode, Vec<OwnedPort>>,
) -> Trace {
    let Some(rom) = rom else {
        return Trace {
            error: Some("Its game isn't in the Library, so nothing can be shown".into()),
            ..Trace::default()
        };
    };
    let mut steps = vec![None; recipe.steps.len()];
    let assets = assets.with_recipe(plugin, recipe);
    let result = clockwork_recipe::run_observed(recipe, rom, &assets, |i, made| {
        steps[i] = Some(summarize(&ports[&RNode::Step(i)], made));
    });
    let (outputs, error, error_step, error_output) = match result {
        Ok(extracted) => (summarize_outputs(recipe, &extracted), None, None, None),
        Err(e) => {
            let output = e
                .output()
                .and_then(|id| recipe.outputs.iter().position(|o| o.id() == id));
            // An unknown name in an output names what it's made from, not its id.
            let output = output.or(match &e {
                clockwork_recipe::RecipeError::UnknownOutput(from) => recipe
                    .outputs
                    .iter()
                    .position(|o| o.ports().iter().any(|p| p.name == from.as_str())),
                _ => None,
            });
            (Vec::new(), Some(e.to_string()), e.step(), output)
        }
    };
    Trace {
        steps,
        outputs,
        error,
        error_step,
        error_output,
    }
}

/// Steps first, then outputs, each in recipe order.
fn order(node: RNode) -> (u8, usize) {
    match node {
        RNode::Step(i) => (0, i),
        RNode::Output(i) => (1, i),
    }
}

fn parse(text: &str) -> Result<(Doc, Recipe), String> {
    let doc = Doc::parse(text).map_err(|e| e.to_string())?;
    let recipe: Recipe = ron::from_str(text).map_err(|e| e.to_string())?;
    Ok((doc, recipe))
}

fn all_ports(recipe: &Recipe) -> HashMap<RNode, Vec<OwnedPort>> {
    (0..recipe.steps.len())
        .map(RNode::Step)
        .chain((0..recipe.outputs.len()).map(RNode::Output))
        .map(|node| (node, ports_of(recipe, node)))
        .collect()
}

/// Each step's column: one past the steps whose output it reads, and level with a step whose
/// output it only changes (a chain of steps adding to one model stays in one column).
fn depths(
    recipe: &Recipe,
    resolution: &Resolution,
    ports: &HashMap<RNode, Vec<OwnedPort>>,
) -> HashMap<RNode, usize> {
    let mut depth: HashMap<RNode, usize> = HashMap::new();
    for i in 0..recipe.steps.len() {
        let node = RNode::Step(i);
        let d = resolution
            .wires
            .iter()
            .filter(|(_, (to, _))| *to == node)
            .filter_map(|((from, _), (_, port))| {
                let updates = ports[&node][*port].access != Access::Read;
                depth.get(from).map(|d| if updates { *d } else { d + 1 })
            })
            .max()
            .unwrap_or(0);
        depth.insert(node, d);
    }
    depth
}

/// A node's title: `12 · Read`, or `Output · Image “sheet”`.
fn title(doc: &Doc, node: RNode) -> String {
    let item = RecipeGraph::node_doc(doc, node);
    match node {
        RNode::Step(i) => {
            let name = item.and_then(Node::name).unwrap_or("Step");
            format!("{i} · {name}")
        }
        RNode::Output(_) => {
            let name = item.and_then(Node::name).unwrap_or("Output");
            let id = item
                .and_then(|n| n.field("id"))
                .and_then(|id| parse_string(doc.text_of(id)))
                .unwrap_or_default();
            format!("Output · {name} “{id}”")
        }
    }
}

/// A key for a node's widgets.
fn node_key(node: RNode) -> String {
    format!("{node:?}")
}

fn variant_info(node: RNode, name: &str) -> Option<&'static VariantInfo> {
    match node {
        RNode::Step(_) => recipe_schema::step(name),
        RNode::Output(_) => recipe_schema::output(name),
    }
}

/// What a field holds that the schema doesn't know, from how it is written.
fn guess_type(text: &str) -> FieldType {
    if parse_int(text).is_some() && !text.contains('.') {
        FieldType::Count
    } else if text.parse::<f64>().is_ok() {
        FieldType::Number
    } else if text == "true" || text == "false" {
        FieldType::Flag
    } else if parse_string(text).is_some() {
        FieldType::Text
    } else {
        FieldType::Structure
    }
}

/// `node`, everything it is made from and everything made from it.
fn lineage(resolution: &Resolution, node: RNode) -> HashSet<RNode> {
    let mut set = HashSet::from([node]);
    for upstream in [true, false] {
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            for ((from, _), (to, _)) in &resolution.wires {
                let (here, next) = if upstream { (to, from) } else { (from, to) };
                if *here == at && set.insert(*next) {
                    stack.push(*next);
                }
            }
        }
    }
    set
}

/// The names of `kind` made before `node` runs, in the order they are first made.
fn candidates(
    ports: &HashMap<RNode, Vec<OwnedPort>>,
    steps: usize,
    node: RNode,
    kind: NameKind,
) -> Vec<String> {
    let before = match node {
        RNode::Step(i) => i,
        RNode::Output(_) => steps,
    };
    let mut names: Vec<String> = Vec::new();
    for i in 0..before {
        for port in ports.get(&RNode::Step(i)).into_iter().flatten() {
            if port.kind == kind
                && port.access.writes()
                && !port.name.is_empty()
                && !names.contains(&port.name)
            {
                names.push(port.name.clone());
            }
        }
    }
    names
}

/// A menu of the names an input can read, plus a field for any other name; the one picked.
fn name_menu(ui: &mut Ui, names: &[String], current: &str, kind: NameKind) -> Option<String> {
    let mut picked = None;
    if names.is_empty() {
        ui.label(kit::dim(format!(
            "No earlier step makes a {}",
            kind.label()
        )));
    }
    for name in names {
        if ui.selectable_label(name == current, name).clicked() {
            picked = Some(name.clone());
        }
    }
    ui.separator();
    if ui
        .selectable_label(current.is_empty(), "— none —")
        .clicked()
    {
        picked = Some(String::new());
    }
    let other = egui::Id::new(("other name", kind.label()));
    let mut text = ui.data(|d| d.get_temp::<String>(other)).unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .hint_text(kit::hint("Other name, Enter"))
            .desired_width(160.0),
    );
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(other, text.clone()));
    }
    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        ui.data_mut(|d| d.remove::<String>(other));
        if !text.trim().is_empty() {
            picked = Some(text.trim().to_string());
        }
    }
    if picked.is_some() {
        ui.close();
    }
    picked
}

/// A text field kept in `edits` while it is typed in, and given back once it is left with text
/// `accept` takes (its text as written then). Text it doesn't take is outlined in red.
fn text_field(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    text: &str,
    multiline: bool,
    accept: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut buffer = edits.get(key).cloned().unwrap_or_else(|| text.to_string());
    let ok = accept(&buffer).is_some();
    let editor = if multiline {
        egui::TextEdit::multiline(&mut buffer)
            .code_editor()
            .desired_rows(text.lines().count().clamp(2, 12))
            .desired_width(f32::INFINITY)
    } else {
        egui::TextEdit::singleline(&mut buffer).desired_width(ui.available_width().min(220.0))
    };
    let editor = if ok {
        editor
    } else {
        editor.background_color(theme::ERROR.gamma_multiply(0.25))
    };
    let response = ui.add(editor.id_salt(("value", node_key(key.0), &key.1)));
    if response.changed() {
        edits.insert(key.clone(), buffer.clone());
    }
    if response.lost_focus() {
        edits.remove(key);
        if buffer != text {
            return accept(&buffer);
        }
    }
    None
}

/// A labelled row (or block, for bigger values) editing one value; its new text once changed.
fn value_row(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    about: Option<&FieldInfo>,
    ty: FieldType,
    text: &str,
) -> Option<String> {
    let block = matches!(ty, FieldType::Structure)
        || matches!(ty, FieldType::Optional(FieldType::Structure)) && text != "None";
    if block {
        let label = ui.label(kit::dim(&key.1));
        if let Some(about) = about {
            label.on_hover_text(about.about);
        }
        edit_value(ui, edits, key, ty, text)
    } else {
        kit::prop(ui, &key.1, |ui| edit_value(ui, edits, key, ty, text))
    }
}

/// An editor for a value by what it holds; its new text once changed.
fn edit_value(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    ty: FieldType,
    text: &str,
) -> Option<String> {
    match ty {
        // Offsets are typed, never dragged: a stray drag would move them silently.
        FieldType::Offset => text_field(ui, edits, key, text, false, |t| {
            parse_int(t.trim()).map(|_| t.trim().to_string())
        }),
        FieldType::Count => match parse_int(text).filter(|_| !text.contains('.')) {
            Some(mut n) => {
                let like = text.to_string();
                let response = ui.add(
                    egui::DragValue::new(&mut n)
                        .speed(0.25)
                        .custom_formatter(move |v, _| format_int_like(v as i64, &like))
                        .custom_parser(|s| parse_int(s).map(|v| v as f64)),
                );
                response.changed().then(|| format_int_like(n, text))
            }
            None => text_field(ui, edits, key, text, false, |t| {
                parse_int(t.trim()).map(|_| t.trim().to_string())
            }),
        },
        FieldType::Number => match text.parse::<f64>() {
            Ok(mut f) => {
                let response = ui.add(egui::DragValue::new(&mut f).speed(0.01));
                response.changed().then(|| format!("{f:?}"))
            }
            Err(_) => text_field(ui, edits, key, text, false, |t| {
                t.trim().parse::<f64>().ok().map(|f| format!("{f:?}"))
            }),
        },
        FieldType::Flag => {
            let mut on = text.trim() == "true";
            ui.checkbox(&mut on, "").changed().then(|| on.to_string())
        }
        FieldType::Choice(variants) => {
            let mut picked = None;
            egui::ComboBox::from_id_salt(("choice", node_key(key.0), &key.1))
                .selected_text(text.trim())
                .show_ui(ui, |ui| {
                    for variant in variants {
                        if ui
                            .selectable_label(text.trim() == *variant, *variant)
                            .clicked()
                        {
                            picked = Some(variant.to_string());
                        }
                    }
                });
            picked
        }
        FieldType::Optional(inner) => {
            let value = text
                .trim()
                .strip_prefix("Some(")
                .and_then(|t| t.strip_suffix(')'))
                .map(str::trim);
            let mut set = value.is_some();
            let mut new = None;
            let mut row = |ui: &mut Ui| {
                if ui
                    .checkbox(&mut set, "")
                    .on_hover_text("Whether it is set; left out, it isn't")
                    .changed()
                {
                    new = Some(if set {
                        format!("Some({})", inner.default_text())
                    } else {
                        "None".to_string()
                    });
                }
                match value {
                    Some(value) => {
                        let key = (key.0, format!("{}/some", key.1));
                        if let Some(value) = edit_value(ui, edits, &key, *inner, value) {
                            new = Some(format!("Some({value})"));
                        }
                    }
                    None => {
                        ui.label(kit::dim("not set"));
                    }
                }
            };
            if *inner == FieldType::Structure {
                ui.vertical(|ui| row(ui));
            } else {
                ui.horizontal(|ui| row(ui));
            }
            new
        }
        FieldType::Name | FieldType::Text => match parse_string(text) {
            Some(string) => text_field(ui, edits, key, &string, false, |t| Some(string_literal(t))),
            None => text_field(ui, edits, key, text, false, |t| Some(t.to_string())),
        },
        // Shown without the indentation it has in the file, and given it back once changed.
        FieldType::Structure => {
            let indent = indent_of(text);
            let shown = reindented(text, &indent, "");
            text_field(ui, edits, key, &shown, true, |t| {
                Some(reindented(t, "", &indent))
            })
        }
    }
}

/// The indentation every line but the first of `text` has in common.
fn indent_of(text: &str) -> String {
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .min_by_key(|indent| indent.len())
        .unwrap_or_default()
        .to_string()
}

/// `text` with `from` taken off the start of every line but the first, and `to` put there.
fn reindented(text: &str, from: &str, to: &str) -> String {
    let mut lines = text.split('\n');
    let mut out = lines.next().unwrap_or_default().to_string();
    for line in lines {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(to);
        }
        out.push_str(line.strip_prefix(from).unwrap_or(line));
    }
    out
}

/// The texture of a node's preview, made again when the recipe ran again.
fn thumb(
    ctx: &egui::Context,
    thumbs: &mut HashMap<String, (u64, TextureHandle)>,
    revision: u64,
    node: RNode,
    image: &RgbaImage,
) -> TextureHandle {
    let key = node_key(node);
    match thumbs.get(&key) {
        Some((r, texture)) if *r == revision => texture.clone(),
        _ => {
            let color = ColorImage::from_rgba_unmultiplied(
                [image.width as usize, image.height as usize],
                &image.pixels,
            );
            let texture = ctx.load_texture(format!("recipe-{key}"), color, TextureOptions::NEAREST);
            thumbs.insert(key, (revision, texture.clone()));
            texture
        }
    }
}

/// The steps and outputs that can be added (those `filter` takes by the kinds they read and
/// make), found by typing; the template and whether it is a step, once one is picked.
fn add_list(
    ui: &mut Ui,
    query: &mut String,
    filter: impl Fn(&[NameKind], &[NameKind]) -> bool,
) -> Option<(String, bool)> {
    let field = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(kit::hint("🔍 Search, Enter adds the first"))
            .desired_width(f32::INFINITY),
    );
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let q = query.to_lowercase();
    let mut first = None;
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            for (heading, templates, step) in [
                ("Steps", STEP_TEMPLATES, true),
                ("Outputs", OUTPUT_TEMPLATES, false),
            ] {
                let matching: Vec<(&str, &str, &str)> = templates
                    .iter()
                    .filter_map(|(name, template)| {
                        let about = if step {
                            recipe_schema::step(name)
                        } else {
                            recipe_schema::output(name)
                        }
                        .map_or("", |i| i.about);
                        let (reads, writes) = template_kinds(template, step);
                        let found = q.is_empty()
                            || name.to_lowercase().contains(&q)
                            || about.to_lowercase().contains(&q);
                        (found && filter(&reads, &writes)).then_some((*name, *template, about))
                    })
                    .collect();
                if matching.is_empty() {
                    continue;
                }
                ui.label(RichText::new(heading).small().color(theme::BRASS));
                for (name, template, about) in matching {
                    if first.is_none() {
                        first = Some((template.to_string(), step));
                    }
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(name).strong().color(theme::PARCHMENT));
                            ui.label(RichText::new(about).small().color(theme::PARCHMENT_DIM));
                        })
                        .response;
                    if response.hovered() {
                        ui.painter().rect_stroke(
                            response.rect.expand(2.0),
                            4.0,
                            Stroke::new(1.0, theme::BRASS_DIM),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if response.clicked() {
                        picked = Some((template.to_string(), step));
                    }
                    ui.add_space(2.0);
                }
            }
            if first.is_none() {
                ui.label(kit::dim("Nothing matches"));
            }
        });
    if enter && picked.is_none() {
        picked = first;
    }
    if picked.is_some() {
        query.clear();
        ui.close();
    }
    picked
}

/// The graph's look, in the editor's colors.
fn style() -> SnarlStyle {
    SnarlStyle {
        node_layout: Some(NodeLayout::sandwich()),
        node_frame: Some(
            egui::Frame::new()
                .fill(theme::RAISED)
                .stroke(Stroke::new(1.0, theme::BRASS_DIM))
                .corner_radius(6)
                .inner_margin(8),
        ),
        bg_frame: Some(egui::Frame::new().fill(theme::BG_DEEP)),
        bg_pattern: Some(BackgroundPattern::grid(vec2(40.0, 40.0), 0.0)),
        bg_pattern_stroke: Some(Stroke::new(1.0, theme::PANEL)),
        pin_placement: Some(PinPlacement::Edge),
        pin_size: Some(9.0),
        wire_width: Some(2.0),
        collapsible: Some(true),
        min_scale: Some(MIN_SCALE),
        max_scale: Some(MAX_SCALE),
        ..SnarlStyle::new()
    }
}

/// Wires and text of nodes outside the selected node's lineage.
const DIMMED: Color32 = Color32::from_rgb(0x3A, 0x36, 0x30);

struct Viewer<'a> {
    nav: &'a mut Navigator,
    doc: &'a Doc,
    /// How many steps the recipe has.
    steps: usize,
    ports: &'a HashMap<RNode, Vec<OwnedPort>>,
    resolution: &'a Resolution,
    trace: &'a Trace,
    thumbs: &'a mut HashMap<String, (u64, TextureHandle)>,
    revision: u64,
    /// The node in the Inspector, and what it is made from and makes (the rest is dimmed).
    focus: Option<RNode>,
    lineage: Option<&'a HashSet<RNode>>,
    add_query: &'a mut String,
    /// Where a new step and a new output go (after the one in the Inspector).
    insert: (Option<usize>, Option<usize>),
    changes: Vec<Change>,
    sizes: HashMap<NodeId, Vec2>,
}

impl Viewer<'_> {
    fn port_in(&self, node: RNode, pin: usize) -> Option<(usize, &OwnedPort)> {
        let ports = self.ports.get(&node)?;
        let i = *inputs_of(ports).get(pin)?;
        Some((i, &ports[i]))
    }

    fn port_out(&self, node: RNode, pin: usize) -> Option<(usize, &OwnedPort)> {
        let ports = self.ports.get(&node)?;
        let i = *outputs_of(ports).get(pin)?;
        Some((i, &ports[i]))
    }

    fn lit(&self, node: RNode) -> bool {
        self.lineage.is_none_or(|l| l.contains(&node))
    }

    /// What the step or output made, in short: its first line and a small preview.
    fn result(&mut self, node: RNode, ui: &mut Ui) {
        let failed = match node {
            RNode::Step(i) => self.trace.error_step == Some(i),
            RNode::Output(i) => self.trace.error_output == Some(i),
        };
        if failed {
            ui.set_max_width(240.0);
            let error = self.trace.error.as_deref().unwrap_or_default();
            ui.label(RichText::new(error).small().color(theme::ERROR));
            return;
        }
        let results = match node {
            RNode::Step(_) => &self.trace.steps,
            RNode::Output(_) => &self.trace.outputs,
        };
        let Some(Some(result)) = results.get(index(node)) else {
            let ran = !self.trace.steps.is_empty();
            if ran && (matches!(node, RNode::Step(_)) || self.trace.error.is_some()) {
                ui.small(RichText::new("Not run").color(theme::PARCHMENT_DIM));
            }
            return;
        };
        if let Some(line) = result.lines.first() {
            let more = if result.lines.len() > 1 { " …" } else { "" };
            ui.small(RichText::new(format!("{line}{more}")).color(theme::PARCHMENT_DIM));
        }
        if let Some(image) = &result.image {
            let texture = thumb(ui.ctx(), self.thumbs, self.revision, node, image);
            let side = image.width.max(image.height).max(1) as f32;
            let scale = (NODE_THUMB / side).min(1.0);
            ui.image((
                texture.id(),
                vec2(image.width as f32, image.height as f32) * scale,
            ));
        }
    }

    fn add(&mut self, template: String, step: bool, at: Pos2, index: Option<usize>) -> &mut Change {
        self.changes.push(Change::Add {
            template,
            step,
            at,
            index,
            read: None,
            feed: None,
        });
        self.changes.last_mut().unwrap()
    }
}

impl SnarlViewer<RNode> for Viewer<'_> {
    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<RNode>,
    ) -> egui::Frame {
        let value = snarl[node];
        let frame = self.nav.frame(default, node);
        if self.focus == Some(value) {
            frame.stroke(Stroke::new(2.0, theme::BRASS_BRIGHT))
        } else if self.lit(value) {
            frame
        } else {
            frame
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER))
        }
    }

    fn has_node_style(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<RNode>,
    ) -> bool {
        !self.lit(snarl[node])
    }

    fn apply_node_style(
        &mut self,
        style: &mut egui::Style,
        _node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<RNode>,
    ) {
        style.visuals.override_text_color = Some(theme::FAINT);
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<RNode>,
    ) {
        self.nav.transform(to_global);
    }

    fn title(&mut self, node: &RNode) -> String {
        title(self.doc, *node)
    }

    fn inputs(&mut self, node: &RNode) -> usize {
        self.ports.get(node).map_or(0, |p| inputs_of(p).len())
    }

    fn outputs(&mut self, node: &RNode) -> usize {
        self.ports.get(node).map_or(0, |p| outputs_of(p).len())
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        snarl: &mut Snarl<RNode>,
    ) -> impl SnarlPin + 'static {
        let node = snarl[pin.id.node];
        let Some((i, port)) = self.port_in(node, pin.id.input) else {
            return PinInfo::circle();
        };
        let missing = self.resolution.missing.contains(&(node, i));
        let name = if port.name.is_empty() {
            "—"
        } else {
            &port.name
        };
        let color = if missing {
            theme::ERROR
        } else {
            theme::PARCHMENT_DIM
        };
        let (label, kind, current) = (port.label(), port.kind, port.name.clone());
        let response = ui
            .add(
                egui::Label::new(RichText::new(format!("{label}: {name}")).color(color))
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(if missing {
                format!(
                    "No earlier step makes the {} \"{}\". Click to pick one.",
                    kind.label(),
                    current
                )
            } else {
                format!("A {}. Click to pick another.", kind.label())
            });
        let names = candidates(self.ports, self.steps, node, kind);
        egui::Popup::menu(&response)
            .id(egui::Id::new(("pick input", node_key(node), i)))
            .show(|ui| {
                ui.label(kit::dim(format!("{label} reads")));
                if let Some(name) = name_menu(ui, &names, &current, kind) {
                    self.changes.push(Change::Rename {
                        at: (node, i),
                        name,
                    });
                }
            });
        let fill = if missing {
            theme::ERROR
        } else {
            kind_color(kind)
        };
        let lit = self.lit(node)
            && self
                .resolution
                .wires
                .iter()
                .any(|(from, to)| *to == (node, i) && self.lit(from.0));
        PinInfo::circle().with_fill(fill).with_wire_color(if lit {
            kind_color(kind)
        } else {
            DIMMED
        })
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        snarl: &mut Snarl<RNode>,
    ) -> impl SnarlPin + 'static {
        let node = snarl[pin.id.node];
        let Some((_, port)) = self.port_out(node, pin.id.output) else {
            return PinInfo::circle();
        };
        ui.label(RichText::new(&port.name).color(theme::PARCHMENT))
            .on_hover_text(port.kind.label());
        let color = kind_color(port.kind);
        PinInfo::circle()
            .with_fill(color)
            .with_wire_color(if self.lit(node) { color } else { DIMMED })
    }

    fn has_body(&mut self, _node: &RNode) -> bool {
        true
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<RNode>,
    ) {
        let node = snarl[node];
        ui.vertical(|ui| {
            ui.set_max_width(240.0);
            self.result(node, ui);
        });
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: Rect,
        _ui: &mut Ui,
        _snarl: &mut Snarl<RNode>,
    ) {
        self.sizes.insert(node, rect.size());
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<RNode>) {
        let (from_node, to_node) = (snarl[from.id.node], snarl[to.id.node]);
        if let (Some((a, _)), Some((b, _))) = (
            self.port_out(from_node, from.id.output),
            self.port_in(to_node, to.id.input),
        ) {
            self.changes.push(Change::Wire {
                from: (from_node, a),
                to: (to_node, b),
            });
        }
    }

    // A wire is a name: it goes away when the input reads something else (click the input).
    fn disconnect(&mut self, _from: &OutPin, _to: &InPin, _snarl: &mut Snarl<RNode>) {}

    fn drop_outputs(&mut self, _pin: &OutPin, _snarl: &mut Snarl<RNode>) {}

    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<RNode>) {
        let node = snarl[pin.id.node];
        if let Some((i, _)) = self.port_in(node, pin.id.input) {
            self.changes.push(Change::Rename {
                at: (node, i),
                name: String::new(),
            });
        }
    }

    fn has_node_menu(&mut self, _node: &RNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<RNode>,
    ) {
        let node = snarl[node];
        let (earlier, later) = match node {
            RNode::Step(_) => ("Run earlier  (Alt+↑)", "Run later  (Alt+↓)"),
            RNode::Output(_) => ("Move up  (Alt+↑)", "Move down  (Alt+↓)"),
        };
        let mut pick = |label: &str, change: Change| {
            if ui.button(label).clicked() {
                self.changes.push(change);
                ui.close();
            }
        };
        pick(earlier, Change::Move { node, later: false });
        pick(later, Change::Move { node, later: true });
        pick("Duplicate  (Ctrl+D)", Change::Duplicate(node));
        pick("Remove  (Delete)", Change::Remove(vec![node]));
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<RNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, _snarl: &mut Snarl<RNode>) {
        ui.set_width(320.0);
        ui.label(kit::hint(if self.insert.0.is_some() {
            "Add after the selected step"
        } else {
            "Add"
        }));
        if let Some((template, step)) = add_list(ui, self.add_query, |_, _| true) {
            let index = if step { self.insert.0 } else { self.insert.1 };
            self.add(template, step, pos, index);
        }
    }

    fn has_dropped_wire_menu(&mut self, src_pins: AnyPins, _snarl: &mut Snarl<RNode>) -> bool {
        matches!(src_pins, AnyPins::Out([_]) | AnyPins::In([_]))
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<RNode>,
    ) {
        ui.set_width(320.0);
        match src_pins {
            AnyPins::Out([from]) => {
                let node = snarl[from.node];
                let Some((_, port)) = self.port_out(node, from.output) else {
                    return;
                };
                let (kind, name) = (port.kind, port.name.clone());
                ui.label(kit::hint(format!("Use {} \"{name}\" in", kind.label())));
                // Right after the step that makes it.
                let after = match node {
                    RNode::Step(i) => Some(i + 1),
                    RNode::Output(_) => None,
                };
                if let Some((template, step)) =
                    add_list(ui, self.add_query, |reads, _| reads.contains(&kind))
                    && let Change::Add { read, .. } =
                        self.add(template, step, pos, if step { after } else { None })
                {
                    *read = Some((kind, name));
                }
            }
            AnyPins::In([to]) => {
                let node = snarl[to.node];
                let Some((i, port)) = self.port_in(node, to.input) else {
                    return;
                };
                let kind = port.kind;
                ui.label(kit::hint(format!("Make the {} with", kind.label())));
                // Right before the step that uses it.
                let before = match node {
                    RNode::Step(i) => Some(i),
                    RNode::Output(_) => None,
                };
                if let Some((template, step)) =
                    add_list(ui, self.add_query, |_, writes| writes.contains(&kind))
                    && let Change::Add { feed, .. } = self.add(template, step, pos, before)
                {
                    *feed = Some((node, i));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = r#"// The test recipe: comments must survive edits.
Recipe(
    steps: [
        // Tiles and palettes.
        Fill(name: "tiles", size: 64, value: 1),
        Fill(name: "map", size: 8),
        Literal(name: "pal", hex: "1F00 E003"),
        GbaPalettes(from: "pal", bank: "bg", slot: 0),
        GbaMetatiles(tiles: "tiles", char_base: 0x0, metatiles: "map", bank: "bg", columns: 1, into: "sheet"),
    ],
    outputs: [
        Image(id: "sheet", from: "sheet"),
    ],
)
"#;

    fn graph() -> RecipeGraph {
        RecipeGraph::new(RECIPE.into(), &RecipeLayout::default(), None).unwrap()
    }

    fn has_wire(graph: &RecipeGraph, from: RNode, to: RNode) -> bool {
        graph
            .resolution
            .wires
            .iter()
            .any(|((f, _), (t, _))| *f == from && *t == to)
    }

    #[test]
    fn steps_are_wired_by_the_names_they_use() {
        let graph = graph();
        assert!(has_wire(&graph, RNode::Step(0), RNode::Step(4)));
        assert!(has_wire(&graph, RNode::Step(3), RNode::Step(4)));
        assert!(has_wire(&graph, RNode::Step(4), RNode::Output(0)));
        assert!(graph.resolution.missing.is_empty());
        assert_eq!(graph.snarl.wires().count(), graph.resolution.wires.len());
    }

    #[test]
    fn the_recipe_runs_and_each_step_shows_what_it_made() {
        let mut graph = graph();
        graph.rom = Some(Arc::new(vec![0; 16]));
        graph.run_trace();
        assert_eq!(graph.trace.error, None);
        let sheet = graph.trace.steps[4].as_ref().unwrap();
        assert_eq!(sheet.lines, vec!["sheet: 16×16".to_string()]);
        assert!(sheet.image.is_some());
        let tiles = graph.trace.steps[0].as_ref().unwrap();
        assert_eq!(tiles.lines, vec!["tiles: 64 bytes"]);
        assert!(tiles.bytes.as_deref().unwrap().starts_with(&[1, 1, 1]));
        let output = graph.trace.outputs[0].as_ref().unwrap();
        assert_eq!(output.lines, vec!["16×16 pixels"]);

        // An output made from nothing: the error is shown on it.
        let mut graph = RecipeGraph::new(
            RECIPE.replace(
                "Image(id: \"sheet\", from: \"sheet\")",
                "Image(id: \"sheet\", from: \"nothing\")",
            ),
            &RecipeLayout::default(),
            Some(Arc::new(vec![0; 16])),
        )
        .unwrap();
        graph.run_trace();
        assert_eq!(graph.trace.error_output, Some(0));
        assert_eq!(graph.trace.error_step, None);
    }

    #[test]
    fn values_change_in_place_and_can_be_undone() {
        let mut graph = graph();
        let size = RecipeGraph::node_doc(&graph.doc, RNode::Step(0))
            .unwrap()
            .field("size")
            .unwrap()
            .span
            .clone();
        graph
            .apply(Change::Replace {
                span: size,
                text: "128".into(),
                field: Some((RNode::Step(0), "size".into())),
            })
            .unwrap();
        assert!(
            graph
                .text()
                .contains("Fill(name: \"tiles\", size: 128, value: 1)")
        );
        assert!(graph.text().contains("// Tiles and palettes."));
        assert!(graph.text_changed());
        graph.undo();
        assert_eq!(graph.text(), RECIPE);
        graph.redo();
        assert!(graph.text().contains("size: 128"));
    }

    #[test]
    fn text_that_does_not_parse_is_refused() {
        let mut graph = graph();
        let size = RecipeGraph::node_doc(&graph.doc, RNode::Step(0))
            .unwrap()
            .field("size")
            .unwrap()
            .span
            .clone();
        let refused = graph.apply(Change::Replace {
            span: size,
            text: "\"lots\"".into(),
            field: None,
        });
        assert!(refused.is_err());
        assert_eq!(graph.text(), RECIPE);
    }

    #[test]
    fn wiring_renames_the_input_and_respects_step_order() {
        let mut graph = graph();
        // The metatile step's "metatiles" input (port 1) reads "map"; wire "tiles" into it.
        graph
            .apply(Change::Wire {
                from: (RNode::Step(0), 0),
                to: (RNode::Step(4), 1),
            })
            .unwrap();
        assert!(graph.text().contains("metatiles: \"tiles\""));
        // A step can't use what a later step makes.
        let refused = graph.apply(Change::Wire {
            from: (RNode::Step(4), 3),
            to: (RNode::Output(0), 0),
        });
        assert!(refused.is_ok(), "outputs come after every step");
        let refused = graph.apply(Change::Wire {
            from: (RNode::Step(1), 0),
            to: (RNode::Step(0), 0),
        });
        assert!(
            refused.is_err(),
            "Fill has no inputs to wire, and steps run in order"
        );
        // Kinds must match: a bank can't be an image.
        assert!(
            graph
                .apply(Change::Wire {
                    from: (RNode::Step(3), 1),
                    to: (RNode::Output(0), 0),
                })
                .is_err()
        );
    }

    #[test]
    fn steps_are_added_wired_and_removed() {
        let mut graph = graph();
        graph
            .apply(Change::Add {
                template: crop(),
                step: true,
                at: Pos2::ZERO,
                index: None,
                read: Some((NameKind::Image, "sheet".into())),
                feed: None,
            })
            .unwrap();
        assert!(graph.text().contains("Crop(from: \"sheet\""));
        assert!(has_wire(&graph, RNode::Step(4), RNode::Step(5)));
        graph.apply(Change::Remove(vec![RNode::Step(5)])).unwrap();
        assert_eq!(graph.text(), RECIPE);

        // An output fed from a new step: the output reads what the step makes.
        graph
            .apply(Change::Add {
                template: crop(),
                step: true,
                at: Pos2::ZERO,
                index: None,
                read: None,
                feed: Some((RNode::Output(0), 0)),
            })
            .unwrap();
        assert!(
            graph
                .text()
                .contains("Image(id: \"sheet\", from: \"image_1\")")
        );
    }

    fn crop() -> String {
        STEP_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "Crop")
            .unwrap()
            .1
            .to_string()
    }

    #[test]
    fn steps_move_and_the_wires_follow() {
        let mut graph = graph();
        // The palettes run after the metatiles: the metatiles' bank input has nothing to read.
        graph
            .apply(Change::Move {
                node: RNode::Step(3),
                later: true,
            })
            .unwrap();
        assert!(matches!(graph.recipe.steps[4], Step::GbaPalettes { .. }));
        assert!(!graph.resolution.missing.is_empty());
        assert!(graph.text().contains("// Tiles and palettes."));
        graph
            .apply(Change::Move {
                node: RNode::Step(4),
                later: false,
            })
            .unwrap();
        assert_eq!(graph.text(), RECIPE);
        assert!(
            graph
                .apply(Change::Move {
                    node: RNode::Step(0),
                    later: false,
                })
                .is_err()
        );
    }

    #[test]
    fn duplicates_make_new_names_and_removing_many_is_one_step() {
        let mut graph = graph();
        graph.apply(Change::Duplicate(RNode::Step(0))).unwrap();
        assert!(
            graph
                .text()
                .contains("Fill(name: \"tiles_2\", size: 64, value: 1),")
        );
        graph.apply(Change::Duplicate(RNode::Output(0))).unwrap();
        assert!(
            graph
                .text()
                .contains("Image(id: \"sheet_2\", from: \"sheet\"),")
        );
        // The copies go right after what they copy.
        assert!(matches!(graph.recipe.steps[1], Step::Fill { .. }));
        assert_eq!(graph.focus, Some(RNode::Output(1)));
        graph
            .apply(Change::Remove(vec![RNode::Step(1), RNode::Output(1)]))
            .unwrap();
        assert_eq!(graph.text(), RECIPE);
        graph.undo();
        assert!(graph.text().contains("tiles_2") && graph.text().contains("sheet_2"));
    }

    #[test]
    fn edits_by_name_reach_the_same_changes() {
        let mut graph = graph();
        graph
            .edit(&Edit::Value("step 0".into(), "size".into(), "96".into()))
            .unwrap();
        assert!(graph.text().contains("size: 96"));
        graph
            .edit(&Edit::Wire(
                "step 0".into(),
                "tiles".into(),
                "step 4".into(),
                "metatiles".into(),
            ))
            .unwrap();
        assert!(graph.text().contains("metatiles: \"tiles\""));
        graph.edit(&Edit::Add("Fill".into())).unwrap();
        assert_eq!(graph.recipe.steps.len(), 6);
        graph.edit(&Edit::Move("step 5".into(), false)).unwrap();
        assert!(matches!(graph.recipe.steps[4], Step::Fill { .. }));
        assert!(graph.edit(&Edit::Add("Teleport".into())).is_err());
        assert!(graph.edit(&Edit::Remove("stage 1".into())).is_err());
        for _ in 0..4 {
            graph.undo();
        }
        assert_eq!(graph.text(), RECIPE);
    }

    #[test]
    fn every_template_is_a_valid_step_or_output() {
        for (name, template) in STEP_TEMPLATES {
            let step: Step = ron::from_str(template).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(step.name(), *name);
        }
        for (name, template) in OUTPUT_TEMPLATES {
            let output: Output = ron::from_str(template).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(output.name(), *name);
        }
    }

    #[test]
    fn the_layout_comes_back_by_place_and_variant() {
        let mut graph = graph();
        let id = graph
            .snarl
            .node_ids()
            .find(|(_, n)| **n == RNode::Step(2))
            .unwrap()
            .0;
        graph.snarl.get_node_info_mut(id).unwrap().pos = pos2(10.0, 20.0);
        let layout = graph.layout();
        assert_eq!(layout.steps[2], ("Literal".to_string(), (10.0, 20.0)));
        let again = RecipeGraph::new(RECIPE.into(), &layout, None).unwrap();
        assert!(!again.arrange_pending);
        assert_eq!(again.layout(), layout);
    }

    #[test]
    fn new_steps_go_after_the_selected_one_and_steps_move_anywhere() {
        let mut graph = graph();
        graph.focus = Some(RNode::Step(1));
        let fill = STEP_TEMPLATES.iter().find(|(n, _)| *n == "Fill").unwrap().1;
        graph
            .apply(Change::Add {
                template: fill.into(),
                step: true,
                at: Pos2::ZERO,
                index: graph.insert_index(true),
                read: None,
                feed: None,
            })
            .unwrap();
        assert_eq!(graph.focus, Some(RNode::Step(2)));
        assert!(matches!(&graph.recipe.steps[2], Step::Fill { name, .. } if name == "buffer_1"));
        // The step that was there moved down, and kept where it was drawn.
        assert!(matches!(graph.recipe.steps[3], Step::Literal { .. }));
        graph.apply(Change::Remove(vec![RNode::Step(2)])).unwrap();
        assert_eq!(graph.text(), RECIPE);

        graph
            .apply(Change::MoveTo {
                node: RNode::Step(4),
                to: 0,
            })
            .unwrap();
        assert!(matches!(graph.recipe.steps[0], Step::GbaMetatiles { .. }));
        assert!(matches!(graph.recipe.steps[1], Step::Fill { .. }));
        assert_eq!(graph.focus, Some(RNode::Step(0)));
        assert!(graph.text().contains("// Tiles and palettes."));
        graph
            .apply(Change::MoveTo {
                node: RNode::Step(0),
                to: 99,
            })
            .unwrap();
        assert_eq!(graph.text(), RECIPE);
    }

    #[test]
    fn inputs_are_renamed_and_names_made_are_renamed_everywhere() {
        let mut graph = graph();
        // The metatile step's "metatiles" input (port 1) reads "map".
        graph
            .apply(Change::Rename {
                at: (RNode::Step(4), 1),
                name: String::new(),
            })
            .unwrap();
        assert!(graph.text().contains("metatiles: \"\""));
        assert!(graph.resolution.missing.contains(&(RNode::Step(4), 1)));
        assert!(
            graph
                .problems()
                .iter()
                .any(|(node, p)| *node == Some(RNode::Step(4)) && p.contains("metatiles"))
        );
        graph.undo();
        graph
            .apply(Change::RenameMade {
                at: (RNode::Step(4), 3),
                name: "town".into(),
            })
            .unwrap();
        assert!(graph.text().contains("into: \"town\""));
        assert!(
            graph
                .text()
                .contains("Image(id: \"sheet\", from: \"town\")")
        );
        assert!(graph.resolution.missing.is_empty());
    }

    #[test]
    fn fields_left_out_are_written_when_changed() {
        let mut graph = graph();
        graph
            .apply(Change::AddField {
                node: RNode::Output(0),
                key: "kind".into(),
                value: "Some(Tileset)".into(),
            })
            .unwrap();
        assert!(
            graph
                .text()
                .contains("Image(id: \"sheet\", from: \"sheet\", kind: Some(Tileset))")
        );
    }

    #[test]
    fn every_template_field_is_described() {
        for (templates, step) in [(STEP_TEMPLATES, true), (OUTPUT_TEMPLATES, false)] {
            for (name, template) in templates {
                let info = if step {
                    recipe_schema::step(name)
                } else {
                    recipe_schema::output(name)
                }
                .unwrap_or_else(|| panic!("{name} isn't described"));
                let doc = Doc::parse(*template).unwrap();
                let Kind::Struct { fields, .. } = &doc.root.kind else {
                    panic!("{name}");
                };
                for field in fields {
                    assert!(info.field(&field.key).is_some(), "{name}.{}", field.key);
                }
                for about in info.fields.iter().filter(|f| !f.optional) {
                    assert!(
                        fields.iter().any(|f| f.key == about.key),
                        "{name} template lacks {}",
                        about.key
                    );
                }
            }
        }
    }

    #[test]
    fn values_are_edited_by_what_they_hold() {
        assert_eq!(guess_type("0x40"), FieldType::Count);
        assert_eq!(guess_type("1.5"), FieldType::Number);
        assert_eq!(guess_type("\"a\""), FieldType::Text);
        assert_eq!(guess_type("(x: 1)"), FieldType::Structure);
        let nested = "(\n            a: 1,\n              b: 2,\n        )";
        let indent = indent_of(nested);
        assert_eq!(indent, "        ");
        let shown = reindented(nested, &indent, "");
        assert_eq!(shown, "(\n    a: 1,\n      b: 2,\n)");
        assert_eq!(reindented(&shown, "", &indent), nested);
        let mut places = Places::new();
        places.insert(RNode::Step(1), (Pos2::ZERO, false));
        places.insert(RNode::Output(0), (Pos2::ZERO, true));
        let places = remap(places, |n| Some(shifted(n, RNode::Step(0))));
        assert_eq!(places[&RNode::Step(2)], (Pos2::ZERO, false));
        assert!(places.contains_key(&RNode::Output(0)));
    }
}

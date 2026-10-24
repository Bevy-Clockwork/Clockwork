//! An extraction plugin's recipe drawn as a node graph: one node per step and per output, wired
//! by the names steps make and use (buffers, palette banks, images, models, collisions, sprites).
//!
//! The graph is a view over `recipe.ron` as text. Every change (a number, a name, a wire, a new
//! or removed step) replaces a span of that text and is parsed again before it is kept, so the
//! agent's comments survive and nothing the format can't hold is ever written. Each change runs
//! the recipe on the user's ROM, and nodes show what their step made.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
    path::Path,
    sync::Arc,
};

use bevy_egui::egui::{self, Color32, Pos2, Rect, RichText, TextureHandle, Ui, Vec2, pos2, vec2};
use clockwork_core::{
    recipe::{Access, NameKind, Recipe, Seg},
    recipe_schema::FieldType,
};
use clockwork_recipe::{
    edit,
    text::{Doc, Kind, Node, parse_int, parse_string, string_literal},
};
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use serde::{Deserialize, Serialize};

use crate::{graph_view::Navigator, kit, theme};

mod blocks;
mod groups;
mod model;
mod order;
mod templates;
mod trace;
mod viewer;
mod widgets;

use groups::{GNode, GroupId, GroupInfo, SavedGroup};
pub use model::RNode;
use model::*;
use templates::*;
use trace::*;
use viewer::*;
use widgets::*;

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

/// Where the nodes sit, saved as `graph.ron`: by a key that stays the same when the recipe
/// changes elsewhere (see [`groups::stable_keys`]), and the groups they are in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeLayout {
    /// Places by step and output index, with the variant: how layouts were kept before keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<(String, (f32, f32))>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<(String, (f32, f32))>,
    /// Places by node key.
    #[serde(default)]
    pub nodes: BTreeMap<String, (f32, f32)>,
    #[serde(default)]
    pub groups: Vec<SavedGroup>,
    /// Values of nodes shown on the closed group they are in, by node key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exposed: BTreeMap<String, Vec<String>>,
    /// Groups were made (or all removed) for this recipe, so it isn't grouped by itself again.
    #[serde(default)]
    pub grouped: bool,
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
    clockwork_library::files::write_atomic(dir.join(LAYOUT_FILE), text).map_err(|e| e.to_string())
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
    /// Adds a block of steps as a closed group, by its name: `AddBlock("GBA tileset")`.
    AddBlock(String),
    /// Sets the note above a node: `Note("step 3", "From the gfx table")`.
    Note(String, String),
    /// Replaces the whole text, as typing it in the Text tab does.
    Text(String),
    Remove(String),
    /// Runs a step later (`true`) or earlier; moves an output down or up.
    Move(String, bool),
    Duplicate(String),
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
    /// Adds a block (an index into [`blocks::BLOCKS`]): its steps after the selected step (or
    /// last), its outputs last, all in a new closed group at `at`.
    AddBlock { block: usize, at: Pos2 },
    /// The note above a node (see [`Doc::note`]).
    Note { node: RNode, note: String },
    /// A whole new text, typed in the Text tab.
    Text(String),
    /// Removes these steps and outputs.
    Remove(Vec<RNode>),
    /// Swaps a step or output with the one after it (`later`) or before it.
    Move { node: RNode, later: bool },
    /// Moves a step or output to this place in its list.
    MoveTo { node: RNode, to: usize },
    /// Adds a copy after it, with the names it makes renamed so nothing else changes.
    Duplicate(RNode),
}

/// Where a node is drawn, whether it is open, the group it is in and which of its values its
/// closed group shows.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    pos: Pos2,
    open: bool,
    group: Option<GroupId>,
    exposed: Vec<String>,
}

impl Place {
    fn at(pos: Pos2) -> Self {
        Self {
            pos,
            open: true,
            group: None,
            exposed: Vec::new(),
        }
    }
}

/// Every node's [`Place`], by node.
type Places = HashMap<RNode, Place>;

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
    snarl: Snarl<GNode>,
    /// Every node's place, drawn or inside a closed group.
    nodes: Places,
    groups: BTreeMap<GroupId, GroupInfo>,
    /// The pins of the closed groups drawn.
    pins: HashMap<GroupId, groups::Pins>,
    /// Groups were made (or all removed), so the recipe isn't grouped by itself again.
    grouped: bool,
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
    /// Whether the order steps run in is drawn over the graph.
    pub show_order: bool,
    /// Every plugin's assets, where the recipe's characters find the parts they name.
    pub assets: Arc<clockwork_recipe::assets::OwnedAssets>,
    /// The plugin's name, for the parts it names of itself.
    pub plugin: String,
    /// The node the Inspector shows: the one last selected, added or moved.
    focus: Option<RNode>,
    /// The group the Inspector shows, when a closed group was selected last.
    focus_group: Option<GroupId>,
    /// The Text tab's text while it doesn't parse (or is being typed), and where to put its
    /// cursor next (a byte in it).
    text_buffer: Option<String>,
    text_jump: Option<usize>,
    /// Whether the Text tab was shown last, not the graph.
    text_shown: bool,
    /// What is typed as a group's new name, while it is renamed.
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
        let ports = all_ports(&recipe);
        let keys = groups::stable_keys(&recipe, &ports);
        let mut places = Places::new();
        for (node, key) in &keys {
            if let Some(p) = layout.nodes.get(key) {
                places.insert(*node, Place::at(pos2(p.0, p.1)));
            }
        }
        // A layout kept before nodes had keys: by place, as long as the variant is the same.
        for (i, step) in recipe.steps.iter().enumerate() {
            if let Some((name, p)) = layout.steps.get(i)
                && name == step.name()
            {
                places
                    .entry(RNode::Step(i))
                    .or_insert(Place::at(pos2(p.0, p.1)));
            }
        }
        for (i, output) in recipe.outputs.iter().enumerate() {
            if let Some((name, p)) = layout.outputs.get(i)
                && name == output.name()
            {
                places
                    .entry(RNode::Output(i))
                    .or_insert(Place::at(pos2(p.0, p.1)));
            }
        }
        let by_key: HashMap<&str, RNode> = keys.iter().map(|(n, k)| (k.as_str(), *n)).collect();
        let mut saved_groups = BTreeMap::new();
        for group in &layout.groups {
            saved_groups.insert(
                group.id,
                GroupInfo {
                    name: group.name.clone(),
                    parent: group.parent,
                    open: group.open,
                    pos: pos2(group.at.0, group.at.1),
                    laid_out: group.laid_out,
                },
            );
            for key in &group.members {
                if let Some(node) = by_key.get(key.as_str())
                    && let Some(place) = places.get_mut(node)
                {
                    place.group = Some(group.id);
                }
            }
        }
        for (key, fields) in &layout.exposed {
            if let Some(node) = by_key.get(key.as_str())
                && let Some(place) = places.get_mut(node)
            {
                place.exposed = fields.clone();
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
            nodes: Places::new(),
            groups: saved_groups,
            pins: HashMap::new(),
            grouped: layout.grouped || !layout.groups.is_empty(),
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
            show_order: false,
            focus: None,
            focus_group: None,
            text_buffer: None,
            text_jump: None,
            text_shown: false,
            selection: Vec::new(),
            add_query: String::new(),
            add_at: None,
            problem: 0,
        };
        graph.nav.open_view();
        graph.refresh(places);
        let count = graph.recipe.steps.len() + graph.recipe.outputs.len();
        if !graph.grouped && count >= groups::AUTO_GROUP {
            graph.group_automatically(count >= groups::AUTO_GROUP_BY_OUTPUTS);
            graph.arrange_pending = true;
        }
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
            let places = self.places_for(&text);
            if self.set_text(&text, places).is_ok() {
                self.redo.push(current);
            }
        }
        self.last_field = None;
    }

    pub fn redo(&mut self) {
        if let Some(text) = self.redo.pop() {
            let current = self.doc.text.clone();
            let places = self.places_for(&text);
            if self.set_text(&text, places).is_ok() {
                self.undo.push(current);
            }
        }
        self.last_field = None;
    }

    /// The places of the nodes for another text of the recipe, by stable key, so a node keeps
    /// its place and group when others before it come or go.
    fn places_for(&self, text: &str) -> Places {
        let mut by_key: HashMap<String, Place> = HashMap::new();
        let mut places = self.places();
        for (node, key) in groups::stable_keys(&self.recipe, &self.ports) {
            if let Some(place) = places.remove(&node) {
                by_key.insert(key, place);
            }
        }
        let Ok((_, recipe)) = parse(text) else {
            return self.places();
        };
        groups::stable_keys(&recipe, &all_ports(&recipe))
            .into_iter()
            .filter_map(|(node, key)| Some((node, by_key.remove(&key)?)))
            .collect()
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
        let keys = groups::stable_keys(&self.recipe, &self.ports);
        let mut layout = RecipeLayout {
            grouped: self.grouped,
            ..RecipeLayout::default()
        };
        let mut members: BTreeMap<GroupId, Vec<String>> = BTreeMap::new();
        for (node, key) in keys {
            let Some(place) = places.get(&node) else {
                continue;
            };
            layout
                .nodes
                .insert(key.clone(), (place.pos.x.round(), place.pos.y.round()));
            if !place.exposed.is_empty() {
                layout.exposed.insert(key.clone(), place.exposed.clone());
            }
            if let Some(g) = place.group {
                members.entry(g).or_default().push(key);
            }
        }
        let shown = self.group_places();
        for (id, info) in &self.groups {
            let pos = shown.get(id).copied().unwrap_or(info.pos);
            layout.groups.push(SavedGroup {
                id: *id,
                name: info.name.clone(),
                parent: info.parent,
                open: info.open,
                at: (pos.x.round(), pos.y.round()),
                laid_out: info.laid_out,
                members: members.remove(id).unwrap_or_default(),
            });
        }
        layout
    }

    /// Where every node is drawn, whether it is open, and the group it is in.
    fn places(&self) -> Places {
        let mut places = self.nodes.clone();
        for info in self.snarl.nodes_info() {
            if let GNode::Item(node) = info.value
                && let Some(place) = places.get_mut(&node)
            {
                place.pos = info.pos;
                place.open = info.open;
            }
        }
        places
    }

    /// Where the closed groups are drawn.
    fn group_places(&self) -> HashMap<GroupId, Pos2> {
        self.snarl
            .nodes_info()
            .filter_map(|info| match info.value {
                GNode::Group(g) => Some((g, info.pos)),
                GNode::Item(_) => None,
            })
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
        // Closed groups keep where they were drawn.
        for (g, pos) in self.group_places() {
            if let Some(info) = self.groups.get_mut(&g) {
                info.pos = pos;
            }
        }
        self.ports = all_ports(&self.recipe);
        self.resolution = resolve(&self.recipe, &self.ports);
        let depth = depths(&self.recipe, &self.resolution, &self.ports);
        let last = depth.values().copied().max().unwrap_or(0) + 1;
        let mut nodes = Places::new();
        for (row, node) in (0..self.recipe.steps.len())
            .map(RNode::Step)
            .chain((0..self.recipe.outputs.len()).map(RNode::Output))
            .enumerate()
        {
            let column = match node {
                RNode::Step(_) => depth.get(&node).copied().unwrap_or(0),
                RNode::Output(_) => last,
            };
            let place = places
                .get(&node)
                .cloned()
                .unwrap_or_else(|| Place::at(pos2(column as f32 * COLUMN, row as f32 * ROW * 0.5)));
            nodes.insert(node, place);
        }
        self.nodes = nodes;
        self.prune_groups();

        let mut snarl = Snarl::new();
        let mut ids: HashMap<GNode, NodeId> = HashMap::new();
        for node in (0..self.recipe.steps.len())
            .map(RNode::Step)
            .chain((0..self.recipe.outputs.len()).map(RNode::Output))
        {
            let shown = groups::shown_as(&self.groups, &self.nodes, node);
            if ids.contains_key(&shown) {
                continue;
            }
            let (pos, open) = match shown {
                GNode::Item(_) => (self.nodes[&node].pos, self.nodes[&node].open),
                GNode::Group(g) => (self.groups[&g].pos, true),
            };
            let id = snarl.insert_node(pos, shown);
            if !open {
                snarl.open_node(id, false);
            }
            ids.insert(shown, id);
        }
        self.pins = groups::pins(&self.groups, &self.nodes, &self.resolution);
        for ((from, from_port), (to, to_port)) in &self.resolution.wires {
            let (a, b) = (
                groups::shown_as(&self.groups, &self.nodes, *from),
                groups::shown_as(&self.groups, &self.nodes, *to),
            );
            if a == b {
                continue;
            }
            let output = match a {
                GNode::Item(_) => outputs_of(&self.ports[from])
                    .iter()
                    .position(|p| p == from_port),
                GNode::Group(g) => self.pins[&g]
                    .outputs
                    .iter()
                    .position(|o| *o == (*from, *from_port)),
            };
            let input = match b {
                GNode::Item(_) => inputs_of(&self.ports[to]).iter().position(|p| p == to_port),
                GNode::Group(g) => self.pins[&g]
                    .inputs
                    .iter()
                    .position(|i| i.from == Some((*from, *from_port))),
            };
            if let (Some(output), Some(input)) = (output, input) {
                snarl.connect(
                    OutPinId {
                        node: ids[&a],
                        output,
                    },
                    InPinId {
                        node: ids[&b],
                        input,
                    },
                );
            }
        }
        self.snarl = snarl;
        self.sizes.clear();
        if self
            .focus_group
            .is_some_and(|g| !self.groups.contains_key(&g))
        {
            self.focus_group = None;
        }
    }

    /// Drops groups with nothing in them, and parents that are gone.
    fn prune_groups(&mut self) {
        loop {
            let used: HashSet<GroupId> = self
                .nodes
                .values()
                .filter_map(|p| p.group)
                .chain(self.groups.values().filter_map(|g| g.parent))
                .collect();
            let empty: Vec<GroupId> = self
                .groups
                .keys()
                .copied()
                .filter(|g| !used.contains(g))
                .collect();
            if empty.is_empty() {
                break;
            }
            for g in empty {
                self.groups.remove(&g);
            }
        }
        let known: HashSet<GroupId> = self.groups.keys().copied().collect();
        for info in self.groups.values_mut() {
            if info.parent.is_some_and(|p| !known.contains(&p)) {
                info.parent = None;
            }
        }
        for place in self.nodes.values_mut() {
            if place.group.is_some_and(|g| !known.contains(&g)) {
                place.group = None;
            }
        }
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
            self.rom.as_ref(),
            &self.assets,
            &self.plugin,
            &self.ports,
            &self.trace,
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
        // Only what the next run can go on from: the steps it need not run again, and what
        // they showed.
        let last = Trace {
            steps: self.trace.steps.clone(),
            kept: self.trace.kept.clone(),
            ..Trace::default()
        };
        let started = std::thread::Builder::new()
            .name("recipe trace".into())
            .spawn(move || trace(&recipe, rom.as_ref(), &assets, &plugin, &ports, &last));
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
        edit::node(doc, item(node))
    }

    /// Makes a change; on success it can be undone.
    fn apply(&mut self, change: Change) -> Result<(), String> {
        let mut places = self.places();
        let field = match &change {
            Change::Replace { field, .. } => field.clone(),
            // Typing is undone at once, as a change to one field.
            Change::Text(_) => Some((RNode::Step(usize::MAX), "text".into())),
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
                // It joins the group of the node before it (after it, when first), if that is
                // open: a new node never disappears into a closed group.
                let neighbour = same_list(node, if place > 0 { place - 1 } else { place + 1 });
                let group = places
                    .get(&neighbour)
                    .and_then(|p| p.group)
                    .filter(|g| groups::shown_group(&self.groups, Some(*g)).is_none());
                places.insert(
                    node,
                    Place {
                        group,
                        ..Place::at(at)
                    },
                );
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
            Change::AddBlock { block, at } => {
                let block = blocks::BLOCKS.get(block).ok_or("no such block")?;
                let (steps, outputs) = block.named(&self.taken_names());
                let group = self.groups.keys().max().map_or(1, |m| m + 1);
                let first = self.insert_index(true).unwrap_or(self.recipe.steps.len());
                let mut doc = self.doc.clone();
                let mut added = Vec::new();
                for (list, items, start) in [
                    ("steps", &steps, first),
                    ("outputs", &outputs, self.recipe.outputs.len()),
                ] {
                    for (i, item) in items.iter().enumerate() {
                        let seq = doc
                            .root
                            .field(list)
                            .ok_or(format!("the recipe has no {list} list"))?;
                        let place = start + i;
                        doc = Doc::parse(doc.inserted(seq, place, item))
                            .map_err(|e| e.to_string())?;
                        let node = if list == "steps" {
                            RNode::Step(place)
                        } else {
                            RNode::Output(place)
                        };
                        places = remap(places, |n| Some(shifted(n, node)));
                        added.push(node);
                    }
                }
                for (i, node) in added.iter().enumerate() {
                    let exposed = block
                        .shown
                        .iter()
                        .filter(|(step, _)| *step == i && matches!(node, RNode::Step(_)))
                        .map(|(_, field)| field.to_string())
                        .collect();
                    places.insert(
                        *node,
                        Place {
                            group: Some(group),
                            exposed,
                            ..Place::at(at)
                        },
                    );
                }
                let taken: HashSet<&str> = self.groups.values().map(|g| g.name.as_str()).collect();
                let name = (1..)
                    .map(|n| match n {
                        1 => block.name.to_string(),
                        n => format!("{} {n}", block.name),
                    })
                    .find(|n| !taken.contains(n.as_str()))
                    .unwrap_or_default();
                self.groups.insert(
                    group,
                    GroupInfo {
                        name,
                        parent: None,
                        open: false,
                        pos: at,
                        laid_out: false,
                    },
                );
                self.grouped = true;
                self.focus_group = Some(group);
                focus = None;
                doc.text
            }
            Change::Note { node, note } => edit::apply(
                &self.doc,
                &edit::Edit::Note {
                    item: item(node),
                    note,
                },
            )?,
            Change::Text(text) => {
                places = self.places_for(&text);
                text
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
                let original = places.get(&node).cloned();
                places = remap(places, |n| Some(shifted(n, copy)));
                if let Some(original) = original {
                    places.insert(
                        copy,
                        Place {
                            pos: original.pos + vec2(40.0, 40.0),
                            exposed: Vec::new(),
                            ..original
                        },
                    );
                }
                focus = Some(copy);
                doc.text
            }
        };
        let before = self.doc.text.clone();
        if let Err(e) = self.set_text(&text, places) {
            // A block's group, made before its text was.
            self.prune_groups();
            return Err(e);
        }
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

    /// A place below everything drawn.
    fn below_everything(&self) -> Pos2 {
        let below = self
            .snarl
            .nodes_pos_ids()
            .map(|(id, pos, _)| pos.y + self.sizes.get(&id).map_or(ROW, |size| size.y))
            .fold(0.0, f32::max);
        pos2(0.0, below + GAP)
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
        let text = edit::moved(&self.doc, item(node), to)?;
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
        Ok(text)
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
            Edit::Note(node, note) => Change::Note {
                node: node_named(node)?,
                note: note.clone(),
            },
            Edit::Text(text) => {
                parse(text)?;
                Change::Text(text.clone())
            }
            Edit::AddBlock(name) => {
                let block = blocks::BLOCKS
                    .iter()
                    .position(|b| b.name == name)
                    .ok_or_else(|| format!("there is no block called {name}"))?;
                Change::AddBlock {
                    block,
                    at: self.below_everything(),
                }
            }
            Edit::Add(variant) => {
                let (template, step) = templates(true)
                    .iter()
                    .find(|(n, _)| n == variant)
                    .map(|(_, t)| (t.as_str(), true))
                    .or_else(|| {
                        templates(false)
                            .iter()
                            .find(|(n, _)| n == variant)
                            .map(|(_, t)| (t.as_str(), false))
                    })
                    .ok_or_else(|| format!("there is no step or output called {variant}"))?;
                Change::Add {
                    template: template.to_string(),
                    step,
                    at: self.below_everything(),
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
            "recipe, {} steps ({} ran), {} outputs, {} wires, {} inputs missing, error {:?}, \
             {} drawn, groups {:?}",
            self.recipe.steps.len(),
            made,
            self.recipe.outputs.len(),
            self.resolution.wires.len(),
            self.resolution.missing.len(),
            self.trace.error,
            self.snarl.node_ids().count(),
            self.group_list()
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
        self.focus_group = None;
        // Closed groups it is in open first.
        let group = self.nodes.get(&node).and_then(|p| p.group);
        if groups::shown_group(&self.groups, group).is_some() {
            let mut places = self.places();
            let closed: Vec<GroupId> = groups::ancestors(&self.groups, group)
                .into_iter()
                .filter(|g| self.groups.get(g).is_some_and(|i| !i.open))
                .collect();
            self.open_groups(&closed, true, &mut places);
            self.refresh(places);
        }
        if let Some((id, pos, _)) = self
            .snarl
            .nodes_pos_ids()
            .find(|(_, _, n)| **n == GNode::Item(node))
        {
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
        self.text_shown = false;
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

        let (doc, groups_info, nodes) = (&self.doc, &self.groups, &self.nodes);
        self.nav.update(
            &self.snarl,
            &self.sizes,
            ui.available_rect_before_wrap(),
            |_, node| match node {
                GNode::Item(node) => {
                    let text = Self::node_doc(doc, *node).map_or("", |n| doc.text_of(n));
                    format!("{} {text}", title(doc, *node))
                }
                // A closed group is found by its name and by what is inside it.
                GNode::Group(g) => {
                    let mut text = groups_info.get(g).map_or(String::new(), |i| i.name.clone());
                    for n in groups::members(groups_info, nodes, *g) {
                        let inner = Self::node_doc(doc, n).map_or("", |item| doc.text_of(item));
                        text.push(' ');
                        text.push_str(&title(doc, n));
                        text.push(' ');
                        text.push_str(inner);
                    }
                    text
                }
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
            focus_group: self.focus_group,
            lineage: lineage.as_ref(),
            add_query: &mut self.add_query,
            insert,
            groups: &self.groups,
            pins: &self.pins,
            nodes: &self.nodes,
            edits: &mut self.edits,
            last_sizes: &self.sizes,
            show_order: self.show_order,
            changes: Vec::new(),
            group_changes: Vec::new(),
            sizes: HashMap::new(),
        };
        let widget = egui_snarl::ui::SnarlWidget::new()
            .id_salt(("recipe_graph", id_salt))
            .style(style())
            .min_size(ui.available_size());
        let response = widget.show(&mut self.snarl, &mut viewer, ui);
        let frames = viewer.frames(&self.snarl);
        changes.extend(std::mem::take(&mut viewer.changes));
        let mut group_changes = std::mem::take(&mut viewer.group_changes);
        let sizes = std::mem::take(&mut viewer.sizes);
        self.nav
            .minimap(ui, id_salt, &self.snarl, &self.sizes, |node| match node {
                GNode::Item(RNode::Step(_)) => theme::PARCHMENT_DIM,
                GNode::Item(RNode::Output(_)) => theme::SUCCESS,
                GNode::Group(_) => theme::BRASS,
            });

        // Open groups have their name and a button to close them above their frame.
        for (g, _, rect) in frames {
            let screen = self.nav.screen_rect(rect);
            if !response.rect.contains(screen.left_top() + vec2(4.0, 4.0)) {
                continue;
            }
            let name = self
                .groups
                .get(&g)
                .map_or(String::new(), |i| i.name.clone());
            egui::Area::new(egui::Id::new(("group header", id_salt, g)))
                .order(egui::Order::Foreground)
                .fixed_pos(screen.right_top() + vec2(-34.0, 6.0))
                .show(ui.ctx(), |ui| {
                    if kit::icon_button(ui, "▾", &format!("Close {name}")).clicked() {
                        group_changes.push(GroupChange::Open(g, false));
                    }
                });
        }

        // A closed group opens with a double-click.
        let double = ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        });
        if double
            && let Some(at) = ui.input(|i| i.pointer.latest_pos())
            && response.rect.contains(at)
        {
            let at = self.nav.graph_pos(at);
            let hit = self.snarl.nodes_pos_ids().find_map(|(id, pos, node)| {
                let size = sizes.get(&id).copied().unwrap_or(vec2(240.0, 100.0));
                match node {
                    GNode::Group(g) if Rect::from_min_size(pos, size).contains(at) => Some(*g),
                    _ => None,
                }
            });
            if let Some(g) = hit {
                group_changes.push(GroupChange::Open(g, true));
            }
        }

        // The Inspector follows the selection when it changes.
        let selected = widget.get_selected_nodes(ui);
        if selected != self.selection {
            let last = selected
                .last()
                .and_then(|id| self.snarl.get_node(*id))
                .copied();
            (self.focus, self.focus_group) = match last {
                Some(GNode::Item(node)) => (Some(node), None),
                Some(GNode::Group(g)) => (None, Some(g)),
                None => (None, None),
            };
            self.selection = selected;
            ui.ctx().request_repaint();
        }

        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            let selected: Vec<GNode> = self
                .selection
                .iter()
                .filter_map(|id| self.snarl.get_node(*id).copied())
                .collect();
            let delete = ui.input(|i| i.key_pressed(egui::Key::Delete));
            if delete {
                let removed: Vec<RNode> = selected
                    .iter()
                    .flat_map(|node| match node {
                        GNode::Item(n) => vec![*n],
                        GNode::Group(g) => groups::members(&self.groups, &self.nodes, *g),
                    })
                    .collect();
                if !removed.is_empty() {
                    changes.insert(0, Change::Remove(removed));
                }
            }
            if !selected.is_empty()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::G))
            {
                self.group(&selected, None);
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
                        ui.label(kit::hint("Add a step, an output or a block"));
                        add_list(ui, &mut self.add_query, true, |_, _| true)
                    })
                });
            match area.inner.inner {
                Some(Picked::Template(template, step)) => {
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
                Some(Picked::Block(block)) => {
                    changes.push(Change::AddBlock { block, at });
                    close = true;
                }
                None => {}
            }
            if close || area.response.clicked_elsewhere() {
                self.add_at = None;
            }
        }

        for change in group_changes {
            match change {
                GroupChange::Open(g, open) => self.set_group_open(g, open),
                GroupChange::Ungroup(g) => self.ungroup(g),
                GroupChange::Rename(g, name) => self.rename_group(g, &name),
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

    /// The recipe as text, edited as such: each change that parses is made (and undone with
    /// the ones typed right before it), one that doesn't shows why. Shown first, it puts the
    /// cursor on the node in focus.
    pub fn show_text(&mut self, ui: &mut Ui, id_salt: &str) {
        if !self.text_shown {
            self.text_jump = self
                .focus
                .and_then(|node| Self::node_doc(&self.doc, node))
                .map(|item| item.span.start);
            self.text_buffer = None;
        }
        self.text_shown = true;
        let mut buffer = self
            .text_buffer
            .take()
            .unwrap_or_else(|| self.doc.text.clone());
        let error = (buffer != self.doc.text)
            .then(|| parse(&buffer).err())
            .flatten();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 4))
            .show(ui, |ui| match &error {
                Some(e) => {
                    ui.label(RichText::new(format!("⚠ Not applied: {e}")).color(theme::ERROR));
                }
                None => {
                    ui.label(kit::hint(
                        "Changes are made as you type, whenever the text is a recipe; \
                         comments stay as written",
                    ));
                }
            });
        let id = egui::Id::new(("recipe text", id_salt));
        let output = egui::ScrollArea::both()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::TextEdit::multiline(&mut buffer)
                    .id(id)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .show(ui)
            })
            .inner;
        if let Some(at) = self.text_jump.take() {
            let char = buffer[..at.min(buffer.len())].chars().count();
            let cursor = egui::text::CCursor::new(char);
            let mut state = output.state.clone();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
            state.store(ui.ctx(), id);
            output.response.request_focus();
            let rect = output.galley.pos_from_cursor(cursor);
            ui.scroll_to_rect(
                rect.translate(output.galley_pos.to_vec2()),
                Some(egui::Align::Center),
            );
        }
        if buffer != self.doc.text && parse(&buffer).is_ok() {
            match self.apply(Change::Text(buffer.clone())) {
                Ok(()) => self.message = None,
                Err(e) => self.message = Some(e),
            }
        }
        // Kept while it differs: typed and not a recipe yet.
        if buffer != self.doc.text {
            self.text_buffer = Some(buffer);
        }
        self.poll_trace();
        if self.trace_dirty && self.trace_job.is_none() {
            self.start_trace();
        }
    }

    /// The node in focus, every value of it and what it made; with nothing in focus, how to edit
    /// and what is wrong.
    fn inspector(&mut self, ui: &mut Ui, changes: &mut Vec<Change>) {
        let Some(node) = self.focus else {
            match self.focus_group.filter(|g| self.groups.contains_key(g)) {
                Some(group) => self.group_inspector(ui, group),
                None => self.overview(ui, changes),
            }
            return;
        };
        let in_group = self
            .nodes
            .get(&node)
            .and_then(|p| p.group)
            .and_then(|g| Some((g, self.groups.get(&g)?.name.clone())));
        let mut toggled: Option<String> = None;
        let Self {
            doc,
            recipe,
            ports,
            resolution,
            trace,
            edits,
            thumbs,
            revision,
            nodes,
            rom,
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
        let mut to_group = None;
        if let Some((group, name)) = &in_group {
            ui.horizontal(|ui| {
                ui.label(kit::dim("In the group"));
                if ui.link(name).clicked() {
                    to_group = Some(*group);
                }
            });
        }
        // The comment above it, where the agent says where its values come from.
        let (_, note) = doc.note(item);
        let note_key = (node, "#note".to_string());
        let buffer = edits
            .entry(note_key.clone())
            .or_insert_with(|| note.clone());
        let response = ui.add(
            egui::TextEdit::multiline(buffer)
                .hint_text("A note: where it comes from, what it is")
                .desired_rows(1)
                .desired_width(f32::INFINITY),
        );
        let typed = buffer.clone();
        if response.lost_focus() && typed != note {
            changes.push(Change::Note { node, note: typed });
        }
        if !response.has_focus() {
            edits.remove(&note_key);
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
            // A Crop's region is drawn on the image it crops.
            if item.name() == Some("Crop")
                && let Some(field) = fields.iter().find(|f| f.key == "region")
                && let Some(region) = parse_region(doc.text_of(&field.value))
                && let Some(from) = node_ports.iter().position(|p| p.label() == "from")
                && let Some((source, _)) = resolution.source.get(&(node, from))
                && let Some(made) = match source {
                    RNode::Step(i) => trace.steps.get(*i).and_then(Option::as_ref),
                    RNode::Output(_) => None,
                }
                && let (Some(image), Some(full)) = (&made.image, made.full)
            {
                kit::section(ui, "Region");
                ui.label(kit::dim("Drag on the image to choose what to keep."));
                let texture = thumb(ui.ctx(), thumbs, *revision, *source, image);
                if let Some(new) = region_picker(ui, &texture, full, region)
                    && new != region
                {
                    let (x, y, w, h) = new;
                    changes.push(Change::Replace {
                        span: field.value.span.clone(),
                        text: format!("(x: {x}, y: {y}, w: {w}, h: {h})"),
                        field: Some((node, "region".into())),
                    });
                }
            }
            // A Read's offset is picked in the ROM itself, its bytes highlighted.
            if item.name() == Some("Read")
                && let Some(rom) = rom.as_deref()
                && let Some(field) = fields.iter().find(|f| f.key == "offset")
                && let Some(offset) = parse_int(doc.text_of(&field.value).trim())
            {
                let size = fields
                    .iter()
                    .find(|f| f.key == "size")
                    .and_then(|f| {
                        let text = doc.text_of(&f.value).trim();
                        parse_int(text.strip_prefix("Some(")?.strip_suffix(')')?)
                    })
                    .unwrap_or(1);
                egui::CollapsingHeader::new("Pick the offset in the ROM")
                    .id_salt(("rom picker", node_key(node)))
                    .show(ui, |ui| {
                        let picked = rom_picker(ui, rom, offset as usize, size as usize, node);
                        if let Some(at) = picked.filter(|at| *at as i64 != offset) {
                            changes.push(Change::Replace {
                                span: field.value.span.clone(),
                                text: format!("{at:#x}"),
                                field: Some((node, "offset".into())),
                            });
                        }
                    });
            }
        }

        // Values the closed group shows, so the group can be changed without opening it.
        if in_group.is_some()
            && let Kind::Struct { fields, .. } = &item.kind
        {
            kit::section(ui, "Shown on its group");
            let exposed = nodes.get(&node).map_or(&[][..], |p| p.exposed.as_slice());
            for field in fields.iter().filter(|f| !names.contains(&f.key.as_str())) {
                let mut on = exposed.contains(&field.key);
                if ui.checkbox(&mut on, field.key.as_str()).changed() {
                    toggled = Some(field.key.clone());
                }
            }
        }
        if let Some(field) = toggled
            && let Some(place) = nodes.get_mut(&node)
        {
            match place.exposed.iter().position(|f| *f == field) {
                Some(i) => {
                    place.exposed.remove(i);
                }
                None => place.exposed.push(field),
            }
        }
        if let Some(group) = to_group {
            self.focus = None;
            self.focus_group = Some(group);
            return;
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

    /// A group in focus: its name, what is in it and what it reads and makes.
    fn group_inspector(&mut self, ui: &mut Ui, group: GroupId) {
        let members = groups::members(&self.groups, &self.nodes, group);
        let Some(info) = self.groups.get_mut(&group) else {
            return;
        };
        ui.label(kit::dim("Group"));
        ui.add(
            egui::TextEdit::singleline(&mut info.name)
                .font(theme::display(18.0))
                .desired_width(f32::INFINITY),
        );
        let open = info.open;
        let parent = info.parent;
        ui.label(kit::dim(format!(
            "{} node(s). Closed, it is drawn as one node with what it reads and makes; \
             open, as a frame around them.",
            members.len()
        )));
        ui.add_space(6.0);
        let mut set_open = None;
        let mut ungroup = false;
        let mut remove = false;
        ui.horizontal(|ui| {
            let label = if open { "Close" } else { "Open" };
            if ui
                .button(label)
                .on_hover_text(kit::tip(
                    "Show what is in it, or fold it back",
                    "Double-click",
                ))
                .clicked()
            {
                set_open = Some(!open);
            }
            if ui
                .button("Ungroup")
                .on_hover_text("Take the group away and keep what is in it")
                .clicked()
            {
                ungroup = true;
            }
            if ui
                .button("Remove its steps")
                .on_hover_text(kit::tip(
                    "Remove everything in it from the recipe",
                    "Delete",
                ))
                .clicked()
            {
                remove = true;
            }
        });
        let mut go = None;
        let mut to_group = None;
        if let Some(parent) = parent
            && let Some(name) = self.groups.get(&parent).map(|i| i.name.clone())
        {
            ui.horizontal(|ui| {
                ui.label(kit::dim("In the group"));
                if ui.link(name).clicked() {
                    to_group = Some(parent);
                }
            });
        }
        if let Some(pins) = self.pins.get(&group) {
            kit::section(ui, &format!("Reads · {}", pins.inputs.len()));
            for input in &pins.inputs {
                let (node, i) = input.targets[0];
                let name = &self.ports[&node][i].name;
                let text = if input.from.is_none() {
                    RichText::new(format!("{name} (missing)")).color(theme::ERROR)
                } else {
                    RichText::new(name)
                };
                if ui.link(text).clicked() {
                    go = Some(input.from.map_or(node, |(from, _)| from));
                }
            }
            kit::section(ui, &format!("Makes · {}", pins.outputs.len()));
            for (node, i) in &pins.outputs {
                if ui.link(&self.ports[node][*i].name).clicked() {
                    go = Some(*node);
                }
            }
        }
        kit::section(ui, "In it");
        for node in members.iter().take(200) {
            if ui.link(title(&self.doc, *node)).clicked() {
                go = Some(*node);
            }
        }
        if members.len() > 200 {
            ui.label(kit::dim(format!("and {} more", members.len() - 200)));
        }
        if let Some(open) = set_open {
            self.set_group_open(group, open);
        }
        if ungroup {
            self.ungroup(group);
            self.focus_group = None;
        }
        if remove && let Err(e) = self.apply(Change::Remove(members)) {
            self.message = Some(e);
        }
        if let Some(parent) = to_group {
            self.focus_group = Some(parent);
        }
        if let Some(node) = go {
            self.go_to(node);
        }
    }

    /// The Inspector with nothing selected: how to edit, then what is wrong.
    fn overview(&mut self, ui: &mut Ui, changes: &mut Vec<Change>) {
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
            ("Ctrl+G", "group the selected nodes"),
            ("Double-click a group", "open or close it"),
            ("Ctrl+F", "find a node"),
            ("Ctrl+Z", "undo"),
            ("Ctrl+Y", "redo"),
        ] {
            ui.horizontal(|ui| {
                kit::kbd(ui, keys);
                ui.label(kit::dim(what));
            });
        }
        kit::section(ui, &format!("Groups · {}", self.groups.len()));
        let mut regroup = None;
        let mut ungroup_all = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("By sections")
                .on_hover_text("A group for each section comment of the recipe")
                .clicked()
            {
                regroup = Some(false);
            }
            if ui
                .button("By outputs")
                .on_hover_text(
                    "A group for the steps that make each output (when it has no sections)",
                )
                .clicked()
            {
                regroup = Some(true);
            }
            if !self.groups.is_empty()
                && ui
                    .button("Ungroup all")
                    .on_hover_text("Take every group away")
                    .clicked()
            {
                ungroup_all = true;
            }
        });
        let mut pick = None;
        for (g, info) in self.groups.iter().filter(|(_, i)| i.parent.is_none()) {
            let count = groups::members(&self.groups, &self.nodes, *g).len();
            ui.horizontal(|ui| {
                if ui.link(&info.name).clicked() {
                    pick = Some(*g);
                }
                ui.label(kit::dim(format!("{count}")));
            });
        }
        if let Some(by_outputs) = regroup {
            self.ungroup_all();
            if self.group_automatically(by_outputs) == 0 {
                self.message = Some("The recipe has no section comments to group by".into());
            }
        }
        if ungroup_all {
            self.ungroup_all();
        }
        if let Some(g) = pick {
            self.focus_group = Some(g);
            self.frame_group(g);
        }

        self.run_order(ui, changes);

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

    /// The steps in the order they run, each dragged by its handle to run elsewhere (dropped
    /// on a step, it takes that step's place). A click goes to it.
    fn run_order(&mut self, ui: &mut Ui, changes: &mut Vec<Change>) {
        let steps = self.recipe.steps.len();
        kit::section(ui, &format!("Run order · {steps}"));
        ui.checkbox(&mut self.show_order, "Show the order on the graph")
            .on_hover_text("A numbered path from each step to the next");
        ui.label(kit::dim("Drag a step by ⠿ to make it run elsewhere."));
        let mut go = None;
        let mut moved = None;
        let row_h = 22.0;
        egui::ScrollArea::vertical()
            .id_salt("run order")
            .max_height(260.0)
            .show_rows(ui, row_h, steps, |ui, rows| {
                for i in rows {
                    let node = RNode::Step(i);
                    let row = ui
                        .horizontal(|ui| {
                            ui.set_min_height(row_h - 2.0);
                            ui.dnd_drag_source(egui::Id::new(("run order", i)), i, |ui| {
                                ui.label(RichText::new("⠿").color(theme::FAINT));
                            })
                            .response
                            .on_hover_cursor(egui::CursorIcon::Grab);
                            if ui.link(title(&self.doc, node)).clicked() {
                                go = Some(node);
                            }
                        })
                        .response;
                    if let Some(from) = row.dnd_release_payload::<usize>()
                        && *from != i
                    {
                        moved = Some((*from, i));
                    }
                    if row.dnd_hover_payload::<usize>().is_some() {
                        ui.painter().hline(
                            row.rect.x_range(),
                            row.rect.top(),
                            egui::Stroke::new(2.0, theme::BRASS_BRIGHT),
                        );
                    }
                }
            });
        if let Some((from, to)) = moved {
            changes.push(Change::MoveTo {
                node: RNode::Step(from),
                to,
            });
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
        let item_column = |node: RNode| match node {
            RNode::Step(_) => depth.get(&node).copied().unwrap_or(0),
            RNode::Output(_) => last,
        };
        // A closed group goes in the column of the earliest node in it.
        let column = |node: GNode| match node {
            GNode::Item(n) => item_column(n),
            GNode::Group(g) => groups::members(&self.groups, &self.nodes, g)
                .into_iter()
                .map(item_column)
                .min()
                .unwrap_or(0),
        };
        let first = |node: GNode| match node {
            GNode::Item(n) => order(n),
            GNode::Group(g) => groups::members(&self.groups, &self.nodes, g)
                .into_iter()
                .map(order)
                .min()
                .unwrap_or((0, 0)),
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
        let mut sorted: Vec<(GNode, NodeId)> =
            self.snarl.node_ids().map(|(id, n)| (*n, id)).collect();
        sorted.sort_by_key(|(n, _)| first(*n));
        let mut bottom = vec![0.0f32; last + 1];
        let mut placed = Vec::new();
        for (node, id) in sorted {
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

    /// Opens or closes a group (see [`RecipeGraph::open_groups`]).
    pub fn set_group_open(&mut self, group: GroupId, open: bool) {
        let mut places = self.places();
        self.open_groups(&[group], open, &mut places);
        self.refresh(places);
        if open {
            self.focus_group = None;
            self.frame_group(group);
        } else {
            self.focus = None;
            self.focus_group = Some(group);
        }
    }

    /// Opens or closes groups in `places`. A group opened for the first time puts what is
    /// in it where it was drawn (see [`groups::local_layout`]); opening one group also moves
    /// what is below it and to its right out of the way.
    fn open_groups(&mut self, open_these: &[GroupId], open: bool, places: &mut Places) {
        let shown = self.group_places();
        let depth = depths(&self.recipe, &self.resolution, &self.ports);
        let drawn: HashMap<GNode, Vec2> = self
            .snarl
            .node_ids()
            .filter_map(|(id, n)| Some((*n, *self.sizes.get(&id)?)))
            .collect();
        for g in open_these {
            if let Some(pos) = shown.get(g)
                && let Some(info) = self.groups.get_mut(g)
            {
                info.pos = *pos;
            }
            let Some(info) = self.groups.get_mut(g) else {
                continue;
            };
            info.open = open;
            if !open || info.laid_out {
                continue;
            }
            info.laid_out = true;
            let at = info.pos;
            // What is directly in it: its nodes and the groups in it, each as one part.
            let mut parts: Vec<(GNode, usize, (u8, usize))> = places
                .iter()
                .filter(|(_, p)| p.group == Some(*g))
                .map(|(n, _)| (GNode::Item(*n), part_column(*n, &depth), order(*n)))
                .collect();
            for (child, child_info) in &self.groups {
                if child_info.parent == Some(*g) {
                    let members = groups::members(&self.groups, places, *child);
                    let column = members
                        .iter()
                        .map(|n| part_column(*n, &depth))
                        .min()
                        .unwrap_or(0);
                    let first = members.iter().map(|n| order(*n)).min().unwrap_or((0, 0));
                    parts.push((GNode::Group(*child), column, first));
                }
            }
            parts.sort_by_key(|(_, _, first)| *first);
            let mut part_of: HashMap<RNode, usize> = HashMap::new();
            for (i, (part, _, _)) in parts.iter().enumerate() {
                match part {
                    GNode::Item(n) => {
                        part_of.insert(*n, i);
                    }
                    GNode::Group(c) => {
                        for n in groups::members(&self.groups, places, *c) {
                            part_of.insert(n, i);
                        }
                    }
                }
            }
            let links: Vec<(usize, usize)> = self
                .resolution
                .wires
                .iter()
                .filter_map(|((from, _), (to, _))| {
                    let (a, b) = (*part_of.get(from)?, *part_of.get(to)?);
                    (a != b).then_some((a, b))
                })
                .collect();
            let sized: Vec<(GNode, usize, Vec2)> = parts
                .iter()
                .map(|(part, column, _)| {
                    let size = drawn.get(part).copied().unwrap_or_else(|| match part {
                        GNode::Item(n) => self.estimated_size(*n),
                        GNode::Group(_) => vec2(260.0, 160.0),
                    });
                    (*part, *column, size)
                })
                .collect();
            let start = at + vec2(FRAME_MARGIN, FRAME_HEADER);
            let placed = groups::local_layout(&sized, &links, start);
            let mut frame = Rect::from_min_size(at, Vec2::ZERO);
            for (i, pos) in placed {
                let (part, _, size) = sized[i];
                frame = frame.union(Rect::from_min_size(pos, size));
                match part {
                    GNode::Item(n) => {
                        if let Some(place) = places.get_mut(&n) {
                            place.pos = pos;
                        }
                    }
                    GNode::Group(c) => {
                        if let Some(child) = self.groups.get_mut(&c) {
                            child.pos = pos;
                        }
                    }
                }
            }
            if open_these.len() == 1 {
                let closed = drawn
                    .get(&GNode::Group(*g))
                    .copied()
                    .unwrap_or(vec2(260.0, 160.0));
                let frame = frame.expand(FRAME_MARGIN);
                self.make_room(*g, Rect::from_min_size(at, closed), frame, places, &drawn);
            }
        }
    }

    /// Moves what is drawn below `closed` (the group as it was) or to its right, by how much
    /// bigger `open` (the group opened) is, so nothing is under it.
    fn make_room(
        &mut self,
        group: GroupId,
        closed: Rect,
        open: Rect,
        places: &mut Places,
        drawn: &HashMap<GNode, Vec2>,
    ) {
        let right = (open.max.x - closed.max.x).max(0.0);
        let down = (open.max.y - closed.max.y).max(0.0);
        let shift = |rect: Rect| {
            if rect.min.x >= closed.max.x - 1.0
                && rect.max.y > closed.min.y
                && rect.min.y < open.max.y
            {
                Some(vec2(right, 0.0))
            } else if rect.min.y >= closed.max.y - 1.0
                && rect.max.x > closed.min.x
                && rect.min.x < open.max.x
            {
                Some(vec2(0.0, down))
            } else {
                None
            }
        };
        let size = |node: GNode| drawn.get(&node).copied().unwrap_or(vec2(260.0, 120.0));
        for (node, place) in places.iter_mut() {
            let mine = groups::within(&self.groups, place.group, group);
            let is_drawn = groups::shown_as(&self.groups, &self.nodes, *node) == GNode::Item(*node);
            if mine || !is_drawn {
                continue;
            }
            if let Some(by) = shift(Rect::from_min_size(place.pos, size(GNode::Item(*node)))) {
                place.pos += by;
            }
        }
        let ids: Vec<(NodeId, GroupId)> = self
            .snarl
            .node_ids()
            .filter_map(|(id, n)| match n {
                GNode::Group(c) if *c != group => Some((id, *c)),
                _ => None,
            })
            .collect();
        for (id, c) in ids {
            if let Some(info) = self.snarl.get_node_info_mut(id)
                && let Some(by) = shift(Rect::from_min_size(info.pos, size(GNode::Group(c))))
            {
                info.pos += by;
            }
        }
    }

    /// About how big a node is drawn, before it is.
    fn estimated_size(&self, node: RNode) -> Vec2 {
        let rows = Self::node_doc(&self.doc, node).map_or(2, |item| match &item.kind {
            Kind::Struct { fields, .. } => fields.len(),
            _ => 2,
        });
        vec2(260.0, 70.0 + 30.0 * rows as f32)
    }

    /// Moves the view to an open group's nodes.
    fn frame_group(&mut self, group: GroupId) {
        let rect = self
            .snarl
            .nodes_pos_ids()
            .filter(|(_, _, node)| {
                if **node == GNode::Group(group) {
                    return true;
                }
                let container = match node {
                    GNode::Item(n) => self.nodes.get(n).and_then(|p| p.group),
                    GNode::Group(c) => self.groups.get(c).and_then(|i| i.parent),
                };
                groups::within(&self.groups, container, group)
            })
            .map(|(id, pos, _)| {
                Rect::from_min_size(
                    pos,
                    self.sizes.get(&id).copied().unwrap_or(vec2(260.0, 120.0)),
                )
            })
            .reduce(Rect::union);
        if let Some(rect) = rect {
            self.nav.show(rect);
        }
    }

    /// Puts nodes and closed groups drawn into a new closed group, inside the innermost group
    /// that holds them all.
    pub fn group(&mut self, selected: &[GNode], name: Option<String>) -> Option<GroupId> {
        if selected.is_empty() {
            return None;
        }
        let mut places = self.places();
        let shown = self.group_places();
        let container = |node: &GNode| match node {
            GNode::Item(n) => places.get(n).and_then(|p| p.group),
            GNode::Group(g) => self.groups.get(g).and_then(|i| i.parent),
        };
        let chains: Vec<Vec<GroupId>> = selected
            .iter()
            .map(|n| groups::ancestors(&self.groups, container(n)))
            .collect();
        let parent = chains[0]
            .iter()
            .find(|g| chains.iter().all(|c| c.contains(g)))
            .copied();
        let pos = selected
            .iter()
            .filter_map(|n| match n {
                GNode::Item(n) => places.get(n).map(|p| p.pos),
                GNode::Group(g) => shown.get(g).copied().or(self.groups.get(g).map(|i| i.pos)),
            })
            .reduce(|a, b| pos2(a.x.min(b.x), a.y.min(b.y)))
            .unwrap_or_default();
        let id = self.groups.keys().max().map_or(1, |m| m + 1);
        for node in selected {
            match node {
                GNode::Item(n) => {
                    if let Some(place) = places.get_mut(n) {
                        place.group = Some(id);
                    }
                }
                GNode::Group(g) => {
                    if let Some(info) = self.groups.get_mut(g) {
                        info.pos = shown.get(g).copied().unwrap_or(info.pos);
                        info.parent = Some(id);
                    }
                }
            }
        }
        self.groups.insert(
            id,
            GroupInfo {
                name: name.unwrap_or_else(|| format!("Group {id}")),
                parent,
                open: false,
                pos,
                laid_out: true,
            },
        );
        self.grouped = true;
        self.refresh(places);
        self.focus = None;
        self.focus_group = Some(id);
        Some(id)
    }

    /// Takes a group away; what was in it goes into the group it was in.
    pub fn ungroup(&mut self, group: GroupId) {
        let mut places = self.places();
        // Its nodes get places first, if it was never opened.
        self.open_groups(&[group], true, &mut places);
        let parent = self.groups.get(&group).and_then(|i| i.parent);
        for place in places.values_mut() {
            if place.group == Some(group) {
                place.group = parent;
            }
        }
        for info in self.groups.values_mut() {
            if info.parent == Some(group) {
                info.parent = parent;
            }
        }
        self.groups.remove(&group);
        self.grouped = true;
        self.refresh(places);
    }

    /// Takes every group away.
    pub fn ungroup_all(&mut self) {
        let mut places = self.places();
        let all: Vec<GroupId> = self.groups.keys().copied().collect();
        self.open_groups(&all, true, &mut places);
        for place in places.values_mut() {
            place.group = None;
        }
        self.groups.clear();
        self.grouped = true;
        self.refresh(places);
    }

    pub fn rename_group(&mut self, group: GroupId, name: &str) {
        if let Some(info) = self.groups.get_mut(&group) {
            info.name = name.to_string();
        }
    }

    /// Groups the recipe by its section comments, or (if `by_outputs` and it has none) by the
    /// output each step goes into. Returns how many groups were made.
    pub fn group_automatically(&mut self, by_outputs: bool) -> usize {
        let mut found: Vec<(String, Vec<RNode>)> = Vec::new();
        for (list, node) in [
            ("steps", RNode::Step as fn(usize) -> RNode),
            ("outputs", RNode::Output),
        ] {
            for (name, indices) in groups::sections(&self.doc, list) {
                found.push((name, indices.into_iter().map(node).collect()));
            }
        }
        // An output no section holds goes with the section that makes all it shows.
        let mut section_of: HashMap<RNode, usize> = HashMap::new();
        for (i, (_, nodes)) in found.iter().enumerate() {
            for n in nodes {
                section_of.insert(*n, i);
            }
        }
        for o in (0..self.recipe.outputs.len()).map(RNode::Output) {
            if section_of.contains_key(&o) {
                continue;
            }
            let from: HashSet<usize> = (0..self.ports[&o].len())
                .filter_map(|i| self.resolution.source.get(&(o, i)))
                .filter_map(|(n, _)| section_of.get(n).copied())
                .collect();
            if from.len() == 1
                && let Some(i) = from.into_iter().next()
            {
                found[i].1.push(o);
            }
        }
        if found.is_empty() && by_outputs {
            let ids: Vec<String> = self
                .recipe
                .outputs
                .iter()
                .map(|o| o.id().to_string())
                .collect();
            found = groups::by_outputs(&self.recipe, &self.resolution, &ids);
        }
        if found.is_empty() {
            return 0;
        }
        let mut places = self.places();
        let first = self.groups.keys().max().map_or(1, |m| m + 1);
        for (next, (name, nodes)) in (first..).zip(&found) {
            let pos = nodes
                .iter()
                .filter_map(|n| places.get(n).map(|p| p.pos))
                .reduce(|a, b| pos2(a.x.min(b.x), a.y.min(b.y)))
                .unwrap_or_default();
            self.groups.insert(
                next,
                GroupInfo {
                    name: name.clone(),
                    parent: None,
                    open: false,
                    pos,
                    laid_out: false,
                },
            );
            for n in nodes {
                if let Some(place) = places.get_mut(n) {
                    place.group = Some(next);
                }
            }
        }
        self.grouped = true;
        self.refresh(places);
        found.len()
    }

    /// The groups, for reports: (name, how many nodes, open).
    pub fn group_list(&self) -> Vec<(String, usize, bool)> {
        self.groups
            .iter()
            .map(|(g, info)| {
                (
                    info.name.clone(),
                    groups::members(&self.groups, &self.nodes, *g).len(),
                    info.open,
                )
            })
            .collect()
    }

    /// The id of the group of that name.
    pub fn group_named(&self, name: &str) -> Result<GroupId, String> {
        self.groups
            .iter()
            .find(|(_, info)| info.name == name)
            .map(|(g, _)| *g)
            .ok_or_else(|| format!("no group is named {name:?}"))
    }

    /// Puts nodes named as [`RecipeGraph::inspect`] names them in a new closed group.
    pub fn group_nodes(&mut self, nodes: &[String], name: &str) -> Result<(), String> {
        let nodes = nodes
            .iter()
            .map(|n| node_named(n).map(GNode::Item))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(n) = nodes.iter().find(|n| match n {
            GNode::Item(n) => !self.ports.contains_key(n),
            GNode::Group(_) => false,
        }) {
            return Err(format!("the recipe has no {n:?}"));
        }
        self.group(&nodes, Some(name.to_string()))
            .map(|_| ())
            .ok_or_else(|| "nothing to group".into())
    }
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

#[cfg(test)]
mod tests {
    use clockwork_core::{
        recipe::{Output, Step},
        recipe_schema,
    };

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

    #[test]
    fn a_change_runs_from_the_last_point_kept_before_it() {
        let steps: Vec<String> = (0..40)
            .map(|i| format!("Fill(name: \"b{i}\", size: {}),", i + 1))
            .collect();
        let text = format!("(steps: [{}], outputs: [])", steps.join("\n"));
        let rom = Some(Arc::new(vec![0; 16]));
        let mut graph = RecipeGraph::new(text.clone(), &RecipeLayout::default(), rom).unwrap();
        graph.run_trace();
        let marks: Vec<usize> = graph.trace.kept.marks.iter().map(|(n, _)| *n).collect();
        assert_eq!(marks, [16, 32]);
        let first = graph.trace.kept.marks[0].1.clone();

        // Step 20 changes: what was kept after 16 steps is gone on from, not made again.
        graph.recipe.steps[20] = ron::from_str("Fill(name: \"b20\", size: 99)").unwrap();
        graph.run_trace();
        assert!(Arc::ptr_eq(&graph.trace.kept.marks[0].1, &first));
        let lines = |g: &RecipeGraph, i: usize| g.trace.steps[i].as_ref().unwrap().lines.clone();
        assert_eq!(lines(&graph, 20), ["b20: 99 bytes"]);
        assert_eq!(lines(&graph, 3), ["b3: 4 bytes"]);
        assert_eq!(lines(&graph, 39), ["b39: 40 bytes"]);
        assert_eq!(graph.trace.kept.marks.len(), 2);
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
        for (name, template) in templates(false) {
            let output: Output = ron::from_str(template).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(output.name(), *name);
        }
    }

    #[test]
    fn regions_are_read_from_their_text() {
        assert_eq!(
            parse_region("(x: 0, y: 0x10, w: 16, h: 8)"),
            Some((0, 16, 16, 8))
        );
        assert_eq!(parse_region("(x: 1, y: 2, w: 3)"), None);
        assert_eq!(parse_region("Some(1)"), None);
    }

    #[test]
    fn every_kind_of_output_can_be_added() {
        let missing: Vec<&str> = recipe_schema::OUTPUTS
            .iter()
            .map(|i| i.name)
            .filter(|n| !templates(false).iter().any(|(t, _)| t == n))
            .collect();
        assert_eq!(missing, Vec::<&str>::new());
        assert_eq!(snake("InputMap"), "input_map");
    }

    #[test]
    fn a_closed_group_is_one_node_with_what_crosses_its_edge() {
        let mut graph = graph();
        let items = [0, 1, 4].map(|i| GNode::Item(RNode::Step(i)));
        let group = graph.group(&items, Some("Sheet".into())).unwrap();
        // Two steps and the output stay, and the group stands for the three.
        assert_eq!(graph.snarl.node_ids().count(), 4);
        assert!(
            graph
                .snarl
                .node_ids()
                .any(|(_, n)| *n == GNode::Group(group))
        );
        let pins = &graph.pins[&group];
        // It reads the palette from outside and makes the sheet the output uses.
        assert_eq!(pins.inputs.len(), 1);
        assert_eq!(pins.inputs[0].from.map(|(n, _)| n), Some(RNode::Step(3)));
        assert_eq!(pins.outputs.len(), 1);
        assert_eq!(pins.outputs[0].0, RNode::Step(4));
        // The palette into its bank, the bank into the group, the group into the output.
        assert_eq!(graph.snarl.wires().count(), 3);

        graph.set_group_open(group, true);
        assert_eq!(graph.snarl.node_ids().count(), 6);
        assert_eq!(graph.snarl.wires().count(), graph.resolution.wires.len());

        graph.ungroup(group);
        assert!(graph.groups.is_empty());
        assert!(graph.nodes.values().all(|p| p.group.is_none()));
    }

    #[test]
    fn groups_come_back_with_their_nodes_after_steps_are_added_before_them() {
        let mut graph = graph();
        let items = [3, 4].map(|i| GNode::Item(RNode::Step(i)));
        graph.group(&items, Some("Sheet".into())).unwrap();
        let layout = graph.layout();
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].members.len(), 2);

        // A step put first moves every index; the keys still find the group's nodes.
        let text = RECIPE.replace(
            "        // Tiles and palettes.",
            "        Fill(name: \"pad\", size: 1),\n        // Tiles and palettes.",
        );
        let again = RecipeGraph::new(text, &layout, None).unwrap();
        let (id, _) = again.groups.iter().next().unwrap();
        let mut members = groups::members(&again.groups, &again.nodes, *id);
        members.sort_by_key(|n| order(*n));
        assert_eq!(members, vec![RNode::Step(4), RNode::Step(5)]);
        assert!(again.nodes[&RNode::Step(0)].group.is_none());
    }

    #[test]
    fn an_opened_group_is_a_grid_and_what_was_below_it_moves_down() {
        let mut text = String::from("Recipe(steps: [\n");
        for i in 0..30 {
            if i % 10 == 0 {
                text.push_str(&format!("// == S{i} ==\n"));
            }
            text.push_str(&format!("Fill(name: \"s{i}\", size: 1),\n"));
        }
        text.push_str("], outputs: [])");
        let mut graph = RecipeGraph::new(text, &RecipeLayout::default(), None).unwrap();
        assert_eq!(graph.group_automatically(false), 3);
        graph.arrange();
        let group = graph.group_named("S10").unwrap();
        graph.set_group_open(group, true);
        let rects: Vec<Rect> = graph
            .snarl
            .nodes_pos_ids()
            .map(|(_, pos, node)| {
                let size = match node {
                    GNode::Item(n) => graph.estimated_size(*n),
                    GNode::Group(_) => vec2(260.0, 120.0),
                };
                Rect::from_min_size(pos, size)
            })
            .collect();
        // Ten unwired steps open as rows of several, not one column of ten.
        let xs: HashSet<i32> = rects.iter().map(|r| r.min.x as i32).collect();
        assert!(xs.len() > 2);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn undoing_a_removal_gives_every_node_its_own_group_back() {
        let mut graph = graph();
        let items = [3, 4].map(|i| GNode::Item(RNode::Step(i)));
        let group = graph.group(&items, Some("Sheet".into())).unwrap();
        graph.apply(Change::Remove(vec![RNode::Step(0)])).unwrap();
        graph.undo();
        assert_eq!(graph.recipe.steps.len(), 5);
        assert!(graph.nodes[&RNode::Step(2)].group.is_none());
        assert_eq!(graph.nodes[&RNode::Step(3)].group, Some(group));
        assert_eq!(graph.nodes[&RNode::Step(4)].group, Some(group));
    }

    #[test]
    fn a_block_comes_in_as_a_closed_group_that_shows_its_offsets() {
        let mut graph = graph();
        let (steps, outputs) = (graph.recipe.steps.len(), graph.recipe.outputs.len());
        graph.edit(&Edit::AddBlock("GBA tileset".into())).unwrap();
        let block = &blocks::BLOCKS[0];
        assert_eq!(graph.recipe.steps.len(), steps + block.steps.len());
        assert_eq!(graph.recipe.outputs.len(), outputs + block.outputs.len());
        let group = graph.group_named("GBA tileset").unwrap();
        assert!(!graph.groups[&group].open);
        assert!(
            graph
                .snarl
                .node_ids()
                .any(|(_, n)| *n == GNode::Group(group))
        );
        // It reads nothing from outside: everything it uses, it makes.
        assert!(graph.pins.get(&group).is_none_or(|p| p.inputs.is_empty()));
        assert!(graph.resolution.missing.is_empty());
        assert_eq!(graph.nodes[&RNode::Step(steps)].exposed, vec!["offset"]);
        // Again: new names, and a second group.
        graph.edit(&Edit::AddBlock("GBA tileset".into())).unwrap();
        assert!(graph.group_named("GBA tileset 2").is_ok());
        assert!(graph.resolution.missing.is_empty());
        assert!(graph.text().contains("\"tileset_2\""));
    }

    #[test]
    fn a_note_is_the_comment_above_a_step_and_typing_is_undone_at_once() {
        let mut graph = graph();
        let item = RecipeGraph::node_doc(&graph.doc, RNode::Step(0)).unwrap();
        assert_eq!(graph.doc.note(item).1, "Tiles and palettes.");
        graph
            .edit(&Edit::Note("step 1".into(), "The map.".into()))
            .unwrap();
        assert!(
            graph
                .text()
                .contains("        // The map.\n        Fill(name: \"map\"")
        );

        let before = graph.text().to_string();
        let typed = before.replace("size: 8", "size: 9");
        graph.edit(&Edit::Text(typed.clone())).unwrap();
        graph
            .edit(&Edit::Text(typed.replace("size: 9", "size: 10")))
            .unwrap();
        assert!(graph.text().contains("size: 10"));
        assert!(graph.edit(&Edit::Text("Recipe(steps: [".into())).is_err());
        graph.undo();
        assert_eq!(graph.text(), before);
    }

    #[test]
    fn layouts_saved_by_index_still_load() {
        let layout = RecipeLayout {
            steps: (0..5)
                .map(|i| ("Fill".to_string(), (i as f32 * 10.0, 0.0)))
                .collect(),
            ..RecipeLayout::default()
        };
        let graph = RecipeGraph::new(RECIPE.into(), &layout, None).unwrap();
        assert_eq!(graph.nodes[&RNode::Step(1)].pos, pos2(10.0, 0.0));
    }

    #[test]
    fn big_recipes_are_grouped_by_what_they_make() {
        let mut text = String::from("Recipe(steps: [\n");
        for i in 0..groups::AUTO_GROUP_BY_OUTPUTS {
            text.push_str(&format!("Fill(name: \"s{i}\", size: 1),\n"));
        }
        text.push_str("], outputs: [\n");
        for i in 0..groups::AUTO_GROUP_BY_OUTPUTS {
            text.push_str(&format!("Image(id: \"o{i}\", from: \"s{i}\"),\n"));
        }
        text.push_str("])");
        let graph = RecipeGraph::new(text, &RecipeLayout::default(), None).unwrap();
        assert!(!graph.groups.is_empty());
        assert!(graph.snarl.node_ids().count() < 2 * groups::AUTO_GROUP_BY_OUTPUTS);
    }

    #[test]
    fn the_layout_comes_back_by_place_and_variant() {
        let mut graph = graph();
        let id = graph
            .snarl
            .node_ids()
            .find(|(_, n)| **n == GNode::Item(RNode::Step(2)))
            .unwrap()
            .0;
        graph.snarl.get_node_info_mut(id).unwrap().pos = pos2(10.0, 20.0);
        let layout = graph.layout();
        let key = layout
            .nodes
            .iter()
            .find(|(_, at)| **at == (10.0, 20.0))
            .map(|(key, _)| key.clone());
        assert!(key.is_some_and(|k| k.starts_with("Literal:")));
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
        for step in [true, false] {
            for (name, template) in templates(step) {
                let info = if step {
                    recipe_schema::step(name)
                } else {
                    recipe_schema::output(name)
                }
                .unwrap_or_else(|| panic!("{name} isn't described"));
                let doc = Doc::parse(template.as_str()).unwrap();
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
        places.insert(RNode::Step(1), Place::at(Pos2::ZERO));
        places.insert(RNode::Output(0), Place::at(pos2(1.0, 1.0)));
        let places = remap(places, |n| Some(shifted(n, RNode::Step(0))));
        assert_eq!(places[&RNode::Step(2)], Place::at(Pos2::ZERO));
        assert!(places.contains_key(&RNode::Output(0)));
    }

    #[test]
    fn wires_are_looked_up_by_input_and_lineage_follows_them() {
        let graph = graph();
        let resolution = &graph.resolution;
        assert_eq!(resolution.source.len(), resolution.wires.len());
        for (from, to) in &resolution.wires {
            assert_eq!(resolution.source.get(to), Some(from));
            assert!(resolution.upstream[&to.0].contains(&from.0));
            assert!(resolution.downstream[&from.0].contains(&to.0));
        }
        // A node's lineage is what feeds it, what it feeds, and nothing else.
        let ((from, _), (to, _)) = resolution.wires[0];
        let lineage = lineage(resolution, to);
        assert!(lineage.contains(&from) && lineage.contains(&to));
        for node in &lineage {
            assert!(
                *node == to
                    || lineage_reaches(resolution, *node, to)
                    || lineage_reaches(resolution, to, *node)
            );
        }
    }

    fn lineage_reaches(resolution: &Resolution, from: RNode, to: RNode) -> bool {
        let mut stack = vec![from];
        let mut seen = HashSet::new();
        while let Some(at) = stack.pop() {
            if at == to {
                return true;
            }
            for ((a, _), (b, _)) in &resolution.wires {
                if *a == at && seen.insert(*b) {
                    stack.push(*b);
                }
            }
        }
        false
    }
}

//! A composition plugin drawn as a node graph: characters and abilities of other plugins on the
//! left, wired through Learn nodes (what plays for each of an ability's roles) into the composed
//! characters of this plugin on the right.
//!
//! The graph is a view: `compose.ron` stays the plugin's content, and only where the nodes sit
//! (plus characters and abilities not wired yet, which `compose.ron` has no place for) is kept in
//! `graph.ron` next to it.

use std::path::Path;
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
};

use bevy_egui::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2, pos2, vec2};
use clockwork_core::{
    ability::RoleBinding,
    compose::{AbilityUse, ComposedCharacter, Composition, OutputRef, Tuning},
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
    kit,
    plugin_editor::{AbilityChoice, CharacterChoice, binding_label, valid_name},
    theme,
};

/// Where the nodes of a composition plugin sit, next to its `compose.ron`.
pub const LAYOUT_FILE: &str = "graph.ron";

/// Distance between the graph's columns and rows before nodes are arranged by their sizes.
const COLUMN: f32 = 300.0;
const ROW: f32 = 150.0;
/// Space between arranged nodes.
const GAP: f32 = 40.0;

#[derive(Clone, Debug, PartialEq)]
pub enum GraphNode {
    /// A playable character of another plugin.
    Character(OutputRef),
    /// An ability of another plugin.
    Ability(OutputRef),
    /// An ability learned by the character it is wired into, and what plays for each role.
    Learn {
        roles: BTreeMap<String, RoleBinding>,
        use_defaults: bool,
        /// `use_defaults` follows the wiring (on for a character from the ability's own plugin)
        /// until it is set by hand.
        auto_defaults: bool,
    },
    /// A character of this plugin: its base on input 0, learned abilities on the inputs after.
    Composed {
        /// Keeps the characters' order in `compose.ron`.
        serial: u64,
        id: String,
        /// Whether the id was chosen (otherwise it follows the base character).
        named: bool,
        tuning: Tuning,
        hand: Option<String>,
        /// Wired ability inputs; one more stays free for the next.
        abilities: usize,
    },
}

/// What a wire carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Character,
    Ability,
    Learned,
}

impl Kind {
    fn color(self) -> Color32 {
        match self {
            Kind::Character => theme::BRASS_BRIGHT,
            Kind::Ability => Color32::from_rgb(0x6F, 0xB3, 0xA2),
            Kind::Learned => Color32::from_rgb(0xD0, 0x84, 0x5A),
        }
    }
}

fn output_kind(node: &GraphNode) -> Option<Kind> {
    match node {
        GraphNode::Character(_) => Some(Kind::Character),
        GraphNode::Ability(_) => Some(Kind::Ability),
        GraphNode::Learn { .. } => Some(Kind::Learned),
        GraphNode::Composed { .. } => None,
    }
}

fn input_kind(node: &GraphNode, input: usize) -> Option<Kind> {
    match node {
        GraphNode::Learn { .. } if input == 0 => Some(Kind::Ability),
        GraphNode::Composed { .. } if input == 0 => Some(Kind::Character),
        GraphNode::Composed { abilities, .. } if input <= abilities + 1 => Some(Kind::Learned),
        _ => None,
    }
}

/// Where nodes sit, saved as `graph.ron`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// Node positions by what the node stands for (see [`node_key`]).
    #[serde(default)]
    pub positions: BTreeMap<String, (f32, f32)>,
    /// Characters and abilities placed but not wired to anything yet.
    #[serde(default)]
    pub loose: Vec<Loose>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Loose {
    Character(OutputRef, (f32, f32)),
    Ability(OutputRef, (f32, f32)),
}

/// Reads a plugin folder's layout; a missing or unreadable one just means nodes get placed anew.
pub fn load_layout(dir: &Path) -> Layout {
    std::fs::read_to_string(dir.join(LAYOUT_FILE))
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_layout(dir: &Path, layout: &Layout) -> Result<(), String> {
    let config = ron::ser::PrettyConfig::new().depth_limit(3);
    let body = ron::ser::to_string_pretty(layout, config).map_err(|e| e.to_string())?;
    let text = format!(
        "// Where the plugin editor draws this composition's nodes. The plugin itself is compose.ron.\n{body}\n"
    );
    clockwork_library::files::write_atomic(dir.join(LAYOUT_FILE), text).map_err(|e| e.to_string())
}

/// The composition a graph stands for, and what keeps it from being saved as drawn.
#[derive(Debug, Default, PartialEq)]
pub struct Built {
    pub composition: Composition,
    /// Problems that block saving.
    pub errors: Vec<String>,
    /// Parts of the graph that aren't saved.
    pub notes: Vec<String>,
}

/// What happened while the graph was drawn.
pub struct Shown {
    /// A composed character asked to be placed in the scene, by id.
    pub place: Option<String>,
    /// The nodes were arranged by themselves (a graph without a saved layout).
    pub arranged: bool,
}

pub struct CompositionGraph {
    pub snarl: Snarl<GraphNode>,
    next_serial: u64,
    /// Node sizes as last drawn, for arranging.
    sizes: HashMap<NodeId, Vec2>,
    /// Arrange the nodes once their sizes are known (no saved layout).
    arrange_pending: bool,
    /// The graph as of the last finished change, and the ones before and undone.
    settled: Snarl<GraphNode>,
    undo: Vec<Snarl<GraphNode>>,
    redo: Vec<Snarl<GraphNode>>,
    /// Finding nodes and moving the view.
    pub nav: Navigator,
}

/// Changes kept for undo.
const UNDO_LIMIT: usize = 100;

/// Every node (with where it sits) and wire of a graph.
type Fingerprint = (
    Vec<(NodeId, GraphNode, (i32, i32))>,
    Vec<(OutPinId, InPinId)>,
);

/// What a graph looks like, to tell whether it changed.
fn fingerprint(snarl: &Snarl<GraphNode>) -> Fingerprint {
    let mut nodes: Vec<_> = snarl
        .nodes_pos_ids()
        .map(|(id, pos, n)| (id, n.clone(), (pos.x.round() as i32, pos.y.round() as i32)))
        .collect();
    nodes.sort_by_key(|(id, ..)| *id);
    let mut wires: Vec<_> = snarl.wires().collect();
    wires.sort();
    (nodes, wires)
}

fn at((x, y): (f32, f32)) -> Pos2 {
    pos2(x, y)
}

fn ref_key(r: &OutputRef) -> String {
    format!("{}/{}", r.plugin, r.output)
}

/// The node wired into an input, if any.
fn source(snarl: &Snarl<GraphNode>, node: NodeId, input: usize) -> Option<NodeId> {
    snarl
        .in_pin(InPinId { node, input })
        .remotes
        .first()
        .map(|o| o.node)
}

/// The nodes an output is wired into.
fn targets(snarl: &Snarl<GraphNode>, node: NodeId) -> Vec<InPinId> {
    snarl.out_pin(OutPinId { node, output: 0 }).remotes
}

fn character_of(snarl: &Snarl<GraphNode>, node: Option<NodeId>) -> Option<&OutputRef> {
    match node.map(|n| &snarl[n]) {
        Some(GraphNode::Character(r)) => Some(r),
        _ => None,
    }
}

fn ability_of(snarl: &Snarl<GraphNode>, node: Option<NodeId>) -> Option<&OutputRef> {
    match node.map(|n| &snarl[n]) {
        Some(GraphNode::Ability(r)) => Some(r),
        _ => None,
    }
}

/// The base character wired into a composed character.
fn base_of(snarl: &Snarl<GraphNode>, composed: NodeId) -> Option<&OutputRef> {
    character_of(snarl, source(snarl, composed, 0))
}

/// The composed character a Learn node is wired into.
fn learner_of(snarl: &Snarl<GraphNode>, learn: NodeId) -> Option<NodeId> {
    targets(snarl, learn).first().map(|pin| pin.node)
}

fn composed_ids(snarl: &Snarl<GraphNode>) -> Vec<String> {
    snarl
        .node_ids()
        .filter_map(|(_, n)| match n {
            GraphNode::Composed { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// A composed character id like `hero_plus` that isn't taken yet.
pub fn unique_id(base: &str, taken: &[String]) -> String {
    let mut id = format!("{base}_plus");
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}_plus_{n}");
        n += 1;
    }
    id
}

/// What a typed name is kept as: lowercase, with what a name can't hold as `_`.
fn name_from(text: &str) -> String {
    text.trim()
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' => c,
            _ => '_',
        })
        .collect()
}

/// Connects two pins if they carry the same kind of thing. An input takes one wire, and a Learn
/// node teaches one character, so wiring them again replaces the old wire.
fn wire(snarl: &mut Snarl<GraphNode>, from: OutPinId, to: InPinId) -> bool {
    let kind = output_kind(&snarl[from.node]);
    if from.node == to.node || kind.is_none() || kind != input_kind(&snarl[to.node], to.input) {
        return false;
    }
    snarl.drop_inputs(to);
    if kind == Some(Kind::Learned) {
        snarl.drop_outputs(from);
    }
    snarl.connect(from, to);
    // A composed character not named yet is named after its base.
    if let GraphNode::Character(base) = &snarl[from.node] {
        let base = base.output.clone();
        let taken = composed_ids(snarl);
        if let GraphNode::Composed { id, named, .. } = &mut snarl[to.node]
            && !*named
        {
            *id = unique_id(&base, &taken);
        }
    }
    true
}

impl CompositionGraph {
    /// The graph of a composition, with nodes where `layout` has them.
    pub fn new(composition: &Composition, layout: &Layout) -> Self {
        let mut snarl = Snarl::new();
        let unplaced = Cell::new(false);
        let place = |key: String, column: f32, y: f32| match layout.positions.get(&key) {
            Some(p) => at(*p),
            None => {
                unplaced.set(true);
                pos2(column * COLUMN, y)
            }
        };
        let mut characters: HashMap<OutputRef, NodeId> = HashMap::new();
        let mut abilities: HashMap<OutputRef, NodeId> = HashMap::new();
        let mut y = 0.0;
        for (serial, c) in composition.characters.iter().enumerate() {
            let composed = snarl.insert_node(
                place(format!("character/{}", c.id), 2.0, y),
                GraphNode::Composed {
                    serial: serial as u64,
                    id: c.id.clone(),
                    named: true,
                    tuning: c.tuning,
                    hand: c.hand.clone(),
                    abilities: c.abilities.len(),
                },
            );
            let base = *characters.entry(c.base.clone()).or_insert_with(|| {
                snarl.insert_node(
                    place(format!("base/{}", ref_key(&c.base)), 0.0, y),
                    GraphNode::Character(c.base.clone()),
                )
            });
            snarl.connect(
                OutPinId {
                    node: base,
                    output: 0,
                },
                InPinId {
                    node: composed,
                    input: 0,
                },
            );
            for (i, learned) in c.abilities.iter().enumerate() {
                let row = y + (i + 1) as f32 * ROW;
                let ability = *abilities.entry(learned.ability.clone()).or_insert_with(|| {
                    snarl.insert_node(
                        place(format!("ability/{}", ref_key(&learned.ability)), 0.0, row),
                        GraphNode::Ability(learned.ability.clone()),
                    )
                });
                let learn = snarl.insert_node(
                    place(
                        format!("learn/{}/{}", c.id, ref_key(&learned.ability)),
                        1.0,
                        row,
                    ),
                    GraphNode::Learn {
                        roles: learned.roles.clone(),
                        use_defaults: learned.use_defaults,
                        auto_defaults: false,
                    },
                );
                snarl.connect(
                    OutPinId {
                        node: ability,
                        output: 0,
                    },
                    InPinId {
                        node: learn,
                        input: 0,
                    },
                );
                snarl.connect(
                    OutPinId {
                        node: learn,
                        output: 0,
                    },
                    InPinId {
                        node: composed,
                        input: i + 1,
                    },
                );
            }
            y += (c.abilities.len() + 2) as f32 * ROW;
        }
        for loose in &layout.loose {
            match loose {
                Loose::Character(r, p) => {
                    snarl.insert_node(at(*p), GraphNode::Character(r.clone()))
                }
                Loose::Ability(r, p) => snarl.insert_node(at(*p), GraphNode::Ability(r.clone())),
            };
        }
        let mut nav = Navigator::default();
        nav.open_view();
        Self {
            settled: snarl.clone(),
            snarl,
            next_serial: composition.characters.len() as u64,
            sizes: HashMap::new(),
            arrange_pending: unplaced.get() && layout.positions.is_empty(),
            undo: Vec::new(),
            redo: Vec::new(),
            nav,
        }
    }

    /// Adds a composed character with no base yet.
    #[cfg(test)]
    fn add_composed(&mut self, pos: Pos2) -> NodeId {
        add_composed(&mut self.snarl, &mut self.next_serial, pos)
    }

    /// Draws the graph for editing.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        id_salt: &str,
        characters: &[CharacterChoice],
        abilities: &[AbilityChoice],
        can_place: bool,
    ) -> Shown {
        self.nav.update(
            &self.snarl,
            &self.sizes,
            ui.available_rect_before_wrap(),
            |_, node| match node {
                GraphNode::Character(r) | GraphNode::Ability(r) => {
                    format!("{} {}", r.output, r.plugin)
                }
                GraphNode::Learn { roles, .. } => {
                    let roles: Vec<&str> = roles.keys().map(String::as_str).collect();
                    format!("learn {}", roles.join(" "))
                }
                GraphNode::Composed { id, .. } => id.clone(),
            },
        );
        let mut viewer = Viewer {
            nav: &mut self.nav,
            characters,
            abilities,
            next_serial: &mut self.next_serial,
            can_place,
            place: None,
            sizes: HashMap::new(),
        };
        let widget = egui_snarl::ui::SnarlWidget::new()
            .id_salt(("composition_graph", id_salt))
            .style(style())
            .min_size(ui.available_size());
        widget.show(&mut self.snarl, &mut viewer, ui);
        let place = viewer.place.take();
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            for node in widget.get_selected_nodes(ui) {
                if self.snarl.get_node(node).is_some() {
                    self.snarl.remove_node(node);
                }
            }
        }
        // Nodes settle into their size over a few frames: arrange once no size changes.
        let settled = viewer.sizes == self.sizes
            && self
                .snarl
                .node_ids()
                .all(|(node, _)| self.sizes.contains_key(&node));
        self.sizes = viewer.sizes;
        self.normalize();
        let arranged = self.arrange_pending && settled;
        if arranged {
            self.arrange();
            self.arrange_pending = false;
            self.settled = self.snarl.clone();
            self.nav.open_view();
        }
        if self.arrange_pending {
            ui.ctx().request_repaint();
        }
        // A change becomes an undo step once nothing is dragged or typed.
        let busy = ui.input(|i| i.pointer.any_down()) || ui.ctx().egui_wants_keyboard_input();
        if !busy && !self.arrange_pending {
            self.track();
        }
        Shown { place, arranged }
    }

    fn track(&mut self) {
        if fingerprint(&self.snarl) != fingerprint(&self.settled) {
            let before = std::mem::replace(&mut self.settled, self.snarl.clone());
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.track();
        if let Some(snarl) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.snarl, snarl));
            self.settled = self.snarl.clone();
        }
    }

    pub fn redo(&mut self) {
        if let Some(snarl) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.snarl, snarl));
            self.settled = self.snarl.clone();
        }
    }

    /// Lays the nodes out in three columns (characters and abilities, Learn nodes, this plugin's
    /// characters), each composed character level with what is wired into it.
    pub fn arrange(&mut self) {
        let snarl = &self.snarl;
        let size = |node: NodeId| self.sizes.get(&node).copied().unwrap_or(vec2(240.0, 120.0));
        let column = |n: &GraphNode| match n {
            GraphNode::Character(_) | GraphNode::Ability(_) => 0,
            GraphNode::Learn { .. } => 1,
            GraphNode::Composed { .. } => 2,
        };
        let mut width = [0.0f32; 3];
        for (node, n) in snarl.node_ids() {
            width[column(n)] = width[column(n)].max(size(node).x);
        }
        let x = [0.0, width[0] + GAP * 2.0, width[0] + width[1] + GAP * 4.0];
        let mut bottom = [0.0f32; 3];
        let mut placed: HashMap<NodeId, Pos2> = HashMap::new();
        // Puts a node in a column at `y` or below what is already there, once.
        let put = |node: NodeId,
                   col: usize,
                   y: f32,
                   placed: &mut HashMap<NodeId, Pos2>,
                   bottom: &mut [f32; 3]| {
            if placed.contains_key(&node) {
                return;
            }
            let y = y.max(bottom[col]);
            placed.insert(node, pos2(x[col], y));
            bottom[col] = y + size(node).y + GAP;
        };

        let mut composed: Vec<(u64, NodeId)> = snarl
            .node_ids()
            .filter_map(|(node, n)| match n {
                GraphNode::Composed { serial, .. } => Some((*serial, node)),
                _ => None,
            })
            .collect();
        composed.sort();
        for (_, node) in composed {
            let top = bottom.iter().copied().fold(0.0, f32::max);
            put(node, 2, top, &mut placed, &mut bottom);
            if let Some(base) = source(snarl, node, 0) {
                put(base, 0, top, &mut placed, &mut bottom);
            }
            let GraphNode::Composed { abilities, .. } = &snarl[node] else {
                continue;
            };
            for input in 1..=*abilities {
                let Some(learn) = source(snarl, node, input) else {
                    continue;
                };
                put(learn, 1, top, &mut placed, &mut bottom);
                let learn_y = placed[&learn].y;
                if let Some(ability) = source(snarl, learn, 0) {
                    put(ability, 0, learn_y, &mut placed, &mut bottom);
                }
            }
        }
        // Whatever isn't wired into a character goes below.
        let top = bottom.iter().copied().fold(0.0, f32::max);
        for (node, n) in snarl.node_ids() {
            put(node, column(n), top, &mut placed, &mut bottom);
        }
        for (node, pos) in placed {
            if let Some(info) = self.snarl.get_node_info_mut(node) {
                info.pos = pos;
            }
        }
    }

    /// The composition the graph stands for.
    pub fn build(&self) -> Built {
        let snarl = &self.snarl;
        let mut built = Built::default();
        let mut composed: Vec<(u64, NodeId)> = snarl
            .node_ids()
            .filter_map(|(node, n)| match n {
                GraphNode::Composed { serial, .. } => Some((*serial, node)),
                _ => None,
            })
            .collect();
        composed.sort();
        for (_, node) in composed {
            let GraphNode::Composed {
                id,
                tuning,
                hand,
                abilities,
                ..
            } = &snarl[node]
            else {
                continue;
            };
            if built.composition.characters.iter().any(|c| c.id == *id) {
                built.errors.push(format!("Two characters are named {id}"));
                continue;
            }
            let Some(base) = base_of(snarl, node) else {
                built
                    .errors
                    .push(format!("{id} needs a base character: wire one into it"));
                continue;
            };
            let mut learned: Vec<AbilityUse> = Vec::new();
            for input in 1..=*abilities {
                let Some(learn) = source(snarl, node, input) else {
                    continue;
                };
                let GraphNode::Learn {
                    roles,
                    use_defaults,
                    ..
                } = &snarl[learn]
                else {
                    continue;
                };
                let Some(ability) = ability_of(snarl, source(snarl, learn, 0)) else {
                    built.notes.push(format!(
                        "A Learn node wired into {id} has no ability, so it isn't saved"
                    ));
                    continue;
                };
                if learned.iter().any(|a| a.ability == *ability) {
                    built.notes.push(format!(
                        "{id} learns {} twice; only the first is saved",
                        ability.output
                    ));
                    continue;
                }
                learned.push(AbilityUse {
                    ability: ability.clone(),
                    roles: roles.clone(),
                    use_defaults: *use_defaults,
                });
            }
            built.composition.characters.push(ComposedCharacter {
                id: id.clone(),
                base: base.clone(),
                abilities: learned,
                tuning: *tuning,
                hand: hand.clone(),
            });
        }
        let stray = snarl
            .node_ids()
            .filter(|(node, n)| {
                matches!(n, GraphNode::Learn { .. }) && learner_of(snarl, *node).is_none()
            })
            .count();
        if stray > 0 {
            built.notes.push(format!(
                "{stray} Learn node(s) not wired into a character aren't saved"
            ));
        }
        built
    }

    /// Where the nodes sit, to save next to the composition.
    pub fn layout(&self) -> Layout {
        let snarl = &self.snarl;
        let mut layout = Layout::default();
        for (node, pos, n) in snarl.nodes_pos_ids() {
            let p = (pos.x.round(), pos.y.round());
            let wired = !targets(snarl, node).is_empty();
            match (n, wired) {
                (GraphNode::Character(r), false) => {
                    layout.loose.push(Loose::Character(r.clone(), p))
                }
                (GraphNode::Ability(r), false) => layout.loose.push(Loose::Ability(r.clone(), p)),
                _ => {
                    if let Some(key) = node_key(snarl, node) {
                        layout.positions.insert(key, p);
                    }
                }
            }
        }
        layout
    }

    /// Keeps the graph tidy after edits: composed characters' ability inputs close up with one
    /// free input after them, and Learn nodes not set by hand follow their wiring.
    pub fn normalize(&mut self) {
        let snarl = &mut self.snarl;
        let composed: Vec<(NodeId, usize)> = snarl
            .node_ids()
            .filter_map(|(node, n)| match n {
                GraphNode::Composed { abilities, .. } => Some((node, *abilities)),
                _ => None,
            })
            .collect();
        for (node, count) in composed {
            let wired: Vec<(usize, OutPinId)> = (1..=count + 1)
                .filter_map(|input| {
                    snarl
                        .in_pin(InPinId { node, input })
                        .remotes
                        .first()
                        .map(|o| (input, *o))
                })
                .collect();
            let tidy = wired.len() == count
                && wired
                    .iter()
                    .enumerate()
                    .all(|(i, (input, _))| *input == i + 1);
            if tidy {
                continue;
            }
            for input in 1..=count + 1 {
                snarl.drop_inputs(InPinId { node, input });
            }
            for (i, (_, from)) in wired.iter().enumerate() {
                snarl.connect(*from, InPinId { node, input: i + 1 });
            }
            if let GraphNode::Composed { abilities, .. } = &mut snarl[node] {
                *abilities = wired.len();
            }
        }

        let learns: Vec<(NodeId, bool)> = snarl
            .node_ids()
            .filter_map(|(node, n)| match n {
                GraphNode::Learn {
                    auto_defaults: true,
                    ..
                } => {
                    let ability = ability_of(snarl, source(snarl, node, 0))?;
                    let base = base_of(snarl, learner_of(snarl, node)?)?;
                    Some((node, ability.plugin == base.plugin))
                }
                _ => None,
            })
            .collect();
        for (node, same_plugin) in learns {
            if let GraphNode::Learn { use_defaults, .. } = &mut snarl[node] {
                *use_defaults = same_plugin;
            }
        }
    }
}

fn add_composed(snarl: &mut Snarl<GraphNode>, next_serial: &mut u64, pos: Pos2) -> NodeId {
    let id = unique_id("character", &composed_ids(snarl));
    *next_serial += 1;
    snarl.insert_node(
        pos,
        GraphNode::Composed {
            serial: *next_serial - 1,
            id,
            named: false,
            tuning: Tuning::default(),
            hand: None,
            abilities: 0,
        },
    )
}

fn new_learn() -> GraphNode {
    GraphNode::Learn {
        roles: BTreeMap::new(),
        use_defaults: false,
        auto_defaults: true,
    }
}

/// What a node stands for, as a key in the saved layout; `None` for nodes that aren't saved.
fn node_key(snarl: &Snarl<GraphNode>, node: NodeId) -> Option<String> {
    match &snarl[node] {
        GraphNode::Composed { id, .. } => Some(format!("character/{id}")),
        GraphNode::Character(r) => Some(format!("base/{}", ref_key(r))),
        GraphNode::Ability(r) => Some(format!("ability/{}", ref_key(r))),
        GraphNode::Learn { .. } => {
            let ability = ability_of(snarl, source(snarl, node, 0))?;
            let GraphNode::Composed { id, .. } = &snarl[learner_of(snarl, node)?] else {
                return None;
            };
            Some(format!("learn/{id}/{}", ref_key(ability)))
        }
    }
}

/// The graph's look, in the editor's colors.
pub fn style() -> SnarlStyle {
    SnarlStyle {
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
        node_layout: Some(NodeLayout::sandwich()),
        pin_placement: Some(PinPlacement::Edge),
        pin_size: Some(9.0),
        wire_width: Some(2.5),
        collapsible: Some(true),
        min_scale: Some(MIN_SCALE),
        max_scale: Some(MAX_SCALE),
        ..SnarlStyle::new()
    }
}

/// Draws the graph's nodes and handles wiring, with the plugins' characters and abilities to
/// choose from.
pub struct Viewer<'a> {
    pub nav: &'a mut Navigator,
    pub characters: &'a [CharacterChoice],
    pub abilities: &'a [AbilityChoice],
    pub next_serial: &'a mut u64,
    /// Whether characters can be placed in the scene (only once saved).
    pub can_place: bool,
    /// A composed character asked to be placed in the scene, by id.
    pub place: Option<String>,
    /// The size of every node drawn.
    pub sizes: HashMap<NodeId, Vec2>,
}

impl Viewer<'_> {
    fn character(&self, r: &OutputRef) -> Option<&CharacterChoice> {
        self.characters.iter().find(|c| c.at == *r)
    }

    fn ability(&self, r: &OutputRef) -> Option<&AbilityChoice> {
        self.abilities.iter().find(|a| a.at == *r)
    }

    fn character_menu(&self, ui: &mut Ui) -> Option<OutputRef> {
        let mut picked = None;
        if self.characters.is_empty() {
            ui.label(RichText::new("No characters extracted yet").color(theme::PARCHMENT_DIM));
        }
        for c in self.characters {
            if ui
                .button(format!("{} · {}", c.at.output, c.at.plugin))
                .clicked()
            {
                picked = Some(c.at.clone());
                ui.close();
            }
        }
        picked
    }

    fn ability_menu(&self, ui: &mut Ui) -> Option<OutputRef> {
        let mut picked = None;
        if self.abilities.is_empty() {
            ui.label(RichText::new("No abilities extracted yet").color(theme::PARCHMENT_DIM));
        }
        for a in self.abilities {
            if ui
                .button(format!("{} · {}", a.name, a.at.plugin))
                .on_hover_text(&a.description)
                .clicked()
            {
                picked = Some(a.at.clone());
                ui.close();
            }
        }
        picked
    }

    fn character_body(&self, ui: &mut Ui, r: &OutputRef) {
        ui.label(RichText::new(format!("from {}", r.plugin)).color(theme::PARCHMENT_DIM));
        match self.character(r) {
            Some(c) => {
                let what = if c.three_d { "3D model" } else { "2D sprite" };
                ui.small(format!("{what} · {} animations", c.animations.len()));
            }
            None => {
                ui.small(RichText::new("Not extracted (yet)").color(theme::PARCHMENT_DIM));
            }
        }
    }

    fn ability_body(&self, ui: &mut Ui, r: &OutputRef) {
        ui.label(RichText::new(format!("from {}", r.plugin)).color(theme::PARCHMENT_DIM));
        match self.ability(r) {
            Some(a) if !a.description.is_empty() => {
                ui.set_max_width(220.0);
                ui.small(&a.description);
            }
            Some(_) => {}
            None => {
                ui.small(RichText::new("Not extracted (yet)").color(theme::PARCHMENT_DIM));
            }
        }
    }

    fn learn_body(&self, node: NodeId, ui: &mut Ui, snarl: &mut Snarl<GraphNode>) {
        let ability = ability_of(snarl, source(snarl, node, 0))
            .and_then(|r| self.ability(r))
            .cloned();
        let animations: Vec<String> = learner_of(snarl, node)
            .and_then(|composed| base_of(snarl, composed))
            .and_then(|base| self.character(base))
            .map(|c| c.animations.clone())
            .unwrap_or_default();
        let Some(ability) = ability else {
            ui.small(RichText::new("Wire an ability into it").color(theme::PARCHMENT_DIM));
            return;
        };
        let GraphNode::Learn {
            roles,
            use_defaults,
            auto_defaults,
        } = &mut snarl[node]
        else {
            return;
        };
        ui.label(
            RichText::new(&ability.name)
                .strong()
                .color(theme::PARCHMENT),
        );
        if ui
            .checkbox(use_defaults, "Unchosen use its own poses")
            .on_hover_text("For characters drawn like the ability's source")
            .changed()
        {
            *auto_defaults = false;
        }
        egui::Grid::new(("roles", node.0))
            .num_columns(2)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                for role in &ability.roles {
                    ui.label(&role.name);
                    let current = roles.get(&role.name).cloned();
                    egui::ComboBox::from_id_salt(("role", node.0, &role.name))
                        .width(160.0)
                        .selected_text(binding_label(current.as_ref(), *use_defaults))
                        .show_ui(ui, |ui| {
                            let mut pick = |binding: Option<RoleBinding>, label: &str| {
                                if ui.selectable_label(current == binding, label).clicked() {
                                    match binding {
                                        Some(b) => roles.insert(role.name.clone(), b),
                                        None => roles.remove(&role.name),
                                    };
                                }
                            };
                            pick(None, "Not chosen");
                            pick(Some(RoleBinding::Default), "The ability's own pose");
                            for animation in &animations {
                                pick(Some(RoleBinding::Animation(animation.clone())), animation);
                            }
                        });
                    ui.end_row();
                }
            });
        if animations.is_empty() {
            ui.small(
                RichText::new("Wire it into a character to pick its animations")
                    .color(theme::PARCHMENT_DIM),
            );
        }
    }

    fn composed_body(&self, node: NodeId, ui: &mut Ui, snarl: &mut Snarl<GraphNode>) {
        let base = base_of(snarl, node)
            .and_then(|r| self.character(r))
            .cloned();
        let GraphNode::Composed {
            id,
            named,
            tuning,
            hand,
            ..
        } = &mut snarl[node]
        else {
            return;
        };
        egui::Grid::new(("composed", node.0))
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Name");
                // Typed into a buffer and kept once the field is left, so it can be cleared and
                // retyped; what a name can't hold becomes `_`.
                let buffer = egui::Id::new(("composed name", node.0));
                let mut text = ui
                    .data(|d| d.get_temp::<String>(buffer))
                    .unwrap_or_else(|| id.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(kit::hint("lowercase_name"))
                        .desired_width(160.0),
                );
                if response.changed() {
                    ui.data_mut(|d| d.insert_temp(buffer, text.clone()));
                }
                let name = name_from(&text);
                if response.has_focus() && name != text {
                    response.show_tooltip_text(if name.is_empty() {
                        "A name needs at least one letter or digit".to_string()
                    } else {
                        format!("Saved as {name}")
                    });
                }
                if response.lost_focus() {
                    if valid_name(&name) && name != *id {
                        *id = name;
                        *named = true;
                    }
                    ui.data_mut(|d| d.remove::<String>(buffer));
                }
                ui.end_row();
                ui.label("Speed");
                ui.add(egui::Slider::new(&mut tuning.speed, 0.25..=3.0).suffix("×"));
                ui.end_row();
                if base.as_ref().is_some_and(|b| b.three_d) {
                    ui.label("Jump height");
                    ui.add(egui::Slider::new(&mut tuning.jump, 0.25..=3.0).suffix("×"));
                    ui.end_row();
                    ui.label("Holds things at");
                    let bones = base
                        .as_ref()
                        .map(|b| b.bones.as_slice())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt(("hand", node.0))
                        .selected_text(hand.as_deref().unwrap_or("its own hand bone"))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(hand.is_none(), "its own hand bone")
                                .clicked()
                            {
                                *hand = None;
                            }
                            for bone in bones {
                                if ui
                                    .selectable_label(hand.as_deref() == Some(bone), bone)
                                    .clicked()
                                {
                                    *hand = Some(bone.clone());
                                }
                            }
                        });
                    ui.end_row();
                }
            });
    }
}

impl SnarlViewer<GraphNode> for Viewer<'_> {
    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<GraphNode>,
    ) -> egui::Frame {
        self.nav.frame(default, node)
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<GraphNode>,
    ) {
        self.nav.transform(to_global);
    }

    fn title(&mut self, node: &GraphNode) -> String {
        match node {
            GraphNode::Character(r) => r.output.clone(),
            GraphNode::Ability(r) => self
                .ability(r)
                .map_or_else(|| r.output.clone(), |a| a.name.clone()),
            GraphNode::Learn { .. } => "Learn".into(),
            GraphNode::Composed { id, .. } => id.clone(),
        }
    }

    fn inputs(&mut self, node: &GraphNode) -> usize {
        match node {
            GraphNode::Character(_) | GraphNode::Ability(_) => 0,
            GraphNode::Learn { .. } => 1,
            GraphNode::Composed { abilities, .. } => abilities + 2,
        }
    }

    fn outputs(&mut self, node: &GraphNode) -> usize {
        match node {
            GraphNode::Composed { .. } => 0,
            _ => 1,
        }
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        snarl: &mut Snarl<GraphNode>,
    ) -> impl SnarlPin + 'static {
        let kind = input_kind(&snarl[pin.id.node], pin.id.input).unwrap_or(Kind::Learned);
        let label = match kind {
            Kind::Character => "based on",
            Kind::Ability => "ability",
            Kind::Learned if pin.remotes.is_empty() => "learns…",
            Kind::Learned => "learns",
        };
        ui.label(RichText::new(label).color(theme::PARCHMENT_DIM));
        PinInfo::circle().with_fill(kind.color())
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        snarl: &mut Snarl<GraphNode>,
    ) -> impl SnarlPin + 'static {
        let kind = output_kind(&snarl[pin.id.node]).unwrap_or(Kind::Learned);
        let label = match kind {
            Kind::Character => "character",
            Kind::Ability => "ability",
            Kind::Learned => "learned",
        };
        ui.label(RichText::new(label).color(theme::PARCHMENT_DIM));
        PinInfo::circle().with_fill(kind.color())
    }

    fn has_body(&mut self, _node: &GraphNode) -> bool {
        true
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        ui.vertical(|ui| {
            ui.set_max_width(300.0);
            ui.spacing_mut().slider_width = 120.0;
            match snarl[node].clone() {
                GraphNode::Character(r) => self.character_body(ui, &r),
                GraphNode::Ability(r) => self.ability_body(ui, &r),
                GraphNode::Learn { .. } => self.learn_body(node, ui, snarl),
                GraphNode::Composed { .. } => self.composed_body(node, ui, snarl),
            }
        });
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: Rect,
        _ui: &mut Ui,
        _snarl: &mut Snarl<GraphNode>,
    ) {
        self.sizes.insert(node, rect.size());
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GraphNode>) {
        wire(snarl, from.id, to.id);
    }

    fn has_node_menu(&mut self, _node: &GraphNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        if let GraphNode::Composed { id, .. } = &snarl[node] {
            let place = ui
                .add_enabled(self.can_place, egui::Button::new("Place in the scene"))
                .on_disabled_hover_text("Save first");
            if place.clicked() {
                self.place = Some(id.clone());
                ui.close();
            }
        }
        if ui.button("Remove  (Delete)").clicked() {
            snarl.remove_node(node);
            ui.close();
        }
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<GraphNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, snarl: &mut Snarl<GraphNode>) {
        ui.label(RichText::new("Add").color(theme::PARCHMENT_DIM));
        if ui.button("Character of this plugin").clicked() {
            add_composed(snarl, self.next_serial, pos);
            ui.close();
        }
        if ui.button("Learn").clicked() {
            snarl.insert_node(pos, new_learn());
            ui.close();
        }
        ui.menu_button("Character from…", |ui| {
            if let Some(r) = self.character_menu(ui) {
                snarl.insert_node(pos, GraphNode::Character(r));
            }
        });
        ui.menu_button("Ability from…", |ui| {
            if let Some(r) = self.ability_menu(ui) {
                snarl.insert_node(pos, GraphNode::Ability(r));
            }
        });
    }

    fn has_dropped_wire_menu(&mut self, src_pins: AnyPins, _snarl: &mut Snarl<GraphNode>) -> bool {
        matches!(src_pins, AnyPins::Out([_]) | AnyPins::In([_]))
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<GraphNode>,
    ) {
        match src_pins {
            AnyPins::Out([from]) => {
                let (label, kind) = match output_kind(&snarl[from.node]) {
                    Some(Kind::Ability) => ("Learn it", Kind::Ability),
                    Some(kind) => ("New character of this plugin", kind),
                    None => return,
                };
                if ui.button(label).clicked() {
                    // A base goes into a character's first input, a learned ability after it.
                    let to = match kind {
                        Kind::Ability => InPinId {
                            node: snarl.insert_node(pos, new_learn()),
                            input: 0,
                        },
                        Kind::Character => InPinId {
                            node: add_composed(snarl, self.next_serial, pos),
                            input: 0,
                        },
                        Kind::Learned => InPinId {
                            node: add_composed(snarl, self.next_serial, pos),
                            input: 1,
                        },
                    };
                    wire(snarl, *from, to);
                    ui.close();
                }
            }
            AnyPins::In([to]) => match input_kind(&snarl[to.node], to.input) {
                Some(Kind::Character) => {
                    ui.label(RichText::new("Based on").color(theme::PARCHMENT_DIM));
                    if let Some(r) = self.character_menu(ui) {
                        let node = snarl.insert_node(pos, GraphNode::Character(r));
                        wire(snarl, OutPinId { node, output: 0 }, *to);
                    }
                }
                Some(Kind::Ability) => {
                    ui.label(RichText::new("Learns").color(theme::PARCHMENT_DIM));
                    if let Some(r) = self.ability_menu(ui) {
                        let node = snarl.insert_node(pos, GraphNode::Ability(r));
                        wire(snarl, OutPinId { node, output: 0 }, *to);
                    }
                }
                Some(Kind::Learned) => {
                    ui.label(RichText::new("Learns").color(theme::PARCHMENT_DIM));
                    if let Some(r) = self.ability_menu(ui) {
                        let learn = snarl.insert_node(pos, new_learn());
                        let ability =
                            snarl.insert_node(pos - vec2(COLUMN, 0.0), GraphNode::Ability(r));
                        wire(
                            snarl,
                            OutPinId {
                                node: ability,
                                output: 0,
                            },
                            InPinId {
                                node: learn,
                                input: 0,
                            },
                        );
                        wire(
                            snarl,
                            OutPinId {
                                node: learn,
                                output: 0,
                            },
                            *to,
                        );
                    }
                }
                None => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(plugin: &str, output: &str) -> OutputRef {
        OutputRef {
            plugin: plugin.into(),
            output: output.into(),
        }
    }

    fn sample() -> Composition {
        Composition {
            characters: vec![
                ComposedCharacter {
                    id: "runner_with_sword".into(),
                    base: r("game-b-runner", "runner"),
                    abilities: vec![
                        AbilityUse {
                            ability: r("game-a-hero", "sword"),
                            roles: [("swing".to_string(), RoleBinding::Animation("punch".into()))]
                                .into(),
                            use_defaults: false,
                        },
                        AbilityUse {
                            ability: r("game-a-hero", "bow"),
                            roles: BTreeMap::new(),
                            use_defaults: true,
                        },
                    ],
                    tuning: Tuning {
                        speed: 1.2,
                        jump: 1.0,
                    },
                    hand: Some("right_hand".into()),
                },
                ComposedCharacter {
                    id: "fast_runner".into(),
                    base: r("game-b-runner", "runner"),
                    abilities: vec![AbilityUse {
                        ability: r("game-a-hero", "sword"),
                        roles: BTreeMap::new(),
                        use_defaults: false,
                    }],
                    tuning: Tuning::default(),
                    hand: None,
                },
            ],
        }
    }

    fn node_where(graph: &CompositionGraph, f: impl Fn(&GraphNode) -> bool) -> NodeId {
        graph.snarl.node_ids().find(|(_, n)| f(n)).unwrap().0
    }

    #[test]
    fn a_composition_survives_the_graph() {
        let graph = CompositionGraph::new(&sample(), &Layout::default());
        let built = graph.build();
        assert_eq!(built.composition, sample());
        assert!(built.errors.is_empty() && built.notes.is_empty());
        // Shared characters and abilities are drawn once.
        let count = |f: fn(&GraphNode) -> bool| graph.snarl.nodes().filter(|n| f(n)).count();
        assert_eq!(count(|n| matches!(n, GraphNode::Character(_))), 1);
        assert_eq!(count(|n| matches!(n, GraphNode::Ability(_))), 2);
        assert_eq!(count(|n| matches!(n, GraphNode::Learn { .. })), 3);
    }

    #[test]
    fn the_layout_brings_nodes_back_where_they_were() {
        let mut graph = CompositionGraph::new(&sample(), &Layout::default());
        let composed = node_where(
            &graph,
            |n| matches!(n, GraphNode::Composed { id, .. } if id == "fast_runner"),
        );
        graph.snarl.get_node_info_mut(composed).unwrap().pos = pos2(900.0, 40.0);
        graph
            .snarl
            .insert_node(pos2(5.0, 6.0), GraphNode::Ability(r("game-c", "hookshot")));
        let layout = graph.layout();
        assert_eq!(layout.positions["character/fast_runner"], (900.0, 40.0));
        assert_eq!(
            layout.loose,
            vec![Loose::Ability(r("game-c", "hookshot"), (5.0, 6.0))]
        );
        let text = ron::to_string(&layout).unwrap();
        let again = CompositionGraph::new(&sample(), &ron::from_str(&text).unwrap());
        assert_eq!(again.layout(), layout);
    }

    #[test]
    fn wiring_checks_kinds_and_names_new_characters() {
        let mut graph = CompositionGraph::new(&Composition::default(), &Layout::default());
        let hero = graph
            .snarl
            .insert_node(Pos2::ZERO, GraphNode::Character(r("game-a-hero", "hero")));
        let bow = graph
            .snarl
            .insert_node(Pos2::ZERO, GraphNode::Ability(r("game-a-hero", "bow")));
        let composed = graph.add_composed(Pos2::ZERO);
        let out = |node| OutPinId { node, output: 0 };
        let inp = |node, input| InPinId { node, input };
        // An ability can't be a base, and a character can't be learned.
        assert!(!wire(&mut graph.snarl, out(bow), inp(composed, 0)));
        assert!(!wire(&mut graph.snarl, out(hero), inp(composed, 1)));
        assert_eq!(graph.build().errors.len(), 1);

        assert!(wire(&mut graph.snarl, out(hero), inp(composed, 0)));
        let learn = graph.snarl.insert_node(Pos2::ZERO, new_learn());
        assert!(wire(&mut graph.snarl, out(bow), inp(learn, 0)));
        assert!(wire(&mut graph.snarl, out(learn), inp(composed, 1)));
        graph.normalize();

        let built = graph.build();
        assert!(built.errors.is_empty());
        let c = &built.composition.characters[0];
        assert_eq!(c.id, "hero_plus");
        assert_eq!(c.abilities[0].ability, r("game-a-hero", "bow"));
        // The same plugin's ability uses its own poses unless set otherwise.
        assert!(c.abilities[0].use_defaults);
        assert!(matches!(
            graph.snarl[composed],
            GraphNode::Composed { abilities: 1, .. }
        ));
    }

    #[test]
    fn ability_inputs_close_up_when_one_is_unwired() {
        let mut graph = CompositionGraph::new(&sample(), &Layout::default());
        let composed = node_where(
            &graph,
            |n| matches!(n, GraphNode::Composed { id, .. } if id == "runner_with_sword"),
        );
        graph.snarl.drop_inputs(InPinId {
            node: composed,
            input: 1,
        });
        graph.normalize();
        assert!(matches!(
            graph.snarl[composed],
            GraphNode::Composed { abilities: 1, .. }
        ));
        let built = graph.build();
        assert_eq!(
            built.composition.characters[0].abilities[0].ability,
            r("game-a-hero", "bow")
        );
        assert_eq!(built.notes.len(), 1, "the unwired Learn node is reported");
    }

    #[test]
    fn duplicate_names_block_saving() {
        let mut composition = sample();
        composition.characters[1].id = "runner_with_sword".into();
        let graph = CompositionGraph::new(&composition, &Layout::default());
        assert_eq!(graph.build().errors.len(), 1);
    }

    #[test]
    fn changes_can_be_undone_and_redone() {
        let mut graph = CompositionGraph::new(&sample(), &Layout::default());
        let composed = node_where(
            &graph,
            |n| matches!(n, GraphNode::Composed { id, .. } if id == "fast_runner"),
        );
        graph.snarl.remove_node(composed);
        graph.track();
        assert_eq!(graph.build().composition.characters.len(), 1);
        graph.undo();
        assert_eq!(graph.build().composition, sample());
        graph.redo();
        assert_eq!(graph.build().composition.characters.len(), 1);
        assert!(!graph.can_redo() && graph.can_undo());
    }

    #[test]
    fn unique_ids_skip_taken_ones() {
        assert_eq!(unique_id("hero", &[]), "hero_plus");
        assert_eq!(unique_id("hero", &["hero_plus".into()]), "hero_plus_2");
    }
}

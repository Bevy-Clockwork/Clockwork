//! How egui-snarl draws the graph: node headers and bodies, pins and wires, and group frames.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy_egui::egui::{self, Color32, Pos2, Rect, RichText, Stroke, TextureHandle, Ui, Vec2, vec2};
use clockwork_recipe::text::Doc;
use egui_snarl::{
    InPin, NodeId, OutPin, Snarl,
    ui::{
        AnyPins, BackgroundPattern, NodeLayout, PinInfo, PinPlacement, SnarlPin, SnarlStyle,
        SnarlViewer,
    },
};

use super::{
    Change, GNode, GroupId, GroupInfo, NODE_THUMB, OwnedPort, Picked, Places, RNode, RecipeGraph,
    Resolution, Trace, add_list, candidates, groups, guess_type, index, inputs_of, kind_color,
    labeled_value_row, name_menu, node_key, order, outputs_of, thumb, title, variant_info,
};
use crate::{
    graph_view::{MAX_SCALE, MIN_SCALE, Navigator},
    kit, theme,
};

/// The graph's look, in the editor's colors.
pub(super) fn style() -> SnarlStyle {
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
pub(super) const DIMMED: Color32 = Color32::from_rgb(0x3A, 0x36, 0x30);

pub(super) struct Viewer<'a> {
    pub(super) nav: &'a mut Navigator,
    pub(super) doc: &'a Doc,
    /// How many steps the recipe has.
    pub(super) steps: usize,
    pub(super) ports: &'a HashMap<RNode, Vec<OwnedPort>>,
    pub(super) resolution: &'a Resolution,
    pub(super) trace: &'a Trace,
    pub(super) thumbs: &'a mut HashMap<String, (u64, TextureHandle)>,
    pub(super) revision: u64,
    /// The node in the Inspector, and what it is made from and makes (the rest is dimmed).
    pub(super) focus: Option<RNode>,
    pub(super) focus_group: Option<GroupId>,
    pub(super) lineage: Option<&'a HashSet<RNode>>,
    pub(super) add_query: &'a mut String,
    /// Where a new step and a new output go (after the one in the Inspector).
    pub(super) insert: (Option<usize>, Option<usize>),
    pub(super) groups: &'a BTreeMap<GroupId, GroupInfo>,
    pub(super) pins: &'a HashMap<GroupId, groups::Pins>,
    pub(super) nodes: &'a Places,
    pub(super) edits: &'a mut HashMap<(RNode, String), String>,
    /// Node sizes as last drawn, for the frames of open groups.
    pub(super) last_sizes: &'a HashMap<NodeId, Vec2>,
    pub(super) show_order: bool,
    pub(super) changes: Vec<Change>,
    /// Groups to open or close, ungroup, or rename.
    pub(super) group_changes: Vec<GroupChange>,
    pub(super) sizes: HashMap<NodeId, Vec2>,
}

/// A change to the groups (not to the recipe) asked for while drawing.
#[derive(Clone, Debug)]
pub(super) enum GroupChange {
    Open(GroupId, bool),
    Ungroup(GroupId),
    Rename(GroupId, String),
}

impl Viewer<'_> {
    pub(super) fn port_in(&self, node: RNode, pin: usize) -> Option<(usize, &OwnedPort)> {
        let ports = self.ports.get(&node)?;
        let i = *inputs_of(ports).get(pin)?;
        Some((i, &ports[i]))
    }

    pub(super) fn port_out(&self, node: RNode, pin: usize) -> Option<(usize, &OwnedPort)> {
        let ports = self.ports.get(&node)?;
        let i = *outputs_of(ports).get(pin)?;
        Some((i, &ports[i]))
    }

    /// The step or output and port an output pin of a snarl node stands for.
    pub(super) fn out_at(&self, node: GNode, pin: usize) -> Option<(RNode, usize)> {
        match node {
            GNode::Item(node) => self.port_out(node, pin).map(|(i, _)| (node, i)),
            GNode::Group(g) => self.pins.get(&g)?.outputs.get(pin).copied(),
        }
    }

    pub(super) fn lit(&self, node: RNode) -> bool {
        self.lineage.is_none_or(|l| l.contains(&node))
    }

    /// Whether anything in a group is in the selected node's lineage.
    pub(super) fn group_lit(&self, g: GroupId) -> bool {
        self.lineage.is_none_or(|l| {
            l.iter()
                .any(|n| groups::within(self.groups, self.nodes.get(n).and_then(|p| p.group), g))
        })
    }

    pub(super) fn gnode_lit(&self, node: GNode) -> bool {
        match node {
            GNode::Item(node) => self.lit(node),
            GNode::Group(g) => self.group_lit(g),
        }
    }

    /// What the step or output made, in short: its first line and a small preview.
    pub(super) fn result(&mut self, node: RNode, ui: &mut Ui) {
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

    /// A closed group's body: what is in it, the last picture it makes, its error, the values
    /// shown on it, and opening it.
    pub(super) fn group_body(&mut self, g: GroupId, ui: &mut Ui) {
        let members = groups::members(self.groups, self.nodes, g);
        let (steps, outputs) = members.iter().fold((0, 0), |(s, o), n| match n {
            RNode::Step(_) => (s + 1, o),
            RNode::Output(_) => (s, o + 1),
        });
        ui.set_max_width(260.0);
        let what = match outputs {
            0 => format!("{steps} steps"),
            n => format!("{steps} steps · {n} outputs"),
        };
        ui.small(RichText::new(what).color(theme::PARCHMENT_DIM));
        let failed = members.iter().any(|n| match n {
            RNode::Step(i) => self.trace.error_step == Some(*i),
            RNode::Output(i) => self.trace.error_output == Some(*i),
        });
        if failed {
            let error = self.trace.error.as_deref().unwrap_or_default();
            ui.label(
                RichText::new(format!("⚠ {error}"))
                    .small()
                    .color(theme::ERROR),
            );
        }
        let picture = members.iter().rev().find_map(|n| {
            let results = match n {
                RNode::Step(_) => &self.trace.steps,
                RNode::Output(_) => &self.trace.outputs,
            };
            let image = results.get(index(*n))?.as_ref()?.image.as_ref()?;
            Some((*n, image))
        });
        if let Some((node, image)) = picture {
            let texture = thumb(ui.ctx(), self.thumbs, self.revision, node, image);
            let side = image.width.max(image.height).max(1) as f32;
            let scale = (NODE_THUMB / side).min(1.0);
            ui.image((
                texture.id(),
                vec2(image.width as f32, image.height as f32) * scale,
            ));
        }
        // The values chosen to be shown on the group (a higher-level node's settings).
        for node in &members {
            let Some(place) = self.nodes.get(node) else {
                continue;
            };
            let Some(item) = RecipeGraph::node_doc(self.doc, *node) else {
                continue;
            };
            let info = variant_info(*node, item.name().unwrap_or_default());
            // Named by what its step makes: "tiles_1 offset", not just "offset".
            let made = self.ports.get(node).and_then(|ports| {
                ports
                    .iter()
                    .find(|p| p.access.writes())
                    .map(|p| p.name.clone())
            });
            for key in &place.exposed {
                let Some(field) = item.field(key) else {
                    continue;
                };
                let text = self.doc.text_of(field);
                let about = info.and_then(|i| i.field(key));
                let ty = about.map_or_else(|| guess_type(text), |f| f.ty);
                let edit_key = (*node, key.clone());
                let label = made.as_ref().map_or(key.clone(), |m| format!("{m} {key}"));
                if let Some(new) =
                    labeled_value_row(ui, &label, self.edits, &edit_key, about, ty, text)
                    && new != text
                {
                    self.changes.push(Change::Replace {
                        span: field.span.clone(),
                        text: new,
                        field: Some(edit_key),
                    });
                }
            }
        }
        if ui
            .small_button("Open ▸")
            .on_hover_text("Show what is inside, here")
            .clicked()
        {
            self.group_changes.push(GroupChange::Open(g, true));
        }
    }

    pub(super) fn add(
        &mut self,
        template: String,
        step: bool,
        at: Pos2,
        index: Option<usize>,
    ) -> &mut Change {
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

    /// The open groups drawn and the snarl nodes in each (also those in groups inside it),
    /// innermost last.
    pub(super) fn frames(&self, snarl: &Snarl<GNode>) -> Vec<(GroupId, usize, Rect)> {
        let mut rects: HashMap<GroupId, Rect> = HashMap::new();
        for (id, pos, node) in snarl.nodes_pos_ids() {
            let size = self
                .last_sizes
                .get(&id)
                .copied()
                .unwrap_or(vec2(240.0, 100.0));
            let rect = Rect::from_min_size(pos, size);
            let container = match node {
                GNode::Item(n) => self.nodes.get(n).and_then(|p| p.group),
                GNode::Group(g) => self.groups.get(g).and_then(|i| i.parent),
            };
            for g in groups::ancestors(self.groups, container) {
                let r = rects.entry(g).or_insert(rect);
                *r = r.union(rect);
            }
        }
        let mut frames: Vec<(GroupId, usize, Rect)> = rects
            .into_iter()
            .map(|(g, r)| {
                let depth = groups::ancestors(self.groups, Some(g)).len();
                (
                    g,
                    depth,
                    r.expand2(vec2(FRAME_MARGIN, FRAME_MARGIN))
                        .with_min_y(r.min.y - FRAME_HEADER),
                )
            })
            .collect();
        frames.sort_by_key(|(g, depth, _)| (*depth, *g));
        frames
    }
}

/// Room around the nodes of an open group, and above them for its name.
pub(super) const FRAME_MARGIN: f32 = 16.0;

pub(super) const FRAME_HEADER: f32 = 44.0;

impl SnarlViewer<GNode> for Viewer<'_> {
    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<GNode>,
    ) -> egui::Frame {
        let value = snarl[node];
        let frame = self.nav.frame(default, node);
        let focused = match value {
            GNode::Item(n) => self.focus == Some(n),
            GNode::Group(g) => self.focus_group == Some(g),
        };
        let frame = match value {
            GNode::Group(_) => frame
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.5, theme::BRASS))
                .corner_radius(10),
            GNode::Item(_) => frame,
        };
        if focused {
            frame.stroke(Stroke::new(2.0, theme::BRASS_BRIGHT))
        } else if self.gnode_lit(value) {
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
        snarl: &Snarl<GNode>,
    ) -> bool {
        !self.gnode_lit(snarl[node])
    }

    fn apply_node_style(
        &mut self,
        style: &mut egui::Style,
        _node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<GNode>,
    ) {
        style.visuals.override_text_color = Some(theme::FAINT);
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<GNode>,
    ) {
        self.nav.transform(to_global);
    }

    fn draw_background(
        &mut self,
        background: Option<&BackgroundPattern>,
        viewport: &Rect,
        snarl_style: &SnarlStyle,
        style: &egui::Style,
        painter: &egui::Painter,
        snarl: &Snarl<GNode>,
    ) {
        if let Some(background) = background {
            background.draw(viewport, snarl_style, style, painter);
        }
        // Open groups: a frame around their nodes, deeper ones lighter.
        for (g, depth, rect) in self.frames(snarl) {
            let fill =
                Color32::from_rgba_unmultiplied(0xC8, 0xA0, 0x60, (8 + 6 * depth).min(40) as u8);
            painter.rect(
                rect,
                12.0,
                fill,
                Stroke::new(1.0, theme::BRASS_DIM),
                egui::StrokeKind::Inside,
            );
            if let Some(info) = self.groups.get(&g) {
                painter.text(
                    rect.left_top() + vec2(14.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    &info.name,
                    egui::FontId::proportional(18.0),
                    theme::BRASS_BRIGHT,
                );
            }
        }
        if self.show_order {
            let drawn: HashMap<GNode, Rect> = snarl
                .nodes_pos_ids()
                .map(|(id, pos, node)| {
                    let size = self
                        .last_sizes
                        .get(&id)
                        .copied()
                        .unwrap_or(vec2(240.0, 100.0));
                    (*node, Rect::from_min_size(pos, size))
                })
                .collect();
            let group_of = |s: usize| self.nodes.get(&RNode::Step(s)).and_then(|p| p.group);
            let path = order::run_path(self.steps, &drawn, self.groups, group_of);
            order::draw(painter, &path);
        }
    }

    fn title(&mut self, node: &GNode) -> String {
        match node {
            GNode::Item(n) => title(self.doc, *n),
            GNode::Group(g) => format!(
                "▣ {}",
                self.groups.get(g).map_or("Group", |i| i.name.as_str())
            ),
        }
    }

    fn inputs(&mut self, node: &GNode) -> usize {
        match node {
            GNode::Item(n) => self.ports.get(n).map_or(0, |p| inputs_of(p).len()),
            GNode::Group(g) => self.pins.get(g).map_or(0, |p| p.inputs.len()),
        }
    }

    fn outputs(&mut self, node: &GNode) -> usize {
        match node {
            GNode::Item(n) => self.ports.get(n).map_or(0, |p| outputs_of(p).len()),
            GNode::Group(g) => self.pins.get(g).map_or(0, |p| p.outputs.len()),
        }
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        snarl: &mut Snarl<GNode>,
    ) -> impl SnarlPin + 'static {
        let node = match snarl[pin.id.node] {
            GNode::Item(node) => node,
            GNode::Group(g) => {
                let Some(input) = self.pins.get(&g).and_then(|p| p.inputs.get(pin.id.input)) else {
                    return PinInfo::circle();
                };
                let Some(&(target, port)) = input.targets.first() else {
                    return PinInfo::circle();
                };
                let port = &self.ports[&target][port];
                let (kind, missing) = (port.kind, input.from.is_none());
                let name = if port.name.is_empty() {
                    "—"
                } else {
                    &port.name
                };
                let readers = input.targets.len();
                ui.label(
                    RichText::new(format!("{}: {name}", kind.label())).color(if missing {
                        theme::ERROR
                    } else {
                        theme::PARCHMENT_DIM
                    }),
                )
                .on_hover_text(if missing {
                    format!(
                        "A step inside reads the {} \"{name}\", which no earlier step makes",
                        kind.label()
                    )
                } else {
                    format!("Read by {readers} input(s) inside. Open the group to change them.")
                });
                let lit = self.group_lit(g) && input.from.is_some_and(|(from, _)| self.lit(from));
                return PinInfo::circle()
                    .with_fill(if missing {
                        theme::ERROR
                    } else {
                        kind_color(kind)
                    })
                    .with_wire_color(if lit { kind_color(kind) } else { DIMMED });
            }
        };
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
        let (ports, steps) = (self.ports, self.steps);
        egui::Popup::menu(&response)
            .id(egui::Id::new(("pick input", node_key(node), i)))
            .show(|ui| {
                // Only while the menu is open: it looks through every earlier step.
                let names = candidates(ports, steps, node, kind);
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
                .source
                .get(&(node, i))
                .is_some_and(|from| self.lit(from.0));
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
        snarl: &mut Snarl<GNode>,
    ) -> impl SnarlPin + 'static {
        let shown = snarl[pin.id.node];
        let Some((node, i)) = self.out_at(shown, pin.id.output) else {
            return PinInfo::circle();
        };
        let port = &self.ports[&node][i];
        ui.label(RichText::new(&port.name).color(theme::PARCHMENT))
            .on_hover_text(port.kind.label());
        let color = kind_color(port.kind);
        PinInfo::circle()
            .with_fill(color)
            .with_wire_color(if self.gnode_lit(shown) { color } else { DIMMED })
    }

    fn has_body(&mut self, _node: &GNode) -> bool {
        true
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GNode>,
    ) {
        match snarl[node] {
            GNode::Item(node) => {
                ui.vertical(|ui| {
                    ui.set_max_width(240.0);
                    self.result(node, ui);
                });
            }
            GNode::Group(g) => {
                ui.vertical(|ui| self.group_body(g, ui));
            }
        }
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: Rect,
        _ui: &mut Ui,
        _snarl: &mut Snarl<GNode>,
    ) {
        self.sizes.insert(node, rect.size());
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GNode>) {
        let Some(from) = self.out_at(snarl[from.id.node], from.id.output) else {
            return;
        };
        match snarl[to.id.node] {
            GNode::Item(to_node) => {
                if let Some((b, _)) = self.port_in(to_node, to.id.input) {
                    self.changes.push(Change::Wire {
                        from,
                        to: (to_node, b),
                    });
                }
            }
            GNode::Group(g) => {
                // One input of a group can be read by several inside: open it to choose.
                self.group_changes.push(GroupChange::Open(g, true));
            }
        }
    }

    // A wire is a name: it goes away when the input reads something else (click the input).
    fn disconnect(&mut self, _from: &OutPin, _to: &InPin, _snarl: &mut Snarl<GNode>) {}

    fn drop_outputs(&mut self, _pin: &OutPin, _snarl: &mut Snarl<GNode>) {}

    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<GNode>) {
        let GNode::Item(node) = snarl[pin.id.node] else {
            return;
        };
        if let Some((i, _)) = self.port_in(node, pin.id.input) {
            self.changes.push(Change::Rename {
                at: (node, i),
                name: String::new(),
            });
        }
    }

    fn has_node_menu(&mut self, _node: &GNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GNode>,
    ) {
        let node = match snarl[node] {
            GNode::Item(node) => node,
            GNode::Group(g) => {
                if ui.button("Open  (double-click)").clicked() {
                    self.group_changes.push(GroupChange::Open(g, true));
                    ui.close();
                }
                let name = self
                    .groups
                    .get(&g)
                    .map_or(String::new(), |i| i.name.clone());
                let id = egui::Id::new(("rename group", g));
                let mut text = ui
                    .data(|d| d.get_temp::<String>(id))
                    .unwrap_or(name.clone());
                ui.horizontal(|ui| {
                    ui.label(kit::dim("Name"));
                    let response = ui.text_edit_singleline(&mut text);
                    if response.lost_focus() && text.trim() != name && !text.trim().is_empty() {
                        self.group_changes
                            .push(GroupChange::Rename(g, text.trim().to_string()));
                    }
                });
                ui.data_mut(|d| d.insert_temp(id, text));
                if ui
                    .button("Ungroup")
                    .on_hover_text("Its nodes stay, outside a group")
                    .clicked()
                {
                    self.group_changes.push(GroupChange::Ungroup(g));
                    ui.close();
                }
                if ui
                    .button(RichText::new("Remove its steps").color(theme::ERROR))
                    .clicked()
                {
                    let members = groups::members(self.groups, self.nodes, g);
                    self.changes.push(Change::Remove(members));
                    ui.close();
                }
                return;
            }
        };
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
        if let Some(g) = self.nodes.get(&node).and_then(|p| p.group)
            && ui.button("Close its group").clicked()
        {
            self.group_changes.push(GroupChange::Open(g, false));
            ui.close();
        }
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<GNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, _snarl: &mut Snarl<GNode>) {
        ui.set_width(320.0);
        ui.label(kit::hint(if self.insert.0.is_some() {
            "Add after the selected step"
        } else {
            "Add"
        }));
        match add_list(ui, self.add_query, true, |_, _| true) {
            Some(Picked::Template(template, step)) => {
                let index = if step { self.insert.0 } else { self.insert.1 };
                self.add(template, step, pos, index);
            }
            Some(Picked::Block(block)) => self.changes.push(Change::AddBlock { block, at: pos }),
            None => {}
        }
    }

    fn has_dropped_wire_menu(&mut self, src_pins: AnyPins, snarl: &mut Snarl<GNode>) -> bool {
        match src_pins {
            AnyPins::Out([_]) => true,
            AnyPins::In([to]) => matches!(snarl[to.node], GNode::Item(_)),
            _ => false,
        }
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<GNode>,
    ) {
        ui.set_width(320.0);
        match src_pins {
            AnyPins::Out([from]) => {
                let Some((node, i)) = self.out_at(snarl[from.node], from.output) else {
                    return;
                };
                let port = &self.ports[&node][i];
                let (kind, name) = (port.kind, port.name.clone());
                ui.label(kit::hint(format!("Use {} \"{name}\" in", kind.label())));
                // Right after the step that makes it.
                let after = match node {
                    RNode::Step(i) => Some(i + 1),
                    RNode::Output(_) => None,
                };
                if let Some(Picked::Template(template, step)) =
                    add_list(ui, self.add_query, false, |reads, _| reads.contains(&kind))
                    && let Change::Add { read, .. } =
                        self.add(template, step, pos, if step { after } else { None })
                {
                    *read = Some((kind, name));
                }
            }
            AnyPins::In([to]) => {
                let GNode::Item(node) = snarl[to.node] else {
                    return;
                };
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
                if let Some(Picked::Template(template, step)) =
                    add_list(ui, self.add_query, false, |_, writes| {
                        writes.contains(&kind)
                    })
                    && let Change::Add { feed, .. } = self.add(template, step, pos, before)
                {
                    *feed = Some((node, i));
                }
            }
            _ => {}
        }
    }
}

//! What the plugin editor's node graphs share: finding nodes and moving the view to them.

use std::collections::HashMap;

use bevy_egui::egui::{self, Rect, RichText, Stroke, Ui, Vec2, emath::TSTransform, vec2};
use egui_snarl::{NodeId, Snarl};

use crate::{kit, theme};

/// How far the view zooms out (big recipes) and in.
pub const MIN_SCALE: f32 = 0.05;
pub const MAX_SCALE: f32 = 2.0;

/// Room left around what the view moves to, in graph units.
const MARGIN: f32 = 60.0;
/// The smallest zoom a graph opens at: below it nothing can be read, so a big graph opens on
/// its top left corner instead of all of it.
const READABLE: f32 = 0.7;

/// Finds nodes by text and moves the view: to a match, or to fit the whole graph.
pub struct Navigator {
    pub query: String,
    matches: Vec<NodeId>,
    /// The match the view is on.
    at: usize,
    /// The query the matches were found for, and what to do next frame.
    searched: String,
    step: i32,
    fit: bool,
    /// The part of the graph to show next frame (graph space).
    look_at: Option<Rect>,
    /// Show what `look_at` holds no smaller than [`READABLE`], from its top left.
    readable: bool,
    /// Where the graph is drawn on screen.
    viewport: Rect,
    /// Graph to screen, as last drawn.
    shown: TSTransform,
}

impl Default for Navigator {
    fn default() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            at: 0,
            searched: String::new(),
            step: 0,
            fit: false,
            look_at: None,
            readable: false,
            viewport: Rect::NOTHING,
            shown: TSTransform::IDENTITY,
        }
    }
}

impl Navigator {
    /// The search field, match count, previous/next and Fit, for a graph's toolbar.
    pub fn toolbar(&mut self, ui: &mut Ui, id_salt: &str) {
        let id = egui::Id::new(("find", id_salt));
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(id)
                .hint_text(kit::hint("🔍 Find node  (Ctrl+F)"))
                .desired_width(160.0),
        );
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.step = 1;
            field.request_focus();
        }
        if !self.query.is_empty() {
            let count = self.matches.len();
            let text = match count {
                0 => "none".to_string(),
                n => format!("{} of {n}", self.at.min(n - 1) + 1),
            };
            ui.label(RichText::new(text).color(theme::PARCHMENT_DIM));
            ui.add_enabled_ui(count > 1, |ui| {
                if kit::icon_button(ui, "⏶", "Previous match").clicked() {
                    self.step = -1;
                }
                if kit::icon_button(ui, "⏷", "Next match  ·  Enter").clicked() {
                    self.step = 1;
                }
            });
        }
        if kit::icon_button(ui, "⛶", "Fit: show the whole graph").clicked() {
            self.fit();
        }
    }

    /// Shows the whole graph next frame.
    pub fn fit(&mut self) {
        self.fit = true;
        self.readable = false;
    }

    /// Shows the whole graph once it is drawn, or its top left at a zoom that can be read.
    pub fn open_view(&mut self) {
        self.fit = true;
        self.readable = true;
    }

    /// Moves the view to a part of the graph (graph space).
    pub fn show(&mut self, rect: Rect) {
        self.look_at = Some(rect);
        self.readable = false;
    }

    /// Where a point on screen is in the graph.
    pub fn graph_pos(&self, screen: egui::Pos2) -> egui::Pos2 {
        self.shown.inverse() * screen
    }

    /// Where a part of the graph is on screen, as last drawn.
    pub fn screen_rect(&self, graph: Rect) -> Rect {
        self.shown * graph
    }

    /// Finds matches (`text` gives what a node is searched by) and decides where the view goes;
    /// call before drawing the graph in `viewport`.
    pub fn update<T>(
        &mut self,
        snarl: &Snarl<T>,
        sizes: &HashMap<NodeId, Vec2>,
        viewport: Rect,
        text: impl Fn(NodeId, &T) -> String,
    ) {
        self.viewport = viewport;
        let rect = |node: NodeId| {
            let pos = snarl.get_node_info(node).map(|n| n.pos).unwrap_or_default();
            Rect::from_min_size(pos, sizes.get(&node).copied().unwrap_or(vec2(240.0, 120.0)))
        };
        if self.query != self.searched {
            self.searched = self.query.clone();
            let query = self.query.to_lowercase();
            self.matches = if query.is_empty() {
                Vec::new()
            } else {
                let mut found: Vec<(NodeId, egui::Pos2)> = snarl
                    .nodes_pos_ids()
                    .filter(|(id, _, node)| text(*id, node).to_lowercase().contains(&query))
                    .map(|(id, pos, _)| (id, pos))
                    .collect();
                // Left to right, then top to bottom, as the graph reads.
                found.sort_by(|a, b| (a.1.x, a.1.y).partial_cmp(&(b.1.x, b.1.y)).unwrap());
                found.into_iter().map(|(id, _)| id).collect()
            };
            self.at = 0;
            if !self.matches.is_empty() {
                self.look_at = Some(rect(self.matches[0]));
            }
        }
        if self.step != 0 && !self.matches.is_empty() {
            let n = self.matches.len() as i32;
            self.at = (self.at as i32 + self.step).rem_euclid(n) as usize;
            self.look_at = Some(rect(self.matches[self.at]));
        }
        self.step = 0;
        if self.fit {
            self.fit = false;
            self.look_at = snarl.node_ids().map(|(id, _)| rect(id)).reduce(Rect::union);
        }
    }

    /// The node frame for a match (outlined) or any other node.
    pub fn frame(&self, default: egui::Frame, node: NodeId) -> egui::Frame {
        match self.matches.iter().position(|m| *m == node) {
            Some(i) if i == self.at => default.stroke(Stroke::new(3.0, theme::BRASS_BRIGHT)),
            Some(_) => default.stroke(Stroke::new(2.0, theme::BRASS)),
            None => default,
        }
    }

    /// Moves the view where [`Navigator::update`] decided, from the viewer's
    /// `current_transform`.
    pub fn transform(&mut self, to_global: &mut TSTransform) {
        let Some(rect) = self.look_at.take() else {
            self.shown = *to_global;
            return;
        };
        let view = self.viewport;
        if view.width() <= 0.0 || view.height() <= 0.0 {
            return;
        }
        let rect = rect.expand(MARGIN);
        let scale = (view.width() / rect.width())
            .min(view.height() / rect.height())
            .clamp(MIN_SCALE, 1.0);
        *to_global = if self.readable && scale < READABLE {
            TSTransform::new(view.min.to_vec2() - rect.min.to_vec2() * READABLE, READABLE)
        } else {
            TSTransform::new(
                view.center().to_vec2() - rect.center().to_vec2() * scale,
                scale,
            )
        };
        self.readable = false;
        self.shown = *to_global;
    }
}

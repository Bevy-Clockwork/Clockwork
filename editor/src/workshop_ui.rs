//! The Workshop screen: scene tree (left), viewport with the adjust-after-drop panel (centre),
//! Inspector (right) and the content browser / log dock (bottom). Follows Godot's layout and
//! Blender's "select, then operate" and "adjust the last action" ideas.

use bevy_egui::egui::{self, Color32, RichText, Stroke, vec2};
use clockwork_core::{
    asset::OutputRef,
    surface::{SurfaceMap, SurfaceProperty},
};
use clockwork_runtime::hud::HudRequest;

use crate::{
    browser::{self, BottomTab, Browser, DraggedItem},
    kit,
    scene::{Node, NodeId, ROOT},
    state::{EditorState, LogLevel},
    surfaces, theme,
    ui::{info_row, log_color},
    workshop::{Extractions, surfaces_for},
};

/// A scene node being dragged in the scene tree.
#[derive(Clone, Copy)]
struct DraggedNode(NodeId);

/// Draws the Workshop and returns the rect left for the 3D viewport.
pub fn workshop_screen(
    root: &mut egui::Ui,
    state: &mut EditorState,
    browser: &mut Browser,
    extractions: &Extractions,
    settings: &crate::settings::Settings,
) -> egui::Rect {
    egui::Panel::left("scene_tree")
        .resizable(true)
        .default_size(260.0)
        .size_range(200.0..=480.0)
        .frame(kit::panel_frame())
        .show(root, |ui| scene_tree(ui, state, extractions));

    egui::Panel::right("inspector")
        .resizable(true)
        .default_size(310.0)
        .size_range(260.0..=520.0)
        .frame(kit::panel_frame())
        .show(root, |ui| inspector(ui, state, browser, extractions));

    egui::Panel::bottom("content")
        .resizable(true)
        .default_size(250.0)
        .size_range(120.0..=600.0)
        .frame(kit::panel_frame())
        .show(root, |ui| bottom_dock(ui, state, browser));

    let items = state
        .workshop
        .scene
        .nodes
        .iter()
        .filter(|n| n.item.is_some())
        .count();
    let can_play = state.workshop.scene.nodes.iter().any(|n| {
        n.item.as_ref().is_some_and(|i| {
            extractions.get(&i.plugin).is_some_and(|x| {
                x.model(&i.output).is_some_and(|m| m.character.is_some())
                    || x.sprite(&i.output).is_some_and(|s| s.character.is_some())
            })
        })
    });
    let mut show_surfaces = state.workshop.show_surfaces;
    let rect = egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            kit::bar(ui, |ui| {
                viewport_toolbar(ui, state, &mut show_surfaces, can_play, items);
            });
        })
        .response
        .rect;
    state.workshop.show_surfaces = show_surfaces;
    // The scene shows below the toolbar.
    let scene = rect.with_min_y(rect.min.y + kit::BAR_H);

    // What the mouse does here, in the viewport's bottom-right corner.
    let hint = if state.workshop.playing.is_some() {
        "Gameplay controls: Settings → Controls"
    } else if items == 0 {
        "Drag content from below into the viewport"
    } else {
        "Click select · Drag orbit · Right drag pan · Wheel zoom"
    };
    root.ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("viewport_hint"),
        ))
        .text(
            scene.right_bottom() - vec2(10.0, 8.0),
            egui::Align2::RIGHT_BOTTOM,
            hint,
            egui::FontId::proportional(11.5),
            Color32::from_white_alpha(140),
        );

    adjust_panel(root.ctx(), scene, state);
    live_panel(root.ctx(), scene, state);

    // The panels live on egui's background layer, so anything above it (menus, popups, the
    // adjust panel), the toolbar, or a widget drag in progress means the pointer isn't the
    // viewport's.
    let ctx = root.ctx();
    state.workshop.viewport_hovered = ctx.pointer_hover_pos().is_some_and(|p| {
        scene.contains(p)
            && ctx
                .layer_id_at(p)
                .is_none_or(|layer| layer.order == egui::Order::Background)
    }) && !ctx.egui_is_using_pointer();

    // Content dropped on the viewport is placed where the cursor points.
    if egui::DragAndDrop::has_payload_of_type::<DraggedItem>(ctx) {
        let over = ctx.pointer_hover_pos().is_some_and(|p| scene.contains(p));
        if over {
            root.painter().rect_stroke(
                scene.shrink(2.0),
                0.0,
                Stroke::new(2.0, theme::BRASS_BRIGHT),
                egui::StrokeKind::Inside,
            );
        }
        if over
            && ctx.input(|i| i.pointer.any_released())
            && let Some(item) = egui::DragAndDrop::take_payload::<DraggedItem>(ctx)
            && let Some(plugin) = state.workshop.plugins.get(item.plugin)
        {
            let name = plugin.manifest.plugin.name.clone();
            state.workshop.place(&name, &item.output, ROOT, true);
        }
    }

    workshop_shortcuts(ctx, state, settings);
    scene
}

fn workshop_shortcuts(
    ctx: &egui::Context,
    state: &mut EditorState,
    settings: &crate::settings::Settings,
) {
    // Gameplay owns its assigned keys while playing; Ctrl editing commands stay available.
    if !ctx.egui_wants_keyboard_input() {
        if !crate::game_ui::gameplay_uses_key(state, settings, "Escape")
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            state.workshop.adjusting = None;
            if state.workshop.playing.is_some() {
                state.workshop.play_request = Some(false);
            }
        }
        if !crate::game_ui::gameplay_uses_key(state, settings, "Delete")
            && ctx.input(|i| i.key_pressed(egui::Key::Delete))
        {
            delete_selection(state);
        }
        let (command, d, c, v, f, a) = ctx.input(|i| {
            (
                i.modifiers.command,
                i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::C),
                i.key_pressed(egui::Key::V),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::A),
            )
        });
        if command && d {
            duplicate_selection(state);
        } else if command && c {
            copy_selection(state);
        } else if command && v {
            paste_clipboard(state);
        } else if command && a && state.workshop.playing.is_none() {
            select_all(state);
        } else if !command && f && state.workshop.playing.is_none() {
            state.workshop.focus_request = state.workshop.selected_node;
        }
    }
}

/// The viewport's toolbar: transform tools, snapping, adding, view options and Play.
fn viewport_toolbar(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    show_surfaces: &mut bool,
    can_play: bool,
    items: usize,
) {
    use crate::manipulate::Tool;
    ui.add_space(8.0);
    let playing = state.workshop.playing.is_some();
    if !playing {
        kit::segmented(
            ui,
            &mut state.workshop.tool,
            &[
                (Tool::Move, "✥ Move", "Move  ·  W"),
                (Tool::Rotate, "⟲ Rotate", "Rotate  ·  E"),
                (Tool::Scale, "⤢ Scale", "Scale  ·  R"),
            ],
        );
        if kit::icon_toggle(
            ui,
            state.workshop.snap,
            "⊞",
            "Snap to 0.5 m, 15° and 0.05  ·  G",
        )
        .clicked()
        {
            state.workshop.snap = !state.workshop.snap;
        }
        ui.add_space(4.0);
        ui.menu_button("+ Add", |ui| {
            ui.label(kit::hint("Behavior node, where the camera looks"));
            for kind in crate::entities::Behavior::KINDS {
                if ui.button(kind).on_hover_text(entity_hint(kind)).clicked() {
                    state.workshop.entity_request = crate::entities::Behavior::new(kind);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Empty group").clicked() {
                let child =
                    state
                        .workshop
                        .scene
                        .add(ROOT, "Group".into(), None, bevy::math::Vec3::ZERO);
                state.workshop.select_only(child);
                ui.close();
            }
        });
    }
    ui.menu_button("View", |ui| {
        ui.checkbox(show_surfaces, "Surfaces")
            .on_hover_text("Tint the cells of 2D collision maps");
        ui.checkbox(&mut state.workshop.debug.colliders, "Behavior rings")
            .on_hover_text("Draw the radius of every behavior");
        ui.checkbox(&mut state.workshop.debug.terrain, "Collision triangles")
            .on_hover_text("Draw the collision triangles of 3D areas");
        ui.separator();
        if ui
            .add(egui::Button::new("Frame selection").shortcut_text(kit::hint("F")))
            .clicked()
        {
            state.workshop.focus_request = state.workshop.selected_node;
            ui.close();
        }
    });

    // Play controls sit in the middle, like a game engine's.
    let middle = ui.max_rect().center().x;
    let width = if playing { 400.0 } else { 90.0 };
    let space = middle - width / 2.0 - ui.cursor().left();
    if space > 8.0 {
        ui.add_space(space);
    } else {
        ui.add_space(8.0);
    }
    if playing {
        let stop = egui::Button::new(RichText::new("■  Stop").color(Color32::WHITE))
            .fill(theme::ERROR.gamma_multiply(0.75));
        if ui
            .add(stop)
            .on_hover_text(kit::tip("Stop", "Esc"))
            .clicked()
        {
            state.workshop.play_request = Some(false);
        }
        let clock = &mut state.workshop.play_clock;
        if kit::icon_toggle(
            ui,
            clock.paused,
            "⏸",
            if clock.paused {
                "Resume  ·  P"
            } else {
                "Pause  ·  P"
            },
        )
        .clicked()
        {
            clock.paused = !clock.paused;
        }
        if kit::icon_button(ui, "⏭", "Run one step  ·  .").clicked() {
            clock.paused = true;
            clock.step_once = true;
        }
        egui::ComboBox::from_id_salt("play_speed")
            .width(64.0)
            .selected_text(format!("{}×", clock.speed))
            .show_ui(ui, |ui| {
                for speed in [0.125f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
                    ui.selectable_value(&mut clock.speed, speed, format!("{speed}×"));
                }
            })
            .response
            .on_hover_text("Speed  ·  [ and ]");
        if ui
            .button("Hit boxes")
            .on_hover_text(kit::tip("Show or hide hit boxes", "H"))
            .clicked()
        {
            state.workshop.hud_requests.push(HudRequest::ToggleHitboxes);
        }
        if ui
            .button("⛶ Game view")
            .on_hover_text(kit::tip("Play in the whole window; Esc comes back", "F11"))
            .clicked()
        {
            state.workshop.game_view = true;
        }
    } else if kit::primary_button(ui, can_play, "▶  Play")
        .on_hover_text(kit::tip(
            "Play the selected character (or the first one in the scene)",
            "F5",
        ))
        .on_disabled_hover_text("Place a playable character first")
        .clicked()
    {
        state.workshop.play_request = Some(true);
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(10.0);
        let caption = match items {
            0 => "Empty scene".to_string(),
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        };
        if ui.available_width() > 80.0 {
            ui.label(kit::hint(caption));
        }
    });
}

/// Offset of duplicated and pasted copies, so they don't hide the original.
const COPY_OFFSET: bevy::math::Vec3 = bevy::math::Vec3::new(1.0, 0.0, 1.0);

fn delete_selection(state: &mut EditorState) {
    let tops = {
        let ws = &state.workshop;
        ws.scene.top_level(&ws.selection())
    };
    for id in &tops {
        state.workshop.scene.remove(*id);
        if state.workshop.adjusting == Some(*id) {
            state.workshop.adjusting = None;
        }
    }
    if !tops.is_empty() {
        state.workshop.selected_node = None;
        state.workshop.selection.clear();
        state.log(
            LogLevel::Info,
            match tops.len() {
                1 => "Removed 1 node".to_string(),
                n => format!("Removed {n} nodes"),
            },
        );
    }
}

pub(crate) fn duplicate_selection(state: &mut EditorState) {
    let ws = &mut state.workshop;
    let tops = ws.scene.top_level(&ws.selection());
    let copies: Vec<NodeId> = tops
        .iter()
        .filter_map(|id| ws.scene.duplicate(*id, COPY_OFFSET))
        .collect();
    if let Some(last) = copies.last().copied() {
        ws.selection = copies;
        ws.selected_node = Some(last);
    }
}

fn copy_selection(state: &mut EditorState) {
    let ws = &mut state.workshop;
    ws.clipboard = ws.scene.copy(&ws.selection());
}

fn paste_clipboard(state: &mut EditorState) {
    let ws = &mut state.workshop;
    if ws.clipboard.is_empty() {
        return;
    }
    // Next to the first copied node, under its original parent if that still exists.
    let parent = ws
        .clipboard
        .first()
        .and_then(|n| n.parent)
        .filter(|p| ws.scene.get(*p).is_some())
        .unwrap_or(ROOT);
    let clipboard = ws.clipboard.clone();
    let pasted = ws.scene.paste(&clipboard, parent, COPY_OFFSET, true);
    if let Some(last) = pasted.last().copied() {
        ws.selection = pasted;
        ws.selected_node = Some(last);
    }
}

pub(crate) fn group_selection(state: &mut EditorState) {
    let ws = &mut state.workshop;
    if let Some(group) = ws.scene.group(&ws.selection()) {
        ws.select_only(group);
    }
}

fn delete_node(state: &mut EditorState, id: NodeId) {
    let name = state
        .workshop
        .scene
        .get(id)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    state.workshop.scene.remove(id);
    state.workshop.selected_node = None;
    if state.workshop.adjusting == Some(id) {
        state.workshop.adjusting = None;
    }
    state.log(LogLevel::Info, format!("Removed {name}"));
}

/// Selects every node but the scene root.
fn select_all(state: &mut EditorState) {
    let ws = &mut state.workshop;
    let all: Vec<NodeId> = ws
        .scene
        .in_tree_order()
        .into_iter()
        .filter(|id| *id != ROOT)
        .collect();
    if let Some(last) = all.last().copied() {
        ws.selection = all;
        ws.selected_node = Some(last);
    }
}

fn bottom_dock(ui: &mut egui::Ui, state: &mut EditorState, browser: &mut Browser) {
    let problems = state
        .log
        .iter()
        .filter(|l| l.level != LogLevel::Info)
        .count();
    let log_label = if problems > 0 {
        format!("Log ({problems})")
    } else {
        "Log".to_string()
    };
    let mut clear = false;
    kit::bar(ui, |ui| {
        ui.add_space(8.0);
        kit::header_tabs(
            ui,
            &mut browser.tab,
            &[
                (BottomTab::Content, "Content".to_string()),
                (BottomTab::Log, log_label),
            ],
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            match browser.tab {
                BottomTab::Content => {
                    if kit::icon_button(
                        ui,
                        "⟳",
                        "Rescan plugins (they are picked up automatically; this checks now)",
                    )
                    .clicked()
                    {
                        state.scan_plugins();
                    }
                }
                BottomTab::Log => {
                    clear = kit::icon_button(ui, "🗑", "Clear the log").clicked();
                    if kit::icon_button(ui, "📋", "Copy the log").clicked() {
                        let text: Vec<&str> = state.log.iter().map(|l| l.text.as_str()).collect();
                        ui.ctx().copy_text(text.join("\n"));
                    }
                }
            }
        });
    });
    if clear {
        state.log.clear();
    }
    kit::body_frame().show(ui, |ui| match browser.tab {
        BottomTab::Content => browser::content_panel(ui, state, browser),
        BottomTab::Log => {
            if state.log.is_empty() {
                ui.label(kit::dim("Nothing logged yet."));
            }
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for line in &state.log {
                        ui.horizontal(|ui| {
                            let (tag, color) = match line.level {
                                LogLevel::Info => ("INFO ", theme::FAINT),
                                LogLevel::Warn => ("WARN ", theme::WARN),
                                LogLevel::Error => ("ERROR", theme::ERROR),
                            };
                            ui.label(RichText::new(tag).monospace().color(color));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&line.text)
                                        .monospace()
                                        .color(log_color(ui, line.level)),
                                )
                                .wrap(),
                            );
                        });
                    }
                });
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Scene tree

/// Things the tree asked for, applied after drawing.
#[derive(Default)]
struct TreeActions {
    /// A node clicked, and whether it adds to the selection (Ctrl or Shift held).
    select: Option<(NodeId, bool)>,
    toggle_visible: Option<NodeId>,
    toggle_lock: Option<NodeId>,
    duplicate: bool,
    group: bool,
    reparent: Option<(NodeId, NodeId)>,
    place: Option<(usize, String, NodeId)>,
    delete: Option<NodeId>,
    /// Point the camera at this node (double-click or Frame).
    focus: Option<NodeId>,
    /// Empty space was clicked: select nothing.
    clear: bool,
    new_group: bool,
}

fn scene_tree(ui: &mut egui::Ui, state: &mut EditorState, extractions: &Extractions) {
    let mut actions = TreeActions::default();
    let filter_id = egui::Id::new("scene_tree_filter");
    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    kit::panel_header(ui, "Scene", |ui| {
        if kit::icon_button(ui, "📁", "New empty group").clicked() {
            actions.new_group = true;
        }
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            kit::search_field(
                ui,
                &mut filter,
                "Filter nodes…",
                ui.available_width() - 14.0,
            );
        });
    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    let filter = filter.to_lowercase();

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for id in state.workshop.scene.in_tree_order() {
                let depth = depth_of(&state.workshop.scene.nodes, id);
                let Some(node) = state.workshop.scene.get(id) else {
                    continue;
                };
                if !filter.is_empty() && !node.name.to_lowercase().contains(&filter) {
                    continue;
                }
                let depth = if filter.is_empty() { depth } else { 0 };
                tree_row(ui, state, node, depth, extractions, &mut actions);
            }
            if state.workshop.scene.nodes.len() == 1 {
                ui.add_space(12.0);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(12, 0))
                    .show(ui, |ui| {
                        ui.label(kit::hint(
                            "Drag content from the browser onto the viewport, or onto a node \
                             here to make it a child.",
                        ));
                    });
            }
            // Clicking empty space clears the selection.
            let rest = ui.allocate_response(
                vec2(ui.available_width(), ui.available_height().max(24.0)),
                egui::Sense::click(),
            );
            if rest.clicked() {
                actions.clear = true;
            }
        });

    let ws = &mut state.workshop;
    if actions.clear {
        ws.selected_node = None;
        ws.selection.clear();
    }
    if actions.new_group {
        let parent = ws.selected_node.unwrap_or(ROOT);
        let child = ws
            .scene
            .add(parent, "Group".into(), None, bevy::math::Vec3::ZERO);
        ws.select_only(child);
    }
    if let Some((id, additive)) = actions.select {
        if additive {
            ws.toggle_select(id);
        } else {
            ws.select_only(id);
        }
        if ws.adjusting != Some(id) {
            ws.adjusting = None;
        }
    }
    if let Some(id) = actions.focus {
        ws.focus_request = Some(id);
    }
    if let Some(id) = actions.toggle_lock
        && let Some(node) = ws.scene.get_mut(id)
    {
        node.locked = !node.locked;
        ws.scene.dirty = true;
    }
    if actions.duplicate {
        duplicate_selection(state);
    }
    if actions.group {
        group_selection(state);
    }
    let ws = &mut state.workshop;
    if let Some(id) = actions.toggle_visible
        && let Some(node) = ws.scene.get_mut(id)
    {
        node.visible = !node.visible;
        ws.scene.dirty = true;
    }
    if let Some((id, parent)) = actions.reparent
        && !ws.scene.reparent(id, parent)
    {
        state.log(
            LogLevel::Warn,
            "A node can't go under itself or its own children",
        );
    }
    if let Some((plugin, output, parent)) = actions.place
        && let Some(p) = state.workshop.plugins.get(plugin)
    {
        let name = p.manifest.plugin.name.clone();
        state.workshop.place(&name, &output, parent, false);
    }
    if let Some(id) = actions.delete {
        if state.workshop.is_selected(id) {
            delete_selection(state);
        } else {
            delete_node(state, id);
        }
    }
}

/// The glyph standing for what a node is: the scene, a group, a behavior, a character, a model,
/// a sprite or a picture.
fn node_icon(node: &Node, extractions: &Extractions) -> &'static str {
    if node.id == ROOT {
        return "◆";
    }
    if !node.behaviors.is_empty() && node.item.is_none() {
        return "◎";
    }
    let Some(item) = &node.item else {
        return "📁";
    };
    let Some(x) = extractions.get(&item.plugin) else {
        return "◇";
    };
    if let Some(m) = x.model(&item.output) {
        if m.character.is_some() { "☻" } else { "⬢" }
    } else if let Some(s) = x.sprite(&item.output) {
        if s.character.is_some() { "☻" } else { "◩" }
    } else {
        "▦"
    }
}

fn depth_of(nodes: &[Node], id: NodeId) -> usize {
    let mut depth = 0;
    let mut current = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
    while let Some(parent) = current {
        depth += 1;
        current = nodes.iter().find(|n| n.id == parent).and_then(|n| n.parent);
    }
    depth
}

fn tree_row(
    ui: &mut egui::Ui,
    state: &EditorState,
    node: &Node,
    depth: usize,
    extractions: &Extractions,
    actions: &mut TreeActions,
) {
    let selected = state.workshop.is_selected(node.id);
    let primary = state.workshop.selected_node == Some(node.id);
    let shown = state.workshop.scene.is_shown(node.id);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::click_and_drag());
    let hovered = response.hovered();
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, theme::SELECTED);
        if primary {
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                0.0,
                theme::BRASS,
            );
        }
    } else if hovered {
        painter.rect_filled(rect, 0.0, theme::RAISED);
    }

    let x = rect.left() + 10.0 + depth as f32 * 14.0;
    let text_color = if !shown {
        theme::FAINT
    } else if selected {
        theme::BRASS_BRIGHT
    } else {
        theme::PARCHMENT
    };
    painter.text(
        egui::pos2(x + 7.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        node_icon(node, extractions),
        egui::FontId::proportional(12.0),
        if node.id == ROOT {
            theme::BRASS
        } else if shown {
            theme::PARCHMENT_DIM
        } else {
            theme::FAINT
        },
    );
    let name_font = if node.id == ROOT {
        theme::strong(13.0)
    } else {
        egui::FontId::proportional(13.0)
    };
    // Room is left for the lock and visibility toggles.
    kit::text_truncated(
        painter,
        egui::pos2(x + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &node.name,
        name_font,
        text_color,
        rect.right() - 52.0 - (x + 20.0),
    );

    // Lock and visibility toggles on the right: always shown when they are not the default,
    // otherwise only on hover.
    let mut right = rect.right() - 6.0;
    let mut toggle = |ui: &mut egui::Ui, on: bool, icon_on: &str, icon_off: &str, tip: &str| {
        let r = egui::Rect::from_center_size(
            egui::pos2(right - 10.0, rect.center().y),
            vec2(20.0, 20.0),
        );
        right -= 22.0;
        let resp = ui.interact(r, ui.id().with((node.id, icon_on)), egui::Sense::click());
        if on || hovered || resp.hovered() {
            if resp.hovered() {
                ui.painter().rect_filled(r, 3.0, theme::RAISED_HOVER);
            }
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                if on { icon_on } else { icon_off },
                egui::FontId::proportional(11.5),
                if on {
                    theme::BRASS
                } else {
                    theme::PARCHMENT_DIM
                },
            );
        }
        resp.on_hover_text(tip).clicked()
    };
    if toggle(
        ui,
        !node.visible,
        "◌",
        "👁",
        if node.visible { "Hide" } else { "Show" },
    ) {
        actions.toggle_visible = Some(node.id);
    }
    if node.id != ROOT
        && toggle(
            ui,
            node.locked,
            "🔒",
            "🔓",
            if node.locked {
                "Unlock: it can be picked and moved in the viewport"
            } else {
                "Lock: keep it from being picked or moved in the viewport"
            },
        )
    {
        actions.toggle_lock = Some(node.id);
    }

    if response.clicked() {
        let additive = ui.input(|i| i.modifiers.command || i.modifiers.shift);
        actions.select = Some((node.id, additive));
    }
    if response.double_clicked() {
        actions.focus = Some(node.id);
    }
    let response = response.on_hover_text_at_pointer(if node.id == ROOT {
        "The scene".to_string()
    } else {
        format!("{}\nDouble-click to frame it", node.name)
    });
    if node.id != ROOT {
        response.dnd_set_drag_payload(DraggedNode(node.id));
        response.context_menu(|ui| {
            if !state.workshop.is_selected(node.id) {
                actions.select = Some((node.id, false));
            }
            if ui
                .add(egui::Button::new("Frame").shortcut_text(kit::hint("F")))
                .clicked()
            {
                actions.focus = Some(node.id);
                ui.close();
            }
            if ui
                .add(egui::Button::new("Duplicate").shortcut_text(kit::hint("Ctrl+D")))
                .clicked()
            {
                actions.duplicate = true;
                ui.close();
            }
            if ui.button("Group selection").clicked() {
                actions.group = true;
                ui.close();
            }
            if ui
                .button(if node.locked { "Unlock" } else { "Lock" })
                .clicked()
            {
                actions.toggle_lock = Some(node.id);
                ui.close();
            }
            ui.separator();
            if ui
                .add(
                    egui::Button::new(RichText::new("Delete").color(theme::ERROR))
                        .shortcut_text(kit::hint("Del")),
                )
                .clicked()
            {
                actions.delete = Some(node.id);
                ui.close();
            }
        });
    }

    // Drop targets: another node (reparent) or content from the browser (place as child).
    let hovering = response.dnd_hover_payload::<DraggedNode>().is_some()
        || response.dnd_hover_payload::<DraggedItem>().is_some();
    if hovering {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            3.0,
            Stroke::new(1.5, theme::BRASS_BRIGHT),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(dragged) = response.dnd_release_payload::<DraggedNode>()
        && dragged.0 != node.id
    {
        actions.reparent = Some((dragged.0, node.id));
    }
    if let Some(item) = response.dnd_release_payload::<DraggedItem>() {
        actions.place = Some((item.plugin, item.output.clone(), node.id));
    }
}

// ---------------------------------------------------------------------------------------------
// Adjust-after-drop panel

/// Play mode's debug readout: what the played character and the scene's behaviors are doing.
/// The game's own HUD is drawn in the viewport by `clockwork_runtime::hud`.
fn live_panel(ctx: &egui::Context, viewport: egui::Rect, state: &EditorState) {
    if state.workshop.playing.is_none() || state.workshop.live.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("live_panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(viewport.right_top() + vec2(-12.0, 40.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(190))
                .stroke(Stroke::new(1.0, theme::BRASS_DIM))
                .corner_radius(4)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(
                        RichText::new("Live")
                            .font(theme::display(14.0))
                            .color(theme::BRASS),
                    );
                    for line in &state.workshop.live {
                        ui.small(RichText::new(line).monospace().color(theme::PARCHMENT_DIM));
                    }
                });
        });
}

/// Blender-style "adjust last operation" panel: after an item is placed, edit where it went
/// without a dialog up front. It stays until Done, Escape, or selecting something else.
fn adjust_panel(ctx: &egui::Context, viewport: egui::Rect, state: &mut EditorState) {
    let Some(id) = state.workshop.adjusting else {
        return;
    };
    if state.workshop.scene.get(id).is_none() {
        state.workshop.adjusting = None;
        return;
    }
    let mut done = false;
    egui::Area::new(egui::Id::new("adjust_panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(viewport.left_bottom() + vec2(12.0, -12.0))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.0, theme::BRASS_DIM))
                .corner_radius(4)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(250.0);
                    let name = state
                        .workshop
                        .scene
                        .get(id)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Placed")
                                .font(theme::display(15.0))
                                .color(theme::BRASS),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Done").clicked() {
                                done = true;
                            }
                        });
                    });
                    ui.small(RichText::new(name).color(theme::PARCHMENT_DIM));
                    ui.add_space(4.0);
                    transform_editor(ui, state, id, "adjust");
                });
        });
    if done {
        state.workshop.adjusting = None;
    }
}

/// Position, height, rotation, scale and parent of a node. Edits apply live.
fn transform_editor(ui: &mut egui::Ui, state: &mut EditorState, id: NodeId, salt: &str) {
    let scene = &mut state.workshop.scene;
    let parents: Vec<(NodeId, String)> = {
        let below = scene.subtree(id);
        scene
            .in_tree_order()
            .into_iter()
            .filter(|n| !below.contains(n))
            .filter_map(|n| scene.get(n).map(|node| (n, node.name.clone())))
            .collect()
    };
    let Some(node) = scene.get_mut(id) else {
        return;
    };
    let mut changed = false;
    let mut new_parent = None;
    ui.push_id(("transform", salt), |ui| {
        kit::prop(ui, "Position", |ui| {
            let w = ((ui.available_width() - 12.0) / 3.0).floor().max(48.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            changed |= kit::axis_drag(ui, "X", theme::AXIS_X, &mut node.position[0], 0.1, "", w);
            changed |= kit::axis_drag(ui, "Y", theme::AXIS_Y, &mut node.position[1], 0.05, "", w);
            changed |= kit::axis_drag(ui, "Z", theme::AXIS_Z, &mut node.position[2], 0.1, "", w);
        });
        kit::prop(ui, "Rotation", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut node.yaw)
                        .speed(1.0)
                        .suffix("°")
                        .max_decimals(1),
                )
                .on_hover_text("Turn around the vertical axis")
                .changed();
        });
        kit::prop(ui, "Scale", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut node.scale)
                        .speed(0.01)
                        .range(0.01..=100.0)
                        .max_decimals(2)
                        .prefix("×"),
                )
                .changed();
            if node.scale != 1.0 && kit::icon_button(ui, "↺", "Reset to 1").clicked() {
                node.scale = 1.0;
                changed = true;
            }
        });

        if id != ROOT {
            kit::prop(ui, "Parent", |ui| {
                let current = node.parent.unwrap_or(ROOT);
                let current_name = parents
                    .iter()
                    .find(|(p, _)| *p == current)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt(("parent", salt))
                    .width(ui.available_width() - 30.0)
                    .selected_text(current_name)
                    .show_ui(ui, |ui| {
                        for (p, name) in &parents {
                            if ui.selectable_label(*p == current, name).clicked() && *p != current {
                                new_parent = Some(*p);
                            }
                        }
                    });
            });
        }
    });
    if changed {
        scene.dirty = true;
    }
    if let Some(parent) = new_parent {
        scene.reparent(id, parent);
    }
}

// ---------------------------------------------------------------------------------------------
// Inspector

fn inspector(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    browser: &mut Browser,
    extractions: &Extractions,
) {
    let count = state.workshop.selection().len();
    kit::panel_header(ui, "Inspector", |ui| {
        if count > 1 {
            ui.label(kit::hint(format!("{count} selected")));
        }
    });
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            kit::body_frame().show(ui, |ui| {
                if let Some(id) = state.workshop.selected_node
                    && state.workshop.scene.get(id).is_some()
                {
                    node_inspector(ui, state, id, extractions);
                } else if let Some(at) = selected_asset(state, browser) {
                    crate::asset_ui::asset_section(ui, state, browser, &at);
                    if let Some(index) = state.workshop.plugin_index(&at.plugin) {
                        ui.add_space(8.0);
                        plugin_inspector(ui, state, index, browser);
                    }
                } else if let Some(index) = state.workshop.selected {
                    plugin_inspector(ui, state, index, browser);
                } else {
                    kit::empty_state(
                        ui,
                        "Nothing selected",
                        "Pick a node in the scene, or content below.",
                    );
                }
            });
        });
}

/// The asset selected in the browser, while the Inspector shows its plugin (or it is built in).
fn selected_asset(state: &EditorState, browser: &Browser) -> Option<OutputRef> {
    let (plugin, output) = browser.selected.as_ref()?;
    (state.workshop.plugin_index(plugin) == state.workshop.selected)
        .then(|| OutputRef::new(plugin, output))
}

fn node_inspector(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    id: NodeId,
    extractions: &Extractions,
) {
    let Some(node) = state.workshop.scene.get(id).cloned() else {
        return;
    };

    // Header: the node's kind and its name.
    let mut delete = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(node_icon(&node, extractions))
                .size(16.0)
                .color(theme::BRASS),
        );
        let mut name = node.name.clone();
        if id == ROOT {
            ui.label(RichText::new(&name).font(theme::strong(15.0)));
        } else if ui
            .add(
                egui::TextEdit::singleline(&mut name)
                    .font(theme::strong(14.0))
                    .desired_width(ui.available_width() - 64.0),
            )
            .on_hover_text("Name")
            .changed()
            && let Some(n) = state.workshop.scene.get_mut(id)
        {
            n.name = name;
            state.workshop.scene.dirty = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if id != ROOT && kit::icon_button(ui, "🗑", "Delete  ·  Del").clicked() {
                delete = true;
            }
            let visible = node.visible;
            if kit::icon_toggle(ui, !visible, if visible { "👁" } else { "◌" }, "Visible").clicked()
                && let Some(n) = state.workshop.scene.get_mut(id)
            {
                n.visible = !visible;
                state.workshop.scene.dirty = true;
            }
        });
    });
    if delete {
        delete_node(state, id);
        return;
    }
    ui.add_space(4.0);

    // What it shows.
    if let Some(item) = &node.item {
        let plugin = state
            .workshop
            .plugin_index(&item.plugin)
            .map(|i| &state.workshop.plugins[i]);
        let game = match plugin {
            Some(p) if p.compose.is_some() => Some("Composition of other plugins".to_string()),
            Some(p) => state.rom_for(p).map(|e| state.title(e)),
            None => None,
        };
        kit::foldout(ui, ("source", id), "Source", true, |ui| {
            kit::prop(ui, "Plugin", |ui| {
                if ui
                    .link(&item.plugin)
                    .on_hover_text("Open in the Plugins screen")
                    .clicked()
                {
                    state.screen = crate::state::Screen::Plugins;
                    state.plugin_editor.selected = Some(item.plugin.clone());
                }
            });
            kit::prop(ui, "Output", |ui| ui.label(&item.output));
            kit::prop(ui, "Game", |ui| {
                ui.label(game.as_deref().unwrap_or("not in Library"))
            });
            if extractions.is_pending(&item.plugin) {
                ui.label(kit::dim("Extracting…"));
            } else if let Some(error) = extractions.error(&item.plugin) {
                ui.label(RichText::new(format!("Can't show: {error}")).color(theme::ERROR));
            }
        });
    } else if id != ROOT && node.behaviors.is_empty() {
        ui.label(kit::hint("An empty node: a group for other nodes."));
    }
    // The scene is the game: its own settings.
    if id == ROOT {
        kit::foldout(ui, ("game", id), "Game", true, |ui| {
            crate::asset_ui::game_settings(ui, state);
        });
    }

    kit::foldout(ui, ("transform", id), "Transform", true, |ui| {
        transform_editor(ui, state, id, "inspector");
        ui.horizontal(|ui| {
            ui.add_space(kit::LABEL_W + 10.0);
            if ui.button("+ Child group").clicked() {
                let child =
                    state
                        .workshop
                        .scene
                        .add(id, "Group".into(), None, bevy::math::Vec3::ZERO);
                state.workshop.select_only(child);
            }
            if ui
                .button("Frame")
                .on_hover_text(kit::tip("Point the camera at it", "F"))
                .clicked()
            {
                state.workshop.focus_request = Some(id);
            }
        });
    });

    if id != ROOT {
        let title = match node.behaviors.as_slice() {
            [] => "Behavior".to_string(),
            [one] => format!("Behavior · {}", one.behavior.kind()),
            many => format!("Behaviors · {}", many.len()),
        };
        kit::foldout(
            ui,
            ("behavior", id),
            &title,
            !node.behaviors.is_empty(),
            |ui| {
                behavior_section(ui, state, id, extractions);
            },
        );
    }

    // Animations of a model or sprite.
    let animations: Vec<String> = node
        .item
        .as_ref()
        .and_then(|item| extractions.get(&item.plugin).zip(Some(item)))
        .map(|(x, item)| {
            if let Some(m) = x.model(&item.output) {
                m.model.animations.iter().map(|a| a.name.clone()).collect()
            } else if let Some(s) = x.sprite(&item.output) {
                s.sprite.animations.iter().map(|a| a.name.clone()).collect()
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default();
    // A character's camera when played: its own, or another camera asset.
    let playable = node.item.as_ref().is_some_and(|item| {
        extractions.get(&item.plugin).is_some_and(|x| {
            x.model(&item.output).is_some_and(|m| m.character.is_some())
                || x.sprite(&item.output)
                    .is_some_and(|s| s.character.is_some())
        })
    });
    if playable {
        kit::foldout(ui, ("play", id), "Play", true, |ui| {
            crate::asset_ui::node_overrides(ui, state, id);
        });
    }
    if !animations.is_empty() {
        kit::foldout(ui, ("animation", id), "Animation", true, |ui| {
            let current = node
                .animation
                .clone()
                .unwrap_or_else(|| animations[0].clone());
            kit::prop(ui, "Plays", |ui| {
                egui::ComboBox::from_id_salt("node_animation")
                    .width(ui.available_width() - 30.0)
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for name in &animations {
                            if ui.selectable_label(*name == current, name).clicked()
                                && let Some(n) = state.workshop.scene.get_mut(id)
                            {
                                n.animation = Some(name.clone());
                                state.workshop.scene.dirty = true;
                            }
                        }
                    });
            });
            ui.label(kit::hint(format!("{} animations", animations.len())));
        });
    }

    // Surfaces of the item, with the cell under the cursor.
    if let Some(item) = &node.item
        && let Some(map) = extractions
            .get(&item.plugin)
            .and_then(|x| surfaces_for(x, &item.output))
    {
        let hovered = state
            .workshop
            .hovered_cell
            .filter(|(n, ..)| *n == id)
            .map(|(_, x, y)| (x, y));
        kit::foldout(ui, ("surfaces", id), "Surfaces", true, |ui| {
            surfaces_section(ui, map, hovered);
        });
    }
}

fn entity_hint(kind: &str) -> &'static str {
    match kind {
        "Target" => "Something that can be hit and destroyed, and may drop an item",
        "Pickup" => "Gives an item when the player walks into it",
        "Trigger" => "Does something when the player walks into it",
        "Checkpoint" => "Where the player comes back after falling out of the world",
        _ => "Blocks movement",
    }
}

/// The items every extracted plugin offers: (id, name).
fn known_items(state: &EditorState, extractions: &Extractions) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for plugin in &state.workshop.plugins {
        let Some(x) = extractions.get(&plugin.manifest.plugin.name) else {
            continue;
        };
        for (_, items) in &x.items {
            for item in items {
                if !out.iter().any(|(id, _)| *id == item.id) {
                    out.push((item.id.clone(), item.name.clone()));
                }
            }
        }
    }
    out
}

/// A drop-down of known items that also takes an id typed in. True if it changed.
fn item_picker(
    ui: &mut egui::Ui,
    salt: &str,
    value: &mut String,
    known: &[(String, String)],
) -> bool {
    let mut changed = false;
    let shown = known
        .iter()
        .find(|(id, _)| id == value)
        .map_or(value.clone(), |(_, name)| name.clone());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(if shown.is_empty() {
            "(choose)".to_string()
        } else {
            shown
        })
        .show_ui(ui, |ui| {
            for (id, name) in known {
                if ui.selectable_label(id == value, name).clicked() {
                    *value = id.clone();
                    changed = true;
                }
            }
        });
    if known.is_empty() {
        changed |= ui.text_edit_singleline(value).changed();
    }
    changed
}

/// What the node does when played, with its properties.
fn behavior_section(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    id: NodeId,
    extractions: &Extractions,
) {
    use crate::entities::{Behavior, NodeBehavior};
    let Some(node) = state.workshop.scene.get(id).cloned() else {
        return;
    };
    let known = known_items(state, extractions);
    let names: Vec<String> = state
        .workshop
        .scene
        .nodes
        .iter()
        .filter(|n| n.id != ROOT && n.id != id)
        .map(|n| n.name.clone())
        .collect();

    // Each behavior: its kind, where it comes from, its fields and when it works.
    enum Change {
        Up(usize),
        Down(usize),
        Remove(usize),
    }
    let mut change = None;
    let count = node.behaviors.len();
    for (k, original) in node.behaviors.iter().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}. {}", k + 1, original.behavior.kind())).strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::icon_button(ui, "✕", "Remove this behavior").clicked() {
                        change = Some(Change::Remove(k));
                    }
                    if k + 1 < count && kit::icon_button(ui, "↓", "Move down").clicked() {
                        change = Some(Change::Down(k));
                    }
                    if k > 0 && kit::icon_button(ui, "↑", "Move up").clicked() {
                        change = Some(Change::Up(k));
                    }
                });
            });
            crate::asset_ui::behavior_link(ui, state, id, k);
            let mut entry = original.clone();
            ui.scope(|ui| {
                if entry.from.is_some() {
                    ui.disable();
                }
                behavior_editor(ui, state, (id, k), &mut entry.behavior, &known, &names);
            });
            kit::prop(ui, "Only when", |ui| {
                condition_choice(ui, ("when", id, k), &mut entry.when, &known, &names);
            });
            let mut reach = entry.height.is_some();
            ui.horizontal(|ui| {
                ui.add_space(kit::LABEL_W + 10.0);
                if ui
                    .checkbox(&mut reach, "Its own reach up and down")
                    .on_hover_text("Without, it reaches its radius above and below, at least 2 m")
                    .changed()
                {
                    entry.height = reach.then(|| entry.reach());
                }
            });
            if let Some(height) = &mut entry.height {
                kit::prop(ui, "Reach (m)", |ui| {
                    ui.add(egui::DragValue::new(height).speed(0.05).range(0.1..=100.0));
                });
            }
            if entry != *original
                && let Some(n) = state.workshop.scene.get_mut(id)
                && let Some(slot) = n.behaviors.get_mut(k)
            {
                let kind_changed = slot.behavior.kind() != entry.behavior.kind();
                *slot = entry;
                if kind_changed {
                    // Another shape is needed in the viewport.
                    state.workshop.scene.changed();
                } else {
                    state.workshop.scene.dirty = true;
                }
            }
        });
    }
    if let Some(change) = change
        && let Some(n) = state.workshop.scene.get_mut(id)
    {
        match change {
            Change::Up(k) => n.behaviors.swap(k, k - 1),
            Change::Down(k) => n.behaviors.swap(k, k + 1),
            Change::Remove(k) => {
                n.behaviors.remove(k);
            }
        }
        state.workshop.scene.changed();
    }

    // A new one, of any kind.
    let mut added = None;
    egui::ComboBox::from_id_salt(("add_behavior", id))
        .selected_text("+ Add behavior")
        .show_ui(ui, |ui| {
            for kind in Behavior::KINDS {
                if ui
                    .selectable_label(false, kind)
                    .on_hover_text(entity_hint(kind))
                    .clicked()
                {
                    added = Behavior::new(kind);
                }
            }
        });
    if let Some(behavior) = added
        && let Some(n) = state.workshop.scene.get_mut(id)
    {
        n.behaviors.push(NodeBehavior::new(behavior));
        state.workshop.scene.changed();
    }

    // When the node itself is there.
    let mut acts = node.acts.clone();
    kit::prop(ui, "In acts", |ui| {
        ui.horizontal_wrapped(|ui| {
            let mut all = acts.is_none();
            if ui.checkbox(&mut all, "All").changed() {
                acts = if all { None } else { Some((1..=6).collect()) };
            }
            if let Some(acts) = &mut acts {
                let last = acts.iter().copied().max().unwrap_or(6).max(6);
                for act in 1..=last {
                    let mut on = acts.contains(&act);
                    if ui.checkbox(&mut on, act.to_string()).changed() {
                        if on {
                            acts.push(act);
                            acts.sort_unstable();
                        } else {
                            acts.retain(|a| *a != act);
                        }
                    }
                }
            }
        });
    });
    if acts != node.acts
        && let Some(n) = state.workshop.scene.get_mut(id)
    {
        n.acts = acts;
        state.workshop.scene.dirty = true;
    }
    let mut appears = node.appears.clone();
    kit::prop(ui, "Appears", |ui| {
        condition_choice(ui, ("appears", id, 0), &mut appears, &known, &names);
    });
    if appears != node.appears
        && let Some(n) = state.workshop.scene.get_mut(id)
    {
        n.appears = appears;
        state.workshop.scene.dirty = true;
    }
}

/// A condition, or none ("Always").
fn condition_choice(
    ui: &mut egui::Ui,
    salt: (&str, NodeId, usize),
    value: &mut Option<crate::entities::Condition>,
    known: &[(String, String)],
    names: &[String],
) {
    ui.vertical(|ui| {
        let mut on = value.is_some();
        egui::ComboBox::from_id_salt(salt)
            .selected_text(if on { "a condition holds" } else { "Always" })
            .show_ui(ui, |ui| {
                if ui.selectable_label(!on, "Always").clicked() {
                    on = false;
                }
                if ui.selectable_label(on, "A condition holds").clicked() {
                    on = true;
                }
            });
        match (on, value.is_some()) {
            (true, false) => {
                *value = Some(crate::entities::Condition::Has {
                    item: known.first().map_or(String::new(), |k| k.0.clone()),
                    count: 1,
                })
            }
            (false, true) => *value = None,
            _ => {}
        }
        if let Some(condition) = value {
            condition_editor(
                ui,
                &format!("{}_{}_{}", salt.0, salt.1, salt.2),
                condition,
                known,
                names,
            );
        }
    });
}

/// Edits a condition, and those inside it.
fn condition_editor(
    ui: &mut egui::Ui,
    salt: &str,
    condition: &mut crate::entities::Condition,
    known: &[(String, String)],
    names: &[String],
) {
    use crate::entities::Condition;
    let kinds = [
        ("Has", "The player carries"),
        ("Flag", "A flag is up"),
        ("Gone", "A node is gone"),
        ("Act", "In act"),
        ("Not", "Not"),
        ("All", "All of"),
        ("Any", "Any of"),
    ];
    let current = match condition {
        Condition::Has { .. } => "Has",
        Condition::Flag(_) => "Flag",
        Condition::Gone(_) => "Gone",
        Condition::Act(_) => "Act",
        Condition::Not(_) => "Not",
        Condition::All(_) => "All",
        Condition::Any(_) => "Any",
    };
    let mut chosen = current;
    egui::ComboBox::from_id_salt((salt, "kind"))
        .selected_text(kinds.iter().find(|k| k.0 == current).map_or("", |k| k.1))
        .show_ui(ui, |ui| {
            for (kind, label) in kinds {
                if ui.selectable_label(kind == current, label).clicked() {
                    chosen = kind;
                }
            }
        });
    if chosen != current {
        let item = known.first().map_or(String::new(), |k| k.0.clone());
        *condition = match chosen {
            "Has" => Condition::Has { item, count: 1 },
            "Flag" => Condition::Flag(String::new()),
            "Gone" => Condition::Gone(names.first().cloned().unwrap_or_default()),
            "Act" => Condition::Act(1),
            "Not" => Condition::Not(Box::new(condition.clone())),
            "All" => Condition::All(vec![condition.clone()]),
            _ => Condition::Any(vec![condition.clone()]),
        };
    }
    match condition {
        Condition::Has { item, count } => {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(count).range(0..=999));
                item_picker(ui, &format!("{salt}_item"), item, known);
            });
        }
        Condition::Flag(name) => {
            ui.add(egui::TextEdit::singleline(name).hint_text("flag name"));
        }
        Condition::Act(act) => {
            ui.add(egui::DragValue::new(act).range(1..=u8::MAX));
        }
        Condition::Gone(node) => {
            egui::ComboBox::from_id_salt((salt, "gone"))
                .selected_text(if node.is_empty() {
                    "(choose a node)"
                } else {
                    node.as_str()
                })
                .show_ui(ui, |ui| {
                    for name in names {
                        if ui.selectable_label(name == node, name).clicked() {
                            *node = name.clone();
                        }
                    }
                });
        }
        Condition::Not(inner) => {
            ui.indent((salt, "not"), |ui| {
                condition_editor(ui, &format!("{salt}_not"), inner, known, names);
            });
        }
        Condition::All(list) | Condition::Any(list) => {
            let mut remove = None;
            ui.indent((salt, "list"), |ui| {
                for (k, inner) in list.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if kit::icon_button(ui, "✕", "Remove").clicked() {
                            remove = Some(k);
                        }
                        ui.vertical(|ui| {
                            condition_editor(ui, &format!("{salt}_{k}"), inner, known, names);
                        });
                    });
                }
                if ui.button("+ Condition").clicked() {
                    list.push(Condition::Flag(String::new()));
                }
            });
            if let Some(k) = remove {
                list.remove(k);
            }
        }
    }
}

/// The fields of one behavior, its kind among them.
fn behavior_editor(
    ui: &mut egui::Ui,
    state: &EditorState,
    (id, k): (NodeId, usize),
    behavior: &mut crate::entities::Behavior,
    known: &[(String, String)],
    names: &[String],
) {
    use crate::entities::Behavior;
    kit::prop(ui, "Kind", |ui| {
        egui::ComboBox::from_id_salt(("behavior_kind", id, k))
            .width(ui.available_width() - 30.0)
            .selected_text(behavior.kind())
            .show_ui(ui, |ui| {
                for kind in Behavior::KINDS {
                    let on = behavior.kind() == kind;
                    if ui
                        .selectable_label(on, kind)
                        .on_hover_text(entity_hint(kind))
                        .clicked()
                        && !on
                        && let Some(new) = Behavior::new(kind)
                    {
                        *behavior = new;
                    }
                }
            });
    });

    let drag =
        |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>| {
            kit::prop(ui, label, |ui| {
                ui.add(
                    egui::DragValue::new(value)
                        .speed(0.05)
                        .range(range)
                        .max_decimals(2),
                );
            });
        };
    let count_drag = |ui: &mut egui::Ui, value: &mut u32| {
        kit::prop(ui, "Count", |ui| {
            ui.add(egui::DragValue::new(value).range(1..=99));
        });
    };
    match behavior {
        Behavior::Target {
            health,
            radius,
            solid,
            drop,
        } => {
            drag(ui, "Health", health, 0.5..=100.0);
            drag(ui, "Radius", radius, 0.1..=10.0);
            ui.checkbox(solid, "Blocks movement");
            let mut drops = drop.is_some();
            if ui.checkbox(&mut drops, "Drops an item").changed() {
                *drop = drops.then(|| (known.first().map_or(String::new(), |k| k.0.clone()), 1));
            }
            if let Some((item, count)) = drop {
                item_picker(ui, &format!("drop_{id}_{k}"), item, known);
                count_drag(ui, count);
            }
        }
        Behavior::Pickup {
            item,
            count,
            radius,
        } => {
            kit::prop(ui, "Item", |ui| {
                item_picker(ui, &format!("pickup_{id}_{k}"), item, known);
            });
            count_drag(ui, count);
            drag(ui, "Radius", radius, 0.1..=10.0);
        }
        Behavior::Trigger {
            radius,
            effect,
            once,
        } => {
            drag(ui, "Radius", radius, 0.1..=50.0);
            ui.checkbox(once, "Only the first time");
            effect_editor(ui, state, &format!("effect_{id}_{k}"), effect, known, names);
        }
        Behavior::Checkpoint { radius } | Behavior::Solid { radius } => {
            drag(ui, "Radius", radius, 0.1..=50.0);
        }
        Behavior::Hazard { radius, damage } => {
            drag(ui, "Radius", radius, 0.1..=50.0);
            drag(ui, "Damage", damage, 0.25..=100.0);
        }
        Behavior::Door { radius, to, fade } => {
            drag(ui, "Radius", radius, 0.1..=50.0);
            kit::prop(ui, "Leads to", |ui| {
                egui::ComboBox::from_id_salt(("door_to", id, k))
                    .selected_text(if to.is_empty() {
                        "(choose a node)"
                    } else {
                        to.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for name in names {
                            if ui.selectable_label(name == to, name).clicked() {
                                *to = name.clone();
                            }
                        }
                    });
            });
            drag(ui, "Fade (s)", fade, 0.0..=5.0);
        }
    }
}

/// Makes an effect of a kind, from the known items and node names.
type NewEffect = fn(&[(String, String)], &[String]) -> crate::entities::Effect;

/// What a trigger does: one effect, or several in order.
fn effect_editor(
    ui: &mut egui::Ui,
    state: &EditorState,
    salt: &str,
    effect: &mut crate::entities::Effect,
    known: &[(String, String)],
    names: &[String],
) {
    use crate::entities::Effect;
    let choices: [(&str, NewEffect); 11] = [
        ("Show a message", |_, _| Effect::Message("Hello!".into())),
        ("Teleport the player", |_, names| {
            Effect::Teleport(names.first().cloned().unwrap_or_default())
        }),
        ("Give an item", |known, _| {
            Effect::Give(known.first().map_or(String::new(), |k| k.0.clone()), 1)
        }),
        ("Restart at the start", |_, _| Effect::Restart),
        ("Hurt the player", |_, _| Effect::Hurt(1.0)),
        ("Heal the player", |_, _| Effect::Heal(1.0)),
        ("Show a dialog", |_, _| {
            Effect::Say(clockwork_core::asset::Slot::Inline(
                clockwork_core::ui::Dialog {
                    speaker: None,
                    pages: vec!["Hello!".into()],
                },
            ))
        }),
        ("Play a sound", |_, _| Effect::Sound(String::new())),
        ("Raise a flag", |_, _| Effect::Set(String::new())),
        ("Lower a flag", |_, _| Effect::Clear(String::new())),
        ("Do several things", |_, _| Effect::All(Vec::new())),
    ];
    let label = match effect {
        Effect::Message(_) => "Show a message",
        Effect::Teleport(_) => "Teleport the player",
        Effect::Give(..) => "Give an item",
        Effect::Restart => "Restart at the start",
        Effect::Hurt(_) => "Hurt the player",
        Effect::Heal(_) => "Heal the player",
        Effect::Say(_) => "Show a dialog",
        Effect::Sound(_) => "Play a sound",
        Effect::Set(_) => "Raise a flag",
        Effect::Clear(_) => "Lower a flag",
        Effect::All(_) => "Do several things",
    };
    egui::ComboBox::from_id_salt((salt, "kind"))
        .selected_text(label)
        .show_ui(ui, |ui| {
            for (name, make) in &choices {
                if ui.selectable_label(*name == label, *name).clicked() && *name != label {
                    let mut new = make(known, names);
                    // Several things start with what was there.
                    if let Effect::All(list) = &mut new {
                        list.push(effect.clone());
                    }
                    *effect = new;
                }
            }
        });
    match effect {
        Effect::Message(text) => {
            ui.text_edit_singleline(text);
        }
        Effect::Teleport(to) => {
            egui::ComboBox::from_id_salt((salt, "teleport"))
                .selected_text(if to.is_empty() {
                    "(choose a node)"
                } else {
                    to.as_str()
                })
                .show_ui(ui, |ui| {
                    for name in names {
                        if ui.selectable_label(name == to, name).clicked() {
                            *to = name.clone();
                        }
                    }
                });
        }
        Effect::Give(item, count) => {
            item_picker(ui, &format!("{salt}_give"), item, known);
            kit::prop(ui, "Count", |ui| {
                ui.add(egui::DragValue::new(count).range(1..=99));
            });
        }
        Effect::Restart => {}
        Effect::Say(clockwork_core::asset::Slot::Inline(dialog)) => {
            let mut speaker = dialog.speaker.clone().unwrap_or_default();
            kit::prop(ui, "Speaker", |ui| {
                ui.text_edit_singleline(&mut speaker);
            });
            dialog.speaker = (!speaker.is_empty()).then_some(speaker);
            let mut pages = dialog.pages.join("\n\n");
            ui.label(kit::hint("Pages, an empty line between them"));
            ui.add(egui::TextEdit::multiline(&mut pages).desired_rows(4));
            dialog.pages = pages
                .replace("\r\n", "\n")
                .split("\n\n")
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
        }
        Effect::Say(clockwork_core::asset::Slot::Ref(at)) => {
            ui.label(format!("The dialog {at}"));
        }
        Effect::Sound(name) => {
            let sounds: Vec<String> =
                clockwork_recipe::assets::AssetIndex::build(&state.workshop.plugins)
                    .of_kind("Sound")
                    .map(|a| a.at.to_string())
                    .collect();
            egui::ComboBox::from_id_salt((salt, "sound"))
                .selected_text(if name.is_empty() {
                    "(choose a sound)"
                } else {
                    name.as_str()
                })
                .show_ui(ui, |ui| {
                    for sound in sounds {
                        if ui.selectable_label(*name == sound, &sound).clicked() {
                            *name = sound;
                        }
                    }
                });
        }
        Effect::Hurt(amount) | Effect::Heal(amount) => {
            ui.add(
                egui::DragValue::new(amount)
                    .range(0.25..=100.0)
                    .speed(0.25)
                    .suffix(" health"),
            );
        }
        Effect::Set(flag) | Effect::Clear(flag) => {
            ui.add(egui::TextEdit::singleline(flag).hint_text("flag name"));
        }
        Effect::All(list) => {
            let mut remove = None;
            ui.indent((salt, "all"), |ui| {
                for (k, inner) in list.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if kit::icon_button(ui, "✕", "Remove").clicked() {
                            remove = Some(k);
                        }
                        ui.vertical(|ui| {
                            effect_editor(ui, state, &format!("{salt}_{k}"), inner, known, names);
                        });
                    });
                }
                if ui.button("+ Effect").clicked() {
                    list.push(Effect::Message("Hello!".into()));
                }
            });
            if let Some(k) = remove {
                list.remove(k);
            }
        }
    }
}

fn plugin_inspector(ui: &mut egui::Ui, state: &mut EditorState, index: usize, browser: &Browser) {
    let Some(plugin) = state.workshop.plugins.get(index).cloned() else {
        return;
    };
    let rom = state.rom_for(&plugin).cloned();
    ui.horizontal(|ui| {
        ui.label(RichText::new("◆").size(16.0).color(theme::BRASS));
        ui.label(RichText::new(&plugin.manifest.plugin.name).font(theme::strong(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if kit::icon_button(ui, "✎", "Open in the Plugins screen").clicked() {
                state.screen = crate::state::Screen::Plugins;
                state.plugin_editor.selected = Some(plugin.manifest.plugin.name.clone());
            }
        });
    });
    ui.add_space(4.0);
    egui::Grid::new("plugin_info")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            info_row(ui, "Version", &plugin.manifest.plugin.version);
            info_row(
                ui,
                "Game",
                &rom.as_ref()
                    .map(|e| state.title(e))
                    .unwrap_or_else(|| format!("{} (not in Library)", plugin.manifest.source.game)),
            );
            info_row(ui, "Steps", &plugin.recipe.steps.len().to_string());
            // A long path would widen the panel: cut short, whole on hover.
            let folder = plugin.dir.display().to_string();
            ui.label(kit::dim("Folder"));
            ui.add(egui::Label::new(&folder).truncate())
                .on_hover_text(&folder);
            ui.end_row();
        });
    kit::section(ui, "Outputs");

    let name = plugin.manifest.plugin.name.clone();
    for output in &plugin.recipe.outputs {
        let Some((id, kind)) = output.item() else {
            continue;
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, theme::BG_DEEP);
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            if let Some(texture) = browser.preview(&name, id) {
                let size = texture.size_vec2();
                let scale = (64.0 / size.x).min(64.0 / size.y);
                let shown = egui::Rect::from_center_size(rect.center(), size * scale);
                ui.painter().image(
                    texture.id(),
                    shown,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ui.vertical(|ui| {
                ui.label(id);
                if let Some(kind) = kind {
                    ui.small(RichText::new(kind.singular()).color(theme::PARCHMENT_DIM));
                }
                if ui
                    .add_enabled(rom.is_some(), egui::Button::new("+ Place in scene"))
                    .on_disabled_hover_text("Import this plugin's game into the Library first")
                    .clicked()
                {
                    state.workshop.place(&name, id, ROOT, false);
                }
            });
        });
        ui.add_space(4.0);
    }
}

/// Legend for the surface overlay and what the cell under the cursor is.
fn surfaces_section(ui: &mut egui::Ui, map: &SurfaceMap, hovered: Option<(u32, u32)>) {
    ui.small(
        RichText::new(format!(
            "{} × {} cells, {} kinds",
            map.width,
            map.height,
            map.kinds.len()
        ))
        .color(theme::PARCHMENT_DIM),
    );
    ui.horizontal_wrapped(|ui| {
        for (label, [r, g, b, _]) in surfaces::LEGEND {
            let (swatch, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(swatch, 2.0, Color32::from_rgb(r, g, b));
            ui.small(label);
            ui.add_space(4.0);
        }
    });

    ui.add_space(6.0);
    let Some((x, y)) = hovered else {
        ui.small(RichText::new("Hover the map to inspect a cell").color(theme::PARCHMENT_DIM));
        return;
    };
    let Some(kind) = map.kind_at(x, y) else {
        return;
    };
    ui.label(RichText::new(format!("Cell {x}, {y}")).strong());
    if kind.properties.is_empty() {
        ui.small(RichText::new("No surface properties").color(theme::PARCHMENT_DIM));
    }
    for p in &kind.properties {
        let text = surfaces::describe(p);
        match p {
            SurfaceProperty::Extension(_) => {
                let fallback: Vec<String> = map
                    .resolve(std::slice::from_ref(p))
                    .iter()
                    .map(surfaces::describe)
                    .collect();
                ui.label(RichText::new(text).color(theme::BRASS_BRIGHT))
                    .on_hover_text("Game-specific surface");
                ui.small(
                    RichText::new(format!("    elsewhere: {}", fallback.join(", ")))
                        .color(theme::PARCHMENT_DIM),
                );
            }
            _ => {
                ui.label(text);
            }
        }
    }
    let codes: Vec<String> = kind
        .codes
        .iter()
        .map(|(grid, code)| format!("{grid} {code:#04x}"))
        .collect();
    ui.small(RichText::new(codes.join(" · ")).color(theme::PARCHMENT_DIM));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_keys_cannot_delete_the_scene_or_stop_play() {
        let mut state = EditorState {
            library: Default::default(),
            missing_roms: Vec::new(),
            known_roms: Vec::new(),
            selected: None,
            search: String::new(),
            agent_prompt: String::new(),
            screen: crate::state::Screen::Workshop,
            log: Vec::new(),
            fetch_box_art: false,
            names: Default::default(),
            decomps: Default::default(),
            decomp_input: String::new(),
            boxart_retry: Vec::new(),
            boxart_busy: false,
            workshop: crate::state::Workshop {
                scene: crate::scene::Scene::default(),
                ..Default::default()
            },
            agents: Vec::new(),
            claude_connected: false,
            player: None,
            plugin_editor: Default::default(),
            quit_asked: false,
            palette_request: None,
        };
        let node = state
            .workshop
            .scene
            .add(ROOT, "Player".into(), None, bevy::math::Vec3::ZERO);
        state.workshop.selected_node = Some(node);
        state.workshop.playing = Some(node);
        let mut settings = crate::settings::Settings::default();
        settings.controls.set_keys("Jump", vec!["Delete".into()]);
        settings.controls.set_keys("Back", vec!["Escape".into()]);

        let press = |state: &mut EditorState, settings: &crate::settings::Settings, key| {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let mut output = egui::Context::default().run_ui(input, |ui| {
                workshop_shortcuts(ui.ctx(), state, settings);
            });
            // This input-only test has no renderer to apply egui's font texture.
            output.textures_delta.clear();
        };

        press(&mut state, &settings, egui::Key::Delete);
        assert!(state.workshop.scene.get(node).is_some());
        press(&mut state, &settings, egui::Key::Escape);
        assert_eq!(state.workshop.play_request, None);

        settings.controls.reset("Back");
        press(&mut state, &settings, egui::Key::Escape);
        assert_eq!(state.workshop.play_request, Some(false));
        state.workshop.playing = None;
        press(&mut state, &settings, egui::Key::Delete);
        assert!(state.workshop.scene.get(node).is_none());
    }
}

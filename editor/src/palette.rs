//! The command palette (Ctrl+K): every action, screen, plugin, game and scene node by typing a
//! few letters of its name, like VS Code's and Blender's search. Also the keyboard shortcut sheet
//! (F1) and the notifications that pop up for warnings and errors.

use bevy_egui::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, vec2};

use crate::{
    kit,
    manipulate::Tool,
    scene::NodeId,
    settings,
    state::{EditorState, LogLevel, Screen},
    theme,
};

/// Something the palette, a menu or a shortcut asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AppCommand {
    Go(Screen),
    ImportRoms,
    ImportFolder,
    ExportGame,
    Undo,
    Redo,
    RescanPlugins,
    TogglePlay,
    ToggleSurfaces,
    ToggleSnap,
    Tool(Tool),
    AddEntity(&'static str),
    FrameSelection,
    ShowLog,
    Settings(settings::Category),
    Shortcuts,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    OpenPlugin(String),
    SelectNode(NodeId),
    ShowGame(usize),
    Quit,
}

/// One entry of the palette.
struct Entry {
    group: &'static str,
    title: String,
    keys: &'static str,
    command: AppCommand,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    highlighted: usize,
    /// Whether the shortcut sheet is open.
    pub shortcuts: bool,
}

impl Palette {
    /// Opens the palette with `query` typed in.
    pub fn open_with(&mut self, query: String) {
        self.open = true;
        self.query = query;
        self.highlighted = 0;
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.highlighted = 0;
    }
}

/// Every command there is right now, in the order they are listed before anything is typed.
fn entries(state: &EditorState) -> Vec<Entry> {
    let e = |group, title: &str, keys, command| Entry {
        group,
        title: title.to_string(),
        keys,
        command,
    };
    let mut out = vec![
        e(
            "Go to",
            "Library",
            "Ctrl+1",
            AppCommand::Go(Screen::Library),
        ),
        e(
            "Go to",
            "Workshop",
            "Ctrl+2",
            AppCommand::Go(Screen::Workshop),
        ),
        e(
            "Go to",
            "Plugins",
            "Ctrl+3",
            AppCommand::Go(Screen::Plugins),
        ),
        e(
            "Go to",
            "Settings",
            "Ctrl+4",
            AppCommand::Go(Screen::Settings),
        ),
        e("File", "Import games…", "Ctrl+O", AppCommand::ImportRoms),
        e(
            "File",
            "Import a folder of games…",
            "",
            AppCommand::ImportFolder,
        ),
        e("File", "Export game…", "Ctrl+E", AppCommand::ExportGame),
        e("File", "Rescan plugins", "", AppCommand::RescanPlugins),
        e("Edit", "Undo", "Ctrl+Z", AppCommand::Undo),
        e("Edit", "Redo", "Ctrl+Y", AppCommand::Redo),
        e("Workshop", "Play / Stop", "F5", AppCommand::TogglePlay),
        e("Workshop", "Move tool", "W", AppCommand::Tool(Tool::Move)),
        e(
            "Workshop",
            "Rotate tool",
            "E",
            AppCommand::Tool(Tool::Rotate),
        ),
        e("Workshop", "Scale tool", "R", AppCommand::Tool(Tool::Scale)),
        e("Workshop", "Toggle snapping", "G", AppCommand::ToggleSnap),
        e(
            "Workshop",
            "Frame selection",
            "F",
            AppCommand::FrameSelection,
        ),
        e(
            "Workshop",
            "Toggle surface overlay",
            "",
            AppCommand::ToggleSurfaces,
        ),
        e("Workshop", "Show the log", "", AppCommand::ShowLog),
    ];
    for kind in crate::entities::Behavior::KINDS {
        out.push(Entry {
            group: "Add entity",
            title: format!("Add {kind}"),
            keys: "",
            command: AppCommand::AddEntity(kind),
        });
    }
    for c in settings::Category::ALL {
        out.push(Entry {
            group: "Settings",
            title: format!("Settings: {}", c.label()),
            keys: "",
            command: AppCommand::Settings(c),
        });
    }
    out.extend([
        e("View", "Zoom in", "Ctrl+=", AppCommand::ZoomIn),
        e("View", "Zoom out", "Ctrl+-", AppCommand::ZoomOut),
        e("View", "Reset zoom", "Ctrl+0", AppCommand::ZoomReset),
        e("Help", "Keyboard shortcuts", "F1", AppCommand::Shortcuts),
        e("File", "Quit", "", AppCommand::Quit),
    ]);
    for plugin in &state.workshop.plugins {
        out.push(Entry {
            group: "Plugin",
            title: plugin.manifest.plugin.name.clone(),
            keys: "",
            command: AppCommand::OpenPlugin(plugin.manifest.plugin.name.clone()),
        });
    }
    for (i, entry) in state.library.entries().iter().enumerate() {
        out.push(Entry {
            group: "Game",
            title: state.title(entry),
            keys: "",
            command: AppCommand::ShowGame(i),
        });
    }
    for node in &state.workshop.scene.nodes {
        if node.id != crate::scene::ROOT {
            out.push(Entry {
                group: "Scene node",
                title: node.name.clone(),
                keys: "",
                command: AppCommand::SelectNode(node.id),
            });
        }
    }
    out
}

/// How well `query` matches `text`: its letters must appear in order. Matches at the start of
/// words and runs of consecutive letters score higher. `None` if it doesn't match.
fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut total = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (at..text.len()).find(|&i| text[i] == q)?;
        let word_start = found == 0 || !text[found - 1].is_alphanumeric();
        total += if word_start { 10 } else { 1 };
        if previous == Some(found.wrapping_sub(1)) {
            total += 5;
        }
        total -= (found - at).min(10) as i32 / 3;
        previous = Some(found);
        at = found + 1;
    }
    Some(total)
}

/// Draws the palette when open. Returns the command chosen this frame.
pub fn show(ctx: &egui::Context, palette: &mut Palette, state: &EditorState) -> Option<AppCommand> {
    if !palette.open {
        return None;
    }
    let all = entries(state);
    let mut matches: Vec<(i32, &Entry)> = all
        .iter()
        .filter_map(|e| {
            let haystack = format!("{} {}", e.title, e.group);
            score(&palette.query, &haystack).map(|s| (s, e))
        })
        .collect();
    if !palette.query.is_empty() {
        matches.sort_by_key(|m| std::cmp::Reverse(m.0));
    }
    matches.truncate(60);
    palette.highlighted = palette.highlighted.min(matches.len().saturating_sub(1));

    let (up, down, enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if down && !matches.is_empty() {
        palette.highlighted = (palette.highlighted + 1) % matches.len();
    }
    if up && !matches.is_empty() {
        palette.highlighted = (palette.highlighted + matches.len() - 1) % matches.len();
    }

    let mut chosen = None;
    let screen = ctx.content_rect();
    // Dim what is behind; a click outside the palette closes it.
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("palette_backdrop"),
    ))
    .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
    let area = egui::Area::new(egui::Id::new("palette"))
        .order(egui::Order::Tooltip)
        .fade_in(false)
        .fixed_pos(screen.center_top() + vec2(0.0, 72.0))
        .pivot(egui::Align2::CENTER_TOP)
        .show(ctx, |ui| {
            Frame::new()
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                .corner_radius(CornerRadius::same(10))
                .shadow(ctx.global_style().visuals.window_shadow)
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(560.0_f32.min(screen.width() - 32.0));
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut palette.query)
                            .hint_text(kit::hint("Type a command, plugin, game or node…"))
                            .font(egui::FontId::proportional(16.0))
                            .frame(Frame::NONE)
                            .margin(Margin::symmetric(6, 6))
                            .desired_width(f32::INFINITY),
                    );
                    if field.changed() {
                        palette.highlighted = 0;
                    }
                    field.request_focus();
                    ui.separator();
                    if matches.is_empty() {
                        ui.add_space(6.0);
                        ui.label(kit::dim("  Nothing matches"));
                        ui.add_space(6.0);
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (i, (_, entry)) in matches.iter().enumerate() {
                                let on = i == palette.highlighted;
                                let (rect, response) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 28.0),
                                    egui::Sense::click(),
                                );
                                if on {
                                    ui.painter().rect_filled(rect, 5.0, theme::SELECTED);
                                    if up || down {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 5.0, theme::RAISED);
                                }
                                let painter = ui.painter();
                                painter.text(
                                    rect.left_center() + vec2(10.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    &entry.title,
                                    egui::FontId::proportional(13.5),
                                    if on {
                                        theme::BRASS_BRIGHT
                                    } else {
                                        theme::PARCHMENT
                                    },
                                );
                                let right = if entry.keys.is_empty() {
                                    entry.group.to_string()
                                } else {
                                    entry.keys.to_string()
                                };
                                painter.text(
                                    rect.right_center() - vec2(10.0, 0.0),
                                    egui::Align2::RIGHT_CENTER,
                                    right,
                                    if entry.keys.is_empty() {
                                        egui::FontId::proportional(11.5)
                                    } else {
                                        egui::FontId::monospace(11.5)
                                    },
                                    theme::FAINT,
                                );
                                if response.clicked() {
                                    chosen = Some(entry.command.clone());
                                }
                            }
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        kit::kbd(ui, "↑+↓");
                        ui.label(kit::hint("move"));
                        ui.add_space(8.0);
                        kit::kbd(ui, "Enter");
                        ui.label(kit::hint("run"));
                        ui.add_space(8.0);
                        kit::kbd(ui, "Esc");
                        ui.label(kit::hint("close"));
                    });
                });
        });
    let clicked_outside = ctx.input(|i| i.pointer.any_click())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !area.response.rect.contains(p));
    if enter && let Some((_, entry)) = matches.get(palette.highlighted) {
        chosen = Some(entry.command.clone());
    }
    if chosen.is_some() || escape || clicked_outside {
        palette.open = false;
        palette.query.clear();
    }
    chosen
}

/// Every keyboard shortcut, by where it works.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "General",
        &[
            ("Ctrl+K", "Command palette"),
            ("F1", "Keyboard shortcuts"),
            ("Ctrl+1 … 4", "Library, Workshop, Plugins, Settings"),
            ("Ctrl+O", "Import games"),
            ("Ctrl+E", "Export the scene as a game"),
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
            ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom the interface"),
        ],
    ),
    (
        "Workshop",
        &[
            ("W / E / R", "Move, rotate, scale"),
            ("G", "Snapping"),
            ("F", "Frame the selection"),
            ("Ctrl+click", "Add to the selection"),
            ("Ctrl+D", "Duplicate"),
            ("Ctrl+C / Ctrl+V", "Copy / paste"),
            ("Delete", "Delete the selection"),
            ("F5", "Play / stop"),
            ("Drag / right drag / wheel", "Orbit / pan / zoom"),
        ],
    ),
    (
        "Playing",
        &[
            ("WASD / arrows", "Move (Shift walks)"),
            ("Space", "Jump"),
            ("J / K", "Item B / item A"),
            ("L", "Dodge"),
            ("I", "Inventory"),
            ("H", "Hit boxes"),
            ("P / .", "Pause / one step"),
            ("[ / ]", "Slower / faster"),
            ("Esc", "Stop"),
        ],
    ),
    (
        "Graphs",
        &[
            ("Ctrl+S", "Save"),
            ("Ctrl+F", "Find a node"),
            ("Right-click canvas", "Add a node"),
            ("Delete", "Remove the selected nodes"),
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
        ],
    ),
];

/// The shortcut list in `columns` columns.
pub fn shortcut_table(ui: &mut egui::Ui, columns: usize) {
    let per = SHORTCUTS.len().div_ceil(columns);
    ui.horizontal_top(|ui| {
        for column in SHORTCUTS.chunks(per) {
            ui.vertical(|ui| {
                ui.set_width(330.0);
                for (group, keys) in column {
                    kit::section(ui, group);
                    egui::Grid::new(("shortcuts", *group))
                        .num_columns(2)
                        .spacing([16.0, 5.0])
                        .show(ui, |ui| {
                            for (key, what) in *keys {
                                ui.label(
                                    RichText::new(*key).monospace().color(theme::BRASS_BRIGHT),
                                );
                                ui.label(*what);
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                }
            });
            ui.add_space(12.0);
        }
    });
}

/// The keyboard shortcut sheet.
pub fn shortcuts_window(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    egui::Window::new("Keyboard shortcuts")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            shortcut_table(ui, 2);
        });
}

/// A warning or error that pops up in the corner for a few seconds.
pub struct Toast {
    level: LogLevel,
    text: String,
    until: f64,
}

#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
    /// How many log lines have been looked at.
    seen: usize,
}

/// Turns new warnings and errors in the log into toasts and draws them above the status bar.
/// Returns true when one was clicked (to show the log).
pub fn toasts(ctx: &egui::Context, toasts: &mut Toasts, state: &EditorState) -> bool {
    let now = ctx.input(|i| i.time);
    if toasts.seen > state.log.len() {
        toasts.seen = 0;
    }
    for line in &state.log[toasts.seen..] {
        if line.level != LogLevel::Info {
            toasts.list.push(Toast {
                level: line.level,
                text: line.text.clone(),
                until: now
                    + if line.level == LogLevel::Error {
                        8.0
                    } else {
                        5.0
                    },
            });
        }
    }
    toasts.seen = state.log.len();
    toasts.list.retain(|t| t.until > now);
    if toasts.list.len() > 4 {
        let extra = toasts.list.len() - 4;
        toasts.list.drain(..extra);
    }
    if toasts.list.is_empty() {
        return false;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(250));

    let mut clicked = false;
    let mut dismiss = None;
    let corner = ctx.content_rect().right_bottom() - vec2(16.0, 36.0);
    egui::Area::new(egui::Id::new("toasts"))
        .order(egui::Order::Foreground)
        .fixed_pos(corner)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (i, toast) in toasts.list.iter().enumerate() {
                let (color, icon) = match toast.level {
                    LogLevel::Error => (theme::ERROR, "⛔"),
                    LogLevel::Warn => (theme::WARN, "⚠"),
                    LogLevel::Info => (theme::INFO, "ℹ"),
                };
                let response = Frame::new()
                    .fill(theme::HEADER)
                    .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                    .corner_radius(CornerRadius::same(6))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(380.0);
                        ui.horizontal(|ui| {
                            let (bar, _) =
                                ui.allocate_exact_size(vec2(3.0, 18.0), egui::Sense::hover());
                            ui.painter().rect_filled(bar, 1.5, color);
                            ui.label(RichText::new(icon).color(color));
                            ui.add(egui::Label::new(&toast.text).wrap());
                            if ui
                                .add(egui::Button::new(kit::hint("✕")).frame(false))
                                .on_hover_text("Dismiss")
                                .clicked()
                            {
                                dismiss = Some(i);
                            }
                        });
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Show the log");
                clicked |= response.clicked();
            }
        });
    if let Some(i) = dismiss {
        toasts.list.remove(i);
        clicked = false;
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn letters_in_order_match() {
        assert!(score("wks", "Workshop").is_some());
        assert!(score("xyz", "Workshop").is_none());
        assert!(score("", "anything").is_some());
    }

    #[test]
    fn word_starts_rank_higher() {
        let start = score("ir", "Import ROMs").unwrap();
        let middle = score("ir", "Stir").unwrap();
        assert!(start > middle);
    }
}

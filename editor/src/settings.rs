//! Editor settings and the Settings screen (modelled on Blender's Preferences: categories on the
//! left, changes apply immediately and are saved as you go, no OK/Apply).
//!
//! Saved to `<data>/settings.ron`. Library options (online lookups) stay in the Library's own
//! file because the agent server reads them too.

use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use bevy_egui::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use clockwork_core::input::{ControlSettings, InputMap};
use clockwork_library::content;

use crate::{
    kit,
    state::{EditorState, LogLevel},
    store, theme,
    ui::info_row,
};

#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Personal gameplay bindings and controller preferences, applied over scene defaults.
    pub controls: ControlSettings,
    /// Size of the whole interface (1.0 = 100%).
    pub ui_scale: f32,
    /// Whether surface overlays are on when the Editor starts.
    pub show_surfaces: bool,
    /// Save the scene automatically after changes.
    pub autosave_scene: bool,
    pub orbit_speed: f32,
    pub pan_speed: f32,
    pub zoom_speed: f32,
    pub invert_orbit_y: bool,
    /// Thumbnail size in the content browser, in pixels.
    pub thumb_size: f32,
    /// Pick up new and changed plugins without pressing Rescan.
    pub watch_plugins: bool,
    /// Extract opens a Claude Code Desktop session instead of copying a prompt.
    pub claude_connected: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            controls: ControlSettings::default(),
            ui_scale: 1.0,
            show_surfaces: true,
            autosave_scene: true,
            orbit_speed: 1.0,
            pan_speed: 1.0,
            zoom_speed: 1.0,
            invert_orbit_y: false,
            thumb_size: 96.0,
            watch_plugins: true,
            claude_connected: false,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        store::data_dir().map(|d| d.join("settings.ron"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no data directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text =
            ron::ser::to_string_pretty(self, Default::default()).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

/// Settings categories, in sidebar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    Interface,
    Library,
    Workshop,
    PluginsAgents,
    Controls,
    Keyboard,
    About,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Interface,
        Category::Library,
        Category::Workshop,
        Category::PluginsAgents,
        Category::Controls,
        Category::Keyboard,
        Category::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Interface => "Interface",
            Category::Library => "Library",
            Category::Workshop => "Workshop",
            Category::PluginsAgents => "Plugins & Agents",
            Category::Controls => "Controls",
            Category::Keyboard => "Keyboard",
            Category::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Category::Interface => "◐",
            Category::Library => "▤",
            Category::Workshop => "✥",
            Category::PluginsAgents => "◆",
            Category::Controls => "✥",
            Category::Keyboard => "⌨",
            Category::About => "ⓘ",
        }
    }

    /// What the category holds, under its title.
    fn blurb(self) -> &'static str {
        match self {
            Category::Interface => "How big everything is.",
            Category::Library => "Online lookups and where Clockwork keeps its files.",
            Category::Workshop => "The scene, the viewport camera and the content browser.",
            Category::PluginsAgents => "Where plugins come from and the agents that make them.",
            Category::Controls => {
                "Your gameplay keys and controllers. Changes are saved automatically."
            }
            Category::Keyboard => "Every shortcut. Ctrl+K finds any command by name.",
            Category::About => "Versions and folders.",
        }
    }
}

/// Where external programs (Ghidra, decompilers, …) live; matches the agent server.
fn tools_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLOCKWORK_TOOLS_DIR") {
        return Some(dir.into());
    }
    store::data_dir().map(|d| d.join("tools"))
}

/// Opens a folder in the system's file manager, creating it first.
fn open_folder(state: &mut EditorState, path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        state.log(
            LogLevel::Warn,
            format!("Could not open {}: {e}", path.display()),
        );
    }
}

/// The Settings screen. `category` is the open sidebar entry.
pub fn settings_screen(
    root: &mut egui::Ui,
    state: &mut EditorState,
    settings: &mut Settings,
    category: &mut Category,
    controls: &mut crate::controls::ControlsUi,
    input_map: &InputMap,
) {
    let before = settings.clone();

    egui::Panel::left("settings_categories")
        .exact_size(220.0)
        .frame(kit::panel_frame())
        .show(root, |ui| {
            kit::panel_header(ui, "Settings", |_| {});
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for c in Category::ALL {
                let selected = *category == c;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Sense::click(),
                );
                let painter = ui.painter();
                if selected {
                    painter.rect_filled(rect, 0.0, theme::SELECTED);
                    painter.rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                        0.0,
                        theme::BRASS,
                    );
                } else if response.hovered() {
                    painter.rect_filled(rect, 0.0, theme::RAISED);
                }
                painter.text(
                    rect.left_center() + egui::vec2(20.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    c.icon(),
                    egui::FontId::proportional(13.0),
                    if selected {
                        theme::BRASS
                    } else {
                        theme::PARCHMENT_DIM
                    },
                );
                painter.text(
                    rect.left_center() + egui::vec2(36.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    c.label(),
                    egui::FontId::proportional(13.5),
                    if selected {
                        theme::BRASS_BRIGHT
                    } else {
                        theme::PARCHMENT
                    },
                );
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    *category = c;
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                    if ui
                        .button("Reset all to defaults")
                        .on_hover_text(
                            "Editor and gameplay settings; your Library, plugins and scene are kept",
                        )
                        .clicked()
                    {
                        *settings = Settings::default();
                        controls.cancel();
                        state.log(LogLevel::Info, "Settings reset to their defaults");
                    }
                });
            });
        });

    if *category != Category::Controls {
        controls.cancel();
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::BG_DEEP))
        .show(root, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(32, 24))
                        .show(ui, |ui| {
                            ui.set_max_width(720.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(category.icon())
                                        .size(20.0)
                                        .color(theme::BRASS),
                                );
                                ui.label(
                                    RichText::new(category.label())
                                        .font(theme::display(22.0))
                                        .color(theme::PARCHMENT),
                                );
                            });
                            ui.label(kit::dim(category.blurb()));
                            ui.add_space(10.0);
                            match category {
                                Category::Interface => interface(ui, settings),
                                Category::Library => library(ui, state),
                                Category::Workshop => workshop(ui, settings),
                                Category::PluginsAgents => plugins_agents(ui, state, settings),
                                Category::Controls => crate::controls::panel(
                                    ui,
                                    &mut settings.controls,
                                    controls,
                                    input_map,
                                ),
                                Category::Keyboard => crate::palette::shortcut_table(ui, 1),
                                Category::About => about(ui),
                            }
                        });
                });
        });

    if *settings != before
        && let Err(e) = settings.save()
    {
        state.log(LogLevel::Warn, format!("Could not save settings: {e}"));
    }
}

/// A setting row: label and help on the left, the control on the right, a hairline below.
pub(crate) fn row(ui: &mut egui::Ui, label: &str, help: &str, control: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let left = ui.cursor().left();
    let text_w = (width - 260.0).max(220.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.label(RichText::new(label).font(theme::strong(13.5)));
            if !help.is_empty() {
                ui.add(egui::Label::new(kit::dim(help).small()).wrap());
            }
        });
        ui.add_space((ui.available_width() - 250.0).max(0.0));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            control,
        );
    });
    ui.add_space(6.0);
    let y = ui.cursor().top();
    ui.painter().line_segment(
        [egui::pos2(left, y), egui::pos2(left + width, y)],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    ui.add_space(8.0);
}

pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    kit::section(ui, title);
    ui.add_space(4.0);
}

fn interface(ui: &mut egui::Ui, settings: &mut Settings) {
    row(
        ui,
        "UI scale",
        "Makes all text and controls bigger or smaller.",
        |ui| {
            ui.add(
                egui::Slider::new(&mut settings.ui_scale, 0.75..=2.0)
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        },
    );
}

fn library(ui: &mut egui::Ui, state: &mut EditorState) {
    section(ui, "Online lookups");
    let mut fetch = state.fetch_box_art;
    row(
        ui,
        "Look up games online",
        "Names from the No-Intro database and box art from libretro-thumbnails. Only the ROM's \
         hash and game code are sent; ROMs never leave your machine.",
        |ui| {
            if ui.checkbox(&mut fetch, "").changed() {
                state.fetch_box_art = fetch;
                state.save();
            }
        },
    );
    row(
        ui,
        "Retry missing box art",
        "Looks up box art again for games that have none.",
        |ui| {
            if ui.button("Retry").clicked() {
                let all: Vec<String> = state
                    .library
                    .entries()
                    .iter()
                    .map(|e| e.sha1.clone())
                    .collect();
                state.boxart_retry.extend(all);
                state.log(LogLevel::Info, "Looking up box art again");
            }
        },
    );

    section(ui, "Storage");
    let data = store::data_dir();
    row(
        ui,
        "Data folder",
        &data
            .as_ref()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        |ui| {
            if let Some(d) = &data
                && ui.button("Open").clicked()
            {
                open_folder(state, d);
            }
        },
    );
    row(
        ui,
        "Preview cache",
        "Content browser thumbnails. They are made again as needed.",
        |ui| {
            if let Some(d) = &data
                && ui.button("Clear").clicked()
            {
                let _ = std::fs::remove_dir_all(d.join("cache").join("previews"));
                state.log(
                    LogLevel::Info,
                    "Preview cache cleared; restart to regenerate",
                );
            }
        },
    );
}

fn workshop(ui: &mut egui::Ui, settings: &mut Settings) {
    section(ui, "Scene");
    row(
        ui,
        "Show surfaces",
        "Whether surface overlays are on when the Editor starts.",
        |ui| {
            ui.checkbox(&mut settings.show_surfaces, "");
        },
    );
    row(
        ui,
        "Save scene automatically",
        "Saves the scene about once a second after changes.",
        |ui| {
            ui.checkbox(&mut settings.autosave_scene, "");
        },
    );

    section(ui, "Camera");
    let speed = |ui: &mut egui::Ui, value: &mut f32| {
        ui.add(
            egui::Slider::new(value, 0.25..=3.0)
                .step_by(0.05)
                .custom_formatter(|v, _| format!("{v:.2}×")),
        );
    };
    row(ui, "Orbit speed", "Left drag in the viewport.", |ui| {
        speed(ui, &mut settings.orbit_speed)
    });
    row(ui, "Pan speed", "Right or middle drag.", |ui| {
        speed(ui, &mut settings.pan_speed)
    });
    row(ui, "Zoom speed", "Mouse wheel.", |ui| {
        speed(ui, &mut settings.zoom_speed)
    });
    row(ui, "Invert vertical orbit", "", |ui| {
        ui.checkbox(&mut settings.invert_orbit_y, "");
    });

    section(ui, "Content browser");
    row(ui, "Thumbnail size", "", |ui| {
        ui.add(
            egui::Slider::new(&mut settings.thumb_size, 64.0..=160.0)
                .step_by(8.0)
                .suffix(" px"),
        );
    });
}

fn plugins_agents(ui: &mut egui::Ui, state: &mut EditorState, settings: &mut Settings) {
    section(ui, "Plugins");
    row(
        ui,
        "Watch plugin folders",
        "New and changed plugins show up without pressing Rescan.",
        |ui| {
            ui.checkbox(&mut settings.watch_plugins, "");
        },
    );

    section(ui, "Content folders");
    ui.label(kit::dim(
        "Plugins and game knowledge live in content folders (each with plugins/ and knowledge/), \
         apart from Clockwork itself. New plugins are saved to the first.",
    ));
    ui.add_space(4.0);
    let mut user = content::user_folders();
    let mut changed = false;
    for folder in content::folders() {
        let added = user.contains(&folder);
        let label = if added {
            "Content folder"
        } else {
            "Data folder (always)"
        };
        row(ui, label, &folder.display().to_string(), |ui| {
            if added && ui.button("Remove").clicked() {
                user.retain(|f| *f != folder);
                changed = true;
            }
            if ui.button("Open").clicked() {
                open_folder(state, &folder.join("plugins"));
            }
        });
    }
    if ui.button("Add content folder…").clicked()
        && let Some(folder) = rfd::FileDialog::new().pick_folder()
        && !user.contains(&folder)
    {
        user.push(folder);
        changed = true;
    }
    if changed {
        match content::save_user_folders(&user) {
            Ok(()) => state.scan_plugins(),
            Err(e) => state.log(
                LogLevel::Error,
                format!("Could not save content folders: {e}"),
            ),
        }
    }

    section(ui, "Claude Code Desktop");
    let installed = crate::claude::installed();
    row(
        ui,
        "Connect Claude Code Desktop",
        if installed {
            "Extract in the Library opens a new Claude Code session in the Clockwork folder with your request filled in. Press Send there to start; Claude Desktop asks you to confirm the folder."
        } else {
            "Claude Desktop may not be installed (no handler for claude:// links found). You can still turn this on; install it from claude.ai/download if Extract does nothing."
        },
        |ui| {
            ui.checkbox(&mut settings.claude_connected, "");
        },
    );

    section(ui, "Agents");
    let running = &state.agents;
    if running.is_empty() {
        ui.label(kit::dim("No agent server running."));
    }
    for a in running.clone() {
        let client = a.client.clone().unwrap_or_else(|| "no client yet".into());
        ui.label(format!(
            "{client} · {} · {} tool calls",
            a.transport, a.tool_calls
        ));
    }
    ui.add_space(6.0);
    let commands = [
        (
            "Start for an MCP client (stdio)",
            "cargo run -q -p clockwork_agent --bin clockwork-mcp",
        ),
        (
            "Start as an HTTP server",
            "cargo run -q -p clockwork_agent --bin clockwork-mcp -- --http 127.0.0.1:3128",
        ),
    ];
    for (label, command) in commands {
        row(ui, label, command, |ui| {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(command.to_string());
                state.log(LogLevel::Info, "Command copied");
            }
        });
    }
    if let Some(tools) = tools_dir() {
        row(
            ui,
            "External tools folder",
            &format!(
                "{}\nOne folder per tool (e.g. tools/ghidra). Used before tools on PATH.",
                tools.display()
            ),
            |ui| {
                if ui.button("Open").clicked() {
                    open_folder(state, &tools);
                }
            },
        );
    }
}

fn about(ui: &mut egui::Ui) {
    egui::Grid::new("about")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            info_row(ui, "Clockwork", env!("CARGO_PKG_VERSION"));
            info_row(ui, "Core vocabulary", clockwork_core::CORE_VERSION);
            info_row(
                ui,
                "Data folder",
                &store::data_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
            );
        });
    ui.add_space(10.0);
    ui.label(kit::hint(
        "Clockwork never downloads games. Use your own legally obtained copies.",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_written_before_controls_keep_their_existing_values() {
        let settings: Settings = ron::from_str("(ui_scale: 1.5, invert_orbit_y: true)").unwrap();
        assert_eq!(settings.ui_scale, 1.5);
        assert!(settings.invert_orbit_y);
        assert_eq!(settings.controls, ControlSettings::default());
    }

    #[test]
    fn gameplay_preferences_survive_settings_roundtrip() {
        let mut settings = Settings::default();
        settings.controls.set_keys("Jump", vec!["KeyZ".into()]);
        settings.controls.set_pad("Jump", vec![]);
        settings.controls.controller_style = clockwork_core::input::ControllerStyle::PlayStation;
        settings.controls.stick_deadzone = 0.25;
        let text = ron::ser::to_string(&settings).unwrap();
        assert_eq!(ron::from_str::<Settings>(&text).unwrap(), settings);
    }
}

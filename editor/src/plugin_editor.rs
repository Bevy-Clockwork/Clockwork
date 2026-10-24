//! The plugin editor: makes and edits composition plugins (characters made from other plugins'
//! characters, with abilities learned from any plugin), and shows what every plugin provides.
//!
//! Compositions hold no game data, only references and choices, so they are edited freely, as a
//! node graph (see [`crate::compose_graph`]): wire a character into a character of this plugin,
//! wire abilities in through Learn nodes that choose what plays for each role, tune its movement,
//! save, and place it in the scene.
//!
//! Extraction plugins come from the user's ROMs (the agent writes their recipes). Their recipe is
//! a node graph too (see [`crate::recipe_graph`]): steps wired by the names they make and use,
//! each showing what it made from the user's ROM; their numbers, names and steps can be changed
//! without losing the recipe's comments. A second tab shows what the plugin provides.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, vec2};
use clockwork_core::{
    ability::{Role, RoleBinding},
    compose::{COMPOSITION_GAME, Composition, OutputRef},
};
use clockwork_recipe::Plugin;

use crate::{
    compose_graph::{self, CompositionGraph, Layout},
    kit,
    recipe_graph::{self, RecipeGraph, RecipeLayout},
    scene::ROOT,
    state::{EditorState, LogLevel, Screen, plugin_dirs},
    theme,
    workshop::Extractions,
};

/// What the plugin editor is looking at and editing.
#[derive(Default)]
pub struct PluginEditor {
    /// The plugin shown, by name.
    pub selected: Option<String>,
    /// Compositions being edited, by plugin name. Kept while other plugins are shown, so
    /// switching doesn't lose changes.
    graphs: HashMap<String, GraphDraft>,
    /// Extraction plugins' recipes being edited, by plugin name.
    recipes: HashMap<String, RecipeDraft>,
    /// What an extraction plugin shows: its recipe or what it provides.
    pub tab: Tab,
    /// The name typed for a new composition.
    new_name: String,
    /// The last thing that went wrong or right, shown under the editor.
    pub message: Option<String>,
    /// Plugins the editor needs extracted (for their characters, animations and abilities).
    pub wanted: Vec<String>,
}

impl PluginEditor {
    /// Plugins with changes that aren't saved.
    pub fn unsaved(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .graphs
            .iter()
            .filter(|(_, d)| d.dirty())
            .map(|(n, _)| n.clone())
            .chain(
                self.recipes
                    .iter()
                    .filter(|(_, d)| d.dirty())
                    .map(|(n, _)| n.clone()),
            )
            .collect();
        names.sort();
        names
    }
}

/// What is shown of an extraction plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Recipe,
    /// The recipe as text.
    Text,
    Provides,
}

/// A recipe being edited, and where its nodes sat when last saved.
struct RecipeDraft {
    graph: Result<RecipeGraph, String>,
    saved_layout: RecipeLayout,
    /// The plugins revision last seen: a recipe changed on disk (by the agent) is loaded again
    /// while there are no changes here, and reported while there are.
    revision: u64,
    disk_changed: bool,
}

impl RecipeDraft {
    fn load(state: &EditorState, plugin: &Plugin) -> Self {
        let rom = state
            .rom_for(plugin)
            .and_then(|entry| state.library.rom_bytes(entry).ok())
            .map(Arc::new);
        let mut graph = RecipeGraph::load(&plugin.dir, rom);
        if let Ok(graph) = &mut graph {
            graph.assets = Arc::new(clockwork_recipe::assets::OwnedAssets::of(
                &state.workshop.plugins,
            ));
            graph.plugin = plugin.name().to_string();
        }
        let saved_layout = graph.as_ref().map(RecipeGraph::layout).unwrap_or_default();
        Self {
            graph,
            saved_layout,
            revision: state.workshop.plugins_revision,
            disk_changed: false,
        }
    }

    fn dirty(&self) -> bool {
        self.graph
            .as_ref()
            .is_ok_and(|g| g.text_changed() || g.layout() != self.saved_layout)
    }
}

/// A composition being edited, and what it was when last saved.
struct GraphDraft {
    graph: CompositionGraph,
    saved: (Composition, Layout),
    /// The plugins revision last seen (see [`RecipeDraft::revision`]).
    revision: u64,
    disk_changed: bool,
}

impl GraphDraft {
    fn load(plugin: &Plugin, revision: u64) -> Self {
        let composition = plugin.compose.clone().unwrap_or_default();
        let graph = CompositionGraph::new(&composition, &compose_graph::load_layout(&plugin.dir));
        let saved = (graph.build().composition, graph.layout());
        Self {
            graph,
            saved,
            revision,
            disk_changed: false,
        }
    }

    fn dirty(&self) -> bool {
        self.graph.build().composition != self.saved.0 || self.graph.layout() != self.saved.1
    }
}

/// Offers to load a plugin file that changed on disk while it has changes here; true to load it.
fn disk_changed_banner(ui: &mut egui::Ui, file: &str, changed: &mut bool) -> bool {
    let mut load = false;
    if !*changed {
        return false;
    }
    egui::Frame::new()
        .fill(theme::RAISED)
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0xD0, 0x80, 0x60)))
        .corner_radius(4)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{file} changed on disk (the agent may have edited it). Saving here would \
                     replace those changes."
                ));
                if ui.button("Load it, drop my changes").clicked() {
                    load = true;
                }
                if ui.button("Keep mine").clicked() {
                    *changed = false;
                }
            });
        });
    load
}

/// A character that can be the base of a composed one.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterChoice {
    pub at: OutputRef,
    pub three_d: bool,
    /// Its animations, to bind roles to.
    pub animations: Vec<String>,
    /// Its bones (3D), to hold things at.
    pub bones: Vec<String>,
}

/// An ability that can be learned.
#[derive(Clone, Debug, PartialEq)]
pub struct AbilityChoice {
    pub at: OutputRef,
    pub name: String,
    pub description: String,
    pub roles: Vec<Role>,
}

/// Every playable character of the extracted plugins, except composed ones (a composition can't
/// build on itself).
pub fn characters(plugins: &[Plugin], extractions: &Extractions) -> Vec<CharacterChoice> {
    let mut out = Vec::new();
    for plugin in plugins.iter().filter(|p| p.compose.is_none()) {
        let Some(x) = extractions.get(plugin.name()) else {
            continue;
        };
        let at = |output: &str| OutputRef {
            plugin: plugin.name().to_string(),
            output: output.to_string(),
        };
        for s in x.sprites.iter().filter(|s| s.character.is_some()) {
            out.push(CharacterChoice {
                at: at(&s.id),
                three_d: false,
                animations: s.sprite.animations.iter().map(|a| a.name.clone()).collect(),
                bones: Vec::new(),
            });
        }
        for m in x.models.iter().filter(|m| m.character.is_some()) {
            out.push(CharacterChoice {
                at: at(&m.id),
                three_d: true,
                animations: m.model.animations.iter().map(|a| a.name.clone()).collect(),
                bones: m.model.bones.iter().map(|b| b.name.clone()).collect(),
            });
        }
    }
    out
}

/// Every ability of the extracted plugins.
pub fn abilities(plugins: &[Plugin], extractions: &Extractions) -> Vec<AbilityChoice> {
    let mut out = Vec::new();
    for plugin in plugins {
        let Some(x) = extractions.get(plugin.name()) else {
            continue;
        };
        for a in &x.abilities {
            out.push(AbilityChoice {
                at: OutputRef {
                    plugin: plugin.name().to_string(),
                    output: a.id.clone(),
                },
                name: a.name.clone(),
                description: a.description.clone(),
                roles: a.roles.clone(),
            });
        }
    }
    out
}

/// Whether a name can be a plugin folder and name: lowercase letters, digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Makes a new, empty composition plugin folder in `content` (a plugins folder).
pub fn create(content: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err("use lowercase letters, digits, - and _".into());
    }
    let dir = content.join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let manifest = format!(
        "[plugin]\nname = \"{name}\"\nversion = \"0.1.0\"\ncore = \"{}\"\n\n\
         # A composition plugin: no game of its own, only the plugins it uses (compose.ron).\n\
         [source]\ngame = \"{COMPOSITION_GAME}\"\nsha1 = []\n",
        clockwork_core::CORE_VERSION
    );
    clockwork_library::files::write_atomic(dir.join("clockwork.toml"), manifest)
        .map_err(|e| e.to_string())?;
    save(&dir, &Composition::default())?;
    Ok(dir)
}

/// Writes a composition to `compose.ron` in a plugin folder.
pub fn save(dir: &Path, composition: &Composition) -> Result<(), String> {
    let config = ron::ser::PrettyConfig::new()
        .depth_limit(6)
        .struct_names(false)
        .separate_tuple_members(false);
    let body = ron::ser::to_string_pretty(composition, config).map_err(|e| e.to_string())?;
    let text = format!(
        "// A composition plugin, written by the Clockwork plugin editor: characters made from\n\
         // other plugins' characters, with abilities learned from any plugin. No game data.\n{body}\n"
    );
    clockwork_library::files::write_atomic(dir.join("compose.ron"), text).map_err(|e| e.to_string())
}

/// What the role binding menu shows for a binding.
pub fn binding_label(binding: Option<&RoleBinding>, use_defaults: bool) -> String {
    match binding {
        Some(RoleBinding::Animation(name)) => name.clone(),
        Some(RoleBinding::Directional(d)) => format!("{} (per facing)", d.south.name),
        Some(RoleBinding::Default) => "The ability's own pose".into(),
        None if use_defaults => "The ability's own pose".into(),
        None => "Standing pose".into(),
    }
}

/// Asks for every plugin to be extracted that the editor needs, and runs those requests.
pub fn extract_system(mut state: ResMut<EditorState>, mut extractions: ResMut<Extractions>) {
    if state.screen != Screen::Plugins {
        return;
    }
    // Everything that can be extracted: the editor lists characters and abilities of all of them.
    if state.plugin_editor.wanted.is_empty() {
        let wanted: Vec<String> = state
            .workshop
            .plugins
            .iter()
            .filter(|p| state.ready(p))
            .map(|p| p.name().to_string())
            .filter(|name| {
                extractions.get(name).is_none()
                    && extractions.error(name).is_none()
                    && !extractions.is_pending(name)
            })
            .collect();
        state.plugin_editor.wanted = wanted;
    }
    // One plugin per frame keeps the editor responsive.
    if let Some(name) = state.plugin_editor.wanted.pop() {
        extractions.ensure(&mut state, &name);
    }
}

pub fn plugin_editor_screen(
    root: &mut egui::Ui,
    state: &mut EditorState,
    extractions: &Extractions,
) {
    egui::Panel::left("plugin_list")
        .resizable(true)
        .default_size(260.0)
        .size_range(220.0..=420.0)
        .frame(kit::panel_frame())
        .show(root, |ui| plugin_list(ui, state, extractions));

    let selected = state
        .plugin_editor
        .selected
        .as_deref()
        .and_then(|name| state.workshop.plugin_index(name))
        .map(|i| state.workshop.plugins[i].clone());
    let canvas = egui::Frame::new().fill(theme::BG_DEEP);
    if let Some(plugin) = selected.as_ref().filter(|p| p.compose.is_some()) {
        egui::CentralPanel::default()
            .frame(canvas)
            .show(root, |ui| composition_view(ui, state, extractions, plugin));
        return;
    }
    if let Some(plugin) = selected
        .as_ref()
        .filter(|_| matches!(state.plugin_editor.tab, Tab::Recipe | Tab::Text))
    {
        egui::CentralPanel::default()
            .frame(canvas)
            .show(root, |ui| recipe_view(ui, state, plugin));
        return;
    }
    egui::CentralPanel::default()
        .frame(canvas)
        .show(root, |ui| {
            if let Some(plugin) = &selected {
                view_header(ui, &mut state.plugin_editor.tab, plugin, |_| {});
            }
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(28, 20))
                        .show(ui, |ui| {
                            ui.set_max_width(760.0);
                            match &selected {
                                Some(plugin) => extraction_view(ui, state, extractions, plugin),
                                None => intro(ui, state),
                            }
                            if let Some(message) = &state.plugin_editor.message {
                                ui.add_space(10.0);
                                ui.label(kit::dim(message));
                            }
                        });
                });
        });
}

/// The bar above a plugin's view: its kind, name and game, the Recipe / Provides switch for
/// extraction plugins, and `actions` on the right.
fn view_header(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    plugin: &Plugin,
    actions: impl FnOnce(&mut egui::Ui),
) {
    let composition = plugin.compose.is_some();
    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if composition { "✦" } else { "◆" })
                        .size(16.0)
                        .color(theme::BRASS),
                );
                ui.label(
                    RichText::new(plugin.name())
                        .font(theme::display(19.0))
                        .color(theme::PARCHMENT),
                );
                if composition {
                    kit::pill(ui, "Composition", theme::BRASS);
                } else {
                    kit::pill(ui, &plugin.manifest.source.game, theme::PARCHMENT_DIM);
                    ui.add_space(10.0);
                    kit::segmented(
                        ui,
                        tab,
                        &[
                            (Tab::Recipe, "Recipe", "Edit the recipe as a node graph"),
                            (
                                Tab::Text,
                                "Text",
                                "Edit recipe.ron as text, comments and all",
                            ),
                            (Tab::Provides, "Provides", "What the plugin gives you"),
                        ],
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
            });
        });
    let rect = ui.min_rect();
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::BORDER),
    );
}

/// Undo, redo, tidy up, revert and save: the right side of a graph's header.
/// Returns (undo, redo, tidy, revert, save) clicks.
fn graph_actions(
    ui: &mut egui::Ui,
    dirty: bool,
    can_save: bool,
    can_undo: bool,
    can_redo: bool,
    save_tip: &str,
) -> [bool; 5] {
    let save = kit::primary_button(ui, dirty && can_save, "Save")
        .on_hover_text(kit::tip(save_tip, "Ctrl+S"))
        .clicked();
    let revert = ui
        .add_enabled(dirty, egui::Button::new("Revert"))
        .on_hover_text("Drop the changes that aren't saved")
        .clicked();
    ui.add_space(6.0);
    let tidy = kit::icon_button(ui, "⊞", "Tidy up: line the nodes up in columns").clicked();
    let redo = ui
        .add_enabled_ui(can_redo, |ui| kit::icon_button(ui, "↷", "Redo  ·  Ctrl+Y"))
        .inner
        .clicked();
    let undo = ui
        .add_enabled_ui(can_undo, |ui| kit::icon_button(ui, "↶", "Undo  ·  Ctrl+Z"))
        .inner
        .clicked();
    if dirty {
        ui.add_space(4.0);
        ui.label(
            RichText::new("● Unsaved")
                .small()
                .color(theme::BRASS_BRIGHT),
        );
    }
    [undo, redo, tidy, revert, save]
}

/// Keys and what they do, for the right side of a graph's toolbar (laid out right to left).
fn key_hints(ui: &mut egui::Ui, hints: &[(&str, &str)]) {
    for (keys, what) in hints.iter().rev() {
        ui.label(kit::hint(*what));
        // Right to left, so the caps go in last first.
        let caps: Vec<&str> = keys.split('+').rev().collect();
        kit::kbd(ui, &caps.join("+"));
        ui.add_space(8.0);
    }
}

/// Whether Ctrl+S was pressed (consumed, so nothing else sees it).
fn save_key(ui: &egui::Ui) -> bool {
    ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
}

fn intro(ui: &mut egui::Ui, state: &EditorState) {
    ui.label(
        RichText::new("Plugins")
            .font(theme::display(24.0))
            .color(theme::PARCHMENT),
    );
    ui.add_space(4.0);
    ui.label(kit::dim(
        "Plugins from your games are made by the agent from your own ROMs. Combine what they \
         provide in a composition: pick a character, teach it abilities from any game, choose \
         what plays for each of their poses and tune how it moves.",
    ));
    ui.add_space(16.0);
    let compositions = state
        .workshop
        .plugins
        .iter()
        .filter(|p| p.compose.is_some())
        .count();
    let extracted = state.workshop.plugins.len() - compositions;
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        for (ui, icon, title, count, body) in [
            (
                left,
                "◆",
                "From your games",
                extracted,
                "Recipes that turn your ROMs into characters, areas, sprites and items. Open one \
                 to see and tune its recipe as a graph.",
            ),
            (
                right,
                "✦",
                "Compositions",
                compositions,
                "Characters built from other plugins: a body from one game, abilities from \
                 another. Make one with + in the list.",
            ),
        ] {
            egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_min_height(120.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon).size(18.0).color(theme::BRASS));
                        ui.label(RichText::new(title).font(theme::strong(15.0)));
                        kit::pill(ui, &count.to_string(), theme::PARCHMENT_DIM);
                    });
                    ui.add_space(4.0);
                    ui.label(kit::dim(body));
                });
        }
    });
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.label(kit::hint("Tip:"));
        kit::kbd(ui, "Ctrl+K");
        ui.label(kit::hint("jumps to any plugin by name."));
    });
}

fn plugin_list(ui: &mut egui::Ui, state: &mut EditorState, extractions: &Extractions) {
    let creating_id = egui::Id::new("creating_composition");
    let mut creating: bool = ui.data(|d| d.get_temp(creating_id)).unwrap_or(false);
    kit::panel_header(ui, "Plugins", |ui| {
        if kit::icon_toggle(ui, creating, "+", "New composition").clicked() {
            creating = !creating;
        }
    });
    let filter_id = egui::Id::new("plugin_filter");
    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            kit::search_field(
                ui,
                &mut filter,
                "Filter plugins…",
                ui.available_width() - 14.0,
            );
            if creating {
                ui.add_space(4.0);
                create_composition(ui, state, &mut creating);
            }
        });
    ui.data_mut(|d| {
        d.insert_temp(filter_id, filter.clone());
        d.insert_temp(creating_id, creating);
    });
    let filter = filter.to_lowercase();

    let row = |ui: &mut egui::Ui, state: &mut EditorState, plugin: &Plugin| {
        let name = plugin.name().to_string();
        let selected = state.plugin_editor.selected.as_deref() == Some(name.as_str());
        let unsaved = state
            .plugin_editor
            .graphs
            .get(&name)
            .is_some_and(GraphDraft::dirty)
            || state
                .plugin_editor
                .recipes
                .get(&name)
                .is_some_and(RecipeDraft::dirty);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 0.0, theme::SELECTED);
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                0.0,
                theme::BRASS,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, 0.0, theme::RAISED);
        }
        let failed = extractions.error(&name).is_some();
        painter.text(
            rect.left_center() + vec2(16.0, 0.0),
            egui::Align2::CENTER_CENTER,
            if plugin.compose.is_some() {
                "✦"
            } else {
                "◆"
            },
            egui::FontId::proportional(11.0),
            if failed {
                theme::ERROR
            } else if selected {
                theme::BRASS
            } else {
                theme::PARCHMENT_DIM
            },
        );
        kit::text_truncated(
            painter,
            rect.left_center() + vec2(28.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(13.0),
            if selected {
                theme::BRASS_BRIGHT
            } else {
                theme::PARCHMENT
            },
            rect.width() - 52.0,
        );
        if unsaved {
            painter.circle_filled(
                rect.right_center() - vec2(14.0, 0.0),
                3.5,
                theme::BRASS_BRIGHT,
            );
        }
        let tip = match extractions.error(&name) {
            Some(e) => format!("{}\nCan't extract: {e}", plugin.manifest.source.game),
            None if unsaved => format!("{}\nChanges not saved", plugin.manifest.source.game),
            None => plugin.manifest.source.game.clone(),
        };
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tip)
            .clicked()
            && !selected
        {
            state.plugin_editor.selected = Some(name);
            state.plugin_editor.message = None;
        }
    };
    let plugins: Vec<Plugin> = state
        .workshop
        .plugins
        .iter()
        .filter(|p| filter.is_empty() || p.name().to_lowercase().contains(&filter))
        .cloned()
        .collect();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (title, compositions) in [("Compositions", true), ("From your games", false)] {
                let list: Vec<&Plugin> = plugins
                    .iter()
                    .filter(|p| p.compose.is_some() == compositions)
                    .collect();
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 2))
                    .show(ui, |ui| {
                        kit::section(ui, &format!("{title}  ·  {}", list.len()));
                    });
                if list.is_empty() {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(14, 2))
                        .show(ui, |ui| {
                            ui.label(kit::hint(if compositions {
                                "None yet: press + to make one."
                            } else {
                                "None yet: ask your agent to extract something."
                            }));
                        });
                }
                for plugin in list {
                    row(ui, state, plugin);
                }
                ui.add_space(8.0);
            }
        });
}

/// The name field and Create button for a new composition.
fn create_composition(ui: &mut egui::Ui, state: &mut EditorState, open: &mut bool) {
    let mut go = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.plugin_editor.new_name)
                .hint_text(kit::hint("new-composition"))
                .desired_width(ui.available_width() - 66.0),
        );
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        go |= ui.button("Create").clicked();
    });
    ui.label(kit::hint("Lowercase letters, digits and dashes."));
    if !go {
        return;
    }
    let name = state.plugin_editor.new_name.trim().to_string();
    // Next to the other plugins of the user's content folders.
    let content = state
        .workshop
        .plugins
        .iter()
        .find_map(|p| p.dir.parent().map(Path::to_path_buf))
        .or_else(|| plugin_dirs().into_iter().next());
    match content
        .ok_or_else(|| "no plugins folder".to_string())
        .and_then(|c| create(&c, &name))
    {
        Ok(dir) => {
            state.log(
                LogLevel::Info,
                format!("Made composition {}", dir.display()),
            );
            state.scan_plugins();
            state.plugin_editor.selected = Some(name);
            state.plugin_editor.new_name.clear();
            state.plugin_editor.message = None;
            *open = false;
        }
        Err(e) => state.log(LogLevel::Warn, format!("Could not make it: {e}")),
    }
}

/// What an extraction plugin provides, read only.
fn extraction_view(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    extractions: &Extractions,
    plugin: &Plugin,
) {
    let Some(x) = extractions.get(plugin.name()) else {
        let why = extractions
            .error(plugin.name())
            .map_or("Extracting…".to_string(), |e| {
                format!("Not extracted: {e}")
            });
        kit::empty_state(ui, "Nothing to show yet", &why);
        return;
    };
    ui.label(kit::dim(
        "Made from your ROM by its recipe. Use its characters and abilities in a composition.",
    ));
    let characters = x
        .sprites
        .iter()
        .filter_map(|s| Some((s.id.as_str(), s.character.as_ref()?.moves())))
        .chain(
            x.models
                .iter()
                .filter_map(|m| Some((m.id.as_str(), m.character.as_ref()?.moves()))),
        );
    for (id, moves) in characters {
        section(ui, &format!("Character \"{id}\""));
        let actions: Vec<&str> = moves.actions.iter().map(|a| a.id.as_str()).collect();
        ui.label(if actions.is_empty() {
            "Moves: walking and its own movement only".to_string()
        } else {
            format!("Actions: {}", actions.join(", "))
        });
        if ui.button("Place in the scene").clicked() {
            state.workshop.place(plugin.name(), id, ROOT, false);
            state.screen = Screen::Workshop;
        }
    }
    for a in &x.abilities {
        section(ui, &format!("Ability \"{}\"", a.name));
        if !a.description.is_empty() {
            ui.label(&a.description);
        }
        let list = |v: Vec<&str>| {
            if v.is_empty() {
                "none".into()
            } else {
                v.join(", ")
            }
        };
        ui.label(
            RichText::new(format!(
                "Actions {} · items {} · poses (roles) {}",
                list(a.moves.actions.iter().map(|x| x.id.as_str()).collect()),
                list(a.items.iter().map(|x| x.name.as_str()).collect()),
                list(a.roles.iter().map(|x| x.name.as_str()).collect()),
            ))
            .color(theme::PARCHMENT_DIM),
        );
    }
    let counts = format!(
        "{} image(s), {} sprite(s), {} model(s), {} surface map(s), {} item(s)",
        x.images.len(),
        x.sprites.len(),
        x.models.len(),
        x.surfaces.len(),
        x.items.iter().map(|(_, i)| i.len()).sum::<usize>()
    );
    ui.add_space(8.0);
    ui.label(RichText::new(counts).color(theme::PARCHMENT_DIM));
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    kit::section(ui, text);
}

/// Whether Ctrl+Z (undo) or Ctrl+Y / Ctrl+Shift+Z (redo) was pressed.
fn undo_keys(ui: &egui::Ui) -> (bool, bool) {
    ui.input(|i| {
        let ctrl = i.modifiers.command;
        (
            ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
            ctrl && (i.key_pressed(egui::Key::Y)
                || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
        )
    })
}

/// Editing an extraction plugin's recipe, as a node graph.
fn recipe_view(ui: &mut egui::Ui, state: &mut EditorState, plugin: &Plugin) {
    let name = plugin.name().to_string();
    let revision = state.workshop.plugins_revision;
    let reload = state
        .plugin_editor
        .recipes
        .get(&name)
        .is_none_or(|d| d.revision != revision && !d.dirty());
    if reload {
        let draft = RecipeDraft::load(state, plugin);
        state.plugin_editor.recipes.insert(name.clone(), draft);
    } else if let Some(draft) = state.plugin_editor.recipes.get_mut(&name)
        && draft.revision != revision
    {
        draft.revision = revision;
        let on_disk = std::fs::read_to_string(plugin.dir.join("recipe.ron")).unwrap_or_default();
        draft.disk_changed = draft
            .graph
            .as_ref()
            .is_ok_and(|g| g.saved_text() != on_disk);
    }

    enum Action {
        Save,
        Revert,
    }
    let mut action = None;
    let typing = ui.ctx().egui_wants_keyboard_input();
    let Some(draft) = state.plugin_editor.recipes.get_mut(&name) else {
        return;
    };
    let dirty = draft.dirty();
    let mut clicks = [false; 5];
    let (can_undo, can_redo) = draft
        .graph
        .as_ref()
        .map_or((false, false), |g| (g.can_undo(), g.can_redo()));
    view_header(ui, &mut state.plugin_editor.tab, plugin, |ui| {
        clicks = graph_actions(
            ui,
            dirty,
            true,
            can_undo,
            can_redo,
            "Write recipe.ron (its comments stay); the plugin is extracted again",
        );
    });
    let [undo, redo, tidy, revert, save] = clicks;
    if revert {
        action = Some(Action::Revert);
    }
    if (save || save_key(ui)) && dirty {
        action = Some(Action::Save);
    }
    if disk_changed_banner(ui, "recipe.ron", &mut draft.disk_changed) {
        action = Some(Action::Revert);
    }
    let Ok(graph) = draft.graph.as_mut() else {
        if let Err(e) = &draft.graph {
            kit::empty_state(ui, "Can't show the recipe", &e.to_string());
        }
        return;
    };
    if undo {
        graph.undo();
    }
    if redo {
        graph.redo();
    }
    if tidy {
        graph.arrange();
    }
    let text = state.plugin_editor.tab == Tab::Text;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (steps, outputs, _) = graph.status();
                if !text {
                    graph.nav.toolbar(ui, &name);
                    ui.add_space(8.0);
                }
                ui.label(kit::hint(format!("{steps} steps · {outputs} outputs")));
                if graph.running() {
                    ui.add_space(8.0);
                    ui.label(kit::hint("Running…"))
                        .on_hover_text("Nodes show the last run until this one is done");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !text {
                        key_hints(
                            ui,
                            &[("Space", "add"), ("Del", "remove"), ("Alt+↑↓", "reorder")],
                        );
                    }
                    // What is wrong goes in the room left, cut short; a click goes to it.
                    let problems = graph.problems();
                    if let Some((_, first)) = problems.first() {
                        ui.add_space(12.0);
                        let text = match problems.len() {
                            1 => format!("⚠ {first}"),
                            n => format!("⚠ {n} problems · {first}"),
                        };
                        let response = ui
                            .add(
                                egui::Label::new(RichText::new(text).color(theme::WARN))
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Click to go to the next problem");
                        if response.clicked() {
                            graph.next_problem();
                        }
                    }
                });
            });
        });
    if let Some(message) = &graph.message {
        ui.label(RichText::new(format!("  {message}")).color(theme::WARN));
    }
    if !typing {
        let (undo, redo) = undo_keys(ui);
        if undo {
            graph.undo();
        } else if redo {
            graph.redo();
        }
    }
    ui.add_space(4.0);
    if text {
        graph.show_text(ui, &name);
    } else {
        let shown = graph.show(ui, &name);
        // Arranging a recipe that had no saved layout is no change to save.
        if shown.arranged && !dirty {
            draft.saved_layout = graph.layout();
        }
    }

    match action {
        Some(Action::Save) => {
            let message = match save_recipe(state, plugin) {
                Ok(message) => message,
                Err(e) => format!("Could not save: {e}"),
            };
            if let Some(Ok(graph)) = state
                .plugin_editor
                .recipes
                .get_mut(&name)
                .map(|d| d.graph.as_mut())
            {
                graph.message = Some(message);
            }
        }
        Some(Action::Revert) => {
            state.plugin_editor.recipes.remove(&name);
        }
        None => {}
    }
}

/// Writes a recipe being edited (`recipe.ron` and its layout) and extracts the plugin again.
fn save_recipe(state: &mut EditorState, plugin: &Plugin) -> Result<String, String> {
    let name = plugin.name();
    let draft = state
        .plugin_editor
        .recipes
        .get_mut(name)
        .ok_or("the recipe isn't open")?;
    let graph = draft.graph.as_mut().map_err(|e| e.clone())?;
    let layout = graph.layout();
    clockwork_library::files::write_atomic(plugin.dir.join("recipe.ron"), graph.text())
        .map_err(|e| e.to_string())?;
    recipe_graph::save_layout(&plugin.dir, &layout)?;
    graph.mark_saved();
    draft.saved_layout = layout;
    state.scan_plugins();
    if let Some(draft) = state.plugin_editor.recipes.get_mut(name) {
        draft.revision = state.workshop.plugins_revision;
        draft.disk_changed = false;
    }
    Ok(format!("Saved {}", plugin.dir.join("recipe.ron").display()))
}

/// Writes a composition being edited (`compose.ron` and its layout).
fn save_composition(state: &mut EditorState, plugin: &Plugin) -> Result<String, String> {
    let name = plugin.name();
    let draft = state
        .plugin_editor
        .graphs
        .get_mut(name)
        .ok_or("the composition isn't open")?;
    let built = draft.graph.build();
    if let Some(error) = built.errors.first() {
        return Err(error.clone());
    }
    let layout = draft.graph.layout();
    compose_graph::save_layout(&plugin.dir, &layout)?;
    save(&plugin.dir, &built.composition)?;
    draft.saved = (built.composition, layout);
    state.scan_plugins();
    if let Some(draft) = state.plugin_editor.graphs.get_mut(name) {
        draft.revision = state.workshop.plugins_revision;
        draft.disk_changed = false;
    }
    Ok(format!(
        "Saved {}",
        plugin.dir.join("compose.ron").display()
    ))
}

/// The plugin editor's graphs, driven by name (the editor's remote control).
#[derive(Debug, Clone, serde::Deserialize)]
pub enum GraphCommand {
    /// Finds nodes, as the Find field does.
    Find(String),
    /// Shows the whole graph.
    Fit,
    /// Shows a recipe node in the Inspector and moves the view to it: `Inspect("step 12")`.
    Inspect(String),
    Undo,
    Redo,
    Save,
    Revert,
    /// Changes the open recipe.
    Recipe(recipe_graph::Edit),
    /// Puts recipe nodes in a new closed group: `Group(["step 3", "step 4"], "Tiles")`.
    Group(Vec<String>, String),
    /// Opens a recipe group by its name.
    OpenGroup(String),
    /// Closes a recipe group by its name.
    CloseGroup(String),
    /// Takes a recipe group away by its name, keeping what is in it.
    Ungroup(String),
    /// Groups the recipe again by its section comments (or, with `true`, by its outputs).
    GroupAutomatically(bool),
    /// Shows the recipe as text (`true`) or as the graph.
    ShowText(bool),
}

/// Runs a graph command on the plugin shown (it must have been drawn once, to be open).
pub fn script(state: &mut EditorState, command: &GraphCommand) -> Result<(), String> {
    let name = state
        .plugin_editor
        .selected
        .clone()
        .ok_or("no plugin is shown")?;
    let plugin = state
        .workshop
        .plugin_index(&name)
        .map(|i| state.workshop.plugins[i].clone())
        .ok_or("the plugin isn't installed")?;
    if plugin.compose.is_some() {
        let draft = state
            .plugin_editor
            .graphs
            .get_mut(&name)
            .ok_or("the composition isn't open yet")?;
        match command {
            GraphCommand::Find(text) => draft.graph.nav.query = text.clone(),
            GraphCommand::Fit => draft.graph.nav.fit(),
            GraphCommand::Undo => draft.graph.undo(),
            GraphCommand::Redo => draft.graph.redo(),
            GraphCommand::Save => {
                state.plugin_editor.message = Some(save_composition(state, &plugin)?);
            }
            GraphCommand::Revert => {
                state.plugin_editor.graphs.remove(&name);
            }
            GraphCommand::Recipe(_)
            | GraphCommand::Inspect(_)
            | GraphCommand::Group(..)
            | GraphCommand::OpenGroup(_)
            | GraphCommand::CloseGroup(_)
            | GraphCommand::Ungroup(_)
            | GraphCommand::GroupAutomatically(_)
            | GraphCommand::ShowText(_) => {
                return Err("this plugin is a composition".into());
            }
        }
        return Ok(());
    }
    if let GraphCommand::ShowText(text) = command {
        state.plugin_editor.tab = if *text { Tab::Text } else { Tab::Recipe };
        return Ok(());
    }
    let draft = state
        .plugin_editor
        .recipes
        .get_mut(&name)
        .ok_or("the recipe isn't open yet")?;
    let graph = draft.graph.as_mut().map_err(|e| e.clone())?;
    match command {
        GraphCommand::Find(text) => graph.nav.query = text.clone(),
        GraphCommand::Fit => graph.nav.fit(),
        GraphCommand::Undo => graph.undo(),
        GraphCommand::Redo => graph.redo(),
        GraphCommand::Recipe(edit) => graph.edit(edit)?,
        GraphCommand::Inspect(node) => graph.inspect(node)?,
        GraphCommand::Group(nodes, name) => graph.group_nodes(nodes, name)?,
        GraphCommand::OpenGroup(name) => graph.set_group_open(graph.group_named(name)?, true),
        GraphCommand::CloseGroup(name) => graph.set_group_open(graph.group_named(name)?, false),
        GraphCommand::Ungroup(name) => graph.ungroup(graph.group_named(name)?),
        GraphCommand::ShowText(_) => {}
        GraphCommand::GroupAutomatically(by_outputs) => {
            graph.ungroup_all();
            graph.group_automatically(*by_outputs);
        }
        GraphCommand::Save => {
            let message = save_recipe(state, &plugin)?;
            state.plugin_editor.message = Some(message);
        }
        GraphCommand::Revert => {
            state.plugin_editor.recipes.remove(&name);
        }
    }
    Ok(())
}

/// What the shown plugin's graph is like, for remote-control reports.
pub fn describe(state: &EditorState) -> Option<String> {
    let name = state.plugin_editor.selected.as_deref()?;
    let unsaved = state.plugin_editor.unsaved().iter().any(|n| n == name);
    let what = if let Some(draft) = state.plugin_editor.graphs.get(name) {
        let built = draft.graph.build();
        format!(
            "composition, {} characters, errors {:?}",
            built.composition.characters.len(),
            built.errors
        )
    } else if let Some(draft) = state.plugin_editor.recipes.get(name) {
        match &draft.graph {
            Ok(graph) => graph.describe(),
            Err(e) => format!("unreadable recipe: {e}"),
        }
    } else {
        "not open".into()
    };
    Some(format!(
        "plugin {name}: {what}{}",
        if unsaved { ", unsaved" } else { "" }
    ))
}

/// Editing a composition plugin, as a node graph.
fn composition_view(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    extractions: &Extractions,
    plugin: &Plugin,
) {
    let name = plugin.name().to_string();
    let revision = state.workshop.plugins_revision;
    let reload = state
        .plugin_editor
        .graphs
        .get(&name)
        .is_none_or(|d| d.revision != revision && !d.dirty());
    if reload {
        state
            .plugin_editor
            .graphs
            .insert(name.clone(), GraphDraft::load(plugin, revision));
    } else if let Some(draft) = state.plugin_editor.graphs.get_mut(&name)
        && draft.revision != revision
    {
        draft.revision = revision;
        draft.disk_changed = plugin.compose.clone().unwrap_or_default() != draft.saved.0;
    }
    let characters_available = characters(&state.workshop.plugins, extractions);
    let abilities_available = abilities(&state.workshop.plugins, extractions);
    let still_extracting = !state.plugin_editor.wanted.is_empty();
    let typing = ui.ctx().egui_wants_keyboard_input();
    let editor = &mut state.plugin_editor;
    let Some(draft) = editor.graphs.get_mut(&name) else {
        return;
    };
    let built = draft.graph.build();
    let layout = draft.graph.layout();
    let dirty = built.composition != draft.saved.0 || layout != draft.saved.1;

    enum Action {
        Save,
        Revert,
        Place(String),
    }
    let mut action = None;
    let [undo, redo, tidy, revert, save] = {
        let mut clicks = [false; 5];
        let (can_undo, can_redo) = (draft.graph.can_undo(), draft.graph.can_redo());
        view_header(ui, &mut editor.tab, plugin, |ui| {
            clicks = graph_actions(
                ui,
                dirty,
                built.errors.is_empty(),
                can_undo,
                can_redo,
                "Write compose.ron; the content browser and scene pick it up",
            );
        });
        clicks
    };
    if revert {
        action = Some(Action::Revert);
    }
    if (save || save_key(ui)) && dirty && built.errors.is_empty() {
        action = Some(Action::Save);
    }
    if undo {
        draft.graph.undo();
    }
    if redo {
        draft.graph.redo();
    }
    if tidy {
        draft.graph.arrange();
    }
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                draft.graph.nav.toolbar(ui, &name);
                ui.add_space(8.0);
                ui.label(kit::hint("Characters made from other plugins"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    key_hints(
                        ui,
                        &[
                            ("Right-click", "add · place"),
                            ("Drag a pin", "wire or add"),
                            ("Del", "remove"),
                        ],
                    );
                });
            });
        });
    if disk_changed_banner(ui, "compose.ron", &mut draft.disk_changed) {
        action = Some(Action::Revert);
    }
    if !typing {
        let (undo, redo) = undo_keys(ui);
        if undo {
            draft.graph.undo();
        } else if redo {
            draft.graph.redo();
        }
    }
    for error in &built.errors {
        ui.label(RichText::new(format!("  ⛔ {error}")).color(theme::ERROR));
    }
    for note in &built.notes {
        ui.label(RichText::new(format!("  ⚠ {note}")).color(theme::WARN));
    }
    if characters_available.is_empty() {
        ui.label(
            RichText::new(if still_extracting {
                "Extracting plugins to find their characters…"
            } else {
                "No playable characters yet: add games to the Library and their plugins."
            })
            .color(theme::PARCHMENT_DIM),
        );
    }
    if let Some(message) = &editor.message {
        ui.label(RichText::new(message).color(theme::PARCHMENT_DIM));
    }
    ui.add_space(4.0);
    let shown = draft.graph.show(
        ui,
        &name,
        &characters_available,
        &abilities_available,
        !dirty,
    );
    // Arranging a graph that had no saved layout is no change to save.
    if shown.arranged && !dirty {
        draft.saved.1 = draft.graph.layout();
    }
    if let Some(id) = shown.place {
        action = Some(Action::Place(id));
    }

    match action {
        Some(Action::Save) => {
            state.plugin_editor.message = Some(match save_composition(state, plugin) {
                Ok(message) => message,
                Err(e) => format!("Could not save: {e}"),
            });
        }
        Some(Action::Revert) => {
            editor.graphs.remove(&name);
        }
        Some(Action::Place(id)) => {
            state.workshop.place(&name, &id, ROOT, false);
            state.screen = Screen::Workshop;
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::compose::{AbilityUse, ComposedCharacter, Tuning};

    #[test]
    fn names() {
        assert!(valid_name("runner-with-heros-items"));
        assert!(!valid_name("Runner!"));
        assert!(!valid_name(""));
    }

    #[test]
    fn saved_compositions_and_layouts_load_again() {
        let dir = std::env::temp_dir().join(format!("clockwork-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugin_dir = create(&dir, "test-composition").unwrap();
        let composition = Composition {
            characters: vec![ComposedCharacter {
                id: "hero_plus".into(),
                base: OutputRef {
                    plugin: "game-a-hero".into(),
                    output: "hero".into(),
                },
                abilities: vec![AbilityUse {
                    ability: OutputRef {
                        plugin: "game-a-hero".into(),
                        output: "bow".into(),
                    },
                    roles: [(
                        "draw_bow".to_string(),
                        RoleBinding::Animation("punch".into()),
                    )]
                    .into(),
                    use_defaults: true,
                }],
                tuning: Tuning {
                    speed: 1.5,
                    jump: 1.0,
                },
                hand: None,
            }],
        };
        save(&plugin_dir, &composition).unwrap();
        let plugin = Plugin::load(&plugin_dir).unwrap();
        assert_eq!(plugin.compose, Some(composition));
        assert!(plugin.recipe.outputs.is_empty());

        let draft = GraphDraft::load(&plugin, 0);
        compose_graph::save_layout(&plugin_dir, &draft.saved.1).unwrap();
        assert_eq!(compose_graph::load_layout(&plugin_dir), draft.saved.1);
        assert_eq!(GraphDraft::load(&plugin, 0).saved, draft.saved);

        assert!(create(&dir, "test-composition").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

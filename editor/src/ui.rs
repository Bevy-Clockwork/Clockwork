use bevy::{
    camera::Viewport,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::PrimaryWindow,
};
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, RichText, Stroke, vec2},
};

use crate::{
    ViewportCamera,
    browser::{BottomTab, Browser},
    covers::{self, Covers},
    decomps, kit,
    palette::{self, AppCommand, Palette, Toasts},
    scene::History,
    settings::{self, Settings},
    state::{EditorState, LogLevel, Screen},
    theme,
    workshop::Extractions,
    workshop_ui,
};

use clockwork_library::ROM_EXTENSIONS;
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

const COVER_H: f32 = 170.0;
const SHELF_PAD: f32 = 12.0;
const TITLE_H: f32 = 40.0;
const CARD_GAP: f32 = 22.0;
const PLANK_H: f32 = 14.0;

/// Height of the top bar and the status bar.
const TOP_H: f32 = 40.0;
const STATUS_H: f32 = 24.0;

/// UI zoom steps for Ctrl+= and Ctrl+-.
const ZOOM_STEP: f32 = 0.1;

#[derive(Default)]
pub struct Dialogs {
    roms: Option<Task<Option<Vec<rfd::FileHandle>>>>,
    /// The game a cover is being picked for, and the dialog.
    cover: Option<(String, Task<Option<rfd::FileHandle>>)>,
    /// The title of a game being exported, and the dialog choosing where it goes.
    export: Option<(String, Task<Option<rfd::FileHandle>>)>,
}

/// Things the UI asked for this frame, applied after drawing.
#[derive(Default)]
struct Actions {
    import_roms: bool,
    pick_cover: Option<String>,
    reset_cover: Option<String>,
    remove: Option<usize>,
    /// Export the scene as a game, with this title.
    export: Option<String>,
    /// Commands from menus, the palette and shortcuts.
    commands: Vec<AppCommand>,
}

/// How the Library shows its games.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum LibraryLayout {
    #[default]
    Shelves,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum LibrarySort {
    #[default]
    Added,
    Title,
    Platform,
}

/// The Library's view options.
#[derive(Default)]
pub struct LibraryView {
    layout: LibraryLayout,
    sort: LibrarySort,
    /// Only games of this platform, if set.
    platform: Option<&'static str>,
}

/// The parts of the window every screen shares: palette, toasts and Library view options.
#[derive(Default)]
pub struct Shell {
    palette: Palette,
    toasts: Toasts,
    library: LibraryView,
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn editor_ui(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut covers: ResMut<Covers>,
    mut dialogs: Local<Dialogs>,
    mut shell: Local<Shell>,
    mut browser: ResMut<Browser>,
    extractions: Res<Extractions>,
    mut themed: Local<bool>,
    mut agents_checked: Local<f64>,
    mut settings: ResMut<Settings>,
    mut controls: ResMut<crate::controls::ControlsUi>,
    mut history: ResMut<History>,
    mut settings_category: Local<settings::Category>,
    mut camera: Single<&mut Camera, With<ViewportCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if !*themed {
        // Fonts take effect next frame, so skip drawing until the display font exists.
        theme::apply(&ctx);
        // Ctrl+= and Ctrl+- change the saved UI scale instead (see `global_shortcuts`).
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        *themed = true;
        return Ok(());
    }

    // UI scale from Settings.
    if (ctx.zoom_factor() - settings.ui_scale).abs() > 0.001 {
        ctx.set_zoom_factor(settings.ui_scale);
    }
    // The browser's thumbnail slider and Settings share one value.
    browser.thumb_size = settings.thumb_size;

    poll_dialogs(&ctx, &mut dialogs, &mut state, &mut covers);
    state.claude_connected = settings.claude_connected;
    let time = ctx.input(|i| i.time);
    if time - *agents_checked > 2.0 || *agents_checked == 0.0 {
        *agents_checked = time;
        state.agents = clockwork_library::agents::running();
    }
    let dropped: Vec<_> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect()
    });
    if !dropped.is_empty() {
        state.import(dropped);
    }

    let mut actions = Actions::default();
    if state.player.is_none() && !controls.blocked() {
        global_shortcuts(&ctx, &state, &settings, &mut shell.palette, &mut actions);
    }
    let mut root = egui::Ui::new(
        ctx.clone(),
        "editor_root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let viewport = if state.player.is_some() {
        // An exported game: just the game, and Escape quits.
        let (rect, import) = crate::player::screen(&ctx, &mut root, &mut state);
        actions.import_roms |= import;
        if !crate::game_ui::gameplay_uses_key(&state, &settings, "Escape")
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            exit.write(AppExit::Success);
        }
        rect
    } else if game_view(&ctx, &mut state, &settings) {
        // Play as the game: the whole window.
        Some(ctx.viewport_rect())
    } else {
        top_bar(&mut root, &mut state, &history, &mut actions);
        if !controls.blocked() && palette_requested(&ctx) {
            shell.palette.toggle();
        }
        if let Some(query) = state.palette_request.take() {
            shell.palette.open_with(query);
        }
        status_bar(&mut root, &state, &extractions, &mut actions);
        match state.screen {
            Screen::Library => {
                library_screen(
                    &mut root,
                    &mut state,
                    &mut covers,
                    &mut shell.library,
                    &mut actions,
                );
                None
            }
            Screen::Workshop => Some(workshop_ui::workshop_screen(
                &mut root,
                &mut state,
                &mut browser,
                &extractions,
                &settings,
            )),
            Screen::Plugins => {
                crate::plugin_editor::plugin_editor_screen(&mut root, &mut state, &extractions);
                None
            }
            Screen::Settings => {
                let input_map = crate::game_ui::base_input_map(&state);
                settings::settings_screen(
                    &mut root,
                    &mut state,
                    &mut settings,
                    &mut settings_category,
                    &mut controls,
                    &input_map,
                );
                None
            }
        }
    };
    export_dialog(&ctx, &mut state, &mut actions);
    quit_dialog(&ctx, &mut state, &mut exit);
    show_drop_overlay(&ctx);
    if state.player.is_none() && !controls.blocked() {
        if let Some(command) = palette::show(&ctx, &mut shell.palette, &state) {
            actions.commands.push(command);
        }
        palette::shortcuts_window(&ctx, &mut shell.palette.shortcuts);
        if palette::toasts(&ctx, &mut shell.toasts, &state) {
            actions.commands.push(AppCommand::ShowLog);
        }
    }

    // Undo history: a change becomes a step once nothing is being dragged or typed.
    let typing = ctx.egui_wants_keyboard_input() || controls.blocked();
    state.workshop.typing = typing;
    let settled = !ctx.input(|i| i.pointer.any_down()) && !typing;
    history.track(&state.workshop.scene, settled);
    // The Plugins screen undoes its own edits.
    if !typing && state.screen != Screen::Plugins {
        let (undo, redo) = ctx.input(|i| {
            let ctrl = i.modifiers.command;
            (
                ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                ctrl && (i.key_pressed(egui::Key::Y)
                    || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if undo {
            undo_redo(&mut state, &mut history, true);
        } else if redo {
            undo_redo(&mut state, &mut history, false);
        }
    }

    for command in std::mem::take(&mut actions.commands) {
        run_command(
            command,
            &mut state,
            &mut history,
            &mut settings,
            &mut settings_category,
            &mut browser,
            &mut shell.palette,
            &mut actions,
            &mut exit,
        );
    }
    if browser.thumb_size != settings.thumb_size {
        settings.thumb_size = browser.thumb_size;
        if let Err(e) = settings.save() {
            state.log(LogLevel::Warn, format!("Could not save settings: {e}"));
        }
    }

    if actions.import_roms && dialogs.roms.is_none() {
        dialogs.roms = Some(
            AsyncComputeTaskPool::get().spawn(
                rfd::AsyncFileDialog::new()
                    .set_title("Import games")
                    .add_filter("Game images and executables", &ROM_EXTENSIONS)
                    .add_filter("Zip archives", &["zip"])
                    .add_filter("All files", &["*"])
                    .pick_files(),
            ),
        );
    }
    if let Some(title) = actions.export
        && dialogs.export.is_none()
    {
        let task = AsyncComputeTaskPool::get().spawn(
            rfd::AsyncFileDialog::new()
                .set_title("Where should the game go?")
                .pick_folder(),
        );
        dialogs.export = Some((title, task));
    }
    if let Some(sha1) = actions.pick_cover
        && dialogs.cover.is_none()
    {
        let task = AsyncComputeTaskPool::get().spawn(
            rfd::AsyncFileDialog::new()
                .set_title("Choose cover art")
                .add_filter("Images", &IMAGE_EXTENSIONS)
                .pick_file(),
        );
        dialogs.cover = Some((sha1, task));
    }
    if let Some(sha1) = actions.reset_cover {
        covers.reset(&sha1);
    }
    if let Some(i) = actions.remove {
        state.remove(i);
    }

    set_viewport(&mut camera, &window, viewport, ctx.pixels_per_point());
    Ok(())
}

/// Shortcuts that work on every screen. Text fields keep their own keys, except the palette's.
fn global_shortcuts(
    ctx: &egui::Context,
    state: &EditorState,
    settings: &Settings,
    palette: &mut Palette,
    actions: &mut Actions,
) {
    use egui::{Key, Modifiers};
    let command = Modifiers::COMMAND;
    ctx.input_mut(|i| {
        if i.consume_key(command, Key::K) || i.consume_key(command | Modifiers::SHIFT, Key::P) {
            palette.toggle();
        }
        if !crate::game_ui::gameplay_uses_key(state, settings, "F1")
            && i.consume_key(Modifiers::NONE, Key::F1)
        {
            palette.shortcuts = !palette.shortcuts;
        }
        for (key, screen) in [
            (Key::Num1, Screen::Library),
            (Key::Num2, Screen::Workshop),
            (Key::Num3, Screen::Plugins),
            (Key::Num4, Screen::Settings),
        ] {
            if i.consume_key(command, key) {
                actions.commands.push(AppCommand::Go(screen));
            }
        }
        if i.consume_key(command, Key::O) {
            actions.commands.push(AppCommand::ImportRoms);
        }
        if i.consume_key(command, Key::E) {
            actions.commands.push(AppCommand::ExportGame);
        }
        if i.consume_key(command, Key::Equals) || i.consume_key(command, Key::Plus) {
            actions.commands.push(AppCommand::ZoomIn);
        }
        if i.consume_key(command, Key::Minus) {
            actions.commands.push(AppCommand::ZoomOut);
        }
        if i.consume_key(command, Key::Num0) {
            actions.commands.push(AppCommand::ZoomReset);
        }
        if state.screen == Screen::Workshop
            && !crate::game_ui::gameplay_uses_key(state, settings, "F5")
            && i.consume_key(Modifiers::NONE, Key::F5)
        {
            actions.commands.push(AppCommand::TogglePlay);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn run_command(
    command: AppCommand,
    state: &mut EditorState,
    history: &mut History,
    settings: &mut Settings,
    settings_category: &mut settings::Category,
    browser: &mut Browser,
    palette: &mut Palette,
    actions: &mut Actions,
    exit: &mut MessageWriter<AppExit>,
) {
    let scale = settings.ui_scale;
    let mut zoom = |to: f32| {
        settings.ui_scale = ((to / 0.05).round() * 0.05).clamp(0.75, 2.0);
        if let Err(e) = settings.save() {
            state.log(LogLevel::Warn, format!("Could not save settings: {e}"));
        }
    };
    match command {
        AppCommand::Go(screen) => state.screen = screen,
        AppCommand::ImportRoms => actions.import_roms = true,
        AppCommand::ExportGame => state.workshop.export = Some(default_export_title(state)),
        AppCommand::Undo => undo_redo(state, history, true),
        AppCommand::Redo => undo_redo(state, history, false),
        AppCommand::RescanPlugins => {
            state.scan_plugins();
            state.log(LogLevel::Info, "Plugins rescanned");
        }
        AppCommand::TogglePlay => {
            state.screen = Screen::Workshop;
            state.workshop.play_request = Some(state.workshop.playing.is_none());
        }
        AppCommand::ToggleSurfaces => {
            state.workshop.show_surfaces = !state.workshop.show_surfaces;
        }
        AppCommand::ToggleSnap => state.workshop.snap = !state.workshop.snap,
        AppCommand::Tool(tool) => {
            state.screen = Screen::Workshop;
            state.workshop.tool = tool;
        }
        AppCommand::AddEntity(kind) => {
            state.screen = Screen::Workshop;
            state.workshop.entity_request = crate::entities::Behavior::new(kind);
        }
        AppCommand::FrameSelection => {
            state.screen = Screen::Workshop;
            state.workshop.focus_request = state.workshop.selected_node;
        }
        AppCommand::ShowLog => {
            state.screen = Screen::Workshop;
            browser.tab = BottomTab::Log;
        }
        AppCommand::Settings(category) => {
            state.screen = Screen::Settings;
            *settings_category = category;
        }
        AppCommand::Shortcuts => palette.shortcuts = true,
        AppCommand::ZoomIn => zoom(scale + ZOOM_STEP),
        AppCommand::ZoomOut => zoom(scale - ZOOM_STEP),
        AppCommand::ZoomReset => zoom(1.0),
        AppCommand::OpenPlugin(name) => {
            state.screen = Screen::Plugins;
            state.plugin_editor.selected = Some(name);
            state.plugin_editor.message = None;
        }
        AppCommand::SelectNode(id) => {
            state.screen = Screen::Workshop;
            state.workshop.select_only(id);
            state.workshop.focus_request = Some(id);
        }
        AppCommand::ShowGame(index) => {
            state.screen = Screen::Library;
            state.selected = Some(index);
        }
        AppCommand::Quit => {
            if state.plugin_editor.unsaved().is_empty() {
                exit.write(AppExit::Success);
            } else {
                state.quit_asked = true;
            }
        }
    }
}

/// The title the Export dialog starts with: the scene's name, unless it is the default.
fn default_export_title(state: &EditorState) -> String {
    match state.workshop.scene.get(crate::scene::ROOT) {
        Some(n) if n.name != "Scene" => n.name.clone(),
        _ => "My Game".to_string(),
    }
}

/// Steps the scene back or forward and keeps the selection valid.
fn undo_redo(state: &mut EditorState, history: &mut History, undo: bool) {
    let scene = &mut state.workshop.scene;
    let done = if undo {
        history.undo(scene)
    } else {
        history.redo(scene)
    };
    if !done {
        return;
    }
    let ws = &mut state.workshop;
    if ws
        .selected_node
        .is_some_and(|id| ws.scene.get(id).is_none())
    {
        ws.selected_node = None;
    }
    if ws.adjusting.is_some_and(|id| ws.scene.get(id).is_none()) {
        ws.adjusting = None;
    }
}

/// Plugins the scene uses that can be exported: name, game and ROM hashes, and where they are.
pub(crate) fn plugins_to_export(
    state: &EditorState,
) -> Vec<(crate::export::RequiredPlugin, std::path::PathBuf)> {
    let index = clockwork_recipe::assets::AssetIndex::build(&state.workshop.plugins);
    crate::export::plugins_used(&state.workshop.scene, &index)
        .into_iter()
        .filter_map(|name| {
            let plugin = state
                .workshop
                .plugins
                .iter()
                .find(|p| p.manifest.plugin.name == name)?;
            Some((
                crate::export::RequiredPlugin {
                    name,
                    game: plugin.manifest.source.game.clone(),
                    sha1: plugin.manifest.source.sha1.clone(),
                },
                plugin.dir.clone(),
            ))
        })
        .collect()
}

/// The Export dialog: a title and a button that asks where to put the game.
fn export_dialog(ctx: &egui::Context, state: &mut EditorState, actions: &mut Actions) {
    let Some(mut title) = state.workshop.export.clone() else {
        return;
    };
    let mut open = true;
    let mut close = false;
    egui::Window::new("Export game")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(380.0);
            let used = crate::export::plugins_used(
                &state.workshop.scene,
                &clockwork_recipe::assets::AssetIndex::build(&state.workshop.plugins),
            );
            ui.label(
                "Packages this scene as a game you can send to someone: the player, the scene and \
                 the recipes it needs. It contains no game data; whoever plays it locates their \
                 own copy of each game.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Title");
                ui.text_edit_singleline(&mut title);
            });
            ui.add_space(4.0);
            ui.small(
                RichText::new(if used.is_empty() {
                    "The scene shows nothing from a plugin yet.".to_string()
                } else {
                    format!("Uses: {}", used.join(", "))
                })
                .color(theme::PARCHMENT_DIM),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !used.is_empty() && !title.trim().is_empty(),
                        egui::Button::new("Choose folder…"),
                    )
                    .clicked()
                {
                    actions.export = Some(title.trim().to_string());
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    state.workshop.export = if open && !close { Some(title) } else { None };
}

fn poll_dialogs(
    ctx: &egui::Context,
    dialogs: &mut Dialogs,
    state: &mut EditorState,
    covers: &mut Covers,
) {
    if let Some(picked) = dialogs
        .export
        .as_mut()
        .and_then(|(_, t)| block_on(poll_once(t)))
    {
        let (title, _) = dialogs.export.take().unwrap();
        if let Some(folder) = picked {
            let plugins = plugins_to_export(state);
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    crate::export::export(
                        &state.workshop.scene,
                        &plugins,
                        &title,
                        folder.path(),
                        &exe,
                    )
                });
            match result {
                Ok(path) => {
                    let text = format!("Exported to {}", path.display());
                    state.log(LogLevel::Info, text.clone());
                    state.workshop.toast = Some((text, 6.0));
                }
                Err(e) => state.log(LogLevel::Error, format!("Export failed: {e}")),
            }
        }
    }
    if let Some(picked) = dialogs.roms.as_mut().and_then(|t| block_on(poll_once(t))) {
        dialogs.roms = None;
        if let Some(files) = picked {
            state.import(files.iter().map(|f| f.path().to_path_buf()));
        }
    }
    if let Some(picked) = dialogs
        .cover
        .as_mut()
        .and_then(|(_, t)| block_on(poll_once(t)))
    {
        let (sha1, _) = dialogs.cover.take().unwrap();
        if let Some(file) = picked {
            match covers.set(ctx, &sha1, file.path()) {
                Ok(()) => state.log(LogLevel::Info, "Cover art updated"),
                Err(e) => state.log(LogLevel::Error, format!("Could not set cover art: {e}")),
            }
        }
    }
}

/// Makes the scene camera render only into the Workshop viewport, and not at all elsewhere.
/// `pixels_per_point` converts egui's points (which include the UI scale) to physical pixels.
fn set_viewport(
    camera: &mut Camera,
    window: &Window,
    rect: Option<egui::Rect>,
    pixels_per_point: f32,
) {
    let scale = pixels_per_point;
    let viewport = rect.and_then(|rect| {
        let pos = UVec2::new((rect.min.x * scale) as u32, (rect.min.y * scale) as u32);
        let size = UVec2::new(
            (rect.width() * scale) as u32,
            (rect.height() * scale) as u32,
        );
        let fits =
            pos.x + size.x <= window.physical_width() && pos.y + size.y <= window.physical_height();
        (size.x > 0 && size.y > 0 && fits).then(|| Viewport {
            physical_position: pos,
            physical_size: size,
            ..default()
        })
    });
    camera.is_active = viewport.is_some();
    camera.viewport = viewport;
}

/// Whether Play shows as the game, in the whole window; F11 switches while playing and
/// Escape goes back to the editor (still playing). The remote control can show the viewport
/// so without playing, for rendering checks.
fn game_view(ctx: &egui::Context, state: &mut EditorState, settings: &Settings) -> bool {
    if state.screen != Screen::Workshop
        || (state.workshop.playing.is_none() && !state.workshop.game_view)
    {
        return false;
    }
    let (f11, escape) = ctx.input(|i| {
        (
            !crate::game_ui::gameplay_uses_key(state, settings, "F11")
                && i.key_pressed(egui::Key::F11),
            !crate::game_ui::gameplay_uses_key(state, settings, "Escape")
                && i.key_pressed(egui::Key::Escape),
        )
    });
    if f11 {
        state.workshop.game_view = !state.workshop.game_view;
    } else if escape && state.workshop.game_view {
        state.workshop.game_view = false;
        // The same press mustn't stop Play once the editor is back.
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    }
    if state.workshop.game_view {
        state.workshop.viewport_hovered = true;
    }
    state.workshop.game_view
}

// ---------------------------------------------------------------------------------------------
// Top bar

fn top_bar(root: &mut egui::Ui, state: &mut EditorState, history: &History, actions: &mut Actions) {
    egui::Panel::top("top_bar")
        .exact_size(TOP_H)
        .frame(egui::Frame::NONE)
        .show_separator_line(false)
        .show(root, |ui| {
            let rect = ui.max_rect();
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, theme::HEADER);
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1.0, theme::BORDER),
            );

            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);
                let (logo, _) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
                let t = ui.input(|i| i.time) as f32;
                theme::gear(
                    ui.painter(),
                    logo.center(),
                    11.0,
                    10,
                    t * 0.15,
                    theme::BRASS,
                    theme::HEADER,
                );
                // The wordmark is painted so its capitals sit centred on the gear; as a label,
                // the serif's tall line box leaves it a little high.
                let galley = ui.painter().layout_no_wrap(
                    "Clockwork".into(),
                    theme::display(17.0),
                    theme::BRASS_BRIGHT,
                );
                let (word, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                let y = logo.center().y - galley.size().y / 2.0 + 2.0;
                ui.painter()
                    .galley(egui::pos2(word.left(), y), galley, theme::BRASS_BRIGHT);
                ui.add_space(10.0);
                menus(ui, state, history, actions);
                ui.add_space(8.0);
                let (sep, _) = ui.allocate_exact_size(vec2(1.0, 20.0), egui::Sense::hover());
                ui.painter().rect_filled(sep, 0.0, theme::BORDER_STRONG);
                ui.add_space(8.0);
                screen_tabs(ui, state);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    command_button(ui);
                });
            });
        });
}

/// The menu bar: File, Edit, View, Help. Every entry names its shortcut.
fn menus(ui: &mut egui::Ui, state: &mut EditorState, history: &History, actions: &mut Actions) {
    let item = |ui: &mut egui::Ui, text: &str, keys: &str| {
        ui.add(egui::Button::new(text).shortcut_text(kit::hint(keys)))
    };
    // Menu titles look like words until hovered or open, as in a menu bar.
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = Stroke::NONE;
        w.hovered.bg_stroke = Stroke::NONE;
        w.open.bg_stroke = Stroke::NONE;
        ui.spacing_mut().button_padding = vec2(7.0, 3.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.menu_button("File", |ui| {
            if item(ui, "Import games…", "Ctrl+O").clicked() {
                actions.commands.push(AppCommand::ImportRoms);
            }
            if item(ui, "Export game…", "Ctrl+E").clicked() {
                actions.commands.push(AppCommand::ExportGame);
            }
            ui.separator();
            if item(ui, "Rescan plugins", "").clicked() {
                actions.commands.push(AppCommand::RescanPlugins);
            }
            if item(ui, "Settings…", "Ctrl+4").clicked() {
                actions.commands.push(AppCommand::Go(Screen::Settings));
            }
            ui.separator();
            if item(ui, "Quit", "").clicked() {
                actions.commands.push(AppCommand::Quit);
            }
        });
        ui.menu_button("Edit", |ui| {
            // The Plugins screen's graphs keep their own history (their toolbar has it).
            let scene = state.screen != Screen::Plugins;
            let undo = egui::Button::new("Undo").shortcut_text(kit::hint("Ctrl+Z"));
            if ui.add_enabled(scene && history.can_undo(), undo).clicked() {
                actions.commands.push(AppCommand::Undo);
            }
            let redo = egui::Button::new("Redo").shortcut_text(kit::hint("Ctrl+Y"));
            if ui.add_enabled(scene && history.can_redo(), redo).clicked() {
                actions.commands.push(AppCommand::Redo);
            }
            ui.separator();
            if item(ui, "Command palette…", "Ctrl+K").clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(toggle_palette_id(), true));
            }
        });
        ui.menu_button("View", |ui| {
            for (screen, label, keys) in [
                (Screen::Library, "Library", "Ctrl+1"),
                (Screen::Workshop, "Workshop", "Ctrl+2"),
                (Screen::Plugins, "Plugins", "Ctrl+3"),
                (Screen::Settings, "Settings", "Ctrl+4"),
            ] {
                let button = egui::Button::selectable(state.screen == screen, label)
                    .shortcut_text(kit::hint(keys));
                if ui.add(button).clicked() {
                    actions.commands.push(AppCommand::Go(screen));
                }
            }
            ui.separator();
            if item(ui, "Show the log", "").clicked() {
                actions.commands.push(AppCommand::ShowLog);
            }
            ui.separator();
            if item(ui, "Zoom in", "Ctrl+=").clicked() {
                actions.commands.push(AppCommand::ZoomIn);
            }
            if item(ui, "Zoom out", "Ctrl+-").clicked() {
                actions.commands.push(AppCommand::ZoomOut);
            }
            if item(ui, "Reset zoom", "Ctrl+0").clicked() {
                actions.commands.push(AppCommand::ZoomReset);
            }
        });
        ui.menu_button("Help", |ui| {
            if item(ui, "Keyboard shortcuts", "F1").clicked() {
                actions.commands.push(AppCommand::Shortcuts);
            }
            if item(ui, "Command palette…", "Ctrl+K").clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(toggle_palette_id(), true));
            }
            ui.separator();
            ui.label(kit::hint(format!(
                "Clockwork {} · core {}",
                env!("CARGO_PKG_VERSION"),
                clockwork_core::CORE_VERSION
            )));
        });
    });
}

/// Workspace tabs, like Blender's: the four screens, the open one underlined.
fn screen_tabs(ui: &mut egui::Ui, state: &mut EditorState) {
    for (screen, label, keys) in [
        (Screen::Library, "Library", "Ctrl+1"),
        (Screen::Workshop, "Workshop", "Ctrl+2"),
        (Screen::Plugins, "Plugins", "Ctrl+3"),
        (Screen::Settings, "Settings", "Ctrl+4"),
    ] {
        let active = state.screen == screen;
        let unsaved = screen == Screen::Plugins && !state.plugin_editor.unsaved().is_empty();
        let text = RichText::new(label)
            .font(theme::strong(13.0))
            .color(if active {
                theme::PARCHMENT
            } else {
                theme::PARCHMENT_DIM
            });
        let mut button = egui::Button::new(text).frame_when_inactive(active);
        if active {
            button = button.fill(theme::RAISED).stroke(Stroke::NONE);
        }
        let response = ui
            .add(button.min_size(vec2(0.0, 26.0)))
            .on_hover_text(kit::tip(label, keys));
        if active {
            let r = response.rect;
            ui.painter().line_segment(
                [
                    egui::pos2(r.left() + 6.0, r.bottom() + 6.0),
                    egui::pos2(r.right() - 6.0, r.bottom() + 6.0),
                ],
                Stroke::new(2.0, theme::BRASS),
            );
        }
        if unsaved {
            ui.painter().circle_filled(
                response.rect.right_top() + vec2(-4.0, 5.0),
                3.0,
                theme::BRASS_BRIGHT,
            );
        }
        if response.clicked() {
            state.screen = screen;
        }
    }
}

/// Looks like a search field; opens the command palette.
fn command_button(ui: &mut egui::Ui) {
    let width = 250.0_f32.min(ui.available_width() - 8.0);
    if width < 120.0 {
        if kit::icon_button(ui, "🔍", "Command palette  ·  Ctrl+K").clicked() {
            ui.ctx()
                .data_mut(|d| d.insert_temp(toggle_palette_id(), true));
        }
        return;
    }
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        theme::RADIUS,
        ui.visuals().text_edit_bg_color(),
        Stroke::new(
            1.0,
            if response.hovered() {
                theme::BORDER_STRONG
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_center() + vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "🔍  Search or run a command…",
        egui::FontId::proportional(12.5),
        theme::FAINT,
    );
    painter.text(
        rect.right_center() - vec2(10.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "Ctrl K",
        egui::FontId::monospace(11.0),
        theme::FAINT,
    );
    let response = response.on_hover_cursor(egui::CursorIcon::Text);
    if response.clicked() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(toggle_palette_id(), true));
    }
}

/// Set by the command button and menus; read by `palette_requested`.
fn toggle_palette_id() -> egui::Id {
    egui::Id::new("toggle_palette")
}

/// Whether the command button asked for the palette this frame.
fn palette_requested(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(toggle_palette_id()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// Library screen

fn library_screen(
    root: &mut egui::Ui,
    state: &mut EditorState,
    covers: &mut Covers,
    view: &mut LibraryView,
    actions: &mut Actions,
) {
    if state.selected_entry().is_some() {
        egui::Panel::right("details")
            .resizable(true)
            .default_size(340.0)
            .size_range(280.0..=520.0)
            .frame(kit::panel_frame())
            .show(root, |ui| details_panel(ui, state, covers, actions));
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::BG_DEEP))
        .show(root, |ui| {
            library_toolbar(ui, state, view, actions);
            let visible = visible_games(state, view);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    if state.library.entries().is_empty() {
                        empty_shelf(ui, actions);
                    } else if visible.is_empty() {
                        kit::empty_state(
                            ui,
                            "No games match",
                            "Try another search, or show every platform.",
                        );
                    } else {
                        library_keys(ui, state, &visible);
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .show(ui, |ui| match view.layout {
                                LibraryLayout::Shelves => {
                                    shelves(ui, state, covers, &visible, actions)
                                }
                                LibraryLayout::List => game_list(ui, state, &visible, actions),
                            });
                    }
                });
        });
}

/// The Library's toolbar: title and count, search, platform filter, sort, layout and Import.
fn library_toolbar(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    view: &mut LibraryView,
    actions: &mut Actions,
) {
    let total = state.library.entries().len();
    let mut platforms: Vec<&'static str> = state
        .library
        .entries()
        .iter()
        .map(|e| e.info.platform)
        .collect();
    platforms.sort();
    platforms.dedup();
    if view.platform.is_some_and(|p| !platforms.contains(&p)) {
        view.platform = None;
    }

    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(20, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Library")
                        .font(theme::display(20.0))
                        .color(theme::PARCHMENT),
                );
                kit::pill(
                    ui,
                    &match total {
                        1 => "1 game".to_string(),
                        n => format!("{n} games"),
                    },
                    theme::PARCHMENT_DIM,
                );
                ui.add_space(14.0);
                if platforms.len() > 1 {
                    let mut choice = view.platform;
                    let mut options: Vec<(Option<&'static str>, String, &str)> =
                        vec![(None, "All".to_string(), "Every platform")];
                    options.extend(platforms.iter().map(|p| (Some(*p), p.to_uppercase(), "")));
                    let options: Vec<(Option<&'static str>, &str, &str)> = options
                        .iter()
                        .map(|(v, label, tip)| (*v, label.as_str(), *tip))
                        .collect();
                    if kit::segmented(ui, &mut choice, &options) {
                        view.platform = choice;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::primary_button(ui, true, "+  Import games")
                        .on_hover_text(kit::tip("Add games you own to the Library", "Ctrl+O"))
                        .clicked()
                    {
                        actions.import_roms = true;
                    }
                    ui.add_space(8.0);
                    kit::segmented(
                        ui,
                        &mut view.layout,
                        &[
                            (LibraryLayout::List, "☰", "List"),
                            (LibraryLayout::Shelves, "▦", "Shelves"),
                        ],
                    );
                    egui::ComboBox::from_id_salt("library_sort")
                        .width(118.0)
                        .selected_text(format!(
                            "Sort: {}",
                            match view.sort {
                                LibrarySort::Added => "Added",
                                LibrarySort::Title => "Title",
                                LibrarySort::Platform => "Platform",
                            }
                        ))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut view.sort, LibrarySort::Added, "Added");
                            ui.selectable_value(&mut view.sort, LibrarySort::Title, "Title");
                            ui.selectable_value(&mut view.sort, LibrarySort::Platform, "Platform");
                        });
                    kit::search_field(ui, &mut state.search, "Search games…", 220.0);
                });
            });
        });
    let rect = ui.min_rect();
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::BORDER),
    );
}

/// Indices of the games the search and filters leave, in the chosen order.
fn visible_games(state: &EditorState, view: &LibraryView) -> Vec<usize> {
    let query = state.search.to_lowercase();
    let mut visible: Vec<usize> = state
        .library
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| view.platform.is_none_or(|p| e.info.platform == p))
        .filter(|(_, e)| {
            query.is_empty()
                || state.title(e).to_lowercase().contains(&query)
                || e.info.game_code.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect();
    let entries = state.library.entries();
    match view.sort {
        LibrarySort::Added => {}
        LibrarySort::Title => {
            visible.sort_by_key(|&i| state.title(&entries[i]).to_lowercase());
        }
        LibrarySort::Platform => visible.sort_by_key(|&i| {
            (
                entries[i].info.platform,
                state.title(&entries[i]).to_lowercase(),
            )
        }),
    }
    visible
}

/// Arrow keys move the selection through the visible games; Escape clears it.
fn library_keys(ui: &egui::Ui, state: &mut EditorState, visible: &[usize]) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (next, previous, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let at = state
        .selected
        .and_then(|s| visible.iter().position(|&i| i == s));
    if next {
        state.selected = Some(visible[at.map_or(0, |a| (a + 1).min(visible.len() - 1))]);
    } else if previous {
        state.selected = Some(visible[at.map_or(0, |a| a.saturating_sub(1))]);
    } else if escape {
        state.selected = None;
    }
}

/// What a game card's right-click menu offers.
fn game_menu(ui: &mut egui::Ui, sha1: &str, index: usize, actions: &mut Actions) {
    if ui.button("Set cover art…").clicked() {
        actions.pick_cover = Some(sha1.to_string());
    }
    if ui.button("Copy SHA-1").clicked() {
        ui.ctx().copy_text(sha1.to_string());
    }
    ui.separator();
    if ui
        .button(RichText::new("Remove from Library").color(theme::ERROR))
        .on_hover_text("The game file itself stays where it is")
        .clicked()
    {
        actions.remove = Some(index);
    }
}

fn shelves(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    covers: &mut Covers,
    visible: &[usize],
    actions: &mut Actions,
) {
    let width = ui.available_width();
    let time = ui.input(|i| i.time) as f32;

    // Every cover has the same height; its width follows the box's proportions, like real boxes
    // standing on a shelf. Rows fill up left to right.
    let mut rows: Vec<Vec<(usize, Option<egui::TextureHandle>, f32)>> = vec![Vec::new()];
    let mut row_w = SHELF_PAD;
    for &index in visible {
        let entry = &state.library.entries()[index];
        let texture = covers.get(ui.ctx(), &entry.sha1);
        let card_w = (COVER_H * covers::aspect(entry, texture.as_ref())).round();
        let current = rows.last_mut().unwrap();
        if !current.is_empty() && row_w + card_w > width - SHELF_PAD {
            rows.push(Vec::new());
            row_w = SHELF_PAD;
        }
        rows.last_mut().unwrap().push((index, texture, card_w));
        row_w += card_w + CARD_GAP;
    }

    for row in rows {
        let (row_rect, _) = ui.allocate_exact_size(
            vec2(width, COVER_H + TITLE_H + PLANK_H + 18.0),
            egui::Sense::hover(),
        );
        // The shelf plank the covers stand on.
        let plank = egui::Rect::from_min_size(
            egui::pos2(row_rect.left(), row_rect.top() + COVER_H + 6.0),
            vec2(width, PLANK_H),
        );
        theme::vertical_gradient(
            ui.painter(),
            plank,
            Color32::from_rgb(0x4A, 0x36, 0x24),
            Color32::from_rgb(0x22, 0x19, 0x11),
        );
        ui.painter().line_segment(
            [plank.left_top(), plank.right_top()],
            Stroke::new(1.0, theme::BRASS_DIM),
        );

        let mut x = row_rect.left() + SHELF_PAD;
        for (index, texture, card_w) in row {
            let card = egui::Rect::from_min_size(
                egui::pos2(x, row_rect.top()),
                vec2(card_w, COVER_H + TITLE_H + PLANK_H),
            );
            x += card_w + CARD_GAP;
            let response = ui.interact(card, ui.id().with(("card", index)), egui::Sense::click());
            let entry = &state.library.entries()[index];
            let selected = state.selected == Some(index);
            let lift = if response.hovered() { 4.0 } else { 0.0 };
            let cover = egui::Rect::from_min_size(
                egui::pos2(card.left(), row_rect.top() + 6.0 - lift),
                vec2(card_w, COVER_H),
            );

            let painter = ui.painter();
            painter.rect_filled(
                cover.translate(vec2(3.0, 4.0 + lift)),
                2.0,
                Color32::from_black_alpha(140),
            );
            let spin = if response.hovered() { time * 0.4 } else { 0.0 };
            covers::paint(
                painter,
                cover,
                entry,
                &state.title(entry),
                texture.as_ref(),
                spin,
            );
            let frame = if selected {
                theme::BRASS_BRIGHT
            } else if response.hovered() {
                theme::BRASS
            } else {
                theme::BRASS_DIM
            };
            theme::riveted_frame(painter, cover, frame);

            let title_rect = egui::Rect::from_min_size(
                egui::pos2(card.left(), plank.bottom() + 4.0),
                vec2(card_w, TITLE_H),
            );
            painter.text(
                title_rect.center_top(),
                egui::Align2::CENTER_TOP,
                truncate(&state.title(entry), (card_w / 7.0) as usize),
                theme::strong(13.0),
                if selected {
                    theme::BRASS_BRIGHT
                } else {
                    theme::PARCHMENT
                },
            );
            painter.text(
                title_rect.center_top() + vec2(0.0, 17.0),
                egui::Align2::CENTER_TOP,
                entry.info.platform.to_uppercase(),
                egui::FontId::proportional(11.0),
                theme::PARCHMENT_DIM,
            );

            let title = state.title(entry);
            let response = response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(title);
            if response.clicked() {
                state.selected = Some(index);
            }
            let sha1 = entry.sha1.clone();
            response.context_menu(|ui| game_menu(ui, &sha1, index, actions));
        }
        ui.add_space(6.0);
    }
}

/// The Library as a table: one row per game, with what is known about it.
fn game_list(ui: &mut egui::Ui, state: &mut EditorState, visible: &[usize], actions: &mut Actions) {
    const COLUMNS: [(&str, f32); 5] = [
        ("Title", 0.0),
        ("Platform", 90.0),
        ("Code", 80.0),
        ("Size", 80.0),
        ("Plugins", 70.0),
    ];
    let width = ui.available_width();
    let title_w = width - COLUMNS[1..].iter().map(|c| c.1).sum::<f32>() - 16.0;
    let widths: Vec<f32> = std::iter::once(title_w)
        .chain(COLUMNS[1..].iter().map(|c| c.1))
        .collect();
    let row_h = 30.0;

    let cells = |ui: &egui::Ui, rect: egui::Rect, texts: [(String, Color32); 5]| {
        let mut x = rect.left() + 12.0;
        for ((text, color), w) in texts.into_iter().zip(&widths) {
            ui.painter().text(
                egui::pos2(x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                truncate(&text, (*w / 7.5) as usize),
                egui::FontId::proportional(13.0),
                color,
            );
            x += w;
        }
    };

    let (header, _) = ui.allocate_exact_size(vec2(width, 26.0), egui::Sense::hover());
    let mut x = header.left() + 12.0;
    for ((name, _), w) in COLUMNS.iter().zip(&widths) {
        ui.painter().text(
            egui::pos2(x, header.center().y),
            egui::Align2::LEFT_CENTER,
            name.to_uppercase(),
            theme::strong(10.5),
            theme::FAINT,
        );
        x += w;
    }
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, theme::BORDER),
    );

    for (row, &index) in visible.iter().enumerate() {
        let entry = state.library.entries()[index].clone();
        let (rect, response) = ui.allocate_exact_size(vec2(width, row_h), egui::Sense::click());
        let selected = state.selected == Some(index);
        let fill = if selected {
            theme::SELECTED
        } else if response.hovered() {
            theme::RAISED
        } else if row % 2 == 1 {
            Color32::from_white_alpha(4)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        let plugins = state
            .workshop
            .plugins
            .iter()
            .filter(|p| p.manifest.supports_rom(&entry.sha1))
            .count();
        cells(
            ui,
            rect,
            [
                (
                    state.title(&entry),
                    if selected {
                        theme::BRASS_BRIGHT
                    } else {
                        theme::PARCHMENT
                    },
                ),
                (entry.info.platform.to_uppercase(), theme::PARCHMENT_DIM),
                (entry.info.game_code.clone(), theme::PARCHMENT_DIM),
                (
                    format!("{:.1} MiB", entry.size as f64 / (1024.0 * 1024.0)),
                    theme::PARCHMENT_DIM,
                ),
                (
                    if plugins == 0 {
                        "—".to_string()
                    } else {
                        plugins.to_string()
                    },
                    theme::PARCHMENT_DIM,
                ),
            ],
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            state.selected = Some(index);
        }
        response.context_menu(|ui| game_menu(ui, &entry.sha1, index, actions));
    }
}

fn empty_shelf(ui: &mut egui::Ui, actions: &mut Actions) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(120.0, 120.0), egui::Sense::hover());
        let t = ui.input(|i| i.time) as f32;
        theme::gear(
            ui.painter(),
            rect.center(),
            56.0,
            14,
            t * 0.1,
            theme::RAISED,
            theme::BG_DEEP,
        );
        ui.add_space(14.0);
        ui.label(
            RichText::new("Your shelves are empty")
                .font(theme::display(24.0))
                .color(theme::PARCHMENT),
        );
        ui.label(kit::dim(
            "Drop game images or executables anywhere on this window, or import them.",
        ));
        ui.add_space(12.0);
        if kit::primary_button(ui, true, "+  Import games…").clicked() {
            actions.import_roms = true;
        }
        ui.add_space(10.0);
        ui.label(kit::hint(
            "GBA · N64 · SNES · Nintendo DS · PS1 · PS2 · PC · ZIP archives",
        ));
    });
}

fn details_panel(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    covers: &mut Covers,
    actions: &mut Actions,
) {
    let Some(index) = state.selected else { return };
    let entry = state.library.entries()[index].clone();
    let texture = covers.get(ui.ctx(), &entry.sha1);

    let mut close = false;
    kit::panel_header(ui, "Details", |ui| {
        close = kit::icon_button(ui, "✕", "Close  ·  Esc").clicked();
    });
    if close {
        state.selected = None;
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            kit::body_frame().show(ui, |ui| {
                // Fit the cover in a box as wide as the panel and at most 280 px tall.
                let aspect = covers::aspect(&entry, texture.as_ref());
                let w = ui.available_width().min(280.0 * aspect);
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(w, w / aspect), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect.translate(vec2(4.0, 5.0)),
                        2.0,
                        Color32::from_black_alpha(140),
                    );
                    covers::paint(
                        ui.painter(),
                        rect,
                        &entry,
                        &state.title(&entry),
                        texture.as_ref(),
                        0.0,
                    );
                    theme::riveted_frame(ui.painter(), rect, theme::BRASS);
                    ui.add_space(12.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(state.title(&entry))
                                .font(theme::display(20.0))
                                .color(theme::PARCHMENT),
                        )
                        .wrap(),
                    );
                    ui.horizontal(|ui| {
                        // Centre the badges under the title.
                        let w = 120.0;
                        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                        kit::pill(ui, &entry.info.platform.to_uppercase(), theme::BRASS);
                        if !entry.info.game_code.is_empty() {
                            kit::pill(ui, &entry.info.game_code, theme::PARCHMENT_DIM);
                        }
                    });
                });
                ui.add_space(8.0);

                ui.horizontal_wrapped(|ui| {
                    if ui.button("🖼  Set cover…").clicked() {
                        actions.pick_cover = Some(entry.sha1.clone());
                    }
                    if texture.is_some()
                        && ui
                            .button("Reset cover")
                            .on_hover_text("Go back to the box art found online")
                            .clicked()
                    {
                        actions.reset_cover = Some(entry.sha1.clone());
                    }
                    if texture.is_none()
                        && clockwork_library::boxart::system_name(entry.info.platform).is_some()
                        && ui
                            .add_enabled(!state.boxart_busy, egui::Button::new("Find box art"))
                            .clicked()
                    {
                        state.boxart_retry.push(entry.sha1.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("⋯", |ui| game_menu(ui, &entry.sha1, index, actions))
                            .response
                            .on_hover_text("More");
                    });
                });

                kit::section(ui, "File");
                egui::Grid::new("rom_info")
                    .num_columns(2)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        info_row(
                            ui,
                            "Size",
                            &format!("{:.1} MiB", entry.size as f64 / (1024.0 * 1024.0)),
                        );
                        ui.label(kit::dim("SHA-1"));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{}…",
                                    &entry.sha1[..12.min(entry.sha1.len())]
                                ))
                                .monospace(),
                            )
                            .on_hover_text(&entry.sha1);
                            if kit::icon_button(ui, "📋", "Copy SHA-1").clicked() {
                                ui.ctx().copy_text(entry.sha1.clone());
                                state.log(LogLevel::Info, "SHA-1 copied");
                            }
                        });
                        ui.end_row();
                        ui.label(kit::dim("Location"));
                        ui.add(
                            egui::Label::new(
                                RichText::new(entry.path.display().to_string()).small(),
                            )
                            .wrap(),
                        );
                        ui.end_row();
                    });

                kit::section(ui, "Plugins");
                let plugins: Vec<String> = state
                    .workshop
                    .plugins
                    .iter()
                    .filter(|p| p.manifest.supports_rom(&entry.sha1))
                    .map(|p| p.manifest.plugin.name.clone())
                    .collect();
                if plugins.is_empty() {
                    ui.label(kit::dim("Nothing extracted from this game yet."));
                }
                for name in plugins {
                    let response = ui
                        .add(
                            egui::Button::new(format!("◆  {name}"))
                                .frame_when_inactive(false)
                                .min_size(vec2(ui.available_width(), 24.0)),
                        )
                        .on_hover_text("Open in the Plugins screen");
                    if response.clicked() {
                        actions.commands.push(AppCommand::OpenPlugin(name));
                    }
                }

                kit::section(ui, "Decompilation");
                decomp_section(ui, state, &entry.sha1);

                kit::section(ui, "Extract");
                extract_section(ui, state, &entry);
            });
        });
}

/// Describe what to take from a game and hand it to the agent.
fn extract_section(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    entry: &clockwork_library::LibraryEntry,
) {
    ui.label(kit::dim(format!(
        "Tell the agent what to take from {}.",
        state.title(entry)
    )));
    ui.add(
        egui::TextEdit::multiline(&mut state.agent_prompt)
            .hint_text(kit::hint("e.g. Extract the hero as a playable character"))
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    );
    let agent = state.agents.first().map(agent_name);
    let prompt = format!(
        "Using the clockwork tools, extract from {} (SHA-1 {}): {}. Save the result as a \
         Clockwork plugin.",
        state.title(entry),
        entry.sha1,
        state.agent_prompt.trim()
    );
    let ready = !state.agent_prompt.trim().is_empty();
    if state.claude_connected {
        ui.horizontal(|ui| {
            if kit::primary_button(ui, ready, "Extract with Claude")
                .on_hover_text("Opens a new Claude Code session with this prompt; press Send there")
                .on_disabled_hover_text("Describe what to extract")
                .clicked()
            {
                match crate::claude::open_session(&prompt) {
                    Ok(()) => state.log(
                        LogLevel::Info,
                        "Opened Claude Code Desktop: press Send to start",
                    ),
                    Err(e) => state.log(LogLevel::Error, e),
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("Copy prompt"))
                .clicked()
            {
                ui.ctx().copy_text(prompt.clone());
                state.log(LogLevel::Info, "Prompt copied");
            }
        });
        return;
    }
    ui.horizontal(|ui| {
        if kit::primary_button(ui, ready && agent.is_some(), "Copy prompt for agent")
            .on_hover_text("Copies a prompt to paste into your agent")
            .on_disabled_hover_text("Connect an agent and describe what to extract")
            .clicked()
        {
            ui.ctx().copy_text(prompt.clone());
            state.log(
                LogLevel::Info,
                "Prompt copied: paste it into your agent's chat",
            );
        }
    });
    ui.horizontal(|ui| {
        kit::status_dot(
            ui,
            if agent.is_some() {
                theme::SUCCESS
            } else {
                theme::FAINT
            },
        );
        ui.label(kit::hint(match &agent {
            Some(name) => format!("Agent: {name}"),
            None => "No agent connected (start clockwork-mcp from your MCP client)".into(),
        }));
    });
    ui.label(kit::hint(
        "Connect Claude Code Desktop in Settings to start from here.",
    ));
}

/// "ZCode 1.2 (http 127.0.0.1:3128)" or "server running (stdio), no client yet".
fn agent_name(status: &clockwork_library::agents::AgentStatus) -> String {
    match &status.client {
        Some(client) => format!("{client} ({})", status.transport),
        None => format!("server running ({}), no client yet", status.transport),
    }
}

/// Status-bar indicator: a green, brass (working) or grey dot and the connected agent.
fn agent_badge(ui: &mut egui::Ui, state: &EditorState) {
    let (color, text, tip) = match state.agents.first() {
        Some(a) => {
            let busy = a
                .last_tool_at
                .is_some_and(|t| clockwork_library::agents::now().saturating_sub(t) < 5);
            let tip = format!(
                "{} tool calls{}",
                a.tool_calls,
                a.last_tool
                    .as_ref()
                    .map(|t| format!(", last: {t}"))
                    .unwrap_or_default()
            );
            let color = if busy {
                theme::BRASS_BRIGHT
            } else {
                theme::SUCCESS
            };
            (color, agent_name(a), tip)
        }
        None => (
            theme::FAINT,
            "No agent".to_string(),
            "Start clockwork-mcp from your MCP client".to_string(),
        ),
    };
    ui.label(RichText::new(text).small().color(theme::PARCHMENT_DIM))
        .on_hover_text(&tip);
    kit::status_dot(ui, color).on_hover_text(tip);
}

/// The game's linked decompilation, or a way to link one.
fn decomp_section(ui: &mut egui::Ui, state: &mut EditorState, sha1: &str) {
    if let Some(url) = state.decomps.get(sha1).cloned() {
        ui.horizontal(|ui| {
            kit::status_dot(ui, theme::SUCCESS);
            ui.hyperlink_to(decomps::short_name(&url), &url)
                .on_hover_text(&url);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Unlink").clicked() {
                    state.decomps.remove(sha1);
                    state.save();
                }
            });
        });
        ui.label(kit::hint(
            "Extraction can use this project's names for code and data.",
        ));
        return;
    }

    ui.label(kit::hint(
        "Link the game's decompilation repository, if it has one. Its names for code and data \
         make extraction much easier.",
    ));
    if let Some(known) = decomps::suggest(sha1) {
        egui::Frame::new()
            .fill(theme::RAISED)
            .corner_radius(theme::RADIUS)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Known: {}", decomps::short_name(&known.repository)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("Link it")
                            .on_hover_text(&known.repository)
                            .clicked()
                        {
                            state
                                .decomps
                                .insert(sha1.to_string(), known.repository.clone());
                            state.save();
                            state.log(
                                LogLevel::Info,
                                format!("Linked {} to {}", known.name, known.repository),
                            );
                        }
                    });
                });
            });
    }
    let mut link = false;
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.decomp_input)
                .hint_text(kit::hint("https://github.com/owner/repo"))
                .desired_width(ui.available_width() - 52.0),
        );
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        link = ui.button("Link").clicked() || enter;
    });
    if link && !state.decomp_input.trim().is_empty() {
        match decomps::normalize(&state.decomp_input) {
            Ok(url) => {
                state.decomps.insert(sha1.to_string(), url.clone());
                state.decomp_input.clear();
                state.save();
                state.log(LogLevel::Info, format!("Linked {url}"));
            }
            Err(e) => state.log(LogLevel::Warn, e),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Status bar

/// The bar along the bottom of every screen: what is happening, the latest message, problems,
/// the agent and totals.
fn status_bar(
    root: &mut egui::Ui,
    state: &EditorState,
    extractions: &Extractions,
    actions: &mut Actions,
) {
    egui::Panel::bottom("status")
        .exact_size(STATUS_H)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(theme::HEADER))
        .show(root, |ui| {
            let rect = ui.max_rect();
            ui.painter().line_segment(
                [rect.left_top(), rect.right_top()],
                Stroke::new(1.0, theme::BORDER),
            );
            ui.horizontal_centered(|ui| {
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                let busy = if state.boxart_busy {
                    Some("Fetching box art…")
                } else if extractions.busy() {
                    Some("Extracting…")
                } else {
                    None
                };
                if let Some(busy) = busy {
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                    let t = ui.input(|i| i.time) as f32;
                    theme::gear(
                        ui.painter(),
                        rect.center(),
                        6.5,
                        8,
                        t * 2.0,
                        theme::BRASS,
                        theme::HEADER,
                    );
                    ui.label(RichText::new(busy).small().color(theme::BRASS));
                    ui.ctx().request_repaint();
                }

                let errors = state
                    .log
                    .iter()
                    .filter(|l| l.level == LogLevel::Error)
                    .count();
                let warnings = state
                    .log
                    .iter()
                    .filter(|l| l.level == LogLevel::Warn)
                    .count();
                let mut show_log = false;
                if errors > 0 {
                    show_log |= kit::pill(ui, &format!("⛔ {errors}"), theme::ERROR)
                        .interact(egui::Sense::click())
                        .on_hover_text("Errors: show the log")
                        .clicked();
                }
                if warnings > 0 {
                    show_log |= kit::pill(ui, &format!("⚠ {warnings}"), theme::WARN)
                        .interact(egui::Sense::click())
                        .on_hover_text("Warnings: show the log")
                        .clicked();
                }
                if let Some(line) = state.log.last() {
                    show_log |= ui
                        .add(
                            egui::Label::new(
                                RichText::new(&line.text)
                                    .small()
                                    .color(log_color(ui, line.level)),
                            )
                            .truncate()
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Show the log")
                        .clicked();
                }
                if show_log {
                    actions.commands.push(AppCommand::ShowLog);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(kit::hint(format!("v{}", env!("CARGO_PKG_VERSION"))));
                    ui.label(kit::hint(format!(
                        "{} games · {} plugins",
                        state.library.entries().len(),
                        state.workshop.plugins.len()
                    )));
                    ui.add_space(4.0);
                    agent_badge(ui, state);
                });
            });
        });
}

// ---------------------------------------------------------------------------------------------
// Helpers

pub(crate) fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(kit::dim(key));
    ui.label(value);
    ui.end_row();
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max.max(2) - 1).collect::<String>())
    }
}

pub(crate) fn log_color(ui: &egui::Ui, level: LogLevel) -> Color32 {
    match level {
        LogLevel::Info => ui.visuals().text_color(),
        LogLevel::Warn => ui.visuals().warn_fg_color,
        LogLevel::Error => ui.visuals().error_fg_color,
    }
}

fn show_drop_overlay(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("rom_drop_target"),
    ));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(200));
    theme::gear(
        &painter,
        rect.center() - vec2(0.0, 50.0),
        40.0,
        12,
        0.0,
        theme::BRASS_DIM,
        Color32::from_black_alpha(200),
    );
    painter.text(
        rect.center() + vec2(0.0, 20.0),
        egui::Align2::CENTER_CENTER,
        "Drop games to add them to the Library",
        theme::display(26.0),
        theme::BRASS_BRIGHT,
    );
}

/// Asks before quitting while plugin edits aren't saved.
fn quit_dialog(ctx: &egui::Context, state: &mut EditorState, exit: &mut MessageWriter<AppExit>) {
    if !state.quit_asked {
        return;
    }
    let unsaved = state.plugin_editor.unsaved();
    let response = egui::Modal::new(egui::Id::new("quit_unsaved")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(
            RichText::new("Quit without saving?")
                .font(theme::display(18.0))
                .color(theme::BRASS_BRIGHT),
        );
        ui.label(format!(
            "These plugins have changes that aren't saved: {}.",
            unsaved.join(", ")
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Quit without saving").clicked() {
                exit.write(AppExit::Success);
            }
            if ui.button("Cancel").clicked() {
                state.quit_asked = false;
            }
            if ui.button("Show them").clicked() {
                state.quit_asked = false;
                state.screen = Screen::Plugins;
                state.plugin_editor.selected = unsaved.first().cloned();
            }
        });
    });
    if response.should_close() {
        state.quit_asked = false;
    }
}

/// Closing the window asks first while plugin edits aren't saved.
pub fn close_system(
    mut requests: MessageReader<bevy::window::WindowCloseRequested>,
    mut state: ResMut<EditorState>,
    mut exit: MessageWriter<AppExit>,
) {
    for _ in requests.read() {
        if state.plugin_editor.unsaved().is_empty() {
            exit.write(AppExit::Success);
        } else {
            state.quit_asked = true;
        }
    }
}

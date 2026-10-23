//! Remote control for testing and automation, without touching the mouse or keyboard.
//!
//! When `CLOCKWORK_CONTROL` names a folder, the editor runs every `*.ron` file put there (oldest
//! name first): a list of [`Command`]s, e.g. `[Screen(Workshop), Frame("Hero"), Wait(10),
//! Screenshot("shot.png")]`. Each file is deleted once read; `<name>.done` is written when its
//! commands have run (with any errors), so scripts can wait for it. Files are read as soon as
//! they appear: write one under another extension and rename it to `.ron` once complete.

use std::path::{Path, PathBuf};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use serde::Deserialize;

use clockwork_core::behavior::NodeBehavior;

use crate::{
    play::ScriptedInput,
    scene::{NodeId, ROOT},
    state::{EditorState, Screen},
    workshop::{Extractions, OrbitCamera},
};

#[derive(Debug, Clone, Deserialize)]
pub enum Command {
    Screen(ScreenName),
    /// Sets the viewport camera: what it looks at, and from where (radians, meters).
    Camera {
        focus: (f32, f32, f32),
        yaw: f32,
        pitch: f32,
        distance: f32,
    },
    /// Points the camera at a node (by name or id), from `distance` meters (default 4).
    Frame(String, #[serde(default)] Option<f32>),
    /// Selects a node in the scene tree (by name or id).
    Select(String),
    /// Sets the animation a node plays.
    Animation(String, String),
    /// Places a plugin output at the camera's focus.
    Place(String, String),
    Undo,
    Redo,
    /// Starts Play mode (the selected character, or the first one).
    Play,
    Stop,
    /// Holds a stick direction for `frames` frames while playing: x right, y forward, relative
    /// to the camera, each -1..1.
    Stick {
        x: f32,
        y: f32,
        #[serde(default)]
        walk: bool,
        frames: u32,
    },
    /// Presses jump once while playing (held for 20 frames).
    Jump,
    /// Presses the dodge button once while playing a 2D character.
    #[serde(alias = "Roll")]
    Dodge,
    /// Holds the B (or A) item button for this many frames, pressing it on the first.
    ItemB(u32),
    ItemA(u32),
    /// Puts an item on a button of the played 2D character.
    Equip(clockwork_core::item::Slot, String),
    /// Opens or closes the inventory.
    Inventory,
    /// Opens or closes the played character's menu of a kind: "Inventory" or "Pause".
    Menu(String),
    /// Presses a game button ("A", "Menu", "Down", ...) as a player would.
    Press(String),
    /// Copies a settings asset ("plugin/output" of a kind) into the user's plugin, my-game.
    CopyAsset(String, String),
    /// Sets a field of an asset ("plugin/output", kind, field, RON value) in its plugin's recipe.
    SetField(String, String, String, String),
    /// Opens the command palette with this text typed in.
    Palette(String),
    /// Opens the plugin editor on a plugin (by name).
    PluginEditor(String),
    /// Changes, finds in, saves, undoes … the graph of the plugin shown:
    /// `Graph(Recipe(Value("step 0", "offset", "0x40")))`, `Graph(Find("Lookup"))`, `Graph(Save)`.
    Graph(crate::plugin_editor::GraphCommand),
    /// Adds a behavior node (Target, Pickup, Trigger, Checkpoint, Solid) where the camera looks.
    AddEntity(String),
    /// Puts a node at a position, in meters relative to its parent.
    Move(String, (f32, f32, f32)),
    /// Gives a node a behavior, written as RON, instead of those it has:
    /// `SetBehavior("Crate", Solid(radius: 1.0))`.
    SetBehavior(String, crate::entities::Behavior),
    /// Adds a behavior to those a node has.
    AddBehavior(String, crate::entities::Behavior),
    /// Makes a node's behavior (by number, from 0) work only while a condition holds (`None`:
    /// always): `SetWhen("Door", 0, Some(Has(item: "key", count: 1)))`.
    SetWhen(String, usize, Option<crate::entities::Condition>),
    /// Makes a node appear while playing only once a condition holds (`None`: from the start).
    Appears(String, Option<crate::entities::Condition>),
    /// Limits a node to these acts (numbered from 1); None means every act.
    Acts(String, Option<Vec<u8>>),
    /// Chooses Play's current act; None opens every node's act restriction.
    Act(Option<u8>),
    /// Selects an asset in the content browser, `"plugin/output"`, so the Inspector shows it.
    Browse(String),
    /// Gives a node a camera asset to use when played (`None`: its character's own).
    NodeCamera(String, Option<String>),
    /// Gives a node another asset for a setting ("Camera", "HUD", "UI", "Input", "Stats",
    /// "Items") when played.
    NodeSetting(String, String, Option<String>),
    /// Makes a node's behavior follow a behavior asset (`None`: its own again).
    LinkBehavior(String, Option<String>),
    /// Places a node following a behavior asset where the camera looks.
    PlaceBehavior(String),
    /// Sets one of the game's settings ("Preset", "Rules", "Camera", "HUD", ...).
    GameSetting(String, Option<String>),
    /// Adds a node to the selection.
    SelectAdd(String),
    /// Duplicates the selection, or groups it.
    Duplicate,
    Group,
    /// Selects what is under this point of the window (pixels), as a click would.
    PickAt(f32, f32),
    /// Chooses the gizmo: "Move", "Rotate" or "Scale".
    ToolMode(String),
    /// Pause, resume, one step and speed of Play mode.
    Pause,
    Resume,
    Step,
    Speed(f32),
    /// Turns on the debug drawing: behavior rings and collision triangles.
    Debug(bool, bool),
    /// Exports the scene as a game into a folder, with a title.
    Export(PathBuf, String),
    /// Adds the played character's state to this file's `.done` report.
    Report,
    /// Lets this many frames render before the next command.
    Wait(u32),
    /// Saves the window, panels included, as a PNG.
    Screenshot(PathBuf),
    /// Shows the viewport alone in the whole window, as Play's game view does (or not).
    GameView(bool),
    /// Renders a folder's views and compares them with the approved pictures (see
    /// [`crate::fidelity`]); this file's report lists the views that changed.
    Fidelity(PathBuf),
    /// Compares the rendered views with the approved ones (what `Fidelity` ends with).
    FidelityReport(PathBuf),
    /// Makes a folder's rendered views (or the one named) the approved pictures.
    Approve(PathBuf, Option<String>),
    Quit,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum ScreenName {
    Library,
    Workshop,
    Plugins,
    Settings,
}

/// Commands waiting to run, and the file they came from.
#[derive(Resource, Default)]
pub struct Control {
    dir: Option<PathBuf>,
    queue: Vec<(String, Command)>,
    waiting: u32,
    errors: Vec<String>,
}

impl Control {
    pub fn from_env() -> Self {
        let dir = std::env::var_os("CLOCKWORK_CONTROL").map(PathBuf::from);
        if let Some(dir) = &dir {
            let _ = std::fs::create_dir_all(dir);
        }
        Self { dir, ..default() }
    }
}

/// Reads new command files and runs queued commands, one per frame (or fewer while waiting, and
/// none while recipes run in the background, so screenshots show finished extractions).
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn control_system(
    mut control: ResMut<Control>,
    mut state: ResMut<EditorState>,
    mut history: ResMut<crate::scene::History>,
    mut orbit: ResMut<OrbitCamera>,
    mut scripted: ResMut<ScriptedInput>,
    mut browser: ResMut<crate::browser::Browser>,
    mut session: ResMut<crate::play::PlaySession>,
    extractions: Res<Extractions>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = control.dir.clone() else {
        return;
    };
    if control.queue.is_empty() {
        read_files(&mut control, &dir);
    }
    if control.waiting > 0 {
        control.waiting -= 1;
        return;
    }
    if extractions.busy() {
        return;
    }
    if control.queue.is_empty() {
        return;
    }
    let (file, command) = control.queue.remove(0);
    // A rendering check runs as the commands of its views, in place of this one.
    if let Command::Fidelity(folder) = &command {
        match crate::fidelity::commands(folder) {
            Ok(list) => {
                for (i, command) in list.into_iter().enumerate() {
                    control.queue.insert(i, (file.clone(), command));
                }
            }
            Err(e) => control.errors.push(format!("Fidelity: {e}")),
        }
    }
    if matches!(command, Command::Report) {
        let selection: Vec<String> = state
            .workshop
            .selection()
            .iter()
            .filter_map(|id| state.workshop.scene.get(*id).map(|n| n.name.clone()))
            .collect();
        let live = state.workshop.live.join(" ; ");
        let toast = state.workshop.toast.as_ref().map(|t| t.0.clone());
        let line = session
            .describe()
            .map(|d| format!("{d} || live: {live} || toast {toast:?}"))
            .unwrap_or_else(|| {
                format!(
                    "not playing | nodes {} | selected {:?} | toast {:?}",
                    state.workshop.scene.nodes.len(),
                    selection,
                    state.workshop.toast.as_ref().map(|t| &t.0)
                )
            });
        let line = match (state.screen, crate::plugin_editor::describe(&state)) {
            (Screen::Plugins, Some(plugin)) => format!("{line} | {plugin}"),
            _ => line,
        };
        control.errors.push(format!("report: {line}"));
    }
    if let Err(e) = run(
        &command,
        &mut state,
        &mut history,
        &mut orbit,
        &mut scripted,
        &mut browser,
        &mut session,
        &mut control,
        &mut commands,
        &dir,
    ) {
        control.errors.push(format!("{command:?}: {e}"));
    }
    if matches!(command, Command::Quit) {
        exit.write(AppExit::Success);
    }
    // Last command of its file: report.
    if !control.queue.iter().any(|(f, _)| *f == file) {
        let report = if control.errors.is_empty() {
            "ok\n".to_string()
        } else {
            control.errors.join("\n") + "\n"
        };
        control.errors.clear();
        let _ = std::fs::write(dir.join(format!("{file}.done")), report);
    }
}

fn read_files(control: &mut Control, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .collect();
    files.sort();
    for path in files {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| ron::from_str::<Vec<Command>>(&text).map_err(|e| e.to_string()));
        let _ = std::fs::remove_file(&path);
        match parsed {
            Ok(list) if !list.is_empty() => {
                control
                    .queue
                    .extend(list.into_iter().map(|c| (name.clone(), c)));
            }
            Ok(_) => {
                let _ = std::fs::write(dir.join(format!("{name}.done")), "ok\n");
            }
            Err(e) => {
                let _ = std::fs::write(dir.join(format!("{name}.done")), format!("{e}\n"));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // Commands reach into most of the editor.
fn run(
    command: &Command,
    state: &mut EditorState,
    history: &mut crate::scene::History,
    orbit: &mut OrbitCamera,
    scripted: &mut ScriptedInput,
    browser: &mut crate::browser::Browser,
    session: &mut crate::play::PlaySession,
    control: &mut Control,
    commands: &mut Commands,
    dir: &Path,
) -> Result<(), String> {
    let asset = |text: &str| {
        clockwork_core::asset::OutputRef::parse(text)
            .ok_or_else(|| format!("\"{text}\" is not \"plugin/output\""))
    };
    match command {
        Command::Browse(at) => {
            state.screen = Screen::Workshop;
            crate::asset_ui::select(state, browser, asset(at)?);
        }
        Command::LinkBehavior(node, at) => {
            let id = find(state, node)?;
            let at = at.as_deref().map(asset).transpose()?;
            crate::asset_ui::link_behavior(state, id, 0, at);
        }
        Command::PlaceBehavior(at) => crate::asset_ui::place_behavior(state, asset(at)?),
        Command::GameSetting(setting, value) => {
            let pick = crate::asset_ui::GAME_PICKS
                .iter()
                .find(|p| p.label == setting)
                .ok_or_else(|| format!("no game setting \"{setting}\""))?;
            let value = value.as_deref().map(asset).transpose()?;
            (pick.set)(&mut state.workshop.scene.game, value);
            state.workshop.scene.dirty = true;
        }
        Command::NodeSetting(node, setting, value) => {
            let id = find(state, node)?;
            let pick = crate::asset_ui::NODE_PICKS
                .iter()
                .find(|p| p.label == setting)
                .ok_or_else(|| format!("no setting \"{setting}\""))?;
            let value = value.as_deref().map(asset).transpose()?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                (pick.set)(n, value);
                state.workshop.scene.dirty = true;
            }
        }
        Command::NodeCamera(node, camera) => {
            let id = find(state, node)?;
            let camera = camera.as_deref().map(asset).transpose()?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.camera = camera;
                state.workshop.scene.dirty = true;
            }
        }
        Command::Screen(screen) => {
            state.screen = match screen {
                ScreenName::Library => Screen::Library,
                ScreenName::Workshop => Screen::Workshop,
                ScreenName::Plugins => Screen::Plugins,
                ScreenName::Settings => Screen::Settings,
            };
        }
        Command::Camera {
            focus,
            yaw,
            pitch,
            distance,
        } => {
            *orbit = OrbitCamera {
                focus: Vec3::new(focus.0, focus.1, focus.2),
                yaw: *yaw,
                pitch: *pitch,
                distance: *distance,
            };
        }
        Command::Frame(node, distance) => {
            let id = find(state, node)?;
            orbit.focus = state.workshop.scene.world_transform(id).translation;
            orbit.distance = distance.unwrap_or(4.0);
        }
        Command::Select(node) => {
            state.workshop.selected_node = Some(find(state, node)?);
        }
        Command::Animation(node, name) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.animation = Some(name.clone());
                state.workshop.scene.dirty = true;
            }
        }
        Command::Place(plugin, output) => state.workshop.place(plugin, output, ROOT, false),
        Command::Undo => {
            history.undo(&mut state.workshop.scene);
        }
        Command::Redo => {
            history.redo(&mut state.workshop.scene);
        }
        Command::Play => state.workshop.play_request = Some(true),
        Command::Stop => state.workshop.play_request = Some(false),
        Command::Stick { x, y, walk, frames } => {
            scripted.stick = Vec2::new(*x, *y).clamp_length_max(1.0);
            scripted.walk = *walk;
            scripted.frames = *frames;
        }
        Command::Jump => {
            scripted.jump = true;
            scripted.jump_hold = 20;
            scripted.frames = scripted.frames.max(1);
        }
        Command::Dodge => scripted.dodge_press = true,
        Command::ItemB(frames) => {
            scripted.b_press = true;
            scripted.b_hold = (*frames).max(1);
        }
        Command::ItemA(frames) => {
            scripted.a_press = true;
            scripted.a_hold = (*frames).max(1);
        }
        Command::Equip(slot, item) => {
            state
                .workshop
                .hud_requests
                .push(clockwork_runtime::hud::HudRequest::Equip(
                    *slot,
                    item.clone(),
                ));
        }
        Command::PluginEditor(name) => {
            state.screen = Screen::Plugins;
            state.plugin_editor.selected = Some(name.clone());
        }
        Command::Palette(query) => state.palette_request = Some(query.clone()),
        Command::Inventory => state.workshop.menu_toggle = Some("Inventory".into()),
        Command::Menu(kind) => state.workshop.menu_toggle = Some(kind.clone()),
        Command::Press(button) => state.workshop.presses.push(button.clone()),
        Command::CopyAsset(at, kind) => {
            let kind = kind_name(kind)?;
            let copy = crate::asset_edit::copy_to_my_game(state, &asset(at)?, kind)?;
            state.log(crate::state::LogLevel::Info, format!("Copied as {copy}"));
        }
        Command::SetField(at, kind, key, value) => {
            let kind = kind_name(kind)?;
            let mut editor = std::mem::take(&mut state.workshop.asset_editor);
            let result = editor.set_field(state, &asset(at)?, kind, key, value);
            state.workshop.asset_editor = editor;
            result?;
        }
        Command::AddEntity(kind) => {
            state.workshop.entity_request =
                Some(crate::entities::Behavior::new(kind).ok_or(format!("no behavior {kind}"))?);
        }
        Command::Move(node, (x, y, z)) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.position = [*x, *y, *z];
                state.workshop.scene.dirty = true;
            }
        }
        Command::SetBehavior(node, behavior) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.behaviors = vec![NodeBehavior::new(behavior.clone())];
                state.workshop.scene.dirty = true;
            }
        }
        Command::AddBehavior(node, behavior) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.behaviors.push(NodeBehavior::new(behavior.clone()));
                state.workshop.scene.dirty = true;
            }
        }
        Command::SetWhen(node, index, when) => {
            let id = find(state, node)?;
            let entry = state
                .workshop
                .scene
                .get_mut(id)
                .and_then(|n| n.behaviors.get_mut(*index))
                .ok_or_else(|| format!("\"{node}\" has no behavior {index}"))?;
            entry.when = when.clone();
            state.workshop.scene.dirty = true;
        }
        Command::Appears(node, when) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.appears = when.clone();
                state.workshop.scene.dirty = true;
            }
        }
        Command::Acts(node, acts) => {
            let id = find(state, node)?;
            if let Some(n) = state.workshop.scene.get_mut(id) {
                n.acts = acts.clone();
                state.workshop.scene.dirty = true;
            }
        }
        Command::Act(act) => {
            if state.workshop.playing.is_none() {
                return Err("not playing".into());
            }
            session.set_act(*act);
            state.workshop.hidden_in_play = session.entities().hidden().collect();
        }
        Command::SelectAdd(node) => {
            let id = find(state, node)?;
            state.workshop.toggle_select(id);
        }
        Command::Duplicate => crate::workshop_ui::duplicate_selection(state),
        Command::Group => crate::workshop_ui::group_selection(state),
        Command::PickAt(x, y) => state.workshop.pick_request = Some(Vec2::new(*x, *y)),
        Command::ToolMode(name) => {
            use crate::manipulate::Tool;
            state.workshop.tool = match name.as_str() {
                "Move" => Tool::Move,
                "Rotate" => Tool::Rotate,
                "Scale" => Tool::Scale,
                other => return Err(format!("no tool {other}")),
            };
        }
        Command::Pause => state.workshop.play_clock.paused = true,
        Command::Resume => state.workshop.play_clock.paused = false,
        Command::Step => {
            state.workshop.play_clock.paused = true;
            state.workshop.play_clock.step_once = true;
        }
        Command::Speed(speed) => state.workshop.play_clock.speed = *speed,
        Command::Debug(colliders, terrain) => {
            state.workshop.debug.colliders = *colliders;
            state.workshop.debug.terrain = *terrain;
        }
        Command::Export(folder, title) => {
            let plugins = crate::ui::plugins_to_export(state);
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let out = crate::export::export(&state.workshop.scene, &plugins, title, folder, &exe)?;
            control
                .errors
                .push(format!("exported to {}", out.display()));
        }
        Command::Wait(frames) => control.waiting = *frames,
        Command::Screenshot(path) => {
            let path = if path.is_relative() {
                dir.join(path)
            } else {
                path.clone()
            };
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            // The capture is saved a few frames later; don't let a Quit cut it off.
            control.waiting = control.waiting.max(30);
        }
        Command::Graph(command) => crate::plugin_editor::script(state, command)?,
        Command::GameView(on) => state.workshop.game_view = *on,
        Command::FidelityReport(folder) => {
            let failed = crate::fidelity::report(folder)?;
            if !failed.is_empty() {
                return Err(format!(
                    "{} changed or new: {} (see {})",
                    failed.len(),
                    failed.join(", "),
                    folder.join("report.md").display()
                ));
            }
        }
        Command::Approve(folder, name) => {
            crate::fidelity::approve(folder, name.as_deref())?;
        }
        Command::Quit | Command::Report | Command::Fidelity(_) => {}
    }
    Ok(())
}

/// A node by id, exact name, or the first name containing the text.
fn find(state: &EditorState, node: &str) -> Result<NodeId, String> {
    let scene = &state.workshop.scene;
    if let Ok(id) = node.parse::<NodeId>()
        && scene.get(id).is_some()
    {
        return Ok(id);
    }
    scene
        .nodes
        .iter()
        .find(|n| n.name == node)
        .or_else(|| scene.nodes.iter().find(|n| n.name.contains(node)))
        .map(|n| n.id)
        .ok_or_else(|| format!("no node \"{node}\""))
}

/// A registered asset kind by name.
fn kind_name(kind: &str) -> Result<&'static str, String> {
    clockwork_core::asset::KINDS
        .iter()
        .map(|k| k.name)
        .find(|k| *k == kind)
        .ok_or_else(|| format!("no asset kind \"{kind}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_commands_read_node_lists_and_play_choices() {
        let commands: Vec<Command> = ron::from_str(
            r#"[Acts("Star", Some([1, 7])), Acts("Star", None), Act(Some(7)), Act(None)]"#,
        )
        .unwrap();
        assert!(matches!(&commands[0], Command::Acts(node, Some(acts))
            if node == "Star" && acts == &[1, 7]));
        assert!(matches!(&commands[1], Command::Acts(_, None)));
        assert!(matches!(&commands[2], Command::Act(Some(7))));
        assert!(matches!(&commands[3], Command::Act(None)));
    }
}

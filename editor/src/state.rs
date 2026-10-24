use std::collections::BTreeMap;
use std::path::PathBuf;

use bevy::prelude::*;
use clockwork_library::{Game, GameNotes, Library, LibraryEntry, boxart::display_name};
use clockwork_recipe::Plugin;

use crate::scene::{ItemRef, NodeId, Scene};
use crate::store;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub struct LogLine {
    pub level: LogLevel,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Library,
    Workshop,
    /// The plugin editor: compositions and what plugins provide.
    Plugins,
    Settings,
}

#[derive(Resource)]
pub struct EditorState {
    pub library: Library,
    /// Remembered games whose file couldn't be read this run (a drive not plugged in, a moved
    /// file). They stay in the saved Library, so they come back once they can be read again,
    /// or once the user locates their file.
    pub missing: Vec<Game>,
    /// Games being read in the background.
    pub imports: crate::imports::Imports,
    pub selected: Option<usize>,
    pub search: String,
    pub agent_prompt: String,
    pub screen: Screen,
    pub log: Vec<LogLine>,
    pub fetch_box_art: bool,
    /// No-Intro names of identified games, by SHA-1.
    pub names: BTreeMap<String, String>,
    /// Decompilation repositories linked to games, by SHA-1.
    pub decomps: BTreeMap<String, String>,
    /// When games were added and last used, favourites and tags, by SHA-1.
    pub notes: BTreeMap<String, GameNotes>,
    /// Text in the details panel's repository field.
    pub decomp_input: String,
    /// Games the user asked to look up box art for again.
    pub boxart_retry: Vec<String>,
    pub boxart_busy: bool,
    pub workshop: Workshop,
    /// Agent servers that are running (refreshed every few seconds).
    pub agents: Vec<clockwork_library::agents::AgentStatus>,
    /// Mirror of the Claude Desktop setting, so panels need not know `Settings`.
    pub claude_connected: bool,
    /// Set when this program is playing an exported game instead of being the Workshop.
    pub player: Option<crate::player::PlayerMode>,
    pub plugin_editor: crate::plugin_editor::PluginEditor,
    /// Quitting was asked for while plugin edits aren't saved: the UI asks first.
    pub quit_asked: bool,
    /// Set by the remote control: open the command palette with this text typed in.
    pub palette_request: Option<String>,
}

/// Workshop state: installed plugins and the scene being composed.
pub struct Workshop {
    pub plugins: Vec<Plugin>,
    /// The plugin shown in the Inspector (when no scene node is selected).
    pub selected: Option<usize>,
    /// Bumped whenever plugins are rescanned, so cached extractions are redone.
    pub plugins_revision: u64,
    pub scene: Scene,
    /// The scene node shown in the Inspector (the last one selected); takes precedence over
    /// `selected`.
    pub selected_node: Option<NodeId>,
    /// Every selected node, when more than one is (see [`Workshop::selection`]).
    pub selection: Vec<NodeId>,
    /// Nodes copied with Ctrl+C.
    pub clipboard: Vec<crate::scene::Node>,
    /// Set by the UI: move the camera's focus to this node.
    pub focus_request: Option<NodeId>,
    /// Set by the remote control: select what is under this window position.
    pub pick_request: Option<bevy::math::Vec2>,
    /// The title being typed in the Export dialog, while it is open.
    pub export: Option<String>,
    /// Set by the UI: put a new behavior node where the camera looks.
    pub entity_request: Option<crate::entities::Behavior>,
    /// The behavior asset the requested node's behavior comes from.
    pub entity_link: Option<clockwork_core::asset::OutputRef>,
    /// What the played character and the scene's behaviors are doing, for the live panel.
    pub live: Vec<String>,
    /// Pause, step and speed of Play mode.
    pub play_clock: PlayClock,
    /// What debug drawing is on.
    pub debug: DebugView,
    /// Nodes destroyed or picked up while playing: hidden until play stops.
    pub hidden_in_play: std::collections::HashSet<NodeId>,
    /// Nodes Play moves while it plays: actors (their scene transform waits until it stops).
    pub acting: std::collections::HashSet<NodeId>,
    /// A message for the player and the seconds it still shows.
    pub toast: Option<(String, f32)>,
    /// The viewport gizmo's mode and whether it snaps.
    pub tool: crate::manipulate::Tool,
    pub snap: bool,
    /// A gizmo handle is under the cursor or being dragged: the camera keeps still.
    pub gizmo_busy: bool,
    /// The node just placed; the adjust-after-drop panel edits it until something else happens.
    pub adjusting: Option<NodeId>,
    /// Set by the UI; the scene system places the items.
    pub place_requests: Vec<PlaceRequest>,
    /// Whether the pointer is over the 3D viewport (and not over a panel or popup).
    pub viewport_hovered: bool,
    /// Whether surface overlays are drawn.
    pub show_surfaces: bool,
    /// The surface cell under the cursor: node and cell.
    pub hovered_cell: Option<(NodeId, u32, u32)>,
    /// The character node being played, while in Play mode.
    pub playing: Option<NodeId>,
    /// Set by the UI: `true` starts Play mode, `false` stops it.
    pub play_request: Option<bool>,
    /// Play fills the window, without the editor around it, as the game would. A game with a
    /// flow (title, save files) starts so; F11 switches, Escape goes back to the editor.
    pub game_view: bool,
    /// A text field has the keyboard, so keys aren't for playing.
    pub typing: bool,
    /// What the played character's fighting looks like to the HUD (items, buttons).
    pub hud: Option<clockwork_runtime::hud::Hud>,
    /// What the HUD asked for since last frame: equipping, starting over.
    pub hud_requests: Vec<clockwork_runtime::hud::HudRequest>,
    /// The played character's screen (HUD, theme, menus) and input map, while playing.
    pub screen: Option<(
        clockwork_runtime::hud::Screen,
        clockwork_core::input::InputMap,
    )>,
    /// A menu open: the game stands still.
    pub menu_paused: bool,
    /// The played character's health and most health, while playing.
    pub health: Option<(f32, f32)>,
    /// Lives and seconds left, when the game's rules count them; how dark a door's fade has
    /// the screen (0 to 16); big text such as "You win!".
    pub lives: Option<u32>,
    pub seconds: Option<u32>,
    pub fade: u8,
    pub banner: Option<String>,
    /// The open dialog and its page; the game waits meanwhile.
    pub dialog: Option<(clockwork_core::ui::Dialog, usize)>,
    /// Sounds to play, as soon as the sound system gets to them.
    pub sounds: Vec<clockwork_core::asset::OutputRef>,
    /// The played character's sounds.
    pub kit_sounds: KitSounds,
    /// Music to play in place of what plays (none: silence), as soon as the music system
    /// gets to it.
    pub music: Option<Option<clockwork_core::asset::OutputRef>>,
    /// Set by the remote control: open or close the menu of this kind.
    pub menu_toggle: Option<String>,
    /// Set by the remote control: game buttons to press, by name.
    pub presses: Vec<String>,
    /// The asset whose fields the Inspector edits.
    pub asset_editor: crate::asset_edit::AssetEditor,
    /// The timeline of the selected character's action.
    pub timeline: crate::timeline::Timeline,
}

/// The sounds of the played character's kit, by what makes them.
#[derive(Clone, Debug, Default)]
pub struct KitSounds {
    pub hurt: Option<clockwork_core::asset::OutputRef>,
    pub pickup: Option<clockwork_core::asset::OutputRef>,
    pub door: Option<clockwork_core::asset::OutputRef>,
    pub menu: Option<clockwork_core::asset::OutputRef>,
}

/// Pause, single step and speed of Play mode.
#[derive(Clone, Copy, Debug)]
pub struct PlayClock {
    pub paused: bool,
    /// Simulation seconds per real second.
    pub speed: f32,
    /// Run one simulation step, then stay paused.
    pub step_once: bool,
}

impl Default for PlayClock {
    fn default() -> Self {
        Self {
            paused: false,
            speed: 1.0,
            step_once: false,
        }
    }
}

impl PlayClock {
    /// The simulation time to run for a frame of `frame` seconds.
    pub fn advance(&mut self, frame: f32, step: f32) -> f32 {
        if std::mem::take(&mut self.step_once) {
            step
        } else if self.paused {
            0.0
        } else {
            frame * self.speed
        }
    }
}

/// What the viewport draws for debugging.
#[derive(Clone, Copy, Debug, Default)]
pub struct DebugView {
    /// Rings for the radius of every behavior (targets, pickups, triggers...).
    pub colliders: bool,
    /// The collision triangles of 3D areas.
    pub terrain: bool,
}

/// An item to put into the scene.
pub struct PlaceRequest {
    pub item: ItemRef,
    pub parent: NodeId,
    /// Place where the cursor points on the ground; otherwise at the camera's focus.
    pub at_cursor: bool,
}

impl Default for Workshop {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            selected: None,
            plugins_revision: 0,
            scene: Scene::default(),
            selected_node: None,
            selection: Vec::new(),
            clipboard: Vec::new(),
            focus_request: None,
            export: None,
            pick_request: None,
            entity_request: None,
            entity_link: None,
            live: Vec::new(),
            play_clock: PlayClock::default(),
            debug: DebugView::default(),
            hidden_in_play: Default::default(),
            acting: Default::default(),
            toast: None,
            tool: Default::default(),
            snap: false,
            gizmo_busy: false,
            adjusting: None,
            place_requests: Vec::new(),
            viewport_hovered: false,
            show_surfaces: true,
            hovered_cell: None,
            playing: None,
            play_request: None,
            game_view: false,
            typing: false,
            hud: None,
            hud_requests: Vec::new(),
            screen: None,
            menu_paused: false,
            health: None,
            lives: None,
            seconds: None,
            fade: 0,
            banner: None,
            dialog: None,
            sounds: Vec::new(),
            kit_sounds: KitSounds::default(),
            music: None,
            menu_toggle: None,
            presses: Vec::new(),
            asset_editor: Default::default(),
            timeline: Default::default(),
        }
    }
}

impl Workshop {
    /// Every selected node that still exists. `selected_node` is the primary one: if it isn't
    /// in the list (code set it directly) it alone is the selection.
    pub fn selection(&self) -> Vec<NodeId> {
        let Some(primary) = self.selected_node.filter(|p| self.scene.get(*p).is_some()) else {
            return Vec::new();
        };
        let list: Vec<NodeId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.scene.get(*id).is_some())
            .collect();
        if list.contains(&primary) {
            list
        } else {
            vec![primary]
        }
    }

    /// Whether a node's item belongs on the ground of a world (characters, objects) rather
    /// than being a world itself (areas and their tilesets).
    pub fn item_drops_on_ground(&self, node: &crate::scene::Node) -> bool {
        use clockwork_core::recipe::Kind;
        let Some(item) = &node.item else {
            return false;
        };
        let kind = self
            .plugin_index(&item.plugin)
            .and_then(|i| {
                self.plugins[i]
                    .recipe
                    .outputs
                    .iter()
                    .find_map(|o| o.item().filter(|(id, _)| *id == item.output))
            })
            .and_then(|(_, kind)| kind);
        !matches!(kind, Some(Kind::Area | Kind::Tileset))
    }

    pub fn is_selected(&self, id: NodeId) -> bool {
        self.selection().contains(&id)
    }

    /// Selects just this node.
    pub fn select_only(&mut self, id: NodeId) {
        self.selection = vec![id];
        self.selected_node = Some(id);
    }

    /// Adds the node to the selection, or takes it out if it is in it.
    pub fn toggle_select(&mut self, id: NodeId) {
        let mut list = self.selection();
        if let Some(at) = list.iter().position(|n| *n == id) {
            list.remove(at);
        } else {
            list.push(id);
        }
        self.selected_node = list.last().copied();
        self.selection = list;
    }

    pub fn plugin_index(&self, name: &str) -> Option<usize> {
        self.plugins
            .iter()
            .position(|p| p.manifest.plugin.name == name)
    }

    /// Asks the scene system to place a plugin output.
    pub fn place(&mut self, plugin: &str, output: &str, parent: NodeId, at_cursor: bool) {
        self.place_requests.push(PlaceRequest {
            item: ItemRef {
                plugin: plugin.to_string(),
                output: output.to_string(),
            },
            parent,
            at_cursor,
        });
    }
}

/// Folders searched for plugins: `plugins/` in every content folder.
pub fn plugin_dirs() -> Vec<PathBuf> {
    clockwork_library::content::plugin_dirs()
}

impl EditorState {
    /// Creates the state and re-imports the ROMs remembered from the last run.
    pub fn load() -> Self {
        let (stored, problem) = store::load_reporting();
        let mut state = Self {
            library: Library::new().with_platforms(clockwork_platforms::all()),
            missing: Vec::new(),
            imports: Default::default(),
            selected: None,
            search: String::new(),
            agent_prompt: String::new(),
            screen: Screen::default(),
            log: Vec::new(),
            fetch_box_art: stored.fetch_box_art,
            names: stored
                .games
                .iter()
                .filter_map(|g| Some((g.sha1.clone(), g.name.clone()?)))
                .collect(),
            decomps: stored
                .games
                .iter()
                .filter_map(|g| Some((g.sha1.clone(), g.decomp.clone()?)))
                .collect(),
            notes: stored
                .games
                .iter()
                .filter(|g| !g.sha1.is_empty() && !g.notes.is_empty())
                .map(|g| (g.sha1.clone(), g.notes.clone()))
                .collect(),
            decomp_input: String::new(),
            boxart_retry: Vec::new(),
            boxart_busy: false,
            workshop: Workshop::default(),
            agents: Vec::new(),
            claude_connected: false,
            player: None,
            plugin_editor: Default::default(),
            quit_asked: false,
            palette_request: None,
        };
        if let Some(problem) = problem {
            state.log(
                LogLevel::Error,
                format!("The Library could not be loaded: {problem}"),
            );
        }
        let (scene, problem) = Scene::load_reporting();
        state.workshop.scene = scene;
        if let Some(problem) = problem {
            state.log(
                LogLevel::Error,
                format!("The scene could not be loaded: {problem}"),
            );
        }
        for game in &stored.games {
            if let Err(e) = state.library.import_game(game) {
                state.log(
                    LogLevel::Warn,
                    format!("Could not reload ROM (it stays in the Library): {e}"),
                );
                state.missing.push(game.clone());
            }
        }
        // Remember what was just read, so the next start can skip it.
        if state.games_to_save() != stored.games {
            state.save();
        }
        state.log(
            LogLevel::Info,
            format!("Library loaded: {} game(s)", state.library.entries().len()),
        );
        state.scan_plugins();
        state
    }

    /// Reloads all plugins. The selected plugin stays selected if it still exists (plugins are
    /// matched by name, since their order may change). Scene nodes refer to plugins by name, so
    /// they pick up the new versions.
    pub fn scan_plugins(&mut self) {
        let selected = self
            .workshop
            .selected
            .and_then(|i| self.workshop.plugins.get(i))
            .map(|p| p.manifest.plugin.name.clone());

        let mut plugins = Vec::new();
        let mut errors = Vec::new();
        for dir in plugin_dirs() {
            for result in Plugin::load_all(&dir) {
                match result {
                    Ok(plugin) => plugins.push(plugin),
                    Err(e) => errors.push(format!("Could not load plugin: {e}")),
                }
            }
        }
        self.workshop.plugins = plugins;
        for e in errors {
            self.log(LogLevel::Error, e);
        }

        self.workshop.selected = selected.and_then(|name| self.workshop.plugin_index(&name));
        self.workshop.plugins_revision += 1;
        let count = self.workshop.plugins.len();
        self.log(LogLevel::Info, format!("Found {count} plugin(s)"));
    }

    /// The Library entry a plugin's recipe can run on, if the user has that ROM.
    pub fn rom_for(&self, plugin: &Plugin) -> Option<&LibraryEntry> {
        self.library
            .entries()
            .iter()
            .find(|e| plugin.manifest.supports_rom(&e.sha1))
    }

    /// Whether a plugin can be extracted: its game is in the Library, or, for a composition
    /// plugin, every plugin it uses can be.
    pub fn ready(&self, plugin: &Plugin) -> bool {
        self.ready_at(plugin, 0)
    }

    fn ready_at(&self, plugin: &Plugin, depth: usize) -> bool {
        match &plugin.compose {
            None => self.rom_for(plugin).is_some(),
            Some(composition) => {
                depth < 8
                    && clockwork_recipe::compose::dependencies(composition)
                        .iter()
                        .all(|name| {
                            self.workshop.plugin_index(name).is_some_and(|i| {
                                self.ready_at(&self.workshop.plugins[i], depth + 1)
                            })
                        })
            }
        }
    }

    pub fn log(&mut self, level: LogLevel, text: impl Into<String>) {
        let text = text.into();
        match level {
            LogLevel::Info => info!("{text}"),
            LogLevel::Warn => warn!("{text}"),
            LogLevel::Error => error!("{text}"),
        }
        self.log.push(LogLine { level, text });
    }

    /// Imports game files, and the game files in folders, reading them in the background
    /// (see [`EditorState::poll_imports`]).
    pub fn import(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let identifier = self.library.identifier();
        self.imports.queue(identifier, paths);
    }

    /// Adds what the background imports read since last time.
    pub fn poll_imports(&mut self) {
        let read = self.imports.take();
        if read.is_empty() {
            return;
        }
        let mut changed = false;
        for (path, result) in read {
            let entry = match result {
                Ok(entry) => entry,
                Err(e) => {
                    self.log(LogLevel::Error, e);
                    continue;
                }
            };
            let (title, sha1) = (entry.info.title.clone(), entry.sha1.clone());
            // A missing game is found: at its old place, or as the same game elsewhere.
            let found = |g: &Game| g.path == path || (!g.sha1.is_empty() && g.sha1 == sha1);
            if self.missing.iter().any(found) {
                self.missing.retain(|g| !found(g));
                changed = true;
            }
            let (index, new) = self.library.add(entry);
            if new {
                changed = true;
                let notes = self.notes.entry(sha1).or_default();
                if notes.added == 0 {
                    notes.added = clockwork_library::agents::now();
                }
                self.selected = Some(index);
                self.log(LogLevel::Info, format!("Imported {title}"));
            } else {
                self.log(LogLevel::Info, format!("{title} is already in the Library"));
            }
        }
        if changed {
            self.save();
        }
    }

    /// The game's full name if it was identified, otherwise the title from its ROM header.
    pub fn title(&self, entry: &LibraryEntry) -> String {
        match self.names.get(&entry.sha1) {
            Some(name) => display_name(name),
            None if entry.info.title.is_empty() => "Untitled".into(),
            None => entry.info.title.clone(),
        }
    }

    pub fn selected_entry(&self) -> Option<&LibraryEntry> {
        self.selected.and_then(|i| self.library.entries().get(i))
    }

    pub fn remove(&mut self, index: usize) {
        if let Some(entry) = self.library.remove(index) {
            let title = self.title(&entry);
            self.log(LogLevel::Info, format!("Removed {title}"));
            self.selected = match self.selected {
                Some(s) if s == index => None,
                Some(s) if s > index => Some(s - 1),
                other => other,
            };
            self.save();
        }
    }

    /// The title of a missing game: its full name, its header's title or its file's name.
    pub fn missing_title(&self, game: &Game) -> String {
        match (
            self.names.get(&game.sha1).or(game.name.as_ref()),
            &game.title,
        ) {
            (Some(name), _) => display_name(name),
            (None, title) if !title.is_empty() => title.clone(),
            _ => game
                .path
                .file_name()
                .map_or("Untitled".into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    /// Looks for a missing game at another file. A file of another game is imported too, and
    /// the missing one stays missing.
    pub fn locate(&mut self, index: usize, path: PathBuf) {
        let Some(game) = self.missing.get(index).cloned() else {
            return;
        };
        let title = self.missing_title(&game);
        let found = self.library.import(&path).map(|e| e.sha1.clone());
        match found {
            Ok(sha1) if game.sha1.is_empty() || sha1 == game.sha1 => {
                self.missing.remove(index);
                self.log(LogLevel::Info, format!("Found {title}"));
            }
            Ok(_) => self.log(
                LogLevel::Warn,
                format!("That file is another game: it is in the Library now, and {title} is still missing"),
            ),
            Err(e) => self.log(LogLevel::Error, e.to_string()),
        }
        self.save();
    }

    /// Forgets a missing game.
    pub fn forget_missing(&mut self, index: usize) {
        if index < self.missing.len() {
            let game = self.missing.remove(index);
            let title = self.missing_title(&game);
            self.log(LogLevel::Info, format!("Forgot {title}"));
            self.save();
        }
    }

    /// What the user's use of a game added to it.
    pub fn notes(&self, sha1: &str) -> GameNotes {
        self.notes.get(sha1).cloned().unwrap_or_default()
    }

    /// Changes a game's notes, and saves the Library.
    pub fn edit_notes(&mut self, sha1: &str, edit: impl FnOnce(&mut GameNotes)) {
        let notes = self.notes.entry(sha1.to_string()).or_default();
        let before = notes.clone();
        edit(notes);
        if *notes != before {
            self.save();
        }
    }

    /// Notes that a game's ROM was read for a plugin.
    pub fn mark_used(&mut self, sha1: &str) {
        let now = clockwork_library::agents::now();
        self.edit_notes(sha1, |n| n.used = now);
    }

    /// The Library's records: every entry, then the missing games.
    fn games_to_save(&self) -> Vec<Game> {
        games_to_save(
            &self.library,
            &self.missing,
            &self.names,
            &self.decomps,
            &self.notes,
        )
    }

    pub fn save(&mut self) {
        let stored = store::Stored::new(self.games_to_save(), self.fetch_box_art);
        if let Err(e) = store::save(&stored) {
            self.log(LogLevel::Error, format!("Could not save the Library: {e}"));
        }
    }
}

/// A record for each Library entry, with its name and decomp, then each missing game not found
/// since (by path or hash).
fn games_to_save(
    library: &Library,
    missing: &[Game],
    names: &BTreeMap<String, String>,
    decomps: &BTreeMap<String, String>,
    notes: &BTreeMap<String, GameNotes>,
) -> Vec<Game> {
    let mut games: Vec<Game> = library.entries().iter().map(|e| library.game(e)).collect();
    for game in missing {
        let found = games
            .iter()
            .any(|g| g.path == game.path || (!game.sha1.is_empty() && g.sha1 == game.sha1));
        if !found {
            games.push(game.clone());
        }
    }
    for game in &mut games {
        if let Some(name) = names.get(&game.sha1) {
            game.name = Some(name.clone());
        }
        if let Some(decomp) = decomps.get(&game.sha1) {
            game.decomp = Some(decomp.clone());
        }
        if let Some(notes) = notes.get(&game.sha1) {
            game.notes = notes.clone();
        }
    }
    games
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roms_that_failed_to_reload_are_still_saved() {
        let mut library = Library::new().with_platforms(clockwork_platforms::all());
        let mut rom = vec![0u8; 0xc0];
        rom[0xa0..0xa4].copy_from_slice(b"TEST");
        rom[0xb2] = 0x96;
        let checksum = rom[0xa0..0xbd]
            .iter()
            .fold(0u8, |sum, b| sum.wrapping_sub(*b))
            .wrapping_sub(0x19);
        rom[0xbd] = checksum;
        library
            .import_bytes(PathBuf::from("here.gba"), &rom)
            .expect("a valid GBA header");
        let mut unplugged = Game::at(PathBuf::from("unplugged/drive.gba"));
        unplugged.sha1 = "beef".into();
        unplugged.name = Some("Unplugged (USA)".into());
        let missing = [unplugged.clone(), Game::at(PathBuf::from("here.gba"))];
        let sha1 = library.entries()[0].sha1.clone();
        let names = BTreeMap::from([(sha1.clone(), "Here (USA)".to_string())]);
        let decomps = BTreeMap::from([(sha1.clone(), "https://example.com/here".to_string())]);
        let notes = BTreeMap::from([(
            sha1.clone(),
            GameNotes {
                favourite: true,
                ..Default::default()
            },
        )]);
        let games = games_to_save(&library, &missing, &names, &decomps, &notes);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].path, PathBuf::from("here.gba"));
        assert_eq!(games[0].name.as_deref(), Some("Here (USA)"));
        assert_eq!(games[0].decomp.as_deref(), Some("https://example.com/here"));
        assert!(games[0].notes.favourite);
        // A missing game keeps what it was, name and all.
        assert_eq!(games[1], unplugged);
    }
}

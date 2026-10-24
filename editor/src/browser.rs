//! The content browser: everything plugins provide, sorted by kind, with previews.
//!
//! Follows Godot's FileSystem dock and Blender's Asset Browser: items appear by themselves
//! (plugin folders are polled), previews are generated in the background and never block the
//! UI, and items are selected first and then opened (double-click, Enter, or drag into the
//! viewport).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::SystemTime;

use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, RichText, Stroke, vec2},
};
use clockwork_core::{
    asset::{AssetId, OutputRef},
    image::RgbaImage,
    recipe::Kind,
};
use clockwork_recipe::{Extracted, Plugin, assets::AssetIndex, preview};

use crate::{
    scene::ROOT,
    settings::Settings,
    state::{EditorState, LogLevel, plugin_dirs},
    store, theme,
    watch::PluginWatcher,
    workshop::Extractions,
};

/// Previews are stored at most this many pixels on their longer side.
const PREVIEW_MAX: u32 = 256;
/// How often plugin folders are looked over when they can't be watched (and checked for new
/// content folders), in seconds.
const POLL_SECONDS: f64 = 2.0;

/// One thing a plugin provides.
#[derive(Clone)]
pub struct Item {
    /// The plugin's index; `None` for built-in assets.
    pub plugin: Option<usize>,
    pub plugin_name: String,
    pub output: String,
    pub kind: Option<Kind>,
    /// The registered asset kind ("Sprite", "Camera", …).
    pub asset_kind: &'static str,
    /// Whether it can be placed in the scene.
    pub placeable: bool,
    /// The game's display name if its ROM is in the Library, else the plugin's game id.
    pub game: String,
    /// Whether the plugin's ROM is in the Library.
    pub ready: bool,
}

impl Item {
    /// "Tileset bottom" from "tileset_bottom".
    pub fn title(&self) -> String {
        let words = self.output.replace(['_', '-'], " ");
        let mut chars = words.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    }

    pub fn key(&self) -> (String, String) {
        (self.plugin_name.clone(), self.output.clone())
    }

    pub fn id(&self) -> AssetId {
        AssetId::new(
            OutputRef::new(&self.plugin_name, &self.output),
            self.asset_kind,
        )
    }
}

/// Which category tab is open.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    All,
    /// One of [`KIND_GROUPS`].
    Group(usize),
    Kind(Kind),
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomTab {
    #[default]
    Content,
    Log,
}

/// What is dragged from the browser.
#[derive(Clone)]
pub struct DraggedItem {
    pub plugin: usize,
    pub output: String,
    /// The other selected items that can be placed, dragged with it: (plugin, output).
    pub more: Vec<(usize, String)>,
}

#[derive(Resource)]
pub struct Browser {
    pub tab: BottomTab,
    pub category: Category,
    pub search: String,
    /// Game filter: `None` shows all games.
    pub game: Option<String>,
    pub thumb_size: f32,
    /// The asset the Inspector shows: the one clicked last.
    pub selected: Option<AssetId>,
    /// Every selected asset, `selected` among them: Ctrl+click selects more.
    pub picked: Vec<AssetId>,
    /// Plugin filter, inside the game filter: `None` shows all of the game's plugins.
    pub plugin: Option<String>,
    /// Rows instead of thumbnails.
    pub list: bool,
    /// Preview textures by (plugin name, output id).
    previews: HashMap<(String, String), egui::TextureHandle>,
    /// Plugins (by cache key) whose previews are loaded, being made, or failed.
    handled: HashSet<String>,
    /// Plugins whose previews could not be made, with why.
    pub failed: HashMap<String, String>,
    /// The plugin whose previews are being generated.
    pub generating: Option<String>,
    /// Reports plugin files changed on disk.
    watcher: Option<PluginWatcher>,
    /// Where the plugins were last looked over, when they can't be watched.
    fingerprint: u64,
    last_poll: f64,
    /// Every asset, as last listed, and what it was listed for (see [`items_key`]).
    items: Option<(u64, std::sync::Arc<Vec<Item>>)>,
    /// Each plugin's preview cache key (see [`cache_key`]), and what they were made for.
    keys: Option<(u64, Vec<PluginKey>)>,
}

impl Browser {
    /// Selects just this asset.
    pub fn select(&mut self, id: AssetId) {
        self.picked = vec![id.clone()];
        self.selected = Some(id);
    }

    /// Adds an asset to the selection, or takes it out.
    pub fn toggle(&mut self, id: AssetId) {
        if let Some(i) = self.picked.iter().position(|p| *p == id) {
            self.picked.remove(i);
            if self.selected.as_ref() == Some(&id) {
                self.selected = self.picked.last().cloned();
            }
        } else {
            self.picked.push(id.clone());
            self.selected = Some(id);
        }
    }

    pub fn is_selected(&self, id: &AssetId) -> bool {
        self.selected.as_ref() == Some(id) || self.picked.contains(id)
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            tab: BottomTab::Content,
            category: Category::All,
            search: String::new(),
            game: None,
            thumb_size: 96.0,
            selected: None,
            picked: Vec::new(),
            plugin: None,
            list: false,
            previews: HashMap::new(),
            handled: HashSet::new(),
            failed: HashMap::new(),
            generating: None,
            watcher: None,
            fingerprint: 0,
            last_poll: f64::NEG_INFINITY,
            items: None,
            keys: None,
        }
    }
}

impl Browser {
    /// The preview of a plugin output, once generated.
    pub fn preview(&self, plugin: &str, output: &str) -> Option<&egui::TextureHandle> {
        self.previews.get(&(plugin.to_string(), output.to_string()))
    }
}

/// What the asset list depends on: the plugins, and the Library's games and their names (which
/// give items their game and whether it is ready).
fn items_key(state: &EditorState) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.workshop.plugins_revision.hash(&mut hasher);
    for entry in state.library.entries() {
        entry.sha1.hash(&mut hasher);
    }
    state.names.hash(&mut hasher);
    hasher.finish()
}

/// Every asset, listed again only when the plugins or the Library changed.
fn cached_items(state: &EditorState, browser: &mut Browser) -> std::sync::Arc<Vec<Item>> {
    let key = items_key(state);
    match &browser.items {
        Some((k, items)) if *k == key => items.clone(),
        _ => {
            let listed = std::sync::Arc::new(items(state));
            browser.items = Some((key, listed.clone()));
            listed
        }
    }
}

/// Every asset: those of all plugins, and the built-in ones.
pub fn items(state: &EditorState) -> Vec<Item> {
    let index = AssetIndex::build(&state.workshop.plugins);
    index
        .all()
        .iter()
        .map(|asset| {
            let plugin = state.workshop.plugin_index(&asset.at.plugin);
            let (game, ready) = match plugin.map(|i| &state.workshop.plugins[i]) {
                None => ("Built-in".to_string(), true),
                Some(p) => (
                    match (&p.compose, state.rom_for(p)) {
                        (Some(_), _) => "Compositions".to_string(),
                        (None, Some(e)) => state.title(e),
                        (None, None) => p.manifest.source.game.clone(),
                    },
                    state.ready(p),
                ),
            };
            Item {
                plugin,
                plugin_name: asset.at.plugin.clone(),
                output: asset.at.output.clone(),
                kind: asset.category,
                asset_kind: asset.kind,
                placeable: asset.placeable,
                game,
                ready,
            }
        })
        .collect()
}

/// Places an item into the scene at the camera's focus.
pub fn open(state: &mut EditorState, plugin: usize, output: &str) {
    let Some(p) = state.workshop.plugins.get(plugin) else {
        return;
    };
    let name = p.manifest.plugin.name.clone();
    if !state.ready(p) {
        state.log(
            LogLevel::Warn,
            format!("{name}: its game (or a game of a plugin it uses) is not in the Library"),
        );
        return;
    }
    state.workshop.place(&name, output, ROOT, false);
}

/// A fingerprint of the plugin folders: names and modification times of their files.
fn fingerprint() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for dir in plugin_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        folders.sort();
        for folder in folders {
            folder.hash(&mut hasher);
            for file in ["clockwork.toml", "recipe.ron", "compose.ron"] {
                let modified = std::fs::metadata(folder.join(file))
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                modified.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// Identifies a plugin's previews: changes when its files or its game's ROM change, and for a
/// composition plugin when anything of the plugins it uses changes. `None` when the plugin can't
/// be extracted.
fn cache_key(state: &EditorState, plugin: &Plugin) -> Option<String> {
    cache_key_at(state, plugin, 0)
}

fn cache_key_at(state: &EditorState, plugin: &Plugin, depth: usize) -> Option<String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in ["clockwork.toml", "recipe.ron", "compose.ron"] {
        std::fs::read(plugin.dir.join(file))
            .unwrap_or_default()
            .hash(&mut hasher);
    }
    match &plugin.compose {
        None => state.rom_for(plugin)?.sha1.hash(&mut hasher),
        Some(composition) => {
            if depth > 8 {
                return None;
            }
            for name in clockwork_recipe::compose::dependencies(composition) {
                let index = state.workshop.plugin_index(&name)?;
                cache_key_at(state, &state.workshop.plugins[index], depth + 1)?.hash(&mut hasher);
            }
        }
    }
    Some(format!(
        "{}-{:016x}",
        plugin.manifest.plugin.name,
        hasher.finish()
    ))
}

fn cache_root() -> Option<PathBuf> {
    store::data_dir().map(|d| d.join("cache").join("previews"))
}

fn cache_dir(key: &str) -> Option<PathBuf> {
    cache_root().map(|root| root.join(key))
}

/// Removes cached previews made for an earlier version of a plugin that now has other ones.
/// Folders of plugins that aren't installed, or can't be extracted right now, are kept.
fn prune_cache(keys: &[PluginKey]) {
    let Some(entries) = cache_root().and_then(|root| std::fs::read_dir(root).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.file_name().to_string_lossy().into_owned();
        if outdated(&folder, keys) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether a cache folder (`<plugin>-<16 hex digits>`) is of an installed plugin whose previews
/// now have another key.
fn outdated(folder: &str, keys: &[PluginKey]) -> bool {
    keys.iter().any(|(name, key)| {
        let Some(key) = key else { return false };
        let hash = folder
            .strip_prefix(name.as_str())
            .and_then(|rest| rest.strip_prefix('-'));
        hash.is_some_and(|h| h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()))
            && folder != key
    })
}

fn upload(ctx: &egui::Context, name: &str, image: &RgbaImage) -> egui::TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width as usize, image.height as usize],
        &image.pixels,
    );
    ctx.load_texture(
        format!("preview-{name}"),
        color,
        egui::TextureOptions::NEAREST,
    )
}

/// A plugin's name and its preview cache key, `None` while it can't be extracted.
type PluginKey = (String, Option<String>);

type PreviewJob = (String, String, Vec<(String, RgbaImage)>);

/// Picks up plugin changes and makes previews, one plugin at a time, in the background, from
/// what the Workshop extracted.
pub fn browser_system(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    settings: Res<Settings>,
    mut browser: ResMut<Browser>,
    mut extractions: ResMut<Extractions>,
    mut job: Local<Option<Task<PreviewJob>>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let now = ctx.input(|i| i.time);

    // New, changed or removed plugins show up by themselves: the system reports changes, and
    // where it can't, the plugin folders are looked over every few seconds.
    if settings.watch_plugins {
        let browser = &mut *browser;
        let watcher = (browser.watcher).get_or_insert_with(|| PluginWatcher::new(plugin_dirs()));
        let mut changed = watcher.poll(now);
        if now - browser.last_poll > POLL_SECONDS {
            browser.last_poll = now;
            if watcher.is_stale(plugin_dirs()) {
                *watcher = PluginWatcher::new(plugin_dirs());
                changed = true;
            }
            if !watcher.is_watching() {
                let print = fingerprint();
                changed |= browser.fingerprint != 0 && print != browser.fingerprint;
                browser.fingerprint = print;
            }
        }
        if changed {
            state.scan_plugins();
            state.log(LogLevel::Info, "Plugins changed; content updated");
        }
    } else {
        browser.watcher = None;
    }

    // A finished preview job.
    if let Some((name, key, images)) = job.as_mut().and_then(|t| block_on(poll_once(t))) {
        *job = None;
        browser.generating = None;
        for (output, image) in images {
            let texture = upload(&ctx, &format!("{name}-{output}"), &image);
            browser.previews.insert((name.clone(), output), texture);
        }
        browser.failed.remove(&name);
        browser.handled.insert(key);
    }
    if job.is_some() {
        return Ok(());
    }

    // Cache keys read the plugins' files, so they are made only when plugins or games change.
    let listed = items_key(&state);
    if browser.keys.as_ref().is_none_or(|(k, _)| *k != listed) {
        let keys: Vec<PluginKey> = state
            .workshop
            .plugins
            .iter()
            .map(|p| (p.name().to_string(), cache_key(&state, p)))
            .collect();
        prune_cache(&keys);
        browser.keys = Some((listed, keys));
    }

    // The next plugin without previews.
    let next = browser.keys.as_ref().and_then(|(_, keys)| {
        keys.iter().find_map(|(name, key)| {
            let key = key.as_ref()?;
            (!browser.handled.contains(key)).then(|| (name.clone(), key.clone()))
        })
    });
    let Some((name, key)) = next else {
        return Ok(());
    };
    let Some(plugin) = state
        .workshop
        .plugin_index(&name)
        .map(|i| state.workshop.plugins[i].clone())
    else {
        return Ok(());
    };
    let outputs: Vec<String> = plugin.items().into_iter().map(|(id, _)| id).collect();

    // Cached previews load straight from disk.
    if let Some(dir) = cache_dir(&key)
        && outputs
            .iter()
            .all(|o| dir.join(format!("{o}.png")).exists())
    {
        for output in outputs {
            if let Ok(image) = image::open(dir.join(format!("{output}.png"))) {
                let rgba = image.to_rgba8();
                let image = RgbaImage {
                    width: rgba.width(),
                    height: rgba.height(),
                    pixels: rgba.into_raw(),
                };
                let texture = upload(&ctx, &format!("{name}-{output}"), &image);
                browser.previews.insert((name.clone(), output), texture);
            }
        }
        browser.handled.insert(key);
        return Ok(());
    }

    // Otherwise wait for the Workshop's extraction (the plugin editor and the scene use the same
    // one), then draw the previews in the background.
    browser.generating = Some(name.clone());
    extractions.ensure(&mut state, &name);
    if let Some(e) = extractions.error(&name) {
        // Extractions already logged why.
        browser.failed.insert(name.clone(), e.to_string());
        browser.generating = None;
        browser.handled.insert(key);
        return Ok(());
    }
    let Some(extracted) = extractions.shared(&name) else {
        return Ok(());
    };
    *job = Some(AsyncComputeTaskPool::get().spawn(async move {
        let images = generate(&extracted, &key);
        (name, key, images)
    }));
    Ok(())
}

/// Returns (and caches) a small preview of each output a plugin extracted.
fn generate(extracted: &Extracted, key: &str) -> Vec<(String, RgbaImage)> {
    let dir = cache_dir(key);
    if let Some(dir) = &dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let models = extracted.models.iter().map(|m| {
        let (id, model) = (&m.id, &m.model);
        let pose = model.animations.first().map(|a| (a, 0));
        (id.clone(), preview::render_model(model, pose, PREVIEW_MAX))
    });
    // A sprite is shown by one frame: the first of its idle animation facing south when it is a
    // character, else of its first animation.
    let sprites = extracted.sprites.iter().map(|s| {
        let animation = s
            .character
            .as_ref()
            .and_then(|c| s.sprite.animation(&c.animations.idle.south.name))
            .or(s.sprite.animations.first());
        let frame = animation
            .and_then(|a| a.steps.first())
            .and_then(|step| {
                Some(
                    s.sprite
                        .frames
                        .get(step.frame)?
                        .flipped(step.flip_x, step.flip_y),
                )
            })
            .or_else(|| s.sprite.frames.first().cloned());
        (
            s.id.clone(),
            frame.map_or_else(|| RgbaImage::new(1, 1), |f| f.image),
        )
    });
    let mut previews = Vec::new();
    for (id, image) in extracted
        .images
        .iter()
        .cloned()
        .chain(models.collect::<Vec<_>>())
        .chain(sprites.collect::<Vec<_>>())
    {
        let small = if image.width.max(image.height) > PREVIEW_MAX {
            preview::fit(&image, PREVIEW_MAX)
        } else {
            image
        };
        if let Some(dir) = &dir {
            let _ = image::save_buffer(
                dir.join(format!("{id}.png")),
                &small.pixels,
                small.width,
                small.height,
                image::ExtendedColorType::Rgba8,
            );
        }
        previews.push((id, small));
    }
    previews
}

// ---------------------------------------------------------------------------------------------
// UI

/// The kinds of assets, in the groups the sidebar shows them in.
pub const KIND_GROUPS: &[(&str, &[Kind])] = &[
    (
        "World",
        &[Kind::Area, Kind::Tileset, Kind::Object, Kind::Camera],
    ),
    (
        "Characters",
        &[
            Kind::Character,
            Kind::Sprite,
            Kind::Animation,
            Kind::Movement,
            Kind::Ability,
            Kind::Stats,
            Kind::Behavior,
        ],
    ),
    (
        "Interface",
        &[
            Kind::Hud,
            Kind::Ui,
            Kind::Menu,
            Kind::Dialog,
            Kind::Font,
            Kind::Input,
        ],
    ),
    ("Game", &[Kind::Game, Kind::Rules, Kind::Flow]),
    ("Sound", &[Kind::Audio]),
];

/// The symbol a kind of asset is drawn with where it has no preview.
pub fn kind_icon(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::Area) => "🗺",
        Some(Kind::Tileset) => "▦",
        Some(Kind::Object) => "◈",
        Some(Kind::Camera) => "📷",
        Some(Kind::Character) => "☺",
        Some(Kind::Sprite) => "🖼",
        Some(Kind::Animation) => "🎞",
        Some(Kind::Movement) => "➠",
        Some(Kind::Ability) => "⚔",
        Some(Kind::Stats) => "♥",
        Some(Kind::Behavior) => "⚙",
        Some(Kind::Hud) => "▤",
        Some(Kind::Ui) => "◧",
        Some(Kind::Menu) => "☰",
        Some(Kind::Dialog) => "🗨",
        Some(Kind::Font) => "A",
        Some(Kind::Input) => "🎮",
        Some(Kind::Game) => "★",
        Some(Kind::Rules) => "⚖",
        Some(Kind::Flow) => "⇄",
        Some(Kind::Audio) => "♪",
        None => "◇",
    }
}

/// A search, read from what was typed: `kind:sprite plugin:tmc is:placeable sword`. Every part
/// must match; words match the title, plugin, game or kind.
#[derive(Debug, Default, PartialEq)]
struct Query {
    words: Vec<String>,
    kinds: Vec<String>,
    plugins: Vec<String>,
    games: Vec<String>,
    /// `placeable`, `ready` or `missing` (its game's ROM is not in the Library).
    is: Vec<String>,
}

impl Query {
    fn parse(text: &str) -> Self {
        let mut query = Query::default();
        for part in text.to_lowercase().split_whitespace() {
            let (list, value) = match part.split_once(':') {
                Some(("kind", v)) => (&mut query.kinds, v),
                Some(("plugin", v)) => (&mut query.plugins, v),
                Some(("game", v)) => (&mut query.games, v),
                Some(("is", v)) => (&mut query.is, v),
                _ => (&mut query.words, part),
            };
            if !value.is_empty() {
                list.push(value.to_string());
            }
        }
        query
    }

    fn matches(&self, item: &Item) -> bool {
        let kind = [
            item.kind.map(|k| k.singular()).unwrap_or_default(),
            item.kind.map(|k| k.label()).unwrap_or_default(),
            item.asset_kind,
        ]
        .map(str::to_lowercase);
        let has = |s: &str, part: &str| s.to_lowercase().contains(part);
        self.kinds
            .iter()
            .all(|k| kind.iter().any(|s| s.contains(k)))
            && self.plugins.iter().all(|p| has(&item.plugin_name, p))
            && self.games.iter().all(|g| has(&item.game, g))
            && self.is.iter().all(|is| match is.as_str() {
                "placeable" => item.placeable,
                "ready" => item.ready,
                "missing" => !item.ready,
                _ => false,
            })
            && self.words.iter().all(|w| {
                has(&item.title(), w)
                    || has(&item.plugin_name, w)
                    || has(&item.game, w)
                    || kind.iter().any(|s| s.contains(w))
            })
    }
}

fn in_category(item: &Item, category: Category) -> bool {
    match category {
        Category::All => true,
        Category::Group(g) => KIND_GROUPS
            .get(g)
            .is_some_and(|(_, kinds)| item.kind.is_some_and(|k| kinds.contains(&k))),
        Category::Kind(kind) => item.kind == Some(kind),
        Category::Other => item.kind.is_none(),
    }
}

fn matches(item: &Item, browser: &Browser) -> bool {
    in_category(item, browser.category)
        && browser.game.as_ref().is_none_or(|g| *g == item.game)
        && browser
            .plugin
            .as_ref()
            .is_none_or(|p| *p == item.plugin_name)
        && Query::parse(&browser.search).matches(item)
}

/// The sidebar: kinds in their groups, then games and their plugins, each with how many assets
/// it has. Choosing one shows only its assets.
fn sidebar(ui: &mut egui::Ui, all: &[Item], browser: &mut Browser) {
    let count = |c: Category| all.iter().filter(|i| in_category(i, c)).count();
    let row = |ui: &mut egui::Ui, on: bool, text: String, indent: f32| {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            ui.add(egui::Button::selectable(on, text).frame_when_inactive(false))
        })
        .inner
    };
    egui::ScrollArea::vertical()
        .id_salt("browser_sidebar")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(crate::kit::dim("KINDS"));
            let on = browser.category == Category::All;
            if row(ui, on, format!("All  ({})", all.len()), 0.0).clicked() {
                browser.category = Category::All;
            }
            for (g, (name, kinds)) in KIND_GROUPS.iter().enumerate() {
                let n = count(Category::Group(g));
                if n == 0 {
                    continue;
                }
                let on = browser.category == Category::Group(g);
                if row(ui, on, format!("{name}  ({n})"), 0.0).clicked() {
                    browser.category = Category::Group(g);
                }
                // A group's kinds show while it, or one of them, is chosen.
                let open =
                    on || matches!(browser.category, Category::Kind(k) if kinds.contains(&k));
                if open {
                    for kind in kinds.iter().copied() {
                        let n = count(Category::Kind(kind));
                        if n == 0 {
                            continue;
                        }
                        let on = browser.category == Category::Kind(kind);
                        let text = format!("{}  {}  ({n})", kind_icon(Some(kind)), kind.label());
                        if row(ui, on, text, 12.0).clicked() {
                            browser.category = Category::Kind(kind);
                        }
                    }
                }
            }
            let n = count(Category::Other);
            if n > 0 {
                let on = browser.category == Category::Other;
                if row(ui, on, format!("Other  ({n})"), 0.0).clicked() {
                    browser.category = Category::Other;
                }
            }

            ui.add_space(8.0);
            ui.label(crate::kit::dim("SOURCES"));
            let on = browser.game.is_none() && browser.plugin.is_none();
            if row(ui, on, "Everything".into(), 0.0).clicked() {
                browser.game = None;
                browser.plugin = None;
            }
            let mut games: Vec<&str> = all.iter().map(|i| i.game.as_str()).collect();
            games.sort();
            games.dedup();
            for game in games {
                let n = all.iter().filter(|i| i.game == game).count();
                let on = browser.game.as_deref() == Some(game) && browser.plugin.is_none();
                if row(ui, on, format!("{game}  ({n})"), 0.0).clicked() {
                    browser.game = Some(game.to_string());
                    browser.plugin = None;
                }
                if browser.game.as_deref() != Some(game) {
                    continue;
                }
                let mut plugins: Vec<&str> = all
                    .iter()
                    .filter(|i| i.game == game)
                    .map(|i| i.plugin_name.as_str())
                    .collect();
                plugins.sort();
                plugins.dedup();
                for plugin in plugins {
                    let n = all.iter().filter(|i| i.plugin_name == plugin).count();
                    let on = browser.plugin.as_deref() == Some(plugin);
                    if row(ui, on, format!("◆  {plugin}  ({n})"), 12.0).clicked() {
                        browser.plugin = Some(plugin.to_string());
                    }
                }
            }
        });
    if browser.category != Category::All && count(browser.category) == 0 {
        browser.category = Category::All;
    }
}

/// The content browser panel. Returns an item to open, if the user opened one.
pub fn content_panel(ui: &mut egui::Ui, state: &mut EditorState, browser: &mut Browser) {
    let all = cached_items(state, browser);

    // The search, how items are shown and their size; the sidebar, then the items.
    ui.horizontal(|ui| {
        crate::kit::search_field(ui, &mut browser.search, "Search content…", 220.0).on_hover_text(
            "Words match names, plugins, games and kinds. Also kind:sprite, plugin:tmc, \
                 game:minish, is:placeable, is:missing",
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::kit::segmented(
                ui,
                &mut browser.list,
                &[(true, "☰", "List"), (false, "▦", "Thumbnails")],
            );
            if !browser.list {
                ui.add(
                    egui::Slider::new(&mut browser.thumb_size, 64.0..=160.0)
                        .step_by(8.0)
                        .show_value(false)
                        .text("Size"),
                );
            }
            let picked = browser.picked.len();
            if picked > 1 {
                ui.label(crate::kit::hint(format!("{picked} selected")));
            }
        });
    });
    ui.add_space(4.0);

    egui::Panel::left("browser_sidebar")
        .resizable(true)
        .default_size(170.0)
        .size_range(120.0..=320.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin {
            right: 6,
            ..Default::default()
        }))
        .show(ui, |ui| sidebar(ui, &all, browser));

    let shown: Vec<&Item> = all.iter().filter(|i| matches(i, browser)).collect();
    if all.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No content yet").strong());
            ui.small(
                RichText::new(
                    "Plugins appear here by themselves. Extract something with your agent, or \
                     put plugin folders in a \"plugins\" folder next to Clockwork.",
                )
                .color(theme::PARCHMENT_DIM),
            );
        });
        return;
    }
    if shown.is_empty() {
        ui.small(RichText::new("Nothing matches the filters.").color(theme::PARCHMENT_DIM));
        return;
    }

    // What a drag carries: every selected item that can be placed, the dragged one first.
    let placeable: Vec<(usize, String, AssetId)> = all
        .iter()
        .filter(|i| i.ready && i.placeable && browser.is_selected(&i.id()))
        .filter_map(|i| Some((i.plugin?, i.output.clone(), i.id())))
        .collect();

    let mut open_item: Option<(usize, String)> = None;
    let openable = |item: &Item| item.plugin.filter(|_| item.placeable);
    // A behavior asset is placed as a node that follows it.
    let mut place_behavior: Option<OutputRef> = None;
    let mut clicked = false;
    egui::ScrollArea::vertical()
        .id_salt("browser_items")
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut each = |ui: &mut egui::Ui,
                            browser: &mut Browser,
                            item: &Item,
                            response: egui::Response| {
                let (opened, was_clicked) = interact(ui, item, response, browser, &placeable);
                clicked |= was_clicked;
                if opened && let Some(plugin) = openable(item) {
                    open_item = Some((plugin, item.output.clone()));
                } else if opened && item.asset_kind == "Behavior" {
                    place_behavior = Some(OutputRef::new(&item.plugin_name, &item.output));
                }
            };
            if browser.list {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (n, item) in shown.into_iter().enumerate() {
                    let response = list_row(ui, item, browser, n);
                    each(ui, browser, item, response);
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for item in shown {
                        let response = card(ui, item, browser);
                        each(ui, browser, item, response);
                    }
                });
            }
        });

    // Enter opens the selected item, as long as no text field has focus.
    let enter =
        ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().egui_wants_keyboard_input();
    if enter
        && let Some(id) = &browser.selected
        && let Some(item) = all.iter().find(|i| i.id() == *id)
        && let Some(plugin) = openable(item)
    {
        open_item = Some((plugin, item.output.clone()));
    }
    if let Some((plugin, output)) = open_item {
        open(state, plugin, &output);
    }
    if let Some(at) = place_behavior {
        crate::asset_ui::place_behavior(state, at);
    }
    // Selecting an item shows it, and its plugin, in the Inspector.
    if clicked && let Some(id) = &browser.selected {
        state.workshop.selected = state.workshop.plugin_index(&id.at.plugin);
        state.workshop.selected_node = None;
    }
}

/// What clicking, dragging and hovering an item does, the same for cards and rows: a click
/// selects it, Ctrl+click adds it to the selection (or takes it out), a drag carries it and
/// the rest of the selection. Returns (opened, clicked).
fn interact(
    ui: &egui::Ui,
    item: &Item,
    response: egui::Response,
    browser: &mut Browser,
    placeable: &[(usize, String, AssetId)],
) -> (bool, bool) {
    let response = response.on_hover_text(format!(
        "{} · {}\nPlugin: {}\n{}",
        item.title(),
        item.game,
        item.plugin_name,
        if item.asset_kind == "Behavior" {
            "Double-click to place a node that follows it"
        } else if !item.placeable {
            "Select it to see what it uses and what uses it"
        } else if item.ready {
            "Double-click or drag into the viewport to show it. Ctrl+click selects more"
        } else {
            "Import this game's ROM into the Library to use it"
        }
    ));
    let id = item.id();
    let toggle = ui.input(|i| i.modifiers.command);
    if response.clicked() && toggle {
        browser.toggle(id.clone());
    } else if response.clicked() || (response.drag_started() && !browser.is_selected(&id)) {
        browser.select(id.clone());
    }
    if item.ready
        && item.placeable
        && let Some(plugin) = item.plugin
    {
        response.dnd_set_drag_payload(DraggedItem {
            plugin,
            output: item.output.clone(),
            more: placeable
                .iter()
                .filter(|(_, _, other)| *other != id && browser.is_selected(&id))
                .map(|(p, o, _)| (*p, o.clone()))
                .collect(),
        });
    }
    (
        response.double_clicked() && (item.ready || item.asset_kind == "Behavior"),
        response.clicked(),
    )
}

/// One item as a row of the list: its symbol, name, kind, plugin and game.
fn list_row(ui: &mut egui::Ui, item: &Item, browser: &Browser, n: usize) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::click_and_drag());
    let selected = browser.is_selected(&item.id());
    let fill = if selected {
        theme::SELECTED
    } else if response.hovered() {
        theme::RAISED
    } else if n % 2 == 1 {
        Color32::from_white_alpha(4)
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, fill);
    let y = rect.center().y;
    painter.text(
        egui::pos2(rect.left() + 14.0, y),
        egui::Align2::CENTER_CENTER,
        kind_icon(item.kind),
        egui::FontId::proportional(13.0),
        theme::BRASS,
    );
    let columns = [
        (item.title(), 0.0, theme::PARCHMENT),
        (
            item.kind
                .map_or(item.asset_kind, |k| k.singular())
                .to_string(),
            0.45,
            theme::PARCHMENT_DIM,
        ),
        (item.plugin_name.clone(), 0.62, theme::PARCHMENT_DIM),
        (
            if item.ready {
                item.game.clone()
            } else {
                format!("{} (needs ROM)", item.game)
            },
            0.80,
            theme::PARCHMENT_DIM,
        ),
    ];
    let left = rect.left() + 30.0;
    let span = rect.width() - 30.0;
    for (i, (text, at, color)) in columns.iter().enumerate() {
        let end = columns.get(i + 1).map_or(1.0, |c| c.1);
        let chars = ((end - at) * span / 7.0) as usize;
        let color = if selected && i == 0 {
            theme::BRASS_BRIGHT
        } else {
            *color
        };
        painter.text(
            egui::pos2(left + at * span, y),
            egui::Align2::LEFT_CENTER,
            crate::ui::truncate(text, chars.max(4)),
            egui::FontId::proportional(12.5),
            color,
        );
    }
    response
}

/// One item card.
fn card(ui: &mut egui::Ui, item: &Item, browser: &Browser) -> egui::Response {
    let size = browser.thumb_size;
    let (rect, response) =
        ui.allocate_exact_size(vec2(size, size + 34.0), egui::Sense::click_and_drag());
    let thumb = egui::Rect::from_min_size(rect.min, vec2(size, size));
    let selected = browser.is_selected(&item.id());
    let painter = ui.painter();

    painter.rect_filled(thumb, 3.0, theme::BG_DEEP);
    // Previews are of placeable things; an ability may share its id with a sprite.
    let preview = browser.previews.get(&item.key()).filter(|_| item.placeable);
    match preview {
        Some(texture) => {
            // Fit the whole preview inside the square; very long sheets (like tilesets) would
            // become a sliver, so they show their first square instead.
            let tex = texture.size_vec2();
            let aspect = tex.x / tex.y;
            let (shown, uv) = if !(0.5..=2.0).contains(&aspect) {
                let uv = if aspect < 1.0 {
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, aspect))
                } else {
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0 / aspect, 1.0))
                };
                (thumb.shrink(1.0), uv)
            } else {
                let scale = (size / tex.x).min(size / tex.y);
                let rect = egui::Rect::from_center_size(thumb.center(), tex * scale);
                (
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                )
            };
            painter.image(texture.id(), shown, uv, Color32::WHITE);
        }
        None => {
            let generating = browser.generating.as_deref() == Some(item.plugin_name.as_str());
            let note = if !item.placeable {
                item.asset_kind
            } else if !item.ready {
                "Needs ROM"
            } else if browser.failed.contains_key(&item.plugin_name) {
                "No preview"
            } else if generating {
                "Generating…"
            } else {
                "Waiting…"
            };
            let t = if generating && item.placeable {
                ui.input(|i| i.time) as f32
            } else {
                0.0
            };
            if item.placeable {
                theme::gear(
                    painter,
                    thumb.center() - vec2(0.0, 8.0),
                    size * 0.18,
                    10,
                    t,
                    theme::RAISED,
                    theme::BG_DEEP,
                );
            } else {
                // Kinds that are not pictures are shown by their symbol.
                painter.text(
                    thumb.center() - vec2(0.0, 8.0),
                    egui::Align2::CENTER_CENTER,
                    kind_icon(item.kind),
                    egui::FontId::proportional(size * 0.34),
                    theme::BRASS_DIM,
                );
            }
            painter.text(
                thumb.center() + vec2(0.0, size * 0.22),
                egui::Align2::CENTER_CENTER,
                note,
                egui::FontId::proportional(11.0),
                theme::PARCHMENT_DIM,
            );
        }
    }
    let frame = if selected {
        theme::BRASS_BRIGHT
    } else if response.hovered() {
        theme::BORDER_STRONG
    } else {
        theme::BORDER
    };
    painter.rect_stroke(
        thumb,
        3.0,
        Stroke::new(if selected { 2.0 } else { 1.0 }, frame),
        egui::StrokeKind::Inside,
    );
    if let Some(kind) = item.kind {
        // The kind, on a dark tag so it reads over any picture.
        let galley = painter.layout_no_wrap(
            kind.singular().to_uppercase(),
            crate::theme::strong(9.0),
            theme::BRASS_BRIGHT,
        );
        let tag = egui::Rect::from_min_size(
            thumb.left_top() + vec2(4.0, 4.0),
            galley.size() + vec2(8.0, 2.0),
        );
        painter.rect_filled(tag, 3.0, Color32::from_black_alpha(190));
        painter.galley(tag.min + vec2(4.0, 1.0), galley, theme::BRASS_BRIGHT);
    }

    let max_chars = (size / 7.0) as usize;
    let cut = |s: String| {
        if s.chars().count() > max_chars {
            format!(
                "{}…",
                s.chars()
                    .take(max_chars.saturating_sub(1))
                    .collect::<String>()
            )
        } else {
            s
        }
    };
    painter.text(
        egui::pos2(rect.left(), thumb.bottom() + 3.0),
        egui::Align2::LEFT_TOP,
        cut(item.title()),
        egui::FontId::proportional(13.0),
        if selected {
            theme::BRASS_BRIGHT
        } else {
            theme::PARCHMENT
        },
    );
    painter.text(
        egui::pos2(rect.left(), thumb.bottom() + 19.0),
        egui::Align2::LEFT_TOP,
        cut(item.plugin_name.clone()),
        egui::FontId::proportional(10.5),
        theme::PARCHMENT_DIM,
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(output: &str, kind: Option<Kind>, asset_kind: &'static str) -> Item {
        Item {
            plugin: Some(0),
            plugin_name: "tmc-link".into(),
            output: output.into(),
            kind,
            asset_kind,
            placeable: kind == Some(Kind::Sprite),
            game: "The Minish Cap".into(),
            ready: true,
        }
    }

    #[test]
    fn every_kind_is_in_one_group() {
        for kind in Kind::ALL {
            let groups = KIND_GROUPS
                .iter()
                .filter(|(_, k)| k.contains(&kind))
                .count();
            assert_eq!(groups, 1, "{kind:?}");
        }
    }

    #[test]
    fn typed_searches_narrow_by_kind_plugin_game_and_state() {
        let sword = item("sword_slash", Some(Kind::Ability), "Ability");
        let sprite = item("link_walk", Some(Kind::Sprite), "Sprite");
        let found = |q: &str| {
            let query = Query::parse(q);
            [&sword, &sprite]
                .into_iter()
                .filter(|i| query.matches(i))
                .map(|i| i.output.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(found(""), ["sword_slash", "link_walk"]);
        assert_eq!(found("kind:sprite"), ["link_walk"]);
        assert_eq!(found("kind:abil slash"), ["sword_slash"]);
        assert_eq!(found("is:placeable"), ["link_walk"]);
        assert_eq!(found("game:minish Walk"), ["link_walk"]);
        assert!(found("plugin:sm64").is_empty());
        assert!(found("is:missing").is_empty());
        // Every word has to match.
        assert!(found("sword walk").is_empty());
    }

    #[test]
    fn ctrl_click_adds_to_the_selection_and_takes_out_again() {
        let mut browser = Browser::default();
        let a = item("a", None, "Items").id();
        let b = item("b", None, "Items").id();
        browser.select(a.clone());
        browser.toggle(b.clone());
        assert!(browser.is_selected(&a) && browser.is_selected(&b));
        assert_eq!(browser.selected, Some(b.clone()));
        browser.toggle(b.clone());
        assert!(!browser.is_selected(&b));
        assert_eq!(browser.selected, Some(a.clone()));
        browser.select(b.clone());
        assert!(!browser.is_selected(&a));
    }

    #[test]
    fn only_outdated_previews_of_installed_plugins_are_pruned() {
        let keys = vec![
            ("tmc".to_string(), Some("tmc-00000000000000aa".to_string())),
            (
                "tmc-link".to_string(),
                Some("tmc-link-00000000000000bb".to_string()),
            ),
            ("sm64".to_string(), None),
        ];
        // An earlier version of an installed plugin.
        assert!(outdated("tmc-00000000000000ff", &keys));
        assert!(outdated("tmc-link-00000000000000ff", &keys));
        // The current ones.
        assert!(!outdated("tmc-00000000000000aa", &keys));
        assert!(!outdated("tmc-link-00000000000000bb", &keys));
        // A plugin that can't be extracted right now, one that isn't installed, other folders.
        assert!(!outdated("sm64-00000000000000ff", &keys));
        assert!(!outdated("other-00000000000000ff", &keys));
        assert!(!outdated("tmc-notes", &keys));
    }
}

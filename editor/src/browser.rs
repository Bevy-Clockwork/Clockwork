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
    workshop::Extractions,
};

/// Previews are stored at most this many pixels on their longer side.
const PREVIEW_MAX: u32 = 256;
/// How often plugin folders are checked for changes, in seconds.
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
}

#[derive(Resource)]
pub struct Browser {
    pub tab: BottomTab,
    pub category: Category,
    pub search: String,
    /// Game filter: `None` shows all games.
    pub game: Option<String>,
    pub thumb_size: f32,
    pub selected: Option<AssetId>,
    /// Preview textures by (plugin name, output id).
    previews: HashMap<(String, String), egui::TextureHandle>,
    /// Plugins (by cache key) whose previews are loaded, being made, or failed.
    handled: HashSet<String>,
    /// Plugins whose previews could not be made, with why.
    pub failed: HashMap<String, String>,
    /// The plugin whose previews are being generated.
    pub generating: Option<String>,
    fingerprint: u64,
    last_poll: f64,
    /// Every asset, as last listed, and what it was listed for (see [`items_key`]).
    items: Option<(u64, std::sync::Arc<Vec<Item>>)>,
    /// Each plugin's preview cache key (see [`cache_key`]), and what they were made for.
    keys: Option<(u64, Vec<PluginKey>)>,
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
            previews: HashMap::new(),
            handled: HashSet::new(),
            failed: HashMap::new(),
            generating: None,
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

    // New, changed or removed plugins show up by themselves.
    if settings.watch_plugins && now - browser.last_poll > POLL_SECONDS {
        browser.last_poll = now;
        let print = fingerprint();
        if browser.fingerprint != 0 && print != browser.fingerprint {
            state.scan_plugins();
            state.log(LogLevel::Info, "Plugins changed; content updated");
        }
        browser.fingerprint = print;
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

fn matches(item: &Item, browser: &Browser) -> bool {
    let category = match browser.category {
        Category::All => true,
        Category::Kind(kind) => item.kind == Some(kind),
        Category::Other => item.kind.is_none(),
    };
    let game = browser.game.as_ref().is_none_or(|g| *g == item.game);
    let query = browser.search.to_lowercase();
    let search = query.is_empty()
        || [
            item.title(),
            item.plugin_name.clone(),
            item.game.clone(),
            item.kind.map(|k| k.label().to_string()).unwrap_or_default(),
        ]
        .iter()
        .any(|s| s.to_lowercase().contains(&query));
    category && game && search
}

/// The content browser panel. Returns an item to open, if the user opened one.
pub fn content_panel(ui: &mut egui::Ui, state: &mut EditorState, browser: &mut Browser) {
    let all = cached_items(state, browser);

    // Filters and thumbnail size, then the category tabs (only kinds that exist), which wrap.
    ui.horizontal(|ui| {
        crate::kit::search_field(ui, &mut browser.search, "Search content…", 180.0);
        let mut games: Vec<String> = all.iter().map(|i| i.game.clone()).collect();
        games.sort();
        games.dedup();
        egui::ComboBox::from_id_salt("game_filter")
            .selected_text(browser.game.clone().unwrap_or_else(|| "All games".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut browser.game, None, "All games");
                for g in games {
                    ui.selectable_value(&mut browser.game, Some(g.clone()), g);
                }
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Slider::new(&mut browser.thumb_size, 64.0..=160.0)
                    .step_by(8.0)
                    .show_value(false)
                    .text("Size"),
            );
        });
    });
    ui.horizontal_wrapped(|ui| {
        let count = |c: Category| {
            all.iter()
                .filter(|i| match c {
                    Category::All => true,
                    Category::Kind(k) => i.kind == Some(k),
                    Category::Other => i.kind.is_none(),
                })
                .count()
        };
        let mut tabs = vec![Category::All];
        tabs.extend(
            Kind::ALL
                .iter()
                .map(|k| Category::Kind(*k))
                .filter(|c| count(*c) > 0),
        );
        if count(Category::Other) > 0 {
            tabs.push(Category::Other);
        }
        if !tabs.contains(&browser.category) {
            browser.category = Category::All;
        }
        for tab in tabs {
            let label = match tab {
                Category::All => "All",
                Category::Kind(k) => k.label(),
                Category::Other => "Other",
            };
            ui.selectable_value(
                &mut browser.category,
                tab,
                format!("{label} ({})", count(tab)),
            );
        }
    });
    ui.add_space(4.0);

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

    let mut open_item: Option<(usize, String)> = None;
    let openable = |item: &Item| item.plugin.filter(|_| item.placeable);
    // A behavior asset is placed as a node that follows it.
    let mut place_behavior: Option<OutputRef> = None;
    let mut clicked = false;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                for item in shown {
                    let (opened, was_clicked) = card(ui, item, browser);
                    clicked |= was_clicked;
                    if opened && let Some(plugin) = openable(item) {
                        open_item = Some((plugin, item.output.clone()));
                    } else if opened && item.asset_kind == "Behavior" {
                        place_behavior = Some(OutputRef::new(&item.plugin_name, &item.output));
                    }
                }
            });
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

/// One item card. Returns (double-clicked, clicked).
fn card(ui: &mut egui::Ui, item: &Item, browser: &mut Browser) -> (bool, bool) {
    let size = browser.thumb_size;
    let (rect, response) =
        ui.allocate_exact_size(vec2(size, size + 34.0), egui::Sense::click_and_drag());
    let thumb = egui::Rect::from_min_size(rect.min, vec2(size, size));
    let selected = browser.selected.as_ref() == Some(&item.id());
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
            theme::gear(
                painter,
                thumb.center() - vec2(0.0, 8.0),
                size * 0.18,
                10,
                t,
                theme::RAISED,
                theme::BG_DEEP,
            );
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
            "Double-click or drag into the viewport to show it"
        } else {
            "Import this game's ROM into the Library to use it"
        }
    ));
    if response.clicked() || response.drag_started() {
        browser.selected = Some(item.id());
    }
    if item.ready
        && item.placeable
        && let Some(plugin) = item.plugin
    {
        response.dnd_set_drag_payload(DraggedItem {
            plugin,
            output: item.output.clone(),
        });
    }
    (
        response.double_clicked() && (item.ready || item.asset_kind == "Behavior"),
        response.clicked(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

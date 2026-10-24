//! The Workshop's 3D side: shows the scene's nodes in the viewport, places dropped items, picks
//! surface cells under the cursor, and moves the viewport camera.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::PrimaryWindow,
};
use clockwork_core::{
    NativeScale,
    image::RgbaImage,
    recipe::{Kind, Output},
    surface::SurfaceMap,
};
use clockwork_recipe::{Extracted, Plugin};

use crate::{
    ViewportCamera, models,
    scene::{ItemRef, Node, NodeId, ROOT},
    settings::Settings,
    sprites,
    state::{EditorState, LogLevel, PlaceRequest},
    surfaces,
    terrains::SceneTerrains,
};

/// The scene shown while nothing is placed.
#[derive(Component)]
pub struct Placeholder;

/// A simple shape standing in for a behavior node that shows nothing from a plugin, so targets
/// and pickups can be seen (and hit) before they have art. It follows the behavior's radius.
#[derive(Component)]
pub struct BehaviorShape(pub NodeId);

/// The viewport camera orbits `focus`. Left drag orbits, right or middle drag pans, the wheel
/// zooms.
#[derive(Resource)]
pub struct OrbitCamera {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.65,
            distance: 12.0,
        }
    }
}

/// GBA games are pixel art on a 16-pixel grid: one tile is one meter.
const GBA_SCALE: f32 = 16.0;

/// Recipe results, by plugin name. Recipes run in the background when a node first needs them.
#[derive(Resource, Default)]
pub struct Extractions {
    /// The plugins revision these results belong to.
    plugins_revision: u64,
    /// Bumped whenever results are thrown away or a new one arrives.
    revision: u64,
    by_plugin: HashMap<String, Result<Arc<Extracted>, String>>,
    /// The plugins as they were when their results were kept, to tell which changed.
    seen: HashMap<String, Plugin>,
    /// Recipes still running.
    pending: HashMap<String, Task<Result<Extracted, String>>>,
    /// ROMs in canonical form, by SHA-1, read on first use. Kept when plugins change.
    roms: HashMap<String, Arc<Vec<u8>>>,
    /// ROMs read since the Library last heard, so it can note when each game was last used.
    read: Vec<String>,
}

impl SceneView {
    /// The entity showing a scene node.
    pub fn entity(&self, node: NodeId) -> Option<Entity> {
        self.entities.get(&node).copied()
    }
}

impl Extractions {
    /// Changes whenever plugins are extracted again or an extraction finishes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, plugin: &str) -> Option<&Extracted> {
        self.by_plugin
            .get(plugin)
            .and_then(|r| r.as_ref().ok())
            .map(|x| x.as_ref())
    }

    /// The result, to hand to work in the background.
    pub fn shared(&self, plugin: &str) -> Option<Arc<Extracted>> {
        self.by_plugin.get(plugin)?.as_ref().ok().cloned()
    }

    /// Why a plugin couldn't be extracted, if it couldn't.
    pub fn error(&self, plugin: &str) -> Option<&str> {
        self.by_plugin
            .get(plugin)
            .and_then(|r| r.as_ref().err())
            .map(String::as_str)
    }

    /// Whether the plugin's recipe is still running.
    pub fn is_pending(&self, plugin: &str) -> bool {
        self.pending.contains_key(plugin)
    }

    /// Whether any recipe is still running.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Plugins changed on disk: drops the results (and cancels the recipes) of plugins that
    /// changed or are gone, of every plugin that uses one of them, and failures, which may work
    /// now. The others are kept, so one edited recipe doesn't extract everything again.
    fn invalidate(&mut self, plugins_revision: u64, plugins: &[Plugin]) {
        self.plugins_revision = plugins_revision;
        self.revision += 1;
        let mut stale: HashSet<String> = self
            .seen
            .iter()
            .filter(|(name, old)| {
                plugins
                    .iter()
                    .find(|p| p.name() == name.as_str())
                    .is_none_or(|p| !same_plugin(p, old))
            })
            .map(|(name, _)| name.clone())
            .collect();
        loop {
            let more: Vec<String> = plugins
                .iter()
                .filter(|p| !stale.contains(p.name()))
                .filter(|p| uses_plugins(p).iter().any(|d| stale.contains(d)))
                .map(|p| p.name().to_string())
                .collect();
            if more.is_empty() {
                break;
            }
            stale.extend(more);
        }
        self.by_plugin
            .retain(|name, result| result.is_ok() && !stale.contains(name));
        self.pending.retain(|name, _| !stale.contains(name));
        self.seen = plugins
            .iter()
            .map(|p| (p.name().to_string(), p.clone()))
            .collect();
    }

    /// Starts the plugin's recipe if it hasn't run yet; [`Extractions::get`] has the result once
    /// it's done. A composition plugin first waits for the plugins it uses, then puts their
    /// outputs together.
    pub fn ensure(&mut self, state: &mut EditorState, plugin: &str) {
        self.ensure_at(state, plugin, 0);
    }

    fn ensure_at(&mut self, state: &mut EditorState, plugin: &str, depth: usize) {
        if self.by_plugin.contains_key(plugin) || self.pending.contains_key(plugin) {
            return;
        }
        let composition = state
            .workshop
            .plugin_index(plugin)
            .and_then(|i| state.workshop.plugins[i].compose.clone());
        if let Some(composition) = &composition
            && depth < 8
        {
            let dependencies = clockwork_recipe::compose::dependencies(composition);
            for dependency in &dependencies {
                self.ensure_at(state, dependency, depth + 1);
            }
            // Composed once every plugin it uses is done: their results bump the revision, which
            // has the scene and the plugin editor ask for it again.
            if dependencies.iter().any(|d| self.pending.contains_key(d)) {
                return;
            }
        }
        let started = (|| {
            let index = state
                .workshop
                .plugin_index(plugin)
                .ok_or("plugin not installed")?;
            if let Some(composition) = &composition {
                return clockwork_recipe::compose::compose(composition, &|name| self.get(name))
                    .map(Some)
                    .map_err(|e| {
                        match clockwork_recipe::compose::dependencies(composition)
                            .iter()
                            .find_map(|d| self.error(d).map(|why| format!("{d}: {why}")))
                        {
                            Some(why) => why,
                            None => e,
                        }
                    });
            }
            let recipe = state.workshop.plugins[index].recipe.clone();
            let rom = self.rom(state, index)?;
            // Characters find the parts they name among every plugin's assets.
            let assets = clockwork_recipe::assets::OwnedAssets::of(&state.workshop.plugins);
            let task = AsyncComputeTaskPool::get().spawn(async move {
                clockwork_recipe::run_in(&recipe, &rom, &assets).map_err(|e| e.to_string())
            });
            self.pending.insert(plugin.to_string(), task);
            Ok(None)
        })();
        match started {
            Ok(None) => {}
            Ok(Some(extracted)) => self.finish(state, plugin, Ok(extracted)),
            Err(e) => self.finish(state, plugin, Err(e)),
        }
    }

    /// The ROM a plugin extracts from, read from disk the first time any plugin needs it.
    fn rom(&mut self, state: &EditorState, index: usize) -> Result<Arc<Vec<u8>>, String> {
        let entry = state
            .rom_for(&state.workshop.plugins[index])
            .ok_or("its game is not in the Library")?;
        if let Some(rom) = self.roms.get(&entry.sha1) {
            return Ok(rom.clone());
        }
        let rom = Arc::new(state.library.rom_bytes(entry).map_err(|e| e.to_string())?);
        self.roms.insert(entry.sha1.clone(), rom.clone());
        self.read.push(entry.sha1.clone());
        Ok(rom)
    }

    /// Keeps a result, logs it and lets everything showing extractions know.
    fn finish(&mut self, state: &mut EditorState, plugin: &str, result: Result<Extracted, String>) {
        match &result {
            Ok(x) => state.log(
                LogLevel::Info,
                format!(
                    "{plugin}: extracted {} image(s), {} surface map(s), {} model(s), {} sprite(s)",
                    x.images.len(),
                    x.surfaces.len(),
                    x.models.len(),
                    x.sprites.len()
                ),
            ),
            Err(e) => state.log(LogLevel::Error, format!("{plugin}: {e}")),
        }
        self.by_plugin
            .insert(plugin.to_string(), result.map(Arc::new));
        self.revision += 1;
    }
}

/// Whether a plugin reloaded from disk is the same as before.
fn same_plugin(a: &Plugin, b: &Plugin) -> bool {
    a.dir == b.dir && a.manifest == b.manifest && a.recipe == b.recipe && a.compose == b.compose
}

/// The other plugins whose outputs a plugin's result is made from: those a composition puts
/// together, and those whose assets a recipe's outputs name (a character's movement, say).
fn uses_plugins(plugin: &Plugin) -> Vec<String> {
    let mut names: Vec<String> = match &plugin.compose {
        Some(composition) => clockwork_recipe::compose::dependencies(composition),
        None => plugin
            .recipe
            .outputs
            .iter()
            .flat_map(|o| (clockwork_core::asset::kind(o).uses)(plugin.name(), o))
            .map(|u| u.asset.plugin)
            .collect(),
    };
    names.retain(|n| n != plugin.name() && n != clockwork_core::asset::BUILTIN);
    names.sort();
    names.dedup();
    names
}

/// Throws results away when plugins changed on disk, and collects recipes that finished in the
/// background.
pub fn extraction_system(mut state: ResMut<EditorState>, mut extractions: ResMut<Extractions>) {
    if state.workshop.plugins_revision != extractions.plugins_revision {
        let plugins = state.workshop.plugins.clone();
        extractions.invalidate(state.workshop.plugins_revision, &plugins);
    }
    let done: Vec<(String, Result<Extracted, String>)> = extractions
        .pending
        .iter_mut()
        .filter_map(|(plugin, task)| Some((plugin.clone(), block_on(poll_once(task))?)))
        .collect();
    for (plugin, result) in done {
        extractions.pending.remove(&plugin);
        extractions.finish(&mut state, &plugin, result);
    }
    for sha1 in std::mem::take(&mut extractions.read) {
        state.mark_used(&sha1);
    }
}

/// The surface map covering an output image exactly, if there is one: the one that names the
/// image, else (for maps that name none) one of the same size.
pub fn surfaces_for<'a>(extracted: &'a Extracted, output: &str) -> Option<&'a SurfaceMap> {
    let (_, image) = extracted.images.iter().find(|(id, _)| id == output)?;
    let named = |id: &str| {
        extracted
            .surface_images
            .iter()
            .find(|(surface, _)| surface == id)
            .map(|(_, image)| image.as_str())
    };
    if let Some((_, map)) = extracted
        .surfaces
        .iter()
        .find(|(id, _)| named(id) == Some(output))
    {
        return Some(map);
    }
    extracted
        .surfaces
        .iter()
        .filter(|(id, _)| named(id).is_none())
        .map(|(_, map)| map)
        .find(|m| m.width * m.cell_size == image.width && m.height * m.cell_size == image.height)
}

/// A scene node's entity.
#[derive(Component)]
pub struct SceneNode(pub NodeId);

/// The image plane of a node, with its size in meters.
#[derive(Component)]
pub struct NodeImage {
    pub node: NodeId,
    pub size: Vec2,
}

/// Marks surface overlay planes.
#[derive(Component)]
pub struct SurfaceOverlay;

/// What the viewport currently shows of the scene.
#[derive(Resource, Default)]
pub struct SceneView {
    seen: u64,
    revision: u64,
    plugins_revision: u64,
    entities: HashMap<NodeId, Entity>,
    textures: HashMap<(String, String), Handle<Image>>,
    models: HashMap<(String, String), models::BuiltModel>,
    sprites: HashMap<(String, String), sprites::BuiltSprite>,
    overlays: HashMap<(String, String), Handle<Image>>,
    /// Frame the camera on this node once it's built.
    frame: Option<NodeId>,
}

/// Puts requested items into the scene: where the cursor points on the ground, or at the
/// camera's focus. The new node is selected and opens the adjust-after-drop panel.
pub fn place_system(
    mut state: ResMut<EditorState>,
    mut view: ResMut<SceneView>,
    mut extractions: ResMut<Extractions>,
    orbit: Res<OrbitCamera>,
    terrains: Res<SceneTerrains>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<ViewportCamera>>,
) {
    if let Some(behavior) = state.workshop.entity_request.take() {
        place_entity(&mut state, &terrains, &orbit, &window, &camera, behavior);
        return;
    }
    let (camera, camera_transform) = *camera;
    // Several items dropped at once are spread out in a row from where they were dropped.
    let requests = std::mem::take(&mut state.workshop.place_requests);
    for (n, request) in requests.into_iter().enumerate() {
        // A scene output becomes a group of nodes, once its plugin has run.
        if is_scene(&state, &request.item) {
            extractions.ensure(&mut state, &request.item.plugin);
            let template = extractions.get(&request.item.plugin).and_then(|x| {
                x.scenes
                    .iter()
                    .find(|(id, _)| *id == request.item.output)
                    .map(|(_, t)| t.clone())
            });
            match template {
                Some(template) => {
                    let id = add_scene(&mut state, &request, &template);
                    view.frame = Some(id);
                }
                // Still running: ask again next frame (a failed run leaves it to the error).
                None if extractions.is_pending(&request.item.plugin) => {
                    state.workshop.place_requests.push(request);
                }
                None => {}
            }
            continue;
        }
        let ray = request
            .at_cursor
            .then(|| window.cursor_position())
            .flatten()
            .and_then(|cursor| camera.viewport_to_world(camera_transform, cursor).ok());
        let mut ground = ray
            .and_then(|ray| {
                let d = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
                Some(ray.get_point(d))
            })
            .unwrap_or(Vec3::new(orbit.focus.x, 0.0, orbit.focus.z));
        // Characters and objects dropped onto a 3D world land on its ground, under the cursor (or
        // where the camera looks when there is no cursor); areas keep to the ground plane.
        let on_ground = drops_on_ground(&state, &request.item);
        if on_ground {
            // Without a cursor, where the camera looks: the ray through the center of the view.
            let looking = Dir3::new(orbit.focus - camera_transform.translation())
                .ok()
                .map(|direction| Ray3d::new(camera_transform.translation(), direction));
            if let Some(hit) = ray.or(looking).and_then(|r| terrains.raycast_floor(r)) {
                ground = hit;
            }
        }
        ground.x += n as f32 * 1.5;

        let scene = &mut state.workshop.scene;
        let first_item = !scene.nodes.iter().any(|n| n.item.is_some());
        // Positions are stored relative to the parent. Dropped onto a node in the scene tree (not
        // the viewport), the item goes to that node's origin.
        let local = if !request.at_cursor && request.parent != ROOT {
            // Onto a 3D area in the tree: at its start point, if it has one.
            match terrains.spawn_of(request.parent).filter(|_| on_ground) {
                Some((spawn, _)) => scene
                    .world_transform(request.parent)
                    .compute_affine()
                    .inverse()
                    .transform_point3(spawn),
                None => Vec3::ZERO,
            }
        } else {
            let parent_world = scene.world_transform(request.parent);
            parent_world
                .compute_affine()
                .inverse()
                .transform_point3(ground)
        };
        let title = request.item.output.replace(['_', '-'], " ");
        let name = format!("{} · {}", capitalize(&title), request.item.plugin);
        let id = scene.add(request.parent, name, Some(request.item), local);

        state.workshop.selected_node = Some(id);
        state.workshop.adjusting = Some(id);
        if first_item {
            view.frame = Some(id);
        }
    }
}

/// Puts a new behavior node where the camera looks: on the ground of a 3D world there, or on the
/// ground plane.
fn place_entity(
    state: &mut EditorState,
    terrains: &SceneTerrains,
    orbit: &OrbitCamera,
    window: &Window,
    camera: &(&Camera, &GlobalTransform),
    behavior: crate::entities::Behavior,
) {
    let _ = window;
    let (camera, camera_transform) = *camera;
    let looking = Dir3::new(orbit.focus - camera_transform.translation())
        .ok()
        .map(|direction| Ray3d::new(camera_transform.translation(), direction));
    let at = looking
        .and_then(|ray| terrains.raycast_floor(ray))
        .or_else(|| {
            looking.and_then(|ray| {
                let d = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
                Some(ray.get_point(d))
            })
        })
        .unwrap_or(Vec3::new(orbit.focus.x, 0.0, orbit.focus.z));
    let _ = camera;
    let name = behavior.kind().to_string();
    let scene = &mut state.workshop.scene;
    let id = scene.add(ROOT, name, None, at);
    let link = state.workshop.entity_link.take();
    if let Some(node) = scene.get_mut(id) {
        node.behaviors = vec![clockwork_core::behavior::NodeBehavior {
            from: link,
            ..clockwork_core::behavior::NodeBehavior::new(behavior)
        }];
    }
    state.workshop.select_only(id);
}

fn is_scene(state: &EditorState, item: &ItemRef) -> bool {
    state.workshop.plugin_index(&item.plugin).is_some_and(|i| {
        state.workshop.plugins[i]
            .recipe
            .outputs
            .iter()
            .any(|o| matches!(o, Output::Scene { id, .. } if *id == item.output))
    })
}

/// Adds a plugin's scene under `request.parent`: a node named after it holding a node per
/// placement, each in its group (made the first time a placement names it). Returns the
/// scene's node.
fn add_scene(
    state: &mut EditorState,
    request: &PlaceRequest,
    template: &clockwork_core::place::SceneTemplate,
) -> NodeId {
    let plugin = request.item.plugin.clone();
    let title = request.item.output.replace(['_', '-'], " ");
    let scene = &mut state.workshop.scene;
    let root = scene.add(
        request.parent,
        format!("{} · {plugin}", capitalize(&title)),
        None,
        Vec3::ZERO,
    );
    let mut groups: HashMap<String, NodeId> = HashMap::new();
    let mut behaviors = Vec::new();
    for node in &template.nodes {
        let parent = match &node.group {
            Some(group) => *groups
                .entry(group.clone())
                .or_insert_with(|| scene.add(root, group.clone(), None, Vec3::ZERO)),
            None => root,
        };
        let item = node.item.as_ref().map(|output| ItemRef {
            plugin: plugin.clone(),
            output: output.clone(),
        });
        let id = scene.add(parent, node.name.clone(), item, Vec3::from(node.position));
        if let Some(added) = scene.get_mut(id) {
            added.animation = node.animation.clone();
        }
        if let Some(door) = &node.door
            && let Some(added) = scene.get_mut(id)
        {
            added.behaviors = vec![clockwork_core::behavior::NodeBehavior::new(
                clockwork_core::behavior::Behavior::Door {
                    radius: door.radius,
                    to: door.to.clone(),
                    fade: door.fade,
                },
            )];
        }
        if let Some(behavior) = &node.behavior {
            behaviors.push((id, clockwork_core::asset::OutputRef::new(&plugin, behavior)));
        }
    }
    // What a placement does follows its plugin's behavior asset (an enemy's actor).
    for (id, at) in behaviors {
        let next = state
            .workshop
            .scene
            .get(id)
            .map_or(0, |n| n.behaviors.len());
        crate::asset_ui::link_behavior(state, id, next, Some(at));
    }
    state.workshop.select_only(root);
    root
}

/// Whether an item belongs on the ground of a world (characters, objects, abilities) rather
/// than being a world itself (areas and their tilesets).
fn drops_on_ground(state: &EditorState, item: &ItemRef) -> bool {
    let kind = state
        .workshop
        .plugin_index(&item.plugin)
        .and_then(|i| {
            state.workshop.plugins[i]
                .recipe
                .outputs
                .iter()
                .find_map(|o| o.item().filter(|(id, _)| *id == item.output))
        })
        .and_then(|(_, kind)| kind);
    !matches!(kind, Some(Kind::Area | Kind::Tileset))
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// Keeps the viewport in sync with the scene: rebuilds entities when the scene's structure
/// changes, copies transforms every frame, and saves the scene after changes.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn scene_system(
    mut state: ResMut<EditorState>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    // Editor shapes' materials, and extracted models' (drawn as their hardware drew them).
    (mut materials, mut model_materials): (
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<clockwork_runtime::retro_material::RetroMaterial>>,
    ),
    mut view: ResMut<SceneView>,
    mut extractions: ResMut<Extractions>,
    mut orbit: ResMut<OrbitCamera>,
    mut transforms: Query<(&SceneNode, &mut Transform, &mut Visibility)>,
    mut rigs: Query<&mut models::Rig>,
    mut sprite_rigs: Query<&mut sprites::SpriteRig>,
    mut shapes: Query<(&BehaviorShape, &mut Transform), Without<SceneNode>>,
    mut placeholders: Query<&mut Visibility, (With<Placeholder>, Without<SceneNode>)>,
    time: Res<Time>,
    settings: Res<Settings>,
    mut last_save: Local<f32>,
) {
    // The camera goes to a node the UI asked for (F).
    if let Some(id) = state.workshop.focus_request.take() {
        orbit.focus = state.workshop.scene.world_transform(id).translation;
    }

    // Plugins changed on disk (extraction_system threw their results away): rebuild.
    if state.workshop.plugins_revision != view.plugins_revision {
        view.plugins_revision = state.workshop.plugins_revision;
        view.textures.clear();
        view.models.clear();
        view.sprites.clear();
        view.overlays.clear();
        view.revision = 0;
    }
    let rebuild =
        state.workshop.scene.generation != view.seen || view.revision != extractions.revision;

    if rebuild {
        // On the very first build (a scene restored at startup), frame its first item.
        if view.seen == 0 {
            view.frame = state.workshop.scene.in_tree_order().into_iter().find(|id| {
                state
                    .workshop
                    .scene
                    .get(*id)
                    .is_some_and(|n| n.item.is_some())
            });
        }
        view.seen = state.workshop.scene.generation;
        view.revision = extractions.revision;
        if let Some(root) = view.entities.get(&ROOT) {
            commands.entity(*root).despawn();
        }
        view.entities.clear();

        let scene = &state.workshop.scene;
        let nodes: Vec<Node> = scene
            .in_tree_order()
            .iter()
            .filter_map(|id| scene.get(*id).cloned())
            .collect();
        let show_surfaces = state.workshop.show_surfaces;
        let mut framed = None;
        for node in nodes {
            let mut entity = commands.spawn((
                SceneNode(node.id),
                node.local_transform(),
                visibility(node.visible),
            ));
            if let Some(parent) = node.parent.and_then(|p| view.entities.get(&p)) {
                entity.insert(ChildOf(*parent));
            }
            let node_entity = entity.id();
            view.entities.insert(node.id, node_entity);

            let Some(item) = &node.item else {
                if let Some(behavior) = node.behaviors.first().map(|b| &b.behavior) {
                    spawn_behavior_shape(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        node.id,
                        behavior,
                        node_entity,
                    );
                }
                continue;
            };
            extractions.ensure(&mut state, &item.plugin);
            let Some(extracted) = extractions.get(&item.plugin) else {
                continue;
            };
            if let Some(m) = extracted.model(&item.output) {
                let (model, meters) = (&m.model, &m.meters_per_unit);
                let key = (item.plugin.clone(), item.output.clone());
                let built = view.models.entry(key).or_insert_with(|| {
                    models::build(model, &mut images, &mut meshes, &mut model_materials)
                });
                let size = built.size * *meters;
                models::spawn(
                    &mut commands,
                    built,
                    model,
                    *meters,
                    node.id,
                    &item.plugin,
                    &item.output,
                    node.animation.clone(),
                    node_entity,
                );
                if view.frame == Some(node.id) {
                    framed = Some((node.id, Vec2::splat(size.max_element() * 3.0)));
                }
                continue;
            }
            if let Some(s) = extracted.sprite(&item.output) {
                let key = (item.plugin.clone(), item.output.clone());
                let built = view
                    .sprites
                    .entry(key)
                    .or_insert_with(|| sprites::build(&s.sprite, s.pixels_per_meter, &mut images))
                    .clone();
                sprites::spawn(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &built,
                    node.id,
                    &item.plugin,
                    &item.output,
                    node.animation.clone(),
                    node_entity,
                );
                if view.frame == Some(node.id) {
                    framed = Some((node.id, Vec2::splat(8.0)));
                }
                continue;
            }
            let Some((_, image)) = extracted.images.iter().find(|(id, _)| *id == item.output)
            else {
                continue;
            };
            let key = (item.plugin.clone(), item.output.clone());
            let scale = NativeScale::tiles(GBA_SCALE);
            let size = Vec2::new(
                scale.to_meters(image.width as f32),
                scale.to_meters(image.height as f32),
            );
            let mesh = meshes.add(Plane3d::default().mesh().size(size.x, size.y));
            let texture = view
                .textures
                .entry(key.clone())
                .or_insert_with(|| images.add(to_bevy_image(image)))
                .clone();
            commands.spawn((
                NodeImage {
                    node: node.id,
                    size,
                },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(texture),
                    unlit: true,
                    alpha_mode: AlphaMode::Mask(0.5),
                    ..default()
                })),
                Transform::IDENTITY,
                ChildOf(node_entity),
            ));
            if let Some(map) = surfaces_for(extracted, &item.output) {
                let overlay = view
                    .overlays
                    .entry(key)
                    .or_insert_with(|| images.add(to_bevy_image(&surfaces::overlay(map))))
                    .clone();
                commands.spawn((
                    SurfaceOverlay,
                    Mesh3d(mesh),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(overlay),
                        unlit: true,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, 0.01, 0.0),
                    visibility(show_surfaces),
                    ChildOf(node_entity),
                ));
            }
            if view.frame == Some(node.id) {
                framed = Some((node.id, size));
            }
        }
        if let Some((id, size)) = framed {
            view.frame = None;
            let at = state.workshop.scene.world_transform(id).translation;
            *orbit = OrbitCamera {
                focus: at,
                yaw: 0.0,
                pitch: 1.1,
                distance: size.max_element() * 1.2,
            };
        }
    } else {
        // Same structure: copy transforms and visibility (the adjust panel edits them live).
        let by_id: HashMap<NodeId, &Node> = (state.workshop.scene.nodes.iter())
            .map(|n| (n.id, n))
            .collect();
        for (node, mut transform, mut vis) in &mut transforms {
            // Play mode moves the played node itself.
            if state.workshop.playing == Some(node.0) {
                continue;
            }
            if let Some(n) = by_id.get(&node.0) {
                if !state.workshop.acting.contains(&node.0) {
                    *transform = n.local_transform();
                }
                *vis = if state.workshop.hidden_in_play.contains(&node.0) {
                    Visibility::Hidden
                } else {
                    visibility(n.visible)
                };
            }
        }
        for (shape, mut transform) in &mut shapes {
            if let Some(behavior) = state
                .workshop
                .scene
                .get(shape.0)
                .and_then(|n| n.behaviors.first())
                .map(|b| &b.behavior)
            {
                transform.scale = behavior_shape_scale(behavior);
                transform.translation.y = transform.scale.y * 0.5;
            }
        }
        for mut rig in &mut rigs {
            let chosen = state.workshop.scene.get(rig.node).map(|n| &n.animation);
            if let Some(chosen) = chosen
                && *chosen != rig.animation
            {
                rig.animation = chosen.clone();
                rig.time = 0.0;
            }
        }
        for mut rig in &mut sprite_rigs {
            let chosen = state.workshop.scene.get(rig.node).map(|n| &n.animation);
            if let Some(chosen) = chosen
                && *chosen != rig.animation
            {
                rig.animation = chosen.clone();
                rig.time = 0.0;
            }
        }
    }

    let empty = !state.workshop.scene.nodes.iter().any(|n| n.item.is_some());
    for mut vis in &mut placeholders {
        *vis = visibility(empty);
    }

    // Save at most once a second after changes.
    let now = time.elapsed_secs();
    if state.player.is_some() {
        // An exported game never writes its scene back.
        state.workshop.scene.dirty = false;
    } else if settings.autosave_scene && state.workshop.scene.dirty && now - *last_save > 1.0 {
        *last_save = now;
        if let Err(e) = state.workshop.scene.save() {
            // It stays dirty: try again in ten seconds rather than every frame.
            *last_save = now + 9.0;
            state.log(LogLevel::Warn, format!("Could not save the scene: {e}"));
        }
    }
}

/// The size of a behavior's placeholder shape: a unit cylinder or sphere scaled to its radius.
fn behavior_shape_scale(behavior: &crate::entities::Behavior) -> Vec3 {
    use crate::entities::Behavior;
    let r = behavior.radius();
    match behavior {
        Behavior::Target { .. } => Vec3::new(r, 1.6, r),
        Behavior::Pickup { .. } => Vec3::splat(r * 1.0),
        Behavior::Solid { .. } => Vec3::new(r, 1.2, r),
        Behavior::Actor(_) => Vec3::new(r, 1.0, r),
        // A flat marker: these are mostly seen as rings, and invisible in play.
        Behavior::Trigger { .. }
        | Behavior::Checkpoint { .. }
        | Behavior::Hazard { .. }
        | Behavior::Door { .. } => Vec3::new(0.001, 0.001, 0.001),
    }
}

fn spawn_behavior_shape(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    node: NodeId,
    behavior: &crate::entities::Behavior,
    parent: Entity,
) {
    use crate::entities::Behavior;
    let [r, g, b] = behavior.color();
    let mesh = match behavior {
        Behavior::Pickup { .. } => meshes.add(Sphere::new(1.0)),
        _ => meshes.add(Cylinder::new(1.0, 1.0)),
    };
    let scale = behavior_shape_scale(behavior);
    commands.spawn((
        BehaviorShape(node),
        Mesh3d(mesh),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: 0.7,
            ..default()
        })),
        Transform::from_xyz(0.0, scale.y * 0.5, 0.0).with_scale(scale),
        ChildOf(parent),
    ));
}

fn visibility(visible: bool) -> Visibility {
    if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn to_bevy_image(image: &RgbaImage) -> Image {
    let mut bevy_image = Image::new(
        Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        image.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Keep pixel art crisp.
    bevy_image.sampler = ImageSampler::nearest();
    bevy_image
}

/// Shows or hides surface overlays, finds the surface cell under the cursor, and outlines the
/// selected node.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn surface_system(
    mut state: ResMut<EditorState>,
    extractions: Res<Extractions>,
    planes: Query<(&NodeImage, &GlobalTransform, &InheritedVisibility)>,
    mut overlays: Query<&mut Visibility, With<SurfaceOverlay>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<ViewportCamera>>,
    mut gizmos: Gizmos,
) {
    let shown = state.workshop.show_surfaces;
    for mut vis in &mut overlays {
        *vis = visibility(shown);
    }

    // Outline the selected node's image.
    if let Some(selected) = state.workshop.selected_node {
        for (plane, transform, _) in &planes {
            if plane.node == selected {
                let (scale, rotation, translation) = transform.to_scale_rotation_translation();
                let iso = Isometry3d::new(
                    translation + Vec3::Y * 0.02,
                    rotation * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                );
                gizmos.rect(iso, plane.size * scale.xz(), Color::srgb(0.89, 0.75, 0.42));
            }
        }
    }

    state.workshop.hovered_cell = None;
    if !state.workshop.viewport_hovered {
        return;
    }
    let (camera, camera_transform) = *camera;
    let Some(ray) = window
        .cursor_position()
        .and_then(|c| camera.viewport_to_world(camera_transform, c).ok())
    else {
        return;
    };
    // The nearest visible image under the cursor that has surfaces.
    let mut best: Option<(f32, NodeId, u32, u32)> = None;
    for (plane, transform, inherited) in &planes {
        if !inherited.get() {
            continue;
        }
        let Some(item) = state
            .workshop
            .scene
            .get(plane.node)
            .and_then(|n| n.item.clone())
        else {
            continue;
        };
        let Some(map) = extractions
            .get(&item.plugin)
            .and_then(|x| surfaces_for(x, &item.output))
        else {
            continue;
        };
        let up = transform.up();
        let Some(d) = ray.intersect_plane(transform.translation(), InfinitePlane3d::new(*up))
        else {
            continue;
        };
        let local = transform
            .affine()
            .inverse()
            .transform_point3(ray.get_point(d));
        // The image's top-left corner is at (-w/2, -h/2) in the plane, rows run along +Z.
        let u = (local.x + plane.size.x / 2.0) / plane.size.x;
        let v = (local.z + plane.size.y / 2.0) / plane.size.y;
        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) && best.is_none_or(|(bd, ..)| d < bd)
        {
            best = Some((
                d,
                plane.node,
                (u * map.width as f32) as u32,
                (v * map.height as f32) as u32,
            ));
        }
    }
    state.workshop.hovered_cell = best.map(|(_, node, x, y)| (node, x, y));
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn camera_system(
    state: Res<EditorState>,
    settings: Res<Settings>,
    input: Res<clockwork_runtime::input::GameInput>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<OrbitCamera>,
    mut dragging: Local<bool>,
    mut camera: Single<&mut Transform, With<ViewportCamera>>,
) {
    let any_down =
        buttons.any_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]);
    if buttons.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]) {
        *dragging = state.workshop.viewport_hovered && !state.workshop.gizmo_busy;
    } else if !any_down {
        *dragging = false;
    }

    if state.workshop.playing.is_some() && !input.blocked {
        // Same orbit and inversion settings as the mouse, independent of frame rate.
        let delta = input.look * (2.4 * settings.orbit_speed * time.delta_secs().min(0.1));
        orbit.yaw -= delta.x;
        let vertical = if settings.invert_orbit_y {
            delta.y
        } else {
            -delta.y
        };
        orbit.pitch = (orbit.pitch + vertical).clamp(0.05, 1.55);
    }

    if *dragging {
        let delta = motion.delta;
        if buttons.pressed(MouseButton::Left) {
            let speed = 0.006 * settings.orbit_speed;
            let vertical = if settings.invert_orbit_y {
                -delta.y
            } else {
                delta.y
            };
            orbit.yaw -= delta.x * speed;
            orbit.pitch = (orbit.pitch + vertical * speed).clamp(0.05, 1.55);
        } else {
            // Pan along the ground, relative to where the camera looks.
            let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
            let forward = Vec3::new(-orbit.yaw.sin(), 0.0, -orbit.yaw.cos());
            let speed = orbit.distance * 0.0016 * settings.pan_speed;
            orbit.focus += (-right * delta.x + forward * delta.y) * speed;
        }
    }

    if state.workshop.viewport_hovered && scroll.delta.y != 0.0 {
        let steps = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        let factor = 0.88f32.powf(steps * settings.zoom_speed);
        orbit.distance = (orbit.distance * factor).clamp(1.0, 500.0);
    }

    let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, -orbit.pitch, 0.0);
    let position = orbit.focus + rotation * Vec3::Z * orbit.distance;
    **camera = Transform::from_translation(position).looking_at(orbit.focus, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(name: &str, outputs: &str) -> Plugin {
        Plugin {
            dir: Default::default(),
            manifest: clockwork_core::PluginManifest::from_toml(&format!(
                "[plugin]\nname = \"{name}\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n[source]\ngame = \"g\"\nsha1 = []\n"
            ))
            .unwrap(),
            recipe: ron::from_str(&format!("(steps: [], outputs: [{outputs}])")).unwrap(),
            compose: None,
        }
    }

    const FLOW: &str =
        r#"Flow(id: "flow", flow: (files: Some((title: (pages: ["FILES"]), slots: 2))))"#;

    #[test]
    fn only_changed_plugins_and_those_using_them_are_extracted_again() {
        let uses_b = r#"Flow(id: "alias", flow: "b/flow")"#;
        let mut plugins = vec![plugin("a", uses_b), plugin("b", FLOW), plugin("c", FLOW)];
        assert_eq!(uses_plugins(&plugins[0]), ["b"]);
        assert!(uses_plugins(&plugins[1]).is_empty());

        let mut extractions = Extractions::default();
        extractions.invalidate(1, &plugins);
        for name in ["a", "b", "c"] {
            extractions
                .by_plugin
                .insert(name.into(), Ok(Arc::new(Extracted::default())));
        }
        extractions
            .by_plugin
            .insert("d".into(), Err("its game is not in the Library".into()));

        // Nothing changed: everything that worked is kept, failures are tried again.
        extractions.invalidate(2, &plugins);
        assert!(["a", "b", "c"].iter().all(|n| extractions.get(n).is_some()));
        assert!(extractions.error("d").is_none());

        // b changed: b and a (which uses b) go, c stays.
        plugins[1] = plugin("b", &FLOW.replace("slots: 2", "slots: 3"));
        let before = extractions.revision();
        extractions.invalidate(3, &plugins);
        assert!(extractions.revision() > before);
        assert!(extractions.get("a").is_none());
        assert!(extractions.get("b").is_none());
        assert!(extractions.get("c").is_some());

        // A plugin that is gone goes too.
        extractions
            .by_plugin
            .insert("b".into(), Ok(Arc::new(Extracted::default())));
        plugins.remove(1);
        extractions.invalidate(4, &plugins);
        assert!(extractions.get("b").is_none());
        assert!(extractions.get("c").is_some());
    }

    #[test]
    fn surfaces_go_with_the_image_they_name() {
        let map = |cells: u8| {
            let meaning = clockwork_core::surface::CodeMeaning {
                codes: [(cells as u16, vec![clockwork_core::SurfaceProperty::Solid])].into(),
                other: None,
            };
            SurfaceMap::build(1, 16, &[("g", &[cells as u16], &meaning)], vec![]).unwrap()
        };
        let image = || RgbaImage::new(16, 16);
        let extracted = Extracted {
            images: vec![("a".into(), image()), ("b".into(), image())],
            surfaces: vec![("a_s".into(), map(1)), ("b_s".into(), map(2))],
            surface_images: vec![("a_s".into(), "a".into()), ("b_s".into(), "b".into())],
            ..Default::default()
        };
        // Both maps fit both images; each goes with the one it names.
        assert_eq!(
            surfaces_for(&extracted, "b"),
            Some(&extracted.surfaces[1].1)
        );
        assert_eq!(
            surfaces_for(&extracted, "a"),
            Some(&extracted.surfaces[0].1)
        );
    }

    #[test]
    fn a_plugin_scene_becomes_a_group_of_nodes() {
        use clockwork_core::place::{Door, SceneNode, SceneTemplate};
        let mut state = EditorState {
            library: clockwork_library::Library::new(),
            missing: Vec::new(),
            imports: Default::default(),
            selected: None,
            search: String::new(),
            agent_prompt: String::new(),
            screen: Default::default(),
            log: Vec::new(),
            fetch_box_art: false,
            names: Default::default(),
            decomps: Default::default(),
            notes: Default::default(),
            decomp_input: String::new(),
            boxart_retry: Vec::new(),
            boxart_busy: false,
            workshop: Default::default(),
            agents: Vec::new(),
            claude_connected: false,
            player: None,
            plugin_editor: Default::default(),
            quit_asked: false,
            palette_request: None,
        };
        let node =
            |name: &str, group: Option<&str>, item: Option<&str>, door: Option<&str>| SceneNode {
                name: name.into(),
                group: group.map(Into::into),
                item: item.map(Into::into),
                position: [1.0, 0.0, 2.0],
                door: door.map(|to| Door {
                    to: to.into(),
                    radius: 0.5,
                    fade: 0.6,
                }),
                animation: item.map(|_| "still".to_string()),
                behavior: None,
            };
        let template = SceneTemplate {
            nodes: vec![
                node("Room", None, Some("room"), None),
                node("Slime", Some("Woods"), Some("slime"), None),
                node("Door", Some("Woods"), None, Some("arrive")),
            ],
        };
        let request = PlaceRequest {
            item: ItemRef {
                plugin: "p".into(),
                output: "world".into(),
            },
            parent: ROOT,
            at_cursor: false,
        };
        let root = add_scene(&mut state, &request, &template);
        let scene = &state.workshop.scene;
        let children = |id| {
            scene
                .nodes
                .iter()
                .filter(move |n| n.parent == Some(id))
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(scene.get(root).unwrap().name, "World · p");
        assert_eq!(children(root), ["Room", "Woods"]);
        let woods = scene.nodes.iter().find(|n| n.name == "Woods").unwrap().id;
        assert_eq!(children(woods), ["Slime", "Door"]);
        let slime = scene.nodes.iter().find(|n| n.name == "Slime").unwrap();
        assert_eq!(slime.item.as_ref().unwrap().output, "slime");
        assert_eq!(slime.position, [1.0, 0.0, 2.0]);
        assert_eq!(slime.animation.as_deref(), Some("still"));
        let door = scene.nodes.iter().find(|n| n.name == "Door").unwrap();
        assert!(matches!(
            &door.behaviors[0].behavior,
            clockwork_core::behavior::Behavior::Door { to, .. } if to == "arrive"
        ));
    }
}

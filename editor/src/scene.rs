//! The scene being composed in the Workshop: a tree of nodes, each with a transform and
//! optionally an item from a plugin (like Godot's scene tree).
//!
//! The scene is saved to `<data>/scenes/untitled.ron` whenever it changes, so it survives
//! restarts. It refers to plugin outputs by name and never contains game data.

use std::{collections::HashMap, path::PathBuf};

use bevy::math::{EulerRot, Quat, Vec3};
use bevy::transform::components::Transform;
use serde::{Deserialize, Serialize};

use clockwork_core::{
    asset::OutputRef,
    behavior::{Behavior, Condition, NodeBehavior},
};

use crate::store;

pub type NodeId = u64;

/// The root node; it always exists and can't be moved or removed.
pub const ROOT: NodeId = 0;

/// A plugin output a node shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    pub plugin: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "NodeFile")]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub parent: Option<NodeId>,
    pub item: Option<ItemRef>,
    /// Position in meters, relative to the parent. y is height.
    pub position: [f32; 3],
    /// Rotation around the vertical axis, in degrees.
    pub yaw: f32,
    pub scale: f32,
    pub visible: bool,
    /// Locked nodes can't be picked or moved in the viewport.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// The animation a model plays; `None` plays its first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    /// What the node does when played (a target, a pickup, a trigger...), each working while
    /// its condition holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub behaviors: Vec<NodeBehavior>,
    /// While playing, the node is there only once this holds (and from then on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appears: Option<Condition>,
    /// Acts (numbered from 1) in which the node is there; None means every act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acts: Option<Vec<u8>>,
    /// A camera asset used instead of the character's own when this node is played.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<OutputRef>,
    /// A HUD, UI theme, input map and menus used instead of the character's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hud: Option<OutputRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<OutputRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<OutputRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menus: Option<Vec<OutputRef>>,
    /// Stats and the items carried, instead of the character's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<OutputRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory: Option<OutputRef>,
}

/// A node as saved: also reads scenes from before nodes had several behaviors (`behavior` and
/// the asset it followed, `behavior_from`), which become the first of `behaviors`.
#[derive(Deserialize)]
struct NodeFile {
    id: NodeId,
    name: String,
    parent: Option<NodeId>,
    item: Option<ItemRef>,
    position: [f32; 3],
    yaw: f32,
    scale: f32,
    visible: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    animation: Option<String>,
    #[serde(default)]
    behaviors: Vec<NodeBehavior>,
    #[serde(default)]
    behavior: Option<Behavior>,
    #[serde(default)]
    behavior_from: Option<OutputRef>,
    #[serde(default)]
    appears: Option<Condition>,
    #[serde(default)]
    acts: Option<Vec<u8>>,
    #[serde(default)]
    camera: Option<OutputRef>,
    #[serde(default)]
    hud: Option<OutputRef>,
    #[serde(default)]
    ui: Option<OutputRef>,
    #[serde(default)]
    input: Option<OutputRef>,
    #[serde(default)]
    menus: Option<Vec<OutputRef>>,
    #[serde(default)]
    stats: Option<OutputRef>,
    #[serde(default)]
    inventory: Option<OutputRef>,
}

impl From<NodeFile> for Node {
    fn from(f: NodeFile) -> Self {
        let mut behaviors = f.behaviors;
        if let Some(behavior) = f.behavior {
            behaviors.insert(
                0,
                NodeBehavior {
                    from: f.behavior_from,
                    ..NodeBehavior::new(behavior)
                },
            );
        }
        Node {
            id: f.id,
            name: f.name,
            parent: f.parent,
            item: f.item,
            position: f.position,
            yaw: f.yaw,
            scale: f.scale,
            visible: f.visible,
            locked: f.locked,
            animation: f.animation,
            behaviors,
            appears: f.appears,
            acts: f.acts,
            camera: f.camera,
            hud: f.hud,
            ui: f.ui,
            input: f.input,
            menus: f.menus,
            stats: f.stats,
            inventory: f.inventory,
        }
    }
}

/// Settings of the game as a whole: used for every played character, unless a node says
/// otherwise; a character's own settings come after them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    /// A Game asset whose settings this game takes, where it sets none itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hud: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menus: Option<Vec<OutputRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<OutputRef>,
    /// What plays from the start, until an effect plays other music.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<OutputRef>,
}

impl Game {
    pub fn is_empty(&self) -> bool {
        *self == Game::default()
    }
}

impl Node {
    /// The node's transform relative to its parent.
    pub fn local_transform(&self) -> Transform {
        Transform {
            translation: Vec3::from(self.position),
            rotation: Quat::from_euler(EulerRot::YXZ, self.yaw.to_radians(), 0.0, 0.0),
            scale: Vec3::splat(self.scale),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub nodes: Vec<Node>,
    next_id: NodeId,
    /// The game's own settings.
    #[serde(default, skip_serializing_if = "Game::is_empty")]
    pub game: Game,
    /// Bumped on every structural change (nodes added, removed, reparented, hidden). Not saved.
    #[serde(skip)]
    pub generation: u64,
    /// Set on any change that should be saved. Not saved.
    #[serde(skip)]
    pub dirty: bool,
}

/// Save and rebuild flags must keep advancing when history is restored.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    nodes: Vec<Node>,
    next_id: NodeId,
    game: Game,
}

/// Settled edits become steps, so dragging and typing can be undone together.
#[derive(bevy::prelude::Resource)]
pub struct History {
    base: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    pub fn new(scene: &Scene) -> Self {
        Self {
            base: scene.snapshot(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Wait for an interaction to finish before recording its changes.
    pub fn track(&mut self, scene: &Scene, settled: bool) {
        if !settled {
            return;
        }
        let current = scene.snapshot();
        if current != self.base {
            let previous = std::mem::replace(&mut self.base, current);
            self.push_undo(previous);
            self.redo.clear();
        }
    }

    /// Preserve even an unfinished edit on redo before returning to the prior step.
    pub fn undo(&mut self, scene: &mut Scene) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(scene.snapshot());
        self.base = previous.clone();
        scene.restore(previous);
        true
    }

    pub fn redo(&mut self, scene: &mut Scene) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.push_undo(scene.snapshot());
        self.base = next.clone();
        scene.restore(next);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            nodes: vec![Node {
                id: ROOT,
                name: "Scene".into(),
                parent: None,
                item: None,
                position: [0.0; 3],
                yaw: 0.0,
                scale: 1.0,
                visible: true,
                locked: false,
                animation: None,
                behaviors: Vec::new(),
                appears: None,
                acts: None,
                camera: None,
                hud: None,
                ui: None,
                input: None,
                menus: None,
                stats: None,
                inventory: None,
            }],
            next_id: 1,
            game: Game::default(),
            generation: 1,
            dirty: false,
        }
    }
}

impl Scene {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            nodes: self.nodes.clone(),
            next_id: self.next_id,
            game: self.game.clone(),
        }
    }

    /// Restored content needs the same rebuild and save as a fresh edit.
    fn restore(&mut self, s: Snapshot) {
        self.nodes = s.nodes;
        self.next_id = s.next_id;
        self.game = s.game;
        self.changed();
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Every node's children, in the order they were added: one pass over the nodes, so walks
    /// of big scenes (a plugin's whole world) don't look every node up again for each node.
    fn child_map(&self) -> HashMap<NodeId, Vec<NodeId>> {
        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for node in &self.nodes {
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(node.id);
            }
        }
        children
    }

    /// The node and everything below it.
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let children = self.child_map();
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            if let Some(kids) = children.get(&out[i]) {
                out.extend(kids);
            }
            i += 1;
        }
        out
    }

    /// How deep each node is below the root (the root is 0).
    pub fn depths(&self) -> HashMap<NodeId, usize> {
        let mut depths = HashMap::from([(ROOT, 0)]);
        let parents: HashMap<NodeId, Option<NodeId>> =
            self.nodes.iter().map(|n| (n.id, n.parent)).collect();
        for id in self.in_tree_order() {
            let depth = parents
                .get(&id)
                .copied()
                .flatten()
                .and_then(|p| depths.get(&p))
                .map_or(0, |d| d + 1);
            depths.insert(id, depth);
        }
        depths
    }

    /// All nodes, parents before children (root first).
    pub fn in_tree_order(&self) -> Vec<NodeId> {
        self.subtree(ROOT)
    }

    pub fn add(
        &mut self,
        parent: NodeId,
        name: String,
        item: Option<ItemRef>,
        position: Vec3,
    ) -> NodeId {
        let parent = if self.get(parent).is_some() {
            parent
        } else {
            ROOT
        };
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(Node {
            id,
            name,
            parent: Some(parent),
            item,
            position: position.to_array(),
            yaw: 0.0,
            scale: 1.0,
            visible: true,
            locked: false,
            animation: None,
            behaviors: Vec::new(),
            appears: None,
            acts: None,
            camera: None,
            hud: None,
            ui: None,
            input: None,
            menus: None,
            stats: None,
            inventory: None,
        });
        self.changed();
        id
    }

    /// The nodes of `ids` that are not below another of them, in tree order: what an operation
    /// on a selection acts on (moving a parent already moves its children).
    pub fn top_level(&self, ids: &[NodeId]) -> Vec<NodeId> {
        let ids: Vec<NodeId> = ids.iter().copied().filter(|id| *id != ROOT).collect();
        let ids = &ids[..];
        self.in_tree_order()
            .into_iter()
            .filter(|id| *id != ROOT && ids.contains(id))
            .filter(|id| {
                let mut parent = self.get(*id).and_then(|n| n.parent);
                while let Some(p) = parent {
                    if ids.contains(&p) {
                        return false;
                    }
                    parent = self.get(p).and_then(|n| n.parent);
                }
                true
            })
            .collect()
    }

    /// Copies of the nodes (and everything below them), for pasting later. Ids are the
    /// originals; `paste` gives them new ones.
    pub fn copy(&self, ids: &[NodeId]) -> Vec<Node> {
        let tops = self.top_level(ids);
        self.in_tree_order()
            .into_iter()
            .filter(|id| tops.iter().any(|t| self.subtree(*t).contains(id)))
            .filter_map(|id| self.get(id).cloned())
            .collect()
    }

    /// Puts copied nodes under `parent`, with new ids; the copies' top nodes move by `offset`
    /// (in their parent's space) and get " copy" in their names if `rename`. Returns the new top
    /// nodes.
    pub fn paste(
        &mut self,
        nodes: &[Node],
        parent: NodeId,
        offset: Vec3,
        rename: bool,
    ) -> Vec<NodeId> {
        let parent = if self.get(parent).is_some() {
            parent
        } else {
            ROOT
        };
        let mut map: Vec<(NodeId, NodeId)> = Vec::new();
        let mut tops = Vec::new();
        for node in nodes {
            let id = self.next_id;
            self.next_id += 1;
            map.push((node.id, id));
            let inside = node
                .parent
                .and_then(|p| map.iter().find(|(old, _)| *old == p))
                .map(|(_, new)| *new);
            let mut copy = node.clone();
            copy.id = id;
            copy.parent = Some(inside.unwrap_or(parent));
            if inside.is_none() {
                copy.position = (Vec3::from(copy.position) + offset).to_array();
                if rename {
                    copy.name = format!("{} copy", copy.name);
                }
                tops.push(id);
            }
            self.nodes.push(copy);
        }
        self.changed();
        tops
    }

    /// Duplicates a node with everything below it, next to the original and moved by `offset`.
    pub fn duplicate(&mut self, id: NodeId, offset: Vec3) -> Option<NodeId> {
        let parent = self.get(id)?.parent?;
        let copy = self.copy(&[id]);
        self.paste(&copy, parent, offset, true).first().copied()
    }

    /// Puts the nodes into a new group at their common center (they stay where they are in the
    /// world). The group goes under the first node's parent.
    pub fn group(&mut self, ids: &[NodeId]) -> Option<NodeId> {
        let tops = self.top_level(ids);
        let first = *tops.first()?;
        let parent = self.get(first)?.parent.unwrap_or(ROOT);
        let center = tops
            .iter()
            .map(|id| self.world_transform(*id).translation)
            .sum::<Vec3>()
            / tops.len() as f32;
        let local = self
            .world_transform(parent)
            .compute_affine()
            .inverse()
            .transform_point3(center);
        let group = self.add(parent, "Group".into(), None, local);
        for id in tops {
            self.reparent(id, group);
        }
        Some(group)
    }

    /// Removes a node and everything below it. The root can't be removed.
    pub fn remove(&mut self, id: NodeId) {
        if id == ROOT {
            return;
        }
        let gone = self.subtree(id);
        self.nodes.retain(|n| !gone.contains(&n.id));
        self.changed();
    }

    /// Moves `id` under `parent`, keeping where it is in the world. Refuses to move the root
    /// or to put a node under itself or its own children.
    pub fn reparent(&mut self, id: NodeId, parent: NodeId) -> bool {
        if id == ROOT || self.get(parent).is_none() || self.subtree(id).contains(&parent) {
            return false;
        }
        if self.get(id).and_then(|n| n.parent) == Some(parent) {
            return true;
        }
        let world = self.world_transform(id);
        let parent_world = self.world_transform(parent);
        let local = parent_world.compute_affine().inverse() * world.compute_affine();
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        let Some(node) = self.get_mut(id) else {
            return false;
        };
        node.parent = Some(parent);
        node.position = translation.to_array();
        node.yaw = rotation.to_euler(EulerRot::YXZ).0.to_degrees();
        node.scale = scale.x;
        self.changed();
        true
    }

    /// The node's transform in the world, combining all its parents.
    pub fn world_transform(&self, id: NodeId) -> Transform {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(node) = self.get(c) else { break };
            chain.push(node.local_transform());
            current = node.parent;
        }
        chain
            .into_iter()
            .rev()
            .fold(Transform::IDENTITY, |acc, t| acc.mul_transform(t))
    }

    /// Whether the node and all its parents are visible.
    pub fn is_shown(&self, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(c) = current {
            match self.get(c) {
                Some(node) if node.visible => current = node.parent,
                Some(_) => return false,
                None => return true,
            }
        }
        true
    }

    /// Marks a structural change: the viewport rebuilds and the scene is saved.
    pub fn changed(&mut self) {
        self.generation += 1;
        self.dirty = true;
    }

    fn path() -> Option<PathBuf> {
        store::data_dir().map(|d| d.join("scenes").join("untitled.ron"))
    }

    /// The saved scene (empty if there is none), and why it couldn't be read if it couldn't.
    /// An unreadable file is set aside, so the next autosave doesn't overwrite it.
    pub fn load_reporting() -> (Self, Option<String>) {
        let Some(path) = Self::path() else {
            return (Self::default(), None);
        };
        match clockwork_library::files::load_ron::<Scene>(&path) {
            Ok(None) => (Self::default(), None),
            Ok(Some(scene)) if scene.get(ROOT).is_some() => (scene.loaded(), None),
            Ok(Some(_)) => {
                let kept = match clockwork_library::files::set_aside(&path) {
                    Some(kept) => format!("it was kept as {}", kept.display()),
                    None => "it could not be moved aside".into(),
                };
                let problem = format!("{} has no root node; {kept}", path.display());
                (Self::default(), Some(problem))
            }
            Err(problem) => (Self::default(), Some(problem)),
        }
    }

    /// A scene from a file, if it holds a valid one.
    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| ron::from_str::<Scene>(&text).ok())
            .filter(|s| s.get(ROOT).is_some())
            .map(Self::loaded)
    }

    fn loaded(mut self) -> Self {
        self.generation = 1;
        self
    }

    /// Writes the scene. It stays dirty if that fails, so a later save tries again.
    pub fn save(&mut self) -> Result<(), String> {
        let path = Self::path().ok_or("no data directory")?;
        let text =
            ron::ser::to_string_pretty(self, Default::default()).map_err(|e| e.to_string())?;
        clockwork_library::files::write_atomic(&path, text).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_add_undo_redo_preserves_id() {
        let mut scene = Scene::default();
        let initial = scene.snapshot();
        let mut history = History::new(&scene);
        let id = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        let added = scene.snapshot();
        history.track(&scene, true);
        assert!(history.can_undo());
        assert!(history.undo(&mut scene));
        assert_eq!(scene.snapshot(), initial);
        assert!(history.can_redo());
        assert!(history.redo(&mut scene));
        assert_eq!(scene.snapshot(), added);
        assert_eq!(scene.get(id).unwrap().name, "a");
    }

    #[test]
    fn history_coalesces_unsettled_changes() {
        let mut scene = Scene::default();
        let initial = scene.snapshot();
        let mut history = History::new(&scene);
        let id = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        history.track(&scene, false);
        scene.get_mut(id).unwrap().position = [1.0, 2.0, 3.0];
        history.track(&scene, false);
        scene.get_mut(id).unwrap().name = "renamed".into();
        history.track(&scene, false);
        assert!(!history.can_undo());
        history.track(&scene, true);
        history.track(&scene, true);
        assert!(history.undo(&mut scene));
        assert_eq!(scene.snapshot(), initial);
        assert!(!history.undo(&mut scene));
    }

    #[test]
    fn history_new_change_clears_redo() {
        let mut scene = Scene::default();
        let mut history = History::new(&scene);
        scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        history.track(&scene, true);
        assert!(history.undo(&mut scene));
        history.track(&scene, true);
        assert!(history.can_redo());
        scene.add(ROOT, "b".into(), None, Vec3::ZERO);
        history.track(&scene, false);
        assert!(history.can_redo());
        history.track(&scene, true);
        assert!(!history.can_redo());
        assert!(!history.redo(&mut scene));
    }

    #[test]
    fn history_empty_operations_do_nothing() {
        let mut scene = Scene::default();
        let mut history = History::new(&scene);
        let initial = scene.snapshot();
        let generation = scene.generation;
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.undo(&mut scene));
        assert!(!history.redo(&mut scene));
        assert_eq!(scene.snapshot(), initial);
        assert_eq!(scene.generation, generation);
        assert!(!scene.dirty);
    }

    #[test]
    fn history_keeps_only_the_latest_100_steps() {
        let mut scene = Scene::default();
        let mut history = History::new(&scene);
        for step in 1..=105 {
            scene.get_mut(ROOT).unwrap().name = step.to_string();
            history.track(&scene, true);
        }
        for step in (5..105).rev() {
            assert!(history.undo(&mut scene));
            assert_eq!(scene.get(ROOT).unwrap().name, step.to_string());
        }
        assert!(!history.undo(&mut scene));
        for step in 6..=105 {
            assert!(history.redo(&mut scene));
            assert_eq!(scene.get(ROOT).unwrap().name, step.to_string());
        }
        assert!(!history.redo(&mut scene));
        assert_eq!(history.undo.len(), 100);
    }

    #[test]
    fn history_restores_trigger_rebuild_and_save() {
        let mut scene = Scene::default();
        let mut history = History::new(&scene);
        scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        history.track(&scene, true);
        let generation = scene.generation;
        scene.dirty = false;
        assert!(history.undo(&mut scene));
        assert_eq!(scene.generation, generation + 1);
        assert!(scene.dirty);
        scene.dirty = false;
        assert!(history.redo(&mut scene));
        assert_eq!(scene.generation, generation + 2);
        assert!(scene.dirty);
        history.track(&scene, true);
        assert!(history.undo(&mut scene));
        assert!(!history.can_undo());
    }

    #[test]
    fn history_undo_preserves_unsettled_edit_on_redo() {
        let mut scene = Scene::default();
        let initial = scene.snapshot();
        let mut history = History::new(&scene);
        let id = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        history.track(&scene, true);
        scene.get_mut(id).unwrap().name = "unfinished".into();
        history.track(&scene, false);
        let unfinished = scene.snapshot();
        assert!(history.undo(&mut scene));
        assert_eq!(scene.snapshot(), initial);
        assert!(!history.can_undo());
        assert!(history.redo(&mut scene));
        assert_eq!(scene.snapshot(), unfinished);
    }

    fn item(name: &str) -> Option<ItemRef> {
        Some(ItemRef {
            plugin: "p".into(),
            output: name.into(),
        })
    }

    #[test]
    fn add_remove_and_tree_order() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), item("a"), Vec3::ZERO);
        let b = scene.add(a, "b".into(), item("b"), Vec3::ZERO);
        let c = scene.add(ROOT, "c".into(), None, Vec3::ZERO);
        assert_eq!(scene.in_tree_order(), vec![ROOT, a, c, b]);
        scene.remove(a);
        assert!(scene.get(b).is_none());
        assert_eq!(scene.in_tree_order(), vec![ROOT, c]);
        scene.remove(ROOT);
        assert!(scene.get(ROOT).is_some());
    }

    #[test]
    fn reparent_keeps_world_position_and_refuses_cycles() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::new(10.0, 0.0, 0.0));
        let b = scene.add(ROOT, "b".into(), None, Vec3::new(12.0, 0.0, 5.0));
        assert!(scene.reparent(b, a));
        assert_eq!(scene.get(b).unwrap().parent, Some(a));
        let world = scene.world_transform(b).translation;
        assert!((world - Vec3::new(12.0, 0.0, 5.0)).length() < 1e-4);
        assert!(
            (Vec3::from(scene.get(b).unwrap().position) - Vec3::new(2.0, 0.0, 5.0)).length() < 1e-4
        );

        assert!(!scene.reparent(a, b)); // a can't go under its own child
        assert!(!scene.reparent(ROOT, a));
        assert!(!scene.reparent(a, a));
    }

    #[test]
    fn hidden_parents_hide_children() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        let b = scene.add(a, "b".into(), None, Vec3::ZERO);
        scene.get_mut(a).unwrap().visible = false;
        assert!(!scene.is_shown(b));
    }

    #[test]
    fn top_level_drops_nodes_below_other_selected_ones() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        let b = scene.add(a, "b".into(), None, Vec3::ZERO);
        let c = scene.add(ROOT, "c".into(), None, Vec3::ZERO);
        assert_eq!(scene.top_level(&[b, a, c, ROOT]), vec![a, c]);
        assert_eq!(scene.top_level(&[b]), vec![b]);
    }

    #[test]
    fn duplicate_copies_the_subtree_with_new_ids() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), item("a"), Vec3::new(1.0, 0.0, 0.0));
        let b = scene.add(a, "b".into(), item("b"), Vec3::new(0.0, 2.0, 0.0));
        let copy = scene.duplicate(a, Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert_ne!(copy, a);
        assert_eq!(scene.get(copy).unwrap().name, "a copy");
        assert_eq!(scene.get(copy).unwrap().position, [2.0, 0.0, 0.0]);
        let kids: Vec<NodeId> = (scene.nodes.iter())
            .filter(|n| n.parent == Some(copy))
            .map(|n| n.id)
            .collect();
        assert_eq!(kids.len(), 1);
        assert_ne!(kids[0], b);
        assert_eq!(scene.get(kids[0]).unwrap().name, "b");
        // The child keeps its place inside the copy and isn't moved or renamed again.
        assert_eq!(scene.get(kids[0]).unwrap().position, [0.0, 2.0, 0.0]);
        assert_eq!(scene.nodes.len(), 5);
        assert!(scene.duplicate(ROOT, Vec3::ZERO).is_none());
    }

    #[test]
    fn copy_and_paste_into_another_parent() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::ZERO);
        let b = scene.add(ROOT, "b".into(), None, Vec3::ZERO);
        let target = scene.add(ROOT, "t".into(), None, Vec3::new(5.0, 0.0, 0.0));
        let clip = scene.copy(&[b, a]);
        assert_eq!(clip.len(), 2);
        let pasted = scene.paste(&clip, target, Vec3::ZERO, false);
        assert_eq!(pasted.len(), 2);
        assert!(
            pasted
                .iter()
                .all(|id| scene.get(*id).unwrap().parent == Some(target))
        );
        assert_eq!(scene.get(pasted[0]).unwrap().name, "a");
    }

    #[test]
    fn grouping_keeps_the_world_positions() {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::new(2.0, 0.0, 0.0));
        let b = scene.add(ROOT, "b".into(), None, Vec3::new(6.0, 0.0, 4.0));
        let g = scene.group(&[a, b]).unwrap();
        assert_eq!(scene.get(a).unwrap().parent, Some(g));
        assert!((scene.get(g).unwrap().position[0] - 4.0).abs() < 1e-4);
        let world = scene.world_transform(b).translation;
        assert!((world - Vec3::new(6.0, 0.0, 4.0)).length() < 1e-4);
        assert!(scene.group(&[]).is_none());
    }

    #[test]
    fn scenes_from_before_several_behaviors_still_load() {
        let old = r#"(nodes: [
            (id: 0, name: "Scene", parent: None, item: None, position: (0.0, 0.0, 0.0), yaw: 0.0,
             scale: 1.0, visible: true),
            (id: 1, name: "Coin", parent: Some(0), item: None, position: (0.0, 0.0, 0.0),
             yaw: 0.0, scale: 1.0, visible: true,
             behavior: Some(Pickup(item: "coin", count: 1, radius: 0.6)),
             behavior_from: Some((plugin: "kit", output: "coin"))),
        ], next_id: 2)"#;
        let scene: Scene = ron::from_str(old).unwrap();
        let coin = scene.get(1).unwrap();
        assert_eq!(coin.behaviors.len(), 1);
        assert_eq!(coin.behaviors[0].behavior.kind(), "Pickup");
        assert_eq!(coin.behaviors[0].from, Some(OutputRef::new("kit", "coin")));
        // Saved again, it is a list.
        let text = ron::to_string(&scene).unwrap();
        assert!(text.contains("behaviors:") && !text.contains("behavior_from"));
        assert_eq!(ron::from_str::<Scene>(&text).unwrap().get(1), Some(coin));
    }

    #[test]
    fn round_trips_through_ron() {
        let mut scene = Scene::default();
        scene.add(ROOT, "a".into(), item("area"), Vec3::new(1.0, 2.0, 3.0));
        let text = ron::to_string(&scene).unwrap();
        let back: Scene = ron::from_str(&text).unwrap();
        assert_eq!(back.nodes, scene.nodes);
    }
}

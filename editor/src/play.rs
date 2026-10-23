//! Play mode: drive a character around the scene with the runtime's generic controller.
//!
//! The scene itself is never changed while playing: the played node's entity is moved directly,
//! and stopping lets the scene's own transform take over again.

use bevy::{math::Affine3A, prelude::*};
use clockwork_core::{
    action::Moves,
    character::Character,
    character2d::Character2d,
    character2d::Direction4,
    game::{self, Goal, Rules, WhenDown},
    item::Slot,
    parts::{self, Stats},
    surface::SurfaceMap,
    surface::SurfaceProperty,
    ui::Words,
};
use clockwork_recipe::assets::AssetIndex;
use clockwork_runtime::{
    billboard::{self, Body25d},
    controller::{self, Body, Ground, Input, World},
    hud::HudRequest,
    input::{GameButton, GameInput},
    inventory::Inventory,
    terrain::WorldTerrain,
    topdown::{self, Body2d},
};

use clockwork_core::{asset, camera, character::FollowCamera, recipe::Output};

use crate::{
    entities::{Effect, Entities, Touch},
    fighting::{Buttons, Fighter},
    models::Rig,
    scene::NodeId,
    sprites::SpriteRig,
    state::{EditorState, LogLevel},
    terrains::SceneTerrains,
    workshop::{Extractions, NodeImage, OrbitCamera, SceneView, surfaces_for},
};

/// The simulation steps at a fixed rate, so movement doesn't depend on the frame rate.
const STEP: f32 = 1.0 / 60.0;

/// The camera looks at this point above the character's feet.
const CAMERA_HEIGHT: f32 = 0.8;

#[derive(Resource, Default)]
pub struct PlaySession {
    /// The played character, while playing.
    played: Option<Played>,
    /// What the scene's behaviors are doing while playing.
    entities: Entities,
    /// A dialog to show once its plugin is extracted: one whose pages come from the ROM.
    waiting_dialog: Option<clockwork_core::asset::OutputRef>,
}

/// The played character: what every kind has (its fighting, health and time), and its body,
/// which its movement asset's kind decides.
struct Played {
    node: NodeId,
    /// Simulation time not stepped yet.
    leftover: f32,
    fighter: Fighter,
    vitals: Vitals,
    body: PlayedBody,
    /// Where it started.
    origin: Vec3,
    rules: Rules,
    /// Lives and seconds left, when the rules count them.
    lives: Option<u32>,
    time_left: Option<f32>,
    /// A door's fade going on.
    fade: Option<Fade>,
    /// The goal is reached: the game stands still.
    won: bool,
}

/// The screen fading out and in on the way through a door.
struct Fade {
    elapsed: f32,
    total: f32,
    to: Vec3,
    moved: bool,
}

impl Fade {
    /// How dark the screen is, 0 to 1: dark halfway.
    fn darkness(&self) -> f32 {
        1.0 - (self.elapsed / (self.total / 2.0) - 1.0).abs().min(1.0)
    }
}

/// Health while playing, and how long the character can't be hurt again.
#[derive(Clone, Copy, Debug)]
pub struct Vitals {
    pub health: f32,
    pub max: f32,
    pub hurt_time: f32,
    /// Seconds left in which it can't be hurt.
    pub safe: f32,
}

impl Vitals {
    fn new(stats: Stats) -> Self {
        Self {
            health: stats.health,
            max: stats.health,
            hurt_time: stats.hurt_time,
            safe: 0.0,
        }
    }

    /// Takes `damage` unless just hurt; returns whether health ran out.
    pub fn hurt(&mut self, damage: f32) -> bool {
        if self.safe > 0.0 || damage <= 0.0 {
            return false;
        }
        self.health = (self.health - damage).max(0.0);
        self.safe = self.hurt_time;
        self.health <= 0.0
    }

    pub fn heal(&mut self, amount: f32) {
        self.health = (self.health + amount).min(self.max);
    }
}

/// How the played character moves and is shown.
enum PlayedBody {
    /// A 3D character running and jumping.
    Model(Box<ModelBody>),
    /// A sprite walking on a map seen from above.
    TopDown(Box<SpriteBody>),
    /// A sprite standing upright in a 3D world (2.5D), walking on its floors.
    Billboard(Box<BillboardBody>),
}

struct ModelBody {
    body: Body,
    character: Character,
    /// A jump press waiting for the next simulation step. Frames often render without a
    /// step (the display is faster than the simulation), and a press must not be lost then.
    jump_pending: bool,
    /// The bone the character leans with.
    lean_bone: Option<usize>,
    /// The action shown last, so a new one starts its animation from the beginning.
    last_action: Option<String>,
    /// The camera following it, if any (see [`played_camera`]).
    camera: Option<FollowCamera>,
}

struct SpriteBody {
    body: Body2d,
    character: Character2d,
    /// Where it starts again.
    start: Vec3,
}

struct BillboardBody {
    body: Body25d,
    character: Character2d,
}

/// One simulation step's input.
struct StepInput {
    /// The stick as seen on screen, and the ground direction it points.
    stick: Vec2,
    direction: Vec2,
    walk: bool,
    jump: bool,
    held: bool,
    /// Where the camera looks.
    yaw: f32,
}

impl PlayedBody {
    fn position(&self) -> Vec3 {
        match self {
            PlayedBody::Model(m) => m.body.position,
            PlayedBody::TopDown(s) => s.body.position,
            PlayedBody::Billboard(b) => b.body.position,
        }
    }

    fn radius(&self) -> f32 {
        match self {
            PlayedBody::Model(m) => m.character.movement.radius,
            PlayedBody::TopDown(s) => s.character.movement.radius,
            PlayedBody::Billboard(b) => b.character.movement.radius,
        }
    }

    fn teleport(&mut self, at: Vec3) {
        match self {
            PlayedBody::Model(m) => m.body.position = at,
            PlayedBody::TopDown(s) => s.body.position = at,
            PlayedBody::Billboard(b) => b.body.position = at,
        }
    }

    /// Where it starts again from now on.
    fn checkpoint(&mut self, at: Vec3) {
        match self {
            PlayedBody::Model(m) => m.body.start = at,
            PlayedBody::TopDown(s) => s.start = at,
            PlayedBody::Billboard(b) => b.body.start = at,
        }
    }

    fn start_over(&mut self) {
        match self {
            PlayedBody::Model(m) => m.body.position = m.body.start,
            PlayedBody::TopDown(s) => s.body.position = s.start,
            PlayedBody::Billboard(b) => b.body.position = b.body.start,
        }
    }

    /// Advances one simulation step: the action first (it says how movement is constrained),
    /// then the controller of its kind.
    fn step(&mut self, fighter: &mut Fighter, world: &SceneWorld, input: &StepInput) {
        match self {
            PlayedBody::Billboard(b) => {
                let modifiers = fighter.step(
                    world,
                    STEP,
                    b.body.position,
                    b.body.facing,
                    b.body.walking,
                    input.yaw,
                );
                // The facing follows the stick as seen on screen, the movement the ground
                // direction.
                billboard::step_with(
                    &mut b.body,
                    input.stick,
                    input.direction,
                    &b.character.movement,
                    world,
                    STEP,
                    &modifiers,
                );
            }
            PlayedBody::TopDown(s) => {
                let modifiers = fighter.step(
                    world,
                    STEP,
                    s.body.position,
                    s.body.facing,
                    s.body.walking,
                    0.0,
                );
                topdown::step_with(
                    &mut s.body,
                    input.direction,
                    &s.character.movement,
                    world,
                    STEP,
                    &modifiers,
                );
            }
            PlayedBody::Model(m) => {
                // Actions see the character's own forward direction: facing north for a camera
                // that looks the way the character does (the camera at yaw + half a turn looks
                // along it).
                let modifiers = fighter.step(
                    world,
                    STEP,
                    m.body.position,
                    Direction4::North,
                    input.direction.length() > 0.0,
                    m.body.yaw + std::f32::consts::PI,
                );
                // What the action allows: slower, no input at all, or a slide whatever the input.
                let mut steer = input.direction * modifiers.speed_factor;
                if modifiers.locked {
                    steer = Vec2::ZERO;
                }
                if let Some(slide) = modifiers.slide {
                    steer = slide.normalize_or_zero();
                }
                let busy = modifiers.locked || modifiers.slide.is_some();
                let step_input = Input {
                    direction: steer,
                    walk: input.walk,
                    jump_pressed: m.jump_pending && !busy,
                    jump_held: (input.held || input.jump) && !busy,
                };
                m.jump_pending = false;
                controller::step(&mut m.body, &step_input, &m.character.movement, world, STEP);
            }
        }
    }
}

impl PlaySession {
    /// The scene's behaviors while playing (empty otherwise).
    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    /// Chooses an act and judges conditions with the current inventory, keeping play progress.
    pub fn set_act(&mut self, act: Option<u8>) {
        self.entities.set_act(act);
        let carried = |item: &str| {
            self.played
                .as_ref()
                .and_then(|p| p.fighter.inventory.as_ref())
                .map_or(0, |i| i.count(item))
        };
        self.entities.refresh(&carried);
    }

    /// The played character's fighting state, if one is played.
    pub fn fighter(&self) -> Option<&Fighter> {
        self.played.as_ref().map(|p| &p.fighter)
    }

    /// Whether the played character is in a 3D view: a 3D character, or a 2D one standing
    /// upright in a 3D world (2.5D), rather than a sprite lying on a map seen from above.
    pub fn in_3d(&self) -> bool {
        self.played
            .as_ref()
            .is_some_and(|p| !matches!(p.body, PlayedBody::TopDown(_)))
    }

    /// The played 3D character's node, what it holds and the bone it holds it at.
    pub fn held_at_hand(&self) -> Option<(NodeId, Vec<crate::sprites::Held>, Option<&str>)> {
        let played = self.played.as_ref()?;
        let PlayedBody::Model(m) = &played.body else {
            return None;
        };
        Some((
            played.node,
            played.fighter.held(),
            m.character.hand.as_deref(),
        ))
    }

    /// Where the played character stands.
    pub fn position(&self) -> Option<Vec3> {
        self.played.as_ref().map(|p| p.body.position())
    }

    /// Moves the played character (backing out of a course, a save file's start).
    pub fn teleport(&mut self, at: Vec3) {
        if let Some(played) = &mut self.played {
            played.body.teleport(at);
        }
    }

    /// Moves the played character to where the game starts, and makes it the place it starts
    /// over from.
    pub fn start_at(&mut self, at: Vec3) {
        if let Some(played) = &mut self.played {
            played.body.teleport(at);
            played.origin = at;
        }
    }

    /// What the played character carries, once its items are there.
    pub fn inventory(&self) -> Option<&Inventory> {
        self.played.as_ref()?.fighter.inventory.as_ref()
    }

    pub fn inventory_mut(&mut self) -> Option<&mut Inventory> {
        self.played.as_mut()?.fighter.inventory.as_mut()
    }

    /// The scene's behaviors while playing, to put a save file into.
    pub fn entities_mut(&mut self) -> &mut Entities {
        &mut self.entities
    }

    /// The played character's state in one line, for the remote control's reports.
    pub fn describe(&self) -> Option<String> {
        let played = self.played.as_ref()?;
        let f = &played.fighter;
        let slot = |s: Slot| {
            f.inventory
                .as_ref()
                .and_then(|i| i.equipped(s))
                .map_or("-".to_string(), |item| item.id.clone())
        };
        let tail = format!(
            " | health {}/{} | B {} A {} | action {} | started {} hits {} spawned {} bursts {} projectiles {}{}",
            played.vitals.health,
            played.vitals.max,
            slot(Slot::B),
            slot(Slot::A),
            f.pose.as_ref().map_or("-", |p| p.0.as_str()),
            recent(&f.stats.started),
            f.stats.hits,
            recent(&f.stats.spawned),
            f.stats.bursts,
            f.projectiles.iter().count(),
            if f.invulnerable { " invulnerable" } else { "" }
        );
        Some(match &played.body {
            PlayedBody::Billboard(s) => {
                let (animation, time) = billboard::pose(&s.body, &s.character.animations);
                format!(
                    "{:?} (billboard) speed {:.2} {} at ({:.2}, {:.2}, {:.2}) animation {}{} at {:.2} s{tail}",
                    s.body.facing,
                    s.body.speed,
                    if s.body.grounded {
                        "grounded"
                    } else {
                        "airborne"
                    },
                    s.body.position.x,
                    s.body.position.y,
                    s.body.position.z,
                    animation.name,
                    if animation.flip_x { " (mirrored)" } else { "" },
                    time
                )
            }
            PlayedBody::TopDown(s) => {
                let (animation, time) = topdown::pose(&s.body, &s.character.animations);
                format!(
                    "{:?} speed {:.2} at ({:.2}, {:.2}, {:.2}) animation {}{} at {:.2} s{tail}",
                    s.body.facing,
                    s.body.speed,
                    s.body.position.x,
                    s.body.position.y,
                    s.body.position.z,
                    animation.name,
                    if animation.flip_x { " (mirrored)" } else { "" },
                    time
                )
            }
            PlayedBody::Model(m) => {
                let b = &m.body;
                format!(
                    "{:?} speed {:.2} vertical {:.2} at ({:.2}, {:.2}, {:.2}) yaw {:.0}° animation {}{tail}",
                    b.state,
                    b.speed,
                    b.vertical,
                    b.position.x,
                    b.position.y,
                    b.position.z,
                    b.yaw.to_degrees(),
                    b.last_animation
                )
            }
        })
    }
}

/// Input from the remote control, used instead of the keyboard while `frames` lasts.
#[derive(Resource, Default)]
pub struct ScriptedInput {
    /// Stick relative to the camera: x right, y forward.
    pub stick: Vec2,
    pub walk: bool,
    pub jump: bool,
    /// Frames the jump button stays held after the press.
    pub jump_hold: u32,
    pub frames: u32,
    /// Frames the A and B buttons stay down after a press, and a dodge press waiting.
    pub a_hold: u32,
    pub b_hold: u32,
    pub a_press: bool,
    pub b_press: bool,
    pub dodge_press: bool,
}

/// An area of the scene the character can stand on: its image plane and surface map.
struct Area<'a> {
    to_local: Affine3A,
    size: Vec2,
    height: f32,
    map: &'a SurfaceMap,
}

struct SceneWorld<'a> {
    areas: Vec<Area<'a>>,
    terrains: Vec<&'a WorldTerrain>,
    /// Obstacles placed in the scene.
    entities: &'a Entities,
}

impl World for SceneWorld<'_> {
    fn ground(&self, x: f32, z: f32) -> Ground {
        if self.entities.blocked(Vec2::new(x, z)) {
            return Ground::Wall;
        }
        // The highest area under the point decides; outside every area there is nothing.
        let mut best: Option<(f32, Ground)> = None;
        for area in &self.areas {
            let local = area.to_local.transform_point3(Vec3::new(x, area.height, z));
            let u = (local.x + area.size.x / 2.0) / area.size.x;
            let v = (local.z + area.size.y / 2.0) / area.size.y;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let map = area.map;
            let (fx, fy) = (u * map.width as f32, v * map.height as f32);
            let ground = match map.kind_at(fx as u32, fy as u32) {
                None => Ground::Hole,
                Some(kind) => {
                    cell_ground(map, &kind.properties, fx.fract(), fy.fract(), area.height)
                }
            };
            if best.as_ref().is_none_or(|(h, _)| area.height > *h) {
                best = Some((area.height, ground));
            }
        }
        best.map_or(Ground::Hole, |(_, g)| g)
    }

    fn ground_at(&self, x: f32, y: f32, z: f32) -> Ground {
        if self.entities.blocked(Vec2::new(x, z)) {
            return Ground::Wall;
        }
        let flat = self.ground(x, z);
        // The highest floor of any 3D area at the body's height; holes only if there is no floor.
        let mut terrain: Option<Ground> = None;
        for found in self.terrains.iter().filter_map(|t| t.floor(x, y, z)) {
            terrain = match (terrain, found) {
                (Some(Ground::Floor(a)), Ground::Floor(b)) => Some(Ground::Floor(a.max(b))),
                (Some(Ground::Floor(a)), _) => Some(Ground::Floor(a)),
                (_, found) => Some(found),
            };
        }
        match (flat, terrain) {
            (Ground::Wall, _) => Ground::Wall,
            (Ground::Floor(a), Some(Ground::Floor(b))) => Ground::Floor(a.max(b)),
            (Ground::Floor(a), _) => Ground::Floor(a),
            (Ground::Hole, Some(found)) => found,
            (Ground::Hole, None) => Ground::Hole,
        }
    }

    fn push_out(&self, position: Vec3, radius: f32) -> Vec3 {
        self.terrains
            .iter()
            .fold(position, |p, t| t.push_out(p, radius))
    }
}

/// What a cell is at a point inside it (fx, fy in 0..1 across the cell).
fn cell_ground(
    map: &SurfaceMap,
    properties: &[SurfaceProperty],
    fx: f32,
    fy: f32,
    height: f32,
) -> Ground {
    let properties = map.resolve(properties);
    let mut ground = Ground::Floor(height);
    for p in &properties {
        match p {
            SurfaceProperty::Solid => return Ground::Wall,
            SurfaceProperty::SolidParts(mask) => {
                let bit = (fx >= 0.5) as u8 + 2 * (fy >= 0.5) as u8;
                if mask & (1 << bit) != 0 {
                    return Ground::Wall;
                }
            }
            SurfaceProperty::FallThrough => ground = Ground::Hole,
            _ => {}
        }
    }
    ground
}

/// Starts and stops Play mode, and moves the played character.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn play_system(
    mut state: ResMut<EditorState>,
    mut session: ResMut<PlaySession>,
    mut scripted: ResMut<ScriptedInput>,
    mut extractions: ResMut<Extractions>,
    terrains: Res<SceneTerrains>,
    view: Res<SceneView>,
    mut orbit: ResMut<OrbitCamera>,
    keys: Res<ButtonInput<KeyCode>>,
    input: Res<GameInput>,
    time: Res<Time>,
    planes: Query<(&NodeImage, &GlobalTransform, &InheritedVisibility)>,
    parents: Query<&GlobalTransform, Without<NodeImage>>,
    mut transforms: Query<&mut Transform>,
    mut rigs: Query<&mut Rig>,
    mut sprite_rigs: Query<&mut SpriteRig>,
    mut flow: ResMut<crate::flow_play::FlowState>,
) {
    match state.workshop.play_request.take() {
        Some(true) if session.played.is_none() => {
            start(
                &mut state,
                &mut session,
                &mut extractions,
                &terrains,
                &mut orbit,
            );
            // Its HUD, UI, menus and input map.
            if let Some(node) = state.workshop.playing {
                let kit = played_kit(&state, &extractions, node);
                state.workshop.screen = Some(crate::play_kit::screen(
                    &mut state,
                    &extractions,
                    node,
                    &kit,
                ));
            }
            // The game's screens: the title, its save files. A game with them is played in
            // the whole window.
            flow.begin(&mut state);
            state.workshop.game_view = flow.is_game();
        }
        Some(false) => {
            if let Some(played) = session.played.take() {
                for mut rig in &mut rigs {
                    if rig.node == played.node {
                        rig.playback = None;
                        rig.bend = None;
                        rig.time = 0.0;
                    }
                }
                for mut rig in &mut sprite_rigs {
                    if rig.node == played.node {
                        rig.playback = None;
                        rig.held.clear();
                        rig.time = 0.0;
                    }
                }
            }
            state.workshop.playing = None;
            state.workshop.game_view = false;
            state.workshop.hud = None;
            state.workshop.screen = None;
            state.workshop.health = None;
            state.workshop.lives = None;
            state.workshop.seconds = None;
            state.workshop.fade = 0;
            state.workshop.banner = None;
            state.workshop.dialog = None;
            state.workshop.kit_sounds = Default::default();
            state.workshop.menu_paused = false;
            state.workshop.hidden_in_play.clear();
            state.workshop.toast = None;
            state.workshop.play_clock = Default::default();
            session.entities = Entities::default();
        }
        _ => {}
    }
    let Some(played) = session.played.as_ref().map(|p| p.node) else {
        return;
    };
    // The theme's font and frame, and the items, once their plugins are extracted.
    crate::play_kit::complete_screen(&mut state, &mut extractions);
    if let Some(played) = &mut session.played {
        complete_inventory(&mut played.fighter, &mut extractions, &mut state);
    }
    if let Some(at) = session.waiting_dialog.clone() {
        extractions.ensure(&mut state, &at.plugin);
        if let Some(extracted) = extractions.get(&at.plugin) {
            match extracted.dialogs.iter().find(|(id, _)| *id == at.output) {
                Some((_, dialog)) => state.workshop.dialog = Some((dialog.clone(), 0)),
                None => state.log(LogLevel::Warn, format!("The dialog {at} has no pages")),
            }
            session.waiting_dialog = None;
        } else if !extractions.is_pending(&at.plugin) {
            state.log(
                LogLevel::Warn,
                format!("The dialog {at} can't be extracted"),
            );
            session.waiting_dialog = None;
        }
    }
    // The node or its item disappeared (deleted, undone): stop.
    if state.workshop.scene.get(played).is_none() {
        session.played = None;
        state.workshop.playing = None;
        state.workshop.game_view = false;
        return;
    }

    // Input: scripted input wins; otherwise the game's buttons. While a menu is open the game
    // stands still and the buttons belong to the menu.
    let typing = state.workshop.typing;
    // A menu, a dialog, a screen, a door's fade or a won game: the player has no say.
    let in_menu = state.workshop.menu_paused
        || state.workshop.dialog.is_some()
        || flow.frozen()
        || session
            .played
            .as_ref()
            .is_some_and(|p| p.fade.is_some() || p.won);
    let (stick, walk, jump, held) = if scripted.frames > 0 {
        scripted.frames -= 1;
        let jump = std::mem::take(&mut scripted.jump);
        let held = scripted.jump_hold > 0;
        scripted.jump_hold = scripted.jump_hold.saturating_sub(1);
        (scripted.stick, scripted.walk, jump, held)
    } else if in_menu {
        (Vec2::ZERO, false, false, false)
    } else {
        (
            input.stick,
            input.held(GameButton::Walk),
            input.pressed(GameButton::Jump),
            input.held(GameButton::Jump),
        )
    };
    // The item buttons and the dodge.
    let mut buttons = Buttons::default();
    if scripted.dodge_press || scripted.a_hold > 0 || scripted.b_hold > 0 {
        buttons.dodge = std::mem::take(&mut scripted.dodge_press);
        buttons.b = (std::mem::take(&mut scripted.b_press), scripted.b_hold > 0);
        buttons.a = (std::mem::take(&mut scripted.a_press), scripted.a_hold > 0);
        scripted.b_hold = scripted.b_hold.saturating_sub(1);
        scripted.a_hold = scripted.a_hold.saturating_sub(1);
    } else if !in_menu {
        buttons.dodge = input.pressed(GameButton::Dodge);
        buttons.b = (input.pressed(GameButton::B), input.held(GameButton::B));
        buttons.a = (input.pressed(GameButton::A), input.held(GameButton::A));
    }
    // Debug keys: H shows or hides the hit boxes, P pauses, . steps, [ and ] change the speed.
    // A key assigned to gameplay belongs to that action, including in exported games.
    let debug_pressed =
        |key| !typing && !input.blocked && !input.uses_key(key) && keys.just_pressed(key);
    if debug_pressed(KeyCode::KeyH) {
        state.workshop.hud_requests.push(HudRequest::ToggleHitboxes);
    }
    if !typing {
        let clock = &mut state.workshop.play_clock;
        if debug_pressed(KeyCode::KeyP) {
            clock.paused = !clock.paused;
        }
        if debug_pressed(KeyCode::Period) {
            clock.paused = true;
            clock.step_once = true;
        }
        for (key, factor) in [(KeyCode::BracketLeft, 0.5), (KeyCode::BracketRight, 2.0)] {
            if debug_pressed(key) {
                clock.speed = (clock.speed * factor).clamp(0.125, 8.0);
            }
        }
    }
    let sim_dt = state
        .workshop
        .play_clock
        .advance(time.delta_secs().min(0.25), STEP);
    let frozen = state.workshop.menu_paused
        || state.workshop.dialog.is_some()
        || flow.frozen()
        || session.played.as_ref().is_some_and(|p| p.won);
    let sim_dt = if frozen { 0.0 } else { sim_dt };
    if let Some((_, left)) = &mut state.workshop.toast {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            state.workshop.toast = None;
        }
    }
    let requests = std::mem::take(&mut state.workshop.hud_requests);
    // The pause menu's "Start over" and "Stop playing".
    let start_over = requests.contains(&HudRequest::Restart);
    // The dialog's next page, or closing it after the last.
    if requests.contains(&HudRequest::NextPage)
        && let Some((dialog, page)) = &mut state.workshop.dialog
    {
        *page += 1;
        if *page >= dialog.pages.len() {
            state.workshop.dialog = None;
        }
    }
    if requests.contains(&HudRequest::Stop) {
        state.workshop.play_request = Some(false);
    }
    // The stick is relative to the camera's view of the ground.
    let right = Vec2::new(orbit.yaw.cos(), -orbit.yaw.sin());
    let forward = Vec2::new(-orbit.yaw.sin(), -orbit.yaw.cos());
    let direction = right * stick.x + forward * stick.y;

    // The world: every visible area with surfaces.
    let areas = planes
        .iter()
        .filter(|(_, _, visible)| visible.get())
        .filter_map(|(plane, transform, _)| {
            let item = state.workshop.scene.get(plane.node)?.item.as_ref()?;
            let map = surfaces_for(extractions.get(&item.plugin)?, &item.output)?;
            Some(Area {
                to_local: transform.affine().inverse(),
                size: plane.size,
                height: transform.translation().y,
                map,
            })
        })
        .collect();
    state.workshop.live = live_lines(&session, &state);
    let session = &mut *session;
    let world = SceneWorld {
        areas,
        terrains: terrains.all(),
        entities: &session.entities,
    };
    let Some(played) = &mut session.played else {
        return;
    };

    // Every kind alike: the buttons, the steps, the scene's behaviors and what they did.
    if let PlayedBody::Model(m) = &mut played.body {
        m.jump_pending |= jump;
    }
    played.fighter.feed(&buttons, &requests);
    played.leftover += sim_dt;
    let step_input = StepInput {
        stick,
        direction,
        walk,
        jump,
        held,
        yaw: orbit.yaw,
    };
    while played.leftover >= STEP {
        played.leftover -= STEP;
        played.body.step(&mut played.fighter, &world, &step_input);
    }
    played.vitals.safe = (played.vitals.safe - sim_dt).max(0.0);
    let projectiles = played.fighter.moves.projectiles.clone();
    // Behind a screen (the title, choosing a file or an act) nothing is touched.
    let outcome = if flow.frozen() {
        EntityOutcome::default()
    } else {
        run_entities(
            &mut session.entities,
            Some(&mut played.fighter),
            &projectiles,
            played.body.position(),
            played.body.radius(),
            sim_dt,
            &mut state,
        )
    };
    if let Some(at) = outcome.teleport {
        played.body.teleport(at);
    }
    // Arriving in a course asks which act to play.
    if let Some(name) = &outcome.arrived {
        flow.arrived(name, played.body.position());
    }
    if outcome.dialog.is_some() {
        session.waiting_dialog = outcome.dialog;
    }
    if let Some(at) = outcome.checkpoint {
        played.body.checkpoint(at);
    }
    if outcome.restart || start_over {
        played.body.start_over();
    }
    played.vitals.heal(outcome.heal);
    // A door: the screen fades out, the player moves, it fades back in.
    if let Some((to, fade)) = outcome.door
        && played.fade.is_none()
    {
        played.fade = Some(Fade {
            elapsed: 0.0,
            total: fade.max(0.01),
            to,
            moved: false,
        });
    }
    if let Some(fade) = &mut played.fade {
        fade.elapsed += time.delta_secs();
        if !fade.moved && fade.elapsed >= fade.total / 2.0 {
            played.body.teleport(fade.to);
            fade.moved = true;
        }
    }
    state.workshop.fade = played
        .fade
        .as_ref()
        .map_or(0, |f| (f.darkness() * 16.0).round() as u8);
    if played.fade.as_ref().is_some_and(|f| f.elapsed >= f.total) {
        played.fade = None;
    }
    // The rules: out of health or out of time is going down.
    let health_before = played.vitals.health;
    let mut down = played.vitals.hurt(outcome.damage);
    if played.vitals.health < health_before {
        let sound = state.workshop.kit_sounds.hurt.clone();
        state.workshop.sounds.extend(sound);
    }
    if outcome.door.is_some() {
        let sound = state.workshop.kit_sounds.door.clone();
        state.workshop.sounds.extend(sound);
    }
    if outcome.picked {
        let sound = state.workshop.kit_sounds.pickup.clone();
        state.workshop.sounds.extend(sound);
    }
    if let Some(left) = &mut played.time_left {
        *left -= sim_dt;
        if *left <= 0.0 {
            down = true;
            *left = played.rules.time_limit.unwrap_or_default();
        }
    }
    if down {
        let game_over = played.lives.is_some_and(|l| l <= 1);
        if let Some(lives) = &mut played.lives {
            *lives = lives.saturating_sub(1);
        }
        played.vitals.health = played.vitals.max;
        if game_over && flow.game_over() {
            // The game's own game over screen; continuing starts Play again.
        } else if game_over {
            played.lives = played.rules.lives;
            played.time_left = played.rules.time_limit;
            played.body.teleport(played.origin);
            state.workshop.toast = toast(&words(&state).game_over);
        } else {
            match played.rules.when_down {
                WhenDown::StartOver => played.body.teleport(played.origin),
                WhenDown::LastCheckpoint => played.body.start_over(),
                WhenDown::StopPlaying => state.workshop.play_request = Some(false),
            }
            state.workshop.toast = toast(&words(&state).down);
        }
    }
    // The goal.
    if !played.won
        && let Some(goal) = &played.rules.goal
    {
        let reached = match goal {
            Goal::Collect { item, count } => played
                .fighter
                .inventory
                .as_ref()
                .is_some_and(|i| i.count(item) >= *count),
            Goal::Reach(name) => state
                .workshop
                .scene
                .nodes
                .iter()
                .find(|n| n.name == *name)
                .is_some_and(|n| {
                    let at = state.workshop.scene.world_transform(n.id).translation;
                    let here = played.body.position();
                    Vec2::new(at.x - here.x, at.z - here.z).length() <= played.body.radius() + 0.5
                }),
            Goal::DestroyAll => {
                let targets = session
                    .entities
                    .live
                    .iter()
                    .filter(|e| matches!(e.behavior, crate::entities::Behavior::Target { .. }));
                targets.clone().count() > 0 && targets.into_iter().all(|e| e.gone)
            }
            Goal::When(condition) => {
                let inventory = played.fighter.inventory.as_ref();
                session
                    .entities
                    .holds(condition, &|item| inventory.map_or(0, |i| i.count(item)))
            }
        };
        if reached {
            played.won = true;
            let win = words(&state).win;
            state.workshop.banner = (!win.is_empty()).then_some(win);
        }
    }
    state.workshop.lives = played.lives;
    state.workshop.seconds = played.time_left.map(|t| t.ceil().max(0.0) as u32);
    let fights = !played.fighter.moves.actions.is_empty() || played.fighter.inventory.is_some();
    state.workshop.hud = fights.then(|| played.fighter.hud());
    state.workshop.health = Some((played.vitals.health, played.vitals.max));

    // Shown as its kind is: the animation, the node's entity and the camera following.
    let node = played.node;
    let fighter = &played.fighter;
    let transform_of = |state: &EditorState, at: Affine3A| {
        let parent = state
            .workshop
            .scene
            .get(node)
            .and_then(|n| n.parent)
            .and_then(|p| view.entity(p))
            .and_then(|e| parents.get(e).ok())
            .map(|g| g.affine())
            .unwrap_or_default();
        let (scale, rotation, translation) =
            (parent.inverse() * at).to_scale_rotation_translation();
        let node_scale = state.workshop.scene.get(node).map_or(1.0, |n| n.scale);
        Transform {
            translation,
            rotation,
            scale: scale * node_scale,
        }
    };
    let dt = time.delta_secs();
    match &mut played.body {
        PlayedBody::Billboard(b) => {
            let (animation, seconds) = billboard::pose(&b.body, &b.character.animations);
            let shown =
                fighter
                    .pose
                    .clone()
                    .unwrap_or((animation.name.clone(), animation.flip_x, seconds));
            for mut rig in &mut sprite_rigs {
                if rig.node == node {
                    rig.playback = Some(shown.clone());
                    rig.upright = Some(orbit.yaw);
                    rig.held = fighter.held();
                }
            }
            // The billboard turns itself to the camera; the node keeps the world's orientation.
            if let Some(entity) = view.entity(node)
                && let Ok(mut transform) = transforms.get_mut(entity)
            {
                *transform = transform_of(&state, Affine3A::from_translation(b.body.position));
            }
            orbit.focus = orbit.focus.lerp(
                b.body.position + Vec3::Y * CAMERA_HEIGHT,
                1.0 - (-8.0 * dt).exp(),
            );
        }
        PlayedBody::TopDown(s) => {
            let (animation, seconds) = topdown::pose(&s.body, &s.character.animations);
            let shown =
                fighter
                    .pose
                    .clone()
                    .unwrap_or((animation.name.clone(), animation.flip_x, seconds));
            for mut rig in &mut sprite_rigs {
                if rig.node == node {
                    rig.playback = Some(shown.clone());
                    rig.held = fighter.held();
                }
            }
            // The sprite keeps the map's orientation (north up) whatever its node's rotation was.
            if let Some(entity) = view.entity(node)
                && let Ok(mut transform) = transforms.get_mut(entity)
            {
                *transform = transform_of(&state, Affine3A::from_translation(s.body.position));
            }
            orbit.focus = orbit.focus.lerp(s.body.position, 1.0 - (-8.0 * dt).exp());
            orbit.yaw = 0.0;
        }
        PlayedBody::Model(m) => {
            let finished = rigs
                .iter()
                .find(|r| r.node == node)
                .is_some_and(|r| r.finished);
            let pose = controller::pose(
                &mut m.body,
                &m.character.animations,
                &m.character.movement,
                finished,
            );
            let (mut animation, mut rate, mut restart, flipped) = (
                pose.animation.to_string(),
                pose.rate,
                pose.restart,
                pose.flipped,
            );
            // An action going on shows its own animation, from its start.
            let action = fighter.pose.as_ref().map(|(name, _, _)| name.clone());
            if let Some(name) = &action {
                restart |= m.last_action.as_ref() != Some(name);
                animation = name.clone();
                rate = 1.0;
            }
            if action.is_none() && m.last_action.is_some() {
                restart = true;
            }
            m.last_action = action;
            let bend = match (&m.character.lean, m.lean_bone) {
                (Some(lean), Some(bone)) => {
                    let (roll, pitch) = controller::lean(&mut m.body, lean, dt);
                    let rotation = Quat::from_axis_angle(
                        Vec3::from(lean.pitch_axis).normalize(),
                        pitch.to_radians(),
                    ) * Quat::from_axis_angle(
                        Vec3::from(lean.roll_axis).normalize(),
                        roll.to_radians(),
                    );
                    Some((bone, rotation))
                }
                _ => None,
            };
            for mut rig in &mut rigs {
                if rig.node == node {
                    if restart {
                        rig.time = 0.0;
                        rig.finished = false;
                    }
                    rig.playback = Some((animation.clone(), rate));
                    rig.bend = bend;
                }
            }
            let body = &m.body;
            let facing = body.yaw + if flipped { std::f32::consts::PI } else { 0.0 };
            if let Some(entity) = view.entity(node)
                && let Ok(mut transform) = transforms.get_mut(entity)
            {
                *transform = transform_of(
                    &state,
                    Affine3A::from_rotation_translation(
                        Quat::from_rotation_y(facing),
                        body.position,
                    ),
                );
            }
            let height = m.camera.as_ref().map_or(CAMERA_HEIGHT, |c| c.height);
            let target = body.position + Vec3::Y * height;
            orbit.focus = orbit.focus.lerp(target, 1.0 - (-8.0 * dt).exp());
            // Swing round behind the character, faster the faster it runs (the camera looks along
            // (-sin yaw, -cos yaw), so behind a character facing `yaw` is yaw + half a turn).
            if let Some(camera) = &m.camera {
                let behind = body.yaw + std::f32::consts::PI;
                let difference = (behind - orbit.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                let pace = (body.speed.abs() / m.character.movement.run_speed).min(1.0);
                orbit.yaw += difference * (1.0 - (-camera.follow * pace * dt).exp());
            }
        }
    }
}

fn start(
    state: &mut EditorState,
    session: &mut PlaySession,
    extractions: &mut Extractions,
    terrains: &SceneTerrains,
    orbit: &mut OrbitCamera,
) {
    let playable = |id: NodeId| {
        let item = state.workshop.scene.get(id)?.item.as_ref()?;
        let model = extractions.get(&item.plugin)?.model(&item.output)?;
        model.character.clone()
    };
    let playable_sprite = |id: NodeId| {
        let item = state.workshop.scene.get(id)?.item.as_ref()?;
        let sprite = extractions.get(&item.plugin)?.sprite(&item.output)?;
        sprite.character.clone()
    };
    let can_play = |id: NodeId| playable(id).is_some() || playable_sprite(id).is_some();
    let chosen = state
        .workshop
        .selected_node
        .filter(|id| can_play(*id))
        .or_else(|| {
            state
                .workshop
                .scene
                .in_tree_order()
                .into_iter()
                .find(|id| can_play(*id))
        });
    let Some(node) = chosen else {
        state.log(
            LogLevel::Warn,
            "Nothing to play: place a playable character first",
        );
        return;
    };
    let sprite = playable_sprite(node);
    let model = playable(node);
    let name = state
        .workshop
        .scene
        .get(node)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let world = state.workshop.scene.world_transform(node);
    {
        let scene = &state.workshop.scene;
        session.entities = Entities::build(scene, |id| scene.world_transform(id).translation);
    }
    state.workshop.hidden_in_play.clear();
    state.workshop.play_clock = Default::default();

    // Its body, by kind: a sprite walks on the flat map seen from above, or stands upright as a
    // billboard in a 3D world; a model runs and jumps. Each with its camera.
    let (body, moves, stats, inventory, how) = match (sprite, model) {
        (Some(character), _) if !terrains.is_empty() => {
            let mut position = world.translation;
            if !terrains.standing_on(position)
                && let Some((spawn, _)) = terrains.spawn()
            {
                position = spawn;
                state.log(LogLevel::Info, "Starting at the area's start point");
            }
            let camera = played_camera(
                state,
                node,
                character.world_camera.as_ref(),
                Some(camera::BILLBOARD),
                "top-down camera",
                asset::top_down_camera,
            )
            .unwrap_or_default();
            *orbit = OrbitCamera {
                focus: position + Vec3::Y * CAMERA_HEIGHT,
                yaw: 0.0,
                pitch: camera.pitch.to_radians().min(1.55),
                distance: camera.distance,
            };
            (
                PlayedBody::Billboard(Box::new(BillboardBody {
                    body: Body25d::new(position, Direction4::South),
                    character: character.clone(),
                })),
                character.moves(),
                character.stats.clone(),
                character.inventory.clone(),
                " (2.5D)",
            )
        }
        (Some(character), _) => {
            let camera = played_camera(
                state,
                node,
                character.camera.as_ref(),
                Some(camera::TOP_DOWN),
                "top-down camera",
                asset::top_down_camera,
            )
            .unwrap_or_default();
            *orbit = OrbitCamera {
                focus: world.translation,
                yaw: 0.0,
                // Straight down is a degenerate view for the orbit camera.
                pitch: camera.pitch.to_radians().min(1.55),
                distance: camera.distance,
            };
            (
                PlayedBody::TopDown(Box::new(SpriteBody {
                    body: Body2d::new(world.translation, Direction4::South),
                    start: world.translation,
                    character: character.clone(),
                })),
                character.moves(),
                character.stats.clone(),
                character.inventory.clone(),
                "",
            )
        }
        (None, Some(character)) => {
            let (rotation, mut position) = (world.rotation, world.translation);
            let (mut yaw, _, _) = rotation.to_euler(EulerRot::YXZ);
            // A character that isn't standing on a 3D area starts at the area's own start point.
            if !terrains.is_empty()
                && !terrains.standing_on(position)
                && let Some((spawn, facing)) = terrains.spawn()
            {
                (position, yaw) = (spawn, facing);
                state.log(LogLevel::Info, "Starting at the area's start point");
            }
            let lean_bone = character.lean.as_ref().and_then(|lean| {
                let item = state.workshop.scene.get(node)?.item.as_ref()?;
                let model = &extractions.get(&item.plugin)?.model(&item.output)?.model;
                model.bones.iter().position(|b| b.name == lean.bone)
            });
            // 3D characters without a camera leave the camera where it was put.
            let follow = played_camera(
                state,
                node,
                character.camera.as_ref(),
                None,
                "follow camera",
                asset::follow_camera,
            );
            if let Some(camera) = &follow {
                *orbit = OrbitCamera {
                    focus: position + Vec3::Y * camera.height,
                    yaw: yaw + std::f32::consts::PI,
                    pitch: camera.pitch.to_radians(),
                    distance: camera.distance,
                };
            }
            (
                PlayedBody::Model(Box::new(ModelBody {
                    body: Body::new(position, yaw),
                    character: character.clone(),
                    jump_pending: false,
                    lean_bone,
                    last_action: None,
                    camera: follow,
                })),
                character.moves(),
                character.stats.clone(),
                character.inventory.clone(),
                "",
            )
        }
        (None, None) => return,
    };

    // What every kind has: its stats and the items it carries, each from the strongest layer.
    let over = state.workshop.scene.get(node).cloned();
    let game = crate::play_kit::game(state);
    let stats = crate::play_kit::setting(
        state,
        node,
        "stats",
        over.as_ref().and_then(|n| n.stats.clone()),
        game.stats.clone(),
        stats.as_ref(),
        Some(parts::builtin_stats()),
        "stats",
        asset::stats_of,
    )
    .unwrap_or_default();
    let inventory = over
        .as_ref()
        .and_then(|n| n.inventory.clone())
        .or(game.inventory.clone())
        .or(inventory);
    let rules = crate::play_kit::setting(
        state,
        node,
        "rules",
        None,
        game.rules.clone(),
        None,
        Some(game::builtin_rules()),
        "rules",
        asset::rules_of,
    )
    .unwrap_or_default();
    let flow = crate::play_kit::flow(state);
    crate::content_check::report(state, &session.entities, rules.goal.as_ref(), flow.as_ref());
    let fighter = new_fighter(extractions, state, node, moves, inventory);
    session.played = Some(Played {
        node,
        leftover: 0.0,
        fighter,
        vitals: Vitals::new(stats),
        origin: body.position(),
        lives: rules.lives,
        time_left: rules.time_limit,
        rules,
        fade: None,
        won: false,
        body,
    });
    state.workshop.banner = None;
    state.workshop.kit_sounds = played_sounds(state, extractions, node);
    state.workshop.playing = Some(node);
    state.workshop.adjusting = None;
    state.log(LogLevel::Info, format!("Playing {name}{how}"));
}

/// The sounds of the character a node plays (see [`clockwork_core::ui::SoundSet`]).
fn played_sounds(
    state: &mut EditorState,
    extractions: &Extractions,
    node: NodeId,
) -> crate::state::KitSounds {
    let kit = played_kit(state, extractions, node);
    let set = crate::play_kit::setting(
        state,
        node,
        "sounds",
        None,
        None,
        kit.sounds.as_ref(),
        None,
        "sounds",
        asset::sounds_of,
    )
    .unwrap_or_default();
    let name = |text: &Option<String>| text.as_deref().and_then(asset::OutputRef::parse);
    crate::state::KitSounds {
        hurt: name(&set.hurt),
        pickup: name(&set.pickup),
        door: name(&set.door),
        menu: name(&set.menu),
    }
}

/// The kit (HUD, UI, menus, input map) of the character a node plays.
fn played_kit(
    state: &EditorState,
    extractions: &Extractions,
    node: NodeId,
) -> clockwork_core::ui::Kit {
    let Some(item) = state.workshop.scene.get(node).and_then(|n| n.item.as_ref()) else {
        return Default::default();
    };
    let Some(x) = extractions.get(&item.plugin) else {
        return Default::default();
    };
    x.sprite(&item.output)
        .and_then(|s| s.character.as_ref())
        .map(|c| c.kit.clone())
        .or_else(|| {
            x.model(&item.output)
                .and_then(|m| m.character.as_ref())
                .map(|c| c.kit.clone())
        })
        .unwrap_or_default()
}

/// The camera of a played character: the node's override, the game's, the character's own,
/// else the built-in one named `builtin` (see [`crate::play_kit::setting`]).
fn played_camera<T: Clone>(
    state: &mut EditorState,
    node: NodeId,
    own: Option<&asset::Slot<T>>,
    builtin: Option<&str>,
    expected: &'static str,
    take: fn(&Output) -> Option<asset::Slot<T>>,
) -> Option<T> {
    let over = state
        .workshop
        .scene
        .get(node)
        .and_then(|n| n.camera.clone());
    let game = crate::play_kit::game(state).camera;
    crate::play_kit::setting(
        state,
        node,
        "camera",
        over,
        game,
        own,
        builtin.map(camera::builtin),
        expected,
        take,
    )
}

/// The fighting state of a played character. Its items come from `inventory` (an `Items`
/// asset) or else its plugin's first list of items; when their plugin isn't extracted yet,
/// [`complete_inventory`] fills them in once it is.
fn new_fighter(
    extractions: &mut Extractions,
    state: &mut EditorState,
    node: NodeId,
    moves: Moves,
    inventory: Option<clockwork_core::asset::OutputRef>,
) -> Fighter {
    // Held items and projectiles are drawn from the character's own plugin.
    let plugin = state
        .workshop
        .scene
        .get(node)
        .and_then(|n| n.item.as_ref())
        .map(|item| item.plugin.clone());
    let mut fighter = Fighter::new(None, moves);
    fighter.plugin = plugin.clone();
    // An exported game doesn't draw hit boxes.
    fighter.hitboxes = state.player.is_none();
    fighter.wanted_items = inventory.or_else(|| {
        // Its plugin's first list of items, if it has one.
        let plugin = plugin?;
        let first = state.workshop.plugin_index(&plugin).and_then(|i| {
            state.workshop.plugins[i]
                .recipe
                .outputs
                .iter()
                .find(|o| o.name() == "Items")
                .map(|o| o.id().to_string())
        })?;
        Some(clockwork_core::asset::OutputRef::new(&plugin, &first))
    });
    complete_inventory(&mut fighter, extractions, state);
    fighter
}

/// Gives a fighter the items it wants once their plugin is extracted (asking for the
/// extraction the first time), with their icons.
fn complete_inventory(
    fighter: &mut Fighter,
    extractions: &mut Extractions,
    state: &mut EditorState,
) {
    if fighter.inventory.is_some() {
        return;
    }
    let Some(at) = fighter.wanted_items.clone() else {
        return;
    };
    extractions.ensure(state, &at.plugin);
    let Some(extracted) = extractions.get(&at.plugin) else {
        if !extractions.is_pending(&at.plugin) {
            state.log(LogLevel::Warn, format!("The items {at} can't be extracted"));
            fighter.wanted_items = None;
        }
        return;
    };
    let Some((_, items)) = extracted.items.iter().find(|(id, _)| *id == at.output) else {
        state.log(LogLevel::Warn, format!("{at} isn't a list of items"));
        fighter.wanted_items = None;
        return;
    };
    fighter.inventory = Some(Inventory::new(items.clone()));
    // Icons come from the plugin of the items.
    fighter.icons = std::sync::Arc::new(
        extracted
            .items
            .iter()
            .flat_map(|(_, items)| items)
            .filter_map(|item| Some((item.id.clone(), extracted.icon(item.icon.as_ref()?)?)))
            .collect(),
    );
}

/// What running the behaviors changed for the player.
#[derive(Default)]
struct EntityOutcome {
    teleport: Option<Vec3>,
    checkpoint: Option<Vec3>,
    restart: bool,
    damage: f32,
    heal: f32,
    /// A door entered: where it leads and how long the screen fades.
    door: Option<(Vec3, f32)>,
    /// Something was picked up.
    picked: bool,
    /// A dialog asset to show from its plugin's extraction.
    dialog: Option<clockwork_core::asset::OutputRef>,
    /// The node a door or a teleport took the player to.
    arrived: Option<String>,
}

/// Applies the player's attacks to targets, and its position to pickups, triggers and
/// checkpoints. `fighter` is a 2D character's fighting state, if it has one; `projectiles` are
/// its projectile specs.
fn run_entities(
    entities: &mut Entities,
    mut fighter: Option<&mut Fighter>,
    projectiles: &[clockwork_core::action::Projectile],
    at: Vec3,
    radius: f32,
    dt: f32,
    state: &mut EditorState,
) -> EntityOutcome {
    let mut outcome = EntityOutcome::default();
    let mut notices: Vec<String> = Vec::new();
    // The player's notices are in the game's words; problems are for the author, in the log.
    let mut problems: Vec<String> = Vec::new();
    let words = words(state);
    entities.update(dt);

    if let Some(fighter) = fighter.as_deref_mut() {
        for report in std::mem::take(&mut fighter.hits) {
            let hit = entities.apply_hit(&report.shape, report.damage, report.projectile);
            if hit.landed
                && let Some(uid) = report.projectile
            {
                fighter.projectiles.on_hit(projectiles, uid);
            }
            for (node, drop) in hit.destroyed {
                let name = state
                    .workshop
                    .scene
                    .get(node)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                say(&mut notices, &words.destroyed, &[("name", &name)]);
                if let Some((item, count)) = drop
                    && let Some(inventory) = &mut fighter.inventory
                {
                    let added = inventory.add(&item, count);
                    if added > 0 {
                        let label = inventory
                            .item(&item)
                            .map_or(item.clone(), |i| i.name.clone());
                        say(
                            &mut notices,
                            &words.got,
                            &[("item", &label), ("n", &added.to_string())],
                        );
                    }
                }
            }
        }
    }

    // Conditions first: what appeared, which behaviors work.
    let carried = |item: &str| {
        fighter
            .as_deref()
            .and_then(|f| f.inventory.as_ref())
            .map_or(0, |i| i.count(item))
    };
    entities.refresh(&carried);
    for touch in entities.touch(at, radius) {
        match touch {
            Touch::Pickup { node, item, count } => {
                let Some(inventory) = fighter.as_deref_mut().and_then(|f| f.inventory.as_mut())
                else {
                    continue;
                };
                if inventory.add(&item, count) > 0 {
                    entities.consume(node);
                    outcome.picked = true;
                    let label = inventory
                        .item(&item)
                        .map_or(item.clone(), |i| i.name.clone());
                    say(
                        &mut notices,
                        &words.got,
                        &[("item", &label), ("n", &count.to_string())],
                    );
                }
            }
            Touch::Checkpoint { at } => {
                outcome.checkpoint = Some(at);
                say(&mut notices, &words.checkpoint, &[]);
            }
            Touch::Hazard { damage } => outcome.damage += damage,
            Touch::Door { to, fade } => {
                let scene = &state.workshop.scene;
                match scene.nodes.iter().find(|n| n.name == to) {
                    Some(node) => {
                        outcome.door = Some((scene.world_transform(node.id).translation, fade));
                        outcome.arrived = Some(to);
                    }
                    None => problems.push(format!("No node called \"{to}\" to go to")),
                }
            }
            Touch::Trigger { effect, .. } => apply_effect(
                effect,
                entities,
                fighter.as_deref_mut(),
                state,
                &mut outcome,
                &mut notices,
                &words,
            ),
        }
    }

    for text in problems {
        state.log(LogLevel::Warn, text);
    }
    for text in notices {
        state.log(LogLevel::Info, text.clone());
        state.workshop.toast = Some((text, 3.0));
    }
    state.workshop.hidden_in_play = entities.hidden().collect();
    outcome
}

/// The words Play shows, from the played screen's HUD.
pub(crate) fn words(state: &EditorState) -> Words {
    state
        .workshop
        .screen
        .as_ref()
        .map(|(screen, _)| screen.layout.words.clone())
        .unwrap_or_default()
}

/// Adds a notice in the game's words, unless the game leaves that one out (empty).
fn say(notices: &mut Vec<String>, template: &str, values: &[(&str, &str)]) {
    if !template.is_empty() {
        notices.push(clockwork_core::ui::fill(template, values));
    }
}

fn toast(text: &str) -> Option<(String, f32)> {
    (!text.is_empty()).then(|| (text.to_string(), 3.0))
}

/// Does what a trigger's effect says.
fn apply_effect(
    effect: Effect,
    entities: &mut Entities,
    mut fighter: Option<&mut Fighter>,
    state: &mut EditorState,
    outcome: &mut EntityOutcome,
    notices: &mut Vec<String>,
    words: &Words,
) {
    match effect {
        Effect::Message(text) => notices.push(text),
        Effect::Restart => outcome.restart = true,
        Effect::Hurt(amount) => outcome.damage += amount,
        Effect::Heal(amount) => outcome.heal += amount,
        Effect::Say(dialog) => {
            let found = {
                let index = AssetIndex::build(&state.workshop.plugins);
                asset::resolve(
                    &[(asset::Layer::Asset, Some(&dialog))],
                    &index,
                    "dialog",
                    asset::dialog_of,
                )
            };
            match found {
                Ok(Some(found)) if !found.value.pages.is_empty() => {
                    state.workshop.dialog = Some((found.value, 0));
                }
                // Its pages may come from the ROM: the plugin's extraction has them.
                Ok(Some(_)) => {
                    if let asset::Slot::Ref(at) = dialog {
                        outcome.dialog = Some(at);
                    }
                }
                Ok(_) => {}
                Err(error) => state.log(LogLevel::Warn, format!("A dialog: {error}")),
            }
        }
        Effect::Sound(name) => match asset::OutputRef::parse(&name) {
            Some(at) => state.workshop.sounds.push(at),
            None => state.log(
                LogLevel::Warn,
                format!("\"{name}\" is not \"plugin/output\""),
            ),
        },
        Effect::Teleport(name) => {
            let scene = &state.workshop.scene;
            if let Some(node) = scene.nodes.iter().find(|n| n.name == name) {
                outcome.teleport = Some(scene.world_transform(node.id).translation);
                outcome.arrived = Some(name);
            } else {
                state.log(
                    LogLevel::Warn,
                    format!("No node called \"{name}\" to go to"),
                );
            }
        }
        Effect::Give(item, count) => {
            if let Some(inventory) = fighter.as_deref_mut().and_then(|f| f.inventory.as_mut())
                && inventory.add(&item, count) > 0
            {
                let label = fighter
                    .as_deref()
                    .and_then(|f| f.inventory.as_ref())
                    .and_then(|i| i.item(&item))
                    .map_or(item.clone(), |i| i.name.clone());
                say(
                    notices,
                    &words.got,
                    &[("item", &label), ("n", &count.to_string())],
                );
            }
        }

        Effect::Set(flag) => entities.set_flag(&flag, true),
        Effect::Clear(flag) => entities.set_flag(&flag, false),
        Effect::All(effects) => {
            for effect in effects {
                apply_effect(
                    effect,
                    entities,
                    fighter.as_deref_mut(),
                    state,
                    outcome,
                    notices,
                    words,
                );
            }
        }
    }
}

/// The live panel's text: the played character, the clock and the scene's behaviors.
/// A growing event list as its count and last few entries, so the line stays short.
fn recent(events: &[String]) -> String {
    const SHOWN: usize = 3;
    let last = &events[events.len().saturating_sub(SHOWN)..];
    let more = if events.len() > SHOWN { "…, " } else { "" };
    format!("{} [{more}{}]", events.len(), last.join(", "))
}

fn live_lines(session: &PlaySession, state: &EditorState) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(line) = session.describe() {
        // The character's line is long: one fact per row.
        lines.extend(line.split(" | ").map(str::to_string));
    }
    let clock = state.workshop.play_clock;
    lines.push(format!(
        "clock: {}{} x{}",
        if clock.paused { "paused" } else { "running" },
        if clock.step_once { " (stepping)" } else { "" },
        clock.speed
    ));
    let entities = session.entities();
    lines.push(match entities.act {
        Some(act) => format!("act: {act}"),
        None => "act: none (all acts)".into(),
    });
    if !entities.flags.is_empty() {
        let flags: Vec<&str> = entities.flags.iter().map(String::as_str).collect();
        lines.push(format!("flags: {}", flags.join(", ")));
    }
    for e in &entities.live {
        if e.gone {
            continue;
        }
        let name = state
            .workshop
            .scene
            .get(e.node)
            .map_or("?", |n| n.name.as_str());
        lines.push(match &e.behavior {
            crate::entities::Behavior::Target { .. } => {
                format!("{name}: target {:.1}/{:.1}", e.health, e.max_health)
            }
            b => format!("{name}: {}", b.kind().to_lowercase()),
        });
        // Waiting on its condition, its node appearing, or its act.
        if !e.active
            && let Some(line) = lines.last_mut()
        {
            line.push_str(" (waiting)");
        }
    }
    for node in &entities.nodes {
        if !node.gone && !entities.live.iter().any(|e| e.node == node.id) {
            lines.push(format!(
                "{}: node{}",
                node.name,
                if node.present(entities.act) {
                    ""
                } else {
                    " (waiting)"
                }
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_an_act_refreshes_play_without_rebuilding_entities() {
        let mut scene = crate::scene::Scene::default();
        let id = scene.add(crate::scene::ROOT, "Wall".into(), None, Vec3::ZERO);
        let node = scene.get_mut(id).unwrap();
        node.acts = Some(vec![2]);
        node.behaviors.push(crate::entities::NodeBehavior::new(
            crate::entities::Behavior::Solid { radius: 1.0 },
        ));
        let mut session = PlaySession {
            entities: Entities::build(&scene, |_| Vec3::ZERO),
            ..Default::default()
        };
        session.set_act(Some(1));
        assert_eq!(session.entities.hidden().collect::<Vec<_>>(), vec![id]);
        assert!(!session.entities.blocked(Vec2::ZERO));
        session.set_act(Some(2));
        assert_eq!(session.entities.hidden().count(), 0);
        assert!(session.entities.blocked(Vec2::ZERO));
        session.entities.consume(id);
        session.set_act(None);
        assert_eq!(session.entities.hidden().collect::<Vec<_>>(), vec![id]);
        assert!(!session.entities.blocked(Vec2::ZERO));
    }

    #[test]
    fn a_door_fade_is_darkest_halfway() {
        let fade = |elapsed| Fade {
            elapsed,
            total: 1.0,
            to: Vec3::ZERO,
            moved: false,
        };
        assert_eq!(fade(0.0).darkness(), 0.0);
        assert_eq!(fade(0.5).darkness(), 1.0);
        assert_eq!(fade(1.0).darkness(), 0.0);
        assert!((fade(0.25).darkness() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn vitals_rest_after_a_hit_and_run_out() {
        let mut v = Vitals::new(Stats {
            health: 2.0,
            hurt_time: 1.0,
        });
        assert!(!v.hurt(1.0));
        assert!(!v.hurt(1.0), "just hurt: no second hit");
        v.safe = 0.0;
        assert!(v.hurt(1.0));
        v.heal(5.0);
        assert_eq!(v.health, 2.0);
    }
}

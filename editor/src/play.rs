//! Play mode: drive a character around the scene. The runtime plays it
//! (`clockwork_runtime::play`); this hands it the scene, the extracted content and the input,
//! and shows what it does: the played node's entity, its animation, the camera, the HUD and
//! what happened.
//!
//! The scene itself is never changed while playing: the played node's entity is moved directly,
//! and stopping lets the scene's own transform take over again.

use std::sync::LazyLock;

use bevy::{math::Affine3A, prelude::*};
use clockwork_core::{action::Moves, character2d::Direction4, game, parts, ui::Words};
use clockwork_recipe::assets::AssetIndex;
use clockwork_runtime::{
    billboard::{self, Body25d},
    controller::{self, Body},
    hud::HudRequest,
    input::{GameButton, GameInput},
    inventory::Inventory,
    play::{
        Area, Avatar, BillboardBody, Controls, Frame, Happened, Level, ModelBody, Play, STEP,
        SpriteBody,
        fighter::{Buttons, Fighter, Held},
    },
    topdown::{self, Body2d},
};

use clockwork_core::{asset, camera, recipe::Output};

use crate::{
    entities::Entities,
    models::Rig,
    scene::NodeId,
    sprites::SpriteRig,
    state::{EditorState, LogLevel},
    terrains::SceneTerrains,
    workshop::{Extractions, NodeImage, OrbitCamera, SceneView, surfaces_for},
};

/// The camera looks at this point above the character's feet.
const CAMERA_HEIGHT: f32 = 0.8;

#[derive(Resource, Default)]
pub struct PlaySession {
    /// The played character and the scene's behaviors, while playing.
    play: Option<Play>,
    /// A dialog to show once its plugin is extracted: one whose pages come from the ROM.
    waiting_dialog: Option<clockwork_core::asset::OutputRef>,
}

/// The behaviors while nothing is played.
static NO_ENTITIES: LazyLock<Entities> = LazyLock::new(Entities::default);

impl PlaySession {
    /// The scene's behaviors while playing (empty otherwise).
    pub fn entities(&self) -> &Entities {
        self.play.as_ref().map_or(&NO_ENTITIES, |p| &p.entities)
    }

    /// The scene's behaviors while playing, to put a save file into.
    pub fn entities_mut(&mut self) -> Option<&mut Entities> {
        self.play.as_mut().map(|p| &mut p.entities)
    }

    /// Chooses an act and judges conditions with the current inventory, keeping play progress.
    pub fn set_act(&mut self, act: Option<u8>) {
        if let Some(play) = &mut self.play {
            play.set_act(act);
        }
    }

    /// The played character's fighting state, if one is played.
    pub fn fighter(&self) -> Option<&Fighter> {
        self.play.as_ref().map(|p| &p.fighter)
    }

    /// Whether the played character is in a 3D view: a 3D character, or a 2D one standing
    /// upright in a 3D world (2.5D), rather than a sprite lying on a map seen from above.
    pub fn in_3d(&self) -> bool {
        self.play.as_ref().is_some_and(Play::in_3d)
    }

    /// The played 3D character's node, what it holds and the bone it holds it at.
    pub fn held_at_hand(&self) -> Option<(NodeId, Vec<Held>, Option<&str>)> {
        let play = self.play.as_ref()?;
        let Avatar::Model(m) = &play.avatar else {
            return None;
        };
        Some((play.node, play.fighter.held(), m.character.hand.as_deref()))
    }

    /// Where the played character stands.
    pub fn position(&self) -> Option<Vec3> {
        self.play.as_ref().map(Play::position)
    }

    /// Moves the played character (backing out of a course, a save file's start).
    pub fn teleport(&mut self, at: Vec3) {
        if let Some(play) = &mut self.play {
            play.teleport(at);
        }
    }

    /// Moves the played character to where the game starts, and makes it the place it starts
    /// over from.
    pub fn start_at(&mut self, at: Vec3) {
        if let Some(play) = &mut self.play {
            play.start_at(at);
        }
    }

    /// What the played character carries, once its items are there.
    pub fn inventory(&self) -> Option<&Inventory> {
        self.play.as_ref()?.inventory()
    }

    pub fn inventory_mut(&mut self) -> Option<&mut Inventory> {
        self.play.as_mut()?.inventory_mut()
    }

    /// The played character's state in one line, for the remote control's reports.
    pub fn describe(&self) -> Option<String> {
        self.play.as_ref().map(Play::describe)
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
        Some(true) if session.play.is_none() => {
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
            // Actors go back to their own animations, and their nodes to where they stand.
            for mut rig in &mut rigs {
                if state.workshop.acting.contains(&rig.node) {
                    rig.playback = None;
                    rig.time = 0.0;
                }
            }
            for mut rig in &mut sprite_rigs {
                if state.workshop.acting.contains(&rig.node) {
                    rig.playback = None;
                    rig.time = 0.0;
                }
            }
            if let Some(played) = session.play.take() {
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
            state.workshop.acting.clear();
            state.workshop.toast = None;
            state.workshop.play_clock = Default::default();
        }
        _ => {}
    }
    let Some(played) = session.play.as_ref().map(|p| p.node) else {
        return;
    };
    // The theme's font and frame, and the items, once their plugins are extracted.
    crate::play_kit::complete_screen(&mut state, &mut extractions);
    if let Some(played) = &mut session.play {
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
        session.play = None;
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
            .play
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
        || session.play.as_ref().is_some_and(|p| p.won);
    let sim_dt = if frozen { 0.0 } else { sim_dt };
    if let Some((_, left)) = &mut state.workshop.toast {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            state.workshop.toast = None;
        }
    }
    let requests = std::mem::take(&mut state.workshop.hud_requests);
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
    let level = Level {
        areas,
        terrains: terrains.all(),
    };
    state.workshop.live = live_lines(&session, &state);
    let session = &mut *session;
    let Some(played) = &mut session.play else {
        return;
    };
    let frame = Frame {
        controls: Controls {
            stick,
            direction,
            walk,
            jump,
            held,
            yaw: orbit.yaw,
            buttons,
        },
        requests,
        sim_dt,
        dt: time.delta_secs(),
        screen: flow.frozen(),
        game_over_screen: flow.has_game_over(),
    };
    let words = words(&state);
    let happened = {
        let scene = &state.workshop.scene;
        let places = |name: &str| {
            scene
                .nodes
                .iter()
                .find(|n| n.name == name)
                .map(|n| scene.world_transform(n.id).translation)
        };
        played.advance(&frame, &level, &places, &words)
    };
    for event in happened {
        match event {
            Happened::Notice(text) => {
                state.log(LogLevel::Info, text.clone());
                state.workshop.toast = Some((text, 3.0));
            }
            Happened::Toast(text) => state.workshop.toast = Some((text, 3.0)),
            Happened::Problem(text) => state.log(LogLevel::Warn, text),
            Happened::Say(dialog) => {
                if let Some(at) = show_dialog(dialog, &mut state) {
                    session.waiting_dialog = Some(at);
                }
            }
            Happened::Sound(name) => match asset::OutputRef::parse(&name) {
                Some(at) => state.workshop.sounds.push(at),
                None => state.log(
                    LogLevel::Warn,
                    format!("\"{name}\" is not \"plugin/output\""),
                ),
            },
            // Arriving in a course asks which act to play.
            Happened::Arrived { name, from } => flow.arrived(&name, from),
            Happened::Hurt => {
                let sound = state.workshop.kit_sounds.hurt.clone();
                state.workshop.sounds.extend(sound);
            }
            Happened::Door => {
                let sound = state.workshop.kit_sounds.door.clone();
                state.workshop.sounds.extend(sound);
            }
            Happened::Picked => {
                let sound = state.workshop.kit_sounds.pickup.clone();
                state.workshop.sounds.extend(sound);
            }
            Happened::GameOver => {
                flow.game_over();
            }
            Happened::Stop => state.workshop.play_request = Some(false),
            Happened::Won(win) => state.workshop.banner = (!win.is_empty()).then_some(win),
        }
    }
    state.workshop.hidden_in_play = played.entities.hidden().collect();
    state.workshop.fade = (played.darkness() * 16.0).round() as u8;
    state.workshop.lives = played.lives;
    state.workshop.seconds = played.time_left.map(|t| t.ceil().max(0.0) as u32);
    let fights = !played.fighter.moves.actions.is_empty() || played.fighter.inventory.is_some();
    state.workshop.hud = fights.then(|| played.fighter.hud());
    state.workshop.health = Some((played.vitals.health, played.vitals.max));

    // Actors where they went, showing their state's animation.
    state.workshop.acting.clear();
    for pose in played.entities.poses() {
        if pose.node == played.node {
            continue;
        }
        state.workshop.acting.insert(pose.node);
        if let Some(entity) = view.entity(pose.node)
            && let Ok(mut transform) = transforms.get_mut(entity)
        {
            let parent = state
                .workshop
                .scene
                .get(pose.node)
                .and_then(|n| n.parent)
                .and_then(|p| view.entity(p))
                .and_then(|e| parents.get(e).ok())
                .map(|g| g.affine())
                .unwrap_or_default();
            transform.translation = parent.inverse().transform_point3(pose.at);
        }
        let Some((animation, flip)) = pose.animation else {
            continue;
        };
        for mut rig in &mut sprite_rigs {
            if rig.node == pose.node {
                rig.playback = Some((animation.to_string(), flip, pose.seconds));
            }
        }
        for mut rig in &mut rigs {
            if rig.node == pose.node {
                if rig
                    .playback
                    .as_ref()
                    .is_none_or(|(name, _)| name != animation)
                {
                    rig.time = 0.0;
                    rig.finished = false;
                }
                rig.playback = Some((animation.to_string(), 1.0));
            }
        }
    }

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
    match &mut played.avatar {
        Avatar::Billboard(b) => {
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
        Avatar::TopDown(s) => {
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
        Avatar::Model(m) => {
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
    let entities = crate::entities::build(&state.workshop.scene);
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
                Avatar::Billboard(Box::new(BillboardBody {
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
                Avatar::TopDown(Box::new(SpriteBody {
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
                Avatar::Model(Box::new(ModelBody {
                    body: Body::new(position, yaw),
                    character: character.clone(),
                    jump_buffer: 0.0,
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
    crate::content_check::report(state, &entities, rules.goal.as_ref(), flow.as_ref());
    let fighter = new_fighter(extractions, state, node, moves, inventory);
    session.play = Some(Play::new(node, body, fighter, stats, rules, entities));
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

/// The words Play shows, from the played screen's HUD.
pub(crate) fn words(state: &EditorState) -> Words {
    state
        .workshop
        .screen
        .as_ref()
        .map(|(screen, _)| screen.layout.words.clone())
        .unwrap_or_default()
}

/// Shows a trigger's dialog. One whose pages come from the ROM is returned, to show once its
/// plugin is extracted.
fn show_dialog(
    dialog: asset::Slot<clockwork_core::ui::Dialog>,
    state: &mut EditorState,
) -> Option<clockwork_core::asset::OutputRef> {
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
            None
        }
        // Its pages may come from the ROM: the plugin's extraction has them.
        Ok(Some(_)) => match dialog {
            asset::Slot::Ref(at) => Some(at),
            asset::Slot::Inline(_) => None,
        },
        Ok(_) => None,
        Err(error) => {
            state.log(LogLevel::Warn, format!("A dialog: {error}"));
            None
        }
    }
}

/// The live panel's text: the played character, the clock and the scene's behaviors.
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

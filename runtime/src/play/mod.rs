//! Play's simulation: a played character moving through a scene, its actions and items, the
//! scene's behaviors and the game's rules. It runs without a window: whoever shows it (the
//! editor's Play mode, an exported game, a test) hands it the frame's input and the world, and
//! gets back what happened, to show and to log.
//!
//! [`Play`] steps at a fixed rate ([`STEP`]), so a game plays the same whatever the frame rate.

pub mod entities;
pub mod fighter;
pub mod world;

use bevy::math::{Vec2, Vec3};
use clockwork_core::{
    asset::Slot,
    character::{Character, FollowCamera},
    character2d::{Character2d, Direction4},
    game::{Goal, Rules, WhenDown},
    item,
    parts::Stats,
    ui::{self, Dialog, Words},
};

use crate::{
    billboard::{self, Body25d},
    controller::{self, Body, Input, World},
    hud::HudRequest,
    inventory::Inventory,
    topdown::{self, Body2d},
};

use entities::{Behavior, Effect, Entities, Touch};
use fighter::{Buttons, Fighter};
pub use world::{Area, Level, PlayWorld};

/// A scene node, by its id.
pub type NodeId = u64;

/// The simulation steps at a fixed rate, so movement doesn't depend on the frame rate.
pub const STEP: f32 = 1.0 / 60.0;
/// How long a jump press is remembered while it can't be used yet, in seconds.
const JUMP_BUFFER: f32 = 0.15;

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
    pub fn new(stats: Stats) -> Self {
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

/// The screen fading out and in on the way through a door.
#[derive(Clone, Debug)]
pub struct Fade {
    pub elapsed: f32,
    pub total: f32,
    pub to: Vec3,
    pub moved: bool,
}

impl Fade {
    /// How dark the screen is, 0 to 1: dark halfway.
    pub fn darkness(&self) -> f32 {
        1.0 - (self.elapsed / (self.total / 2.0) - 1.0).abs().min(1.0)
    }
}

/// How the played character moves and is shown: its movement asset's kind decides.
pub enum Avatar {
    /// A 3D character running and jumping.
    Model(Box<ModelBody>),
    /// A sprite walking on a map seen from above.
    TopDown(Box<SpriteBody>),
    /// A sprite standing upright in a 3D world (2.5D), walking on its floors.
    Billboard(Box<BillboardBody>),
}

pub struct ModelBody {
    pub body: Body,
    pub character: Character,
    /// Seconds a jump press stays waiting to be used. Frames often render without a step (the
    /// display is faster than the simulation), and a press made a moment too early (in the
    /// air just before landing, or as an attack ends) should still jump.
    pub jump_buffer: f32,
    /// The bone the character leans with.
    pub lean_bone: Option<usize>,
    /// The action shown last, so a new one starts its animation from the beginning.
    pub last_action: Option<String>,
    /// The camera following it, if any.
    pub camera: Option<FollowCamera>,
}

pub struct SpriteBody {
    pub body: Body2d,
    pub character: Character2d,
    /// Where it starts again.
    pub start: Vec3,
}

pub struct BillboardBody {
    pub body: Body25d,
    pub character: Character2d,
}

/// The player's input in one frame.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    /// The stick as seen on screen, and the ground direction it points.
    pub stick: Vec2,
    pub direction: Vec2,
    pub walk: bool,
    /// The jump button pressed this frame, and held.
    pub jump: bool,
    pub held: bool,
    /// Where the camera looks.
    pub yaw: f32,
    pub buttons: Buttons,
}

impl Avatar {
    pub fn position(&self) -> Vec3 {
        match self {
            Avatar::Model(m) => m.body.position,
            Avatar::TopDown(s) => s.body.position,
            Avatar::Billboard(b) => b.body.position,
        }
    }

    pub fn radius(&self) -> f32 {
        match self {
            Avatar::Model(m) => m.character.movement.radius,
            Avatar::TopDown(s) => s.character.movement.radius,
            Avatar::Billboard(b) => b.character.movement.radius,
        }
    }

    pub fn teleport(&mut self, at: Vec3) {
        match self {
            Avatar::Model(m) => m.body.position = at,
            Avatar::TopDown(s) => s.body.position = at,
            Avatar::Billboard(b) => b.body.position = at,
        }
    }

    /// Where it starts again from now on.
    pub fn checkpoint(&mut self, at: Vec3) {
        match self {
            Avatar::Model(m) => m.body.start = at,
            Avatar::TopDown(s) => s.start = at,
            Avatar::Billboard(b) => b.body.start = at,
        }
    }

    pub fn start_over(&mut self) {
        match self {
            Avatar::Model(m) => m.body.position = m.body.start,
            Avatar::TopDown(s) => s.body.position = s.start,
            Avatar::Billboard(b) => b.body.position = b.body.start,
        }
    }

    /// Advances one simulation step: the action first (it says how movement is constrained),
    /// then the controller of its kind.
    fn step(&mut self, fighter: &mut Fighter, world: &impl World, input: &Controls) {
        match self {
            Avatar::Billboard(b) => {
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
            Avatar::TopDown(s) => {
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
            Avatar::Model(m) => {
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
                    jump_pressed: m.jump_buffer > 0.0 && !busy,
                    jump_held: (input.held || input.jump) && !busy,
                };
                let jumped =
                    controller::step(&mut m.body, &step_input, &m.character.movement, world, STEP);
                // A press waits out a busy action or the end of a fall, but not forever.
                m.jump_buffer = if jumped {
                    0.0
                } else {
                    (m.jump_buffer - STEP).max(0.0)
                };
            }
        }
    }
}

/// One frame of play: the player's input and how much time passes.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub controls: Controls,
    /// What the HUD and menus asked for.
    pub requests: Vec<HudRequest>,
    /// Seconds to simulate: none while paused, in a menu or behind a screen.
    pub sim_dt: f32,
    /// Seconds that passed on the clock: fades go on while the game stands still.
    pub dt: f32,
    /// A screen (the title, choosing a file or an act) is in front: nothing is touched.
    pub screen: bool,
    /// The game has its own game over screen, which shows when the last life is lost.
    pub game_over_screen: bool,
}

/// Where scene nodes are, by name: where doors and teleports lead, and the goal to reach.
pub trait Places {
    fn find(&self, name: &str) -> Option<Vec3>;
}

impl<F: Fn(&str) -> Option<Vec3>> Places for F {
    fn find(&self, name: &str) -> Option<Vec3> {
        self(name)
    }
}

/// What happened in a frame, for whoever shows the game: in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Happened {
    /// Words for the player, in the game's words (picked up, destroyed, a trigger's message).
    Notice(String),
    /// Words for the player that aren't worth a log line (going down, game over).
    Toast(String),
    /// Something wrong with the game's content, for its author.
    Problem(String),
    /// A dialog to show: written in place or a dialog asset.
    Say(Slot<Dialog>),
    /// A sound asset to play, `"plugin/output"` as written.
    Sound(String),
    /// The player arrived at a node through a door or a teleport, from where it stood.
    Arrived { name: String, from: Vec3 },
    /// The played character lost health.
    Hurt,
    /// The played character went through a door.
    Door,
    /// The played character picked something up.
    Picked,
    /// The last life is lost and the game's game over screen should show.
    GameOver,
    /// The rules say play stops.
    Stop,
    /// The goal is reached, with the words that say so.
    Won(String),
}

/// A played character in a scene, and everything that goes on while it is played.
pub struct Play {
    /// The node it plays.
    pub node: NodeId,
    pub avatar: Avatar,
    pub fighter: Fighter,
    pub vitals: Vitals,
    /// What the scene's behaviors are doing.
    pub entities: Entities,
    /// Where it started.
    pub origin: Vec3,
    pub rules: Rules,
    /// Lives and seconds left, when the rules count them.
    pub lives: Option<u32>,
    pub time_left: Option<f32>,
    /// A door's fade going on.
    pub fade: Option<Fade>,
    /// The goal is reached: the game stands still.
    pub won: bool,
    /// Simulation time not stepped yet.
    leftover: f32,
}

/// What running the behaviors changed for the player.
#[derive(Default)]
struct Outcome {
    teleport: Option<Vec3>,
    checkpoint: Option<Vec3>,
    restart: bool,
    damage: f32,
    heal: f32,
    /// A door entered: where it leads and how long the screen fades.
    door: Option<(Vec3, f32)>,
    /// Something was picked up.
    picked: bool,
    /// The node a door or a teleport took the player to.
    arrived: Option<String>,
}

impl Play {
    pub fn new(
        node: NodeId,
        avatar: Avatar,
        fighter: Fighter,
        stats: Stats,
        rules: Rules,
        entities: Entities,
    ) -> Self {
        Self {
            node,
            origin: avatar.position(),
            avatar,
            fighter,
            vitals: Vitals::new(stats),
            entities,
            lives: rules.lives,
            time_left: rules.time_limit,
            rules,
            fade: None,
            won: false,
            leftover: 0.0,
        }
    }

    /// Where the played character stands.
    pub fn position(&self) -> Vec3 {
        self.avatar.position()
    }

    /// Moves the played character (backing out of a course, a save file's start).
    pub fn teleport(&mut self, at: Vec3) {
        self.avatar.teleport(at);
    }

    /// Moves the played character to where the game starts, and makes it the place it starts
    /// over from.
    pub fn start_at(&mut self, at: Vec3) {
        self.avatar.teleport(at);
        self.origin = at;
    }

    /// Whether the played character is in a 3D view: a 3D character, or a 2D one standing
    /// upright in a 3D world (2.5D), rather than a sprite lying on a map seen from above.
    pub fn in_3d(&self) -> bool {
        !matches!(self.avatar, Avatar::TopDown(_))
    }

    /// How many of an item the played character carries.
    pub fn carried(&self, item: &str) -> u32 {
        self.fighter.inventory.as_ref().map_or(0, |i| i.count(item))
    }

    /// What the played character carries, once its items are there.
    pub fn inventory(&self) -> Option<&Inventory> {
        self.fighter.inventory.as_ref()
    }

    pub fn inventory_mut(&mut self) -> Option<&mut Inventory> {
        self.fighter.inventory.as_mut()
    }

    /// Chooses an act and judges conditions with the current inventory, keeping play progress.
    pub fn set_act(&mut self, act: Option<u8>) {
        self.entities.set_act(act);
        let inventory = self.fighter.inventory.as_ref();
        self.entities
            .refresh(&|item| inventory.map_or(0, |i| i.count(item)));
    }

    /// How dark the screen is for a door's fade, 0 to 1.
    pub fn darkness(&self) -> f32 {
        self.fade.as_ref().map_or(0.0, Fade::darkness)
    }

    /// Plays a frame in `level`: the buttons, the steps, the scene's behaviors and what they
    /// did, the rules and the goal. `places` finds the nodes doors lead to; `words` are the
    /// game's own for what it tells the player.
    pub fn advance(
        &mut self,
        frame: &Frame,
        level: &Level,
        places: &impl Places,
        words: &Words,
    ) -> Vec<Happened> {
        let mut happened = Vec::new();
        let controls = &frame.controls;
        if let Avatar::Model(m) = &mut self.avatar
            && controls.jump
        {
            m.jump_buffer = JUMP_BUFFER;
        }
        self.fighter.feed(&controls.buttons, &frame.requests);
        self.leftover += frame.sim_dt;
        {
            let world = PlayWorld {
                level,
                entities: &self.entities,
            };
            while self.leftover >= STEP {
                self.leftover -= STEP;
                self.avatar.step(&mut self.fighter, &world, controls);
            }
        }
        self.vitals.safe = (self.vitals.safe - frame.sim_dt).max(0.0);
        // Behind a screen (the title, choosing a file or an act) nothing is touched.
        let outcome = if frame.screen {
            Outcome::default()
        } else {
            self.run_entities(frame.sim_dt, places, words, &mut happened)
        };
        if let Some(at) = outcome.teleport {
            self.avatar.teleport(at);
        }
        if let Some(name) = outcome.arrived {
            happened.push(Happened::Arrived {
                name,
                from: self.avatar.position(),
            });
        }
        if let Some(at) = outcome.checkpoint {
            self.avatar.checkpoint(at);
        }
        if outcome.restart || frame.requests.contains(&HudRequest::Restart) {
            self.avatar.start_over();
        }
        self.vitals.heal(outcome.heal);
        // A door: the screen fades out, the player moves, it fades back in.
        if let Some((to, fade)) = outcome.door
            && self.fade.is_none()
        {
            self.fade = Some(Fade {
                elapsed: 0.0,
                total: fade.max(0.01),
                to,
                moved: false,
            });
        }
        if let Some(fade) = &mut self.fade {
            fade.elapsed += frame.dt;
            if !fade.moved && fade.elapsed >= fade.total / 2.0 {
                self.avatar.teleport(fade.to);
                fade.moved = true;
            }
        }
        if self.fade.as_ref().is_some_and(|f| f.elapsed >= f.total) {
            self.fade = None;
        }
        // The rules: out of health or out of time is going down.
        let health_before = self.vitals.health;
        let mut down = self.vitals.hurt(outcome.damage);
        if self.vitals.health < health_before {
            happened.push(Happened::Hurt);
        }
        if outcome.door.is_some() {
            happened.push(Happened::Door);
        }
        if outcome.picked {
            happened.push(Happened::Picked);
        }
        if let Some(left) = &mut self.time_left {
            *left -= frame.sim_dt;
            if *left <= 0.0 {
                down = true;
                *left = self.rules.time_limit.unwrap_or_default();
            }
        }
        if down {
            self.go_down(frame.game_over_screen, words, &mut happened);
        }
        self.judge_goal(places, words, &mut happened);
        happened
    }

    /// Out of health or time: a life lost, and back to where the rules say.
    fn go_down(&mut self, game_over_screen: bool, words: &Words, happened: &mut Vec<Happened>) {
        let game_over = self.lives.is_some_and(|l| l <= 1);
        if let Some(lives) = &mut self.lives {
            *lives = lives.saturating_sub(1);
        }
        self.vitals.health = self.vitals.max;
        if game_over && game_over_screen {
            // The game's own game over screen; continuing starts Play again.
            happened.push(Happened::GameOver);
        } else if game_over {
            self.lives = self.rules.lives;
            self.time_left = self.rules.time_limit;
            self.avatar.teleport(self.origin);
            toast(happened, &words.game_over);
        } else {
            match self.rules.when_down {
                WhenDown::StartOver => self.avatar.teleport(self.origin),
                WhenDown::LastCheckpoint => self.avatar.start_over(),
                WhenDown::StopPlaying => happened.push(Happened::Stop),
            }
            toast(happened, &words.down);
        }
    }

    fn judge_goal(&mut self, places: &impl Places, words: &Words, happened: &mut Vec<Happened>) {
        if self.won {
            return;
        }
        let Some(goal) = &self.rules.goal else {
            return;
        };
        let reached = match goal {
            Goal::Collect { item, count } => self.carried(item) >= *count,
            Goal::Reach(name) => places.find(name).is_some_and(|at| {
                let here = self.avatar.position();
                Vec2::new(at.x - here.x, at.z - here.z).length() <= self.avatar.radius() + 0.5
            }),
            Goal::DestroyAll => {
                let targets = self
                    .entities
                    .live
                    .iter()
                    .filter(|e| matches!(e.behavior, Behavior::Target { .. }));
                targets.clone().count() > 0 && targets.into_iter().all(|e| e.gone)
            }
            Goal::When(condition) => {
                let inventory = self.fighter.inventory.as_ref();
                self.entities
                    .holds(condition, &|item| inventory.map_or(0, |i| i.count(item)))
            }
        };
        if reached {
            self.won = true;
            happened.push(Happened::Won(words.win.clone()));
        }
    }

    /// Applies the player's attacks to targets, and its position to pickups, triggers and
    /// checkpoints.
    fn run_entities(
        &mut self,
        dt: f32,
        places: &impl Places,
        words: &Words,
        happened: &mut Vec<Happened>,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        let entities = &mut self.entities;
        let fighter = &mut self.fighter;
        entities.update(dt);

        for report in std::mem::take(&mut fighter.hits) {
            let hit = entities.apply_hit(&report.shape, report.damage, report.projectile);
            if hit.landed
                && let Some(uid) = report.projectile
            {
                let projectiles = fighter.moves.projectiles.clone();
                fighter.projectiles.on_hit(&projectiles, uid);
            }
            for (node, drop) in hit.destroyed {
                let name = entities
                    .nodes
                    .iter()
                    .find(|n| n.id == node)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                say(happened, &words.destroyed, &[("name", &name)]);
                if let Some((item, count)) = drop
                    && let Some(inventory) = &mut fighter.inventory
                {
                    let added = inventory.add(&item, count);
                    if added > 0 {
                        let label = inventory
                            .item(&item)
                            .map_or(item.clone(), |i| i.name.clone());
                        say(
                            happened,
                            &words.got,
                            &[("item", &label), ("n", &added.to_string())],
                        );
                    }
                }
            }
        }

        // Conditions first: what appeared, which behaviors work.
        let inventory = fighter.inventory.as_ref();
        entities.refresh(&|item| inventory.map_or(0, |i| i.count(item)));
        let position = self.avatar.position();
        for touch in entities.touch(position, self.avatar.radius()) {
            match touch {
                Touch::Pickup { node, item, count } => {
                    let Some(inventory) = fighter.inventory.as_mut() else {
                        continue;
                    };
                    if inventory.add(&item, count) > 0 {
                        entities.consume(node);
                        outcome.picked = true;
                        let label = inventory
                            .item(&item)
                            .map_or(item.clone(), |i| i.name.clone());
                        say(
                            happened,
                            &words.got,
                            &[("item", &label), ("n", &count.to_string())],
                        );
                    }
                }
                Touch::Checkpoint { at } => {
                    outcome.checkpoint = Some(at);
                    say(happened, &words.checkpoint, &[]);
                }
                Touch::Hazard { damage } => outcome.damage += damage,
                Touch::Door { to, fade } => match places.find(&to) {
                    Some(at) => {
                        outcome.door = Some((at, fade));
                        outcome.arrived = Some(to);
                    }
                    None => happened.push(Happened::Problem(format!(
                        "No node called \"{to}\" to go to"
                    ))),
                },
                Touch::Trigger { effect, .. } => apply_effect(
                    effect,
                    entities,
                    fighter,
                    places,
                    &mut outcome,
                    happened,
                    words,
                ),
            }
        }
        outcome
    }

    /// The played character's state in one line, for reports.
    pub fn describe(&self) -> String {
        let f = &self.fighter;
        let slot = |s: item::Slot| {
            f.inventory
                .as_ref()
                .and_then(|i| i.equipped(s))
                .map_or("-".to_string(), |item| item.id.clone())
        };
        let tail = format!(
            " | health {}/{} | B {} A {} | action {} | started {} hits {} spawned {} bursts {} projectiles {}{}",
            self.vitals.health,
            self.vitals.max,
            slot(item::Slot::B),
            slot(item::Slot::A),
            f.pose.as_ref().map_or("-", |p| p.0.as_str()),
            recent(&f.stats.started),
            f.stats.hits,
            recent(&f.stats.spawned),
            f.stats.bursts,
            f.projectiles.iter().count(),
            if f.invulnerable { " invulnerable" } else { "" }
        );
        match &self.avatar {
            Avatar::Billboard(s) => {
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
            Avatar::TopDown(s) => {
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
            Avatar::Model(m) => {
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
        }
    }
}

/// A growing event list as its count and last few entries, so the line stays short.
fn recent(events: &[String]) -> String {
    const SHOWN: usize = 3;
    let last = &events[events.len().saturating_sub(SHOWN)..];
    let more = if events.len() > SHOWN { "…, " } else { "" };
    format!("{} [{more}{}]", events.len(), last.join(", "))
}

/// Adds a notice in the game's words, unless the game leaves that one out (empty).
fn say(happened: &mut Vec<Happened>, template: &str, values: &[(&str, &str)]) {
    if !template.is_empty() {
        happened.push(Happened::Notice(ui::fill(template, values)));
    }
}

fn toast(happened: &mut Vec<Happened>, text: &str) {
    if !text.is_empty() {
        happened.push(Happened::Toast(text.to_string()));
    }
}

/// Does what a trigger's effect says.
fn apply_effect(
    effect: Effect,
    entities: &mut Entities,
    fighter: &mut Fighter,
    places: &impl Places,
    outcome: &mut Outcome,
    happened: &mut Vec<Happened>,
    words: &Words,
) {
    match effect {
        Effect::Message(text) => happened.push(Happened::Notice(text)),
        Effect::Restart => outcome.restart = true,
        Effect::Hurt(amount) => outcome.damage += amount,
        Effect::Heal(amount) => outcome.heal += amount,
        Effect::Say(dialog) => happened.push(Happened::Say(dialog)),
        Effect::Sound(name) => happened.push(Happened::Sound(name)),
        Effect::Teleport(name) => match places.find(&name) {
            Some(at) => {
                outcome.teleport = Some(at);
                outcome.arrived = Some(name);
            }
            None => happened.push(Happened::Problem(format!(
                "No node called \"{name}\" to go to"
            ))),
        },
        Effect::Give(item, count) => {
            if let Some(inventory) = fighter.inventory.as_mut()
                && inventory.add(&item, count) > 0
            {
                let label = inventory
                    .item(&item)
                    .map_or(item.clone(), |i| i.name.clone());
                say(
                    happened,
                    &words.got,
                    &[("item", &label), ("n", &count.to_string())],
                );
            }
        }
        Effect::Set(flag) => entities.set_flag(&flag, true),
        Effect::Clear(flag) => entities.set_flag(&flag, false),
        Effect::All(effects) => {
            for effect in effects {
                apply_effect(effect, entities, fighter, places, outcome, happened, words);
            }
        }
    }
}

#[cfg(test)]
mod tests;

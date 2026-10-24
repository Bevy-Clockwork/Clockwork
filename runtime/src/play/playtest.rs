//! Playing a scene from a script instead of a player: hold the stick one way (or towards a node)
//! and press buttons for so many seconds, step after step, and get back what happened and when,
//! where the played character ended up, and what it never reached. It runs at the fixed
//! [`STEP`], so the same script plays the same way every time: an agent or a test can check a
//! game without a window.

use std::collections::HashMap;
use std::fmt;

use bevy::math::{Vec2, Vec3};
use clockwork_core::ui::Words;

use super::{
    Controls, Frame, Happened, Level, NodeId, Places, Play, STEP,
    entities::{Behavior, Effect},
    fighter::Buttons,
};

/// How far the played character must move in [`STUCK_TIME`] while the stick is held, or it is
/// stuck (against a wall, in a corner).
const STUCK_DISTANCE: f32 = 0.1;
const STUCK_TIME: f32 = 0.5;
/// Events kept in a report; a longer play keeps the first ones and counts the rest.
const MOST_EVENTS: usize = 300;

/// Where the stick points during a step.
#[derive(Clone, Debug, PartialEq)]
pub enum Hold {
    Still,
    /// A direction on the ground: x east, y south (down the map). Its length is how far the
    /// stick is pushed, at most 1.
    Direction(Vec2),
    /// Towards the node of this name (where it is now, for an actor), until the played
    /// character reaches it.
    Toward(String),
}

/// A game button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Jump,
    A,
    B,
    Dodge,
}

/// One step of a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub hold: Hold,
    /// How long it lasts; a step towards a node ends early once it is reached.
    pub seconds: f32,
    pub walk: bool,
    /// Pressed as the step starts and held until it ends.
    pub buttons: Vec<Button>,
}

impl Step {
    /// The step in a few words, for reports.
    pub fn label(&self) -> String {
        let mut text = match &self.hold {
            Hold::Still => format!("wait {:.2} s", self.seconds),
            Hold::Direction(d) => format!("hold ({:.2}, {:.2}) {:.2} s", d.x, d.y, self.seconds),
            Hold::Toward(name) => format!("toward \"{name}\" (at most {:.2} s)", self.seconds),
        };
        if self.walk {
            text.push_str(", walking");
        }
        if !self.buttons.is_empty() {
            let buttons: Vec<String> = self
                .buttons
                .iter()
                .map(|b| format!("{b:?}").to_lowercase())
                .collect();
            text.push_str(&format!(", pressing {}", buttons.join(" + ")));
        }
        text
    }
}

/// Something that happened, at `time` seconds into play, during step `step` (from 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub time: f32,
    pub step: usize,
    pub what: String,
}

/// How a step went.
#[derive(Clone, Debug, PartialEq)]
pub struct StepEnd {
    pub label: String,
    /// When a step towards a node reached it.
    pub reached: Option<f32>,
    /// How close a step towards a node came to it, on the ground.
    pub closest: Option<f32>,
    /// When and where the stick was first held without the character getting anywhere.
    pub stuck: Option<(f32, Vec3)>,
    /// Where the character stood at its end.
    pub at: Vec3,
    /// Not played: the game ended before it.
    pub skipped: bool,
}

/// A node the played character never got to, or never beat.
#[derive(Clone, Debug, PartialEq)]
pub struct Untouched {
    pub name: String,
    /// Its behavior's kind, lowercase.
    pub kind: String,
    /// How close the character came to its edge, in meters.
    pub closest: f32,
}

/// What a scripted play did.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Report {
    pub events: Vec<Event>,
    /// Events left out of a long play.
    pub more_events: usize,
    pub steps: Vec<StepEnd>,
    /// Seconds played.
    pub time: f32,
    pub position: Vec3,
    /// Health and its most.
    pub health: (f32, f32),
    pub lives: Option<u32>,
    /// Items carried at the end, with how many.
    pub carried: Vec<(String, u32)>,
    /// When the goal was reached.
    pub won: Option<f32>,
    /// When the last life was lost, or the rules stopped play.
    pub over: Option<f32>,
    /// Nodes to walk into (pickups, doors, triggers, checkpoints) never reached.
    pub never_reached: Vec<Untouched>,
    /// Targets and actors that can be beaten, still standing.
    pub standing: Vec<Untouched>,
    /// Content problems Play found, once each.
    pub problems: Vec<String>,
}

/// Plays `steps` from where `play` stands, in `level`, frame after frame at [`STEP`]. It stops
/// early once the game is won or over. `places` finds nodes by name, `words` are the game's.
pub fn run(
    play: &mut Play,
    level: &Level,
    places: &impl Places,
    words: &Words,
    steps: &[Step],
) -> Report {
    let mut report = Report::default();
    let mut frames = 0u64;
    let mut states: HashMap<NodeId, String> = initial_states(play);
    let mut closest: HashMap<NodeId, f32> = HashMap::new();
    watch(play, &mut closest);
    let mut ended = false;
    for (index, step) in steps.iter().enumerate() {
        let mut end = StepEnd {
            label: step.label(),
            reached: None,
            closest: None,
            stuck: None,
            at: play.position(),
            skipped: ended,
        };
        if ended {
            report.steps.push(end);
            continue;
        }
        let count = (step.seconds.max(0.0) / STEP).round() as u64;
        // Where the character stood when the stick last got it anywhere.
        let mut moved_from = (play.position(), 0u64);
        if let Hold::Toward(name) = &step.hold
            && target(play, places, name).is_none()
        {
            note(
                &mut report,
                frames,
                index,
                format!("no node called \"{name}\" to walk to"),
            );
        }
        for n in 0..count {
            let time = frames as f32 * STEP;
            let here = play.position();
            let reach = play.avatar.radius() + 0.5;
            let direction = match &step.hold {
                Hold::Still => Vec2::ZERO,
                Hold::Direction(d) => d.clamp_length_max(1.0),
                Hold::Toward(name) => match target(play, places, name) {
                    Some(at) => {
                        let to = Vec2::new(at.x - here.x, at.z - here.z);
                        let distance = to.length();
                        end.closest = Some(end.closest.map_or(distance, |c| c.min(distance)));
                        if distance <= reach {
                            end.reached = Some(time);
                            break;
                        }
                        to / distance
                    }
                    None => Vec2::ZERO,
                },
            };
            // A door's fade leaves the player no say, as in Play.
            let free = play.fade.is_none();
            let direction = if free { direction } else { Vec2::ZERO };
            let pressed = |button| n == 0 && step.buttons.contains(&button);
            let held = |button| step.buttons.contains(&button);
            let frame = Frame {
                controls: Controls {
                    // The stick as seen by a camera looking north: up the screen is north.
                    stick: Vec2::new(direction.x, -direction.y),
                    direction,
                    walk: step.walk,
                    jump: pressed(Button::Jump),
                    held: held(Button::Jump),
                    yaw: 0.0,
                    buttons: Buttons {
                        dodge: pressed(Button::Dodge),
                        b: (pressed(Button::B), held(Button::B)),
                        a: (pressed(Button::A), held(Button::A)),
                    },
                },
                sim_dt: STEP,
                dt: STEP,
                game_over_screen: true,
                ..Default::default()
            };
            let gone_before: Vec<NodeId> = play.entities.gone().collect();
            let happened = play.advance(&frame, level, places, words);
            frames += 1;
            let time = frames as f32 * STEP;
            for h in happened {
                let what = match h {
                    Happened::Notice(text) | Happened::Toast(text) => text,
                    Happened::Problem(text) => {
                        if !report.problems.contains(&text) {
                            report.problems.push(text.clone());
                        }
                        format!("problem: {text}")
                    }
                    Happened::Sound(name) => format!("sound {name}"),
                    Happened::Arrived { name, .. } => format!("arrived at \"{name}\""),
                    Happened::Hurt => format!(
                        "hurt{}, health {}/{}",
                        hurt_by(play),
                        play.vitals.health,
                        play.vitals.max
                    ),
                    Happened::Door => "through a door".to_string(),
                    Happened::Picked => continue,
                    Happened::GameOver => {
                        report.over = Some(time);
                        "game over: the last life is lost".to_string()
                    }
                    Happened::Stop => {
                        report.over = Some(time);
                        "the rules stop play".to_string()
                    }
                    Happened::Won(text) => {
                        report.won = Some(time);
                        if text.is_empty() {
                            "won".to_string()
                        } else {
                            format!("won: {text}")
                        }
                    }
                    other => format!("{other:?}"),
                };
                note(&mut report, frames, index, what);
            }
            for node in play.entities.gone().filter(|g| !gone_before.contains(g)) {
                let what = format!("\"{}\" is gone", node_name(play, node));
                note(&mut report, frames, index, what);
            }
            for (node, state) in changed_states(play, &mut states) {
                let what = format!("\"{}\" is now {state}", node_name(play, node));
                note(&mut report, frames, index, what);
            }
            watch(play, &mut closest);
            // Held one way without getting anywhere: stuck.
            let now = play.position();
            let ground = |v: Vec3| Vec2::new(v.x, v.z);
            if direction == Vec2::ZERO
                || ground(now).distance(ground(moved_from.0)) > STUCK_DISTANCE
            {
                moved_from = (now, frames);
            } else if end.stuck.is_none() && (frames - moved_from.1) as f32 * STEP >= STUCK_TIME {
                end.stuck = Some((time, now));
                let what = format!("stuck at {}", position(now));
                note(&mut report, frames, index, what);
            }
            if report.won.is_some() || report.over.is_some() {
                ended = true;
                break;
            }
        }
        // Reached on the last frame of the step.
        if let Hold::Toward(name) = &step.hold
            && end.reached.is_none()
            && let Some(at) = target(play, places, name)
        {
            let here = play.position();
            let distance = Vec2::new(at.x - here.x, at.z - here.z).length();
            end.closest = Some(end.closest.map_or(distance, |c| c.min(distance)));
            if distance <= play.avatar.radius() + 0.5 {
                end.reached = Some(frames as f32 * STEP);
            }
        }
        end.at = play.position();
        report.steps.push(end);
    }
    report.time = frames as f32 * STEP;
    report.position = play.position();
    report.health = (play.vitals.health, play.vitals.max);
    report.lives = play.lives;
    report.carried = play.inventory().map_or(Vec::new(), |inventory| {
        inventory
            .items()
            .iter()
            .map(|item| (item.id.clone(), inventory.count(&item.id)))
            .filter(|(_, n)| *n > 0)
            .collect()
    });
    untouched(play, &closest, &mut report);
    report
}

/// Adds an event, unless the report is full.
fn note(report: &mut Report, frames: u64, step: usize, what: String) {
    if report.events.len() >= MOST_EVENTS {
        report.more_events += 1;
        return;
    }
    report.events.push(Event {
        time: frames as f32 * STEP,
        step,
        what,
    });
}

/// Where the node of this name is now: an actor where it went, any other node where it was put.
fn target(play: &Play, places: &impl Places, name: &str) -> Option<Vec3> {
    let entities = &play.entities;
    entities
        .nodes
        .iter()
        .filter(|n| n.name == name)
        .find_map(|n| entities.live.iter().find(|e| e.node == n.id).map(|e| e.at))
        .or_else(|| places.find(name))
}

fn node_name(play: &Play, node: NodeId) -> String {
    play.entities
        .nodes
        .iter()
        .find(|n| n.id == node)
        .map_or(format!("node {node}"), |n| n.name.clone())
}

/// What hurts the played character where it stands: hazards, actors and hurting triggers it
/// touches.
fn hurt_by(play: &Play) -> String {
    let here = play.position();
    let radius = play.avatar.radius();
    let mut names: Vec<String> = Vec::new();
    for e in play.entities.live.iter().filter(|e| e.working()) {
        let hurts = match &e.behavior {
            Behavior::Hazard { .. } | Behavior::Actor(_) => true,
            Behavior::Trigger { effect, .. } => hurts(effect),
            _ => false,
        };
        let near = Vec2::new(e.at.x - here.x, e.at.z - here.z).length()
            <= e.behavior.radius() + radius + 0.05;
        let name = node_name(play, e.node);
        if hurts && near && !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        String::new()
    } else {
        format!(" by \"{}\"", names.join("\", \""))
    }
}

fn hurts(effect: &Effect) -> bool {
    match effect {
        Effect::Hurt(_) => true,
        Effect::All(effects) => effects.iter().any(hurts),
        _ => false,
    }
}

fn initial_states(play: &Play) -> HashMap<NodeId, String> {
    play.entities
        .poses()
        .map(|p| (p.node, p.state.to_string()))
        .collect()
}

/// Actors whose state changed since last time.
fn changed_states(play: &Play, states: &mut HashMap<NodeId, String>) -> Vec<(NodeId, String)> {
    let mut changed = Vec::new();
    for pose in play.entities.poses() {
        let before = states.insert(pose.node, pose.state.to_string());
        if before.is_some_and(|b| b != pose.state) {
            changed.push((pose.node, pose.state.to_string()));
        }
    }
    changed
}

/// How close the played character has come to the edge of each node with a behavior.
fn watch(play: &Play, closest: &mut HashMap<NodeId, f32>) {
    let here = play.position();
    let radius = play.avatar.radius();
    for e in play.entities.live.iter().filter(|e| e.working()) {
        let gap =
            Vec2::new(e.at.x - here.x, e.at.z - here.z).length() - e.behavior.radius() - radius;
        let entry = closest.entry(e.node).or_insert(f32::INFINITY);
        *entry = entry.min(gap.max(0.0));
    }
}

/// Nodes to walk into that were never reached, and those to beat still standing.
fn untouched(play: &Play, closest: &HashMap<NodeId, f32>, report: &mut Report) {
    let mut seen: Vec<NodeId> = Vec::new();
    for e in &play.entities.live {
        if e.gone || seen.contains(&e.node) {
            continue;
        }
        let gap = closest.get(&e.node).copied().unwrap_or(f32::INFINITY);
        let found = Untouched {
            name: node_name(play, e.node),
            kind: e.behavior.kind().to_lowercase(),
            closest: gap,
        };
        match &e.behavior {
            Behavior::Pickup { .. }
            | Behavior::Door { .. }
            | Behavior::Trigger { .. }
            | Behavior::Checkpoint { .. }
                if gap > 0.0 =>
            {
                seen.push(e.node);
                report.never_reached.push(found);
            }
            Behavior::Target { .. } => {
                seen.push(e.node);
                report.standing.push(found);
            }
            Behavior::Actor(actor) if actor.health.is_some() => {
                seen.push(e.node);
                report.standing.push(found);
            }
            _ => {}
        }
    }
}

fn position(at: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", at.x, at.y, at.z)
}

/// A list of nodes, the closest first, at most `most`.
fn nodes(f: &mut fmt::Formatter, title: &str, list: &[Untouched], most: usize) -> fmt::Result {
    if list.is_empty() {
        return Ok(());
    }
    let mut sorted: Vec<&Untouched> = list.iter().collect();
    sorted.sort_by(|a, b| a.closest.total_cmp(&b.closest));
    let shown: Vec<String> = sorted
        .iter()
        .take(most)
        .map(|u| {
            if u.closest.is_finite() {
                format!(
                    "\"{}\" ({}, {:.1} m away at closest)",
                    u.name, u.kind, u.closest
                )
            } else {
                format!("\"{}\" ({}, never there)", u.name, u.kind)
            }
        })
        .collect();
    let more = list.len().saturating_sub(most);
    write!(f, "{title} ({}): {}", list.len(), shown.join(", "))?;
    if more > 0 {
        write!(f, ", and {more} more")?;
    }
    writeln!(f)
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Played {:.2} s.", self.time)?;
        writeln!(f, "Events:")?;
        if self.events.is_empty() {
            writeln!(f, "  none")?;
        }
        for e in &self.events {
            writeln!(f, "  {:7.2} s  [step {}] {}", e.time, e.step + 1, e.what)?;
        }
        if self.more_events > 0 {
            writeln!(f, "  … and {} more", self.more_events)?;
        }
        writeln!(f, "Steps:")?;
        for (i, s) in self.steps.iter().enumerate() {
            write!(f, "  {}. {}: ", i + 1, s.label)?;
            if s.skipped {
                writeln!(f, "not played (the game had ended)")?;
                continue;
            }
            match (s.reached, s.closest) {
                (Some(t), _) => write!(f, "reached at {t:.2} s, ")?,
                (None, Some(c)) => write!(f, "not reached ({c:.2} m away at closest), ")?,
                _ => {}
            }
            if let Some((t, at)) = s.stuck {
                write!(f, "stuck at {} from {t:.2} s, ", position(at))?;
            }
            writeln!(f, "ended at {}", position(s.at))?;
        }
        write!(
            f,
            "End: at {}, health {}/{}",
            position(self.position),
            self.health.0,
            self.health.1
        )?;
        if let Some(lives) = self.lives {
            write!(f, ", {lives} lives")?;
        }
        let carried: Vec<String> = self
            .carried
            .iter()
            .map(|(item, n)| format!("{item} x{n}"))
            .collect();
        if carried.is_empty() {
            writeln!(f, ", carrying nothing.")?;
        } else {
            writeln!(f, ", carrying {}.", carried.join(", "))?;
        }
        match (self.won, self.over) {
            (Some(t), _) => writeln!(f, "Won at {t:.2} s.")?,
            (None, Some(t)) => writeln!(f, "Game over at {t:.2} s.")?,
            _ => writeln!(f, "Not won.")?,
        }
        nodes(f, "Never reached", &self.never_reached, 20)?;
        nodes(f, "Still standing", &self.standing, 20)?;
        if !self.problems.is_empty() {
            writeln!(f, "Problems:")?;
            for p in &self.problems {
                writeln!(f, "- {p}")?;
            }
        }
        Ok(())
    }
}

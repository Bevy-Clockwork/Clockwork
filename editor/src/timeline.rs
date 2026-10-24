//! An action's timeline in the Inspector: when its hits land, what it spawns and when, when it
//! can't be hurt and when another action may cut in, drawn on a time line that plays the action
//! back. Dragging a time writes it into the plugin's `recipe.ron`, changing only that number.

use std::collections::HashMap;

use bevy_egui::egui::{
    self, Color32, ColorImage, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Ui,
    pos2, vec2,
};
use clockwork_core::{action::Action, asset::OutputRef, recipe::Recipe, sprite::Sprite};
use clockwork_recipe::text::{Doc, Kind, Node};

use crate::{
    asset_edit::{find_output, recipe_path},
    kit,
    state::{EditorState, LogLevel},
    theme,
    workshop::Extractions,
};

/// Times snap to the frames of a 60 Hz game.
const FRAME: f32 = 1.0 / 60.0;
/// The height of a row, and the width of the labels before the rows.
const ROW: f32 = 20.0;
const LABEL: f32 = 78.0;

const HIT: Color32 = Color32::from_rgb(0xE5, 0x6E, 0x5C);
const SAFE: Color32 = Color32::from_rgb(0x6F, 0xB3, 0xA2);
const SPAWN: Color32 = Color32::from_rgb(0x9A, 0x8F, 0xD0);

/// A time of an action that can be dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Duration,
    Cancel,
    InvulnerableFrom,
    InvulnerableUntil,
    HitFrom(usize),
    HitUntil(usize),
    Spawn(usize),
}

/// A step from an action's text to one of its times.
#[derive(Debug, PartialEq)]
enum Seg {
    Field(&'static str),
    /// An item of a list or tuple.
    Item(usize),
}

impl Handle {
    fn get(self, a: &Action) -> Option<f32> {
        match self {
            Handle::Duration => Some(a.duration),
            Handle::Cancel => a.cancel_from,
            Handle::InvulnerableFrom => a.invulnerable.map(|(from, _)| from),
            Handle::InvulnerableUntil => a.invulnerable.map(|(_, until)| until),
            Handle::HitFrom(i) => a.hits.get(i).map(|h| h.from),
            Handle::HitUntil(i) => a.hits.get(i).map(|h| h.until),
            Handle::Spawn(i) => a.spawns.get(i).map(|s| s.at),
        }
    }

    /// Sets the time, kept within what is around it (a hit ends after it starts).
    fn set(self, a: &mut Action, t: f32) {
        let t = t.max(0.0);
        match self {
            Handle::Duration => a.duration = t.max(FRAME),
            Handle::Cancel => a.cancel_from = Some(t),
            Handle::InvulnerableFrom => {
                if let Some((from, until)) = &mut a.invulnerable {
                    *from = t.min(*until);
                }
            }
            Handle::InvulnerableUntil => {
                if let Some((from, until)) = &mut a.invulnerable {
                    *until = t.max(*from);
                }
            }
            Handle::HitFrom(i) => {
                if let Some(h) = a.hits.get_mut(i) {
                    h.from = t.min(h.until);
                }
            }
            Handle::HitUntil(i) => {
                if let Some(h) = a.hits.get_mut(i) {
                    h.until = t.max(h.from);
                }
            }
            Handle::Spawn(i) => {
                if let Some(s) = a.spawns.get_mut(i) {
                    s.at = t;
                }
            }
        }
    }

    /// Where the time is written, under the action.
    fn path(self) -> Vec<Seg> {
        use Seg::*;
        match self {
            Handle::Duration => vec![Field("duration")],
            Handle::Cancel => vec![Field("cancel_from")],
            Handle::InvulnerableFrom => vec![Field("invulnerable"), Item(0)],
            Handle::InvulnerableUntil => vec![Field("invulnerable"), Item(1)],
            Handle::HitFrom(i) => vec![Field("hits"), Item(i), Field("from")],
            Handle::HitUntil(i) => vec![Field("hits"), Item(i), Field("until")],
            Handle::Spawn(i) => vec![Field("spawns"), Item(i), Field("at")],
        }
    }
}

/// The action shown, where playback is, and a time being dragged or just written.
#[derive(Default)]
pub struct Timeline {
    /// The character and the id of its action shown.
    shown: Option<(OutputRef, String)>,
    time: f32,
    playing: bool,
    /// The action as dragged, until the drag ends, then as written until the plugin is read
    /// again with it.
    dragged: Option<(Handle, Action)>,
    written: Option<Action>,
    /// Preview textures, by sprite and frame.
    frames: HashMap<(String, usize), TextureHandle>,
    message: Option<String>,
    /// A plugin to run so its character's actions can be shown.
    pub wanted: Option<String>,
}

/// The actions of a character asset (a sprite or model output with a character), with its
/// timeline. Nothing is shown for assets without actions.
pub fn actions_section(
    ui: &mut Ui,
    state: &mut EditorState,
    at: &OutputRef,
    kind: &str,
    extractions: &Extractions,
) {
    if !matches!(kind, "Sprite" | "Model") {
        return;
    }
    let Some(extracted) = extractions.get(&at.plugin) else {
        // Its actions show once its plugin has run.
        state.workshop.timeline.wanted = Some(at.plugin.clone());
        return;
    };
    let (actions, sprite) = match kind {
        "Sprite" => match extracted.sprite(&at.output) {
            Some(s) => match &s.character {
                Some(c) => (c.actions.clone(), Some(&s.sprite)),
                None => return,
            },
            None => return,
        },
        "Model" => match extracted
            .model(&at.output)
            .and_then(|m| m.character.as_ref())
        {
            Some(c) => (c.actions.clone(), None),
            None => return,
        },
        _ => return,
    };
    if actions.is_empty() {
        return;
    }
    let mut timeline = std::mem::take(&mut state.workshop.timeline);
    kit::foldout(ui, ("asset_actions", at), "Actions", true, |ui| {
        let mut id = match &timeline.shown {
            Some((shown, id)) if shown == at && actions.iter().any(|a| a.id == *id) => id.clone(),
            _ => actions[0].id.clone(),
        };
        let before = id.clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("action", at))
                .selected_text(&id)
                .show_ui(ui, |ui| {
                    for a in &actions {
                        ui.selectable_value(&mut id, a.id.clone(), &a.id);
                    }
                });
            let play = if timeline.playing { "⏸" } else { "▶" };
            if kit::icon_button(ui, play, "Play the action again and again").clicked() {
                timeline.playing = !timeline.playing;
            }
            ui.label(kit::hint(format!(
                "{:.2} s · frame {}",
                timeline.time,
                (timeline.time / FRAME).round()
            )));
        });
        if timeline.shown.as_ref() != Some(&(at.clone(), id.clone())) || id != before {
            timeline.shown = Some((at.clone(), id.clone()));
            timeline.time = 0.0;
            timeline.dragged = None;
            timeline.written = None;
            timeline.message = None;
        }
        let index = actions.iter().position(|a| a.id == id).unwrap_or(0);
        let extracted_action = &actions[index];
        // What was written shows until the plugin is read again with it.
        if timeline
            .written
            .as_ref()
            .is_some_and(|w| w == extracted_action)
        {
            timeline.written = None;
        }
        let base = timeline
            .written
            .clone()
            .unwrap_or_else(|| extracted_action.clone());

        if let Some(sprite) = sprite {
            let shown = timeline.dragged.as_ref().map_or(&base, |(_, a)| a).clone();
            preview(ui, &mut timeline, &shown, sprite, &at.to_string());
        }
        if let Some((handle, value)) = draw(ui, &mut timeline, &base) {
            match write(state, at, kind, index, handle, value) {
                Ok(()) => {
                    state.log(LogLevel::Info, format!("Saved {at}: {id}"));
                    timeline.message = None;
                }
                Err(why) => {
                    timeline.written = None;
                    timeline.message = Some(why);
                }
            }
        }
        if let Some(why) = &timeline.message {
            ui.label(RichText::new(why).color(theme::ERROR));
        }
        ui.label(kit::hint(
            "Drag the edges of a bar or a marker; click the ruler to look at a moment.",
        ));
    });
    state.workshop.timeline = timeline;
}

/// The end of the time line: a little past the last time of the action.
fn span(action: &Action) -> f32 {
    let mut end = action.duration;
    for h in &action.hits {
        end = end.max(h.until);
    }
    for s in &action.spawns {
        end = end.max(s.at);
    }
    if let Some((_, until)) = action.invulnerable {
        end = end.max(until);
    }
    end = end.max(action.cancel_from.unwrap_or(0.0));
    if action.hold.is_some() {
        end += 0.3;
    }
    (end * 1.15).max(0.25)
}

/// The sprite's frame at the playback time, facing the camera: the action's animation, then its
/// hold.
fn preview(ui: &mut Ui, timeline: &mut Timeline, action: &Action, sprite: &Sprite, key: &str) {
    let (animation, t) = match &action.hold {
        Some(hold) if timeline.time >= action.duration => {
            (&hold.animation.south, timeline.time - action.duration)
        }
        _ => (&action.animation.south, timeline.time),
    };
    let Some((step, _)) = sprite
        .animations
        .iter()
        .find(|a| a.name == animation.name)
        .and_then(|a| a.step_at(t))
    else {
        ui.label(kit::hint(format!("No animation \"{}\"", animation.name)));
        return;
    };
    let Some(frame) = sprite.frames.get(step.frame) else {
        return;
    };
    let texture = timeline
        .frames
        .entry((key.to_string(), step.frame))
        .or_insert_with(|| {
            let image = &frame.image;
            let color = ColorImage::from_rgba_unmultiplied(
                [image.width as usize, image.height as usize],
                &image.pixels,
            );
            ui.ctx().load_texture(
                format!("timeline {key} {}", step.frame),
                color,
                TextureOptions::NEAREST,
            )
        });
    let size = texture.size_vec2();
    let scale = (96.0 / size.y.max(1.0)).clamp(1.0, 6.0).floor();
    let flip = step.flip_x != animation.flip_x;
    let uv = if flip {
        Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0))
    } else {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
    };
    let hitting = action
        .hits
        .iter()
        .any(|h| (h.from..=h.until).contains(&timeline.time));
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), size.y * scale + 8.0),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(80));
    let image = Rect::from_center_size(rect.center(), size * scale);
    painter.image(texture.id(), image, uv, Color32::WHITE);
    if hitting {
        painter.rect_stroke(
            image.expand(2.0),
            2.0,
            Stroke::new(2.0, HIT),
            egui::StrokeKind::Outside,
        );
        painter.text(
            rect.left_top() + vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            "hit",
            egui::FontId::proportional(12.0),
            HIT,
        );
    }
}

/// What a row of the time line shows.
enum Mark {
    /// From one time to another, either end dragged by its handle (`None`: fixed).
    Bar(Option<Handle>, f32, Option<Handle>, f32, Color32, String),
    /// One time.
    Pin(Handle, f32, Color32, String),
}

/// Draws the time line of `base` (as it is while a time is dragged); a drag that ended gives the
/// time it moved and where to.
fn draw(ui: &mut Ui, timeline: &mut Timeline, base: &Action) -> Option<(Handle, f32)> {
    let shown = timeline.dragged.as_ref().map_or(base, |(_, a)| a).clone();
    let action = &shown;
    // The scale stays as it was while dragging, so a time dragged past the end doesn't run off.
    let end = span(base);
    let dim = theme::PARCHMENT_DIM;
    let mut rows: Vec<(String, Vec<Mark>)> = vec![(
        "animation".into(),
        vec![Mark::Bar(
            None,
            0.0,
            Some(Handle::Duration),
            action.duration,
            theme::BRASS,
            String::new(),
        )],
    )];
    if let Some(row) = rows.first_mut()
        && action.hold.is_some()
    {
        row.1.push(Mark::Bar(
            None,
            action.duration,
            None,
            end,
            theme::BRASS.gamma_multiply(0.35),
            "held".into(),
        ));
    }
    if let Some(t) = action.cancel_from {
        rows.push((
            "can cut in".into(),
            vec![Mark::Pin(
                Handle::Cancel,
                t,
                theme::BRASS_BRIGHT,
                String::new(),
            )],
        ));
    }
    if let Some((from, until)) = action.invulnerable {
        rows.push((
            "unhurt".into(),
            vec![Mark::Bar(
                Some(Handle::InvulnerableFrom),
                from,
                Some(Handle::InvulnerableUntil),
                until,
                SAFE,
                String::new(),
            )],
        ));
    }
    for (i, h) in action.hits.iter().enumerate() {
        rows.push((
            format!("hit {}", i + 1),
            vec![Mark::Bar(
                Some(Handle::HitFrom(i)),
                h.from,
                Some(Handle::HitUntil(i)),
                h.until,
                HIT,
                format!("{} dmg", h.damage),
            )],
        ));
    }
    for (i, s) in action.spawns.iter().enumerate() {
        rows.push((
            "spawns".into(),
            vec![Mark::Pin(
                Handle::Spawn(i),
                s.at,
                SPAWN,
                s.projectile.clone(),
            )],
        ));
    }

    let width = ui.available_width().max(LABEL + 80.0);
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, ROW * (rows.len() + 1) as f32 + 4.0),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let left = rect.left() + LABEL;
    let x_of = |t: f32| left + (t / end) * (rect.right() - left - 6.0);
    let t_of = |x: f32| ((x - left) / (rect.right() - left - 6.0) * end).max(0.0);
    let snap = |t: f32| ((t / FRAME).round() * FRAME * 1000.0).round() / 1000.0;

    // The ruler: click or drag on it to look at a moment.
    let ruler = Rect::from_min_max(pos2(left, rect.top()), pos2(rect.right(), rect.top() + ROW));
    let response = ui.interact(
        ruler,
        ui.id().with("timeline ruler"),
        Sense::click_and_drag(),
    );
    if let Some(p) = response.interact_pointer_pos() {
        timeline.time = snap(t_of(p.x)).min(end);
        timeline.playing = false;
    }
    let step = if end > 2.0 {
        0.5
    } else if end > 0.8 {
        0.2
    } else {
        0.1
    };
    let mut t = 0.0;
    while t <= end {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, ruler.bottom() - 5.0), pos2(x, ruler.bottom())],
            Stroke::new(1.0, dim),
        );
        painter.text(
            pos2(x + 2.0, ruler.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{t:.1}"),
            egui::FontId::proportional(10.0),
            dim,
        );
        t += step;
    }

    let mut ended = None;
    for (row, (label, marks)) in rows.iter().enumerate() {
        let top = rect.top() + ROW * (row + 1) as f32;
        let band = Rect::from_min_max(pos2(left, top + 3.0), pos2(rect.right(), top + ROW - 3.0));
        painter.text(
            pos2(rect.left(), band.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            dim,
        );
        painter.rect_filled(band, 2.0, Color32::from_white_alpha(6));
        let mut handles: Vec<(Handle, f32)> = Vec::new();
        for mark in marks {
            match mark {
                Mark::Bar(from_h, from, until_h, until, color, text) => {
                    let active = (*from..=*until).contains(&timeline.time);
                    let bar = Rect::from_min_max(
                        pos2(x_of(*from), band.top()),
                        pos2(x_of(*until).max(x_of(*from) + 2.0), band.bottom()),
                    );
                    let color = if active {
                        *color
                    } else {
                        color.gamma_multiply(0.6)
                    };
                    painter.rect_filled(bar, 2.0, color);
                    if !text.is_empty() {
                        painter.text(
                            bar.left_center() + vec2(4.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(11.0),
                            Color32::BLACK,
                        );
                    }
                    handles.extend(from_h.map(|h| (h, *from)));
                    handles.extend(until_h.map(|h| (h, *until)));
                }
                Mark::Pin(h, at, color, text) => {
                    let x = x_of(*at);
                    painter.line_segment(
                        [pos2(x, band.top()), pos2(x, band.bottom())],
                        Stroke::new(2.0, *color),
                    );
                    painter.circle_filled(pos2(x, band.center().y), 4.0, *color);
                    if !text.is_empty() {
                        painter.text(
                            pos2(x + 7.0, band.center().y),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(11.0),
                            *color,
                        );
                    }
                    handles.push((*h, *at));
                }
            }
        }
        for (handle, at) in handles {
            let grip = Rect::from_center_size(pos2(x_of(at), band.center().y), vec2(8.0, ROW));
            let response = ui
                .interact(
                    grip,
                    ui.id().with(("timeline handle", format!("{handle:?}"))),
                    Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text(format!("{at:.3} s · frame {}", (at / FRAME).round()));
            if response.drag_started() {
                timeline.dragged = Some((handle, base.clone()));
                timeline.playing = false;
            }
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos()
                && let Some((h, dragged)) = &mut timeline.dragged
                && *h == handle
            {
                let t = snap(t_of(p.x));
                handle.set(dragged, t);
                timeline.time = handle.get(dragged).unwrap_or(t);
            }
            if response.drag_stopped()
                && let Some((h, dragged)) = timeline.dragged.take()
                && h == handle
                && let Some(value) = handle.get(&dragged)
                && Some(value) != handle.get(base)
            {
                timeline.written = Some(dragged);
                ended = Some((handle, value));
            }
        }
    }

    // The playhead.
    let x = x_of(timeline.time);
    painter.line_segment(
        [pos2(x, rect.top()), pos2(x, rect.bottom())],
        Stroke::new(1.5, theme::BRASS_BRIGHT),
    );
    if timeline.playing {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        timeline.time += dt;
        if timeline.time > end {
            timeline.time = 0.0;
        }
        ui.ctx().request_repaint();
    }
    ended
}

/// A time as RON writes a number: always with a decimal point.
fn number(value: f32) -> String {
    let text = format!("{value}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// The node at `path` under `node`, looking through `Some(…)`.
fn under<'a>(mut node: &'a Node, path: &[Seg]) -> Option<&'a Node> {
    for seg in path {
        let here = node.unwrap_some();
        node = match seg {
            Seg::Field(key) => here.field(key)?,
            Seg::Item(i) => match &here.kind {
                Kind::Seq(items) | Kind::Tuple { items, .. } => items.get(*i)?,
                _ => return None,
            },
        };
    }
    Some(node.unwrap_some())
}

/// `text` (a recipe) with one time of an action of a character output set to `value`.
fn with_time(
    text: &str,
    kind: &str,
    output: &str,
    action: usize,
    handle: Handle,
    value: f32,
) -> Result<String, String> {
    let doc = Doc::parse(text.trim_start_matches('\u{FEFF}')).map_err(|e| e.to_string())?;
    let item = find_output(&doc, kind, output).ok_or("the asset is gone")?;
    let mut path = vec![
        Seg::Field("character"),
        Seg::Field("actions"),
        Seg::Item(action),
    ];
    path.extend(handle.path());
    let node = under(item, &path).ok_or("that time isn't written in the recipe")?;
    let text = doc.replaced(node.span.clone(), &number(value));
    ron::from_str::<Recipe>(&text).map_err(|e| format!("Not saved: {e}"))?;
    Ok(text)
}

fn write(
    state: &EditorState,
    at: &OutputRef,
    kind: &str,
    action: usize,
    handle: Handle,
    value: f32,
) -> Result<(), String> {
    let path = recipe_path(state, &at.plugin).ok_or("the plugin isn't installed")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let text = with_time(&text, kind, &at.output, action, handle, value)?;
    clockwork_library::files::write_atomic(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = r#"(
    steps: [],
    outputs: [
        // The hero.
        Sprite(id: "hero", from: "hero", pixels_per_meter: 16.0, character: Some((
            movement: (speed: 4.0, radius: 0.4),
            animations: (
                idle: (north: (name: "idle"), east: (name: "idle"), south: (name: "idle"), west: (name: "idle")),
                walk: (north: (name: "walk"), east: (name: "walk"), south: (name: "walk"), west: (name: "walk")),
            ),
            actions: [(
                id: "slash",
                animation: (north: (name: "slash"), east: (name: "slash"), south: (name: "slash"), west: (name: "slash")),
                duration: 0.4,
                hits: [(from: 0.1, until: 0.2, area: Around(radius: 1.0), damage: 1.0)],
                invulnerable: Some((0.0, 0.15)),
            )],
        ))),
    ],
)"#;

    #[test]
    fn a_dragged_time_is_written_in_place() {
        let text = with_time(RECIPE, "Sprite", "hero", 0, Handle::HitUntil(0), 0.25).unwrap();
        assert!(text.contains("until: 0.25"), "{text}");
        assert!(text.contains("// The hero."), "comments stay");
        let text = with_time(&text, "Sprite", "hero", 0, Handle::InvulnerableUntil, 0.3).unwrap();
        assert!(text.contains("Some((0.0, 0.3))"), "{text}");
        let text = with_time(&text, "Sprite", "hero", 0, Handle::Duration, 1.0).unwrap();
        assert!(text.contains("duration: 1.0,"), "{text}");
        // A time the recipe leaves out can't be dragged into it.
        assert!(with_time(&text, "Sprite", "hero", 0, Handle::Cancel, 0.2).is_err());
    }

    #[test]
    fn handles_keep_hits_in_order() {
        let recipe: Recipe = ron::from_str(RECIPE).unwrap();
        let clockwork_core::recipe::Output::Sprite {
            character: Some(c), ..
        } = &recipe.outputs[0]
        else {
            panic!("not a character");
        };
        let mut action = c.actions[0].clone();
        Handle::HitFrom(0).set(&mut action, 0.5);
        assert_eq!(action.hits[0].from, 0.2, "a hit can't start after it ends");
        Handle::HitUntil(0).set(&mut action, 0.05);
        assert_eq!(action.hits[0].until, 0.2);
        assert!(span(&action) > action.duration);
    }
}

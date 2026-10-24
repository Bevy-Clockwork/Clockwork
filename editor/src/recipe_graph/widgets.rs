//! The controls inside nodes: name menus, value fields, the ROM and region pickers, previews
//! and the add list.

use std::collections::HashMap;

use bevy_egui::egui::{
    self, Color32, ColorImage, Pos2, Rect, RichText, Stroke, TextureHandle, TextureOptions, Ui,
    vec2,
};
use clockwork_core::{
    image::RgbaImage,
    recipe::NameKind,
    recipe_schema::{self, FieldInfo, FieldType},
};
use clockwork_recipe::text::{format_int_like, parse_int, parse_string, string_literal};

use super::{OwnedPort, RNode, blocks, node_key, template_kinds, templates};
use crate::{kit, theme};

/// The names of `kind` made before `node` runs, in the order they are first made.
pub(super) fn candidates(
    ports: &HashMap<RNode, Vec<OwnedPort>>,
    steps: usize,
    node: RNode,
    kind: NameKind,
) -> Vec<String> {
    let before = match node {
        RNode::Step(i) => i,
        RNode::Output(_) => steps,
    };
    let mut names: Vec<String> = Vec::new();
    for i in 0..before {
        for port in ports.get(&RNode::Step(i)).into_iter().flatten() {
            if port.kind == kind
                && port.access.writes()
                && !port.name.is_empty()
                && !names.contains(&port.name)
            {
                names.push(port.name.clone());
            }
        }
    }
    names
}

/// A menu of the names an input can read, plus a field for any other name; the one picked.
pub(super) fn name_menu(
    ui: &mut Ui,
    names: &[String],
    current: &str,
    kind: NameKind,
) -> Option<String> {
    let mut picked = None;
    if names.is_empty() {
        ui.label(kit::dim(format!(
            "No earlier step makes a {}",
            kind.label()
        )));
    }
    for name in names {
        if ui.selectable_label(name == current, name).clicked() {
            picked = Some(name.clone());
        }
    }
    ui.separator();
    if ui
        .selectable_label(current.is_empty(), "— none —")
        .clicked()
    {
        picked = Some(String::new());
    }
    let other = egui::Id::new(("other name", kind.label()));
    let mut text = ui.data(|d| d.get_temp::<String>(other)).unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .hint_text(kit::hint("Other name, Enter"))
            .desired_width(160.0),
    );
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(other, text.clone()));
    }
    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        ui.data_mut(|d| d.remove::<String>(other));
        if !text.trim().is_empty() {
            picked = Some(text.trim().to_string());
        }
    }
    if picked.is_some() {
        ui.close();
    }
    picked
}

/// A text field kept in `edits` while it is typed in, and given back once it is left with text
/// `accept` takes (its text as written then). Text it doesn't take is outlined in red.
pub(super) fn text_field(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    text: &str,
    multiline: bool,
    accept: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut buffer = edits.get(key).cloned().unwrap_or_else(|| text.to_string());
    let ok = accept(&buffer).is_some();
    let editor = if multiline {
        egui::TextEdit::multiline(&mut buffer)
            .code_editor()
            .desired_rows(text.lines().count().clamp(2, 12))
            .desired_width(f32::INFINITY)
    } else {
        egui::TextEdit::singleline(&mut buffer).desired_width(ui.available_width().min(220.0))
    };
    let editor = if ok {
        editor
    } else {
        editor.background_color(theme::ERROR.gamma_multiply(0.25))
    };
    let response = ui.add(editor.id_salt(("value", node_key(key.0), &key.1)));
    if response.changed() {
        edits.insert(key.clone(), buffer.clone());
    }
    if response.lost_focus() {
        edits.remove(key);
        if buffer != text {
            return accept(&buffer);
        }
    }
    None
}

/// A labelled row (or block, for bigger values) editing one value; its new text once changed.
pub(super) fn value_row(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    about: Option<&FieldInfo>,
    ty: FieldType,
    text: &str,
) -> Option<String> {
    labeled_value_row(ui, &key.1, edits, key, about, ty, text)
}

/// A [`value_row`] under another label than the field's name.
pub(super) fn labeled_value_row(
    ui: &mut Ui,
    label: &str,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    about: Option<&FieldInfo>,
    ty: FieldType,
    text: &str,
) -> Option<String> {
    let block = matches!(ty, FieldType::Structure)
        || matches!(ty, FieldType::Optional(FieldType::Structure)) && text != "None";
    if block {
        let label = ui.label(kit::dim(label));
        if let Some(about) = about {
            label.on_hover_text(about.about);
        }
        edit_value(ui, edits, key, ty, text)
    } else {
        kit::prop(ui, label, |ui| edit_value(ui, edits, key, ty, text))
    }
}

/// An editor for a value by what it holds; its new text once changed.
pub(super) fn edit_value(
    ui: &mut Ui,
    edits: &mut HashMap<(RNode, String), String>,
    key: &(RNode, String),
    ty: FieldType,
    text: &str,
) -> Option<String> {
    match ty {
        // Offsets are typed, never dragged: a stray drag would move them silently.
        FieldType::Offset => text_field(ui, edits, key, text, false, |t| {
            parse_int(t.trim()).map(|_| t.trim().to_string())
        }),
        FieldType::Count => match parse_int(text).filter(|_| !text.contains('.')) {
            Some(mut n) => {
                let like = text.to_string();
                let response = ui.add(
                    egui::DragValue::new(&mut n)
                        .speed(0.25)
                        .custom_formatter(move |v, _| format_int_like(v as i64, &like))
                        .custom_parser(|s| parse_int(s).map(|v| v as f64)),
                );
                response.changed().then(|| format_int_like(n, text))
            }
            None => text_field(ui, edits, key, text, false, |t| {
                parse_int(t.trim()).map(|_| t.trim().to_string())
            }),
        },
        FieldType::Number => match text.parse::<f64>() {
            Ok(mut f) => {
                let response = ui.add(egui::DragValue::new(&mut f).speed(0.01));
                response.changed().then(|| format!("{f:?}"))
            }
            Err(_) => text_field(ui, edits, key, text, false, |t| {
                t.trim().parse::<f64>().ok().map(|f| format!("{f:?}"))
            }),
        },
        FieldType::Flag => {
            let mut on = text.trim() == "true";
            ui.checkbox(&mut on, "").changed().then(|| on.to_string())
        }
        FieldType::Choice(variants) => {
            let mut picked = None;
            egui::ComboBox::from_id_salt(("choice", node_key(key.0), &key.1))
                .selected_text(text.trim())
                .show_ui(ui, |ui| {
                    for variant in variants {
                        if ui
                            .selectable_label(text.trim() == *variant, *variant)
                            .clicked()
                        {
                            picked = Some(variant.to_string());
                        }
                    }
                });
            picked
        }
        FieldType::Optional(inner) => {
            let value = text
                .trim()
                .strip_prefix("Some(")
                .and_then(|t| t.strip_suffix(')'))
                .map(str::trim);
            let mut set = value.is_some();
            let mut new = None;
            let mut row = |ui: &mut Ui| {
                if ui
                    .checkbox(&mut set, "")
                    .on_hover_text("Whether it is set; left out, it isn't")
                    .changed()
                {
                    new = Some(if set {
                        format!("Some({})", inner.default_text())
                    } else {
                        "None".to_string()
                    });
                }
                match value {
                    Some(value) => {
                        let key = (key.0, format!("{}/some", key.1));
                        if let Some(value) = edit_value(ui, edits, &key, *inner, value) {
                            new = Some(format!("Some({value})"));
                        }
                    }
                    None => {
                        ui.label(kit::dim("not set"));
                    }
                }
            };
            if *inner == FieldType::Structure {
                ui.vertical(|ui| row(ui));
            } else {
                ui.horizontal(|ui| row(ui));
            }
            new
        }
        FieldType::Name | FieldType::Text => match parse_string(text) {
            Some(string) => text_field(ui, edits, key, &string, false, |t| Some(string_literal(t))),
            None => text_field(ui, edits, key, text, false, |t| Some(t.to_string())),
        },
        // Shown without the indentation it has in the file, and given it back once changed.
        FieldType::Structure => {
            let indent = indent_of(text);
            let shown = reindented(text, &indent, "");
            text_field(ui, edits, key, &shown, true, |t| {
                Some(reindented(t, "", &indent))
            })
        }
    }
}

/// `(x: 0, y: 8, w: 16, h: 16)` read as (x, y, w, h).
pub(super) fn parse_region(text: &str) -> Option<(u32, u32, u32, u32)> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut values = HashMap::new();
    for part in inner.split(',').filter(|p| !p.trim().is_empty()) {
        let (key, value) = part.split_once(':')?;
        values.insert(key.trim(), u32::try_from(parse_int(value.trim())?).ok()?);
    }
    Some((
        *values.get("x")?,
        *values.get("y")?,
        *values.get("w")?,
        *values.get("h")?,
    ))
}

/// The image a region is cut from (`full` pixels, shown as `texture`), the region outlined;
/// a drag draws a new one, in whole pixels. Returns it when the drag ends.
pub(super) fn region_picker(
    ui: &mut Ui,
    texture: &TextureHandle,
    full: (u32, u32),
    region: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let (fw, fh) = (full.0.max(1) as f32, full.1.max(1) as f32);
    let width = ui.available_width().min(320.0);
    let size = vec2(width, width * fh / fw).min(vec2(width, 320.0));
    let size = vec2(size.y * fw / fh, size.y).min(size);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::BG_DEEP);
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let scale = size.x / fw;
    let to_pixel = |p: Pos2| {
        let v = (p - rect.min) / scale;
        (
            v.x.clamp(0.0, fw).round() as u32,
            v.y.clamp(0.0, fh).round() as u32,
        )
    };
    let to_screen = |(x, y): (u32, u32)| rect.min + vec2(x as f32, y as f32) * scale;
    let id = response.id.with("start");
    let start: Option<Pos2> = ui.data(|d| d.get_temp(id));
    if response.drag_started()
        && let Some(at) = response.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(id, at));
    }
    let drawn = match (start, response.interact_pointer_pos()) {
        (Some(a), Some(b)) if response.dragged() || response.drag_stopped() => {
            let (a, b) = (to_pixel(a), to_pixel(b));
            let (x, y) = (a.0.min(b.0), a.1.min(b.1));
            Some((x, y, a.0.max(b.0) - x, a.1.max(b.1) - y))
        }
        _ => None,
    };
    let (x, y, w, h) = drawn.unwrap_or(region);
    let shown = Rect::from_min_max(to_screen((x, y)), to_screen((x + w, y + h)));
    painter.rect_stroke(
        shown,
        0.0,
        Stroke::new(2.0, theme::BRASS_BRIGHT),
        egui::StrokeKind::Outside,
    );
    ui.label(kit::dim(format!(
        "x {x}, y {y}, {w}×{h} of {}×{}",
        full.0, full.1
    )));
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Pos2>(id));
        return drawn.filter(|(_, _, w, h)| *w > 0 && *h > 0);
    }
    None
}

/// Sixteen rows of a ROM's bytes from about `offset`, the `size` bytes read from there
/// highlighted; paged with the buttons. Returns the address of a byte clicked.
pub(super) fn rom_picker(
    ui: &mut Ui,
    rom: &[u8],
    offset: usize,
    size: usize,
    node: RNode,
) -> Option<usize> {
    const ROW: usize = 16;
    const PAGE: usize = ROW * 16;
    let id = egui::Id::new(("rom picker start", node_key(node)));
    let last = rom.len().saturating_sub(1) / ROW * ROW;
    let mut start: usize = ui
        .data(|d| d.get_temp(id))
        .unwrap_or(offset / ROW * ROW)
        .min(last);
    ui.horizontal(|ui| {
        if kit::icon_button(ui, "⏪", "Back a page").clicked() {
            start = start.saturating_sub(PAGE);
        }
        if kit::icon_button(ui, "⏩", "On a page").clicked() {
            start = (start + PAGE).min(last);
        }
        if ui
            .small_button("To the offset")
            .on_hover_text("Show where it reads now")
            .clicked()
        {
            start = offset / ROW * ROW;
        }
        ui.label(kit::dim("Click a byte to read from there"));
    });
    ui.data_mut(|d| d.insert_temp(id, start));
    let mut picked = None;
    let font = egui::FontId::monospace(11.5);
    let read = offset..offset + size.max(1);
    ui.spacing_mut().item_spacing.y = 1.0;
    for row in (start..rom.len().min(start + PAGE)).step_by(ROW) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(
                RichText::new(format!("{row:06X}"))
                    .font(font.clone())
                    .color(theme::FAINT),
            );
            for (at, byte) in rom[row..rom.len().min(row + ROW)].iter().enumerate() {
                let at = row + at;
                let color = if read.contains(&at) {
                    theme::BRASS_BRIGHT
                } else {
                    theme::PARCHMENT_DIM
                };
                let text = RichText::new(format!("{byte:02X}"))
                    .font(font.clone())
                    .color(color);
                let response = ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text(format!("{at:#x}"));
                if response.hovered() {
                    ui.painter().rect_stroke(
                        response.rect.expand(1.0),
                        2.0,
                        Stroke::new(1.0, theme::BRASS),
                        egui::StrokeKind::Outside,
                    );
                }
                if response.clicked() {
                    picked = Some(at);
                }
            }
        });
    }
    picked
}

/// The indentation every line but the first of `text` has in common.
pub(super) fn indent_of(text: &str) -> String {
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .min_by_key(|indent| indent.len())
        .unwrap_or_default()
        .to_string()
}

/// `text` with `from` taken off the start of every line but the first, and `to` put there.
pub(super) fn reindented(text: &str, from: &str, to: &str) -> String {
    let mut lines = text.split('\n');
    let mut out = lines.next().unwrap_or_default().to_string();
    for line in lines {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(to);
        }
        out.push_str(line.strip_prefix(from).unwrap_or(line));
    }
    out
}

/// The texture of a node's preview, made again when the recipe ran again.
pub(super) fn thumb(
    ctx: &egui::Context,
    thumbs: &mut HashMap<String, (u64, TextureHandle)>,
    revision: u64,
    node: RNode,
    image: &RgbaImage,
) -> TextureHandle {
    let key = node_key(node);
    match thumbs.get(&key) {
        Some((r, texture)) if *r == revision => texture.clone(),
        _ => {
            let color = ColorImage::from_rgba_unmultiplied(
                [image.width as usize, image.height as usize],
                &image.pixels,
            );
            let texture = ctx.load_texture(format!("recipe-{key}"), color, TextureOptions::NEAREST);
            thumbs.insert(key, (revision, texture.clone()));
            texture
        }
    }
}

/// The steps and outputs that can be added (those `filter` takes by the kinds they read and
/// make), found by typing; the template and whether it is a step, once one is picked.
/// What was picked in [`add_list`].
pub(super) enum Picked {
    /// A step (`true`) or output template.
    Template(String, bool),
    /// An index into [`blocks::BLOCKS`].
    Block(usize),
}

pub(super) fn add_list(
    ui: &mut Ui,
    query: &mut String,
    with_blocks: bool,
    filter: impl Fn(&[NameKind], &[NameKind]) -> bool,
) -> Option<Picked> {
    let field = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(kit::hint("🔍 Search, Enter adds the first"))
            .desired_width(f32::INFINITY),
    );
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let q = query.to_lowercase();
    let mut first = None;
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            let choice = |ui: &mut Ui, name: &str, about: &str| {
                let response = ui
                    .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(name).strong().color(theme::PARCHMENT));
                        ui.label(RichText::new(about).small().color(theme::PARCHMENT_DIM));
                    })
                    .response;
                if response.hovered() {
                    ui.painter().rect_stroke(
                        response.rect.expand(2.0),
                        4.0,
                        Stroke::new(1.0, theme::BRASS_DIM),
                        egui::StrokeKind::Outside,
                    );
                }
                ui.add_space(2.0);
                response.clicked()
            };
            let blocks: Vec<usize> = (0..blocks::BLOCKS.len())
                .filter(|_| with_blocks)
                .filter(|&b| {
                    let block = &blocks::BLOCKS[b];
                    q.is_empty()
                        || block.name.to_lowercase().contains(&q)
                        || block.about.to_lowercase().contains(&q)
                })
                .collect();
            if !blocks.is_empty() {
                ui.label(RichText::new("Blocks").small().color(theme::BRASS));
            }
            for b in blocks {
                let block = &blocks::BLOCKS[b];
                if first.is_none() {
                    first = Some(Picked::Block(b));
                }
                if choice(ui, block.name, block.about) {
                    picked = Some(Picked::Block(b));
                }
            }
            for (heading, templates, step) in [
                ("Steps", templates(true), true),
                ("Outputs", templates(false), false),
            ] {
                let matching: Vec<(&str, &str, &str)> = templates
                    .iter()
                    .filter_map(|(name, template)| {
                        let about = if step {
                            recipe_schema::step(name)
                        } else {
                            recipe_schema::output(name)
                        }
                        .map_or("", |i| i.about);
                        let (reads, writes) = template_kinds(template, step);
                        let found = q.is_empty()
                            || name.to_lowercase().contains(&q)
                            || about.to_lowercase().contains(&q);
                        (found && filter(&reads, &writes)).then_some((
                            *name,
                            template.as_str(),
                            about,
                        ))
                    })
                    .collect();
                if matching.is_empty() {
                    continue;
                }
                ui.label(RichText::new(heading).small().color(theme::BRASS));
                for (name, template, about) in matching {
                    if first.is_none() {
                        first = Some(Picked::Template(template.to_string(), step));
                    }
                    if choice(ui, name, about) {
                        picked = Some(Picked::Template(template.to_string(), step));
                    }
                }
            }
            if first.is_none() {
                ui.label(kit::dim("Nothing matches"));
            }
        });
    if enter && picked.is_none() {
        picked = first;
    }
    if picked.is_some() {
        query.clear();
        ui.close();
    }
    picked
}

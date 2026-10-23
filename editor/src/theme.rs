//! Clockwork's look: a quiet graphite workspace with one brass accent, a clear sans-serif for
//! everything you read and work with, and the serif display face (and gears) kept for the brand.
//!
//! Surfaces step up in small lightness increments (canvas → panel → raised → hover) so depth
//! reads without heavy borders; brass marks what is selected, active or primary; green, amber and
//! red are kept for status.

use std::f32::consts::TAU;
use std::sync::Arc;

use bevy_egui::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Mesh,
    Painter, Pos2, Rect, Shadow, Shape, Stroke, TextStyle, Vec2, pos2, vec2,
};

// Surfaces, darkest first.
/// The canvas behind everything: viewports, graphs, the Library's shelves.
pub const BG_DEEP: Color32 = Color32::from_rgb(0x13, 0x12, 0x10);
/// Docked panels.
pub const PANEL: Color32 = Color32::from_rgb(0x1C, 0x1A, 0x17);
/// Panel headers, toolbars and the status bar.
pub const HEADER: Color32 = Color32::from_rgb(0x22, 0x20, 0x1C);
/// Fields, buttons and cards.
pub const RAISED: Color32 = Color32::from_rgb(0x2A, 0x27, 0x22);
pub const RAISED_HOVER: Color32 = Color32::from_rgb(0x35, 0x31, 0x2A);

// Lines.
pub const BORDER: Color32 = Color32::from_rgb(0x33, 0x2F, 0x29);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x4A, 0x44, 0x3A);

// The accent.
pub const BRASS: Color32 = Color32::from_rgb(0xC9, 0xA2, 0x56);
pub const BRASS_BRIGHT: Color32 = Color32::from_rgb(0xE8, 0xC4, 0x74);
pub const BRASS_DIM: Color32 = Color32::from_rgb(0x7A, 0x63, 0x38);
/// Fill behind selected rows and toggled buttons.
pub const SELECTED: Color32 = Color32::from_rgb(0x4A, 0x3B, 0x20);

// Text.
pub const PARCHMENT: Color32 = Color32::from_rgb(0xE6, 0xE0, 0xD4);
pub const PARCHMENT_DIM: Color32 = Color32::from_rgb(0x9E, 0x96, 0x88);
/// Hints, placeholders and disabled text.
pub const FAINT: Color32 = Color32::from_rgb(0x6C, 0x65, 0x5A);

// Status.
pub const SUCCESS: Color32 = Color32::from_rgb(0x7C, 0xC4, 0x8A);
pub const WARN: Color32 = Color32::from_rgb(0xE3, 0xB2, 0x4F);
pub const ERROR: Color32 = Color32::from_rgb(0xE5, 0x6E, 0x5C);
pub const INFO: Color32 = Color32::from_rgb(0x74, 0xA8, 0xDC);

// The three axes, as in every 3D tool.
pub const AXIS_X: Color32 = Color32::from_rgb(0xE0, 0x5D, 0x5D);
pub const AXIS_Y: Color32 = Color32::from_rgb(0x7F, 0xC0, 0x5A);
pub const AXIS_Z: Color32 = Color32::from_rgb(0x5B, 0x8F, 0xE0);

/// Corner radius of controls; panels and windows use a little more.
pub const RADIUS: u8 = 4;

/// Font family for the brand, screen titles and empty states. Falls back to the UI font.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}

/// The semibold UI face, for labels that need weight without colour.
pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("strong".into()))
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(PARCHMENT);
    v.weak_text_color = Some(PARCHMENT_DIM);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(130),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.extreme_bg_color = BG_DEEP;
    v.text_edit_bg_color = Some(Color32::from_rgb(0x17, 0x16, 0x13));
    v.code_bg_color = RAISED;
    v.faint_bg_color = Color32::from_rgb(0x20, 0x1E, 0x1A);
    v.hyperlink_color = BRASS_BRIGHT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = SELECTED;
    v.selection.stroke = Stroke::new(1.0, BRASS_BRIGHT);
    v.text_cursor.stroke = Stroke::new(1.5, BRASS_BRIGHT);
    v.slider_trailing_fill = true;
    v.indent_has_left_vline = true;
    v.collapsing_header_frame = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    // Panel separators and `ui.separator()` use this.
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, PARCHMENT);
    w.noninteractive.corner_radius = CornerRadius::same(RADIUS);
    for (state, fill, stroke, fg) in [
        (&mut w.inactive, RAISED, BORDER, PARCHMENT),
        (&mut w.hovered, RAISED_HOVER, BORDER_STRONG, Color32::WHITE),
        (
            &mut w.active,
            Color32::from_rgb(0x40, 0x3A, 0x30),
            BRASS,
            BRASS_BRIGHT,
        ),
        (&mut w.open, RAISED_HOVER, BORDER_STRONG, BRASS_BRIGHT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.5, fg);
        state.corner_radius = CornerRadius::same(RADIUS);
        state.expansion = 0.0;
    }
    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        let s = &mut style.spacing;
        s.item_spacing = vec2(6.0, 5.0);
        s.button_padding = vec2(9.0, 3.0);
        s.interact_size = vec2(36.0, 24.0);
        s.menu_margin = egui::Margin::same(6);
        s.window_margin = egui::Margin::same(14);
        s.indent = 16.0;
        s.icon_width = 15.0;
        s.icon_width_inner = 9.0;
        s.combo_width = 120.0;
        s.slider_rail_height = 4.0;
        s.scroll = egui::style::ScrollStyle::thin();

        style.interaction.tooltip_delay = 0.35;
        style.interaction.selectable_labels = false;

        style.text_styles.insert(TextStyle::Heading, display(20.0));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.5));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.5));
    });
}

/// Uses system fonts if they are installed; nothing is bundled or downloaded.
///
/// Each candidate has a downward glyph shift (as a fraction of the font size). Some fonts have
/// tall line boxes, so without it their text sits visibly above the middle of buttons and fields.
fn fonts() -> FontDefinitions {
    const DISPLAY: &[(&str, f32)] = &[
        (r"C:\Windows\Fonts\palab.ttf", 0.16),
        (r"C:\Windows\Fonts\georgiab.ttf", 0.08),
        ("/System/Library/Fonts/Supplemental/Georgia Bold.ttf", 0.08),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf", 0.0),
    ];
    const BODY: &[(&str, f32)] = &[
        (r"C:\Windows\Fonts\segoeui.ttf", 0.02),
        ("/System/Library/Fonts/SFNS.ttf", 0.0),
        ("/usr/share/fonts/truetype/inter/Inter-Regular.ttf", 0.0),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0.0),
    ];
    const STRONG: &[(&str, f32)] = &[
        (r"C:\Windows\Fonts\seguisb.ttf", 0.02),
        ("/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf", 0.0),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0.0),
    ];
    const MONO: &[(&str, f32)] = &[
        (r"C:\Windows\Fonts\CascadiaMono.ttf", 0.0),
        (r"C:\Windows\Fonts\consola.ttf", 0.0),
        ("/System/Library/Fonts/SFNSMono.ttf", 0.0),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0.0),
    ];
    /// Arrows, media and geometric symbols the UI font lacks (▶ ⏸ ◆ …).
    const SYMBOLS: &[(&str, f32)] = &[(r"C:\Windows\Fonts\seguisym.ttf", 0.02)];

    let mut defs = FontDefinitions::default();
    let defaults = defs.families[&FontFamily::Proportional].clone();
    let mut load = |name: &str, candidates: &[(&str, f32)]| {
        let (bytes, shift) = candidates
            .iter()
            .find_map(|&(path, shift)| std::fs::read(path).ok().map(|b| (b, shift)))?;
        let data = FontData::from_owned(bytes).tweak(FontTweak {
            y_offset_factor: shift,
            ..Default::default()
        });
        defs.font_data.insert(name.into(), Arc::new(data));
        Some(name.to_string())
    };
    let display = load("clockwork-display", DISPLAY);
    let body = load("clockwork-body", BODY);
    let strong = load("clockwork-strong", STRONG);
    let mono = load("clockwork-mono", MONO);
    let symbols = load("clockwork-symbols", SYMBOLS);

    // Proportional: the UI font, then egui's own (which carry its emoji and icon glyphs), then
    // the symbol font for anything still missing.
    let mut proportional = defaults.clone();
    if let Some(name) = &body {
        proportional.insert(0, name.clone());
    }
    if let Some(name) = &symbols {
        proportional.push(name.clone());
    }
    let with_first = |first: Option<String>| {
        let mut family = proportional.clone();
        if let Some(name) = first {
            family.insert(0, name);
        }
        family
    };
    defs.families
        .insert(FontFamily::Name("display".into()), with_first(display));
    defs.families
        .insert(FontFamily::Name("strong".into()), with_first(strong));
    if let Some(name) = mono {
        defs.families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .insert(0, name);
    }
    defs.families.insert(FontFamily::Proportional, proportional);
    defs
}

/// Paints a gear. `rotation` is in radians.
pub fn gear(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    teeth: usize,
    rotation: f32,
    fill: Color32,
    hole: Color32,
) {
    let tooth_len = radius * 0.2;
    let body = radius - tooth_len;
    let tooth_w = TAU * body / teeth as f32 * 0.5;
    for i in 0..teeth {
        let a = rotation + i as f32 * TAU / teeth as f32;
        let dir = Vec2::angled(a);
        let perp = vec2(-dir.y, dir.x);
        let base = center + dir * (body - 1.0);
        let tip = center + dir * radius;
        painter.add(Shape::convex_polygon(
            vec![
                base - perp * tooth_w * 0.5,
                base + perp * tooth_w * 0.5,
                tip + perp * tooth_w * 0.32,
                tip - perp * tooth_w * 0.32,
            ],
            fill,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(center, body, fill);
    painter.circle_stroke(center, body * 0.72, Stroke::new(body * 0.06, hole));
    painter.circle_filled(center, body * 0.28, hole);
}

pub fn vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A brass frame with rivets in the corners.
pub fn riveted_frame(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    let inset = 6.0;
    for corner in [
        pos2(rect.left() + inset, rect.top() + inset),
        pos2(rect.right() - inset, rect.top() + inset),
        pos2(rect.right() - inset, rect.bottom() - inset),
        pos2(rect.left() + inset, rect.bottom() - inset),
    ] {
        painter.circle_filled(corner, 2.2, color);
    }
}

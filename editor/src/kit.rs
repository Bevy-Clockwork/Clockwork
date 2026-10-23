//! Building blocks every screen shares, so panels, toolbars and inspectors look and behave the
//! same everywhere: panel headers, foldout sections, icon and toggle buttons, segmented controls,
//! search fields, keycaps, badges and empty states.

use bevy_egui::egui::{
    self, Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, vec2,
};

use crate::theme;

/// Height of panel headers and toolbars.
pub const BAR_H: f32 = 32.0;
/// Width of the label column in property grids.
pub const LABEL_W: f32 = 92.0;

/// Text in the secondary colour.
pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(theme::PARCHMENT_DIM)
}

/// Small text in the faintest colour, for hints.
pub fn hint(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).small().color(theme::FAINT)
}

/// A docked panel with no inner margin; its header and body bring their own.
pub fn panel_frame() -> Frame {
    Frame::new().fill(theme::PANEL)
}

/// The padded body below a panel header.
pub fn body_frame() -> Frame {
    Frame::new().inner_margin(Margin::symmetric(10, 8))
}

/// A panel's title bar: its name on the left, `actions` (usually icon buttons) on the right.
pub fn panel_header(ui: &mut Ui, title: &str, actions: impl FnOnce(&mut Ui)) {
    bar(ui, |ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new(title.to_uppercase())
                .font(theme::strong(11.0))
                .color(theme::PARCHMENT_DIM),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            actions(ui);
        });
    });
}

/// A full-width header-coloured strip with a bottom border, for headers and toolbars.
pub fn bar(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, BAR_H), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::HEADER);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::BORDER),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    contents(&mut child);
}

/// Tabs drawn as words with an underline under the open one, for panel headers.
pub fn header_tabs<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, tabs: &[(T, String)]) {
    for (tab, label) in tabs {
        let on = *value == *tab;
        let text = RichText::new(label.to_uppercase())
            .font(theme::strong(11.0))
            .color(if on { theme::PARCHMENT } else { theme::FAINT });
        let response = ui
            .add(egui::Button::new(text).frame(false))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if on {
            let r = response.rect;
            let y = ui.max_rect().bottom() - 1.0;
            ui.painter().line_segment(
                [
                    egui::pos2(r.left() + 2.0, y),
                    egui::pos2(r.right() - 2.0, y),
                ],
                Stroke::new(2.0, theme::BRASS),
            );
        }
        if response.clicked() {
            *value = *tab;
        }
        ui.add_space(4.0);
    }
}

/// A section title inside a panel: small, uppercase, with a hairline after it.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .font(theme::strong(10.5))
                .color(theme::BRASS),
        );
        let rect = ui.available_rect_before_wrap();
        let y = rect.center().y;
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 4.0, y),
                egui::pos2(rect.right(), y),
            ],
            Stroke::new(1.0, theme::BORDER),
        );
    });
    ui.add_space(2.0);
}

/// A section that folds away, like the components of an Inspector. Remembers whether it is open.
pub fn foldout<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    default_open: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let open = state.is_open();
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::RAISED);
    }
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, theme::BORDER),
    );
    painter.text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::CENTER_CENTER,
        if open { "⏷" } else { "⏵" },
        egui::FontId::proportional(10.0),
        theme::PARCHMENT_DIM,
    );
    painter.text(
        rect.left_center() + vec2(18.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        theme::strong(12.5),
        theme::PARCHMENT,
    );
    let header = response.clone();
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        state.toggle(ui);
    }
    state.store(ui.ctx());
    // The body is as wide as the header, never wider.
    let shown = state.show_body_indented(&header, ui, |ui| {
        ui.add_space(2.0);
        let r = contents(ui);
        ui.add_space(6.0);
        r
    });
    shown.map(|s| s.inner)
}

/// A frameless square button with a glyph, highlighted on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    icon_toggle(ui, false, icon, tip)
}

/// An icon button that shows when it is on.
pub fn icon_toggle(ui: &mut Ui, on: bool, icon: &str, tip: &str) -> Response {
    let size = vec2(24.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let fill = if on {
        theme::SELECTED
    } else if response.hovered() {
        theme::RAISED_HOVER
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    let color = if !ui.is_enabled() {
        theme::FAINT
    } else if on {
        theme::BRASS_BRIGHT
    } else if response.hovered() {
        theme::PARCHMENT
    } else {
        theme::PARCHMENT_DIM
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        color,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
}

/// The one button that does the main thing of a view: filled with brass.
pub fn primary_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    let color = if enabled {
        Color32::from_rgb(0x1A, 0x14, 0x0A)
    } else {
        theme::FAINT
    };
    let button = egui::Button::new(RichText::new(text).font(theme::strong(13.0)).color(color))
        .fill(if enabled { theme::BRASS } else { theme::RAISED })
        .stroke(Stroke::new(
            1.0,
            if enabled {
                theme::BRASS_BRIGHT
            } else {
                theme::BORDER
            },
        ));
    ui.add_enabled(enabled, button)
}

/// A row of joined buttons where exactly one is on. Each option is (value, label, tooltip).
/// True if the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str, &str)],
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(theme::BG_DEEP)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS + 1))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = vec2(9.0, 2.0);
            for (option, label, tip) in options {
                let on = *value == *option;
                let text = RichText::new(*label).color(if on {
                    theme::BRASS_BRIGHT
                } else {
                    theme::PARCHMENT_DIM
                });
                let mut response = ui.add(
                    egui::Button::new(text)
                        .selected(on)
                        .frame_when_inactive(on)
                        .min_size(vec2(0.0, 20.0)),
                );
                if !tip.is_empty() {
                    response = response.on_hover_text(*tip);
                }
                if response.clicked() && !on {
                    *value = *option;
                    changed = true;
                }
            }
        });
    changed
}

/// A keycap, e.g. `kbd(ui, "Ctrl+K")` draws Ctrl and K as separate caps.
pub fn kbd(ui: &mut Ui, keys: &str) {
    ui.spacing_mut().item_spacing.x = 3.0;
    for key in keys.split('+') {
        Frame::new()
            .fill(theme::RAISED)
            .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::symmetric(5, 0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(key)
                        .font(egui::FontId::monospace(11.0))
                        .color(theme::PARCHMENT),
                );
            });
    }
}

/// "Move  (W)": a tooltip naming what a control does and its shortcut.
pub fn tip(what: &str, keys: &str) -> String {
    format!("{what}  ·  {keys}")
}

/// A search field with a magnifier and a clear button. Escape clears it too.
pub fn search_field(ui: &mut Ui, text: &mut String, hint_text: &str, width: f32) -> Response {
    let response = Frame::new()
        .fill(ui.visuals().text_edit_bg_color())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(6, 0))
        .show(ui, |ui| {
            // Left to right even inside a right-to-left toolbar.
            ui.allocate_ui_with_layout(
                vec2(width, 24.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new("🔍").size(11.0).color(theme::FAINT));
                    let clear_w = if text.is_empty() { 0.0 } else { 18.0 };
                    let response = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text(hint(hint_text))
                            .frame(Frame::NONE)
                            .desired_width(ui.available_width() - clear_w),
                    );
                    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        text.clear();
                    }
                    if !text.is_empty()
                        && ui
                            .add(egui::Button::new(RichText::new("✕").size(10.0)).frame(false))
                            .on_hover_text("Clear")
                            .clicked()
                    {
                        text.clear();
                    }
                    response
                },
            )
            .inner
        });
    response.inner
}

/// Paints one line of text cut off with "…" at `max_width`. Returns where it went.
pub fn text_truncated(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
) -> egui::Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(8.0));
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// A small rounded label, e.g. a count or a kind.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(6, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(color));
        })
        .response
}

/// A coloured dot, e.g. for a connection's status.
pub fn status_dot(ui: &mut Ui, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 6.0, color.gamma_multiply(0.18));
    ui.painter().circle_filled(rect.center(), 3.5, color);
    response
}

/// A quiet placeholder for a view with nothing in it yet: a gear, a title, a line of help.
pub fn empty_state(ui: &mut Ui, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        theme::gear(
            ui.painter(),
            rect.center(),
            28.0,
            12,
            0.2,
            theme::RAISED,
            ui.visuals().panel_fill,
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new(title)
                .font(theme::display(17.0))
                .color(theme::PARCHMENT),
        );
        ui.add_space(2.0);
        ui.label(dim(body));
    });
}

/// The label column of a property grid: fixed width, dim, right-aligned like Unity's Inspector.
pub fn prop_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(
        vec2(LABEL_W, ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(dim(text));
        },
    );
}

/// A property row: label, then the control filling the rest of the width.
pub fn prop<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        // Long values are cut off rather than widening the panel.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        prop_label(ui, label);
        ui.add_space(4.0);
        control(ui)
    })
    .inner
}

/// A number dragged with the mouse, tagged with its axis colour (X red, Y green, Z blue).
pub fn axis_drag(
    ui: &mut Ui,
    axis: &str,
    color: Color32,
    value: &mut f32,
    speed: f32,
    suffix: &str,
    width: f32,
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(ui.visuals().text_edit_bg_color())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                vec2(width - 2.0, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let (tag, _) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::hover());
                    ui.painter().rect_filled(
                        tag,
                        CornerRadius {
                            nw: theme::RADIUS,
                            sw: theme::RADIUS,
                            ne: 0,
                            se: 0,
                        },
                        color.gamma_multiply(0.85),
                    );
                    ui.painter().text(
                        tag.center(),
                        egui::Align2::CENTER_CENTER,
                        axis,
                        theme::strong(10.5),
                        Color32::from_rgb(0x14, 0x12, 0x10),
                    );
                    ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
                    changed = ui
                        .add_sized(
                            vec2(ui.available_width(), 22.0),
                            egui::DragValue::new(value)
                                .speed(speed)
                                .suffix(suffix)
                                .max_decimals(2),
                        )
                        .changed();
                },
            );
        });
    changed
}

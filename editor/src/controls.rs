//! Gameplay rebinding. Physical input is sampled before editor shortcuts and kept separate
//! from egui keyboard events so international keyboard layouts and controller triggers work.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use clockwork_core::input::{BUTTONS, ControlSettings, ControllerStyle, InputMap};
use clockwork_runtime::input::{controller_style, key, key_label, pad_button, pad_label};

use crate::{kit, settings, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    Keyboard,
    Controller,
}

#[derive(Clone, Debug)]
struct Capture {
    action: String,
    kind: BindingKind,
    started_frame: u64,
}

#[derive(Clone, Debug)]
struct Proposal {
    action: String,
    kind: BindingKind,
    token: String,
}

#[derive(Default)]
struct FrameInput {
    keys: Vec<String>,
    pad: Vec<String>,
    any_held: bool,
    escape: bool,
    mouse: bool,
}

struct ConnectedController {
    id: Entity,
    name: String,
    style: ControllerStyle,
}

/// UI state and the input barrier shared with the host. Settings own the persistent bindings.
#[derive(Resource, Default)]
pub struct ControlsUi {
    frame: u64,
    capture: Option<Capture>,
    proposal: Option<Proposal>,
    /// Keep a captured/cancelled press out of gameplay until the user releases it.
    release_guard: bool,
    devices: Vec<ConnectedController>,
    active_controller: Option<Entity>,
    status: Option<String>,
}

impl ControlsUi {
    /// Suppress editor shortcuts and gameplay while listening or resolving a shared binding.
    pub fn blocked(&self) -> bool {
        self.capture.is_some() || self.proposal.is_some() || self.release_guard
    }

    /// Called when leaving Settings or changing category, as well as by the Cancel button.
    pub fn cancel(&mut self) {
        let capture = self.capture.take().is_some();
        let proposal = self.proposal.take().is_some();
        if capture || proposal {
            self.release_guard = true;
            self.status = Some("Binding unchanged.".into());
        }
    }

    fn begin(&mut self, action: &str, kind: BindingKind) {
        self.capture = Some(Capture {
            action: action.into(),
            kind,
            started_frame: self.frame,
        });
        self.proposal = None;
        self.status = None;
    }

    fn sample(&mut self, mut input: FrameInput) {
        self.frame += 1;
        if !input.any_held {
            self.release_guard = false;
        }
        if input.escape || (input.mouse && self.capture.is_some()) {
            self.cancel();
            return;
        }
        let Some(capture) = &self.capture else {
            return;
        };
        if self.frame <= capture.started_frame {
            return;
        }
        let tokens = match capture.kind {
            BindingKind::Keyboard => &mut input.keys,
            BindingKind::Controller => &mut input.pad,
        };
        // Stable choice if more than one physical input lands in the same frame.
        tokens.sort();
        if let Some(token) = tokens.first() {
            self.proposal = Some(Proposal {
                action: capture.action.clone(),
                kind: capture.kind,
                token: token.clone(),
            });
            self.capture = None;
            self.release_guard = true;
        }
    }

    fn display_style(&self, selected: ControllerStyle) -> ControllerStyle {
        if selected == ControllerStyle::Auto {
            self.devices
                .iter()
                .find(|d| Some(d.id) == self.active_controller)
                .or_else(|| self.devices.first())
                .map(|d| d.style)
                .unwrap_or(ControllerStyle::Generic)
        } else {
            selected
        }
    }
}

/// Schedule after Bevy's input collection and before editor/game input routing.
pub fn sample_system(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    mut controls: ResMut<ControlsUi>,
) {
    let mut input = FrameInput {
        keys: keys
            .get_just_pressed()
            .map(|k| format!("{k:?}"))
            .filter(|name| key(name).is_some())
            .collect(),
        any_held: keys.get_pressed().next().is_some() || mouse.get_pressed().next().is_some(),
        escape: keys.just_pressed(KeyCode::Escape),
        mouse: mouse.get_just_pressed().next().is_some(),
        ..default()
    };
    controls.devices.clear();
    for (entity, pad, name) in &pads {
        if pad.get_just_pressed().next().is_some() {
            controls.active_controller = Some(entity);
        }
        let name = name.map(Name::as_str).unwrap_or("Game controller");
        controls.devices.push(ConnectedController {
            id: entity,
            name: name.to_string(),
            style: controller_style(name, pad.vendor_id()),
        });
        input.any_held |= pad.get_pressed().next().is_some();
        input.pad.extend(
            pad.get_just_pressed()
                .map(|button| format!("{button:?}"))
                .filter(|name| pad_button(name).is_some()),
        );
    }
    controls.devices.sort_by_key(|d| d.id);
    controls.sample(input);
}

fn style_label(style: ControllerStyle) -> &'static str {
    match style {
        ControllerStyle::Auto => "Automatic",
        ControllerStyle::Xbox => "Xbox",
        ControllerStyle::PlayStation => "PS5 / PlayStation",
        ControllerStyle::Generic => "Generic",
    }
}

fn tokens<'a>(map: &'a InputMap, action: &str, kind: BindingKind) -> &'a [String] {
    map.binding(action)
        .map(|binding| match kind {
            BindingKind::Keyboard => binding.keys.as_slice(),
            BindingKind::Controller => binding.pad.as_slice(),
        })
        .unwrap_or_default()
}

fn assign(settings: &mut ControlSettings, action: &str, kind: BindingKind, values: Vec<String>) {
    match kind {
        BindingKind::Keyboard => settings.set_keys(action, values),
        BindingKind::Controller => settings.set_pad(action, values),
    }
}

fn conflicts(map: &InputMap, proposal: &Proposal) -> Vec<String> {
    map.bindings
        .iter()
        .filter(|binding| binding.button != proposal.action)
        .filter(|binding| tokens(map, &binding.button, proposal.kind).contains(&proposal.token))
        .map(|binding| binding.button.clone())
        .collect()
}

fn apply_proposal(
    settings: &mut ControlSettings,
    base: &InputMap,
    proposal: &Proposal,
    replace: bool,
) {
    if replace {
        let effective = settings.effective_map(base);
        for action in conflicts(&effective, proposal) {
            let remaining = tokens(&effective, &action, proposal.kind)
                .iter()
                .filter(|token| **token != proposal.token)
                .cloned()
                .collect();
            assign(settings, &action, proposal.kind, remaining);
        }
    }
    assign(
        settings,
        &proposal.action,
        proposal.kind,
        vec![proposal.token.clone()],
    );
}

fn token_label(token: &str, kind: BindingKind, style: ControllerStyle) -> String {
    match kind {
        BindingKind::Keyboard => key_label(token),
        BindingKind::Controller => pad_label(token, style),
    }
}

/// Matches Settings' compact rows and theme; no separate preferences window or Apply step.
pub fn panel(
    ui: &mut egui::Ui,
    settings: &mut ControlSettings,
    controls: &mut ControlsUi,
    base: &InputMap,
) {
    settings::section(ui, "Controllers");
    if controls.devices.is_empty() {
        ui.label(kit::dim("No controller connected. Connect an Xbox, PlayStation or generic controller by USB or Bluetooth."));
    } else {
        for device in &controls.devices {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("●").color(theme::SUCCESS));
                ui.label(&device.name);
                ui.label(kit::dim(style_label(device.style)));
            });
        }
    }
    ui.add_space(10.0);
    settings::row(
        ui,
        "Button labels",
        "Automatic follows your connected controller. Change this if its buttons have different names.",
        |ui| {
            egui::ComboBox::from_id_salt("controller_style")
                .selected_text(style_label(settings.controller_style))
                .width(180.0)
                .show_ui(ui, |ui| {
                    for style in [
                        ControllerStyle::Auto,
                        ControllerStyle::Xbox,
                        ControllerStyle::PlayStation,
                        ControllerStyle::Generic,
                    ] {
                        ui.selectable_value(
                            &mut settings.controller_style,
                            style,
                            style_label(style),
                        );
                    }
                });
        },
    );
    settings::row(
        ui,
        "Stick deadzone",
        "Increase this if a released stick moves your character. The left stick always controls movement.",
        |ui| {
            ui.add(
                egui::Slider::new(&mut settings.stick_deadzone, 0.05..=0.5)
                    .step_by(0.01)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        },
    );

    settings::section(ui, "Gameplay bindings");
    ui.label(kit::dim("Click a binding, then press a key or controller button. Escape or a mouse click cancels. Editor shortcuts stay in Keyboard."));
    ui.add_space(8.0);
    let style = controls.display_style(settings.controller_style);
    // A captured Space or Enter can also be egui's activation key for the focused cell.
    // Disable that grid until the press is released so it cannot restart capture.
    let captured_press = controls.release_guard;

    if let Some(proposal) = controls.proposal.clone() {
        let shared = conflicts(&settings.effective_map(base), &proposal);
        if shared.is_empty() {
            apply_proposal(settings, base, &proposal, false);
            controls.status = Some(format!(
                "{} bound to {}.",
                proposal.action,
                token_label(&proposal.token, proposal.kind, style)
            ));
            controls.proposal = None;
        } else {
            egui::Frame::new().fill(theme::PANEL).inner_margin(10).show(ui, |ui| {
                ui.label(RichText::new(format!("{} is also used by {}.", token_label(&proposal.token, proposal.kind, style), shared.join(", "))).color(theme::WARN));
                ui.label(kit::dim(format!("Choose how to bind {}. Shared buttons can be useful in different game menus.", proposal.action)));
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Keep shared").clicked() {
                        apply_proposal(settings, base, &proposal, false);
                        controls.proposal = None;
                        controls.status = Some(format!("{} now shares this binding.", proposal.action));
                    }
                    if ui.button("Replace other bindings").on_hover_text("Remove this key or button from the listed actions; their other bindings are kept.").clicked() {
                        apply_proposal(settings, base, &proposal, true);
                        controls.proposal = None;
                        controls.status = Some(format!("Binding moved to {}.", proposal.action));
                    }
                    if ui.button("Cancel").clicked() {
                        controls.cancel();
                    }
                });
            });
            ui.add_space(8.0);
        }
    }

    if let Some(capture) = controls.capture.clone() {
        let device = match capture.kind {
            BindingKind::Keyboard => "key",
            BindingKind::Controller => "controller button or trigger",
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{}: press a {device}…", capture.action))
                    .color(theme::BRASS_BRIGHT),
            );
            if ui.button("Cancel").clicked() {
                controls.cancel();
            }
        });
        ui.add_space(8.0);
    } else if let Some(status) = &controls.status {
        ui.label(kit::dim(status));
        ui.add_space(8.0);
    }

    let effective = settings.effective_map(base);
    ui.add_enabled_ui(controls.proposal.is_none() && !captured_press, |ui| {
        egui::Grid::new("gameplay_binding_grid")
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label(kit::dim("Action"));
                ui.label(kit::dim("Keyboard"));
                ui.label(kit::dim("Controller"));
                ui.label("");
                ui.end_row();
                for action in BUTTONS {
                    ui.add_sized(
                        [64.0, 28.0],
                        egui::Label::new(RichText::new(action).font(theme::strong(13.0))),
                    );
                    binding_cell(
                        ui,
                        settings,
                        controls,
                        &effective,
                        action,
                        BindingKind::Keyboard,
                        style,
                    );
                    binding_cell(
                        ui,
                        settings,
                        controls,
                        &effective,
                        action,
                        BindingKind::Controller,
                        style,
                    );
                    if ui
                        .button("Reset")
                        .on_hover_text(
                            "Restore this action's game defaults for both keyboard and controller.",
                        )
                        .clicked()
                    {
                        controls.cancel();
                        settings.reset(action);
                        controls.status = Some(format!("{action} restored to its game defaults."));
                    }
                    ui.end_row();
                }
            });
    });
    ui.add_space(12.0);
    if ui
        .button("Restore all gameplay bindings")
        .on_hover_text("Use the current game's defaults. Controller labels and deadzone are kept.")
        .clicked()
    {
        controls.cancel();
        settings.overrides.clear();
        controls.status = Some("All gameplay bindings restored to their game defaults.".into());
    }
}

fn binding_cell(
    ui: &mut egui::Ui,
    settings: &mut ControlSettings,
    controls: &mut ControlsUi,
    map: &InputMap,
    action: &str,
    kind: BindingKind,
    style: ControllerStyle,
) {
    let bound = tokens(map, action, kind);
    let listening = controls
        .capture
        .as_ref()
        .is_some_and(|c| c.action == action && c.kind == kind);
    let label = if listening {
        "Listening…".into()
    } else if bound.is_empty() {
        "Unbound".into()
    } else {
        bound
            .iter()
            .map(|token| token_label(token, kind, style))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    ui.horizontal(|ui| {
        let mut button = egui::Button::new(RichText::new(label).color(if listening {
            theme::BRASS_BRIGHT
        } else {
            theme::PARCHMENT
        }));
        if listening {
            button = button.fill(theme::SELECTED);
        }
        if ui
            .add_sized([168.0, 28.0], button)
            .on_hover_text("Click, then press one input to replace this binding.")
            .clicked()
        {
            controls.begin(action, kind);
        }
        if ui
            .add_enabled(!bound.is_empty(), egui::Button::new("×"))
            .on_hover_text("Clear this binding.")
            .clicked()
        {
            controls.cancel();
            assign(settings, action, kind, vec![]);
            controls.status = Some(format!("{action} binding cleared."));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::input::default_map;

    #[test]
    fn automatic_labels_follow_the_controller_used_for_capture() {
        let mut world = World::new();
        let xbox = world.spawn_empty().id();
        let ps5 = world.spawn_empty().id();
        let mut controls = ControlsUi {
            devices: vec![
                ConnectedController {
                    id: xbox,
                    name: "Xbox".into(),
                    style: ControllerStyle::Xbox,
                },
                ConnectedController {
                    id: ps5,
                    name: "PS5".into(),
                    style: ControllerStyle::PlayStation,
                },
            ],
            active_controller: Some(ps5),
            ..default()
        };
        assert_eq!(
            controls.display_style(ControllerStyle::Auto),
            ControllerStyle::PlayStation
        );
        assert_eq!(
            controls.display_style(ControllerStyle::Generic),
            ControllerStyle::Generic
        );
        controls.devices.retain(|device| device.id != ps5);
        assert_eq!(
            controls.display_style(ControllerStyle::Auto),
            ControllerStyle::Xbox
        );
    }

    #[test]
    fn capture_ignores_the_opening_frame_and_keys_already_held() {
        let mut controls = ControlsUi::default();
        controls.sample(FrameInput {
            keys: vec!["KeyK".into()],
            any_held: true,
            ..default()
        });
        controls.begin("Jump", BindingKind::Keyboard);
        assert!(controls.proposal.is_none());
        controls.sample(FrameInput {
            any_held: true,
            ..default()
        });
        assert!(controls.proposal.is_none());
        controls.sample(FrameInput {
            keys: vec!["KeyZ".into()],
            any_held: true,
            ..default()
        });
        assert_eq!(controls.proposal.as_ref().unwrap().token, "KeyZ");
        controls.proposal = None;
        assert!(controls.blocked());
        controls.sample(FrameInput::default());
        assert!(!controls.blocked());
    }

    #[test]
    fn escape_and_mouse_cancel_without_accepting_an_input() {
        for input in [
            FrameInput {
                keys: vec!["Escape".into()],
                escape: true,
                any_held: true,
                ..default()
            },
            FrameInput {
                mouse: true,
                any_held: true,
                ..default()
            },
        ] {
            let mut controls = ControlsUi::default();
            controls.begin("Jump", BindingKind::Keyboard);
            controls.sample(input);
            assert!(controls.capture.is_none());
            assert!(controls.proposal.is_none());
            assert!(controls.blocked());
            controls.sample(FrameInput::default());
            assert!(!controls.blocked());
        }
    }

    #[test]
    fn controller_capture_ignores_keys_and_accepts_triggers() {
        let mut controls = ControlsUi::default();
        controls.begin("A", BindingKind::Controller);
        controls.sample(FrameInput {
            keys: vec!["KeyP".into()],
            any_held: true,
            ..default()
        });
        assert!(controls.capture.is_some());
        controls.sample(FrameInput {
            pad: vec!["RightTrigger2".into()],
            any_held: true,
            ..default()
        });
        assert_eq!(controls.proposal.as_ref().unwrap().token, "RightTrigger2");
    }

    #[test]
    fn keeping_a_shared_controller_binding_preserves_existing_actions() {
        let base = default_map();
        let mut settings = ControlSettings::default();
        let proposal = Proposal {
            action: "A".into(),
            kind: BindingKind::Controller,
            token: "East".into(),
        };
        assert_eq!(conflicts(&base, &proposal), ["Jump", "Back"]);
        apply_proposal(&mut settings, &base, &proposal, false);
        let map = settings.effective_map(&base);
        for action in ["A", "Jump", "Back"] {
            assert_eq!(tokens(&map, action, BindingKind::Controller), ["East"]);
        }
    }

    #[test]
    fn replacing_a_conflict_preserves_other_keys_and_other_device_bindings() {
        let base = default_map();
        let mut settings = ControlSettings::default();
        let proposal = Proposal {
            action: "Jump".into(),
            kind: BindingKind::Keyboard,
            token: "KeyW".into(),
        };
        apply_proposal(&mut settings, &base, &proposal, true);
        let map = settings.effective_map(&base);
        assert_eq!(tokens(&map, "Up", BindingKind::Keyboard), ["ArrowUp"]);
        assert_eq!(tokens(&map, "Up", BindingKind::Controller), ["DPadUp"]);
        assert_eq!(tokens(&map, "Jump", BindingKind::Keyboard), ["KeyW"]);
        assert_eq!(tokens(&map, "Jump", BindingKind::Controller), ["East"]);
    }
}

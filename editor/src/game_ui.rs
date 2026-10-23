//! Connects Play mode to the game's own HUD and menus (`clockwork_runtime::hud`): hands them
//! what to show and the played character's input map, takes what they asked for, and keeps the
//! game's buttons away while a text field is typed in.

use bevy::prelude::*;
use clockwork_runtime::{
    hud::{DialogView, HudRequest, HudRequests, MenuState, ShownHud},
    input::{Bindings, ControllerOptions, GameInput},
};

use crate::{
    controls::ControlsUi,
    settings::Settings,
    state::{EditorState, Screen},
};

/// The game's defaults remain the source of controls that the player has not overridden.
pub fn base_input_map(state: &EditorState) -> clockwork_core::input::InputMap {
    state
        .workshop
        .screen
        .as_ref()
        .filter(|_| state.workshop.playing.is_some())
        .map(|(_, map)| map.clone())
        .unwrap_or_else(clockwork_core::input::default_map)
}

/// Unmodified editor shortcuts yield to assigned gameplay keys during Play.
pub fn gameplay_uses_key(state: &EditorState, settings: &Settings, key: &str) -> bool {
    state.workshop.playing.is_some()
        && settings
            .controls
            .effective_map(&base_input_map(state))
            .bindings
            .iter()
            .any(|binding| binding.keys.iter().any(|bound| bound == key))
}

/// Prepare controls before the runtime reads them, so neither HUD nor gameplay consumes
/// keys while rebinding, in Settings, typing, or while this window is unfocused.
pub fn prepare_input_system(
    state: Res<EditorState>,
    settings: Res<Settings>,
    mut controls: ResMut<ControlsUi>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut bindings: ResMut<Bindings>,
    mut options: ResMut<ControllerOptions>,
    mut input: ResMut<GameInput>,
) {
    if state.screen != Screen::Settings {
        controls.cancel();
    }
    input.blocked = controls.blocked()
        || state.workshop.typing
        || (state.player.is_none() && state.screen == Screen::Settings)
        || windows.iter().any(|window| !window.focused);
    let map = settings.controls.effective_map(&base_input_map(&state));
    if bindings.map != map {
        *bindings = Bindings::new(&map);
    }
    options.style = settings.controls.controller_style;
    options.deadzone = settings.controls.stick_deadzone;
}

pub fn hud_bridge_system(
    mut state: ResMut<EditorState>,
    mut shown: ResMut<ShownHud>,
    mut requests: ResMut<HudRequests>,
    mut menus: ResMut<MenuState>,
    mut input: ResMut<GameInput>,
    mut exit: MessageWriter<AppExit>,
) {
    for name in std::mem::take(&mut state.workshop.presses) {
        if let Some(button) = clockwork_runtime::input::GameButton::named(&name) {
            input.inject(button);
        }
    }
    let playing = state.workshop.playing.is_some();
    let screen = state.workshop.screen.as_ref().filter(|_| playing);
    let now =
        ShownHud {
            hud: state.workshop.hud.clone().filter(|_| playing),
            health: state.workshop.health.filter(|_| playing),
            toast: state.workshop.toast.as_ref().map(|(text, _)| text.clone()),
            screen: screen.map(|(screen, _)| screen.clone()),
            lives: state.workshop.lives.filter(|_| playing),
            seconds: state.workshop.seconds.filter(|_| playing),
            fade: if playing { state.workshop.fade } else { 0 },
            banner: state.workshop.banner.clone().filter(|_| playing),
            dialog: state.workshop.dialog.as_ref().filter(|_| playing).and_then(
                |(dialog, page)| {
                    Some(DialogView {
                        speaker: dialog.speaker.clone(),
                        text: dialog.pages.get(*page)?.clone(),
                        more: page + 1 < dialog.pages.len(),
                    })
                },
            ),
        };
    // Only a real change marks it changed.
    shown.set_if_neq(now);
    if let Some(kind) = state.workshop.menu_toggle.take() {
        menus.toggle = Some(kind);
    }
    // A menu opening plays the character's menu sound.
    if menus.paused() && !state.workshop.menu_paused {
        let menu = state.workshop.kit_sounds.menu.clone();
        state.workshop.sounds.extend(menu);
    }
    state.workshop.menu_paused = menus.paused();
    // Stopping an exported game closes it.
    if state.player.is_some() && requests.0.contains(&HudRequest::Stop) {
        exit.write(AppExit::Success);
    }
    state.workshop.hud_requests.append(&mut requests.0);
}

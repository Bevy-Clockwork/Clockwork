//! Screens report intent; Play owns the view, its focus and the transitions between screens.

use bevy::prelude::*;
use clockwork_core::flow::{ScreenAction, ScreenView};

use crate::input::{GameButton, GameInput};

/// The screen over the world, using the theme and pictures in [`super::ShownHud::screen`].
/// `None` returns to the in-game HUD. Fill this before [`super::HudSystems`], and read
/// [`ScreenRequest`] after it. A screen takes input away from HUD menus and dialogs.
/// Play must also suspend world input and simulation while a screen is shown.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ShownScreen {
    pub view: Option<ScreenView>,
    /// A rendered model logo, supplied by the host, with its pixel dimensions. An image
    /// named by the title in `Screen::images` takes precedence.
    pub model_logo_image: Option<(Handle<Image>, Vec2)>,
    /// The flow's counted item id; its picture comes from `Hud::icons` when available.
    pub counted_item: Option<String>,
}

/// Both selectors are vertical lists. Play decides whether moves stop or wrap at the ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenMove {
    Up,
    Down,
}

/// Screen input for Play. The wrapper keeps Bevy out of the shared flow vocabulary.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenRequest {
    Action(ScreenAction),
    Move(ScreenMove),
}

fn request(view: &ScreenView, pressed: impl Fn(GameButton) -> bool) -> Option<ScreenRequest> {
    use ScreenAction::*;
    let confirm = pressed(GameButton::A);
    let action = match view {
        ScreenView::Title { .. } => (confirm || pressed(GameButton::Menu)).then_some(Start),
        ScreenView::GameOver { .. } => (confirm || pressed(GameButton::Menu)).then_some(Continue),
        ScreenView::Files { slots, focus, .. } => {
            if pressed(GameButton::B) || pressed(GameButton::Back) {
                Some(Back)
            } else {
                (confirm && *focus < slots.len()).then_some(File(*focus))
            }
        }
        ScreenView::Acts { acts, focus, .. } => {
            if pressed(GameButton::B) || pressed(GameButton::Back) {
                Some(Back)
            } else {
                (confirm && *focus < acts.len()).then_some(Act(*focus))
            }
        }
    };
    if let Some(action) = action {
        return Some(ScreenRequest::Action(action));
    }
    let selectable = match view {
        ScreenView::Files { slots, .. } => !slots.is_empty(),
        ScreenView::Acts { acts, .. } => !acts.is_empty(),
        _ => false,
    };
    if selectable {
        // One request per frame: confirming chooses the focus the player actually saw.
        if pressed(GameButton::Up) {
            return Some(ScreenRequest::Move(ScreenMove::Up));
        }
        if pressed(GameButton::Down) {
            return Some(ScreenRequest::Move(ScreenMove::Down));
        }
    }
    None
}

pub(super) fn input_system(
    shown: Res<ShownScreen>,
    input: Res<GameInput>,
    mut requests: MessageWriter<ScreenRequest>,
) {
    if let Some(view) = &shown.view
        && let Some(request) = request(view, |button| input.pressed(button))
    {
        requests.write(request);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hud::{DialogView, HudRequests, MenuState, Screen, ShownHud, menu},
        input::{Bindings, input_system as read_input},
    };
    use clockwork_core::flow::{ActView, FileView};

    fn title() -> ScreenView {
        ScreenView::Title {
            logo: None,
            logo_camera: None,
            prompt: "START".into(),
            background: clockwork_core::ui::Rgba(0, 0, 0, 255),
        }
    }

    fn files() -> ScreenView {
        ScreenView::Files {
            title: "Files".into(),
            slots: vec![
                FileView {
                    name: "A".into(),
                    empty: true,
                    count: None
                };
                2
            ],
            focus: 1,
        }
    }

    fn acts() -> ScreenView {
        ScreenView::Acts {
            course: "Course".into(),
            acts: vec![
                ActView {
                    name: "Act".into(),
                    done: false
                };
                2
            ],
            focus: 1,
        }
    }

    fn press(view: &ScreenView, buttons: &[GameButton]) -> Option<ScreenRequest> {
        request(view, |button| buttons.contains(&button))
    }

    #[test]
    fn title_and_game_over_accept_a_or_menu() {
        for button in [GameButton::A, GameButton::Menu] {
            assert_eq!(
                press(&title(), &[button]),
                Some(ScreenRequest::Action(ScreenAction::Start))
            );
            assert_eq!(
                press(
                    &ScreenView::GameOver {
                        text: "GAME OVER".into()
                    },
                    &[button]
                ),
                Some(ScreenRequest::Action(ScreenAction::Continue))
            );
        }
        assert_eq!(press(&title(), &[GameButton::B, GameButton::Down]), None);
    }

    #[test]
    fn selectors_confirm_the_owners_focus_and_report_moves() {
        for (view, action) in [
            (files(), ScreenAction::File(1)),
            (acts(), ScreenAction::Act(1)),
        ] {
            assert_eq!(
                press(&view, &[GameButton::A]),
                Some(ScreenRequest::Action(action))
            );
            for button in [GameButton::B, GameButton::Back] {
                assert_eq!(
                    press(&view, &[button]),
                    Some(ScreenRequest::Action(ScreenAction::Back))
                );
            }
            for (button, step) in [
                (GameButton::Up, ScreenMove::Up),
                (GameButton::Down, ScreenMove::Down),
            ] {
                assert_eq!(press(&view, &[button]), Some(ScreenRequest::Move(step)));
            }
            assert_eq!(
                press(
                    &view,
                    &[GameButton::Left, GameButton::Right, GameButton::Menu]
                ),
                None
            );
            assert_eq!(
                press(&view, &[GameButton::A, GameButton::Down]),
                Some(ScreenRequest::Action(action))
            );
            assert_eq!(
                press(&view, &[GameButton::B, GameButton::A]),
                Some(ScreenRequest::Action(ScreenAction::Back))
            );
        }
    }

    #[test]
    fn empty_lists_and_invalid_focus_cannot_be_confirmed() {
        for view in [
            ScreenView::Files {
                title: String::new(),
                slots: Vec::new(),
                focus: 0,
            },
            ScreenView::Acts {
                course: String::new(),
                acts: Vec::new(),
                focus: 0,
            },
        ] {
            assert_eq!(press(&view, &[GameButton::A, GameButton::Down]), None);
            assert_eq!(
                press(&view, &[GameButton::B]),
                Some(ScreenRequest::Action(ScreenAction::Back))
            );
        }
        let mut view = files();
        if let ScreenView::Files { focus, .. } = &mut view {
            *focus = 99;
        }
        assert_eq!(press(&view, &[GameButton::A]), None);
        assert_eq!(
            press(&view, &[GameButton::Up]),
            Some(ScreenRequest::Move(ScreenMove::Up))
        );
    }

    fn app(view: Option<ScreenView>) -> App {
        let mut app = App::new();
        app.init_resource::<GameInput>()
            .init_resource::<Bindings>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(ShownScreen { view, ..default() })
            .add_message::<ScreenRequest>()
            .add_systems(PreUpdate, read_input)
            .add_systems(Update, input_system);
        app
    }

    #[test]
    fn bound_buttons_emit_messages_once_without_mutating_focus() {
        let mut app = app(Some(files()));
        let original = app.world().resource::<ShownScreen>().clone();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowDown);
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<ScreenRequest>>()
                .drain()
                .collect::<Vec<_>>(),
            [ScreenRequest::Move(ScreenMove::Down)]
        );
        assert_eq!(app.world().resource::<ShownScreen>(), &original);
        app.update();
        assert_eq!(app.world().resource::<Messages<ScreenRequest>>().len(), 0);
        app.world_mut()
            .resource_mut::<GameInput>()
            .inject(GameButton::A);
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<ScreenRequest>>()
                .drain()
                .collect::<Vec<_>>(),
            [ScreenRequest::Action(ScreenAction::File(1))]
        );
    }

    #[test]
    fn inactive_or_blocked_screens_do_not_emit() {
        let mut app = app(None);
        app.world_mut()
            .resource_mut::<GameInput>()
            .inject(GameButton::A);
        app.update();
        assert_eq!(app.world().resource::<Messages<ScreenRequest>>().len(), 0);
        app.world_mut().resource_mut::<ShownScreen>().view = Some(title());
        app.world_mut().resource_mut::<GameInput>().blocked = true;
        app.world_mut()
            .resource_mut::<GameInput>()
            .inject(GameButton::Menu);
        app.update();
        assert_eq!(app.world().resource::<Messages<ScreenRequest>>().len(), 0);
    }

    #[test]
    fn screen_input_takes_priority_over_menus_and_dialogs() {
        let mut app = app(Some(title()));
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<HudRequests>()
            .insert_resource(MenuState {
                open: Some(0),
                toggle: Some("Pause".into()),
                ..default()
            })
            .insert_resource(ShownHud {
                screen: Some(Screen::default()),
                dialog: Some(DialogView {
                    speaker: None,
                    text: "Hello".into(),
                    more: false,
                }),
                ..default()
            })
            .add_systems(Update, menu::menu_system.after(input_system));
        app.world_mut()
            .resource_mut::<GameInput>()
            .inject(GameButton::A);
        app.update();
        assert!(app.world().resource::<HudRequests>().0.is_empty());
        assert_eq!(app.world().resource::<MenuState>().open, None);
        assert_eq!(app.world().resource::<MenuState>().toggle, None);
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<ScreenRequest>>()
                .drain()
                .collect::<Vec<_>>(),
            [ScreenRequest::Action(ScreenAction::Start)]
        );
    }
}

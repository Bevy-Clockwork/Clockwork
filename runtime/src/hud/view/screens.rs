//! Full-screen flow views share the HUD's text, frame and image cache.

use clockwork_core::{flow::ScreenView, ui::Rgba};

use super::*;

#[derive(Component)]
pub(crate) struct ScreenPrompt;

#[derive(Component)]
struct ScreenEntry;

#[derive(Component)]
struct ScreenContent;

#[derive(Component)]
struct ScreenOverlay;

/// Real time keeps the title blinking even while the game's clock is paused.
pub(crate) fn blink_system(
    time: Res<Time<Real>>,
    mut prompts: Query<&mut Visibility, With<ScreenPrompt>>,
) {
    let visible = (time.elapsed_secs_f64() % 1.0) < 0.6;
    for mut visibility in &mut prompts {
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

struct Entry<'a> {
    name: &'a str,
    detail: Option<String>,
    done: bool,
    icon: bool,
}

impl Painter<'_> {
    pub(super) fn flow_screen(
        &self,
        root: &mut ChildSpawnerCommands,
        view: &ScreenView,
        shown: &ShownScreen,
    ) {
        let (heading, entries, focus, background) = match view {
            ScreenView::Title {
                prompt, background, ..
            } => (prompt.as_str(), Vec::new(), None, *background),
            ScreenView::Files {
                title,
                slots,
                focus,
            } => (
                title.as_str(),
                slots
                    .iter()
                    .map(|slot| Entry {
                        name: &slot.name,
                        detail: if slot.empty {
                            Some(self.screen.layout.words.new_file.clone())
                        } else {
                            slot.count.map(|count| count.to_string())
                        },
                        done: false,
                        icon: !slot.empty && slot.count.is_some(),
                    })
                    .collect(),
                Some(*focus),
                self.theme.shade,
            ),
            ScreenView::Acts {
                course,
                acts,
                focus,
            } => (
                course.as_str(),
                acts.iter()
                    .map(|act| Entry {
                        name: &act.name,
                        detail: None,
                        done: act.done,
                        icon: false,
                    })
                    .collect(),
                Some(*focus),
                self.theme.shade,
            ),
            ScreenView::GameOver { text } => (text.as_str(), Vec::new(), None, self.theme.shade),
        };
        let logo = match view {
            ScreenView::Title { logo, .. } => logo
                .as_deref()
                .and_then(|id| self.pictures.get(&self.screen.images, id))
                .or(shown.model_logo_image.as_ref()),
            _ => None,
        };
        // A logo the game's own camera drew fills the screen behind the words, which sit
        // low; any other sits above them.
        let (backdrop, logo) = match view {
            ScreenView::Title {
                logo_camera: Some(_),
                ..
            } => (logo, None),
            _ => (None, logo),
        };
        let icon = shown
            .counted_item
            .as_deref()
            .and_then(|id| self.pictures.get(&self.hud?.icons, id));
        let font = if matches!(view, ScreenView::GameOver { .. }) {
            self.dialog_font.or(self.font)
        } else {
            self.font
        };
        let size = if matches!(view, ScreenView::GameOver { .. }) {
            self.theme.dialog_font_size.unwrap_or(self.font())
        } else {
            self.font()
        };
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            8.0
        };
        let width = (self.viewport.x - 20.0).clamp(120.0, 300.0);
        let border = self
            .theme
            .frame
            .as_ref()
            .map_or(BORDER, |f| f.border as f32);
        let inset = if focus.is_some() { border + 6.0 } else { 0.0 };
        // A title's logo takes much of the screen, as a game's does; the column widens for it.
        let logo_size = logo.map(|(_, dimensions)| {
            let bounds = Vec2::new(self.viewport.x - 20.0, self.viewport.y * 0.55);
            fit_image(*dimensions, bounds.max(Vec2::splat(80.0)))
        });
        let width = width
            .max(inset * 2.0 + 80.0)
            .max(logo_size.map_or(0.0, |l| l.x));
        let text_width = width - inset * 2.0;
        let row_width = text_width - 10.0;
        let height_of = |text: &str, width| text_height(font.map(|f| f.0), text, size, width);
        let mut height = inset * 2.0 + height_of(heading, text_width);
        if let Some(logo_size) = logo_size {
            height += logo_size.y + GAP;
        }
        for entry in &entries {
            height += GAP + 10.0 + height_of(entry.name, row_width);
            if let Some(detail) = &entry.detail {
                let has_icon = entry.icon && icon.is_some();
                let detail_width = row_width - if has_icon { ICON + GAP } else { 0.0 };
                height +=
                    GAP + height_of(detail, detail_width).max(if has_icon { ICON } else { 0.0 });
            }
        }
        // Wrap at game resolution, then shrink the whole panel only when it cannot fit.
        // Borders, glyphs and icons shrink together, including on very short windows.
        let fit = ((self.viewport.x * 0.94) / width)
            .min((self.viewport.y * 0.94) / height.max(1.0))
            .clamp(0.0, 1.0);
        let ui = Self {
            s: self.s * fit,
            font,
            exact_font_size: true,
            ..*self
        };
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                overflow: Overflow::clip(),
                ..default()
            },
            // A screen owns the frame: the world behind it, standing still, doesn't show.
            BackgroundColor(color(Rgba(background.0, background.1, background.2, 255))),
            ScreenOverlay,
            Pickable::IGNORE,
        ))
        .with_children(|overlay| {
            if let Some((image, dimensions)) = backdrop {
                let size = fit_image(*dimensions, self.viewport);
                overlay.spawn((
                    ImageNode::new(image.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        width: self.px(size.x),
                        height: self.px(size.y),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            let node = Node {
                width: ui.px(width),
                height: ui.px(height),
                margin: UiRect::top(if backdrop.is_some() {
                    self.px(self.viewport.y * 0.6)
                } else {
                    px(0.0)
                }),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: ui.px(GAP),
                padding: UiRect::all(ui.px(if focus.is_some() { 6.0 } else { 0.0 })),
                flex_shrink: 0.0,
                ..default()
            };
            let panel = if focus.is_some() {
                ui.panel(overlay, node, (ScreenContent, Pickable::IGNORE))
            } else {
                overlay.spawn((node, ScreenContent, Pickable::IGNORE)).id()
            };
            overlay.commands_mut().entity(panel).with_children(|panel| {
                if let Some(((image, _), dimensions)) = logo.zip(logo_size) {
                    ui.screen_image(panel, image, dimensions);
                }
                let heading =
                    ui.write_wrapped(panel, heading, size, color(self.theme.text), text_width);
                if matches!(view, ScreenView::Title { .. }) {
                    panel
                        .commands_mut()
                        .entity(heading)
                        .insert((ScreenPrompt, Visibility::Inherited));
                }
                for (index, entry) in entries.iter().enumerate() {
                    let focused = focus == Some(index);
                    panel
                        .spawn((
                            Node {
                                width: ui.px(text_width),
                                padding: UiRect::all(ui.px(4.0)),
                                border: UiRect::all(ui.px(BORDER)),
                                flex_direction: FlexDirection::Column,
                                row_gap: ui.px(GAP),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            BorderColor::all(if focused {
                                color(self.theme.accent)
                            } else {
                                Color::NONE
                            }),
                            ScreenEntry,
                            Pickable::IGNORE,
                        ))
                        .with_children(|row| {
                            ui.write_wrapped(
                                row,
                                entry.name,
                                size,
                                color(if entry.done {
                                    self.theme.accent
                                } else {
                                    self.theme.text
                                }),
                                row_width,
                            );
                            if let Some(detail) = &entry.detail {
                                row.spawn(Node {
                                    align_items: AlignItems::Center,
                                    column_gap: ui.px(GAP),
                                    flex_shrink: 0.0,
                                    ..default()
                                })
                                .with_children(|line| {
                                    let icon = icon.filter(|_| entry.icon);
                                    if let Some((image, dimensions)) = icon {
                                        ui.screen_image(
                                            line,
                                            image,
                                            fit_image(*dimensions, Vec2::splat(ICON)),
                                        );
                                    }
                                    ui.write_wrapped(
                                        line,
                                        detail,
                                        size,
                                        color(self.theme.dim),
                                        row_width - if icon.is_some() { ICON + GAP } else { 0.0 },
                                    );
                                });
                            }
                        });
                }
            });
        });
    }

    fn screen_image(&self, parent: &mut ChildSpawnerCommands, image: &Handle<Image>, size: Vec2) {
        parent.spawn((
            ImageNode::new(image.clone()),
            Node {
                width: self.px(size.x),
                height: self.px(size.y),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

fn fit_image(size: Vec2, bounds: Vec2) -> Vec2 {
    let fit = (bounds.x / size.x.max(1.0)).min(bounds.y / size.y.max(1.0));
    size * if fit >= 1.0 { fit.floor() } else { fit }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::flow::{ActView, FileView};

    struct Fixture {
        screen: Screen,
        hud: Hud,
        shown: ShownScreen,
        viewport: Vec2,
    }

    impl Default for Fixture {
        fn default() -> Self {
            Self {
                screen: Screen::default(),
                hud: Hud::default(),
                shown: ShownScreen::default(),
                viewport: Vec2::new(320.0, 240.0),
            }
        }
    }

    impl Fixture {
        fn paint(&self, view: &ScreenView) -> World {
            let mut pictures = Images::default();
            let mut images = Assets::default();
            pictures.refresh(&self.screen.images, &mut images);
            pictures.refresh(&self.hud.icons, &mut images);
            let mut layouts = Assets::default();
            let mut fonts = (Fonts::default(), Fonts::default());
            fonts
                .0
                .refresh(self.screen.font.as_ref(), &mut images, &mut layouts);
            fonts
                .1
                .refresh(self.screen.dialog_font.as_ref(), &mut images, &mut layouts);
            let input = GameInput::default();
            let s = pixel_scale(self.viewport.y);
            let painter = Painter {
                s,
                viewport: self.viewport / s,
                exact_font_size: false,
                theme: &self.screen.theme,
                screen: &self.screen,
                pictures: &pictures,
                input: &input,
                hud: Some(&self.hud),
                health: None,
                lives: None,
                seconds: None,
                font: self
                    .screen
                    .font
                    .as_deref()
                    .zip(fonts.0.handles.as_ref())
                    .map(|(font, (image, layout))| (font, image, layout)),
                dialog_font: self
                    .screen
                    .dialog_font
                    .as_deref()
                    .zip(fonts.1.handles.as_ref())
                    .map(|(font, (image, layout))| (font, image, layout)),
            };
            let mut world = World::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            Commands::new(&mut queue, &world)
                .spawn_empty()
                .with_children(|root| {
                    painter.flow_screen(root, view, &self.shown);
                });
            queue.apply(&mut world);
            world
        }
    }

    fn texts(world: &mut World) -> Vec<String> {
        world
            .query::<&Text>()
            .iter(world)
            .map(|text| text.0.clone())
            .collect()
    }

    fn title() -> ScreenView {
        ScreenView::Title {
            logo: Some("game/logo".into()),
            logo_camera: None,
            prompt: "PRESS START".into(),
            background: Rgba(20, 30, 40, 255),
        }
    }

    #[test]
    fn title_draws_image_pixels_prompt_and_background() {
        let mut fixture = Fixture::default();
        fixture.screen.images = Arc::new(vec![("game/logo".into(), RgbaImage::new(200, 50))]);
        fixture.shown.model_logo_image = Some((Handle::default(), Vec2::splat(20.0)));
        let mut world = fixture.paint(&title());
        assert_eq!(texts(&mut world), ["PRESS START"]);
        let (image, node) = world.query::<(&ImageNode, &Node)>().single(&world).unwrap();
        assert_ne!(image.image, Handle::default());
        assert_eq!((node.width, node.height), (px(200.0), px(50.0)));
        assert_eq!(
            world
                .query_filtered::<&BackgroundColor, With<ScreenOverlay>>()
                .single(&world)
                .unwrap()
                .0,
            color(Rgba(20, 30, 40, 255))
        );
        assert_eq!(
            world
                .query_filtered::<Entity, With<ScreenPrompt>>()
                .iter(&world)
                .count(),
            1
        );
    }

    #[test]
    fn title_accepts_a_rendered_model_logo_or_no_logo() {
        let mut fixture = Fixture::default();
        let mut world = fixture.paint(&title());
        assert_eq!(world.query::<&ImageNode>().iter(&world).count(), 0);
        fixture.shown.model_logo_image = Some((Handle::default(), Vec2::new(400.0, 200.0)));
        let mut world = fixture.paint(&title());
        let node = world
            .query_filtered::<&Node, With<ImageNode>>()
            .single(&world)
            .unwrap();
        // Up to 55% of the 240-pixel screen's height: 132 of 200, keeping its shape.
        assert_eq!((node.width, node.height), (px(264.0), px(132.0)));
    }

    #[test]
    fn title_blinks_without_removing_its_layout() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .add_systems(Update, blink_system);
        let entity = app
            .world_mut()
            .spawn((ScreenPrompt, Visibility::Inherited))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(entity),
            Some(&Visibility::Inherited)
        );
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(std::time::Duration::from_millis(700));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(entity),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(std::time::Duration::from_millis(400));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(entity),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn files_show_names_new_counts_icons_and_focus() {
        let mut fixture = Fixture::default();
        fixture.hud.icons = Arc::new(vec![("star".into(), RgbaImage::new(64, 64))]);
        fixture.shown.counted_item = Some("star".into());
        let view = ScreenView::Files {
            title: "Choose a file".into(),
            slots: vec![
                FileView {
                    name: "Mario A".into(),
                    empty: true,
                    count: Some(99),
                },
                FileView {
                    name: "Mario B".into(),
                    empty: false,
                    count: Some(42),
                },
            ],
            focus: 1,
        };
        let mut world = fixture.paint(&view);
        assert_eq!(
            texts(&mut world),
            ["Choose a file", "Mario A", "NEW", "Mario B", "42"]
        );
        let borders: Vec<_> = world
            .query_filtered::<&BorderColor, With<ScreenEntry>>()
            .iter(&world)
            .copied()
            .collect();
        assert_eq!(
            borders,
            [
                BorderColor::all(Color::NONE),
                BorderColor::all(color(fixture.screen.theme.accent))
            ]
        );
        let node = world
            .query_filtered::<&Node, With<ImageNode>>()
            .single(&world)
            .unwrap();
        assert_eq!((node.width, node.height), (px(18.0), px(18.0)));
        fixture.hud.icons = Arc::default();
        let mut world = fixture.paint(&view);
        assert!(texts(&mut world).contains(&"42".into()));
        assert_eq!(world.query::<&ImageNode>().iter(&world).count(), 0);
    }

    #[test]
    fn acts_mark_done_with_accent_independently_of_focus() {
        let fixture = Fixture::default();
        let mut world = fixture.paint(&ScreenView::Acts {
            course: "Mountain".into(),
            acts: vec![
                ActView {
                    name: "First".into(),
                    done: true,
                },
                ActView {
                    name: "Second".into(),
                    done: false,
                },
            ],
            focus: 1,
        });
        assert_eq!(texts(&mut world), ["Mountain", "First", "Second"]);
        let colors: Vec<_> = world
            .query::<(&Text, &TextColor)>()
            .iter(&world)
            .map(|(text, c)| (text.0.clone(), c.0))
            .collect();
        assert!(colors.contains(&("First".into(), color(fixture.screen.theme.accent))));
        assert!(colors.contains(&("Second".into(), color(fixture.screen.theme.text))));
        let borders: Vec<_> = world
            .query_filtered::<&BorderColor, With<ScreenEntry>>()
            .iter(&world)
            .copied()
            .collect();
        assert_eq!(
            borders,
            [
                BorderColor::all(Color::NONE),
                BorderColor::all(color(fixture.screen.theme.accent))
            ]
        );
    }

    #[test]
    fn game_over_is_centered_and_uses_the_dialog_font() {
        let mut fixture = Fixture::default();
        fixture.screen.dialog_font = Some(Arc::new(BitmapFont {
            image: RgbaImage::new(8, 8),
            glyph: (8, 8),
            chars: "V".into(),
            advance: None,
        }));
        let mut world = fixture.paint(&ScreenView::GameOver { text: "V".into() });
        assert!(texts(&mut world).is_empty());
        assert_eq!(world.query::<&ImageNode>().iter(&world).count(), 1);
        let overlay = world
            .query_filtered::<&Node, With<ScreenOverlay>>()
            .single(&world)
            .unwrap();
        assert_eq!(overlay.justify_content, JustifyContent::Center);
        assert_eq!(overlay.align_items, AlignItems::Center);
        assert_eq!(
            world
                .query_filtered::<Entity, With<ScreenEntry>>()
                .iter(&world)
                .count(),
            0
        );
    }

    #[test]
    fn long_names_wrap_and_all_screens_fit_small_and_large_windows() {
        let long = "A very long name with many words ".repeat(10);
        let views = [
            ScreenView::Title {
                prompt: long.clone(),
                logo: None,
                logo_camera: None,
                background: Rgba(0, 0, 0, 255),
            },
            ScreenView::Files {
                title: long.clone(),
                slots: vec![
                    FileView {
                        name: long.clone(),
                        empty: true,
                        count: None
                    };
                    4
                ],
                focus: 0,
            },
            ScreenView::Acts {
                course: long.clone(),
                acts: vec![
                    ActView {
                        name: long.clone(),
                        done: false
                    };
                    6
                ],
                focus: 0,
            },
            ScreenView::GameOver { text: long.clone() },
        ];
        for viewport in [
            Vec2::new(32.0, 24.0),
            Vec2::new(320.0, 240.0),
            Vec2::new(1920.0, 1080.0),
        ] {
            let fixture = Fixture {
                viewport,
                ..default()
            };
            for view in &views {
                let mut world = fixture.paint(view);
                assert!(texts(&mut world).iter().any(|text| text.contains('\n')));
                let node = world
                    .query_filtered::<&Node, With<ScreenContent>>()
                    .single(&world)
                    .unwrap();
                let (Val::Px(width), Val::Px(height)) = (node.width, node.height) else {
                    panic!("pixel-sized content");
                };
                assert!(width > 0.0 && width <= viewport.x, "{width}");
                assert!(height > 0.0 && height <= viewport.y, "{height}");
            }
        }
    }

    #[test]
    fn selectors_use_the_theme_frame_and_wrap_bitmap_names() {
        let mut fixture = Fixture::default();
        fixture.screen.images = Arc::new(vec![("frame".into(), RgbaImage::new(16, 16))]);
        fixture.screen.theme.frame = Some(clockwork_core::ui::Frame {
            image: "frame".into(),
            border: 4,
        });
        fixture.screen.font = Some(Arc::new(BitmapFont {
            image: RgbaImage::new(8, 8),
            glyph: (8, 8),
            chars: "A".into(),
            advance: None,
        }));
        let mut world = fixture.paint(&ScreenView::Files {
            title: "A".into(),
            slots: vec![FileView {
                name: "A".repeat(100),
                empty: false,
                count: None,
            }],
            focus: 0,
        });
        let frame = world
            .query_filtered::<&ImageNode, With<ScreenContent>>()
            .single(&world)
            .unwrap();
        assert!(matches!(frame.image_mode, NodeImageMode::Sliced(_)));
        assert!(texts(&mut world).is_empty());
        let entry = world
            .query_filtered::<&Children, With<ScreenEntry>>()
            .single(&world)
            .unwrap();
        let lines = world.get::<Children>(entry[0]).unwrap();
        assert!(lines.len() > 1);
        assert_eq!(
            world
                .query::<&ImageNode>()
                .iter(&world)
                .filter(|image| image.texture_atlas.is_some())
                .count(),
            101
        );
    }

    #[test]
    fn hud_plugin_rebuilds_on_screen_changes_and_returns_to_the_hud() {
        use crate::hud::HudPlugin;
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<GameInput>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<TextureAtlasLayout>>()
            .add_plugins(HudPlugin);
        let camera = app
            .world_mut()
            .spawn((
                HudTarget,
                Camera {
                    computed: bevy::camera::ComputedCameraValues {
                        target_info: Some(bevy::camera::RenderTargetInfo {
                            physical_size: UVec2::new(320, 240),
                            scale_factor: 1.0,
                        }),
                        ..default()
                    },
                    ..default()
                },
            ))
            .id();
        app.world_mut().resource_mut::<ShownHud>().toast = Some("Toast".into());
        app.world_mut().resource_mut::<ShownScreen>().view = Some(title());
        app.update();
        assert_eq!(texts(app.world_mut()), ["PRESS START"]);
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<HudRoot>>()
            .single(app.world())
            .unwrap();
        assert_eq!(app.world().get::<UiTargetCamera>(root).unwrap().0, camera);
        app.update();
        assert!(app.world().get_entity(root).is_ok());
        app.world_mut().resource_mut::<ShownScreen>().view = Some(ScreenView::GameOver {
            text: "GAME OVER".into(),
        });
        app.update();
        assert!(app.world().get_entity(root).is_err());
        assert_eq!(texts(app.world_mut()), ["GAME OVER"]);
        app.world_mut().resource_mut::<ShownScreen>().view = Some(ScreenView::Acts {
            course: "Course".into(),
            acts: vec![
                ActView {
                    name: "Act".into(),
                    done: false,
                };
                2
            ],
            focus: 0,
        });
        app.update();
        if let Some(ScreenView::Acts { focus, .. }) =
            &mut app.world_mut().resource_mut::<ShownScreen>().view
        {
            *focus = 1;
        }
        app.world_mut()
            .get_mut::<Camera>(camera)
            .unwrap()
            .computed
            .target_info
            .as_mut()
            .unwrap()
            .physical_size = UVec2::new(80, 60);
        app.update();
        let borders: Vec<_> = app
            .world_mut()
            .query_filtered::<&BorderColor, With<ScreenEntry>>()
            .iter(app.world())
            .copied()
            .collect();
        assert_eq!(
            borders,
            [
                BorderColor::all(Color::NONE),
                BorderColor::all(color(Screen::default().theme.accent))
            ]
        );
        let node = app
            .world_mut()
            .query_filtered::<&Node, With<ScreenContent>>()
            .single(app.world())
            .unwrap();
        let (Val::Px(width), Val::Px(height)) = (node.width, node.height) else {
            panic!("pixel-sized content");
        };
        assert!(width <= 80.0 && height <= 60.0);
        app.world_mut().resource_mut::<ShownScreen>().view = None;
        app.update();
        assert_eq!(texts(app.world_mut()), ["Toast"]);
    }
}

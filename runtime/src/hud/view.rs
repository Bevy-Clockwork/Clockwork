//! Builds the HUD's and menus' UI nodes from [`ShownHud`], again whenever what they show changes:
//! each widget of the HUD layout where its anchor and offset put it, in the theme's look.

pub(super) mod screens;

use std::{collections::HashMap, sync::Arc};

use bevy::{
    asset::RenderAssetUsages,
    ecs::hierarchy::ChildSpawnerCommands,
    image::ImageSampler,
    picking::hover::Hovered,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    text::LineHeight,
};
use clockwork_core::{
    font::BitmapFont,
    image::RgbaImage,
    item::Slot,
    ui::{Anchor, MenuKind, Rgba, Theme, Widget, WidgetKind, Words},
};

use super::{
    DialogView, Hud, HudTarget, ItemView, Screen, ShownHud, ShownScreen,
    menu::{MenuSlotButton, MenuState, MenuTile},
    pixel_scale,
};
use crate::input::{Device, GameButton, GameInput};

/// An item button's box, in game pixels.
const SLOT: f32 = 26.0;
/// A cell of the inventory's grid.
const TILE: f32 = 22.0;
/// Icons fit into this square inside a box, scaled by whole steps.
const ICON: f32 = 18.0;
const BORDER: f32 = 1.0;
const RADIUS: f32 = 3.0;
const GAP: f32 = 4.0;

/// The HUD's top node.
#[derive(Component)]
pub struct HudRoot;

/// What the HUD was last built from.
#[derive(PartialEq)]
pub struct Built {
    shown: ShownHud,
    flow: ShownScreen,
    camera: Entity,
    scale: f32,
    viewport: Vec2,
    device: Device,
    open: Option<usize>,
    focus: usize,
}

/// Pictures as Bevy images, made once per list (item icons per play session, theme images per
/// screen).
#[derive(Default)]
pub struct Images {
    lists: HashMap<usize, HashMap<String, (Handle<Image>, Vec2)>>,
}

impl Images {
    fn refresh(&mut self, list: &Arc<Vec<(String, RgbaImage)>>, images: &mut Assets<Image>) {
        let key = Arc::as_ptr(list) as usize;
        self.lists.entry(key).or_insert_with(|| {
            list.iter()
                .map(|(id, image)| {
                    let size = Vec2::new(image.width as f32, image.height as f32);
                    (id.clone(), (images.add(to_bevy_image(image)), size))
                })
                .collect()
        });
    }

    fn get(
        &self,
        list: &Arc<Vec<(String, RgbaImage)>>,
        id: &str,
    ) -> Option<&(Handle<Image>, Vec2)> {
        self.lists.get(&(Arc::as_ptr(list) as usize))?.get(id)
    }

    /// Forgets lists no longer shown.
    fn keep(&mut self, keys: &[usize]) {
        self.lists.retain(|k, _| keys.contains(k));
    }
}

/// The theme's font as a Bevy image and glyph grid, made once per font.
#[derive(Default)]
pub struct Fonts {
    font: usize,
    handles: Option<(Handle<Image>, Handle<TextureAtlasLayout>)>,
}

impl Fonts {
    fn refresh(
        &mut self,
        font: Option<&Arc<BitmapFont>>,
        images: &mut Assets<Image>,
        layouts: &mut Assets<TextureAtlasLayout>,
    ) {
        let key = font.map_or(0, |f| Arc::as_ptr(f) as usize);
        if key == self.font {
            return;
        }
        self.font = key;
        self.handles = font.map(|font| {
            let layout = TextureAtlasLayout::from_grid(
                UVec2::new(font.glyph.0, font.glyph.1),
                font.columns(),
                font.rows(),
                None,
                None,
            );
            (images.add(to_bevy_image(&font.image)), layouts.add(layout))
        });
    }
}

fn to_bevy_image(image: &RgbaImage) -> Image {
    let mut bevy_image = Image::new(
        Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        image.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Keep pixel art crisp.
    bevy_image.sampler = ImageSampler::nearest();
    bevy_image
}

fn color(c: Rgba) -> Color {
    Color::srgba_u8(c.0, c.1, c.2, c.3)
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn build_system(
    mut commands: Commands,
    shown: Res<ShownHud>,
    flow: Res<ShownScreen>,
    input: Res<GameInput>,
    menu: Res<MenuState>,
    cameras: Query<(Entity, &Camera), With<HudTarget>>,
    roots: Query<Entity, With<HudRoot>>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut built: Local<Option<Built>>,
    mut pictures: Local<Images>,
    mut fonts: Local<(Fonts, Fonts)>,
) {
    let Some((camera, size)) = cameras
        .iter()
        .find_map(|(e, c)| Some((e, c.logical_viewport_size()?)))
    else {
        return;
    };
    let now = Built {
        shown: shown.clone(),
        flow: flow.clone(),
        camera,
        scale: pixel_scale(size.y),
        viewport: size,
        device: input.device,
        open: menu.open,
        focus: menu.focus,
    };
    if built.as_ref() == Some(&now) {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let screen = shown.screen.clone().unwrap_or_default();
    fonts
        .0
        .refresh(screen.font.as_ref(), &mut images, &mut layouts);
    fonts
        .1
        .refresh(screen.dialog_font.as_ref(), &mut images, &mut layouts);
    let mut lists = vec![Arc::as_ptr(&screen.images) as usize];
    pictures.refresh(&screen.images, &mut images);
    if let Some(hud) = &shown.hud {
        pictures.refresh(&hud.icons, &mut images);
        lists.push(Arc::as_ptr(&hud.icons) as usize);
    }
    pictures.keep(&lists);
    let ui = Painter {
        s: now.scale,
        viewport: size / now.scale,
        exact_font_size: false,
        theme: &screen.theme,
        screen: &screen,
        pictures: &pictures,
        input: &input,
        hud: shown.hud.as_ref(),
        health: shown.health,
        lives: shown.lives,
        seconds: shown.seconds,
        font: screen
            .font
            .as_deref()
            .zip(fonts.0.handles.as_ref())
            .map(|(font, (image, layout))| (font, image, layout)),
        dialog_font: screen
            .dialog_font
            .as_deref()
            .zip(fonts.1.handles.as_ref())
            .map(|(font, (image, layout))| (font, image, layout)),
    };
    commands
        .spawn((
            HudRoot,
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|root| {
            if let Some(view) = &flow.view {
                ui.flow_screen(root, view, &flow);
                return;
            }
            if let Some(toast) = &shown.toast {
                ui.toast(root, toast);
            }
            if shown.screen.is_none() {
                return;
            }
            for widget in &screen.layout.widgets {
                ui.widget(root, widget);
            }
            if let Some(banner) = &shown.banner {
                ui.banner(root, banner);
            }
            if let Some(dialog) = &shown.dialog {
                ui.dialog(root, dialog);
            }
            if let Some(open) = menu.open.and_then(|i| screen.menus.get(i)) {
                ui.menu(root, &open.title, &open.kind, menu.focus);
            }
        });
    // A door's fade covers everything.
    if shown.fade > 0 {
        commands.spawn((
            HudRoot,
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, shown.fade as f32 / 16.0)),
            GlobalZIndex(10),
            Pickable::IGNORE,
        ));
    }
    *built = Some(now);
}

fn glyph_width(font: &BitmapFont, ch: char) -> f32 {
    let advance = font.advance_of(ch);
    if font.cell(ch).is_none() && !ch.is_whitespace() {
        advance.max(font.glyph.1) as f32
    } else {
        advance as f32
    }
}

fn fallback_glyph(
    font: &BitmapFont,
    ch: char,
    height: f32,
) -> Option<(Text, TextFont, LineHeight)> {
    (font.cell(ch).is_none() && !ch.is_whitespace()).then(|| {
        (
            Text::new(ch.to_string()),
            TextFont::from_font_size(height),
            LineHeight::Px(height),
        )
    })
}

/// Keeps words together when possible, splitting words wider than a line at characters.
fn wrap_text(text: &str, width: f32, advance: impl Fn(char) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let space = advance(' ');
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0.0;
        for word in paragraph.split_whitespace() {
            let word_width: f32 = word.chars().map(&advance).sum();
            if !line.is_empty() {
                if used + space + word_width > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0.0;
                } else {
                    line.push(' ');
                    used += space;
                }
            }
            for ch in word.chars() {
                let next = advance(ch);
                if !line.is_empty() && used + next > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0.0;
                }
                line.push(ch);
                used += next;
            }
        }
        lines.push(line);
    }
    lines
}

fn wrap_bitmap(font: &BitmapFont, text: &str, scale: f32, width: f32) -> Vec<String> {
    // A cell can extend past its advance, including at the right edge of a line.
    let overhang = text.chars().fold(0.0_f32, |max, ch| {
        max.max(font.glyph.0 as f32 - glyph_width(font, ch))
    });
    wrap_text(text, width - overhang * scale, |ch| {
        glyph_width(font, ch) * scale
    })
}

fn text_height(font: Option<&BitmapFont>, text: &str, size: f32, width: f32) -> f32 {
    let lines = font
        .map_or_else(
            || wrap_text(text, width, |_| size),
            |font| wrap_bitmap(font, text, size / font.glyph.1.max(1) as f32, width),
        )
        .len() as f32;
    font.map_or(lines * size * 1.2, |f| {
        lines * size + (lines - 1.0) * size / f.glyph.1.max(1) as f32
    })
}

fn fit_dialog_size(
    font: Option<&BitmapFont>,
    blocks: &[&str; 3],
    size: f32,
    width: f32,
    height: f32,
) -> f32 {
    let widest = blocks
        .iter()
        .flat_map(|text| text.chars())
        .fold(1.0_f32, |max, ch| {
            max.max(font.map_or(1.0, |f| {
                glyph_width(f, ch).max(f.glyph.0 as f32) / f.glyph.1.max(1) as f32
            }))
        });
    let size = size.min(width / widest);
    let used: f32 = blocks
        .iter()
        .enumerate()
        .map(|(i, text)| text_height(font, text, if i == 2 { size * 0.75 } else { size }, width))
        .sum();
    // Smaller text cannot add wrapped lines, so this also fits pages with explicit newlines.
    size * (height / used.max(1.0)).min(1.0)
}

type FontImage<'a> = (
    &'a BitmapFont,
    &'a Handle<Image>,
    &'a Handle<TextureAtlasLayout>,
);

/// Spawns the HUD's parts at scale `s`, in the theme's look.
#[derive(Clone, Copy)]
struct Painter<'a> {
    s: f32,
    viewport: Vec2,
    exact_font_size: bool,
    theme: &'a Theme,
    screen: &'a Screen,
    pictures: &'a Images,
    input: &'a GameInput,
    hud: Option<&'a Hud>,
    health: Option<(f32, f32)>,
    lives: Option<u32>,
    seconds: Option<u32>,
    /// The theme's bitmap font, its image and its glyph grid.
    font: Option<FontImage<'a>>,
    dialog_font: Option<FontImage<'a>>,
}

impl Painter<'_> {
    fn px(&self, v: f32) -> Val {
        px(v * self.s)
    }

    fn text(&self, text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
        (
            Text::new(text),
            TextFont::from_font_size(size * self.s),
            TextColor(color),
            // Readable over any ground, in or out of a panel.
            TextShadow {
                offset: Vec2::splat(self.s.max(1.0)),
                color: Color::srgba(0.0, 0.0, 0.0, 0.85),
            },
            Pickable::IGNORE,
        )
    }

    fn font(&self) -> f32 {
        self.theme.font_size
    }

    /// Writes `text` at `size` game pixels: in the theme's bitmap font if it has one, at a
    /// whole-number scale, else in the default font.
    fn write(
        &self,
        parent: &mut ChildSpawnerCommands,
        text: impl Into<String>,
        size: f32,
        c: Color,
    ) -> Entity {
        self.write_in(parent, text.into(), size, c, None)
    }

    /// Like [`Painter::write`], wrapping at `width` game pixels.
    fn write_wrapped(
        &self,
        parent: &mut ChildSpawnerCommands,
        text: impl Into<String>,
        size: f32,
        c: Color,
        width: f32,
    ) -> Entity {
        self.write_in(parent, text.into(), size, c, Some(width))
    }

    fn write_in(
        &self,
        parent: &mut ChildSpawnerCommands,
        text: String,
        size: f32,
        c: Color,
        width: Option<f32>,
    ) -> Entity {
        let Some((font, image, layout)) = self.font else {
            let text = if self.exact_font_size {
                wrap_text(&text, width.unwrap_or(f32::INFINITY), |_| size).join("\n")
            } else {
                text
            };
            let mut node = parent.spawn(self.text(text, size, c));
            if self.exact_font_size {
                node.insert((TextLayout::no_wrap(), LineHeight::RelativeToFont(1.2)));
            }
            if let Some(width) = width {
                node.insert(Node {
                    max_width: self.px(width),
                    flex_shrink: 0.0,
                    ..default()
                });
            }
            return node.id();
        };
        let scale = size / font.glyph.1.max(1) as f32;
        let scale = if self.exact_font_size {
            scale
        } else {
            scale.round().max(1.0)
        };
        let k = scale * self.s;
        let height = font.glyph.1.max(1) as f32 * k;
        let lines = wrap_bitmap(font, &text, scale, width.unwrap_or(f32::INFINITY));
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(k),
                    max_width: width.map_or(Val::Auto, |w| self.px(w)),
                    flex_shrink: 0.0,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|parent| {
                for line in lines {
                    parent
                        .spawn((
                            Node {
                                height: px(height),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_children(|glyphs| {
                            for ch in line.chars() {
                                let advance = glyph_width(font, ch) * k;
                                let mut slot = glyphs.spawn((
                                    Node {
                                        width: px(advance),
                                        height: px(height),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    Pickable::IGNORE,
                                ));
                                if let Some(index) = font.cell(ch) {
                                    slot.with_children(|slot| {
                                        slot.spawn((
                                            ImageNode {
                                                color: c,
                                                texture_atlas: Some(TextureAtlas {
                                                    layout: layout.clone(),
                                                    index,
                                                }),
                                                ..ImageNode::new(image.clone())
                                            },
                                            Node {
                                                position_type: PositionType::Absolute,
                                                width: px(font.glyph.0 as f32 * k),
                                                height: px(height),
                                                ..default()
                                            },
                                            Pickable::IGNORE,
                                        ));
                                    });
                                } else if let Some(fallback) = fallback_glyph(font, ch, height) {
                                    slot.with_children(|slot| {
                                        slot.spawn((fallback, TextColor(c), Pickable::IGNORE));
                                    });
                                }
                            }
                        });
                }
            })
            .id()
    }

    fn small(&self) -> f32 {
        (self.theme.font_size * 0.75).round().max(5.0)
    }

    /// A panel: the theme's frame picture, or its colours, border and corners.
    fn panel(&self, parent: &mut ChildSpawnerCommands, node: Node, extra: impl Bundle) -> Entity {
        let frame = self.theme.frame.as_ref().and_then(|frame| {
            let (image, _) = self.pictures.get(&self.screen.images, &frame.image)?;
            Some((image.clone(), frame.border as f32))
        });
        match frame {
            Some((image, border)) => parent
                .spawn((
                    Node {
                        border: UiRect::all(self.px(border)),
                        ..node
                    },
                    ImageNode {
                        image_mode: NodeImageMode::Sliced(TextureSlicer {
                            border: BorderRect::all(border),
                            max_corner_scale: self.s,
                            ..default()
                        }),
                        ..ImageNode::new(image)
                    },
                    extra,
                ))
                .id(),
            None => parent
                .spawn((
                    Node {
                        border: UiRect::all(self.px(BORDER)),
                        border_radius: BorderRadius::all(self.px(RADIUS)),
                        ..node
                    },
                    BackgroundColor(color(self.theme.panel)),
                    BorderColor::all(color(self.theme.border)),
                    extra,
                ))
                .id(),
        }
    }

    /// An item's picture, scaled by whole steps to fit the icon square.
    /// A picture the screen names (`"plugin/output"`), at the size of an item icon.
    fn picture(&self, parent: &mut ChildSpawnerCommands, id: &str) {
        if let Some((image, size)) = self.pictures.get(&self.screen.images, id) {
            self.image(parent, image.clone(), *size, false);
        }
    }

    fn icon(&self, parent: &mut ChildSpawnerCommands, item: &str, dim: bool) {
        let Some(hud) = self.hud else {
            return;
        };
        let Some((image, size)) = self.pictures.get(&hud.icons, item) else {
            return;
        };
        self.image(parent, image.clone(), *size, dim);
    }

    fn image(
        &self,
        parent: &mut ChildSpawnerCommands,
        image: Handle<Image>,
        size: Vec2,
        dim: bool,
    ) {
        let fit = ((ICON * self.s) / size.x.max(size.y)).floor().max(1.0);
        parent.spawn((
            ImageNode {
                color: Color::WHITE.with_alpha(if dim { 0.4 } else { 1.0 }),
                ..ImageNode::new(image)
            },
            Node {
                width: px(size.x * fit),
                height: px(size.y * fit),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }

    /// Text in a corner of its parent.
    fn corner(&self, parent: &mut ChildSpawnerCommands, text: &str, top_left: bool, c: Color) {
        let node = if top_left {
            Node {
                position_type: PositionType::Absolute,
                left: self.px(1.0),
                top: self.px(0.0),
                ..default()
            }
        } else {
            Node {
                position_type: PositionType::Absolute,
                right: self.px(1.0),
                bottom: self.px(0.0),
                ..default()
            }
        };
        parent
            .spawn((node, Pickable::IGNORE))
            .with_children(|corner| {
                self.write(corner, text, self.small(), c);
            });
    }

    fn toast(&self, root: &mut ChildSpawnerCommands, text: &str) {
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: self.px(24.0),
                width: percent(100),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            let panel = self.panel(
                row,
                Node {
                    padding: UiRect::axes(self.px(6.0), self.px(3.0)),
                    ..default()
                },
                (),
            );
            row.commands_mut().entity(panel).with_children(|panel| {
                self.write(panel, text, self.font(), color(self.theme.text));
            });
        });
    }

    /// A widget where its anchor and offset put it.
    fn widget(&self, root: &mut ChildSpawnerCommands, widget: &Widget) {
        use Anchor::*;
        let (dx, dy) = (widget.offset.0 as f32, widget.offset.1 as f32);
        let (justify, align) = match widget.anchor {
            TopLeft => (JustifyContent::FlexStart, AlignItems::FlexStart),
            Top => (JustifyContent::Center, AlignItems::FlexStart),
            TopRight => (JustifyContent::FlexEnd, AlignItems::FlexStart),
            Left => (JustifyContent::FlexStart, AlignItems::Center),
            Center => (JustifyContent::Center, AlignItems::Center),
            Right => (JustifyContent::FlexEnd, AlignItems::Center),
            BottomLeft => (JustifyContent::FlexStart, AlignItems::FlexEnd),
            Bottom => (JustifyContent::Center, AlignItems::FlexEnd),
            BottomRight => (JustifyContent::FlexEnd, AlignItems::FlexEnd),
        };
        // The offset pushes inwards from the anchored sides; on a centred axis it moves right
        // or down.
        let right = matches!(widget.anchor, TopRight | Right | BottomRight);
        let bottom = matches!(widget.anchor, BottomLeft | Bottom | BottomRight);
        let padding = UiRect {
            left: self.px(if right { 0.0 } else { dx.max(0.0) }),
            right: self.px(if right { dx } else { (-dx).max(0.0) }),
            top: self.px(if bottom { 0.0 } else { dy.max(0.0) }),
            bottom: self.px(if bottom { dy } else { (-dy).max(0.0) }),
        };
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: justify,
                align_items: align,
                padding,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|place| self.widget_body(place, widget));
    }

    fn words(&self) -> &Words {
        &self.screen.layout.words
    }

    /// A widget's picture (its own, else `item`'s icon) and its number written its way.
    fn counted(
        &self,
        place: &mut ChildSpawnerCommands,
        widget: &Widget,
        item: Option<&str>,
        n: u32,
        c: Color,
    ) {
        place
            .spawn(Node {
                column_gap: self.px(2.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                match (&widget.icon, item) {
                    (Some(icon), _) => self.picture(row, icon),
                    (None, Some(item)) => self.icon(row, item, false),
                    (None, None) => {}
                }
                self.write(row, widget.text(n), self.font(), c);
            });
    }

    fn widget_body(&self, place: &mut ChildSpawnerCommands, widget: &Widget) {
        match &widget.kind {
            WidgetKind::ItemSlot(slot) => {
                if self.hud.is_some() {
                    self.slot_box(place, *slot);
                }
            }
            WidgetKind::Button { label, button } => {
                let Some(button) = GameButton::named(button) else {
                    return;
                };
                // A dodge button only for characters that dodge, items only with items.
                let shown = match button {
                    GameButton::Dodge => self.hud.is_some_and(|h| h.has_dodge),
                    GameButton::Menu => self.hud.is_some_and(|h| !h.items.is_empty()),
                    _ => true,
                };
                if shown {
                    self.key_box(place, label, button);
                }
            }
            WidgetKind::Action => {
                if let Some(action) = self.hud.and_then(|h| h.action.as_deref()) {
                    self.write(place, action, self.small(), color(self.theme.dim));
                }
            }
            WidgetKind::Health { per_heart } => {
                if let Some((health, max)) = self.health {
                    self.health(place, health, max, *per_heart);
                }
            }
            WidgetKind::Counter { item } => {
                let Some(view) = self
                    .hud
                    .and_then(|h| h.items.iter().find(|i| i.id == *item))
                else {
                    return;
                };
                self.counted(
                    place,
                    widget,
                    Some(item),
                    view.count,
                    color(self.theme.text),
                );
            }
            WidgetKind::Label(text) => {
                self.write(place, text, self.font(), color(self.theme.text));
            }
            WidgetKind::Lives => {
                if let Some(lives) = self.lives {
                    self.counted(place, widget, None, lives, color(self.theme.text));
                }
            }
            WidgetKind::Timer => {
                if let Some(seconds) = self.seconds {
                    let c = if seconds <= 10 {
                        self.theme.accent
                    } else {
                        self.theme.text
                    };
                    self.counted(place, widget, None, seconds, color(c));
                }
            }
        }
    }

    /// A dialog's page in a box at the bottom of the screen.
    fn dialog(&self, root: &mut ChildSpawnerCommands, dialog: &DialogView) {
        let ui = Self {
            font: self.dialog_font.or(self.font),
            exact_font_size: true,
            ..*self
        };
        let width = 220.0_f32.min((self.viewport.x - 20.0).max(1.0));
        let border = self
            .theme
            .frame
            .as_ref()
            .map_or(BORDER, |f| f.border as f32);
        let inset = (6.0 + border).min(width / 4.0);
        let text_width = (width - inset * 2.0).max(1.0);
        let available = (self.viewport.y - 20.0 - inset * 2.0 - 6.0).max(1.0);
        let size = self.theme.dialog_font_size.unwrap_or(self.font());
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            8.0
        };
        let hint = format!(
            "{}: {}",
            self.input.label(GameButton::A),
            if dialog.more {
                &self.words().next
            } else {
                &self.words().close
            }
        );
        let font = ui.font.map(|(font, _, _)| font);
        let size = fit_dialog_size(
            font,
            &[dialog.speaker.as_deref().unwrap_or(""), &dialog.text, &hint],
            size,
            text_width,
            available,
        );
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexEnd,
                padding: UiRect::all(self.px(10.0)),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|bottom| {
            let panel = self.panel(
                bottom,
                Node {
                    flex_direction: FlexDirection::Column,
                    width: self.px(width),
                    max_height: self.px((self.viewport.y - 20.0).max(1.0)),
                    padding: UiRect::all(self.px((inset - border).max(0.0))),
                    row_gap: self.px(3.0),
                    flex_shrink: 0.0,
                    overflow: Overflow::clip(),
                    ..default()
                },
                (),
            );
            bottom.commands_mut().entity(panel).with_children(|panel| {
                if let Some(speaker) = &dialog.speaker {
                    ui.write_wrapped(panel, speaker, size, color(self.theme.accent), text_width);
                }
                ui.write_wrapped(
                    panel,
                    &dialog.text,
                    size,
                    color(self.theme.text),
                    text_width,
                );
                ui.write_wrapped(panel, hint, size * 0.75, color(self.theme.dim), text_width);
            });
        });
    }

    /// Big text in the middle of the screen.
    fn banner(&self, root: &mut ChildSpawnerCommands, text: &str) {
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|middle| {
            let panel = self.panel(
                middle,
                Node {
                    padding: UiRect::axes(self.px(10.0), self.px(5.0)),
                    ..default()
                },
                (),
            );
            middle.commands_mut().entity(panel).with_children(|panel| {
                self.write(panel, text, self.font() * 2.0, color(self.theme.accent));
            });
        });
    }

    /// Hearts, `per_heart` health each (part-filled too), or a bar.
    fn health(
        &self,
        place: &mut ChildSpawnerCommands,
        health: f32,
        max: f32,
        per_heart: Option<f32>,
    ) {
        let full = color(self.theme.accent);
        let empty = color(self.theme.panel);
        let edge = color(self.theme.border);
        match per_heart.filter(|p| *p > 0.0) {
            Some(per) => {
                let hearts = (max / per).ceil() as usize;
                place
                    .spawn(Node {
                        column_gap: self.px(1.0),
                        ..default()
                    })
                    .with_children(|row| {
                        for i in 0..hearts {
                            let fill = ((health - i as f32 * per) / per).clamp(0.0, 1.0);
                            row.spawn((
                                Node {
                                    width: self.px(7.0),
                                    height: self.px(7.0),
                                    border: UiRect::all(self.px(BORDER)),
                                    border_radius: BorderRadius::all(self.px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(empty),
                                BorderColor::all(edge),
                            ))
                            .with_children(|heart| {
                                heart.spawn((
                                    Node {
                                        width: percent(fill * 100.0),
                                        height: percent(100),
                                        ..default()
                                    },
                                    BackgroundColor(full),
                                ));
                            });
                        }
                    });
            }
            None => {
                place
                    .spawn((
                        Node {
                            width: self.px(60.0),
                            height: self.px(6.0),
                            border: UiRect::all(self.px(BORDER)),
                            ..default()
                        },
                        BackgroundColor(empty),
                        BorderColor::all(edge),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Node {
                                width: percent((health / max.max(0.001)).clamp(0.0, 1.0) * 100.0),
                                height: percent(100),
                                ..default()
                            },
                            BackgroundColor(full),
                        ));
                    });
            }
        }
    }

    /// One item button: its letter, the item on it, how many are left and its key below.
    fn slot_box(&self, place: &mut ChildSpawnerCommands, slot: Slot) {
        let Some(hud) = self.hud else {
            return;
        };
        let view = hud.slots[slot.index()].as_ref();
        let (letter, button) = match slot {
            Slot::A => ("A", GameButton::A),
            Slot::B => ("B", GameButton::B),
        };
        place
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: self.px(1.0),
                ..default()
            })
            .with_children(|column| {
                let cell = self.panel(
                    column,
                    Node {
                        width: self.px(SLOT),
                        height: self.px(SLOT),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    (),
                );
                column.commands_mut().entity(cell).with_children(|cell| {
                    if let Some(view) = view {
                        self.icon(cell, &view.id, !view.usable);
                    }
                    self.corner(cell, letter, true, color(self.theme.accent));
                    if let Some(count) = view.and_then(|v| v.count.as_deref()) {
                        self.corner(
                            cell,
                            count.trim_start_matches("x "),
                            false,
                            color(self.theme.text),
                        );
                    }
                });
                self.write(
                    column,
                    self.input.label(button),
                    self.small(),
                    color(self.theme.text),
                );
            });
    }

    /// A button without an item: its name, and its key below.
    fn key_box(&self, place: &mut ChildSpawnerCommands, name: &str, button: GameButton) {
        place
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: self.px(1.0),
                ..default()
            })
            .with_children(|column| {
                let panel = self.panel(
                    column,
                    Node {
                        padding: UiRect::axes(self.px(3.0), self.px(2.0)),
                        ..default()
                    },
                    (),
                );
                column.commands_mut().entity(panel).with_children(|panel| {
                    self.write(panel, name, self.small(), color(self.theme.text));
                });
                self.write(
                    column,
                    self.input.label(button),
                    self.small(),
                    color(self.theme.text),
                );
            });
    }

    /// An open menu over the dimmed game.
    fn menu(&self, root: &mut ChildSpawnerCommands, title: &str, kind: &MenuKind, focus: usize) {
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(color(self.theme.shade)),
        ))
        .with_children(|shade| {
            let panel = self.panel(
                shade,
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(self.px(6.0)),
                    row_gap: self.px(GAP),
                    ..default()
                },
                (),
            );
            shade.commands_mut().entity(panel).with_children(|panel| {
                self.write(panel, title, self.font() + 2.0, color(self.theme.accent));
                match kind {
                    MenuKind::Inventory { columns } => self.inventory(panel, *columns, focus),
                    MenuKind::Pause => self.pause(panel, focus),
                }
            });
        });
    }

    fn inventory(&self, panel: &mut ChildSpawnerCommands, columns: u32, focus: usize) {
        let items = self.hud.map(|h| h.menu_items()).unwrap_or_default();
        let focus = focus.min(items.len().saturating_sub(1));
        if items.is_empty() {
            self.write(
                panel,
                &self.words().nothing_carried,
                self.font(),
                color(self.theme.dim),
            );
        } else {
            panel
                .spawn(Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(
                        columns.max(1) as u16,
                        TILE * self.s,
                    ),
                    column_gap: self.px(2.0),
                    row_gap: self.px(2.0),
                    ..default()
                })
                .with_children(|grid| {
                    for (i, item) in items.iter().enumerate() {
                        self.tile(grid, i, item, i == focus);
                    }
                });
            self.details(panel, items[focus], columns);
        }
        self.write(
            panel,
            // The default font has no middle dot.
            format!(
                "{}: {}   {}: {}   {}: {}",
                self.input.label(GameButton::B),
                self.words().on_b,
                self.input.label(GameButton::A),
                self.words().on_a,
                self.input.label(GameButton::Menu),
                self.words().close,
            ),
            self.small(),
            color(self.theme.dim),
        );
    }

    fn pause(&self, panel: &mut ChildSpawnerCommands, focus: usize) {
        let words = self.words();
        let entries = [&words.resume, &words.start_over, &words.stop_playing];
        for (i, entry) in entries.into_iter().enumerate() {
            let on = i == focus;
            panel
                .spawn((
                    Node {
                        padding: UiRect::axes(self.px(4.0), self.px(1.0)),
                        border: UiRect::all(self.px(BORDER)),
                        border_radius: BorderRadius::all(self.px(RADIUS)),
                        ..default()
                    },
                    BorderColor::all(if on {
                        color(self.theme.accent)
                    } else {
                        Color::NONE
                    }),
                    MenuTile(i),
                    Hovered::default(),
                ))
                .with_children(|row| {
                    self.write(
                        row,
                        entry,
                        self.font(),
                        color(if on {
                            self.theme.accent
                        } else {
                            self.theme.text
                        }),
                    );
                });
        }
        self.write(
            panel,
            format!(
                "{}: {}   {}: {}",
                self.input.label(GameButton::A),
                self.words().choose,
                self.input.label(GameButton::Back),
                self.words().back,
            ),
            self.small(),
            color(self.theme.dim),
        );
    }

    /// An item of the grid, with the letters of the buttons it is on.
    fn tile(&self, grid: &mut ChildSpawnerCommands, index: usize, item: &ItemView, focused: bool) {
        grid.spawn((
            Node {
                width: self.px(TILE),
                height: self.px(TILE),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(self.px(BORDER)),
                border_radius: BorderRadius::all(self.px(RADIUS)),
                ..default()
            },
            BackgroundColor(color(self.theme.panel)),
            BorderColor::all(if focused {
                color(self.theme.accent)
            } else {
                color(self.theme.border).with_alpha(0.2)
            }),
            MenuTile(index),
            Hovered::default(),
        ))
        .with_children(|cell| {
            self.icon(cell, &item.id, !item.owned);
            if let Some(slot) = item.on {
                let letter = if slot == Slot::A { "A" } else { "B" };
                self.corner(cell, letter, true, color(self.theme.accent));
            }
        });
    }

    /// The chosen item: name, count, description and buttons to put it on A or B.
    fn details(&self, panel: &mut ChildSpawnerCommands, item: &ItemView, columns: u32) {
        panel
            .spawn(Node {
                column_gap: self.px(GAP),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                let c = if item.owned {
                    self.theme.text
                } else {
                    self.theme.dim
                };
                self.write(row, &item.name, self.font(), color(c));
                if item.max > 1 {
                    self.write(
                        row,
                        format!("{} / {}", item.count, item.max),
                        self.small(),
                        color(self.theme.dim),
                    );
                }
                if item.can_equip {
                    for (slot, letter) in [(Slot::B, "B"), (Slot::A, "A")] {
                        let on = item.on == Some(slot);
                        let button = self.panel(
                            row,
                            Node {
                                padding: UiRect::axes(self.px(3.0), self.px(1.0)),
                                ..default()
                            },
                            (MenuSlotButton(slot), Hovered::default()),
                        );
                        row.commands_mut().entity(button).with_children(|button| {
                            self.write(
                                button,
                                letter,
                                self.small(),
                                color(if on {
                                    self.theme.accent
                                } else {
                                    self.theme.text
                                }),
                            );
                        });
                    }
                }
            });
        if !item.description.is_empty() {
            self.write_wrapped(
                panel,
                &item.description,
                self.small(),
                color(self.theme.dim),
                columns.max(1) as f32 * (TILE + 2.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(draw: impl FnOnce(&Painter, &mut ChildSpawnerCommands)) -> World {
        let screen = Screen {
            theme: Theme {
                dialog_font_size: Some(8.0),
                ..clockwork_core::ui::default_theme()
            },
            ..default()
        };
        let font = font();
        let dialog_font = BitmapFont {
            chars: "V".into(),
            ..font.clone()
        };
        let image = Handle::default();
        let layout = Handle::default();
        let pictures = Images::default();
        let input = GameInput::default();
        let painter = Painter {
            s: 1.0,
            viewport: Vec2::new(320.0, 240.0),
            exact_font_size: false,
            theme: &screen.theme,
            screen: &screen,
            pictures: &pictures,
            input: &input,
            hud: None,
            health: None,
            lives: None,
            seconds: None,
            font: Some((&font, &image, &layout)),
            dialog_font: Some((&dialog_font, &image, &layout)),
        };
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        Commands::new(&mut queue, &world)
            .spawn_empty()
            .with_children(|parent| draw(&painter, parent));
        queue.apply(&mut world);
        world
    }

    #[test]
    fn hud_spawns_default_text_for_the_missing_v() {
        let mut world = paint(|painter, parent| {
            painter.write(parent, "LIVES", 16.0, Color::WHITE);
        });
        let text: Vec<_> = world
            .query::<&Text>()
            .iter(&world)
            .map(|text| text.0.as_str())
            .collect();
        assert_eq!(text, ["V"]);
        assert_eq!(world.query::<&ImageNode>().iter(&world).count(), 4);
    }

    #[test]
    fn dialog_spawns_its_own_font_at_the_requested_height() {
        let mut world = paint(|painter, parent| {
            painter.dialog(
                parent,
                &DialogView {
                    speaker: None,
                    text: "V".into(),
                    more: false,
                },
            );
        });
        assert!(
            !world
                .query::<&Text>()
                .iter(&world)
                .any(|text| text.0 == "V")
        );
        let glyphs: Vec<_> = world
            .query::<(&ImageNode, &Node)>()
            .iter(&world)
            .filter(|(image, _)| image.texture_atlas.is_some())
            .collect();
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].1.height, px(8.0));
    }

    fn font() -> BitmapFont {
        BitmapFont {
            image: RgbaImage::new(64, 16),
            glyph: (16, 16),
            chars: "LIES".into(),
            advance: None,
        }
    }

    #[test]
    fn wraps_words_at_the_requested_size() {
        let font = font();
        assert_eq!(
            wrap_text("LIVES LEFT", 80.0, |ch| glyph_width(&font, ch)),
            ["LIVES", "LEFT"]
        );
        assert_eq!(
            wrap_text("LIVES LEFT", 80.0, |ch| glyph_width(&font, ch) * 0.5),
            ["LIVES LEFT"]
        );
    }

    #[test]
    fn wrapping_uses_variable_advances() {
        assert_eq!(
            wrap_text("ii WW ii", 8.0, |ch| if ch == 'W' { 3.0 } else { 1.0 }),
            ["ii", "WW", "ii"]
        );
        assert_eq!(wrap_text("one two", 7.0, |_| 1.0), ["one two"]);
    }

    #[test]
    fn wrapping_keeps_the_last_cell_inside_the_box() {
        let font = BitmapFont {
            advance: Some(vec![5; 4]),
            ..font()
        };
        let lines = wrap_bitmap(&font, "LLLLLLLLLL", 1.0, 32.0);
        assert_eq!(lines, ["LLLL", "LLLL", "LL"]);
        for line in lines {
            assert!((line.len() - 1) * 5 + 16 <= 32);
        }
    }

    #[test]
    fn wrapping_preserves_newlines_and_empty_lines() {
        assert_eq!(
            wrap_text("one\n\ntwo\n", 10.0, |_| 1.0),
            ["one", "", "two", ""]
        );
        assert_eq!(wrap_text("", 10.0, |_| 1.0), [""]);
        assert_eq!(wrap_text("  one\t two  ", 10.0, |_| 1.0), ["one two"]);
    }

    #[test]
    fn long_words_split_at_characters() {
        assert_eq!(
            wrap_text("\u{e9}\u{6c34}abc", 2.0, |_| 1.0),
            ["\u{e9}\u{6c34}", "ab", "c"]
        );
        assert_eq!(wrap_text("abc", 0.0, |_| 1.0), ["a", "b", "c"]);
    }

    #[test]
    fn missing_glyphs_use_default_text_at_bitmap_height() {
        let font = font();
        for ch in ['V', 'J', 'Q', 'X', 'Z'] {
            let (text, font, line) = fallback_glyph(&font, ch, 16.0).unwrap();
            assert_eq!(text.0, ch.to_string());
            assert_eq!(font.font_size, bevy::text::FontSize::Px(16.0));
            assert_eq!(font.font, bevy::text::FontSource::default());
            assert_eq!(line, LineHeight::Px(16.0));
        }
        assert!(fallback_glyph(&font, 'L', 16.0).is_none());
        assert!(fallback_glyph(&font, 'l', 16.0).is_none());
        assert!(fallback_glyph(&font, ' ', 16.0).is_none());
    }

    #[test]
    fn fallback_reserves_enough_width_for_the_default_font() {
        let mut font = font();
        font.glyph.0 = 8;
        assert_eq!(glyph_width(&font, 'L'), 8.0);
        assert_eq!(glyph_width(&font, 'V'), 16.0);
    }

    #[test]
    fn dialog_keeps_its_size_when_it_fits() {
        assert_eq!(
            fit_dialog_size(
                Some(&font()),
                &["Mario", "Hello!", "A: close"],
                8.0,
                206.0,
                200.0
            ),
            8.0
        );
    }

    #[test]
    fn dialog_fits_long_pages_and_narrow_viewports() {
        let font = font();
        let page = "LIVES LEFT ".repeat(100);
        let newlines = "LIVES\n".repeat(100);
        for font in [Some(&font), None] {
            for page in [&page, &newlines] {
                for width in [16.0, 100.0, 206.0] {
                    let blocks = ["Mario", page.as_str(), "A: close"];
                    let size = fit_dialog_size(font, &blocks, 16.0, width, 180.0);
                    let height = text_height(font, blocks[0], size, width)
                        + text_height(font, blocks[1], size, width)
                        + text_height(font, blocks[2], size * 0.75, width);
                    assert!(height <= 180.001, "{height}");
                    assert!(size > 0.0 && size <= 16.0);
                }
            }
        }
    }
}

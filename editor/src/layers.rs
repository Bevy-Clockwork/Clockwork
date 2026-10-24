//! The layers and animated tiles of 2D areas in the viewport (`clockwork_core::layer`). Each
//! layer is a plane lying above or below the area's image in draw order; layers over sprites lie
//! above them. Frames swap textures as time goes on, and while playing, layers move with the
//! camera by what their scroll leaves out (the Workshop shows them unscrolled).

use bevy::{
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    math::Affine2,
    prelude::*,
};
use clockwork_core::layer::{FrameTiming, Frames, Layer, Layers};

use crate::{
    state::EditorState,
    workshop::{OrbitCamera, to_bevy_image},
};

/// How far apart layers lie, in meters: layers in front of the area stay under the surface
/// overlays (0.01).
const STEP: f32 = 0.002;
/// Where layers over sprites start (sprites lie at 0.03, what they hold a little above).
const OVER_SPRITES: f32 = 0.05;
/// How far above the area's image its patches lie: under the next layer in draw order.
const PATCH: f32 = STEP / 2.0;

/// Textures that take turns, and when.
#[derive(Clone)]
pub struct BuiltFrames {
    pub textures: Vec<Handle<Image>>,
    pub timing: FrameTiming,
}

/// A layer's textures and how it is drawn, built once per output.
#[derive(Clone)]
pub struct BuiltLayer {
    pub still: Handle<Image>,
    pub frames: Option<BuiltFrames>,
    /// The image's size in meters.
    pub size: Vec2,
    pub scroll: Vec2,
    pub wrap: bool,
    /// How far above the area's image it lies.
    pub height: f32,
}

/// A part of an area that animates on its own: its frames and where it lies.
#[derive(Clone)]
pub struct BuiltPatch {
    pub frames: BuiltFrames,
    /// Its size, and its top left corner from the area's, in meters.
    pub size: Vec2,
    pub at: Vec2,
}

/// An area's animation (its still image is the node's own texture), layers back to front, and
/// patches.
#[derive(Clone)]
pub struct BuiltLayers {
    pub frames: Option<BuiltFrames>,
    pub layers: Vec<BuiltLayer>,
    pub patches: Vec<BuiltPatch>,
}

/// Uploads an area's layer and frame images, `meters_per_pixel` to the meter.
pub fn build(layers: &Layers, meters_per_pixel: f32, images: &mut Assets<Image>) -> BuiltLayers {
    let area_frames = upload_frames(&layers.frames, false, images);
    let heights = heights(layers);
    let built = (layers.layers.iter().zip(heights))
        .map(|(layer, height): (&Layer, f32)| BuiltLayer {
            still: upload(&layer.image, layer.wrap, images),
            frames: upload_frames(&layer.frames, layer.wrap, images),
            size: Vec2::new(layer.image.width as f32, layer.image.height as f32) * meters_per_pixel,
            scroll: Vec2::new(layer.scroll.0, layer.scroll.1),
            wrap: layer.wrap,
            height,
        })
        .collect();
    let patches = layers
        .patches
        .iter()
        .filter_map(|patch| {
            let frames = upload_frames(&Some(patch.frames.clone()), false, images)?;
            let (w, h) = patch.size();
            Some(BuiltPatch {
                frames,
                size: Vec2::new(w as f32, h as f32) * meters_per_pixel,
                at: Vec2::new(patch.at.0 as f32, patch.at.1 as f32) * meters_per_pixel,
            })
        })
        .collect();
    BuiltLayers {
        frames: area_frames,
        layers: built,
        patches,
    }
}

/// A texture of the image; a wrapping one repeats.
fn upload(
    image: &clockwork_core::image::RgbaImage,
    wrap: bool,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let mut texture = to_bevy_image(image);
    if wrap {
        texture.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::nearest()
        });
    }
    images.add(texture)
}

fn upload_frames(
    frames: &Option<Frames>,
    wrap: bool,
    images: &mut Assets<Image>,
) -> Option<BuiltFrames> {
    frames.as_ref().map(|f| BuiltFrames {
        textures: f.images.iter().map(|i| upload(i, wrap, images)).collect(),
        timing: f.timing(),
    })
}

/// How far above the area's image each layer lies: a step apart in draw order, layers over
/// sprites above the sprites.
fn heights(layers: &Layers) -> Vec<f32> {
    let order = layers.draw_order();
    let area = order.iter().position(Option::is_none).unwrap_or(0) as f32;
    let mut heights = vec![0.0; layers.layers.len()];
    let mut over = 0.0;
    for (rank, which) in order.iter().enumerate() {
        let Some(i) = *which else { continue };
        heights[i] = if layers.layers[i].over_sprites {
            over += 1.0;
            OVER_SPRITES + over * STEP
        } else {
            (rank as f32 - area) * STEP
        };
    }
    heights
}

/// A plane showing a layer of an area (or the area's own image, when it is animated).
#[derive(Component)]
pub struct AreaLayer {
    frames: Option<BuiltFrames>,
    scroll: Vec2,
    wrap: bool,
    /// The layer's size and the area's, in meters.
    size: Vec2,
    area: Vec2,
    height: f32,
    material: Handle<StandardMaterial>,
    shown: Option<usize>,
    shift: Vec2,
    /// Where its top left corner is from the area's, in meters (a patch's place).
    offset: Vec2,
}

/// Marks the planes of layers (not the area's own image), which clicks go through.
#[derive(Component)]
pub struct LayerPlane;

/// Animates the plane of an area's own image (`plane`, drawn with `material`), if it has frames.
pub fn animate_area(
    commands: &mut Commands,
    built: &BuiltLayers,
    area: Vec2,
    plane: Entity,
    material: Handle<StandardMaterial>,
) {
    if let Some(frames) = &built.frames {
        commands.entity(plane).insert(AreaLayer {
            frames: Some(frames.clone()),
            scroll: Vec2::ONE,
            wrap: false,
            size: area,
            area,
            height: 0.0,
            material,
            shown: None,
            shift: Vec2::ZERO,
            offset: Vec2::ZERO,
        });
    }
}

/// Adds the planes of an area's layers under `parent`, the area's node; `area` is its image's
/// size in meters.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    built: &BuiltLayers,
    area: Vec2,
    parent: Entity,
) {
    for layer in &built.layers {
        // A wrapping layer covers the whole area and repeats its texture across it.
        let plane = if layer.wrap { area } else { layer.size };
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(layer.still.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Mask(0.5),
            uv_transform: wrap_uv(layer.wrap, area, layer.size, Vec2::ZERO),
            ..default()
        });
        commands.spawn((
            AreaLayer {
                frames: layer.frames.clone(),
                scroll: layer.scroll,
                wrap: layer.wrap,
                size: layer.size,
                area,
                height: layer.height,
                material: material.clone(),
                shown: None,
                shift: Vec2::ZERO,
                offset: Vec2::ZERO,
            },
            LayerPlane,
            Mesh3d(meshes.add(Plane3d::default().mesh().size(plane.x, plane.y))),
            MeshMaterial3d(material),
            Transform::from_translation(place(
                layer.wrap,
                area,
                layer.size,
                Vec2::ZERO,
                layer.height,
            )),
            ChildOf(parent),
        ));
    }
    // Patches: a plane the size of each, at its place just over the area, swapping its frames.
    for patch in &built.patches {
        let material = materials.add(StandardMaterial {
            base_color_texture: patch.frames.textures.first().cloned(),
            unlit: true,
            alpha_mode: AlphaMode::Mask(0.5),
            ..default()
        });
        commands.spawn((
            AreaLayer {
                frames: Some(patch.frames.clone()),
                scroll: Vec2::ONE,
                wrap: false,
                size: patch.size,
                area,
                height: PATCH,
                material: material.clone(),
                shown: None,
                shift: Vec2::ZERO,
                offset: patch.at,
            },
            LayerPlane,
            Mesh3d(meshes.add(Plane3d::default().mesh().size(patch.size.x, patch.size.y))),
            MeshMaterial3d(material),
            Transform::from_translation(place(false, area, patch.size, patch.at, PATCH)),
            ChildOf(parent),
        ));
    }
}

/// Where a layer's plane goes in its area's space when moved by `shift` (meters, east and
/// south): its top left corner at the area's, plus the shift. A wrapping layer stays put (its
/// texture moves instead).
fn place(wrap: bool, area: Vec2, size: Vec2, shift: Vec2, height: f32) -> Vec3 {
    if wrap {
        return Vec3::new(0.0, height, 0.0);
    }
    let center = -area / 2.0 + size / 2.0 + shift;
    Vec3::new(center.x, height, center.y)
}

/// The texture coordinates of a wrapping layer moved by `shift`: the image repeats across the
/// area, starting `shift` from its top left corner.
fn wrap_uv(wrap: bool, area: Vec2, size: Vec2, shift: Vec2) -> Affine2 {
    if !wrap || size.x <= 0.0 || size.y <= 0.0 {
        return Affine2::IDENTITY;
    }
    Affine2::from_scale_angle_translation(area / size, 0.0, -shift / size)
}

/// Shows each layer's current frame, and while playing moves layers with the camera.
pub fn layers_system(
    state: Res<EditorState>,
    time: Res<Time>,
    orbit: Res<OrbitCamera>,
    parents: Query<&GlobalTransform>,
    mut planes: Query<(&mut AreaLayer, &mut Transform, &ChildOf)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let seconds = time.elapsed_secs();
    let playing = state.workshop.playing.is_some();
    for (mut layer, mut transform, parent) in &mut planes {
        // Where the view's centre is from the area's top left corner, in meters.
        let view = match parents.get(parent.parent()) {
            Ok(at) if playing && layer.scroll != Vec2::ONE => {
                let local = at.affine().inverse().transform_point3(orbit.focus);
                Vec2::new(local.x, local.z) + layer.area / 2.0
            }
            _ => Vec2::ZERO,
        };
        let shift = (Vec2::ONE - layer.scroll) * view;
        let shown = (layer.frames.as_ref()).map(|f| f.timing.index_at(seconds));
        if shift == layer.shift && shown == layer.shown {
            continue;
        }
        if shift != layer.shift {
            let at = shift + layer.offset;
            transform.translation = place(layer.wrap, layer.area, layer.size, at, layer.height);
        }
        let texture = shown
            .zip(layer.frames.as_ref())
            .and_then(|(i, f)| f.textures.get(i).cloned());
        let uv = wrap_uv(layer.wrap, layer.area, layer.size, shift);
        if let Some(mut material) = materials.get_mut(&layer.material) {
            if texture.is_some() && shown != layer.shown {
                material.base_color_texture = texture;
            }
            if layer.wrap && shift != layer.shift {
                material.uv_transform = uv;
            }
        }
        layer.shift = shift;
        layer.shown = shown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::image::RgbaImage;

    fn layer(order: i32, over_sprites: bool) -> Layer {
        Layer {
            image: RgbaImage::new(1, 1),
            order,
            scroll: (1.0, 1.0),
            wrap: false,
            over_sprites,
            frames: None,
        }
    }

    #[test]
    fn layers_lie_in_draw_order_with_foregrounds_above_sprites() {
        let layers = Layers {
            frames: None,
            patches: Vec::new(),
            layers: vec![
                layer(-2, false),
                layer(3, true),
                layer(1, false),
                layer(-1, false),
            ],
        };
        let heights = heights(&layers);
        assert!(heights[0] < heights[3] && heights[3] < 0.0);
        assert!(heights[2] > 0.0 && heights[2] < 0.01);
        assert!(heights[1] > 0.03);
    }

    #[test]
    fn layers_line_up_with_the_area_s_top_left_corner_and_move_by_their_shift() {
        let area = Vec2::new(10.0, 6.0);
        let size = Vec2::new(4.0, 2.0);
        assert_eq!(
            place(false, area, size, Vec2::ZERO, 0.1),
            Vec3::new(-3.0, 0.1, -2.0)
        );
        assert_eq!(
            place(false, area, size, Vec2::new(1.0, 0.5), 0.0),
            Vec3::new(-2.0, 0.0, -1.5)
        );
        // A wrapping layer repeats 2.5 times across, and its texture moves instead.
        let uv = wrap_uv(true, area, size, Vec2::new(1.0, 0.0));
        assert_eq!(uv.transform_point2(Vec2::ZERO), Vec2::new(-0.25, 0.0));
        assert_eq!(uv.transform_point2(Vec2::ONE), Vec2::new(2.25, 3.0));
        assert_eq!(place(true, area, size, Vec2::ONE, 0.0), Vec3::ZERO);
        // A patch lies at its place from the area's top left corner, just above the area.
        assert_eq!(
            place(false, area, Vec2::new(1.0, 1.0), Vec2::new(2.0, 3.0), PATCH),
            Vec3::new(-2.5, PATCH, 0.5)
        );
        const { assert!(PATCH > 0.0 && PATCH < STEP) };
    }
}

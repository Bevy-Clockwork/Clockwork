//! A title screen's logo when it is a model (Super Mario 64's is): drawn by a camera of its own
//! into a picture, which the title screen shows like an image logo. The model is spawned only
//! while the title is up, on a render layer the scene camera doesn't see.

use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::render_resource::TextureFormat,
};
use clockwork_core::{asset::OutputRef, flow::ScreenView};
use clockwork_runtime::hud::ShownScreen;
use clockwork_runtime::retro_material::RetroMaterial;

use crate::{models, scene::ROOT, state::EditorState, workshop::Extractions};

/// The render layer only the logo's camera sees.
const LAYER: usize = 7;
/// The picture's width in pixels; its height follows the logo's shape.
const WIDTH: u32 = 640;

/// The logo model shown, its camera and picture.
#[derive(Resource, Default)]
pub struct TitleLogo {
    shown: Option<String>,
    root: Option<Entity>,
    camera: Option<Entity>,
}

/// Marks the logo's own entities.
#[derive(Component)]
pub struct LogoRoot;

#[allow(clippy::too_many_arguments)]
pub fn title_logo_system(
    mut commands: Commands,
    mut logo: ResMut<TitleLogo>,
    mut shown: ResMut<ShownScreen>,
    mut state: ResMut<EditorState>,
    mut extractions: ResMut<Extractions>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RetroMaterial>>,
    unlayered: Query<Entity, (With<Mesh3d>, Without<RenderLayers>)>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<LogoRoot>>,
) {
    let (wanted, game_camera) = match &shown.view {
        Some(ScreenView::Title {
            logo: Some(name),
            logo_camera,
            ..
        }) => (Some(name.clone()), *logo_camera),
        _ => (None, None),
    };
    if wanted != logo.shown {
        for entity in [logo.root.take(), logo.camera.take()].into_iter().flatten() {
            commands.entity(entity).despawn();
        }
        shown.model_logo_image = None;
        logo.shown = None;
    }
    let Some(name) = wanted else {
        return;
    };
    if logo.root.is_some() {
        // The model's meshes, spawned under the root, join its layer once they exist.
        for mesh in &unlayered {
            if parents.iter_ancestors(mesh).any(|a| roots.contains(a)) {
                commands.entity(mesh).insert(RenderLayers::layer(LAYER));
            }
        }
        return;
    }
    // An image logo needs none of this; a model one waits for its plugin's extraction.
    let Some(at) = OutputRef::parse(&name) else {
        return;
    };
    extractions.ensure(&mut state, &at.plugin);
    let Some(found) = extractions
        .get(&at.plugin)
        .and_then(|x| x.model(&at.output))
    else {
        return;
    };
    let (model, meters) = (&found.model, found.meters_per_unit);
    let Some((lo, hi)) = model.rest_bounds() else {
        return;
    };
    let (lo, hi) = (Vec3::from(lo) * meters, Vec3::from(hi) * meters);
    let size = (hi - lo).max(Vec3::splat(0.01));
    // The game's own camera sees the whole 4:3 screen from where it stood. Without one, the
    // picture is the logo's shape and the camera stands straight on, far enough back that the
    // logo's front fits.
    let (center, height, fov, eye) = match game_camera {
        Some(camera) => (
            Vec3::ZERO,
            WIDTH * 3 / 4,
            camera.fov.to_radians(),
            Vec3::from(camera.from) * meters,
        ),
        None => {
            let fov = std::f32::consts::FRAC_PI_4;
            let distance = (size.y / 2.0) / (fov / 2.0).tan() + size.z / 2.0;
            (
                (lo + hi) / 2.0,
                ((WIDTH as f32 * size.y / size.x).round() as u32).clamp(32, WIDTH),
                fov,
                Vec3::new(0.0, 0.0, distance * 1.05),
            )
        }
    };
    let image = images.add(Image::new_target_texture(
        WIDTH,
        height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));

    let built = models::build(model, &mut images, &mut meshes, &mut materials);
    let root = commands
        .spawn((
            LogoRoot,
            Transform::from_translation(-center),
            Visibility::Visible,
            RenderLayers::layer(LAYER),
        ))
        .id();
    models::spawn(
        &mut commands,
        &built,
        model,
        meters,
        ROOT,
        &at.plugin,
        &at.output,
        None,
        root,
    );
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::from(image.clone()),
            Projection::Perspective(PerspectiveProjection {
                fov,
                aspect_ratio: WIDTH as f32 / height as f32,
                ..default()
            }),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(LAYER),
        ))
        .id();
    logo.root = Some(root);
    logo.camera = Some(camera);
    logo.shown = Some(name);
    shown.model_logo_image = Some((image, Vec2::new(WIDTH as f32, height as f32)));
}

mod asset_edit;
mod asset_ui;
mod boxart;
mod browser;
mod claude;
mod compose_graph;
mod content_check;
mod control;
mod controls;
mod covers;
mod decomps;
mod entities;
mod export;
mod fidelity;
mod fighting;
mod flow_play;
mod game_sound;
mod game_ui;
mod graph_view;
mod kit;
mod manipulate;
mod models;
mod overlays;
mod palette;
mod play;
mod play_kit;
mod player;
mod plugin_editor;
mod recipe_graph;
mod saves;
mod scene;
mod settings;
mod sprites;
mod state;
mod surfaces;
mod terrains;
mod theme;
mod title_logo;
mod ui;
mod workshop;
mod workshop_ui;

use clockwork_library::store;

use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use clockwork_runtime::ClockworkRuntimePlugin;

use state::EditorState;

/// The camera that renders the scene into the central viewport.
#[derive(Component)]
pub(crate) struct ViewportCamera;

fn main() {
    // An exported game: play it instead of opening the Workshop.
    let player = player::detect().map(|dir| match player::prepare(&dir) {
        Ok(info) => (dir, info),
        Err(e) => {
            eprintln!("Can't play this game: {e}");
            std::process::exit(1);
        }
    });
    let settings = settings::Settings::load();
    let mut state = EditorState::load();
    state.workshop.show_surfaces = settings.show_surfaces;
    let title = player
        .as_ref()
        .map_or("Clockwork".to_string(), |(_, info)| info.title.clone());
    if let Some((dir, info)) = player {
        if let Some(scene) = scene::Scene::load_from(&dir.join(export::SCENE_FILE)) {
            state.workshop.scene = scene;
        }
        state.workshop.show_surfaces = false;
        state.player = Some(player::PlayerMode::new(info));
    }

    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(0x15, 0x11, 0x0D)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: title.clone(),
                resolution: (1600u32, 900u32).into(),
                ..default()
            }),
            // Closing asks first while plugin edits aren't saved (ui::close_system).
            close_when_requested: false,
            ..default()
        }))
        .add_plugins((EguiPlugin::default(), ClockworkRuntimePlugin))
        .insert_resource(scene::History::new(&state.workshop.scene))
        .insert_resource(control::Control::from_env())
        .init_resource::<play::PlaySession>()
        .init_resource::<play::ScriptedInput>()
        .init_resource::<controls::ControlsUi>()
        .init_resource::<terrains::SceneTerrains>()
        .init_resource::<manipulate::Manipulation>()
        .insert_resource(state)
        .insert_resource(settings)
        .init_resource::<covers::Covers>()
        .init_resource::<browser::Browser>()
        .init_resource::<workshop::SceneView>()
        .init_resource::<sprites::SpriteCache>()
        .init_resource::<workshop::Extractions>()
        .init_resource::<workshop::OrbitCamera>()
        .init_resource::<flow_play::FlowState>()
        .init_resource::<title_logo::TitleLogo>()
        .add_systems(
            PreUpdate,
            (controls::sample_system, game_ui::prepare_input_system)
                .chain()
                .after(bevy::input::InputSystems)
                .before(clockwork_runtime::input::InputSystems),
        )
        .add_systems(
            Update,
            (
                control::control_system,
                workshop::extraction_system,
                terrains::refresh_system,
                workshop::place_system,
                asset_ui::relink_system,
                workshop::scene_system,
                plugin_editor::extract_system,
                game_ui::hud_bridge_system,
                // Play, then its screens (title, files, acts) over it.
                (
                    play::play_system,
                    flow_play::flow_system,
                    title_logo::title_logo_system,
                )
                    .chain(),
                game_sound::sound_system,
                // Pose the bones, then turn billboards to the camera.
                (models::animate_system, models::face_camera_system).chain(),
                sprites::billboard_system,
                sprites::animate_system,
                sprites::projectile_system,
                sprites::hand_system,
                manipulate::manipulate_system,
                workshop::camera_system,
                workshop::surface_system,
                fighting::gizmo_system,
                overlays::overlay_system,
            )
                .chain(),
        )
        // The HUD's menu answers this frame's buttons before Play mode takes its requests.
        .configure_sets(
            Update,
            clockwork_runtime::hud::HudSystems.before(game_ui::hud_bridge_system),
        )
        .add_systems(Startup, setup)
        .add_systems(Update, ui::close_system)
        .add_systems(
            EguiPrimaryContextPass,
            (
                boxart::boxart_system,
                browser::browser_system,
                ui::editor_ui,
            )
                .chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut egui_settings: ResMut<EguiGlobalSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The UI gets its own camera so it covers the whole window while the scene camera only
    // renders into the viewport left between the panels.
    egui_settings.auto_create_primary_context = false;
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 1,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));

    commands.spawn((
        ViewportCamera,
        clockwork_runtime::hud::HudTarget,
        Camera3d::default(),
        // A game's colors reach the screen as they are: no tonemapping.
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_xyz(6.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Placeholder scene until plugin output is shown.
    commands.spawn((
        workshop::Placeholder,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(16.0, 16.0))),
        MeshMaterial3d(materials.add(Color::srgb_u8(0x2A, 0x23, 0x1B))),
    ));
    commands.spawn((
        workshop::Placeholder,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb_u8(0xC1, 0x9A, 0x4B),
            metallic: 0.9,
            perceptual_roughness: 0.35,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
}

//! Sprites in the viewport: each frame becomes a texture, and a scene node shows one quad that
//! swaps frames as its animation plays. It lies on the ground (like the area images, seen from
//! above), or, in a 3D world, stands upright facing the camera (a 2.5D billboard).
//!
//! While a character uses an item, what it holds (another sprite, frame by frame with it or
//! playing its own animation) is drawn with it, and projectiles with a sprite are drawn as sprites.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use clockwork_core::{
    action::AttachmentFrames,
    character2d::{AnimationRef, Direction4},
    sprite::Sprite,
};

use crate::{
    play::PlaySession,
    scene::NodeId,
    state::EditorState,
    terrains::SceneTerrains,
    workshop::{Extractions, OrbitCamera},
};

/// Sprites lie this far above the ground planes, which sit at 0 (surface overlays at 0.01).
const HEIGHT: f32 = 0.03;
/// How far in front of (or behind) its character a held sprite is drawn, so they don't fight.
const LAYER: f32 = 0.004;

/// One frame of a sprite, ready to show.
#[derive(Clone)]
pub struct BuiltFrame {
    pub texture: Handle<Image>,
    /// Size in meters, east by south.
    pub size: Vec2,
    /// The sprite's origin in meters from the frame's top left.
    pub origin: Vec2,
}

/// A sprite's frames, built once and shared by every node that shows it.
#[derive(Clone)]
pub struct BuiltSprite {
    pub frames: Arc<[BuiltFrame]>,
}

pub fn build(sprite: &Sprite, pixels_per_meter: f32, images: &mut Assets<Image>) -> BuiltSprite {
    let scale = 1.0 / pixels_per_meter.max(f32::EPSILON);
    let frames = sprite
        .frames
        .iter()
        .map(|frame| {
            let mut texture = Image::new(
                Extent3d {
                    width: frame.image.width,
                    height: frame.image.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                frame.image.pixels.clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            // Keep pixel art crisp.
            texture.sampler = ImageSampler::nearest();
            BuiltFrame {
                texture: images.add(texture),
                size: Vec2::new(frame.image.width as f32, frame.image.height as f32) * scale,
                origin: Vec2::new(frame.origin[0] as f32, frame.origin[1] as f32) * scale,
            }
        })
        .collect();
    BuiltSprite { frames }
}

/// Built sprites by (plugin, output) for held items and projectiles, rebuilt when plugins are
/// extracted again.
#[derive(Resource, Default)]
pub struct SpriteCache {
    built: HashMap<(String, String), BuiltSprite>,
    revision: u64,
}

impl SpriteCache {
    /// The built sprite, building it on first use; `None` if the plugin has no such sprite.
    pub fn get(
        &mut self,
        extractions: &Extractions,
        plugin: &str,
        output: &str,
        images: &mut Assets<Image>,
    ) -> Option<BuiltSprite> {
        if self.revision != extractions.revision() {
            self.revision = extractions.revision();
            self.built.clear();
        }
        let key = (plugin.to_string(), output.to_string());
        if let Some(built) = self.built.get(&key) {
            return Some(built.clone());
        }
        let extracted = extractions.get(plugin)?.sprite(output)?;
        let built = build(&extracted.sprite, extracted.pixels_per_meter, images);
        self.built.insert(key, built.clone());
        Some(built)
    }
}

/// Something a character holds now, as Play mode sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    /// A sprite output of the character's plugin.
    pub sprite: String,
    pub frames: AttachmentFrames,
    pub behind: bool,
    pub facing: Direction4,
    /// Seconds since the action started, for attachments with their own animation.
    pub seconds: f32,
    /// The frame shown by a 3D character, which it can't follow frame by frame.
    pub still: Option<usize>,
}

/// A sprite shown by a scene node, and what it plays.
#[derive(Component)]
pub struct SpriteRig {
    pub node: NodeId,
    pub plugin: String,
    pub output: String,
    pub frames: Arc<[BuiltFrame]>,
    /// The animation chosen in the Inspector.
    pub animation: Option<String>,
    /// Play mode's choice, which wins: animation, mirrored, and seconds since it started.
    pub playback: Option<(String, bool, f32)>,
    pub time: f32,
    /// Standing upright facing a camera that looks along this yaw (radians), instead of lying
    /// on the ground.
    pub upright: Option<f32>,
    /// What the character holds now (Play mode sets it).
    pub held: Vec<Held>,
    /// The quads that show `held`.
    held_quads: Vec<Entity>,
    /// The frame shown now (frame, mirrored horizontally, vertically), to change as little as
    /// possible.
    shown: Option<(usize, bool, bool)>,
}

/// A quad showing something a character holds.
#[derive(Component)]
pub struct HeldQuad;

type HeldQuadParts = (
    &'static mut Transform,
    &'static MeshMaterial3d<StandardMaterial>,
    &'static mut Visibility,
);

/// A quad showing a projectile (by index in the played character's projectiles).
#[derive(Component)]
pub struct ProjectileQuad(usize);

fn quad_material() -> StandardMaterial {
    StandardMaterial {
        unlit: true,
        alpha_mode: AlphaMode::Mask(0.5),
        // Mirroring flips the winding.
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

/// Spawns the sprite's quad under `parent`.
#[allow(clippy::too_many_arguments)] // A sprite needs all of these to be put together.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    built: &BuiltSprite,
    node: NodeId,
    plugin: &str,
    output: &str,
    animation: Option<String>,
    parent: Entity,
) {
    commands.spawn((
        SpriteRig {
            node,
            plugin: plugin.to_string(),
            output: output.to_string(),
            frames: built.frames.clone(),
            animation,
            playback: None,
            time: 0.0,
            upright: None,
            held: Vec::new(),
            held_quads: Vec::new(),
            shown: None,
        },
        Mesh3d(meshes.add(Plane3d::default().mesh().size(1.0, 1.0))),
        MeshMaterial3d(materials.add(quad_material())),
        Transform::from_xyz(0.0, HEIGHT, 0.0),
        Visibility::Inherited,
        ChildOf(parent),
    ));
}

/// Stands sprites up that are on the ground of a 3D world, turned to the camera; the played one
/// is up to Play mode.
pub fn billboard_system(
    state: Res<EditorState>,
    terrains: Res<SceneTerrains>,
    orbit: Res<OrbitCamera>,
    positions: Query<&GlobalTransform, Without<SpriteRig>>,
    mut rigs: Query<(&mut SpriteRig, &ChildOf)>,
) {
    for (mut rig, parent) in &mut rigs {
        if state.workshop.playing == Some(rig.node) {
            continue;
        }
        let upright = positions
            .get(parent.parent())
            .ok()
            .filter(|at| terrains.floor_below(at.translation()).is_some())
            .map(|_| orbit.yaw);
        if rig.upright != upright {
            rig.upright = upright;
        }
    }
}

/// Where a frame's quad goes relative to the sprite's origin: lying on the ground, or upright
/// turned to a camera at `upright` yaw. `layer` moves it towards the viewer (positive) or away.
fn place(
    frame: &BuiltFrame,
    flip_x: bool,
    flip_y: bool,
    upright: Option<f32>,
    layer: f32,
) -> Transform {
    // The quad's center relative to the origin, mirrored about the origin when flipped; a
    // negative scale mirrors the texture.
    let center = frame.size / 2.0 - frame.origin;
    let sign = |flip| if flip { -1.0 } else { 1.0 };
    let (sx, sy) = (sign(flip_x), sign(flip_y));
    let scale = Vec3::new(frame.size.x * sx, 1.0, frame.size.y * sy);
    match upright {
        None => Transform {
            translation: Vec3::new(center.x * sx, HEIGHT + layer, center.y * sy),
            rotation: Quat::IDENTITY,
            scale,
        },
        Some(yaw) => {
            // Stood up: the image's down is down and its normal faces +z, then turned about the
            // vertical axis so that it faces the camera, which sits at +yaw.
            let turn = Quat::from_rotation_y(yaw);
            Transform {
                translation: turn * Vec3::new(center.x * sx, -center.y * sy, layer),
                rotation: turn * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                scale,
            }
        }
    }
}

/// The frame an animation reference shows `seconds` in, with its mirroring.
fn frame_of(
    sprite: &Sprite,
    reference: &AnimationRef,
    seconds: f32,
) -> Option<(usize, bool, bool)> {
    let (step, _) = sprite.animation(&reference.name)?.step_at(seconds)?;
    Some((
        step.frame,
        step.flip_x != reference.flip_x,
        step.flip_y != reference.flip_y,
    ))
}

/// Advances every sprite and shows its current frame, with what it holds.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn animate_system(
    mut commands: Commands,
    time: Res<Time>,
    extractions: Res<Extractions>,
    mut cache: ResMut<SpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rigs: Query<(
        &mut SpriteRig,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
        &ChildOf,
    )>,
    mut quads: Query<HeldQuadParts, (With<HeldQuad>, Without<SpriteRig>)>,
) {
    for (mut rig, mut transform, material, parent) in &mut rigs {
        rig.time += time.delta_secs();
        let Some(sprite) = extractions
            .get(&rig.plugin)
            .and_then(|x| x.sprite(&rig.output))
            .map(|s| &s.sprite)
        else {
            continue;
        };
        let (name, extra_flip, seconds) = match &rig.playback {
            Some((name, flip, seconds)) => (Some(name.as_str()), *flip, *seconds),
            None => (rig.animation.as_deref(), false, rig.time),
        };
        let animation = name
            .and_then(|n| sprite.animation(n))
            .or_else(|| sprite.animations.first());
        let (frame, flip_x, flip_y) = match animation.and_then(|a| a.step_at(seconds)) {
            Some((step, _)) => (step.frame, step.flip_x != extra_flip, step.flip_y),
            None => (0, extra_flip, false),
        };
        let Some(built) = rig.frames.get(frame).cloned() else {
            continue;
        };
        if rig.shown != Some((frame, flip_x, flip_y)) {
            rig.shown = Some((frame, flip_x, flip_y));
            if let Some(mut material) = materials.get_mut(&material.0) {
                material.base_color_texture = Some(built.texture.clone());
            }
        }
        *transform = place(&built, flip_x, flip_y, rig.upright, 0.0);

        // What it holds: one quad each, next to the character's quad.
        let held = rig.held.clone();
        while rig.held_quads.len() < held.len() {
            let quad = commands
                .spawn((
                    HeldQuad,
                    Mesh3d(meshes.add(Plane3d::default().mesh().size(1.0, 1.0))),
                    MeshMaterial3d(materials.add(quad_material())),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(parent.parent()),
                ))
                .id();
            rig.held_quads.push(quad);
        }
        for (i, &quad) in rig.held_quads.iter().enumerate() {
            let Ok((mut quad_transform, quad_material, mut visibility)) = quads.get_mut(quad)
            else {
                continue;
            };
            let shown = held.get(i).and_then(|h| {
                let other = cache.get(&extractions, &rig.plugin, &h.sprite, &mut images)?;
                let source = &extractions.get(&rig.plugin)?.sprite(&h.sprite)?.sprite;
                let (f, fx, fy) = match &h.frames {
                    AttachmentFrames::FollowFrame { .. } => {
                        (h.frames.follow(frame, other.frames.len())?, flip_x, flip_y)
                    }
                    AttachmentFrames::Animation(directional) => {
                        frame_of(source, directional.get(h.facing), h.seconds)?
                    }
                };
                Some((other.frames.get(f)?.clone(), fx, fy, h.behind))
            });
            match shown {
                Some((built, fx, fy, behind)) => {
                    if let Some(mut m) = materials.get_mut(&quad_material.0) {
                        m.base_color_texture = Some(built.texture.clone());
                    }
                    let layer = if behind { -LAYER } else { LAYER };
                    *quad_transform = place(&built, fx, fy, rig.upright, layer);
                    *visibility = Visibility::Inherited;
                }
                None => *visibility = Visibility::Hidden,
            }
        }
    }
}

/// Draws the played character's projectiles that have a sprite, lying on the ground or (in a
/// 3D world) upright facing the camera. Others are drawn as shapes by the fighting gizmos.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn projectile_system(
    mut commands: Commands,
    session: Res<PlaySession>,
    extractions: Res<Extractions>,
    orbit: Res<OrbitCamera>,
    mut cache: ResMut<SpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut quads: Query<(
        Entity,
        &ProjectileQuad,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    let fighter = session.fighter();
    let upright = session.in_3d().then_some(orbit.yaw);
    let mut shown: Vec<(BuiltFrame, bool, bool, Vec3)> = Vec::new();
    if let Some(fighter) = fighter
        && let Some(plugin) = fighter.plugin.as_deref()
    {
        for p in fighter.projectiles.iter() {
            let Some(look) = fighter.look_of(&p.id) else {
                continue;
            };
            let Some(source) = extractions
                .get(plugin)
                .and_then(|x| x.sprite(&look.sprite))
                .map(|s| &s.sprite)
            else {
                continue;
            };
            let dir = p.dir();
            // North is up the map (-z).
            let facing = Direction4::from_input(dir.x, -dir.y, Direction4::South);
            let picked = match (&look.burst, p.is_bursting()) {
                (Some(burst), true) => frame_of(
                    source,
                    &AnimationRef {
                        name: burst.clone(),
                        flip_x: false,
                        flip_y: false,
                    },
                    p.age,
                ),
                _ => frame_of(source, look.animation.get(facing), p.age),
            };
            let Some((frame, fx, fy)) = picked else {
                continue;
            };
            let Some(built) = cache
                .get(&extractions, plugin, &look.sprite, &mut images)
                .and_then(|b| b.frames.get(frame).cloned())
            else {
                continue;
            };
            let at = Vec3::new(p.position.x, p.height + look.height, p.position.y);
            shown.push((built, fx, fy, at));
        }
    }

    let mut existing: Vec<_> = quads.iter_mut().collect();
    existing.sort_by_key(|(_, q, ..)| q.0);
    for (i, (built, fx, fy, at)) in shown.iter().enumerate() {
        let mut transform = place(built, *fx, *fy, upright, 0.0);
        transform.translation += *at;
        if let Some((_, _, quad_transform, material, visibility)) = existing.get_mut(i) {
            if let Some(mut m) = materials.get_mut(&material.0) {
                m.base_color_texture = Some(built.texture.clone());
            }
            **quad_transform = transform;
            **visibility = Visibility::Inherited;
        } else {
            let mut material = quad_material();
            material.base_color_texture = Some(built.texture.clone());
            commands.spawn((
                ProjectileQuad(i),
                Mesh3d(meshes.add(Plane3d::default().mesh().size(1.0, 1.0))),
                MeshMaterial3d(materials.add(material)),
                transform,
                Visibility::Inherited,
            ));
        }
    }
    for (_, _, _, _, visibility) in existing.iter_mut().skip(shown.len()) {
        **visibility = Visibility::Hidden;
    }
}

/// A quad showing something the played 3D character holds (by index in what it holds).
#[derive(Component)]
pub struct HandQuad(usize);

/// Draws what the played 3D character holds: each attachment's still frame, upright and facing
/// the camera, centered on the hand bone (or the chest when the character names no hand).
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn hand_system(
    mut commands: Commands,
    session: Res<PlaySession>,
    extractions: Res<Extractions>,
    orbit: Res<OrbitCamera>,
    mut cache: ResMut<SpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    rigs: Query<&crate::models::Rig>,
    bones: Query<&GlobalTransform>,
    mut quads: Query<(
        &HandQuad,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    let mut shown: Vec<(BuiltFrame, Vec3)> = Vec::new();
    if let Some((node, held, hand)) = session.held_at_hand()
        && let Some(plugin) = session.fighter().and_then(|f| f.plugin.clone())
        && let Some(rig) = rigs.iter().find(|r| r.node == node)
    {
        let bone = hand.and_then(|name| {
            let model = &extractions.get(&rig.plugin)?.model(&rig.output)?.model;
            let index = model.bones.iter().position(|b| b.name == name)?;
            rig.bones.get(index).copied()
        });
        let at = bone
            .and_then(|b| bones.get(b).ok())
            .map(|g| g.translation())
            .or_else(|| session.position().map(|p| p + Vec3::Y * 0.9));
        for h in &held {
            let (Some(frame), Some(at)) = (h.still, at) else {
                continue;
            };
            if let Some(built) = cache
                .get(&extractions, &plugin, &h.sprite, &mut images)
                .and_then(|b| b.frames.get(frame).cloned())
            {
                shown.push((built, at));
            }
        }
    }

    let mut existing: Vec<_> = quads.iter_mut().collect();
    existing.sort_by_key(|(q, ..)| q.0);
    let turn = Quat::from_rotation_y(orbit.yaw);
    for (i, (built, at)) in shown.iter().enumerate() {
        // Centered on the hand, a little towards the camera so the hand doesn't cover it.
        let transform = Transform {
            translation: *at + turn * Vec3::new(0.0, 0.0, 0.15),
            rotation: turn * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(built.size.x, 1.0, built.size.y),
        };
        if let Some((_, quad_transform, material, visibility)) = existing.get_mut(i) {
            if let Some(mut m) = materials.get_mut(&material.0) {
                m.base_color_texture = Some(built.texture.clone());
            }
            **quad_transform = transform;
            **visibility = Visibility::Inherited;
        } else {
            let mut material = quad_material();
            material.base_color_texture = Some(built.texture.clone());
            commands.spawn((
                HandQuad(i),
                Mesh3d(meshes.add(Plane3d::default().mesh().size(1.0, 1.0))),
                MeshMaterial3d(materials.add(material)),
                transform,
                Visibility::Inherited,
            ));
        }
    }
    for (_, _, _, visibility) in existing.iter_mut().skip(shown.len()) {
        **visibility = Visibility::Hidden;
    }
}

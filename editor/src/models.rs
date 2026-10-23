//! Models in the viewport: Bevy meshes built from extracted models, a rig of bone entities per
//! scene node, and animation playback.

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use clockwork_core::model::{Animation, Blend, Model, Wrap};
use clockwork_runtime::retro_material::RetroMaterial;

use crate::{scene::NodeId, workshop::Extractions};

/// A model's meshes and materials, built once and shared by every node that shows it.
pub struct BuiltModel {
    /// (bone, mesh, material) for each of the model's meshes.
    pub parts: Vec<(usize, Handle<Mesh>, Handle<RetroMaterial>)>,
    /// Rest-pose size in model units, for framing.
    pub size: Vec3,
}

pub fn build(
    model: &Model,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<RetroMaterial>,
) -> BuiltModel {
    let textures: Vec<Handle<Image>> = model
        .textures
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let wrap = model.texture_wrap.get(i).copied().unwrap_or_default();
            images.add(texture(t, wrap))
        })
        .collect();
    let parts = model
        .meshes
        .iter()
        .map(|m| {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
            if !m.normals.is_empty() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
            }
            if !m.uvs.is_empty() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
            }
            let colors: Vec<[f32; 4]> = m
                .colors
                .iter()
                .map(|c| LinearRgba::from(Color::srgba(c[0], c[1], c[2], c[3])).to_f32_array())
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            mesh.insert_indices(Indices::U32(m.indices.clone()));
            // Drawn as the hardware drew it (see `RetroMaterial`), not lit by the scene.
            let material = RetroMaterial::new(
                m.shading,
                m.environment_map,
                m.texture.and_then(|t| textures.get(t).cloned()),
                match m.blend {
                    Blend::Opaque => AlphaMode::Opaque,
                    Blend::Cutout => AlphaMode::Mask(0.5),
                    Blend::Transparent => AlphaMode::Blend,
                },
            );
            (m.bone, meshes.add(mesh), materials.add(material))
        })
        .collect();
    let size = model
        .rest_bounds()
        .map(|(lo, hi)| Vec3::from(hi) - Vec3::from(lo))
        .unwrap_or(Vec3::ONE);
    BuiltModel { parts, size }
}

fn texture(image: &clockwork_core::image::RgbaImage, wrap: [Wrap; 2]) -> Image {
    let mut out = Image::new(
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
    let mode = |w| match w {
        Wrap::Repeat => ImageAddressMode::Repeat,
        Wrap::Mirror => ImageAddressMode::MirrorRepeat,
        Wrap::Clamp => ImageAddressMode::ClampToEdge,
    };
    out.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: mode(wrap[0]),
        address_mode_v: mode(wrap[1]),
        ..ImageSamplerDescriptor::linear()
    });
    out
}

/// A model shown by a scene node: its bone entities and what it plays.
#[derive(Component)]
pub struct Rig {
    pub node: NodeId,
    pub plugin: String,
    pub output: String,
    pub bones: Vec<Entity>,
    pub rest: Vec<Vec3>,
    pub animation: Option<String>,
    /// Play mode's choice, which wins over `animation`, and its playback rate.
    pub playback: Option<(String, f32)>,
    pub time: f32,
    /// An extra rotation of one bone in its parent's space (Play mode's lean), applied after
    /// the animation.
    pub bend: Option<(usize, Quat)>,
    /// Whether the animation has played through once (its last frame was reached).
    pub finished: bool,
}

/// Spawns the model under `parent`: a root scaled to meters, one entity per bone, meshes on
/// their bones.
#[allow(clippy::too_many_arguments)] // A model needs all of these to be put together.
pub fn spawn(
    commands: &mut Commands,
    built: &BuiltModel,
    model: &Model,
    meters_per_unit: f32,
    node: NodeId,
    plugin: &str,
    output: &str,
    animation: Option<String>,
    parent: Entity,
) {
    let root = commands
        .spawn((
            Transform::from_scale(Vec3::splat(meters_per_unit)),
            Visibility::Inherited,
            ChildOf(parent),
        ))
        .id();
    let mut bones = Vec::with_capacity(model.bones.len());
    // Where each bone's meshes hang: the bone, or for a billboard a child that faces the camera.
    let mut holders = Vec::with_capacity(model.bones.len());
    for bone in &model.bones {
        let parent = bone
            .parent
            .and_then(|p| bones.get(p).copied())
            .unwrap_or(root);
        let entity = commands
            .spawn((
                Transform::from_translation(Vec3::from(bone.translation)),
                Visibility::Inherited,
                ChildOf(parent),
            ))
            .id();
        bones.push(entity);
        holders.push(if bone.billboard {
            commands
                .spawn((
                    Billboard,
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(entity),
                ))
                .id()
        } else {
            entity
        });
    }
    for (bone, mesh, material) in &built.parts {
        let Some(&parent) = holders.get(*bone) else {
            continue;
        };
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            ChildOf(parent),
        ));
    }
    commands.entity(root).insert(Rig {
        node,
        plugin: plugin.to_string(),
        output: output.to_string(),
        rest: model
            .bones
            .iter()
            .map(|b| Vec3::from(b.translation))
            .collect(),
        bones,
        animation,
        playback: None,
        time: 0.0,
        bend: None,
        finished: false,
    });
}

/// The animation a rig plays: the chosen one, or the model's first.
pub fn chosen<'a>(model: &'a Model, name: Option<&str>) -> Option<&'a Animation> {
    name.and_then(|n| model.animations.iter().find(|a| a.name == n))
        .or_else(|| model.animations.first())
}

/// The frame shown at `time` seconds: loops from `loop_start`, or stops on the last frame.
pub fn frame_at(animation: &Animation, time: f32) -> u32 {
    let frames = animation.frames.max(1);
    let frame = (time * animation.fps) as u32;
    match animation.loop_start {
        Some(start) if frame >= frames && start < frames => {
            start + (frame - start) % (frames - start)
        }
        _ => frame.min(frames - 1),
    }
}

/// Holds a billboard bone's meshes, turned every frame to face the camera
/// (see [`face_camera_system`]).
#[derive(Component)]
pub struct Billboard;

/// The rotation, in its parent's space, that makes a billboard face the camera the way the
/// N64's billboard matrix does: its geometry lies in the screen plane (its +Z towards the
/// viewer, its up the screen's up), whatever the bones above it do.
pub fn facing(parent: Quat, camera: Quat) -> Quat {
    parent.inverse() * camera
}

/// Turns every billboard to face the viewport camera, after animation posed the bones.
pub fn face_camera_system(
    camera: Query<&GlobalTransform, With<crate::ViewportCamera>>,
    mut billboards: Query<(&mut Transform, &ChildOf), With<Billboard>>,
    parents: Query<&GlobalTransform>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let camera = camera.compute_transform().rotation;
    for (mut transform, child_of) in &mut billboards {
        if let Ok(parent) = parents.get(child_of.parent()) {
            transform.rotation = facing(parent.compute_transform().rotation, camera);
        }
    }
}

/// Advances every rig and poses its bones.
pub fn animate_system(
    time: Res<Time>,
    extractions: Res<Extractions>,
    mut rigs: Query<&mut Rig>,
    mut transforms: Query<&mut Transform, Without<Rig>>,
) {
    for mut rig in &mut rigs {
        let rate = rig.playback.as_ref().map_or(1.0, |(_, rate)| *rate);
        rig.time += time.delta_secs() * rate;
        let Some(model) = extractions
            .get(&rig.plugin)
            .and_then(|x| x.model(&rig.output))
            .map(|m| &m.model)
        else {
            continue;
        };
        let name = rig
            .playback
            .as_ref()
            .map(|(name, _)| name.as_str())
            .or(rig.animation.as_deref());
        let Some(animation) = chosen(model, name) else {
            continue;
        };
        let frame = frame_at(animation, rig.time);
        rig.finished = rig.time * animation.fps >= animation.frames.saturating_sub(1) as f32;
        for track in &animation.tracks {
            let (Some(&entity), Some(&rest)) =
                (rig.bones.get(track.bone), rig.rest.get(track.bone))
            else {
                continue;
            };
            let Ok(mut transform) = transforms.get_mut(entity) else {
                continue;
            };
            transform.translation = track.translation(frame).map(Vec3::from).unwrap_or(rest);
            transform.rotation = track
                .rotation(frame)
                .map(Quat::from_array)
                .unwrap_or(Quat::IDENTITY);
        }
        // Only bones the animation sets each frame, or the bend would add up.
        if let Some((bone, bend)) = rig.bend
            && animation.tracks.iter().any(|t| t.bone == bone)
            && let Some(&entity) = rig.bones.get(bone)
            && let Ok(mut transform) = transforms.get_mut(entity)
        {
            transform.translation = bend * transform.translation;
            transform.rotation = bend * transform.rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(frames: u32, loop_start: Option<u32>) -> Animation {
        Animation {
            name: String::new(),
            frames,
            fps: 10.0,
            loop_start,
            tracks: Vec::new(),
        }
    }

    #[test]
    fn billboards_face_the_camera_from_any_side() {
        // Whatever way the bones above it turn, and wherever the camera stands, the billboard's
        // world rotation is the camera's: its face (+Z) towards the viewer, upright on screen.
        for parent in [
            Quat::IDENTITY,
            Quat::from_rotation_y(2.0),
            Quat::from_euler(EulerRot::XYZ, 0.3, -1.2, 0.7),
        ] {
            for camera in [
                Quat::IDENTITY,
                Quat::from_rotation_y(std::f32::consts::PI),
                Quat::from_euler(EulerRot::YXZ, 0.8, -0.5, 0.0),
            ] {
                let world = parent * facing(parent, camera);
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    let (got, want) = (world * axis, camera * axis);
                    assert!(got.distance(want) < 1e-4, "{parent:?} {camera:?} {axis:?}");
                }
            }
        }
    }

    #[test]
    fn frames_loop_or_hold() {
        assert_eq!(frame_at(&animation(10, Some(0)), 1.25), 2);
        assert_eq!(frame_at(&animation(10, Some(4)), 1.0), 4);
        assert_eq!(frame_at(&animation(10, Some(4)), 1.7), 5);
        assert_eq!(frame_at(&animation(10, None), 5.0), 9);
    }
}

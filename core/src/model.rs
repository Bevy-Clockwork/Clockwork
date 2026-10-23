//! Rigid skeletal models: the common output of 3D graphics decoders and recipes.
//!
//! A model is a tree of bones; each mesh moves with one bone. Animations turn bones over time.
//! Positions are in the model's own units; the recipe output says how many meters one unit is.

use serde::{Deserialize, Serialize};

use crate::image::RgbaImage;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub bones: Vec<Bone>,
    pub meshes: Vec<Mesh>,
    /// Textures that meshes refer to by index.
    pub textures: Vec<RgbaImage>,
    /// How each texture repeats outside 0..1, in u and v (parallel to `textures`).
    pub texture_wrap: Vec<[Wrap; 2]>,
    pub animations: Vec<Animation>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Always an earlier bone, so bones can be processed in order.
    pub parent: Option<usize>,
    /// Offset from the parent bone in its rest pose.
    pub translation: [f32; 3],
    /// Its geometry always faces the camera (a tree, a coin), turning about its own origin.
    pub billboard: bool,
}

/// How the original hardware shaded a mesh, so it can be drawn the same way.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Shading {
    /// Vertex colors are the final colors (times the texture).
    #[default]
    Unlit,
    /// Lit like the N64's RSP: `ambient` plus `diffuse` times how much the normal faces
    /// `direction` (a unit vector in model space, towards the light). Colors are sRGB, 0..1.
    Lit {
        ambient: [f32; 3],
        diffuse: [f32; 3],
        direction: [f32; 3],
    },
}

/// Triangles that share a texture and blend mode, attached to one bone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub bone: usize,
    pub positions: Vec<[f32; 3]>,
    /// Unit normals: for lit and environment-mapped meshes; empty otherwise.
    pub normals: Vec<[f32; 3]>,
    /// RGBA per vertex (sRGB, 0..1), multiplied with the texture.
    pub colors: Vec<[f32; 4]>,
    /// Texture coordinates in 0..1 (may repeat outside); empty without a texture.
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub texture: Option<usize>,
    pub blend: Blend,
    /// How it was lit; `Lit` meshes have normals.
    pub shading: Shading,
    /// Its texture coordinates come from the normals as seen by the camera (the N64's
    /// texture generation: shiny, mirror-like surfaces such as stars and metal caps).
    pub environment_map: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Opaque,
    /// Pixels below half alpha are discarded.
    Cutout,
    Transparent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wrap {
    #[default]
    Repeat,
    Mirror,
    Clamp,
}

/// Bone poses over time. Bones without a track keep their rest pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub name: String,
    pub frames: u32,
    pub fps: f32,
    /// Playback continues from this frame after the last one; `None` stops at the end.
    pub loop_start: Option<u32>,
    pub tracks: Vec<Track>,
}

/// One bone's pose per frame. Lists may be shorter than the animation (the last value holds)
/// or empty (the rest pose is used).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub bone: usize,
    /// Replaces the bone's rest translation.
    pub translations: Vec<[f32; 3]>,
    /// Quaternions (x, y, z, w) relative to the parent.
    pub rotations: Vec<[f32; 4]>,
}

impl Track {
    pub fn translation(&self, frame: u32) -> Option<[f32; 3]> {
        held(&self.translations, frame)
    }

    pub fn rotation(&self, frame: u32) -> Option<[f32; 4]> {
        held(&self.rotations, frame)
    }
}

fn held<T: Copy>(values: &[T], frame: u32) -> Option<T> {
    values
        .get((frame as usize).min(values.len().checked_sub(1)?))
        .copied()
}

impl Model {
    /// Bounds of the vertices triangles use, in rest pose with bone offsets applied (rotations
    /// ignored), for framing and previews. A display list may load vertices it never draws.
    pub fn rest_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let offsets = self.rest_offsets();
        let mut points = self.meshes.iter().flat_map(|m| {
            let o = offsets.get(m.bone).copied().unwrap_or_default();
            m.indices
                .iter()
                .filter_map(|&i| m.positions.get(i as usize))
                .map(move |p| [p[0] + o[0], p[1] + o[1], p[2] + o[2]])
        });
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        }))
    }

    /// Each bone's rest position in model space, ignoring rotations.
    pub fn rest_offsets(&self) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = Vec::with_capacity(self.bones.len());
        for bone in &self.bones {
            let base = bone
                .parent
                .and_then(|p| out.get(p).copied())
                .unwrap_or_default();
            let t = bone.translation;
            out.push([base[0] + t[0], base[1] + t[1], base[2] + t[2]]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_hold_their_last_value() {
        let track = Track {
            bone: 0,
            translations: vec![[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            rotations: vec![],
        };
        assert_eq!(track.translation(5), Some([2.0, 0.0, 0.0]));
        assert_eq!(track.rotation(0), None);
    }

    #[test]
    fn rest_offsets_follow_parents() {
        let bone = |parent, x| Bone {
            name: String::new(),
            parent,
            translation: [x, 0.0, 0.0],
            billboard: false,
        };
        let model = Model {
            bones: vec![bone(None, 1.0), bone(Some(0), 2.0)],
            meshes: vec![Mesh {
                bone: 1,
                // The second vertex is loaded but no triangle uses it: it doesn't count.
                positions: vec![[0.0, 1.0, 0.0], [50.0, 50.0, 50.0]],
                indices: vec![0, 0, 0],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(model.rest_offsets(), vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
        assert_eq!(
            model.rest_bounds(),
            Some(([3.0, 1.0, 0.0], [3.0, 1.0, 0.0]))
        );
    }
}

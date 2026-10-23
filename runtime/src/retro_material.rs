//! Drawing extracted models the way their hardware did, instead of with physically based
//! lighting: a mesh's [`Shading`] says how (vertex colors as they are, or lit like the N64's
//! RSP), and [`RetroMaterial`] does exactly that, in the colors' own (sRGB) space, untouched by
//! scene lights or tonemapping.

use bevy::{
    asset::{embedded_asset, embedded_path},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use clockwork_core::model::Shading;

pub struct RetroMaterialPlugin;

impl Plugin for RetroMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "retro_material.wesl");
        app.add_plugins(MaterialPlugin::<RetroMaterial>::default());
    }
}

/// Flags the shader reads.
const TEXTURED: u32 = 1;
const LIT: u32 = 2;
const ENVIRONMENT_MAP: u32 = 4;
const CUTOUT: u32 = 8;

/// One mesh's look: its texture, how it is shaded, and how it blends.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct RetroMaterial {
    #[uniform(0)]
    pub params: RetroParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
}

/// What the shader is told: colors in sRGB 0..1, the light's direction in view space.
#[derive(ShaderType, Clone, Debug, Default, PartialEq)]
pub struct RetroParams {
    pub ambient: Vec4,
    pub diffuse: Vec4,
    pub direction: Vec4,
    pub flags: u32,
    pub alpha_cutoff: f32,
}

impl RetroMaterial {
    /// The material for a mesh shaded `shading`, with `texture`, blending as `alpha_mode`.
    pub fn new(
        shading: Shading,
        environment_map: bool,
        texture: Option<Handle<Image>>,
        alpha_mode: AlphaMode,
    ) -> Self {
        Self {
            params: params(shading, environment_map, texture.is_some(), alpha_mode),
            texture,
            alpha_mode,
        }
    }
}

/// The shader's parameters for a mesh: pure, so it can be tested without a GPU.
pub fn params(
    shading: Shading,
    environment_map: bool,
    textured: bool,
    alpha_mode: AlphaMode,
) -> RetroParams {
    let mut flags = 0;
    if textured {
        flags |= TEXTURED;
    }
    if environment_map {
        flags |= ENVIRONMENT_MAP;
    }
    let mut alpha_cutoff = 0.0;
    if let AlphaMode::Mask(cutoff) = alpha_mode {
        flags |= CUTOUT;
        alpha_cutoff = cutoff;
    }
    let (ambient, diffuse, direction) = match shading {
        Shading::Unlit => (Vec3::ZERO, Vec3::ZERO, Vec3::Z),
        Shading::Lit {
            ambient,
            diffuse,
            direction,
        } => {
            flags |= LIT;
            (
                Vec3::from(ambient),
                Vec3::from(diffuse),
                Vec3::from(direction).normalize_or(Vec3::Z),
            )
        }
    };
    RetroParams {
        ambient: ambient.extend(1.0),
        diffuse: diffuse.extend(1.0),
        direction: direction.extend(0.0),
        flags,
        alpha_cutoff,
    }
}

impl Material for RetroMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(embedded_path!("retro_material.wesl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    // Old hardware casts no shadows on itself; its look is already in its colors.
    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // N64 models mix windings and are often single sided: draw both faces.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlit_meshes_are_their_vertex_colors_and_lit_ones_carry_their_lights() {
        let unlit = params(Shading::Unlit, false, true, AlphaMode::Opaque);
        assert_eq!(unlit.flags, TEXTURED);
        let lit = params(
            Shading::Lit {
                ambient: [0.5, 0.0, 0.0],
                diffuse: [1.0, 0.0, 0.0],
                direction: [0.0, 0.0, 2.0],
            },
            false,
            false,
            AlphaMode::Opaque,
        );
        assert_eq!(lit.flags, LIT);
        assert_eq!(lit.ambient, Vec4::new(0.5, 0.0, 0.0, 1.0));
        assert_eq!(lit.diffuse, Vec4::new(1.0, 0.0, 0.0, 1.0));
        // The direction arrives normalized.
        assert_eq!(lit.direction, Vec4::new(0.0, 0.0, 1.0, 0.0));
    }

    #[test]
    fn environment_maps_and_cutouts_set_their_flags() {
        let shiny = params(Shading::Unlit, true, true, AlphaMode::Mask(0.5));
        assert_eq!(shiny.flags, TEXTURED | ENVIRONMENT_MAP | CUTOUT);
        assert_eq!(shiny.alpha_cutoff, 0.5);
    }
}

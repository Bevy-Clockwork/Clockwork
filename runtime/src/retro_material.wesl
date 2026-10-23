// Draws a mesh the way old hardware did (see retro_material.rs): texture times vertex color, or
// lit like the N64's RSP (ambient plus diffuse times how much the view-space normal faces the
// light), all in the colors' own sRGB space; then converted to linear for the screen. No scene
// lights, no tonemapping.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

struct RetroParams {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    direction: vec4<f32>,
    flags: u32,
    alpha_cutoff: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material: RetroParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_sampler: sampler;

const TEXTURED: u32 = 1u;
const LIT: u32 = 2u;
const ENVIRONMENT_MAP: u32 = 4u;
const CUTOUT: u32 = 8u;

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}

/// The normal as the camera sees it: the N64 lights and maps in eye space. Only lit and
/// environment-mapped meshes have normals.
fn view_normal(in: VertexOutput, is_front: bool) -> vec3<f32> {
    let normal = normalize((view.view_from_world * vec4(in.world_normal, 0.0)).xyz);
    return select(-normal, normal, is_front);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    var shade = vec4(1.0);
@if(VERTEX_COLORS)
    shade = vec4(to_srgb(in.color.rgb), in.color.a);

    if (material.flags & LIT) != 0u {
        let facing = max(dot(view_normal(in, is_front), material.direction.xyz), 0.0);
        let light = min(material.ambient.rgb + material.diffuse.rgb * facing, vec3(1.0));
        shade = vec4(shade.rgb * light, shade.a);
    }

    var color = shade;
    if (material.flags & TEXTURED) != 0u {
        var uv = vec2(0.0);
@if(VERTEX_UVS_A)
        uv = in.uv;
        if (material.flags & ENVIRONMENT_MAP) != 0u {
            let normal = view_normal(in, is_front);
            uv = vec2(normal.x * 0.5 + 0.5, 0.5 - normal.y * 0.5);
        }
        let texel = textureSample(base_texture, base_sampler, uv);
        color = vec4(shade.rgb * to_srgb(texel.rgb), shade.a * texel.a);
    }

    if (material.flags & CUTOUT) != 0u {
        if color.a < material.alpha_cutoff {
            discard;
        }
        color.a = 1.0;
    }
    return vec4(to_linear(color.rgb), color.a);
}

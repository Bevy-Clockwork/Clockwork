//! Previews of recipe results, shared by the editor and the agent: surface overlays drawn from the
//! shared vocabulary (so every plugin's surfaces look the same), code highlights, and image views
//! with zoom and visible transparency.

use clockwork_core::action::AttachmentFrames;
use clockwork_core::character2d::{Character2d, Direction4};
use clockwork_core::image::RgbaImage;
use clockwork_core::model::{Animation, Blend, Model, Shading};
use clockwork_core::sprite::{Sprite, SpriteAnimation, SpriteFrame};
use clockwork_core::surface::{SurfaceMap, SurfaceProperty};

/// Overlay colors, in drawing order (later ones draw on top), with their legend labels.
pub const LEGEND: [(&str, [u8; 4]); 8] = [
    ("Shallow", [120, 200, 255, 110]),
    ("Swimmable", [40, 110, 255, 150]),
    ("Slippery", [235, 245, 255, 140]),
    ("Climbable", [240, 210, 60, 150]),
    ("Damaging", [200, 60, 220, 160]),
    ("Fall through", [10, 5, 5, 200]),
    ("Solid", [225, 50, 40, 150]),
    ("Game-specific", [80, 230, 120, 230]),
];

fn color(label: &str) -> [u8; 4] {
    LEGEND.iter().find(|(l, _)| *l == label).unwrap().1
}

/// An RGBA image the size of the map in native units (one pixel per native unit).
pub fn overlay(map: &SurfaceMap) -> RgbaImage {
    let size = map.cell_size;
    let mut image = RgbaImage::new(map.width * size, map.height * size);
    for y in 0..map.height {
        for x in 0..map.width {
            let Some(kind) = map.kind_at(x, y) else {
                continue;
            };
            let (x0, y0) = (x * size, y * size);
            let fill = |image: &mut RgbaImage, rgba: [u8; 4], part: Option<u8>| {
                for py in 0..size {
                    for px in 0..size {
                        if let Some(mask) = part {
                            let quarter = (px >= size / 2) as u8 + 2 * (py >= size / 2) as u8;
                            if mask >> quarter & 1 == 0 {
                                continue;
                            }
                        }
                        image.set(x0 + px, y0 + py, rgba);
                    }
                }
            };

            let props = map.resolve(&kind.properties);
            let has = |f: fn(&SurfaceProperty) -> bool| props.iter().any(f);
            for (label, test) in [
                (
                    "Shallow",
                    (|p| matches!(p, SurfaceProperty::Shallow)) as fn(&SurfaceProperty) -> bool,
                ),
                ("Swimmable", |p| matches!(p, SurfaceProperty::Swimmable)),
                ("Slippery", |p| matches!(p, SurfaceProperty::Slippery(_))),
                ("Climbable", |p| matches!(p, SurfaceProperty::Climbable)),
                ("Damaging", |p| matches!(p, SurfaceProperty::Damaging(_))),
                ("Fall through", |p| {
                    matches!(p, SurfaceProperty::FallThrough)
                }),
                ("Solid", |p| matches!(p, SurfaceProperty::Solid)),
            ] {
                if has(test) {
                    fill(&mut image, color(label), None);
                }
            }
            for p in &props {
                if let SurfaceProperty::SolidParts(mask) = p {
                    fill(&mut image, color("Solid"), Some(*mask));
                }
            }

            // Game-specific surfaces get a corner marker.
            if kind
                .properties
                .iter()
                .any(|p| matches!(p, SurfaceProperty::Extension(_)))
            {
                let marker = color("Game-specific");
                for i in 0..size / 3 {
                    for j in 0..(size / 3 - i) {
                        image.set(x0 + i, y0 + j, marker);
                    }
                }
            }
        }
    }
    image
}

/// Marks every cell whose code in `grid` is one of `codes`, in magenta with an outline.
pub fn highlight(map: &SurfaceMap, grid: &str, codes: &[u16]) -> RgbaImage {
    let size = map.cell_size;
    let mut image = RgbaImage::new(map.width * size, map.height * size);
    for y in 0..map.height {
        for x in 0..map.width {
            let Some(kind) = map.kind_at(x, y) else {
                continue;
            };
            let hit = kind
                .codes
                .iter()
                .any(|(g, code)| g == grid && codes.contains(code));
            if !hit {
                continue;
            }
            for py in 0..size {
                for px in 0..size {
                    let edge = px == 0 || py == 0 || px == size - 1 || py == size - 1;
                    let rgba = if edge {
                        [255, 0, 255, 255]
                    } else {
                        [255, 0, 255, 120]
                    };
                    image.set(x * size + px, y * size + py, rgba);
                }
            }
        }
    }
    image
}

/// `top` alpha-blended over `base` (top-left aligned).
pub fn blend(base: &RgbaImage, top: &RgbaImage) -> RgbaImage {
    let mut out = base.clone();
    for y in 0..top.height.min(base.height) {
        for x in 0..top.width.min(base.width) {
            let t = top.get(x, y);
            if t[3] == 0 {
                continue;
            }
            let b = out.get(x, y);
            let a = t[3] as u32;
            let mix = |i: usize| ((t[i] as u32 * a + b[i] as u32 * (255 - a)) / 255) as u8;
            out.set(x, y, [mix(0), mix(1), mix(2), b[3].max(t[3])]);
        }
    }
    out
}

/// The image over a grey checkerboard, so transparent pixels are visible.
pub fn on_checkerboard(image: &RgbaImage) -> RgbaImage {
    let mut board = RgbaImage::new(image.width, image.height);
    for y in 0..image.height {
        for x in 0..image.width {
            let light = (x / 8 + y / 8) % 2 == 0;
            let v = if light { 200 } else { 150 };
            board.set(x, y, [v, v, v, 255]);
        }
    }
    blend(&board, image)
}

/// Scales an image by a whole factor so its longer side is close to `target` pixels: shrinks
/// big images, enlarges small crops (nearest neighbor, so pixels stay crisp).
/// The image cropped to its visible pixels (a 1×1 transparent image when there are none).
pub fn trim(image: &RgbaImage) -> RgbaImage {
    let visible = |x: u32, y: u32| image.get(x, y)[3] != 0;
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..image.height {
        for x in 0..image.width {
            if visible(x, y) {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left == u32::MAX {
        return RgbaImage::new(1, 1);
    }
    image.crop(left, top, right - left, bottom - top)
}

pub fn fit(image: &RgbaImage, target: u32) -> RgbaImage {
    let longest = image.width.max(image.height).max(1);
    if longest > target {
        let factor = longest.div_ceil(target);
        let (w, h) = (
            (image.width / factor).max(1),
            (image.height / factor).max(1),
        );
        let mut out = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                out.set(x, y, image.get(x * factor, y * factor));
            }
        }
        out
    } else {
        let factor = (target / longest).max(1);
        let mut out = RgbaImage::new(image.width * factor, image.height * factor);
        for y in 0..out.height {
            for x in 0..out.width {
                out.set(x, y, image.get(x / factor, y / factor));
            }
        }
        out
    }
}

type Vec3 = [f32; 3];

/// Frames per row and rows at most in a contact sheet, so huge sprites still give a readable one.
const SHEET_COLUMNS: usize = 12;
const SHEET_ROWS: usize = 40;

/// A contact sheet of a sprite: one row per animation showing its steps in order (mirroring
/// applied, feet lined up by origin), or the first frames in a grid when there are none.
pub fn sprite_sheet(sprite: &Sprite) -> RgbaImage {
    let rows: Vec<Vec<SpriteFrame>> = if sprite.animations.is_empty() {
        sprite
            .frames
            .chunks(SHEET_COLUMNS)
            .take(SHEET_ROWS)
            .map(|chunk| chunk.to_vec())
            .collect()
    } else {
        sprite
            .animations
            .iter()
            .take(SHEET_ROWS)
            .map(|animation| {
                distinct_steps(animation)
                    .into_iter()
                    .filter_map(|(frame, fx, fy)| Some(sprite.frames.get(frame)?.flipped(fx, fy)))
                    .collect()
            })
            .collect()
    };
    frame_rows(&rows)
}

/// An animation's steps with repeats of the same picture left out, at most a sheet row.
fn distinct_steps(animation: &SpriteAnimation) -> Vec<(usize, bool, bool)> {
    let mut shown: Vec<(usize, bool, bool)> = Vec::new();
    for step in &animation.steps {
        let key = (step.frame, step.flip_x, step.flip_y);
        if shown.last() != Some(&key) {
            shown.push(key);
        }
    }
    shown.truncate(SHEET_COLUMNS);
    shown
}

/// A playable character's actions with what they hold: one row per action and facing (south,
/// then east), the action's animation followed by its hold, each frame drawn with its
/// attachments. `sprites` finds the plugin's other sprite outputs. `None` without actions.
pub fn actions_sheet<'a>(
    sprite: &Sprite,
    character: &Character2d,
    sprites: &dyn Fn(&str) -> Option<&'a Sprite>,
) -> Option<RgbaImage> {
    if character.actions.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    for action in character.actions.iter().take(SHEET_ROWS / 2) {
        for facing in [Direction4::South, Direction4::East] {
            let mut row = Vec::new();
            let phases = std::iter::once(action.animation.get(facing))
                .chain(action.hold.as_ref().map(|h| h.animation.get(facing)));
            for reference in phases {
                let Some(animation) = sprite.animation(&reference.name) else {
                    continue;
                };
                for (frame, fx, fy) in distinct_steps(animation) {
                    let (fx, fy) = (fx != reference.flip_x, fy != reference.flip_y);
                    let Some(body) = sprite.frames.get(frame) else {
                        continue;
                    };
                    let body = body.flipped(fx, fy);
                    let held: Vec<(bool, SpriteFrame)> = action
                        .attachments
                        .iter()
                        .filter_map(|a| {
                            let other = sprites(&a.sprite)?;
                            let shown = match &a.frames {
                                AttachmentFrames::FollowFrame { .. } => {
                                    let f = a.frames.follow(frame, other.frames.len())?;
                                    other.frames[f].flipped(fx, fy)
                                }
                                AttachmentFrames::Animation(directional) => {
                                    let r = directional.get(facing);
                                    let (step, _) = other.animation(&r.name)?.step_at(0.0)?;
                                    other
                                        .frames
                                        .get(step.frame)?
                                        .flipped(step.flip_x != r.flip_x, step.flip_y != r.flip_y)
                                }
                            };
                            Some((a.behind, shown))
                        })
                        .collect();
                    let mut layers: Vec<&SpriteFrame> =
                        held.iter().filter(|(b, _)| *b).map(|(_, f)| f).collect();
                    layers.push(&body);
                    layers.extend(held.iter().filter(|(b, _)| !*b).map(|(_, f)| f));
                    row.push(SpriteFrame::stack(&layers));
                    if row.len() >= SHEET_COLUMNS {
                        break;
                    }
                }
            }
            rows.push(row);
        }
    }
    Some(frame_rows(&rows))
}

/// Rows of frames in a grid; every cell has the room the largest frame needs around the origin,
/// so feet line up.
fn frame_rows(rows: &[Vec<SpriteFrame>]) -> RgbaImage {
    const PAD: i32 = 2;
    let frames = || rows.iter().flatten();
    let left = frames().map(|f| f.origin[0]).max().unwrap_or(0).max(0);
    let up = frames().map(|f| f.origin[1]).max().unwrap_or(0).max(0);
    let right = frames()
        .map(|f| f.image.width as i32 - f.origin[0])
        .max()
        .unwrap_or(1)
        .max(1);
    let down = frames()
        .map(|f| f.image.height as i32 - f.origin[1])
        .max()
        .unwrap_or(1)
        .max(1);
    let (cell_w, cell_h) = (left + right + PAD, up + down + PAD);
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut sheet = RgbaImage::new(
        (cell_w * columns as i32) as u32,
        (cell_h * rows.len().max(1) as i32) as u32,
    );
    for (row, frames) in rows.iter().enumerate() {
        for (column, frame) in frames.iter().enumerate() {
            let x = column as i32 * cell_w + left - frame.origin[0];
            let y = row as i32 * cell_h + up - frame.origin[1];
            sheet.overlay(&frame.image, x.max(0) as u32, y.max(0) as u32);
        }
    }
    sheet
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalized(v: Vec3) -> Vec3 {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        v.map(|x| x / length)
    } else {
        [0.0; 3]
    }
}

/// Rigid transforms keep translation out of normals and compose parent-first.
#[derive(Clone, Copy)]
struct Transform {
    rows: [Vec3; 3],
    translation: Vec3,
}

impl Transform {
    const IDENTITY: Self = Self {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [0.0; 3],
    };

    fn from_pose(translation: Vec3, quaternion: [f32; 4]) -> Self {
        let length = quaternion.iter().map(|x| x * x).sum::<f32>().sqrt();
        let [x, y, z, w] = if length > 0.0 {
            quaternion.map(|x| x / length)
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        Self {
            rows: [
                [
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y - z * w),
                    2.0 * (x * z + y * w),
                ],
                [
                    2.0 * (x * y + z * w),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z - x * w),
                ],
                [
                    2.0 * (x * z - y * w),
                    2.0 * (y * z + x * w),
                    1.0 - 2.0 * (x * x + y * y),
                ],
            ],
            translation,
        }
    }

    fn vector(self, v: Vec3) -> Vec3 {
        self.rows.map(|row| dot(row, v))
    }

    fn point(self, p: Vec3) -> Vec3 {
        add(self.vector(p), self.translation)
    }

    fn compose(self, child: Self) -> Self {
        Self {
            rows: std::array::from_fn(|i| {
                std::array::from_fn(|j| (0..3).map(|k| self.rows[i][k] * child.rows[k][j]).sum())
            }),
            translation: self.point(child.translation),
        }
    }
}

#[derive(Clone, Copy)]
struct PreviewVertex {
    position: Vec3,
    color: [f32; 4],
    uv: [f32; 2],
    /// What its color is multiplied by: its mesh's light (see `Shading`), or white.
    light: [f32; 3],
}

struct PreviewTriangle<'a> {
    vertices: [PreviewVertex; 3],
    texture: Option<&'a RgbaImage>,
    blend: Blend,
}

/// Software previews need no graphics context; supersampling smooths silhouettes and cutouts.
pub fn render_model(model: &Model, pose: Option<(&Animation, u32)>, size: u32) -> RgbaImage {
    let mut image = RgbaImage::new(size, size);
    if size == 0 {
        return image;
    }
    let mut bones: Vec<Transform> = Vec::with_capacity(model.bones.len());
    for (index, bone) in model.bones.iter().enumerate() {
        let mut translation = bone.translation;
        let mut rotation = [0.0, 0.0, 0.0, 1.0];
        if let Some((animation, frame)) = pose {
            for track in animation.tracks.iter().filter(|t| t.bone == index) {
                translation = track.translation(frame).unwrap_or(translation);
                rotation = track.rotation(frame).unwrap_or(rotation);
            }
        }
        let parent = bone
            .parent
            .and_then(|p| bones.get(p))
            .copied()
            .unwrap_or(Transform::IDENTITY);
        bones.push(parent.compose(Transform::from_pose(translation, rotation)));
    }

    let (sy, cy) = 30.0_f32.to_radians().sin_cos();
    let (sp, cp) = 20.0_f32.to_radians().sin_cos();
    let camera = Transform {
        rows: [
            [cy, 0.0, sy],
            [sp * sy, cp, -sp * cy],
            [-cp * sy, sp, cp * cy],
        ],
        translation: [0.0; 3],
    };
    // Billboards face the camera: their geometry turns as the camera is turned (its rotation
    // undone), staying where its bone is.
    for (bone, transform) in model.bones.iter().zip(bones.iter_mut()) {
        if bone.billboard {
            let r = camera.rows;
            transform.rows = std::array::from_fn(|i| std::array::from_fn(|j| r[j][i]));
        }
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let mut vertices: Vec<Vec<PreviewVertex>> = model
        .meshes
        .iter()
        .map(|mesh| {
            let bone = bones.get(mesh.bone).copied().unwrap_or(Transform::IDENTITY);
            mesh.positions
                .iter()
                .enumerate()
                .map(|(i, &p)| {
                    let world = bone.point(p);
                    if world.iter().all(|x| x.is_finite()) {
                        for axis in 0..3 {
                            lo[axis] = lo[axis].min(world[axis]);
                            hi[axis] = hi[axis].max(world[axis]);
                        }
                    }
                    // Shaded as the viewport shades it (runtime's RetroMaterial): in view
                    // space, the mesh's own lights, environment maps from the view normal.
                    let normal = mesh
                        .normals
                        .get(i)
                        .map(|&n| normalized(camera.vector(bone.vector(n))));
                    let light = match (mesh.shading, normal) {
                        (
                            Shading::Lit {
                                ambient,
                                diffuse,
                                direction,
                            },
                            Some(n),
                        ) => {
                            let facing = dot(n, direction).max(0.0);
                            std::array::from_fn(|c| (ambient[c] + diffuse[c] * facing).min(1.0))
                        }
                        _ => [1.0; 3],
                    };
                    let uv = match normal {
                        Some(n) if mesh.environment_map => [n[0] * 0.5 + 0.5, 0.5 - n[1] * 0.5],
                        _ => mesh.uvs.get(i).copied().unwrap_or([0.0; 2]),
                    };
                    PreviewVertex {
                        position: camera.vector(world),
                        color: mesh.colors.get(i).copied().unwrap_or([1.0; 4]),
                        uv,
                        light,
                    }
                })
                .collect()
        })
        .collect();
    if !lo[0].is_finite() {
        return image;
    }
    let center = camera.vector(add(lo, hi).map(|v| v * 0.5));
    let extent = vertices
        .iter()
        .flatten()
        .filter(|v| v.position.iter().all(|x| x.is_finite()))
        .fold(0.0_f32, |extent, v| {
            extent
                .max((v.position[0] - center[0]).abs())
                .max((v.position[1] - center[1]).abs())
        })
        .max(1e-6);
    let side = size as usize * 2;
    let scale = side as f32 * 0.45 / extent;
    for vertex in vertices.iter_mut().flatten() {
        vertex.position[0] = (vertex.position[0] - center[0]) * scale + side as f32 * 0.5;
        vertex.position[1] = (center[1] - vertex.position[1]) * scale + side as f32 * 0.5;
    }
    let mut pixels = vec![[0.0; 4]; side * side];
    let mut depth = vec![f32::NEG_INFINITY; side * side];
    let mut transparent = Vec::new();
    for (mesh, vertices) in model.meshes.iter().zip(&vertices) {
        let texture = mesh
            .texture
            .and_then(|i| model.textures.get(i))
            .filter(|t| t.width > 0 && t.height > 0 && mesh.uvs.len() == vertices.len());
        for indices in mesh.indices.as_chunks::<3>().0 {
            let [Some(&a), Some(&b), Some(&c)] =
                [indices[0], indices[1], indices[2]].map(|i| vertices.get(i as usize))
            else {
                continue;
            };
            let triangle = PreviewTriangle {
                vertices: [a, b, c],
                texture,
                blend: mesh.blend,
            };
            if mesh.blend == Blend::Transparent {
                transparent.push(triangle);
            } else {
                rasterize(&triangle, side, &mut pixels, &mut depth);
            }
        }
    }
    // Far-to-near blending preserves translucent surfaces across mesh boundaries.
    transparent.sort_by(|a, b| {
        let z = |t: &PreviewTriangle<'_>| t.vertices.iter().map(|v| v.position[2]).sum::<f32>();
        z(a).total_cmp(&z(b))
    });
    for triangle in &transparent {
        rasterize(triangle, side, &mut pixels, &mut depth);
    }
    // Average premultiplied samples, then return straight RGBA without dark edge fringes.
    for y in 0..size as usize {
        for x in 0..size as usize {
            let base = 2 * y * side + 2 * x;
            let mut sum = [0.0; 4];
            for offset in [0, 1, side, side + 1] {
                for (channel, value) in sum.iter_mut().enumerate() {
                    *value += pixels[base + offset][channel];
                }
            }
            if sum[3] > 0.0 {
                let rgb = |i: usize| (sum[i] / sum[3] * 255.0).round() as u8;
                image.set(
                    x as u32,
                    y as u32,
                    [rgb(0), rgb(1), rgb(2), (sum[3] * 63.75).round() as u8],
                );
            }
        }
    }
    image
}

fn edge(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Half-open edges ensure adjacent transparent triangles blend each sample only once.
fn rasterize(
    triangle: &PreviewTriangle<'_>,
    side: usize,
    pixels: &mut [[f32; 4]],
    depth: &mut [f32],
) {
    let [a, mut b, mut c] = triangle.vertices;
    if ![a, b, c]
        .iter()
        .all(|v| v.position.iter().all(|x| x.is_finite()))
    {
        return;
    }
    let mut area = edge(a.position, b.position, c.position);
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
        area = -area;
    }
    if area <= f32::EPSILON {
        return;
    }
    let points = [a.position, b.position, c.position];
    let min = |axis: usize| {
        points
            .iter()
            .map(|p| p[axis])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0)
            .min(side as f32) as usize
    };
    let max = |axis: usize| {
        points
            .iter()
            .map(|p| p[axis])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.0)
            .min(side as f32) as usize
    };
    let edges = [
        (b.position, c.position),
        (c.position, a.position),
        (a.position, b.position),
    ];
    let inclusive = edges.map(|(a, b)| {
        let d = sub(b, a);
        d[1] < 0.0 || (d[1] == 0.0 && d[0] > 0.0)
    });
    let inverse_area = 1.0 / area;
    for y in min(1)..max(1) {
        for x in min(0)..max(0) {
            let p = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
            let weights = edges.map(|(a, b)| edge(a, b, p));
            if weights
                .iter()
                .zip(inclusive)
                .any(|(&w, include)| w < 0.0 || (w == 0.0 && !include))
            {
                continue;
            }
            let [_, wb, wc] = weights.map(|w| w * inverse_area);
            let interpolate = |a: f32, b: f32, c: f32| a + wb * (b - a) + wc * (c - a);
            let z = interpolate(a.position[2], b.position[2], c.position[2]);
            let index = y * side + x;
            if z < depth[index] {
                continue;
            }
            let mut color: [f32; 4] =
                std::array::from_fn(|i| interpolate(a.color[i], b.color[i], c.color[i]));
            if let Some(texture) = triangle.texture {
                let uv: [f32; 2] =
                    std::array::from_fn(|i| interpolate(a.uv[i], b.uv[i], c.uv[i]).rem_euclid(1.0));
                let tx = ((uv[0] * texture.width as f32) as u32).min(texture.width - 1);
                let ty = ((uv[1] * texture.height as f32) as u32).min(texture.height - 1);
                for (channel, sample) in color.iter_mut().zip(texture.get(tx, ty)) {
                    *channel *= sample as f32 / 255.0;
                }
            }
            if triangle.blend == Blend::Cutout && color[3] < 0.5 {
                continue;
            }
            let alpha = if triangle.blend == Blend::Transparent {
                color[3].clamp(0.0, 1.0)
            } else {
                1.0
            };
            let target = &mut pixels[index];
            for i in 0..3 {
                let light = interpolate(a.light[i], b.light[i], c.light[i]);
                target[i] = (color[i] * light).clamp(0.0, 1.0) * alpha + target[i] * (1.0 - alpha);
            }
            target[3] = alpha + target[3] * (1.0 - alpha);
            if triangle.blend != Blend::Transparent {
                depth[index] = z;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::model::{Bone, Mesh, Track};
    use clockwork_core::surface::CodeMeaning;
    use std::collections::BTreeMap;

    fn triangle_model() -> Model {
        Model {
            bones: vec![Bone {
                name: String::new(),
                parent: None,
                translation: [0.0; 3],
                billboard: false,
            }],
            meshes: vec![Mesh {
                positions: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]],
                colors: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                indices: vec![0, 1, 2],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn model_triangle_has_opaque_center_and_transparent_corners() {
        let mut model = triangle_model();
        let image = render_model(&model, None, 64);
        assert_eq!(image.get(32, 32), [255, 0, 0, 255]);
        for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
            assert_eq!(image.get(x, y), [0; 4]);
        }
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|&p| p[3] > 0 && p[3] < 255)
        );
        model.meshes[0].indices.reverse();
        assert_eq!(image, render_model(&model, None, 64));
        assert_eq!(render_model(&model, None, 0), RgbaImage::new(0, 0));
        assert_eq!(
            render_model(&Model::default(), None, 8),
            RgbaImage::new(8, 8)
        );
    }

    #[test]
    fn billboards_face_the_preview_camera() {
        // A billboard is drawn in its own XY plane, facing +Z. However its bone turns, it shows
        // the camera the same face; an ordinary triangle turned edge-on shows nothing.
        let mut model = triangle_model();
        let turned = Animation {
            name: "turn".into(),
            frames: 1,
            fps: 30.0,
            loop_start: None,
            tracks: vec![Track {
                bone: 0,
                translations: Vec::new(),
                // A quarter turn about x: the triangle lies flat.
                rotations: vec![[
                    std::f32::consts::FRAC_1_SQRT_2,
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                ]],
            }],
        };
        let still = render_model(&model, None, 64);
        let flat = render_model(&model, Some((&turned, 0)), 64);
        assert_ne!(flat, still);
        model.bones[0].billboard = true;
        let facing = render_model(&model, None, 64);
        assert_eq!(render_model(&model, Some((&turned, 0)), 64), facing);
        assert_eq!(facing.get(32, 32), [255, 0, 0, 255]);
    }

    #[test]
    fn previews_shade_as_the_hardware_did() {
        // Unlit, even with normals: exactly its vertex color, whatever way it faces.
        let mut model = triangle_model();
        model.meshes[0].normals = vec![[0.0, 0.0, 1.0]; 3];
        assert_eq!(render_model(&model, None, 64).get(32, 32), [255, 0, 0, 255]);
        // Lit: ambient, plus diffuse where the normal faces the light (in view space).
        let mut lit = |direction| {
            model.meshes[0].colors = vec![[1.0; 4]; 3];
            model.meshes[0].shading = Shading::Lit {
                ambient: [0.25, 0.25, 0.25],
                diffuse: [0.5, 0.5, 0.5],
                direction,
            };
            render_model(&model, None, 64).get(32, 32)
        };
        let away = lit([0.0, 0.0, -1.0]);
        let towards = lit([0.0, 0.0, 1.0]);
        assert_eq!(away, [64, 64, 64, 255], "only the ambient light");
        assert!(towards[0] > away[0], "{towards:?} brighter than {away:?}");
    }

    #[test]
    fn model_child_translation_changes_preview() {
        let mut model = triangle_model();
        model.bones.push(Bone {
            name: String::new(),
            parent: Some(0),
            translation: [3.0, 0.0, 0.0],
            billboard: false,
        });
        let mut child = model.meshes[0].clone();
        child.bone = 1;
        child.colors.fill([0.0, 1.0, 0.0, 1.0]);
        model.meshes.push(child);
        let offset = render_model(&model, None, 64);
        assert!(offset.pixels.as_chunks::<4>().0.contains(&[255, 0, 0, 255]));
        assert!(offset.pixels.as_chunks::<4>().0.contains(&[0, 255, 0, 255]));
        model.bones[1].translation = [0.0; 3];
        assert_ne!(offset, render_model(&model, None, 64));
    }

    #[test]
    fn model_animation_rotation_changes_preview_and_holds() {
        let model = triangle_model();
        let animation = Animation {
            name: String::new(),
            frames: 2,
            fps: 30.0,
            loop_start: None,
            tracks: vec![Track {
                bone: 0,
                rotations: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]],
                ..Default::default()
            }],
        };
        let rest = render_model(&model, None, 64);
        assert_eq!(rest, render_model(&model, Some((&animation, 0)), 64));
        let rotated = render_model(&model, Some((&animation, 1)), 64);
        assert_ne!(rest, rotated);
        assert_eq!(rotated, render_model(&model, Some((&animation, 20)), 64));
        let parent = Transform::from_pose([2.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]);
        let child = Transform::from_pose([3.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            parent.compose(child).point([1.0, 0.0, 0.0]),
            [-2.0, 0.0, 0.0]
        );
    }

    #[test]
    fn model_texture_repeat_cutout_and_transparency() {
        let mut model = triangle_model();
        model.textures.push(RgbaImage {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 255, 255, 255, 0,
            ],
        });
        let mesh = &mut model.meshes[0];
        mesh.colors.fill([1.0; 4]);
        mesh.texture = Some(0);
        mesh.uvs = vec![[1.25, -0.25]; 3];
        mesh.blend = Blend::Transparent;
        assert_eq!(render_model(&model, None, 64).get(32, 32), [0, 0, 255, 128]);
        model.meshes[0].blend = Blend::Cutout;
        assert_eq!(render_model(&model, None, 64).get(32, 32), [0, 0, 255, 255]);
        model.meshes[0].uvs.fill([0.75, 0.75]);
        assert_eq!(render_model(&model, None, 64), RgbaImage::new(64, 64));
    }

    #[test]
    fn model_depth_and_transparent_order() {
        let mut model = triangle_model();
        let mut front = model.meshes[0].clone();
        // Moving along the camera direction keeps the projected triangles aligned.
        let (sy, cy) = 30.0_f32.to_radians().sin_cos();
        let (sp, cp) = 20.0_f32.to_radians().sin_cos();
        for p in &mut front.positions {
            *p = add(*p, [-cp * sy, sp, cp * cy]);
        }
        front.colors.fill([0.0, 0.0, 1.0, 0.5]);
        front.blend = Blend::Transparent;
        model.meshes.insert(0, front);
        let image = render_model(&model, None, 64);
        assert_eq!(image.get(32, 32), [128, 0, 128, 255]);
        model.meshes.reverse();
        assert_eq!(image, render_model(&model, None, 64));
        model.meshes[1].blend = Blend::Opaque;
        assert_eq!(render_model(&model, None, 64).get(32, 32), [0, 0, 255, 255]);
        model.meshes.reverse();
        assert_eq!(render_model(&model, None, 64).get(32, 32), [0, 0, 255, 255]);
    }

    /// Opt-in timing keeps hardware-dependent performance checks out of normal tests.
    #[test]
    #[ignore]
    fn model_preview_release_timing() {
        let mut model = triangle_model();
        model.meshes.clear();
        model.bones.resize(20, model.bones[0].clone());
        for bone in 0..20 {
            let mut mesh = Mesh {
                bone,
                ..Default::default()
            };
            for tile in 0..50 {
                let x = (tile % 10) as f32;
                let y = (tile / 10 + bone * 5) as f32 * 0.1;
                let base = mesh.positions.len() as u32;
                mesh.positions.extend([
                    [x, y, 0.0],
                    [x + 1.0, y, 0.0],
                    [x + 1.0, y + 0.1, 0.0],
                    [x, y + 0.1, 0.0],
                ]);
                mesh.normals.extend([[0.0, 0.0, 1.0]; 4]);
                mesh.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            model.meshes.push(mesh);
        }
        let start = std::time::Instant::now();
        let image = render_model(&model, None, 256);
        eprintln!("20 bones / 2000 triangles / 256px: {:?}", start.elapsed());
        assert!(image.pixels.as_chunks::<4>().0.iter().any(|&p| p[3] == 255));
    }

    #[test]
    fn highlights_and_fits() {
        let meaning = CodeMeaning {
            codes: BTreeMap::new(),
            other: Some(vec![]),
        };
        let map = SurfaceMap::build(2, 4, &[("collision", &[0, 7], &meaning)], vec![]).unwrap();
        let marked = highlight(&map, "collision", &[7]);
        assert_eq!(marked.get(0, 0)[3], 0);
        assert_eq!(marked.get(4, 0), [255, 0, 255, 255]);

        assert_eq!((fit(&marked, 4).width, fit(&marked, 4).height), (4, 2));
        assert_eq!((fit(&marked, 64).width, fit(&marked, 64).height), (64, 32));
        assert_eq!(on_checkerboard(&marked).get(0, 0)[3], 255);
    }
}

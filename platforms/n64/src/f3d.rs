//! Fast3D display lists translated into rigid meshes, without game-specific recipes.

use crate::texture::{self, TexFormat, TexSize};
pub use clockwork_core::model::Wrap;
use clockwork_core::model::{Blend, Mesh, Model, Shading};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum F3dError {
    #[error("segment {0:#x} is missing")]
    MissingSegment(u8),
    #[error("read at {address:#010x} of {length} bytes is out of bounds")]
    OutOfBounds { address: u32, length: usize },
    #[error("display list nesting exceeds 16")]
    TooDeep,
    #[error("display list exceeds 100000 commands")]
    Runaway,
    #[error("unsupported or invalid texture")]
    BadTexture,
    #[error("invalid or unloaded vertex slot {0}")]
    BadVertex(usize),
}

#[derive(Default)]
pub struct Memory<'a> {
    segments: [Option<&'a [u8]>; 16],
}
impl<'a> Memory<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Segment numbers use the same low four bits as segmented addresses.
    pub fn set(&mut self, segment: u8, data: &'a [u8]) {
        self.segments[(segment & 15) as usize] = Some(data);
    }
    pub fn read(&self, address: u32, length: usize) -> Result<&'a [u8], F3dError> {
        let segment = ((address >> 24) & 15) as u8;
        let data = self.segments[segment as usize].ok_or(F3dError::MissingSegment(segment))?;
        let start = (address & 0xffffff) as usize;
        start
            .checked_add(length)
            .and_then(|end| data.get(start..end))
            .ok_or(F3dError::OutOfBounds { address, length })
    }
}

/// Geometry mode bit that makes vertex colors normals, lit by the current lights.
pub const G_LIGHTING: u32 = 0x0002_0000;
/// Geometry mode bit that makes texture coordinates from normals (environment mapping).
pub const G_TEXGEN: u32 = 0x0004_0000;
fn wrap(bits: u32) -> Wrap {
    if bits & 2 != 0 {
        Wrap::Clamp
    } else if bits & 1 != 0 {
        Wrap::Mirror
    } else {
        Wrap::Repeat
    }
}

#[derive(Clone, Copy)]
struct Tile {
    fmt: Option<TexFormat>,
    siz: TexSize,
    width: u32,
    height: u32,
    palette: usize,
    wrap: [Wrap; 2],
}
impl Default for Tile {
    fn default() -> Self {
        Self {
            fmt: Some(TexFormat::Rgba),
            siz: TexSize::B16,
            width: 0,
            height: 0,
            palette: 0,
            wrap: [Wrap::Repeat; 2],
        }
    }
}
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    st: [f32; 2],
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct TextureKey {
    address: u32,
    fmt: TexFormat,
    siz: TexSize,
    width: u32,
    height: u32,
    palette: Vec<u8>,
    shade: Option<[u8; 3]>,
    wrap: [Wrap; 2],
}

pub struct ModelBuilder<'a> {
    pub model: Model,
    /// Counts ignored commands so recipes can diagnose unsupported features.
    pub skipped: BTreeMap<u8, u32>,
    memory: Memory<'a>,
    vertices: [Option<Vertex>; 16],
    geometry: u32,
    diffuse: [u8; 3],
    ambient: [u8; 3],
    light_direction: [i8; 3],
    enabled: bool,
    scale: [f32; 2],
    image: u32,
    loaded: Option<u32>,
    palette: Vec<u8>,
    tiles: [Tile; 8],
    decal: bool,
    textures: BTreeMap<TextureKey, usize>,
}
impl<'a> ModelBuilder<'a> {
    pub fn new(memory: Memory<'a>) -> Self {
        Self {
            model: Model::default(),
            skipped: BTreeMap::new(),
            memory,
            vertices: [None; 16],
            geometry: 0,
            diffuse: [255; 3],
            ambient: [0; 3],
            light_direction: [0; 3],
            enabled: false,
            scale: [1.0; 2],
            image: 0,
            loaded: None,
            palette: Vec::new(),
            tiles: [Tile::default(); 8],
            decal: false,
            textures: BTreeMap::new(),
        }
    }

    /// Sets the geometry mode the game has set up before drawing models (games usually turn on
    /// lighting once per frame, not in every model's lists).
    pub fn with_geometry_mode(mut self, bits: u32) -> Self {
        self.geometry = bits;
        self
    }
    /// Interprets one root list; RSP state persists across calls for shared setup lists.
    pub fn draw(&mut self, dl: u32, bone: usize, blend: Blend) -> Result<(), F3dError> {
        let mut pc = dl;
        let mut stack = Vec::new();
        for _ in 0..100_000 {
            let cmd = self.memory.read(pc, 8)?;
            let w0 = u32::from_be_bytes(cmd[..4].try_into().unwrap());
            let w1 = u32::from_be_bytes(cmd[4..].try_into().unwrap());
            let next = pc
                .checked_add(8)
                .filter(|n| n & 0xff000000 == pc & 0xff000000)
                .ok_or(F3dError::OutOfBounds {
                    address: pc,
                    length: 8,
                })?;
            pc = next;
            match (w0 >> 24) as u8 {
                0x04 => {
                    let n = ((w0 >> 20) & 15) as usize + 1;
                    let v0 = ((w0 >> 16) & 15) as usize;
                    if v0 + n > 16 {
                        return Err(F3dError::BadVertex(v0 + n - 1));
                    }
                    let data = self.memory.read(w1, n * 16)?;
                    for (i, v) in data.as_chunks::<16>().0.iter().enumerate() {
                        let short = |p| i16::from_be_bytes([v[p], v[p + 1]]) as f32;
                        let normal =
                            normalize([v[12] as i8 as f32, v[13] as i8 as f32, v[14] as i8 as f32]);
                        // A lit vertex's color comes from the lights (see `Shading::Lit`).
                        let rgb = if self.geometry & G_LIGHTING != 0 {
                            [255; 3]
                        } else {
                            [v[12], v[13], v[14]]
                        };
                        self.vertices[v0 + i] = Some(Vertex {
                            position: [short(0), short(2), short(4)],
                            normal,
                            color: [
                                rgb[0] as f32 / 255.0,
                                rgb[1] as f32 / 255.0,
                                rgb[2] as f32 / 255.0,
                                v[15] as f32 / 255.0,
                            ],
                            st: [short(8), short(10)],
                        });
                    }
                }
                0xbf => self.triangle(w1, bone, blend)?,
                0x06 => {
                    if (w0 >> 16) & 255 == 0 {
                        if stack.len() >= 15 {
                            return Err(F3dError::TooDeep);
                        }
                        stack.push(next);
                    }
                    pc = w1;
                }
                0xb8 => match stack.pop() {
                    Some(ret) => pc = ret,
                    None => return Ok(()),
                },
                0x03 => match (w0 >> 16) & 255 {
                    0x86 => {
                        let light = self.memory.read(w1, 11)?;
                        self.diffuse.copy_from_slice(&light[..3]);
                        self.light_direction = [light[8] as i8, light[9] as i8, light[10] as i8];
                    }
                    0x88 => self.ambient.copy_from_slice(self.memory.read(w1, 3)?),
                    _ => {}
                },
                0xb7 => self.geometry |= w1,
                0xb6 => self.geometry &= !w1,
                0xbb => {
                    self.enabled = w0 & 255 != 0;
                    self.scale = [(w1 >> 16) as f32 / 65536.0, (w1 & 65535) as f32 / 65536.0];
                }
                0xfd => self.image = w1,
                0xf5 => {
                    let tile = &mut self.tiles[((w1 >> 24) & 7) as usize];
                    tile.fmt = TexFormat::from_bits((w0 >> 21) & 7);
                    tile.siz = TexSize::from_bits((w0 >> 19) & 3).unwrap();
                    tile.palette = ((w1 >> 20) & 15) as usize;
                    tile.wrap = [wrap(w1 >> 8), wrap(w1 >> 18)];
                }
                0xf2 => {
                    let tile = &mut self.tiles[((w1 >> 24) & 7) as usize];
                    tile.width = (((w1 >> 12) & 4095)
                        .checked_sub((w0 >> 12) & 4095)
                        .ok_or(F3dError::BadTexture)?
                        >> 2)
                        + 1;
                    tile.height = ((w1 & 4095)
                        .checked_sub(w0 & 4095)
                        .ok_or(F3dError::BadTexture)?
                        >> 2)
                        + 1;
                }
                0xf3 | 0xf4 => self.loaded = Some(self.image),
                0xf0 => {
                    let count = ((w1 >> 14) & 1023) as usize + 1;
                    self.palette = self.memory.read(self.image, count * 2)?.to_vec();
                }
                0xfc => {
                    self.decal = (w0 >> 20) & 15 == 1
                        && (w0 >> 15) & 31 == 8
                        && (w1 >> 28) & 15 == 4
                        && (w1 >> 15) & 7 == 4
                }
                opcode => *self.skipped.entry(opcode).or_default() += 1,
            }
        }
        Err(F3dError::Runaway)
    }
    /// How triangles drawn now are shaded. The RSP lights in eye space: loading a light
    /// transforms its direction by the modelview matrix of the moment, so the direction a
    /// display list gives is relative to the camera (a lit model looks lit from the same side of
    /// the screen however the camera turns). The renderer takes it as a view-space direction.
    fn shading(&self) -> Shading {
        if self.geometry & G_LIGHTING == 0 {
            return Shading::Unlit;
        }
        let color = |c: [u8; 3]| c.map(|v| v as f32 / 255.0);
        let d = self.light_direction.map(|v| v as f32);
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        Shading::Lit {
            ambient: color(self.ambient),
            diffuse: color(self.diffuse),
            direction: if length > 0.0 {
                d.map(|v| v / length)
            } else {
                [0.0, 0.0, 1.0]
            },
        }
    }

    fn texture(&mut self) -> Result<Option<usize>, F3dError> {
        if !self.enabled {
            return Ok(None);
        }
        let tile = self.tiles[0];
        let fmt = tile.fmt.ok_or(F3dError::BadTexture)?;
        if tile.width == 0 || tile.height == 0 {
            return Err(F3dError::BadTexture);
        }
        let palette = if fmt == TexFormat::Ci {
            self.palette
                .get(
                    if tile.siz == TexSize::B4 {
                        tile.palette * 32
                    } else {
                        0
                    }..,
                )
                .ok_or(F3dError::BadTexture)?
                .to_vec()
        } else {
            Vec::new()
        };
        let key = TextureKey {
            address: self.loaded.ok_or(F3dError::BadTexture)?,
            fmt,
            siz: tile.siz,
            width: tile.width,
            height: tile.height,
            palette,
            shade: self.decal.then_some(self.diffuse),
            wrap: tile.wrap,
        };
        if let Some(&index) = self.textures.get(&key) {
            return Ok(Some(index));
        }
        let length = tile
            .siz
            .bytes(tile.width as usize * tile.height as usize)
            .ok_or(F3dError::BadTexture)?;
        let data = self.memory.read(key.address, length)?;
        let mut image = texture::decode(
            fmt,
            tile.siz,
            data,
            tile.width,
            tile.height,
            Some(&key.palette),
        )
        .ok_or(F3dError::BadTexture)?;
        if let Some(shade) = key.shade {
            for p in image.pixels.as_chunks_mut::<4>().0 {
                let a = p[3] as u32;
                for c in 0..3 {
                    p[c] = ((shade[c] as u32 * (255 - a) + p[c] as u32 * a + 127) / 255) as u8;
                }
                p[3] = 255;
            }
        }
        let index = self.model.textures.len();
        self.model.textures.push(image);
        self.model.texture_wrap.push(tile.wrap);
        self.textures.insert(key, index);
        Ok(Some(index))
    }
    fn triangle(&mut self, word: u32, bone: usize, blend: Blend) -> Result<(), F3dError> {
        let mut vertices = Vec::with_capacity(3);
        for shift in [16, 8, 0] {
            let encoded = ((word >> shift) & 255) as usize;
            let slot = encoded / 10;
            if !encoded.is_multiple_of(10) {
                return Err(F3dError::BadVertex(slot));
            }
            vertices.push(
                self.vertices
                    .get(slot)
                    .copied()
                    .flatten()
                    .ok_or(F3dError::BadVertex(slot))?,
            );
        }
        let texture = self.texture()?;
        let shading = self.shading();
        // Texture generation only changes texture coordinates: untextured, it does nothing.
        let environment_map = self.geometry & G_TEXGEN != 0 && texture.is_some();
        let index = self
            .model
            .meshes
            .iter()
            .position(|m| {
                m.bone == bone
                    && m.texture == texture
                    && m.blend == blend
                    && m.shading == shading
                    && m.environment_map == environment_map
            })
            .unwrap_or_else(|| {
                self.model.meshes.push(Mesh {
                    bone,
                    texture,
                    blend,
                    shading,
                    environment_map,
                    ..Default::default()
                });
                self.model.meshes.len() - 1
            });
        let mesh = &mut self.model.meshes[index];
        let normals = shading != Shading::Unlit || environment_map;
        for v in vertices {
            mesh.indices.push(mesh.positions.len() as u32);
            mesh.positions.push(v.position);
            if normals {
                mesh.normals.push(v.normal);
            }
            mesh.colors.push(if self.decal && texture.is_some() {
                [1.0; 4]
            } else {
                v.color
            });
            if texture.is_some() {
                mesh.uvs.push(if self.geometry & 0x40000 != 0 {
                    [v.normal[0] * 0.5 + 0.5, v.normal[1] * 0.5 + 0.5]
                } else {
                    [
                        v.st[0] / 32.0 * self.scale[0] / self.tiles[0].width as f32,
                        v.st[1] / 32.0 * self.scale[1] / self.tiles[0].height as f32,
                    ]
                });
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Model {
        self.model
    }
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length == 0.0 {
        [0.0; 3]
    } else {
        v.map(|n| n / length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(data: &mut Vec<u8>, a: u32, b: u32) {
        data.extend(a.to_be_bytes());
        data.extend(b.to_be_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut data = Vec::new();
        command(&mut data, 0x03860000, 0x01000100);
        command(&mut data, 0xb7000000, G_LIGHTING);
        command(&mut data, 0x04300000, 0x01000110);
        command(&mut data, 0x06000000, 0x01000080);
        command(&mut data, 0xb8000000, 0);
        data.resize(0x80, 0);
        command(&mut data, 0xbf000000, 0x00000a14);
        command(&mut data, 0xbf000000, 0x0000141e);
        command(&mut data, 0xb8000000, 0);
        data.resize(0x100, 0);
        data.extend([128, 64, 32, 0, 0, 0, 0, 0, 0, 0, 127, 0, 0, 0, 0, 0]);
        for (x, y) in [(0i16, 0i16), (2, 0), (2, 2), (0, 2)] {
            for n in [x, y, -3, 0, x * 32, y * 32] {
                data.extend(n.to_be_bytes());
            }
            data.extend([0, 0, 127, 192]);
        }
        data.extend([0xf8, 1, 7, 0xc1, 0, 0x3f, 0xff, 0xff]);
        data
    }
    fn builder(data: &[u8]) -> ModelBuilder<'_> {
        let mut memory = Memory::new();
        memory.set(1, data);
        ModelBuilder::new(memory)
    }
    #[test]
    fn lit_quad_and_call() {
        let data = fixture();
        let mut b = builder(&data);
        b.draw(0x01000000, 3, Blend::Opaque).unwrap();
        let m = &b.model.meshes[0];
        assert_eq!(m.bone, 3);
        assert_eq!(
            m.positions,
            vec![
                [0., 0., -3.],
                [2., 0., -3.],
                [2., 2., -3.],
                [0., 0., -3.],
                [2., 2., -3.],
                [0., 2., -3.]
            ]
        );
        assert_eq!(m.normals, vec![[0., 0., 1.]; 6]);
        // Lit: the vertices are white and the lights carry the colors, the direction in eye
        // space as the display list gave it.
        assert_eq!(m.colors, vec![[1., 1., 1., 192. / 255.]; 6]);
        assert_eq!(
            m.shading,
            Shading::Lit {
                ambient: [0.0; 3],
                diffuse: [128. / 255., 64. / 255., 32. / 255.],
                direction: [0.0, 0.0, 1.0],
            }
        );
        assert!(!m.environment_map);
        assert_eq!(m.indices, vec![0, 1, 2, 3, 4, 5]);
        assert!(m.uvs.is_empty());
    }
    fn textured(data: &mut Vec<u8>) -> u32 {
        let address = 0x01000000 + data.len() as u32;
        command(data, 0xfd100000, 0x01000150);
        command(data, 0xf3000000, 0);
        command(data, 0xf5100000, (2 << 8) | (1 << 18));
        command(data, 0xf2000000, (4 << 12) | 4);
        command(data, 0xbb000001, 0x80008000);
        command(data, 0x06000000, 0x01000000);
        command(data, 0xb8000000, 0);
        address
    }
    #[test]
    fn texture_uvs_wraps_and_reuse() {
        let mut data = fixture();
        let dl = textured(&mut data);
        let mut b = builder(&data);
        b.draw(dl, 0, Blend::Cutout).unwrap();
        b.draw(dl, 0, Blend::Cutout).unwrap();
        let model = b.finish();
        assert_eq!(model.textures.len(), 1);
        assert_eq!(model.meshes.len(), 1);
        assert_eq!(model.texture_wrap, vec![[Wrap::Clamp, Wrap::Mirror]]);
        assert_eq!(
            model.textures[0].pixels,
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
            ]
        );
        let m = &model.meshes[0];
        assert_eq!(m.texture, Some(0));
        assert_eq!(m.indices.len(), 12);
        assert_eq!(&m.uvs[..3], &[[0., 0.], [0.5, 0.], [0.5, 0.5]]);
    }
    #[test]
    fn decal_environment_and_unlit() {
        let mut data = fixture();
        data[0x151] = 0;
        let dl = textured(&mut data);
        let root = 0x01000000 + data.len() as u32;
        command(
            &mut data,
            0xfc000000 | (1 << 20) | (8 << 15),
            (4 << 28) | (4 << 15),
        );
        command(&mut data, 0xb7000000, 0x40000);
        command(&mut data, 0x06000000, dl);
        command(&mut data, 0xb6000000, G_LIGHTING);
        command(&mut data, 0x04300000, 0x01000110);
        command(&mut data, 0xbb000000, 0);
        command(&mut data, 0x06000000, 0x01000080);
        command(&mut data, 0xb8000000, 0);
        let mut b = builder(&data);
        b.draw(root, 0, Blend::Opaque).unwrap();
        assert_eq!(b.model.textures[0].get(0, 0), [128, 64, 32, 255]);
        assert_eq!(b.model.meshes[0].colors, vec![[1.; 4]; 6]);
        assert_eq!(b.model.meshes[0].uvs, vec![[0.5, 0.5]; 6]);
        // Texture generation: an environment-mapped mesh, with normals for the renderer.
        assert!(b.model.meshes[0].environment_map);
        assert_eq!(b.model.meshes[0].normals.len(), 6);
        assert_eq!(b.model.meshes[1].shading, Shading::Unlit);
        assert!(b.model.meshes[1].normals.is_empty());
        assert_eq!(
            b.model.meshes[1].colors[0],
            [0., 0., 127. / 255., 192. / 255.]
        );
    }
    #[test]
    fn memory_and_execution_errors() {
        assert!(matches!(
            Memory::new().read(0x02000000, 1),
            Err(F3dError::MissingSegment(2))
        ));
        let mut mem = Memory::new();
        mem.set(1, &[1, 2]);
        assert_eq!(mem.read(0xf1000001, 1).unwrap(), [2]);
        assert!(matches!(
            mem.read(0x01000001, usize::MAX),
            Err(F3dError::OutOfBounds { .. })
        ));
        let mut data = Vec::new();
        command(&mut data, 0x06000000, 0x01000000);
        assert_eq!(
            builder(&data).draw(0x01000000, 0, Blend::Opaque),
            Err(F3dError::TooDeep)
        );
        data[1] = 1;
        assert_eq!(
            builder(&data).draw(0x01000000, 0, Blend::Opaque),
            Err(F3dError::Runaway)
        );
        data.clear();
        command(&mut data, 0xbf000000, 0);
        assert_eq!(
            builder(&data).draw(0x01000000, 0, Blend::Opaque),
            Err(F3dError::BadVertex(0))
        );
        data.clear();
        command(&mut data, 0xe7000000, 0);
        command(&mut data, 0xb8000000, 0);
        let mut b = builder(&data);
        b.draw(0x01000000, 0, Blend::Opaque).unwrap();
        assert_eq!(b.skipped.get(&0xe7), Some(&1));
    }
    /// Local ROM verification is opt-in; no source data is stored in the repository.
    #[test]
    #[ignore]
    fn local_rom_smoke() {
        let Ok(path) = std::env::var("N64_SMOKE_ROM") else {
            return;
        };
        let offset =
            usize::from_str_radix(&std::env::var("N64_SMOKE_OFFSET").unwrap(), 16).unwrap();
        let rom = std::fs::read(path).unwrap();
        let segment = crate::mio0::decompress(&rom[offset..]).unwrap();
        for dl in std::env::var("N64_SMOKE_LISTS").unwrap().split(',') {
            let dl = u32::from_str_radix(dl, 16).unwrap();
            let mut mem = Memory::new();
            mem.set((dl >> 24) as u8, &segment);
            let mut b = ModelBuilder::new(mem).with_geometry_mode(G_LIGHTING);
            b.draw(dl, 0, Blend::Opaque).unwrap();
            println!(
                "{dl:#010x}: {} meshes, {} triangles, {} textures, {} lit vertices; skipped {:?}",
                b.model.meshes.len(),
                b.model
                    .meshes
                    .iter()
                    .map(|m| m.indices.len() / 3)
                    .sum::<usize>(),
                b.model.textures.len(),
                b.model
                    .meshes
                    .iter()
                    .map(|m| m.normals.len())
                    .sum::<usize>(),
                b.skipped
            );
            assert!(!b.model.meshes.is_empty());
        }
    }
}

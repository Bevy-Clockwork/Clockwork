//! Loads plugin folders and runs their extraction recipes against a ROM.
//!
//! Everything runs locally on the user's own ROM; plugins only carry offsets and parameters.

mod ability;
mod anim;
pub mod assets;
pub mod check;
mod collision;
pub mod compose;
pub mod edit;
pub mod preview;
mod sprite;
pub mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clockwork_core::asset::{self, AssetSource};
use clockwork_core::image::RgbaImage;
use clockwork_core::manifest::{ManifestError, PluginManifest};
use clockwork_core::model::Model;
use clockwork_core::place::{self, Placement, SceneTemplate};
use clockwork_core::recipe::{Codec, Output, Recipe, Rect, Step, parse_hex};
use clockwork_core::sprite::Sprite;
use clockwork_core::surface::{CodeMeaning, SurfaceError, SurfaceMap};
use clockwork_core::terrain::{Collision, Terrain, TerrainError};
use clockwork_gba::gfx::{PaletteBank, render_metatile_map, render_metatiles};
use clockwork_gba::lz77;

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("step {step}: offset {offset:#x} is outside the ROM")]
    OutOfRange { step: usize, offset: u32 },
    #[error("step {step}: uncompressed reads need a size")]
    MissingSize { step: usize },
    #[error("step {step}: {source}")]
    Lz77 {
        step: usize,
        source: lz77::Lz77Error,
    },
    #[error("step {step}: unknown {kind} \"{name}\"")]
    Unknown {
        step: usize,
        kind: &'static str,
        name: String,
    },
    #[error("step {step}: index {index} is outside table \"{table}\"")]
    LookupOutOfRange {
        step: usize,
        index: u16,
        table: String,
    },
    #[error("step {step}: {message}")]
    Literal { step: usize, message: String },
    #[error("step {step}: {message}")]
    Decode { step: usize, message: String },
    #[error("step {step}: bone \"{bone}\" has unknown parent \"{parent}\"")]
    UnknownParent {
        step: usize,
        bone: String,
        parent: String,
    },
    #[error("step {step}: {what} of {size} is over the limit of {max}")]
    TooLarge {
        step: usize,
        what: &'static str,
        size: u64,
        max: u64,
    },
    #[error("output \"{0}\": unknown image")]
    UnknownOutput(String),
    #[error("output \"{id}\": {message}")]
    Part { id: String, message: String },
    #[error(
        "output \"{id}\": the character uses animation \"{name}\", which the sprite doesn't have"
    )]
    UnknownAnimation { id: String, name: String },
    #[error("output \"{id}\": unknown buffer \"{buffer}\"")]
    UnknownGrid { id: String, buffer: String },
    #[error("output \"{id}\": {source}")]
    Surfaces { id: String, source: SurfaceError },
    #[error("output \"{id}\": {message}")]
    Character { id: String, message: String },
    #[error("output \"{id}\": unknown collision \"{name}\"")]
    UnknownCollision { id: String, name: String },
    #[error("output \"{id}\": {source}")]
    Terrain { id: String, source: TerrainError },
}

impl RecipeError {
    /// The step that failed, for errors in a step.
    pub fn step(&self) -> Option<usize> {
        match self {
            RecipeError::OutOfRange { step, .. }
            | RecipeError::MissingSize { step }
            | RecipeError::TooLarge { step, .. }
            | RecipeError::Lz77 { step, .. }
            | RecipeError::Unknown { step, .. }
            | RecipeError::LookupOutOfRange { step, .. }
            | RecipeError::Literal { step, .. }
            | RecipeError::Decode { step, .. }
            | RecipeError::UnknownParent { step, .. } => Some(*step),
            _ => None,
        }
    }

    /// The id of the output that failed, for errors in an output.
    pub fn output(&self) -> Option<&str> {
        match self {
            RecipeError::UnknownAnimation { id, .. }
            | RecipeError::UnknownGrid { id, .. }
            | RecipeError::Surfaces { id, .. }
            | RecipeError::Character { id, .. }
            | RecipeError::UnknownCollision { id, .. }
            | RecipeError::Terrain { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// What a recipe produced.
#[derive(Debug, Default)]
pub struct Extracted {
    /// Output images by id, in the order the recipe lists them.
    pub images: Vec<(String, RgbaImage)>,
    /// Output surface maps by id, in the order the recipe lists them.
    pub surfaces: Vec<(String, SurfaceMap)>,
    /// Output models, in the order the recipe lists them.
    pub models: Vec<ExtractedModel>,
    /// Output sprites, in the order the recipe lists them.
    pub sprites: Vec<ExtractedSprite>,
    /// Output item lists, by id, in the order the recipe lists them.
    pub items: Vec<(String, Vec<clockwork_core::item::ItemDef>)>,
    /// Abilities other characters can learn.
    pub abilities: Vec<clockwork_core::ability::Ability>,
    /// Output fonts by id.
    pub fonts: Vec<(String, clockwork_core::font::BitmapFont)>,
    /// Output sounds by id.
    pub sounds: Vec<(String, clockwork_core::sound::Pcm)>,
    /// Output dialogs whose pages come from extracted text, by id.
    pub dialogs: Vec<(String, clockwork_core::ui::Dialog)>,
    /// Output scenes by id: nodes to add to the Workshop's scene.
    pub scenes: Vec<(String, SceneTemplate)>,
    /// The image each surface map says it covers: (surface map id, image id).
    pub surface_images: Vec<(String, String)>,
}

/// Text from `bytes` starting at `at` up to the `end` byte (or `length` bytes): each byte the
/// text `codes` gives it, else its character in a run of `runs`, else `other`.
fn decode_text(
    bytes: &[u8],
    at: u32,
    runs: &[(u8, String)],
    codes: &std::collections::BTreeMap<u8, String>,
    (end, length): (u8, Option<u32>),
    other: Option<&str>,
) -> Result<String, String> {
    let mut text = String::new();
    let mut i = at as usize;
    loop {
        if length.is_some_and(|n| i >= at as usize + n as usize) {
            return Ok(text);
        }
        let Some(&byte) = bytes.get(i) else {
            return Err(format!("no end byte {end:#04x} after {at:#x}"));
        };
        if byte == end {
            return Ok(text);
        }
        if let Some(code) = codes.get(&byte) {
            text.push_str(code);
        } else if let Some(c) = runs.iter().find_map(|(first, chars)| {
            byte.checked_sub(*first)
                .and_then(|k| chars.chars().nth(k as usize))
        }) {
            text.push(c);
        } else if let Some(other) = other {
            text.push_str(other);
        } else {
            return Err(format!("byte {byte:#04x} at {i:#x} means nothing"));
        }
        i += 1;
    }
}

/// A model output.
#[derive(Debug, Clone)]
pub struct ExtractedModel {
    pub id: String,
    pub model: Model,
    pub meters_per_unit: f32,
    /// How it moves when played, if it is a playable character.
    pub character: Option<clockwork_core::character::Character>,
    /// What a character can walk on, if the model is an area.
    pub terrain: Option<Terrain>,
}

/// A sprite output.
#[derive(Debug, Clone)]
pub struct ExtractedSprite {
    pub id: String,
    pub sprite: Sprite,
    pub pixels_per_meter: f32,
    /// How it moves when played, if it is a playable character.
    pub character: Option<clockwork_core::character2d::Character2d>,
}

impl Extracted {
    pub fn sprite(&self, id: &str) -> Option<&ExtractedSprite> {
        self.sprites.iter().find(|s| s.id == id)
    }

    pub fn model(&self, id: &str) -> Option<&ExtractedModel> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The picture of an item icon, cropped to what it shows.
    pub fn icon(&self, icon: &clockwork_core::item::Icon) -> Option<RgbaImage> {
        use clockwork_core::item::Icon;
        let image = match icon {
            Icon::Frame {
                sprite,
                frame,
                flip_x,
            } => self
                .sprite(sprite)?
                .sprite
                .frames
                .get(*frame)?
                .image
                .flipped(*flip_x, false),
            Icon::Animation { sprite, animation } => {
                let sprite = &self.sprite(sprite)?.sprite;
                let step = sprite.animation(animation)?.steps.first()?;
                sprite
                    .frames
                    .get(step.frame)?
                    .image
                    .flipped(step.flip_x, step.flip_y)
            }
            Icon::Image(id) => self.images.iter().find(|(i, _)| i == id)?.1.clone(),
        };
        Some(preview::trim(&image))
    }
}

/// What a recipe has made so far, as [`run_observed`] shows it after each step. Kept (cloned)
/// after a step, it is where [`run_from`] goes on from.
#[derive(Debug, Clone, Default)]
pub struct Made {
    buffers: HashMap<String, Vec<u8>>,
    banks: HashMap<String, PaletteBank>,
    images: HashMap<String, RgbaImage>,
    models: HashMap<String, Model>,
    collisions: HashMap<String, Collision>,
    sprites: HashMap<String, Sprite>,
    places: HashMap<String, Vec<Placement>>,
}

impl Made {
    pub fn buffer(&self, name: &str) -> Option<&[u8]> {
        self.buffers.get(name).map(Vec::as_slice)
    }

    pub fn has_bank(&self, name: &str) -> bool {
        self.banks.contains_key(name)
    }

    pub fn image(&self, name: &str) -> Option<&RgbaImage> {
        self.images.get(name)
    }

    pub fn model(&self, name: &str) -> Option<&Model> {
        self.models.get(name)
    }

    pub fn collision(&self, name: &str) -> Option<&Collision> {
        self.collisions.get(name)
    }

    pub fn sprite(&self, name: &str) -> Option<&Sprite> {
        self.sprites.get(name)
    }

    pub fn places(&self, name: &str) -> Option<&[Placement]> {
        self.places.get(name).map(Vec::as_slice)
    }

    /// About how much memory its buffers and images take, in bytes, to decide whether keeping
    /// a copy is worth it.
    pub fn bytes(&self) -> usize {
        let buffers: usize = self.buffers.values().map(Vec::len).sum();
        let images: usize = self.images.values().map(|i| i.pixels.len()).sum();
        let sprites: usize = (self.sprites.values())
            .flat_map(|s| &s.frames)
            .map(|f| f.image.pixels.len())
            .sum();
        buffers + images + sprites
    }
}

/// Runs a recipe on its own: the parts its characters name must be built in or written in place.
/// The largest buffer a `Fill` step makes, in bytes. Plugins are shared, so a recipe's numbers
/// are checked before memory is set aside for them.
pub const MAX_FILL: u64 = 64 * 1024 * 1024;
/// The most pixels a `Canvas` step makes (4096 x 4096).
pub const MAX_CANVAS_PIXELS: u64 = 4096 * 4096;

fn within(step: usize, what: &'static str, size: u64, max: u64) -> Result<(), RecipeError> {
    if size > max {
        return Err(RecipeError::TooLarge {
            step,
            what,
            size,
            max,
        });
    }
    Ok(())
}

pub fn run(recipe: &Recipe, rom: &[u8]) -> Result<Extracted, RecipeError> {
    run_in(recipe, rom, &asset::Builtins)
}

/// Runs a recipe whose characters' parts are found in `assets`.
pub fn run_in(
    recipe: &Recipe,
    rom: &[u8],
    assets: &dyn AssetSource,
) -> Result<Extracted, RecipeError> {
    run_observed(recipe, rom, assets, |_, _| {})
}

/// Runs a recipe, showing `observe` what has been made after every step (by step index).
pub fn run_observed(
    recipe: &Recipe,
    rom: &[u8],
    assets: &dyn AssetSource,
    observe: impl FnMut(usize, &Made),
) -> Result<Extracted, RecipeError> {
    run_from(recipe, rom, assets, Made::default(), 0, observe)
}

/// Runs a recipe from step `from` on, given what the steps before it made (as [`run_observed`]
/// showed it after step `from - 1`), so a change to step `from` or later runs only from there.
pub fn run_from(
    recipe: &Recipe,
    rom: &[u8],
    assets: &dyn AssetSource,
    mut made: Made,
    from: usize,
    mut observe: impl FnMut(usize, &Made),
) -> Result<Extracted, RecipeError> {
    for (step, s) in recipe.steps.iter().enumerate().skip(from) {
        run_step(step, s, rom, &mut made)?;
        observe(step, &made);
    }
    outputs(recipe, &made, assets)
}

/// Runs one step on what was made before it.
fn run_step(step: usize, s: &Step, rom: &[u8], made: &mut Made) -> Result<(), RecipeError> {
    let Made {
        buffers,
        banks,
        images,
        models,
        collisions,
        sprites,
        places,
    } = made;
    {
        let unknown = |kind, name: &str| RecipeError::Unknown {
            step,
            kind,
            name: name.to_string(),
        };
        match s {
            Step::Read {
                name,
                offset,
                size,
                codec,
            } => {
                let data = rom.get(*offset as usize..).ok_or(RecipeError::OutOfRange {
                    step,
                    offset: *offset,
                })?;
                let bytes = match codec {
                    Codec::GbaLz77 => lz77::decompress(data)
                        .map_err(|source| RecipeError::Lz77 { step, source })?,
                    Codec::Popuri => clockwork_gba::popuri::decompress(data)
                        .map_err(|message| RecipeError::Decode { step, message })?,
                    Codec::Mio0 => mio0_decompress(data)
                        .map_err(|message| RecipeError::Decode { step, message })?,
                    Codec::None => {
                        let size = size.ok_or(RecipeError::MissingSize { step })? as usize;
                        data.get(..size)
                            .ok_or(RecipeError::OutOfRange {
                                step,
                                offset: *offset,
                            })?
                            .to_vec()
                    }
                };
                buffers.insert(name.clone(), bytes);
            }
            Step::Fill { name, size, value } => {
                within(step, "a filled buffer", u64::from(*size), MAX_FILL)?;
                buffers.insert(name.clone(), vec![*value; *size as usize]);
            }
            Step::Literal { name, hex } => {
                let bytes =
                    parse_hex(hex).map_err(|message| RecipeError::Literal { step, message })?;
                buffers.insert(name.clone(), bytes);
            }
            Step::Crop { from, region, into } => {
                let image = images
                    .get(from.as_str())
                    .ok_or_else(|| unknown("image", from))?;
                let cropped = image.crop(region.x, region.y, region.w, region.h);
                images.insert(into.clone(), cropped);
            }
            Step::Text {
                from,
                at,
                runs,
                codes,
                end,
                length,
                other,
                into,
            } => {
                let bytes = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let text = decode_text(bytes, *at, runs, codes, (*end, *length), other.as_deref())
                    .map_err(|message| RecipeError::Decode { step, message })?;
                buffers.insert(into.clone(), text.into_bytes());
            }
            Step::Canvas {
                name,
                width,
                height,
            } => {
                let pixels = u64::from(*width) * u64::from(*height);
                within(step, "a canvas", pixels, MAX_CANVAS_PIXELS)?;
                images.insert(name.clone(), RgbaImage::new(*width, *height));
            }
            Step::Paste { from, into, at } => {
                let source = images
                    .get(from.as_str())
                    .ok_or_else(|| unknown("image", from))?
                    .clone();
                let target = images
                    .get_mut(into.as_str())
                    .ok_or_else(|| unknown("image", into))?;
                for y in 0..source.height {
                    for x in 0..source.width {
                        target.set(at.0 + x, at.1 + y, source.get(x, y));
                    }
                }
            }
            Step::N64Texture {
                from,
                at,
                format,
                width,
                height,
                palette,
                into,
            } => {
                use clockwork_core::recipe::N64TextureFormat as F;
                use clockwork_n64::texture::{TexFormat, TexSize};
                let (fmt, size) = match format {
                    F::Rgba16 => (TexFormat::Rgba, TexSize::B16),
                    F::Rgba32 => (TexFormat::Rgba, TexSize::B32),
                    F::Ia4 => (TexFormat::Ia, TexSize::B4),
                    F::Ia8 => (TexFormat::Ia, TexSize::B8),
                    F::Ia16 => (TexFormat::Ia, TexSize::B16),
                    F::I4 => (TexFormat::I, TexSize::B4),
                    F::I8 => (TexFormat::I, TexSize::B8),
                    F::Ci4 => (TexFormat::Ci, TexSize::B4),
                    F::Ci8 => (TexFormat::Ci, TexSize::B8),
                };
                let data = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let texels = data.get(*at as usize..).unwrap_or_default();
                let palette = match palette {
                    Some((buffer, offset)) => Some(
                        buffers
                            .get(buffer.as_str())
                            .ok_or_else(|| unknown("buffer", buffer))?
                            .get(*offset as usize..)
                            .unwrap_or_default(),
                    ),
                    None => None,
                };
                let image = clockwork_n64::texture::decode(fmt, size, texels, *width, *height, palette)
                    .ok_or_else(|| RecipeError::Decode {
                        step,
                        message: format!(
                            "{width}x{height} {format:?} texels don't fit at {at:#x} of \"{from}\" (or the palette is short)"
                        ),
                    })?;
                images.insert(into.clone(), image);
            }
            Step::Place { from, into, at } => {
                let src = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?
                    .clone();
                let dst = buffers.entry(into.clone()).or_default();
                let end = *at as usize + src.len();
                if dst.len() < end {
                    dst.resize(end, 0);
                }
                dst[*at as usize..end].copy_from_slice(&src);
            }
            Step::GbaPalettes { from, bank, slot } => {
                let data = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                banks
                    .entry(bank.clone())
                    .or_default()
                    .load(data, *slot as usize);
            }
            Step::GbaMetatiles {
                tiles,
                char_base,
                metatiles,
                bank,
                columns,
                into,
                depth,
            } => {
                let tiles = buffers
                    .get(tiles.as_str())
                    .ok_or_else(|| unknown("buffer", tiles))?;
                let tiles = tiles.get(*char_base as usize..).unwrap_or(&[]);
                let meta = buffers
                    .get(metatiles.as_str())
                    .ok_or_else(|| unknown("buffer", metatiles))?;
                let bank = banks
                    .get(bank.as_str())
                    .ok_or_else(|| unknown("palette bank", bank))?;
                images.insert(
                    into.clone(),
                    render_metatiles(tiles, meta, bank, *columns, *depth),
                );
            }
            Step::GbaMetatileMap {
                tiles,
                char_base,
                metatiles,
                map,
                width,
                bank,
                into,
                depth,
            } => {
                let tiles = buffers
                    .get(tiles.as_str())
                    .ok_or_else(|| unknown("buffer", tiles))?;
                let tiles = tiles.get(*char_base as usize..).unwrap_or(&[]);
                let meta = buffers
                    .get(metatiles.as_str())
                    .ok_or_else(|| unknown("buffer", metatiles))?;
                let map = buffers
                    .get(map.as_str())
                    .ok_or_else(|| unknown("buffer", map))?;
                let bank = banks
                    .get(bank.as_str())
                    .ok_or_else(|| unknown("palette bank", bank))?;
                images.insert(
                    into.clone(),
                    render_metatile_map(tiles, meta, map, *width, bank, *depth),
                );
            }
            Step::Lookup {
                indices,
                index,
                table,
                value,
                into,
            } => {
                let idx = buffers
                    .get(indices.as_str())
                    .ok_or_else(|| unknown("buffer", indices))?;
                let tab = buffers
                    .get(table.as_str())
                    .ok_or_else(|| unknown("buffer", table))?;
                let values = value.read_all(tab);
                let mut out = Vec::new();
                for i in index.read_all(idx) {
                    let v =
                        *values
                            .get(i as usize)
                            .ok_or_else(|| RecipeError::LookupOutOfRange {
                                step,
                                index: i,
                                table: table.clone(),
                            })?;
                    value.write(&mut out, v);
                }
                buffers.insert(into.clone(), out);
            }
            Step::Places {
                from,
                at,
                stride,
                count,
                until,
                when,
                x,
                y,
                size,
                origin,
                item,
                name,
                group,
                door,
                code,
                extent,
                into,
            } => {
                let bytes = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let short = |what: &str, index: usize| RecipeError::Decode {
                    step,
                    message: format!("record {index} of \"{from}\" is too short for its {what}"),
                };
                if count.is_none() && until.is_none() {
                    return Err(RecipeError::Decode {
                        step,
                        message: "a Places step needs a count or an until".into(),
                    });
                }
                let limit = count.map_or(usize::MAX, |n| n as usize);
                let list = places.entry(into.clone()).or_default();
                let mut index = 0;
                while index < limit {
                    let start = *at as usize + index * (*stride).max(1) as usize;
                    let Some(record) = bytes.get(start..start + (*stride).max(1) as usize) else {
                        if until.is_some() && count.is_none() {
                            return Err(RecipeError::Decode {
                                step,
                                message: format!(
                                    "the records of \"{from}\" end before one matches until"
                                ),
                            });
                        }
                        return Err(short("record", index));
                    };
                    if until.as_ref().is_some_and(|u| u.holds(record)) {
                        break;
                    }
                    if when.iter().all(|m| m.holds(record)) {
                        let mut px = x.read(record).ok_or_else(|| short("x", index))? as f32;
                        let mut py = y.read(record).ok_or_else(|| short("y", index))? as f32;
                        if let Some((w, h)) = size {
                            px += w.read(record).ok_or_else(|| short("width", index))? as f32 / 2.0;
                            py +=
                                h.read(record).ok_or_else(|| short("height", index))? as f32 / 2.0;
                        }
                        let shown = item.as_ref().and_then(|i| i.pick(record, index));
                        let fill = |t: &clockwork_core::place::Template| {
                            t.fill(record, shown, index)
                                .ok_or_else(|| short("name", index))
                        };
                        let half = extent.as_ref().map_or(Ok((0, 0)), |e| {
                            e.by.read(record)
                                .map(|v| e.sizes.get(&v).copied().unwrap_or(e.other))
                                .ok_or_else(|| short("extent", index))
                        })?;
                        list.push(Placement {
                            name: fill(name)?,
                            group: group.as_deref().map(fill).transpose()?,
                            item: shown.map(str::to_string),
                            x: px + origin.0 as f32,
                            y: py + origin.1 as f32,
                            door: door
                                .as_ref()
                                .map(|d| {
                                    Ok::<_, RecipeError>(place::Door {
                                        to: fill(&d.to)?,
                                        radius: d.radius,
                                        fade: d.fade,
                                    })
                                })
                                .transpose()?,
                            code: match code {
                                Some(field) => {
                                    field.read(record).ok_or_else(|| short("code", index))?
                                }
                                None => index as i64 + 1,
                            },
                            half,
                        });
                    }
                    index += 1;
                }
            }
            Step::MarkPlaces {
                from,
                width,
                height,
                cell_size,
                origin,
                into,
            } => {
                let list = places
                    .get(from.as_str())
                    .ok_or_else(|| unknown("placement list", from))?;
                let cells = *width as u64 * *height as u64;
                within(step, "a grid", cells, MAX_FILL)?;
                let grid = buffers.entry(into.clone()).or_default();
                if (grid.len() as u64) < cells {
                    grid.resize(cells as usize, 0);
                }
                place::mark(grid, list, (*width, *height), *cell_size, *origin);
            }
            Step::Overlay {
                from,
                into,
                region,
                replace,
            } => {
                let top = images
                    .get(from.as_str())
                    .ok_or_else(|| unknown("image", from))?;
                let r = region.unwrap_or(Rect {
                    x: 0,
                    y: 0,
                    w: top.width,
                    h: top.height,
                });
                let part = top.crop(r.x, r.y, r.w, r.h);
                let base = images
                    .get_mut(into.as_str())
                    .ok_or_else(|| unknown("image", into))?;
                if *replace {
                    base.paste(&part, r.x, r.y);
                } else {
                    base.overlay(&part, r.x, r.y);
                }
            }
            Step::N64Model {
                microcode: _,
                segments,
                bones,
                into,
            } => {
                let model = n64_model(step, buffers, segments, bones)?;
                models.insert(into.clone(), model);
            }
            Step::CollisionTriangles {
                from,
                at,
                layout,
                into,
            } => {
                let data = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let collision = collision::parse(data, *at as usize, layout).map_err(|e| {
                    RecipeError::Decode {
                        step,
                        message: e.to_string(),
                    }
                })?;
                collisions.insert(into.clone(), collision);
            }
            Step::ChannelAnimation {
                from,
                name,
                index,
                values,
                frames,
                loop_start,
                fps,
                root_translation,
                bones,
                into,
            } => {
                let data = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let model = models
                    .get_mut(into.as_str())
                    .ok_or_else(|| unknown("model", into))?;
                let layout = anim::Layout {
                    index: *index as usize,
                    values: *values as usize,
                    frames: *frames,
                    bones: bones.map_or(model.bones.len(), |n| (n as usize).min(model.bones.len())),
                    root_translation: *root_translation,
                };
                let tracks = anim::decode(data, &layout).ok_or_else(|| RecipeError::Decode {
                    step,
                    message: format!("animation tables run past buffer \"{from}\""),
                })?;
                model
                    .animations
                    .push(anim::animation(name, *frames, *fps, *loop_start, tracks));
            }
            Step::GbaObjectFrames {
                tiles,
                bank,
                objects,
                layout,
                frames,
                into,
            } => {
                let built = sprite::object_frames(
                    step, buffers, banks, tiles, bank, objects, layout, frames,
                )?;
                sprites.insert(into.clone(), built);
            }
            Step::MergeSprite { from, into } => {
                let source = sprites
                    .get(from.as_str())
                    .cloned()
                    .ok_or_else(|| unknown("sprite", from))?;
                let target = sprites
                    .get_mut(into.as_str())
                    .ok_or_else(|| unknown("sprite", into))?;
                if let Some(name) = source
                    .animations
                    .iter()
                    .find(|a| target.animation(&a.name).is_some())
                    .map(|a| a.name.clone())
                {
                    return Err(RecipeError::Decode {
                        step,
                        message: format!("both sprites have an animation called \"{name}\""),
                    });
                }
                let offset = target.frames.len();
                target.frames.extend(source.frames);
                for mut animation in source.animations {
                    for s in &mut animation.steps {
                        s.frame += offset;
                    }
                    target.animations.push(animation);
                }
            }
            Step::KeyframeAnimation {
                from,
                name,
                at,
                layout,
                fps,
                into,
            } => {
                let data = buffers
                    .get(from.as_str())
                    .ok_or_else(|| unknown("buffer", from))?;
                let target = sprites
                    .get_mut(into.as_str())
                    .ok_or_else(|| unknown("sprite", into))?;
                let animation =
                    sprite::keyframe_animation(step, data, target, name, at, layout, *fps)?;
                target.animations.push(animation);
            }
        }
    }
    Ok(())
}

/// What a recipe's outputs are, from what its steps made.
fn outputs(
    recipe: &Recipe,
    made: &Made,
    assets: &dyn AssetSource,
) -> Result<Extracted, RecipeError> {
    let part = |id: &String| {
        let id = id.clone();
        move |e: asset::ResolveError| RecipeError::Part {
            id,
            message: e.to_string(),
        }
    };
    let Made {
        buffers,
        images,
        models,
        collisions,
        sprites,
        ..
    } = made;
    let mut extracted = Extracted::default();
    for output in &recipe.outputs {
        match output {
            Output::Image { id, from, .. } => {
                let image = images
                    .get(from.as_str())
                    .cloned()
                    .ok_or_else(|| RecipeError::UnknownOutput(from.clone()))?;
                extracted.images.push((id.clone(), image));
            }
            Output::Surfaces {
                id,
                width,
                cell_size,
                grids,
                extensions,
                image,
            } => {
                if let Some(image) = image {
                    extracted.surface_images.push((id.clone(), image.clone()));
                }
                let mut codes = Vec::new();
                for grid in grids {
                    let bytes = buffers.get(grid.from.as_str()).ok_or_else(|| {
                        RecipeError::UnknownGrid {
                            id: id.clone(),
                            buffer: grid.from.clone(),
                        }
                    })?;
                    let meaning = CodeMeaning {
                        codes: grid.codes.clone(),
                        other: grid.other.clone(),
                    };
                    codes.push((grid.name.as_str(), grid.element.read_all(bytes), meaning));
                }
                let grids: Vec<_> = codes
                    .iter()
                    .map(|(name, values, meaning)| (*name, values.as_slice(), meaning))
                    .collect();
                let map = SurfaceMap::build(*width, *cell_size, &grids, extensions.clone())
                    .map_err(|source| RecipeError::Surfaces {
                        id: id.clone(),
                        source,
                    })?;
                extracted.surfaces.push((id.clone(), map));
            }
            Output::Model {
                id,
                from,
                meters_per_unit,
                character,
                terrain,
                ..
            } => {
                let model = models
                    .get(from.as_str())
                    .cloned()
                    .ok_or_else(|| RecipeError::UnknownOutput(from.clone()))?;
                let terrain = terrain
                    .as_deref()
                    .map(|spec| {
                        let collision = collisions.get(spec.from.as_str()).ok_or_else(|| {
                            RecipeError::UnknownCollision {
                                id: id.clone(),
                                name: spec.from.clone(),
                            }
                        })?;
                        let meaning = CodeMeaning {
                            codes: spec.codes.clone(),
                            other: spec.other.clone(),
                        };
                        Terrain::build(
                            collision,
                            &meaning,
                            spec.extensions.clone(),
                            spec.floor_normal_y,
                            spec.spawn,
                        )
                        .map_err(|source| RecipeError::Terrain {
                            id: id.clone(),
                            source,
                        })
                    })
                    .transpose()?;
                let character = character
                    .as_deref()
                    .map(|c| c.resolve(assets).map_err(part(id)))
                    .transpose()?;
                extracted.models.push(ExtractedModel {
                    id: id.clone(),
                    model,
                    meters_per_unit: *meters_per_unit,
                    character,
                    terrain,
                });
            }
            Output::Items { id, items } => {
                extracted.items.push((id.clone(), items.clone()));
            }
            // Built once every character and item exists (below).
            Output::Ability { .. } => {}
            // Settings, not graphics: read from the recipe where they are named.
            Output::Camera { .. }
            | Output::Hud { .. }
            | Output::Ui { .. }
            | Output::Menu { .. }
            | Output::Flow { .. }
            | Output::InputMap { .. }
            | Output::Movement { .. }
            | Output::Animations { .. }
            | Output::Stats { .. }
            | Output::Behavior { .. }
            | Output::Rules { .. }
            | Output::Game { .. }
            | Output::Dialog { from: None, .. }
            | Output::Sounds { .. } => {}
            Output::Dialog {
                id,
                dialog,
                from: Some(from),
                lines_per_page,
            } => {
                let bytes = buffers
                    .get(from.as_str())
                    .ok_or_else(|| RecipeError::Part {
                        id: id.clone(),
                        message: format!("unknown buffer \"{from}\""),
                    })?;
                let text = String::from_utf8_lossy(bytes);
                let speaker = match dialog {
                    clockwork_core::asset::Slot::Inline(d) => d.speaker.clone(),
                    clockwork_core::asset::Slot::Ref(_) => None,
                };
                extracted.dialogs.push((
                    id.clone(),
                    clockwork_core::ui::Dialog {
                        speaker,
                        pages: clockwork_core::recipe::dialog_pages(&text, *lines_per_page),
                    },
                ));
            }
            Output::Font {
                id,
                from,
                glyph,
                chars,
                advance,
            } => {
                let image = images
                    .get(from.as_str())
                    .cloned()
                    .ok_or_else(|| RecipeError::UnknownOutput(from.clone()))?;
                extracted.fonts.push((
                    id.clone(),
                    clockwork_core::font::BitmapFont {
                        image,
                        glyph: *glyph,
                        chars: chars.clone(),
                        advance: advance.clone(),
                    },
                ));
            }
            Output::Sound {
                id,
                from,
                rate,
                format,
            } => {
                let bytes = buffers
                    .get(from.as_str())
                    .ok_or_else(|| RecipeError::Part {
                        id: id.clone(),
                        message: format!("unknown buffer \"{from}\""),
                    })?;
                extracted.sounds.push((
                    id.clone(),
                    clockwork_core::sound::Pcm::decode(bytes, *format, *rate),
                ));
            }
            Output::Scene {
                id,
                from,
                pixels_per_meter,
            } => {
                let mut lists = Vec::new();
                for name in from {
                    lists.push(made.places.get(name.as_str()).ok_or_else(|| {
                        RecipeError::Part {
                            id: id.clone(),
                            message: format!("unknown placement list \"{name}\""),
                        }
                    })?);
                }
                let scene =
                    SceneTemplate::from_placements(lists.into_iter().flatten(), *pixels_per_meter);
                let shows = |item: &str| {
                    recipe
                        .outputs
                        .iter()
                        .any(|o| o.item().is_some_and(|(i, _)| i == item))
                };
                if let Some(node) = scene
                    .nodes
                    .iter()
                    .find(|n| n.item.as_deref().is_some_and(|i| !shows(i)))
                {
                    return Err(RecipeError::Part {
                        id: id.clone(),
                        message: format!(
                            "node \"{}\" shows \"{}\", which is not an image, model or sprite of this plugin",
                            node.name,
                            node.item.as_deref().unwrap_or_default()
                        ),
                    });
                }
                extracted.scenes.push((id.clone(), scene));
            }
            Output::Sprite {
                id,
                from,
                pixels_per_meter,
                character,
                ..
            } => {
                let sprite = sprites
                    .get(from.as_str())
                    .cloned()
                    .ok_or_else(|| RecipeError::UnknownOutput(from.clone()))?;
                let character = character
                    .as_deref()
                    .map(|c| c.resolve(assets).map_err(part(id)))
                    .transpose()?;
                if let Some(character) = &character {
                    check_character(id, &sprite, character)?;
                }
                extracted.sprites.push(ExtractedSprite {
                    id: id.clone(),
                    sprite,
                    pixels_per_meter: *pixels_per_meter,
                    character,
                });
            }
        }
    }
    check_references(&extracted)?;
    for output in &recipe.outputs {
        if let Output::Ability { .. } = output {
            let ability = ability::build(output, &extracted)?;
            extracted.abilities.push(ability);
        }
    }
    Ok(extracted)
}

/// What characters and items show from other outputs (held items, projectiles, icons) must exist.
fn check_references(extracted: &Extracted) -> Result<(), RecipeError> {
    use clockwork_core::action::AttachmentFrames;
    use clockwork_core::character2d::Directional;
    use clockwork_core::item::Icon;

    let sprite = |id: &str, owner: &str, what: &str| {
        extracted
            .sprite(id)
            .map(|s| &s.sprite)
            .ok_or_else(|| RecipeError::Character {
                id: owner.to_string(),
                message: format!("{what} uses sprite \"{id}\", which the recipe doesn't output"),
            })
    };
    let animations = |sprite: &Sprite, directional: &Directional, owner: &str, what: &str| {
        for reference in [
            &directional.north,
            &directional.east,
            &directional.south,
            &directional.west,
        ] {
            if sprite.animation(&reference.name).is_none() {
                return Err(RecipeError::Character {
                    id: owner.to_string(),
                    message: format!("{what} uses unknown animation \"{}\"", reference.name),
                });
            }
        }
        Ok(())
    };
    for s in &extracted.sprites {
        let Some(character) = &s.character else {
            continue;
        };
        for action in &character.actions {
            for attachment in &action.attachments {
                let what = format!("action \"{}\"", action.id);
                let target = sprite(&attachment.sprite, &s.id, &what)?;
                if let AttachmentFrames::Animation(directional) = &attachment.frames {
                    animations(target, directional, &s.id, &what)?;
                }
            }
        }
        for projectile in &character.projectiles {
            if let Some(look) = &projectile.sprite {
                let what = format!("projectile \"{}\"", projectile.id);
                let target = sprite(&look.sprite, &s.id, &what)?;
                animations(target, &look.animation, &s.id, &what)?;
                if let Some(burst) = &look.burst
                    && target.animation(burst).is_none()
                {
                    return Err(RecipeError::Character {
                        id: s.id.clone(),
                        message: format!("{what} bursts with unknown animation \"{burst}\""),
                    });
                }
            }
        }
    }
    for (id, items) in &extracted.items {
        for item in items {
            let what = format!("item \"{}\"", item.id);
            match &item.icon {
                Some(Icon::Frame {
                    sprite: name,
                    frame,
                    ..
                }) => {
                    let target = sprite(name, id, &what)?;
                    if *frame >= target.frames.len() {
                        return Err(RecipeError::Character {
                            id: id.clone(),
                            message: format!(
                                "{what}'s icon is frame {frame} of \"{name}\", which has {}",
                                target.frames.len()
                            ),
                        });
                    }
                }
                Some(Icon::Animation {
                    sprite: name,
                    animation,
                }) => {
                    let target = sprite(name, id, &what)?;
                    if target.animation(animation).is_none() {
                        return Err(RecipeError::Character {
                            id: id.clone(),
                            message: format!(
                                "{what}'s icon is unknown animation \"{animation}\" of \"{name}\""
                            ),
                        });
                    }
                }
                Some(Icon::Image(name)) if !extracted.images.iter().any(|(i, _)| i == name) => {
                    return Err(RecipeError::Character {
                        id: id.clone(),
                        message: format!("{what}'s icon is unknown image \"{name}\""),
                    });
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// A character's animations must exist, or Play mode would show nothing.
fn check_character(
    id: &str,
    sprite: &Sprite,
    character: &clockwork_core::character2d::Character2d,
) -> Result<(), RecipeError> {
    use clockwork_core::character2d::Directional;
    let fail = |message: String| RecipeError::Character {
        id: id.to_string(),
        message,
    };
    let check = |directional: &Directional| -> Result<(), RecipeError> {
        for reference in [
            &directional.north,
            &directional.east,
            &directional.south,
            &directional.west,
        ] {
            if sprite.animation(&reference.name).is_none() {
                return Err(RecipeError::UnknownAnimation {
                    id: id.to_string(),
                    name: reference.name.clone(),
                });
            }
        }
        Ok(())
    };
    let animations = &character.animations;
    check(&animations.idle)?;
    check(&animations.walk)?;

    let has_action = |name: &str| character.actions.iter().any(|a| a.id == name);
    for (i, action) in character.actions.iter().enumerate() {
        if character.actions[..i].iter().any(|a| a.id == action.id) {
            return Err(fail(format!("two actions are called \"{}\"", action.id)));
        }
        check(&action.animation)?;
        if let Some(hold) = &action.hold {
            check(&hold.animation)?;
            if let Some(walk) = &hold.walk_animation {
                check(walk)?;
            }
            if let Some(release) = &hold.release
                && !has_action(&release.action)
            {
                return Err(fail(format!(
                    "action \"{}\" releases into unknown action \"{}\"",
                    action.id, release.action
                )));
            }
        }
        for spawn in &action.spawns {
            if !character
                .projectiles
                .iter()
                .any(|p| p.id == spawn.projectile)
            {
                return Err(fail(format!(
                    "action \"{}\" spawns unknown projectile \"{}\"",
                    action.id, spawn.projectile
                )));
            }
        }
    }
    if let Some(dodge) = &character.dodge
        && !has_action(dodge)
    {
        return Err(fail(format!("the dodge is unknown action \"{dodge}\"")));
    }
    Ok(())
}

fn mio0_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    clockwork_n64::mio0::decompress(data).map_err(|e| e.to_string())
}

/// Builds a model from display lists, one bone at a time. Drawing state (lights, textures)
/// carries over between lists, as on the console.
fn n64_model(
    step: usize,
    buffers: &HashMap<String, Vec<u8>>,
    segments: &std::collections::BTreeMap<u8, String>,
    bones: &[clockwork_core::recipe::ModelBone],
) -> Result<Model, RecipeError> {
    use clockwork_n64::f3d::{G_LIGHTING, Memory, ModelBuilder};

    let mut memory = Memory::new();
    for (segment, name) in segments {
        let data = buffers
            .get(name.as_str())
            .ok_or_else(|| RecipeError::Unknown {
                step,
                kind: "buffer",
                name: name.clone(),
            })?;
        memory.set(*segment, data);
    }
    let mut builder = ModelBuilder::new(memory).with_geometry_mode(G_LIGHTING);
    for (index, bone) in bones.iter().enumerate() {
        let parent = match &bone.parent {
            None => None,
            Some(name) => Some(
                bones[..index]
                    .iter()
                    .position(|b| b.name == *name)
                    .ok_or_else(|| RecipeError::UnknownParent {
                        step,
                        bone: bone.name.clone(),
                        parent: name.clone(),
                    })?,
            ),
        };
        builder.model.bones.push(clockwork_core::model::Bone {
            name: bone.name.clone(),
            parent,
            translation: bone.at,
            billboard: bone.billboard,
        });
        for dl in &bone.draw {
            builder
                .draw(*dl, index, bone.blend)
                .map_err(|e| RecipeError::Decode {
                    step,
                    message: format!("bone \"{}\", display list {dl:#010x}: {e}", bone.name),
                })?;
        }
        // A placed piece's own turn and size, baked into its vertices.
        if bone.rotate != [0.0; 3] || bone.scale.is_some() {
            let turn = |p| turn(p, bone.rotate);
            let size = bone.scale.unwrap_or(1.0);
            for mesh in builder.model.meshes.iter_mut().filter(|m| m.bone == index) {
                for p in &mut mesh.positions {
                    *p = turn(*p).map(|v| v * size);
                }
                for n in &mut mesh.normals {
                    *n = turn(*n);
                }
            }
        }
    }
    Ok(builder.finish())
}

/// `p` turned by `degrees` about x, then y, then z.
fn turn(p: [f32; 3], degrees: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = degrees.map(f32::to_radians);
    let (s, c) = x.sin_cos();
    let p = [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c];
    let (s, c) = y.sin_cos();
    let p = [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c];
    let (s, c) = z.sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
}

/// A plugin folder: `clockwork.toml` plus `recipe.ron`.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub dir: PathBuf,
    pub manifest: PluginManifest,
    /// The extraction recipe; empty for composition plugins.
    pub recipe: Recipe,
    /// What a composition plugin makes of other plugins' outputs (`compose.ron`).
    pub compose: Option<clockwork_core::compose::Composition>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Manifest {
        path: PathBuf,
        source: ManifestError,
    },
    #[error("{path}: {source}")]
    Recipe {
        path: PathBuf,
        source: Box<ron::error::SpannedError>,
    },
}

impl Plugin {
    pub fn name(&self) -> &str {
        &self.manifest.plugin.name
    }

    /// Outputs that can be placed in a scene, with their kind: a recipe's images, models and
    /// sprites, and a composition's characters.
    pub fn items(&self) -> Vec<(String, Option<clockwork_core::recipe::Kind>)> {
        let recipe = self
            .recipe
            .outputs
            .iter()
            .filter_map(|o| o.item().map(|(id, kind)| (id.to_string(), kind)));
        let composed = self.compose.iter().flat_map(|c| {
            c.characters
                .iter()
                .map(|c| (c.id.clone(), Some(clockwork_core::recipe::Kind::Character)))
        });
        recipe.chain(composed).collect()
    }

    /// Runs the plugin: its recipe on its game's ROM, or, for a composition plugin, the plugins
    /// it uses (found in `plugins`) and then the composition. `rom` reads a plugin's ROM.
    pub fn extract(
        &self,
        plugins: &[Plugin],
        rom: &dyn Fn(&Plugin) -> Result<Vec<u8>, String>,
    ) -> Result<Extracted, String> {
        self.extract_at(plugins, rom, 0)
    }

    fn extract_at(
        &self,
        plugins: &[Plugin],
        rom: &dyn Fn(&Plugin) -> Result<Vec<u8>, String>,
        depth: usize,
    ) -> Result<Extracted, String> {
        let Some(composition) = &self.compose else {
            let assets = assets::AssetIndex::build(plugins);
            return run_in(&self.recipe, &rom(self)?, &assets).map_err(|e| e.to_string());
        };
        // Compositions of compositions are fine; a loop is not.
        if depth > 8 {
            return Err(format!("{}: plugins use each other in a loop", self.name()));
        }
        let mut done: Vec<(String, Extracted)> = Vec::new();
        for name in compose::dependencies(composition) {
            let plugin = plugins
                .iter()
                .find(|p| p.name() == name)
                .ok_or_else(|| format!("plugin \"{name}\" is not installed"))?;
            let extracted = plugin
                .extract_at(plugins, rom, depth + 1)
                .map_err(|e| format!("{name}: {e}"))?;
            done.push((name, extracted));
        }
        compose::compose(composition, &|name: &str| {
            done.iter().find(|(n, _)| n == name).map(|(_, x)| x)
        })
    }
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, PluginError> {
        let dir = dir.as_ref();
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path)
                // Editors on Windows often save UTF-8 with a byte order mark.
                .map(|text| text.trim_start_matches('\u{FEFF}').to_string())
                .map_err(|source| PluginError::Io { path, source })
        };
        let manifest = PluginManifest::from_toml(&read("clockwork.toml")?).map_err(|source| {
            PluginError::Manifest {
                path: dir.join("clockwork.toml"),
                source,
            }
        })?;
        let bad = |name: &str| {
            let path = dir.join(name);
            move |source| PluginError::Recipe {
                path,
                source: Box::new(source),
            }
        };
        // A composition plugin has `compose.ron` and needs no recipe.
        let compose = match dir.join("compose.ron").exists() {
            true => Some(ron::from_str(&read("compose.ron")?).map_err(bad("compose.ron"))?),
            false => None,
        };
        let recipe = if compose.is_some() && !dir.join("recipe.ron").exists() {
            Recipe {
                steps: Vec::new(),
                outputs: Vec::new(),
            }
        } else {
            ron::from_str(&read("recipe.ron")?).map_err(bad("recipe.ron"))?
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
            recipe,
            compose,
        })
    }

    /// Loads every plugin folder directly inside `root`. Folders that fail to load are returned
    /// as errors so the caller can report them.
    pub fn load_all(root: impl AsRef<Path>) -> Vec<Result<Plugin, PluginError>> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("clockwork.toml").exists())
            .collect();
        dirs.sort();
        dirs.into_iter().map(Plugin::load).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_decodes_through_runs_and_codes_into_dialog_pages() {
        let recipe: Recipe = ron::from_str(
            r#"(
                steps: [
                    Literal(name: "rom", hex: "99 00 01 02 FE 01 02 9E 00 FE 02 FF"),
                    Text(from: "rom", at: 1, runs: [(0x00, "ABC")], codes: {0xFE: "\n", 0x9E: " "}, end: 0xFF, into: "said"),
                ],
                outputs: [
                    Dialog(id: "hello", from: Some("said"), lines_per_page: Some(2)),
                ],
            )"#,
        )
        .unwrap();
        let out = run(&recipe, &[]).unwrap();
        let (id, dialog) = &out.dialogs[0];
        assert_eq!(id, "hello");
        assert_eq!(dialog.pages, vec!["ABC\nBC A".to_string(), "C".to_string()]);
        // A byte nothing names is an error.
        let mut runs = vec![(0x00, "AB".to_string())];
        assert!(
            decode_text(
                &[0x02, 0xFF],
                0,
                &runs,
                &Default::default(),
                (0xFF, None),
                None
            )
            .is_err()
        );
        runs.clear();
        assert_eq!(
            decode_text(
                &[0x02, 0xFF],
                0,
                &runs,
                &Default::default(),
                (0xFF, None),
                Some("?")
            )
            .unwrap(),
            "?"
        );
        // Text of a fixed size stops after its length, end byte or not.
        runs.push((0x00, "AB".to_string()));
        assert_eq!(
            decode_text(
                &[0x00, 0x01, 0x00],
                0,
                &runs,
                &Default::default(),
                (0xFF, Some(2)),
                None
            )
            .unwrap(),
            "AB"
        );
    }

    #[test]
    fn placed_pieces_turn_about_x_then_y_then_z() {
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        // A quarter turn about y takes +x to -z, as N64 yaw does.
        assert!(close(
            turn([1.0, 0.0, 0.0], [0.0, 90.0, 0.0]),
            [0.0, 0.0, -1.0]
        ));
        // x first: +y goes to +z, then the y turn takes +z to +x.
        assert!(close(
            turn([0.0, 1.0, 0.0], [90.0, 90.0, 0.0]),
            [1.0, 0.0, 0.0]
        ));
        assert_eq!(turn([1.0, 2.0, 3.0], [0.0; 3]), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn n64_textures_decode_and_paste_into_a_canvas() {
        // Two RGBA16 texels, red and transparent green: 0xF801 and 0x07C0.
        let recipe: Recipe = ron::from_str(
            r#"(steps: [
                Literal(name: "texels", hex: "00 00 F8 01 07 C0"),
                N64Texture(from: "texels", at: 2, format: Rgba16, width: 2, height: 1, into: "pair"),
                Canvas(name: "sheet", width: 4, height: 2),
                Paste(from: "pair", into: "sheet", at: (1, 1)),
                Paste(from: "pair", into: "sheet", at: (3, 0)),
            ], outputs: [Image(id: "sheet", from: "sheet")])"#,
        )
        .unwrap();
        let out = run(&recipe, &[]).unwrap();
        let sheet = &out.images[0].1;
        assert_eq!((sheet.width, sheet.height), (4, 2));
        assert_eq!(sheet.get(1, 1), [255, 0, 0, 255]);
        assert_eq!(sheet.get(2, 1), [0, 255, 0, 0]);
        assert_eq!(
            sheet.get(3, 0),
            [255, 0, 0, 255],
            "pixels past the edge are left out"
        );
        assert_eq!(sheet.get(0, 0), [0, 0, 0, 0]);
        let bad: Recipe = ron::from_str(
            r#"(steps: [
                Literal(name: "texels", hex: "00"),
                N64Texture(from: "texels", format: Rgba16, width: 4, height: 4, into: "x"),
            ], outputs: [])"#,
        )
        .unwrap();
        assert!(run(&bad, &[]).is_err(), "too few texels is an error");
    }

    #[test]
    fn a_run_goes_on_from_what_was_kept() {
        let recipe: Recipe = ron::from_str(
            r#"(steps: [
                Literal(name: "texels", hex: "00 00 F8 01 07 C0"),
                N64Texture(from: "texels", at: 2, format: Rgba16, width: 2, height: 1, into: "pair"),
                Canvas(name: "sheet", width: 4, height: 2),
                Paste(from: "pair", into: "sheet", at: (1, 1)),
                Paste(from: "pair", into: "sheet", at: (3, 0)),
            ], outputs: [Image(id: "sheet", from: "sheet")])"#,
        )
        .unwrap();
        let mut kept = None;
        let full = run_observed(&recipe, &[], &asset::Builtins, |i, made| {
            if i == 2 {
                kept = Some(made.clone());
            }
        })
        .unwrap();
        let mut seen = Vec::new();
        let rest = run_from(&recipe, &[], &asset::Builtins, kept.unwrap(), 3, |i, _| {
            seen.push(i)
        })
        .unwrap();
        assert_eq!(seen, [3, 4], "only the steps after what was kept run");
        assert_eq!(rest.images[0].1.pixels, full.images[0].1.pixels);
    }

    #[test]
    fn runs_a_small_recipe() {
        // ROM: an uncompressed 4bpp tile (all color 1) at 0, a palette at 32,
        // and one metatile using that tile four times at 64.
        let mut rom = vec![0x11u8; 32];
        rom.extend_from_slice(&[0, 0, 0x1F, 0]); // color 0, color 1 = red
        rom.resize(64, 0);
        rom.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);

        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    Read(name: "tiles", offset: 0, size: Some(32)),
                    Read(name: "pal", offset: 32, size: Some(4)),
                    Read(name: "meta", offset: 64, size: Some(8)),
                    Place(from: "tiles", into: "vram", at: 0),
                    GbaPalettes(from: "pal", bank: "bg", slot: 0),
                    GbaMetatiles(tiles: "vram", char_base: 0, metatiles: "meta", bank: "bg", columns: 1, into: "sheet"),
                ],
                outputs: [Image(id: "tileset", from: "sheet")],
            )"#,
        )
        .unwrap();
        let out = run(&recipe, &rom).unwrap();
        let (id, image) = &out.images[0];
        assert_eq!(id, "tileset");
        assert_eq!((image.width, image.height), (16, 16));
        assert_eq!(image.get(15, 15), [255, 0, 0, 255]);
    }

    #[test]
    fn collision_becomes_a_terrain_with_meaning_and_a_start() {
        // Collision at ROM offset 4: three vertices, then one default triangle (0 1 2), then the
        // end of the groups.
        let words: [u16; 17] = [
            0x40, 3, 0, 0, 0, 0, 0, 100, 100, 0, 0, // start, count, three vertices
            0, 1, 0, 1, 2, // type 0, one triangle
            0x41,
        ];
        let mut rom = vec![0u8; 4];
        rom.extend(words.iter().flat_map(|w| w.to_be_bytes()));
        let text = |codes: &str| {
            format!(
                r#"Recipe(
                    steps: [
                        Read(name: "level", offset: 4, size: Some(34)),
                        CollisionTriangles(from: "level", at: 0, into: "ground",
                            layout: (word: U16, big_endian: true, start: Some(0x40), end: 0x41)),
                        N64Model(segments: {{}}, bones: [(name: "root", parent: None, at: (0, 0, 0), draw: [])], into: "m"),
                    ],
                    outputs: [Model(id: "area", from: "m", kind: Some(Area), meters_per_unit: 0.01,
                        terrain: Some((from: "ground", codes: {codes}, spawn: Some((position: (1, 2, 3), yaw: 90.0)))))],
                )"#
            )
        };
        let run_with = |codes: &str| run(&ron::from_str(&text(codes)).unwrap(), &rom);
        let out = run_with("{ 0: [Walkable] }").unwrap();
        let terrain = out.model("area").unwrap().terrain.as_ref().unwrap();
        assert_eq!(terrain.triangles, vec![[0, 1, 2]]);
        assert_eq!(terrain.vertices[1], [0.0, 0.0, 100.0]);
        assert_eq!(terrain.floor_normal_y, 0.01);
        assert_eq!(terrain.spawn.unwrap().yaw, 90.0);
        // A code without a meaning is an error, not a silent gap.
        assert!(matches!(
            run_with("{ 7: [Walkable] }"),
            Err(RecipeError::Terrain { .. })
        ));
    }

    #[test]
    fn characters_are_checked_for_missing_animations_and_actions() {
        use clockwork_core::character2d::Character2d;
        use clockwork_core::sprite::SpriteAnimation;
        let sprite = Sprite {
            frames: vec![],
            animations: ["idle", "walk", "hit"]
                .iter()
                .map(|n| SpriteAnimation {
                    name: n.to_string(),
                    steps: vec![],
                    fps: 60.0,
                    loop_start: None,
                })
                .collect(),
        };
        let directional = |name: &str| {
            format!(
                "(north: (name: \"{name}\"), east: (name: \"{name}\"), south: (name: \"{name}\"), west: (name: \"{name}\", flip_x: true))"
            )
        };
        let character = |extra: &str| -> Character2d {
            ron::from_str(&format!(
                "(movement: (speed: 4.0, radius: 0.4), animations: (idle: {i}, walk: {w}), {extra})",
                i = directional("idle"),
                w = directional("walk"),
            ))
            .unwrap()
        };
        let slash = format!(
            "(id: \"slash\", animation: {}, duration: 0.2)",
            directional("hit")
        );
        let ok = character(&format!("actions: [{slash}], dodge: Some(\"slash\")"));
        assert!(check_character("hero", &sprite, &ok).is_ok());
        // An action with an animation the sprite doesn't have, an unknown dodge, a release into
        // nothing and a projectile that doesn't exist are all caught.
        let bad_animation = character(&format!(
            "actions: [(id: \"x\", animation: {}, duration: 1.0)]",
            directional("nope")
        ));
        assert!(matches!(
            check_character("hero", &sprite, &bad_animation),
            Err(RecipeError::UnknownAnimation { .. })
        ));
        let bad_dodge = character(&format!("actions: [{slash}], dodge: Some(\"dodge\")"));
        assert!(matches!(
            check_character("hero", &sprite, &bad_dodge),
            Err(RecipeError::Character { .. })
        ));
        let bad_spawn = character(&format!(
            "actions: [(id: \"x\", animation: {}, duration: 1.0, spawns: [(at: 0.0, projectile: \"arrow\")])]",
            directional("hit")
        ));
        assert!(matches!(
            check_character("hero", &sprite, &bad_spawn),
            Err(RecipeError::Character { .. })
        ));
        let duplicate = character(&format!("actions: [{slash}, {slash}]"));
        assert!(matches!(
            check_character("hero", &sprite, &duplicate),
            Err(RecipeError::Character { .. })
        ));

        // What a character shows from other outputs must exist: held sprites, projectile
        // sprites and item icons.
        let extracted = |character: Character2d, items: &str| {
            let sword = Sprite {
                frames: vec![clockwork_core::sprite::SpriteFrame {
                    image: RgbaImage::new(1, 1),
                    origin: [0, 0],
                }],
                animations: vec![],
            };
            let sprite_output = |id: &str, sprite: Sprite, character| ExtractedSprite {
                id: id.into(),
                sprite,
                pixels_per_meter: 16.0,
                character,
            };
            Extracted {
                sprites: vec![
                    sprite_output("hero", sprite.clone(), Some(character)),
                    sprite_output("sword", sword, None),
                ],
                items: vec![("items".into(), ron::from_str(items).unwrap())],
                ..Default::default()
            }
        };
        let holding = |sprite: &str| {
            character(&format!(
                "actions: [(id: \"x\", animation: {}, duration: 1.0, attachments: [(sprite: \"{sprite}\", frames: FollowFrame(offset: 0))])]",
                directional("hit")
            ))
        };
        let icon = |frame: usize| {
            format!(
                "[(id: \"sword\", name: \"Sword\", description: \"\", category: Weapon, icon: Some(Frame(sprite: \"sword\", frame: {frame})))]"
            )
        };
        assert!(check_references(&extracted(holding("sword"), &icon(0))).is_ok());
        assert!(check_references(&extracted(holding("shield"), &icon(0))).is_err());
        assert!(check_references(&extracted(holding("sword"), &icon(3))).is_err());
        let thrown = character(
            "projectiles: [(id: \"arrow\", motion: Stationary, lifetime: 1.0, radius: 0.1, sprite: Some((sprite: \"sword\", animation: (north: (name: \"fly\"), east: (name: \"fly\"), south: (name: \"fly\"), west: (name: \"fly\")))))]",
        );
        assert!(check_references(&extracted(thrown, "[]")).is_err());
    }

    #[test]
    fn literal_fill_crop_and_region_overlay() {
        // One 4bpp tile of color 1, rendered as a 2×1 map of metatiles: 32×16 px.
        let mut rom = vec![0x11u8; 32];
        rom.extend_from_slice(&[0, 0, 0x1F, 0]);
        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    Read(name: "tiles", offset: 0, size: Some(32)),
                    Read(name: "pal", offset: 32, size: Some(4)),
                    GbaPalettes(from: "pal", bank: "bg", slot: 0),
                    // Metatile 0 uses tile 0 (red); metatile 1 uses tile 1, which doesn't exist,
                    // so it renders transparent.
                    Fill(name: "meta", size: 8),
                    Literal(name: "meta_blank", hex: "01 00 01 00 01 00 01 00"),
                    Literal(name: "map", hex: "00 00 00 00"),
                    GbaMetatileMap(tiles: "tiles", char_base: 0, metatiles: "meta", map: "map", width: 2, bank: "bg", into: "red"),
                    GbaMetatileMap(tiles: "tiles", char_base: 0, metatiles: "meta_blank", map: "map", width: 2, bank: "bg", into: "canvas"),
                    Overlay(from: "red", into: "canvas", region: Some((x: 16, y: 0, w: 8, h: 8))),
                    GbaMetatileMap(tiles: "tiles", char_base: 0, metatiles: "meta", map: "map", width: 2, bank: "bg", into: "replaced"),
                    Overlay(from: "canvas", into: "replaced", region: Some((x: 0, y: 0, w: 8, h: 8)), replace: true),
                    Crop(from: "red", region: (x: 16, y: 0, w: 16, h: 16), into: "right"),
                ],
                outputs: [
                    Image(id: "canvas", from: "canvas"),
                    Image(id: "right", from: "right"),
                    Image(id: "replaced", from: "replaced"),
                ],
            )"#,
        )
        .unwrap();
        let out = run(&recipe, &rom).unwrap();
        let (_, canvas) = &out.images[0];
        assert_eq!(canvas.get(16, 0), [255, 0, 0, 255]);
        assert_eq!(canvas.get(23, 7), [255, 0, 0, 255]);
        assert_eq!(canvas.get(24, 0)[3], 0);
        assert_eq!(canvas.get(0, 0)[3], 0);
        let (_, right) = &out.images[1];
        assert_eq!((right.width, right.height), (16, 16));
        assert_eq!(right.get(0, 0), [255, 0, 0, 255]);
        // `replace` copies the canvas's transparent corner over the red image.
        let (_, replaced) = &out.images[2];
        assert_eq!(replaced.get(0, 0)[3], 0);
        assert_eq!(replaced.get(8, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn refuses_oversized_buffers_and_canvases() {
        let fill = Recipe {
            steps: vec![Step::Fill {
                name: "x".into(),
                size: u32::MAX,
                value: 0,
            }],
            outputs: vec![],
        };
        assert!(matches!(
            run(&fill, &[]),
            Err(RecipeError::TooLarge { step: 0, .. })
        ));
        let canvas = Recipe {
            steps: vec![Step::Canvas {
                name: "x".into(),
                width: 65536,
                height: 65536,
            }],
            outputs: vec![],
        };
        assert!(matches!(
            run(&canvas, &[]),
            Err(RecipeError::TooLarge { step: 0, .. })
        ));
        let fits = Recipe {
            steps: vec![Step::Canvas {
                name: "x".into(),
                width: 4096,
                height: 4096,
            }],
            outputs: vec![],
        };
        assert!(run(&fits, &[]).is_ok());
    }

    #[test]
    fn reports_bad_offsets() {
        let recipe = Recipe {
            steps: vec![Step::Read {
                name: "x".into(),
                offset: 100,
                size: Some(4),
                codec: Codec::None,
            }],
            outputs: vec![],
        };
        assert!(matches!(
            run(&recipe, &[0; 10]),
            Err(RecipeError::OutOfRange {
                step: 0,
                offset: 100
            })
        ));
    }

    #[test]
    fn places_records_marks_their_cells_and_makes_a_scene() {
        // Records of 6 bytes: kind, id, x (u16), y (u16); kind 0xFF ends the list.
        let rom = [
            1, 5, 24, 0, 8, 0, // a slime at (24, 8)
            2, 9, 40, 0, 40, 0, // a door (kind 2) at (40, 40)
            1, 7, 8, 0, 40, 0, // an unknown enemy at (8, 40)
            0xFF, 0, 0, 0, 0, 0,
        ];
        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    Read(name: "records", offset: 0, size: Some(24)),
                    Fill(name: "pixels", size: 64, value: 0x11),
                    Literal(name: "meta", hex: "00 00 00 00 00 00 00 00"),
                    Literal(name: "map", hex: "00 00 00 00 00 00 00 00"),
                    Literal(name: "pal", hex: "00 00 1F 00"),
                    GbaPalettes(from: "pal", bank: "bg", slot: 1),
                    GbaMetatileMap(tiles: "pixels", char_base: 0, metatiles: "meta", map: "map",
                                   width: 2, bank: "bg", into: "room", depth: Bpp8),
                    Places(from: "records", stride: 6, until: Some((field: (at: 0), values: [0xFF])),
                           when: [(field: (at: 0), values: [1])],
                           x: (at: 2, size: U16), y: (at: 4, size: U16), origin: (100, 0),
                           item: Some((by: Some((at: 1)), outputs: { 5: "room" })),
                           name: (text: "enemy {} #{index}", fields: [(at: 1)]),
                           group: Some((text: "Room")), code: Some((at: 1)), into: "enemies"),
                    Places(from: "records", stride: 6, count: Some(3),
                           when: [(field: (at: 0), values: [2])],
                           x: (at: 2, size: U16), y: (at: 4, size: U16), origin: (100, 0),
                           name: (text: "door"), door: Some((to: (text: "enemy {} #0", fields: [(at: 1)]))),
                           extent: Some((by: (at: 0), sizes: { 2: (12, 4) })), into: "doors"),
                    MarkPlaces(from: "enemies", width: 4, height: 4, cell_size: 16,
                               origin: (100, 0), into: "cells"),
                    MarkPlaces(from: "doors", width: 4, height: 4, cell_size: 16,
                               origin: (100, 0), into: "cells"),
                ],
                outputs: [
                    Image(id: "room", from: "room", kind: Some(Area)),
                    Scene(id: "world", from: ["enemies", "doors"]),
                ],
            )"#,
        )
        .unwrap();
        let extracted = run(&recipe, &rom).unwrap();

        // 256 colors: byte 0x11 is color 17, palette 1's first color after transparent.
        let (_, room) = &extracted.images[0];
        assert_eq!(room.get(0, 0), [255, 0, 0, 255]);

        let (_, scene) = &extracted.scenes[0];
        let names: Vec<&str> = scene.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["enemy 5 #0", "enemy 7 #2", "door"]);
        assert_eq!(scene.nodes[0].item.as_deref(), Some("room"));
        assert_eq!(scene.nodes[1].item, None);
        assert_eq!(scene.nodes[0].group.as_deref(), Some("Room"));
        // (24 + 100, 8) pixels at 16 per meter.
        assert_eq!(scene.nodes[0].position, [7.75, 0.0, 0.5]);
        assert_eq!(
            scene.nodes[2].door.as_ref().map(|d| d.to.as_str()),
            Some("enemy 9 #0")
        );

        // Enemies mark one cell with their id; the door, record 2 counting from 1, marks the
        // cells (40, 40) ± (12, 4) pixels reach: x 28..52, columns 1 to 3 of row 2.
        let mut made = Made::default();
        for (i, s) in recipe.steps.iter().enumerate() {
            run_step(i, s, &rom, &mut made).unwrap();
        }
        let cells = made.buffer("cells").unwrap();
        assert_eq!(cells[1], 5);
        assert_eq!(cells[2 * 4], 7);
        assert_eq!(&cells[2 * 4 + 1..2 * 4 + 4], &[2, 2, 2]);
    }

    #[test]
    fn a_scene_shows_only_this_plugins_outputs() {
        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    Literal(name: "r", hex: "00 00"),
                    Places(from: "r", stride: 2, count: Some(1), x: (at: 0), y: (at: 1),
                           item: Some((outputs: {}, other: Some("missing"))),
                           name: (text: "x"), into: "p"),
                ],
                outputs: [Scene(id: "s", from: ["p"])],
            )"#,
        )
        .unwrap();
        assert!(matches!(run(&recipe, &[]), Err(RecipeError::Part { .. })));
    }
}

/// Runs a plugin folder against a local ROM and writes previews, for checking recipes by eye:
/// `CLOCKWORK_SMOKE_PLUGIN=<plugin folder> CLOCKWORK_SMOKE_ROM=game.z64 CLOCKWORK_SMOKE_OUT=dir
/// cargo test -p clockwork_recipe smoke -- --ignored`. The ROM must be in canonical form
/// (N64: big-endian).
#[cfg(test)]
mod smoke {
    use super::*;

    #[test]
    #[ignore]
    fn run_local_plugin() {
        let var = |name| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let plugin = Plugin::load(var("CLOCKWORK_SMOKE_PLUGIN")).unwrap();
        let rom = std::fs::read(var("CLOCKWORK_SMOKE_ROM")).unwrap();
        let out = PathBuf::from(var("CLOCKWORK_SMOKE_OUT"));
        let start = std::time::Instant::now();
        let extracted = run(&plugin.recipe, &rom).unwrap();
        println!("ran in {:?}", start.elapsed());
        let save = |name: &str, image: &RgbaImage| {
            image::save_buffer(
                out.join(format!("{name}.png")),
                &image.pixels,
                image.width,
                image.height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        };
        for (id, image) in &extracted.images {
            save(id, image);
        }
        for (_, items) in &extracted.items {
            for item in items {
                if let Some(icon) = item.icon.as_ref().and_then(|i| extracted.icon(i)) {
                    save(&format!("icon-{}", item.id), &icon);
                }
            }
        }
        for s in &extracted.sprites {
            println!(
                "{}: {} frames, {} animations",
                s.id,
                s.sprite.frames.len(),
                s.sprite.animations.len()
            );
            save(
                &format!("{}-sheet", s.id),
                &preview::sprite_sheet(&s.sprite),
            );
            if let Some(character) = &s.character
                && let Some(sheet) = preview::actions_sheet(&s.sprite, character, &|id| {
                    extracted.sprite(id).map(|o| &o.sprite)
                })
            {
                save(&format!("{}-actions", s.id), &sheet);
            }
        }
        for ExtractedModel {
            id,
            model,
            meters_per_unit: meters,
            ..
        } in &extracted.models
        {
            let triangles: usize = model.meshes.iter().map(|m| m.indices.len() / 3).sum();
            println!(
                "{id}: {} bones, {} meshes, {triangles} triangles, {} textures, {} animations, {meters} m/unit",
                model.bones.len(),
                model.meshes.len(),
                model.textures.len(),
                model.animations.len()
            );
            save(
                &format!("{id}-rest"),
                &preview::render_model(model, None, 512),
            );
            for animation in &model.animations {
                for frame in [0, animation.frames / 2] {
                    let name = format!("{id}-{}-{frame}", animation.name);
                    save(
                        &name,
                        &preview::render_model(model, Some((animation, frame)), 512),
                    );
                }
            }
            for (i, texture) in model.textures.iter().enumerate() {
                save(&format!("{id}-texture-{i}"), texture);
            }
        }
    }
}

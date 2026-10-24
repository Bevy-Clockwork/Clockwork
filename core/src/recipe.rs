//! Extraction recipes, `recipe.ron` (DESIGN.md §5.1).
//!
//! A recipe is a list of deterministic steps that turn the user's ROM into content. It contains
//! only offsets and parameters, never game data. Steps work on named buffers (raw bytes),
//! palette banks and images; `outputs` names what the plugin provides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::character::CharacterSpec;
use crate::character2d::Character2dSpec;
use crate::item::ItemDef;
use crate::model::Blend;
pub use crate::place::{Extent, Match, PlaceDoor, PlaceItem, Template};
use crate::surface::{SurfaceExtension, SurfaceProperty};
use crate::terrain::Spawn;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub steps: Vec<Step>,
    pub outputs: Vec<Output>,
}

impl Recipe {
    /// A recipe of one output and no steps, to write an output on its own.
    pub fn with_output(output: &Output) -> Self {
        Self {
            steps: Vec::new(),
            outputs: vec![output.clone()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Step {
    /// Reads bytes from the ROM into buffer `name`. Compressed data knows its own size;
    /// uncompressed reads need `size`.
    Read {
        name: String,
        offset: u32,
        #[serde(default)]
        size: Option<u32>,
        #[serde(default)]
        codec: Codec,
    },
    /// Copies buffer `from` into buffer `into` at byte offset `at`, growing `into` as needed.
    /// Used to assemble memory like VRAM from several pieces.
    Place { from: String, into: String, at: u32 },
    /// Creates buffer `name` of `size` bytes, all `value` (default 0).
    Fill {
        name: String,
        size: u32,
        #[serde(default)]
        value: u8,
    },
    /// Creates buffer `name` from bytes written as hex, e.g. "00 01 FF". For small tables and
    /// markers only: never game data.
    Literal { name: String, hex: String },
    /// Loads BGR555 palettes from buffer `from` into palette bank `bank`, starting at `slot`.
    GbaPalettes {
        from: String,
        bank: String,
        slot: u8,
    },
    /// Renders 16×16 metatiles (four GBA tile map entries each) into image `into`, `columns`
    /// metatiles wide. Tile numbers count from `char_base` bytes into buffer `tiles`. Tiles are
    /// 16-color (`depth: Bpp4`, the default) or 256-color (`Bpp8`, for backgrounds in 256-color
    /// mode, which ignore the entries' palette numbers).
    GbaMetatiles {
        tiles: String,
        char_base: u32,
        metatiles: String,
        bank: String,
        columns: u32,
        into: String,
        #[serde(default, skip_serializing_if = "TileDepth::is_default")]
        depth: TileDepth,
    },
    /// Renders a map of metatile numbers (buffer `map`, `width` metatiles wide) into image
    /// `into`, using the metatile definitions and tiles as in `GbaMetatiles`.
    GbaMetatileMap {
        tiles: String,
        char_base: u32,
        metatiles: String,
        map: String,
        width: u32,
        bank: String,
        into: String,
        #[serde(default, skip_serializing_if = "TileDepth::is_default")]
        depth: TileDepth,
    },
    /// Draws image `from` on top of image `into` (both must exist), skipping transparent pixels.
    /// With `region`, only pixels inside that rectangle are drawn (images line up at 0, 0).
    /// With `replace`, transparent pixels are copied too, so the region shows exactly `from`.
    Overlay {
        from: String,
        into: String,
        #[serde(default)]
        region: Option<Rect>,
        #[serde(default)]
        replace: bool,
    },
    /// Copies a rectangle of image `from` into a new image `into`.
    Crop {
        from: String,
        region: Rect,
        into: String,
    },
    /// Decodes text from buffer `from`, starting at byte `at` and stopping at byte `end` (or
    /// after `length` bytes, for text of a fixed size), into buffer `into` (UTF-8). Each byte means the text `codes` gives it, else the character of
    /// a run: `(first, "ABC")` makes `first` "A", `first + 1` "B" and so on. A byte neither
    /// names is an error, unless `other` says what it means.
    Text {
        from: String,
        #[serde(default)]
        at: u32,
        #[serde(default)]
        runs: Vec<(u8, String)>,
        #[serde(default)]
        codes: BTreeMap<u8, String>,
        end: u8,
        #[serde(default)]
        length: Option<u32>,
        #[serde(default)]
        other: Option<String>,
        into: String,
    },
    /// A new transparent image `name` of `width` by `height` pixels, to paste into.
    Canvas {
        name: String,
        width: u32,
        height: u32,
    },
    /// Draws image `from` into image `into` with its top left corner at `at` (pixels), pixels
    /// outside `into` left out. Builds sheets (a font's glyphs) from images made one by one.
    Paste {
        from: String,
        into: String,
        at: (u32, u32),
    },
    /// Decodes an N64 texture: `width` by `height` texels in `format`, starting at byte `at` of
    /// buffer `from`. The colour-indexed formats read their palette (RGBA16 entries) from
    /// buffer `palette.0` at byte `palette.1`.
    N64Texture {
        from: String,
        #[serde(default)]
        at: u32,
        format: N64TextureFormat,
        width: u32,
        height: u32,
        #[serde(default)]
        palette: Option<(String, u32)>,
        into: String,
    },
    /// Looks up every element of buffer `indices` in buffer `table` and writes the results to
    /// buffer `into`: `into[i] = table[indices[i]]`. Chains of lookups turn per-cell numbers
    /// into per-cell attributes (cell → tile type → collision code).
    Lookup {
        indices: String,
        #[serde(default)]
        index: Element,
        table: String,
        #[serde(default)]
        value: Element,
        into: String,
    },
    /// Builds model `into` from N64 display lists. `segments` names the buffer holding each
    /// RAM segment, so segmented addresses like 0x04001234 resolve. Bones are listed parents
    /// first; each draws its display lists.
    N64Model {
        #[serde(default)]
        microcode: Microcode,
        segments: BTreeMap<u8, String>,
        bones: Vec<ModelBone>,
        into: String,
    },
    /// Reads collision triangles into collision `into`, from buffer `from` at byte `at`: a vertex
    /// count and that many (x, y, z) vertices, then groups of triangles that share a code (a code,
    /// a count, and that many triangles of three vertex indices), until the `layout`'s end code.
    /// The code of a group is each of its triangles' code.
    CollisionTriangles {
        from: String,
        at: u32,
        layout: TriangleLayout,
        into: String,
    },
    /// Adds an animation to model `into`, stored as value lists per channel: a table of
    /// (count, first value) pairs of big-endian u16 at byte `index` of buffer `from`, and
    /// big-endian i16 values at byte `values`. Channels are the root bone's translation x, y, z
    /// (with `root_translation`), then x, y, z rotations of every bone in order, as fractions of
    /// 65536 per turn, applied x first, then y, then z. A channel with fewer values than frames
    /// holds its last one. With `bones`, only the model's first that many bones are animated
    /// (bones after them, such as billboards hung on a part, keep their rest pose).
    ChannelAnimation {
        from: String,
        name: String,
        index: u32,
        values: u32,
        frames: u32,
        #[serde(default)]
        loop_start: Option<u32>,
        #[serde(default = "default_fps")]
        fps: f32,
        #[serde(default)]
        root_translation: bool,
        #[serde(default)]
        bones: Option<u32>,
        into: String,
    },
    /// Builds sprite `into` whose frames are drawn from hardware objects (OAM entries: rectangles
    /// of 4bpp tiles). Each frame has a list of objects placed relative to the sprite's origin;
    /// `frames` says where the lists are. Objects are drawn in list order, the first on top, as
    /// on the hardware. Frames are numbered in table order.
    GbaObjectFrames {
        /// Buffer of 4bpp tiles.
        tiles: String,
        bank: String,
        /// Buffer holding the object lists.
        objects: String,
        #[serde(default)]
        layout: ObjectLayout,
        frames: Box<FrameTables>,
        into: String,
    },
    /// Appends the frames and animations of sprite `from` to sprite `into`. Games draw one
    /// character from several sheets of frames (a dodge drawn in another sheet than the walk);
    /// merged, one sprite plays them all. Animation names must be unique across both.
    MergeSprite { from: String, into: String },
    /// Adds an animation to sprite `into`, decoded from keyframes in buffer `from`: a list of
    /// (frame, duration, flags) records, as many 8- and 16-bit games store them.
    KeyframeAnimation {
        from: String,
        name: String,
        at: AnimationAt,
        layout: KeyframeLayout,
        /// Ticks per second the durations count in.
        fps: f32,
        into: String,
    },
    /// Places what a game lists in a table of records (the enemies of a room, its doors, the
    /// rooms of an area) into placement list `into`, added to it if it exists, for a `Scene`
    /// output or a `MarkPlaces` step. Records of `stride` bytes start at byte `at` of buffer
    /// `from`: `count` of them, or up to the first that matches `until`. Only records matching
    /// every `when` are placed, at (`x`, `y`) plus `origin` pixels; with `size` (width and height
    /// fields) the position is the record's top left corner and the placement goes to its centre.
    Places {
        from: String,
        #[serde(default)]
        at: u32,
        stride: u32,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        until: Option<Box<Match>>,
        #[serde(default)]
        when: Vec<Match>,
        x: Field,
        y: Field,
        #[serde(default)]
        size: Option<(Field, Field)>,
        #[serde(default)]
        origin: (i32, i32),
        /// The output each placement shows.
        #[serde(default)]
        item: Option<Box<PlaceItem>>,
        name: Box<Template>,
        /// The group node each placement goes under.
        #[serde(default)]
        group: Option<Box<Template>>,
        /// Walking into the placement takes the player to another node.
        #[serde(default)]
        door: Option<Box<PlaceDoor>>,
        /// What `MarkPlaces` writes into the cells: this field's value, else the record's number
        /// from 1.
        #[serde(default)]
        code: Option<Field>,
        /// How far the placement reaches, for `MarkPlaces` and doors; one cell without it.
        #[serde(default)]
        extent: Option<Box<Extent>>,
        into: String,
    },
    /// Marks the cells each placement of list `from` covers with its code, in buffer `into`
    /// (one byte per cell, `width` by `height` cells of `cell_size` pixels, made if new). `origin`
    /// is where the grid's top left corner is in the placements' space, so the cells line up
    /// with the image they cover: a surface grid of where things are, read from the game.
    MarkPlaces {
        from: String,
        width: u32,
        height: u32,
        cell_size: u32,
        #[serde(default)]
        origin: (i32, i32),
        into: String,
    },
}

/// How many bits a GBA tile's pixel takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileDepth {
    /// 16 colors, 32 bytes a tile: the palette comes from the tile map entry.
    #[default]
    Bpp4,
    /// 256 colors, 64 bytes a tile.
    Bpp8,
}

impl TileDepth {
    pub fn is_default(&self) -> bool {
        *self == TileDepth::Bpp4
    }
}

fn empty_dialog() -> crate::asset::Slot<crate::ui::Dialog> {
    crate::asset::Slot::Inline(crate::ui::Dialog {
        speaker: None,
        pages: Vec::new(),
    })
}

/// The pages of a dialog's text: `lines_per_page` lines each (blank lines don't count), or
/// split at blank lines.
pub fn dialog_pages(text: &str, lines_per_page: Option<u32>) -> Vec<String> {
    let pages: Vec<String> = match lines_per_page {
        Some(n) => text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .chunks(n.max(1) as usize)
            .map(|lines| lines.join("\n"))
            .collect(),
        None => text.split("\n\n").map(str::to_string).collect(),
    };
    pages
        .into_iter()
        .map(|page| page.trim_matches('\n').to_string())
        .filter(|page| !page.trim().is_empty())
        .collect()
}

fn default_fps() -> f32 {
    30.0
}

/// The width of a number stored in a record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldSize {
    #[default]
    U8,
    U16,
    U32,
}

impl FieldSize {
    pub fn bytes(self) -> usize {
        match self {
            FieldSize::U8 => 1,
            FieldSize::U16 => 2,
            FieldSize::U32 => 4,
        }
    }
}

/// How a game stores collision triangles (see [`Step::CollisionTriangles`]): every number is one
/// word, vertices are signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriangleLayout {
    pub word: FieldSize,
    #[serde(default)]
    pub big_endian: bool,
    /// A word the data must start with, if the game marks it.
    #[serde(default)]
    pub start: Option<u32>,
    /// The code that ends the groups (what follows is not read).
    pub end: u32,
    /// Words that follow a triangle's vertex indices, by group code (none for codes not listed).
    #[serde(default)]
    pub extra: BTreeMap<u32, u32>,
}

/// A number inside a fixed-size record: `size` bytes (little-endian) at byte `at`, shifted right
/// by `shift`, keeping the lowest `bits` bits (0: all of them). With `signed`, the kept bits are
/// a two's complement number. This describes any packed record layout without code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub at: u32,
    #[serde(default)]
    pub size: FieldSize,
    #[serde(default)]
    pub shift: u8,
    #[serde(default)]
    pub bits: u8,
    #[serde(default)]
    pub signed: bool,
}

impl Field {
    pub const fn new(at: u32, size: FieldSize) -> Self {
        Self {
            at,
            size,
            shift: 0,
            bits: 0,
            signed: false,
        }
    }

    pub const fn bits(mut self, shift: u8, bits: u8) -> Self {
        self.shift = shift;
        self.bits = bits;
        self
    }

    pub const fn signed(mut self) -> Self {
        self.signed = true;
        self
    }

    /// Reads the field from the start of `record`; `None` if the record is too short.
    pub fn read(&self, record: &[u8]) -> Option<i64> {
        let start = self.at as usize;
        let bytes = record.get(start..start + self.size.bytes())?;
        let mut raw = 0u64;
        for (i, b) in bytes.iter().enumerate() {
            raw |= (*b as u64) << (8 * i);
        }
        raw >>= self.shift.min(63);
        let width = match self.bits {
            0 => (self.size.bytes() * 8).saturating_sub(self.shift as usize) as u32,
            n => n as u32,
        };
        if width == 0 || width >= 64 {
            return Some(raw as i64);
        }
        raw &= (1u64 << width) - 1;
        if self.signed && raw & (1u64 << (width - 1)) != 0 {
            Some(raw as i64 - (1i64 << width))
        } else {
            Some(raw as i64)
        }
    }
}

/// `count` records of `stride` bytes starting at `offset` of a buffer, each read with `field`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub buffer: String,
    pub offset: u32,
    pub stride: u32,
    pub count: u32,
    pub field: Field,
}

impl Table {
    /// The values of the table in `data` (the buffer's bytes); `None` if it runs past the end,
    /// or has more records than the buffer has bytes (only possible with a stride of 0).
    pub fn values(&self, data: &[u8]) -> Option<Vec<i64>> {
        if self.count as usize > data.len() {
            return None;
        }
        (0..self.count as usize)
            .map(|i| {
                let start = self.offset as usize + i * self.stride as usize;
                self.field.read(data.get(start..)?)
            })
            .collect()
    }
}

/// How one hardware object is stored in a buffer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ObjectLayout {
    /// The hardware's own six bytes: attribute 0, 1 and 2 as little-endian u16.
    #[default]
    Oam,
    /// A game's own packed record.
    Fields(ObjectFields),
}

/// A game's object record: where each hardware attribute sits. Shape and size are the
/// hardware's two-bit codes (shape 0 square, 1 wide, 2 tall; size 0..3 grows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectFields {
    pub stride: u32,
    pub x: Field,
    pub y: Field,
    pub shape: Field,
    pub size: Field,
    #[serde(default)]
    pub hflip: Option<Field>,
    #[serde(default)]
    pub vflip: Option<Field>,
    /// The first tile, counted from the frame's tile window.
    pub tile: Field,
    #[serde(default)]
    pub palette: Option<Field>,
}

impl ObjectLayout {
    pub fn fields(&self) -> ObjectFields {
        match self {
            ObjectLayout::Fields(fields) => fields.clone(),
            ObjectLayout::Oam => ObjectFields {
                stride: 6,
                y: Field::new(0, FieldSize::U16).bits(0, 8).signed(),
                shape: Field::new(0, FieldSize::U16).bits(14, 2),
                x: Field::new(2, FieldSize::U16).bits(0, 9).signed(),
                hflip: Some(Field::new(2, FieldSize::U16).bits(12, 1)),
                vflip: Some(Field::new(2, FieldSize::U16).bits(13, 1)),
                size: Field::new(2, FieldSize::U16).bits(14, 2),
                tile: Field::new(4, FieldSize::U16).bits(0, 10),
                palette: Some(Field::new(4, FieldSize::U16).bits(12, 4)),
            },
        }
    }
}

/// Where the frames of a [`Step::GbaObjectFrames`] are: one entry per frame in each table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameTables {
    /// Each frame's object list: its offset in the objects buffer (times `list_unit`), plus
    /// `list_base`. A list is a count (`count_size`) followed by that many objects.
    pub lists: Table,
    #[serde(default)]
    pub list_base: u32,
    #[serde(default)]
    pub count_size: FieldSize,
    /// Bytes per unit of the `lists` values: the object size, for games whose frames name their
    /// first object by number. Default 1 (the values are byte offsets).
    #[serde(default = "one")]
    pub list_unit: u32,
    /// Each frame's object count, for games that keep it in the frame's record: a list is then
    /// only its objects, with no count before them (`count_size` is not read).
    #[serde(default)]
    pub counts: Option<Table>,
    /// Each frame's own palettes, for games that load a frame's palettes as they draw it: an
    /// object with palette number p uses palette (first + p) of the buffer instead of slot p of
    /// the step's bank.
    #[serde(default)]
    pub palettes: Option<FramePalettes>,
    /// Each frame's first tile in the tiles buffer, counted in tiles. Object tile numbers are
    /// relative to it, as games that stream a frame's tiles into video memory do. Without it
    /// every frame starts at tile 0.
    #[serde(default)]
    pub tile_base: Option<Table>,
    /// Added to every frame's first tile: for sprites whose tiles the game keeps at a fixed
    /// place in video memory (object tile numbers then count from there).
    #[serde(default)]
    pub first_tile: u32,
}

fn one() -> u32 {
    1
}

/// A [`FrameTables`]' per-frame palettes: 16-color BGR555 palettes (32 bytes each) in `buffer`,
/// and each frame's first one, counted in palettes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramePalettes {
    pub buffer: String,
    pub first: Table,
}

/// Where a [`Step::KeyframeAnimation`]'s keyframes start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationAt {
    /// At this offset of the buffer.
    Offset(u32),
    /// Through a table of little-endian u32 pointers in the same buffer: the `index`th pointer
    /// minus `base` (the address the buffer's start has in memory) is the offset.
    Pointer { table: u32, index: u32, base: u32 },
    /// Through a directory of records (`stride` bytes each, at `table` in the same buffer), for
    /// games that keep an animation's length apart from its keyframes: record `index` gives the
    /// keyframe count and the first keyframe's number, keyframes being numbered from offset
    /// `keys`. The count ends the list; the layout's `end` is not read.
    Directory {
        table: u32,
        index: u32,
        stride: u32,
        count: Field,
        first: Field,
        keys: u32,
    },
}

/// How keyframes are stored: records of `stride` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyframeLayout {
    pub stride: u32,
    pub frame: Field,
    pub ticks: Field,
    #[serde(default)]
    pub flip_x: Option<Field>,
    #[serde(default)]
    pub flip_y: Option<Field>,
    /// Nonzero on a list's last keyframe.
    pub end: Field,
    #[serde(default)]
    pub after_end: AfterEnd,
}

/// What an animation does after its last keyframe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterEnd {
    /// Holds the last keyframe.
    #[default]
    Stop,
    /// Starts again at the first keyframe.
    Restart,
    /// The byte after the last keyframe says how many keyframes to go back (0: stop).
    RewindByte,
}

/// How display lists are encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Microcode {
    /// Fast3D, used by early N64 games.
    #[default]
    F3d,
}

/// A bone of an [`Step::N64Model`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBone {
    pub name: String,
    /// The parent's name; bones without one hang from the model's origin.
    #[serde(default)]
    pub parent: Option<String>,
    /// Offset from the parent, in model units.
    #[serde(default)]
    pub at: [f32; 3],
    /// Segmented addresses of display lists drawn with this bone.
    #[serde(default)]
    pub draw: Vec<u32>,
    #[serde(default)]
    pub blend: Blend,
    /// Its geometry always faces the camera (geo layout billboard nodes: trees, coins).
    #[serde(default)]
    pub billboard: bool,
    /// Turns the bone's own geometry by these angles in degrees (x, then y, then z) as it is
    /// built: a piece placed rotated, such as a level's static objects. Not a joint rotation.
    #[serde(default)]
    pub rotate: [f32; 3],
    /// Scales the bone's own geometry as it is built.
    #[serde(default)]
    pub scale: Option<f32>,
}

/// A rectangle in pixels: left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.w && y - self.y < self.h
    }
}

/// Parses hex bytes such as "00 01 ff" or "0001ff".
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return Err("hex needs an even number of digits".into());
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Size of one element in a buffer of numbers (little-endian).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Element {
    #[default]
    U8,
    U16,
}

impl Element {
    pub fn size(self) -> usize {
        match self {
            Element::U8 => 1,
            Element::U16 => 2,
        }
    }

    /// Reads all elements of `bytes`; a trailing partial element is ignored.
    pub fn read_all(self, bytes: &[u8]) -> Vec<u16> {
        match self {
            Element::U8 => bytes.iter().map(|&b| b as u16).collect(),
            Element::U16 => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect(),
        }
    }

    pub fn write(self, out: &mut Vec<u8>, value: u16) {
        match self {
            Element::U8 => out.push(value as u8),
            Element::U16 => out.extend_from_slice(&value.to_le_bytes()),
        }
    }
}

/// A grid of game-specific codes and what they mean, for a [`Output::Surfaces`] map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGrid {
    /// Name shown when inspecting a cell, e.g. "collision".
    pub name: String,
    /// Buffer holding one code per cell.
    pub from: String,
    #[serde(default)]
    pub element: Element,
    pub codes: BTreeMap<u16, Vec<SurfaceProperty>>,
    #[serde(default)]
    pub other: Option<Vec<SurfaceProperty>>,
}

/// The collision of a 3D area: a collision read by a step, and what its codes mean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainSpec {
    /// The collision (a step's `into`).
    pub from: String,
    pub codes: BTreeMap<u16, Vec<SurfaceProperty>>,
    /// Properties for codes not listed. Without it, an unlisted code is an error.
    #[serde(default)]
    pub other: Option<Vec<SurfaceProperty>>,
    #[serde(default)]
    pub extensions: Vec<SurfaceExtension>,
    /// Triangles whose normal points up more than this (its y, 0..1) are floors, those pointing
    /// down more are ceilings, the rest walls. Take it from the game's own collision loader.
    #[serde(default = "crate::terrain::default_floor_normal_y")]
    pub floor_normal_y: f32,
    /// Where a character starts, in model units (e.g. the level script's start position).
    #[serde(default)]
    pub spawn: Option<Spawn>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    #[default]
    None,
    /// GBA BIOS LZ77 (header byte 0x10).
    GbaLz77,
    /// Popuri 0x70 Huffman/LZSS/delta streams.
    Popuri,
    /// Nintendo's MIO0 (header "MIO0"), common in N64 games.
    Mio0,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Output {
    /// An image the plugin provides, under a stable `id`. `kind` sorts it in the Editor's
    /// content browser; without one it is listed under Other.
    Image {
        id: String,
        from: String,
        #[serde(default)]
        kind: Option<Kind>,
    },
    /// A surface map: `width` cells wide, each `cell_size` native units, built from one or more
    /// grids of codes. Game-specific surfaces are declared in `extensions`.
    Surfaces {
        id: String,
        width: u32,
        cell_size: u32,
        grids: Vec<SurfaceGrid>,
        #[serde(default)]
        extensions: Vec<SurfaceExtension>,
        /// The image output it covers. Without it, an image of the same size is taken, which
        /// is ambiguous when several images share a size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// A model, with its animations. `meters_per_unit` scales model units to meters. With
    /// `character`, the model can be played: how it moves and which animation goes with what.
    Model {
        id: String,
        from: String,
        #[serde(default)]
        kind: Option<Kind>,
        meters_per_unit: f32,
        #[serde(default)]
        character: Option<Box<CharacterSpec>>,
        /// Collision for a model that is an area: what a character can walk on.
        #[serde(default)]
        terrain: Option<Box<TerrainSpec>>,
    },
    /// The items of a game (see [`crate::item`]) that characters of the plugin can carry.
    Items { id: String, items: Vec<ItemDef> },
    /// An ability other characters can learn (see [`crate::ability`]), made of character output
    /// `from`'s `actions` (and the actions they release into), the projectiles they throw, the
    /// `items` that start them (with their ammunition) and, with `dodge`, its dodge. Their poses
    /// become roles named after the action (`slash`, `slash_hold`, `slash_hold_walk`).
    Ability {
        id: String,
        name: String,
        #[serde(default)]
        description: String,
        from: String,
        actions: Vec<String>,
        #[serde(default)]
        items: Vec<String>,
        #[serde(default, alias = "roll")]
        dodge: bool,
    },
    /// A sprite with its animations. `pixels_per_meter` says how big a pixel is in the world.
    /// With `character`, it can be played from above: how it moves and which animation goes
    /// with what.
    Sprite {
        id: String,
        from: String,
        #[serde(default)]
        kind: Option<Kind>,
        pixels_per_meter: f32,
        #[serde(default)]
        character: Option<Box<Character2dSpec>>,
    },
    /// A camera a character can name (see [`crate::camera`]): written in place, or the name of
    /// another camera asset, `"plugin/output"`.
    Camera {
        id: String,
        camera: crate::asset::Slot<crate::camera::CameraRig>,
    },
    /// A HUD a character can name (see [`crate::ui`]): which widgets, where.
    Hud {
        id: String,
        hud: crate::asset::Slot<crate::ui::HudLayout>,
    },
    /// A UI theme: how panels and text look.
    Ui {
        id: String,
        ui: crate::asset::Slot<crate::ui::Theme>,
    },
    /// A menu: the inventory, a pause menu.
    Menu {
        id: String,
        menu: crate::asset::Slot<crate::ui::Menu>,
    },
    /// The screens around a game's world (see [`crate::flow`]).
    Flow {
        id: String,
        flow: crate::asset::Slot<crate::flow::Flow>,
    },
    /// Which keys and pad buttons press which game button (see [`crate::input`]).
    InputMap {
        id: String,
        input: crate::asset::Slot<crate::input::InputMap>,
    },
    /// How characters move (see [`crate::parts`]): `TopDown(...)` or `Platformer(...)`.
    Movement {
        id: String,
        movement: crate::asset::Slot<crate::parts::AnyMovement>,
    },
    /// Which animations play for what: `Directional(...)` or `States(...)`.
    Animations {
        id: String,
        animations: crate::asset::Slot<crate::parts::AnyAnimations>,
    },
    /// A character's health and hurt time.
    Stats {
        id: String,
        stats: crate::asset::Slot<crate::parts::Stats>,
    },
    /// What a scene node does when played (see [`crate::behavior`]).
    Behavior {
        id: String,
        behavior: crate::asset::Slot<crate::behavior::Behavior>,
    },
    /// A game's rules (see [`crate::game`]).
    Rules {
        id: String,
        rules: crate::asset::Slot<crate::game::Rules>,
    },
    /// The assets a whole game uses (see [`crate::game::GameSettings`]).
    Game {
        id: String,
        game: crate::asset::Slot<crate::game::GameSettings>,
    },
    /// A bitmap font cut from image `from` (see [`crate::font`]): cells of `glyph` pixels in
    /// a grid, one per character of `chars`, `advance` pixels wide each if given.
    Font {
        id: String,
        from: String,
        glyph: (u32, u32),
        chars: String,
        #[serde(default)]
        advance: Option<Vec<u8>>,
    },
    /// A sound: the samples of buffer `from`, stored as `format`, played at `rate` per second.
    Sound {
        id: String,
        from: String,
        rate: u32,
        #[serde(default)]
        format: crate::sound::SampleFormat,
    },
    /// Text shown a page at a time in a box (see [`crate::ui::Dialog`]): written in place, or
    /// with `from`, the text of a buffer (see [`Step::Text`]), `lines_per_page` lines a page
    /// (a page each blank line without it).
    Dialog {
        id: String,
        #[serde(default = "empty_dialog")]
        dialog: crate::asset::Slot<crate::ui::Dialog>,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        lines_per_page: Option<u32>,
    },
    /// The sounds a character makes (see [`crate::ui::SoundSet`]).
    Sounds {
        id: String,
        sounds: crate::asset::Slot<crate::ui::SoundSet>,
    },
    /// A scene made of the placement lists `from` (see [`Step::Places`]): each placement a node
    /// showing one of this plugin's outputs (or a named point), under its group, with its door.
    /// Positions are pixels, `pixels_per_meter` to a meter. The Workshop adds it to its scene
    /// as a group of nodes.
    Scene {
        id: String,
        from: Vec<String>,
        #[serde(default = "sixteen")]
        pixels_per_meter: f32,
    },
}

fn sixteen() -> f32 {
    16.0
}

impl Output {
    /// The id and kind of outputs that can be placed in a scene (images, models and sprites).
    pub fn item(&self) -> Option<(&str, Option<Kind>)> {
        match self {
            Output::Image { id, kind, .. }
            | Output::Model { id, kind, .. }
            | Output::Sprite { id, kind, .. } => Some((id.as_str(), *kind)),
            Output::Scene { id, .. } => Some((id.as_str(), Some(Kind::Scene))),
            Output::Surfaces { .. }
            | Output::Items { .. }
            | Output::Ability { .. }
            | Output::Camera { .. }
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
            | Output::Font { .. }
            | Output::Sound { .. }
            | Output::Dialog { .. }
            | Output::Sounds { .. } => None,
        }
    }
}

/// How an N64 texture's texels are stored (format and size, as the RDP names them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum N64TextureFormat {
    Rgba16,
    Rgba32,
    Ia4,
    Ia8,
    Ia16,
    I4,
    I8,
    Ci4,
    Ci8,
}

/// What a name in a recipe stands for. Each kind has its own names: a buffer and an image may
/// both be called `sheet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    Buffer,
    Bank,
    Image,
    Model,
    Collision,
    Sprite,
    /// A placement list (see [`Step::Places`]).
    Places,
}

impl NameKind {
    pub fn label(self) -> &'static str {
        match self {
            NameKind::Buffer => "buffer",
            NameKind::Bank => "palette bank",
            NameKind::Image => "image",
            NameKind::Model => "model",
            NameKind::Collision => "collision",
            NameKind::Sprite => "sprite",
            NameKind::Places => "placement list",
        }
    }
}

/// How a step uses a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Uses what an earlier step made.
    Read,
    /// Makes it (replacing anything of that name).
    Write,
    /// Changes what an earlier step made.
    Update,
    /// Changes it, making it first if no earlier step did.
    Grow,
}

impl Access {
    pub fn reads(self) -> bool {
        self != Access::Write
    }

    pub fn writes(self) -> bool {
        self != Access::Read
    }
}

/// A step of a path to a value in a step or output, as written in RON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Field(&'static str),
    Index(usize),
    /// A map entry with this (numeric) key.
    Key(u64),
}

/// A name a step or output uses, and where in it the name is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port<'a> {
    pub path: Vec<Seg>,
    pub name: &'a str,
    pub kind: NameKind,
    pub access: Access,
}

fn port<'a>(field: &'static str, name: &'a str, kind: NameKind, access: Access) -> Port<'a> {
    Port {
        path: vec![Seg::Field(field)],
        name,
        kind,
        access,
    }
}

impl Step {
    /// The variant's name, as written in RON.
    pub fn name(&self) -> &'static str {
        match self {
            Step::Read { .. } => "Read",
            Step::Place { .. } => "Place",
            Step::Fill { .. } => "Fill",
            Step::Literal { .. } => "Literal",
            Step::GbaPalettes { .. } => "GbaPalettes",
            Step::GbaMetatiles { .. } => "GbaMetatiles",
            Step::GbaMetatileMap { .. } => "GbaMetatileMap",
            Step::Overlay { .. } => "Overlay",
            Step::Crop { .. } => "Crop",
            Step::Text { .. } => "Text",
            Step::Canvas { .. } => "Canvas",
            Step::Paste { .. } => "Paste",
            Step::N64Texture { .. } => "N64Texture",
            Step::Lookup { .. } => "Lookup",
            Step::N64Model { .. } => "N64Model",
            Step::CollisionTriangles { .. } => "CollisionTriangles",
            Step::ChannelAnimation { .. } => "ChannelAnimation",
            Step::GbaObjectFrames { .. } => "GbaObjectFrames",
            Step::MergeSprite { .. } => "MergeSprite",
            Step::KeyframeAnimation { .. } => "KeyframeAnimation",
            Step::Places { .. } => "Places",
            Step::MarkPlaces { .. } => "MarkPlaces",
        }
    }

    /// Every name the step uses or makes, in the order it lists them.
    pub fn ports(&self) -> Vec<Port<'_>> {
        use Access::*;
        use NameKind::*;
        match self {
            Step::Read { name, .. } | Step::Fill { name, .. } | Step::Literal { name, .. } => {
                vec![port("name", name, Buffer, Write)]
            }
            Step::Place { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Buffer, Grow),
            ],
            Step::GbaPalettes { from, bank, .. } => vec![
                port("from", from, Buffer, Read),
                port("bank", bank, Bank, Grow),
            ],
            Step::GbaMetatiles {
                tiles,
                metatiles,
                bank,
                into,
                ..
            } => vec![
                port("tiles", tiles, Buffer, Read),
                port("metatiles", metatiles, Buffer, Read),
                port("bank", bank, Bank, Read),
                port("into", into, Image, Write),
            ],
            Step::GbaMetatileMap {
                tiles,
                metatiles,
                map,
                bank,
                into,
                ..
            } => vec![
                port("tiles", tiles, Buffer, Read),
                port("metatiles", metatiles, Buffer, Read),
                port("map", map, Buffer, Read),
                port("bank", bank, Bank, Read),
                port("into", into, Image, Write),
            ],
            Step::Overlay { from, into, .. } => vec![
                port("from", from, Image, Read),
                port("into", into, Image, Update),
            ],
            Step::Crop { from, into, .. } => vec![
                port("from", from, Image, Read),
                port("into", into, Image, Write),
            ],
            Step::Text { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Buffer, Write),
            ],
            Step::Canvas { name, .. } => vec![port("name", name, Image, Write)],
            Step::Paste { from, into, .. } => vec![
                port("from", from, Image, Read),
                port("into", into, Image, Update),
            ],
            Step::N64Texture {
                from,
                palette,
                into,
                ..
            } => {
                let mut ports = vec![port("from", from, Buffer, Read)];
                if let Some((buffer, _)) = palette {
                    ports.push(Port {
                        path: vec![Seg::Field("palette"), Seg::Index(0)],
                        name: buffer,
                        kind: Buffer,
                        access: Read,
                    });
                }
                ports.push(port("into", into, Image, Write));
                ports
            }
            Step::Lookup {
                indices,
                table,
                into,
                ..
            } => vec![
                port("indices", indices, Buffer, Read),
                port("table", table, Buffer, Read),
                port("into", into, Buffer, Write),
            ],
            Step::N64Model { segments, into, .. } => segments
                .iter()
                .map(|(segment, name)| Port {
                    path: vec![Seg::Field("segments"), Seg::Key(*segment as u64)],
                    name,
                    kind: Buffer,
                    access: Read,
                })
                .chain([port("into", into, Model, Write)])
                .collect(),
            Step::CollisionTriangles { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Collision, Write),
            ],
            Step::ChannelAnimation { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Model, Update),
            ],
            Step::GbaObjectFrames {
                tiles,
                bank,
                objects,
                frames,
                into,
                ..
            } => {
                let mut ports = vec![
                    port("tiles", tiles, Buffer, Read),
                    port("bank", bank, Bank, Read),
                    port("objects", objects, Buffer, Read),
                    Port {
                        path: vec![
                            Seg::Field("frames"),
                            Seg::Field("lists"),
                            Seg::Field("buffer"),
                        ],
                        name: &frames.lists.buffer,
                        kind: Buffer,
                        access: Read,
                    },
                ];
                for (field, table) in [("tile_base", &frames.tile_base), ("counts", &frames.counts)]
                {
                    if let Some(table) = table {
                        ports.push(Port {
                            path: vec![
                                Seg::Field("frames"),
                                Seg::Field(field),
                                Seg::Field("buffer"),
                            ],
                            name: &table.buffer,
                            kind: Buffer,
                            access: Read,
                        });
                    }
                }
                if let Some(palettes) = &frames.palettes {
                    for (path, name) in [
                        (vec![Seg::Field("buffer")], &palettes.buffer),
                        (
                            vec![Seg::Field("first"), Seg::Field("buffer")],
                            &palettes.first.buffer,
                        ),
                    ] {
                        let mut full = vec![Seg::Field("frames"), Seg::Field("palettes")];
                        full.extend(path);
                        ports.push(Port {
                            path: full,
                            name,
                            kind: Buffer,
                            access: Read,
                        });
                    }
                }
                ports.push(port("into", into, Sprite, Write));
                ports
            }
            Step::MergeSprite { from, into } => vec![
                port("from", from, Sprite, Read),
                port("into", into, Sprite, Update),
            ],
            Step::KeyframeAnimation { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Sprite, Update),
            ],
            Step::Places { from, into, .. } => vec![
                port("from", from, Buffer, Read),
                port("into", into, Places, Grow),
            ],
            Step::MarkPlaces { from, into, .. } => vec![
                port("from", from, Places, Read),
                port("into", into, Buffer, Grow),
            ],
        }
    }
}

impl Output {
    /// The variant's name, as written in RON.
    pub fn name(&self) -> &'static str {
        match self {
            Output::Image { .. } => "Image",
            Output::Surfaces { .. } => "Surfaces",
            Output::Model { .. } => "Model",
            Output::Items { .. } => "Items",
            Output::Ability { .. } => "Ability",
            Output::Sprite { .. } => "Sprite",
            Output::Camera { .. } => "Camera",
            Output::Hud { .. } => "Hud",
            Output::Ui { .. } => "Ui",
            Output::Menu { .. } => "Menu",
            Output::Flow { .. } => "Flow",
            Output::InputMap { .. } => "InputMap",
            Output::Movement { .. } => "Movement",
            Output::Animations { .. } => "Animations",
            Output::Stats { .. } => "Stats",
            Output::Behavior { .. } => "Behavior",
            Output::Rules { .. } => "Rules",
            Output::Game { .. } => "Game",
            Output::Font { .. } => "Font",
            Output::Sound { .. } => "Sound",
            Output::Dialog { .. } => "Dialog",
            Output::Sounds { .. } => "Sounds",
            Output::Scene { .. } => "Scene",
        }
    }

    /// The output's id.
    pub fn id(&self) -> &str {
        match self {
            Output::Image { id, .. }
            | Output::Surfaces { id, .. }
            | Output::Model { id, .. }
            | Output::Items { id, .. }
            | Output::Ability { id, .. }
            | Output::Sprite { id, .. }
            | Output::Camera { id, .. }
            | Output::Hud { id, .. }
            | Output::Ui { id, .. }
            | Output::Menu { id, .. }
            | Output::Flow { id, .. }
            | Output::InputMap { id, .. }
            | Output::Movement { id, .. }
            | Output::Animations { id, .. }
            | Output::Stats { id, .. }
            | Output::Behavior { id, .. }
            | Output::Rules { id, .. }
            | Output::Game { id, .. }
            | Output::Font { id, .. }
            | Output::Sound { id, .. }
            | Output::Dialog { id, .. }
            | Output::Sounds { id, .. }
            | Output::Scene { id, .. } => id,
        }
    }

    /// Renames the output.
    pub fn set_id(&mut self, new: &str) {
        match self {
            Output::Image { id, .. }
            | Output::Surfaces { id, .. }
            | Output::Model { id, .. }
            | Output::Items { id, .. }
            | Output::Ability { id, .. }
            | Output::Sprite { id, .. }
            | Output::Camera { id, .. }
            | Output::Hud { id, .. }
            | Output::Ui { id, .. }
            | Output::Menu { id, .. }
            | Output::Flow { id, .. }
            | Output::InputMap { id, .. }
            | Output::Movement { id, .. }
            | Output::Animations { id, .. }
            | Output::Stats { id, .. }
            | Output::Behavior { id, .. }
            | Output::Rules { id, .. }
            | Output::Game { id, .. }
            | Output::Font { id, .. }
            | Output::Sound { id, .. }
            | Output::Dialog { id, .. }
            | Output::Sounds { id, .. }
            | Output::Scene { id, .. } => *id = new.to_string(),
        }
    }

    /// The names the output is made from (all read). An ability is made from another output,
    /// not from a name, so it has none.
    pub fn ports(&self) -> Vec<Port<'_>> {
        use Access::*;
        use NameKind::*;
        match self {
            Output::Image { from, .. } => vec![port("from", from, Image, Read)],
            Output::Surfaces { grids, .. } => grids
                .iter()
                .enumerate()
                .map(|(i, grid)| Port {
                    path: vec![Seg::Field("grids"), Seg::Index(i), Seg::Field("from")],
                    name: &grid.from,
                    kind: Buffer,
                    access: Read,
                })
                .collect(),
            Output::Model { from, terrain, .. } => {
                let mut ports = vec![port("from", from, Model, Read)];
                if let Some(terrain) = terrain {
                    ports.push(Port {
                        path: vec![Seg::Field("terrain"), Seg::Field("from")],
                        name: &terrain.from,
                        kind: Collision,
                        access: Read,
                    });
                }
                ports
            }
            Output::Sprite { from, .. } => vec![port("from", from, Sprite, Read)],
            Output::Items { .. }
            | Output::Ability { .. }
            | Output::Camera { .. }
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
            | Output::Sounds { .. } => vec![],
            Output::Dialog {
                from: Some(from), ..
            } => vec![port("from", from, Buffer, Read)],
            Output::Font { from, .. } => vec![port("from", from, Image, Read)],
            Output::Sound { from, .. } => vec![port("from", from, Buffer, Read)],
            Output::Scene { from, .. } => from
                .iter()
                .enumerate()
                .map(|(i, name)| Port {
                    path: vec![Seg::Field("from"), Seg::Index(i)],
                    name,
                    kind: Places,
                    access: Read,
                })
                .collect(),
        }
    }
}

/// What kind of thing an output is, for sorting and placing it in the Editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    /// A playable map; a surface map of the same size gives it collision.
    Area,
    /// Building blocks of maps, e.g. a metatile sheet.
    Tileset,
    /// A character's or object's graphics.
    Sprite,
    Character,
    Ability,
    Object,
    Audio,
    Camera,
    Hud,
    Ui,
    Menu,
    Flow,
    Input,
    Movement,
    Animation,
    Stats,
    Behavior,
    Rules,
    Game,
    Font,
    Dialog,
    /// Things placed together: the rooms of a world with their enemies and doors.
    Scene,
}

impl Kind {
    pub const ALL: [Kind; 22] = [
        Kind::Area,
        Kind::Tileset,
        Kind::Sprite,
        Kind::Character,
        Kind::Ability,
        Kind::Object,
        Kind::Audio,
        Kind::Camera,
        Kind::Hud,
        Kind::Ui,
        Kind::Menu,
        Kind::Flow,
        Kind::Input,
        Kind::Movement,
        Kind::Animation,
        Kind::Stats,
        Kind::Behavior,
        Kind::Rules,
        Kind::Game,
        Kind::Font,
        Kind::Dialog,
        Kind::Scene,
    ];

    /// Singular label for the Editor, e.g. "Ability".
    pub fn singular(self) -> &'static str {
        match self {
            Kind::Area => "Area",
            Kind::Tileset => "Tileset",
            Kind::Sprite => "Sprite",
            Kind::Character => "Character",
            Kind::Ability => "Ability",
            Kind::Object => "Object",
            Kind::Audio => "Audio",
            Kind::Camera => "Camera",
            Kind::Hud => "HUD",
            Kind::Ui => "UI",
            Kind::Menu => "Menu",
            Kind::Flow => "Flow",
            Kind::Input => "Input map",
            Kind::Movement => "Movement",
            Kind::Animation => "Animations",
            Kind::Stats => "Stats",
            Kind::Behavior => "Behavior",
            Kind::Rules => "Rules",
            Kind::Game => "Game",
            Kind::Font => "Font",
            Kind::Dialog => "Dialog",
            Kind::Scene => "Scene",
        }
    }

    /// Plural label for the Editor, e.g. "Areas".
    pub fn label(self) -> &'static str {
        match self {
            Kind::Area => "Areas",
            Kind::Tileset => "Tilesets",
            Kind::Sprite => "Sprites",
            Kind::Character => "Characters",
            Kind::Ability => "Abilities",
            Kind::Object => "Objects",
            Kind::Audio => "Audio",
            Kind::Camera => "Cameras",
            Kind::Hud => "HUDs",
            Kind::Ui => "UI",
            Kind::Menu => "Menus",
            Kind::Flow => "Flows",
            Kind::Input => "Input",
            Kind::Movement => "Movement",
            Kind::Animation => "Animations",
            Kind::Stats => "Stats",
            Kind::Behavior => "Behaviors",
            Kind::Rules => "Rules",
            Kind::Game => "Games",
            Kind::Font => "Fonts",
            Kind::Dialog => "Dialogs",
            Kind::Scene => "Scenes",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_flow_output_round_trips() {
        let text = r#"Flow(id: "sm64", flow: (
            title: Some((logo: Some("art/logo"), prompt: "words/start", background: (1, 2, 3, 255))),
            files: Some((title: (pages: ["SELECT FILE"]), slots: 4, counts: Some("star"))),
            acts: Some((courses: [
                (name: "words/course", entry: "node name", acts: ["words/act", (pages: ["ACT 2"])])
            ])),
            game_over: Some((text: (pages: ["GAME OVER"]))),
        ))"#;
        let mut output: Output = ron::from_str(text).unwrap();
        let written = ron::to_string(&output).unwrap();
        assert_eq!(ron::from_str::<Output>(&written).unwrap(), output);
        assert_eq!(output.name(), "Flow");
        assert_eq!(output.id(), "sm64");
        assert!(output.item().is_none());
        assert!(output.ports().is_empty());
        output.set_id("copy");
        assert_eq!(output.id(), "copy");
        let alias: Output = ron::from_str(r#"Flow(id: "alias", flow: "p/sm64")"#).unwrap();
        assert_eq!(
            ron::from_str::<Output>(&ron::to_string(&alias).unwrap()).unwrap(),
            alias
        );
        assert!(Kind::ALL.contains(&Kind::Flow));
        assert_eq!(Kind::Flow.singular(), "Flow");
        assert_eq!(Kind::Flow.label(), "Flows");
    }

    #[test]
    fn fields_read_packed_numbers() {
        // 0xC0FF in little-endian: bits 14-15 = 3, low 9 bits = 0x0FF, 9-bit signed = -1 for 0x1FF.
        let record = [0xFF, 0xC0, 0xFF, 0x01];
        assert_eq!(
            Field::new(0, FieldSize::U16).bits(14, 2).read(&record),
            Some(3)
        );
        assert_eq!(Field::new(0, FieldSize::U8).read(&record), Some(0xFF));
        assert_eq!(
            Field::new(0, FieldSize::U8).signed().read(&record),
            Some(-1)
        );
        assert_eq!(
            Field::new(2, FieldSize::U16)
                .bits(0, 9)
                .signed()
                .read(&record),
            Some(-1)
        );
        assert_eq!(
            Field::new(2, FieldSize::U16).bits(0, 9).read(&record),
            Some(0x1FF)
        );
        assert_eq!(
            Field::new(0, FieldSize::U32).read(&record),
            Some(0x01FF_C0FF)
        );
        assert_eq!(Field::new(3, FieldSize::U16).read(&record), None);
    }

    #[test]
    fn tables_read_every_record() {
        let table = Table {
            buffer: "t".into(),
            offset: 1,
            stride: 3,
            count: 2,
            field: Field::new(1, FieldSize::U8),
        };
        assert_eq!(table.values(&[9, 0, 7, 0, 0, 8, 0]), Some(vec![7, 8]));
        assert_eq!(table.values(&[9, 0, 7, 0, 0]), None);
        let repeated = Table {
            stride: 0,
            count: u32::MAX,
            ..table
        };
        assert_eq!(repeated.values(&[9, 0, 7, 0, 0, 8, 0]), None);
    }

    #[test]
    fn the_oam_layout_matches_the_hardware() {
        let fields = ObjectLayout::Oam.fields();
        // attr0: y = 0xF0 (-16), shape 2. attr1: x = 0x1F8 (-8), hflip, size 1. attr2: tile 5, palette 2.
        let record = [0xF0, 0x80, 0xF8, 0x51, 0x05, 0x20];
        assert_eq!(fields.y.read(&record), Some(-16));
        assert_eq!(fields.shape.read(&record), Some(2));
        assert_eq!(fields.x.read(&record), Some(-8));
        assert_eq!(fields.hflip.unwrap().read(&record), Some(1));
        assert_eq!(fields.vflip.unwrap().read(&record), Some(0));
        assert_eq!(fields.size.read(&record), Some(1));
        assert_eq!(fields.tile.read(&record), Some(5));
        assert_eq!(fields.palette.unwrap().read(&record), Some(2));
    }

    #[test]
    fn parses_sprite_steps_and_outputs() {
        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    GbaObjectFrames(tiles: "t", bank: "obj", objects: "o", frames: (
                        lists: (buffer: "o", offset: 0, stride: 4, count: 2, field: (at: 0, size: U32)),
                    ), into: "s"),
                    KeyframeAnimation(from: "r", name: "a", at: Pointer(table: 0, index: 1, base: 0x08000000),
                        layout: (stride: 4, frame: (at: 0), ticks: (at: 1),
                                 end: (at: 3, shift: 7, bits: 1), after_end: RewindByte),
                        fps: 60.0, into: "s"),
                ],
                outputs: [Sprite(id: "s", from: "s", kind: Some(Character), pixels_per_meter: 16.0)],
            )"#,
        )
        .unwrap();
        assert!(matches!(recipe.steps[0], Step::GbaObjectFrames { .. }));
        assert_eq!(recipe.outputs[0].item(), Some(("s", Some(Kind::Character))));
    }

    #[test]
    fn parses_ron() {
        let recipe: Recipe = ron::from_str(
            r#"Recipe(
                steps: [
                    Read(name: "tiles", offset: 0x100, codec: GbaLz77),
                    Read(name: "pal", offset: 0x200, size: Some(32)),
                    Place(from: "tiles", into: "vram", at: 0x4000),
                    GbaPalettes(from: "pal", bank: "bg", slot: 2),
                    GbaMetatiles(tiles: "vram", char_base: 0, metatiles: "meta", bank: "bg", columns: 16, into: "sheet"),
                ],
                outputs: [Image(id: "tileset", from: "sheet")],
            )"#,
        )
        .unwrap();
        assert_eq!(recipe.steps.len(), 5);
        assert_eq!(
            recipe.steps[1],
            Step::Read {
                name: "pal".into(),
                offset: 0x200,
                size: Some(32),
                codec: Codec::None
            }
        );
    }
}

//! What each step and output of a recipe holds, field by field, for editors: so an offset is
//! written in hex, a codec is picked from a list and a value that may be left out can be.
//!
//! It describes the format only (no game appears here). Names a step uses or makes are listed
//! too; editors show them as wires (see [`crate::recipe::Step::ports`]).

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// A byte offset or address, written in hex.
    Offset,
    /// A count, size, index or other whole number.
    Count,
    /// A number with a fraction.
    Number,
    Flag,
    /// One of these variants, written bare.
    Choice(&'static [&'static str]),
    /// A value that may be left out (`None`), or `Some` of the inner type.
    Optional(&'static FieldType),
    /// A name of a buffer, bank, image, model, collision or sprite.
    Name,
    Text,
    /// Anything bigger (a record layout, a list, a map), edited as RON.
    Structure,
}

impl FieldType {
    /// The value a field of this type starts with when it is added.
    pub fn default_text(self) -> &'static str {
        match self {
            FieldType::Offset => "0x0",
            FieldType::Count => "0",
            FieldType::Number => "1.0",
            FieldType::Flag => "false",
            FieldType::Choice(variants) => variants[0],
            FieldType::Optional(_) => "None",
            FieldType::Name | FieldType::Text => "\"\"",
            FieldType::Structure => "()",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldInfo {
    pub key: &'static str,
    pub ty: FieldType,
    /// Whether it can be left out of the text (it has a default).
    pub optional: bool,
    pub about: &'static str,
}

/// A step or output variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantInfo {
    pub name: &'static str,
    /// One line saying what it does.
    pub about: &'static str,
    pub fields: &'static [FieldInfo],
}

impl VariantInfo {
    pub fn field(&self, key: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.key == key)
    }
}

use FieldType::*;

const fn f(key: &'static str, ty: FieldType, about: &'static str) -> FieldInfo {
    FieldInfo {
        key,
        ty,
        optional: false,
        about,
    }
}

/// A field with a default, so it may be missing from the text.
const fn d(key: &'static str, ty: FieldType, about: &'static str) -> FieldInfo {
    FieldInfo {
        key,
        ty,
        optional: true,
        about,
    }
}

const CODECS: &[&str] = &["None", "GbaLz77", "Popuri", "Mio0"];
const ELEMENTS: &[&str] = &["U8", "U16"];
const MICROCODES: &[&str] = &["F3d"];
const KINDS: &[&str] = &[
    "Area",
    "Tileset",
    "Sprite",
    "Character",
    "Ability",
    "Object",
    "Audio",
    "Camera",
    "Hud",
    "Ui",
    "Menu",
    "Flow",
    "Input",
    "Movement",
    "Animation",
    "Stats",
    "Behavior",
    "Rules",
    "Game",
    "Font",
    "Dialog",
    "Scene",
];
const DEPTHS: &[&str] = &["Bpp4", "Bpp8"];
const COUNT: FieldType = Count;
const KIND: FieldType = Choice(KINDS);
const TEXT: FieldType = Text;
const STRUCTURE: FieldType = Structure;

pub const STEPS: &[VariantInfo] = &[
    VariantInfo {
        name: "Read",
        about: "Reads bytes from the ROM into a buffer, decompressing them if a codec is set.",
        fields: &[
            f("name", Name, "The buffer it makes"),
            f("offset", Offset, "Where the bytes start in the ROM"),
            d(
                "size",
                Optional(&COUNT),
                "How many bytes; compressed data knows its own size",
            ),
            d("codec", Choice(CODECS), "How the bytes are compressed"),
        ],
    },
    VariantInfo {
        name: "Place",
        about: "Copies a buffer into another at a byte offset, growing it as needed (e.g. VRAM).",
        fields: &[
            f("from", Name, "The buffer copied"),
            f("into", Name, "The buffer copied into"),
            f("at", Offset, "Where in it"),
        ],
    },
    VariantInfo {
        name: "Fill",
        about: "Makes a buffer of a size, every byte one value.",
        fields: &[
            f("name", Name, "The buffer it makes"),
            f("size", Count, "Bytes"),
            d("value", Count, "Every byte's value"),
        ],
    },
    VariantInfo {
        name: "Literal",
        about: "Makes a small buffer from hex bytes (tables and markers, never game data).",
        fields: &[
            f("name", Name, "The buffer it makes"),
            f("hex", Text, "Bytes such as 00 01 FF"),
        ],
    },
    VariantInfo {
        name: "GbaPalettes",
        about: "Loads BGR555 palettes from a buffer into a palette bank.",
        fields: &[
            f("from", Name, "The buffer of colors"),
            f("bank", Name, "The palette bank"),
            f("slot", Count, "The first palette it fills"),
        ],
    },
    VariantInfo {
        name: "GbaMetatiles",
        about: "Draws a sheet of 16×16 metatiles (four GBA tile entries each) into an image.",
        fields: &[
            f("tiles", Name, "The tile graphics"),
            f(
                "char_base",
                Offset,
                "Where tile 0 starts in the tiles, in bytes",
            ),
            f("metatiles", Name, "The metatile definitions"),
            f("bank", Name, "The palettes"),
            f("columns", Count, "Metatiles per row of the sheet"),
            f("into", Name, "The image it makes"),
            d(
                "depth",
                Choice(DEPTHS),
                "Bits per pixel: Bpp4 (16 colors) or Bpp8 (256-color backgrounds)",
            ),
        ],
    },
    VariantInfo {
        name: "GbaMetatileMap",
        about: "Draws a map of metatile numbers into an image.",
        fields: &[
            f("tiles", Name, "The tile graphics"),
            f(
                "char_base",
                Offset,
                "Where tile 0 starts in the tiles, in bytes",
            ),
            f("metatiles", Name, "The metatile definitions"),
            f("map", Name, "The metatile numbers"),
            f("width", Count, "Metatiles per row of the map"),
            f("bank", Name, "The palettes"),
            f("into", Name, "The image it makes"),
            d(
                "depth",
                Choice(DEPTHS),
                "Bits per pixel: Bpp4 (16 colors) or Bpp8 (256-color backgrounds)",
            ),
        ],
    },
    VariantInfo {
        name: "Overlay",
        about: "Draws one image on top of another, optionally only inside a region.",
        fields: &[
            f("from", Name, "The image drawn"),
            f("into", Name, "The image drawn on"),
            d(
                "region",
                Optional(&STRUCTURE),
                "Only this rectangle: (x, y, w, h)",
            ),
            d("replace", Flag, "Copy transparent pixels too"),
        ],
    },
    VariantInfo {
        name: "Crop",
        about: "Copies a rectangle of an image into a new image.",
        fields: &[
            f("from", Name, "The image cropped"),
            f("region", Structure, "The rectangle: (x, y, w, h)"),
            f("into", Name, "The image it makes"),
        ],
    },
    VariantInfo {
        name: "Text",
        about: "Decodes text bytes through a character map, e.g. a game's dialog.",
        fields: &[
            f("from", Name, "The buffer holding the text"),
            d("at", Offset, "Where it starts in the buffer"),
            d(
                "runs",
                Structure,
                "Bytes in a row: [(0x0A, \"ABCDEFGH\")] makes 0x0A \"A\", 0x0B \"B\"",
            ),
            d("codes", Structure, "Single bytes: {0xFE: \"\\n\"}"),
            f("end", Count, "The byte that ends the text"),
            d("length", Count, "Or how many bytes it is: Some(64)"),
            d(
                "other",
                Text,
                "What a byte neither names means; without it, an error",
            ),
            f("into", Name, "The text it makes (a buffer, UTF-8)"),
        ],
    },
    VariantInfo {
        name: "Canvas",
        about: "A new transparent image, to paste into.",
        fields: &[
            f("name", Name, "The image it makes"),
            f("width", Count, "Pixels wide"),
            f("height", Count, "Pixels high"),
        ],
    },
    VariantInfo {
        name: "Paste",
        about: "Draws an image into another at a position (pixels), e.g. glyphs into a font sheet.",
        fields: &[
            f("from", Name, "The image drawn"),
            f("into", Name, "The image drawn into"),
            f("at", Structure, "Its top left corner: (x, y)"),
        ],
    },
    VariantInfo {
        name: "N64Texture",
        about: "Decodes an N64 texture from a buffer into an image.",
        fields: &[
            f("from", Name, "The buffer holding the texels"),
            d("at", Offset, "Where they start in the buffer"),
            f(
                "format",
                Choice(&[
                    "Rgba16", "Rgba32", "Ia4", "Ia8", "Ia16", "I4", "I8", "Ci4", "Ci8",
                ]),
                "How the texels are stored",
            ),
            f("width", Count, "Texels wide"),
            f("height", Count, "Texels high"),
            d(
                "palette",
                Structure,
                "For Ci4 and Ci8: Some((\"buffer\", offset))",
            ),
            f("into", Name, "The image it makes"),
        ],
    },
    VariantInfo {
        name: "Lookup",
        about: "Looks up every number of a buffer in a table: into[i] = table[indices[i]].",
        fields: &[
            f("indices", Name, "The numbers looked up"),
            d(
                "index",
                Choice(ELEMENTS),
                "How big each of those numbers is",
            ),
            f("table", Name, "The table"),
            d("value", Choice(ELEMENTS), "How big each table entry is"),
            f("into", Name, "The buffer it makes"),
        ],
    },
    VariantInfo {
        name: "N64Model",
        about: "Builds a model from N64 display lists, one or more per bone.",
        fields: &[
            d(
                "microcode",
                Choice(MICROCODES),
                "How the display lists are encoded",
            ),
            f("segments", Structure, "The buffer holding each RAM segment"),
            f(
                "bones",
                Structure,
                "Bones, parents first, with the display lists they draw",
            ),
            f("into", Name, "The model it makes"),
        ],
    },
    VariantInfo {
        name: "CollisionTriangles",
        about: "Reads collision triangles (vertices, then groups of triangles by code).",
        fields: &[
            f("from", Name, "The buffer"),
            f("at", Offset, "Where the data starts in it"),
            f("layout", Structure, "How the numbers are stored"),
            f("into", Name, "The collision it makes"),
        ],
    },
    VariantInfo {
        name: "ChannelAnimation",
        about: "Adds an animation to a model, stored as value lists per channel.",
        fields: &[
            f("from", Name, "The buffer"),
            f("name", Text, "The animation's name"),
            f("index", Offset, "Where the channel table starts"),
            f("values", Offset, "Where the values start"),
            f("frames", Count, "How many frames"),
            d("loop_start", Optional(&COUNT), "The frame it loops back to"),
            d("fps", Number, "Frames per second"),
            d(
                "root_translation",
                Flag,
                "The first channels move the root bone",
            ),
            d(
                "bones",
                Optional(&COUNT),
                "How many of the model's bones, in order, it animates (all without)",
            ),
            f("into", Name, "The model it adds to"),
        ],
    },
    VariantInfo {
        name: "GbaObjectFrames",
        about: "Builds a sprite whose frames are drawn from hardware objects (OAM entries).",
        fields: &[
            f("tiles", Name, "The 4bpp tiles"),
            f("bank", Name, "The palettes"),
            f("objects", Name, "The object lists"),
            d("layout", Structure, "How one object is stored"),
            f("frames", Structure, "Where each frame's object list is"),
            f("into", Name, "The sprite it makes"),
        ],
    },
    VariantInfo {
        name: "MergeSprite",
        about: "Adds the frames and animations of one sprite to another.",
        fields: &[
            f("from", Name, "The sprite added"),
            f("into", Name, "The sprite added to"),
        ],
    },
    VariantInfo {
        name: "KeyframeAnimation",
        about: "Adds an animation to a sprite, decoded from (frame, duration, flags) records.",
        fields: &[
            f("from", Name, "The buffer"),
            f("name", Text, "The animation's name"),
            f(
                "at",
                Structure,
                "Where the keyframes start: Offset(…), Pointer(…) or Directory(…)",
            ),
            f("layout", Structure, "How a keyframe is stored"),
            f("fps", Number, "Ticks per second the durations count in"),
            f("into", Name, "The sprite it adds to"),
        ],
    },
    VariantInfo {
        name: "Places",
        about: "Places what a game lists in a table of records (enemies, doors, rooms) into a placement list.",
        fields: &[
            f("from", Name, "The buffer of records"),
            d("at", Offset, "Where the first record starts"),
            f("stride", Count, "Bytes per record"),
            d("count", Optional(&COUNT), "How many records"),
            d(
                "until",
                Optional(&STRUCTURE),
                "The record that ends the list: (field, values)",
            ),
            d(
                "when",
                Structure,
                "Only records whose fields hold these values: [(field, values)]",
            ),
            f("x", Structure, "The field of the x position, in pixels"),
            f("y", Structure, "The field of the y position, in pixels"),
            d(
                "size",
                Optional(&STRUCTURE),
                "Width and height fields: the position is then the top left corner",
            ),
            d("origin", Structure, "Pixels added to every position"),
            d(
                "item",
                Optional(&STRUCTURE),
                "Which output each shows: (by, outputs, other)",
            ),
            f("name", Structure, "Each node's name: (text, fields)"),
            d(
                "group",
                Optional(&STRUCTURE),
                "The group node: (text, fields)",
            ),
            d(
                "door",
                Optional(&STRUCTURE),
                "Where walking into it takes the player: (to, radius, fade)",
            ),
            d(
                "code",
                Optional(&STRUCTURE),
                "The field MarkPlaces writes; the record's number without it",
            ),
            d(
                "extent",
                Optional(&STRUCTURE),
                "How far each reaches, by a field: (by, sizes, other)",
            ),
            f("into", Name, "The placement list"),
        ],
    },
    VariantInfo {
        name: "MarkPlaces",
        about: "Marks the cells placements cover with their codes, as a grid for a surface map.",
        fields: &[
            f("from", Name, "The placement list"),
            f("width", Count, "Cells per row"),
            f("height", Count, "Rows"),
            f("cell_size", Count, "Pixels per cell"),
            d(
                "origin",
                Structure,
                "Where the grid's top left corner is, in pixels",
            ),
            f("into", Name, "The buffer of cells"),
        ],
    },
];

pub const OUTPUTS: &[VariantInfo] = &[
    VariantInfo {
        name: "Image",
        about: "An image the plugin provides.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Name, "The image"),
            d(
                "kind",
                Optional(&KIND),
                "Where the content browser lists it",
            ),
        ],
    },
    VariantInfo {
        name: "Surfaces",
        about: "A surface map (what each cell of an area is) built from grids of codes.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("width", Count, "Cells per row"),
            f("cell_size", Count, "How big a cell is, in native units"),
            f("grids", Structure, "The grids of codes and what they mean"),
            d("extensions", Structure, "Surfaces this game adds"),
            d(
                "image",
                Optional(&TEXT),
                "The image output it covers (else one of the same size)",
            ),
        ],
    },
    VariantInfo {
        name: "Model",
        about: "A model with its animations; playable with a character, walkable with terrain.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Name, "The model"),
            d(
                "kind",
                Optional(&KIND),
                "Where the content browser lists it",
            ),
            f("meters_per_unit", Number, "How big a model unit is"),
            d(
                "character",
                Optional(&STRUCTURE),
                "How it moves when played",
            ),
            d(
                "terrain",
                Optional(&STRUCTURE),
                "Its collision, for an area",
            ),
        ],
    },
    VariantInfo {
        name: "Sprite",
        about: "A sprite with its animations; playable from above with a character.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Name, "The sprite"),
            d(
                "kind",
                Optional(&KIND),
                "Where the content browser lists it",
            ),
            f(
                "pixels_per_meter",
                Number,
                "How big a pixel is in the world",
            ),
            d(
                "character",
                Optional(&STRUCTURE),
                "How it moves when played",
            ),
        ],
    },
    VariantInfo {
        name: "Items",
        about: "The items characters of the plugin can carry.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("items", Structure, "The items"),
        ],
    },
    VariantInfo {
        name: "Ability",
        about: "An ability other characters can learn, made of a character's actions.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("name", Text, "Its name"),
            d("description", Text, "What it does"),
            f("from", Text, "The character output it comes from"),
            f("actions", Structure, "The actions it is made of"),
            d("items", Structure, "Items that start it"),
            d("dodge", Flag, "It includes the character's dodge"),
        ],
    },
    VariantInfo {
        name: "Camera",
        about: "A camera characters can name: Follow(...), TopDown(...) or another camera's name.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "camera",
                Structure,
                "The camera, or \"plugin/output\" of another",
            ),
        ],
    },
    VariantInfo {
        name: "Hud",
        about: "A HUD characters can name: (widgets: [(kind: ItemSlot(B), anchor: BottomLeft, offset: (8, 8)), (kind: Lives, anchor: TopLeft, icon: Some(\"plugin/head\"), format: Some(\"x{n}\"))], words: (got: \"\", win: \"YOU WIN\")).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "hud",
                Structure,
                "The widgets, or \"plugin/output\" of another HUD",
            ),
        ],
    },
    VariantInfo {
        name: "Ui",
        about: "A UI theme: panel, border, text, dim, accent and shade colours, frame, font and font_size (8 by default). Optional dialog_font and dialog_font_size override the font and size for dialogs; omitted values inherit font and font_size.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "ui",
                Structure,
                "The theme, or \"plugin/output\" of another",
            ),
        ],
    },
    VariantInfo {
        name: "Menu",
        about: "A menu: (kind: Inventory(columns: 6) or Pause, title: \"Items\", opens_with: \"Menu\").",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "menu",
                Structure,
                "The menu, or \"plugin/output\" of another",
            ),
        ],
    },
    VariantInfo {
        name: "Flow",
        about: "Screens around the world: optional title (logo, prompt, background), files (title, slots, counts), acts (courses with name, entry and acts), and game_over (text). Labels are dialogs written in place or named by asset path.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "flow",
                Structure,
                "The screens, or \"plugin/output\" of another Flow; title, files, acts and game_over each use Some(...) or None",
            ),
        ],
    },
    VariantInfo {
        name: "InputMap",
        about: "Which keys and pad buttons press which game button.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "input",
                Structure,
                "The bindings, or \"plugin/output\" of another map",
            ),
        ],
    },
    VariantInfo {
        name: "Movement",
        about: "How characters move: TopDown((speed, radius, ...)) or Platformer((walk_speed, ...)).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "movement",
                Structure,
                "The movement, or \"plugin/output\" of another",
            ),
        ],
    },
    VariantInfo {
        name: "Animations",
        about: "Which animations play for what: Directional((idle, walk)) or States((...)).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "animations",
                Structure,
                "The animations, or \"plugin/output\" of others",
            ),
        ],
    },
    VariantInfo {
        name: "Stats",
        about: "A character's health and how long it can't be hurt again: (health: 6.0, hurt_time: 1.0).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "stats",
                Structure,
                "The stats, or \"plugin/output\" of others",
            ),
        ],
    },
    VariantInfo {
        name: "Behavior",
        about: "What a scene node does when played: Target(...), Pickup(...), Trigger(...), Checkpoint(...), Solid(...), Hazard(radius, damage) or Door(radius, to, fade).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "behavior",
                Structure,
                "The behavior, or \"plugin/output\" of another",
            ),
        ],
    },
    VariantInfo {
        name: "Rules",
        about: "A game's rules: (when_down: StartOver | LastCheckpoint | StopPlaying, lives, time_limit, goal: Collect(item, count) | Reach(node) | DestroyAll | When(condition)).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "rules",
                Structure,
                "The rules, or \"plugin/output\" of others",
            ),
        ],
    },
    VariantInfo {
        name: "Game",
        about: "The assets a whole game uses: (rules, flow, camera, hud, ui, input, menus, stats, inventory), each \"plugin/output\".",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "game",
                Structure,
                "The settings, or \"plugin/output\" of others",
            ),
        ],
    },
    VariantInfo {
        name: "Font",
        about: "A bitmap font cut from an image: glyph cells in a grid, one per character.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Name, "The image of the glyphs"),
            f(
                "glyph",
                Structure,
                "Width and height of a glyph cell: (8, 8)",
            ),
            f(
                "chars",
                Text,
                "The characters of the cells, in reading order",
            ),
            d(
                "advance",
                Structure,
                "How wide each glyph is: Some([8, 6, ...])",
            ),
        ],
    },
    VariantInfo {
        name: "Sound",
        about: "A sound: samples read from the ROM.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Name, "The buffer of samples"),
            f("rate", Count, "Samples per second"),
            d(
                "format",
                Choice(&["S8", "U8", "S16"]),
                "How the samples are stored",
            ),
        ],
    },
    VariantInfo {
        name: "Dialog",
        about: "Text shown a page at a time: (speaker: Some(\"Zelda\"), pages: [\"...\"]).",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            d(
                "dialog",
                Structure,
                "The dialog, or \"plugin/output\" of another",
            ),
            d(
                "from",
                Name,
                "Its pages from the text of a buffer (see Text) instead",
            ),
            d(
                "lines_per_page",
                Count,
                "Lines a page of that text; without it, blank lines",
            ),
        ],
    },
    VariantInfo {
        name: "Sounds",
        about: "The sounds a character makes: (hurt, pickup, door, menu), each Some(\"plugin/sound\").",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f(
                "sounds",
                Structure,
                "The sounds, or \"plugin/output\" of others",
            ),
        ],
    },
    VariantInfo {
        name: "Scene",
        about: "Things placed together from placement lists; the Workshop adds them as a group.",
        fields: &[
            f("id", Text, "Its id in the plugin"),
            f("from", Structure, "The placement lists: [\"name\", …]"),
            d("pixels_per_meter", Number, "Pixels to a meter (default 16)"),
        ],
    },
];

pub fn step(name: &str) -> Option<&'static VariantInfo> {
    STEPS.iter().find(|v| v.name == name)
}

pub fn output(name: &str) -> Option<&'static VariantInfo> {
    OUTPUTS.iter().find(|v| v.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_name_real_variants() {
        for codec in CODECS {
            let parsed: crate::recipe::Codec = ron::from_str(codec).unwrap();
            assert_eq!(&format!("{parsed:?}"), codec);
        }
        for element in ELEMENTS {
            let _: crate::recipe::Element = ron::from_str(element).unwrap();
        }
        for kind in KINDS {
            let _: crate::recipe::Kind = ron::from_str(kind).unwrap();
        }
        for depth in DEPTHS {
            let _: crate::recipe::TileDepth = ron::from_str(depth).unwrap();
        }
    }
}

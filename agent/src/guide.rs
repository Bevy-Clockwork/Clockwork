//! Text the agent reads: the server instructions and the recipe reference.

pub const INSTRUCTIONS: &str = "\
Clockwork extracts content from games the user owns into shareable plugins. A plugin is a \
recipe: offsets and parameters that turn the user's own ROM into images and surface maps. \
Plugins never contain game data.

The Library recognizes GBA, N64, SNES, Nintendo DS, PS1, PS2 and PC game files. Import support \
does not imply a decoder or recipe for every game. SNES bytes exclude the 512-byte copier \
header; N64 bytes are big-endian. DS and PC bytes are unchanged. PlayStation disc bytes \
retain their source layout (2048-byte ISO or 2352-byte raw sectors); CUE imports use the \
referenced binary file starting at the data track's INDEX 01. Recipe offsets refer to those \
bytes, not CPU addresses. PC import reads an executable, not its entire installation.

Workflow:
1. list_games: pick the game. Note its decomp link, if any.
2. Learn the game: list_knowledge / read_knowledge for notes, list_plugins / read_plugin for \
existing recipes (complete ones make the best examples), and the decomp if linked \
(decomp_sync once, then decomp_list, decomp_search, decomp_read). Decomp asset tables and \
headers usually give exact offsets.
3. Check facts against the ROM: read_rom, search_rom, decompress.
4. Write a recipe (recipe_guide explains the format) and run_recipe it. Look at the previews: \
wrong palettes, shuffled tiles or garbage mean an offset or base is wrong. Iterate.
5. save_plugin once the output looks right. Then add what you learned to the game's knowledge \
notes so the next extraction is faster (tell the user; you can't write knowledge files).
6. To change a saved plugin, use edit_plugin with just the changes (set a value, insert, \
replace, move or remove a step or output, write a step's note) instead of saving the whole \
recipe again: it keeps the recipe's comments and layout, runs the result and only saves it \
if it works. Notes (// lines right above a step) are what the user reads in the editor.
7. A plugin with a Scene output can be played without the user: play_test walks a character \
through it from a script and reports what happened (see \"Play-testing a scene\" in \
recipe_guide). Use it to check your own work: the player reaches the goal, doors lead \
somewhere, enemies hurt and can be beaten, nothing that should be reachable is out of reach.

Tools for external programs (git, Ghidra, …) live in their own folder; list_external_tools \
shows what is available.";

pub const RECIPE_GUIDE: &str = r#"# Recipe format (recipe.ron)

A recipe is RON: `Recipe(steps: [...], outputs: [...])`. Steps run in order and work on named
values: buffers (bytes), palette banks, images. Comments (`//`) are allowed; use them to cite
where each offset comes from. In a recipe of more than a few dozen steps, start each part with a
section comment such as `// == Title screen ==`: the editor folds each section into one node.

## Steps

Read(name: "x", offset: 0x123400, codec: GbaLz77)
    Reads from the ROM into buffer x. Compressed data knows its size. Codecs: GbaLz77 (GBA BIOS
    LZ77), Popuri (0x70 Huffman/LZSS/delta), Mio0 (N64, header "MIO0").
Read(name: "x", offset: 0x200000, size: Some(160))
    Uncompressed reads need a size. codec defaults to None.
Place(from: "x", into: "vram", at: 0x4000)
    Copies buffer x into buffer vram at a byte offset (grows vram). Use it to assemble VRAM.
    Place(from: "vram", into: "vram_b", at: 0) copies a whole buffer.
Fill(name: "zeros", size: 64, value: 0)
    A buffer of `size` bytes, all `value` (default 0).
Literal(name: "marker", hex: "01 00 01 00")
    A small buffer written as hex: markers, tiny tables. Never game data.
Lookup(indices: "map", index: U16, table: "types", value: U16, into: "cell_types")
    into[i] = table[indices[i]]. index/value are U8 or U16 (default U8). Chain lookups to turn
    per-cell numbers into per-cell attributes.
GbaPalettes(from: "pal", bank: "bg", slot: 2)
    Loads BGR555 16-color palettes from buffer pal into bank bg starting at palette slot 2.
    Later loads overwrite earlier ones, like the hardware.
GbaMetatiles(tiles: "vram", char_base: 0x0000, metatiles: "meta", bank: "bg", columns: 16, into: "sheet")
    Renders 16x16 metatiles (4 tile-map entries each: TL, TR, BL, BR; 8 bytes) as a sheet.
    Tile numbers count from char_base bytes into the tiles buffer; 4bpp, 32 bytes per tile.
GbaMetatileMap(tiles: "vram", char_base: 0, metatiles: "meta", map: "room", width: 63, bank: "bg", into: "img")
    Renders a map of u16 metatile numbers, width metatiles per row. Both steps take
    `depth: Bpp8` for backgrounds in 256-color mode (64-byte tiles, colors from the whole bank;
    the BG control register's bit 7). Games that keep a background as a plain tile map of 32x32
    screen blocks can still use it: regroup the entries into 16x16 metatiles with a Lookup
    (indices: the four entries' positions of each metatile) and draw them with a map 0, 1, 2...
Overlay(from: "top", into: "img")
    Draws image top over image img (both must exist), skipping transparent pixels.
Overlay(from: "state_b", into: "img", region: Some((x: 0, y: 512, w: 384, h: 448)), replace: true)
    Only inside the rectangle (pixels; images line up at 0,0). With replace, the region shows
    exactly `from`, transparent pixels included. Use it when different parts of a map need
    different graphics (see "Graphics that change at runtime" below).
Crop(from: "sheet", region: (x: 0, y: 0, w: 32, h: 32), into: "sprite")
    A new image from a rectangle of another.
Canvas(name: "sheet", width: 128, height: 64)
    A new transparent image, to paste into.
Paste(from: "glyph_a", into: "sheet", at: (16, 0))
    Draws an image into another with its top left corner at (x, y), e.g. glyphs into a font
    sheet. Pixels outside the target are left out.
N64Texture(from: "seg02", at: 0x5800, format: Rgba16, width: 16, height: 16, into: "coin")
    Decodes a raw N64 texture (HUD icons, fonts, anything not drawn by a display list).
    Formats: Rgba16, Rgba32, Ia4, Ia8, Ia16, I4, I8, Ci4, Ci8; the colour-indexed ones read
    RGBA16 palette entries from `palette: Some(("buffer", offset))`. Textures stored one after
    another decode as one tall strip: width 16, height 16 * count.
Text(from: "seg02", at: 0x10F68, runs: [(0x00, "0123456789ABCDEF")], codes: {0xFE: "\n", 0x9E: " "}, end: 0xFF, into: "sign")
    Decodes text through the game's character map into a buffer (UTF-8): `runs` give bytes in
    a row a character each, `codes` single bytes any text; a byte neither names is an error
    unless `other: Some("?")`. Dialog outputs show it (`from`).
MergeSprite(from: "sheet2", into: "hero")
    Appends sprite sheet2's frames and animations to sprite hero (animation names must be
    unique). Games draw one character from several sheets of frames (a dodge drawn in another
    sheet than the walk); build each with GbaObjectFrames and add its animations to it, then merge.

### 3D models (N64)

N64 ROMs are big-endian here (byte order is normalized on import). RAM addresses in display
lists are segmented: 0x04001234 = offset 0x1234 of segment 4. Games load segments from ROM (often
MIO0-compressed); the game's level scripts or decomp say which ROM range becomes which segment.

N64Model(microcode: F3d, segments: {4: "seg04"}, bones: [
    (name: "root", parent: None, at: (0, 0, 0), draw: []),
    (name: "torso", parent: Some("root"), at: (68, 0, 0), draw: [0x04010370]),
    (name: "wings", parent: Some("torso"), draw: [0x0401CA00], blend: Cutout),
], into: "model")
    Builds a model: a tree of bones (parents listed first), each drawing display lists. `at` is
    the offset from the parent in model units. blend: Opaque (default), Cutout or Transparent.
    Lighting, vertex colors and textures come from the display lists. Microcodes: F3d (Fast3D,
    used by early N64 games). In games with scene graphs (geo layouts), animated parts are
    the bones; pick one case of each switch (LOD, eyes, hands) and list them in graph order.
    `billboard: true` makes a bone's geometry face the camera (geo billboard nodes: trees,
    coins, flames); give each billboard its own bone at its position, after the animated
    parts, so animations don't shift (see ChannelAnimation's `bones`). Models are drawn as the
    hardware drew them: unlit vertex colors, the display list's own lights, environment maps.
CollisionTriangles(from: "seg07", at: 0x4000, into: "ground",
    layout: (word: U16, big_endian: true, start: Some(0x40), end: 0x41, extra: { 0x0E: 1 }))
    Reads collision triangles into collision `ground`: a vertex count and that many signed
    (x, y, z) vertices, then groups of (code, count, count triangles of three vertex indices),
    until the `end` code. Each triangle's code is its group's (usually a surface type). `word`
    is the width of every number (U8, U16, U32), little-endian unless `big_endian`; `start` is
    a marker the data must begin with, if the game has one; `extra` lists codes whose triangles
    carry more words after their indices (how many), which are skipped. Read every value from
    the game's own collision loader. See "3D areas" below for what to do with it.
ChannelAnimation(from: "anim", name: "walk", index: 24, values: 276, frames: 77,
                 loop_start: Some(0), fps: 30.0, root_translation: true, into: "model")
    Adds an animation to a model. Buffer anim holds a table of (count, first value) BE u16
    pairs at byte `index` and BE i16 values at byte `values`: one channel per pair, the root's
    translation x, y, z first (with root_translation), then x, y, z rotations of every bone in
    order (65536 = one turn, applied x, then y, then z). Short channels hold their last value.
    loop_start None plays once. Read the game's animation header to fill it in. With
    `bones: Some(n)` only the model's first n bones are animated (its animated parts), so bones
    after them (billboards) keep their rest pose.

### 3D areas (N64)

A 3D level is a Model output of kind Area, plus its collision. Build the model from the level's
display lists (the geo layout lists them; one bone per blend: Opaque, Cutout for alpha layers,
Transparent for decals), read the collision with CollisionTriangles, and give the model output a
`terrain`, which says what the collision's codes mean (like a Surfaces grid: every code needs
a meaning, or `other`):

Model(id: "grounds", from: "grounds", kind: Some(Area), meters_per_unit: 0.01,
      terrain: Some((from: "ground",
                     codes: { 0x0000: [Walkable], 0x000A: [FallThrough], 0x0028: [Solid] },
                     other: None, extensions: [],
                     floor_normal_y: 0.01,
                     spawn: Some((position: (100, 0, -250), yaw: 180.0)))))
    Play mode walks a character on the terrain's floors (triangles facing up; floors up to a
    meter above the feet count, so a falling body lands), is stopped by its walls (nearly
    vertical triangles, and anything Solid) and falls through FallThrough floors. Ceilings are
    ignored. `floor_normal_y` is the largest |normal y| of a wall (the game's collision loader).
    `spawn` (model units, degrees, 0 faces +z) is where a character that is not already standing
    on the area starts (often the level script's start position). `meters_per_unit` must be the
    scale the game draws the level at (1 unit ~ 1 cm is 0.01), the same as its characters'.
    run_recipe's summary lists the terrain's triangles by facing and by code: check them against
    the model (a code you did not expect is a hint to read the game's surface types).

### 2D sprites

Console sprites are not images: a frame is a handful of hardware objects (OAM entries, small
rectangles of tiles) placed around the character's origin, and animations are lists of timed
frame numbers. Rendering tiles with GbaMetatiles can't compose them (objects sit at arbitrary
pixel offsets); use these steps, which build a sprite: frames (images with an origin) and
animations.

GbaObjectFrames(tiles: "gfx", bank: "obj", objects: "lists", layout: Oam, frames: (
    lists: (buffer: "table", offset: 0, stride: 4, count: 120, field: (at: 0, size: U32)),
    list_base: 0x1A2B00, count_size: U8,
    tile_base: Some((buffer: "frames", offset: 0, stride: 4, count: 120, field: (at: 2, size: U16))),
), into: "hero")
    One frame per table entry. `lists` gives each frame's object list: an offset into the
    `objects` buffer (plus list_base), where a count (count_size) and that many objects follow.
    `tile_base` (optional) gives each frame's first tile, counted in tiles of the `tiles`
    buffer; objects' tile numbers are relative to it (games that stream a frame's tiles into
    VRAM do this; check how the game picks the tiles). `bank` is a palette bank loaded with
    GbaPalettes; an object's palette number picks the slot. Objects are layered like the
    hardware: the first on top. The layout is `Oam` (the hardware's six bytes) or your game's
    packed record, described field by field:
    layout: Fields((stride: 5, x: (at: 0, signed: true), y: (at: 1, signed: true),
                    shape: (at: 2, shift: 6, bits: 2), size: (at: 2, shift: 4, bits: 2),
                    hflip: Some((at: 2, shift: 2, bits: 1)), tile: (at: 3, size: U16, bits: 10),
                    palette: Some((at: 3, size: U16, shift: 12, bits: 4))))
    A field is `size` bytes (U8, U16, U32, little-endian) at byte `at`, shifted right by `shift`,
    keeping `bits` bits (0: all), two's complement with `signed`. Shape 0 is square, 1 wide,
    2 tall; size 0..3 grows (8x8 .. 64x64 for squares).
    Games whose frame records hold (object count, first object, first palette) instead of
    counted lists: `counts: Some(<table>)` gives each frame's object count (lists are then
    only objects), `list_unit: 8` turns object numbers into byte offsets (the object size),
    and `palettes: Some((buffer: "pals", first: <table>))` gives each frame its own palettes:
    an object's palette p is palette (first + p) of the buffer, as games that load a frame's
    palettes when they draw it do (more palettes than the 16 slots of a bank).
KeyframeAnimation(from: "rom_low", name: "walk_south", at: Pointer(table: 0x1000, index: 6, base: 0x08000000),
    layout: (stride: 4, frame: (at: 0), ticks: (at: 1), end: (at: 3, shift: 7, bits: 1),
             after_end: RewindByte), fps: 59.7275, into: "hero")
    Adds an animation to the sprite from keyframes in a buffer: records of `stride` bytes with a
    frame number, a duration in ticks (`fps` ticks per second: the game's update rate), optional
    flip_x / flip_y flags, and an end flag on the last record. `at` is Offset(0x4AEC) or a
    pointer table (little-endian u32, minus `base`). after_end: Stop (hold the last keyframe),
    Restart, or RewindByte (the byte after the last record says how many keyframes to go back;
    0 stops). Read the buffer first, e.g. the ROM region holding the tables.
    Games that keep each animation's length in a directory instead of an end flag:
    at: Directory(table: 0x0, index: 37, stride: 4, count: (at: 0, size: U16),
                  first: (at: 2, size: U16), keys: 0x7D60)
    record `index` of the directory gives the keyframe count and the first keyframe's number
    (keyframes numbered from offset `keys`); the count ends the list and `end` is not read.

### Where things are (placements)

Games keep where they put things in tables of records: room headers, exit lists, entity
lists. Read them with Places instead of writing positions into the recipe (positions are game
data, like graphics):

Places(from: "rom", at: 0x1234, stride: 16, until: Some((field: (at: 0), values: [255])),
       when: [(field: (at: 0, bits: 4), values: [3]), (field: (at: 2), values: [0])],
       x: (at: 8, size: U16), y: (at: 10, size: U16), origin: (2048, 1024),
       item: Some((outputs: {}, other: Some("enemy_slime"))),
       name: (text: "Slime"), group: Some((text: "Forest")),
       code: Some((at: 2)), into: "forest_enemies")
    Adds a placement per record to list `into`: `count` records of `stride` bytes from byte
    `at` of buffer `from`, or up to the first matching `until`; only records matching every
    `when` (a field and the values it may hold). Positions are pixels, plus `origin` (where the
    records' space starts: a room's place on the world map). With `size: Some((w, h))` (two
    fields) the position is a top left corner and the placement goes to the centre.
    `item: Some((by: Some(field), outputs: {value: "output"}, other))` picks the output each
    shows by a field (by the record's number without `by`). `name`, `group` and a door's `to`
    are templates: (text, fields), `{}` taking the next field's value, `{item}` the output,
    `{index}` the record number. `door: Some((to: (text: "arrive {} {}", fields: [...]),
    radius: 0.75))` makes walking into it take the player to the node of that name (place the
    arrival points with another Places step whose names match). `extent: Some((by: field,
    sizes: {value: (half_w, half_h)}))` says how far it reaches. `animation: Some((text:
    "frame_{}", fields: [(at: 3)]))` names the animation each placement plays: objects that
    show one frame chosen by their record (give the sprite a one-keyframe animation per frame,
    from a Literal of keyframe records) or that start in another state. `behavior:
    Some((by: Some((at: 1)), outputs: {3: "octorok", 7: "moblin"}))` picks, like `item`, the
    `Behavior` output each placement does when played: an enemy's `Actor` by its kind field.
    `behaviors: [(outputs: {}, other: Some("solid")), (by: ..., outputs: {...})]` adds more,
    each picked the same way (a person who talks and blocks the way, a chest that is solid
    and opens); a pick that finds nothing for a record adds nothing.
MarkPlaces(from: "forest_enemies", width: 64, height: 40, cell_size: 16, origin: (2048, 1024), into: "cells")
    Marks the cells each placement covers with its `code` (a field of its record, or its
    number from 1): a grid for a Surfaces output, so the map shows where enemies, doors and
    objects are. `origin` is the grid's top left corner in the placements' space.

## Outputs

Image(id: "area", from: "img", kind: Some(Area))
    kind sorts the image in the Editor's content browser: Area, Tileset, Sprite, Character,
    Ability, Object or Audio. Always set it; images without a kind are listed under Other.
    An Area is a playable map; a Surfaces output covering it gives it collision. An area can
    have background layers and animated tiles: see "Background layers and animated tiles".
Surfaces(
    id: "surfaces", width: 63, cell_size: 16,
    grids: [
        (name: "collision", from: "cell_collision", element: U8,
         codes: { 0x00: [Walkable], 0x0F: [Solid] },
         other: Some([Extension("game::Special")])),
    ],
    extensions: [(id: "game::Special", fallback: [Walkable])],
    image: Some("area"),
)
    One code per cell per grid; a cell's properties are the union over grids. `image` names
    the image it covers; without it, an image of the same pixel size is taken (ambiguous when a
    plugin has several such images, as rooms of a game often are). Every code needs
    a meaning: list it in `codes`, or give `other` (an explicit catch-all; Some([]) means "no
    surface meaning"). Unlisted codes without `other` are an error, on purpose.
Scene(id: "world", from: ["rooms", "forest_enemies", "forest_doors"], pixels_per_meter: 16.0)
    The placement lists (Places steps) as one scene: a node per placement showing its item (an
    image, model or sprite of this plugin), under its group node, with its door. The user places
    it in the Workshop like any item and gets the whole group: rooms where the game's maps put
    them, with their enemies, objects and doors. run_recipe previews it as a map.

Shared surface properties: Solid, Walkable, FallThrough, Swimmable, Shallow,
Damaging(Fire|Spikes|Electric|Generic), Slippery(0.0-1.0), Climbable, Friction(f32),
Ledge(North|South|East|West), SolidParts(mask: bit0 TL, bit1 TR, bit2 BL, bit3 BR),
Extension("namespace::Name") — declared in `extensions` with a fallback of shared properties.
Only use a shared property when you have evidence; otherwise use an extension and say why in a
comment. Name extensions after the game, with a short id of it (game::Special).

Model(id: "hero", from: "model", kind: Some(Character), meters_per_unit: 0.0025)
    A model with its animations. meters_per_unit scales model units to meters (include any
    scale the game applies to the whole model).
Model(..., character: Some((
    movement: (walk_speed: 4.0, run_speed: 8.0, acceleration: 10.0, acceleration_falloff: 0.5,
               braking: 10.0, turn_speed: 360.0,
               skid: Some((min_speed: 4.0, reverse_angle: 100.0, braking: 20.0, turn_kick_speed: 2.0)),
               jumps: [(speed: 12.0, speed_from_ground: 0.25, keep_ground_speed: 0.8,
                        animation: "jump", land_animation: Some("jump_land")), ...],
               chain_window: 0.3, landing_time: 0.1,
               jump_release: Some((above: 6.0, factor: 0.25, rate: 30.0)),
               gravity: 30.0, max_fall_speed: 20.0,
               air: (turn: false, drag: 3.0, acceleration: 12.0, strafe_speed: 3.0, max_speed: 8.0),
               radius: 0.4),
    animations: (idle: ["idle"], walk: "walk", run: "run", run_above: 6.0, walk_below: 5.0,
                 stride_speed: 1.2, fall: "fall", skid_turn: Some(..), turn: Some(..),
                 turn_faces_back: true, skid_stop: Some(..), skid_stopped: Some(..)),
    lean: Some((bone: "torso", roll_axis: (0, 1, 0), pitch_axis: (0, 0, 1), roll_per_turn: -0.01,
                pitch_per_speed: 3.0, max: 30.0, rate: 180.0)),
    camera: Some((distance: 8.0, pitch: 15.0, height: 1.0, follow: 2.0)),
)))
    Makes the model playable in the editor's Play mode with Clockwork's generic controller:
    acceleration curves, skids (turnarounds and stops), chains of jumps (jump i+1 if jumping
    again within chain_window of landing and fast enough), shorter jumps when the button is
    let go, air control, a leaning bone and a follow camera. Meters, seconds, degrees. Take every
    number and choice from the game's own movement code (decomp constants: a per-frame speed
    × fps, a per-frame change × fps²) and say where each comes from in comments, so it feels
    like the original. Animations that play once must have loop_start None. The docs of
    clockwork_core::character explain each field.

Sprite(id: "hero", from: "hero", kind: Some(Character), pixels_per_meter: 16.0)
    A sprite with its animations. pixels_per_meter says how big a pixel is in the world (GBA
    content is placed at 16 px = 1 m, so 16.0 there; use the same for everything from one game).
Sprite(..., character: Some((
    movement: (speed: 5.0, acceleration: None, radius: 0.4),
    animations: (
        idle: (north: (name: "idle_north"), east: (name: "idle_east"), south: (name: "idle_south"),
               west: (name: "idle_east", flip_x: true)),
        walk: (north: (name: "walk_north"), east: (name: "walk_east"), south: (name: "walk_south"),
               west: (name: "walk_east", flip_x: true)),
    ),
    camera: Some((distance: 9.0, pitch: 90.0)),
)))
    Makes the sprite playable from above in the editor's Play mode: it walks on the ground of
    surface maps (Solid cells block it), faces the stronger axis of the stick (north is up) and
    plays the idle or walk animation of that facing. Meters, seconds, degrees: take the speed
    from the game's movement code (pixels per frame × frame rate ÷ pixels_per_meter) and cite
    it in a comment. Every animation named must exist on the sprite. flip_x mirrors an
    animation, for games that draw one side view. clockwork_core::character2d has the docs.
    More than walking: `actions` (see "Actions and items" below).

### Actions and items

A 2D character can have `actions: [...]`, `projectiles: [...]` and `dodge: Some("id")` (the
action of the dodge button: a roll, dash or sidestep) next to
`movement`, `animations` and `camera`, and the plugin can output `Items`:

(id: "slash", animation: <per facing>, duration: 0.25, motion: Rooted | Walk(0.7) | Slide([(0.0, 7.5), (0.3, 0.0)]),
 hits: [(from: 0.03, until: 0.18, area: Front(near: 0.2, far: 1.5, half_width: 0.65) | Around(radius: 1.6), damage: 1.0, knockback: 0.6)],
 invulnerable: Some((0.3, 0.4)), spawns: [(at: 0.1, projectile: "arrow", ahead: 0.6)],
 hold: Some((animation: <pose>, walk_animation: Some(<walking pose>), walk_factor: 0.7, guard: Some(70.0),
             release: Some((action: "spin", min_time: 1.0)))), cancel_from: Some(0.35))
    One action. All times are seconds from the action's start; `duration` is how long it lasts
    (an animation often ends in a long final hold: read the keyframe ticks). `hold`: after the
    action, while the button that started it stays down, the character keeps the pose (walking
    slower, optionally guarding the front), and letting go performs `release.action` if it was
    held `min_time`. Hit areas are relative to the feet, along the facing.
 attachments: [(sprite: "sword", frames: FollowFrame(offset: -100, from: Some(100), until: Some(160))),
               (sprite: "shield", frames: Animation(<per facing>), behind: true)]
    What the character holds during the action and its hold: another Sprite output of the
    plugin, drawn at the character's origin. Games often draw held items as a second sprite in
    step with the character: FollowFrame shows the attachment's frame (character frame +
    `offset`), mirrored with the character, only for character frames in from..until (count
    merged sprites: frames of a merged sheet come after the earlier ones). Animation plays one of
    the attachment's own animations per facing. run_recipe shows a playable sprite's actions
    with their attachments, so wrong offsets show at once.
(id: "bomb", motion: Stationary | Straight(speed: 11.2) | Return(speed: 7.5, range: 3.75), lifetime: 2.5,
 radius: 0.35, damage: 0.0, burst: Some((radius: 1.6, damage: 2.0, duration: 0.4, knockback: 1.2)),
 stops_at_walls: true, limit: Some(1), color: (0.1, 0.1, 0.2),
 sprite: Some((sprite: "arrow", height: 0.5, burst: Some("explode"),
               animation: (north: (name: "fly_up"), east: (name: "fly_side", flip_x: true),
                           south: (name: "fly_up", flip_y: true), west: (name: "fly_side")))))
    A projectile. `limit` caps how many exist at once. `sprite` draws it with a Sprite output:
    the animation of the direction it was thrown in (flip_x / flip_y mirror it), `burst` while
    it explodes, `height` meters above the ground. Without a sprite it is a colored shape.
    Projectiles often have no frame table: their tiles stay in video memory at a fixed place.
    Assemble that memory with Read/Place and give GbaObjectFrames `first_tile: 64` (the
    entity's tile offset) instead of `tile_base`.
Items(id: "items", items: [(id: "bow", name: "Bow", description: "...", category: Weapon,
    action: Some("draw_bow"), ammo: Some((item: "arrows", cost: 1)), max: 1, start: 1, slot: Some(A),
    icon: Some(Frame(sprite: "bow", frame: 10)))])
    `icon` is the item's picture in the inventory and on its button: Frame(sprite, frame,
    flip_x), Animation(sprite, animation) (its first picture) or Image("output").
Ability(id: "sword", name: "Sword", description: "...", from: "hero", actions: ["slash"], items: ["sword"], dodge: false)
    Packages some of a character's moves so other characters (any game, 2D or 3D) can learn
    them in a composition plugin: the actions (plus what they release into), the projectiles
    they throw, the items that start them (plus their ammunition), and with `dodge` the dodge.
    Their poses become roles named after the action (`slash`, `slash_hold`,
    `slash_hold_walk`, and the dodge's action) that the learner fills with its own animations. Give
    attachments a `still: Some(frame)`: the frame a 3D character holds at its hand, since it
    can't follow the character frame by frame. Make one ability per item or move.
    3D characters (Model outputs) can have `actions`, `projectiles`, `dodge` and `hand: Some("bone")`
    in their `character` too: an action's animation names a model animation (the same for
    every facing).
    The items of the game: which action each starts, its ammunition, how many fit (max), how many
    the character starts with and the button it starts on (A or B). Items without an action can
    be owned but not used (ammunition, keys). The inventory (Play mode: I) lists them and puts
    them on the buttons; J and K press B and A, L dodges, H shows hit boxes.
    Every animation an action names must exist, every release and `dodge` must name an action,
    and every spawn a projectile; attachments, projectile sprites and icons must name outputs
    and animations that exist. Otherwise the plugin doesn't extract.
    In 3D worlds (areas with a terrain, see "3D areas") the same character plays as a 2.5D
    billboard: upright, facing the camera, walking camera-relative on the terrain's floors and
    falling off ledges (`movement.gravity`, default 9.8, is only used there). Its facing
    animation follows the stick as seen on screen. `world_camera: Some((distance: 8.0,
    pitch: 30.0))` sets that view (these are the defaults). Nothing else is needed.

Cameras are assets too. A `camera` or `world_camera` can name a camera asset instead of
writing one in place: `camera: Some("builtin/top-down")` (built-ins: `builtin/top-down`,
`builtin/billboard`, `builtin/follow`), or one of a plugin's own outputs,
`Camera(id: "low", camera: TopDown((distance: 8.0, pitch: 25.0)))` (or `Follow((...))` for 3D
characters) named as `"plugin/low"`. Without one, top-down characters get `builtin/top-down`,
billboards `builtin/billboard`, and 3D characters leave the camera where it is.

A character's screen and buttons are assets as well, named in its `kit` (each optional, the
built-in one otherwise): `kit: (hud: Some("plugin/hud-id"), ui: Some("plugin/ui-id"),
menus: Some(["plugin/bag", "builtin/pause"]), input: Some("plugin/keys"))`. The outputs:
`Hud(id, hud: (widgets: [(kind: ItemSlot(B), anchor: BottomLeft, offset: (8, 8)), ...]))` with
widget kinds `ItemSlot(A|B)`, `Button(label, button)`, `Action`, `Health(per_heart)`,
`Counter(item)`, `Lives`, `Timer` and `Label(text)`, anchored at a corner, edge or `Center`,
offsets in game pixels (the screen is 240 high). A counter, `Lives` or `Timer` can have `icon:
Some("plugin/image")` before it and `format: Some("x{n}")` (a timer `{m}`, `{ss}`). The HUD's
`words: (got: "Got {item} x{n}", resume: "Resume", ...)` are the words Play itself shows:
`resume`, `start_over`, `stop_playing`, `nothing_carried`, `on_a`, `on_b`, `close`, `choose`,
`back`, `next`, `got`, `destroyed`, `checkpoint`, `down`, `game_over`, `win`, `file`
(`{letter}`) and `new_file`; give the ones your font can't write in its letters, or `""` to
leave one out. `Ui(id, ui: (panel, border, text, dim, accent, shade: (r, g,
b, a), frame: Some((image: "plugin/image", border: 3)), font: Some("plugin/font"),
font_size, dialog_font: Some("plugin/dialog-font"), dialog_font_size: Some(8.0)))`;
`dialog_font` and `dialog_font_size` are optional and inherit `font` and `font_size` when
omitted. Dialogs wrap to the box width and shrink text only when needed to fit the screen.
Missing bitmap glyphs try the other letter case, then use the default font at the same height.
`Menu(id, menu: (kind: Inventory(columns: 6) | Pause, title, opens_with:
"Menu"))`; `InputMap(id, input: (bindings: [(button: "A", keys: ["KeyK"], pad: ["North"],
labels: Some(("K", "Y")))]))` for the buttons Up, Down, Left, Right, A, B, Dodge, Jump, Walk,
Menu, Pause and Back. Built-ins: `builtin/hud`, `builtin/ui`, `builtin/inventory`,
`builtin/pause`, `builtin/input`.

A character's parts can be assets too, shared between characters: `movement` and
`animations` may name a `Movement(id, movement: TopDown((speed, radius, ...)) |
Platformer((walk_speed, ...)))` or `Animations(id, animations: Directional((idle, walk)) |
States((...)))` output instead of being written in place (the kind must suit the character:
top-down for sprites, platformer for models). `stats: Some("plugin/stats-id")` names a
`Stats(id, stats: (health: 6.0, hurt_time: 1.0))` (built-in `builtin/stats`), and
`inventory: Some("plugin/items-id")` the `Items` output it carries (otherwise its plugin's
first). Names lead to other plugins' outputs as well.

Behaviors that scene nodes use can be outputs too, `Behavior(id, behavior: Target(health,
radius, solid, drop) | Pickup(item, count, radius) | Trigger(radius, effect, once) |
Checkpoint(radius) | Solid(radius) | Hazard(radius, damage) | Door(radius, to: "node name",
fade))`, with trigger effects `Message`, `Teleport`, `Give`, `Restart`, `Hurt(amount)`,
`Heal(amount)`, `Set("flag")`, `Clear("flag")` and `All([...])` (built-ins: `builtin/target`,
`builtin/checkpoint`, `builtin/post`, `builtin/spikes`, `builtin/heart`).

Enemies, NPCs and moving things are `Actor((...))` behaviors: a state machine you translate
from the game's own code (its state functions, timers and distance checks), with the game's
numbers converted to meters and seconds:

Behavior(id: "octorok", behavior: Actor((
    health: Some(1.0), radius: 0.4, solid: true, contact: 0.5, speed: 1.0, knockback: 0.8,
    drop: Some(("rupee", 1)),
    states: [
        (name: "walk", motion: Wander(radius: 3.0, pause: 0.5), animation: Some(Facing((
            north: (name: "walk_up"), east: (name: "walk_side", flip_x: true),
            south: (name: "walk_down"), west: (name: "walk_side")))),
         next: [(to: "aim", when: All([Near(5.0), Chance(0.5)]))]),
        (name: "aim", motion: Still, enter: Some(Sound("tmc-sounds/spit")),
         next: [(to: "walk", when: After(0.8)), (to: "hurt", when: Hurt)]),
        (name: "hurt", contact: Some(0.0), invulnerable: true,
         next: [(to: "walk", when: After(0.4))]),
    ],
)))
    It starts in its first state. Motions: `Still`, `Wander(radius, pause)` (random points near
    where it was placed), `Chase`, `Flee`, `Charge` (straight at where the player was when the
    state began, until blocked), `Home`, `Patrol(points: [(x, z), ...], looped)` (meters from
    where it was placed). A state's `speed` multiplies the actor's; `contact` overrides its
    contact damage (0 while stunned); `invulnerable` ignores hits; `enter` is a trigger effect
    on entering; `animation` is `One("name")` or `Facing(...)` per direction it moves. The first
    transition whose cue holds wins, checked every 1/60 s: `Near(m)`, `Far(m)`, `After(s)`,
    `Hurt`, `HealthBelow(h)`, `Arrived` (a wander point, home or a patrol's end), `Blocked` (a
    wall or hole stopped it), `Chance(per_second)`, `When(condition)`, `Not`, `All`, `Any`.
    Actors without `health` can't be beaten; beaten actors count for `DestroyAll`. Transitions
    to states the actor doesn't have are reported when the plugin is saved.

What happens over time rather than at once (a cutscene, a boss's entrance, an NPC walking up to
talk, a door that opens a moment after a lever) is a script, started by the effect `Run`:
`Trigger(radius: 1.0, once: true, effect: Run((hold_player: true, steps: [Walk(node: "Guard",
to: "player", speed: 2.0), Do(Say((speaker: Some("Guard"), pages: ["Halt!"]))),
Wait(0.5), Until(Flag("lever")), Do(Set("gate_open"))])))`. Commands: `Do(effect)` (any
effect, at once; the game waits while a dialog is open), `Wait(seconds)`, `Until(condition)`,
`Walk(node, to, speed)` (in a straight line to where node `to` stands, `"player"` for the
player either way; its behaviors go with it), `Place(node, to)` (at once), `Hold(bool)` (the
player can't move or act), `If(when, then: [...], otherwise: [...])`, `Repeat(times:
Some(n), steps: [...])` (forever without `times`; it must wait somewhere) and `End`.
`hold_player` holds the player until the script ends. Translate a game's event scripts (its
cutscene commands, its timers) into these, not into new code. A scene node has a
list of behaviors, each working only while its condition holds, and can appear only once one
does: conditions are `Has(item, count)`, `Flag("name")`, `Gone("node name")` (destroyed or
picked up), `Act(n)` (the current act is n), `Not(...)`, `All([...])` and `Any([...])`.
A node's `acts: Some([1, 3])` makes it present, with working behaviors, only in those acts
(numbered from 1); omit `acts` or use `None` for every act. With no act chosen, all act
restrictions are open, but `Act(n)` is false. Changing acts keeps destroyed or collected
nodes gone and remembers which `appears` conditions have held. The Inspector's "In acts"
row edits this list; remote control uses `Acts("node", Some([1, 3]))` and, during Play,
`Act(Some(1))` (or `Act(None)` to clear the choice).
A game's rules: `Rules(id, rules: (when_down: StartOver |
LastCheckpoint | StopPlaying, lives: Some(3), time_limit: Some(120.0), goal: Some(Collect(item:
"rupee", count: 10) | Reach("node") | DestroyAll | When(condition))))`. A `Game(id, game: (rules, flow, camera, hud,
ui, input, menus, stats, inventory))` output bundles the assets of a whole game, each
`Some("plugin/output")`; a scene names it as its preset.

A `Flow` asset describes the screens around the world; a game chooses it with
`flow: Some("plugin/sm64")`, with the scene's choice taking priority over its preset:
`Flow(id: "sm64", flow: (title: Some((logo: Some("plugin/output"), prompt: "plugin/dialog")),
files: Some((title: (pages: ["SELECT FILE"]), slots: 4, counts: Some("star"))),
acts: Some((courses: [(name: (pages: ["COURSE"]), entry: "node name", acts: [(pages: ["ACT 1"])])])),
game_over: Some((text: (pages: ["GAME OVER"])))))`. Every label can name a dialog asset or
be written in place, e.g. `prompt: (pages: ["PRESS START"])`; each screen is optional. A model
logo can be framed the way the game's own camera framed it, filling the screen:
`logo_camera: Some((from: (0.0, 0.0, 3200.0), fov: 45.0))` (model units, vertical degrees).
`start: Some("node name")` is where the game begins once a file is chosen. A game with a flow
is played in the whole window (F11 switches, Esc returns to the editor).

Presentation from the ROM: `Font(id, from: "image", glyph: (8, 8), chars: "ABC...", advance:
Some([...]))` cuts a bitmap font from an image a step makes (cells in a grid, one per character
in reading order); a UI theme writes with it (`font: Some("plugin/font")`) and can frame its
panels with an `Image` (`frame: Some((image: "plugin/image", border: 3))`, nine-slice).
`Sound(id, from: "buffer", rate: 13379, format: S8 | U8 | S16)` reads samples (GBA games often
keep plain signed 8-bit PCM); triggers play them with the effect `Sound("plugin/sound")`, and a
character's kit can name `sounds: Some("plugin/sounds")`, a `Sounds(id, sounds: (hurt, pickup,
door, menu))` output. Music: most GBA games play theirs with the m4a engine ("Sappy");
`find_songs` lists a ROM's song table, and `Music(id, song: 0x...)` names a song by its
header's offset (rendered with its instruments and looped where the song loops; `seconds:
Some(90.0)` cuts a song that never ends). A game's settings play one from the start (`music:
Some("plugin/music")`), and the effect `Music("plugin/music")` changes it (`Music("")` stops
it): put it in a trigger at an area's door, or a boss actor's state. `Dialog(id, dialog: (speaker: Some("Sign"), pages: ["...", "..."]))` is
shown by the trigger effect `Say("plugin/dialog")` (or `Say((speaker, pages))` in place).
A game's own text never goes in a recipe: decode it with a `Text` step and give the dialog
`from: Some("text"), lines_per_page: Some(4)` instead of pages.

In the Workshop the user can edit any plugin asset's fields from the Inspector (the change is
written into that plugin's recipe.ron, value by value), and copy settings assets, built-ins
too, into their own plugin `my-game` to change them there.

## Looking at results (run_recipe)

- Plain call: a summary (every surface code per grid, in hex, with cell counts) and previews of
  up to 4 images. Transparent pixels show as a grey checkerboard. A sprite is shown as a contact
  sheet with one row per animation (feet lined up by origin, mirroring applied): garbage frames
  mean a wrong tile window or object layout, missing pieces a wrong list. Models are drawn from the
  front-left, posed by their first animation (a stick figure or exploded parts means wrong bone
  offsets or channel order; spiky garbage means a wrong display list or segment).
- surfaces: {} draws the surface overlay on the image it covers.
- surfaces: {grid: "collision", codes: ["0x10", "0x20"]} highlights those cells in magenta: the
  quickest way to learn what a code means.
- zoom: {image: "area", x: 400, y: 300, w: 220, h: 160} shows that rectangle enlarged; combine
  it with surfaces to inspect cells closely.

## Play-testing a scene (play_test)

play_test plays a Scene output the way the Workshop's Play mode would, without a window, at
Play's fixed 1/60 s step: the same script always plays the same way. The scene's images with
surface maps are its floors (16 px a meter, centered on their node) and its models with
collision its 3D areas; doors and behavior assets of placements work as in Play.

- `plugin` (saved) or `recipe` (+ `game`, or `plugin` for an edit not saved yet), `scene`
  (default the first), `character` ("plugin/output" of a sprite or model with `character`:
  another plugin's Link works in this plugin's rooms), `start` (a node's name or [x, y, z]),
  `rules` ("plugin/output" or RON, e.g. `(lives: Some(3), goal: Some(Reach("Exit")))`), `act`.
- `nodes`: behaviors added for this play, to scene nodes by name or to new nodes with `at`:
  `{"name": "Chest", "behaviors": ["Pickup(item: \"key\", count: 1, radius: 0.5)"]}` or
  `{"name": "Goal", "at": [12, 0, 4], "behaviors": []}` (a point to reach).
- `script`: steps, each `{"hold": [dx, dz], "seconds": 2}` ([1, 0] east, [0, 1] south, down the
  map), `{"toward": "Octorok"}` (straight at a node, an actor where it is now, until reached or
  `seconds`, default 10), `{"seconds": 1}` (stand still), with `"walk": true` and `"press":
  ["a"]` (pressed as the step starts and held through it; "jump", "a", "b", "dodge"; a 0.1 s
  step is a tap). At most 600 s in all.

The report lists events with their time and step (items got, nodes gone, "hurt by
\"Octorok\"", doors and where they led, actors changing state, sounds, dialogs, problems),
how each step ended (reached, how close it came, "stuck at" a wall for half a second), the end
(position, health, lives, items carried, won or game over: play stops there) and the nodes
never reached (pickups, doors, triggers, checkpoints) and still standing (targets, actors that
can be beaten), with how close the player came. Content problems come first: names that lead
nowhere, a start that isn't on a floor, behaviors that don't resolve. A toward step that gets
stuck usually means a wall in the surface map or a node placed off its room.

## Graphics that change at runtime

Some areas have more graphics than fit in VRAM and swap tiles while you play: by camera
position (a large town map), by animation (water, flowers) or by story state. Signs:
magenta or garbage tiles in only part of a render, or wrong art in one region. Search the decomp
for managers that copy graphics (e.g. "TileSetManager", "LoadGfx", region checks). To show the
area as players see it, build one VRAM per state (Place), render each, and draw each state only
in its region with Overlay(region, replace: true). Tiles swapped by animation are `frames`
(next section), not a still image.

## Background layers and animated tiles

GBA areas are drawn as up to four backgrounds (BG0-BG3): the map, a far layer that scrolls
slower (sky, clouds, a distant forest) and parts drawn over the characters (treetops, roofs,
bridges). Render each background with GbaMetatileMap into its own image and list them on the
area's Image output:

Image(id: "forest", from: "bg2", kind: Some(Area),
      frames: Some((images: ["bg2", "bg2_water_1", "bg2_water_2"], ticks: [12], fps: 59.7275)),
      layers: [
          (from: "bg3", order: -1, scroll: (0.25, 0.25), wrap: true),
          (from: "bg1", order: 1, over_sprites: true),
      ])
    `from` is the area itself (the layer its Surfaces cover, order 0). Each layer: `order`
    (higher draws over lower; equal orders draw after the area, as listed), `scroll` (how far it
    moves per meter the camera moves, across and down: 1.0 like the map, 0.25 far away, 0.0
    fixed to the screen; read it from the code that sets the background's scroll registers: a
    background scrolled by the camera position shifted right by 2 has 0.25), `wrap` (repeats the image across the area, like a hardware
    background that is smaller than the map) and `over_sprites` (drawn over characters: the
    hardware priority of that background is above the sprites'). Layers line up with the area
    when the camera is at its top left corner. Play mode scrolls them; the Workshop shows them
    unscrolled. Transparent pixels show what is behind.
    `frames` (on the area or on any layer) are images shown in turn in place of the still one,
    each for its `ticks` (one number per image, or one for all) at `fps` ticks a second, looping:
    water, lava, flowers. Games animate tiles by copying new tile graphics into VRAM every few
    frames from a list of sources (search the decomp for the tile animation table or the code
    that copies to VRAM on a timer). Build one VRAM per frame (Place each frame's tiles where the
    game copies them), render the same map with each, and list the renders. Every frame must be
    the size of the image it replaces. run_recipe lists the layers back to front and previews
    the area with all of them.

## Habits that work

- Verify meanings visually: highlight a code's cells, then look at what is there.
- Color index 0 is transparent. Black or garbage tiles usually mean a missing palette or a wrong
  char_base. Shuffled but correctly colored tiles mean the wrong tile base.
- Games often load common palettes/graphics at startup before area data; look for that code.
- Pointers in GBA ROMs are 0x08xxxxxx; ROM offset = pointer - 0x08000000. search_rom for a
  pointer's 4 bytes (little-endian) finds tables that reference it.
"#;

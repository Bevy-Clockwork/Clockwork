# Clockwork — Design Document

> Status: draft · Last updated: 2026-10-21

Clockwork is a shared layer between existing games and the [Bevy](https://bevyengine.org) game engine.
Users import games they own (ROMs, executables) into a **Library**. They pick a game and tell an **AI
agent** what to extract. The agent produces a shareable **plugin**. A Godot-style **Editor** then
combines plugins from different games into new projects.

**Example (The Minish Cap):**
- "Extract Link as a playable character"
- "Extract the entire overworld"
- "Extract the Cane of Pacci mechanic"

---

## 1. Goals and non-goals

### Goals
- **Modding first.** Lift characters, mechanics, areas and assets out of games and combine them.
- **Combine whole games eventually.** The architecture must scale from one ability up to entire games.
- **Modular everywhere.** New platforms, games and systems plug in without changes to the core.
- **Plugins carry no game data.** A plugin is a *recipe* that turns the user's own copy of the game
  into usable content. It never contains the original assets or code.
- **Easy sharing.** Plugins are small, readable files that work across machines and Clockwork versions.

### Non-goals (for now)
- **Frame-accurate behavior.** "Playable" means *it feels like the original*, not a byte-exact port.
  Trace comparison against the original is an optional check, not a requirement.
- **Running original executables or emulating consoles.** Import exposes game bytes to recipes;
  platform recognition alone does not provide a game-specific extraction or runtime.
- **Distributing game content.** Clockwork never downloads or bundles ROMs or extracted assets.

---

## 2. Initial scope

| Platform | Why |
|---|---|
| **GBA** | 2D, well documented, strong decomp scene (e.g. `zeldaret/tmc` for The Minish Cap). |
| **N64** | 3D, strong decomp scene (OoT, MM, SM64). Forces us to solve 2D↔3D early. |

The first game is **The Minish Cap**. The second game is **Super Mario 64** (N64, complete decomp:
`n64decomp/sm64`). The unified layer only proves itself once two games meet.

Library imports also recognize SNES, Nintendo DS, PS1, PS2 and PC executables. See the README
for accepted file formats and current limits. GBA and N64 remain the initial extraction targets;
new platform recognition gives the agent validated bytes and identity, not universal decoders.

SM64 was chosen over Ocarina of Time on purpose. It differs from Minish Cap in **both** dimension
(2D → 3D) **and** genre (top-down action-adventure → physics-heavy platformer). That pushes the shared
vocabulary harder from the start. The cost: when something doesn't fit, it's harder to tell whether
the dimension or the genre caused it. To manage this, SM64 work is staged (assets → area → character),
and every vocabulary gap is recorded with its cause (see §4.2).

---

## 3. Extraction tiers

| Tier | Examples | Difficulty | Approach |
|---|---|---|---|
| **1. Assets** | Sprites, palettes, tilesets, music, text | Low | Deterministic decoders in platform modules |
| **2. Structure** | Maps, rooms, entity placement, warps, collision | Medium | Decoders plus meaning from the knowledge pack |
| **3. Behavior** | Characters, items, mechanics | High | The agent analyzes the code and maps it onto generic systems |

For Tier 3, the agent doesn't port code line by line. It reads the original (ideally a decompilation)
the way a designer would, and pulls out:
- **Parameters:** speeds, acceleration, timings, invulnerability windows, jump arcs.
- **State machines:** states, transitions, triggers.
- **Asset bindings:** which animations and sounds belong to which states.

These map onto Clockwork's **generic runtime systems**. Custom code is the exception, used only for
truly unusual mechanics.

---

## 4. Architecture

```
clockwork/
├─ core/            Shared vocabulary: components, interfaces, units, plugin manifest
├─ runtime/         Generic Bevy systems (character controller, abilities, tiles, world graph)
├─ platforms/
│  ├─ gba/          ROM loading, memory map, 4bpp/8bpp tiles, OAM, m4a audio
│  └─ n64/          ROM byte order, display lists (F3DEX*), textures, audio
├─ agent/           Agent tools (MCP server) and the extraction loop
├─ library/         ROM import (also from .zip), hashing, game names and box art
├─ recipe/          Loads plugin folders and runs their recipes
└─ editor/          Godot-style composer
```

### 4.1 Layers

- **Platform modules** describe *how a console works*: data formats, memory layout, graphics and
  audio decoding. Adding a platform (SNES, NDS…) means adding one module.
- **Knowledge packs** describe *how a specific game works*: where data lives in the ROM, its formats,
  and symbol names from decomps. These packs, more than the model, decide how good extractions are.
  The community can share and improve them.
- **Core** is the shared vocabulary. Every plugin is written against it. It must stay small and
  stable.
- **Runtime** provides generic Bevy implementations of the core interfaces, set up by plugin
  parameters.

### 4.2 Shared vocabulary (core)

**Entity level (first version):**
- `Character`: something that can be controlled and has states
- `Mover`: locomotion parameters and movement model
- `Collider`: collision shape and layers
- `SurfaceSemantics`: what a tile or collision surface does (see §4.4)
- `Ability` / `Item`: an action with inputs, costs and effects
- `Damageable`: health, damage types, invulnerability
- `InputIntent`: abstract input (move, action A/B, etc.), separate from physical buttons
- `Camera`: camera behavior description

**Game level (planned, needed for combining whole games):**
- `WorldGraph`: areas, transitions, how areas connect
- `Progression`: flags, inventory, key items, what gates what
- `Dialogue` / `Events`: cutscenes and scripted sequences
- `Rules` / `HUD`: health model, currency, save model

Keep the first version small. Let the second game show what's missing.

**Vocabulary gap log:** every time an SM64 concept doesn't fit, record it with its cause:
*dimension* (2D/3D), *genre* (platformer vs. action-adventure) or *both*. Expected early gaps:
momentum and air control in `Mover`, slopes and SM64 surface types in `SurfaceSemantics`, a
free-moving 3D `Camera`, and wall-jump and ledge-grab `Ability` interactions.

### 4.3 Space, dimensions and units

- **Everything lives in 3D space.** 2D is treated as a special case. This matches Bevy's design.
- **Gameplay space and presentation are separate.** An entity has a gameplay position (with a
  height) and a separate way of being drawn (sprite, mesh, billboard).
- **Dimension adapters** handle mixing:
  - **2D in a 3D world:** billboard sprites with 2.5D movement (like Paper Mario or the
    Link's Awakening remake). Minish Cap's existing layers and elevation help here.
  - **3D in a 2D world:** locked to a plane and drawn with a fixed camera, or rendered down to
    sprites.
- **Unit normalization:** each plugin states its native scale (e.g. 16px tile, SM64 units).
  Clockwork converts everything to meters.

### 4.4 Surface semantics

Minish Cap has tiles; SM64 has collision triangles with surface types. Both are described the same
way: a surface is a **set of shared properties**, not one fixed type.

**Shared properties (first version):**

| Property | Meaning | Minish Cap | SM64 |
|---|---|---|---|
| `Solid` | Blocks movement | Walls, rocks | Walls |
| `Walkable` | Can be stood on | Floor | Floor triangles |
| `FallThrough` | Falling into it leaves the area or causes a fall | Holes, pits | Death planes |
| `Swimmable` | Swim movement | Deep water | Water volumes |
| `Shallow` | Wading: slows, no swimming | Shallow water | — |
| `Damaging(kind)` | Hurts on contact (`Fire`, `Spikes`, `Electric`…) | Lava, spikes | Lava, burning surfaces |
| `Slippery(amount)` | Lower grip | Ice | Ice, slides |
| `Climbable` | Climb movement | Ladders, vines | Poles, climbable walls |
| `Friction(value)` | Surface grip | — | Per-surface friction |
| `Ledge(direction)` | Jump down in one direction only | Ledges | — |
| `SolidParts(mask)` | Only some 2×2 quarters of a cell are solid | Partial walls, tree edges | — |
| `Extension(id)` | A declared game-specific surface (see below) | `tmc::MinishPortal` | — |

A single surface can combine properties (e.g. lava = `Damaging(Fire)` + `FallThrough`).

**Game-specific extras** that don't fit the shared properties are namespaced
(`tmc::MinishPortal`, `sm64::QuicksandDeep`). Each extra **must declare a fallback** made of shared
properties, so a game that doesn't understand it still behaves sensibly:

```ron
// surfaces.ron (in a Minish Cap plugin)
Extension(
    id: "tmc::MinishPortal",
    fallback: [Walkable],
)
```

**Rules:**
- Plugins `require` properties, not specific tiles (`surfaces = ["FallThrough"]`).
- A plugin that understands an extra uses it. Everything else uses the fallback.
- An extra used by several games can later be promoted to a shared property in a new `core` version.
- Unmapped surfaces in a recipe are an extraction error, so nothing silently ends up with no meaning.

---

## 5. Plugin format

Plugins are **data first**. On disk, a plugin is a folder:

```
tmc-cane-of-pacci/
├─ clockwork.toml     Manifest (TOML)
├─ recipe.ron         Extraction recipe (RON)
├─ ability.ron        Configuration for generic systems (RON)
├─ behavior.wasm      Optional custom behavior (WebAssembly component)
└─ tests/             Optional behavior checks
```

A plugin contains:

| Part | Contents |
|---|---|
| **Manifest** | Name, version, Clockwork core version, what it `provides` and what it `requires` |
| **Source identity** | Hash(es) of the exact ROM version(s) it applies to |
| **Extraction recipe** | Deterministic steps that pull assets and data out of the user's ROM, run locally |
| **Configuration** | Parameters and state machines that configure generic runtime systems |
| **Custom behavior** *(optional)* | Sandboxed scripts or WebAssembly for mechanics the generic systems can't express |
| **Tests** *(optional)* | Behavior checks, optional trace comparison against the original |

**Why not compiled Rust plugins:** Bevy has no stable binary interface, so shipping compiled plugins
is fragile. Data plus sandboxed scripts is portable, easy to review and easy to edit in the Editor.

**Example dependencies:** Cane of Pacci
- `provides`: `Ability::Flip`
- `requires`: `SurfaceSemantics::FallThrough`, `Character::Jump`

### 5.1 Data format: TOML manifest + RON content

- **Manifest (`clockwork.toml`)** is TOML, written like `Cargo.toml`: easy to read and hand-edit,
  with good editor support.
- **Content (`*.ron`)** is RON, Bevy's native format. It maps directly onto Rust structs and enums
  (`SurfaceSemantics::FallThrough`, `InputIntent::ActionB`), so it suits recipes, parameters and state machines.
- Both are deserialized with `serde` into types from `core`. Validation errors point to the exact
  file and field, and they go back to the agent in its revise loop.

```toml
# clockwork.toml
[plugin]
name = "tmc-cane-of-pacci"
version = "0.1.0"
core = "0.1"

[source]
game = "minish_cap"
sha1 = ["<hash of the supported ROM revision>"]

[provides]
abilities = ["Flip"]

[requires]
surfaces = ["FallThrough"]
character = ["Jump"]
```

```ron
// ability.ron
Ability(
    name: "Flip",
    input: ActionB,
    projectile: Some((speed: 3.0, range: 6.0)),
    on_hit: [Flip(duration: 2.0)],
)
```

### 5.2 Scripting runtime: WebAssembly + WIT

Custom behavior ships as **WebAssembly components**, run by `wasmtime` inside the runtime.

- **Stable boundary:** the interfaces custom behavior can call are defined as **WIT** files
  generated from `core`, and versioned with it. This gives plugins a stable interface even though
  Bevy itself has no stable binary interface.
- **Sandboxed:** a component can reach only the host functions Clockwork gives it. It has no file
  system or network access.
- **Any language:** the agent writes Rust compiled to WebAssembly (the same language as `core`,
  typed against the WIT bindings). Humans can use any language that targets WebAssembly components.
- **Cost:** people who hand-write custom behavior need a build toolchain. Most plugins shouldn't need
  custom behavior at all (§3). The Editor or CLI can wrap the build so modders don't have to set it up
  themselves.

### 5.3 Distribution: archives + git index

- **The unit is one archive file** (`.cwplugin`): the plugin folder zipped, plus a checksum. It can be
  shared anywhere: Discord, GitHub releases, a USB stick. Installing from a file always works, with no
  account and no network.
- **Knowledge packs** use the same mechanism (`.cwknowledge`), with the same manifest and versioning.
- **A central git repository acts as the index**, like the crates.io index, Homebrew or Scoop. Each
  entry lists a plugin's versions, download URLs, checksums, and its `provides`/`requires`. Publishing
  is a pull request against the index. No server is needed.
- **Versioning:** plugins use semver. Each one declares the `core` version it targets; the WIT
  interfaces are versioned with `core` (§5.2).
- **Dependency fetching:** the Editor resolves unmet `requires` by searching the index for providers.
- **Later, if needed:** a hosted registry with accounts and search, built on top of the same index
  format so nothing has to change for existing plugins.


### 5.4 Content folders: nothing game-specific in Clockwork

Clockwork itself contains no game code, no game data and no game-specific knowledge, so it can be
shipped cleanly. That includes reimplemented game logic: engine code only implements console
formats (codecs, graphics, display lists), written from format knowledge. Everything about a
particular game lives in **content folders**, outside the repository:

- Each content folder holds `plugins/` (recipes: offsets and parameters) and `knowledge/`
  (notes per game, and `decomps.ron`: known decomp projects by ROM hash, which the editor
  suggests for Library games).
- The data folder is always a content folder. More are added in Settings → Plugins & Agents
  (`<data>/content.ron`) or with `CLOCKWORK_CONTENT`. New plugins go to the first.
- The editor and the agent search the same folders (`clockwork_library::content`).
---

## 6. Agent

The agent runs the extraction loop:

1. Read the request, the Library entry and the game's knowledge pack.
2. Analyze the relevant data and code (decomp output where available).
3. Write the plugin (recipe + configuration + optional custom behavior).
4. Load it into a **headless Bevy test scene** and check that it behaves sensibly.
5. Revise until the checks pass, then hand the plugin to the user.

Tools are exposed to the agent through an **MCP server**: read ROM regions, query the knowledge pack
or decomp, run decoders, render previews, run test scenes. If a decomp exists for a game, the agent's
job becomes translation, not reverse engineering from scratch. This makes decomps a key accelerator.

### 6.0 Tools (implemented: `clockwork-mcp`)

All tools are game-agnostic; game knowledge comes from the ROM, the linked decomp, knowledge
packs and existing plugins.

| Area | Tools |
|---|---|
| Library | `list_games` |
| ROM | `read_rom` (hex/u16/u32), `search_rom`, `decompress` |
| Decomp | `decomp_sync` (shallow `git clone` into `<data>/decomps/`), `decomp_list`, `decomp_search` (regex), `decomp_read` |
| Knowledge | `recipe_guide`, `list_knowledge`, `read_knowledge`, `list_plugins`, `read_plugin` |
| Results | `run_recipe` (summary + image previews the agent can see), `save_plugin` (runs the recipe first) |
| External programs | `list_external_tools` |

**External programs** (git, Ghidra, decompilers) live in their own directory, `<data>/tools/`, one
folder per tool, found before the system PATH. Clockwork does not download them; the user installs
them there. Ghidra is listed but not wired into any tool yet.

### 6.1 Hosting: MCP first, any model

- **Phase 1:** Clockwork ships the MCP server. Any MCP client can drive extractions from day one:
  Claude Code, Claude Desktop, or others. Clockwork doesn't need its own agent UI to be useful.
- **Phase 2:** the Editor gets a **built-in agent panel** that runs the same loop against the same
  MCP tools, using the user's own API key. Claude is the default; the model backend is pluggable.
- **Cost:** users pay for their own model use. Clockwork runs no servers. Cost per extraction is
  measured during M3.
- **What leaves the machine:** recipes run locally, and the ROM is never uploaded. The agent does send
  the excerpts it reads (decomp code, data dumps) to the model provider. The Editor should say so
  clearly.

---

## 7. Editor

A Godot-style composer for combining plugins:

- Browse the Library and installed plugins. The Library is saved as `library.ron` with one
  record per game (`clockwork_library::Game`: its file and what was learned from it, its full
  name and its decompilation), shared with the agent. A game whose file can't be read stays in
  the Library: the toolbar's **⚠ missing** menu lists each with where it was, **Locate…** (pick
  the file where it is now; the same game by hash or a file at its old place counts as found)
  and **Forget**. Games are read and hashed on a worker thread (`editor/src/imports.rs`), so a
  disc image never stalls the window: files and folders can be dropped anywhere or imported
  with **Import games** or **Import a folder of games** (every game file in it and its folders,
  a cue sheet standing for its `.bin` files; `clockwork_library::scan_folder`), and the toolbar
  shows "Importing 12 of 340…" with **Cancel**. The remote control has `Import(["path", …])`.
  Each record also keeps `notes` (`GameNotes`): when the game was added and last used (its ROM
  last read for a plugin), whether it is a favourite (★ on its cover, from its menu or the
  details panel) and its tags (added in the details panel; the search finds games by tag). The
  toolbar's **Filter** shows only favourites, games with plugins or games with a
  decompilation, **Sort** has **Last used**, and a cover shows how many plugins are made for its
  game. The remote control's `Game("title")` shows a game's details.
- **Resolve dependencies:** show unmet `requires`, choose providers or stubs.
- **Resolve conflicts:** competing providers, scale mismatches, input conflicts.
- Choose **dimension adapters** when mixing 2D and 3D content.
- Edit plugin parameters visually (e.g. Link's walk speed as a slider).
- Play-test the composed project in place.

### 7.1 Content browser (implemented)

The Workshop's bottom dock lists what plugins provide, not the plugins themselves. It follows
Godot's FileSystem dock and Blender's Asset Browser (non-modal, non-blocking, select then operate):

- **Items** are a plugin's image outputs, sorted by their `kind` (Area, Tileset, Sprite,
  Character, Ability, Object, Audio; none → Other). Category tabs only show kinds that exist.
- **Appear by themselves:** plugin folders are checked every 2 s (file names and modification
  times); new, changed or removed plugins update the browser without Rescan. The selected and
  loaded plugin are kept across rescans. Only plugins that changed, and those using them, are
  extracted again.
- **Previews** are drawn in the background from the Workshop's extraction, one plugin at a
  time, and cached at `<data>/cache/previews/<plugin>-<hash>/` (hash of manifest, recipe or
  composition and ROM SHA-1), so they're only regenerated when something changes; folders of
  earlier versions are removed. Items show "Generating…", "Needs ROM" or "No preview"
  instead; the UI never waits.
- **Search** (name, plugin, game, kind), a **game filter** and a **thumbnail size** slider;
  filters survive refreshes. Very long sheets show their first square instead of a sliver.
- **Select, then operate:** click selects (the Inspector shows the plugin); double-click, Enter or
  dragging onto the viewport shows the item there.

### 7.2 Scene tree (implemented)

The Workshop composes a **scene**: a tree of nodes, each with a transform (position in meters,
height, rotation about the vertical axis, uniform scale), a visibility flag, and optionally an
item (plugin + output, referenced by name, never game data). Like Godot's scene tree:

- **Placing:** drop content on the viewport (placed where the cursor points on the ground), on a
  tree node (placed as its child, at its origin), or use *Place* in the Inspector / double-click
  in the browser (placed at the camera's focus).
- **Parenting:** drag a node onto another in the tree, or pick a parent in the Inspector. The node
  keeps its place in the world. Cycles are refused.
- **Adjust after drop** (Blender's redo panel): the newly placed node's position, height,
  rotation, scale and parent can be edited live in a small viewport panel until Done, Escape or
  selecting something else.
- **Inspector** for nodes: name, item, transform, visibility, add empty child, delete, and the
  item's surfaces with the hovered cell. Delete key removes the selected node.
- **Saved** to `<data>/scenes/untitled.ron` (at most once a second after changes) and restored,
  framed, at startup.
- Recipes run on demand when a node needs them and are cached; plugin changes re-extract.
- **Undo / redo** (Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z, Edit menu): the scene is snapshotted once an
  edit settles (no mouse button held, no text field focused), so a whole drag or a typed name is
  one step. 100 steps; shortcuts are left to text fields while one has focus.

Next: several named scenes / projects, saved dock layouts, snapping, and playing a scene.

### 7.3 Settings (implemented)

A Settings screen (Blender Preferences style: categories on the left) saved to
`<data>/settings.ron`, applied live:

- **Interface:** UI scale.
- **Library:** fetch box art, retry missing box art, data folder, clear the preview cache.
- **Workshop:** surfaces shown at startup, scene autosave, orbit / pan / zoom speed, invert
  vertical orbit, browser thumbnail size.
- **Plugins & Agents:** watch plugin folders, plugin folders, running agents, the MCP commands to
  connect an agent, the external tools folder.
- **Controls:** connected controllers, Xbox/PlayStation/generic prompts, stick deadzone and
  gameplay key/button capture. Personal overrides inherit each game's defaults, with explicit
  shared-binding conflict handling, clear/reset actions and automatic saving. Editor shortcuts
  remain in **Keyboard**; gameplay keys take precedence during Play.
- **About**, and *Reset all to defaults*.

### 7.4 Models and characters (implemented)

A recipe can output a **model**: a tree of bones, meshes that move rigidly with one bone each,
textures, and animations (per-bone translation and rotation per frame). The shared types live in
`clockwork_core::model`, so every platform decodes into the same thing.

- **N64:** `Mio0` decompression and a Fast3D display list interpreter (`clockwork_n64`):
  vertices, triangles, lights, textures in every N64 format, decals baked over the light color,
  wrap modes. The `N64Model` step lists bones (offset, parent, display lists); the recipe author
  reads them from the game's scene graph (SM64 geo layouts), so no engine format is hard-coded.
- **Animations:** `ChannelAnimation` decodes value lists per channel (SM64's layout, described by
  offsets in the step).
- **First character:** Mario from Super Mario 64 (a plugin kept in a content folder, §5.4): Mario's high-poly model with idle, walk and run.
- **Editor:** models appear under Characters in the content browser (software-rendered previews,
  also used by the agent's `run_recipe`), are placed like any item, and play their animation in
  the viewport; the Inspector picks the animation (undoable, saved with the scene).

Next: more animations and states (crouch, long jump, swim), and characters from other games.

### 7.6 Play mode (implemented)

*Play* in the viewport's toolbar drives a character (the selected one, or the first in the
scene) with the runtime's generic controller (`clockwork_runtime::controller`): camera-relative
WASD or arrows, Shift to walk, Space to jump, Escape or *Stop* to end.

- **Feels like the original:** the model output's `character` holds only data, taken from the
  game's own movement code: an acceleration curve, braking, turn rate, skids (turning around
  and stopping at speed), a chain of jumps (single, double, triple) with speed-dependent height
  and shorter jumps when the button is let go, air control (SM64 doesn't turn in the air), an
  animation per state (idle cycle, walk/run with hysteresis and speed-matched playback, skid,
  turn, jump rise/fall/land), a bone that leans into turns, and a follow camera that swings
  behind. The controller (`clockwork_runtime::controller`) implements these mechanisms
  generically; Mario's choices and numbers live in his plugin, never in Clockwork.
- **Scale between games:** a 2D map at true scale (one tile = 1 m) is cramped for a 3D
  character running at 9.6 m/s; scaling the area node (×3–4) gives it room. Adapters that
  choose this automatically belong to M5.
- **Surfaces are the world:** every visible area with a surface map is ground at its height.
  `Solid` and the solid quarters of `SolidParts` are walls, `FallThrough` and anything outside
  an area are holes; game-specific surfaces use their fallbacks. Falling out of the world
  respawns at the start. Content from different games works together this way.
- **The scene is never changed:** the played node's entity moves, the scene doesn't, so
  stopping puts everything back and undo history is untouched.
- The remote control can `Play`, `Stop`, hold a `Stick` direction and `Jump`.

### 7.7 Sprites and 2D characters (implemented)

Console sprites are not images: a frame is a few hardware objects (OAM entries: rectangles of
tiles) placed around the character's origin, and an animation is a list of timed frame numbers
with mirroring. The vocabulary mirrors §7.4 for 2D:

- **`clockwork_core::sprite`:** a `Sprite` is frames (an image plus an origin pixel) and
  animations (steps of frame, ticks, mirroring; a tick rate; an optional loop step). Nothing in it
  is platform-specific.
- **Steps (no game knowledge in code):** `GbaObjectFrames` composes frames from object lists
  (`clockwork_gba::obj`: the hardware's shapes, sizes, flips and layering). Where the lists are
  comes from tables in the recipe (`Table`: count records of stride bytes read with a `Field`: size,
  shift, bits, signed), and a game's own packed object record is described field by field
  (`ObjectLayout::Fields`); `Oam` is the hardware's own six bytes. Optional per-frame tile
  windows cover games that stream a frame's tiles into VRAM. `KeyframeAnimation` decodes a
  keyframe list (frame, duration, flip, end flag; stop, restart or "rewind by the byte after") at
  an offset or through a pointer table. Other consoles add a composer step and reuse the rest.
- **`Sprite` output** with `pixels_per_meter`, and optionally a `character`
  (`clockwork_core::character2d`): speed, optional acceleration, collision radius, one animation
  per facing for idle and walk (each can mirror), and a camera. Every named animation must exist.
- **Play mode:** `clockwork_runtime::topdown` is a pure top-down controller: it walks on the ground
  of surface maps (walls and holes block, with sliding), faces the stronger stick axis (north is
  up the map) and picks the animation of its facing. The editor shows a sprite as a quad lying on
  the ground that swaps frames (`editor/src/sprites.rs`), so it sits on 2D areas like the area
  images; Play looks straight down at it. Sprites appear under their `kind` in the content
  browser (a character's idle frame is its thumbnail), and the agent's `run_recipe` shows a
  contact sheet with one row per animation.
- **First sprite:** Link from The Minish Cap (`tmc-link`): all 253 frames, idle and walk in four
  directions, speed from `kWalkSpeed`. Facts found while extracting are in
  `knowledge/minish_cap/README.md`.

Next: more of Link's states (sword, roll, items), jumping and swimming in the 2D controller, 8-way
facing, and sprites for other games.

### 7.8 3D areas and terrain (implemented)

Games with 3D levels describe the ground as triangles, not cells. A **Model output of kind Area**
can carry a **`terrain`**: collision triangles whose game-specific codes get their meaning exactly
like a surface map's (unmapped codes are an error, extensions declare fallbacks).

- **`clockwork_core::terrain`:** `Collision` (what a platform decodes: vertices and triangles with
  the game's surface codes), `Terrain` (the codes' meaning, one surface "cell" per triangle, so it
  reuses `SurfaceMap`), `Spawn` (where a character starts, in model units) and `Facing`
  (floor/ceiling/wall by the normal's y against a per-game `floor_normal_y`).
- **Collision:** the `CollisionTriangles` step reads a vertex list and triangle groups by code,
  laid out as the recipe's `TriangleLayout` says (word size, byte order, start and end markers,
  extra words per code), so no game's format is built in (`clockwork_recipe::collision`); the
  recipe's `terrain` output says what its codes mean.
- **Runtime:** `clockwork_runtime::terrain::WorldTerrain` is a terrain placed in the world (meters,
  with the node's position, yaw and scale). Floors under a point are the highest one up to a meter
  above the feet (so falling bodies land and stacked levels work), `FallThrough` floors are holes,
  and walls (near-vertical triangles, anything `Solid`) push a body out, from the side they face
  only, as the game's own collision does. Ceilings are not used yet. The controller's `World`
  trait gained `ground_at(x, y, z)` and `push_out(position, radius)`; the defaults keep 2D
  worlds as they were.
- **Play mode:** every visible area with a terrain is part of the world, next to the image areas.
  A character that isn't standing on a terrain when Play starts begins at the area's start point.
- **First area:** Castle Grounds from Super Mario 64 (`sm64-castle-grounds`): the level's
  geometry and textures, and all 879 collision triangles, with Mario's own start point.

- **Dropping onto the ground:** characters and objects dragged into a world (anything that isn't
  an Area or Tileset) land on the first floor under the cursor, found by casting the viewport ray
  against the scene's terrains (`SceneTerrains`, which keeps them placed as nodes move); with no
  cursor (the Place command) they land where the camera looks (the center of the view), and a
  drop onto a terrain area's node in the tree puts them at the area's start point. Without terrain they keep to the
  ground plane.
- **2.5D (2D characters in 3D worlds):** a sprite standing on a terrain is an upright billboard
  that always faces the camera (`SpriteRig::upright`), and Play mode drives it with
  `clockwork_runtime::billboard`: camera-relative movement on the terrain's floors, small steps
  up and down, walls and tall steps block, ledges are fallen off (`Movement2d::gravity`), and
  the facing animation follows the stick as seen on screen. `Character2d::world_camera` sets the
  view (default: 8 m away, 30° down). The same plugin plays top-down on flat maps and 2.5D in 3D
  worlds.

Not yet: swimming (water boxes), special collision objects (doors, bridge, pipes), ceilings,
sliding on slippery or steep floors, skyboxes, level objects and actors.

### 7.9 Actions, items and the inventory (implemented)

What a 2D character does beyond walking is data, like movement. A plugin gives a character
**actions** (`clockwork_core::action`): an animation per facing, a duration, a motion (rooted,
slowed, or sliding along a speed profile: the roll), hit windows (boxes in front, circles around),
invulnerable spans, projectile spawns, and a **hold** (stay in a pose while the button is down,
slowly walking and maybe guarding; letting go after `min_time` performs another action: the sword's
charge and spin, the bow's draw and shot). **Projectiles** (arrows, bombs with a fuse and
explosion, boomerangs that return) are data too. A plugin's **Items** output (`clockwork_core::item`)
lists the game's items with their action, ammunition, capacity, start count and button.

- **Runtime:** `clockwork_runtime::actions::Runner` plays one action at a time and returns, each
  update, the pose to show, how the movement controller is constrained (`Modifiers`: speed factor,
  locked, sliding, fixed facing) and events (started, hits, spawns). `projectiles` moves
  projectiles (walls, returning, bursting), `inventory::Inventory` keeps counts and the A and B
  buttons (equip and swap, ammunition spent per use). Both 2D movers (`topdown`, `billboard`)
  take the modifiers, so the same moves work from above and as 2.5D billboards.
- **Editor:** in Play mode J and K press the B and A buttons, L rolls, I opens the inventory
  (items, counts, put on A or B), H shows or hides hit boxes, which are drawn on the ground for
  a moment when they hit; projectiles are drawn as colored shapes. The remote control has `Roll`,
  `ItemB(frames)`, `ItemA(frames)`, `Equip(slot, item)` and `Inventory`, and reports the action,
  items and counts.
- **Sprites from several sheets:** `MergeSprite` appends the frames and animations of one sprite
  to another, since a game draws one character from several sheets.
- **First moves (Link, `tmc-link`):** the sword (slash, charge, spin attack), the roll, the shield
  (raised while held, slow walk), the bow (draw, hold, shoot), bombs (one at a time, fuse and burst)
  and the boomerang; the roll, sword and shield use the game's own animations, timings
  (keyframe ticks), speeds (`kWalkSpeedRolling` and friends) and charge time.

- **Item graphics:** an action's `attachments` are sprites drawn with the character while it
  lasts (and its hold): `FollowFrame` shows the attachment's frame that goes with the
  character's (character frame + offset, mirrored with it, within a frame range), as games that
  draw held items as a second sprite in step with the character do; `Animation` plays one of
  the attachment's own animations per facing. A projectile's `sprite` draws it with the animation
  of the direction it was thrown in (mirroring per direction, another animation while it bursts).
  An item's `icon` (a sprite frame, an animation's first picture or an image) is shown on its
  button and in the inventory. `GbaObjectFrames` has `first_tile` for sprites whose tiles stay at
  a fixed place in video memory. References to other outputs are checked at extraction, and the
  agent's `run_recipe` shows a playable sprite's actions drawn with their attachments.
- **Link's item graphics (`tmc-link`):** the sword, shield and bow are the game's own item sheets
  (14, 17 and 21, frame offsets from `src/playerItem/*.c`), arrows, bombs and the flying
  boomerang sheet 166 with the tiles `LoadGfxGroups` keeps in object memory. Icons are pictures
  from these sprites, not the item menu's art.

Not yet: things to hit (no enemies or breakable objects exist yet: hits are drawn and counted),
the boomerang in Link's hand while throwing (the game picks its frame from keyframe flags),
explosion sprites, the item menu's own icons, hit box and damage data from the game (the
numbers are approximations from the animations), jumping with the Roc's Cape, digging, lifting,
pushing and pulling, swimming, and the other items (lantern, gust jar, Cane of Pacci, mole mitts,
boots, ocarina, rods, bottles, passive items).

### 7.10 The Workshop as a game-dev tool (implemented)

- **Direct manipulation** (`editor/src/manipulate.rs`): click a node in the viewport to select it
  (Ctrl or Shift adds); W, E and R choose the move, rotate and scale gizmo, G snaps (0.5 m, 15°,
  0.05). The gizmo works on every selected node at once (moving a parent moves its children, locked
  nodes stay put), and dropping with the ground handle keeps characters on terrain. Picking casts
  the cursor ray against the scene's meshes. In the tree: Ctrl or Shift click for multi-select,
  lock toggles, a context menu (Duplicate, Group, Lock, Delete); Ctrl+D, Ctrl+C and Ctrl+V,
  Delete and F (frame) work on the selection. Everything is one undo step per drag.
- **Behaviors** (`editor/src/entities.rs`): a node can do something when played, whatever it
  shows: a **Target** (health, radius, blocks movement, drops an item when destroyed), a **Pickup**
  (gives an item), a **Trigger** (message, teleport, give an item, restart; once or every time), a
  **Checkpoint** or a **Solid** obstacle. They are generic and edited in the Inspector's Behavior
  section; `＋ Entity` in the viewport toolbar adds one where the camera looks. Without art they are
  drawn as simple shapes and rings. The sword, arrows, boomerang and bomb bursts hurt targets; the
  inventory takes pickups and drops.
- **Play and debug:** pause, single step and speed (P, `.`, `[` and `]`, or the toolbar), a live
  panel with the character's state and every behavior's, hit boxes (H), and a View menu that
  draws behavior rings and the collision triangles of 3D areas.
- **Export:** File, Export game writes a folder with the player (a copy of this program, which
  plays a `game/` folder next to it), the scene and only the plugins the scene uses. It holds no
  game data: the player asks for the person's own copy of each game the first time, remembers where
  it is, and starts playing full window (Esc quits). `clockwork --play <folder>` plays any game
  folder.
- **Remote control** also has `AddEntity`, `SetBehavior`, `Move`, `SelectAdd`, `Duplicate`,
  `Group`, `PickAt`, `ToolMode`, `Pause`, `Resume`, `Step`, `Speed`, `Debug` and `Export`.

Not yet: a gizmo for rotation about other axes (nodes turn about the vertical axis only), several
named scenes in a project, behaviors that react to each other (a trigger that opens a door),
enemies that move or attack, and exporting for other platforms than the one the editor runs on.

### 7.11 Abilities and composition plugins (implemented)

Moves go from one character to another, across games and between 2D and 3D:

- **Moves** (`clockwork_core::action::Moves`: actions, projectiles, the dodge button's action) are the same for 2D and
  3D characters. A 3D `Character` can have them too (an action's animation names a model
  animation) and a `hand` bone. The runtime's action runner and the editor's fighting state work
  on `Moves`, so Mario runs Link's actions with the same code as Link: hits, holds and releases,
  projectiles, the inventory and HUD, and the movement limits an action sets (slowed, locked,
  sliding) applied to the 3D controller.
- **Abilities** (`clockwork_core::ability`): an `Ability` output packages some of a character's
  actions (plus what they release into), the projectiles they throw and the items that start
  them. Every pose becomes a **role** (`slash`, `slash_hold`, ...) with the source's animations
  as its default. A learner binds roles to its own animations (`Animation` for all facings,
  `Directional` per facing, `Default` for the source's); unbound roles play its idle pose, so a
  move always works even when it can't be shown. Held items have a `still` frame for characters
  that can't follow them frame by frame: a 3D character holds it at its hand bone, upright
  towards the camera.
- **Composition plugins** (`clockwork_core::compose`, `compose.ron` instead of `recipe.ron`;
  manifest game `composition`, no hashes) hold only references and choices: a character
  (`base`: another plugin's model or sprite output), the abilities it learns with their role
  bindings, movement tuning (speed, jump) and the hand bone. `clockwork_recipe::compose` turns
  one into an ordinary extraction: the sprites learned moves show (held items, projectiles,
  icons) are copied in under `"<plugin>/<output>"` and references renamed, so everything that
  shows or plays outputs works unchanged. The editor extracts the plugins it uses first; the
  content browser lists its characters (with previews) under "Compositions".
- **First composition:** `mario-with-links-items`: Mario with Link's sword, shield, bow, bombs,
  boomerang and roll (his triple jump flip as the roll's pose). `tmc-link` outputs those six
  abilities.

Not yet: Mario's own attack animations (only movement ones are extracted, so most of Link's poses
play his standing pose), learning abilities from 3D characters into 2D ones with their poses
(the moves work, poses fall back to idle), and abilities that change movement itself (Roc's Cape,
Pegasus Boots).

### 7.12 Plugin editor (implemented)

A **Plugins** screen next to the Workshop (`editor/src/plugin_editor.rs`), following the Settings
screen's layout (plugins on the left, the selected one on the right):

- **Compositions** are edited as a **node graph** (`editor/src/compose_graph.rs`, drawn with
  `egui-snarl`). Three kinds of wire: *character* (a playable character of any extracted plugin),
  *ability* (any plugin's ability) and *learned*. A **Learn** node takes an ability and holds what
  plays for each of its roles ("Not chosen", "The ability's own pose", or any animation of the
  character it is wired into) and whether unchosen roles use the ability's own poses (follows the
  wiring, on for a character from the ability's own plugin, until set by hand). A **character of
  this plugin** node takes a base on its first input and learned abilities on the inputs after it
  (one free input always waits for the next), with its name, speed, jump height and hand bone.
  Right-click the canvas to add nodes, drag a wire into empty space to add a node already wired
  (from a Learn input, an ability; from a character's free input, an ability and its Learn node),
  right-click a node to remove it (or press Delete for the selected nodes) or place its
  character in the scene. Undo/Redo (and Ctrl+Z/Ctrl+Y) step through finished changes (a drag
  or an edit, once the pointer is up and nothing is typed). Inputs take one wire and a
  Learn node teaches one character, so rewiring replaces. Problems that block saving (a character
  without a base, two characters with one name) and parts that aren't saved (a stray Learn node)
  show above the canvas.
  The graph is a view: **Save** writes `compose.ron` as before, plus `graph.ron` next to it with
  where the nodes sit (keyed by what they stand for, e.g. `learn/<character>/<plugin>/<ability>`)
  and characters and abilities placed but not wired yet. Without a `graph.ron` (a composition the
  agent wrote) the nodes are arranged in columns by their size; **Tidy up** does that again.
  **Revert** drops the changes. A name and **Create** make a new composition plugin next to the
  other plugins.
- **Plugins from your games** have two tabs. **Recipe** (`editor/src/recipe_graph.rs`) draws the
  recipe as a node graph: a node per step and per output, wired by the names steps make and use.
  Every step and output lists those names with how it uses them (`Step::ports`, `Output::ports`
  in `core`: read, write, update, or grow, which makes the name if no earlier step did), so
  wires follow the recipe's order; an input nothing earlier makes shows red. Each node edits its
  values in place (numbers drag and keep being written in hex if they were, other values are
  edited as RON) and shows what its step made on the user's ROM (`run_observed` reports after
  every step): sizes, counts, an image preview, or the error of the step that failed. Dragging a
  wire from an output to an input renames what the input uses (refused if the order of steps
  would make it read something else); dropping a wire on empty space or right-clicking the canvas
  adds a step or output from a template (already wired when dropped from a pin). A node's menu
  runs it earlier or later (it swaps lines with its neighbour; comment lines above stay, as they
  often describe a group), duplicates it (at the end, with the names it makes renamed) or removes
  it; Delete removes the selected nodes. Buffers show their first bytes in hex, sprites their
  first frame, and outputs what the whole recipe made of them (or the error of the output that
  failed). After a change the recipe runs again in the background (nodes show the last run
  until it is done). Undo/Redo (and Ctrl+Z/Ctrl+Y) cover every change.
  The graph is a view over `recipe.ron` as text (`clockwork_recipe::text`: where every value sits
  in RON): each change replaces a span and is parsed again before it is kept, so the agent's
  comments survive and nothing the format can't hold is written. **Save** writes `recipe.ron` and
  `graph.ron` (node positions by a key that survives steps added or removed elsewhere: the
  variant and the first name a step makes, or an output's id; older files by place still load),
  and the plugin is extracted again. Without a layout the
  steps are arranged in columns by distance from the ROM (a step that only adds to something
  stays level with it); **Tidy up** does that again. **Provides** shows their characters and
  actions (with *Place in the scene*), abilities (actions, items, roles) and how much they provide.
- **Groups** (`editor/src/recipe_graph/groups.rs`) fold part of a recipe into one node, which
  shows only what crosses its edge: an input per value it reads from outside, an output per value
  made inside and used outside, how many steps it holds, the last image made in it and any of
  its steps' values chosen with *Shown on its group* (edited there without opening it).
  Double-click (or **Open**) shows what is in it inside a frame; the first time, it lays its
  steps out as a grid of the chains in it and moves what is below and to its right out of the
  way. Ctrl+G groups the selected nodes; groups nest, and are kept in `graph.ron` with their
  members by key, so they change nothing in `recipe.ron`. A recipe of 40 nodes or more that was
  never grouped is grouped when it opens: by its section comments (`// == Tiles ==`,
  `// -- Tiles`, `// ## Tiles`, or without those a comment after a blank line; an output joins
  the section that makes all it shows), or for 80 nodes or more without sections by the output
  each step goes into. With nothing selected the Inspector lists the groups and can group again
  by sections or by outputs, or ungroup all. Recipes of thousands of steps open and stay
  responsive: wires and lineage are looked up, not searched.
- **Blocks** (`editor/src/recipe_graph/blocks.rs`) are higher-level nodes: the add menu (Space
  or right-click) lists them first. *GBA tileset*, *GBA tile map*, *GBA palette* and *N64
  texture* each add the steps and output usually written together (after the selected step, or
  last) as a closed group that shows the values worth changing, such as each Read's offset and
  the image's columns, labelled by what their step makes. The names they make get one shared
  number (`tiles_2`, `bank_2`, ...) so a block added twice doesn't clash. Once added they are
  plain steps in `recipe.ron`; open the group to change anything else.
- Recipes written by the agent are easiest to change where they are explained: the Inspector's
  first field is a step's **note**, the `//` lines right above it (up to a blank line or a
  section header), edited as text and written back as comments (`Doc::note`, `Doc::with_note`).
  The **Text** tab edits `recipe.ron` itself: each change that parses is made as it is typed
  (undone together, like typing in a field; nodes keep their places and groups by key), one that
  doesn't says why and waits. Opened from the graph, it puts the cursor on the selected node.
- Both graphs have **Find** (Ctrl+F; matches are outlined and the view moves to each in turn,
  searching a step's whole text in a recipe) and **Fit** (the whole graph), and zoom out far
  enough for recipes of a hundred steps (`editor/src/graph_view.rs`).
- **Changes are kept per plugin** while others are shown; a plugin with changes that aren't
  saved has a ● in the list (and the Plugins tab one too), and quitting (the window or
  File → Quit) asks first. A plugin file changed on disk (the agent) is loaded again while there
  are no changes here; with changes, a banner offers to load it (dropping them) or keep them.
- The editor extracts every plugin it can (one per frame) to list characters and abilities. The
  remote control has `Screen(Plugins)`, `PluginEditor("name")` and `Graph(…)` for the graph of the
  plugin shown: `Find`, `Fit`, `Undo`, `Redo`, `Save`, `Revert`, and recipe edits by name
  (`Recipe(Value("step 0", "offset", "0x40"))`, `Wire("step 0", "tiles", "step 4", "metatiles")`,
  `Add("Crop")`, `AddBlock("GBA tileset")`, `Note("step 3", "From the gfx table")`, `Text("…")`, `Move("step 5", false)`, `Duplicate("output 0")`, `Remove("step 3")`), groups
  (`Group(["step 3", "step 4"], "Tiles")`, `OpenGroup("Tiles")`, `CloseGroup("Tiles")`,
  `Ungroup("Tiles")`, `GroupAutomatically(false)`, `true` to group by outputs) and
  `ShowText(true)` for the Text tab; `Report`
  adds the graph's state (steps that ran, wires, missing inputs, error, nodes drawn, groups,
  unsaved).

Not yet: an action timeline (hit windows, spawns and holds dragged on a time line with live
playback), previews of a role's animation in the role menu, and moving the comment above a step
with it.

### 7.5 Remote control (for testing)

With `CLOCKWORK_CONTROL=<folder>`, the editor runs command files dropped into that folder
(`*.ron`, a list such as `[Screen(Workshop), Frame("Mario"), Animation("Mario", "walk"),
Wait(30), Screenshot("walk.png"), Quit]`) and writes `<file>.done` when done. Screenshots come
from the renderer, so tests never move the user's mouse or need the window in front.
A file is read as soon as it appears, so scripts write it under another name (`cmd.tmp`) and
rename it to `cmd.ron` when complete.

---

## 8. Legal principles

- Users supply their own legally obtained copies. Clockwork never downloads games.
- Plugins contain recipes, not game data. This mirrors ROM hack patches and PC ports that need your
  own ROM.
- **Caveat:** behavior reimplemented from decompiled code is a greyer area than asset extraction.
  Data-first plugins (parameters and state machines on generic systems) reduce this risk. Custom
  behavior derived closely from original code needs more care when shared publicly.

---

## 9. Roadmap

| # | Milestone | Proves |
|---|---|---|
| **M1** ✓ | Library import + ROM hash; extract a Minish Cap tileset into Bevy | Library, GBA module, recipe format |
| **M2** ✓ | Render one Minish Cap overworld area with its collision (`tmc-minish-woods`) | Tier 2, `SurfaceSemantics`, runtime |
| **M3** | Agent + MCP tools (tools done: `clockwork-mcp`); extract "feels like" Link movement (first step ✓: Link walks, §7.7) | Tier 3, agent loop, generic controller |
| **M4** | First SM64 extraction, staged: assets (Mario ✓) → one course → "feels like" Mario movement | 3D path, N64 module, gap log |
| **M5** | Combine Minish Cap and SM64 content in one scene (first step ✓: Mario plays on Minish Woods' surfaces) | Dimension adapters, units, shared vocabulary |
| **M6** | Editor MVP: dependency resolution and parameter editing (first step ✓: plugin editor for compositions, §7.12) | Composition workflow |

M1 and M2 need no AI on purpose. They build the foundation the agent stands on.

---

## 10. Decisions

| Date | Decision | See |
|---|---|---|
| 2026-10-21 | Plugins use a TOML manifest and RON content | §5.1 |
| 2026-10-21 | Custom behavior uses WebAssembly components with WIT interfaces (`wasmtime`) | §5.2 |
| 2026-10-21 | Second game is Super Mario 64 | §2 |
| 2026-10-21 | Agent is MCP first and works with any model; built-in Editor agent later, default Claude | §6.1 |
| 2026-10-21 | Distribution uses `.cwplugin` / `.cwknowledge` archives and a git index | §5.3 |
| 2026-10-21 | Surfaces are sets of shared properties plus namespaced extras with fallbacks; `TileSemantics` renamed `SurfaceSemantics` | §4.4 |
| 2026-10-21 | ROM hashes are taken in the platform's canonical form (N64: big-endian `.z64`), matching No-Intro | §5 |
| 2026-10-21 | Recipe steps are typed RON enums over named buffers, palette banks and images; platform-specific steps are prefixed (`GbaPalettes`, `GbaMetatiles`, …) | §5.1 |
| 2026-10-21 | GBA content is placed at 16 px = 1 m (one tile per meter) | §4.3 |
| 2026-10-21 | Library games can be linked to a decompilation repository; known decomps are suggested by ROM hash from `knowledge/decomps.ron`. The agent (M3) uses the linked decomp as its main source | §6 |
| 2026-10-21 | Game-specific knowledge lives in plugin data, never in Clockwork code: recipes use generic steps (`Lookup`), and what a game's codes mean is a code → property table in the plugin (`Surfaces` output) | §4.4, §5.1 |
| 2026-10-23 | Nothing names a game: collision is read by a data-described `CollisionTriangles` step (replacing SM64-only `N64Collision`), known decomps come from content folders, the roll slot is the `dodge` and the boomerang motion `Return` (old names still load) | §4, §5 |

## 11. First agent test (2026-10-21)

An agent with no prior context, using only Clockwork's MCP tools, extracted Hyrule Town
(`plugins/tmc-hyrule-town`): the area, both tileset sheets and surfaces, correctly rendered,
including the town's camera-dependent graphics swaps, which it found in the decomp by itself.
About 55 agent steps and 130 tool invocations, roughly 190k tokens and 12 minutes.

Gaps it reported, all addressed since:
- **No crop/mask step.** Compositing regions took ~460 generated `Place` steps that blank map
  spans with zero runs found in the ROM (USA-only). → `Overlay` takes a `region` (and
  `replace: true`), plus new `Crop`, `Fill` and `Literal` steps. `tmc-hyrule-town` went from 490
  to 125 steps, pixel-identical, without borrowed ROM bytes.
- **No surface preview.** It built magenta "highlight" probes to see where codes sit. →
  `run_recipe` takes `surfaces: {}` (property overlay) or `surfaces: {grid, codes}` (highlight).
  The overlay drawing is shared with the Workshop (`clockwork_recipe::preview`).
- **Previews only shrink.** → `run_recipe` takes `zoom: {image, x, y, w, h}`; transparent pixels
  show as a checkerboard.
- **The guide doesn't mention runtime graphics swaps.** → `recipe_guide` has a section on them.
- Numeric arguments accept integers; surface summaries list every code per grid in hex with
  counts; opacity is reported exactly.

## 12. Open questions

- **Trust:** should index entries be signed? What review, if any, does a pull request to the index get?
- **Cost per extraction:** to be measured in M3. Does it suggest a cheaper default model for simple
  (Tier 1/2) extractions?
- **Promotion policy:** when does a namespaced surface extra become a shared property in `core`?

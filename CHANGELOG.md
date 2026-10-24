# Changelog

## Unreleased

### Added

- Scripts: the effect `Run` starts commands that run over time (`Do`, `Wait`, `Until`, `Walk`,
  `Place`, `Hold`, `If`, `Repeat`, `End`), for cutscenes and events. A script can hold the
  player still, and nodes it walks take their behaviors along. The Inspector edits scripts, and
  content checks report scripts that never wait or name nodes that don't exist.
- `Music` recipe output: an m4a song (the music engine most GBA games use), rendered with its
  instruments and looped where it loops. Games play music from the start (the `music` game
  setting) and change it with the `Music` effect. The agent's `find_songs` tool finds a ROM's
  song table.
- `Places` recipe step: reads where a game places things (rooms, doors, enemies, objects) from
  its own tables of records, with filters, an end marker, names and groups made from record
  fields, the output each shows, door targets and how far each reaches.
- `MarkPlaces` recipe step: marks the cells placements cover with a code from their record,
  for surface grids of where things are.
- `Scene` output: a plugin's placements as a group of Workshop nodes, with `Door` behaviors.
  Listed under World in the content browser with a map preview; placing it adds the whole
  group.
- `depth: Bpp8` on `GbaMetatiles` and `GbaMetatileMap` for 256-color backgrounds.
- `image` on `Surfaces` outputs: the image a surface map covers.
- `Actor` behaviors: enemies, NPCs and moving things as state machines of plain data (how
  each state moves: still, wander, chase, flee, charge, home or patrol; its animation, contact
  damage and entry effect; and when it changes: near, far, after, hurt, health, arrived,
  blocked, chance or a game condition). They take hits with knockback, hurt on contact, drop
  items and count for `DestroyAll`. The Inspector edits their numbers and states.
- `behavior` on `Places` steps: the `Behavior` output each placement does, picked by a record
  field like `item`, so a room's enemies come with their actors.
- `clockwork_runtime::play`: Play's simulation (the played character, its actions and items,
  the scene's behaviors, rules and goals) runs without a window, so it can be tested headless.
  The editor's Play mode drives it.
- `layers` on area `Image` outputs: background and foreground layers with a draw order, a
  scroll factor (parallax) and wrapping, and layers drawn over sprites. Play scrolls them with
  the camera.
- `frames` on area images and their layers: animated tiles (water, lava, flowers) as images
  shown in turn, each for its ticks. The Workshop and Play animate them; `run_recipe` previews
  an area with all its layers.
- `play_test` agent tool: plays a plugin's Scene output without a window from a script of
  inputs (hold a direction or walk toward a node, press buttons, for so many seconds) and
  reports what happened and when, how each step ended or got stuck, the end (health, lives,
  items, won or game over) and what was never reached. Behaviors and nodes can be added for
  the test. The same script always plays the same way.

### Fixed

- A surface map is matched to the image it names, so images of the same size no longer share
  the first surface map of that size in the Workshop, Play mode and `run_recipe` previews.

## 0.1.0 — Preview

Initial preview release. These features are available for experimentation; importing a game
does not emulate it or provide a complete extraction automatically.

### Included

- A Bevy-based editor with a local game Library, plugin workflow and Workshop for combining
  extracted content.
- Shared data formats and runtime systems for plugin-defined content and gameplay.
- Game-file identification, hashing and byte access for GBA, N64, SNES, Nintendo DS, PS1,
  PS2 and PC executables, within the format limits documented in the README.
- Cartridge metadata and box-art lookup, with a user option to disable online lookups.
- The `clockwork-mcp` extraction server, available over stdio or local HTTP.
- Gameplay key and controller-button rebinding in Settings, persistent personal overrides,
  shared-binding conflict handling and reset controls.
- Xbox, PS5/PlayStation and generic controller labels, connection handling, adjustable stick
  deadzone and right-stick camera control during Play.
- A Windows x64 release package containing the editor, MCP server, licenses and dependency
  notices, with a SHA-256 checksum file.

### Known limitations

- This is a content extraction and composition toolkit, not an emulator or a launcher for
  the original games. Game-specific extraction requires recipes and suitable decoders.
- No game data, BIOS files, game-specific content packs, AI client or external extraction
  tools are bundled.
- The initial prebuilt release is Windows x64 and is unsigned. Other platforms require a
  source build and are not included in the binary release.
- Import support does not cover every dump or executable format. PC import reads one
  executable rather than an installation folder; disc images are read into memory.
- Controller operation depends on OS support and device mappings. Rumble, adaptive triggers
  and touchpad gestures are not implemented. Editor shortcuts cannot be rebound.
- Minimum hardware requirements and compatibility across all supported controller families
  have not been established through a hardware test matrix.
- Preview data formats and behavior may change. Back up personal data and content folders
  before updating.

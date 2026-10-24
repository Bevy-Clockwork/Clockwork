# Changelog

## Unreleased

### Added

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
- `clockwork_runtime::play`: Play's simulation (the played character, its actions and items,
  the scene's behaviors, rules and goals) runs without a window, so it can be tested headless.
  The editor's Play mode drives it.

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

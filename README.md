# Clockwork

A shared layer between existing games and the Bevy engine: import games you own, have an AI agent
extract characters, mechanics and areas as shareable plugins, and combine them in an editor.

See [DESIGN.md](DESIGN.md) for the full design.

## Preview release

Clockwork **0.1.0** is an early preview for building and inspecting game-content plugins.
Import compatibility means that Clockwork can identify and read a supported game file; it
does not mean the original game can run in Clockwork. Extraction and playable results depend
on game-specific plugins and recipes. No games, ROMs, BIOS files or extracted game assets are
included in the release.

The initial downloadable build targets **Windows x64**. Source builds on other platforms are
not covered by this binary release. You need a graphics device and driver compatible with
Bevy/wgpu; minimum GPU and RAM requirements have not been benchmarked. Large disc images are
loaded into memory, so available RAM limits which files you can import.

### Install on Windows

1. Install the [Microsoft Visual C++ v14 x64 Redistributable](https://aka.ms/vc14/vc_redist.x64.exe)
   if it is not already present. The binaries require `VCRUNTIME140.dll`; Microsoft's
   [supported runtime downloads](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
   explain the runtime requirement.
2. Open [Releases](https://github.com/Bevy-Clockwork/Clockwork/releases) and choose the preview release. Download
   `clockwork-0.1.0-windows-x86_64.zip` and `SHA256SUMS.txt` from the same release.
3. In PowerShell, run `Get-FileHash .\clockwork-0.1.0-windows-x86_64.zip -Algorithm SHA256`
   and compare the hash with the ZIP's entry in `SHA256SUMS.txt`.
4. Extract the ZIP into a writable folder, then run `clockwork.exe` from the extracted folder.
5. Import a game you own with **File → Import games**. Configure personal gameplay bindings
   under **Settings → Controls**. Set up an MCP client if you want agent-assisted extraction.

The preview binaries are unsigned, so Windows may display an unknown-publisher warning.
The ZIP also contains `clockwork-mcp.exe`, an `.mcp.json` configuration for the packaged
server, both Clockwork licenses, third-party license notices, this README and the changelog.
Rust is not required to run the packaged binaries; the Microsoft C++ runtime and a compatible
graphics driver are required.
External extraction tools and an AI/MCP client are separate installations.

To update, extract the new release into a new folder. Your Library and preferences live in
the user data folder described below; keep a backup of that folder and your content folders
when trying preview versions. There is no automatic updater.

See [CHANGELOG.md](CHANGELOG.md) for release changes and [docs/RELEASING.md](docs/RELEASING.md)
for the maintainer's build and release checklist.

## Layout

| Path | Crate | Role |
|---|---|---|
| `core/` | `clockwork_core` | Shared vocabulary: manifests, surfaces, units, platform trait |
| `runtime/` | `clockwork_runtime` | Generic Bevy systems configured by plugins |
| `platforms/gba/` | `clockwork_gba` | Game Boy Advance module |
| `platforms/n64/` | `clockwork_n64` | Nintendo 64 module |
| `platforms/snes/` | `clockwork_snes` | Super Nintendo cartridge identification and copier-header normalization |
| `platforms/nds/` | `clockwork_nds` | Nintendo DS cartridge identification |
| `platforms/playstation/` | `clockwork_playstation` | PlayStation and PlayStation 2 executable/disc identification |
| `platforms/pc/` | `clockwork_pc` | Windows PE and Linux x86/x64 ELF executable identification |
| `platforms/all/` | `clockwork_platforms` | The list of every platform module, shared by the Library, editor and agent |
| `library/` | `clockwork_library` | ROM import, hashing, game names, box art, saved Library |
| `recipe/` | `clockwork_recipe` | Loads plugins and runs their recipes |
| `agent/` | `clockwork_agent` | MCP server for the extraction agent (`clockwork-mcp` binary) |
| `editor/` | `clockwork_editor` | The editor (`clockwork` binary) |

Clockwork contains nothing specific to a game. Plugins and game knowledge live in **content
folders** outside this repository, each with `plugins/` and `knowledge/`: Clockwork's data folder,
plus folders added in Settings → Plugins & Agents (saved in `<data>/content.ron`) or listed in
`CLOCKWORK_CONTENT`.

## Supported game imports

Use **File → Import games**, or drop files into the Library. Formats are detected from their
contents; renaming an unsupported file does not make it compatible.

| Platform | Accepted input |
|---|---|
| GBA | `.gba` |
| N64 | `.z64`, `.n64`, `.v64`; byte order normalized to big-endian |
| SNES | `.sfc`, `.smc`; linear LoROM, HiROM and ExHiROM, with or without a 512-byte copier header |
| Nintendo DS | `.nds`; valid DS/DS-compatible cartridge headers |
| PS1 | `.iso` (2048-byte sectors), `.bin` (2352-byte sectors), `.cue` referencing a binary data track, PS-X EXE files (`.exe`, `.ps-exe`) |
| PS2 | `.iso`, `.bin`, `.cue`; ISO9660 discs containing `SYSTEM.CNF` and a PS2 boot executable |
| PC | Windows PE32/PE32+ `.exe` (x86, x64, ARM64); Linux x86/x64 ELF executables (`.elf` or extensionless via All files/drag and drop) |

ZIP imports choose the first recognized game image or executable with a listed extension,
skipping unrelated files. Extract CUE/BIN sets before importing the CUE. CUE loading reads the
referenced data file from `INDEX 01`; separate audio-track files are not loaded. Compressed disc
formats such as CHD/CSO/PBP, interleaved SNES dumps, DSi-only games, standalone PS2 ELF files,
and PC installation folders are not supported. PC import reads the selected executable only;
it does not collect adjacent assets. DLLs and ELF shared libraries are excluded; Linux static
PIE executables without an interpreter are not recognized.

These additions provide Library import, hashing, metadata and byte access for agent tools and
recipes. They do not emulate or launch the original games, and game-specific extraction still
needs recipes and the appropriate decoders. SNES recipes see bytes without the copier header.
PS1/PS2 recipes see the original ISO or raw-sector layout, preserving XA data; different disc
layouts can have different hashes. DS and PC files remain unchanged. Disc images are currently
read into memory, so large PS2 images require enough available RAM.

## Gameplay controls

Open **Settings → Controls** to rebind gameplay keys and controller buttons. Click a binding,
then press the replacement key, button or trigger. Escape or a mouse click cancels. If the
input is already assigned, choose **Keep shared** or **Replace other bindings**. Clear a
binding with **×**, reset one action, or restore all gameplay bindings to the game's defaults.

Xbox, PS5/PlayStation and generic controllers use Bevy's gamepad backend and can connect or
disconnect while the editor is running. The left stick moves, the right stick controls the
camera during Play, and the D-pad follows your direction bindings. **Button labels** selects
automatic, Xbox, PlayStation or generic prompts; **Stick deadzone** adjusts drift filtering.
The camera uses the Workshop's orbit speed and vertical-inversion preferences.

Changes apply immediately and persist in `<data>/settings.ron` as personal overrides over each
game's input map. They also apply in the exported player on this machine. Original plugin
bindings remain intact. The **Keyboard** settings category still lists editor shortcuts;
only gameplay controls are rebindable. An assigned gameplay key takes precedence over the
same unmodified editor shortcut during Play. Controller availability depends on OS drivers
and device mappings; rumble, adaptive triggers and touchpad gestures are not implemented.

## Build

Install Rust with rustup and the native build prerequisites for your platform. The repository
pins Rust **1.98.1** in `rust-toolchain.toml`.
On Windows, use the MSVC Rust target and Visual Studio's C++ build tools with a Windows SDK.
Run these commands from the repository root; the checked-in lockfile pins dependencies.

```bash
cargo test --workspace --locked
```

```bash
cargo run --locked -p clockwork_editor
```

For an optimized build of both applications:

```bash
cargo build --release --locked -p clockwork_editor -p clockwork_agent
```

Windows maintainers can build the distributable ZIP with `pwsh ./scripts/package-release.ps1`
after installing the license-report tool described in [docs/RELEASING.md](docs/RELEASING.md).
See [CONTRIBUTING.md](CONTRIBUTING.md) for development checks.

By default the editor looks games up online: the No-Intro database (from libretro-database) gives
each cartridge game's full name, Redump covers PS1/PS2, and libretro-thumbnails supplies box art.
PC executables use the filename as their title and have no automatic box-art lookup. Only the
game's hash and game code are used; ROMs never leave your machine. Turn this off under
**File → Fetch box art automatically**.

The editor stores the Library and cover art in your user data folder (on Windows,
`%APPDATA%\Clockwork\data`). Set `CLOCKWORK_DATA_DIR` to use a different folder, e.g. for testing.

## Extraction agent (MCP)

`clockwork-mcp` gives any MCP client (Claude Code, Claude Desktop, …) the tools to extract content
into plugins: read the Library and ROMs, search a game's linked decomp, run recipes with image
previews, and save plugins. Opening this folder in Claude Code offers the `clockwork` server from
[.mcp.json](.mcp.json). Elsewhere, point your client at:

```bash
cargo run --locked -q -p clockwork_agent --bin clockwork-mcp
```

With the Windows release ZIP, configure your client's stdio server command as the absolute
path to the extracted `clockwork-mcp.exe`, with no arguments. It uses the same user data
folder as the editor. The included `.mcp.json` uses `./clockwork-mcp.exe` for clients opened
in the extracted folder. The packaged server can also use HTTP:

```powershell
.\clockwork-mcp.exe --http 127.0.0.1:3128
```

Clients whose stdio spawning is unreliable (or that reconnect often) can instead use streamable HTTP:

```bash
cargo run --locked -q -p clockwork_agent --bin clockwork-mcp -- --http 127.0.0.1:3128
```

Then configure the client with the URL `http://127.0.0.1:3128/mcp`; the server keeps running across client restarts.

Then ask, for example: *"Extract the first town of this game as a plugin."*

Folders in Clockwork's data folder:

| Folder | Contents |
|---|---|
| `tools/` | External programs, one folder per tool (`tools/ghidra/`, …). Found before PATH. `CLOCKWORK_TOOLS_DIR` overrides it. |
| `decomps/` | Shallow clones of linked decomp repositories, made by `decomp_sync` |
| `plugins/`, `knowledge/` | Plugins and game notes; `knowledge/decomps.ron` lists known decomps by ROM hash (the data folder is always a content folder) |
| `content.ron` | Other content folders |
| `databases/` | Cached No-Intro databases |

Clockwork never downloads games. Use your own legally obtained copies.

## License and contributions

Clockwork is available under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option. Third-party dependencies retain their
own licenses; release archives include `THIRD-PARTY-NOTICES.html`.

The Clockwork license does not grant rights to game data or content extracted from a game.
Keep game files and private content folders out of issues, pull requests and release assets.
See [CONTRIBUTING.md](CONTRIBUTING.md) for contributing and reporting reproducible bugs.

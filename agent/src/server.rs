//! The MCP server: parses tool arguments and formats results. The work happens in `tools`.

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use clockwork_core::recipe::Codec;
use clockwork_recipe::edit;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
        ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tools::{decomp, knowledge, recipe, rom};
use crate::{external, guide, presence::Presence, workspace::Workspace};

type ToolResult = Result<CallToolResult, McpError>;

/// Runs slow file work (reading a ROM, hashing the Library, searching a decomp) without
/// stalling the runtime's other tasks: other sessions and their keep-alives. On a
/// single-threaded runtime, where that isn't possible, it just runs.
fn blocking<T>(work: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

fn reply(result: Result<String, String>) -> ToolResult {
    Ok(match result {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
    })
}

/// A number given as JSON number or as text ("0x324EB4", "3296948").
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Number {
    Int(u64),
    Text(String),
}

impl Number {
    pub fn value(&self) -> Result<usize, String> {
        match self {
            Number::Int(n) => Ok(*n as usize),
            Number::Text(t) => number(t),
        }
    }
}

/// Parses "0x324EB4" or "3296948".
fn number(text: &str) -> Result<usize, String> {
    let t = text.trim().replace('_', "");
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => t.parse(),
    };
    parsed.map_err(|_| format!("\"{text}\" is not a number (use 0x… for hex)"))
}

#[derive(Deserialize, JsonSchema)]
pub struct GameArg {
    /// The game: its SHA-1 (or a prefix) or part of its name, from list_games.
    pub game: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadRomArgs {
    /// The game: SHA-1 (or prefix) or part of its name.
    pub game: String,
    /// Game-file offset, e.g. "0x324EB4". N64 is big-endian; SNES has no copier header.
    /// PlayStation discs retain ISO/raw sector layout; PC offsets are file offsets.
    pub offset: Number,
    /// Bytes to read, at most 4096. Default 256.
    pub length: Option<Number>,
    /// "hex" (default), "u16" or "u32" (little-endian values).
    pub view: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchRomArgs {
    pub game: String,
    /// Bytes to find, as hex, e.g. "80 37 B3 08" (a little-endian pointer).
    pub hex: String,
    /// Only report offsets divisible by this (4 for pointers). Default 1.
    pub align: Option<usize>,
    /// Most results to return. Default 50.
    pub max_results: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DecompressArgs {
    pub game: String,
    /// Offset of the compressed data, e.g. "0x324EB4".
    pub offset: Number,
    /// "GbaLz77" (default), "Popuri", or "Mio0".
    pub codec: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DecompListArgs {
    pub game: String,
    /// Directory inside the decomp; default is its root.
    pub path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DecompSearchArgs {
    pub game: String,
    /// Regular expression (case-insensitive), e.g. "gAreaTileSet_MinishWoods".
    pub pattern: String,
    /// Only search files whose path contains this, e.g. "data/map" or ".json".
    pub path_filter: Option<String>,
    /// Most matches to return. Default 60.
    pub max_results: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DecompReadArgs {
    pub game: String,
    /// File inside the decomp, e.g. "data/map/tileset_headers.s".
    pub path: String,
    /// First line (1-based). Default 1.
    pub start_line: Option<usize>,
    /// Lines to return. Default 200, at most 1000.
    pub lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FileArg {
    /// Path relative to the knowledge folder, from list_knowledge.
    pub file: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct NameArg {
    /// Plugin name, from list_plugins.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunRecipeArgs {
    pub game: String,
    /// The whole recipe.ron text (see recipe_guide).
    pub recipe: String,
    /// Return image previews. Default true.
    pub previews: Option<bool>,
    /// Show a surface map on the image it covers instead of the plain previews.
    pub surfaces: Option<SurfacesArg>,
    /// Show one rectangle of an output image enlarged (combines with `surfaces`).
    pub zoom: Option<ZoomArg>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SurfacesArg {
    /// Surface output id; default the first.
    pub id: Option<String>,
    /// Grid to highlight codes of, e.g. "collision". Without grid and codes: the full property
    /// overlay.
    pub grid: Option<String>,
    /// Codes to highlight in magenta, e.g. ["0x5F", 48].
    pub codes: Option<Vec<Number>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ZoomArg {
    /// Output image id, e.g. "area".
    pub image: String,
    /// Left edge in pixels.
    pub x: Number,
    /// Top edge in pixels.
    pub y: Number,
    /// Width in pixels.
    pub w: Number,
    /// Height in pixels.
    pub h: Number,
}

#[derive(Deserialize, JsonSchema)]
pub struct SavePluginArgs {
    /// Plugin name: lowercase, digits, '-' and '_', e.g. "my-game-market-town".
    pub name: String,
    /// The game the recipe runs on; its ROM's SHA-1 goes into the manifest.
    pub game: String,
    /// Short id for the game used across plugins, e.g. "my_game".
    pub game_id: String,
    /// The recipe.ron text. It is run first and only saved if it works.
    pub recipe: String,
    /// Default "0.1.0".
    pub version: Option<String>,
    /// Replace an existing plugin with this name. Default false.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EditPluginArgs {
    /// Plugin name, from list_plugins.
    pub name: String,
    /// The changes, made in order: each sees the recipe the ones before it left (so after
    /// removing step 3, the step that was 4 is step 3).
    pub edits: Vec<EditArg>,
    /// Only run the changed recipe and report; save nothing. Default false.
    pub dry_run: Option<bool>,
}

/// One change. Items are written "step 3" or "output 0" (counted from 0); values, steps and
/// outputs in RON as in recipe.ron.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditArg {
    /// Set a field, e.g. value "0x40", "Some(16)" or "\"tiles\"". A field left out is added.
    Set {
        item: String,
        field: String,
        value: String,
    },
    /// Put a step at a place among the steps (last without `at`).
    InsertStep {
        at: Option<usize>,
        step: String,
    },
    /// Put an output at a place among the outputs (last without `at`).
    InsertOutput {
        at: Option<usize>,
        output: String,
    },
    /// Write a whole step or output anew, keeping its place and note.
    Replace {
        item: String,
        with: String,
    },
    Remove {
        item: String,
    },
    /// Move a step or output to another place in its list.
    Move {
        item: String,
        to: usize,
    },
    /// The comment lines right above a step or output; "" removes them.
    Note {
        item: String,
        note: String,
    },
}

impl EditArg {
    fn edit(self) -> Result<edit::Edit, String> {
        Ok(match self {
            EditArg::Set { item, field, value } => edit::Edit::Set {
                item: item.parse()?,
                field,
                value,
            },
            EditArg::InsertStep { at, step } => edit::Edit::InsertStep { at, step },
            EditArg::InsertOutput { at, output } => edit::Edit::InsertOutput { at, output },
            EditArg::Replace { item, with } => edit::Edit::Replace {
                item: item.parse()?,
                with,
            },
            EditArg::Remove { item } => edit::Edit::Remove {
                item: item.parse()?,
            },
            EditArg::Move { item, to } => edit::Edit::Move {
                item: item.parse()?,
                to,
            },
            EditArg::Note { item, note } => edit::Edit::Note {
                item: item.parse()?,
                note,
            },
        })
    }
}

#[derive(Clone)]
pub struct Clockwork {
    workspace: Arc<Mutex<Workspace>>,
    tool_router: ToolRouter<Self>,
    /// Tells the editor this server is running and who uses it.
    presence: Option<Presence>,
}

impl Default for Clockwork {
    fn default() -> Self {
        Self::new()
    }
}

impl Clockwork {
    pub fn new() -> Self {
        Self::with_workspace(Self::load_workspace())
    }

    /// Loads the Library once, to share between servers (one per HTTP session).
    pub fn load_workspace() -> Arc<Mutex<Workspace>> {
        Arc::new(Mutex::new(blocking(Workspace::load)))
    }

    /// A server working on a shared Library, so new sessions don't re-read every ROM.
    pub fn with_workspace(workspace: Arc<Mutex<Workspace>>) -> Self {
        Self {
            workspace,
            tool_router: Self::tool_router(),
            presence: None,
        }
    }

    /// Reports connections and tool calls to the editor through `presence`.
    pub fn with_presence(mut self, presence: Presence) -> Self {
        self.presence = Some(presence);
        self
    }

    fn track(&self, tool: &str) {
        if let Some(presence) = &self.presence {
            presence.tool_called(tool);
        }
    }

    /// The shared Library, with what the editor changed in it since the last call.
    fn workspace(&self) -> MutexGuard<'_, Workspace> {
        let mut ws = self.workspace.lock().unwrap_or_else(|e| e.into_inner());
        ws.refresh();
        ws
    }

    /// The local clone path of a game's linked decomp.
    fn decomp_root(&self, game: &str) -> Result<PathBuf, String> {
        let ws = self.workspace();
        let entry = ws.find(game)?;
        let url = ws.decomp(&entry).ok_or_else(|| {
            format!(
                "{} has no linked decomp. The user can link one in the Library's details panel.",
                ws.title(&entry)
            )
        })?;
        Ok(decomp::local_path(&url))
    }

    fn rom_of(&self, game: &str) -> Result<Arc<Vec<u8>>, String> {
        blocking(|| {
            let mut ws = self.workspace();
            let entry = ws.find(game)?;
            ws.rom(&entry)
        })
    }
}

#[tool_router(router = tool_router)]
impl Clockwork {
    #[tool(
        description = "List the games in the user's Library: SHA-1, name, platform, header \
                          game code, size and linked decomp. Re-reads the Library."
    )]
    async fn list_games(&self) -> ToolResult {
        self.track("list_games");
        let mut ws = self.workspace();
        *ws = blocking(Workspace::load);
        let mut text = serde_json::to_string_pretty(&ws.games()).unwrap_or_default();
        if !ws.problems().is_empty() {
            let _ = write!(text, "\nCould not load: {}", ws.problems().join("; "));
        }
        reply(Ok(text))
    }

    #[tool(
        description = "Read bytes from a game's ROM as a hex dump, or as little-endian u16 or \
                          u32 values. At most 4096 bytes."
    )]
    async fn read_rom(&self, args: Parameters<ReadRomArgs>) -> ToolResult {
        self.track("read_rom");
        let a = args.0;
        reply((|| {
            let rom = self.rom_of(&a.game)?;
            let offset = a.offset.value()?;
            let length = a
                .length
                .as_ref()
                .map(Number::value)
                .transpose()?
                .unwrap_or(256);
            rom::read(&rom, offset, length, rom::View::parse(a.view.as_deref())?)
        })())
    }

    #[tool(
        description = "Find a byte sequence in a game's ROM, e.g. a pointer (4 little-endian \
                          bytes) to see what references some data."
    )]
    async fn search_rom(&self, args: Parameters<SearchRomArgs>) -> ToolResult {
        self.track("search_rom");
        let a = args.0;
        reply((|| {
            let rom = self.rom_of(&a.game)?;
            let needle = rom::parse_hex(&a.hex)?;
            let hits = rom::search(
                &rom,
                &needle,
                a.align.unwrap_or(1),
                a.max_results.unwrap_or(50),
            );
            Ok(if hits.is_empty() {
                "Not found.".to_string()
            } else {
                hits.iter()
                    .map(|o| format!("{o:#08X}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })())
    }

    #[tool(
        description = "Decompress data in a game's ROM and report its size and first bytes, to \
                          check that an offset really points at compressed data."
    )]
    async fn decompress(&self, args: Parameters<DecompressArgs>) -> ToolResult {
        self.track("decompress");
        let a = args.0;
        reply((|| {
            let rom = self.rom_of(&a.game)?;
            let codec = match a.codec.as_deref().unwrap_or("GbaLz77") {
                "GbaLz77" => Codec::GbaLz77,
                "Mio0" => Codec::Mio0,
                "Popuri" => Codec::Popuri,
                "None" => Codec::None,
                other => {
                    return Err(format!(
                        "Unknown codec \"{other}\"; use GbaLz77, Popuri or Mio0"
                    ));
                }
            };
            rom::decompress(&rom, a.offset.value()?, codec)
        })())
    }

    #[tool(
        description = "Show which external programs (git, Ghidra, …) are available and where \
                          they were found. They live in Clockwork's tools folder, one folder \
                          per tool, or on PATH."
    )]
    async fn list_external_tools(&self) -> ToolResult {
        self.track("list_external_tools");
        let mut text = format!("Tools folder: {}\n", external::tools_dir().display());
        for known in external::KNOWN {
            let found = external::find(known.name)
                .map_or("not found".to_string(), |p| p.display().to_string());
            let _ = writeln!(text, "- {}: {found}\n  {}", known.name, known.purpose);
        }
        reply(Ok(text))
    }

    #[tool(
        description = "Clone (shallow) or update the game's linked decompilation repository, \
                          so decomp_list/search/read can use it. Run once per game."
    )]
    async fn decomp_sync(&self, args: Parameters<GameArg>) -> ToolResult {
        self.track("decomp_sync");
        let url = {
            let ws = self.workspace();
            match ws.find(&args.0.game) {
                Ok(entry) => ws.decomp(&entry),
                Err(e) => return reply(Err(e)),
            }
        };
        let Some(url) = url else {
            return reply(Err(
                "This game has no linked decomp. The user can link one in the Library.".into(),
            ));
        };
        let result = tokio::task::spawn_blocking(move || decomp::sync(&url))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        reply(result)
    }

    #[tool(description = "List a directory of the game's decomp clone.")]
    async fn decomp_list(&self, args: Parameters<DecompListArgs>) -> ToolResult {
        self.track("decomp_list");
        let a = args.0;
        reply(
            self.decomp_root(&a.game)
                .and_then(|root| decomp::list(&root, a.path.as_deref().unwrap_or(""))),
        )
    }

    #[tool(
        description = "Search the game's decomp clone with a regular expression \
                          (case-insensitive). Returns path:line: text."
    )]
    async fn decomp_search(&self, args: Parameters<DecompSearchArgs>) -> ToolResult {
        self.track("decomp_search");
        let a = args.0;
        reply(self.decomp_root(&a.game).and_then(|root| {
            blocking(|| {
                decomp::search(
                    &root,
                    &a.pattern,
                    a.path_filter.as_deref().unwrap_or(""),
                    a.max_results.unwrap_or(60),
                )
            })
        }))
    }

    #[tool(description = "Read lines of a file in the game's decomp clone.")]
    async fn decomp_read(&self, args: Parameters<DecompReadArgs>) -> ToolResult {
        self.track("decomp_read");
        let a = args.0;
        reply(self.decomp_root(&a.game).and_then(|root| {
            decomp::read(
                &root,
                &a.path,
                a.start_line.unwrap_or(1),
                a.lines.unwrap_or(200).min(1000),
            )
        }))
    }

    #[tool(
        description = "The recipe format: every step and output, surface properties, and \
                          habits that work. Read this before writing a recipe."
    )]
    async fn recipe_guide(&self) -> ToolResult {
        self.track("recipe_guide");
        reply(Ok(guide::RECIPE_GUIDE.to_string()))
    }

    #[tool(description = "List knowledge files: notes about specific games and known decomps.")]
    async fn list_knowledge(&self) -> ToolResult {
        self.track("list_knowledge");
        reply(knowledge::list().map(|files| files.join("\n")))
    }

    #[tool(description = "Read a knowledge file.")]
    async fn read_knowledge(&self, args: Parameters<FileArg>) -> ToolResult {
        self.track("read_knowledge");
        reply(knowledge::read(&args.0.file))
    }

    #[tool(description = "List installed plugins with their game, ROM hashes and outputs.")]
    async fn list_plugins(&self) -> ToolResult {
        self.track("list_plugins");
        let mut text = String::new();
        for plugin in recipe::plugins() {
            match plugin {
                Ok(p) => {
                    let _ = writeln!(
                        text,
                        "{} (game {}, {} steps, {} outputs) at {}",
                        p.manifest.plugin.name,
                        p.manifest.source.game,
                        p.recipe.steps.len(),
                        p.recipe.outputs.len(),
                        p.dir.display()
                    );
                }
                Err(e) => {
                    let _ = writeln!(text, "(broken plugin) {e}");
                }
            }
        }
        reply(Ok(if text.is_empty() {
            "No plugins.".into()
        } else {
            text
        }))
    }

    #[tool(
        description = "Read a plugin's clockwork.toml and recipe.ron, e.g. to learn from an \
                          existing extraction."
    )]
    async fn read_plugin(&self, args: Parameters<NameArg>) -> ToolResult {
        self.track("read_plugin");
        reply(recipe::find_plugin(&args.0.name).and_then(|p| {
            let read = |f: &str| std::fs::read_to_string(p.dir.join(f)).map_err(|e| e.to_string());
            Ok(format!(
                "# clockwork.toml\n{}\n# recipe.ron\n{}",
                read("clockwork.toml")?,
                read("recipe.ron")?
            ))
        }))
    }

    #[tool(
        description = "Run a recipe against a game's ROM without saving it. Returns a summary \
                          of every output, and previews of the images so you can check them."
    )]
    async fn run_recipe(&self, args: Parameters<RunRecipeArgs>) -> ToolResult {
        self.track("run_recipe");
        let a = args.0;
        let result = (|| {
            let recipe = recipe::parse(&a.recipe)?;
            let rom = self.rom_of(&a.game)?;
            recipe::run(&recipe, &rom)
        })();
        let extracted = match result {
            Ok(x) => x,
            Err(e) => return reply(Err(e)),
        };
        let mut content = vec![ContentBlock::text(recipe::summary(&extracted))];
        if !a.previews.unwrap_or(true) {
            return Ok(CallToolResult::success(content));
        }
        let views = (|| {
            let codes = a
                .surfaces
                .as_ref()
                .and_then(|s| s.codes.as_ref())
                .map(|codes| codes.iter().map(|c| c.value().map(|v| v as u16)).collect())
                .transpose()?
                .unwrap_or_default();
            let surfaces = a.surfaces.as_ref().map(|s| recipe::SurfaceView {
                id: s.id.as_deref(),
                grid: s.grid.as_deref(),
                codes,
            });
            let zoom = match &a.zoom {
                Some(z) => Some(recipe::Zoom {
                    image: &z.image,
                    rect: (
                        z.x.value()? as u32,
                        z.y.value()? as u32,
                        z.w.value()? as u32,
                        z.h.value()? as u32,
                    ),
                }),
                None => None,
            };
            recipe::previews(&extracted, surfaces, zoom)
        })();
        match views {
            Ok(views) => {
                for (label, png) in views {
                    content.push(ContentBlock::text(format!("{label}:")));
                    content.push(ContentBlock::image(png, "image/png"));
                }
            }
            Err(e) => content.push(ContentBlock::text(format!("No preview: {e}"))),
        }
        Ok(CallToolResult::success(content))
    }

    #[tool(
        description = "Change a saved plugin's recipe.ron a step at a time: set values, insert, \
                          replace, move or remove steps and outputs, write notes. Its comments \
                          and layout stay. The result is run on the plugin's game and saved only \
                          if it works (or not at all with dry_run)."
    )]
    async fn edit_plugin(&self, args: Parameters<EditPluginArgs>) -> ToolResult {
        self.track("edit_plugin");
        let a = args.0;
        reply((|| {
            let plugin = recipe::find_plugin(&a.name)?;
            let path = plugin.dir.join("recipe.ron");
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let edits = a
                .edits
                .into_iter()
                .map(EditArg::edit)
                .collect::<Result<Vec<_>, _>>()?;
            let (changed, parsed) = edit::apply_all(&text, &edits)?;
            let game = plugin
                .manifest
                .source
                .sha1
                .iter()
                .find(|sha1| self.workspace().find(sha1).is_ok())
                .ok_or("The plugin's game isn't in the Library, so the change can't be run")?
                .clone();
            let rom = self.rom_of(&game)?;
            let extracted = recipe::run(&parsed, &rom)?;
            let summary = recipe::summary(&extracted);
            if a.dry_run.unwrap_or(false) {
                return Ok(format!("Not saved (dry run). It runs:\n{summary}"));
            }
            std::fs::write(&path, &changed).map_err(|e| e.to_string())?;
            Ok(format!(
                "Saved {} ({} change(s)). The editor loads it by itself.\n{summary}",
                path.display(),
                edits.len()
            ))
        })())
    }

    #[tool(
        description = "Save a working recipe as a plugin (clockwork.toml + recipe.ron) in the \
                          plugins folder. The recipe is run first and only saved if it works. \
                          The Workshop shows it after Rescan."
    )]
    async fn save_plugin(&self, args: Parameters<SavePluginArgs>) -> ToolResult {
        self.track("save_plugin");
        let a = args.0;
        reply((|| {
            let parsed = recipe::parse(&a.recipe)?;
            let entry = self.workspace().find(&a.game)?;
            let rom = self.rom_of(&a.game)?;
            let extracted = recipe::run(&parsed, &rom)?;
            let dir = recipe::save(
                &a.name,
                &a.game_id,
                a.version.as_deref().unwrap_or("0.1.0"),
                &entry,
                &a.recipe,
                a.overwrite.unwrap_or(false),
            )?;
            // What it provides, leaving out kinds it has none of.
            let counts = [
                (extracted.images.len(), "images"),
                (extracted.surfaces.len(), "surface maps"),
                (extracted.models.len(), "models"),
                (extracted.sprites.len(), "sprites"),
                (extracted.items.len(), "item lists"),
                (extracted.abilities.len(), "abilities"),
                (extracted.fonts.len(), "fonts"),
                (extracted.sounds.len(), "sounds"),
                (extracted.dialogs.len(), "dialogs from text"),
            ];
            let made: usize = counts.iter().map(|(n, _)| n).sum();
            let counts: Vec<String> = counts
                .into_iter()
                .chain([(parsed.outputs.len().saturating_sub(made), "other assets")])
                .filter(|(n, _)| *n > 0)
                .map(|(n, what)| format!("{n} {what}"))
                .collect();
            // What it names must exist and its games' fonts must write their words.
            let plugins: Vec<_> = recipe::plugins().into_iter().flatten().collect();
            let index = clockwork_recipe::assets::AssetIndex::build(&plugins);
            let problems = clockwork_recipe::check::check(&index, Some(&a.name));
            let problems = if problems.is_empty() {
                String::new()
            } else {
                let lines: Vec<String> = problems.iter().map(|p| format!("\n- {p}")).collect();
                format!("\nProblems to fix:{}", lines.concat())
            };
            Ok(format!(
                "Saved {} ({}). The user can load it in the Workshop.{problems}",
                dir.display(),
                if counts.is_empty() {
                    "no outputs".to_string()
                } else {
                    counts.join(", ")
                }
            ))
        })())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Clockwork {
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if let Some(presence) = &self.presence {
            let client = &request.client_info;
            let name = client.title.as_deref().unwrap_or(&client.name);
            presence.set_client(format!("{name} {}", client.version));
        }
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("clockwork", env!("CARGO_PKG_VERSION")))
            .with_instructions(guide::INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numbers() {
        assert_eq!(number("0x324EB4"), Ok(0x324EB4));
        assert_eq!(number("4096"), Ok(4096));
        assert!(number("zz").is_err());
        let args: ReadRomArgs =
            serde_json::from_str(r#"{"game": "x", "offset": "0x10", "length": 64}"#).unwrap();
        assert_eq!(args.offset.value(), Ok(16));
        assert_eq!(args.length.unwrap().value(), Ok(64));
        assert_eq!(number(" 0X1_000 "), Ok(0x1000));
        assert!(number("").is_err());
        assert!(number("-1").is_err());
    }

    #[test]
    fn edits_are_read_from_json() {
        let args: EditPluginArgs = serde_json::from_str(
            r#"{"name": "tmc", "edits": [
                {"op": "set", "item": "step 3", "field": "offset", "value": "0x40"},
                {"op": "insert_step", "step": "Fill(name: \"pad\", size: 4)"},
                {"op": "move", "item": "output 1", "to": 0},
                {"op": "note", "item": "step 0", "note": "From the gfx table"}
            ]}"#,
        )
        .unwrap();
        let edits: Vec<edit::Edit> = args.edits.into_iter().map(|e| e.edit().unwrap()).collect();
        assert_eq!(
            edits[0],
            edit::Edit::Set {
                item: edit::Item::Step(3),
                field: "offset".into(),
                value: "0x40".into(),
            }
        );
        assert_eq!(
            edits[2],
            edit::Edit::Move {
                item: edit::Item::Output(1),
                to: 0,
            }
        );
        let bad = EditArg::Remove {
            item: "stage 2".into(),
        };
        assert!(bad.edit().is_err());
    }

    #[test]
    fn blocking_work_runs_on_any_runtime() {
        assert_eq!(blocking(|| 1), 1);
        let single = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(single.block_on(async { blocking(|| 2) }), 2);
        let multi = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        assert_eq!(multi.block_on(async { blocking(|| 3) }), 3);
    }
}

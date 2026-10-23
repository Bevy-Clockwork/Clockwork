//! Player mode: this program playing an exported game (see [`crate::export`]) instead of being
//! the Workshop. It shows the game full window, asks for the player's own copy of each game it
//! needs the first time, and starts playing by itself.

use std::path::{Path, PathBuf};

use bevy_egui::egui::{self, Color32, RichText, vec2};

use crate::{
    export::{GAME_DIR, GameInfo, INFO_FILE, SCENE_FILE},
    state::EditorState,
    theme,
};

/// An exported game being played.
pub struct PlayerMode {
    pub info: GameInfo,
    frames: u32,
    attempts: u32,
}

impl PlayerMode {
    pub fn new(info: GameInfo) -> Self {
        Self {
            info,
            frames: 0,
            attempts: 0,
        }
    }
}

/// The game folder to play: `--play <folder>`, or a `game/` folder next to a program that isn't
/// the Workshop itself (the exported player).
pub fn detect() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--play" {
            return args.next().map(PathBuf::from);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_string_lossy().to_lowercase();
    let next_to = exe.parent()?.join(GAME_DIR);
    (name == "player" && next_to.join(SCENE_FILE).exists()).then_some(next_to)
}

/// Reads the game's info and points Clockwork at the game's own plugins and a data folder of its
/// own (where the player's ROM locations are remembered). Must run before anything reads the
/// data folder.
pub fn prepare(dir: &Path) -> Result<GameInfo, String> {
    let text = std::fs::read_to_string(dir.join(INFO_FILE))
        .map_err(|e| format!("{}: {e}", dir.join(INFO_FILE).display()))?;
    let info: GameInfo = ron::from_str(&text).map_err(|e| format!("{INFO_FILE}: {e}"))?;
    let data = directories::ProjectDirs::from("", "", "Clockwork Player")
        .map(|d| d.data_dir().join(crate::export::folder_name(&info.title)))
        .unwrap_or_else(|| dir.join("player-data"));
    // SAFETY: this runs at the start of `main`, before any other thread exists.
    unsafe {
        std::env::set_var("CLOCKWORK_CONTENT", dir);
        std::env::set_var("CLOCKWORK_DATA_DIR", data);
    }
    Ok(info)
}

/// Draws the player's screen. Returns the rect for the game (none while it waits for ROMs), and
/// whether the player asked to locate a ROM.
pub fn screen(
    ctx: &egui::Context,
    root: &mut egui::Ui,
    state: &mut EditorState,
) -> (Option<egui::Rect>, bool) {
    // Which plugins of the game have no copy of their game yet.
    let missing: Vec<String> = state
        .player
        .as_ref()
        .map(|p| {
            let needed: Vec<_> = p
                .info
                .plugins
                .iter()
                .filter(|required| {
                    !state
                        .library
                        .entries()
                        .iter()
                        .any(|e| required.sha1.contains(&e.sha1))
                })
                .cloned()
                .collect();
            crate::export::games(&needed)
        })
        .unwrap_or_default();
    let title = state
        .player
        .as_ref()
        .map(|p| p.info.title.clone())
        .unwrap_or_default();

    if !missing.is_empty() {
        let mut import = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG_DEEP).inner_margin(32))
            .show(root, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(
                        RichText::new(&title)
                            .font(theme::display(34.0))
                            .color(theme::BRASS_BRIGHT),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "This game is built from games you own. Locate your own copy of:",
                        )
                        .color(theme::PARCHMENT),
                    );
                    ui.add_space(10.0);
                    for game in &missing {
                        ui.label(RichText::new(game).color(theme::BRASS));
                    }
                    ui.add_space(18.0);
                    if ui
                        .add_sized(vec2(200.0, 36.0), egui::Button::new("Locate ROM…"))
                        .clicked()
                    {
                        import = true;
                    }
                    ui.add_space(8.0);
                    ui.small(
                        RichText::new(
                            "Or drop the file on this window. It stays on your computer.",
                        )
                        .color(theme::PARCHMENT_DIM),
                    );
                });
            });
        return (None, import);
    }

    // Everything is there: play. Starting needs the plugins extracted, which happens while the
    // scene is built, so ask again every half second until it works.
    let playing = state.workshop.playing.is_some();
    if let Some(player) = &mut state.player {
        player.frames += 1;
        if !playing && player.frames > 20 && player.frames % 30 == 0 && player.attempts < 20 {
            player.attempts += 1;
            state.workshop.play_request = Some(true);
        }
        if !playing && player.attempts >= 20 {
            egui::Area::new(egui::Id::new("player_error"))
                .fixed_pos(ctx.content_rect().center())
                .pivot(egui::Align2::CENTER_CENTER)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("Nothing in this game can be played.")
                            .color(Color32::from_rgb(0xE0, 0x80, 0x70)),
                    );
                });
        }
    }
    // The game's HUD is drawn in the game's view (`clockwork_runtime::hud`).
    (Some(ctx.content_rect()), false)
}

//! What a played character's settings resolve to: its camera, HUD, UI theme, menus and input
//! map, each from the strongest layer that has one (the played node's override, the game's, the
//! character's own, the built-in one). A name that leads nowhere is logged and the next layer
//! used.

use std::sync::Arc;

use clockwork_core::{
    asset::{self, Layer, OutputRef, Slot},
    input::{self, InputMap},
    recipe::Output,
    ui::{self, HudLayout, Kit, Menu, Theme},
};
use clockwork_recipe::assets::AssetIndex;
use clockwork_runtime::hud::Screen;

use crate::{
    scene::NodeId,
    state::{EditorState, LogLevel},
    workshop::Extractions,
};

/// One setting of a played node: `over` is the node's override, `game` the game's, `own` the
/// character's, `builtin` the default. `what` names it in the log ("camera", "HUD").
#[allow(clippy::too_many_arguments)]
pub fn setting<T: Clone>(
    state: &mut EditorState,
    node: NodeId,
    what: &str,
    over: Option<OutputRef>,
    game: Option<OutputRef>,
    own: Option<&Slot<T>>,
    builtin: Option<OutputRef>,
    expected: &'static str,
    take: fn(&Output) -> Option<Slot<T>>,
) -> Option<T> {
    let over = over.map(Slot::Ref);
    let game = game.map(Slot::Ref);
    let fallback = builtin.map(Slot::Ref);
    let mut layers = vec![
        (Layer::Node, over.as_ref()),
        (Layer::Game, game.as_ref()),
        (Layer::Asset, own),
        (Layer::Default, fallback.as_ref()),
    ];
    let mut problems = Vec::new();
    let found = {
        let index = AssetIndex::build(&state.workshop.plugins);
        loop {
            match asset::resolve(&layers, &index, expected, take) {
                Ok(found) => break found.map(|r| r.value),
                Err(error) => {
                    problems.push(error.to_string());
                    // Skip the layer that failed and try the next.
                    match layers.iter_mut().find(|(_, s)| s.is_some()) {
                        Some(layer) => layer.1 = None,
                        None => break None,
                    }
                }
            }
        }
    };
    let name = state
        .workshop
        .scene
        .get(node)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    for problem in problems {
        state.log(LogLevel::Warn, format!("{name}'s {what}: {problem}"));
    }
    found
}

/// The game's own settings, a preset's filling in what it leaves open.
pub fn game(state: &mut EditorState) -> crate::scene::Game {
    let mut game = state.workshop.scene.game.clone();
    let Some(preset) = game.preset.clone() else {
        return game;
    };
    let found = {
        let index = AssetIndex::build(&state.workshop.plugins);
        asset::resolve(
            &[(Layer::Game, Some(&Slot::Ref(preset.clone())))],
            &index,
            "game",
            asset::game_of,
        )
    };
    let settings = match found {
        Ok(Some(found)) => found.value,
        Ok(None) => return game,
        Err(error) => {
            state.log(LogLevel::Warn, format!("The game's preset: {error}"));
            return game;
        }
    };
    let name = |text: &Option<String>| text.as_deref().and_then(OutputRef::parse);
    game.rules = game.rules.or(name(&settings.rules));
    game.flow = game.flow.or(name(&settings.flow));
    game.camera = game.camera.or(name(&settings.camera));
    game.hud = game.hud.or(name(&settings.hud));
    game.ui = game.ui.or(name(&settings.ui));
    game.input = game.input.or(name(&settings.input));
    game.stats = game.stats.or(name(&settings.stats));
    game.inventory = game.inventory.or(name(&settings.inventory));
    game.menus = game.menus.or_else(|| {
        settings
            .menus
            .map(|m| m.iter().filter_map(|t| OutputRef::parse(t)).collect())
    });
    game
}

/// The played game's flow, from the scene or its preset; absent when neither names one.
pub fn flow(state: &mut EditorState) -> Option<clockwork_core::flow::Flow> {
    let game = game(state);
    setting(
        state,
        crate::scene::ROOT,
        "flow",
        None,
        game.flow,
        None,
        None,
        "flow",
        asset::flow_of,
    )
}

/// The screen and input map of a played node whose character has `kit`.
pub fn screen(
    state: &mut EditorState,
    extractions: &Extractions,
    node: NodeId,
    kit: &Kit,
) -> (Screen, InputMap) {
    let overrides = state.workshop.scene.get(node).cloned();
    let over =
        |pick: fn(&crate::scene::Node) -> Option<OutputRef>| overrides.as_ref().and_then(pick);
    let game = game(state);
    let layout = setting(
        state,
        node,
        "HUD",
        over(|n| n.hud.clone()),
        game.hud.clone(),
        kit.hud.as_ref(),
        Some(ui::builtin(ui::HUD)),
        "HUD",
        asset::hud_of,
    )
    .unwrap_or_else(ui::default_hud);
    let theme = setting(
        state,
        node,
        "UI",
        over(|n| n.ui.clone()),
        game.ui.clone(),
        kit.ui.as_ref(),
        Some(ui::builtin(ui::UI)),
        "UI theme",
        asset::ui_of,
    )
    .unwrap_or_else(ui::default_theme);
    let input = setting(
        state,
        node,
        "input map",
        over(|n| n.input.clone()),
        game.input.clone(),
        kit.input.as_ref(),
        Some(input::builtin()),
        "input map",
        asset::input_of,
    )
    .unwrap_or_else(input::default_map);

    // Menus are a list: the strongest layer that has one gives all of them.
    let named = overrides
        .as_ref()
        .and_then(|n| n.menus.clone())
        .or(game.menus.clone());
    let menus: Vec<Slot<Menu>> = match named {
        Some(names) => names.into_iter().map(Slot::Ref).collect(),
        None => kit.menus.clone().unwrap_or_else(|| {
            vec![
                Slot::Ref(ui::builtin(ui::INVENTORY)),
                Slot::Ref(ui::builtin(ui::PAUSE)),
            ]
        }),
    };
    let menus = menus
        .iter()
        .filter_map(|menu| {
            setting(
                state,
                node,
                "menu",
                None,
                None,
                Some(menu),
                None,
                "menu",
                asset::menu_of,
            )
        })
        .collect();

    let images = screen_images(&theme, &layout, extractions);
    let font = theme_font(theme.font.as_deref(), extractions);
    let dialog_font = theme_font(theme.dialog_font.as_deref(), extractions);
    (
        Screen {
            layout,
            theme,
            menus,
            images: Arc::new(images),
            font,
            dialog_font,
        },
        input,
    )
}

fn theme_font(
    name: Option<&str>,
    extractions: &Extractions,
) -> Option<Arc<clockwork_core::font::BitmapFont>> {
    let at = OutputRef::parse(name?)?;
    extractions
        .get(&at.plugin)?
        .fonts
        .iter()
        .find(|(id, _)| *id == at.output)
        .map(|(_, font)| Arc::new(font.clone()))
}

/// The pictures a screen names: its theme's frame and its widgets' icons.
fn pictures_named(theme: &Theme, layout: &HudLayout) -> Vec<String> {
    let frame = theme.frame.as_ref().map(|f| f.image.clone());
    let icons = layout.widgets.iter().filter_map(|w| w.icon.clone());
    frame.into_iter().chain(icons).collect()
}

/// The pictures a screen names, from their plugins' extractions.
fn screen_images(
    theme: &Theme,
    layout: &HudLayout,
    extractions: &Extractions,
) -> Vec<(String, clockwork_core::image::RgbaImage)> {
    pictures_named(theme, layout)
        .into_iter()
        .filter_map(|name| {
            let at = OutputRef::parse(&name)?;
            let (_, image) = extractions
                .get(&at.plugin)?
                .images
                .iter()
                .find(|(id, _)| *id == at.output)?;
            Some((name, image.clone()))
        })
        .collect()
}

/// Fills in the played screen's font and frame picture once their plugins are extracted
/// (asking for the extraction the first time).
pub fn complete_screen(state: &mut EditorState, extractions: &mut Extractions) {
    let Some((screen, _)) = &state.workshop.screen else {
        return;
    };
    let theme = screen.theme.clone();
    let layout = screen.layout.clone();
    let pictures = pictures_named(&theme, &layout);
    let wants_font = theme.font.is_some() && screen.font.is_none();
    let wants_dialog_font = theme.dialog_font.is_some() && screen.dialog_font.is_none();
    let wants_pictures = screen.images.len() < pictures.len();
    if !wants_font && !wants_dialog_font && !wants_pictures {
        return;
    }
    let fonts = [theme.font.clone(), theme.dialog_font.clone()];
    let named = fonts.into_iter().flatten().chain(pictures.iter().cloned());
    for at in named.filter_map(|n| OutputRef::parse(&n)) {
        extractions.ensure(state, &at.plugin);
    }
    let font = theme_font(theme.font.as_deref(), extractions);
    let dialog_font = theme_font(theme.dialog_font.as_deref(), extractions);
    let images = screen_images(&theme, &layout, extractions);
    if let Some((screen, _)) = &mut state.workshop.screen {
        if wants_font && font.is_some() {
            screen.font = font;
        }
        if wants_dialog_font && dialog_font.is_some() {
            screen.dialog_font = dialog_font;
        }
        if wants_pictures && images.len() > screen.images.len() {
            screen.images = Arc::new(images);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> EditorState {
        EditorState {
            library: clockwork_library::Library::new(),
            missing: Vec::new(),
            selected: None,
            search: String::new(),
            agent_prompt: String::new(),
            screen: Default::default(),
            log: Vec::new(),
            fetch_box_art: false,
            names: Default::default(),
            decomps: Default::default(),
            decomp_input: String::new(),
            boxart_retry: Vec::new(),
            boxart_busy: false,
            workshop: Default::default(),
            agents: Vec::new(),
            claude_connected: false,
            player: None,
            plugin_editor: Default::default(),
            quit_asked: false,
            palette_request: None,
        }
    }

    #[test]
    fn the_scenes_flow_wins_then_the_presets_then_none() {
        let mut state = state();
        state.workshop.plugins.push(clockwork_recipe::Plugin {
            dir: Default::default(),
            manifest: clockwork_core::PluginManifest::from_toml(
                "[plugin]\nname = \"p\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n[source]\ngame = \"g\"\nsha1 = []\n",
            ).unwrap(),
            recipe: ron::from_str(r#"(steps: [], outputs: [
                Game(id: "preset", game: (flow: Some("p/alias"))),
                Game(id: "empty", game: (rules: None)),
                Flow(id: "alias", flow: "p/preset-flow"),
                Flow(id: "preset-flow", flow: (files: Some((title: (pages: ["FILES"]), slots: 2)))),
                Flow(id: "scene-flow", flow: (files: Some((title: (pages: ["FILES"]), slots: 4)))),
            ])"#).unwrap(),
            compose: None,
        });
        assert_eq!(flow(&mut state), None);
        state.workshop.scene.game.preset = Some(OutputRef::new("p", "preset"));
        assert_eq!(game(&mut state).flow, Some(OutputRef::new("p", "alias")));
        assert_eq!(flow(&mut state).unwrap().files.unwrap().slots, 2);
        state.workshop.scene.game.flow = Some(OutputRef::new("p", "scene-flow"));
        assert_eq!(
            game(&mut state).flow,
            Some(OutputRef::new("p", "scene-flow"))
        );
        assert_eq!(flow(&mut state).unwrap().files.unwrap().slots, 4);
        // The scene keeps its own choices, without baking in inherited settings.
        state.workshop.scene.game.flow = None;
        assert_eq!(flow(&mut state).unwrap().files.unwrap().slots, 2);
        assert_eq!(state.workshop.scene.game.flow, None);
        state.workshop.scene.game.preset = Some(OutputRef::new("p", "empty"));
        assert_eq!(flow(&mut state), None);
        state.workshop.scene.game.flow = Some(OutputRef::new("p", "missing"));
        assert_eq!(flow(&mut state), None);
        assert_eq!(state.log.len(), 1);
    }

    #[test]
    fn the_flow_pick_and_scene_setting_round_trip() {
        let mut game: crate::scene::Game = ron::from_str("()").unwrap();
        assert_eq!(game.flow, None);
        assert!(!ron::to_string(&game).unwrap().contains("flow"));
        let pick = crate::asset_ui::GAME_PICKS
            .iter()
            .find(|p| p.kind == "Flow")
            .unwrap();
        let at = Some(OutputRef::new("p", "screens"));
        (pick.set)(&mut game, at.clone());
        assert_eq!((pick.get)(&game), at);
        let text = ron::to_string(&game).unwrap();
        assert_eq!(ron::from_str::<crate::scene::Game>(&text).unwrap(), game);
    }
}

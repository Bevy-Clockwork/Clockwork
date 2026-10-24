//! Exporting a scene as a game that can be played without the Workshop.
//!
//! An export is a folder: a copy of this program (which plays the game when it finds `game/` next
//! to it), the scene, the plugins the scene uses (offsets and parameters only), and a README. It
//! never contains game data: whoever plays it points the player at their own copy of each game
//! the first time it runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use clockwork_core::asset::{BUILTIN, OutputRef};
use clockwork_recipe::assets::AssetIndex;

use crate::{
    entities::{Behavior, Effect},
    scene::Scene,
};

pub const SCENE_FILE: &str = "scene.ron";
pub const INFO_FILE: &str = "game.ron";
/// The folder an exported game keeps its files in, next to the player.
pub const GAME_DIR: &str = "game";

/// What the player needs to know about an exported game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameInfo {
    pub title: String,
    /// The version of Clockwork that made it.
    pub clockwork: String,
    pub plugins: Vec<RequiredPlugin>,
}

/// A plugin the game uses, and which game's ROM it needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredPlugin {
    pub name: String,
    pub game: String,
    pub sha1: Vec<String>,
}

/// A folder name made of the characters of `title` that are safe in one.
pub fn folder_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "Game".into()
    } else {
        cleaned
    }
}

/// The games a set of plugins needs, once each, as readable names ("first_game" is "First Game").
pub fn games(plugins: &[RequiredPlugin]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for plugin in plugins {
        let name = plugin
            .game
            .split(['_', '-'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Names of the plugins whose items the scene shows, in the order they first appear.
/// The assets an effect names: dialogs, sounds and music, also inside `All`.
fn effect_assets(effect: &Effect, wanted: &mut Vec<OutputRef>) {
    match effect {
        Effect::Say(dialog) => wanted.extend(dialog.target().cloned()),
        Effect::Sound(name) | Effect::Music(name) => wanted.extend(OutputRef::parse(name)),
        Effect::All(effects) => {
            for effect in effects {
                effect_assets(effect, wanted);
            }
        }
        Effect::Run(script) => {
            for effect in script.effects() {
                effect_assets(effect, wanted);
            }
        }
        _ => {}
    }
}

pub fn plugins_used(scene: &Scene, assets: &AssetIndex) -> Vec<String> {
    // Every asset the scene names: what its nodes show, their settings and behaviors, the
    // dialogs and sounds their triggers name, and the game's settings and preset.
    let mut wanted: Vec<OutputRef> = Vec::new();
    let mut plugins: Vec<String> = Vec::new();
    for id in scene.in_tree_order() {
        let Some(node) = scene.get(id) else {
            continue;
        };
        if let Some(item) = &node.item {
            plugins.push(item.plugin.clone());
            wanted.push(OutputRef::new(&item.plugin, &item.output));
        }
        wanted.extend(
            [
                &node.camera,
                &node.hud,
                &node.ui,
                &node.input,
                &node.stats,
                &node.inventory,
            ]
            .into_iter()
            .flatten()
            .cloned(),
        );
        wanted.extend(node.menus.iter().flatten().cloned());
        for entry in &node.behaviors {
            wanted.extend(entry.from.clone());
            if let Behavior::Trigger { effect, .. } = &entry.behavior {
                effect_assets(effect, &mut wanted);
            }
        }
    }
    let game = &scene.game;
    wanted.extend(
        [
            &game.preset,
            &game.rules,
            &game.flow,
            &game.camera,
            &game.hud,
            &game.ui,
            &game.input,
            &game.stats,
            &game.inventory,
            &game.music,
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    wanted.extend(game.menus.iter().flatten().cloned());
    // And everything those use, in turn.
    let mut seen: Vec<OutputRef> = Vec::new();
    while let Some(at) = wanted.pop() {
        if seen.contains(&at) {
            continue;
        }
        if at.plugin != BUILTIN {
            plugins.push(at.plugin.clone());
        }
        for asset in assets.all().iter().filter(|a| a.at == at) {
            wanted.extend(asset.uses.iter().map(|u| u.asset.clone()));
        }
        seen.push(at);
    }
    let mut out: Vec<String> = Vec::new();
    for plugin in plugins {
        if !out.contains(&plugin) {
            out.push(plugin);
        }
    }
    out
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Writes the game into `<destination>/<title>/` and returns that folder. `plugins` are the
/// plugins the scene needs, with where their folders are; `exe` is the program to copy.
pub fn export(
    scene: &Scene,
    plugins: &[(RequiredPlugin, PathBuf)],
    title: &str,
    destination: &Path,
    exe: &Path,
) -> Result<PathBuf, String> {
    let root = destination.join(folder_name(title));
    let game = root.join(GAME_DIR);
    let io = |what: &str, e: std::io::Error| format!("{what}: {e}");
    std::fs::create_dir_all(&game).map_err(|e| io("Could not create the folder", e))?;

    let exe_name = if cfg!(windows) {
        "Player.exe"
    } else {
        "Player"
    };
    std::fs::copy(exe, root.join(exe_name)).map_err(|e| io("Could not copy the player", e))?;

    for (plugin, dir) in plugins {
        copy_dir(dir, &game.join("plugins").join(&plugin.name))
            .map_err(|e| io(&format!("Could not copy plugin {}", plugin.name), e))?;
    }

    let info = GameInfo {
        title: title.to_string(),
        clockwork: env!("CARGO_PKG_VERSION").to_string(),
        plugins: plugins.iter().map(|(p, _)| p.clone()).collect(),
    };
    let ron_options = ron::ser::PrettyConfig::default();
    std::fs::write(
        game.join(INFO_FILE),
        ron::ser::to_string_pretty(&info, ron_options.clone()).map_err(|e| e.to_string())?,
    )
    .map_err(|e| io("Could not write the game info", e))?;
    std::fs::write(
        game.join(SCENE_FILE),
        ron::ser::to_string_pretty(scene, ron_options).map_err(|e| e.to_string())?,
    )
    .map_err(|e| io("Could not write the scene", e))?;

    let needs: String = games(&info.plugins)
        .iter()
        .map(|g| format!("  - {g}\n"))
        .collect();
    std::fs::write(
        root.join("README.txt"),
        format!(
            "{title}\n\nMade with Clockwork {version}.\n\nRun {exe_name}. The first time, it asks you to locate\nyour own copy of each game this one is built from:\n\n{needs}\nThis folder contains no game data: only the scene, and recipes (offsets and\nparameters) that turn your own copy of the game into what you see.\n\nControls: WASD or arrows move, Shift walks, Space jumps, J and K use the B and A\nitems, L dodges, I opens the inventory, Esc quits.\n",
            version = info.clockwork,
        ),
    )
    .map_err(|e| io("Could not write the README", e))?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ItemRef, ROOT};
    use bevy::math::Vec3;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clockwork-export-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("My Game: Part 1/2"), "My Game_ Part 1_2");
        assert_eq!(folder_name("  "), "Game");
    }

    #[test]
    fn games_are_listed_once_with_readable_names() {
        let plugin = |name: &str, game: &str| RequiredPlugin {
            name: name.into(),
            game: game.into(),
            sha1: vec![],
        };
        let list = games(&[
            plugin("a", "first_game"),
            plugin("b", "second-game_64"),
            plugin("c", "first_game"),
        ]);
        assert_eq!(
            list,
            vec!["First Game".to_string(), "Second Game 64".to_string()]
        );
    }

    #[test]
    fn plugins_used_lists_each_once_in_order() {
        let mut scene = Scene::default();
        let item = |p: &str| {
            Some(ItemRef {
                plugin: p.into(),
                output: "x".into(),
            })
        };
        scene.add(ROOT, "a".into(), item("b"), Vec3::ZERO);
        scene.add(ROOT, "c".into(), None, Vec3::ZERO);
        scene.add(ROOT, "d".into(), item("a"), Vec3::ZERO);
        scene.add(ROOT, "e".into(), item("b"), Vec3::ZERO);
        assert_eq!(
            plugins_used(&scene, &AssetIndex::default()),
            vec!["b".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn plugins_named_only_by_settings_are_used_too() {
        let mut scene = Scene::default();
        scene.game.preset = Some(OutputRef::new("kit", "game"));
        let id = scene.add(ROOT, "spikes".into(), None, Vec3::ZERO);
        scene.get_mut(id).unwrap().behaviors = vec![clockwork_core::behavior::NodeBehavior {
            from: Some(OutputRef::new("traps", "spikes")),
            ..clockwork_core::behavior::NodeBehavior::new(Behavior::new("Hazard").unwrap())
        }];
        scene.get_mut(id).unwrap().hud = Some(OutputRef::new(BUILTIN, "hud"));
        let used = plugins_used(&scene, &AssetIndex::default());
        assert!(used.contains(&"kit".to_string()));
        assert!(used.contains(&"traps".to_string()));
        assert!(!used.contains(&BUILTIN.to_string()));
    }

    #[test]
    fn a_games_flow_and_its_dependencies_are_exported() {
        let plugins = [clockwork_recipe::Plugin {
            dir: Default::default(),
            manifest: clockwork_core::PluginManifest::from_toml(
                "[plugin]\nname = \"screens\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n[source]\ngame = \"g\"\nsha1 = []\n",
            ).unwrap(),
            recipe: ron::from_str(r#"(steps: [], outputs: [
                Game(id: "preset", game: (flow: Some("screens/alias"))),
                Flow(id: "alias", flow: "screens/main"),
                Flow(id: "main", flow: (
                    title: Some((logo: Some("art/logo"), prompt: "words/start")),
                    files: Some((title: "files/title")),
                    acts: Some((courses: [(name: "courses/name", entry: "door", acts: ["acts/name"])])),
                    game_over: Some((text: "end/text")),
                )),
            ])"#).unwrap(),
            compose: None,
        }];
        let assets = AssetIndex::build(&plugins);
        let mut scene = Scene::default();
        for preset in [false, true] {
            scene.game.flow = (!preset).then(|| OutputRef::new("screens", "alias"));
            scene.game.preset = preset.then(|| OutputRef::new("screens", "preset"));
            let mut used = plugins_used(&scene, &assets);
            used.sort();
            assert_eq!(
                used,
                ["acts", "art", "courses", "end", "files", "screens", "words"]
            );
        }
        let users = assets.used_by(&OutputRef::new("art", "logo"));
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].0.at, OutputRef::new("screens", "main"));
    }

    #[test]
    fn an_export_has_the_player_the_scene_and_only_recipes() {
        let work = scratch("full");
        let plugin_dir = work.join("plugins").join("p");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("clockwork.toml"), "[plugin]").unwrap();
        std::fs::write(plugin_dir.join("recipe.ron"), "Recipe()").unwrap();
        let exe = work.join("clockwork.exe");
        std::fs::write(&exe, b"binary").unwrap();

        let mut scene = Scene::default();
        scene.add(
            ROOT,
            "Hero".into(),
            Some(ItemRef {
                plugin: "p".into(),
                output: "hero".into(),
            }),
            Vec3::new(1.0, 0.0, 2.0),
        );
        let required = RequiredPlugin {
            name: "p".into(),
            game: "first_game".into(),
            sha1: vec!["abc".into()],
        };
        let out = export(
            &scene,
            &[(required.clone(), plugin_dir)],
            "Test Game",
            &work,
            &exe,
        )
        .unwrap();
        assert_eq!(out, work.join("Test Game"));
        let game = out.join(GAME_DIR);
        assert!(
            out.join(if cfg!(windows) {
                "Player.exe"
            } else {
                "Player"
            })
            .exists()
        );
        assert!(out.join("README.txt").exists());
        assert!(game.join("plugins/p/recipe.ron").exists());

        let info: GameInfo =
            ron::from_str(&std::fs::read_to_string(game.join(INFO_FILE)).unwrap()).unwrap();
        assert_eq!(info.title, "Test Game");
        assert_eq!(info.plugins, vec![required]);
        let back: Scene =
            ron::from_str(&std::fs::read_to_string(game.join(SCENE_FILE)).unwrap()).unwrap();
        assert_eq!(back.nodes, scene.nodes);
        // Nothing but small text files besides the program.
        let mut sizes = Vec::new();
        for entry in walk(&game) {
            sizes.push(std::fs::metadata(entry).unwrap().len());
        }
        assert!(sizes.iter().all(|s| *s < 10_000));
        let _ = std::fs::remove_dir_all(work);
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }
}

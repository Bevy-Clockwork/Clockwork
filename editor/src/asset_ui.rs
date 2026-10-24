//! The Inspector's view of an asset: what it uses, as a tree, and what uses it (other assets and
//! the scene's nodes), each a click away. Also the camera a played node uses instead of its
//! character's own.

use bevy::prelude::{Local, ResMut};
use bevy_egui::egui::{self, RichText};
use clockwork_core::{
    asset::{AssetId, Layer, OutputRef, Slot},
    behavior::NodeBehavior,
};
use clockwork_recipe::assets::{Asset, AssetIndex};

use crate::{
    browser::Browser,
    kit,
    scene::{Game, Node, NodeId},
    state::{EditorState, LogLevel},
    theme,
};

/// How deep the "Uses" tree goes before it stops.
const MAX_DEPTH: usize = 8;

/// Something the asset view asked for.
enum Pick {
    Asset(OutputRef),
    Node(NodeId),
}

/// The asset's kind, the tree of what it uses and the list of what uses it.
pub fn asset_section(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    browser: &mut Browser,
    id: &AssetId,
    extractions: &crate::workshop::Extractions,
) {
    let at = &id.at;
    // Its name and kind, a character's actions, then its fields (which may write its plugin),
    // then what it uses and what uses it.
    let kind = {
        let index = AssetIndex::build(&state.workshop.plugins);
        let Some(asset) = index.find(id) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("◇").size(16.0).color(theme::BRASS));
            ui.label(RichText::new(&at.output).font(theme::strong(15.0)));
            ui.label(kit::hint(asset.kind));
        });
        ui.label(kit::hint(at.to_string()));
        asset.kind
    };
    // A character's actions come first: their timeline is what is changed most.
    crate::timeline::actions_section(ui, state, at, kind, extractions);
    let mut editor = std::mem::take(&mut state.workshop.asset_editor);
    crate::asset_edit::fields_section(ui, state, &mut editor, at, kind);
    state.workshop.asset_editor = editor;

    let mut pick = None;
    {
        let index = AssetIndex::build(&state.workshop.plugins);
        let Some(asset) = index.get_kind(at, kind) else {
            return;
        };

        kit::foldout(ui, ("asset_uses", at), "Uses", true, |ui| {
            if asset.uses.is_empty() {
                ui.label(kit::hint("Nothing: everything is written in place."));
            }
            let mut path = vec![at.clone()];
            uses_tree(ui, &index, asset, &mut path, &mut pick);
        });

        kit::foldout(ui, ("asset_used_by", at), "Used by", true, |ui| {
            let users = index.used_by(at);
            let nodes: Vec<(NodeId, String, &str)> = state
                .workshop
                .scene
                .nodes
                .iter()
                .flat_map(|n| {
                    let placed = n
                        .item
                        .as_ref()
                        .is_some_and(|i| i.plugin == at.plugin && i.output == at.output);
                    let setting = NODE_PICKS.iter().find(|p| (p.get)(n).as_ref() == Some(at));
                    let menu = n.menus.iter().flatten().any(|m| m == at);
                    let behavior = n.behaviors.iter().any(|b| b.from.as_ref() == Some(at));
                    [
                        placed.then(|| (n.id, n.name.clone(), "placed in the scene")),
                        setting.map(|p| (n.id, n.name.clone(), p.label)),
                        menu.then(|| (n.id, n.name.clone(), "menu")),
                        behavior.then(|| (n.id, n.name.clone(), "behavior")),
                    ]
                })
                .flatten()
                .collect();
            let game = &state.workshop.scene.game;
            let game_uses: Vec<&str> = GAME_PICKS
                .iter()
                .filter(|p| (p.get)(game).as_ref() == Some(at))
                .map(|p| p.label)
                .chain(
                    game.menus
                        .iter()
                        .flatten()
                        .filter(|m| *m == at)
                        .map(|_| "menu"),
                )
                .collect();
            if users.is_empty() && nodes.is_empty() && game_uses.is_empty() {
                ui.label(kit::hint("Nothing uses it yet."));
            }
            for how in game_uses {
                ui.horizontal(|ui| {
                    ui.label("The game");
                    ui.label(kit::hint(how));
                });
            }
            for (user, slot) in users {
                ui.horizontal(|ui| {
                    if ui.link(user.at.to_string()).clicked() {
                        pick = Some(Pick::Asset(user.at.clone()));
                    }
                    ui.label(kit::hint(format!("as {slot}")));
                });
            }
            for (id, name, how) in nodes {
                ui.horizontal(|ui| {
                    if ui.link(format!("Scene · {name}")).clicked() {
                        pick = Some(Pick::Node(id));
                    }
                    ui.label(kit::hint(how));
                });
            }
        });
    }
    match pick {
        Some(Pick::Asset(at)) => select(state, browser, at),
        Some(Pick::Node(id)) => {
            state.workshop.selected_node = Some(id);
        }
        None => {}
    }
}

/// One level of the "Uses" tree under `at`; `path` holds the assets above, to stop at loops.
fn uses_tree(
    ui: &mut egui::Ui,
    index: &AssetIndex,
    asset: &Asset,
    path: &mut Vec<OutputRef>,
    pick: &mut Option<Pick>,
) {
    for used in &asset.uses {
        let label = format!("{}: {}", used.slot, used.asset);
        let found = index.get(&used.asset);
        let looped = path.contains(&used.asset);
        let leaf = looped || path.len() >= MAX_DEPTH || found.is_none_or(|a| a.uses.is_empty());
        if leaf {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                if ui.link(&label).clicked() && found.is_some() {
                    *pick = Some(Pick::Asset(used.asset.clone()));
                }
                if found.is_none() {
                    ui.label(RichText::new("missing").color(theme::ERROR));
                } else if looped {
                    ui.label(RichText::new("loop").color(theme::ERROR));
                }
            });
            continue;
        }
        let response = egui::CollapsingHeader::new(&label)
            .id_salt((path.clone(), used.slot, &used.asset))
            .default_open(true)
            .show(ui, |ui| {
                path.push(used.asset.clone());
                if let Some(found) = found {
                    uses_tree(ui, index, found, path, pick);
                }
                path.pop();
            });
        if response.header_response.double_clicked() {
            *pick = Some(Pick::Asset(used.asset.clone()));
        }
    }
}

/// Selects an asset in the browser, so the Inspector shows it.
pub fn select(state: &mut EditorState, browser: &mut Browser, at: OutputRef) {
    state.workshop.selected = state.workshop.plugin_index(&at.plugin);
    state.workshop.selected_node = None;
    let id = AssetIndex::build(&state.workshop.plugins)
        .get(&at)
        .map(|a| a.id());
    match id {
        Some(id) => browser.select(id),
        None => browser.selected = None,
    }
}

/// A node's or the game's setting that names one asset of a kind.
pub struct Setting {
    pub label: &'static str,
    pub kind: &'static str,
    pub get: fn(&Node) -> Option<OutputRef>,
    pub set: fn(&mut Node, Option<OutputRef>),
}

/// The settings a played node can take from other assets than its character's.
pub const NODE_PICKS: [Setting; 6] = [
    Setting {
        label: "Camera",
        kind: "Camera",
        get: |n| n.camera.clone(),
        set: |n, v| n.camera = v,
    },
    Setting {
        label: "HUD",
        kind: "Hud",
        get: |n| n.hud.clone(),
        set: |n, v| n.hud = v,
    },
    Setting {
        label: "UI",
        kind: "Ui",
        get: |n| n.ui.clone(),
        set: |n, v| n.ui = v,
    },
    Setting {
        label: "Input",
        kind: "InputMap",
        get: |n| n.input.clone(),
        set: |n, v| n.input = v,
    },
    Setting {
        label: "Stats",
        kind: "Stats",
        get: |n| n.stats.clone(),
        set: |n, v| n.stats = v,
    },
    Setting {
        label: "Items",
        kind: "Items",
        get: |n| n.inventory.clone(),
        set: |n, v| n.inventory = v,
    },
];

/// A drop-down of the assets of one kind, with "inherit" (`None`) first. Returns the choice
/// when one was made.
pub fn asset_picker(
    ui: &mut egui::Ui,
    index: &AssetIndex,
    salt: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    kind: &str,
    current: Option<&OutputRef>,
    inherit: &str,
) -> Option<Option<OutputRef>> {
    let shown = current.map_or(inherit.to_string(), |c| c.to_string());
    let mut chosen = None;
    kit::prop(ui, label, |ui| {
        egui::ComboBox::from_id_salt(salt)
            .width(ui.available_width() - 30.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), inherit).clicked() {
                    chosen = Some(None);
                }
                for asset in index.of_kind(kind) {
                    let on = current == Some(&asset.at);
                    if ui.selectable_label(on, asset.at.to_string()).clicked() {
                        chosen = Some(Some(asset.at.clone()));
                    }
                }
            });
    });
    chosen
}

/// A choice of menus: "inherit" (`None`) or a list of menu assets, ticked in a drop-down.
/// Returns the new choice when one was made.
fn menus_picker(
    ui: &mut egui::Ui,
    index: &AssetIndex,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: Option<&Vec<OutputRef>>,
    inherit: &str,
) -> Option<Option<Vec<OutputRef>>> {
    let shown = match current {
        None => inherit.to_string(),
        Some(list) if list.is_empty() => "No menus".to_string(),
        Some(list) => list
            .iter()
            .map(|m| m.output.clone())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let mut chosen = None;
    kit::prop(ui, "Menus", |ui| {
        egui::ComboBox::from_id_salt(salt)
            .width(ui.available_width() - 30.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), inherit).clicked() {
                    chosen = Some(None);
                }
                for menu in index.of_kind("Menu") {
                    let mut on = current.is_some_and(|c| c.contains(&menu.at));
                    if ui.checkbox(&mut on, menu.at.to_string()).changed() {
                        let mut list = current.cloned().unwrap_or_default();
                        if on {
                            list.push(menu.at.clone());
                        } else {
                            list.retain(|m| *m != menu.at);
                        }
                        chosen = Some(Some(list));
                    }
                }
            });
    });
    chosen
}

/// The camera, HUD, UI and input a played node uses: its character's own, or assets chosen here.
pub fn node_overrides(ui: &mut egui::Ui, state: &mut EditorState, id: NodeId) {
    let Some(node) = state.workshop.scene.get(id).cloned() else {
        return;
    };
    let mut changes = Vec::new();
    let menus;
    {
        let index = AssetIndex::build(&state.workshop.plugins);
        for (i, pick) in NODE_PICKS.iter().enumerate() {
            if let Some(choice) = asset_picker(
                ui,
                &index,
                ("node_pick", id, i),
                pick.label,
                pick.kind,
                (pick.get)(&node).as_ref(),
                "Character's own",
            ) {
                changes.push((i, choice));
            }
        }
        menus = menus_picker(
            ui,
            &index,
            ("node_menus", id),
            node.menus.as_ref(),
            "Character's own",
        );
    }
    if changes.is_empty() && menus.is_none() {
        return;
    }
    if let Some(node) = state.workshop.scene.get_mut(id) {
        for (i, choice) in changes {
            (NODE_PICKS[i].set)(node, choice);
        }
        if let Some(menus) = menus {
            node.menus = menus;
        }
    }
    state.workshop.scene.dirty = true;
}

/// The behavior asset `at` resolved to its behavior, or why not.
fn behavior_asset(
    state: &EditorState,
    at: &OutputRef,
) -> Result<clockwork_core::behavior::Behavior, String> {
    let index = AssetIndex::build(&state.workshop.plugins);
    clockwork_core::asset::resolve(
        &[(Layer::Asset, Some(&Slot::Ref(at.clone())))],
        &index,
        "behavior",
        clockwork_core::asset::behavior_of,
    )
    .map_err(|e| e.to_string())?
    .map(|r| r.value)
    .ok_or_else(|| format!("{at} holds no behavior"))
}

/// Makes a node's behavior number `index` a copy of a behavior asset, kept up to date with it
/// (one past its last adds one); `None` keeps the copy as the node's own, to edit.
pub fn link_behavior(state: &mut EditorState, id: NodeId, index: usize, at: Option<OutputRef>) {
    let behavior = match &at {
        Some(at) => match behavior_asset(state, at) {
            Ok(b) => Some(b),
            Err(why) => {
                state.log(LogLevel::Warn, why);
                return;
            }
        },
        None => None,
    };
    if let Some(node) = state.workshop.scene.get_mut(id) {
        match (node.behaviors.get_mut(index), behavior) {
            (Some(entry), behavior) => {
                if let Some(behavior) = behavior {
                    entry.behavior = behavior;
                }
                entry.from = at;
            }
            (None, Some(behavior)) => node.behaviors.push(NodeBehavior {
                from: at,
                ..NodeBehavior::new(behavior)
            }),
            (None, None) => return,
        }
        state.workshop.scene.dirty = true;
    }
}

/// Brings nodes whose behavior comes from an asset up to date when plugins change.
pub fn relink_system(mut state: ResMut<EditorState>, mut seen: Local<Option<u64>>) {
    if *seen == Some(state.workshop.plugins_revision) {
        return;
    }
    *seen = Some(state.workshop.plugins_revision);
    let linked: Vec<(NodeId, usize, OutputRef)> = state
        .workshop
        .scene
        .nodes
        .iter()
        .flat_map(|n| {
            n.behaviors
                .iter()
                .enumerate()
                .filter_map(|(k, b)| Some((n.id, k, b.from.clone()?)))
        })
        .collect();
    for (id, k, at) in linked {
        match behavior_asset(&state, &at) {
            Ok(behavior) => {
                let stale = state
                    .workshop
                    .scene
                    .get(id)
                    .and_then(|n| n.behaviors.get(k))
                    .is_some_and(|b| b.behavior != behavior);
                if stale
                    && let Some(entry) = state
                        .workshop
                        .scene
                        .get_mut(id)
                        .and_then(|n| n.behaviors.get_mut(k))
                {
                    entry.behavior = behavior;
                    state.workshop.scene.changed();
                }
            }
            Err(why) => state.log(LogLevel::Warn, why),
        }
    }
}

/// Where a node's behavior number `index` comes from: its own, or a behavior asset it follows.
pub fn behavior_link(ui: &mut egui::Ui, state: &mut EditorState, id: NodeId, index: usize) {
    let Some(current) = state
        .workshop
        .scene
        .get(id)
        .and_then(|n| n.behaviors.get(index))
        .map(|b| b.from.clone())
    else {
        return;
    };
    let choice = {
        let assets = AssetIndex::build(&state.workshop.plugins);
        asset_picker(
            ui,
            &assets,
            ("behavior_from", id, index),
            "From",
            "Behavior",
            current.as_ref(),
            "Its own",
        )
    };
    if let Some(choice) = choice {
        link_behavior(state, id, index, choice);
    }
    if current.is_some() {
        ui.label(kit::hint(
            "Follows the asset: choose \"Its own\" to change it here.",
        ));
    }
}

/// A game setting that names one asset of a kind.
pub struct GameSetting {
    pub label: &'static str,
    pub kind: &'static str,
    pub get: fn(&Game) -> Option<OutputRef>,
    pub set: fn(&mut Game, Option<OutputRef>),
}

pub const GAME_PICKS: [GameSetting; 10] = [
    GameSetting {
        label: "Preset",
        kind: "Game",
        get: |g| g.preset.clone(),
        set: |g, v| g.preset = v,
    },
    GameSetting {
        label: "Flow",
        kind: "Flow",
        get: |g| g.flow.clone(),
        set: |g, v| g.flow = v,
    },
    GameSetting {
        label: "Rules",
        kind: "Rules",
        get: |g| g.rules.clone(),
        set: |g, v| g.rules = v,
    },
    GameSetting {
        label: "Camera",
        kind: "Camera",
        get: |g| g.camera.clone(),
        set: |g, v| g.camera = v,
    },
    GameSetting {
        label: "HUD",
        kind: "Hud",
        get: |g| g.hud.clone(),
        set: |g, v| g.hud = v,
    },
    GameSetting {
        label: "UI",
        kind: "Ui",
        get: |g| g.ui.clone(),
        set: |g, v| g.ui = v,
    },
    GameSetting {
        label: "Input",
        kind: "InputMap",
        get: |g| g.input.clone(),
        set: |g, v| g.input = v,
    },
    GameSetting {
        label: "Stats",
        kind: "Stats",
        get: |g| g.stats.clone(),
        set: |g, v| g.stats = v,
    },
    GameSetting {
        label: "Items",
        kind: "Items",
        get: |g| g.inventory.clone(),
        set: |g, v| g.inventory = v,
    },
    GameSetting {
        label: "Music",
        kind: "Music",
        get: |g| g.music.clone(),
        set: |g, v| g.music = v,
    },
];

/// The game's own settings, shown with the scene's root.
pub fn game_settings(ui: &mut egui::Ui, state: &mut EditorState) {
    let game = state.workshop.scene.game.clone();
    let mut changes = Vec::new();
    let menus;
    {
        let index = AssetIndex::build(&state.workshop.plugins);
        for (i, pick) in GAME_PICKS.iter().enumerate() {
            let inherit = match pick.label {
                "Preset" => "None",
                "Flow" => "Preset or none",
                "Rules" => "Built-in",
                "Music" => "None",
                _ => "Characters' own",
            };
            if let Some(choice) = asset_picker(
                ui,
                &index,
                ("game_pick", i),
                pick.label,
                pick.kind,
                (pick.get)(&game).as_ref(),
                inherit,
            ) {
                changes.push((i, choice));
            }
        }
        menus = menus_picker(
            ui,
            &index,
            "game_menus",
            game.menus.as_ref(),
            "Characters' own",
        );
    }
    ui.label(kit::hint(
        "Every played character uses these, unless its node chooses otherwise.",
    ));
    if changes.is_empty() && menus.is_none() {
        return;
    }
    for (i, choice) in changes {
        (GAME_PICKS[i].set)(&mut state.workshop.scene.game, choice);
    }
    if let Some(menus) = menus {
        state.workshop.scene.game.menus = menus;
    }
    state.workshop.scene.dirty = true;
}

/// Places a node, where the camera looks, whose behavior follows the behavior asset `at`.
pub fn place_behavior(state: &mut EditorState, at: OutputRef) {
    match behavior_asset(state, &at) {
        Ok(behavior) => {
            state.workshop.entity_request = Some(behavior);
            state.workshop.entity_link = Some(at);
        }
        Err(why) => state.log(LogLevel::Warn, why),
    }
}

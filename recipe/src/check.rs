//! Checks content before it is played, from the plugins' recipes alone (no ROM is read): that
//! what an asset names exists, that a game's fonts have the letters its screens write, and that
//! its HUD counts items there are. Play runs it when it starts and the agent when it saves a
//! plugin, so a missing letter or a broken name is reported instead of drawn wrong.

use std::fmt;

use clockwork_core::{
    asset::{OutputRef, Slot},
    flow::{Flow, Label},
    game::GameSettings,
    recipe::Output,
    ui::{self, HudLayout, Menu, WidgetKind, template_letters},
};

use crate::assets::AssetIndex;

/// Something wrong with an asset.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub at: OutputRef,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// The problems of every asset, or of one plugin's.
pub fn check(index: &AssetIndex, plugin: Option<&str>) -> Vec<Problem> {
    let mine = |at: &OutputRef| plugin.is_none_or(|p| at.plugin == p);
    let mut problems = Vec::new();
    for asset in index.all().iter().filter(|a| mine(&a.at)) {
        for used in &asset.uses {
            if index.get(&used.asset).is_none() {
                problems.push(Problem {
                    at: asset.at.clone(),
                    message: format!("its {} \"{}\" doesn't exist", used.slot, used.asset),
                });
            }
        }
        if let Some(Output::Behavior {
            behavior: Slot::Inline(clockwork_core::behavior::Behavior::Actor(actor)),
            ..
        }) = asset.output
        {
            problems.extend(actor.problems().into_iter().map(|message| Problem {
                at: asset.at.clone(),
                message,
            }));
        }
        // An area's animated tiles and layers have timing and scroll that work.
        if let Some(Output::Image { layers, frames, .. }) = asset.output {
            let problems_here = (frames.iter().filter_map(|f| f.problem()))
                .chain(layers.iter().filter_map(|l| l.problem()));
            for message in problems_here {
                problems.push(Problem {
                    at: asset.at.clone(),
                    message,
                });
            }
        }
        if let Some(Output::Game { game, .. }) = asset.output
            && let Some(game) = follow(index, game, |o| match o {
                Output::Game { game, .. } => Some(game),
                _ => None,
            })
        {
            check_game(index, &asset.at, &game, &mut problems);
        }
    }
    problems
}

/// A setting written in place, or the one it names (following names a few times).
fn follow<T: Clone>(
    index: &AssetIndex,
    slot: &Slot<T>,
    of: impl Fn(&Output) -> Option<&Slot<T>>,
) -> Option<T> {
    let mut slot = slot.clone();
    for _ in 0..8 {
        match slot {
            Slot::Inline(value) => return Some(value),
            Slot::Ref(at) => slot = index.get(&at)?.output.and_then(&of)?.clone(),
        }
    }
    None
}

fn named<T: Clone>(
    index: &AssetIndex,
    name: Option<&str>,
    of: impl Fn(&Output) -> Option<&Slot<T>>,
) -> Option<T> {
    follow(index, &Slot::Ref(OutputRef::parse(name?)?), of)
}

/// The characters of a font output, `"plugin/output"`.
fn font_chars(index: &AssetIndex, name: &str) -> Option<String> {
    let at = OutputRef::parse(name)?;
    index.all().iter().find_map(|a| match a.output {
        Some(Output::Font { chars, .. }) if a.at == at => Some(chars.clone()),
        _ => None,
    })
}

/// The letters of `text` a font of `chars` can't write, in either case.
pub fn missing_letters(chars: &str, text: &str) -> Vec<char> {
    let has = |c: char| {
        chars.contains(c)
            || c.to_uppercase().any(|u| chars.contains(u))
            || c.to_lowercase().any(|l| chars.contains(l))
    };
    let mut missing: Vec<char> = template_letters(text)
        .chars()
        .filter(|c| !c.is_whitespace() && !has(*c))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    missing
}

fn check_game(
    index: &AssetIndex,
    at: &OutputRef,
    game: &GameSettings,
    problems: &mut Vec<Problem>,
) {
    let theme = named(index, game.ui.as_deref(), |o| match o {
        Output::Ui { ui, .. } => Some(ui),
        _ => None,
    })
    .unwrap_or_else(ui::default_theme);
    let layout = named(index, game.hud.as_deref(), |o| match o {
        Output::Hud { hud, .. } => Some(hud),
        _ => None,
    })
    .unwrap_or_else(ui::default_hud);
    let menus: Vec<Menu> = match &game.menus {
        Some(names) => names
            .iter()
            .filter_map(|n| {
                named(index, Some(n), |o| match o {
                    Output::Menu { menu, .. } => Some(menu),
                    _ => None,
                })
            })
            .collect(),
        None => [ui::INVENTORY, ui::PAUSE]
            .iter()
            .filter_map(|id| {
                follow(index, &Slot::Ref(ui::builtin(id)), |o| match o {
                    Output::Menu { menu, .. } => Some(menu),
                    _ => None,
                })
            })
            .collect(),
    };
    let flow = named(index, game.flow.as_deref(), |o| match o {
        Output::Flow { flow, .. } => Some(flow),
        _ => None,
    });
    let items = named_items(index, game.inventory.as_deref());

    // Every counter counts an item the game has.
    if let Some(items) = &items {
        for widget in &layout.widgets {
            if let WidgetKind::Counter { item } = &widget.kind
                && !items.iter().any(|(id, _)| id == item)
            {
                problems.push(Problem {
                    at: at.clone(),
                    message: format!("its HUD counts \"{item}\", which its items don't have"),
                });
            }
        }
    }

    // The fonts have the letters of what is written with them.
    let mut letters = |font: &Option<String>, what: String, text: &str| {
        let Some(font) = font else { return };
        let Some(chars) = font_chars(index, font) else {
            return;
        };
        let missing = missing_letters(&chars, text);
        if !missing.is_empty() {
            let missing: String = missing.into_iter().collect();
            problems.push(Problem {
                at: at.clone(),
                message: format!(
                    "{what} \"{text}\" uses letters the font {font} doesn't have: {missing}"
                ),
            });
        }
    };
    let dialog_font = theme.dialog_font.clone().or(theme.font.clone());
    for text in hud_texts(&layout, &menus) {
        letters(&theme.font, text.0, &text.1);
    }
    if let Some(items) = &items
        && layout.words.got.contains("{item}")
    {
        for (id, name) in items {
            letters(&theme.font, format!("the name of item \"{id}\""), name);
        }
    }
    if let Some(flow) = &flow {
        for (what, label, dialog) in flow_labels(flow) {
            if let Slot::Inline(d) = label {
                for page in &d.pages {
                    let font = if dialog { &dialog_font } else { &theme.font };
                    letters(font, what.clone(), page);
                }
            }
        }
    }
}

fn named_items(index: &AssetIndex, name: Option<&str>) -> Option<Vec<(String, String)>> {
    let at = OutputRef::parse(name?)?;
    index.all().iter().find_map(|a| match a.output {
        Some(Output::Items { items, .. }) if a.at == at => Some(
            items
                .iter()
                .map(|i| (i.id.clone(), i.name.clone()))
                .collect(),
        ),
        _ => None,
    })
}

/// What the HUD and menus write in the theme's font, with what each is.
fn hud_texts(layout: &HudLayout, menus: &[Menu]) -> Vec<(String, String)> {
    let mut texts: Vec<(String, String)> = layout
        .words
        .all()
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(name, text)| (format!("the HUD word {name}"), text.to_string()))
        .collect();
    for widget in &layout.widgets {
        if let Some(format) = &widget.format {
            texts.push(("a HUD widget's format".into(), format.clone()));
        }
        match &widget.kind {
            WidgetKind::Label(text) => texts.push(("a HUD label".into(), text.clone())),
            WidgetKind::Button { label, .. } => texts.push(("a HUD button".into(), label.clone())),
            _ => {}
        }
    }
    for menu in menus {
        texts.push(("a menu title".into(), menu.title.clone()));
    }
    texts
}

/// The flow's words, with what each is and whether the dialog font writes it.
fn flow_labels(flow: &Flow) -> Vec<(String, &Label, bool)> {
    let mut labels = Vec::new();
    if let Some(title) = &flow.title {
        labels.push(("the title's prompt".to_string(), &title.prompt, false));
    }
    if let Some(files) = &flow.files {
        labels.push(("the file select's title".to_string(), &files.title, false));
    }
    for course in flow.acts.iter().flat_map(|a| &a.courses) {
        labels.push(("a course name".to_string(), &course.name, false));
        for act in &course.acts {
            labels.push(("an act name".to_string(), act, false));
        }
    }
    if let Some(over) = &flow.game_over {
        labels.push(("the game over text".to_string(), &over.text, true));
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Plugin;

    fn plugin(recipe: &str) -> Plugin {
        Plugin {
            dir: Default::default(),
            manifest: clockwork_core::PluginManifest::from_toml(
                "[plugin]\nname = \"p\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n[source]\ngame = \"g\"\nsha1 = []\n",
            )
            .unwrap(),
            recipe: ron::from_str(recipe).unwrap(),
            compose: None,
        }
    }

    #[test]
    fn letters_count_in_either_case_and_keys_are_left_out() {
        assert_eq!(missing_letters("ABC", "abc {n}"), Vec::<char>::new());
        assert_eq!(missing_letters("ABCx", "NEXT x{n}"), vec!['E', 'N', 'T']);
        assert_eq!(missing_letters("", "  "), Vec::<char>::new());
    }

    #[test]
    fn a_game_s_words_need_its_font_s_letters_and_its_names_must_exist() {
        let plugins = [plugin(
            r#"(steps: [], outputs: [
                Font(id: "font", from: "sheet", glyph: (8, 8), chars: "0123456789ABCDEFGHIKLMNOPRSTUWY×"),
                Items(id: "things", items: [(id: "coin", name: "COIN", description: "", category: Consumable, max: 9)]),
                Ui(id: "ui", ui: (panel: (0, 0, 0, 0), border: (0, 0, 0, 0), text: (0, 0, 0, 0),
                    dim: (0, 0, 0, 0), accent: (0, 0, 0, 0), shade: (0, 0, 0, 0), font: Some("p/font"))),
                Hud(id: "hud", hud: (
                    widgets: [
                        (kind: Lives, icon: Some("p/head"), format: Some("×{n}")),
                        (kind: Counter(item: "coin"), format: Some("×{n}")),
                        (kind: Counter(item: "star")),
                    ],
                    words: (resume: "CONTINUE", start_over: "RESTART", stop_playing: "EXIT",
                        nothing_carried: "", on_a: "", on_b: "", close: "", choose: "", back: "",
                        next: "", got: "", destroyed: "", checkpoint: "", down: "", game_over: "",
                        win: "", file: "", new_file: ""),
                )),
                Menu(id: "pause", menu: (kind: Pause, title: "PAUSE", opens_with: "Pause")),
                Game(id: "game", game: (ui: Some("p/ui"), hud: Some("p/hud"), menus: Some(["p/pause"]),
                    inventory: Some("p/things"))),
            ])"#,
        )];
        let index = AssetIndex::build(&plugins);
        let problems: Vec<String> = check(&index, Some("p"))
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(
            problems,
            [
                "p/hud: its icon \"p/head\" doesn't exist",
                "p/game: its HUD counts \"star\", which its items don't have",
                "p/game: the HUD word stop_playing \"EXIT\" uses letters the font p/font doesn't have: X",
            ],
            "{problems:#?}"
        );
    }

    #[test]
    fn animated_tiles_need_timing_that_works() {
        let plugins = [plugin(
            r#"(steps: [], outputs: [
                Image(id: "area", from: "map", kind: Some(Area),
                      frames: Some((images: ["a", "b"], ticks: [8, 8, 8], fps: 60.0)),
                      layers: [(from: "water", frames: Some((images: ["w"], ticks: [4], fps: 0.0))),
                               (from: "sky", scroll: (0.5, 0.5))]),
            ])"#,
        )];
        let index = AssetIndex::build(&plugins);
        let problems: Vec<String> = check(&index, Some("p"))
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(
            problems,
            [
                "p/area: frames have 2 images but 3 ticks (give one per image, or one for all)",
                "p/area: layer \"water\": frames need fps above 0, not 0",
            ],
            "{problems:#?}"
        );
    }
}

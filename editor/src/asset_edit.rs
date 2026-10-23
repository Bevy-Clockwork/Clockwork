//! Editing an asset from the Inspector: every field its kind has (from the recipe schema), as
//! RON text. Applying writes the plugin's `recipe.ron`, changing only those values, so the rest
//! of the file (comments above all) stays as it was. Built-in assets are read-only; any settings
//! asset can be copied into the user's own plugin, `my-game`, to change there.

use std::path::{Path, PathBuf};

use bevy_egui::egui::{self, RichText};
use clockwork_core::{
    asset::{BUILTIN, OutputRef},
    recipe::{Output, Recipe},
    recipe_schema,
};
use clockwork_recipe::text::{Doc, Node, parse_string};

use crate::{
    kit,
    state::{EditorState, LogLevel},
    theme,
};

/// The plugin assets are copied into.
pub const MY_GAME: &str = "my-game";

/// Kinds that hold settings only, so a copy works anywhere: the others name images, buffers or
/// outputs of their own plugin.
const COPYABLE: [&str; 14] = [
    "Camera",
    "Hud",
    "Ui",
    "Menu",
    "Flow",
    "InputMap",
    "Movement",
    "Animations",
    "Stats",
    "Behavior",
    "Rules",
    "Game",
    "Dialog",
    "Sounds",
];

/// The asset being edited and its fields' text.
#[derive(Default)]
pub struct AssetEditor {
    at: Option<(OutputRef, &'static str)>,
    /// Field, its text as edited, its text in the file (`None`: not written there), and the
    /// indentation of its line there (its text is shown without it).
    fields: Vec<(&'static str, String, Option<String>)>,
    indents: Vec<String>,
    read_only: bool,
    message: Option<(String, bool)>,
}

/// The `outputs` item of kind `kind` with id `id` in a recipe's text.
fn find_output<'a>(doc: &'a Doc, kind: &str, id: &str) -> Option<&'a Node> {
    doc.root.field("outputs")?.items().iter().find(|item| {
        item.name() == Some(kind)
            && item
                .field("id")
                .and_then(|n| parse_string(doc.text_of(n)))
                .as_deref()
                == Some(id)
    })
}

/// `text` with `indent` taken off the start of its lines after the first.
fn dedent(text: &str, indent: &str) -> String {
    text.split('\n')
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line
            } else {
                line.strip_prefix(indent).unwrap_or(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn recipe_path(state: &EditorState, plugin: &str) -> Option<PathBuf> {
    let index = state.workshop.plugin_index(plugin)?;
    Some(state.workshop.plugins[index].dir.join("recipe.ron"))
}

impl AssetEditor {
    /// Loads the fields of an asset of `kind`, unless it is loaded already.
    fn load(&mut self, state: &EditorState, at: &OutputRef, kind: &'static str) {
        if self.at.as_ref() == Some(&(at.clone(), kind)) {
            return;
        }
        *self = AssetEditor {
            at: Some((at.clone(), kind)),
            ..Default::default()
        };
        let Some(info) = recipe_schema::output(kind) else {
            return;
        };
        let text = if at.plugin == BUILTIN {
            self.read_only = true;
            clockwork_core::asset::builtins()
                .iter()
                .find(|o| o.name() == kind && o.id() == at.output)
                .and_then(|o| ron::ser::to_string_pretty(&Recipe::with_output(o), pretty()).ok())
        } else {
            recipe_path(state, &at.plugin).and_then(|p| std::fs::read_to_string(p).ok())
        };
        let Some(text) = text else {
            return;
        };
        let Ok(doc) = Doc::parse(text.trim_start_matches('\u{FEFF}')) else {
            self.message = Some(("The recipe doesn't parse".into(), false));
            return;
        };
        let Some(item) = find_output(&doc, kind, &at.output) else {
            return;
        };
        let line_indent = |at: usize| {
            let start = doc.text[..at].rfind('\n').map_or(0, |i| i + 1);
            let line = &doc.text[start..];
            line[..line.len() - line.trim_start().len()].to_string()
        };
        for f in info.fields.iter().filter(|f| f.key != "id") {
            let node = item.field(f.key);
            let indent = node.map(|n| line_indent(n.span.start)).unwrap_or_default();
            let written = node.map(|n| dedent(doc.text_of(n), &indent));
            self.fields
                .push((f.key, written.clone().unwrap_or_default(), written));
            self.indents.push(indent);
        }
    }

    /// Sets one field of an asset of `kind` and writes it, as the Inspector's Apply does.
    pub fn set_field(
        &mut self,
        state: &mut EditorState,
        at: &OutputRef,
        kind: &'static str,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        self.at = None;
        self.load(state, at, kind);
        if self.read_only {
            return Err(format!("{at} is built in: copy it first"));
        }
        let field = self
            .fields
            .iter_mut()
            .find(|(k, _, _)| *k == key)
            .ok_or_else(|| format!("{kind} has no field \"{key}\""))?;
        field.1 = value.to_string();
        self.apply(state);
        match &self.message {
            Some((why, false)) => Err(why.clone()),
            _ => Ok(()),
        }
    }

    fn changed(&self) -> bool {
        self.fields
            .iter()
            .any(|(_, text, written)| Some(text.as_str()) != written.as_deref())
    }

    /// Writes the changed fields into the plugin's recipe, if the result still reads.
    fn apply(&mut self, state: &mut EditorState) {
        let Some((at, kind)) = self.at.clone() else {
            return;
        };
        let result = (|| {
            let path = recipe_path(state, &at.plugin).ok_or("the plugin isn't installed")?;
            let mut text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            for ((key, value, written), indent) in self.fields.iter().zip(&self.indents) {
                let value = &value.replace('\n', &format!("\n{indent}"));
                if written
                    .as_ref()
                    .is_some_and(|w| w.replace('\n', &format!("\n{indent}")) == *value)
                    || value.trim().is_empty()
                {
                    continue;
                }
                let doc =
                    Doc::parse(text.trim_start_matches('\u{FEFF}')).map_err(|e| e.to_string())?;
                let item = find_output(&doc, kind, &at.output).ok_or("the asset is gone")?;
                text = match item.field(key) {
                    Some(node) => doc.replaced(node.span.clone(), value.trim()),
                    None => doc
                        .field_added(item, key, value.trim())
                        .ok_or("the field can't be added")?,
                };
            }
            ron::from_str::<Recipe>(text.trim_start_matches('\u{FEFF}'))
                .map_err(|e| format!("Not saved: {e}"))?;
            std::fs::write(&path, &text).map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                state.log(LogLevel::Info, format!("Saved {at}"));
                self.message = Some(("Saved".into(), true));
                for (_, text, written) in &mut self.fields {
                    *written = Some(text.clone());
                }
            }
            Err(why) => self.message = Some((why, false)),
        }
    }
}

fn pretty() -> ron::ser::PrettyConfig {
    // Plain newlines on every system, like the recipes copies go into.
    ron::ser::PrettyConfig::new()
        .depth_limit(3)
        .new_line("\n".to_string())
}

/// The asset's fields, editable unless it is built in, and "Copy to my game".
pub fn fields_section(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    editor: &mut AssetEditor,
    at: &OutputRef,
    kind: &'static str,
) {
    editor.load(state, at, kind);
    kit::foldout(ui, ("asset_fields", at), "Fields", true, |ui| {
        if editor.fields.is_empty() {
            ui.label(kit::hint("Nothing to edit here."));
        }
        for (key, text, _) in &mut editor.fields {
            ui.label(RichText::new(*key).color(theme::PARCHMENT_DIM));
            let lines = text.lines().count().clamp(1, 12);
            let edit = if lines > 1 || text.len() > 40 {
                egui::TextEdit::multiline(text)
                    .desired_rows(lines)
                    .code_editor()
            } else {
                egui::TextEdit::singleline(text).code_editor()
            };
            ui.add_enabled(!editor.read_only, edit.desired_width(ui.available_width()));
        }
        ui.horizontal(|ui| {
            if !editor.read_only
                && ui
                    .add_enabled(editor.changed(), egui::Button::new("Apply"))
                    .clicked()
            {
                editor.apply(state);
            }
            if COPYABLE.contains(&kind)
                && ui
                    .button("Copy to my game")
                    .on_hover_text(format!("A copy in the plugin \"{MY_GAME}\", to change"))
                    .clicked()
            {
                editor.message = Some(match copy_to_my_game(state, at, kind) {
                    Ok(copy) => (format!("Copied as {copy}"), true),
                    Err(why) => (why, false),
                });
            }
        });
        if editor.read_only {
            ui.label(kit::hint("Built in: copy it to change it."));
        }
        if let Some((message, ok)) = &editor.message {
            ui.label(RichText::new(message).color(if *ok {
                theme::PARCHMENT_DIM
            } else {
                theme::ERROR
            }));
        }
    });
}

/// The user's own plugin, made the first time.
fn my_game_dir() -> Result<PathBuf, String> {
    let dir = clockwork_library::store::data_dir()
        .ok_or("no data folder")?
        .join("plugins")
        .join(MY_GAME);
    if !dir.join("recipe.ron").exists() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(
            dir.join("clockwork.toml"),
            format!(
                "[plugin]\nname = \"{MY_GAME}\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n\n[source]\ngame = \"{}\"\nsha1 = []\n",
                clockwork_core::compose::COMPOSITION_GAME
            ),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(
            dir.join("recipe.ron"),
            "// Assets copied here from the Workshop, to change.\n(\n    steps: [],\n    outputs: [],\n)\n",
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Copies a settings asset into `my-game` under a free id; returns its name.
pub fn copy_to_my_game(
    state: &mut EditorState,
    at: &OutputRef,
    kind: &str,
) -> Result<OutputRef, String> {
    let output = find_asset_output(state, at, kind).ok_or("the asset isn't there")?;
    let dir = my_game_dir()?;
    copy_into(&dir, &output)
}

fn find_asset_output(state: &EditorState, at: &OutputRef, kind: &str) -> Option<Output> {
    if at.plugin == BUILTIN {
        return clockwork_core::asset::builtins()
            .iter()
            .find(|o| o.name() == kind && o.id() == at.output)
            .cloned();
    }
    let index = state.workshop.plugin_index(&at.plugin)?;
    state.workshop.plugins[index]
        .recipe
        .outputs
        .iter()
        .find(|o| o.name() == kind && o.id() == at.output)
        .cloned()
}

/// Appends `output`, renamed to a free id, to the recipe in `dir`.
fn copy_into(dir: &Path, output: &Output) -> Result<OutputRef, String> {
    let path = dir.join("recipe.ron");
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let recipe: Recipe =
        ron::from_str(text.trim_start_matches('\u{FEFF}')).map_err(|e| e.to_string())?;
    let taken = |id: &str| recipe.outputs.iter().any(|o| o.id() == id);
    let base = output.id().to_string();
    let id = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}-{n}")))
        .find(|id| !taken(id))
        .expect("an endless supply of ids");
    let mut copy = output.clone();
    copy.set_id(&id);
    let doc = Doc::parse(text.trim_start_matches('\u{FEFF}')).map_err(|e| e.to_string())?;
    let outputs = doc
        .root
        .field("outputs")
        .ok_or("the recipe has no outputs")?;
    // The copy's lines indented like the outputs around it.
    let line_start = |at: usize| doc.text[..at].rfind('\n').map_or(0, |i| i + 1);
    let indent = match outputs.items().last() {
        Some(last) => last.span.start - line_start(last.span.start),
        None => {
            let line = &doc.text[line_start(outputs.span.start)..outputs.span.start];
            line.len() - line.trim_start().len() + 4
        }
    };
    let item = ron::ser::to_string_pretty(&copy, pretty())
        .map_err(|e| e.to_string())?
        .replace('\n', &format!("\n{}", " ".repeat(indent)));
    let written = doc.appended(outputs, &item);
    ron::from_str::<Recipe>(&written).map_err(|e| e.to_string())?;
    std::fs::write(&path, written).map_err(|e| e.to_string())?;
    Ok(OutputRef::new(MY_GAME, &id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_field_is_editable_and_copyable() {
        assert!(COPYABLE.contains(&"Flow"));
        let info = recipe_schema::output("Flow").unwrap();
        assert!(info.field("flow").is_some());
        let doc = Doc::parse(
            r#"(steps: [], outputs: [
            // the screens
            Flow(id: "screens", flow: ()),
        ])"#,
        )
        .unwrap();
        let item = find_output(&doc, "Flow", "screens").unwrap();
        let field = item.field("flow").unwrap();
        let edited = doc.replaced(
            field.span.clone(),
            r#"(title: Some((prompt: (pages: ["PRESS START"]))))"#,
        );
        assert!(edited.contains("// the screens"));
        let recipe: Recipe = ron::from_str(&edited).unwrap();
        let Some(clockwork_core::asset::Slot::Inline(flow)) =
            clockwork_core::asset::flow_of(&recipe.outputs[0])
        else {
            panic!("an inline flow")
        };
        assert!(flow.title.is_some());
    }

    #[test]
    fn a_copy_gets_a_free_id_and_the_recipe_still_reads() {
        let dir = std::env::temp_dir().join(format!("clockwork-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("recipe.ron"),
            "(\n    steps: [],\n    outputs: [\n    ],\n)\n",
        )
        .unwrap();
        let camera = clockwork_core::asset::builtins()
            .iter()
            .find(|o| o.name() == "Camera")
            .unwrap()
            .clone();
        let first = copy_into(&dir, &camera).unwrap();
        let second = copy_into(&dir, &camera).unwrap();
        assert_eq!(first.output, camera.id());
        assert_eq!(second.output, format!("{}-2", camera.id()));
        let text = std::fs::read_to_string(dir.join("recipe.ron")).unwrap();
        let recipe: Recipe = ron::from_str(&text).unwrap();
        // Each copy on its own lines, indented under `outputs`.
        assert!(
            text.contains(
                "
        Camera(
            id:"
            ),
            "{text}"
        );
        assert_eq!(recipe.outputs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fields_show_without_the_indentation_of_their_line() {
        let written = "Hazard(\n            radius: 0.8,\n        )";
        let shown = dedent(written, "        ");
        assert_eq!(shown, "Hazard(\n    radius: 0.8,\n)");
        assert_eq!(shown.replace('\n', "\n        "), written);
    }

    #[test]
    fn a_field_is_found_and_replaced_in_place() {
        let text = "(\n    steps: [],\n    outputs: [\n        // the camera\n        Camera(id: \"c\", camera: TopDown((distance: 9.0, pitch: 90.0))),\n    ],\n)\n";
        let doc = Doc::parse(text).unwrap();
        let item = find_output(&doc, "Camera", "c").unwrap();
        let field = item.field("camera").unwrap();
        let edited = doc.replaced(field.span.clone(), "\"builtin/top-down\"");
        assert!(edited.contains("// the camera"));
        let recipe: Recipe = ron::from_str(&edited).unwrap();
        assert!(matches!(
            &recipe.outputs[0],
            Output::Camera {
                camera: clockwork_core::asset::Slot::Ref(_),
                ..
            }
        ));
    }
}

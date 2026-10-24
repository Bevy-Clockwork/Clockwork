//! Changes to a recipe's text that keep everything else as it was written: the agent's
//! comments, sections and layout. The editor's recipe graph and the agent's `edit_plugin` tool
//! both make their changes through these, so a plugin can be changed a step at a time instead
//! of written again whole.

use std::fmt;
use std::str::FromStr;

use clockwork_core::recipe::Recipe;

use crate::text::{Doc, Kind, Node};

/// A step or an output, by its place in its list: `"step 3"`, `"output 0"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Step(usize),
    Output(usize),
}

impl Item {
    /// Its place in its list.
    pub fn index(self) -> usize {
        match self {
            Item::Step(i) | Item::Output(i) => i,
        }
    }

    /// The item of the same list at another place.
    pub fn at(self, index: usize) -> Item {
        match self {
            Item::Step(_) => Item::Step(index),
            Item::Output(_) => Item::Output(index),
        }
    }

    /// The list it is in: `"steps"` or `"outputs"`.
    pub fn list(self) -> &'static str {
        match self {
            Item::Step(_) => "steps",
            Item::Output(_) => "outputs",
        }
    }
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Item::Step(i) => write!(f, "step {i}"),
            Item::Output(i) => write!(f, "output {i}"),
        }
    }
}

impl FromStr for Item {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (kind, index) = text
            .trim()
            .split_once(' ')
            .ok_or_else(|| format!("write a node as \"step 3\" or \"output 0\", not \"{text}\""))?;
        let index: usize = index
            .trim()
            .parse()
            .map_err(|_| format!("\"{index}\" isn't a number"))?;
        match kind {
            "step" => Ok(Item::Step(index)),
            "output" => Ok(Item::Output(index)),
            _ => Err(format!("\"{kind}\" isn't step or output")),
        }
    }
}

/// One change to a recipe.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Sets a field of a step or output to a value written in RON (`"0x40"`, `"Some(16)"`,
    /// `"\"tiles\""`), adding the field when it was left out.
    Set {
        item: Item,
        field: String,
        value: String,
    },
    /// Puts a step (written in RON) at a place among the steps, or last.
    InsertStep {
        at: Option<usize>,
        step: String,
    },
    /// Puts an output (written in RON) at a place among the outputs, or last.
    InsertOutput {
        at: Option<usize>,
        output: String,
    },
    /// Writes a whole step or output anew, keeping its place and its note.
    Replace {
        item: Item,
        with: String,
    },
    Remove {
        item: Item,
    },
    /// Moves a step or output to another place in its list, one place at a time, each with
    /// the comment on its line.
    Move {
        item: Item,
        to: usize,
    },
    /// The `//` lines right above a step or output (see [`Doc::note`]); empty removes them.
    Note {
        item: Item,
        note: String,
    },
}

/// The node of a step or output in a recipe's text.
pub fn node(doc: &Doc, item: Item) -> Option<&Node> {
    doc.root.field(item.list())?.items().get(item.index())
}

fn parse(text: String) -> Result<Doc, String> {
    Doc::parse(text).map_err(|e| e.to_string())
}

fn find(doc: &Doc, item: Item) -> Result<&Node, String> {
    node(doc, item).ok_or_else(|| format!("there is no {item}"))
}

/// The text with `item` moved to place `to` of its list (the last place at most).
pub fn moved(doc: &Doc, item: Item, to: usize) -> Result<String, String> {
    let len = doc
        .root
        .field(item.list())
        .map_or(0, |list| list.items().len());
    find(doc, item)?;
    let to = to.min(len.saturating_sub(1));
    let mut doc = doc.clone();
    let mut at = item.index();
    while at != to {
        let next = if to > at { at + 1 } else { at - 1 };
        let a = find(&doc, item.at(at.min(next)))?;
        let b = find(&doc, item.at(at.max(next)))?;
        let text = doc.swapped(a, b).ok_or_else(|| {
            format!("{item} shares a line with another in recipe.ron, so it can't move")
        })?;
        doc = parse(text)?;
        at = next;
    }
    Ok(doc.text)
}

/// The text with one change made. It is only checked to be RON; see [`apply_all`].
pub fn apply(doc: &Doc, edit: &Edit) -> Result<String, String> {
    let text = match edit {
        Edit::Set { item, field, value } => {
            let node = find(doc, *item)?;
            if !matches!(node.kind, Kind::Struct { .. }) {
                return Err(format!("{item} has no fields"));
            }
            match node.field(field) {
                Some(old) => doc.replaced(old.span.clone(), value.trim()),
                None => doc
                    .field_added(node, field, value.trim())
                    .ok_or_else(|| format!("{item} has no fields"))?,
            }
        }
        Edit::InsertStep { at, step } => insert(doc, "steps", *at, step)?,
        Edit::InsertOutput { at, output } => insert(doc, "outputs", *at, output)?,
        Edit::Replace { item, with } => {
            let node = find(doc, *item)?;
            doc.replaced(node.span.clone(), with.trim())
        }
        Edit::Remove { item } => doc.removed(find(doc, *item)?),
        Edit::Move { item, to } => moved(doc, *item, *to)?,
        Edit::Note { item, note } => doc.with_note(find(doc, *item)?, note),
    };
    parse(text.clone())?;
    Ok(text)
}

fn insert(doc: &Doc, list: &str, at: Option<usize>, item: &str) -> Result<String, String> {
    let seq = doc
        .root
        .field(list)
        .ok_or_else(|| format!("the recipe has no {list}"))?;
    let item = item.trim().trim_end_matches(',');
    Ok(match at {
        Some(at) => doc.inserted(seq, at, item),
        None => doc.appended(seq, item),
    })
}

/// The text with every change made in turn (each sees the recipe the ones before it left),
/// and the recipe it is. Fails at the first change that can't be made, or when the result
/// isn't a recipe, saying which.
pub fn apply_all(text: &str, edits: &[Edit]) -> Result<(String, Recipe), String> {
    let mut doc = parse(text.to_string())?;
    for (n, edit) in edits.iter().enumerate() {
        let text = apply(&doc, edit).map_err(|e| format!("change {}: {e}", n + 1))?;
        doc = parse(text)?;
    }
    let recipe = ron::from_str::<Recipe>(&doc.text).map_err(|e| e.to_string())?;
    Ok((doc.text, recipe))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = r#"Recipe(
    steps: [
        // == Tiles ==
        // The tiles, from the gfx table
        Read(name: "tiles", offset: 0x100, size: Some(64)),
        Read(name: "map", offset: 0x200), // the map
        Fill(name: "blank", size: 16),
    ],
    outputs: [],
)
"#;

    fn edited(edits: &[Edit]) -> String {
        apply_all(RECIPE, edits).unwrap().0
    }

    #[test]
    fn items_are_named_as_the_editor_names_them() {
        assert_eq!("step 3".parse(), Ok(Item::Step(3)));
        assert_eq!(" output 0 ".parse(), Ok(Item::Output(0)));
        assert!("stage 1".parse::<Item>().is_err());
        assert_eq!(Item::Output(2).to_string(), "output 2");
    }

    #[test]
    fn values_are_set_in_place_or_added() {
        let text = edited(&[
            Edit::Set {
                item: Item::Step(0),
                field: "offset".into(),
                value: "0x180".into(),
            },
            Edit::Set {
                item: Item::Step(1),
                field: "size".into(),
                value: "Some(32)".into(),
            },
        ]);
        assert!(text.contains("Read(name: \"tiles\", offset: 0x180, size: Some(64)),"));
        assert!(text.contains("offset: 0x200, size: Some(32))"));
        // Comments stay.
        assert!(text.contains("// The tiles, from the gfx table"));
        assert!(text.contains("// the map"));
    }

    #[test]
    fn steps_are_inserted_moved_and_removed_with_their_comments() {
        let text = edited(&[
            Edit::InsertStep {
                at: Some(1),
                step: r#"Fill(name: "pad", size: 4)"#.into(),
            },
            Edit::Move {
                item: Item::Step(2),
                to: 3,
            },
            Edit::Remove {
                item: Item::Step(0),
            },
        ]);
        let (_, recipe) = apply_all(&text, &[]).unwrap();
        assert_eq!(recipe.steps.len(), 3);
        assert!(text.contains("// == Tiles =="));
        let pad = text.find("\"pad\"").unwrap();
        let blank = text.find("\"blank\"").unwrap();
        let map = text.find("\"map\"").unwrap();
        assert!(pad < blank && blank < map, "{text}");
        assert!(text.contains("offset: 0x200), // the map"));
    }

    #[test]
    fn notes_and_outputs_are_written() {
        let text = edited(&[
            Edit::Note {
                item: Item::Step(2),
                note: "Room for the font".into(),
            },
            Edit::InsertOutput {
                at: None,
                output: r#"Image(id: "tiles", from: "tiles")"#.into(),
            },
            Edit::Replace {
                item: Item::Step(1),
                with: r#"Read(name: "map", offset: 0x220)"#.into(),
            },
        ]);
        assert!(text.contains("        // Room for the font\n        Fill("));
        assert!(text.contains("Image(id: \"tiles\", from: \"tiles\")"));
        assert!(text.contains("offset: 0x220), // the map"));
    }

    #[test]
    fn a_change_that_cannot_be_made_says_which() {
        let error = apply_all(
            RECIPE,
            &[
                Edit::Remove {
                    item: Item::Step(2),
                },
                Edit::Remove {
                    item: Item::Step(2),
                },
            ],
        )
        .unwrap_err();
        assert_eq!(error, "change 2: there is no step 2");
        // Text that isn't RON, and RON that isn't a recipe.
        assert!(
            apply_all(
                RECIPE,
                &[Edit::Set {
                    item: Item::Step(0),
                    field: "offset".into(),
                    value: "0x10 )".into(),
                }]
            )
            .is_err()
        );
        assert!(
            apply_all(
                RECIPE,
                &[Edit::Set {
                    item: Item::Step(0),
                    field: "offset".into(),
                    value: "\"here\"".into(),
                }]
            )
            .is_err()
        );
    }
}

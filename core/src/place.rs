//! Placements: where a game puts things, read from its own tables of records (the enemies of a
//! room, its doors, the rooms of an area), and the scenes made of them.
//!
//! A [`crate::recipe::Step::Places`] step reads records into a placement list; a
//! [`crate::recipe::Output::Scene`] turns lists into nodes the Workshop adds to its scene. Like
//! everything a recipe makes, placements come from the user's ROM when the recipe runs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::recipe::Field;

/// A record is used only if `field` holds one of `values`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub field: Field,
    pub values: Vec<i64>,
}

impl Match {
    pub fn holds(&self, record: &[u8]) -> bool {
        self.field
            .read(record)
            .is_some_and(|v| self.values.contains(&v))
    }
}

/// Text made from a record: each `{}` is replaced by the next of `fields`' values (decimal),
/// `{item}` by the output the placement shows and `{index}` by the record's number (from 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub text: String,
    #[serde(default)]
    pub fields: Vec<Field>,
}

impl Template {
    /// The text for `record`; `None` if the record is too short for a field.
    pub fn fill(&self, record: &[u8], item: Option<&str>, index: usize) -> Option<String> {
        let mut values = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            values.push(field.read(record)?);
        }
        let text = self
            .text
            .replace("{item}", item.unwrap_or(""))
            .replace("{index}", &index.to_string());
        let mut out = String::with_capacity(text.len());
        let mut values = values.into_iter();
        let mut rest = text.as_str();
        while let Some(at) = rest.find("{}") {
            out.push_str(&rest[..at]);
            if let Some(v) = values.next() {
                out.push_str(&v.to_string());
            }
            rest = &rest[at + 2..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// Which output a placement shows: the one `outputs` names for the value of `by` (the record's
/// number from 0 without it), else `other`. A record whose value names no output and has no
/// `other` is still placed, showing nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceItem {
    #[serde(default)]
    pub by: Option<Field>,
    pub outputs: BTreeMap<i64, String>,
    #[serde(default)]
    pub other: Option<String>,
}

impl PlaceItem {
    pub fn pick(&self, record: &[u8], index: usize) -> Option<&str> {
        let key = match &self.by {
            Some(field) => field.read(record)?,
            None => index as i64,
        };
        self.outputs
            .get(&key)
            .or(self.other.as_ref())
            .map(String::as_str)
    }
}

/// A door: walking into the placement takes the player to the node named `to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceDoor {
    pub to: Template,
    /// Meters.
    #[serde(default = "door_radius")]
    pub radius: f32,
    /// Seconds the screen fades out and in.
    #[serde(default = "door_fade")]
    pub fade: f32,
}

fn door_radius() -> f32 {
    0.6
}

fn door_fade() -> f32 {
    0.6
}

/// How far a placement reaches from its position (half width, half height in pixels), by the
/// value of `by`: a door's shape, say. Values not listed reach `other`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extent {
    pub by: Field,
    pub sizes: BTreeMap<i64, (u32, u32)>,
    #[serde(default)]
    pub other: (u32, u32),
}

/// One thing placed: a name, where (pixels, in the space of the records plus the step's
/// origin), what it shows and does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub name: String,
    /// The group node it goes under (made by the first placement that names it).
    pub group: Option<String>,
    /// An output of the same plugin.
    pub item: Option<String>,
    pub x: f32,
    pub y: f32,
    pub door: Option<Door>,
    /// What a MarkPlaces step writes into its cells.
    pub code: i64,
    /// Half width and height in pixels.
    pub half: (u32, u32),
}

/// A placement's door, its target name filled in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Door {
    pub to: String,
    pub radius: f32,
    pub fade: f32,
}

/// A scene a plugin provides: its nodes, positions in meters (x right, z down the map).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneTemplate {
    pub nodes: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub name: String,
    pub group: Option<String>,
    pub item: Option<String>,
    pub position: [f32; 3],
    pub door: Option<Door>,
}

impl SceneTemplate {
    /// Nodes for `placements`, `pixels_per_meter` pixels to a meter.
    pub fn from_placements<'a>(
        placements: impl IntoIterator<Item = &'a Placement>,
        pixels_per_meter: f32,
    ) -> Self {
        let scale = 1.0 / pixels_per_meter.max(f32::EPSILON);
        Self {
            nodes: placements
                .into_iter()
                .map(|p| SceneNode {
                    name: p.name.clone(),
                    group: p.group.clone(),
                    item: p.item.clone(),
                    position: [p.x * scale, 0.0, p.y * scale],
                    door: p.door.clone(),
                })
                .collect(),
        }
    }
}

/// Marks the cells each placement covers in a grid of `width` by `height` cells of `cell_size`
/// pixels, whose top left corner is at `origin` (pixels, in the placements' space), with the
/// placement's code. Placements outside the grid mark nothing.
pub fn mark(
    grid: &mut [u8],
    placements: &[Placement],
    (width, height): (u32, u32),
    cell_size: u32,
    origin: (i32, i32),
) {
    let cell = cell_size.max(1) as f32;
    for p in placements {
        let (x, y) = (p.x - origin.0 as f32, p.y - origin.1 as f32);
        let (hx, hy) = (p.half.0 as f32, p.half.1 as f32);
        let x0 = ((x - hx) / cell).floor().max(0.0) as i64;
        let y0 = ((y - hy) / cell).floor().max(0.0) as i64;
        // A placement with no extent marks the one cell it stands in.
        let x1 = (((x + hx) / cell).ceil() as i64 - 1).max(x0);
        let y1 = (((y + hy) / cell).ceil() as i64 - 1).max(y0);
        for cy in y0..=y1.min(height as i64 - 1) {
            for cx in x0..=x1.min(width as i64 - 1) {
                if let Some(c) = grid.get_mut((cy * width as i64 + cx) as usize) {
                    *c = p.code as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::FieldSize;

    #[test]
    fn templates_fill_fields_item_and_index() {
        let t = Template {
            text: "{item} to {}/{} #{index}".into(),
            fields: vec![Field::new(0, FieldSize::U8), Field::new(1, FieldSize::U16)],
        };
        assert_eq!(
            t.fill(&[3, 0x10, 0x01], Some("door"), 7).as_deref(),
            Some("door to 3/272 #7")
        );
        assert_eq!(t.fill(&[3], None, 0), None);
    }

    #[test]
    fn items_are_picked_by_field_or_index() {
        let by_field = PlaceItem {
            by: Some(Field::new(1, FieldSize::U8)),
            outputs: BTreeMap::from([(5, "slime".to_string())]),
            other: None,
        };
        assert_eq!(by_field.pick(&[0, 5], 0), Some("slime"));
        assert_eq!(by_field.pick(&[0, 6], 0), None);
        let by_index = PlaceItem {
            by: None,
            outputs: BTreeMap::from([(1, "room_b".to_string())]),
            other: Some("room".into()),
        };
        assert_eq!(by_index.pick(&[], 1), Some("room_b"));
        assert_eq!(by_index.pick(&[], 0), Some("room"));
    }

    #[test]
    fn marks_cells_covered() {
        let place = |x: f32, y: f32, half, code| Placement {
            name: String::new(),
            group: None,
            item: None,
            x,
            y,
            door: None,
            code,
            half,
        };
        let mut grid = vec![0u8; 4 * 3];
        mark(
            &mut grid,
            &[place(40.0, 24.0, (0, 0), 1), place(16.0, 16.0, (6, 6), 2)],
            (4, 3),
            16,
            (16, 0),
        );
        // (40, 24) less the origin is cell (1, 1); (16, 16) less the origin is (0, 16), and
        // ± 6 pixels around it covers column 0 of rows 0 and 1.
        assert_eq!(grid, vec![2, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0]);
    }
}

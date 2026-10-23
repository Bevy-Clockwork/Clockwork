//! Surface semantics: what a tile or collision surface does (DESIGN.md §4.4).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// A shared surface property. A surface is a set of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceProperty {
    Solid,
    Walkable,
    FallThrough,
    Swimmable,
    Shallow,
    Damaging(DamageKind),
    /// How much grip is lost, from 0.0 (none) to 1.0 (all).
    Slippery(f32),
    Climbable,
    Friction(f32),
    /// Can only be jumped down in this direction.
    Ledge(Direction),
    /// Only part of the cell is solid: a 2×2 mask, bit 0 = top left, 1 = top right,
    /// 2 = bottom left, 3 = bottom right.
    SolidParts(u8),
    /// A game-specific surface declared as a [`SurfaceExtension`]. Games that don't understand
    /// it use the extension's fallback.
    Extension(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageKind {
    Fire,
    Spikes,
    Electric,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    North,
    South,
    East,
    West,
}

/// A game-specific surface type, namespaced like `game::Portal`.
///
/// Games that don't understand the extension fall back to its shared properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceExtension {
    pub id: String,
    pub fallback: Vec<SurfaceProperty>,
}

impl SurfaceExtension {
    /// The namespace part of `id` (`game` in `game::Portal`), if it has one.
    pub fn namespace(&self) -> Option<&str> {
        self.id.split_once("::").map(|(ns, _)| ns)
    }
}

/// What one or more grids of game-specific codes mean, for a cell-based surface map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeMeaning {
    /// Properties for each code value.
    pub codes: BTreeMap<u16, Vec<SurfaceProperty>>,
    /// Properties for codes not listed. Without it, an unlisted code is an error, so no surface
    /// silently ends up with no meaning (DESIGN.md §4.4).
    #[serde(default)]
    pub other: Option<Vec<SurfaceProperty>>,
}

/// One kind of surface found in a [`SurfaceMap`]: its properties and the codes that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceKind {
    pub properties: Vec<SurfaceProperty>,
    /// The game-specific source of these properties: (grid name, code) per grid.
    pub codes: Vec<(String, u16)>,
}

/// Surfaces for a grid of cells, e.g. a 2D room's collision.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceMap {
    pub width: u32,
    pub height: u32,
    /// Size of one cell in the game's native units (e.g. 16 pixels).
    pub cell_size: u32,
    pub kinds: Vec<SurfaceKind>,
    /// Index into `kinds` for each cell, row-major.
    pub cells: Vec<u16>,
    pub extensions: Vec<SurfaceExtension>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SurfaceError {
    #[error("grid \"{grid}\": code {code:#x} has no meaning and there is no `other`")]
    Unmapped { grid: String, code: u16 },
    #[error("surface extension \"{0}\" is used but not declared")]
    UndeclaredExtension(String),
    #[error("grids have different sizes")]
    SizeMismatch,
}

impl SurfaceMap {
    /// Builds a map from named grids of codes and what each grid's codes mean. A cell's
    /// properties are the union of what its codes in every grid mean.
    pub fn build(
        width: u32,
        cell_size: u32,
        grids: &[(&str, &[u16], &CodeMeaning)],
        extensions: Vec<SurfaceExtension>,
    ) -> Result<Self, SurfaceError> {
        let len = grids.first().map_or(0, |(_, g, _)| g.len());
        if grids.iter().any(|(_, g, _)| g.len() != len) {
            return Err(SurfaceError::SizeMismatch);
        }

        let mut kinds: Vec<SurfaceKind> = Vec::new();
        let mut by_codes: HashMap<Vec<u16>, u16> = HashMap::new();
        let mut cells = Vec::with_capacity(len);
        for i in 0..len {
            let key: Vec<u16> = grids.iter().map(|(_, g, _)| g[i]).collect();
            if let Some(&kind) = by_codes.get(&key) {
                cells.push(kind);
                continue;
            }
            let mut properties = Vec::new();
            for ((name, _, meaning), &code) in grids.iter().zip(&key) {
                let props = meaning
                    .codes
                    .get(&code)
                    .or(meaning.other.as_ref())
                    .ok_or_else(|| SurfaceError::Unmapped {
                        grid: name.to_string(),
                        code,
                    })?;
                for p in props {
                    if let SurfaceProperty::Extension(id) = p
                        && !extensions.iter().any(|e| &e.id == id)
                    {
                        return Err(SurfaceError::UndeclaredExtension(id.clone()));
                    }
                    if !properties.contains(p) {
                        properties.push(p.clone());
                    }
                }
            }
            let kind = kinds.len() as u16;
            kinds.push(SurfaceKind {
                properties,
                codes: grids
                    .iter()
                    .map(|(n, _, _)| n.to_string())
                    .zip(key.iter().copied())
                    .collect(),
            });
            by_codes.insert(key, kind);
            cells.push(kind);
        }

        Ok(Self {
            width,
            height: (len as u32).div_ceil(width.max(1)),
            cell_size,
            kinds,
            cells,
            extensions,
        })
    }

    pub fn kind_at(&self, x: u32, y: u32) -> Option<&SurfaceKind> {
        if x >= self.width {
            return None;
        }
        let index = *self.cells.get((y * self.width + x) as usize)?;
        self.kinds.get(index as usize)
    }

    /// Properties with extensions replaced by their fallbacks, for consumers that only know the
    /// shared vocabulary.
    pub fn resolve(&self, properties: &[SurfaceProperty]) -> Vec<SurfaceProperty> {
        let mut out = Vec::new();
        for p in properties {
            let expanded = match p {
                SurfaceProperty::Extension(id) => self
                    .extensions
                    .iter()
                    .find(|e| &e.id == id)
                    .map(|e| e.fallback.clone())
                    .unwrap_or_default(),
                other => vec![other.clone()],
            };
            for p in expanded {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_surface_map_from_code_grids() {
        let collision = CodeMeaning {
            codes: BTreeMap::from([
                (0x0, vec![SurfaceProperty::Walkable]),
                (0xF, vec![SurfaceProperty::Solid]),
            ]),
            other: None,
        };
        let behavior = CodeMeaning {
            codes: BTreeMap::from([(
                0x3D,
                vec![SurfaceProperty::Extension("game::Portal".into())],
            )]),
            other: Some(vec![]),
        };
        let portal = SurfaceExtension {
            id: "game::Portal".into(),
            fallback: vec![SurfaceProperty::Walkable],
        };
        let map = SurfaceMap::build(
            2,
            16,
            &[
                ("collision", &[0x0, 0xF, 0x0, 0x0], &collision),
                ("behavior", &[0x0, 0x0, 0x3D, 0x0], &behavior),
            ],
            vec![portal],
        )
        .unwrap();

        assert_eq!((map.width, map.height), (2, 2));
        assert_eq!(map.kinds.len(), 3);
        assert_eq!(
            map.kind_at(1, 0).unwrap().properties,
            [SurfaceProperty::Solid]
        );
        let portal_cell = map.kind_at(0, 1).unwrap();
        assert_eq!(portal_cell.codes[1], ("behavior".to_string(), 0x3D));
        assert_eq!(
            map.resolve(&portal_cell.properties),
            [SurfaceProperty::Walkable]
        );
        assert_eq!(map.kind_at(1, 1), map.kind_at(0, 0));
    }

    #[test]
    fn unmapped_codes_are_errors() {
        let meaning = CodeMeaning {
            codes: BTreeMap::new(),
            other: None,
        };
        let err = SurfaceMap::build(1, 16, &[("collision", &[7], &meaning)], vec![]).unwrap_err();
        assert_eq!(
            err,
            SurfaceError::Unmapped {
                grid: "collision".into(),
                code: 7
            }
        );
    }

    #[test]
    fn parses_extension_from_ron() {
        let ext: SurfaceExtension =
            ron::from_str(r#"SurfaceExtension(id: "game::Portal", fallback: [Walkable])"#).unwrap();
        assert_eq!(ext.namespace(), Some("game"));
        assert_eq!(ext.fallback, [SurfaceProperty::Walkable]);
    }

    #[test]
    fn parses_combined_properties() {
        let lava: Vec<SurfaceProperty> = ron::from_str("[Damaging(Fire), FallThrough]").unwrap();
        assert_eq!(
            lava,
            [
                SurfaceProperty::Damaging(DamageKind::Fire),
                SurfaceProperty::FallThrough
            ]
        );
    }
}

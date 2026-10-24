//! Blocks: higher-level nodes. A block is a few steps and outputs that are usually written
//! together (a GBA tileset is tiles, a palette and metatiles into one image), added at once as a
//! closed group that shows the values worth changing. Once added they are plain steps: open the
//! group to change anything else.

use std::collections::HashSet;

use clockwork_core::recipe::Recipe;

/// A block of steps and outputs.
#[derive(Debug)]
pub struct Block {
    pub name: &'static str,
    pub about: &'static str,
    pub steps: &'static [&'static str],
    pub outputs: &'static [&'static str],
    /// The values its group shows: (step, field).
    pub shown: &'static [(usize, &'static str)],
}

pub const BLOCKS: &[Block] = &[
    Block {
        name: "GBA tileset",
        about: "Compressed 4bpp tiles, a palette and metatiles drawn into one image",
        steps: &[
            r#"Read(name: "tiles", offset: 0x0, codec: GbaLz77)"#,
            r#"Read(name: "palette", offset: 0x0, size: Some(512))"#,
            r#"GbaPalettes(from: "palette", bank: "bank", slot: 0)"#,
            r#"Read(name: "metatiles", offset: 0x0, size: Some(2048))"#,
            r#"GbaMetatiles(tiles: "tiles", char_base: 0x0, metatiles: "metatiles", bank: "bank", columns: 16, into: "tileset")"#,
        ],
        outputs: &[r#"Image(id: "tileset", from: "tileset")"#],
        shown: &[(0, "offset"), (1, "offset"), (3, "offset"), (4, "columns")],
    },
    Block {
        name: "GBA tile map",
        about: "A map of metatiles drawn with its tiles and palette, as one image",
        steps: &[
            r#"Read(name: "tiles", offset: 0x0, codec: GbaLz77)"#,
            r#"Read(name: "palette", offset: 0x0, size: Some(512))"#,
            r#"GbaPalettes(from: "palette", bank: "bank", slot: 0)"#,
            r#"Read(name: "metatiles", offset: 0x0, size: Some(2048))"#,
            r#"Read(name: "map", offset: 0x0, size: Some(512))"#,
            r#"GbaMetatileMap(tiles: "tiles", char_base: 0x0, metatiles: "metatiles", map: "map", width: 16, bank: "bank", into: "map_image")"#,
        ],
        outputs: &[r#"Image(id: "map_image", from: "map_image")"#],
        shown: &[
            (0, "offset"),
            (1, "offset"),
            (3, "offset"),
            (4, "offset"),
            (5, "width"),
        ],
    },
    Block {
        name: "GBA palette",
        about: "A palette read from the ROM into a bank, for tiles and sprites to use",
        steps: &[
            r#"Read(name: "palette", offset: 0x0, size: Some(32))"#,
            r#"GbaPalettes(from: "palette", bank: "bank", slot: 0)"#,
        ],
        outputs: &[],
        shown: &[(0, "offset"), (1, "slot")],
    },
    Block {
        name: "N64 texture",
        about: "Texture bytes read from the ROM and decoded into an image",
        steps: &[
            r#"Read(name: "texture_data", offset: 0x0, size: Some(2048))"#,
            r#"N64Texture(from: "texture_data", at: 0x0, format: Rgba16, width: 32, height: 32, into: "texture")"#,
        ],
        outputs: &[r#"Image(id: "texture", from: "texture")"#],
        shown: &[(0, "offset"), (1, "format"), (1, "width"), (1, "height")],
    },
];

impl Block {
    /// The recipe of just this block.
    pub fn recipe_text(&self) -> String {
        format!(
            "Recipe(steps: [{}], outputs: [{}])",
            self.steps.join(", "),
            self.outputs.join(", ")
        )
    }

    /// Its steps and outputs with every name it makes (and its output ids) given the same
    /// number, the lowest that makes none of them taken: `tiles_2`, `palette_2`, ...
    pub fn named(&self, taken: &HashSet<String>) -> (Vec<String>, Vec<String>) {
        let made = self.made_names();
        let n = (1..)
            .find(|n| made.iter().all(|m| !taken.contains(&format!("{m}_{n}"))))
            .unwrap_or(1);
        let rename = |item: &&str| {
            let mut text = item.to_string();
            for m in &made {
                text = text.replace(&format!("\"{m}\""), &format!("\"{m}_{n}\""));
            }
            text
        };
        (
            self.steps.iter().map(rename).collect(),
            self.outputs.iter().map(rename).collect(),
        )
    }

    /// The names its steps make and its output ids.
    fn made_names(&self) -> Vec<String> {
        let Ok(recipe) = ron::from_str::<Recipe>(&self.recipe_text()) else {
            return Vec::new();
        };
        let mut made: Vec<String> = recipe
            .steps
            .iter()
            .flat_map(|s| s.ports())
            .filter(|p| p.access.writes())
            .map(|p| p.name.to_string())
            .chain(recipe.outputs.iter().map(|o| o.id().to_string()))
            .collect();
        made.sort();
        made.dedup();
        made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_is_a_recipe_that_reads_only_what_it_makes() {
        for block in BLOCKS {
            let recipe: Recipe = ron::from_str(&block.recipe_text())
                .unwrap_or_else(|e| panic!("{}: {e}", block.name));
            let made = block.made_names();
            for step in &recipe.steps {
                for port in step.ports().iter().filter(|p| p.access.reads()) {
                    assert!(
                        made.contains(&port.name.to_string()),
                        "{}: {} is read but not made",
                        block.name,
                        port.name
                    );
                }
            }
            for (step, field) in block.shown {
                assert!(
                    block.steps[*step].contains(&format!("{field}:")),
                    "{}: step {step} has no {field}",
                    block.name
                );
            }
        }
    }

    #[test]
    fn a_block_added_twice_gets_new_names() {
        let block = &BLOCKS[0];
        let (steps, outputs) = block.named(&HashSet::new());
        assert!(steps[0].contains("\"tiles_1\""));
        assert!(outputs[0].contains("id: \"tileset_1\""));
        let taken: HashSet<String> = ["tiles_1".to_string()].into();
        let (steps, _) = block.named(&taken);
        assert!(steps[4].contains("tiles: \"tiles_2\""));
        assert!(steps[4].contains("bank: \"bank_2\""));
    }
}

//! Bitmap fonts: a sheet of glyphs cut from an image a recipe makes, in a grid, one glyph per
//! character of `chars` in reading order. HUDs and menus write with them in the game's own
//! letters.

use serde::{Deserialize, Serialize};

use crate::image::RgbaImage;

/// A font ready to write with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapFont {
    pub image: RgbaImage,
    /// Width and height of a glyph's cell, in pixels.
    pub glyph: (u32, u32),
    /// The characters of the cells, in reading order.
    pub chars: String,
    /// How far each glyph moves the pen, in pixels (by cell); the cell width otherwise.
    pub advance: Option<Vec<u8>>,
}

/// How a font's glyphs sit on its image, as a recipe's `Font` output writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontLayout {
    pub glyph: (u32, u32),
    pub chars: String,
    #[serde(default)]
    pub advance: Option<Vec<u8>>,
}

impl BitmapFont {
    /// Glyphs per row of the sheet.
    pub fn columns(&self) -> u32 {
        (self.image.width / self.glyph.0.max(1)).max(1)
    }

    pub fn rows(&self) -> u32 {
        (self.image.height / self.glyph.1.max(1)).max(1)
    }

    /// The cell of a character; lower case falls back to upper case and back.
    pub fn cell(&self, c: char) -> Option<usize> {
        let find = |c: char| self.chars.chars().position(|x| x == c);
        find(c)
            .or_else(|| find(c.to_ascii_uppercase()))
            .or_else(|| find(c.to_ascii_lowercase()))
            .filter(|i| (*i as u32) < self.columns() * self.rows())
    }

    /// How far a character moves the pen, in pixels.
    pub fn advance_of(&self, c: char) -> u32 {
        match self.cell(c) {
            Some(i) => self
                .advance
                .as_ref()
                .and_then(|a| a.get(i))
                .map_or(self.glyph.0, |a| *a as u32),
            None => self.glyph.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_find_their_cells_and_widths() {
        let font = BitmapFont {
            image: RgbaImage::new(32, 16),
            glyph: (8, 8),
            chars: "ABCDEFG".into(),
            advance: Some(vec![8, 6, 8, 8, 8, 8, 8]),
        };
        assert_eq!(font.columns(), 4);
        assert_eq!(font.cell('B'), Some(1));
        assert_eq!(font.cell('b'), Some(1));
        assert_eq!(font.cell('Z'), None);
        assert_eq!(font.advance_of('B'), 6);
        assert_eq!(font.advance_of(' '), 8);
    }

    #[test]
    fn case_fallback_prefers_the_original_character() {
        let font = BitmapFont {
            image: RgbaImage::new(32, 8),
            glyph: (8, 8),
            chars: "AaJq".into(),
            advance: Some(vec![8, 6, 7, 5]),
        };
        assert_eq!(font.cell('A'), Some(0));
        assert_eq!(font.cell('a'), Some(1));
        assert_eq!(font.cell('j'), Some(2));
        assert_eq!(font.cell('Q'), Some(3));
        assert_eq!(font.advance_of('Q'), 5);
        assert_eq!(font.cell('V'), None);
    }
}

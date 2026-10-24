//! GBA graphics: BGR555 palettes, 4bpp and 8bpp tiles and tile map entries.

use clockwork_core::image::RgbaImage;
pub use clockwork_core::recipe::TileDepth;

pub const TILE_SIZE: u32 = 8;
const TILE_BYTES_4BPP: usize = 32;
const TILE_BYTES_8BPP: usize = 64;

/// Converts a BGR555 color to RGBA, scaling 5-bit channels to the full 8-bit range.
pub fn bgr555_to_rgba(color: u16) -> [u8; 4] {
    let expand = |c: u16| ((c << 3) | (c >> 2)) as u8;
    [
        expand(color & 0x1F),
        expand((color >> 5) & 0x1F),
        expand((color >> 10) & 0x1F),
        0xFF,
    ]
}

/// Palette RAM for one layer (background or sprites): 16 palettes of 16 colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteBank {
    pub colors: [[u8; 4]; 256],
}

impl Default for PaletteBank {
    fn default() -> Self {
        Self {
            colors: [[0, 0, 0, 0xFF]; 256],
        }
    }
}

impl PaletteBank {
    /// Loads consecutive 16-color palettes from BGR555 `data`, starting at palette `slot`.
    pub fn load(&mut self, data: &[u8], slot: usize) {
        for (i, pair) in data.as_chunks::<2>().0.iter().enumerate() {
            let index = slot * 16 + i;
            if index >= 256 {
                break;
            }
            self.colors[index] = bgr555_to_rgba(u16::from_le_bytes([pair[0], pair[1]]));
        }
    }
}

/// One entry of a GBA text-mode tile map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileEntry {
    pub tile: u16,
    pub hflip: bool,
    pub vflip: bool,
    pub palette: u8,
}

impl TileEntry {
    pub fn from_u16(value: u16) -> Self {
        Self {
            tile: value & 0x3FF,
            hflip: value & 0x400 != 0,
            vflip: value & 0x800 != 0,
            palette: (value >> 12) as u8,
        }
    }
}

/// Draws one 4bpp tile from `tiles` (character data) at pixel (x, y).
///
/// Color 0 of each palette is transparent, as on the hardware. Tiles past the end of `tiles`
/// are skipped.
pub fn draw_tile_4bpp(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    tiles: &[u8],
    entry: TileEntry,
    palettes: &PaletteBank,
) {
    let start = entry.tile as usize * TILE_BYTES_4BPP;
    let Some(data) = tiles.get(start..start + TILE_BYTES_4BPP) else {
        return;
    };
    for py in 0..TILE_SIZE {
        for px in 0..TILE_SIZE {
            let byte = data[(py * 4 + px / 2) as usize];
            let index = if px % 2 == 0 { byte & 0xF } else { byte >> 4 };
            if index == 0 {
                continue;
            }
            let color = palettes.colors[entry.palette as usize * 16 + index as usize];
            let dx = if entry.hflip { TILE_SIZE - 1 - px } else { px };
            let dy = if entry.vflip { TILE_SIZE - 1 - py } else { py };
            image.set(x + dx, y + dy, color);
        }
    }
}

/// Draws one 8bpp tile from `tiles` at pixel (x, y): each byte is a color of the whole
/// 256-color palette, so the entry's palette number is ignored, as in 256-color mode.
pub fn draw_tile_8bpp(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    tiles: &[u8],
    entry: TileEntry,
    palettes: &PaletteBank,
) {
    let start = entry.tile as usize * TILE_BYTES_8BPP;
    let Some(data) = tiles.get(start..start + TILE_BYTES_8BPP) else {
        return;
    };
    for py in 0..TILE_SIZE {
        for px in 0..TILE_SIZE {
            let index = data[(py * TILE_SIZE + px) as usize];
            if index == 0 {
                continue;
            }
            let dx = if entry.hflip { TILE_SIZE - 1 - px } else { px };
            let dy = if entry.vflip { TILE_SIZE - 1 - py } else { py };
            image.set(x + dx, y + dy, palettes.colors[index as usize]);
        }
    }
}

/// Draws one tile of `depth` (see [`draw_tile_4bpp`] and [`draw_tile_8bpp`]).
pub fn draw_tile(
    image: &mut RgbaImage,
    (x, y): (u32, u32),
    tiles: &[u8],
    entry: TileEntry,
    palettes: &PaletteBank,
    depth: TileDepth,
) {
    match depth {
        TileDepth::Bpp4 => draw_tile_4bpp(image, x, y, tiles, entry, palettes),
        TileDepth::Bpp8 => draw_tile_8bpp(image, x, y, tiles, entry, palettes),
    }
}

/// Renders 16×16 metatiles (four tile entries each: top-left, top-right, bottom-left,
/// bottom-right) into a sheet `columns` metatiles wide.
pub fn render_metatiles(
    tiles: &[u8],
    metatiles: &[u8],
    palettes: &PaletteBank,
    columns: u32,
    depth: TileDepth,
) -> RgbaImage {
    let count = (metatiles.len() / 8) as u32;
    let rows = count.div_ceil(columns.max(1));
    let mut image = RgbaImage::new(columns * 16, rows * 16);
    for (i, meta) in metatiles.as_chunks::<8>().0.iter().enumerate() {
        let i = i as u32;
        let (mx, my) = ((i % columns) * 16, (i / columns) * 16);
        for (quarter, pair) in meta.as_chunks::<2>().0.iter().enumerate() {
            let entry = TileEntry::from_u16(u16::from_le_bytes([pair[0], pair[1]]));
            let q = quarter as u32;
            draw_tile(
                &mut image,
                (mx + (q % 2) * 8, my + (q / 2) * 8),
                tiles,
                entry,
                palettes,
                depth,
            );
        }
    }
    image
}

/// Renders a map of metatile numbers (little-endian u16, row-major, `width` metatiles wide)
/// using the metatile definitions in `metatiles`.
pub fn render_metatile_map(
    tiles: &[u8],
    metatiles: &[u8],
    map: &[u8],
    width: u32,
    palettes: &PaletteBank,
    depth: TileDepth,
) -> RgbaImage {
    let count = (map.len() / 2) as u32;
    let width = width.max(1);
    let height = count.div_ceil(width);
    let mut image = RgbaImage::new(width * 16, height * 16);
    for (i, pair) in map.as_chunks::<2>().0.iter().enumerate() {
        let i = i as u32;
        let meta = u16::from_le_bytes([pair[0], pair[1]]) as usize;
        let Some(meta) = metatiles.get(meta * 8..meta * 8 + 8) else {
            continue;
        };
        let (mx, my) = ((i % width) * 16, (i / width) * 16);
        for (quarter, pair) in meta.as_chunks::<2>().0.iter().enumerate() {
            let entry = TileEntry::from_u16(u16::from_le_bytes([pair[0], pair[1]]));
            let q = quarter as u32;
            draw_tile(
                &mut image,
                (mx + (q % 2) * 8, my + (q / 2) * 8),
                tiles,
                entry,
                palettes,
                depth,
            );
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_colors() {
        assert_eq!(bgr555_to_rgba(0x001F), [255, 0, 0, 255]);
        assert_eq!(bgr555_to_rgba(0x7C00), [0, 0, 255, 255]);
        assert_eq!(bgr555_to_rgba(0x7FFF), [255, 255, 255, 255]);
    }

    #[test]
    fn parses_tile_entries() {
        let e = TileEntry::from_u16(0b0011_1100_0000_0101);
        assert_eq!(
            e,
            TileEntry {
                tile: 5,
                hflip: true,
                vflip: true,
                palette: 3
            }
        );
    }

    #[test]
    fn draws_flipped_tile_with_transparency() {
        // Tile 0: first row is colors 1,2,0,0,0,0,0,0; everything else 0.
        let mut tiles = vec![0u8; 32];
        tiles[0] = 0x21;
        let mut palettes = PaletteBank::default();
        palettes.load(&[0, 0, 0x1F, 0, 0xE0, 0x03], 1); // palette 1: -, red, green
        let mut image = RgbaImage::new(8, 8);
        let entry = TileEntry {
            tile: 0,
            hflip: true,
            vflip: false,
            palette: 1,
        };
        draw_tile_4bpp(&mut image, 0, 0, &tiles, entry, &palettes);
        assert_eq!(image.get(7, 0), [255, 0, 0, 255]);
        assert_eq!(image.get(6, 0), [0, 255, 0, 255]);
        assert_eq!(image.get(0, 0)[3], 0);
    }

    #[test]
    fn draws_8bpp_tiles_from_the_whole_palette() {
        // Tile 1 (bytes 64..128): first row colors 0x11, 0x00, 0x21; the entry's palette is
        // ignored.
        let mut tiles = vec![0u8; 128];
        tiles[64] = 0x11;
        tiles[66] = 0x21;
        let mut palettes = PaletteBank::default();
        palettes.load(&[0, 0, 0x1F, 0], 1); // color 0x11: red
        palettes.load(&[0, 0, 0xE0, 0x03], 2); // color 0x21: green
        let mut image = RgbaImage::new(8, 8);
        let entry = TileEntry {
            tile: 1,
            hflip: false,
            vflip: false,
            palette: 5,
        };
        draw_tile(
            &mut image,
            (0, 0),
            &tiles,
            entry,
            &palettes,
            TileDepth::Bpp8,
        );
        assert_eq!(image.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(image.get(1, 0)[3], 0);
        assert_eq!(image.get(2, 0), [0, 255, 0, 255]);
    }
}

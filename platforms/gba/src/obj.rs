//! GBA objects (OAM sprites): rectangles of 4bpp tiles that games combine into one sprite frame.
//!
//! Objects use the 1D tile mapping: a w×h tile object reads w·h consecutive tiles, row by row.

use clockwork_core::image::RgbaImage;

use crate::gfx::{PaletteBank, TILE_SIZE, TileEntry, draw_tile_4bpp};

/// One hardware object, with its position relative to the sprite's origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object {
    pub x: i32,
    pub y: i32,
    /// The hardware's shape code: 0 square, 1 wide, 2 tall.
    pub shape: u8,
    /// The hardware's size code, 0..3.
    pub size: u8,
    pub hflip: bool,
    pub vflip: bool,
    /// First tile, counted from the frame's tile window.
    pub tile: u32,
    /// Palette slot.
    pub palette: u8,
}

/// Width and height in pixels of an object shape and size, or `None` for the unused shape 3.
pub fn object_size(shape: u8, size: u8) -> Option<(u32, u32)> {
    const SIZES: [[(u32, u32); 4]; 3] = [
        [(8, 8), (16, 16), (32, 32), (64, 64)],
        [(16, 8), (32, 8), (32, 16), (64, 32)],
        [(8, 16), (8, 32), (16, 32), (32, 64)],
    ];
    SIZES.get(shape as usize)?.get(size as usize).copied()
}

/// Draws objects into one image, the way the hardware layers them: the first object is on top.
///
/// `tiles` is the tile memory and `tile_base` the tile the frame's window starts at in it.
/// Returns the image, just big enough for all objects, and the position of the sprite's origin
/// in it. Objects with the unused shape are skipped; no objects give a 1×1 transparent image.
pub fn compose(
    objects: &[Object],
    tiles: &[u8],
    tile_base: u32,
    palettes: &PaletteBank,
) -> (RgbaImage, [i32; 2]) {
    let placed: Vec<(&Object, u32, u32)> = objects
        .iter()
        .filter_map(|o| object_size(o.shape, o.size).map(|(w, h)| (o, w, h)))
        .collect();
    if placed.is_empty() {
        return (RgbaImage::new(1, 1), [0, 0]);
    }
    let left = placed.iter().map(|(o, _, _)| o.x).min().unwrap_or(0);
    let top = placed.iter().map(|(o, _, _)| o.y).min().unwrap_or(0);
    let right = placed
        .iter()
        .map(|(o, w, _)| o.x + *w as i32)
        .max()
        .unwrap_or(0);
    let bottom = placed
        .iter()
        .map(|(o, _, h)| o.y + *h as i32)
        .max()
        .unwrap_or(0);
    let mut frame = RgbaImage::new((right - left) as u32, (bottom - top) as u32);
    for (object, w, h) in placed.into_iter().rev() {
        let mut part = RgbaImage::new(w, h);
        let (columns, rows) = (w / TILE_SIZE, h / TILE_SIZE);
        for ty in 0..rows {
            for tx in 0..columns {
                let tile = tile_base + object.tile + ty * columns + tx;
                let entry = TileEntry {
                    tile: tile.min(u16::MAX as u32) as u16,
                    hflip: object.hflip,
                    vflip: object.vflip,
                    palette: object.palette & 0xF,
                };
                let dx = if object.hflip { columns - 1 - tx } else { tx };
                let dy = if object.vflip { rows - 1 - ty } else { ty };
                draw_tile_4bpp(
                    &mut part,
                    dx * TILE_SIZE,
                    dy * TILE_SIZE,
                    tiles,
                    entry,
                    palettes,
                );
            }
        }
        frame.overlay(&part, (object.x - left) as u32, (object.y - top) as u32);
    }
    (frame, [-left, -top])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile whose pixels are all `color`.
    fn solid(color: u8) -> Vec<u8> {
        vec![color | (color << 4); 32]
    }

    fn palettes() -> PaletteBank {
        let mut bank = PaletteBank::default();
        // Palette 0: transparent, red, green.
        bank.load(&[0, 0, 0x1F, 0, 0xE0, 0x03], 0);
        bank
    }

    fn object(x: i32, y: i32, shape: u8, size: u8, tile: u32) -> Object {
        Object {
            x,
            y,
            shape,
            size,
            hflip: false,
            vflip: false,
            tile,
            palette: 0,
        }
    }

    #[test]
    fn sizes_follow_the_hardware_table() {
        assert_eq!(object_size(0, 1), Some((16, 16)));
        assert_eq!(object_size(1, 0), Some((16, 8)));
        assert_eq!(object_size(2, 3), Some((32, 64)));
        assert_eq!(object_size(3, 0), None);
    }

    #[test]
    fn frames_fit_their_objects_and_keep_the_origin() {
        let tiles = [solid(1), solid(2)].concat();
        // A red 8×8 at (-8, -4) and a green 8×8 at (4, 0).
        let objects = [object(-8, -4, 0, 0, 0), object(4, 0, 0, 0, 1)];
        let (image, origin) = compose(&objects, &tiles, 0, &palettes());
        assert_eq!((image.width, image.height), (20, 12));
        assert_eq!(origin, [8, 4]);
        assert_eq!(image.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(image.get(12, 4), [0, 255, 0, 255]);
        assert_eq!(image.get(10, 0)[3], 0);
    }

    #[test]
    fn the_first_object_is_on_top() {
        let tiles = [solid(1), solid(2)].concat();
        let objects = [object(0, 0, 0, 0, 0), object(0, 0, 0, 0, 1)];
        let (image, _) = compose(&objects, &tiles, 0, &palettes());
        assert_eq!(image.get(3, 3), [255, 0, 0, 255]);
    }

    #[test]
    fn tile_windows_and_one_dimensional_mapping() {
        // Four tiles, colors 0 (unused), 1, 2, 1; a 16×8 object reads two consecutive tiles from
        // the window that starts at tile 1.
        let tiles = [solid(0), solid(1), solid(2), solid(1)].concat();
        let (image, _) = compose(&[object(0, 0, 1, 0, 0)], &tiles, 1, &palettes());
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(image.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(image.get(8, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn flips_mirror_the_whole_object() {
        let tiles = [solid(1), solid(2)].concat();
        let mut wide = object(0, 0, 1, 0, 0);
        wide.hflip = true;
        let (image, _) = compose(&[wide], &tiles, 0, &palettes());
        // Tile 0 (red) now sits on the right, tile 1 (green) on the left.
        assert_eq!(image.get(0, 0), [0, 255, 0, 255]);
        assert_eq!(image.get(15, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn no_objects_give_an_empty_frame() {
        let (image, origin) = compose(&[], &[], 0, &palettes());
        assert_eq!((image.width, image.height, origin), (1, 1, [0, 0]));
        assert_eq!(image.get(0, 0)[3], 0);
    }
}

//! A plain RGBA image, the common output of graphics decoders and recipes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, 4 bytes per pixel.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[i..i + 4].try_into().unwrap()
    }

    /// Sets a pixel; coordinates outside the image are ignored.
    pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x < self.width && y < self.height {
            let i = (y as usize * self.width as usize + x as usize) * 4;
            self.pixels[i..i + 4].copy_from_slice(&rgba);
        }
    }

    /// A copy of the rectangle (x, y, w, h), clipped to the image.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        let mut out = RgbaImage::new(w, h);
        for oy in 0..h {
            let src = ((y + oy) * self.width + x) as usize * 4;
            let dst = (oy * w) as usize * 4;
            out.pixels[dst..dst + w as usize * 4]
                .copy_from_slice(&self.pixels[src..src + w as usize * 4]);
        }
        out
    }

    /// A mirrored copy.
    pub fn flipped(&self, horizontal: bool, vertical: bool) -> RgbaImage {
        let mut out = RgbaImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let sx = if horizontal { self.width - 1 - x } else { x };
                let sy = if vertical { self.height - 1 - y } else { y };
                out.set(x, y, self.get(sx, sy));
            }
        }
        out
    }

    /// Copies `other` onto this image at (x, y), including transparent pixels.
    pub fn paste(&mut self, other: &RgbaImage, x: u32, y: u32) {
        for oy in 0..other.height {
            for ox in 0..other.width {
                self.set(x + ox, y + oy, other.get(ox, oy));
            }
        }
    }

    /// Draws `other` on top of this image at (x, y), skipping fully transparent pixels.
    pub fn overlay(&mut self, other: &RgbaImage, x: u32, y: u32) {
        for oy in 0..other.height {
            for ox in 0..other.width {
                let p = other.get(ox, oy);
                if p[3] != 0 {
                    self.set(x + ox, y + oy, p);
                }
            }
        }
    }
}

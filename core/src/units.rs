//! Unit normalization (DESIGN.md §4.3). Clockwork measures everything in meters.

use serde::{Deserialize, Serialize};

/// How many meters one of a game's native units is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NativeScale {
    pub meters_per_unit: f32,
}

impl NativeScale {
    /// A pixel-based 2D game where one tile of `tile_px` pixels is one meter.
    pub fn tiles(tile_px: f32) -> Self {
        Self {
            meters_per_unit: 1.0 / tile_px,
        }
    }

    pub fn to_meters(self, native: f32) -> f32 {
        native * self.meters_per_unit
    }

    pub fn from_meters(self, meters: f32) -> f32 {
        meters / self.meters_per_unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_pixel_tiles() {
        let scale = NativeScale::tiles(16.0);
        assert_eq!(scale.to_meters(32.0), 2.0);
        assert_eq!(scale.from_meters(0.5), 8.0);
    }
}

//! Background layers and animated tiles of 2D areas (DESIGN.md §7.10.3).
//!
//! 2D consoles draw an area as several backgrounds: a far one that scrolls slower than the
//! camera, the map itself, and parts drawn over the characters (treetops, roofs). Tiles that
//! move (water, lava, flowers) are graphics the game swaps every few frames. Both are data here:
//! an area's [`crate::recipe::Output::Image`] lists its layers (images with a draw order and a
//! scroll factor) and the frames it cycles through, all made by the recipe's steps. Animated
//! tiles that cover little of a large area are patches: frames the size of the part that moves,
//! drawn at its place over the area, each patch with its own timing, so a big map with some
//! water doesn't need a copy of the whole map for every frame.

use serde::{Deserialize, Serialize};

use crate::image::RgbaImage;

/// A layer of an area, as a recipe writes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// The image (a step's `into`).
    pub from: String,
    /// Draw order: the area's own image is 0, higher draws over lower. Layers of the same
    /// order draw after the area's image, in the order listed.
    #[serde(default)]
    pub order: i32,
    /// How far it moves for each meter the camera moves, across and down: 1.0 moves with the
    /// map, 0.25 is a far background, 0.0 stays on screen.
    #[serde(default = "full_scroll", skip_serializing_if = "is_full_scroll")]
    pub scroll: (f32, f32),
    /// Repeats the image across the whole area (backgrounds that wrap around).
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrap: bool,
    /// Drawn over characters and objects.
    #[serde(default, skip_serializing_if = "is_false")]
    pub over_sprites: bool,
    /// Images it cycles through in place of `from`, for animated tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<FramesSpec>,
}

/// Images an area or layer cycles through, as a recipe writes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramesSpec {
    /// The images (steps' `into`), each the size of the image they replace.
    pub images: Vec<String>,
    /// How long each shows, in ticks: one number per image, or one for all.
    pub ticks: Vec<u32>,
    /// Ticks per second (the game's update rate).
    pub fps: f32,
}

/// A part of an area that animates on its own, as a recipe writes it: frames smaller than the
/// area, drawn with their top left corner at `at` (pixels) over the area's image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSpec {
    pub at: (u32, u32),
    /// The images, all the same size, and their timing.
    pub frames: FramesSpec,
}

fn full_scroll() -> (f32, f32) {
    (1.0, 1.0)
}

fn is_full_scroll(scroll: &(f32, f32)) -> bool {
    *scroll == full_scroll()
}

fn is_false(value: &bool) -> bool {
    !value
}

impl FramesSpec {
    /// What is wrong with the timing, if anything.
    pub fn problem(&self) -> Option<String> {
        if self.images.is_empty() {
            return Some("frames need at least one image".into());
        }
        if self.ticks.len() != 1 && self.ticks.len() != self.images.len() {
            return Some(format!(
                "frames have {} images but {} ticks (give one per image, or one for all)",
                self.images.len(),
                self.ticks.len()
            ));
        }
        if self.ticks.contains(&0) {
            return Some("a frame lasts 0 ticks".into());
        }
        if !(self.fps.is_finite() && self.fps > 0.0) {
            return Some(format!("frames need fps above 0, not {}", self.fps));
        }
        None
    }
}

impl PatchSpec {
    /// What is wrong with its timing, if anything.
    pub fn problem(&self) -> Option<String> {
        let frames = self.frames.problem()?;
        Some(format!("the patch at {:?}: {frames}", self.at))
    }
}

impl LayerSpec {
    /// What is wrong with it, other than its names, if anything.
    pub fn problem(&self) -> Option<String> {
        if !(self.scroll.0.is_finite() && self.scroll.1.is_finite()) {
            return Some(format!(
                "layer \"{}\" has a scroll that isn't a number",
                self.from
            ));
        }
        let frames = self.frames.as_ref()?.problem()?;
        Some(format!("layer \"{}\": {frames}", self.from))
    }
}

/// Images shown one after another, each for its ticks, looping.
#[derive(Debug, Clone, PartialEq)]
pub struct Frames {
    pub images: Vec<RgbaImage>,
    /// One per image, or one for all.
    pub ticks: Vec<u32>,
    pub fps: f32,
}

impl Frames {
    /// When each frame shows, without the images.
    pub fn timing(&self) -> FrameTiming {
        FrameTiming {
            count: self.images.len(),
            ticks: self.ticks.clone(),
            fps: self.fps,
        }
    }

    /// How many ticks frame `i` shows.
    pub fn ticks_of(&self, i: usize) -> u32 {
        self.timing().ticks_of(i)
    }

    /// The frame shown `seconds` after the area appears.
    pub fn index_at(&self, seconds: f32) -> usize {
        self.timing().index_at(seconds)
    }
}

/// When each of `count` frames shows: for its ticks (one per frame, or one for all) at `fps`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameTiming {
    pub count: usize,
    pub ticks: Vec<u32>,
    pub fps: f32,
}

impl FrameTiming {
    /// How many ticks frame `i` shows.
    pub fn ticks_of(&self, i: usize) -> u32 {
        let ticks = self.ticks.get(i).or(self.ticks.first());
        ticks.copied().unwrap_or(1).max(1)
    }

    /// The frame shown `seconds` after the area appears.
    pub fn index_at(&self, seconds: f32) -> usize {
        let total: u64 = (0..self.count).map(|i| u64::from(self.ticks_of(i))).sum();
        if total == 0 || self.fps.is_nan() || self.fps <= 0.0 {
            return 0;
        }
        let mut tick = ((seconds.max(0.0) * self.fps) as u64) % total;
        for i in 0..self.count {
            let ticks = u64::from(self.ticks_of(i));
            if tick < ticks {
                return i;
            }
            tick -= ticks;
        }
        0
    }
}

/// A layer of an area, as extracted.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub image: RgbaImage,
    pub order: i32,
    pub scroll: (f32, f32),
    pub wrap: bool,
    pub over_sprites: bool,
    pub frames: Option<Frames>,
}

impl Layer {
    /// How far the layer is moved from where it lies on the area (in the unit `view` is in)
    /// when the view's centre is `view` from the area's top left corner: it moves with the
    /// camera by what its scroll leaves out. A layer that scrolls fully stays put.
    pub fn shift(&self, view: (f32, f32)) -> (f32, f32) {
        (
            (1.0 - self.scroll.0) * view.0,
            (1.0 - self.scroll.1) * view.1,
        )
    }
}

/// A part of an area that animates on its own, as extracted: its frames drawn with their top
/// left corner at `at` (pixels) over the area's image.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub at: (u32, u32),
    pub frames: Frames,
}

impl Patch {
    /// Its size in pixels (every frame's).
    pub fn size(&self) -> (u32, u32) {
        self.frames
            .images
            .first()
            .map_or((0, 0), |i| (i.width, i.height))
    }
}

/// An area's layers and animation, as extracted, besides its image.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
    /// Images the area's own image cycles through.
    pub frames: Option<Frames>,
    pub layers: Vec<Layer>,
    /// Parts of the area's image that animate on their own, drawn over it.
    pub patches: Vec<Patch>,
}

impl Layers {
    /// The order to draw in, back to front: `None` is the area's own image. Layers over sprites
    /// come last whatever their order.
    pub fn draw_order(&self) -> Vec<Option<usize>> {
        let mut all: Vec<(bool, i32, isize, Option<usize>)> = vec![(false, 0, -1, None)];
        for (i, layer) in self.layers.iter().enumerate() {
            all.push((layer.over_sprites, layer.order, i as isize, Some(i)));
        }
        all.sort_by_key(|(over, order, listed, _)| (*over, *order, *listed));
        all.into_iter().map(|(.., which)| which).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(ticks: Vec<u32>, count: usize) -> Frames {
        Frames {
            images: vec![RgbaImage::new(1, 1); count],
            ticks,
            fps: 60.0,
        }
    }

    fn layer(order: i32, over_sprites: bool) -> Layer {
        Layer {
            image: RgbaImage::new(1, 1),
            order,
            scroll: (0.25, 0.5),
            wrap: false,
            over_sprites,
            frames: None,
        }
    }

    #[test]
    fn frames_follow_their_ticks_and_loop() {
        let f = frames(vec![30, 15, 15], 3);
        assert_eq!(f.index_at(0.0), 0);
        assert_eq!(f.index_at(0.49), 0);
        assert_eq!(f.index_at(0.5), 1);
        assert_eq!(f.index_at(0.75), 2);
        assert_eq!(f.index_at(1.0), 0);
        assert_eq!(f.index_at(1.6), 1);
        // One number for every frame.
        let even = frames(vec![6], 4);
        assert_eq!(even.index_at(0.1), 1);
        assert_eq!(even.index_at(0.35), 3);
        assert_eq!(even.index_at(0.45), 0);
    }

    #[test]
    fn layers_shift_by_what_their_scroll_leaves_out() {
        let far = layer(-1, false);
        assert_eq!(far.shift((100.0, 40.0)), (75.0, 20.0));
        let map = Layer {
            scroll: (1.0, 1.0),
            ..far
        };
        assert_eq!(map.shift((100.0, 40.0)), (0.0, 0.0));
    }

    #[test]
    fn layers_draw_by_order_with_foregrounds_last() {
        let layers = Layers {
            frames: None,
            patches: Vec::new(),
            layers: vec![
                layer(1, true),
                layer(-2, false),
                layer(0, false),
                layer(-5, true),
                layer(-1, false),
            ],
        };
        assert_eq!(
            layers.draw_order(),
            vec![Some(1), Some(4), None, Some(2), Some(3), Some(0)]
        );
    }

    #[test]
    fn specs_read_with_defaults_and_report_bad_timing() {
        let spec: LayerSpec =
            ron::from_str(r#"(from: "sky", order: -1, scroll: (0.25, 0.0))"#).unwrap();
        assert_eq!(spec.scroll, (0.25, 0.0));
        assert!(!spec.wrap && !spec.over_sprites && spec.frames.is_none());
        assert_eq!(spec.problem(), None);
        let written = ron::to_string(&spec).unwrap();
        assert_eq!(ron::from_str::<LayerSpec>(&written).unwrap(), spec);

        let plain: LayerSpec = ron::from_str(r#"(from: "roofs", over_sprites: true)"#).unwrap();
        assert_eq!((plain.order, plain.scroll), (0, (1.0, 1.0)));

        let bad = FramesSpec {
            images: vec!["a".into(), "b".into()],
            ticks: vec![8, 8, 8],
            fps: 60.0,
        };
        assert!(bad.problem().unwrap().contains("2 images but 3 ticks"));
        let zero = FramesSpec {
            ticks: vec![0],
            ..bad.clone()
        };
        assert!(zero.problem().is_some());
        let still = FramesSpec {
            fps: 0.0,
            ticks: vec![8],
            ..bad
        };
        assert!(still.problem().is_some());
    }
}

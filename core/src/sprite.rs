//! Sprites: flat, pre-drawn frames and the animations that play them. The common output of 2D
//! graphics decoders and recipes, as [`crate::model`] is for 3D.
//!
//! A frame is an image plus an origin: the pixel that sits at the sprite's position in the world
//! (usually between the feet). Animations are lists of timed steps over the frames, with
//! optional mirroring, so a side view can serve both directions.

use crate::image::RgbaImage;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sprite {
    pub frames: Vec<SpriteFrame>,
    pub animations: Vec<SpriteAnimation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteFrame {
    pub image: RgbaImage,
    /// The pixel of `image` that is the sprite's position, counted from the top left. May lie
    /// outside the image.
    pub origin: [i32; 2],
}

/// Timed steps over a sprite's frames.
#[derive(Debug, Clone, PartialEq)]
pub struct SpriteAnimation {
    pub name: String,
    pub steps: Vec<AnimationStep>,
    /// Ticks per second: the game's own update rate, in which `AnimationStep::ticks` is counted.
    pub fps: f32,
    /// After the last step, playback continues from this step; `None` stops on the last one.
    pub loop_start: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationStep {
    /// Index into [`Sprite::frames`].
    pub frame: usize,
    /// How long the step lasts, in ticks.
    pub ticks: u32,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl SpriteFrame {
    /// Frames drawn over each other with their origins lined up, the first at the bottom: a
    /// character with what it holds. The result is just big enough for all of them.
    pub fn stack(frames: &[&SpriteFrame]) -> SpriteFrame {
        let left = frames.iter().map(|f| f.origin[0]).max().unwrap_or(0);
        let up = frames.iter().map(|f| f.origin[1]).max().unwrap_or(0);
        let right = frames
            .iter()
            .map(|f| f.image.width as i32 - f.origin[0])
            .max()
            .unwrap_or(0);
        let down = frames
            .iter()
            .map(|f| f.image.height as i32 - f.origin[1])
            .max()
            .unwrap_or(0);
        let mut image = RgbaImage::new((left + right).max(1) as u32, (up + down).max(1) as u32);
        for frame in frames {
            let x = left - frame.origin[0];
            let y = up - frame.origin[1];
            image.overlay(&frame.image, x.max(0) as u32, y.max(0) as u32);
        }
        SpriteFrame {
            image,
            origin: [left, up],
        }
    }

    /// The frame mirrored about its origin, which is a pixel corner: a frame that extends right
    /// of its origin extends left of it afterwards.
    pub fn flipped(&self, horizontal: bool, vertical: bool) -> SpriteFrame {
        let [ox, oy] = self.origin;
        SpriteFrame {
            image: self.image.flipped(horizontal, vertical),
            origin: [
                if horizontal {
                    self.image.width as i32 - ox
                } else {
                    ox
                },
                if vertical {
                    self.image.height as i32 - oy
                } else {
                    oy
                },
            ],
        }
    }
}

impl Sprite {
    pub fn animation(&self, name: &str) -> Option<&SpriteAnimation> {
        self.animations.iter().find(|a| a.name == name)
    }
}

impl SpriteAnimation {
    /// The step playing `seconds` after the animation started, and whether the animation has
    /// played through at least once (a looping one keeps going).
    pub fn step_at(&self, seconds: f32) -> Option<(&AnimationStep, bool)> {
        let last = self.steps.len().checked_sub(1)?;
        let mut ticks = (seconds.max(0.0) * self.fps) as u64;
        let total: u64 = self.steps.iter().map(|s| s.ticks.max(1) as u64).sum();
        let mut finished = false;
        if ticks >= total {
            finished = true;
            match self.loop_start.filter(|&start| start <= last) {
                None => return Some((&self.steps[last], true)),
                Some(start) => {
                    let prefix: u64 = self.steps[..start]
                        .iter()
                        .map(|s| s.ticks.max(1) as u64)
                        .sum();
                    let cycle = (total - prefix).max(1);
                    ticks = prefix + (ticks - total) % cycle;
                }
            }
        }
        for step in &self.steps {
            let length = step.ticks.max(1) as u64;
            if ticks < length {
                return Some((step, finished));
            }
            ticks -= length;
        }
        Some((&self.steps[last], finished))
    }

    /// One pass through the animation, in seconds.
    pub fn duration(&self) -> f32 {
        let ticks: u32 = self.steps.iter().map(|s| s.ticks.max(1)).sum();
        ticks as f32 / self.fps.max(f32::EPSILON)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(frame: usize, ticks: u32) -> AnimationStep {
        AnimationStep {
            frame,
            ticks,
            flip_x: false,
            flip_y: false,
        }
    }

    fn animation(loop_start: Option<usize>) -> SpriteAnimation {
        SpriteAnimation {
            name: "walk".into(),
            steps: vec![step(0, 2), step(1, 3), step(2, 1)],
            fps: 10.0,
            loop_start,
        }
    }

    fn frame_at(a: &SpriteAnimation, seconds: f32) -> (usize, bool) {
        let (step, finished) = a.step_at(seconds).unwrap();
        (step.frame, finished)
    }

    #[test]
    fn steps_follow_their_durations() {
        let a = animation(Some(0));
        assert_eq!(frame_at(&a, 0.0), (0, false));
        assert_eq!(frame_at(&a, 0.19), (0, false));
        assert_eq!(frame_at(&a, 0.2), (1, false));
        assert_eq!(frame_at(&a, 0.5), (2, false));
    }

    #[test]
    fn loops_from_the_loop_step() {
        let a = animation(Some(1));
        // One pass is 6 ticks; afterwards steps 1 and 2 repeat (4 ticks).
        assert_eq!(frame_at(&a, 0.6), (1, true));
        assert_eq!(frame_at(&a, 0.9), (2, true));
        assert_eq!(frame_at(&a, 1.0), (1, true));
        let whole = animation(Some(0));
        assert_eq!(frame_at(&whole, 0.6), (0, true));
    }

    #[test]
    fn stops_without_a_loop() {
        let a = animation(None);
        assert_eq!(frame_at(&a, 5.0), (2, true));
        assert!((a.duration() - 0.6).abs() < 1e-6);
    }

    #[test]
    fn stacking_lines_up_origins() {
        let mut body = RgbaImage::new(2, 2);
        body.set(0, 0, [1, 0, 0, 255]);
        let mut sword = RgbaImage::new(3, 1);
        sword.set(2, 0, [2, 0, 0, 255]);
        let body = SpriteFrame {
            image: body,
            origin: [1, 2],
        };
        // The sword reaches 2 px right of the origin, one row above it.
        let sword = SpriteFrame {
            image: sword,
            origin: [0, 1],
        };
        let both = SpriteFrame::stack(&[&body, &sword]);
        assert_eq!(both.origin, [1, 2]);
        assert_eq!((both.image.width, both.image.height), (4, 2));
        assert_eq!(both.image.get(0, 0), [1, 0, 0, 255]);
        assert_eq!(both.image.get(3, 1), [2, 0, 0, 255]);
    }

    #[test]
    fn flipping_mirrors_about_the_origin() {
        let mut image = RgbaImage::new(3, 1);
        image.set(0, 0, [1, 0, 0, 255]);
        let frame = SpriteFrame {
            image,
            origin: [1, 0],
        };
        let flipped = frame.flipped(true, false);
        assert_eq!(flipped.image.get(2, 0), [1, 0, 0, 255]);
        assert_eq!(flipped.origin, [2, 0]);
    }

    #[test]
    fn empty_animations_show_nothing() {
        let a = SpriteAnimation {
            steps: vec![],
            ..animation(None)
        };
        assert!(a.step_at(1.0).is_none());
    }
}

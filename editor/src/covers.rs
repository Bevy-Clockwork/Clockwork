//! Cover art for Library entries.
//!
//! A cover is either downloaded box art (see `boxart`) or an image the user picked. Either way
//! it's stored as `covers/<sha1>.png` in Clockwork's data folder. Games without one get a
//! generated cover. `covers/<sha1>.none` marks games that should keep the generated cover, so
//! box art isn't fetched for them again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use bevy_egui::egui::{
    self, Align, Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions,
    text::LayoutJob, vec2,
};
use clockwork_library::LibraryEntry;

use crate::{store, theme};

/// Covers are scaled down to this height when imported.
const MAX_HEIGHT: u32 = 600;

#[derive(Resource, Default)]
pub struct Covers {
    /// `None` means we looked and there is no custom cover.
    loaded: HashMap<String, Option<TextureHandle>>,
}

impl Covers {
    pub fn get(&mut self, ctx: &egui::Context, sha1: &str) -> Option<TextureHandle> {
        self.loaded
            .entry(sha1.to_string())
            .or_insert_with(|| {
                let path = cover_path(sha1)?;
                let image = image::open(path).ok()?;
                Some(upload(ctx, sha1, &image))
            })
            .clone()
    }

    /// Copies `source` into the covers folder as this game's cover.
    pub fn set(&mut self, ctx: &egui::Context, sha1: &str, source: &Path) -> Result<(), String> {
        let bytes = std::fs::read(source).map_err(|e| format!("could not read image: {e}"))?;
        self.set_bytes(ctx, sha1, &bytes)
    }

    /// Stores an encoded image (PNG, JPEG, WebP) as this game's cover.
    pub fn set_bytes(
        &mut self,
        ctx: &egui::Context,
        sha1: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let image =
            image::load_from_memory(bytes).map_err(|e| format!("could not read image: {e}"))?;
        let image = if image.height() > MAX_HEIGHT {
            image.resize(u32::MAX, MAX_HEIGHT, image::imageops::FilterType::Lanczos3)
        } else {
            image
        };
        let path = cover_path(sha1).ok_or("no data directory for this user")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        image.save(&path).map_err(|e| e.to_string())?;
        if let Some(marker) = marker_path(sha1) {
            let _ = std::fs::remove_file(marker);
        }
        self.loaded
            .insert(sha1.to_string(), Some(upload(ctx, sha1, &image)));
        Ok(())
    }

    /// Removes the cover and keeps the generated one from now on.
    pub fn reset(&mut self, sha1: &str) {
        if let Some(path) = cover_path(sha1) {
            let _ = std::fs::remove_file(path);
        }
        self.mark_no_boxart(sha1);
        self.loaded.insert(sha1.to_string(), None);
    }

    /// Whether this game should get box art fetched for it.
    pub fn wants_boxart(&self, sha1: &str) -> bool {
        let exists = |p: Option<PathBuf>| p.is_some_and(|p| p.exists());
        !exists(cover_path(sha1)) && !exists(marker_path(sha1))
    }

    pub fn mark_no_boxart(&self, sha1: &str) {
        if let Some(marker) = marker_path(sha1) {
            if let Some(dir) = marker.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(marker, "");
        }
    }

    /// Allows box art to be fetched again for a game that was marked.
    pub fn unmark_no_boxart(&self, sha1: &str) {
        if let Some(marker) = marker_path(sha1) {
            let _ = std::fs::remove_file(marker);
        }
    }
}

fn cover_path(sha1: &str) -> Option<PathBuf> {
    store::data_dir().map(|dir| dir.join("covers").join(format!("{sha1}.png")))
}

fn marker_path(sha1: &str) -> Option<PathBuf> {
    store::data_dir().map(|dir| dir.join("covers").join(format!("{sha1}.none")))
}

fn upload(ctx: &egui::Context, sha1: &str, image: &image::DynamicImage) -> TextureHandle {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture(format!("cover-{sha1}"), color, TextureOptions::LINEAR)
}

/// Paints a cover into `rect`: the custom image cropped to fill, or a generated one.
pub fn paint(
    painter: &Painter,
    rect: Rect,
    entry: &LibraryEntry,
    title: &str,
    texture: Option<&TextureHandle>,
    gear_spin: f32,
) {
    match texture {
        Some(tex) => {
            painter.image(
                tex.id(),
                rect,
                fill_uv(tex.size_vec2(), rect.size()),
                Color32::WHITE,
            );
        }
        None => generated(painter, rect, entry, title, gear_spin),
    }
}

/// Width / height of a game's cover: the image's own proportions, or the usual box shape for the
/// platform (GBA boxes are about square, N64 boxes wider than tall).
pub fn aspect(entry: &LibraryEntry, texture: Option<&TextureHandle>) -> f32 {
    let aspect = match texture {
        Some(tex) => {
            let size = tex.size_vec2();
            size.x / size.y
        }
        None => match entry.info.platform {
            "n64" => 1.4,
            _ => 1.0,
        },
    };
    aspect.clamp(0.6, 1.6)
}

/// UV rect that crops the image to fill the target without stretching.
fn fill_uv(image: egui::Vec2, target: egui::Vec2) -> Rect {
    let image_aspect = image.x / image.y;
    let target_aspect = target.x / target.y;
    if image_aspect > target_aspect {
        let w = target_aspect / image_aspect;
        Rect::from_min_max(
            egui::pos2((1.0 - w) / 2.0, 0.0),
            egui::pos2((1.0 + w) / 2.0, 1.0),
        )
    } else {
        let h = image_aspect / target_aspect;
        Rect::from_min_max(
            egui::pos2(0.0, (1.0 - h) / 2.0),
            egui::pos2(1.0, (1.0 + h) / 2.0),
        )
    }
}

fn generated(painter: &Painter, rect: Rect, entry: &LibraryEntry, title: &str, gear_spin: f32) {
    let (top, bottom) = match entry.info.platform {
        "gba" => (
            Color32::from_rgb(0x4A, 0x22, 0x2A),
            Color32::from_rgb(0x1C, 0x0E, 0x12),
        ),
        "n64" => (
            Color32::from_rgb(0x1E, 0x40, 0x3B),
            Color32::from_rgb(0x0B, 0x18, 0x16),
        ),
        _ => (theme::RAISED, theme::BG_DEEP),
    };
    theme::vertical_gradient(painter, rect, top, bottom);

    // Each game gets its own gear arrangement, derived from its hash.
    let seed = u32::from_str_radix(&entry.sha1[..8], 16).unwrap_or(0);
    let painter = painter.with_clip_rect(rect);
    let faint = Color32::from_rgba_unmultiplied(0xC1, 0x9A, 0x4B, 70);
    let hole = top.gamma_multiply(0.85);
    // Sizes follow the height, which is the same for every cover on a shelf.
    let h = rect.height();
    let big_r = h * 0.34;
    let big = egui::pos2(
        rect.left() + rect.width() * (0.3 + (seed % 40) as f32 / 100.0),
        rect.top() + rect.height() * 0.3,
    );
    let teeth = 12 + (seed % 6) as usize;
    let phase = (seed % 628) as f32 / 100.0;
    theme::gear(&painter, big, big_r, teeth, gear_spin + phase, faint, hole);
    // A smaller gear meshing with the big one.
    let small_r = big_r * 0.5;
    let small = big + egui::Vec2::angled(0.4 + (seed % 7) as f32 * 0.35) * (big_r + small_r * 0.72);
    theme::gear(
        &painter,
        small,
        small_r,
        9,
        -gear_spin * big_r / small_r,
        faint,
        hole,
    );

    // Shrink the title until its longest word fits on one line.
    let wrap = rect.width() - 24.0;
    let longest = title
        .split_whitespace()
        .max_by_key(|w| w.len())
        .unwrap_or(title);
    let mut size = h * 0.12;
    while size > 10.0
        && painter
            .layout_no_wrap(longest.to_string(), theme::display(size), theme::PARCHMENT)
            .size()
            .x
            > wrap
    {
        size -= 1.0;
    }
    let mut job = LayoutJob::simple(
        title.to_string(),
        theme::display(size),
        theme::PARCHMENT,
        wrap,
    );
    job.halign = Align::Center;
    let galley = painter.layout_job(job);
    let title_y = rect.top() + rect.height() * 0.68 - galley.size().y / 2.0;
    painter.galley(
        egui::pos2(rect.center().x, title_y),
        galley,
        theme::PARCHMENT,
    );

    // Platform plate along the bottom edge.
    let plate = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.09),
        vec2(h * 0.3, h * 0.085),
    );
    painter.rect_filled(plate, 2.0, theme::BRASS_DIM);
    painter.text(
        plate.center(),
        egui::Align2::CENTER_CENTER,
        entry.info.platform.to_uppercase(),
        egui::FontId::proportional(plate.height() * 0.72),
        theme::BG_DEEP,
    );
}

//! Running recipes against the user's ROM, and reading and saving plugins.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::io::Cursor;
use std::path::PathBuf;

use base64::Engine;
use clockwork_core::image::RgbaImage;
use clockwork_core::manifest::{Capabilities, PluginInfo, PluginManifest, SourceIdentity};
use clockwork_core::recipe::Recipe;
use clockwork_library::LibraryEntry;
use clockwork_recipe::{Extracted, Plugin, preview};

use crate::workspace::plugin_dirs;

/// Previews are scaled down to fit this many pixels on their longer side.
const PREVIEW_SIZE: u32 = 768;

pub fn parse(text: &str) -> Result<Recipe, String> {
    ron::from_str(text.trim_start_matches('\u{FEFF}'))
        .map_err(|e| format!("Recipe doesn't parse: {e}"))
}

/// Runs a recipe; parts its characters name are found among the installed plugins' assets.
pub fn run(recipe: &Recipe, rom: &[u8]) -> Result<Extracted, String> {
    let installed: Vec<Plugin> = plugins().into_iter().flatten().collect();
    let assets = clockwork_recipe::assets::OwnedAssets::of(&installed);
    clockwork_recipe::run_in(recipe, rom, &assets).map_err(|e| format!("Recipe failed: {e}"))
}

/// What the collision of a 3D area is made of: triangles by facing and by surface code.
fn terrain_summary(id: &str, terrain: &clockwork_core::terrain::Terrain) -> String {
    use clockwork_core::terrain::Facing;
    let count = terrain.triangles.len();
    let facing = |f| (0..count).filter(|&i| terrain.facing(i) == Some(f)).count();
    let mut out = format!(
        "terrain of \"{id}\": {} vertices, {count} triangles ({} floors, {} walls, {} ceilings, \
         {} without area)",
        terrain.vertices.len(),
        facing(Facing::Floor),
        facing(Facing::Wall),
        facing(Facing::Ceiling),
        (0..count).filter(|&i| terrain.facing(i).is_none()).count(),
    );
    match terrain.spawn {
        Some(spawn) => {
            let _ = write!(
                out,
                "; start {:?} facing {} degrees",
                spawn.position, spawn.yaw
            );
        }
        None => out.push_str("; no start point"),
    }
    out.push('\n');
    let mut codes: BTreeMap<u16, usize> = BTreeMap::new();
    for i in 0..count {
        for (_, code) in terrain.codes(i) {
            *codes.entry(code).or_default() += 1;
        }
    }
    for (code, n) in codes {
        let _ = writeln!(out, "  code {code:#x}: {n} triangles");
    }
    out
}

/// An area image's animation and layers, back to front.
fn layers_summary(layers: &clockwork_core::layer::Layers) -> String {
    let frames = |f: &Option<clockwork_core::layer::Frames>| match f {
        Some(f) => {
            let ticks: u32 = (0..f.images.len()).map(|i| f.ticks_of(i)).sum();
            format!(
                ", {} frames looping every {:.2} s",
                f.images.len(),
                ticks as f32 / f.fps
            )
        }
        None => String::new(),
    };
    let mut out = String::new();
    for which in layers.draw_order() {
        let line = match which {
            None => format!("  the image itself{}", frames(&layers.frames)),
            Some(i) => {
                let l = &layers.layers[i];
                format!(
                    "  layer {i}: {}×{} px, order {}, scroll ({}, {}){}{}{}",
                    l.image.width,
                    l.image.height,
                    l.order,
                    l.scroll.0,
                    l.scroll.1,
                    if l.wrap { ", wraps" } else { "" },
                    if l.over_sprites { ", over sprites" } else { "" },
                    frames(&l.frames)
                )
            }
        };
        let _ = writeln!(out, "{line}");
    }
    out
}

/// A text report of what a recipe produced.
pub fn summary(extracted: &Extracted) -> String {
    let mut out = String::new();
    for (id, image) in &extracted.images {
        let opaque = image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] != 0)
            .count();
        let total = (image.width * image.height).max(1) as usize;
        let _ = writeln!(
            out,
            "image \"{id}\": {}×{} px, {opaque} of {total} pixels opaque ({:.1}%)",
            image.width,
            image.height,
            opaque as f64 * 100.0 / total as f64
        );
        if let Some(layers) = extracted.layers(id) {
            out.push_str(&layers_summary(layers));
        }
    }
    for m in &extracted.models {
        let (id, model, meters) = (&m.id, &m.model, m.meters_per_unit);
        let triangles: usize = model.meshes.iter().map(|m| m.indices.len() / 3).sum();
        let height = model
            .rest_bounds()
            .map_or(0.0, |(lo, hi)| (hi[1] - lo[1]) * meters);
        let animations: Vec<String> = model
            .animations
            .iter()
            .map(|a| format!("{} ({} frames)", a.name, a.frames))
            .collect();
        let _ = writeln!(
            out,
            "model \"{id}\": {} bones, {} meshes, {triangles} triangles, {} textures, rest pose \
             {height:.2} m tall; animations: {}",
            model.bones.len(),
            model.meshes.len(),
            model.textures.len(),
            if animations.is_empty() {
                "none".to_string()
            } else {
                animations.join(", ")
            }
        );
    }
    for m in &extracted.models {
        if m.model.meshes.iter().all(|mesh| mesh.indices.is_empty()) {
            let _ = writeln!(
                out,
                "Problem: model \"{}\" draws nothing (no triangles): check its display lists",
                m.id
            );
        }
    }
    for m in &extracted.models {
        if let Some(terrain) = &m.terrain {
            out.push_str(&terrain_summary(&m.id, terrain));
        }
    }
    for (id, scene) in &extracted.scenes {
        let doors = scene.nodes.iter().filter(|n| n.door.is_some()).count();
        let shown = scene.nodes.iter().filter(|n| n.item.is_some()).count();
        let groups: std::collections::BTreeSet<_> = scene
            .nodes
            .iter()
            .filter_map(|n| n.group.as_ref())
            .collect();
        let _ = writeln!(
            out,
            "scene \"{id}\": {} nodes ({shown} show an output, {doors} doors) in {} groups",
            scene.nodes.len(),
            groups.len()
        );
    }
    for (id, items) in &extracted.items {
        let names: Vec<String> = items
            .iter()
            .map(|i| match &i.action {
                Some(action) => format!("{} (does {action})", i.id),
                None => i.id.clone(),
            })
            .collect();
        let _ = writeln!(
            out,
            "items \"{id}\": {} items: {}",
            items.len(),
            names.join(", ")
        );
    }
    for s in &extracted.sprites {
        if let Some(c) = &s.character
            && !c.actions.is_empty()
        {
            let actions: Vec<String> = c
                .actions
                .iter()
                .map(|a| {
                    format!(
                        "{} ({:.2} s{}{}{}{})",
                        a.id,
                        a.duration,
                        if a.hits.is_empty() { "" } else { ", hits" },
                        if a.spawns.is_empty() { "" } else { ", throws" },
                        if a.hold.is_some() { ", holds" } else { "" },
                        match a.motion {
                            clockwork_core::action::Motion::Slide(_) => ", slides",
                            _ => "",
                        }
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "sprite \"{}\" actions: {}; projectiles: {}; dodge: {}",
                s.id,
                actions.join(", "),
                c.projectiles
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                c.dodge.as_deref().unwrap_or("none")
            );
        }
        let animations: Vec<String> = s
            .sprite
            .animations
            .iter()
            .map(|a| {
                format!(
                    "{} ({} steps, {:.2} s)",
                    a.name,
                    a.steps.len(),
                    a.duration()
                )
            })
            .collect();
        let _ = writeln!(
            out,
            "sprite \"{}\": {} frames, {} px per meter{}; animations: {}",
            s.id,
            s.sprite.frames.len(),
            s.pixels_per_meter,
            if s.character.is_some() {
                ", playable"
            } else {
                ""
            },
            if animations.is_empty() {
                "none".to_string()
            } else {
                animations.join(", ")
            }
        );
    }
    for a in &extracted.abilities {
        let list = |names: Vec<&str>| {
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        };
        let _ = writeln!(
            out,
            "ability \"{}\": actions {}; projectiles {}; items {}; roles {}{}",
            a.id,
            list(a.moves.actions.iter().map(|x| x.id.as_str()).collect()),
            list(a.moves.projectiles.iter().map(|x| x.id.as_str()).collect()),
            list(a.items.iter().map(|x| x.id.as_str()).collect()),
            list(a.roles.iter().map(|x| x.name.as_str()).collect()),
            if a.moves.dodge.is_some() {
                "; the dodge"
            } else {
                ""
            },
        );
    }
    for (id, items) in &extracted.items {
        let names: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let _ = writeln!(out, "items \"{id}\": {}", names.join(", "));
    }
    for (id, font) in &extracted.fonts {
        let _ = writeln!(
            out,
            "font \"{id}\": {} characters of {}×{} px",
            font.chars.chars().count(),
            font.glyph.0,
            font.glyph.1
        );
    }
    for (id, _) in &extracted.sounds {
        let _ = writeln!(out, "sound \"{id}\"");
    }
    for (id, dialog) in &extracted.dialogs {
        // The start of its first page, so a wrong offset or character map shows.
        let first: String = dialog
            .pages
            .first()
            .map(|p| p.replace('\n', " / ").chars().take(48).collect())
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "dialog \"{id}\": {} pages, \"{first}\"",
            dialog.pages.len()
        );
    }
    for (id, map) in &extracted.surfaces {
        let _ = writeln!(
            out,
            "surfaces \"{id}\": {}×{} cells, {} kinds",
            map.width,
            map.height,
            map.kinds.len()
        );

        // Every code of every grid, with how many cells have it.
        let mut per_grid: BTreeMap<&str, BTreeMap<u16, usize>> = BTreeMap::new();
        for &cell in &map.cells {
            for (grid, code) in &map.kinds[cell as usize].codes {
                *per_grid.entry(grid).or_default().entry(*code).or_default() += 1;
            }
        }
        for (grid, codes) in per_grid {
            let list: Vec<String> = codes
                .iter()
                .map(|(code, n)| format!("{code:#04X}×{n}"))
                .collect();
            let _ = writeln!(out, "  {grid} codes: {}", list.join(" "));
        }

        let mut kinds: Vec<(usize, usize)> = (0..map.kinds.len())
            .map(|k| (k, map.cells.iter().filter(|&&c| c as usize == k).count()))
            .collect();
        kinds.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        let _ = writeln!(out, "  most common kinds:");
        for (k, n) in kinds.iter().take(15) {
            let kind = &map.kinds[*k];
            let codes: Vec<String> = kind
                .codes
                .iter()
                .map(|(grid, code)| format!("{grid} {code:#04X}"))
                .collect();
            let _ = writeln!(
                out,
                "  {n:5} cells  {:?}  from {}",
                kind.properties,
                codes.join(", ")
            );
        }
        if kinds.len() > 15 {
            let _ = writeln!(out, "  … {} more kinds", kinds.len() - 15);
        }
    }
    if out.is_empty() {
        out.push_str("The recipe ran but declares no outputs.");
    }
    out
}

/// What to show of a surface map in a preview.
pub struct SurfaceView<'a> {
    /// Surface output id; the first one if `None`.
    pub id: Option<&'a str>,
    /// With `codes`: highlight cells whose code in this grid is listed. Without: the full
    /// property overlay.
    pub grid: Option<&'a str>,
    pub codes: Vec<u16>,
}

/// A rectangle of one output image to show enlarged.
pub struct Zoom<'a> {
    pub image: &'a str,
    pub rect: (u32, u32, u32, u32),
}

/// The previews to return: every image (up to 4), or one image with surfaces and/or zoom.
/// Returns (label, PNG as base64).
pub fn previews(
    extracted: &Extracted,
    surfaces: Option<SurfaceView>,
    zoom: Option<Zoom>,
) -> Result<Vec<(String, String)>, String> {
    if surfaces.is_none() && zoom.is_none() {
        // Models first (posed by their first animation), then sprites and images; four at most.
        let models = extracted.models.iter().map(|m| {
            let (id, model) = (&m.id, &m.model);
            let animation = model.animations.first();
            let shown = preview::on_checkerboard(&preview::render_model(
                model,
                animation.map(|a| (a, 0)),
                PREVIEW_SIZE,
            ));
            let pose = animation.map_or("rest pose".to_string(), |a| {
                format!("animation \"{}\" frame 0", a.name)
            });
            Ok((format!("Model \"{id}\", {pose}"), png(&shown)?))
        });
        // A playable sprite shows its actions with what they hold first: that's where held items
        // that don't line up with the character show.
        let sprites = extracted.sprites.iter().map(|s| {
            let actions = s.character.as_ref().and_then(|character| {
                preview::actions_sheet(&s.sprite, character, &|id| {
                    extracted.sprite(id).map(|o| &o.sprite)
                })
            });
            let (label, sheet) = match actions {
                Some(sheet) => (
                    format!(
                        "Sprite \"{}\", its actions with attachments: rows of south and east",
                        s.id
                    ),
                    sheet,
                ),
                None => (
                    format!("Sprite \"{}\", one row per animation", s.id),
                    preview::sprite_sheet(&s.sprite),
                ),
            };
            let shown = preview::fit(&preview::on_checkerboard(&sheet), PREVIEW_SIZE);
            Ok((label, png(&shown)?))
        });
        // An area with layers shows them all, as it first appears.
        let images = extracted.images.iter().map(|(id, image)| {
            let (label, image) = match extracted.layers(id) {
                Some(layers) => (
                    format!("Preview of \"{id}\" with its layers (first frames, not scrolled)"),
                    preview::layered(image, layers),
                ),
                None => (format!("Preview of \"{id}\""), image.clone()),
            };
            let shown = preview::fit(&preview::on_checkerboard(&image), PREVIEW_SIZE);
            Ok((label, png(&shown)?))
        });
        let scenes = extracted.scenes.iter().map(|(id, scene)| {
            let image = |output: &str| {
                extracted
                    .images
                    .iter()
                    .find(|(i, _)| i == output)
                    .map(|(_, image)| image)
            };
            let map = preview::scene_map(scene, image, 16.0, PREVIEW_SIZE);
            Ok((
                format!("Scene \"{id}\": its images where they stand, other nodes as dots"),
                png(&preview::on_checkerboard(&map))?,
            ))
        });
        return models
            .chain(sprites)
            .chain(scenes)
            .chain(images)
            .take(4)
            .collect();
    }

    // Pick the surface map, and the image it covers (or the zoomed image).
    let map = match &surfaces {
        Some(view) => Some(match view.id {
            Some(id) => extracted
                .surfaces
                .iter()
                .find(|(s, _)| s == id)
                .map(|(_, m)| m)
                .ok_or_else(|| format!("No surface output \"{id}\""))?,
            None => extracted
                .surfaces
                .first()
                .map(|(_, m)| m)
                .ok_or("The recipe has no surface output")?,
        }),
        None => None,
    };
    let image_id = match (&zoom, map) {
        (Some(z), _) => Some(z.image.to_string()),
        // The image the map names, else one of its size.
        (None, Some(m)) => {
            let named = extracted.surfaces.iter().find_map(|(id, map)| {
                std::ptr::eq(map, m)
                    .then(|| extracted.surface_images.iter().find(|(s, _)| s == id))
                    .flatten()
                    .map(|(_, image)| image.clone())
            });
            named.or_else(|| {
                extracted
                    .images
                    .iter()
                    .find(|(_, i)| {
                        i.width == m.width * m.cell_size && i.height == m.height * m.cell_size
                    })
                    .map(|(id, _)| id.clone())
            })
        }
        (None, None) => None,
    };
    let mut label = String::new();
    let mut shown = match &image_id {
        Some(id) => {
            let image = extracted
                .images
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, i)| i)
                .ok_or_else(|| format!("No image output \"{id}\""))?;
            label.push_str(&format!("\"{id}\""));
            preview::on_checkerboard(image)
        }
        None => {
            let m = map.unwrap();
            label.push_str("surfaces on an empty background");
            preview::on_checkerboard(&RgbaImage::new(
                m.width * m.cell_size,
                m.height * m.cell_size,
            ))
        }
    };

    if let (Some(view), Some(map)) = (&surfaces, map) {
        let layer = match (view.grid, view.codes.is_empty()) {
            (Some(grid), false) => {
                let codes: Vec<String> = view.codes.iter().map(|c| format!("{c:#04X}")).collect();
                label.push_str(&format!(", {grid} codes {} highlighted", codes.join(" ")));
                preview::highlight(map, grid, &view.codes)
            }
            _ => {
                label.push_str(", surface overlay (red solid, blue water, black fall, green corner = game-specific)");
                preview::overlay(map)
            }
        };
        shown = preview::blend(&shown, &layer);
    }

    if let Some(z) = &zoom {
        let (x, y, w, h) = z.rect;
        shown = shown.crop(x, y, w, h);
        if shown.width == 0 || shown.height == 0 {
            return Err(format!("The zoom rectangle is outside \"{}\"", z.image));
        }
        label.push_str(&format!(
            ", zoomed to x {x}, y {y}, {}×{} px",
            shown.width, shown.height
        ));
    }
    let scaled = preview::fit(&shown, PREVIEW_SIZE);
    Ok(vec![(label, png(&scaled)?)])
}

/// The image as a PNG, base64-encoded.
fn png(image: &RgbaImage) -> Result<String, String> {
    let buffer = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
        .ok_or("bad image size")?;
    let mut png = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

pub fn plugins() -> Vec<Result<Plugin, String>> {
    plugin_dirs()
        .iter()
        .flat_map(Plugin::load_all)
        .map(|r| r.map_err(|e| e.to_string()))
        .collect()
}

pub fn find_plugin(name: &str) -> Result<Plugin, String> {
    plugins()
        .into_iter()
        .flatten()
        .find(|p| p.manifest.plugin.name == name)
        .ok_or_else(|| format!("No plugin named \"{name}\"; see list_plugins"))
}

/// Whether `id` is a safe plugin or game id: a folder name on every system, never a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Writes a verified plugin folder and returns its path.
pub fn save(
    name: &str,
    game_id: &str,
    version: &str,
    entry: &LibraryEntry,
    recipe_text: &str,
    overwrite: bool,
) -> Result<PathBuf, String> {
    if !valid_id(name) || !valid_id(game_id) {
        return Err("name and game_id may only use a-z, 0-9, '-' and '_'".into());
    }
    let dir = plugin_dirs()
        .into_iter()
        .next()
        .ok_or("No plugins folder")?
        .join(name);
    if dir.exists() && !overwrite {
        return Err(format!(
            "{} already exists; pass overwrite: true to replace it",
            dir.display()
        ));
    }
    let manifest = PluginManifest {
        plugin: PluginInfo {
            name: name.into(),
            version: version.into(),
            core: clockwork_core::CORE_VERSION.into(),
        },
        source: SourceIdentity {
            game: game_id.into(),
            sha1: vec![entry.sha1.clone()],
        },
        provides: Capabilities::default(),
        requires: Capabilities::default(),
    };
    let toml = toml::to_string(&manifest).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    clockwork_library::files::write_atomic(dir.join("clockwork.toml"), toml)
        .map_err(|e| e.to_string())?;
    clockwork_library::files::write_atomic(dir.join("recipe.ron"), recipe_text)
        .map_err(|e| e.to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_are_listed_back_to_front() {
        use clockwork_core::layer::{Frames, Layer, Layers};
        let layers = Layers {
            frames: Some(Frames {
                images: vec![RgbaImage::new(1, 1); 3],
                ticks: vec![20],
                fps: 60.0,
            }),
            layers: vec![Layer {
                image: RgbaImage::new(256, 256),
                order: -1,
                scroll: (0.25, 0.5),
                wrap: true,
                over_sprites: false,
                frames: None,
            }],
        };
        assert_eq!(
            layers_summary(&layers),
            "  layer 0: 256×256 px, order -1, scroll (0.25, 0.5), wraps\n  \
             the image itself, 3 frames looping every 1.00 s\n"
        );
    }

    #[test]
    fn plugin_ids_are_plain_folder_names() {
        assert!(valid_id("minish-cap_link2"));
        for bad in [
            "",
            "Link",
            "../escape",
            "a/b",
            "a\\b",
            "c:",
            "dot.ted",
            "space d",
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
    }
}

//! Running the recipe for the graph: what every step made, summed up for its node, and the
//! points a run keeps so the next one goes on from them.

use std::{collections::HashMap, sync::Arc};

use clockwork_core::{
    asset::Slot,
    camera::CameraRig,
    image::RgbaImage,
    parts::{AnyAnimations, AnyMovement},
    recipe::{NameKind, Output, Recipe, Step},
};
use clockwork_recipe::Made;

use super::{OwnedPort, RNode, THUMB};

/// What a step or output made, for its node.
#[derive(Clone, Debug, Default)]
pub(super) struct StepResult {
    pub(super) lines: Vec<String>,
    /// The first bytes of a buffer it made.
    pub(super) bytes: Option<Vec<u8>>,
    /// A picture of what it made (shrunk), and the size of the image it shows.
    pub(super) image: Option<RgbaImage>,
    pub(super) full: Option<(u32, u32)>,
}

#[derive(Default)]
pub(super) struct Trace {
    /// By step; `None` for steps that didn't run.
    pub(super) steps: Vec<Option<StepResult>>,
    /// By output, once the whole recipe ran.
    pub(super) outputs: Vec<Option<StepResult>>,
    pub(super) error: Option<String>,
    pub(super) error_step: Option<usize>,
    /// The output the error is in.
    pub(super) error_output: Option<usize>,
    /// What the run kept to go on from, so the next run after a change starts near it.
    pub(super) kept: Kept,
}

/// What the steps of a run had made at points along the way, for the next run to go on from:
/// a change to step 150 of 180 runs only the steps from the last point before it.
#[derive(Clone, Default)]
pub(super) struct Kept {
    /// The steps that ran, and the ROM they ran on.
    pub(super) steps: Vec<Step>,
    pub(super) rom: Option<Arc<Vec<u8>>>,
    /// After how many steps, and what they had made by then.
    pub(super) marks: Vec<(usize, Arc<Made>)>,
}

/// Steps between two points a run keeps.
pub(super) const KEEP_EVERY: usize = 16;

/// The most memory a run's kept points may take, in bytes.
pub(super) const KEEP_BUDGET: usize = 256 * 1024 * 1024;

impl Kept {
    /// Where a run of `steps` on `rom` can go on from: the steps that need not run again, and
    /// what they made (`None` to run all of them).
    pub(super) fn start(&self, steps: &[Step], rom: &Arc<Vec<u8>>) -> Option<(usize, &Arc<Made>)> {
        if !self.rom.as_ref().is_some_and(|r| Arc::ptr_eq(r, rom)) {
            return None;
        }
        let same = (self.steps.iter())
            .zip(steps)
            .take_while(|(a, b)| a == b)
            .count();
        (self.marks.iter())
            .rev()
            .find(|(n, _)| *n <= same)
            .map(|(n, made)| (*n, made))
    }
}

/// Bytes kept of a buffer, to show.
pub(super) const PEEK: usize = 256;

/// Bytes as rows of 16 in hex, each after its offset.
pub(super) fn hex_rows(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            format!("{:04X}  {}", row * 16, hex.join(" "))
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// What each output of a recipe that ran is, for its node.
pub(super) fn summarize_outputs(
    recipe: &Recipe,
    extracted: &clockwork_recipe::Extracted,
) -> Vec<Option<StepResult>> {
    recipe
        .outputs
        .iter()
        .map(|output| {
            let id = output.id();
            let mut result = StepResult::default();
            match output {
                Output::Image { .. } => {
                    let (_, image) = extracted.images.iter().find(|(i, _)| i == id)?;
                    result
                        .lines
                        .push(format!("{}×{} pixels", image.width, image.height));
                    result.image = Some(shrink(image, THUMB));
                }
                Output::Surfaces { .. } => {
                    let (_, map) = extracted.surfaces.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{}×{} cells of {}, {} kinds of surface",
                        map.width,
                        map.height,
                        map.cell_size,
                        map.kinds.len()
                    ));
                }
                Output::Model { .. } => {
                    let m = extracted.models.iter().find(|m| m.id == id)?;
                    result.lines.push(format!(
                        "{} bones, {} meshes, {} animations",
                        m.model.bones.len(),
                        m.model.meshes.len(),
                        m.model.animations.len()
                    ));
                    if let Some(terrain) = &m.terrain {
                        result
                            .lines
                            .push(format!("walkable: {} triangles", terrain.triangles.len()));
                    }
                    if m.character.is_some() {
                        result.lines.push("playable".into());
                    }
                }
                Output::Sprite { .. } => {
                    let s = extracted.sprites.iter().find(|s| s.id == id)?;
                    result.lines.push(format!(
                        "{} frames, {} animations{}",
                        s.sprite.frames.len(),
                        s.sprite.animations.len(),
                        if s.character.is_some() {
                            ", playable"
                        } else {
                            ""
                        }
                    ));
                    result.image = s.sprite.frames.first().map(|f| shrink(&f.image, THUMB));
                }
                Output::Items { .. } => {
                    let (_, items) = extracted.items.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!("{} items", items.len()));
                }
                Output::Ability { .. } => {
                    let a = extracted.abilities.iter().find(|a| a.id == id)?;
                    result
                        .lines
                        .push(format!("{}: {} roles", a.name, a.roles.len()));
                }
                Output::Camera { camera, .. } => result.lines.push(match camera {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(CameraRig::Follow(c)) => {
                        format!("behind, {} m at {}°", c.distance, c.pitch)
                    }
                    Slot::Inline(CameraRig::TopDown(c)) => {
                        format!("above, {} m at {}°", c.distance, c.pitch)
                    }
                }),
                Output::Hud { hud, .. } => result.lines.push(match hud {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(h) => format!("{} widgets", h.widgets.len()),
                }),
                Output::Ui { ui, .. } => result.lines.push(match ui {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(t) => format!(
                        "{} font, {}",
                        t.font.as_deref().unwrap_or("default"),
                        if t.frame.is_some() {
                            "framed panels"
                        } else {
                            "plain panels"
                        }
                    ),
                }),
                Output::Menu { menu, .. } => result.lines.push(match menu {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(m) => format!("{}: opens with {}", m.title, m.opens_with),
                }),
                Output::InputMap { input, .. } => result.lines.push(match input {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(m) => format!("{} buttons bound", m.bindings.len()),
                }),
                Output::Movement { movement, .. } => result.lines.push(match movement {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(AnyMovement::TopDown(m)) => format!("walks at {} m/s", m.speed),
                    Slot::Inline(AnyMovement::Platformer(m)) => {
                        format!("runs at {} m/s", m.run_speed)
                    }
                }),
                Output::Animations { animations, .. } => result.lines.push(match animations {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(AnyAnimations::Directional(_)) => "by facing".to_string(),
                    Slot::Inline(AnyAnimations::States(_)) => "by state".to_string(),
                }),
                Output::Stats { stats, .. } => result.lines.push(match stats {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(s) => format!("{} health", s.health),
                }),
                Output::Behavior { behavior, .. } => result.lines.push(match behavior {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(b) => format!("{}, {} m around", b.kind(), b.radius()),
                }),
                Output::Rules { rules, .. } => result.lines.push(match rules {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(r) => format!(
                        "{:?}{}",
                        r.when_down,
                        r.lives.map_or(String::new(), |l| format!(", {l} lives"))
                    ),
                }),
                Output::Game { game, .. } => result.lines.push(match game {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(_) => "a game's assets".to_string(),
                }),
                Output::Flow { flow, .. } => result.lines.push(match flow {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(f) => {
                        let screens: Vec<&str> = [
                            f.title.as_ref().map(|_| "title"),
                            f.files.as_ref().map(|_| "files"),
                            f.acts.as_ref().map(|_| "acts"),
                            f.game_over.as_ref().map(|_| "game over"),
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        format!("screens: {}", screens.join(", "))
                    }
                }),
                Output::Font { chars, .. } => {
                    let (_, font) = extracted.fonts.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{} glyphs of {}×{}",
                        chars.chars().count(),
                        font.glyph.0,
                        font.glyph.1
                    ));
                    result.image = Some(shrink(&font.image, THUMB));
                }
                Output::Scene { .. } => {
                    let (_, scene) = extracted.scenes.iter().find(|(i, _)| i == id)?;
                    let doors = scene.nodes.iter().filter(|n| n.door.is_some()).count();
                    result
                        .lines
                        .push(format!("{} nodes, {doors} doors", scene.nodes.len()));
                }
                Output::Sound { .. } => {
                    let (_, pcm) = extracted.sounds.iter().find(|(i, _)| i == id)?;
                    result.lines.push(format!(
                        "{:.2} s at {} Hz",
                        pcm.samples.len() as f32 / pcm.rate.max(1) as f32,
                        pcm.rate
                    ));
                }
                Output::Music { .. } => {
                    let (_, song) = extracted.music.iter().find(|(i, _)| i == id)?;
                    let tracks = song.rom.get(song.at).copied().unwrap_or(0);
                    result
                        .lines
                        .push(format!("a song of {tracks} tracks, played in Play"));
                }
                Output::Dialog { dialog, .. } => result.lines.push(match dialog {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(d) => format!("{} pages", d.pages.len()),
                }),
                Output::Sounds { sounds, .. } => result.lines.push(match sounds {
                    Slot::Ref(at) => format!("same as {at}"),
                    Slot::Inline(_) => "a character's sounds".to_string(),
                }),
            }
            Some(result)
        })
        .collect()
}

pub(super) fn summarize(ports: &[OwnedPort], made: &Made) -> StepResult {
    let mut result = StepResult::default();
    for port in ports.iter().filter(|p| p.access.writes()) {
        let name = &port.name;
        let line = match port.kind {
            NameKind::Buffer => made.buffer(name).map(|b| {
                if result.bytes.is_none() {
                    result.bytes = Some(b[..b.len().min(PEEK)].to_vec());
                }
                format!("{name}: {} bytes", b.len())
            }),
            NameKind::Bank => made.has_bank(name).then(|| format!("{name}: palettes")),
            NameKind::Image => made.image(name).map(|image| {
                if result.image.is_none() {
                    result.image = Some(shrink(image, THUMB));
                    result.full = Some((image.width, image.height));
                }
                format!("{name}: {}×{}", image.width, image.height)
            }),
            NameKind::Model => made.model(name).map(|m| {
                format!(
                    "{name}: {} bones, {} meshes, {} animations",
                    m.bones.len(),
                    m.meshes.len(),
                    m.animations.len()
                )
            }),
            NameKind::Collision => made
                .collision(name)
                .map(|c| format!("{name}: {} triangles", c.triangles.len())),
            NameKind::Sprite => made.sprite(name).map(|s| {
                if result.image.is_none() {
                    result.image = s.frames.first().map(|f| shrink(&f.image, THUMB));
                }
                format!(
                    "{name}: {} frames, {} animations",
                    s.frames.len(),
                    s.animations.len()
                )
            }),
            NameKind::Places => made
                .places(name)
                .map(|p| format!("{name}: {} placed", p.len())),
        };
        result.lines.extend(line);
    }
    result
}

/// `image` made at most `side` pixels on its longer side (nearest pixel), for previews.
pub(super) fn shrink(image: &RgbaImage, side: u32) -> RgbaImage {
    let longest = image.width.max(image.height).max(1);
    if longest <= side {
        return image.clone();
    }
    let w = (image.width * side / longest).max(1);
    let h = (image.height * side / longest).max(1);
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.set(x, y, image.get(x * image.width / w, y * image.height / h));
        }
    }
    out
}

/// Runs a recipe on the ROM and keeps what every step made, going on from the `last` run
/// where the steps before a change are the same.
pub(super) fn trace(
    recipe: &Recipe,
    rom: Option<&Arc<Vec<u8>>>,
    assets: &clockwork_recipe::assets::OwnedAssets,
    plugin: &str,
    ports: &HashMap<RNode, Vec<OwnedPort>>,
    last: &Trace,
) -> Trace {
    let Some(rom) = rom else {
        return Trace {
            error: Some("Its game isn't in the Library, so nothing can be shown".into()),
            ..Trace::default()
        };
    };
    let mut steps = vec![None; recipe.steps.len()];
    let mut kept = Kept {
        steps: recipe.steps.clone(),
        rom: Some(rom.clone()),
        marks: Vec::new(),
    };
    let (from, made) = match last.kept.start(&recipe.steps, rom) {
        // What the steps before showed is still right, as long as all of them did.
        Some((n, made)) if last.steps.len() >= n && last.steps[..n].iter().all(Option::is_some) => {
            steps[..n].clone_from_slice(&last.steps[..n]);
            kept.marks = (last.kept.marks.iter())
                .filter(|(m, _)| *m <= n)
                .cloned()
                .collect();
            (n, Made::clone(made))
        }
        _ => (0, Made::default()),
    };
    let mut budget: usize = kept.marks.iter().map(|(_, m)| m.bytes()).sum();
    let assets = assets.with_recipe(plugin, recipe);
    let result = clockwork_recipe::run_from(recipe, rom, &assets, made, from, |i, made| {
        steps[i] = Some(summarize(&ports[&RNode::Step(i)], made));
        let n = i + 1;
        if n % KEEP_EVERY == 0 && n < recipe.steps.len() {
            let bytes = made.bytes();
            if budget + bytes <= KEEP_BUDGET {
                budget += bytes;
                kept.marks.push((n, Arc::new(made.clone())));
            }
        }
    });
    let (outputs, error, error_step, error_output) = match result {
        Ok(extracted) => (summarize_outputs(recipe, &extracted), None, None, None),
        Err(e) => {
            let output = e
                .output()
                .and_then(|id| recipe.outputs.iter().position(|o| o.id() == id));
            // An unknown name in an output names what it's made from, not its id.
            let output = output.or(match &e {
                clockwork_recipe::RecipeError::UnknownOutput(from) => recipe
                    .outputs
                    .iter()
                    .position(|o| o.ports().iter().any(|p| p.name == from.as_str())),
                _ => None,
            });
            (Vec::new(), Some(e.to_string()), e.step(), output)
        }
    };
    Trace {
        steps,
        outputs,
        error,
        error_step,
        error_output,
        kept,
    }
}

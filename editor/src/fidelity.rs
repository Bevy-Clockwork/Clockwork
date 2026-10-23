//! Rendering regression checks: a game's views (remote-control commands that set up a shot)
//! are rendered, and each picture is compared with the one approved for it, so a change that
//! draws a game differently is seen. Approved pictures come from the user's own ROM, so they
//! live with the game's content (`fidelity/<game>/approved/`), never in Clockwork itself.
//!
//! A folder holds `views.ron`, `(views: [(name: "grounds", commands: [Frame("castle"), ...])])`;
//! `Fidelity(folder)` renders them into `current/`, compares, and writes `report.md` (with a
//! `.diff.png` for each changed view); `Approve(folder, None)` makes the current pictures the
//! approved ones (or `Some("name")`, one of them).

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use serde::Deserialize;

use crate::control::Command;

/// A channel differing by more than this counts the pixel as changed: leaves room for
/// rounding differences between GPUs and drivers.
const CHANNEL_SLACK: u8 = 8;

#[derive(Debug, Clone, Deserialize)]
pub struct Views {
    pub views: Vec<View>,
    /// The share of a picture's pixels that may change before it fails (default 0.5%).
    #[serde(default)]
    pub tolerance: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct View {
    pub name: String,
    /// What sets the shot up; the picture is taken once they ran and `settle` frames passed.
    pub commands: Vec<Command>,
    #[serde(default = "settle")]
    pub settle: u32,
    /// Parts of the picture left out of the comparison, as (x, y, width, height) shares of
    /// its size: what moves on its own, like an idling character.
    #[serde(default)]
    pub ignore: Vec<(f32, f32, f32, f32)>,
}

fn settle() -> u32 {
    60
}

pub fn load(folder: &Path) -> Result<Views, String> {
    let path = folder.join("views.ron");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The commands that render every view into `current/`, then report on them.
pub fn commands(folder: &Path) -> Result<Vec<Command>, String> {
    let views = load(folder)?;
    let current = folder.join("current");
    std::fs::create_dir_all(&current).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for view in &views.views {
        out.extend(view.commands.iter().cloned());
        out.push(Command::Wait(view.settle));
        out.push(Command::Screenshot(
            current.join(format!("{}.png", view.name)),
        ));
    }
    out.push(Command::FidelityReport(folder.to_path_buf()));
    Ok(out)
}

/// How a picture differs from the approved one.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Same,
    /// The share of pixels that changed, and a picture marking them.
    Changed(f32, RgbaImage),
    Resized((u32, u32), (u32, u32)),
}

pub fn compare(
    current: &RgbaImage,
    approved: &RgbaImage,
    tolerance: f32,
    ignore: &[(f32, f32, f32, f32)],
) -> Outcome {
    if current.dimensions() != approved.dimensions() {
        return Outcome::Resized(approved.dimensions(), current.dimensions());
    }
    let (w, h) = (current.width() as f32, current.height() as f32);
    let ignored = |x: u32, y: u32| {
        let (x, y) = ((x as f32 + 0.5) / w, (y as f32 + 0.5) / h);
        ignore
            .iter()
            .any(|&(l, t, rw, rh)| x >= l && x < l + rw && y >= t && y < t + rh)
    };
    let mut diff = RgbaImage::new(current.width(), current.height());
    let mut changed = 0usize;
    for ((x, y, a), b) in current.enumerate_pixels().zip(approved.pixels()) {
        let differs = !ignored(x, y)
            && a.0
                .iter()
                .zip(b.0)
                .any(|(p, q)| p.abs_diff(q) > CHANNEL_SLACK);
        let shade = |c: u8| c / 3;
        diff.put_pixel(
            x,
            y,
            if differs {
                changed += 1;
                Rgba([255, 0, 64, 255])
            } else {
                Rgba([shade(a[0]), shade(a[1]), shade(a[2]), 255])
            },
        );
    }
    let share = changed as f32 / (current.width() * current.height()).max(1) as f32;
    if share > tolerance {
        Outcome::Changed(share, diff)
    } else {
        Outcome::Same
    }
}

/// Compares every view's picture with its approved one, writes `report.md` and the diff
/// pictures, and returns the views that failed.
pub fn report(folder: &Path) -> Result<Vec<String>, String> {
    let views = load(folder)?;
    let tolerance = views.tolerance.unwrap_or(0.005);
    let (current, approved) = (folder.join("current"), folder.join("approved"));
    let mut lines = vec![
        "# Rendering check".to_string(),
        String::new(),
        "| View | Result |".to_string(),
        "| --- | --- |".to_string(),
    ];
    let mut failed = Vec::new();
    for view in &views.views {
        let file = format!("{}.png", view.name);
        let open = |dir: &PathBuf| image::open(dir.join(&file)).map(|i| i.to_rgba8());
        let result = match (open(&current), open(&approved)) {
            (Err(e), _) => format!("not rendered ({e})"),
            (Ok(_), Err(_)) => {
                failed.push(view.name.clone());
                "new: nothing approved yet".to_string()
            }
            (Ok(now), Ok(then)) => match compare(&now, &then, tolerance, &view.ignore) {
                Outcome::Same => "same".to_string(),
                Outcome::Resized(was, is) => {
                    failed.push(view.name.clone());
                    format!("size changed from {}×{} to {}×{}", was.0, was.1, is.0, is.1)
                }
                Outcome::Changed(share, diff) => {
                    failed.push(view.name.clone());
                    let diff_file = format!("{}.diff.png", view.name);
                    diff.save(current.join(&diff_file))
                        .map_err(|e| e.to_string())?;
                    format!(
                        "**changed**: {:.2}% of pixels (current/{diff_file})",
                        share * 100.0
                    )
                }
            },
        };
        lines.push(format!("| {} | {result} |", view.name));
    }
    lines.push(String::new());
    if failed.is_empty() {
        lines.push("Every view draws as approved.".to_string());
    } else {
        lines.push(format!(
            "Look at the changed views; if they are right, approve them with \
             `Approve(\"{}\", None)`.",
            folder.display()
        ));
    }
    std::fs::write(folder.join("report.md"), lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(failed)
}

/// Makes the current pictures (or one view's) the approved ones; returns how many.
pub fn approve(folder: &Path, name: Option<&str>) -> Result<usize, String> {
    let views = load(folder)?;
    let approved = folder.join("approved");
    std::fs::create_dir_all(&approved).map_err(|e| e.to_string())?;
    let mut count = 0;
    for view in views
        .views
        .iter()
        .filter(|v| name.is_none_or(|n| v.name == n))
    {
        let file = format!("{}.png", view.name);
        let from = folder.join("current").join(&file);
        if from.exists() {
            std::fs::copy(&from, approved.join(&file)).map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    if count == 0 {
        return Err("no current pictures to approve: run Fidelity first".into());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(width: u32, height: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(c))
    }

    #[test]
    fn small_differences_pass_and_real_changes_fail() {
        let approved = filled(10, 10, [100, 100, 100, 255]);
        // A driver's rounding: every channel off by a little.
        assert_eq!(
            compare(&filled(10, 10, [104, 97, 100, 255]), &approved, 0.005, &[]),
            Outcome::Same
        );
        let mut changed = approved.clone();
        for x in 0..10 {
            changed.put_pixel(x, 0, Rgba([255, 0, 0, 255]));
        }
        // The changed row, left out: the same.
        assert_eq!(
            compare(&changed, &approved, 0.005, &[(0.0, 0.0, 1.0, 0.1)]),
            Outcome::Same
        );
        match compare(&changed, &approved, 0.005, &[]) {
            Outcome::Changed(share, diff) => {
                assert!((share - 0.1).abs() < 1e-6);
                assert_eq!(diff.get_pixel(0, 0).0, [255, 0, 64, 255]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            compare(&filled(5, 5, [0; 4]), &approved, 0.005, &[]),
            Outcome::Resized((10, 10), (5, 5))
        );
    }

    #[test]
    fn views_render_compare_and_get_approved() {
        let folder =
            std::env::temp_dir().join(format!("clockwork-fidelity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("current")).unwrap();
        std::fs::write(
            folder.join("views.ron"),
            r#"(views: [(name: "a", commands: [Wait(1)]), (name: "b", commands: [])])"#,
        )
        .unwrap();
        let queued = commands(&folder).unwrap();
        assert_eq!(queued.len(), 2 + 2 + 1 + 1);
        assert!(matches!(queued.last(), Some(Command::FidelityReport(_))));

        let shot = |name: &str, c| {
            filled(4, 4, c)
                .save(folder.join("current").join(format!("{name}.png")))
                .unwrap()
        };
        shot("a", [10, 20, 30, 255]);
        shot("b", [10, 20, 30, 255]);
        // Nothing approved: both are new.
        assert_eq!(report(&folder).unwrap(), ["a", "b"]);
        assert_eq!(approve(&folder, None).unwrap(), 2);
        assert!(report(&folder).unwrap().is_empty());
        shot("b", [200, 20, 30, 255]);
        assert_eq!(report(&folder).unwrap(), ["b"]);
        assert!(folder.join("current/b.diff.png").exists());
        let text = std::fs::read_to_string(folder.join("report.md")).unwrap();
        assert!(text.contains("| b | **changed**"), "{text}");
        assert_eq!(approve(&folder, Some("b")).unwrap(), 1);
        assert!(report(&folder).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }
}

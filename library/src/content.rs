//! Content folders: where plugins and game knowledge live. They are kept apart from Clockwork
//! itself, so Clockwork ships without anything specific to a game.
//!
//! Each content folder holds `plugins/` and `knowledge/`. The user's own folders come first:
//! those listed in `<data>/content.ron` (edited in the editor's Settings) and in
//! `CLOCKWORK_CONTENT` (separated like PATH, e.g. for testing). The data folder is always last.

use std::path::PathBuf;

use crate::store;

fn list_path() -> Option<PathBuf> {
    store::data_dir().map(|d| d.join("content.ron"))
}

/// The content folders the user added.
pub fn user_folders() -> Vec<PathBuf> {
    list_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_user_folders(folders: &[PathBuf]) -> std::io::Result<()> {
    let path = list_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
    let text =
        ron::ser::to_string_pretty(folders, Default::default()).map_err(std::io::Error::other)?;
    crate::files::write_atomic(&path, text)
}

/// Every content folder, in search order. New plugins go to the first.
pub fn folders() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os("CLOCKWORK_CONTENT")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    out.extend(user_folders());
    out.extend(store::data_dir());
    let mut seen = Vec::new();
    out.retain(|p| {
        let new = !seen.contains(p);
        seen.push(p.clone());
        new
    });
    out
}

pub fn plugin_dirs() -> Vec<PathBuf> {
    folders().into_iter().map(|f| f.join("plugins")).collect()
}

pub fn knowledge_dirs() -> Vec<PathBuf> {
    folders().into_iter().map(|f| f.join("knowledge")).collect()
}

//! Identifies games and fetches box art in the background.
//!
//! For each game without a known name or cover, the No-Intro database gives its full name, and
//! libretro-thumbnails its box art.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use bevy_egui::EguiContexts;
use clockwork_library::{LibraryEntry, boxart::BoxArtSource};

use crate::{
    covers::Covers,
    state::{EditorState, LogLevel},
    store,
};

struct Job {
    entry: LibraryEntry,
    known_name: Option<String>,
    wants_art: bool,
}

struct Outcome {
    entry: LibraryEntry,
    name: Result<Option<String>, String>,
    /// `None` if no box art was wanted.
    art: Option<Result<Option<Vec<u8>>, String>>,
}

pub struct Fetcher {
    source: Arc<Mutex<BoxArtSource>>,
    task: Option<Task<Vec<Outcome>>>,
    /// Games already tried this session, so failures aren't retried every frame.
    attempted: HashSet<String>,
    /// Games the user explicitly asked for, fetched even when online lookups are off.
    requested: HashSet<String>,
}

impl Default for Fetcher {
    fn default() -> Self {
        let cache = store::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("databases");
        Self {
            source: Arc::new(Mutex::new(BoxArtSource::new(cache))),
            task: None,
            attempted: HashSet::new(),
            requested: HashSet::new(),
        }
    }
}

pub fn boxart_system(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut covers: ResMut<Covers>,
    mut fetcher: Local<Fetcher>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();

    if let Some(outcomes) = fetcher.task.as_mut().and_then(|t| block_on(poll_once(t))) {
        fetcher.task = None;
        apply(&ctx, &mut state, &mut covers, outcomes);
    }

    for sha1 in std::mem::take(&mut state.boxart_retry) {
        covers.unmark_no_boxart(&sha1);
        fetcher.attempted.remove(&sha1);
        fetcher.requested.insert(sha1);
    }

    state.boxart_busy = fetcher.task.is_some();
    if fetcher.task.is_some() {
        return Ok(());
    }

    let jobs: Vec<Job> = state
        .library
        .entries()
        .iter()
        .filter(|e| state.fetch_box_art || fetcher.requested.contains(&e.sha1))
        .filter(|e| !fetcher.attempted.contains(&e.sha1))
        .filter_map(|e| {
            let known_name = state.names.get(&e.sha1).cloned();
            let wants_art = covers.wants_boxart(&e.sha1);
            (known_name.is_none() || wants_art).then(|| Job {
                entry: e.clone(),
                known_name,
                wants_art,
            })
        })
        .collect();
    if jobs.is_empty() {
        return Ok(());
    }

    for job in &jobs {
        fetcher.attempted.insert(job.entry.sha1.clone());
        fetcher.requested.remove(&job.entry.sha1);
    }
    let source = fetcher.source.clone();
    fetcher.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        let mut source = source.lock().unwrap();
        jobs.into_iter()
            .map(|job| {
                let name = match job.known_name {
                    Some(name) => Ok(Some(name)),
                    None => source.identify(&job.entry),
                };
                let art = match (&name, job.wants_art) {
                    (Ok(Some(name)), true) => Some(source.fetch(&job.entry, name)),
                    (Ok(None), true) => Some(Ok(None)),
                    (Err(e), true) => Some(Err(e.clone())),
                    (_, false) => None,
                };
                Outcome {
                    entry: job.entry,
                    name,
                    art,
                }
            })
            .collect()
    }));
    state.boxart_busy = true;
    Ok(())
}

fn apply(
    ctx: &bevy_egui::egui::Context,
    state: &mut EditorState,
    covers: &mut Covers,
    outcomes: Vec<Outcome>,
) {
    let mut names_changed = false;
    for outcome in outcomes {
        let sha1 = &outcome.entry.sha1;
        if let Ok(Some(name)) = &outcome.name
            && state.names.get(sha1) != Some(name)
        {
            state.names.insert(sha1.clone(), name.clone());
            names_changed = true;
        }

        let title = state.title(&outcome.entry);
        match outcome.art {
            Some(Ok(Some(bytes))) => match covers.set_bytes(ctx, sha1, &bytes) {
                Ok(()) => state.log(LogLevel::Info, format!("Found box art for {title}")),
                Err(e) => state.log(LogLevel::Warn, format!("Box art for {title}: {e}")),
            },
            Some(Ok(None)) => {
                covers.mark_no_boxart(sha1);
                state.log(LogLevel::Info, format!("No box art found for {title}"));
            }
            Some(Err(e)) => state.log(LogLevel::Warn, format!("Box art for {title}: {e}")),
            None => {}
        }
    }
    if names_changed {
        state.save();
    }
}

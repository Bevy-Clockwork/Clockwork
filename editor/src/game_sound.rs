//! Plays the sounds and music Play mode asks for: sound and music assets read from the ROM by
//! their plugins' recipes, each made into a playable source once.

use std::collections::HashMap;

use bevy::{audio::AudioSource, prelude::*};
use clockwork_core::asset::OutputRef;

use crate::{
    state::{EditorState, LogLevel},
    workshop::Extractions,
};

pub fn sound_system(
    mut commands: Commands,
    mut state: ResMut<EditorState>,
    mut extractions: ResMut<Extractions>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut made: Local<HashMap<OutputRef, Handle<AudioSource>>>,
) {
    if state.workshop.sounds.is_empty() {
        return;
    }
    let wanted = std::mem::take(&mut state.workshop.sounds);
    let mut later = Vec::new();
    for at in wanted {
        if let Some(handle) = made.get(&at) {
            commands.spawn((AudioPlayer(handle.clone()), PlaybackSettings::DESPAWN));
            continue;
        }
        extractions.ensure(&mut state, &at.plugin);
        if extractions.is_pending(&at.plugin) {
            // Played once its plugin is extracted.
            later.push(at);
            continue;
        }
        let Some(pcm) = extractions
            .get(&at.plugin)
            .and_then(|x| x.sounds.iter().find(|(id, _)| *id == at.output))
            .map(|(_, pcm)| pcm.clone())
        else {
            state.log(LogLevel::Warn, format!("No sound {at}"));
            continue;
        };
        let handle = sources.add(AudioSource {
            bytes: pcm.to_wav().into(),
        });
        made.insert(at, handle.clone());
        commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN));
    }
    state.workshop.sounds.extend(later);
}

/// One part of the music playing: the part played once, or the part repeated after it.
#[derive(Component)]
pub struct MusicPart;

/// A song's part played once (none when it loops from its start) and its loop (none when it
/// ends).
type Parts = (Option<Handle<AudioSource>>, Option<Handle<AudioSource>>);

/// The music played: rendered on another thread the first time (a song takes a while), then
/// its part played once followed by its loop.
#[derive(Default)]
pub struct MusicPlayer {
    /// What should be playing.
    wanted: Option<OutputRef>,
    /// Songs being rendered.
    rendering: Vec<(
        OutputRef,
        std::sync::mpsc::Receiver<Result<clockwork_core::sound::Music, String>>,
    )>,
    /// Each song's part played once and its loop.
    made: HashMap<OutputRef, Parts>,
    /// The part sounding, and whether it is the loop.
    sounding: Option<(Entity, bool)>,
}

pub fn music_system(
    mut commands: Commands,
    mut state: ResMut<EditorState>,
    mut extractions: ResMut<Extractions>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut player: Local<MusicPlayer>,
    parts: Query<Entity, With<MusicPart>>,
) {
    if let Some(wanted) = state.workshop.music.take() {
        if let Some(at) = &wanted {
            extractions.ensure(&mut state, &at.plugin);
            if extractions.is_pending(&at.plugin) {
                // Played once its plugin is extracted.
                state.workshop.music = Some(wanted);
                return;
            }
        }
        if wanted != player.wanted || wanted.is_none() {
            for part in &parts {
                commands.entity(part).despawn();
            }
            player.sounding = None;
            player.wanted = wanted.clone();
        }
        if let Some(at) = wanted.filter(|at| {
            !player.made.contains_key(at) && !player.rendering.iter().any(|(r, _)| r == at)
        }) {
            let Some(song) = extractions
                .get(&at.plugin)
                .and_then(|x| x.music.iter().find(|(id, _)| *id == at.output))
                .map(|(_, song)| song.clone())
            else {
                state.log(LogLevel::Warn, format!("No music {at}"));
                player.wanted = None;
                return;
            };
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = send.send(song.render());
            });
            player.rendering.push((at, receive));
        }
    }
    let mut rendered = Vec::new();
    player
        .rendering
        .retain(|(at, receive)| match receive.try_recv() {
            Ok(result) => {
                rendered.push((at.clone(), result));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
    for (at, result) in rendered {
        match result {
            Ok(music) => {
                let (once, looped) = music.parts();
                let once = (!once.is_empty()).then(|| {
                    sources.add(AudioSource {
                        bytes: music.wav(once).into(),
                    })
                });
                let looped = looped.map(|range| {
                    sources.add(AudioSource {
                        bytes: music.wav(range).into(),
                    })
                });
                player.made.insert(at, (once, looped));
            }
            Err(error) => {
                state.log(LogLevel::Warn, format!("Music {at}: {error}"));
                if player.wanted.as_ref() == Some(&at) {
                    player.wanted = None;
                }
            }
        }
    }
    let Some(at) = player.wanted.clone() else {
        return;
    };
    let Some((once, looped)) = player.made.get(&at).cloned() else {
        return;
    };
    let once_over = match player.sounding {
        None => match once {
            Some(once) => {
                let part = commands
                    .spawn((AudioPlayer(once), PlaybackSettings::DESPAWN, MusicPart))
                    .id();
                player.sounding = Some((part, false));
                false
            }
            None => true,
        },
        Some((part, false)) => parts.get(part).is_err(),
        Some((_, true)) => false,
    };
    // The part played once is over: on to the loop, or done.
    if once_over {
        match looped {
            Some(looped) => {
                let part = commands
                    .spawn((AudioPlayer(looped), PlaybackSettings::LOOP, MusicPart))
                    .id();
                player.sounding = Some((part, true));
            }
            None => player.wanted = None,
        }
    }
}

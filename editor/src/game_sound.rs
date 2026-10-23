//! Plays the sounds Play mode asks for: sound assets read from the ROM by their plugins' recipes,
//! each made into a playable source once.

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

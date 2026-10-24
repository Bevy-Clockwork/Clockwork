//! Play's flow around the world, when the game has a `Flow`: the title screen, choosing a save
//! file, choosing an act on arriving in a course, and game over. Shows the screen of the moment
//! (`ShownScreen`), answers what the player chose on it (`ScreenRequest`), and keeps the save
//! file: loaded into the world when chosen, saved when the player gains something lasting.

use std::collections::BTreeMap;

use bevy::prelude::*;
use clockwork_core::{
    asset::{self, OutputRef, Slot},
    flow::{ActView, FileView, Flow, Label, ScreenAction, ScreenView},
    item::Category,
    save::SaveData,
};
use clockwork_recipe::assets::AssetIndex;
use clockwork_runtime::hud::{ScreenMove, ScreenRequest, ShownScreen};

use crate::{
    entities::Behavior,
    play::PlaySession,
    saves::SaveStore,
    state::{EditorState, LogLevel},
    workshop::Extractions,
};

/// Which screen is up; `World` while the world is played.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    World,
    Title,
    Files,
    /// Choosing an act of the course with this index in the flow's act select.
    Acts(usize),
    GameOver,
}

#[derive(Resource, Default)]
pub struct FlowState {
    flow: Option<Flow>,
    pub stage: Stage,
    store: Option<SaveStore>,
    /// The save file played, from 0.
    slot: Option<usize>,
    /// The highlighted file or act.
    focus: usize,
    /// The course played (its index in the act select) and its act, from 1.
    course: Option<(usize, u8)>,
    /// Where the player stood before arriving in a course: where backing out of it leads.
    came_from: Option<Vec3>,
    /// A chosen file waiting for the played character's items to arrive.
    pending: Option<SaveData>,
    /// The save file as played so far.
    save: SaveData,
    /// The items carried when the file was last saved.
    saved: BTreeMap<String, u32>,
    /// Play stopped to start again (after game over).
    restart: bool,
    /// The course just left, until this frame's save has counted what was gained in it.
    left: Option<(usize, u8)>,
    /// The stage last shown, and the frames left before its screen takes buttons.
    shown_stage: Stage,
    settle: u8,
}

/// How long a new screen ignores the buttons: a quarter second at 60 frames a second.
const SETTLE_FRAMES: u8 = 15;

impl FlowState {
    /// Play starts: the game's flow decides the first screen. Without one, the world.
    pub fn begin(&mut self, state: &mut EditorState) {
        *self = FlowState::default();
        let Some(flow) = crate::play_kit::flow(state) else {
            return;
        };
        if flow.files.is_some() {
            let game = match &state.player {
                Some(player) => player.info.title.clone(),
                None => "workshop".to_string(),
            };
            let data = clockwork_library::store::data_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("clockwork"));
            let store = SaveStore::new(&data, &game);
            // A file that can't be read shows as empty; say why, once.
            let slots = flow.files.as_ref().map_or(0, |f| f.slots as usize);
            for (slot, why) in store.list(slots).errors {
                state.log(LogLevel::Warn, format!("Save file {}: {why}", slot + 1));
            }
            self.store = Some(store);
        }
        self.stage = if flow.title.is_some() {
            Stage::Title
        } else if flow.files.is_some() {
            Stage::Files
        } else {
            Stage::World
        };
        self.flow = Some(flow);
    }

    /// Whether the game has screens around its world: it is played as the game.
    pub fn is_game(&self) -> bool {
        self.flow.is_some()
    }

    /// Whether a screen is up: the world stands still.
    pub fn frozen(&self) -> bool {
        self.stage != Stage::World
    }

    /// The player arrived at the node called `name` (through a door or a teleport) from `from`:
    /// a course's entry opens its act select.
    pub fn arrived(&mut self, name: &str, from: Vec3) {
        let Some(acts) = self.flow.as_ref().and_then(|f| f.acts.as_ref()) else {
            return;
        };
        if let Some(course) = acts.courses.iter().position(|c| c.entry == name) {
            self.stage = Stage::Acts(course);
            self.focus = 0;
            self.came_from = Some(from);
        } else if self.course.is_some() && !name.starts_with(&self.course_prefix()) {
            // Left the course for somewhere else (a star's exit): no act is played any more.
            // What was gained on the way out still counts for it, so it is kept a moment.
            self.left = self.course.take();
        }
    }

    /// Whether the game has its own game over screen.
    pub fn has_game_over(&self) -> bool {
        self.flow.as_ref().is_some_and(|f| f.game_over.is_some())
    }

    /// Out of lives: the game over screen, if the game has one.
    pub fn game_over(&mut self) -> bool {
        if self.flow.as_ref().is_some_and(|f| f.game_over.is_some()) {
            self.stage = Stage::GameOver;
            true
        } else {
            false
        }
    }

    /// The course's node names start like its entry's level (`bob/...`).
    fn course_prefix(&self) -> String {
        let entry = self
            .course
            .and_then(|(c, _)| self.flow.as_ref()?.acts.as_ref()?.courses.get(c))
            .map(|c| c.entry.as_str())
            .unwrap_or_default();
        entry.split('/').next().unwrap_or_default().to_string() + "/"
    }
}

/// Shows the screen of the moment and answers the player's choices on it; gives a chosen file
/// to the world and saves the file when the player gains something lasting.
pub fn flow_system(
    mut state: ResMut<EditorState>,
    mut session: ResMut<PlaySession>,
    mut flow: ResMut<FlowState>,
    mut extractions: ResMut<Extractions>,
    mut shown: ResMut<ShownScreen>,
    mut requests: MessageReader<ScreenRequest>,
    time: Res<Time>,
) {
    if state.workshop.playing.is_none() || flow.flow.is_none() {
        if shown.view.is_some() {
            *shown = ShownScreen::default();
        }
        requests.clear();
        if state.workshop.playing.is_none() && std::mem::take(&mut flow.restart) {
            state.workshop.play_request = Some(true);
        }
        return;
    }
    // A new screen waits a moment for the buttons, so the press that left the last one (still
    // held, or arriving in a course straight after choosing a file) doesn't choose on it too.
    if flow.stage != flow.shown_stage {
        flow.shown_stage = flow.stage;
        flow.settle = SETTLE_FRAMES;
    }
    if flow.settle > 0 {
        flow.settle -= 1;
        requests.clear();
    }
    for request in requests.read() {
        answer(*request, &mut flow, &mut session, &mut state);
    }
    // A chosen file goes into the world once the character carries its items.
    if flow.pending.is_some() && session.inventory().is_some() {
        let save = flow.pending.take().unwrap_or_default();
        give(&save, &mut session);
        flow.saved = save.items.clone();
        flow.save = save;
    }
    if flow.stage == Stage::World && flow.slot.is_some() {
        flow.save.seconds += f64::from(time.delta_secs());
    }
    keep(&mut flow, &mut session, &mut state);
    flow.left = None;
    let view = view(&flow, &mut state, &mut extractions);
    let counted = flow
        .flow
        .as_ref()
        .and_then(|f| f.files.as_ref())
        .and_then(|f| f.counts.clone());
    if shown.view != view || shown.counted_item != counted {
        shown.view = view;
        shown.counted_item = counted;
    }
}

/// What a choice on a screen does.
fn answer(
    request: ScreenRequest,
    flow: &mut FlowState,
    session: &mut PlaySession,
    state: &mut EditorState,
) {
    let Some(game) = flow.flow.clone() else {
        return;
    };
    let choices = match flow.stage {
        Stage::Files => game.files.as_ref().map_or(0, |f| f.slots as usize),
        Stage::Acts(course) => game
            .acts
            .as_ref()
            .and_then(|a| a.courses.get(course))
            .map_or(0, |c| c.acts.len()),
        _ => 0,
    };
    match request {
        ScreenRequest::Move(ScreenMove::Up) => flow.focus = flow.focus.saturating_sub(1),
        ScreenRequest::Move(ScreenMove::Down) => {
            flow.focus = (flow.focus + 1).min(choices.saturating_sub(1))
        }
        ScreenRequest::Action(ScreenAction::Start) => {
            flow.focus = 0;
            flow.stage = if game.files.is_some() {
                Stage::Files
            } else {
                start(&game, session, state);
                Stage::World
            };
        }
        ScreenRequest::Action(ScreenAction::File(slot)) => {
            let loaded = flow.store.as_ref().map(|s| s.load(slot));
            let save = match loaded {
                Some(Ok(save)) => save.unwrap_or_default(),
                Some(Err(why)) => {
                    // A file that can't be read isn't played over: it stays as it is.
                    state.log(LogLevel::Warn, why.clone());
                    state.workshop.toast = Some((why, 4.0));
                    return;
                }
                None => SaveData::default(),
            };
            flow.slot = Some(slot);
            flow.pending = Some(save);
            start(&game, session, state);
            flow.stage = Stage::World;
        }
        ScreenRequest::Action(ScreenAction::Act(act)) => {
            if let Stage::Acts(course) = flow.stage {
                let act = act as u8 + 1;
                session.set_act(Some(act));
                flow.course = Some((course, act));
                flow.stage = Stage::World;
            }
        }
        ScreenRequest::Action(ScreenAction::Back) => match flow.stage {
            Stage::Files if game.title.is_some() => flow.stage = Stage::Title,
            Stage::Acts(_) => {
                // Not entering after all: back where the player came from.
                if let Some(at) = flow.came_from.take() {
                    session.teleport(at);
                }
                flow.stage = Stage::World;
            }
            _ => {}
        },
        ScreenRequest::Action(ScreenAction::Continue) => {
            // A new start: Play stops and starts again, the world as it was and the flow from
            // its first screen, where the file is chosen again.
            flow.restart = true;
            state.workshop.play_request = Some(false);
        }
    }
}

/// The game begins: the played character goes to the flow's start, if it names one.
fn start(game: &Flow, session: &mut PlaySession, state: &mut EditorState) {
    let Some(name) = &game.start else {
        return;
    };
    let scene = &state.workshop.scene;
    match scene.nodes.iter().find(|n| n.name == *name) {
        Some(node) => session.start_at(scene.world_transform(node.id).translation),
        None => state.log(
            LogLevel::Warn,
            format!("The game starts at \"{name}\", and no node is called that"),
        ),
    }
}

/// Puts a save file into the world: what the character carries, the flags, what is gone.
fn give(save: &SaveData, session: &mut PlaySession) {
    if let Some(inventory) = session.inventory_mut() {
        for (item, count) in &save.items {
            inventory.add(item, *count);
        }
    }
    let Some(entities) = session.entities_mut() else {
        return;
    };
    entities.flags = save.flags.clone();
    let gone: Vec<_> = entities
        .nodes
        .iter()
        .filter(|n| save.gone.contains(&n.name))
        .map(|n| n.id)
        .collect();
    for id in gone {
        entities.consume(id);
    }
}

/// Saves the file when the character has gained something lasting (anything but consumables:
/// stars, keys), marking the course's act done when it was the flow's counted item.
fn keep(flow: &mut FlowState, session: &mut PlaySession, state: &mut EditorState) {
    let (Some(slot), Some(store)) = (flow.slot, flow.store.as_ref()) else {
        return;
    };
    if flow.pending.is_some() {
        return;
    }
    let Some(inventory) = session.inventory() else {
        return;
    };
    let items: BTreeMap<String, u32> = inventory
        .items()
        .iter()
        .map(|i| (i.id.clone(), inventory.count(&i.id)))
        .filter(|(_, n)| *n > 0)
        .collect();
    let lasting = |id: &str| {
        inventory
            .item(id)
            .is_some_and(|i| i.category != Category::Consumable)
    };
    let gained: Vec<String> = items
        .iter()
        .filter(|(id, n)| lasting(id) && **n > flow.saved.get(*id).copied().unwrap_or(0))
        .map(|(id, _)| id.clone())
        .collect();
    if gained.is_empty() {
        return;
    }
    let counted = flow
        .flow
        .as_ref()
        .and_then(|f| f.files.as_ref())
        .and_then(|f| f.counts.clone());
    if let Some((course, act)) = flow.course.or(flow.left)
        && counted.as_ref().is_some_and(|c| gained.contains(c))
        && let Some(entry) = flow
            .flow
            .as_ref()
            .and_then(|f| f.acts.as_ref())
            .and_then(|a| a.courses.get(course))
            .map(|c| c.entry.clone())
    {
        flow.save.acts_done.entry(entry).or_default().insert(act);
    }
    // What stays gone: things that gave something lasting, and beaten bosses (several hits).
    let scene = &state.workshop.scene;
    let keeps = |name: &str| {
        scene.nodes.iter().filter(|n| n.name == name).any(|n| {
            n.behaviors.iter().any(|b| match &b.behavior {
                Behavior::Pickup { item, .. } => lasting(item),
                Behavior::Target { health, .. } => *health > 1.0,
                _ => false,
            })
        })
    };
    let entities = session.entities();
    flow.save.items = items.clone();
    flow.save.flags = entities.flags.clone();
    flow.save.gone = entities
        .nodes
        .iter()
        .filter(|n| n.gone && keeps(&n.name))
        .map(|n| n.name.clone())
        .collect();
    match store.save(slot, &flow.save) {
        Ok(()) => flow.saved = items,
        Err(why) => state.log(LogLevel::Warn, why),
    }
}

/// The screen of the moment, its words looked up.
fn view(
    flow: &FlowState,
    state: &mut EditorState,
    extractions: &mut Extractions,
) -> Option<ScreenView> {
    let game = flow.flow.as_ref()?;
    match flow.stage {
        Stage::World => None,
        Stage::Title => {
            let title = game.title.as_ref()?;
            Some(ScreenView::Title {
                logo: title.logo.clone(),
                logo_camera: title.logo_camera,
                prompt: words(&title.prompt, state, extractions),
                background: title.background,
            })
        }
        Stage::Files => {
            let files = game.files.as_ref()?;
            let listed = flow.store.as_ref()?.list(files.slots as usize);
            let file_word = crate::play::words(state).file;
            let slots = listed
                .slots
                .iter()
                .enumerate()
                .map(|(k, save)| FileView {
                    name: clockwork_core::ui::fill(
                        &file_word,
                        &[("letter", &((b'A' + k as u8) as char).to_string())],
                    ),
                    empty: save.as_ref().is_none_or(SaveData::is_new),
                    count: files.counts.as_ref().and_then(|item| {
                        save.as_ref()
                            .map(|s| s.items.get(item).copied().unwrap_or(0))
                    }),
                })
                .collect();
            Some(ScreenView::Files {
                title: words(&files.title, state, extractions),
                slots,
                focus: flow.focus,
            })
        }
        Stage::Acts(course) => {
            let course = game.acts.as_ref()?.courses.get(course)?;
            let done = flow.save.acts_done.get(&course.entry);
            Some(ScreenView::Acts {
                course: words(&course.name, state, extractions),
                acts: course
                    .acts
                    .iter()
                    .enumerate()
                    .map(|(k, name)| ActView {
                        name: words(name, state, extractions),
                        done: done.is_some_and(|d| d.contains(&(k as u8 + 1))),
                    })
                    .collect(),
                focus: flow.focus,
            })
        }
        Stage::GameOver => Some(ScreenView::GameOver {
            text: words(&game.game_over.as_ref()?.text, state, extractions),
        }),
    }
}

/// A label's words: written in place, a dialog asset's first page, or the first page of a
/// dialog whose pages come from the ROM (empty until its plugin is extracted).
fn words(label: &Label, state: &mut EditorState, extractions: &mut Extractions) -> String {
    let at: OutputRef = match label {
        Slot::Inline(dialog) => return dialog.pages.first().cloned().unwrap_or_default(),
        Slot::Ref(at) => at.clone(),
    };
    let found = {
        let index = AssetIndex::build(&state.workshop.plugins);
        asset::resolve(
            &[(asset::Layer::Asset, Some(label))],
            &index,
            "dialog",
            asset::dialog_of,
        )
    };
    if let Ok(Some(found)) = found
        && let Some(page) = found.value.pages.first()
    {
        return page.clone();
    }
    extractions.ensure(state, &at.plugin);
    extractions
        .get(&at.plugin)
        .and_then(|x| x.dialogs.iter().find(|(id, _)| *id == at.output))
        .and_then(|(_, d)| d.pages.first().cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::flow::{ActSelect, Course, FileSelect, GameOver, TitleScreen};

    fn words(text: &str) -> Label {
        Slot::Inline(clockwork_core::ui::Dialog {
            speaker: None,
            pages: vec![text.into()],
        })
    }

    fn game() -> FlowState {
        FlowState {
            flow: Some(Flow {
                title: Some(TitleScreen {
                    logo: None,
                    prompt: words("PRESS START"),
                    background: clockwork_core::ui::Rgba(0, 0, 0, 255),
                    logo_camera: None,
                }),
                files: Some(FileSelect {
                    title: words("FILES"),
                    slots: 4,
                    counts: Some("star".into()),
                }),
                acts: Some(ActSelect {
                    courses: vec![Course {
                        name: words("HILLS"),
                        entry: "hills/1/warp 10".into(),
                        acts: vec![words("ONE"), words("TWO"), words("THREE")],
                    }],
                }),
                game_over: Some(GameOver {
                    text: words("GAME OVER"),
                }),
                start: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn arriving_at_a_course_entry_asks_for_an_act_and_leaving_keeps_it_a_moment() {
        let mut flow = game();
        flow.arrived("castle/1/warp 3", Vec3::ZERO);
        assert_eq!(flow.stage, Stage::World);
        flow.arrived("hills/1/warp 10", Vec3::X);
        assert_eq!(flow.stage, Stage::Acts(0));
        assert_eq!(flow.came_from, Some(Vec3::X));
        assert!(flow.frozen());
        // Inside the course, moving between its own nodes keeps the act.
        flow.stage = Stage::World;
        flow.course = Some((0, 2));
        flow.arrived("hills/2/warp 5", Vec3::ZERO);
        assert_eq!(flow.course, Some((0, 2)));
        // Leaving it (a star's exit) ends the act but keeps it for this frame's save.
        flow.arrived("castle/1/warp 50", Vec3::ZERO);
        assert_eq!((flow.course, flow.left), (None, Some((0, 2))));
    }

    #[test]
    fn game_over_shows_its_screen_only_when_the_flow_has_one() {
        let mut flow = game();
        assert!(flow.game_over());
        assert_eq!(flow.stage, Stage::GameOver);
        let mut none = FlowState::default();
        assert!(!none.game_over());
        assert_eq!(none.stage, Stage::World);
    }
}

//! What Play checks when it starts, so broken content is reported rather than played wrong: the
//! content check of the game's plugin (names that exist, fonts that have the letters of the
//! words they write, items the HUD counts), and the scene's names: doors, teleports, the goal,
//! conditions and the courses' entries each lead to one node of that name.

use clockwork_core::{flow::Flow, game::Goal};
use clockwork_recipe::{assets::AssetIndex, check};
use clockwork_runtime::play::check::scene_names;

use crate::entities::Entities;
use crate::state::{EditorState, LogLevel};

/// Logs every problem and says how many there are; returns them.
pub fn report(
    state: &mut EditorState,
    entities: &Entities,
    goal: Option<&Goal>,
    flow: Option<&Flow>,
) -> Vec<String> {
    let mut problems: Vec<String> = match state.workshop.scene.game.preset.clone() {
        Some(preset) => {
            let index = AssetIndex::build(&state.workshop.plugins);
            check::check(&index, Some(&preset.plugin))
                .iter()
                .map(|p| p.to_string())
                .collect()
        }
        None => Vec::new(),
    };
    let names: Vec<&str> = state
        .workshop
        .scene
        .nodes
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    problems.extend(scene_names(&names, entities, goal, flow));
    for problem in &problems {
        state.log(LogLevel::Warn, format!("Content check: {problem}"));
    }
    if !problems.is_empty() {
        state.workshop.toast = Some((
            format!("{} content problems: see the log", problems.len()),
            4.0,
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ROOT, Scene};
    use bevy::math::Vec3;
    use clockwork_core::behavior::{Behavior, Condition, NodeBehavior};

    #[test]
    fn names_must_lead_to_one_node() {
        let mut scene = Scene::default();
        let door = scene.add(ROOT, "Door".into(), None, Vec3::ZERO);
        scene.add(ROOT, "Star".into(), None, Vec3::ZERO);
        scene.add(ROOT, "Star".into(), None, Vec3::ZERO);
        scene
            .get_mut(door)
            .unwrap()
            .behaviors
            .push(NodeBehavior::new(Behavior::Door {
                radius: 1.0,
                to: "Hall".into(),
                fade: 0.0,
            }));
        let entities = crate::entities::build(&scene);
        let names: Vec<&str> = scene.nodes.iter().map(|n| n.name.as_str()).collect();
        let goal = Goal::When(Condition::Gone("Star".into()));
        assert_eq!(
            scene_names(&names, &entities, Some(&goal), None),
            [
                "the door of \"Door\" leads to \"Hall\", and no node is called that",
                "the goal leads to \"Star\", and 2 nodes are called that",
            ]
        );
    }
}

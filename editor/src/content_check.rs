//! What Play checks when it starts, so broken content is reported rather than played wrong: the
//! content check of the game's plugin (names that exist, fonts that have the letters of the
//! words they write, items the HUD counts), and the scene's names: doors, teleports, the goal,
//! conditions and the courses' entries each lead to one node of that name.

use std::collections::{BTreeMap, BTreeSet};

use clockwork_core::{
    behavior::{Behavior, Condition, Effect},
    flow::Flow,
    game::Goal,
};
use clockwork_recipe::{assets::AssetIndex, check};

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

/// Names that lead nowhere, or to more than one node.
pub fn scene_names(
    names: &[&str],
    entities: &Entities,
    goal: Option<&Goal>,
    flow: Option<&Flow>,
) -> Vec<String> {
    // What leads to a node by name, with what it is.
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut problems = BTreeSet::new();
    let by_node = |id| {
        entities
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(String::new(), |n| n.name.clone())
    };
    for live in &entities.live {
        let from = by_node(live.node);
        match &live.behavior {
            Behavior::Door { to, .. } if !to.is_empty() => {
                wanted.push((to.clone(), format!("the door of \"{from}\"")))
            }
            Behavior::Trigger { effect, .. } => {
                teleports(effect, &mut |to| {
                    wanted.push((to.to_string(), format!("a teleport of \"{from}\"")))
                });
                script_problems(effect, &mut |problem| {
                    problems.insert(format!("a script of \"{from}\": {problem}"));
                });
            }
            Behavior::Actor(actor) => {
                for problem in actor.problems() {
                    problems.insert(format!("the actor of \"{from}\": {problem}"));
                }
                for effect in actor.states.iter().filter_map(|s| s.enter.as_ref()) {
                    teleports(effect, &mut |to| {
                        wanted.push((to.to_string(), format!("a teleport of \"{from}\"")))
                    });
                }
            }
            _ => {}
        }
        if let Some(when) = &live.when {
            gone(when, &mut |name| {
                wanted.push((name.to_string(), format!("a condition of \"{from}\"")))
            });
        }
    }
    if let Some(Goal::Reach(name)) = goal {
        wanted.push((name.clone(), "the goal".to_string()));
    }
    if let Some(Goal::When(when)) = goal {
        gone(when, &mut |name| {
            wanted.push((name.to_string(), "the goal".to_string()))
        });
    }
    if let Some(start) = flow.and_then(|f| f.start.as_ref()) {
        wanted.push((start.clone(), "the game's start".to_string()));
    }
    for course in flow.iter().flat_map(|f| &f.acts).flat_map(|a| &a.courses) {
        wanted.push((course.entry.clone(), "a course's entry".to_string()));
    }

    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for name in names {
        *count.entry(name).or_default() += 1;
    }
    for (name, what) in wanted {
        match count.get(name.as_str()).copied().unwrap_or(0) {
            0 => problems.insert(format!(
                "{what} leads to \"{name}\", and no node is called that"
            )),
            1 => false,
            n => problems.insert(format!(
                "{what} leads to \"{name}\", and {n} nodes are called that"
            )),
        };
    }
    problems.into_iter().collect()
}

/// The nodes an effect leads to by name: teleports, and the nodes its scripts walk and place.
fn teleports(effect: &Effect, found: &mut dyn FnMut(&str)) {
    match effect {
        Effect::Teleport(to) => found(to),
        Effect::All(effects) => effects.iter().for_each(|e| teleports(e, found)),
        Effect::Run(script) => {
            script.nodes().into_iter().for_each(&mut *found);
            script
                .effects()
                .into_iter()
                .for_each(|e| teleports(e, found));
        }
        _ => {}
    }
}

/// What is wrong with the scripts an effect runs.
fn script_problems(effect: &Effect, found: &mut dyn FnMut(String)) {
    match effect {
        Effect::All(effects) => effects.iter().for_each(|e| script_problems(e, found)),
        Effect::Run(script) => {
            script.problems().into_iter().for_each(&mut *found);
            script
                .effects()
                .into_iter()
                .for_each(|e| script_problems(e, found));
        }
        _ => {}
    }
}

fn gone(condition: &Condition, found: &mut dyn FnMut(&str)) {
    match condition {
        Condition::Gone(name) => found(name),
        Condition::Not(c) => gone(c, found),
        Condition::All(cs) | Condition::Any(cs) => cs.iter().for_each(|c| gone(c, found)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ROOT, Scene};
    use bevy::math::Vec3;
    use clockwork_core::behavior::NodeBehavior;

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

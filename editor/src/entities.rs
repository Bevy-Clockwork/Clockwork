//! The scene's behaviors in Play: the runtime runs them (`clockwork_runtime::play::entities`);
//! this hands it the scene's nodes.

use crate::scene::Scene;

pub use clockwork_runtime::play::entities::{
    Behavior, Condition, Effect, Entities, NodeBehavior, Placed,
};

/// The scene's shown nodes as Play places them: where each stands in the world and what it
/// does.
pub fn placed(scene: &Scene) -> Vec<Placed> {
    scene
        .nodes
        .iter()
        .filter(|n| scene.is_shown(n.id))
        .map(|n| Placed {
            id: n.id,
            name: n.name.clone(),
            at: scene.world_transform(n.id).translation,
            behaviors: n.behaviors.clone(),
            appears: n.appears.clone(),
            acts: n.acts.clone(),
        })
        .collect()
}

/// The behaviors of the scene's shown nodes.
pub fn build(scene: &Scene) -> Entities {
    Entities::build(placed(scene))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ROOT;
    use bevy::math::Vec3;

    #[test]
    fn hidden_nodes_have_no_behavior_and_children_stand_where_their_parents_put_them() {
        let mut scene = Scene::default();
        let parent = scene.add(ROOT, "Room".into(), None, Vec3::new(10.0, 0.0, 0.0));
        let child = scene.add(parent, "Chest".into(), None, Vec3::new(1.0, 0.0, 0.0));
        let hidden = scene.add(ROOT, "Hidden".into(), None, Vec3::ZERO);
        for id in [child, hidden] {
            scene.get_mut(id).unwrap().behaviors =
                vec![NodeBehavior::new(Behavior::new("Target").unwrap())];
        }
        scene.get_mut(hidden).unwrap().visible = false;
        let entities = build(&scene);
        assert_eq!(entities.live.len(), 1);
        assert_eq!(entities.live[0].node, child);
        assert_eq!(
            entities.live[0].at,
            scene.world_transform(child).translation
        );
    }

    #[test]
    fn old_scenes_default_to_all_acts_and_act_lists_roundtrip() {
        let old = r#"(nodes: [(id: 0, name: "Scene", parent: None, item: None,
            position: (0.0, 0.0, 0.0), yaw: 0.0, scale: 1.0, visible: true)], next_id: 1)"#;
        let mut scene: Scene = ron::from_str(old).unwrap();
        assert_eq!(scene.nodes[0].acts, None);
        assert!(!ron::to_string(&scene).unwrap().contains("acts:"));
        for acts in [vec![1, 3, 9], vec![]] {
            scene.nodes[0].acts = Some(acts.clone());
            let saved = ron::to_string(&scene).unwrap();
            assert!(saved.contains("acts:Some(["));
            let loaded: Scene = ron::from_str(&saved).unwrap();
            assert_eq!(loaded.nodes[0].acts, Some(acts));
        }
    }
}

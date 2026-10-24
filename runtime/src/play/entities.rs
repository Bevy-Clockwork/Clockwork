//! Behaviors that make scene nodes do things when played: targets that can be hit and destroyed,
//! pickups, triggers, checkpoints, solid obstacles, hazards and doors. A behavior is generic (it
//! knows nothing of any game); what a node looks like is whatever plugin output it shows.
//!
//! A node can have several behaviors (a target that also hurts), each working while its
//! condition holds (a door that opens with a key), and can appear only once its own condition
//! holds (a star after the boss). A node's `acts` restricts it to those acts while keeping
//! what appeared or was taken away. [`Behavior`] is scene data (and can be an asset, see
//! `clockwork_core::behavior`). [`Entities`] is the running state while playing: pure, so it can
//! be tested without a window. It knows a scene only as the [`Placed`] nodes it is built from.

use std::collections::{BTreeSet, HashSet};

use bevy::math::{Vec2, Vec3};

use super::NodeId;
use crate::actions::Shape;

pub use clockwork_core::behavior::{Behavior, Condition, Effect, Facts, NodeBehavior};

/// A node of the played scene, as behaviors see it: its name, where it stands in the world and
/// what it does. Only nodes that are shown are placed.
#[derive(Debug, Clone, Default)]
pub struct Placed {
    pub id: NodeId,
    pub name: String,
    /// Where it stands in the world.
    pub at: Vec3,
    pub behaviors: Vec<NodeBehavior>,
    /// It appears once this holds; there from the start without one.
    pub appears: Option<Condition>,
    /// The acts it is there in; every act without a list.
    pub acts: Option<Vec<u8>>,
}

/// One behavior at work.
#[derive(Debug, Clone)]
pub struct LiveEntity {
    pub node: NodeId,
    pub behavior: Behavior,
    pub when: Option<Condition>,
    /// How far above and below it reaches.
    reach: f32,
    /// Where it is (ground plane, then height).
    pub at: Vec3,
    pub health: f32,
    pub max_health: f32,
    /// Its node was destroyed or picked up: gone until play stops.
    pub gone: bool,
    /// Working now: its node is there and its condition holds.
    pub active: bool,
    /// Seconds left of its being-hit flash.
    pub flash: f32,
    fired: bool,
    inside: bool,
}

impl LiveEntity {
    fn ground(&self) -> Vec2 {
        Vec2::new(self.at.x, self.at.z)
    }

    /// There and working.
    pub fn working(&self) -> bool {
        self.active && !self.gone
    }
}

/// A node whose behaviors are at work, or that waits to appear.
#[derive(Debug, Clone)]
pub struct LiveNode {
    pub id: NodeId,
    pub name: String,
    appears: Option<Condition>,
    acts: Option<Vec<u8>>,
    /// It has appeared (nodes without a condition are there from the start).
    pub shown: bool,
    /// Destroyed or picked up.
    pub gone: bool,
}

impl LiveNode {
    /// There in this act, once it has appeared and until it is taken away.
    pub fn present(&self, act: Option<u8>) -> bool {
        self.shown
            && !self.gone
            && act.is_none_or(|act| self.acts.as_ref().is_none_or(|acts| acts.contains(&act)))
    }
}

/// What the player's walking around caused.
#[derive(Debug, Clone, PartialEq)]
pub enum Touch {
    Pickup {
        node: NodeId,
        item: String,
        count: u32,
    },
    Trigger {
        node: NodeId,
        effect: Effect,
    },
    Checkpoint {
        at: Vec3,
    },
    /// Standing in a hazard: hurts every frame (the player's hurt time spaces the hits).
    Hazard {
        damage: f32,
    },
    /// Entering a door.
    Door {
        to: String,
        fade: f32,
    },
}

/// What a hit did.
#[derive(Debug, Default, PartialEq)]
pub struct HitOutcome {
    /// It touched something that can be hit.
    pub landed: bool,
    /// Targets destroyed by it, with what they drop.
    pub destroyed: Vec<(NodeId, Option<(String, u32)>)>,
}

/// How long a target flashes after being hit, in seconds.
const FLASH: f32 = 0.2;

#[derive(Debug, Default)]
pub struct Entities {
    pub live: Vec<LiveEntity>,
    pub nodes: Vec<LiveNode>,
    /// Flags raised by effects (see [`Effect::Set`]).
    pub flags: BTreeSet<String>,
    /// The current act; None leaves every node's act restriction open.
    pub act: Option<u8>,
    /// (projectile, node): a projectile hurts a target once.
    memo: HashSet<(u64, NodeId)>,
}

/// The facts conditions are judged by: the entities' flags and gone nodes, and what the player
/// carries.
struct Judge<'a> {
    entities: &'a Entities,
    carried: &'a dyn Fn(&str) -> u32,
}

impl Facts for Judge<'_> {
    fn count(&self, item: &str) -> u32 {
        (self.carried)(item)
    }

    fn flag(&self, name: &str) -> bool {
        self.entities.flags.contains(name)
    }

    fn gone(&self, node: &str) -> bool {
        self.entities.nodes.iter().any(|n| n.gone && n.name == node)
    }

    fn act(&self) -> Option<u8> {
        self.entities.act
    }
}

impl Entities {
    /// The behaviors of the placed nodes, where they stand. Nothing works until
    /// [`Entities::refresh`] judges the conditions.
    pub fn build(placed: impl IntoIterator<Item = Placed>) -> Self {
        let mut live = Vec::new();
        let mut nodes = Vec::new();
        for n in placed {
            if n.behaviors.is_empty() && n.appears.is_none() && n.acts.is_none() {
                continue;
            }
            for b in &n.behaviors {
                let health = match &b.behavior {
                    Behavior::Target { health, .. } => *health,
                    _ => 0.0,
                };
                live.push(LiveEntity {
                    node: n.id,
                    behavior: b.behavior.clone(),
                    when: b.when.clone(),
                    reach: b.reach(),
                    at: n.at,
                    health,
                    max_health: health,
                    gone: false,
                    active: false,
                    flash: 0.0,
                    fired: false,
                    inside: false,
                });
            }
            nodes.push(LiveNode {
                id: n.id,
                shown: n.appears.is_none(),
                name: n.name,
                appears: n.appears,
                acts: n.acts,
                gone: false,
            });
        }
        Self {
            live,
            nodes,
            flags: BTreeSet::new(),
            act: None,
            memo: HashSet::new(),
        }
    }

    pub fn update(&mut self, dt: f32) {
        for e in &mut self.live {
            e.flash = (e.flash - dt).max(0.0);
        }
    }

    /// Chooses an act without resetting progress. Refresh conditions before behaviors run again.
    pub fn set_act(&mut self, act: Option<u8>) {
        if self.act != act {
            self.act = act;
            for e in &mut self.live {
                e.active = false;
            }
        }
    }

    /// Judges every condition again: nodes appear (for good) once theirs holds, behaviors work
    /// while theirs does. `carried` says how many of an item the player has.
    pub fn refresh(&mut self, carried: &dyn Fn(&str) -> u32) {
        let judge = Judge {
            entities: self,
            carried,
        };
        let appeared: Vec<bool> = self
            .nodes
            .iter()
            .map(|n| n.shown || n.appears.as_ref().is_some_and(|c| c.holds(&judge)))
            .collect();
        let holding: Vec<bool> = self
            .live
            .iter()
            .map(|e| e.when.as_ref().is_none_or(|c| c.holds(&judge)))
            .collect();
        for (n, shown) in self.nodes.iter_mut().zip(appeared) {
            n.shown = shown;
        }
        let shown: HashSet<NodeId> = self
            .nodes
            .iter()
            .filter(|n| n.present(self.act))
            .map(|n| n.id)
            .collect();
        for (e, holds) in self.live.iter_mut().zip(holding) {
            e.active = holds && shown.contains(&e.node);
            if !e.active {
                e.inside = false;
            }
        }
    }

    /// Whether a condition holds now.
    pub fn holds(&self, condition: &Condition, carried: &dyn Fn(&str) -> u32) -> bool {
        condition.holds(&Judge {
            entities: self,
            carried,
        })
    }

    /// Raises or lowers a flag.
    pub fn set_flag(&mut self, name: &str, up: bool) {
        if up {
            self.flags.insert(name.to_string());
        } else {
            self.flags.remove(name);
        }
    }

    /// Whether a point on the ground is inside something solid.
    pub fn blocked(&self, p: Vec2) -> bool {
        self.live.iter().any(|e| {
            e.working()
                && matches!(
                    e.behavior,
                    Behavior::Solid { .. } | Behavior::Target { solid: true, .. }
                )
                && e.ground().distance(p) < e.behavior.radius()
        })
    }

    /// A hit of `damage` over `shape`; `projectile` is the id of the projectile that did it, so
    /// it hurts a target once. A destroyed target takes its node, and its other behaviors, away.
    pub fn apply_hit(&mut self, shape: &Shape, damage: f32, projectile: Option<u64>) -> HitOutcome {
        let mut outcome = HitOutcome::default();
        for e in &mut self.live {
            let Behavior::Target { radius, drop, .. } = &e.behavior else {
                continue;
            };
            if !e.working() || !shape.touches(Vec2::new(e.at.x, e.at.z), *radius) {
                continue;
            }
            if let Some(id) = projectile
                && !self.memo.insert((id, e.node))
            {
                continue;
            }
            outcome.landed = true;
            e.health -= damage;
            e.flash = FLASH;
            if e.health <= 0.0 {
                outcome.destroyed.push((e.node, drop.clone()));
            }
        }
        for (node, _) in &outcome.destroyed {
            self.consume(*node);
        }
        outcome
    }

    /// What the player at `at` (with a body of `radius`) touches now: within a behavior's
    /// radius on the ground and its reach up and down. Pickups are only offered: the caller
    /// decides whether the inventory takes them and then calls [`Entities::consume`].
    pub fn touch(&mut self, at: Vec3, radius: f32) -> Vec<Touch> {
        let ground = Vec2::new(at.x, at.z);
        let mut touches = Vec::new();
        for e in &mut self.live {
            if !e.working() {
                e.inside = false;
                continue;
            }
            let inside = e.ground().distance(ground) <= e.behavior.radius() + radius
                && (e.at.y - at.y).abs() <= e.reach;
            match &e.behavior {
                Behavior::Pickup { item, count, .. } if inside && !item.is_empty() => {
                    touches.push(Touch::Pickup {
                        node: e.node,
                        item: item.clone(),
                        count: *count,
                    });
                }
                Behavior::Trigger { effect, once, .. } => {
                    if inside && !e.inside && !(*once && e.fired) {
                        e.fired = true;
                        touches.push(Touch::Trigger {
                            node: e.node,
                            effect: effect.clone(),
                        });
                    }
                }
                Behavior::Checkpoint { .. } if inside && !e.inside => {
                    touches.push(Touch::Checkpoint { at: e.at });
                }
                Behavior::Hazard { damage, .. } if inside => {
                    touches.push(Touch::Hazard { damage: *damage });
                }
                Behavior::Door { to, fade, .. } if inside && !e.inside && !to.is_empty() => {
                    touches.push(Touch::Door {
                        to: to.clone(),
                        fade: *fade,
                    });
                }
                _ => {}
            }
            e.inside = inside;
        }
        touches
    }

    /// Takes a node away (picked up or destroyed), with every behavior it has.
    pub fn consume(&mut self, node: NodeId) {
        for e in self.live.iter_mut().filter(|e| e.node == node) {
            e.gone = true;
        }
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == node) {
            n.gone = true;
        }
    }

    /// The nodes that are gone.
    pub fn gone(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().filter(|n| n.gone).map(|n| n.id)
    }

    /// The nodes to hide while playing: gone, not there yet, or out of this act.
    pub fn hidden(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .filter(|n| !n.present(self.act))
            .map(|n| n.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: NodeId = 0;

    /// The nodes of a scene, added as the Workshop adds them.
    #[derive(Default)]
    struct Scene {
        nodes: Vec<Placed>,
    }

    impl Scene {
        fn add(&mut self, _parent: NodeId, name: String, _item: Option<()>, at: Vec3) -> NodeId {
            let id = self.nodes.len() as NodeId + 1;
            self.nodes.push(Placed {
                id,
                name,
                at,
                ..Default::default()
            });
            id
        }

        fn get_mut(&mut self, id: NodeId) -> Option<&mut Placed> {
            self.nodes.iter_mut().find(|n| n.id == id)
        }
    }

    fn scene_with(behaviors: Vec<(Vec3, Behavior)>) -> (Scene, Vec<NodeId>) {
        let mut scene = Scene::default();
        let ids = behaviors
            .into_iter()
            .map(|(at, b)| {
                let id = scene.add(ROOT, b.kind().into(), None, at);
                scene.get_mut(id).unwrap().behaviors = vec![NodeBehavior::new(b)];
                id
            })
            .collect();
        (scene, ids)
    }

    fn entities(behaviors: Vec<(Vec3, Behavior)>) -> (Entities, Vec<NodeId>) {
        let (scene, ids) = scene_with(behaviors);
        let mut e = Entities::build(scene.nodes.clone());
        e.refresh(&|_| 0);
        (e, ids)
    }

    fn circle(x: f32, z: f32, radius: f32) -> Shape {
        Shape::Circle {
            center: Vec2::new(x, z),
            radius,
        }
    }

    #[test]
    fn targets_take_damage_flash_and_drop_things_when_destroyed() {
        let target = Behavior::Target {
            health: 3.0,
            radius: 0.5,
            solid: true,
            drop: Some(("bombs".into(), 2)),
        };
        let (mut e, ids) = entities(vec![(Vec3::new(2.0, 0.0, 0.0), target)]);
        assert!(!e.apply_hit(&circle(5.0, 5.0, 1.0), 1.0, None).landed);
        let outcome = e.apply_hit(&circle(2.0, 0.0, 0.3), 2.0, None);
        assert!(outcome.landed && outcome.destroyed.is_empty());
        assert!(e.live[0].flash > 0.0);
        let outcome = e.apply_hit(&circle(2.0, 0.0, 0.3), 2.0, None);
        assert_eq!(
            outcome.destroyed,
            vec![(ids[0], Some(("bombs".to_string(), 2)))]
        );
        assert!(e.live[0].gone);
        // Gone targets are no longer hit or in the way.
        assert!(!e.apply_hit(&circle(2.0, 0.0, 0.3), 2.0, None).landed);
        assert!(!e.blocked(Vec2::new(2.0, 0.0)));
    }

    #[test]
    fn a_projectile_hurts_a_target_only_once() {
        let target = Behavior::Target {
            health: 10.0,
            radius: 0.5,
            solid: false,
            drop: None,
        };
        let (mut e, _) = entities(vec![(Vec3::ZERO, target)]);
        let shape = circle(0.0, 0.0, 0.2);
        assert!(e.apply_hit(&shape, 1.0, Some(7)).landed);
        assert!(!e.apply_hit(&shape, 1.0, Some(7)).landed);
        assert!(e.apply_hit(&shape, 1.0, Some(8)).landed);
        assert_eq!(e.live[0].health, 8.0);
    }

    #[test]
    fn solid_things_block_a_circle_around_them() {
        let (e, _) = entities(vec![
            (Vec3::new(1.0, 0.0, 1.0), Behavior::Solid { radius: 0.6 }),
            (
                Vec3::new(5.0, 0.0, 0.0),
                Behavior::Target {
                    health: 1.0,
                    radius: 0.5,
                    solid: false,
                    drop: None,
                },
            ),
        ]);
        assert!(e.blocked(Vec2::new(1.2, 1.0)));
        assert!(!e.blocked(Vec2::new(2.0, 1.0)));
        assert!(
            !e.blocked(Vec2::new(5.0, 0.0)),
            "a target that isn't solid doesn't block"
        );
    }

    #[test]
    fn pickups_triggers_and_checkpoints_react_to_the_player() {
        let (mut e, ids) = entities(vec![
            (
                Vec3::new(0.0, 0.0, 0.0),
                Behavior::Pickup {
                    item: "arrows".into(),
                    count: 5,
                    radius: 0.5,
                },
            ),
            (
                Vec3::new(10.0, 0.0, 0.0),
                Behavior::Trigger {
                    radius: 1.0,
                    effect: Effect::Message("hi".into()),
                    once: true,
                },
            ),
            (
                Vec3::new(20.0, 2.0, 0.0),
                Behavior::Checkpoint { radius: 1.0 },
            ),
        ]);
        // Far from everything.
        assert!(e.touch(Vec3::new(-5.0, 0.0, 0.0), 0.4).is_empty());
        // On the pickup: offered every update until consumed.
        let t = e.touch(Vec3::new(0.2, 0.0, 0.0), 0.4);
        assert_eq!(
            t,
            vec![Touch::Pickup {
                node: ids[0],
                item: "arrows".into(),
                count: 5
            }]
        );
        assert_eq!(e.touch(Vec3::new(0.2, 0.0, 0.0), 0.4).len(), 1);
        e.consume(ids[0]);
        assert!(e.touch(Vec3::new(0.2, 0.0, 0.0), 0.4).is_empty());
        assert_eq!(e.gone().collect::<Vec<_>>(), vec![ids[0]]);
        // A one-time trigger fires on entering, not while staying, and never again.
        assert_eq!(e.touch(Vec3::new(10.0, 0.0, 0.0), 0.4).len(), 1);
        assert!(e.touch(Vec3::new(10.1, 0.0, 0.0), 0.4).is_empty());
        assert!(e.touch(Vec3::new(0.0, 0.0, 5.0), 0.4).is_empty());
        assert!(e.touch(Vec3::new(10.0, 0.0, 0.0), 0.4).is_empty());
        // A checkpoint reports where it is, once per visit.
        assert_eq!(
            e.touch(Vec3::new(20.0, 0.0, 0.0), 0.4),
            vec![Touch::Checkpoint {
                at: Vec3::new(20.0, 2.0, 0.0)
            }]
        );
        assert!(e.touch(Vec3::new(20.0, 0.0, 0.0), 0.4).is_empty());
    }

    #[test]
    fn behavior_kinds_have_defaults() {
        for kind in Behavior::KINDS {
            assert_eq!(Behavior::new(kind).unwrap().kind(), kind);
        }
        assert!(Behavior::new("Nope").is_none());
    }

    #[test]
    fn a_hazard_hurts_while_touched_and_a_door_on_entering() {
        let (mut e, _) = entities(vec![
            (
                Vec3::ZERO,
                Behavior::Hazard {
                    radius: 1.0,
                    damage: 2.0,
                },
            ),
            (
                Vec3::new(10.0, 0.0, 0.0),
                Behavior::Door {
                    radius: 1.0,
                    to: "there".into(),
                    fade: 0.5,
                },
            ),
        ]);
        for _ in 0..3 {
            assert_eq!(
                e.touch(Vec3::ZERO, 0.3),
                vec![Touch::Hazard { damage: 2.0 }]
            );
        }
        let door = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(
            e.touch(door, 0.3),
            vec![Touch::Door {
                to: "there".into(),
                fade: 0.5
            }]
        );
        assert!(e.touch(door, 0.3).is_empty());
    }

    #[test]
    fn a_node_with_two_behaviors_hurts_until_it_is_beaten() {
        let mut scene = Scene::default();
        let id = scene.add(ROOT, "Goomba".into(), None, Vec3::ZERO);
        scene.get_mut(id).unwrap().behaviors = vec![
            NodeBehavior::new(Behavior::Target {
                health: 1.0,
                radius: 0.5,
                solid: false,
                drop: None,
            }),
            NodeBehavior::new(Behavior::Hazard {
                radius: 0.5,
                damage: 1.0,
            }),
        ];
        let mut e = Entities::build(scene.nodes.clone());
        e.refresh(&|_| 0);
        assert_eq!(
            e.touch(Vec3::ZERO, 0.3),
            vec![Touch::Hazard { damage: 1.0 }]
        );
        let hit = e.apply_hit(&circle(0.0, 0.0, 0.3), 1.0, None);
        assert_eq!(hit.destroyed.len(), 1);
        // Beaten, it no longer hurts.
        assert!(e.touch(Vec3::ZERO, 0.3).is_empty());
        assert_eq!(e.gone().collect::<Vec<_>>(), vec![id]);
    }

    #[test]
    fn behaviors_work_while_their_condition_holds() {
        let mut scene = Scene::default();
        let id = scene.add(ROOT, "Star door".into(), None, Vec3::ZERO);
        let locked = Condition::Not(Box::new(Condition::Has {
            item: "star".into(),
            count: 1,
        }));
        scene.get_mut(id).unwrap().behaviors = vec![
            NodeBehavior {
                when: Some(locked.clone()),
                ..NodeBehavior::new(Behavior::Solid { radius: 1.0 })
            },
            NodeBehavior {
                when: Some(locked),
                ..NodeBehavior::new(Behavior::Trigger {
                    radius: 1.5,
                    effect: Effect::Message("You need a star".into()),
                    once: false,
                })
            },
        ];
        let mut e = Entities::build(scene.nodes.clone());
        e.refresh(&|_| 0);
        assert!(e.blocked(Vec2::ZERO));
        assert_eq!(e.touch(Vec3::ZERO, 0.3).len(), 1);
        // With a star the door is open and says nothing.
        e.refresh(&|item| u32::from(item == "star"));
        assert!(!e.blocked(Vec2::ZERO));
        assert!(e.touch(Vec3::new(0.1, 0.0, 0.0), 0.3).is_empty());
    }

    #[test]
    fn nodes_appear_once_their_condition_holds_and_stay() {
        let mut scene = Scene::default();
        let boss = scene.add(ROOT, "King".into(), None, Vec3::ZERO);
        scene.get_mut(boss).unwrap().behaviors = vec![NodeBehavior::new(Behavior::Target {
            health: 1.0,
            radius: 0.5,
            solid: false,
            drop: None,
        })];
        let star = scene.add(ROOT, "Star".into(), None, Vec3::new(5.0, 0.0, 0.0));
        let node = scene.get_mut(star).unwrap();
        node.appears = Some(Condition::Gone("King".into()));
        node.behaviors = vec![NodeBehavior::new(Behavior::Pickup {
            item: "star".into(),
            count: 1,
            radius: 0.5,
        })];
        let flagged = scene.add(ROOT, "Cap block".into(), None, Vec3::new(9.0, 0.0, 0.0));
        scene.get_mut(flagged).unwrap().appears = Some(Condition::Flag("wing".into()));
        let mut e = Entities::build(scene.nodes.clone());
        e.refresh(&|_| 0);
        assert_eq!(e.hidden().collect::<Vec<_>>(), vec![star, flagged]);
        assert!(e.touch(Vec3::new(5.0, 0.0, 0.0), 0.3).is_empty());
        e.apply_hit(&circle(0.0, 0.0, 0.3), 1.0, None);
        e.set_flag("wing", true);
        e.refresh(&|_| 0);
        assert_eq!(e.hidden().collect::<Vec<_>>(), vec![boss]);
        assert_eq!(e.touch(Vec3::new(5.0, 0.0, 0.0), 0.3).len(), 1);
        // Lowering the flag again doesn't take back what appeared.
        e.set_flag("wing", false);
        e.refresh(&|_| 0);
        assert!(!e.hidden().any(|n| n == flagged));
    }

    #[test]
    fn behaviors_reach_only_so_far_up_and_down() {
        let (mut e, _) = entities(vec![
            (
                Vec3::new(0.0, 10.0, 0.0),
                Behavior::Door {
                    radius: 1.0,
                    to: "upstairs".into(),
                    fade: 0.5,
                },
            ),
            (
                Vec3::new(0.0, -10.0, 0.0),
                Behavior::Door {
                    radius: 1.0,
                    to: "basement".into(),
                    fade: 0.5,
                },
            ),
        ]);
        assert_eq!(
            e.touch(Vec3::new(0.0, -10.0, 0.0), 0.3),
            vec![Touch::Door {
                to: "basement".into(),
                fade: 0.5
            }]
        );
        assert!(e.touch(Vec3::ZERO, 0.3).is_empty());
    }

    #[test]
    fn conditions_combine() {
        let e = Entities::default();
        let has = |n| Condition::Has {
            item: "coin".into(),
            count: n,
        };
        let carried = |item: &str| if item == "coin" { 5 } else { 0 };
        assert!(e.holds(&has(5), &carried));
        assert!(!e.holds(&has(6), &carried));
        assert!(e.holds(&Condition::All(vec![]), &carried));
        assert!(e.holds(&Condition::Any(vec![has(9), has(1)]), &carried));
        assert!(!e.holds(&Condition::All(vec![has(9), has(1)]), &carried));
        assert!(e.holds(
            &Condition::Not(Box::new(Condition::Flag("x".into()))),
            &carried
        ));
    }

    #[test]
    fn an_out_of_act_node_is_hidden_and_every_behavior_is_inert() {
        let (mut scene, ids) = scene_with(vec![
            (Vec3::ZERO, Behavior::new("Target").unwrap()),
            (Vec3::ZERO, Behavior::new("Hazard").unwrap()),
        ]);
        let node = scene.get_mut(ids[0]).unwrap();
        node.acts = Some(vec![2, 7]);
        node.behaviors.push(NodeBehavior::new(Behavior::Pickup {
            item: "star".into(),
            count: 1,
            radius: 1.0,
        }));
        node.behaviors.push(NodeBehavior::new(Behavior::Hazard {
            radius: 1.0,
            damage: 1.0,
        }));
        let decoration = scene.add(ROOT, "Decoration".into(), None, Vec3::ZERO);
        scene.get_mut(decoration).unwrap().acts = Some(vec![]);
        let mut e = Entities::build(scene.nodes.clone());
        e.set_act(Some(1));
        e.refresh(&|_| 0);
        assert_eq!(e.hidden().collect::<Vec<_>>(), vec![ids[0], decoration]);
        assert!(!e.blocked(Vec2::ZERO));
        assert!(!e.apply_hit(&circle(0.0, 0.0, 1.0), 1.0, None).landed);
        assert_eq!(
            e.touch(Vec3::ZERO, 0.3),
            vec![Touch::Hazard { damage: 1.0 }]
        );
        for act in [Some(2), Some(7), None] {
            e.set_act(act);
            e.refresh(&|_| 0);
            assert!(!e.hidden().any(|id| id == ids[0]));
            assert!(e.blocked(Vec2::ZERO));
            assert_eq!(e.touch(Vec3::ZERO, 0.3).len(), 3);
        }
        assert_eq!(
            e.hidden().count(),
            0,
            "no act also opens an empty acts list"
        );
    }

    #[test]
    fn switching_acts_preserves_gone_appeared_and_damage() {
        let (mut scene, ids) = scene_with(vec![
            (Vec3::ZERO, Behavior::new("Target").unwrap()),
            (Vec3::new(5.0, 0.0, 0.0), Behavior::new("Target").unwrap()),
        ]);
        for id in &ids {
            scene.get_mut(*id).unwrap().acts = Some(vec![1]);
        }
        let appeared = scene.add(ROOT, "Reward".into(), None, Vec3::ZERO);
        let node = scene.get_mut(appeared).unwrap();
        node.acts = Some(vec![1]);
        node.appears = Some(Condition::Flag("won".into()));
        let mut e = Entities::build(scene.nodes.clone());
        e.set_act(Some(1));
        e.refresh(&|_| 0);
        e.apply_hit(&circle(0.0, 0.0, 0.5), 3.0, None);
        e.apply_hit(&circle(5.0, 0.0, 0.5), 1.0, None);
        e.set_act(Some(2));
        e.set_flag("won", true);
        e.refresh(&|_| 0);
        assert_eq!(e.hidden().count(), 3);
        assert!(e.nodes.iter().find(|n| n.id == appeared).unwrap().shown);
        e.set_flag("won", false);
        for act in [Some(1), None, Some(1)] {
            e.set_act(act);
            e.refresh(&|_| 0);
            assert_eq!(e.hidden().collect::<Vec<_>>(), vec![ids[0]]);
            assert_eq!(e.gone().collect::<Vec<_>>(), vec![ids[0]]);
            assert_eq!(e.live[1].health, 2.0);
            assert!(e.live[1].working());
        }
    }

    #[test]
    fn act_conditions_control_behaviors_and_latch_appearances() {
        let (mut scene, ids) = scene_with(vec![(Vec3::ZERO, Behavior::Solid { radius: 1.0 })]);
        scene.get_mut(ids[0]).unwrap().behaviors[0].when = Some(Condition::Act(2));
        let reward = scene.add(ROOT, "Reward".into(), None, Vec3::ZERO);
        scene.get_mut(reward).unwrap().appears = Some(Condition::Act(2));
        let mut e = Entities::build(scene.nodes.clone());
        e.refresh(&|_| 0);
        assert!(!e.blocked(Vec2::ZERO));
        assert_eq!(e.hidden().collect::<Vec<_>>(), vec![reward]);
        e.set_act(Some(2));
        e.refresh(&|_| 0);
        assert!(e.blocked(Vec2::ZERO));
        assert_eq!(e.hidden().count(), 0);
        for act in [Some(1), None] {
            e.set_act(act);
            assert!(
                !e.blocked(Vec2::ZERO),
                "no stale act behavior before refresh"
            );
            e.refresh(&|_| 0);
            assert!(!e.blocked(Vec2::ZERO));
            assert_eq!(e.hidden().count(), 0);
        }
    }

    #[test]
    fn switching_acts_reenters_triggers_but_does_not_rearm_once() {
        let (mut scene, ids) = scene_with(
            [false, true]
                .into_iter()
                .map(|once| {
                    (
                        Vec3::ZERO,
                        Behavior::Trigger {
                            radius: 1.0,
                            effect: Effect::Message("hello".into()),
                            once,
                        },
                    )
                })
                .collect(),
        );
        for id in &ids {
            scene.get_mut(*id).unwrap().acts = Some(vec![1, 3]);
        }
        let mut e = Entities::build(scene.nodes.clone());
        e.set_act(Some(1));
        e.refresh(&|_| 0);
        assert_eq!(e.touch(Vec3::ZERO, 0.3).len(), 2);
        e.set_act(Some(3));
        e.refresh(&|_| 0);
        assert!(e.touch(Vec3::ZERO, 0.3).is_empty());
        e.set_act(Some(2));
        e.refresh(&|_| 0);
        e.set_act(Some(1));
        e.refresh(&|_| 0);
        let touches = e.touch(Vec3::ZERO, 0.3);
        assert!(matches!(&touches[..], [Touch::Trigger { node, .. }] if *node == ids[0]));
    }
}

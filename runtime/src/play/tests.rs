use super::*;
use clockwork_core::{
    behavior::{Condition, NodeBehavior},
    character2d::Character2d,
    item::ItemDef,
    surface::{SurfaceKind, SurfaceMap, SurfaceProperty},
};
use entities::Placed;

/// A sprite character that walks 4 m/s from the start, with a key it can carry.
fn walker() -> Character2d {
    ron::from_str(
        r#"(
            movement: (speed: 4.0, radius: 0.3),
            animations: (
                idle: (north: (name: "idle"), east: (name: "idle"), south: (name: "idle"), west: (name: "idle")),
                walk: (north: (name: "walk"), east: (name: "walk"), south: (name: "walk"), west: (name: "walk")),
            ),
        )"#,
    )
    .unwrap()
}

fn key() -> ItemDef {
    ron::from_str(r#"(id: "key", name: "Key", description: "", category: Quest)"#).unwrap()
}

/// A 20 m square of open floor, one cell per meter, centered on the origin.
fn floor() -> SurfaceMap {
    SurfaceMap {
        width: 20,
        height: 20,
        cell_size: 16,
        kinds: vec![SurfaceKind {
            properties: vec![SurfaceProperty::Walkable],
            codes: vec![],
        }],
        cells: vec![0; 400],
        extensions: vec![],
    }
}

fn placed(id: NodeId, name: &str, at: Vec3, behaviors: Vec<Behavior>) -> Placed {
    Placed {
        id,
        name: name.into(),
        at,
        behaviors: behaviors.into_iter().map(NodeBehavior::new).collect(),
        ..Default::default()
    }
}

/// A top-down play of `walker` from the origin among `nodes`.
fn play(nodes: Vec<Placed>, rules: Rules) -> Play {
    let character = walker();
    let avatar = Avatar::TopDown(Box::new(SpriteBody {
        body: Body2d::new(Vec3::ZERO, Direction4::South),
        start: Vec3::ZERO,
        character: character.clone(),
    }));
    let fighter = Fighter::new(Some(Inventory::new(vec![key()])), character.moves());
    let mut play = Play::new(
        1,
        avatar,
        fighter,
        Stats {
            health: 3.0,
            hurt_time: 1.0,
        },
        rules,
        Entities::build(nodes),
    );
    play.set_act(None);
    play
}

/// Holds the stick towards `direction` (x east, y south on the ground) for `seconds`.
fn walk(
    play: &mut Play,
    map: &SurfaceMap,
    places: &impl Places,
    direction: Vec2,
    seconds: f32,
) -> Vec<Happened> {
    let level = Level {
        areas: vec![Area {
            to_local: bevy::math::Affine3A::IDENTITY,
            size: Vec2::splat(20.0),
            height: 0.0,
            map,
        }],
        terrains: vec![],
    };
    let frame = Frame {
        controls: Controls {
            stick: direction,
            direction,
            ..Default::default()
        },
        sim_dt: STEP,
        dt: STEP,
        ..Default::default()
    };
    let mut happened = Vec::new();
    for _ in 0..(seconds / STEP).round() as usize {
        happened.extend(play.advance(&frame, &level, places, &Words::default()));
    }
    happened
}

fn nowhere(_: &str) -> Option<Vec3> {
    None
}

#[test]
fn a_played_sprite_walks_picks_up_a_key_and_reaches_the_goal() {
    let map = floor();
    let nodes = vec![
        placed(
            2,
            "Key",
            Vec3::new(3.0, 0.0, 0.0),
            vec![Behavior::Pickup {
                item: "key".into(),
                count: 1,
                radius: 0.5,
            }],
        ),
        placed(3, "Exit", Vec3::new(6.0, 0.0, 0.0), vec![]),
    ];
    let rules = Rules {
        goal: Some(Goal::When(Condition::Has {
            item: "key".into(),
            count: 1,
        })),
        ..Default::default()
    };
    let mut play = play(nodes, rules);
    let happened = walk(&mut play, &map, &nowhere, Vec2::X, 1.0);
    assert!(
        play.position().x > 3.0,
        "walked east: {:?}",
        play.position()
    );
    assert_eq!(play.carried("key"), 1);
    assert!(happened.contains(&Happened::Picked));
    assert!(matches!(happened.last(), Some(Happened::Won(_))));
    assert!(play.won);
    assert_eq!(play.entities.gone().collect::<Vec<_>>(), vec![2]);
}

#[test]
fn a_door_fades_out_moves_the_player_and_fades_in() {
    let map = floor();
    let nodes = vec![placed(
        2,
        "Door",
        Vec3::new(1.0, 0.0, 0.0),
        vec![Behavior::Door {
            radius: 0.5,
            to: "Inside".into(),
            fade: 0.5,
        }],
    )];
    let inside = Vec3::new(-8.0, 0.0, -8.0);
    let places = |name: &str| (name == "Inside").then_some(inside);
    let mut play = play(nodes, Rules::default());
    // Into the door, then standing still while the screen fades.
    let mut happened = walk(&mut play, &map, &places, Vec2::X, 0.1);
    happened.extend(walk(&mut play, &map, &places, Vec2::ZERO, 0.1));
    assert!(happened.contains(&Happened::Door));
    assert!(
        happened
            .iter()
            .any(|h| matches!(h, Happened::Arrived { name, .. } if name == "Inside"))
    );
    assert!(play.darkness() > 0.0);
    walk(&mut play, &map, &places, Vec2::ZERO, 0.5);
    assert_eq!(play.darkness(), 0.0);
    assert!(play.position().distance(inside) < 0.01);
}

#[test]
fn a_hazard_takes_lives_and_the_last_one_starts_the_game_over() {
    let map = floor();
    let nodes = vec![placed(
        2,
        "Spikes",
        Vec3::ZERO,
        vec![Behavior::Hazard {
            radius: 1.0,
            damage: 3.0,
        }],
    )];
    let rules = Rules {
        lives: Some(2),
        ..Default::default()
    };
    let mut play = play(nodes, rules);
    let happened = walk(&mut play, &map, &nowhere, Vec2::ZERO, STEP);
    assert!(happened.contains(&Happened::Hurt));
    assert_eq!(play.lives, Some(1));
    // Still standing in it once it can be hurt again: the last life goes.
    let happened = walk(&mut play, &map, &nowhere, Vec2::ZERO, 1.1);
    assert!(
        happened
            .iter()
            .any(|h| matches!(h, Happened::Toast(text) if *text == Words::default().game_over))
    );
    assert_eq!(play.lives, Some(2), "a new game has every life back");
}

#[test]
fn choosing_an_act_refreshes_play_without_rebuilding_entities() {
    let mut wall = placed(2, "Wall", Vec3::ZERO, vec![Behavior::Solid { radius: 1.0 }]);
    wall.acts = Some(vec![2]);
    let mut play = play(vec![wall], Rules::default());
    play.set_act(Some(1));
    assert_eq!(play.entities.hidden().collect::<Vec<_>>(), vec![2]);
    assert!(!play.entities.blocked(Vec2::ZERO));
    play.set_act(Some(2));
    assert_eq!(play.entities.hidden().count(), 0);
    assert!(play.entities.blocked(Vec2::ZERO));
    play.entities.consume(2);
    play.set_act(None);
    assert_eq!(play.entities.hidden().collect::<Vec<_>>(), vec![2]);
    assert!(!play.entities.blocked(Vec2::ZERO));
}

#[test]
fn a_door_fade_is_darkest_halfway() {
    let fade = |elapsed| Fade {
        elapsed,
        total: 1.0,
        to: Vec3::ZERO,
        moved: false,
    };
    assert_eq!(fade(0.0).darkness(), 0.0);
    assert_eq!(fade(0.5).darkness(), 1.0);
    assert_eq!(fade(1.0).darkness(), 0.0);
    assert!((fade(0.25).darkness() - 0.5).abs() < 1e-6);
}

#[test]
fn vitals_rest_after_a_hit_and_run_out() {
    let mut v = Vitals::new(Stats {
        health: 2.0,
        hurt_time: 1.0,
    });
    assert!(!v.hurt(1.0));
    assert!(!v.hurt(1.0), "just hurt: no second hit");
    v.safe = 0.0;
    assert!(v.hurt(1.0));
    v.heal(5.0);
    assert_eq!(v.health, 2.0);
}

/// An enemy that waits until the player is within 3 m, then chases and hurts on contact.
fn chaser() -> Behavior {
    Behavior::Actor(Box::new(
        ron::from_str(
            r#"(
                health: Some(2.0), radius: 0.4, contact: 1.0, speed: 2.0, knockback: 1.0,
                states: [
                    (name: "wait", next: [(to: "chase", when: Near(3.0))]),
                    (name: "chase", motion: Chase, animation: Some(One("run")),
                     enter: Some(Set("spotted")),
                     next: [(to: "stunned", when: Hurt)]),
                    (name: "stunned", contact: Some(0.0), next: [(to: "chase", when: After(0.5))]),
                ],
            )"#,
        )
        .unwrap(),
    ))
}

#[test]
fn an_actor_chases_the_player_and_hurts_on_contact() {
    let map = floor();
    let nodes = vec![placed(
        2,
        "Moblin",
        Vec3::new(5.0, 0.0, 0.0),
        vec![chaser()],
    )];
    let mut play = play(nodes, Rules::default());
    walk(&mut play, &map, &nowhere, Vec2::ZERO, 0.5);
    assert_eq!(
        play.entities.live[0].at,
        Vec3::new(5.0, 0.0, 0.0),
        "too far to notice"
    );
    // Coming within 3 m: it starts after the player, raising its flag.
    walk(&mut play, &map, &nowhere, Vec2::X, 0.6);
    assert!(play.entities.flags.contains("spotted"));
    let happened = walk(&mut play, &map, &nowhere, Vec2::ZERO, 1.0);
    assert!(happened.contains(&Happened::Hurt));
    let pose = play.entities.poses().next().unwrap();
    assert_eq!(
        (pose.state, pose.animation),
        ("chase", Some(("run", false)))
    );
    assert!(pose.at.x < 5.0, "it came to the player: {}", pose.at);
}

/// A turret that fires a rock at the player every second and never moves.
fn turret() -> Behavior {
    Behavior::Actor(Box::new(
        ron::from_str(
            r#"(
                radius: 0.4,
                states: [
                    (name: "aim", shoot: Some((at: 0.1, ahead: 0.5,
                        projectile: (id: "rock", motion: Straight(speed: 8.0), lifetime: 2.0,
                                     radius: 0.2, damage: 2.0))),
                     next: [(to: "reload", when: After(0.2))]),
                    (name: "reload", next: [(to: "aim", when: After(0.8))]),
                ],
            )"#,
        )
        .unwrap(),
    ))
}

#[test]
fn an_actor_shoots_the_player_from_afar() {
    let map = floor();
    let nodes = vec![placed(
        2,
        "Turret",
        Vec3::new(6.0, 0.0, 0.0),
        vec![turret()],
    )];
    let mut play = play(nodes, Rules::default());
    // The rock leaves at 0.1 s and flies the 6 m at 8 m/s: the player is hurt once by it.
    let happened = walk(&mut play, &map, &nowhere, Vec2::ZERO, 0.9);
    assert_eq!(
        happened.iter().filter(|h| **h == Happened::Hurt).count(),
        1,
        "{happened:?}"
    );
    assert_eq!(play.vitals.health, play.vitals.max - 2.0);
    assert_eq!(
        play.entities.shots.count("rock"),
        0,
        "it stopped at the player"
    );
    assert_eq!(play.entities.live[0].at, Vec3::new(6.0, 0.0, 0.0));
    let (rock, plugin) = play.entities.shot("rock").unwrap();
    assert_eq!((rock.damage, plugin), (2.0, None));
    // It fires again each time it aims.
    walk(&mut play, &map, &nowhere, Vec2::ZERO, 0.5);
    assert_eq!(play.entities.shots.count("rock"), 1);
}

#[test]
fn beating_every_actor_wins_a_destroy_all_game() {
    let map = floor();
    let nodes = vec![placed(
        2,
        "Moblin",
        Vec3::new(1.0, 0.0, 0.0),
        vec![chaser()],
    )];
    let rules = Rules {
        goal: Some(Goal::DestroyAll),
        ..Default::default()
    };
    let mut play = play(nodes, rules);
    // Near enough: it is chasing when the bomb goes off just west of it.
    walk(&mut play, &map, &nowhere, Vec2::ZERO, STEP);
    let x = play.entities.live[0].at.x;
    let hit = crate::actions::Shape::Circle {
        center: Vec2::new(x - 0.3, 0.0),
        radius: 0.5,
    };
    let first = play.entities.apply_hit(&hit, 1.0, None);
    assert!(first.landed && first.destroyed.is_empty());
    // Stunned, then pushed back east by the blast.
    walk(&mut play, &map, &nowhere, Vec2::ZERO, 0.1);
    assert_eq!(play.entities.poses().next().unwrap().state, "stunned");
    assert!(play.entities.live[0].at.x > x);
    let at = play.entities.live[0].at;
    let hit = crate::actions::Shape::Circle {
        center: Vec2::new(at.x, at.z),
        radius: 0.5,
    };
    assert_eq!(play.entities.apply_hit(&hit, 1.0, None).destroyed.len(), 1);
    let happened = walk(&mut play, &map, &nowhere, Vec2::ZERO, STEP);
    assert!(matches!(happened.last(), Some(Happened::Won(_))));
}

#[test]
fn a_trigger_runs_a_cutscene_that_holds_the_player_and_walks_a_guard() {
    let map = floor();
    let cutscene: Effect = ron::from_str(
        r#"Run((hold_player: true, steps: [
            Walk(node: "Guard", to: "Gate", speed: 2.0),
            Do(Message("Halt!")),
            Wait(0.5),
            Do(Set("talked")),
        ]))"#,
    )
    .unwrap();
    let nodes = vec![
        placed(
            2,
            "Plate",
            Vec3::new(0.5, 0.0, 0.0),
            vec![Behavior::Trigger {
                radius: 0.5,
                effect: cutscene,
                once: true,
            }],
        ),
        placed(
            3,
            "Guard",
            Vec3::new(4.0, 0.0, 0.0),
            vec![Behavior::Solid { radius: 0.4 }],
        ),
        placed(4, "Gate", Vec3::new(4.0, 0.0, 2.0), vec![]),
    ];
    let places = |name: &str| match name {
        "Gate" => Some(Vec3::new(4.0, 0.0, 2.0)),
        "Guard" => Some(Vec3::new(4.0, 0.0, 0.0)),
        _ => None,
    };
    let mut play = play(nodes, Rules::default());
    // Stepping on the plate starts the cutscene; the player stops there however it pushes.
    walk(&mut play, &map, &places, Vec2::X, 0.1);
    assert!(play.held());
    let stopped = play.position();
    let happened = walk(&mut play, &map, &places, Vec2::X, 0.5);
    assert_eq!(play.position(), stopped);
    // Two meters at two meters a second: the guard is at the gate after a second.
    assert!(!happened.contains(&Happened::Notice("Halt!".into())));
    let happened = walk(&mut play, &map, &places, Vec2::X, 0.6);
    assert!(happened.contains(&Happened::Notice("Halt!".into())));
    assert_eq!(play.moved.get(&3), Some(&Vec3::new(4.0, 0.0, 2.0)));
    assert!(
        play.entities
            .live
            .iter()
            .any(|e| e.node == 3 && e.at.z == 2.0)
    );
    walk(&mut play, &map, &places, Vec2::X, 0.6);
    assert!(play.entities.flags.contains("talked"));
    assert!(!play.held(), "the cutscene ended");
    walk(&mut play, &map, &places, Vec2::X, 0.3);
    assert!(play.position().x > stopped.x, "the player walks again");
}

/// The open floor as a level.
fn level(map: &SurfaceMap) -> Level<'_> {
    Level {
        areas: vec![Area {
            to_local: bevy::math::Affine3A::IDENTITY,
            size: Vec2::splat(20.0),
            height: 0.0,
            map,
        }],
        terrains: vec![],
    }
}

fn toward(name: &str, seconds: f32) -> playtest::Step {
    playtest::Step {
        hold: playtest::Hold::Toward(name.into()),
        seconds,
        walk: false,
        buttons: vec![],
    }
}

#[test]
fn a_script_walks_to_the_key_then_the_exit_and_wins() {
    let map = floor();
    let nodes = vec![
        placed(
            2,
            "Key",
            Vec3::new(3.0, 0.0, 2.0),
            vec![Behavior::Pickup {
                item: "key".into(),
                count: 1,
                radius: 0.5,
            }],
        ),
        placed(3, "Exit", Vec3::new(-4.0, 0.0, 0.0), vec![]),
    ];
    let exit = Vec3::new(-4.0, 0.0, 0.0);
    let places = |name: &str| (name == "Exit").then_some(exit);
    let rules = Rules {
        goal: Some(Goal::Reach("Exit".into())),
        ..Default::default()
    };
    let steps = [toward("Key", 5.0), toward("Exit", 5.0), toward("Key", 1.0)];
    let report = playtest::run(
        &mut play(nodes, rules),
        &level(&map),
        &places,
        &Words::default(),
        &steps,
    );
    let said: Vec<&str> = report.events.iter().map(|e| e.what.as_str()).collect();
    assert!(said.contains(&"Got Key x1"), "{said:?}");
    assert!(said.contains(&"\"Key\" is gone"), "{said:?}");
    assert!(report.steps[0].reached.is_some());
    assert!(report.steps[1].reached.is_some() || report.won.is_some());
    assert!(report.won.is_some());
    assert!(report.steps[2].skipped, "nothing is played once won");
    assert_eq!(report.carried, vec![("key".to_string(), 1)]);
    assert!(report.never_reached.is_empty());
    // The same script plays the same way.
    let again = playtest::run(
        &mut play(
            vec![
                placed(
                    2,
                    "Key",
                    Vec3::new(3.0, 0.0, 2.0),
                    vec![Behavior::Pickup {
                        item: "key".into(),
                        count: 1,
                        radius: 0.5,
                    }],
                ),
                placed(3, "Exit", exit, vec![]),
            ],
            Rules {
                goal: Some(Goal::Reach("Exit".into())),
                ..Default::default()
            },
        ),
        &level(&map),
        &places,
        &Words::default(),
        &steps,
    );
    assert_eq!(report, again);
}

#[test]
fn a_script_reports_walls_hurts_and_what_it_never_reached() {
    let map = floor();
    let nodes = vec![
        placed(
            2,
            "Wall",
            Vec3::new(2.0, 0.0, 0.0),
            vec![Behavior::Solid { radius: 1.0 }],
        ),
        placed(3, "Moblin", Vec3::new(0.0, 0.0, 2.5), vec![chaser()]),
        placed(
            4,
            "Chest",
            Vec3::new(-6.0, 0.0, -6.0),
            vec![Behavior::Pickup {
                item: "key".into(),
                count: 1,
                radius: 0.5,
            }],
        ),
    ];
    let steps = [
        playtest::Step {
            hold: playtest::Hold::Direction(Vec2::X),
            seconds: 1.5,
            walk: false,
            buttons: vec![],
        },
        playtest::Step {
            hold: playtest::Hold::Still,
            seconds: 2.0,
            walk: false,
            buttons: vec![playtest::Button::A],
        },
        toward("Nowhere", 0.5),
    ];
    let report = playtest::run(
        &mut play(nodes, Rules::default()),
        &level(&map),
        &nowhere,
        &Words::default(),
        &steps,
    );
    let text = report.to_string();
    assert!(report.steps[0].stuck.is_some(), "{text}");
    assert!(
        report
            .events
            .iter()
            .any(|e| e.what.starts_with("hurt by \"Moblin\"")),
        "{text}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|e| e.what == "\"Moblin\" is now chase"),
        "{text}"
    );
    assert!(text.contains("no node called \"Nowhere\""), "{text}");
    assert_eq!(report.never_reached.len(), 1, "{text}");
    assert_eq!(report.never_reached[0].name, "Chest");
    assert_eq!(report.standing[0].name, "Moblin");
    assert!(report.health.0 < report.health.1);
    assert!(report.won.is_none());
}

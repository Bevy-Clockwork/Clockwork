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

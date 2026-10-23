//! Walks a character on a 3D area extracted from a local ROM. Opt-in: no source data is stored in
//! the repository.
//!
//! ```text
//! CLOCKWORK_TEST_ROM=<rom> CLOCKWORK_TEST_RECIPE=<recipe.ron> cargo test -p clockwork_runtime \
//!     --test terrain_rom -- --ignored --nocapture
//! ```
//!
//! The recipe must output a model with a terrain and a start point (any 3D area plugin).

use bevy::math::{Affine3A, Vec2, Vec3};
use clockwork_core::character::Movement;
use clockwork_core::recipe::Recipe;
use clockwork_runtime::controller::{self, Body, Ground, Input, World};
use clockwork_runtime::terrain::WorldTerrain;

struct Level(WorldTerrain);

impl World for Level {
    fn ground(&self, _x: f32, _z: f32) -> Ground {
        Ground::Hole
    }

    fn ground_at(&self, x: f32, y: f32, z: f32) -> Ground {
        self.0.floor(x, y, z).unwrap_or(Ground::Hole)
    }

    fn push_out(&self, position: Vec3, radius: f32) -> Vec3 {
        self.0.push_out(position, radius)
    }
}

fn movement() -> Movement {
    // Plain made-up numbers: the test is about the terrain, not how the character feels.
    ron::from_str(
        "(walk_speed: 4.0, run_speed: 8.0, acceleration: 10.0, acceleration_falloff: 0.5,
          braking: 10.0, turn_speed: 360.0, skid: None, jumps: [], chain_window: 0.3,
          landing_time: 0.1, jump_release: None, gravity: 30.0, max_fall_speed: 20.0,
          air: (turn: false, drag: 3.0, acceleration: 12.0, strafe_speed: 3.0, max_speed: 8.0),
          radius: 0.4)",
    )
    .expect("movement")
}

#[test]
#[ignore]
fn character_lands_walks_and_is_stopped_by_walls() {
    let (Ok(rom), Ok(recipe)) = (
        std::env::var("CLOCKWORK_TEST_ROM"),
        std::env::var("CLOCKWORK_TEST_RECIPE"),
    ) else {
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let order = clockwork_n64::ByteOrder::detect(&rom).expect("an N64 ROM");
    let rom = clockwork_n64::to_big_endian(&rom, order);
    let recipe: Recipe = ron::from_str(&std::fs::read_to_string(recipe).unwrap()).unwrap();
    let extracted = clockwork_recipe::run(&recipe, &rom).unwrap();
    let area = extracted
        .models
        .iter()
        .find(|m| m.terrain.is_some())
        .unwrap();
    let terrain = area.terrain.as_ref().unwrap();
    let scale = Affine3A::from_scale(Vec3::splat(area.meters_per_unit));
    let level = Level(WorldTerrain::new(terrain, scale));
    let (start, yaw) = level.0.spawn().expect("a start point");
    println!("start {start:?} yaw {:.0}", yaw.to_degrees());

    let movement = movement();
    let step = 1.0 / 60.0;
    let mut body = Body::new(start + Vec3::Y * 0.5, yaw);
    let rest = Input::default();
    // 1. Dropped a little above the start point, the body lands and stays there.
    for _ in 0..120 {
        controller::step(&mut body, &rest, &movement, &level, step);
    }
    println!("landed at {:?} grounded {}", body.position, body.grounded);
    assert!(body.grounded, "lands on the floor under the start point");
    assert!(
        (body.position.y - start.y).abs() < 1.0,
        "lands on the start platform"
    );

    // 2. Walking in every direction from the start never leaves the level through a wall or
    // under the floor, and a wall stops the body: the farthest it gets in each direction.
    for degrees in (0..360).step_by(45) {
        let mut body = Body::new(start, yaw);
        let a = (degrees as f32).to_radians();
        let input = Input {
            direction: Vec2::new(a.sin(), a.cos()),
            ..Default::default()
        };
        let mut lowest = body.position.y;
        for _ in 0..600 {
            controller::step(&mut body, &input, &movement, &level, step);
            lowest = lowest.min(body.position.y);
        }
        println!(
            "walk {degrees:3}°: ends at ({:7.2}, {:6.2}, {:7.2}) speed {:.2} lowest {:.2}",
            body.position.x, body.position.y, body.position.z, body.speed, lowest
        );
    }
}

//! A played 2D character's actions, items and projectiles: what the buttons do (dodge, the items on
//! A and B), kept for as long as Play mode lasts, with what the HUD and the viewport show of it.

use bevy::math::{Vec2, Vec3};
use clockwork_core::{
    action::{AttachmentFrames, Moves, ProjectileSprite},
    character2d::Direction4,
    item::Slot,
};

use crate::{
    actions::{Event, Modifiers, Runner, Shape, Source},
    controller::World,
    hud::{Hud, HudRequest, ItemView, SlotView},
    inventory::Inventory,
    projectiles::{self, Projectiles},
};

/// How long the hit of an action stays drawn after it happened, in seconds.
const FLASH_TIME: f32 = 0.12;

/// The world direction (ground plane, unit) a facing points in for a camera that looks along
/// `yaw` (0: north is up the map, -z).
pub fn facing_dir(facing: Direction4, yaw: f32) -> Vec2 {
    let right = Vec2::new(yaw.cos(), -yaw.sin());
    let forward = Vec2::new(-yaw.sin(), -yaw.cos());
    match facing {
        Direction4::North => forward,
        Direction4::South => -forward,
        Direction4::East => right,
        Direction4::West => -right,
    }
}

/// Something a character holds now: an attachment of the action it is doing.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    /// A sprite output of the character's plugin.
    pub sprite: String,
    pub frames: AttachmentFrames,
    pub behind: bool,
    pub facing: Direction4,
    /// Seconds since the action started, for attachments with their own animation.
    pub seconds: f32,
    /// The frame shown by a 3D character, which it can't follow frame by frame.
    pub still: Option<usize>,
}

/// A hit or explosion to draw for a moment.
#[derive(Clone, Debug)]
pub struct Flash {
    pub shape: Shape,
    pub height: f32,
    pub age: f32,
    /// Explosions last as long as they burn; hits are short.
    pub lifetime: f32,
    pub color: [f32; 3],
}

/// Something an attack did to the world this step: the Play system applies it to targets.
#[derive(Clone, Debug)]
pub struct HitReport {
    /// Absolute on the ground (x east, z south).
    pub shape: Shape,
    pub damage: f32,
    /// The projectile that did it, which hurts a target once.
    pub projectile: Option<u64>,
}

/// What happened so far, for reports and tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub started: Vec<String>,
    pub hits: u32,
    pub spawned: Vec<String>,
    pub bursts: u32,
}

/// How long an item or dodge press waits for a busy action to allow a new one, in seconds.
const PRESS_BUFFER: f32 = 0.15;

/// The buttons of one frame: (pressed this frame, held) for the item buttons, pressed for dodge.
#[derive(Clone, Copy, Debug, Default)]
pub struct Buttons {
    pub dodge: bool,
    pub b: (bool, bool),
    pub a: (bool, bool),
}

pub struct Fighter {
    pub runner: Runner,
    pub projectiles: Projectiles,
    pub inventory: Option<Inventory>,
    pub flashes: Vec<Flash>,
    /// The pose the current action wants shown: animation, mirrored, seconds.
    pub pose: Option<(String, bool, f32)>,
    pub invulnerable: bool,
    pub guard: Option<(Vec2, f32)>,
    pub hitboxes: bool,
    /// The list of items it carries, until they are given to it (the editor's Play fills them in).
    pub wanted_items: Option<clockwork_core::asset::OutputRef>,
    pub stats: Stats,
    /// Attacks since the last time the Play system took them.
    pub hits: Vec<HitReport>,
    /// How each projectile is drawn without a sprite (RGB), by id.
    colors: Vec<(String, [f32; 3])>,
    /// Projectile sprites, by id.
    looks: Vec<(String, ProjectileSprite)>,
    /// What the character can do: its own moves and the ones it learned.
    pub moves: Moves,
    /// The plugin the character comes from: where its held items' and projectiles' sprites are.
    pub plugin: Option<String>,
    /// Item pictures, by item id.
    pub icons: std::sync::Arc<Vec<(String, clockwork_core::image::RgbaImage)>>,
    /// Seconds each press keeps waiting to be used: the two slots, then dodge. A press waits
    /// for the next simulation step, and for a busy action to allow a new one.
    pressed: [f32; 3],
    held: [bool; 2],
}

impl Fighter {
    pub fn new(inventory: Option<Inventory>, moves: Moves) -> Self {
        Self {
            colors: moves
                .projectiles
                .iter()
                .map(|p| (p.id.clone(), p.color))
                .collect(),
            looks: moves
                .projectiles
                .iter()
                .filter_map(|p| Some((p.id.clone(), p.sprite.clone()?)))
                .collect(),
            plugin: None,
            icons: Default::default(),
            moves,
            runner: Runner::new(),
            projectiles: Projectiles::default(),
            inventory,
            flashes: Vec::new(),
            pose: None,
            invulnerable: false,
            guard: None,
            hitboxes: true,
            wanted_items: None,
            stats: Stats::default(),
            hits: Vec::new(),
            pressed: [0.0; 3],
            held: [false; 2],
        }
    }

    /// The projectile's sprite, if it has one.
    pub fn look_of(&self, projectile: &str) -> Option<&ProjectileSprite> {
        self.looks
            .iter()
            .find(|(id, _)| id == projectile)
            .map(|(_, look)| look)
    }

    /// What the character holds now: the current action's attachments.
    pub fn held(&self) -> Vec<Held> {
        let Some((action, facing, seconds)) = self.runner.active(&self.moves) else {
            return Vec::new();
        };
        action
            .attachments
            .iter()
            .map(|a| Held {
                sprite: a.sprite.clone(),
                frames: a.frames.clone(),
                behind: a.behind,
                facing,
                seconds,
                still: a.still,
            })
            .collect()
    }

    pub fn color_of(&self, projectile: &str) -> Option<[f32; 3]> {
        self.colors
            .iter()
            .find(|(id, _)| id == projectile)
            .map(|(_, c)| *c)
    }

    /// Takes this frame's buttons and HUD requests.
    pub fn feed(&mut self, buttons: &Buttons, requests: &[HudRequest]) {
        if buttons.dodge {
            self.press_roll();
        }
        for (slot, (pressed, down)) in [(Slot::B, buttons.b), (Slot::A, buttons.a)] {
            if pressed {
                self.press(slot);
            }
            self.hold(slot, down);
        }
        for request in requests {
            self.request(request);
        }
    }

    pub fn press_roll(&mut self) {
        self.pressed[2] = PRESS_BUFFER;
    }

    pub fn press(&mut self, slot: Slot) {
        self.pressed[slot.index()] = PRESS_BUFFER;
    }

    /// Whether a slot's button is down now (called every frame).
    pub fn hold(&mut self, slot: Slot, down: bool) {
        self.held[slot.index()] = down;
    }

    /// Answers a HUD request.
    pub fn request(&mut self, request: &HudRequest) {
        match request {
            HudRequest::ToggleHitboxes => self.hitboxes = !self.hitboxes,
            HudRequest::Equip(slot, id) => {
                if let Some(inventory) = &mut self.inventory {
                    inventory.equip(*slot, id);
                }
            }
            HudRequest::Unequip(slot) => {
                if let Some(inventory) = &mut self.inventory {
                    inventory.unequip(*slot);
                }
            }
            // Answered by Play mode.
            HudRequest::Restart | HudRequest::Stop | HudRequest::NextPage => {}
        }
    }

    /// Advances by `dt`: starts what the presses ask for, runs the action, throws and moves
    /// projectiles. Returns how the movement controller is constrained now. `at` is where the
    /// character stands, `yaw` where the camera looks.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        world: &impl World,
        dt: f32,
        at: Vec3,
        facing: Direction4,
        walking: bool,
        yaw: f32,
    ) -> Modifiers {
        let dir = facing_dir(facing, yaw);
        // A press made while an action can't be interrupted waits a moment for it to end.
        if self.pressed[2] > 0.0 {
            let waiting = match self.moves.dodge.clone() {
                Some(dodge) if self.runner.can_start(&self.moves) => {
                    self.runner
                        .start(&self.moves, &dodge, Source::Dodge, facing, dir);
                    false
                }
                Some(_) => true,
                None => false,
            };
            if !waiting {
                self.pressed[2] = 0.0;
            }
        }
        for slot in [Slot::B, Slot::A] {
            if self.pressed[slot.index()] > 0.0 && self.use_slot(slot, facing, dir) {
                self.pressed[slot.index()] = 0.0;
            }
        }
        for left in &mut self.pressed {
            *left = (*left - dt).max(0.0);
        }

        let held = self.held;
        let frame = self.runner.update(
            &self.moves,
            dt,
            |source| match source {
                Source::Slot(slot) => held[slot.index()],
                Source::Dodge => false,
            },
            walking,
        );
        self.pose = frame
            .pose
            .map(|p| (p.animation.name.clone(), p.animation.flip_x, p.time));
        self.invulnerable = frame.invulnerable;
        self.guard = frame.guard;
        let ground = Vec2::new(at.x, at.z);
        for event in frame.events {
            match event {
                Event::Started(id) => self.stats.started.push(id),
                Event::Ended(_) => {}
                Event::Hit(hit) => {
                    self.stats.hits += hit.first as u32;
                    if hit.first {
                        self.hits.push(HitReport {
                            shape: hit.shape.translated(ground),
                            damage: hit.damage,
                            projectile: None,
                        });
                    }
                    self.flashes.push(Flash {
                        shape: hit.shape.translated(ground),
                        height: at.y,
                        age: 0.0,
                        lifetime: FLASH_TIME,
                        color: [1.0, 0.85, 0.3],
                    });
                }
                Event::Spawn {
                    projectile,
                    offset,
                    dir,
                } => {
                    if let Some(spec) = self.moves.projectiles.iter().find(|p| p.id == projectile)
                        && self.projectiles.spawn(spec, ground + offset, at.y, dir)
                    {
                        self.stats.spawned.push(projectile);
                    }
                }
            }
        }

        let events = self
            .projectiles
            .update(&self.moves.projectiles, at, world, dt);
        for event in events {
            if let projectiles::Event::Hit {
                uid,
                position,
                radius,
                damage,
                ..
            } = &event
            {
                self.hits.push(HitReport {
                    shape: Shape::Circle {
                        center: *position,
                        radius: *radius,
                    },
                    damage: *damage,
                    projectile: Some(*uid),
                });
            }
            if let projectiles::Event::Burst {
                position,
                radius,
                damage,
                first,
                ..
            } = event
            {
                if first {
                    self.hits.push(HitReport {
                        shape: Shape::Circle {
                            center: position,
                            radius,
                        },
                        damage,
                        projectile: None,
                    });
                }
                self.stats.bursts += first as u32;
                self.flashes.push(Flash {
                    shape: Shape::Circle {
                        center: position,
                        radius,
                    },
                    height: at.y,
                    age: 0.0,
                    lifetime: dt * 1.5,
                    color: [1.0, 0.45, 0.2],
                });
            }
        }
        for flash in &mut self.flashes {
            flash.age += dt;
        }
        self.flashes.retain(|f| f.age < f.lifetime);
        frame.modifiers
    }

    /// Uses the item on a button if it can be used: owned, enough ammunition, nothing busy
    /// and none of its projectiles at their limit.
    /// Uses the item in `slot` if it can be. Returns false while the press should keep
    /// waiting (an action that can't be interrupted yet), true once it's settled either way.
    fn use_slot(&mut self, slot: Slot, facing: Direction4, dir: Vec2) -> bool {
        if !self.runner.can_start(&self.moves) {
            return false;
        }
        let Some(inventory) = &mut self.inventory else {
            return true;
        };
        let Some(item) = inventory.equipped(slot).cloned() else {
            return true;
        };
        let Some(action) = item
            .action
            .as_deref()
            .and_then(|id| self.moves.actions.iter().find(|a| a.id == id))
        else {
            return true;
        };
        let at_limit = action.spawns.iter().any(|spawn| {
            self.moves
                .projectiles
                .iter()
                .find(|p| p.id == spawn.projectile)
                .and_then(|p| p.limit.map(|limit| (p, limit)))
                .is_some_and(|(p, limit)| self.projectiles.count(&p.id) >= limit as usize)
        });
        if at_limit || !inventory.can_use(&item.id) {
            return true;
        }
        if self
            .runner
            .start(&self.moves, &action.id, Source::Slot(slot), facing, dir)
        {
            inventory.spend(&item.id);
        }
        true
    }

    /// What the HUD draws now.
    pub fn hud(&self) -> Hud {
        let view = |slot: Slot| {
            let inventory = self.inventory.as_ref()?;
            let item = inventory.equipped(slot)?;
            let counted = item.ammo.as_ref().map(|a| a.item.as_str());
            let count = counted
                .map(|id| inventory.count(id))
                .or((item.max > 1).then(|| inventory.count(&item.id)));
            Some(SlotView {
                id: item.id.clone(),
                name: item.name.clone(),
                count: count.map(|n| format!("x {n}")),
                usable: inventory.can_use(&item.id),
            })
        };
        let items = self
            .inventory
            .as_ref()
            .map(|inventory| {
                inventory
                    .items()
                    .iter()
                    .map(|item| ItemView {
                        id: item.id.clone(),
                        name: item.name.clone(),
                        description: item.description.clone(),
                        category: item.category,
                        owned: inventory.owns(&item.id),
                        count: inventory.count(&item.id),
                        max: item.max,
                        on: Slot::ALL
                            .into_iter()
                            .find(|s| inventory.equipped(*s).is_some_and(|e| e.id == item.id)),
                        can_equip: inventory.owns(&item.id) && item.action.is_some(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Hud {
            slots: [view(Slot::A), view(Slot::B)],
            action: self.runner.current(&self.moves).map(str::to_string),
            items,
            hitboxes: self.hitboxes,
            has_dodge: self.moves.dodge.is_some(),
            icons: self.icons.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::Ground;
    use clockwork_core::action::{Action, Motion};
    use clockwork_core::character2d::{AnimationRef, Directional};

    struct Flat;
    impl World for Flat {
        fn ground(&self, _: f32, _: f32) -> Ground {
            Ground::Floor(0.0)
        }
    }

    fn rolling() -> Fighter {
        let one = AnimationRef {
            name: "roll".into(),
            flip_x: false,
            flip_y: false,
        };
        let roll = Action {
            id: "roll".into(),
            animation: Directional {
                north: one.clone(),
                east: one.clone(),
                south: one.clone(),
                west: one,
            },
            duration: 0.3,
            motion: Motion::Rooted,
            hits: vec![],
            invulnerable: None,
            spawns: vec![],
            hold: None,
            cancel_from: None,
            attachments: vec![],
        };
        Fighter::new(
            None,
            Moves {
                actions: vec![roll],
                projectiles: vec![],
                dodge: Some("roll".into()),
            },
        )
    }

    /// Steps `seconds` in 10 ms steps.
    fn run(fighter: &mut Fighter, seconds: f32) {
        for _ in 0..(seconds / 0.01).round() as usize {
            fighter.step(&Flat, 0.01, Vec3::ZERO, Direction4::South, false, 0.0);
        }
    }

    #[test]
    fn a_press_just_before_an_action_ends_starts_the_next() {
        let mut fighter = rolling();
        fighter.press_roll();
        run(&mut fighter, 0.2);
        assert_eq!(fighter.runner.current(&fighter.moves), Some("roll"));
        // Pressed 0.1 s before the roll ends: it waits, then rolls again.
        fighter.press_roll();
        run(&mut fighter, 0.15);
        assert_eq!(fighter.runner.current(&fighter.moves), Some("roll"));
        run(&mut fighter, 0.3);
        assert_eq!(fighter.runner.current(&fighter.moves), None);
    }

    #[test]
    fn a_press_long_before_an_action_ends_is_dropped() {
        let mut fighter = rolling();
        fighter.press_roll();
        run(&mut fighter, 0.01);
        fighter.press_roll();
        run(&mut fighter, 0.35);
        assert_eq!(fighter.runner.current(&fighter.moves), None);
    }
}

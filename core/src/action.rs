//! Actions: things a 2D character does for a while besides walking (a dodge, a sword swing, drawing
//! a shield or a bow), and the projectiles they throw. Plugins give the numbers and choices; the
//! runtime's generic action runner plays them (see `clockwork_runtime::actions`).
//!
//! An action plays an animation per facing, moves the character in a way (rooted, slowed or sliding
//! along a speed profile), is dangerous in places (hit windows) and can throw projectiles. After
//! its own animation it may `hold` a pose as long as its button stays down (a charged sword, a raised
//! shield), and on release perform another action (the spin attack, the arrow's flight).
//!
//! All values are in meters, seconds and degrees, and times count from the start of the action.

use serde::{Deserialize, Serialize};

use crate::character2d::Directional;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    /// The sprite animation per facing. It plays from the start; the action ends after
    /// `duration` (animations often end in a long final hold).
    pub animation: Directional,
    pub duration: f32,
    #[serde(default)]
    pub motion: Motion,
    #[serde(default)]
    pub hits: Vec<HitWindow>,
    /// While the action lasts the character can't be hurt, between these times.
    #[serde(default)]
    pub invulnerable: Option<(f32, f32)>,
    #[serde(default)]
    pub spawns: Vec<Spawn>,
    /// What happens after `duration` while the button that started the action stays down.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Another action can interrupt this one, from this time on (a dodge can be followed at once
    /// by the next action). Without it only the end frees the character.
    #[serde(default)]
    pub cancel_from: Option<f32>,
    /// Sprites drawn with the character while the action (and its hold) lasts: the sword in
    /// the hand, the raised shield, the drawn bow.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// Everything a character can do besides moving: its actions, what they throw, and which one the
/// dodge button does. 2D and 3D characters have the same moves, so moves can go from one to the
/// other (see [`crate::ability`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moves {
    pub actions: Vec<Action>,
    pub projectiles: Vec<Projectile>,
    pub dodge: Option<String>,
}

impl Moves {
    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn projectile(&self, id: &str) -> Option<&Projectile> {
        self.projectiles.iter().find(|p| p.id == id)
    }

    /// Adds moves that aren't there yet (by id); the dodge only if there is none.
    pub fn merge(&mut self, other: Moves) {
        for action in other.actions {
            if self.action(&action.id).is_none() {
                self.actions.push(action);
            }
        }
        for projectile in other.projectiles {
            if self.projectile(&projectile.id).is_none() {
                self.projectiles.push(projectile);
            }
        }
        if self.dodge.is_none() {
            self.dodge = other.dodge;
        }
    }
}

/// A sprite drawn together with the character, at the character's origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    /// A sprite output of the same plugin.
    pub sprite: String,
    pub frames: AttachmentFrames,
    /// Drawn behind the character instead of in front of it.
    #[serde(default)]
    pub behind: bool,
    /// The frame shown by characters it can't follow frame by frame (3D characters, other
    /// games' sprites), held at their hand.
    #[serde(default)]
    pub still: Option<usize>,
}

/// How an attachment picks the frame it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttachmentFrames {
    /// Frame by frame with the character: the attachment's frame is the character's frame plus
    /// `offset`, and it is mirrored when the character's frame is. Games that draw held items as
    /// a second sprite in step with the character do this. Character frames outside
    /// `from..until` (when given) show nothing, nor do frames the attachment doesn't have.
    FollowFrame {
        offset: i32,
        #[serde(default)]
        from: Option<u32>,
        #[serde(default)]
        until: Option<u32>,
    },
    /// One of the attachment's own animations per facing, from the start of the action.
    Animation(Directional),
}

impl AttachmentFrames {
    /// The attachment frame shown with character frame `frame`, if any.
    pub fn follow(&self, frame: usize, frames: usize) -> Option<usize> {
        let AttachmentFrames::FollowFrame {
            offset,
            from,
            until,
        } = self
        else {
            return None;
        };
        let frame = frame as i64;
        if from.is_some_and(|f| frame < f as i64) || until.is_some_and(|u| frame >= u as i64) {
            return None;
        }
        usize::try_from(frame + *offset as i64)
            .ok()
            .filter(|f| *f < frames)
    }
}

/// How an action moves the character.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Motion {
    /// Standing still.
    #[default]
    Rooted,
    /// Walking as usual, at this fraction of the walking speed.
    Walk(f32),
    /// Moving along the facing regardless of input, at these speeds: `(from, speed)` pairs, each
    /// speed holding until the next time (m/s).
    Slide(Vec<(f32, f32)>),
}

/// A part of an action where it hurts what it touches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitWindow {
    pub from: f32,
    pub until: f32,
    pub area: HitArea,
    pub damage: f32,
    /// How far it pushes what it hits, in meters.
    #[serde(default)]
    pub knockback: f32,
}

/// Where a hit lands, relative to the character's feet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HitArea {
    /// A box in front: from `near` to `far` meters ahead, `half_width` to either side.
    Front {
        near: f32,
        far: f32,
        half_width: f32,
    },
    /// A circle around the character.
    Around { radius: f32 },
}

/// A projectile thrown at a time of an action: `ahead` meters in front of the character.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub at: f32,
    /// A [`Projectile::id`] of the character.
    pub projectile: String,
    #[serde(default)]
    pub ahead: f32,
}

/// Staying in a pose while the button that started the action is held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    /// The pose, per facing. It loops.
    pub animation: Directional,
    /// Played instead while walking, if the character walks in the pose.
    #[serde(default)]
    pub walk_animation: Option<Directional>,
    /// Walking speed in the pose, as a fraction of the normal one (0 stands still). The
    /// character keeps facing the way it faced.
    #[serde(default = "default_walk_factor")]
    pub walk_factor: f32,
    /// Blocks hits from the front while held: the half angle of the arc it covers, in degrees.
    #[serde(default)]
    pub guard: Option<f32>,
    /// What happens when the button is let go.
    #[serde(default)]
    pub release: Option<Release>,
}

fn default_walk_factor() -> f32 {
    1.0
}

/// Letting go of a hold: perform `action` if the pose was held at least `min_time` seconds
/// (a sword charged long enough spins); otherwise the character just goes back to normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub action: String,
    #[serde(default)]
    pub min_time: f32,
}

/// A thing an action throws or places: arrows, bombs, things that come back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projectile {
    pub id: String,
    pub motion: ProjectileMotion,
    /// Seconds until it disappears, or bursts when it has a `burst`.
    pub lifetime: f32,
    /// Its size on the ground.
    pub radius: f32,
    /// Damage to what it touches on its way (0: none).
    #[serde(default)]
    pub damage: f32,
    /// What it does at the end of its lifetime.
    #[serde(default)]
    pub burst: Option<Burst>,
    /// It stops at walls (arrows, bombs); otherwise it flies through them.
    #[serde(default = "default_true")]
    pub stops_at_walls: bool,
    /// At most this many of it exist at once (a character can't place a second bomb).
    #[serde(default)]
    pub limit: Option<u32>,
    /// How it looks without a sprite: RGB, 0..1.
    #[serde(default)]
    pub color: [f32; 3],
    /// How it looks: a sprite output of the same plugin.
    #[serde(default)]
    pub sprite: Option<ProjectileSprite>,
}

/// A projectile's sprite: an animation per direction it flies in (the facing it was thrown with),
/// and optionally another while it bursts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectileSprite {
    pub sprite: String,
    pub animation: Directional,
    #[serde(default)]
    pub burst: Option<String>,
    /// Drawn this far above the ground, in meters (arrows fly at hand height).
    #[serde(default)]
    pub height: f32,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectileMotion {
    /// Stays where it was put.
    Stationary,
    /// Flies straight ahead.
    Straight { speed: f32 },
    /// Flies ahead `range` meters, then returns to the character and ends there.
    #[serde(alias = "Boomerang")]
    Return { speed: f32, range: f32 },
}

/// An explosion: hurts everything within `radius` and lasts `duration` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Burst {
    pub radius: f32,
    pub damage: f32,
    pub duration: f32,
    #[serde(default)]
    pub knockback: f32,
}

impl Action {
    /// The speed of a sliding action `seconds` into it.
    pub fn slide_speed(&self, seconds: f32) -> Option<f32> {
        match &self.motion {
            Motion::Slide(speeds) => speeds
                .iter()
                .take_while(|(from, _)| *from <= seconds)
                .last()
                .map(|(_, speed)| *speed)
                .or(Some(0.0)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directional(name: &str) -> Directional {
        let one = |flip_x| crate::character2d::AnimationRef {
            name: name.into(),
            flip_x,
            flip_y: false,
        };
        Directional {
            north: one(false),
            east: one(false),
            south: one(false),
            west: one(true),
        }
    }

    #[test]
    fn slides_follow_their_speed_steps() {
        let action = Action {
            id: "dodge".into(),
            animation: directional("dodge"),
            duration: 0.4,
            motion: Motion::Slide(vec![(0.0, 6.0), (0.1, 4.0), (0.3, 0.0)]),
            hits: vec![],
            invulnerable: None,
            spawns: vec![],
            hold: None,
            cancel_from: None,
            attachments: vec![],
        };
        assert_eq!(action.slide_speed(0.0), Some(6.0));
        assert_eq!(action.slide_speed(0.2), Some(4.0));
        assert_eq!(action.slide_speed(0.35), Some(0.0));
    }

    #[test]
    fn attachments_follow_the_characters_frame() {
        let follow = AttachmentFrames::FollowFrame {
            offset: -10,
            from: Some(10),
            until: Some(20),
        };
        assert_eq!(follow.follow(12, 100), Some(2));
        assert_eq!(follow.follow(9, 100), None);
        assert_eq!(follow.follow(20, 100), None);
        // Frames the attachment doesn't have show nothing.
        assert_eq!(follow.follow(15, 3), None);
        let open = AttachmentFrames::FollowFrame {
            offset: 5,
            from: None,
            until: None,
        };
        assert_eq!(open.follow(0, 10), Some(5));
    }

    #[test]
    fn parses_ron() {
        let action: Action = ron::from_str(
            r#"(
                id: "slash",
                animation: (north: (name: "a"), east: (name: "b"), south: (name: "c"),
                            west: (name: "b", flip_x: true)),
                duration: 0.25,
                motion: Rooted,
                hits: [(from: 0.03, until: 0.17, area: Front(near: 0.2, far: 1.4, half_width: 0.7),
                        damage: 1.0)],
                hold: Some((
                    animation: (north: (name: "h"), east: (name: "h"), south: (name: "h"),
                                west: (name: "h", flip_x: true)),
                    walk_factor: 0.7,
                    release: Some((action: "spin", min_time: 1.3)),
                )),
            )"#,
        )
        .unwrap();
        assert_eq!(action.hits.len(), 1);
        let hold = action.hold.unwrap();
        assert_eq!(hold.walk_factor, 0.7);
        assert_eq!(hold.release.unwrap().action, "spin");
        assert_eq!(action.motion, Motion::Rooted);
    }
}

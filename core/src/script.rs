//! Scripts: commands a game runs one after another while it plays, for what happens over time
//! rather than at once: a cutscene, a boss's entrance, a door that opens after a lever and a
//! pause, a character walking to the player to talk. The effect `Run` starts one; several can run
//! at a time. Nothing in it names a game: a game's event scripts (its cutscene bytecode, its
//! timers) are written as these commands.
//!
//! ```ron
//! Run((hold_player: true, steps: [
//!     Do(Music("tmc-music/boss")),
//!     Walk(node: "Guard", to: "Gate", speed: 2.0),
//!     Wait(0.5),
//!     Do(Say((speaker: Some("Guard"), pages: ["Halt!"]))),
//!     Until(Flag("lever")),
//!     Do(Set("gate_open")),
//! ]))
//! ```

use serde::{Deserialize, Serialize};

use crate::behavior::{Condition, Effect};

/// What `Walk` and `Place` name to mean the played character.
pub const PLAYER: &str = "player";

/// Commands run in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub steps: Vec<Command>,
    /// The player can't move or act until the script ends (as in a cutscene); `Hold` changes it
    /// on the way.
    #[serde(default)]
    pub hold_player: bool,
}

/// One thing a script does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Any effect, at once: a message, a dialog (the game waits while it is open), a sound,
    /// music, a flag, items, a teleport, another script.
    Do(Effect),
    /// Waits this many seconds.
    Wait(f32),
    /// Waits until a condition of the game holds.
    Until(Condition),
    /// Walks node `node` (or the player, `"player"`) in a straight line to where node `to` stands,
    /// `speed` meters a second, and waits until it gets there. Its behaviors go with it.
    Walk {
        node: String,
        to: String,
        speed: f32,
    },
    /// Puts node `node` (or the player) where node `to` stands, at once.
    Place { node: String, to: String },
    /// Stops the player moving and acting (true) or lets it again (false).
    Hold(bool),
    /// Runs `then` if the condition holds now, else `otherwise`.
    If {
        when: Condition,
        then: Vec<Command>,
        #[serde(default)]
        otherwise: Vec<Command>,
    },
    /// Runs `steps` `times` times, or for as long as the game plays without.
    Repeat {
        #[serde(default)]
        times: Option<u32>,
        steps: Vec<Command>,
    },
    /// Ends the script here.
    End,
}

impl Script {
    /// What is wrong with it: walks that never arrive and loops that never wait.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        check(&self.steps, &mut problems);
        problems
    }

    /// The nodes it names (walked, placed or walked to), for checking that the scene has them.
    pub fn nodes(&self) -> Vec<&str> {
        let mut names = Vec::new();
        collect(&self.steps, &mut names);
        names.retain(|n| *n != PLAYER);
        names
    }

    /// The effects it can do, for finding the assets and nodes they name.
    pub fn effects(&self) -> Vec<&Effect> {
        let mut effects = Vec::new();
        effects_of(&self.steps, &mut effects);
        effects
    }
}

fn check(steps: &[Command], problems: &mut Vec<String>) {
    for step in steps {
        match step {
            Command::Walk { node, speed, .. } if *speed <= 0.0 => problems.push(format!(
                "\"{node}\" walks at speed {speed}, so it never arrives"
            )),
            Command::Repeat { times: None, steps } if !waits(steps) => problems.push(
                "a Repeat without times never waits (add a Wait, Until or Walk to it)".into(),
            ),
            Command::If {
                then, otherwise, ..
            } => {
                check(then, problems);
                check(otherwise, problems);
            }
            Command::Repeat { steps, .. } => check(steps, problems),
            _ => {}
        }
    }
}

/// Whether running `steps` always takes time.
fn waits(steps: &[Command]) -> bool {
    steps.iter().any(|s| match s {
        Command::Wait(seconds) => *seconds > 0.0,
        Command::Until(_) | Command::Walk { .. } => true,
        Command::If {
            then, otherwise, ..
        } => waits(then) && waits(otherwise),
        Command::Repeat { times, steps } => times.is_none_or(|t| t > 0) && waits(steps),
        _ => false,
    })
}

fn collect<'a>(steps: &'a [Command], names: &mut Vec<&'a str>) {
    for step in steps {
        match step {
            Command::Walk { node, to, .. } | Command::Place { node, to } => {
                names.push(node);
                names.push(to);
            }
            Command::If {
                then, otherwise, ..
            } => {
                collect(then, names);
                collect(otherwise, names);
            }
            Command::Repeat { steps, .. } => collect(steps, names),
            _ => {}
        }
    }
}

fn effects_of<'a>(steps: &'a [Command], effects: &mut Vec<&'a Effect>) {
    for step in steps {
        match step {
            Command::Do(effect) => effects.push(effect),
            Command::If {
                then, otherwise, ..
            } => {
                effects_of(then, effects);
                effects_of(otherwise, effects);
            }
            Command::Repeat { steps, .. } => effects_of(steps, effects),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_doc_example_reads() {
        let effect: Effect = ron::from_str(
            r#"Run((hold_player: true, steps: [
                Do(Music("tmc-music/boss")),
                Walk(node: "Guard", to: "Gate", speed: 2.0),
                Wait(0.5),
                Do(Say((speaker: Some("Guard"), pages: ["Halt!"]))),
                Until(Flag("lever")),
                Do(Set("gate_open")),
            ]))"#,
        )
        .unwrap();
        let Effect::Run(script) = effect else {
            panic!("not a script")
        };
        assert!(script.hold_player);
        assert_eq!(script.steps.len(), 6);
        assert_eq!(script.nodes(), ["Guard", "Gate"]);
        assert_eq!(script.effects().len(), 3);
        assert!(script.problems().is_empty());
    }

    #[test]
    fn loops_that_never_wait_and_walks_that_never_arrive_are_problems() {
        let script: Script = ron::from_str(
            r#"(steps: [
                Repeat(steps: [Do(Set("a")), If(when: Flag("b"), then: [Wait(1.0)])]),
                Walk(node: "player", to: "Door", speed: 0.0),
                Repeat(steps: [Wait(1.0)]),
            ])"#,
        )
        .unwrap();
        assert_eq!(script.problems().len(), 2);
        assert_eq!(script.nodes(), ["Door"]);
    }
}

//! Running scripts (see [`clockwork_core::script`]): each keeps its place in its commands and
//! does as many as it can each frame, stopping at the first that takes time. What the commands
//! touch (conditions, effects, where nodes are) comes from a [`Stage`], so this knows nothing of
//! the rest of Play.

use clockwork_core::{
    behavior::{Condition, Effect},
    script::{Command, Script},
};

/// Commands a script may run in one frame before it is taken to never wait.
const MOST: usize = 1000;

/// What scripts act on.
pub trait Stage {
    fn holds(&self, condition: &Condition) -> bool;
    fn effect(&mut self, effect: Effect);
    /// Moves node `node` up to `distance` meters towards where `to` stands: true once it is
    /// there (or either can't be found, which is a problem said once).
    fn walk(&mut self, node: &str, to: &str, distance: f32) -> bool;
    fn place(&mut self, node: &str, to: &str);
    fn problem(&mut self, text: String);
}

/// A script being run.
#[derive(Debug, Clone)]
pub struct Running {
    blocks: Vec<Block>,
    /// Seconds spent in the current `Wait`.
    waited: f32,
    /// The player can't move or act.
    pub hold: bool,
}

/// Commands being run: the script's own, or those of an `If` or a `Repeat`.
#[derive(Debug, Clone)]
struct Block {
    steps: Vec<Command>,
    at: usize,
    /// Times to run again once at the end; forever without.
    again: Option<u32>,
}

impl Running {
    pub fn new(script: Script) -> Self {
        Self {
            blocks: vec![Block {
                steps: script.steps,
                at: 0,
                again: Some(0),
            }],
            waited: 0.0,
            hold: script.hold_player,
        }
    }

    /// Runs the script for `dt` seconds: false once it has ended.
    pub fn step(&mut self, dt: f32, stage: &mut impl Stage) -> bool {
        // Time not yet used by a command that takes time.
        let mut left = dt;
        for _ in 0..MOST {
            let Some(block) = self.blocks.last_mut() else {
                return false;
            };
            let Some(command) = block.steps.get(block.at).cloned() else {
                match &mut block.again {
                    Some(0) => {
                        self.blocks.pop();
                    }
                    Some(n) => {
                        *n -= 1;
                        block.at = 0;
                    }
                    None => block.at = 0,
                }
                continue;
            };
            match command {
                Command::Do(effect) => stage.effect(effect),
                Command::Wait(seconds) => {
                    self.waited += left;
                    if self.waited < seconds {
                        return true;
                    }
                    left = self.waited - seconds;
                    self.waited = 0.0;
                }
                Command::Until(condition) => {
                    if !stage.holds(&condition) {
                        return true;
                    }
                }
                Command::Walk { node, to, speed } => {
                    let there = stage.walk(&node, &to, speed * left);
                    if !there {
                        return true;
                    }
                    // Arriving takes the time it takes; the rest goes on.
                    left = 0.0;
                }
                Command::Place { node, to } => stage.place(&node, &to),
                Command::Hold(hold) => self.hold = hold,
                Command::If {
                    when,
                    then,
                    otherwise,
                } => {
                    let steps = if stage.holds(&when) { then } else { otherwise };
                    self.blocks.last_mut().unwrap().at += 1;
                    self.blocks.push(Block {
                        steps,
                        at: 0,
                        again: Some(0),
                    });
                    continue;
                }
                Command::Repeat { times, steps } => {
                    self.blocks.last_mut().unwrap().at += 1;
                    if times != Some(0) {
                        self.blocks.push(Block {
                            steps,
                            at: 0,
                            again: times.map(|t| t - 1),
                        });
                    }
                    continue;
                }
                Command::End => {
                    self.blocks.clear();
                    return false;
                }
            }
            if let Some(block) = self.blocks.last_mut() {
                block.at += 1;
            }
        }
        stage.problem(format!(
            "A script ran {MOST} commands in one frame without waiting, so it was stopped"
        ));
        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;

    /// Flags, and nodes standing on a line.
    #[derive(Default)]
    struct Line {
        flags: BTreeSet<String>,
        at: HashMap<String, f32>,
        did: Vec<String>,
        problems: Vec<String>,
    }

    impl Stage for Line {
        fn holds(&self, condition: &Condition) -> bool {
            match condition {
                Condition::Flag(f) => self.flags.contains(f),
                _ => false,
            }
        }
        fn effect(&mut self, effect: Effect) {
            match effect {
                Effect::Set(f) => {
                    self.did.push(format!("set {f}"));
                    self.flags.insert(f);
                }
                Effect::Message(m) => self.did.push(m),
                _ => {}
            }
        }
        fn walk(&mut self, node: &str, to: &str, distance: f32) -> bool {
            let goal = self.at[to];
            let here = self.at.get_mut(node).unwrap();
            let gap = goal - *here;
            *here += gap.clamp(-distance, distance);
            (*here - goal).abs() < 1e-4
        }
        fn place(&mut self, node: &str, to: &str) {
            let goal = self.at[to];
            self.at.insert(node.into(), goal);
        }
        fn problem(&mut self, text: String) {
            self.problems.push(text);
        }
    }

    fn script(text: &str) -> Running {
        Running::new(ron::from_str(text).unwrap())
    }

    #[test]
    fn waits_take_their_time_and_carry_what_is_left() {
        let mut run =
            script(r#"(steps: [Wait(0.25), Do(Message("a")), Wait(0.25), Do(Message("b"))])"#);
        let mut line = Line::default();
        assert!(run.step(0.2, &mut line));
        assert!(line.did.is_empty());
        // 0.4 seconds in: past the first wait, partway through the second.
        assert!(run.step(0.2, &mut line));
        assert_eq!(line.did, ["a"]);
        assert!(!run.step(0.2, &mut line));
        assert_eq!(line.did, ["a", "b"]);
    }

    #[test]
    fn until_waits_for_its_condition_and_if_picks_a_branch() {
        let mut run = script(
            r#"(steps: [
                Until(Flag("lever")),
                If(when: Flag("key"), then: [Do(Message("open"))], otherwise: [Do(Message("locked"))]),
            ])"#,
        );
        let mut line = Line::default();
        assert!(run.step(1.0, &mut line));
        line.flags.insert("lever".into());
        assert!(!run.step(0.0, &mut line));
        assert_eq!(line.did, ["locked"]);
    }

    #[test]
    fn walks_go_at_their_speed_and_place_is_at_once() {
        let mut run = script(
            r#"(hold_player: true, steps: [
                Walk(node: "guard", to: "gate", speed: 2.0),
                Hold(false),
                Place(node: "guard", to: "home"),
            ])"#,
        );
        let mut line = Line {
            at: HashMap::from([
                ("guard".into(), 0.0),
                ("gate".into(), 3.0),
                ("home".into(), -1.0),
            ]),
            ..Default::default()
        };
        assert!(run.hold);
        assert!(run.step(1.0, &mut line));
        assert_eq!(line.at["guard"], 2.0);
        assert!(!run.step(1.0, &mut line));
        assert!(!run.hold);
        assert_eq!(line.at["guard"], -1.0);
    }

    #[test]
    fn repeats_count_and_endless_loops_without_waits_are_stopped() {
        let mut run = script(
            r#"(steps: [Repeat(times: Some(3), steps: [Do(Message("x"))]), End, Do(Message("never"))])"#,
        );
        let mut line = Line::default();
        assert!(!run.step(0.0, &mut line));
        assert_eq!(line.did, ["x", "x", "x"]);
        let mut forever = script(r#"(steps: [Repeat(steps: [Do(Set("a"))])])"#);
        assert!(!forever.step(0.0, &mut line));
        assert_eq!(line.problems.len(), 1);
        let mut ticking = script(r#"(steps: [Repeat(steps: [Wait(1.0), Do(Message("tick"))])])"#);
        let mut line = Line::default();
        for _ in 0..3 {
            assert!(ticking.step(1.0, &mut line));
        }
        assert_eq!(line.did, ["tick", "tick", "tick"]);
    }
}

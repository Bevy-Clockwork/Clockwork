//! The order steps run in, drawn over the graph: a path from each step to the next, through
//! the closed groups that hold them.

use std::collections::{BTreeMap, HashMap};

use bevy_egui::egui::{Rect, Shape, Stroke, pos2, vec2};

use super::groups::{GNode, GroupId, GroupInfo};
use super::{RNode, theme};

/// Each drawn node in the order its steps run, with the first of its steps: a closed group
/// stands for the steps inside it, and runs of steps in one node are one stop. Steps that are
/// not drawn (in a group that is not either) are left out.
pub fn run_path(
    steps: usize,
    drawn: &HashMap<GNode, Rect>,
    groups: &BTreeMap<GroupId, GroupInfo>,
    group_of: impl Fn(usize) -> Option<GroupId>,
) -> Vec<(usize, Rect)> {
    let mut path: Vec<(usize, GNode, Rect)> = Vec::new();
    for step in 0..steps {
        let item = GNode::Item(RNode::Step(step));
        let mut at = drawn.get(&item).map(|r| (item, *r));
        let mut group = group_of(step);
        while at.is_none()
            && let Some(g) = group
        {
            at = drawn.get(&GNode::Group(g)).map(|r| (GNode::Group(g), *r));
            group = groups.get(&g).and_then(|i| i.parent);
        }
        let Some((node, rect)) = at else { continue };
        if path.last().is_none_or(|(_, last, _)| *last != node) {
            path.push((step, node, rect));
        }
    }
    path.into_iter().map(|(s, _, r)| (s, r)).collect()
}

/// Draws the path: a line with an arrow from each stop to the next, and each stop's first step
/// number on its top left corner.
pub fn draw(painter: &bevy_egui::egui::Painter, path: &[(usize, Rect)]) {
    let color = theme::BRASS_BRIGHT.gamma_multiply(0.7);
    for pair in path.windows(2) {
        let (a, b) = (pair[0].1, pair[1].1);
        let (from, to) = (a.center(), b.center());
        painter.add(Shape::dashed_line(
            &[from, to],
            Stroke::new(2.0, color),
            10.0,
            6.0,
        ));
        // An arrowhead half way.
        let mid = from + (to - from) * 0.5;
        let dir = (to - from).normalized();
        let side = vec2(-dir.y, dir.x);
        painter.add(Shape::convex_polygon(
            vec![
                mid + dir * 9.0,
                mid - dir * 6.0 + side * 6.0,
                mid - dir * 6.0 - side * 6.0,
            ],
            color,
            Stroke::NONE,
        ));
    }
    for (step, rect) in path {
        // Left of the node, on the background, so the node does not cover it.
        let at = rect.left_top() + vec2(-18.0, 14.0);
        painter.circle_filled(at, 13.0, theme::BRASS);
        painter.text(
            pos2(at.x, at.y),
            bevy_egui::egui::Align2::CENTER_CENTER,
            step.to_string(),
            theme::strong(12.0),
            theme::BG_DEEP,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_egui::egui::Pos2;

    fn rect(x: f32) -> Rect {
        Rect::from_min_size(pos2(x, 0.0), vec2(10.0, 10.0))
    }

    #[test]
    fn closed_groups_are_one_stop_for_the_steps_in_them() {
        let groups = BTreeMap::from([(
            7,
            GroupInfo {
                name: "Tiles".into(),
                parent: None,
                open: false,
                pos: Pos2::ZERO,
                laid_out: false,
            },
        )]);
        let drawn = HashMap::from([
            (GNode::Item(RNode::Step(0)), rect(0.0)),
            (GNode::Group(7), rect(100.0)),
            (GNode::Item(RNode::Step(3)), rect(200.0)),
        ]);
        // Steps 1 and 2 are in the group; step 4 is nowhere drawn.
        let group_of = |s: usize| (1..=2).contains(&s).then_some(7);
        let path = run_path(5, &drawn, &groups, group_of);
        let stops: Vec<(usize, f32)> = path.iter().map(|(s, r)| (*s, r.min.x)).collect();
        assert_eq!(stops, [(0, 0.0), (1, 100.0), (3, 200.0)]);
    }
}

//! Groups: steps and outputs drawn as one node, so a recipe of a thousand steps reads as a
//! handful of parts. A group can hold groups. Closed, it shows the values it reads from outside
//! and the ones it makes that are used outside; opened, its nodes are drawn in place inside a
//! frame. Groups are only a view: they live in `graph.ron`, never in `recipe.ron`.
//!
//! Recipes the agent wrote are grouped by their section comments (`// == Tiles ==`), or, for a
//! big recipe without them, by the output each step goes into.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy_egui::egui::{Pos2, Vec2, pos2, vec2};
use clockwork_core::recipe::Recipe;
use clockwork_recipe::text::Doc;
use serde::{Deserialize, Serialize};

use super::{OwnedPort, Places, RNode, Resolution};

pub type GroupId = u32;

/// Recipes with at least this many nodes are grouped when first opened.
pub const AUTO_GROUP: usize = 40;
/// Without section comments, only recipes this big are grouped by their outputs.
pub const AUTO_GROUP_BY_OUTPUTS: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub struct GroupInfo {
    pub name: String,
    /// The group it is in.
    pub parent: Option<GroupId>,
    pub open: bool,
    /// Where it is drawn while closed.
    pub pos: Pos2,
    /// Whether its nodes were given places near it (they are, the first time it opens).
    pub laid_out: bool,
}

/// A group as `graph.ron` keeps it: its nodes by their keys (see [`stable_keys`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedGroup {
    pub id: GroupId,
    pub name: String,
    #[serde(default)]
    pub parent: Option<GroupId>,
    #[serde(default)]
    pub open: bool,
    pub at: (f32, f32),
    #[serde(default)]
    pub laid_out: bool,
    pub members: Vec<String>,
}

/// What a node of the snarl stands for: a step or output, or a closed group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GNode {
    Item(RNode),
    Group(GroupId),
}

/// A closed group's inputs (one per value read from outside, then one per input that reads
/// nothing) and outputs (one per value made inside and used outside).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pins {
    pub inputs: Vec<GroupInput>,
    pub outputs: Vec<(RNode, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupInput {
    /// What feeds it; `None` for an input inside that reads nothing.
    pub from: Option<(RNode, usize)>,
    /// The inputs inside that read it.
    pub targets: Vec<(RNode, usize)>,
}

/// A key for every node that stays the same when steps are added, removed or moved elsewhere
/// in the recipe: its variant and the first name it makes (or uses), numbered when several
/// nodes share both. Outputs are keyed by their id.
pub fn stable_keys(
    recipe: &Recipe,
    ports: &HashMap<RNode, Vec<OwnedPort>>,
) -> Vec<(RNode, String)> {
    let mut keys = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut push = |node: RNode, base: String| {
        let n = seen.entry(base.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { base } else { format!("{base}#{n}") };
        keys.push((node, key));
    };
    for (i, step) in recipe.steps.iter().enumerate() {
        let node = RNode::Step(i);
        let node_ports = ports.get(&node).map(Vec::as_slice).unwrap_or_default();
        let name = node_ports
            .iter()
            .find(|p| p.access.writes() && !p.name.is_empty())
            .or_else(|| node_ports.iter().find(|p| !p.name.is_empty()))
            .map_or("", |p| p.name.as_str());
        push(node, format!("{}:{name}", step.name()));
    }
    for (i, output) in recipe.outputs.iter().enumerate() {
        push(
            RNode::Output(i),
            format!("output:{}:{}", output.name(), output.id()),
        );
    }
    keys
}

/// The groups a node or group is in, innermost first.
pub fn ancestors(groups: &BTreeMap<GroupId, GroupInfo>, mut at: Option<GroupId>) -> Vec<GroupId> {
    let mut chain = Vec::new();
    while let Some(g) = at {
        if chain.contains(&g) {
            break;
        }
        chain.push(g);
        at = groups.get(&g).and_then(|info| info.parent);
    }
    chain
}

/// What stands for a node in the snarl: the outermost closed group it is in, or itself.
pub fn shown_as(groups: &BTreeMap<GroupId, GroupInfo>, places: &Places, node: RNode) -> GNode {
    let group = places.get(&node).and_then(|p| p.group);
    shown_group(groups, group).map_or(GNode::Item(node), GNode::Group)
}

/// The outermost closed group among a group and those it is in.
pub fn shown_group(
    groups: &BTreeMap<GroupId, GroupInfo>,
    group: Option<GroupId>,
) -> Option<GroupId> {
    ancestors(groups, group)
        .into_iter()
        .rfind(|g| groups.get(g).is_some_and(|info| !info.open))
}

/// Whether `group` is `inside` or a group in it.
pub fn within(
    groups: &BTreeMap<GroupId, GroupInfo>,
    group: Option<GroupId>,
    inside: GroupId,
) -> bool {
    ancestors(groups, group).contains(&inside)
}

/// The nodes in a group or the groups in it.
pub fn members(
    groups: &BTreeMap<GroupId, GroupInfo>,
    places: &Places,
    group: GroupId,
) -> Vec<RNode> {
    let mut nodes: Vec<RNode> = places
        .iter()
        .filter(|(_, p)| within(groups, p.group, group))
        .map(|(n, _)| *n)
        .collect();
    nodes.sort_by_key(|n| super::order(*n));
    nodes
}

/// The pins of each closed group drawn, from the recipe's wires.
pub fn pins(
    groups: &BTreeMap<GroupId, GroupInfo>,
    places: &Places,
    resolution: &Resolution,
) -> HashMap<GroupId, Pins> {
    let mut pins: HashMap<GroupId, Pins> = HashMap::new();
    let mut wires = resolution.wires.clone();
    wires.sort_by_key(|((_, _), (to, port))| (super::order(*to), *port));
    for (from, to) in wires {
        let (a, b) = (
            shown_as(groups, places, from.0),
            shown_as(groups, places, to.0),
        );
        if a == b {
            continue;
        }
        if let GNode::Group(g) = a {
            let outputs = &mut pins.entry(g).or_default().outputs;
            if !outputs.contains(&from) {
                outputs.push(from);
            }
        }
        if let GNode::Group(g) = b {
            let inputs = &mut pins.entry(g).or_default().inputs;
            match inputs.iter_mut().find(|i| i.from == Some(from)) {
                Some(input) => input.targets.push(to),
                None => inputs.push(GroupInput {
                    from: Some(from),
                    targets: vec![to],
                }),
            }
        }
    }
    let mut missing: Vec<(RNode, usize)> = resolution.missing.iter().copied().collect();
    missing.sort_by_key(|(n, p)| (super::order(*n), *p));
    for at in missing {
        if let GNode::Group(g) = shown_as(groups, places, at.0) {
            pins.entry(g).or_default().inputs.push(GroupInput {
                from: None,
                targets: vec![at],
            });
        }
    }
    pins
}

/// A recipe's sections, from its comments: each one starts at a step (or output) with a header
/// above it and runs until the next. Headers are comment lines marked `==`, `--` or `##`
/// (`// == Tiles ==`); without any, a comment block after a blank line starts a section, named
/// by its first line. Sections of one node are left out.
pub fn sections(doc: &Doc, list: &str) -> Vec<(String, Vec<usize>)> {
    let Some(seq) = doc.root.field(list) else {
        return Vec::new();
    };
    let items = seq.items();
    let mut strong: Vec<(usize, String)> = Vec::new();
    let mut weak: Vec<(usize, String)> = Vec::new();
    let mut start = seq.span.start + 1;
    for (i, item) in items.iter().enumerate() {
        let gap = &doc.text[start.min(item.span.start)..item.span.start];
        // What follows the previous item (or the list's `[`) on its line is not a header.
        let gap = gap.split_once('\n').map_or("", |(_, rest)| rest);
        let lines: Vec<&str> = gap.lines().map(str::trim).collect();
        if let Some(header) = lines.iter().find_map(|l| marked_header(l)) {
            strong.push((i, header));
        } else if let Some(first) = blank_then_comment(&lines) {
            weak.push((i, first));
        }
        start = item.span.end;
    }
    let headers = if strong.is_empty() { weak } else { strong };
    let mut found = Vec::new();
    for (n, (at, name)) in headers.iter().enumerate() {
        let end = headers.get(n + 1).map_or(items.len(), |(next, _)| *next);
        if end - at >= 2 {
            found.push((name.clone(), (*at..end).collect()));
        }
    }
    found
}

/// The name of a header comment: `// == Tiles ==`, `// -- Tiles`, `// ## Tiles`.
fn marked_header(line: &str) -> Option<String> {
    let text = line.strip_prefix("//")?.trim();
    let marked = ["==", "--", "##"].iter().any(|m| text.starts_with(m));
    if !marked {
        return None;
    }
    let name = text.trim_matches(|c: char| c == '=' || c == '-' || c == '#' || c.is_whitespace());
    (!name.is_empty()).then(|| short(name))
}

/// The first line of a comment block that follows a blank line.
fn blank_then_comment(lines: &[&str]) -> Option<String> {
    let blank = lines.iter().position(|l| l.is_empty())?;
    let comment = lines[blank..].iter().find(|l| l.starts_with("//"))?;
    let name = comment.trim_start_matches('/').trim();
    (!name.is_empty()).then(|| short(name))
}

fn short(name: &str) -> String {
    let name = name.trim_end_matches([':', '.']);
    if name.chars().count() > 40 {
        format!("{}…", name.chars().take(39).collect::<String>())
    } else {
        name.to_string()
    }
}

/// Groups for a big recipe without sections: the steps each output is made from (with the
/// output), the steps several outputs share, and the steps no output uses.
pub fn by_outputs(
    recipe: &Recipe,
    resolution: &Resolution,
    ids: &[String],
) -> Vec<(String, Vec<RNode>)> {
    let mut used_by: HashMap<RNode, HashSet<usize>> = HashMap::new();
    for o in 0..recipe.outputs.len() {
        let mut stack = vec![RNode::Output(o)];
        let mut seen = HashSet::new();
        while let Some(at) = stack.pop() {
            for next in resolution.upstream.get(&at).into_iter().flatten() {
                if seen.insert(*next) {
                    used_by.entry(*next).or_default().insert(o);
                    stack.push(*next);
                }
            }
        }
    }
    let mut found: Vec<(String, Vec<RNode>)> = Vec::new();
    for (o, id) in ids.iter().enumerate() {
        let mut nodes: Vec<RNode> = (0..recipe.steps.len())
            .map(RNode::Step)
            .filter(|n| {
                used_by
                    .get(n)
                    .is_some_and(|u| u.len() == 1 && u.contains(&o))
            })
            .collect();
        nodes.push(RNode::Output(o));
        if nodes.len() >= 2 {
            found.push((format!("Makes {id}"), nodes));
        }
    }
    let shared: Vec<RNode> = (0..recipe.steps.len())
        .map(RNode::Step)
        .filter(|n| used_by.get(n).is_some_and(|u| u.len() > 1))
        .collect();
    if shared.len() >= 2 {
        found.push(("Shared".into(), shared));
    }
    let unused: Vec<RNode> = (0..recipe.steps.len())
        .map(RNode::Step)
        .filter(|n| !used_by.contains_key(n))
        .collect();
    if unused.len() >= 2 {
        found.push(("Used by no output".into(), unused));
    }
    found
}

/// Where the parts of a group go the first time it opens, from `at`, by part index. Parts wired together
/// form a cluster, laid out in columns (each part has its column); the clusters are then
/// packed in rows, in the order of their first part, so a group of many separate chains
/// opens as a grid rather than one long column.
pub fn local_layout(
    parts: &[(GNode, usize, Vec2)],
    links: &[(usize, usize)],
    at: Pos2,
) -> Vec<(usize, Pos2)> {
    // Clusters, by union-find over the links.
    let mut root: Vec<usize> = (0..parts.len()).collect();
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for &(a, b) in links {
        let (a, b) = (find(&mut root, a), find(&mut root, b));
        if a != b {
            root[a.max(b)] = a.min(b);
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..parts.len() {
        let r = find(&mut root, i);
        clusters.entry(r).or_default().push(i);
    }

    // Each cluster in its own columns, from its own top left.
    let mut laid: Vec<(Vec<(usize, Pos2)>, Vec2)> = Vec::new();
    for members in clusters.values() {
        let mut columns: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &i in members {
            columns.entry(parts[i].1).or_default().push(i);
        }
        let mut placed = Vec::new();
        let (mut x, mut height) = (0.0f32, 0.0f32);
        for column in columns.values() {
            let (mut y, mut width) = (0.0f32, 0.0f32);
            for &i in column {
                placed.push((i, pos2(x, y)));
                y += parts[i].2.y + super::GAP;
                width = width.max(parts[i].2.x);
            }
            height = height.max(y - super::GAP);
            x += width + super::GAP * 2.0;
        }
        laid.push((placed, vec2(x - super::GAP * 2.0, height)));
    }

    // Rows of clusters, about as wide as the whole is tall.
    let area: f32 = laid
        .iter()
        .map(|(_, size)| (size.x + super::GAP * 2.0) * (size.y + super::GAP * 2.0))
        .sum();
    let widest = laid.iter().map(|(_, size)| size.x).fold(0.0, f32::max);
    let row_width = (area.sqrt() * 1.6).max(widest);
    let mut out = Vec::with_capacity(parts.len());
    let (mut x, mut y, mut row_height) = (0.0f32, 0.0f32, 0.0f32);
    for (placed, size) in laid {
        if x > 0.0 && x + size.x > row_width {
            x = 0.0;
            y += row_height + super::GAP * 2.0;
            row_height = 0.0;
        }
        for (i, pos) in placed {
            out.push((i, at + vec2(x, y) + pos.to_vec2()));
        }
        x += size.x + super::GAP * 2.0;
        row_height = row_height.max(size.y);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_headers_start_sections() {
        let doc = Doc::parse(
            r#"(steps: [
    // == Tiles ==
    Fill(name: "a", size: 1),
    Fill(name: "b", size: 1), // b's own comment
    // -- Palette
    Fill(name: "c", size: 1),
    Fill(name: "d", size: 1),

    // a note, not a header, since there are marked ones
    Fill(name: "e", size: 1),
], outputs: [])"#,
        )
        .unwrap();
        assert_eq!(
            sections(&doc, "steps"),
            vec![
                ("Tiles".to_string(), vec![0, 1]),
                ("Palette".to_string(), vec![2, 3, 4]),
            ]
        );
    }

    #[test]
    fn without_markers_comment_blocks_after_blank_lines_start_sections() {
        let doc = Doc::parse(
            r#"(steps: [
    // The tiles: from the decomp's gfx table.
    Fill(name: "a", size: 1),
    // the next one
    Fill(name: "b", size: 1),

    // The palette.
    Fill(name: "c", size: 1),
    Fill(name: "d", size: 1),
    Fill(name: "e", size: 1),

    // Alone.
    Fill(name: "f", size: 1),
], outputs: [])"#,
        )
        .unwrap();
        assert_eq!(
            sections(&doc, "steps"),
            vec![("The palette".to_string(), vec![2, 3, 4])]
        );
    }

    fn info(parent: Option<GroupId>, open: bool) -> GroupInfo {
        GroupInfo {
            name: String::new(),
            parent,
            open,
            pos: Pos2::ZERO,
            laid_out: false,
        }
    }

    #[test]
    fn a_node_is_shown_as_its_outermost_closed_group() {
        let groups = BTreeMap::from([
            (1, info(None, true)),
            (2, info(Some(1), false)),
            (3, info(Some(2), false)),
        ]);
        let mut places = Places::new();
        let place = |group| super::super::Place {
            pos: Pos2::ZERO,
            open: true,
            group,
            exposed: Vec::new(),
        };
        places.insert(RNode::Step(0), place(Some(3)));
        places.insert(RNode::Step(1), place(Some(1)));
        places.insert(RNode::Step(2), place(None));
        assert_eq!(shown_as(&groups, &places, RNode::Step(0)), GNode::Group(2));
        assert_eq!(
            shown_as(&groups, &places, RNode::Step(1)),
            GNode::Item(RNode::Step(1))
        );
        assert_eq!(
            shown_as(&groups, &places, RNode::Step(2)),
            GNode::Item(RNode::Step(2))
        );
        assert_eq!(
            members(&groups, &places, 1),
            vec![RNode::Step(0), RNode::Step(1)]
        );
        assert_eq!(ancestors(&groups, Some(3)), vec![3, 2, 1]);
    }
}

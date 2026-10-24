//! What the graph is a view of: a recipe's steps and outputs as nodes, the names each uses or
//! makes as ports, and the wires those names make.

use std::collections::{HashMap, HashSet};

use clockwork_core::{
    recipe::{Access, NameKind, Output, Recipe, Seg, Step},
    recipe_schema::{self, VariantInfo},
};
use clockwork_recipe::{edit, text::Doc};

/// A node: a step or an output, by its place in the recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RNode {
    Step(usize),
    Output(usize),
}

/// A name a node uses or makes (see [`clockwork_core::recipe::Port`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedPort {
    pub(super) path: Vec<Seg>,
    pub(super) name: String,
    pub(super) kind: NameKind,
    pub(super) access: Access,
}

impl OwnedPort {
    /// What the input or output is called on the node.
    pub(super) fn label(&self) -> String {
        match self.path.as_slice() {
            [Seg::Field("segments"), Seg::Key(k)] => format!("segment {k:#04x}"),
            [Seg::Field("grids"), Seg::Index(i), ..] => format!("grid {i}"),
            [Seg::Field("frames"), _, Seg::Index(j)] => format!("frame {j}"),
            [Seg::Field("layers"), Seg::Index(i), Seg::Field("from")] => format!("layer {i}"),
            [Seg::Field("layers"), Seg::Index(i), .., Seg::Index(j)] => {
                format!("layer {i} frame {j}")
            }
            [Seg::Field(outer), Seg::Field(inner), ..] if *inner != "from" => (*inner).into(),
            [Seg::Field(field), ..] => (*field).into(),
            _ => "?".into(),
        }
    }
}

pub(super) fn ports_of(recipe: &Recipe, node: RNode) -> Vec<OwnedPort> {
    let ports = match node {
        RNode::Step(i) => recipe.steps.get(i).map(Step::ports),
        RNode::Output(i) => recipe.outputs.get(i).map(Output::ports),
    };
    ports
        .unwrap_or_default()
        .into_iter()
        .map(|p| OwnedPort {
            path: p.path,
            name: p.name.to_string(),
            kind: p.kind,
            access: p.access,
        })
        .collect()
}

/// The ports a node shows as inputs (those it reads), by port index.
pub(super) fn inputs_of(ports: &[OwnedPort]) -> Vec<usize> {
    (0..ports.len())
        .filter(|&i| ports[i].access.reads())
        .collect()
}

/// The ports a node shows as outputs (those it makes or changes), by port index.
pub(super) fn outputs_of(ports: &[OwnedPort]) -> Vec<usize> {
    (0..ports.len())
        .filter(|&i| ports[i].access.writes())
        .collect()
}

/// Which step makes what each input reads: wires from (node, port) to (node, port).
#[derive(Default, Debug, PartialEq)]
pub(super) struct Resolution {
    pub(super) wires: Vec<((RNode, usize), (RNode, usize))>,
    /// Inputs that need something no earlier step makes.
    pub(super) missing: HashSet<(RNode, usize)>,
    /// What feeds each wired input, so drawing a pin doesn't search every wire.
    pub(super) source: HashMap<(RNode, usize), (RNode, usize)>,
    /// The nodes wired into each node, and those each node is wired into.
    pub(super) upstream: HashMap<RNode, Vec<RNode>>,
    pub(super) downstream: HashMap<RNode, Vec<RNode>>,
}

pub(super) fn resolve(recipe: &Recipe, ports: &HashMap<RNode, Vec<OwnedPort>>) -> Resolution {
    let mut resolution = Resolution::default();
    let mut latest: HashMap<(NameKind, &str), (RNode, usize)> = HashMap::new();
    let nodes = (0..recipe.steps.len())
        .map(RNode::Step)
        .chain((0..recipe.outputs.len()).map(RNode::Output));
    for node in nodes {
        let node_ports = &ports[&node];
        for (i, port) in node_ports.iter().enumerate() {
            if !port.access.reads() {
                continue;
            }
            match latest.get(&(port.kind, port.name.as_str())) {
                Some(from) => {
                    resolution.wires.push((*from, (node, i)));
                    resolution.source.insert((node, i), *from);
                    resolution.upstream.entry(node).or_default().push(from.0);
                    resolution.downstream.entry(from.0).or_default().push(node);
                }
                None if port.access != Access::Grow => {
                    resolution.missing.insert((node, i));
                }
                None => {}
            }
        }
        for (i, port) in node_ports.iter().enumerate() {
            if port.access.writes() {
                latest.insert((port.kind, port.name.as_str()), (node, i));
            }
        }
    }
    resolution
}

pub(super) fn node_named(text: &str) -> Result<RNode, String> {
    Ok(match text.parse::<edit::Item>()? {
        edit::Item::Step(i) => RNode::Step(i),
        edit::Item::Output(i) => RNode::Output(i),
    })
}

/// The same node, as the shared edits name it.
pub(super) fn item(node: RNode) -> edit::Item {
    match node {
        RNode::Step(i) => edit::Item::Step(i),
        RNode::Output(i) => edit::Item::Output(i),
    }
}

/// The column a node goes in when its group lays it out.
pub(super) fn part_column(node: RNode, depth: &HashMap<RNode, usize>) -> usize {
    match node {
        RNode::Step(_) => depth.get(&node).copied().unwrap_or(0),
        RNode::Output(_) => usize::MAX / 2,
    }
}

/// Steps first, then outputs, each in recipe order.
pub(super) fn order(node: RNode) -> (u8, usize) {
    match node {
        RNode::Step(i) => (0, i),
        RNode::Output(i) => (1, i),
    }
}

pub(super) fn parse(text: &str) -> Result<(Doc, Recipe), String> {
    let doc = Doc::parse(text).map_err(|e| e.to_string())?;
    let recipe: Recipe = ron::from_str(text).map_err(|e| e.to_string())?;
    Ok((doc, recipe))
}

pub(super) fn all_ports(recipe: &Recipe) -> HashMap<RNode, Vec<OwnedPort>> {
    (0..recipe.steps.len())
        .map(RNode::Step)
        .chain((0..recipe.outputs.len()).map(RNode::Output))
        .map(|node| (node, ports_of(recipe, node)))
        .collect()
}

/// Each step's column: one past the steps whose output it reads, and level with a step whose
/// output it only changes (a chain of steps adding to one model stays in one column).
pub(super) fn depths(
    recipe: &Recipe,
    resolution: &Resolution,
    ports: &HashMap<RNode, Vec<OwnedPort>>,
) -> HashMap<RNode, usize> {
    let mut depth: HashMap<RNode, usize> = HashMap::new();
    for i in 0..recipe.steps.len() {
        let node = RNode::Step(i);
        let d = resolution
            .wires
            .iter()
            .filter(|(_, (to, _))| *to == node)
            .filter_map(|((from, _), (_, port))| {
                let updates = ports[&node][*port].access != Access::Read;
                depth.get(from).map(|d| if updates { *d } else { d + 1 })
            })
            .max()
            .unwrap_or(0);
        depth.insert(node, d);
    }
    depth
}

/// A key for a node's widgets.
pub(super) fn node_key(node: RNode) -> String {
    format!("{node:?}")
}

pub(super) fn variant_info(node: RNode, name: &str) -> Option<&'static VariantInfo> {
    match node {
        RNode::Step(_) => recipe_schema::step(name),
        RNode::Output(_) => recipe_schema::output(name),
    }
}

/// `node`, everything it is made from and everything made from it.
pub(super) fn lineage(resolution: &Resolution, node: RNode) -> HashSet<RNode> {
    let mut set = HashSet::from([node]);
    for links in [&resolution.upstream, &resolution.downstream] {
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            for next in links.get(&at).into_iter().flatten() {
                if set.insert(*next) {
                    stack.push(*next);
                }
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_s_layers_and_frames_are_labelled_by_number() {
        let recipe: Recipe = ron::from_str(
            r#"(steps: [], outputs: [Image(id: "a", from: "map",
                frames: Some((images: ["map", "map_b"], ticks: [8], fps: 60.0)),
                layers: [(from: "sky", frames: Some((images: ["sky_b"], ticks: [8], fps: 60.0)))])])"#,
        )
        .unwrap();
        let labels: Vec<String> = ports_of(&recipe, RNode::Output(0))
            .iter()
            .map(OwnedPort::label)
            .collect();
        assert_eq!(
            labels,
            ["from", "frame 0", "frame 1", "layer 0", "layer 0 frame 0"]
        );
    }
}

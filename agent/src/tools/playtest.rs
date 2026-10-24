//! Play-testing a plugin's scene without a window: its nodes, areas and behaviors are set up as
//! the Workshop places a scene output, a character is played through it from a script of
//! inputs (`clockwork_runtime::play::playtest`), and what happened comes back as text.

use std::fmt::Write;

use bevy::math::{Affine3A, Vec2, Vec3};
use clockwork_core::{
    asset::{self, AssetSource, Layer, OutputRef, Slot},
    behavior::{Behavior, NodeBehavior},
    character::Character,
    character2d::{Character2d, Direction4},
    game::{self, Rules},
    item::ItemDef,
    parts::{self, Stats},
    place::SceneTemplate,
    recipe::Output,
    ui::Words,
};
use clockwork_recipe::{Extracted, IMAGE_PIXELS_PER_METER};
use clockwork_runtime::{
    billboard::Body25d,
    controller::{Body, Ground, World},
    inventory::Inventory,
    play::{
        Area, Avatar, BillboardBody, Level, ModelBody, Play, SpriteBody, check,
        entities::{Entities, Placed},
        fighter::Fighter,
        playtest::{self, Step},
    },
    terrain::WorldTerrain,
    topdown::Body2d,
};

/// The longest script, in seconds of play.
pub const MOST_SECONDS: f32 = 600.0;

/// Where the played character starts.
pub enum Start {
    /// Where the node of this name stands.
    Node(String),
    At(Vec3),
}

/// A node to give behaviors to: one of the scene's, by name, or a new one at `at`.
pub struct Extra {
    pub name: String,
    pub at: Option<Vec3>,
    /// Each a behavior asset, `"plugin/output"`, or a behavior written in RON.
    pub behaviors: Vec<String>,
}

/// What to play.
#[derive(Default)]
pub struct Request {
    /// The scene output; the plugin's first without it.
    pub scene: Option<String>,
    /// The played character, `"plugin/output"` or an output of this plugin; without it, the
    /// first playable character the scene shows, else the plugin's first.
    pub character: Option<String>,
    pub start: Option<Start>,
    /// A rules or game asset, `"plugin/output"`, or rules written in RON; without it, the
    /// plugin's own game or rules asset, else the built-in rules.
    pub rules: Option<String>,
    pub act: Option<u8>,
    pub nodes: Vec<Extra>,
    pub steps: Vec<Step>,
}

/// The played plugin and where everything else it names is found.
pub struct Content<'a> {
    pub plugin: &'a str,
    /// Its recipe's outputs, and what they made.
    pub outputs: &'a [Output],
    pub extracted: &'a Extracted,
    /// Its outputs and every installed plugin's, as assets.
    pub assets: &'a dyn AssetSource,
    /// Another plugin's extraction, by name.
    pub other: &'a dyn Fn(&str) -> Result<Extracted, String>,
}

/// A playable character.
enum Played {
    Sprite(Character2d),
    Model(Character),
}

impl Played {
    fn of(extracted: &Extracted, output: &str) -> Option<Self> {
        if let Some(c) = extracted.sprite(output).and_then(|s| s.character.clone()) {
            return Some(Played::Sprite(c));
        }
        extracted
            .model(output)
            .and_then(|m| m.character.clone())
            .map(Played::Model)
    }

    fn stats(&self) -> Option<&Slot<Stats>> {
        match self {
            Played::Sprite(c) => c.stats.as_ref(),
            Played::Model(c) => c.stats.as_ref(),
        }
    }

    fn inventory(&self) -> Option<&OutputRef> {
        match self {
            Played::Sprite(c) => c.inventory.as_ref(),
            Played::Model(c) => c.inventory.as_ref(),
        }
    }
}

/// Plays the request's script in the scene and reports what happened.
pub fn run(content: &Content, request: &Request) -> Result<String, String> {
    let seconds: f32 = request.steps.iter().map(|s| s.seconds.max(0.0)).sum();
    if request.steps.is_empty() {
        return Err("The script has no steps".into());
    }
    if seconds > MOST_SECONDS {
        return Err(format!(
            "The script lasts {seconds:.0} s; at most {MOST_SECONDS:.0} s"
        ));
    }
    let x = content.extracted;
    let (scene_id, template) = match &request.scene {
        Some(id) => x
            .scenes
            .iter()
            .find(|(s, _)| s == id)
            .ok_or_else(|| format!("No scene output \"{id}\"; it has: {}", scene_ids(x)))?,
        None => x.scenes.first().ok_or(
            "The plugin has no Scene output (a Scene output made of Places lists is what is \
             played)",
        )?,
    };
    let mut problems: Vec<String> = Vec::new();
    let (mut placed, mut names) = nodes(content, template, &mut problems);
    for extra in &request.nodes {
        add(content, extra, &mut placed, &mut names, &mut problems)?;
    }

    // The played character, and where it stands in the scene if it is one of its nodes.
    let (who, played) = character(content, template, request.character.as_deref())?;
    let in_scene = template
        .nodes
        .iter()
        .find(|n| who.plugin == content.plugin && n.item.as_deref() == Some(&who.output))
        .map(|n| Vec3::from(n.position));

    // What there is to stand on: images with surface maps, models with collision.
    let mut areas = Vec::new();
    let mut terrains = Vec::new();
    for node in &template.nodes {
        let Some(item) = &node.item else { continue };
        let at = Vec3::from(node.position);
        if let Some(map) = x.surfaces_for(item)
            && let Some((_, image)) = x.images.iter().find(|(id, _)| id == item)
        {
            areas.push(Area {
                to_local: Affine3A::from_translation(at).inverse(),
                size: Vec2::new(image.width as f32, image.height as f32) / IMAGE_PIXELS_PER_METER,
                height: at.y,
                map,
            });
        }
        if let Some(model) = x.model(item)
            && let Some(terrain) = &model.terrain
        {
            let to_world = Affine3A::from_translation(at)
                * Affine3A::from_scale(Vec3::splat(model.meters_per_unit));
            terrains.push(WorldTerrain::new(terrain, to_world));
        }
    }
    let level = Level {
        areas,
        terrains: terrains.iter().collect(),
    };
    if level.areas.is_empty() && level.terrains.is_empty() {
        problems.push(
            "Nothing in the scene can be stood on: no image with a surface map, no model with \
             collision"
                .into(),
        );
    }

    let place = |name: &str| names.iter().find(|(n, _)| n == name).map(|(_, at)| *at);
    let spawn = terrains.iter().find_map(WorldTerrain::spawn);
    let (start, facing) = match &request.start {
        Some(Start::Node(name)) => (
            place(name).ok_or_else(|| format!("No node called \"{name}\" to start at"))?,
            None,
        ),
        Some(Start::At(at)) => (*at, None),
        None => match (in_scene, spawn) {
            (Some(at), _) => (at, None),
            (None, Some((at, yaw))) => (at, Some(yaw)),
            (None, None) => (
                level
                    .areas
                    .first()
                    .map_or(Vec3::ZERO, |a| a.to_local.inverse().translation.into()),
                None,
            ),
        },
    };
    if matches!(level.ground_at(start.x, start.y, start.z), Ground::Hole) {
        problems.push(format!(
            "The start ({:.2}, {:.2}, {:.2}) is not on any floor",
            start.x, start.y, start.z
        ));
    }

    // Its body, stats and items, as Play gives them.
    let stats = resolve_stats(content, played.stats())?;
    let items = items(content, &who, played.inventory())?;
    let (avatar, moves, kind) = match played {
        Played::Sprite(character) if !level.terrains.is_empty() => (
            Avatar::Billboard(Box::new(BillboardBody {
                body: Body25d::new(start, Direction4::South),
                character: character.clone(),
            })),
            character.moves(),
            "a sprite standing in a 3D area",
        ),
        Played::Sprite(character) => (
            Avatar::TopDown(Box::new(SpriteBody {
                body: Body2d::new(start, Direction4::South),
                start,
                character: character.clone(),
            })),
            character.moves(),
            "a sprite seen from above",
        ),
        Played::Model(character) => (
            Avatar::Model(Box::new(ModelBody {
                body: Body::new(start, facing.unwrap_or_default()),
                character: character.clone(),
                jump_buffer: 0.0,
                lean_bone: None,
                last_action: None,
                camera: None,
            })),
            character.moves(),
            "a 3D character",
        ),
    };
    let mut fighter = Fighter::new(items.map(Inventory::new), moves);
    fighter.plugin = Some(who.plugin.clone());
    fighter.hitboxes = false;
    let (rules, rules_from) = rules(content, request.rules.as_deref())?;
    let entities = Entities::build(placed);
    let all: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    problems.extend(check::scene_names(
        &all,
        &entities,
        rules.goal.as_ref(),
        None,
    ));
    let goal = rules
        .goal
        .as_ref()
        .map_or("none".to_string(), |g| format!("{g:?}"));
    let mut play = Play::new(1, avatar, fighter, stats, rules, entities);
    play.set_act(request.act);

    let places = |name: &str| place(name);
    let report = playtest::run(
        &mut play,
        &level,
        &places,
        &Words::default(),
        &request.steps,
    );

    let mut out = String::new();
    let _ = writeln!(
        out,
        "Scene \"{scene_id}\" of {}: {} nodes, {} with behaviors, {} areas, {} 3D areas.",
        content.plugin,
        names.len(),
        play.entities.nodes.len(),
        level.areas.len(),
        level.terrains.len()
    );
    let _ = writeln!(
        out,
        "Playing {who} ({kind}) from ({:.2}, {:.2}, {:.2}); rules {rules_from}, goal {goal}.",
        start.x, start.y, start.z
    );
    if !problems.is_empty() {
        let _ = writeln!(out, "Content problems:");
        for p in &problems {
            let _ = writeln!(out, "- {p}");
        }
    }
    let _ = write!(out, "{report}");
    Ok(out)
}

fn scene_ids(x: &Extracted) -> String {
    let ids: Vec<&str> = x.scenes.iter().map(|(id, _)| id.as_str()).collect();
    if ids.is_empty() {
        "no scenes".into()
    } else {
        ids.join(", ")
    }
}

/// The scene's nodes as Play places them (ids from 2: 1 is the played character), and every
/// node's name and position. A placement's door and behavior asset become its behaviors.
fn nodes(
    content: &Content,
    template: &SceneTemplate,
    problems: &mut Vec<String>,
) -> (Vec<Placed>, Vec<(String, Vec3)>) {
    let mut placed = Vec::new();
    let mut names = Vec::new();
    for (i, node) in template.nodes.iter().enumerate() {
        let at = Vec3::from(node.position);
        let mut behaviors = Vec::new();
        if let Some(door) = &node.door {
            behaviors.push(NodeBehavior::new(Behavior::Door {
                radius: door.radius,
                to: door.to.clone(),
                fade: door.fade,
            }));
        }
        if let Some(name) = &node.behavior {
            let at = OutputRef::new(content.plugin, name);
            match behavior_asset(content, &at) {
                Ok(behavior) => behaviors.push(NodeBehavior {
                    from: Some(at),
                    ..NodeBehavior::new(behavior)
                }),
                Err(e) => problems.push(format!("the behavior of \"{}\": {e}", node.name)),
            }
        }
        placed.push(Placed {
            id: i as u64 + 2,
            name: node.name.clone(),
            at,
            behaviors,
            ..Default::default()
        });
        names.push((node.name.clone(), at));
    }
    (placed, names)
}

fn behavior_asset(content: &Content, at: &OutputRef) -> Result<Behavior, String> {
    asset::resolve(
        &[(Layer::Asset, Some(&Slot::Ref(at.clone())))],
        content.assets,
        "behavior",
        asset::behavior_of,
    )
    .map_err(|e| e.to_string())?
    .map(|r| r.value)
    .ok_or_else(|| format!("{at} holds no behavior"))
}

/// A behavior written in RON, or the behavior asset a `"plugin/output"` names.
fn parse_behavior(content: &Content, text: &str) -> Result<NodeBehavior, String> {
    let text = text.trim();
    if !text.contains('(')
        && let Some(at) = OutputRef::parse(text)
    {
        return Ok(NodeBehavior {
            from: Some(at.clone()),
            ..NodeBehavior::new(behavior_asset(content, &at)?)
        });
    }
    // A whole node behavior (with `when` or `height`), or just the behavior.
    ron::from_str::<NodeBehavior>(text)
        .or_else(|_| ron::from_str::<Behavior>(text).map(NodeBehavior::new))
        .map_err(|e| format!("\"{text}\" is neither a behavior nor \"plugin/output\": {e}"))
}

/// Gives a scene node more behaviors, or adds a node.
fn add(
    content: &Content,
    extra: &Extra,
    placed: &mut Vec<Placed>,
    names: &mut Vec<(String, Vec3)>,
    problems: &mut Vec<String>,
) -> Result<(), String> {
    let mut behaviors = Vec::new();
    for text in &extra.behaviors {
        match parse_behavior(content, text) {
            Ok(b) => behaviors.push(b),
            Err(e) => problems.push(format!("\"{}\": {e}", extra.name)),
        }
    }
    if let Some(node) = placed.iter_mut().find(|n| n.name == extra.name) {
        if let Some(at) = extra.at {
            node.at = at;
            if let Some((_, place)) = names.iter_mut().find(|(n, _)| *n == extra.name) {
                *place = at;
            }
        }
        node.behaviors.extend(behaviors);
        return Ok(());
    }
    let at = extra.at.ok_or_else(|| {
        format!(
            "The scene has no node \"{}\": a new node needs `at`",
            extra.name
        )
    })?;
    placed.push(Placed {
        id: placed.len() as u64 + 2,
        name: extra.name.clone(),
        at,
        behaviors,
        ..Default::default()
    });
    names.push((extra.name.clone(), at));
    Ok(())
}

/// The played character: the one asked for, else the first playable one the scene shows, else
/// the plugin's first.
fn character(
    content: &Content,
    template: &SceneTemplate,
    asked: Option<&str>,
) -> Result<(OutputRef, Played), String> {
    let x = content.extracted;
    if let Some(asked) = asked {
        let at = OutputRef::parse(asked).unwrap_or_else(|| OutputRef::new(content.plugin, asked));
        let other;
        let from = if at.plugin == content.plugin {
            x
        } else {
            other = (content.other)(&at.plugin)?;
            &other
        };
        let played = Played::of(from, &at.output).ok_or_else(|| {
            format!("{at} is not a playable character (a sprite or model with `character`)")
        })?;
        return Ok((at, played));
    }
    let shown = template.nodes.iter().filter_map(|n| n.item.as_deref());
    let outputs = x
        .sprites
        .iter()
        .map(|s| s.id.as_str())
        .chain(x.models.iter().map(|m| m.id.as_str()));
    shown
        .chain(outputs)
        .find_map(|id| Some((OutputRef::new(content.plugin, id), Played::of(x, id)?)))
        .ok_or_else(|| {
            "The plugin has no playable character: name one with `character`, \"plugin/output\""
                .to_string()
        })
}

fn resolve_stats(content: &Content, own: Option<&Slot<Stats>>) -> Result<Stats, String> {
    let builtin = Slot::Ref(parts::builtin_stats());
    asset::resolve(
        &[(Layer::Asset, own), (Layer::Default, Some(&builtin))],
        content.assets,
        "stats",
        asset::stats_of,
    )
    .map_err(|e| format!("The character's stats: {e}"))
    .map(|r| r.map(|r| r.value).unwrap_or_default())
}

/// The items the character carries: its own list, else its plugin's first.
fn items(
    content: &Content,
    who: &OutputRef,
    own: Option<&OutputRef>,
) -> Result<Option<Vec<ItemDef>>, String> {
    let lookup = |plugin: &str, id: Option<&str>| -> Result<Option<Vec<ItemDef>>, String> {
        let pick = |x: &Extracted| {
            x.items
                .iter()
                .find(|(i, _)| id.is_none_or(|id| i == id))
                .map(|(_, items)| items.clone())
        };
        if plugin == content.plugin {
            Ok(pick(content.extracted))
        } else {
            Ok(pick(&(content.other)(plugin)?))
        }
    };
    match own {
        Some(at) => lookup(&at.plugin, Some(&at.output))?
            .map(Some)
            .ok_or_else(|| format!("The character's items: {at} isn't a list of items")),
        None => lookup(&who.plugin, None),
    }
}

/// The rules played by, and where they came from.
fn rules(content: &Content, asked: Option<&str>) -> Result<(Rules, String), String> {
    let from_asset = |at: &OutputRef| -> Result<Rules, String> {
        // A game asset names its rules.
        let outputs = content.assets.outputs(at);
        let named = outputs.iter().find_map(|o| match asset::game_of(o) {
            Some(Slot::Inline(game)) => game.rules.as_deref().and_then(OutputRef::parse),
            _ => None,
        });
        let at = named.unwrap_or_else(|| at.clone());
        asset::resolve(
            &[(Layer::Game, Some(&Slot::Ref(at.clone())))],
            content.assets,
            "rules",
            asset::rules_of,
        )
        .map_err(|e| format!("The rules: {e}"))?
        .map(|r| r.value)
        .ok_or_else(|| format!("{at} holds no rules"))
    };
    if let Some(text) = asked {
        let text = text.trim();
        if !text.contains('(')
            && let Some(at) = OutputRef::parse(text)
        {
            return Ok((from_asset(&at)?, at.to_string()));
        }
        let rules: Rules = ron::from_str(text)
            .map_err(|e| format!("The rules are neither \"plugin/output\" nor RON: {e}"))?;
        return Ok((rules, "as written".into()));
    }
    // The plugin's own game, else its own rules.
    for kind in ["Game", "Rules"] {
        if let Some(output) = content.outputs.iter().find(|o| o.name() == kind) {
            let at = OutputRef::new(content.plugin, output.id());
            return Ok((from_asset(&at)?, at.to_string()));
        }
    }
    let builtin = game::builtin_rules();
    Ok((from_asset(&builtin)?, builtin.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clockwork_core::{
        image::RgbaImage,
        place::{Door, SceneNode},
        recipe::Recipe,
        surface::{SurfaceKind, SurfaceMap, SurfaceProperty},
    };
    use clockwork_recipe::{ExtractedSprite, assets::OwnedAssets};
    use clockwork_runtime::play::playtest::Hold;

    fn node(name: &str, item: Option<&str>, at: [f32; 3]) -> SceneNode {
        SceneNode {
            name: name.into(),
            group: None,
            item: item.map(str::to_string),
            position: at,
            door: None,
            animation: None,
            behavior: None,
        }
    }

    /// A 20 m room of open floor, a hero, a gem (a behavior asset), a door to the exit.
    fn plugin() -> (Recipe, Extracted) {
        let recipe: Recipe = ron::from_str(
            r#"(steps: [], outputs: [
                Behavior(id: "gem", behavior: Pickup(item: "gem", count: 1, radius: 0.5)),
            ])"#,
        )
        .unwrap();
        let character = ron::from_str(
            r#"(
                movement: (speed: 4.0, radius: 0.3),
                animations: (
                    idle: (north: (name: "idle"), east: (name: "idle"), south: (name: "idle"), west: (name: "idle")),
                    walk: (north: (name: "walk"), east: (name: "walk"), south: (name: "walk"), west: (name: "walk")),
                ),
            )"#,
        )
        .unwrap();
        let mut gem = node("Gem", None, [3.0, 0.0, 0.0]);
        gem.behavior = Some("gem".into());
        let mut door = node("Door", None, [6.0, 0.0, 0.0]);
        door.door = Some(Door {
            to: "Exit".into(),
            radius: 0.5,
            fade: 0.4,
        });
        let extracted = Extracted {
            images: vec![(
                "room".into(),
                RgbaImage {
                    width: 320,
                    height: 320,
                    pixels: vec![0; 320 * 320 * 4],
                },
            )],
            surfaces: vec![(
                "room_surfaces".into(),
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
                },
            )],
            surface_images: vec![("room_surfaces".into(), "room".into())],
            sprites: vec![ExtractedSprite {
                id: "hero".into(),
                sprite: Default::default(),
                pixels_per_meter: 16.0,
                character: Some(character),
            }],
            items: vec![(
                "items".into(),
                ron::from_str(r#"[(id: "gem", name: "Gem", description: "", category: Quest)]"#)
                    .unwrap(),
            )],
            scenes: vec![(
                "rooms".into(),
                SceneTemplate {
                    nodes: vec![
                        node("Room", Some("room"), [0.0, 0.0, 0.0]),
                        node("Hero", Some("hero"), [-5.0, 0.0, 0.0]),
                        gem,
                        door,
                        node("Exit", None, [-6.0, 0.0, 6.0]),
                    ],
                },
            )],
            ..Default::default()
        };
        (recipe, extracted)
    }

    fn play(request: &Request) -> Result<String, String> {
        let (recipe, extracted) = plugin();
        let owned = OwnedAssets::default();
        let assets = owned.with_recipe("test", &recipe);
        let none = |name: &str| Err(format!("no plugin {name}"));
        run(
            &Content {
                plugin: "test",
                outputs: &recipe.outputs,
                extracted: &extracted,
                assets: &assets,
                other: &none,
            },
            request,
        )
    }

    fn toward(name: &str) -> Step {
        Step {
            hold: Hold::Toward(name.into()),
            seconds: 10.0,
            walk: false,
            buttons: vec![],
        }
    }

    #[test]
    fn a_scene_is_played_through_its_door_to_an_added_goal() {
        let request = Request {
            rules: Some(r#"(goal: Some(Reach("Goal")))"#.into()),
            nodes: vec![Extra {
                name: "Goal".into(),
                at: Some(Vec3::new(-2.0, 0.0, 6.0)),
                behaviors: vec![],
            }],
            steps: vec![toward("Gem"), toward("Door"), toward("Goal")],
            ..Default::default()
        };
        let text = play(&request).unwrap();
        assert!(
            text.contains("Playing test/hero (a sprite seen from above)"),
            "{text}"
        );
        assert!(text.contains("from (-5.00, 0.00, 0.00)"), "{text}");
        assert!(text.contains("Got Gem x1"), "{text}");
        assert!(text.contains("arrived at \"Exit\""), "{text}");
        assert!(text.contains("Won at"), "{text}");
        assert!(text.contains("carrying gem x1"), "{text}");
        assert!(!text.contains("Content problems"), "{text}");
        assert_eq!(play(&request).unwrap(), text, "the same every time");
    }

    #[test]
    fn what_is_missing_is_said() {
        let wrong = Request {
            scene: Some("caves".into()),
            steps: vec![toward("Gem")],
            ..Default::default()
        };
        assert!(play(&wrong).unwrap_err().contains("it has: rooms"));
        let lost = Request {
            start: Some(Start::At(Vec3::new(50.0, 0.0, 0.0))),
            nodes: vec![Extra {
                name: "Gem".into(),
                at: None,
                behaviors: vec!["Door(radius: 1.0, to: \"Attic\", fade: 0.5)".into()],
            }],
            steps: vec![Step {
                hold: Hold::Still,
                seconds: 0.5,
                walk: false,
                buttons: vec![],
            }],
            ..Default::default()
        };
        let text = play(&lost).unwrap();
        assert!(text.contains("is not on any floor"), "{text}");
        assert!(text.contains("leads to \"Attic\""), "{text}");
        assert!(text.contains("Never reached (2)"), "{text}");
    }
}

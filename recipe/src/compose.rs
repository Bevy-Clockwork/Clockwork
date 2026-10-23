//! Running composition plugins: characters made from other plugins' outputs, with abilities
//! learned from any plugin.
//!
//! The result is an ordinary [`Extracted`], so everything that shows or plays plugin outputs works
//! with compositions unchanged. Sprites that learned moves show (held items, projectiles, icons)
//! are copied in from their plugins under `"<plugin>/<output>"` names, and the moves are rewritten
//! to use those names.

use clockwork_core::ability::Ability;
use clockwork_core::action::{Moves, Projectile};
use clockwork_core::character2d::Directional;
use clockwork_core::compose::{ComposedCharacter, Composition, OutputRef};
use clockwork_core::item::{Icon, ItemDef};

use crate::{Extracted, ExtractedModel, ExtractedSprite};

/// The plugins a composition uses.
pub fn dependencies(composition: &Composition) -> Vec<String> {
    let mut plugins: Vec<String> = composition
        .characters
        .iter()
        .flat_map(|c| {
            std::iter::once(c.base.plugin.clone())
                .chain(c.abilities.iter().map(|a| a.ability.plugin.clone()))
        })
        .collect();
    plugins.sort();
    plugins.dedup();
    plugins
}

/// Builds a composition plugin's outputs. `source` gives the extraction of another plugin.
pub fn compose<'a>(
    composition: &Composition,
    source: &dyn Fn(&str) -> Option<&'a Extracted>,
) -> Result<Extracted, String> {
    let mut out = Extracted::default();
    let mut items: Vec<ItemDef> = Vec::new();
    for character in &composition.characters {
        compose_character(character, source, &mut out, &mut items)?;
    }
    if !items.is_empty() {
        out.items.push(("items".into(), items));
    }
    Ok(out)
}

fn compose_character<'a>(
    composed: &ComposedCharacter,
    source: &dyn Fn(&str) -> Option<&'a Extracted>,
    out: &mut Extracted,
    items: &mut Vec<ItemDef>,
) -> Result<(), String> {
    let base_plugin = &composed.base.plugin;
    let base = source(base_plugin).ok_or_else(|| missing(base_plugin))?;
    let name = &composed.id;

    // The base character's own moves and items, imported from its plugin like learned ones.
    let (mut moves, idle) = if let Some(s) = base.sprite(&composed.base.output) {
        let character = s
            .character
            .as_ref()
            .ok_or_else(|| not_character(&composed.base))?;
        (character.moves(), character.animations.idle.clone())
    } else if let Some(m) = base.model(&composed.base.output) {
        let character = m
            .character
            .as_ref()
            .ok_or_else(|| not_character(&composed.base))?;
        let idle = character
            .animations
            .idle
            .first()
            .map(|n| Directional::uniform(n))
            .unwrap_or_else(|| Directional::uniform(""));
        (character.moves(), idle)
    } else {
        return Err(format!(
            "\"{name}\": plugin \"{base_plugin}\" has no output \"{}\"",
            composed.base.output
        ));
    };
    import(&mut moves, items, base_plugin, base, out, base.items_all());

    for learned in &composed.abilities {
        let plugin = &learned.ability.plugin;
        let extracted = source(plugin).ok_or_else(|| missing(plugin))?;
        let ability: &Ability = extracted
            .abilities
            .iter()
            .find(|a| a.id == learned.ability.output)
            .ok_or_else(|| {
                format!(
                    "\"{name}\": plugin \"{plugin}\" has no ability \"{}\"",
                    learned.ability.output
                )
            })?;
        let mut learned_moves = ability.moves_for(&learned.roles, learned.use_defaults, &idle);
        import(
            &mut learned_moves,
            items,
            plugin,
            extracted,
            out,
            ability.items.clone(),
        );
        moves.merge(learned_moves);
    }

    let tuning = composed.tuning;
    if let Some(s) = base.sprite(&composed.base.output) {
        let mut character = s
            .character
            .clone()
            .expect("checked above")
            .with_moves(moves);
        character.movement.speed *= tuning.speed;
        out.sprites.push(ExtractedSprite {
            id: name.clone(),
            sprite: s.sprite.clone(),
            pixels_per_meter: s.pixels_per_meter,
            character: Some(character),
        });
    } else if let Some(m) = base.model(&composed.base.output) {
        let mut character = m
            .character
            .clone()
            .expect("checked above")
            .with_moves(moves);
        character.movement.walk_speed *= tuning.speed;
        character.movement.run_speed *= tuning.speed;
        // Height grows with the square of the take-off speed.
        let jump = tuning.jump.max(0.0).sqrt();
        for j in &mut character.movement.jumps {
            j.speed *= jump;
        }
        if composed.hand.is_some() {
            character.hand = composed.hand.clone();
        }
        out.models.push(ExtractedModel {
            id: name.clone(),
            model: m.model.clone(),
            meters_per_unit: m.meters_per_unit,
            character: Some(character),
            terrain: None,
        });
    }
    Ok(())
}

impl Extracted {
    /// Every item of every `Items` output.
    fn items_all(&self) -> Vec<ItemDef> {
        self.items.iter().flat_map(|(_, i)| i.clone()).collect()
    }
}

/// Makes `moves` and `new_items` from `plugin` usable in the composition: the sprites they show
/// are copied in under `"<plugin>/<id>"` and every reference is renamed. Items already there (by
/// id) are kept.
fn import(
    moves: &mut Moves,
    items: &mut Vec<ItemDef>,
    plugin: &str,
    from: &Extracted,
    out: &mut Extracted,
    new_items: Vec<ItemDef>,
) {
    let mut rename = |sprite: &mut String| {
        let id = format!("{plugin}/{sprite}");
        if !out.sprites.iter().any(|s| s.id == id)
            && let Some(s) = from.sprite(sprite)
        {
            out.sprites.push(ExtractedSprite {
                id: id.clone(),
                sprite: s.sprite.clone(),
                pixels_per_meter: s.pixels_per_meter,
                character: None,
            });
        }
        *sprite = id;
    };
    for action in &mut moves.actions {
        for attachment in &mut action.attachments {
            rename(&mut attachment.sprite);
        }
    }
    for Projectile { sprite, .. } in &mut moves.projectiles {
        if let Some(look) = sprite {
            rename(&mut look.sprite);
        }
    }
    for mut item in new_items {
        if items.iter().any(|i| i.id == item.id) {
            continue;
        }
        match &mut item.icon {
            Some(Icon::Frame { sprite, .. } | Icon::Animation { sprite, .. }) => rename(sprite),
            Some(Icon::Image(image)) => {
                let id = format!("{plugin}/{image}");
                if let Some((_, picture)) = from.images.iter().find(|(i, _)| i == image)
                    && !out.images.iter().any(|(i, _)| *i == id)
                {
                    out.images.push((id.clone(), picture.clone()));
                }
                *image = id;
            }
            None => {}
        }
        items.push(item);
    }
}

fn missing(plugin: &str) -> String {
    format!("plugin \"{plugin}\" is not installed or its game is not in the Library")
}

fn not_character(output: &OutputRef) -> String {
    format!(
        "\"{}\" of plugin \"{}\" is not a playable character",
        output.output, output.plugin
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use clockwork_core::ability::{Role, RoleBinding};
    use clockwork_core::action::{Action, Attachment, AttachmentFrames, Motion};
    use clockwork_core::character2d::Character2d;
    use clockwork_core::compose::{AbilityUse, Tuning};
    use clockwork_core::image::RgbaImage;
    use clockwork_core::item::Category;
    use clockwork_core::sprite::{Sprite, SpriteFrame};

    use super::*;

    fn character(actions: Vec<Action>) -> Character2d {
        let mut c: Character2d = ron::from_str(
            r#"(movement: (speed: 4.0, radius: 0.4), animations: (
                idle: (north: (name: "idle"), east: (name: "idle"), south: (name: "idle"), west: (name: "idle")),
                walk: (north: (name: "walk"), east: (name: "walk"), south: (name: "walk"), west: (name: "walk"))))"#,
        )
        .unwrap();
        c.actions = actions;
        c
    }

    fn sprite(id: &str, character: Option<Character2d>) -> ExtractedSprite {
        ExtractedSprite {
            id: id.into(),
            sprite: Sprite {
                frames: vec![SpriteFrame {
                    image: RgbaImage::new(1, 1),
                    origin: [0, 0],
                }],
                animations: vec![],
            },
            pixels_per_meter: 16.0,
            character,
        }
    }

    /// A plugin with a hero who swings a sword it draws from another sprite, as an ability.
    fn hero_plugin() -> Extracted {
        let slash = Action {
            id: "slash".into(),
            animation: Directional::uniform("slash"),
            duration: 0.2,
            motion: Motion::Rooted,
            hits: vec![],
            invulnerable: None,
            spawns: vec![],
            hold: None,
            cancel_from: None,
            attachments: vec![Attachment {
                sprite: "blade".into(),
                frames: AttachmentFrames::FollowFrame {
                    offset: 0,
                    from: None,
                    until: None,
                },
                behind: false,
                still: Some(0),
            }],
        };
        let item = ItemDef {
            id: "sword".into(),
            name: "Sword".into(),
            description: String::new(),
            category: Category::Weapon,
            action: Some("slash".into()),
            ammo: None,
            max: 1,
            start: 1,
            slot: None,
            icon: Some(Icon::Frame {
                sprite: "blade".into(),
                frame: 0,
                flip_x: false,
            }),
        };
        let mut ability_moves = Moves {
            actions: vec![slash.clone()],
            ..Default::default()
        };
        ability_moves.actions[0].animation = Directional::uniform("slash");
        Extracted {
            sprites: vec![
                sprite("hero", Some(character(vec![slash]))),
                sprite("blade", None),
            ],
            items: vec![("items".into(), vec![item.clone()])],
            abilities: vec![Ability {
                id: "sword".into(),
                name: "Sword".into(),
                description: String::new(),
                moves: ability_moves,
                items: vec![item],
                roles: vec![Role {
                    name: "slash".into(),
                    description: String::new(),
                    default: Directional::uniform("hero_slash"),
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn learned_abilities_bring_their_sprites_and_items() {
        let hero = hero_plugin();
        let other = Extracted {
            sprites: vec![sprite("rabbit", Some(character(vec![])))],
            ..Default::default()
        };
        let composition = Composition {
            characters: vec![ComposedCharacter {
                id: "armed_rabbit".into(),
                base: OutputRef {
                    plugin: "other".into(),
                    output: "rabbit".into(),
                },
                abilities: vec![AbilityUse {
                    ability: OutputRef {
                        plugin: "hero".into(),
                        output: "sword".into(),
                    },
                    roles: BTreeMap::from([(
                        "slash".to_string(),
                        RoleBinding::Animation("hop".into()),
                    )]),
                    use_defaults: false,
                }],
                tuning: Tuning {
                    speed: 1.5,
                    jump: 1.0,
                },
                hand: None,
            }],
        };
        let source = |p: &str| match p {
            "hero" => Some(&hero),
            "other" => Some(&other),
            _ => None,
        };
        let out = compose(&composition, &source).unwrap();
        let rabbit = out.sprite("armed_rabbit").unwrap();
        let character = rabbit.character.as_ref().unwrap();
        assert_eq!(character.movement.speed, 6.0);
        let slash = &character.actions[0];
        assert_eq!(slash.animation.north.name, "hop");
        assert_eq!(slash.attachments[0].sprite, "hero/blade");
        assert!(out.sprite("hero/blade").is_some());
        let (_, items) = &out.items[0];
        assert_eq!(
            items[0].icon,
            Some(Icon::Frame {
                sprite: "hero/blade".into(),
                frame: 0,
                flip_x: false
            })
        );
        assert_eq!(dependencies(&composition), vec!["hero", "other"]);
    }

    #[test]
    fn missing_plugins_and_abilities_are_reported() {
        let composition = Composition {
            characters: vec![ComposedCharacter {
                id: "x".into(),
                base: OutputRef {
                    plugin: "nope".into(),
                    output: "x".into(),
                },
                abilities: vec![],
                tuning: Tuning::default(),
                hand: None,
            }],
        };
        let err = compose(&composition, &|_| None).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}

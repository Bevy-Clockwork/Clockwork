//! Ability outputs: some of a character's actions, packaged for other characters to learn.

use clockwork_core::ability::{Ability, Role};
use clockwork_core::action::Moves;
use clockwork_core::character2d::Directional;
use clockwork_core::recipe::Output;

use crate::{Extracted, RecipeError};

/// Builds an `Ability` output from the character it names and the plugin's items.
pub(crate) fn build(output: &Output, extracted: &Extracted) -> Result<Ability, RecipeError> {
    let Output::Ability {
        id,
        name,
        description,
        from,
        actions,
        items,
        dodge,
    } = output
    else {
        unreachable!("only ability outputs are built here");
    };
    let fail = |message: String| RecipeError::Character {
        id: id.clone(),
        message,
    };
    let source = extracted
        .sprite(from)
        .and_then(|s| s.character.as_ref().map(|c| c.moves()))
        .or_else(|| {
            extracted
                .model(from)
                .and_then(|m| m.character.as_ref().map(|c| c.moves()))
        })
        .ok_or_else(|| fail(format!("\"{from}\" is not a character output")))?;

    // The listed actions, the actions they release into, and the dodge if asked for.
    let mut wanted: Vec<String> = actions.clone();
    if *dodge {
        let Some(dodge) = &source.dodge else {
            return Err(fail(format!("\"{from}\" has no dodge")));
        };
        wanted.push(dodge.clone());
    }
    let mut moves = Moves::default();
    while let Some(next) = wanted.pop() {
        if moves.action(&next).is_some() {
            continue;
        }
        let action = source
            .action(&next)
            .ok_or_else(|| fail(format!("\"{from}\" has no action \"{next}\"")))?;
        if let Some(release) = action.hold.as_ref().and_then(|h| h.release.as_ref()) {
            wanted.push(release.action.clone());
        }
        for spawn in &action.spawns {
            if moves.projectile(&spawn.projectile).is_none()
                && let Some(p) = source.projectile(&spawn.projectile)
            {
                moves.projectiles.push(p.clone());
            }
        }
        moves.actions.push(action.clone());
    }
    // Keep the source's order, which reads better in the editor.
    moves
        .actions
        .sort_by_key(|a| source.actions.iter().position(|s| s.id == a.id));
    if *dodge {
        moves.dodge = source.dodge.clone();
    }

    // Every pose becomes a role, with the source's animations as its default.
    let mut roles = Vec::new();
    let mut role = |name: String, directional: &mut Directional| {
        roles.push(Role {
            name: name.clone(),
            description: String::new(),
            default: directional.clone(),
        });
        *directional = Directional::uniform(&name);
    };
    for action in &mut moves.actions {
        role(action.id.clone(), &mut action.animation);
        if let Some(hold) = &mut action.hold {
            role(format!("{}_hold", action.id), &mut hold.animation);
            if let Some(walk) = &mut hold.walk_animation {
                role(format!("{}_hold_walk", action.id), walk);
            }
        }
    }

    // The items that start the actions, and what they spend.
    let all_items: Vec<_> = extracted.items.iter().flat_map(|(_, i)| i).collect();
    let mut chosen = Vec::new();
    let mut wanted_items: Vec<String> = items.clone();
    while let Some(next) = wanted_items.pop() {
        if chosen
            .iter()
            .any(|i: &clockwork_core::item::ItemDef| i.id == next)
        {
            continue;
        }
        let item = all_items
            .iter()
            .find(|i| i.id == next)
            .ok_or_else(|| fail(format!("there is no item \"{next}\"")))?;
        if let Some(ammo) = &item.ammo {
            wanted_items.push(ammo.item.clone());
        }
        if let Some(action) = &item.action
            && moves.action(action).is_none()
        {
            return Err(fail(format!(
                "item \"{next}\" starts \"{action}\", which the ability doesn't have"
            )));
        }
        chosen.push((*item).clone());
    }
    chosen.sort_by_key(|c| all_items.iter().position(|i| i.id == c.id));

    Ok(Ability {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        moves,
        items: chosen,
        roles,
    })
}

#[cfg(test)]
mod tests {
    use clockwork_core::character2d::Character2d;
    use clockwork_core::item::ItemDef;
    use clockwork_core::sprite::Sprite;

    use super::*;
    use crate::ExtractedSprite;

    fn hero() -> Extracted {
        let directional = |n: &str| {
            format!(
                "(north: (name: \"{n}_n\"), east: (name: \"{n}_e\"), south: (name: \"{n}_s\"), west: (name: \"{n}_e\", flip_x: true))"
            )
        };
        let character: Character2d = ron::from_str(&format!(
            "(movement: (speed: 4.0, radius: 0.4), animations: (idle: {i}, walk: {w}),
              actions: [
                (id: \"draw\", animation: {d}, duration: 0.1,
                 hold: Some((animation: {h}, release: Some((action: \"shoot\"))))),
                (id: \"shoot\", animation: {s}, duration: 0.3, spawns: [(at: 0.1, projectile: \"arrow\")]),
                (id: \"dodge\", animation: {r}, duration: 0.4),
              ],
              projectiles: [(id: \"arrow\", motion: Straight(speed: 10.0), lifetime: 1.0, radius: 0.1)],
              dodge: Some(\"dodge\"))",
            i = directional("idle"),
            w = directional("walk"),
            d = directional("draw"),
            h = directional("hold"),
            s = directional("shoot"),
            r = directional("dodge"),
        ))
        .unwrap();
        let items: Vec<ItemDef> = ron::from_str(
            "[(id: \"bow\", name: \"Bow\", description: \"\", category: Weapon, action: Some(\"draw\"), ammo: Some((item: \"arrows\", cost: 1))),
              (id: \"arrows\", name: \"Arrows\", description: \"\", category: Consumable, max: 30)]",
        )
        .unwrap();
        Extracted {
            sprites: vec![ExtractedSprite {
                id: "hero".into(),
                sprite: Sprite::default(),
                pixels_per_meter: 16.0,
                character: Some(character),
            }],
            items: vec![("items".into(), items)],
            ..Default::default()
        }
    }

    fn output(actions: &[&str], items: &[&str], dodge: bool) -> Output {
        Output::Ability {
            id: "bow".into(),
            name: "Bow".into(),
            description: String::new(),
            from: "hero".into(),
            actions: actions.iter().map(|s| s.to_string()).collect(),
            items: items.iter().map(|s| s.to_string()).collect(),
            dodge,
        }
    }

    #[test]
    fn follows_releases_projectiles_and_ammunition() {
        let ability = build(&output(&["draw"], &["bow"], false), &hero()).unwrap();
        let ids: Vec<&str> = ability
            .moves
            .actions
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, ["draw", "shoot"]);
        assert_eq!(ability.moves.projectiles[0].id, "arrow");
        let items: Vec<&str> = ability.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(items, ["bow", "arrows"]);
        // Every pose is a role now, with the hero's animations as its default.
        let roles: Vec<&str> = ability.roles.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(roles, ["draw", "draw_hold", "shoot"]);
        assert_eq!(ability.moves.actions[0].animation.role(), Some("draw"));
        assert_eq!(ability.role("draw").unwrap().default.west.name, "draw_e");
        assert_eq!(ability.moves.dodge, None);
    }

    #[test]
    fn the_dodge_and_mistakes() {
        let dodge = build(&output(&[], &[], true), &hero()).unwrap();
        assert_eq!(dodge.moves.dodge.as_deref(), Some("dodge"));
        assert!(build(&output(&["nope"], &[], false), &hero()).is_err());
        // An item whose action the ability lacks.
        assert!(build(&output(&["dodge"], &["bow"], false), &hero()).is_err());
    }
}

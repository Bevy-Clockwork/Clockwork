//! Every asset of every plugin, plus the built-in ones, with what each uses and what uses it.
//! Built from the plugins' recipes and compositions alone: no ROM is read.

use clockwork_core::{
    asset::{self, AssetId, AssetSource, BUILTIN, OutputRef, Use},
    compose::PART_SLOTS,
    recipe::{Kind, Output, Recipe},
};

use crate::Plugin;

/// One asset.
#[derive(Debug, Clone)]
pub struct Asset<'a> {
    pub at: OutputRef,
    /// The registered kind's name ("Sprite", "Camera", …); composed characters are "Character".
    pub kind: &'static str,
    /// Where the browser lists it.
    pub category: Option<Kind>,
    pub placeable: bool,
    /// The output itself; `None` for a composition's characters, which are made, not written.
    pub output: Option<&'a Output>,
    pub uses: Vec<Use>,
}

impl Asset<'_> {
    pub fn id(&self) -> AssetId {
        AssetId::new(self.at.clone(), self.kind)
    }
}

/// The assets of a set of plugins.
#[derive(Debug, Clone, Default)]
pub struct AssetIndex<'a> {
    assets: Vec<Asset<'a>>,
}

impl<'a> AssetIndex<'a> {
    pub fn build(plugins: &'a [Plugin]) -> Self {
        let mut assets: Vec<Asset<'a>> = Vec::new();
        for plugin in plugins {
            let name = plugin.name();
            assets.extend(plugin.recipe.outputs.iter().map(|o| of_output(name, o)));
            for character in plugin.compose.iter().flat_map(|c| &c.characters) {
                let mut uses = vec![Use {
                    slot: "base",
                    asset: character.base.clone(),
                }];
                uses.extend(character.abilities.iter().map(|a| Use {
                    slot: "ability",
                    asset: a.ability.clone(),
                }));
                uses.extend(character.parts.iter().filter_map(|(slot, at)| {
                    let (slot, _) = PART_SLOTS.iter().find(|(s, _)| s == slot)?;
                    Some(Use {
                        slot,
                        asset: at.clone(),
                    })
                }));
                assets.push(Asset {
                    at: OutputRef::new(name, &character.id),
                    kind: "Character",
                    category: Some(Kind::Character),
                    placeable: true,
                    output: None,
                    uses,
                });
            }
        }
        assets.extend(
            asset::builtins()
                .iter()
                .map(|output| of_output(BUILTIN, output)),
        );
        Self { assets }
    }

    pub fn all(&self) -> &[Asset<'a>] {
        &self.assets
    }

    /// The first asset of that name (see [`AssetSource`] for names shared between kinds).
    pub fn get(&self, at: &OutputRef) -> Option<&Asset<'a>> {
        self.assets.iter().find(|a| a.at == *at)
    }

    /// The asset of that name and kind.
    pub fn get_kind(&self, at: &OutputRef, kind: &str) -> Option<&Asset<'a>> {
        self.assets.iter().find(|a| a.at == *at && a.kind == kind)
    }

    /// The asset with that id.
    pub fn find(&self, id: &AssetId) -> Option<&Asset<'a>> {
        self.get_kind(&id.at, id.kind)
    }

    /// The assets that use `at`, with the slot they use it in.
    pub fn used_by(&self, at: &OutputRef) -> Vec<(&Asset<'a>, &'static str)> {
        self.assets
            .iter()
            .flat_map(|a| {
                a.uses
                    .iter()
                    .filter(|u| u.asset == *at)
                    .map(move |u| (a, u.slot))
            })
            .collect()
    }

    /// The assets of a kind.
    pub fn of_kind(&self, kind: &str) -> impl Iterator<Item = &Asset<'a>> {
        self.assets.iter().filter(move |a| a.kind == kind)
    }
}

impl AssetSource for AssetIndex<'_> {
    fn outputs(&self, at: &OutputRef) -> Vec<&Output> {
        self.assets
            .iter()
            .filter(|a| a.at == *at)
            .filter_map(|a| a.output)
            .collect()
    }
}

/// Every plugin's outputs, copied: for work that outlives the plugin list (an extraction in the
/// background) or edits one recipe (whose own outputs then come from the edited recipe).
#[derive(Debug, Clone, Default)]
pub struct OwnedAssets {
    outputs: Vec<(OutputRef, Output)>,
}

impl OwnedAssets {
    pub fn of(plugins: &[Plugin]) -> Self {
        Self {
            outputs: plugins
                .iter()
                .flat_map(|p| {
                    p.recipe
                        .outputs
                        .iter()
                        .map(|o| (OutputRef::new(p.name(), o.id()), o.clone()))
                })
                .collect(),
        }
    }

    /// These assets with plugin `plugin`'s outputs taken from `recipe` instead.
    pub fn with_recipe<'a>(&'a self, plugin: &'a str, recipe: &'a Recipe) -> EditedRecipe<'a> {
        EditedRecipe {
            base: self,
            plugin,
            recipe,
        }
    }
}

impl AssetSource for OwnedAssets {
    fn outputs(&self, at: &OutputRef) -> Vec<&Output> {
        let mut found: Vec<&Output> = self
            .outputs
            .iter()
            .filter(|(a, _)| a == at)
            .map(|(_, o)| o)
            .collect();
        found.extend(asset::Builtins.outputs(at));
        found
    }
}

/// [`OwnedAssets`] with one plugin's recipe being edited.
pub struct EditedRecipe<'a> {
    base: &'a OwnedAssets,
    plugin: &'a str,
    recipe: &'a Recipe,
}

impl AssetSource for EditedRecipe<'_> {
    fn outputs(&self, at: &OutputRef) -> Vec<&Output> {
        if at.plugin == self.plugin {
            return self
                .recipe
                .outputs
                .iter()
                .filter(|o| o.id() == at.output)
                .collect();
        }
        self.base.outputs(at)
    }
}

fn of_output<'a>(plugin: &str, output: &'a Output) -> Asset<'a> {
    let kind = asset::kind(output);
    let category = output.item().and_then(|(_, k)| k).or(kind.category);
    Asset {
        at: OutputRef::new(plugin, output.id()),
        kind: kind.name,
        category,
        placeable: kind.placeable,
        output: Some(output),
        uses: (kind.uses)(plugin, output),
    }
}

#[cfg(test)]
mod tests {
    use clockwork_core::camera;

    use super::*;

    fn plugin(name: &str, recipe: &str) -> Plugin {
        Plugin {
            dir: Default::default(),
            manifest: clockwork_core::PluginManifest::from_toml(&format!(
                "[plugin]\nname = \"{name}\"\nversion = \"0.1.0\"\ncore = \"0.1\"\n\n[source]\ngame = \"g\"\nsha1 = []\n"
            ))
            .unwrap(),
            recipe: ron::from_str(recipe).unwrap(),
            compose: None,
        }
    }

    #[test]
    fn built_ins_and_named_cameras_are_indexed_with_their_users() {
        let plugins = [plugin(
            "hero",
            r#"(steps: [], outputs: [
                Camera(id: "low", camera: "builtin/billboard"),
                Ability(id: "slash", name: "Slash", from: "hero", actions: []),
            ])"#,
        )];
        let index = AssetIndex::build(&plugins);
        let billboard = camera::builtin(camera::BILLBOARD);
        assert_eq!(index.get(&billboard).map(|a| a.kind), Some("Camera"));
        let low = index.get(&OutputRef::new("hero", "low")).unwrap();
        assert_eq!(low.category, Some(Kind::Camera));
        assert!(!low.placeable);
        let users = index.used_by(&billboard);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].0.at, OutputRef::new("hero", "low"));
        assert_eq!(users[0].1, "same as");
        assert_eq!(
            index.used_by(&OutputRef::new("hero", "hero"))[0].1,
            "character"
        );
    }

    #[test]
    fn a_name_shared_between_kinds_finds_the_kind_asked_for() {
        let plugins = [plugin(
            "hero",
            r#"(steps: [], outputs: [
                Ability(id: "view", name: "View", from: "hero", actions: []),
                Camera(id: "view", camera: TopDown((distance: 5.0, pitch: 45.0))),
            ])"#,
        )];
        let index = AssetIndex::build(&plugins);
        let slot = asset::Slot::Ref(OutputRef::new("hero", "view"));
        let found = asset::resolve(
            &[(asset::Layer::Asset, Some(&slot))],
            &index,
            "top-down camera",
            asset::top_down_camera,
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.value.pitch, 45.0);
        // An id names one of them exactly.
        let at = OutputRef::new("hero", "view");
        for kind in ["Ability", "Camera"] {
            let id = asset::AssetId::new(at.clone(), kind);
            assert_eq!(index.find(&id).map(|a| a.id()), Some(id));
        }
        assert!(index.find(&asset::AssetId::new(at, "Sprite")).is_none());
    }
}

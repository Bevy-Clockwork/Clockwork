//! Input maps: which keys and gamepad buttons press which game button. Keys and pad buttons are
//! named as Bevy names them (`"KeyK"`, `"Space"`, `"ArrowUp"`; `"South"`, `"Start"`,
//! `"DPadUp"`), so plugins and games can rebind without code.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    recipe::Output,
};

/// The game buttons, by name.
pub const BUTTONS: [&str; 12] = [
    "Up", "Down", "Left", "Right", "A", "B", "Dodge", "Jump", "Walk", "Menu", "Pause", "Back",
];

/// The keys and pad buttons of one game button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub button: String,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub pad: Vec<String>,
    /// The name the HUD shows for it on a keyboard and on a gamepad.
    #[serde(default)]
    pub labels: Option<(String, String)>,
}

/// Every game button's keys and pad buttons.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InputMap {
    pub bindings: Vec<Binding>,
}

impl InputMap {
    pub fn binding(&self, button: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.button == button)
    }
}

/// Button names shown for a controller. Auto follows the connected controller's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControllerStyle {
    #[default]
    Auto,
    Xbox,
    PlayStation,
    Generic,
}

/// A player's override of one action. None inherits the game's binding; an empty list
/// explicitly unbinds that device. Keyboard and controller bindings are independent.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BindingOverride {
    pub button: String,
    pub keys: Option<Vec<String>>,
    pub pad: Option<Vec<String>>,
}

/// Local gameplay preferences, layered over a game's own input map. These are not written
/// into plugins, so changing controls never changes a game's shared recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlSettings {
    pub overrides: Vec<BindingOverride>,
    pub controller_style: ControllerStyle,
    pub stick_deadzone: f32,
}

impl Default for ControlSettings {
    fn default() -> Self {
        Self {
            overrides: Vec::new(),
            controller_style: ControllerStyle::Auto,
            stick_deadzone: 0.18,
        }
    }
}

impl ControlSettings {
    pub fn effective_map(&self, base: &InputMap) -> InputMap {
        let mut map = base.clone();
        for name in BUTTONS {
            let Some(overridden) = self.overrides.iter().find(|b| b.button == name) else {
                continue;
            };
            if overridden.keys.is_none() && overridden.pad.is_none() {
                continue;
            }
            let index = match map.bindings.iter().position(|b| b.button == name) {
                Some(index) => index,
                None => {
                    map.bindings.push(Binding {
                        button: name.into(),
                        keys: Vec::new(),
                        pad: Vec::new(),
                        labels: None,
                    });
                    map.bindings.len() - 1
                }
            };
            let binding = &mut map.bindings[index];
            if let Some(keys) = &overridden.keys {
                binding.keys = keys.clone();
            }
            if let Some(pad) = &overridden.pad {
                binding.pad = pad.clone();
            }
            // The runtime derives the new labels from the physical binding. Keeping the
            // game's old label here would show the wrong key after a rebind.
            binding.labels = None;
        }
        map
    }

    pub fn set_keys(&mut self, button: &str, keys: Vec<String>) {
        if let Some(binding) = self.override_mut(button) {
            binding.keys = Some(keys);
        }
    }

    pub fn set_pad(&mut self, button: &str, pad: Vec<String>) {
        if let Some(binding) = self.override_mut(button) {
            binding.pad = Some(pad);
        }
    }

    /// Restores both devices to this game's defaults for one action.
    pub fn reset(&mut self, button: &str) {
        self.overrides.retain(|binding| binding.button != button);
    }

    fn override_mut(&mut self, button: &str) -> Option<&mut BindingOverride> {
        if !BUTTONS.contains(&button) {
            return None;
        }
        let index = match self.overrides.iter().position(|b| b.button == button) {
            Some(index) => index,
            None => {
                self.overrides.push(BindingOverride {
                    button: button.into(),
                    ..Default::default()
                });
                self.overrides.len() - 1
            }
        };
        self.overrides.get_mut(index)
    }
}

pub const INPUT: &str = "input";

pub fn builtin() -> OutputRef {
    OutputRef::new(BUILTIN, INPUT)
}

/// The built-in map: WASD or arrows, J and K for the item buttons, L to dodge, I for items,
/// Enter to pause; a gamepad's stick or pad, X and Y, A to dodge, Start for items.
pub fn default_map() -> InputMap {
    let bind = |button: &str, keys: &[&str], pad: &[&str], labels: Option<(&str, &str)>| Binding {
        button: button.into(),
        keys: keys.iter().map(|k| k.to_string()).collect(),
        pad: pad.iter().map(|p| p.to_string()).collect(),
        labels: labels.map(|(k, p)| (k.to_string(), p.to_string())),
    };
    InputMap {
        bindings: vec![
            bind("Up", &["KeyW", "ArrowUp"], &["DPadUp"], None),
            bind("Down", &["KeyS", "ArrowDown"], &["DPadDown"], None),
            bind("Left", &["KeyA", "ArrowLeft"], &["DPadLeft"], None),
            bind("Right", &["KeyD", "ArrowRight"], &["DPadRight"], None),
            bind("A", &["KeyK"], &["North"], Some(("K", "Y"))),
            bind("B", &["KeyJ"], &["West"], Some(("J", "X"))),
            bind(
                "Dodge",
                &["KeyL"],
                &["South", "RightTrigger"],
                Some(("L", "A")),
            ),
            bind("Jump", &["Space"], &["East"], Some(("Space", "B"))),
            bind(
                "Walk",
                &["ShiftLeft", "ShiftRight"],
                &["LeftTrigger"],
                Some(("Shift", "LB")),
            ),
            bind("Menu", &["KeyI"], &["Start"], Some(("I", "Start"))),
            bind("Pause", &["Enter"], &["Select"], Some(("Enter", "Select"))),
            bind("Back", &["Backspace"], &["East"], Some(("Backspace", "B"))),
        ],
    }
}

pub(crate) fn builtins() -> Vec<Output> {
    vec![Output::InputMap {
        id: INPUT.into(),
        input: Slot::Inline(default_map()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_map_binds_every_button() {
        let map = default_map();
        for button in BUTTONS {
            assert!(map.binding(button).is_some(), "{button} is unbound");
        }
    }

    #[test]
    fn overrides_inherit_the_game_and_keep_devices_independent() {
        let mut base = default_map();
        base.bindings
            .iter_mut()
            .find(|b| b.button == "Jump")
            .unwrap()
            .keys = vec!["KeyQ".into()];
        let mut settings = ControlSettings::default();
        assert_eq!(settings.effective_map(&base), base);
        settings.set_keys("A", vec!["KeyZ".into()]);
        settings.set_pad("Jump", vec!["South".into()]);
        let effective = settings.effective_map(&base);
        assert_eq!(effective.binding("A").unwrap().keys, ["KeyZ"]);
        assert_eq!(effective.binding("A").unwrap().pad, ["North"]);
        assert!(effective.binding("A").unwrap().labels.is_none());
        assert_eq!(effective.binding("Jump").unwrap().keys, ["KeyQ"]);
        assert_eq!(effective.binding("Jump").unwrap().pad, ["South"]);
        assert_eq!(effective.binding("Menu"), base.binding("Menu"));
        settings.reset("Jump");
        assert_eq!(
            settings.effective_map(&base).binding("Jump"),
            base.binding("Jump")
        );
    }

    #[test]
    fn unbinding_and_old_settings_round_trip() {
        let old: ControlSettings = ron::from_str("()").unwrap();
        assert_eq!(old, ControlSettings::default());
        let mut settings = old;
        settings.set_keys("A", Vec::new());
        settings.set_keys("NotAnAction", vec!["KeyQ".into()]);
        assert_eq!(settings.overrides.len(), 1);
        settings.controller_style = ControllerStyle::PlayStation;
        settings.stick_deadzone = 0.25;
        let text = ron::to_string(&settings).unwrap();
        let loaded: ControlSettings = ron::from_str(&text).unwrap();
        assert_eq!(loaded, settings);
        let map = loaded.effective_map(&default_map());
        assert!(map.binding("A").unwrap().keys.is_empty());
        assert_eq!(map.binding("A").unwrap().pad, ["North"]);
    }

    #[test]
    fn an_override_can_bind_an_action_missing_from_the_game_map() {
        let mut settings = ControlSettings::default();
        settings.set_keys("Pause", vec!["Enter".into()]);
        let map = settings.effective_map(&InputMap::default());
        assert_eq!(map.bindings.len(), 1);
        assert_eq!(map.binding("Pause").unwrap().keys, ["Enter"]);
    }
}

//! The game's buttons, read from the keyboard and any gamepad through an input map (an asset, see
//! `clockwork_core::input`), so play code and the HUD ask for "A" or "Menu" instead of keys.

use bevy::{input::gamepad::Gamepad, prelude::*};
use clockwork_core::input::{ControllerStyle, InputMap};

/// A button of the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameButton {
    Up,
    Down,
    Left,
    Right,
    /// The item buttons.
    A,
    B,
    Dodge,
    Jump,
    /// Held to walk instead of run.
    Walk,
    /// Opens and closes the inventory.
    Menu,
    /// Opens and closes the pause menu.
    Pause,
    /// Leaves a menu.
    Back,
}

impl GameButton {
    pub const ALL: [GameButton; 12] = [
        GameButton::Up,
        GameButton::Down,
        GameButton::Left,
        GameButton::Right,
        GameButton::A,
        GameButton::B,
        GameButton::Dodge,
        GameButton::Jump,
        GameButton::Walk,
        GameButton::Menu,
        GameButton::Pause,
        GameButton::Back,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// Its name in input maps and HUD layouts.
    pub fn name(self) -> &'static str {
        clockwork_core::input::BUTTONS[self.index()]
    }

    pub fn named(name: &str) -> Option<GameButton> {
        GameButton::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// Where the player's input last came from, for the button names the HUD shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Keyboard,
    Gamepad,
}

/// How far the stick must lean to count as a direction button.
const STICK_PRESS: f32 = 0.5;

/// Player preferences shared by the settings screen and gameplay input.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ControllerOptions {
    pub style: ControllerStyle,
    pub deadzone: f32,
}

impl Default for ControllerOptions {
    fn default() -> Self {
        Self {
            style: ControllerStyle::Auto,
            deadzone: 0.18,
        }
    }
}

/// The input map in use, read into keys and pad buttons.
#[derive(Resource, Debug, Clone)]
pub struct Bindings {
    keys: [Vec<KeyCode>; GameButton::ALL.len()],
    pad: [Vec<GamepadButton>; GameButton::ALL.len()],
    labels: [(String, String); GameButton::ALL.len()],
    /// The map it was read from, to tell when it changes.
    pub map: InputMap,
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings::new(&clockwork_core::input::default_map())
    }
}

impl Bindings {
    /// Reads a map; names Bevy doesn't know are left out.
    pub fn new(map: &InputMap) -> Self {
        let binding = |b: GameButton| map.binding(b.name());
        let keys = GameButton::ALL.map(|b| {
            binding(b)
                .map(|b| b.keys.iter().filter_map(|k| key(k)).collect())
                .unwrap_or_default()
        });
        let pad = GameButton::ALL.map(|b| {
            binding(b)
                .map(|b| b.pad.iter().filter_map(|p| pad_button(p)).collect())
                .unwrap_or_default()
        });
        let labels = GameButton::ALL.map(|b| match binding(b) {
            Some(binding) => binding.labels.clone().unwrap_or_else(|| {
                (
                    binding
                        .keys
                        .first()
                        .map(|name| key_label(name))
                        .unwrap_or_default(),
                    binding
                        .pad
                        .first()
                        .map(|name| pad_label(name, ControllerStyle::Generic))
                        .unwrap_or_default(),
                )
            }),
            None => Default::default(),
        });
        Self {
            keys,
            pad,
            labels,
            map: map.clone(),
        }
    }
}

/// A keyboard key by its Bevy name.
pub fn key(name: &str) -> Option<KeyCode> {
    use KeyCode::*;
    const KEYS: &[KeyCode] = &[
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Space,
        Enter,
        Backspace,
        Tab,
        Escape,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        Comma,
        Period,
        Slash,
        Semicolon,
        Quote,
        Minus,
        Equal,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadEnter,
        Backquote,
        Backslash,
        BracketLeft,
        BracketRight,
        IntlBackslash,
        IntlRo,
        IntlYen,
        CapsLock,
        ContextMenu,
        SuperLeft,
        SuperRight,
        Convert,
        KanaMode,
        Lang1,
        Lang2,
        Lang3,
        Lang4,
        Lang5,
        NonConvert,
        Delete,
        End,
        Help,
        Home,
        Insert,
        PageDown,
        PageUp,
        NumLock,
        NumpadAdd,
        NumpadBackspace,
        NumpadClear,
        NumpadClearEntry,
        NumpadComma,
        NumpadDecimal,
        NumpadDivide,
        NumpadEqual,
        NumpadHash,
        NumpadMemoryAdd,
        NumpadMemoryClear,
        NumpadMemoryRecall,
        NumpadMemoryStore,
        NumpadMemorySubtract,
        NumpadMultiply,
        NumpadParenLeft,
        NumpadParenRight,
        NumpadStar,
        NumpadSubtract,
        Fn,
        FnLock,
        PrintScreen,
        ScrollLock,
        Pause,
        BrowserBack,
        BrowserFavorites,
        BrowserForward,
        BrowserHome,
        BrowserRefresh,
        BrowserSearch,
        BrowserStop,
        Eject,
        LaunchApp1,
        LaunchApp2,
        LaunchMail,
        MediaPlayPause,
        MediaSelect,
        MediaStop,
        MediaTrackNext,
        MediaTrackPrevious,
        Power,
        Sleep,
        AudioVolumeDown,
        AudioVolumeMute,
        AudioVolumeUp,
        WakeUp,
        Meta,
        Hyper,
        Turbo,
        Abort,
        Resume,
        Suspend,
        Again,
        Copy,
        Cut,
        Find,
        Open,
        Paste,
        Props,
        Select,
        Undo,
        Hiragana,
        Katakana,
        F13,
        F14,
        F15,
        F16,
        F17,
        F18,
        F19,
        F20,
        F21,
        F22,
        F23,
        F24,
        F25,
        F26,
        F27,
        F28,
        F29,
        F30,
        F31,
        F32,
        F33,
        F34,
        F35,
    ];
    KEYS.iter().copied().find(|k| format!("{k:?}") == name)
}

/// A gamepad button by its Bevy name.
pub fn pad_button(name: &str) -> Option<GamepadButton> {
    if let Some(index) = name
        .strip_prefix("Other(")
        .and_then(|name| name.strip_suffix(')'))
    {
        return index.parse::<u32>().ok().map(GamepadButton::Other);
    }
    GamepadButton::all()
        .into_iter()
        .find(|b| format!("{b:?}") == name)
}

/// A short, readable label for a physical keyboard binding.
pub fn key_label(name: &str) -> String {
    if let Some(letter) = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
    {
        return letter.to_owned();
    }
    let label = match name {
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ShiftLeft" => "Left Shift",
        "ShiftRight" => "Right Shift",
        "ControlLeft" => "Left Ctrl",
        "ControlRight" => "Right Ctrl",
        "AltLeft" => "Left Alt",
        "AltRight" => "Right Alt",
        "SuperLeft" => "Left Super",
        "SuperRight" => "Right Super",
        "Backquote" => "`",
        "Backslash" => "\\",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Semicolon" => ";",
        "Quote" => "'",
        "Minus" => "-",
        "Equal" => "=",
        _ => return spaced_name(name),
    };
    label.to_owned()
}

fn spaced_name(name: &str) -> String {
    let mut label = String::new();
    for (index, character) in name.chars().enumerate() {
        if index > 0 && character.is_uppercase() {
            label.push(' ');
        }
        label.push(character);
    }
    label
}

/// Selects prompt artwork by reliable vendor IDs or recognizable device names.
/// An ambiguous name such as "Wireless Controller" stays generic without a vendor ID.
pub fn controller_style(name: &str, vendor_id: Option<u16>) -> ControllerStyle {
    match vendor_id {
        Some(0x054c) => return ControllerStyle::PlayStation,
        Some(0x045e) => return ControllerStyle::Xbox,
        _ => {}
    }
    let name = name.to_lowercase();
    if [
        "dualsense",
        "dualshock",
        "playstation",
        "ps5",
        "ps4",
        "sony",
    ]
    .iter()
    .any(|part| name.contains(part))
    {
        ControllerStyle::PlayStation
    } else if name.contains("xbox") || name.contains("xinput") || name.contains("x-box") {
        ControllerStyle::Xbox
    } else {
        ControllerStyle::Generic
    }
}

/// Names a mapped gamepad button using the selected controller's printed labels.
/// Auto has neutral labels until an actual controller can be identified.
pub fn pad_label(name: &str, style: ControllerStyle) -> String {
    use ControllerStyle::*;
    let label = match (style, name) {
        (Xbox, "South") => "A",
        (Xbox, "East") => "B",
        (Xbox, "West") => "X",
        (Xbox, "North") => "Y",
        (Xbox, "LeftTrigger") => "LB",
        (Xbox, "RightTrigger") => "RB",
        (Xbox, "LeftTrigger2") => "LT",
        (Xbox, "RightTrigger2") => "RT",
        (Xbox, "Select") => "View",
        (Xbox, "Start") => "Menu",
        (Xbox, "Mode") => "Xbox",
        (PlayStation, "South") => "Cross",
        (PlayStation, "East") => "Circle",
        (PlayStation, "West") => "Square",
        (PlayStation, "North") => "Triangle",
        (PlayStation, "LeftTrigger") => "L1",
        (PlayStation, "RightTrigger") => "R1",
        (PlayStation, "LeftTrigger2") => "L2",
        (PlayStation, "RightTrigger2") => "R2",
        (PlayStation, "LeftThumb") => "L3",
        (PlayStation, "RightThumb") => "R3",
        (PlayStation, "Select") => "Create / Share",
        (PlayStation, "Start") => "Options",
        (PlayStation, "Mode") => "PS",
        (_, "South") => "South",
        (_, "East") => "East",
        (_, "West") => "West",
        (_, "North") => "North",
        (_, "LeftTrigger") => "Left Bumper",
        (_, "RightTrigger") => "Right Bumper",
        (_, "LeftTrigger2") => "Left Trigger",
        (_, "RightTrigger2") => "Right Trigger",
        (_, "LeftThumb") => "Left Stick",
        (_, "RightThumb") => "Right Stick",
        (_, "DPadUp") => "D-pad Up",
        (_, "DPadDown") => "D-pad Down",
        (_, "DPadLeft") => "D-pad Left",
        (_, "DPadRight") => "D-pad Right",
        _ => {
            return match pad_button(name) {
                Some(GamepadButton::Other(index)) => format!("Button {index}"),
                _ => name.to_owned(),
            };
        }
    };
    label.to_owned()
}

/// This frame's game input.
#[derive(Resource, Debug, Default)]
pub struct GameInput {
    /// The movement stick, at most 1 long. Right and up are positive.
    pub stick: Vec2,
    /// The right stick for camera movement, at most 1 long.
    pub look: Vec2,
    held: [bool; GameButton::ALL.len()],
    pressed: [bool; GameButton::ALL.len()],
    pub device: Device,
    /// Set by the host while the keyboard belongs to something else (a text field): the game
    /// then sees nothing pressed.
    pub blocked: bool,
    labels: [(String, String); GameButton::ALL.len()],
    /// Buttons the host presses for the next frame (the remote control).
    injected: Vec<GameButton>,
    active_pad: Option<Entity>,
    pad_style: ControllerStyle,
    bound_keys: Vec<KeyCode>,
}

impl GameInput {
    /// Hosts can keep editor/debug shortcuts from also firing a mapped gameplay key.
    pub fn uses_key(&self, key: KeyCode) -> bool {
        self.bound_keys.contains(&key)
    }

    /// Whether the button is down.
    pub fn held(&self, button: GameButton) -> bool {
        self.held[button.index()]
    }

    /// Whether the button went down this frame.
    pub fn pressed(&self, button: GameButton) -> bool {
        self.pressed[button.index()]
    }

    /// The name of the button on the device in use, for the HUD.
    pub fn label(&self, button: GameButton) -> &str {
        let (keyboard, pad) = &self.labels[button.index()];
        match self.device {
            Device::Keyboard => keyboard,
            Device::Gamepad => pad,
        }
    }

    /// Presses a button for the next frame, as if a player did.
    pub fn inject(&mut self, button: GameButton) {
        self.injected.push(button);
    }

    /// Takes the buttons that are down now: what went down since last frame is pressed.
    fn update(&mut self, mut held: [bool; GameButton::ALL.len()], stick: Vec2) {
        for button in std::mem::take(&mut self.injected) {
            held[button.index()] = true;
        }
        let (held, stick) = if self.blocked {
            (Default::default(), Vec2::ZERO)
        } else {
            (held, stick)
        };
        for ((pressed, now), before) in self.pressed.iter_mut().zip(held).zip(self.held) {
            *pressed = now && !before;
        }
        self.held = held;
        self.stick = stick.clamp_length_max(1.0);
        self.look = if self.blocked {
            Vec2::ZERO
        } else {
            self.look.clamp_length_max(1.0)
        };
    }
}

/// The buttons and stick the keyboard holds.
fn from_keys(
    bindings: &Bindings,
    keys: &ButtonInput<KeyCode>,
) -> ([bool; GameButton::ALL.len()], Vec2) {
    let held = GameButton::ALL.map(|b| keys.any_pressed(bindings.keys[b.index()].iter().copied()));
    let axis = |plus: GameButton, minus: GameButton| {
        held[plus.index()] as i32 as f32 - held[minus.index()] as i32 as f32
    };
    let stick = Vec2::new(
        axis(GameButton::Right, GameButton::Left),
        axis(GameButton::Up, GameButton::Down),
    );
    (held, stick.normalize_or_zero())
}

/// Removes drift radially, preserving the direction and rescaling the remaining travel.
fn radial_deadzone(stick: Vec2, deadzone: f32) -> Vec2 {
    if !stick.is_finite() {
        return Vec2::ZERO;
    }
    let deadzone = if deadzone.is_finite() {
        deadzone.clamp(0.0, 0.95)
    } else {
        ControllerOptions::default().deadzone
    };
    let length = stick.length();
    if length <= deadzone {
        Vec2::ZERO
    } else {
        stick / length * ((length.min(1.0) - deadzone) / (1.0 - deadzone))
    }
}

/// Direction buttons follow their bindings; the left stick remains an analog movement source.
fn from_pad(
    bindings: &Bindings,
    pad: &Gamepad,
    deadzone: f32,
) -> ([bool; GameButton::ALL.len()], Vec2) {
    let analog = radial_deadzone(pad.left_stick(), deadzone);
    let mut held =
        GameButton::ALL.map(|b| pad.any_pressed(bindings.pad[b.index()].iter().copied()));
    let axis = |plus: GameButton, minus: GameButton| {
        held[plus.index()] as i32 as f32 - held[minus.index()] as i32 as f32
    };
    let digital = Vec2::new(
        axis(GameButton::Right, GameButton::Left),
        axis(GameButton::Up, GameButton::Down),
    );
    let stick = (analog + digital).clamp_length_max(1.0);
    held[GameButton::Right.index()] |= analog.x > STICK_PRESS;
    held[GameButton::Left.index()] |= analog.x < -STICK_PRESS;
    held[GameButton::Up.index()] |= analog.y > STICK_PRESS;
    held[GameButton::Down.index()] |= analog.y < -STICK_PRESS;
    (held, stick)
}

/// Reads the keyboard and every gamepad into [`GameInput`] through the [`Bindings`].
pub fn input_system(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    bindings: Res<Bindings>,
    options: Option<Res<ControllerOptions>>,
    mut input: ResMut<GameInput>,
) {
    let options = options.as_deref().copied().unwrap_or_default();
    let (mut held, mut stick) = from_keys(&bindings, &keys);
    let mut look = Vec2::ZERO;
    let keyboard = held.iter().any(|h| *h);
    let mut gamepad = false;
    let mut first_pad = None;
    let mut previous_pad = None;
    let mut active_pad = None;
    for (entity, pad, name) in &pads {
        let style = controller_style(name.map(Name::as_str).unwrap_or_default(), pad.vendor_id());
        first_pad.get_or_insert((entity, style));
        if input.active_pad == Some(entity) {
            previous_pad = Some((entity, style));
        }
        let (pad_held, pad_stick) = from_pad(&bindings, pad, options.deadzone);
        let pad_look = radial_deadzone(pad.right_stick(), options.deadzone);
        let active =
            pad_held.iter().any(|h| *h) || pad_stick != Vec2::ZERO || pad_look != Vec2::ZERO;
        gamepad |= active;
        if active && (active_pad.is_none() || pad.get_just_pressed().next().is_some()) {
            active_pad = Some((entity, style));
        }
        for (h, p) in held.iter_mut().zip(pad_held) {
            *h |= p;
        }
        if pad_stick.length() > stick.length() {
            stick = pad_stick;
        }
        if pad_look.length_squared() > look.length_squared() {
            look = pad_look;
        }
    }
    let selected_pad = active_pad.or(previous_pad).or(first_pad);
    input.active_pad = selected_pad.map(|(entity, _)| entity);
    let style = match options.style {
        ControllerStyle::Auto => selected_pad
            .map(|(_, style)| style)
            .unwrap_or(ControllerStyle::Generic),
        style => style,
    };
    if bindings.is_changed() || input.pad_style != style {
        input.bound_keys = bindings.keys.iter().flatten().copied().collect();
        input.labels = GameButton::ALL.map(|button| {
            let keyboard = bindings.labels[button.index()].0.clone();
            let pad = bindings
                .map
                .binding(button.name())
                .and_then(|binding| binding.pad.first())
                .map(|name| pad_label(name, style))
                .unwrap_or_default();
            (keyboard, pad)
        });
        input.pad_style = style;
    }
    if keyboard || selected_pad.is_none() {
        input.device = Device::Keyboard;
    } else if gamepad {
        input.device = Device::Gamepad;
    }
    input.look = look;
    input.update(held, stick);
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameInput>()
            .init_resource::<Bindings>()
            .init_resource::<ControllerOptions>()
            .configure_sets(PreUpdate, InputSystems.after(bevy::input::InputSystems))
            .add_systems(PreUpdate, input_system.in_set(InputSystems));
    }
}

/// Reads [`GameInput`]; it runs after Bevy's own input.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputSystems;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::gamepad::{
        GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
        RawGamepadButtonChangedEvent, RawGamepadEvent,
    };

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keys = ButtonInput::default();
        for key in pressed {
            keys.press(*key);
        }
        keys
    }

    #[test]
    fn every_name_in_the_default_map_is_known() {
        for binding in clockwork_core::input::default_map().bindings {
            assert!(GameButton::named(&binding.button).is_some());
            for k in &binding.keys {
                assert!(key(k).is_some(), "key {k}");
            }
            for p in &binding.pad {
                assert!(pad_button(p).is_some(), "pad {p}");
            }
        }
    }

    #[test]
    fn keys_map_to_buttons_and_a_unit_stick() {
        let bindings = Bindings::default();
        let (held, stick) = from_keys(
            &bindings,
            &keys(&[KeyCode::KeyK, KeyCode::KeyW, KeyCode::ArrowRight]),
        );
        assert!(held[GameButton::A.index()]);
        assert!(!held[GameButton::B.index()]);
        assert!((stick.length() - 1.0).abs() < 1e-5);
        assert!(stick.x > 0.0 && stick.y > 0.0);
    }

    #[test]
    fn a_rebound_button_follows_its_map() {
        let mut map = clockwork_core::input::default_map();
        for binding in &mut map.bindings {
            if binding.button == "A" {
                binding.keys = vec!["KeyZ".into()];
            }
        }
        let bindings = Bindings::new(&map);
        let (held, _) = from_keys(&bindings, &keys(&[KeyCode::KeyZ]));
        assert!(held[GameButton::A.index()]);
        let (held, _) = from_keys(&bindings, &keys(&[KeyCode::KeyK]));
        assert!(!held[GameButton::A.index()]);
    }

    #[test]
    fn a_button_is_pressed_only_on_the_frame_it_goes_down() {
        let mut input = GameInput::default();
        let (held, stick) = from_keys(&Bindings::default(), &keys(&[KeyCode::KeyI]));
        input.update(held, stick);
        assert!(input.pressed(GameButton::Menu));
        input.update(held, stick);
        assert!(!input.pressed(GameButton::Menu));
        assert!(input.held(GameButton::Menu));
    }

    #[test]
    fn blocked_input_holds_nothing() {
        let mut input = GameInput {
            blocked: true,
            look: Vec2::ONE,
            ..default()
        };
        let (held, stick) = from_keys(&Bindings::default(), &keys(&[KeyCode::KeyJ, KeyCode::KeyD]));
        input.update(held, stick);
        assert!(!input.held(GameButton::B));
        assert_eq!(input.stick, Vec2::ZERO);
        assert_eq!(input.look, Vec2::ZERO);
    }

    #[test]
    fn punctuation_navigation_numpad_and_generic_buttons_can_be_rebound() {
        for name in [
            "BracketLeft",
            "Backslash",
            "Backquote",
            "Delete",
            "Home",
            "PageDown",
            "NumpadAdd",
            "NumpadMultiply",
            "NumpadDecimal",
            "F24",
            "F35",
            "SuperLeft",
        ] {
            assert!(key(name).is_some(), "{name}");
        }
        assert_eq!(pad_button("Other(0)"), Some(GamepadButton::Other(0)));
        assert_eq!(pad_button("Other(255)"), Some(GamepadButton::Other(255)));
        assert_eq!(
            pad_button("Other(65536)"),
            Some(GamepadButton::Other(65536))
        );
        assert_eq!(pad_button("Other(4294967296)"), None);
        assert_eq!(pad_button("Other(-1)"), None);
        assert_eq!(key("UnrecognizedKey"), None);
        assert_eq!(key_label("BracketLeft"), "[");
        assert_eq!(key_label("KeyZ"), "Z");
        assert_eq!(key_label("ControlRight"), "Right Ctrl");
    }

    #[test]
    fn radial_deadzone_removes_drift_and_preserves_direction() {
        assert_eq!(radial_deadzone(Vec2::new(0.1, 0.1), 0.18), Vec2::ZERO);
        let input = Vec2::new(0.3, 0.4);
        let filtered = radial_deadzone(input, 0.18);
        assert!((filtered.length() - (0.5 - 0.18) / 0.82).abs() < 1e-5);
        assert!((filtered.normalize() - input.normalize()).length() < 1e-5);
        assert!((radial_deadzone(Vec2::ONE, 0.18).length() - 1.0).abs() < 1e-5);
        assert_eq!(radial_deadzone(Vec2::ZERO, 0.0), Vec2::ZERO);
        assert_eq!(radial_deadzone(Vec2::splat(f32::NAN), 0.18), Vec2::ZERO);
        assert!(radial_deadzone(Vec2::X, f32::NAN).is_finite());
    }

    #[test]
    fn rebound_dpad_only_moves_when_bound_to_a_direction() {
        let mut map = clockwork_core::input::default_map();
        for binding in &mut map.bindings {
            if binding.button == "Up" {
                binding.pad = vec!["South".into()];
            } else if binding.button == "A" {
                binding.pad = vec!["DPadUp".into()];
            }
        }
        let bindings = Bindings::new(&map);
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::DPadUp);
        pad.analog_mut().set(GamepadButton::DPadUp, 1.0);
        let (held, stick) = from_pad(&bindings, &pad, 0.18);
        assert!(held[GameButton::A.index()]);
        assert!(!held[GameButton::Up.index()]);
        assert_eq!(stick, Vec2::ZERO);
        pad.digital_mut().release(GamepadButton::DPadUp);
        pad.digital_mut().press(GamepadButton::South);
        let (held, stick) = from_pad(&bindings, &pad, 0.18);
        assert!(held[GameButton::Up.index()]);
        assert_eq!(stick, Vec2::Y);
    }

    #[test]
    fn controller_identity_and_prompts_use_accurate_button_names() {
        use ControllerStyle::*;
        assert_eq!(
            controller_style("Wireless Controller", Some(0x054c)),
            PlayStation
        );
        assert_eq!(
            controller_style("DualSense Wireless Controller", None),
            PlayStation
        );
        assert_eq!(controller_style("Xbox Wireless Controller", None), Xbox);
        assert_eq!(controller_style("Controller", Some(0x045e)), Xbox);
        assert_eq!(controller_style("Wireless Controller", None), Generic);
        assert_eq!(controller_style("USB Gamepad", Some(0x1234)), Generic);
        assert_eq!(pad_label("South", PlayStation), "Cross");
        assert_eq!(pad_label("North", Xbox), "Y");
        assert_eq!(pad_label("LeftTrigger", PlayStation), "L1");
        assert_eq!(pad_label("LeftTrigger2", Xbox), "LT");
        assert_eq!(pad_label("South", Generic), "South");
        assert_eq!(pad_label("Other(17)", Generic), "Button 17");
    }

    fn input_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::input::InputPlugin, InputPlugin));
        app
    }

    fn connect(app: &mut App, entity: Entity) {
        app.world_mut()
            .resource_mut::<Messages<GamepadConnectionEvent>>()
            .write(GamepadConnectionEvent::new(
                entity,
                GamepadConnection::Connected {
                    name: "Wireless Controller".into(),
                    vendor_id: Some(0x054c),
                    product_id: Some(0x0ce6),
                },
            ));
    }

    fn send_pad(app: &mut App, entity: Entity, button: GamepadButton, value: f32) {
        app.world_mut()
            .resource_mut::<Messages<RawGamepadEvent>>()
            .write(RawGamepadButtonChangedEvent::new(entity, button, value).into());
    }

    #[test]
    fn bevy_events_reach_gameplay_in_the_same_frame_and_disconnect_clears_input() {
        let mut app = input_app();
        let entity = app.world_mut().spawn_empty().id();
        connect(&mut app, entity);
        send_pad(&mut app, entity, GamepadButton::North, 1.0);
        app.world_mut()
            .resource_mut::<Messages<RawGamepadEvent>>()
            .write(RawGamepadAxisChangedEvent::new(entity, GamepadAxis::RightStickX, 1.0).into());
        app.update();
        let input = app.world().resource::<GameInput>();
        assert!(input.pressed(GameButton::A));
        assert_eq!(input.look, Vec2::X);
        assert_eq!(input.device, Device::Gamepad);
        assert_eq!(input.label(GameButton::A), "Triangle");
        app.update();
        assert!(!app.world().resource::<GameInput>().pressed(GameButton::A));

        app.world_mut()
            .resource_mut::<Messages<GamepadConnectionEvent>>()
            .write(GamepadConnectionEvent::new(
                entity,
                GamepadConnection::Disconnected,
            ));
        app.update();
        let input = app.world().resource::<GameInput>();
        assert!(!input.held(GameButton::A));
        assert_eq!(input.look, Vec2::ZERO);
        assert_eq!(input.stick, Vec2::ZERO);
        assert_eq!(input.device, Device::Keyboard);

        connect(&mut app, entity);
        app.update();
        assert!(!app.world().resource::<GameInput>().held(GameButton::A));
        send_pad(&mut app, entity, GamepadButton::North, 1.0);
        app.update();
        assert!(app.world().resource::<GameInput>().pressed(GameButton::A));
    }

    #[test]
    fn manual_style_and_changed_bindings_update_prompts_without_reconnecting() {
        let mut app = input_app();
        let entity = app.world_mut().spawn_empty().id();
        connect(&mut app, entity);
        send_pad(&mut app, entity, GamepadButton::North, 1.0);
        app.update();
        app.world_mut().resource_mut::<ControllerOptions>().style = ControllerStyle::Xbox;
        app.update();
        assert_eq!(
            app.world().resource::<GameInput>().label(GameButton::A),
            "Y"
        );
        let mut map = clockwork_core::input::default_map();
        let binding = map
            .bindings
            .iter_mut()
            .find(|binding| binding.button == "A")
            .unwrap();
        binding.pad = vec!["Other(23)".into()];
        app.insert_resource(Bindings::new(&map));
        send_pad(&mut app, entity, GamepadButton::North, 0.0);
        send_pad(&mut app, entity, GamepadButton::Other(23), 1.0);
        app.update();
        let input = app.world().resource::<GameInput>();
        assert!(input.held(GameButton::A));
        assert_eq!(input.label(GameButton::A), "Button 23");
    }

    #[test]
    fn blocking_clears_controller_camera_and_buttons() {
        let mut app = input_app();
        let entity = app.world_mut().spawn_empty().id();
        connect(&mut app, entity);
        send_pad(&mut app, entity, GamepadButton::North, 1.0);
        app.world_mut()
            .resource_mut::<Messages<RawGamepadEvent>>()
            .write(RawGamepadAxisChangedEvent::new(entity, GamepadAxis::RightStickY, 1.0).into());
        app.world_mut().resource_mut::<GameInput>().blocked = true;
        app.update();
        let input = app.world().resource::<GameInput>();
        assert_eq!(input.look, Vec2::ZERO);
        assert!(!input.held(GameButton::A));
    }
}

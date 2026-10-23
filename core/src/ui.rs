//! The played character's screen: its HUD (widgets placed on the screen), its UI theme (how
//! panels and text look) and its menus (inventory, pause). Each is an asset a character names in
//! its [`Kit`]; without one it gets the built-in one.
//!
//! Sizes and offsets are in game pixels: the game shows them at a whole-number scale.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    input::InputMap,
    item::Slot as ItemSlot,
    recipe::Output,
};

/// A colour, red, green, blue and opacity from 0 to 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

/// Where on the screen a widget sits; its offset moves it inwards from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    #[default]
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];
}

/// What a widget shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WidgetKind {
    /// An item button: the item on it, how many are left, and its key.
    ItemSlot(ItemSlot),
    /// A button without an item, with its key: `"Dodge"`, `"Items"`.
    Button { label: String, button: String },
    /// The action going on.
    Action,
    /// The played character's health, as hearts (`per_heart` health each) or a bar.
    Health {
        #[serde(default)]
        per_heart: Option<f32>,
    },
    /// How many of an item the character carries.
    Counter { item: String },
    /// Lives left, when the game's rules count them.
    Lives,
    /// Seconds left, when the game's rules set a time limit.
    Timer,
    /// Fixed text.
    Label(String),
}

/// One thing on the HUD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub kind: WidgetKind,
    #[serde(default)]
    pub anchor: Anchor,
    /// Game pixels inwards from the anchor: right and up for bottom-left, and so on.
    #[serde(default)]
    pub offset: (i32, i32),
    /// A picture before the text, `"plugin/output"`: Mario's head before his lives. A counter
    /// without one shows its item's icon.
    #[serde(default)]
    pub icon: Option<String>,
    /// How a counter, lives or timer is written, `{n}` being the number (`"x{n}"`); for a
    /// timer `{m}` and `{ss}`, minutes and two-digit seconds. Without one just the number
    /// (a timer `{m}:{ss}`).
    #[serde(default)]
    pub format: Option<String>,
}

impl Widget {
    pub fn new(kind: WidgetKind, anchor: Anchor, offset: (i32, i32)) -> Self {
        Self {
            kind,
            anchor,
            offset,
            icon: None,
            format: None,
        }
    }

    /// The widget's text for the number `n` (seconds for a timer).
    pub fn text(&self, n: u32) -> String {
        match self.kind {
            WidgetKind::Timer => fill(
                self.format.as_deref().unwrap_or("{m}:{ss}"),
                &[
                    ("m", &(n / 60).to_string()),
                    ("ss", &format!("{:02}", n % 60)),
                    ("n", &n.to_string()),
                ],
            ),
            _ => fill(
                self.format.as_deref().unwrap_or("{n}"),
                &[("n", &n.to_string())],
            ),
        }
    }
}

/// What the HUD shows, and where.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HudLayout {
    pub widgets: Vec<Widget>,
    /// The words Play itself puts on the screen, in the game's own way.
    #[serde(default)]
    pub words: Words,
}

/// The words Play itself shows (pause menu, notices, button hints), so that a game can write
/// them its own way, in the letters its font has, or leave them out: an empty one isn't
/// shown. `{item}`, `{name}`, `{n}` and `{letter}` are filled in where a word has them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Words {
    pub resume: String,
    pub start_over: String,
    pub stop_playing: String,
    /// An empty inventory.
    pub nothing_carried: String,
    /// The inventory's button hints: what A and B do, and closing it.
    pub on_a: String,
    pub on_b: String,
    pub close: String,
    /// The pause menu's button hints.
    pub choose: String,
    pub back: String,
    /// A dialog's button hint while it has more pages.
    pub next: String,
    /// Picking something up: `{item}`, `{n}`.
    pub got: String,
    /// A target beaten: `{name}`.
    pub destroyed: String,
    pub checkpoint: String,
    /// Losing all health with lives left, and with none.
    pub down: String,
    pub game_over: String,
    /// The game's goal reached.
    pub win: String,
    /// A save file's name (`{letter}`: A, B, …) and a new file's mark.
    pub file: String,
    pub new_file: String,
}

impl Default for Words {
    fn default() -> Self {
        let w = |s: &str| s.to_string();
        Self {
            resume: w("Resume"),
            start_over: w("Start over"),
            stop_playing: w("Stop playing"),
            nothing_carried: w("Nothing carried."),
            on_a: w("on A"),
            on_b: w("on B"),
            close: w("close"),
            choose: w("choose"),
            back: w("back"),
            next: w("next"),
            got: w("Got {item} x{n}"),
            destroyed: w("{name} destroyed"),
            checkpoint: w("Checkpoint"),
            down: w("Down!"),
            game_over: w("Game over! Starting over"),
            win: w("You win!"),
            file: w("File {letter}"),
            new_file: w("NEW"),
        }
    }
}

impl Words {
    /// Every word with its name, to check a font has their letters.
    pub fn all(&self) -> [(&'static str, &str); 18] {
        [
            ("resume", &self.resume),
            ("start_over", &self.start_over),
            ("stop_playing", &self.stop_playing),
            ("nothing_carried", &self.nothing_carried),
            ("on_a", &self.on_a),
            ("on_b", &self.on_b),
            ("close", &self.close),
            ("choose", &self.choose),
            ("back", &self.back),
            ("next", &self.next),
            ("got", &self.got),
            ("destroyed", &self.destroyed),
            ("checkpoint", &self.checkpoint),
            ("down", &self.down),
            ("game_over", &self.game_over),
            ("win", &self.win),
            ("file", &self.file),
            ("new_file", &self.new_file),
        ]
    }
}

/// `template` with each `{key}` replaced by its value.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// The text of a template, without its `{key}`s: the letters a font must have for it.
pub fn template_letters(template: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in template.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// An image drawn stretched around its edges (nine-slice): `border` game pixels stay as they
/// are on each side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// An image output: `"plugin/output"`.
    pub image: String,
    pub border: u32,
}

/// How panels and text look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub panel: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    /// Chosen things and button letters.
    pub accent: Rgba,
    /// Behind open menus.
    pub shade: Rgba,
    /// A picture for panels instead of the plain colours.
    #[serde(default)]
    pub frame: Option<Frame>,
    /// A font output, `"plugin/output"`; without one the default font.
    #[serde(default)]
    pub font: Option<String>,
    /// Text size in game pixels.
    #[serde(default = "font_size")]
    pub font_size: f32,
    /// A font output for dialogs; otherwise `font`.
    #[serde(default)]
    pub dialog_font: Option<String>,
    /// Dialog text height in game pixels; otherwise `font_size`.
    #[serde(default)]
    pub dialog_font_size: Option<f32>,
}

fn font_size() -> f32 {
    8.0
}

/// What a menu is for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MenuKind {
    /// The items carried, in a grid; A and B put the chosen one on that button.
    Inventory { columns: u32 },
    /// The game stops; resume, start over or stop playing.
    Pause,
}

/// A menu and the button that opens it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Menu {
    pub kind: MenuKind,
    pub title: String,
    /// The game button that opens and closes it (see [`crate::input::BUTTONS`]).
    pub opens_with: String,
}

/// Text shown a page at a time in a box at the bottom of the screen; the game waits meanwhile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialog {
    #[serde(default)]
    pub speaker: Option<String>,
    pub pages: Vec<String>,
}

/// The sounds a played character makes, each a sound asset, `"plugin/output"`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundSet {
    #[serde(default)]
    pub hurt: Option<String>,
    #[serde(default)]
    pub pickup: Option<String>,
    #[serde(default)]
    pub door: Option<String>,
    /// A menu opening.
    #[serde(default)]
    pub menu: Option<String>,
}

/// What a played character's screen and buttons are: each a slot, empty for the built-in one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kit {
    #[serde(default)]
    pub hud: Option<Slot<HudLayout>>,
    #[serde(default)]
    pub ui: Option<Slot<Theme>>,
    #[serde(default)]
    pub menus: Option<Vec<Slot<Menu>>>,
    #[serde(default)]
    pub input: Option<Slot<InputMap>>,
    #[serde(default)]
    pub sounds: Option<Slot<SoundSet>>,
}

impl Kit {
    pub fn is_empty(&self) -> bool {
        *self == Kit::default()
    }
}

pub const HUD: &str = "hud";
pub const UI: &str = "ui";
pub const INVENTORY: &str = "inventory";
pub const PAUSE: &str = "pause";

pub fn builtin(id: &str) -> OutputRef {
    OutputRef::new(BUILTIN, id)
}

/// The built-in HUD: the item buttons bottom left, the action above them, health top left.
pub fn default_hud() -> HudLayout {
    let at = Widget::new;
    let button = |label: &str, button: &str| WidgetKind::Button {
        label: label.into(),
        button: button.into(),
    };
    HudLayout {
        widgets: vec![
            at(
                WidgetKind::ItemSlot(ItemSlot::B),
                Anchor::BottomLeft,
                (8, 8),
            ),
            at(
                WidgetKind::ItemSlot(ItemSlot::A),
                Anchor::BottomLeft,
                (38, 8),
            ),
            at(button("Dodge", "Dodge"), Anchor::BottomLeft, (68, 8)),
            at(button("Items", "Menu"), Anchor::BottomLeft, (104, 8)),
            at(WidgetKind::Action, Anchor::BottomLeft, (8, 46)),
            at(
                WidgetKind::Health {
                    per_heart: Some(2.0),
                },
                Anchor::TopLeft,
                (8, 8),
            ),
            Widget {
                format: Some("Lives {n}".into()),
                ..at(WidgetKind::Lives, Anchor::TopLeft, (8, 20))
            },
            at(WidgetKind::Timer, Anchor::Top, (0, 8)),
        ],
        words: Words::default(),
    }
}

pub fn default_theme() -> Theme {
    Theme {
        panel: Rgba(10, 10, 15, 210),
        border: Rgba(217, 217, 230, 140),
        text: Rgba(242, 242, 242, 255),
        dim: Rgba(242, 242, 242, 140),
        accent: Rgba(255, 219, 89, 255),
        shade: Rgba(0, 0, 0, 115),
        frame: None,
        font: None,
        font_size: font_size(),
        dialog_font: None,
        dialog_font_size: None,
    }
}

pub(crate) fn builtins() -> Vec<Output> {
    vec![
        Output::Hud {
            id: HUD.into(),
            hud: Slot::Inline(default_hud()),
        },
        Output::Ui {
            id: UI.into(),
            ui: Slot::Inline(default_theme()),
        },
        Output::Menu {
            id: INVENTORY.into(),
            menu: Slot::Inline(Menu {
                kind: MenuKind::Inventory { columns: 6 },
                title: "Items".into(),
                opens_with: "Menu".into(),
            }),
        },
        Output::Menu {
            id: PAUSE.into(),
            menu: Slot::Inline(Menu {
                kind: MenuKind::Pause,
                title: "Paused".into(),
                opens_with: "Pause".into(),
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_write_their_number_their_own_way() {
        let lives = Widget {
            format: Some("x{n}".into()),
            ..Widget::new(WidgetKind::Lives, Anchor::TopLeft, (0, 0))
        };
        assert_eq!(lives.text(4), "x4");
        let timer = Widget::new(WidgetKind::Timer, Anchor::Top, (0, 0));
        assert_eq!(timer.text(65), "1:05");
        let coins = Widget::new(
            WidgetKind::Counter {
                item: "coin".into(),
            },
            Anchor::Top,
            (0, 0),
        );
        assert_eq!(coins.text(12), "12");
        assert_eq!(template_letters("Got {item} x{n}"), "Got  x");
    }

    #[test]
    fn old_layouts_get_the_default_words_and_new_ones_change_some() {
        let old: HudLayout = ron::from_str("(widgets: [(kind: Lives)])").unwrap();
        assert_eq!(old.words, Words::default());
        assert_eq!(old.widgets[0].text(3), "3");
        let new: HudLayout =
            ron::from_str(r#"(widgets: [], words: (got: "", win: "YOU WIN"))"#).unwrap();
        assert_eq!(new.words.got, "");
        assert_eq!(new.words.win, "YOU WIN");
        assert_eq!(new.words.resume, "Resume");
    }

    #[test]
    fn old_themes_inherit_dialog_settings() {
        let theme: Theme = ron::from_str(
            r#"(
            panel: (0, 0, 0, 255), border: (0, 0, 0, 255), text: (255, 255, 255, 255),
            dim: (128, 128, 128, 255), accent: (255, 255, 0, 255), shade: (0, 0, 0, 128),
            font: Some("sm64/hud"), font_size: 16.0,
        )"#,
        )
        .unwrap();
        assert_eq!(theme.dialog_font, None);
        assert_eq!(theme.dialog_font_size, None);
        assert_eq!(theme.font.as_deref(), Some("sm64/hud"));
        assert_eq!(theme.font_size, 16.0);
    }

    #[test]
    fn dialog_font_settings_round_trip() {
        let theme = Theme {
            dialog_font: Some("sm64/dialog".into()),
            dialog_font_size: Some(8.0),
            ..default_theme()
        };
        let saved = ron::to_string(&theme).unwrap();
        assert_eq!(ron::from_str::<Theme>(&saved).unwrap(), theme);
    }
}

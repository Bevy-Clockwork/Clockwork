//! The menus: each opens and closes with its button (an inventory with Menu, a pause menu with
//! Pause), and Back closes any. The inventory is a grid moved through with the directions,
//! where A and B put the chosen item on that button; the pause menu is a list.

use bevy::{picking::hover::Hovered, prelude::*};
use clockwork_core::{
    item::{Category, Slot},
    ui::{Menu, MenuKind},
};

use super::{HudRequest, HudRequests, ItemView, ShownHud, ShownScreen};
use crate::input::{GameButton, GameInput};

/// How many entries a pause menu has: resume, start over, stop playing.
pub const PAUSE_ENTRIES: usize = 3;

/// Which menu is open and what it points at.
#[derive(Resource, Debug, Default)]
pub struct MenuState {
    /// The open menu, as an index into the screen's menus.
    pub open: Option<usize>,
    pub focus: usize,
    /// Set by the host (the remote control): open or close the first menu of this kind,
    /// `"Inventory"` or `"Pause"`.
    pub toggle: Option<String>,
}

impl MenuState {
    /// Whether the game should stand still.
    pub fn paused(&self) -> bool {
        self.open.is_some()
    }

    fn switch(&mut self, menu: usize) {
        self.open = if self.open == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.focus = 0;
    }
}

/// A grid cell or list entry: hovering it with the pointer points the menu at it.
#[derive(Component, Debug)]
pub struct MenuTile(pub usize);

/// A button that puts the chosen item on a slot (or takes it off).
#[derive(Component, Debug)]
pub struct MenuSlotButton(pub Slot);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuMove {
    Up,
    Down,
    Left,
    Right,
}

/// The menu shows weapons first, then what helps getting around, then the rest.
pub fn category_order(category: Category) -> u8 {
    match category {
        Category::Weapon => 0,
        Category::Equipment => 1,
        Category::Tool => 2,
        Category::Consumable => 3,
        Category::Quest => 4,
    }
}

/// Where the focus goes in a grid of `len` items `columns` wide. Moves stop at the edges.
pub fn navigate(focus: usize, len: usize, columns: usize, step: MenuMove) -> usize {
    if len == 0 {
        return 0;
    }
    let columns = columns.max(1);
    let focus = focus.min(len - 1);
    let column = focus % columns;
    match step {
        MenuMove::Left if column > 0 => focus - 1,
        MenuMove::Right if column + 1 < columns && focus + 1 < len => focus + 1,
        MenuMove::Up if focus >= columns => focus - columns,
        MenuMove::Down if focus + columns < len => focus + columns,
        // Down into a shorter last row lands on its last item.
        MenuMove::Down if focus / columns < (len - 1) / columns => len - 1,
        _ => focus,
    }
}

/// What pressing a slot's button on an item asks for: it goes on the slot, or comes off it if it
/// is there already.
pub fn choose(item: &ItemView, slot: Slot) -> Option<HudRequest> {
    if !item.can_equip {
        return None;
    }
    Some(if item.on == Some(slot) {
        HudRequest::Unequip(slot)
    } else {
        HudRequest::Equip(slot, item.id.clone())
    })
}

/// The kind name of a menu, as [`MenuState::toggle`] names it.
fn kind_name(menu: &Menu) -> &'static str {
    match menu.kind {
        MenuKind::Inventory { .. } => "Inventory",
        MenuKind::Pause => "Pause",
    }
}

/// Opens and closes the menus and answers their buttons, the pointer's included.
#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn menu_system(
    input: Res<GameInput>,
    shown: Res<ShownHud>,
    flow: Res<ShownScreen>,
    mut state: ResMut<MenuState>,
    mut requests: ResMut<HudRequests>,
    mouse: Res<ButtonInput<MouseButton>>,
    tiles: Query<(&Hovered, &MenuTile)>,
    buttons: Query<(&Hovered, &MenuSlotButton)>,
) {
    // A full screen owns the buttons, even when a dialog or menu was already open.
    if flow.view.is_some() {
        state.open = None;
        state.toggle = None;
        return;
    }
    let Some(screen) = &shown.screen else {
        state.open = None;
        state.toggle = None;
        return;
    };
    // An open dialog takes the buttons: A, Jump or Back turns its page.
    if shown.dialog.is_some() {
        if [GameButton::A, GameButton::Jump, GameButton::Back]
            .into_iter()
            .any(|b| input.pressed(b))
            || mouse.just_pressed(MouseButton::Left)
        {
            requests.0.push(HudRequest::NextPage);
        }
        return;
    }
    let menus = &screen.menus;
    if state.open.is_some_and(|i| i >= menus.len()) {
        state.open = None;
    }
    if let Some(kind) = state.toggle.take()
        && let Some(i) = menus.iter().position(|m| kind_name(m) == kind)
    {
        state.switch(i);
        return;
    }
    // A menu's own button opens it, or closes it; Back closes any.
    for (i, menu) in menus.iter().enumerate() {
        if GameButton::named(&menu.opens_with).is_some_and(|b| input.pressed(b))
            && (state.open.is_none() || state.open == Some(i))
        {
            state.switch(i);
            return;
        }
    }
    let Some(open) = state.open else {
        return;
    };
    if input.pressed(GameButton::Back) {
        state.open = None;
        return;
    }
    let clicked = mouse.just_pressed(MouseButton::Left);
    for (hovered, tile) in &tiles {
        if hovered.0 {
            state.focus = tile.0;
        }
    }
    match menus[open].kind {
        MenuKind::Inventory { columns } => {
            let Some(hud) = &shown.hud else {
                return;
            };
            let items = hud.menu_items();
            for (button, step) in [
                (GameButton::Up, MenuMove::Up),
                (GameButton::Down, MenuMove::Down),
                (GameButton::Left, MenuMove::Left),
                (GameButton::Right, MenuMove::Right),
            ] {
                if input.pressed(button) {
                    state.focus = navigate(state.focus, items.len(), columns as usize, step);
                }
            }
            let Some(item) = items.get(state.focus.min(items.len().saturating_sub(1))) else {
                return;
            };
            let mut slots: Vec<Slot> = [(GameButton::A, Slot::A), (GameButton::B, Slot::B)]
                .into_iter()
                .filter(|(button, _)| input.pressed(*button))
                .map(|(_, slot)| slot)
                .collect();
            if clicked {
                slots.extend(
                    buttons
                        .iter()
                        .filter(|(hovered, _)| hovered.0)
                        .map(|(_, button)| button.0),
                );
            }
            requests
                .0
                .extend(slots.into_iter().filter_map(|slot| choose(item, slot)));
        }
        MenuKind::Pause => {
            let len = PAUSE_ENTRIES;
            if input.pressed(GameButton::Up) {
                state.focus = state.focus.saturating_sub(1);
            }
            if input.pressed(GameButton::Down) {
                state.focus = (state.focus + 1).min(len - 1);
            }
            let chosen = input.pressed(GameButton::A)
                || input.pressed(GameButton::Jump)
                || (clicked && tiles.iter().any(|(h, t)| h.0 && t.0 == state.focus));
            if chosen {
                match state.focus {
                    0 => {}
                    1 => requests.0.push(HudRequest::Restart),
                    _ => requests.0.push(HudRequest::Stop),
                }
                state.open = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(on: Option<Slot>, can_equip: bool) -> ItemView {
        ItemView {
            id: "bow".into(),
            name: "Bow".into(),
            description: String::new(),
            category: Category::Weapon,
            owned: true,
            count: 1,
            max: 1,
            on,
            can_equip,
        }
    }

    #[test]
    fn moves_stay_inside_the_grid() {
        assert_eq!(navigate(0, 8, 6, MenuMove::Left), 0);
        assert_eq!(navigate(0, 8, 6, MenuMove::Up), 0);
        assert_eq!(navigate(5, 8, 6, MenuMove::Right), 5);
        assert_eq!(navigate(1, 8, 6, MenuMove::Down), 7);
        assert_eq!(navigate(7, 8, 6, MenuMove::Right), 7);
        assert_eq!(navigate(7, 8, 6, MenuMove::Up), 1);
        assert_eq!(navigate(3, 0, 6, MenuMove::Down), 0);
        assert_eq!(navigate(0, 8, 4, MenuMove::Down), 4);
    }

    #[test]
    fn down_from_the_last_row_stays() {
        assert_eq!(navigate(6, 8, 6, MenuMove::Down), 6);
        assert_eq!(navigate(2, 3, 6, MenuMove::Down), 2);
    }

    #[test]
    fn a_slot_button_equips_or_takes_off() {
        assert_eq!(
            choose(&item(None, true), Slot::B),
            Some(HudRequest::Equip(Slot::B, "bow".into()))
        );
        assert_eq!(
            choose(&item(Some(Slot::B), true), Slot::B),
            Some(HudRequest::Unequip(Slot::B))
        );
        assert_eq!(choose(&item(None, false), Slot::A), None);
    }

    #[test]
    fn a_menu_opens_and_closes_with_its_button() {
        let mut state = MenuState::default();
        state.switch(1);
        assert_eq!(state.open, Some(1));
        assert!(state.paused());
        state.switch(1);
        assert_eq!(state.open, None);
    }
}

//! The macOS menu bar.
//!
//! Every Bridge item carries a command id from `src/keymap.ts` and the
//! accelerator that table declares, so a menu pick and a chord reach the same
//! handler in the webview — the menu is a second way in, never a second
//! implementation. The predefined Edit items are not decoration: a custom
//! macOS menu without them takes copy and paste away from the webview.

use tauri::{
    menu::{Menu, MenuEvent, MenuItem, MenuItemBuilder, SubmenuBuilder},
    AppHandle, Emitter, Runtime,
};

/// The event the webview listens on. Mirrors `MENU_COMMAND_EVENT` in
/// `src/keymap.ts`.
pub const MENU_COMMAND_EVENT: &str = "bridge-menu-command";

/// Command id, menu label, accelerator. Mirrors the keymap entries that name a
/// submenu; the ids are the contract, the labels are Apple's title casing.
const APP_COMMANDS: &[Command] = &[Command("open-settings", "Settings…", "CmdOrCtrl+,")];
const FILE_COMMANDS: &[Command] = &[
    Command("new-chat", "New Chat", "CmdOrCtrl+N"),
    Command("new-project", "New Project", "Shift+CmdOrCtrl+N"),
    Command("interrupt-turn", "Interrupt Turn", "CmdOrCtrl+."),
];
const VIEW_COMMANDS: &[Command] = &[
    Command("toggle-sidebar", "Toggle Sidebar", "CmdOrCtrl+B"),
    Command("toggle-fullscreen", "Fullscreen Layout", "Alt+CmdOrCtrl+F"),
    // Zoom, spelled the way `src/keymap.ts` spells it. The step itself is the
    // webview's to take, in `src/zoom.ts`.
    Command("zoom-in", "Zoom In", "CmdOrCtrl+="),
    Command("zoom-out", "Zoom Out", "CmdOrCtrl+-"),
    Command("zoom-reset", "Actual Size", "CmdOrCtrl+0"),
];
const DOCK_COMMANDS: &[Command] = &[
    Command("toggle-dock", "Toggle Dock", "Alt+CmdOrCtrl+0"),
    Command("expand-dock", "Expand Dock", "Alt+CmdOrCtrl+Enter"),
];
const GO_COMMANDS: &[Command] = &[
    Command("open-recall", "Search This Chat", "CmdOrCtrl+K"),
    Command("open-projects", "Projects", "Shift+CmdOrCtrl+P"),
];
const CHAT_COMMANDS: &[Command] = &[
    Command("next-chat", "Next Chat", "Alt+CmdOrCtrl+Down"),
    Command("previous-chat", "Previous Chat", "Alt+CmdOrCtrl+Up"),
];
const HELP_COMMANDS: &[Command] = &[Command("show-shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")];

#[derive(Clone, Copy)]
struct Command(&'static str, &'static str, &'static str);

fn groups() -> [&'static [Command]; 7] {
    [
        APP_COMMANDS,
        FILE_COMMANDS,
        VIEW_COMMANDS,
        DOCK_COMMANDS,
        GO_COMMANDS,
        CHAT_COMMANDS,
        HELP_COMMANDS,
    ]
}

/// True for an id this module put in the menu. Predefined items — quit, copy,
/// minimize — own their own ids and are handled by the platform, so they must
/// not reach the webview as commands.
fn is_command(id: &str) -> bool {
    groups()
        .iter()
        .any(|group| group.iter().any(|command| command.0 == id))
}

fn items<R: Runtime>(
    handle: &AppHandle<R>,
    commands: &[Command],
) -> tauri::Result<Vec<MenuItem<R>>> {
    commands
        .iter()
        .map(|command| {
            MenuItemBuilder::new(command.1)
                .id(command.0)
                .accelerator(command.2)
                .build(handle)
        })
        .collect()
}

fn refs<R: Runtime>(items: &[MenuItem<R>]) -> Vec<&dyn tauri::menu::IsMenuItem<R>> {
    items
        .iter()
        .map(|item| item as &dyn tauri::menu::IsMenuItem<R>)
        .collect()
}

pub fn build<R: Runtime>(handle: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_items = items(handle, APP_COMMANDS)?;
    let file_items = items(handle, FILE_COMMANDS)?;
    let view_items = items(handle, VIEW_COMMANDS)?;
    let dock_items = items(handle, DOCK_COMMANDS)?;
    let go_items = items(handle, GO_COMMANDS)?;
    let chat_items = items(handle, CHAT_COMMANDS)?;
    let help_items = items(handle, HELP_COMMANDS)?;

    let app = SubmenuBuilder::new(handle, "Bridge")
        .about(None)
        .separator()
        .items(&refs(&app_items))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = SubmenuBuilder::new(handle, "File")
        .items(&refs(&file_items))
        .separator()
        .close_window()
        .build()?;
    // Undo through Select All are what make the webview a text field. Losing
    // them is the classic cost of replacing the default macOS menu.
    let edit = SubmenuBuilder::new(handle, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view = SubmenuBuilder::new(handle, "View")
        .items(&refs(&view_items))
        .separator()
        .items(&refs(&dock_items))
        .build()?;
    let go = SubmenuBuilder::new(handle, "Go")
        .items(&refs(&go_items))
        .separator()
        .items(&refs(&chat_items))
        .build()?;
    let window = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .maximize()
        .build()?;
    let help = SubmenuBuilder::new(handle, "Help")
        .items(&refs(&help_items))
        .build()?;

    Menu::with_items(
        handle,
        &[&app, &file, &edit, &view, &go, &window, &help],
    )
}

/// Forward a Bridge menu pick to the webview, which dispatches it exactly as
/// it dispatches the same command's chord.
pub fn dispatch<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if is_command(id) {
        let _ = app.emit(MENU_COMMAND_EVENT, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_command_is_forwarded_and_nothing_else_is() {
        assert!(is_command("new-chat"));
        assert!(is_command("show-shortcuts"));
        // Predefined ids belong to the platform, not to the webview.
        assert!(!is_command("quit"));
        assert!(!is_command("copy"));
    }

    #[test]
    fn no_command_is_declared_twice() {
        let mut ids: Vec<&str> = groups()
            .iter()
            .flat_map(|group| group.iter().map(|command| command.0))
            .collect();
        let declared = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), declared, "a command id appears in two submenus");
    }

    #[test]
    fn every_accelerator_names_a_modifier() {
        for group in groups() {
            for command in group {
                assert!(
                    command.2.contains("CmdOrCtrl"),
                    "{} has no command modifier",
                    command.0
                );
            }
        }
    }
}

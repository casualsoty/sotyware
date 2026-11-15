//! The macOS application menu (ADR-037).
//!
//! A Mac app is expected to have a menu bar: the application menu (About,
//! Hide, Quit), Edit and Window. Without an Edit menu the webview receives
//! no ⌘C/⌘V/⌘X/⌘A, because macOS routes those key equivalents through the
//! menu. Tauri's default macOS menu would do that too, but its Quit item
//! terminates the app directly; sotyware's Quit is its own item that ends
//! the process through `AppControl::quit`, exactly like the tray's Quit, so
//! there is one way out with one cleanup (`release_resources` on
//! `RunEvent::Exit`). ⌘W is the predefined Close Window, which sotyware turns
//! into hide like every close (lib.rs).
//!
//! Windows and Linux keep no menu bar: the window there is a dashboard of a
//! tray app, and its shortcuts are the page's own.

use tauri::{
    AppHandle, Runtime,
    menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::AppControl;

/// The id of the menu's Quit item. Not the tray's `quit`, so neither menu's
/// handler acts on the other's events (Tauri hands every menu event to
/// every menu event handler).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MENU_QUIT: &str = "app-menu-quit";

/// What an application menu item of sotyware's own does. The predefined
/// items (copy, hide, minimize, close …) are handled by the system.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum MenuAction {
    Quit,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl MenuAction {
    fn from_menu_id(id: &str) -> Option<Self> {
        (id == MENU_QUIT).then_some(Self::Quit)
    }

    fn perform(self, app: &impl AppControl) {
        match self {
            Self::Quit => app.quit(),
        }
    }
}

/// Builds the menu bar. Called once, by the builder's `menu` hook (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let info = app.package_info();
    let name = info.name.clone();
    let about = PredefinedMenuItem::about(
        app,
        Some(&format!("About {name}")),
        Some(AboutMetadata {
            name: Some(name.clone()),
            version: Some(info.version.to_string()),
            comments: Some("Cloud storage, backups and sync for your desktop.".into()),
            ..Default::default()
        }),
    )?;
    let quit = MenuItem::with_id(
        app,
        MENU_QUIT,
        format!("Quit {name}"),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let app_menu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

/// Dispatches the menu's own items; every other id (the tray's, the
/// predefined items') is ignored here.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if let Some(action) = MenuAction::from_menu_id(event.id().as_ref()) {
        action.perform(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingApp;

    #[test]
    fn quit_is_the_only_own_item_and_quits() {
        assert_eq!(MenuAction::from_menu_id(MENU_QUIT), Some(MenuAction::Quit));
        let app = RecordingApp::default();
        MenuAction::Quit.perform(&app);
        assert_eq!(*app.calls.borrow(), vec!["quit"]);
    }

    #[test]
    fn tray_ids_are_not_menu_actions() {
        // Tauri delivers every menu event to every handler: the tray's items
        // must not be mistaken for the menu bar's, and the other way round.
        for id in ["quit", "open-dashboard", "", "app-menu"] {
            assert_eq!(MenuAction::from_menu_id(id), None, "{id}");
        }
        assert_ne!(MENU_QUIT, "quit");
    }
}

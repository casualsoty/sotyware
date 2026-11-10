//! System tray icon and its context menu.

use tauri::{
    AppHandle,
    menu::{Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const TRAY_ID: &str = "sotyware-tray";
const MENU_OPEN: &str = "open-dashboard";
const MENU_QUIT: &str = "quit";

/// What a tray menu item does.
#[derive(Debug, PartialEq, Eq)]
enum TrayAction {
    OpenDashboard,
    Quit,
}

impl TrayAction {
    fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            MENU_OPEN => Some(Self::OpenDashboard),
            MENU_QUIT => Some(Self::Quit),
            _ => None,
        }
    }
}

/// Creates the tray icon. Must be called once, from `setup`.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open Dashboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("sotyware")
        .menu(&menu)
        // Left click opens the dashboard; the menu is on right click.
        // (Linux/AppIndicator ignores this and always shows the menu.)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_icon_event);

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    } else {
        eprintln!("sotyware: no default window icon; tray icon may be invisible");
    }

    // The tray is registered with the app and lives until exit; the handle can be dropped.
    builder.build(app)?;
    Ok(())
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match TrayAction::from_menu_id(event.id().as_ref()) {
        Some(TrayAction::OpenDashboard) => crate::show_dashboard(app),
        Some(TrayAction::Quit) => app.exit(0),
        None => {}
    }
}

/// Tray icon click events are emitted on Windows (and macOS) only, not on Linux.
fn handle_tray_icon_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        crate::show_dashboard(tray.app_handle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(
            TrayAction::from_menu_id(MENU_OPEN),
            Some(TrayAction::OpenDashboard)
        );
        assert_eq!(TrayAction::from_menu_id(MENU_QUIT), Some(TrayAction::Quit));
        assert_eq!(TrayAction::from_menu_id("unknown"), None);
    }
}

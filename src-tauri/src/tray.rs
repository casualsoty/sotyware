//! System tray icon and its context menu.

use tauri::{
    AppHandle,
    menu::{IsMenuItem, Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::AppControl;

const TRAY_ID: &str = "sotyware-tray";
const MENU_OPEN: &str = "open-dashboard";
const MENU_QUIT: &str = "quit";

/// The tray menu, in display order: (menu id, label).
const MENU_ITEMS: [(&str, &str); 2] = [(MENU_OPEN, "Open Dashboard"), (MENU_QUIT, "Quit")];

/// What a tray menu item or tray icon click does.
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

    /// Left click (on release) opens the dashboard; every other click does nothing
    /// here (right click shows the menu, handled by the OS).
    fn from_icon_click(button: MouseButton, state: MouseButtonState) -> Option<Self> {
        match (button, state) {
            (MouseButton::Left, MouseButtonState::Up) => Some(Self::OpenDashboard),
            _ => None,
        }
    }

    fn perform(self, app: &impl AppControl) {
        match self {
            Self::OpenDashboard => app.show_dashboard(),
            Self::Quit => app.quit(),
        }
    }
}

/// Creates the tray icon. Must be called once, from `setup`.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let items = MENU_ITEMS
        .iter()
        .map(|(id, label)| MenuItem::with_id(app, *id, *label, true, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let refs: Vec<&dyn IsMenuItem<_>> = items.iter().map(|i| i as &dyn IsMenuItem<_>).collect();
    let menu = Menu::with_items(app, &refs)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("sotyware")
        .menu(&menu)
        // Left click opens the dashboard; the menu is on right click.
        // (Linux ignores this: the tray host decides. The ksni backend marks the
        // item as not menu-only, so hosts send a left click as Activate.)
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
    if let Some(action) = TrayAction::from_menu_id(event.id().as_ref()) {
        action.perform(app);
    }
}

/// On Linux (ksni backend) only the left click (StatusNotifierItem `Activate`) and
/// the middle click (`SecondaryActivate`) arrive here, both as button-up events,
/// and only if the tray host sends them.
fn handle_tray_icon_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button,
        button_state,
        ..
    } = event
        && let Some(action) = TrayAction::from_icon_click(button, button_state)
    {
        action.perform(tray.app_handle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingApp;

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(
            TrayAction::from_menu_id(MENU_OPEN),
            Some(TrayAction::OpenDashboard)
        );
        assert_eq!(TrayAction::from_menu_id(MENU_QUIT), Some(TrayAction::Quit));
        assert_eq!(TrayAction::from_menu_id("unknown"), None);
        assert_eq!(TrayAction::from_menu_id(""), None);
    }

    #[test]
    fn menu_shows_open_dashboard_then_quit() {
        let labels: Vec<&str> = MENU_ITEMS.iter().map(|(_, label)| *label).collect();
        assert_eq!(labels, ["Open Dashboard", "Quit"]);
    }

    #[test]
    fn every_menu_item_has_an_action_and_a_unique_kebab_case_id() {
        for (i, (id, _)) in MENU_ITEMS.iter().enumerate() {
            assert!(
                TrayAction::from_menu_id(id).is_some(),
                "menu id {id} has no action"
            );
            assert!(
                !id.is_empty()
                    && id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "menu id {id} is not lowercase kebab-case"
            );
            assert!(
                MENU_ITEMS[i + 1..].iter().all(|(other, _)| other != id),
                "menu id {id} is used twice"
            );
        }
    }

    #[test]
    fn only_left_click_release_opens_the_dashboard() {
        use MouseButton::*;
        use MouseButtonState::*;
        assert_eq!(
            TrayAction::from_icon_click(Left, Up),
            Some(TrayAction::OpenDashboard)
        );
        // Opening on press as well would show the dashboard twice per click.
        assert_eq!(TrayAction::from_icon_click(Left, Down), None);
        for state in [Up, Down] {
            assert_eq!(TrayAction::from_icon_click(Right, state), None);
            assert_eq!(TrayAction::from_icon_click(Middle, state), None);
        }
    }

    #[test]
    fn open_dashboard_shows_and_never_quits() {
        let app = RecordingApp::default();
        TrayAction::OpenDashboard.perform(&app);
        assert_eq!(*app.calls.borrow(), ["show_dashboard"]);
    }

    #[test]
    fn quit_only_quits() {
        let app = RecordingApp::default();
        TrayAction::Quit.perform(&app);
        assert_eq!(*app.calls.borrow(), ["quit"]);
    }
}

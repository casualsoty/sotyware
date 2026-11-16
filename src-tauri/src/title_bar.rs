//! The dashboard's own title bar on Windows (ADR-040).
//!
//! On Windows the dashboard has no native title bar: `setup` removes the
//! decorations of the still hidden window, and the page draws its own top
//! bar with the window controls (`src/TitleBar.svelte`), in the same colours
//! as the rest of the dashboard.
//!
//! What stays native: an undecorated Tauri window on Windows keeps the
//! system's resize border (a thin child window along the top edge, the DWM
//! frame on the other sides), its drop shadow and, on Windows 11, rounded
//! corners. tao keeps the `WS_CAPTION`, `WS_SYSMENU`, `WS_SIZEBOX`,
//! `WS_MAXIMIZEBOX` and `WS_MINIMIZEBOX` styles and only hides the frame, so
//! Aero Snap while dragging, Win+Arrow, Win+Z, Alt+Space and Alt+F4 work as
//! for any window. Dragging starts from the page's drag region
//! (`data-tauri-drag-region`, which Tauri turns into the native move loop),
//! and a double click there maximizes or restores. Not possible with a
//! title bar drawn by the page: the Snap Layouts flyout when hovering the
//! maximize button (it needs a native caption button).
//!
//! The page may drive the window only when its decorations are gone: the
//! permissions for the controls are in a capability granted here at
//! runtime, on Windows only, like hiding in tray mode (ADR-012). The page
//! asks `isDecorated()` (allowed everywhere, read-only) and draws the
//! controls only for an undecorated window. macOS and Linux keep their
//! native title bars.

use tauri::ipc::CapabilityBuilder;
use tauri::{Manager, Runtime, WebviewWindow};

use crate::DASHBOARD_LABEL;

/// The runtime capability with the window controls' permissions.
pub const CAPABILITY: &str = "window-controls";

/// What the page's title bar calls: drag and double-click (Tauri's drag
/// region script), the three buttons, and the maximized state for the
/// maximize/restore button's icon.
pub const PERMISSIONS: &[&str] = &[
    "core:window:allow-start-dragging",
    "core:window:allow-internal-toggle-maximize",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-is-maximized",
    "core:window:allow-close",
];

/// Whether this platform gets the dashboard's own title bar.
pub const fn wanted() -> bool {
    cfg!(windows)
}

/// Grants the controls and only then removes the native decorations, so a
/// window without decorations always has controls. Failures are logged and
/// leave the native title bar (the page then draws no controls).
pub fn apply<R: Runtime, M: Manager<R>>(manager: &M, window: &WebviewWindow<R>) {
    let capability = PERMISSIONS.iter().fold(
        CapabilityBuilder::new(CAPABILITY).window(DASHBOARD_LABEL),
        |c, p| c.permission(*p),
    );
    if let Err(e) = manager.add_capability(capability) {
        eprintln!("sotyware: keeping the native title bar: cannot grant the window controls: {e}");
        return;
    }
    if let Err(e) = window.set_decorations(false) {
        eprintln!("sotyware: keeping the native title bar: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_draws_its_own_title_bar() {
        assert_eq!(wanted(), cfg!(windows));
    }

    /// The page's title bar needs exactly these, and nothing that could
    /// change the window otherwise (size, position, decorations, theme).
    #[test]
    fn the_controls_need_only_their_own_permissions() {
        assert_eq!(
            PERMISSIONS,
            [
                "core:window:allow-start-dragging",
                "core:window:allow-internal-toggle-maximize",
                "core:window:allow-minimize",
                "core:window:allow-toggle-maximize",
                "core:window:allow-is-maximized",
                "core:window:allow-close",
            ]
        );
        for p in PERMISSIONS {
            assert!(!p.contains("set-"), "{p}");
        }
    }

    /// The runtime capability must not collide with the static ones or
    /// with tray mode's.
    #[test]
    fn its_capability_has_its_own_name() {
        assert_ne!(CAPABILITY, crate::TRAY_MODE_CAPABILITY);
        assert_ne!(CAPABILITY, "default");
    }
}

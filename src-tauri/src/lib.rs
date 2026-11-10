//! sotyware desktop runtime: one dashboard window plus a system tray icon.
//!
//! Lifecycle rules:
//! - Hiding the dashboard (button) only hides the window.
//! - Closing the dashboard (title-bar X, Alt+F4) is intercepted and turned into a hide.
//! - Only the tray "Quit" item terminates the process, via `AppHandle::exit(0)`.
//! - If the tray icon cannot be created, the app exits with code 1.
//! - Only one instance runs: launching the app again shows the existing dashboard
//!   (via `tauri-plugin-single-instance`) and the new process exits.

mod tray;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

/// Label of the single dashboard window, as declared in `tauri.conf.json`.
pub const DASHBOARD_LABEL: &str = "main";

/// Shows, un-minimizes and focuses the existing dashboard window.
///
/// The window is never destroyed while the app runs (close is turned into hide),
/// so this always reuses the one window instead of creating a duplicate.
pub fn show_dashboard(app: &AppHandle) {
    let Some(window) = app.get_webview_window(DASHBOARD_LABEL) else {
        eprintln!("sotyware: dashboard window '{DASHBOARD_LABEL}' not found");
        return;
    };
    if let Err(e) = window.unminimize() {
        eprintln!("sotyware: failed to unminimize dashboard: {e}");
    }
    if let Err(e) = window.show() {
        eprintln!("sotyware: failed to show dashboard: {e}");
    }
    if let Err(e) = window.set_focus() {
        eprintln!("sotyware: failed to focus dashboard: {e}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)] // only mutated on desktop
    let mut builder = tauri::Builder::default();

    // Single-instance guard. It must be the first plugin registered: its setup runs
    // inside `build()`, before the dashboard window, the tray and our `setup` exist.
    // A second launch therefore forwards its arguments to the running instance and
    // exits with code 0 without ever creating a window or a tray icon.
    // In the running instance the callback only reuses the existing dashboard.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_dashboard(app);
        }));
    }

    let app = builder
        // `setup` runs exactly once, so the tray and its handlers are registered once.
        .setup(|app| {
            // Without a tray there is no way to restore or quit a hidden dashboard,
            // so a tray failure is fatal: report it and exit instead of lingering
            // as an unreachable background process.
            if let Err(e) = tray::create(app.handle()) {
                eprintln!("sotyware: cannot create system tray icon: {e}");
                app.handle().exit(1);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == DASHBOARD_LABEL
            {
                // Closing the dashboard must not end the app: keep the window, hide it.
                api.prevent_close();
                if let Err(e) = window.hide() {
                    eprintln!("sotyware: failed to hide dashboard on close: {e}");
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build sotyware");

    app.run(|_app, event| {
        // `code: None` means the runtime wants to exit because no windows remain.
        // That must never stop the background process; only an explicit
        // `exit(code)` (tray "Quit") carries `Some(code)` and is allowed through.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}

//! sotyware desktop runtime: one dashboard window plus a system tray icon.
//!
//! Lifecycle rules:
//! - Hiding the dashboard (button) only hides the window.
//! - Closing the dashboard (title-bar X, Alt+F4) is intercepted and turned into a hide.
//! - Only the tray "Quit" item terminates the process, via `AppHandle::exit(0)`.
//! - If the tray icon cannot be created, the app exits with code 1.
//! - Only one instance runs: launching the app again shows the existing dashboard
//!   (via `tauri-plugin-single-instance`) and the new process exits.
//!
//! The decisions behind these rules are plain functions (`close_action`,
//! `exit_allowed`, `tray_failure_exit_code`, `on_second_launch`, `reveal`) so that
//! unit tests can check them without a display, a tray host or an event loop. The
//! Tauri handlers below only wire them to the runtime.

mod tray;

use tauri::{AppHandle, Manager, RunEvent, Runtime, WebviewWindow, WindowEvent};

/// Label of the single dashboard window, as declared in `tauri.conf.json`.
pub const DASHBOARD_LABEL: &str = "main";

/// Exit code of the tray "Quit" item.
const EXIT_QUIT: i32 = 0;

/// Exit code when the tray icon cannot be created.
const EXIT_TRAY_FAILED: i32 = 1;

/// The app-level actions the tray and the single-instance callback trigger.
///
/// Implemented by `AppHandle`. Unit tests use a recording double, so the dispatch
/// logic can be checked without a running app.
pub(crate) trait AppControl {
    fn show_dashboard(&self);
    fn quit(&self);
}

impl<R: Runtime> AppControl for AppHandle<R> {
    fn show_dashboard(&self) {
        show_dashboard(self);
    }

    fn quit(&self) {
        self.exit(EXIT_QUIT);
    }
}

/// The window operations `show_dashboard` needs, so `reveal` can be tested with a
/// double that records calls and injects failures.
trait DashboardWindow {
    fn unminimize(&self) -> tauri::Result<()>;
    fn show(&self) -> tauri::Result<()>;
    fn set_focus(&self) -> tauri::Result<()>;
}

impl<R: Runtime> DashboardWindow for WebviewWindow<R> {
    fn unminimize(&self) -> tauri::Result<()> {
        WebviewWindow::unminimize(self)
    }

    fn show(&self) -> tauri::Result<()> {
        WebviewWindow::show(self)
    }

    fn set_focus(&self) -> tauri::Result<()> {
        WebviewWindow::set_focus(self)
    }
}

/// Shows, un-minimizes and focuses the existing dashboard window.
///
/// The window is never destroyed while the app runs (close is turned into hide),
/// so this always reuses the one window instead of creating a duplicate.
pub fn show_dashboard<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(DASHBOARD_LABEL) else {
        eprintln!("sotyware: dashboard window '{DASHBOARD_LABEL}' not found");
        return;
    };
    reveal(&window);
}

/// Un-minimizes, shows and focuses `window`, in that order. Each step is attempted
/// even if an earlier one fails: a window that is shown but not focused is still
/// usable, so failures are only logged.
fn reveal(window: &impl DashboardWindow) {
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

/// What to do when the window manager asks to close a window.
#[derive(Debug, PartialEq, Eq)]
enum CloseAction {
    /// Keep the window and hide it; the process keeps running.
    HideInstead,
    /// Let the runtime close it.
    Allow,
}

fn close_action(label: &str) -> CloseAction {
    if label == DASHBOARD_LABEL {
        CloseAction::HideInstead
    } else {
        CloseAction::Allow
    }
}

/// Whether a runtime exit request may end the process.
///
/// `code: None` means the runtime wants to exit because no windows remain. That
/// must never stop the background process; only an explicit `exit(code)` (tray
/// "Quit", fatal tray failure) carries `Some(code)` and is allowed through.
fn exit_allowed(code: Option<i32>) -> bool {
    code.is_some()
}

/// The exit code `setup` must use after trying to create the tray, or `None` if the
/// app can continue. Without a tray there is no way to restore or quit a hidden
/// dashboard, so a tray failure is fatal instead of leaving an unreachable process.
fn tray_failure_exit_code(result: tauri::Result<()>) -> Option<i32> {
    match result {
        Ok(()) => None,
        Err(e) => {
            eprintln!("sotyware: cannot create system tray icon: {e}");
            Some(EXIT_TRAY_FAILED)
        }
    }
}

/// What the primary instance does when a second launch hands over to it.
#[cfg_attr(not(desktop), allow(dead_code))]
fn on_second_launch(app: &impl AppControl) {
    app.show_dashboard();
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
            on_second_launch(app);
        }));
    }

    let app = builder
        // `setup` runs exactly once, so the tray and its handlers are registered once.
        .setup(|app| {
            if let Some(code) = tray_failure_exit_code(tray::create(app.handle())) {
                app.handle().exit(code);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && close_action(window.label()) == CloseAction::HideInstead
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
        if let RunEvent::ExitRequested { code, api, .. } = event
            && !exit_allowed(code)
        {
            api.prevent_exit();
        }
    });
}

/// Test doubles shared by the unit tests of this crate.
#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;

    use super::AppControl;

    /// Records which `AppControl` actions were triggered, in order.
    #[derive(Default)]
    pub struct RecordingApp {
        pub calls: RefCell<Vec<&'static str>>,
    }

    impl AppControl for RecordingApp {
        fn show_dashboard(&self) {
            self.calls.borrow_mut().push("show_dashboard");
        }

        fn quit(&self) {
            self.calls.borrow_mut().push("quit");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::test_support::RecordingApp;
    use super::*;

    /// Records the window operations `reveal` performs; the named one fails.
    struct FakeWindow {
        fail: Option<&'static str>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl FakeWindow {
        fn new(fail: Option<&'static str>) -> Self {
            Self {
                fail,
                calls: RefCell::default(),
            }
        }

        fn op(&self, name: &'static str) -> tauri::Result<()> {
            self.calls.borrow_mut().push(name);
            if self.fail == Some(name) {
                Err(tauri::Error::WindowNotFound)
            } else {
                Ok(())
            }
        }
    }

    impl DashboardWindow for FakeWindow {
        fn unminimize(&self) -> tauri::Result<()> {
            self.op("unminimize")
        }
        fn show(&self) -> tauri::Result<()> {
            self.op("show")
        }
        fn set_focus(&self) -> tauri::Result<()> {
            self.op("set_focus")
        }
    }

    const REVEAL_ORDER: [&str; 3] = ["unminimize", "show", "set_focus"];

    #[test]
    fn reveal_unminimizes_then_shows_then_focuses() {
        let window = FakeWindow::new(None);
        reveal(&window);
        assert_eq!(*window.calls.borrow(), REVEAL_ORDER);
    }

    #[test]
    fn reveal_attempts_every_step_when_one_fails() {
        for failing in REVEAL_ORDER {
            let window = FakeWindow::new(Some(failing));
            reveal(&window);
            assert_eq!(
                *window.calls.borrow(),
                REVEAL_ORDER,
                "a failing {failing} must not skip the remaining steps"
            );
        }
    }

    #[test]
    fn closing_the_dashboard_hides_it() {
        assert_eq!(close_action(DASHBOARD_LABEL), CloseAction::HideInstead);
    }

    #[test]
    fn closing_another_window_is_allowed() {
        assert_eq!(close_action("other"), CloseAction::Allow);
        // Labels are case-sensitive in Tauri; only the exact dashboard label is kept.
        assert_eq!(close_action("Main"), CloseAction::Allow);
    }

    #[test]
    fn implicit_exit_is_blocked() {
        assert!(
            !exit_allowed(None),
            "no windows left must not end the process"
        );
    }

    #[test]
    fn explicit_exit_is_allowed() {
        assert!(exit_allowed(Some(EXIT_QUIT)));
        assert!(exit_allowed(Some(EXIT_TRAY_FAILED)));
    }

    #[test]
    fn tray_success_lets_the_app_continue() {
        assert_eq!(tray_failure_exit_code(Ok(())), None);
    }

    #[test]
    fn tray_failure_exits_with_code_1() {
        assert_eq!(
            tray_failure_exit_code(Err(tauri::Error::FailedToReceiveMessage)),
            Some(1)
        );
    }

    #[test]
    fn second_launch_only_shows_the_dashboard() {
        let app = RecordingApp::default();
        on_second_launch(&app);
        assert_eq!(*app.calls.borrow(), ["show_dashboard"]);
    }

    fn read_json(path: &str) -> serde_json::Value {
        let full = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{full}: {e}"))
    }

    #[test]
    fn dashboard_label_matches_tauri_conf() {
        let conf = read_json("tauri.conf.json");
        let labels: Vec<&str> = conf["app"]["windows"]
            .as_array()
            .expect("app.windows must be an array")
            .iter()
            .map(|w| w["label"].as_str().expect("every window needs a label"))
            .collect();
        assert_eq!(
            labels,
            [DASHBOARD_LABEL],
            "tauri.conf.json must declare exactly one window, labelled DASHBOARD_LABEL"
        );
    }

    #[test]
    fn capability_targets_the_dashboard_and_only_allows_hide() {
        let cap = read_json("capabilities/default.json");
        assert_eq!(cap["windows"], serde_json::json!([DASHBOARD_LABEL]));
        // Widening the frontend's permissions is a design decision (ADR-006);
        // update this test together with decisions.md.
        assert_eq!(
            cap["permissions"],
            serde_json::json!(["core:window:allow-hide"])
        );
    }
}

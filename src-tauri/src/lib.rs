//! sotyware desktop runtime: one dashboard window plus, where possible, a system
//! tray icon.
//!
//! `setup` tries to create the tray once and picks the [`OperatingMode`]:
//! - **Tray mode** (tray created): hiding the dashboard (button) only hides the
//!   window; closing it (title-bar X, Alt+F4) is turned into a hide; only the tray
//!   "Quit" item ends the process, via `AppHandle::exit(0)`.
//! - **Dashboard-only mode** (Linux without a reachable tray host): the dashboard is
//!   the only way to reach sotyware, so the frontend may not hide it and closing it
//!   ends the process, via `AppHandle::exit(0)`.
//! - Any other tray error is fatal: the app exits with code 1.
//!
//! In both modes only one instance runs: launching the app again shows the existing
//! dashboard (via `tauri-plugin-single-instance`) and the new process exits.
//!
//! Built-in features (`features`) are registered when the app is built, started in
//! `setup` once the mode is known (in both modes), and stopped when the process
//! exits. Whether each one starts comes from the stored feature preferences
//! (`storage`, SQLite), which are loaded between `build()` and `run()`, before
//! the event loop runs.
//!
//! The cloud storage accounts (`cloud::dashboard::CloudAccounts`) are loaded at
//! the same point, from their own SQLite database; the OS credential store is
//! not touched until the user acts on an account in the dashboard.
//!
//! The decisions behind these rules are plain functions (`select_mode`,
//! `close_action`, `frontend_may_hide`, `exit_allowed`, `on_second_launch`,
//! `reveal`) so that unit tests can check them without a display, a tray host or an
//! event loop. The Tauri handlers below only wire them to the runtime.

pub mod cloud;
mod features;
mod storage;
mod tray;

use std::sync::Arc;

use tauri::{
    AppHandle, Manager, RunEvent, Runtime, State, WebviewWindow, WindowEvent,
    ipc::CapabilityBuilder,
};

/// Label of the single dashboard window, as declared in `tauri.conf.json`.
pub const DASHBOARD_LABEL: &str = "main";

/// Exit code of the tray "Quit" item, and of closing the dashboard in
/// dashboard-only mode.
const EXIT_QUIT: i32 = 0;

/// Exit code when the tray icon cannot be created for a reason other than a
/// missing tray host.
const EXIT_TRAY_FAILED: i32 = 1;

/// Runtime capability that lets the dashboard hide itself. Granted only in tray
/// mode, where the tray can bring the window back (ADR-012).
const TRAY_MODE_CAPABILITY: &str = "tray-mode";
const HIDE_PERMISSION: &str = "core:window:allow-hide";

/// How sotyware runs. Chosen once in `setup` from the result of creating the tray
/// and stored as Tauri state; it never changes while the process runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperatingMode {
    /// The tray exists. sotyware runs in the background; the tray restores and quits it.
    Tray,
    /// No tray host is reachable (Linux only). The dashboard stays the one way to
    /// reach sotyware, so it is never hidden and closing it ends the process.
    DashboardOnly,
}

impl OperatingMode {
    /// The value the frontend receives from the `operating_mode` command.
    fn as_str(self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::DashboardOnly => "dashboard-only",
        }
    }
}

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
    /// End the process with `exit(EXIT_QUIT)`: nothing could bring a closed
    /// dashboard back.
    Quit,
    /// Let the runtime close it.
    Allow,
}

fn close_action(label: &str, mode: OperatingMode) -> CloseAction {
    match (label == DASHBOARD_LABEL, mode) {
        (false, _) => CloseAction::Allow,
        (true, OperatingMode::Tray) => CloseAction::HideInstead,
        (true, OperatingMode::DashboardOnly) => CloseAction::Quit,
    }
}

/// Whether the frontend gets the permission to hide the dashboard. Only the tray
/// can show a hidden dashboard again, so without one hiding is never allowed: not
/// by the button, which the frontend does not render then, nor by any other call.
fn frontend_may_hide(mode: OperatingMode) -> bool {
    mode == OperatingMode::Tray
}

/// Whether a runtime exit request may end the process.
///
/// `code: None` means the runtime wants to exit because no windows remain. That
/// must never stop the background process; only an explicit `exit(code)` (tray
/// "Quit", closing the dashboard in dashboard-only mode, fatal tray failure)
/// carries `Some(code)` and is allowed through.
fn exit_allowed(code: Option<i32>) -> bool {
    code.is_some()
}

/// The operating mode after trying to create the tray, or the exit code `setup`
/// must use if sotyware cannot run.
///
/// Only an error that `tray::unavailable_reason` recognises as "no tray host in this
/// environment" selects dashboard-only mode. Every other error is a defect or a
/// broken environment and stays fatal, as it was before dashboard-only mode existed.
fn select_mode(result: tauri::Result<()>) -> Result<OperatingMode, i32> {
    let Err(e) = result else {
        return Ok(OperatingMode::Tray);
    };
    if let Some(reason) = tray::unavailable_reason(&e) {
        eprintln!("sotyware: no system tray available ({reason}): {e}");
        eprintln!("sotyware: running in dashboard-only mode; closing the dashboard quits");
        Ok(OperatingMode::DashboardOnly)
    } else {
        eprintln!("sotyware: cannot create system tray icon: {e}");
        Err(EXIT_TRAY_FAILED)
    }
}

/// Tells the frontend the operating mode: `"tray"` or `"dashboard-only"`.
/// Allowed for the dashboard by the `allow-operating-mode` permission.
#[tauri::command]
fn operating_mode(mode: State<'_, OperatingMode>) -> &'static str {
    mode.as_str()
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
        // The set of features is fixed here; `setup` starts them, exit stops them.
        .manage(features::FeatureRegistry::with_features(features::builtin()))
        .invoke_handler(tauri::generate_handler![
            operating_mode,
            features::list_features,
            features::set_feature_enabled,
            cloud::dashboard::list_cloud_providers,
            cloud::dashboard::list_cloud_accounts,
            cloud::dashboard::add_cloud_account,
            cloud::dashboard::update_cloud_account,
            cloud::dashboard::remove_cloud_account,
            cloud::dashboard::check_cloud_account
        ])
        // `setup` runs exactly once, so the tray and its handlers are registered once,
        // and the mode is chosen once. Without a tray host the tray is not retried.
        .setup(|app| {
            match select_mode(tray::create(app.handle())) {
                Ok(mode) => {
                    if frontend_may_hide(mode) {
                        app.add_capability(
                            CapabilityBuilder::new(TRAY_MODE_CAPABILITY)
                                .window(DASHBOARD_LABEL)
                                .permission(HIDE_PERMISSION),
                        )?;
                    }
                    app.manage(mode);
                    // After the mode, so a fatal tray error never starts a feature.
                    // A failing feature is recorded in its state; the app runs on.
                    // The preferences were loaded before the event loop started,
                    // so this reads memory only.
                    if let (Some(registry), Some(prefs)) = (
                        app.try_state::<features::FeatureRegistry>(),
                        app.try_state::<features::FeaturePreferences>(),
                    ) {
                        registry.start_all(|m| prefs.enabled_at_startup(m));
                    }
                }
                Err(code) => app.handle().exit(code),
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            // The mode is managed in `setup`, before the event loop delivers window
            // events. Should it ever be missing, no tray is known to exist, so the
            // dashboard must not be hidden.
            let mode = window
                .try_state::<OperatingMode>()
                .map_or(OperatingMode::DashboardOnly, |mode| *mode);
            match close_action(window.label(), mode) {
                CloseAction::HideInstead => {
                    // Closing the dashboard must not end the app: keep the window, hide it.
                    api.prevent_close();
                    if let Err(e) = window.hide() {
                        eprintln!("sotyware: failed to hide dashboard on close: {e}");
                    }
                }
                CloseAction::Quit => {
                    // An explicit exit code, so `exit_allowed` lets it through.
                    api.prevent_close();
                    window.app_handle().exit(EXIT_QUIT);
                }
                CloseAction::Allow => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build sotyware");

    // Load the feature preferences now: after `build()` (a second launch has
    // already handed over and exited inside it, so only the primary instance
    // opens the database) and before `run()`, so the file I/O never runs on the
    // event loop. Any storage failure is kept and shown by the dashboard; the
    // features then use their defaults (ADR-015).
    let feature_ids: Vec<&'static str> = app
        .state::<features::FeatureRegistry>()
        .list()
        .iter()
        .map(|f| f.id)
        .collect();
    let store = storage::open_feature_store(app.path().app_local_data_dir());
    app.manage(features::FeaturePreferences::load(store, &feature_ids));
    // The cloud storage accounts, for the same reasons at the same point. Only
    // the account database is read here; the OS credential store is first
    // used when the user adds, changes, removes or checks an account (on
    // Linux it may show an unlock prompt), always off the event loop
    // (ADR-018). A database that cannot be opened is shown by the page.
    app.manage(Arc::new(cloud::dashboard::CloudAccounts::load(
        storage::open_account_store(app.path().app_local_data_dir()),
        Arc::new(cloud::keyring::OsCredentialStore::new()),
        cloud::s3::ClientOptions::default(),
    )));

    app.run(|app, event| match event {
        RunEvent::ExitRequested { code, api, .. } if !exit_allowed(code) => {
            api.prevent_exit();
        }
        // The process is ending (only ever through an explicit exit code): give
        // running features the chance to release their resources. Stopping here
        // is not a user choice, so no preference changes. Then wait for a save
        // in progress and refuse new ones, so none is cut off halfway.
        RunEvent::Exit => {
            if let Some(registry) = app.try_state::<features::FeatureRegistry>() {
                registry.stop_all();
            }
            if let Some(prefs) = app.try_state::<features::FeaturePreferences>() {
                prefs.close();
            }
        }
        _ => {}
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

    const MODES: [OperatingMode; 2] = [OperatingMode::Tray, OperatingMode::DashboardOnly];

    #[test]
    fn tray_mode_closing_the_dashboard_hides_it() {
        assert_eq!(
            close_action(DASHBOARD_LABEL, OperatingMode::Tray),
            CloseAction::HideInstead
        );
    }

    #[test]
    fn dashboard_only_closing_the_dashboard_quits() {
        assert_eq!(
            close_action(DASHBOARD_LABEL, OperatingMode::DashboardOnly),
            CloseAction::Quit
        );
    }

    #[test]
    fn closing_another_window_is_allowed() {
        for mode in MODES {
            assert_eq!(close_action("other", mode), CloseAction::Allow);
            // Labels are case-sensitive in Tauri; only the exact dashboard label is kept.
            assert_eq!(close_action("Main", mode), CloseAction::Allow);
        }
    }

    #[test]
    fn tray_mode_lets_the_frontend_hide_the_dashboard() {
        assert!(frontend_may_hide(OperatingMode::Tray));
    }

    #[test]
    fn dashboard_only_never_lets_the_frontend_hide_the_dashboard() {
        assert!(!frontend_may_hide(OperatingMode::DashboardOnly));
    }

    #[test]
    fn frontend_receives_the_mode_names() {
        assert_eq!(OperatingMode::Tray.as_str(), "tray");
        assert_eq!(OperatingMode::DashboardOnly.as_str(), "dashboard-only");
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
    fn tray_success_selects_tray_mode() {
        assert_eq!(select_mode(Ok(())), Ok(OperatingMode::Tray));
    }

    #[test]
    fn unexpected_tray_failure_exits_with_code_1() {
        assert_eq!(
            select_mode(Err(tauri::Error::FailedToReceiveMessage)),
            Err(1)
        );
        assert_eq!(select_mode(Err(tauri::Error::WindowNotFound)), Err(1));
    }

    /// The real errors are classified in `tray::unavailable_reason` (tested there);
    /// this checks that `select_mode` turns them into a mode, not an exit.
    #[cfg(target_os = "linux")]
    #[test]
    fn missing_tray_host_selects_dashboard_only_mode() {
        let no_watcher = ksni::Error::Watcher(zbus::fdo::Error::ServiceUnknown(
            "The name org.kde.StatusNotifierWatcher was not provided by any .service files".into(),
        ));
        let no_bus = ksni::Error::Dbus(zbus::Error::Connection(
            std::sync::Arc::new(std::io::ErrorKind::NotFound.into()),
            "unix:path=/nonexistent/bus".parse().unwrap(),
        ));
        for e in [no_watcher, no_bus, ksni::Error::WontShow] {
            let e = tauri::Error::Tray(tray_icon::Error::Ksni(e));
            assert_eq!(select_mode(Err(e)), Ok(OperatingMode::DashboardOnly));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unexpected_ksni_failure_is_not_turned_into_dashboard_only_mode() {
        let e = ksni::Error::Watcher(zbus::fdo::Error::AccessDenied("rejected".into()));
        let e = tauri::Error::Tray(tray_icon::Error::Ksni(e));
        assert_eq!(select_mode(Err(e)), Err(1));
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
    fn dashboard_starts_visible() {
        // In both modes the dashboard is on screen at startup; in dashboard-only
        // mode it is the only way to reach sotyware.
        let conf = read_json("tauri.conf.json");
        assert_eq!(
            conf["app"]["windows"][0]["visible"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn capability_targets_the_dashboard_and_only_allows_the_app_commands() {
        let cap = read_json("capabilities/default.json");
        assert_eq!(cap["windows"], serde_json::json!([DASHBOARD_LABEL]));
        // Widening the frontend's permissions is a design decision (ADR-013,
        // ADR-014, ADR-018); update this test together with decisions.md. Hiding is granted
        // at runtime, in tray mode only (TRAY_MODE_CAPABILITY).
        assert_eq!(
            cap["permissions"],
            serde_json::json!([
                "allow-operating-mode",
                "allow-list-features",
                "allow-set-feature-enabled",
                "allow-list-cloud-providers",
                "allow-list-cloud-accounts",
                "allow-add-cloud-account",
                "allow-update-cloud-account",
                "allow-remove-cloud-account",
                "allow-check-cloud-account"
            ])
        );
        assert_eq!(HIDE_PERMISSION, "core:window:allow-hide");
    }
}

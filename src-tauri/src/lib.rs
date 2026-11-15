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
//! macOS (ADR-037) is always tray mode (a menu bar icon). It also has a menu
//! bar (`app_menu`: About, Hide, Quit, Edit, Window), and clicking the Dock
//! icon shows the dashboard (`RunEvent::Reopen`).
//!
//! Built-in features (`features`) are registered when the app is built, started in
//! `setup` once the mode is known (in both modes), and stopped when the process
//! exits. Whether each one starts comes from the stored feature preferences
//! (`storage`, SQLite), which are loaded between `build()` and `run()`, before
//! the event loop runs.
//!
//! The cloud storage accounts (`cloud::dashboard::CloudAccounts`) are loaded at
//! the same point, from their own SQLite database; the OS credential store is
//! not touched until the user acts on an account in the dashboard. A cloud
//! download that is still running when the process exits is cancelled and its
//! temporary file removed (`cloud::download::CloudDownloads::shutdown`); a
//! running upload batch is cancelled and incomplete multipart uploads are
//! aborted, within a bounded wait (`cloud::upload::CloudUploads::shutdown`).
//!
//! The decisions behind these rules are plain functions (`select_mode`,
//! `close_action`, `frontend_may_hide`, `exit_allowed`, `on_second_launch`,
//! `reveal`) so that unit tests can check them without a display, a tray host or an
//! event loop. The Tauri handlers below only wire them to the runtime.

pub(crate) mod account;
mod app_menu;
pub(crate) mod backup;
pub mod cloud;
mod features;
pub(crate) mod journal;
mod storage;
#[cfg(any(test, feature = "test-profile"))]
mod test_profile;
mod tray;
pub(crate) mod updates;

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

/// What a click on the Dock icon does (macOS `RunEvent::Reopen`): the
/// hidden dashboard comes back, as from the tray's Open Dashboard.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn on_reopen(app: &impl AppControl) {
    app.show_dashboard();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A test-profile build (ADR-027) checks its profile before anything else
    // exists: no plugin, window, store or credential is touched when the
    // profile is missing or invalid, and there is no fallback to the normal
    // profile.
    #[cfg(feature = "test-profile")]
    let profile = match test_profile::from_env(std::env::var_os(test_profile::ENV)) {
        Ok(profile) => {
            eprintln!(
                "sotyware: test profile {} at {}",
                profile.run(),
                profile.root().display()
            );
            profile
        }
        Err(e) => {
            eprintln!("sotyware: test profile refused: {e}");
            std::process::exit(test_profile::EXIT_REFUSED);
        }
    };
    #[allow(unused_mut)] // only changed in a test-profile build
    let mut context = tauri::generate_context!();
    #[cfg(feature = "test-profile")]
    {
        // Its own identifier: its own single-instance lock and default
        // folders, never the installed app's. The dashboard is created in
        // `setup` with the profile's webview folder.
        let config = context.config_mut();
        config.identifier = test_profile::IDENTIFIER.to_owned();
        for window in &mut config.app.windows {
            window.create = false;
        }
    }
    #[cfg(feature = "test-profile")]
    let webview_dir = profile.webview_dir();

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
        // The native save dialog for cloud downloads, used from Rust only; no
        // capability grants the frontend a `dialog:` permission (ADR-020).
        builder = builder.plugin(tauri_plugin_dialog::init());
        // Updates (ADR-033), driven from Rust only: no capability grants the
        // frontend an `updater:` permission; the page uses sotyware's own
        // update commands, which verify a signed manifest first.
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }
    // The menu bar a Mac app is expected to have; its Quit goes through
    // `AppControl::quit` like the tray's (app_menu.rs, ADR-037).
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .menu(app_menu::build)
            .on_menu_event(app_menu::handle_menu_event);
    }

    let app = builder
        // The set of features is fixed here; `setup` starts them, exit stops them.
        .manage(features::FeatureRegistry::with_features(features::builtin()))
        // Holds no file or connection until a download starts.
        .manage(Arc::new(cloud::download::CloudDownloads::new()))
        // Its own slot: one upload batch can run beside one download.
        .manage(Arc::new(cloud::upload::CloudUploads::new()))
        // Its own slot: one file operation (copy, move, rename, delete,
        // restore) can run beside the transfers (ADR-026).
        .manage(Arc::new(cloud::operations::CloudOperations::new()))
        // Continuation tokens of searches, behind cursors (ADR-026).
        .manage(Arc::new(cloud::manage::CloudSearches::default()))
        // Its own slot: one download of several files or folders (ADR-026).
        .manage(Arc::new(cloud::bulk_download::BulkDownloads::new()))
        // Paths of files dropped on the dashboard, kept in Rust (ADR-026).
        .manage(Arc::new(cloud::drop::CloudDrops::default()))
        .invoke_handler(tauri::generate_handler![
            operating_mode,
            features::list_features,
            features::set_feature_enabled,
            cloud::dashboard::list_cloud_providers,
            cloud::dashboard::list_cloud_accounts,
            cloud::dashboard::add_cloud_account,
            cloud::dashboard::update_cloud_account,
            cloud::dashboard::remove_cloud_account,
            cloud::dashboard::check_cloud_account,
            cloud::browser::list_cloud_folder,
            cloud::download::download_cloud_file,
            cloud::download::cancel_cloud_download,
            cloud::download::retry_cloud_download,
            cloud::upload::upload_cloud_files,
            cloud::upload::cancel_cloud_upload,
            cloud::upload::retry_cloud_upload,
            cloud::operations::plan_cloud_operation,
            cloud::operations::discard_cloud_operation_plan,
            cloud::operations::start_cloud_operation,
            cloud::operations::cancel_cloud_operation,
            cloud::operations::retry_cloud_operation,
            cloud::manage::create_cloud_folder,
            cloud::manage::cloud_object_properties,
            cloud::manage::preview_cloud_file,
            cloud::manage::search_cloud_folder,
            cloud::manage::list_cloud_deleted,
            cloud::manage::share_cloud_file,
            cloud::upload::upload_cloud_folder,
            cloud::bulk_download::download_cloud_items,
            cloud::bulk_download::cancel_cloud_items_download,
            cloud::bulk_download::retry_cloud_items_download,
            cloud::places::list_cloud_places,
            cloud::places::set_cloud_favorite,
            cloud::places::record_cloud_recent,
            cloud::places::clear_cloud_recent,
            cloud::drop::watch_cloud_drops,
            cloud::drop::unwatch_cloud_drops,
            cloud::drop::upload_cloud_drop,
            cloud::recovery::list_cloud_recovery,
            cloud::recovery::inspect_cloud_recovery,
            cloud::recovery::dismiss_cloud_recovery,
            cloud::recovery::list_cloud_incomplete_uploads,
            cloud::recovery::abort_cloud_incomplete_upload,
            backup::list_backups,
            backup::save_backup_job,
            backup::remove_backup_job,
            backup::start_backup_task,
            backup::cancel_backup_task,
            backup::list_backup_snapshots,
            backup::restore_backup,
            backup::export_backup_manifest,
            backup::check_backup_protection,
            account::account_status,
            account::account_register,
            account::account_verify,
            account::account_sign_in,
            account::account_sign_out,
            account::account_set_up_sync,
            account::account_check_recovery_key,
            account::account_unlock,
            account::account_request_approval,
            account::account_approval_status,
            account::account_enrollments,
            account::account_answer_enrollment,
            account::account_devices,
            account::account_revoke_device,
            account::account_rotate_keys,
            account::account_sync,
            account::account_resolve_conflict,
            account::account_change_password,
            account::account_reset_password,
            account::account_delete,
            updates::update_status,
            updates::check_for_update,
            updates::install_update,
            updates::cancel_update,
            updates::postpone_update,
            updates::restart_after_update
        ])
        // `setup` runs exactly once, so the tray and its handlers are registered once,
        // and the mode is chosen once. Without a tray host the tray is not retried.
        .setup(move |app| {
            // What Tauri does for the windows of tauri.conf.json, but with
            // the profile's webview folder (ADR-027).
            #[cfg(feature = "test-profile")]
            {
                let config = app
                    .config()
                    .app
                    .windows
                    .iter()
                    .find(|w| w.label == DASHBOARD_LABEL)
                    .cloned()
                    .ok_or(tauri::Error::WindowNotFound)?;
                tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                    .data_directory(webview_dir.clone())
                    .build()?;
            }
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
            // Dropped files: their paths go to the drop store only; the page
            // hears names and counts if it shows a bucket (ADR-026).
            if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                if let Some(drops) = window.try_state::<Arc<cloud::drop::CloudDrops>>() {
                    drops.dropped(paths.clone());
                }
                return;
            }
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
        .build(context)
        .expect("failed to build sotyware");

    // Where the databases live, and the credential namespace. A test-profile
    // build first refuses a profile that overlaps the app's own folders
    // (those of the real identifier, whatever the build runs under).
    #[cfg(feature = "test-profile")]
    let (data_dir, namespace) = {
        let real = cloud::keyring::SERVICE;
        let path = app.path();
        let normal: Vec<std::path::PathBuf> = [
            path.local_data_dir(),
            path.data_dir(),
            path.config_dir(),
            path.cache_dir(),
        ]
        .into_iter()
        .filter_map(Result::ok)
        .map(|base| base.join(real))
        .collect();
        if let Err(e) = test_profile::check_separate(&profile, &normal) {
            eprintln!("sotyware: test profile refused: {e}");
            std::process::exit(test_profile::EXIT_REFUSED);
        }
        (Ok(profile.data_dir()), profile.namespace().clone())
    };
    #[cfg(not(feature = "test-profile"))]
    let (data_dir, namespace) = (
        app.path().app_local_data_dir(),
        cloud::keyring::CredentialNamespace::normal(),
    );
    // Private to the user before any database is created in it (Linux
    // 0700; Windows inherits %LOCALAPPDATA%'s per-user ACL). A failure is
    // logged; each store then reports its own error if it cannot open.
    if let Ok(dir) = &data_dir
        && let Err(e) = storage::prepare_data_dir(dir)
    {
        eprintln!("sotyware: cannot make the data folder private: {e}");
    }

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
    let store = storage::open_feature_store(clone_dir(&data_dir));
    app.manage(features::FeaturePreferences::load(store, &feature_ids));
    // The cloud storage accounts, for the same reasons at the same point. Only
    // the account database is read here; the OS credential store is first
    // used when the user adds, changes, removes or checks an account (on
    // Linux it may show an unlock prompt), always off the event loop
    // (ADR-018). A database that cannot be opened is shown by the page.
    // A recorder build (ADR-024) records the app's S3 requests if the
    // `SOTYWARE_REQUEST_RECORD_*` variables ask for it; before any store
    // exists. Normal builds have no such code.
    #[cfg(feature = "request-recorder")]
    cloud::recorder::start_from_env();
    app.manage(Arc::new(cloud::dashboard::CloudAccounts::load(
        storage::open_account_store(clone_dir(&data_dir)),
        Arc::new(cloud::keyring::OsCredentialStore::new(namespace.clone())),
        cloud::s3::ClientOptions::default(),
    )));
    // Favorites and recent items (ADR-026): a separate small database; an
    // unusable one only disables those lists.
    app.manage(Arc::new(cloud::places::CloudPlaces::load(
        storage::open_places_store(clone_dir(&data_dir)),
    )));
    // The operation journal (ADR-030), before anything can run: first the
    // temporary files and staging folders an earlier session recorded and
    // could not remove (only sotyware's own, by name); everything else it
    // holds waits for the recovery view. Every transfer and operation slot
    // records itself in it from now on. An unusable journal is reported by
    // the page; the work then goes on unrecorded.
    let journal = journal::Journal::load(storage::open_journal_store(clone_dir(&data_dir)));
    let cleaned = journal.clean_local();
    if cleaned > 0 {
        eprintln!(
            "sotyware: removed {cleaned} temporary download file(s) left by an earlier session"
        );
    }
    app.state::<Arc<cloud::upload::CloudUploads>>()
        .record_in(Arc::clone(&journal));
    app.state::<Arc<cloud::download::CloudDownloads>>()
        .record_in(Arc::clone(&journal));
    app.state::<Arc<cloud::bulk_download::BulkDownloads>>()
        .record_in(Arc::clone(&journal));
    app.state::<Arc<cloud::operations::CloudOperations>>()
        .record_in(Arc::clone(&journal));
    // Backup jobs and their history (ADR-031): their own database and a
    // spool folder sotyware owns. The `backup-scheduler` feature starts the
    // scheduler in `setup` through this link.
    let backups = backup::Backups::load(
        storage::open_backup_store(clone_dir(&data_dir)),
        match &data_dir {
            Ok(dir) => dir.join("backup-spool"),
            Err(_) => std::env::temp_dir().join("sotyware-backup-spool"),
        },
    );
    backups.record_in(Arc::clone(&journal));
    let _ = backup::SCHEDULER_LINK.set((
        Arc::clone(&backups),
        Arc::clone(&app.state::<Arc<cloud::dashboard::CloudAccounts>>()),
    ));
    // The optional sotyware account (ADR-032): no request is sent unless
    // the user signs in, and release builds have no service configured.
    // Same namespace as the storage credentials (a test profile's own),
    // entries under their own prefix.
    let secret_namespace = namespace;
    let account_service = account::AccountService::new(
        account::configured_server(),
        Arc::new(cloud::keyring::OsSecretStore::new(secret_namespace)),
        storage::open_account_state(clone_dir(&data_dir)),
        Arc::clone(&app.state::<Arc<cloud::dashboard::CloudAccounts>>()),
    );
    account_service.with_backups(Arc::clone(&backups));
    app.manage(account_service);
    app.manage(backups);
    app.manage(journal);
    // Updates (ADR-033): release builds have no update source configured
    // until a production key and endpoint exist; the `update-check` feature
    // starts the automatic checks in `setup` through this link.
    let updates = updates::Updates::new(
        updates::configured_source(),
        storage::open_update_state(clone_dir(&data_dir)),
    );
    let _ = updates::CHECK_LINK.set((
        Arc::clone(&updates),
        updates::Updates::current(&app.package_info().version.to_string()),
    ));
    app.manage(updates);
    // Tells the test driver which process, paths and credential namespace
    // this is; it compares them with what it set up before touching the
    // window (ADR-027). Without it the driver refuses to go on, so a failed
    // write ends the app.
    #[cfg(feature = "test-profile")]
    {
        let dir = profile.data_dir();
        let stores = [
            storage::FILE_NAME,
            storage::ACCOUNTS_FILE_NAME,
            storage::PLACES_FILE_NAME,
            storage::JOURNAL_FILE_NAME,
            storage::BACKUP_FILE_NAME,
            storage::ACCOUNT_STATE_FILE_NAME,
            storage::UPDATES_FILE_NAME,
        ]
        .map(|name| dir.join(name));
        if let Err(e) = test_profile::write_ack(&profile, &stores) {
            eprintln!("sotyware: cannot write the test profile acknowledgement: {e}");
            std::process::exit(test_profile::EXIT_REFUSED);
        }
    }

    app.run(|app, event| match event {
        RunEvent::ExitRequested { code, api, .. } if !exit_allowed(code) => {
            api.prevent_exit();
        }
        // The process is ending (only ever through an explicit exit code): give
        // running features the chance to release their resources. Stopping here
        // is not a user choice, so no preference changes. Then wait for a save
        // in progress and refuse new ones, so none is cut off halfway.
        RunEvent::Exit => release_resources(app),
        // The Dock icon was clicked (macOS).
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => on_reopen(app),
        _ => {}
    });
}

/// What every end of the process releases: running features stop, a save in
/// progress finishes and new ones are refused, transfers, operations and
/// backups are cancelled (bounded waits). Called on `RunEvent::Exit` and by
/// the updater just before it ends the process to install (Windows,
/// ADR-033). Stopping here is not a user choice, so no preference changes.
pub(crate) fn release_resources<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(registry) = app.try_state::<features::FeatureRegistry>() {
        registry.stop_all();
    }
    if let Some(prefs) = app.try_state::<features::FeaturePreferences>() {
        prefs.close();
    }
    // A running download is cancelled; its temporary file is removed
    // before the process ends (bounded wait, ADR-020).
    if let Some(downloads) = app.try_state::<Arc<cloud::download::CloudDownloads>>() {
        downloads.shutdown(cloud::download::SHUTDOWN_WAIT);
    }
    // A running upload batch is cancelled and incomplete multipart
    // uploads are aborted (bounded wait, ADR-022).
    if let Some(uploads) = app.try_state::<Arc<cloud::upload::CloudUploads>>() {
        uploads.shutdown(cloud::upload::SHUTDOWN_WAIT);
    }
    // A running download of several files is cancelled and its
    // staging folder removed (bounded wait, ADR-026).
    if let Some(bulk) = app.try_state::<Arc<cloud::bulk_download::BulkDownloads>>() {
        bulk.shutdown(cloud::bulk_download::SHUTDOWN_WAIT);
    }
    // A running file operation stops after its requests in flight
    // (bounded wait, ADR-026).
    if let Some(ops) = app.try_state::<Arc<cloud::operations::CloudOperations>>() {
        ops.shutdown(cloud::operations::SHUTDOWN_WAIT);
    }
    // A running backup task is cancelled; the scheduler stops
    // (bounded wait, ADR-031).
    if let Some(backups) = app.try_state::<Arc<backup::Backups>>() {
        backups.shutdown(backup::SHUTDOWN_WAIT);
    }
    // A running update download is cancelled; an installation that has
    // begun replacing the app in place (macOS, Linux) is waited for
    // (bounded, `updates::INSTALL_WAIT`).
    if let Some(updates) = app.try_state::<Arc<updates::Updates>>() {
        updates.shutdown();
    }
    // Last, so the aborts of a quit are recorded (bounded wait).
    #[cfg(feature = "request-recorder")]
    cloud::recorder::finish_installed();
}

/// `tauri::Error` is not `Clone`; the stores each take the directory.
fn clone_dir(dir: &tauri::Result<std::path::PathBuf>) -> tauri::Result<std::path::PathBuf> {
    match dir {
        Ok(dir) => Ok(dir.clone()),
        Err(e) => Err(tauri::Error::Io(std::io::Error::other(e.to_string()))),
    }
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

    #[test]
    fn a_dock_click_only_shows_the_dashboard() {
        let app = RecordingApp::default();
        on_reopen(&app);
        assert_eq!(*app.calls.borrow(), vec!["show_dashboard"]);
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
    fn the_content_security_policy_allows_only_bundled_code_and_data_images() {
        // Widening it is a design decision (ADR-026: `img-src data:` for the
        // raster previews Rust recognised by their bytes; never scripts,
        // frames or remote content).
        let conf = read_json("tauri.conf.json");
        assert_eq!(
            conf["app"]["security"]["csp"],
            serde_json::json!(
                "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:"
            )
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
        // ADR-014, ADR-018, ADR-019, ADR-020, ADR-022, ADR-030, ADR-031, ADR-032, ADR-033, ADR-035, ADR-036); update this test together with decisions.md. Hiding is granted
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
                "allow-check-cloud-account",
                "allow-list-cloud-folder",
                "allow-download-cloud-file",
                "allow-cancel-cloud-download",
                "allow-retry-cloud-download",
                "allow-upload-cloud-files",
                "allow-cancel-cloud-upload",
                "allow-retry-cloud-upload",
                "allow-plan-cloud-operation",
                "allow-discard-cloud-operation-plan",
                "allow-start-cloud-operation",
                "allow-cancel-cloud-operation",
                "allow-retry-cloud-operation",
                "allow-create-cloud-folder",
                "allow-cloud-object-properties",
                "allow-preview-cloud-file",
                "allow-search-cloud-folder",
                "allow-list-cloud-deleted",
                "allow-share-cloud-file",
                "allow-upload-cloud-folder",
                "allow-download-cloud-items",
                "allow-cancel-cloud-items-download",
                "allow-retry-cloud-items-download",
                "allow-list-cloud-places",
                "allow-set-cloud-favorite",
                "allow-record-cloud-recent",
                "allow-clear-cloud-recent",
                "allow-watch-cloud-drops",
                "allow-unwatch-cloud-drops",
                "allow-upload-cloud-drop",
                "allow-list-cloud-recovery",
                "allow-inspect-cloud-recovery",
                "allow-dismiss-cloud-recovery",
                "allow-list-cloud-incomplete-uploads",
                "allow-abort-cloud-incomplete-upload",
                "allow-list-backups",
                "allow-save-backup-job",
                "allow-remove-backup-job",
                "allow-start-backup-task",
                "allow-cancel-backup-task",
                "allow-list-backup-snapshots",
                "allow-restore-backup",
                "allow-export-backup-manifest",
                "allow-check-backup-protection",
                "allow-account-status",
                "allow-account-register",
                "allow-account-verify",
                "allow-account-sign-in",
                "allow-account-sign-out",
                "allow-account-set-up-sync",
                "allow-account-check-recovery-key",
                "allow-account-unlock",
                "allow-account-request-approval",
                "allow-account-approval-status",
                "allow-account-enrollments",
                "allow-account-answer-enrollment",
                "allow-account-devices",
                "allow-account-revoke-device",
                "allow-account-rotate-keys",
                "allow-account-sync",
                "allow-account-resolve-conflict",
                "allow-account-change-password",
                "allow-account-reset-password",
                "allow-account-delete",
                "allow-update-status",
                "allow-check-for-update",
                "allow-install-update",
                "allow-cancel-update",
                "allow-postpone-update",
                "allow-restart-after-update",
                // The theme picker (ADR-035) asks the window to follow it.
                "core:window:allow-set-theme"
            ])
        );
        assert_eq!(HIDE_PERMISSION, "core:window:allow-hide");
    }
}

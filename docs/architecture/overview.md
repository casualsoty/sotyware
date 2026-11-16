# Architecture overview

This describes the architecture **as implemented today**. Nothing here is planned
work; for that, see [the roadmap](../status/roadmap.md). Where code lives:
[repository-map.md](repository-map.md).

## Shape

sotyware is one Tauri 2 process with:

- **one dashboard window** (label `main`) rendering a Svelte 5 single-page app, and
- **one system tray icon** with an "Open Dashboard" / "Quit" menu, where the
  environment has a tray host, and
- a **registry of built-in features**: native Rust modules that start with
  the app, can be switched on and off from the dashboard, and stop at exit
  ([features.md](features.md)): `backup-scheduler` (scheduled backups) and
  `update-check` (automatic update checks). The user's choice per feature is stored in a local SQLite
  database and survives restarts ([features.md](features.md#persistence)), and
- a **Cloud Storage** section in the dashboard that manages storage accounts
  at S3-compatible providers (list, add, edit, check the connection, remove).
  Keys live only in the OS credential store; account settings in a second
  SQLite database ([cloud-storage.md](cloud-storage.md#dashboard)).

In **tray mode** the process keeps running in the background. The window is
hidden and shown, never destroyed, and only the tray "Quit" ends the process. On
Linux without a reachable tray host it runs in **dashboard-only mode**: no tray,
the dashboard cannot be hidden, and closing it ends the process. The mode is
chosen once at startup ([lifecycle.md](lifecycle.md#operating-modes)).

```
                ┌──────────────────────── sotyware process ───────────────────────┐
 2nd launch ──► │ tauri-plugin-single-instance ──► show_dashboard()               │
 (exits 0)      │                                        ▲                        │
                │ tray (tray.rs) ── Open Dashboard ──────┘   Quit ──► exit(0)     │
                │                                                                 │
                │ before run(): storage.rs ─► FeaturePreferences::load (state)    │
                │               (preferences.sqlite3, app local data folder)      │
                │               storage.rs ─► CloudAccounts::load (state)         │
                │               (cloud-accounts.sqlite3; no credential read)      │
                │ setup: tray::create() → select_mode() → OperatingMode (state)   │
                │        → FeatureRegistry::start_all(stored choice or default)   │
                │                                                                 │
                │ window "main" ◄── CloseRequested → tray mode: hide()  (lib.rs)  │
                │                                   dashboard-only: exit(0)       │
                │   └ WebView: Svelte app (src/) ── invoke("operating_mode")      │
                │                                  getCurrentWindow().hide()      │
                │                                  (allowed in tray mode only)    │
                │                                  invoke("list_features")        │
                │                                  invoke("set_feature_enabled")  │
                │                                   └► switch, then save choice   │
                │                                  invoke("…_cloud_account(s)")   │
                │                                   └► spawn_blocking: SQLite,    │
                │                                      OS credential store; check │
                │                                      → HeadBucket (network)     │
                │                                                                 │
                │ RunEvent::ExitRequested { code: None } → prevent_exit()         │
                │ RunEvent::Exit → FeatureRegistry::stop_all(), prefs.close()     │
                └─────────────────────────────────────────────────────────────────┘
```

## Components and boundaries

| Component | File | Responsibility |
| --- | --- | --- |
| Binary entry point | `src-tauri/src/main.rs` | Calls `sotyware_lib::run()`. Nothing else. |
| Runtime and lifecycle | `src-tauri/src/lib.rs` | Builds the Tauri app: registers the single-instance plugin, creates the tray in `setup` and picks the `OperatingMode`, turns window close into hide (tray mode) or exit (dashboard-only mode), adds the hide capability in tray mode only, serves the `operating_mode` command, blocks implicit exits. Owns `show_dashboard()`, `DASHBOARD_LABEL` and the `AppControl` trait (show dashboard / quit). The lifecycle decisions are plain functions (`select_mode`, `close_action`, `frontend_may_hide`, `exit_allowed`, `on_second_launch`, `reveal`) that the handlers call, so they are unit-tested ([ADR-009](decisions.md#adr-009-lifecycle-decisions-are-plain-functions-behind-small-traits)). |
| Built-in features | `src-tauri/src/features/` | `Feature` trait and metadata, the `builtin()` list, `FeatureRegistry` (registration, lifecycle, error isolation), `FeaturePreferences` and the `PreferenceStore` trait (stored choices: startup decision, save after a switch, failure policy), the `list_features` and `set_feature_enabled` commands, and the two features (`backup.rs`, `updates.rs`). Started from `setup`, stopped on `RunEvent::Exit` ([features.md](features.md), [ADR-014](decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry)). |
| Storage | `src-tauri/src/storage.rs` | `SqliteStore`, the `PreferenceStore` implementation: opens `preferences.sqlite3` in the app's local data folder, checks it, runs the schema migrations (`MIGRATIONS`, `PRAGMA user_version`), loads and saves feature choices. The only code that uses SQLite (`rusqlite`, bundled) ([features.md](features.md#persistence), [ADR-015](decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)). |
| Cloud storage | `src-tauri/src/cloud/` | Provider profiles (data), storage accounts without secrets, credentials by account id in the OS credential store (`OsCredentialStore`, ADR-017), and `S3Store`, the one S3 adapter (`aws-sdk-s3`) with end-to-end SHA-256 and a transfer queue per account. Account configurations in `cloud-accounts.sqlite3` through `SqliteAccountStore` in `storage.rs`. `dashboard.rs`: `CloudAccounts` (managed state) and the six account commands; `browser.rs`: `list_cloud_folder`; `download.rs`: `CloudDownloads` (managed state, one download at a time) and the three download commands, with the native save dialog opened from Rust (`tauri-plugin-dialog`); `upload.rs`: `CloudUploads` (managed state, one upload batch at a time in its own slot) and the three upload commands, with the native multi-file dialog opened from Rust; the browser's upload panel (`src/CloudUpload.svelte`) calls them. No other changes to objects are reachable ([ADR-022](decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust), [cloud-storage.md](cloud-storage.md#dashboard), [ADR-016](decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter), [ADR-018](decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)). |
| macOS menu bar | `src-tauri/src/app_menu.rs` | macOS only: the application, Edit and Window menus; its Quit calls `AppControl::quit` like the tray's, and the Edit menu is what gives the webview ⌘C/⌘V ([ADR-037](decisions.md#adr-037-macos-is-a-supported-build-target-keychain-a-menu-bar-separate-apple-silicon-and-intel-builds)). A Dock click (`RunEvent::Reopen`) shows the dashboard. |
| System tray | `src-tauri/src/tray.rs` | Creates the tray icon and menu from `MENU_ITEMS`, maps menu ids and icon clicks to `TrayAction`, which calls `AppControl` (implemented by `AppHandle`: `show_dashboard()` or `exit(0)`). `unavailable_reason()` tells a missing Linux tray host apart from other tray errors. |
| Build script | `src-tauri/build.rs` | `tauri_build` with an app manifest that declares the app's commands (`operating_mode`, `list_features`, `set_feature_enabled`, the six cloud account commands, `list_cloud_folder`, the three download and the three upload commands), so each needs a permission. |
| Window definition | `src-tauri/tauri.conf.json` | Declares window `main` (1120×760, at least 640×480, visible at start), CSP, bundle settings (including `bundle.macOS`: minimum 10.15, hardened runtime, the .dmg layout), `identifier`. |
| Frontend permissions | `src-tauri/capabilities/default.json` | Grants window `main` exactly `allow-operating-mode`, `allow-list-features`, `allow-set-feature-enabled` and `allow-list-cloud-providers`, `allow-list-cloud-accounts`, `allow-add-cloud-account`, `allow-update-cloud-account`, `allow-remove-cloud-account`, `allow-check-cloud-account`, `allow-list-cloud-folder`, `allow-download-cloud-file`, `allow-cancel-cloud-download`, `allow-retry-cloud-download`, `allow-upload-cloud-files`, `allow-cancel-cloud-upload`, `allow-retry-cloud-upload`. (…and one `allow-<command>` per command listed below), plus `core:window:allow-set-theme` (the theme picker, ADR-035) and `core:window:allow-is-decorated` (whether to draw the page's own title bar, ADR-040). No `dialog:`, `fs:` or `updater:` permission. `core:window:allow-hide` is added at runtime, in tray mode only; the title bar's controls (`allow-start-dragging`, `allow-internal-toggle-maximize`, `allow-minimize`, `allow-toggle-maximize`, `allow-is-maximized`, `allow-close`) at runtime on Windows only, capability `window-controls` (`title_bar.rs`). |
| Frontend | `index.html`, `src/main.ts`, `src/App.svelte` (shell), `src/Sidebar.svelte`, `src/Overview.svelte`, `src/Settings.svelte`, the section pages and panels (`src/Cloud*.svelte`, `src/Backup.svelte`, `src/Account.svelte`, `src/Features.svelte`, `src/Updates.svelte`, `src/Modal.svelte`), the design system (`src/styles/tokens.css`, `src/styles/base.css`, `src/Icon.svelte`, `src/theme.ts`) and the typed command wrappers (`src/*.ts`) | A desktop shell: a sidebar with the sections, the storage accounts and Settings, and the main area with the page shown and one update notification. Navigation is local state (no router). Details: [frontend.md](frontend.md) (ADR-035). |

Boundaries that hold today:

- **Frontend ↔ Rust.** The frontend reaches Rust through 83 custom commands
  and Tauri's window API. The commands are `operating_mode` (read-only, returns
  `"tray"` or `"dashboard-only"`), `list_features` (read-only),
  `set_feature_enabled` (switches one feature through the registry's lifecycle
  rules) and the cloud account commands `list_cloud_providers`,
  `list_cloud_accounts`, `add_cloud_account`, `update_cloud_account`,
  `remove_cloud_account`, `check_cloud_account`
  ([cloud-storage.md](cloud-storage.md#dashboard)), and `list_cloud_folder`
  (read-only, one page of one folder of an account's bucket;
  [cloud-storage.md](cloud-storage.md#browsing-a-bucket)), and
  `download_cloud_file`, `cancel_cloud_download`, `retry_cloud_download`
  (one file at a time to a path chosen in a save dialog that Rust opens; the
  page never sends a path; progress over a Tauri `Channel` passed with the
  command, which reaches only that call;
  [cloud-storage.md](cloud-storage.md#downloading-a-file)), and
  `upload_cloud_files`, `cancel_cloud_upload`, `retry_cloud_upload` (up to
  100 files picked in a dialog that Rust opens, uploaded one after another
  into one folder; events over a per-batch `Channel`; called by the
  browser's upload panel; [cloud-storage.md](cloud-storage.md#uploading-files)), and
  the 22 file-management commands of ADR-026: `plan_cloud_operation`,
  `discard_cloud_operation_plan`, `start_cloud_operation`,
  `cancel_cloud_operation`, `retry_cloud_operation` (copy, move, rename,
  delete, restore as reviewed plans), `create_cloud_folder`,
  `cloud_object_properties`, `preview_cloud_file`, `search_cloud_folder`,
  `list_cloud_deleted`, `share_cloud_file`, `upload_cloud_folder`,
  `download_cloud_items`, `cancel_cloud_items_download`,
  `retry_cloud_items_download`, `list_cloud_places`, `set_cloud_favorite`,
  `record_cloud_recent`, `clear_cloud_recent`, `watch_cloud_drops`,
  `unwatch_cloud_drops`, `upload_cloud_drop`
  ([cloud-storage.md](cloud-storage.md#file-management)), and the five
  recovery commands of ADR-030: `list_cloud_recovery`,
  `inspect_cloud_recovery` (reads only), `dismiss_cloud_recovery`,
  `list_cloud_incomplete_uploads`, `abort_cloud_incomplete_upload`
  ([cloud-storage.md](cloud-storage.md#operation-journal-and-recovery)),
  and the three usage commands of ADR-041: `storage_usage_overview` (reads
  measurements and computes estimates), `measure_storage_usage` (lists a
  bucket, read-only; events over a per-call `Channel`),
  `cancel_storage_usage`
  ([cloud-storage.md](cloud-storage.md#storage-usage-and-cost-estimates)),
  and the eleven backup commands of ADR-031 and ADR-041: `list_backups`,
  `save_backup_job`, `remove_backup_job`, `start_backup_task`,
  `cancel_backup_task`, `list_backup_snapshots`, `restore_backup`,
  `export_backup_manifest`, `check_backup_protection`,
  `backup_snapshot_details`, `delete_backup_snapshot`
  ([backup.md](backup.md#commands-and-page)), and the 20 account
  commands of ADR-032: `account_status`, `account_register`, `account_verify`, `account_sign_in`, `account_sign_out`, `account_set_up_sync`, `account_check_recovery_key`, `account_unlock`, `account_request_approval`, `account_approval_status`, `account_enrollments`, `account_answer_enrollment`, `account_devices`, `account_revoke_device`, `account_rotate_keys`, `account_sync`, `account_resolve_conflict`, `account_change_password`, `account_reset_password`, `account_delete`
  ([accounts.md](accounts.md)), and the five update commands of ADR-033:
  `update_status`, `check_for_update`, `install_update`, `cancel_update`,
  `postpone_update`, and `restart_after_update` (ADR-036;
  [updates.md](updates.md)); none returns a
  stored credential or a local path (a share link carries the access key
  id, never the secret). The built-in calls are Tauri's window API:
  `getCurrentWindow().hide()` (tray mode only), `setTheme()` (the theme
  chosen in Settings, ADR-035), `isDecorated()`, and on Windows only the
  title bar's `minimize()`, `toggleMaximize()`, `isMaximized()`, `close()`
  and Tauri's drag region (ADR-040). There is no other custom command, and there are **no custom
  events** (`emit`/`listen`). Adding either is an architectural change: record
  it in [decisions.md](decisions.md).
- **Platform detection lives in Rust.** Only `tray::unavailable_reason()` looks
  at the tray error. The frontend never decides behaviour by the OS; it reads
  `navigator.userAgent` only to word labels (⌘ instead of Ctrl, "menu bar",
  the credential store's name) and to guess, for the first paint only,
  whether the window has its own title bar (the window's `isDecorated()`
  decides). What differs in behaviour (`usesInstaller`
  for updates, the operating mode) comes from Rust.
- **Lifecycle lives in `lib.rs`.** Window, exit and single-instance handling are
  registered once on the builder. `tray.rs` does not decide lifecycle policy; it
  calls `show_dashboard()` or `AppHandle::exit(0)`.
- **One window.** All code that shows the dashboard looks up the existing window
  by `DASHBOARD_LABEL` and never creates one.
- **Features live behind the registry.** Lifecycle code (`lib.rs`) calls only
  `start_all` and `stop_all`, and the frontend uses the two feature commands.
  Nothing else reaches a feature, and a feature's error never affects the mode,
  the tray, the window or another feature.
- **Storage stays out of the registry.** `features::preferences` decides what a
  stored choice means; `storage.rs` is the only code that knows SQLite; the
  registry knows neither ([ADR-015](decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)).
- **Little background work.** No threads, timers, polling or retries of
  sotyware's own. Everything is driven by Tauri events and user actions.
  `set_feature_enabled` (an `async` command)
  runs on Tauri's async runtime, off the event loop, and so does the
  preference save it makes. The cloud account and folder commands run their SQLite and
  credential-store work on the runtime's blocking pool
  (`spawn_blocking`). The one thing that runs after its command returns is
  a download started from the browser (one at a time, on the async runtime,
  until it completes, fails or is cancelled; quitting cancels it), and an
  upload batch (likewise, in its own slot; quitting cancels it and aborts
  incomplete multipart uploads, waiting at most 35 s), a file operation
  and a download of several items (each in its own slot; quitting cancels
  them, waiting at most 15 s and 10 s). The
  databases are read once, before the event loop starts.
- **Network only on request, no sync, no settings.** Stored: each feature's
  enabled choice (`preferences.sqlite3`), the storage accounts'
  configurations (`cloud-accounts.sqlite3`) and their favorites and recent
  items (`cloud-places.sqlite3`), all in the app's local data
  folder, and the accounts' keys in the OS credential store. The only network
  requests are the ones "Check connection" sends (`HeadBucket`, at most one
  `ListObjectsV2` of one key, to the account's bucket) and one
  `ListObjectsV2` per folder opened, refreshed or "Load more" in the bucket
  browser, and one `GetObject` per download the user starts. The upload
  commands send `HeadObject` and upload requests only for files picked in
  their dialog, when the user chooses Upload files… in the bucket browser;
  nothing is changed otherwise or
  synchronised. The file-management commands send requests only when
  the user acts in the browser (each listed in
  [cloud-storage.md](cloud-storage.md#what-s3-means-for-each-action));
  every change to objects goes through a plan the user reviewed first.

## Security model

- CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:`
  (inline styles are needed by Svelte; `data:` images for the file previews,
  ADR-026; pinned by a test in `lib.rs`).
- Capability `default` applies to window `main` only and grants
  `allow-operating-mode`, `allow-list-features`,
  `allow-set-feature-enabled`, the six `allow-…-cloud-…` account permissions
  `allow-list-cloud-folder`, the three download and the three upload
  permissions, the 22 file-management permissions of ADR-026 and the five
  recovery permissions of ADR-030 and the nine backup permissions of
  ADR-031 and the 20 account permissions of ADR-032 (the app's 72
  commands) and nothing else, not even
  `core:default` or a `dialog:`/`fs:` permission: the save dialog for
  downloads and the file dialog for uploads are opened by Rust, and the
  page never names or receives a local path
  ([ADR-014](decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry),
  [ADR-018](decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands),
  [ADR-019](decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust),
  [ADR-020](decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel),
  [ADR-022](decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)).
- Cloud account keys travel only from the page to Rust (`add_cloud_account`,
  `update_cloud_account`) and from Rust to the OS credential store and the
  request signer; no response, log line or error message contains one. In tray mode, `setup` adds the runtime capability `tray-mode`,
  which grants window `main` `core:window:allow-hide` and nothing else
  ([ADR-013](decisions.md#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode)).
- The single-instance plugin has no JavaScript API and needs no capability.
  The dialog plugin's JavaScript commands exist but no capability grants
  them, so the page cannot open a file dialog itself.
- Any new frontend permission must be the narrowest one that works and must be
  recorded in [decisions.md](decisions.md).
- Test builds: a `test-profile` build (ADR-027; also every recorder build)
  runs only inside a test profile a desktop driver created, under its own
  identifier and credential namespace, and cannot name the app's own
  credentials (`CredentialNamespace::normal()` is not compiled in). Normal
  builds contain none of it; the packaging guard refuses such binaries
  ([ADR-027](decisions.md#adr-027-desktop-tests-run-a-test-profile-build-that-cannot-reach-the-users-data)).
- Release builds: `npm run build:release` remaps the build machine's
  directories out of the binary, and the packaging guard refuses a binary
  that still names them
  ([ADR-028](decisions.md#adr-028-release-binaries-are-built-with-the-build-machines-paths-remapped)).

## Further reading

- [lifecycle.md](lifecycle.md): window, tray and exit behaviour.
- [cloud-storage.md](cloud-storage.md) and [cloud-providers.md](cloud-providers.md):
  the cloud storage proof of concept and the provider evaluation.
- [features.md](features.md): built-in features, their lifecycle, and how to add one.
- [single-instance.md](single-instance.md): how the second launch is handled.
- [decisions.md](decisions.md): why things are the way they are.

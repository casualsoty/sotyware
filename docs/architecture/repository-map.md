# Repository map

Where things are. Generated and ignored paths are listed in
[workflow.md](../development/workflow.md#generated-and-ignored-paths).

```
.
├── AGENTS.md                 Canonical instructions for coding agents
├── CLAUDE.md                 Claude Code entry point (imports AGENTS.md)
├── README.md                 Human-facing overview and quick start
├── .github/
│   └── workflows/
│       ├── ci.yml            Automatic headless CI (Linux + Windows): check, build, fmt, clippy, test:headless
│       ├── desktop-virtual.yml           Manual: Linux virtual-display test (Xvfb + openbox)
│       └── desktop-windows-isolated.yml  Manual, experimental: Windows private-desktop test
├── docs/
│   ├── CONTEXT.md            Short session-start context for agents
│   ├── architecture/         How it works and why (this file, overview, lifecycle, features, cloud storage and providers, single instance, decisions)
│   ├── development/          Setup, commands, testing, cross-platform rules
│   └── status/               What is implemented and validated, limitations, roadmap
├── index.html                Vite HTML entry; mounts #app, loads src/main.ts
├── src/                      Frontend (Svelte 5 + TypeScript)
│   ├── main.ts               Mounts App into #app
│   ├── App.svelte            The dashboard UI: navigation (Overview, Cloud Storage) and the Overview section
│   ├── App.test.ts           Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
│   ├── CloudStorage.svelte   Cloud Storage section: account list, empty state, check, remove with confirmation
│   ├── CloudAccountForm.svelte  Add/edit form for one storage account (credential fields cleared after use)
│   ├── CloudBrowser.svelte   Browser of one account's bucket: folders, files, breadcrumbs, Up, Refresh, Load more; Download per file with progress, Cancel, Try again
│   ├── CloudBrowser.test.ts  Tests of the bucket browser and its downloads, opened from the Cloud Storage section
│   ├── CloudUpload.svelte    The browser's upload panel: one batch's progress, per-file list, Cancel, Try again, cleanup notes, summary
│   ├── CloudUpload.test.ts   Tests of the upload panel inside the browser (mocked upload commands and channels)
│   ├── CloudStorage.test.ts  Tests of the Cloud Storage section, its form and cloud.ts
│   ├── cloud.ts              Typed wrappers for the six cloud account commands, list_cloud_folder, the three download commands (progress channel) and the three upload commands (per-batch event channel); validates responses and events, CloudCommandError
│   ├── cloudMessages.ts      Labels and hints for check results and command error kinds; upload skip reasons, remote outcomes and upload error summaries
│   ├── format.ts             formatSize and shownName, shared by the browser and the upload panel
│   ├── Features.svelte       Minimal Features section of the dashboard (state + enable checkbox per feature)
│   ├── Features.test.ts      Tests of the Features section and of features.ts
│   └── features.ts           Typed wrappers for list_features / set_feature_enabled; validates responses
├── package.json              npm scripts and JS dependencies
├── package-lock.json         Pinned JS dependency tree
├── vite.config.ts            Vite: port 1420, strictPort, ignores src-tauri/, output dist/; Vitest config
├── svelte.config.js          vitePreprocess (TypeScript in .svelte)
├── tsconfig.json             Strict TypeScript settings
├── src-tauri/                Rust crate and Tauri configuration
│   ├── Cargo.toml            Crate `sotyware`, lib `sotyware_lib`, dependencies, release profile
│   ├── Cargo.lock            Pinned Rust dependency tree
│   ├── build.rs              tauri_build with an app manifest declaring the app's ten commands
│   ├── tauri.conf.json       Product name, identifier, window `main`, CSP, bundles, dev/build hooks
│   ├── capabilities/
│   │   └── default.json      Frontend permissions for window `main` (hide is added at runtime, tray mode only)
│   ├── icons/                Bundle and tray icons (Tauri placeholders)
│   ├── patches/              Dependencies built from a local copy via `[patch.crates-io]` (ADR-023)
│   │   ├── README.md         What is patched, why, how to verify the copy, when to remove it
│   │   ├── rfd-0.17.2-osstring.patch  The full diff against the published crate
│   │   └── rfd/              rfd 0.17.2 as published, `com.rs` changed: Windows dialog paths as `OsString` + tests
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, OperatingMode and its selection, lifecycle, operating_mode command, single instance, show_dashboard(), AppControl + unit tests
│       ├── tray.rs           Tray icon, menu, TrayAction, unavailable_reason() (no Linux tray host) + unit tests
│       ├── storage.rs        SqliteStore: preferences.sqlite3, schema, MIGRATIONS, PreferenceStore impl; SqliteAccountStore: cloud-accounts.sqlite3, ACCOUNT_MIGRATIONS + tests (incl. restarts with the built-in features)
│       ├── cloud/            Cloud storage: backend, the dashboard's account commands, the bucket browser, downloads and upload batches
│       │   ├── mod.rs        Module overview, re-exports SqliteAccountStore
│       │   ├── provider.rs   ProviderProfile data: backblaze-b2, hetzner, s3-compatible + tests
│       │   ├── account.rs    AccountConfig + resolve(), Accounts, AccountStore trait + tests
│       │   ├── credentials.rs  Secret, Credentials (validation, stored format), CredentialStore trait, CredentialError; test-only EnvCredentials, MemoryCredentialStore + tests
│       │   ├── keyring.rs    OsCredentialStore: Windows Credential Manager / Secret Service via keyring-core, entry names, error mapping + tests, opt-in real-backend test
│       │   ├── s3.rs         S3Store (aws-sdk-s3): check_bucket, list, list_folder (one delimited page), upload_file wrapper, download (streamed, progress, TransferControl cancel/commit, temporary file, final move), delete, SHA-256, errors + tests
│       │   ├── s3/
│       │   │   ├── upload.rs  S3Store::upload (ADR-021): first hashing pass with per-part SHA-256, destination HEAD, request bodies that re-hash and hold back their last chunk (commit point), stall timeout instead of the read timeout, multipart abort in its own task (open upload id on TransferControl; abort_incomplete_upload), HEAD verification, UploadError/RemoteOutcome, throttled UploadProgress
│       │   │   └── upload/tests.rs  Upload tests against the in-process S3 server: basics, cancellation at every stage, multipart cleanup, file changes, SDK retries, throttled and stalled links
│       │   ├── dashboard.rs  CloudAccounts (Tauri state), the six account commands, safe DTOs, CommandError, connection check, listing cursors + tests
│       │   ├── browser.rs    list_cloud_folder: folder/file DTOs, prefix and breadcrumb rules, error mapping (tests in browser/tests.rs)
│       │   ├── download.rs   download_cloud_file / cancel / retry: CloudDownloads (one slot), save dialog, progress messages, error mapping (tests in download/tests.rs)
│       │   ├── upload.rs     upload_cloud_files / cancel / retry (ADR-022): CloudUploads (one batch slot), multi-file dialog, selection checks, sequential batch with HEAD before each file, error sorting, events with seq and one final summary, retry, incomplete multipart ids in memory, bounded shutdown (tests in upload/tests.rs)
│       │   ├── tests.rs      Storage operations against the in-process S3 server
│       │   ├── test_server.rs  In-process S3 server (s3s + s3s-fs, or an in-memory listing backend with exact keys and continuation tokens; can refuse chosen operations, records them with part numbers, injects faults per operation and part: 500, 403, no answer, a callback), a throttled or stalling TCP relay (ThrottledLink), an object backend with exact keys and slow/stalled/cut bodies, temp-file helpers, the UI tests' listing and object servers (serve_listing_for_ui, serve_objects_for_ui), tests only
│       │   ├── integration.rs  Opt-in (#[ignore]) tests against real Backblaze B2 / Hetzner accounts (round trip, B2 connection check and B2 folder listing with recorded requests, B2 bucket fingerprint) + the same check and listing cases against the test server
│       │   └── integration/
│       │       ├── download.rs  Opt-in B2 download test: one generated file (SHA-256 known before upload) under sotyware-it/download-validation/<run>/, one download with the read-only key through CloudDownloads, version-exact cleanup + the same download against the test server
│       │       └── browse.rs  Opt-in B2 browse test on a temporary dataset (create under sotyware-it/browse-validation/<run>/, browse through list_folder, version-exact cleanup; kept for the UI tests with a manifest; cleanup by run id) + the same checks against the test server
│       └── features/         Built-in features
│           ├── mod.rs        Feature trait, metadata, state, builtin() list, the two feature commands (FeatureList, FeatureChange) + tests
│           ├── preferences.rs  FeaturePreferences, PreferenceStore trait: startup choice, save after a switch, failure policy + tests
│           ├── registry.rs   FeatureRegistry: registration, lifecycle table, per-feature locking, start_all/stop_all + tests
│           └── example.rs    The example feature + tests
└── tests/
    └── desktop/              Desktop tests of the release binary
        ├── check-ps-variables.ps1  Static check of the Windows test scripts (parses only, runs nothing): fails if one scope spells a variable two ways ($out/$Out are one variable in PowerShell)
        ├── cloud-ui.sh       Opt-in Linux Cloud Storage end-to-end test: Xvfb + openbox, private D-Bus session, real GNOME Keyring; phases local, hang, browse (local listing server), download (local object server, GTK save dialog), upload (local s3s-fs server, GTK multi-file dialog, stub tray watcher), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key)
        ├── cloud_ui.py       Its driver: add, check, open bucket, browse, download through the save dialog, rename, replace key, remove through AT-SPI + XTest; keyring and database checked from outside
        ├── cloud_ui_upload.py  Its upload phase (run by cloud_ui.py): batches through the GTK multi-file dialog, server state checked after each, exact-key cleanup
        ├── fake_secret_service.py  A Secret Service that never answers (cloud-ui.sh hang phase)
        ├── run.sh            Linux/X11 simulated test: private D-Bus session, runtime dir and data dir (XDG_DATA_HOME)
        ├── sni_watcher.py    Stub tray host (org.kde.StatusNotifierWatcher); --reject, --no-host
        ├── single_instance.py  Linux test driver: windows, tray menu, launches, tray failure, dashboard-only mode, feature preferences across restarts, Cloud Storage navigation and empty state, page via AT-SPI
        ├── virtual.sh        Linux: run.sh on a private Xvfb display with openbox
        ├── windows.ps1       Windows test on the real desktop: hide button, tray, close, launches, feature preferences across restarts, Cloud Storage navigation and empty state
        ├── windows-cloud-ui.ps1  Opt-in Windows Cloud Storage end-to-end test through UI Automation, real Credential Manager; -Mode local, browse (local listing server), download (local object server, Windows save dialog), upload (local s3s-fs server, Windows multi-file dialog; moves the mouse once for tray Quit), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key) or b2-download (one temporary file downloaded from Backblaze through the dashboard and the save dialog)
        └── windows-isolated.ps1  Unattended Windows test on a private desktop: window, hide button, close, launches (no tray)
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | `operating_mode` in `lib.rs`; `list_features` and `set_feature_enabled` in `features/mod.rs`; the six cloud account commands in `cloud/dashboard.rs`; `list_cloud_folder` in `cloud/browser.rs`; `download_cloud_file`, `cancel_cloud_download` and `retry_cloud_download` in `cloud/download.rs`; `upload_cloud_files`, `cancel_cloud_upload` and `retry_cloud_upload` in `cloud/upload.rs`. All are registered with `invoke_handler` in `run()` and declared in the app manifest in `build.rs` (permissions `allow-<command, with ->` in `capabilities/default.json`) |
| Where are built-in features? | `src-tauri/src/features/`: listed in `builtin()` in `mod.rs`, run by `FeatureRegistry` in `registry.rs`; how to add one: [features.md](features.md#adding-a-built-in-feature) |
| Where are feature choices stored? | Policy: `features/preferences.rs`; SQLite, schema and migrations: `storage.rs`; loaded in `run()` in `lib.rs` before the event loop; the file is `preferences.sqlite3` in the app's local data folder ([features.md](features.md#persistence)) |
| Where is cloud storage? | `src-tauri/src/cloud/`; the dashboard's commands in `cloud/dashboard.rs`, `cloud/browser.rs` and `cloud/download.rs`, its page in `src/CloudStorage.svelte` (browser: `src/CloudBrowser.svelte`, upload panel: `src/CloudUpload.svelte`; upload commands in `cloud/upload.rs`); accounts database in `storage.rs`, loaded in `run()`; design: [cloud-storage.md](cloud-storage.md), providers: [cloud-providers.md](cloud-providers.md) |
| Where is the operating mode chosen? | `select_mode()` in `lib.rs`, from `tray::create()`'s result classified by `tray::unavailable_reason()` |
| Where are events emitted or listened to? | No custom events exist. Window events: `on_window_event` in `run()`. Exit events: `app.run(...)` closure in `run()` |
| Where is tray behaviour? | `src-tauri/src/tray.rs`, created from `setup` in `lib.rs` |
| Where is "show the dashboard"? | `show_dashboard()` in `lib.rs`, used by the tray and the single-instance callback |
| Where is the window declared? | `app.windows` in `src-tauri/tauri.conf.json` (label `main`, mirrored by `DASHBOARD_LABEL`) |
| Where are frontend permissions? | `src-tauri/capabilities/default.json` (static); the runtime capability `tray-mode` in `setup` in `lib.rs` |
| Where is the CSP? | `app.security.csp` in `src-tauri/tauri.conf.json` |
| Where is the frontend? | `src/`, entered from `index.html` |
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules (cloud storage: also `cloud/tests.rs`, `cloud/s3/upload/tests.rs`, `cloud/browser/tests.rs`, `cloud/download/tests.rs`, `cloud/upload/tests.rs`, opt-in `cloud/integration.rs`, `cloud/integration/browse.rs` and `cloud/integration/download.rs`); frontend tests in `src/*.test.ts` (`App.test.ts`, `Features.test.ts`, `CloudStorage.test.ts`, `CloudBrowser.test.ts`, `CloudUpload.test.ts`); desktop tests in `tests/desktop/` (`run.sh`/`virtual.sh` Linux, `windows.ps1`, `windows-isolated.ps1` Windows; opt-in Cloud Storage end-to-end: `cloud-ui.sh`, `windows-cloud-ui.ps1`); all headless tests: `npm run test:headless`; CI workflows in `.github/workflows/` ([testing.md](../development/testing.md#ci)) |
| Where is a dependency patched? | `src-tauri/patches/` (rfd, wired in by `[patch.crates-io]` in `src-tauri/Cargo.toml`; [README](../../src-tauri/patches/README.md), [ADR-023](decisions.md#adr-023-rfd-is-patched-locally-so-windows-dialog-paths-are-read-as-osstring)) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

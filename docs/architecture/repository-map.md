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
│   ├── CloudStorage.test.ts  Tests of the Cloud Storage section, its form and cloud.ts
│   ├── cloud.ts              Typed wrappers for the six cloud account commands; validates responses, CloudCommandError
│   ├── cloudMessages.ts      Labels and hints for check results and command error kinds
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
│   ├── build.rs              tauri_build with an app manifest declaring the app's nine commands
│   ├── tauri.conf.json       Product name, identifier, window `main`, CSP, bundles, dev/build hooks
│   ├── capabilities/
│   │   └── default.json      Frontend permissions for window `main` (hide is added at runtime, tray mode only)
│   ├── icons/                Bundle and tray icons (Tauri placeholders)
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, OperatingMode and its selection, lifecycle, operating_mode command, single instance, show_dashboard(), AppControl + unit tests
│       ├── tray.rs           Tray icon, menu, TrayAction, unavailable_reason() (no Linux tray host) + unit tests
│       ├── storage.rs        SqliteStore: preferences.sqlite3, schema, MIGRATIONS, PreferenceStore impl; SqliteAccountStore: cloud-accounts.sqlite3, ACCOUNT_MIGRATIONS + tests (incl. restarts with the built-in features)
│       ├── cloud/            Cloud storage: backend and the dashboard's account commands
│       │   ├── mod.rs        Module overview, re-exports SqliteAccountStore
│       │   ├── provider.rs   ProviderProfile data: backblaze-b2, hetzner, s3-compatible + tests
│       │   ├── account.rs    AccountConfig + resolve(), Accounts, AccountStore trait + tests
│       │   ├── credentials.rs  Secret, Credentials (validation, stored format), CredentialStore trait, CredentialError; test-only EnvCredentials, MemoryCredentialStore + tests
│       │   ├── keyring.rs    OsCredentialStore: Windows Credential Manager / Secret Service via keyring-core, entry names, error mapping + tests, opt-in real-backend test
│       │   ├── s3.rs         S3Store (aws-sdk-s3): check_bucket, list, upload (single/multipart), download, delete, SHA-256, errors + tests
│       │   ├── dashboard.rs  CloudAccounts (Tauri state), the six account commands, safe DTOs, CommandError, connection check + tests
│       │   ├── tests.rs      Storage operations against the in-process S3 server
│       │   ├── test_server.rs  In-process S3 server (s3s + s3s-fs; can refuse chosen operations, records them) and temp-file helpers, tests only
│       │   └── integration.rs  Opt-in (#[ignore]) tests against real Backblaze B2 / Hetzner accounts
│       └── features/         Built-in features
│           ├── mod.rs        Feature trait, metadata, state, builtin() list, the two feature commands (FeatureList, FeatureChange) + tests
│           ├── preferences.rs  FeaturePreferences, PreferenceStore trait: startup choice, save after a switch, failure policy + tests
│           ├── registry.rs   FeatureRegistry: registration, lifecycle table, per-feature locking, start_all/stop_all + tests
│           └── example.rs    The example feature + tests
└── tests/
    └── desktop/              Desktop tests of the release binary
        ├── run.sh            Linux/X11 simulated test: private D-Bus session, runtime dir and data dir (XDG_DATA_HOME)
        ├── sni_watcher.py    Stub tray host (org.kde.StatusNotifierWatcher); --reject, --no-host
        ├── single_instance.py  Linux test driver: windows, tray menu, launches, tray failure, dashboard-only mode, feature preferences across restarts, Cloud Storage navigation and empty state, page via AT-SPI
        ├── virtual.sh        Linux: run.sh on a private Xvfb display with openbox
        ├── windows.ps1       Windows test on the real desktop: hide button, tray, close, launches, feature preferences across restarts, Cloud Storage navigation and empty state
        └── windows-isolated.ps1  Unattended Windows test on a private desktop: window, hide button, close, launches (no tray)
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | `operating_mode` in `lib.rs`; `list_features` and `set_feature_enabled` in `features/mod.rs`; the six cloud account commands in `cloud/dashboard.rs`. All are registered with `invoke_handler` in `run()` and declared in the app manifest in `build.rs` (permissions `allow-<command, with ->` in `capabilities/default.json`) |
| Where are built-in features? | `src-tauri/src/features/`: listed in `builtin()` in `mod.rs`, run by `FeatureRegistry` in `registry.rs`; how to add one: [features.md](features.md#adding-a-built-in-feature) |
| Where are feature choices stored? | Policy: `features/preferences.rs`; SQLite, schema and migrations: `storage.rs`; loaded in `run()` in `lib.rs` before the event loop; the file is `preferences.sqlite3` in the app's local data folder ([features.md](features.md#persistence)) |
| Where is cloud storage? | `src-tauri/src/cloud/`; the dashboard's commands in `cloud/dashboard.rs`, its page in `src/CloudStorage.svelte`; accounts database in `storage.rs`, loaded in `run()`; design: [cloud-storage.md](cloud-storage.md), providers: [cloud-providers.md](cloud-providers.md) |
| Where is the operating mode chosen? | `select_mode()` in `lib.rs`, from `tray::create()`'s result classified by `tray::unavailable_reason()` |
| Where are events emitted or listened to? | No custom events exist. Window events: `on_window_event` in `run()`. Exit events: `app.run(...)` closure in `run()` |
| Where is tray behaviour? | `src-tauri/src/tray.rs`, created from `setup` in `lib.rs` |
| Where is "show the dashboard"? | `show_dashboard()` in `lib.rs`, used by the tray and the single-instance callback |
| Where is the window declared? | `app.windows` in `src-tauri/tauri.conf.json` (label `main`, mirrored by `DASHBOARD_LABEL`) |
| Where are frontend permissions? | `src-tauri/capabilities/default.json` (static); the runtime capability `tray-mode` in `setup` in `lib.rs` |
| Where is the CSP? | `app.security.csp` in `src-tauri/tauri.conf.json` |
| Where is the frontend? | `src/`, entered from `index.html` |
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules (cloud storage: also `cloud/tests.rs`, opt-in `cloud/integration.rs`); frontend tests in `src/*.test.ts` (`App.test.ts`, `Features.test.ts`, `CloudStorage.test.ts`); desktop tests in `tests/desktop/` (`run.sh`/`virtual.sh` Linux, `windows.ps1`, `windows-isolated.ps1` Windows); all headless tests: `npm run test:headless`; CI workflows in `.github/workflows/` ([testing.md](../development/testing.md#ci)) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

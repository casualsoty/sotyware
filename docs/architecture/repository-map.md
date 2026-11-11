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
│   ├── architecture/         How it works and why (this file, overview, lifecycle, features, single instance, decisions)
│   ├── development/          Setup, commands, testing, cross-platform rules
│   └── status/               What is implemented and validated, limitations, roadmap
├── index.html                Vite HTML entry; mounts #app, loads src/main.ts
├── src/                      Frontend (Svelte 5 + TypeScript)
│   ├── main.ts               Mounts App into #app
│   ├── App.svelte            The dashboard UI
│   ├── App.test.ts           Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
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
│   ├── build.rs              tauri_build with an app manifest declaring the app's three commands
│   ├── tauri.conf.json       Product name, identifier, window `main`, CSP, bundles, dev/build hooks
│   ├── capabilities/
│   │   └── default.json      Frontend permissions for window `main` (hide is added at runtime, tray mode only)
│   ├── icons/                Bundle and tray icons (Tauri placeholders)
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, OperatingMode and its selection, lifecycle, operating_mode command, single instance, show_dashboard(), AppControl + unit tests
│       ├── tray.rs           Tray icon, menu, TrayAction, unavailable_reason() (no Linux tray host) + unit tests
│       └── features/         Built-in features
│           ├── mod.rs        Feature trait, metadata, state, builtin() list, enabled_at_startup(), the two feature commands + tests
│           ├── registry.rs   FeatureRegistry: registration, lifecycle table, per-feature locking, start_all/stop_all + tests
│           └── example.rs    The example feature + tests
└── tests/
    └── desktop/              Desktop tests of the release binary
        ├── run.sh            Linux/X11 simulated test: private D-Bus session + runtime dir
        ├── sni_watcher.py    Stub tray host (org.kde.StatusNotifierWatcher); --reject, --no-host
        ├── single_instance.py  Linux test driver: windows, tray menu, launches, tray failure, dashboard-only mode, page via AT-SPI
        ├── virtual.sh        Linux: run.sh on a private Xvfb display with openbox
        ├── windows.ps1       Windows test on the real desktop: hide button, tray, close, launches
        └── windows-isolated.ps1  Unattended Windows test on a private desktop: window, hide button, close, launches (no tray)
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | `operating_mode` in `lib.rs`; `list_features` and `set_feature_enabled` in `features/mod.rs`. All are registered with `invoke_handler` in `run()` and declared in the app manifest in `build.rs` (permissions `allow-operating-mode`, `allow-list-features`, `allow-set-feature-enabled`) |
| Where are built-in features? | `src-tauri/src/features/`: listed in `builtin()` in `mod.rs`, run by `FeatureRegistry` in `registry.rs`; how to add one: [features.md](features.md#adding-a-built-in-feature) |
| Where is the operating mode chosen? | `select_mode()` in `lib.rs`, from `tray::create()`'s result classified by `tray::unavailable_reason()` |
| Where are events emitted or listened to? | No custom events exist. Window events: `on_window_event` in `run()`. Exit events: `app.run(...)` closure in `run()` |
| Where is tray behaviour? | `src-tauri/src/tray.rs`, created from `setup` in `lib.rs` |
| Where is "show the dashboard"? | `show_dashboard()` in `lib.rs`, used by the tray and the single-instance callback |
| Where is the window declared? | `app.windows` in `src-tauri/tauri.conf.json` (label `main`, mirrored by `DASHBOARD_LABEL`) |
| Where are frontend permissions? | `src-tauri/capabilities/default.json` (static); the runtime capability `tray-mode` in `setup` in `lib.rs` |
| Where is the CSP? | `app.security.csp` in `src-tauri/tauri.conf.json` |
| Where is the frontend? | `src/`, entered from `index.html` |
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules; frontend tests in `src/*.test.ts` (`App.test.ts`, `Features.test.ts`); desktop tests in `tests/desktop/` (`run.sh`/`virtual.sh` Linux, `windows.ps1`, `windows-isolated.ps1` Windows); all headless tests: `npm run test:headless`; CI workflows in `.github/workflows/` ([testing.md](../development/testing.md#ci)) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

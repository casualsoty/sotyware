# Repository map

Where things are. Generated and ignored paths are listed in
[workflow.md](../development/workflow.md#generated-and-ignored-paths).

```
.
├── AGENTS.md                 Canonical instructions for coding agents
├── CLAUDE.md                 Claude Code entry point (imports AGENTS.md)
├── README.md                 Human-facing overview and quick start
├── docs/
│   ├── CONTEXT.md            Short session-start context for agents
│   ├── architecture/         How it works and why (this file, overview, lifecycle, single instance, decisions)
│   ├── development/          Setup, commands, testing, cross-platform rules
│   └── status/               What is implemented and validated, limitations, roadmap
├── index.html                Vite HTML entry; mounts #app, loads src/main.ts
├── src/                      Frontend (Svelte 5 + TypeScript)
│   ├── main.ts               Mounts App into #app
│   └── App.svelte            The dashboard UI
├── package.json              npm scripts and JS dependencies
├── package-lock.json         Pinned JS dependency tree
├── vite.config.ts            Vite: port 1420, strictPort, ignores src-tauri/, output dist/
├── svelte.config.js          vitePreprocess (TypeScript in .svelte)
├── tsconfig.json             Strict TypeScript settings
├── src-tauri/                Rust crate and Tauri configuration
│   ├── Cargo.toml            Crate `sotyware`, lib `sotyware_lib`, dependencies, release profile
│   ├── Cargo.lock            Pinned Rust dependency tree
│   ├── build.rs              tauri_build::build()
│   ├── tauri.conf.json       Product name, identifier, window `main`, CSP, bundles, dev/build hooks
│   ├── capabilities/
│   │   └── default.json      Frontend permissions for window `main`
│   ├── icons/                Bundle and tray icons (Tauri placeholders)
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, lifecycle, single instance, show_dashboard(), AppControl + unit tests
│       └── tray.rs           Tray icon, menu, TrayAction + unit tests
└── tests/
    └── desktop/              Desktop tests of the release binary
        ├── run.sh            Linux/X11 simulated test: private D-Bus session + runtime dir
        ├── sni_watcher.py    Stub tray host (org.kde.StatusNotifierWatcher)
        ├── single_instance.py  Linux test driver: windows, tray menu, launches
        └── windows.ps1       Windows test on the real desktop: hide button, tray, close, launches
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | Nowhere yet. A new command would be registered with `invoke_handler` in `run()` |
| Where are events emitted or listened to? | No custom events exist. Window events: `on_window_event` in `run()`. Exit events: `app.run(...)` closure in `run()` |
| Where is tray behaviour? | `src-tauri/src/tray.rs`, created from `setup` in `lib.rs` |
| Where is "show the dashboard"? | `show_dashboard()` in `lib.rs`, used by the tray and the single-instance callback |
| Where is the window declared? | `app.windows` in `src-tauri/tauri.conf.json` (label `main`, mirrored by `DASHBOARD_LABEL`) |
| Where are frontend permissions? | `src-tauri/capabilities/default.json` |
| Where is the CSP? | `app.security.csp` in `src-tauri/tauri.conf.json` |
| Where is the frontend? | `src/`, entered from `index.html` |
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules; desktop tests in `tests/desktop/` (`run.sh` Linux, `windows.ps1` Windows) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

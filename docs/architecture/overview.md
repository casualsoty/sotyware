# Architecture overview

This describes the architecture **as implemented today**. Nothing here is planned
work; for that, see [the roadmap](../status/roadmap.md). Where code lives:
[repository-map.md](repository-map.md).

## Shape

sotyware is one Tauri 2 process with:

- **one dashboard window** (label `main`) rendering a Svelte 5 single-page app, and
- **one system tray icon** with an "Open Dashboard" / "Quit" menu.

The process is meant to keep running in the background. The window is hidden
and shown, never destroyed, and only the tray "Quit" ends the process.

```
                ┌──────────────────────── sotyware process ───────────────────────┐
 2nd launch ──► │ tauri-plugin-single-instance ──► show_dashboard()               │
 (exits 0)      │                                        ▲                        │
                │ tray (tray.rs) ── Open Dashboard ──────┘   Quit ──► exit(0)     │
                │                                                                 │
                │ window "main" ◄── CloseRequested → hide()   (lib.rs)            │
                │   └ WebView: Svelte app (src/) ── getCurrentWindow().hide()     │
                │                                                                 │
                │ RunEvent::ExitRequested { code: None } → prevent_exit()         │
                └─────────────────────────────────────────────────────────────────┘
```

## Components and boundaries

| Component | File | Responsibility |
| --- | --- | --- |
| Binary entry point | `src-tauri/src/main.rs` | Calls `sotyware_lib::run()`. Nothing else. |
| Runtime and lifecycle | `src-tauri/src/lib.rs` | Builds the Tauri app: registers the single-instance plugin, creates the tray in `setup`, turns window close into hide, blocks implicit exits. Owns `show_dashboard()` and `DASHBOARD_LABEL`. |
| System tray | `src-tauri/src/tray.rs` | Creates the tray icon and menu, maps menu ids to `TrayAction`, handles clicks. Calls back into `crate::show_dashboard`. |
| Window definition | `src-tauri/tauri.conf.json` | Declares window `main` (800×600, visible at start), CSP, bundle settings, `identifier`. |
| Frontend permissions | `src-tauri/capabilities/default.json` | Grants window `main` exactly `core:window:allow-hide`. |
| Frontend | `index.html`, `src/main.ts`, `src/App.svelte` | Static dashboard with a "Hide dashboard" button. |

Boundaries that hold today:

- **Frontend ↔ Rust.** The frontend reaches Rust only through Tauri's built-in
  window API (`getCurrentWindow().hide()`), allowed by its capability. There are
  **no custom Tauri commands** (`#[tauri::command]`, `invoke_handler`) and **no
  custom events** (`emit`/`listen`). Adding either is an architectural change:
  record it in [decisions.md](decisions.md).
- **Lifecycle lives in `lib.rs`.** Window, exit and single-instance handling are
  registered once on the builder. `tray.rs` does not decide lifecycle policy; it
  calls `show_dashboard()` or `AppHandle::exit(0)`.
- **One window.** All code that shows the dashboard looks up the existing window
  by `DASHBOARD_LABEL` and never creates one.
- **No background work.** No threads, timers, polling or async tasks of
  sotyware's own. Everything is driven by Tauri events.
- **No persistence, no network, no settings.** Nothing is stored or fetched.

## Security model

- CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'` (inline styles are
  needed by Svelte).
- Capability `default` applies to window `main` only and grants
  `core:window:allow-hide` and nothing else, not even `core:default`.
- The single-instance plugin has no JavaScript API and needs no capability.
- Any new frontend permission must be the narrowest one that works and must be
  recorded in [decisions.md](decisions.md).

## Further reading

- [lifecycle.md](lifecycle.md): window, tray and exit behaviour.
- [single-instance.md](single-instance.md): how the second launch is handled.
- [decisions.md](decisions.md): why things are the way they are.

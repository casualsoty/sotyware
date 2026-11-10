# Architecture overview

This describes the architecture **as implemented today**. Nothing here is planned
work; for that, see [the roadmap](../status/roadmap.md). Where code lives:
[repository-map.md](repository-map.md).

## Shape

sotyware is one Tauri 2 process with:

- **one dashboard window** (label `main`) rendering a Svelte 5 single-page app, and
- **one system tray icon** with an "Open Dashboard" / "Quit" menu, where the
  environment has a tray host.

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
                │ setup: tray::create() → select_mode() → OperatingMode (state)   │
                │                                                                 │
                │ window "main" ◄── CloseRequested → tray mode: hide()  (lib.rs)  │
                │                                   dashboard-only: exit(0)       │
                │   └ WebView: Svelte app (src/) ── invoke("operating_mode")      │
                │                                  getCurrentWindow().hide()      │
                │                                  (allowed in tray mode only)    │
                │                                                                 │
                │ RunEvent::ExitRequested { code: None } → prevent_exit()         │
                └─────────────────────────────────────────────────────────────────┘
```

## Components and boundaries

| Component | File | Responsibility |
| --- | --- | --- |
| Binary entry point | `src-tauri/src/main.rs` | Calls `sotyware_lib::run()`. Nothing else. |
| Runtime and lifecycle | `src-tauri/src/lib.rs` | Builds the Tauri app: registers the single-instance plugin, creates the tray in `setup` and picks the `OperatingMode`, turns window close into hide (tray mode) or exit (dashboard-only mode), adds the hide capability in tray mode only, serves the `operating_mode` command, blocks implicit exits. Owns `show_dashboard()`, `DASHBOARD_LABEL` and the `AppControl` trait (show dashboard / quit). The lifecycle decisions are plain functions (`select_mode`, `close_action`, `frontend_may_hide`, `exit_allowed`, `on_second_launch`, `reveal`) that the handlers call, so they are unit-tested ([ADR-009](decisions.md#adr-009-lifecycle-decisions-are-plain-functions-behind-small-traits)). |
| System tray | `src-tauri/src/tray.rs` | Creates the tray icon and menu from `MENU_ITEMS`, maps menu ids and icon clicks to `TrayAction`, which calls `AppControl` (implemented by `AppHandle`: `show_dashboard()` or `exit(0)`). `unavailable_reason()` tells a missing Linux tray host apart from other tray errors. |
| Build script | `src-tauri/build.rs` | `tauri_build` with an app manifest that declares the `operating_mode` command, so it needs a permission. |
| Window definition | `src-tauri/tauri.conf.json` | Declares window `main` (800×600, visible at start), CSP, bundle settings, `identifier`. |
| Frontend permissions | `src-tauri/capabilities/default.json` | Grants window `main` exactly `allow-operating-mode`. `core:window:allow-hide` is added at runtime, in tray mode only. |
| Frontend | `index.html`, `src/main.ts`, `src/App.svelte` | Static dashboard. Tray mode: a "Hide dashboard" button. Dashboard-only mode: a note instead. |

Boundaries that hold today:

- **Frontend ↔ Rust.** The frontend reaches Rust through exactly two calls: the
  custom command `operating_mode` (read-only, returns `"tray"` or
  `"dashboard-only"`) and Tauri's built-in window API
  (`getCurrentWindow().hide()`, tray mode only). There is no other custom command
  and there are **no custom events** (`emit`/`listen`). Adding either is an
  architectural change: record it in [decisions.md](decisions.md).
- **Platform detection lives in Rust.** Only `tray::unavailable_reason()` looks
  at the tray error. The frontend never inspects the OS or D-Bus.
- **Lifecycle lives in `lib.rs`.** Window, exit and single-instance handling are
  registered once on the builder. `tray.rs` does not decide lifecycle policy; it
  calls `show_dashboard()` or `AppHandle::exit(0)`.
- **One window.** All code that shows the dashboard looks up the existing window
  by `DASHBOARD_LABEL` and never creates one.
- **No background work.** No threads, timers, polling, retries or async tasks
  of sotyware's own. Everything is driven by Tauri events.
- **No persistence, no network, no settings.** Nothing is stored or fetched.

## Security model

- CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'` (inline styles are
  needed by Svelte).
- Capability `default` applies to window `main` only and grants
  `allow-operating-mode` (the app's one command) and nothing else, not even
  `core:default`. In tray mode, `setup` adds the runtime capability `tray-mode`,
  which grants window `main` `core:window:allow-hide` and nothing else
  ([ADR-013](decisions.md#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode)).
- The single-instance plugin has no JavaScript API and needs no capability.
- Any new frontend permission must be the narrowest one that works and must be
  recorded in [decisions.md](decisions.md).

## Further reading

- [lifecycle.md](lifecycle.md): window, tray and exit behaviour.
- [single-instance.md](single-instance.md): how the second launch is handled.
- [decisions.md](decisions.md): why things are the way they are.

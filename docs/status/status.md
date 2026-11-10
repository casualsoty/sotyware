# Implementation status

Last updated: 2026-11-10. Update this file whenever
implementation or validation changes. Never mark something validated that was
not run on that environment.

## Current milestone

**Milestone 0: prove the desktop runtime.** Show that the stack (Tauri 2 +
Svelte 5) and the background lifecycle (tray, hide-on-close, explicit Quit,
single instance) work on Windows 10/11 and Linux desktops, before any product
features are built.

State: implemented, and validated only in a simulated Linux environment
(WSL2/WSLg). Validation on real Windows and real Linux desktops is the open part.

## Implemented

| Feature | Where |
| --- | --- |
| Tauri 2 desktop runtime (one process, one window `main`) | `src-tauri/src/lib.rs`, `tauri.conf.json` |
| Svelte 5 dashboard (static page with a "Hide dashboard" button) | `src/App.svelte` |
| System tray with "Open Dashboard" and "Quit" | `src-tauri/src/tray.rs` |
| Hide-on-close lifecycle | `on_window_event` in `lib.rs` |
| Explicit Quit is the only exit; implicit exits blocked | `app.run` closure in `lib.rs`, `tray.rs` |
| Fatal exit (code 1) if the tray cannot be created | `setup` in `lib.rs` |
| Single instance via `tauri-plugin-single-instance` | `run()` in `lib.rs` |
| Minimal capability (`core:window:allow-hide`) and CSP | `capabilities/default.json`, `tauri.conf.json` |

Implemented but without automated test in the repository: the "Hide dashboard"
button, left-click-to-open (Windows only), restoring a minimized dashboard and
moving focus (both skipped under WSLg).

Nothing else exists: no product features, no settings, no persistence, no
custom Tauri commands or events.

## Validation matrix

Legend: ✅ passed · ⏭️ skipped by the environment (not a pass) · ❌ failed ·
— not run.

"Re-run 2026-11-10" means the check was repeated while writing this document.
"Earlier run" means it was recorded in the original README from a run whose
script or procedure is not in the repository; it was not repeated.

### Static checks and builds (WSL2, Ubuntu 24.04, Rust 1.99.1, Node 24.22.0)

| Check | Result | When |
| --- | --- | --- |
| `npm run check` (0 errors, 0 warnings) | ✅ | Re-run 2026-11-10 |
| `npm run build` | ✅ | Re-run 2026-11-10 |
| `cargo fmt --check` | ✅ | Re-run 2026-11-10 |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Re-run 2026-11-10 |
| `cargo test` (1 test) | ✅ | Re-run 2026-11-10 |
| `npm run tauri build -- --no-bundle` | ✅ | Re-run 2026-11-10 |
| `npm run tauri build`: `.deb`, `.rpm`, AppImage built (with WSL `PATH` filter) | ✅ built, not installed or run | Re-run 2026-11-10 |
| `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ✅ | Re-run 2026-11-10 |

### Runtime behaviour, simulated Linux desktop (WSL2/WSLg, X11, stub tray host)

`tests/desktop/run.sh` on the release binary. Re-run 2026-11-10:
**54 passed, 0 failed, 2 skipped**.

| Behaviour | Result | Source |
| --- | --- | --- |
| First launch: dashboard visible, one tray item with Open Dashboard / Quit | ✅ | desktop test |
| Close request hides (does not destroy) the window; process stays | ✅ | desktop test |
| Tray "Open Dashboard" restores the same single window | ✅ | desktop test |
| Second launch (dashboard visible or hidden): exits 0, no new window, process or tray item | ✅ | desktop test |
| Second launch while hidden: dashboard shown and focused | ✅ | desktop test |
| 5 sequential + 4 concurrent launches: always one process, tray item, window | ✅ | desktop test |
| Tray "Quit" exits 0, no process left; relaunch starts a fresh primary | ✅ | desktop test |
| Second launch focuses an already visible dashboard | ⏭️ WSLg WM ignores `_NET_ACTIVE_WINDOW` | desktop test |
| Second launch restores a minimized dashboard | ⏭️ WSLg WM ignores `XIconifyWindow` | desktop test |
| "Hide dashboard" button hides the window; process stays | ✅ | Earlier run |
| Repeated "Open Dashboard" creates no second window | ✅ | Earlier run (also implied by desktop test window counts) |
| Native Wayland backend: runs, tray registers, Quit exits 0 (window state not inspected) | ✅ | Earlier run |
| `XDG_RUNTIME_DIR` pointing to a missing directory: tray error, exit 1 | ✅ | Re-run 2026-11-10 |

This environment is **not a real desktop**: the tray host is a test stub, and no
tray icon was ever seen or clicked by a person.

### Not yet validated

| Environment | Status |
| --- | --- |
| Windows 10 | — never built or run |
| Windows 11 | — never built or run |
| Windows: left-click-to-open, named-mutex single instance, focus rules | — reviewed in code/plugin source only |
| Real GNOME (with and without the AppIndicator extension) | — |
| Real KDE Plasma | — |
| XFCE | — |
| Real Wayland session with a real tray host | — |
| Focus and un-minimize on second launch, on any real WM | — (skipped under WSLg) |
| Installing and running `.deb` / `.rpm` / AppImage | — |

To validate an environment, follow the manual checklist in
[testing.md](../development/testing.md#manual-check-real-desktop-linux-or-windows)
and update this table.

## See also

- [limitations.md](limitations.md): known limitations.
- [roadmap.md](roadmap.md): next steps.

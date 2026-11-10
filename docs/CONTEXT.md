# sotyware: session context

Read this at the start of every task. It is a summary; the linked documents hold
the details and win if anything here disagrees. Rules for agents:
[AGENTS.md](../AGENTS.md).

## What it is

A cross-platform (Windows 10/11, Linux) desktop app that runs in the background,
lives in the system tray and opens a dashboard window on demand. Built with
Tauri 2 (Rust) and Svelte 5 + TypeScript. Version 0.1.0.

## What works today

Only the desktop runtime; **no product features exist**.

- One dashboard window (`main`) with a "Hide dashboard" button.
- Tray icon with "Open Dashboard" and "Quit".
- Close (✕) hides the window; only tray "Quit" ends the process.
- Single instance: a second launch shows the existing dashboard and exits 0.
- The app exits 1 if the tray icon cannot be created.

## Repository at a glance

```
src/                 Svelte frontend (main.ts, App.svelte)
src-tauri/src/       main.rs → lib.rs (builder, lifecycle, single instance) + tray.rs
src-tauri/           tauri.conf.json, capabilities/default.json, Cargo.toml, icons/
src/App.test.ts      Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
tests/desktop/       Desktop tests of the release binary (run.sh: Linux/X11 simulated; virtual.sh: same on Xvfb + openbox; windows.ps1: real Windows desktop)
docs/                architecture/, development/, status/
```

Full map: [repository-map.md](architecture/repository-map.md).

## Entry points and boundaries

- Startup: `src-tauri/src/main.rs` → `sotyware_lib::run()` in `src-tauri/src/lib.rs`.
- The Linux tray backend is chosen in `src-tauri/Cargo.toml` (Linux-only
  `tray-icon` with feature `ksni`, ADR-011), not in code.
- Showing the dashboard always goes through `show_dashboard()` in `lib.rs`; the
  window is never created at runtime or destroyed.
- The frontend may only call `getCurrentWindow().hide()`
  (`core:window:allow-hide`). No custom Tauri commands or events exist.
- The single-instance plugin must stay the first plugin registered.

Details: [overview.md](architecture/overview.md),
[lifecycle.md](architecture/lifecycle.md),
[single-instance.md](architecture/single-instance.md),
[decisions.md](architecture/decisions.md).

## Commands

```bash
npm ci
npm run tauri dev
npm run check && npm run build
npm run test:headless                      # frontend + Rust unit tests, no display needed
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run tauri build -- --no-bundle && tests/desktop/run.sh   # or: npm run test:desktop:virtual
```

**WSL:** non-interactive shells may pick up Windows `npm` from `/mnt/c` and not find
`node`/`cargo`. Fix `PATH` first: [setup.md](development/setup.md#wsl2-notes).
More: [workflow.md](development/workflow.md), [testing.md](development/testing.md).

## Tests

- `cargo test`: 17 unit tests of lifecycle and tray decisions (close → hide,
  implicit exit blocked, tray failure → exit 1, second launch, tray dispatch,
  config labels). No display needed.
- `npm run test:headless` (17 Rust + 8 frontend tests): passes with no display,
  window manager, tray host or D-Bus session (2026-11-10, WSL2), and natively on
  Windows 11 (2026-11-10, `1f5853b`).
- `npm run test:desktop:virtual`: 68 passed, 0 failed, 0 skipped on Xvfb +
  openbox, 3 runs in a row (2026-11-10, `12a6185`, `ksni` tray backend, WSL2
  Ubuntu 24.04.1 with apt-installed packages); real release binary, stub tray
  host. A 420-launch loop registered every tray item. With libayatana (66-check
  driver, `53d9743`) it failed about 1 run in 60 on a library race.
- `tests/desktop/run.sh`: 57 passed, 0 failed, 2 skipped on WSL2/WSLg
  (2026-11-10, 65-check driver before `53d9743`). The tray host is a stub; SKIP
  is never a pass.
- `tests/desktop/windows.ps1`: 80 passed, 0 failed on native Windows 11 25H2
  (2026-11-10; again at `1f5853b`, release binary and NSIS-installed copy), real
  desktop; it moves the mouse. Covers the Hide button.
- Test categories (headless / virtual display / real desktop):
  [testing.md](development/testing.md). No JS linter, no CI.

## Platform limitations (summary)

Windows 11 is validated by the automated Windows test (no person's visual check
yet, MSI not installed). Windows 10 and real Linux desktops (GNOME, KDE, XFCE, real
Wayland) are **not validated**. Windows 11 puts the tray icon in the hidden-icons
flyout. Linux: the tray is a StatusNotifierItem via `tray-icon`'s pure-Rust
`ksni` backend, which avoids a registration race in libayatana-appindicator;
without a StatusNotifierWatcher or session bus sotyware exits 1 (no XEmbed
fallback); vanilla GNOME has no watcher; left click needs the tray host to send
`Activate`; single instance silently turns off without a D-Bus session bus.
Placeholder icons; no autostart. Full list:
[limitations.md](status/limitations.md).

## Milestone and next steps

Milestone 0: prove the desktop runtime on Windows and Linux. Implemented;
validated natively on Windows 11 (automated) and in Linux simulation. Next: the
manual check on Windows 11, Windows 10, then real Linux desktops; record results in the
[validation matrix](status/status.md#validation-matrix). See
[roadmap.md](status/roadmap.md). SQLite, feature manager, plugins, sync and
autostart are undecided ideas, not tasks.

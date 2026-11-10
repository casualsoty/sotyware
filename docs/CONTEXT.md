# sotyware: session context

Read this at the start of every task. It is a summary; the linked documents hold
the details and win if anything here disagrees. Rules for agents:
[AGENTS.md](../AGENTS.md).

## What it is

A cross-platform (Windows 10/11, Linux) desktop app that runs in the background,
lives in the system tray and opens a dashboard window on demand. On Linux
without a tray host it runs as a plain window (dashboard-only mode). Built with
Tauri 2 (Rust) and Svelte 5 + TypeScript. Version 0.1.0.

## What works today

Only the desktop runtime; **no product features exist**.

- Two operating modes, chosen once at startup:
  - **Tray mode**: tray icon with "Open Dashboard" and "Quit"; the dashboard
    (`main`) has a "Hide dashboard" button; close (✕) hides it; only tray "Quit"
    ends the process.
  - **Dashboard-only mode** (Linux without a session bus, StatusNotifierWatcher
    or StatusNotifierHost): no tray, a note instead of the Hide button, hiding
    is not permitted, close (✕) ends the process (exit 0).
- Single instance in both modes: a second launch shows the existing dashboard
  and exits 0.
- Any other tray creation error: exit 1.

## Repository at a glance

```
src/                 Svelte frontend (main.ts, App.svelte)
src-tauri/src/       main.rs → lib.rs (builder, operating mode, lifecycle, single instance, operating_mode command) + tray.rs
src-tauri/           tauri.conf.json, capabilities/default.json, Cargo.toml, icons/
src/App.test.ts      Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
.github/workflows/   CI: ci.yml (automatic, headless), desktop-*.yml (manual)
tests/desktop/       Desktop tests of the release binary (run.sh: Linux/X11 simulated; virtual.sh: same on Xvfb + openbox; windows.ps1: real Windows desktop; windows-isolated.ps1: Windows private desktop)
docs/                architecture/, development/, status/
```

Full map: [repository-map.md](architecture/repository-map.md).

## Entry points and boundaries

- Startup: `src-tauri/src/main.rs` → `sotyware_lib::run()` in `src-tauri/src/lib.rs`.
- The Linux tray backend is chosen in `src-tauri/Cargo.toml` (Linux-only
  `tray-icon` with feature `ksni`, ADR-011), not in code.
- Showing the dashboard always goes through `show_dashboard()` in `lib.rs`; the
  window is never created at runtime, and destroyed only when the process ends.
- The mode is chosen in `select_mode()` (`lib.rs`) from the tray result. Only
  `tray::unavailable_reason()` (Linux, typed `ksni`/`zbus` errors) may select
  dashboard-only mode; every other error stays fatal. No retries or polling.
- The frontend may call the `operating_mode` command (`allow-operating-mode`)
  and, in tray mode only, `getCurrentWindow().hide()` (`core:window:allow-hide`,
  granted at runtime). No other custom command, no custom events.
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
npm run test:desktop:windows               # Windows, real desktop, moves the mouse
npm run test:desktop:windows:isolated      # Windows, private desktop, unattended (no tray)
```

**WSL:** non-interactive shells may pick up Windows `npm` from `/mnt/c` and not find
`node`/`cargo`. Fix `PATH` first: [setup.md](development/setup.md#wsl2-notes).
More: [workflow.md](development/workflow.md), [testing.md](development/testing.md).

## Tests

- `cargo test`: 31 unit tests of lifecycle, mode and tray decisions (mode
  selection from the real `ksni`/`zbus` error types, close → hide or quit per
  mode, hide granted only in tray mode, implicit exit blocked, unexpected tray
  failure → exit 1, second launch, tray dispatch, config labels). No display needed.
- `npm run test:headless` (31 Rust + 13 frontend tests): passes on WSL2
  (2026-11-10, `8ca3bd7`) and natively on Windows 11 (2026-11-10, `e8834fd`;
  23 Rust tests there, the 8 `ksni`/`zbus` ones are Linux-only; again at
  `195fa8a`). This is the only headless suite on either platform.
- `npm run test:desktop:virtual`: 92 passed, 0 failed, 0 skipped on Xvfb +
  openbox, 3 runs in a row (2026-11-10, `c2cbeb4`, WSL2 Ubuntu 24.04.1); real
  release binary, stub tray host, page read via AT-SPI. Covers tray mode and
  dashboard-only mode (no watcher, no host, no bus) and a rejecting watcher
  (exit 1). The 68-check tray-mode driver passed 3 runs at `12a6185` and a
  420-launch loop registered every tray item.
- `tests/desktop/run.sh`: 57 passed, 0 failed, 2 skipped on WSL2/WSLg
  (2026-11-10, 65-check driver before `53d9743`). The tray host is a stub; SKIP
  is never a pass.
- `tests/desktop/windows.ps1`: 80 passed, 0 failed on native Windows 11 25H2
  (2026-11-10, `e8834fd` with the test fix `71442d3`: 3 runs on the release
  binary, 2 on the NSIS-installed copy), real desktop; it moves the mouse.
  Covers the Hide button, which works with hide granted at runtime. Again 80/80,
  3 runs, at `195fa8a` (`npm run test:desktop:windows`). Needs the unlocked,
  interactive desktop with a taskbar; otherwise it exits 3 without running.
- `tests/desktop/windows-isolated.ps1`: 56 passed, 0 failed, 3 runs each at
  `195fa8a` and `0131e09`, native Windows 11. Real binary on a private desktop:
  unattended, nothing on screen, no input, but **not headless** (needs a
  logged-in session) and no tray, tray menu or Quit. A close-quits mutant fails
  9 checks. It cannot run elevated (WebView2 then starts its browser on the
  Default desktop; `0x80070578`) and reports NOT RUN, exit 4; again 56/56,
  3 runs, at `468a2c3`.
- Test categories (headless / virtual display / private desktop / real desktop):
  [testing.md](development/testing.md). No JS linter.
- CI (GitHub Actions, private repo on GitHub Free: $12 of hosted compute a
  month, then blocked): `ci.yml` runs automatically on pull requests to and
  pushes to `main` (Linux + Windows: `npm ci`, check, build, fmt, clippy,
  `test:headless`; docs-only changes skipped). Manual only:
  `desktop-virtual.yml` (virtual display, not a real desktop) and
  `desktop-windows-isolated.yml` (experimental). The real-desktop Windows test
  is not in CI. `desktop-virtual.yml` ran once (`14a5de1`, ubuntu-26.04
  runner) and failed 89/2/1 on a driver bug (AT-SPI role name renamed in
  at-spi2-core 2.60), fixed in `ead47e7` and passing 3/3 in an Ubuntu 26.04
  container; not re-run on GitHub yet. `desktop-windows-isolated.yml` ran once
  (Windows Server 2025) and failed on the elevated hosted job, reproduced on
  Windows 11 elevated; it now reports NOT RUN (exit 4) there, not re-run yet.
  No other workflow result is recorded.
  [testing.md](development/testing.md#ci).

## Platform limitations (summary)

Windows 11, the primary runtime platform, is validated by the automated Windows
test at `e8834fd`, including the NSIS-installed copy (no person's visual check
yet, MSI not installed). Windows 10 and real Linux desktops (GNOME, KDE, XFCE,
real Wayland) are **not validated**; the development machine has no real Linux
desktop (WSL2 only, Windows 11 Home without a VM, surveyed 2026-11-10). Windows 11 puts the tray icon in the
hidden-icons flyout. Linux: the tray is a StatusNotifierItem via `tray-icon`'s
pure-Rust `ksni` backend, which avoids a registration race in
libayatana-appindicator. Without a StatusNotifierWatcher (vanilla GNOME), host or
session bus sotyware runs dashboard-only and does not run in the background (no
XEmbed fallback). That has been seen only under the virtual display. Some
tray-host failures (a watcher that leaves mid-call, other watcher errors) still
exit 1. Left click needs the tray host to send `Activate`. Single instance
silently turns off without a D-Bus session bus. Placeholder icons; no autostart.
Full list: [limitations.md](status/limitations.md).

## Milestone and next steps

Milestone 0: prove the desktop runtime on Windows and Linux. Implemented;
validated natively on Windows 11 (automated, last at `195fa8a`) and in Linux
simulation (WSL2 is the development environment, not a runtime target). Next:
the manual check on Windows 11, Windows 10, then real Linux desktops (including
dashboard-only mode on vanilla GNOME; needs a VM, live session or Linux machine); record results in the
[validation matrix](status/status.md#validation-matrix). See
[roadmap.md](status/roadmap.md). SQLite, feature manager, plugins, sync and
autostart are undecided ideas, not tasks.

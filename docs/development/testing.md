# Testing guide

sotyware has three layers of verification. Each catches different problems; none
replaces another.

| Layer | What it covers | Where | Needs |
| --- | --- | --- | --- |
| Static checks | Types, Svelte diagnostics, formatting, Clippy lints | `npm run check`, `cargo fmt --check`, `cargo clippy` | nothing special |
| Rust unit tests | Pure logic only (today: tray menu id → action mapping) | `#[cfg(test)] mod tests` inside the module, e.g. `src-tauri/src/tray.rs` | nothing special |
| Simulated desktop test | Real release binary: window lifecycle, tray menu, single instance | `tests/desktop/` | Linux, X11 display, D-Bus tools |
| Manual check | Real tray host, real window manager, focus | checklist below | a real desktop |

There are no frontend unit tests and no test framework for TypeScript.

## Rust unit tests

```bash
cd src-tauri && cargo test
```

Keep logic that can be tested without a running Tauri app in plain functions or
types (as `TrayAction::from_menu_id` is) and test it next to its code. Window,
tray and single-instance behaviour cannot run under `cargo test`; they need a
display and an event loop.

## Simulated desktop test

```bash
npm run tauri build -- --no-bundle
tests/desktop/run.sh [binary]
```

Files:

- `tests/desktop/run.sh`: creates a temporary private `$XDG_RUNTIME_DIR` and a
  private D-Bus session (`dbus-run-session`), starts the stub tray host, then runs
  the Python driver with `GDK_BACKEND=x11`. A sotyware already running on your
  desktop is not affected. Exits 2 if the binary is missing.
- `tests/desktop/sni_watcher.py`: a minimal `org.kde.StatusNotifierWatcher`, so
  libayatana-appindicator registers a tray item. It records each registration in a
  file that the driver counts.
- `tests/desktop/single_instance.py`: drives the binary. It inspects X11 windows via
  `ctypes` (no extra Python packages), sends `WM_DELETE_WINDOW` to simulate the
  close button, clicks tray menu items over `com.canonical.dbusmenu`, and launches
  further instances sequentially and concurrently. It does **not** click the
  dashboard's "Hide dashboard" button; that path has no automated test in the
  repository.

Requirements: `python3`, `python3-dbus`, `python3-gi`, `libX11`,
`dbus-run-session`, and an X11 display (`$DISPLAY`: WSLg, or Xvfb plus a window
manager).

Results: each check prints `PASS`, `FAIL` or `SKIP`, then a summary line. `SKIP`
means the environment could not exercise the behaviour (for example, a window
manager that ignores minimize requests). **A SKIP is never a pass.** Do not turn
a SKIP into a PASS by loosening a check; record it as unverified instead.

The driver exits 1 if any check fails and 0 otherwise; SKIPs do not change the
exit code, so always read the summary line too. It aborts at once if a
`sotyware` process from the same binary is already running.

Under WSLg two checks are always skipped: focusing an already visible dashboard
(the WM ignores `_NET_ACTIVE_WINDOW`) and restoring a minimized dashboard (the WM
ignores `XIconifyWindow`). The latest results are recorded in
[the validation matrix](../status/status.md#validation-matrix).

What this test is **not**: a real desktop. The tray host is a stub, nobody sees or
clicks an icon, and the window manager is whatever serves `$DISPLAY`.

## Manual check (real desktop, Linux or Windows)

Use this on any environment listed as not validated in
[the validation matrix](../status/status.md#validation-matrix), and record the
result there.

1. Start sotyware once. Note the single tray icon.
2. Start it again (app menu, shortcut, or the binary/`.exe`). Expected: no second
   tray icon, and the existing dashboard comes to the front.
3. Click another app, then start sotyware again. Expected: the dashboard gets focus,
   or at least flashes in the taskbar if the OS blocks focus stealing.
4. Minimize the dashboard, start sotyware again. Expected: the dashboard is restored.
5. Close the dashboard with ✕, start sotyware again. Expected: it reappears.
6. Repeat a few times. Check the process list (Task Manager, or
   `pgrep -a sotyware`): always exactly one `sotyware` process.
7. Tray → Quit. Expected: the process is gone. Start sotyware again: it starts
   normally.
8. Windows only: left-click the tray icon. Expected: the dashboard opens.
   Right-click shows the menu.

## Rules

- Never weaken, skip or delete a check to make a change pass. If a check is wrong,
  fix it in its own commit and explain why.
- Report what was actually run, on which environment, and what remains unverified.
- When behaviour changes, update the desktop test and the manual checklist in the
  same change.

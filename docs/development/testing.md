# Testing guide

sotyware has four layers of verification. Each catches different problems; none
replaces another.

| Layer | What it covers | Where | Needs |
| --- | --- | --- | --- |
| Static checks | Types, Svelte diagnostics, formatting, Clippy lints | `npm run check`, `cargo fmt --check`, `cargo clippy` | nothing special |
| Rust unit tests | Pure logic only (today: tray menu id → action mapping) | `#[cfg(test)] mod tests` inside the module, e.g. `src-tauri/src/tray.rs` | nothing special |
| Simulated desktop test | Real release binary: window lifecycle, tray menu, single instance | `tests/desktop/run.sh` | Linux, X11 display, D-Bus tools |
| Windows desktop test | Real release binary on the real Windows desktop: also the "Hide dashboard" button, tray left click, title-bar close button | `tests/desktop/windows.ps1` | Windows, an unattended desktop session |
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
  dashboard's "Hide dashboard" button; on Linux that path has no automated test
  (the [Windows desktop test](#windows-desktop-test) covers it on Windows).

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

## Windows desktop test

Run on Windows, from a Windows checkout (not a WSL path), after a release build:

```powershell
npm run tauri build -- --no-bundle
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1 [-Exe <path>]
```

`-Exe` defaults to `src-tauri\target\release\sotyware.exe`. To test an installed
copy, pass its path, e.g. `-Exe $env:LOCALAPPDATA\sotyware\sotyware.exe` for the
NSIS per-user install.

It needs only Windows PowerShell 5.1 (UI Automation and `user32` via `Add-Type`), no
extra modules. **It runs on your real desktop:** it moves the mouse and clicks the
tray icon, the tray menu, the dashboard's button and its title-bar close button, and
opens the notification area's "Show Hidden Icons" flyout. Do not touch the mouse or
keyboard while it runs (about 30 seconds).

What it does, and how:

- Finds the dashboard with `EnumWindows` (class `Tauri Window`, owned by a
  sotyware process). UI Automation is not enough: it does not list hidden windows.
- Clicks "Hide dashboard" inside WebView2 twice: once through UI Automation
  `Invoke` and once with a real mouse click at the button's position. Each time it
  checks that the window becomes invisible, is not destroyed, and the process stays.
- Finds the sotyware icon in the notification area by its UI Automation name
  (`sotyware`, the tooltip). Windows 11 puts new icons in the hidden-icons flyout,
  so the test opens it first. It left-clicks the icon (expects the dashboard and no
  menu) and right-clicks it for the menu.
- Reads the tray menu through the Win32 menu handle (`MN_GETHMENU`), because the
  items are owner-drawn and invisible to UI Automation. It clicks items at their
  `GetMenuItemRect` position.
- Clicks the real title-bar close button, located with `WM_NCHITTEST` = `HTCLOSE`
  (the title bar is not in the UI Automation tree), and also sends a programmatic
  close request (`WindowPattern.Close`). It does not send Alt+F4, which would close
  another app if focus moved.
- Launches second instances while the dashboard is hidden, visible and minimized,
  5 sequentially and 4 concurrently, and checks exit code 0, one process (same
  pid), one window, one tray icon.
- Hides, quits from the tray, checks exit code 0, no process and no tray icon left,
  then relaunches a fresh instance and quits it.

Results: `PASS`/`FAIL` per check, `INFO` for observations Windows may legitimately
vary (whether the dashboard ended up in the foreground), then a summary line. Exit
code 0 if all passed, 1 if any failed, 2 if the binary is missing or a sotyware
from the same path is already running. It does not catch an icon that is in the
tray but invisible to a person; the [manual check](#manual-check-real-desktop-linux-or-windows)
still covers looking at it.

Verified that it detects a broken button: with `hideDashboard` changed to do
nothing, all three hide checks failed (2026-11-10).

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

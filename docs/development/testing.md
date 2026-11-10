# Testing guide

Tests are grouped by what the environment must provide. Each category catches
different problems; none replaces another, and passing one says nothing about the
others.

| Category | Command | Runs the release binary? | Needs |
| --- | --- | --- | --- |
| **A. Headless** | `npm run test:headless` | no | Node, Rust and the Tauri build libraries; **no** display, window manager, tray host or D-Bus session |
| **B. Virtual display** (Linux) | `npm run test:desktop:virtual` | yes | Xvfb, openbox, D-Bus, AT-SPI and Python tools; no monitor, no desktop session |
| B′. Simulated desktop on an existing display (Linux) | `tests/desktop/run.sh` | yes | an X11 display you already have (WSLg, or a desktop session) |
| **B″. Private desktop** (Windows) | `npm run test:desktop:windows:isolated` (`tests/desktop/windows-isolated.ps1`) | yes | Windows, a logged-in session (window station `WinSta0`); **no** visible or unlocked screen, no mouse, no taskbar |
| **C. Real desktop**: Windows | `npm run test:desktop:windows` (`tests/desktop/windows.ps1`) | yes | Windows, an unlocked interactive desktop with a taskbar that nobody touches for 30 s |
| **C. Real desktop**: manual | [checklist](#manual-check-real-desktop-linux-or-windows) | yes | a real desktop and a person |

Static checks (`npm run check`, `cargo fmt --check`, `cargo clippy`) need nothing
special and run before every commit ([workflow.md](workflow.md)).

**Only category A is headless.** Every test that starts the release binary needs a
display: on Linux an X server (B, B′), on Windows a logged-in session with a window
station (B″, C). No Windows test of the real binary runs without one; see
[Headless Windows desktop testing](#headless-windows-desktop-testing).

### What each category really exercises

B″ (Windows private desktop) covers the rows marked for the Windows test except
the tray rows, real mouse clicks and Quit; details in
[its section](#windows-isolated-desktop-test).

| Behaviour | A. Headless | B. Virtual display | C. Windows test | C. Manual |
| --- | --- | --- | --- | --- |
| Close → hide; second launch → show dashboard | ✅ unit tests (the decision) | ✅ real binary | ✅ real binary | ✅ |
| Mode selection: tray created ⇒ tray mode; no session bus / watcher / host ⇒ dashboard-only; other errors ⇒ exit 1 | ✅ unit tests (classification of the real error types) | ✅ real binary (no watcher, no host, no bus, rejecting watcher) | — (tray mode only) | — |
| Unexpected tray failure → exit 1 | ✅ unit tests (the decision) | ✅ real binary (watcher answers `AccessDenied`) | — | — |
| Dashboard-only: starts visible, note on the page, no Hide button, close → exit 0, second launch → show | ✅ unit tests (decisions), mocked IPC (page) | ✅ real binary, page read via AT-SPI | — | — |
| Implicit exit (`code: None`) blocked | ✅ unit tests (the decision) | — not triggered: the window is never destroyed | — | — |
| Tray menu items and click dispatch (the decision) | ✅ unit tests | menu over D-Bus | ✅ real clicks | ✅ |
| "Hide dashboard" button sends the hide request; errors shown | ✅ mocked IPC | ✅ real button via AT-SPI (tray mode, hides the window) | ✅ real button | ✅ |
| Hide permission granted in tray mode only (runtime capability) | ✅ unit test (the decision) | ✅ tray mode: Hide works; dashboard-only: no button | ✅ tray mode | — |
| The OS window really hides / shows / restores | — | ✅ X11 under openbox | ✅ Win32 | ✅ |
| Focus and un-minimize on second launch | — | ✅ openbox honours the requests | ✅ | ✅ |
| Single-instance handover (D-Bus on Linux, Win32 on Windows), concurrent launches | — | ✅ private session bus | ✅ | ✅ |
| Tray icon registers with a tray host | — | stub host (`sni_watcher.py`) | ✅ real notification area | ✅ |
| Tray icon visible to a person, real tray host (GNOME, KDE, …), Wayland | — | — | partly (icon found by UI Automation) | ✅ |
| Visual appearance | — | — | — | ✅ |
| Screen-reader access | — | — | — | — not in the checklist; untested |

**Mocked or simulated, never real:** in A, the Tauri runtime (recording doubles
of `AppControl`/`DashboardWindow`), the webview (happy-dom) and IPC
(`@tauri-apps/api/mocks`). In B, the tray host (a stub
`StatusNotifierWatcher`; nobody sees an icon), the display (Xvfb, no GPU) and the
desktop (openbox, not GNOME/KDE, not Wayland), and the user (X11 client messages,
D-Bus calls and AT-SPI actions instead of clicks).


## Headless tests

```bash
npm run test:headless        # = npm run test:frontend && npm run test:rust
```

Runs the [Rust unit tests](#rust-unit-tests) and the
[frontend tests](#frontend-tests). Neither starts sotyware, opens a window or
talks to D-Bus, so no display, window manager, tray host or session bus is needed.
Exits non-zero if any test fails. The same command runs unchanged on native
Windows (PowerShell or `cmd.exe`, MSVC Rust); it does not need, and does not run,
the Linux-only desktop scripts. Results per platform:
[validation matrix](../status/status.md#validation-matrix).

## Rust unit tests

```bash
cd src-tauri && cargo test
```

Keep logic that can be tested without a running Tauri app in plain functions or
types (as `TrayAction::from_menu_id` is) and test it next to its code. Where a
decision triggers an app or window operation, it goes through a small trait
(`AppControl`, `DashboardWindow` in `lib.rs`) so the test can pass a recording
double ([ADR-009](../architecture/decisions.md#adr-009-lifecycle-decisions-are-plain-functions-behind-small-traits)).
The real window, tray and single-instance plugin cannot run under `cargo test`;
they need a display and an event loop.

## Frontend tests

```bash
npm run test:frontend
```

`src/App.test.ts` mounts the real `App.svelte` (and imports `main.ts`) in
[happy-dom](https://github.com/capricorn86/happy-dom) under Vitest, with Tauri's
official mocks (`mockWindows`, `mockIPC` from `@tauri-apps/api/mocks`) in place of
the Rust side. The mocked Rust side answers `operating_mode` with `"tray"` unless
a test says otherwise. It checks that:

- the page asks for the mode once on mount and makes no other call;
- in tray mode, "Hide dashboard" sends exactly `plugin:window|hide` for window
  `main` once per click, a failing or denied call shows the error in a
  `role="alert"`, and a successful retry clears it;
- in dashboard-only mode, a `role="status"` note says that no compatible tray is
  available, the dashboard cannot be hidden, the window must stay open, and
  closing it quits; there is no button and no hide call;
- if `operating_mode` fails or returns an unknown value, the error is shown and
  there is no Hide button;
- `main.ts` throws without `#app`.

It does **not** show that the OS window hides: no webview, no Rust, no
capability check runs. That is the job of the virtual-display test (scenario 12)
and the [Windows desktop test](#windows-desktop-test).

## Simulated desktop test

```bash
npm run tauri build -- --no-bundle
tests/desktop/run.sh [binary]
```

Files:

- `tests/desktop/run.sh`: creates a temporary private `$XDG_RUNTIME_DIR` and a
  private D-Bus session (`dbus-run-session`), then runs the Python driver with
  `GDK_BACKEND=x11`. A sotyware already running on your desktop is not affected.
  Exits 2 if the binary is missing.
- `tests/desktop/sni_watcher.py`: a minimal `org.kde.StatusNotifierWatcher`, so
  sotyware's tray (`ksni` backend) registers a tray item. It records each
  registration in a file that the driver counts. `--reject` answers every
  registration with `AccessDenied` (a broken watcher); `--no-host` reports
  `IsStatusNotifierHostRegistered = false`.
- `tests/desktop/single_instance.py`: drives the binary. It starts and stops the
  stub tray host itself, on the private bus, and waits for its bus name to appear
  or vanish instead of sleeping. It inspects X11 windows via `ctypes`, sends
  `WM_DELETE_WINDOW` to simulate the close button, clicks tray menu items over
  `com.canonical.dbusmenu`, reads and clicks the page inside the webview through
  AT-SPI (WebKitGTK's accessibility tree), and launches further instances
  sequentially and concurrently. Scenarios:
  - 1–9: tray mode: first launch, second launches while visible, hidden and
    minimized, sequential and concurrent launches, close → hide, tray Open
    Dashboard, tray Quit, relaunch.
  - 10: unexpected tray failure: a watcher that rejects the item. Exit 1, the
    `cannot create system tray icon` message, no claim of dashboard-only mode,
    nothing left running.
  - 11: `$XDG_RUNTIME_DIR` under a read-only directory (as under WSL): still
    starts with one tray item and quits with 0, because the `ksni` backend
    writes no icon file.
  - 12: tray mode, the page: tray hint and a "Hide dashboard" button; activating
    it through AT-SPI hides the window (so the runtime `tray-mode` capability
    works); tray Open Dashboard restores it; Quit.
  - 13: dashboard-only mode, session bus without a watcher: process alive and
    dashboard visible; mode and reason on stderr; no item registered and no
    `org.kde.StatusNotifierItem-<pid>-*` name owned; the page shows the no-tray
    note and no button (scenario 12 is the control that proves a button would
    be found); a second launch exits 0 and restores the minimized dashboard
    without a new window or process; a watcher started afterwards gets no
    registration (no retry); closing the dashboard exits 0 and leaves nothing.
  - 14: relaunch with the watcher back: tray mode again, close hides, Quit.
  - 15: watcher without a host (`--no-host`): dashboard-only mode, no item left
    served, close exits 0.
  - 16: no session bus at all: dashboard-only mode, close exits 0. Each walk of
  the X11 window tree holds a server grab, so it cannot miss a window that the
  window manager is moving out of its frame (the dashboard right after a close).
  Tray menu clicks do not wait for a D-Bus reply, because "Quit" ends the process
  while it handles the call; the checks look at the effect instead. If the first
  launch registers no tray item within 10 s, the driver reports a FAIL with the
  primary's output and stops, since every later scenario needs the tray menu; the
  relaunch after Quit has its own tray-registration check. The only fixed wait
  is the 2 s in scenario 13 that gives a late watcher the chance to receive a
  registration that must not come.

Requirements: `python3`, `python3-dbus`, `python3-gi`, the AT-SPI typelib
(`gir1.2-atspi-2.0`) and registry (`at-spi2-core`), `libX11`, `dbus-run-session`,
and an X11 display (`$DISPLAY`: WSLg or a desktop session; for a display of its
own use the [virtual-display test](#virtual-display-test)).

Results: each check prints `PASS`, `FAIL` or `SKIP`, then a summary line. `SKIP`
means the environment could not exercise the behaviour (for example, a window
manager that ignores minimize requests). **A SKIP is never a pass.** Do not turn
a SKIP into a PASS by loosening a check; record it as unverified instead.

The driver exits 1 if any check fails and 0 otherwise; SKIPs do not change the
exit code, so always read the summary line too. It aborts at once if a
`sotyware` process from the same binary is already running.

Under WSLg two checks are always skipped: focusing an already visible dashboard
(the WM ignores `_NET_ACTIVE_WINDOW`) and restoring a minimized dashboard (the WM
ignores `XIconifyWindow`). The [virtual-display test](#virtual-display-test) runs
both under openbox. The latest results are recorded in
[the validation matrix](../status/status.md#validation-matrix).

What this test is **not**: a real desktop. The tray host is a stub, nobody sees or
clicks an icon, and the window manager is whatever serves `$DISPLAY`.

## Virtual-display test

```bash
npm run tauri build -- --no-bundle
npm run test:desktop:virtual              # = tests/desktop/virtual.sh [binary]
```

`tests/desktop/virtual.sh` runs the [simulated desktop test](#simulated-desktop-test)
unchanged, against the same release binary, on a display it creates itself:

- **Xvfb** on a free display number (searched from `:99` down, skipping any with a
  lock file or socket), listening only on the abstract socket: no TCP and no file
  in `/tmp/.X11-unix`. `-displayfd` tells the script when it accepts clients.
- **openbox** as the window manager. It runs `--startup` once it manages the
  screen, which is the ready signal. Unlike WSLg's window manager it honours
  `_NET_ACTIVE_WINDOW` and `XIconifyWindow`, so the focus and un-minimize checks
  run instead of being skipped.
- `WAYLAND_DISPLAY` is unset and `DISPLAY` points at the private server, so
  nothing reaches the display you are using. `run.sh` still adds its private
  D-Bus session and runtime directory, and the driver its stub tray host.
- The whole run has a timeout (`SOTYWARE_VIRTUAL_TIMEOUT`, default 300 s). On
  exit, the script kills sotyware processes started on its display, openbox and
  Xvfb, and prints the Xvfb and openbox logs if the run did not pass.

Requirements (Debian/Ubuntu): `sudo apt install xvfb openbox python3-dbus python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-x11`.
A missing tool is reported by name and the script exits 2 without starting
anything. These packages alone were enough on Ubuntu 24.04.1 (versions in the
[validation matrix](../status/status.md#validation-matrix)); `xvfb` pulls in
`x11-xkb-utils` for `xkbcomp`. Xvfb and openbox print harmless warnings
(unresolved `XF86…` keysyms, no `debian-menu.xml`), and the xdg portals the
webview activates on the private bus warn about PipeWire and RealtimeKit; none
affects the result.

Interrupting the run (Ctrl+C, or SIGTERM to `virtual.sh`) cleans up like a normal
exit: nothing it started is left running and its temporary directories are
removed (checked 2026-11-10).

A FAIL on "one tray item" (first launch, or relaunch after Quit) is a real
defect, not a test flake. Do not retry it away. Until `12a6185`, it was the
intermittent
[libayatana-appindicator race](../status/limitations.md#historical-libayatana-appindicator-registration-race)
(`G_IS_DBUS_CONNECTION` in the instance output). The `ksni` backend avoids that
library, and with it no such FAIL has been seen.

Because the environment is fully controlled, **a SKIP fails this run**: the
script exits 1 if the summary reports any skipped check. Exit codes: 0 all
passed, 1 a check failed or was skipped, 2 binary or dependency missing, 3 Xvfb
or openbox did not start, 124 timeout.

What it does **not** validate: GNOME, KDE, XFCE or any real tray host, Wayland,
GPU rendering, what a person sees, or Windows.

## Windows desktop test

Run on Windows, from a Windows checkout (not a WSL path), after a release build:

```powershell
npm run tauri build -- --no-bundle
npm run test:desktop:windows              # = powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1
npm run test:desktop:windows -- -Exe <path>
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
  so the test opens it first. It treats the flyout as open only while it is the
  foreground window: when hiding the dashboard hands the foreground to another
  app, the flyout starts closing but UI Automation still lists it and its icons,
  so the test waits for it to close and opens it again. It left-clicks the icon
  (expects the dashboard and no menu) and right-clicks it for the menu.
- Reads the tray menu through the Win32 menu handle (`MN_GETHMENU`), because UI
  Automation exposes none of its items. It clicks items at their
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

Before it starts sotyware it checks that it runs on the desktop that receives
input (`OpenInputDesktop` names the same desktop as its own thread's) and that
this desktop has a taskbar (`Shell_TrayWnd`). If not (a locked or disconnected
session, a service, a private desktop, an SSH session) it prints `ABORT` with the
reason and exits 3 without running any check, instead of reporting failures that
say nothing about sotyware. Verified 2026-11-10 by running it on a private
desktop: exit 3, no sotyware process started.

Results: `PASS`/`FAIL` per check, `INFO` for observations Windows may legitimately
vary (whether the dashboard ended up in the foreground), then a summary line. Exit
code 0 if all passed, 1 if any failed, 2 if the binary is missing or a sotyware
from the same path is already running, 3 if there is no interactive desktop. It does not catch an icon that is in the
tray but invisible to a person; the [manual check](#manual-check-real-desktop-linux-or-windows)
still covers looking at it.

Verified that it detects a broken button: with `hideDashboard` changed to do
nothing, all three hide checks failed (2026-11-10).

## Windows isolated-desktop test

```powershell
npm run tauri build -- --no-bundle
npm run test:desktop:windows:isolated              # = powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-isolated.ps1
npm run test:desktop:windows:isolated -- -Exe <path>
```

`tests/desktop/windows-isolated.ps1` creates a new desktop (`CreateDesktop`) in the
session's window station and starts a copy of itself on it (`CreateProcess` with
`lpDesktop`). That copy starts sotyware, which inherits the desktop, and runs the
checks; the launcher prints the copy's output and returns its exit code. The
desktop is never switched to, so nothing appears on your screen, and the test
sends no mouse or keyboard input: you can keep working while it runs (about
40 seconds).

It checks, against the real release binary and WebView2: startup (one process,
one visible `Tauri Window` titled sotyware); the "Hide dashboard" button read and
invoked through UI Automation inside WebView2 (hidden, process alive, window not
destroyed); second launches while hidden, visible and minimized (exit 0, same
pid, one window, shown or restored); `WM_CLOSE` (what ✕ and Alt+F4 send) and UIA
`WindowPattern.Close` hide instead of quitting; 3 sequential and 4 concurrent
launches; Hide still works afterwards. It records the primary's stderr as INFO.

What it **cannot** cover, and why: a private desktop has no taskbar, so
`Shell_NotifyIcon` has no notification area to add to. sotyware still starts in
tray mode, because `tray-icon` keeps its window and waits for Explorer's
`TaskbarCreated` message (which never comes there), so the Hide button exists,
but there is no tray icon, tray menu, left click or Quit to test. The test ends
sotyware with `Stop-Process` (cleanup, not a check of Quit). Real mouse clicks,
focus and the title-bar ✕ position are not tested either. Those stay in the
[Windows desktop test](#windows-desktop-test).

It is **not headless**: it needs a logged-in session (window station `WinSta0`)
to create the desktop and windows in, but not an unlocked or visible screen.
Before any check, the copy verifies that it really runs on the private desktop
and exits 3 otherwise. Exit codes: 0 all passed, 1 a check failed, 2 binary
missing or already running, 3 the private desktop could not be created or used,
124 timeout (`-Timeout`, default 180 s).

## Headless Windows desktop testing

Investigated 2026-11-10 on Windows 11 Home 25H2. What each part of
`windows.ps1` needs:

| Part of the Windows desktop test | Needs |
| --- | --- |
| Starting sotyware, the dashboard window, WebView2 rendering | a window station and desktop in a logged-in session |
| Reading and invoking the page through UI Automation; window state through `user32` | the same desktop as the test (`EnumWindows` lists only the caller's desktop; the test drives UIA from that desktop too) |
| Second launch, concurrent launches | the same desktop: the single-instance plugin hands over through a named mutex and `FindWindowW` + `WM_COPYDATA`, and `FindWindowW` searches only the caller's desktop |
| Tray icon, menu (`MN_GETHMENU`), left click, Quit, icon removed | Explorer's notification area (`Shell_TrayWnd`) on that desktop |
| Real mouse clicks (Hide, tray, title-bar ✕) | the **input** desktop: unlocked, not disconnected, not minimised away (RDP) |

Findings:

- Nothing that starts sotyware runs without a session: there is no Windows
  equivalent of Xvfb. `npm run test:headless` (category A) is the only headless
  Windows suite; it runs on Windows unchanged and needs no desktop.
- A private desktop in the logged-in session runs everything except the tray and
  real input, without touching the screen: that is the
  [isolated-desktop test](#windows-isolated-desktop-test).
- The tray and real-click checks need the interactive, unlocked desktop with
  Explorer. `windows.ps1` detects when it does not have one and exits 3 instead
  of reporting failures.
- Not tried: Session 0 (a service or a scheduled task set to run whether the
  user is logged on or not). Registering such a task was not permitted in the
  investigating session. Windows services cannot show UI and Session 0 has no
  Explorer, so the tray checks cannot run there in any case; whether WebView2 and
  the isolated-desktop test work there is unknown.
- GitHub-hosted Windows runners: not tried. Public reports disagree on whether
  their job runs in an interactive desktop session with Explorer. Until a trial
  job shows otherwise, treat them as headless only (category A), and run the
  isolated-desktop test there only as an experiment: its exit code 3 tells you
  the environment could not run it.

## CI

There is no CI configuration in the repository yet. When one is added, run the
categories as separate jobs so a missing graphical package never blocks the
headless results:

| Job | Runner | Install | Run |
| --- | --- | --- | --- |
| headless | `ubuntu-24.04` | the Tauri build libraries from [setup.md](setup.md#linux-system-packages), Node, Rust | `npm ci`, `npm run check`, `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (in `src-tauri/`), `npm run test:headless` |
| headless | `windows-latest` | Node, Rust (MSVC) | same commands |
| private desktop (trial) | `windows-latest` | Node, Rust (MSVC) | `npm run tauri build -- --no-bundle`, `npm run test:desktop:windows:isolated`; exit 3 means the runner has no usable session, which must fail the job, not pass it |
| real desktop | self-hosted Windows runner, started with `run.cmd` in an auto-logged-on user session (not installed as a service), screen never locked, nobody using it | Node, Rust (MSVC) | `npm run tauri build -- --no-bundle`, `npm run test:desktop:windows` |
| virtual display | `ubuntu-24.04` | additionally `xvfb openbox python3-dbus python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-x11` | `npm run tauri build -- --no-bundle`, `npm run test:desktop:virtual` |

The headless job still compiles the Tauri crate, so on Linux it needs the
build packages from [setup.md](setup.md#linux-system-packages), but no display server, window
manager, tray host or D-Bus session. A CI job for the virtual display must also
fail on `SKIP` in the summary if the goal is to prove focus and minimize; the exit
code alone ignores SKIPs. The Windows desktop test needs an interactive, unlocked
desktop session with Explorer and moves the mouse; a runner installed as a
service runs in Session 0, which has neither. None of this has been run in an
actual CI service.

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

Dashboard-only mode (Linux desktops without a tray host, for example vanilla
GNOME without the AppIndicator extension):

9. Start sotyware. Expected: the dashboard opens, shows the note that no
   compatible system tray is available, and has no Hide button. No tray icon.
10. Minimize it, start sotyware again. Expected: the same dashboard comes back,
    still one process.
11. Close the dashboard with ✕. Expected: the process is gone
    (`pgrep -a sotyware` prints nothing).
12. Enable a tray host (for example the AppIndicator extension) and start
    sotyware again. Expected: tray mode, as in steps 1–7.

## Rules

- Never weaken, skip or delete a check to make a change pass. If a check is wrong,
  fix it in its own commit and explain why.
- Report what was actually run, on which environment, and what remains unverified.
- When behaviour changes, update the desktop test and the manual checklist in the
  same change.

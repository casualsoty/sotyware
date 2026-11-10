# Implementation status

Last updated: 2026-11-10. Update this file whenever
implementation or validation changes. Never mark something validated that was
not run on that environment.

## Current milestone

**Milestone 0: prove the desktop runtime.** Show that the stack (Tauri 2 +
Svelte 5) and the background lifecycle (tray, hide-on-close, explicit Quit,
single instance) work on Windows 10/11 and Linux desktops, before any product
features are built.

State: implemented. Validated natively on **Windows 11** (25H2, build 26200.9711),
sotyware's primary runtime platform, by an automated test on the real desktop, the
headless suite and the NSIS-installed copy, last at `e8834fd` (application code
of `8ca3bd7`, with the Windows test fix `71442d3`;
[details](#dashboard-only-mode-regression-run-native-windows-11)). Linux is
validated only in simulated environments (WSL2/WSLg; Xvfb + openbox), never on
a real Linux desktop. Open: Windows 10, a person's visual check on Windows, the
MSI installer, and real Linux desktops. The Linux tray uses `tray-icon`'s `ksni`
backend since `12a6185`, which avoids the libayatana-appindicator registration
race ([details](#ksni-tray-backend-wsl2-xvfb--openbox-stub-tray-host)). Since
`8ca3bd7`, Linux without a reachable tray host runs in dashboard-only mode
instead of exiting 1 ([details](#dashboard-only-mode-wsl2-xvfb--openbox-stub-tray-host)).
Dashboard-only mode cannot occur on Windows, where every tray error stays fatal.
On 2026-11-10 (at `0131e09`) no real Linux desktop was available to test on
([survey](#real-linux-desktop-availability-2026-11-10)), and the Windows tests
were split into headless, private-desktop and real-desktop categories
([results](#windows-test-categories-native-windows-11-0131e09)).

## Implemented

| Feature | Where |
| --- | --- |
| Tauri 2 desktop runtime (one process, one window `main`) | `src-tauri/src/lib.rs`, `tauri.conf.json` |
| Svelte 5 dashboard (static page; "Hide dashboard" button in tray mode, a no-tray note in dashboard-only mode) | `src/App.svelte` |
| Operating modes: tray mode, and dashboard-only mode on Linux without a tray host (close quits, no hide) | `select_mode`, `close_action` in `lib.rs`; `unavailable_reason` in `tray.rs` |
| `operating_mode` command (the frontend's only custom call) | `lib.rs`, app manifest in `build.rs` |
| System tray with "Open Dashboard" and "Quit" | `src-tauri/src/tray.rs` |
| Linux tray backend: `tray-icon`'s `ksni` (pure Rust, no libayatana-appindicator) | Linux-only `tray-icon` dependency in `src-tauri/Cargo.toml` |
| Hide-on-close lifecycle (tray mode) | `on_window_event` in `lib.rs` |
| Only explicit exits (tray Quit; close in dashboard-only mode); implicit exits blocked | `app.run` closure and `on_window_event` in `lib.rs`, `tray.rs` |
| Fatal exit (code 1) if the tray cannot be created for a reason other than a missing tray host | `setup` in `lib.rs` |
| Single instance via `tauri-plugin-single-instance` | `run()` in `lib.rs` |
| Minimal capability (`allow-operating-mode`; `core:window:allow-hide` added at runtime in tray mode only) and CSP | `capabilities/default.json`, `setup` in `lib.rs`, `tauri.conf.json` |

Automated tests per platform: the "Hide dashboard" button with a real mouse
click, and left-click-to-open, are covered only by the Windows desktop test
(`tests/desktop/windows.ps1`). On Linux the virtual-display test activates the
button through AT-SPI and reads the dashboard-only page the same way. Restoring
a minimized dashboard and focus on second launch run on Linux only under the
virtual display (Xvfb + openbox); WSLg skips them. Test categories:
[testing.md](../development/testing.md).

Nothing else exists: no product features, no settings, no persistence, no
custom Tauri events, and no custom command other than `operating_mode`.

## Validation matrix

Legend: ✅ passed · ⏭️ skipped by the environment (not a pass) · ❌ failed ·
— not run.

Each result belongs to one test category
([testing.md](../development/testing.md)): **headless** (no display, nothing
real from the OS), **virtual display** (real release binary, Xvfb + openbox, stub
tray host), **simulated desktop** (real release binary on WSLg, stub tray host) or
**real desktop** (Windows test, manual check). A pass in one category says nothing
about another.

"Re-run 2026-11-10" means the check was repeated while writing this document.
"Native Windows 2026-11-10" means it ran on the Windows 11 machine described below.
"Earlier run" means it was recorded in the original README from a run whose
script or procedure is not in the repository; it was not repeated.

### Static checks and builds (WSL2, Ubuntu 24.04, Rust 1.99.1, Node 24.22.0)

| Check | Result | When |
| --- | --- | --- |
| `npm run check` (0 errors, 0 warnings) | ✅ | Re-run 2026-11-10 |
| `npm run build` | ✅ | Re-run 2026-11-10 |
| `cargo fmt --check` | ✅ | Re-run 2026-11-10 |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Re-run 2026-11-10 |
| `cargo test` (1 test) | ✅ | Re-run 2026-11-10, before the headless suite |
| `npm run tauri build -- --no-bundle` | ✅ | Re-run 2026-11-10 |
| `npm run tauri build`: `.deb`, `.rpm`, AppImage built (with WSL `PATH` filter) | ✅ built, not installed or run | Re-run 2026-11-10 |
| `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ✅ (the first is no longer needed since `12a6185`) | Re-run 2026-11-10, again at `12a6185` |

### Headless tests (WSL2, Ubuntu 24.04, Rust 1.99.1, Node 24.22.0)

Category A. Run under `env -i` with only `HOME`, `PATH` and a private `TMPDIR`:
no `DISPLAY`, `WAYLAND_DISPLAY` or D-Bus session. Nothing here starts sotyware or
touches the OS window system.

| Check | Result | When |
| --- | --- | --- |
| `npm run test:headless`: exit 0 | ✅ | 2026-11-10 |
| `cargo test`: 17 tests (close → hide, implicit exit blocked, tray failure → exit 1, reveal order and error tolerance, second launch, tray menu and click dispatch, config labels) | ✅ | 2026-11-10 |
| `npm run test:frontend`: 8 tests (Hide button sends `plugin:window\|hide` for `main`, errors shown and cleared, `main.ts` mount) | ✅ | 2026-11-10 |
| Not vacuous: 5 Rust and 4 `App.svelte` hand-made mutations each fail ≥ 1 test; `test:headless` exits non-zero for both halves | ✅ | 2026-11-10 |

The same suite on native Windows is in
[Headless tests, native Windows 11](#headless-tests-native-windows-11).

### Native Windows 11: environment

Windows 11 Home, version 25H2, OS build 26200.9711 (the registry `ProductName`
still says "Windows 10", a known Windows quirk; build 26200 is Windows 11). x64.
Checkout `C:\dev\sotyware`, a fresh `git clone` of `origin` at `f8174da` on the
NTFS drive (not a WSL path), `core.autocrlf=true`. Commands were started from WSL
through Windows interop, but compiled and ran as native Windows processes in the
interactive desktop session.

**Regression run at `1f5853b` (2026-11-10).** The same checkout was fast-forwarded
from `f8174da` to `1f5853b` (the headless-test commits, not yet pushed) with a
`git bundle` made in the WSL repository, so the commit hashes are identical
([how](../development/setup.md#testing-unpushed-commits-on-windows)). Every command
ran in Windows PowerShell 5.1 with the machine and user `PATH` from the registry
(the `PATH` inherited through WSL interop does not contain `~\.cargo\bin`), so
`node.exe`, `cargo.exe` and `npm` were the Windows binaries listed below. No
`DISPLAY`, `WAYLAND_DISPLAY` or D-Bus variables were set. Tools were the same
versions as in the table; additionally rustup 1.29.3 and Git 2.47.1.windows.1.

| Tool | Version |
| --- | --- |
| Rust / Cargo | 1.99.1 / 1.99.1, `stable-x86_64-pc-windows-msvc` (rustup) |
| MSVC | Visual Studio Community 2022, C++ build tools |
| Node.js / npm | 24.14.1 / 11.12.1 |
| Tauri CLI | 2.12.4 (from `npm ci`) |
| WebView2 Runtime | 157.0.4341.42 |
| Windows PowerShell (test) | 5.1 |

### Dashboard-only mode regression run, native Windows 11

Run 2026-11-10 at `e8834fd`, the first native Windows run of the shared Rust
changes from `8ca3bd7` (operating mode, the `operating_mode` command, and hide
granted through the runtime `tray-mode` capability instead of the static
capability file). `c2cbeb4` and `e8834fd` change only Linux tests and docs. The
Windows checkout had already been fast-forwarded from `5dbafd8` to `e8834fd`
with a `git bundle` (identical hashes, clean tree); the test fix `71442d3` was
moved the same way. Same machine and procedure as above: Windows 11 Home 25H2,
OS build 26200.9711, x64; Windows PowerShell 5.1 started from WSL through
interop with the registry `PATH`, so every tool was the native Windows binary:
Rust/Cargo 1.99.1 (`stable-x86_64-pc-windows-msvc`), Visual Studio Community 2022
17.12 C++ tools, Node.js 24.14.1, npm 11.12.1, Tauri CLI 2.12.4, Git
2.47.1.windows.1. WebView2 Runtime is now 157.0.4341.46 (was .42).

| Check | Result |
| --- | --- |
| `npm ci`, `npm run check` (0 errors, 0 warnings), `npm run build` | ✅ |
| `npm run test:headless`: exit 0; 13 frontend tests and 23 Rust tests. The other 8 of the 31 Rust tests classify `ksni`/`zbus` errors and are `cfg(target_os = "linux")`, so they are not compiled for Windows | ✅ |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | ✅ |
| `cargo test` (MSVC): 23 tests in `sotyware_lib`, 0 in the binary, 0 doc-tests | ✅ |
| Windows graph (`cargo tree --target x86_64-pc-windows-msvc -e features`): no `ksni`, `zbus`, `gtk` or `libappindicator` crate; `sotyware` depends only on `tauri` 2.12.5 and `tauri-plugin-single-instance` 2.5.5. `tray-icon` has no `ksni` feature there (the Linux-only declaration does not apply) | ✅ |
| `npm run tauri build`: `sotyware.exe` (4,341,248 bytes), NSIS `sotyware_0.1.0_x64-setup.exe` (1.41 MiB), MSI `sotyware_0.1.0_x64_en-US.msi` (2.04 MiB); no capability or permission error at build time | ✅ built |
| `tests/desktop/windows.ps1` as committed at `e8834fd`, release binary | ❌ 4 of 4 runs failed: 78 passed / 2 failed three times, 71 / 9 once. Test defect, not a sotyware regression (below) |
| Same unchanged script on a release build of `1f5853b` (`git archive` outside the checkout), which had passed 80/80 earlier that day | ❌ 71 / 9, then 78 / 2: the same failure, so not caused by the new commits |
| `tests/desktop/windows.ps1` at `71442d3`, release binary | ✅ **80 passed, 0 failed**, 3 consecutive runs (plus 4 runs of the same change before committing it and 1 on the `1f5853b` build, all 80 / 0) |
| Supplementary scratch script (not in the repository), release binary with stderr captured: 5 × "Hide dashboard" (hidden, then a second launch restores it with exit 0, no error or permission text in the page), 3 × ✕ (hidden, process alive; tray "Open Dashboard" restores the same window; one window, one process), tray "Quit" exits 0, no process left, stderr without any permission or ACL message | ✅ 29 / 29 |
| NSIS `/S` install: exit 0; `%LOCALAPPDATA%\sotyware\` (`sotyware.exe`, `uninstall.exe`); HKCU uninstall entry "sotyware" 0.1.0; Start menu and desktop shortcuts pointing at the installed exe | ✅ |
| Installed copy: `tests/desktop/windows.ps1` (`71442d3`) | ✅ 80 / 0, 2 runs |
| Installed copy: supplementary script | ✅ 29 / 29 |
| Installed copy: the Start menu shortcut starts it; after Hide, the desktop shortcut shows the dashboard with the same pid and one window; one tray icon; tray "Quit" leaves no process | ✅ 8 / 8 |
| NSIS uninstaller `/S`: exit 0; install directory, uninstall entry and both shortcuts removed; no process left | ✅ |
| `HKCU\Software\sotyware\sotyware` (default value = install directory) after uninstall | ⚠️ remains. It was already there before this install, left by the earlier install/uninstall, and this uninstall did not remove it ([limitations](limitations.md#windows-installers)) |
| WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` | ⚠️ still present, as after the earlier uninstall |
| MSI | — not run (per-machine, needs an elevated install) |

**Runtime hide permission works on Windows.** The `core:window:allow-hide`
permission granted in `setup` reaches the dashboard: every Hide check in the
suite passed (UIA invoke and a real mouse click), as did the 5 supplementary
hides on both the build output and the installed copy, with no error shown in the
page. The only stderr line, at every quit, is WebView2's
`ERROR:ui\gfx\win\window_impl.cc:172] Failed to unregister class Chrome_WidgetWin_0. Error = 1412`,
which does not affect behaviour.

**Why the committed test failed.** The failing check was always the tray menu
right after the title-bar ✕ ("tray menu Open Dashboard again: invoked: context
menu did not open"), and in two runs also the tray "Quit". An instrumented copy
showed the cause: hiding the dashboard handed the foreground to another app's
window (`Chrome_WidgetWin_1`, a browser open behind it). The "Show Hidden Icons"
flyout lost focus and started closing, but UI Automation still reported it and
the sotyware icon with a valid rectangle. The test therefore skipped reopening
it and right-clicked the vanishing flyout. The earlier 80/80 runs had no such
window behind the dashboard (the machine had since rebooted). `71442d3` counts
the flyout as open only while it is the foreground window and waits out a
closing one. No check was removed and no timeout raised.

Not run on Windows: dashboard-only mode (it cannot occur there), Alt+F4, the
interactive NSIS UI, the MSI, a person's visual check, and Windows 10.

### Windows test categories, native Windows 11 (`0131e09`)

Run 2026-11-10 on the machine and toolchain described
[above](#dashboard-only-mode-regression-run-native-windows-11) (Windows 11 Home
25H2, build 26200.9711, WebView2 157.0.4341.46). The Windows checkout
`C:\dev\sotyware` was fast-forwarded from `798e312` to `195fa8a` and then
`0131e09` with `git bundle` (identical hashes, clean tree). Release binary
rebuilt at `195fa8a` (`npm run tauri build -- --no-bundle`); `0131e09` changes
only the test script. Commands started from WSL through interop in PowerShell 5.1
with the registry `PATH`. Categories: [testing.md](../development/testing.md).

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0 errors, 0 warnings) | static | ✅ |
| `npm run test:headless`: 13 frontend + 23 Rust tests, exit 0 | headless | ✅ (`195fa8a`) |
| `npm run test:desktop:windows:isolated` (private desktop, no input, nothing on screen) | private desktop | ✅ **56 passed, 0 failed**, 3 consecutive runs at `195fa8a` and 3 more at `0131e09`; primary's stderr empty |
| Same suite on a mutant build (outside the checkout, from `git archive`) whose close quits in tray mode | private desktop | ✅ detected: 43 passed, 9 failed, exit 1 at `0131e09`. At `195fa8a` it also failed 9 checks but exited 0, a launcher bug fixed in `0131e09` |
| Same suite with `-Timeout 5` | private desktop | ✅ exit 124, no sotyware process left |
| `npm run test:desktop:windows` (`windows.ps1`, real desktop, real mouse) | real desktop | ✅ **80 passed, 0 failed**, 3 consecutive runs at `195fa8a` |
| `windows.ps1` started on a private desktop (no input desktop, no taskbar) | guard | ✅ `ABORT`, exit 3, no sotyware process started (pre-commit copy of `44a131d`'s script) |
| Isolated-desktop probe (scratch script, not in the repository): on a private desktop sotyware starts in tray mode, the WebView2 page is readable through UIA, Hide, second launch and `WM_CLOSE` behave as on the real desktop; no `Shell_TrayWnd`, so no tray icon | investigation | ✅ observed |
| Session 0 (scheduled task "run whether the user is logged on or not") | investigation | — not run: registering the task was not permitted in the investigating session |
| A GitHub-hosted Windows runner | CI | — not run yet: the manual workflow `desktop-windows-isolated.yml` exists since `5fe62b9` but has not been started ([CI](#ci-github-actions-2026-11-10)) |

Conclusion: no Windows test of the real binary runs headlessly. The
private-desktop suite runs unattended inside a logged-in session without using
the screen or the input devices; the tray, real clicks and Quit need the
interactive desktop. Details:
[testing.md](../development/testing.md#headless-windows-desktop-testing).

### CI: GitHub Actions (2026-11-10)

Added in `5fe62b9`; details in [testing.md](../development/testing.md#ci). The
repository is private on GitHub Free ($12 of hosted compute a month).

| Workflow | Trigger | Result |
| --- | --- | --- |
| `ci.yml` headless (Linux, Windows) | automatic: pull request / push to `main` | — **not run on GitHub yet** (nothing pushed). Its command sequence passed locally on WSL2 Ubuntu 24.04 at `5fe62b9` (`npm ci`, check 0/0, build, `cargo fetch --locked`, fmt, clippy, 13 frontend + 31 Rust tests) |
| `desktop-virtual.yml` | manual | — not run on GitHub. `npm run test:desktop:virtual` locally on WSL2 at `5fe62b9`: 92 passed, 0 failed, 0 skipped (matches `EXPECTED_CHECKS`) |
| `desktop-windows-isolated.yml` | manual, experimental | — not run on GitHub; **GitHub-hosted support unknown** |
| Workflow syntax | — | YAML parses, every action pinned to a full SHA; not checked with `actionlint` and not accepted by GitHub yet |

Record the first GitHub results here, with the real job durations, and replace
the cost estimate in [testing.md](../development/testing.md#github-free-budget).

### Real Linux desktop availability (2026-11-10)

Surveyed on the development machine before planning any Linux desktop run.

| Looked for | Found |
| --- | --- |
| WSL distributions | only `Ubuntu` (24.04.1), the development environment; WSLg serves its display (`DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`), no desktop session (`XDG_CURRENT_DESKTOP` empty) |
| Desktop environments in that distribution | none installed (no `gnome-shell`, `plasmashell`, `xfce4-panel`, `gdm3`, `lightdm`, `mutter`) |
| Virtual machines on the Windows host | none: Windows 11 **Home** (no Hyper-V Manager, no `Get-VM`), no VirtualBox or VMware installed |
| Other Linux machines | none configured (no SSH hosts) |
| Docker Desktop | present, but runs Linux containers without a desktop session or tray host; not a real desktop |

**No real Linux desktop was tested.** GNOME, KDE Plasma, XFCE and real Wayland
stay unvalidated; the only Linux results remain the virtual-display and
simulated-desktop runs below, which use a stub tray host. Installing a full
desktop environment inside WSL was not done: it would still run under WSLg's
compositor with no real shell tray host, so it would not be a real desktop
either, and it would add several hundred packages to the development
environment. Options that would give a real desktop: a VirtualBox (or VMware
Workstation) VM with Ubuntu Desktop (GNOME), a live USB session on real
hardware, or a separate Linux machine. Each needs the user's decision.

### Headless tests, native Windows 11

Category A at `1f5853b`. Run with `TEMP`/`TMP` pointed at a new empty directory,
no sotyware process running and none installed.

| Check | Result | When |
| --- | --- | --- |
| `npm run test:headless`: exit 0 (34 s, Rust test build partly cached) | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `npm run test:frontend` (Vitest 4.1.12, happy-dom): 8 tests | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `cargo test` (MSVC): 17 tests in `sotyware_lib`, 0 in the binary, 0 doc-tests | ✅ | Native Windows 2026-11-10, `1f5853b` |
| Not vacuous: `App.svelte` without the `hide()` call → 5 of 8 frontend tests fail, exit 1; `close_action` returning `Allow` for `main` → `closing_the_dashboard_hides_it` fails, exit 101; `exit_allowed` always `true` → `implicit_exit_is_blocked` fails, exit 101. Each mutation was a temporary, uncommitted edit, restored right after | ✅ | Native Windows 2026-11-10, `1f5853b` |
| Side effects: nothing left in the private `TEMP` except Node's `node-compile-cache` (enabled by Vite's CLI, also on Linux); no sotyware process started; the WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` unchanged (file list, sizes and times compared); `git status` clean | ✅ | Native Windows 2026-11-10, `1f5853b` |

No Unix-only command is on this path: the npm scripts use only `&&`, which
`cmd.exe` (npm's default script shell on Windows) supports. No test writes
temporary files or reads runtime environment variables; the only paths are the
two Rust config tests, which read `tauri.conf.json` and
`capabilities/default.json` as `CARGO_MANIFEST_DIR` + `/` + name (Windows accepts
the mixed separator) from the CRLF checkout.

### Static checks and builds, native Windows 11

| Check | Result | When |
| --- | --- | --- |
| `npm ci` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `npm run check` (0 errors, 0 warnings) | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `npm run build` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo fmt --check` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo clippy -- -D warnings` | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo test` | ✅ 1 test at `f8174da`; 17 tests at `1f5853b` | Native Windows 2026-11-10 |
| `npm run tauri build`: `sotyware.exe` (3.2 MB), `sotyware_0.1.0_x64-setup.exe` (NSIS, 1.13 MiB), `sotyware_0.1.0_x64_en-US.msi` (1.64 MiB) | ✅ built; same sizes at `1f5853b` (154 s) | Native Windows 2026-11-10; re-run at `1f5853b` |

### Runtime behaviour, native Windows 11 (real desktop)

`tests/desktop/windows.ps1` on the release binary: **80 passed, 0 failed**, run
three times in a row with the same result (2026-11-10). Earlier revisions of the
same script (78 checks) also passed on four further consecutive runs. No sotyware
defect was found.

Re-run at `1f5853b` after the headless-test commits (which moved the lifecycle
decisions into plain functions): **80 passed, 0 failed** on the release binary
and **80 passed, 0 failed** on the NSIS-installed copy (2026-11-10), one run each.
No regression against the earlier 80/80.

Re-run at `e8834fd` (dashboard-only mode, hide granted at runtime) with the test
fix `71442d3`: **80 passed, 0 failed**, 3 runs on the release binary and 2 on the
NSIS-installed copy ([details](#dashboard-only-mode-regression-run-native-windows-11)).
The table below holds for all three commits.

| Behaviour | Result | Source |
| --- | --- | --- |
| First launch: dashboard visible, title "sotyware", one process, one window | ✅ | Windows test |
| Exactly one sotyware notification-area icon (it lands in the hidden-icons flyout) | ✅ | Windows test |
| Tray menu (right click) contains exactly "Open Dashboard", "Quit" | ✅ | Windows test (read via `MN_GETHMENU`) |
| Tray left click opens the dashboard and shows no menu | ✅ | Windows test (real mouse click) |
| "Hide dashboard" button hides the window; window not destroyed; process stays | ✅ | Windows test (UIA invoke and real mouse click) |
| Tray "Open Dashboard" restores the same single window | ✅ | Windows test (real mouse click on the item) |
| Title-bar close button (✕) hides instead of exiting; process stays | ✅ | Windows test (real mouse click) |
| Programmatic close request (`WindowPattern.Close`) hides; process stays | ✅ | Windows test |
| Alt+F4 | — not sent (see testing.md) | |
| Second launch while hidden: exits 0; same single process and window; dashboard shown | ✅ | Windows test |
| Second launch while visible: exits 0; no new process or window | ✅ | Windows test |
| Second launch while minimized: exits 0; dashboard restored | ✅ | Windows test |
| Dashboard is the foreground window after a second launch (hidden, visible, minimized) and after a tray left click | ✅ observed every run | Windows test (INFO lines) |
| 5 sequential + 4 concurrent launches: all exit 0; one process, window and tray icon | ✅ | Windows test |
| Hide, then tray "Quit": exits 0; no process and no tray icon left | ✅ | Windows test |
| Relaunch after Quit starts a fresh primary; Quit exits 0 again | ✅ | Windows test |
| A test with the button made a no-op fails the 3 hide checks | ✅ (test is not vacuous) | Mutation run |
| A person looked at the tray icon and menu and clicked them | — not yet done | |

Focus note: the second launches were started by a background script, and the
dashboard still became the foreground window every time. Windows' foreground-lock
rules can behave differently when another app is actively in use; that case was
not tested.

### Installers, native Windows 11

| Step | Result | When |
| --- | --- | --- |
| NSIS `sotyware_0.1.0_x64-setup.exe /S`: exit 0; per-user install to `%LOCALAPPDATA%\sotyware` (`sotyware.exe`, `uninstall.exe`); HKCU uninstall entry "sotyware" 0.1.0; Start menu and desktop shortcuts | ✅ | Native Windows 2026-11-10 |
| Installed copy: `tests/desktop/windows.ps1 -Exe %LOCALAPPDATA%\sotyware\sotyware.exe` | ✅ 80 passed, 0 failed with the committed script; an earlier install also passed the 78-check revision | Native Windows 2026-11-10 |
| At `1f5853b`: NSIS `/S` install (exit 0), installed copy 80 passed / 0 failed, uninstaller `/S` (exit 0; install dir and HKCU uninstall entry removed). The installed `sotyware.exe` has the same size as the build output but a different SHA-256; not investigated | ✅ | Native Windows 2026-11-10 |
| NSIS uninstaller `/S` after the app had quit: exit 0; files, uninstall entry and shortcuts removed | ✅ | Native Windows 2026-11-10 |
| Start menu shortcut launches the installed exe; a second shortcut launch keeps one process | ✅ | Native Windows 2026-11-10 |
| NSIS uninstaller `/S` while sotyware was running: exit 0; process ended; install dir, uninstall entry and both shortcuts removed | ✅ | Native Windows 2026-11-10 |
| WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` after uninstall | ⚠️ still present; it was also created by earlier runs of the build output, so this does not show whether the uninstaller would remove it | Native Windows 2026-11-10 |
| NSIS installer with its interactive UI | — not run (silent mode only) | |
| MSI `msiexec /i … /qn` | ❌ not installed: exit 1603, "Installation UI level is silent, no credential elevation is possible". The MSI installs per machine and needs administrator rights (UAC). Not a sotyware defect; MSI install, run and uninstall remain unvalidated | Native Windows 2026-11-10 |

### Dashboard-only mode (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `c2cbeb4` (implementation `8ca3bd7`,
[ADR-012](../architecture/decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode),
[ADR-013](../architecture/decisions.md#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode)),
same machine and apt packages as the runs below, plus `gir1.2-atspi-2.0` and
`at-spi2-core` (already installed). Release binary built from `8ca3bd7`.

| Check | Result |
| --- | --- |
| `npm run check` 0/0, `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run tauri build -- --no-bundle` | ✅ |
| `cargo test`: 31 tests (17 before, adapted: close action per mode, `select_mode`; new: mode selection from the real `ksni`/`zbus` error values, what stays fatal, hide granted only in tray mode, dashboard starts visible, capability) | ✅ |
| `npm run test:frontend`: 13 tests (8 before, adapted to the mode request; new: dashboard-only note, no Hide action, failed or unknown mode) | ✅ |
| `npm run test:headless` (31 Rust + 13 frontend) | ✅ |
| `npm run test:desktop:virtual`, 92-check driver: **92 passed, 0 failed, 0 skipped**, 3 consecutive runs, no sotyware process left | ✅ |
| Same suite with the 68-check driver from before, on the new binary: 63 passed, then scenario 10 hung (a missing watcher no longer exits 1) | ❌ expected, driver adapted in `c2cbeb4` |
| Error shapes from the real binary, before the change: no bus ⇒ `D-Bus connection error: Failed to connect to address …: No such file or directory`; no watcher ⇒ `…ServiceUnknown…` | ✅ matches the classification |
| `Cargo.lock`: only `ksni` and `zbus` added to `sotyware`'s dependency list, no new crate. `cargo tree --target x86_64-pc-windows-msvc`: `sotyware` still depends only on `tauri` and `tauri-plugin-single-instance`; `ksni` and `zbus` are not in the Windows graph | ✅ |

What the 24 new virtual-display checks cover: [testing.md](../development/testing.md#simulated-desktop-test)
(scenarios 12–16). The tray-mode checks of scenarios 1–9 and 11 are unchanged.
Scenario 10 now uses a watcher that rejects the item instead of no watcher.

Native Windows was run afterwards, at `e8834fd`:
[results](#dashboard-only-mode-regression-run-native-windows-11). Not run:
`run.sh` on WSLg, a real GNOME or any real Linux desktop, an
XEmbed-only panel, Wayland, and a person looking at the dashboard-only note.

### ksni tray backend (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `12a6185` (Linux tray switched to `tray-icon`'s `ksni` backend,
[ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)),
same machine and apt packages as the run below. Release binary
`src-tauri/target/release/sotyware`, sha256 `4dd03869…54176` (4,976,808 bytes;
the libayatana build `10c5e588…29512` was 4,429,896). The backend probe and the
launch loop were scratch scripts outside the repository, with the same isolation
as `virtual.sh`: private Xvfb display, openbox, `dbus-run-session`, private
`XDG_RUNTIME_DIR`, `sni_watcher.py`. The loop ran a byte-identical copy of the
release binary (`cmp`).

Dependency graph (`cargo tree`): one `tray-icon` 0.25.1, used by both `tauri`
and `sotyware`, with features `ksni`, `libappindicator`, `muda-snapshot`,
`muda-gtk3`, `muda-serde`, `muda-common-controls-v6`, `serde`. New in
`Cargo.lock`: `ksni` 0.3.7, `arc-swap` 1.9.5, `crossbeam-queue` 0.3.14,
`pastey` 0.2.3, `task-local` 0.1.1. `zbus` 5.19.0 was already present. No new
duplicate crate versions. For `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin`, the resolved packages and features are identical to
before the change.

| Check | Result |
| --- | --- |
| Backend selected in source: `tray-icon` 0.25.1 `platform_impl/mod.rs` compiles `gtk/` only with `libappindicator` and `not(ksni)`, and `ksni/` whenever `ksni` is on | ✅ |
| Binary: `ldd`/`readelf` NEEDED list no appindicator library (neither did the libayatana build, which `dlopen`ed it) | ✅ |
| Binary strings: `libayatana-appindicator3.so.1`, `app_indicator_new` and `libappindicator-sys` absent (each present once in the libayatana build). `org.kde.StatusNotifierItem-`, `/StatusNotifierItem`, `/MenuBar` and `ksni-0.3.7` present | ✅ |
| Running process (`/proc/<pid>/maps`): no `libayatana-*`, `libdbusmenu-*` or `ido` library mapped (the libayatana build mapped 5). Extra thread `tray-icon-menu-watcher` | ✅ |
| Registration: name `org.kde.StatusNotifierItem-<pid>-1`, path `/StatusNotifierItem`, menu `/MenuBar`, `Id` `sotyware-tray`, `ItemIsMenu` false, one 32×32 `IconPixmap`, tooltip `sotyware` (the libayatana build used `/org/ayatana/NotificationItem/…` and an icon file) | ✅ |
| `npm run check` 0/0, `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | ✅ |
| `npm run test:headless` with `DISPLAY`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR` unset: 17 Rust + 8 frontend passed | ✅ |
| `npm run test:desktop:virtual`, 66-check driver, before adapting it: 63 passed, 3 failed. All 3 were scenario 10: the unwritable `$XDG_RUNTIME_DIR` no longer breaks the tray, so the instance ran with its tray and was killed after 30 s | ❌ expected, driver adapted in `12a6185` |
| `npm run test:desktop:virtual`, 68-check driver at `12a6185`: **68 passed, 0 failed, 0 skipped**, 3 consecutive runs | ✅ |
| 420 consecutive launches, each: wait ≤ 10 s for one registration, read the menu, tray "Quit", wait ≤ 10 s for exit. No retries | ✅ **420/420** registered (0.148 s min, 0.178 s median, 0.203 s p95, 0.293 s max), 420 menus with exactly Open Dashboard / Quit, 420 exits with 0 through tray Quit, 0 `CRITICAL` / `G_IS_DBUS_CONNECTION`, 0 hangs, 0 processes left |
| Left click (`Activate` over D-Bus) on a hidden dashboard shows the same window. Middle click (`SecondaryActivate`) does not | ✅ |
| Stub watcher killed and restarted: the item registers again after 0.10 s, and its menu still works | ✅ |
| Private bus without a watcher: exit 1 after 0.36 s, `cannot create system tray icon: … failed to register to the StatusNotifierWatcher: …ServiceUnknown…`, nothing left (behaviour at the time; since `8ca3bd7` dashboard-only mode) | ✅ |
| No session bus: exit 1, `cannot create system tray icon: … D-Bus connection error …`, nothing left (behaviour at the time; since `8ca3bd7` dashboard-only mode) | ✅ |
| After all runs: no Xvfb, openbox, stub watcher, portal, session `dbus-daemon` or sotyware process of these runs left (an unrelated Xvfb on `:25059`, started hours earlier, was still running) | ✅ |
| `npm run tauri build -- --bundles deb`: `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ⚠️ the first one is no longer needed ([limitations](limitations.md#linux-tray)) |

Not run: native Windows (the only Windows-relevant change is unused lockfile
entries), a real Linux desktop or tray host, Wayland, a system without
libayatana-appindicator installed. Zero failures in 420 launches is evidence,
not proof. The libayatana build failed 1 in 420 under the same loop.

### Runtime behaviour, virtual display, standard Ubuntu packages (WSL2, Xvfb + openbox, stub tray host)

Category B, run 2026-11-10 at `53d9743` on Ubuntu 24.04.1 LTS under WSL2 (kernel
6.18.54.1-microsoft-standard-WSL2, x86_64), Node 24.22.0, npm 11.19.1, Rust and
Cargo 1.99.1, tauri-cli 2.12.4. **All tools installed with `apt` as documented**
(`sudo apt install -y xvfb openbox python3-dbus python3-gi dbus-x11`): xvfb
2:21.1.12-1ubuntu1.9, openbox 3.6.1-12build5, x11-xkb-utils 7.7+8build2 (pulled
in by xvfb, provides `xkbcomp`), dbus-x11 1.14.10-4ubuntu4.1, python3-dbus
1.3.2-5build3, python3-gi 3.48.2-1, libayatana-appindicator3-1 0.5.93-1build3.
No extracted `.deb`s, no shim, no `PATH` changes beyond the WSL notes in
[setup.md](../development/setup.md#wsl2-notes). Simulated: the display (Xvfb),
the desktop (openbox), the tray host (`sni_watcher.py`) and the user (X11 client
messages, D-Bus calls). Real: the release binary
(`src-tauri/target/release/sotyware`, sha256 `10c5e588…29512`; a rebuild from
unchanged sources was byte-identical).

| Check | Result |
| --- | --- |
| Missing `Xvfb`/`openbox` (before installing): names them, prints the install command, exits 2, starts nothing | ✅ |
| Two consecutive `npm run test:desktop:virtual` runs at `53d9743`: **66 passed, 0 failed, 0 skipped** each, exit 0, about 7 s each | ✅ |
| 30 further consecutive runs at the same driver: 66/0/0 each | ✅ |
| Before the driver fix (`53d9743`), 65-check driver: first run 65/0/0, second run 64/1/0. In about 90 runs (the driver and instrumented scratch copies of it), 5 failed or crashed because of races in the driver and 2 because of the tray defect below | ❌ driver fixed |
| Private display `:99` (abstract socket only); the pre-existing WSLg display `:0` and an unrelated Xvfb on `:25059` were not used and still running afterwards | ✅ |
| After every run: no Xvfb, openbox, `dbus-daemon --session`, stub tray host, xdg portal or sotyware process left; no new `/tmp` entry or X lock file | ✅ |
| Interrupted mid-run (SIGINT to the process group: exit 1; SIGTERM to the script: exit 143): same, nothing left | ✅ |
| A fake "sotyware" window shown during a second launch fails "no other window appeared" (control) | ✅ |
| A stub tray host that records no registration gives a FAIL, the primary's output and exit 1, not a traceback (control) | ✅ |

**Failures found and fixed (test driver, `53d9743`):** the window-tree walk raced
openbox unmanaging the just-closed dashboard, so the primary's own window (same id
and pid) was reported as "new" or the "8 close" check missed it; the tray "Quit"
click raised D-Bus `NoReply` when sotyware exited before replying; and a missing
tray registration crashed the driver. None was a sotyware defect.

**Found, not fixed (tray library):** a launch occasionally registers no
tray item at all and prints
`GLib-GIO-CRITICAL **: g_dbus_proxy_new: assertion 'G_IS_DBUS_CONNECTION (connection)' failed`;
3 of about 420 primary launches. Root cause investigated below. The suite
reports it as a FAIL. Avoided since `12a6185`: sotyware no longer uses the library
([ksni run](#ksni-tray-backend-wsl2-xvfb--openbox-stub-tray-host)).

### Missing tray item investigation (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `c5540c9`, the machine and packages above (GLib
2.80.0-6ubuntu3.10), release binary sha256 `10c5e588…29512` (same as at
`53d9743`). The launch loop and the diagnostic shim were scratch tools outside
the repository. The loop used the same isolation as `virtual.sh`: private Xvfb
display, openbox, `dbus-run-session`, private `XDG_RUNTIME_DIR`,
`sni_watcher.py`. Each launch waited up to 10 s for one registration, then ended
with tray "Quit" (SIGTERM if not registered). No retries.

| Check | Result |
| --- | --- |
| 420 consecutive launches, unmodified binary | ❌ 1 without a tray item (launch 58: alive, watcher owned its name, `G_IS_DBUS_CONNECTION` in the output, nothing else), 419 registered in ≤ 714 ms with no critical, all exited 0 |
| 420 consecutive launches with a logging `LD_PRELOAD` shim, no delay | ✅ 0 failures. In every launch libayatana's own `g_bus_get` result arrived before the watcher's name-appeared callback, by 0.03 ms (minimum) to about 1.4 ms (median) |
| Shim delays only libayatana's `g_bus_get` result by 300 ms | ❌ 2 of 2 without a tray item, same critical. `g_dbus_proxy_new` called from `libayatana-appindicator3.so.1` with `connection=(nil)` (the forced-order proof) |
| After every launch, both loops: no sotyware process left; display, bus and runtime directory removed | ✅ |

Result: the defect is a race in libayatana-appindicator (cause, evidence and
fix status in [limitations.md](limitations.md#historical-libayatana-appindicator-registration-race)).
No sotyware change at the time; the limitation stayed open. Not run outside WSL2.
Superseded by the switch to the `ksni` backend (`12a6185`).

The headless suite, static checks and release build on the same machine and
commit: `npm run test:headless` 8 + 17 passed with `DISPLAY`, `WAYLAND_DISPLAY`,
`DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` unset, exit 1 with one injected
failing test; `npm run check` 0/0, `npm run build`, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings` and `npm run tauri build -- --no-bundle`
passed.

### Runtime behaviour, virtual display, extracted packages (WSL2, Xvfb + openbox, stub tray host)

Superseded by the standard-package run above; kept as the earlier record.
Category B. `npm run test:desktop:virtual` on the release binary (built
2026-11-10 from the code with the lifecycle decisions extracted):
**65 passed, 0 failed, 0 skipped**, twice in a row (2026-11-10). Xvfb
2:21.1.12-1ubuntu1.9 and openbox 3.6.1 came from the Ubuntu 24.04 `.deb`s
extracted into a user directory (no root on this machine), and Xvfb was started
through a user-namespace shim so it finds `xkbcomp`; the WSL `/tmp/.X11-unix` is
read-only, which the script's abstract-socket-only setting handles.

| Behaviour | Result |
| --- | --- |
| Everything in the simulated desktop table below that passed there | ✅ |
| Second launch focuses an already visible dashboard (`_NET_ACTIVE_WINDOW` observed) | ✅ |
| Second launch restores a minimized dashboard (no longer iconic) | ✅ |
| Tray failure (unwritable `$XDG_RUNTIME_DIR`): exit 1, error message, nothing left | ✅ |
| Timeout path: exit 124, logs printed, no leftover sotyware, openbox or Xvfb | ✅ |

openbox on Xvfb is **not** GNOME, KDE, XFCE or Wayland, and the tray host is a
stub. Focus behaviour on real Linux window managers remains unvalidated.

### Runtime behaviour, simulated Linux desktop (WSL2/WSLg, X11, stub tray host)

`tests/desktop/run.sh` on the release binary. Re-run 2026-11-10 after the
lifecycle decisions were extracted: **57 passed, 0 failed, 2 skipped** (54/0/2
before the tray-failure scenario was added; same 2 skips). A build with the tray
failure made non-fatal fails the new exit-code check. Not re-run with the
66-check driver from `53d9743`.

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
| "Hide dashboard" button hides the window; process stays | ✅ | Earlier run (no automated test on Linux) |
| Repeated "Open Dashboard" creates no second window | ✅ | Earlier run (also implied by desktop test window counts) |
| Native Wayland backend: runs, tray registers, Quit exits 0 (window state not inspected) | ✅ | Earlier run |
| `XDG_RUNTIME_DIR` that cannot be created (read-only parent): tray error, exit 1, nothing left | ✅ | desktop test (scenario 10, libayatana backend, before `12a6185`; with `ksni` sotyware starts normally, scenario 11) |

This environment is **not a real desktop**: the tray host is a test stub, and no
tray icon was ever seen or clicked by a person.

### Not yet validated

| Environment | Status |
| --- | --- |
| Windows 10 | — never built or run |
| Windows 11: a person's visual check of tray icon and menu (manual checklist) | — |
| Windows 11: MSI install/run/uninstall; NSIS interactive install | — |
| Windows 11: focus on second launch while another app is actively in use | — |
| Real GNOME (with and without the AppIndicator extension; without it, dashboard-only mode) | — no real Linux desktop available on 2026-11-10 ([survey](#real-linux-desktop-availability-2026-11-10)) |
| Dashboard-only mode on any real desktop, an XEmbed-only panel, or Wayland | — (virtual display only) |
| Real KDE Plasma | — |
| XFCE | — |
| Real Wayland session with a real tray host | — |
| Focus and un-minimize on second launch, on any real Linux WM | — (skipped under WSLg; passes only under openbox on Xvfb) |
| Headless suite and virtual-display test on GitHub Actions | — workflows exist (`5fe62b9`), not run on GitHub yet ([CI](#ci-github-actions-2026-11-10)) |
| Windows private-desktop test in Session 0 or on a GitHub-hosted runner | — (runs only in a logged-in session so far; the manual workflow is untried) |
| Windows tray, real clicks and Quit without an interactive, unlocked desktop | — not possible: `windows.ps1` needs the input desktop with Explorer |
| Headless suite on Windows 10 | — |
| Installing and running `.deb` / `.rpm` / AppImage | — |

To validate an environment, follow the manual checklist in
[testing.md](../development/testing.md#manual-check-real-desktop-linux-or-windows)
and update this table.

## See also

- [limitations.md](limitations.md): known limitations.
- [roadmap.md](roadmap.md): next steps.

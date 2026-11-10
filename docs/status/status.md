# Implementation status

Last updated: 2026-11-10. Update this file whenever
implementation or validation changes. Never mark something validated that was
not run on that environment.

## Current milestone

**Milestone 0: prove the desktop runtime.** Show that the stack (Tauri 2 +
Svelte 5) and the background lifecycle (tray, hide-on-close, explicit Quit,
single instance) work on Windows 10/11 and Linux desktops, before any product
features are built.

State: implemented. Validated natively on **Windows 11** (25H2, build 26200.9711)
by an automated test on the real desktop, and in a simulated Linux environment
(WSL2/WSLg). Open: Windows 10, a person's visual check on Windows, the MSI
installer, and real Linux desktops.

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

Automated tests per platform: the "Hide dashboard" button (real click) and
left-click-to-open are covered only by the Windows desktop test
(`tests/desktop/windows.ps1`). On Linux the button is covered only by the headless
frontend test with mocked IPC. Restoring a minimized dashboard and focus on second
launch run on Linux only under the virtual display (Xvfb + openbox); WSLg skips
them. Test categories: [testing.md](../development/testing.md).

Nothing else exists: no product features, no settings, no persistence, no
custom Tauri commands or events.

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
| `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ✅ | Re-run 2026-11-10 |

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
| Headless suite on Windows | — not run | |

### Native Windows 11: environment

Windows 11 Home, version 25H2, OS build 26200.9711 (the registry `ProductName`
still says "Windows 10", a known Windows quirk; build 26200 is Windows 11). x64.
Checkout `C:\dev\sotyware`, a fresh `git clone` of `origin` at `f8174da` on the
NTFS drive (not a WSL path), `core.autocrlf=true`. Commands were started from WSL
through Windows interop, but compiled and ran as native Windows processes in the
interactive desktop session.

| Tool | Version |
| --- | --- |
| Rust / Cargo | 1.99.1 / 1.99.1, `stable-x86_64-pc-windows-msvc` (rustup) |
| MSVC | Visual Studio Community 2022, C++ build tools |
| Node.js / npm | 24.14.1 / 11.12.1 |
| Tauri CLI | 2.12.4 (from `npm ci`) |
| WebView2 Runtime | 157.0.4341.42 |
| Windows PowerShell (test) | 5.1 |

### Static checks and builds, native Windows 11

| Check | Result | When |
| --- | --- | --- |
| `npm ci` | ✅ | Native Windows 2026-11-10 |
| `npm run check` (0 errors, 0 warnings) | ✅ | Native Windows 2026-11-10 |
| `npm run build` | ✅ | Native Windows 2026-11-10 |
| `cargo fmt --check` | ✅ | Native Windows 2026-11-10 |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Native Windows 2026-11-10 |
| `cargo test` (1 test) | ✅ | Native Windows 2026-11-10 |
| `npm run tauri build`: `sotyware.exe` (3.2 MB), `sotyware_0.1.0_x64-setup.exe` (NSIS, 1.13 MiB), `sotyware_0.1.0_x64_en-US.msi` (1.64 MiB) | ✅ built | Native Windows 2026-11-10 |

### Runtime behaviour, native Windows 11 (real desktop)

`tests/desktop/windows.ps1` on the release binary: **80 passed, 0 failed**, run
three times in a row with the same result (2026-11-10). Earlier revisions of the
same script (78 checks) also passed on four further consecutive runs. No sotyware
defect was found.

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
| NSIS uninstaller `/S` after the app had quit: exit 0; files, uninstall entry and shortcuts removed | ✅ | Native Windows 2026-11-10 |
| Start menu shortcut launches the installed exe; a second shortcut launch keeps one process | ✅ | Native Windows 2026-11-10 |
| NSIS uninstaller `/S` while sotyware was running: exit 0; process ended; install dir, uninstall entry and both shortcuts removed | ✅ | Native Windows 2026-11-10 |
| WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` after uninstall | ⚠️ still present; it was also created by earlier runs of the build output, so this does not show whether the uninstaller would remove it | Native Windows 2026-11-10 |
| NSIS installer with its interactive UI | — not run (silent mode only) | |
| MSI `msiexec /i … /qn` | ❌ not installed: exit 1603, "Installation UI level is silent, no credential elevation is possible". The MSI installs per machine and needs administrator rights (UAC). Not a sotyware defect; MSI install, run and uninstall remain unvalidated | Native Windows 2026-11-10 |

### Runtime behaviour, virtual display (WSL2, Xvfb + openbox, stub tray host)

Category B. `npm run test:desktop:virtual` on the release binary (built
2026-11-10 from the code with the lifecycle decisions extracted):
**65 passed, 0 failed, 0 skipped**, twice in a row (2026-11-10). Xvfb
2:21.1.12-1ubuntu1.9 and openbox 3.6.1 came from the Ubuntu 24.04 `.deb`s
extracted into a user directory (no root on this machine), and Xvfb was started
through a user-namespace shim so it finds `xkbcomp`; the WSL `/tmp/.X11-unix` is
read-only, which the script's abstract-socket-only setting handles. Not yet run
with normally installed packages or in CI.

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
failure made non-fatal fails the new exit-code check.

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
| `XDG_RUNTIME_DIR` that cannot be created (read-only parent): tray error, exit 1, nothing left | ✅ | desktop test (scenario 10) |

This environment is **not a real desktop**: the tray host is a test stub, and no
tray icon was ever seen or clicked by a person.

### Not yet validated

| Environment | Status |
| --- | --- |
| Windows 10 | — never built or run |
| Windows 11: a person's visual check of tray icon and menu (manual checklist) | — |
| Windows 11: MSI install/run/uninstall; NSIS interactive install | — |
| Windows 11: focus on second launch while another app is actively in use | — |
| Real GNOME (with and without the AppIndicator extension) | — |
| Real KDE Plasma | — |
| XFCE | — |
| Real Wayland session with a real tray host | — |
| Focus and un-minimize on second launch, on any real Linux WM | — (skipped under WSLg; passes only under openbox on Xvfb) |
| Headless suite and virtual-display test in a CI service | — no CI exists |
| Installing and running `.deb` / `.rpm` / AppImage | — |

To validate an environment, follow the manual checklist in
[testing.md](../development/testing.md#manual-check-real-desktop-linux-or-windows)
and update this table.

## See also

- [limitations.md](limitations.md): known limitations.
- [roadmap.md](roadmap.md): next steps.

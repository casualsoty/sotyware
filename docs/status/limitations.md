# Known limitations

Known gaps in the current implementation. These are **not** solved. When one is
fixed, remove it here in the same change and update
[status.md](status.md).

## Linux tray

- **Desktop support is unverified.** The tray uses StatusNotifierItem via
  libayatana-appindicator. KDE Plasma, Cinnamon, XFCE (status notifier plugin) and
  Budgie are expected to show it. Vanilla **GNOME hides tray icons** unless the
  *AppIndicator and KStatusNotifierItem Support* extension is installed (Ubuntu
  enables it by default). None of this has been verified.
- **Missing tray host is not detected.** With no StatusNotifierItem host,
  libayatana falls back to a legacy XEmbed icon. On desktops without an XEmbed
  tray, the icon is invisible. A hidden dashboard then cannot be restored, and the
  process keeps running until stopped manually. Tray *creation* succeeded, so the
  fatal-exit rule does not trigger. (A second launch still shows the dashboard,
  through the single-instance callback.)
- **No click-to-open on Linux.** AppIndicator emits no icon click events; any click
  opens the menu.
- **Tray icon image needs a usable `$XDG_RUNTIME_DIR`.** If it points to a missing
  directory, tray creation fails and sotyware exits with code 1 (seen under WSL).

## Single instance

- **D-Bus fallback.** Linux needs a D-Bus session bus. Without one (bare SSH, cron),
  the plugin starts normally, so the guard is off and a second process can start.
  Its warning goes through `tracing`, and sotyware installs no subscriber, so
  **nothing is printed**.
- **Startup race.** If a second launch arrives after the first instance took the
  lock but before its window exists, the callback logs
  `sotyware: dashboard window 'main' not found` and does nothing. The first
  instance shows its dashboard anyway because it starts visible; no duplicate is
  created. Derived from the code order; not reproduced by a test.
- **Snap and Flatpak.** Their sandboxes block the D-Bus name by default. sotyware
  ships neither; packaging for them would need a manifest that allows
  `com.sotyware.desktop.SingleInstance`.
- **Focus is up to the platform.** The app calls `set_focus()`, but Wayland
  compositors and Windows' foreground-lock rules may refuse and flash the taskbar
  entry or notify instead. The dashboard is still shown and un-minimized. On
  Windows 11 the dashboard did become the foreground window in every tested case
  (second launch while hidden, visible or minimized; tray left click), but the
  launches came from a script, not while a person was typing in another app. Not
  verified on Windows 10 or any real Linux window manager.
- **Per user session.** Two users logged in at once each get their own instance.
- **Arguments ignored.** The second launch's arguments and working directory reach
  the running instance but are not used.

## Windows tray

- **The icon starts in the hidden-icons flyout.** Windows 11 puts a new
  notification-area icon behind the "Show Hidden Icons" (^) button, so on first
  run users do not see sotyware in the taskbar until they open the flyout or pin
  the icon. This is the OS default for every app; sotyware cannot change it.
  Observed on Windows 11 25H2.
- **Tray menu items are invisible to UI Automation.** The menu opens as a Win32
  popup menu (class `#32768`) whose items carry their labels (readable through the
  Win32 menu handle, which the test uses), but UI Automation exposes no menu item
  elements, only an empty pane. The cause is not confirmed (likely custom drawing
  in the menu library, `muda`). Screen readers may therefore not announce "Open
  Dashboard" and "Quit". Not checked with an actual screen reader.

## Windows installers

- **The MSI needs administrator rights.** It installs per machine, so a silent
  (`/qn`) install from a non-elevated shell fails with exit 1603. The NSIS
  installer installs per user and needs no elevation.

## General

- **Placeholder icons.** `src-tauri/icons/` contains Tauri's default icons.
- **No autostart** on login.
- **Logging is `eprintln!` only.** No log files, levels or subscriber; output is
  lost when started from a desktop launcher.
- **`npm audit`** reports a low-severity advisory in `deepmerge`
  (GHSA-gcqx-w48m-w2h4), a build-time dependency of
  `@sveltejs/vite-plugin-svelte`, with no fix available (re-checked 2026-11-10).
  It is not shipped to users.

## Tests

- **Headless tests check decisions, not wiring.** `cargo test` proves what the
  lifecycle functions decide, not that Tauri delivers the events to them or that
  the OS obeys. Only the desktop tests run the real binary.
- **Headless still needs the build libraries.** `cargo test` compiles the Tauri
  crate, so on Linux the WebKitGTK and AppIndicator development packages must be
  installed (no display server, though).
- **The implicit-exit guard (`code: None` blocked) is unit-tested only.** No
  desktop test triggers it, because the dashboard is never destroyed.
- **On Linux the "Hide dashboard" button is tested only with mocked IPC.** No
  Linux test clicks it in the real webview.
- **The virtual-display test was validated only on WSL2**, with Xvfb and openbox
  extracted from `.deb`s and Xvfb started through a user-namespace shim, not with
  normally installed packages and not in CI.
- **No test covers screen-reader access** on either platform.

## Development environment (WSL)

See [setup.md](../development/setup.md#wsl2-notes): Windows `npm` shadowing the
Linux toolchain, AppImage bundling needing a `PATH` filter, WSLg ignoring focus and
minimize requests.

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
- **A launch occasionally registers no tray item (race in libayatana-appindicator).**
  Seen 3 times in about 420 primary launches in the virtual-display test, then
  1 in 420 in a launch loop (Ubuntu 24.04.1 on WSL2,
  libayatana-appindicator3-1 0.5.93-1build3, stub tray host), 2026-11-10. The
  dashboard opens and the process keeps running, but no StatusNotifierItem is
  registered. The only output is
  `GLib-GIO-CRITICAL **: g_dbus_proxy_new: assertion 'G_IS_DBUS_CONNECTION (connection)' failed`,
  which no successful launch prints. Tray creation counts as a success, so the
  fatal-exit rule does not trigger. After a close, the dashboard can be reached
  only by launching sotyware again. The desktop test reports it as a FAIL.

  *Cause (established, not fixed):* the defect is in libayatana-appindicator
  itself, not in sotyware, Tauri or `tray-icon`. `app_indicator_init()` starts two
  asynchronous operations whose completion order is not defined:
  `g_bus_get()` for its own session-bus connection (stored by `bus_creation()` in
  `priv->connection`) and `g_bus_watch_name()` for
  `org.kde.StatusNotifierWatcher`. When the watcher already owns its name,
  `name_appeared_handler()` calls `g_dbus_proxy_new(priv->connection, …)`, using
  the private field instead of the connection argument it was given. If that
  callback runs before `bus_creation()`, the field is still NULL. GLib then prints
  the assertion and returns without calling back. `watcher_proxy` is never set,
  so `RegisterStatusNotifierItem` is never sent, and no fallback timer starts.
  The name-appeared callback does not run again unless the watcher restarts, so
  the item stays unregistered for the life of the process. Both completions go
  through GLib's worker-thread pool (GDBusConnection uses the default
  `GAsyncInitable` implementation), and in successful launches the library's own
  connection won by only 0.03 ms to about 2 ms.

  *Evidence (2026-11-10, release binary built at `c5540c9`, scratch diagnostics
  not in the repository):* the libayatana-appindicator 0.5.93 and GLib 2.80.0
  sources. A diagnostic `LD_PRELOAD` shim logged the caller and arguments of
  `g_bus_get`, `g_bus_watch_name` and `g_dbus_proxy_new`. It showed that
  `libayatana-appindicator3.so.1` makes the failing `g_dbus_proxy_new` call with
  `connection=(nil)`, while the handler's own connection argument was valid.
  Delaying only the library's `g_bus_get` result by 300 ms reproduced the defect
  in 2 of 2 launches, with the same message and no registration. Unmodified
  launches: 1 failure in 420 (same message). Shim-logged launches: 0 failures in
  420, all with the library's connection arriving first.

  *Fix status:* upstream master (0.6.1) still has the same
  `name_appeared_handler()`. The likely upstream fix is a one-line change: pass
  the handler's `connection` argument. No released version and no Tauri
  configuration avoids the race: Tauri 2.12.5 pins `tray-icon` 0.25 with the
  libappindicator backend, and `tray-icon` 0.26's pure-Rust `ksni` backend is not
  exposed by Tauri. Opening the session bus before creating the tray would only
  narrow the window, because both results still go through the thread pool.
  sotyware has no local fix. Reproduced only under WSL2 with the stub tray host;
  not checked on native Linux or with a real tray host, where the same code runs.
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
- **The virtual-display test was validated only on WSL2** (Ubuntu 24.04.1, with
  the apt-installed packages from [setup.md](../development/setup.md#linux-system-packages)),
  not on a native Ubuntu install and not in CI. It fails, correctly, on the
  intermittent [missing tray item](#linux-tray), so a CI job would be flaky at
  roughly 1 run in 60 while the library race is unfixed.
- **No test covers screen-reader access** on either platform.

## Development environment (WSL)

See [setup.md](../development/setup.md#wsl2-notes): Windows `npm` shadowing the
Linux toolchain, AppImage bundling needing a `PATH` filter, WSLg ignoring focus and
minimize requests.

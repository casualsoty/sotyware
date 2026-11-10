# Known limitations

Known gaps in the current implementation. These are **not** solved. When one is
fixed, remove it here in the same change and update
[status.md](status.md).

## Linux tray

The tray is a StatusNotifierItem served by `tray-icon`'s pure-Rust `ksni`
backend (`ksni` 0.3.7 over `zbus`), not libayatana-appindicator
([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)).

- **Desktop support is unverified.** KDE Plasma, Cinnamon, XFCE (status notifier
  plugin) and Budgie are expected to show the item. Vanilla **GNOME has no
  StatusNotifierWatcher** unless the *AppIndicator and KStatusNotifierItem
  Support* extension is installed (Ubuntu enables it by default). None of this
  has been verified.
- **No tray host means sotyware does not start.** `ksni` registers with
  `org.kde.StatusNotifierWatcher` before tray creation returns. Without a
  watcher, or without a session bus, tray creation fails and sotyware exits
  with code 1 ([ADR-004](../architecture/decisions.md#adr-004-failure-to-create-the-tray-is-fatal)).
  The error goes to stderr only, so someone starting sotyware from a launcher
  sees nothing happen. There is **no XEmbed fallback**: desktops with only a
  legacy XEmbed tray (for example some window-manager panels) cannot run
  sotyware. A watcher that starts *after* sotyware (for example a login race,
  if autostart is ever added) is not waited for. Checked under the virtual
  display only (2026-11-10).
- **A watcher without a visible host is not detected.** If a watcher accepts
  the registration but no panel displays it, sotyware runs with an invisible
  icon. A hidden dashboard can then be reached only by launching sotyware again.
  Real watchers (KDE, the GNOME extension) always report a host as registered,
  according to `ksni`'s source comments; this has not been checked.
- **Left click depends on the tray host.** The item is not marked menu-only, so
  hosts are expected to send a left click as `Activate`, which opens the
  dashboard. The menu is on right click. Seen only with D-Bus calls against the
  stub tray host, not with a real panel.
- **The `.deb` still declares `libayatana-appindicator3-1`.** Tauri's bundler
  adds it whenever the tray feature is on, although the binary never loads the
  library. It costs an unnecessary package, nothing more. Running without the
  library installed has not been tried.

### Historical: libayatana-appindicator registration race

Avoided since `12a6185`, **not fixed upstream**. Until then, a launch
occasionally registered no tray item: 3 times in about 420 primary launches in
the virtual-display test, then 1 in 420 in a launch loop (Ubuntu 24.04.1 on
WSL2, libayatana-appindicator3-1 0.5.93-1build3, stub tray host), 2026-11-10.
The only output was
`GLib-GIO-CRITICAL **: g_dbus_proxy_new: assertion 'G_IS_DBUS_CONNECTION (connection)' failed`.
sotyware no longer uses this library; the same launch loop with the `ksni`
backend registered all 420 launches
([status](status.md#ksni-tray-backend-wsl2-xvfb--openbox-stub-tray-host)). Any
other program on the same system that uses libayatana-appindicator is still
affected.

  *Cause (established, not fixed upstream):* the defect is in libayatana-appindicator
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
  the handler's `connection` argument. Opening the session bus before creating
  the tray would only narrow the window, because both results still go through
  the thread pool. Reproduced only under WSL2 with the stub tray host; not
  checked on native Linux or with a real tray host, where the same code runs.
  (An earlier version of this entry said that `ksni` first appears in
  `tray-icon` 0.26 and that Tauri does not expose it. That was wrong: `tray-icon`
  0.25.1 has the feature, and Cargo feature unification enables it, see
  [ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator).)

## Single instance

- **D-Bus fallback.** Linux needs a D-Bus session bus. Without one (bare SSH, cron),
  the plugin starts normally, so the guard is off. Its warning goes through
  `tracing`, and sotyware installs no subscriber, so **nothing is printed**.
  Since the `ksni` tray backend, the tray also needs the session bus, so such a
  process exits 1 right after (`cannot create system tray icon: … D-Bus
  connection error`). A second process therefore cannot keep running either.
  The guard itself is still off.
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
  not on a native Ubuntu install and not in CI. With libayatana-appindicator it
  failed, correctly, about 1 run in 60 on the
  [registration race](#historical-libayatana-appindicator-registration-race).
  With the `ksni` backend no run has failed so far (3 runs and a 420-launch
  loop, 2026-11-10). That is evidence, not proof.
- **No test covers screen-reader access** on either platform.

## Development environment (WSL)

See [setup.md](../development/setup.md#wsl2-notes): Windows `npm` shadowing the
Linux toolchain, AppImage bundling needing a `PATH` filter, WSLg ignoring focus and
minimize requests.

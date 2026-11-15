# Application lifecycle and system tray

How the dashboard window, the tray and the process interact. All of it is
implemented in `src-tauri/src/lib.rs` and `src-tauri/src/tray.rs`. Second-launch
handling is in [single-instance.md](single-instance.md). Why there are two modes:
[ADR-012](decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode).

## Operating modes

`setup` tries to create the tray once and stores an `OperatingMode` as Tauri
state. It never changes while the process runs:

- **Tray mode**: the tray icon exists. sotyware runs in the background; the
  dashboard is hidden and shown, and only tray "Quit" ends the process. This is
  the only mode on Windows.
- **Dashboard-only mode** (Linux only): no tray host is reachable (no session bus,
  no StatusNotifierWatcher, or a watcher without a StatusNotifierHost). The
  dashboard is the only way to reach sotyware, so it is never hidden, and
  closing it ends the process.
- Any other tray error is fatal: sotyware exits 1 (see
  [mode selection](#mode-selection)).

## User actions

| User action | Tray mode | Dashboard-only mode |
| --- | --- | --- |
| **Hide** (dashboard button) | Window hidden; process and tray keep running | No button. A note says that no compatible system tray is available, the dashboard cannot be hidden, and closing it quits. A hide call is refused by Tauri (permission not granted) |
| **Close** (title-bar ✕, Alt+F4, WM close) | Intercepted and turned into hide; process keeps running | Process exits cleanly with code 0 |
| **Quit** (tray menu) | Process exits cleanly with code 0 | No tray. Close quits |
| **Open Dashboard** (tray menu, or left click on the icon; on Linux if the tray host sends it) | Existing window un-minimized, shown, focused | No tray |
| **Launch sotyware again** while it runs | No new process, tray icon or window; the existing dashboard is shown, un-minimized, focused | The same, as long as a session bus exists (without one the single-instance guard is off) |
| **Restart now** (after an update installed in place, macOS and Linux; ADR-036) | `request_restart`: the normal exit (`RunEvent::Exit`, `release_resources`, single-instance lock released), then the new version starts | The same |

**macOS** (ADR-037) is always tray mode. In addition: **Quit sotyware** in
the menu bar (⌘Q) is the tray's Quit (`AppControl::quit`, exit 0); **Close
Window** (⌘W) is a close, so it hides; a **click on the Dock icon**
(`RunEvent::Reopen`) shows the dashboard like Open Dashboard; the Dock's
Quit and a logout terminate the app from outside, which arrives as
`RunEvent::Exit` without an exit request, so cleanup runs and nothing
delays the logout.

## Startup sequence

1. `main()` calls `sotyware_lib::run()`.
2. `run()` registers the single-instance plugin first (desktop only). Its setup
   runs inside `Builder::build()`. A second instance exits here.
3. `Builder::build()` returns the app; no window exists yet.
4. Still before the event loop, `run()` opens the feature preferences database
   (`preferences.sqlite3` in the app's local data folder, created if needed),
   checks and migrates it, loads the stored choices and manages
   `FeaturePreferences`. If the database cannot be used, the reason is logged
   and kept for the dashboard, and the features use their defaults
   ([features.md](features.md#persistence)). A second instance never gets here.
5. `app.run(...)` starts the event loop. On its first event (`Ready`), Tauri
   creates window `main` from `tauri.conf.json` (`visible: true`), with the
   webview's data in the same local data folder (if Tauri cannot create that
   folder, it panics with `Failed to setup app` and sotyware does not start),
   and then runs `setup` once (Tauri 2.12.5, `app.rs`, `fn setup`).
   `setup` calls `tray::create()` once and passes the result to `select_mode()`:
   - success: tray mode. `setup` adds the runtime capability `tray-mode`
     (`core:window:allow-hide` for window `main`) and manages the mode;
   - no tray host: dashboard-only mode. Two lines on stderr give the reason
     and the mode. The mode is managed, no capability is added;
   - any other error: prints `sotyware: cannot create system tray icon: …` and
     calls `exit(1)`.
   In tray mode and dashboard-only mode alike, `setup` then starts the built-in
   features (`FeatureRegistry::start_all`), each according to its stored choice
   or, without one, its default. A feature that fails to start is
   recorded as `failed` and does not affect startup. With the fatal tray error,
   no feature is started ([features.md](features.md#lifecycle)).
6. The page asks for the mode with the `operating_mode` command and renders
   accordingly, and asks for the feature list with `list_features`.

## Mode selection

`tray::unavailable_reason()` maps the typed error to a reason, or to `None`
(fatal). On Linux, `tray::create()`'s error is
`tauri::Error::Tray(tray_icon::Error::Ksni(…))` when the `ksni` backend fails:

| `ksni` error | Reason printed | Result |
| --- | --- | --- |
| `Dbus(zbus::Error::Connection(io, _))`, `io` is `NotFound` or `ConnectionRefused` | `no D-Bus session bus` | dashboard-only |
| `Watcher(ServiceUnknown)` or `Watcher(NameHasNoOwner)` | `no StatusNotifierWatcher` | dashboard-only |
| `WontShow` | `no StatusNotifierHost` | dashboard-only |
| anything else | — | exit 1 |

zbus returns `Connection` only when connecting to the bus address fails, so a
failure on an established connection is never taken for a missing bus. The
full list of what stays fatal, and why the classification is safe, is in
[ADR-012](decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode).
On Windows every tray error is fatal. The tray is not retried and nothing waits
for a tray host: one that appears later is used on the next start.

## Rules and how they are implemented

- **One window, never destroyed.** `on_window_event` catches
  `WindowEvent::CloseRequested` and asks `close_action(label, mode)`. For
  `DASHBOARD_LABEL` in tray mode it calls `api.prevent_close()` and hides the
  window. In dashboard-only mode it calls `api.prevent_close()` and `exit(0)`.
  If the mode is somehow not managed yet, the handler treats it as
  dashboard-only, so it never hides a window that no tray could bring back.
  Because the window exists until the process ends, `show_dashboard()` can
  look it up with `get_webview_window("main")` and never needs to create one.
- **`show_dashboard()`** calls `unminimize()`, `show()` and `set_focus()` in that
  order. Each failure is logged and the next step still runs. If the window is not
  found it logs and returns.
- **Only an explicit exit ends the process.** The tray "Quit" item, and closing
  the dashboard in dashboard-only mode, call `AppHandle::exit(0)`. In
  `RunEvent::ExitRequested`, a request with `code: None` (the runtime stopping
  because no windows remain) is cancelled with `prevent_exit()`. Requests with an
  explicit code, which only come from `exit()`, pass.
- **Hide uses the native window API, in tray mode only.** The frontend calls
  `getCurrentWindow().hide()`, allowed by `core:window:allow-hide` from the
  runtime capability `tray-mode`, which `setup` adds only in tray mode
  (`frontend_may_hide`). An error is shown in the page.
- **The frontend learns the mode from Rust.** The `operating_mode` command
  (permission `allow-operating-mode`) returns `"tray"` or `"dashboard-only"`.
  The page shows the Hide button only for `"tray"`. If the command fails, it
  shows the error and no Hide button
  ([ADR-013](decisions.md#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode)).
- **Features stop when the process ends.** On `RunEvent::Exit`, which follows
  every allowed exit, `FeatureRegistry::stop_all()` stops the running features
  in reverse registration order. Errors are logged. If the OS kills the
  process, or the session ends without Tauri's exit path, no feature is stopped
  ([limitations](../status/limitations.md#built-in-features)). Stopping at exit
  changes no stored choice: a feature running at Quit starts again next time.
  Then `FeaturePreferences::close()` waits for a save in progress and refuses
  later ones; every completed save is already on disk.
  Then `CloudDownloads::shutdown()` refuses new downloads, cancels a running
  cloud download and waits up to 5 s for it to remove its temporary file
  (removing it itself if the wait runs out); a download already moving its
  verified file into place is waited for. A process killed by the OS leaves
  the temporary file ([cloud-storage.md](cloud-storage.md#downloading-a-file)).
  Last, `CloudUploads::shutdown()` refuses new upload batches (also one
  whose file dialog is still open), cancels a running batch and aborts every
  multipart upload whose earlier abort failed, waiting at most 35 s for all
  of it; what is not done by then is logged and may leave parts at the
  provider ([cloud-storage.md](cloud-storage.md#uploading-files)).
- **An unexpected tray failure is fatal.** `setup` exits with code 1 when
  `select_mode()` does not recognise the error as a missing tray host. (A
  watcher that accepts the item while no panel displays it is not detected; see
  [limitations](../status/limitations.md#linux-tray).)
- **Handlers are registered once.** On the builder and in `setup`, which runs once.
  No polling, timers, retries or background tasks.

## System tray

`tray::create()` builds a tray icon with id `sotyware-tray`, tooltip `sotyware`,
the app's default window icon, and a menu:

| Menu id | Label | `TrayAction` | Effect |
| --- | --- | --- | --- |
| `open-dashboard` | Open Dashboard | `OpenDashboard` | `AppControl::show_dashboard` → `show_dashboard(app)` |
| `quit` | Quit | `Quit` | `AppControl::quit` → `app.exit(0)` |

Unknown ids are ignored. `show_menu_on_left_click(false)` makes a left click on the
icon open the dashboard and a right click open the menu on Windows. On Linux the
tray host decides. The item is not marked menu-only, so hosts send a left click as
`Activate`, which arrives as a left-click event and opens the dashboard. A middle
click (`SecondaryActivate`) does nothing, and the host shows the menu on right
click. If there is no default window icon, a warning is logged and the tray may be
invisible.

On Linux the tray is a StatusNotifierItem served by `tray-icon`'s pure-Rust
`ksni` backend over `zbus`, not by libayatana-appindicator
([ADR-011](decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)).
`tray::create()` returns only after `ksni` has opened its session-bus
connection, served the item (`/StatusNotifierItem`) and its menu (`/MenuBar`),
taken the name `org.kde.StatusNotifierItem-<pid>-1` and called
`RegisterStatusNotifierItem` on the watcher. Each step uses that one connection,
so registration cannot be lost to an ordering race. If any step fails, tray
creation fails: no session bus or no `org.kde.StatusNotifierWatcher` select
dashboard-only mode, other errors exit 1 ([mode selection](#mode-selection)).
There is no XEmbed fallback. If the watcher restarts, `ksni` registers again. The
icon is sent over D-Bus as a pixmap, so nothing is written to `$XDG_RUNTIME_DIR`.
See [cross-platform.md](../development/cross-platform.md) and
[limitations.md](../status/limitations.md#linux-tray).

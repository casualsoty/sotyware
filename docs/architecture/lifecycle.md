# Application lifecycle and system tray

How the dashboard window, the tray and the process interact. All of it is
implemented in `src-tauri/src/lib.rs` and `src-tauri/src/tray.rs`. Second-launch
handling is in [single-instance.md](single-instance.md).

## User actions

| User action | Result |
| --- | --- |
| **Hide** (dashboard button) | Window hidden; process and tray keep running |
| **Close** (title-bar ✕, Alt+F4, WM close) | Intercepted and turned into hide; process keeps running |
| **Quit** (tray menu) | Process exits cleanly with code 0 |
| **Open Dashboard** (tray menu, or left click on the icon; on Linux if the tray host sends it) | Existing window un-minimized, shown, focused |
| **Launch sotyware again** while it runs | No new process, tray icon or window; the existing dashboard is shown, un-minimized, focused |

## Startup sequence

1. `main()` calls `sotyware_lib::run()`.
2. `run()` registers the single-instance plugin first (desktop only). Its setup
   runs inside `Builder::build()`. A second instance exits here.
3. `Builder::build()` creates window `main` from `tauri.conf.json`
   (`visible: true`), then runs `setup`.
4. `setup` calls `tray::create()`. On failure it prints the error and calls
   `exit(1)`.
5. `app.run(...)` enters the event loop.

## Rules and how they are implemented

- **One window, never destroyed.** `on_window_event` catches
  `WindowEvent::CloseRequested` for `DASHBOARD_LABEL`, calls `api.prevent_close()`
  and hides the window. Because the window always exists, `show_dashboard()` can
  look it up with `get_webview_window("main")` and never needs to create one.
- **`show_dashboard()`** calls `unminimize()`, `show()` and `set_focus()` in that
  order. Each failure is logged and the next step still runs. If the window is not
  found it logs and returns.
- **Quit is the only way out.** The tray "Quit" item calls `AppHandle::exit(0)`.
  In `RunEvent::ExitRequested`, a request with `code: None` (the runtime stopping
  because no windows remain) is cancelled with `prevent_exit()`. Requests with an
  explicit code, which only come from `exit()`, pass.
- **Hide uses the native window API.** The frontend calls
  `getCurrentWindow().hide()`, allowed by the `core:window:allow-hide` permission.
  An error is shown in the page.
- **A missing tray is fatal.** Without a tray a hidden dashboard could never be
  restored, so `setup` exits with code 1 if `tray::create()` fails. (This cannot
  detect a watcher that accepts the item while no panel displays it; see
  [limitations](../status/limitations.md#linux-tray).)
- **Handlers are registered once.** On the builder and in `setup`, which runs once.
  No polling, timers or background tasks.

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
so registration cannot be lost to an ordering race. If any step fails (no session
bus, no `org.kde.StatusNotifierWatcher`), tray creation fails and sotyware exits 1.
There is no XEmbed fallback. If the watcher restarts, `ksni` registers again. The
icon is sent over D-Bus as a pixmap, so nothing is written to `$XDG_RUNTIME_DIR`.
See [cross-platform.md](../development/cross-platform.md) and
[limitations.md](../status/limitations.md#linux-tray).

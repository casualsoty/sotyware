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
| **Open Dashboard** (tray menu, or left click on the icon on Windows) | Existing window un-minimized, shown, focused |
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
  detect a tray that was created but has no host to display it; see
  [limitations](../status/limitations.md).)
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
icon open the dashboard and a right click open the menu, **on Windows**. Linux
(AppIndicator) emits no icon click events, so any click opens the menu. If there is
no default window icon, a warning is logged and the tray may be invisible.

On Linux the tray is a StatusNotifierItem via libayatana-appindicator, which falls
back to a legacy XEmbed icon when no StatusNotifierItem host exists. See
[cross-platform.md](../development/cross-platform.md) and
[limitations.md](../status/limitations.md).

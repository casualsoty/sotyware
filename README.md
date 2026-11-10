# sotyware

sotyware is a cross-platform desktop application meant to run continuously in the
background while staying lightweight. It lives in the system tray and opens a
dashboard window on demand.

**Current state:** this repository contains only the minimal desktop runtime: one
dashboard window, a system tray icon, and the window/process lifecycle. No product
features exist yet. Its purpose is to prove the stack and lifecycle on Windows and Linux
before anything else is built.

## Technologies

| Layer           | Technology                                       |
| --------------- | ------------------------------------------------ |
| Desktop runtime | [Tauri 2](https://v2.tauri.app/) (Rust)          |
| Frontend        | Svelte 5, TypeScript (strict), Vite              |
| Tray (Linux)    | libayatana-appindicator (StatusNotifierItem)     |
| Webview         | WebView2 (Windows), WebKitGTK 4.1 (Linux)        |

Plain Vite + Svelte is used, not SvelteKit, because a single-page dashboard needs no router or SSR.
The only runtime JS dependency is `@tauri-apps/api`. The Rust dependencies are
`tauri` (with the `tray-icon` feature) and, on desktop targets only,
[`tauri-plugin-single-instance`](https://v2.tauri.app/plugin/single-instance/)
`2.x` (resolved to 2.5.5 in `Cargo.lock`). The plugin has no JavaScript API, needs
no capability, and is the only Tauri plugin in use.

## Project layout

```
index.html, src/          Svelte 5 dashboard (App.svelte, main.ts)
src-tauri/
  tauri.conf.json         Window "main", bundle settings, CSP
  capabilities/default.json   Frontend permissions (only core:window:allow-hide)
  src/lib.rs              Builder, single-instance guard, window-close and exit lifecycle, show_dashboard()
  src/tray.rs             Tray icon + menu ("Open Dashboard", "Quit")
  src/main.rs             Binary entry point
  icons/                  Placeholder icons (Tauri defaults, to be replaced)
```

## Prerequisites

All platforms:

- **Rust** ≥ 1.90 (stable), via [rustup](https://rustup.rs/)
- **Node.js** ≥ 20 (developed with 24.x) and npm

### Linux

Debian / Ubuntu (this exact list was used on Ubuntu 24.04):

```bash
sudo apt update
sudo apt install -y build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev
```

For Fedora, Arch, openSUSE and others, use the package lists in the official
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux). These have
not been tested for sotyware.

At runtime, end users need WebKitGTK 4.1, GTK 3 and libayatana-appindicator3. The `.deb`
declares these as dependencies (verified). The `.rpm` dependencies have not been
inspected. The AppImage bundles the libraries.

### Windows 10 / 11

- **Microsoft C++ Build Tools** (or Visual Studio) with the *Desktop development with C++*
  workload.
- **Rust** installed with the MSVC toolchain (`rustup default stable-msvc`).
- **WebView2 runtime.** It ships with Windows 11 and is installed on up-to-date
  Windows 10 systems. The Windows installers that Tauri produces download it
  automatically if it is missing (Tauri's default `downloadBootstrapper` mode).

Build Windows binaries on Windows itself, from a normal Windows path. Do not build
on a WSL path (`\\wsl.localhost\...`).

## Install dependencies

```bash
npm install
```

Rust crates are fetched automatically by Cargo on the first build.

## Run in development

```bash
npm run tauri dev
```

This starts Vite on `http://127.0.0.1:1420` and launches the app with hot reload for the
frontend.

## Build

```bash
npm run tauri build
```

Outputs, under `src-tauri/target/release/`:

- Linux: `sotyware` binary, `bundle/deb/*.deb`, `bundle/rpm/*.rpm`, `bundle/appimage/*.AppImage`
- Windows: `sotyware.exe`, `bundle/msi/*.msi`, `bundle/nsis/*-setup.exe`

To build just the binary: `npm run tauri build -- --no-bundle`.

## Checks

```bash
npm run check                      # svelte-check + TypeScript (fails on warnings)
npm run build                      # frontend production build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

No JS linter (ESLint/Prettier) is configured yet. `svelte-check` runs in strict mode.

## Lifecycle design

The app separates three actions that are easy to confuse:

| User action                                         | Result                                   |
| --------------------------------------------------- | ---------------------------------------- |
| **Hide** (dashboard button)                         | Window hidden; process and tray keep running |
| **Close** (title-bar ✕, Alt+F4, WM close)           | Intercepted and turned into hide; process keeps running |
| **Quit** (tray menu)                                | Process exits cleanly with code 0        |
| **Open Dashboard** (tray menu, or left-click on Windows) | Existing window shown, un-minimized, focused |
| **Launch sotyware again** (while it is running)     | No new process, tray icon or window; the existing dashboard is shown, un-minimized, focused |

How it is implemented (`src-tauri/src/lib.rs`):

- **One window, never destroyed.** The `main` window is created from `tauri.conf.json`.
  `WindowEvent::CloseRequested` calls `api.prevent_close()` and hides the window. Because
  the window always exists, "Open Dashboard" reuses it via `get_webview_window("main")`
  and can never create a duplicate.
- **Quit is the only way out.** The tray "Quit" item calls `AppHandle::exit(0)`. In
  `RunEvent::ExitRequested`, an exit request with `code: None` (the runtime deciding to
  stop because no windows remain) is cancelled with `prevent_exit()`. Requests with an
  explicit code, which only come from `exit()`, are allowed.
- **No polling, no background tasks.** Everything is driven by Tauri window, menu and
  run-loop events. All handlers are registered once, in the builder and in `setup`, which
  runs once.
- **Hide uses the native window API.** The frontend calls `getCurrentWindow().hide()`.
  The dashboard's capability grants only `core:window:allow-hide`, not even `core:default`.
- **A missing tray is fatal.** If the tray icon cannot be created, the app prints the
  error and exits with code 1. Without a tray, a hidden dashboard could never be
  restored and the process could only be killed.
- **CSP** is restricted to `default-src 'self'` (plus inline styles for Svelte).

### Single instance

sotyware uses Tauri's official
[single-instance plugin](https://v2.tauri.app/plugin/single-instance/) rather than a
custom lock file or socket.

- **Registered first.** The plugin is the first (and only) `.plugin(...)` call on the
  builder, as its documentation requires. Its setup runs inside `Builder::build()`,
  which happens before Tauri creates the `main` window from `tauri.conf.json` and before
  our `setup` creates the tray. A second instance is therefore detected before it has
  any window, tray icon or app state.
- **Second launch.** The new process finds the running instance, sends it its
  command-line arguments and working directory, and exits with code 0. It never
  reaches `run()`.
- **Running instance.** The plugin calls our callback, which only calls
  `show_dashboard()`: the same function the tray "Open Dashboard" item uses. It looks
  up the existing `main` window and un-minimizes, shows and focuses it. It never
  creates a window, so it works whether the dashboard is visible, hidden or
  minimized. Arguments and working directory are ignored for now.
- **How the instances talk.**
  - Linux: D-Bus session bus. The first instance owns the name
    `com.sotyware.desktop.SingleInstance` (from `identifier` in `tauri.conf.json`). A
    second instance calls a method on it, then exits.
  - Windows: a named mutex (`com.sotyware.desktop-sim`) plus a hidden message window
    that receives the arguments via `WM_COPYDATA`. The second instance grants the
    first one permission to take the foreground (`AllowSetForegroundWindow`) before
    exiting.
- **Quit releases the lock.** Tauri releases the D-Bus name / mutex when the app
  exits, so launching after "Quit" starts a fresh instance.
- **Scope.** "Single" means one instance per user session: per D-Bus session bus on
  Linux, per Windows session for the (session-local) mutex. The plugin's `semver`
  feature is not enabled, so different versions of sotyware also exclude each other.

Decision record: the official plugin was chosen over a hand-written lock because it
already handles the per-platform IPC, stale locks after a crash (the OS releases
D-Bus names and mutexes when a process dies), forwarding of arguments and the
Windows foreground-permission rule. It adds no frontend permission. It is limited to
desktop targets with `cfg(any(target_os = "macos", windows, target_os = "linux"))` in
`Cargo.toml` and `#[cfg(desktop)]` in `lib.rs`, as its documentation recommends.

## Validation status

Be careful to distinguish tested from untested here.

### Tested: Linux (Ubuntu 24.04 under WSL2 / WSLg)

Release binary, driven by an automated script. The script uses a private D-Bus session
with a minimal `org.kde.StatusNotifierWatcher` standing in for the tray host. It clicks
menu items over `com.canonical.dbusmenu`, sends `WM_DELETE_WINDOW` to simulate the
close button, and uses XTest to click the Hide button.

- Launch shows the dashboard; the tray item registers with "Open Dashboard" and "Quit". ✅
- Hide button hides the window; process stays alive. ✅
- Tray "Open Dashboard" restores the same single window. ✅
- Close request hides (does not destroy) the window; process stays alive. ✅
- Repeated "Open Dashboard" does not create a second window. ✅
- Tray "Quit" from the hidden state exits with code 0. ✅
- Native Wayland backend: app runs, tray registers, "Quit" exits with code 0. ✅
  Window state was not inspected on Wayland.
- Tray creation failure (missing `$XDG_RUNTIME_DIR`): clean exit with code 1. ✅
- `.deb`, `.rpm` and AppImage bundles build. ✅ (Built, not installed or run.)

This is **not** a real desktop environment. The tray host was a test stub, so no tray
icon was ever seen or clicked by a person.

### Not tested

- **Windows 10 / 11: entirely untested.** The code uses only cross-platform Tauri APIs.
  Left-click-to-open is Windows-specific behaviour that has been reviewed but not run.
  The single-instance guard on Windows (named mutex + `WM_COPYDATA`) has only been
  read in the plugin's source, never built or run on Windows.
- Real Linux desktops: GNOME, KDE Plasma, XFCE, etc.
- Installing and running the `.deb`/`.rpm`/AppImage bundles.

## Known limitations

**Linux tray**

- The tray uses the StatusNotifierItem/AppIndicator protocol. KDE Plasma, Cinnamon,
  XFCE (status notifier plugin) and Budgie are expected to show it. Vanilla **GNOME
  hides tray icons** unless the *AppIndicator and KStatusNotifierItem Support* extension
  is installed (Ubuntu enables it by default). None of this has been verified yet.
- With no StatusNotifierItem host, libayatana falls back to a legacy XEmbed icon. On
  desktops without an XEmbed tray, the icon is invisible. In that case a hidden
  dashboard cannot be restored and the process must be stopped manually. The app keeps
  running in this state; it cannot detect the missing host.
- Tray icon click events are not emitted on Linux. Any click opens the menu, so there is
  no left-click-to-open.
- The tray icon image is written to `$XDG_RUNTIME_DIR`. If that directory is missing
  (seen under WSL), the app exits with code 1.

**Single instance**

- **Linux needs a D-Bus session bus.** Without one (e.g. a bare SSH or cron
  environment), the plugin logs a warning and starts normally, so the guard is off
  and a second process can start. All normal desktop sessions have a session bus.
- **Snap and Flatpak** sandboxing blocks the D-Bus name by default. sotyware ships
  neither; if it ever does, the manifest must allow `com.sotyware.desktop.SingleInstance`
  (see the plugin docs).
- **Focus is up to the window manager.** The app asks for focus, but Wayland
  compositors and Windows' focus-stealing rules may refuse and flash the taskbar entry
  or show a notification instead. The dashboard is still shown and un-minimized.
- **Very early second launch.** If a second launch arrives in the brief moment after
  the first instance registered its lock but before its window exists, the callback
  logs `dashboard window 'main' not found` and does nothing. The first instance then
  shows its dashboard anyway, because it starts visible. No duplicate is created.
- One instance **per user session**: two users logged in at the same time each get
  their own.
- Command-line arguments of the second launch are received but ignored.

**General**

- No autostart on login.
- Icons are Tauri's placeholder icons.
- `npm audit` reports a low-severity advisory in `deepmerge`, a build-time dependency of
  `@sveltejs/vite-plugin-svelte`. It is not shipped to users, and no fix is available
  yet.

**WSL-specific (development only)**

- AppImage bundling fails under WSL because `linuxdeploy` crashes on the Windows
  directories that WSL appends to `PATH`. Build with them removed:

  ```bash
  PATH=$(echo "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd:) npm run tauri build
  ```

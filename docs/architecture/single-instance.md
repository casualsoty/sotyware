# Single-instance behaviour

sotyware allows one running instance per user session. It uses Tauri's official
[single-instance plugin](https://v2.tauri.app/plugin/single-instance/)
(`tauri-plugin-single-instance`, `2` in `Cargo.toml`, resolved to 2.5.5 in
`Cargo.lock`). Why: [decisions.md](decisions.md#adr-005-use-the-official-single-instance-plugin).

## How it works

- **Registered first.** The plugin is the first and only `.plugin(...)` call on
  the builder in `run()`, as its documentation requires. Its setup runs inside
  `Builder::build()`, before Tauri creates window `main` and before `setup`
  creates the tray. A second instance is therefore detected before it has any
  window, tray icon or state.
- **Second launch.** The new process finds the running instance, sends it its
  command-line arguments and working directory, and exits with code 0 once the
  running instance accepted them. It never reaches `app.run()`.
- **Running instance.** The plugin calls the callback in `run()`, which only calls
  `show_dashboard()`, the same function the tray "Open Dashboard" item uses. It
  works whether the dashboard is visible, hidden or minimized. Arguments and
  working directory are ignored for now.
- **Quit releases the lock.** The lock is released when the process exits (and by
  the OS if it crashes), so launching after "Quit" starts a fresh instance.
- **Desktop only.** Gated by
  `cfg(any(target_os = "macos", windows, target_os = "linux"))` in `Cargo.toml` and
  `#[cfg(desktop)]` in `lib.rs`.

## Per-platform mechanism (from the plugin's 2.5.5 source)

`<identifier>` is `com.sotyware.desktop` (`tauri.conf.json`). The plugin's
`semver` feature is not enabled, so different sotyware versions also exclude each
other.

- **Linux:** D-Bus session bus. The first instance owns the name
  `com.sotyware.desktop.SingleInstance` and serves
  `/com/sotyware/desktop/SingleInstance`. A second instance gets `NameTaken`, calls
  `ExecuteCallback` on it, then exits 0.
  - **No session bus:** the plugin emits a `tracing::warn!` and starts normally,
    so the guard is off. sotyware installs no `tracing` subscriber, so **this
    warning is not printed anywhere**.
- **Windows:** a named mutex `com.sotyware.desktop-sim` plus a hidden message
  window that receives arguments via `WM_COPYDATA`. The second instance calls
  `AllowSetForegroundWindow` for the first one before exiting. Read in source only;
  never built or run.

## Scope

"Single" means per user session: per D-Bus session bus on Linux, per Windows
session for the session-local mutex. Two users logged in at once each get their
own instance.

Limitations (D-Bus fallback, startup race, Snap/Flatpak, focus):
[limitations.md](../status/limitations.md#single-instance). Test coverage:
[validation matrix](../status/status.md#validation-matrix).

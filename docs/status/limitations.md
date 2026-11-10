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
  entry or notify instead. The dashboard is still shown and un-minimized. Not
  verified on any real window manager.
- **Per user session.** Two users logged in at once each get their own instance.
- **Arguments ignored.** The second launch's arguments and working directory reach
  the running instance but are not used.

## General

- **Placeholder icons.** `src-tauri/icons/` contains Tauri's default icons.
- **No autostart** on login.
- **Logging is `eprintln!` only.** No log files, levels or subscriber; output is
  lost when started from a desktop launcher.
- **`npm audit`** reports a low-severity advisory in `deepmerge`
  (GHSA-gcqx-w48m-w2h4), a build-time dependency of
  `@sveltejs/vite-plugin-svelte`, with no fix available (re-checked 2026-11-10).
  It is not shipped to users.

## Development environment (WSL)

See [setup.md](../development/setup.md#wsl2-notes): Windows `npm` shadowing the
Linux toolchain, AppImage bundling needing a `PATH` filter, WSLg ignoring focus and
minimize requests.

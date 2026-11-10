# Cross-platform development

Target platforms: **Windows 10/11 and Linux desktops**. macOS is not a target,
although the single-instance dependency is also enabled for it (see below).
Mobile is not a target; the `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on
`run()` is Tauri template boilerplate.

## Rules

- **Prefer cross-platform Tauri APIs.** All current code uses them; there is no
  `#[cfg(target_os = ...)]` in sotyware's own source.
- **Gate desktop-only code twice, as the single-instance plugin is gated:** in
  `Cargo.toml` with
  `[target.'cfg(any(target_os = "macos", windows, target_os = "linux"))'.dependencies]`,
  and in code with `#[cfg(desktop)]`.
- **Do not rely on behaviour one platform lacks.** If a feature cannot work on
  one platform, say so in a code comment at the call site and in
  [the known limitations](../status/limitations.md).
- **Keep `src-tauri/src/main.rs`'s `windows_subsystem = "windows"` attribute.**
  It stops a console window from opening with release builds on Windows.
- **Keep the `_lib` suffix on the library name** (`sotyware_lib`). It avoids an
  output filename collision on Windows (rust-lang/cargo#8519).
- **Code you have not run on a platform is unverified there.** Say so in commit
  messages and in [the status document](../status/status.md).

## Platform differences that affect current code

| Area | Windows | Linux |
| --- | --- | --- |
| Webview | WebView2 | WebKitGTK 4.1 |
| Tray protocol | native notification area | StatusNotifierItem via `tray-icon`'s `ksni` backend (pure Rust, `zbus`); no watcher or no session bus ⇒ tray creation fails ⇒ exit 1; no XEmbed fallback |
| Tray icon click events | emitted; left click opens the dashboard | left click (`Activate`) and middle click (`SecondaryActivate`) emitted if the tray host sends them; left click opens the dashboard |
| Tray icon image | in memory | sent over D-Bus as a pixmap; nothing written to disk |
| Tray backend selection | `tray-icon` Windows backend | Linux-only `tray-icon` dependency with feature `ksni` in `Cargo.toml` ([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)) |
| Single-instance IPC | named mutex `<identifier>-sim` + hidden message window, `WM_COPYDATA` | D-Bus session bus name `<identifier>.SingleInstance` |
| Focus on second launch | subject to foreground-lock rules; plugin calls `AllowSetForegroundWindow` | up to the window manager / compositor |
| Build host | must build on Windows, from a Windows path | native or WSL2 |

`<identifier>` is `com.sotyware.desktop` from `src-tauri/tauri.conf.json`.
Changing the identifier changes the single-instance lock names.

Details: [single-instance.md](../architecture/single-instance.md),
[lifecycle.md](../architecture/lifecycle.md),
[limitations.md](../status/limitations.md).

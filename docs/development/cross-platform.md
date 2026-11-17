# Cross-platform development

Code targets: **Windows, Linux and macOS desktops**; every change must keep
all three building and working (macOS since ADR-037; it is compiled and
tested only on GitHub's macOS runners, [macos.md](macos.md)). This is not
the release support scope: the first release targets Windows 11 only, and
Windows 10, Linux and macOS are not validated
([release.md](release.md#support-scope)).
Mobile is not a target; the `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on
`run()` is Tauri template boilerplate.

## Rules

- **Prefer cross-platform Tauri APIs.** All current code uses them. Platform
  checks in sotyware's own source are few and kept where the difference is:
  `tray::unavailable_reason()` (only Linux has tray errors that mean "no
  tray host"), the credential store backends in `cloud/keyring.rs` (one per
  OS), the macOS menu bar and Dock click (`app_menu.rs`, `RunEvent::Reopen`
  in `lib.rs`), `cfg!(windows)` for whether an installer finishes an
  update (`updates.rs`), and the autoclicker's input backends
  (`automation/native.rs` picks `windows.rs` or `x11.rs`, none on macOS;
  everything else in `automation/` is platform-independent and tested on
  every platform through the mock backend). The rest of the code sees only the
  `OperatingMode`. Keep pure decisions in plain functions so every
  platform's tests check them (as `macos_kind` and `app_menu`'s
  `MenuAction` are tested on Linux too).
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

macOS (ADR-037) behaves like Windows' tray mode: always a tray (menu bar)
icon, close hides, plus a menu bar with Quit ⌘Q and the Edit menu the
webview needs for ⌘C/⌘V, and a Dock click shows the dashboard. Webview
WKWebView; credentials in the login keychain; single instance through a
Unix socket; builds on a Mac only, one per architecture. Details:
[macos.md](macos.md#what-is-mac-specific).

| Area | Windows | Linux |
| --- | --- | --- |
| Webview | WebView2 | WebKitGTK 4.1 |
| Tray protocol | native notification area | StatusNotifierItem via `tray-icon`'s `ksni` backend (pure Rust, `zbus`); no XEmbed fallback |
| Without a tray | every tray creation error ⇒ exit 1 | no session bus, no watcher or no host ⇒ dashboard-only mode (close quits, no hide); other tray errors ⇒ exit 1 ([ADR-012](../architecture/decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode)) |
| Operating modes in practice | tray mode only | tray mode with a StatusNotifierItem host (KDE, GNOME with the AppIndicator extension, XFCE plugin, …); dashboard-only mode on vanilla GNOME and XEmbed-only panels (expected, not yet checked on a real desktop) |
| Tray icon click events | emitted; left click opens the dashboard | left click (`Activate`) and middle click (`SecondaryActivate`) emitted if the tray host sends them; left click opens the dashboard |
| Tray icon image | in memory | sent over D-Bus as a pixmap; nothing written to disk |
| Refused TCP connection (closed local port) | reported after about 2 s (Windows retries the connection); a shorter connect or read timeout reports a timeout instead | reported at once |
| Cloud storage TLS | rustls on ring, OS root certificates (Windows certificate store) | rustls on ring, OS root certificates (system CA bundle) |
| Tray backend selection | `tray-icon` Windows backend | Linux-only `tray-icon` dependency with feature `ksni` in `Cargo.toml` ([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)) |
| Single-instance IPC | named mutex `<identifier>-sim` + hidden message window, `WM_COPYDATA` | D-Bus session bus name `<identifier>.SingleInstance` |
| Focus on second launch | subject to foreground-lock rules; plugin calls `AllowSetForegroundWindow` | up to the window manager / compositor |
| Build host | must build on Windows, from a Windows path | native or WSL2 |
| Input automation (ADR-050) | `SendInput`, low-level hooks, `RegisterHotKey` on one input thread; physical pixels (per-monitor DPI aware v2); no input to elevated programs (UIPI) | X11 only: XTest, RECORD, `XGrabKey` (libraries loaded at runtime); refused on Wayland sessions; macOS: none ([automation.md](../architecture/automation.md#native-input)) |
| Application manifest | Common Controls v6 manifest embedded by the linker into every binary, unit-test binaries included (`build.rs`, ADR-034); without it a test that builds a Tauri app does not start | none |

`<identifier>` is `com.sotyware.desktop` from `src-tauri/tauri.conf.json`.
Changing the identifier changes the single-instance lock names.

Details: [single-instance.md](../architecture/single-instance.md),
[lifecycle.md](../architecture/lifecycle.md),
[limitations.md](../status/limitations.md).

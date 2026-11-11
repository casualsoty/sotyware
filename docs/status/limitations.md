# Known limitations

Known gaps in the current implementation. These are **not** solved. When one is
fixed, remove it here in the same change and update
[status.md](status.md).

## Linux tray

The tray is a StatusNotifierItem served by `tray-icon`'s pure-Rust `ksni`
backend (`ksni` 0.3.7 over `zbus`), not libayatana-appindicator
([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)).

- **Desktop support is unverified.** KDE Plasma, Cinnamon, XFCE (status notifier
  plugin) and Budgie are expected to show the item (tray mode). Vanilla **GNOME
  has no StatusNotifierWatcher** unless the *AppIndicator and KStatusNotifierItem
  Support* extension is installed (Ubuntu enables it by default), so it is
  expected to run in dashboard-only mode. None of this has been verified on a
  real desktop.
- **Without a tray host sotyware does not run in the background.** No session
  bus, no `org.kde.StatusNotifierWatcher`, or a watcher reporting no
  StatusNotifierHost selects dashboard-only mode
  ([ADR-012](../architecture/decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode)):
  the dashboard stays open, cannot be hidden, and closing it quits. There is
  **no XEmbed fallback**: on desktops with only a legacy XEmbed tray (for
  example some window-manager panels) sotyware runs dashboard-only. A watcher
  that starts *after* sotyware (for example a login race, if autostart is ever
  added) is not picked up until the next start. Checked under the virtual
  display only (2026-11-10).
- **Some tray-host failures are still fatal.** Only the errors listed in
  [lifecycle.md](../architecture/lifecycle.md#mode-selection) select
  dashboard-only mode. A watcher that disappears *during* the registration call
  (`NoReply`, the same error as a hung watcher), a watcher that answers with
  another error, a bus socket sotyware may not open (`PermissionDenied`) or a
  malformed `DBUS_SESSION_BUS_ADDRESS` exit 1. The message goes to stderr only,
  so someone starting sotyware from a launcher sees nothing happen.
- **A watcher without a visible host is only partly detected.** A watcher that
  reports `IsStatusNotifierHostRegistered = false` selects dashboard-only mode.
  Real watchers (KDE, the GNOME extension) always report `true`, according to
  `ksni`'s source comments (not checked). If such a watcher accepts the item but
  no panel displays it, sotyware runs in tray mode with an invisible icon. A
  hidden dashboard can then be reached only by launching sotyware again.
- **Left click depends on the tray host.** The item is not marked menu-only, so
  hosts are expected to send a left click as `Activate`, which opens the
  dashboard. The menu is on right click. Seen only with D-Bus calls against the
  stub tray host, not with a real panel.
- **The `.deb` still declares `libayatana-appindicator3-1`.** Tauri's bundler
  adds it whenever the tray feature is on, although the binary never loads the
  library. It costs an unnecessary package, nothing more, and is unrelated to
  dashboard-only mode. Running without the library installed has not been
  tried. Tracked as a separate follow-up in [roadmap.md](roadmap.md).

## Dashboard-only mode

- **Only the virtual display has run it.** The no-watcher, no-host and no-bus
  cases ran under Xvfb + openbox with a stub watcher (2026-11-10). Not run on a
  real GNOME, an XEmbed-only panel, Wayland or any other real desktop, and never
  looked at by a person.
- **The note is the only indication.** Nothing is shown outside the window (no
  notification, no dialog). The reason is printed on stderr.
- **Closing quits without asking.** The note says so, but there is no
  confirmation. Unsaved state does not exist yet, so nothing is lost today.
- **The mode cannot change while running.** A tray host that appears later is
  ignored until sotyware is started again; one that disappears in tray mode is
  re-registered by `ksni` when it comes back, as before, but the dashboard can
  still be hidden in the meantime.

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
  The tray also needs the session bus, so such a process runs in dashboard-only
  mode (`no system tray available (no D-Bus session bus)` on stderr). Further
  launches then start further dashboard-only processes, each with its own
  window (derived from the code; not tested).
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
- **The NSIS uninstaller leaves a registry key.** After `uninstall.exe /S`,
  `HKCU\Software\sotyware\sotyware` (default value: the install directory) is
  still present; files, the uninstall entry and the shortcuts are removed.
  Observed on Windows 11 at `e8834fd` (2026-11-10); the key was already left by the
  earlier install at `1f5853b`. Whether the WebView2 data folder
  `%LOCALAPPDATA%\com.sotyware.desktop` would be removed is unknown, because
  running the build output also creates it.

## Built-in features

Design and rationale: [features.md](../architecture/features.md),
[ADR-014](../architecture/decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry).

- **Only one feature, and it does nothing.** `example` demonstrates the
  lifecycle. There are no product features.
- **No events.** The page sees a state change only in the response to its own
  switch, plus the list it reads on mount. A feature still `initializing` when
  the page loads is shown so until the page is reloaded.
- **A running feature cannot report a later failure.** It stays `running`.
- **Panics are not isolated.** Release builds abort on panic, so a panicking
  feature ends the whole process. Only returned errors are isolated.
- **Stop at exit only on Tauri's exit path.** `stop_all` runs on
  `RunEvent::Exit`. A killed process, a crash, or an OS shutdown or logoff that
  ends the process without that path skips it. The Windows behaviour on
  logoff/shutdown is not checked.
- **Validation:** the unit tests run on Linux and Windows; the real page is
  checked by the Linux virtual-display test and the Windows real-desktop test
  (`windows.ps1`, 2026-11-11). The Windows private-desktop test
  (`windows-isolated.ps1`) has no feature checks. Invalid feature IDs and
  transitions are tested only at the registry level: the dashboard cannot send
  them, and no test sends them through the real IPC. Not checked on Windows 10,
  real Linux desktops or the installed copies.

## Feature preferences

Design and rationale: [features.md](../architecture/features.md#persistence),
[ADR-015](../architecture/decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait).

- **A corrupt or unusable database needs manual recovery.** sotyware never
  repairs, replaces or deletes `preferences.sqlite3`. While it cannot be used,
  every feature runs with its default, the dashboard shows "Could not load the
  saved feature choices", and no change is saved. To recover: quit sotyware and
  move the file away (Linux `~/.local/share/com.sotyware.desktop/`, Windows
  `%LOCALAPPDATA%\com.sotyware.desktop\`); the next start creates a new one.
  The same applies to a database from a newer sotyware (no downgrade).
- **No retry.** A database that could not be opened at startup is not tried
  again until the next start, and a failed save is not retried.
- **An unsaved switch is not undone.** If saving fails, the feature stays as
  switched until sotyware quits; the next start uses the last saved choice or
  the default. The dashboard says so.
- **An unusable data folder stops sotyware.** If the app's local data folder
  cannot be created (for example `$XDG_DATA_HOME` is a file), Tauri fails its
  own setup for the webview data (`Failed to setup app`, abort, exit by
  `SIGABRT` on Linux) before any window opens. This was so before persistence;
  seen 2026-11-11 under the virtual display. sotyware logs the preferences error
  first.
- **A save in flight when the process is killed is lost.** SQLite rolls it back
  on the next open; earlier saves are intact. On a normal exit, a save in
  progress finishes first.
- **A save can lose to a second process.** With single instance off (no D-Bus
  session bus), a save waits at most 2 s for the other process's lock, then
  fails and is shown like any failed save. SQLite's locking is not fair, so a
  connection can be starved; seen only in a stress test of 4 connections saving
  back to back, on a GitHub-hosted Windows runner (2026-11-11), not in use.
- **The file outlives an uninstall.** Like the webview data next to it, the
  database is in the app's local data folder; whether the NSIS/MSI uninstallers
  remove that folder is not checked (see [Windows installers](#windows-installers)).
- **Stale saves are excluded by design, not by a test.** Concurrent switches
  store the final state because each save reads and writes under one lock; the
  unit test checks the result under contention but cannot force the one
  interleaving a naive implementation would get wrong.
- **Validation:** unit tests on Linux and Windows; real restarts through the
  Linux virtual-display test and the Windows real-desktop test (2026-11-11).
  Corrupt and unopenable databases are tested on the real binary only on Linux
  (Windows: unit tests). Not checked on Windows 10, real Linux desktops, the
  installed copies, a roaming or redirected profile, or a network drive.

## Cloud storage (proof of concept)

Design: [cloud-storage.md](../architecture/cloud-storage.md); providers:
[cloud-providers.md](../architecture/cloud-providers.md).

- **Not part of the app.** No feature, command or dashboard element uses the
  `cloud` module; a user cannot configure an account or transfer a file.
- **Backblaze verified only for what the opt-in test does; Hetzner never
  run.** Against a real B2 bucket in `eu-central-003` (2026-11-11, WSL2 and
  native Windows 11): endpoint, region, path style, a 200 kB PUT, a 3-part
  multipart upload, SHA-256 round trip, hide-on-delete, delete by version and
  the wrong-secret error ([status.md](status.md#backblaze-b2-against-the-real-provider-2026-11-11)).
  Still unverified at Backblaze: an unknown key id, the SDK's checksum headers
  (turned off), files larger than 12 MiB or the 5 GB single-PUT limit, many
  parts, throttling (429/503), interrupted transfers, Object Lock and
  lifecycle rules, and keys without a prefix restriction. Hetzner: nothing
  (path-style addressing, the minimum part size, its error codes).
- **Credentials only in the OS credential store, and no fallback**
  ([cloud-storage.md](../architecture/cloud-storage.md#credentials), ADR-017).
  Linux without a Secret Service (no keyring daemon, no session bus: WSL2,
  minimal window managers, headless sessions, SSH logins) cannot save or read
  any account credentials; every call reports `Unavailable`. A locked keyring
  whose unlock prompt cannot be shown (no prompter, as in a headless session)
  is `AccessDenied`. Neither has a workaround in sotyware by design.
- **Secret Service calls can block** on an unlock prompt until the user
  answers; callers must stay off the UI thread (nothing calls them yet).
- **Windows access errors are not told apart.** The store crate's error type
  is private, so `ERROR_ACCESS_DENIED` and every other Win32 code except "no
  logon session" are reported as a backend failure with the code.
- **Backends validated:** Windows Credential Manager on native Windows 11
  (2026-11-11) and GNOME Keyring 46.1 on a private D-Bus session under WSL2
  (2026-11-11), both by the opt-in test only. Not tested: KWallet, KeePassXC,
  any real Linux desktop session, Windows 10, a roaming or domain profile, an
  unlock prompt answered by a person, two sotyware processes writing at once.
- **Credentials do not travel with the accounts database.** Copying
  `cloud-accounts.sqlite3` to another machine (or restoring it) leaves its
  accounts without credentials (`NotFound`) until their keys are entered
  again. Removing the database by hand leaves orphaned OS entries; they are
  visible in Credential Manager or Seahorse under `com.sotyware.desktop`.
- **Environment-variable credentials exist only in test builds** (the opt-in
  provider tests); the app cannot use them, and nothing imports them.
- **Uploads:** the file is read twice (hash, then send); the parts of one file
  are sent one after another, not in parallel; an interrupted upload starts
  again from the beginning (no resume); a file that changes during upload is
  detected only by size and modification time.
- **Deletes on versioned buckets:** `delete_object` without a version id only
  hides the object on Backblaze (always versioned) and on a Hetzner bucket with
  versioning; the data stays billed. Only the exact version returned by the
  upload is removed for good.
- **No retrying queue.** Transient errors are retried only by the SDK (3
  attempts per request); a failed transfer is not queued again.
- **Pricing is not in the code.** The cost figures in cloud-providers.md are
  estimates from list prices of 2026-11-11, with VAT for French consumers and
  card currency fees unconfirmed.
- **Test server limits:** `s3s-fs` lists nothing below a second folder level
  (seen with restic's `data/xx/` packs, so `restic check` against it reports
  missing packs), stores no versions, and does not check SigV4 checksums of
  streamed bodies the way a provider might. Its errors are not the providers'
  errors.
- **Dependencies:** `aws-sdk-s3` is held at 1.102.0 by `rust-version = 1.90`;
  `s3s` 0.12 needs the dev-only `digest` release-candidate pin.

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
- **On Linux the "Hide dashboard" button is activated through AT-SPI**, not
  with a pointer click (virtual-display scenario 12). The dashboard-only page is
  checked the same way. What a person sees is not checked.
- **The virtual-display test passes only on WSL2** (Ubuntu 24.04.1, with
  the apt-installed packages from [setup.md](../development/setup.md#linux-system-packages))
  and in an Ubuntu 26.04 Docker container on that machine
  ([status](status.md#virtual-display-test-on-ubuntu-2604-container-2026-11-10)),
  not on a native Ubuntu install. Its one GitHub run failed on a driver bug
  since fixed, and has not been repeated. With libayatana-appindicator it
  failed, correctly, about 1 run in 60 on the
  [registration race](#historical-libayatana-appindicator-registration-race).
  With the `ksni` backend no run has failed so far (3 runs and a 420-launch
  loop, 2026-11-10; 3 runs of the 92-check driver with dashboard-only mode,
  2026-11-10). That is evidence, not proof.
- **No test covers screen-reader access** on either platform.
- **No Windows test of the real binary is headless.** Starting sotyware needs a
  logged-in session (window station and desktop); there is no Windows
  counterpart of Xvfb. `npm run test:headless` is the only Windows suite that
  needs no session.
- **The Windows private-desktop test does not cover the tray.** A private
  desktop has no taskbar, so the tray icon, menu, left click and Quit are
  tested only by `windows.ps1` on the interactive, unlocked desktop, which moves
  the mouse. The private-desktop test also does not use real clicks or check
  focus. On quit it is ended with `Stop-Process`.
- **The Windows private-desktop test cannot run elevated, so not on
  GitHub-hosted runners.** With an administrator token, WebView2 starts its
  browser process de-elevated through `explorer.exe`, on Explorer's desktop
  instead of the private one, and sotyware's webview cannot be created there
  (`0x80070578`). The test reports NOT RUN (exit 4) when elevated. Hosted jobs are
  elevated: the first hosted run failed this way, the second (with the check)
  printed `elevated: True` and reported NOT RUN
  ([testing.md](../development/testing.md#windows-isolated-desktop-test),
  [evidence](status.md#windows-11-isolated-desktop-test-elevated-2026-11-11)).
  Starting sotyware non-elevated from an elevated job has not been tried.
- **Windows Session 0 is untested.** Whether WebView2 and the private-desktop
  test work in Session 0 (a service, a scheduled task without a logged-on user)
  is unknown
  ([testing.md](../development/testing.md#headless-windows-desktop-testing)).

## CI

- **The Windows private-desktop test cannot pass on GitHub.** Both automatic
  `ci.yml` runs passed (Ubuntu and Windows jobs) and `desktop-virtual.yml`
  passed (92/0/0), but `desktop-windows-isolated.yml` reports NOT RUN on
  elevated hosted runners, which is not a pass (results and causes in
  [status.md](status.md#ci-github-actions-2026-11-10)). A passing virtual-display
  run is not a real Linux desktop: GNOME, KDE and XFCE remain untested. Real
  job durations are not recorded, so the cost figures are still estimates.
- **CI time is a limited allowance.** The repository is private on GitHub Free:
  $12 of hosted compute a month, after which Actions is blocked until the next
  cycle (no payment method is set up). Hence only the headless jobs run
  automatically, and the desktop tests only by hand
  ([budget](../development/testing.md#github-free-budget)).
- **Automatic CI never starts sotyware.** Window, tray, single-instance and close
  behaviour are covered there only by unit tests with doubles; the desktop
  workflows are manual and may lag behind `main`.
- **Not in CI at all:** the real-desktop Windows test (needs an unlocked desktop
  with Explorer and the mouse), the manual checklist, Windows 10, real Linux
  desktops, installers.
- **Docs-only changes skip CI** (`paths-ignore`), so a required status check
  would never report on such a pull request; branch protection is not
  configured.
- **The desktop workflows hard-code the expected check counts** (92 and 56); a
  change to a test driver must update `EXPECTED_CHECKS` in its workflow.
- **No real Linux desktop is available in the development environment**
  (2026-11-10: WSL2 only, Windows 11 Home without a VM;
  [survey](status.md#real-linux-desktop-availability-2026-11-10)). Linux results
  come only from WSLg and Xvfb with a stub tray host.

## Development environment (WSL)

See [setup.md](../development/setup.md#wsl2-notes): Windows `npm` shadowing the
Linux toolchain, AppImage bundling needing a `PATH` filter, WSLg ignoring focus and
minimize requests.

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

- **Nothing is signed.** There is no code-signing certificate; the MSI, the
  NSIS setup program and `sotyware.exe` are unsigned, so SmartScreen warns
  on every download and Smart App Control (where enabled) blocks them.
  Requirements and verification: [release.md](../development/release.md#signing-not-set-up).
- **The installers of the current code were never installed.** Fresh
  install, upgrade with user data, uninstall, app-data deletion,
  credential leftovers and reinstall need a disposable Windows 11 VM or
  machine ([plan](../development/release.md#installation-test-plan-disposable-windows-11)).
  The development machine has a real sotyware installation sharing the
  MSI's UpgradeCode and the real profile, so it must not be used.
- **No release support beyond Windows 11.** Windows 10 and Linux are not
  validated and are not claimed ([scope](../development/release.md#support-scope)).
- **A release binary must be built with `npm run build:release`** (ADR-028);
  a plain `tauri build` leaves 601 build-machine paths in the Windows binary
  and its bundling is refused.
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

- **Two features, both switches for background work.** `backup-scheduler`
  and `update-check` start and stop work that lives elsewhere (`crate::backup`,
  `crate::updates`). The `example` feature was removed (2026-11-15); a choice
  stored for it stays in `preferences.sqlite3`, ignored.
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

## Cloud storage

Design: [cloud-storage.md](../architecture/cloud-storage.md); providers:
[cloud-providers.md](../architecture/cloud-providers.md).

- **No overwrite, no history, no backups.** The dashboard manages
  accounts, browses buckets, transfers files (single and several-item
  downloads, file, folder and drag-and-drop uploads) and manages files
  (ADR-026, below); it never overwrites, and has no transfer queue, upload
  or download history, backups or sync
  ([cloud-storage.md](../architecture/cloud-storage.md#file-management)).
- **Downloads at a real provider: three single Backblaze runs** (2026-11-12).
  One through the app's download code in Rust (WSL2, no UI): `GetObject`
  returned `Content-Length` and the stored `sotyware-sha256`, the file was
  SHA-256 verified, and the progress bytes never went down
  ([status.md](status.md#one-download-from-backblaze-2026-11-12)). One
  through the complete native Windows 11 flow (dashboard, browser, native
  save dialog, real Credential Manager): `sha256-verified`, byte-for-byte
  equal, completion only after the file was in place
  ([status.md](status.md#real-backblaze-download-through-the-windows-dashboard-2026-11-12));
  there the 16 MiB arrived within 0.6 s, so the UI showed only one progress
  line ("0% · 0 B of 16 MiB"). The Windows flow passed again with the
  hardened test driver at `bb8c178`, showing two progress lines
  ([status.md](status.md#hardened-backblaze-download-test-on-the-windows-dashboard-2026-11-12)).
  Not run: more than one run of the Rust-only test, a
  download through the UI from a real provider on Linux or Windows 10 or by
  a person, larger or multipart-uploaded files, a checksum mismatch or a
  missing checksum at the provider, Hetzner. On Linux the UI and dialog are
  verified only against the in-process object server
  ([status.md](status.md#cloud-storage-single-file-downloads-2026-11-12)).
- **Download limits:** one at a time; no resume (a failed or cancelled
  download starts over); no retries inside a download (the SDK retries only
  the request, not a broken body); a body that delivers nothing for about
  5 s (the SDK's stalled-stream protection) fails as a network error;
  leaving the browser cancels the download; reloading the page while a
  download runs loses its progress display, and new downloads are refused as
  busy until it ends; editing or removing the account does not stop a
  running download; objects without the SHA-256 that sotyware stores at
  upload are checked only by size (the page says so).
- **A killed process leaves its temporary download file until the next
  start.** Quitting cancels and removes it (waiting up to 5 s); after a crash,
  power loss or `kill -9` the hidden `.<name>.<pid>-<n>.sotyware-partial`
  stays next to the destination until sotyware starts again and removes it
  from the operation journal's record (ADR-030). A file whose path cannot be
  written as UTF-8 (a non-UTF-8 destination name on Linux) is recorded with
  a lossy path and is then not found by that cleanup. If the journal could
  not be opened, nothing is recorded or cleaned up.
- **Replacing a file relies on the save dialog's confirmation.** The GTK and
  Windows dialogs ask before returning an existing file, and only such a
  file is replaced. On file systems without hard links (FAT, exFAT, some
  network shares) a file that appears at a new destination in the moment
  between the final check and the rename would be replaced. With the XDG
  desktop portal's dialog (not used: rfd's GTK 3 backend) the confirmation
  was not checked. Quitting during a download was tested end to end only on
  Linux (dashboard-only mode, window closed); on Windows only through the
  Rust tests (the tray's Quit is not driven by the download test).
- **A key restricted to a file name prefix cannot browse the bucket's root.**
  Backblaze refuses the root listing for such a key (`AccessDenied`, "not
  entitled", 2026-11-11), and the browser always starts at the root, so it
  shows Access denied and the key's own prefix cannot be reached through the
  UI. There is no start-folder setting.
- **Browsing is verified at one real provider, Backblaze B2, with a
  temporary dataset; never at Hetzner.** On 2026-11-11 a 1,040-object
  dataset (nested and marker-only folders, Unicode, `+ & ? # %`, `//`, `..`,
  1,010 entries in one folder) was listed correctly through the Rust code
  and through the UI on WSL2 (Xvfb, not a real desktop) and Windows 11, and
  deleted again
  ([status.md](status.md#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11)).
  Only two pages (1,010 entries) were seen through the UI. The dataset test
  uses billed listing transactions: about 250 per UI run on 2026-11-11, when
  the account's daily cap was reached and one run's 1,040 test objects could
  only be cleaned up the next day, after the cap reset
  ([status.md](status.md#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11)).
  Since 2026-11-12 its provider walk of `many/` uses pages of 336 instead of
  7, about 95 listings per UI run (estimate); the cap can still be reached by
  many runs in one day, and the new page size has not yet run at Backblaze
  ([status.md](status.md#real-windows-desktop-and-fewer-backblaze-listings-2026-11-12)).
- **Backblaze accepts a continuation token it never issued** and answers with
  the folder's first page instead of an error (made-up token, 2026-11-11;
  an expired real token was not tried). sotyware sends only tokens Backblaze
  issued, and "Load more" drops rows it already shows; if Backblaze ever
  restarted a listing this way, "Load more" would add nothing and could
  keep offering more.
- **Browser limits:** no total item count (S3 gives none); folders first,
  then files, each in the provider's order (binary key order), no sorting or
  filtering; no cache, so every folder opened, refresh and "Load more" is one
  billed `ListObjectsV2` (a Backblaze class C transaction); at most 256 unused
  "Load more" cursors are kept (an older one gives "out of date", and the page
  refreshes); each loaded page stays in the page's memory until the folder is
  left, so "Load more" through a very large folder grows the table without
  limit (1,005 rows tested); a slow listing can take up to the client's
  timeouts (10 s connect, 60 s read, 3 attempts) and cannot be cancelled,
  only left; object versions, hidden files and delete markers are not shown;
  keys that are not valid XML text cannot be listed (the SDK does not request
  `encoding-type=url`).
- **The connection check is verified at Backblaze by one opt-in run and one
  UI run each on Linux (GNOME Keyring) and Windows (Credential Manager), and
  never at Hetzner.** On 2026-11-11 (WSL2) Backblaze
  answered `HeadBucket` with 200 for the bucket-restricted test key
  (connected, one request), 403 for a wrong secret or unknown key id (then
  the one-key listing, also 403: invalid credentials) and 404 for a bucket
  the key cannot use (**bucket unavailable**, also when the bucket exists
  but belongs to someone else: the page then says the bucket does not exist)
  ([status.md](status.md#cloud-storage-ui-end-to-end-and-backblaze-connection-check-2026-11-11)).
  The `ListObjectsV2` fallback has no prefix and was never reached at
  Backblaze with a valid key; a key restricted to a file name prefix might be
  refused it, so such a key could report **access denied** if Backblaze ever
  refused it the HEAD. A key that may neither `HeadBucket` nor list the
  bucket (for example write-only) reports **access denied**, although it
  could upload.
- **One bucket per account in the dashboard.** The backend allows several
  (or none, meaning any); the form edits exactly one, and saving an edited
  account replaces its bucket list with that one.
- **Check results are not kept.** They live in memory until sotyware quits
  or the account changes; after a restart every account is checked again
  once (ADR-046), so it reads "Checking connection…" for a moment. After a
  locked or unavailable credential store the other accounts stay "Not
  checked yet" until checked by hand. Accounts added or synced in later are
  not checked automatically.
- **The startup check can ask to unlock the keyring** on Linux (once) and
  the Keychain on macOS, also when sotyware starts hidden in the tray
  (ADR-046). Not seen on a real desktop.
- **One credential-store call at a time.** While an add, edit, removal or
  check waits for the credential store (for example an unlock prompt nobody
  answers), every other account call waits behind it: the window stays
  responsive (tested), but the Cloud Storage page shows "Loading…" or
  "Saving…" until the prompt is answered or fails. There is no cancel.
- **No key id is shown.** Showing it would mean reading the credential store
  (possibly an unlock prompt) just to list accounts; the page shows provider,
  bucket, region and endpoint only.
- **Secrets in the page's memory:** the typed key is held in the form's state
  only while the form is open and is cleared afterwards, but JavaScript
  strings cannot be wiped, and the Rust side does not zero its copy either
  (`String`); both are freed, not overwritten.
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
  answers. The dashboard's commands run them on the blocking pool, so the
  window is not blocked (shown with a Secret Service that never answers,
  under WSL2); an unlock prompt answered by a person on a real desktop was not
  tried.
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
- **Uploads (ADR-021, ADR-022):** tested against the in-process server
  only: in Rust, with mocked commands (Vitest), and through the release
  binary and the native GTK multi-file dialog on a **virtual display under
  WSL2** (`cloud-ui.sh upload`, 2026-11-12) and on the **native Windows 11
  desktop** with the native Windows multi-file dialog
  (`windows-cloud-ui.ps1 -Mode upload`, 2026-11-12). **At Backblaze**
  through the Rust batch path once on WSL2 and once on native Windows 11
  (2026-11-13, no UI or dialog,
  [WSL2](status.md#uploads-at-backblaze-2026-11-13),
  [Windows](status.md#uploads-at-backblaze-on-native-windows-11-2026-11-13)),
  and **through the native Windows 11 dashboard and multi-file dialog once**
  (2026-11-13, one batch of four files with one existing object;
  [status](status.md#dashboard-upload-at-backblaze-on-native-windows-11-rerun-passed-2026-11-13));
  then multipart (release settings), cancel, retry, leaving the browser
  and quit once each through the same dashboard
  ([status](status.md#upload-lifecycle-at-backblaze-on-native-windows-11-2026-11-13));
  needs-inspection and a refused abort only against the local server;
  hiding during a batch not at a provider; **never at
  Hetzner**, never on Windows 10, a real Linux desktop, Wayland or with
  the XDG portal's dialog. At most 100 files per batch, one batch at a time, the
  files one after another; a skipped file (existing object, duplicate name,
  invalid name) has to be renamed or moved by the user, nothing is
  overwritten or renamed automatically. The file is read twice (the first
  hashing pass, then while sending); the parts of one file are sent one
  after another, not in parallel; an interrupted upload starts again from
  the beginning (no resume).
- **On Linux a file whose name is not valid UTF-8 disappears from the
  selection.** The GTK dialog shows it ("(invalid encoding)") and it can be
  selected, but rfd 0.17.2 drops every path that is not valid UTF-8 from the
  dialog's answer (`get_results` in its GTK 3 backend), so it never reaches
  sotyware's `invalid-name` check: the panel lists the other files and says
  nothing about it. Seen 2026-11-12 on the virtual display. Not changed by
  the Windows patch (ADR-023), which leaves rfd's Linux code as it is.
  Windows differs (next item).
- **Windows dialog paths depend on a locally patched rfd.** NTFS allows
  file names with an unpaired surrogate (UTF-16 that is not valid
  Unicode). Upstream rfd 0.17.2 (and 0.18.0) decodes every Windows dialog
  answer with `String::from_utf16(..).unwrap()`, which ended sotyware with
  `0xC0000409` (seen 2026-11-12 in the upload dialog, and on 2026-11-13 in
  the save dialog with an unpatched build). Fixed since 2026-11-13 by a
  local copy of rfd that reads the path as `OsString` (ADR-023): such a
  file chosen for upload is skipped as "name cannot be an object key", a
  download is saved under exactly that name. The fix lasts only while
  Cargo uses the patch: a `tauri-plugin-dialog` update that requires
  another rfd version would bring the abort back (Cargo only warns that
  the patch was not used); `npm run check:rfd-patch` fails then, in
  `test:headless` and CI, but a build started without it (a plain
  `cargo build` or `npm run tauri build`) does not check
  ([`src-tauri/patches/`](../../src-tauri/patches/README.md#updating-or-removing-the-patch)). The upload panel shows such a name with U+FFFD in place of the
  surrogate (display only; the file is never sent). Builds print rfd's own
  nine upstream dead-code warnings, as Cargo does not silence a path
  dependency
  ([status](status.md#windows-dialog-paths-with-unpaired-surrogates-2026-11-13)).
- **The Windows upload test does not select files the way a person does.**
  It selects them in the dialog's file list through UI Automation or types
  their quoted names into File name; the dialog returns the same items
  either way, but mouse clicks and Ctrl+A in the list were not reproduced.
- **Upload conflicts are checked, not prevented.** The destination is
  checked by `HeadObject` before any data is sent (and before completing a
  multipart upload); another client that creates the same key in between is
  overwritten (or, on a versioned bucket, hidden behind the new version). No
  conditional write is sent to Backblaze, Hetzner or a generic service: none
  documents `If-None-Match`, and it was never verified at either provider.
  The mechanism exists (ADR-029) and is tested only against the local test
  server, which enforces the header as Amazon documents it. That server
  reads a write it refuses (412) to its end before answering; how a real
  provider answers a refusal while the body is still being sent was never
  tried (closing the connection under it loses the 412 on Windows).
  The checks and the verification need a key that may read objects as well
  as write them.
- **Incomplete multipart uploads can remain.** A failed or cancelled
  multipart upload is aborted; if the abort fails or does not answer within
  30 s, the error says that parts may remain, and they stay stored (and
  billed) until removed. A process killed or crashing during a multipart
  upload aborts nothing by itself; since ADR-030 the upload id is in the
  operation journal (written right after `CreateMultipartUpload`, before
  any part), so the recovery view offers to abort exactly that upload after
  the next start, and **Unfinished large uploads** lists every incomplete
  upload of a bucket (another device's too) for one-by-one aborts. A crash
  in the instant between `CreateMultipartUpload` and the journal write
  leaves an id only the bucket listing shows. Nothing is aborted
  automatically, and sotyware does not configure a lifecycle rule.
- **The upload panel shows one batch and forgets it.** Leaving the browser
  cancels a running batch and drops its result; there is no upload history.
  Its progress bar counts finished files, not bytes. Leftover multipart
  parts of a batch whose only failures are `incomplete-parts` or
  `needs-inspection` get no Try again (by design), so the page cannot ask
  for their cleanup; it happens on the next retry of that account or when
  sotyware quits. A file whose name is taken (`destination-exists`) has to
  be renamed by the user; there is no overwrite or "keep both".
- **Quitting during an upload can take up to 35 s** if the provider does
  not answer the multipart abort; normally it takes one request.
- **A file whose upload "needs inspection" stays as it is.** It is not
  retried or deleted; the user has to check the object in the bucket (or
  upload the file again under another name).
- **Upload stall detection sees the local socket, not the network.** Bytes
  in the operating system's send buffer count as sent, for progress and for
  the 60 s stall timeout. After the last byte is handed over, the response
  must arrive within 60 s including the time the buffer needs to drain; on
  loopback through the test relay about 2.6 MiB drained in 26 s at
  100 KiB/s, so with such a buffer a working link below about 44 KiB/s would
  time out at the end of each request. Real slow links usually keep smaller
  buffers (not measured).
- **Touching a file during its upload fails it.** A new modification time
  counts as a change (`FileChanged`) even if the content is the same. On
  file systems with coarse timestamps a change that keeps the size and the
  timestamp is caught only through the bytes the first pass had already
  read; a change to bytes it had not read yet is hashed and sent as the
  file now is, so the object is consistent but reflects the change.
- **After an upload's commit point a failure is not cleaned up.** If the
  provider may have stored the object but its verification fails (or the
  response is lost), the error says so (`NeedsInspection`) and the object is
  left as it is.
- **Deletes on versioned buckets:** `delete_object` without a version id only
  hides the object on Backblaze (always versioned) and on a Hetzner bucket with
  versioning; the data stays billed. Only the exact version returned by the
  upload is removed for good.
- **No retrying queue.** Transient errors are retried only by the SDK (3
  attempts per request); a failed transfer is not queued again.
- **Pricing is not in the code.** The cost figures in cloud-providers.md are
  estimates from list prices of 2026-11-11, with VAT for French consumers and
  card currency fees unconfirmed.
- **Test server limits:** the in-memory listing backend (browser tests) has
  only `ListObjectsV2` and `HeadBucket`, and its continuation tokens are the
  hex of the last entry, not opaque like a provider's. `s3s-fs` lists nothing below a second folder level
  (seen with restic's `data/xx/` packs, so `restic check` against it reports
  missing packs), stores no versions, and does not check SigV4 checksums of
  streamed bodies the way a provider might. Its errors are not the providers'
  errors.
- **Dependencies:** `aws-sdk-s3` is held at 1.102.0 by `rust-version = 1.90`;
  `s3s` 0.12 needs the dev-only `digest` release-candidate pin.

### Operation journal and recovery (ADR-030)

- **Recovery is evidence-based, not automatic.** After a crash, interrupted
  uploads and operations are listed and inspected at the provider; nothing
  is retried, finished or deleted without the user. Finishing an
  interrupted move means deleting the kept originals through a normal
  reviewed delete in the browser.
- **Granularity:** an operation's step results are written after each phase
  of each item; a crash in the middle of a phase leaves those steps as
  `planned`, and inspection resolves them from the provider's state.
  Inspection checks at most 500 steps per click.
- **An upload ended by a crash before its journal entry** (during the first
  hashing pass) left nothing remote, so nothing is lost by not recording it.
- **Only this device's journal:** another device's interrupted work is not
  known here (its incomplete multipart uploads appear in the bucket listing,
  marked as not recorded).
- **Local test servers only:** the journal and recovery view ran against
  s3s-fs and the in-memory versioned server, and through the frontend with
  mocked commands. No desktop driver run and no real provider yet.

### A part can land after the multipart abort

Found 2026-11-13 (native Windows 11 quit rehearsal with the request
recorder; reproduced on WSL2 by the opt-in probe
`probe_a_part_landing_after_the_abort` in `cloud/s3/upload/tests.rs`, 1 of
3 trials) and **fixed the same day for the common case**
([ADR-025](../architecture/decisions.md#adr-025-a-cancelled-part-that-was-handed-over-completely-is-awaited-before-the-abort)):
a cancelled part whose bytes were all handed to the connection is now
awaited (at most 5 s) before the abort; the probe then left 0 of 3. Local
server only; no real provider was asked.

- **What remains:** if that part gets no answer within 5 s (a very slow
  link or a provider that does not answer), the abort goes out anyway and
  the part can still be stored afterwards; sotyware logs it and still
  reports `NothingStored`. A cancel near the end of a part can take up to
  5 s longer. Seen once on native Windows 11 (2026-11-13, the first
  `-Scenario settle` run, whose test link carried only about 1 MiB/s): the
  wait ran into the bound, part 3 was dropped 5.05 s after the cancel, and
  its file was still written after the 204 abort (local server; removed
  only with the test server's folder). sotyware's log line for it was not
  captured: the Windows UI driver does not keep the app's stderr.
- **Real providers:** AWS documents that an in-progress part "might or
  might not succeed" after an abort, and that an upload may have to be
  aborted more than once and checked with ListParts
  ([AWS AbortMultipartUpload](https://docs.aws.amazon.com/AmazonS3/latest/API/API_AbortMultipartUpload.html)).
  Backblaze's S3 documentation says nothing about it
  ([B2 S3 AbortMultipartUpload](https://www.backblaze.com/apidocs/s3-abort-multipart-upload)).
  Ceph RGW (Hetzner Object Storage) has had abort and orphaned-part
  races ([Red Hat bug 2008835](https://bugzilla.redhat.com/show_bug.cgi?id=2008835)).
  Whether Backblaze or Hetzner keep, bill or list such a part is
  **unchecked**. A bucket lifecycle rule that aborts incomplete multipart
  uploads remains the safety net.

### Request recorder (test builds only, ADR-024)

- Never run against a real provider. Validated only against the
  in-process server (unit tests, the Linux virtual-display driver and the
  native Windows 11 driver; details in
  [status.md](status.md#request-recorder-2026-11-13)).
- Seen with it (not a recorder defect): a part that lands after the
  abort, see [below](#a-part-can-land-after-the-multipart-abort).
- The binary that records is not the shipped binary (one feature more);
  it proves what the app's code sends, not that a packaged build sends
  the same.
- `op-dropped` does not say whether the provider received or completed
  the request; a `gap`, a missing `end` line or an open operation makes
  the count unknown. A missing event never proves a request was not sent.
- A killed or crashed app writes no `end` line. On Windows in tray mode
  the only orderly exit is the tray Quit (real mouse), so a recording run
  of the Windows driver moves the mouse once.
- Connections, TLS, connection reuse, bytes on the wire and provider-side
  timings are not observable; timings are local.
- The hook semantics are those of the locked `aws-smithy-runtime` 1.10.0;
  an SDK update must pass `hook_assumptions.rs` again (it runs in every
  `cargo test`).
- The packaging guard reads the `tauri build` command line from the
  process tree; where that fails it checks every recorder binary at least
  as new as the normal one, which may refuse a normal bundle next to a
  newer recorder build (remove `src-tauri/target/recorder` then).

### File management (ADR-026)

- **No real provider has seen any of it.** Copy, move, rename, delete,
  restore, deleted items, search, previews, share links, several-item
  downloads, folder uploads and drops ran only against the in-memory
  versioned test backend and s3s-fs. Whether Backblaze accepts
  `x-amz-copy-source-if-match`, returns `x-amz-delete-marker`, lets a
  restricted key call `GetBucketVersioning` or `ListObjectVersions`, and
  how Hetzner behaves, is unverified.
- **Desktop coverage is partial** (2026-11-14, local servers only,
  [status](status.md#native-folder-dialogs-drag-and-drop-clipboard-and-final-audit-2026-11-14)):
  the Linux virtual display (`cloud-ui.sh files`: Xvfb + openbox, GTK 3.24
  dialogs, **not a real GNOME, KDE or XFCE desktop**) and the native
  Windows 11 25H2 desktop (`windows-cloud-ui.ps1 -Mode files`, UI
  Automation). Folder uploads and several-item downloads through the
  native folder dialogs were driven on both. Still not driven:
  - **drag and drop on Windows** (dropping from Explorer needs a real
    pointer drag on the user's desktop; not done) and on any real Linux
    desktop; on the virtual display a drop came from a small GTK program
    over XDND (`drag_source.py`), not from a file manager;
  - **the system clipboard:** Ctrl+C, Ctrl+X and Ctrl+V act on sotyware's
    own clipboard of items by design and never touch the system's; the
    only system-clipboard write is Copy link in Properties
    (`navigator.clipboard.writeText`), read back nowhere (no clipboard
    manager on Xvfb; not done on Windows to leave the user's clipboard
    alone). The keyboard shortcuts were driven with real keys on the
    virtual display only; Windows has component tests for them, no native
    key run;
  - cancelling a folder upload while a file is sent (the same batch code
    as Upload files…, whose cancel the `upload` phases drive);
  - Ctrl/Shift-click with the pointer on Linux (XTest modifiers are not
    seen by WebKitGTK on Xvfb; keyboard and Vitest only).
- **Empty folders are not uploaded.** A folder upload sends files only
  (S3 folders exist through their objects), so an empty local folder
  leaves no trace and a folder holding only empty folders is refused
  ("the folder holds no files").
- **A file refused with access denied is not retried** by Try again in an
  upload batch or Download the rest again: access, credential and
  connection errors stop the batch, and only files not started (or failed
  for a transient reason) are offered again. Fix the account, then upload
  or download again (existing objects and files are skipped or numbered).
- **An intermittent Windows driver timeout, not explained.** In one of
  five `windows-cloud-ui.ps1 -Mode files` runs (2026-11-14, run 3) a
  several-item download beside a local `one.txt` never started: the app
  planned it (its listing and `HeadObject` reached the test server), the
  driver chose the folder in the native dialog and the dialog closed, but
  no `GetObject` followed, no panel and no staging folder appeared, and the
  next batch started normally, so the dialog most likely returned no
  folder. The same run showed the dialog resetting its Folder box (fixed in
  the driver since). Not reproduced; runs 4 and 5 passed
  ([status](status.md#final-review-of-adr-026-and-adr-027-2026-11-14)).
- **One request of race.** Between the `HeadObject` that checks a source or
  destination and the `CopyObject` or `DeleteObject` that follows, another
  client can change the object (S3 offers no conditional delete at
  Backblaze). The copy itself is conditional on the planned ETag.
- **Copies above 5 GiB are refused** (no multipart copy); such items are
  left out of a plan with the reason.
- **A copy is verified by size, and by ETag only for plain-MD5 ETags**
  (single-part uploads); content is not re-read. Copies get a new date and
  version id and may get another storage class.
- **Delete markers only.** Deleting never removes a version permanently;
  on a versioned bucket hidden versions stay billed until a lifecycle rule
  or the provider's console removes them. On a bucket without versions a
  delete is final. Whether versions are kept is shown as "unknown" when the
  key may not ask. **On a bucket whose versioning is suspended** the
  provider's delete marker (version id `null`) replaces an object written
  while versioning was off or suspended: that object is gone for good and
  is counted so (`restorable_after_delete`); the review says only "may be
  gone for good", since which objects have a `null` version is known only
  per object, at delete time. Not tested against a suspended bucket (the
  test backend has no suspended state; the rule is unit-tested).
- **Objects added to a folder after its review** are not part of the
  operation; a folder move, rename or delete that finds them afterwards is
  reported `partial` ("source-not-empty"), not done (since the 2026-11-14
  audit). An object added under the destination meanwhile is a conflict
  per object (skipped, never overwritten).
- **Deleted items read at most 5,000 versions** per folder view; deeper
  folders must be opened. A file deleted in an unversioned bucket never
  appears there.
- **Search lists keys**, at most 10 pages (10,000 objects) per call, then
  "Search further"; it cannot search contents, and a whole-bucket name
  search reads every key (requests are billed by the provider).
- **Several-item downloads**: at most 10,000 files; one file after
  another; a killed process leaves its hidden staging folder
  (`.sotyware-download-<pid>-<n>`) in the chosen folder until the next start
  removes it from the journal's record (ADR-030); on a file system
  without hard links, a file appearing under the same name in the last
  instant could be replaced by the final rename (as for single downloads).
  No ZIP option. A key that is also a folder prefix (`a/b` next to
  `a/b/c`): the file is saved as `a/b (1)` (since the 2026-11-14 audit;
  before, that item failed). A folder that appears at the chosen name in
  the instant between the existence check and the rename could, on Linux,
  be replaced if it is empty (rename semantics).
- **Folder uploads and drops**: at most 1,000 files; empty folders are not
  created; symbolic links are skipped; a drop is kept 2 minutes.
- **Share links** contain the access key id and cannot be withdrawn before
  they expire except by removing the key at the provider.
- **Previews**: text up to 256 KiB, PNG/JPEG/GIF/WebP/BMP up to 10 MiB;
  no PDF, SVG or HTML preview.
- **Sorting and filtering** apply to the entries loaded (one page plus
  "Load more"), not to the whole folder.
- **Favorites and recent items** may point at something deleted meanwhile;
  opening it shows an empty folder or "the file no longer exists".
- **Test-server limitation:** s3s cannot copy keys containing `?` (it
  splits the copy source after decoding); the tests avoid `?` in copied
  keys.
- **The clipboard of "Cut"** is emptied when the move's review opens, also
  if the review is then cancelled (cut again to retry).
- **Desktop tests run a test-profile build** (ADR-027): its identifier,
  data and webview folders and credential namespace differ from a shipped
  build's; a packaged build's exact paths are not exercised by any driver.
  The drivers prove isolation through the app's acknowledgement and
  fingerprints of the user's folders; a component that ignores the folders
  it is given (WebView2, WebKitGTK) could still write elsewhere (not seen).
  On Windows from a coding agent, `%LOCALAPPDATA%` may be MSIX-virtualized
  (2026-11-14 incident,
  [status.md](status.md#test-isolation-incident-root-cause-and-fix-2026-11-14));
  the fingerprint then covers the virtual view.

## Backups (ADR-031)

- **Local test servers only.** Every backup, verification, retention and
  restore path ran against two in-process S3 servers (s3s-fs) in the Rust
  tests, and the page with mocked commands. No real provider, no desktop
  driver, no native Windows run yet.
- **Not encrypted by sotyware:** blobs and manifests are plain objects (so
  they stay restorable with standard tools); whoever can read the backup
  bucket can read the files.
- **One object at a time, through a spool file** as large as the largest
  object, in the app's local data folder; large first backups are slow.
- **Quick verification reads metadata only** (size, `sotyware-sha256`);
  same-size corruption at the provider is found only by "Verify content".
- **Change detection relies on listing ETags and modification times;** a
  provider whose listing has no ETags makes every run re-read every object
  (deduplication still avoids re-uploading; s3s-fs is such a server).
- **No cross-device locking:** two devices running the same job write two
  snapshots; retention protects unreferenced blobs younger than a day.
- **No desktop notification** when a scheduled backup fails; the Backups
  page shows the health. Schedules run only while sotyware runs.
- **Folder markers are not backed up** (keys ending in `/`), and empty
  folders are not restored.
- **Object Lock is only read and reported,** never configured; per-object
  retention is not set on uploads.
- **Limits:** 200,000 objects per job, 2,000 manifests per repository.

## Account service (ADR-032)

- **One free VM, no redundancy** (ADR-045): a Google Cloud e2-micro (1 GB
  of memory, 1 GB swap) in `us-east1`, so account data are stored in the
  US. No monitoring or alerting; if the VM stops, nobody can sign in or
  sync (local storage accounts, backups and folder syncs keep working).
- **Off-host backups** (ADR-047, running since 2026-11-17): hourly, so up
  to an hour of changes can be lost; losing the age private key makes
  every backup unreadable; deleted accounts stay in backups for up to 31
  days; the restore on a new VM is a manual procedure
  ([step 14](../development/server-setup.md#14-if-the-vm-is-lost)) never
  rehearsed on a scratch VM (only the decrypt and `restore` in WSL2); the
  restore check is monthly by the maintainer's reminder, not automatic.
- **The VM's address is ephemeral:** recreating the VM changes it, and the
  DNS record must then follow by hand (Brevo's authorised IPs too, if
  the blocking is turned on again).
- **The service's SMTP key works from any address:** Brevo's IP blocking
  is off since 2026-11-17 for the privacy address
  ([server.md](../development/server.md#the-privacy-address)), so a leaked
  key can send as sotyware.com until revoked.
- **Free-plan limits:** Brevo sends at most 300 mails a day; Google's free
  tier includes about 1 GB of outgoing traffic a month.
- **Mail delivery** through Brevo over STARTTLS with authentication was
  checked with `send-test-mail` from the VM on 2026-11-17; the retry and
  drop paths were tested only against an SMTP server in the test process.
- **Mail is at least once, not exactly once:** a mail being sent when the
  service stops is sent again at the next start; one not accepted within a
  day is dropped (logged by row id).
- **One SQLite connection, hashing under its lock:** roughly 20 sign-ins per
  second per instance.
- **Rate limits and the unknown-address lockout are in memory** and start
  over when the service restarts.
- **Old access tokens stay valid until they expire** (15 minutes) after a
  refresh; revocation (sign-out, device revoke, password change or reset,
  reuse detection) ends them at once.
- **A pending device sees the recovery wrap** (needed to unlock with the
  recovery key); the wrap is useless without the 256-bit recovery key.
- **Metadata is visible to the operator:** email, device names and
  platforms, sync times and vault size.
- **Retention is applied hourly** (ADR-048): a row can outlive its
  period by up to an hour, and purged rows stay in the backups until they
  age out (31 days). Expired enrollments are not purged; they end with
  their account. Tested only in process; never run on the VM.

## Accounts on the desktop (ADR-032)

- **Not usable in released versions yet:** `v0.1.0` and `v0.1.1` have no
  service configured, so their Account page says accounts are not
  available. Builds from `main` use `https://accounts.sotyware.com`
  (ADR-045); a test-profile build uses none unless
  `SOTYWARE_ACCOUNT_SERVER` names one, and development builds can point
  that variable at another service.
- **No background sync:** changes reach another computer when it syncs
  (sign-in, unlock, approval, key operations, "Sync now").
- **A fresh device cannot detect a rolled-back vault** (it has never seen a
  newer revision); existing devices refuse one.
- **Revoked devices keep what they decrypted;** rotate the keys and the
  storage keys at the providers after losing a computer.
- **Only storage accounts and backup jobs sync;** favorites, recent items
  and feature switches stay per device.
- **Tested with in-memory secret stores only;** the OS credential store
  path (`OsSecretStore`) shares the code of the storage credentials but
  was not run with account secrets on Windows or Linux.
- **Losing the recovery key and every enrolled device** makes the vault
  unrecoverable by design; storage data is unaffected.

## Updates (ADR-033, ADR-036, ADR-039)

- **Only one release exists:** `v0.1.0`, published on 2026-11-15 with the
  release key (its public half in `src-tauri/update-key.pub`, the private
  half and its password in the `release` environment). An installation
  finds it up to date; no update was ever offered or installed
  ([status](status.md#first-release-v010-2026-11-15)).
- **`v0.1.0` was published before the installation test** that the
  support scope names as the condition for Windows 11
  ([release.md](../development/release.md#support-scope)). One install on
  the maintainer's personal Windows 11 is reported working (which
  installer, the build and the update check not recorded;
  [status](status.md#v010-installed-by-the-maintainer-2026-11-15)); the
  other installer, upgrading, uninstalling and reinstalling were never
  tried. As an immutable release it cannot be changed: a problem found
  needs a new version.
- **Development and desktop-test builds check GitHub too:** a debug or
  test-profile build without `SOTYWARE_UPDATE_ENDPOINT` and
  `SOTYWARE_UPDATE_PUBKEY` uses the compiled-in key like a release build,
  so with `update-check` on (the default) it fetches `latest.json` from
  github.com 60 s after starting and then daily. The desktop drivers set
  neither variable; the answer shows only in the update panels, which no
  driver checks (the shell's notification appears only for an available
  or installed update), and without network it is a network error there.
- **`check-key` lets a stray character through:** Node decodes base64
  leniently, so a key line ending in a `%` copied from a terminal (zsh's
  mark for output without a final newline; 2026-11-15) passed
  `npm run check:update-key`, and would pass `release.yml`'s prepare. The
  Rust tests decode strictly and failed on it, so `ci.yml` and the
  release's own `ci` job stop such a key before anything is signed.
- **The signing key's protection depends on repository settings,** made
  on 2026-11-15: `release` admits only the tag `v*` and needs the owner's
  approval, and the `v*` tag rulesets let only an administrator create a
  tag and nobody move or delete one. They are settings, not code: nothing
  in the repository keeps them as they are. `release.yml` refuses a real
  release while the environment is missing or admits a branch, or while
  the key is a repository secret, but it cannot stop another workflow
  from naming an unprotected environment, only warns when no reviewer is
  set and does not check the rulesets
  ([release.md](../development/release.md#repository-settings)).
  `macos-signing` has to exist before any run of `release.yml`, a dry run
  included, or of `macos.yml` with `package` (it was created by hand on
  2026-11-15): their macOS package job would create it without
  protection, and `prepare` only notes that it is missing.
- **The Windows installers are not Authenticode-signed**; publishing them
  needs the explicit opt-in `SOTYWARE_ALLOW_UNSIGNED_WINDOWS`.
- **The Linux packages are built on `ubuntu-22.04`,** which GitHub removes
  on 2027-04-17 (brownouts from 2027-03-23); from then on releases fail
  until `release.yml` moves to `ubuntu-24.04`, which needs glibc 2.39
  instead of 2.35 ([release.md](../development/release.md#building-a-release)).
  `ci.yml`'s Linux jobs and `isolation-guard.yml` run there too (since
  2026-11-15), so they fail in the brownouts as well until they move with
  it.
- **A dry run cannot check the release key:** it signs with a throwaway
  key; only a real release's publish job (or the local check in
  [release.md](../development/release.md#update-signing-key)) proves that
  the secret matches `update-key.pub`.
- **A failure after the pre-release is published and before the
  promotion** leaves that pre-release visible on the releases page (not
  offered to installations); release immutability is on (since
  2026-11-15), so it cannot be repaired, only deleted, and its version is
  used up. An immutable pre-release can still be made the latest release
  (`v0.1.0` was, on 2026-11-15).
- **A failure after the promotion request** (the `/releases/latest`
  read-back or the endpoint check) leaves a full release that GitHub was
  asked to make the latest and that installations may already be
  offered. Its files passed every check before the promotion; the run's
  warning calls it the latest release without having confirmed that
  ([release.md](../development/release.md#publishing-a-release)).
- **The draft is deleted only while the runner lives:** the deletion is a
  step of the publish job, so a force-cancel (which skips `always()`
  steps) or a lost runner leaves the draft, and the next attempt stops at
  it until it is deleted by hand.
- **The publish job finds its release by tag name** (`gh release …
  vX.Y.Z`). `gh` asks for the published release of the tag and for a
  draft of it at once and takes the first answer; with two drafts of the
  tag it acts on the one GitHub's GraphQL `release(tagName:)` returns,
  without an error (seen with `gh` 2.104.0 against a local stand-in,
  2026-11-15). The job refuses to start while a release or draft of the
  tag exists, but counts a failed lookup as none, and nothing stops a
  second release of the tag being made during the run. A later step can
  then check, publish or promote the other release (a draft cannot become
  the latest; the read-back then fails), and the deletion can remove a
  draft the run did not make and keep its own. Only someone with write
  access can make a release (today only the owner; no workflow but this
  job has `contents: write`), and no unverified file can become the
  latest: every file is compared byte for byte with the signed ones
  before the promotion, and `latest.json` with its `.sig` after it.
  Addressing the release by its id from its creation on would close this
  ([status](status.md#release-rehearsal-readiness-2026-11-15)).
- **Updates installed once, on Windows only** (the maintainer's Windows
  11, 0.1.0 → 0.1.1 through Update now;
  [status](status.md#the-first-update-through-the-app-010-to-011)); otherwise download,
  verification and refusal are tested against local servers (one behaving
  like GitHub's redirects) with the plugin on Tauri's mock runtime, and the
  release commands were rehearsed locally with the real Tauri CLI and a
  throwaway key; starting the Windows installer, the in-place replacement
  on macOS and Linux and `request_restart` starting the new version are
  not tested; on Windows, the one real update ran the installer and
  restarted, with the maintainer's data kept (as reported, not inspected).
- **The first manifest a fresh installation sees sets its floor;** before
  that any validly signed, newer manifest is accepted. Manifests do not
  expire, so GitHub (or someone between it and the user, without the key)
  can withhold an update.
- **The manifest and its signature are two requests;** a release published
  between them is handled by fetching both once more, a second mismatch is
  reported as a signature failure.
- **Linux deb and rpm updates** run `dpkg -i`/`rpm -U` through `pkexec` or
  a graphical `sudo`, which needs a polkit agent on the desktop; never tried.
  The AppImage needs its file to be writable.
- **macOS updates** swap `sotyware.app` in place; in a folder the user may
  not write (`/Applications` for a standard user) the system asks for an
  administrator's password. Never tried.
- **No delta updates, no background download:** the whole package is
  downloaded when the user clicks Update.

## macOS (ADR-037)

- **Never run on a Mac a person used.** The macOS code compiles and is
  tested only on GitHub's hosted runners: `macos.yml`, on every pull
  request to and push to `main`, first ran at `d9cf561` (2026-11-15) and
  compiled cleanly on both architectures, with one unit test failing (a
  test-profile path check); at `ec42d41` everything passed there, the
  Keychain round trip included
  ([status](status.md#hosted-runs-at-ec42d41-macos-passes-2026-11-15));
  the macOS-only lines (`apple-native-keyring-store`
  calls, the menu bar wiring, `RunEvent::Reopen`) were never compiled on
  the development machine, which has no Mac and no cross toolchain.
- **No signing or notarization yet:** no Developer ID certificate exists.
  Unsigned builds are blocked by Gatekeeper when downloaded and are left
  out of releases.
- **Keychain prompts on unsigned builds:** the keychain ties an item's
  access to the binary's signature, so every rebuild of an unsigned or
  development build asks again.
- **The menu bar icon is the coloured app icon,** not a monochrome
  template image, so it does not follow the menu bar's light/dark tint.
- **Minimum macOS 10.15 is declared, not tested;** only the runners'
  versions (macOS 15) have run anything. Intel builds depend on GitHub's
  `macos-15-intel` image, its last Intel one, announced to be retired in
  2027; after that there is no hosted Intel Mac to build and test on.
- **File names from older HFS+ volumes** may be decomposed (NFD); object
  keys keep the local name's exact bytes, so such a name and its composed
  (NFC) spelling are different keys.

## Title bar and window placement (ADR-040)

- **Seen on Windows only through the desktop test.** Besides unit tests
  (geometry, chrome permissions, the title bar's controls with mocked
  IPC), `tests/desktop/windows.ps1` passed 130/0 on native Windows 11 25H2
  on 2026-11-16 (first placement, controls, double click, a real drag,
  the frame styles, resizing by the bottom-right corner and the top edge,
  close, placement after a restart;
  [status](status.md#release-011-2026-11-16)), on one monitor at the
  machine's own scaling. Not checked: a person's look
  ([manual check](../development/testing.md#manual-check-title-bar-and-window-placement-windows-11)),
  snapping by dragging to an edge or by Win+Arrow (only the window styles
  they use), other scalings, several monitors or a monitor that goes away,
  Windows 10. Linux keeps its native title bar; the hidden-then-placed
  startup ran there only on the virtual display.
- **No Snap Layouts flyout.** Hovering the page's Maximize button does not
  open Windows 11's snap layouts (only a native caption button can); Win+Z,
  dragging to the screen's top edge and Win+Arrow work.
- **Windows 10** shows a 1 px border around an undecorated window with a
  shadow (Windows 10 is not supported).
- **No system menu on right click** of the title bar (Alt+Space opens it).
- **Placement is saved when the window is hidden, loses focus or sotyware
  quits**, not while it is dragged: a crash or a power cut loses a move made
  since. A placement on a monitor that is gone gives the default place; a
  partly visible one is moved fully inside the monitor it overlaps most
  (also when the user put it partly off-screen on purpose).
- **Decorated windows (macOS, Linux):** the default size counts the frame
  only where the platform reports it before the window is first shown; on
  Linux the window can come out a title bar's height taller than half the
  work area. Wayland compositors ignore the position.

## Storage usage and estimates (ADR-041)

- **Measured, not reported:** a measurement lists the whole bucket (one
  request per 1,000 versions; free at both providers today, but it takes
  minutes for millions of objects, and a Backblaze transaction cap can stop
  it). It stops after 20 million entries ("at least"). It is a snapshot of
  that moment: nothing measures continuously.
- **What is not measured:** older versions on a service or key that cannot
  list versions (said in the measurement); the parts of more than 100
  unfinished uploads, or of uploads whose parts cannot be listed; Hetzner's
  billed metadata; anything outside the account's configured bucket.
- **Estimates, not bills:** no provider API gives billing data. Downloads
  are what the user enters; taxes, card fees, contracts, promotions and
  price changes after the price list's `checkedOn` date are not included;
  buckets are assumed to share one provider account per Backblaze region and
  one per Hetzner (both shown on the page). Prices are maintained by hand in
  `src-tauri/pricing/`; a generic S3 service has none.
- **Never run at a real provider:** measured only against sotyware's
  in-process S3 servers.
- **In `v0.1.1` a measurement never finishes on the page:** the
  measurement completes and is kept, but the page drops its last message
  and keeps showing the progress line and Cancel; opening the bucket again
  shows the result. Rust named one field of that message `save_error`
  where the page reads `saveError`. Fixed on `main` on 2026-11-16, after
  `v0.1.1`
  ([status](status.md#the-usage--costs-tab-in-the-linux-desktop-test-2026-11-16)).

## Snapshots of a bucket (ADR-041)

- **Backblaze's own snapshots cannot be managed:** Backblaze offers no API
  to create, list, download or delete them or to read their status; the tab
  only explains them. A `b2-snapshot-…` bucket can be browsed only as a
  storage account of its own, with a key that may read it.
- **Deleting a sotyware snapshot** removes its manifest at once and its
  unshared data only once that data is a day older than the oldest
  remaining snapshot; on a versioned destination the provider keeps the
  deleted versions until a lifecycle rule removes them. A deletion is not
  journaled (like retention); it cannot be cancelled once started. Never run
  at a real provider.

## Automatic sync (ADR-042)

- Events only: a window that is never focused again does not sync; a change
  made on one computer reaches another when that one starts, is focused or
  changes something. No service exists in release builds, so nothing syncs
  there yet.

## Folder sync (ADR-043)

- **Tested only headless, against test servers:** in-process test servers,
  on the WSL2 file system ([status](status.md#folder-sync-2026-11-16)) and
  on GitHub's Ubuntu, Windows and macOS runners
  ([runs](status.md#hosted-runs-at-d3a301c-folder-sync-passes-on-macos-2026-11-16));
  never at a real provider, on macOS or a real Linux desktop, or with a
  large bucket. In the app it ran only through the desktop tests against a
  local server, on the WSL2 virtual display and native Windows 11
  ([status](status.md#desktop-tests-for-folder-sync-sign-up-and-the-github-pages-2026-11-17)).
- **Every file is kept on the computer;** there are no online-only
  (on-demand) files, no selective sync of folders, no bandwidth limit and no
  exclusion patterns of the user's own (the fixed exclusions: `desktop.ini`,
  `Thumbs.db`, `.DS_Store`, Office `~$` and LibreOffice lock files,
  sotyware's temporary files).
- **One sync per bucket** (per storage account), of the whole bucket.
- **Remote changes arrive at the interval** (5 to 60 minutes; S3 has no
  change feed), or at once when made through sotyware on this computer, or
  on "Sync now". Each check lists the whole bucket: one request per 1,000
  objects (Backblaze bills listing calls above its daily free amount).
- **Races:** between the check of an object and the write that replaces or
  deletes it another client can still change it (as for every sotyware
  upload, ADR-021, ADR-029); a file changed during the instant between the
  last check and a download's move into place is replaced (milliseconds).
- **Unversioned buckets:** a file deleted or changed in the sync folder is
  deleted or replaced in the bucket for good (said before the sync starts).
- **Names:** an object whose name this system cannot hold (Windows: `< > :
  " \ | ? *`, trailing dots or spaces, reserved names; anywhere: control
  characters, over 255 bytes per name) is not synced, nor a file whose name
  cannot be a key. HFS+ (old macOS volumes) normalises
  Unicode names, which can make such a name look like a new file; APFS does
  not.
- **Case** ([how](../architecture/folder-sync.md#names-that-differ-only-in-case)):
  on a file system that ignores case (Windows, macOS, FAT and exFAT drives),
  of keys that differ only in case one is synced (the spelling here, else
  the synced one, else the first in byte order), the others and everything
  inside them stay in the bucket, listed as not synced. Renaming only the
  case of a file over 5 GiB uploads it again and leaves the old key in the
  bucket. Case is compared by Unicode lower case, which is close to but not
  exactly NTFS's or APFS's rules, and Unicode normalisation is not compared:
  a name that APFS takes as the same in another normalisation is not left
  out, and a file inside a folder so spelled lands in the folder here (then
  uploaded under its spelling here). In a folder Windows marks
  case-sensitive inside a sync folder that ignores case, two spellings that
  are both here are both synced, but a second spelling from the bucket is
  not brought down.
  Checked on WSL2 (ext4 with case ignored by force, and NTFS through
  `/mnt/c`) and by the tests on GitHub's Windows and macOS runners, not
  through the app on a desktop.
- **Links, junctions, devices, sockets and pipes** in the folder are not
  synced (reported), nor anything in the bucket under a link's name: those
  keys stay in the bucket, and a synced folder replaced by a link keeps its
  objects there until the link is gone (then the folder counts as
  deleted here). Checked with symbolic links on WSL2 and on GitHub's Ubuntu
  and macOS runners, not with Windows junctions.
- **A provider without ETags on `HeadObject`** (s3s-fs) cannot sync changed
  files: the replacement is refused and reported.
- **Moves** are recognised for files up to 5 GiB with the same content
  (here) or the same ETag and size (in the bucket); larger or changed ones
  are uploaded or downloaded again and the old one deleted.
- **After a crash** an interrupted sync upload is listed under Interrupted
  work like any upload (ADR-030); the sync redoes it, and the entry can be
  dismissed.
- **Watching:** on Linux each folder takes an inotify watch; a folder too
  large for the system's watch limit is synced without watching (changes
  found at each check). Network folders may report changes late or not at
  all.
- **The sync trash** is a hidden folder in the sync folder; it uses the
  sync folder's drive and is emptied after 30 days, not when the drive is
  full.

## Autoclicker (ADR-050)

- **Where it was tested** ([status](status.md#autoclicker-and-macros-2026-11-17)):
  the engines, the service and the page headless (mock backend and fake
  clock; Tauri's IPC mocks), on WSL2, on native Windows 11 (Rust only
  there) and on GitHub's hosted Ubuntu, Windows and both Macs
  ([hosted runs](status.md#hosted-runs-at-423605f-the-autoclicker-branch-passes-on-every-platform-2026-11-18));
  the X11 backend with real input on a private Xvfb display only,
  and the app through its page with real input on the private display of
  `cloud-ui.sh autoclicker`; the Windows backend unit-tested on native
  Windows 11 and run with real input, into a window of its own, by the
  opt-in desktop test (`automation::windows::desktop`) on the
  maintainer's Windows 11 only
  ([status](status.md#the-autoclickers-windows-backend-with-real-input-2026-11-18)).
  The page has **never been driven on Windows** (its driver,
  `tests/desktop/windows-autoclicker.ps1`, is written but has not run);
  never on a real Linux desktop, macOS or Windows 10. Input to a program
  running as administrator is not tested anywhere (it needs an elevated
  program; Windows' rule, below).
- **Platforms:** Windows and Linux on X11 only. **Wayland** sessions get
  nothing (the system lets no application send or record other programs'
  input; a portal-based backend would be a new decision); **macOS** has no
  backend (CGEvent and an event tap need Accessibility permission and a
  signed app). On those systems macros and profiles can still be edited,
  imported and exported, and the feature fails to start with the reason.
- **Windows' own rules:** input does not reach programs running as
  administrator unless sotyware also runs as administrator (UIPI;
  `SendInput` reports success anyway, so sotyware cannot tell), nor UAC
  prompts, the lock screen or other secure desktops; recording does not
  see input to those either. Games with anti-cheat may ignore or refuse
  injected input.
- **Timing** depends on the system's timer: on Windows a click lands within
  about a millisecond of its time (high-resolution sleeps), but the system
  may coalesce or delay input under load; rates above about 100 clicks per
  second are often dropped by the receiving program. Recorded times have
  millisecond precision.
- **Positions** are physical pixels of the virtual desktop. Absolute
  positions adapt to changed monitors by role and relative place (with
  "Adapt to changed screens"), not by what is on them; a window-relative
  macro needs the window it was recorded with to be active at the start.
  Points between monitors of different sizes are moved onto the nearest
  monitor.
- **Keys** are physical keys (scan codes): a macro recorded on one layout
  types what those keys type on the layout in use when it plays. Text
  cannot be typed as text (no Unicode injection); keys outside the table
  (some media and OEM keys) are left out of recordings and counted.
  Shortcuts that would take a typing key alone are refused; F12 alone is
  Windows' debugger key.
- **The pointer is moved for clicks** at a position (it stays where the
  last click left it); clicks "where the pointer is" never move it.
- **The fail-safe corners** only work when the pointer can reach a corner
  between two actions (fast clicking at a fixed position moves it back
  every interval), and not at the exact spot the run itself clicks or
  moves to (a position in a corner is clicked; pushing the pointer there
  is then indistinguishable from the run's own move). A single push in the
  instant between the last check and the run's next move is undone by
  that move; a hand keeps pushing, a script moving the pointer once may
  not. The emergency stop shortcut and the page's Stop always work.
- **While something runs, the emergency stop's key with any other
  modifier** (Shift+F8, Ctrl+Alt+F8, …) is sotyware's, so that it works
  while a macro holds a modifier: other programs lose those combinations
  until the run ends. A combination another program had first stays that
  program's, and pressing it then does not stop the run (F8 alone, the
  page's Stop and the corners still do).
- **A process killed from outside** (Task Manager, a crash that is not a
  Rust panic, power loss) cannot release a key or button a macro held;
  pressing and releasing that key or button releases it. Nothing restarts
  afterwards.
- **Recordings hold typed text in plain text** in `automation.sqlite3` (not
  encrypted at rest) and in exported files; they are not part of sync or
  backups.
- **One session at a time**; no scheduling (start at a time), no
  conditions on screen content (pixel colours, images), no variables or
  loops other than repetitions and one repeated section, no text typing
  actions.
- **Not implemented** although asked for "where feasible": starting
  sotyware at login and starting it minimised (an app-wide lifecycle
  change, listed under the roadmap's ideas), relative pointer movement
  for games that read raw mouse deltas (moves are absolute).

## Dashboard (ADR-035)

- **The theme is per webview profile:** it lives in the webview's
  `localStorage`; clearing the webview data, a test profile or a new
  computer starts from the system theme. It is not synced.
- **On Linux a theme just chosen reaches the disk a moment later, through
  WebKit, not sotyware** (WebKitGTK 2.52.6, read in its source and
  measured on WSL2, 2026-11-18): WebKit's network process commits a
  `localStorage` write 0.5 s after it is made, or, when sotyware quits
  first, as soon as it sees sotyware gone, and then exits (5 to 32 ms
  after sotyware on WSL2). A quit right after the choice therefore keeps
  it. But a sotyware started again before that late commit shows the
  previous theme; and when the webview had nothing else stored, WebKit
  deletes the store it read as empty when that sotyware quits, so the
  choice is lost for good. A choice whose helper is killed before it
  commits is lost as well (a session that ends at that moment, for
  example; not tested). It never happened on its own on WSL2 (14
  relaunches started at once after a quit, 9 of them with the CPUs
  loaded), only when the helper was held back; the desktop test's one
  failure on a GitHub runner, a relaunch 0.2 s after the quit, is the
  same failure as when the helper's commit is held back 2 s
  ([status](status.md#the-theme-check-after-a-quick-quit-and-relaunch-2026-11-18)).
  The downloads assumption on Usage & costs is kept the same way. Not
  examined: WebView2 (Windows) and WKWebView (macOS).
- **WebKitGTK and native controls:** checkboxes and radio buttons keep the
  GTK theme's look in dark mode on Linux.
- **Seen only on WebKitGTK:** the redesign was looked at through the Linux
  engine (screenshots of a preview with mocked data, and the real app on a
  virtual display through AT-SPI). It has not been seen in WebView2 on
  Windows or WKWebView on macOS, at another scaling, or with a screen
  reader.
- **The Windows desktop drivers were updated, not run** after the redesign
  (`windows.ps1`, `windows-cloud-ui.ps1`).

## General

- **No autostart** on login.
- **Logging is `eprintln!` only.** No log files, levels or subscriber; output is
  lost when started from a desktop launcher.
- **`npm audit`** reports a low-severity advisory in `deepmerge`
  (GHSA-gcqx-w48m-w2h4), a build-time dependency of
  `@sveltejs/vite-plugin-svelte`, with no fix available (re-checked
  2026-11-15). It is not shipped to users.
- **`cargo audit`** (2026-11-15) reports advisories the app cannot fix
  today: `quick-xml` in the test-only S3 server, `lru` (unsound, through
  `aws-sdk-s3`; a fix needs a newer SDK than Rust 1.90 allows), `glib`
  (Linux GTK 3) and five unmaintained crates; none reachable with hostile
  input in sotyware's use
  ([threat model](../architecture/threat-model.md#dependency-review-2026-11-15)).
- **The databases are not encrypted at rest;** the data folder is private to
  the user (Linux `0700`), not to the process.

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
- **The locked-keyring checks of `cloud-ui.sh local` decline the unlock
  prompt; none accepts it.** With a system-installed gnome-keyring the driver
  cancels `gcr-prompter`'s dialog through AT-SPI; unlocking through the prompt
  (typing the password) is not tested. Since this was adapted, the
  no-prompter path (an unpacked gnome-keyring) has not been run again
  ([status.md](status.md#cloud-storage-ui-test-locked-keyring-with-a-system-prompter-2026-11-11)).
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
- **The desktop workflows are still manual.** They were made manual while
  the repository was private and CI time was a $12 monthly allowance; since
  2026-11-15 the repository is public and standard runners are free
  ([runners and cost](../development/testing.md#runners-and-cost)), but
  only the headless jobs (`ci.yml`, `macos.yml`) run automatically.
- **Automatic CI never starts sotyware.** Window, tray, single-instance and close
  behaviour are covered there only by unit tests with doubles; the desktop
  workflows are manual and may lag behind `main`.
- **Not in CI at all:** the real-desktop Windows test (needs an unlocked desktop
  with Explorer and the mouse), the manual checklist, Windows 10, real Linux
  desktops, installing the installers. Building and unpacking them is the
  manual `release-build.yml` (passed twice, runs 43643326406 and
  43646814979, 2026-11-14); it does not inspect the installers' icons.
- **The test-profile and recorder tests run in CI on Linux only** (one
  recorder build, which includes test-profile; passed on GitHub
  2026-11-14). On Windows CI only type-checks the test-profile build.
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

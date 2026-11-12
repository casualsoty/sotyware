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

## Cloud storage

Design: [cloud-storage.md](../architecture/cloud-storage.md); providers:
[cloud-providers.md](../architecture/cloud-providers.md).

- **Accounts, browsing, single-file downloads and upload batches only.** The
  dashboard manages accounts, checks their connection, lists their buckets
  one folder at a time, downloads one file at a time and uploads up to 100
  picked files into the folder shown; it cannot overwrite, upload folders
  or by drag and drop, rename, move, delete or create anything, download
  folders or several files at once, and has no transfer queue, upload or
  download history, backups or sync
  ([cloud-storage.md](../architecture/cloud-storage.md#downloading-a-file)).
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
- **A killed process leaves its temporary download file.** Quitting cancels
  and removes it (waiting up to 5 s), but a crash, power loss or `kill -9`
  leaves `.<name>.<pid>-<n>.sotyware-partial` next to the destination (hidden
  on Linux, visible on Windows); sotyware does not look for such files later.
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
  or the account changes; after a restart every account is "Not checked yet".
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
  WSL2** (`cloud-ui.sh upload`, 2026-11-12). **Never at Backblaze or
  Hetzner**, never on native Windows (no upload mode in
  `windows-cloud-ui.ps1`), never on a real Linux desktop, Wayland or with
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
  nothing about it. Seen 2026-11-12 on the virtual display; Windows not
  checked.
- **Upload conflicts are checked, not prevented.** The destination is
  checked by `HeadObject` before any data is sent (and before completing a
  multipart upload); another client that creates the same key in between is
  overwritten (or, on a versioned bucket, hidden behind the new version). No
  conditional write is sent: Backblaze does not document `If-None-Match`.
  The checks and the verification need a key that may read objects as well
  as write them.
- **Incomplete multipart uploads can remain.** A failed or cancelled
  multipart upload is aborted; if the abort fails or does not answer within
  30 s, the error says that parts may remain, and they stay stored (and
  billed) until a bucket lifecycle rule removes them. A process killed or
  crashing during a multipart upload aborts nothing. Upload batches keep
  the ids whose abort failed **in memory only** and abort them again on a
  retry of that account's batch and when quitting (at most 35 s); after
  sotyware ends, or if the provider still refuses, they are forgotten.
  sotyware does not list old multipart uploads at the provider and does
  not configure a lifecycle rule.
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

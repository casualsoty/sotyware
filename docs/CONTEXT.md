# sotyware: session context

Read this at the start of every task. It is a summary; the linked documents hold
the details and win if anything here disagrees. Rules for agents:
[AGENTS.md](../AGENTS.md).

## What it is

A cross-platform (Windows 10/11, Linux) desktop app that runs in the background,
lives in the system tray and opens a dashboard window on demand. On Linux
without a tray host it runs as a plain window (dashboard-only mode). Built with
Tauri 2 (Rust) and Svelte 5 + TypeScript. Version 0.1.0.

## What works today

The desktop runtime, the foundation for built-in features, cloud storage
account management and read-only bucket browsing; **no file transfers,
backups or other product features exist**.

- Two operating modes, chosen once at startup:
  - **Tray mode**: tray icon with "Open Dashboard" and "Quit"; the dashboard
    (`main`) has a "Hide dashboard" button; close (✕) hides it; only tray "Quit"
    ends the process.
  - **Dashboard-only mode** (Linux without a session bus, StatusNotifierWatcher
    or StatusNotifierHost): no tray, a note instead of the Hide button, hiding
    is not permitted, close (✕) ends the process (exit 0).
- Single instance in both modes: a second launch shows the existing dashboard
  and exits 0.
- Any other tray creation error: exit 1.
- Built-in features (both modes): native Rust modules (`Feature` trait) listed
  in `features::builtin()` and run by a `FeatureRegistry`. Enabled ones start in
  `setup`, and running ones stop on exit. A failing feature becomes `failed`
  without affecting the others. The dashboard lists them and has an enable
  checkbox per feature. The only feature is `example` (does nothing visible).
  [features.md](architecture/features.md).
- Feature preferences (both modes): each user switch is saved in
  `preferences.sqlite3` in Tauri's `app_local_data_dir()`
  (`~/.local/share/com.sotyware.desktop/`, `%LOCALAPPDATA%\com.sotyware.desktop\`)
  and survives restarts; without a stored choice a feature uses
  `enabled_by_default`. Loaded once before the event loop; saved by
  `set_feature_enabled` on Tauri's thread pool. One STRICT table, version in
  `PRAGMA user_version`, migrations in one transaction. A corrupt, newer or
  unopenable database is left untouched: defaults are used and the dashboard
  shows the error; a failed save keeps the switch for the session and is shown.
  [features.md](architecture/features.md#persistence), ADR-015.

- Cloud Storage section (both modes; the dashboard's second navigation link,
  [ADR-018](architecture/decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)):
  list, add, edit, check and remove storage accounts at Backblaze B2, Hetzner
  or any S3-compatible service; one bucket per account; several accounts at
  one provider. Keys go from two password inputs straight to Rust and into
  the OS credential store (Windows Credential Manager, Linux Secret Service
  via `keyring-core`; no fallback, so Linux without a Secret Service cannot
  store any); no command returns them; the form clears them. Account settings
  in `cloud-accounts.sqlite3` (separate from the preferences). Editing without
  a new key never touches the credential store; a new key replaces the old one
  only once stored; removal asks first, never touches remote data, and keeps
  the account if its key cannot be deleted. "Check connection": `HeadBucket`
  on the account's bucket, a one-key `ListObjectsV2` only after a refused
  HEAD, never `ListBuckets` (a bucket-restricted Backblaze key is refused
  that); statuses connected / invalid credentials / access denied / bucket
  unavailable / network / credential store unavailable or locked / credentials
  missing / configuration / other. Credential-store and SQLite calls run in
  `spawn_blocking`. **Open bucket** (read-only browser, `CloudBrowser.svelte`,
  [ADR-019](architecture/decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust)):
  the account's configured bucket one folder level and one page (1,000
  entries) at a time through `list_cloud_folder` (`ListObjectsV2`, delimiter
  `/`; never recursive, never `ListBuckets`): folders, files, sizes, dates,
  breadcrumbs, Up, Refresh, Load more (no total), back to the accounts.
  Folder prefixes are passed back exactly as returned (never joined from
  names), marker objects hidden; the continuation token stays in Rust behind
  a cursor tied to account, bucket and prefix; stale answers are dropped;
  structured errors (missing key → Edit account). A key limited to a file
  name prefix is refused the root at Backblaze, so it cannot browse. Backend underneath: provider profiles as data, one S3
  adapter on `aws-sdk-s3` with end-to-end SHA-256, multipart and a transfer
  queue per account; uploads and downloads are **not reachable from the
  app**. Recommended first provider: Backblaze B2, EU Central.
  [cloud-storage.md](architecture/cloud-storage.md#dashboard),
  [cloud-providers.md](architecture/cloud-providers.md), ADR-016, ADR-017.

## Repository at a glance

```
src/                 Svelte frontend (main.ts, App.svelte, Features.svelte, features.ts, CloudStorage.svelte, CloudAccountForm.svelte, CloudBrowser.svelte, cloud.ts, cloudMessages.ts)
src-tauri/src/       main.rs → lib.rs (builder, operating mode, lifecycle, single instance, operating_mode command) + tray.rs + storage.rs (SQLite)
src-tauri/src/features/  built-in features: mod.rs (trait, builtin list, commands), registry.rs, preferences.rs (stored choices), example.rs
src-tauri/src/cloud/     cloud storage: provider.rs, account.rs, credentials.rs, keyring.rs (OS credential store), s3.rs, dashboard.rs (the six account commands), browser.rs (list_cloud_folder); tests.rs, browser/tests.rs + test_server.rs (in-process S3), integration.rs (opt-in)
src-tauri/           tauri.conf.json, capabilities/default.json, Cargo.toml, icons/
src/*.test.ts        Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
.github/workflows/   CI: ci.yml (automatic, headless), desktop-*.yml (manual)
tests/desktop/       Desktop tests of the release binary (run.sh: Linux/X11 simulated; virtual.sh: same on Xvfb + openbox; windows.ps1: real Windows desktop; windows-isolated.ps1: Windows private desktop; opt-in Cloud Storage end-to-end incl. browsing: cloud-ui.sh, windows-cloud-ui.ps1)
docs/                architecture/, development/, status/
```

Full map: [repository-map.md](architecture/repository-map.md).

## Entry points and boundaries

- Startup: `src-tauri/src/main.rs` → `sotyware_lib::run()` in `src-tauri/src/lib.rs`.
- The Linux tray backend is chosen in `src-tauri/Cargo.toml` (Linux-only
  `tray-icon` with feature `ksni`, ADR-011), not in code.
- Showing the dashboard always goes through `show_dashboard()` in `lib.rs`; the
  window is never created at runtime, and destroyed only when the process ends.
- The mode is chosen in `select_mode()` (`lib.rs`) from the tray result. Only
  `tray::unavailable_reason()` (Linux, typed `ksni`/`zbus` errors) may select
  dashboard-only mode; every other error stays fatal. No retries or polling.
- The frontend may call the commands `operating_mode`, `list_features`,
  `set_feature_enabled`, `list_cloud_providers`, `list_cloud_accounts`,
  `add_cloud_account`, `update_cloud_account`, `remove_cloud_account`,
  `check_cloud_account` and `list_cloud_folder` (one `allow-<command>`
  permission each) and, in tray
  mode only, `getCurrentWindow().hide()` (`core:window:allow-hide`, granted at
  runtime). No other custom command, no custom events. No command returns a
  stored credential.
- A new built-in feature is a module in `src-tauri/src/features/` plus one line
  in `builtin()`. Lifecycle code calls only `start_all`/`stop_all`.
  ([how to add one](architecture/features.md#adding-a-built-in-feature))
- The registry never touches storage: `features::preferences` holds the policy
  behind the `PreferenceStore` trait; only `storage.rs` uses SQLite. Schema
  changes are appended to `storage::MIGRATIONS`, never edited. Feature IDs are
  database keys: never change a released one.
- The single-instance plugin must stay the first plugin registered.

Details: [overview.md](architecture/overview.md),
[lifecycle.md](architecture/lifecycle.md),
[features.md](architecture/features.md),
[single-instance.md](architecture/single-instance.md),
[decisions.md](architecture/decisions.md).

## Commands

```bash
npm ci
npm run tauri dev
npm run check && npm run build
npm run test:headless                      # frontend + Rust unit tests, no display needed
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run tauri build -- --no-bundle && tests/desktop/run.sh   # or: npm run test:desktop:virtual
npm run test:desktop:windows               # Windows, real desktop, moves the mouse
npm run test:desktop:windows:isolated      # Windows, private desktop, unattended (no tray)
```

**WSL:** non-interactive shells may pick up Windows `npm` from `/mnt/c` and not find
`node`/`cargo`. Fix `PATH` first: [setup.md](development/setup.md#wsl2-notes).
More: [workflow.md](development/workflow.md), [testing.md](development/testing.md).

## Tests

- Cloud Storage bucket browsing (2026-11-11, uncommitted tree on `ad014e7`):
  WSL2 `cargo test` 208 passed, 8 ignored (19 new), frontend 81 (18 new);
  Windows 11 199 + 81. Against the local listing server through the release
  binary's UI: Linux `cloud-ui.sh browse` 17/17 (4 runs), Windows
  `windows-cloud-ui.ps1 -Mode browse` 29/29 (3 runs). **Real Backblaze,
  read-only** (`backblaze_b2_folder_listing`, WSL2 and Windows, once each;
  `cloud-ui.sh b2` 21/21 and `-Mode b2` 25/25, once each): the
  prefix-restricted test key is refused the root (Access denied), its prefix
  is empty; bucket unchanged. Not seen at a real provider: non-empty folders,
  pagination. Regression: virtual display 119/0/0 (3), Windows private
  desktop 56/0 (3), real desktop 103/0 (1), `cloud-ui.sh hang` 8/8 (3),
  Windows `local` 19/19 (3); Linux `cloud-ui.sh local` 14/2, 3 runs: its two
  locked-keyring checks fail also on `ad014e7` since gnome-keyring was
  installed system-wide on the machine (test problem).
  [status.md](status/status.md#cloud-storage-bucket-browsing-2026-11-11).
- Cloud Storage UI end-to-end and Backblaze connection check (2026-11-11,
  uncommitted tree on `6af454d`): new opt-in tests.
  `backblaze_b2_connection_check` (records the real HTTP requests, fingerprints
  the bucket) **passed once against Backblaze** (run by the user, WSL2):
  restricted key → `HeadBucket` 200, connected; wrong secret / unknown key →
  invalid credentials; another bucket → 404, bucket unavailable; no
  `ListBuckets`, nothing created. `cloud-ui.sh b2` **passed 19/19 against
  Backblaze through the Linux UI** (user, once: add, Connected, rename,
  wrong key → Invalid credentials, real key → Connected, other bucket →
  Bucket unavailable, remove; bucket unchanged), and `windows-cloud-ui.ps1
  -Mode b2` **23/23 with the real Credential Manager and Backblaze** (user,
  once).
  Without Backblaze: `tests/desktop/cloud-ui.sh` (Linux, real GNOME Keyring; `local`
  14/14 and `hang` 8/8, 3 runs) and `tests/desktop/windows-cloud-ui.ps1`
  (native Windows 11, **real Credential Manager through the UI**: add, check →
  Network error, rename, replace key, remove; 17/17, 3 runs). WSL2 `cargo test`
  189 passed, 6 ignored; Windows 180; virtual display 119/0/0, Windows private
  desktop 56/0 and real desktop 103/0, 3 runs each.
  [status.md](status/status.md#cloud-storage-ui-end-to-end-and-backblaze-connection-check-2026-11-11).
- Cloud Storage dashboard (2026-11-11, uncommitted tree on `c022b4e`): WSL2
  `cargo test` 188 passed, 4 ignored (28 new: commands, error kinds, no
  secrets, connection checks against the in-process S3 server with simulated
  bucket-restricted keys, credential store waiting without blocking the async
  thread), frontend 63 (25 new), virtual display **119/0/0, 3 runs**. A
  one-off end-to-end run of the release binary through its UI (AT-SPI + XTest)
  against a **real GNOME Keyring on a private D-Bus session** passed 14/14
  three times (add, check, rename, key replacement, removal, locked keyring),
  and against a Secret Service that never answers 7/7 (window and other
  commands stay responsive). Native Windows 11: see
  [status.md](status/status.md#cloud-storage-dashboard-2026-11-11). The
  connection check was **not** run against Backblaze or Hetzner. Counts below
  are from before this change.
- OS credential store (2026-11-11, uncommitted tree on `de2e7f3`): `cargo
  test` 160 on WSL2 (151 on native Windows 11), 4 ignored; the opt-in
  `cloud::keyring::os_backend::round_trip` passed **against the real Windows
  Credential Manager** (3 runs, entries checked with `cmdkey`, none left) and
  **against a real GNOME Keyring 46.1 on a private D-Bus session under WSL2**
  (3 runs, none left); "no Secret Service" reported as `Unavailable` in three
  real WSL2 conditions; a locked keyring as `AccessDenied`. Not run on a real
  Linux desktop session. Regression: virtual display 115/0/0 and Windows
  private desktop 56/0, 3 runs each.
  [status.md](status/status.md#os-credential-store-2026-11-11). The counts
  below are from before this change.
- `cargo test`: 140 unit tests (2026-11-11, cloud storage proof of concept:
  passes on WSL2 and on native Windows 11, 132 there; virtual display 115/0/0
  and Windows private desktop 56/0, 3 runs each, unchanged). 51 cover cloud
  storage: profiles, account validation and persistence, credentials,
  upload planning, error mapping, and 14 run against an S3 server inside the
  test process (round trips with SHA-256, multipart, bad keys, network
  failure, timeout, corruption, two accounts at once, separate transfer
  queues). 2 more are opt-in, `#[ignore]`d, against real Backblaze/Hetzner
  accounts: the Backblaze one **passed against a real B2 bucket**
  (`eu-central-003`, 2026-11-11, 3 runs each on WSL2 and native Windows 11;
  [status.md](status/status.md#backblaze-b2-against-the-real-provider-2026-11-11)),
  the Hetzner one was never run (no account). The other 89: 31 cover the lifecycle, mode and tray
  decisions (mode selection from the real `ksni`/`zbus` error types, close →
  hide or quit per mode, hide granted only in tray mode, implicit exit blocked,
  unexpected tray failure → exit 1, second launch, tray dispatch, config
  labels). 28 cover the feature registry (registration, duplicate IDs,
  lifecycle table, start/stop failures, isolation, shutdown order, concurrent
  requests, JSON shapes). 30 cover feature preferences: 13 the policy with an
  in-memory store, 17 the SQLite store on real files (migrations, corrupt and
  newer databases, read-only file, concurrent connections, a save blocked past
  the busy timeout, restarts with the
  built-in features). No display needed. Frontend: 38 Vitest tests.
- Persistent feature preferences (2026-11-11, `93f7667`; drivers `7696d44`,
  `a7bf8f3`): WSL2 headless 88 Rust + 38 frontend; `npm run
  test:desktop:virtual` **115/0/0 in 3 runs** (19 new checks: disable → Quit →
  relaunch → still disabled, enable → Quit → relaunch → running, database rows
  checked; corrupt database and a folder in its place shown and left
  untouched). Native Windows 11 25H2: headless 80 Rust + 38 frontend, fmt,
  clippy, release build; `windows.ps1` **99/0 in 3 runs** (the same restart
  sequence; the old "Running again (not persisted)" check now expects
  Disabled), plus one run that moved a planted database aside and restored it
  byte-identical. Counts below are from before this change.
- Built-in features (2026-11-11, WSL2, `e30ac7b`/`a9fb338`): headless 58 Rust +
  29 frontend tests, and `npm run test:desktop:virtual` 96/0/0 in 3 runs (the
  4 new checks switch the example feature through AT-SPI).
- Built-in features, native Windows 11 25H2 (2026-11-11, `0a8a8f1`/`3a8a715`):
  `npm run test:headless` 50 Rust (the 8 `ksni`/`zbus` tests are Linux-only) +
  29 frontend tests; fmt, clippy, release build pass; no Linux crate in the
  Windows graph. `windows.ps1` now has 90 checks (10 new: Features section,
  example metadata, initial state, disable/enable by UIA toggle and real
  click, responsiveness, Running again after relaunch): 90/0 in 3 runs.
  `windows-isolated.ps1` 56/0 in 3 runs (no feature checks).
  The counts below are from before this change.
- `npm run test:headless` (31 Rust + 13 frontend tests): passes on WSL2
  (2026-11-10, `8ca3bd7`) and natively on Windows 11 (2026-11-10, `e8834fd`;
  23 Rust tests there, the 8 `ksni`/`zbus` ones are Linux-only; again at
  `195fa8a`). This is the only headless suite on either platform.
- `npm run test:desktop:virtual`: 92 passed, 0 failed, 0 skipped on Xvfb +
  openbox, 3 runs in a row (2026-11-10, `c2cbeb4`, WSL2 Ubuntu 24.04.1); real
  release binary, stub tray host, page read via AT-SPI. Covers tray mode and
  dashboard-only mode (no watcher, no host, no bus) and a rejecting watcher
  (exit 1). The 68-check tray-mode driver passed 3 runs at `12a6185` and a
  420-launch loop registered every tray item.
- `tests/desktop/run.sh`: 57 passed, 0 failed, 2 skipped on WSL2/WSLg
  (2026-11-10, 65-check driver before `53d9743`). The tray host is a stub; SKIP
  is never a pass.
- `tests/desktop/windows.ps1`: 80 passed, 0 failed on native Windows 11 25H2
  (2026-11-10, `e8834fd` with the test fix `71442d3`: 3 runs on the release
  binary, 2 on the NSIS-installed copy), real desktop; it moves the mouse.
  Covers the Hide button, which works with hide granted at runtime. Again 80/80,
  3 runs, at `195fa8a` (`npm run test:desktop:windows`). Needs the unlocked,
  interactive desktop with a taskbar; otherwise it exits 3 without running.
- `tests/desktop/windows-isolated.ps1`: 56 passed, 0 failed, 3 runs each at
  `195fa8a` and `0131e09`, native Windows 11. Real binary on a private desktop:
  unattended, nothing on screen, no input, but **not headless** (needs a
  logged-in session) and no tray, tray menu or Quit. A close-quits mutant fails
  9 checks. It cannot run elevated (WebView2 then starts its browser on the
  Default desktop; `0x80070578`) and reports NOT RUN, exit 4; again 56/56,
  3 runs, at `468a2c3`.
- Test categories (headless / virtual display / private desktop / real desktop):
  [testing.md](development/testing.md). No JS linter.
- CI (GitHub Actions, private repo on GitHub Free: $12 of hosted compute a
  month, then blocked): `ci.yml` runs automatically on pull requests to and
  pushes to `main` (Linux + Windows: `npm ci`, check, build, fmt, clippy,
  `test:headless`; docs-only changes skipped). Its two automatic runs so far
  passed: both the Ubuntu and the Windows headless job (run ids, commits and
  durations not recorded). Manual only:
  `desktop-virtual.yml` (virtual display, not a real desktop) and
  `desktop-windows-isolated.yml` (experimental). The real-desktop Windows test
  is not in CI. `desktop-virtual.yml` passed on GitHub, 92/0/0 (run id and
  commit not recorded); this supersedes its first run (`14a5de1`, failed
  89/2/1 on a driver bug fixed in `ead47e7`). It is a virtual display with a
  stub tray host, so it says nothing about real GNOME, KDE or XFCE
  desktops. `desktop-windows-isolated.yml` ran twice
  (Windows Server 2025): the first run failed on the elevated hosted job,
  reproduced on Windows 11 elevated; the second (2026-11-10) printed
  `elevated: True` and reported NOT RUN, exit 4 (not a pass): hosted runners
  cannot run this test.
  No other workflow result is recorded.
  [testing.md](development/testing.md#ci).

## Platform limitations (summary)

Windows 11, the primary runtime platform, is validated by the automated Windows
test at `e8834fd`, including the NSIS-installed copy (no person's visual check
yet, MSI not installed). Windows 10 and real Linux desktops (GNOME, KDE, XFCE,
real Wayland) are **not validated**; the development machine has no real Linux
desktop (WSL2 only, Windows 11 Home without a VM, surveyed 2026-11-10). Windows 11 puts the tray icon in the
hidden-icons flyout. Linux: the tray is a StatusNotifierItem via `tray-icon`'s
pure-Rust `ksni` backend, which avoids a registration race in
libayatana-appindicator. Without a StatusNotifierWatcher (vanilla GNOME), host or
session bus sotyware runs dashboard-only and does not run in the background (no
XEmbed fallback). That has been seen only under the virtual display. Some
tray-host failures (a watcher that leaves mid-call, other watcher errors) still
exit 1. Left click needs the tray host to send `Activate`. Single instance
silently turns off without a D-Bus session bus. Placeholder icons; no autostart.
A corrupt feature-preferences database is never repaired: defaults are used
until the user moves the file away. Cloud account credentials need the OS
credential store: Linux without a Secret Service (WSL2, minimal window
managers) cannot store any, by design. The connection check is verified at Backblaze by one
opt-in run and one UI run each on Linux and Windows, never at Hetzner; while one credential-store call waits (an unlock
prompt), the other account calls wait behind it. The bucket browser is verified
at a real provider only for a refused root and an empty folder; a key limited
to a file name prefix cannot browse (Backblaze refuses the root); no total
count, no cache, no file operations. Full list: [limitations.md](status/limitations.md).

## Milestone and next steps

Milestone 0: prove the desktop runtime on Windows and Linux. Implemented;
validated natively on Windows 11 (automated, last at `a7bf8f3`, including the built-in features and their saved choices) and in Linux
simulation (WSL2 is the development environment, not a runtime target). Next:
the manual check on Windows 11, Windows 10, then real Linux desktops (including
dashboard-only mode on vanilla GNOME; needs a VM, live session or Linux machine); record results in the
[validation matrix](status/status.md#validation-matrix). See
[roadmap.md](status/roadmap.md). Cloud storage next steps (browsing a real bucket with content, then
downloads from the browser): [roadmap.md](status/roadmap.md#cloud-storage-after-the-proof-of-concept).
Further local persistence (settings, other
data), a feature-management UI, third-party plugins, sync and autostart are
undecided ideas, not tasks.

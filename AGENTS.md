# AGENTS.md

Canonical instructions for every coding agent (and human) working on sotyware.
Tool-specific files such as `CLAUDE.md` point here and must not add competing
rules. Start each task by reading [docs/CONTEXT.md](docs/CONTEXT.md).

## Project identity

- **Name:** `sotyware`, always lowercase, also at the start of a sentence and in UI
  text. Bundle identifier `com.sotyware.desktop`; Cargo package and binary
  `sotyware`; Rust library `sotyware_lib` ([why](docs/architecture/decisions.md#adr-007-library-crate-named-sotyware_lib)).
- **Purpose:** a desktop app that runs continuously in the background, stays
  lightweight, lives in the system tray and opens a dashboard window on demand.
  Where Linux has no tray host, it runs as a plain window instead
  (dashboard-only mode).
- **Platforms:** code and builds target Windows, Linux and macOS (Apple
  Silicon and Intel, ADR-037), and every change must keep all three working
  ([cross-platform.md](docs/development/cross-platform.md)); macOS is built
  and tested only on GitHub's hosted runners ([macos.md](docs/development/macos.md)).
  **Release support** is narrower: the releases, `v0.1.0` (published
  2026-11-15), `v0.1.1` (2026-11-16) and `v0.1.2` (2026-11-17), target
  Windows 11 only, and their
  installers have not yet been through the installation test; Windows 10 is not validated, Linux has
  never run on a real desktop and macOS never on a Mac a person used, so
  none of them may be described as supported
  ([release.md](docs/development/release.md#support-scope)).
- **Current scope:** the desktop runtime (window, tray, lifecycle, single
  instance) and the foundation for built-in features (registry, lifecycle,
  enable/disable from the dashboard), and cloud storage
  in the dashboard: account management (add, edit, check, remove), bucket
  browsing, downloading one file at a time, uploading batches of up to 100
  files into the folder shown, and file management (ADR-026): folders,
  rename, copy, move, delete and restore as reviewed plans, search,
  sorting and filters, properties, previews, share links, several-item
  downloads, folder and drag-and-drop uploads, favorites and recent items
  (never overwriting), an operation journal with a recovery view for
  work interrupted by a crash (ADR-030), backups to an independent
  destination with verification, retention, restore and a standalone
  recovery tool (ADR-031), and an optional sotyware account with
  end-to-end encrypted sync of storage accounts and backup jobs (ADR-032;
  the service runs at `https://accounts.sotyware.com` since 2026-11-17,
  ADR-045, and builds from `main` and `v0.1.2` use it; earlier releases
  have none),
  and signed application updates from GitHub
  Releases of this public repository, `casualsoty/sotyware` (ADR-033,
  ADR-036, ADR-039; published by `release.yml` with `GITHUB_TOKEN`, the
  signing key only in the protected `release` environment; the release key
  exists since 2026-11-15, its public half in `src-tauri/update-key.pub`,
  and `v0.1.0` was published with it that day, `v0.1.1` on 2026-11-16,
  `v0.1.2` on 2026-11-17; the
  maintainer's Windows 11 updated itself from 0.1.0 to 0.1.1 through
  Update now, the one update installed so far), in a
  desktop shell with a sidebar, Settings and one design system (ADR-035),
  on Windows with the page's own title bar and everywhere a remembered window
  placement that starts at half the primary work area (ADR-040); storage usage
  measured by listing, with cost estimates from versioned price lists, and a
  bucket's snapshots and backups in Cloud Storage, including deleting one
  sotyware backup snapshot (ADR-041); automatic sync of an enrolled device on
  events (ADR-042); folder sync: an account's bucket kept in step with a
  folder the user chose, in both directions, in the background (ADR-043;
  tested against local test servers only); and the autoclicker and macros
  (ADR-050: automatic clicking, recording mouse and keyboard input into
  macros, an editor, playback with repetitions and speed, a macro library,
  profiles and global shortcuts, sent and recorded by Rust through native
  APIs on Windows and Linux X11 only, while the built-in feature
  `autoclicker` runs, which is off by default; X11 tested with real input
  on a private Xvfb only, Windows unit-tested natively and its backend run
  with real input by the opt-in desktop test on the maintainer's
  Windows 11 on 2026-11-18; the page driven on Windows by
  `windows-autoclicker.ps1` on 2026-11-19: 101 of 102 checks, the shortcut
  field's refusal having no text for UI Automation). No other product features exist yet. Current milestone and status:
  [docs/status/status.md](docs/status/status.md).

### Terminology

| Term | Meaning |
| --- | --- |
| dashboard | the one webview window, label `main` (`DASHBOARD_LABEL`) |
| tray | the system tray icon and its menu |
| hide | make the dashboard invisible; process keeps running |
| close | the window manager's close (✕, Alt+F4); sotyware turns it into hide in tray mode, into quit in dashboard-only mode |
| quit | end the process; the tray "Quit" does this, and close in dashboard-only mode |
| tray mode | the tray icon exists; the normal, background-running mode (`OperatingMode::Tray`) |
| dashboard-only mode | Linux without a reachable tray host: no tray, no hide, close quits (`OperatingMode::DashboardOnly`) |
| open / show | `show_dashboard()`: un-minimize, show, focus the existing window |
| primary instance / second launch | the running process / a later start that hands over and exits |
| feature | a built-in Rust module implementing `Feature`, listed in `features::builtin()` and run by the `FeatureRegistry`; not a third-party plugin |
| enable / disable (a feature) | start it / stop it through the registry's lifecycle (`set_feature_enabled`) |
| feature preference | the user's saved enable/disable choice for one feature, keyed by its ID, in `preferences.sqlite3`; absent until the user switches the feature |
| storage account | one set of credentials at one cloud storage provider (`AccountConfig`, kebab-case ID); two keys at the same provider are two accounts |
| provider profile | what sotyware knows about one S3-compatible provider, as data (`ProviderProfile`); not code |
| plan (file operation) | what a copy, move, rename, delete or restore would do, computed by Rust with reads only and shown for review before anything changes (`PlanSummary`, ADR-026) |
| delete marker | what a delete adds on a versioned bucket instead of removing the object; the versions stay and the file can be restored |
| operation journal | `operations-journal.sqlite3`: each upload, download temporary file, staging folder and file operation is recorded before it can change anything and removed once its outcome is confirmed; what is left after a crash is shown in the recovery view (ADR-030) |
| backup job / snapshot | a configured copy of an account's bucket (or folder) into another destination / one completed run's manifest (`snapshots/<job>/<time>-<id>.json`) listing every object and its content blob (ADR-031) |
| vault / account key / recovery key | the end-to-end encrypted sync document / the random key that encrypts it / the 256-bit key shown once that can unwrap the account key on a new computer (ADR-032) |
| enrolled device | a computer whose sealed copy of the account key is stored at the service (unlocked with the recovery key or approved by another enrolled device) |
| provider capability | one behaviour a provider may or may not have (conditional writes, delete markers, …) with its evidence: documented, verified at the provider, assumed from Amazon S3, or unknown (ADR-029) |
| test profile | a directory a desktop test creates for one run; a test-profile build (Cargo feature `test-profile`) refuses to start without one and keeps its databases, webview data and credentials (own namespace) inside it (ADR-027) |
| sync / sync folder (folder sync) | one storage account's bucket kept in step with one folder on this computer, in both directions, in the background (`SyncConfig`, ADR-043) / that folder, chosen by the user, marked by `.sotyware-sync/root.json`; not the vault sync of the sotyware account |
| synced state | per path, what the sync folder and the bucket last agreed on (local stamp, SHA-256, ETag); the base of folder sync's three-way comparison |
| sync trash / deletion guard | `.sotyware-sync/trash/<day>/`: files folder sync removed here because they were deleted in the bucket, kept 30 days / the rule that holds a pass's deletions for the user's answer when they are many (≥ 1,000, or ≥ 50 and more than half of the synced files) |
| autoclicker / session (automation) | the built-in feature `autoclicker` and its page (ADR-050) / the one thing that may run at a time: the clicker, a recording or a macro's playback |
| macro / action | a recorded or written sequence of input with its timing and playback settings, stored as a versioned JSON document (`Macro`) / one step of it (`Action`: move, button, click, scroll, key, keys, wait, pause), done its delay after the previous one |
| profile (automation) | a named set of the clicker's configuration, shortcuts and a macro to play (`Profile`); the default profile is the one sotyware starts with |
| binding / emergency stop | a shortcut that can be switched off without forgetting its keys (`Binding`, a `Chord` like `Ctrl+Alt+KeyR`) / the binding shared by every profile that stops whatever runs and releases what it held |
| input backend | the platform's implementation of `automation::input::Backend` (Windows: SendInput, low-level hooks, RegisterHotKey; X11: XTest, RECORD, XGrabKey) |
| update manifest / version floor | `sotyware-update.json`, the minisign-signed description of a release (version, channel, per-platform installer URL, signature and SHA-256) / the highest version a verified manifest ever offered this installation; anything lower is refused as a replay (ADR-033) |

Use these words consistently in code, UI, commits and docs. Docs use British
spelling ("behaviour").

## Technology stack

Declared ranges are in the manifests; resolved versions are from the lockfiles
(`package-lock.json`, `src-tauri/Cargo.lock`) as of 2026-11-15 (rfd built from a local patched copy, ADR-023).

| Component | Declared | Resolved |
| --- | --- | --- |
| Tauri (Rust crate, feature `tray-icon`) | `2` | 2.12.5 |
| tauri-build | `2` | 2.7.4 |
| tauri-plugin-single-instance (desktop targets only) | `2` | 2.5.5 |
| tauri-plugin-dialog (desktop targets only; Rust-side save dialog for cloud downloads, no frontend permission) | `2` | 2.8.4 (with `rfd` 0.17.2: GTK 3 on Linux, common item dialog on Windows; **built from `src-tauri/patches/rfd`** through `[patch.crates-io]`, Windows dialog paths read as `OsString`, ADR-023; `tauri-plugin-fs` 2.7.1 as its dependency, not registered) |
| Rust | edition 2024, `rust-version = "1.90"` | developed with 1.99.1 |
| `serde` (feature `derive`; already in the tree through Tauri) | `1` | 1.0.229 |
| `rusqlite` (feature `bundled`, no default features; feature preferences) | `0.41` | 0.41.1 |
| `libsqlite3-sys` (through `rusqlite`; compiles the bundled SQLite) | through `rusqlite` | 0.39.1 (SQLite 3.54.1) |
| `@tauri-apps/api` (only runtime JS dependency) | `^2.12.4` | 2.12.4 |
| `@tauri-apps/cli` | `^2.12.4` | 2.12.4 |
| Svelte | `^5.57.3` | 5.57.3 |
| `@sveltejs/vite-plugin-svelte` | `^7.3.1` | 7.3.1 |
| Vitest (frontend tests) | `^4.1.12` | 4.1.12 |
| happy-dom (DOM for Vitest) | `^21.0.4` | 21.0.4 |
| svelte-check | `^4.7.8` | 4.7.8 |
| TypeScript | `~6.0.3` | 6.0.3 |
| Vite | `^8.3.7` | 8.3.7 |
| Node.js | `^20.19 \|\| ^22.12 \|\| >=24` (dependency `engines`) | developed with 24.22.0 |
| Package manager | npm (lockfile v3) | developed with 11.19.1 |
| `tray-icon` (through Tauri; Linux-only direct dependency adds feature `ksni`) | `0.25` | 0.25.1 |
| `ksni` (Linux tray backend, pure Rust over `zbus` 5.19.0) | through `tray-icon`; also a Linux-only direct dependency `0.3` (no features, to name its error types) | 0.3.7 |
| `zbus` (Linux only) | through `ksni` and the single-instance plugin; also a Linux-only direct dependency `5` (no features, to name its error types) | 5.19.0 |
| `aws-sdk-s3` (no default features; `rt-tokio`, `http-1x`; cloud storage proof of concept) | `1` | 1.102.0 (newest supporting Rust 1.90) |
| `aws-smithy-http-client` (feature `rustls-ring`: rustls on ring, not aws-lc-rs) | `1` | 1.1.9 |
| `tokio` (`fs`, `io-util`, `sync`, `rt`, `time`; already in the tree through Tauri) | `1` | 1.54.1 |
| `http-body` (the upload request body's `Body` trait, ADR-021; already in the tree through `aws-sdk-s3` and hyper) | `1` | 1.1.0 |
| `sha2` (already in the tree through Tauri) | `0.10` | 0.10.9 |
| `serde_json` (already in the tree through Tauri; backup manifests and job configurations, ADR-031, and the recorder) | `1` | 1.0.154 |
| Vault cryptography (ADR-032, RustCrypto): `chacha20poly1305` (XChaCha20-Poly1305; `alloc`, `getrandom`), `crypto_box` (X25519 sealed boxes; `seal`, `salsa20`, `getrandom`), `hkdf` (already in the tree), `zeroize`, `base64` (already in the tree) | `0.10`, `0.9`, `0.12`, `1`, `0.23` | 0.10.1, 0.9.1, 0.12.4, 1.9.1, 0.23.1 |
| Account service client (ADR-032): `reqwest` (already in the tree through Tauri; `json`, `rustls-no-provider`: the TLS config is built with ring, never aws-lc-rs), `rustls` (`ring`, `std`, `tls12`), `rustls-native-certs` (both already in the tree) | `0.13`, `0.23`, `0.8` | 0.13.5, 0.23.46, 0.8.5 |
| `sotyware-server` (path `../server`) and `axum` (dev only: the end-to-end account tests run the service in process; it brings the service's `lettre` tree into the desktop lock file, ADR-044) | path, `0.8` | 0.1.0, 0.8.9 |
| `aws-smithy-runtime-api`, `aws-smithy-types` (optional, only with the off-by-default feature `request-recorder`, ADR-024; already in the tree; also dev-only for `hook_assumptions.rs`) | `1`, `1` | 1.11.3, 1.4.3 |
| `tauri-plugin-updater` (desktop targets only, no default features, `rustls-tls`: rustls without a provider, sotyware passes its ring-based TLS configuration; driven from Rust only, ADR-033) | `2` | 2.14.1 |
| `minisign-verify` (the update manifest's signature, ADR-033; already in the tree through the updater) | `0.3` | 0.3.0 |
| `notify` (desktop targets only, no default features but `macos_fsevent`: folder sync's file watcher, ADR-043; adds `notify-types`, `inotify`, `inotify-sys`, `kqueue`, `kqueue-sys`, `objc2-core-services`) | `8` | 8.2.2 |
| `tauri-plugin-opener` (desktop targets only: folder sync's "Open folder" through its `open_path` and the feedback buttons through its `open_url` (feedback, bug reports, privacy notice: ADR-049), called from Rust; not registered, no `opener:` permission, ADR-043; adds `open`, `is-wsl`, `is-docker`) | `2` | 2.7.3 |
| `rustix` (Unix only, `fs`, `std`: folder sync's free space, `statvfs`; Linux only also `event`, `pipe`: the autoclicker's X11 shortcut thread, ADR-050; already in the tree with all four) / `windows-sys` (Windows only, `Win32_Storage_FileSystem`: free space and the hidden attribute; `Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_System_LibraryLoader`, `Win32_System_Threading`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_WindowsAndMessaging`: the autoclicker's input backend, ADR-050; every feature already compiled in the tree) | `1` / `0.61` | 1.1.6 / 0.61.2 |
| `x11-dl` (Linux only: the autoclicker's X11 backend, ADR-050; loads libX11, libXtst, libXrandr at runtime; already in the tree through tao and wry) | `2` | 2.21.0 |
| `semver` (update versions and the version floor, ADR-033; already in the tree through Tauri) | `1` | 1.0.28 |
| `minisign` (dev only: signs test manifests and packages with keys generated in the tests) and `tauri` feature `test` (dev only: the mock runtime that drives the updater plugin in tests) | `0.7`, `2` | 0.7.9, 2.12.5 |
| `keyring-core` (cloud account credentials, ADR-017) | `1` | 1.0.1 |
| `windows-native-keyring-store` (Windows only, no default features: Credential Manager) | `1` | 1.1.0 |
| `apple-native-keyring-store` (macOS only, no default features, `keychain`: the login keychain, ADR-037) | `1` | 1.0.2 |
| `security-framework` (macOS only, no default features: names the Keychain's error type; already in the tree) | `3` | 3.7.0 |
| `zbus-secret-service-keyring-store` (Linux only, `rt-async-io-crypto-rust`: Secret Service on the existing `zbus`) | `1` | 1.0.1 |
| `secret-service` (Linux only, through the store above; also a direct dependency without features, to name its error type) | `5` | 5.1.0 |
| `s3s` + `s3s-fs` (dev only: in-process S3 test server), `hyper-util` (dev only), `digest` (dev only, pinned `=0.11.0-rc.4` for `s3s`) | `0.12` | 0.12.0 |
| `hyper` (dev only, already in the tree: the test server's per-request service, which reads a request it refused with 412 to its end) | `1` | 1.12.0 |
| `http` (dev only, already in the tree: the test server's `ListMultipartUploads` route) | `1` | 1.5.0 |
| `bytes`, `futures-core` (dev only, already in the tree: the test server's slow, stalled or cut-off object bodies) | `1`, `0.3` | 1.12.1, 0.3.34 |
| Linux webview / tray | WebKitGTK 4.1 / StatusNotifierItem via `ksni` (libayatana-appindicator3 is not loaded) | system packages / crate |
| **Account service** (`server/`, own `Cargo.lock`, ADR-032): `axum` (http1, json, tokio, query), `tokio`, `serde`, `serde_json`, `rusqlite` (bundled, backup), `argon2` (Argon2id), `getrandom`, `sha2`, `subtle`, `base64`, `lettre` (no default features; `smtp-transport`, `builder`, `hostname`, `tokio1-rustls`, `ring`, `rustls-native-certs`: SMTP over rustls on ring, ADR-044; adds `rustls` 0.23.46, `tokio-rustls` 0.26.6, `rustls-native-certs` 0.8.5, `hostname` 0.4.2); dev: `tower`, `http-body-util`, `tokio` (`io-util`: the test SMTP server) | `0.8`, `1`, `1`, `1`, `0.41`, `0.5`, `0.3`, `0.10`, `2`, `0.22`, `0.11.23`; `0.5`, `0.1`, `1` | 0.8.9, 1.54.1, 1.0.229, 1.0.154, 0.41.1, 0.5.3, 0.3.4, 0.10.9, 2.6.1, 0.22.1, 0.11.23; 0.5.3, 0.1.5, 1.54.1 |
| Windows webview | WebView2 | system |

Re-check this table against the lockfiles when you change dependencies.

## Architecture

Implemented today (details in [docs/architecture/](docs/architecture/overview.md)):

- **Entry point:** `src-tauri/src/main.rs` → `sotyware_lib::run()` in
  `src-tauri/src/lib.rs`.
- **Runtime:** `run()` builds the Tauri app: single-instance plugin (first, desktop
  only) → `FeatureRegistry` from `features::builtin()` (managed state) →
  `build()` → feature preferences loaded from SQLite (managed state, before the
  event loop) → `app.run()`: window `main` from `tauri.conf.json` (created
  hidden) → `setup` creates the tray, picks the `OperatingMode`
  (`select_mode`), starts the features that are enabled (stored choice, else
  default), then gives the dashboard its chrome (Windows: `title_bar`) and its
  place (`window_state`: saved, else half the primary work area, at least
  900 × 540, centred) and
  shows it (ADR-040) → event loop. `RunEvent::Exit` stops the running
  features, closes the preferences and saves the window placement.
- **Operating modes:** tray mode when the tray is created. Dashboard-only mode
  when `tray::unavailable_reason()` recognises a missing Linux tray host (no
  session bus, no StatusNotifierWatcher, no StatusNotifierHost). Any other tray
  error → `exit(1)`. Platform checks stay in `unavailable_reason()`; other code
  sees only the mode.
- **Window lifecycle:** close is turned into hide in tray mode and into `exit(0)`
  in dashboard-only mode; implicit exits (`ExitRequested { code: None }`) are
  blocked; only `exit(code)` ends the process.
- **Tray:** `src-tauri/src/tray.rs`, "Open Dashboard" → `show_dashboard()`,
  "Quit" → `exit(0)`. Exists in tray mode only.
- **Single instance:** second launch hands over to the primary, which calls
  `show_dashboard()`; the new process exits 0. Same in both modes.
- **Built-in features:** `src-tauri/src/features/` (`Feature` trait, `builtin()`
  list, `FeatureRegistry` with states registered/disabled/initializing/running/
  failed/stopping, per-feature error isolation; the `backup-scheduler` and
  `update-check` features; the `example` feature was removed 2026-11-15). Details
  and how to add a feature: [features.md](docs/architecture/features.md).
- **Feature preferences:** each user switch is saved in `preferences.sqlite3`
  in Tauri's `app_local_data_dir()`; startup uses the stored choice, else
  `enabled_by_default`. Policy in `src-tauri/src/features/preferences.rs`
  (`FeaturePreferences`, `PreferenceStore` trait), SQLite only in
  `src-tauri/src/storage.rs` (schema version `PRAGMA user_version`,
  append-only `MIGRATIONS`). Loaded in `run()` between `build()` and `run()`;
  the registry never touches storage
  ([ADR-015](docs/architecture/decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)).
- **Cloud storage:**
  `src-tauri/src/cloud/`: provider profiles as data (`backblaze-b2`,
  `hetzner`, `s3-compatible`), accounts without secrets (`AccountConfig`,
  `Accounts`, `AccountStore`; SQLite `cloud-accounts.sqlite3` in
  `storage.rs`), credentials by account id in the OS credential store only
  (`CredentialStore` trait, `OsCredentialStore` in `cloud/keyring.rs`:
  Windows Credential Manager, Linux Secret Service; no fallback; environment
  variables only in test builds), and one S3 adapter (`S3Store`,
  `aws-sdk-s3`) with end-to-end SHA-256 and a transfer queue per account.
  Its upload (`S3Store::upload` in `cloud/s3/upload.rs`) refuses an existing key, re-hashes what it sends,
  has one commit point for cancellation, aborts failed multipart uploads
  (after awaiting, at most 5 s, a cancelled part that was handed over
  completely, [ADR-025](docs/architecture/decisions.md#adr-025-a-cancelled-part-that-was-handed-over-completely-is-awaited-before-the-abort)),
  verifies by `HeadObject` and times out on stalls instead of the read
  timeout ([ADR-021](docs/architecture/decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration)).
  The dashboard reaches only the account part, through `cloud/dashboard.rs`
  (`CloudAccounts`, managed state loaded in `run()` before the event loop, no
  credential read at startup): list, add, edit, remove accounts and a
  connection check (`S3Store::check_bucket`: `HeadBucket`, at most one
  `ListObjectsV2` of one key; never `ListBuckets`). Credential-store and
  SQLite work runs in `spawn_blocking`. Read-only browsing of an account's
  configured bucket goes through `cloud/browser.rs` (`list_cloud_folder`:
  one `ListObjectsV2` page of one folder level, delimiter `/`; the
  provider's continuation token stays in Rust behind a cursor tied to the
  account, bucket and prefix). No command deletes or otherwise changes
  objects, or returns a credential. One file at a time can be downloaded
  through `cloud/download.rs` (`download_cloud_file`, `cancel_cloud_download`,
  `retry_cloud_download`; `CloudDownloads` managed state, one slot): Rust
  opens the native save dialog (`tauri-plugin-dialog`; the frontend never
  sends a path), `S3Store::download` streams into a temporary file next to
  the destination, verifies size and stored SHA-256, then moves it into
  place; progress goes over a Tauri `Channel` passed with the command;
  quitting cancels a running download. Upload batches go through
  `cloud/upload.rs` (`upload_cloud_files`, `cancel_cloud_upload`,
  `retry_cloud_upload`; `CloudUploads` managed state, its own slot, one
  batch at a time beside one download): Rust opens the native multi-file
  dialog (the page sends only the destination folder and never receives a
  path), checks up to 100 files before sending anything (invalid ones are
  skipped with a reason), then uploads them one after another through
  `S3Store::upload` after a `HeadObject` that skips existing objects; events
  with `seq` and one final summary go over a per-batch `Channel`; retry
  takes only eligible files; multipart ids whose abort failed are kept in
  memory and aborted again on retry and on quit (bounded wait). The
  browser's upload panel (`src/CloudUpload.svelte`) calls them and only
  shows what the events say. File management (ADR-026): `cloud/operations.rs`
  (`CloudOperations`, its own slot: plan / discard / start / cancel / retry of
  copy, move, rename, delete and restore; folders are prefixes enumerated
  when planning; copies are `CopyObject` with metadata and the planned ETag,
  verified; moves delete sources only after every copy is verified; deletes
  report delete markers; restore removes exactly the newer delete markers),
  `cloud/manage.rs` (create folder, properties, previews, search with
  cursors in Rust, deleted items, presigned share links),
  `cloud/bulk_download.rs` (`BulkDownloads`: several files and folders into
  a picked folder via a staging folder, never replacing), the folder upload
  in `cloud/upload.rs`, `cloud/drop.rs` (`CloudDrops`: dropped paths kept in
  Rust), `cloud/places.rs` (`CloudPlaces`: favorites and recent items in
  `cloud-places.sqlite3`) and `cloud/s3/manage.rs` (the requests).
  Provider behaviour is a capability model with evidence in
  `cloud/provider.rs` (ADR-029); `If-None-Match: *` is sent only where
  `conditional_put` is established (no shipped profile).
  **Operation journal** (ADR-030): `src-tauri/src/journal.rs` (`Journal`,
  `JournalStore`; SQLite `operations-journal.sqlite3` in `storage.rs`);
  every slot records itself (`record_in`); at startup `clean_local` removes
  only sotyware's own recorded temporary files and staging folders;
  `cloud/recovery.rs` lists, inspects (reads only), dismisses entries and
  lists/aborts incomplete multipart uploads one by one; page
  `src/CloudRecovery.svelte`.
  **Backups** (ADR-031): `src-tauri/src/backup/` (`model.rs` pure rules:
  manifest format, change check, retention, health; `engine.rs`: run,
  verify, prune, restore over two `S3Store`s through a spool folder;
  `mod.rs`: `Backups` managed state with `backup.sqlite3`, one task at a
  time, the scheduler, nine commands); the `backup-scheduler` feature;
  page `src/Backup.svelte`; standalone `scripts/recovery/sotyware-restore.py`.
  **Accounts and sync** (ADR-032): `src-tauri/src/account/` (`crypto.rs`:
  XChaCha20-Poly1305 envelopes, HKDF recovery and key-proof derivations,
  X25519 sealed boxes, recovery key format, fingerprints; `vault.rs`: the
  vault document and three-way merge with conflict copies; `client.rs`:
  HTTPS to the service (loopback HTTP only for development); `mod.rs`:
  `AccountService`, 20 commands); secrets in the OS store through
  `cloud::keyring::OsSecretStore` (entries `sotyware-account/<name>`),
  local state in `account.sqlite3`; `PRODUCTION_SERVER` is
  `https://accounts.sotyware.com` (ADR-045; test-profile builds have none);
  `SOTYWARE_ACCOUNT_SERVER` only in debug and test-profile builds; page
  `src/Account.svelte`.
  **Updates** (ADR-033, ADR-036, ADR-039, [updates.md](docs/architecture/updates.md)):
  `src-tauri/src/updates.rs` (`Updates` managed state, six commands,
  `updates.sqlite3` with the version floor): `latest.json` and its
  minisign signature from the latest release of this repository
  (`release_repository!` → `RELEASE_REPOSITORY` `casualsoty/sotyware`,
  `MANIFEST_URL`, at
  most five HTTPS redirects, one refetch on a signature mismatch) are
  verified with the key compiled in from `src-tauri/update-key.pub`
  (empty: updates off; endpoint and key from `SOTYWARE_UPDATE_*` only in
  debug and test-profile builds) before `tauri-plugin-updater` gets exactly
  that manifest (`restore_update`), downloads, verifies the package
  signature and its version, then sotyware checks the SHA-256 and installs
  only on the user's click while nothing else runs; Windows' installer
  restarts sotyware (`release_resources` runs first), macOS and Linux
  replace in place and `restart_after_update` uses `request_restart`;
  platform entries for Windows (nsis, msi), macOS (aarch64, x86_64 app)
  and Linux (appimage, deb, rpm); automatic checks are the built-in
  feature `update-check`; Settings › About & updates and one notification
  in the shell; release tools `scripts/update-manifest.mjs` and
  `scripts/release-assemble.sh`; releases only through
  `.github/workflows/release.yml` (dry runs sign with a throwaway key;
  only its publish job, in the `release` environment, gets the signing
  key; Apple credentials only in `macos-signing`)
  ([cloud-storage.md](docs/architecture/cloud-storage.md#dashboard),
  [ADR-016](docs/architecture/decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter),
  [ADR-017](docs/architecture/decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store),
  [ADR-018](docs/architecture/decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands),
  [ADR-019](docs/architecture/decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust),
  [ADR-020](docs/architecture/decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel),
  [ADR-022](docs/architecture/decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)).
- **Test profiles (desktop test builds only, ADR-027):** the
  off-by-default Cargo feature `test-profile` (also turned on by
  `request-recorder`) adds `src-tauri/src/test_profile.rs`: the app refuses
  to start (exit 3) unless `SOTYWARE_TEST_PROFILE` names a profile the
  driver created, refuses one inside its own folders, runs under the
  identifier `com.sotyware.desktop.test`, keeps databases and webview data
  in the profile and credentials in their own namespace
  (`CredentialNamespace`, service `com.sotyware.desktop.test` + nonce;
  the app's own namespace is not compiled in), and writes an
  acknowledgement the drivers check before touching the window. Built
  only into `--target-dir target/test-profile` (or `target/recorder`),
  never packaged (`scripts/check-release-binary.mjs`). Every desktop driver
  uses it (`tests/desktop/test_profile.py`, `TestProfile.psm1`);
  `scripts/check-desktop-isolation.mjs` (in `test:headless`) keeps them
  from naming the user's folders or credentials
  ([ADR-027](docs/architecture/decisions.md#adr-027-desktop-tests-run-a-test-profile-build-that-cannot-reach-the-users-data)).
- **Request recorder (test builds only):** the off-by-default Cargo
  feature `request-recorder` adds `src-tauri/src/cloud/recorder.rs`, one
  read-only SDK interceptor per `S3Store` (installed in `S3Store::new`)
  that records the app's own S3 requests, redacted, when the
  `SOTYWARE_REQUEST_RECORD_*` variables are set. Built only into
  `--target-dir target/recorder` (build.rs), never packaged
  (`scripts/check-release-binary.mjs`, `beforeBundleCommand`)
  ([ADR-024](docs/architecture/decisions.md#adr-024-an-opt-in-request-recorder-observes-the-apps-own-s3-requests-through-an-sdk-interceptor-compiled-only-into-a-test-build),
  [cloud-storage.md](docs/architecture/cloud-storage.md#request-recorder)).
- **Folder sync** (ADR-043, [folder-sync.md](docs/architecture/folder-sync.md)):
  `src-tauri/src/folder_sync/` (`model.rs` pure rules: paths and names,
  exclusions, the decision table, conflict names, deletion guard, retries;
  `scan.rs`: walk, stamps, hashes, marker, `.sotyware-sync` staging and
  trash, free space; `engine.rs`: one task per sync, passes, listing,
  planning, actions with live checks; `watch.rs`: `notify`; `migrate.rs`:
  moving a sync folder; `mod.rs`: `FolderSyncs` managed state, live status,
  16 commands), `src-tauri/src/storage/folder_sync.rs`
  (`folder-sync.sqlite3`, TEMP scan tables, the planning query), the
  built-in feature `folder-sync`, `S3Store::upload_with`
  (`Destination::Replace` of the synced ETag, stored modification time);
  page `src/CloudSync.svelte` (the bucket's Sync tab), the sidebar's
  indicator through `src/folderSyncModel.svelte.ts`.
- **Autoclicker and macros** (ADR-050, [automation.md](docs/architecture/automation.md)):
  `src-tauri/src/automation/` (`model.rs` pure rules: clicker
  configuration, actions, macros, playback, profiles, settings, bindings
  and conflicts, screen geometry, the export file; `keys.rs`: the key
  table, `Key`, `Chord`; `timing.rs`: `Control`, `Clock`; `input.rs`: the
  `Backend` trait, `Output` (held input released on every exit path, also
  from a panic hook), `MockBackend`; `clicker.rs`, `recorder.rs`,
  `player.rs`: the engines; `native.rs`, `windows.rs` (SendInput, low-level
  hooks, RegisterHotKey on one input thread), `x11.rs` (XTest, RECORD,
  XGrabKey; refused on Wayland); `mod.rs`: `Automation` managed state, one
  session at a time, shortcut registration, the live status on a channel,
  29 commands), `src-tauri/src/storage/automation.rs`
  (`automation.sqlite3`), the built-in feature `autoclicker` (off by
  default), page `src/Autoclicker.svelte` (its own JS chunk) with its tabs,
  the sidebar's indicator through `src/automationModel.svelte.ts`.
- **macOS menu bar** (ADR-037): `src-tauri/src/app_menu.rs` (Quit ⌘Q
  through `AppControl::quit`, Edit for the webview's clipboard keys,
  Window); `RunEvent::Reopen` (Dock click) shows the dashboard; credentials
  in the login keychain (`cloud/keyring.rs`).
- **Frontend** ([frontend.md](docs/architecture/frontend.md), ADR-035):
  the shell `src/App.svelte` (sidebar `src/Sidebar.svelte`, pages
  `src/Overview.svelte`, `src/Settings.svelte`, update notification on
  `src/updatesModel.svelte.ts`; navigation types in `src/navigation.ts`),
  the design system (`src/styles/tokens.css`, `src/styles/base.css`,
  `src/Icon.svelte`, theme in `src/theme.ts`), with `src/Features.svelte` and `src/CloudStorage.svelte`
  (+ `src/CloudAccountForm.svelte`, `src/CloudBrowser.svelte`,
  `src/CloudUpload.svelte`, `src/CloudOperation.svelte`,
  `src/CloudFolderPicker.svelte`, `src/CloudProperties.svelte`,
  `src/CloudBulkDownload.svelte`, `src/CloudDeleted.svelte`,
  `src/CloudRecovery.svelte`, `src/Modal.svelte`, `src/Backup.svelte`,
  `src/Account.svelte`, `src/Updates.svelte`, `src/Autoclicker.svelte` and
  its tabs;
  pure list logic in `src/fileList.ts`). Its only
  access to Rust is the commands
  `operating_mode`, `list_features`, `set_feature_enabled`,
  `list_cloud_providers`, `list_cloud_accounts`, `add_cloud_account`,
  `update_cloud_account`, `remove_cloud_account`, `check_cloud_account`,
  `list_cloud_folder`, `download_cloud_file`, `cancel_cloud_download`,
  `retry_cloud_download`, `upload_cloud_files`, `cancel_cloud_upload`,
  `retry_cloud_upload` and the 22 file-management commands of ADR-026
  (`plan_cloud_operation`, `discard_cloud_operation_plan`,
  `start_cloud_operation`, `cancel_cloud_operation`, `retry_cloud_operation`,
  `create_cloud_folder`, `cloud_object_properties`, `preview_cloud_file`,
  `search_cloud_folder`, `list_cloud_deleted`, `share_cloud_file`,
  `upload_cloud_folder`, `download_cloud_items`,
  `cancel_cloud_items_download`, `retry_cloud_items_download`,
  `list_cloud_places`, `set_cloud_favorite`, `record_cloud_recent`,
  `clear_cloud_recent`, `watch_cloud_drops`, `unwatch_cloud_drops`,
  `upload_cloud_drop`) and the five recovery commands of ADR-030
  (`list_cloud_recovery`, `inspect_cloud_recovery`,
  `dismiss_cloud_recovery`, `list_cloud_incomplete_uploads`,
  `abort_cloud_incomplete_upload`) and the three usage commands of ADR-041
  (`storage_usage_overview`, `measure_storage_usage`,
  `cancel_storage_usage`) and the eleven backup commands of ADR-031 and
  ADR-041 (`list_backups`, `save_backup_job`, `remove_backup_job`,
  `start_backup_task`, `cancel_backup_task`, `list_backup_snapshots`,
  `restore_backup`, `export_backup_manifest`, `check_backup_protection`,
  `backup_snapshot_details`, `delete_backup_snapshot`) and
  the 20 account commands of ADR-032 (`account_status`, `account_register`, `account_verify`, `account_sign_in`, `account_sign_out`, `account_set_up_sync`, `account_check_recovery_key`, `account_unlock`, `account_request_approval`, `account_approval_status`, `account_enrollments`, `account_answer_enrollment`, `account_devices`, `account_revoke_device`, `account_rotate_keys`, `account_sync`, `account_resolve_conflict`, `account_change_password`, `account_reset_password`, `account_delete`)
  and the six update commands of ADR-033/ADR-036 (`update_status`,
  `check_for_update`, `install_update`, `cancel_update`, `postpone_update`,
  `restart_after_update`) and the 16 folder sync commands of ADR-043
  (`list_folder_syncs`, `watch_folder_syncs`, `unwatch_folder_syncs`,
  `choose_sync_folder`, `discard_sync_folder_choice`, `enable_folder_sync`,
  `set_folder_sync_paused`, `sync_folder_now`, `retry_folder_sync`,
  `set_folder_sync_interval`, `disable_folder_sync`, `change_sync_folder`,
  `open_sync_folder`, `folder_sync_details`,
  `resolve_folder_sync_conflict`, `answer_folder_sync_deletions`; the only
  commands that return a local path: the sync folder the user chose, to
  show it) and `open_github_page` (ADR-049: opens a GitHub issue form
  for feedback or a bug, or the privacy notice, in the browser; the page
  sends only which one) and the 29 automation commands of ADR-050
  (`automation_status`, `watch_automation`, `unwatch_automation`,
  `automation_config`, `start_clicker`, `play_macro`, `start_recording`,
  `set_automation_paused`, `stop_automation`, `save_recording`,
  `discard_recording`, `list_macros`, `get_macro`, `create_macro`,
  `save_macro`, `update_macro_info`, `duplicate_macro`, `delete_macro`,
  `create_profile`, `save_profile`, `delete_profile`, `activate_profile`,
  `set_default_profile`, `save_automation_settings`, `export_automation`,
  `import_automation` (dialogs opened by Rust), `capture_cursor_position`,
  `capture_active_window`, `screen_layout`)
  (one `allow-<command>` permission each in
  `src-tauri/capabilities/default.json`; typed wrappers in `src/features.ts`,
  `src/cloud.ts`, `src/cloudFiles.ts`, `src/cloudRecovery.ts`, `src/cloudUsage.ts`, `src/backup.ts`, `src/account.ts`, `src/updates.ts`, `src/folderSync.ts`, `src/githubPage.ts` and `src/automation.ts`) and, in tray mode only, `getCurrentWindow().hide()`
  (`core:window:allow-hide`, granted at runtime by the `tray-mode` capability in
  `setup`), `getCurrentWindow().setTheme()` (`core:window:allow-set-theme`,
  ADR-035) and `isDecorated()` (`core:window:allow-is-decorated`), and on
  Windows only the title bar's drag region, `minimize()`, `toggleMaximize()`,
  `isMaximized()` and `close()` (granted at runtime by the `window-controls`
  capability, `title_bar.rs`, ADR-040). No other custom command, no custom events (download progress,
  upload, operation, several-item download and drop notices use per-call
  Tauri channels, not events), no `dialog:`, `fs:` or `updater:` permission. CSP:
  `img-src 'self' data:` added for previews (ADR-026).

- **Account service (`server/`, ADR-032):** a separate Rust crate (not
  part of the desktop build; own lock file; CI job `server`): axum + SQLite
  (`db.rs` append-only migrations, WAL, `secure_delete`), `security.rs`
  (Argon2id, random tokens and codes stored as SHA-256, rate limits,
  lockout), `api.rs` (`/v1`: registration with verification codes, sign-in,
  refresh rotation with reuse detection, sign-out, account, password change
  and reset, deletion, recovery wrap, device wraps with a key proof, vault
  with optimistic concurrency, devices, enrollment by approval, audit
  events, `/healthz`), `main.rs` (`serve`, `backup`, `restore`, `send-test-mail`, `outbox`),
  `mail.rs` (ADR-044: the outbox delivered through an SMTP relay set by
  `SOTYWARE_SMTP_*`, retried, never logging an address; without a relay
  mails stay in the outbox), `db::purge` run hourly by `run_purge`
  (ADR-048: sessions, codes, never-verified accounts after 7 days, audit
  events after 365 days or 31 days after deletion). Deployed 2026-11-17 (ADR-045) at
  `https://accounts.sotyware.com`: one Google Cloud free-tier e2-micro VM
  in `us-east1`, Caddy with Let's Encrypt in front, Brevo as the relay;
  `server/ops/sotyware-backup` (ADR-047): hourly age-encrypted copies to a
  Backblaze B2 bucket with Object Lock, running on the VM since
  2026-11-17
  ([server.md](docs/development/server.md),
  [server-setup.md](docs/development/server-setup.md)).

**Not implemented (planned or only discussed):** everything in
[docs/status/roadmap.md](docs/status/roadmap.md). In particular there is no
persistence other than the feature enabled/disabled choices, the cloud
storage accounts and their favorites and recent items, the operation
journal, the backup jobs, the account state, the update state, the window
placement, the storage usage measurements, the folder syncs with their
synced state and the autoclicker's settings, profiles and macros, no
settings, autostart,
third-party plugin system, feature-management UI, a second account service instance or failover, XEmbed
tray fallback, overwriting uploads (other than folder sync replacing the
version it synced), parallel uploads, ZIP downloads,
multipart copies above 5 GiB, permanent deletion of versions, a trash of
sotyware's own in the bucket, content search, transfer queue UI or download history,
online-only (on-demand) files, selective folder sync, sync bandwidth limits or
exclusion patterns,
credential
import or any credential fallback outside the OS store, restic
integration, encrypted backups, backup notifications, billing data from a
provider (no API offers it; costs are estimates), management of Backblaze's
console snapshots (no API), Windows 11 Snap Layouts on the page's maximize
button, custom event, background (periodic) sync of the sotyware account, an
input backend for macOS or Wayland, typing text as text, scheduled or
conditional automation (screen content, variables), relative mouse movement
for games, or custom command besides
the 129 listed above. Never describe these as existing.

## Coding conventions

Follow what the code already does. When in doubt, read `lib.rs` and `tray.rs`.

### Rust

- Default `rustfmt`; `cargo clippy --all-targets -- -D warnings` must pass.
- Module docs with `//!`, public items with `///`. Comments explain *why*
  (platform quirks, ordering constraints), not *what*.
- Identifiers that must match elsewhere (window label, menu ids, tray id) are
  `const`s, never repeated string literals. `DASHBOARD_LABEL` must match the label
  in `tauri.conf.json`.
- Keep testable logic in plain functions/types (like `TrayAction::from_menu_id`)
  and unit-test them in a `#[cfg(test)] mod tests` in the same file.
- Keep lifecycle policy in `lib.rs`; other modules call `show_dashboard()` or
  `AppHandle::exit` instead of re-implementing it.
- Register handlers once, on the builder or in `setup`. No polling loops or timers.

### Error handling

- Fallible setup code returns `tauri::Result` and uses `?`.
- A failing window operation that leaves the app usable is logged and the flow
  continues (`if let Err(e) = window.show() { eprintln!(...) }`).
- A failure that would leave the app unreachable is fatal: log it and
  `app.handle().exit(1)` (as for an unexpected tray error). Downgrade to a
  reduced mode only for errors you can identify by type as an expected
  environment condition (as `tray::unavailable_reason()` does); never by
  catching every error or matching message text.
- `expect` only where the app cannot start at all (`Builder::build`). No `unwrap`
  in runtime paths.
- Frontend: catch errors from Tauri API calls and show them in the UI (see
  `hideDashboard` in `App.svelte`).

### Logging

- `eprintln!` with the prefix `sotyware: ` and a lowercase message, e.g.
  `eprintln!("sotyware: failed to show dashboard: {e}")`.
- There is no logging crate or `tracing` subscriber. Adding one is an
  architectural decision (record it in
  [decisions.md](docs/architecture/decisions.md)).

### TypeScript

- Strict settings in `tsconfig.json` (`strict`, `noUncheckedIndexedAccess`,
  `exactOptionalPropertyTypes`, no unused locals/parameters). Do not relax them.
- Explicit return types on functions (`async function hideDashboard(): Promise<void>`).
- Fail loudly on broken invariants (`main.ts` throws if `#app` is missing).
- `npm run check` must report 0 errors and 0 warnings.
- Frontend tests live next to the code as `src/**/*.test.ts` (Vitest, happy-dom)
  and mock Tauri with `@tauri-apps/api/mocks`, never by stubbing `App.svelte`.

### Svelte

- Svelte 5 runes (`$state`, …) and event attributes (`onclick={…}`), not Svelte 4
  syntax (`on:click`, `export let`, stores for local state).
- `<script lang="ts">`; component-scoped `<style>`; `:global(...)` only for
  document-level styles.
- Support dark mode via `prefers-color-scheme`. Use semantic HTML (`type="button"`,
  `role="alert"` for errors).

### Dependencies

- Add a dependency only when the platform or existing code cannot reasonably do
  the job. Prefer official Tauri plugins over custom platform code. Record new
  runtime dependencies in [decisions.md](docs/architecture/decisions.md).
- Desktop-only Rust crates go in the
  `[target.'cfg(any(target_os = "macos", windows, target_os = "linux"))'.dependencies]`
  section and their use behind `#[cfg(desktop)]`.
- A new Tauri plugin or frontend API call needs the narrowest capability
  permission in `src-tauri/capabilities/default.json`. A new custom command must
  also be declared in the app manifest in `src-tauri/build.rs`.
- Commit `package-lock.json` and `Cargo.lock` changes together with the manifest
  change. Install with `npm ci` unless you are changing dependencies.
- Keep versions in `package.json`, `src-tauri/Cargo.toml` and
  `src-tauri/tauri.conf.json` identical.

### Naming

- Rust: `snake_case` functions/modules, `CamelCase` types, `SCREAMING_SNAKE_CASE`
  consts. TypeScript: `camelCase`; Svelte components `PascalCase.svelte`.
- Tauri ids and labels: lowercase kebab-case (`open-dashboard`, `sotyware-tray`),
  window labels lowercase (`main`).
- UI labels: Title Case for menu items ("Open Dashboard"), sentence case for
  buttons ("Hide dashboard").

### Testing

See [docs/development/testing.md](docs/development/testing.md). Pure logic gets
unit tests; runtime behaviour changes need the desktop test and the manual
checklist updated.

### Cross-platform code

See [docs/development/cross-platform.md](docs/development/cross-platform.md).
Windows and Linux behaviour must both be considered for every change. State in the
commit message on which platforms a change was actually run.

## Agent behaviour

Every coding agent must:

1. Inspect the relevant code before modifying it. Read the code, not only the docs.
2. Understand existing abstractions (`show_dashboard()`, `AppControl`,
   `TrayAction`, `Feature`/`FeatureRegistry`, `FeaturePreferences`/
   `PreferenceStore`, `Accounts`/`S3Store`/`ProviderProfile`/`CredentialStore`,
   `CloudAccounts`, `cloud::browser`, `CloudDownloads`/`TransferControl`,
   `CloudUploads`, `CloudOperations`, `BulkDownloads`, `CloudPlaces`,
   `CloudDrops`, `Journal`/`JournalLink` (operation journal), `Backups` and
   the backup `model`/`engine`, `AccountService`, `account::crypto`,
   `FolderSyncs`, `folder_sync::model`/`engine`/`scan` and
   `Destination`/`UploadOptions` (folder sync), `Automation`, the
   `Backend` trait and `Output`, the automation `model`, `keys` and engines
   (`clicker`, `recorder`, `player`) and `Clock`/`Control` (autoclicker),
   `Updates`/`release_resources` (updates), `WindowStates`/`window_state`
   and `title_bar` (placement and Windows chrome), `StorageUsage`/`usage`
   and `pricing` (usage and estimates), the frontend shell and design
   system (`App.svelte`, `TitleBar.svelte`, `tokens.css`/`base.css` and their
   vocabulary, `UpdatesModel`, `AutoSync`, `Icon.svelte`), `app_menu` (macOS),
   `account::vault`, `SecretStore`, provider
   `Capabilities`/`Support`, `test_server::Versioned`, `CredentialNamespace`, the test
   profile helpers (`test_profile.rs`, `tests/desktop/test_profile.py`,
   `TestProfile.psm1`), the capability file) before
   introducing new ones. New built-in functionality is a feature in
   `src-tauri/src/features/`, not new code in `lib.rs`.
3. Avoid unnecessary dependencies.
4. Avoid speculative abstractions and premature architecture. Build what the task
   needs, nothing for hypothetical features.
5. Preserve cross-platform behaviour.
6. Never weaken, skip or delete tests or checks to make a change pass.
   Desktop tests run only a test-profile build in a fresh test profile
   (ADR-027); never move, copy or delete the app's own databases or
   credentials, and never compute the app's data paths in a driver.
7. Update documentation in the same change when behaviour or architectural
   decisions change (see [Documentation rules](#documentation-rules)).
8. Run the relevant checks before committing
   ([commands](docs/development/workflow.md#checks-run-before-every-commit-that-touches-code)).
9. Commit every logically complete step.
10. Never push without explicit approval.
11. Never overwrite, discard or commit unrelated user changes. Run `git status`
    and `git diff` before starting and before committing.
12. Report what was actually tested, on which environment, versus what remains
    unverified.

## Git workflow

- [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`,
  `docs:`, `test:`, `build:`, `refactor:`, `chore:`. Subject in the imperative,
  lowercase after the type. The body explains why and what was verified.
- One logical change per commit; validate it before committing.
- Never amend, reset, rebase, force-push or otherwise rewrite existing commits
  without explicit authorization.
- Never push to a remote without explicit approval.
- Stage files explicitly; never commit build output (`dist/`, `src-tauri/target/`,
  `src-tauri/gen/`) or `node_modules/`.

## Documentation rules

- **The repository is the only source of truth.** No Notion or other external
  documentation dependency
  ([ADR-008](docs/architecture/decisions.md#adr-008-the-repository-is-the-only-source-of-truth-for-documentation)).
- **One home per fact.** Write details once and link to them. Where things belong:

  | Change | Update |
  | --- | --- |
  | Behaviour, lifecycle, tray, single instance | `docs/architecture/` (and `docs/CONTEXT.md` if the summary changes) |
  | Significant design decision | new entry in `docs/architecture/decisions.md`, with rationale |
  | Files moved, added or renamed | `docs/architecture/repository-map.md` |
  | Commands, setup, tooling | `docs/development/` |
  | Something implemented or validated | `docs/status/status.md` |
  | Limitation found or fixed | `docs/status/limitations.md` |
  | Plan or next step | `docs/status/roadmap.md` |
  | Dependency versions | the stack table above |
  | Agent rules | this file only |

- **Planned is not implemented.** Roadmap items stay in `roadmap.md` until they
  are done. Validation is recorded only for environments where it actually ran,
  with the date.
- **Remove outdated documentation only after checking** that anything still true
  is preserved elsewhere.
- Check that relative links resolve after editing docs.

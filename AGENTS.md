# AGENTS.md

Canonical instructions for every coding agent (and human) working on sotyware.
Tool-specific files such as `CLAUDE.md` point here and must not add competing
rules. Start each task by reading [docs/CONTEXT.md](docs/CONTEXT.md).

## Project identity

- **Name:** `sotyware`, always lowercase, also at the start of a sentence and in UI
  text. Bundle identifier `com.sotyware.desktop`; Cargo package and binary
  `sotyware`; Rust library `sotyware_lib` ([why](docs/architecture/decisions.md#adr-007-library-crate-named-sotyware_lib)).
- **Purpose:** a cross-platform desktop app for Windows 10/11 and Linux that runs
  continuously in the background, stays lightweight, lives in the system tray and
  opens a dashboard window on demand. Where Linux has no tray host, it runs as a
  plain window instead (dashboard-only mode).
- **Current scope:** the desktop runtime (window, tray, lifecycle, single
  instance) and the foundation for built-in features (registry, lifecycle,
  enable/disable from the dashboard, one example feature), and cloud storage
  in the dashboard: account management (add, edit, check, remove), read-only
  bucket browsing, downloading one file at a time and uploading batches of
  up to 100 files into the folder shown (never overwriting; no other
  changes to objects). No other product features exist yet. Current milestone and status:
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

Use these words consistently in code, UI, commits and docs. Docs use British
spelling ("behaviour").

## Technology stack

Declared ranges are in the manifests; resolved versions are from the lockfiles
(`package-lock.json`, `src-tauri/Cargo.lock`) as of 2026-11-13 (rfd built from a local patched copy, ADR-023).

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
| `serde_json`, `aws-smithy-runtime-api`, `aws-smithy-types` (optional, only with the off-by-default feature `request-recorder`, ADR-024; all already in the tree; the two SDK crates also dev-only for `hook_assumptions.rs`) | `1`, `1`, `1` | 1.0.154, 1.11.3, 1.4.3 |
| `keyring-core` (cloud account credentials, ADR-017) | `1` | 1.0.1 |
| `windows-native-keyring-store` (Windows only, no default features: Credential Manager) | `1` | 1.1.0 |
| `zbus-secret-service-keyring-store` (Linux only, `rt-async-io-crypto-rust`: Secret Service on the existing `zbus`) | `1` | 1.0.1 |
| `secret-service` (Linux only, through the store above; also a direct dependency without features, to name its error type) | `5` | 5.1.0 |
| `s3s` + `s3s-fs` (dev only: in-process S3 test server), `hyper-util` (dev only), `digest` (dev only, pinned `=0.11.0-rc.4` for `s3s`) | `0.12` | 0.12.0 |
| `bytes`, `futures-core` (dev only, already in the tree: the test server's slow, stalled or cut-off object bodies) | `1`, `0.3` | 1.12.1, 0.3.34 |
| Linux webview / tray | WebKitGTK 4.1 / StatusNotifierItem via `ksni` (libayatana-appindicator3 is not loaded) | system packages / crate |
| Windows webview | WebView2 | system |

Re-check this table against the lockfiles when you change dependencies.

## Architecture

Implemented today (details in [docs/architecture/](docs/architecture/overview.md)):

- **Entry point:** `src-tauri/src/main.rs` → `sotyware_lib::run()` in
  `src-tauri/src/lib.rs`.
- **Runtime:** `run()` builds the Tauri app: single-instance plugin (first, desktop
  only) → `FeatureRegistry` from `features::builtin()` (managed state) →
  `build()` → feature preferences loaded from SQLite (managed state, before the
  event loop) → `app.run()`: window `main` from `tauri.conf.json` → `setup`
  creates the tray, picks the `OperatingMode` (`select_mode`) and starts the
  features that are enabled (stored choice, else default) → event loop.
  `RunEvent::Exit` stops the running features and closes the preferences.
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
  failed/stopping, per-feature error isolation, the `example` feature). Details
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
  shows what the events say
  ([cloud-storage.md](docs/architecture/cloud-storage.md#dashboard),
  [ADR-016](docs/architecture/decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter),
  [ADR-017](docs/architecture/decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store),
  [ADR-018](docs/architecture/decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands),
  [ADR-019](docs/architecture/decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust),
  [ADR-020](docs/architecture/decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel),
  [ADR-022](docs/architecture/decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)).
- **Request recorder (test builds only):** the off-by-default Cargo
  feature `request-recorder` adds `src-tauri/src/cloud/recorder.rs`, one
  read-only SDK interceptor per `S3Store` (installed in `S3Store::new`)
  that records the app's own S3 requests, redacted, when the
  `SOTYWARE_REQUEST_RECORD_*` variables are set. Built only into
  `--target-dir target/recorder` (build.rs), never packaged
  (`scripts/check-release-binary.mjs`, `beforeBundleCommand`)
  ([ADR-024](docs/architecture/decisions.md#adr-024-an-opt-in-request-recorder-observes-the-apps-own-s3-requests-through-an-sdk-interceptor-compiled-only-into-a-test-build),
  [cloud-storage.md](docs/architecture/cloud-storage.md#request-recorder)).
- **Frontend:** `src/App.svelte` (navigation links: Overview, Cloud Storage)
  with `src/Features.svelte` and `src/CloudStorage.svelte`
  (+ `src/CloudAccountForm.svelte`, `src/CloudBrowser.svelte`,
  `src/CloudUpload.svelte`). Its only
  access to Rust is the commands
  `operating_mode`, `list_features`, `set_feature_enabled`,
  `list_cloud_providers`, `list_cloud_accounts`, `add_cloud_account`,
  `update_cloud_account`, `remove_cloud_account`, `check_cloud_account`,
  `list_cloud_folder`, `download_cloud_file`, `cancel_cloud_download`,
  `retry_cloud_download`, `upload_cloud_files`, `cancel_cloud_upload` and
  `retry_cloud_upload`
  (one `allow-<command>` permission each in
  `src-tauri/capabilities/default.json`; typed wrappers in `src/features.ts`
  and `src/cloud.ts`) and, in tray mode only, `getCurrentWindow().hide()`
  (`core:window:allow-hide`, granted at runtime by the `tray-mode` capability in
  `setup`). No other custom command, no custom events (download progress
  and upload events use per-call Tauri channels, not events), no `dialog:` or
  `fs:` permission.

**Not implemented (planned or only discussed):** everything in
[docs/status/roadmap.md](docs/status/roadmap.md). In particular there is no
persistence other than the feature enabled/disabled choices and the cloud
storage accounts, no settings, autostart,
third-party plugin system, feature-management UI, sync, backend service, XEmbed
tray fallback, file operations in the cloud browser other than downloading
one file at a time and uploading batches into the folder shown (no overwrite,
parallel uploads, folder uploads, folder or multiple downloads, rename, delete
or folder creation), transfer queue UI or download history, persistent
record of incomplete multipart uploads, credential
import or any credential fallback outside the OS store, backups
or restic integration, cost estimator, custom event, or custom command besides
the sixteen listed above. Never describe these as existing.

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
   `CloudUploads`,
   the capability file) before
   introducing new ones. New built-in functionality is a feature in
   `src-tauri/src/features/`, not new code in `lib.rs`.
3. Avoid unnecessary dependencies.
4. Avoid speculative abstractions and premature architecture. Build what the task
   needs, nothing for hypothetical features.
5. Preserve cross-platform behaviour.
6. Never weaken, skip or delete tests or checks to make a change pass.
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

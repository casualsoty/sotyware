# Implementation status

Last updated: 2026-11-13 (Windows dialog paths with unpaired surrogates no longer end sotyware: rfd patched locally, native Windows 11 and WSL2 runs: [details](#windows-dialog-paths-with-unpaired-surrogates-2026-11-13)). Update this file whenever
implementation or validation changes. Never mark something validated that was
not run on that environment.

## Current milestone

**Milestone 0: prove the desktop runtime.** Show that the stack (Tauri 2 +
Svelte 5) and the background lifecycle (tray, hide-on-close, explicit Quit,
single instance) work on Windows 10/11 and Linux desktops, before any product
features are built.

State: implemented. Validated natively on **Windows 11** (25H2, build 26200.9711),
sotyware's primary runtime platform, by an automated test on the real desktop, the
headless suite and the NSIS-installed copy, last at `e8834fd` (application code
of `8ca3bd7`, with the Windows test fix `71442d3`;
[details](#dashboard-only-mode-regression-run-native-windows-11)). Linux is
validated only in simulated environments (WSL2/WSLg; Xvfb + openbox), never on
a real Linux desktop. Open: Windows 10, a person's visual check on Windows, the
MSI installer, and real Linux desktops. The Linux tray uses `tray-icon`'s `ksni`
backend since `12a6185`, which avoids the libayatana-appindicator registration
race ([details](#ksni-tray-backend-wsl2-xvfb--openbox-stub-tray-host)). Since
`8ca3bd7`, Linux without a reachable tray host runs in dashboard-only mode
instead of exiting 1 ([details](#dashboard-only-mode-wsl2-xvfb--openbox-stub-tray-host)).
Dashboard-only mode cannot occur on Windows, where every tray error stays fatal.
On 2026-11-10 (at `0131e09`) no real Linux desktop was available to test on
([survey](#real-linux-desktop-availability-2026-11-10)), and the Windows tests
were split into headless, private-desktop and real-desktop categories
([results](#windows-test-categories-native-windows-11-0131e09)). The built-in
feature foundation passed on native Windows 11 on 2026-11-11 (headless suite,
release build, real-desktop test 90/0 three times;
[details](#built-in-feature-foundation-native-windows-11-2026-11-11)).
Feature preferences are stored in SQLite since `93f7667` and survive real
restarts on both platforms: virtual display 115/0/0 three times, Windows 11
real desktop 99/0 three times
([details](#persistent-feature-preferences-2026-11-11)).
The Cloud Storage section (account management, connection check) passed on
2026-11-11: virtual display 119/0/0 and Windows 11 real desktop 103/0 and
private desktop 56/0, three runs each; the connection check has not been run
against a real provider
([details](#cloud-storage-dashboard-2026-11-11)).
Read-only bucket browsing passed on 2026-11-11 against the local listing
server through the UI on both platforms (Linux 17/17, Windows 29/29, several
runs) and read-only against Backblaze B2, where the prefix-restricted test key
is refused the root and its own prefix is empty
([details](#cloud-storage-bucket-browsing-2026-11-11)). With a temporary
1,040-object dataset at Backblaze, browsing passed through Rust (133/133)
and through the UI (Linux 30/30, Windows 46/46, several runs); every
run's dataset was deleted again (one only on 2026-11-12, after Backblaze's
daily transaction cap had refused its cleanup)
([details](#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11)).
On 2026-11-12 the real-desktop Windows test (`windows.ps1`) passed 103/0
three times on the tree with bucket browsing, and the dataset test lists
`many/` with far fewer billed requests
([details](#real-windows-desktop-and-fewer-backblaze-listings-2026-11-12)).
Single-file downloads passed on 2026-11-12 against the local object server
through the UI and the native save dialog on both platforms, and once
against **real Backblaze B2** through the Rust download code (WSL2, no UI):
`Content-Length` and the stored SHA-256 arrived, the file was verified
([details](#one-download-from-backblaze-2026-11-12)). The same day one file
from Backblaze was downloaded through the **complete native Windows 11 flow**
(dashboard, browser, native save dialog, real Credential Manager) and
verified byte for byte; the object was deleted again by its version id
([details](#real-backblaze-download-through-the-windows-dashboard-2026-11-12)).

## Implemented

| Feature | Where |
| --- | --- |
| Tauri 2 desktop runtime (one process, one window `main`) | `src-tauri/src/lib.rs`, `tauri.conf.json` |
| Svelte 5 dashboard (static page; "Hide dashboard" button in tray mode, a no-tray note in dashboard-only mode) | `src/App.svelte` |
| Operating modes: tray mode, and dashboard-only mode on Linux without a tray host (close quits, no hide) | `select_mode`, `close_action` in `lib.rs`; `unavailable_reason` in `tray.rs` |
| `operating_mode` command | `lib.rs`, app manifest in `build.rs` |
| Built-in feature foundation: `Feature` trait, `builtin()` list, `FeatureRegistry` (duplicate/malformed IDs rejected, lifecycle registered → disabled / initializing → running / failed, stopping → disabled; per-feature error isolation; started in `setup`, stopped on exit) | `src-tauri/src/features/` ([features.md](../architecture/features.md)) |
| Persistent feature preferences: each user switch saved in `preferences.sqlite3` (app local data folder, SQLite via `rusqlite` bundled), loaded before the event loop, stored choice or `enabled_by_default` at startup; schema versioned with `PRAGMA user_version` and transactional migrations; unusable database left untouched and reported (`storageError`), failed saves reported (`saveError`) | `features/preferences.rs`, `storage.rs`, `run()` in `lib.rs` ([features.md](../architecture/features.md#persistence), [ADR-015](../architecture/decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)) |
| Example feature `example` (enabled by default, does nothing visible) | `src-tauri/src/features/example.rs` |
| `list_features` (`{ features, storageError }`) and `set_feature_enabled` (`{ feature, saveError }`) commands, and the dashboard's minimal Features section (state and enable checkbox per feature, storage and save errors as alerts) | `features/mod.rs`, `build.rs`, `capabilities/default.json`; `src/features.ts`, `src/Features.svelte` |
| Cloud Storage section in the dashboard (navigation link; account list with empty state, add/edit form with provider presets, connection check, removal with confirmation; keys in two password inputs, sent only to Rust and never returned) and its six commands `list_cloud_providers`, `list_cloud_accounts`, `add_cloud_account`, `update_cloud_account`, `remove_cloud_account`, `check_cloud_account` (safe DTOs, structured errors, `spawn_blocking` for SQLite and the credential store); connection check `S3Store::check_bucket` (`HeadBucket`, one-key `ListObjectsV2` only after a refused HEAD, never `ListBuckets`) | `src/App.svelte`, `src/CloudStorage.svelte`, `src/CloudAccountForm.svelte`, `src/cloud.ts`, `src/cloudMessages.ts`; `src-tauri/src/cloud/dashboard.rs`, `check_bucket` in `cloud/s3.rs`, `run()` in `lib.rs`, `build.rs`, `capabilities/default.json` ([cloud-storage.md](../architecture/cloud-storage.md#dashboard), [ADR-018](../architecture/decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)) |
| Read-only bucket browser in the Cloud Storage section ("Open bucket" per account: folders, files, sizes, dates, breadcrumbs, Up, Refresh, Load more, back to the accounts) and the command `list_cloud_folder` (one `ListObjectsV2` page of one folder level of the account's configured bucket, delimiter `/`; folder prefixes kept exactly, marker objects hidden; continuation tokens kept in Rust behind cursors tied to account, bucket and prefix; structured errors) | `src/CloudBrowser.svelte`, `src/CloudStorage.svelte`, `src/cloud.ts`, `src/cloudMessages.ts`; `src-tauri/src/cloud/browser.rs`, cursors in `cloud/dashboard.rs`, `list_folder` in `cloud/s3.rs`, `build.rs`, `capabilities/default.json` ([cloud-storage.md](../architecture/cloud-storage.md#browsing-a-bucket), [ADR-019](../architecture/decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust)) |
| Single-file downloads from the bucket browser (Download per file; native save dialog opened by Rust, the page never sends a path; one at a time; progress with percent and bytes over a per-call Tauri channel; Cancel; Try download again to the same destination; completed / cancelled / failed with what was verified) and the commands `download_cloud_file`, `cancel_cloud_download`, `retry_cloud_download`; `S3Store::download` streams into a hidden temporary file next to the destination, verifies size and the stored SHA-256, then hard-links or renames it into place; `TransferControl` settles cancel against completion; quitting cancels and cleans up | `src/CloudBrowser.svelte`, `src/cloud.ts`, `src/cloudMessages.ts`; `src-tauri/src/cloud/download.rs`, `download` in `cloud/s3.rs`, `prepare_download` in `cloud/dashboard.rs`, `run()` in `lib.rs` (dialog plugin, managed state, shutdown), `build.rs`, `capabilities/default.json` ([cloud-storage.md](../architecture/cloud-storage.md#downloading-a-file), [ADR-020](../architecture/decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel)) |
| Upload panel in the bucket browser (Upload files… into the folder shown; batch and per-file progress by stage with bytes, a percentage only for a known non-zero total; per-file list with skip reasons, conflicts, errors and remote outcome notes; Cancel by batch id with `finishing`/`not-active` explained; Try again only for `retryable` files as a new batch; cleanup results apart from the files; final summary; stale events dropped by start, batch id and per-file `seq`; leaving the browser cancels) | `src/CloudUpload.svelte`, `src/CloudBrowser.svelte`, `src/cloud.ts` (`uploadFiles`, `cancelUpload`, `retryUpload`, parsers), `src/cloudMessages.ts`, `src/format.ts` ([cloud-storage.md](../architecture/cloud-storage.md#the-upload-panel)) |
| Upload batches, backend and commands: `upload_cloud_files` (native multi-file dialog opened by Rust, at most 100 files, every file checked before anything is sent and skipped with a reason if unusable, destination folder fixed when the dialog opens), `cancel_cloud_upload`, `retry_cloud_upload`; one file after another, one batch at a time in its own slot (a download can run meanwhile), `HeadObject` before each file so nothing is overwritten, per-file failures continue while credential, access, configuration and connection failures stop the batch, events with `seq` on a per-batch Tauri channel and one final summary, retry of eligible files only, incomplete multipart uploads kept in memory and aborted again on retry and on quit (bounded) | `src-tauri/src/cloud/upload.rs`, `upload_limits`/`prepare_upload` in `cloud/dashboard.rs`, `open_multipart`/`abort_incomplete_upload` in `cloud/s3.rs` and `cloud/s3/upload.rs`, `run()` in `lib.rs` (managed state, shutdown), `build.rs`, `capabilities/default.json` ([cloud-storage.md](../architecture/cloud-storage.md#uploading-files), [ADR-022](../architecture/decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)) |
| Cloud storage backend (changes to objects other than upload batches not reachable from the app): provider profiles (Backblaze B2, Hetzner, any S3-compatible), storage accounts without secrets (validation, add/update/remove, SQLite `cloud-accounts.sqlite3`), credentials by account id in the OS credential store (Windows Credential Manager, Linux Secret Service; no fallback; environment variables only in test builds), account add/rotate/remove keeping configuration and credentials consistent, one S3 adapter on `aws-sdk-s3` (list buckets/objects, single and multipart upload, verified download, delete), end-to-end SHA-256, typed errors, a transfer queue per account; the upload lifecycle `S3Store::upload` (existing key refused, cancellable until one commit point, bytes re-hashed while sent, multipart aborted on failure, `HeadObject` verification, stall timeout instead of the read timeout, throttled progress by stage) | `src-tauri/src/cloud/`, `cloud/s3/upload.rs`, `SqliteAccountStore` in `storage.rs` ([cloud-storage.md](../architecture/cloud-storage.md), [ADR-016](../architecture/decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter), [ADR-017](../architecture/decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store), [ADR-021](../architecture/decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration)) |
| System tray with "Open Dashboard" and "Quit" | `src-tauri/src/tray.rs` |
| Linux tray backend: `tray-icon`'s `ksni` (pure Rust, no libayatana-appindicator) | Linux-only `tray-icon` dependency in `src-tauri/Cargo.toml` |
| Hide-on-close lifecycle (tray mode) | `on_window_event` in `lib.rs` |
| Only explicit exits (tray Quit; close in dashboard-only mode); implicit exits blocked | `app.run` closure and `on_window_event` in `lib.rs`, `tray.rs` |
| Fatal exit (code 1) if the tray cannot be created for a reason other than a missing tray host | `setup` in `lib.rs` |
| Single instance via `tauri-plugin-single-instance` | `run()` in `lib.rs` |
| Minimal capability (`allow-operating-mode`, `allow-list-features`, `allow-set-feature-enabled`, one `allow-…` permission per cloud account command, `allow-list-cloud-folder`, the three download and the three upload permissions; no `dialog:` or `fs:` permission; `core:window:allow-hide` added at runtime in tray mode only) and CSP | `capabilities/default.json`, `setup` in `lib.rs`, `tauri.conf.json` |

Automated tests per platform: the "Hide dashboard" button with a real mouse
click, and left-click-to-open, are covered only by the Windows desktop test
(`tests/desktop/windows.ps1`). On Linux the virtual-display test activates the
button through AT-SPI and reads the dashboard-only page the same way. Restoring
a minimized dashboard and focus on second launch run on Linux only under the
virtual display (Xvfb + openbox); WSLg skips them. Test categories:
[testing.md](../development/testing.md).

Nothing else exists: no product features (only the example feature and
cloud storage accounts, browsing, single-file downloads and upload batches),
no settings, no persistence other than the feature choices and the cloud
storage accounts, no file operations in the bucket browser other than
downloading one file at a time and uploading batches into the folder shown
(no overwrite, folder or multiple downloads, rename or delete), no transfer
queue UI or download history, no backups, no sync, no custom Tauri events,
no feature-management UI, no third-party plugins, and no custom command other
than `operating_mode`, `list_features`, `set_feature_enabled`, the six cloud
account commands, `list_cloud_folder`, the three download commands and the
three upload commands.

## Validation matrix

Legend: ✅ passed · ⏭️ skipped by the environment (not a pass) · ❌ failed ·
— not run.

Each result belongs to one test category
([testing.md](../development/testing.md)): **headless** (no display, nothing
real from the OS), **virtual display** (real release binary, Xvfb + openbox, stub
tray host), **simulated desktop** (real release binary on WSLg, stub tray host) or
**real desktop** (Windows test, manual check). A pass in one category says nothing
about another.

"Re-run 2026-11-10" means the check was repeated while writing this document.
"Native Windows 2026-11-10" means it ran on the Windows 11 machine described below.
"Earlier run" means it was recorded in the original README from a run whose
script or procedure is not in the repository; it was not repeated.

### The late-part control test on a loaded machine (2026-11-13)

On `f902efb`, `ci.yml` failed on Linux (Ubuntu 26.04 runner, Rust
1.100.0): `without_the_wait_a_handed_over_part_lands_after_the_abort`
found no late `part-2` (332 passed, 1 failed). Reproduced on WSL2 by
running the whole `cargo test --lib` pinned to 2 CPUs (`taskset -c 0,1`):
failed 3 of 3. Cause (test code only, no production change): the client
handed over all of part 2 and closed cleanly, the `Pace::Buffered` relay
forwarded every byte and then half-closed its upstream connection; hyper's
server (half-close off) took that EOF as the connection closing and
dropped s3s-fs's `upload_part` before it moved the part into place. Idle,
the debug-build handler finished first. The relay now keeps the upstream
write side open after its client left, waits up to 60 s for the answer,
then closes. WSL2 after the change: pinned to 2 CPUs the control passed
11 of 11 full-suite runs, the three ADR-025 tests pinned to 1 CPU 8 of 8;
`cargo test` 333 passed, 26 ignored, 3 runs; feature build with every
store recorded 360; fmt and clippy clean in both configurations. Not yet
run on CI or Windows. Seen meanwhile, unrelated (no `ThrottledLink`):
`cloud::upload::tests::cancelling_during_a_single_put_stores_nothing`
reports its cancelled file at stage `hashing` instead of `sending` when
CPU is short: full suite pinned to 2 CPUs 1 of 10 runs before the change
and 8 of 14 after it (the control now stores its 5 MiB part, more load);
`cloud::upload::tests` alone pinned to 1 CPU 10 of 10; never seen
unpinned or on CI.

### Late-part wait and the remaining recorder rehearsals on native Windows 11 (2026-11-13)

Tree `ad6f69c` + test code then uncommitted, committed afterwards
unchanged as `bf991d0` (`-Scenario settle` in
`windows-cloud-ui.ps1`; `SOTYWARE_UI_UPLOAD_PACE=buffered` and
deadline pacing of `Pace::Buffered` in `test_server.rs`; no production
change). Copy `C:\dev\sotyware-recorder`, recorder build SHA-256
`0ae6f848…d64061` (built from `ad6f69c`), Windows 11 Home 25H2 build
26200.9711, real desktop, local in-process server only, every run ended
with the tray Quit (real mouse). **No real provider.**

- Rehearsals not yet run on this build, one run each, `-RecordRequests`:
  `download` **40/40** (recorder 20 = server 20; the save-dialog step
  passed), `upload` **78/78** (104 = 104, 5 dropped, helper's 293
  excluded), `b2-lifecycle -Rehearse` `multipart` **42/42** (17 = 17),
  `retry` **42/42** (19 = 19), `leave` **38/38** (9 = 9, 1 dropped).
  `cancel` and `quit` had passed on this build before (39/39 each). In
  every recording: start and end line, `accounting: complete`, sequence
  numbers without gaps, every operation ended or dropped, attempts
  without an answer exactly those of dropped operations; no key, secret,
  endpoint, signature, object key or body; bucket `configured`, keys,
  prefixes and upload ids hashed. Buckets empty, no multipart upload left.
- None of these reached the ADR-025 wait: every cancelled part (upload
  mode 4, cancel, quit) was still being handed over (dropped 1.0–13.3 s
  into a part of about 16 s), the abort 0–5 ms later.
- `-Scenario settle` (new, [testing.md](../development/testing.md#upload-lifecycle-through-the-windows-dashboard-at-backblaze)):
  first run **failed, 38 passed, 4 failed** (not a validation): the
  cancel came right after the percentage showed part 3 handed over (part
  3 started 431 ms before the press), the app waited the 5 s bound (part
  3 `op-dropped` 5,054 ms after the press, abort 204 then), and the
  part's file (`.upload_id-….part-3`, s3s-fs) was still written after
  the abort, so the two "no multipart upload left" checks and two settle
  checks failed; parts took about 16 s because the relay's per-piece
  sleep is capped by Windows' ~15.6 ms timer. That file was removed only
  with the whole temporary server folder when the helper exited (exit 0,
  checked by the run); afterwards (reviewed the same day) `%TEMP%` held
  no `sotyware-s3-test-*` folder and no file of that upload id, the
  run's Credential Manager entry was gone and the account database was
  back. sotyware's own line for the bound ("part 3 got no answer within
  5 s …") was not seen: the Windows driver does not keep the app's
  stderr. With deadline pacing in `Pace::Buffered` (parts about 2.7 s):
  **42/42 in 3 consecutive runs**; each time part 3 was in flight at the
  press (started 335–419 ms before it), answered 200 at +2,242 to
  +2,336 ms, abort 204 0 ms after that answer, 12 = 12 app requests, 0
  dropped, no part left. The recording marks request start and answer,
  not the moment a body was handed over completely; that part follows
  from the buffering link and the percentage.
  WSL2 after the pacing change: `cargo test` 333 passed, 26 ignored; the
  `cloud::s3::upload` tests (with the `Buffered` ones) 37 passed, 3 runs;
  `probe_a_part_landing_after_the_abort` 0 of 3; fmt and clippy clean in
  both configurations.

### A cancelled part is awaited before the abort (2026-11-13)

Uncommitted tree on `1e50821` (ADR-025, with the request recorder
changes). **No real provider.**

- New tests in `cloud/s3/upload/tests.rs` against s3s-fs behind a link that
  buffers the body and forwards it at 4 MiB/s (`Pace::Buffered`), 3 runs
  each: a cancel after part 2 was handed over completely waits about 1.3 s
  for its answer and the abort leaves no part; the control with
  `part_settle_timeout` zero leaves the late `part-2` every time; with a
  1 s bound at 1 MiB/s the wait ends at the bound. The opt-in probe
  `probe_a_part_landing_after_the_abort`: **0 of 3** trials left a part
  (1 of 3 before the change).
- `cargo test` **333 passed, 26 ignored**, 3 runs (9 s); feature build 360;
  with every store recorded 360, 3 runs (after making the hook
  assumption's cancel deterministic: it used a 700 ms timer, which under
  load could also fall on a completely handed-over part and wait the
  bound); fmt and clippy clean in both configurations; `npm run
  test:headless` passed.
- Release binaries rebuilt (normal `b984b28c…`, recorder `f9fe442a…`):
  Linux virtual display `cloud-ui.sh download upload` normal **75/75**,
  recorder **77/77** (21 = 21, 92 = 92 app requests); the upload phase now
  has 4 dropped operations and 87 answered attempts instead of 5 and 86:
  one cancelled part that was already handed over is now answered before
  the abort.
- **Native Windows 11 25H2** (copy `C:\dev\sotyware-recorder`, rebuilt
  recorder build SHA-256 `0ae6f848…d64061`, real desktop, local server,
  after the user's approval; tray Quit with the real mouse):
  `windows-cloud-ui.ps1 -Mode b2-lifecycle -Rehearse -RecordRequests`
  `-Scenario cancel` **39/39** (11 = 11 app requests) and `-Scenario
  quit` **39/39** (12 = 12; exit after the tray Quit in 1.8 s; no part
  left), one run each. In both the cancel came while part 2 was still
  being handed over (cancel: dropped after 11.7 s, quit: after 13.3 s;
  part 1 took 15.9 s; abort 0 ms later, no wait), so the new wait itself did **not** run on
  Windows in these runs; it did later, in `-Scenario settle`
  ([below](#late-part-wait-and-the-remaining-recorder-rehearsals-on-native-windows-11-2026-11-13)).

### Request recorder (2026-11-13)

Uncommitted tree on `1e50821` (ADR-024 implemented; test code, a guard
script and docs; no change to upload, download, browse or account
behaviour). **No real provider was contacted.** WSL2 Ubuntu 24.04 unless
noted.

- **SDK assumptions first** (`cloud/s3/hook_assumptions.rs`, 6 tests,
  locked `aws-smithy-runtime` 1.10.0 / `-api` 1.11.3, in-process server):
  both critical ones hold (state in `interceptor_state()` is dropped with
  a dropped future, its `Drop` runs without `read_after_execution`; the
  client interceptor runs for uploads with their per-operation override);
  hook order for success, a 403, a 500-then-404 retry and a drop as
  expected; typed input at `read_before_execution`; status and attempt
  numbers readable. One assumption was wrong: the operation `Metadata` is
  not there yet at `read_before_execution` (deviation in ADR-024).
- **Headless:** `cargo test` (default build, no recorder) **330 passed, 25
  ignored** (324 before + 6 assumption tests); `cargo test --features
  request-recorder --target-dir target/recorder` **357 passed** (27 new:
  26 in `cloud/recorder/tests.rs`, 1 quit test in `cloud/upload/tests.rs`;
  the recorder tests 3 runs in a row); the same with
  `SOTYWARE_RECORDER_TEST_ALL=1` (every store of the test process
  recorded) **357 passed**; `cargo fmt --check` and `cargo clippy
  --all-targets -- -D warnings` clean in both configurations;
  `npm run check:release-binary` ok; its self-test 13/13.
- **Builds:** normal release (`--no-bundle`) SHA-256 `1e4140a3…6d66644`
  contains none of `sotyware-request-recorder`, `request-recorder`,
  `SOTYWARE_REQUEST_RECORD`; the recorder release build
  (`target/recorder`) `2438ff77…227e23b` contains each once and is refused
  by `check-release-binary.mjs`. `build.rs` refuses the feature in the
  default target directory. Real `tauri build`: `--bundles deb --features
  request-recorder -- --target-dir target/recorder` **refused** in
  `beforeBundleCommand` before Tauri patched or bundled anything; a normal
  `--bundles deb` passed the guard and bundled (that run overwrote the
  existing `target/release/bundle/deb/*.deb` build output; the normal
  binary was relinked afterwards, byte-identical, `1e4140a3…`). A copy of
  the recorder binary patched the way Tauri marks a deb bundle refused to
  record (no file; stderr "this binary was packaged"); the unpatched copy
  recorded, and when killed with SIGTERM left only the `start` line
  (incomplete, as designed).
- **Linux virtual display** (`cloud-ui.sh download upload` with
  `SOTYWARE_CLOUD_UI_RECORD=1` and the recorder build, native GTK dialogs,
  local servers): **77/77, 3 runs**. Recorder versus server, every run the
  same: download 1 file, 17 operations, **21 attempts sent = 21 app
  requests on the server** (ListObjectsV2 8, GetObject 13), 15 answered;
  upload 1 file, 92 operations (5 dropped), **92 sent = 92 app requests**
  (HeadObject 52, PutObject 16, ListObjectsV2 10, UploadPart 6,
  AbortMultipartUpload 4, CreateMultipartUpload 3, GetObject 1), 86
  answered; the helper's own 245 requests (its own key) were not in the
  recording; every recording complete, no key or secret in it.
  Regression with the normal binary: **75/75**, server counts identical to
  the recorder runs.
- **Native Windows 11 25H2** (copy `C:\dev\sotyware-recorder`, recorder
  build SHA-256 `55ed7cf1…70be75`, real desktop, local servers,
  `windows-cloud-ui.ps1 -RecordRequests`; after the user's approval, since
  a recording run ends with the tray Quit, real mouse):
  - First two `-Mode download` runs, before the tray-Quit ending: run 1
    **28 passed, 11 failed** (the save dialog's file name step failed while
    the Linux runs loaded the machine, the rest cascaded); run 2 all 36 UI
    checks passed, **3 failed** because the driver killed the app
    (`Stop-Process`), so the recording had no `end` line and was reported
    as "count unknown", not as a match. The driver now ends a recording run
    with the tray Quit.
  - With the tray Quit: `download` **35 passed, 5 failed** (the save dialog
    of the slow download stayed open, the same step that passed before;
    recorder complete, **12 = 12** app requests), rerun **40/40**
    (**20 = 20**); `upload` **78/78**, two recording files (the mode quits
    and relaunches), **104 = 104** app requests (HeadObject 59, PutObject
    18, ListObjectsV2 12, UploadPart 6, AbortMultipartUpload 4,
    CreateMultipartUpload 3, HeadBucket 1, GetObject 1), the helper's own
    293 requests excluded.
  - `b2-lifecycle -Rehearse` (the scenarios of the Backblaze lifecycle
    run, local server): multipart **42/42** (17 = 17), cancel **39/39**
    (11 = 11, 1 dropped), retry **42/42** (19 = 19), leave **38/38** (9 = 9,
    1 dropped), quit **37 passed, 2 failed** then rerun **39/39** (12 = 12
    both times). The failed quit run: the recording shows part 2 in flight
    for 14.5 s (part 1 took 14.7 s) when quitting dropped it, and the abort
    answered 204 3 ms later; s3s-fs then still wrote the part's file from
    the bytes the throttled link had buffered, so a part file of the
    aborted upload remained. The app sent exactly the expected requests;
    this is the "received or not: unknown" case of `op-dropped`, here a
    race in the local test setup (whether a real provider can keep a part
    that arrives after the abort was not checked). Reproduced on WSL2 the
    same day by the opt-in probe `probe_a_part_landing_after_the_abort`:
    1 of 3 trials left a part file while sotyware returned
    `NothingStored`
    ([limitations.md](limitations.md#a-part-can-land-after-the-multipart-abort)).
  - Every Windows recording without the kill: complete, no key, secret or
    bucket name; every comparison: every answered attempt reached the
    server, and the server saw nothing the recorder did not see sent.

### Upload lifecycle at Backblaze on native Windows 11 (2026-11-13)

Uncommitted tree on `4d9d9a7` (the prepared lifecycle tests below), after
the user's approval of all five scenarios. **Each scenario passed once at
Backblaze** through `windows-cloud-ui.ps1 -Mode b2-lifecycle -Scenario
<scenario> -ConfirmB2Lifecycle lifecycle-ui-validation -ExpectSha256
5e364f2a…94f5e1f`: release binary with the release settings (64 MiB
threshold, 16 MiB parts), native Windows 11 25H2 build 26200.9711, real
desktop, native multi-file dialog, real Credential Manager; app key
`b2-upload.xml` (no file name prefix), prepare/probe/verify/cleanup with the
write key through the run scope (test program SHA-256 `6e8d7706…d043dd`);
bucket region `eu-central-003`. One run each, first attempt, in this order:

| Scenario | Checks | What Backblaze held before cleanup | Cleanup |
| --- | --- | --- | --- |
| `multipart` | **35/35** | mid-upload probe: exactly **one open multipart upload, for `multipart.bin`**; afterwards anchor + `multipart.bin` (75,497,475 bytes), one version each; downloaded `sha256-verified`, byte-equal (1.35 s); no open multipart upload | 2 versions deleted by id; 0 versions, 0 multipart uploads left |
| `cancel` | **32/32** | Cancel upload at 22 % of 144 MiB: "Cancelled during its upload", "Nothing was stored"; only the anchor; **no open multipart upload** (the app's own abort worked with its key) | 1 version deleted; 0 / 0 left |
| `retry` | **35/35** | locked file failed, only it offered and retried; anchor (its own version, not overwritten) + `a-first.txt`, `b-locked.txt`, `c-third.txt`, **one version each** (nothing sent twice); all three `sha256-verified` | 4 versions deleted; 0 / 0 left |
| `leave` | **31/31** | ← Back to accounts at 14 % of the 60 MiB PUT; only the anchor (the cut-off PUT stored nothing, `b-away.txt` never started); slot free afterwards | 1 version deleted; 0 / 0 left |
| `quit` | **32/32** | tray Quit (real mouse) at 22 %: exit 0 in **1.4 s**, never shown as finished; only the anchor; **no open multipart upload** (aborted on quit); relaunch fine | 1 version deleted; 0 / 0 left |

Write-key requests exactly as planned (multipart 13, cancel 10, retry 16,
leave 10, quit 10; nothing refused by the run scope, every DELETE by
version id). The app's own requests were not recorded at Backblaze (the
release binary has no recorder; counts known from the rehearsals); the
outcomes above come from the provider's listings. Credential Manager and
the account database were as before after each run; local files removed.
The 72 MiB multipart file reached Backblaze in about 5 s (fast link), so
the 20 % triggers fired with a few seconds to spare.

**Not covered here:** needs-inspection and a refused abort (need a
misbehaving provider; local only); hiding during a batch; Linux; Windows
10; Hetzner. Each scenario ran once.

### Upload lifecycle tests at Backblaze: prepared (2026-11-13)

Uncommitted tree on `4d9d9a7`. **Prepared and rehearsed locally only;
nothing was sent to Backblaze or any other provider here.** (Run
afterwards: [above](#upload-lifecycle-at-backblaze-on-native-windows-11-2026-11-13).) New
`tests\desktop\windows-cloud-ui.ps1 -Mode b2-lifecycle -Scenario
multipart|cancel|retry|leave|quit` (one scenario per run, each under its
own prefix, release binary with its release settings) and its provider
side in `cloud/integration/upload/lifecycle.rs` (prepare, probe, verify,
cleanup; design, requests and safeguards:
[testing.md](../development/testing.md#upload-lifecycle-through-the-windows-dashboard-at-backblaze)).
Production code is unchanged.

Why: the local UI tests never completed a multipart upload with the
release settings (64 MiB threshold, 16 MiB parts; every 72 MiB UI file
was cancelled, refused or quit), and the Backblaze batch test uses 6 MiB /
5 MiB parts, so the release binary's multipart path had not completed
anywhere through the UI.

Native Windows 11 25H2 build 26200.9711, real desktop, copy
`C:\dev\sotyware-lifecycle`, release binary SHA-256
`5e364f2a…94f5e1f`, rehearsals against the in-process s3s-fs server at
2 MiB/s (one run each, after two fixes to the driver's own checks):

| Scenario | Checks | App requests (by operation) |
| --- | --- | --- |
| `multipart` | **38/38** (one open multipart upload seen while sending; 5 parts; Uploaded and verified; percentages 0 → 100, never down) | `HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 4, `CreateMultipartUpload` 1, `UploadPart` 5, `CompleteMultipartUpload` 1 |
| `cancel` | **35/35** (cancelled at 20 %, nothing stored, no multipart upload left) | `HeadBucket` 1, `ListObjectsV2` 4, `HeadObject` 2, `CreateMultipartUpload` 1, `UploadPart` 2, `AbortMultipartUpload` 1 |
| `retry` | **38/38, twice** (locked file failed, only it retried; the two uploaded files and the skipped anchor not sent again) | `HeadBucket` 1, `ListObjectsV2` 4, `HeadObject` 11, `PutObject` 3 |
| `leave` | **34/34** (Back to accounts at 10 %, nothing stored, `b-away.txt` never started, slot free) | `HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 2, `PutObject` 1 (cut off) |
| `quit` | **35/35** (tray Quit with the real mouse at 20 %, exit 0 in 1.3 s, never shown as finished, aborted, relaunch fine) | as `cancel`, `ListObjectsV2` 5 |

The first `multipart` run failed 2 checks, both in the driver: it counted
s3s-fs's two files per open upload as two uploads, and expected 3
`HeadObject` where the adapter sends 4 (it checks the key again just
before `CompleteMultipartUpload`). Both fixed; the app was not changed.
Regression: `-Mode b2-upload -Rehearse` 41/41; the gates (no or wrong
`-ConfirmB2Lifecycle`, no `-Scenario`, `-Scenario` with another mode)
exit 2 before anything runs; the four opt-in Rust tests refuse without
`SOTYWARE_IT_B2_UPLOAD_CONFIRM` ("nothing was sent").
`check-ps-variables.ps1` and the PowerShell parser: clean. Windows: `npm
run check`, release build, fmt, clippy, `test:headless` (Vitest 135, Rust
314 passed, 25 ignored). WSL2: fmt, clippy, `npm run check`,
`test:headless` (Vitest 135, Rust 324 passed, 25 ignored; 5 new unit tests
in `lifecycle.rs`). The full local `-Mode upload` was not rerun (its code
paths were not changed). Not run: anything at Backblaze, Linux, Windows 10.

### Dashboard upload at Backblaze on native Windows 11: rerun passed (2026-11-13)

**`windows-cloud-ui.ps1 -Mode b2-upload` passed, 38/38, once**, after
explicit approval, run id `20261113054909817` (06:49 local), on its
first try; the rerun of the [first run](#dashboard-upload-at-backblaze-on-native-windows-11-2026-11-13)
with the verify test's request count fixed. Same machine (Windows 11 Home
25H2, build 26200.9711, x64, real desktop), same release binary
`d17382a13060091e3a41282f0533814aa1b73f98a6049a4e8bfbdd91a0785d0d`
(`-ExpectSha256`), test program SHA-256
`0481bd3f4d932d01be49510c2a2fed83d7edcc0f86e56722f30ce23bd0ebb8a8`
(unchanged before the prepare and after the verification), app key
`b2-upload.xml`, write key for prepare, verify and cleanup. **This is the
first real-provider result for the dashboard's upload path** (release
binary, native Windows multi-file dialog, page, commands, batch, S3
adapter); it is separate from the Rust batch results at Backblaze and from
the local dashboard runs.

- Prepare: run prefix empty; seed `existing.txt` (58 B) uploaded; 5
  requests, nothing refused.
- Dashboard: Connected; root → `sotyware-it` → `upload-ui-validation` →
  run folder (only the seed); destination named before Open; "Upload
  finished. 4 files: 3 uploaded, 1 skipped."; three "Uploaded and
  verified", `existing.txt` "Skipped: already in this folder … left as it
  is"; no Try again; `progress.bin` Sending 0, 39, 77, 100 % and Verifying
  seen; no local path in any text; Show destination folder listed the four
  objects, `existing.txt` 58 B. Account removed; Credential Manager and
  account database as before.
- Verify: each key downloaded once, `Sha256Verified`, size, SHA-256 and
  **every byte equal** to the local file (the seed for `existing.txt`);
  exactly four keys, one version each, no delete marker, `existing.txt`
  still the seed's version id; no incomplete multipart upload; 12 requests
  (4 `GetObject`, 4 listings, 4 `DeleteObject` by version id → 204).
- Cleanup: afterwards 0 versions and 0 multipart uploads under the run
  prefix; local files removed.
- App requests at Backblaze were not recorded (no recorder in the release
  binary); the rehearsal counted 19 for the same steps.
- **Not run:** cancel, retry, multipart and the browser lifecycle at
  Backblaze (local only, on purpose); Linux; Windows 10; Hetzner.

### Dashboard upload at Backblaze on native Windows 11 (2026-11-13)

One run of `windows-cloud-ui.ps1 -Mode b2-upload` after explicit approval
(run id `20261113054534091`, 06:45 local), first attempt, on the real
Windows 11 desktop (Home 25H2, build 26200.9711, x64), release binary
`C:\dev\sotyware-b2upui\src-tauri\target\release\sotyware.exe` SHA-256
`d17382a13060091e3a41282f0533814aa1b73f98a6049a4e8bfbdd91a0785d0d` (checked
by `-ExpectSha256`), test program SHA-256 `04944d20…29e1b5f` (unchanged
before the prepare and after the verification). App key: the new
`b2-upload.xml` (no file name prefix); prepare, verify and cleanup: the
write key. **37 passed, 1 failed. The failure is a defect in the test's
own request count, not in sotyware; no rerun yet.**

- **Prepare:** run prefix empty (versions and multipart uploads); seed
  `existing.txt` (58 B) uploaded; 5 requests as planned, nothing refused.
- **Dashboard:** account added through the form, Connected; browsed root →
  `sotyware-it` → `upload-ui-validation` → the run folder (only the seed
  shown); the panel named the run folder before Open was pressed in the
  native dialog; one batch: "Upload finished. 4 files: 3 uploaded, 1
  skipped."; `a-small.txt`, `größe +test.bin`, `progress.bin` (24 MiB) each
  "Uploaded and verified", `existing.txt` "Skipped: already in this folder
  … left as it is"; no Try again; `progress.bin` seen Sending at 0, 45 and
  100 % (only 10 polls: the upload was fast); no local path in any text;
  Show destination folder listed the four objects, `existing.txt` with the
  seed's 58 B. Account renamed, key replaced and removed; Credential
  Manager and the account database as before.
- **Provider side (verify):** each of the four keys downloaded once and
  reported `Sha256Verified` with the expected size (the stored SHA-256
  equals the received bytes' hash); the version listing found exactly the
  four keys, one version each, no delete marker, sizes 77 / 58 / 200,003 /
  25,165,829, and `existing.txt` still the seed's version id (not
  overwritten); no incomplete multipart upload. 12 requests, exactly as
  planned.
- **Cleanup:** 4 `DeleteObject` by version id → 204; afterwards 0
  versions and 0 multipart uploads under the run prefix. Local files
  removed.
- **The failed check:** `backblaze_b2_upload_ui_verify` did not print its
  final "passed" line. Its request count looked for `GET` requests whose
  path contains `/sotyware-it/`; the test bucket's own name ends in
  `sotyware-it` (checked without printing it), so the four listings
  matched too (8, not 4). This assertion comes after the cleanup and after
  the found-versions checks, which the printed listing satisfies. The
  driver discarded the test's stderr, so the panic message itself was not
  seen, and the byte-for-byte comparison of each download (asserted after
  its `Sha256Verified` line) is therefore **not shown** by this run. Fixed:
  the count now matches `x-id=GetObject`; the driver keeps stderr for this
  mode; the local test uses a bucket name ending in `sotyware-it`. WSL2
  `cargo test` 319 + 21 ignored; Windows 309 + 21 ignored; new test
  program SHA-256 `0481bd3f…ebb8a8`.

The rerun with the fixed test program passed
([above](#dashboard-upload-at-backblaze-on-native-windows-11-rerun-passed-2026-11-13)).

### Dashboard upload test at Backblaze: prepared (2026-11-13)

Uncommitted tree on `48c33c4`. **Prepared and validated locally only;
nothing was sent to Backblaze or any other provider.** New
`tests\desktop\windows-cloud-ui.ps1 -Mode b2-upload` (one batch through
the release binary, its bundled page and the native Windows multi-file
dialog into the B2 test bucket) with a local `-Rehearse` variant, and its
provider side in `cloud/integration/upload/ui.rs` (prepare, verify, cleanup;
design, requests and safeguards:
[testing.md](../development/testing.md#uploads-through-the-windows-dashboard-at-backblaze)).
Production code is unchanged. Test code changed elsewhere: the batch test's
run scope and cleanup take their list of file names; `serve_uploads_for_ui`
can be seeded with one given file; `Pick` in the driver can cancel the
dialog instead of pressing Open. **The dashboard is not validated at a real
provider yet**: the three upload results stay separate (Rust batch path at
Backblaze; dashboard against the local server; dashboard at Backblaze: not
run).

- **WSL2** (Ubuntu 24.04, cargo 1.99.1): `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, `npm run check` clean; `npm run
  test:headless`: rfd guard ok, Vitest 135, Rust **319 passed, 21 ignored**.
  New: `upload_ui_run_against_the_test_server` (seed, the dashboard's batch
  through the code behind `upload_cloud_files`, verification downloads, all
  through the run scope; 13 batch requests, the skipped file `started:
  false`) and three rule tests; three new opt-in tests, `#[ignore]`d. The
  prepare test without `SOTYWARE_IT_B2_UPLOAD_CONFIRM` failed at once with
  "nothing was sent".
- **Native Windows 11** (Home 25H2, build 26200.9711, x64; fresh copy
  `C:\dev\sotyware-b2upui`, real desktop): release binary
  `src-tauri\target\release\sotyware.exe` SHA-256 `d17382a1…785d0d`
  (bundled page; nothing listened on port 1420); fmt, clippy,
  `npm run check` clean; `test:headless` Vitest 135, Rust 309 passed, 21
  ignored; the same prepare guard. Against local servers only, one run
  each: **`-Mode b2-upload -Rehearse` 41/41, twice** (navigation to the run
  folder; destination named before Open; native dialog; "Upload finished. 4
  files: 3 uploaded, 1 skipped."; `progress.bin` seen Sending 0 → 100 %
  in 67 samples; no local path in any text; Show destination folder lists
  the four objects with the seed's size for `existing.txt`; the server held
  exactly the four keys with matching content and SHA-256 metadata; the app
  sent exactly `HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 10,
  `PutObject` 3). Regression: `-Mode upload` **74/74** (moved the mouse
  once, tray Quit), `download` 36/36, `browse` 29/29, `local` 19/19.
  Credential Manager and `%TEMP%` unchanged afterwards, account database
  put back. Gates: `b2-upload` without or with a wrong `-ConfirmB2Upload`,
  `-Rehearse` with another mode, and a wrong `-ExpectSha256` each stopped
  with exit 2 before anything ran.
- **Not run** in this preparation: anything at Backblaze (the run
  followed after approval, [above](#dashboard-upload-at-backblaze-on-native-windows-11-2026-11-13));
  Linux (no Linux variant of this mode); Windows 10.

### Uploads at Backblaze on native Windows 11 (2026-11-13)

Tree `96c9461` (no change to code or tests; a fresh copy of the clean
tree at `C:\dev\sotyware-b2upwin`). **`backblaze_b2_upload_batch` passed
once against the real B2 test bucket on native Windows 11**, first
attempt, 4.18 s, after explicit approval. This is a separate result from
the [WSL2 run](#uploads-at-backblaze-2026-11-13); neither stands in for
the other. Same Rust batch path (`begin_upload`, `CloudUploads`,
`CloudDownloads`), not the UI or a file dialog.

- **Environment:** Windows 11 Home 25H2, build 26200.9711, x64, not
  elevated; Windows PowerShell 5.1 (64-bit process), no WSL variables, no
  container. Rust 1.99.1 `x86_64-pc-windows-msvc`. The test program
  `target\debug\deps\sotyware_lib-dee1785c274cb1ee.exe` (native x64 PE,
  `test` profile as on WSL2, built with `--locked`), SHA-256
  `66ce9ebe43f2c333e668310b927fc82342daedd444302e8e93579533cc885842`; the
  run script refused to start unless the program still had that hash, and
  ran it directly from `src-tauri\` (as `cargo test` does). This is a test
  program, not the app's release binary.
- **Account:** the same bucket (`eu-central-003`) and the same write key,
  limited to `sotyware-it/`, as the WSL2 run (compared by hashes of the
  bucket name and key id; neither shown), read from the DPAPI file
  `b2.xml` instead of `b2.env`. Run prefix
  `sotyware-it/upload-validation/1794546792148463900-35800/`, new; the
  preflight found it empty.
- **Local checks first, on the same machine, without any `SOTYWARE_*`
  variable:** `cargo fmt --check`, `cargo clippy --locked --all-targets --
  -D warnings`; `cloud::integration::upload` 7 passed, 2 ignored; the
  Backblaze test forced with `--ignored` but without the confirmation
  stopped before building a client ("nothing was sent"); full lib suite
  305 passed, 18 ignored.
- **41 requests, the same sequence and counts as on WSL2**, all inside
  the run scope (nothing refused):

  | Request | Count |
  | --- | --- |
  | `ListObjectVersions` / `ListMultipartUploads` of the run prefix | 3 / 3, all 200 |
  | `HeadObject` | 18 |
  | `GetObject` | 3, all 200 |
  | `PutObject` | 2, both 200 |
  | `CreateMultipartUpload` / `UploadPart` / `CompleteMultipartUpload` / `AbortMultipartUpload` | 2 / 5 / 1 / 1 (204) |
  | `DeleteObject` with `versionId` | 3, all 204 |

- **Batch 1** completed: `empty.bin`, `größe +test.bin` (sent as
  `gr%C3%B6%C3%9Fe%20%2Btest.bin`, read from an NTFS file of that name)
  and `multipart.bin` (three `UploadPart`s → 200). Each upload: two
  `HeadObject`s → 404, then a verifying `HeadObject` by version id → 200;
  `multipart.bin` one more `HeadObject` → 404 before
  `CompleteMultipartUpload`. Sizes and `sotyware-sha256` matched.
- **Existing key:** two `HeadObject`s → 200, no `PUT`; the skipped file
  reported `started: false`, `destination-exists`; `upload_file` returned
  `AlreadyExists`.
- **Cancellation:** `cancelled.bin` part 1 → 200, part 2 cut off (no
  status), `AbortMultipartUpload` → 204, `HeadObject` → 404; remote state
  `nothing-stored` after 5,242,880 bytes.
- **Downloads:** three `GetObject`s, each `sha256-verified`, SHA-256 and
  bytes equal to the source.
- **Cleanup:** the multipart listing found none; the version listing
  found exactly the three versions, no delete marker; each deleted by
  version id (204). Afterwards `versions Ok([])`, `multipart uploads
  Ok([])`.
- **Same at the provider as on WSL2:** `ListMultipartUploads` of the run
  prefix accepted; the completed multipart object's version id equals its
  upload id. **No platform-specific difference** was seen in requests,
  statuses or results.
- **Not run:** the UI and native dialogs at a real provider; Windows 10;
  Hetzner; `backblaze_b2_upload_cleanup` (not needed). One run only.

### Uploads at Backblaze (2026-11-13)

Tree `2828582` plus uncommitted doc-comment and documentation changes (no
code change). **`backblaze_b2_upload_batch` passed once against the real
B2 test bucket** (`eu-central-003`, write key limited to `sotyware-it/`),
run by the user on **WSL2** with the command in
[testing.md](../development/testing.md#uploads-at-backblaze), first
attempt, 4.44 s. Run prefix
`sotyware-it/upload-validation/1794546057268171887-144403/`. This is the
Rust batch path (`begin_upload`, `CloudUploads`, `CloudDownloads`), not the
UI or a file dialog.

- **41 requests, exactly as planned**, all inside the run scope (nothing
  refused):

  | Request | Count |
  | --- | --- |
  | `ListObjectVersions` / `ListMultipartUploads` of the run prefix | 3 / 3, all 200 |
  | `HeadObject` | 18 |
  | `GetObject` | 3, all 200 |
  | `PutObject` | 2 |
  | `CreateMultipartUpload` / `UploadPart` / `CompleteMultipartUpload` / `AbortMultipartUpload` | 2 / 5 / 1 / 1 (204) |
  | `DeleteObject` with `versionId` | 3, all 204 |

- **Batch 1:** `empty.bin`, `größe +test.bin` (sent as
  `gr%C3%B6%C3%9Fe%20%2Btest.bin`) and `multipart.bin` (three
  `UploadPart`s) completed. Each upload began with two `HeadObject`s → 404
  (the batch's, then `S3Store::upload`'s) and ended with a verifying
  `HeadObject` by version id → 200. `multipart.bin` had a third
  `HeadObject` → 404 after its last part, before
  `CompleteMultipartUpload`. Sizes and `sotyware-sha256` matched.
- **Existing key:** two `HeadObject`s of `größe +test.bin` → 200 (the
  batch's skip, then `S3Store::upload_file`'s refusal); no `PUT`, so it
  was not overwritten. The skipped file reported `started: false`.
- **Cancellation:** `cancelled.bin` part 1 → 200; part 2 was cut off by
  the cancellation (no status recorded); `AbortMultipartUpload` → 204;
  `HeadObject` → 404.
- **Downloads:** three `GetObject`s, each `sha256-verified`, with SHA-256
  and bytes equal to the source.
- **Cleanup:** the multipart-upload listing found none, so nothing was
  aborted. The version listing found exactly three versions, one per
  uploaded key, under the keys as uploaded, with no delete marker and none
  of `cancelled.bin`; each was deleted by version id. Afterwards both
  listings were empty (`versions Ok([])`, `multipart uploads Ok([])`).
- **Learned at the provider:**
  - Backblaze accepts `ListMultipartUploads` limited to the run prefix
    with the prefix-restricted key. No earlier test had used it.
  - The version id of the completed multipart object equals its upload id
    (B2's large-file id).
- **Not run:** the UI and the native dialogs at a real provider; native
  Windows 11 for this test (run later the same day:
  [above](#uploads-at-backblaze-on-native-windows-11-2026-11-13)); Hetzner;
  `backblaze_b2_upload_cleanup` (not needed). One run only.

### Upload test at Backblaze: prepared (2026-11-13)

Uncommitted tree on `3e72226`. **Prepared and validated locally only;
nothing was sent to Backblaze or any other provider.** New opt-in
`backblaze_b2_upload_batch` and `backblaze_b2_upload_cleanup` in
`cloud/integration/upload.rs` (scope, requests, cleanup and how to run:
[testing.md](../development/testing.md#uploads-at-backblaze)). Production
code is unchanged. The only additions outside the test module are the
test-only `Accounts::set_connected` / `CloudAccounts::set_connected`
(`#[cfg(test)]`), and three helpers of `integration/download.rs` made
visible to their sibling.

- **WSL2** (Ubuntu 24.04, cargo 1.99.1): `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` clean; `cargo test`
  **315 passed, 18 ignored** (7 new, 2 new ignored; 308 + 16 before).
  `upload_scenario_against_the_test_server` passed in every run (8 runs). It drives the
  real batch path against the in-process server: one batch of the empty
  file, the Unicode name and a 3-part multipart upload; the existing key
  skipped and refused; a multipart upload cancelled in part 2 and aborted;
  three downloads verified by SHA-256 and byte comparison. 32 requests, all
  inside the run scope.
  `run_scope_refuses_requests_outside_the_run`: six out-of-scope calls are
  refused before sending (none reached the server), and the SDK's real
  listings of the run prefix pass.
- **Gate checked:** with the account variables unset, running both B2
  tests with `--ignored` but without `SOTYWARE_IT_B2_UPLOAD_CONFIRM` failed
  at once with "nothing was sent". CI and `test:headless` never pass
  `--ignored`, and no workflow sets a `SOTYWARE_*` variable or secret.
- **Found and fixed:** a file skipped because its key exists reported
  `started: true` (the batch marks a file started before its
  `HeadObject`), while `UploadFile::started` documents `false` for
  skipped files. The documentation was right: such a file was never
  uploaded, like a file skipped at selection, and the page does not read
  `started` for skipped files. `upload_one` now resets it on a
  `destination-exists` skip (also one found by `S3Store::upload` after
  hashing); it stays true while the destination is checked. No event is
  sent at that moment (a file's first event is its first `hashing`
  progress or its final state), so the panel's "Starting…" text (pending
  and started) is never shown for a real batch; the row says "Waiting"
  until hashing begins (found 2026-11-13 in the readiness audit; not
  changed). The doc comments in `upload.rs` and `cloud.ts` now say so.
  New assertions in
  `an_existing_object_is_skipped_before_the_file_is_read`,
  `a_conflict_found_by_the_upload_itself_skips_the_file` and the
  integration scenario; without the fix all three fail.
- **Not run** at the time (the Backblaze run followed the same day,
  [above](#uploads-at-backblaze-2026-11-13)): anything at Backblaze; native Windows 11 (the test is a
  plain `cargo test` and should run there as the round trip did, but it
  has not); CI (not prepared, on purpose); frontend tests (nothing in the
  frontend changed).

### rfd patch guard (2026-11-13)

Uncommitted tree on `207363d`: new `scripts/check-rfd-patch.mjs`
(`npm run check:rfd-patch`, now the first step of `npm run test:headless`,
a step after `cargo fetch --locked` in `ci.yml` and before the release
build in both manual desktop workflows) and its tests
`scripts/check-rfd-patch.test.mjs` (`npm run test:rfd-patch-guard`, also in
`ci.yml`). No production code, dependency or patch changed. How it works:
[patches/README.md](../../src-tauri/patches/README.md), ADR-023.

- **WSL2** (Ubuntu 24.04, Node 24.22.0, cargo 1.99.1): guard on the real
  tree passed (exit 0, 0.24 s). `test:rfd-patch-guard` **10/10 passed**:
  real tree from another directory and an unchanged copy pass; temporary
  copies fail (exit 1) when the patch is unused because its version is
  0.18.0 (Cargo warned "was not used" and resolved rfd 0.17.2 from
  crates.io; the guard named the registry source, the registry path, the
  `[[patch.unused]]` entry and the warning), when the `[patch.crates-io]`
  entry is removed, when the patch resolves as 0.17.3, when `Cargo.lock` is
  stale (`--locked` refused), when `patches/rfd` or `com.rs` is missing and
  when the conversion is reverted to `String::from_utf16(slice).unwrap()`;
  wrong usage exits 2. `npm run test:headless`: guard ok, frontend 135,
  Rust 308 passed + 16 ignored; `npm run check` 0/0.
- **Native Windows 11** (build 26200, Node 24.14.1, cargo 1.99.1; a plain
  copy of the tree, no `node_modules`): `cargo fetch --locked`, then the
  guard from `%TEMP%` by absolute path and through `npm run
  check:rfd-patch` both passed (exit 0, `C:\…\patches\rfd`);
  `test:rfd-patch-guard` **10/10 passed** (the same cases). `test:headless`
  was not run there (no `node_modules` in the copy; nothing it tests
  changed).
- **GitHub Actions** (2026-11-13, committed as `3c48288`, run at
  `f19dd8b` on `main`; every job checked out exactly that commit; details
  and durations [below](#runs-at-f19dd8b-2026-11-13)): `ci.yml`
  ([run 43360177079](https://github.com/casualsoty/sotyware/actions/runs/43360177079))
  passed on `ubuntu-latest` and `windows-latest`: `cargo fetch --locked`,
  then the guard ok (before the first compile), `test:rfd-patch-guard`
  **10/10 on each OS** (the negative cases re-resolved rfd from crates.io on
  the runners), fmt, clippy, `test:headless` (guard ok again, Vitest 135,
  Rust 308 + 16 ignored on Linux, 298 + 16 ignored on Windows).
  `desktop-virtual.yml` and `desktop-windows-isolated.yml`: guard ok before
  the release build. Every build compiled `rfd v0.17.2` from
  `src-tauri/patches/rfd`; no "was not used" warning in any log.
- **Not run:** Windows 10.

### Windows dialog paths with unpaired surrogates (2026-11-13)

Uncommitted tree on `bd704af`: rfd 0.17.2 patched locally
([ADR-023](../architecture/decisions.md#adr-023-rfd-is-patched-locally-so-windows-dialog-paths-are-read-as-osstring),
[`src-tauri/patches/`](../../src-tauri/patches/README.md)); production
change only in that copy (`com.rs`: `OsString::from_wide` instead of
`String::from_utf16(..).unwrap()`) and the `[patch.crates-io]` entry; tests
added in the copy, `names_that_are_not_utf8_are_skipped` (both platform
variants) now also checks that the path is kept unchanged and no key is
made, and `windows-cloud-ui.ps1 -Mode download` gained one step (save
under a name with an unpaired surrogate). No real provider was contacted.

- **Root cause:** every Windows dialog answer goes through
  `IShellItem::get_path` → `read_to_string`, which `unwrap`ped the UTF-16
  decoding: the multi-file upload dialog (`get_results`) and the save
  dialog (`get_result`) were both affected. Nothing after it was lossy:
  `tauri-plugin-dialog` passes the `PathBuf` on as is (`FilePath::Path`,
  `into_path`), `check_file` skips a non-Unicode name (`to_str`, never a
  lossy key), downloads keep the destination `PathBuf` (only the hidden
  temporary file's name and the name shown are made with
  `to_string_lossy`).

**Native Windows 11** (Home 25H2, build 26200.9711; Rust 1.99.1, Node
24.14.1, npm 11.12.1), fresh copy `C:\dev\sotyware-rfdfix`, real unlocked
desktop (the user agreed to keep hands off):

- Release binary `npm run tauri build -- --no-bundle`: 9,733,120 bytes,
  SHA-256 `24f4db80e05a561628dfa97e7a595223477b2a3632fe19caae5f85715f7d1c66`;
  its dependency file lists the sources under `patches\rfd`.
- `cargo test --manifest-path patches\rfd\Cargo.toml --lib --locked`:
  **6 passed** (5 new: ASCII, Unicode with a surrogate pair, empty,
  four unpaired-surrogate patterns kept code unit for code unit and not
  valid UTF-8, and real files, two of them with unpaired surrogates,
  through `SHCreateItemFromParsingName` → `get_path`: the same code units
  and the same file come back; 1 upstream). `cargo fmt --check`, `cargo
  clippy --all-targets -- -D warnings` and `npm run check` (0/0) clean;
  `npm run test:headless`: frontend **135**, Rust **298 passed, 0
  failed**, 16 ignored.
- `windows-cloud-ui.ps1 -Mode upload` (native multi-file dialog, local
  s3s-fs server): **74 passed, 0 failed**, one run. Batch 11
  (`lone-<U+D800>-surrogate.txt` with `z-valid.txt`): the dialog closed,
  the process kept running, the row said "Skipped: name cannot be an object
  key", `z-valid.txt` was uploaded and verified, the summary said "2 files:
  1 uploaded, 1 skipped"; the server held no key the run did not know
  (nothing under a replaced name) and was empty after cleanup. Unicode and
  special-character names (`Grüße Ünïcödé 日本.txt`, `report 2024 (final) +
  notes & more #1.txt`) were stored under exactly those keys with matching
  SHA-256. Tray Quit (real mouse) during a multipart upload: exit 0 after
  1.3 s, parts aborted.
- `-Mode download` (native save dialog): **35/35** before the new step,
  then **36/36** with it (one run each): `readme.txt` saved as
  `lone-<U+D800>-copy.txt` through the save dialog, sotyware kept running,
  "Downloaded … matched", the file exists under exactly that name (read
  through .NET) with the object's SHA-256, no temporary file left.
- **Negative control** (same copy, only the `[patch]` line removed, release
  binary SHA-256 `688a1bf0133c1abd44fa3abee03562c7f711fd85a4f55b789174376f8f671191`,
  rfd from crates.io): `-Mode download` **21 passed, 15 failed**: the new
  step ended sotyware with **`0xC0000409`**, every later check failed
  because the process was gone. So the save dialog was affected too, and
  the new step catches it. The aborted run left its test account's
  Credential Manager entry (local-server test key), removed by exact target
  with `cmdkey /delete`; the patched binary was put back (same SHA-256).
- `%LOCALAPPDATA%\com.sotyware.desktop` (file hashes), Credential Manager
  (0 sotyware entries) and `%TEMP%` were the same before and after.
- **Not run:** a folder (rather than a file) with an unpaired surrogate in
  the save dialog's path (same `get_path` call, covered by the unit test
  only); the folder picker (not used by sotyware); Windows 10; any real
  provider.

**WSL2** (Ubuntu 24.04, Rust 1.99.1), not a Windows result:

- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
  clean (rfd prints its 9 upstream dead-code warnings, ADR-023); `cargo
  test` **308 passed, 16 ignored**; the patch's crate alone (`cargo test
  --manifest-path patches/rfd/Cargo.toml --lib --locked`): 1 passed, 6
  ignored (upstream tests; the new ones are Windows-only and not compiled).
  Frontend tests not re-run (no frontend change).
- Release binary (SHA-256
  `3bb702234465b580c5dde5d9f2e5665789831b55735402b7115158ba09cd7e1c`) on the
  virtual display with the GTK dialog: `cloud-ui.sh upload` **51/51**,
  `download` **22/22**, one run each (unchanged counts; the non-UTF-8 file
  still drops out of the GTK selection, as before).

### Cloud Storage: uploads on the native Windows 11 desktop (2026-11-12)

Uncommitted tree on `695e089` (test code and docs only: new `-Mode upload`
in `tests/desktop/windows-cloud-ui.ps1`; `ui_upload_state` in
`src-tauri/src/cloud/test_server.rs` now builds object keys with `/` also
on Windows, where it had joined them with `\`; no production file
changed). **Native Windows 11 Home 25H2, build 26200.9711**, Rust 1.99.1,
Node 24.14.1, a fresh copy in `C:\dev\sotyware-upwin`. **Real, unlocked
desktop** (the user agreed to keep hands off). **Release binary**
`src-tauri\target\release\sotyware.exe` built there by
`npm run tauri build -- --no-bundle` (06:31, 9,734,144 bytes, SHA-256
`a760184320d7cf9527b0f05fbea9b644dd9baba62add2a36ccb816558204f07c`, the
same file in every run below); nothing listened on the dev-server port
1420 and the window driven belonged to the started process. Accounts in the
**real Windows Credential Manager** (entry checked with `CredRead` and
`cmdkey`). **Local s3s-fs server** in the test process
(`serve_uploads_for_ui`, a new temporary bucket per run, made-up key)
behind a link taking **2 MiB/s**; no provider contacted, no real
credential used. **The native Windows multi-file dialog was used**
("Choose files to upload", common item dialog of sotyware's process): the
folder opened through its File name box and Open, the files selected in
its file list through UI Automation (batch 1) or typed as quoted names
into File name (the others), then Open; Cancel once. Local paths never
appeared on the page. The tray Quit used the real mouse.
([testing.md](../development/testing.md#cloud-storage-ui-end-to-end-test))

- `windows-cloud-ui.ps1 -Mode upload`: **74 passed, 1 failed, 3
  consecutive runs** (08:20, 08:26, 08:31; about 5 minutes each) with the
  final script. The one failure is the defect below; it is the same in each
  run. Every batch's objects were read on the server:
  - the dialog cancelled: no panel, Upload files… enabled again, no upload
    request.
  - into `inbox/`, chosen in the dialog's list: `Grüße Ünïcödé 日本.txt`,
    `empty.txt` (0 bytes, `e3b0…`), `report 2024 (final) + notes & more
    #1.txt`, a 12 MiB file → uploaded and verified under their exact keys
    (content, size, `HeadObject` size and SHA-256 metadata);
    `existing.txt` → skipped, the object unchanged; **`locked.txt`, held
    open by another process without sharing** (Windows only) → "Failed",
    "The file could not be read on this computer", nothing stored, the
    batch went on; summary "6 files: 4 uploaded, 1 skipped, 1 failed.";
    once released, Try 1 file again uploaded it. The browser was moved to
    the bucket root while the dialog was open; the panel kept `inbox/`.
    While the 12 MiB file showed "Sending · n%" the server had no object
    for it.
  - cancel during a 40 MiB PUT → 1 uploaded, 1 cancelled (nothing
    stored), 1 not started; Try 2 files again uploaded exactly those.
  - cancel during a 72 MiB multipart upload → aborted, no parts left.
  - abort refused (injected) → "parts may remain", parts on the server,
    no retry for it; Try 1 file again removed them (cleanup shown) and
    uploaded the other file.
  - verification refused (injected) → stopped, needs inspection, the
    object really stored and not deleted, no retry offered for it.
  - a file appended to while sent (sotyware's read handle lets other
    programs write) → failed, nothing stored; the retry stored the new
    content.
  - a 24 MiB download through the native save dialog completed (SHA-256
    matched) while an upload in `downloads/` was sending; the upload
    finished too.
  - ← Back to accounts during a batch → neither file stored, no parts,
    Upload files… enabled afterwards.
  - close (`SC_CLOSE`, as the title bar's X) during an upload → hidden,
    process running; a second launch exited 0 and showed the same batch,
    which finished.
  - **tray Quit (real mouse) during a 72 MiB multipart upload → exit 0
    after 1.1 s**, no object, parts aborted. After a relaunch the account
    and its key still worked.
  - per run the app sent 17 PUT, 3 create, 6 parts, 4 aborts, 56 HEAD, 13
    lists, 1 GET, 1 `HeadBucket` (no other operation). Keys created and
    then deleted by exact key: `downloads/a-alongside.bin`, `inbox/` +
    `a-changing.bin`, `a-first.txt`, `a-hidden.bin`, `a-inspect.txt`,
    `b-after.txt`, `b-big.bin`, `b-next.txt`, `c-third.txt`, `empty.txt`,
    `Grüße Ünïcödé 日本.txt`, `locked.txt`, `medium.bin`, `report 2024
    (final) + notes & more #1.txt`, plus the 3 seeded objects. Three
    multipart uploads per run (`a-multi.bin`, `a-parts.bin`,
    `a-quit.bin`), all aborted. Each run ended with 0 objects and no
    multipart files; `%LOCALAPPDATA%\com.sotyware.desktop` (hashes),
    Credential Manager (0 sotyware entries) and `%TEMP%` were as before.
- **Defect found (fixed 2026-11-13 by a patched rfd, [details](#windows-dialog-paths-with-unpaired-surrogates-2026-11-13)):** a file whose name holds an unpaired
  surrogate (NTFS allows it), chosen in the dialog with a valid file,
  **ends sotyware with `0xC0000409`** at once: seen in each of the five
  attempts whose output was kept (2, 3 and the final three; attempt 4's log
  was overwritten, and in attempt 1 the step reached the wrong process). Cause: rfd 0.17.2 decodes each path with
  `String::from_utf16(..).unwrap()`, and the release profile has
  `panic = "abort"`. Nothing was stored, and the relaunched app worked.
  Proposed fix, for a decision: patch rfd to decode lossless
  (`OsString::from_wide`, upstream or as a `[patch]`); sotyware's
  `invalid-name` skip then applies. Switching to `panic = "unwind"` alone
  would not help: rfd's dialog thread would still die and the dialog
  would never answer
  ([limitations](limitations.md#cloud-storage)).
- Regression on the same binary, native Windows 11: `windows-cloud-ui.ps1
  -Mode download` **35/35, 3 runs**, `-Mode local` 19/19 and `-Mode
  browse` 29/29 (once each; accounts, Credential Manager, browsing);
  `npm run test:headless`: frontend 135 passed; Rust **297 passed, 1
  failed**, 16 ignored. The failure is a **Windows-incompatible unit test**,
  present since `e7ae326` and not touched here:
  `cloud::upload::tests::a_selection_is_checked_file_by_file_without_failing_as_a_whole`
  writes a file named `größe test+&?#%.bin`, and Windows refuses `?` in a
  file name, so the test helper's `fs::write` panics
  (`test_server.rs:854`). The upload tests had not run on native Windows
  before. Not changed then; **fixed 2026-11-13**: on Windows the test uses
  `größe test+&#%.bin` (without `?`), elsewhere still the name with `?`;
  afterwards native Windows `npm run test:headless` 135 + Rust **298
  passed, 0 failed**, 16 ignored, and WSL2 `cargo test` 308 passed, 16
  ignored, fmt and clippy clean. WSL2: `cargo test` 308 passed, 16 ignored; `cargo
  fmt --check` and `cargo clippy --all-targets -- -D warnings` pass (the
  Linux `cloud-ui.sh upload` phase was not rerun after the helper change,
  which only affects `\` separators). `windows.ps1` (real desktop, tray
  lifecycle) was not run; the tray menu and Quit were exercised by the
  upload run only.
- Driver, not app: the first four attempts failed in the driver (keys
  joined with `\` by the helper on Windows; whole-tree UI Automation
  searches too slow for a 40 MiB PUT at 6 MiB/s; a literal `·` in a regex
  that Windows PowerShell 5.1 read as `Â·`, so the Sending percentage was
  never matched; a relaunch that only handed over to the still-running
  app). Their server-side results agreed with the panel each time.

Not run: Windows 10; a person selecting files with the mouse or Ctrl+A in
the dialog; a cancel between two files; names Windows refuses (control
characters, `< > : " | ? *`) or that cannot be selected from one folder
(duplicate keys); key-too-long and too-large skips; an abort that hangs at
quit; any real provider.

### Cloud Storage: uploads through the release binary on a virtual display (2026-11-12)

Uncommitted tree on `f9026f5` (test code and docs only: new
`serve_uploads_for_ui` helper in `src-tauri/src/cloud/test_server.rs`, new
`upload` phase in `tests/desktop/cloud-ui.sh`, `cloud_ui.py` and
`cloud_ui_upload.py`; no production file changed). WSL2 Ubuntu 24.04.1,
Rust 1.99.1, Node 24.22.0. **Virtual display** (Xvfb + openbox, private
D-Bus session, stub tray watcher, real GNOME Keyring with a throwaway
keyring), **release binary** `src-tauri/target/release/sotyware` built by
`npm run tauri build -- --no-bundle` at 05:32 (SHA-256 `8788e2f3…18aa`;
nothing listened on the dev-server port 1420, so the page was the bundled
frontend). **Local s3s-fs server** in the test process behind a throttled
link (6 MiB/s towards the server), made-up key; no provider contacted.
**The native GTK multi-file dialog was used** (location bar, Ctrl+A in its
file list, Open; driven through AT-SPI and XTest).
([testing.md](../development/testing.md#cloud-storage-ui-end-to-end-test))

- `cloud-ui.sh upload`: **51 passed, 0 failed, 3 runs** (after the driver
  was debugged; the earlier attempts failed in the driver, not the app).
  Every batch's objects were read on the server (content, size, `HeadObject`
  size and SHA-256 metadata, open multipart files):
  - into `inbox/`: empty file (0 bytes, `e3b0…`), `Grüße Ünïcödé 日本.txt`,
    `report 2024 (final) + notes & more #1.txt`, a 12 MiB file → uploaded
    and verified, stored under their exact keys; `existing.txt` → skipped,
    the existing object unchanged; a link to `/dev/null` → not a file; a
    broken link → could not be read; a name with U+0001 → control
    characters. Summary "8 files: 4 uploaded, 4 skipped." The destination
    stayed `inbox/` although the browser was moved to the root while the
    dialog was open. While the 12 MiB file showed "Sending · n%" the server
    had no object for it yet.
  - cancel during a 40 MiB PUT: the file before it kept, the file after it
    "Not started", nothing stored for the cancelled one; **Try 2 files
    again** uploaded exactly those two.
  - cancel during a 72 MiB multipart upload: aborted, no parts left.
  - abort refused (injected): "parts may remain", the parts really on the
    server, not retryable; **Try 1 file again** took only the other file and
    first removed the remembered parts (cleanup shown, none left).
  - verification `HeadObject` refused after the PUT (injected): the batch
    stopped, the file "needs inspection" (the object really was stored and
    was not deleted), **no retry offered for it**; Try 1 file again took
    only the not-started file.
  - a file appended to while it was sent: failed, nothing stored, retry
    uploaded the new content.
  - a download through the GTK save dialog completed (SHA-256 matched)
    while an upload in `downloads/` was still sending; the upload finished
    too.
  - ← Back to accounts during a batch: neither file stored 10 s later, no
    parts; Upload files… enabled again.
  - close (✕, tray mode: hide) during an upload: window unmapped, process
    running; a second launch showed it again (exit 0) with the same batch,
    which then finished.
  - tray Quit during a multipart upload: exit 0 after about 3 s, no object,
    parts aborted.
  - the app sent only `HeadObject`, `PutObject`, multipart calls,
    `GetObject` and `ListObjectsV2` (per run: 16 PUT, 3 create, 6 parts, 4
    aborts, 52 HEAD, 1 GET, 10 lists). Cleanup deleted exactly the 13
    uploaded keys and the 3 seeded ones by key; the bucket ended with 0
    objects and no multipart upload.
- Found: on Linux the GTK dialog's answer never contains a file whose name
  is not valid UTF-8: rfd 0.17.2 (`backend/gtk3/file_dialog/dialog_ffi.rs`,
  `get_results`) drops such paths silently, so sotyware's own
  `invalid-name` skip is never reached and the page does not mention the
  file ([limitations](limitations.md#cloud-storage)). Not changed.
- Only "Sending · n%" was seen in the panel; hashing and verifying are too
  short at these sizes to be sampled through AT-SPI (they are covered by
  the Rust and Vitest tests).
- Regression on the same binary: `cloud-ui.sh download` **22/22, 3 runs**;
  `local` 16/16, `hang` 8/8, `browse` 17/17 (once each);
  `npm run test:desktop:virtual` **119/0/0**; `npm run test:headless` 308
  passed, 16 ignored (the new helper is one) + 135 frontend;
  `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.

Not run: native Windows (`windows-cloud-ui.ps1` has no upload mode and was
not run), a real Linux desktop or Wayland, the XDG portal's file dialog,
any real provider, a duplicate-key selection (the GTK dialog picks from one
folder, so two files cannot share a name; covered by Rust tests), a
cancel in the exact gap between two files (too short to hit from the UI;
Rust tests cover it), key-too-long and too-large skips, a hang of the abort
at quit (the 35 s bound).

### Cloud Storage: upload UI (2026-11-12)

Uncommitted tree on `e7ae326`, WSL2 Ubuntu 24.04.1, Node 24.22.0, Rust
1.99.1. Frontend only: `src/CloudUpload.svelte` (new), the Upload files…
action in `src/CloudBrowser.svelte`, typed wrappers and parsers in
`src/cloud.ts`, upload messages in `src/cloudMessages.ts`, `src/format.ts`
(moved out of the browser unchanged). No Rust file changed.
([cloud-storage.md](../architecture/cloud-storage.md#the-upload-panel))

- `npm run check`: 0 errors, 0 warnings; `npm run build` passes.
- Vitest: **135 passed** (before: 93). 42 new in `src/CloudUpload.test.ts`
  ([list](../development/testing.md#frontend-tests)); the existing 93,
  including every download test in `src/CloudBrowser.test.ts`, pass
  unchanged (no test file other than the new one was edited). Three
  mutations of the panel each made tests fail.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`: pass;
  `cargo test`: **308 passed, 15 ignored** (unchanged).

Not run: any desktop or UI test of the release binary (Linux virtual
display, `cloud-ui.sh`, Windows `windows-cloud-ui.ps1`; their existing
browse and download checks press buttons by name and were not rerun), the
native multi-file dialog, native Windows, any real provider. The tests mock
the three commands and feed their channels; they show what the page renders
and asks for, not that the native pieces work.

### Upload batches: backend and commands (2026-11-12)

Uncommitted tree on `241de11`, WSL2 Ubuntu 24.04.1, Rust 1.99.1, Node
24.22.0. `cloud/upload.rs` (`upload_cloud_files`, `cancel_cloud_upload`,
`retry_cloud_upload`, `CloudUploads`;
[ADR-022](../architecture/decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust))
and two small additions to the Phase 1 upload (the open multipart id on
`TransferControl`, `abort_incomplete_upload`). No page calls the commands.

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: **308 passed, 15 ignored**, three runs in a row (before:
  272 + 15). 36 new tests in `cloud/upload/tests.rs` (list:
  [testing.md](../development/testing.md#rust-unit-tests)), all against the
  in-process server or pure; the upload batch tests alone passed three
  more times. The 25 download tests (`cloud::download`) and the 34
  `S3Store::upload` tests pass unchanged; no download file was modified.
- `npm run check`: 0 errors, 0 warnings; Vitest **93 passed** (unchanged;
  only `file-changed` was added to `ERROR_KINDS` and its summary sentence).
- Found while testing: a `PutObject` refused after its whole (small) body
  was handed over is past the commit point, so Phase 1 rightly reports
  `NeedsInspection`, not "nothing stored"; the stop-the-batch test uses a
  refused `HeadObject` instead.

Not run: any real provider (Backblaze, Hetzner), native Windows, any
desktop or UI test (there is no upload UI; the native multi-file dialog was
never opened: tests pass the dialog's answer directly), a crash during a
multipart upload.

### Backend upload lifecycle (2026-11-12)

Uncommitted tree on `7b11ecf`, WSL2 Ubuntu 24.04.1, Rust 1.99.1.
`S3Store::upload` and its tests
([ADR-021](../architecture/decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration));
backend only, no command, dialog or page.

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: **272 passed, 15 ignored**, three runs in a row (before:
  238 + 14). 34 new upload tests in `cloud/s3/upload/tests.rs` against the
  in-process server (list: [testing.md](../development/testing.md#rust-unit-tests)),
  one more ignored (the minute-long test below). The existing upload,
  download and timeout tests pass unchanged; `cloud/tests.rs` only got the
  two new `ClientOptions` fields in its option literal.
- Timeout evidence through a throttled TCP relay on loopback: a plain
  `PutObject` with a 1 s read timeout fails with a timeout (the defect,
  reproduced); with the upload override a 12 MiB PUT at 2 MiB/s took about
  7.5 s under the same 1 s read timeout and passed; a relay stalled after
  256 KiB timed out after about 2.7 s with a 1.5 s stall timeout, nothing
  stored. The opt-in `a_slow_upload_longer_than_a_minute_succeeds_with_the_default_timeouts`
  (7 MiB in one PUT at 100 KiB/s, **default 60 s** read and stall timeouts)
  passed twice: 72.4 s each; in the second run, after the progress fix,
  the body was handed over in bursts (about 0.9 MiB every 9 s) until 46.3 s
  and the response came at 72.4 s.
- Found and fixed while testing: progress updates held back by the 250 ms
  throttle were only shown with the next burst; they are now sent once due.

Not run: any real provider (Backblaze, Hetzner), native Windows, the opt-in
provider tests with the two added `HeadObject` per `upload_file`, the
frontend and desktop suites (no frontend or runtime change).

### Hardened Backblaze download test on the Windows dashboard (2026-11-12)

Tree: `bb8c178`, committed and clean (no uncommitted change); it adds the
hardening from the [review below](#review-of-the-download-tests-2026-11-12)
to the flow that [passed before](#real-backblaze-download-through-the-windows-dashboard-2026-11-12).
Environment: as in that run: **native Windows 11 Home 25H2, build
26200.9711, the real, unlocked desktop**, release binary built in a fresh
copy `C:\dev\sotyware-dlreval` (Rust 1.99.1, Node 24.14.1), driven from WSL2
through `powershell.exe`; UI Automation and the dialog's control ids, no
mouse, no keystrokes. Provider **Backblaze B2**,
`https://s3.eu-central-003.backblazeb2.com`, the dedicated test bucket.

Offline checks first, with no `SOTYWARE_*` variable set: WSL2
`check-ps-variables.ps1` PASS on the three Windows scripts, `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` **238
passed, 14 ignored** (all three download opt-in tests among the ignored);
Windows `npm ci`, `npm run check` 0/0, fmt, clippy, `test:headless` **228 +
14 ignored, 93 frontend**, release build, `check-ps-variables.ps1` PASS.

**Result: `windows-cloud-ui.ps1 -Mode b2-download` 35/35, one run, first
attempt** (run id `1794451082434055600-36332`):

- Prepare: endpoint checked; `ListObjectVersions` of
  `sotyware-it/download-ui-validation/1794451082434055600-36332/` → none;
  generated 16,777,219 bytes, SHA-256
  `ff94cdeeab8f48567823bfeff8f15cf487ceb2fe342fb4272d883fe70e9c5009`
  computed before the upload; one `PutObject` of `größe test.bin` with the
  `sotyware-sha256` metadata → version
  `4_zdd327be63254c7b0a71e0912_f10683753345deaa7_d20261112_m023803_c003_v0312048_t0055_u01794451083415`.
- **New manifest check passed** before the app started: the manifest's run,
  version id, file name, size, SHA-256 and key equal what the upload line
  printed, and the generated file has that SHA-256.
- Dashboard (read-only key, real Credential Manager): Connected; the run
  folder with exactly one row `größe test.bin | 16 MiB`; the native save
  dialog suggested the name unchanged; saved into a new folder under
  `%TEMP%`. Two progress lines this time ("0% · 0 B of 16 MiB", "63% · 10 MiB
  of 16 MiB"), never going down; a `.sotyware-partial` file seen meanwhile;
  final message after 0.6 s: "Downloaded 16 MiB. The SHA-256 checksum stored
  with the file matched.", with the file already in place at full size and
  no temporary file left. Saved file: 16,777,219 bytes, same SHA-256,
  byte-for-byte equal to the original.
- Cleanup: `DeleteObject` with exactly that version id → 204 (no delete
  without a version id); `ListObjectVersions` of the run prefix → **0
  versions or delete markers left**. Both local folders removed, no
  `.sotyware-partial` under `%TEMP%`.
- Checked from PowerShell before and after: `cloud-accounts.sqlite3` and
  `preferences.sqlite3` with the same SHA-256, 0 sotyware entries in
  Credential Manager, no sotyware process left.

Remote requests: write key 4 (2 listings, 1 PUT, 1 versioned DELETE); read
key about 6 through the app (not recorded).

**Not verified:** the hardened `backblaze_b2_download_file` (Rust only, WSL2)
was not run against Backblaze; a failing prepare, manifest mismatch or
failing cleanup at the real provider (those paths are only read, not
exercised); everything listed as not verified in the earlier run.

### Review of the download tests (2026-11-12)

Tree: uncommitted, on `5a7453a`; test code and docs only, **no application
change**. WSL2 (Ubuntu 24.04.1, Rust 1.99.1); Windows PowerShell 5.1 of the
Windows 11 host, called from WSL2, only to **parse** the Windows scripts.
No remote request, no desktop interaction, no dialog opened.

The production download path (`cloud/download.rs`, `S3Store::download`)
was reviewed without finding a defect: the body is streamed chunk by chunk,
synced, size- and SHA-256-checked before the commit point; cancel and the
final move are decided once in `TransferControl`; the temporary file is
removed on every error and on cancel; "completed" is sent after the file is
in place; `sha256-verified` only for a stored, matching SHA-256; the page
gets only the destination's file name (local write errors name the path
the user chose).

Changed in the tests:

- `windows-cloud-ui.ps1`: the `b2-browse` block's `$out` renamed (it is the
  same variable as the download folder `$Out`; harmless today only because
  the modes exclude each other); `b2-download` stops before starting the
  app unless the manifest names the run and version the upload printed.
- New `tests/desktop/check-ps-variables.ps1` (static, parse only): fails
  when one scope spells a variable two ways. It found the `$out`/`$Out`
  clash above, flags mutants that reintroduce the two clashes seen in the
  `b2-download` attempts (`$out`/`$Out`, `$a`/`$A`), and passes
  `windows.ps1`, `windows-isolated.ps1` and the fixed `windows-cloud-ui.ps1`.
- `cloud/integration/download.rs`: `backblaze_b2_download_file` now checks
  the endpoint and that its run prefix is empty before uploading (one more
  listing; **not yet run against Backblaze**); the cleanup test checks the
  endpoint too; prepare and cleanup print a failure's reason on stdout,
  which the Windows driver keeps (it drops stderr, where a panic goes).

Checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`;
`cargo test` **238 passed, 14 ignored** (no `SOTYWARE_*` variable set);
`check-ps-variables.ps1` PASS on the three Windows scripts. Frontend not
rerun (unchanged). The changed `b2-download` path and the changed opt-in
tests have **not run** against Backblaze or on the Windows desktop.

### Real Backblaze download through the Windows dashboard (2026-11-12)

Tree: uncommitted, on `fc76109` (test code only: `windows-cloud-ui.ps1
-Mode b2-download` and the opt-in tests `backblaze_b2_download_ui_prepare` /
`_cleanup` in `cloud/integration/download.rs`; no application change).
Environment: **native Windows 11 Home 25H2, build 26200.9711, the real,
unlocked desktop** (not a private desktop, not WSL2 or a virtual display),
release binary built from that tree in a fresh copy `C:\dev\sotyware-dlui`
(Rust 1.99.1, Node 24.14.1), driven from WSL2 through `powershell.exe`. The
dashboard was driven with UI Automation, the common item dialog by control id
and window messages (no mouse, no keystrokes). Provider **Backblaze B2**,
`https://s3.eu-central-003.backblazeb2.com`, the dedicated test bucket.
[testing.md](../development/testing.md#downloading-through-the-windows-dashboard-at-backblaze).

**Result: 35/35, one run** (run id `1794449850614005000-36480`):

- Prepare (write key): endpoint and bucket as configured, read-only key
  different from the write key; `ListObjectVersions` of
  `sotyware-it/download-ui-validation/1794449850614005000-36480/` → none;
  generated 16,777,219 bytes (16 MiB + 3), SHA-256
  `ff94cdeeab8f48567823bfeff8f15cf487ceb2fe342fb4272d883fe70e9c5009`
  computed locally before the upload; one `PutObject` of `größe test.bin`
  (single part) → version
  `4_zdd327be63254c7b0a71e0912_f1015a9a0a26512a9_d20261112_m021731_c003_v0312048_t0019_u01794449851601`.
- Dashboard (read-only key, added through the form; real Credential
  Manager): Check connection → Connected; Open bucket → root with
  `sotyware-it`; `sotyware-it` → `download-ui-validation` → the run folder,
  exactly one row `größe test.bin | 16 MiB`; Download → the **native Windows
  save dialog** with `größe test.bin` suggested unchanged; saved into a new
  folder under `%TEMP%` (outside the repository).
- Progress: the page showed "0% · 0 B of 16 MiB" (total known, so
  `GetObject` carried `Content-Length`) and a hidden `.sotyware-partial`
  file existed; the final message came after 0.6 s, so only **one** progress
  line was sampled: "never goes down" was trivially true here. Byte-level
  monotonicity at Backblaze is shown only by the WSL2 Rust run below.
- Final message: "Downloaded 16 MiB. The SHA-256 checksum stored with the
  file matched." (`sha256-verified`: the `GetObject` response carried the
  `sotyware-sha256` metadata). When it appeared, the file was already in
  place at 16,777,219 bytes and no temporary file existed.
- Saved file, checked outside the app: 16,777,219 bytes; SHA-256 (`Get-FileHash`)
  equal to the value above; byte-for-byte equal to the generated original;
  the folder held only that file. The ETag was not used.
- Afterwards rename, key replacement and removal through the UI as in the
  other modes; Credential Manager: 0 `com.sotyware.desktop` entries before
  and after (checked again with `cmdkey` afterwards).
- Cleanup (write key): `DeleteObject` with exactly that version id → 204;
  `ListObjectVersions` of the run prefix → **0 versions or delete markers
  left**. Local: downloaded file, generated original and both folders
  removed; no `.sotyware-partial` anywhere under `%TEMP%` (checked again
  from PowerShell); the user's `cloud-accounts.sqlite3` was moved aside and
  put back unchanged (same size and time stamp).

Remote requests of this run: write key 4 (2 listings of the run prefix, 1
PUT, 1 versioned DELETE); read key through the app about 6 (`HeadBucket`, 4
`ListObjectsV2` folder levels, 1 `GetObject`; the app's requests are not
recorded). Nothing else in the bucket was listed or changed.

**An earlier attempt failed in the test driver, not in the app** (same
day, run id `1794449764897407200-35528`, 25/26): in PowerShell `$out`
(the prepare test's output) and `$Out` (the download folder) are the same
variable, so the driver failed building the destination path right after
the save dialog had opened with the right name; the dialog was cancelled,
**no `GetObject` was sent**. Its version
(`4_zdd327be63254c7b0a71e0912_f1172e1b607c1d84c_d20261112_m021605_c003_v0312049_t0017_u01794449765844`)
was deleted by id and the listing showed 0 left; its two local folders were
removed by hand. The driver was fixed (and a second clash, `$a` with `$A`,
found by reading) before the passing run, which uploaded one new object.

Regression on this tree: WSL2 `cargo test` **238 passed, 14 ignored** (1
new unit test, 2 new opt-in tests), frontend 93, fmt, clippy; native Windows
11 `npm run check`, fmt, clippy, `test:headless` **228 + 93**, release build.
No other desktop suite was rerun (no application change).

**Not verified:** more than one passing run; several progress lines from a
real provider in the UI (the download finished within one sample); a
download through the UI from Backblaze on Linux, on Windows 10 or by a
person; multipart-uploaded or larger files; a missing or wrong checksum at a
real provider; Hetzner.

### One download from Backblaze (2026-11-12)

Tree: uncommitted, on `d95978d` (new test file
`cloud/integration/download.rs`, test code only; no application change).
Environment: WSL2 (Ubuntu 24.04.1, Rust 1.99.1), one run, the user's
agreement for one temporary upload. Provider **Backblaze B2**, endpoint
`https://s3.eu-central-003.backblazeb2.com`, the dedicated test bucket.
Test: `backblaze_b2_download_file`
([testing.md](../development/testing.md#one-download-from-backblaze)).
**Not the app's UI and not native Windows:** the download ran through
`CloudDownloads` (the code behind `download_cloud_file`) with a temporary
path instead of the save dialog and a recorder instead of the page.

No existing object could be used: the bucket holds no permanent test object
(every Backblaze test deletes what it created; `sotyware-it/` had 0 versions
after the last recorded cleanup) and no SHA-256 of a provider object was
recorded anywhere. So the test generated one file and computed its SHA-256
locally before uploading it.

| What | Result |
| --- | --- |
| Object | key `sotyware-it/download-validation/1794448986685823938-585899/größe test.bin`, 16,777,219 bytes (16 MiB + 3), one `PutObject` with the write key (prefix-restricted), version `4_zdd327be63254c7b0a71e0912_f1189661b41b38ff4_d20261112_m020309_c003_v0312001_t0012_u01794448989942` |
| Expected SHA-256 (computed locally from the generated file, before the upload) | `ff94cdeeab8f48567823bfeff8f15cf487ceb2fe342fb4272d883fe70e9c5009` |
| `HeadObject` (read-only key) | 200; size 16,777,219; `sotyware-sha256` metadata **present**, equal to the expected value |
| `GetObject` through `CloudDownloads` (read-only key, account's configured bucket, exact key) | ✅ completed in 0.47 s; the key with the space and `ö`/`ß` reached Backblaze and came back unchanged (URL-encoded in the recorded requests) |
| `Content-Length` on `GetObject` | **present**: every "active" message after the first carried total 16,777,219 |
| `sotyware-sha256` on `GetObject` | **present and matching**: the final message said `sha256-verified`, which the code gives only when the response's metadata equals the hash of the received bytes |
| Progress | 3 "active" messages: (0 bytes, no total) before the request, (0, 16,777,219) at the response, (14,469,279, 16,777,219) during the body; bytes never decreased; then one `completed` with bytes = total = 16,777,219 |
| Completion after the file was in place | ✅ when `completed` was sent the destination had its full size and no `.sotyware-partial` file existed |
| Downloaded file | 16,777,219 bytes; SHA-256 computed locally `ff94cdee…5009` = expected; **byte-for-byte equal** to the generated file |
| Local cleanup | no `.sotyware-partial` left; the destination folder (under the session's temporary folder, outside the repository) removed; the temporary folder empty afterwards |
| Remote cleanup | `DeleteObject` of exactly that version id → 204; `ListObjectVersions` of the run prefix → 200, **0 versions or delete markers left** |
| Credentials | read from the two 0600 env files into the test process only; none of the four values appears in the saved test output (checked with `grep -F`) |

Remote requests, all one attempt: write key `PutObject` → 200,
`DeleteObject` (with `versionId`) → 204, `ListObjectVersions` (run prefix
only) → 200; read-only key `HeadObject` → 200, `GetObject` (through the
app's client, not recorded; the completed, verified download shows it
succeeded) — 5 in total. Whole test 5.9 s.

Regression (WSL2): `cargo fmt --check`, clippy `-D warnings` ✅; `cargo test`
✅ **237 passed, 12 ignored** (1 new: the same download against the
in-process server; 1 new ignored: this test). Frontend and desktop tests not
rerun (no application or frontend change).

**Not verified:** the download through the dashboard's UI and native save
dialog at a real provider (native Windows 11 included: it would control the
desktop), a second run, a multipart-uploaded or larger object, a provider
object without `sotyware-sha256` or with a wrong one (covered only by the
in-process server), Hetzner. ETags were not used or compared.

### Cloud Storage: single-file downloads (2026-11-12)

Tree: uncommitted, on `8e23fd4`. New: `cloud/download.rs` (three commands,
ADR-020), `S3Store::download` with `TransferControl` (progress, cancel, new
temporary-file naming, final move by hard link or rename), the dialog plugin
(`tauri-plugin-dialog` 2.8.4 with `rfd` 0.17.2, plus `tauri-plugin-fs` 2.7.1 as
its dependency; 4 new crates), the Download action and progress panel in
`CloudBrowser.svelte`, an object backend for the in-process test server
(`start_objects`: exact keys, optional SHA-256 metadata, no `Content-Length`,
slow, stalled or cut bodies) and a `download` phase in both Cloud Storage UI
drivers. Nothing was written to or read from a real provider.

| Check | WSL2 (Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0) | Native Windows 11 Home 25H2 (build 26200.9711; fresh copy `C:\dev\sotyware-download`) |
| --- | --- | --- |
| fmt, clippy `-D warnings`, `npm run check` (0/0), `npm run build`, release build | ✅ | ✅ |
| `cargo test` | ✅ **236 passed, 11 ignored**, 3 runs (27 new: 24 in `download/tests.rs`, 3 in `s3.rs`; 1 new ignored UI helper) | ✅ **226 passed, 11 ignored** (the Linux-only tray tests and the Unix permission test do not build there) |
| Frontend (Vitest) | ✅ **93** (12 new), 3 runs | ✅ 93 |
| `npm run test:desktop:virtual` (virtual display) | ✅ 119/0/0, 3 runs | — |
| `windows-isolated.ps1` (private desktop) | — | ✅ 56/0, 3 runs |
| Cloud UI `local` | ✅ 16/16, 3 runs (`cloud-ui.sh`, Xvfb, real GNOME Keyring) | ✅ 19/19, 4 runs (real Credential Manager) |
| Cloud UI `hang` | ✅ 8/8, 3 runs | — |
| Cloud UI `browse` (local listing server) | ✅ 17/17 in 5 of 6 runs; run 2 of the first 3 stopped with an AT-SPI reply timeout (below) | ✅ 29/29, 4 runs |
| **Cloud UI `download`** (local object server, **native save dialog**) | ✅ **22/22, 6 runs** (GTK 3 file chooser under Xvfb + openbox: a virtual display, not a real desktop) | ✅ **35/35, 3 runs** on the **real, interactive Windows 11 desktop** (common item dialog and "Confirm Save As"; driven through UI Automation and window messages, no mouse or keystrokes, nobody watching) |
| `windows.ps1` (real desktop, moves the mouse) | — | ✅ **103/0, 3 runs** (run with the user's agreement after the other checks; both databases moved aside and put back) |

What the `download` phase showed through the release binary: a Download
action per file; the native dialog opening with the key's last segment
suggested (also `Bericht 2024 ü & co.pdf` unchanged); cancelling the dialog
starts nothing; Save → the file's SHA-256 equals the server's and the page
says the stored checksum matched; an existing file → the dialog's own
question (GTK "already exists" + Replace, Windows "Confirm Save As" + Yes),
the old file unchanged until answered, replaced afterwards, no temporary
file left; a file without stored SHA-256 → completed with "could not be
verified"; a slow 16 MiB file → percent and "of 16 MiB", one hidden
`.large.bin.<pid>-<n>.sotyware-partial`, Cancel → "Download cancelled.
Nothing was saved." and no file; a connection cut after 256 KiB → failed,
Try download again without a dialog fails the same way, nothing saved; a
SHA-256 mismatch → integrity failure, nothing saved; at the end exactly the
completed files in the folder. Linux only: closing the window (dashboard-only
mode quits) during a download → exit 0 and the temporary file removed. The
test server saw only `GetObject`, `ListObjectsV2` and `HeadBucket` (13
`GetObject` per Linux run, 12 per Windows run).

Problems met and fixed during the runs (test code only):

- Windows: the large-file test read the temporary file's size through
  `DirEntry::metadata`, which on Windows reports a still-open file's size as
  of its last close; it now asks the file (`fs::metadata`). Failed once
  before the fix, then passed.
- Windows: the common item dialog's File name box, Save, Cancel and the Yes
  of "Confirm Save As" are not in its UI Automation tree (Windows 11 25H2);
  the first three `-Mode download` attempts failed there (and, by a script
  exception, left their made-up test entries in Credential Manager, which
  were removed by hand; the mode now turns any exception into a failed check
  and still removes the account). The driver finds them by control id and
  uses `WM_SETTEXT`/`BM_CLICK`/`TDM_CLICK_BUTTON`.
- WSL2: in one of the first three `cloud-ui.sh` runs, pressing "← Back to
  accounts" after the 1,000-row page got no AT-SPI reply in time (the
  browser now has one Download button per row, and a Windows build ran on
  the same machine meanwhile) and the driver stopped. `press` now reports a
  missing reply and lets the following check decide; the 3 `browse` reruns
  passed without hitting it.
- The Windows C: drive was full (0 bytes; `os error 112` in the first
  Windows build); with the user's agreement the `src-tauri\target` folders of
  seven earlier test copies under `C:\dev` were deleted (about 57 GB).

Not verified: any download from Backblaze or Hetzner (the opt-in tests were
not extended; see [roadmap.md](roadmap.md#cloud-storage-after-the-proof-of-concept);
one Backblaze download through the Rust code followed the same day:
[details](#one-download-from-backblaze-2026-11-12));
quitting during a download on Windows through the tray (only the Rust test
and the Linux window-close run); the XDG portal dialog; a person using the
dialogs; Windows 10.

### Real Windows desktop and fewer Backblaze listings (2026-11-12)

Tree: `f304019` plus the uncommitted change to
`src-tauri/src/cloud/integration/browse.rs` below (test code only; the
application is that of `f304019`).

**Real-desktop Windows test.** `windows.ps1` had not been run since bucket
browsing was added. Run from WSL2 on the unlocked, interactive desktop of the
native Windows 11 machine (Home 25H2, build 26200.9711), with the user's
agreement and hands off, against the release binary built in a fresh copy
`C:\dev\sotyware-winval` (`npm run tauri build -- --no-bundle`):
`powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1`
→ ✅ **103 passed, 0 failed, 3 runs** (the script has no skips). It moved the
existing `preferences.sqlite3` and `cloud-accounts.sqlite3` aside and put
them back; afterwards both had the size and modification time from before
and no backup file was left. This is a real desktop, unlike the private
desktop and virtual display rows below.

**Fewer listings in the Backblaze dataset test.** The page size of 7 was not
in the UI: the UI tests page with the browser's own 1,000 (`LIST_PAGE_SIZE`,
unchanged). It was in `validate()` of `backblaze_b2_browse_dataset`, which
every Backblaze UI run also calls to create its dataset: besides the
dashboard's listings it walked the provider's own `ListObjectsV2` pages of
the run root in pages of 4, `many/` (1,010 entries) in pages of 7 and
`many/` in pages of 1,000: 4 + 145 + 2 = 151 billed listings, of about 185
in the whole check (the other 34: bucket root, `browse-validation/`, the 26
folders with two pages for `many/`, five cursor and Refresh listings;
refused cursors never reach the provider). Now `many/` is walked in pages of
**336** (`MANY_PROVIDER_PAGE`): 4 + 4 + 2 = **10** listings, so about **44
instead of about 185** per dataset check (−76 %) and, with the UI's own
listings (about 50, unchanged), about **95 instead of about 250 per UI run**
(estimates from the code; no run at Backblaze was made for this change).
The check is also stricter: each provider page must be **exactly the
expected slice of the folder in key order** (before: only the total count
and no duplicates), there must be at least 2 pages, and the number of
listings sent must equal the number of pages. Pages of 336 split `many/` as
336 files, 336 files, 333 files + 3 subfolders, 2 subfolders: boundaries
inside the files, inside the subfolders, and one page holding both.

Against the in-process listing server (`browse_checks_pass_against_the_test_server`,
part of every `cargo test`; contacts no provider):

| Check | WSL2 | Native Windows 11 |
| --- | --- | --- |
| provider pages of 4 at the run root: 13 entries, 4 pages, each the expected slice | ✅ | ✅ |
| provider pages of 336 in `many/`: 1,010 entries, 4 pages, each the expected slice | ✅ | ✅ |
| provider pages of 1,000 in `many/`: 1,010 entries, 2 pages, each the expected slice | ✅ | ✅ |
| 10 provider listings for 10 pages, all delimited `ListObjectsV2` | ✅ | ✅ |
| dashboard pages of `many/` (1,000 + 10), cursor used twice and across folders refused, Refresh resets, every folder and special name | ✅ (unchanged checks) | ✅ |
| mutant: expected pages cut one entry larger | ❌ all three page checks fail, as they must (WSL2 only, reverted) | — |

**Regression** (WSL2: Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0; Windows:
the copy above, Rust 1.99.1, Node 24.14.1):

| Check | WSL2 | Native Windows 11 |
| --- | --- | --- |
| fmt, clippy, `npm run check`, `npm run build`, release build | ✅ | ✅ |
| `cargo test` | ✅ 209 passed, 10 ignored | ✅ 200 passed, 10 ignored |
| Frontend (Vitest) | ✅ 81 | ✅ 81 |
| `npm run test:desktop:virtual` (virtual display, not a real desktop) | ✅ 119/0/0, 3 runs | — |
| `cloud-ui.sh local hang browse` (Xvfb, not a real desktop) | ✅ 16/16, 8/8, 17/17, 3 runs | — |
| `windows-isolated.ps1` (private desktop, not the real one) | — | ✅ 56/0, 3 runs |
| `windows-cloud-ui.ps1` local / `-Mode browse` | — | ✅ 19/19 / 29/29, 3 runs each |
| `windows.ps1` (**real desktop**) | — | ✅ 103/0, 3 runs |

Not run: the Backblaze tests (`backblaze_b2_browse_dataset`, `cloud-ui.sh
b2-browse`, `-Mode b2-browse`): nothing was written to, read from or deleted
at Backblaze for this change, so the new page size has been seen only
against the local listing server.

### Cloud Storage: browsing a temporary dataset at Backblaze (2026-11-11)

Tested as an uncommitted working tree on top of `8641c1a` (the tree was clean
before; nothing committed). Test code only, no app code changed: the opt-in
`backblaze_b2_browse_dataset` and `backblaze_b2_browse_cleanup`
(`src-tauri/src/cloud/integration/browse.rs`), the always-run
`browse_checks_pass_against_the_test_server`, and a `b2-browse` phase /
`-Mode b2-browse` in the Cloud Storage UI tests
([how](../development/testing.md#browsing-a-temporary-dataset-at-backblaze)).
No dependency added.

**Bucket and keys.** The dedicated B2 test bucket in `eu-central-003`
(endpoint `https://s3.eu-central-003.backblazeb2.com`, checked by every run).
Every write used the existing key (that bucket only, file name prefix
`sotyware-it/`). The UI used a second key the user created for this:
Read Only, the same bucket, **no** file name prefix (Backblaze refuses the
root to a prefix-restricted key, and the browser always opens at the root).
Every run showed that this key lists the root and the run but is refused a
`PutObject`. Keys came from the user's files outside the repository
(`~/.config/sotyware-it/b2.env` and `b2-read.env`, mode 600; on Windows the
DPAPI files `b2.xml`, `b2-read.xml`); key ids, secrets and the bucket name
were removed from all output.

**Dataset**, under `sotyware-it/browse-validation/<unix nanos>-<pid>/`, new
for every run: 1,040 objects, 49,515 bytes, contents derived from the key,
SHA-256 recorded locally and as metadata. Categories: files at the run root
(a space, an emoji, `café` in NFC and in NFD); folders 1 to 5 levels deep;
`photos/` with a marker object and content; two marker-only (empty) folders;
Latin-with-diacritics, Cyrillic, Arabic and Japanese folder and file names;
`+ & ? # %` and a literal `%20` in file names and in a folder name;
`double//inner.txt`, `double///triple.txt`; `dots/../escape.txt`, `dots/..`,
`dots/./here.txt`, `dots/..name.txt`; `many/` with 1,005 files and 5
subfolders. **Backblaze accepted and stored every key exactly as sent**,
including `//`, `.` and `..` (the version listing showed the same 1,040 keys).

**Through the Rust listing** (`browser::list_folder`, the dashboard's own
path; WSL2 3 runs, native Windows 11 1 run, plus the creating step of every
UI run below): **133/133 checks every time.** Verified at Backblaze:

| Behaviour | Result |
| --- | --- |
| bucket root, prefix-restricted key | `AccessDenied` ("not entitled") |
| `sotyware-it/browse-validation/` | only the run's folder, no files |
| each of the 26 folders | exactly its immediate subfolders and files, sizes as uploaded, every date from this run (RFC 3339, `Z`), nothing listed twice; every folder reachable by navigation, none extra |
| nested folders | appear only as their top folder; deeper levels need their own listing |
| marker objects | `photos/` lists 1 file (not its marker); `marker-only/` and `docs/empty-nested/` open empty, one page |
| special characters, Unicode | `+ & ? # %`, `%20`, NFC and NFD `café` (two files), emoji, Cyrillic/Arabic/Japanese preserved byte for byte; the folder `f+o&l?d#e%r` opens by its returned prefix |
| `..` and `.` | `dots/` holds folders `..` and `.` and files `..`, `..name.txt`; `dots/../` lists `escape.txt` and its parent is `dots/`; `escape.txt` never at the run root |
| repeated separators | `double/` holds one folder without a name (`double//`), which holds `inner.txt` and another such folder |
| pagination (`many/`, 1,010 entries) | 2 pages: 1,000 files, then 5 files + 5 folders (key order); all entries, none twice, no further page |
| cursors | a used cursor → `outdated-page`; a cursor of `many/` used for `docs/` → `outdated-page`; Refresh → the same first page, new cursor; `docs/` after `many/` opens at its first page |
| the provider's own pages of 4, 7 and 1,000 | the same entries, none twice |
| requests | only `GET ?list-type=2&delimiter=%2F…` for listings (never recursive, no `ListBuckets`) |
| HEAD of every key outside `many/` | the uploaded size and SHA-256 (30 keys) |

**Differences from the in-process listing server** (the same keys loaded into
it, every folder compared, including order, page splits, parents and
breadcrumbs): **none**, except one: a continuation token Backblaze never
issued is **accepted** and answered with the folder's first page (1,000
entries, "more": true); the local server answers 400, which sotyware shows
as an out-of-date page. The app sends only tokens the provider issued, and
Load more drops rows it already shows; an expired real token was not tried.

**Through the desktop UI** (the release binary, read-only key; every row of
every visited folder compared with the provider's listing: name, size as the
page formats it, date formatted by the page's own `toLocaleString` call;
navigation as listed in [testing.md](../development/testing.md#cloud-storage-ui-end-to-end-test)):

| Platform | Runs | Result |
| --- | --- | --- |
| WSL2: Xvfb + openbox, private D-Bus, real GNOME Keyring (not a real desktop) | 5 | ✅ 30/30 UI checks and all 4 script checks in runs 1, 3 and 4. Run 2: 30/30 UI, but ❌ "`sotyware-it/` as before the dataset": a Windows run's dataset existed at the same time (the fingerprint covers all of `sotyware-it/`; its own cleanup was complete). Run 3 overlapped the same Windows run, which existed before and after it. Run 5: ❌ **Backblaze's daily transaction cap** (below) |
| Native Windows 11, UI Automation, real Credential Manager | 4 | Run 1: ❌ 26 passed, 20 failed: every name and size matched, but the script formatted dates in the regional format (fr-FR, `11 nov. 2026, 20:30`) while WebView2 shows the display language (en-US, `Nov 11, 2026, 08:30 PM`), a script bug, fixed (`Get-UICulture`); the failed checks also delayed later steps, and its fingerprints overlapped a Linux run. Runs 2–4 (alone): ✅ **46/46** each |

Checked in each passing UI run: the read-only key **Connected**; Open bucket at
the root lists `sotyware-it`; navigation to the run; the run root and every
visited folder with every row equal to Backblaze's listing (`docs/` five
levels down, the `2024` breadcrumb, Up one folder, both marker-only folders
empty, `photos/` without its marker, `special/` and `f+o&l?d#e%r/`, both
Unicode folders, `dots/` → `..` → `escape.txt` → Up to `dots/`, `double/` →
(no name) → (no name)); `many/` 1,000 rows with Load more, Load more → 1,010
rows, no Load more, Refresh → 1,000; Back to accounts and Open bucket again
→ root; the account removed from page, database and credential store; no
key in sotyware's output; the fingerprint after creating = after browsing
(the UI wrote nothing).

**Cleanup.** 13 runs created a dataset (Rust only: 3 on WSL2, 1 on Windows;
UI: 5 on WSL2, 4 on Windows). In **12** of them every version and delete
marker under the run prefix was listed, each matched a dataset key of that
run, each was deleted by version id (1,040 versions, no delete markers: only
`PutObject` was used), and the prefix was listed again: **0 left**. Nothing
outside the run prefixes was touched (fingerprint of `sotyware-it/` 0
versions before and after every run that did not overlap another).

**The 13th run (WSL2 UI run 5) was cleaned up only the next day.** It created all 1,040
objects under `sotyware-it/browse-validation/1794428166750019532-401942/`
(the version listing right after creation showed 1,040 versions, one per
dataset key). Its checks then failed with Backblaze's `AccessDenied`
"Transaction cap exceeded, see the Caps & Alerts page to increase your
cap": the account's daily cap for the listing transactions was reached
(about 20:15 UTC), and the cleanup's version listing is one of those
transactions, so it was refused twice (in the test and in the script's exit
handler). Nothing of that run was deleted. Its version ids could not be
listed then; the keys are the 1,040 dataset keys under that prefix (no
probe object: the read-only key's write was refused, "not entitled").
✅ **Cleaned up 2026-11-12, 00:05 UTC** (WSL2, first attempt after the cap
reset): `backblaze_b2_browse_cleanup` with
`SOTYWARE_IT_B2_BROWSE_RUN=1794428166750019532-401942` listed 1,040
versions and 0 delete markers under the run prefix, all 1,040 belonging to
this run and 0 that did not; deleted all 1,040 by version id, 0 deletes
failed; the listing again showed **0 left**. `backblaze_b2_fingerprint`
right after: `sotyware-it/` **0 versions** (the empty fingerprint, as before
the first dataset). So all 13 runs are cleaned up. Each run lists a lot:
about 200 listing transactions for the Rust checks (most from walking
`many/` in pages of 7) plus about 50 for a UI run.

**Regression** (the tree with these changes):

| Check | WSL2 (Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0) | Native Windows 11 (Home 25H2, build 26200.9711; copy in `C:\dev\sotyware-b2browse`) |
| --- | --- | --- |
| fmt, clippy, `npm run check`, `npm run build`, release build | ✅ | ✅ |
| `cargo test` / `npm run test:headless` | ✅ 209 passed, 10 ignored (1 new, 2 new ignored) | ✅ 200 passed, 10 ignored |
| Frontend (Vitest) | ✅ 81 | ✅ 81 |
| `npm run test:desktop:virtual` | ✅ 119/0/0, 3 runs | — |
| `cloud-ui.sh local hang browse` | ✅ 16/16, 8/8, 17/17, 3 runs | — |
| `windows-isolated.ps1` | — | ✅ 56/0, 3 runs (it again left an empty `cloud-accounts.sqlite3` in `%LOCALAPPDATA%\com.sotyware.desktop`; it holds 0 accounts; deleting it was left to the user) |
| `windows-cloud-ui.ps1` local / `-Mode browse` | — | ✅ 19/19 / 29/29, 3 runs each |
| `windows.ps1` (real desktop, moves the mouse) | — | — not run this time (it moves the user's mouse); run on 2026-11-12, 103/0 three times ([details](#real-windows-desktop-and-fewer-backblaze-listings-2026-11-12)) |

WSL2 is not a real Linux desktop: the Linux UI runs are Xvfb + openbox with a
private D-Bus session and a throwaway GNOME Keyring.

Not verified: Hetzner; an expired (rather than made-up) continuation token;
folders with more than 1,010 entries or more than two pages through the UI;
keys that are not valid XML text; the browser on a real Linux desktop, on
Windows 10, or with a screen reader.

### Cloud Storage UI test: locked keyring with a system prompter (2026-11-11)

Tested as an uncommitted working tree on top of `ad014e7`, together with the
bucket browsing change below; test-only (`tests/desktop/cloud_ui.py`), no app
code changed. The two locked-keyring checks of `cloud-ui.sh local` failed on
WSL2 (below) because the apt-installed gnome-keyring ships
`org.gnome.keyring.SystemPrompter.service`: the private session bus started
`/usr/libexec/gcr-prompter` (seen in the process list during the check), and
its unlock dialog waited for a person, so "Check connection" never finished.
The driver now cancels that dialog through AT-SPI and still requires
**Credential store locked** and a removal that keeps the account.

| Check (WSL2, Ubuntu 24.04.1, gnome-keyring 46.1-2ubuntu0.2) | Result |
| --- | --- |
| `tests/desktop/cloud-ui.sh` (local, hang, browse) | ✅ exit 0, 3 runs: local 16/16 (1 prompt declined during the check, 1 more during the removal; the account was kept), hang 8/8, browse 17/17 and the server check |
| the same with an unpacked gnome-keyring (no prompter) | — not run again since the change |

### Cloud Storage bucket browsing (2026-11-11)

Tested as an uncommitted working tree on top of `ad014e7` (not committed; the
tree was clean before this change). New: `list_cloud_folder`
(`src-tauri/src/cloud/browser.rs`, `S3Store::list_folder`, listing cursors in
`CloudAccounts`), the page `src/CloudBrowser.svelte` with **Open bucket** per
account, an in-memory listing backend for the in-process S3 test server, the
opt-in `backblaze_b2_folder_listing`, and a `browse` phase / `-Mode browse`
in the Cloud Storage UI tests
([cloud-storage.md](../architecture/cloud-storage.md#browsing-a-bucket),
[ADR-019](../architecture/decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust)).
No dependency added.

**WSL2** (Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0):

| Check | Result |
| --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check`, `npm run build`, release build | ✅ |
| `cargo test` | ✅ 208 passed, 8 ignored (19 new: 18 in `cloud/browser/tests.rs`, `folder_listing_cases_against_the_test_server`; 2 new ignored: `backblaze_b2_folder_listing`, the UI tests' server helper `serve_listing_for_ui`) |
| Frontend (Vitest) | ✅ 81 (18 new in `CloudBrowser.test.ts`, 1 changed in `CloudStorage.test.ts` for the new button) |
| Mutants (each reverted) | caught: folder marker shown as a file (3 tests fail), no delimiter sent (1), cursor not retired after use (2), stale answer not dropped (1 frontend), no de-duplication on Load more (3 frontend) |
| `npm run test:desktop:virtual` | ✅ 119/0/0, 3 runs (unchanged suite: regression only) |
| `cloud-ui.sh browse` (release binary, real GNOME Keyring, local listing server) | ✅ 17/17 and the server check (only `ListObjectsV2`, 12 requests), 4 runs: open bucket, folder with spaces and Unicode, bucket breadcrumb, `photos/` without its marker or nested file, `photos/2024/`, Up, marker-only folder empty, 1,005 entries → 1,000 + Load more → 1,005, Refresh → 1,000, Back to accounts |
| `cloud-ui.sh hang` | ✅ 8/8, 3 runs |
| `cloud-ui.sh local` | ❌ (fixed in the test since: [above](#cloud-storage-ui-test-locked-keyring-with-a-system-prompter-2026-11-11)) 14 passed, 2 failed, 3 runs: the new Open bucket checks (network error, Try again, back) pass; the two **locked-keyring** checks fail. They fail the same way with the unchanged `ad014e7` binary and script (12 passed, 2 failed), so not because of this change. Likely cause: `gnome-keyring` was installed system-wide on this machine at 18:39 today (`/var/log/dpkg.log`); the check expects the unlock prompt to be refused at once, and it no longer is. Not investigated further |
| `backblaze_b2_folder_listing` (**real Backblaze B2**, read-only) | ✅ once, see below |
| `cloud-ui.sh b2` (**real Backblaze B2** through the UI) | ✅ 21/21 and the fingerprint check, once; Open bucket at the root → Access denied |

**Native Windows 11** (Home 25H2, build 26200.9711; a plain copy of the tree
in `C:\dev\sotyware-browse`):

| Check | Result |
| --- | --- |
| fmt, clippy, `npm run check`, `npm run build`, release build | ✅ |
| `npm run test:headless` | ✅ 199 Rust passed, 8 ignored (the Linux-only tests are not built there) + 81 frontend |
| `windows-cloud-ui.ps1` (local: real Credential Manager, endpoint nobody listens on) | ✅ 19/19, 3 runs (Open bucket → network error with Try again, back) |
| `windows-cloud-ui.ps1 -Mode browse` (local listing server, UI Automation) | ✅ 29/29, 3 runs (the same browsing steps as on Linux; Check connection: Connected); one earlier run 28/1 failed on a bug in the script (exit code of the server process not read), fixed before the 3 runs |
| `windows-cloud-ui.ps1 -Mode b2` (**real Backblaze B2**, real Credential Manager) | ✅ 25/25, once; Open bucket at the root → Access denied; fingerprint unchanged |
| `backblaze_b2_folder_listing` (**real Backblaze B2**) | ✅ once, same results as on WSL2 |
| `windows-isolated.ps1` | ✅ 56/0, 3 runs (it left an empty `cloud-accounts.sqlite3` in `%LOCALAPPDATA%\com.sotyware.desktop`, where none had been; removed after checking it held 0 accounts) |
| `windows.ps1` (real desktop, moves the mouse) | ✅ 103/0, once |

**Real Backblaze B2, read-only** (the dedicated test bucket in
`eu-central-003`; the key is restricted to that bucket and to the file name
prefix `sotyware-it/`), WSL2 and Windows, same results:

| Case | Result |
| --- | --- |
| root (`prefix=""`, delimiter `/`) | `403 AccessDenied` ("not entitled"): a prefix-restricted key cannot list the root; the browser shows Access denied |
| `sotyware-it/` | `200`, 0 folders, 0 objects (the test prefix is empty) |
| `sotyware-it/` in pages of one entry | 1 page, 0 entries |
| a continuation token Backblaze never issued | `200`, accepted, 0 entries (not an error, at least on an empty prefix) |
| the same through `list_folder` (account, in-memory credential store, cursor) | `sotyware-it/` listed (empty), root → `access-denied` |
| requests | 5, all `GET ?list-type=2&delimiter=%2F…`; no `ListBuckets`, nothing written |
| object versions under `sotyware-it/` before / after | 0 / 0, fingerprint identical |

Both real-provider runs were started by the agent this time: the key came
from the user's files outside the repository (`~/.config/sotyware-it/b2.env`,
mode 600, on WSL2; the DPAPI-encrypted `%USERPROFILE%\.sotyware-it\b2.xml` on
Windows) through scripts that removed key id, secret and bucket name from the
output.

So against the real provider only the refusal, an empty folder and the
request shape were seen. **Not verified at any real provider:** a folder with
objects, subfolders, marker objects, special characters in real keys,
pagination (more than 1,000 entries, or any continuation token Backblaze
issued), last-modified dates as Backblaze formats them, a key without a name
prefix (root listing), and Hetzner. Writing test objects to the bucket was
out of scope for this change (the listing test must not write).

Not run: Windows 10, real Linux desktops, KWallet, screen readers.

### Cloud Storage UI end-to-end and Backblaze connection check (2026-11-11)

Tested as an uncommitted working tree on top of `6af454d` (not committed).
New: the opt-in tests `backblaze_b2_connection_check` and
`backblaze_b2_fingerprint`, the always-run
`connection_check_cases_against_the_test_server`, a test-only SDK
interceptor hook (`S3Store::with_interceptor`), and the opt-in UI tests
`tests/desktop/cloud-ui.sh` (+ `cloud_ui.py`, `fake_secret_service.py`, the
earlier out-of-repository script brought in) and
`tests/desktop/windows-cloud-ui.ps1`
([testing.md](../development/testing.md#cloud-storage-ui-end-to-end-test)).
No app code changed, no dependency added.

**Real Backblaze B2, connection check: ✅ run once by the user on WSL2
(2026-11-11)**, `backblaze_b2_connection_check` against the dedicated test
bucket in `eu-central-003` with the bucket-restricted, prefix-restricted test
key. The agent session itself could not reach the key (its tool sandbox
refused to read `~/.config/sotyware-it/b2.env`); the user loaded it into
their own shell and pasted the output, in which the key and bucket are
redacted. Requests as recorded by the SDK interceptor:

| Case | Requests sent | Status |
| --- | --- | --- |
| the key and its bucket | `HEAD /<bucket>/ -> 200` | **connected**: Backblaze answers `HeadBucket` for a bucket-restricted key; no listing needed |
| wrong secret | `HEAD -> 403`, `GET ?list-type=2&max-keys=1 -> 403` ("Signature validation failed") | invalid credentials |
| unknown key id (`003` + zeros) | `HEAD -> 403`, `GET ?list-type=2&max-keys=1 -> 403` ("The key … is not valid") | invalid credentials |
| a bucket name new for the run | `HEAD -> 404` | **bucket unavailable** (B2 says not found, not access denied, for a bucket the key cannot use) |
| `http://127.0.0.1:9` | `HEAD`, connection refused | network |

No `ListBuckets`, only `HEAD` and one-key `GET`s; 0 object versions under
`sotyware-it/` before and after, fingerprint identical (the prefix was empty,
so this shows nothing was created, not that existing objects survived).
Whole test 1.39 s. Not run on Windows; one run, not three.

**Real Backblaze B2 through the UI on Linux: ✅ `tests/desktop/cloud-ui.sh
b2`, run once by the user on WSL2 (2026-11-11)**, release binary on Xvfb +
openbox, real GNOME Keyring 46.1 (unpacked packages, throwaway keyring),
same bucket and key, output pasted with key and bucket redacted. **19/19**:
add a Backblaze B2 account through the form (name, bucket, region
`eu-central-003`, key) → the keyring item `cloud-account/sotyware-e2e-<run>`
holds exactly the typed key, the database has the account with the default
endpoint and no key, the page shows provider, bucket and region but not the
key → Check connection **Connected** (the bucket-restricted key) → rename
without a key → still Connected → replace the key with a wrong secret (one
entry, that key) → **Invalid credentials** → the real key back →
**Connected** → bucket changed to a name the key cannot use → **Bucket
unavailable** (not invalid credentials) → bucket set back → Connected →
Remove (the confirmation names the bucket and says it is not deleted or
changed) → gone from page, database and keyring; no key id or secret in
sotyware's output. Then `backblaze_b2_fingerprint` after the run equal to
before: 0 versions under `sotyware-it/` (so: nothing created; the prefix was
empty). The throwaway keyring was removed. One run, not three.

**Real Backblaze B2 through the Windows UI: ✅ `windows-cloud-ui.ps1 -Mode
b2`, run once by the user on native Windows 11 Home 25H2 (build 26200.9711,
2026-11-11)** in `C:\dev\sotyware-cloud-b2` (the copy and release binary
above), key from the DPAPI-encrypted `b2.xml`, output pasted (the summary
line was not in the paste; all 23 check lines PASS, none FAIL). The **real
Windows Credential Manager and real Backblaze together**: bucket
fingerprinted → add a Backblaze B2 account through the UI →
`com.sotyware.desktop/cloud-account/sotyware-e2e-<run>` holds exactly the
typed key (`CredRead`, compared in the script), `cmdkey` lists one entry,
page and database hold no key → Check connection **Connected** → rename →
still Connected → replace the key with a wrong secret (entry updated, still
one) → **Invalid credentials** → the real key back → **Connected** → Remove
(confirmation names the bucket and says it is not deleted or changed) → gone
from the page, `CredRead` and `cmdkey`; process responding; `cmdkey /list`
`com.sotyware.desktop` lines 0 before and after; fingerprint after equal to
before (0 versions under `sotyware-it/`). The Windows script has no
other-bucket step (the Linux one does). One run, not three.

**Local and mock testing, WSL2 (Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0):**

| Check | Category | Result |
| --- | --- | --- |
| `npm run check` (0/0), `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, release build | static | ✅ |
| `npm run test:headless`: frontend 63; `cargo test` **189 passed, 6 ignored** (188 and 4 before: +1 `connection_check_cases_against_the_test_server`, +2 ignored B2 tests) | headless | ✅ |
| The five connection-check cases against the in-process server with `ListBuckets` refused, requests recorded by the new interceptor: valid key → connected, `HEAD /<bucket>/ -> 200` only; wrong secret and unknown key id → invalid credentials via `HEAD 403` then one `GET ?list-type=2&max-keys=1 -> 403`; a bucket that does not exist → bucket unavailable (`HEAD 404`); `127.0.0.1:9` → network. No `ListBuckets`, only `HEAD`/`GET` | headless | ✅ |
| `cloud-ui.sh` (default phases), release binary, Xvfb + openbox, private D-Bus session, **real GNOME Keyring 46.1** (unpacked packages, throwaway keyring): `local` 14/14 (the earlier one-off script's checks) and `hang` 8/8; the throwaway folder and keyring removed afterwards, no daemon left | real Secret Service, virtual display | ✅ 3/3 runs |
| `npm run test:desktop:virtual` (regression; `EXPECTED_CHECKS` stays 119) | virtual display | ✅ 119/0/0, 3 runs |
| `cloud-ui.sh b2` against Backblaze (run by the user, above) | real provider, real Secret Service, virtual display | ✅ 19/19 + bucket unchanged, 1 run |

**Native Windows UI validation, Windows 11 Home 25H2 (build 26200.9711),
Rust 1.99.1 MSVC, Node 24.14.1**, a new copy of the tree
(`C:\dev\sotyware-cloud-b2`, not a git checkout; `C:\dev\sotyware` not
touched), not elevated:

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0/0), `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, release build | static | ✅ |
| `npm run test:headless`: frontend 63; `cargo test` 180 passed, 6 ignored | headless | ✅ |
| `windows-cloud-ui.ps1` (local mode), **through the app's UI with the real Windows Credential Manager**: Cloud Storage → add a Backblaze B2 account (name, bucket, region, key, an endpoint nobody listens on) → `CredRead` of `com.sotyware.desktop/cloud-account/sotyware-e2e-<run>` equals exactly the typed key (compared in the script, never shown), `cmdkey /list:<target>` lists one entry, page and database hold no key id or secret → Check connection → **Network error** → rename (entry unchanged) → Replace access key (one entry, now the new key) → Remove (confirmation names the bucket and says it is not deleted or changed) → gone from page, `CredRead` and `cmdkey`; same `com.sotyware.desktop` lines in `cmdkey /list` before and after (0 and 0) | real desktop session, real OS backend | ✅ **17/17, 3 runs** |
| Found while writing it and fixed before those runs (script bugs, not sotyware's): the Region input has a datalist, so UI Automation exposes it as a ComboBox; PowerShell names ignore case (`$Value`/`$value`, `$S`/`$s`); `cmdkey /list:<target>` names the target in its header even when nothing is stored | — | ❌ → ✅ |
| `windows-isolated.ps1` (regression) | private desktop | ✅ 56/0, 3 runs |
| `windows.ps1` (regression, moves the mouse; your databases moved aside and put back) | real desktop | ✅ 103/0, 3 runs |
| Afterwards: `%LOCALAPPDATA%\com.sotyware.desktop` holds only what it held before (`preferences.sqlite3`, `EBWebView`); the empty `cloud-accounts.sqlite3` the isolated runs left (0 accounts, checked) was removed; no `com.sotyware.desktop` entry in `cmdkey /list` | — | ✅ |
| `windows-cloud-ui.ps1 -Mode b2` (Connected, wrong-key replacement → Invalid credentials, real key back → Connected, bucket fingerprint unchanged) | — | — not run (no key) |

**Connection-check edge cases at Backblaze:** correct key and bucket,
wrong secret, unknown key id, a bucket the key cannot use, unreachable
endpoint, no `ListBuckets`, nothing created: verified once on WSL2 (table
above). "A valid key without permission on an existing bucket" cannot be
produced with one key and one bucket, and the `ListObjectsV2` fallback was
never reached at Backblaze (every refused HEAD was settled by it only for bad
keys); whether B2 would refuse that prefix-less listing to a valid
prefix-restricted key is therefore still unknown. A network failure at the
provider itself (DNS, TLS, an outage) is not simulated. Hetzner: nothing.

### Cloud Storage dashboard (2026-11-11)

Tested as an uncommitted working tree on top of `c022b4e` (not committed).
Code and design: [cloud-storage.md](../architecture/cloud-storage.md#dashboard),
[ADR-018](../architecture/decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands).
No dependency was added (`Cargo.lock` and `package-lock.json` unchanged).

**WSL2, Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0:**

| Check | Category | Result |
| --- | --- | --- |
| `npm run check` (0 errors, 0 warnings), `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | static | ✅ |
| `cargo test`: 188 passed, 4 ignored (160 before; 28 new in `cloud::dashboard`, list in [testing.md](../development/testing.md#rust-unit-tests)); every existing cloud backend test still passes | headless | ✅ |
| Connection checks against the in-process S3 server with refused operations: valid key → one `HeadBucket`; key refused `ListBuckets` → connected, no `ListBuckets` sent; `HeadBucket` refused → one `ListObjectsV2`, connected; both refused → access denied (not invalid credentials); wrong secret / unknown key → invalid credentials; missing bucket; unreachable endpoint; missing/unavailable/locked credentials send no request | headless | ✅ |
| A credential store that blocks (as during an unlock prompt) leaves a single-threaded async runtime free during add and check | headless | ✅ |
| Frontend: 63 passed (38 before; 25 in `CloudStorage.test.ts`) | headless | ✅ |
| `npm run test:desktop:virtual` on the release binary: **119 passed, 0 failed, 0 skipped, 3 runs** (4 new checks: navigation links, the Cloud Storage page with its empty state and only "Add account" through the real capability, the account database created with no accounts, back to the overview) | virtual display | ✅ 3/3 |
| One-off end-to-end run of the release binary through its UI (script not in the repository: Xvfb + openbox, private D-Bus session, AT-SPI to find and press controls, XTest keystrokes to type, since this WebKitGTK exposes no AT-SPI `EditableText`), against a **real GNOME Keyring 46.1** (`gnome-keyring-daemon --unlock`, throwaway keyring, the Ubuntu packages unpacked as in [testing.md](../development/testing.md#os-credential-store-test)), dashboard-only mode: add an account with an endpoint nobody listens on → the Secret Service item `cloud-account/photos` holds exactly the typed key (read back over D-Bus), the database row has no key id or secret (file bytes checked), the page shows neither; Check connection → **Network error**; rename → keyring entry unchanged; Replace access key → still one entry, now the new key; Remove → confirmation text, then gone from page, database and keyring; a second account, the login collection locked over D-Bus → Check shows **Credential store locked**, Remove reports the error and keeps the account and its key; no key id or secret in sotyware's output | real Secret Service, virtual display | ✅ 14/14, 3 runs |
| Same script against a fake Secret Service that never answers `OpenSession`: the add shows "Saving…"; meanwhile the Overview link switches sections within 5 s and the example feature can still be disabled (another command); back on Cloud Storage the page shows "Loading…" (the account list waits behind the pending add, [limitations](limitations.md#cloud-storage)); the process keeps running | virtual display | ✅ 7/7, 3 runs |
| Found by that run and fixed before the results above: labels wrapping their inputs gave each input an accessible name with its placeholder or value appended ("Name Photos"); the form now uses `for`/`id` pairs | virtual display | ❌ → ✅ |
| Release binary 7,471,224 → 10,656,632 bytes (+3.19 MB: the S3 SDK and rustls are now linked, as the app calls them) | static | ✅ recorded |
| `tests/desktop/run.sh` (WSLg), a real Linux desktop session, an unlock prompt answered by a person, KWallet | — | — not run |

**Native Windows 11 Home 25H2 (build 26200.9711), Rust 1.99.1 (MSVC), Node
24.14.1, npm 11.12.1**, a separate copy of the same tree (`C:\dev\sotyware-cloud-ui`,
not a git checkout; later edits not copied: doc comments in `lib.rs` and
`storage.rs`, `EXPECTED_CHECKS` in `desktop-virtual.yml`, the documentation):

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0/0), `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, release build (`--no-bundle`) | static | ✅ |
| `npm run test:headless`: frontend 63 passed; `cargo test` 179 passed, 4 ignored (151 before; the Linux-only tests do not exist there) | headless | ✅ |
| `windows-isolated.ps1`: 56 passed, 0 failed, not elevated | private desktop | ✅ 3/3 |
| `windows.ps1`: **103 passed, 0 failed** (99 before; 4 new: navigation links through UI Automation `Hyperlink`/`Invoke`, heading, empty state, "Add account", no Hide button and no error on the Cloud Storage page, `cloud-accounts.sqlite3` created, back to the Hide button and features); the existing preferences database moved aside and put back each run, unchanged afterwards (size and timestamp checked) | real desktop | ✅ 3/3 |
| Credential Manager after all runs: no `com.sotyware.desktop` entry (`cmdkey /list`); the empty `cloud-accounts.sqlite3` the isolated runs created (0 accounts, checked) was removed afterwards | real desktop | ✅ |
| Adding, checking, editing or removing an account through the Windows UI (Credential Manager through the dashboard); Windows 10; the NSIS/MSI installers | — | — not run |

**Not run anywhere in this section:** the connection check against
**Backblaze B2** or **Hetzner** (the opt-in round trip now calls
`check_bucket`, but no credentials were available in this session; the
Backblaze check ran later,
[above](#cloud-storage-ui-end-to-end-and-backblaze-connection-check-2026-11-11)); whether Backblaze answers
`HeadBucket` for a bucket-restricted key is therefore unconfirmed),
`desktop-virtual.yml` on GitHub (now expects 119), and a person's visual
check of the new section in either theme.

### OS credential store (2026-11-11)

Tested as an uncommitted working tree on top of `de2e7f3` (not committed
yet). Code: `cloud/credentials.rs`, `cloud/keyring.rs`, `cloud/account.rs`
([cloud-storage.md](../architecture/cloud-storage.md#credentials), ADR-017).
The app still does not call the module.

**Linux: WSL2, Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0.** No desktop
keyring is installed and `DBUS_SESSION_BUS_ADDRESS` points to
`/run/user/1000/bus`, which does not exist.

| Check | Category | Result |
| --- | --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check` (0/0), `npm run build`, `npm run tauri build -- --no-bundle` | static | ✅ |
| `npm run test:headless`: 38 frontend and **160 Rust tests** (140 before: +5 credentials, +7 keyring of which 1 Linux-only, +8 accounts; one S3-server test replaced by a key-rotation/removal test), 4 ignored (3 before + the OS keyring test) | headless | ✅ |
| Dependency graph: Linux 365 → 379 crates; zbus feature set unchanged (`cargo tree -e features -i zbus` identical before and after) | static | ✅ |
| Release binary 7,469,688 → 7,471,224 bytes (+1,536; the module is not called) | static | ✅ recorded |
| `npm run test:desktop:virtual` on the new release binary (regression) | virtual display | ✅ 115/0/0, 3 runs |
| Real backend, **no Secret Service** (opt-in test with `SOTYWARE_IT_KEYRING_EXPECT=unavailable`): (a) the WSL2 bus address with no socket, (b) no bus address and a missing runtime folder, (c) a real `dbus-daemon` (`dbus-run-session`) without a provider (`ServiceUnknown`). Every call `Unavailable` | headless, real zbus/D-Bus | ✅ each case. The first run of (a) failed: zbus's `Connection` error was reported as `Backend`; fixed in the mapping, unit test added |
| Real backend, **GNOME Keyring 46.1** (Ubuntu's `gnome-keyring` 46.1-2ubuntu0.2 with `libgck-1-0`/`libgcr-base-3-1` 3.41.2, unpacked without root, SHA-256 checked against the apt index) on a private `dbus-run-session` bus, throwaway HOME and login keyring, unlocked with `--unlock`: opt-in round trip (missing → store → read → second account → update → delete → gone) | headless, real Secret Service | ✅ 3/3 runs on the final code (plus 4 earlier passing runs before the last test edits) |
| Independent check after those runs: `SearchItems` over D-Bus for `service=com.sotyware.desktop` | real Secret Service | ✅ 0 items |
| Login collection locked over D-Bus, then the round trip: reads of missing entries `NotFound`; the first write asked for an unlock prompt that cannot be shown there and failed with `AccessDenied` ("prompt dismissed"); 0 items afterwards | real Secret Service | ✅ as designed (the test is expected to fail at that step) |
| A real Linux desktop session (GNOME, KDE/KWallet, KeePassXC), an unlock prompt answered by a person | real desktop | — not run (no Linux desktop available) |
| B2 integration test with the test-only `EnvCredentials` | — | — not run (no credentials in this session); compiles, and the `EnvCredentials` unit tests pass |

**Windows: native Windows 11 Home 25H2, build 26200, Rust 1.99.1 MSVC, Node
24.14.1**, not elevated, PowerShell 5.1 from WSL with the registry `PATH`; a
copy of the working tree in a new folder `C:\dev\sotyware-keyring` (files
checked identical by SHA-256).

| Check | Category | Result |
| --- | --- | --- |
| First `cargo clippy`: `PlatformKind::AccessDenied` never constructed on Windows (`-D warnings`); fixed with a Linux-only `allow(dead_code)` | static | ❌ → ✅ |
| `npm ci`, `npm run check` (0/0), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run tauri build -- --no-bundle` (`sotyware.exe` 5,494,272 bytes, unchanged) | static | ✅ |
| Windows graph 288 → 290 crates (`keyring-core`, `windows-native-keyring-store`); no `zbus`/`secret-service` | static | ✅ |
| `npm run test:headless`: 38 frontend and **151 Rust tests** (the 9 Linux-only tests excluded), 4 ignored | headless | ✅ |
| **Real Windows Credential Manager**, opt-in round trip with two dedicated test accounts (missing → store → read → second account → update → delete → gone, fresh store sees nothing) | real OS backend | ✅ 3/3 runs |
| Independent check in each run with `cmdkey /list:<target>`: not listed before storing, listed as `com.sotyware.desktop/cloud-account/<id>` after storing, not listed after deleting | real OS backend | ✅ 3/3 (the first 3 attempts failed in the test's own `cmdkey` parsing, which matched the header line; fixed before these runs) |
| `cmdkey /list` lines containing `com.sotyware.desktop` before and after all runs | real OS backend | ✅ 0 and 0 |
| `npm run test:desktop:windows:isolated` on the new release binary (regression) | private desktop | ✅ 56/0, 3 runs |
| `windows.ps1` (real desktop), Windows 10, a roaming/domain profile, `Local` persistence checked from outside, a second process reading the entry | — | — not run |

### Backblaze B2 against the real provider (2026-11-11)

Tested as the working tree of `3c992d2` (parent `18f6457`), before it was
committed; on Windows the same test change was applied as a patch on
`18f6457` (the comment-only `provider.rs` edit and the docs were not copied).
These are **real requests to Backblaze**, not the local test server.

Target: a dedicated private test bucket (Object Lock off) in the user's
Backblaze account in EU Central, reached at
`https://s3.eu-central-003.backblazeb2.com`, path style. Key: an application
key restricted to that bucket, read and write, file name prefix
`sotyware-it/`, without "list all bucket names". Each run used its own prefix
`sotyware-it/<nanoseconds>-<pid>/`.

What `backblaze_b2_round_trip` checked against Backblaze, in every run:

| Check | Verified behaviour |
| --- | --- |
| Endpoint and region | `s3.eu-central-003.backblazeb2.com` accepted SigV4 requests signed for region `eu-central-003`; no profile change needed |
| `ListBuckets` | refused for this key: `AccessDenied`, message "not entitled" (expected for a bucket-restricted key; reported, not failed) |
| Run prefix new | no objects and no versions under it before the run |
| Single PUT (200,000 bytes) | 1 part; Backblaze returned a version id; `HeadObject` returned the stored SHA-256 metadata |
| Multipart (12,582,919 bytes, 5 MiB parts) | 3 parts; version id returned; SHA-256 metadata stored |
| Download | both files `Sha256Matched` and byte-identical |
| `ListObjectsV2` | both objects listed |
| Plain delete (no version id) | the object disappears from `HeadObject`, listings and `GetObject` (`NotFound`, "Key not found", no partial file left); `ListObjectVersions` shows one new delete marker and the uploaded version still stored |
| Object never uploaded | `HeadObject` → `None` |
| Wrong secret | `InvalidCredentials`, "Signature validation failed" |
| Cleanup | deletes exactly the uploaded versions and the delete marker by version id; afterwards `ListObjectsV2` **and `ListObjectVersions`** under the run prefix are empty |

| Platform | Runs | Result | Timings (upload small / large, download small / large) |
| --- | --- | --- | --- |
| WSL2, Ubuntu 24.04.1, Rust 1.99.1 | 1 with the test as committed in `18f6457`, then 3 with the extension | ✅ 4/4 | e.g. 0.35 s / 3.65 s, 0.37 s / 0.91 s; whole test 7.8–10.6 s (first run 30.5 s) |
| Native Windows 11 25H2, MSVC Rust 1.99.1, checkout `C:\dev\sotyware` | 3 with the extension | ✅ 3/3 | e.g. 0.46 s / 2.74 s, 0.37 s / 0.48 s; whole test 5.2–15.5 s |

The first WSL2 run (before the extension) cleaned up by version id and left no
current object, but its prefix was not checked with `ListObjectVersions`.
Offline suites after the change: `cargo fmt --check`, `cargo clippy
--all-targets -- -D warnings` and `cargo test` (140 passed, 3 ignored on WSL2;
132 passed, 3 ignored on Windows). Frontend tests were not rerun (no frontend
change). Commands: [testing.md](../development/testing.md#cloud-storage-integration-tests).

Not verified at Backblaze: an unknown key id, the SDK's checksum headers
(`WhenRequired` stays), objects above 12 MiB, throttling, interrupted
transfers, Object Lock, lifecycle rules, a key without prefix restriction.

### Cloud storage proof of concept (2026-11-11)

Tested as the working tree of the commit that adds this entry (parent `b0c4300`). No provider account was used:
**no request reached Backblaze or Hetzner**; the opt-in integration tests
were not run.

**Linux: WSL2, Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0.**

| Check | Category | Result |
| --- | --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check` (0/0), `npm run build`, `npm run tauri build -- --no-bundle` | static | ✅ |
| `npm run test:headless`: 38 frontend and 140 Rust tests (89 existing + 51 new: 3 provider, 14 account, 6 credentials, 8 S3 planning/error mapping, 14 against the in-process S3 server, 6 SQLite account store) plus 3 ignored (2 opt-in provider tests, 1 measurement) | headless | ✅ |
| Cloud tests repeated 5 times in a row | headless | ✅ 5/5 |
| No `aws-lc-sys` in the dependency graph (`cargo tree -i aws-lc-sys`) | static | ✅ |
| Release binary 7,454,840 → 7,469,688 bytes (+14.8 KB; the module is not called) | static | ✅ |
| `npm run test:desktop:virtual` on the new release binary (regression: the app's behaviour is unchanged) | virtual display | ✅ 115/0/0, 3 runs |
| Local throughput (`local_throughput`, release, in-process server on the same machine): 268 MB upload in 16 parts 4.51 s (≈ 60 MB/s including hashing), download with verification 3.21 s (≈ 84 MB/s) | measurement | recorded |
| restic 0.19.1 (official binary, `SHA256SUMS` checked) against a local S3 server (`s3s-fs` 0.17, auth on, 127.0.0.1): `init`; backup of 25 MB (2 random "photos", 50 text files); second backup after adding a 20 MB copy and changing one file added 24,463 bytes (deduplication); `snapshots --json` lists both; `restore latest` byte-identical to the source (`diff -r`); restoring the first snapshot gives the old state; no plaintext marker in the stored objects (encryption); wrong password exit 12, missing repository exit 10, wrong S3 secret exit 1 ("Access Denied") | experiment | ✅ |
| `restic check --read-data` against `s3s-fs` | experiment | ❌ "pack … does not exist": `s3s-fs` lists no objects below `data/xx/`, so restic sees no packs (`list packs` → 0, 5 on disk). The same backup in a local-folder repository: `check --read-data` "no errors were found", restore identical. A test-server limitation, not a restic or provider result |

**Windows: native Windows 11 25H2, Rust 1.99.1 MSVC, Node 24.14.1**, copy of
the working tree in a new folder `C:\dev\sotyware-cloud-poc` (not the usual
checkout), not elevated.

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0/0), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run tauri build -- --no-bundle` (`sotyware.exe` 5,494,272 bytes; 5,480,960 before) | static | ✅ |
| No `aws-lc-sys` in the dependency graph | static | ✅ |
| `npm run test:headless`: 38 frontend and 132 Rust tests (the 8 `ksni`/`zbus` tests are Linux-only), 3 ignored | headless | ✅ (after the fix below) |
| First run: `an_unreachable_endpoint_is_a_transient_network_error` failed. Windows reports a refused local connection only after about 2 s of retries, so the test's 2 s connect and read timeouts reported `Timeout` instead of `Network` (still transient). Fixed in the test (10 s timeouts for that test only), not in the adapter | headless | ❌ → ✅ |
| Cloud tests repeated 3 times after the fix | headless | ✅ 3/3 |
| `npm run test:desktop:windows:isolated` on the new release binary (regression) | private desktop | ✅ 56/0, 3 runs |

Not run: the real-desktop Windows test (`windows.ps1`, moves the mouse), any
real provider, restic against a real provider or on Windows, Windows 10, real
Linux desktops.

### Persistent feature preferences (2026-11-11)

Application code of `93f7667`, Linux desktop driver of `7696d44`, Windows
desktop test of `a7bf8f3`.

**Linux: WSL2, Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0.**

| Check | Category | Result |
| --- | --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check` (0/0), `npm run build`, `npm run tauri build -- --no-bundle` | static | ✅ |
| `npm run test:headless`: 38 frontend (29 existing + 9 new) and 88 Rust tests (58 existing − 1 replaced + 31 new: 1 registry, 1 JSON shape, 13 preferences policy, 16 SQLite store incl. 2 restart sequences with the built-in features) | headless | ✅ |
| Release binary size: 7,454,840 bytes (6,339,920 before; bundled SQLite) | static | ✅ recorded |
| `npm run test:desktop:virtual`: **115 passed, 0 failed, 0 skipped**, 3 consecutive runs (96 existing, unchanged, + 19 new in scenarios 17–19) | virtual display | ✅ |
| Real restart persistence (scenario 17): disable → tray Quit (exit 0, no process) → database row 0 → new process shows Disabled; enable → Quit → row 1 → new process shows Running | virtual display | ✅ 3 runs |
| Corrupt database (18): defaults, "Could not load the saved feature choices … not a database" on the page and in the log, switch works but says it was not saved, file byte-identical after Quit | virtual display | ✅ 3 runs |
| Folder in place of the database (19): defaults, warning with `cannot open`, normal Quit | virtual display | ✅ 3 runs |
| The developer's own `~/.local/share/com.sotyware.desktop/` has no `preferences.sqlite3` after the runs (the suite uses a private `XDG_DATA_HOME`) | virtual display | ✅ |
| Not vacuous: a mutant of `save_current` that reads the state before taking the lock survives the concurrency unit test (the bad interleaving cannot be forced from outside); recorded as a design guarantee in ADR-015, not as tested | headless | ⚠️ known gap |
| Observed: with `XDG_DATA_HOME` set to a file, Tauri fails its own setup (`Failed to setup app: Not a directory`, abort) because it creates the webview data folder there; pre-existing, not caused by the database | virtual display | recorded ([limitations](limitations.md#feature-preferences)) |
| `tests/desktop/run.sh` (WSLg), manual checklist step 9, `desktop-virtual.yml` on GitHub (expects 115 since `b4918e1`) | — | — not run (`desktop-virtual.yml` passed later at `f19dd8b` with 119 checks, [CI](#runs-at-f19dd8b-2026-11-13)) |

**Windows: Windows 11 Home 25H2, OS build 26200.9711, x64**, same machine and
tool setup as [below](#built-in-feature-foundation-native-windows-11-2026-11-11)
(Rust 1.99.1 MSVC, Node.js 24.14.1, npm 11.12.1, PowerShell 5.1 from WSL with
the registry `PATH`, not elevated). `C:\dev\sotyware` fast-forwarded from
`7070932` to `7696d44` with `git bundle` (identical hashes, clean tree); the
desktop test ran from the uncommitted script, committed unchanged as `a7bf8f3`.

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0/0), `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | static | ✅ |
| `npm run test:headless`: 38 frontend and 80 Rust tests (the 8 `ksni`/`zbus` tests are Linux-only), including the read-only-file, corrupt-file, migration and concurrent-connection tests on NTFS | headless | ✅ |
| Windows graph: `rusqlite` 0.41.1 → `libsqlite3-sys` 0.39.1; no `ksni`/`zbus` | static | ✅ |
| `npm run tauri build -- --no-bundle` (MSVC compiles the bundled SQLite): `sotyware.exe`, 5,480,960 bytes (4,359,168 at `0a8a8f1`) | static | ✅ |
| `windows.ps1` (99 checks: 90 existing with "Running again (not persisted)" replaced by "still Disabled (persisted)", + 9 new), release binary, no prior database | real desktop | ✅ **99 passed, 0 failed**, 3 consecutive runs |
| Real restart persistence: disable → tray Quit (exit 0, no process) → new process shows Disabled and `preferences.sqlite3` exists → enable → Quit → second new process shows Running | real desktop | ✅ in each of the 3 runs |
| Backup and restore: with a planted database (example stored disabled), the script moved it aside, the run started from defaults (99/0), and the file came back byte-identical (same SHA-256); the planted file was then deleted | real desktop | ✅ 1 run |
| `windows-isolated.ps1`, NSIS/MSI installers, manual checklist step 9, Windows 10 | — | — not run |

### Concurrent-save test on a GitHub-hosted Windows runner (2026-11-11)

`ci.yml`, job `headless (windows-latest)` (run id and commit not in the pasted
log; `main` was at `a1a3b07`): 38 frontend tests passed, Rust **79 passed, 1
failed**. `storage::tests::concurrent_connections_never_corrupt_the_database`
(4 connections, 200 saves each, back to back) panicked: three threads got
`cannot save: database is locked` after the 2 s busy timeout. The same test
had passed on the native Windows 11 machine and on WSL2. Cause: SQLite's file
locking is not fair, and on the runner's disk a connection was starved past
the timeout. The code was behaving as designed: the save failed cleanly
instead of hanging. The test, however, assumed every save succeeds.

Fix: the test now accepts a clean "database is locked" failure and checks the
stored values against the saves that actually succeeded. A new test holds the
write lock from another connection and checks that a save fails after at least
2 s, changes nothing, and works again once the lock is released. Documented in
[features.md](../architecture/features.md#behaviour) and
[limitations](limitations.md#feature-preferences).

| Check | Category | Result |
| --- | --- | --- |
| WSL2: `cargo fmt --check`, clippy `-D warnings`, `npm run test:headless` (38 frontend, 89 Rust) | headless | ✅ |
| Not vacuous: with the busy timeout set to 0 (not committed), 777 of 800 saves failed and the rewritten test still passed, checking every stored value | headless | ✅ 5 runs |
| `ci.yml` on GitHub (Linux, Windows) and native Windows 11 with the fix | — | — not run then; `ci.yml` passed both jobs later at `f19dd8b`, the Windows job including this test ([CI](#runs-at-f19dd8b-2026-11-13)) |

### Built-in feature foundation (WSL2, Ubuntu 24.04.1, Rust 1.99.1, Node 24.22.0, 2026-11-11)

Application code of `e30ac7b` (registry `5e2e475`, frontend `e30ac7b`),
desktop driver of `a9fb338`. This section is Linux evidence only; Windows:
[below](#built-in-feature-foundation-native-windows-11-2026-11-11).

| Check | Category | Result |
| --- | --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | static | ✅ |
| `npm run check` (0 errors, 0 warnings), `npm run build`, `npm run tauri build -- --no-bundle` | static | ✅ |
| `npm run test:headless`: 29 frontend (13 existing + 16 new) and 58 Rust (31 existing + 27 new) tests | headless | ✅ |
| Not vacuous: 3 hand-made mutants of `Features.svelte` (no checkbox restore, no in-flight lock, no state update) each fail ≥ 1 frontend test | headless | ✅ |
| `npm run test:desktop:virtual`: 96 passed, 0 failed, 0 skipped (92 existing + 4 feature checks), 3 runs | virtual display | ✅ |
| Not vacuous: with `allow-set-feature-enabled` removed from the capability, the run fails exactly the two switch checks (94/2/0; the page shows "Command set_feature_enabled not allowed by ACL") | virtual display | ✅ |
| `tests/desktop/run.sh` (WSLg), manual checklist step 9 | — | — not run |

### Built-in feature foundation, native Windows 11 (2026-11-11)

Application code of `0a8a8f1` (unchanged since `e30ac7b`), Windows desktop test
of `3a8a715`. `C:\dev\sotyware` was fast-forwarded from `44be84e` to `0a8a8f1`
and then `3a8a715` with `git bundle` (identical hashes, clean tree before and
after). Windows 11 Home 25H2, OS build 26200.9711, x64; Windows PowerShell
5.1.26100.9711 started from WSL through interop with the registry `PATH`, so
every tool was the native Windows binary: Rust/Cargo 1.99.1
(`stable-x86_64-pc-windows-msvc`, rustup 1.29.3), Node.js 24.14.1, npm 11.12.1,
Tauri CLI 2.12.4, Git 2.47.1.windows.1, WebView2 Runtime 157.0.4341.46. Not
elevated.

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0 errors, 0 warnings), `npm run build` | static | ✅ |
| `npm run test:headless`: 29 frontend tests (13 `App.test.ts` + 16 `Features.test.ts`) and 50 Rust tests, exit 0. The other 8 of the 58 Rust tests classify `ksni`/`zbus` errors and are Linux-only, so not compiled for Windows | headless | ✅ |
| `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` | static | ✅ |
| `cargo test --locked` (MSVC): 50 tests in `sotyware_lib` (all 27 feature tests among them, including unknown IDs, invalid transitions and the lifecycle table), 0 in the binary, 0 doc-tests | headless | ✅ |
| Windows graph (`cargo tree --locked --target x86_64-pc-windows-msvc -e normal,build`): 223 crates, none matching `ksni`, `zbus`, `gtk`, `gdk`, `glib`, `appindicator`, `webkit` or `dbus`; `sotyware`'s direct dependencies are `serde`, `tauri` 2.12.5 and `tauri-plugin-single-instance` 2.5.5; `tray-icon` 0.25.1 has no `ksni` feature | static | ✅ |
| `npm run tauri build -- --no-bundle`: `sotyware.exe`, 4,359,168 bytes; no capability or permission error at build time | static | ✅ |
| `windows.ps1` at `3a8a715` (90 checks: 80 existing + 10 feature checks), release binary | real desktop | ✅ **90 passed, 0 failed**, 3 consecutive runs (`npm run test:desktop:windows`), plus 1 run of the same checks before committing |
| Not vacuous: build outside the checkout (`git archive`) without `allow-set-feature-enabled` | real desktop | ✅ detected: 85 passed, 5 failed, exactly the switch checks; the page showed the error |
| `npm run test:desktop:windows:isolated` (unchanged, no feature checks) | private desktop | ✅ **56 passed, 0 failed**, 3 consecutive runs, primary's stderr empty |
| NSIS/MSI installers, manual checklist step 9, Windows 10 | — | — not run |

What the real-desktop run shows on Windows: the static capability lets the
WebView2 page call `list_features` (the example is listed with name, version
0.1.0, its description, checked and Running) and `set_feature_enabled` (UIA
toggle and real mouse click each disable and re-enable it; the label follows
Rust's answer); the process keeps answering; after tray Quit and a relaunch the
example runs again (not persisted). The 80 existing tray, Hide, close/restore,
single-instance and Quit checks passed unchanged. Invalid feature IDs and
lifecycle transitions are covered only by the Rust unit tests above: the page
cannot send them.

### Static checks and builds (WSL2, Ubuntu 24.04, Rust 1.99.1, Node 24.22.0)

| Check | Result | When |
| --- | --- | --- |
| `npm run check` (0 errors, 0 warnings) | ✅ | Re-run 2026-11-10 |
| `npm run build` | ✅ | Re-run 2026-11-10 |
| `cargo fmt --check` | ✅ | Re-run 2026-11-10 |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Re-run 2026-11-10 |
| `cargo test` (1 test) | ✅ | Re-run 2026-11-10, before the headless suite |
| `npm run tauri build -- --no-bundle` | ✅ | Re-run 2026-11-10 |
| `npm run tauri build`: `.deb`, `.rpm`, AppImage built (with WSL `PATH` filter) | ✅ built, not installed or run | Re-run 2026-11-10 |
| `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ✅ (the first is no longer needed since `12a6185`) | Re-run 2026-11-10, again at `12a6185` |

### Headless tests (WSL2, Ubuntu 24.04, Rust 1.99.1, Node 24.22.0)

Category A. Run under `env -i` with only `HOME`, `PATH` and a private `TMPDIR`:
no `DISPLAY`, `WAYLAND_DISPLAY` or D-Bus session. Nothing here starts sotyware or
touches the OS window system.

| Check | Result | When |
| --- | --- | --- |
| `npm run test:headless`: exit 0 | ✅ | 2026-11-10 |
| `cargo test`: 17 tests (close → hide, implicit exit blocked, tray failure → exit 1, reveal order and error tolerance, second launch, tray menu and click dispatch, config labels) | ✅ | 2026-11-10 |
| `npm run test:frontend`: 8 tests (Hide button sends `plugin:window\|hide` for `main`, errors shown and cleared, `main.ts` mount) | ✅ | 2026-11-10 |
| Not vacuous: 5 Rust and 4 `App.svelte` hand-made mutations each fail ≥ 1 test; `test:headless` exits non-zero for both halves | ✅ | 2026-11-10 |

The same suite on native Windows is in
[Headless tests, native Windows 11](#headless-tests-native-windows-11).

### Native Windows 11: environment

Windows 11 Home, version 25H2, OS build 26200.9711 (the registry `ProductName`
still says "Windows 10", a known Windows quirk; build 26200 is Windows 11). x64.
Checkout `C:\dev\sotyware`, a fresh `git clone` of `origin` at `f8174da` on the
NTFS drive (not a WSL path), `core.autocrlf=true`. Commands were started from WSL
through Windows interop, but compiled and ran as native Windows processes in the
interactive desktop session.

**Regression run at `1f5853b` (2026-11-10).** The same checkout was fast-forwarded
from `f8174da` to `1f5853b` (the headless-test commits, not pushed at the time) with a
`git bundle` made in the WSL repository, so the commit hashes are identical
([how](../development/setup.md#testing-unpushed-commits-on-windows)). Every command
ran in Windows PowerShell 5.1 with the machine and user `PATH` from the registry
(the `PATH` inherited through WSL interop does not contain `~\.cargo\bin`), so
`node.exe`, `cargo.exe` and `npm` were the Windows binaries listed below. No
`DISPLAY`, `WAYLAND_DISPLAY` or D-Bus variables were set. Tools were the same
versions as in the table; additionally rustup 1.29.3 and Git 2.47.1.windows.1.

| Tool | Version |
| --- | --- |
| Rust / Cargo | 1.99.1 / 1.99.1, `stable-x86_64-pc-windows-msvc` (rustup) |
| MSVC | Visual Studio Community 2022, C++ build tools |
| Node.js / npm | 24.14.1 / 11.12.1 |
| Tauri CLI | 2.12.4 (from `npm ci`) |
| WebView2 Runtime | 157.0.4341.42 |
| Windows PowerShell (test) | 5.1 |

### Dashboard-only mode regression run, native Windows 11

Run 2026-11-10 at `e8834fd`, the first native Windows run of the shared Rust
changes from `8ca3bd7` (operating mode, the `operating_mode` command, and hide
granted through the runtime `tray-mode` capability instead of the static
capability file). `c2cbeb4` and `e8834fd` change only Linux tests and docs. The
Windows checkout had already been fast-forwarded from `5dbafd8` to `e8834fd`
with a `git bundle` (identical hashes, clean tree); the test fix `71442d3` was
moved the same way. Same machine and procedure as above: Windows 11 Home 25H2,
OS build 26200.9711, x64; Windows PowerShell 5.1 started from WSL through
interop with the registry `PATH`, so every tool was the native Windows binary:
Rust/Cargo 1.99.1 (`stable-x86_64-pc-windows-msvc`), Visual Studio Community 2022
17.12 C++ tools, Node.js 24.14.1, npm 11.12.1, Tauri CLI 2.12.4, Git
2.47.1.windows.1. WebView2 Runtime is now 157.0.4341.46 (was .42).

| Check | Result |
| --- | --- |
| `npm ci`, `npm run check` (0 errors, 0 warnings), `npm run build` | ✅ |
| `npm run test:headless`: exit 0; 13 frontend tests and 23 Rust tests. The other 8 of the 31 Rust tests classify `ksni`/`zbus` errors and are `cfg(target_os = "linux")`, so they are not compiled for Windows | ✅ |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | ✅ |
| `cargo test` (MSVC): 23 tests in `sotyware_lib`, 0 in the binary, 0 doc-tests | ✅ |
| Windows graph (`cargo tree --target x86_64-pc-windows-msvc -e features`): no `ksni`, `zbus`, `gtk` or `libappindicator` crate; `sotyware` depends only on `tauri` 2.12.5 and `tauri-plugin-single-instance` 2.5.5. `tray-icon` has no `ksni` feature there (the Linux-only declaration does not apply) | ✅ |
| `npm run tauri build`: `sotyware.exe` (4,341,248 bytes), NSIS `sotyware_0.1.0_x64-setup.exe` (1.41 MiB), MSI `sotyware_0.1.0_x64_en-US.msi` (2.04 MiB); no capability or permission error at build time | ✅ built |
| `tests/desktop/windows.ps1` as committed at `e8834fd`, release binary | ❌ 4 of 4 runs failed: 78 passed / 2 failed three times, 71 / 9 once. Test defect, not a sotyware regression (below) |
| Same unchanged script on a release build of `1f5853b` (`git archive` outside the checkout), which had passed 80/80 earlier that day | ❌ 71 / 9, then 78 / 2: the same failure, so not caused by the new commits |
| `tests/desktop/windows.ps1` at `71442d3`, release binary | ✅ **80 passed, 0 failed**, 3 consecutive runs (plus 4 runs of the same change before committing it and 1 on the `1f5853b` build, all 80 / 0) |
| Supplementary scratch script (not in the repository), release binary with stderr captured: 5 × "Hide dashboard" (hidden, then a second launch restores it with exit 0, no error or permission text in the page), 3 × ✕ (hidden, process alive; tray "Open Dashboard" restores the same window; one window, one process), tray "Quit" exits 0, no process left, stderr without any permission or ACL message | ✅ 29 / 29 |
| NSIS `/S` install: exit 0; `%LOCALAPPDATA%\sotyware\` (`sotyware.exe`, `uninstall.exe`); HKCU uninstall entry "sotyware" 0.1.0; Start menu and desktop shortcuts pointing at the installed exe | ✅ |
| Installed copy: `tests/desktop/windows.ps1` (`71442d3`) | ✅ 80 / 0, 2 runs |
| Installed copy: supplementary script | ✅ 29 / 29 |
| Installed copy: the Start menu shortcut starts it; after Hide, the desktop shortcut shows the dashboard with the same pid and one window; one tray icon; tray "Quit" leaves no process | ✅ 8 / 8 |
| NSIS uninstaller `/S`: exit 0; install directory, uninstall entry and both shortcuts removed; no process left | ✅ |
| `HKCU\Software\sotyware\sotyware` (default value = install directory) after uninstall | ⚠️ remains. It was already there before this install, left by the earlier install/uninstall, and this uninstall did not remove it ([limitations](limitations.md#windows-installers)) |
| WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` | ⚠️ still present, as after the earlier uninstall |
| MSI | — not run (per-machine, needs an elevated install) |

**Runtime hide permission works on Windows.** The `core:window:allow-hide`
permission granted in `setup` reaches the dashboard: every Hide check in the
suite passed (UIA invoke and a real mouse click), as did the 5 supplementary
hides on both the build output and the installed copy, with no error shown in the
page. The only stderr line, at every quit, is WebView2's
`ERROR:ui\gfx\win\window_impl.cc:172] Failed to unregister class Chrome_WidgetWin_0. Error = 1412`,
which does not affect behaviour.

**Why the committed test failed.** The failing check was always the tray menu
right after the title-bar ✕ ("tray menu Open Dashboard again: invoked: context
menu did not open"), and in two runs also the tray "Quit". An instrumented copy
showed the cause: hiding the dashboard handed the foreground to another app's
window (`Chrome_WidgetWin_1`, a browser open behind it). The "Show Hidden Icons"
flyout lost focus and started closing, but UI Automation still reported it and
the sotyware icon with a valid rectangle. The test therefore skipped reopening
it and right-clicked the vanishing flyout. The earlier 80/80 runs had no such
window behind the dashboard (the machine had since rebooted). `71442d3` counts
the flyout as open only while it is the foreground window and waits out a
closing one. No check was removed and no timeout raised.

Not run on Windows: dashboard-only mode (it cannot occur there), Alt+F4, the
interactive NSIS UI, the MSI, a person's visual check, and Windows 10.

### Windows test categories, native Windows 11 (`0131e09`)

Run 2026-11-10 on the machine and toolchain described
[above](#dashboard-only-mode-regression-run-native-windows-11) (Windows 11 Home
25H2, build 26200.9711, WebView2 157.0.4341.46). The Windows checkout
`C:\dev\sotyware` was fast-forwarded from `798e312` to `195fa8a` and then
`0131e09` with `git bundle` (identical hashes, clean tree). Release binary
rebuilt at `195fa8a` (`npm run tauri build -- --no-bundle`); `0131e09` changes
only the test script. Commands started from WSL through interop in PowerShell 5.1
with the registry `PATH`. Categories: [testing.md](../development/testing.md).

| Check | Category | Result |
| --- | --- | --- |
| `npm ci`, `npm run check` (0 errors, 0 warnings) | static | ✅ |
| `npm run test:headless`: 13 frontend + 23 Rust tests, exit 0 | headless | ✅ (`195fa8a`) |
| `npm run test:desktop:windows:isolated` (private desktop, no input, nothing on screen) | private desktop | ✅ **56 passed, 0 failed**, 3 consecutive runs at `195fa8a` and 3 more at `0131e09`; primary's stderr empty |
| Same suite on a mutant build (outside the checkout, from `git archive`) whose close quits in tray mode | private desktop | ✅ detected: 43 passed, 9 failed, exit 1 at `0131e09`. At `195fa8a` it also failed 9 checks but exited 0, a launcher bug fixed in `0131e09` |
| Same suite with `-Timeout 5` | private desktop | ✅ exit 124, no sotyware process left |
| `npm run test:desktop:windows` (`windows.ps1`, real desktop, real mouse) | real desktop | ✅ **80 passed, 0 failed**, 3 consecutive runs at `195fa8a` |
| `windows.ps1` started on a private desktop (no input desktop, no taskbar) | guard | ✅ `ABORT`, exit 3, no sotyware process started (pre-commit copy of `44a131d`'s script) |
| Isolated-desktop probe (scratch script, not in the repository): on a private desktop sotyware starts in tray mode, the WebView2 page is readable through UIA, Hide, second launch and `WM_CLOSE` behave as on the real desktop; no `Shell_TrayWnd`, so no tray icon | investigation | ✅ observed |
| Session 0 (scheduled task "run whether the user is logged on or not") | investigation | — not run: registering the task was not permitted in the investigating session |
| A GitHub-hosted Windows runner | CI | ❌ run once, failed: WebView2 cannot render on a private desktop in an elevated session ([CI](#ci-github-actions-2026-11-10), [cause](#windows-11-isolated-desktop-test-elevated-2026-11-11)) |

Conclusion: no Windows test of the real binary runs headlessly. The
private-desktop suite runs unattended inside a logged-in session without using
the screen or the input devices; the tray, real clicks and Quit need the
interactive desktop. Details:
[testing.md](../development/testing.md#headless-windows-desktop-testing).

### Windows 11: isolated-desktop test elevated (2026-11-11)

Investigation of the failed hosted run of `desktop-windows-isolated.yml`
([CI](#ci-github-actions-2026-11-10)). Same machine as
[above](#windows-test-categories-native-windows-11-0131e09) (Windows 11 Home
25H2, build 26200.9711, WebView2 157.0.4341.46, UAC on); checkout
fast-forwarded to `55504d4` with `git bundle`, release binary rebuilt there.
Elevated runs were started through a UAC prompt the user accepted. The probes
were scratch scripts, not in the repository.

| Check | Result |
| --- | --- |
| Unchanged suite at `55504d4`, non-elevated | ✅ 56 passed, 0 failed, primary's stderr empty |
| Unchanged suite at `55504d4`, **elevated** | ❌ 13 passed, 39 failed, exit 1: the hosted failure reproduced. Primary's stderr: `failed to create webview: WebView2 error: ... HRESULT(0x80070578) "Invalid window handle."` at `tauri-2.12.5/src/app.rs:1444`; second launches exit -1073740791 (`0xC0000409`, `panic = "abort"`). The window exists briefly before the webview fails, so the startup visibility check passes |
| Probe: where WebView2's processes run, non-elevated, private desktop | browser `msedgewebview2.exe` is a child of `sotyware.exe` (medium); its windows and the GPU process's (low) are on the private desktop, under the `Tauri Window` |
| Same probe, elevated | browser `msedgewebview2.exe` is a child of **`explorer.exe`** (medium, de-elevated); all its windows are on **`WinSta0\Default`**, none on the private desktop; sotyware panics as above |
| Probe: sotyware elevated on `WinSta0\Default` | ✅ WebView2 renders (`Chrome_RenderWidgetHostHWND` under the `Tauri Window`), no panic: elevation alone is not the cause |
| Private desktop's security descriptor, elevated vs not | same DACL (logon SID, `RC`, `BA`, `SY`, AppContainer ACEs) except the owner; an explicit low label (`S:(ML;;NW;;;LW)`) on an elevated private desktop still fails: not a desktop-access problem |
| Suite at `468a2c3` (elevation check), elevated | ✅ `NOT RUN`, exit 4, no desktop created, no sotyware process left |
| Suite at `468a2c3`, non-elevated | ✅ **56 passed, 0 failed**, 3 consecutive runs, primary's stderr empty |

Conclusion: the private-desktop test cannot run elevated, because WebView2
launches its de-elevated browser process on Explorer's desktop. sotyware itself
is unaffected on a real desktop, elevated or not, so the app is unchanged. The
hosted job is elevated: the second hosted run printed `elevated: True` and
reported NOT RUN, exit 4 ([CI](#ci-github-actions-2026-11-10)).

### CI: GitHub Actions (2026-11-10)

Added in `5fe62b9`; details in [testing.md](../development/testing.md#ci). The
repository is private on GitHub Free ($12 of hosted compute a month).

| Workflow | Trigger | Result |
| --- | --- | --- |
| `ci.yml` headless (Linux, Windows) | automatic: pull request / push to `main` | ✅ **two automatic runs, both passed**: job `headless (ubuntu-latest)` passed and job `headless (windows-latest)` passed in each (confirmed on GitHub, recorded 2026-11-11; run ids, commits, timestamps and job durations were not recorded). Committed in `5fe62b9`; on GitHub since the push of 2026-11-10 22:53 (+0100) that moved `main` to `3f3b9b9`. That push changed `ci.yml` itself and the push to `14a5de1` changed `package.json`, so both match the `push` trigger; later pushes (`55504d4`, `44be84e`, and the docs-only pushes after them) changed only ignored paths. Its command sequence also passed locally on WSL2 Ubuntu 24.04 at `5fe62b9` (`npm ci`, check 0/0, build, `cargo fetch --locked`, fmt, clippy, 13 frontend + 31 Rust tests) |
| `desktop-virtual.yml` | manual | ✅ **passed on GitHub: 92 passed, 0 failed, 0 skipped** (confirmed, recorded 2026-11-11; run id, commit, runner image and duration not recorded). Virtual display (Xvfb + openbox, stub tray host): **not** validation on a real GNOME, KDE or XFCE desktop. Supersedes the first GitHub run, at `14a5de1` (runner image ubuntu-26.04 20261101.186, 2026-11-10): build 5 min 27 s, then 89 passed, 2 failed, 1 skipped, exit 1. Scenario 12 found no Hide button; cause was the driver matching the AT-SPI role name, renamed in at-spi2-core 2.60 ([below](#virtual-display-test-on-ubuntu-2604-container-2026-11-10)), fixed in `ead47e7` |
| `desktop-windows-isolated.yml` | manual, experimental | ❌ run once on GitHub ([run 42919688755](https://github.com/casualsoty/sotyware/actions/runs/42919688755), Windows Server 2025; commit and duration not recorded here): private desktop `sotyware-test-8872` created, input desktop `Default`, no taskbar; the window appeared, then the title and Hide-button checks failed, the primary panicked (`failed to create webview`, `0x80070578` Invalid window handle) and second launches exited -1073740791. Reproduced on Windows 11 by running the suite elevated; cause and evidence [above](#windows-11-isolated-desktop-test-elevated-2026-11-11). Since `468a2c3` an elevated job reports NOT RUN (exit 4). **Second run** (2026-11-10 23:41 UTC, Windows Server 2025 Datacenter 24H2 build 26100.33674; run id and commit not in the pasted log, `origin/main` was `44be84e`): `elevated: True`, `NOT RUN`, exit 4, job failed as intended; no check ran |
| Workflow syntax | — | YAML parses, every action pinned to a full SHA; not checked with `actionlint`. GitHub accepted all three workflows (each has run) |

#### Runs at `f19dd8b` (2026-11-13)

The push of `main` from `b0c4300` to `f19dd8b` (28 commits) triggered
`ci.yml`; the two manual workflows were started by hand on `main`
afterwards. Every job's checkout fetched
`f19dd8b25c5660a788589105283b709a7d3a94f9`, so each ran exactly the
committed tree. Read from the full job logs; runner version 2.338.2.
Durations are from the first to the last log line of each job. Both
`ci.yml` jobs started with no Cargo cache (`No cache found`) and saved one
at the end (push to `main`). No real provider was contacted: every
Backblaze and Hetzner test was ignored as opt-in, and no provider endpoint
appears in any log.

| Workflow | Run | Runner | Result |
| --- | --- | --- | --- |
| `ci.yml`, `headless (ubuntu-latest)` | [43360177079](https://github.com/casualsoty/sotyware/actions/runs/43360177079), 03:37 UTC | image ubuntu-26.04 (20261111.196.1) | ✅ **passed**, 7 min 37 s, cold Cargo cache: check, build, `cargo fetch --locked`, rfd guard ok, guard self-test 10/10, fmt, clippy `-D warnings` (rfd's 9 upstream dead-code warnings from `oneshot.rs` printed, as expected, ADR-023), `test:headless`: guard ok, Vitest 135/135, Rust **308 passed, 16 ignored** |
| `ci.yml`, `headless (windows-latest)` | same run | image windows-2025-vs2026 (20261103.298.1) | ✅ **passed**, 17 min 23 s, cold Cargo cache: same steps; guard self-test 10/10; Rust **298 passed, 16 ignored** (the Linux-only tests absent), including `concurrent_connections_never_corrupt_the_database` on the hosted runner |
| `desktop-virtual.yml` | run id not in the downloaded log, 03:56 UTC | image ubuntu-26.04 (20261101.186.1) | ✅ **passed**, 9 min 7 s: rfd guard ok, release build 7 min 49 s, **119 passed, 0 failed, 0 skipped** (the expected count). Virtual display, not a real desktop |
| `desktop-windows-isolated.yml` | run id not in the downloaded log, 04:07 UTC | Windows Server 2025 Datacenter 24H2 build 26100.33674, image windows-2025-vs2026 (20261103.298.1) | ❌ **failed as intended, NOT RUN**: rfd guard ok, release build 9 min 20 s, then `elevated: True`, exit 4; no check ran (11 min 10 s). The step's process exit code shows as 1 while the summary and error line say exit 4: an `exit` inside the script GitHub runs through `pwsh -command ". '{0}'"`; it predates the guard |

Record further GitHub results here, with the real job durations when they are
read, and replace the cost estimate in
[testing.md](../development/testing.md#github-free-budget).

### Virtual-display test on Ubuntu 26.04 (container, 2026-11-10)

Reproduces the `desktop-virtual.yml` failure off GitHub. An `ubuntu:26.04`
Docker image (26.04.1) on the WSL2 machine, with the runtime packages the
workflow installs (at-spi2-core 2.60.4, GTK 3.24.52, GLib 2.88.0, WebKitGTK
2.52.6), runs `bash tests/desktop/virtual.sh` as a non-root user against the
release binary built at `14a5de1` on WSL2 Ubuntu 24.04.1. Xvfb + openbox, stub
tray host, as on the runner. Not a real desktop.

| Check | Result |
| --- | --- |
| Driver before the fix (`14a5de1`) | ❌ 2 of 2 runs 89 passed, 2 failed, 1 skipped, exit 1: the same two scenario 12 failures and scenario 13 skip as on GitHub |
| AT-SPI tree in tray mode, read directly | ✅ page rendered, `button 'Hide dashboard'` present; at-spi2-core 2.60 names the role "button", 2.52 "push button" (enum value 43 in both) |
| Driver after the fix (`ead47e7`, roles matched by enum) | ✅ 3 consecutive runs 92 passed, 0 failed, 0 skipped, exit 0 |
| Same fixed driver on WSL2 Ubuntu 24.04.1 (at-spi2-core 2.52.0) | ✅ 3 consecutive runs 92/0/0 |

### Real Linux desktop availability (2026-11-10)

Surveyed on the development machine before planning any Linux desktop run.

| Looked for | Found |
| --- | --- |
| WSL distributions | only `Ubuntu` (24.04.1), the development environment; WSLg serves its display (`DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`), no desktop session (`XDG_CURRENT_DESKTOP` empty) |
| Desktop environments in that distribution | none installed (no `gnome-shell`, `plasmashell`, `xfce4-panel`, `gdm3`, `lightdm`, `mutter`) |
| Virtual machines on the Windows host | none: Windows 11 **Home** (no Hyper-V Manager, no `Get-VM`), no VirtualBox or VMware installed |
| Other Linux machines | none configured (no SSH hosts) |
| Docker Desktop | present, but runs Linux containers without a desktop session or tray host; not a real desktop |

**No real Linux desktop was tested.** GNOME, KDE Plasma, XFCE and real Wayland
stay unvalidated; the only Linux results remain the virtual-display and
simulated-desktop runs below, which use a stub tray host. Installing a full
desktop environment inside WSL was not done: it would still run under WSLg's
compositor with no real shell tray host, so it would not be a real desktop
either, and it would add several hundred packages to the development
environment. Options that would give a real desktop: a VirtualBox (or VMware
Workstation) VM with Ubuntu Desktop (GNOME), a live USB session on real
hardware, or a separate Linux machine. Each needs the user's decision.

### Headless tests, native Windows 11

Category A at `1f5853b`. Run with `TEMP`/`TMP` pointed at a new empty directory,
no sotyware process running and none installed.

| Check | Result | When |
| --- | --- | --- |
| `npm run test:headless`: exit 0 (34 s, Rust test build partly cached) | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `npm run test:frontend` (Vitest 4.1.12, happy-dom): 8 tests | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `cargo test` (MSVC): 17 tests in `sotyware_lib`, 0 in the binary, 0 doc-tests | ✅ | Native Windows 2026-11-10, `1f5853b` |
| Not vacuous: `App.svelte` without the `hide()` call → 5 of 8 frontend tests fail, exit 1; `close_action` returning `Allow` for `main` → `closing_the_dashboard_hides_it` fails, exit 101; `exit_allowed` always `true` → `implicit_exit_is_blocked` fails, exit 101. Each mutation was a temporary, uncommitted edit, restored right after | ✅ | Native Windows 2026-11-10, `1f5853b` |
| Side effects: nothing left in the private `TEMP` except Node's `node-compile-cache` (enabled by Vite's CLI, also on Linux); no sotyware process started; the WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` unchanged (file list, sizes and times compared); `git status` clean | ✅ | Native Windows 2026-11-10, `1f5853b` |

No Unix-only command is on this path: the npm scripts use only `&&`, which
`cmd.exe` (npm's default script shell on Windows) supports. No test writes
temporary files or reads runtime environment variables; the only paths are the
two Rust config tests, which read `tauri.conf.json` and
`capabilities/default.json` as `CARGO_MANIFEST_DIR` + `/` + name (Windows accepts
the mixed separator) from the CRLF checkout.

### Static checks and builds, native Windows 11

| Check | Result | When |
| --- | --- | --- |
| `npm ci` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `npm run check` (0 errors, 0 warnings) | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `npm run build` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo fmt --check` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo clippy -- -D warnings` | ✅ | Native Windows 2026-11-10, `1f5853b` |
| `cargo clippy --all-targets -- -D warnings` | ✅ | Native Windows 2026-11-10; re-run at `1f5853b` |
| `cargo test` | ✅ 1 test at `f8174da`; 17 tests at `1f5853b` | Native Windows 2026-11-10 |
| `npm run tauri build`: `sotyware.exe` (3.2 MB), `sotyware_0.1.0_x64-setup.exe` (NSIS, 1.13 MiB), `sotyware_0.1.0_x64_en-US.msi` (1.64 MiB) | ✅ built; same sizes at `1f5853b` (154 s) | Native Windows 2026-11-10; re-run at `1f5853b` |

### Runtime behaviour, native Windows 11 (real desktop)

`tests/desktop/windows.ps1` on the release binary: **80 passed, 0 failed**, run
three times in a row with the same result (2026-11-10). Earlier revisions of the
same script (78 checks) also passed on four further consecutive runs. No sotyware
defect was found.

Re-run at `1f5853b` after the headless-test commits (which moved the lifecycle
decisions into plain functions): **80 passed, 0 failed** on the release binary
and **80 passed, 0 failed** on the NSIS-installed copy (2026-11-10), one run each.
No regression against the earlier 80/80.

Re-run at `e8834fd` (dashboard-only mode, hide granted at runtime) with the test
fix `71442d3`: **80 passed, 0 failed**, 3 runs on the release binary and 2 on the
NSIS-installed copy ([details](#dashboard-only-mode-regression-run-native-windows-11)).
The table below holds for all three commits.

| Behaviour | Result | Source |
| --- | --- | --- |
| First launch: dashboard visible, title "sotyware", one process, one window | ✅ | Windows test |
| Exactly one sotyware notification-area icon (it lands in the hidden-icons flyout) | ✅ | Windows test |
| Tray menu (right click) contains exactly "Open Dashboard", "Quit" | ✅ | Windows test (read via `MN_GETHMENU`) |
| Tray left click opens the dashboard and shows no menu | ✅ | Windows test (real mouse click) |
| "Hide dashboard" button hides the window; window not destroyed; process stays | ✅ | Windows test (UIA invoke and real mouse click) |
| Tray "Open Dashboard" restores the same single window | ✅ | Windows test (real mouse click on the item) |
| Title-bar close button (✕) hides instead of exiting; process stays | ✅ | Windows test (real mouse click) |
| Programmatic close request (`WindowPattern.Close`) hides; process stays | ✅ | Windows test |
| Alt+F4 | — not sent (see testing.md) | |
| Second launch while hidden: exits 0; same single process and window; dashboard shown | ✅ | Windows test |
| Second launch while visible: exits 0; no new process or window | ✅ | Windows test |
| Second launch while minimized: exits 0; dashboard restored | ✅ | Windows test |
| Dashboard is the foreground window after a second launch (hidden, visible, minimized) and after a tray left click | ✅ observed every run | Windows test (INFO lines) |
| 5 sequential + 4 concurrent launches: all exit 0; one process, window and tray icon | ✅ | Windows test |
| Hide, then tray "Quit": exits 0; no process and no tray icon left | ✅ | Windows test |
| Relaunch after Quit starts a fresh primary; Quit exits 0 again | ✅ | Windows test |
| A test with the button made a no-op fails the 3 hide checks | ✅ (test is not vacuous) | Mutation run |
| A person looked at the tray icon and menu and clicked them | — not yet done | |

Focus note: the second launches were started by a background script, and the
dashboard still became the foreground window every time. Windows' foreground-lock
rules can behave differently when another app is actively in use; that case was
not tested.

### Installers, native Windows 11

| Step | Result | When |
| --- | --- | --- |
| NSIS `sotyware_0.1.0_x64-setup.exe /S`: exit 0; per-user install to `%LOCALAPPDATA%\sotyware` (`sotyware.exe`, `uninstall.exe`); HKCU uninstall entry "sotyware" 0.1.0; Start menu and desktop shortcuts | ✅ | Native Windows 2026-11-10 |
| Installed copy: `tests/desktop/windows.ps1 -Exe %LOCALAPPDATA%\sotyware\sotyware.exe` | ✅ 80 passed, 0 failed with the committed script; an earlier install also passed the 78-check revision | Native Windows 2026-11-10 |
| At `1f5853b`: NSIS `/S` install (exit 0), installed copy 80 passed / 0 failed, uninstaller `/S` (exit 0; install dir and HKCU uninstall entry removed). The installed `sotyware.exe` has the same size as the build output but a different SHA-256; not investigated | ✅ | Native Windows 2026-11-10 |
| NSIS uninstaller `/S` after the app had quit: exit 0; files, uninstall entry and shortcuts removed | ✅ | Native Windows 2026-11-10 |
| Start menu shortcut launches the installed exe; a second shortcut launch keeps one process | ✅ | Native Windows 2026-11-10 |
| NSIS uninstaller `/S` while sotyware was running: exit 0; process ended; install dir, uninstall entry and both shortcuts removed | ✅ | Native Windows 2026-11-10 |
| WebView2 data folder `%LOCALAPPDATA%\com.sotyware.desktop` after uninstall | ⚠️ still present; it was also created by earlier runs of the build output, so this does not show whether the uninstaller would remove it | Native Windows 2026-11-10 |
| NSIS installer with its interactive UI | — not run (silent mode only) | |
| MSI `msiexec /i … /qn` | ❌ not installed: exit 1603, "Installation UI level is silent, no credential elevation is possible". The MSI installs per machine and needs administrator rights (UAC). Not a sotyware defect; MSI install, run and uninstall remain unvalidated | Native Windows 2026-11-10 |

### Dashboard-only mode (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `c2cbeb4` (implementation `8ca3bd7`,
[ADR-012](../architecture/decisions.md#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode),
[ADR-013](../architecture/decisions.md#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode)),
same machine and apt packages as the runs below, plus `gir1.2-atspi-2.0` and
`at-spi2-core` (already installed). Release binary built from `8ca3bd7`.

| Check | Result |
| --- | --- |
| `npm run check` 0/0, `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run tauri build -- --no-bundle` | ✅ |
| `cargo test`: 31 tests (17 before, adapted: close action per mode, `select_mode`; new: mode selection from the real `ksni`/`zbus` error values, what stays fatal, hide granted only in tray mode, dashboard starts visible, capability) | ✅ |
| `npm run test:frontend`: 13 tests (8 before, adapted to the mode request; new: dashboard-only note, no Hide action, failed or unknown mode) | ✅ |
| `npm run test:headless` (31 Rust + 13 frontend) | ✅ |
| `npm run test:desktop:virtual`, 92-check driver: **92 passed, 0 failed, 0 skipped**, 3 consecutive runs, no sotyware process left | ✅ |
| Same suite with the 68-check driver from before, on the new binary: 63 passed, then scenario 10 hung (a missing watcher no longer exits 1) | ❌ expected, driver adapted in `c2cbeb4` |
| Error shapes from the real binary, before the change: no bus ⇒ `D-Bus connection error: Failed to connect to address …: No such file or directory`; no watcher ⇒ `…ServiceUnknown…` | ✅ matches the classification |
| `Cargo.lock`: only `ksni` and `zbus` added to `sotyware`'s dependency list, no new crate. `cargo tree --target x86_64-pc-windows-msvc`: `sotyware` still depends only on `tauri` and `tauri-plugin-single-instance`; `ksni` and `zbus` are not in the Windows graph | ✅ |

What the 24 new virtual-display checks cover: [testing.md](../development/testing.md#simulated-desktop-test)
(scenarios 12–16). The tray-mode checks of scenarios 1–9 and 11 are unchanged.
Scenario 10 now uses a watcher that rejects the item instead of no watcher.

Native Windows was run afterwards, at `e8834fd`:
[results](#dashboard-only-mode-regression-run-native-windows-11). Not run:
`run.sh` on WSLg, a real GNOME or any real Linux desktop, an
XEmbed-only panel, Wayland, and a person looking at the dashboard-only note.

### ksni tray backend (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `12a6185` (Linux tray switched to `tray-icon`'s `ksni` backend,
[ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)),
same machine and apt packages as the run below. Release binary
`src-tauri/target/release/sotyware`, sha256 `4dd03869…54176` (4,976,808 bytes;
the libayatana build `10c5e588…29512` was 4,429,896). The backend probe and the
launch loop were scratch scripts outside the repository, with the same isolation
as `virtual.sh`: private Xvfb display, openbox, `dbus-run-session`, private
`XDG_RUNTIME_DIR`, `sni_watcher.py`. The loop ran a byte-identical copy of the
release binary (`cmp`).

Dependency graph (`cargo tree`): one `tray-icon` 0.25.1, used by both `tauri`
and `sotyware`, with features `ksni`, `libappindicator`, `muda-snapshot`,
`muda-gtk3`, `muda-serde`, `muda-common-controls-v6`, `serde`. New in
`Cargo.lock`: `ksni` 0.3.7, `arc-swap` 1.9.5, `crossbeam-queue` 0.3.14,
`pastey` 0.2.3, `task-local` 0.1.1. `zbus` 5.19.0 was already present. No new
duplicate crate versions. For `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin`, the resolved packages and features are identical to
before the change.

| Check | Result |
| --- | --- |
| Backend selected in source: `tray-icon` 0.25.1 `platform_impl/mod.rs` compiles `gtk/` only with `libappindicator` and `not(ksni)`, and `ksni/` whenever `ksni` is on | ✅ |
| Binary: `ldd`/`readelf` NEEDED list no appindicator library (neither did the libayatana build, which `dlopen`ed it) | ✅ |
| Binary strings: `libayatana-appindicator3.so.1`, `app_indicator_new` and `libappindicator-sys` absent (each present once in the libayatana build). `org.kde.StatusNotifierItem-`, `/StatusNotifierItem`, `/MenuBar` and `ksni-0.3.7` present | ✅ |
| Running process (`/proc/<pid>/maps`): no `libayatana-*`, `libdbusmenu-*` or `ido` library mapped (the libayatana build mapped 5). Extra thread `tray-icon-menu-watcher` | ✅ |
| Registration: name `org.kde.StatusNotifierItem-<pid>-1`, path `/StatusNotifierItem`, menu `/MenuBar`, `Id` `sotyware-tray`, `ItemIsMenu` false, one 32×32 `IconPixmap`, tooltip `sotyware` (the libayatana build used `/org/ayatana/NotificationItem/…` and an icon file) | ✅ |
| `npm run check` 0/0, `npm run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | ✅ |
| `npm run test:headless` with `DISPLAY`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR` unset: 17 Rust + 8 frontend passed | ✅ |
| `npm run test:desktop:virtual`, 66-check driver, before adapting it: 63 passed, 3 failed. All 3 were scenario 10: the unwritable `$XDG_RUNTIME_DIR` no longer breaks the tray, so the instance ran with its tray and was killed after 30 s | ❌ expected, driver adapted in `12a6185` |
| `npm run test:desktop:virtual`, 68-check driver at `12a6185`: **68 passed, 0 failed, 0 skipped**, 3 consecutive runs | ✅ |
| 420 consecutive launches, each: wait ≤ 10 s for one registration, read the menu, tray "Quit", wait ≤ 10 s for exit. No retries | ✅ **420/420** registered (0.148 s min, 0.178 s median, 0.203 s p95, 0.293 s max), 420 menus with exactly Open Dashboard / Quit, 420 exits with 0 through tray Quit, 0 `CRITICAL` / `G_IS_DBUS_CONNECTION`, 0 hangs, 0 processes left |
| Left click (`Activate` over D-Bus) on a hidden dashboard shows the same window. Middle click (`SecondaryActivate`) does not | ✅ |
| Stub watcher killed and restarted: the item registers again after 0.10 s, and its menu still works | ✅ |
| Private bus without a watcher: exit 1 after 0.36 s, `cannot create system tray icon: … failed to register to the StatusNotifierWatcher: …ServiceUnknown…`, nothing left (behaviour at the time; since `8ca3bd7` dashboard-only mode) | ✅ |
| No session bus: exit 1, `cannot create system tray icon: … D-Bus connection error …`, nothing left (behaviour at the time; since `8ca3bd7` dashboard-only mode) | ✅ |
| After all runs: no Xvfb, openbox, stub watcher, portal, session `dbus-daemon` or sotyware process of these runs left (an unrelated Xvfb on `:25059`, started hours earlier, was still running) | ✅ |
| `npm run tauri build -- --bundles deb`: `.deb` declares `libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` | ⚠️ the first one is no longer needed ([limitations](limitations.md#linux-tray)) |

Not run: native Windows (the only Windows-relevant change is unused lockfile
entries), a real Linux desktop or tray host, Wayland, a system without
libayatana-appindicator installed. Zero failures in 420 launches is evidence,
not proof. The libayatana build failed 1 in 420 under the same loop.

### Runtime behaviour, virtual display, standard Ubuntu packages (WSL2, Xvfb + openbox, stub tray host)

Category B, run 2026-11-10 at `53d9743` on Ubuntu 24.04.1 LTS under WSL2 (kernel
6.18.54.1-microsoft-standard-WSL2, x86_64), Node 24.22.0, npm 11.19.1, Rust and
Cargo 1.99.1, tauri-cli 2.12.4. **All tools installed with `apt` as documented**
(`sudo apt install -y xvfb openbox python3-dbus python3-gi dbus-x11`): xvfb
2:21.1.12-1ubuntu1.9, openbox 3.6.1-12build5, x11-xkb-utils 7.7+8build2 (pulled
in by xvfb, provides `xkbcomp`), dbus-x11 1.14.10-4ubuntu4.1, python3-dbus
1.3.2-5build3, python3-gi 3.48.2-1, libayatana-appindicator3-1 0.5.93-1build3.
No extracted `.deb`s, no shim, no `PATH` changes beyond the WSL notes in
[setup.md](../development/setup.md#wsl2-notes). Simulated: the display (Xvfb),
the desktop (openbox), the tray host (`sni_watcher.py`) and the user (X11 client
messages, D-Bus calls). Real: the release binary
(`src-tauri/target/release/sotyware`, sha256 `10c5e588…29512`; a rebuild from
unchanged sources was byte-identical).

| Check | Result |
| --- | --- |
| Missing `Xvfb`/`openbox` (before installing): names them, prints the install command, exits 2, starts nothing | ✅ |
| Two consecutive `npm run test:desktop:virtual` runs at `53d9743`: **66 passed, 0 failed, 0 skipped** each, exit 0, about 7 s each | ✅ |
| 30 further consecutive runs at the same driver: 66/0/0 each | ✅ |
| Before the driver fix (`53d9743`), 65-check driver: first run 65/0/0, second run 64/1/0. In about 90 runs (the driver and instrumented scratch copies of it), 5 failed or crashed because of races in the driver and 2 because of the tray defect below | ❌ driver fixed |
| Private display `:99` (abstract socket only); the pre-existing WSLg display `:0` and an unrelated Xvfb on `:25059` were not used and still running afterwards | ✅ |
| After every run: no Xvfb, openbox, `dbus-daemon --session`, stub tray host, xdg portal or sotyware process left; no new `/tmp` entry or X lock file | ✅ |
| Interrupted mid-run (SIGINT to the process group: exit 1; SIGTERM to the script: exit 143): same, nothing left | ✅ |
| A fake "sotyware" window shown during a second launch fails "no other window appeared" (control) | ✅ |
| A stub tray host that records no registration gives a FAIL, the primary's output and exit 1, not a traceback (control) | ✅ |

**Failures found and fixed (test driver, `53d9743`):** the window-tree walk raced
openbox unmanaging the just-closed dashboard, so the primary's own window (same id
and pid) was reported as "new" or the "8 close" check missed it; the tray "Quit"
click raised D-Bus `NoReply` when sotyware exited before replying; and a missing
tray registration crashed the driver. None was a sotyware defect.

**Found, not fixed (tray library):** a launch occasionally registers no
tray item at all and prints
`GLib-GIO-CRITICAL **: g_dbus_proxy_new: assertion 'G_IS_DBUS_CONNECTION (connection)' failed`;
3 of about 420 primary launches. Root cause investigated below. The suite
reports it as a FAIL. Avoided since `12a6185`: sotyware no longer uses the library
([ksni run](#ksni-tray-backend-wsl2-xvfb--openbox-stub-tray-host)).

### Missing tray item investigation (WSL2, Xvfb + openbox, stub tray host)

Run 2026-11-10 at `c5540c9`, the machine and packages above (GLib
2.80.0-6ubuntu3.10), release binary sha256 `10c5e588…29512` (same as at
`53d9743`). The launch loop and the diagnostic shim were scratch tools outside
the repository. The loop used the same isolation as `virtual.sh`: private Xvfb
display, openbox, `dbus-run-session`, private `XDG_RUNTIME_DIR`,
`sni_watcher.py`. Each launch waited up to 10 s for one registration, then ended
with tray "Quit" (SIGTERM if not registered). No retries.

| Check | Result |
| --- | --- |
| 420 consecutive launches, unmodified binary | ❌ 1 without a tray item (launch 58: alive, watcher owned its name, `G_IS_DBUS_CONNECTION` in the output, nothing else), 419 registered in ≤ 714 ms with no critical, all exited 0 |
| 420 consecutive launches with a logging `LD_PRELOAD` shim, no delay | ✅ 0 failures. In every launch libayatana's own `g_bus_get` result arrived before the watcher's name-appeared callback, by 0.03 ms (minimum) to about 1.4 ms (median) |
| Shim delays only libayatana's `g_bus_get` result by 300 ms | ❌ 2 of 2 without a tray item, same critical. `g_dbus_proxy_new` called from `libayatana-appindicator3.so.1` with `connection=(nil)` (the forced-order proof) |
| After every launch, both loops: no sotyware process left; display, bus and runtime directory removed | ✅ |

Result: the defect is a race in libayatana-appindicator (cause, evidence and
fix status in [limitations.md](limitations.md#historical-libayatana-appindicator-registration-race)).
No sotyware change at the time; the limitation stayed open. Not run outside WSL2.
Superseded by the switch to the `ksni` backend (`12a6185`).

The headless suite, static checks and release build on the same machine and
commit: `npm run test:headless` 8 + 17 passed with `DISPLAY`, `WAYLAND_DISPLAY`,
`DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` unset, exit 1 with one injected
failing test; `npm run check` 0/0, `npm run build`, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings` and `npm run tauri build -- --no-bundle`
passed.

### Runtime behaviour, virtual display, extracted packages (WSL2, Xvfb + openbox, stub tray host)

Superseded by the standard-package run above; kept as the earlier record.
Category B. `npm run test:desktop:virtual` on the release binary (built
2026-11-10 from the code with the lifecycle decisions extracted):
**65 passed, 0 failed, 0 skipped**, twice in a row (2026-11-10). Xvfb
2:21.1.12-1ubuntu1.9 and openbox 3.6.1 came from the Ubuntu 24.04 `.deb`s
extracted into a user directory (no root on this machine), and Xvfb was started
through a user-namespace shim so it finds `xkbcomp`; the WSL `/tmp/.X11-unix` is
read-only, which the script's abstract-socket-only setting handles.

| Behaviour | Result |
| --- | --- |
| Everything in the simulated desktop table below that passed there | ✅ |
| Second launch focuses an already visible dashboard (`_NET_ACTIVE_WINDOW` observed) | ✅ |
| Second launch restores a minimized dashboard (no longer iconic) | ✅ |
| Tray failure (unwritable `$XDG_RUNTIME_DIR`): exit 1, error message, nothing left | ✅ |
| Timeout path: exit 124, logs printed, no leftover sotyware, openbox or Xvfb | ✅ |

openbox on Xvfb is **not** GNOME, KDE, XFCE or Wayland, and the tray host is a
stub. Focus behaviour on real Linux window managers remains unvalidated.

### Runtime behaviour, simulated Linux desktop (WSL2/WSLg, X11, stub tray host)

`tests/desktop/run.sh` on the release binary. Re-run 2026-11-10 after the
lifecycle decisions were extracted: **57 passed, 0 failed, 2 skipped** (54/0/2
before the tray-failure scenario was added; same 2 skips). A build with the tray
failure made non-fatal fails the new exit-code check. Not re-run with the
66-check driver from `53d9743`.

| Behaviour | Result | Source |
| --- | --- | --- |
| First launch: dashboard visible, one tray item with Open Dashboard / Quit | ✅ | desktop test |
| Close request hides (does not destroy) the window; process stays | ✅ | desktop test |
| Tray "Open Dashboard" restores the same single window | ✅ | desktop test |
| Second launch (dashboard visible or hidden): exits 0, no new window, process or tray item | ✅ | desktop test |
| Second launch while hidden: dashboard shown and focused | ✅ | desktop test |
| 5 sequential + 4 concurrent launches: always one process, tray item, window | ✅ | desktop test |
| Tray "Quit" exits 0, no process left; relaunch starts a fresh primary | ✅ | desktop test |
| Second launch focuses an already visible dashboard | ⏭️ WSLg WM ignores `_NET_ACTIVE_WINDOW` | desktop test |
| Second launch restores a minimized dashboard | ⏭️ WSLg WM ignores `XIconifyWindow` | desktop test |
| "Hide dashboard" button hides the window; process stays | ✅ | Earlier run (no automated test on Linux) |
| Repeated "Open Dashboard" creates no second window | ✅ | Earlier run (also implied by desktop test window counts) |
| Native Wayland backend: runs, tray registers, Quit exits 0 (window state not inspected) | ✅ | Earlier run |
| `XDG_RUNTIME_DIR` that cannot be created (read-only parent): tray error, exit 1, nothing left | ✅ | desktop test (scenario 10, libayatana backend, before `12a6185`; with `ksni` sotyware starts normally, scenario 11) |

This environment is **not a real desktop**: the tray host is a test stub, and no
tray icon was ever seen or clicked by a person.

### Not yet validated

| Environment | Status |
| --- | --- |
| Windows 10 | — never built or run |
| Windows 11: a person's visual check of tray icon and menu (manual checklist) | — |
| Windows 11: MSI install/run/uninstall; NSIS interactive install | — |
| Windows 11: focus on second launch while another app is actively in use | — |
| Real GNOME (with and without the AppIndicator extension; without it, dashboard-only mode) | — no real Linux desktop available on 2026-11-10 ([survey](#real-linux-desktop-availability-2026-11-10)) |
| Dashboard-only mode on any real desktop, an XEmbed-only panel, or Wayland | — (virtual display only) |
| Real KDE Plasma | — |
| XFCE | — |
| Real Wayland session with a real tray host | — |
| Focus and un-minimize on second launch, on any real Linux WM | — (skipped under WSLg; passes only under openbox on Xvfb) |
| Headless suite and virtual-display test on GitHub Actions | headless: ✅ two automatic `ci.yml` runs passed, Ubuntu and Windows jobs; ✅ again at `f19dd8b` (run 43360177079, with the rfd guard and its 10 tests on both OSes). Virtual-display: ✅ `desktop-virtual.yml` passed, 92/0/0, superseding the 89/2/1 failure at `14a5de1` (driver bug, fixed in `ead47e7`); ✅ 119/0/0 at `f19dd8b`; not a real desktop ([CI](#ci-github-actions-2026-11-10), [runs at `f19dd8b`](#runs-at-f19dd8b-2026-11-13)) |
| Windows private-desktop test in Session 0 or on a GitHub-hosted runner | — Session 0 not tried. GitHub-hosted runner: not possible, hosted jobs are elevated; the manual workflow failed once and then reported NOT RUN, exit 4, twice (2026-11-10, 2026-11-13 at `f19dd8b`) ([CI](#ci-github-actions-2026-11-10)) |
| Windows tray, real clicks and Quit without an interactive, unlocked desktop | — not possible: `windows.ps1` needs the input desktop with Explorer |
| Headless suite on Windows 10 | — |
| Installing and running `.deb` / `.rpm` / AppImage | — |
| Feature preferences on Windows 10, real Linux desktops, installed copies; corrupt database on the real Windows binary | — (unit tests and the 2026-11-11 desktop runs only) |
| The dataset test's provider walk of `many/` in pages of 336 at Backblaze | — (local listing server only, 2026-11-12; the pages of 7 it replaces passed at Backblaze: [details](#real-windows-desktop-and-fewer-backblaze-listings-2026-11-12)) |
| Downloads through the UI and save dialog at a real provider on Linux or Windows 10; more than one Windows 11 run; downloads at Hetzner | — (Backblaze: once through the Rust download code on WSL2, [details](#one-download-from-backblaze-2026-11-12); once through the native Windows 11 dashboard and save dialog, 35/35, [details](#real-backblaze-download-through-the-windows-dashboard-2026-11-12); both 2026-11-12) |
| Bucket browsing at Hetzner; at Backblaze on a real Linux desktop or Windows 10; more than two pages through the UI | — (Backblaze with content verified 2026-11-11 on WSL2/Xvfb and Windows 11: [details](#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11)) |

To validate an environment, follow the manual checklist in
[testing.md](../development/testing.md#manual-check-real-desktop-linux-or-windows)
and update this table.

## See also

- [limitations.md](limitations.md): known limitations.
- [roadmap.md](roadmap.md): next steps.

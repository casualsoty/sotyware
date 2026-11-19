# Testing guide

Tests are grouped by what the environment must provide. Each category catches
different problems; none replaces another, and passing one says nothing about the
others.

| Category | Command | Runs the release binary? | Needs |
| --- | --- | --- | --- |
| **A. Headless** | `npm run test:headless` | no | Node, Rust and the Tauri build libraries; **no** display, window manager, tray host or D-Bus session |
| **B. Virtual display** (Linux) | `npm run test:desktop:virtual` | yes | Xvfb, openbox, D-Bus, AT-SPI and Python tools; no monitor, no desktop session |
| B′. Simulated desktop on an existing display (Linux) | `tests/desktop/run.sh` | yes | an X11 display you already have (WSLg, or a desktop session) |
| **B″. Private desktop** (Windows) | `npm run test:desktop:windows:isolated` (`tests/desktop/windows-isolated.ps1`) | yes | Windows, a logged-in session (window station `WinSta0`); **no** visible or unlocked screen, no mouse, no taskbar |
| **C. Real desktop**: Windows | `npm run test:desktop:windows` (`tests/desktop/windows.ps1`) | yes | Windows, an unlocked interactive desktop with a taskbar that nobody touches for 30 s |
| **C. Real desktop**: manual | [checklist](#manual-check-real-desktop-linux-or-windows) | yes | a real desktop and a person |

Static checks (`npm run check`, `cargo fmt --check`, `cargo clippy`) need nothing
special and run before every commit ([workflow.md](workflow.md)).

On GitHub Actions, the static checks and category A run automatically on Linux
and Windows; B and B″ run only when started by hand; B′ and C never run there
([CI](#ci)).

**Only category A is headless.** Every test that starts the release binary needs a
display: on Linux an X server (B, B′), on Windows a logged-in session with a window
station (B″, C). No Windows test of the real binary runs without one; see
[Headless Windows desktop testing](#headless-windows-desktop-testing).

### What each category really exercises

B″ (Windows private desktop) covers the rows marked for the Windows test except
the tray rows, real mouse clicks and Quit; details in
[its section](#windows-isolated-desktop-test).

| Behaviour | A. Headless | B. Virtual display | C. Windows test | C. Manual |
| --- | --- | --- | --- | --- |
| Close → hide; second launch → show dashboard | ✅ unit tests (the decision) | ✅ real binary | ✅ real binary | ✅ |
| Mode selection: tray created ⇒ tray mode; no session bus / watcher / host ⇒ dashboard-only; other errors ⇒ exit 1 | ✅ unit tests (classification of the real error types) | ✅ real binary (no watcher, no host, no bus, rejecting watcher) | — (tray mode only) | — |
| Unexpected tray failure → exit 1 | ✅ unit tests (the decision) | ✅ real binary (watcher answers `AccessDenied`) | — | — |
| Dashboard-only: starts visible, note on the page, no Hide button, close → exit 0, second launch → show | ✅ unit tests (decisions), mocked IPC (page) | ✅ real binary, page read via AT-SPI | — | — |
| Implicit exit (`code: None`) blocked | ✅ unit tests (the decision) | — not triggered: the window is never destroyed | — | — |
| Tray menu items and click dispatch (the decision) | ✅ unit tests | menu over D-Bus | ✅ real clicks | ✅ |
| "Hide dashboard" button sends the hide request; errors shown | ✅ mocked IPC | ✅ real button via AT-SPI (tray mode, hides the window) | ✅ real button | ✅ |
| Built-in features: registration, lifecycle table, start/stop failures, isolation, shutdown order, concurrent requests | ✅ unit tests (registry with recording and failing test features) | — | — | — |
| Feature preferences: SQLite round trip, reopen, schema migration and rollback, newer/corrupt/damaged/unopenable/read-only databases, unknown IDs, defaults, save failures, concurrent saves and connections, exit | ✅ unit tests (real SQLite files in a temp folder; policy with an in-memory store) | — | — | — |
| Feature choice survives a real restart (quit, new process) | ✅ unit test with the real store and built-in features (simulated restart) | ✅ real binary: disable → Quit → relaunch: disabled; enable → Quit → relaunch: running; database rows checked | ✅ real binary: the same, in `%LOCALAPPDATA%` | ✅ (step 9) |
| Storage failure shown on the dashboard | ✅ mocked IPC | ✅ real binary: corrupt database and a folder in its place: warning shown, defaults used, "not saved" after a switch, file untouched | — | — |
| Features section lists features and switches them; errors shown | ✅ mocked IPC | ✅ real page via AT-SPI: Scheduled backups listed as running, unchecked → disabled, checked → running (so the static capability allows both commands); listed in dashboard-only mode too | ✅ real WebView2 page via UIA: Scheduled backups listed with version and description, checked and Running; UIA toggle and real mouse click each disable and re-enable it; still Disabled after Quit and relaunch (persisted), Running after enabling, Quit and a second relaunch | ✅ (step 9) |
| Features stopped on exit | ✅ unit tests (`stop_all`) | — not observable from outside | — | — |
| Hide permission granted in tray mode only (runtime capability) | ✅ unit test (the decision) | ✅ tray mode: Hide works; dashboard-only: no button | ✅ tray mode | — |
| The OS window really hides / shows / restores | — | ✅ X11 under openbox | ✅ Win32 | ✅ |
| Focus and un-minimize on second launch | — | ✅ openbox honours the requests | ✅ | ✅ |
| Single-instance handover (D-Bus on Linux, Win32 on Windows), concurrent launches | — | ✅ private session bus | ✅ | ✅ |
| Tray icon registers with a tray host | — | stub host (`sni_watcher.py`) | ✅ real notification area | ✅ |
| Tray icon visible to a person, real tray host (GNOME, KDE, …), Wayland | — | — | partly (icon found by UI Automation) | ✅ |
| Visual appearance | — | — | — | ✅ |
| Screen-reader access | — | — | — | — not in the checklist; untested |

**Mocked or simulated, never real:** in A, the Tauri runtime (recording doubles
of `AppControl`/`DashboardWindow`), the webview (happy-dom) and IPC
(`@tauri-apps/api/mocks`). In B, the tray host (a stub
`StatusNotifierWatcher`; nobody sees an icon), the display (Xvfb, no GPU) and the
desktop (openbox, not GNOME/KDE, not Wayland), and the user (X11 client messages,
D-Bus calls and AT-SPI actions instead of clicks).


## Test profiles (ADR-027)

Every desktop test runs a **test-profile build**, never a normal one, and
never against the user's own data
([ADR-027](../architecture/decisions.md#adr-027-desktop-tests-run-a-test-profile-build-that-cannot-reach-the-users-data)):

```bash
npm run build:test-profile   # = tauri build --no-bundle --features test-profile -- --target-dir target/test-profile
```

The binary is `src-tauri/target/test-profile/release/sotyware(.exe)`; a
recorder build (`--features request-recorder --target-dir target/recorder`)
is a test-profile build too. Neither can be packaged
(`scripts/check-release-binary.mjs`).

- The app starts only with `SOTYWARE_TEST_PROFILE` naming a directory that
  holds `sotyware-test-profile.json` (format 1, purpose
  `sotyware-desktop-test-profile`, run id, 32-digit hex nonce); otherwise it
  prints `sotyware: test profile refused: …` and exits 3 before any window.
  It refuses a profile inside (or around) its own folders, runs under the
  identifier `com.sotyware.desktop.test`, keeps `preferences.sqlite3`,
  `cloud-accounts.sqlite3` and `cloud-places.sqlite3` in `<profile>/data`,
  the webview data in `<profile>/webview`, credentials under service
  `com.sotyware.desktop.test` with user names `<nonce>/cloud-account/<id>`
  (Windows target `com.sotyware.desktop.test/<nonce>/cloud-account/<id>`),
  and writes `sotyware-test-profile-ack.json` once its stores are open.
- The drivers use the helpers `tests/desktop/test_profile.py` (Linux) and
  `tests/desktop/TestProfile.psm1` (Windows): a fresh profile per run (under
  the run's work folder on Linux, `src-tauri\target\desktop-test-profiles\`
  on Windows), the acknowledgement checked after each launch that becomes
  the primary instance (run, nonce, pid, identifier, every path inside the
  profile), launches without a profile and with an invalid one that must
  exit 3, fingerprints of the user's own folders (and, on Windows, of the
  user's `com.sotyware.desktop/` Credential Manager lines) before and after,
  and cleanup of the run's own profile and credentials only. Any mismatch
  stops the run before the window is touched. On Windows the drivers also
  read the user's real app folders through a process WMI starts outside
  their own (possibly MSIX-virtualized) process tree, before and after;
  if that read fails they **stop before launching** (since 2026-11-14).
- Checks without a desktop: `npm run test:profile-helpers` (Python helper,
  10 tests), `tests/desktop/test-profile-selftest.ps1` (PowerShell helper,
  20 checks, Windows), `npm run check:desktop-isolation` (static check of
  every driver, part of `test:headless`, and of `isolation-guard.yml` for
  changes to the drivers alone) and its self-test
  `npm run test:desktop-isolation-guard` (9 tests, including the bypasses
  an audit found on 2026-11-14), and the Rust unit tests of
  `test_profile.rs` and `CredentialNamespace` (also with
  `--features test-profile --target-dir target/test-profile`).
- **On Windows from a coding agent:** the agent's processes may run inside
  an MSIX package (the Claude desktop app) whose `%LOCALAPPDATA%` is
  virtualized; a listing of `%LOCALAPPDATA%` from such a process shows the
  virtual view, not the user's real folder. Use a process started through
  WMI (`Invoke-CimMethod Win32_Process Create`) to look at the real one.

## Headless tests

```bash
npm run test:headless        # = check:rfd-patch && check:release-binary && check:desktop-isolation && check:icons && check:update-key && test:frontend && test:rust
```

`check:rfd-patch` is not a test but a guard: it fails if Cargo no longer
builds rfd from the local patch
([patches/README.md](../../src-tauri/patches/README.md), ADR-023).
`check:update-key` is one too: `src-tauri/update-key.pub` may be empty
(updates off) but must never hold a private key, more than one key line or
the throwaway key of a signing fixture (`scripts/fixtures/**/test-key.pub`,
ADR-039).
`check:release-binary` is another: it fails if a default (or any other
feature) turns on the request recorder (ADR-024,
[below](#request-recorder-tests)).

Runs the [Rust unit tests](#rust-unit-tests) and the
[frontend tests](#frontend-tests). Neither starts sotyware, opens a window or
talks to D-Bus, so no display, window manager, tray host or session bus is needed.
Exits non-zero if any test fails. The same command runs unchanged on native
Windows (PowerShell or `cmd.exe`, MSVC Rust); it does not need, and does not run,
the Linux-only desktop scripts. Results per platform:
[validation matrix](../status/status.md#validation-matrix).

## Rust unit tests

```bash
cd src-tauri && cargo test
```

Keep logic that can be tested without a running Tauri app in plain functions or
types (as `TrayAction::from_menu_id` is) and test it next to its code. Where a
decision triggers an app or window operation, it goes through a small trait
(`AppControl`, `DashboardWindow` in `lib.rs`) so the test can pass a recording
double ([ADR-009](../architecture/decisions.md#adr-009-lifecycle-decisions-are-plain-functions-behind-small-traits)).
The real window, tray and single-instance plugin cannot run under `cargo test`;
they need a display and an event loop.

The feature registry needs no such doubles: it never touches Tauri, so
`features/registry.rs` tests it with plain test features that record their
`start`/`stop` calls in a shared log and fail on request (`TestFeature`), plus
one that blocks in `start` until released (`SlowFeature`, run on a second
thread). They cover registration (order, metadata, duplicate and malformed
IDs, empty fields), the whole lifecycle table, startup (enabled and disabled
features, runs once), start and stop failures (failed state, retry, disable
without `stop`, error kept and cleared), isolation (a failing feature between
two working ones), `stop_all` (reverse order, only running ones, continues
after a failing stop) and concurrency (a starting feature is visible as
`initializing`, a second request is rejected, other features stay usable).
`features/mod.rs` checks that every built-in feature registers and starts and
stops cleanly, and the JSON shapes the frontend parses (`FeatureInfo`,
`FeatureList`, `FeatureChange`). The command functions themselves are one-line
delegations to `feature_list` and `switch_feature`, which are tested; that the
real webview may call them is checked by the virtual-display test.

Feature preferences are tested in two layers
([features.md](../architecture/features.md#persistence)):

- `features/preferences.rs` tests the policy with an in-memory
  `PreferenceStore` double (`MemoryStore`, which can fail loading or saving)
  and small test features (`Flag`, one that fails to start): default without a
  stored choice, a stored choice overriding the default both ways, a new
  feature, unknown stored IDs ignored and kept, saving after a switch and not
  after a rejected one, a failed start saved as enabled and retried at the next
  start, a failed save (switch kept, error returned, old value stored, next
  start uses it), an unreadable or unopenable store (defaults, no writes, every
  save fails), exit (`stop_all` changes nothing stored, saves after `close`
  fail), and 16 threads switching four features at once (every stored value
  equals the final state). The one interleaving that would store a stale value
  with a naive implementation cannot be forced from a test; the code excludes
  it by reading and writing under one lock (ADR-015).
- `storage.rs` tests `SqliteStore` on real database files in a fresh folder
  under the system temp directory: first open creates folder, file and schema
  version 1; save, overwrite and read back; reopening; unknown IDs kept; the
  `CHECK` constraint; no migration on a current database; migration of a
  version-1 database with rows to a test version 2; a failing migration rolled
  back completely; a newer schema refused and the file unchanged; a text file
  and a database with overwritten pages refused and unchanged; a folder that
  cannot be created; a read-only file (loads, saving fails); four connections
  saving concurrently (integrity check passes, last values stored). Two tests
  run the startup and exit sequence of `run()` with the real store and
  `builtin()`: `backup-scheduler` stays disabled, then enabled, across
  simulated restarts, and a corrupt file gives defaults, a save error, and an
  unchanged file. A third loads a database holding a choice for the removed
  `example` feature (sotyware 0.1.0): ignored, the other choices kept, the
  row left in the file.

Cloud storage (`src-tauri/src/cloud/`,
[cloud-storage.md](../architecture/cloud-storage.md)) is tested in three
layers, the first two in every `cargo test`:

- **Pure logic, no server:** provider profiles (unique IDs, consistent
  limits, https endpoints), `AccountConfig::resolve` (endpoints, addressing,
  plain HTTP only on this machine, IDs, regions, bucket names, dotted buckets,
  concurrency bounds), `Accounts` with an in-memory `AccountStore` (add,
  update, remove, restart, duplicates, failed saves, two accounts of one
  provider, a stored account that no longer validates) and an injectable
  `MemoryCredentialStore` that can fail each operation: credentials stored,
  rotated and deleted with their account; two accounts of one provider kept
  apart; missing credentials (`NotFound`); an unavailable backend on every
  path; a failed configuration save rolling the credentials back, and a failed
  rollback reported; a failed rotation keeping the old key and connection; a
  failed credential delete keeping the account; a failed account removal after
  the credentials reported; no secret in any error. `credentials.rs`: the
  stored format (round trip, other formats refused), input validation, and
  `EnvCredentials` (variable names, per-account lookup, missing or padded
  values, no secret in `Debug`/`Display`). `keyring.rs`: entry names derived
  from the account id (and `SERVICE` equal to `tauri.conf.json`'s
  identifier), invalid ids and values refused before the backend is opened,
  the `keyring-core` error mapping (stored bytes never shown) and, on Linux
  only, the `secret-service`/`zbus` error types sorted into unavailable,
  access denied and backend failure. `plan_upload` (single PUT,
  multipart, never one part, provider minimum, very large files, too large) and
  the S3 error mapping. `storage.rs` tests `SqliteAccountStore` on real files:
  every field across a restart, replacing an account and its buckets, removal
  without orphans, no secret in the file or schema, a corrupt file left alone,
  an out-of-range stored value.
- **Against an S3 server in the test process** (`cloud/test_server.rs`:
  `s3s` + `s3s-fs` on 127.0.0.1, a fresh temporary folder per server; no
  network, no account, works on Windows too). `cloud/tests.rs`: single-PUT and
  multipart round trips with SHA-256 verification and byte-for-byte
  comparison, listing buckets and prefixes, wrong secret and unknown key
  (`InvalidCredentials`), unreachable endpoint (`Network`), a server that never
  answers (timeout), an object corrupted at rest (`Integrity`, destination
  never created), an object without stored checksum (`SizeOnly`), no
  overwriting of an existing file, missing objects and buckets, buckets outside
  the account (refused, no request), a missing local file, two accounts with
  different keys uploading and downloading at the same time, and one account's
  full transfer queue not delaying another, a rotated key used by the next
  connection, and removing an account deleting its credentials but not its
  objects. A measurement, not a check:
  `cargo test --release --lib cloud::tests::local_throughput -- --ignored --nocapture`.
- **The dashboard's account commands** (`cloud/dashboard.rs`), with the
  in-memory stores and the in-process S3 server: provider presets as JSON,
  ids from labels (also non-ASCII and very long ones), an added account's JSON
  shape without key id or secret, trimmed inputs, two accounts with the same
  name and provider kept apart, invalid input changing nothing, an
  unavailable or locked credential store saving no account (each its own
  error kind), a rename never touching the credential store (every credential
  call made to fail), hidden settings kept on edit, a new key replacing the
  old one only once stored (old key kept on a refused or invalid replacement),
  a failed save after a new key reported as `partial`, a provider change
  refused, removal kept when the key cannot be deleted and `partial` when only
  the configuration removal fails, unknown accounts, an unusable account
  database, a stored account that no longer validates, `CredentialInput`'s
  redacted `Debug` and unknown fields refused, and no secret in any error.
  Connection checks against the S3 server, which records every operation and
  can refuse chosen ones with `AccessDenied` (`TestServer::start_denying`):
  a valid key → `connected` with exactly one `HeadBucket`; a key refused
  `ListBuckets` (a bucket-restricted Backblaze key) → `connected`, no
  `ListBuckets` sent; `HeadBucket` refused but listing allowed → `connected`
  via one `ListObjectsV2`; both refused → `access-denied`, not invalid
  credentials; wrong secret or unknown key id → `invalid-credentials`; a
  missing bucket → `bucket-unavailable`; an unreachable endpoint →
  `network`; missing, unavailable or locked credentials → their own status
  with no request sent; a result dropped when the account changes. And, on a
  single-threaded runtime with a credential store that blocks until released
  (as during an unlock prompt), that an add and a check leave the async
  thread free.
- **Bucket browsing** (`cloud/browser/tests.rs`), against the in-process
  server's in-memory listing backend (`TestServer::start_listing`: keys
  exactly as given, including `a//b`, keys ending in `/`, spaces and any
  Unicode, with real `ListObjectsV2` prefix, delimiter, `max-keys` and
  continuation-token handling; s3s-fs can hold none of these keys and returns
  no token). Pure rules: breadcrumbs and parents for `""`, `a/`, `a//b/`,
  `/`, `../`; only `""` or prefixes ending in `/` accepted; the marker
  hidden, entries outside the prefix dropped, deeper entries folded into
  their folder; every error kind with sotyware's own text (no provider
  text), a 404 not claiming the bucket does not exist. Through
  `list_folder` with an account and the in-memory credential store: the root
  (immediate folders and files only, size, RFC 3339 date, one
  `ListObjectsV2`, no `ListBuckets`); nested folders one level at a time; an
  empty bucket, a marker-only folder and a missing prefix as empty; spaces,
  Unicode, `+&?#%`, `a//b/`, a leading `/`, `../` and `\` kept exactly;
  pages of 3 over 10 entries with no duplicate or gap and one cursor left;
  a cursor refused for another folder, after use, when unknown, for another
  account at the same provider and after an edit (`outdated-page`), Refresh
  starting again; a token the provider refuses → `outdated-page` (and the
  adapter's own 400); two accounts at the same provider against two servers
  listing only their own objects, and one account's key refused by the
  other's server; missing, unavailable, locked and failing credential
  stores with no request sent; wrong secret, refused listing, missing
  bucket, nothing listening, and a server that never answers (`timeout`),
  without provider text or secret in the errors; a listing refused when the
  account changed or went while it ran, an account without a bucket, a
  prefix that is not a folder; the JSON shape without key or token; and that
  browsing sends nothing but `ListObjectsV2`.
- **Single-file downloads** (`cloud/download/tests.rs`), against the
  in-process server's object backend (`TestServer::start_objects`: exact
  keys, optional `sotyware-sha256` metadata, with or without
  `Content-Length`, bodies that are slow, stall or break off after N bytes).
  Pure rules: the suggested file name (last segment, `..`, `folder/`,
  Windows-illegal and control characters, `CON`/`COM1`, length), key
  lengths, which destinations may be replaced (the dialog's existing file
  only; a folder or relative path refused), the 250 ms progress throttle, and
  every error kind with sotyware's own text. Through `CloudDownloads` and
  `S3Store::download`: a 300 KiB file verified, its messages (one `active` at
  0 bytes first, bytes only growing, exactly one final message, last), no
  temporary file left, one `GetObject`, no secret in any message; an empty
  file; a 24 MiB file whose temporary file grows on disk during the stream
  (no step over 1 MiB); progress throttled with no announced size (`total`
  `null`); cancel during a stalled body (stops within seconds, not by the
  30 s read timeout; another id and a repeat do nothing; nothing left; not
  retryable); cancel right before the final move (cancelled, nothing
  written) and right after it (`finishing`, completed); a SHA-256 mismatch;
  no stored SHA-256 (`size-only`); a connection cut mid-body (failed,
  retried as a new transfer to the same destination, only the latest failure
  retryable); a retry after the missing key was stored; a simulated full disk
  (`disk-full`, temporary file removed); a read-only folder and a vanished
  folder (Unix); an existing destination refused, one that appears during
  the download left untouched, a chosen one replaced, a failed replacement
  keeping the old file; ten unusual keys (`a//b`, `../../outside.txt`,
  Unicode, `+ & ? # %`, `\`, spaces, an emoji) fetched exactly into the
  chosen files and nothing written outside; `a/b` not found where `a//b`
  exists; two accounts at two servers with their own keys and distinct
  transfer ids, one account's key refused by the other; one download or
  dialog at a time (`busy`, a closed dialog frees the slot); a changed bucket
  (`outdated-page`) or removed account with no request sent; quitting during
  a download (cancelled, temporary file removed, no new download); and a
  stalled body ending as `network` after the SDK's ~5 s stalled-stream
  protection.
- **Uploads** (`cloud/s3/upload/tests.rs`, `S3Store::upload`, ADR-021),
  against the in-process server, which records every operation (`UploadPart`
  with its part number), injects faults per operation and part
  (`TestServer::inject`: 500 once, 403, no answer, or a callback such as
  writing a competing object), plus hooks on `TransferControl`
  (`during_hashing`, `during_send(part)`, `before_commit`, `after_commit`)
  and a TCP relay that throttles or stalls the client-to-server direction
  (`ThrottledLink`, small receive buffer). Basics: a small file (exactly
  HEAD, PUT, HEAD), an empty file, a 3-part multipart file (exact operation
  sequence, no multipart files left, downloaded back with SHA-256), Unicode,
  `+ & = % ; # ~`, spaces and nested keys kept exactly, an existing key
  refused after one HEAD (also through `upload_file`), an object created by
  another client during part 2 found by the second check (aborted, theirs
  untouched), a stored object of the wrong size or without the SHA-256
  metadata failing verification with `NeedsInspection` and **not** deleted.
  Cancellation: before the start (no request), during hashing (no request),
  during a single PUT and during part 2 (nothing stored; aborted, no part 3,
  no completion), right before completion and right before a PUT's last
  chunk (nothing stored), after the commit point (both `cancel()` calls
  return `false`, the upload completes and verifies), repeated (always
  `true`), and one upload cancelled while another with its own control
  completes. Multipart cleanup: a refused part and a refused completion
  aborted, nothing left; a refused abort and an abort that never answers
  (given up after `abort_timeout`) reported as `IncompleteParts`, with the
  parts really still on the server. Integrity: same-size changes during
  hashing, during a PUT and to a later part during multipart, a truncation,
  and a rewrite with the same content (new modification time), all
  `FileChanged` and never completed; per-part SHA-256 at exact boundaries
  (multiples, +1, single, empty); a transient 500 on part 2 and on a PUT
  retried by the SDK with a fresh hash, verified, content equal. Timeouts:
  the control experiment (a plain `PutObject` through a 1 MiB/s relay fails
  with the 1 s read timeout), a 12 MiB PUT through a 2 MiB/s relay taking
  about 7.5 s with that same read timeout (passes; at least 8 progress
  updates while sending), and a relay that stalls after 256 KiB (timeout
  after the 1.5 s stall timeout, nothing stored). Progress in every test:
  stages forward only, bytes never down, throttled to 250 ms except a
  stage's last, `verifying` last only when completed; the throttle and its
  held-back update also as pure unit tests. Opt-in, about 72 s: one 7 MiB PUT
  through a 100 KiB/s relay with the **default** 60 s read and stall
  timeouts:
  `cargo test --lib cloud::s3::upload::tests::a_slow_upload_longer_than_a_minute -- --ignored --nocapture`.
  What the server cannot model: a provider's real conditional-write or
  versioning behaviour, a provider that refuses after reading the body
  (s3s answers before reading it, so a refused part reaches the client as
  403 or as a broken pipe), and socket buffers of a real slow link (the
  relay runs on loopback).
- **Upload batches** (`cloud/upload/tests.rs`, `CloudUploads`, ADR-022),
  against the same in-process server (`TestServer::start`, s3s-fs, real
  multipart), with test hooks on `CloudUploads` (`before_file(index)`,
  `file_control(index, control)` to reach each file's `TransferControl`
  hooks) and the dialog's answer passed straight to `begin_upload` (no
  dialog is opened). Pure rules: the selection checks (a file, a folder, a
  missing file, Unicode and `+ & ? # %` kept, control characters incl. DEL
  and C1, a non-UTF-8 name on Unix and an unpaired surrogate on Windows,
  both on real files: skipped with `invalid-name`, no key, the path kept
  byte for byte or code unit for code unit),
  keys of exactly 1,024 and 1,025 bytes, provider size limits, duplicate
  keys from two folders with case kept), prefixes, the sorting of every
  error into skip / file failed / stop batch / cancelled and sotyware's own
  messages without local paths, and the JSON shape of events. Through
  whole batches: a cancelled and an empty dialog (nothing started, no
  request), 101 files refused and 100 uploaded, no valid file (no batch),
  skipped invalid and duplicate files beside an uploaded one, an existing
  object skipped before hashing (one HEAD, theirs untouched), a conflict
  created during hashing found by the upload's own check (skipped);
  one file at a time across a multipart, a small and an empty file (never
  two in flight, operations never interleaved) with a second batch and
  dialog refused (`busy`) meanwhile; a download completing while a batch is
  uploading; cancel during hashing, during a single PUT, during multipart
  part 2 (aborted, nothing left), between files, after the commit point
  (`finishing`, that file completes, no other starts), idempotent and only
  for its own id; a changed file failing alone while the next completes;
  access denied stopping the batch, then a retry rechecking a destination
  created meanwhile; a retry of only the eligible files; `NeedsInspection`
  (verification refused after the commit point) shown, not deleted, never
  retried; `IncompleteParts` (abort refused) kept, cleaned up on retry with
  a `cleanup` event and never retried, a cleanup that fails again keeping
  the id; missing credentials failing the batch before any request, then a
  retry; a stale batch id cancelling or retrying nothing; and shutdown
  waiting for a slow multipart abort, aborting known incomplete uploads,
  returning within its limit when aborts hang (the id kept), and refusing a
  batch whose dialog was open. Every batch test also checks the rules each
  batch keeps: the slot is free when `finished` arrives, `finished` comes
  exactly once and last, `seq` counts 1, 2, 3, …, progress never goes
  backwards, files never overlap, every file ends final, and no result or
  event contains the test folder's path.
- **The patched rfd** (`src-tauri/patches/rfd/`, ADR-023), Windows only,
  not part of `npm run test:headless`: from `src-tauri`, `cargo test
  --manifest-path patches/rfd/Cargo.toml --lib --locked` (rfd as its own
  package with its published lockfile, because Cargo refuses `cargo test
  -p rfd` for a non-member with dev-dependencies; `--target-dir` keeps its
  build elsewhere if wanted). Five tests at the end of
  `backend/win_cid/file_dialog/com.rs`: ASCII, Unicode, empty and
  unpaired-surrogate wide strings read back unchanged and not replaced by
  U+FFFD, and real files (two with unpaired surrogates) through
  `SHCreateItemFromParsingName` → `IShellItem::get_path`, the function
  behind every Windows dialog result: the same code units and the same
  file come back. On Linux the crate's tests run but these are not
  compiled.
- **Real providers**, opt-in only: [below](#cloud-storage-integration-tests).
- **The real OS credential store**, opt-in only:
  [below](#os-credential-store-test).

## OS credential store test

`cloud::keyring::os_backend::round_trip` is `#[ignore]`d: ordinary `cargo test`
and CI never touch a keyring. It uses two new account ids per run,
`it-keyring-<nanoseconds>-a` and `-b` (never a real account), checks that both
start empty (`NotFound`), stores `a`, reads it back, stores `b`, replaces `a`
and checks `b` is unchanged, deletes `a` (twice; the second is not an error)
while `b` stays, deletes `b`, and checks with a fresh store that both are gone.
A guard deletes both entries however the test ends. On Windows it also asks
`cmdkey /list:<target>` that the entry exists under
`com.sotyware.desktop/cloud-account/<id>` after storing and is gone at the
end.

```bash
cd src-tauri
cargo test --lib cloud::keyring::os_backend -- --ignored --nocapture
```

With `SOTYWARE_IT_KEYRING_EXPECT=unavailable` it checks the opposite: every
call (store, load, exists, delete) must fail with `Unavailable`. Use it where
no Secret Service runs:

```bash
SOTYWARE_IT_KEYRING_EXPECT=unavailable cargo test --lib cloud::keyring::os_backend -- --ignored --nocapture                  # WSL2: bus socket missing
dbus-run-session -- env SOTYWARE_IT_KEYRING_EXPECT=unavailable cargo test --lib cloud::keyring::os_backend -- --ignored      # a bus, no provider
```

**Windows:** run it in PowerShell from a Windows checkout; it writes to your
own Credential Manager (and removes what it wrote). `cmdkey /list |
Select-String com.sotyware.desktop` should print nothing afterwards.

**Linux desktop:** run it in the desktop session; GNOME Keyring or KWallet
may ask to unlock. Seahorse ("Passwords and Keys") shows the entries while the
test runs.

**WSL2 (no desktop keyring):** a real GNOME Keyring can run on a private
session bus with a throwaway keyring, never touching `~/.local/share/keyrings`.
Without root, unpack the Ubuntu packages `gnome-keyring`, `libgck-1-0` and
`libgcr-base-3-1` (`apt-get download`, or the same files from an Ubuntu
mirror, checking each SHA-256 against `apt-cache show <pkg>=<version>`) with
`dpkg -x` into a folder `$GK`, then, in one script run under
`dbus-run-session --`:

```bash
export HOME=$(mktemp -d) XDG_DATA_HOME=$HOME/data GSETTINGS_BACKEND=memory
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/gk.XXXXXX)   # short: a Unix socket path must stay under 108 bytes
export CARGO_HOME=<your ~/.cargo> RUSTUP_HOME=<your ~/.rustup> LD_LIBRARY_PATH=$GK/usr/lib/x86_64-linux-gnu
printf 'test-only-password' | $GK/usr/bin/gnome-keyring-daemon --foreground --unlock --components=secrets &
# wait until org.freedesktop.secrets has an owner (dbus-send … NameHasOwner), then:
cargo test --lib cloud::keyring::os_backend -- --ignored --nocapture
```

`--unlock` creates and unlocks a "login" collection with that password.
Locking it (`org.freedesktop.Secret.Service.Lock` on
`/org/freedesktop/secrets/collection/login` with `dbus-send`) makes the next
write ask for an unlock prompt that cannot be shown there: `AccessDenied`
("prompt dismissed"), nothing written. This is a real Secret Service
implementation on a real bus, but not a desktop session: no prompt is ever
shown to a person.

## Cloud storage integration tests

`cloud/integration.rs` holds one test per provider, `#[ignore]`d, so neither
`cargo test` nor CI ever contacts a provider. Each one, against a bucket you
name: lists buckets (a bucket-restricted key may be refused; that is
reported, not failed), checks that its run prefix `sotyware-it/<run id>/` has
no objects and no versions, uploads a 200 kB file (one PUT) and a 12 MiB file (3 parts of 5 MiB),
checks the stored SHA-256, downloads and compares both, lists them, checks
that each upload returned a version id, deletes the small file without a
version id and checks that this only hides it (one delete marker, the version
still stored, `NotFound` on download, no partial file), checks that a wrong
secret gives `InvalidCredentials`, then deletes **exactly the versions and the
delete marker it created** (never by prefix) and checks that the prefix has no
objects and no versions again. The version listing goes straight through the
SDK client (`S3Store::sdk_client`, test builds only); the adapter itself offers
no version listing. It prints timings and the endpoint. Test data is generated; no personal
file is read. Use a dedicated test bucket anyway.

`backblaze_b2_connection_check` (B2 only, also `#[ignore]`d) **writes
nothing**. It runs the dashboard's connection check (`S3Store::check_bucket`)
in the five cases one valid key allows, and records every HTTP request the
SDK really sends (method, path and query, status; headers, and so the
signature, are never read) through an SDK interceptor added in test builds
only (`S3Store::with_interceptor`):

| Case | Must be |
| --- | --- |
| the key and its bucket | `connected` |
| the key id with a wrong secret | `invalid-credentials` |
| a well-formed key id no account has (`003` + zeros) with the secret | `invalid-credentials` |
| the key with a bucket name new for the run (`sotyware-it-missing-<run>`) | `access-denied` or `bucket-unavailable`, never `invalid-credentials` or `connected` |
| `http://127.0.0.1:9` (nothing listens) | `network` |

For every request: only `HEAD` or `GET`, never `GET /` (`ListBuckets`), every
`GET` a `ListObjectsV2` with `max-keys=1`; each case starts with `HEAD` and
sends at most two requests (one attempt each). Before and after, it hashes
every object version and delete marker under `sotyware-it/` (the prefix the
test key is restricted to) and fails if anything changed. It prints each
case's status and requests with the bucket name replaced by `<bucket>`.
`connection_check_cases_against_the_test_server` runs the same cases and
assertions against the in-process server in every `cargo test`, so the
recording itself is tested.

`backblaze_b2_folder_listing` (B2 only, `#[ignore]`d) **writes nothing**
either: it lists like the bucket browser (`S3Store::list_folder`) with the
requests recorded. The root, which a key restricted to a file name prefix may
be refused (only a listing or `AccessDenied` passes); every folder under
`sotyware-it/`, one level per request (at most 25 folders), checking that
each page holds only immediate children inside its prefix and no more than
`max-keys`; the same prefix in pages of one entry against one page of 1,000
(same entries, none twice); a continuation token Backblaze never issued
(printed, not judged); and the dashboard's `list_folder` with an in-memory
credential store for `sotyware-it/` and the root. Every request must be a
`GET` with `list-type=2` and `delimiter=%2F`; the version fingerprint under
`sotyware-it/` must be unchanged. It prints counts, never names.
`folder_listing_cases_against_the_test_server` runs it against the in-process
listing server in every `cargo test`.

```bash
cargo test --lib cloud::integration::backblaze_b2_folder_listing -- --ignored --nocapture
```

`backblaze_b2_fingerprint` only prints that hash
(`sotyware-it fingerprint <sha256> versions <n>`), for the UI tests below.

```bash
cargo test --lib cloud::integration::backblaze_b2_connection_check -- --ignored --nocapture
```

**Backblaze B2:**

1. In the Backblaze web console: create a private bucket for tests only (for
   example `sotyware-it-<something unique>`), **Object Lock disabled** (locked
   versions could not be cleaned up). Note the endpoint shown with it
   (`s3.<region>.backblazeb2.com`); the part between `s3.` and
   `.backblazeb2.com` is the region (`eu-central-003` for EU Central,
   confirmed 2026-11-11).
2. Application Keys → Add a New Application Key: access to that bucket only,
   read and write, file name prefix `sotyware-it/` (every test key is under
   it), "list all bucket names" off; an expiry is a good idea. Note the keyID
   and the applicationKey (shown once).
3. Run, in `src-tauri/`, typing the values at the prompts so they stay out of
   the shell history and the command line:

   ```bash
   export SOTYWARE_IT_B2_REGION=eu-central-003
   read -r SOTYWARE_IT_B2_BUCKET && export SOTYWARE_IT_B2_BUCKET
   read -rs SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID && export SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID
   read -rs SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY && export SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY
   cargo test --lib cloud::integration::backblaze_b2_round_trip -- --ignored --nocapture
   ```

   How the 2026-11-11 runs were done, so an agent (whose shell does not keep
   variables between commands) could run them without seeing the values:

   - WSL2: the four `NAME=value` lines in `~/.config/sotyware-it/b2.env`
     (directory mode 700, file 600, written in an editor, outside the
     repository); a script outside the repository did `set -a; .
     ~/.config/sotyware-it/b2.env; set +a`, ran the `cargo test` command above
     and replaced the key id, secret and bucket name in the output with
     `<keyID>`, `<secret>`, `<bucket>`.
   - Windows: `Get-Credential | Export-Clixml
     $env:USERPROFILE\.sotyware-it\b2.xml` (keyID as user name, applicationKey
     as password; encrypted with DPAPI for that Windows user) and the bucket
     name in `b2-bucket.txt` next to it. A script outside the repository
     reloaded the Windows `PATH`, set `$env:SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID
     = $c.UserName` and `..._SECRET_ACCESS_KEY =
     $c.GetNetworkCredential().Password` from `$c = Import-Clixml …`, ran the
     same `cargo test` in `C:\dev\sotyware\src-tauri` and redacted the
     output the same way.

   Delete these files (and the key in the console) when the bucket is no
   longer used for tests.

B2 buckets are always versioned; the test deletes its versions by id, so
nothing stays billed. A failed run prints `CLEANUP FAILED` lines naming what to
delete by hand.

**Hetzner Object Storage:**

1. In the Hetzner Console, in a project: Object Storage → create a bucket in
   fsn1, nbg1 or hel1 (private). Then Security → S3 credentials → generate;
   note the access key and the secret key (shown once). The key reaches every
   bucket of the project, so prefer a project used only for tests.
2. Run, in `src-tauri/`:

   ```bash
   export SOTYWARE_IT_HETZNER_BUCKET=<bucket> SOTYWARE_IT_HETZNER_REGION=fsn1
   export SOTYWARE_CLOUD_IT_HETZNER_ACCESS_KEY_ID=<access key>
   read -rs SOTYWARE_CLOUD_IT_HETZNER_SECRET_ACCESS_KEY && export SOTYWARE_CLOUD_IT_HETZNER_SECRET_ACCESS_KEY
   cargo test --lib cloud::integration::hetzner_round_trip -- --ignored --nocapture
   ```

   Hetzner bills its base fee hourly (€0.0104/h, at most €6.49 a month,
   excl. VAT) for every hour in which at least one bucket exists; delete the
   test bucket afterwards if it is not kept.

On Windows (PowerShell), set the same variables with `$env:NAME = '…'`; for
the secret, `$env:NAME = Read-Host` keeps it out of the shell history. Record
results in [status.md](../status/status.md) with the date, platform and
provider; a test that was not run is not a pass.

### Browsing a temporary dataset at Backblaze

`backblaze_b2_browse_dataset` (`cloud/integration/browse.rs`, `#[ignore]`d)
is the only browsing test that **writes**: it creates a small dataset in the
B2 test bucket, browses it with the dashboard's own code
(`browser::list_folder` through `CloudAccounts`, in-memory credential store)
and deletes it again. Rules it enforces:

- Every write is one `PutObject` of a dataset key under
  `sotyware-it/browse-validation/<unix nanos>-<pid>/` (new for every run).
  Before the first write it checks the provider (`backblaze-b2`), that the
  endpoint is `https://s3.<region>.backblazeb2.com`, the connection check,
  and that the run prefix has no object, version or delete marker; then one
  probe object proves write, read and delete-by-version-id inside the prefix
  (and is deleted).
- The dataset (1,040 objects, about 50 kB, contents derived from the key,
  SHA-256 stored as metadata): files at the run root (a space, an emoji,
  `café` in NFC and in NFD), folders 1 to 5 levels deep, a folder with a
  marker object and content, two marker-only (empty) folders, Unicode folder
  and file names (Latin, Cyrillic, Arabic, Japanese), `+ & ? # %` and a
  literal `%20` in file and folder names, `double//` and `double///`,
  `dots/../escape.txt`, `dots/..`, `dots/./here.txt`, and `many/` with 1,005
  files and 5 subfolders (two pages of the browser's 1,000). Keys a provider
  may refuse (repeated separators, `.`/`..`) are recorded, not failed.
- The checks run in their own task, so a failed check (or a panic) cannot
  skip the cleanup. Cleanup lists every version and delete marker under the
  run prefix, deletes by version id **only those whose key is a dataset key
  of this run**, reports (never deletes) anything else, lists again and
  fails unless nothing is left. The fingerprint of `sotyware-it/` must be
  the same before and after.

What it checks (each line printed as `PASS`/`FAIL`): every folder, opened
only by prefixes the listing returned, shows exactly its immediate
subfolders and its own files with their sizes and a date from this run,
nothing twice; every folder implied by the keys is reachable; marker objects
are not files; `..` is a folder named `..` whose parent is `dots/`;
`double/` holds one folder without a name; `many/` comes in two pages
(1,000 + 10); a used cursor and a cursor of another folder are refused as
`outdated-page`; Refresh gives the same first page with a new cursor; the
provider's own pages of 4 (run root), 336 and 1,000 (`many/`) each give
exactly the expected slice of the folder in key order, at least two pages
(pages of 336 split `many/` as 336 files, 336, 333 + 3 subfolders, 2
subfolders: boundaries inside the files, inside the subfolders and a page
with both); one listing is sent per page and every request is a delimited
`ListObjectsV2`; HEAD of every key outside
`many/` returns the uploaded size and SHA-256. The same folders are listed
from the in-process listing server holding the same keys and every
difference is printed (a comparison, not a failure); a made-up continuation
token is printed, not judged. `browse_checks_pass_against_the_test_server`
runs all these checks against the in-process server in every `cargo test`.

**Billed listings.** Every listing is a Backblaze class C transaction and
counts against the account's daily cap (reached once on 2026-11-11). One
dataset check sends about 44 listings: 34 from the dashboard path (the 26
folders, `many/` in the browser's two pages, the cursor and Refresh checks;
refused cursors never reach the provider) and 10 from the provider walk.
Until 2026-11-12 that walk used pages of 7 for `many/` (145 listings, about
185 in all); keep its page sizes large. A UI run adds about 50 listings of
its own (the browser's page size, 1,000, is not changed for tests).

```bash
cargo test --lib cloud::integration::browse::backblaze_b2_browse_dataset -- --ignored --nocapture
```

With `SOTYWARE_IT_B2_BROWSE_KEEP=1` the dataset is kept (for the UI tests
below), `SOTYWARE_IT_B2_BROWSE_MANIFEST=<file>` receives the listing of every
folder as JSON, and the run id is printed. `backblaze_b2_browse_cleanup` with
`SOTYWARE_IT_B2_BROWSE_RUN=<run id>` then deletes that run exactly as above;
use it also after an interrupted run (the run id is the last path part of the
printed run prefix).

**The read-only key for the UI.** The test key is limited to the file name
prefix `sotyware-it/`, and Backblaze refuses such a key the root listing;
the browser always opens at the root, so the UI can reach the dataset only
with a second key: Application Keys → Add a New Application Key, the test
bucket only, **Read Only**, **no** file name prefix, "list all bucket names"
off, a short expiry. Its variables are `SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID`
and `SOTYWARE_CLOUD_IT_B2_READ_SECRET_ACCESS_KEY` (WSL2 run of 2026-11-11: a
second 0600 file `~/.config/sotyware-it/b2-read.env`; Windows:
`Get-Credential | Export-Clixml $env:USERPROFILE\.sotyware-it\b2-read.xml`).
When they are set, `backblaze_b2_browse_dataset` also checks that this key
lists the root and the run but is **refused a write** (a `PutObject` of
`read-key-probe.txt` in the run prefix; were it accepted, that version would
be deleted at once and the test would fail). All writes stay with the
prefix-restricted key.

### One download from Backblaze

`backblaze_b2_download_file` (`cloud/integration/download.rs`, `#[ignore]`d)
checks one download from the real B2 test bucket through the code behind
`download_cloud_file`: `CloudDownloads` with an in-memory `CloudAccounts`
and credential store, a recorder in place of the page's channel, and a
path in a temporary folder in place of the save dialog. The bucket has no
permanent test object (every test deletes what it created), so the test
makes its own:

1. Checks that the account is Backblaze's own endpoint for the region and
   that the read-only key differs from the write key; then, write key, one
   `ListObjectVersions` of the new run prefix, which must hold no object,
   version or delete marker (nothing is uploaded otherwise). Generates
   16 MiB + 3 bytes (deterministic pattern) in a temporary folder and
   computes their SHA-256 **locally, before the upload**: the independent
   expectation.
2. Write key: `upload_file` of `sotyware-it/download-validation/<unix
   nanos>-<pid>/größe test.bin` (a space and non-ASCII letters, so the key
   must come back exactly), with the `sotyware-sha256` metadata as the app's
   uploader sets it: since ADR-021 a `HeadObject` (must find nothing), one
   `PutObject` and a verifying `HeadObject`, so the write key needs read
   access too. It must return a version id, or the test stops and asks
   for manual cleanup.
3. Read-only key: one `HeadObject`; size and `sotyware-sha256` must equal
   the local values.
4. Read-only key: one `GetObject` through `CloudDownloads` to a new
   destination in another temporary folder. The final message must be
   `completed` with integrity `sha256-verified` and bytes = total = the
   size; every "active" message after the first must carry the
   `Content-Length` as total, with bytes never going down; when the final
   message is sent the destination must have the full size and no
   `.sotyware-partial` file may exist; the file's SHA-256 must equal the
   local one and its bytes the generated file's.
5. Write key, also after a failed check: `DeleteObject` of exactly that
   version id, then one `ListObjectVersions` of the run prefix, which must be
   empty.

Eight remote requests per run (five in the recorded run of 2026-11-12: the
listing before the upload and the upload's two `HeadObject` were added
afterwards and have not run against Backblaze yet); every client makes one attempt, so nothing is retried
against the daily transaction cap. Requests from the write key and
the `HeadObject` are recorded and printed (bucket as `<bucket>`; the
`GetObject` goes through the app's own client, without the recorder). It
needs both keys' variables:

```bash
set -a; . ~/.config/sotyware-it/b2.env; . ~/.config/sotyware-it/b2-read.env; set +a
cd src-tauri && cargo test cloud::integration::download::backblaze_b2_download_file -- --ignored --nocapture
```

**Costs.** Each request counts against the B2 account's daily transaction
caps (`HeadObject`, `GetObject`: class B; `ListObjectVersions`: class C;
upload and delete: class A); the run downloads 16 MiB, and the 16 MiB
version is stored (and billed) only until the cleanup deletes it. A failed
cleanup prints `CLEANUP FAILED` with the key and version id to delete by
hand; nothing is retried.

`download_through_app_against_the_test_server` runs step 4's checks against
the in-process server in every `cargo test`. Locally the 16 MiB arrive
within the 250 ms progress interval, so there are only the two "active"
messages with 0 bytes.

### Downloading through the Windows dashboard at Backblaze

`windows-cloud-ui.ps1 -Mode b2-download` (see the
[UI end-to-end test](#cloud-storage-ui-end-to-end-test)) downloads one file
from the real B2 test bucket through the release binary's dashboard and the
native Windows save dialog. Two opt-in Rust tests in
`cloud/integration/download.rs` do the provider side, with the write key
(`b2.xml`), one attempt per request:

Both print a failure's reason on stdout as well as in the panic, because
the driver keeps only stdout.

- `backblaze_b2_download_ui_prepare`: checks that the account is Backblaze's
  own endpoint and that the read-only key differs from the write key, makes
  a new run id, lists the versions of
  `sotyware-it/download-ui-validation/<run id>/` (must be none: no object,
  version or delete marker), generates 16 MiB + 3 bytes into
  `SOTYWARE_IT_B2_DOWNLOAD_UI_DIR` (a new, empty folder under `%TEMP%`),
  computes their SHA-256, uploads them as `größe test.bin` with the
  `sotyware-sha256` metadata, prints the run id and version id at once and
  writes `manifest.json` (key, size, SHA-256, version id). The object is
  kept for the dashboard. Requests: `ListObjectVersions`, then the upload's
  `HeadObject`, `PutObject` and verifying `HeadObject` (ADR-021; not yet run
  at Backblaze with these two HEADs).
- `backblaze_b2_download_ui_cleanup`: `DeleteObject` of exactly
  `SOTYWARE_IT_B2_DOWNLOAD_UI_VERSION` (never without a version id), then
  one `ListObjectVersions` of the run prefix, which must be empty. If the
  delete is refused (for example by the daily transaction cap) it sends
  nothing more and fails with the run id and version id.

If the prepare step fails, or its manifest does not name the run and version
the upload printed, the script stops before starting the app: it deletes
only a version the upload reported (by id) and removes both local folders.
The cleanup always uses the run and version id from the upload's own output
line, never a listing.

The script adds the account with the **read-only key** (`b2-read.xml`) through
the form, checks the connection, opens the bucket, goes `sotyware-it` →
`download-ui-validation` → the run folder (which must show exactly one row,
`größe test.bin`, 16 MiB), presses Download, checks that the save dialog
suggests the name unchanged, saves into a second new folder under `%TEMP%`,
and samples the page's progress line until the final message. It checks:
every progress line says "of 16 MiB" (the total from `Content-Length`) and
never goes down (percent and the shown size; the exact byte counts stay in
the channel, so they are checked only by the Rust test above); the final
message is "Downloaded 16 MiB. The SHA-256 checksum stored with the file
matched." (`sha256-verified`: the `GetObject` response carried the
`sotyware-sha256` metadata, equal to the received bytes' hash); at the moment
it appears the file is already in place at full size (opened with sharing,
since NTFS directory entries lag) and no `.sotyware-partial` exists; the saved
file's size, its SHA-256 (`Get-FileHash`) and every byte equal the generated
original's. Then the usual tail (rename, replace key, remove; Credential
Manager as before). Always, also after a failed check: the cleanup test, and
both local folders removed. No fingerprint of `sotyware-it/` is taken, so the
rest of the bucket is not listed. Requests per run: write key 6 (above), read
key about 6 through the app (`HeadBucket`, `ListObjectsV2` of the root,
`sotyware-it/`, `download-ui-validation/` and the run folder, one
`GetObject`; not recorded, the app's own client). Costs as for the
[Rust download test](#one-download-from-backblaze). It needs the Rust
toolchain (the two tests are compiled on the first run).

### Uploads at Backblaze

**Status: passed against Backblaze once on WSL2 and once on native
Windows 11 (2026-11-13, 41 requests each, cleanup verified empty each
time;
[WSL2](../status/status.md#uploads-at-backblaze-2026-11-13),
[Windows](../status/status.md#uploads-at-backblaze-on-native-windows-11-2026-11-13)).** `backblaze_b2_upload_batch`
(`cloud/integration/upload.rs`, `#[ignore]`d) sends one controlled upload
validation to the B2 test bucket through the code behind
`upload_cloud_files`: `begin_upload` and `CloudUploads` with an in-memory
`CloudAccounts` and credential store, a fixed file list in place of the
file dialog, and a recorder in place of the page's channel. The downloads
go through `CloudDownloads`, as in the
[download test](#one-download-from-backblaze). Everything uses the
**write key only** (`b2.env` / `b2.xml`), which Backblaze limits to
`sotyware-it/`. The read-only key is not used.

**What it does**, in order. Each client makes one attempt per request.
Multipart starts at 6 MiB in parts of 5 MiB for this test only (the app
defaults are 64 and 16 MiB).

1. **Gate, before any request.** `SOTYWARE_IT_B2_UPLOAD_CONFIRM` must be
   exactly `upload-validation`, and the account must be Backblaze's own
   endpoint for the region. A new run id `<unix nanos>-<pid>` is made and the
   run prefix `sotyware-it/upload-validation/<run id>/` is printed.
2. **Preflight.** One `ListObjectVersions` and one `ListMultipartUploads` of
   the run prefix. Both must succeed and find nothing (no version, delete
   marker or incomplete multipart upload). Otherwise the test stops and
   nothing is written.
3. **Batch 1**, three generated files (SHA-256 computed locally first):
   `empty.bin` (0 bytes, one PUT), `größe +test.bin` (200,003 bytes, one
   PUT; a space, `+` and non-ASCII letters), `multipart.bin` (10 MiB + 3,
   three parts). The batch must end `completed` with every file
   `completed`, under its key exactly as named, bytes = total = size. Then
   one `HeadObject` each: size and `sotyware-sha256` must equal the local
   values.
4. **Existing key.** Batch 2 picks a different local file named
   `größe +test.bin`. It must be `skipped` with `destination-exists` and
   not `started` (the batch's `HeadObject`; the file is not read). Then
   `S3Store::upload_file` of that file to the same key must fail with
   `AlreadyExists` (its own `HeadObject`; nothing sent).
5. **Cancellation.** Batch 3 uploads `cancelled.bin` (10 MiB + 3) and is
   cancelled once part 2 has begun. It must end `cancelled` with remote
   state `nothing-stored` (the multipart upload was aborted), no unresolved
   upload id, and `HeadObject` must find nothing.
6. **Downloads.** Each of the three uploaded files is downloaded once
   through `CloudDownloads`. Each must end `completed` with
   `sha256-verified` and the full size, and its SHA-256 and bytes must equal
   the source. `größe +test.bin` must therefore still be the first upload.
7. **Cleanup** (`cleanup_run`, also after a failed check: the checks run
   in their own task):
   - `ListMultipartUploads` of the run prefix, then `AbortMultipartUpload`
     by upload id, only for this run's keys.
   - `ListObjectVersions`, then `DeleteObject` **with the version id**,
     only for this run's keys. These four names are the only keys a run
     creates.
   - Anything else found is reported (`NOT OURS, left alone`) and left as
     it is.
   - Then both listings once more: both must be empty.
8. **After cleanup**, from what step 7 found:
   - Exactly three versions: the three keys exactly as uploaded (the
     Unicode key comes back as given), sizes 0, 200,003 and 10 MiB + 3.
   - No delete marker.
   - No version of `cancelled.bin`.
   - Only one version of `größe +test.bin`: the existing key was not
     overwritten.
   - No incomplete multipart upload before the aborts: the cancellation's
     own abort worked.

   From the recorded requests: three `UploadPart`s of `multipart.bin`, one
   abort of `cancelled.bin`, three deletes by version id, at most 45
   requests.

**Run scope: enforced before a request is sent.** Every request goes
through one client, which the batch and the downloads also use
(`CloudAccounts::set_connected`, test builds only). That client carries
`RunScope`, an SDK interceptor that refuses a request in
`read_before_transmit`, before it is sent, unless it is one of these:

- `GET` of the bucket with `versions` or `uploads`, `prefix` exactly the
  run prefix and no delimiter;
- a request for one of the four run keys (path percent-decoded and
  compared exactly), `HEAD`, `GET`, `PUT` or `POST`;
- a `DELETE` of a run key with a non-empty `versionId` or `uploadId`. A
  plain delete, which on B2 would only hide the object, is refused.

`ListBuckets`, `ListObjectsV2`, `HeadBucket`, `DeleteObjects` and any key
outside the run are refused. A refusal fails the test and lists what was
refused. `run_scope_refuses_requests_outside_the_run` checks this against
the in-process server: six such calls, and none reaches it. It also checks
that the SDK's own `ListObjectVersions` and `ListMultipartUploads` of the
run prefix pass the scope and reach the server.

**Expected requests** (write key, one attempt each; counted from the local
run plus the provider-only listings):

| Request | Count | B2 class |
| --- | --- | --- |
| `ListObjectVersions` (run prefix) | 3 (preflight, cleanup, check) | C |
| `ListMultipartUploads` (run prefix) | 3 (same) | C |
| `HeadObject` | 18 (batch and upload checks, step 3, step 4, step 5) | B |
| `GetObject` | 3 | B |
| `PutObject` | 2 | A |
| `CreateMultipartUpload` / `UploadPart` / `CompleteMultipartUpload` / `AbortMultipartUpload` | 2 / 4–5 / 1 / 1 | A |
| `DeleteObject` with version id | 3 | A |

About 41 requests. About 16–21 MiB are sent (part 2 of `cancelled.bin` is
cut off) and about 10 MiB downloaded. About 10 MiB is stored, and billed,
for the minutes until cleanup. Class B and C requests count against the
account's daily free allowance and its daily caps (the class C cap was
reached once, on 2026-11-11, by the browse test). Check Backblaze's current
price list before running; prices are not repeated here. Backblaze
accepted `ListMultipartUploads` of the run prefix with the
prefix-restricted key in the 2026-11-13 run. If it ever refuses it, the
preflight fails before any write.

**Credentials and permissions.** These are the variables of the
[integration tests](#cloud-storage-integration-tests):
`SOTYWARE_IT_B2_BUCKET`, `SOTYWARE_IT_B2_REGION`,
`SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID` and
`SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY`, plus
`SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation`. The key is the existing
write key:

- the test bucket only, read and write, so it can list, read, write and
  delete files;
- file name prefix `sotyware-it/`;
- "list all bucket names" off.

Object Lock must be off on the bucket, or the version deletes fail. The test
reads no secret into its output. It prints the run prefix, request paths
(bucket as `<bucket>`), sizes, SHA-256s and version ids. Provider error
text is printed as the SDK reports it, so redact the output as for the
earlier runs before sharing it.

**Running it explicitly** (it is never part of `cargo test`,
`test:headless` or CI):

```bash
# WSL2, in src-tauri/, with the 0600 file from the integration tests
set -a; . ~/.config/sotyware-it/b2.env; set +a
SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation cargo test --lib cloud::integration::upload::backblaze_b2_upload_batch -- --ignored --exact --nocapture
```

```powershell
# Native Windows 11, in a copy of the tree (src-tauri\), credentials from the DPAPI file
$c = Import-Clixml $env:USERPROFILE\.sotyware-it\b2.xml
$env:SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $c.UserName
$env:SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $c.GetNetworkCredential().Password
$env:SOTYWARE_IT_B2_BUCKET = Get-Content $env:USERPROFILE\.sotyware-it\b2-bucket.txt
$env:SOTYWARE_IT_B2_REGION = 'eu-central-003'
$env:SOTYWARE_IT_B2_UPLOAD_CONFIRM = 'upload-validation'
cargo test --lib cloud::integration::upload::backblaze_b2_upload_batch -- --ignored --exact --nocapture
```

**How accidental runs are prevented.** There are three layers, and all
three must be passed:

- `#[ignore]`, so `cargo test`, `npm run test:headless` and `ci.yml` never
  run it. No npm script, workflow or desktop driver passes `--ignored` with
  a filter that matches it.
- The account variables, which CI does not have: no workflow sets a
  `SOTYWARE_*` variable or reads a secret.
- The confirmation variable. Without it the test fails with "nothing was
  sent" before building a client, so a broad `cargo test -- --ignored` with
  `b2.env` loaded for another test does not write.

The older B2 tests (`backblaze_b2_round_trip`, `backblaze_b2_download_file`,
`backblaze_b2_browse_dataset`, …) have only the first two layers.

**If the process crashes or cleanup fails.** The run id is printed before
the first write. What can be left: at most three versions under the run
prefix, and incomplete multipart uploads of `multipart.bin` or
`cancelled.bin` with their parts. Delete markers are impossible, since no
request without a version id can be sent. Incomplete uploads are not shown
by version listings, so the cleanup lists them separately. To remove what a
run left:

```bash
SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation SOTYWARE_IT_B2_UPLOAD_RUN=<run id> cargo test --lib cloud::integration::upload::backblaze_b2_upload_cleanup -- --ignored --exact --nocapture
```

This runs the same scoped `cleanup_run` and fails unless the run prefix is
empty afterwards. A failed delete or abort prints
`CLEANUP FAILED: … <key> … <id>`; nothing is retried. If the daily cap
refuses the cleanup, run it again the next day. Nothing outside the run's
four keys is ever deleted or aborted. No bucket setting, lifecycle rule or
key is changed. A bucket lifecycle rule for unfinished large files would be
a safety net, but setting one is a decision for the bucket's owner and not
part of this test. If the run id is lost, find the run under
`sotyware-it/upload-validation/` in the Backblaze web console.

**Where it can run.**

- **WSL2:** yes; passed once (2026-11-13).
- **Native Windows 11:** yes, in a copy of the tree, with the PowerShell
  commands above; passed once (2026-11-13). That run executed the test
  program (`target\debug\deps\sotyware_lib-*.exe`) directly from
  `src-tauri\` after checking its SHA-256 against the one approved, with
  the same arguments. No desktop or dialog is involved.
- **GitHub CI:** not prepared, on purpose. It would need repository secrets
  and a manual, approval-gated workflow, and its log would be public on
  GitHub (the repository is public). It would also count against the B2
  caps.

The real file dialog and the dashboard UI are not part of this test. The
release binary's upload workflow was validated only against the local
server
([UI end-to-end test](#cloud-storage-ui-end-to-end-test)); its Backblaze
counterpart [passed once](#uploads-through-the-windows-dashboard-at-backblaze).

**Known limitations.**

- The run scope assumes path-style requests (B2's profile).
- The run scope checks method, path and the id query parameters only. A
  `PUT` or `POST` on a run key passes whatever headers or subresources it
  carries (`x-amz-copy-source`, `?retention`, `?legal-hold`, `?tagging`,
  `?acl`). Neither the adapter nor the test sends any of these. A retention
  or legal hold on a run key would block its cleanup; Object Lock is off on
  the bucket, so it could not be set anyway.
- Checks after the commit point (`needs-inspection`) and a refused abort
  (`incomplete-parts`) cannot be produced safely at a real provider. They
  are covered only against the in-process server.
- The race in which another client writes the key between the `HeadObject`
  and the PUT is not tested (ADR-021).

`upload_scenario_against_the_test_server` runs steps 3–6 with the same
run scope against the in-process server in every `cargo test`, with
Backblaze's limits, so the plan is three parts. That server has no
versioning and no `ListMultipartUploads`, so steps 2, 7 and 8 are replaced
there by its operation log and its files: three `UploadPart`s, one abort,
no open multipart files, no object for `cancelled.bin`, and one `DELETE` in
all. The cleanup's selection rules (`cleanup_touches_only_this_runs_keys`),
the run scope's rules (`in_scope_allows_only_the_run`), the run prefix and
the confirmation are unit-tested too.

### Uploads through the Windows dashboard at Backblaze

**Passed once against Backblaze (2026-11-13, 38/38, native Windows 11)**
after a first run that failed only on the verify test's own request count
(fixed), see
[status.md](../status/status.md#dashboard-upload-at-backblaze-on-native-windows-11-rerun-passed-2026-11-13).
`tests\desktop\windows-cloud-ui.ps1
-Mode b2-upload` uploads one batch through the **release binary**, its
bundled page and the **native Windows multi-file dialog** into the real B2
test bucket, then checks and removes what arrived. It is the UI
counterpart of [Uploads at Backblaze](#uploads-at-backblaze) (Rust batch
path, no UI) and of the local `-Mode upload` (UI, local server only). The
three are separate results; one does not stand in for another.

**Why a third key.** The browser always opens at the bucket's root, and
Backblaze refuses the root listing to a key limited to a file name prefix,
so the write key (`b2.xml`, prefix `sotyware-it/`) cannot reach the run
folder in the UI, and the read-only key (`b2-read.xml`) cannot upload. The
app therefore uses a third key, `%USERPROFILE%\.sotyware-it\b2-upload.xml`
(`Get-Credential | Export-Clixml`, keyID as user name): Application Keys →
Add a New Application Key, the test bucket only, **Read and Write**, **no**
file name prefix, "list all bucket names" off, a short expiry. Creating it
is the bucket owner's decision. This key could write anywhere in the test
bucket; the safeguards against that are below. Prepare, verification and
cleanup use only the write key, through the batch test's run scope.

**What it does.**

1. Writes the local files into a new `%TEMP%\sotyware-b2-upload-<run id>\`:
   `seed\existing.txt` and `batch\` with `existing.txt` (other content and
   size), `a-small.txt`, `größe +test.bin` (200,003 bytes) and
   `progress.bin` (24 MiB + 5 bytes; one `PutObject` with the release
   binary's 64 MiB multipart threshold). The run id is the script's UTC
   timestamp, chosen before any request.
2. `backblaze_b2_upload_ui_prepare` (`cloud/integration/upload/ui.rs`, write
   key, run scope limited to the run prefix and these four names): checks
   the local layout; lists versions and incomplete multipart uploads of
   `sotyware-it/upload-ui-validation/<run id>/` (both must be empty); uploads
   the seed as `existing.txt`, which makes the run folder visible to the
   browser and is the object the dashboard must not overwrite. Prints the
   seed's version id.
3. The dashboard (account added with `b2-upload.xml` through the form;
   Check connection → Connected) opens the bucket, then `sotyware-it`,
   `upload-ui-validation` and the run folder, which must show exactly the
   seed. Upload files… opens the native dialog; **while it is open the panel
   must name `<bucket> / sotyware-it/upload-ui-validation/<run id>/` as the
   destination, or the dialog is cancelled and nothing is sent**. The batch
   folder is opened through File name + Open, the four files are selected in
   the dialog's file list through UI Automation, Open.
4. Checks the panel: "Upload finished. 4 files: 3 uploaded, 1 skipped.",
   one row per file with its exact name, three "Uploaded and verified",
   `existing.txt` "Skipped: already in this folder … left as it is", no Try
   again button, `progress.bin` seen "Sending · n%" with a percentage that
   never goes down, and no local path in any page or panel text seen (the
   folder, the profile, `%TEMP%`, a drive letter). Show destination folder
   must then list exactly the four objects, `existing.txt` with the seed's
   size. Then rename, key replacement (made-up key, no request) and removal
   of the account, as in every mode.
5. After sotyware is stopped: `backblaze_b2_upload_ui_verify` (write key,
   same scope) downloads each of the four keys once through
   `CloudDownloads` and compares it byte for byte with the local file (the
   seed for `existing.txt`; each must be `sha256-verified`), then runs the
   batch test's cleanup for these four keys and checks what it found: the
   four keys, one version each, no delete marker, `existing.txt` still the
   seed's version id, the batch files' sizes, no incomplete multipart upload;
   nothing may be left. If Open was never pressed,
   `backblaze_b2_upload_ui_cleanup` runs instead (cleanup only).
6. Removes the local folder; the account database and Credential Manager
   are back as before (as in every mode).

The `started: false` of the skipped file is not visible in the page (the
panel's wording does not depend on it). It is covered by the Rust tests
(`upload_ui_run_against_the_test_server` checks it for this very batch,
and the Backblaze batch test checked it at the provider).

**Requests, if nothing is retried.** Write key (recorded by the tests):
prepare 5 (`ListObjectVersions`, `ListMultipartUploads`, `HeadObject`,
`PutObject`, `HeadObject`); verify and cleanup 12 (4 `GetObject`,
`ListMultipartUploads`, `ListObjectVersions`, 4 `DeleteObject` with a
version id, `ListObjectVersions`, `ListMultipartUploads`). App key (not
recorded; the release binary has no recorder): 19, namely `HeadBucket` 1;
`ListObjectsV2` 5 (root, `sotyware-it/`, `upload-ui-validation/`, the run
folder, Show destination folder); per uploaded file the batch's
`HeadObject`, the adapter's `HeadObject`, `PutObject` and the verifying
`HeadObject` (12); one `HeadObject` for the skipped file. **36 in all.** The
app's SDK makes up to 3 attempts per request, so a transient error adds
requests. About 24.3 MiB are uploaded by the app and downloaded by the
verification; it is billed for the minutes until cleanup.

**Safeguards.**

- Refused before anything (exit 2) unless `-ConfirmB2Upload
  upload-ui-validation` is given; `-ExpectSha256 <hex>` stops it unless the
  release binary is the approved one. The Rust tests also need `--ignored`,
  the account variables and `SOTYWARE_IT_B2_UPLOAD_CONFIRM` (set by the
  driver for those child processes only, with `--exact`); `b2-upload.xml`
  must not be the write key.
- The run prefix must be empty before the first write; the run id is set
  before the prepare, so the cleanup runs even if the prepare fails after
  its upload.
- The app may send only after the panel named the run folder (step 3);
  the destination is fixed when the dialog opens. The provider-side checks
  then require exactly the four keys under the run prefix.
- Prepare, verify and cleanup go through the run scope: listings only of
  the run prefix, requests only for its four keys, every `DELETE` with a
  version id or an upload id; nothing is retried. No bucket setting,
  lifecycle rule or key is changed.

**Not covered by this mode.** Cancel, retry, multipart, leaving the
browser and quitting during a batch are kept out of this run so that it
checks one plain batch; they have their own mode, one scenario per run
([below](#upload-lifecycle-through-the-windows-dashboard-at-backblaze)).
Hiding during a batch and the error paths stay with the local
`-Mode upload`.

**Residual risks.** The app key has no prefix, so a defect that sent an
upload elsewhere in the bucket would not be caught by the cleanup's scope
(only by the destination check before Open and the provider-side checks of
the run prefix). The app's own requests are counted only in the rehearsal,
not at Backblaze. If a run is interrupted, remove it with:

```powershell
# in src-tauri\ of the copy, write key variables as for the batch test
$env:SOTYWARE_IT_B2_UPLOAD_CONFIRM = 'upload-validation'
$env:SOTYWARE_IT_B2_UPLOAD_UI_RUN = '<run id>'
cargo test --lib cloud::integration::upload::ui::backblaze_b2_upload_ui_cleanup -- --ignored --exact --nocapture
```

**Rehearsal.** `-Mode b2-upload -Rehearse` runs steps 1, 3, 4 and 6
unchanged against the in-process s3s-fs server (`serve_uploads_for_ui`,
seeded through `SOTYWARE_UI_UPLOAD_SEED_KEY`/`_SEED_FILE` with only the seed
under the run's key; made-up key; 2 MiB/s link; contacts no provider) and
replaces steps 2 and 5 by the server's state: exactly the four keys,
contents and SHA-256 metadata as the local files, the seed unchanged, no
multipart; then exact-key deletion. It also checks the app's requests by
operation (`HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 10, `PutObject`
3), which is how the 19 above are known.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-upload -Rehearse
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-upload -ConfirmB2Upload upload-ui-validation -ExpectSha256 <approved hash>
```

`upload_ui_run_against_the_test_server` (every `cargo test`) runs the
seed, the same batch through the code behind `upload_cloud_files` and the
verification downloads against the in-process server, with the run scope:
13 batch requests, the skipped file `started: false`, 4 `GetObject`. The
local layout check, the run prefix and the scope's four names are
unit-tested.

### Upload lifecycle through the Windows dashboard at Backblaze

**Each scenario passed once at Backblaze (2026-11-13, native Windows 11)**,
see [status.md](../status/status.md#upload-lifecycle-at-backblaze-on-native-windows-11-2026-11-13).
`tests\desktop\windows-cloud-ui.ps1
-Mode b2-lifecycle -Scenario <scenario>` checks one upload lifecycle
behaviour per run through the **release binary with its release settings**
(`ClientOptions::default()`: one `PutObject` up to 64 MiB, above that
multipart in 16 MiB parts), its bundled page and the native Windows
multi-file dialog, against the Backblaze test bucket. The local
`-Mode upload` covers the same behaviours against the s3s-fs server; this
mode is the provider counterpart, one scenario at a time, so each result
stands on its own and a failed one does not hide the others. It replaces
the earlier decision (above) to keep these paths local only.

| Scenario | Local files (`batch\`) | What the dashboard does | Must hold afterwards |
| --- | --- | --- | --- |
| `multipart` | `multipart.bin`, 72 MiB + 3 (5 parts: 4 × 16 MiB, 8 MiB + 3) | uploads it; at 10 % or more `backblaze_b2_upload_lifecycle_probe` lists the run prefix's open multipart uploads (must be exactly one, for this key); "Upload finished. 1 file: 1 uploaded.", "Uploaded and verified", Sending percentages rising, Show destination folder lists it with its size | anchor + `multipart.bin`, downloaded `sha256-verified` and byte-equal |
| `cancel` | `cancel.bin`, 144 MiB (9 parts) | Cancel upload at 20 % or more: "Upload cancelled. 1 file: 0 uploaded, 1 cancelled.", "Cancelled during its upload", "Nothing was stored", no "may remain" | only the anchor; **no incomplete multipart upload** |
| `retry` | `anchor.txt` (other content), `a-first.txt`, `b-locked.txt`, `c-third.txt` | `b-locked.txt` is held open without sharing by the script: "Upload finished. 4 files: 2 uploaded, 1 skipped, 1 failed."; only "Try 1 file again" offered; released, Try again: "1 file: 1 uploaded.", only `b-locked.txt` in the new batch | anchor (its own version: not overwritten) + the three small files, one version each (a completed file sent twice would leave two) |
| `leave` | `a-away.bin`, 60 MiB (one PUT), `b-away.txt` | ← Back to accounts at 10 % or more; 35 s later Open bucket: no panel, Upload files… enabled | only the anchor (the PUT was cut before its last chunk; `b-away.txt` never started) |
| `quit` | `quit.bin`, 144 MiB (9 parts) | tray Quit at 20 % or more (**real mouse**, as `-Mode upload`): exit 0 within 35 s; the panel never said finished or uploaded; relaunch, the bucket opens, no panel | only the anchor; **no incomplete multipart upload** |

Needs-inspection (a refused verification after the commit point) and a
refused abort (`incomplete-parts`) need a provider that misbehaves on
purpose; they stay with the local `-Mode upload` (`verify-denied`,
`abort-denied`).

**Steps of every scenario.** Keys as for `-Mode b2-upload` (the app uses
`b2-upload.xml`, the Rust tests the write key `b2.xml`).

1. Writes the local files into a new
   `%TEMP%\sotyware-b2-lifecycle-<scenario>-<run id>\` (`seed\anchor.txt`
   and `batch\`). The run id is `<scenario>-<UTC timestamp>`, chosen before
   any request; the run prefix is
   `sotyware-it/upload-lifecycle-validation/<scenario>-<timestamp>/`.
2. `backblaze_b2_upload_lifecycle_prepare`
   (`cloud/integration/upload/lifecycle.rs`, write key, run scope limited
   to the run prefix and the scenario's names): checks the local layout,
   including that each file is the single `PutObject` or the multipart
   upload of the stated part count **with the release settings at
   Backblaze's limits**; lists versions and incomplete multipart uploads of
   the run prefix (both must be empty); uploads `anchor.txt` so the browser
   can open the run folder. Prints the anchor's version id.
3. The dashboard (account added with `b2-upload.xml`; Connected) opens
   `sotyware-it`, `upload-lifecycle-validation` and the run folder (exactly
   the anchor). Upload files…; **while the dialog is open the panel must
   name `<bucket> / <run prefix>` as the destination, or the dialog is
   cancelled and nothing is sent**; the batch folder's files are selected in
   the dialog's file list; Open. Then the scenario's steps (table).
4. After sotyware is stopped: `backblaze_b2_upload_lifecycle_verify`
   downloads what the scenario must have stored and compares it, runs the
   cleanup for the scenario's names (aborts incomplete multipart uploads
   **by upload id**, deletes versions **by version id**, lists again) and
   fails if it found anything but the anchor (its own version) and the
   stored files, one version each, or **any incomplete multipart upload**.
   If Open was never pressed, `backblaze_b2_upload_lifecycle_cleanup` runs
   instead.
5. Removes the local folder; account database and Credential Manager as
   before.

**Requests, if nothing is retried.** Write key (recorded by the tests):
prepare 5; probe 1 (`multipart` only); verify and cleanup: one `GetObject`
per stored file, `ListMultipartUploads`, `ListObjectVersions`, one
`DeleteObject` with a version id per version, `ListObjectVersions`,
`ListMultipartUploads`. App key (not recorded at Backblaze; known from the
rehearsal): `HeadBucket` 1; `ListObjectsV2` 4 (root, `sotyware-it/`,
`upload-lifecycle-validation/`, the run folder) plus 1 (Show destination
folder, the root after leaving, or the root after the relaunch); per file
uploaded one `HeadObject` in the batch and one in the adapter before
sending, one after it (a multipart upload one more, just before
`CompleteMultipartUpload`, the commit point).

| Scenario | App | Write key | Uploaded by the app | Downloaded |
| --- | --- | --- | --- | --- |
| `multipart` | 17: `HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 4, `CreateMultipartUpload` 1, `UploadPart` 5, `CompleteMultipartUpload` 1 | 5 + 1 + 7 = 13 | 72 MiB | 72 MiB |
| `cancel` | 9 + k: `HeadBucket` 1, `ListObjectsV2` 4, `HeadObject` 2, `CreateMultipartUpload` 1, `UploadPart` k (k ≈ 2–3; some may be cut off), `AbortMultipartUpload` 1 | 5 + 5 = 10 | about 20–50 MiB (cut off) | — |
| `retry` | 19: `HeadBucket` 1, `ListObjectsV2` 4, `HeadObject` 11 (the existing `anchor.txt` 1, the locked file 1 in the first batch, 3 per uploaded file), `PutObject` 3 | 5 + 11 = 16 | < 1 KiB | < 1 KiB |
| `leave` | 8–9: `HeadBucket` 1, `ListObjectsV2` 5, `HeadObject` 2, `PutObject` ≤ 1 (body cut off) | 5 + 5 = 10 | about 6–60 MiB (cut off) | — |
| `quit` | 10 + k, as `cancel` with `ListObjectsV2` 5 | 5 + 5 = 10 | about 30–60 MiB (cut off) | — |

The app's SDK makes up to 3 attempts per request, so a transient error adds
requests. Listings are Backblaze class C transactions, `HeadObject` and
`GetObject` class B, uploads and deletes class A (free); all five
scenarios together stay far below the daily free caps (2,500 B and C
each), but a cap reached by other tests that day would refuse the
cleanup's listings (seen once, 2026-11-11); the cleanup is then run again
the next day with the command below.

**Safeguards.** As for `-Mode b2-upload`: refused before anything (exit 2)
unless `-ConfirmB2Lifecycle lifecycle-ui-validation` and `-Scenario` are
given (`-ExpectSha256` pins the approved binary); the Rust tests need
`--ignored`, the account variables and `SOTYWARE_IT_B2_UPLOAD_CONFIRM`; the
run prefix must be empty before the first write; the app sends only after
the panel named the run folder; every write-key request goes through the
run scope (listings only of the run prefix, requests only for the
scenario's names, every `DELETE` with a version id or upload id; nothing
retried). No bucket setting, lifecycle rule or key is changed. `quit` moves
the mouse once and needs the interactive desktop (exit 3 otherwise).

**Residual risks.** As for `-Mode b2-upload` (the app key has no prefix;
the app's own requests are not recorded at Backblaze, only the outcome is
checked by listing). In addition: a cut-off `UploadPart` that Backblaze
completes after the abort could leave a part; the verify fails then and its
cleanup aborts the upload by id. If the link is so fast that the file
completes before the cancel, leave or quit (at 20 % of 144 MiB, a few
seconds at 100 Mbit/s), the scenario fails as inconclusive and the cleanup
removes the stored object. If a run is interrupted:

```powershell
# in src-tauri\ of the copy, write key variables as for the batch test
$env:SOTYWARE_IT_B2_UPLOAD_CONFIRM = 'upload-validation'
$env:SOTYWARE_IT_B2_LIFECYCLE_SCENARIO = '<scenario>'
$env:SOTYWARE_IT_B2_LIFECYCLE_RUN = '<scenario>-<timestamp>'
cargo test --lib cloud::integration::upload::lifecycle::backblaze_b2_upload_lifecycle_cleanup -- --ignored --exact --nocapture
```

**Rehearsal.** `-Mode b2-lifecycle -Rehearse -Scenario <scenario>` runs
steps 1, 3 and 5 unchanged against the in-process s3s-fs server
(`serve_uploads_for_ui`, seeded with only the anchor under the run's key,
2 MiB/s link, made-up key; contacts no provider), uses the server's open
multipart files for the probe and its state instead of step 4 (exactly the
anchor and the stored files, contents and SHA-256 metadata, no multipart
left; then exact-key deletion), and checks the app's requests by operation
(the "App" column above).

**Scenario `settle`** (rehearsal only; refused without `-Rehearse
-RecordRequests`, so it ends with the tray Quit and moves the mouse once):
checks the wait of
[ADR-025](../architecture/decisions.md#adr-025-a-cancelled-part-that-was-handed-over-completely-is-awaited-before-the-abort)
through the dashboard. `serve_uploads_for_ui` runs with
`SOTYWARE_UI_UPLOAD_PACE=buffered` (`Pace::Buffered`, 6 MiB/s): the link
takes every request body at once, so each 16 MiB part of `settle.bin`
(144 MiB, 9 parts) is handed over completely and then forwarded for about
2.7 s. The page's percentage therefore moves once per part; the driver
presses Cancel upload right after it moves (20% or more) and stamps the
press. From the recording (absolute times: `started_unix_ms` + `t_us`) it
checks that exactly one part was in flight at the press, that it ended with
`op-end` 200 at least 500 ms after the press (not `op-dropped`), that no
part was dropped or started after the press, and that the one
`AbortMultipartUpload` (204) started after that answer, within 1 s. A wait
that runs into the 5 s bound shows as `op-dropped` about 5 s after the
press and fails the check. The percentage only times the press; every
settle check reads the recording. The recording has no event for "body
handed over completely": that the cancelled part was handed over follows
from the buffering link and the percentage, so these checks alone would
not tell the wait apart from a cancel that is ignored until the part ends
(the `cancel` and `quit` scenarios and the unit tests show a part not
handed over being dropped at once). The driver does not keep the app's
stderr, so sotyware's line for a wait that reached the bound is not seen.

The throttled links pace by deadline in `Buffered` mode only:
`BytesPerSecond` still sleeps once per 16 KiB piece, and Windows timers fire
only every ~15.6 ms, so on Windows that link carries about 1 MiB/s whatever
`SOTYWARE_UI_UPLOAD_RATE` says (a 16 MiB part takes about 16 s at the
nominal 2 MiB/s); every Windows `upload` and lifecycle run so far ran
at that speed.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-lifecycle -Rehearse -Scenario multipart
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-lifecycle -Rehearse -RecordRequests -Scenario settle -Exe src-tauri\target\recorder\release\sotyware.exe
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-lifecycle -Scenario multipart -ConfirmB2Lifecycle lifecycle-ui-validation -ExpectSha256 <approved hash>
```

The layout checks against the release settings, the run prefix, the
scope and the final-state check are unit-tested in `lifecycle.rs`.

## File management tests

ADR-026 ([cloud-storage.md](../architecture/cloud-storage.md#file-management)).
All headless (part of `npm run test:headless`); **no real provider, no
desktop**.

- **The versioned test backend** (`cloud/test_server/versioned.rs`,
  `TestServer::start_versioned`): an in-memory s3s backend with S3's
  versioning semantics (delete markers on versioned buckets, `null`
  versions otherwise, `ListObjectVersions` with markers and pages,
  `GetBucketVersioning`, `CopyObject` with metadata directive and
  `copy-source-if-match`, ranged `GetObject`), exact keys, and the same
  fault injection as the other servers (`inject`: 500, 403, no answer, a
  callback). The returned handle reads and changes stored versions, as a
  second client would. It models S3's documented behaviour; it does not
  prove Backblaze's or Hetzner's. Limitation found: s3s decodes
  `x-amz-copy-source` before splitting off `?versionId`, so it cannot copy a
  key containing `?` (S3 decodes after splitting; sotyware encodes `?` as
  `%3F` either way). The copy tests therefore use `#%+&` but not `?`.
- `cloud/operations/tests.rs` (19): metadata and content type kept by a
  copy; conflicts shown, skipped or renamed, never overwritten; copying
  into the same folder; move on versioned (delete marker) and unversioned
  buckets; rename and refusal of an existing or invalid name; folder rename
  with markers, `//`, `..`, `#%+&` and neighbours (`photos-backup/`,
  `photosx`) untouched; into-itself and in-place moves refused; folder
  delete counts (hidden vs removed); a source changed after review is not
  copied and no original is deleted; a failed delete of a move keeps the
  original and retry finishes it without copying again; a copy stopped by a
  5xx retried without duplicates; cancel before the next item; a file
  changed after review is not deleted; restore removes exactly the delete
  markers; 2,150 objects across pages; plans expire when used, replaced or
  discarded; invalid requests send nothing; shutdown cancels and refuses;
  event shapes. Pure rules: `cloud/operations/names.rs` (7).
- `cloud/manage/tests.rs` (8): folder creation (and refusal of an existing
  one or an invalid name); properties; previews (size limits, SVG shown as
  text, binary unsupported, truncated text); base64 and text detection;
  name and prefix search across 10,501 objects with a one-time cursor;
  deleted items (restorable or not, neighbour folder not included); share
  links (signature, expiry, no secret, downloads the file without
  credentials, invalid lifetime refused).
- `cloud/bulk_download/tests.rs` (6): safe local names; structure and
  markers kept; an existing name never replaced or merged (`name (n)`); a
  failed file leaves nothing of its item and no staging folder, retry
  finishes; cancel during a stalled file removes the current item and keeps
  saved ones; a cancelled dialog or a missing file starts nothing; no local
  path in any event.
- `cloud/upload/tests.rs` (5 new): folder walk with relative keys and
  without following links; a non-Unicode name skips only that file (Unix);
  folder upload keeps its structure and never overwrites; a retried folder
  upload keeps relative keys; an empty folder starts nothing.
- `cloud/places.rs` (2), `cloud/drop.rs` (2), the CSP pin in `lib.rs`.
- Frontend: `src/fileList.test.ts` (13: sorting incl. bucket order,
  categories, filters, selection modes, focus moves, shortcuts and typing
  fields) and `src/CloudFileManager.test.ts` (25: selection by click,
  Ctrl, Shift, checkboxes, Ctrl+A, Escape; shortcuts left to an open
  dialog; delete review with counts and the
  versioning note, events, stale events ignored, refresh; Cancel discards
  the plan; partial failure and Try again; picker, conflicts and "Keep
  both"; folder creation in the picker; rename with F2; plan errors; new
  folder; search with Clear; sorting and filters; deleted items and
  restore; versions not listable; properties with text never rendered as
  markup; SVG refused as image; allowed image; share link; several-item
  download; single file keeps the save dialog; context menu and favorites;
  folder upload; drop banner and unwatch on leaving).

Commands: `cargo test --lib operations`, `cargo test --lib cloud::manage`,
`cargo test --lib bulk_download`, `npx vitest run src/CloudFileManager.test.ts`.

## Request recorder tests

The request recorder (ADR-024,
[cloud-storage.md](../architecture/cloud-storage.md#request-recorder)) is
compiled only with the Cargo feature `request-recorder`, and only into a
target directory named `recorder` (build.rs refuses any other). None of
this contacts a provider.

```bash
cd src-tauri
cargo test                                                        # default build: no recorder; includes the SDK hook assumptions
cargo test --features request-recorder --target-dir target/recorder
SOTYWARE_RECORDER_TEST_ALL=1 cargo test --features request-recorder --target-dir target/recorder   # every store recorded
cargo clippy --all-targets --features request-recorder --target-dir target/recorder -- -D warnings
cd .. && npm run check:release-binary && npm run test:release-binary-guard
```

- `cloud/s3/hook_assumptions.rs` (every build): what the recorder relies on
  in the locked SDK, with a probe interceptor against the in-process
  server: per-operation state in the `ConfigBag` is dropped with a dropped
  future (and only then without `read_after_execution`); client
  interceptors run for uploads with their per-operation config override;
  hook order for success, a refused request, a retry and a drop; the typed
  input at `read_before_execution` (the operation `Metadata` is not there
  yet); status and attempt numbers without credentials or bodies.
- `cloud/recorder/tests.rs` (feature build): settings (none → off; partial
  or invalid → refused, nothing created), the file (fresh, 0600, an
  existing file or a symbolic link refused and untouched, a missing or
  read-only directory), no interceptor without a recorder, the connection
  check, browsing, a single PUT, a multipart upload and a download compared
  with the server's own request log, a 500-then-200 retry (one operation,
  attempts 1 and 2), a refused request (error code, no message), an
  unreachable endpoint, cancelling during a part (the part `op-dropped`,
  the abort its own operation), the stall timeout and the abort timeout
  (`op-dropped`), redaction (no key id, secret, bucket name, raw key,
  body, signature or endpoint; harness-computable hashes; out-of-scope and
  ancestor prefixes), 128 concurrent operations on two accounts (whole
  lines, unique `seq` and `op`), a full queue (gaps exactly match the
  dropped events), a writer failing after 1,500 bytes, a dead writer, a
  finish that waits only its limit, and hooks that never fail a request.
  `cloud/upload/tests.rs` adds quitting during a multipart upload in the
  order `RunEvent::Exit` uses.
- `SOTYWARE_RECORDER_TEST_ALL=1` (test builds only) records every store of
  the test process into a discarding sink, so the whole existing suite runs
  with the interceptor installed and must pass unchanged.
- ADR-025 (`cloud/s3/upload/tests.rs`, every build, about 8 s): behind a
  link that buffers the request body and forwards it slowly
  (`Pace::Buffered` in `test_server.rs`), a cancel after part 2 was handed
  over completely waits for its answer and the abort leaves no part; with
  `part_settle_timeout` zero (the control) the late part remains; the
  wait ends at its bound. The cancel comes from the test hook
  `TransferControl::handed_over`, the moment part 2's last chunk is
  handed over (a progress update is throttled and, on a busy machine, can
  come after part 2 was answered), and the server's multipart files are
  listed when the abort arrives: part 2 is stored by then with the wait,
  not without it, so neither test depends on the machine's speed
  ([status](../status/status.md#the-late-part-wait-test-on-a-busy-runner-2026-11-15)).
  After its client went away, the `Buffered` link
  does not half-close towards the server (it waits up to a minute for the
  answer, then closes): hyper's server takes that EOF as a closed
  connection and drops a handler still storing the part, which on a
  loaded machine (CI) made the control fail.
- `probe_a_part_landing_after_the_abort` (`cloud/s3/upload/tests.rs`,
  `#[ignore]`, prints its findings, needs no feature): cancels a
  multipart upload right after part 2 was handed to the connection
  behind a 1 MiB/s link and reports whether a part of the aborted upload
  remains on the server
  ([limitations.md](../status/limitations.md#a-part-can-land-after-the-multipart-abort)).
  `cargo test --lib part_landing_after_the_abort -- --ignored --nocapture`.
- `scripts/check-release-binary.test.mjs`: the manifest check against
  broken copies, the binary scan, and the before-bundle check with made-up
  command lines and target directories.

**Recorder build and the desktop drivers.** Build it (never packaged; the
packaging guard refuses a bundle):

```bash
npm run tauri build -- --no-bundle --features request-recorder -- --target-dir target/recorder
```

Then the Linux driver records in the `download` and `upload` phases:

```bash
SOTYWARE_CLOUD_UI_RECORD=1 SOTYWARE_CLOUD_UI_BIN=src-tauri/target/recorder/release/sotyware tests/desktop/cloud-ui.sh download upload
```

and the Windows driver with `-RecordRequests` (modes `download`, `upload`,
`b2-upload`, `b2-lifecycle`; `-Exe` must be the recorder build). Against
the local server, the helper (`check_recordings` in `test_server.rs`)
fails the phase unless every recording is complete, contains none of the
keys, and agrees with the requests the server saw signed with the app's
key: every attempt that got an answer is among them, and the server saw
nothing the recorder did not see sent. The helper's own requests (seeding,
`dump`, `delete`) are signed with a second key and never count. A
recording that is incomplete fails the run as "count unknown", never as
"no requests".

## Title bar, placement, usage and snapshot tests (ADR-040 to ADR-042)

Headless, in `npm run test:headless`; nothing below needs a display or a
provider:

```bash
cd src-tauri && cargo test --lib window_state:: title_bar:: cloud::usage cloud::pricing backup::
npm run test:frontend -- src/TitleBar.test.ts src/CloudUsage.test.ts src/CloudSnapshots.test.ts src/accountSync.test.ts
```

- `window_state.rs`: the default placement (half of a 1920 × 1080 work area
  is 960 × 540, centred; taskbars on any side; the 900 × 540 first-launch
  minimum on 1366 and 1280 px screens and in logical pixels at 150 %;
  screens smaller than it; decorations; negative coordinates), restoring (unchanged, maximized, on an unplugged
  monitor → default, partly visible → moved inside, too large → shrunk,
  the monitor overlapped most, a changed scaling keeps the logical size),
  the saved row's strict format, writes only on change, an unusable store.
  `storage.rs`: `window-state.sqlite3` and `storage-usage.sqlite3` (newest
  first, trimmed to 100 per account, unreadable rows skipped).
- `title_bar.rs`: Windows only, exactly the controls' permissions, its own
  capability name. `lib.rs`: the window is created hidden with native
  decorations; the capability file's permissions (incl. `allow-is-decorated`).
- `cloud/usage/tests.rs` against the in-process servers: versions, delete
  markers and small objects counted apart; 2,345 objects over three pages,
  no page read twice; an empty bucket; s3s-fs (no version listing) falling
  back to current objects while an unfinished upload's part is measured;
  cancellation; wrong key and missing bucket in sotyware's words; the slot,
  events and the kept result; the overview's grouping (Backblaze per region,
  Hetzner per provider, generic without estimate), downloads per group,
  per-currency totals, history order, the page's JSON shape.
- `cloud/pricing.rs`: both price lists valid and pinned to the figures read
  on 2026-11-15; Backblaze's free 10 GB and 3× egress; Hetzner's base price,
  per-TB-hour storage, egress and the 64 kB minimum; assumptions and
  sources stated; a broken list refused.
- `backup/tests.rs`: deleting a chosen snapshot removes its manifest at once
  and its unshared blob only after the margin, keeps shared blobs, refuses
  a foreign key; health rests on the previous snapshot after the newest was
  deleted, and on none after the last; a deletion waits for the slot.
- Vitest: `TitleBar.test.ts` (controls, maximize state followed after a
  resize, drag region, close wording per mode, errors shown, narrow toggle,
  inactive look) and `App.test.ts` (drawn only for an undecorated window,
  name moved, close request); `CloudUsage.test.ts` (empty state, measuring
  with progress and stale events dropped, tiles and parts, the estimate as an
  estimate, downloads sent and remembered, the Hetzner meter, history and
  its table, failures and cancel, busy); `CloudSnapshots.test.ts` (provider
  snapshots explained, jobs and snapshots, details, Back up now, typed
  deletion and its warnings, refusal, no job); `CloudBrowser.test.ts` (the
  tabs); `accountSync.test.ts` and `App.test.ts` (when an enrolled device
  syncs, the sidebar's sync state).

### Manual check: title bar and window placement (Windows 11)

Not automated beyond `windows.ps1`. On Windows 11, with a test-profile build
in a fresh test profile (or an installed copy that was never started):

1. **First start:** the dashboard opens centred on the primary monitor at
   half its work area, at least 900 × 540 (on 1920 × 1080 with a 48 px
   taskbar: 960 × 540; on 1366 × 768: 900 × 540),
   with no system title bar: sotyware's own bar continues the sidebar's
   colour on the left and the page's on the right, with no line between bar
   and page; rounded corners and a shadow as other Windows 11 windows.
2. **Controls:** hover shows the light fill, Close turns red; Minimize goes
   to the taskbar; Maximize fills the work area (taskbar visible) and the
   middle icon becomes Restore; Restore returns to the same place; Close
   hides to the tray (tray mode), Open Dashboard brings it back.
3. **Dragging:** dragging the bar's empty area moves the window; dragging to
   the top edge maximizes, to a side snaps; double click maximizes and
   restores; buttons never start a drag; Win+Z opens snap layouts;
   Win+Arrow, Alt+Space and Alt+F4 behave as for any window.
4. **Resizing:** every edge and corner resizes; the minimum is 640 × 480;
   when maximized there is no resize border.
5. **Themes and focus:** Settings › Appearance Light and Dark recolour the
   bar; another window active dims its text.
6. **Placement:** move and resize, Quit from the tray, start again: the same
   place and size; maximize, Quit, start: maximized, and Restore gives the
   earlier normal bounds; disconnect the monitor the window was on (or
   lower the resolution) and start: it opens on a connected monitor, fully
   visible.
7. **Scaling:** at 150 % (Settings › Display) the bar is crisp, its glyphs
   sharp, and a saved size keeps its apparent size after a scaling change.
8. **Narrow window** (below 760 px): the navigation toggle and the page's
   name appear in the bar; the sidebar overlay opens below the bar.

Record the result with the build, Windows version and scaling in
[status.md](../status/status.md).

## Folder sync

Folder sync (ADR-043, [folder-sync.md](../architecture/folder-sync.md)):
headless, in `npm run test:headless`; nothing needs a display or a
provider:

```bash
cd src-tauri && cargo test --lib -- folder_sync replacement
npm run test:frontend -- src/CloudSync.test.ts src/App.test.ts src/CloudBrowser.test.ts
```

On WSL2 the same tests run on a file system that ignores case when their
temporary folders are on `/mnt/c` (NTFS through drvfs; there the case probe
falls back to folder listings, as Linux's `realpath` keeps the spelling it
is given). The running engine's watcher test fails there: drvfs delivers no
inotify events.

```bash
mkdir -p /mnt/c/dev/sotyware-casetest
cd src-tauri && TMPDIR=/mnt/c/dev/sotyware-casetest cargo test --lib -- folder_sync
```

- `folder_sync/model.rs`: every row of the decision table (new, unchanged,
  changed on one side, on both, deleted, a change against a deletion,
  folders, a file against a folder), keys ↔ relative paths (prefixes, own
  folder, empty segments), Windows and Unix name rules on every system,
  long keys, exclusions, the deletion guard's thresholds, retry and offline
  delays, conflict copy names, UTC dates and trash ages, intervals, the
  case fold.
- `folder_sync/scan.rs`: the walk (own folder and system files left out),
  links never followed, an unreadable folder reported as unknown (Unix),
  hashing with its stamp, the marker and the folder states, the trash
  (never replacing, old days removed), inspecting a chosen folder, exact
  spellings (the canonical path and the folder listings agree on this file
  system; a path through a link to a folder is not spelled as stored), the
  case probe (it agrees with the file system and leaves nothing; on
  Windows it trusts the canonical path) and nothing looked at through a
  link (Unix).
- `storage/folder_sync.rs`: the schema on disk and the TEMP tables per
  session, only differing paths as candidates (paged), an unfinished
  listing never dropping entries, a folder marker never cleared by an
  implied folder, a full scan keeping what is below an unreadable folder,
  subtrees by range (never by `LIKE`), move sources, removing one sync
  only, bounded activity, counts, spellings that would share a file where
  case is ignored (the spelling here wins, else the synced one, else the
  first in byte order; a folder's contents listed once; two spellings both
  here both kept).
- `folder_sync/migrate.rs`: a verified copy (content and time, never over a
  file); `watch.rs`: relative paths.
- `folder_sync/tests.rs`, end to end with real folders against the
  in-process versioned S3 server (s3s-fs for multipart), passes driven by
  hand: choosing (checked, marked, one per bucket; refused: a whole drive,
  sotyware's data folder, a missing folder, folders inside or around
  another sync's, a read-only folder on Unix, an unknown interval); an
  empty bucket receiving a folder (folder markers only for empty folders;
  a second pass only lists); a bucket coming down (Unicode, implied and
  marker folders, keys skipped); a folder with files merged (equal content
  recorded without a transfer, different content kept twice); changes both
  ways (a touched file recorded, not sent); a watched path looked at alone;
  renames as server-side copies and local renames (nothing sent); deletions
  both ways (delete markers, the sync trash, no version removed); a change
  winning over a deletion; conflicts kept and settled three ways; the
  deletion guard holding an emptied listing (restore, then an allowed
  deletion); a failed listing never deleting (offline, refused); a failed
  transfer retried after its time or on "Try again now"; a disconnected
  drive and an empty folder in its place (nothing deleted, sync goes on
  when it is back); a full drive holding downloads while uploads go on; an
  unreadable folder never taken as deleted (Unix); links, system files and
  unsupported names skipped; a changed account bucket stopping the sync; a
  12 MiB file uploaded in three parts and downloaded whole on a second
  folder with its time; a restart during transfers (a fresh service on the
  same database) without duplicates and with the staging folder cleared;
  an inconsistent synced state repaired without a transfer; pausing,
  turning off (files and objects kept) and the running engine with a real
  watcher and a nudge; moving a folder (one drive: nothing sent again;
  another drive: `/dev/shm` where it is another file system, else copying
  forced: hashed, not sent); starting fresh; re-linking a folder that lost
  its marker; removing the account; names differing only in case on the
  file system as it is (where it ignores case: the Windows and macOS
  runners, `/mnt/c` on WSL2) and with case ignored by force on any file
  system (two keys of one name come down once, a folder spelled otherwise
  here keeps its spelling, renaming only the case is a server-side copy,
  deleting the file here deletes its own spelling and frees the name);
  links (Unix): keys under a link's name never written through it, and a
  synced folder replaced by a link left alone on both sides, the sync in
  step each time.
- `cloud/s3/upload/tests.rs`: a replacement only over the expected ETag
  (another ETag or none refused with only a `HeadObject`; the time stored);
  refused where `HeadObject` sends no ETag (s3s-fs).
- Frontend: `CloudSync.test.ts` (no sync: choose, cancel, the facts shown
  before anything starts, a problem blocking; with a sync: figures, live
  updates and transfers, pause/resume/now/open by id only, a refusal shown,
  the guard's two answers, conflicts settled, activity, problems with
  retry, not synced, interval, turning off after a question, moving only
  into an empty folder; the parsers); `App.test.ts` (the sidebar's sync
  glyph and summary, live updates, late overviews ignored);
  `CloudBrowser.test.ts` (the fourth tab mounted only when chosen).

On a desktop, the release binary's Sync tab is driven by the `sync` phase
of the [Cloud Storage UI end-to-end test](#cloud-storage-ui-end-to-end-test)
(Linux `cloud-ui.sh sync`, Windows `windows-cloud-ui.ps1 -Mode sync`),
against a local server only. Not run: macOS, Windows junctions and file
locking, Explorer, APFS case rules, a real provider.

## Autoclicker tests

The autoclicker and macros (ADR-050, [automation.md](../architecture/automation.md#testing)).
Everything below except the three real-input tests runs in
`npm run test:headless` and in CI, and never sends or records real input:
the engines run on a mock backend and a fake clock.

```bash
cd src-tauri && cargo test --lib automation
npm run test:frontend -- src/Autoclicker.test.ts src/automationFormat.test.ts src/App.test.ts src/navigation.test.ts
```

- `automation/clicker.rs`, `recorder.rs`, `player.rs`: the engines with
  exact timing (`FakeClock` moves time only when an engine waits, and
  pauses, resumes or stops a session at given times): interval and count
  accuracy, endless runs and stops, pause and resume, random intervals,
  double clicks, batches, positions and areas, the fail-safe corner,
  sotyware's own window, time limits; the recorder's timing, sampling,
  shortcut stripping, filters and pauses; playback timing, speed,
  repetition, sections, jitter, placement (pointer, window, changed
  monitors, off-screen points), target programs, pause points, released
  keys and buttons.
- `automation/model.rs`, `keys.rs`, `timing.rs`, `input.rs`: validation,
  the JSON shapes (strict; no unknown action), the key table, shortcut
  rules, conflicts, monitor mapping, fail-safe corners, the real clock's
  wake-up on stop and pause, held-input release.
- `automation/tests.rs`: the service with an in-memory store and the mock
  backend (registration, conflicts, shortcut presses, sessions,
  confirmations, recording to a macro, playback, library, profiles,
  persistence, import/export, broken stores, the feature's stop, the
  status shape); `storage/automation.rs` on real files.
- `automation/windows.rs` (Windows only): coordinate normalisation and the
  keyboard inputs built for `SendInput` (no input is sent).

**X11, real input on a private display** (Linux, opt-in; needs `Xvfb`,
libXtst and libXrandr). Starts its own Xvfb on an abstract socket (as
`virtual.sh` does, since `/tmp/.X11-unix` is read-only under WSLg),
creates a window there and sends, records and grabs only on that display:

```bash
cd src-tauri && cargo test --lib automation::x11 -- --ignored --test-threads=1
```

**The app, real input on a private display** (Linux, opt-in): the
`autoclicker` phase of the [Linux desktop driver](#cloud-storage-ui-end-to-end-test)
drives the test-profile build's Autoclicker page through AT-SPI and sends
real input through XTest, but only on the script's own Xvfb display, into
a window of the driver that records what it receives (each event with the
X server's time). Besides the first macro (the table below), it does what
the manual check of the page used to ask of a person: the right button and
double clicks at 5 per second (timed by the X server), a shortcut changed
by pressing it in its field and the keys the field refuses, the
confirmation without an emergency stop, repetitions at 2× (timed), F7 and
F8 during playback, Shift+F11, the editor (delay, add, move, duplicate,
delete, Preview sends nothing, leaving asks, Save, Revert, the changes
played), a restart, a second profile, a macro's own shortcut, export and
import through the GTK dialogs, keys a macro holds released by F8, Stop
and quitting, 100 clicks per second with the page in use (rate, tab
switches, CPU), the emergency stop's key refused while the feature is off,
and the page's message in a Wayland session. About 6 minutes:

```bash
npm run build:test-profile && tests/desktop/cloud-ui.sh autoclicker
```

**Windows, the page with real input on the real desktop** (opt-in,
**moves the pointer, clicks and presses keys**; never in CI): the same
checks as the Linux phase through UI Automation and `SendInput` (keys by
scan code, so any layout works), into a topmost window of the script that
records every button and key with its time; before each run it checks
that this window is under every point sotyware will click (else it stops
without starting anything), and at the end it puts the pointer back and
lets go of anything still down. It quits sotyware three times with the
tray icon's Quit (a real right click). It refuses to start without
`-ConfirmRealInput pointer-and-keys`, needs the unlocked interactive
desktop (exit 3 otherwise) and `node` on PATH (the database is read with
`node:sqlite`). Run it on an idle desktop; about 8 minutes:

```powershell
npm run build:test-profile
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-autoclicker.ps1 -ConfirmRealInput pointer-and-keys
```

**Windows, the backend with real input** (opt-in, **moves the pointer
and clicks**; run it only on a desktop nobody is using, never in CI). It
clicks only into a topmost window it creates, checks that window is under
each point first (else it fails without sending), sends keys only while
that window has the keyboard (else it skips those checks and says so) and
puts the pointer back, also when a check fails. The clicker and playback
run through a wrapper (`Confined`) that lets them send to that window
only, or nothing at all where the test checks that they skip sotyware's
own window (the test window belongs to the test process, so it is
sotyware's own to them). Keys are counted by scan code, so any keyboard
layout works. About 2 seconds:

```powershell
cd src-tauri; cargo test --lib automation::windows::desktop -- --ignored --test-threads=1
```

The frontend tests mount the page with Tauri's IPC mocks (a fake channel
for `watch_automation`). The page was also looked at in Chromium, the
engine of WebView2, through a throwaway Vite harness (the shell with IPC
mocks answering sample data, headless Edge screenshots; not kept in the
repository); that is not the app in WebView2.

### The page's former manual check, automated

The twelve steps a person was asked to do on Windows (2026-11-18) are
checks of the two drivers above: off by default and Turn on (1), the
button, double clicks and speed (2), shortcuts from another program and
the keys a field refuses (3), the emergency stop, the corners, sotyware's
own window and the confirmation (4), recording (5), playback with
repetitions, speed, F7, F8 and Shift+F11 (6), the editor (7), a restart
(8), profiles, a macro's shortcut, export and import (9), held keys
released by F8, Stop and quitting (10), responsiveness at 100 clicks per
second (11), and turning the feature off (12). Automating them found two
defects, both fixed ([status](../status/status.md#the-autoclickers-manual-check-automated-2026-11-18)).

Left to a person, because no driver can do it here: input to a program
running as administrator (Windows' UIPI; it needs an elevated program,
and the result is Windows' rule, not sotyware's), how smooth the page looks
while it clicks (the drivers measure tab switches and CPU, not frames),
and other environments (real GNOME or KDE desktops on Xorg, Windows 10,
other monitor arrangements). The Windows driver needs the maintainer's
go-ahead for each run, as it moves the pointer on their desktop.

## Update tests

```bash
cd src-tauri && cargo test --lib updates::
npm run test:update-manifest
npm run test:release-assemble
```

`src-tauri/src/updates/tests.rs` (ADR-033) signs manifests and stand-in
installers with minisign keys generated in the test (`minisign` dev
dependency), serves them from an axum server on a loopback port and drives
the real `tauri-plugin-updater` on Tauri's mock runtime (`tauri` feature
`test`, dev only). It never installs: on Linux the plugin would replace the
running test binary, on Windows start an installer. It covers the manifest
checks (signature, format, channel, version floor, platform entry, HTTPS),
the stored floor and postponing, the plugin's download with every way a
package can differ from the signed manifest, cancelling a stalled download,
and a manifest and installers signed once by the real `tauri signer sign
--app-version` (`scripts/fixtures/update/`, throwaway key, public half
only). For GitHub Releases (ADR-036) it adds a local server that answers
like GitHub (`releases/latest/download/<asset>` → the tag's URL → an asset
store), a release published between the manifest and signature requests
(fetched once more, then refused), redirect loops and a redirect to plain
HTTP (refused), a missing release (HTTP 404 named), every failure's kind,
the restart rule, the key file's parsing and that `MANIFEST_URL` is the
repository's, and the real-CLI fixture `scripts/fixtures/update-github/`
(a `latest.json` for all seven platforms made with `release.yml`'s own
commands; every platform's entry, signature and SHA-256 verified).
`scripts/update-manifest.test.mjs` checks the release tool against both
fixtures and keys made with Node's crypto: every platform kind, the GitHub
asset-name rule, `--require` (exactly the platforms built), `--base-url`
(URLs under the tag's folder), `check-signature`, `check-key` and that the
repository in `updates.rs` is the one publishing (`release-repository
--expect`). `scripts/release-assemble.test.mjs` runs
`scripts/release-assemble.sh` (the signing and checking step of
`release.yml`'s publish and rehearse jobs) under bash with the real Tauri
CLI and throwaway keys made in the test: Windows and Linux only while the
Mac builds are not both signed and notarized, all seven platforms when
they are, a missing package, a wrong public key, no private key and a
non-empty output folder refused, the key and its password never in the
output (Linux and macOS; skipped on Windows). `src/Updates.test.ts`
covers the panel (error kinds, restart, per-platform wording) and
`src/App.test.ts` the notification, with mocked commands and channels.

Not covered by any test: installing an update on any platform, the Windows
installer or `request_restart` starting the new version, data surviving an
update (needs two signed releases;
[release.md](release.md#testing-updates-locally)). `release.yml`'s
publish job has run once, for `v0.1.0`
([status](../status/status.md#first-release-v010-2026-11-15)).

## Cloud Storage UI end-to-end test

Opt-in, never in CI: the release binary's Cloud Storage page driven like a

**Safety stop (2026-11-14):** `windows-cloud-ui.ps1` presses buttons by
name (the first Edit, Remove, Open bucket). It now aborts (`throw`, no
further step; the `finally` block still restores the database) when the
dashboard window does not appear within 20 s or when the Cloud Storage page
is not empty at startup. Before this, a run whose account database swap
had not taken effect acted on the user's own account
([status.md](../status/status.md#ui-test-acted-on-a-real-account-2026-11-14)).
Check `cmdkey /list` and the account database before any run.

user would, with the credential store checked from outside the app. Two
scripts, one per platform; each has a mode that contacts no provider and a
`b2` mode against the Backblaze test bucket of the
[integration tests](#cloud-storage-integration-tests).

**What both check:** open Cloud Storage → add a Backblaze B2 account (name,
bucket, region, key) → the OS credential store holds exactly the typed key
under `com.sotyware.desktop` / `cloud-account/<id>` (compared, never shown),
the account database holds no key id or secret (file bytes), the page shows
neither → Check connection → rename without a key (store entry unchanged) →
Replace access key (still one entry, now the new key) → Remove (the
confirmation says the bucket and its files are not deleted or changed) →
gone from page, database and credential store; no key id or secret in
sotyware's output. Account ids and labels are new for every run
(`sotyware-e2e-<timestamp>`), so a run never meets a real account.

| Mode | Check connection expects | Also |
| --- | --- | --- |
| Linux `local` | **Network error** (endpoint `http://127.0.0.1:9`, made-up keys) | a second account with the login collection locked over D-Bus: **Credential store locked**, and Remove keeps the account while its key cannot be deleted. Where gnome-keyring's system prompter is installed (the distribution package ships `org.gnome.keyring.SystemPrompter.service`, which the private bus activates), the unlock dialog of `gcr-prompter` opens on the Xvfb display; the driver presses its Cancel through AT-SPI, as a person who declines would, and prints how many it declined. With an unpacked gnome-keyring there is no prompter and the store refuses at once; nothing is pressed |
| Linux `hang` | — (a Secret Service that never answers) | while the add waits: the Settings link and the Scheduled backups switch still work, the process keeps running |
| Linux `browse`, Windows `-Mode browse` | — (Windows: **Connected**) | the bucket browser against the in-process listing server (`cargo test … serve_listing_for_ui -- --ignored`, started by the script in this checkout's `src-tauri`, made-up key): Open bucket → the root's folders and files with size; a folder with spaces and Unicode; the bucket breadcrumb; `photos/` without its marker or nested file; `photos/2024/` with a MiB size; Up one folder; a marker-only folder → "This folder is empty."; a folder of 1,005 entries → "1000 items shown; more are available", Load more → 1,005, no Load more; Refresh → 1,000 again; ← Back to accounts; remove. The server fails the phase if anything but `ListObjectsV2` (or `HeadBucket`) reached it |
| Linux `download`, Windows `-Mode download` | — (Windows: **Connected**) | downloads through the **native save dialog** from the in-process object server (`cargo test … serve_objects_for_ui -- --ignored`, made-up key; it writes the objects' sizes and SHA-256 next to its endpoint file): every file row has Download; the dialog opens with the key's last segment suggested, and cancelling it starts nothing; Save → "Downloaded …" with the SHA-256 matched and the file's SHA-256 equal to the server's; an existing file → the dialog's own replace question (GTK: "already exists" + Replace; Windows: "Confirm Save As" + Yes), the old file untouched until then, replaced afterwards; `docs/Bericht 2024 ü & co.pdf` (3 MiB) suggested unchanged and verified; a file without a stored SHA-256 → completed, "could not be verified"; a slow 16 MiB file → percent and "of 16 MiB", a hidden `.large.bin.*.sotyware-partial`, Cancel → "Download cancelled. Nothing was saved.", no file left; a connection cut after 256 KiB → failed with the network text, Try download again (no dialog) fails the same way; content that does not match its SHA-256 → integrity failure, nothing saved; Windows only: `readme.txt` saved under `lone-<U+D800>-copy.txt` (an unpaired surrogate, typed into File name) → sotyware keeps running, verified, the file under exactly that name (failed with `0xC0000409` before the rfd patch, ADR-023); at the end the download folder holds exactly the completed files. Linux only: closing the window during a download (dashboard-only mode quits) → exit 0 and no temporary file. Files go to a folder in the throwaway HOME (Linux) or a new folder under `%TEMP%` (Windows, removed afterwards). The server fails the phase if anything but `GetObject`, `ListObjectsV2` or `HeadBucket` reached it. On Linux the GTK dialog's name field is found through AT-SPI and the path typed with XTest (ASCII paths only); on Windows the common item dialog's File name box, Save and Cancel are not in its UI Automation tree (Windows 11 25H2), so they are found as child windows by control id (Edit `1001`, Button `1`, `2`) and driven with `WM_SETTEXT` / `BM_CLICK`, and the Yes of "Confirm Save As" with `TDM_CLICK_BUTTON`; any script error in this mode is a failed check and the account is still removed |
| Linux `upload` | — | upload batches through the **native GTK multi-file dialog** into the in-process **s3s-fs** server (`cargo test … serve_uploads_for_ui -- --ignored`, made-up key) behind a `ThrottledLink` taking 6 MiB/s towards the server, with the stub tray watcher of `virtual.sh` (**tray mode**). The server is seeded through `S3Store::upload` with `inbox/existing.txt`, `other/readme.txt` and a 24 MiB `downloads/big.bin`. The helper takes commands from `<endpoint file>.ctl` (`dump` → `.state.json` with every object's content SHA-256, size, `HeadObject` size and SHA-256 metadata, the open multipart files and the operations; `verify-denied`; `abort-denied`; `delete <key>`) and fails the phase if the app sent anything but listing, reads and upload requests. The dialog is driven by Ctrl+L, the folder path typed with XTest, Return, Ctrl+A in its `Files` table and its Open button pressed through AT-SPI. Ten batches ([results](../status/status.md#cloud-storage-uploads-through-the-release-binary-on-a-virtual-display-2026-11-12)): odd names, empty file, existing object and the Rust skip reasons, destination fixed while the browser moves; cancel during a PUT and Try again; cancel during multipart (aborted); refused abort (parts kept, retry cleans them); refused verification (needs inspection, no retry for it); a file changed while sent; a download beside an upload; ← Back to accounts during a batch; close (hide) and a second launch during a batch; tray Quit during multipart. Ends by deleting exactly the known keys and checking the bucket is empty. The phase is in `cloud_ui_upload.py`, run by `cloud_ui.py` |
| Windows `-Mode upload` | **Connected** | the same server, seed and state reading as Linux `upload` (`serve_uploads_for_ui`, a new temporary bucket per run; the script talks to the helper through the same `.ctl`/`.state.json` files), with the link set to **2 MiB/s** (`SOTYWARE_UI_UPLOAD_RATE`: UI Automation reads the page far more slowly than AT-SPI, and at 6 MiB/s a 40 MiB PUT ended before the first poll), through the **native Windows multi-file dialog** ("Choose files to upload", the common item dialog `#32770`): the folder is opened by putting its path into File name (Edit `1148`) and pressing Open (Button `1`), and the address bar (ToolbarWindow32 `1001`) must then name it; files are chosen in its file list through UI Automation (`UIItem`, SelectionItem `Select`/`AddToSelection`; the dialog then shows them as quoted names in File name) or by putting their quoted names into File name; Cancel is Button `2`. Batches: the dialog cancelled (no panel, nothing sent); odd names (Unicode, spaces, `+ & # ( )`), the empty file, an existing object, and **a file another process holds open without sharing** (Failed, "could not be read on this computer", nothing stored; Try again uploads it once released), with the browser moved to the root while the dialog is open; cancel during a single PUT and Try again; cancel during multipart; refused abort; refused verification (needs inspection, no retry for it); a file appended to while it is sent; a download through the save dialog beside an upload; ← Back to accounts during a batch; close (`WM_SYSCOMMAND SC_CLOSE`, as the title bar's X: hide in tray mode) and a second launch during a batch; **tray Quit during multipart with the real mouse** (right click on the tray icon in the "Show Hidden Icons" flyout, click on Quit, as `windows.ps1`); relaunch (the account still works); then **a file name with an unpaired surrogate** (NTFS allows it) chosen with a valid one: sotyware must keep running, skip it as "name cannot be an object key" and upload the valid one (it ended with `0xC0000409` before the rfd patch, ADR-023). Every batch's server state is checked; the end deletes exactly the known keys and checks the bucket is empty. Also checked: the window driven belongs to the started binary, nothing listens on the dev-server port 1420. Needs the unlocked interactive desktop with a taskbar (exit 3 otherwise) |
| Linux `b2`, Windows `-Mode b2` | **Connected** for the bucket-restricted key; after the rename still Connected; after replacing the key with a wrong secret **Invalid credentials** (so the replaced key is the one used); after putting the real key back Connected | Linux only: the bucket changed to a name the key may not use → **Access denied** or **Bucket unavailable**, not Invalid credentials, then back to Connected. Both: `backblaze_b2_fingerprint` before and after; the run fails if any object version under `sotyware-it/` changed |
| Linux `b2-browse`, Windows `-Mode b2-browse` | **Connected** for the read-only key | the [temporary dataset](#browsing-a-temporary-dataset-at-backblaze) in the real bucket: `backblaze_b2_browse_dataset` with `SOTYWARE_IT_B2_BROWSE_KEEP=1` creates and checks it (with the write key) and writes the manifest; the app gets the read-only key; Open bucket → root shows `sotyware-it`; down to the run folder; the run root, `docs/` → `2024/` → `q1/` → `deep/` → `er/`, the `2024` breadcrumb, Up one folder, both marker-only folders (empty), `photos/` (no marker row), `special/` and its folder `f+o&l?d#e%r`, the Unicode folders, `dots/` → `..` (holds `escape.txt`) → Up, `double/` → (no name) → (no name), `many/` (1,000 rows and Load more → 1,010 rows, no Load more → Refresh → 1,000); each folder must show **every row exactly as the provider listed it** (name, size as the page formats it, date formatted with the page's own `toLocaleString` call run in `node`); ← Back, Open bucket again → root; remove. Fingerprint after creating = after browsing (the UI wrote nothing); then `backblaze_b2_browse_cleanup` (also on exit or interrupt), and the fingerprint before the dataset = after cleanup |
| Windows `-Mode b2-download` | **Connected** for the read-only key | one real download through the dashboard and the native save dialog, [described above](#downloading-through-the-windows-dashboard-at-backblaze) |
| Windows `-Mode b2-upload` | **Connected** for the upload key (`b2-upload.xml`) | one real upload batch through the dashboard and the native multi-file dialog, [described above](#uploads-through-the-windows-dashboard-at-backblaze); `-Rehearse` runs the same steps against the local s3s-fs server |
| Windows `-Mode b2-lifecycle -Scenario <scenario>` | **Connected** for the upload key (`b2-upload.xml`) | one upload lifecycle scenario (multipart, cancel, retry, leave, quit) through the dashboard with the release settings, [described above](#upload-lifecycle-through-the-windows-dashboard-at-backblaze); `-Rehearse` runs it against the local s3s-fs server |
| Linux `sync`, Windows `-Mode sync` | — (Windows: **Connected**) | folder sync (ADR-043) against the in-memory versioned server of `serve_sync_for_ui` (`src-tauri/src/cloud/test_server/sync_ui.rs`, one versioned bucket seeded with `welcome.txt` and `docs/plan.txt`, made-up key), which plays a second computer on the same bucket through its `.ctl` file (`put <key> <text>`, `delete <key>`) and answers `dump` with every key's current SHA-256, versions and delete markers and the app's operations. Through the bucket's **Sync** tab: Choose folder… in the **native folder dialog** (GTK: as in `files`; Windows: the pick-folders dialog, Folder box and Select Folder by control id) → the confirmation names the empty folder → Start sync → `.sotyware-sync/root.json`; the first pass brings both files down byte for byte; a file written into the folder reaches the bucket; a file put at the server comes down (Sync now pressed every 10 s, so nothing waits for the 5-minute listing); a local rename becomes a rename in the bucket (the old key a delete marker, its version kept); a file deleted at the server leaves the folder and lands in `.sotyware-sync/trash/<day>/` with its content; a file changed on both sides is kept twice (both contents survive, one under `plan (conflicted copy …)`); then sotyware is ended and started again in the same profile (Linux: the window's close, which quits in dashboard-only mode; Windows: `Stop-Process`, as closing only hides in tray mode and the tray Quit would move the mouse), the **startup connection check** (ADR-046) must reach the server with no click (its `HeadBucket` count grows; folder sync sends none) and the account show Connected, and a file written after the restart must reach the bucket with no click; Turn sync off… changes nothing in the folder or the bucket. Every step is checked on disk and at the server. The server fails the phase if a real version was ever removed by id, the app sent a request folder sync and the check do not send, or no transfer reached it. The phase is in `cloud_ui_sync.py`, run by `cloud_ui.py` |
| Linux `account`, Windows `-Mode account` | — (Windows: **Network error**: this mode's storage account points nowhere) | the GitHub pages of ADR-049 and the sotyware account. A test-profile build never starts a browser: `open_github_page` appends the address to `opened-pages.txt` in the run's profile, which the driver reads. Settings › About & updates: nothing recorded before a click; **Send feedback** → exactly the feedback form's URL; **Report a bug** → the bug form with `version=<x.y.z>&os=Linux%2C%20<arch>` (Windows: `Windows%2C%20<arch>`) and nothing else; **Privacy notice** → the notice on `main`. Then sign-up against a local account service, this checkout's `server/target/release/sotyware-server` (built by the script if needed; loopback, a free port, its own database in the run's folder, no mail relay, so mails stay in its outbox), reached through `SOTYWARE_ACCOUNT_SERVER`, which only debug and test-profile builds read: the age line only in Create account; its Privacy notice link records the notice and sends nothing; register (the service's database: the address, not verified); the code read with `sotyware-server outbox`; Verify and sign in (verified); Set up sync → the recovery key shown once, typed again, I saved it → the synced state (the service holds the recovery wrap and a vault containing neither the key, the password nor the address); Delete account… with the password → gone from the service. The service's database is read directly (Linux: Python's `sqlite3`; Windows: Node's built-in `node:sqlite`); the phase fails if the service logged an error or the password or recovery key reached sotyware's output. The phase is in `cloud_ui_account.py`, run by `cloud_ui.py` |
| Linux `autoclicker` | — | the autoclicker and macros (ADR-050) with **real input through XTest, on the script's private display only**, into a plain window of the driver that records every click and key it receives (no keyring, no server; the X11 backend works because the script unsets `WAYLAND_DISPLAY`): off by default (the driver can grab F6 and F8 itself; F6 does nothing); **Turn on** → the six suggested shortcuts registered (the driver can grab none of them); the Clicker tab's At a fixed position, X, Y, Stop after 25 clicks and 20 per second, typed as real keys → saved in the profile (`automation.sqlite3`, read directly); **Start autoclicker** → no click in the countdown's first 2 s, then exactly 25 left clicks at that position; from the test window F6 starts and stops it, F7 pauses (no click while paused) and resumes it, F8 (the emergency stop) stops it, the pointer pushed into a corner stops it; with the pointer on sotyware's own window every click is skipped and counted; F9 records two clicks and the key A typed into the test window, F10 stops, **Save macro** → the database holds both clicks where they were made and the key, nothing of F9 or F10; **Play** replays them into the test window; after every stop no button or key is held (`XQueryPointer`, `XQueryKeymap`); Settings › Features turns it off → the driver can grab all six shortcuts again and F6 does nothing; in between, the page's [former manual check](#the-pages-former-manual-check-automated): buttons and speed timed by the X server, shortcut fields and the keys they refuse, the confirmation without an emergency stop, playback options (2× timed, F7, F8, Shift+F11), the editor, a restart, profiles, a macro's shortcut, export and import through the GTK dialogs, keys a macro holds released by F8, Stop and quitting, 100 clicks per second (rate, tab switches, CPU), screen-reader text (AT-SPI), the emergency stop's key refused while off, and the Wayland message (103 checks). The phase is in `cloud_ui_autoclicker.py`, run by `cloud_ui.py` |
| Windows `-Mode local` | **Network error**, and Open bucket shows the network error with Try again | the real Windows Credential Manager (read with `CredRead` and `cmdkey /list:<target>`; `cmdkey` never shows secrets); the same `com.sotyware.desktop` lines in `cmdkey /list` before and after |

In `local` (both platforms) Open bucket must show "Could not load this
folder." with the network error and Try again; in `b2` it is pressed at the
root once and must show a listing or Access denied (the test key is limited
to `sotyware-it/`), then ← Back to accounts.

**Linux** (`tests/desktop/cloud-ui.sh [local] [hang] [browse] [download] [upload] [files] [usage] [sync] [account] [b2] [b2-browse]`,
default `local hang browse download upload files usage sync account`): Xvfb + openbox and a private `dbus-run-session`, a real
`gnome-keyring-daemon --unlock` with a throwaway login keyring in a
throwaway HOME under `/tmp/sotyware-cloud-ui.XXXXXX` (removed at the end,
keyring included; never your own keyrings or display). It needs what
`virtual.sh` needs plus GNOME Keyring: installed (`sudo apt install
gnome-keyring`), or unpacked without root as in
[the OS credential store test](#os-credential-store-test) with
`SOTYWARE_GNOME_KEYRING_ROOT` pointing at that folder. Controls are found
and pressed through AT-SPI; text is typed with XTest key events, because this
WebKitGTK exposes no AT-SPI `EditableText`.

```bash
npm run build:test-profile
tests/desktop/cloud-ui.sh
```

Every phase runs the app in a fresh [test profile](#test-profiles-adr-027)
inside the throwaway `HOME`; the credentials go to the private keyring under
service `com.sotyware.desktop.test` and the profile's nonce.

The `files` phase (ADR-026) drives the file manager against the in-memory
server of `serve_files_for_ui` (`src-tauri/src/cloud/test_server/files_ui.rs`:
a versioned and an unversioned bucket, 127.0.0.1, made-up key): folders
(create, open, breadcrumbs, nested, rename with the `photos/` versus
`photos-backup/` boundary, copy through the picker, move with a conflict
kept both, delete, empty folder), files (rename, properties, conflicts
skipped or kept both, a move whose delete is refused and retried, a copy
stopped by a refused request and retried, Cancel in the review, delete and
restore from Deleted items, a partly refused folder delete and its retry),
selection (click, Ctrl-click, Shift-click with the real pointer, Escape,
Ctrl+A, context menu, reset on navigation), sorting and filters, search
(folder, whole bucket with Search further, key prefix), Load more,
previews (text, JSON, a text starting with "BM", HTML and SVG as text, a
cut text, a PNG, an image over 10 MiB), a share link fetched without
credentials, sotyware's own clipboard with real X key events (Ctrl+X,
Ctrl+C, Ctrl+V opening the reviewed plan, a second Ctrl+V under the review
ignored, Cancel, a conflict skipped by default, a folder pasted into
itself refused, the keys left to the search box while typing), **folder
uploads through the native GTK folder dialog** (a Unicode folder with
`&`, `#`, `%`, `+`, nested and empty folders: exact keys, content and
stored SHA-256; the same folder again skipped; a refused PUT stopping the
batch, Try again, the rest), **several-item downloads through it**
(unsafe names and case clashes made safe and numbered, a second download
to `dlx (1)`, an existing local file never replaced, a refused file
stopping the batch and Download the rest again, Cancel while a GET is held
open and the retry), **a drop from another program** (`drag_source.py`, a
GTK drag source, dragged with the XTest pointer over XDND: a file and a
folder announced by name, Upload here, the same drop skipped, Ignore) and
a delete on the unversioned bucket. After each step it reads the server's
state (`dump`: every key's current object and versions) instead of
trusting the page; the helper fails at the end if a real version of the
versioned bucket was ever removed by id or the app sent a request the
file manager does not use. How the folder dialog is driven: GTK's chooser
starts in "Recent", where Open is disabled (AT-SPI's press of a disabled
button does nothing; that is why the earlier driver failed), so the driver
clicks Home in the sidebar, then puts the path with a trailing `/` into the
location bar (AT-SPI EditableText: the Xvfb keymap has no `ö` or `ß`),
removes GTK's inline completion and presses Return, which accepts the
folder. It does not test the system clipboard (no clipboard manager),
Ctrl/Shift-click with the pointer (XTest modifiers are not seen by
WebKitGTK on Xvfb), or cancelling a folder upload while a file is sent
(the same batch code as Upload files…, cancelled in the `upload` phase),
and says so. The server's `hang <Operation> <n>` holds requests open for
the cancel step.

The `usage` phase (ADR-041, `cloud_ui_usage.py`) drives a bucket's **Usage
& costs** tab against the in-memory server of `serve_usage_for_ui`
(`src-tauri/src/cloud/test_server/usage_ui.rs`: one versioned bucket with
current files, older versions and a file under a delete marker, 127.0.0.1,
made-up key). The server writes the figures a measurement must find; the
unit test `a_measurement_of_the_seed_finds_what_the_desktop_test_expects`
checks them against the measurement code, so the two cannot drift. The
phase: the tab before any measurement; Measure now, which must finish
(with 2 listing requests); the tiles, the stored parts and the delete
markers against those figures; the estimate from Backblaze B2's price list
(each component, the long note on API calls, the total); How this is
estimated; 700 GB of expected downloads (egress and the total follow);
Measure again (the history chart of 2 measurements). Then, at the
first-launch size (900 × 540 on its 1280 × 900 screen), at 1,240 × 820 and
at 700 × 540 (below 760 px the navigation is an overlay), the window
resized through the window manager (`XResizeWindow`, as dragging its
border would), it compares the AT-SPI extents of the Usage & costs and the
Snapshots & backups tabs' sections and of every visible element in them
with the edges of the section and of the page's scroll area (`<main>`,
which keeps the window's width less the sidebar): nothing may reach past
them. That is the check for
[the cost table running past its card](../status/status.md#the-cost-table-inside-its-card-2026-11-16),
which no headless test can see (happy-dom lays nothing out). Both edges,
because the engines fail differently: in Chromium the table spills out of
a card that keeps its width; in WebKitGTK it widens the card, the section
and the bucket view up to `<main>`, so a comparison with the section alone
passed a build without the fix. It counts content scrolled out of the
window too, and fails unless the cost table's row headers were among the
elements compared. The server fails the phase if the app sent anything
but listings (`ListObjectsV2`, `ListObjectVersions`,
`ListMultipartUploads`, `ListParts`, `HeadBucket`) or the bucket changed.

For `b2`, the same four variables as the integration test, entered at
prompts (or loaded from your own 0600 file, as described there) so they
are neither in the shell history nor on a command line; the script reads them
from the environment, strips them from sotyware's environment and never
prints them (the bucket name is shown as `<bucket>`):

```bash
export SOTYWARE_IT_B2_REGION=eu-central-003
read -r SOTYWARE_IT_B2_BUCKET && export SOTYWARE_IT_B2_BUCKET
read -rs SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID && export SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID
read -rs SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY && export SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY
tests/desktop/cloud-ui.sh b2
```

**Windows** (`tests\desktop\windows-cloud-ui.ps1 [-Mode local|browse|download|upload|files|usage|b2|b2-browse|b2-download|b2-upload|b2-lifecycle]`;
`usage`: the steps of the Linux `usage` phase against `serve_usage_for_ui`,
through UI Automation (the tab by Invoke or SelectionItem, the cost table
read from its header items, the expected downloads set through the Value
pattern), with the window sized by `SetWindowPos` (no mouse) so that the
page is 900 × 540, about 1,240 × 820 (less where the work area is smaller)
and 700 × 540 CSS pixels at the window's scale, and every element's
bounding rectangle compared with the section's and with `<main>`'s (the
landmark whose ARIA role is `main`);
`files`: the file manager against `serve_files_for_ui`, UI Automation
only, as the Linux `files` phase minus selection by pointer, search, the
clipboard keys and the drop; with folder uploads and several-item
downloads through the **native Windows folder dialog** (found as the
`#32770` window of sotyware's process; the Folder box is the classic
Edit 1148, set with `WM_SETTEXT`, and Select Folder is button 1, pressed
with `BM_CLICK`); the server's dump is read with `JavaScriptSerializer`
because Windows PowerShell's `ConvertFrom-Json` refuses keys that differ
only in case):
UI Automation only (Invoke, Value and Toggle patterns): no mouse, no
keystrokes, but a logged-in session (`download` shows the real save dialog
on the desktop; it is driven through UI Automation too, not clicked).
**Exception: `-Mode upload` moves the mouse** (one right click on the tray
icon and one click on Quit, after opening the "Show Hidden Icons" flyout;
it may press Esc once to close the flyout) and needs the unlocked desktop;
do not use the mouse or keyboard while it runs (about 15 minutes). The app
runs in a fresh [test profile](#test-profiles-adr-027) of the run (a
test-profile or recorder build is required); the user's own folders and
`com.sotyware.desktop/` credentials are compared before and after and never
touched. `-Mode b2` reads the key
from `%USERPROFILE%\.sotyware-it\b2.xml` (`Get-Credential | Export-Clixml`:
keyID as user name, applicationKey as password, DPAPI-encrypted for your
Windows user) and the bucket from `b2-bucket.txt` next to it; it sets the
integration-test variables only around the `cargo test` fingerprint calls.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode browse
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode download
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode b2-download
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 -Mode upload
```

Before running a changed Windows script, check it statically (it parses
the scripts, runs nothing and opens no window; it also runs from WSL2
through `powershell.exe` with `wslpath -w` paths):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\check-ps-variables.ps1
```

PowerShell variable names ignore case, so `$out` and `$Out` are one
variable; that clash broke a `b2-download` attempt after its upload on
2026-11-12. The check fails when one scope (the script, or one function)
spells a variable two ways, and on a parse error. Without arguments it
checks `windows.ps1`, `windows-isolated.ps1` and `windows-cloud-ui.ps1`.

Keep the Windows scripts **ASCII only**, comments included; write other
characters as `[char]0x…` or as a regex escape (`\u00b7`). Windows
PowerShell 5.1 reads a script without a byte order mark as ANSI, so a
literal `·` in a regex became `Â·` and never matched (seen 2026-11-12 in the
first `-Mode upload` runs).

`browse`, `download` and `upload` (and the same phases of `cloud-ui.sh`) compile and
run the test build the first time, so they need the Rust toolchain, like
`b2`.

`b2-browse` needs the variables of `b2` plus the read-only key's two (Linux;
Windows reads `b2.xml`, `b2-read.xml` and `b2-bucket.txt`), the Rust
toolchain, and `node` on `PATH` (for the date format); it takes about four
minutes, most of it creating the 1,040 objects. Run one Backblaze test at a time
(on one machine or two): the fingerprint covers all of `sotyware-it/`, so a
second run's dataset makes the before/after checks fail (seen 2026-11-11 when
a Linux and a Windows run overlapped; each run's own cleanup was still
complete). On Windows the page's dates follow the Windows display language
(WebView2), not the regional format that `node` would use by default, so the
script passes `(Get-UICulture).Name` to the same `toLocaleString` call.

Not covered by either: an unlock prompt answered by a person, KWallet,
Windows 10, the connection check against Hetzner, browsing at Hetzner,
downloading from a real provider on Linux (the UI) or from Hetzner, a
checksum mismatch at a real provider, the XDG desktop portal's save dialog,
quitting through the tray during a download (Windows), uploads at any
real provider or on Windows 10, uploads through the XDG portal's file
dialog, a person selecting files in the Windows dialog with the mouse or
Ctrl+A (the driver selects through UI Automation or the File name box), a
duplicate-key selection (both dialogs select within one folder, which
holds each name once), names Windows itself refuses (control
characters, `< > : " | ? *`), a cancel exactly between two files,
key-too-long and too-large skips, an abort that hangs at quit, and the
hashing and verifying stages in the panel (too short to be sampled; only
"Sending" is seen).

## Frontend tests

```bash
npm run test:frontend
```

`src/App.test.ts` mounts the real `App.svelte` (and imports `main.ts`) in
[happy-dom](https://github.com/capricorn86/happy-dom) under Vitest, with Tauri's
official mocks (`mockWindows`, `mockIPC` from `@tauri-apps/api/mocks`) in place of
the Rust side. The mocked Rust side answers `operating_mode` with `"tray"` unless
a test says otherwise. It checks that:

- the shell asks for the mode, the accounts (sidebar), the backups (Overview
  summary) and the update status once each on mount, and nothing else;
- in tray mode, "Hide dashboard" sends exactly `plugin:window|hide` for window
  `main` once per click, a failing or denied call shows the error in a
  `role="alert"`, and a successful retry clears it;
- in dashboard-only mode, a `role="status"` note says that no compatible tray is
  available, the dashboard cannot be hidden, the window must stay open, and
  closing it quits; there is no button and no hide call;
- if `operating_mode` fails or returns an unknown value, the error is shown and
  there is no Hide button;
- `main.ts` throws without `#app`, and mounts the Overview into it;
- the update notification shows once for an available version, installs
  only on its **Update** button, is disabled while a transfer runs, goes
  away with **Later** (saved) or close (this session), offers **Restart
  now** after an in-place install, and **Details** leads to Settings › About
  & updates (where it is not repeated);
- Ctrl/⌘+3 and Ctrl/⌘+, switch sections, plain keys do not; navigation
  moves focus to the page's `<h1>`; the theme chosen in Settings applies
  `data-theme`, is saved in `localStorage` and asks the window to follow
  (`plugin:window|set_theme`); the Settings tabs follow the arrow keys.

`src/theme.test.ts` checks the theme preference against a stub and a
throwing storage; `src/navigation.test.ts` the shell's shortcut mapping
and the colour meaning of every connection-check result.
`src/CloudStorage.test.ts` also opens a bucket from the sidebar (and the
Cloud Storage link closes it) and adds an account from the sidebar's **Add
storage account**.

`src/Features.test.ts` mounts `Features.svelte` (and once the whole `App`) with
a mocked registry. It checks that the list is read once on mount and shown in
order with name, version, state and description; that a failed feature shows
its error without an alert; that the checkbox is unavailable while a feature is
starting or stopping, or while a request is in flight; that unchecking and
checking send `set_feature_enabled` with `{ id, enabled }` and show the new
state; that a refused switch restores the checkbox and shows the error in a
`role="alert"`; that a list error or a malformed answer is shown as an error;
and that the dashboard's only button is still "Hide dashboard". For stored
choices it checks that a `storageError` is shown as an alert while the
features are still listed and switchable, and stays after a switch; that a
`saveError` after a disable or an enable shows the feature's new state and an
alert saying the choice lasts until sotyware quits; that a later saved switch
clears that alert; and that a malformed switch answer is an error.
`parseFeature`, `parseFeatureList` and `parseFeatureChange` are tested against
the shapes Rust sends and against malformed values. `App.test.ts`'s mocked Rust
side answers `list_features` with no features and no storage error.

`src/CloudStorage.test.ts` mounts `CloudStorage.svelte` (and the whole `App`
for navigation) with a mocked Rust side that plays the six cloud commands. It
checks the navigation (the dashboard opens on the Overview with only the
sidebar's account list requested and "Hide dashboard" as the page header's
one button; the sidebar's section links; the "Cloud Storage" link shows the
section and back; an account opened from the sidebar; the sidebar's Add
storage account); the empty state with its Add account action;
the list with provider, bucket, region, endpoint and status, also for two
accounts at the same provider, each with its own actions; a stored account
that no longer validates; an unopenable account database (no actions) and a
failing list. Adding: the first provider's presets and region suggestions,
the endpoint becoming required for a generic service, password inputs without
autocomplete, exactly `{ account, credentials }` sent, the new account
listed, no key id or secret left in the DOM or in local/session storage,
empty credential fields when the form opens again, a second account at the
same provider, a credential store that is unavailable (form kept open, the
message says nothing is stored elsewhere; cancel clears the key), invalid
input and a locked keyring as their own messages, the form disabled while
saving. Editing: a rename sends `credentials: null` and needs no key; a key
replacement sends the new key and the fields are empty afterwards; unticking
"Replace access key" clears typed values; a refused replacement keeps the
form open; a `partial` result clears the key and reloads the list. Removing:
a confirmation that names the bucket and says its files are not deleted,
cancel sends nothing, confirm removes only that account; a failed removal
keeps it listed with the error. Checking: "Checking connection…" and a
disabled button while pending (the other account unaffected), then the
result; every status with its own label and, for failures, the hint and
detail; access denied not shown as invalid credentials; a vanished account
reloads the list. `cloud.ts`'s parsers and `toCommandError` are tested
against the Rust shapes and malformed values. Each account's actions now
start with Open bucket.

`src/CloudBrowser.test.ts` opens the browser from `CloudStorage.svelte` as a
user would, with a mocked `list_cloud_folder` that serves pages per account
and prefix. It checks Open bucket per account (disabled for settings that do
not validate, no request before it is pressed); the root with folders first,
sizes (bytes, KiB, MiB; exact bytes as the title), dates, "5 items shown.",
no Load more, the account list and Add account gone; "Loading folder…" while
pending; the empty states of a bucket and a folder; opening folders by the
exact prefix Rust returned, breadcrumbs (the current one not a button), Up
and the bucket breadcrumb; a folder with an empty name ("(no name)") under
`a//`, Unicode, and a name with markup shown as text; Back to accounts with
no request; a slow folder's late answer dropped after another navigation;
Load more sending the cursor with the same prefix, appending without
duplicates and without a total, and disappearing on the last page; Refresh
and opening a folder starting again with `page: null`; "Loading more…" and a
late page dropped after leaving the folder; a failed page retried with the
same cursor (what was loaded stays) and an outdated page refreshed; Try
again after a failed folder; every error kind's own text and Edit account
only where editing fixes it; Edit account opening that account's form; a
removed account closing the browser with a message and a reload; and that a
slow answer for one account never shows in another's browser.
`parseFolderListing` is tested against the Rust shape and malformed values.
Downloads, with mocked `download_cloud_file`, `cancel_cloud_download` and
`retry_cloud_download` whose channel the test feeds directly: a Download
action per file (none per folder) sending the account, the browsed bucket and
the exact key (with markup in its name, shown as text) and no path; "Choose
where to save" while the dialog is open; a cancelled dialog (`null`)
starting nothing; progress with percent and bytes, every Download action
disabled meanwhile (a second click sends nothing) while folders still open;
an unknown size shown as bytes only; completion with "SHA-256 … matched" or
the size-only text, a late message ignored, Dismiss; Cancel sending the
transfer id, "Cancelling…", then "Download cancelled"; a cancel answered
`finishing`; a failure with its text, Try download again sending the old
transfer id (no dialog) and following the new one while ignoring the old
transfer on both channels; messages of another transfer ignored and the
own one accepted before the command's reply; a refused start (`busy`) asking
again through the dialog; Edit account for a missing key; and leaving the
browser cancelling the running download.

`src/CloudUpload.test.ts` (42 tests) opens the browser the same way and
mocks `upload_cloud_files`, `cancel_cloud_upload` and `retry_cloud_upload`;
the test feeds each call's channel with events in the Rust shape
(`upload.rs`). It does **not** open the native file dialog or upload
anything: what the dialog and the backend do is tested in Rust. It checks:
Upload files… beside Up one folder and Refresh, sending only the account,
the browsed bucket and the folder's exact prefix; a cancelled or empty
dialog showing nothing and no error (and keeping the previous result); one
batch at a time; refused starts (`credentials-missing` with Edit account,
more than 100 files, `busy`, a removed account closing the browser); every
skip reason in the reply, and a selection with nothing to upload; stage,
bytes, percentage and "File *i* of *m*" for files one after another, no
percentage for a total of 0, nothing complete before its `completed` event;
older events (lower `seq`) never applied, also when they arrive late or
before the reply; the final summary with counts, conflicts, errors and
"not started"; network, timeout and integrity texts; 100 files in the
scrolling list; an event without the Rust shape reported without its
content; Cancel sent once by batch id, the batch shown as cancelled only
after `finished`, started-and-cancelled versus never started, `finishing`
and `not-active`, a failed cancel request, cancelling between files; Try
again offered only for `retryable` files, as a new batch id, ignoring the
old channel (also while the retry's reply is pending) and other batch ids;
no Try again for `needs-inspection` and `incomplete-parts` and no "try
again" in their notes; cleanup events listed apart from the files, without
claiming completion, not twice; a refused retry keeping or closing the
button by error kind; leaving the browser cancelling a running batch (also
one whose reply arrives later), not a finished one, and not on
`visibilitychange`/`blur`; the destination unchanged while browsing
elsewhere; another account's browser starting fresh; nothing applied after
unmount; an upload beside a download; and no local path in the text, alerts,
labels or titles. `parseUploadSelection` and `parseUploadEvent` are tested
against the Rust shapes and malformed values. A few mutations of the panel
(no stale-start check, retry by `failed` instead of `retryable`, no `seq`
check) each made tests fail.

It does **not** show that the OS window hides: no webview, no Rust, no
capability check runs. That is the job of the virtual-display test (scenario 12)
and the [Windows desktop test](#windows-desktop-test).

## Simulated desktop test

```bash
npm run build:test-profile
tests/desktop/run.sh [binary]
```

Files:

- `tests/desktop/run.sh`: creates a temporary private `$XDG_RUNTIME_DIR`, a
  private `$XDG_DATA_HOME` and a private D-Bus session (`dbus-run-session`), then
  runs the Python driver with `GDK_BACKEND=x11`. A sotyware already running on
  your desktop is not affected, and your own feature preferences
  (`~/.local/share/com.sotyware.desktop/preferences.sqlite3`) are never read or
  written: each run starts with no database. The driver refuses to start if
  `$XDG_DATA_HOME` is not inside the run's work folder. Exits 2 if the binary is
  missing.
- `tests/desktop/sni_watcher.py`: a minimal `org.kde.StatusNotifierWatcher`, so
  sotyware's tray (`ksni` backend) registers a tray item. It records each
  registration in a file that the driver counts. `--reject` answers every
  registration with `AccessDenied` (a broken watcher); `--no-host` reports
  `IsStatusNotifierHostRegistered = false`.
- `tests/desktop/single_instance.py`: drives the binary. It starts and stops the
  stub tray host itself, on the private bus, and waits for its bus name to appear
  or vanish instead of sleeping. It inspects X11 windows via `ctypes`, sends
  `WM_DELETE_WINDOW` to simulate the close button, clicks tray menu items over
  `com.canonical.dbusmenu`, reads and clicks the page inside the webview through
  AT-SPI (WebKitGTK's accessibility tree), and launches further instances
  sequentially and concurrently. Page elements are recognised by their
  `Atspi.Role` enum value (`PUSH_BUTTON`, `STATUS_BAR`), never by the role
  name: at-spi2-core 2.53.1 renamed "push button" to "button" (Ubuntu 26.04
  ships 2.60), while the enum value stayed 43. Scenarios:
  - 1–9: tray mode: first launch, second launches while visible, hidden and
    minimized, sequential and concurrent launches, close → hide, tray Open
    Dashboard, tray Quit, relaunch.
  - 10: unexpected tray failure: a watcher that rejects the item. Exit 1, the
    `cannot create system tray icon` message, no claim of dashboard-only mode,
    nothing left running.
  - 11: `$XDG_RUNTIME_DIR` under a read-only directory (as under WSL): still
    starts with one tray item and quits with 0, because the `ksni` backend
    writes no icon file.
  - 12: tray mode, the page: tray hint and a "Hide dashboard" button; the
    Features section lists Scheduled backups (`backup-scheduler`) as running
    with a checked checkbox (`CHECK_BOX` role, named "Scheduled backups"; the
    `example` feature it switched before 2026-11-15 was removed); toggling it through AT-SPI
    shows it disabled, toggling again shows it running, with no error (so the
    static capability allows `list_features` and `set_feature_enabled`);
    activating Hide through AT-SPI hides the window (so the runtime `tray-mode`
    capability works); tray Open Dashboard restores it; Quit. Before Hide, the
    Cloud Storage section: exactly the links "Overview" and "Cloud Storage"
    (`LINK` role); following "Cloud Storage" shows its heading, the empty
    state and "Add account" as the only button, with no error (so the static
    capability allows `list_cloud_providers` and `list_cloud_accounts`);
    `cloud-accounts.sqlite3` exists in the private data folder with no
    accounts; "Overview" brings back "Hide dashboard" and the features. No
    account is added: the private bus has no Secret Service.
  - 13: dashboard-only mode, session bus without a watcher: process alive and
    dashboard visible; mode and reason on stderr; no item registered and no
    `org.kde.StatusNotifierItem-<pid>-*` name owned; the page shows the no-tray
    note and no button (scenario 12 is the control that proves a button would
    be found); Scheduled backups is listed as running here too; a second launch exits 0 and restores the minimized dashboard
    without a new window or process; a watcher started afterwards gets no
    registration (no retry); closing the dashboard exits 0 and leaves nothing.
  - 14: relaunch with the watcher back: tray mode again, close hides, Quit.
  - 15: watcher without a host (`--no-host`): dashboard-only mode, no item left
    served, close exits 0.
  - 17: feature preferences across real restarts (tray mode). Each restart is a
    tray Quit with exit code 0 and no process left, then a new process; the page
    is never just reloaded. "No process left" includes the instance's WebKit
    helpers: they outlive sotyware by about 0.1 s, WebKitGTK's network
    process committing the page's `localStorage` meanwhile, and a relaunch
    before that would read the previous theme
    ([limitations](../status/limitations.md#dashboard-adr-035)), so the
    driver waits up to 15 s for them (WebKit's own watchdog is 10 s). Every
    launch carries its label in its environment (`SI_DRIVER_LAUNCH`, which
    sotyware never reads), and WebKit's helpers inherit it with the test
    profile, so a helper names its launch even after sotyware has ended.
    A quit waits only for its own launch's helpers; its
    `WebKitNetworkProcess` must be among those listed as sotyware's children
    before the Quit, and the check says how long they outlived sotyware.
    Scenarios 1 to 16 do not wait for theirs, so before 17 starts a check
    requires that no helper of an earlier launch is left (at most 15 s). A
    helper left is named with its program, pid, parent, state, threads,
    whether it is dumping core after a crash (`CoreDumping` in
    `/proc/<pid>/status`), start relative to the Quit (or that wait) and
    launch, then its command line, its threads (name, state, kernel wait
    channel) and its launch's output are printed; a quit's check names
    other launches' helpers still running without waiting for them. On
    GitHub's runner WebKit's web process sometimes crashed as sotyware
    quit, and its core dump outlasted the wait
    ([limitations](../status/limitations.md#dashboard-adr-035)).
    Scheduled backups (left enabled and saved by 12) is unchecked, Dark is
    chosen in Settings › General and sotyware quits at once, the database row
    (read with Python's `sqlite3`, read-only) says disabled, and the relaunch
    shows it disabled with no error and no storage warning in the log, and
    Dark still chosen; it is checked again, sotyware quits, the row says
    enabled, and a second relaunch shows it running. Scenarios 18 and 19 quit
    the same way.
  - 18: a corrupt `preferences.sqlite3` (text): sotyware starts in tray mode
    with Scheduled backups running (its default); the page shows "Could not
    load the saved feature choices: … not a database" and the log says so;
    unchecking disables it and the page says the choice could not be saved and
    lasts until sotyware quits; after Quit the file is byte-for-byte unchanged.
  - 19: a folder in place of `preferences.sqlite3`: default, the "Could not load"
    warning with `cannot open`, normal Quit, the folder left alone. An unusable
    data folder as a whole is not tested: Tauri creates the same folder for the
    webview's data and fails its own setup (`Failed to setup app`, abort) before
    sotyware's code matters; seen with `$XDG_DATA_HOME` set to a file.
  - 17–19 run between 15 and 16: under this AT-SPI setup, the first instance
    started after 16's no-bus instance never appeared on the accessibility
    desktop (the next one did), so its page could not be read.
  - 16: no session bus at all: dashboard-only mode, close exits 0. Each walk of
  the X11 window tree holds a server grab, so it cannot miss a window that the
  window manager is moving out of its frame (the dashboard right after a close).
  Tray menu clicks do not wait for a D-Bus reply, because "Quit" ends the process
  while it handles the call; the checks look at the effect instead. If the first
  launch registers no tray item within 10 s, the driver reports a FAIL with the
  primary's output and stops, since every later scenario needs the tray menu; the
  relaunch after Quit has its own tray-registration check. The only fixed wait
  is the 2 s in scenario 13 that gives a late watcher the chance to receive a
  registration that must not come.

Requirements: `python3`, `python3-dbus`, `python3-gi`, the AT-SPI typelib
(`gir1.2-atspi-2.0`) and registry (`at-spi2-core`), `libX11`, `dbus-run-session`,
and an X11 display (`$DISPLAY`: WSLg or a desktop session; for a display of its
own use the [virtual-display test](#virtual-display-test)).

Results: each check prints `PASS`, `FAIL` or `SKIP`, then a summary line. `SKIP`
means the environment could not exercise the behaviour (for example, a window
manager that ignores minimize requests). **A SKIP is never a pass.** Do not turn
a SKIP into a PASS by loosening a check; record it as unverified instead.

The driver exits 1 if any check fails and 0 otherwise; SKIPs do not change the
exit code, so always read the summary line too. It aborts at once if a
`sotyware` process from the same binary is already running in this run's scope.

Every process count (the abort check, "only the original process is running",
"nothing left") includes only processes of this run: same binary
(`/proc/<pid>/exe`), same `DISPLAY` and same `DBUS_SESSION_BUS_ADDRESS` in
`/proc/<pid>/environ`. A launch without a bus address (scenario 16) counts if
its `XDG_RUNTIME_DIR` lies in one of the driver's own temporary directories. So
a sotyware from the same binary on another display or bus (a concurrent
virtual-display run, a probe, your own instance) neither aborts the run nor
fails a check; one this run started is still always counted. Checked
2026-11-10 on WSL2: 92 passed, 0 failed, 0 skipped with a probe sotyware running
the whole time on its own Xvfb display `:57` and private bus.

Under WSLg two checks are always skipped: focusing an already visible dashboard
(the WM ignores `_NET_ACTIVE_WINDOW`) and restoring a minimized dashboard (the WM
ignores `XIconifyWindow`). The [virtual-display test](#virtual-display-test) runs
both under openbox. The latest results are recorded in
[the validation matrix](../status/status.md#validation-matrix).

What this test is **not**: a real desktop. The tray host is a stub, nobody sees or
clicks an icon, and the window manager is whatever serves `$DISPLAY`.

## Virtual-display test

```bash
npm run build:test-profile
npm run test:desktop:virtual              # = bash tests/desktop/virtual.sh [binary]
```

`tests/desktop/virtual.sh` runs the [simulated desktop test](#simulated-desktop-test)
unchanged, against the same release binary, on a display it creates itself:

- **Xvfb** on a free display number, listening only on the abstract socket: no
  TCP and no file in `/tmp/.X11-unix`. `-displayfd` tells the script when it
  accepts clients. The number is searched from `:99` down. A number is skipped
  if it has a lock file, a socket file, or a bound abstract socket
  (`@/tmp/.X11-unix/X<n>` in `/proc/net/unix`). If Xvfb exits before
  reporting ready, the script tries the next number. The script never lets
  Xvfb choose its own number: Xvfb would start at `:0` and could take the
  abstract socket of a real display that has only a socket file.
- **openbox** as the window manager. It runs `--startup` once it manages the
  screen, which is the ready signal. Unlike WSLg's window manager it honours
  `_NET_ACTIVE_WINDOW` and `XIconifyWindow`, so the focus and un-minimize checks
  run instead of being skipped.
- `WAYLAND_DISPLAY` is unset and `DISPLAY` points at the private server, so
  nothing reaches the display you are using. `run.sh` still adds its private
  D-Bus session and runtime directory, and the driver its stub tray host.
- The whole run has a timeout (`SOTYWARE_VIRTUAL_TIMEOUT`, default 300 s). On
  exit, the script kills sotyware processes started on its display, openbox and
  Xvfb, and prints the Xvfb and openbox logs if the run did not pass.

Requirements (Debian/Ubuntu): `sudo apt install xvfb openbox python3-dbus python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-x11`.
A missing tool is reported by name and the script exits 2 without starting
anything. These packages alone were enough on Ubuntu 24.04.1 (versions in the
[validation matrix](../status/status.md#validation-matrix)); `xvfb` pulls in
`x11-xkb-utils` for `xkbcomp`. Xvfb and openbox print harmless warnings
(unresolved `XF86…` keysyms, no `debian-menu.xml`), and the xdg portals the
webview activates on the private bus warn about PipeWire and RealtimeKit; none
affects the result.

Interrupting the run (Ctrl+C, or SIGTERM to `virtual.sh`) cleans up like a normal
exit: nothing it started is left running and its temporary directories are
removed (checked 2026-11-10).

Several runs can share one machine; each gets its own display. With
`-displayfd`, Xvfb writes no `/tmp/.X<n>-lock`: Xvfb 21.1.12 writes one
without the option and none with it (checked 2026-11-10, Ubuntu 24.04.1).
Before the abstract-socket check, the script could not see another run's
display. A second run started 5 s after the first chose `:99` again, and its
Xvfb could not bind the socket ("server already running"), so it exited 3.
Two runs can still pass the check together and pick the same number. Binding
the socket is atomic, so one Xvfb loses, and its run moves to the next number.
On WSL2 Ubuntu 24.04.1 (2026-11-10), two runs started 5 s apart and two started
together each passed 92/0/0, on `:99` and `:98`. The simultaneous pair took the
retry path ("Xvfb could not take :99, trying the next number").

A FAIL on "one tray item" (first launch, or relaunch after Quit) is a real
defect, not a test flake. Do not retry it away. Until `12a6185`, it was the
intermittent
[libayatana-appindicator race](../status/limitations.md#historical-libayatana-appindicator-registration-race)
(`G_IS_DBUS_CONNECTION` in the instance output). The `ksni` backend avoids that
library, and with it no such FAIL has been seen.

Because the environment is fully controlled, **a SKIP fails this run**: the
script exits 1 if the summary reports any skipped check. Exit codes: 0 all
passed, 1 a check failed or was skipped, 2 binary or dependency missing, 3 Xvfb
or openbox did not start, 124 timeout.

What it does **not** validate: GNOME, KDE, XFCE or any real tray host, Wayland,
GPU rendering, what a person sees, or Windows.

## Windows desktop test

Run on Windows, from a Windows checkout (not a WSL path), after a release build:

```powershell
npm run build:test-profile
npm run test:desktop:windows              # = powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1
npm run test:desktop:windows -- -Exe <path>
```

`-Exe` defaults to `src-tauri\target\test-profile\release\sotyware.exe`; it
must be a test-profile build ([ADR-027](#test-profiles-adr-027)): the
script refuses any other binary, so an installed copy (a normal build) can no
longer be tested with it.

It needs only Windows PowerShell 5.1 (UI Automation and `user32` via `Add-Type`), no
extra modules. **It runs on your real desktop:** it moves the mouse and clicks the
tray icon, the tray menu, the dashboard's button and feature checkbox, and
its own title bar (Close, a double click, and a drag that moves the window
by 80 × 60 px; ADR-040), drags the window's bottom-right corner and top edge
to resize it, and
opens the notification area's "Show Hidden Icons" flyout. Do not touch the mouse or
keyboard while it runs (about 40 seconds).

What it does, and how:

- Since 2026-11-15 (ADR-040; first run on native Windows 11 on 2026-11-16,
  130/0 with the frame checks below): the first
  placement (half the primary work area from `SPI_GETWORKAREA`, at least
  900 × 540 at the window's DPI, centred, ±2 px); no native caption
  (`WM_NCHITTEST` finds no `HTCLOSE`); the page's Minimize, Maximize and
  Close in the UIA group "Window"; Maximize and Restore through UIA
  (`IsZoomed`, back to the same rectangle); a real double click on the
  bar's middle maximizes and restores; a real drag moves the window by
  80 × 60 px without resizing it; the frame (since 2026-11-16): the window
  keeps `WS_CAPTION`, `WS_THICKFRAME`, `WS_MINIMIZEBOX` and `WS_MAXIMIZEBOX`
  (tao only hides the frame, so Snap, Win+Arrow and the system menu keep
  working), a real drag of the bottom-right corner (the system's invisible
  border) grows it by 60 × 40 px in place, and one of the top edge (Tauri's
  resize strip, `SM_CYFRAME` high, above the drag region) by 30 px with its
  bottom kept; Minimize through UIA; the page's Close
  clicked with the real mouse hides to the tray (replacing the old native
  close button check); after Quit and relaunch the window opens where it
  was and `window-state.sqlite3` is in the test profile.
- Finds the dashboard with `EnumWindows` (class `Tauri Window`, owned by a
  sotyware process). UI Automation is not enough: it does not list hidden windows.
- Clicks "Hide dashboard" inside WebView2 twice: once through UI Automation
  `Invoke` and once with a real mouse click at the button's position. Each time it
  checks that the window becomes invisible, is not destroyed, and the process stays.
- Reads the Features section through UI Automation: the heading
  (`features-heading`), the `CheckBox` of Scheduled backups (`backup-scheduler`;
  the removed `example` feature before 2026-11-15) and its
  `ListItem`, whose name is the item's text (version, state label, description).
  It checks the feature is listed checked and Running, then switches it off and
  on twice: through `TogglePattern` and with a real mouse click on the checkbox
  (only after `WindowFromPoint` confirms the dashboard is the window under the
  point). Each switch waits until the checkbox state, its enabled flag (it is
  disabled while a request is pending) and the state label match and no
  "Could not" text is on the page; then it checks that the process still
  answers (`Process.Responding`). This proves the static capability lets
  WebView2 call `list_features` and `set_feature_enabled`.
- Follows the "Cloud Storage" navigation link (UI Automation `Hyperlink`,
  `InvokePattern`) and checks the section's heading (`cloud-storage-heading`),
  the empty state ("No storage accounts yet"), an "Add account" button, no
  "Hide dashboard" button and no error text; that
  `cloud-accounts.sqlite3` was created in the test profile; then follows
  "Overview" back to the Hide button and the features. It adds no account,
  so it never writes to the Credential Manager.
- Checks that the choice survives real restarts: before Quit it disables the
  feature again; the relaunch (a new process) must show it Disabled and
  unchecked, with `preferences.sqlite3` present; it enables it, quits (exit 0,
  no process left), and a second relaunch (another new process) must show it
  Running and checked, all in the run's test profile, which starts empty
  (before 2026-11-14 the script moved the user's
  `%LOCALAPPDATA%\com.sotyware.desktop\preferences.sqlite3` aside and deleted
  "its" copy at the end, which is unsafe:
  [ADR-027](../architecture/decisions.md#adr-027-desktop-tests-run-a-test-profile-build-that-cannot-reach-the-users-data)).
- Since 2026-11-18, the theme chosen just before Quit, as scenario 17 of
  the Linux single-instance driver: after enabling the feature in the
  first relaunch it chooses Dark in Settings › General (the radio button's
  UIA `SelectionItem`, whose `IsSelected` is its checked state), quits
  through the tray at once (only the tray menu lies between the choice and
  Quit; not at the first quit, which hides the dashboard first), and the
  second relaunch must show Dark chosen and nothing else. The theme lives
  in the webview's `localStorage`, which WebView2 writes to disk itself;
  on Linux a relaunch that read it before WebKit's late commit showed the
  previous theme ([limitations](../status/limitations.md#dashboard-adr-035)).
  INFO lines give the time from the choice to the Quit click and to
  sotyware's end, and whether WebView2's local storage files (a LevelDB,
  `webview\EBWebView\Default\Local Storage\leveldb` in the profile, read
  without locking it) held the choice just after it was made, once
  sotyware had ended and once WebView2 had ended.
- Since 2026-11-18, every relaunch waits until WebView2's processes of the
  quitting instance have ended too, at most 30 s, so that it reads what
  any later start would. They are the `msedgewebview2.exe` processes whose
  command line names this run's test profile (WebView2 gives each process
  the user data folder, here the profile's `webview` folder, as
  `--user-data-dir`; the profile as the driver and as the app's
  acknowledgement spell it, lower case, without `\\?\`). A handle to each
  is opened before the Quit, so their exit times are read exactly
  (`GetProcessTimes`) after they have ended; one that started later is
  waited for by listing when its parent is that sotyware or one of its
  WebView2 processes (another sotyware in the profile would start a
  browser of its own). Each quit's check passes when they were found,
  one browser process (no `--type`) started after this sotyware among
  them, and all ended; an INFO line says how long each outlived sotyware.
  At the end any left are stopped by id, so the profile can be removed.
- Finds the sotyware icon in the notification area by its UI Automation name
  (`sotyware`, the tooltip). Windows 11 puts new icons in the hidden-icons flyout,
  so the test opens it first. It treats the flyout as open only while it is the
  foreground window: when hiding the dashboard hands the foreground to another
  app, the flyout starts closing but UI Automation still lists it and its icons,
  so the test waits for it to close and opens it again. It left-clicks the icon
  (expects the dashboard and no menu) and right-clicks it for the menu.
- Reads the tray menu through the Win32 menu handle (`MN_GETHMENU`), because UI
  Automation exposes none of its items. It clicks items at their
  `GetMenuItemRect` position.
- Clicks the real title-bar close button, located with `WM_NCHITTEST` = `HTCLOSE`
  (the title bar is not in the UI Automation tree), and also sends a programmatic
  close request (`WindowPattern.Close`). It does not send Alt+F4, which would close
  another app if focus moved.
- Launches second instances while the dashboard is hidden, visible and minimized,
  5 sequentially and 4 concurrently, and checks exit code 0, one process (same
  pid), one window, one tray icon.
- Hides, quits from the tray, checks exit code 0, no process and no tray icon left,
  then relaunches a fresh instance and quits it, and relaunches once more (the
  persistence checks above).

Before it starts sotyware it checks that it runs on the desktop that receives
input (`OpenInputDesktop` names the same desktop as its own thread's) and that
this desktop has a taskbar (`Shell_TrayWnd`). If not (a locked or disconnected
session, a service, a private desktop, an SSH session) it prints `ABORT` with the
reason and exits 3 without running any check, instead of reporting failures that
say nothing about sotyware. Verified 2026-11-10 by running it on a private
desktop: exit 3, no sotyware process started.

Results: `PASS`/`FAIL` per check, `INFO` for observations Windows may legitimately
vary (whether the dashboard ended up in the foreground), then a summary line. Exit
code 0 if all passed, 1 if any failed, 2 if the binary is missing or a sotyware
from the same path is already running, 3 if there is no interactive desktop. It does not catch an icon that is in the
tray but invisible to a person; the [manual check](#manual-check-real-desktop-linux-or-windows)
still covers looking at it.

Verified that it detects a broken button: with `hideDashboard` changed to do
nothing, all three hide checks failed (2026-11-10). Verified that it detects a
missing feature permission: a build without `allow-set-feature-enabled` in the
capability failed exactly the 5 switch checks, 85 passed / 5 failed (2026-11-11,
`3a8a715`'s script).

The count is 136 checks since 2026-11-18 (the theme across a quick quit:
the choice, the check after relaunch 2, WebView2's processes ended at each
of the three quits and none left at cleanup), 130 since `ec27b17`
(2026-11-16, the title bar, placement and frame checks), 99 since `a7bf8f3`
(90 since `3a8a715`, 80 before). Of the
10 feature checks added at `3a8a715`, "Running again after relaunch (not
persisted)" was replaced at `a7bf8f3` by "still Disabled (persisted)", and 9
persistence checks were added. The isolated-desktop test below has no feature
checks; it starts sotyware with whatever `preferences.sqlite3` exists and does
not switch any feature, so it changes no stored choice.

## Windows isolated-desktop test

```powershell
npm run build:test-profile
npm run test:desktop:windows:isolated              # = powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-isolated.ps1
npm run test:desktop:windows:isolated -- -Exe <path>
```

`tests/desktop/windows-isolated.ps1` creates a new desktop (`CreateDesktop`) in the
session's window station and starts a copy of itself on it (`CreateProcess` with
`lpDesktop`). That copy starts sotyware, which inherits the desktop, and runs the
checks; the launcher prints the copy's output and returns its exit code. The
desktop is never switched to, so nothing appears on your screen, and the test
sends no mouse or keyboard input: you can keep working while it runs (about
40 seconds).

It checks, against the real release binary and WebView2: startup (one process,
one visible `Tauri Window` titled sotyware); the "Hide dashboard" button read and
invoked through UI Automation inside WebView2 (hidden, process alive, window not
destroyed); second launches while hidden, visible and minimized (exit 0, same
pid, one window, shown or restored); `WM_CLOSE` (what ✕ and Alt+F4 send) and UIA
`WindowPattern.Close` hide instead of quitting; 3 sequential and 4 concurrent
launches; Hide still works afterwards. It records the primary's stderr as INFO.

What it **cannot** cover, and why: a private desktop has no taskbar, so
`Shell_NotifyIcon` has no notification area to add to. sotyware still starts in
tray mode, because `tray-icon` keeps its window and waits for Explorer's
`TaskbarCreated` message (which never comes there), so the Hide button exists,
but there is no tray icon, tray menu, left click or Quit to test. The test ends
sotyware with `Stop-Process` (cleanup, not a check of Quit). Real mouse clicks,
focus and the title-bar ✕ position are not tested either. Those stay in the
[Windows desktop test](#windows-desktop-test).

It is **not headless**: it needs a logged-in session (window station `WinSta0`)
to create the desktop and windows in, but not an unlocked or visible screen.
Before any check, the copy verifies that it really runs on the private desktop
and exits 3 otherwise. Exit codes: 0 all passed, 1 a check failed, 2 binary
missing or already running, 3 the private desktop could not be created or used,
4 not run because the session is elevated, 124 timeout (`-Timeout`, default
180 s).

**It cannot run elevated.** When the WebView2 host process has an administrator
token, WebView2 starts its browser process (`msedgewebview2.exe`) de-elevated
through `explorer.exe`. That process inherits Explorer's desktop,
`WinSta0\Default`, not the private one, and cannot attach to the dashboard
window there: `CreateCoreWebView2Controller` fails with `0x80070578` (Invalid
window handle), Tauri's `setup` panics ("failed to create webview") and, with
`panic = "abort"`, the process exits `0xC0000409` (-1073740791). Every later
launch then becomes a new primary and fails the same way. Seen on Windows 11
25H2 on 2026-11-11 ([status.md](../status/status.md#windows-11-isolated-desktop-test-elevated-2026-11-11)):
elevated on the Default desktop WebView2 renders, so this is not a sotyware
failure, and the private desktop's DACL is the same elevated or not. The
launcher therefore prints `elevated: True|False` and, when elevated, prints
`NOT RUN` and exits 4 before creating the desktop or starting sotyware. Run it
from a non-elevated PowerShell. GitHub-hosted runners run jobs as administrator
([CI](#manual-desktop-workflows)).

The launcher creates the run's [test profile](#test-profiles-adr-027) and
removes it at the end; the copy on the private desktop checks the app's
acknowledgement before it touches the window. Until 2026-11-14 this test
left an empty `cloud-accounts.sqlite3` in `%LOCALAPPDATA%\com.sotyware.desktop\`;
it no longer touches that folder.

## Headless Windows desktop testing

Investigated 2026-11-10 on Windows 11 Home 25H2. What each part of
`windows.ps1` needs:

| Part of the Windows desktop test | Needs |
| --- | --- |
| Starting sotyware, the dashboard window, WebView2 rendering | a window station and desktop in a logged-in session |
| Reading and invoking the page through UI Automation; window state through `user32` | the same desktop as the test (`EnumWindows` lists only the caller's desktop; the test drives UIA from that desktop too) |
| Second launch, concurrent launches | the same desktop: the single-instance plugin hands over through a named mutex and `FindWindowW` + `WM_COPYDATA`, and `FindWindowW` searches only the caller's desktop |
| Tray icon, menu (`MN_GETHMENU`), left click, Quit, icon removed | Explorer's notification area (`Shell_TrayWnd`) on that desktop |
| Real mouse clicks (Hide, tray, title-bar ✕) | the **input** desktop: unlocked, not disconnected, not minimised away (RDP) |

Findings:

- Nothing that starts sotyware runs without a session: there is no Windows
  equivalent of Xvfb. `npm run test:headless` (category A) is the only headless
  Windows suite; it runs on Windows unchanged and needs no desktop.
- A private desktop in the logged-in session runs everything except the tray and
  real input, without touching the screen: that is the
  [isolated-desktop test](#windows-isolated-desktop-test).
- The tray and real-click checks need the interactive, unlocked desktop with
  Explorer. `windows.ps1` detects when it does not have one and exits 3 instead
  of reporting failures.
- Not tried: Session 0 (a service or a scheduled task set to run whether the
  user is logged on or not). Registering such a task was not permitted in the
  investigating session. Windows services cannot show UI and Session 0 has no
  Explorer, so the tray checks cannot run there in any case; whether WebView2 and
  the isolated-desktop test work there is unknown.
- GitHub-hosted Windows runners: the isolated-desktop test ran twice, in the
  manual, experimental workflow ([CI](#manual-desktop-workflows)). The first run
  could not render WebView2; the second reported NOT RUN (exit 4). The runner's session has an input desktop (`Default`) and a
  private desktop could be created, but the job runs elevated, which the test
  cannot support ([why](#windows-isolated-desktop-test)); it now reports exit 4
  there. The automatic CI uses these runners for category A only.

## CI

GitHub Actions, in `.github/workflows/`. The repository is **public**, so
GitHub's standard hosted runners cost nothing
([runners and cost](#runners-and-cost)). The headless workflows (`ci.yml` on
Linux and Windows, `macos.yml` on Apple Silicon and Intel) run on every
pull request to `main` and every push to `main`; the desktop tests and the
release build run only when started by hand. The automatic Linux jobs
(`ci.yml`'s and `isolation-guard.yml`'s) run on `ubuntu-22.04`, the runner
`release.yml` builds the Linux packages on (glibc 2.35), not on
`ubuntu-latest`, which GitHub is moving to Ubuntu 26.04;
`desktop-virtual.yml` and `release.yml`'s `prepare`, `rehearse` and
`publish` jobs stay on `ubuntu-latest`. Only `release.yml`'s publish
job (a real release, on a tag) can read the update signing key, from the
`release` environment, and publish a release, in this repository with its
own `GITHUB_TOKEN` (ADR-039); a dry run signs with a throwaway key.
`macos.yml`'s package job reads the Apple credentials from the
`macos-signing` environment when they exist; no pull request job sees a
secret. Nothing runs on a schedule. A workflow can be started by hand only
once its file is on the default branch (`macos.yml` and `release.yml` are
on `main` since `d9cf561`, 2026-11-15).

| Workflow | Trigger | Runner | Runs | Category |
| --- | --- | --- | --- | --- |
| `ci.yml`, job `headless (ubuntu-22.04)` | **automatic**: pull request to `main`, push to `main`; also manual | `ubuntu-22.04` | `npm ci`, `npm run check`, `npm run build`, `cargo fetch --locked`, `npm run check:rfd-patch`, `npm run test:rfd-patch-guard`, the release-binary and desktop-isolation guards' self-tests, the icon check's tests (`npm run test:icons`), the installer icon check's tests (`npm run test:installer-icons`), the update manifest tool's tests (`npm run test:update-manifest`), the release assembly script's tests (`npm run test:release-assemble`, Linux only), the test-profile helpers' tests, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, the same clippy for the test-profile build (`--features test-profile --target-dir target/test-profile`), `npm run test:headless`, then `cargo test --locked --features request-recorder --target-dir target/recorder` (the recorder and test-profile unit tests) | A. headless |
| `ci.yml`, job `headless (windows-latest)` | same | `windows-latest` | same commands except the recorder-build `cargo test`, native Windows (MSVC Rust, no WSL), the PowerShell test-profile helpers' self-test instead of the Python one; does not start sotyware | A. headless |
| `ci.yml`, job `account service (ubuntu-22.04)` | same | `ubuntu-22.04` | in `server/` (ADR-032, its own lock file): `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` | A. headless |
| `isolation-guard.yml` | **automatic**, only for changes to `tests/desktop/**`, the guard or the desktop workflows (which `ci.yml` skips) | `ubuntu-22.04` + `windows-latest` | Linux: `node scripts/check-desktop-isolation.mjs`, its self-test, the Python test-profile helpers' tests, and a check that every desktop workflow running a driver runs `check:desktop-isolation` and `build:test-profile` and no plain `tauri build`/`build:release`; Windows: `test-profile-selftest.ps1` (no Rust, no npm install, a few minutes) | A. headless |
| `desktop-virtual.yml` | **manual only** (`workflow_dispatch`) | `ubuntu-latest` | `npm run check:rfd-patch`, `npm run check:desktop-isolation`, `npm run build:test-profile` (ADR-027), `npm run test:desktop:virtual` | B. virtual display: **not a real Linux desktop** |
| `desktop-windows-isolated.yml` | **manual only**, **experimental** | `windows-latest` | `npm run check:rfd-patch`, `npm run check:desktop-isolation`, `npm run build:test-profile` (ADR-027), `npm run test:desktop:windows:isolated` | B″. private desktop |
| `release-build.yml` | **manual only**; passed on GitHub (runs 43643326406 at `369d0cb` and 43646814979 at `f9a0c71`, 2026-11-14) | `windows-latest` | `npm ci`, `cargo fetch --locked`, rfd and release-binary guards and their self-tests, `npm run build:release` (LTO release binary with the path remaps of ADR-028, `beforeBundleCommand` guard, MSI + NSIS), then each installer unpacked without installing (`msiexec /a`, `7z x`) and its `sotyware.exe` compared with the checked binary (equal apart from the bundle-type marker Tauri patches in per bundle) and checked with `--local-paths`; then `npm run check:icons`, `npm run test:installer-icons` and the installer icon check on the real artifacts (MSI `ProductIcon` and shortcuts, MSI image, NSIS setup program and reconstructed uninstaller, every `sotyware.exe`; [icons.md](icons.md#installer-icons)). Signs nothing; called by `release.yml` with `upload: true` it uploads the two checked installers as the `windows-installers` artifact, which `release.yml`'s publish job signs (ADR-036). Nothing published | release artifacts (no install) |
| `macos.yml` | **automatic**: pull request to `main`, push to `main` (same skipped paths as `ci.yml`); **manual**; or called by `release.yml` with `package: true` (ADR-037, ADR-039; failed one test on both architectures at `d9cf561`, passed at `ec42d41`: [status](../status/status.md#hosted-runs-at-ec42d41-macos-passes-2026-11-15)) | `macos-15` (Apple Silicon, M1) + `macos-15-intel` (Intel) | job `macOS tests` per architecture: native machine and rustc host checked (no Rosetta), `npm ci`, check, build, `cargo fetch --locked`, rfd guard, fmt, clippy (default and test-profile), `npm run test:headless`, the OS credential store round trip in a keychain made for the run; no secret. Job `macOS package` (only with `package`, environment `macos-signing`): `npm run build:release` without caches, binary architecture and `Info.plist` checks, Developer ID signing and notarization when the Apple secrets exist ([macos.md](macos.md#ci)) | A. headless + release artifacts |
| `release.yml` | **tag `vX.Y.Z`**; or manual, a **dry run** by default, on any branch (ADR-036, ADR-039; dry runs failed at `ecf4339` on one test and passed at `8125739` and, with the release key and every setting, at `f846bdd`; the first release, `v0.1.0`, run 43749158657, 2026-11-15: [status](../status/status.md#first-release-v010-2026-11-15)) | `ubuntu-latest`, `ubuntu-22.04` + the called workflows | prepare: versions (and tag), `updates.rs` names this repository; for a real release also the commit on `main`, no existing release, `update-key.pub` a real key, the `release` environment tag-only, no signing secret at repository level, unsigned Windows installers allowed (warnings in a dry run); `ci.yml`; `release-build.yml` (upload); Linux AppImage/deb/rpm; `macos.yml` (package); then **rehearse** (dry run: throwaway key, `scripts/release-assemble.sh`, artifact `release-dry-run`) or **publish** (environment `release`: tag re-checked, key probe, `release-assemble.sh`, draft with `--verify-tag`, files compared and downloaded back, pre-release, anonymous download verified, latest, endpoint checked; [release.md](release.md#publishing-a-release)) | release |
| — | not in CI | — | `npm run test:desktop:windows` and the [manual checklist](#manual-check-real-desktop-linux-or-windows) | C. real desktop |

Start a manual workflow from the repository's **Actions** tab (select the
workflow, **Run workflow**, pick the branch), or with the GitHub CLI:

```bash
gh workflow run desktop-virtual.yml --ref main
```

```bash
gh workflow run desktop-windows-isolated.yml --ref main
```

```bash
gh workflow run release-build.yml --ref main
```

```bash
gh workflow run macos.yml --ref main -f package=true
```

```bash
gh workflow run release.yml --ref main
```

The last one is a release dry run ([release.md](release.md#publishing-a-release)).
The last two run the macOS package job in the `macos-signing`
environment: create it first ([release.md](release.md#secrets)), or GitHub
creates it, without protection, when that job starts.

### Automatic headless workflow (`ci.yml`)

- **Triggers.** `pull_request` targeting `main` and `push` to `main` only. A push
  to any other branch runs nothing, so a pull request's commits are built once
  (by `pull_request`), not twice. Merging a pull request runs the workflow once
  more on `main`, which tests the merged result; direct commits to `main` are
  tested by that same `push` trigger.
- **Skipped paths.** A commit or pull request that changes only `docs/**`,
  `**/*.md`, `tests/desktop/**` or the two `desktop-*.yml` workflows starts no
  run: the headless jobs do not exercise any of them. Because the desktop
  isolation guard scans exactly `tests/desktop/**`, `isolation-guard.yml`
  runs it (and its self-test) for such changes (added 2026-11-14; before,
  a change to the drivers alone met no guard in CI). If branch protection ever
  makes this check required, such a pull request would wait for a check that
  never starts; drop `paths-ignore` or add a path filter job then.
- **Concurrency.** One run per pull request (or per branch for pushes); a newer
  run cancels the older one (`cancel-in-progress: true`). On `main` that means a
  quickly superseded commit may have no completed run; the newer commit's run
  covers it.
- **Jobs.** Linux and Windows are one matrix with `fail-fast: false`: they run
  in parallel and neither cancels the other. Timeouts: 30 min (Linux), 40 min
  (Windows).
- **Order.** `npm run build` runs before any Cargo command because the Tauri
  crate embeds `dist/` at compile time. `cargo fetch --locked` fails if
  `Cargo.lock` does not match `Cargo.toml`; `npm ci` does the same for
  `package-lock.json`. Right after it, before anything compiles,
  `npm run check:rfd-patch` fails if the resolved rfd is not the local
  patch (an unused `[patch]` is only a Cargo warning), and
  `npm run test:rfd-patch-guard` runs the guard against broken temporary
  copies of `src-tauri` (it re-resolves rfd from crates.io there; a few
  seconds).
- **Test-profile build.** Clippy runs a second time with
  `--features test-profile --target-dir target/test-profile` (build.rs
  refuses the feature in any other target directory), so the code only that
  build compiles (its identifier, the profile's paths, no normal credential
  namespace; ADR-027) is type-checked on both platforms. Added 2026-11-14:
  before, CI never compiled it.
- **Recorder and test-profile tests (Linux only).** After `test:headless`,
  the Linux job runs `cargo test --locked --features request-recorder
  --target-dir target/recorder`. build.rs gives each feature its own target
  directory, so each suite is a full extra compile; that is why they were
  left out of CI at first. `request-recorder` turns `test-profile` on and no
  code is compiled only without the recorder, so this one build runs both
  suites (440 tests locally on WSL2, 2026-11-14). Windows runs neither suite
  in CI (Windows runner time); its test-profile build is type-checked only.
  Added 2026-11-14; passed on GitHub in run 43641836906 (440 passed).
- **Linux packages.** Only the Tauri build libraries from
  [setup.md](setup.md#linux-system-packages); no display server, window manager,
  tray host or D-Bus session.
- **Toolchains.** Node 24 through `actions/setup-node`; Rust stable through the
  `rustup` preinstalled on the images (with `rustfmt` and `clippy`), not a
  third-party toolchain action.
- **Caches.** `actions/setup-node` caches the npm download cache (keyed on
  `package-lock.json`). `Swatinem/rust-cache` caches the Cargo registry and the
  compiled **dependencies** in `src-tauri/target` (it prunes sotyware's own build
  output). The Cargo cache is saved from `main` only; pull requests restore it
  but add no entries, so cache storage stays at roughly one entry per OS.
  Nothing else (binaries, `dist/`, whole build directories) is cached.
- **Security.** `permissions: contents: read`; `actions/checkout` with
  `persist-credentials: false`; no secrets are used, so pull request code has
  none to read; every action is pinned to a full commit SHA with its version in
  a comment. Pinned on 2026-11-10: `actions/checkout` v7.0.1, `actions/setup-node`
  v7.1.0, `Swatinem/rust-cache` v2.9.3. Update a pin by resolving the new tag to
  its commit (`git ls-remote --tags https://github.com/<owner>/<repo>`) and
  reviewing the release.
- **Failure semantics.** No step uses `continue-on-error`. Any failing command,
  missing package, compile error, warning under `-D warnings`, formatting
  difference or test failure fails the job. A green run means every command
  above ran and passed on both platforms.

### Manual desktop workflows

Neither runs on pushes or pull requests, and neither caches or uploads
anything. Each builds a release binary (`--no-bundle`, no installers).

- **`desktop-virtual.yml`** (Linux virtual display). Installs the build packages
  plus exactly what `virtual.sh` needs (`xvfb openbox python3-dbus python3-gi
  gir1.2-atspi-2.0 at-spi2-core dbus-x11`). `virtual.sh` already fails on any
  FAIL or SKIP, a missing dependency (exit 2) and a display that does not start
  (exit 3). The workflow additionally requires the summary line
  `140 passed, 0 failed, 0 skipped` (`EXPECTED_CHECKS` in the workflow; update it
  in the same commit that adds or removes a check; it stayed at 92 when
  `a9fb338` added 4 checks and was corrected to 115 at `b4918e1`), names the exit code in the
  run summary, and fails on anything else. A last step, run also when the
  test failed, waits up to 120 s for any WebKit helper still dumping core
  and prints what the runner's systemd-coredump recorded about WebKit's
  helpers, with the crashed thread's stack trace. It is virtual-display
  testing, not real Linux desktop validation (see
  [what it does not validate](#virtual-display-test)).
- **`desktop-windows-isolated.yml`** (Windows private desktop, **experimental**).
  Every non-zero exit code fails the job; the run summary says which: 1 a check
  failed, 2 binary missing or already running, **3 the runner could not create
  or use a private desktop, so the test did not run** (never a pass), **4 the
  job is elevated, so the test did not run** (never a pass), 124 timeout. With
  exit 0 it also requires `summary: 56 passed, 0 failed` (`EXPECTED_CHECKS`).
  Its first hosted run (Windows Server 2025, before the elevation check existed)
  failed: the primary panicked creating the WebView2 controller (`0x80070578`)
  and the second launches exited `0xC0000409`; the same failure reproduces on
  Windows 11 when the suite runs elevated ([why](#windows-isolated-desktop-test)).
  Its second hosted run (2026-11-10, Windows Server 2025 Datacenter 24H2 build
  26100.33674) printed `elevated: True` and ended with exit 4, NOT RUN: hosted
  jobs are elevated, so this workflow ends with exit 4 until the test can start sotyware non-elevated, which has not been
  tried. Its third run (2026-11-13, `f19dd8b`) again ended with exit 4 after
  the rfd guard and the release build passed; the step's process exit code
  is shown as 1 there, while the summary and error line give 4. It does not
  cover the tray, tray menu or Quit.
- **The real-desktop Windows test (`npm run test:desktop:windows`) is not in
  CI.** It needs an unlocked interactive desktop with Explorer's taskbar and moves
  the mouse. Running it automatically would need a self-hosted runner in an
  auto-logged-on session, which is a separate decision (security and approval)
  and is not configured.

### Runners and cost

Checked on 2026-11-15 against GitHub's documentation
([GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions),
[limits](https://docs.github.com/en/actions/reference/limits)). These
figures change; re-check them before relying on them.

- **Cost:** "Use of the standard GitHub-hosted runners is free and
  unlimited on public repositories." Every runner these workflows use is a
  standard one: `ubuntu-latest`, `ubuntu-22.04` (deprecated, removed
  2027-04-17; the Linux packages and the automatic Linux checks run there,
  [release.md](release.md#building-a-release)), `windows-latest`,
  `macos-15` (arm64, M1, 3 CPUs, 7 GB) and `macos-15-intel` (x64, 4 CPUs,
  14 GB). Larger runners are always billed; none is used. Until
  2026-11-15 the repository was private on GitHub Free ($12 of compute a
  month), which is why the desktop workflows are manual; they could now run
  automatically too.
- **Limits that still apply:** concurrent jobs per account (on Free 20 in
  all and 5 macOS jobs at a time, per GitHub's limits page; a release
  starts two Mac test and two Mac package jobs), job and workflow time
  limits, artifact and cache storage (10 GB of cache per repository; entries
  not used for 7 days are removed).
- **Intel Macs:** `macos-15-intel` is GitHub's last x86_64 macOS image,
  announced to be retired in 2027 ([macos.md](macos.md#architectures)).
- **Fork pull requests** run on the same free runners. The repository
  setting *Require approval for all external contributors*
  ([release.md](release.md#repository-settings)) keeps a stranger's pull
  request from starting runs without a maintainer's approval.

**Measured durations** (2026-11-13, runs at `f19dd8b`, first to last log
line of each job, while the repository was private;
[status.md](../status/status.md#runs-at-f19dd8b-2026-11-13)): with a
**cold** Cargo cache a headless run took **7 min 37 s on Linux and 17 min
23 s on Windows** (Windows Rust tests alone 97 s). A manual virtual-display
run took **9 min 7 s** (release build 7 min 49 s); a manual isolated-desktop
run **11 min 10 s** (release build 9 min 20 s), although it always ends NOT
RUN on hosted runners. At `d9cf561` (2026-11-15, a push;
[status](../status/status.md#hosted-runs-at-d9cf561-the-redesign-macos-and-release-work-pushed-2026-11-15)):
`headless (ubuntu-22.04)` took **9 min 46 s** with no Cargo cache,
`headless (windows-latest)` **7 min 2 s** with a partly restored one,
`account service` 43 s, and the first `macOS tests` **9 min 43 s on Apple
Silicon and 15 min 26 s on Intel**, both without a cache and both stopped
at a failing test before the Keychain round trip. At `ec42d41`, with the
Linux and Windows caches that run saved and macOS again without one
([status](../status/status.md#hosted-runs-at-ec42d41-macos-passes-2026-11-15)):
`headless (ubuntu-22.04)` **4 min 56 s**, `headless (windows-latest)`
**6 min 30 s**, `account service` 25 s, and `macOS tests` **10 min 52 s
on Apple Silicon and 19 min 19 s on Intel**, each to the end. The
`release.yml` dry run at `8125739` took **23 min** in all, set by the
Intel macOS package (22 min 29 s, a release build without caches); its
Windows installers took 9 min 20 s and its Linux packages 6 min 59 s
([status](../status/status.md#release-dry-runs-2026-11-15)); the one at
`f846bdd` 22.5 min (Intel macOS package 21 min 48 s, Windows installers
10 min 43 s, Linux packages 6 min 52 s;
[status](../status/status.md#release-settings-complete-and-the-final-dry-run-2026-11-15)).

Per run: open the run in the **Actions** tab; the summary shows each job's
duration (see
[viewing job execution time](https://docs.github.com/en/actions/how-tos/monitor-workflows/view-job-execution-time)).
Cache size: the repository's **Actions → Caches** page.

### What CI does not validate

- Anything a real desktop shows or does: real tray hosts (GNOME, KDE, XFCE,
  Windows' notification area), real clicks, focus, visual appearance, Wayland,
  GPU rendering, screen readers. The automatic jobs never start sotyware.
- The Windows tray, tray menu, left click and Quit (only `windows.ps1` on a real
  desktop, by hand).
- Windows 10, macOS on a real Mac (only hosted runners), release
  builds with LTO in the automatic jobs (only the manual and release
  workflows build `--release`), installing any package (`.msi`, NSIS,
  `.deb`, `.rpm`, AppImage, `.dmg`), Authenticode signing, installing an
  update.
- Real Linux desktops: the virtual-display workflow uses Xvfb, openbox and a
  stub tray host.

## Manual check (real desktop, Linux or Windows)

Use this on any environment listed as not validated in
[the validation matrix](../status/status.md#validation-matrix), and record the
result there.

Start sotyware from a terminal (`./sotyware 2>&1 | tee sotyware.log` on Linux)
so its startup and shutdown messages are kept. In tray mode sotyware itself
prints nothing; in dashboard-only mode it prints `no system tray available (…)`
and `running in dashboard-only mode; …`. Anything else from sotyware, the
webview or the desktop is worth recording. Note the desktop environment, its version, X11 or Wayland
(`echo $XDG_SESSION_TYPE`), and the tray host (for GNOME: whether the
AppIndicator extension is enabled).

1. Start sotyware once. Note the single tray icon: does it look right (not a blank
   or broken image), and does hovering show "sotyware"? Open its menu: exactly
   "Open Dashboard" and "Quit". Click "Hide dashboard" on the page. Expected: the
   dashboard disappears, the tray icon stays; tray → Open Dashboard brings back
   the same window.
2. Start it again (app menu, shortcut, or the binary/`.exe`). Expected: no second
   tray icon, and the existing dashboard comes to the front.
3. Click another app, then start sotyware again. Expected: the dashboard gets focus,
   or at least flashes in the taskbar if the OS blocks focus stealing.
4. Minimize the dashboard, start sotyware again. Expected: the dashboard is restored.
5. Close the dashboard with ✕, start sotyware again. Expected: it reappears.
6. Repeat a few times. Check the process list (Task Manager, or
   `pgrep -a sotyware`): always exactly one `sotyware` process.
7. Tray → Quit. Expected: the process is gone, the tray icon disappears, and the
   log shows no error. Start sotyware again: it starts normally.
8. Windows only: left-click the tray icon. Expected: the dashboard opens.
   Right-click shows the menu.
9. In Settings › Features, "Scheduled backups" is listed as Running with a
   checked box. Uncheck it: it shows Disabled and no red error. Check it again:
   Running. Uncheck it again, Quit, and start sotyware: it is still Disabled
   (the choice is saved). Check it, Quit, start: Running. No "Could not load the
   saved feature choices" warning appears. Repeat in dashboard-only mode
   (closing the dashboard quits there). The choice is stored in
   `preferences.sqlite3` in `~/.local/share/com.sotyware.desktop/` (Linux) or
   `%LOCALAPPDATA%\com.sotyware.desktop\` (Windows).
10. Cloud Storage (needs a test bucket and an application key restricted to
    it; on Linux a running keyring such as GNOME Keyring or KWallet). Follow
    "Cloud Storage": a heading, an explanation and "No storage accounts yet".
    "Add account": Backblaze B2 with region `eu-central-003` pre-filled; enter
    a name, the bucket, the key ID and the secret (both shown as dots); "Add
    account". Expected: the account is listed with provider, bucket, region and
    endpoint, and the key is in Credential Manager (`cmdkey /list |
    Select-String com.sotyware.desktop`) or Seahorse under
    `com.sotyware.desktop`, but not in `cloud-accounts.sqlite3`. If the keyring
    asks to unlock, the window stays responsive while the prompt is open.
    "Check connection": "Checking connection…", then **Connected** (a key
    restricted to the bucket must not show Invalid credentials or Access
    denied). Edit the name only: saved without asking for the key. Edit with
    "Replace access key" and a wrong secret: the check now shows **Invalid
    credentials**. Edit the bucket to one that does not exist: **Bucket
    unavailable**. Disconnect the network: **Network error**. Add a second
    account at the same provider: two separate entries. Remove one: the
    confirmation says the bucket and files are not deleted; afterwards the
    key is gone from the credential store and the bucket's files are still
    there in the provider's console. Quit and start sotyware: the remaining
    account is listed, "Not checked yet".

Dashboard-only mode (Linux desktops without a tray host, for example vanilla
GNOME without the AppIndicator extension):

11. Start sotyware. Expected: the dashboard opens, shows the note that no
   compatible system tray is available, and has no Hide button. No tray icon. The
   log names dashboard-only mode and the reason.
12. Minimize it, start sotyware again. Expected: the same dashboard comes back,
    still one process.
13. Close the dashboard with ✕. Expected: the process is gone
    (`pgrep -a sotyware` prints nothing), the shell prompt returns, and the exit
    status is 0 (`echo ${PIPESTATUS[0]}` after the `tee` command above).
14. Enable a tray host (for example the AppIndicator extension) and start
    sotyware again. Expected: tray mode, as in steps 1–7.

## Rules

- Never weaken, skip or delete a check to make a change pass. If a check is wrong,
  fix it in its own commit and explain why.
- Report what was actually run, on which environment, and what remains unverified.
- When behaviour changes, update the desktop test and the manual checklist in the
  same change.

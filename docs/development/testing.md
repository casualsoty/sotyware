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
| Features section lists features and switches them; errors shown | ✅ mocked IPC | ✅ real page via AT-SPI: example listed as running, unchecked → disabled, checked → running (so the static capability allows both commands); listed in dashboard-only mode too | ✅ real WebView2 page via UIA: example listed with version and description, checked and Running; UIA toggle and real mouse click each disable and re-enable it; still Disabled after Quit and relaunch (persisted), Running after enabling, Quit and a second relaunch | ✅ (step 9) |
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


## Headless tests

```bash
npm run test:headless        # = npm run test:frontend && npm run test:rust
```

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
  `builtin()`: the example feature stays disabled, then enabled, across
  simulated restarts, and a corrupt file gives defaults, a save error, and an
  unchanged file.

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

## Cloud Storage UI end-to-end test

Opt-in, never in CI: the release binary's Cloud Storage page driven like a
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
| Linux `hang` | — (a Secret Service that never answers) | while the add waits: the Overview link and the example feature's switch still work, the process keeps running |
| Linux `browse`, Windows `-Mode browse` | — (Windows: **Connected**) | the bucket browser against the in-process listing server (`cargo test … serve_listing_for_ui -- --ignored`, started by the script in this checkout's `src-tauri`, made-up key): Open bucket → the root's folders and files with size; a folder with spaces and Unicode; the bucket breadcrumb; `photos/` without its marker or nested file; `photos/2024/` with a MiB size; Up one folder; a marker-only folder → "This folder is empty."; a folder of 1,005 entries → "1000 items shown; more are available", Load more → 1,005, no Load more; Refresh → 1,000 again; ← Back to accounts; remove. The server fails the phase if anything but `ListObjectsV2` (or `HeadBucket`) reached it |
| Linux `download`, Windows `-Mode download` | — (Windows: **Connected**) | downloads through the **native save dialog** from the in-process object server (`cargo test … serve_objects_for_ui -- --ignored`, made-up key; it writes the objects' sizes and SHA-256 next to its endpoint file): every file row has Download; the dialog opens with the key's last segment suggested, and cancelling it starts nothing; Save → "Downloaded …" with the SHA-256 matched and the file's SHA-256 equal to the server's; an existing file → the dialog's own replace question (GTK: "already exists" + Replace; Windows: "Confirm Save As" + Yes), the old file untouched until then, replaced afterwards; `docs/Bericht 2024 ü & co.pdf` (3 MiB) suggested unchanged and verified; a file without a stored SHA-256 → completed, "could not be verified"; a slow 16 MiB file → percent and "of 16 MiB", a hidden `.large.bin.*.sotyware-partial`, Cancel → "Download cancelled. Nothing was saved.", no file left; a connection cut after 256 KiB → failed with the network text, Try download again (no dialog) fails the same way; content that does not match its SHA-256 → integrity failure, nothing saved; at the end the download folder holds exactly the completed files. Linux only: closing the window during a download (dashboard-only mode quits) → exit 0 and no temporary file. Files go to a folder in the throwaway HOME (Linux) or a new folder under `%TEMP%` (Windows, removed afterwards). The server fails the phase if anything but `GetObject`, `ListObjectsV2` or `HeadBucket` reached it. On Linux the GTK dialog's name field is found through AT-SPI and the path typed with XTest (ASCII paths only); on Windows the common item dialog's File name box, Save and Cancel are not in its UI Automation tree (Windows 11 25H2), so they are found as child windows by control id (Edit `1001`, Button `1`, `2`) and driven with `WM_SETTEXT` / `BM_CLICK`, and the Yes of "Confirm Save As" with `TDM_CLICK_BUTTON`; any script error in this mode is a failed check and the account is still removed |
| Linux `upload` | — | upload batches through the **native GTK multi-file dialog** into the in-process **s3s-fs** server (`cargo test … serve_uploads_for_ui -- --ignored`, made-up key) behind a `ThrottledLink` taking 6 MiB/s towards the server, with the stub tray watcher of `virtual.sh` (**tray mode**). The server is seeded through `S3Store::upload` with `inbox/existing.txt`, `other/readme.txt` and a 24 MiB `downloads/big.bin`. The helper takes commands from `<endpoint file>.ctl` (`dump` → `.state.json` with every object's content SHA-256, size, `HeadObject` size and SHA-256 metadata, the open multipart files and the operations; `verify-denied`; `abort-denied`; `delete <key>`) and fails the phase if the app sent anything but listing, reads and upload requests. The dialog is driven by Ctrl+L, the folder path typed with XTest, Return, Ctrl+A in its `Files` table and its Open button pressed through AT-SPI. Ten batches ([results](../status/status.md#cloud-storage-uploads-through-the-release-binary-on-a-virtual-display-2026-11-12)): odd names, empty file, existing object and the Rust skip reasons, destination fixed while the browser moves; cancel during a PUT and Try again; cancel during multipart (aborted); refused abort (parts kept, retry cleans them); refused verification (needs inspection, no retry for it); a file changed while sent; a download beside an upload; ← Back to accounts during a batch; close (hide) and a second launch during a batch; tray Quit during multipart. Ends by deleting exactly the known keys and checking the bucket is empty. The phase is in `cloud_ui_upload.py`, run by `cloud_ui.py` |
| Windows `-Mode upload` | **Connected** | the same server, seed and state reading as Linux `upload` (`serve_uploads_for_ui`, a new temporary bucket per run; the script talks to the helper through the same `.ctl`/`.state.json` files), with the link set to **2 MiB/s** (`SOTYWARE_UI_UPLOAD_RATE`: UI Automation reads the page far more slowly than AT-SPI, and at 6 MiB/s a 40 MiB PUT ended before the first poll), through the **native Windows multi-file dialog** ("Choose files to upload", the common item dialog `#32770`): the folder is opened by putting its path into File name (Edit `1148`) and pressing Open (Button `1`), and the address bar (ToolbarWindow32 `1001`) must then name it; files are chosen in its file list through UI Automation (`UIItem`, SelectionItem `Select`/`AddToSelection`; the dialog then shows them as quoted names in File name) or by putting their quoted names into File name; Cancel is Button `2`. Batches: the dialog cancelled (no panel, nothing sent); odd names (Unicode, spaces, `+ & # ( )`), the empty file, an existing object, and **a file another process holds open without sharing** (Failed, "could not be read on this computer", nothing stored; Try again uploads it once released), with the browser moved to the root while the dialog is open; cancel during a single PUT and Try again; cancel during multipart; refused abort; refused verification (needs inspection, no retry for it); a file appended to while it is sent; a download through the save dialog beside an upload; ← Back to accounts during a batch; close (`WM_SYSCOMMAND SC_CLOSE`, as the title bar's X: hide in tray mode) and a second launch during a batch; **tray Quit during multipart with the real mouse** (right click on the tray icon in the "Show Hidden Icons" flyout, click on Quit, as `windows.ps1`); relaunch (the account still works); then **a file name with an unpaired surrogate** (NTFS allows it) chosen with a valid one: the check requires sotyware to keep running. Every batch's server state is checked; the end deletes exactly the known keys and checks the bucket is empty. Also checked: the window driven belongs to the started binary, nothing listens on the dev-server port 1420. Needs the unlocked interactive desktop with a taskbar (exit 3 otherwise) |
| Linux `b2`, Windows `-Mode b2` | **Connected** for the bucket-restricted key; after the rename still Connected; after replacing the key with a wrong secret **Invalid credentials** (so the replaced key is the one used); after putting the real key back Connected | Linux only: the bucket changed to a name the key may not use → **Access denied** or **Bucket unavailable**, not Invalid credentials, then back to Connected. Both: `backblaze_b2_fingerprint` before and after; the run fails if any object version under `sotyware-it/` changed |
| Linux `b2-browse`, Windows `-Mode b2-browse` | **Connected** for the read-only key | the [temporary dataset](#browsing-a-temporary-dataset-at-backblaze) in the real bucket: `backblaze_b2_browse_dataset` with `SOTYWARE_IT_B2_BROWSE_KEEP=1` creates and checks it (with the write key) and writes the manifest; the app gets the read-only key; Open bucket → root shows `sotyware-it`; down to the run folder; the run root, `docs/` → `2024/` → `q1/` → `deep/` → `er/`, the `2024` breadcrumb, Up one folder, both marker-only folders (empty), `photos/` (no marker row), `special/` and its folder `f+o&l?d#e%r`, the Unicode folders, `dots/` → `..` (holds `escape.txt`) → Up, `double/` → (no name) → (no name), `many/` (1,000 rows and Load more → 1,010 rows, no Load more → Refresh → 1,000); each folder must show **every row exactly as the provider listed it** (name, size as the page formats it, date formatted with the page's own `toLocaleString` call run in `node`); ← Back, Open bucket again → root; remove. Fingerprint after creating = after browsing (the UI wrote nothing); then `backblaze_b2_browse_cleanup` (also on exit or interrupt), and the fingerprint before the dataset = after cleanup |
| Windows `-Mode b2-download` | **Connected** for the read-only key | one real download through the dashboard and the native save dialog, [described above](#downloading-through-the-windows-dashboard-at-backblaze) |
| Windows `-Mode local` | **Network error**, and Open bucket shows the network error with Try again | the real Windows Credential Manager (read with `CredRead` and `cmdkey /list:<target>`; `cmdkey` never shows secrets); the same `com.sotyware.desktop` lines in `cmdkey /list` before and after |

In `local` (both platforms) Open bucket must show "Could not load this
folder." with the network error and Try again; in `b2` it is pressed at the
root once and must show a listing or Access denied (the test key is limited
to `sotyware-it/`), then ← Back to accounts.

**Linux** (`tests/desktop/cloud-ui.sh [local] [hang] [browse] [download] [upload] [b2] [b2-browse]`, default `local
hang browse download upload`): Xvfb + openbox and a private `dbus-run-session`, a real
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
npm run tauri build -- --no-bundle
tests/desktop/cloud-ui.sh
```

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

**Windows** (`tests\desktop\windows-cloud-ui.ps1 [-Mode local|browse|download|upload|b2|b2-browse|b2-download]`):
UI Automation only (Invoke, Value and Toggle patterns): no mouse, no
keystrokes, but a logged-in session (`download` shows the real save dialog
on the desktop; it is driven through UI Automation too, not clicked).
**Exception: `-Mode upload` moves the mouse** (one right click on the tray
icon and one click on Quit, after opening the "Show Hidden Icons" flyout;
it may press Esc once to close the flyout) and needs the unlocked desktop;
do not use the mouse or keyboard while it runs (about 15 minutes). The existing `cloud-accounts.sqlite3`
is moved aside and put back, as in `windows.ps1`. `-Mode b2` reads the key
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

- the page asks for the mode once on mount and makes no other call;
- in tray mode, "Hide dashboard" sends exactly `plugin:window|hide` for window
  `main` once per click, a failing or denied call shows the error in a
  `role="alert"`, and a successful retry clears it;
- in dashboard-only mode, a `role="status"` note says that no compatible tray is
  available, the dashboard cannot be hidden, the window must stay open, and
  closing it quits; there is no button and no hide call;
- if `operating_mode` fails or returns an unknown value, the error is shown and
  there is no Hide button;
- `main.ts` throws without `#app`.

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
checks the navigation (the dashboard opens on the Overview with no cloud
command sent and the only button "Hide dashboard"; the "Cloud Storage" link
shows the section and back); the empty state with its Add account action;
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
npm run tauri build -- --no-bundle
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
    Features section lists the example feature as running with a checked
    checkbox (`CHECK_BOX` role, named "Example"); toggling it through AT-SPI
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
    be found); the example feature is listed as running here too; a second launch exits 0 and restores the minimized dashboard
    without a new window or process; a watcher started afterwards gets no
    registration (no retry); closing the dashboard exits 0 and leaves nothing.
  - 14: relaunch with the watcher back: tray mode again, close hides, Quit.
  - 15: watcher without a host (`--no-host`): dashboard-only mode, no item left
    served, close exits 0.
  - 17: feature preferences across real restarts (tray mode). Each restart is a
    tray Quit with exit code 0 and no process left, then a new process; the page
    is never just reloaded. The example feature (left enabled and saved by 12)
    is unchecked, sotyware quits, the database row (read with Python's
    `sqlite3`, read-only) says disabled, and the relaunch shows it disabled with
    no error and no storage warning in the log; it is checked again, sotyware
    quits, the row says enabled, and a second relaunch shows it running.
  - 18: a corrupt `preferences.sqlite3` (text): sotyware starts in tray mode
    with the example feature running (its default); the page shows "Could not
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
npm run tauri build -- --no-bundle
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
npm run tauri build -- --no-bundle
npm run test:desktop:windows              # = powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1
npm run test:desktop:windows -- -Exe <path>
```

`-Exe` defaults to `src-tauri\target\release\sotyware.exe`. To test an installed
copy, pass its path, e.g. `-Exe $env:LOCALAPPDATA\sotyware\sotyware.exe` for the
NSIS per-user install.

It needs only Windows PowerShell 5.1 (UI Automation and `user32` via `Add-Type`), no
extra modules. **It runs on your real desktop:** it moves the mouse and clicks the
tray icon, the tray menu, the dashboard's button and feature checkbox and its
title-bar close button, and
opens the notification area's "Show Hidden Icons" flyout. Do not touch the mouse or
keyboard while it runs (about 30 seconds).

What it does, and how:

- Finds the dashboard with `EnumWindows` (class `Tauri Window`, owned by a
  sotyware process). UI Automation is not enough: it does not list hidden windows.
- Clicks "Hide dashboard" inside WebView2 twice: once through UI Automation
  `Invoke` and once with a real mouse click at the button's position. Each time it
  checks that the window becomes invisible, is not destroyed, and the process stays.
- Reads the Features section through UI Automation: the heading
  (`features-heading`), the example feature's `CheckBox` (named "Example") and its
  `ListItem`, whose name is the item's text (version, state label, description).
  It checks the example is listed checked and Running, then switches it off and
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
  `cloud-accounts.sqlite3` was created; then follows "Overview" back to the
  Hide button and the features. It adds no account, so it never writes to
  the Credential Manager. An existing `cloud-accounts.sqlite3` is moved aside
  and put back like the preferences database.
- Checks that the choice survives real restarts: before Quit it disables the
  example again; the relaunch (a new process) must show it Disabled and
  unchecked, with `preferences.sqlite3` present; it enables it, quits (exit 0,
  no process left), and a second relaunch (another new process) must show it
  Running and checked. Windows does not let a test redirect `%LOCALAPPDATA%`,
  so an existing `%LOCALAPPDATA%\com.sotyware.desktop\preferences.sqlite3`
  (and its journal) is moved to `preferences.sqlite3.windows-test-backup` before
  sotyware starts, so the run begins with the defaults, and put back at the end
  (the run's own database is deleted). The script prints both steps. If a
  backup from an interrupted run is still there, it aborts with exit 2 instead
  of overwriting it; put it back by hand.
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

The count is 99 checks since `a7bf8f3` (90 since `3a8a715`, 80 before). Of the
10 feature checks added at `3a8a715`, "Running again after relaunch (not
persisted)" was replaced at `a7bf8f3` by "still Disabled (persisted)", and 9
persistence checks were added. The isolated-desktop test below has no feature
checks; it starts sotyware with whatever `preferences.sqlite3` exists and does
not switch any feature, so it changes no stored choice.

## Windows isolated-desktop test

```powershell
npm run tauri build -- --no-bundle
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

It does not move databases aside: sotyware creates `preferences.sqlite3` and
`cloud-accounts.sqlite3` in `%LOCALAPPDATA%\com.sotyware.desktop\` if they are
missing (both left empty, since the test switches nothing and adds no
account). Remove them afterwards if you had none before.

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

GitHub Actions, in `.github/workflows/`. The repository is **private on GitHub
Free**, so hosted-runner time comes out of a small monthly allowance (see
[GitHub Free budget](#github-free-budget)). Only the headless workflow runs
automatically; the desktop tests run only when started by hand. Nothing in CI
builds installers, uploads artifacts, runs on a schedule, uses secrets, or uses a
runner other than standard `ubuntu-latest` and `windows-latest`.

| Workflow | Trigger | Runner | Runs | Category |
| --- | --- | --- | --- | --- |
| `ci.yml`, job `headless (ubuntu-latest)` | **automatic**: pull request to `main`, push to `main`; also manual | `ubuntu-latest` | `npm ci`, `npm run check`, `npm run build`, `cargo fetch --locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `npm run test:headless` | A. headless |
| `ci.yml`, job `headless (windows-latest)` | same | `windows-latest` | same commands, native Windows (MSVC Rust, no WSL); does not start sotyware | A. headless |
| `desktop-virtual.yml` | **manual only** (`workflow_dispatch`) | `ubuntu-latest` | `npm run tauri build -- --no-bundle`, `npm run test:desktop:virtual` | B. virtual display: **not a real Linux desktop** |
| `desktop-windows-isolated.yml` | **manual only**, **experimental** | `windows-latest` | `npm run tauri build -- --no-bundle`, `npm run test:desktop:windows:isolated` | B″. private desktop |
| — | not in CI | — | `npm run test:desktop:windows` and the [manual checklist](#manual-check-real-desktop-linux-or-windows) | C. real desktop |

Start a manual workflow from the repository's **Actions** tab (select the
workflow, **Run workflow**, pick the branch), or with the GitHub CLI:

```bash
gh workflow run desktop-virtual.yml --ref main
```

```bash
gh workflow run desktop-windows-isolated.yml --ref main
```

### Automatic headless workflow (`ci.yml`)

- **Triggers.** `pull_request` targeting `main` and `push` to `main` only. A push
  to any other branch runs nothing, so a pull request's commits are built once
  (by `pull_request`), not twice. Merging a pull request runs the workflow once
  more on `main`, which tests the merged result; direct commits to `main` are
  tested by that same `push` trigger.
- **Skipped paths.** A commit or pull request that changes only `docs/**`,
  `**/*.md`, `tests/desktop/**` or the two `desktop-*.yml` workflows starts no
  run: the headless jobs do not exercise any of them. If branch protection ever
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
  `package-lock.json`.
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
  `119 passed, 0 failed, 0 skipped` (`EXPECTED_CHECKS` in the workflow; update it
  in the same commit that adds or removes a check; it stayed at 92 when
  `a9fb338` added 4 checks and was corrected to 115 at `b4918e1`), names the exit code in the
  run summary, and fails on anything else. It is virtual-display testing, not
  real Linux desktop validation (see [what it does not validate](#virtual-display-test)).
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
  tried. It does not cover the tray, tray menu or Quit.
- **The real-desktop Windows test (`npm run test:desktop:windows`) is not in
  CI.** It needs an unlocked interactive desktop with Explorer's taskbar and moves
  the mouse. Running it automatically would need a self-hosted runner in an
  auto-logged-on session, which is a separate decision (security and approval)
  and is not configured.

### GitHub Free budget

Checked on 2026-11-10 against GitHub's documentation
([Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions),
[billing and usage](https://docs.github.com/en/actions/concepts/billing-and-usage),
[limits](https://docs.github.com/en/actions/reference/limits),
[dependency caching](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)).
These figures change; re-check them before relying on them.

- **Included per month (GitHub Free, private repositories):** compute worth
  **$12 USD**, expressed as money rather than minutes; **500 MB** artifact
  storage (shared with GitHub Packages); **10 GB** cache per repository. The
  allowance resets at the start of each billing cycle.
- **Per-minute rates of standard runners:** Linux 2-core x64 **$0.006**,
  Windows 2-core **$0.010** (Windows costs about 1.7 times Linux), macOS $0.062
  (not used). $12 is therefore about 2,000 Linux minutes, or 1,200 Windows
  minutes, or any mix. The pages read do not state how a job's time is rounded.
- **When the allowance is used up:** without a payment method on file, "usage is
  blocked once you consume the included compute usage" until the next cycle.
  With a payment method, further usage is billed, limited only by any budgets
  you set. This project adds no payment method, budget or paid feature.
- **Public repositories** get standard hosted runners free; this repository is
  private, so that does not apply.
- **Cache entries** not accessed for 7 days are removed; beyond 10 GB the least
  recently used are evicted. Logs do not count toward artifact storage; runs,
  logs and artifacts are kept 90 days by default.

**What consumes minutes.** Only the workflows above: the two headless jobs on
every qualifying pull request update and every push to `main`, and the two
desktop workflows when started by hand. Local runs (`npm run test:headless`,
`npm run test:desktop:virtual`, the Windows scripts on your own machine) cost
nothing.

**Estimated cost** (not measured on GitHub: the first `ci.yml` runs passed, but
their job times were not recorded; replace with the real job times): a headless run takes roughly 4 min on Linux and 6 min on
Windows with a warm Cargo cache, about twice that cold, so about $0.08 warm to
$0.17 cold, i.e. roughly 70–140 headless runs a month. A pull request with
`n` pushes costs `n` runs plus one on merge. A manual virtual-display run is
roughly 10–15 Linux minutes ($0.06–0.09); a manual isolated-desktop run roughly
15–20 Windows minutes ($0.15–0.20).

**Monitoring usage.** Personal account: <https://github.com/settings/billing>,
then the **Usage** view filtered to Actions (or "Actions grouped by SKU"); GitHub
emails at 90 % and 100 % of the included amount. Per run: open the run in the
**Actions** tab; the summary shows each job's duration (see
[viewing job execution time](https://docs.github.com/en/actions/how-tos/monitor-workflows/view-job-execution-time)).
Cache size: the repository's **Actions → Caches** page.

### What CI does not validate

- Anything a real desktop shows or does: real tray hosts (GNOME, KDE, XFCE,
  Windows' notification area), real clicks, focus, visual appearance, Wayland,
  GPU rendering, screen readers. The automatic jobs never start sotyware.
- The Windows tray, tray menu, left click and Quit (only `windows.ps1` on a real
  desktop, by hand).
- Windows 10, macOS, release builds with LTO in the automatic jobs (only the
  manual workflows build `--release`), installers (`.msi`, NSIS, `.deb`, `.rpm`,
  AppImage), code signing.
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
9. In the dashboard's Features section, "Example" is listed as Running with a
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

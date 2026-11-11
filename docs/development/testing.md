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
  provider, missing credentials, a stored account that no longer validates),
  `EnvCredentials` (variable names, per-account lookup, missing or padded
  values, no secret in `Debug`/`Display`), `plan_upload` (single PUT,
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
  full transfer queue not delaying another. A measurement, not a check:
  `cargo test --release --lib cloud::tests::local_throughput -- --ignored --nocapture`.
- **Real providers**, opt-in only: [below](#cloud-storage-integration-tests).

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
    capability works); tray Open Dashboard restores it; Quit.
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
  `115 passed, 0 failed, 0 skipped` (`EXPECTED_CHECKS` in the workflow; update it
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

Dashboard-only mode (Linux desktops without a tray host, for example vanilla
GNOME without the AppIndicator extension):

10. Start sotyware. Expected: the dashboard opens, shows the note that no
   compatible system tray is available, and has no Hide button. No tray icon. The
   log names dashboard-only mode and the reason.
11. Minimize it, start sotyware again. Expected: the same dashboard comes back,
    still one process.
12. Close the dashboard with ✕. Expected: the process is gone
    (`pgrep -a sotyware` prints nothing), the shell prompt returns, and the exit
    status is 0 (`echo ${PIPESTATUS[0]}` after the `tee` command above).
13. Enable a tray host (for example the AppIndicator extension) and start
    sotyware again. Expected: tray mode, as in steps 1–7.

## Rules

- Never weaken, skip or delete a check to make a change pass. If a check is wrong,
  fix it in its own commit and explain why.
- Report what was actually run, on which environment, and what remains unverified.
- When behaviour changes, update the desktop test and the manual checklist in the
  same change.

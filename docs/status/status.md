# Implementation status

Last updated: 2026-11-11 (Cloud Storage bucket browsing: read-only folder listing in the app; headless, desktop and Cloud Storage UI suites on WSL2 and native Windows 11, read-only listing against real Backblaze B2). Update this file whenever
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
([details](#cloud-storage-bucket-browsing-2026-11-11)).

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
| Cloud storage backend (uploads, downloads and changes to objects not reachable from the app): provider profiles (Backblaze B2, Hetzner, any S3-compatible), storage accounts without secrets (validation, add/update/remove, SQLite `cloud-accounts.sqlite3`), credentials by account id in the OS credential store (Windows Credential Manager, Linux Secret Service; no fallback; environment variables only in test builds), account add/rotate/remove keeping configuration and credentials consistent, one S3 adapter on `aws-sdk-s3` (list buckets/objects, single and multipart upload, verified download, delete), end-to-end SHA-256, typed errors, a transfer queue per account | `src-tauri/src/cloud/`, `SqliteAccountStore` in `storage.rs` ([cloud-storage.md](../architecture/cloud-storage.md), [ADR-016](../architecture/decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter), [ADR-017](../architecture/decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store)) |
| System tray with "Open Dashboard" and "Quit" | `src-tauri/src/tray.rs` |
| Linux tray backend: `tray-icon`'s `ksni` (pure Rust, no libayatana-appindicator) | Linux-only `tray-icon` dependency in `src-tauri/Cargo.toml` |
| Hide-on-close lifecycle (tray mode) | `on_window_event` in `lib.rs` |
| Only explicit exits (tray Quit; close in dashboard-only mode); implicit exits blocked | `app.run` closure and `on_window_event` in `lib.rs`, `tray.rs` |
| Fatal exit (code 1) if the tray cannot be created for a reason other than a missing tray host | `setup` in `lib.rs` |
| Single instance via `tauri-plugin-single-instance` | `run()` in `lib.rs` |
| Minimal capability (`allow-operating-mode`, `allow-list-features`, `allow-set-feature-enabled`, one `allow-…` permission per cloud account command, and `allow-list-cloud-folder`; `core:window:allow-hide` added at runtime in tray mode only) and CSP | `capabilities/default.json`, `setup` in `lib.rs`, `tauri.conf.json` |

Automated tests per platform: the "Hide dashboard" button with a real mouse
click, and left-click-to-open, are covered only by the Windows desktop test
(`tests/desktop/windows.ps1`). On Linux the virtual-display test activates the
button through AT-SPI and reads the dashboard-only page the same way. Restoring
a minimized dashboard and focus on second launch run on Linux only under the
virtual display (Xvfb + openbox); WSLg skips them. Test categories:
[testing.md](../development/testing.md).

Nothing else exists: no product features (only the example feature and
cloud account management), no settings, no persistence other than the feature
choices and the cloud storage accounts, no file operations in the bucket
browser (it only lists), no upload, download or transfer UI, no backups, no
sync, no custom Tauri events, no feature-management UI, no third-party
plugins, and no custom command other than `operating_mode`, `list_features`,
`set_feature_enabled`, the six cloud account commands and
`list_cloud_folder`.

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
| `tests/desktop/run.sh` (WSLg), manual checklist step 9, `desktop-virtual.yml` on GitHub (expects 115 since `b4918e1`) | — | — not run |

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
| `ci.yml` on GitHub (Linux, Windows) and native Windows 11 with the fix | — | — not run |

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
| Headless suite and virtual-display test on GitHub Actions | headless: ✅ two automatic `ci.yml` runs passed, Ubuntu and Windows jobs. Virtual-display: ✅ `desktop-virtual.yml` passed, 92/0/0, superseding the 89/2/1 failure at `14a5de1` (driver bug, fixed in `ead47e7`); not a real desktop ([CI](#ci-github-actions-2026-11-10)) |
| Windows private-desktop test in Session 0 or on a GitHub-hosted runner | — Session 0 not tried. GitHub-hosted runner: not possible, hosted jobs are elevated; the manual workflow failed once and then reported NOT RUN, exit 4 ([CI](#ci-github-actions-2026-11-10)) |
| Windows tray, real clicks and Quit without an interactive, unlocked desktop | — not possible: `windows.ps1` needs the input desktop with Explorer |
| Headless suite on Windows 10 | — |
| Installing and running `.deb` / `.rpm` / AppImage | — |
| Feature preferences on Windows 10, real Linux desktops, installed copies; corrupt database on the real Windows binary | — (unit tests and the 2026-11-11 desktop runs only) |
| Bucket browsing at a real provider with objects: non-empty folders, subfolders, markers, odd keys, pagination, a key without a name prefix; Hetzner | — (Backblaze seen only refusing the root and listing an empty prefix, 2026-11-11; [details](#cloud-storage-bucket-browsing-2026-11-11)) |

To validate an environment, follow the manual checklist in
[testing.md](../development/testing.md#manual-check-real-desktop-linux-or-windows)
and update this table.

## See also

- [limitations.md](limitations.md): known limitations.
- [roadmap.md](roadmap.md): next steps.

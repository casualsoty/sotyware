# sotyware: session context

Read this at the start of every task. It is a summary; the linked documents hold
the details and win if anything here disagrees. Rules for agents:
[AGENTS.md](../AGENTS.md).

## What it is

A desktop app that runs in the background, lives in the system tray and opens
a dashboard window on demand. On Linux without a tray host it runs as a plain
window (dashboard-only mode). Built with Tauri 2 (Rust) and Svelte 5 +
TypeScript. Version 0.1.3, **released on 2026-11-19** as `v0.1.3`, with
the autoclicker and macros
([status](status/status.md#release-013-2026-11-19)), after `v0.1.2` on
2026-11-17, the first release offering sotyware accounts
([status](status/status.md#release-012-2026-11-17)), `v0.1.1` on
2026-11-16 ([status](status/status.md#release-011-2026-11-16)) and `v0.1.0`
on 2026-11-15
([status](status/status.md#first-release-v010-2026-11-15)). Code and builds target
Windows, Linux and macOS (Apple Silicon and Intel; macOS built and tested
only on GitHub's runners, ADR-037); the **releases target Windows 11
only**, and their installers have not yet been through the installation
test; Windows 10 is not validated, Linux
has not run on a real desktop and macOS not on a Mac a person used, so none
is claimed as supported
([release.md](development/release.md#support-scope)). Release binaries are
built with `npm run build:release`, which keeps the build machine's paths out
(ADR-028).

## What works today

**Gaming notes overlay** (2026-11-23, ADR-052,
[notes-overlay.md](architecture/notes-overlay.md), [user guide](user/notes-overlay.md);
[status](status/status.md#gaming-notes-overlay-2026-11-23); committed as
`4706140` on the branch `feat/gaming-notes-overlay`, **not pushed**): a second window, created at runtime while the built-in
feature `notes-overlay` runs (**off by default**): frameless, see-through
(20 to 100 %), always on top, moved, resized and collapsed from its own
title bar, remembering its place across monitors and scalings; several
Markdown notes with checklists, tabs, search and pins, autosaved in
`notes.sqlite3`; **interactive** or **passive** (clicks go through, never
takes the keyboard, never stuck: the mode shortcut, Make interactive, a
restart); global shortcuts Ctrl+Shift+F1 to F5 (show/hide, mode, new
note, previous/next) through a second instance of the autoclicker's
backend (Windows, X11), the two features sharing who holds which chord
(`automation::claims`); Settings › Notes overlay; 19 commands (148 in
all). On Windows sotyware shows, hides and styles the window itself (a
tool window, `SWP_NOACTIVATE`): tao's `SW_SHOW` activates a window even
with `WS_EX_NOACTIVATE`, which a probe and the Windows driver showed.
Tested headless (WSL2; Rust also on native Windows 11), with real input
on the private display of `cloud-ui.sh overlay` (57 checks; nine of nine
runs after two fixes of where it reopens), and by
`tests/desktop/windows-overlay.ps1` on the maintainer's Windows 11:
unattended on a private desktop (50 of 50 four times after the keyboard
fix) and, with their go-ahead, with real input on their desktop against a
window of the driver playing the game (2026-11-24: 80 of 80 three times
after fixes of the driver only). Not run: hosted CI, macOS, real Linux
desktops, Wayland, Windows 10, a real game.

**Autoclicker and macros** (2026-11-17, ADR-050,
[automation.md](architecture/automation.md), [user guide](user/automation.md);
[status](status/status.md#autoclicker-and-macros-2026-11-17)): the
**Autoclicker** section clicks automatically (buttons, single or double,
1 ms to 24 h intervals with optional randomness, at the pointer, fixed
positions, lists or areas, limits, batches), records mouse and keyboard
input into **macros** with their timing (filters, sampling, never
sotyware's own window or the stop shortcut), edits them (timeline,
preview without input, paged action table, bulk changes), plays them
(repetitions, speed, jitter, sections, pointer- or window-relative
positions, screens mapped, target program checks, pause points), keeps a
library (folders, favourites, per-macro shortcuts, import/export as
versioned JSON) and **profiles**, all from **global shortcuts**
(suggested F6 toggle, F7 pause, **F8 emergency stop**, F9/F10 recording,
Shift+F11 play). Rust sends and records all input through native APIs
(Windows: SendInput, low-level hooks, RegisterHotKey; Linux X11: XTest,
RECORD, XGrabKey; refused on Wayland; none on macOS), one session at a
time, only while the built-in feature `autoclicker` runs (**off by
default**); every held key and button is released on stop, pause, end,
error, quit and panic; fail-safe corners; sotyware's own window is never
clicked. `automation.sqlite3`; 29 commands; the page is its own JS chunk.
Tested headless (engines on a mock backend and a fake clock), the X11
backend with real input on a private Xvfb, the app through its page with
real input on the private display of `cloud-ui.sh autoclicker` (whose
first runs found and fixed WebKitGTK field names and two fail-safe
defects), the Windows build natively (unit tests, clippy), and the
Windows backend with real input on the maintainer's Windows 11
(2026-11-18, opt-in `automation::windows::desktop`, into its own window:
three passes after fixing moves that landed a pixel off and the test's
own defects;
[status](status/status.md#the-autoclickers-windows-backend-with-real-input-2026-11-18));
the page driven on Windows on 2026-11-19 (the maintainer's desktop; the
second run 101 of 102 checks, the emergency stop with Shift held and every
release of held keys among them; the one failure, a shortcut field's
refusal without a name for UI Automation, fixed the same day;
[status](status/status.md#the-autoclicker-page-on-windows-11-the-drivers-first-runs-2026-11-19)).
Pushed as the branch
`feat/autoclicker-macros` on 2026-11-18: at `423605f`, `ci.yml` (Ubuntu
22.04, Windows), `macos.yml` (both Macs, the automation code's first
macOS build), `isolation-guard.yml`, `release-build.yml` and
`desktop-virtual.yml` (139/0/0) passed, started by hand
([status](status/status.md#hosted-runs-at-423605f-the-autoclicker-branch-passes-on-every-platform-2026-11-18)).
**On `main` since 2026-11-19** (a fast-forward to `37581df`):
`ci.yml`, `macos.yml` and `isolation-guard.yml` passed at the first
attempt
([status](status/status.md#hosted-runs-at-37581df-the-autoclicker-on-main-2026-11-19)).
**Released in `v0.1.3`** the same day
([status](status/status.md#release-013-2026-11-19)).
The page's manual check is automated since (2026-11-18,
[testing.md](development/testing.md#the-pages-former-manual-check-automated)):
the Linux phase grew to 103 checks on its private display, and
`tests/desktop/windows-autoclicker.ps1` does the same on the real Windows
desktop (run 2026-11-19, above). The new checks found two defects, both fixed: F8
did not stop a macro that held Shift (the system matches modifiers
exactly; while a session runs the emergency stop is now registered with
every other modifier combination too), and WebKitGTK hid the page's
hints, units and shortcut labels from screen readers (plain `<span>` flex
items; now paragraphs)
([status](status/status.md#the-autoclickers-manual-check-automated-2026-11-18)).

**Folder sync** (2026-11-16, ADR-043, [folder-sync.md](architecture/folder-sync.md);
[status](status/status.md#folder-sync-2026-11-16)): a storage account's
bucket kept in step with a folder the user chose (any drive; native folder
dialog opened by Rust; the page shows the path, never sends one), in both
directions, in the background while the built-in feature `folder-sync`
runs: file watcher (`notify`) plus a listing every 5–60 minutes (15 by
default), a three-way comparison against the synced state
(`folder-sync.sqlite3`; scans and listings in TEMP tables), live checks
before every change (`S3Store::upload_with` replaces only the synced ETag),
moves as server-side copies or local renames, conflicts kept twice
(`name (conflicted copy …)`) and settled later, deletions into the sync
trash (`.sotyware-sync/trash`, 30 days) or as delete markers behind the
deletion guard, a marker in the folder (unplugged drive: wait; foreign
folder: stop), moving the folder (verified copies) or starting fresh,
turning off without deleting anything; the bucket's **Sync** tab, a sync
glyph and summary in the sidebar, the local copy on Usage & costs.
Committed as `b67d6a6` and pushed: `ci.yml` passed (Ubuntu, Windows),
`macos.yml` failed on Apple Silicon only, two keys differing only in case
coming down as two files; fixed in `2c51283` (one spelling per name where
the file system ignores case, exact spellings in every check; reproduced
and fixed on WSL2 through `/mnt/c`;
[status](status/status.md#folder-sync-where-the-file-system-ignores-case-2026-11-16)),
and nothing is synced through a link any more (`d3a301c`; keys under a
link's name were downloaded behind it;
[status](status/status.md#folder-sync-never-goes-through-a-link-2026-11-16)).
Both pushed: at `d3a301c`, `ci.yml` and `macos.yml` passed on Ubuntu
22.04, Windows and both Macs, the case test included (one run each;
[runs](status/status.md#hosted-runs-at-d3a301c-folder-sync-passes-on-macos-2026-11-16)).
WSL2: `test:headless` (Vitest 307, Rust 651 + 29 ignored), recorder build
679, clippy (default and test-profile), the Sync tab seen in headless Edge
through a mocked harness. Since 2026-11-17 the desktop tests drive it in
the app (`cloud-ui.sh sync`, `windows-cloud-ui.ps1 -Mode sync`, with a
restart for the startup check; local server only; WSL2 virtual display
and native Windows 11;
[status](status/status.md#desktop-tests-for-folder-sync-sign-up-and-the-github-pages-2026-11-17)).
Not run: Windows junctions, a real provider, macOS.

**Desktop UX, usage and costs, snapshots, automatic sync** (2026-11-15,
committed on `main` after `310fd49` and pushed on 2026-11-16, where
`ci.yml`, `macos.yml` and `isolation-guard.yml` passed at `5d68c0f`;
**released in `v0.1.1`** that day;
[status](status/status.md#desktop-ux-usage-and-costs-snapshots-and-automatic-sync-2026-11-15)):
on Windows the dashboard draws its own title bar (drag region, Minimize,
Maximize/Restore, Close; native resizing, snapping, Alt+F4 kept) and
everywhere it opens at half the primary monitor's work area (at least
900 × 540), centred, then where the user left it (`window-state.sqlite3`; ADR-040). The bucket view
has tabs: **Usage & costs** measures a bucket by listing it (versions,
unfinished uploads; no provider API reports size or billing), keeps the
history (`storage-usage.sqlite3`) and estimates 30-day costs from price
lists in `src-tauri/pricing/` (Backblaze, Hetzner; read 2026-11-15),
labelled as estimates with components and assumptions; **Snapshots &
backups** explains Backblaze's console-only snapshots and manages
sotyware's backup snapshots of the bucket, including deleting one chosen
snapshot (ADR-041). An enrolled device syncs its vault at startup, on focus
(at most every 15 minutes) and after local account changes, with the state
by the sidebar's Account item (ADR-042). The `example` feature was removed
(a stored choice for it is ignored and kept). WSL2 only: `test:headless`
(Vitest 284, Rust 566, all guards), clippy (default and test-profile),
recorder build 594; the title bar and placement first ran on native
Windows 11 on 2026-11-16 (`windows.ps1` 130/0 with new frame checks,
`windows-isolated.ps1` 57/0;
[status](status/status.md#release-011-2026-11-16)); the Cloud Storage
drivers and no provider were run.
[Infrastructure proposal for accounts](development/server.md#production-deployment-proposal-2026-11-15).

**Redesign, macOS, GitHub Releases updates** (2026-11-15, committed as
`2e3972e` on branch `feat/desktop-redesign-macos-github-updates`, pushed
to `main` with `d9cf561`; [status](status/status.md#desktop-redesign-macos-and-github-releases-updates-2026-11-15)):
the dashboard is a desktop shell (sidebar with sections and storage
accounts, Overview with summaries, Settings with Appearance/Features/About
& updates, one update notification) on one token-based design system with
a persisted System/Light/Dark theme
([frontend.md](architecture/frontend.md), ADR-035); macOS support (login
keychain, menu bar with ⌘Q and the Edit menu, Dock click, separate Apple
Silicon and Intel builds, signing and notarization in CI when secrets
exist; [macos.md](development/macos.md), ADR-037); updates come from the
latest GitHub release's signed `latest.json`, with bounded HTTPS redirects,
platform entries for Windows, macOS and Linux, Restart now after in-place
installs, and `release.yml` publishing draft → pre-release → verified →
latest (ADR-036). **Releases are published in this repository**,
`casualsoty/sotyware`, which the maintainer made public on 2026-11-15
(ADR-039, superseding ADR-038's separate release repository; committed on
branch `feat/releases-in-public-repository` after `77b1923`, pushed with
`d9cf561`;
[status](status/status.md#releases-in-the-public-source-repository-2026-11-15)):
the app fetches
`https://github.com/casualsoty/sotyware/releases/latest/download/latest.json`
anonymously; `release.yml` publishes with its own `GITHUB_TOKEN`
(`contents: write` in the publish job only), the signing key lives only in
the `release` environment (tags `v*`) and reaches only the publish job's
signing step, dry runs sign with a throwaway key, the Apple credentials
live in `macos-signing`, and `macos.yml` runs natively on both Mac
architectures on every pull request to and push to `main`. The release key
file can never hold a fixture's or a private key. **`v0.1.0` is
published** (2026-11-15, run 43749158657: `publish` failed once on the
password secret, then passed; GitHub's latest release, immutable,
verified anonymously with the release key;
[first release](status/status.md#first-release-v010-2026-11-15)); one
update was installed through the app: the maintainer's Windows 11, 0.1.0
to 0.1.1 ([status](status/status.md#the-first-update-through-the-app-010-to-011)). Every release setting exists since 2026-11-15: the
release key (public half in `src-tauri/update-key.pub`, key id
`3A6E8F2D58D68E0D`; private half and password in the `release`
environment, created by hand that day like `macos-signing`;
[key](status/status.md#release-key-committed-2026-11-15)), the
`SOTYWARE_ALLOW_UNSIGNED_WINDOWS` variable, the `v*` tag and `main`
rulesets, the Actions settings and release immutability. Before the
release, `release.yml`'s dry runs failed at `ecf4339` on one test on the
Intel Mac, passed at `8125739` with `rehearse`, and passed again at
`f846bdd` with the key and every setting and no warning
([final dry run](status/status.md#release-settings-complete-and-the-final-dry-run-2026-11-15),
[readiness audit](status/status.md#release-rehearsal-readiness-2026-11-15)).
The maintainer runs `v0.1.0` on their personal Windows 11 and reports it
working ([status](status/status.md#v010-installed-by-the-maintainer-2026-11-15));
still to do: the full installation test on a disposable Windows 11, with
the published installers.
On this tree, WSL2 only:
Vitest 241, Rust 510 (recorder build 538), the release tools' tests,
actionlint with shellcheck clean, the virtual display 139/0/0 and the
Linux Cloud Storage phases (local 16, hang 8, browse 17, download 22,
upload 51, files 99, no failure). Tested earlier (2e3972e): Vitest 240,
Rust 507 (recorder build 535), the virtual display 139/0/0 and every Linux
Cloud Storage driver phase (files 99/99) against the real binary, after an
AT-SPI audit that found and fixed text WebKitGTK hid from screen readers; the release commands rehearsed with the real
Tauri CLI and a throwaway key. Not run then: `macos.yml` (first run at
`d9cf561`, below), `release.yml`, a Mac a person used, the Windows
drivers after the redesign. A **pre-push audit of the
release workflow** (2026-11-15, branch `fix/release-workflow-audit`,
pushed with `d9cf561`; [status](status/status.md#release-workflow-audit-before-the-first-push-2026-11-15))
made the promotion explicit and read back (pre-release with
`make_latest=false`, then `prerelease=false` and `make_latest=true`),
deletes a cancelled run's draft too, tightened the environment checks,
and fixed `ci.yml`'s concurrency group (a release dry run on `main` and a
push to `main` cancelled each other) and `update-manifest.mjs` exiting 0
unchecked when started through a symlink; the publish steps ran only
against a local stand-in for GitHub. The automatic Linux CI jobs then
moved from `ubuntu-latest` (becoming Ubuntu 26.04) to `ubuntu-22.04`,
where the Linux packages are built (glibc 2.35)
([status](status/status.md#linux-ci-jobs-on-ubuntu-2204-2026-11-15)).
**Pushed to `main` as `d9cf561`** (2026-11-15,
[hosted runs](status/status.md#hosted-runs-at-d9cf561-the-redesign-macos-and-release-work-pushed-2026-11-15)):
`ci.yml` passed, with the first build on Ubuntu 22.04 (Rust 510, recorder
build 538; Windows 496; Vitest 241 on each), and so did
`isolation-guard.yml`; `macos.yml`'s first run compiled cleanly on Apple
Silicon and Intel but failed one test on both: `test_profile`'s path
check missed a `..` spelling behind macOS's `/var` symbolic link. Fixed
in `ec42d41`, where `ci.yml` (Ubuntu 22.04, Windows) and `macos.yml`
(Apple Silicon, Intel: Rust 501 and the Keychain round trip) passed
([status](status/status.md#hosted-runs-at-ec42d41-macos-passes-2026-11-15)).

The desktop runtime, the foundation for built-in features, cloud storage
account management, bucket browsing, single-file downloads, upload batches
into the folder shown, and file management (ADR-026; audited 2026-11-14
and driven on the Linux virtual display and the native Windows 11 desktop
against local servers only, never at a real provider); the operation
journal and recovery view (ADR-030), backups (ADR-031), optional accounts
with end-to-end encrypted sync (ADR-032; service deployed 2026-11-17, ADR-045) and signed
updates (ADR-033; switched off in release builds until a release key and
update server exist), all tested headless against in-process servers only.
**No overwrite or other product features exist**. Desktop tests run only a
**test-profile build** that cannot reach the user's data (ADR-027).

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
  checkbox per feature. The features are `backup-scheduler` (ADR-031) and
  `update-check` (ADR-033); the `example` feature was removed on 2026-11-15
  (a choice stored for it is ignored and kept).
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
  the account if its key cannot be deleted. "Check connection" (also run
  once for every account when the dashboard starts, stopping at a locked
  credential store; ADR-046, 2026-11-17): `HeadBucket`
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
  name prefix is refused the root at Backblaze, so it cannot browse.
  **Download** per file (`download_cloud_file`, `cancel_cloud_download`,
  `retry_cloud_download`,
  [ADR-020](architecture/decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel)):
  Rust opens the native save dialog (`tauri-plugin-dialog`; GTK 3 / Windows
  common item dialog; the page never sends a path; the dialog asks before
  replacing an existing file, and only that file may be replaced); one
  download at a time (`busy` otherwise); the body streams into a hidden
  `.<name>.<pid>-<n>.sotyware-partial` next to the destination, is synced and
  checked (size; SHA-256 if sotyware stored one at upload, else "size only"
  is said; "transfer only" if no size was announced either), then hard-linked/renamed into place; progress on a per-call
  Tauri `Channel` (≤ 4/s, then one final completed/cancelled/failed); Cancel
  drops the stream and removes the temporary file, too late once the final
  move began (`finishing`); Try download again reuses the destination;
  leaving the browser or quitting cancels. Backend underneath: provider
  profiles as data, one S3 adapter on `aws-sdk-s3` with end-to-end SHA-256,
  multipart and a transfer queue per account. **Upload batches**
  (`cloud/upload.rs`, `upload_cloud_files`, `cancel_cloud_upload`,
  `retry_cloud_upload`, `CloudUploads`,
  [ADR-022](architecture/decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust);
  called by the browser's **Upload files…** and its panel
  `CloudUpload.svelte`, [cloud-storage.md](architecture/cloud-storage.md#the-upload-panel):
  destination captured at the click, per-file rows with stage, bytes and
  skip/error/remote notes, Cancel by batch id, Try again only for
  `retryable` files as a new batch, cleanup shown apart, final summary;
  stale events dropped by start, batch id and per-file `seq`; leaving the
  browser cancels, hiding does not): Rust opens the native multi-file dialog
  (paths never reach the page), up to 100 files checked before anything is
  sent (not a file, non-UTF-8 or control-character name, key > 1,024 bytes,
  too large, duplicate key → skipped with a reason), destination folder
  fixed when the dialog opens, one batch at a time in its own slot (a
  download can run beside it), files one after another, `HeadObject` before
  each (existing → skipped, never overwritten); per-file errors continue,
  credential/access/bucket/configuration/connection errors stop the batch;
  events `file`/`cleanup`/`finished` with `seq` on a per-batch channel, one
  final summary after the slot is free; cancel idempotent (current file up
  to its commit point, no further file); retry only failed/cancelled with
  nothing stored or not started, never `needs-inspection` or
  `incomplete-parts`; multipart ids whose abort failed kept in memory,
  aborted again on retry and on quit (≤ 35 s). Each file's backend
  lifecycle (`S3Store::upload`,
  [ADR-021](architecture/decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration)):
  existing key refused (HEAD; race documented), cancellable until one commit
  point (a PUT's held-back last chunk; before `CompleteMultipartUpload`),
  bytes re-hashed while sent (per part, per attempt) → `FileChanged`,
  failed multipart aborted in its own task (≤ 30 s; else "parts may
  remain"), `HeadObject` verification, 60 s stall timeout instead of the
  read timeout for PUT/UploadPart, progress `hashing`/`sending`/`verifying`. Recommended first provider: Backblaze B2, EU Central.
  [cloud-storage.md](architecture/cloud-storage.md#dashboard),
  [cloud-providers.md](architecture/cloud-providers.md), ADR-016, ADR-017.

- **File management** (both modes; ADR-026,
  [cloud-storage.md](architecture/cloud-storage.md#file-management)):
  new folder (marker), rename / copy / move / delete of files and folders
  and restore of deleted files as **reviewed plans** (`plan_cloud_operation`
  reads only; the review shows items, object counts, conflicts with a
  `name (n)` alternative and what a delete means on this bucket; then
  `start_cloud_operation`, events with `seq`, cancel, retry of what is
  left, `partial` states named). Folders are prefixes ending in `/`
  enumerated when planning (≤ 50,000 objects); copies are `CopyObject`
  with metadata and the planned ETag, verified by `HeadObject`; moves delete
  originals only after every copy is verified; deletes add delete markers on
  versioned buckets (restorable from Deleted items) and say which objects
  are gone for good; never overwrite or merge; nothing above 5 GiB is
  copied. Also: destination picker, search (names in Rust or key prefix at
  the provider, cursors in Rust), client-side sort and filters, selection
  with keyboard and context menu, internal copy/cut/paste, properties,
  previews (≤ 256 KiB text as text, raster images ≤ 10 MiB, never SVG/HTML;
  CSP `img-src data:`), presigned share links (1 h / 1 day / 7 days),
  several-item downloads into a picked folder (staging, `name (n)`, safe
  local names), folder uploads and drag and drop through the upload batch
  (≤ 50,000 files since 2026-11-21, paths kept in Rust), favorites and recent items per
  account (`cloud-places.sqlite3`).

- **Provider capabilities and conditional writes** (2026-11-15, ADR-029):
  every behaviour the storage code relies on is a capability with its
  evidence in `cloud/provider.rs`, shown in the account form;
  `If-None-Match: *` only where established (no shipped profile, so the
  HEAD-then-write race stays at Backblaze and Hetzner).
- **Operation journal and recovery view** (2026-11-15, ADR-030):
  `operations-journal.sqlite3`; uploads (with the multipart id), download
  temporary files, staging folders and file operations (every planned step)
  are recorded before they can change anything; at startup only sotyware's
  own recorded temporary files are removed; everything else appears under
  **Interrupted work** in Cloud Storage, inspected at the provider (reads
  only) and dismissed by the user; incomplete multipart uploads can be
  listed per bucket and aborted one by one. Nothing is repeated
  automatically. Local test servers only.
- **Downloads report three levels** (2026-11-15): `sha256-verified`,
  `size-only`, `transfer-only`.
- **Backups** (2026-11-15, ADR-031, [backup.md](architecture/backup.md)):
  jobs copying an account's bucket (or folder) into another account as
  content-addressed blobs plus one JSON manifest per run
  (`snapshots/<job>/…`); unchanged objects not transferred, identical
  content stored once; quick and full verification gate health (never
  "healthy" unverified); retention only after a verified, unsuspicious
  snapshot (mass-change check by content); restores to a folder or bucket,
  never overwriting, every file checked against its SHA-256; recovery file
  export; `scripts/recovery/sotyware-restore.py` restores with Python only
  ([user guide](user/recovery.md)); scheduler feature `backup-scheduler`;
  Backups page. Disaster scenarios tested against two in-process servers
  only; never at a real provider.
- **sotyware accounts and encrypted sync** (2026-11-15, ADR-032,
  [accounts.md](architecture/accounts.md)): service in `server/` (not
  deployed; [server.md](development/server.md)); desktop client
  `src-tauri/src/account/` and Account page. Storage accounts (with keys)
  and backup jobs sync inside a vault encrypted with XChaCha20-Poly1305
  under a random account key, wrapped by a recovery key (shown once,
  retyped to confirm) and sealed to each enrolled device's X25519 key; a
  password never unlocks it; new computers unlock with the recovery key
  or an approval with matching fingerprints; three-way merge with
  conflict copies; rollback refused; revoke + atomic key rotation. Tested
  end to end with the service in process and several device profiles.
  Since 2026-11-16 the service sends its mail through any SMTP relay
  (ADR-044). **Deployed on 2026-11-17** (ADR-045,
  [status](status/status.md#account-service-deployed-2026-11-17)) by
  following [server-setup.md](development/server-setup.md): one Google
  Cloud free-tier e2-micro VM in `us-east1`, Caddy with Let's Encrypt,
  Brevo as the relay, at `https://accounts.sotyware.com`, which is now
  `PRODUCTION_SERVER` (test-profile builds have none). Its database is
  backed up hourly, encrypted with age, to a Backblaze B2 bucket with
  Object Lock (30 days) and a healthchecks.io monitor (ADR-047; first
  upload and restore check on 2026-11-17). **`v0.1.2` (2026-11-17) is
  the first release that offers accounts**, with the
  [privacy notice](user/privacy.md) in effect; an external review is still
  open ([status](status/status.md#release-012-2026-11-17)).
- **Updates** (2026-11-15, ADR-033; source since ADR-036: the latest
  GitHub release, see above; [updates.md](architecture/updates.md)):
  a minisign-signed `sotyware-update.json` is verified (signature, format,
  channel, version floor against replays, HTTPS package entry with
  signature and SHA-256) before `tauri-plugin-updater` receives exactly
  it; the package's signature and signed version and its SHA-256 are
  checked before installing, only on **Update now**, never during a
  transfer, file operation or backup; Overview panel; automatic checks are
  the `update-check` feature; `scripts/update-manifest.mjs` makes and
  checks manifests in `release.yml`'s publish job, which publishes in this
  public repository (ADR-039). `src-tauri/update-key.pub` holds the
  release key's public half since 2026-11-15, so builds check GitHub's
  latest release, `v0.1.0` since that day and `v0.1.1` since 2026-11-16. Tested with test keys, a local server, the
  plugin on Tauri's mock runtime and a fixture signed by the real Tauri
  CLI; **one update installed for real**: the maintainer's Windows 11,
  0.1.0 → 0.1.1 through Update now ([status](status/status.md#the-first-update-through-the-app-010-to-011)); never on
  macOS or Linux.

## Repository at a glance

```
src/                 Svelte frontend (NotesOverlay.svelte, NoteView.svelte, NotesOverlaySettings.svelte + notesOverlay.ts, noteMarkdown.ts, noteAutosave.ts, overlayModel.svelte.ts, overlayMain.ts (the overlay's page, overlay.html); Autoclicker*.svelte + automation.ts, automationFormat.ts, automationModel.svelte.ts, HotkeyInput.svelte, PlaybackForm.svelte; main.ts, App.svelte, TitleBar.svelte, CloudUsage.svelte + cloudUsage.ts, CloudSnapshots.svelte, accountSync.svelte.ts, Features.svelte, features.ts, Updates.svelte + updates.ts, CloudStorage.svelte, CloudAccountForm.svelte, CloudBrowser.svelte (incl. downloads and the file manager), CloudUpload.svelte (upload panel), CloudOperation/CloudFolderPicker/CloudProperties/CloudBulkDownload/CloudDeleted/Modal.svelte, cloud.ts, cloudFiles.ts, fileList.ts, cloudMessages.ts, format.ts)
src-tauri/src/       main.rs → lib.rs (builder, operating mode, lifecycle, single instance, operating_mode command) + tray.rs + title_bar.rs + window_state.rs + storage.rs (SQLite)
src-tauri/src/features/  built-in features: mod.rs (trait, builtin list, commands), registry.rs, preferences.rs (stored choices), backup.rs, updates.rs, folder_sync.rs, autoclicker.rs, notes_overlay.rs
src-tauri/src/automation/  autoclicker and macros (ADR-050): mod.rs (Automation service, 29 commands), model.rs, keys.rs, timing.rs, input.rs (Backend trait, held input, mock), clicker.rs, recorder.rs, player.rs, native.rs, windows.rs, x11.rs, claims.rs (which feature holds which chord), tests.rs
src-tauri/src/overlay/  gaming notes overlay (ADR-052): mod.rs (NotesOverlay service, 19 commands), model.rs, placement.rs, host.rs (the window, built at runtime), focus.rs (Windows), tests.rs; storage in src-tauri/src/storage/notes.rs
src-tauri/src/cloud/     cloud storage: provider.rs, account.rs, credentials.rs, keyring.rs (OS credential store), s3.rs (+ s3/upload.rs, s3/hook_assumptions.rs), recorder.rs (feature request-recorder only), dashboard.rs (the six account commands), browser.rs (list_cloud_folder), download.rs (the three download commands), upload.rs (the upload batch commands, incl. folder uploads), operations.rs (+ operations/names.rs; file operations as plans), manage.rs (folders, properties, previews, search, deleted items, share links), bulk_download.rs, places.rs, drop.rs, s3/manage.rs, usage.rs (storage usage, ADR-041), pricing.rs (+ src-tauri/pricing/*.json); tests.rs, browser/tests.rs, download/tests.rs, upload/tests.rs, operations/tests.rs, manage/tests.rs, bulk_download/tests.rs + test_server.rs (in-process S3; test_server/versioned.rs: in-memory versioned S3), integration.rs (opt-in)
src-tauri/           tauri.conf.json, capabilities/default.json, Cargo.toml, icons/ (generated from icons/source/*.svg, npm run icons), patches/ (rfd patched locally, ADR-023)
src/*.test.ts        Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
.github/workflows/   CI: ci.yml (automatic, headless, incl. the rfd patch guard), desktop-*.yml (manual)
scripts/             check-rfd-patch.mjs (+ .test.mjs): fails unless Cargo builds rfd from src-tauri/patches/rfd; check-release-binary.mjs (+ .test.mjs): no test build and no build-machine path in a package; build-release.mjs (+ .test.mjs): npm run build:release, the release build with paths remapped (ADR-028)
tests/desktop/       Desktop tests of the release binary (run.sh: Linux/X11 simulated; virtual.sh: same on Xvfb + openbox; windows.ps1: real Windows desktop; windows-isolated.ps1: Windows private desktop; opt-in Cloud Storage end-to-end incl. browsing, downloads and uploads: cloud-ui.sh, windows-cloud-ui.ps1)
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
  `check_cloud_account`, `list_cloud_folder`, `download_cloud_file`,
  `cancel_cloud_download`, `retry_cloud_download`, `upload_cloud_files`,
  `cancel_cloud_upload`, `retry_cloud_upload` and the 22 file-management
  commands of ADR-026, the five recovery commands of ADR-030, the three
  usage commands of ADR-041, the eleven backup commands of ADR-031/ADR-041,
  the 20 account commands of ADR-032, the six update commands of
  ADR-033/ADR-036, the 16 folder sync commands of ADR-043,
  `open_github_page` of ADR-049, the 29 automation commands of ADR-050
  and the 19 overlay commands of ADR-052 (the notes ones only for the
  overlay's own window, capability `notes-overlay`)
  (148 in all; listed in
  [overview.md](architecture/overview.md#components-and-boundaries); one
  `allow-<command>` permission each; no `dialog:`/`fs:`/`updater:` permission) and, in tray
  mode only, `getCurrentWindow().hide()` (`core:window:allow-hide`, granted at
  runtime), `setTheme()` for the theme chosen in Settings
  (`core:window:allow-set-theme`), `isDecorated()`, and on Windows only the
  title bar's drag region, minimize, maximize and close (runtime capability
  `window-controls`, ADR-040); the notes overlay's window may start a drag
  or a resize from its title bar and corner (`notes-overlay`, ADR-052). No other custom command, no custom events (download progress
  and upload events use per-call channels). No command returns a stored
  credential or accepts a local path; only the folder sync commands return
  one, the sync folder the user chose (ADR-043).
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
npm run test:headless                      # rfd patch guard + release/isolation guards + frontend + Rust unit tests, no display needed
npm run check:rfd-patch                    # fails unless Cargo resolves rfd from src-tauri/patches/rfd
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run build:test-profile && tests/desktop/run.sh   # desktop tests need a test-profile build (ADR-027); or: npm run test:desktop:virtual
npm run test:desktop:windows               # Windows, real desktop, moves the mouse
npm run test:desktop:windows:isolated      # Windows, private desktop, unattended (no tray)
```

**WSL:** non-interactive shells may pick up Windows `npm` from `/mnt/c` and not find
`node`/`cargo`. Fix `PATH` first: [setup.md](development/setup.md#wsl2-notes).
More: [workflow.md](development/workflow.md), [testing.md](development/testing.md).

## Tests

- **Gaming notes overlay** (2026-11-23, `4706140` on a branch, not pushed,
  [status.md](status/status.md#gaming-notes-overlay-2026-11-23)): unit and
  frontend tests in `test:headless`; the Linux phase `cloud-ui.sh overlay`
  (57 checks) found four defects, all fixed (the collapsed window 200 px
  high under GTK, every note tab read as selected and text hidden from
  screen readers under WebKitGTK, the overlay reopening at 0, 0 after
  openbox's or tao's first position report);
  `windows-overlay.ps1` found two on Windows, fixed (a taskbar style, and
  a show that activated the overlay: tao's `SW_SHOW`); with real input on
  the maintainer's desktop it passed 80/80 three times (2026-11-24), after
  three first runs that found defects of the driver only.
- **Whose WebKit helper the Linux driver finds left** (2026-11-19, test
  code and docs only,
  [status.md](status/status.md#the-linux-driver-tells-whose-webkit-helper-is-left-2026-11-19)):
  the helper that failed `desktop-virtual.yml` at `1ef6bfa` was not the
  quitting instance's but an earlier launch's, very probably scenario 12's
  web process, which ran on for 22 to 25 s. Every launch now carries its
  label in its environment, which WebKit's helpers inherit; a quit waits
  only for its own launch's helpers (its network process among them), a
  new check before 17 requires that none of an earlier launch is left,
  and a helper left is named with its program, parent, state, start and
  launch, its threads and kernel wait channels printed. 140 checks.
  Pushed as the branch `test/whose-webkit-helper-is-left`; its first
  hosted run (`8b1deac`) failed 138/2 and showed why: **WebKit's web
  process crashed as sotyware quit** (glibc: a corrupt heap) and was still
  dumping core into the runner's systemd-coredump 15 s later, in 2 of 14
  instances; never on WSL2 (42 of 42 ended with 0). The driver now also
  says when a helper is dumping core, and the workflow prints
  systemd-coredump's record. In five hosted runs (two more were cancelled
  on the Ubuntu archive's timeouts), two had crashes (4 of 70 web
  processes), and the stack traces show where: glibc aborts in a C++
  static destructor of Mesa 26.0.8's `libgallium` as the process exits,
  the heap already corrupt; WebKit's and Mesa's code, not sotyware's.
  Since then the driver counts a helper dumping core as ended and names
  the crash without failing (a crashed network process still fails), and
  the workflow puts the number of WebKit core dumps in the run summary.
  Three hosted runs at `c02ea7e` passed, one of them naming a real crash
  ([status](status/status.md#hosted-runs-at-c02ea7e-a-crash-at-quit-named-the-run-passing-2026-11-19),
  [limitation](status/limitations.md#dashboard-adr-035)).
- **The theme check on Windows** (2026-11-18/19, test code and docs only,
  [status.md](status/status.md#the-theme-check-on-windows-webview2-after-a-quick-quit-2026-11-18)):
  `windows.ps1` now chooses Dark just before a tray Quit and checks it
  after the relaunch, as the Linux driver does; every relaunch waits for
  the quitting instance's WebView2 processes (`msedgewebview2.exe` naming
  the run's test profile on its command line) and records from their exit
  times how long they outlived sotyware (136 checks). Checked statically
  and in a scratch harness (37/37). A probe on a private desktop (sotyware
  ended with `Stop-Process`, no tray there) found WebView2 commits a
  choice 5 s after it, or at once when sotyware ends, and lost none, also
  with a relaunch 0.13 s after the choice. On the maintainer's Windows 11
  desktop (2026-11-19) `windows.ps1` passed **136/0** once: Dark chosen
  3.9 s before the tray Quit was kept, through WebView2's commit as
  sotyware ended, and WebView2's processes ended 56 to 76 ms after
  sotyware. No limitation on Windows; on `main` since 2026-11-19.
- **The theme check after a quick quit and relaunch** (2026-11-18, test
  code and docs only,
  [status.md](status/status.md#the-theme-check-after-a-quick-quit-and-relaunch-2026-11-18)):
  `desktop-virtual.yml` failed once at `0b79903` on "the theme chosen
  before quitting (Dark) is still chosen". WebKitGTK commits a
  `localStorage` write 0.5 s after it, or in its network process just
  after sotyware has ended, so a quit right after the choice keeps it,
  but the driver's relaunch 0.2 s later can read before that commit
  (holding the helper's commit back 2 s gives the same failure). The
  driver now waits for WebKit's helpers of the quitting instance to end
  (at most 15 s; they take about 0.1 s); the product side is a
  [limitation](status/limitations.md#dashboard-adr-035). WSL2 virtual
  display 139/0/0 three times, `test:headless` passed. On `main` since
  2026-11-19; its first hosted run, `desktop-virtual.yml` at `1ef6bfa`,
  kept the theme but failed the new wait once: a WebKit helper of the
  run's test profile outlived it after the first Quit, an earlier
  launch's, as found since (entry above;
  [status](status/status.md#every-branch-merged-into-main-hosted-runs-at-1ef6bfa-2026-11-19)).
- **Usage & costs tab in the Linux desktop test** (2026-11-16,
  [status.md](status/status.md#the-usage--costs-tab-in-the-linux-desktop-test-2026-11-16)):
  `cloud-ui.sh usage` measures a seeded versioned bucket through the UI
  and checks at three window sizes that nothing on the Usage & costs and
  Snapshots & backups tabs reaches past its section or `<main>`; it fails
  a build without the cost table's wrap fix. Its first runs found two
  defects, both fixed: a measurement never finished on the page (the last
  event's `save_error`; **in `v0.1.1`**) and the tiles' figures were
  missing from the accessibility tree. WSL2 virtual display, 21/21
  three times; and on native Windows 11 the same day,
  `windows-cloud-ui.ps1 -Mode usage` (UI Automation, no mouse): 41/41,
  its layout checks failing a build without the fix too
  ([status.md](status/status.md#the-usage--costs-tab-in-the-windows-desktop-test-2026-11-16)).
- **Two flaky tests after the release key push** (2026-11-15,
  [status.md](status/status.md#two-flaky-tests-after-the-release-key-push-2026-11-15)):
  recent items recorded in one millisecond were trimmed by name (fixed in
  the store: each is stored as newer than every other); the test server
  now reads a request it refuses with 412 to its end, which Windows lost
  under the reset connection. `f178d3b` passed everywhere once re-run,
  and the fixes (`f846bdd`) passed on every platform.
- **Late-part wait test on a busy runner** (2026-11-15, test code only,
  [status.md](status/status.md#the-late-part-wait-test-on-a-busy-runner-2026-11-15)):
  the first `release.yml` dry run failed only on the Intel Mac, where
  the ADR-025 test missed its 500 ms floor (483.6 ms); its cancel came
  from throttled progress updates. It now cancels from the test hook
  `TransferControl::handed_over` and checks at the abort's arrival that
  part 2 was stored; reproduced and fixed under CPU load on WSL2 (the old
  test 0/6 there, the new 6/6, its control 10/10; the new test fails with
  the wait switched off). Pushed as `8125739`: `ci.yml` and `macos.yml`
  passed on every platform, then the repeated dry run.
- **Upload tests on a single CPU** (2026-11-15, `25eb0f6`,
  [status.md](status/status.md#upload-tests-on-a-single-cpu-2026-11-15)):
  test hooks run in `block_in_place` on a multi-thread runtime, so a
  blocking hook no longer stalls a one-worker runtime; two test races
  fixed without loosening assertions. WSL2 pinned to one and two CPUs
  and unpinned: default 412, recorder 440 (27 ignored); `ci.yml` run
  43657354594 passed (Ubuntu 412 + 440, Windows 399, Vitest 179).
- **Hosted runs at `00c7872`** (2026-11-15,
  [status.md](status/status.md#hosted-runs-at-00c7872-and-the-upload-shutdown-test-race-2026-11-15)):
  `release-build.yml` (43649816381) passed, with the installer icon check
  7/7 on the real MSI, image and NSIS setup; `isolation-guard.yml`
  (43649813923) passed; `ci.yml` (43649807439) failed on Ubuntu twice on
  a race in the upload shutdown test (the final event is sent after the
  slot is freed); the test now waits for it. At `82b17c6` all three
  passed (`ci.yml` 43653244431, `isolation-guard.yml` 43653245468,
  `release-build.yml` 43653247634, icon check 7/7).
- **Installer icon verification** (2026-11-14, committed as `00c7872`,
  [status.md](status/status.md#installer-icon-verification-in-release-builds-2026-11-14),
  [icons.md](development/icons.md#installer-icons)):
  `scripts/check-installer-icons.mjs` reads the MSI (`ARPPRODUCTICON` →
  `ProductIcon`, shortcuts, administrative image), the NSIS setup program
  and the uninstaller it would write (solid LZMA decoded, icon patch
  applied in memory) and every `sotyware.exe`, and compares their icons
  with `icon.ico` byte for byte; run by `release-build.yml`, tests (28)
  in `ci.yml`. Native Windows 11 `build:release` + check passed, a
  stale-icon control failed everywhere; nothing installed or run. Passed
  on GitHub at `00c7872` (above).
- **Application icons** (2026-11-14, [status.md](status/status.md#application-icons-2026-11-14),
  [icons.md](development/icons.md)): placeholders replaced by a cube on a
  blue-to-teal tile from three SVG sources (`npm run icons`, check
  `npm run check:icons` in `test:headless`); NSIS setup and uninstaller
  icons set. WSL2 checks and `test:headless` passed; native Windows 11
  `npm run build:release` passed, MSI and NSIS unpacked without
  installing: every icon resource equals `icon.ico`, the packaged binaries
  are the checked one. Seen running (test-profile build, Windows 11 25H2
  at 100 %, [status](status/status.md#application-icons-in-the-running-app-on-native-windows-11-2026-11-14)):
  title bar, taskbar button and tray flyout show the cube, also after hide
  and reopen; Explorer, Start menu, other scalings and Linux not seen.
  Pushed; on GitHub at `f9a0c71` `ci.yml` (run 43646805839),
  `isolation-guard.yml` (43646812566) and `release-build.yml`
  (43646814979: MSI + NSIS unpacked, their `sotyware.exe` equal to the
  checked binary; icons not inspected there) passed
  ([status](status/status.md#hosted-runs-at-f9a0c71-icons-pushed-2026-11-14)).
- **Release hardening** (2026-11-14, uncommitted on `b1ae386`,
  [status.md](status/status.md#release-hardening-2026-11-14)): release
  builds go through `npm run build:release` (`--remap-path-prefix`,
  ADR-028); the bundling guard refuses a binary naming the build
  machine's paths (it refused the plain Windows build: 601 ×
  `C:\Users\<name>`). Support scope narrowed to Windows 11 (README,
  AGENTS, [release.md](development/release.md)); manual
  `release-build.yml` added; passed once on GitHub (run 43643326406 at
  `369d0cb`: MSI + NSIS built, unpacked, their `sotyware.exe` equal to
  the checked binary; nothing installed, signed or uploaded). At
  `369d0cb`, `ci.yml` (run 43641836906) and `isolation-guard.yml`
  (43641836899) passed too; CI now runs the recorder and test-profile
  tests on Linux and the PowerShell helpers' self-test. The unexpected 21:46
  activity in the real Windows profile was one launch of the pinned
  development build through Explorer, which created an empty account
  database ([status](status/status.md#unexpected-activity-in-the-real-windows-profile-2026-11-14));
  who or what triggered it is not established.
- **Final review of ADR-026/027** (2026-11-14, uncommitted on
  `9d4737d`, local servers only,
  [status.md](status/status.md#final-review-of-adr-026-and-adr-027-2026-11-14)):
  two defects fixed (the several-item download summary counted files of
  failed items; a plan with every item left out opened an empty review),
  the Windows run-3 timeout traced to a dialog that returned no folder
  (not reproduced). WSL2: Vitest 179, Rust 412/411/440, all guards;
  `cloud-ui.sh files` 97/0; native Windows 11 `-Mode files` 65/0.
- **Native folder dialogs, drag and drop, clipboard, final audit**
  (2026-11-14, uncommitted on `124364c`, local servers only,
  [status.md](status/status.md#native-folder-dialogs-drag-and-drop-clipboard-and-final-audit-2026-11-14)):
  folder uploads and several-item downloads through the **native Windows
  folder dialog** (`windows-cloud-ui.ps1 -Mode files` 65/0, twice) and the
  **GTK folder dialog** on the virtual display (`cloud-ui.sh files` 97/0,
  with a drop from another GTK program over XDND and the in-app clipboard
  with real keys; not a real Linux desktop). One defect fixed (a long
  local name cut to end in `.`/space). The isolation guard was tightened
  (it passed the incident-era `cloud_ui.py`), Windows drivers stop if the
  real folder cannot be read through WMI, CI now type-checks the
  test-profile build and runs the guard for driver-only changes
  (`isolation-guard.yml`; passed on GitHub 2026-11-14). Not tested: drop from
  Explorer, the system clipboard, real keys on Windows, `windows.ps1`.
- **Test isolation incident and fix** (2026-11-14,
  [status.md](status/status.md#test-isolation-incident-root-cause-and-fix-2026-11-14)):
  root cause established: the agent's Windows processes run inside the
  Claude desktop app's MSIX package, where `%LOCALAPPDATA%` is virtualized;
  the driver moved its virtual copy of the account database while the app
  opened the user's real one, and the driver's cleanup deleted that real
  file. Fixed by construction (ADR-027): the `test-profile` feature (exit 3
  without a valid profile, own identifier, databases/webview/credentials in
  the profile, acknowledgement checked by every driver before touching the
  window), `check-desktop-isolation.mjs` in `test:headless`. The user's
  account was not restored and no provider was contacted.
- **File management audit and desktop tests** (2026-11-14,
  [status.md](status/status.md#file-management-desktop-tests-and-audit-2026-11-14)):
  12 defects fixed with regression tests (suspended-versioning deletes,
  folders that gained objects after review, late cancel, restore retry,
  BMP-looking text, search cursor, deleted folder markers, several-item
  download counting and file/folder name clash, shortcuts in Deleted items,
  WebKitGTK context menu, recorder operation names). Linux virtual display
  `cloud-ui.sh files` and native Windows 11 `windows-cloud-ui.ps1 -Mode
  files` against an in-memory versioned server; results there.
- **File management** (2026-11-14, uncommitted on `ae11691`, headless
  only): WSL2 `test:headless` Vitest **173** (38 new), `cargo test`
  **385 + 26 ignored** (about 52 new, 3 runs), recorder build 412, fmt,
  clippy (both builds), check, build; native Windows 11 copy: check, fmt,
  clippy, `test:headless` (173 + Rust 374 + 26 ignored) passed. Against the in-memory versioned S3 backend
  (`test_server::Versioned`) and s3s-fs only. **No desktop driver, no
  native folder dialog, no drop from a real file manager, no real
  provider.** [status.md](status/status.md#file-management-2026-11-14).
- **ADR-024 Backblaze smoke test: not run** (2026-11-14). Its local
  rehearsal on native Windows 11 acted on the user's own account (the
  account database swap had not taken effect): one read-only
  `ListObjectsV2` of the bucket root with the user's key, then the account
  was renamed, given a made-up key and removed (account and Credential
  Manager entry gone; bucket unchanged). `windows-cloud-ui.ps1` now stops
  unless the window appears and the account list is empty.
  [status.md](status/status.md#ui-test-acted-on-a-real-account-2026-11-14).
- **Release-binary guard self-test on Windows** (2026-11-14, test code
  only): `ci.yml` at `d5de7c8` passed on Ubuntu; on Windows two manifest
  tests of `test:release-binary-guard` failed (CRLF checkout, the test's
  `"[features]\n"` edit matched nothing). The test now edits an LF copy
  and fails on an edit that changes nothing; WSL2 13/13 with CRLF and LF.
  Not yet on Windows or CI.
  [status.md](status/status.md#the-release-binary-guards-self-test-on-a-windows-checkout-2026-11-14).
- **Late-part control on a loaded machine** (2026-11-13, test code only):
  CI failed at `f902efb` on
  `without_the_wait_a_handed_over_part_lands_after_the_abort`; the
  `Pace::Buffered` relay half-closed upstream after its client left, and
  hyper's server then dropped the part being stored. The relay no longer
  half-closes (waits ≤ 60 s for the answer). WSL2, pinned to 2 CPUs: 11/11
  full-suite runs; `cargo test` 333 + 26 ignored. Not yet on CI or
  Windows. The older flake seen then
  (`cancelling_during_a_single_put_stores_nothing`, stage `hashing`) was
  a reporting race, fixed: a file is `sending` as soon as its PUT or
  UploadPart starts ([status](status/status.md#a-file-is-sending-as-soon-as-its-request-starts-2026-11-13)).
  [status.md](status/status.md#the-late-part-control-test-on-a-loaded-machine-2026-11-13).
- **Late-part wait on native Windows 11** (2026-11-13, test code on
  `ad6f69c`, committed as `bf991d0`; recorder build `0ae6f848…`, local
  server): new rehearsal-only `windows-cloud-ui.ps1 -Mode b2-lifecycle
  -Rehearse -RecordRequests -Scenario settle` (buffering link) **42/42, 3
  consecutive runs**: the part in flight at the cancel was answered 200
  about 2.3 s later and the abort followed it (ADR-025 wait ran). The
  first run **failed 38/4**: on a link capped by Windows' timer it hit
  the 5 s bound and a part was written after the abort (removed only
  with the local server's folder; the app's log line not captured). The other
  recorder rehearsals on this build: download 40/40 (20 = 20), upload
  78/78 (104 = 104), multipart 42/42, retry 42/42, leave 38/38, each
  recording complete and equal to the server.
  [status.md](status/status.md#late-part-wait-and-the-remaining-recorder-rehearsals-on-native-windows-11-2026-11-13).
- **Request recorder** (2026-11-13, uncommitted on `1e50821`, ADR-024):
  opt-in Cargo feature `request-recorder` (off by default, only into
  `--target-dir target/recorder`, refused by the packaging guard
  `scripts/check-release-binary.mjs` in `beforeBundleCommand`; a packaged
  recorder binary never records) records the app's own S3 requests when
  `SOTYWARE_REQUEST_RECORD_DIR`, `_RUN` and `_SCOPE` are set: one
  read-only interceptor per `S3Store`, JSON Lines per run and process,
  redacted, bounded queue + writer thread
  ([cloud-storage.md](architecture/cloud-storage.md#request-recorder)).
  Local only: default `cargo test` 330 + 25 ignored, feature build 357
  (also with every store recorded); Linux virtual display `cloud-ui.sh
  download upload` with the recorder **77/77, 3 runs**, recorder = server
  (21 = 21, 92 = 92 app requests); native Windows 11 (recorder runs end
  with the tray Quit): download 40/40 (20 = 20; one earlier run had 5
  save-dialog failures), upload 78/78 (104 = 104), `b2-lifecycle
  -Rehearse` multipart/cancel/retry/leave all passed, quit 37/2 (a part
  file left by an s3s-fs race the recording explains) then 39/39.
  **Never run at a real provider.**
  [status.md](status/status.md#request-recorder-2026-11-13).
- Upload **lifecycle at Backblaze through the Windows dashboard**
  (2026-11-13, after approval): **all five scenarios passed once each**
  (multipart 35/35: one open multipart upload seen, 5 parts completed,
  `sha256-verified`; cancel 32/32 and quit 32/32 (exit in 1.4 s): no
  object, no open multipart upload; retry 35/35: only the locked file
  retried, one version per file; leave 31/31: nothing stored); cleanup by
  version id left 0 versions and 0 multipart uploads each time. App
  requests not recorded at Backblaze.
  [status.md](status/status.md#upload-lifecycle-at-backblaze-on-native-windows-11-2026-11-13).
  Preparation (same day, uncommitted on `4d9d9a7`): `windows-cloud-ui.ps1 -Mode b2-lifecycle
  -Scenario multipart|cancel|retry|leave|quit` (refused without
  `-ConfirmB2Lifecycle lifecycle-ui-validation`; provider side in
  `cloud/integration/upload/lifecycle.rs`; release settings, so
  `multipart` completes a 5-part upload through the UI for the first
  time). `-Rehearse` against the local server on native Windows 11,
  release binary `5e364f2a…94f5e1f`: 38/38, 35/35, 38/38 (twice), 34/34,
  35/35 (quit in 1.3 s); `b2-upload -Rehearse` 41/41; WSL2 Rust 324 + 25
  ignored, Windows 314 + 25, Vitest 135.
  [status.md](status/status.md#upload-lifecycle-tests-at-backblaze-prepared-2026-11-13).
- Uploads through the **Windows dashboard at Backblaze** (2026-11-13,
  after approval): `windows-cloud-ui.ps1 -Mode b2-upload` **passed 38/38
  once** (rerun; the first run failed only on the verify test's own request
  count, fixed). Release binary `d17382a1…785d0d`, native Windows 11 25H2
  real desktop, native multi-file dialog, app key without file name prefix:
  one batch, 3 uploaded, `existing.txt` skipped and not overwritten; the
  four objects downloaded `Sha256Verified` and byte-equal; cleanup by
  version id left 0 versions and 0 multipart uploads. Not at Backblaze:
  cancel, retry, multipart, browser lifecycle; Linux; Windows 10.
  [status.md](status/status.md#dashboard-upload-at-backblaze-on-native-windows-11-rerun-passed-2026-11-13).
  Preparation (same day, on `48c33c4`, committed as `32bd7d7`):
  `windows-cloud-ui.ps1 -Mode b2-upload` (refused without
  `-ConfirmB2Upload upload-ui-validation`; needs a third key without file
  name prefix, `b2-upload.xml`) with provider side in
  `cloud/integration/upload/ui.rs` (36 requests planned, cleanup by version
  id). Local only, native Windows 11, release binary `d17382a1…785d0d`:
  `-Rehearse` 41/41 twice (app requests exactly as planned), `upload`
  74/74, `download` 36/36, `browse` 29/29, `local` 19/19; Windows Rust 309
  + 21 ignored, WSL2 319 + 21 ignored, Vitest 135.
  [testing.md](development/testing.md#uploads-through-the-windows-dashboard-at-backblaze),
  [status.md](status/status.md#dashboard-upload-test-at-backblaze-prepared-2026-11-13).
- Uploads at **real Backblaze B2** (2026-11-13, `2828582` + uncommitted
  docs): opt-in `backblaze_b2_upload_batch`
  (`cloud/integration/upload.rs`; needs `--ignored`, the B2 variables and
  `SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation`) **passed once on
  WSL2** (run by the user, 4.44 s). Through the Rust batch path (no UI,
  no dialog): one batch (empty, Unicode `größe +test.bin`, 3-part
  multipart) completed with size and SHA-256 matched; an existing key was
  skipped (`started: false`) and refused, never overwritten; a multipart
  upload cancelled in part 2 was aborted (204); three downloads were
  `sha256-verified` and byte-equal. 41 requests, exactly as planned, all
  inside the `RunScope` (which refuses any request outside the run before
  it is sent). Cleanup deleted the 3 versions by id; afterwards 0 versions
  and 0 multipart uploads under the run prefix. Backblaze accepts
  `ListMultipartUploads` with the prefix-restricted key. **Native
  Windows 11** (same day, `96c9461`, separate result): the same test
  **passed once** (4.18 s) on Windows 11 25H2 build 26200.9711 x64, test
  program SHA-256 `66ce9ebe…885842`, same bucket and write key; 41
  requests with the same counts and statuses, nothing refused, 3 versions
  deleted by id, afterwards 0 versions and 0 multipart uploads; no
  platform difference seen. **Not run:** the UI at a real provider,
  Windows 10, Hetzner.
  [testing.md](development/testing.md#uploads-at-backblaze),
  [status.md](status/status.md#uploads-at-backblaze-2026-11-13),
  [Windows](status/status.md#uploads-at-backblaze-on-native-windows-11-2026-11-13).
- rfd patch guard (2026-11-13, uncommitted on `207363d`):
  `npm run check:rfd-patch` (`scripts/check-rfd-patch.mjs`, first step of
  `test:headless`, in `ci.yml` after `cargo fetch --locked` and before the
  manual workflows' release builds) fails unless `cargo metadata --locked`
  resolves rfd once, at 0.17.2, from `src-tauri/patches/rfd`, and `com.rs`
  keeps `OsString::from_wide`. Its tests (`npm run test:rfd-patch-guard`,
  temporary broken copies: unused patch, removed entry, other version,
  stale lock, missing copy or file, reverted conversion) **10/10 on WSL2
  and 10/10 on native Windows 11**; WSL2 `test:headless` 135 + 308.
  **On GitHub at `f19dd8b`** (2026-11-13): `ci.yml` run 43360177079
  passed on Linux and Windows (guard after `cargo fetch --locked`, before
  any compile; self-test 10/10 on each; Vitest 135; Rust 308 / 298 + 16
  ignored); `desktop-virtual.yml` passed 119/0/0 with the guard before the
  release build; `desktop-windows-isolated.yml` guard and build ok, test NOT
  RUN (exit 4, elevated runner, as expected). Every build compiled rfd from
  `src-tauri/patches/rfd`.
  [status.md](status/status.md#rfd-patch-guard-2026-11-13).
- Windows dialog paths with an unpaired surrogate (2026-11-13,
  uncommitted on `bd704af`): rfd 0.17.2 patched locally
  (`src-tauri/patches/rfd`, `OsString::from_wide` instead of
  `String::from_utf16(..).unwrap()`, ADR-023; upload and save dialogs were
  both affected). **Native Windows 11** (release binary SHA-256
  `24f4db80…7d1c66`): patch tests 6 passed (real `IShellItem` of files
  with unpaired surrogates returns the same code units); `-Mode upload`
  **74/74** (the surrogate file skipped as "name cannot be an object key",
  the valid one uploaded, nothing under another key); `-Mode download`
  **36/36** with a new save under an unpaired-surrogate name; negative
  control without the patch: that step ended sotyware with `0xC0000409`;
  `test:headless` 135 + 298, 0 failed. **WSL2:** `cargo test` 308 + 16
  ignored, fmt, clippy; `cloud-ui.sh upload` 51/51, `download` 22/22.
  One run each. No real provider, no Windows 10.
  [status.md](status/status.md#windows-dialog-paths-with-unpaired-surrogates-2026-11-13).
- Uploads on the **native Windows 11 desktop** (2026-11-12, uncommitted
  test code on `695e089`; 25H2 build 26200.9711, release binary SHA-256
  `a7601843…f07c`): `windows-cloud-ui.ps1 -Mode upload` **74 passed, 1
  failed, 3 consecutive runs**, native **Windows multi-file dialog**, real
  Credential Manager, local s3s-fs server at 2 MiB/s; real mouse only for
  tray Quit. Same batches as Linux plus a dialog cancel and a file another
  process holds open (failed, then retried); bucket empty afterwards. The
  failure is a **defect**: an unpaired-surrogate file name chosen in the
  dialog ends sotyware (`0xC0000409`, rfd's `unwrap` under `panic =
  "abort"`; fixed 2026-11-13, above). Regression: `-Mode download` 35/35 (3 runs),
  `local` 19/19, `browse` 29/29; Windows `test:headless` 135 + Rust 297
  passed, **1 failed** (an existing unit test wrote a file name with `?`,
  which Windows refuses; fixed 2026-11-13: Windows Rust 298 passed, 0
  failed); WSL2 `cargo test` 308 + 16 ignored.
  **No real provider, no Windows 10.**
  [status.md](status/status.md#cloud-storage-uploads-on-the-native-windows-11-desktop-2026-11-12).
- Uploads through the **release binary** on the **WSL2 virtual display**
  (2026-11-12, uncommitted test code on `f9026f5`): `cloud-ui.sh upload`
  **51/51, 3 runs**, native **GTK multi-file dialog**, local s3s-fs server
  (cancel, retry, multipart abort and refused abort, needs-inspection
  without retry, changed file, download beside it, leaving the browser,
  hide + second launch, tray Quit; bucket empty afterwards). Regression:
  `download` 22/22 (3), local/hang/browse 16/8/17, virtual display 119/0/0,
  headless 308 + 16 ignored, 135. **No native Windows, no real provider.**
  [status.md](status/status.md#cloud-storage-uploads-through-the-release-binary-on-a-virtual-display-2026-11-12).
- Upload UI (2026-11-12, uncommitted on `e7ae326`, WSL2, frontend only):
  Vitest **135 passed** (42 new in `src/CloudUpload.test.ts`, mocked upload
  commands and channels; the 93 existing, including the download tests,
  unchanged); `npm run check` clean; `cargo test` 308 + 15 ignored
  (unchanged). **No desktop run, no native dialog, no native Windows, no
  real provider.**
  [status.md](status/status.md#cloud-storage-upload-ui-2026-11-12).
- Upload batches, backend and commands (2026-11-12, uncommitted on
  `241de11`, WSL2, in-process server only): `cargo test` **308 passed, 15
  ignored**, 3 runs (36 new in `cloud/upload/tests.rs`: dialog cancel and
  empty, 100/101 files, every skip reason, existing objects, one file at a
  time, one batch at a time, upload beside a download, cancel at every
  stage, retry, stop-the-batch errors, `NeedsInspection`, `IncompleteParts`
  cleanup, shutdown, stale ids, no paths in any event); download tests
  unchanged and passing; frontend 93. **No real provider, no native
  Windows, no UI, the dialog never opened.**
  [status.md](status/status.md#upload-batches-backend-and-commands-2026-11-12).
- Backend upload lifecycle (2026-11-12, uncommitted on `7b11ecf`, WSL2,
  in-process server only): `cargo test` **272 passed, 15 ignored**, 3 runs
  (34 new: cancellation at every stage, multipart abort and its failure,
  file changes, SDK retries, conflicts, verification, throttled and stalled
  links); the opt-in one-minute PUT with the default timeouts passed (72 s).
  **No real provider, no native Windows.**
  [status.md](status/status.md#backend-upload-lifecycle-2026-11-12).
- Hardened Backblaze download test, **native Windows 11 dashboard**
  (2026-11-12, `bb8c178`, no change): offline checks first (variable check
  PASS; WSL2 `cargo test` 238 + 14 ignored, Windows 228 + 93); then
  `windows-cloud-ui.ps1 -Mode b2-download` **35/35, one run, first attempt**
  on the real desktop: prefix empty, manifest matched the uploaded run and
  version, native save dialog, `sha256-verified`, size, SHA-256 and bytes
  equal; exactly that version deleted, 0 left; account database, Credential
  Manager and `%TEMP%` unchanged. The Rust-only `backblaze_b2_download_file`
  was not rerun.
  [status.md](status/status.md#hardened-backblaze-download-test-on-the-windows-dashboard-2026-11-12).
- Review of the download tests (2026-11-12, uncommitted on `5a7453a`,
  test code and docs only; no remote request, no desktop interaction): no
  production defect found; new static check
  `tests/desktop/check-ps-variables.ps1` (PowerShell variables ignore case;
  it found a latent `$out`/`$Out` clash, now renamed); the opt-in download
  tests check endpoint and empty prefix and print failure reasons on stdout
  (**changed paths not yet run at Backblaze**). `cargo test` 238 + 14
  ignored. [status.md](status/status.md#review-of-the-download-tests-2026-11-12).
- Real Backblaze download through the **native Windows 11 dashboard**
  (2026-11-12, uncommitted tree on `fc76109`, test code only):
  `windows-cloud-ui.ps1 -Mode b2-download` on the real, unlocked Windows 11
  25H2 desktop, release binary: prepare test checks the run prefix
  `sotyware-it/download-ui-validation/<run>/` is empty and uploads one
  generated 16 MiB + 3 `größe test.bin` (SHA-256 first); the dashboard
  (read-only key, real Credential Manager) browses to it, Download opens the
  **native save dialog** with the name unchanged, saves to `%TEMP%`; final
  "Downloaded 16 MiB. The SHA-256 checksum stored with the file matched.",
  the file in place at full size when shown, SHA-256 and bytes equal the
  original; only one progress line seen (0.6 s); version deleted by id, 0
  left; local files removed; **35/35, one run** (an earlier attempt failed
  in the driver, no GET; its object was deleted too). WSL2 `cargo test` 238
  + 14 ignored, Windows 228 + 93.
  [status.md](status/status.md#real-backblaze-download-through-the-windows-dashboard-2026-11-12).
- One download from **real Backblaze B2** (2026-11-12, uncommitted tree on
  `d95978d`, test code only): `backblaze_b2_download_file` uploaded one
  generated 16 MiB + 3 file (SHA-256 computed locally first) to
  `sotyware-it/download-validation/<run>/größe test.bin`, downloaded it
  once with the read-only key through `CloudDownloads` (the code behind
  `download_cloud_file`; **no UI, no save dialog, WSL2 only**), and deleted
  exactly its version (0 left). 5 requests: PUT, HEAD, GET, DELETE, one
  version listing. `GetObject` carried `Content-Length` and the stored
  `sotyware-sha256`; final message `sha256-verified` after the file was in
  place; local SHA-256 and bytes equal; no partial file. `cargo test` 237
  passed, 12 ignored. **Not run:** the UI/dialog at a real provider,
  native Windows, Hetzner.
  [status.md](status/status.md#one-download-from-backblaze-2026-11-12).
- Cloud Storage single-file downloads (2026-11-12, uncommitted tree on
  `8e23fd4`): WSL2 `cargo test` **236 passed, 11 ignored** (27 new), frontend
  **93** (12 new); Windows 11 226 + 93. Through the release binary with the
  **native save dialog** against the in-process object server: Linux
  `cloud-ui.sh download` **22/22, 6 runs** (GTK under Xvfb: virtual display)
  and Windows `windows-cloud-ui.ps1 -Mode download` **35/35, 3 runs** on the
  **real Windows 11 desktop** (dialog cancel, save + SHA-256, replace after
  the dialog's confirmation, Unicode name, no stored checksum, progress +
  Cancel, broken connection + Try again, checksum mismatch; Linux also quit
  during a download). Regression: virtual display 119/0/0 (3), `cloud-ui.sh`
  local 16/16, hang 8/8 (3), browse 17/17 (5 of 6; one AT-SPI reply timeout,
  driver hardened); Windows private desktop 56/0 (3), cloud UI local 19/19
  and browse 29/29 (4), real desktop `windows.ps1` 103/0 (3). **Not run:**
  any real provider.
  [status.md](status/status.md#cloud-storage-single-file-downloads-2026-11-12).
- Real Windows desktop and fewer Backblaze listings (2026-11-12, tree
  `f304019` + test-only change): `windows.ps1` on the **real** Windows 11
  desktop, release binary, **103/0, 3 runs** (user data put back unchanged).
  The dataset test's provider walk of `many/` now uses pages of **336
  instead of 7** (4 listings instead of 145; about 44 instead of 185 per
  dataset check, about 95 instead of 250 per Backblaze UI run, estimated) and
  checks every page against the expected key-order slice; local listing
  server only, no Backblaze run. Regression: WSL2 `cargo test` 209 + 10
  ignored, frontend 81, virtual display 119/0/0 (3), `cloud-ui.sh` 16/16,
  8/8, 17/17 (3); Windows 11 200 + 81, private desktop 56/0 (3), cloud UI
  local 19/19 and browse 29/29 (3).
  [status.md](status/status.md#real-windows-desktop-and-fewer-backblaze-listings-2026-11-12).
- Browsing a temporary dataset at **real Backblaze** (2026-11-11, uncommitted
  tree on `8641c1a`, test code only): `backblaze_b2_browse_dataset` writes
  1,040 objects (nested and marker-only folders, Unicode, `+ & ? # %`, `//`,
  `..`, 1,010 entries in `many/`) under
  `sotyware-it/browse-validation/<run>/`, lists them through
  `browser::list_folder` (133/133: WSL2 3 runs, Windows 1, plus each UI
  run's creation) and deletes exactly its versions. UI (`b2-browse`, read-only
  key without a name prefix, every row compared with the provider): Linux
  30/30 (runs 1, 3, 4), Windows 46/46 (runs 2–4). Same listings as the local
  server except: Backblaze answers a made-up continuation token with the
  first page. Cleanup verified (0 left) in all 13 runs; the 13th only on
  2026-11-12 (Backblaze's daily transaction cap had refused its version
  listing; afterwards `sotyware-it/` held 0 versions). WSL2 regression: `cargo test` 209 + 10
  ignored, frontend 81, virtual display 119/0/0 (3), `cloud-ui.sh` local
  16/16, hang 8/8, browse 17/17 (3); Windows 11: 200 + 81, private desktop
  56/0 (3), cloud UI local 19/19 and browse 29/29 (3); `windows.ps1` not run
  then (run 2026-11-12, above).
  [status.md](status/status.md#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11).
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
  Windows `local` 19/19 (3); Linux `cloud-ui.sh local` 16/16 (3) after the
  driver learned to decline the unlock prompt that a system-installed
  gnome-keyring now opens (before: 14/2, the two locked-keyring checks, also on
  `ad014e7`).
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
- CI (GitHub Actions; the repository is public since 2026-11-15, so
  standard runners are free; until then it was private on GitHub Free with
  $12 of compute a month): `ci.yml` runs automatically on pull requests to and
  pushes to `main` (Linux + Windows: `npm ci`, check, build, fmt, clippy,
  `test:headless`; docs-only changes skipped), and so does `macos.yml`
  (Apple Silicon + Intel; its first run, at `d9cf561`, failed one test on
  both; at `ec42d41` it passed:
  [status](status/status.md#hosted-runs-at-ec42d41-macos-passes-2026-11-15)).
  Its two automatic runs so far
  passed: both the Ubuntu and the Windows headless job (run ids, commits and
  durations not recorded); a third, run 43360177079 at `f19dd8b`
  (2026-11-13, with the rfd guard), passed both jobs (7 min 37 s Linux,
  17 min 23 s Windows, cold Cargo cache). Manual only:
  `desktop-virtual.yml` (virtual display, not a real desktop) and
  `desktop-windows-isolated.yml` (experimental). The real-desktop Windows test
  is not in CI. `desktop-virtual.yml` passed on GitHub, 92/0/0 (run id and
  commit not recorded), and 119/0/0 at `f19dd8b` (2026-11-13); this supersedes its first run (`14a5de1`, failed
  89/2/1 on a driver bug fixed in `ead47e7`). It is a virtual display with a
  stub tray host, so it says nothing about real GNOME, KDE or XFCE
  desktops. `desktop-windows-isolated.yml` ran twice
  (Windows Server 2025): the first run failed on the elevated hosted job,
  reproduced on Windows 11 elevated; the second (2026-11-10) printed
  `elevated: True` and reported NOT RUN, exit 4 (not a pass): hosted runners
  cannot run this test; a third run at `f19dd8b` (2026-11-13) did the same.
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
silently turns off without a D-Bus session bus. No autostart.
A corrupt feature-preferences database is never repaired: defaults are used
until the user moves the file away. Cloud account credentials need the OS
credential store: Linux without a Secret Service (WSL2, minimal window
managers) cannot store any, by design. The connection check is verified at Backblaze by one
opt-in run and one UI run each on Linux and Windows, never at Hetzner; while one credential-store call waits (an unlock
prompt), the other account calls wait behind it. The bucket browser is verified
at Backblaze with a temporary 1,040-object dataset (WSL2/Xvfb and Windows 11),
never at Hetzner; Backblaze answers a continuation token it never issued with
the first page instead of an error; a key limited to a file name prefix cannot
browse (Backblaze refuses the root); no total count, no cache. Downloads
are verified at Backblaze once through the Rust code (WSL2) and twice through
the native Windows 11 dashboard and save dialog; through the UI on Linux
only against the local object server; one at a time, without resume; a killed process leaves its hidden
`.sotyware-partial` file; on file systems without hard links a file
appearing at a new destination in the last instant could be replaced.
Uploads ran at a real provider only at Backblaze: through the Rust
batch path once on WSL2 and once on native Windows 11, and through the
native Windows 11 dashboard and multi-file dialog (one batch, then
multipart, cancel, retry, leaving the browser and quit once each with the
release settings; needs-inspection and a refused abort only against the
local server);
never at Hetzner, on Windows 10 or a real Linux desktop (Linux: the
release binary with the GTK dialog only against the local server on the
WSL2 virtual display); on Linux a non-UTF-8 file name
silently drops out of the dialog's selection (rfd); on Windows names with
an unpaired surrogate work only through the locally patched rfd (a
dialog-plugin update that bypasses the patch brings the abort back;
ADR-023; `npm run check:rfd-patch` fails then, in `test:headless` and
CI); incomplete multipart uploads whose abort failed are remembered
only while sotyware runs (afterwards only a bucket lifecycle rule removes
them); quitting can take up to 35 s if an abort hangs; a part whose bytes
were all handed over when cancelling is awaited up to 5 s before the
abort (ADR-025), past that it can still land after the abort (unchecked
at real providers).
File management (ADR-026) ran only against local test servers: no real
provider; desktop runs on the Linux virtual display and native Windows 11,
folder dialogs included (no drop from Explorer or a real file manager,
no system clipboard, clipboard keys only on the virtual display); one request of
race between a check and a copy or delete; copies above 5 GiB refused; deletes only add delete markers
(hidden versions stay billed) or remove for good on unversioned buckets
([limitations.md](status/limitations.md#file-management-adr-026)).
Full list: [limitations.md](status/limitations.md).

## Milestone and next steps

Milestone 0: prove the desktop runtime on Windows and Linux. Implemented;
validated natively on Windows 11 (automated, last at `a7bf8f3`, including the built-in features and their saved choices) and in Linux
simulation (WSL2 is the development environment, not a runtime target). Next:
the manual check on Windows 11, Windows 10, then real Linux desktops (including
dashboard-only mode on vanilla GNOME; needs a VM, live session or Linux machine); record results in the
[validation matrix](status/status.md#validation-matrix). See
[roadmap.md](status/roadmap.md). Cloud storage next step (desktop driver runs of file management against the local servers; with approval, one Backblaze run per file operation; uploads at a real provider on Linux and Windows 10; still open for downloads: the UI at a real provider on Linux, Hetzner): [roadmap.md](status/roadmap.md#cloud-storage-after-the-proof-of-concept).
Production-readiness work of 2026-11-15 (storage reliability, backups,
accounts and sync, updates, security review; nothing pushed, deployed or
run at a provider): [journal](status/journal.md#final-assessment); next,
with the maintainer's approval, hosted CI on this tree, desktop driver runs
of the new pages, then the external gates (keys, certificates, hosting,
email, provider access, an external review). Further local persistence
(settings, other data), a feature-management UI, third-party plugins,
background sync and autostart are undecided ideas, not tasks.

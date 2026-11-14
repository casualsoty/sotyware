# Repository map

Where things are. Generated and ignored paths are listed in
[workflow.md](../development/workflow.md#generated-and-ignored-paths).

```
.
├── AGENTS.md                 Canonical instructions for coding agents
├── CLAUDE.md                 Claude Code entry point (imports AGENTS.md)
├── README.md                 Human-facing overview and quick start
├── .github/
│   └── workflows/
│       ├── ci.yml            Automatic headless CI (Linux + Windows): check, build, rfd patch guard, guard self-tests, fmt, clippy (default and test-profile build), test:headless
│       ├── isolation-guard.yml  Automatic for changes to tests/desktop/** or the guard (which ci.yml skips): desktop isolation guard, its self-test, the Python profile helpers' tests
│       ├── desktop-virtual.yml           Manual: Linux virtual-display test (Xvfb + openbox)
│       ├── desktop-windows-isolated.yml  Manual, experimental: Windows private-desktop test
│       └── release-build.yml  Manual: Windows release binary + MSI + NSIS via npm run build:release (ADR-028), installers unpacked (never installed) and their sotyware.exe compared with the checked binary; nothing signed or uploaded
├── docs/
│   ├── CONTEXT.md            Short session-start context for agents
│   ├── architecture/         How it works and why (this file, overview, lifecycle, features, cloud storage and providers, single instance, decisions)
│   ├── development/          Setup, commands, testing, cross-platform rules, release builds and installer testing (release.md), application icons (icons.md)
│   └── status/               What is implemented and validated, limitations, roadmap
├── index.html                Vite HTML entry; mounts #app, loads src/main.ts
├── src/                      Frontend (Svelte 5 + TypeScript)
│   ├── main.ts               Mounts App into #app
│   ├── App.svelte            The dashboard UI: navigation (Overview, Cloud Storage) and the Overview section
│   ├── App.test.ts           Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
│   ├── CloudStorage.svelte   Cloud Storage section: account list, empty state, check, remove with confirmation
│   ├── CloudAccountForm.svelte  Add/edit form for one storage account (credential fields cleared after use)
│   ├── CloudBrowser.svelte   Browser of one account's bucket: folders, files, breadcrumbs, Up, Refresh, Load more; Download per file; file manager (ADR-026): selection, context menu, selection bar, shortcuts, search, sort, filters, new folder, rename, favorites/recent, drop banner
│   ├── CloudOperation.svelte Review dialog and progress panel of copy/move/rename/delete/restore plans
│   ├── CloudFolderPicker.svelte  Destination folder picker (browse folders, create one)
│   ├── CloudProperties.svelte    Properties, size-limited preview (text as text, raster images only) and share links
│   ├── CloudBulkDownload.svelte  Panel of a download of several files and folders
│   ├── CloudDeleted.svelte   Deleted items of the folder (delete markers) with Restore
│   ├── Modal.svelte          Modal dialog (focus kept inside, Escape closes)
│   ├── CloudFileManager.test.ts  Tests of the file manager (mocked commands and channels)
│   ├── CloudBrowser.test.ts  Tests of the bucket browser and its downloads, opened from the Cloud Storage section
│   ├── CloudUpload.svelte    The browser's upload panel: one batch's progress, per-file list, Cancel, Try again, cleanup notes, summary
│   ├── CloudUpload.test.ts   Tests of the upload panel inside the browser (mocked upload commands and channels)
│   ├── CloudStorage.test.ts  Tests of the Cloud Storage section, its form and cloud.ts
│   ├── cloud.ts              Typed wrappers for the six cloud account commands, list_cloud_folder, the three download commands (progress channel) and the three upload commands (per-batch event channel); validates responses and events, CloudCommandError
│   ├── cloudFiles.ts         Typed wrappers for the 22 file-management commands (ADR-026); validates responses and events
│   ├── fileList.ts           Pure list logic: entries, sorting, filters, selection model, keyboard shortcuts
│   ├── fileList.test.ts      Its tests
│   ├── cloudMessages.ts      Labels and hints for check results and command error kinds; upload skip reasons, remote outcomes, upload and file-operation error summaries
│   ├── format.ts             formatSize and shownName, shared by the browser and the upload panel
│   ├── Features.svelte       Minimal Features section of the dashboard (state + enable checkbox per feature)
│   ├── Features.test.ts      Tests of the Features section and of features.ts
│   └── features.ts           Typed wrappers for list_features / set_feature_enabled; validates responses
├── package.json              npm scripts and JS dependencies
├── scripts/
│   ├── check-rfd-patch.mjs   Guard (npm run check:rfd-patch): fails unless Cargo resolves rfd 0.17.2 from src-tauri/patches/rfd (ADR-023)
│   ├── check-rfd-patch.test.mjs  Its tests (node --test, npm run test:rfd-patch-guard) against broken temporary copies of src-tauri
│   ├── build-release.mjs  The normal release build (npm run build:release): tauri build with --remap-path-prefix for home, RUSTUP_HOME, CARGO_HOME and the checkout, then --local-paths on the binary (ADR-028)
│   ├── build-release.test.mjs  Its tests (in npm run test:release-binary-guard)
│   ├── check-release-binary.mjs  Guard against shipping a test build, the request recorder (ADR-024) or a test profile (ADR-027), or a binary naming the build machine's paths (ADR-028): --manifest (npm run check:release-binary), a binary scan, --local-paths, and Tauri's beforeBundleCommand (--before-bundle)
│   ├── check-release-binary.test.mjs  Its tests (node --test, npm run test:release-binary-guard)
│   ├── check-desktop-isolation.mjs  Static check of tests/desktop (npm run check:desktop-isolation, in test:headless): no driver names the user's app folder or credential namespace, moves or deletes a database, deletes a credential directly, or launches without the test-profile checks (ADR-027)
│   ├── check-desktop-isolation.test.mjs  Its tests (node --test, npm run test:desktop-isolation-guard)
│   ├── icons.mjs         The application icons from src-tauri/icons/source (npm run icons, through tauri icon; writes icon.ico itself) and their check (npm run check:icons, in test:headless)
│   └── icons.test.mjs    Its tests (node --test, npm run test:icons)
├── package-lock.json         Pinned JS dependency tree
├── vite.config.ts            Vite: port 1420, strictPort, ignores src-tauri/, output dist/; Vitest config
├── svelte.config.js          vitePreprocess (TypeScript in .svelte)
├── tsconfig.json             Strict TypeScript settings
├── src-tauri/                Rust crate and Tauri configuration
│   ├── Cargo.toml            Crate `sotyware`, lib `sotyware_lib`, dependencies, release profile
│   ├── Cargo.lock            Pinned Rust dependency tree
│   ├── build.rs              tauri_build with an app manifest declaring the app's 38 commands
│   ├── tauri.conf.json       Product name, identifier, window `main`, CSP, bundles, dev/build hooks
│   ├── capabilities/
│   │   └── default.json      Frontend permissions for window `main` (hide is added at runtime, tray mode only)
│   ├── icons/                Application, window and tray icons, generated by npm run icons ([icons.md](../development/icons.md)): icon.ico (16–256 px), 32x32.png, 128x128.png, 128x128@2x.png, icon.png, icon.icns
│   │   └── source/           The SVG sources: sotyware.svg (master, 48 px and up), sotyware-32.svg (24/32 px), sotyware-16.svg (16 px)
│   ├── patches/              Dependencies built from a local copy via `[patch.crates-io]` (ADR-023)
│   │   ├── README.md         What is patched, why, how to verify the copy, when to remove it
│   │   ├── rfd-0.17.2-osstring.patch  The full diff against the published crate
│   │   └── rfd/              rfd 0.17.2 as published, `com.rs` changed: Windows dialog paths as `OsString` + tests
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, OperatingMode and its selection, lifecycle, operating_mode command, single instance, show_dashboard(), AppControl + unit tests; with the feature test-profile, the profile check, identifier, data and webview folders and acknowledgement (ADR-027)
│       ├── test_profile.rs   Only with the feature test-profile (and in unit tests): SOTYWARE_TEST_PROFILE and its marker validated, overlap with the app's own folders refused, CredentialNamespace of the profile, the acknowledgement file + tests (ADR-027)
│       ├── tray.rs           Tray icon, menu, TrayAction, unavailable_reason() (no Linux tray host) + unit tests
│       ├── storage.rs        SqliteStore: preferences.sqlite3, schema, MIGRATIONS, PreferenceStore impl; SqliteAccountStore: cloud-accounts.sqlite3, ACCOUNT_MIGRATIONS; SqlitePlacesStore: cloud-places.sqlite3, PLACES_MIGRATIONS + tests (incl. restarts with the built-in features)
│       ├── cloud/            Cloud storage: backend, the dashboard's account commands, the bucket browser, downloads, upload batches and file management (ADR-026)
│       │   ├── mod.rs        Module overview, re-exports SqliteAccountStore
│       │   ├── provider.rs   ProviderProfile data: backblaze-b2, hetzner, s3-compatible + tests
│       │   ├── account.rs    AccountConfig + resolve(), Accounts, AccountStore trait + tests
│       │   ├── credentials.rs  Secret, Credentials (validation, stored format), CredentialStore trait, CredentialError; test-only EnvCredentials, MemoryCredentialStore + tests
│       │   ├── keyring.rs    OsCredentialStore: Windows Credential Manager / Secret Service via keyring-core, entry names, error mapping + tests, opt-in real-backend test
│       │   ├── s3.rs         S3Store (aws-sdk-s3): check_bucket, list, list_folder (one delimited page), upload_file wrapper, download (streamed, progress, TransferControl cancel/commit, temporary file, final move), delete, SHA-256, errors + tests
│       │   ├── s3/
│       │   │   ├── upload.rs  S3Store::upload (ADR-021): first hashing pass with per-part SHA-256, destination HEAD, request bodies that re-hash and hold back their last chunk (commit point), stall timeout instead of the read timeout, multipart abort in its own task (open upload id on TransferControl; abort_incomplete_upload), HEAD verification, UploadError/RemoteOutcome, throttled UploadProgress
│       │   │   ├── upload/tests.rs  Upload tests against the in-process S3 server: basics, cancellation at every stage, multipart cleanup, file changes, SDK retries, throttled and stalled links
│       │   │   ├── manage.rs  File-management requests (ADR-026): copy_object (MetadataDirective COPY, copy-source-if-match), put_folder_marker, delete_reporting (delete marker or not), list_recursive_page, list_versions_page, bucket_versioning, object_properties, read_head (ranged GET), presign_get + tests
│       │   │   └── hook_assumptions.rs  Tests (every build) of the SDK interceptor behaviour the request recorder relies on (ADR-024), with a probe interceptor
│       │   ├── recorder.rs   Only with the feature request-recorder (ADR-024): settings from SOTYWARE_REQUEST_RECORD_*, the read-only interceptor installed in S3Store::new, bounded queue and writer thread, redaction, health (tests in recorder/tests.rs)
│       │   ├── dashboard.rs  CloudAccounts (Tauri state), the six account commands, safe DTOs, CommandError, connection check, listing cursors + tests
│       │   ├── browser.rs    list_cloud_folder: folder/file DTOs, prefix and breadcrumb rules, error mapping (tests in browser/tests.rs)
│       │   ├── download.rs   download_cloud_file / cancel / retry: CloudDownloads (one slot), save dialog, progress messages, error mapping (tests in download/tests.rs)
│       │   ├── upload.rs     upload_cloud_files / cancel / retry (ADR-022): CloudUploads (one batch slot), multi-file dialog, selection checks, sequential batch with HEAD before each file, error sorting, events with seq and one final summary, retry, incomplete multipart ids in memory, bounded shutdown (tests in upload/tests.rs)
│       │   ├── operations.rs CloudOperations (one slot): plan / discard / start / cancel / retry of copy, move, rename, delete and restore (ADR-026): planning (enumeration, conflicts, versioning), execution phases with bounded concurrency, verification, item and operation states, events, retry rules, bounded shutdown (tests in operations/tests.rs)
│       │   ├── operations/names.rs  Pure rules: valid names, prefix boundaries, relocation, numbered names, nested selections + tests
│       │   ├── manage.rs     create_cloud_folder, cloud_object_properties, preview_cloud_file, search_cloud_folder (CloudSearches cursors), list_cloud_deleted, share_cloud_file (tests in manage/tests.rs)
│       │   ├── bulk_download.rs  BulkDownloads (one slot): download_cloud_items / cancel / retry: folder dialog, enumeration, safe local names, staging folder, move into place without replacing (tests in bulk_download/tests.rs)
│       │   ├── places.rs     CloudPlaces: favorites and recent items per account (cloud-places.sqlite3) and their four commands + tests
│       │   ├── drop.rs       CloudDrops: dropped paths kept in Rust, watch/unwatch/upload_cloud_drop + tests
│       │   ├── tests.rs      Storage operations against the in-process S3 server
│       │   ├── test_server/versioned.rs  In-memory S3 backend with versions and delete markers (HeadObject, ranged GetObject, PutObject, CopyObject, DeleteObject, ListObjectsV2, ListObjectVersions, GetBucketVersioning); records every version removed by id, tests only
│       │   ├── test_server/files_ui.rs  serve_files_for_ui: the local server of the file-management desktop test (a versioned and an unversioned bucket, seeded; dump, fail, add commands; fails if a real version was removed by id), tests only
│       │   ├── test_server.rs  In-process S3 server (s3s + s3s-fs, or an in-memory listing backend with exact keys and continuation tokens; can refuse chosen operations, records them with part numbers, injects faults per operation and part: 500, 403, no answer, a callback), a throttled or stalling TCP relay (ThrottledLink), an object backend with exact keys and slow/stalled/cut bodies, temp-file helpers, the UI tests' listing, object and upload servers (serve_listing_for_ui, serve_objects_for_ui, serve_uploads_for_ui; the last one optionally seeded with one given file; a second key for the helper's own requests; check_recordings compares request recordings with the app's requests), tests only
│       │   ├── integration.rs  Opt-in (#[ignore]) tests against real Backblaze B2 / Hetzner accounts (round trip, B2 connection check and B2 folder listing with recorded requests, B2 bucket fingerprint) + the same check and listing cases against the test server
│       │   └── integration/
│       │       ├── upload.rs  Opt-in B2 upload test (passed at Backblaze 2026-11-13, WSL2 and Windows 11): one batch (empty, Unicode name, 3-part multipart) under sotyware-it/upload-validation/<run>/ through begin_upload/CloudUploads, existing key skipped and refused, multipart cancelled and aborted, downloads through CloudDownloads; RunScope interceptor refuses any request outside the run before it is sent; cleanup by version id and upload id (also by run id) + the same scenario against the test server
│       │       ├── upload/ui.rs  Provider side of the Windows dashboard upload test (windows-cloud-ui.ps1 -Mode b2-upload; passed once at Backblaze 2026-11-13): checks the local files, run prefix empty, seed existing.txt under sotyware-it/upload-ui-validation/<run>/; afterwards downloads and compares the four keys, cleanup by version id and upload id (also cleanup only), all through the run scope with the write key + the same batch against the test server
│       │       ├── upload/lifecycle.rs  Provider side of the Windows dashboard upload lifecycle tests (windows-cloud-ui.ps1 -Mode b2-lifecycle -Scenario multipart|cancel|retry|leave|quit; each passed once at Backblaze 2026-11-13): checks the local files against the release settings (one PUT or the stated multipart part count), run prefix empty, anchor.txt under sotyware-it/upload-lifecycle-validation/<scenario>-<run>/; a probe of open multipart uploads; afterwards downloads what must be stored, cleanup by version id and upload id, fails on any incomplete multipart upload (also cleanup only); all through the run scope with the write key
│       │       ├── download.rs  Opt-in B2 download test: one generated file (SHA-256 known before upload) under sotyware-it/download-validation/<run>/, one download with the read-only key through CloudDownloads, version-exact cleanup + the same download against the test server
│       │       └── browse.rs  Opt-in B2 browse test on a temporary dataset (create under sotyware-it/browse-validation/<run>/, browse through list_folder, version-exact cleanup; kept for the UI tests with a manifest; cleanup by run id) + the same checks against the test server
│       └── features/         Built-in features
│           ├── mod.rs        Feature trait, metadata, state, builtin() list, the two feature commands (FeatureList, FeatureChange) + tests
│           ├── preferences.rs  FeaturePreferences, PreferenceStore trait: startup choice, save after a switch, failure policy + tests
│           ├── registry.rs   FeatureRegistry: registration, lifecycle table, per-feature locking, start_all/stop_all + tests
│           └── example.rs    The example feature + tests
└── tests/
    └── desktop/              Desktop tests of the release binary, always a test-profile build (ADR-027)
        ├── check-ps-variables.ps1  Static check of the Windows test scripts (parses only, runs nothing): fails if one scope spells a variable two ways ($out/$Out are one variable in PowerShell)
        ├── cloud-ui.sh       Opt-in Linux Cloud Storage end-to-end test (test-profile build): Xvfb + openbox, private D-Bus session, real GNOME Keyring; phases local, hang, browse (local listing server), download (local object server, GTK save dialog), upload (local s3s-fs server, GTK multi-file dialog, stub tray watcher), files (file management against serve_files_for_ui), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key)
        ├── cloud_ui.py       Its driver: add, check, open bucket, browse, download through the save dialog, rename, replace key, remove through AT-SPI + XTest; keyring and database checked from outside
        ├── cloud_ui_upload.py  Its upload phase (run by cloud_ui.py): batches through the GTK multi-file dialog, server state checked after each, exact-key cleanup
        ├── cloud_ui_files.py  Its files phase (ADR-026, run by cloud_ui.py): folders, files, conflicts, partial failures and retries, delete and restore, selection with the real pointer, sorting, filters, search, previews, share link, folder uploads and several-item downloads through the GTK folder dialog (unsafe names, no overwrite, refused file and retry, cancel), a drop from another program (drag_source.py), sotyware's own clipboard with real keys, unversioned delete; server state checked after each step
        ├── drag_source.py    A GTK 3 drag source (text/uri-list over XDND) the files phase drags from with the XTest pointer, so sotyware gets a real drop on the virtual display
        ├── test_profile.py   Test-profile helper of the Linux drivers (ADR-027): create a profile, check the app's acknowledgement, remove only this run's profile, fingerprint the user's folders
        ├── test_profile_test.py  Its unit tests (npm run test:profile-helpers)
        ├── TestProfile.psm1  Test-profile helper of the Windows drivers (ADR-027): the same, plus this profile's and the user's Credential Manager lines and deleting only this profile's entries
        ├── test-profile-selftest.ps1  Its self-test (no app, no desktop)
        ├── fake_secret_service.py  A Secret Service that never answers (cloud-ui.sh hang phase)
        ├── run.sh            Linux/X11 simulated test (test-profile build): private D-Bus session, runtime dir and data dir (XDG_DATA_HOME)
        ├── sni_watcher.py    Stub tray host (org.kde.StatusNotifierWatcher); --reject, --no-host
        ├── single_instance.py  Linux test driver: windows, tray menu, launches, tray failure, dashboard-only mode, feature preferences across restarts, Cloud Storage navigation and empty state, page via AT-SPI
        ├── virtual.sh        Linux: run.sh on a private Xvfb display with openbox
        ├── windows.ps1       Windows test on the real desktop: hide button, tray, close, launches, feature preferences across restarts, Cloud Storage navigation and empty state
        ├── windows-cloud-ui.ps1  Opt-in Windows Cloud Storage end-to-end test through UI Automation (test-profile build, credentials in the profile's namespace); -Mode files (file manager against serve_files_for_ui, incl. folder uploads and several-item downloads through the Windows folder dialog), local, browse (local listing server), download (local object server, Windows save dialog), upload (local s3s-fs server, Windows multi-file dialog; moves the mouse once for tray Quit), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key), b2-download (one temporary file downloaded from Backblaze through the dashboard and the save dialog), b2-upload (one batch uploaded to Backblaze through the dashboard and the multi-file dialog with a third key, refused without -ConfirmB2Upload; -Rehearse: the same against the local s3s-fs server) or b2-lifecycle -Scenario multipart|cancel|retry|leave|quit (one upload lifecycle behaviour at Backblaze with the release settings, refused without -ConfirmB2Lifecycle; -Rehearse: the same against the local s3s-fs server; quit moves the mouse once)
        └── windows-isolated.ps1  Unattended Windows test on a private desktop: window, hide button, close, launches (no tray)
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | `operating_mode` in `lib.rs`; `list_features` and `set_feature_enabled` in `features/mod.rs`; the six cloud account commands in `cloud/dashboard.rs`; `list_cloud_folder` in `cloud/browser.rs`; `download_cloud_file`, `cancel_cloud_download` and `retry_cloud_download` in `cloud/download.rs`; `upload_cloud_files`, `upload_cloud_folder`, `cancel_cloud_upload` and `retry_cloud_upload` in `cloud/upload.rs`; the file-management commands (ADR-026) in `cloud/operations.rs`, `cloud/manage.rs`, `cloud/bulk_download.rs`, `cloud/places.rs` and `cloud/drop.rs`. All are registered with `invoke_handler` in `run()` and declared in the app manifest in `build.rs` (permissions `allow-<command, with ->` in `capabilities/default.json`) |
| Where are built-in features? | `src-tauri/src/features/`: listed in `builtin()` in `mod.rs`, run by `FeatureRegistry` in `registry.rs`; how to add one: [features.md](features.md#adding-a-built-in-feature) |
| Where are feature choices stored? | Policy: `features/preferences.rs`; SQLite, schema and migrations: `storage.rs`; loaded in `run()` in `lib.rs` before the event loop; the file is `preferences.sqlite3` in the app's local data folder ([features.md](features.md#persistence)) |
| Where is cloud storage? | `src-tauri/src/cloud/`; the dashboard's commands in `cloud/dashboard.rs`, `cloud/browser.rs` and `cloud/download.rs`, its page in `src/CloudStorage.svelte` (browser: `src/CloudBrowser.svelte`, upload panel: `src/CloudUpload.svelte`; upload commands in `cloud/upload.rs`); accounts database in `storage.rs`, loaded in `run()`; design: [cloud-storage.md](cloud-storage.md), providers: [cloud-providers.md](cloud-providers.md) |
| Where is the operating mode chosen? | `select_mode()` in `lib.rs`, from `tray::create()`'s result classified by `tray::unavailable_reason()` |
| Where are events emitted or listened to? | No custom events exist. Window events: `on_window_event` in `run()`. Exit events: `app.run(...)` closure in `run()` |
| Where is tray behaviour? | `src-tauri/src/tray.rs`, created from `setup` in `lib.rs` |
| Where is "show the dashboard"? | `show_dashboard()` in `lib.rs`, used by the tray and the single-instance callback |
| Where is the window declared? | `app.windows` in `src-tauri/tauri.conf.json` (label `main`, mirrored by `DASHBOARD_LABEL`) |
| Where are frontend permissions? | `src-tauri/capabilities/default.json` (static); the runtime capability `tray-mode` in `setup` in `lib.rs` |
| Where is the CSP? | `app.security.csp` in `src-tauri/tauri.conf.json` |
| Where is the frontend? | `src/`, entered from `index.html` |
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules (cloud storage: also `cloud/tests.rs`, `cloud/s3/upload/tests.rs`, `cloud/browser/tests.rs`, `cloud/download/tests.rs`, `cloud/upload/tests.rs`, `cloud/operations/tests.rs`, `cloud/manage/tests.rs`, `cloud/bulk_download/tests.rs`, opt-in `cloud/integration.rs`, `cloud/integration/browse.rs`, `cloud/integration/download.rs`, `cloud/integration/upload.rs` and `cloud/integration/upload/ui.rs`); frontend tests in `src/*.test.ts` (`App.test.ts`, `Features.test.ts`, `CloudStorage.test.ts`, `CloudBrowser.test.ts`, `CloudUpload.test.ts`, `CloudFileManager.test.ts`, `fileList.test.ts`); desktop tests in `tests/desktop/` (`run.sh`/`virtual.sh` Linux, `windows.ps1`, `windows-isolated.ps1` Windows; opt-in Cloud Storage end-to-end: `cloud-ui.sh`, `windows-cloud-ui.ps1`); all headless tests: `npm run test:headless`; CI workflows in `.github/workflows/` ([testing.md](../development/testing.md#ci)) |
| Where is a dependency patched? | `src-tauri/patches/` (rfd, wired in by `[patch.crates-io]` in `src-tauri/Cargo.toml`; [README](../../src-tauri/patches/README.md), [ADR-023](decisions.md#adr-023-rfd-is-patched-locally-so-windows-dialog-paths-are-read-as-osstring)) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

# Repository map

Where things are. Generated and ignored paths are listed in
[workflow.md](../development/workflow.md#generated-and-ignored-paths).

```
.
├── AGENTS.md                 Canonical instructions for coding agents
├── CLAUDE.md                 Claude Code entry point (imports AGENTS.md)
├── README.md                 Human-facing overview and quick start
├── .gitattributes            Keeps the signed update fixtures (update/, update-github/) byte-exact (no line-ending conversion)
├── .github/
│   ├── ISSUE_TEMPLATE/       Issue forms the app's feedback buttons open (ADR-049): bug_report.yml (fields version and os, filled from the URL), feedback.yml, config.yml (privacy questions go to privacy@sotyware.com)
│   └── workflows/
│       ├── ci.yml            Automatic headless CI (Linux + Windows): check, build, rfd patch guard, guard self-tests, fmt, clippy (default and test-profile build), test:headless; also called by release.yml
│       ├── macos.yml         Automatic (pull request to / push to main), manual or called by release.yml: macOS Apple Silicon and Intel, each natively on its own runner: job "macOS tests" (headless checks and tests, Keychain round trip in a throwaway keychain; no secret); with package: true job "macOS package" in the macos-signing environment (.app/.dmg, signing and notarization when the Apple secrets exist) (ADR-037, ADR-039)
│       ├── release.yml       Tag vX.Y.Z, or manual (a dry run by default, any branch): version, repository and key checks, ci.yml, Windows/Linux/macOS builds, then rehearse (dry run: throwaway key) or publish (environment release, the only job with the update key: signs, writes latest.json, publishes this repository's draft → pre-release → verified → latest with GITHUB_TOKEN) (ADR-036, ADR-039)
│       ├── isolation-guard.yml  Automatic for changes to tests/desktop/** or the guard (which ci.yml skips): desktop isolation guard, its self-test, the Python profile helpers' tests
│       ├── desktop-virtual.yml           Manual: Linux virtual-display test (Xvfb + openbox)
│       ├── desktop-windows-isolated.yml  Manual, experimental: Windows private-desktop test
│       └── release-build.yml  Manual or called by release.yml: Windows release binary + MSI + NSIS via npm run build:release (ADR-028), installers unpacked (never installed) and their sotyware.exe compared with the checked binary, then their icons checked (check-installer-icons.mjs); with upload: true the installers as the windows-installers artifact (signed for updates by release.yml)
├── docs/
│   ├── CONTEXT.md            Short session-start context for agents
│   ├── architecture/         How it works and why (this file, overview, frontend, lifecycle, features, cloud storage and providers, single instance, decisions, threat model, backup, accounts, updates)
│   ├── development/          Setup, commands, testing, cross-platform rules, release builds, GitHub Releases and installer testing (release.md), macOS (macos.md), application icons (icons.md), the account service (server.md) and its first deployment step by step (server-setup.md)
│   ├── status/               What is implemented and validated, limitations, roadmap, the production-readiness journal (journal.md)
│   └── user/                 User guides: recovering files from a backup (recovery.md), the autoclicker and macros (automation.md), the gaming notes overlay (notes-overlay.md), privacy notice (privacy.md, draft)
├── index.html                Vite HTML entry; mounts #app, loads src/main.ts
├── overlay.html              Second Vite HTML entry: the gaming notes overlay's window (ADR-052); loads src/overlayMain.ts
├── src/                      Frontend (Svelte 5 + TypeScript)
│   ├── main.ts               Loads the styles, applies the saved theme before the first paint, mounts App into #app
│   ├── App.svelte            The shell (ADR-035, docs/architecture/frontend.md): sidebar + main area, navigation state, update notification, keyboard shortcuts
│   ├── Sidebar.svelte        Sections, storage accounts (open their bucket), Account and Settings
│   ├── Overview.svelte       Tray/dashboard-only note, Hide dashboard, summaries of storage, backups and updates
│   ├── Settings.svelte       Tabs: General (theme, window, keyboard), Features, About & updates
│   ├── navigation.ts         Section and tab types, the shell's shortcuts (+ navigation.test.ts)
│   ├── theme.ts              Theme choice: localStorage, data-theme, window setTheme (+ theme.test.ts)
│   ├── Icon.svelte           sotyware's line-drawing icon set
│   ├── updatesModel.svelte.ts  The window's update state and actions, shared by the notification and Settings
│   ├── styles/tokens.css     Design tokens: type, space, radii, sizes, layers, motion, light and dark colours
│   ├── styles/base.css       Element defaults and the shared class vocabulary
│   ├── App.test.ts           Frontend tests (Vitest, happy-dom, Tauri IPC mocks)
│   ├── TitleBar.svelte       The page's own title bar on Windows (ADR-040): drag region, Minimize, Maximize/Restore, Close; TitleBar.test.ts its tests
│   ├── Logo.svelte           sotyware's mark (sidebar and title bar)
│   ├── accountSync.svelte.ts Automatic sync of an enrolled device at startup, on focus (15 min) and after local account changes, and the shell's sync summary (ADR-042); accountSync.test.ts its tests
│   ├── CloudStorage.svelte   Cloud Storage section: account list, empty state, check, remove with confirmation
│   ├── startupChecks.svelte.ts  The connection check of every storage account once at startup (ADR-046); startupChecks.test.ts its tests
│   ├── CloudAccountForm.svelte  Add/edit form for one storage account (credential fields cleared after use)
│   ├── CloudBrowser.svelte   Browser of one account's bucket: folders, files, breadcrumbs, Up, Refresh, Load more; Download per file; file manager (ADR-026): selection, context menu, selection bar, shortcuts, search, sort, filters, new folder, rename, favorites/recent, drop banner
│   ├── CloudOperation.svelte Review dialog and progress panel of copy/move/rename/delete/restore plans
│   ├── CloudFolderPicker.svelte  Destination folder picker (browse folders, create one)
│   ├── CloudProperties.svelte    Properties, size-limited preview (text as text, raster images only) and share links
│   ├── CloudBulkDownload.svelte  Panel of a download of several files and folders
│   ├── CloudDeleted.svelte   Deleted items of the folder (delete markers) with Restore
│   ├── CloudRecovery.svelte  Interrupted work from the operation journal (inspect, dismiss, abort an incomplete upload) and the per-bucket list of unfinished large uploads (ADR-030)
│   ├── CloudRecovery.test.ts Tests of the recovery view (mocked recovery commands)
│   ├── CloudUsage.svelte     The bucket view's Usage & costs tab (ADR-041): measure, progress, tiles, parts bar, included-storage meter, history chart and table, the estimate with components and assumptions; CloudUsage.test.ts its tests
│   ├── cloudUsage.ts         Typed wrappers for the three usage commands; validates responses and events; formatMoney
│   ├── usagePrefs.ts         The downloads per 30 days assumed per provider account (webview storage)
│   ├── CloudSnapshots.svelte The bucket view's Snapshots & backups tab (ADR-041): the provider's snapshots explained, sotyware backup snapshots of the bucket (details, Back up now, Verify, Delete with typed confirmation); CloudSnapshots.test.ts its tests
│   ├── CloudSync.svelte      The bucket view's Sync tab (ADR-043): choose a folder (dialog opened by Rust) and confirm what was found, the live state, folder, figures, transfers, the deletion guard's question, conflicts, activity, problems, not synced, interval, sync trash, pause, change folder, turn off; CloudSync.test.ts its tests
│   ├── folderSync.ts         Typed wrappers for the 16 folder sync commands; validates responses; state labels and the sidebar's summary
│   ├── folderSyncModel.svelte.ts  The live folder sync state (one watch_folder_syncs subscription), shared by the sidebar and the Sync tab
│   ├── Autoclicker.svelte    The Autoclicker page (ADR-050, docs/architecture/automation.md): header with the profile switcher, availability banners, the running session's bar, the tabs; its own JS chunk (+ Autoclicker.test.ts)
│   ├── AutoclickerOverview.svelte, AutoclickerClicker.svelte, AutoclickerRecorder.svelte, AutoclickerMacros.svelte, AutoclickerEditor.svelte, AutoclickerProfiles.svelte, AutoclickerSettings.svelte  Its tabs: status and controls; the clicker (simple/advanced, autosaved); recording; the macro library and the macro editor; profiles; shortcuts and safety
│   ├── HotkeyInput.svelte    A shortcut field that records the keys pressed (Esc cancels, Backspace clears); the autoclicker's and the notes overlay's settings
│   ├── overlayMain.ts        The notes overlay's entry (ADR-052): the styles, the dark theme, mounts NotesOverlay
│   ├── NotesOverlay.svelte   The notes overlay's page (docs/architecture/notes-overlay.md): title bar, tabs, search, the note rendered or edited, formatting bar, menus, passive and collapsed states (+ NotesOverlay.test.ts)
│   ├── NoteView.svelte       A note's Markdown rendered with Svelte's own elements: checkboxes, links, click to edit at the line
│   ├── noteMarkdown.ts       Pure: the Markdown subset parsed into blocks and inlines, checklist toggling, the editor's keys (lists, indent, wrap, line kinds, links), search (+ noteMarkdown.test.ts)
│   ├── noteAutosave.ts       The autosave: half a second after typing, at least every two seconds, one save at a time per note, recovery after failures (+ noteAutosave.test.ts)
│   ├── notesOverlay.ts       Typed wrappers for the 19 overlay commands; validates every response; OverlayError
│   ├── overlayModel.svelte.ts  The overlay's live status for the dashboard (one watch_overlay subscription), its configuration; opens Settings › Notes overlay when the overlay asks
│   ├── NotesOverlaySettings.svelte  Settings › Notes overlay: on/off, show/hide, mode, look, size and place, notes, shortcuts, fullscreen games, reset (+ NotesOverlaySettings.test.ts)
│   ├── PlaybackForm.svelte   A macro's playback settings (repeat, speed, delays, jitter, positions, time limit, repeated part)
│   ├── automation.ts         Typed wrappers for the 29 automation commands; validates every response; AutomationError
│   ├── automationFormat.ts   Pure helpers: durations, rates, shortcuts from key presses and their rules, action labels, timeline, summaries (+ automationFormat.test.ts)
│   ├── automationModel.svelte.ts  The live automation state (one watch_automation subscription), the configuration and the library, shared by the sidebar's indicator and the page
│   ├── cloudRecovery.ts      Typed wrappers for the five recovery commands (ADR-030); validates responses
│   ├── Backup.svelte         The Backups section (ADR-031): jobs with health, form, run/verify/retention tasks with progress and Cancel, snapshots, restore, recovery file, protection
│   ├── Backup.test.ts        Tests of the Backups section (mocked backup commands and channels)
│   ├── backup.ts             Typed wrappers for the eleven backup commands; validates responses and events; the health labels
│   ├── Account.svelte        The Account section (ADR-032): sign in, create, verify, reset; set up sync with the recovery key shown once and retyped; unlock by recovery key or approval with a fingerprint; sync, conflicts, devices, requests, revoke, rotate, password, delete
│   ├── Account.test.ts       Tests of the Account section (mocked account commands)
│   ├── account.ts            Typed wrappers for the 20 account commands
│   ├── Updates.svelte        The Overview's Updates panel (ADR-033): version, check, Update now / Later, progress, Cancel, verification and failure messages
│   ├── Updates.test.ts       Its tests (mocked update commands and channels)
│   ├── updates.ts            Typed wrappers for the five update commands; validates responses and events
│   ├── Feedback.svelte       Settings › About & updates: "Send feedback", "Report a bug" and the privacy notice link (ADR-049)
│   ├── Feedback.test.ts      Its tests (mocked open_github_page)
│   ├── githubPage.ts         Typed wrapper for open_github_page (also used by Account.svelte's privacy notice link)
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
│   ├── format.ts             formatSize, formatDecimalSize, formatAgo and shownName
│   ├── Features.svelte       Minimal Features section of the dashboard (state + enable checkbox per feature)
│   ├── Features.test.ts      Tests of the Features section and of features.ts
│   └── features.ts           Typed wrappers for list_features / set_feature_enabled; validates responses
├── package.json              npm scripts and JS dependencies
├── server/                   The account and sync service (ADR-032), its own crate and Cargo.lock; operations: docs/development/server.md, first deployment: docs/development/server-setup.md
│   ├── src/main.rs           sotyware-server: serve, backup <file>, restore <file>, send-test-mail <address>, outbox (development)
│   ├── src/mail.rs           Delivery of the outbox through an SMTP relay (SmtpConfig from SOTYWARE_SMTP_*, Mailer, retries; ADR-044)
│   ├── src/lib.rs            AppState, Config (token lifetimes, Argon2 cost, proxy trust)
│   ├── src/api.rs            The /v1 HTTP API (axum): accounts, sessions, devices, enrollments, recovery wrap, vault, audit events, health
│   ├── src/db.rs             SQLite schema (append-only MIGRATIONS), queries, online backup and checked restore
│   ├── src/security.rs       Argon2id, random tokens/codes/ids, SHA-256 storage of secrets, constant-time comparison, rate limits, lockout
│   ├── ops/sotyware-backup   Hourly backup script for the VM (ADR-047): online backup, age encryption, SigV4 PUT to Backblaze B2 through curl, ETag check, local pruning, monitor ping; ops/sotyware-backup.service and .timer: its systemd units
│   ├── tests/api.rs          The API over HTTP in process (fresh database, controllable clock)
│   └── tests/mail.rs         Mail delivery against an SMTP server in the test process (sent, retried, dropped; a registration code by mail)
├── scripts/
│   ├── check-rfd-patch.mjs   Guard (npm run check:rfd-patch): fails unless Cargo resolves rfd 0.17.2 from src-tauri/patches/rfd (ADR-023)
│   ├── check-rfd-patch.test.mjs  Its tests (node --test, npm run test:rfd-patch-guard) against broken temporary copies of src-tauri
│   ├── build-release.mjs  The normal release build (npm run build:release): tauri build with --remap-path-prefix for home, RUSTUP_HOME, CARGO_HOME and the checkout, then --local-paths on the binary (ADR-028)
│   ├── build-release.test.mjs  Its tests (in npm run test:release-binary-guard)
│   ├── check-release-binary.mjs  Guard against shipping a test build, the request recorder (ADR-024) or a test profile (ADR-027), or a binary naming the build machine's paths (ADR-028): --manifest (npm run check:release-binary), a binary scan, --local-paths, and Tauri's beforeBundleCommand (--before-bundle)
│   ├── check-release-binary.test.mjs  Its tests (node --test, npm run test:release-binary-guard)
│   ├── check-desktop-isolation.mjs  Static check of tests/desktop (npm run check:desktop-isolation, in test:headless): no driver names the user's app folder or credential namespace, moves or deletes a database, deletes a credential directly, or launches without the test-profile checks (ADR-027)
│   ├── check-desktop-isolation.test.mjs  Its tests (node --test, npm run test:desktop-isolation-guard)
│   ├── check-installer-icons.mjs  Reads the MSI (compound file, MSI tables), the NSIS setup program (PE resources, solid LZMA data, uninstaller icon patch) and executables, and checks their icons equal src-tauri/icons/icon.ico; run by release-build.yml (docs/development/icons.md#installer-icons)
│   ├── check-installer-icons.test.mjs  Its tests with in-memory fixtures (node --test, npm run test:installer-icons)
│   ├── icons.mjs         The application icons from src-tauri/icons/source (npm run icons, through tauri icon; writes icon.ico itself) and their check (npm run check:icons, in test:headless)
│   ├── icons.test.mjs    Its tests (node --test, npm run test:icons)
│   ├── update-manifest.mjs  Creates latest.json from signed updater packages of every platform, verifies a signed manifest, its platforms, URLs and packages, checks one signed file (ADR-033, ADR-036; signing itself is tauri signer sign --app-version)
│   ├── update-manifest.test.mjs  Its tests (node --test, npm run test:update-manifest)
│   ├── release-assemble.sh  release.yml's signing and checking step (publish and rehearse jobs): picks the built files (macOS only when both builds are signed and notarized), signs them and latest.json with tauri signer sign, verifies with the public key alone, writes SHA256SUMS (ADR-039)
│   ├── release-assemble.test.mjs  Its tests: bash, the real Tauri CLI, throwaway keys (node --test, npm run test:release-assemble)
│   ├── fixtures/update/  A manifest and two stand-in installers signed once by the real Tauri CLI with a throwaway key (public key only; private key deleted); verified by the script's tests and src-tauri/src/updates/tests.rs
│   ├── fixtures/update-github/  A GitHub release's latest.json for all seven platforms and stand-in packages, made once with release.yml's own commands, the real Tauri CLI and a throwaway key (public key only); verified by the script's tests and src-tauri/src/updates/tests.rs
│   └── recovery/
│       └── sotyware-restore.py  Restores a backup snapshot without sotyware (Python standard library only, SigV4); tested against the in-process server by backup::tests (ignored by default, run in ci.yml) (ADR-031)
├── package-lock.json         Pinned JS dependency tree
├── vite.config.ts            Vite: port 1420, strictPort, ignores src-tauri/, output dist/; Vitest config
├── svelte.config.js          vitePreprocess (TypeScript in .svelte)
├── tsconfig.json             Strict TypeScript settings
├── src-tauri/                Rust crate and Tauri configuration
│   ├── Cargo.toml            Crate `sotyware`, lib `sotyware_lib`, dependencies, release profile
│   ├── Cargo.lock            Pinned Rust dependency tree
│   ├── build.rs              tauri_build with an app manifest declaring the app's commands; on Windows the linker embeds windows-app-manifest.xml into every binary (ADR-034)
│   ├── windows-app-manifest.xml  The Common Controls v6 manifest (a copy of tauri-build's default), embedded by the linker on Windows
│   ├── tauri.conf.json       Product name, identifier, window `main` (size, minimum size), CSP, bundles (incl. bundle.macOS), dev/build hooks
│   ├── update-key.pub        The update signing key's public half, compiled in (empty until generated; ADR-036)
│   ├── capabilities/
│   │   ├── default.json      Frontend permissions for window `main` (hide is added at runtime, tray mode only)
│   │   └── overlay.json      Permissions of the notes overlay's window `overlay` (capability notes-overlay, ADR-052): its 16 commands, dragging and resizing
│   ├── icons/                Application, window and tray icons, generated by npm run icons ([icons.md](../development/icons.md)): icon.ico (16–256 px), 32x32.png, 128x128.png, 128x128@2x.png, icon.png, icon.icns
│   │   └── source/           The SVG sources: sotyware.svg (master, 48 px and up), sotyware-32.svg (24/32 px), sotyware-16.svg (16 px)
│   ├── pricing/              Published prices as data (ADR-041): backblaze-b2.json, hetzner.json (format sotyware-pricing v1, compiled in by cloud/pricing.rs)
│   ├── patches/              Dependencies built from a local copy via `[patch.crates-io]` (ADR-023)
│   │   ├── README.md         What is patched, why, how to verify the copy, when to remove it
│   │   ├── rfd-0.17.2-osstring.patch  The full diff against the published crate
│   │   └── rfd/              rfd 0.17.2 as published, `com.rs` changed: Windows dialog paths as `OsString` + tests
│   └── src/
│       ├── main.rs           Binary entry point → sotyware_lib::run()
│       ├── lib.rs            App builder, OperatingMode and its selection, lifecycle (release_resources: what every exit and an update install release), operating_mode command, single instance, show_dashboard(), AppControl + unit tests; with the feature test-profile, the profile check, identifier, data and webview folders and acknowledgement (ADR-027)
│       ├── test_profile.rs   Only with the feature test-profile (and in unit tests): SOTYWARE_TEST_PROFILE and its marker validated, overlap with the app's own folders refused, CredentialNamespace of the profile, the acknowledgement file + tests (ADR-027)
│       ├── tray.rs           Tray icon, menu, TrayAction, unavailable_reason() (no Linux tray host) + unit tests
│       ├── title_bar.rs      Windows: the window-controls capability, then decorations off (ADR-040) + tests
│       ├── window_state.rs   Where the dashboard opens: default placement (half the primary work area), restore with visibility and scaling rules, tracking and saving (window-state.sqlite3) + tests (ADR-040)
│       ├── app_menu.rs       The macOS menu bar (About, Hide, Quit ⌘Q via AppControl, Edit, Window) + unit tests (ADR-037)
│       ├── updates.rs        Updates (ADR-033, ADR-036): latest.json from the latest GitHub release (bounded HTTPS redirects, one refetch), signed-manifest verification, version floor, platform choice (Windows, macOS, Linux), plugin cross-check, download with SHA-256 check, install gate, restart after an in-place install, the six commands; updates/tests.rs (test keys, local servers incl. GitHub-style redirects, the plugin on the mock runtime, the Tauri CLI fixtures)
│       ├── github_page.rs    open_github_page (ADR-049): builds the GitHub URL of the feedback or bug form (version and platform prefilled) or the privacy notice and opens it in the browser + tests
│       ├── storage.rs        SqliteStore: preferences.sqlite3, schema, MIGRATIONS, PreferenceStore impl; SqliteAccountStore: cloud-accounts.sqlite3, ACCOUNT_MIGRATIONS; SqlitePlacesStore: cloud-places.sqlite3, PLACES_MIGRATIONS; SqliteJournal: operations-journal.sqlite3, JOURNAL_MIGRATIONS (ADR-030); SqliteBackupStore: backup.sqlite3, BACKUP_MIGRATIONS (ADR-031); SqliteAccountState: account.sqlite3 (ADR-032); SqliteUpdateState: updates.sqlite3 (ADR-033); SqliteWindowState: window-state.sqlite3 (ADR-040); SqliteUsageStore: storage-usage.sqlite3 (ADR-041) + tests (incl. restarts with the built-in features)
│       ├── storage/folder_sync.rs  SqliteSyncStore: folder-sync.sqlite3 (ADR-043), SYNC_MIGRATIONS, the TEMP local_scan/remote_scan tables, the planning query, move sources, issues, activity + tests
│       ├── automation/       Autoclicker and macros (ADR-050, docs/architecture/automation.md): mod.rs (Automation service: sessions, shortcuts, library, profiles, settings, import/export, status, the 29 commands), claims.rs (which feature holds which global shortcut, shared with the notes overlay, ADR-052), model.rs (data and rules; pure), keys.rs (the key table, Key, Chord), timing.rs (Control, Clock, FakeClock, Rng), input.rs (Backend trait, Output and held input, panic release, MockBackend), clicker.rs, recorder.rs, player.rs (the engines), native.rs (the platform's backend, the import/export dialogs), windows.rs (SendInput, low-level hooks, RegisterHotKey; windows/desktop.rs: the opt-in real-desktop test), x11.rs (XTest, RECORD, XGrabKey; x11/tests.rs: the opt-in private-Xvfb test), tests.rs (the service with the mock backend)
│       ├── storage/automation.rs  SqliteAutomationStore: automation.sqlite3 (ADR-050), AUTOMATION_MIGRATIONS: settings, profiles, macros (JSON with a summary) + tests
│       ├── overlay/          Gaming notes overlay (ADR-052, docs/architecture/notes-overlay.md): mod.rs (NotesOverlay service: lifecycle, the window through OverlayHost, interactive/passive, collapse, placement tracking, shortcuts and their dispatcher, notes, settings, status, the 19 commands), model.rs (notes, order, settings, actions, state; pure), placement.rs (corners, restore, collapsed height; pure), host.rs (TauriHost: the window through Tauri), focus.rs (Windows: shown, hidden and styled without activation through SetWindowPos, a tool window, the passive styles, the foreground window taken and given back), tests.rs (the service with a fake window, an in-memory store and the mock backend)
│       ├── storage/notes.rs  SqliteNotesStore: notes.sqlite3 (ADR-052), NOTES_MIGRATIONS: notes, the settings and state documents + tests
│       ├── folder_sync/      Folder sync (ADR-043, docs/architecture/folder-sync.md): mod.rs (FolderSyncs service, live status, subscriptions, choosing folders, the 16 commands), model.rs (paths and names, exclusions, the decision table, conflict names, deletion guard, retries; pure), scan.rs (walk, stamps, hashes, marker, .sotyware-sync staging and trash, free space), engine.rs (one sync's task: passes, listing, planning cycles, actions with live checks), watch.rs (notify watcher, desktop), migrate.rs (moving a sync folder: renames, verified copies), tests.rs (end to end against the in-process servers)
│       ├── journal.rs        The operation journal (ADR-030): Journal, JournalLink, JournalStore trait, startup cleanup of sotyware's own recorded temporary files + tests
│       ├── account/          sotyware accounts and encrypted sync (ADR-032): mod.rs (AccountService, StateStore, the 20 commands), crypto.rs (envelopes, wraps, sealed boxes, recovery key, key proof, fingerprint), vault.rs (vault document, three-way merge), client.rs (HTTPS client, ring TLS), tests.rs (end to end with the service in process and several device profiles)
│       ├── backup/           Backups (ADR-031): mod.rs (Backups service, BackupStore trait, scheduler, the nine commands), model.rs (manifest format, plan, change check, retention, health; pure), engine.rs (run, verify, prune, restore over two S3Stores), tests.rs (disaster-recovery tests against two in-process servers, incl. the Python recovery tool)
│       ├── cloud/            Cloud storage: backend, the dashboard's account commands, the bucket browser, downloads, upload batches and file management (ADR-026)
│       │   ├── mod.rs        Module overview, re-exports SqliteAccountStore
│       │   ├── provider.rs   ProviderProfile data: backblaze-b2, hetzner, s3-compatible + tests
│       │   ├── account.rs    AccountConfig + resolve(), Accounts, AccountStore trait + tests
│       │   ├── credentials.rs  Secret, Credentials (validation, stored format), CredentialStore trait, CredentialError; test-only EnvCredentials, MemoryCredentialStore + tests
│       │   ├── keyring.rs    OsCredentialStore: Windows Credential Manager / Secret Service via keyring-core, entry names, error mapping + tests, opt-in real-backend test
│       │   ├── s3.rs         S3Store (aws-sdk-s3): check_bucket, list, list_folder (one delimited page), upload_file wrapper, download (streamed, progress, TransferControl cancel/commit, temporary file, final move), delete, SHA-256, errors + tests
│       │   ├── s3/
│       │   │   ├── upload.rs  S3Store::upload (ADR-021) and upload_with (Destination::Replace of an expected ETag and the stored modification time, ADR-043): first hashing pass with per-part SHA-256, destination HEAD, request bodies that re-hash and hold back their last chunk (commit point), stall timeout instead of the read timeout, multipart abort in its own task (open upload id on TransferControl; abort_incomplete_upload), HEAD verification, UploadError/RemoteOutcome, throttled UploadProgress
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
│       │   ├── usage.rs      Storage usage (ADR-041): measure (versions, fallback, unfinished uploads and parts), StorageUsage (one slot, history in storage-usage.sqlite3), the overview with per-account estimates, three commands (tests in usage/tests.rs)
│       │   ├── pricing.rs    Price lists from src-tauri/pricing/*.json, validation, the 30-day estimate with components and assumptions + tests
│       │   ├── recovery.rs   The five recovery commands over the operation journal: list, inspect (reads only), dismiss, list and abort incomplete multipart uploads (ADR-030); recovery/tests.rs: crashes simulated against the in-process servers
│       │   ├── drop.rs       CloudDrops: dropped paths kept in Rust, watch/unwatch/upload_cloud_drop + tests
│       │   ├── tests.rs      Storage operations against the in-process S3 server
│       │   ├── test_server/versioned.rs  In-memory S3 backend with versions and delete markers (HeadObject, ranged GetObject, PutObject, CopyObject, DeleteObject, ListObjectsV2, ListObjectVersions, GetBucketVersioning); records every version removed by id, tests only
│       │   ├── test_server/files_ui.rs  serve_files_for_ui: the local server of the file-management desktop test (a versioned and an unversioned bucket, seeded; dump, fail, add commands; fails if a real version was removed by id), tests only
│       │   ├── test_server/usage_ui.rs  serve_usage_for_ui: the local server of the storage usage desktop test (one versioned bucket with current files, older versions and a deleted file; writes what a measurement must find; fails if the app sent more than listings or the bucket changed), and a test that a measurement of the seed finds exactly that; tests only
│       │   ├── test_server/sync_ui.rs  serve_sync_for_ui: the local server of the folder sync desktop test (one versioned bucket, seeded; dump, put and delete commands as a second computer; fails if a real version was removed by id or the app sent a request folder sync and the connection check do not send)
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
│           ├── autoclicker.rs  The autoclicker feature (off by default): starts and stops the automation service (ADR-050)
│           ├── backup.rs     The backup-scheduler feature: starts and stops the backup scheduler (ADR-031)
│           ├── folder_sync.rs  The folder-sync feature: starts and stops the folder sync engines (ADR-043)
│           ├── notes_overlay.rs  The notes-overlay feature (off by default): starts and stops the gaming notes overlay (ADR-052)
│           └── updates.rs    The update-check feature: starts and stops the automatic update checks (ADR-033)
└── tests/
    └── desktop/              Desktop tests of the release binary, always a test-profile build (ADR-027)
        ├── check-ps-variables.ps1  Static check of the Windows test scripts (parses only, runs nothing): fails if one scope spells a variable two ways ($out/$Out are one variable in PowerShell), or if a name is hidden at run time (a function's own variable hiding the script's from what it calls, a block reading a variable of the function it is given to)
        ├── cloud-ui.sh       Opt-in Linux Cloud Storage end-to-end test (test-profile build): Xvfb + openbox, private D-Bus session, real GNOME Keyring; phases local, hang, browse (local listing server), download (local object server, GTK save dialog), upload (local s3s-fs server, GTK multi-file dialog, stub tray watcher), files (file management against serve_files_for_ui), usage (the Usage & costs tab against serve_usage_for_ui), sync (folder sync against serve_sync_for_ui), account (a local account service), autoclicker (real XTest input on the private display only), overlay (the gaming notes overlay, real XTest input on the private display only), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key)
        ├── cloud_ui.py       Its driver: add, check, open bucket, browse, download through the save dialog, rename, replace key, remove through AT-SPI + XTest; keyring and database checked from outside
        ├── cloud_ui_upload.py  Its upload phase (run by cloud_ui.py): batches through the GTK multi-file dialog, server state checked after each, exact-key cleanup
        ├── cloud_ui_files.py  Its files phase (ADR-026, run by cloud_ui.py): folders, files, conflicts, partial failures and retries, delete and restore, selection with the real pointer, sorting, filters, search, previews, share link, folder uploads and several-item downloads through the GTK folder dialog (unsafe names, no overwrite, refused file and retry, cancel), a drop from another program (drag_source.py), sotyware's own clipboard with real keys, unversioned delete; server state checked after each step
        ├── cloud_ui_usage.py  Its usage phase (ADR-041, run by cloud_ui.py): measure, the figures against what the server says a measurement finds, the estimate and the expected downloads, the history, and at three window sizes nothing on the Usage & costs and Snapshots & backups tabs past its section or the page's scroll area, <main> (AT-SPI extents; the window resized through the window manager)
        ├── cloud_ui_sync.py  Its sync phase (ADR-043, ADR-046, run by cloud_ui.py): the Sync tab through the GTK folder dialog, both directions, rename, the sync trash, a conflict, a restart with the startup connection check, turning sync off; disk and server checked after each step
        ├── cloud_ui_account.py  Its account phase (ADR-049, ADR-032, run by cloud_ui.py): the feedback, bug report and privacy notice pages (recorded by the test-profile build), sign-up, verification from the outbox, sync set-up with the recovery key and deletion against a local sotyware-server; its database checked after each step
        ├── cloud_ui_autoclicker.py  Its autoclicker phase (ADR-050, run by cloud_ui.py): real input through XTest on the script's private display only, into a window of the driver that records what it receives (with the X server's times); turning the feature on and off (shortcuts grabbed and released), the clicker (buttons, double clicks, speed timed), F6/F7/F8 and a screen corner, sotyware's own window, recording with F9/F10, playback (repetitions and speed timed, F7, F8, Shift+F11), shortcut fields and their refusals, the confirmation without an emergency stop, the editor, restarts, profiles, export and import through the GTK dialogs, held keys released, 100 clicks per second, the Wayland message
        ├── cloud_ui_overlay.py  Its overlay phase (ADR-052, run by cloud_ui.py): real input through XTest on the script's private display only, a window of the driver playing the game; turning the overlay on and off (its window's properties and place, the shortcuts grabbed and released), a note typed and autosaved, a checkbox, Ctrl+Shift+F1–F5 from the game, clicks staying on the overlay and, passive, reaching the game, collapse, a new corner, hiding, a restart
        ├── cloud_ui_gtk.py   Helpers shared by the files, sync, autoclicker and overlay phases (exec'd by them): find(), the XTest pointer and the GTK folder dialog
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
        ├── windows-autoclicker.ps1  Opt-in Windows test of the Autoclicker page on the real desktop (ADR-050): the Linux phase's checks through UI Automation and SendInput into a topmost window of the script; moves the pointer, clicks, presses keys and quits through the tray; refused without -ConfirmRealInput pointer-and-keys
        ├── windows-overlay.ps1  Opt-in Windows test of the gaming notes overlay (ADR-052) through UI Automation: unattended on a private desktop (styles, places, activation through GetGUIThreadInfo, notes, modes, a restart, turning it off), or with -ConfirmRealInput pointer-and-keys on the real desktop with real keys and clicks and a window of the script as the game
        ├── windows-cloud-ui.ps1  Opt-in Windows Cloud Storage end-to-end test through UI Automation (test-profile build, credentials in the profile's namespace); -Mode files (file manager against serve_files_for_ui, incl. folder uploads and several-item downloads through the Windows folder dialog), sync (folder sync against serve_sync_for_ui through the Windows folder dialog, with a restart for the startup check), account (the GitHub pages and sign-up against a local sotyware-server), local, browse (local listing server), download (local object server, Windows save dialog), upload (local s3s-fs server, Windows multi-file dialog; moves the mouse once for tray Quit), b2 (real Backblaze), b2-browse (temporary dataset at Backblaze, read-only key), b2-download (one temporary file downloaded from Backblaze through the dashboard and the save dialog), b2-upload (one batch uploaded to Backblaze through the dashboard and the multi-file dialog with a third key, refused without -ConfirmB2Upload; -Rehearse: the same against the local s3s-fs server) or b2-lifecycle -Scenario multipart|cancel|retry|leave|quit (one upload lifecycle behaviour at Backblaze with the release settings, refused without -ConfirmB2Lifecycle; -Rehearse: the same against the local s3s-fs server; quit moves the mouse once)
        └── windows-isolated.ps1  Unattended Windows test on a private desktop: window, hide button, close, launches (no tray)
```

## Where to look for…

| Question | Answer |
| --- | --- |
| Where does the app start? | `src-tauri/src/main.rs` → `run()` in `src-tauri/src/lib.rs` |
| Where are Tauri plugins registered? | `run()` in `lib.rs`, on the builder; the single-instance plugin must stay first |
| Where are Tauri commands defined? | `operating_mode` in `lib.rs`; `list_features` and `set_feature_enabled` in `features/mod.rs`; the six cloud account commands in `cloud/dashboard.rs`; `list_cloud_folder` in `cloud/browser.rs`; `download_cloud_file`, `cancel_cloud_download` and `retry_cloud_download` in `cloud/download.rs`; `upload_cloud_files`, `upload_cloud_folder`, `cancel_cloud_upload` and `retry_cloud_upload` in `cloud/upload.rs`; the file-management commands (ADR-026) in `cloud/operations.rs`, `cloud/manage.rs`, `cloud/bulk_download.rs`, `cloud/places.rs` and `cloud/drop.rs`; the recovery commands (ADR-030) in `cloud/recovery.rs`; the usage commands (ADR-041) in `cloud/usage.rs`; the folder sync commands (ADR-043) in `folder_sync/mod.rs`; the automation commands (ADR-050) in `automation/mod.rs`; the notes overlay commands (ADR-052) in `overlay/mod.rs`; the backup commands in `backup/mod.rs`; the account commands in `account/mod.rs`; the update commands in `updates.rs`; `open_github_page` in `github_page.rs`. All are registered with `invoke_handler` in `run()` and declared in the app manifest in `build.rs` (permissions `allow-<command, with ->` in `capabilities/default.json`, and for the notes overlay's window in `capabilities/overlay.json`) |
| Where is the notes overlay? | `src-tauri/src/overlay/` (service, model, placement, window host), its database in `src-tauri/src/storage/notes.rs`, the feature in `features/notes_overlay.rs`, the page `overlay.html` + `src/NotesOverlay.svelte` and the settings `src/NotesOverlaySettings.svelte` ([notes-overlay.md](notes-overlay.md)) |
| Where is the autoclicker? | `src-tauri/src/automation/` (service, engines, key table, native backends), its database in `src-tauri/src/storage/automation.rs`, the feature in `features/autoclicker.rs`, the page `src/Autoclicker*.svelte` ([automation.md](automation.md)) |
| Where is folder sync? | `src-tauri/src/folder_sync/` (engine, rules, scan, watcher, moving), its database in `src-tauri/src/storage/folder_sync.rs`, the feature in `features/folder_sync.rs`, the tab in `src/CloudSync.svelte` ([folder-sync.md](folder-sync.md)) |
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
| Where are tests? | Rust unit tests inline in `#[cfg(test)]` modules (cloud storage: also `cloud/tests.rs`, `cloud/s3/upload/tests.rs`, `cloud/browser/tests.rs`, `cloud/download/tests.rs`, `cloud/upload/tests.rs`, `cloud/operations/tests.rs`, `cloud/manage/tests.rs`, `cloud/bulk_download/tests.rs`, opt-in `cloud/integration.rs`, `cloud/integration/browse.rs`, `cloud/integration/download.rs`, `cloud/integration/upload.rs` and `cloud/integration/upload/ui.rs`); frontend tests in `src/*.test.ts` (`App.test.ts`, `Features.test.ts`, `CloudStorage.test.ts`, `CloudBrowser.test.ts`, `CloudUpload.test.ts`, `CloudFileManager.test.ts`, `fileList.test.ts`); desktop tests in `tests/desktop/` (`run.sh`/`virtual.sh` Linux, `windows.ps1`, `windows-isolated.ps1` Windows; opt-in Cloud Storage end-to-end: `cloud-ui.sh`, `windows-cloud-ui.ps1`; opt-in autoclicker page on Windows: `windows-autoclicker.ps1`); all headless tests: `npm run test:headless`; CI workflows in `.github/workflows/` ([testing.md](../development/testing.md#ci)) |
| Where is a dependency patched? | `src-tauri/patches/` (rfd, wired in by `[patch.crates-io]` in `src-tauri/Cargo.toml`; [README](../../src-tauri/patches/README.md), [ADR-023](decisions.md#adr-023-rfd-is-patched-locally-so-windows-dialog-paths-are-read-as-osstring)) |
| Where are versions set? | `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say `0.1.0`; keep them in sync |

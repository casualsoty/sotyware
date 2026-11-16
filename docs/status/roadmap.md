# Roadmap

Nothing on this page is implemented. Implemented features are listed only in
[status.md](status.md). Move an item there when it is done, and record any
architectural decision it needs in
[decisions.md](../architecture/decisions.md).

## Next: finish Milestone 0 (validation)

In priority order:

1. **Finish Windows validation.** Windows 11 is built and passes the headless
   suite and the automated
   [Windows desktop test](../development/testing.md#windows-desktop-test),
   including the NSIS-installed copy, at `e8834fd` (with the test fix `71442d3`);
   the built-in feature foundation passed the headless suite and the extended
   desktop test (90 checks) at `3a8a715`, from the build output only.
   Still to do: a person running the
   [manual checklist](../development/testing.md#manual-check-real-desktop-linux-or-windows)
   on Windows 11, the MSI installer (needs an elevated install), and everything on
   Windows 10. Record results in the [validation matrix](status.md#validation-matrix).
2. **Validate on real Linux desktops:** GNOME (with and without the AppIndicator
   extension), KDE Plasma, XFCE, and a real Wayland session. Blocked on access:
   the development machine has none
   ([survey](status.md#real-linux-desktop-availability-2026-11-10)); it needs a
   VM (for example VirtualBox with Ubuntu Desktop), a live USB session or a
   Linux machine, which is the user's decision. With the `ksni`
   backend, check in particular that the icon shows and that a left click opens the
   dashboard. Without a tray host (vanilla GNOME, an XEmbed-only panel), run the
   dashboard-only steps of the manual checklist.
3. **Install and run the bundles** (`.deb`, `.rpm`, AppImage, the Windows MSI). The
   NSIS installer is done on Windows 11 (silent mode), at `e8834fd`, before
   the cloud storage work.

## Next: first release (Windows 11 only)

Not started unless status.md says so. Scope and procedures:
[release.md](../development/release.md).

1. Done 2026-11-14 at `369d0cb`: `ci.yml`, `isolation-guard.yml` and a
   manual `release-build.yml` passed on GitHub
   ([status.md](status.md#ci-coverage-of-the-test-profile-and-recorder-tests-2026-11-14)).
2. Choose and obtain code signing (certificate or signing service), wire it
   into a protected, manual release job, verify the signatures.
3. Run the [installation test plan](../development/release.md#installation-test-plan-disposable-windows-11)
   on a disposable Windows 11 VM or machine, with signed installers.
4. Drive Windows drag and drop from Explorer and the keyboard shortcuts
   with real input once (the manual procedure in release.md, test-profile
   build only).
5. Done 2026-11-14: the placeholder icons are replaced
   ([status.md](status.md#application-icons-2026-11-14)); seen in the
   running test-profile app on Windows 11 at 100 % (window, taskbar, tray,
   [status](status.md#application-icons-in-the-running-app-on-native-windows-11-2026-11-14));
   pushed and passed `ci.yml`, `isolation-guard.yml` and `release-build.yml`
   at `f9a0c71` ([status](status.md#hosted-runs-at-f9a0c71-icons-pushed-2026-11-14));
   Explorer, Start menu, other scalings and the installers are part of the
   installation test (3).
6. Updates through GitHub Releases (ADR-036, ADR-039; implemented
   2026-11-15 but not usable). Decided 2026-11-15: the source repository
   `casualsoty/sotyware` is public and releases are published there.
   Left, each by the maintainer: push and let `ci.yml` and `macos.yml`
   run; make the repository settings
   ([release.md](../development/release.md#repository-settings)); create
   the `release` and `macos-signing` environments
   ([secrets](../development/release.md#secrets)); run `release.yml` as a
   dry run from `main`; generate the release key offline, commit its public
   half to `src-tauri/update-key.pub` and store the private half in
   `release` ([update signing key](../development/release.md#update-signing-key));
   set `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` or add Authenticode signing; then
   publish release N; install it on a disposable machine, publish N+1 and
   update through the notification, checking the data folder and
   credentials survive.
7. Run the redesigned dashboard through the Windows drivers
   (`windows.ps1`, `windows-cloud-ui.ps1`, updated but not run) and look at
   it in WebView2 at 100 % and 150 %.

## Desktop UX, usage, snapshots, accounts (after ADR-040 to ADR-042)

1. Build on native Windows 11 and run `tests/desktop/windows.ps1` (its new
   title-bar and placement checks have never run), then the manual title
   bar check in [testing.md](../development/testing.md#manual-check-title-bar-and-window-placement-windows-11),
   at 100 % and 150 % scaling and on two monitors.
2. Measure a real bucket at Backblaze (read-only) and compare with the
   Backblaze console; the same at Hetzner when an account exists.
3. Delete a sotyware snapshot at a real destination (a throwaway backup)
   and check what the provider kept.
4. Production account service: the decisions and resources in
   [server.md](../development/server.md#production-deployment-proposal-2026-11-15),
   then mail delivery from the outbox, then `PRODUCTION_SERVER`.

## macOS (after ADR-037)

1. Run `macos.yml` (both architectures; a pull request to `main` or a push
   to `main` starts it, then by hand with `package: true`); fix what the
   first real compile and test run finds.
2. Obtain an Apple Developer ID certificate and an App Store Connect API
   key; set the secrets in the `macos-signing` environment ([macos.md](../development/macos.md#signing-and-notarization));
   run `macos.yml` again and check signing and notarization.
3. On an Apple Silicon and an Intel Mac: the manual check in
   [macos.md](../development/macos.md#signing-and-notarization), then an
   update from one signed release to the next.
4. A monochrome template image for the menu bar icon.

Fixes found during validation come before anything below.

## Cloud storage (after the proof of concept)

The proof of concept is in [cloud-storage.md](../architecture/cloud-storage.md);
the provider recommendation (Backblaze B2, EU Central) is in
[cloud-providers.md](../architecture/cloud-providers.md#recommendation). In
order:

1. **Run the opt-in integration tests with real accounts.** Backblaze B2:
   done 2026-11-11 on WSL2 and native Windows 11, no profile correction needed
   ([status.md](status.md#backblaze-b2-against-the-real-provider-2026-11-11)).
   Hetzner: not run, needs the user's account
   ([how](../development/testing.md#cloud-storage-integration-tests)).
2. Confirm VAT and currency charges on the first Backblaze invoice, and set the
   bucket's lifecycle rule (keep only the last version).
3. Credentials in the OS credential store: done 2026-11-11 (ADR-017,
   [status.md](status.md#os-credential-store-2026-11-11)). Linux without a
   Secret Service stays unsupported for accounts by design. Open: a run on a
   real Linux desktop session (GNOME, KDE/KWallet) with an unlock prompt
   answered by a person.
4. Account management in the dashboard: done 2026-11-11 (ADR-018, not a
   feature; `cloud-accounts.sqlite3` stays separate;
   [status.md](status.md#cloud-storage-dashboard-2026-11-11)). Open:
   - the Backblaze connection check: done once on WSL2 2026-11-11
     (`backblaze_b2_connection_check`,
     [status.md](status.md#cloud-storage-ui-end-to-end-and-backblaze-connection-check-2026-11-11));
     and the UI tests against Backblaze (`tests/desktop/cloud-ui.sh b2`
     19/19 and `windows-cloud-ui.ps1 -Mode b2` 23/23, once each,
     [how](../development/testing.md#cloud-storage-ui-end-to-end-test));
     still to run: repeats of those (one run each so far), the
     round trip again (it calls the check too), and Hetzner's for the first
     time;
   - a person's run on a real Linux desktop (GNOME Keyring or KWallet with an
     unlock prompt) and on Windows 10.
5. Read-only bucket browsing: done 2026-11-11 (ADR-019, `list_cloud_folder`,
   [status.md](status.md#cloud-storage-bucket-browsing-2026-11-11)). Open:
   - seen at Backblaze with content on 2026-11-11: a temporary dataset of
     1,040 objects (nested and marker-only folders, odd names, 1,010 entries
     in one folder) created, browsed through the Rust listing and through the
     UI on WSL2 and Windows 11, and deleted by version id
     ([status.md](status.md#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11));
     still open: Hetzner for the first time, and a person's look at a real
     bucket on a real Linux desktop;
   - keys restricted to a file name prefix cannot reach their prefix in the
     UI ([limitations.md](limitations.md#cloud-storage)): decide whether the
     browser should start at a configured folder (the UI test needed a
     second, read-only key without a prefix);
   - Backblaze answers a continuation token it never issued with its first
     page instead of an error (seen with a made-up token; an expired one was
     not tried): decide whether Load more should detect a page that starts
     over (today the page only drops the repeated rows).
6. Single-file downloads from the browser: done 2026-11-12 (ADR-020,
   `download_cloud_file`, `cancel_cloud_download`, `retry_cloud_download`;
   native save dialog opened by Rust, progress on a Tauri channel, cancel,
   retry, one at a time;
   [status.md](status.md#cloud-storage-single-file-downloads-2026-11-12)).
   Open:
   - a download from a real provider: done once at Backblaze on 2026-11-12
     (WSL2, Rust download path without the dialog, a temporary file;
     `GetObject` returned `Content-Length` and the stored `sotyware-sha256`;
     [status.md](status.md#one-download-from-backblaze-2026-11-12)), and once
     through the native Windows 11 dashboard and save dialog on 2026-11-12
     ([status.md](status.md#real-backblaze-download-through-the-windows-dashboard-2026-11-12));
     still open: the UI against a real provider on Linux, and Hetzner;
   - a person's run with the dialogs on a real Linux desktop (GNOME/KDE,
     possibly the XDG portal instead of GTK's own dialog) and Windows 10;
   - decide whether stale `.sotyware-partial` files left by a killed process
     should be found and offered for removal.
7. Uploads: the backend lifecycle (ADR-021) and the batch commands
   (ADR-022: up to 100 picked files, one after another, never overwriting,
   cancel, retry, multipart cleanup in memory) are done, in-process server
   only ([status.md](status.md#upload-batches-backend-and-commands-2026-11-12)).
   The upload UI in the bucket browser (Upload files… in the current
   folder, per-file rows with progress, Cancel, Try again, skip and "needs
   inspection" explanations; leaving the browser cancels) is done too,
   tested with mocked commands only
   ([status.md](status.md#cloud-storage-upload-ui-2026-11-12)).
   The release binary's upload workflow with the native GTK multi-file
   dialog passed against the in-process server on the Linux virtual display
   ([status.md](status.md#cloud-storage-uploads-through-the-release-binary-on-a-virtual-display-2026-11-12)).
   The same workflow passed on the native Windows 11 desktop with the
   Windows multi-file dialog, except one defect
   ([status.md](status.md#cloud-storage-uploads-on-the-native-windows-11-desktop-2026-11-12)),
   fixed on 2026-11-13 by a locally patched rfd (ADR-023; native Windows 11
   upload and save dialog with an unpaired-surrogate name passed,
   [status.md](status.md#windows-dialog-paths-with-unpaired-surrogates-2026-11-13)).
   Drop the patch once a fixed rfd reaches `tauri-plugin-dialog`.
   The opt-in Backblaze upload test passed through the Rust batch path
   once on WSL2 and once on native Windows 11
   ([WSL2](status.md#uploads-at-backblaze-2026-11-13),
   [Windows](status.md#uploads-at-backblaze-on-native-windows-11-2026-11-13)).
   Uploads through the Windows dashboard and native dialog at Backblaze
   passed once (2026-11-13,
   [status.md](status.md#dashboard-upload-at-backblaze-on-native-windows-11-rerun-passed-2026-11-13)).
   **Next:** the same at a real provider on a real Linux desktop and on
   Windows 10.
   Later: whether incomplete multipart uploads need a persistent registry
   or a documented lifecycle rule.
8. File management (ADR-026: folders, rename, copy, move, delete,
   restore, search, previews, share links, several-item downloads, folder
   uploads, drag and drop, favorites): implemented 2026-11-14; audited
   and driven on the Linux virtual display and native Windows 11 against
   local servers, in test profiles
   ([status.md](status.md#file-management-desktop-tests-and-audit-2026-11-14)).
   **Next:**
   - drive what is still not driven: the folder dialogs (GTK and Windows),
     a drop from a real file manager, the clipboard, Ctrl/Shift-click with
     the pointer on a real Linux desktop;
   - with the user's approval and a disposable test prefix, one run at
     Backblaze per operation (does it accept `x-amz-copy-source-if-match`,
     return `x-amz-delete-marker`, allow `GetBucketVersioning` and
     `ListObjectVersions` with the test keys); Hetzner later;
   - decide on multipart copy above 5 GiB and a ZIP download only if
     needed.
9. ~~restic orchestration~~: superseded by sotyware's own backup engine
   (ADR-031, implemented 2026-11-15 against local servers). **Next for
   backups:** with explicit approval and isolated test buckets, one run of
   the backup → verify → restore cycle between two real providers
   (Backblaze B2 → Hetzner or a second B2 account), including a key without
   delete rights at the destination and a bucket with Object Lock; desktop
   driver steps for the Backups page; desktop notifications for failed
   scheduled backups (would need `tauri-plugin-notification`).
10. Cost estimates: done 2026-11-15 (ADR-041; usage measured by listing,
    price lists in `src-tauri/pricing/`, 30-day estimates;
    [cloud-storage.md](../architecture/cloud-storage.md#storage-usage-and-cost-estimates)).
    Next: one measurement at Backblaze and at Hetzner compared with their
    consoles' figures, and a check of the price lists before each release.

Synchronisation, folder watching and a virtual filesystem stay ideas until then (temporary share links exist,
ADR-026; public ACLs do not).

## Known follow-ups (not started, no design yet)

- CI follow-ups (the workflows exist since `5fe62b9`, see
  [testing.md](../development/testing.md#ci); `ci.yml` passed on both runners in
  its two automatic runs): record the real job durations and replace the cost
  estimate (`desktop-virtual.yml` has passed on GitHub too);
  `desktop-windows-isolated.yml` is
  settled (hosted jobs are elevated: NOT RUN, exit 4). Record results in
  [status.md](status.md#ci-github-actions-2026-11-10).
- Possibly, a separate decision: making the headless check required by branch
  protection (needs `paths-ignore` reworked), or the real-desktop Windows test on
  a self-hosted runner in a logged-on session (security review first).
- Drop `libayatana-appindicator3-1` from the `.deb` dependencies: the binary
  never loads it since `12a6185` ([limitations.md](limitations.md#linux-tray)).
  Separate packaging task, unrelated to dashboard-only mode.
- Optionally report the libayatana-appindicator registration race upstream. It no
  longer affects sotyware
  ([limitations.md](limitations.md#historical-libayatana-appindicator-registration-race)).
- With every Tauri upgrade, check that the Linux-only `tray-icon` declaration
  still unifies with Tauri's copy and still selects `ksni`, and drop it once Tauri
  lets apps choose the backend
  ([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)).
- Make the D-Bus fallback warning visible (for example by installing a logging
  subscriber), if that is wanted.
- Built-in features: the manual checklist's step 9 (a person, Windows and
  Linux, now including restarts), and optionally feature checks in the Windows
  private-desktop test (`windows-isolated.ps1`) and a run on the NSIS-installed
  copy. The Windows build, headless suite and real-desktop Features and
  restart-persistence checks passed on 2026-11-11. The foundation and its saved
  choices are implemented ([features.md](../architecture/features.md)). The
  items they deliberately leave out are under Ideas.
- Feature preferences and the Cloud Storage section: run `desktop-virtual.yml`
  once on GitHub with `EXPECTED_CHECKS: 119` (115 before the Cloud Storage
  checks); check whether the NSIS/MSI uninstallers leave
  `%LOCALAPPDATA%\com.sotyware.desktop` (and so `preferences.sqlite3`) behind,
  together with the existing open question about the webview data
  ([limitations.md](limitations.md#feature-preferences)).

## Ideas (not decided)

These have been mentioned as possible future directions. There is no design, no
decision and no code for any of them. Do not implement them without an explicit
request and a recorded decision.

- Autostart on login.
- More local persistence (settings, other data). Only the feature choices are
  stored today ([features.md](../architecture/features.md#persistence)); a new
  table would be a new migration in `storage::MIGRATIONS`.
- Automatic recovery from a corrupt preferences database (for example renaming
  it aside and starting fresh). Rejected for now in
  [ADR-015](../architecture/decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)
  in favour of manual recovery.
- A feature-management UI beyond the minimal Features section, events from
  Rust to the page for feature state, a way for a running feature to report a
  failure, and dependencies between features.
- Third-party plugins (possibly sandboxed WASM). The built-in feature registry
  ([ADR-014](../architecture/decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry))
  is not a plugin runtime.
- Synchronization.

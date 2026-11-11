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
   NSIS installer is done on Windows 11 (silent mode).

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
4. Wire the module in as a built-in feature with its commands, permissions and
   a minimal account UI (its own ADR; decide then whether the accounts table
   moves into `preferences.sqlite3`).
5. restic orchestration (child process, `--json`, exit codes), tried first
   against the B2 test bucket.
6. The cost estimator with pricing data files ([proposal](../architecture/cloud-storage.md#cost-estimation-proposal-not-built)).

Synchronisation, folder watching, scheduled backups, retention, sharing and a
virtual filesystem stay ideas until then.

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
- Replace the placeholder icons.
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
- Feature preferences: run `desktop-virtual.yml` once on GitHub with
  `EXPECTED_CHECKS: 115`; check whether the NSIS/MSI uninstallers leave
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

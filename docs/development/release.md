# Release builds, signing and installer testing

sotyware 0.1.0 has **not been released**. This page says what a first release
may claim, how its binaries are built, what signing needs, and how its
installers must be tested before anyone is told to install them. Nothing here
has been done for a release yet unless [status.md](../status/status.md) records
it with a date.

## Support scope

| Platform | Code and builds | Release support |
| --- | --- | --- |
| Windows 11 | built and tested (automated desktop tests on 25H2, headless CI) | **the first release's only target**, once the installers have passed the [installation test](#installation-test-plan-disposable-windows-11) |
| Windows 10 | the same binary; never run | **not validated, not claimed.** Do not list it as supported |
| Linux (X11, Wayland; GNOME, KDE, XFCE, …) | builds; tested only on WSL2 and virtual displays (Xvfb + openbox, stub tray host) | **not validated on a real desktop.** Releases carry the AppImage, deb and rpm for testing, labelled as such in the release notes |
| macOS (Apple Silicon, Intel) | builds and tests only on GitHub's hosted macOS runners ([macos.md](macos.md)); never run on a Mac a person used | **not validated.** Releases carry Mac packages only when they are signed and notarized; none can be until a Developer ID certificate exists |
| mobile | not a target | none |

Linux code and the Linux build stay: every change must keep both platforms
working ([cross-platform.md](cross-platform.md)). The scope is about what a
release promises, not what the code supports. Widen it only after the
platform has run on a real installation, and record that run in the
[validation matrix](../status/status.md#validation-matrix).

## Building a release

```powershell
npm ci
npm run build:release                    # release binary + MSI + NSIS on Windows
npm run build:release -- --bundles msi   # extra arguments go to `tauri build`
npm run build:release -- --no-bundle     # binary only
```

`scripts/build-release.mjs` (ADR-028) runs `tauri build` with rustc's
`--remap-path-prefix` for the home directory, RUSTUP_HOME, CARGO_HOME and the
checkout, passed in `CARGO_ENCODED_RUSTFLAGS`, and afterwards checks the binary
with `node scripts/check-release-binary.mjs --local-paths`. Without it, Rust's
panic locations put absolute dependency paths such as
`C:\Users\<name>\.cargo\registry\src\…` into the binary (601 of them in one
Windows build). Remapped, a panic location reads
`cargo\registry\src\index.crates.io-…\tokio-1.54.1\src\…` on Windows
(`cargo/registry/…` on Linux), which still names the crate, version and
file. Standard-library locations were already `/rustc/<commit>/library/…`. The release profile already strips symbols
(`strip = true`), and the MSVC linker writes only the PDB's file name.

Plain `npm run tauri build` still compiles, but **bundling is refused**: Tauri's
`beforeBundleCommand` (`check-release-binary.mjs --before-bundle`) rejects a
binary to be bundled that names any of those directories, as it rejects a
request-recorder or test-profile build (ADR-024, ADR-027). `--features`,
`--target`, `--target-dir`, `--debug` and `--config` are refused by the wrapper:
test builds use `npm run build:test-profile`, whose binaries are never
packaged. Debug, test and test-profile builds are not remapped; they keep
full paths for debugging.

Two things the remap does not cover, by design: a `[build] rustflags` from a
Cargo config file is overridden by the environment variable (the wrapper keeps
`RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` but cannot see config files), and a
dependency that stores `env!("CARGO_MANIFEST_DIR")` as a string would not be
remapped (none was found: the check after the build would fail).

Outputs under `src-tauri/target/release/`: on Windows `sotyware.exe`,
`bundle/msi/sotyware_<version>_x64_en-US.msi`,
`bundle/nsis/sotyware_<version>_x64-setup.exe`; on Linux `sotyware`,
`bundle/appimage/…AppImage`, `bundle/deb/…deb`, `bundle/rpm/…rpm`; on macOS
`bundle/macos/sotyware.app` and `bundle/dmg/sotyware_<version>_<arch>.dmg`
([macos.md](macos.md)). `release.yml` builds each on its own runner. Building the MSI makes WiX
validate it through Windows Installer, which writes "Windows Installer
installed the product" events (MsiInstaller 1033/11707) to the Application
log **without installing anything**; do not mistake those for an
installation.

Before a release, also run the manual `release-build.yml` workflow
([testing.md](testing.md#ci)): it builds the same way on a clean hosted
runner, unpacks both installers without installing them and checks that the
`sotyware.exe` inside each is the checked binary (apart from the
bundle-type marker Tauri patches in before each bundle). It then checks the
installers' own icons by reading their bytes
(`scripts/check-installer-icons.mjs`, [icons.md](icons.md#installer-icons)):
the MSI's Add/Remove Programs icon (`ARPPRODUCTICON` → `ProductIcon`) and
Start menu shortcut icon are byte-equal to `src-tauri/icons/icon.ico`, the
desktop shortcut takes its icon from `sotyware.exe`, and the icon groups of
the NSIS setup program, of the uninstaller it would write (rebuilt in
memory from the setup program's header and the uninstaller icon data) and
of every `sotyware.exe` hold exactly `icon.ico`'s images. Neither installer
is run. To run the same check locally after `npm run build:release`, see
[icons.md](icons.md#installer-icons). What Windows then shows (Explorer,
Start menu, Add/Remove Programs, other scalings) is still checked only in
the installation test below.

## Signing (not set up)

Nothing is signed today and no certificate exists. Do not add placeholder
credentials, a test certificate or a self-signed one to the repository or to
CI; an installer signed with an untrusted certificate is worse than an
unsigned one.

- **Certificate:** an Authenticode **code-signing** certificate from a
  publicly trusted CA (OV, or EV), or a managed signing service whose
  certificates chain to a trusted CA (Microsoft's Azure Trusted Signing,
  now called Artifact Signing, where it is available to the publisher; check
  the current eligibility rules). Since 2023 the private key of an OV or EV
  code-signing certificate must live in a hardware module (a USB token or a
  cloud HSM); a `.pfx` file on disk is no longer issued. The same
  certificate signs the MSI, the NSIS setup program and the `sotyware.exe`
  inside them.
- **How Tauri takes it** (Tauri 2, `bundle.windows` in `tauri.conf.json`):
  either `certificateThumbprint` (a certificate in the build machine's
  Windows certificate store) with `digestAlgorithm: "sha256"` and a
  `timestampUrl` (the CA's RFC 3161 server), or `signCommand`, a command
  Tauri runs for every file it signs (`%1` is the file), for HSM or cloud
  signing tools. Tauri then signs the binary, the NSIS uninstaller and setup
  program, and the MSI. Always timestamp: a timestamped signature stays valid
  after the certificate expires. Put only the thumbprint or the command in
  the repository, never a key or a password.
- **CI secrets:** keep signing out of `ci.yml` and the pull-request path.
  Sign in a manual or tag-triggered job bound to a protected GitHub
  **environment** (required reviewer, `main`/tags only) whose secrets are not
  given to pull requests. Prefer short-lived credentials (OpenID Connect
  federation to the cloud signing service) over stored keys; if a token or
  key password must be stored, store it as an environment secret, never in a
  file or log. The job must fail, not fall back to unsigned, when signing is
  configured but fails.
- **SmartScreen:** Microsoft Defender SmartScreen judges a downloaded file
  (one with the Mark of the Web) by the reputation of its signer and file.
  Unsigned, every new build starts with none and users see "Windows protected
  your PC". Signing lets reputation accumulate on the publisher's
  certificate instead of each file, but a new certificate also starts with
  little; an EV certificate no longer grants reputation by itself. Windows
  11's Smart App Control, where enabled, blocks unsigned programs outright.
  Distribution through a store or a package manager has its own checks; none
  is chosen.
- **Verifying:** on the signed outputs and on the installed
  `sotyware.exe`, `signtool verify /pa /all /v <file>` (Windows SDK) must
  report a valid chain and a timestamp, and PowerShell's
  `Get-AuthenticodeSignature <file>` must show `Status: Valid` with the
  expected signer. Check the MSI, the setup program, the uninstaller and the
  installed binary, and that the installed binary's hash equals the one
  checked before signing changed it (signing changes the hash: record the
  hash after signing).

## GitHub Releases

Releases are published on GitHub Releases of `casualsoty/sotyware`, which is
also where installations look for updates
([ADR-036](../architecture/decisions.md#adr-036-updates-come-from-the-latest-github-release-latestjson-signed-and-verified-as-in-adr-033),
[updates.md](../architecture/updates.md)). Three things must be true before
the first release; none of them is yet:

1. **The releases are public.** GitHub serves the assets of a private
   repository only to signed-in clients; installations are anonymous. The
   repository is private today, so `release.yml` refuses to publish (a dry
   run works). Either make the repository public, or publish releases to a
   separate public repository: then change `RELEASE_REPOSITORY` and
   `MANIFEST_URL` in `src-tauri/src/updates.rs` and the `gh release`
   commands in `release.yml` (with a token for that repository) in one
   reviewed change.
2. **A release key exists** and its public half is in
   `src-tauri/update-key.pub` ([below](#update-signing-key)).
3. **The secrets are set** in the GitHub environment `release`
   ([below](#secrets)). Protect that environment: required reviewer,
   deployment only from tags `v*`.

### Publishing a release

1. Raise the version in `package.json`, `package-lock.json` (`npm version
   --no-git-tag-version X.Y.Z` does both), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (`cargo update -p sotyware`) and
   `src-tauri/tauri.conf.json`; commit; let `ci.yml` pass on `main`.
2. Optional rehearsal: push the tag to a branch-protected fork, or run
   `gh workflow run release.yml --ref vX.Y.Z` (on the tag) with `dry_run`
   (the default for a manual run): everything is built, signed and
   verified, the files are kept as the `release-dry-run` artifact, no
   release is made.
3. Tag and push: `git tag -s vX.Y.Z -m "sotyware X.Y.Z" -m "<release
   notes>"`, `git push origin vX.Y.Z`. The tag's message becomes the
   release notes shown in the app.
4. `release.yml` runs: prepare (tag = every version; the secret matches the
   committed key; the repository is public), `ci.yml` on the tag, the
   Windows installers (`release-build.yml`, unpacked and checked), Linux
   (Ubuntu 22.04: AppImage, deb, rpm), macOS (`macos.yml` on both
   architectures), then publish: sign every updater package and
   `latest.json`, verify them with the public key alone, upload to a
   **draft**, compare the uploaded names and sizes, publish as a
   **pre-release**, download every file through its public URL and verify
   again (signatures, signed versions, SHA256SUMS), then mark it **latest**
   and check that `releases/latest/download/latest.json` serves it.
5. If any step fails, the previous release stays the latest: installations
   see nothing. Fix the cause and release a new patch version; never
   re-publish an older version or move a tag (installations refuse a lower
   version than they were offered, and a moved tag breaks the URLs in the
   signed manifest).

Assets of a release (version 0.2.0 shown):

| File | Platform | In `latest.json` as |
| --- | --- | --- |
| `sotyware_0.2.0_x64-setup.exe` (+ `.sig`) | Windows, NSIS | `windows-x86_64-nsis` |
| `sotyware_0.2.0_x64_en-US.msi` (+ `.sig`) | Windows, MSI | `windows-x86_64-msi` |
| `sotyware_0.2.0_amd64.AppImage` (+ `.sig`) | Linux | `linux-x86_64-appimage` |
| `sotyware_0.2.0_amd64.deb` (+ `.sig`) | Linux, Debian/Ubuntu | `linux-x86_64-deb` |
| `sotyware-0.2.0-1.x86_64.rpm` (+ `.sig`) | Linux, Fedora/openSUSE | `linux-x86_64-rpm` |
| `sotyware_0.2.0_aarch64.dmg`, `sotyware_0.2.0_aarch64.app.tar.gz` (+ `.sig`) | macOS, Apple Silicon (signed and notarized only) | `darwin-aarch64-app` |
| `sotyware_0.2.0_x64.dmg`, `sotyware_0.2.0_x64.app.tar.gz` (+ `.sig`) | macOS, Intel (signed and notarized only) | `darwin-x86_64-app` |
| `latest.json`, `latest.json.sig`, `SHA256SUMS` | all | |

Authenticode signing of the Windows installers ([above](#signing-not-set-up))
is not part of the workflow yet; when it is added it must happen in the
Windows build job, before the update signatures (signing changes the bytes).

### Secrets

| Secret (environment `release`) | Used by | Required |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | `release.yml` prepare (key check) and publish (signing) only | yes |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the same | yes, if the key has a password (it should) |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | `macos.yml` signing | for Mac packages ([macos.md](macos.md#signing-and-notarization)) |
| `APPLE_API_ISSUER`, `APPLE_API_KEY_ID`, `APPLE_API_KEY` | `macos.yml` notarization | for Mac packages |

`GITHUB_TOKEN` (with `contents: write` in the publish job only) creates and
edits the release. No secret is ever echoed; the Tauri CLI reads the
signing key from the environment.

### Update signing key

The key that signs every update is the trust root of every installation.

- **Generate** once, on a trusted machine (not CI):
  `npx tauri signer generate -w sotyware-update.key` with a strong
  password. It writes the private key (`sotyware-update.key`, encrypted
  with the password) and the public key (`.pub`, one base64 line).
- **Commit the public key**: put the `.pub` line into
  `src-tauri/update-key.pub` below its comments, in a reviewed commit. Only
  builds made from then on can be updated.
- **Store the private key**: as the `release` environment's secrets
  `TAURI_SIGNING_PRIVATE_KEY` (the key file's content) and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and an offline backup of both
  (password manager plus hardware or paper), never in the repository, a
  chat or a CI log. `release.yml` signs a probe with the secret and
  verifies it with the committed key before anything is built, so a
  mismatch stops the release at once.
- **Rotation** (planned, e.g. a person with access leaves): installations
  trust only the key compiled into them. Release one version signed with
  the **old** key whose `update-key.pub` holds the **new** key; once users
  have it, sign later releases with the new key. Installations that skip
  that release can no longer update and need a manual install.
- **Compromise** (the private key leaked): an attacker can sign updates for
  every installation that has the key. Remove the secrets, release at once
  (signed with the leaked key, since nothing else is trusted) a version
  holding a new key, announce it, and treat every later release signed with
  the old key as hostile. The version floor stops replays of old releases
  but not new versions signed with a stolen key.
- **Loss** (no backup of the private key): no installation can be updated
  again; users must install the next version by hand. Keep the backup.

### Testing updates locally

Never install an unsigned update or one signed with a real key outside a
release. The mechanism is tested without either:

- `cargo test` (`src-tauri/src/updates/tests.rs`): keys generated in the
  test; a local server that behaves like GitHub's redirects
  (`releases/latest/download/` → tag → asset store), the publish race,
  redirect limits, missing releases, replays, wrong keys, changed bytes,
  oversized manifests; the updater plugin on Tauri's mock runtime
  downloading and checking a package without installing it; and the
  real-CLI fixture `scripts/fixtures/update-github/` for all seven
  platforms.
- `npm run test:update-manifest`: the release tool with keys generated in
  the test and the same fixtures.
- A debug or test-profile build against a local server: generate a
  throwaway key (`npx tauri signer generate -w /tmp/k --ci -p x`), sign two
  test packages and a manifest the way `release.yml` does, serve the folder
  over loopback HTTP, and start the app with
  `SOTYWARE_UPDATE_ENDPOINT=http://127.0.0.1:<port>/latest.json` and
  `SOTYWARE_UPDATE_PUBKEY=<the .pub line>`. Release builds ignore both
  variables. Delete the throwaway key afterwards.
- **Before the first real update:** install release N on a disposable
  machine per platform, publish N+1, update through the notification, and
  record that the data folder, credentials and settings survived (not done
  yet).

## Installation test plan (disposable Windows 11)

**Never on a machine with a real sotyware installation or real data.** The
MSI shares its UpgradeCode with any sotyware already installed (it would
upgrade or replace it), and every normal build uses the real profile
`%LOCALAPPDATA%\com.sotyware.desktop` and the Credential Manager namespace
`com.sotyware.desktop`. The development machine has both. Use a disposable
Windows 11 VM or spare machine with a snapshot taken before step 1, a local
(non-admin) user plus an admin for per-machine installs, no real cloud key,
and a local or throwaway S3 test bucket. Reading the source is not a
substitute for this test.

Record for each step: Windows build, installer and file SHA-256, what was
seen (screenshots for dialogs), and pass/fail.

1. **Fresh install.** Check signatures as above (once signed). Run the NSIS
   setup as the normal user (per-user install), note the SmartScreen
   dialog. On a second snapshot run the MSI (per-machine, needs elevation).
   Check: install location, Start menu entry, uninstall entry (name,
   version, publisher), that no profile folder exists yet, that WebView2 is
   present or installed by the bootstrapper. The sotyware icon
   ([icons.md](icons.md)) on the setup program, the Start menu entry and
   the uninstall entry, Explorer, at 100 % and at a higher display scaling
   (`release-build.yml` checks only the icon bytes the installers carry,
   [icons.md](icons.md#installer-icons), not what Windows displays).
   (Window, taskbar and tray icons were already seen in a test-profile
   build at 100 % on the development machine,
   [status](../status/status.md#application-icons-in-the-running-app-on-native-windows-11-2026-11-14);
   Explorer and Start menu need this installation, because a test-profile
   build refuses to start from a shortcut without its profile.)
2. **First launch.** Start from the Start menu. Tray icon (in the
   hidden-icons flyout; the cube, legible on a light and a dark taskbar),
   taskbar and title-bar icon, dashboard opens, close hides, tray Open Dashboard,
   a second launch shows the existing window, tray Quit ends the process.
   The profile folder now holds `preferences.sqlite3` (if a feature was
   switched), `cloud-accounts.sqlite3`, `cloud-places.sqlite3` and
   `EBWebView`. Add a test account with a throwaway key (a local test
   server works); check that it lands in Credential Manager under
   `com.sotyware.desktop/cloud-account/<id>`; switch the example feature
   off; add a favorite.
3. **Upgrade from the previous version.** Start from a snapshot with the
   previous installer installed and used as in step 2, then install the new
   one of the same kind (MSI over MSI, NSIS over NSIS). With equal version
   numbers (0.1.0 → 0.1.0) note what each installer does; a real upgrade
   test needs a higher version number. Check: one uninstall entry, the new
   binary's hash, and **existing user data preserved**: the account, its
   credential, the feature choice and the favorite are still there; nothing
   in the profile was deleted. Mixing MSI and NSIS on one machine is not
   supported; note what happens.
4. **Uninstall** from Settings → Apps. Check: program files and Start menu
   entry gone; whether the profile folder and the Credential Manager entry
   remain (expected: both remain; the uninstaller does not delete user
   data unless the NSIS "delete app data" option is ticked).
5. **Optional app-data deletion.** On a fresh snapshot repeat step 4 with
   the NSIS uninstaller's app-data option ticked. Record exactly what is
   removed (the profile folder; the WebView2 data) and what is not: the
   Credential Manager entries are not known to the uninstaller and are
   expected to remain. That leftover must be documented for users.
6. **Credential-store behaviour.** After step 4 without data deletion,
   reinstall and launch: the account appears and its check works with the
   stored key. After step 5, the account is gone but its credential is left
   behind; adding an account with the same id must replace it, not fail.
7. **Reinstall after uninstall.** Install again (both kinds), launch, check
   step 2's points again, then uninstall and roll the VM back.

## Windows drag and drop and keyboard shortcuts

Both are implemented and covered by component tests; on Windows neither has
been driven with real input (the Linux virtual display ran the shortcuts
with real keys and a drop from a GTK program). They cannot be automated
without real input:

- A drop from Explorer is an OLE drag: the mouse button must be held over
  Explorer and released over the webview, so a test must move the user's
  real pointer on the interactive desktop.
- Real keys (`SendInput`) go to whichever window has focus on the input
  desktop; on the private desktop that `windows-isolated.ps1` uses there is
  no input, and posting key messages to WebView2's window is not a real key
  press.

So an automated run would take over the mouse and keyboard of the user's
desktop and must be agreed first (like `windows.ps1`). The safe procedure is
manual, with a **test-profile build** (`npm run build:test-profile`) started
in a profile made with `New-TestProfile` from `tests/desktop/TestProfile.psm1`
(and its acknowledgement checked with `Wait-TestProfileAck` before touching
the window), or in the disposable VM, against the local test server, never
the normal build:

1. Open a test account on the local server, open the bucket, go to an empty
   folder.
2. **Drop:** in Explorer select two files and one folder holding a file,
   drag them onto the file list, release. Expected: the "Dropped: 2 files
   and 1 folder (with everything in them)" notice with their names; Upload
   sends them into the folder shown; the upload panel ends with every file
   uploaded; the listing shows them. Drop the same files again: the
   existing ones are skipped, never overwritten.
3. **Keys** (focus on the file list, not in a text field): Down/Up move the
   selection, Shift+Down extends it, Ctrl+A selects all, Enter opens a
   folder, Escape clears the selection, F5 and Ctrl+R refresh, F2 starts a
   rename of one selected item, Delete opens a delete plan (cancel it),
   Ctrl+C then Ctrl+V in another folder opens a copy plan, Ctrl+X then
   Ctrl+V opens a move plan. Expected: each plan's review appears; nothing
   changes before it is started. In a text field (search, rename) the same
   keys type or edit text and trigger no file action.
4. Quit from the tray and remove the test profile and its credentials with
   `Remove-TestCredentials` and `Remove-TestProfile`.

Record the result in [status.md](../status/status.md) with the date and the
Windows build.

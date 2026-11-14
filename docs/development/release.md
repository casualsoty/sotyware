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
| Linux (X11, Wayland; GNOME, KDE, XFCE, …) | builds; tested only on WSL2 and virtual displays (Xvfb + openbox, stub tray host) | **not validated on a real desktop, not a release target.** No Linux package is published as stable |
| macOS, mobile | not targets | none |

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

Outputs under `src-tauri/target/release/`: `sotyware.exe`,
`bundle/msi/sotyware_<version>_x64_en-US.msi`,
`bundle/nsis/sotyware_<version>_x64-setup.exe`. Building the MSI makes WiX
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

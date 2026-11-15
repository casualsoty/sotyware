# macOS: development, builds, signing and notarization

Status: macOS is a **build target**
([ADR-037](../architecture/decisions.md#adr-037-macos-is-a-supported-build-target-keychain-a-menu-bar-separate-apple-silicon-and-intel-builds)),
**not a validated release platform**: no build has run on Mac hardware a
person looked at, and no signing certificate exists. What ran where:
[status.md](../status/status.md#desktop-redesign-macos-and-github-releases-updates-2026-11-15).
Release scope: [release.md](release.md#support-scope).

## What is Mac-specific

| Area | macOS behaviour | Code |
| --- | --- | --- |
| Credentials | the user's login keychain (generic password items, service `com.sotyware.desktop`, account `cloud-account/<id>`); a dismissed access prompt or a locked keychain is reported as refused, no keychain as unavailable | `cloud/keyring.rs`, `apple-native-keyring-store` |
| Menu bar | sotyware (About, Services, Hide, Hide Others, Show All, Quit ⌘Q), Edit (⌘Z, ⇧⌘Z, ⌘X, ⌘C, ⌘V, ⌘A: what lets the webview use the clipboard), Window (Minimize, Zoom, Close ⌘W = hide) | `app_menu.rs` |
| Tray | an icon in the menu bar; left click opens the dashboard, the menu holds Open Dashboard and Quit. Always tray mode | `tray.rs` |
| Dock | the Dock icon stays; a click on it shows the dashboard | `lib.rs` (`RunEvent::Reopen`) |
| Quit | ⌘Q, the tray's Quit, the Dock's Quit and logout all end in `RunEvent::Exit`, so transfers are cancelled and databases closed | `lib.rs` |
| Data folder | `~/Library/Application Support/com.sotyware.desktop/` (mode 0700) | Tauri `app_local_data_dir()` |
| Dialogs, drag and drop | native panels (`NSOpenPanel`/`NSSavePanel` through rfd), Tauri's drop events | unchanged |
| Updates | `darwin-aarch64-app` / `darwin-x86_64-app`: the plugin swaps `sotyware.app` in place, then **Restart now** | `updates.rs` |
| Shortcuts on the page | ⌘ instead of Ctrl; ⌘⌫ deletes in a bucket | `src/fileList.ts`, `src/navigation.ts` |

## Local development (on a Mac)

Requirements: macOS 13 or later to develop (the app itself targets 10.15
and later), Xcode Command Line Tools (`xcode-select --install`), Rust stable
(rustup), Node.js 20.19, 22.12 or 24 (as in [setup.md](setup.md)). No other
system package: WebKit, the Keychain and the menu bar are part of macOS.

```bash
npm ci
npm run tauri dev                      # debug build, the real login keychain
npm run test:headless                  # guards, Vitest, cargo test
npm run build:release                  # sotyware.app and the .dmg, unsigned
```

Outputs: `src-tauri/target/release/bundle/macos/sotyware.app` and
`src-tauri/target/release/bundle/dmg/sotyware_<version>_<arch>.dmg`, for the
architecture of the Mac you build on (`aarch64` or `x64`). To build for the
other architecture add the target (`rustup target add x86_64-apple-darwin`)
and use the CI instead of `--target`: `build:release` refuses `--target`
(ADR-028), and each architecture is tested on its own runner.

A debug or unsigned build writes keychain items that macOS ties to that
binary's signature: after a rebuild the Keychain asks "sotyware wants to use
your confidential information…" once per item. Choose **Always Allow**, or
remove test accounts afterwards. A signed release does not ask again across
updates signed with the same Developer ID. Desktop tests use a test-profile
build (ADR-027), whose credentials live under `com.sotyware.desktop.test`.

Opening an unsigned `.app` that was downloaded (CI artifacts) is blocked by
Gatekeeper; for testing only, right-click → Open, or remove the quarantine
attribute: `xattr -dr com.apple.quarantine sotyware.app`. Never tell users
to do that.

## Architectures

Separate builds, not a universal binary (ADR-037):

| Architecture | Runner | Update entry | Files |
| --- | --- | --- | --- |
| Apple Silicon (aarch64) | `macos-15` | `darwin-aarch64-app` | `sotyware_<v>_aarch64.dmg`, `sotyware_<v>_aarch64.app.tar.gz` |
| Intel (x86_64) | `macos-15-intel` | `darwin-x86_64-app` | `sotyware_<v>_x64.dmg`, `sotyware_<v>_x64.app.tar.gz` |

Minimum macOS: 10.15 (`bundle.macOS.minimumSystemVersion`; Tauri 2's own
minimum). Apple Silicon needs macOS 11 anyway. Only the runners' macOS
versions have run anything; older versions are not tested.

## CI

`.github/workflows/macos.yml`, started by hand or by `release.yml`
(macOS minutes are billed at about ten times the Linux rate on a private
repository, so it does not run on every push). Per architecture: the
headless checks of `ci.yml` (svelte-check, Vite build, the guards, Vitest,
fmt, clippy for the normal and the test-profile build, `cargo test` with the
in-process S3, updater and account-service tests), the OS credential store
round trip in a keychain made for the run (never the runner user's login
keychain), and with `package: true` the release build, its checks (no test
build, no build-machine path, `LSMinimumSystemVersion`, the binary's
architecture) and the release files (`.dmg`, `.app.tar.gz`, `status.txt`).

## Signing and notarization

Signing needs an Apple Developer Program membership (paid) and a
**Developer ID Application** certificate; notarization needs an App Store
Connect API key with the Developer role. Neither exists yet.

Repository secrets (environment `release`; never in a file, never printed):

| Secret | What |
| --- | --- |
| `APPLE_CERTIFICATE` | the Developer ID Application certificate and its private key as a `.p12`, base64 (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12`'s password |
| `APPLE_SIGNING_IDENTITY` | the identity's name, e.g. `Developer ID Application: <Name> (<TEAMID>)` |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID |
| `APPLE_API_KEY_ID` | the API key's ID |
| `APPLE_API_KEY` | the API key's `.p8` file contents |

What `macos.yml` does with them, each secret only in its step:

1. imports the `.p12` as **non-extractable** into a keychain made for the
   run (deleted with the runner), allowed for `codesign` only;
2. builds with only `APPLE_SIGNING_IDENTITY` in the environment: Tauri signs
   `sotyware.app` with the hardened runtime and a secure timestamp before it
   puts it into the `.dmg`; the step checks the signature of the app and of
   the copy inside the `.dmg`, and signs the `.dmg` if Tauri did not;
3. writes the `.p8` to a file for one step, submits the app (zipped with
   `ditto`) and the `.dmg` to `notarytool --wait`, staples both, checks the
   app with `stapler validate` and `spctl --assess`, removes the file;
4. records `signed=` and `notarized=` in `status.txt`.

Without the secrets the build is an **unsigned development build**:
`status.txt` says `signed=false`, and `release.yml` leaves macOS out of the
release and out of `latest.json`. A release never offers an unsigned or
unnotarized Mac package as an update.

No entitlements are needed: sotyware is not sandboxed, WebKit's JIT runs in
the system's WebContent process, and the legacy keychain needs no access
group. The app inside the `.dmg` is notarized with the `.dmg` but not
stapled itself (the `.dmg` is made before notarization); Gatekeeper checks
it online on first launch. The `.app.tar.gz` used for updates holds the
stapled app.

**Before calling macOS supported:** run a signed, notarized build on an
Apple Silicon and an Intel Mac (first launch from the `.dmg`, Gatekeeper,
tray, menu bar, ⌘Q, Dock click, an account in the Keychain, browse, upload,
download, a folder upload and a several-item download through the native
panels, drag and drop), then install an update from one signed release to
the next, and record it in the [validation
matrix](../status/status.md#validation-matrix).

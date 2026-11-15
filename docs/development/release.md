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

Releases are built, tested and signed in the private source repository
`casualsoty/sotyware` and published on GitHub Releases of the **public
release repository `casualsoty/sotyware-releases`**, which is also where
installations look for updates
([ADR-036](../architecture/decisions.md#adr-036-updates-come-from-the-latest-github-release-latestjson-signed-and-verified-as-in-adr-033),
[ADR-038](../architecture/decisions.md#adr-038-releases-are-published-to-a-separate-public-repository-casualsotysotyware-releases),
[updates.md](../architecture/updates.md)). The release repository holds
distribution files and release metadata only; the source code is never
pushed there. The repository name is written once, in the
`release_repository!` macro in `src-tauri/src/updates.rs`, compiled into
every build and read from there by `release.yml`.

Before the first release, all of the following must be true; **none of
them is yet**:

1. **The release repository exists and is public**
   ([below](#the-public-release-repository)).
2. **A release key exists** and its public half is in
   `src-tauri/update-key.pub` ([below](#update-signing-key)).
3. **The secrets and the variable are set** ([below](#secrets)).
4. **Unsigned Windows installers are allowed on purpose**, as long as no
   Authenticode certificate exists ([above](#signing-not-set-up)): the
   repository variable `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` is `true`.
   Without it `release.yml` refuses to publish.

### The public release repository

What a release puts there, per tag: the installers and their `.sig` files,
`latest.json`, `latest.json.sig` and `SHA256SUMS` (table below). GitHub
also attaches "Source code (zip, tar.gz)" archives of the release
repository's own default branch to every release, which is why that branch
may hold only a README and similar notes: `release.yml` refuses to publish
if it finds anything other than `README.md`, `README`, `LICENSE`,
`LICENSE.md`, `LICENSE.txt`, `SECURITY.md`, `.gitattributes`, `.gitignore`
or `.github`.

Set it up once, by hand (sotyware's tools never create it):

1. On GitHub: **New repository**, owner `casualsoty`, name
   `sotyware-releases`, **Public**, tick **Add a README file** (a release
   needs at least one commit: its tag is created on the default branch).
   No `.gitignore` template is needed; a licence file is optional.
2. Edit the README to say what the repository is, for example: "Installers
   and update files of sotyware. The source code is not published here.
   Every package and `latest.json` carry a minisign signature of
   sotyware's release key."
3. In the repository's **Settings**:
   - **General → Features:** turn off Wikis, Issues, Projects and
     Discussions unless you want them (they would be public).
   - **Actions → General:** **Disable actions** (nothing runs there; the
     source repository's workflow publishes from outside).
   - **Rules → Rulesets → New tag ruleset:** target all tags (`*`),
     enforcement **Active**, rules **Restrict deletions** and **Block force
     pushes** (and **Restrict updates**), bypass list empty. A published
     tag must never move: the signed manifest names its URLs. Tags are
     still created by the release token when a draft is published.
   - **Rules → Rulesets → New branch ruleset** for the default branch:
     **Restrict deletions**, **Block force pushes**, **Require a pull
     request before merging** if others ever get access.
   - Optional: **General → Releases → Enable release immutability**, which
     locks a published release's assets and tag. sotyware's workflow
     uploads every file while the release is a draft, so it fits, but its
     last step changes a published pre-release into the latest release;
     check on the first release that GitHub allows that with immutability
     on (not tried).
4. Create the release token ([Secrets](#secrets)).
5. Nothing else: no workflow, no source, no branch besides the default one.

Check it from a machine that is not signed in to GitHub: the repository
page opens, and after the first release
`https://github.com/casualsoty/sotyware-releases/releases/latest/download/latest.json`
downloads.

### Publishing a release

1. Raise the version in `package.json`, `package-lock.json` (`npm version
   --no-git-tag-version X.Y.Z` does both), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (`cargo update -p sotyware`) and
   `src-tauri/tauri.conf.json`; commit; let `ci.yml` pass on `main`.
2. Optional rehearsal: `gh workflow run release.yml --ref vX.Y.Z` (on the
   tag; `dry_run` is the default for a manual run). Everything is built,
   signed with the real key and verified, the files are kept as the
   `release-dry-run` artifact (7 days; treat them as release material:
   they carry valid signatures), and no release is made. A dry run warns
   instead of failing when the release repository is missing or wrong, so
   the builds can be rehearsed first. GitHub starts a workflow by hand only
   once its file is on the default branch: `release.yml` must be on `main`.
3. Tag and push: `git tag -s vX.Y.Z -m "sotyware X.Y.Z" -m "<release
   notes>"`, `git push origin vX.Y.Z`. The tag's message becomes the
   release notes, **published** in the release repository and shown in the
   app.
4. `release.yml` runs: prepare (tag = every version; the release
   repository is not the source repository, is public and holds only
   notes; the tagged commit is on `main`; the release token exists; no
   release or tag `vX.Y.Z` exists there yet; unsigned Windows installers
   allowed; `update-key.pub` holds a real key, not a fixture's, and the
   signing secret matches it), `ci.yml` on the tag, the Windows installers
   (`release-build.yml`, unpacked and checked), Linux (Ubuntu 22.04:
   AppImage, deb, rpm), macOS (`macos.yml` on both architectures), then
   publish: sign every updater package and `latest.json`, verify them with
   the public key alone, upload to a **draft** in the release repository
   (visible only to you), compare the uploaded names and sizes, publish as
   a **pre-release**, download every file through its public URL without
   a token and verify again (signatures, signed versions, SHA256SUMS),
   then mark it **latest** and check that
   `https://github.com/casualsoty/sotyware-releases/releases/latest/download/latest.json`
   serves it. One release runs at a time.
5. If any step fails, the previous release stays the latest: installations
   see nothing. A failed run can leave a draft or a pre-release in the
   release repository: delete that release there (a tag it created can
   stay: the version is used up either way), fix the cause and release a
   new patch version. Never re-publish an older version or move a
   published tag (installations refuse a lower version than they were
   offered, and a moved tag breaks the URLs in the signed manifest).

Assets of a release (version 0.2.0 shown), all under
`https://github.com/casualsoty/sotyware-releases/releases/download/v0.2.0/`:

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
Windows build job, before the update signatures (signing changes the bytes),
and replace the `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` opt-in with a check that
the installers are signed.

### Secrets

GitHub plans decide where secrets can live. **Environments and environment
secrets are not available to private repositories on GitHub Free**; they
need GitHub Pro (personal accounts), Team or Enterprise. Required
reviewers and wait timers are not available to private repositories on
Free, Pro or Team at all. `release.yml` names the environment `release` in
its `prepare` and `publish` jobs, the only jobs that read the update key
and the release token.

| Name | Kind and where | Used by | Required |
| --- | --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | secret, environment `release` | `release.yml` prepare (probe) and publish (signing), one step each | yes |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret, environment `release` | the same steps | yes (the key must have a password) |
| `SOTYWARE_RELEASES_TOKEN` | secret, environment `release` | `release.yml` prepare (does the release exist?) and the publish steps that run `gh release` | yes, outside a dry run |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | secrets, **repository** | `macos.yml` signing | for Mac packages ([macos.md](macos.md#signing-and-notarization)) |
| `APPLE_API_ISSUER`, `APPLE_API_KEY_ID`, `APPLE_API_KEY` | secrets, **repository** | `macos.yml` notarization | for Mac packages |
| `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` | variable, repository | `release.yml` prepare | `true` while no Authenticode certificate exists |

The Apple secrets must be repository secrets: `release.yml` hands them to
the reusable `macos.yml`, and a job that calls a reusable workflow cannot
name an environment, so environment secrets would never reach it (the Mac
builds would silently be unsigned and left out). `release.yml` passes
exactly those six, never `secrets: inherit`.

**Set up** (Settings of `casualsoty/sotyware`):

1. With GitHub Pro (or Team): **Environments → New environment**
   `release`. **Deployment branches and tags → Selected branches and
   tags → Add rule**, type **Tag**, pattern `v*`; remove any branch rule.
   Then **Environment secrets**: add `TAURI_SIGNING_PRIVATE_KEY`,
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` and `SOTYWARE_RELEASES_TOKEN`. A
   dry run is started on the tag too, so the tag rule allows it.
   On GitHub Free, environments are unavailable for a private repository:
   upgrade, or store the three as **repository** secrets (every workflow
   run of the repository could then read them; pull requests from forks
   still get none). Whether GitHub runs a job that names an environment the
   plan does not offer has not been tried; if `prepare` fails on it, that
   is the reason.
2. **Secrets and variables → Actions → Variables → New repository
   variable** `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` = `true` (only once you
   accept publishing unsigned Windows installers).
3. Apple secrets, when a Developer ID certificate exists: **Secrets → New
   repository secret** for each of the six ([macos.md](macos.md#signing-and-notarization)).
4. **Rules → Rulesets → New tag ruleset** in the source repository for
   `v*`: only you may create, update or delete such tags (pushing a `v*`
   tag starts a release). Rulesets on a private repository also need
   GitHub Pro or above; on Free, you are the only one with write access,
   which has the same effect while that stays true.

**The release token** (`SOTYWARE_RELEASES_TOKEN`): GitHub → your avatar →
**Settings → Developer settings → Personal access tokens → Fine-grained
tokens → Generate new token**. Name `sotyware-releases publishing`,
resource owner `casualsoty`, an expiry you will remember (renew it before
it lapses; a release fails at its first `gh release` step otherwise),
**Repository access → Only select repositories →
`casualsoty/sotyware-releases`**, **Repository permissions → Contents:
Read and write** (Metadata: Read-only is added automatically), nothing
else. Copy it once into the secret; do not keep another copy. Revoke and
replace it if it may have leaked: it can publish to the release
repository, but not sign, so installations would refuse what it published
(the release key is the authority).

`GITHUB_TOKEN` only reads the source repository (`contents: read`
everywhere, including the publish job). No secret is ever echoed; the
Tauri CLI reads the signing key from the environment.

### Update signing key

The key that signs every update is the trust root of every installation.
It is generated once, by you, on a trusted machine; never in CI, never by
an assistant, never a throwaway key (the fixtures' keys are test-only and
`npm run check:update-key` and `release.yml` refuse them).

1. **Generate** it on your own computer, offline if you can, in a folder
   outside every repository (not in a synced folder, not in WSL's
   `/mnt/c` if you can avoid it):

   ```bash
   mkdir -p ~/sotyware-release-key && cd ~/sotyware-release-key
   npx @tauri-apps/cli@2.12.4 signer generate -w ./sotyware-update.key
   ```

   Type a long, unique password at the prompt (a password manager's
   generated passphrase); do not pass it with `-p` (it would land in the
   shell history) and do not use `--ci` (no password). It writes
   `sotyware-update.key` (the private key, encrypted with the password,
   base64) and `sotyware-update.key.pub` (the public key, one base64 line).
2. **Commit the public key**: put the single line of
   `sotyware-update.key.pub` into `src-tauri/update-key.pub` below its
   comments, run `npm run check:update-key` (it prints the key when it is a
   valid public key, refuses a private key or a fixture key), and commit it
   in a reviewed commit. Only builds made from then on can be updated.
3. **Store the private key** in the secrets: `TAURI_SIGNING_PRIVATE_KEY` =
   the whole content of `sotyware-update.key` (one line, as written), and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = the password. With the GitHub
   CLI, without the key passing through the clipboard or the history:

   ```bash
   gh secret set TAURI_SIGNING_PRIVATE_KEY --env release --repo casualsoty/sotyware < ~/sotyware-release-key/sotyware-update.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release --repo casualsoty/sotyware
   ```

   (the second prompts for the value; leave out `--env release` for
   repository secrets on GitHub Free).
4. **Back up** the key file and the password separately and offline (for
   example the password in a password manager, the key file on two
   encrypted USB drives or printed), never in the repository, a chat, an
   issue or a CI log. Then delete the working copy from the computer.
5. **Check**: `gh workflow run release.yml --ref vX.Y.Z` with `dry_run`
   (once `release.yml` is on `main` and a version tag exists). Its first
   job signs a probe with the secret and verifies it with the committed key
   before anything is built, so a mismatch stops at once.

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
  test; the compiled-in key never being a fixture's; a local server that
  behaves like GitHub's redirects
  (`releases/latest/download/` → tag → asset store), the publish race,
  redirect limits, missing releases, replays, wrong keys, changed bytes,
  oversized manifests; the updater plugin on Tauri's mock runtime
  downloading and checking a package without installing it; and the
  real-CLI fixture `scripts/fixtures/update-github/` for all seven
  platforms.
- `npm run test:update-manifest`: the release tool with keys generated in
  the test and the same fixtures, the release repository read from
  `updates.rs`, and `check-key` (fixture keys, private keys and malformed
  key files refused).
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

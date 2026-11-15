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
([macos.md](macos.md)). `release.yml` builds each on its own runner. The
Linux packages are built on `ubuntu-22.04` so they run with glibc 2.35 and
newer; GitHub deprecated that runner on 2026-09-17 and removes it on
2027-04-17 (brownouts from 2027-03-23). Before then the Linux job must move
to `ubuntu-24.04`, which raises the floor to glibc 2.39 (Ubuntu 24.04,
Debian 13, Fedora 40 or newer); that changes which Linux systems a
release can run on and is the maintainer's decision. `ci.yml`'s Linux jobs
(and `isolation-guard.yml`) run on the same runner, so every push checks
the code against the libraries the packages are built with; move them
together. Building the MSI makes WiX
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

Releases are built, tested, signed and published by `release.yml` in this
repository, `casualsoty/sotyware`, which is public, and installations
look for updates in its GitHub Releases
([ADR-036](../architecture/decisions.md#adr-036-updates-come-from-the-latest-github-release-latestjson-signed-and-verified-as-in-adr-033),
[ADR-039](../architecture/decisions.md#adr-039-releases-are-published-in-the-public-source-repository-with-github_token-signing-credentials-live-in-protected-environments),
[updates.md](../architecture/updates.md)). The repository is written once,
in the `release_repository!` macro in `src-tauri/src/updates.rs`, compiled
into every build; `release.yml` refuses to publish anywhere else.
Publishing needs no token besides the workflow's own `GITHUB_TOKEN`.

Before the first release, all of the following must be true; **all five
are** (read back 2026-11-15, after the final dry run at `f846bdd`; the
installation test below is still to do):

1. **The repository settings are made** ([below](#repository-settings)).
2. **The `release` environment exists**, admits only `v*` tags and holds
   the signing key; `macos-signing` exists, admitting `main` and `v*`
   ([Secrets](#secrets)). `macos-signing` is needed before **any** run of
   `release.yml`, a dry run included: its macOS package job names it, and
   GitHub would create it without protection. Both were created by hand
   on 2026-11-15 with the rules in [Secrets](#secrets); `release` holds
   `TAURI_SIGNING_PRIVATE_KEY` and its password since that day,
   `macos-signing` no secret.
3. **A release key exists** and its public half is in
   `src-tauri/update-key.pub` ([below](#update-signing-key)): since
   2026-11-15, key id `3A6E8F2D58D68E0D`. Whether the secret is its
   private half is checked by step 4 there and by the publish job's probe.
4. **Unsigned Windows installers are allowed on purpose**, as long as no
   Authenticode certificate exists ([above](#signing-not-set-up)): the
   repository variable `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` is `true`.
   Without it `release.yml` refuses to publish.
5. **`release.yml` is on `main`** (it is, since `d9cf561`): GitHub starts
   a workflow by hand only once its file is on the default branch (a
   pushed tag starts it from the tagged commit's copy).

### Repository settings

Once, by hand, in **Settings** of `casualsoty/sotyware`. On 2026-11-15 the
repository had: Actions allowing every action, SHA pinning not required,
the default `GITHUB_TOKEN` read-only, Actions not allowed to approve pull
requests, approval required only for first-time contributors' fork pull
requests, no environment, no ruleset, no secret or variable, no
protection on `main`, release immutability off. Later that day the
maintainer made every setting below, release immutability included, and
created both environments ([Secrets](#secrets)); all read back the same
day ([status](../status/status.md#release-settings-complete-and-the-final-dry-run-2026-11-15)).

- **Actions → General**
  - **Actions permissions:** *Allow casualsoty, and select non-casualsoty,
    actions and reusable workflows* → *Allow actions created by GitHub*
    and, under *Allow specified actions*, `Swatinem/rust-cache@*`. Tick
    **Require actions to be pinned to a full-length commit SHA** (every
    workflow already pins them).
  - **Fork pull request workflows from outside collaborators:** *Require
    approval for all external contributors*. Fork pull requests never get
    secrets or a write token, but they run on hosted runners.
  - **Workflow permissions:** keep *Read repository contents and packages
    permissions* and leave *Allow GitHub Actions to create and approve pull
    requests* off (both already so). Each workflow also declares its own
    permissions.
- **Rules → Rulesets → New tag ruleset**, twice, both enforcement
  **Active** and target **Include by pattern** `v*`:
  - `release tags: who` with **Restrict creations**, bypass list
    **Repository admin** only: pushing a `v*` tag starts a release, so only
    you can create one;
  - `release tags: never move` with **Restrict updates**, **Restrict
    deletions** and **Block force pushes**, bypass list **empty**: a
    published tag must never move (the signed manifest names its URLs),
    not even by you without first editing the ruleset.
- **Rules → Rulesets → New branch ruleset** for `main` (the default
  branch): **Restrict deletions**, **Block force pushes**; **Require a pull
  request before merging** once anyone else has write access. Only commits
  on `main` are released.
- **General → Releases → Enable release immutability** (recommended): a
  published release's files and tag can no longer change. `release.yml`
  uploads every file while the release is a draft and afterwards changes
  only its pre-release and latest flags, which GitHub documents as still
  editable on an immutable release (not yet tried here). The cost: a
  release that fails after it became a pre-release cannot be repaired, only
  deleted, and its version is used up.
- **Environments:** `release` and `macos-signing` ([Secrets](#secrets)).

Read them back:

```bash
gh api repos/casualsoty/sotyware/actions/permissions
```

```bash
gh api repos/casualsoty/sotyware/environments --jq '.environments[] | {name, protection_rules, deployment_branch_policy}'
```

### Publishing a release

1. Raise the version in `package.json`, `package-lock.json` (`npm version
   --no-git-tag-version X.Y.Z` does both), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (`cargo update -p sotyware`) and
   `src-tauri/tauri.conf.json`; commit; let `ci.yml` and `macos.yml` pass
   on `main`.
2. Rehearse (recommended before the first release and after workflow
   changes): `gh workflow run release.yml --ref main` (`dry_run` is the
   default for a manual run). **Create `macos-signing` first**
   ([Secrets](#secrets), step 2): the macOS package job runs in it, and a
   running job that names a missing environment makes GitHub create it
   without protection. Nothing else has to exist: the release key, the
   `release` environment and the repository variable are not used, and
   `prepare` warns about each. The skipped publish job names `release`,
   but a skipped job creates no environment (`macos.yml`'s package job,
   skipped on every push, never created `macos-signing`). First passed at
   `8125739` (run 43735919504, 23 min;
   [status](../status/status.md#release-dry-runs-2026-11-15)), then with
   the release key and every setting at `f846bdd` (run 43746112094, 22.5
   min, no warning;
   [status](../status/status.md#release-settings-complete-and-the-final-dry-run-2026-11-15)).
   - **What runs:** `prepare` (the versions must agree; the repository in
     `updates.rs`, the release key, repository secrets, both environments
     and the Windows opt-in are only warned about), `ci.yml`, the Windows
     installers, the Linux packages, the macOS tests and unsigned
     packages, then `rehearse`: every updater package and `latest.json`
     signed with a **throwaway key** made in the run and verified with its
     public half, `SHA256SUMS` checked, the files kept as the
     `release-dry-run` artifact (7 days).
   - **What never runs:** `prepare`'s checks that the commit is on `main`
     and that no release of the tag exists (skipped without a message),
     and the whole publish job (shown as skipped): the release key probe,
     the draft and its comparison, the pre-release, the anonymous
     downloads, the promotion and both read-backs.
   - **What it changes:** no job gets a write token, so no release, tag
     or file of the repository can change. It may save build caches for
     `main`, as a push does, and it records a deployment to
     `macos-signing`. Its artifacts, the unsigned installers among them,
     can be downloaded by any signed-in GitHub user while they are kept
     (the repository is public). They are not release material: nothing
     trusts that key.
   - From any branch but `main` the macOS package job is refused by
     `macos-signing`'s rules, and the run fails.
3. Tag and push: `git tag -s vX.Y.Z -m "sotyware X.Y.Z" -m "<release
   notes>"`, `git push origin vX.Y.Z`. The tag's message becomes the
   published release notes and the notes the app shows.
4. `release.yml` runs:
   - **prepare:** every version equals `X.Y.Z`; `updates.rs` names this
     repository; the tagged commit is on `main`; no release `vX.Y.Z`
     exists; `update-key.pub` holds a real key (not a fixture's, not a
     private key); the `release` environment exists and admits exactly
     the tag rule `v*` (a missing required reviewer and administrators
     allowed to bypass are warnings, and so is a `macos-signing`
     environment admitting anything but `main` and `v*`); the signing key
     and the Apple credentials are not repository secrets; unsigned
     Windows installers are allowed.
   - **ci, windows, linux, macos:** `ci.yml` on the commit; the Windows
     installers (`release-build.yml`, unpacked and checked); Linux
     (Ubuntu 22.04: AppImage, deb, rpm); macOS (`macos.yml`: tests on both
     architectures, packages signed and notarized in `macos-signing` when
     the Apple credentials are there).
   - **publish**, in the `release` environment: if it has a reviewer, the
     run waits for your approval on its page (**Review deployments**).
     Then: the tag still names the built commit; no release (or draft)
     exists; a probe signed with the secret verifies with `update-key.pub`;
     every updater package and `latest.json` are signed and verified with
     the public key alone (`scripts/release-assemble.sh`); a **draft** is
     created on the tag (visible only to people with write access); its
     names and sizes are compared and its files downloaded back with the
     token and checked against `SHA256SUMS`; it is published as a
     **pre-release** (public, `make_latest=false`: never "latest"); every
     file is downloaded again **anonymously** over HTTPS, compared byte for
     byte with what was signed and verified (signatures, signed versions,
     SHA-256); then one request sets `prerelease=false` and
     `make_latest=true`, GitHub's latest release must be this one
     (`/releases/latest`), and
     `https://github.com/casualsoty/sotyware/releases/latest/download/latest.json`
     and its `.sig` must serve exactly the signed manifest and signature,
     which must verify. One release runs at a time: a second run waits,
     and since GitHub keeps one waiting run per concurrency group, a third
     cancels the waiting one before it has started anything (start that
     one again).

   A tag that exists can also be released by hand:
   `gh workflow run release.yml --ref vX.Y.Z -f dry_run=false`.
5. What a failure leaves depends on how far the publish job got (if an
   earlier job fails, publish never starts and nothing changes). Once its
   upload has started, its last step reads the release's flags and names
   the case:
   - **Before the pre-release** (the checks and signing, the upload, the
     draft's comparison): nothing is public and the previous release
     stays the latest. A draft this run made is deleted, also after a
     cancellation or the job's timeout, so after fixing the cause you can
     **Re-run failed jobs**. A draft that existed before is left alone and
     stops the run. A force-cancel or a lost runner skips the deletion:
     delete the draft by hand, or the next attempt stops at it.
   - **After the pre-release, before the promotion** (the anonymous check,
     or the promotion request itself): the release stays a public
     pre-release, never the latest, so installations, which follow the
     latest release, see nothing; the run says it did not pass every check
     (also when only the promotion request failed). Delete it (with
     immutability on, GitHub may keep its tag and not let the version be
     used again) and release a new patch version.
   - **After the promotion request** (the `/releases/latest` read-back or
     the endpoint check failed, or the run was cancelled there): the
     release is a full release that GitHub was asked to make the latest,
     and installations may already be offered it; its files passed every
     check before the promotion. The run's warning says it "was made the
     latest release", which the run did not confirm when the read-back is
     what failed. Look at what
     `gh api repos/casualsoty/sotyware/releases/latest` names and what
     `releases/latest/download/latest.json` serves before doing anything
     else. Do not delete it in haste: installations that saw its manifest
     refuse every lower version from then on.

   The job finds its release by the tag name (`gh release … vX.Y.Z`). It
   refuses to start while a release or draft of the tag exists, but it
   counts a failed lookup as none, and a second release of the tag made
   during the run (only someone with write access can make one) can be the
   one a later step or the deletion acts on
   ([limitations](../status/limitations.md#updates-adr-033-adr-036-adr-039)).
   Never re-publish an older version or move a published tag
   (installations refuse a lower version than they were offered, and a
   moved tag breaks the URLs in the signed manifest).

Assets of a release (version 0.2.0 shown), all under
`https://github.com/casualsoty/sotyware/releases/download/v0.2.0/`:

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

GitHub adds "Source code (zip, tar.gz)" archives of the tagged commit to
every release.

Authenticode signing of the Windows installers ([above](#signing-not-set-up))
is not part of the workflow yet; when it is added it must happen in the
Windows build job, before the update signatures (signing changes the bytes),
and replace the `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` opt-in with a check that
the installers are signed.

### Secrets

The repository is public, so GitHub Free offers environments, environment
secrets, deployment branch and tag rules and required reviewers. Secrets
of an environment reach only a job that names it, and only once that
environment's rules let the job start: a job on another branch or tag, or
a pull request (from a fork none gets secrets at all), cannot read them.
No workflow uses `pull_request_target` or `workflow_run`, which would run
with secrets or a write token next to code from a fork.

| Name | Kind and where | Used by | Required |
| --- | --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | secret, environment **`release`** | `release.yml` publish job, its signing step only | yes |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret, environment **`release`** | the same step | yes (the key must have a password) |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | secrets, environment **`macos-signing`** | `macos.yml` package job, signing steps | for Mac packages ([macos.md](macos.md#signing-and-notarization)) |
| `APPLE_API_ISSUER`, `APPLE_API_KEY_ID`, `APPLE_API_KEY` | secrets, environment **`macos-signing`** | `macos.yml` package job, notarization step | for Mac packages |
| `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` | variable, repository | `release.yml` prepare and publish | `true` while no Authenticode certificate exists |
| `GITHUB_TOKEN` | automatic, per job | read-only in every job; `contents: write` only in the publish job, given to the steps that call `gh` | — |

No other token exists or is needed. Never store the signing key or an
Apple credential as a **repository** secret: every workflow run of the
repository could read it, and `release.yml` refuses to release while one
is there (its `prepare` job, which names no environment, only checks
whether those names are visible to it, which they are only as repository
or organisation secrets).

**Set up**, `macos-signing` before the first run of `release.yml` (a dry
run included) or of `macos.yml` with `package`, and the rest before
pushing the first release tag. A running job that names an environment
that does not exist makes GitHub create it, without any protection; a
skipped job does not.

1. **Settings → Environments → New environment** `release` (a dry run
   only warns while it is missing; creating it first lets the dry run
   check its rules for real):
   - **Required reviewers:** yourself (leave *Prevent self-review* off:
     you are the only maintainer, and it would block your own approval).
   - **Allow administrators to bypass configured protection rules:** off.
   - **Deployment branches and tags → Selected branches and tags → Add
     deployment branch or tag rule:** ref type **Tag**, pattern `v*`. No
     branch rule.
   - **Environment secrets:** `TAURI_SIGNING_PRIVATE_KEY` and
     `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
     ([how](#update-signing-key)).
2. **New environment** `macos-signing`, **before the first dry run**:
   **Deployment branches and tags → Selected branches and tags**, a
   **Branch** rule `main` and a **Tag** rule `v*` (dry runs from `main`
   package macOS too); **Allow administrators to bypass configured
   protection rules** off; a reviewer is optional (every Mac packaging
   run, dry runs included, would wait for it). No secret for a rehearsal:
   without the Apple values the build is unsigned and macOS stays out of
   the release. Secrets: the six `APPLE_*` values, once a Developer ID
   certificate exists ([macos.md](macos.md#signing-and-notarization)). If
   a packaging run created it already, it is unprotected and empty:
   restrict it before adding a secret.
3. **Secrets and variables → Actions → Variables → New repository
   variable** `SOTYWARE_ALLOW_UNSIGNED_WINDOWS` = `true`, only once you
   accept publishing unsigned Windows installers:

   ```bash
   gh variable set SOTYWARE_ALLOW_UNSIGNED_WINDOWS --repo casualsoty/sotyware --body true
   ```

4. Check the environments the way `release.yml` does:

   ```bash
   gh api repos/casualsoty/sotyware/environments/release/deployment-branch-policies --jq '.branch_policies[] | "\(.type) \(.name)"'
   ```

   It must print `tag v*` and nothing else.

   ```bash
   gh api repos/casualsoty/sotyware/environments/release --jq '{can_admins_bypass, reviewers: [.protection_rules[] | select(.type == "required_reviewers") | .reviewers[].reviewer.login], deployment_branch_policy}'
   ```

   It must show `"can_admins_bypass": false`, your login as the reviewer
   and `"custom_branch_policies": true`.

   ```bash
   gh api repos/casualsoty/sotyware/environments/macos-signing/deployment-branch-policies --jq '.branch_policies[] | "\(.type) \(.name)"'
   ```

   It must print `branch main` and `tag v*`, and nothing else.

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

   (the second prompts for the value). Only the `release` environment,
   never a repository secret ([Secrets](#secrets)).
4. **Check the pair** on the same computer, from the repository with the
   committed `update-key.pub` (the signer prompts for the password):

   ```bash
   echo "sotyware key check" > /tmp/sotyware-key-check.txt
   npx @tauri-apps/cli@2.12.4 signer sign -f ~/sotyware-release-key/sotyware-update.key --app-version 0.0.0 /tmp/sotyware-key-check.txt
   node scripts/update-manifest.mjs check-signature --pubkey src-tauri/update-key.pub --file /tmp/sotyware-key-check.txt --version 0.0.0
   ```

   A dry run of `release.yml` cannot check it (it never uses the release
   key); a real release checks it again with a probe before signing
   anything, and stops on a mismatch.
5. **Back up** the key file and the password separately and offline (for
   example the password in a password manager, the key file on two
   encrypted USB drives or printed), never in the repository, a chat, an
   issue or a CI log. Then delete the working copy from the computer.

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
  `updates.rs` (and refused unless it is the repository publishing), and
  `check-key` (fixture keys, private keys and malformed key files refused).
- `npm run test:release-assemble`: `scripts/release-assemble.sh`, the
  signing and checking step of `release.yml`'s publish and rehearse jobs,
  run with the real Tauri CLI and throwaway keys made in the test (macOS
  left out unless both builds are signed and notarized, missing packages
  and a wrong key refused, the key never printed).
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

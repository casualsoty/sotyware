# Application updates

Status: implemented 2026-11-15
([ADR-033](decisions.md#adr-033-updates-use-tauri-plugin-updater-behind-a-separately-signed-manifest-installed-only-on-the-users-click),
source and restart:
[ADR-036](decisions.md#adr-036-updates-come-from-the-latest-github-release-latestjson-signed-and-verified-as-in-adr-033),
where releases are published and who can sign:
[ADR-039](decisions.md#adr-039-releases-are-published-in-the-public-source-repository-with-github_token-signing-credentials-live-in-protected-environments)).
**The first release, `v0.1.0`, was published on 2026-11-15**
([status](../status/status.md#first-release-v010-2026-11-15)), signed with
the release key whose public half `src-tauri/update-key.pub` holds since
that day; builds made from then on check GitHub's latest release, which
is that one. **No update was ever installed:** that needs a later
version. What was tested, and
where: [status.md](../status/status.md#desktop-redesign-macos-and-github-releases-updates-2026-11-15).
How to publish: [release.md](../development/release.md#publishing-a-release).

Code: `src-tauri/src/updates.rs` (+ `updates/tests.rs`), the built-in
feature `update-check` (`src-tauri/src/features/updates.rs`), the page
`src/Updates.svelte` in Settings › About & updates and the notification in
`src/App.svelte`, both on one `UpdatesModel` (`src/updatesModel.svelte.ts`,
`src/updates.ts`), the release tools `scripts/update-manifest.mjs` and
`scripts/release-assemble.sh`, and the workflow
`.github/workflows/release.yml`.

## Where updates come from

GitHub Releases of the public repository `casualsoty/sotyware`
(`RELEASE_REPOSITORY`, ADR-039), published by its own `release.yml`. Each
release carries the packages, their `.sig` files, `latest.json`,
`latest.json.sig` and `SHA256SUMS`. sotyware asks, without any GitHub
account or token,

```
https://github.com/casualsoty/sotyware/releases/latest/download/latest.json
https://github.com/casualsoty/sotyware/releases/latest/download/latest.json.sig
```

GitHub answers with a redirect to the release it marks as latest (drafts
and pre-releases never are), then to its asset store. GitHub is only the
transport: what is installed is decided by the release key's signature.
The repository is written once, in the `release_repository!` macro in
`updates.rs`, from which `RELEASE_REPOSITORY` and `MANIFEST_URL` are built
and which `release.yml` reads (`update-manifest.mjs release-repository
--expect`), refusing to publish to any other repository.

## Flow

1. **Check.** With the `update-check` feature running (default on), one
   minute after startup and then every 24 hours (one timer; no polling);
   and whenever the user clicks **Check for updates** (also with the
   feature off).
2. sotyware fetches `latest.json` and `latest.json.sig`: HTTPS only (rustls
   on ring, the system's root certificates), at most five redirects, each
   to an HTTPS address (loopback HTTP only in test builds), at most
   256 KiB, 30 s. A signature that does not match is fetched once more,
   with the manifest, because a release published between the two requests
   pairs one release's manifest with another's signature. HTTP 404 means
   no published release has update information (or the repository is not
   public).
3. It verifies the manifest's minisign signature with the compiled-in key
   **before reading anything in it**, then checks: `format`
   `sotyware-update`, `formatVersion` 1; `channel` equals the
   installation's (`stable`); `version` is semver and not lower than the
   **version floor** (the highest version a verified manifest ever offered
   this installation, stored in `updates.sqlite3`), else it is refused as a
   possible replay; a version not newer than the running one means "up to
   date". For a newer version it picks this installation's platform entry,
   which must have an HTTPS URL, a signature and a SHA-256:

   | Installation | Entry |
   | --- | --- |
   | Windows, NSIS setup | `windows-x86_64-nsis` |
   | Windows, MSI | `windows-x86_64-msi` |
   | macOS, Apple Silicon | `darwin-aarch64-app` |
   | macOS, Intel | `darwin-x86_64-app` |
   | Linux AppImage / deb / rpm | `linux-x86_64-appimage` / `-deb` / `-rpm` |

   The kind comes from Tauri's bundle-type marker in the binary; a build
   without one (a development build) gets no package. The floor rises with
   every verified manifest.
4. **Notify.** The shell shows one notification (version, **Update**,
   **Details**, **Later**, close). **Later** saves the postponement for that
   version (a newer one or a manual check shows it again); closing hides it
   for this session. Settings › About & updates shows the current version,
   the last check, the state, the release notes and the release date.
5. **Update** is refused while an upload, a download, a download of several
   items, a file operation or a backup task runs (the page names it; Rust
   checks again before installing). sotyware hands **exactly the verified
   manifest** to `tauri-plugin-updater` (`restore_update`: no second fetch;
   the plugin then requires the package's signature to name the same
   version), requires the plugin's view of the release (version, URL,
   signature) to equal the manifest's entry, and downloads through the
   plugin (progress at most 4 times a second; **Cancel** until the download
   ends; a cancel that arrives after the commit point, taken under the
   install slot's lock just before the plugin installs, is refused, since
   the installation is one blocking call that cannot be interrupted). The
   plugin verifies the package's minisign signature and signed version;
   sotyware then checks its SHA-256 against the signed manifest.
6. **Install.**
   - Windows: the plugin writes the installer to a temporary file,
     sotyware releases what an ordinary quit releases (`release_resources`),
     the plugin starts the installer in passive mode and ends the process;
     the installer starts the new version.
   - macOS: the plugin unpacks `sotyware.app` from the `.app.tar.gz` and
     swaps it with the installed bundle (asking for an administrator's
     password through the system if the app's folder is not writable).
   - Linux: the AppImage file is replaced; a deb or rpm is installed with
     `pkexec`/`sudo` (`dpkg -i`, `rpm -U`).
   Quitting while the app is being replaced on macOS or Linux (after the
   commit point) waits for the replacement to finish, at most 60 s
   (`INSTALL_WAIT`), instead of ending the process in the middle; on
   Windows the installation itself ends the process.
   On macOS and Linux the page then says **Restart required** and offers
   **Restart now** (`restart_after_update`), refused while something runs.
   It uses `AppHandle::request_restart`: the normal exit
   (`RunEvent::Exit`, `release_resources`, the single-instance lock
   released), then the new process starts.
7. Any failure leaves the installed version untouched and is shown with its
   kind: `network` (server unreachable, no release), `signature` (not
   signed by the release key), `integrity` (package does not match its
   signed description), `replay` (older than offered before), `platform`
   (no package for this installation), `malformed`, `installer`, `busy`.

## Manifest

The updater plugin's static format, plus sotyware's fields, so the plugin
reads the same JSON:

```json
{
  "format": "sotyware-update",
  "formatVersion": 1,
  "channel": "stable",
  "version": "0.2.0",
  "notes": "The tag's message",
  "pub_date": "2026-12-01T12:00:00Z",
  "platforms": {
    "windows-x86_64-nsis": {
      "url": "https://github.com/casualsoty/sotyware/releases/download/v0.2.0/sotyware_0.2.0_x64-setup.exe",
      "signature": "<the package's .sig>",
      "sha256": "…"
    },
    "darwin-aarch64-app": {
      "url": "https://github.com/casualsoty/sotyware/releases/download/v0.2.0/sotyware_0.2.0_aarch64.app.tar.gz",
      "signature": "…",
      "sha256": "…"
    }
  }
}
```

Every URL points at the release's own tag folder (never `latest`), so a
manifest always names the files it was signed with. Asset names use only
letters, digits, `.`, `-` and `_`, which GitHub keeps unchanged.
Signatures are minisign, made by `tauri signer sign --app-version
<version>` (the signed trusted comment carries `version:<version>`; the
manifest is signed the same way). `scripts/update-manifest.mjs create`
writes the manifest only after verifying every package's signature and
signed version; `verify` checks a signed manifest, that it names exactly
the expected platforms (`--require`), that every URL is under the tag's
folder (`--base-url`) and the packages it names; `check-signature` checks
one signed file (the release job's key check); `check-key` checks
`update-key.pub`; `release-repository` prints the repository named in
`updates.rs` (with `--expect`, fails for any other).
`scripts/release-assemble.sh` runs `tauri signer sign` and these commands
in the release jobs. A fixture made by the real
Tauri CLI for all seven platforms (`scripts/fixtures/update-github/`, key
deleted) is verified by both the script's and the Rust tests.

## Keys and configuration

- **Release key:** `src-tauri/update-key.pub` holds the public half (the
  base64 line `tauri signer generate` writes), compiled in: since
  2026-11-15 the maintainer's key, id `3A6E8F2D58D68E0D`. A build without
  a key line reports updates as not set up. `npm run check:update-key`
  (in `test:headless`), `release.yml` and a Rust test refuse a private
  key, more than one key line and the
  signing fixtures' throwaway keys (`scripts/fixtures/**/test-key.pub`,
  test-only). The private key and its password are the GitHub secrets
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` of
  the `release` environment, which admits only `v*` tags; only the signing
  step of `release.yml`'s publish job receives them, never a build job, a
  pull request or a dry run (which signs with a throwaway key made for
  it). Publishing uses the workflow's own `GITHUB_TOKEN` (`contents:
  write` in that job only), which cannot sign. Generation,
  storage, rotation and recovery:
  [release.md](../development/release.md#update-signing-key).
- `tauri.conf.json` → `plugins.updater` holds only what the plugin needs to
  start (an empty `pubkey`, `requireSignedVersion`, passive Windows
  install); sotyware passes the real key to each updater it builds. No
  capability grants the frontend an `updater:` permission.
- **Debug and test-profile builds** read `SOTYWARE_UPDATE_ENDPOINT` and
  `SOTYWARE_UPDATE_PUBKEY` and accept loopback HTTP, so tests run against a
  local server with keys generated for the test. Release builds ignore
  both variables. Without both, a debug or test-profile build uses the
  compiled-in key and GitHub like a release build, so `npm run tauri dev`
  and the desktop tests' builds check GitHub 60 s after they start (the
  drivers set neither variable).

## Commands and state

`update_status`, `check_for_update`, `install_update` (events on a per-call
channel: `progress`, `verified`, `installing`, `installed`, `failed` with a
kind, `cancelled`), `cancel_update`, `postpone_update`,
`restart_after_update`. None returns or accepts a key, a URL or a path. The
status carries `errorKind`, `installed` (a version installed by this
process, awaiting a restart) and `usesInstaller` (Windows). `updates.sqlite3`
(one JSON row: floor, last check, postponed version; no secret) is one of
the stores a test profile reports.

## What an update keeps

The installers and the in-place replacements change program files only;
the data folder, webview data (including the theme choice) and credential
store entries are outside them. **Not yet verified by installing an
update** on any platform: that needs two signed releases (see
[roadmap.md](../status/roadmap.md)).

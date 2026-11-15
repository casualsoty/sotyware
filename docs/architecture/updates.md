# Application updates

Status: implemented 2026-11-15
([ADR-033](decisions.md#adr-033-updates-use-tauri-plugin-updater-behind-a-separately-signed-manifest-installed-only-on-the-users-click)),
**not usable in release builds yet**: no production signing key and no
update server exist (`updates::PRODUCTION` is `None`), so a release build
says "Updates are not set up in this build". What was tested, and where:
[status.md](../status/status.md#updates-2026-11-15). Release steps:
[release.md](../development/release.md#updates).

Code: `src-tauri/src/updates.rs` (+ `updates/tests.rs`), the built-in
feature `update-check` (`src-tauri/src/features/updates.rs`), the page
`src/Updates.svelte` on the Overview (`src/updates.ts`), the release tool
`scripts/update-manifest.mjs`.

## Flow

1. **Check.** With the `update-check` feature running (default on), one
   minute after startup and then every 24 hours (one timer; no polling);
   and whenever the user clicks **Check for updates** (also with the
   feature off). Switching the feature off means sotyware contacts the
   update server only on that click.
2. sotyware fetches `sotyware-update.json` and `sotyware-update.json.sig`
   from the configured URL: HTTPS only (rustls on ring, the system's root
   certificates), no redirect, at most 256 KiB, 30 s.
3. It verifies the manifest's minisign signature with the compiled-in
   public key **before reading anything in it**, then checks: `format`
   `sotyware-update`, `formatVersion` 1; `channel` equals the
   installation's (`stable`); `version` is semver and not lower than the
   **version floor** (the highest version this installation was ever
   offered by a verified manifest, stored in `updates.sqlite3`), else the
   check fails as a possible replay; a version not newer than the running
   one means "up to date". For a newer version it picks this installation's
   platform entry (`windows-x86_64-nsis` for an NSIS installation,
   `windows-x86_64-msi` for an MSI one, from Tauri's bundle-type marker;
   then `{os}-{arch}`, which sotyware's own manifests do not contain), which
   must have an HTTPS URL, a signature and a SHA-256. The floor rises with
   every verified manifest.
4. The Overview shows the version, its notes, **Update now** and **Later**
   (Later hides the reminder for that version; a newer one or a manual check
   shows it again).
5. **Update now** is refused while an upload, a download, a download of
   several items, a file operation or a backup task runs (the page names
   it; Rust checks again before installing). sotyware then hands **exactly
   the verified manifest** to `tauri-plugin-updater` (`restore_update`: no
   second fetch, and the plugin requires the package's signature to name
   the same version), requires the plugin's view of the release (version,
   URL, signature) to equal the manifest's entry, and downloads through the
   plugin (progress at most 4 per second, **Cancel** until the download
   ends). The plugin verifies the package's minisign signature and its
   signed version; sotyware then checks the package's SHA-256 against the
   signed manifest.
6. **Install.** Windows: the plugin writes the installer to a temporary
   file, sotyware releases what an ordinary quit releases
   (`release_resources`: features stopped, transfers cancelled, databases
   closed), the plugin starts the installer in passive mode and ends the
   process; the installer starts the new version. Elsewhere (not a release
   target) the plugin installs and returns; the page says the update takes
   effect at the next start (a restart from inside would meet the
   single-instance lock).
7. Any failure leaves the installed version untouched and is shown with its
   reason: server unreachable, not signed by the release key, replay
   (older than offered before), no package for this installation, package
   does not match its signed description, installer failed.

## Manifest

A superset of the plugin's release format, so the plugin reads the same
JSON; the plugin's own unsigned `latest.json` is not used.

```json
{
  "format": "sotyware-update",
  "formatVersion": 1,
  "channel": "stable",
  "version": "0.2.0",
  "notes": "…",
  "pub_date": "2026-12-01T12:00:00Z",
  "platforms": {
    "windows-x86_64-nsis": { "url": "https://…/sotyware_0.2.0_x64-setup.exe",
                             "signature": "<the installer's .sig>", "sha256": "…" },
    "windows-x86_64-msi":  { "url": "https://…/sotyware_0.2.0_x64_en-US.msi",
                             "signature": "…", "sha256": "…" }
  }
}
```

Signatures are minisign, made by `tauri signer sign --app-version <version>`
(the installer's signature carries `version:<version>` in its signed trusted
comment; the manifest is signed the same way). `scripts/update-manifest.mjs
create` writes the manifest only after verifying every installer's signature
and signed version; `verify` checks a signed manifest and the installers it
names. No generic `windows-x86_64` entry: an installation whose installer
type is unknown (a development build) is offered nothing.

## Keys and configuration

- **Production:** `updates::PRODUCTION` holds the manifest URL and the
  public key (base64 of the minisign public key text, as `tauri signer
  generate` writes it). It is `None`; the key pair and the server are
  external gates ([release.md](../development/release.md#updates)). The
  private key never enters the repository or the application; the release
  workflow reads it from a repository secret only when one is configured.
- `tauri.conf.json` → `plugins.updater` holds only what the plugin needs to
  start (an empty `pubkey`, `requireSignedVersion`, passive Windows
  install); sotyware passes the real key to each updater it builds. No
  capability grants the frontend an `updater:` permission, so the plugin's
  own commands are unreachable from the page.
- **Debug and test-profile builds** read `SOTYWARE_UPDATE_ENDPOINT` and
  `SOTYWARE_UPDATE_PUBKEY` and accept loopback HTTP, so tests run against a
  local server with keys generated for the test. Release builds ignore both
  variables and accept HTTPS only.

## Commands and state

`update_status`, `check_for_update`, `install_update` (events on a per-call
channel: `progress`, `verified`, `installing`, `installed`, `failed`,
`cancelled`), `cancel_update`, `postpone_update`. None returns or accepts a
key, a URL or a path. `updates.sqlite3` (one JSON row: floor, last check,
postponed version; no secret) is the seventh store a test profile reports.

## What an update keeps

The NSIS and MSI upgrades replace program files; the data folder, webview
data and credential store entries are outside them. **Not yet verified by
installing an update**: no test has installed one (that needs two signed
release builds and a Windows machine; see
[roadmap.md](../status/roadmap.md)).

# Application updates

Status: design accepted 2026-11-15
([ADR-033](decisions.md#adr-033-updates-use-tauri-plugin-updater-behind-a-separately-signed-manifest-installed-only-on-the-users-click)).
What is implemented and tested is recorded in
[status.md](../status/status.md) and the [journal](../status/journal.md).
Release steps: [release.md](../development/release.md).

## Flow

1. **Check** at startup (after the dashboard is up), every 24 hours while
   running (one timer), and when the user clicks "Check for updates".
2. sotyware downloads `sotyware-update.json` and `sotyware-update.json.sig`
   from the configured endpoint (HTTPS only in release builds) and verifies
   the minisign signature with the compiled-in public key.
3. It checks the manifest: `channel` equals the installation's channel;
   `version` is valid semver, strictly newer than the running version and not
   lower than the **version floor** (the highest version this installation
   was ever offered, stored locally); the platform entry for this
   installation (Windows: `windows-x86_64-nsis` or `windows-x86_64-msi`,
   depending on how sotyware was installed) has a URL, a minisign signature
   and a SHA-256.
4. The dashboard shows the new version, its release notes and "Update now" /
   "Later".
5. On "Update now" (refused while an upload, download, file operation or
   backup runs; the page says which), `tauri-plugin-updater` checks the same
   endpoint; sotyware requires its result (version, download URL, signature)
   to equal the signed manifest's entry, downloads with progress (cancellable;
   nothing is written outside the plugin's temporary file), the plugin
   verifies the artifact's minisign signature, sotyware verifies the SHA-256,
   then installs (Windows: passive NSIS/MSI installer) and restarts.
6. Any failure leaves the installed version untouched and is shown with its
   reason (no network, signature invalid, checksum mismatch, version not
   newer, installer failed).

## Manifest

```json
{
  "format": "sotyware-update", "version": 1,
  "channel": "stable",
  "release": "0.2.0",
  "pubDate": "2026-12-01T12:00:00Z",
  "notes": "…",
  "platforms": {
    "windows-x86_64-nsis": { "url": "https://…/sotyware_0.2.0_x64-setup.exe",
                             "signature": "<minisign .sig contents>", "sha256": "…" },
    "windows-x86_64-msi":  { "url": "…", "signature": "…", "sha256": "…" }
  }
}
```

The same release also publishes the plugin's `latest.json` (same versions,
URLs and signatures) because the plugin reads that format. Both are generated
and cross-checked by `scripts/update-manifest.mjs`.

## Keys and configuration

- The **public key** (minisign, as `tauri signer generate` produces) is
  compiled in from `tauri.conf.json` → `plugins.updater.pubkey`. The private
  key never enters the repository: CI reads it from the secret
  `TAURI_SIGNING_PRIVATE_KEY` (+ password) in a protected, manual release
  job.
- No production key exists yet. Until one is configured, the shipped
  configuration has no endpoint and the dashboard says "Updates are not
  configured for this build".
- Test-profile builds read the endpoint and a test public key from the
  test profile, so tests run against a local update server with keys
  generated for the test.

## What survives an update

The installer replaces program files only. Databases in the local data
folder, the webview data and OS credential store entries are not touched by
the NSIS/MSI upgrade; this is verified by the update tests (database files
and a credential entry compared before and after).

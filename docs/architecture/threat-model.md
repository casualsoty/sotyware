# Threat model

What sotyware protects, against whom, how, and what remains. It covers the
desktop app (Tauri process, dashboard webview, local databases, OS credential
store), the cloud storage providers it talks to, the backup engine
([backup.md](backup.md)), the sotyware account service and its end-to-end
encrypted vault ([accounts.md](accounts.md)), and application updates
([updates.md](updates.md)). Each section names the assets at stake, the
controls, and the residual risk. Controls that depend on external
configuration (a signing key, a hosted service) say so. Written 2026-11-15
as the design target of the production-readiness work; whether each control
is implemented and tested is recorded in [status.md](../status/status.md)
and the [journal](../status/journal.md), not here.

## Assets

| Asset | Where it lives | Why it matters |
| --- | --- | --- |
| User files (objects) | Provider buckets (primary), backup destination | The data itself |
| Cloud provider credentials (key id + secret) | OS credential store (Windows Credential Manager, Linux Secret Service), per account id; inside the encrypted vault when sync is on | Full access to the buckets they cover, including deletion |
| Account key (vault key) | OS credential store of each enrolled device (wrapped), wrapped copies on the server | Decrypts the vault, hence every synced credential |
| Recovery key | Only where the user keeps it (printed, password manager) | Unlocks the vault on a device that has no enrolled key |
| sotyware account password | User's memory/password manager; Argon2id hash on the server | Signs in; never decrypts the vault |
| Session tokens | Refresh token in the OS credential store; access token in memory | Act as the user towards the account service |
| Update signing key | Release maintainer's secret storage / CI secret (not in the repository) | Whoever holds it can ship code to every installation |
| Local databases | App local data folder (`%LOCALAPPDATA%\com.sotyware.desktop`, `~/.local/share/com.sotyware.desktop`) | Configuration, favorites, journal, backup history; no secrets |

## Trust boundaries

1. **Webview ↔ Rust.** The page is treated as less trusted than Rust: it
   reaches Rust only through the declared commands, each with its own
   permission in `capabilities/default.json`; it never receives a credential,
   a token, a key or a local path, and never sends a local path (dialogs are
   opened by Rust). CSP allows no remote content.
2. **sotyware ↔ storage provider.** HTTPS (rustls with ring and the OS root
   store); SigV4 request signing; the provider is trusted to store bytes but
   its answers are verified (sizes, SHA-256, ETags where meaningful).
3. **sotyware ↔ account service.** HTTPS only in release builds; the service
   is **not trusted with plaintext**: it stores ciphertext and wrapped keys
   only and cannot decrypt them.
4. **sotyware ↔ update server (GitHub Releases, ADR-036).** Not trusted:
   everything it serves is checked against signatures made with a key that
   neither GitHub nor the build jobs hold. A compromised GitHub account or
   token can publish a release, but not a validly signed one.
5. **Process ↔ other local processes.** Same-user processes are outside
   sotyware's control (see "malicious local process").

## Threats and controls

### Stolen computer

- *Exposure:* local databases (no secrets), OS credential store entries,
  cached vault ciphertext.
- *Controls:* credentials only in the OS credential store, which is
  protected by the user's login (Windows DPAPI-backed Credential Manager;
  Secret Service keyrings locked with the login password). No credential,
  token or key is written to SQLite, logs or files. The vault on disk is
  ciphertext.
- *Residual:* a thief who can log in as the user (or a running, unlocked
  session) can use the stored provider keys and the enrolled device key. The
  user must revoke the device in the account settings (which stops vault
  access for that device and triggers key rotation) and **rotate the
  provider keys at the provider**: revoking a device cannot make it forget
  secrets it already decrypted. Full-disk encryption (BitLocker) is the
  user's control and is recommended in the user documentation.

### Compromised sotyware account (password known to an attacker)

- *Controls:* the password signs in but **does not decrypt the vault**. A
  new device needs the recovery key or an approval from an already enrolled
  device, with a fingerprint the user compares on both screens. Rate limits
  and progressive lockout on sign-in; refresh-token rotation with reuse
  detection revokes a stolen session family; password change revokes other
  sessions.
- *Residual:* the attacker can delete the account (and so the server copy of
  the vault), list device names, and create pending enrollment requests
  (which an inattentive user might approve — the approval screen shows the
  requesting device's name, platform and fingerprint and says that approving
  gives access to every synced credential). Deleting the account does not
  delete local configuration on enrolled devices, from which the vault can be
  re-created.

### Compromised account service (or its operator)

- *Controls:* it stores only ciphertext, wrapped keys (sealed boxes,
  AEAD-wrapped account key) and non-secret metadata. It never receives the
  account key, the recovery key or any provider credential in plaintext. The
  list of trusted device keys lives **inside** the encrypted vault, so the
  server cannot add its own key to the set that receives a re-wrapped account
  key. The vault payload carries a monotonically increasing revision checked
  against the highest revision each device has seen, so a rollback to an old
  vault is detected by existing devices.
- *Residual:* the service can deny service, delete data, serve an old vault
  to a **fresh** device that has never seen a newer revision (rollback not
  detectable there), and learn metadata (email, device names and platforms,
  sync times, vault size). It could also present a malicious new-device
  enrollment request; the fingerprint comparison is the control.

### Compromised cloud provider account or key

- *Controls:* backups go to a separate destination, ideally another provider
  account with its own key that has no delete permission, and with Object
  Lock where the provider and bucket support it. The app reports the
  destination's actual lock configuration and never calls a backup immutable
  unless the bucket enforces default retention in compliance mode.
- *Residual:* a key with delete rights on the primary bucket can destroy the
  primary data; a key with delete rights on the backup bucket (and no Object
  Lock) can destroy backups. Governance-mode retention can be bypassed by a
  key with the bypass permission.

### Malicious update server, compromised GitHub account or update artifact

- *Controls (implemented 2026-11-15, [updates.md](updates.md)):* sotyware
  verifies a separately signed manifest with the compiled-in public key
  before reading it; installs only a version newer than the running one and
  refuses one lower than the highest version a verified manifest ever
  offered this installation (replay/rollback); hands exactly that manifest
  to `tauri-plugin-updater` (no second, unsigned fetch), whose package
  signature must name the same version; checks the package's SHA-256
  against the signed manifest; HTTPS only, and at most five redirects,
  each to HTTPS (GitHub's `releases/latest/download/` redirects twice). The
  signing key is used in one job of `release.yml` (bound to the
  `release` environment), never in a build job whose dependencies' build
  scripts could read it; the job checks the secret against the committed
  public key first, and the committed key can never be a signing fixture's
  throwaway key or a private key (`check-key`, ADR-038). Releases live in a
  separate public repository holding only distribution files
  (`casualsoty/sotyware-releases`, ADR-038); the token that writes there
  (`SOTYWARE_RELEASES_TOKEN`, fine-grained, Contents on that repository
  only) is given only to the steps that run `gh release` and cannot sign,
  so a leaked token can publish files installations refuse, not updates.
  The plugin's own commands are unreachable from the page (no `updater:`
  permission). Nothing installs without the user's click, and not while a
  transfer, file operation or backup runs. Tested with test keys and a
  fixture signed by the real Tauri CLI (`updates/tests.rs`,
  `scripts/update-manifest.test.mjs`).
- *Residual:* whoever holds the update signing key can ship arbitrary code.
  The key must live outside the repository (CI secret or offline). A
  production key does **not** exist yet (`src-tauri/update-key.pub` is
  empty); release builds have updates switched off. A fresh installation has
  no version floor yet, so the first manifest it sees can be any validly
  signed one, newer than the installed version. A withheld update (the
  server answering an old manifest, or nothing) is not detected beyond the
  floor: there is no expiry on manifests. Whoever controls the release
  repository (the account, or the release token) can delete releases or
  mark an older one latest, which installations see as "up to date" (the
  floor refuses anything lower than what they were offered). On GitHub
  Free a private repository has no environments, so the update key and
  release token are repository secrets readable by any workflow run of the
  source repository ([release.md](../development/release.md#secrets)).

### Malicious local process (same user)

- *Residual, by design of desktop operating systems:* a process running as
  the same user can read the user's files, query the OS credential store
  (Linux Secret Service grants any same-session client once unlocked;
  Windows Credential Manager entries are readable by the same user), attach a
  debugger, or inject input. sotyware cannot defend against this; it keeps no
  secrets in files and keeps the IPC surface narrow (no local HTTP server,
  no named pipes beyond the single-instance plugin, which only shows the
  dashboard).

### Another user on the same computer

- *Controls (2026-11-15):* the data folder is private to the user: on Linux
  `storage::prepare_data_dir` creates it with mode `0700` and narrows an
  existing one from an older version (only that folder; a link in its place
  is not followed); on Windows `%LOCALAPPDATA%` is per-user by its ACL.
  Credentials are in the user's OS credential store, never in files.
- *Residual:* an administrator or root can read everything; the databases
  are not encrypted at rest (full-disk encryption is the user's or the
  organisation's choice).

### Accidental user deletion

- *Controls:* every delete is a reviewed plan; on versioned buckets a delete
  adds a delete marker and the file can be restored from Deleted items;
  sotyware never deletes versions permanently; backups keep historical
  snapshots under a retention policy and are restorable byte for byte; the
  recovery manifest makes restoration possible without sotyware.
- *Residual:* deletes on unversioned buckets are permanent at the primary
  (the review says so); recovery then depends on a backup.

### Ransomware-like mass modification / incorrect synchronization / app bug

- *Controls:* backups are content-addressed and append-only (a changed file
  is a new blob; the old one stays referenced by older snapshots). A snapshot
  whose change ratio exceeds the configured threshold is marked suspicious:
  retention pruning stops and the backup health warns. Sync never deletes
  remote objects (it carries configuration only) and never lets a stale device
  overwrite a newer vault (optimistic concurrency with revisions).
- *Residual:* a slow, below-threshold corruption is captured into new
  snapshots; older snapshots keep the good versions until retention removes
  them.

### Provider lifecycle policies / backup destination deletion

- *Controls:* backup verification lists and checks every blob of the latest
  snapshot; missing blobs mark the backup unhealthy. Object Lock (where
  configured) prevents deletion before the retention date.
- *Residual:* a lifecycle rule configured at the provider can remove backup
  data between verifications.

### Application database corruption

- *Controls:* SQLite databases are integrity-checked at open; a damaged or
  newer database is never repaired or replaced automatically, the app runs
  with defaults and shows the error. Losing local databases loses no user
  data: objects stay at the provider, credentials in the OS store, the vault
  on the server, backups describe themselves.

### Provider outage / sotyware backend outage

- *Controls:* storage works without the account service (accounts are
  optional, local-only installations keep working). A backup on another
  provider keeps the data reachable during a primary-provider outage, with
  standard S3 tools if needed. The service's own database is backed up and
  its restore is tested ([server operations](../development/server.md)).
- *Residual:* during a service outage, new devices cannot enroll and sync
  pauses; existing devices keep their local configuration.

## Privacy

- No telemetry, analytics or crash reporting exists. stderr diagnostics may
  contain object keys (file names) and backup job ids; they are never
  persisted or sent by sotyware. Reviewed 2026-11-15: no log line in the
  app or the service contains a credential, token, password, account key,
  recovery key or update key (errors that could carry one are typed and
  tested, e.g. `errors_never_contain_a_secret`). The service's `outbox`
  command prints development mail, verification codes included, by design;
  production must use a mail provider instead.
- An update check sends a plain GET with `User-Agent: sotyware/<version>`;
  GitHub (and its asset store) learns the IP address, the time and the
  version.
- The account service learns: email, password hash, device names/platforms,
  IP addresses in its rate limiter (memory only), sync timestamps and vault
  sizes. Its audit log records event types, account and device ids and times,
  never content or secrets.

## Dependency review (2026-11-15)

`cargo audit` (RustSec database, 1,509 advisories) and `npm audit` on the
lock files of this commit:

- **Account service** (`server/Cargo.lock`): no advisory.
- **Frontend runtime** (`npm audit --omit=dev`): no advisory. Dev only:
  `deepmerge` (low, GHSA-gcqx-w48m-w2h4, build time, no fix).
- **Desktop app** (`src-tauri/Cargo.lock`): two vulnerabilities in
  `quick-xml` 0.37.5 (RUSTSEC-2026-0194, -0195: denial of service on
  hostile XML), reached only through `s3s`, the in-process S3 server of the
  tests (a dev-dependency, never in a build). Unsound: `lru` 0.12.5
  (RUSTSEC-2026-0002, -0253) through `aws-sdk-s3`, which uses only
  `get_or_insert_mut` in its S3 Express session cache (not the affected
  `IterMut` or `pop`, and S3 Express is AWS-only); a fixed `lru` needs a
  newer `aws-sdk-s3` than Rust 1.90 allows. `glib` 0.18.5
  (RUSTSEC-2024-0429, `VariantStrIter`, Linux GTK 3 through Tauri; no GTK 3
  release with the fix). Unmaintained: `zbus_names`, `zvariant`,
  `zvariant_derive` (Linux D-Bus), `pin-utils`, `proc-macro-error` (build
  time). None is reachable with attacker-controlled input in sotyware's use;
  re-run before every release.

## Residual risks summary

1. Same-user malware can read stored credentials.
2. Revoked devices keep what they already decrypted: rotate provider keys.
3. Losing both the recovery key and every enrolled device makes the vault
   unrecoverable; there is no backdoor. Storage data itself is unaffected
   (credentials can be re-entered from the provider console).
4. A fresh device can be served an old vault by a malicious service.
5. The update signing key holder can ship code; production key not yet
   created, so updates are off in release builds. Manifests do not expire,
   so a server can withhold updates (never roll back below the floor).
6. Check-then-write races remain where providers lack conditional writes.
7. Backups are stored as plaintext objects (readable by anyone with read
   access to the backup bucket), chosen so they stay restorable with
   standard tools; use a private bucket with a dedicated key.

# Cloud storage

How sotyware talks to S3-compatible cloud storage. Status: the backend is a
tested library module; the dashboard's **Cloud Storage** section uses its
account part (list, add, edit, check, remove; [Dashboard](#dashboard),
[ADR-018](decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)).
Uploads, downloads, listings of objects and the transfer queue are not
reachable from the app: no command exposes them. Decision and rationale of
the backend: [ADR-016](decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter).
The provider research, prices and the recommendation are in
[cloud-providers.md](cloud-providers.md).

Code: `src-tauri/src/cloud/` (`provider.rs`, `account.rs`, `credentials.rs`,
`keyring.rs`, `s3.rs`, `dashboard.rs`) and `SqliteAccountStore` in
`src-tauri/src/storage.rs`; the page is `src/CloudStorage.svelte` with
`src/CloudAccountForm.svelte`, `src/cloud.ts` and `src/cloudMessages.ts`.
Credentials: [ADR-017](decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store).

## Pieces

```
AccountConfig (no secret) ──resolve()──▶ ResolvedAccount ──┐
        │                                  ▲               ├─▶ S3Store (one per account)
        │ AccountStore (SQLite)            │ ProviderProfile│     aws-sdk-s3 client + connection pool
        ▼                                  │ (data)         │     transfer queue (semaphore)
   Accounts ── connect(id) ─────────────────────────────────┘     SHA-256 end to end
        │
        │ CredentialStore (OS: Credential Manager / Secret Service), by account id
```

| Piece | What it is |
| --- | --- |
| `ProviderProfile` (`provider.rs`) | **Data, not code**: endpoint rule, addressing style, SDK checksum policy, documented transfer limits, capabilities (versioning, Object Lock), known regions, documentation URL. Profiles: `backblaze-b2`, `hetzner`, `s3-compatible` (custom endpoint). |
| `AccountConfig` (`account.rs`) | One set of credentials at one provider: `id` (kebab-case, stable), `label`, `provider`, `region`, optional `endpoint` and `path_style` overrides, the `buckets` it may use, `max_concurrent_transfers`. **No secret.** `resolve()` validates it and computes the endpoint. |
| `Accounts` (`account.rs`) | The set of accounts and their one `CredentialStore`: `add(config, credentials)`, `update(config)`, `set_credentials(id, credentials)`, `has_credentials(id)`, `remove(id)` (also deletes the credentials), list, and `connect(id)`, which loads the credentials and creates and caches one `S3Store` per account. Order and failure handling: [Credentials](#credentials). |
| `AccountStore` trait / `SqliteAccountStore` | `load`, `save`, `remove`. SQLite in `cloud-accounts.sqlite3` (tables `storage_accounts`, `storage_account_buckets`; `ACCOUNT_MIGRATIONS`, `PRAGMA user_version`), using the same open/check/migrate code as the preferences database. Schema has no column for a secret. |
| `CredentialStore` trait (`credentials.rs`) | `store`, `load`, `delete`, `exists` for an account's key pair, **by account id**. `Secret` never prints its value in `Debug`/`Display`. `EnvCredentials` (`SOTYWARE_CLOUD_<ID>_ACCESS_KEY_ID`, `…_SECRET_ACCESS_KEY`) exists only in test builds, for the opt-in provider tests. |
| `OsCredentialStore` (`keyring.rs`) | The one real `CredentialStore`: Windows Credential Manager or the Linux Secret Service through `keyring-core`. No fallback. |
| `S3Store` (`s3.rs`) | The one adapter: `check_bucket` (the connection check), `list_buckets`, `list_objects` (all pages), `head_object`, `upload_file`, `download_file`, `delete_object`. |
| `CloudAccounts` (`dashboard.rs`) | What the dashboard's commands work on: `Accounts` (or why its database could not be opened) plus the last connection check per account, in memory. Safe shapes (`AccountInfo`, `ProviderInfo`), structured `CommandError`, account ids from labels. [Dashboard](#dashboard). |

### Why one adapter and no `ObjectStore` trait

Backblaze B2 and Hetzner both speak the S3 API with SigV4. Everything that
differs between them is configuration: endpoint format, addressing style,
limits, versioning default. One `S3Store` with a per-provider profile covers
both, and any other S3-compatible service through `s3-compatible`. A trait
would have exactly one implementation (AGENTS.md: no speculative
abstractions). It becomes worth adding when a provider without an S3 API is
needed (for example Google Drive's own API). Callers already go through
`Accounts`/`S3Store`, never through a provider type, so the frontend and the
core logic would not change.

## Requests

- **SDK:** `aws-sdk-s3` (official AWS SDK for Rust) without default features,
  with an HTTPS client on rustls + **ring** (`aws-smithy-http-client`,
  `rustls-ring`), using the operating system's root certificates. The default
  features would pull in aws-lc-rs, which needs CMake/NASM on Windows. No
  `aws-config`: credentials and region come from the account, never from
  `~/.aws` or `AWS_*` variables.
- **Checksums:** the SDK's own CRC32 request checksums (default since 2025)
  are set to `WhenRequired` for every profile, because neither provider
  documents support for `x-amz-checksum-*`/`aws-chunked`. Integrity comes from
  sotyware's own SHA-256 instead (below).
- **Timeouts and retries:** connect 10 s, read 60 s, 3 attempts with the SDK's
  standard exponential backoff (transient errors only). `ClientOptions`.
- **Plain HTTP** is refused except for `localhost`/`127.0.0.1`/`::1` (the test
  server); an endpoint is `scheme://host[:port]` only.
- **Bucket allow-list:** if an account lists buckets, any operation on another
  bucket is refused before a request is sent.

## Uploads and integrity

1. The file is hashed (SHA-256) and its size and modification time noted.
2. `plan_upload` picks a single PUT (up to `multipart_threshold`, 64 MiB, and
   never above the provider's single-PUT limit) or a multipart upload: parts of
   16 MiB, raised to the provider minimum and to whatever keeps the count within
   `max_parts`, whole MiB; never a one-part multipart upload (B2 requires two);
   refused before any request if larger than the provider's object limit.
3. The SHA-256 is stored as user metadata `x-amz-meta-sotyware-sha256`. Parts
   are streamed from the file (`ByteStream::read_from().offset().length()`), so
   memory use does not grow with the file size. Parts of one file go one after
   another.
4. A failed multipart upload is **aborted**, so no orphaned parts stay billed.
5. If the file's size or modification time changed meanwhile, the version just
   created is deleted and the upload fails with `FileChanged`.

Downloads stream into `<dest>.sotyware-partial` while hashing, then compare
size and SHA-256; only a match is renamed to `<dest>`. A mismatch is
`Integrity`; the partial file is removed and `<dest>` is never created. An
existing `<dest>` is never overwritten. Objects without the metadata (not
uploaded by sotyware) are checked by size only and reported as
`Verification::SizeOnly`. ETags are not used: they are not an MD5 for multipart
uploads and differ between providers.

## Errors

`StorageError` sorts failures by what can be done about them:
`InvalidCredentials` (unknown key id, wrong secret), `AccessDenied`,
`NotFound`, `Network`, `Timeout`, `Integrity`, `FileChanged`, `TooLarge`,
`Refused` (bucket not configured, bad key, existing destination), `Io`
(local file), `Provider { status, code, message }`. `is_transient()` is true for
`Network`, `Timeout`, 5xx and 429: the basis for a later retrying queue. The
mapping from S3 error codes is a pure function (`classify_response`). Which
codes Backblaze and Hetzner really return for a bad key is checked only by the
opt-in integration tests (the test server returns `NotSignedUp` and
`SignatureDoesNotMatch`). Backblaze, 2026-11-11: a wrong secret →
`InvalidCredentials` ("Signature validation failed"), `ListBuckets` with a
bucket-restricted key → `AccessDenied` ("not entitled"), reading a hidden
object → `NotFound` ("Key not found"); an unknown key id was not tried. No error message contains a secret.

## Accounts, credentials and queues

- **Several accounts, also at the same provider:** each account has its own
  id, configuration, credentials (looked up by its id), HTTP client and
  connection pool. Tests run two accounts at once against two servers with
  different keys and the same bucket and key names.
- **Several buckets per account:** `buckets` lists the ones sotyware may use.
  Empty means any bucket the key reaches. (B2 keys can be limited to one
  bucket; Hetzner keys reach every bucket of the project unless a bucket policy
  restricts them.)
- **Independent transfer queues:** every `S3Store` has its own semaphore of
  `max_concurrent_transfers` (default 4, 1 to 32) that each upload and download
  waits on. A full queue on one account never delays another (tested).
- **Credentials:** never in `AccountConfig`, the database, logs or error
  messages; kept in the OS credential store by account id. See below.
- **Persistence:** `cloud-accounts.sqlite3`, a separate file next to
  `preferences.sqlite3` in the app's local data folder. Kept separate when the
  dashboard was wired in (ADR-018): the schemas evolve independently, and the
  preferences schema did not have to change. A stored account that no longer validates (an unknown provider, an
  out-of-range value) is kept and refused only when used.

## Credentials

Decision: [ADR-017](decisions.md#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store).

### Backend per OS

| OS | Backend | Crate | Where the entry is |
| --- | --- | --- | --- |
| Windows 10/11 | Windows Credential Manager (generic credential, `CRED_PERSIST_LOCAL_MACHINE`: survives logoff and reboot, does not roam) | `windows-native-keyring-store` | target `com.sotyware.desktop/cloud-account/<id>`, user `cloud-account/<id>`; visible in Control Panel → Credential Manager → Windows Credentials, and with `cmdkey /list:com.sotyware.desktop/*` |
| Linux | Secret Service API over the D-Bus session bus, DH-encrypted session. Tested with GNOME Keyring 46.1 only; KWallet and KeePassXC also offer the API (not tested) | `zbus-secret-service-keyring-store` (pure Rust, on the `zbus` already used by the tray) | attributes `service=com.sotyware.desktop`, `username=cloud-account/<id>`, label `sotyware cloud account <id>`, in the default collection (usually "login") |
| other | none: every call fails with `Unavailable` | — | — |

The names are derived from the **account id only**, never from the provider,
so two accounts at one provider are two entries. Account ids are kebab-case
(checked again before any backend call), so they cannot inject separators.

### Stored value

One entry per account, holding both parts:
`sotyware-s3-credentials-v1\n<access key id>\n<secret access key>`. A rotation
replaces the whole entry in one write. A value in any other format (for
example written by hand) is refused as `Invalid` without being repeated.
Parts must be 1 to 512 bytes without whitespace or control characters; this is
checked before the backend is called, and keeps the value inside Windows'
2,560-byte credential limit.

### Operations and their order

`Accounts` runs every operation under one lock:

| Operation | Order | On failure |
| --- | --- | --- |
| `add(config, credentials)` | validate both → credentials stored → configuration saved | credential store fails: nothing saved. Configuration save fails: the credentials are deleted again (`Store` error); if that delete fails too, `AddRollbackFailed` names what is left (adding again overwrites it). A duplicate id never touches the existing credentials. |
| `set_credentials(id, credentials)` | one replace; then the cached connection is dropped | the old credentials and connection stay; nothing is deleted first |
| `update(config)` | configuration only | credentials untouched |
| `remove(id)` | credentials deleted (missing ones count as deleted) → configuration removed | credential delete fails: nothing changes, the error is returned, the account stays listed (never a silent success). Configuration remove fails: `RemovedOnlyCredentials`; removing again finishes. |
| `connect(id)` | credentials loaded → `S3Store` created | `NotFound` says "enter the account's key again" |

**Removing an account never sends a request to the provider:** its buckets and
objects stay (tested against the in-process S3 server). `Accounts::load` does
not read credentials, so startup never opens an unlock prompt.

### Errors

`CredentialError`: `NotFound` (nothing stored; actionable hint), `Unavailable`
(no Secret Service, no session bus, no Windows logon session; the message says
sotyware never falls back to storing elsewhere), `AccessDenied` (Linux: the
collection is locked and the unlock prompt was dismissed or could not be
shown, or D-Bus `AccessDenied`), `Backend` (anything else, with the Windows
error code or D-Bus error), `Invalid` (bad input or an entry not in
sotyware's format). Sorting is by error type (`secret-service`/`zbus` types,
D-Bus error names), never by message text. No variant holds a secret: the
`keyring-core` errors that carry stored bytes are never formatted, and tests
check `Display` and `Debug` of every path for the secret.

On Windows the store crate keeps its error type private, so `ERROR_ACCESS_DENIED`
and other codes are all `Backend` ("Windows error code N"); only "no logon
session" is recognised (as `Unavailable`).

### Threads and blocking

Calls block: a D-Bus round trip, and on Linux possibly an unlock prompt that
waits for the user. The dashboard's commands run every call that can reach
the credential store or SQLite in `tokio::task::spawn_blocking` (Tauri runs
`async` commands on its tokio workers, which must not block either), so
neither the event loop nor an async worker waits for a prompt
([Dashboard](#dashboard)). `OsCredentialStore`
serialises its own calls, because `keyring-core` warns that the platform
stores may not handle one credential from several threads. The platform store
is opened on first use; while it cannot be opened (no Secret Service yet), each
call tries again, so a keyring daemon that starts later is used without a
restart. There is no polling.

### Migration from environment variables

Nothing to migrate in stored data: the accounts schema never had a secret
column and is unchanged, so existing `cloud-accounts.sqlite3` files load as
before. What changed is the code path: the app has no way to read credentials
from the environment any more (`EnvCredentials` is test-only), and nothing
copies environment variables into the OS store. An account saved before this
change has no OS entry, so `connect` reports `NotFound` until its key is
entered again (`set_credentials`). No released build ever used cloud accounts,
so no user is affected.

## Backups with restic (planned, not built)

restic is the intended backup engine; sotyware orchestrates it and has no
backup format of its own. Findings (restic 0.19.1, docs and the experiment in
[status.md](../status/status.md#cloud-storage-proof-of-concept-2026-11-11)):

- **Both providers:** through restic's S3 backend,
  `-r s3:https://<endpoint>/<bucket>/<prefix>`. restic's documentation
  recommends B2's S3 API over its native `b2:` backend. restic wants path-style
  URLs; for Hetzner, path style is undocumented, so `-o s3.bucket-lookup=dns`
  may be needed (to check with a real account).
- **Credentials:** environment variables of the restic child process only:
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_DEFAULT_REGION`, and the
  repository password through `RESTIC_PASSWORD_COMMAND` or
  `RESTIC_PASSWORD_FILE` (never on the command line, where other processes can
  read it). sotyware would take them from the same `CredentialStore`.
- **Encryption and deduplication:** AES-256-CTR + Poly1305-AES, scrypt key
  derivation, content-defined chunking. Seen in the experiment: no plaintext in
  the stored objects; a second backup with a 20 MB copy of an existing file
  added 24 KB.
- **Orchestration:** run the official restic binary as a child process with
  `--json`, read its JSON lines (`status`, `summary`, `error`), and act on its
  documented exit codes (0 ok, 1 failed, 3 incomplete backup, 10 no repository,
  11 locked, 12 wrong password, 130 cancelled; any other code is a failure).
  `snapshots --json` lists snapshots; `restore <id> --target <dir>` restores;
  `check` (optionally `--read-data-subset`) verifies. Open questions: shipping
  the binary per platform (and checking its `SHA256SUMS`), cancellation,
  locking, and Backblaze's "keep all versions" default, which keeps data that
  `prune` removed billed until a lifecycle rule hides it.

## Cost estimation (proposal, not built)

- **Pricing is data, not code.** A versioned pricing file (for example
  `pricing/<provider>.json`, shipped with the app and replaceable) per provider:
  currency, VAT treatment, `valid_from`, `checked_on`, `source_url`, and
  components (`base_fee`, `storage_per_tb_month`, `included_storage_tb`,
  `free_egress` as a fixed amount or as a multiple of stored data,
  `egress_per_tb`, `request_classes`, `minimum_object_size`,
  `minimum_storage_days`). Profiles reference a pricing id; they never contain
  prices.
- **Inputs:** stored bytes per account (sum of `list_objects`, or the
  provider's usage API where there is one); egress counted locally by sotyware
  for its own downloads.
- **Outputs:** monthly and yearly estimate for current use and for 200 GB,
  1 TB, 2 TB and 5 TB, with the pricing file's date and source shown. Every
  number is labelled **estimate**. Billing data from the provider (B2 has a
  usage report; Hetzner shows usage in its console) would be a separate,
  clearly labelled "billed" value, never mixed with the estimate.
- Local egress counting misses downloads by other tools, and listing is not
  free in time for millions of objects. Both are reasons to prefer provider
  usage data when it can be fetched.

## Dashboard

The **Cloud Storage** section of the dashboard (decision:
[ADR-018](decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)).
It manages accounts only; it is not a file browser.

```
CloudStorage.svelte ── cloud.ts ──invoke──▶ cloud::dashboard (commands)
  CloudAccountForm.svelte                     │ spawn_blocking: SQLite, credential store
                                              ▼
                                           CloudAccounts ── Accounts ── AccountStore (SQLite)
                                              │                  └──── OsCredentialStore
                                              └─ check: S3Store::check_bucket (async runtime)
```

| Command (permission `allow-<name with ->`) | Arguments | Returns |
| --- | --- | --- |
| `list_cloud_providers` | — | every `ProviderInfo`: id, name, `endpointRequired`, `defaultRegion`, `knownRegions` |
| `list_cloud_accounts` | — | `{ accounts: AccountInfo[], storageError }` |
| `add_cloud_account` | `account`, `credentials` | the new `AccountInfo` |
| `update_cloud_account` | `id`, `account`, `credentials` or `null` | the changed `AccountInfo` |
| `remove_cloud_account` | `id` | nothing |
| `check_cloud_account` | `id` | the `AccountInfo` with `lastCheck` |

`account` is `{ label, provider, region, endpoint, bucket }` (`endpoint` `null`
for the provider's regional one); `credentials` is
`{ accessKeyId, secretAccessKey }`. `AccountInfo` is `{ id, label, provider,
providerName, region, endpoint, endpointUrl, bucket, configError, lastCheck }`;
it never contains a key id or a secret, and no command returns one. Failures
reject with `{ kind, message }` (kinds in ADR-018); the page picks its text
by `kind` and shows `message` as the detail.

**Flows:**

- **Add:** the form asks for provider, name, bucket, region (pre-filled with
  the provider's first known region, suggestions from the profile; `us-east-1`
  for a generic service), endpoint (required only for `s3-compatible`,
  otherwise an optional override) and the key pair in two password inputs.
  Rust trims the inputs, generates the id from the name (`Photos` →
  `photos`, the next one `photos-2`), validates, stores the credentials,
  then the configuration (rolling the credentials back if that fails).
- **Edit:** the provider is shown read-only and cannot change. Without
  "Replace access key" only the configuration is saved and the credential
  store is not touched. With it, everything is validated first, the new key
  replaces the old one in one write (the old one stays if that fails), then
  the configuration is saved; if only that fails, the error is `partial`
  ("the new access key was saved, but the other changes were not") and the
  page reloads.
- **Remove:** asks for confirmation and says that the bucket and its objects
  at the provider are not deleted or changed (no request is sent). Uses
  `Accounts::remove`: credentials first; if they cannot be deleted the
  account stays listed and the error is shown; never a success with a key
  left behind.
- **Credentials in the page:** only in the form's state while it is open;
  cleared after a successful save, on cancel, when "Replace access key" is
  unticked and when the form is destroyed. Never in local or session storage,
  never echoed back.

**Connection check** (`S3Store::check_bucket`): `HeadBucket` on the account's
bucket proves the key and the bucket together and needs no listing
permission. A HEAD response has no body, so a 403 is ambiguous (wrong secret
or missing permission); only then one `ListObjectsV2` with `max-keys=1` on
the same bucket is sent, and its error code decides: `InvalidAccessKeyId` or
`SignatureDoesNotMatch` → invalid credentials, `AccessDenied` → access denied.
`ListBuckets` is never sent (a valid Backblaze key restricted to one bucket
is refused it), and nothing is written. Results:

| Status | When |
| --- | --- |
| `connected` | `HeadBucket` (or the fallback listing) succeeded |
| `invalid-credentials` | unknown key id or wrong secret |
| `access-denied` | key accepted, bucket refused |
| `bucket-unavailable` | no such bucket at this endpoint/region |
| `network` | unreachable, connection broken, timed out |
| `credential-store-unavailable` / `credential-store-locked` / `credentials-missing` | the key could not be loaded; no request sent |
| `invalid-configuration` | the stored settings do not validate, or no bucket |
| `error` | anything else (message shown) |

The credentials are loaded on the blocking pool, the request runs on the async
runtime with the client's timeouts (10 s connect, 60 s read, 3 attempts).
Results are kept in memory per account until the account changes; a check
that was running during a change does not record its result.

**Against which providers this is verified:** the in-process S3 server (`s3s`)
with an access hook that refuses chosen operations, so a key that is refused
`ListBuckets`, or `HeadBucket` too, or everything, is simulated; and the
real binary through its UI against a real GNOME Keyring under WSL2. The
opt-in Backblaze and Hetzner integration tests call `check_bucket` too, but
were not run after it was added: whether Backblaze answers `HeadBucket` for a
bucket-restricted key is not yet confirmed against the real provider.

## Not built (deliberately)

No object browser, upload or download UI, transfer queue UI, event or
feature; no synchronisation, folder watching,
scheduled backups, retention, restic integration, client-side encryption,
sharing or public links, virtual filesystem, retrying queue, parallel parts
within one file, resumable uploads, credential import from environment
variables or files, automatic provider or bucket discovery, cost estimator,
or a credential fallback outside the OS store. See
[roadmap.md](../status/roadmap.md).

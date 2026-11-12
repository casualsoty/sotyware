# Cloud storage

How sotyware talks to S3-compatible cloud storage. Status: the backend is a
tested library module; the dashboard's **Cloud Storage** section uses its
account part (list, add, edit, check, remove; [Dashboard](#dashboard),
[ADR-018](decisions.md#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands))
and lists each account's bucket one folder and one page at a time, read-only
([Browsing a bucket](#browsing-a-bucket),
[ADR-019](decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust)).
From the browser one file at a time can be downloaded to a place the user
picks in the native save dialog ([Downloading a file](#downloading-a-file),
[ADR-020](decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel)).
Uploads, any change to objects and the transfer queue are not reachable
from the app: no command exposes them. The backend's upload lifecycle
(commit point, cancellation, re-hashing, verification, stall timeout) is
built and tested against the in-process server only
([Uploads and integrity](#uploads-and-integrity),
[ADR-021](decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration)). Decision and rationale of
the backend: [ADR-016](decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter).
The provider research, prices and the recommendation are in
[cloud-providers.md](cloud-providers.md).

Code: `src-tauri/src/cloud/` (`provider.rs`, `account.rs`, `credentials.rs`,
`keyring.rs`, `s3.rs` with `s3/upload.rs`, `dashboard.rs`, `browser.rs`, `download.rs`) and `SqliteAccountStore` in
`src-tauri/src/storage.rs`; the page is `src/CloudStorage.svelte` with
`src/CloudAccountForm.svelte`, `src/CloudBrowser.svelte`, `src/cloud.ts` and
`src/cloudMessages.ts`.
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
| `S3Store` (`s3.rs`) | The one adapter: `check_bucket` (the connection check), `list_folder` (one page of one folder level, for the browser), `list_buckets`, `list_objects` (all pages), `head_object`, `upload` (`s3/upload.rs`: refuses an existing key, cancellable through a `TransferControl` until its commit point, progress by stage, verified by `HeadObject`; `upload_file` is it without cancellation or progress), `download` (streamed, with progress and a `TransferControl` for cancellation; `download_file` is it without either), `delete_object`. |
| `CloudAccounts` (`dashboard.rs`) | What the dashboard's commands work on: `Accounts` (or why its database could not be opened) plus the last connection check and the listing cursors per account, in memory. Safe shapes (`AccountInfo`, `ProviderInfo`), structured `CommandError`, account ids from labels. [Dashboard](#dashboard). |
| `cloud::browser` (`browser.rs`) | `list_cloud_folder`: `FolderListing` (folders, files, breadcrumbs, next-page cursor), the prefix and key rules, listing errors. [Browsing a bucket](#browsing-a-bucket). |
| `cloud::download` (`download.rs`) | `download_cloud_file`, `cancel_cloud_download`, `retry_cloud_download`; `CloudDownloads` (the one download slot, managed state), progress messages, the save dialog, download errors. [Downloading a file](#downloading-a-file). |

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
  The read timeout covers a whole request, body included, so requests that
  carry file data (`PutObject`, `UploadPart`) switch it off per operation and
  use a stall timeout instead (60 s without body bytes taken and without a
  response; below).
- **Plain HTTP** is refused except for `localhost`/`127.0.0.1`/`::1` (the test
  server); an endpoint is `scheme://host[:port]` only.
- **Bucket allow-list:** if an account lists buckets, any operation on another
  bucket is refused before a request is sent.

## Uploads and integrity

`S3Store::upload(bucket, key, path, control, progress)` in
`src-tauri/src/cloud/s3/upload.rs`; decision and rejected alternatives:
[ADR-021](decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration).
Not reachable from the app.

1. **Checks** without a request: bucket configured for the account, key 1 to
   1,024 bytes; a transfer already cancelled stops here. Then a slot in the
   account's transfer queue (cancellable).
2. **Hashing** (stage `hashing`): size and modification time noted, then one
   pass over the file for its SHA-256 (stored as user metadata
   `x-amz-meta-sotyware-sha256`, so it must be known before sending) and, for
   multipart, the SHA-256 of each part at its exact boundaries. Cancellable
   after every 1 MiB.
3. `plan_upload` picks a single PUT (up to `multipart_threshold`, 64 MiB, and
   never above the provider's single-PUT limit) or a multipart upload: parts of
   16 MiB, raised to the provider minimum and to whatever keeps the count within
   `max_parts`, whole MiB; never a one-part multipart upload (B2 requires two);
   refused before any request if larger than the provider's object limit.
4. **Destination:** `HeadObject` on the key. An existing object fails with
   `AlreadyExists` before any file data is sent; nothing is overwritten on
   purpose. A multipart upload checks again right before completing. Between
   check and write another client can still create the object, which the
   upload then replaces (or, on a versioned bucket, hides): no
   `If-None-Match: *` is sent, because Backblaze does not document it.
5. **Sending** (stage `sending`): parts are streamed from the file
   (`ByteStream::read_from().offset().length()`, 64 KiB chunks), one after
   another, so memory use does not grow with the file size. Each request
   body hashes the bytes as the connection takes them, with a fresh hash per
   SDK attempt (the SDK rebuilds the body, reopening the file), and holds
   back its last chunk until the hash matches the first pass.
6. **Commit point.** A single PUT: right before its last chunk is handed
   over (hash matched, size and modification time unchanged, then
   `TransferControl` → committed); without that chunk the provider never has
   a complete request. An empty file commits before its request. Multipart:
   after all parts, the second destination check and an unchanged file,
   right before `CompleteMultipartUpload`.
7. **Verifying** (stage `verifying`): `HeadObject` on the version created
   (if the provider named one) must report the size sent and the SHA-256
   metadata. Only then does `upload` return `Ok`.

| What happens | Before the commit point | After it |
| --- | --- | --- |
| `cancel()` | returns `true`; the request in flight is dropped; `Cancelled` | returns `false` ("finishing"); the upload runs to its end |
| File differs from the first pass (content, size, modification time) | `FileChanged`; the last chunk is never sent, multipart is aborted | cannot happen: checked before the commit |
| Request or part fails | that error; multipart is aborted | `CompleteMultipartUpload` failed: aborted; if the provider no longer knows the upload, `NeedsInspection` |
| Verification fails | — | `Integrity` (or the request's error) with `NeedsInspection`; the object is **not** deleted |
| What `UploadError::remote` says | `NothingStored`, or `IncompleteParts { upload_id }` if the abort failed | `NeedsInspection` |

**Multipart abort:** `AbortMultipartUpload` runs in its own task, so it
finishes even if the caller drops the upload, and is given at most
`abort_timeout` (30 s) including retries. If it fails or times out, the
error says that parts may remain (and are billed) at the provider. A process
that ends during a multipart upload aborts nothing; a bucket lifecycle rule
that removes unfinished multipart uploads is the only cleanup then.
`CreateMultipartUpload` is never dropped in flight, so no upload id can exist
that sotyware did not learn.

**Timeouts:** `PutObject` and `UploadPart` run with a per-operation override
(connect timeout kept, read timeout and the SDK's stalled-stream protection
off) and fail with `Timeout` once the body has handed no bytes to the
connection, and no response has come, for `stall_timeout` (60 s). The
measure is the local TCP stack: bytes waiting in the operating system's send
buffer count as taken, so after the last byte the response must come within
the stall timeout *including* the time that buffer needs to drain. Through
the test relay on loopback at 100 KiB/s the last byte was taken after 46 s
and the response came after 72 s: about 2.6 MiB, 26 s of draining. With a
buffer that size, a link slower than about 44 KiB/s would time out after
its last byte although it works; operating systems size the buffer by the
connection's bandwidth and delay, so slow real links usually keep it much
smaller (not measured). Every other request, downloads
included, keeps the 60 s read timeout.

**Progress:** `UploadProgress { stage, bytes, total }`, stages only forward,
bytes never down within a stage (a retried request is not shown until it
passes the earlier position), at most one update per 250 ms except a stage's
first and last; an update held back by the throttle is sent once due.
`verifying` is the last update of a completed upload; completion itself is
the `Ok`.

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
`NotFound`, `Network`, `Timeout`, `Integrity`, `FileChanged`,
`AlreadyExists` (an upload's key is taken), `TooLarge`, `Refused` (bucket
not configured, bad key), `Io` (local file), `LocalWrite` (a download's
destination), `Cancelled`, `Provider { status, code, message }`. `upload`
returns it inside `UploadError` together with `RemoteOutcome` (above). `is_transient()` is true for
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
It manages accounts and opens each one's bucket in a read-only browser
([below](#browsing-a-bucket)).

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
| `list_cloud_folder` | `id`, `prefix`, `page` or `null` | one `FolderListing` ([Browsing a bucket](#browsing-a-bucket)) |
| `download_cloud_file` | `id`, `bucket`, `key`, `onEvent` (channel) | `null` (dialog cancelled) or `{ transfer, fileName }` ([Downloading a file](#downloading-a-file)) |
| `cancel_cloud_download` | `transfer` | `cancelling`, `finishing` or `not-active` |
| `retry_cloud_download` | `transfer`, `onEvent` (channel) | `{ transfer, fileName }` |

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

## Browsing a bucket

Decision: [ADR-019](decisions.md#adr-019-read-only-bucket-browsing-is-one-command-with-cursors-kept-in-rust).
Each account in the list has **Open bucket** (disabled without a bucket or
with settings that do not validate). The browser takes the account list's
place and shows the account's name, its bucket, the current folder, a
breadcrumb trail (bucket → folders), **Up one folder**, **Refresh** and
**← Back to accounts**; then a table of folders (first) and files with size
(binary units, exact bytes on hover) and last-modified date where the
provider gives one. Read-only: it opens folders and nothing else.

```
CloudBrowser.svelte ── listFolder(id, prefix, page) ──▶ browser::list_folder
                                                        │ spawn_blocking: account, cursor, credentials
                                                        ▼
                                                     S3Store::list_folder ── ListObjectsV2
                                                        │   the account's bucket, prefix, delimiter "/",
                                                        │   max-keys 1000, continuation token
                                                        ▼
                                                     entries(): folders, files
                                                        │ spawn_blocking: cursor for the next page
```

`FolderListing` is `{ accountId, bucket, prefix, parent, breadcrumbs:
[{ name, prefix }], folders: [{ name, prefix }], files: [{ name, key, size,
lastModified }], nextPage }`. It never holds a credential or the provider's
token.

**Folders and keys** (`browser.rs`):

- The root is `""`; a folder is a prefix ending in `/`. Other prefixes are
  refused (`invalid-input`), as is one over 1,024 bytes.
- The page opens a folder by sending back the `prefix` Rust gave it; it never
  joins names, so `a//b/`, `a/ /` or `../` keep every character. `..`, `.`,
  `\` and a leading `/` are ordinary characters, never navigation. The bucket
  is always the account's configured one.
- File names are the part of the key after the folder's prefix; folder names
  the part between it and the next `/`. Either may be empty (shown as
  "(no name)"). Names are shown as text, never as markup.
- An object whose key equals the folder's prefix is its **marker** (what
  consoles create for an empty folder): not shown. A folder that is only a
  marker appears in its parent and opens empty ("This folder is empty.").
- Entries outside the prefix are dropped; should a provider ignore the
  delimiter, deeper keys and prefixes are folded into their immediate child
  folder, so nested content still appears only after opening it.

**Pages:** one request returns at most 1,000 entries (folders and files
together). If the provider has more, the listing carries a cursor and the
page shows **Load more**; "N items shown; more are available" is all it says
(S3 gives no total). Cursors live in `CloudAccounts`: tied to the account and
its revision, the bucket and the prefix; retired once used; at most 256 kept;
dropped when the account is edited or removed. Opening another folder and
Refresh start again from the first page; the page also ignores the answer of
any navigation but the latest, and de-duplicates by key and prefix when
appending. A cursor that does not fit, or a token the provider rejects with
HTTP 400, is `outdated-page` (the page offers Refresh); a provider that hands
back the same token, or claims more without one, ends the listing. There is
no cache: going back to a folder lists it again.

**Errors** (`ErrorKind`, text in `cloudMessages.ts`): `credentials-missing`
(with **Edit account**), `credential-store-unavailable`,
`credential-store-locked`, `credential-store`, `invalid-credentials` (with
Edit account), `access-denied` (the text adds that a key limited to a file
name prefix lists only inside it), `bucket-unavailable` (not found *or not
visible to this key*: Backblaze answers 404 for both), `network`, `timeout`,
`invalid-configuration` (with Edit account), `provider` (HTTP status and
code), `outdated-page`, `unknown-account` (the account was removed: the
browser closes, says so and reloads the list). Every failure has **Try
again**; a failed "Load more" keeps what was loaded. Messages are sotyware's
own; the provider's error text goes only to the log.

**Verified against:** the in-process listing server (keys exactly as given,
real continuation tokens, refused operations), through Rust tests and the
release binary's UI on Linux and Windows; and Backblaze B2, read-only
(2026-11-11): the test key, limited to `sotyware-it/`, is refused the root
(`AccessDenied`, "not entitled"). With a temporary 1,040-object dataset
(nested and marker-only folders, odd and Unicode names, `//` and `..` in
keys, 1,010 entries in one folder) every rule above held at Backblaze,
through Rust and through the UI on Linux and Windows, and the listings
equalled the local server's; the one difference: Backblaze answers a
continuation token it never issued with the first page, not an error
([status.md](../status/status.md#cloud-storage-browsing-a-temporary-dataset-at-backblaze-2026-11-11)).
Not seen at Hetzner.

## Downloading a file

Decision: [ADR-020](decisions.md#adr-020-single-file-downloads-go-through-a-save-dialog-opened-by-rust-one-at-a-time-with-progress-on-a-channel).
Every file row in the browser has a **Download** action. One download runs
at a time; while it runs (or its save dialog is open) every Download action
is disabled, and the browser stays usable (folders, Refresh, Load more).

```
CloudBrowser.svelte ── downloadFile(id, bucket, key, onEvent) ──▶ download_cloud_file
   ▲                                                               │ reserve the slot (else busy)
   │                                                               │ spawn_blocking: account, bucket still the same?
   │                                                               │ native save dialog (Rust, tauri-plugin-dialog)
   │                                                               │   cancelled → null, nothing started
   │  { transfer, fileName }  ◀────────────────────────────────────┤ start: spawn on the async runtime
   │                                                               ▼
   │  Channel: DownloadEvent ◀── CloudDownloads::run ── spawn_blocking: credentials (OS store)
   │   active (≤ 4/s), then one of                    S3Store::download ── GetObject (configured bucket, exact key)
   │   completed / cancelled / failed                   │ body chunk by chunk → temporary file, SHA-256 on the way
   │                                                    │ sync, check size (+ stored SHA-256)
   └── cancel_cloud_download(transfer) ──▶ TransferControl: cancel, or "finishing" after the commit point
                                                        │ move into place (hard link / rename)
```

**Where the file goes.** Rust opens the native save dialog (GTK 3 file
chooser on Linux, the common item dialog on Windows) over the dashboard,
titled "Save download as", suggesting the key's last segment with `< > : "
/ \ | ? *` and control characters replaced by `_`, trailing dots and spaces
dropped, `CON`/`NUL`/`COM1`… prefixed with `_`, and `download` for an empty
name (`folder/`, `..`). That is only a suggestion: the path is whatever the
user picks, and the page never sends or sees one (it gets the chosen file's
name for display). The path must be absolute and not a folder.

**Existing files.** Both dialogs ask before returning a file that exists
(GTK's overwrite confirmation; the Windows dialog's "Confirm Save As"), so
such a file is replaced, in one rename once the download is verified; until
then the old file is untouched, and a failed or cancelled download keeps it.
A destination that did not exist must still not exist at the end: the
verified file is hard-linked to it, which fails if something appeared there
meanwhile (`destination-exists`; that file is left as it is). On file
systems without hard links (FAT, exFAT, some network shares) it is checked
and then renamed, so a file created in that one moment would be replaced.

**Temporary file.** The data is written to a new file next to the
destination, `.<name>.<process id>-<n>.sotyware-partial` (hidden on Linux; at
most 100 bytes of the name; opened with create-new, so it never opens or
truncates an existing file). The final move is a rename or link on the same
file system, so the destination is never partly written. The temporary file
is removed on every failure, on cancel, and when quitting during a download.
Only a process that is killed (crash, power loss) leaves it behind.

**Streaming and memory.** The response body is read chunk by chunk (the
SDK's chunks, at most about 1 MiB in tests) and each chunk is written before
the next is read, so memory use does not grow with the file size (a 24 MiB
test shows the temporary file growing while it streams). The transfer runs on
the async runtime; the command returns as soon as it has started. The
client's limits apply: 10 s to connect, 60 s read timeout; the SDK's
stalled-stream protection ends a body that delivers nothing for its 5 s grace
period (about 6 s in the test), which is reported as a network failure. A
download is one `GetObject`; there are no retries within a download and no
resuming.

**Progress** goes over a Tauri `Channel` created for that one download:
`{ transfer, state, bytes, total, integrity, error }`, `state` one of
`active`, `completed`, `cancelled`, `failed`. `total` is the size the provider
announced, `null` if it gave none (the page then shows bytes without a
percentage). "active" messages are throttled to one per 250 ms (the first one
always goes); then exactly one final message follows, sent after the slot is
free and, for `completed`, after the verified file is in place. The page
accepts messages only for its current download and transfer id (also one
that arrives before the command's reply), ignores everything after the final
one, and treats a later download's messages separately (each download has its
own channel).

**Cancel** (`cancel_cloud_download`) acts only on the running transfer with
that id; any other id, a finished one, or a repeat returns `not-active` or
the same answer again. It stops waiting for the credential store, drops the
request or body stream (closing the connection) and removes the temporary
file; the final message is `cancelled`. The race with completion is decided
by `TransferControl`: once the verified file is about to be moved into place
the answer is `finishing` and the download completes; a cancel just before
that point wins and nothing is written. Leaving the browser (Back to
accounts) cancels a running download. Quitting (`RunEvent::Exit`) refuses
new downloads, cancels the running one and waits up to 5 s for its cleanup,
then removes its temporary file itself if it is still there; a download that
is already finishing is waited for, not cut off.

**Integrity.** The file's SHA-256 is computed while it streams. If the
object has the SHA-256 that sotyware stores at upload
(`x-amz-meta-sotyware-sha256`), it must match, and the page says "The SHA-256
checksum stored with the file matched." Otherwise only the size the provider
announced is checked, and the page says the content could not be verified.
A size or checksum mismatch fails the download (`integrity`) and nothing is
saved. sotyware does not claim provider-side checksums.

**Failures and retry.** A failed download shows its reason (`busy`,
`outdated-page` if the account's bucket changed, `unknown-account`,
`credentials-missing` / `invalid-credentials` / `invalid-configuration` with
**Edit account**, `credential-store-*`, `access-denied`, `object-not-found`,
`network`, `timeout`, `provider`, `integrity`, `disk-full` (also over quota),
`local-permission` (also a read-only file system), `destination-unavailable`
(folder gone, drive disconnected), `destination-exists`, `local-file`) and
"Nothing was saved". **Try download again** repeats the last failed download
as a new transfer to the same destination without a dialog (a start that
was refused before the dialog asks again). Cancelled downloads are not
retried. Expired or wrong keys fail like the listing does.

**Accounts.** Each download uses the account's own `S3Store` and
credentials; the account and bucket are checked again when the transfer
starts. Editing or removing an account during a download does not stop it.

**Verified against:** the in-process object server (exact keys, bodies that
are slow, stall or break off, wrong checksums, no announced size), through
Rust tests and the release binary's UI with the real native save dialog: GTK
under Xvfb (WSL2, a virtual display) and the common item dialog on the real
Windows 11 desktop (2026-11-12,
[status.md](../status/status.md#cloud-storage-single-file-downloads-2026-11-12)).
At a real provider (Backblaze B2, a 16 MiB file, 2026-11-12): once without
the UI through `CloudDownloads` (WSL2); its `GetObject` carried
`Content-Length` and the stored `sotyware-sha256`, and the download was
SHA-256 verified
([status.md](../status/status.md#one-download-from-backblaze-2026-11-12));
and once through the native Windows 11 dashboard and save dialog, verified
byte for byte
([status.md](../status/status.md#real-backblaze-download-through-the-windows-dashboard-2026-11-12)).

## Not built (deliberately)

No upload UI, folder downloads, several downloads at once, download history,
file operations in the browser (rename, delete, create folder, move),
transfer queue UI, custom event or feature; no synchronisation, folder watching,
scheduled backups, retention, restic integration, client-side encryption,
sharing or public links, virtual filesystem, retrying queue, parallel parts
within one file, resumable uploads, credential import from environment
variables or files, automatic provider or bucket discovery, cost estimator,
or a credential fallback outside the OS store. See
[roadmap.md](../status/roadmap.md).

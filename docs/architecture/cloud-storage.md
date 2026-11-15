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
Uploads of up to 100 picked files into one folder, one file after another,
run from the browser's upload panel ([Uploading files](#uploading-files),
[ADR-022](decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)).
Each file goes through the backend's upload lifecycle (commit point,
cancellation, re-hashing, verification, stall timeout;
[Uploads and integrity](#uploads-and-integrity),
[ADR-021](decisions.md#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration)).
File management (folders, rename, move, copy, delete, restore, search,
properties, previews, share links, several-item downloads, folder uploads,
drag and drop, favorites; [File management](#file-management),
[ADR-026](decisions.md#adr-026-file-management-runs-as-reviewed-plans-in-rust-with-s3-semantics-made-explicit))
changes objects only through reviewed plans.
Both are tested against the in-process server only. Decision and rationale of
the backend: [ADR-016](decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter).
The provider research, prices and the recommendation are in
[cloud-providers.md](cloud-providers.md).

Code: `src-tauri/src/cloud/` (`provider.rs`, `account.rs`, `credentials.rs`,
`keyring.rs`, `s3.rs` with `s3/upload.rs` and `s3/manage.rs`, `dashboard.rs`,
`browser.rs`, `download.rs`, `upload.rs`, `operations.rs`, `manage.rs`,
`bulk_download.rs`, `places.rs`, `drop.rs`) and `SqliteAccountStore` /
`SqlitePlacesStore` in `src-tauri/src/storage.rs`; the page is
`src/CloudStorage.svelte` with `src/CloudAccountForm.svelte`,
`src/CloudBrowser.svelte` (and the file-manager components listed under
[File management](#file-management)), `src/cloud.ts`, `src/cloudFiles.ts`,
`src/fileList.ts` and `src/cloudMessages.ts`.
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
Called once per file by upload batches ([Uploading files](#uploading-files)).

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
   upload then replaces (or, on a versioned bucket, hides), unless the
   provider's `conditional_put` capability is established (ADR-029): then
   `PutObject` and `CompleteMultipartUpload` carry `If-None-Match: *`, and a
   412 answer is inspected (`HeadObject`): this upload's own content (an SDK
   retry after a lost answer) counts as uploaded, anything else fails with
   `AlreadyExists` and nothing stored (an open multipart upload is aborted).
   No shipped profile has it established: Backblaze and Hetzner do not
   document it and it was never verified at either.
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
| `cancel()` | returns `true`; the request in flight is dropped (a part already handed over completely is first awaited, at most 5 s); `Cancelled` | returns `false` ("finishing"); the upload runs to its end |
| File differs from the first pass (content, size, modification time) | `FileChanged`; the last chunk is never sent, multipart is aborted | cannot happen: checked before the commit |
| Request or part fails | that error; multipart is aborted | `CompleteMultipartUpload` failed: aborted; if the provider no longer knows the upload, `NeedsInspection` |
| Verification fails | — | `Integrity` (or the request's error) with `NeedsInspection`; the object is **not** deleted |
| What `UploadError::remote` says | `NothingStored`, or `IncompleteParts { upload_id }` if the abort failed | `NeedsInspection` |

**Multipart abort:** `AbortMultipartUpload` runs in its own task, so it
finishes even if the caller drops the upload, and is given at most
`abort_timeout` (30 s) including retries. If it fails or times out, the
error says that parts may remain (and are billed) at the provider. From
`CreateMultipartUpload` until completion or a successful abort, the upload
id is also held by the `TransferControl` (`open_multipart()`), so the
caller never loses it; upload batches keep failed ids in memory and abort
them again later (`S3Store::abort_incomplete_upload`, same timeout; see
[Uploading files](#uploading-files)). A process that ends before an abort
succeeded forgets the id; a bucket lifecycle rule that removes unfinished
multipart uploads is the only cleanup then.
`CreateMultipartUpload` is never dropped in flight, so no upload id can exist
that sotyware did not learn.

**A part handed over completely is awaited**
([ADR-025](decisions.md#adr-025-a-cancelled-part-that-was-handed-over-completely-is-awaited-before-the-abort)):
when the cancel comes after the connection took a part's last byte, that
part reaches the provider even if its request is dropped, and could land
after the abort and remain. The cancel therefore waits for the part's
answer, at most `part_settle_timeout` (5 s), before the abort; past the
bound the abort goes out anyway and the part may still land (logged).

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
`Verification::SizeOnly`; if the provider announced no size either, nothing
could be compared and the download is reported as `Verification::TransferOnly`
(`transfer-only`, 2026-11-15): a completed transfer is never called
size-verified or integrity-verified unless that comparison happened
(`s3::verification_of`). ETags are not used: they are not an MD5 for multipart
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

## Backups with restic (superseded, not built)

**Superseded on 2026-11-15 by sotyware's own backup engine**
([backup.md](backup.md), ADR-031): restic's encrypted repository format
would make recovery depend on restic and its password. The findings below
are kept for reference. Findings (restic 0.19.1, docs and the experiment in
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
It manages accounts and opens each one's bucket in a browser
([below](#browsing-a-bucket)) that lists folders, downloads one file at a
time and uploads files into the folder shown ([the upload
panel](#the-upload-panel)); nothing in it renames or deletes.

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
provider gives one. Listing is read-only; the browser's only actions that
touch objects are Download ([below](#downloading-a-file)) and **Upload
files…** ([below](#the-upload-panel)).

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

## Uploading files

Decision: [ADR-022](decisions.md#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust).
Code: `src-tauri/src/cloud/upload.rs` (`CloudUploads`, managed state);
typed wrappers `uploadFiles`, `cancelUpload`, `retryUpload` and checked
parsers in `src/cloud.ts`; the page in `src/CloudUpload.svelte`
([below](#the-upload-panel)).

```
page ── upload_cloud_files(id, bucket, prefix, onEvent) ──▶ validate prefix; reserve the upload slot (else busy)
  ▲                                                        │ spawn_blocking: bucket still the account's? provider limits
  │                                                        │ native multi-file dialog (Rust)
  │                                                        │   cancelled / empty → null, nothing started
  │                                                        │   > 100 files → invalid-input, nothing started
  │                                                        │ spawn_blocking: check every file (no reading)
  │  { batch | null, prefix, files[] } ◀───────────────────┤ start (or batch: null if no file can go)
  │                                                        ▼
  │  Channel: UploadEvent ◀── CloudUploads::run ── [retry: abort known incomplete uploads → cleanup events]
  │   file (state, stage, bytes) …                  spawn_blocking: credentials (OS store), once
  │   finished (summary), once, after the slot      for each file, in order:
  │   is free                                         HeadObject → exists: skipped
  │                                                   S3Store::upload (ADR-021) → completed / failed / cancelled
  └── cancel_cloud_upload(batch) ──▶ current file cancelled (or "finishing"), no further file starts
      retry_cloud_upload(batch, onEvent) ──▶ a new batch with what is left
```

**Destination.** `prefix` + file name in the account's configured bucket.
The page passes the folder it shows when it asks for the dialog (`""` for
the top level, otherwise a prefix ending in `/`, as the listing returned
it); that folder is the batch's, whatever the page shows later. `bucket`
must still be the account's configured bucket (`outdated-page`).

**Selection.** The native dialog ("Choose files to upload", several files)
returns paths to Rust only. A cancelled or empty dialog returns `null`. More
than 100 files (`MAX_FILES`) are refused as a whole. Every file is checked
before anything is sent, without reading its content (`check_selection`):

| Check | Skip reason |
| --- | --- |
| A regular file (a link to one is followed) | `not-a-file` |
| Its size can be read | `unreadable` |
| The name is valid UTF-8 | `invalid-name` |
| No control characters in the name | `control-characters` |
| `prefix` + name ≤ 1,024 bytes | `key-too-long` |
| Size within the provider's limits (`plan_upload`) | `too-large` |
| No other selected file has the same key (all of them are skipped) | `duplicate-key` |
| (before its turn) no object at the key | `destination-exists` |

The command returns the batch id (or `null` if no file can go: then no batch
exists and the slot is free) and every file as `pending` or `skipped` with
its reason and a sentence. Skipped files are never uploaded.

**One file after another.** The batch loads the account's credentials once
(blocking pool), then takes the files in order. Before each one a
`HeadObject` on its key: an existing object skips the file before it is
read (nothing is ever overwritten). Then `S3Store::upload` does the rest
(its own destination checks, hashing, sending, verification). One batch at
a time; a second batch or dialog is `busy`. Uploads have their own slot, so
one download can run beside a batch; both still wait for the account's
transfer queue (four transfers by default).

**What stops what.**

| A file ends with | The file | The batch |
| --- | --- | --- |
| verified upload | `completed` | goes on |
| an existing object (our check or the upload's) | `skipped` (`destination-exists`) | goes on |
| `file-changed`, a local read error, `integrity`, `too large`, a non-transient provider refusal (4xx) | `failed` | goes on |
| `invalid-credentials`, `access-denied`, `bucket-unavailable`, `invalid-configuration`, `network`, `timeout`, provider 5xx/429 | `failed` | stops: `failed`, with that error |
| a cancel | `cancelled` | stops: `cancelled` |

Files the batch never reached end as `cancelled` with `started: false`. A
batch is `completed` only when every file ended as completed, skipped or
failed. Failing to load the credentials fails the batch before any file.

**States and events.** File states: `pending`, `hashing`, `sending`,
`verifying` (the stages of `S3Store::upload`), then `completed`, `skipped`,
`failed` or `cancelled`. Every file carries `index`, `name` (not its
folder), `key`, `size`, `state`, `stage` (the last reached), `bytes`,
`total`, `started`, `skip`, `error`, `remote` (`nothing-stored`,
`incomplete-parts`, `needs-inspection`) and `retryable`. Events on the
channel passed with the command: `{ batch, seq, kind: "file", count, file }`
for every state change and for progress (as throttled by `S3Store::upload`:
at most one per 250 ms within a stage, first and last of each stage always;
never backwards; a file is `sending` from the moment a request carrying its
data starts, at the bytes of the parts before it, so a cancel or failure
right after the first bytes is never reported at `hashing`),
`{ kind: "cleanup", key, outcome }` on a retry, and
exactly one `{ kind: "finished", summary }` with the batch state, counts
(`completed`, `skipped`, `failed`, `cancelled`, `notStarted`, `retryable`,
`needsAttention`), the stopping error and every file in its final state.
`finished` comes last and after the slot is free. `seq` counts from 1 per
batch; batch ids (`upload-<n>`) are never reused, and each batch has its own
channel, so nothing from an earlier batch can reach a later one.

**Cancel** (`cancel_cloud_upload`) acts only on the running batch with that
id; any other id or a finished batch is `not-active`, a repeat gives the
same answer. It cancels the current file (waiting for credentials, our
`HeadObject`, hashing, sending; a multipart upload is aborted) and keeps
the rest from starting. If the current file has passed its commit point the
answer is `finishing`: that file completes, no other starts, and the batch
ends `cancelled`. Leaving the browser cancels a running batch, as for
downloads ([the upload panel](#the-upload-panel)); hiding or minimising the
dashboard does not.

**Retry** (`retry_cloud_upload`) works on the last finished batch only and
starts a new batch (new id) into the same folder with its files that are
`retryable`: failed or cancelled with `nothing-stored`, or never started.
They are checked again like a new selection, and the destination again
before each file. Completed and skipped files are never retried, nor any
with `needs-inspection` (the provider may hold the object: check it in the
bucket) or `incomplete-parts`. Starting any new batch drops the old retry.

**Incomplete multipart uploads.** When `S3Store::upload` cannot abort a
multipart upload (`incomplete-parts`), the batch keeps its account, bucket,
key and upload id, with the store that created it, in memory. A retry of a
batch of that account aborts each one again first and reports a `cleanup`
event (`removed` or `failed`); a retry may consist of only this. Quitting
aborts every one. An id is forgotten only when the provider says the upload
is gone; a failed attempt is never reported as removed. Nothing is written
to disk: after sotyware ends (or crashes) the parts can only be removed by
a bucket lifecycle rule for unfinished multipart uploads, which sotyware
does not configure.

**Shutdown** (`RunEvent::Exit`, after the downloads' shutdown): no new batch
starts (also not one whose dialog was still open), the running batch is
cancelled, and every known incomplete upload is aborted, concurrently. The
wait is bounded: 35 s (`SHUTDOWN_WAIT`: the 30 s abort timeout plus 5 s), in
practice the time of one abort request. What is not done by then is logged
("parts may remain"), and the process ends.

**Privacy.** No command result or event contains a local path: file names
only, and sotyware's own sentences for local errors (their messages name
the path and are only logged). Provider text is never shown.

**Verified against:** the in-process S3 server only (WSL2, 2026-11-12,
[status.md](../status/status.md#upload-batches-backend-and-commands-2026-11-12)).
Later, through the Rust batch path at Backblaze once on WSL2 and once on
native Windows 11 (2026-11-13,
[WSL2](../status/status.md#uploads-at-backblaze-2026-11-13),
[Windows](../status/status.md#uploads-at-backblaze-on-native-windows-11-2026-11-13));
never at Hetzner.

### The upload panel

`src/CloudUpload.svelte`, inside the bucket browser (`CloudBrowser.svelte`),
both modes. It shows one batch, the latest, exactly as the events describe
it; it never decides a file's state, its progress, a conflict or whether it
can be retried.

**Workflow.**

1. In any folder, **Upload files…** (beside Up one folder and Refresh) calls
   `upload_cloud_files` with the account, the bucket the browser shows and
   the folder's prefix as Rust listed it (`""` at the top). The panel shows
   "Choose the files to upload…" and the button is disabled.
2. Rust opens the native multi-file dialog. Cancelled or empty: the panel
   goes back to what it showed before (nothing, or the previous result), no
   error. Refused (`busy`, more than 100 files, missing key, …): "The
   upload did not start", with the reason (and Edit account where that fixes
   it). An account removed meanwhile closes the browser, as for downloads.
3. The reply lists every chosen file: `Waiting`, or `Skipped:` with the
   reason (not a file, could not be read, name cannot be an object key,
   control characters, name too long for this folder, too large for this
   provider, same name as another selected file) and Rust's sentence. If no
   file can go: "None of the chosen files can be uploaded, so nothing was
   sent."
4. While the batch runs: "*n* of *m* files finished" with a progress bar by
   finished files (skipped ones count as finished), and for the file being
   uploaded "File *i* of *m*: name", its stage (Computing the SHA-256
   checksum / Sending / Verifying the stored object), bytes of total, and a
   percentage only when the total is above 0. Every file has a row with its
   status (Waiting, its stage, Uploaded and verified, Skipped: …, Failed,
   Cancelled during its upload, Not started: …) and notes below. A file is
   shown as uploaded only after its `completed` event. The list scrolls
   (at most about 15 lines high), so a batch of 100 does not push the folder
   out of view.
5. At the end the `finished` summary replaces the files' states: "Upload
   finished / cancelled / stopped after an error. *m* files: *x* uploaded,
   *y* skipped, *z* failed, … not started.", the stopping error, and "*k*
   files need attention" for `needs-inspection` or `incomplete-parts`. Then
   **Try *n* files again** (only if the summary's `retryable` is above 0),
   **Show destination folder** (if anything was uploaded: opens the batch's
   folder, which also lists it again), and **Dismiss**.

The destination ("Upload to bucket / prefix") is shown from the values
captured at step 1 (the reply's `prefix` replaces it); browsing to other
folders meanwhile changes nothing for the batch.

**Messages** (`cloudMessages.ts`: `SKIP_LABELS`, `REMOTE_NOTES`,
`describeUploadError`), chosen by the structured reason, error kind and
remote state, with Rust's sentence as the detail. No sentence says "try
again" by itself: a retry is offered only where the backend set
`retryable`. Per remote state: `nothing-stored` "Nothing was stored for this
file."; `needs-inspection` "The provider may have stored this file, but
sotyware could not verify it. Check the object in the bucket …; sotyware
does not retry or delete it."; `incomplete-parts` "parts of it may remain at
the provider (and may be billed) …, tried again when this account's upload
is retried and when sotyware quits; otherwise only a lifecycle rule on the
bucket removes them". Upload errors have their own summaries (access denied
mentions write permission and a key limited to a prefix; `file-changed`,
`integrity`, `network`, `timeout`, `local-file` each say what happened).
Nothing shown contains a local path (Rust sends none); an event without the
Rust shape is reported as "an upload message it does not understand",
without its content.

**Cancel upload** is enabled while the batch runs and its id is known, once:
the first click disables it. `cancelling` shows "Cancelling…";
`finishing` "The current file is already being completed at the provider
and will finish; no further file starts."; `not-active` "The upload already
ended; its result follows." A note says beforehand that the current file
may finish. The batch is shown as cancelled only when the `finished` event
says so; files are told apart by `started`: "Cancelled during its upload"
(with its remote note) versus "Not started: the upload was cancelled" (or
"… stopped after an error" when the batch failed). A failed cancel request
is shown and the button is enabled again.

**Try again** calls `retry_cloud_upload` with the finished batch's id (no
dialog) and shows the new batch ("Retrying upload to …") with only the files
Rust took. `cleanup` events are listed apart from the files: "The
incomplete parts left by an earlier failed upload of *key* were removed.
That earlier upload was not completed." or "… could still not be removed".
A refused retry shows the earlier result again with the reason; for
`invalid-input` (Rust no longer holds that batch, or nothing is left) the
button is removed, otherwise it stays. Files with `needs-inspection` or
`incomplete-parts` are never retried (Rust does not mark them `retryable`),
and a batch whose only failures are of these kinds offers no Try again.

**Stale events and lifecycle.** Every start and retry gets a new local
number and a new channel; an event or reply is applied only while that
number is the one shown, only for its batch id (learnt from the reply or
the first event, whichever comes first), and never after `finished`. Within
a batch, a file's event is applied only if its `seq` is above the last one
applied to that file, so progress never goes back and the reply's
`pending` never overwrites an earlier event. Leaving the browser (Back to
accounts, Edit account, another section, another account's bucket: the
browser is keyed by account) unmounts the panel; a running batch is then
cancelled by its id, and a reply that arrives after unmount (the dialog was
still open) is cancelled on arrival. After unmount nothing is applied.
Hiding or minimising the dashboard does not unmount anything, so uploads go
on. A download can run beside a batch; neither affects the other.

**Verified against:** frontend tests only (`src/CloudUpload.test.ts`,
Vitest with Tauri's IPC mocks, 2026-11-12,
[status.md](../status/status.md#cloud-storage-upload-ui-2026-11-12)); no
desktop run, the native dialog was never opened, no real provider.

## File management

[ADR-026](decisions.md#adr-026-file-management-runs-as-reviewed-plans-in-rust-with-s3-semantics-made-explicit).
Tested against an in-memory versioned S3 backend (`test_server::Versioned`)
and s3s-fs only; **never against a real provider**.

### What S3 means for each action

| Action | Requests | What it means |
| --- | --- | --- |
| New folder | `ListObjectsV2` (1 key) under `prefix/name/`, then `PutObject` of the zero-byte marker | refused if anything is already at or under that prefix; a folder exists only through its objects, the marker makes an empty one visible |
| Copy | per object: `HeadObject` (destination must not exist), `CopyObject` (`MetadataDirective: COPY`, `x-amz-copy-source-if-match` = planned ETag), `HeadObject` (verify) | content type, `Cache-Control`, `Content-Encoding`, `Content-Disposition` and user metadata (including `sotyware-sha256`) are copied; the copy gets a new date, a new version id and may get another storage class; above 5 GiB refused |
| Move, rename | copy as above for every object of the item, then per object `HeadObject` (source ETag unchanged) and `DeleteObject` | originals deleted only after every copy is verified; on a versioned bucket they stay restorable |
| Delete | per object `HeadObject` (ETag unchanged) and `DeleteObject` without version id | versioned bucket: delete marker, restorable; otherwise gone; the answer (`x-amz-delete-marker`) is reported per object, except that a marker with version id `null` over a version whose id is `null` (versioning suspended) counts as gone for good, not restorable (`restorable_after_delete`, since the 2026-11-14 audit) |
| Restore | planning: `ListObjectVersions` of the key; then `DeleteObject` with each newer delete marker's version id (one already gone, 404, is skipped: a retry after a lost answer), `HeadObject` | the newest kept version becomes current; nothing else changes; only ids the listing reported as delete markers are ever deleted |
| Deleted items | `ListObjectVersions` under the folder (≤ 5,000 entries) | keys whose latest entry is a delete marker; restorable if an older version exists |
| Search | `ListObjectsV2` without delimiter, ≤ 10 pages per call | `name`: file and folder names containing the text, case-insensitive, matched in Rust; `prefix`: keys starting with folder + text, filtered by the provider; never file contents |
| Properties | `HeadObject` | size, date, content type, ETag, version id, storage class, headers, user metadata, stored SHA-256 |
| Preview | `HeadObject`, ranged `GetObject` | ≤ 256 KiB of text (shown as text); raster images ≤ 10 MiB by their first bytes (PNG, JPEG, GIF, WebP, BMP); never SVG, HTML or PDF |
| Share link | `HeadObject`, then local signing | presigned `GetObject` URL, 1 h, 1 day or 7 days; carries the key id and a signature, never the secret; cannot be withdrawn |

### Plans, items and states

A plan names items (selected files and folders; a file inside a selected
folder is part of the folder). Each item has its objects (keys, sizes,
ETags) and, for copy and move, its destination; an item whose destination
exists has a conflict and, if one was found within 20 lookups, a free
`name (n)` (`report (1).pdf`, `photos (1)/`). Problems (moving into
itself, already there, gone, over 5 GiB, a key that would exceed 1,024
bytes) leave an item out; the review shows why. When every item is left
out (a folder pasted into itself, a file moved into its own folder),
no review opens: the page discards the plan and says why instead.

Item states: `pending`, `running`, `completed`, `skipped`, `failed`,
`partial` (with a note: `copied-original-kept`,
`original-kept-copy-incomplete`, `incomplete`, `source-not-empty`),
`cancelled`. A folder that was moved, renamed or deleted completely is
checked once more (one `ListObjectsV2` of one key under its prefix): if
objects were added after the review, the item is `partial` with
`source-not-empty` (they stay where they are; a new review includes them),
never `completed`. Operation states: `completed`, `partial`, `cancelled`
(only if an item was left undone: a cancel after the last request does not
turn a finished operation into a cancelled one), `failed` (stopped by an
error that would fail every further request: credentials, access,
connection, timeouts, provider 5xx/429). Errors carry an `ErrorKind`, including the
new `remote-changed` (an object changed or appeared after the review),
`too-large` and `outdated-plan`.

Retry starts a new operation with: objects that failed with a retryable
error or never started; for moves, the originals of fully copied items
(their copies are checked again first). A destination that exists on a
retry counts as copied only with the source's size and ETag. Conflicts,
changed objects and integrity failures are never retried; a move with such
a failure is not retried at all.

### Downloads of several items

`download_cloud_items` (selected files and folders, ≤ 10,000 files): Rust
first lists the selected folders and checks each selected file
(`ListObjectsV2`, `HeadObject`), then opens a folder dialog (cancelling it
starts nothing and shows nothing); files go into `.sotyware-download-<pid>-<n>` inside
the chosen folder through `S3Store::download`; an item moves into place
(hard link for a file, refusing an existing name atomically, else rename;
a folder by rename after an existence check) once all its files are
verified, under its own name or `name (n)`. Key segments become local names
with `< > : " / \ | ? *` and control characters replaced by `_`, cut to
200 bytes and only then trailing dots and spaces removed, reserved device names (`CON`, `NUL`, `COM1`, …)
prefixed with `_`, `.`, `..` and empty names as `_`; clashes (also by case)
are numbered, and so is a file whose local path is also a folder's (the
keys `a/b` and `a/b/c`: `a/b (1)` and `a/b/c`). Events carry names below the
chosen folder, never its path. A batch is `cancelled` only if an item was
left undone. The summary's file and byte counts include only files of
saved items (an item that failed or was cancelled is removed with the
files it had completed).

### Folder uploads and drag and drop

`upload_cloud_folder` opens a folder dialog and builds one upload batch from
every regular file below it (≤ 1,000; symbolic links skipped, empty folders
add nothing) with keys `prefix + <folder name>/<path>`; a name that is not
valid Unicode anywhere on the path skips that file. Dropped files and
folders (`WindowEvent::DragDrop`) are kept in `CloudDrops` for 2 minutes;
while a bucket is shown the page subscribes (`watch_cloud_drops`), sees
counts and names, and confirms with `upload_cloud_drop` into the folder
shown. Everything else is the upload batch of
[Uploading files](#uploading-files).

### The page

`CloudBrowser.svelte` keeps its first three columns (Name, Size, Last
modified, as the desktop drivers read them) and adds Type, a selection
checkbox and actions (Download, a menu). Selection: click, Ctrl/⌘-click,
Shift-click (range from the last clicked row), the checkboxes, Ctrl+A;
arrow keys move it (Shift extends). A selection bar and a context menu
(right-click or ⋯) offer Download, Copy to…, Move to…, Copy/Cut (internal
clipboard; Paste in another folder), Rename (F2), Delete (Del),
Properties, favorites and, for search hits, Show in folder. Shortcuts are
ignored while typing in a field (Escape still closes). Sorting (bucket
order, name, size, date, type) and filters (files/folders, categories,
extensions, size and date ranges) apply to the loaded entries only.
`CloudOperation.svelte` shows the review and the progress;
`CloudFolderPicker.svelte` picks destinations (and creates folders);
`CloudProperties.svelte` shows properties, the preview and share links;
`CloudBulkDownload.svelte` the several-item downloads; `CloudDeleted.svelte`
the deleted items. Favorites and recent items (folders opened, files
previewed or downloaded; 30 kept) live in `cloud-places.sqlite3`.

## Operation journal and recovery

ADR-030 (2026-11-15). Code: `src-tauri/src/journal.rs` (`Journal`,
`JournalLink`, `JournalStore`, startup cleanup), `storage.rs`
(`SqliteJournal`, `operations-journal.sqlite3`, own append-only
migrations), the hooks in `cloud/s3.rs` (download), `cloud/s3/upload.rs`,
`cloud/bulk_download.rs` and `cloud/operations.rs`, the commands in
`cloud/recovery.rs`, the page `src/CloudRecovery.svelte` (shown at the top
of Cloud Storage).

**What is recorded, and when.** Each slot (`CloudUploads`, `CloudDownloads`,
`BulkDownloads`, `CloudOperations`) gets the journal once in `run()`
(`record_in`) and hands it to each `TransferControl`:

| Work | Entry written | Updated | Removed |
| --- | --- | --- | --- |
| Upload (`S3Store::upload`) | after the first hashing pass, before the destination check and any request that could store: account, bucket, key, local path, size, SHA-256, multipart or not | the multipart upload id right after `CreateMultipartUpload`, before any part | when the outcome is confirmed (stored and verified, or nothing stored); kept as `needs-inspection` or `incomplete-parts` otherwise, until a later abort of that id succeeds |
| Download (`S3Store::download`) | right after the temporary file is created, before any byte | — | once the temporary file is gone (renamed into place or removed) |
| Several-item download | the staging folder, before anything is downloaded into it; each file's temporary file as above | — | once the staging folder is removed |
| File operation | before the first request: kind, account, bucket and every planned step (source, planned ETag, destination) | each step's result after every phase of every item (a move's deletes start only once its copies are on record) | at the end, unless some steps took effect and others did not (`partial`) |

A failed write on an open journal stops the step it was meant to record
(`StorageError::Journal`, error kind `journal`): an upload or download does
not start, an operation changes nothing. A journal that cannot be opened is
left untouched like every database; work then goes on unrecorded and the
page says so.

**At startup** (before the event loop), `Journal::clean_local` removes the
temporary download files and staging folders that earlier sessions
recorded, only when the recorded path's last component has sotyware's
pattern (`.<name>.<pid>-<n>.sotyware-partial`, `.sotyware-download-<pid>-<n>`)
and is a regular file or a real folder (not a link). Nothing else is
removed and nothing is sent.

**The recovery view** lists every remaining entry with what is known
locally (`summary`), never a local path (an upload shows its file name).
*Inspect* (`inspect_cloud_recovery`) reads only: an upload's key by
`HeadObject` (`stored-verified` when size and SHA-256 match → nothing left to
do; `absent`; `different` → another writer's object, left alone) and, for a
multipart upload, `ListMultipartUploads` filtered to exactly that key (each
marked `recorded` when the journal knows its id); an operation's steps (at
most 500 per inspection) by `HeadObject` on source and destination, sorted
into `moved`, `original-kept`, `not-started`, `destination-differs`,
`missing`, `copied`/`not-copied`, `deleted`/`still-there`/`changed-since`,
`restored`/`not-restored`, with up to 50 keys per category. The only
changes the view can make are *Dismiss* (removes the entry, after a
confirmation that warns when parts may remain) and *Abort…* on one
incomplete multipart upload (after a confirmation that warns when sotyware
did not record it: another device may still be uploading). Finishing an
interrupted move is a normal reviewed delete of the kept originals in the
browser.

**Unfinished large uploads** (`list_cloud_incomplete_uploads`): every
incomplete multipart upload in an account's bucket (up to 5,000), each
abortable one by one the same way.

Tested against the in-process servers (`cloud/recovery/tests.rs`): a crash
during a multipart upload simulated by dropping the running future (entry
`multipart-open` with the id, inspection, bucket listing, abort, entry
resolved), uploads judged by what the provider holds, a partial move kept
with `original-kept` and inspected without changing anything, a completed
operation leaving nothing, a download's temporary file recorded while it
runs and removed by the next start after a "crash", and journal write
failures refusing uploads, downloads and operations before any request.
The test server answers `ListMultipartUploads` from s3s-fs's files (s3s-fs
itself answers 501). Not run at a real provider.

## Request recorder

Test builds for controlled integration runs only
([ADR-024](decisions.md#adr-024-an-opt-in-request-recorder-observes-the-apps-own-s3-requests-through-an-sdk-interceptor-compiled-only-into-a-test-build)).
**Not in any normal or shipped build**, and never run against a real
provider so far.

- **Build gate:** the Cargo feature `request-recorder` (off by default;
  optional `serde_json`, `aws-smithy-runtime-api`, `aws-smithy-types`)
  compiles `src-tauri/src/cloud/recorder.rs`. Without it the module, the
  environment reads and the interceptor do not exist. `build.rs` refuses
  the feature unless the target directory is named `recorder`
  (`--target-dir target/recorder`), so the recorder binary never replaces
  `target/release/sotyware`. `scripts/check-release-binary.mjs` checks
  that no default turns the feature on (`npm run check:release-binary`, in
  `test:headless` and CI), that the binaries the manual desktop workflows
  test contain no recorder, and, as Tauri's `beforeBundleCommand`, refuses
  to package when the `tauri build` command line asks for the feature or
  the binary to be bundled contains the recorder's marker
  (`sotyware-request-recorder`). A recorder binary that a bundler patched
  anyway (`tauri::utils::platform::bundle_type()`) never records.
- **Runtime gate:** recording starts in `run()`, before any store exists,
  only if `SOTYWARE_REQUEST_RECORD_DIR` (absolute path of an existing
  directory, not a symbolic link), `SOTYWARE_REQUEST_RECORD_RUN`
  (`[a-z0-9-]`, 8 to 64) and `SOTYWARE_REQUEST_RECORD_SCOPE` (a key prefix
  ending in `/`) are all set and valid; `SOTYWARE_REQUEST_RECORD_ACCOUNT`
  (an account id) is optional. None set: nothing happens. Some set, or any
  invalid, or the file cannot be created: no recorder (one `sotyware:` line
  on stderr); the app runs unchanged. No file, thread or interceptor exists
  then.
- **Operations recorded by name** (ADR-024, extended by the 2026-11-14
  audit): `HeadBucket`, `ListBuckets`, `ListObjectsV2`, `HeadObject`,
  `GetObject`, `PutObject`, the multipart requests, `DeleteObject` (whether
  with a version id, never the id), `CopyObject` (destination and source
  keys as run-scoped hashes or `outside-scope`, source bucket
  `configured`/`other`, whether a version and an ETag condition were sent,
  whether metadata is copied), `ListObjectVersions` (prefix, whether key and
  version markers were sent) and `GetBucketVersioning`; anything else is
  `unknown` with no field.
- **Integration point:** one read-only SDK interceptor per `S3Store`,
  added in `S3Store::new` (`Intercept` hooks `read_before_execution`,
  `read_before_transmit`, `read_after_attempt`, `read_after_execution`).
  It never returns an error, never reads a credential, a body, an error
  message or a header other than `content-length` (request) and
  `x-amz-version-id` (response), and changes nothing the client does
  (retries, timeouts, order, cancellation). What it relies on in the
  locked SDK is tested in every build (`cloud/s3/hook_assumptions.rs`).
- **File:** `<dir>/<run>.<pid>.requests.jsonl`, created exclusively (an
  existing file or symbolic link is refused, never followed; mode 0600 on
  Unix). One file per process, so a run that relaunches the app gets one
  file per launch. JSON Lines: `start` (format, run, app version, pid,
  hashed scope, queue size), then events, each with a global `seq` and
  `t_us` since start: `op-start` (operation id `op`, name from the typed
  input, account, bucket class, key or prefix, allow-listed parameters),
  `attempt-start` (attempt number, the SDK's operation name, request
  `content-length`), `attempt-end` (status or no status, outcome, duration,
  hashed version id), `op-end` (outcome `ok` / `service-error` with the
  provider's error code / `timeout` / `connector` / `response` / `other`,
  last status, attempts, duration), `op-dropped`, `gap`, and `end` on an
  orderly exit (counts and `accounting: complete|incomplete` with
  reasons). An SDK retry is another `attempt-*` pair of the same `op`.
- **Redaction (the only mode):** bucket `configured` or `other`; account
  its id only if it equals `SOTYWARE_REQUEST_RECORD_ACCOUNT`, else
  `other-<n>`; a key or list prefix inside the scope is
  `scope:` + the first 12 bytes (hex) of SHA-256(run 0 `key` 0 the part after
  the scope), the root `root`, a parent of the scope `scope-ancestor`,
  anything else `outside-scope`; upload and version ids are hashed the same
  way (domains `upload-id`, `version-id`). Only typed input fields are read,
  never the URL, so there is no query string to leak.
- **Writer:** each event is built in the hook and handed to one writer
  thread through a bounded queue (4,096 events) with `try_send`; the hook
  never waits and never touches the disk. A full queue drops the event
  (its `seq` becomes a `gap` line), a dead writer counts it
  (`writer_gone`), a write error stops writing (no `end` line). Quitting
  finishes the recorder last in `RunEvent::Exit`, after the download and
  upload shutdowns (so a quit's aborts are recorded), and waits at most 2 s
  for the `end` line; events after it are counted, not written. The module
  forbids `unwrap`, `expect`, indexing and `panic!` (the release profile
  aborts on panic).

What the records mean, and what they do not:

- **`op-dropped`:** the operation's future was dropped before the SDK
  finished it: cancellation, the upload stall timeout, the abort timeout,
  leaving the browser or quitting. `in_flight: true` means an attempt had
  been handed to the HTTP client; whether the provider received it, or even
  completed it, is **unknown**. A dropped operation is finished for the
  accounting; it is not an open one.
- **`gap`:** these sequence numbers were never written (queue full). The
  requests behind them may have been sent.
- **Incomplete recording:** no `end` line (crash, kill, write error, the
  2 s wait ran out), `accounting: incomplete`, a partial last line, a gap,
  or operations still open at the end. Then the count of requests is
  unknown; a missing event never proves that a request was not sent.
- Not observable at all: TCP/TLS connections, connection reuse, bytes on
  the wire, provider-side timing.

## Not built (deliberately)

No overwriting uploads, parallel uploads, upload or download history,
transfer queue UI, ZIP downloads, multipart copies above 5 GiB, permanent
deletion of versions from the app, a trash of sotyware's own, merging
folders on conflict, previews of PDF, SVG or HTML, content search, custom
event or feature; no synchronisation, folder watching, restic
integration, client-side encryption of objects or backups, public ACLs or
bucket permission changes, virtual filesystem, retrying queue, parallel
parts within one file, resumable uploads, credential import from
environment variables or files, automatic provider or bucket discovery,
cost estimator, or a credential fallback outside the OS store. See
[roadmap.md](../status/roadmap.md).

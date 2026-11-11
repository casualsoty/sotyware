# Cloud storage (proof of concept)

How sotyware talks to S3-compatible cloud storage. Status: a tested library
module, **not wired into the running app**. No feature, command or dashboard
element uses it yet, so the release binary behaves exactly as before. Decision
and rationale: [ADR-016](decisions.md#adr-016-cloud-storage-goes-through-one-generic-s3-adapter).
The provider research, prices and the recommendation are in
[cloud-providers.md](cloud-providers.md).

Code: `src-tauri/src/cloud/` (`provider.rs`, `account.rs`, `credentials.rs`,
`s3.rs`) and `SqliteAccountStore` in `src-tauri/src/storage.rs`.

## Pieces

```
AccountConfig (no secret) ──resolve()──▶ ResolvedAccount ──┐
        │                                  ▲               ├─▶ S3Store (one per account)
        │ AccountStore (SQLite)            │ ProviderProfile│     aws-sdk-s3 client + connection pool
        ▼                                  │ (data)         │     transfer queue (semaphore)
   Accounts ── connect(id, CredentialSource) ──────────────┘     SHA-256 end to end
```

| Piece | What it is |
| --- | --- |
| `ProviderProfile` (`provider.rs`) | **Data, not code**: endpoint rule, addressing style, SDK checksum policy, documented transfer limits, capabilities (versioning, Object Lock), known regions, documentation URL. Profiles: `backblaze-b2`, `hetzner`, `s3-compatible` (custom endpoint). |
| `AccountConfig` (`account.rs`) | One set of credentials at one provider: `id` (kebab-case, stable), `label`, `provider`, `region`, optional `endpoint` and `path_style` overrides, the `buckets` it may use, `max_concurrent_transfers`. **No secret.** `resolve()` validates it and computes the endpoint. |
| `Accounts` (`account.rs`) | The set of accounts: add, update, remove (each saved through `AccountStore` first; nothing changes if the save fails), list, and `connect(id, credentials)`, which creates and caches one `S3Store` per account. |
| `AccountStore` trait / `SqliteAccountStore` | `load`, `save`, `remove`. SQLite in `cloud-accounts.sqlite3` (tables `storage_accounts`, `storage_account_buckets`; `ACCOUNT_MIGRATIONS`, `PRAGMA user_version`), using the same open/check/migrate code as the preferences database. Schema has no column for a secret. |
| `CredentialSource` trait (`credentials.rs`) | Looks up an account's key pair **by account id**. Today: `EnvCredentials` (`SOTYWARE_CLOUD_<ID>_ACCESS_KEY_ID`, `…_SECRET_ACCESS_KEY`). `Secret` never prints its value in `Debug`/`Display`. |
| `S3Store` (`s3.rs`) | The one adapter: `list_buckets`, `list_objects` (all pages), `head_object`, `upload_file`, `download_file`, `delete_object`. |

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
  messages. The proof of concept reads environment variables. Planned: the OS
  credential store (Windows Credential Manager, Linux Secret Service), as a
  second `CredentialSource`. The Linux part must handle a desktop without a
  Secret Service, like dashboard-only mode handles a missing tray host.
- **Persistence:** `cloud-accounts.sqlite3`, a separate file, so that this
  unwired proof of concept does not change the released preferences schema.
  Whether it merges into `preferences.sqlite3` is decided when it is wired in.
  A stored account that no longer validates (an unknown provider, an
  out-of-range value) is kept and refused only when used.

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
  read it). sotyware would take them from the same `CredentialSource`.
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

## Not built (deliberately)

No UI, Tauri command, event or feature; no synchronisation, folder watching,
scheduled backups, retention, restic integration, client-side encryption,
sharing or public links, virtual filesystem, retrying queue, parallel parts
within one file, resumable uploads, or keyring storage. See
[roadmap.md](../status/roadmap.md).

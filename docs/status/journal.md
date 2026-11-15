# Production-readiness mission: implementation journal

Started 2026-11-15 on `7af4e3a` (working tree clean, `origin/main` at the same
commit). This journal is the resumable record of the mission "make sotyware
production-ready: storage reliability, disaster recovery, accounts, secure
credential sync and auto-updates". It records the audit, decisions, exact test
results, defects, blockers and the next concrete task. Facts that have a
permanent home elsewhere (architecture, ADRs, status, limitations) are linked,
not repeated; this page is the running log.

Environments named below are kept apart on purpose: **WSL2** (Ubuntu 24.04 on
the development machine, headless), **Linux virtual display** (Xvfb + openbox
under WSL2), **native Windows 11** (the same machine), **hosted CI** (GitHub
Actions), **real provider** and **local mock server** (s3s-fs or
`test_server::Versioned` in the test process). One is never evidence for
another.

## Current milestone

Phase A (audit and foundations) → see [Progress log](#progress-log) for where
work stands.

## Baseline (2026-11-15, `7af4e3a`, WSL2)

`npm run test:headless`: rfd patch guard ok, release-binary guard ok, desktop
isolation guard ok, icons check ok, Vitest **179 passed** (7 files), Rust
**412 passed, 0 failed, 27 ignored**. Exit 0.

## Audit (Phase A, 2026-11-15)

The code was read against the documentation (AGENTS.md, CONTEXT.md, ADR-016
to ADR-028, cloud-storage.md, limitations.md, release.md, testing.md) in
`cloud/s3.rs`, `cloud/s3/upload.rs`, `cloud/upload.rs`, `cloud/download.rs`,
`cloud/operations.rs`, `cloud/provider.rs`, `storage.rs`, `lib.rs`,
`features/`. The existing storage code is careful: one commit point per
upload, re-hashing of sent bytes, HEAD verification, explicit
`NeedsInspection`/`IncompleteParts` outcomes, never-overwrite checks,
plan-then-start file operations, delete markers reported, versions never
deleted. The documentation matches the code in every place checked. The
findings below are the gaps that matter for production use, ordered by
severity.

| # | Area | Finding | Severity | Plan |
| --- | --- | --- | --- | --- |
| A1 | Disaster recovery | There is no backup of any kind: one copy of each object in one bucket of one account. Provider versioning (always on at Backblaze) protects against overwrites and plain deletes but not against account loss, credential compromise (a key with delete rights can remove versions), lifecycle rules, or provider outage. | Critical for the stated goal | Phase C: independent backup engine ([backup.md](../architecture/backup.md), ADR-031). |
| A2 | Crash recovery | All in-flight state is in memory only: multipart upload ids whose abort failed, the remote outcome of an upload interrupted by a crash, half-finished moves/deletes, downloads' `.sotyware-partial` files and the several-item download's staging folder. After a crash or power loss the app forgets them: orphaned multipart parts stay billed and invisible, a move may leave copy + original without explanation, temporary files remain. | High | Phase B: persistent operation journal and a recovery view (ADR-030). |
| A3 | Integrity reporting | A download whose response carried neither `Content-Length` nor sotyware's SHA-256 is reported as `size-only` although no size was compared (`stream_to` only checks the size when the provider announced one). The mission requires telling *transfer completed*, *size verified* and *cryptographically verified* apart. | Medium (misrepresentation) | Phase B: a third verification level `transfer-only`. |
| A4 | Overwrite race | Uploads, copies and folder creation check the destination with `HeadObject` and then write; another client (another device) can create the key in between. No conditional write (`If-None-Match: *`) is sent because provider support is undocumented. On an unversioned bucket the other object is replaced. | Medium (multi-device) | Phase B: capability model with a `conditional_put` capability; send `If-None-Match: *` only where the capability is established; residual risk documented. |
| A5 | Provider assumptions | `ProviderProfile::capabilities` has only `versioning` and `object_lock`. Other assumptions (copy-if-match, delete-marker headers, `ListMultipartUploads`, presigning, metadata preserved on copy, version listing) live in code comments and docs, not in one model with evidence levels. | Medium | Phase B: capability model with evidence (documented / verified at provider / local mock only / assumed / unknown), ADR-029. |
| A6 | Incomplete multipart discovery | Incomplete multipart uploads left by a crash, by an earlier version or by another device cannot be listed or cleaned up from the app. | Medium | Phase B: list them (`ListMultipartUploads`) in the recovery view; abort only on explicit confirmation, never automatically (another device may still be uploading). |
| A7 | Updates | No update mechanism; every release would need a manual reinstall. | Feature gap | Phase F (ADR-033). |
| A8 | Accounts | No sotyware account, no multi-device configuration. | Feature gap | Phases D–E (ADR-032). |
| A9 | Local data permissions | SQLite files are created with the process umask (Linux typically `0644` inside a `0755` folder under `~/.local/share`). They hold no secret (account ids, endpoints, bucket names, favorites), but bucket names and object keys are personal data. Windows `%LOCALAPPDATA%` is per-user by ACL. | Low | Phase G: create the data folder `0700` on Linux; document. |
| A10 | Logging | `eprintln!` lines include object keys (e.g. "cannot abort the incomplete upload of '{key}'"). stderr is not persisted by sotyware and never leaves the machine, but keys are user data. No credential is ever printed (checked: `Secret` has no `Display`, errors never format secrets). | Low | Keep; document in the threat model. No telemetry exists and none is added. |
| A11 | Stale partial files | A killed process leaves its hidden `.sotyware-partial` file (documented limitation). | Low | Covered by A2: the journal records partial files and removes its own on the next start. |
| A12 | Trust in stored SHA-256 | `sotyware-sha256` is user metadata: it proves the bytes match what sotyware (or anyone with write access) declared at upload, not who wrote them. It detects corruption, not a malicious writer. | Informational | Document in the threat model; backups verify against their own manifests. |

Checked and found sound (no change planned): cancellation/commit race
(`TransferControl`), multipart abort in its own task with bounded wait,
ADR-025 settle wait, part re-hashing per attempt, stall timeout replacing
the read timeout for uploads, retry only of eligible files, never-overwrite
on retry, move deletes only after verified copies, restore removes only the
newer delete markers, plan TTL on a monotonic clock (clock changes cannot
extend a plan), places timestamps on the wall clock (ordering only), SQLite
integrity check and refusal of newer schemas, download into a temporary file
with `sync_all` before an atomic rename, disk-full reported as
`LocalWrite { StorageFull }`, pagination tokens kept in Rust and loop
detection on a repeated token, 429/5xx classified as transient with the SDK's
standard retry (3 attempts).

Reliability scenarios the audit traced and their current behaviour:

| Scenario | Today | After this mission |
| --- | --- | --- |
| Crash during single PUT | Nothing stored unless the last chunk left (commit point); nothing remembers the attempt | Journal entry → recovery view checks the key (HEAD + SHA-256) and says stored-and-verified / not stored / other content |
| Crash during multipart | Parts stay billed and invisible | Journal keeps the upload id → recovery view offers abort of that exact id |
| Crash during move | Copies and originals both exist, no record | Journal lists copied-but-not-deleted originals → recovery view explains and offers "keep both" or a reviewed retry |
| Crash during download | Hidden partial file stays | Journal → removed on next start (only files it recorded, only with sotyware's partial-name pattern) |
| Network loss | Transient errors stop the batch/operation, retry offered | unchanged |
| Disk full (download) | `LocalWrite { StorageFull }`, temp file removed | unchanged |
| Corrupt local DB | Refused, defaults used, file kept | unchanged; journal DB follows the same rule |
| Multiple devices | HEAD-then-write race (A4) | conditional writes where established; race documented elsewhere |
| Lost local DB | Accounts lost (credentials stay in the OS store) | sync (Phase E) restores configuration; backups are self-describing (Phase C) |

## Decisions (summary; full rationale in the ADRs)

- **ADR-029 Provider capability model**: every behavioural assumption is a
  capability with an evidence level; code asks the profile, never the
  provider id.
- **ADR-030 Operation journal and recovery**: one SQLite journal
  (`operations-journal.sqlite3`) written before each step that can change
  remote or local state; recovery inspects, never repeats an operation
  automatically; own temporary files are the only thing removed without
  asking.
- **ADR-031 Backup engine**: content-addressed blobs + JSON snapshot
  manifests in a second account/bucket, readable with standard S3 tools;
  verification before health; retention never prunes without a verified
  newer snapshot and never on a suspicious (mass-change) snapshot.
- **ADR-032 sotyware accounts and end-to-end encrypted sync**: separate Rust
  service (`server/`), Argon2id passwords, opaque rotating tokens; vault
  encrypted on the client with a random account key wrapped by a
  high-entropy recovery key and by per-device X25519 keys; the server never
  holds a decryptable key; password alone never unlocks the vault.
- **ADR-033 Updates**: `tauri-plugin-updater` (minisign-signed artifacts)
  plus a separately signed manifest checked by sotyware before the
  updater is used (version floor against downgrade/replay); install only
  on the user's click and only when no transfer or operation is running.

## Progress log

### 2026-11-15

- Phase A: baseline recorded, audit written (above), threat model
  ([threat-model.md](../architecture/threat-model.md)), ADR-029 to ADR-033
  and the design pages [backup.md](../architecture/backup.md),
  [accounts.md](../architecture/accounts.md) and
  [updates.md](../architecture/updates.md) committed as design (docs only).

- Phase B, A3 fixed: downloads now report three levels
  (`sha256-verified`, `size-only`, `transfer-only`); `transfer-only` when
  the provider announced neither size nor SHA-256. Tests:
  `s3::tests::a_download_claims_only_what_was_compared`,
  `download::tests::without_a_size_or_a_checksum_only_the_transfer_is_claimed`,
  the download panel test in `CloudBrowser.test.ts`.

- Phase B, A5 + A4: provider capability model (`Support`/`Evidence`,
  nine capabilities per profile, shown in the account form) and
  conditional writes (`If-None-Match: *` on `PutObject` and
  `CompleteMultipartUpload`) used only where `conditional_put` is
  established — no shipped profile; a 412 is inspected by `HeadObject`
  (own content after a lost answer = success; otherwise `AlreadyExists`,
  nothing stored, multipart aborted). The test server now enforces
  `If-None-Match` like Amazon documents it and records which requests
  carried it.

- Phase B, A2 + A6: operation journal (`journal.rs`,
  `operations-journal.sqlite3`) recording uploads (with multipart id),
  download temporary files, staging folders and file operations before
  they can change anything; startup cleanup of sotyware's own recorded
  temporary files; recovery view with inspect (reads only), dismiss and
  abort of one incomplete multipart upload; bucket-wide listing of
  unfinished uploads. Decision change recorded in ADR-030: an unusable
  journal does not block work (it is reported), a failed write on an open
  journal stops the step. Test drivers now expect four stores.

## Tests run during the mission

(Exact commands and counts, newest last.)

| Date | Commit/tree | Environment | Command | Result |
| --- | --- | --- | --- | --- |
| 2026-11-15 | `7af4e3a` | WSL2 | `npm run test:headless` | Vitest 179 passed; Rust 412 passed, 27 ignored; guards ok |
| 2026-11-15 | A3 change | WSL2 | `cargo test --lib`; `vitest run`; `npm run check`; fmt; clippy `-D warnings` | Rust 414 passed, 27 ignored; Vitest 179; 0 errors/warnings; clippy exit 0 |
| 2026-11-15 | A2/A6 change | WSL2 | `npm run test:headless`; clippy (default and test-profile); recorder tests; `npm run check`; `npm run test:profile-helpers` | guards ok; Vitest 187; Rust 434 passed, 27 ignored; recorder 462 passed, 27 ignored; clippy 0 / 0; check clean; Python helpers 10 OK |
| 2026-11-15 | A4/A5 change | WSL2 | same + `cargo test --features request-recorder --target-dir target/recorder --lib` | Rust 422 passed, 27 ignored; recorder build 450 passed, 27 ignored; Vitest 180; check clean; clippy exit 0 |

## Open defects and blockers

None yet beyond the audit table.

## External gates (cannot be completed autonomously)

- Production update signing key and its safe storage (CI secret) — the
  mechanism uses test keys only.
- Production account service hosting, domain, TLS certificate, SMTP for
  verification and reset mails, server backups storage.
- Code signing for the Windows installers (already an open roadmap item).
- Any real-provider test (Backblaze, Hetzner) needs new explicit
  authorization and an isolated test bucket; earlier approvals do not carry
  over.
- External security review of the account service and the vault design.

## Next concrete task

Phase C: backup engine (`src-tauri/src/backup/`): model (done in
`backup/model.rs`, uncommitted), engine (run, verify, retention, restore),
`backup.sqlite3` store, commands, scheduler feature, Backup page,
disaster-recovery tests, provider-independent recovery script.

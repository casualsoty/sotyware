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

- Phase C done: backup engine (`backup/`), `backup.sqlite3`, nine
  commands, `backup-scheduler` feature, Backups page, recovery guide
  (`docs/user/recovery.md`), stdlib-only recovery tool
  (`scripts/recovery/sotyware-restore.py`) tested against the in-process
  server and added to `ci.yml` (Linux). Bugs found by the new tests and
  fixed before commit: entries took HEAD's second-precision timestamp while
  change detection compares listing timestamps (every object looked
  changed); change counts and the suspicious check were computed before
  content was known (now by SHA-256 after transfer).

- Phase D done: `server/` (axum + SQLite) with the `/v1` API, Argon2id,
  rotating tokens with reuse detection, lockout and rate limits, key proof
  for vault writes and enrollment, pre-registration defence, audit, online
  backup/restore, `secure_delete` (found by the plaintext test: deleted
  rows lingered in free pages), CI job `server`, operations guide
  `docs/development/server.md`.

- Native Windows 11 (headless only, fresh copy): everything passed except
  the Rust unit tests, whose binary did not start (`STATUS_ENTRYPOINT_NOT_FOUND`):
  the Phase F mock-runtime tests link Common Controls v6 code and the test
  binary had no manifest. Fixed with ADR-034 (the linker embeds
  `tauri-build`'s manifest into every Windows binary); then 480 passed, 0
  failed, 28 ignored on Windows. The C: drive filled up once during the
  run (the copy's staticlib build); its build output was deleted.

- Phase G done: security review. `cargo audit` (app: 2 `quick-xml`
  advisories, test-only through `s3s`; unsound `lru`/`glib` and five
  unmaintained crates, none reachable; server: none) and `npm audit`
  (runtime none; dev `deepmerge` low). Fixed: the backup spool's
  recursive cleanup (an unqualified `remove_dir_all` added in phase C, now
  only sotyware's own spool files, no recursion, links never followed);
  the data folder is now `0700` on Linux (A9). Logs reviewed: no secret
  (A10). Threat model gained "another user on the same computer" and the
  dependency review. Self-review of `updates.rs` found the manifest size
  limit applied only after reading the whole answer; now streamed.

- Phase F done: updates (ADR-033): `updates.rs` (signed manifest
  verified with minisign before the plugin sees it, version floor in
  `updates.sqlite3`, platform entry, `restore_update` cross-check, SHA-256
  after the plugin's signature and signed-version check, install gate and
  `release_resources` before the installer ends the process), five
  commands, `update-check` feature, Overview panel,
  `scripts/update-manifest.mjs` (+ fixture signed by the real Tauri CLI
  with a throwaway key), gated signing steps in `release-build.yml`.
  Findings while building it: the plugin's `UpdaterBuilder::build` needs
  an endpoint even for `restore_update` (a reserved `.invalid` URL is
  set); its download builds a reqwest client without a TLS provider (ours
  is passed in); on Windows `install` ends the process itself (cleanup
  moved into `release_resources`); a restart from inside would meet the
  single-instance lock (not done; Linux reports "installed").

- Phase E done: desktop client (`account/`: crypto, vault merge, HTTPS
  client, service, 20 commands), Account page, `OsSecretStore`,
  `account.sqlite3`; end-to-end tests with the service in process. A design
  gap found by the rotation test (multi-request rotation could leave the
  vault, recovery wrap and device wraps under different keys after a
  crash) was fixed with an atomic `/v1/keys/rotate`.

## Tests run during the mission

(Exact commands and counts, newest last.)

| Date | Commit/tree | Environment | Command | Result |
| --- | --- | --- | --- | --- |
| 2026-11-15 | `7af4e3a` | WSL2 | `npm run test:headless` | Vitest 179 passed; Rust 412 passed, 27 ignored; guards ok |
| 2026-11-15 | A3 change | WSL2 | `cargo test --lib`; `vitest run`; `npm run check`; fmt; clippy `-D warnings` | Rust 414 passed, 27 ignored; Vitest 179; 0 errors/warnings; clippy exit 0 |
| 2026-11-15 | `4896f78` + ADR-034 | native Windows 11 (build 26200), copy | `npm ci`; `npm run check`; `npm run test:headless`; `test:update-manifest`; clippy; server `cargo test`; after the fix `cargo test --locked` | check clean; guards ok; Vitest 213; update-manifest 8/8; clippy 0; server 5 + 20; Rust first did not start (0xC0000139), after ADR-034 480 passed, 0 failed, 28 ignored |
| 2026-11-15 | Phase G | WSL2 | `npm run test:headless`; clippy (default, test-profile); recorder tests; `cargo fmt --check`; `cargo audit`; `npm audit`; `cargo build --release --locked` + strings check + release-binary guard | guards ok; Vitest 213; Rust 494 passed, 28 ignored; recorder 522 passed, 28 ignored; clippy 0/0; fmt clean; audits as in the threat model; release binary without overrides, guard ok |
| 2026-11-15 | Phase F | WSL2 | `npm run test:headless`; clippy (default, test-profile); recorder tests; `test:update-manifest`; release-binary and isolation guard self-tests; profile helpers; `npm run check` | guards ok; Vitest 213; Rust 492 passed, 28 ignored (14 update tests; 2 mutants caught); recorder 520 passed, 28 ignored; update-manifest 8/8; guards 39/39 and 9/9; helpers 10 OK; clippy 0/0; check clean |
| 2026-11-15 | Phase E | WSL2 | `npm run test:headless`; clippy (default, test-profile, server); recorder tests; server tests; profile helpers | guards ok; Vitest 205; Rust 478 passed, 28 ignored; recorder 506 passed; server 5 + 20; clippy 0/0/0; helpers OK. Clippy found an ignored 409 in conflict resolution (fixed) |
| 2026-11-15 | Phase D | WSL2 | `cd server && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` | 5 unit + 18 API tests passed; clippy 0 |
| 2026-11-15 | Phase C | WSL2 | `npm run test:headless`; clippy both builds; recorder tests; `--ignored` Python recovery test; profile helpers | guards ok; Vitest 197; Rust 458 passed, 28 ignored; recorder 486 passed, 28 ignored; clippy 0/0; Python recovery test 1 passed; helpers 10 OK |
| 2026-11-15 | A2/A6 change | WSL2 | `npm run test:headless`; clippy (default and test-profile); recorder tests; `npm run check`; `npm run test:profile-helpers` | guards ok; Vitest 187; Rust 434 passed, 27 ignored; recorder 462 passed, 27 ignored; clippy 0 / 0; check clean; Python helpers 10 OK |
| 2026-11-15 | A4/A5 change | WSL2 | same + `cargo test --features request-recorder --target-dir target/recorder --lib` | Rust 422 passed, 27 ignored; recorder build 450 passed, 27 ignored; Vitest 180; check clean; clippy exit 0 |

## Open defects and blockers

None yet beyond the audit table.

## External gates (cannot be completed autonomously)

- Production update signing key and its safe storage (CI secret), and an
  HTTPS update server — the mechanism uses test keys only; release builds
  have updates switched off. Installing a real update (two signed builds,
  disposable Windows 11) has not been done.
- Production account service hosting, domain, TLS certificate, SMTP for
  verification and reset mails, server backups storage.
- Code signing for the Windows installers (already an open roadmap item).
- Any real-provider test (Backblaze, Hetzner) needs new explicit
  authorization and an isolated test bucket; earlier approvals do not carry
  over.
- External security review of the account service and the vault design.

## Next concrete task

Phase G: security review: `cargo audit`/`npm audit` where available, the
data folder's permissions on Linux (audit A9), logging review for secrets,
threat-model status; then Phase H (production-readiness audit and the final
report).

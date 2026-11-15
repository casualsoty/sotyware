# Backups and disaster recovery

Status: design accepted 2026-11-15
([ADR-031](decisions.md#adr-031-backups-are-content-addressed-blobs-plus-plain-json-snapshot-manifests-in-an-independent-destination)).
What is implemented and tested is recorded in
[status.md](../status/status.md) and the [journal](../status/journal.md).
Recovering without sotyware: docs/user/recovery.md (written with the backup engine).

## Why

A single copy of each object in one bucket of one account is not a backup.
Provider versioning (always on at Backblaze, optional at Hetzner) protects
against overwrites and plain deletes, but one compromised key with delete
rights, a lifecycle rule, a closed account or a provider outage still loses
everything. A backup job keeps an **independent recovery copy**: a second
destination (another bucket, ideally another account at another provider)
holding every backed-up object's content and, per run, a manifest of what
the source looked like.

## Repository layout (the backup destination)

Everything lives under the job's destination prefix `P/` (may be empty) in
the destination bucket:

```
P/sotyware-backup.json                 repository descriptor (format, id, created)
P/blobs/sha256/ab/abcdef…(64 hex)      one object per distinct content
P/snapshots/20261115T103000Z-<id>.json one manifest per completed run
```

- **Blobs** are named by the SHA-256 of their content and carry the
  `sotyware-sha256` metadata. A blob is uploaded only if absent and never
  overwritten (`S3Store::upload` refuses an existing key; a blob that already
  exists is accepted only if its size and `sotyware-sha256` match). Identical
  content in several keys, or unchanged between runs, is stored once.
- **Snapshot manifests** are UTF-8 JSON, written once, never modified:

```json
{
  "format": "sotyware-backup-snapshot",
  "version": 1,
  "repository": "<repository id>",
  "snapshot": "<snapshot id>",
  "created": "2026-11-15T10:30:00Z",
  "job": { "id": "<job id>", "name": "Photos" },
  "source": { "provider": "backblaze-b2", "endpoint": "https://s3.eu-central-003.backblazeb2.com",
              "bucket": "my-photos", "prefix": "2026/" },
  "entries": [
    { "key": "2026/a.jpg", "size": 123, "sha256": "…", "etag": "\"…\"",
      "versionId": "…", "lastModified": "2026-11-14T09:00:00Z",
      "contentType": "image/jpeg", "blob": "blobs/sha256/ab/abcdef…" }
  ],
  "stats": { "objects": 1, "bytes": 123, "new": 1, "changed": 0, "deleted": 0,
             "transferred": 1, "transferredBytes": 123 },
  "suspicious": false
}
```

The manifest contains no credential, no local path and no sotyware-internal
id beyond the job id. It is the **recovery manifest**: with it and read access
to the destination, any S3 tool can restore every file (procedure in
docs/user/recovery.md (written with the backup engine)). sotyware can also export it to
a local file chosen in a save dialog.

## A backup run

1. **List the source** recursively under the job's prefix (one
   `ListObjectsV2` per 1,000 keys). Folder markers (keys ending in `/`) are
   skipped.
2. **Load the previous snapshot** of this job (the newest manifest under
   `P/snapshots/` whose job id matches, read from the destination, not a
   local cache).
3. **Plan:** an entry whose key, size, ETag and modification time equal the
   previous snapshot's entry is **unchanged**: its SHA-256 and blob are
   reused without any transfer. Everything else is new or changed.
4. **Change check:** if `(changed + deleted) / previous objects` exceeds the
   job's suspicious threshold (default 30 %, at least 10 objects), the
   snapshot is marked `suspicious`. The run still completes (a new snapshot
   never removes an old one), but retention will not prune, and the health
   says "large change detected".
5. **Transfer** each new or changed object: `GetObject` streamed into a spool
   file in the app's local data folder (SHA-256 computed while streaming;
   verified against the source's `sotyware-sha256` if it has one), then, if
   the blob is absent at the destination, `S3Store::upload` of the spool
   file to the blob key (re-hashed while sent, verified by `HeadObject`). The
   spool file is removed after each object. Errors that stop every further
   request (credentials, access, network, timeouts, 5xx) stop the run; an
   object-level error (object vanished, changed while read) is recorded and
   the run goes on.
6. **Write the manifest** only if every planned object ended in a blob.
   Otherwise the run is `partial`: no manifest is written for it (a manifest
   always describes a complete source state), the blobs already uploaded stay
   and are reused by the next run.
7. **Verify** (quick) the new snapshot right away (below).
8. **Apply retention** if the job has a policy and the snapshot is verified
   and not suspicious.

One run at a time (its own slot beside uploads, downloads and file
operations). Cancel stops after the request in flight; the run is
`cancelled`, nothing is pruned. Quitting cancels within a bounded wait.

## Verification

- **Quick** (after every run, and on request): read the manifest back from
  the destination and parse it; for each distinct blob, `HeadObject`: it
  exists, its size equals the manifest's, its `sotyware-sha256` equals the
  name. Missing or mismatching blobs fail verification with their count.
- **Full** (on request): download every blob of the snapshot and hash it.
  Proves the bytes, not only the metadata.

## Health

A job's health is computed from its local run history and shown on the
Backup page:

| Health | When |
| --- | --- |
| Healthy | the newest snapshot was verified after it was written, no newer run failed, and it is newer than twice the schedule interval |
| Not verified | the newest snapshot has not passed verification |
| Verification failed | the newest verification found missing or damaged blobs |
| Partial | the newest run transferred some objects but did not complete |
| Failed | the newest run failed before transferring anything (with the error kind: destination unavailable, credentials, …) |
| Out of date | no verified snapshot newer than twice the schedule interval |
| Large change detected | the newest snapshot is suspicious |
| Never run | no run yet |

An unverified backup is never shown as healthy.

## Retention

Policy per job: keep the last `N` snapshots, plus the newest snapshot of each
of the last `D` days and `W` weeks (UTC). Pruning, in order, only when the
newest snapshot is verified and not suspicious:

1. List every manifest under `P/snapshots/` (all jobs sharing the
   repository) and read all of them. If any cannot be read or parsed, stop:
   nothing is deleted.
2. Choose the manifests of this job to delete by the policy (never the newest,
   never one younger than the newest verified one).
3. Delete those manifests (`DeleteObject`).
4. Compute the blobs referenced by every **remaining** manifest; delete only
   blobs referenced by none of them and older than the oldest remaining
   manifest's creation time (a blob uploaded by a concurrent run is newer and
   therefore kept).

On a versioned destination a delete adds a delete marker (the data stays
until the provider's lifecycle removes it). With Object Lock retention the
provider refuses deletes of locked versions; the run reports them as
retained. A destination key without delete permission makes pruning report
"not permitted" without affecting backups.

## Restore

From a chosen snapshot, all entries or those under a prefix, to:

- **a local folder** picked in a folder dialog opened by Rust: each blob
  streamed into a staging folder, its SHA-256 checked against the manifest,
  then moved into place under the original relative path; an existing local
  file is never replaced (the restored one gets `name (n)`); key segments
  become safe local names as in several-item downloads;
- **a bucket prefix** of any configured account: each blob downloaded to the
  spool, verified, uploaded with `S3Store::upload` (never overwrites; an
  existing key is skipped and reported).

Every restored file is verified against the manifest's SHA-256 — a restore
is byte-identical or reported as failed.

## Protection characteristics (what to tell the user)

- **Independent copy:** only if the destination is another account (best:
  another provider). Same-account destinations are allowed but labelled
  "same account: does not protect against a compromised key".
- **Delete protection:** recommend a destination key without delete rights
  (Backblaze: an application key without `deleteFiles`), at the cost of no
  pruning.
- **Immutability:** read with `GetObjectLockConfiguration` on the
  destination bucket and shown as found: "Object Lock off", "governance mode,
  default retention N days (can be bypassed by keys with bypass rights)",
  "compliance mode, default retention N days (cannot be shortened, also not
  by you)". Only the last is called immutable.
- **Not encrypted at rest by sotyware:** blobs and manifests are plain
  objects so they stay restorable with standard tools; protect the
  destination bucket (private, dedicated key; provider-side encryption where
  offered).

## Scheduling

The built-in feature `backup` (Features section) runs due jobs: one timer
task that sleeps until the next job is due (no polling), runs it, and
computes the next due time from the job's interval (hours) and the last
run's start. Disabled feature → no scheduled runs (manual runs still work).
While the app is not running, nothing runs; on start, overdue jobs run once.

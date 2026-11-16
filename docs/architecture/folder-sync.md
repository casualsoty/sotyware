# Folder sync

A storage account's bucket kept in step with a folder on this computer, in
both directions, in the background
([ADR-043](decisions.md#adr-043-folder-sync-keeps-a-chosen-folder-and-a-bucket-in-step-by-a-three-way-comparison-with-guarded-deletions-and-one-background-engine-per-sync)).
The user works with the bucket's files in their file manager as ordinary
files: open, copy, paste, rename, delete, create folders. Every file is kept
on the computer (there are no online-only files yet).

Validated only against the in-process test servers and the WSL2 file
system ([status](../status/status.md#folder-sync-2026-11-16)); limits:
[limitations.md](../status/limitations.md#folder-sync-adr-043).

## Terms

| Term | Meaning |
| --- | --- |
| sync | one account's configured bucket and one folder on this computer, kept in step (`SyncConfig`) |
| sync folder | the folder the user chose; any folder on any drive, never a whole drive, the user folder itself, sotyware's data folder, or one inside (or around) another sync's folder |
| synced state | what both sides last agreed on, per path (`synced` table): local size, time and identity, SHA-256, the object's ETag and size |
| local scan / remote listing | the folder and the bucket as last seen (TEMP tables, per session) |
| pass | one round: check the folder, scan, list when due, plan and run |
| sync trash | `.sotyware-sync/trash/<day>/` in the sync folder: files sync removed because they were deleted in the bucket; kept 30 days |
| deletion guard | holds a pass's deletions for the user's answer when they are many (below) |
| conflicted copy | `name (conflicted copy YYYY-MM-DD HHMM).ext`: the bucket's version of a file changed on both sides |

## Components

| Part | File | Role |
| --- | --- | --- |
| Rules | `src-tauri/src/folder_sync/model.rs` | paths ↔ keys, name rules per platform, exclusions, the decision table, conflict names, the guard, retry delays, intervals (pure, unit-tested) |
| Local side | `src-tauri/src/folder_sync/scan.rs` | the walk, stamps, hashes, the marker, `.sotyware-sync` (staging, trash), free space, inspecting a chosen folder |
| Engine | `src-tauri/src/folder_sync/engine.rs` | one sync's task: passes, listing, planning cycles, every action with its live checks |
| Watcher | `src-tauri/src/folder_sync/watch.rs` | `notify` (desktop): names changed paths, asks for a full scan when events may be lost |
| Moving | `src-tauri/src/folder_sync/migrate.rs` | moving a sync to another folder, or starting fresh there |
| Service | `src-tauri/src/folder_sync/mod.rs` | `FolderSyncs` (managed state), the live status, subscriptions, the 16 commands |
| Database | `src-tauri/src/storage/folder_sync.rs` | `folder-sync.sqlite3` and the planning query |
| Feature | `src-tauri/src/features/folder_sync.rs` | `folder-sync`: starts and stops the engines |
| Page | `src/CloudSync.svelte`, `src/folderSync.ts`, `src/folderSyncModel.svelte.ts` | the bucket view's Sync tab; the sidebar's indicator (`src/Sidebar.svelte`) |

## The decision for one path

`model::decide` compares the local scan and the remote listing with the
synced state. A local file is **changed** when its size, modification time
or identity (inode on Linux and macOS) differs; an object when its ETag or
size does. A changed stamp is not yet a changed content: the file is hashed
first, and an equal SHA-256 only updates the recorded stamp (likewise an
object whose stored `sotyware-sha256` is the synced one).

| Synced | Here | In the bucket | Decision |
| --- | --- | --- | --- |
| none | file | none | upload (the key must be free) |
| none | none | object | download (the path must be free) |
| none | file | object | compare: equal content is recorded; else **keep both** |
| yes | same | same | nothing |
| yes | changed | same | upload replacing exactly the synced ETag |
| yes | same | changed | download replacing exactly the synced local file |
| yes | changed | changed | compare: same content on both sides → record; else keep both |
| yes | gone | same | delete in the bucket (guarded) |
| yes | same | gone | move here into the sync trash (guarded) |
| yes | gone | gone | forget |
| yes | gone | changed | download (a change wins over a deletion) |
| yes | changed | gone | upload (a change wins over a deletion) |

Folders: a folder only here gets a marker object `name/` in the bucket if it
is empty (a folder with files arrives with them); a folder only in the
bucket (a marker, or implied by keys below it) is created here; a synced
folder deleted on one side is removed on the other once it is empty (its
files are handled one by one first; only its own marker is deleted). A file
on one side and a folder of the same name on the other is never synced
(reported).

**Moves.** A new file here whose content is a synced file that is gone here
(and unchanged in the bucket) is a move: `CopyObject` (with
`x-amz-copy-source-if-match`) to the new key, then the old key deleted,
nothing uploaded (up to 5 GiB; larger files are uploaded). A new object
whose ETag and size are those of a synced object gone from the bucket
(while the file is unchanged here) is renamed here, nothing downloaded.

## A pass

1. **The folder.** It must exist and hold this sync's marker
   (`.sotyware-sync/root.json` with the sync's marker id). Gone: state
   *unavailable*, look again every 30 s, nothing else (a disconnected drive
   is never a deletion). There but without the marker: state *error*, wait
   for the user (another disk mounted at the same letter, or an unmounted
   mount point, would otherwise look like everything was deleted).
2. **The account.** Its configured bucket and endpoint must be the ones the
   sync was set up with, and its key must be in the credential store.
3. **Local scan.** All of it after a start, a watcher overflow, a returned
   folder or "Sync now"; otherwise only the paths the watcher named (and a
   folder that appeared with them). Links, junctions, devices, sockets and
   pipes are skipped and reported, and nothing inside a link is synced from
   either side: the bucket's keys under a link's name (a folder on another
   computer) stay in the bucket, and a synced folder replaced by a link is
   left as it is on both sides. So are names that cannot be object keys
   (not Unicode, control characters, keys over 1,024 bytes) or that this
   system refuses (Windows: `< > : " \ | ? *`, trailing dots and spaces,
   `CON`, `NUL`, `COM1`, …). A folder that cannot be read is **unknown**:
   nothing below it counts as deleted.
4. **Remote listing,** when due: every page of `ListObjectsV2` (no
   delimiter), recorded under a new generation; only when the last page
   arrived does it replace the previous listing. A listing that fails part
   way is dropped and the sync is *offline* (or *error* for a refused key),
   retried after 30 s, doubling to 15 minutes. Keys inside `.sotyware-sync/`
   and keys with empty segments (`a//b`) are left out; names this system
   refuses are reported.
5. **Planning cycles.** The candidates (paths where the three states do
   not all agree) are read in path order, 500 at a time, decided and run:
   transfers and folders first (three at a time; each transfer still waits
   for the account's transfer queue), then the deletions behind the guard,
   then folder removals, deepest first. Another cycle follows while one
   changed something (at most six per pass).
6. **Status.** Synced when nothing is pending, failed or held; *attention*
   otherwise, with the reason.

**Triggers:** start; watcher events (1.5 s after the last, at most 10 s
after the first); the interval (5, 15, 30 or 60 minutes; 15 by default);
"Sync now"; a failed item's retry time; sotyware's own changes to the
bucket (upload batches, file operations and new folders of the same account
list it at once); every 30 s while the folder is unavailable.

A file modified in the last two seconds may still be written: its upload
waits until it has not changed for two seconds. A file that changes while
it is uploaded fails that upload (`FileChanged`) and is decided again.

## Safety

- **Live checks before every change.** A replacement upload carries the
  ETag it expects (`Destination::Replace`): another object there, or none,
  refuses it before any byte is sent (and, for multipart, again before the
  upload is completed). A deletion in the bucket needs the file still gone
  here and the object still the synced one (`HeadObject`). A move to the
  sync trash needs the object still gone from the bucket and the file still
  the synced one. A download goes to the staging folder and is moved into
  place over exactly the file it expects (or where none is: a hard link,
  which never replaces a file that appeared meanwhile). Whatever is not as
  expected is recorded and decided again.
- **Verified transfers.** Uploads and downloads are `S3Store`'s: SHA-256
  stored with each upload and checked after it, every download checked
  against its size and stored SHA-256 before it is moved into place
  (ADR-020, ADR-021).
- **Nothing is unlinked.** Files deleted in the bucket go to the sync trash
  (30 days; old days are removed when the engine starts). A deleted object
  stays as an earlier version where the bucket keeps versions (restorable in
  Deleted items).
- **The deletion guard.** When a pass would delete at least 1,000 files, or
  at least 50 that are more than half of the synced files, none of those
  deletions runs. The Sync tab asks: **Delete them** (they run; the answer
  covers this many, a larger count asks again) or **Restore them** (their
  synced state is forgotten, so each is copied back to the side it was
  deleted from). Typical causes: an emptied listing (a wrong bucket, a
  provider fault), a folder emptied by another program.
- **Unknown is never gone:** an unreadable folder, an unfinished listing, a
  missing sync folder.
- **Never through a link.** What is behind a link or junction is outside
  the sync folder: the walk and the watcher do not follow one, and every
  look at one path, download, local move, move to the sync trash and
  folder creation or removal first checks that no folder on its way is a
  link (`scan::through_link`).
- **One sync per bucket, no overlapping folders,** and a sync never follows
  its account to another bucket or endpoint (its synced state would not
  fit).

## Conflicts

When both sides changed a file (or both have one that was never synced,
with different content), both versions are kept: the bucket's version is
copied server side to the conflicted copy's key (`CopyObject` with
`x-amz-copy-source-if-match`; above 5 GiB it is downloaded and uploaded),
downloaded under that name here, then this computer's version is uploaded
over the original key, replacing exactly the version that was copied. The
copy's name is recorded first, so an interrupted conflict goes on with the
same name. The Sync tab lists conflicts until the user settles them:

- **Keep this computer's:** the conflicted copy goes to the sync trash here
  and is deleted in the bucket.
- **Keep the other one:** this computer's version goes to the sync trash and
  the conflicted copy takes the original name (then uploaded as a change).
- **Keep both:** the conflict is dismissed.

## Names that differ only in case

Windows and macOS (by default) keep the case of a name but ignore it when
looking one up: `Report.txt` and `report.txt` are one file there and two
keys in a bucket. When its engine starts, and again after the folder was
gone, each sync probes its folder's file system: a file in `.sotyware-sync/`
looked up under another spelling (`probe_case` in `scan.rs`). A FAT or
exFAT drive on Linux ignores case too; ext4 and most Linux file systems do
not, and there nothing below applies.

Where case is ignored:

- **One spelling per name.** Before each planning cycle the bucket's paths
  are compared, folded to lower case (the TEMP scan tables keep each path's
  fold), with the folder's and with each other. A key whose name the folder
  holds under another spelling (`Docs/a.txt` while the folder is `docs`), or
  whose name another key keeps (the synced spelling, else the first in byte
  order), is not planned, nor anything inside it: it stays in the bucket as
  it is and is listed as not synced ("differs only in capitalisation from
  …"). A path the folder holds exactly as listed is never left out (a
  folder Windows marks case-sensitive can hold both). When the name is free
  again, the other spelling is decided in the next cycle: deleting
  `report.txt` here deletes that key, then `Report.txt` comes down.
- **Exact spellings in every check.** A look at one path, and the checks
  before a change, ask for the path as spelled: the canonical path names
  every segment as stored (`GetFinalPathNameByHandleW` on Windows,
  `realpath` on macOS; one call), or, where the probe found that it does
  not (Linux's `realpath` on such a volume), the folder listings do. A file
  found only under another spelling is not the path: it is never recorded
  under it, replaced, moved to the sync trash or renamed, and a deletion in
  the bucket waits while any spelling of the file is here.
- **Renaming only the case** here is a move like any other: the object is
  copied to the new key and the old one deleted (up to 5 GiB; a larger file
  is uploaded again, and the old key stays, listed as not synced).

The fold is Unicode lower case (`str::to_lowercase`): close to, not exactly,
NTFS's upcase table or APFS's rules; Unicode normalisation (NFC and NFD) is
not folded. Where the two differ, a new file that would land on another
spelling of its own name is still refused when it is placed and listed as
not synced; one inside a folder spelled so is not
([limitations](../status/limitations.md#folder-sync-adr-043)).

## The sync folder

**Choosing.** Rust opens the native folder dialog and checks the folder:
not a whole drive, not the user folder itself, not sotyware's data folder,
not inside or around another sync's folder, writable (a probe file is
created and removed). Only then does it look inside (at most 100,000 files
or two seconds): files and size, free space, whether it was a sync folder
before, and the bucket's versioning (`GetBucketVersioning`) and last
measured size. The page shows all of it, with the path, before the user
confirms; the choice is a token kept in Rust for 30 minutes.

**Existing files** are merged: files only here are uploaded, only in the
bucket downloaded, equal ones recorded, different ones kept twice. Nothing
is deleted at the first sync.

**`.sotyware-sync`** (hidden on Windows by its attribute, on Linux and
macOS by its dot) holds `root.json` (the marker: format, sync id, account,
bucket, creation time), `staging/` (downloads being verified; cleared when
the engine starts) and `trash/`. It is never synced.

**Changing the folder.** The user chooses another folder; the page offers
**Move the files** (only into an empty folder: everything in the sync
folder, synced or not, and the sync trash move; on one drive by renaming
each entry, between drives by copying each file, reading it back and
comparing its SHA-256, keeping its modification time; the originals are
removed only when every copy is verified, and if anything fails the copies
are removed and nothing moved) or **Leave them** (the old folder is no
longer synced; the new one is merged from scratch). Choosing the same
folder again after its marker was lost syncs it from scratch.

**Turning sync off** forgets the configuration and the synced state and
removes the marker and the staging folder. Files stay; the sync trash stays
if it holds files; the bucket is not touched. Removing the storage account
turns its sync off the same way (also when the removal arrives through the
encrypted account sync, ADR-032).

## Transfers and requests

| Action | Requests |
| --- | --- |
| remote check | `ListObjectsV2` per 1,000 objects |
| upload | `HeadObject` (free or the expected ETag), `PutObject` or the multipart requests (a second `HeadObject` before completing), `HeadObject` (verification) |
| download | `GetObject` (after `HeadObject` when only the ETag changed) |
| delete in the bucket | `HeadObject`, `DeleteObject` |
| move to the sync trash | `HeadObject` |
| move in the bucket | `HeadObject`, `CopyObject`, `HeadObject`, `DeleteObject` |
| conflict | `HeadObject` per name tried, `CopyObject`, `HeadObject`, `GetObject`, then an upload |
| folders | `PutObject` of a marker / `DeleteObject` of a marker |

Uploads store the file's modification time as `x-amz-meta-sotyware-mtime`
(milliseconds); downloads give the file that time (else the object's
`Last-Modified`), so a file has the same time on every computer. Sync
transfers record themselves in the operation journal like every transfer
(ADR-030): a crash leaves their entries in Interrupted work (an open
multipart upload can be aborted there), and the next pass does the work
again. Free space is checked before each download (the file plus 256 MiB);
a full drive stops downloads for the pass and is shown; uploads go on.

Errors: a refused or unreachable provider stops the pass (offline, retried
with backoff); a refused key or a missing bucket is an error the user must
fix (the tab offers Edit account); everything else is recorded for its path
and retried after 30 s, doubling to an hour ("Try again now" retries at
once).

## Persistence

`folder-sync.sqlite3` in the app's local data folder (schema in
`storage/folder_sync.rs`, append-only migrations):

- `syncs`: each configuration as JSON (`SyncConfig`: account, bucket,
  endpoint, prefix, folder, marker id, paused, interval, versioning, last
  time in step).
- `synced`: the synced state per path.
- `issues`: per path and kind: `failed` (operation, error, attempts, next
  try), `skipped-local`, `skipped-remote`, `skipped-case`, `conflict`
  (copy name, the bucket's version kept, state), `deletions` (the guard's
  question and answer).
- `activity`: the last 500 operations per sync (JSON).
- TEMP `local_scan`, `remote_scan`, `scan_state`: the session's trees
  (each path with its lower-case fold, for [case](#names-that-differ-only-in-case)).

## Status and the Sync tab

States: *starting*, *scanning*, *syncing*, *synced*, *attention*
(something waits: failures, held deletions, a full drive), *paused*,
*offline*, *unavailable* (the folder), *error*, *moving*, *stopped* (the
feature is off). The tab shows the state and its reason, the folder with
**Open folder** and **Change folder…**, the files in step, the bytes on
this computer and free on the drive, the bucket's current files and bytes
from the last listing, the last time in step, what waits to be uploaded and
downloaded, transfers with progress and speed, the guard's question,
conflicts, recent activity, problems (with **Try again now**), what is not
synced and why, the interval, the bucket's versioning, the sync trash
(**Open the sync trash**), **Sync now**, **Pause sync** / **Resume sync**
and **Turn sync off…**. The sidebar marks each synced account and sums every
sync up in one line. The Usage & costs tab shows what sync keeps on this
computer apart from what the bucket stores.

The page gets the status from `watch_folder_syncs` (a `Channel`; Rust sends
at most four overviews a second) and the lists from `folder_sync_details`.

## Commands

`list_folder_syncs`, `watch_folder_syncs`, `unwatch_folder_syncs`,
`choose_sync_folder` (opens the dialog), `discard_sync_folder_choice`,
`enable_folder_sync`, `set_folder_sync_paused`, `sync_folder_now`,
`retry_folder_sync`, `set_folder_sync_interval`, `disable_folder_sync`,
`change_sync_folder`, `open_sync_folder` (the folder or its trash, through
the opener plugin's `open_path`), `folder_sync_details`,
`resolve_folder_sync_conflict`, `answer_folder_sync_deletions`. One
`allow-<command>` permission each; none accepts a path.

## Lifecycle

`run()` loads `FolderSyncs` (before the event loop, no engine yet) and links
it for the feature; the `folder-sync` feature starts an engine for each sync
that is not paused in `setup` and stops them when switched off; quitting
stops them and waits at most 10 s (transfers are cancelled, partial
downloads removed). Pausing stops a sync's engine (its transfers are
cancelled); resuming starts it with a full scan and a listing.

## Testing

Unit tests: `model.rs` (decision table, names, exclusions, guard, retries,
conflict names, dates, the case fold), `scan.rs` (with the exact spellings
and the case probe), `migrate.rs`, `watch.rs`, `storage/folder_sync.rs`
(the planning query, generations, unknown folders, subtrees, spellings that
would share a file). End to end (`folder_sync/tests.rs`, real folders against
the in-process versioned S3 server, s3s-fs for multipart): see
[testing.md](../development/testing.md#folder-sync). Frontend:
`src/CloudSync.test.ts`, `src/App.test.ts` (sidebar).

# Recovering your files from a sotyware backup

This page is for the day something went wrong: your computer is gone,
sotyware is not installed (or not working), the app's database is lost, or
your primary bucket or provider account is unavailable. Your backup does not
depend on sotyware to be read. Design: [backup.md](../architecture/backup.md).

## What you need

1. **Where the backup is:** the provider's S3 endpoint, the region, the
   backup bucket and the backup folder (the job's "Folder for the backup",
   for example `sotyware-backup/`). A recovery file saved from the Backup page
   ("Save recovery file…") contains all of these; keep a copy somewhere other
   than the computer it was made on.
2. **A key that may read the backup bucket.** It is not in the recovery file
   and not in the backup. If you lost it, create a new application key in
   the provider's web console (log in to the provider directly).
3. One of: sotyware on any computer, **Python 3** (any OS), the **AWS CLI**,
   or **rclone**.

## What a backup looks like

Inside the backup folder:

```
sotyware-backup.json                          identifies the backup
snapshots/<job>/<YYYYMMDDTHHMMSSZ>-<id>.json  one per backup run: the list of files
blobs/sha256/<2 characters>/<64 characters>   the files' contents, named by their SHA-256
```

A snapshot file is plain JSON. Each entry has the file's original `key`
(path), `size`, `sha256`, `lastModified`, `contentType` and `blob` (where the
content is, relative to the backup folder). Identical files are stored once.
Older snapshots keep older versions of your files until retention removes
them.

## With sotyware (new computer or reinstall)

1. Install sotyware and add the backup account (and, if it still exists, the
   primary account) under Cloud Storage.
2. Under Backups, create a job with the **same name** and the same backup
   account and folder as before. Its snapshots are found at the
   destination; nothing local is needed.
3. Snapshots… → choose a snapshot → "Restore to a folder…" (or "Restore to
   the bucket" to put files back into an account). Every restored file is
   checked against its SHA-256; nothing that exists is replaced.

## With Python only (no sotyware)

`scripts/recovery/sotyware-restore.py` in the sotyware source is a single
file that uses only Python's standard library. Download it from the
repository (or copy it from this checkout) and run:

```bash
export AWS_ACCESS_KEY_ID=<backup key id>
export AWS_SECRET_ACCESS_KEY=<backup key secret>
python3 sotyware-restore.py --endpoint https://s3.eu-central-003.backblazeb2.com \
  --region eu-central-003 --bucket my-backup --prefix sotyware-backup/ --list
python3 sotyware-restore.py --endpoint https://s3.eu-central-003.backblazeb2.com \
  --region eu-central-003 --bucket my-backup --prefix sotyware-backup/ \
  --latest photos --out ~/restored
```

On Windows PowerShell set the variables with
`$env:AWS_ACCESS_KEY_ID = "..."` and run `py -3 sotyware-restore.py ...`.
Use `--snapshot <key>` for a specific snapshot, `--export <recovery file>`
to use a saved recovery file, and `--only photos/2026/` to restore one
folder. The script verifies every file's SHA-256, never replaces existing
files, and exits with status 1 if anything failed.

## With the AWS CLI or rclone

1. List snapshots:
   `aws s3 ls s3://my-backup/sotyware-backup/snapshots/ --recursive --endpoint-url <endpoint>`
2. Download the snapshot you want and open it in any text editor or JSON
   viewer.
3. For each entry, download `s3://my-backup/sotyware-backup/<blob>` and save
   it under the entry's `key`. Check it with `sha256sum` (Linux),
   `shasum -a 256` (macOS) or `Get-FileHash -Algorithm SHA256` (PowerShell):
   the result must equal the entry's `sha256`.

rclone works the same way with an S3 remote for your provider.

## What this does not cover

- **Lost backup key and no provider access:** only the provider can help;
  the backup is readable only with a valid key.
- **Credentials of your storage accounts** are not in the backup. With a
  sotyware account and sync, they are in your encrypted vault
  ([accounts.md](../architecture/accounts.md)); otherwise create new keys in
  the providers' consoles.
- **Backups are not encrypted by sotyware:** anyone who can read the backup
  bucket can read your files. Keep that bucket private, with its own key.

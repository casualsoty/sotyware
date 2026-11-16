//! The backup engine's remote work (ADR-031): a backup run, verification,
//! retention and restore, each against two [`S3Store`]s (source and
//! destination may be different accounts at different providers).
//!
//! Every object's content goes through a spool file in a folder sotyware
//! owns: streamed from the source by [`S3Store::download`] (hashed while it
//! streams, checked against the source's stored SHA-256 when it has one),
//! then uploaded by [`S3Store::upload`] (re-hashed while sent, verified by
//! `HeadObject`, never overwriting). A blob that already exists is accepted
//! only when its size and `sotyware-sha256` match its name. Nothing here
//! ever deletes a source object; retention deletes only manifests the policy
//! drops and blobs no remaining manifest references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::model::{
    self, DESCRIPTOR, Descriptor, Entry, FORMAT_VERSION, JobConfig, Listed, Manifest,
    REPOSITORY_FORMAT, SNAPSHOT_FORMAT, SnapshotInfo, Stats,
};
use crate::cloud::s3::{Existing, S3Store, StorageError, TransferControl, Verification};
use crate::journal::Journal;

/// Page size of the recursive listings.
const LIST_PAGE: i32 = 1000;
/// Most objects one job may back up (its manifest stays below ~50 MB).
pub const MAX_OBJECTS: usize = 200_000;
/// Most manifests retention reads before it refuses to prune.
pub const MAX_MANIFESTS: usize = 2_000;
/// Unreferenced blobs younger than the oldest remaining manifest minus
/// this margin are kept: another device may be running the same job, its
/// blobs uploaded before its manifest exists (and clocks differ).
pub const PRUNE_MARGIN: Duration = Duration::from_secs(24 * 3600);

/// Cancels a backup task between requests.
#[derive(Debug, Default)]
pub struct Cancel {
    flag: AtomicBool,
    current: std::sync::Mutex<Option<Arc<TransferControl>>>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(c) = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            c.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// A control for the next transfer, cancelled with the task.
    fn control(&self, journal: Option<&Arc<Journal>>) -> Arc<TransferControl> {
        let c = Arc::new(TransferControl::new());
        if let Some(j) = journal {
            c.record_in(Arc::clone(j));
        }
        if self.is_cancelled() {
            c.cancel();
        }
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&c));
        c
    }
}

/// What the engine reports while it works.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// `listing`, `transferring`, `writing`, `verifying`, `pruning`,
    /// `restoring`.
    pub phase: &'static str,
    pub done: u64,
    pub total: u64,
    pub bytes: u64,
}

/// One object that could not be backed up, verified or restored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectError {
    pub key: String,
    pub message: String,
}

/// The shared inputs of every engine task.
pub struct Ctx<'a> {
    pub dest: &'a S3Store,
    pub dest_bucket: &'a str,
    pub dest_prefix: &'a str,
    /// A folder sotyware owns for spool files; emptied by the caller.
    pub spool: &'a Path,
    pub cancel: &'a Cancel,
    pub journal: Option<&'a Arc<Journal>>,
    pub progress: &'a mut (dyn FnMut(Progress) + Send + Sync),
}

/// Whether an error would fail every further request.
fn stops(e: &StorageError) -> bool {
    match e {
        StorageError::InvalidCredentials(_)
        | StorageError::AccessDenied(_)
        | StorageError::Network(_)
        | StorageError::Timeout(_)
        | StorageError::Refused(_)
        | StorageError::Journal(_)
        | StorageError::Cancelled => true,
        StorageError::LocalWrite { kind, .. } => matches!(
            kind,
            std::io::ErrorKind::StorageFull
                | std::io::ErrorKind::QuotaExceeded
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::ReadOnlyFilesystem
        ),
        StorageError::Provider { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

/// Now, RFC 3339 UTC to the second.
pub fn now_rfc3339() -> String {
    aws_sdk_s3::primitives::DateTime::from(SystemTime::now())
        .fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime)
        .map(|s| {
            // Drop fractional seconds: manifests sort and compare as text.
            match s.find('.') {
                Some(i) => format!("{}Z", &s[..i]),
                None => s,
            }
        })
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

/// A fresh identifier (not secret, only unique).
pub fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = Sha256::new();
    h.update(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    h.update(std::process::id().to_le_bytes());
    h.update(N.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    h.finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn spool_file(spool: &Path) -> PathBuf {
    spool.join(format!("object-{}", new_id()))
}

async fn remove(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// Downloads `bucket/key` from `store` into a fresh spool file.
async fn fetch(
    store: &S3Store,
    bucket: &str,
    key: &str,
    ctx: &Ctx<'_>,
) -> Result<(PathBuf, crate::cloud::s3::DownloadReport), StorageError> {
    let path = spool_file(ctx.spool);
    let control = ctx.cancel.control(ctx.journal);
    match store
        .download(
            bucket,
            key,
            &path,
            Existing::Refuse,
            &control,
            &mut |_, _| {},
        )
        .await
    {
        Ok(r) => Ok((path, r)),
        Err(e) => {
            remove(&path).await;
            Err(e)
        }
    }
}

/// Reads and parses a JSON object of the destination.
async fn read_json<T: serde::de::DeserializeOwned>(
    ctx: &Ctx<'_>,
    key: &str,
) -> Result<T, StorageError> {
    let (path, _) = fetch(ctx.dest, ctx.dest_bucket, key, ctx).await?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| StorageError::Io(format!("spool file: {e}")));
    remove(&path).await;
    serde_json::from_slice(&bytes?)
        .map_err(|e| StorageError::Integrity(format!("{key}: not a valid sotyware file: {e}")))
}

/// Writes `value` as JSON to `key` at the destination (never overwriting).
async fn write_json<T: Serialize>(ctx: &Ctx<'_>, key: &str, value: &T) -> Result<(), StorageError> {
    let path = spool_file(ctx.spool);
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| StorageError::Io(format!("cannot write JSON: {e}")))?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| StorageError::LocalWrite {
            kind: e.kind(),
            message: format!("spool file: {e}"),
        })?;
    let control = ctx.cancel.control(ctx.journal);
    let r = ctx
        .dest
        .upload(ctx.dest_bucket, key, &path, &control, &mut |_| {})
        .await;
    remove(&path).await;
    r.map(drop).map_err(|e| e.error)
}

/// The repository's id: read from its descriptor, or a new descriptor
/// written if the destination prefix holds none yet.
pub async fn ensure_repository(ctx: &Ctx<'_>) -> Result<String, StorageError> {
    let key = format!("{}{DESCRIPTOR}", ctx.dest_prefix);
    if ctx
        .dest
        .object_properties(ctx.dest_bucket, &key)
        .await?
        .is_some()
    {
        let d: Descriptor = read_json(ctx, &key).await?;
        if d.format != REPOSITORY_FORMAT || d.version != FORMAT_VERSION {
            return Err(StorageError::Integrity(format!(
                "{key}: not a supported sotyware backup repository ({} version {})",
                d.format, d.version
            )));
        }
        return Ok(d.repository);
    }
    let d = Descriptor {
        format: REPOSITORY_FORMAT.into(),
        version: FORMAT_VERSION,
        repository: new_id(),
        created: now_rfc3339(),
    };
    match write_json(ctx, &key, &d).await {
        Ok(()) => Ok(d.repository),
        // Created meanwhile (another device): use that one.
        Err(StorageError::AlreadyExists(_)) => {
            let d: Descriptor = read_json(ctx, &key).await?;
            Ok(d.repository)
        }
        Err(e) => Err(e),
    }
}

/// Every key under `prefix` at `store`, following pages.
pub async fn list_all(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
    cancel: &Cancel,
    max: usize,
) -> Result<Vec<crate::cloud::s3::ListedObject>, StorageError> {
    let mut out = Vec::new();
    let mut token: Option<String> = None;
    loop {
        if cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        let page = store
            .list_recursive_page(bucket, prefix, token.as_deref(), LIST_PAGE)
            .await?;
        out.extend(page.objects);
        if out.len() > max {
            return Err(StorageError::Refused(format!(
                "more than {max} objects under '{prefix}'"
            )));
        }
        match page.next_token {
            Some(t) => token = Some(t),
            None => return Ok(out),
        }
    }
}

/// One snapshot of a job, as listed at the destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRef {
    pub key: String,
    /// The time encoded in its name, RFC 3339.
    pub created: String,
    pub size: u64,
}

/// `20261115T103000Z-…` → `2026-11-15T10:30:00Z`.
fn created_of(key: &str) -> Option<String> {
    let name = key.rsplit('/').next()?;
    let t = name.get(..16)?;
    let b = t.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    Some(format!(
        "{}-{}-{}T{}:{}:{}Z",
        &t[0..4],
        &t[4..6],
        &t[6..8],
        &t[9..11],
        &t[11..13],
        &t[13..15]
    ))
}

/// A job's snapshots at the destination, oldest first.
pub async fn snapshots(ctx: &Ctx<'_>, job: &str) -> Result<Vec<SnapshotRef>, StorageError> {
    let prefix = model::snapshots_prefix(ctx.dest_prefix, job);
    let mut list: Vec<SnapshotRef> = list_all(
        ctx.dest,
        ctx.dest_bucket,
        &prefix,
        ctx.cancel,
        MAX_MANIFESTS,
    )
    .await?
    .into_iter()
    .filter(|o| o.key.ends_with(".json"))
    .filter_map(|o| {
        Some(SnapshotRef {
            created: created_of(&o.key)?,
            key: o.key,
            size: o.size,
        })
    })
    .collect();
    list.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(list)
}

/// Reads and checks one manifest.
pub async fn read_manifest(ctx: &Ctx<'_>, key: &str) -> Result<Manifest, StorageError> {
    let m: Manifest = read_json(ctx, key).await?;
    m.check()
        .map_err(|e| StorageError::Integrity(format!("{key}: {e}")))?;
    Ok(m)
}

/// How a backup run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunReport {
    pub state: model::RunState,
    /// The manifest written, if the run completed.
    pub snapshot: Option<String>,
    pub stats: Stats,
    pub suspicious: bool,
    pub object_errors: Vec<ObjectError>,
    /// The error that stopped the run, if one did.
    pub error: Option<StorageError>,
}

/// A source: an account's store, its bucket and the job's prefix.
pub struct Source<'a> {
    pub store: &'a S3Store,
    pub bucket: &'a str,
    pub prefix: &'a str,
    pub provider: &'a str,
    pub endpoint: &'a str,
}

/// Runs one backup of `job`: lists the source, reuses unchanged entries of
/// the previous snapshot, transfers new and changed content into blobs, and
/// writes a manifest only if every object ended in a blob.
pub async fn run(ctx: &mut Ctx<'_>, source: &Source<'_>, job: &JobConfig) -> RunReport {
    let mut report = RunReport {
        state: model::RunState::Failed,
        snapshot: None,
        stats: Stats::default(),
        suspicious: false,
        object_errors: Vec::new(),
        error: None,
    };
    let stop = |mut report: RunReport, e: StorageError| {
        report.state = if matches!(e, StorageError::Cancelled) {
            model::RunState::Cancelled
        } else if report.stats.transferred > 0 {
            model::RunState::Partial
        } else {
            model::RunState::Failed
        };
        report.error = Some(e);
        report
    };
    (ctx.progress)(Progress {
        phase: "listing",
        done: 0,
        total: 0,
        bytes: 0,
    });
    let repository = match ensure_repository(ctx).await {
        Ok(r) => r,
        Err(e) => return stop(report, e),
    };
    let previous = match snapshots(ctx, &job.id).await {
        Ok(list) => match list.last() {
            Some(s) => match read_manifest(ctx, &s.key).await {
                Ok(m) => Some(m),
                Err(e) => return stop(report, e),
            },
            None => None,
        },
        Err(e) => return stop(report, e),
    };
    let listed = match list_all(
        source.store,
        source.bucket,
        source.prefix,
        ctx.cancel,
        MAX_OBJECTS,
    )
    .await
    {
        Ok(l) => l,
        Err(e) => return stop(report, e),
    };
    let plan = model::plan(
        listed
            .into_iter()
            .map(|o| Listed {
                key: o.key,
                size: o.size,
                etag: o.e_tag,
                last_modified: o.last_modified,
            })
            .collect(),
        previous.as_ref(),
    );
    let previous_objects = previous.as_ref().map_or(0, |m| m.entries.len());
    // New and changed are counted by content once it is known: a listing
    // without ETags (some services) makes every object a candidate, but
    // only a different SHA-256 is a change.
    let before: BTreeMap<String, String> = previous
        .as_ref()
        .map(|m| {
            m.entries
                .iter()
                .map(|e| (e.key.clone(), e.sha256.clone()))
                .collect()
        })
        .unwrap_or_default();
    report.stats.deleted = plan.deleted;
    let mut entries = plan.unchanged;
    let total = plan.transfer.len() as u64;
    let mut failed = false;
    for (i, l) in plan.transfer.into_iter().enumerate() {
        if ctx.cancel.is_cancelled() {
            return stop(report, StorageError::Cancelled);
        }
        (ctx.progress)(Progress {
            phase: "transferring",
            done: i as u64,
            total,
            bytes: report.stats.transferred_bytes,
        });
        match transfer_one(ctx, source, &l).await {
            Ok(Some((entry, sent))) => {
                if sent {
                    report.stats.transferred += 1;
                    report.stats.transferred_bytes += entry.size;
                }
                match before.get(&entry.key) {
                    None => report.stats.new += 1,
                    Some(sha) if *sha != entry.sha256 => report.stats.changed += 1,
                    Some(_) => {}
                }
                entries.push(entry);
            }
            // Deleted from the source while the backup ran: not part of
            // this snapshot.
            Ok(None) => {
                report.stats.deleted += 1;
            }
            Err(e) if stops(&e) => return stop(report, e),
            Err(e) => {
                failed = true;
                report.object_errors.push(ObjectError {
                    key: l.key.clone(),
                    message: e.to_string(),
                });
            }
        }
    }
    report.suspicious = model::suspicious(
        previous_objects,
        report.stats.changed,
        report.stats.deleted,
        job.suspicious_percent,
    );
    if failed {
        report.state = model::RunState::Partial;
        return report;
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    report.stats.objects = entries.len() as u64;
    report.stats.bytes = entries.iter().map(|e| e.size).sum();
    let created = now_rfc3339();
    let snapshot = new_id();
    let manifest = Manifest {
        format: SNAPSHOT_FORMAT.into(),
        version: FORMAT_VERSION,
        repository,
        snapshot: snapshot.clone(),
        created: created.clone(),
        job: model::JobRef {
            id: job.id.clone(),
            name: job.name.clone(),
        },
        source: model::SourceRef {
            provider: source.provider.into(),
            endpoint: source.endpoint.into(),
            bucket: source.bucket.into(),
            prefix: source.prefix.into(),
        },
        entries,
        stats: report.stats,
        suspicious: report.suspicious,
    };
    (ctx.progress)(Progress {
        phase: "writing",
        done: total,
        total,
        bytes: report.stats.transferred_bytes,
    });
    let key = model::snapshot_key(ctx.dest_prefix, &job.id, &created, &snapshot);
    if let Err(e) = write_json(ctx, &key, &manifest).await {
        return stop(report, e);
    }
    report.state = model::RunState::Completed;
    report.snapshot = Some(key);
    report
}

/// Whether the blob for `sha256` exists at the destination with that
/// content's size and SHA-256 metadata. A blob with the right name but
/// other attributes is an integrity error, never reused or replaced.
async fn blob_ok(ctx: &Ctx<'_>, sha256: &str, size: u64) -> Result<bool, StorageError> {
    let key = model::blob_key(ctx.dest_prefix, sha256);
    match ctx.dest.object_properties(ctx.dest_bucket, &key).await? {
        None => Ok(false),
        Some(p) if p.size == size && p.sha256().is_some_and(|s| s.eq_ignore_ascii_case(sha256)) => {
            Ok(true)
        }
        Some(_) => Err(StorageError::Integrity(format!(
            "{key}: the stored blob does not match its name"
        ))),
    }
}

/// Backs up one object: `Some((entry, transferred))`, or `None` if it no
/// longer exists at the source.
async fn transfer_one(
    ctx: &Ctx<'_>,
    source: &Source<'_>,
    l: &Listed,
) -> Result<Option<(Entry, bool)>, StorageError> {
    let Some(props) = source
        .store
        .object_properties(source.bucket, &l.key)
        .await?
    else {
        return Ok(None);
    };
    let entry = |sha: String, size: u64| Entry {
        key: l.key.clone(),
        size,
        blob: model::blob_key("", &sha),
        sha256: sha,
        // The listing's values: the next run compares its listing with them
        // (a HEAD may give the time with less precision than a listing).
        etag: l.etag.clone().or_else(|| props.e_tag.clone()),
        version_id: props.version_id.clone(),
        last_modified: l
            .last_modified
            .clone()
            .or_else(|| props.last_modified.clone()),
        content_type: props.content_type.clone(),
    };
    // Content sotyware uploaded carries its SHA-256: when that blob is
    // already stored (another key, an earlier run), nothing is transferred.
    if let Some(sha) = props.sha256().map(str::to_ascii_lowercase)
        && sha.len() == 64
        && blob_ok(ctx, &sha, props.size).await?
    {
        return Ok(Some((entry(sha, props.size), false)));
    }
    let (path, report) = match fetch(source.store, source.bucket, &l.key, ctx).await {
        Ok(x) => x,
        Err(StorageError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let result = async {
        if props.sha256().is_some() && report.verification != Verification::Sha256Matched {
            return Err(StorageError::Integrity(format!(
                "{}: the source object was not verified against its stored SHA-256",
                l.key
            )));
        }
        if blob_ok(ctx, &report.sha256, report.size).await? {
            return Ok((entry(report.sha256.clone(), report.size), false));
        }
        let key = model::blob_key(ctx.dest_prefix, &report.sha256);
        let control = ctx.cancel.control(ctx.journal);
        match ctx
            .dest
            .upload(ctx.dest_bucket, &key, &path, &control, &mut |_| {})
            .await
        {
            Ok(_) => Ok((entry(report.sha256.clone(), report.size), true)),
            // Stored meanwhile (another job or device): accepted only if
            // it is exactly this content.
            Err(e) if matches!(e.error, StorageError::AlreadyExists(_)) => {
                if blob_ok(ctx, &report.sha256, report.size).await? {
                    Ok((entry(report.sha256.clone(), report.size), false))
                } else {
                    Err(StorageError::Integrity(format!(
                        "{key}: a different blob exists"
                    )))
                }
            }
            Err(e) => Err(e.error),
        }
    }
    .await;
    remove(&path).await;
    result.map(Some)
}

/// What a verification found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub snapshot: String,
    pub deep: bool,
    pub blobs: u64,
    pub bytes: u64,
    pub missing: Vec<String>,
    pub damaged: Vec<String>,
    pub passed: bool,
}

/// Checks that every blob of the snapshot `key` exists with its size and
/// SHA-256 metadata (`deep`: downloads and hashes each one too).
pub async fn verify(
    ctx: &mut Ctx<'_>,
    key: &str,
    deep: bool,
) -> Result<VerifyReport, StorageError> {
    let manifest = read_manifest(ctx, key).await?;
    let blobs: Vec<(String, u64)> = manifest
        .blobs()
        .into_iter()
        .map(|(b, s)| (b.to_owned(), s))
        .collect();
    let mut report = VerifyReport {
        snapshot: key.to_owned(),
        deep,
        blobs: blobs.len() as u64,
        bytes: blobs.iter().map(|(_, s)| s).sum(),
        ..VerifyReport::default()
    };
    let total = blobs.len() as u64;
    for (i, (blob, size)) in blobs.into_iter().enumerate() {
        if ctx.cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        (ctx.progress)(Progress {
            phase: "verifying",
            done: i as u64,
            total,
            bytes: 0,
        });
        let sha = blob.rsplit('/').next().unwrap_or_default().to_owned();
        let full = format!("{}{blob}", ctx.dest_prefix);
        match blob_ok(ctx, &sha, size).await {
            Ok(true) => {}
            Ok(false) => {
                report.missing.push(blob);
                continue;
            }
            Err(StorageError::Integrity(_)) => {
                report.damaged.push(blob);
                continue;
            }
            Err(e) => return Err(e),
        }
        if deep {
            match fetch(ctx.dest, ctx.dest_bucket, &full, ctx).await {
                Ok((path, r)) => {
                    remove(&path).await;
                    if r.sha256 != sha || r.size != size {
                        report.damaged.push(blob);
                    }
                }
                Err(StorageError::Integrity(_)) => report.damaged.push(blob),
                Err(StorageError::NotFound(_)) => report.missing.push(blob),
                Err(e) => return Err(e),
            }
        }
    }
    report.passed = report.missing.is_empty() && report.damaged.is_empty();
    Ok(report)
}

/// What retention did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    pub snapshots_kept: u64,
    pub snapshots_deleted: u64,
    pub blobs_deleted: u64,
    pub bytes_deleted: u64,
    /// Deletes the provider refused (no permission, Object Lock).
    pub refused: u64,
}

/// Applies `job`'s retention policy: deletes the manifests it drops, then
/// the blobs that no remaining manifest of any job references and that are
/// older than the oldest remaining manifest minus [`PRUNE_MARGIN`]. Reads
/// every manifest first; if any cannot be read, nothing is deleted.
pub async fn prune(ctx: &mut Ctx<'_>, job: &JobConfig) -> Result<PruneReport, StorageError> {
    prune_with_margin(ctx, job, PRUNE_MARGIN).await
}

/// [`prune`] with another safety margin (tests).
pub async fn prune_with_margin(
    ctx: &mut Ctx<'_>,
    job: &JobConfig,
    margin: Duration,
) -> Result<PruneReport, StorageError> {
    if job.retention.keeps_everything() {
        return Ok(PruneReport::default());
    }
    (ctx.progress)(Progress {
        phase: "pruning",
        done: 0,
        total: 0,
        bytes: 0,
    });
    delete_chosen(ctx, &job.id, margin, |mine| {
        let keep = model::keep(mine, job.retention);
        Ok(mine
            .iter()
            .filter(|s| !keep.contains(&s.key))
            .map(|s| s.key.clone())
            .collect())
    })
    .await
}

/// Deletes one snapshot of `job` that the user chose (ADR-041): its
/// manifest, then the blobs no remaining manifest references, by the same
/// rules and with the same safety margin as retention. Reads every manifest
/// first; nothing is deleted if one cannot be read, or if `key` is not a
/// snapshot of `job`.
pub async fn delete_snapshot(
    ctx: &mut Ctx<'_>,
    job: &JobConfig,
    key: &str,
) -> Result<PruneReport, StorageError> {
    delete_snapshot_with_margin(ctx, job, key, PRUNE_MARGIN).await
}

/// [`delete_snapshot`] with another safety margin (tests).
pub async fn delete_snapshot_with_margin(
    ctx: &mut Ctx<'_>,
    job: &JobConfig,
    key: &str,
    margin: Duration,
) -> Result<PruneReport, StorageError> {
    (ctx.progress)(Progress {
        phase: "deleting",
        done: 0,
        total: 0,
        bytes: 0,
    });
    delete_chosen(ctx, &job.id, margin, |mine| {
        if mine.iter().any(|s| s.key == key) {
            Ok(BTreeSet::from([key.to_owned()]))
        } else {
            Err(StorageError::NotFound(format!(
                "{key} is not a snapshot of this backup (it may have been deleted already)"
            )))
        }
    })
    .await
}

/// The deletion shared by retention and a user's delete: reads every
/// manifest under `P/snapshots/` (all jobs of the repository; stops if any
/// cannot be read), deletes the manifests of `job_id` that `choose` picks,
/// then deletes the blobs that no remaining manifest references and that
/// were last modified more than `margin` before the oldest remaining
/// manifest was created (a blob uploaded by a run on another device has no
/// manifest yet, and clocks differ).
async fn delete_chosen(
    ctx: &mut Ctx<'_>,
    job_id: &str,
    margin: Duration,
    choose: impl FnOnce(&[SnapshotInfo]) -> Result<BTreeSet<String>, StorageError>,
) -> Result<PruneReport, StorageError> {
    let mut report = PruneReport::default();
    let all_prefix = format!("{}snapshots/", ctx.dest_prefix);
    let listed = list_all(
        ctx.dest,
        ctx.dest_bucket,
        &all_prefix,
        ctx.cancel,
        MAX_MANIFESTS,
    )
    .await?;
    let mut manifests: BTreeMap<String, Manifest> = BTreeMap::new();
    for o in listed.iter().filter(|o| o.key.ends_with(".json")) {
        if ctx.cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        manifests.insert(o.key.clone(), read_manifest(ctx, &o.key).await?);
    }
    let mine: Vec<SnapshotInfo> = manifests
        .iter()
        .filter(|(_, m)| m.job.id == job_id)
        .map(|(k, m)| SnapshotInfo {
            key: k.clone(),
            created: m.created.clone(),
        })
        .collect();
    let chosen = choose(&mine)?;
    for s in mine.iter().filter(|s| chosen.contains(&s.key)) {
        if ctx.cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        match ctx
            .dest
            .delete_reporting(ctx.dest_bucket, &s.key, None)
            .await
        {
            Ok(_) => {
                report.snapshots_deleted += 1;
                manifests.remove(&s.key);
            }
            Err(StorageError::AccessDenied(_)) => report.refused += 1,
            Err(e) => return Err(e),
        }
    }
    report.snapshots_kept = manifests.values().filter(|m| m.job.id == job_id).count() as u64;
    let referenced: BTreeSet<String> = manifests
        .values()
        .flat_map(|m| {
            m.entries
                .iter()
                .map(|e| format!("{}{}", ctx.dest_prefix, e.blob))
        })
        .collect();
    // Only blobs clearly older than every remaining snapshot may go.
    let Some(oldest) = manifests.values().map(|m| m.created.clone()).min() else {
        return Ok(report);
    };
    let cutoff = shift_rfc3339(
        &oldest,
        -i64::try_from(margin.as_secs()).unwrap_or(i64::MAX),
    );
    let blobs = list_all(
        ctx.dest,
        ctx.dest_bucket,
        &format!("{}blobs/", ctx.dest_prefix),
        ctx.cancel,
        MAX_OBJECTS * 2,
    )
    .await?;
    for b in blobs {
        if ctx.cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        let old_enough = b
            .last_modified
            .as_deref()
            .is_some_and(|t| cutoff.as_deref().is_some_and(|c| t < c));
        if referenced.contains(&b.key) || !old_enough {
            continue;
        }
        match ctx
            .dest
            .delete_reporting(ctx.dest_bucket, &b.key, None)
            .await
        {
            Ok(_) => {
                report.blobs_deleted += 1;
                report.bytes_deleted += b.size;
            }
            Err(StorageError::AccessDenied(_)) => report.refused += 1,
            Err(e) => return Err(e),
        }
    }
    Ok(report)
}

/// `t` (RFC 3339 UTC, to the second) moved by `secs`.
pub fn shift_rfc3339(t: &str, secs: i64) -> Option<String> {
    let days = model::days_from_civil(t)?;
    let h: i64 = t.get(11..13)?.parse().ok()?;
    let m: i64 = t.get(14..16)?.parse().ok()?;
    let s: i64 = t.get(17..19)?.parse().ok()?;
    let total = days * 86_400 + h * 3600 + m * 60 + s + secs;
    let secs = u64::try_from(total).ok()?;
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    aws_sdk_s3::primitives::DateTime::from(when)
        .fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime)
        .ok()
}

/// Where a restore writes.
pub enum Target<'a> {
    /// A local folder: files under their key (relative to the snapshot's
    /// source prefix), never replacing an existing file.
    Local(&'a Path),
    /// A bucket prefix of some account: never overwriting an object.
    Bucket {
        store: &'a S3Store,
        bucket: &'a str,
        prefix: &'a str,
    },
}

/// What a restore did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored: u64,
    pub bytes: u64,
    /// Bucket targets: keys that already existed and were left alone.
    pub skipped: Vec<String>,
    /// Local targets: files saved under another name because the name was
    /// taken (key → local relative path).
    pub renamed: Vec<(String, String)>,
    pub failed: Vec<ObjectError>,
}

/// The entries of `manifest` under `filter` (a key prefix; empty = all).
pub fn selected<'m>(manifest: &'m Manifest, filter: &str) -> Vec<&'m Entry> {
    manifest
        .entries
        .iter()
        .filter(|e| e.key.starts_with(filter))
        .collect()
}

/// Restores the entries of `manifest` under `filter` to `target`. Every
/// restored file is checked against the manifest's SHA-256; a mismatch
/// removes what was written for it and reports it failed.
pub async fn restore(
    ctx: &mut Ctx<'_>,
    manifest: &Manifest,
    filter: &str,
    target: &Target<'_>,
) -> Result<RestoreReport, StorageError> {
    let entries = selected(manifest, filter);
    let total = entries.len() as u64;
    let mut report = RestoreReport::default();
    for (i, e) in entries.into_iter().enumerate() {
        if ctx.cancel.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        (ctx.progress)(Progress {
            phase: "restoring",
            done: i as u64,
            total,
            bytes: report.bytes,
        });
        let rel = e
            .key
            .strip_prefix(&manifest.source.prefix)
            .unwrap_or(&e.key)
            .to_owned();
        let blob = format!("{}{}", ctx.dest_prefix, e.blob);
        let outcome = match target {
            Target::Local(dir) => restore_local(ctx, e, &blob, dir, &rel).await,
            Target::Bucket {
                store,
                bucket,
                prefix,
            } => restore_bucket(ctx, e, &blob, store, bucket, &format!("{prefix}{rel}")).await,
        };
        match outcome {
            Ok(Placed::Restored) => {
                report.restored += 1;
                report.bytes += e.size;
            }
            Ok(Placed::Renamed(to)) => {
                report.restored += 1;
                report.bytes += e.size;
                report.renamed.push((e.key.clone(), to));
            }
            Ok(Placed::Skipped) => report.skipped.push(e.key.clone()),
            Err(err) if stops(&err) => return Err(err),
            Err(err) => report.failed.push(ObjectError {
                key: e.key.clone(),
                message: err.to_string(),
            }),
        }
    }
    Ok(report)
}

enum Placed {
    Restored,
    Renamed(String),
    Skipped,
}

async fn restore_local(
    ctx: &Ctx<'_>,
    e: &Entry,
    blob: &str,
    dir: &Path,
    rel: &str,
) -> Result<Placed, StorageError> {
    if rel.is_empty() || rel.ends_with('/') {
        return Ok(Placed::Skipped);
    }
    let segments: Vec<String> = rel
        .split('/')
        .map(crate::cloud::bulk_download::local_segment)
        .collect();
    let (name, parents) = segments.split_last().unwrap_or((&segments[0], &[]));
    let folder = parents.iter().fold(dir.to_path_buf(), |p, s| p.join(s));
    tokio::fs::create_dir_all(&folder)
        .await
        .map_err(|err| StorageError::LocalWrite {
            kind: err.kind(),
            message: format!("cannot create a folder: {err}"),
        })?;
    // The first free name: never replace a file that is there.
    let mut chosen = name.clone();
    let mut n = 1;
    while tokio::fs::try_exists(folder.join(&chosen))
        .await
        .unwrap_or(true)
    {
        n += 1;
        if n > 1000 {
            return Err(StorageError::LocalWrite {
                kind: std::io::ErrorKind::AlreadyExists,
                message: "no free file name".into(),
            });
        }
        chosen = crate::cloud::operations::numbered_name(name, n - 1);
    }
    let path = folder.join(&chosen);
    let control = ctx.cancel.control(ctx.journal);
    let r = ctx
        .dest
        .download(
            ctx.dest_bucket,
            blob,
            &path,
            Existing::Refuse,
            &control,
            &mut |_, _| {},
        )
        .await?;
    if r.sha256 != e.sha256 || r.size != e.size {
        // Only the file this restore just created.
        remove(&path).await;
        return Err(StorageError::Integrity(format!(
            "{}: the restored content does not match the snapshot",
            e.key
        )));
    }
    if chosen == *name {
        Ok(Placed::Restored)
    } else {
        let mut shown: Vec<&str> = parents.iter().map(String::as_str).collect();
        shown.push(&chosen);
        Ok(Placed::Renamed(shown.join("/")))
    }
}

async fn restore_bucket(
    ctx: &Ctx<'_>,
    e: &Entry,
    blob: &str,
    store: &S3Store,
    bucket: &str,
    key: &str,
) -> Result<Placed, StorageError> {
    if store.object_properties(bucket, key).await?.is_some() {
        return Ok(Placed::Skipped);
    }
    let (path, r) = fetch(ctx.dest, ctx.dest_bucket, blob, ctx).await?;
    let result = async {
        if r.sha256 != e.sha256 || r.size != e.size {
            return Err(StorageError::Integrity(format!(
                "{}: the backed-up content does not match the snapshot",
                e.key
            )));
        }
        let control = ctx.cancel.control(ctx.journal);
        match store
            .upload(bucket, key, &path, &control, &mut |_| {})
            .await
        {
            Ok(_) => Ok(Placed::Restored),
            Err(err) if matches!(err.error, StorageError::AlreadyExists(_)) => Ok(Placed::Skipped),
            Err(err) => Err(err.error),
        }
    }
    .await;
    remove(&path).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_times_round_trip_through_their_names() {
        assert_eq!(
            created_of("p/snapshots/j/20261115T103000Z-abc.json").as_deref(),
            Some("2026-11-15T10:30:00Z")
        );
        assert_eq!(created_of("p/snapshots/j/garbage.json"), None);
        assert_eq!(
            shift_rfc3339("2026-11-15T10:30:00Z", -86_400).as_deref(),
            Some("2026-11-14T10:30:00Z")
        );
        let now = now_rfc3339();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.ends_with('Z'));
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn only_errors_that_would_repeat_stop_a_run() {
        assert!(stops(&StorageError::Network("x".into())));
        assert!(stops(&StorageError::AccessDenied("x".into())));
        assert!(stops(&StorageError::Cancelled));
        assert!(!stops(&StorageError::Integrity("x".into())));
        assert!(!stops(&StorageError::NotFound("x".into())));
        assert!(stops(&StorageError::Provider {
            status: 503,
            code: String::new(),
            message: String::new()
        }));
    }
}

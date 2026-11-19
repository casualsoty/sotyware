//! The backup engine's data and pure rules (ADR-031): job configuration and
//! its validation, the repository's object names, the snapshot manifest
//! format, the change check that marks a snapshot suspicious, retention
//! selection and health. No I/O here, so every rule is unit-tested.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The manifest's `format` value.
pub const SNAPSHOT_FORMAT: &str = "sotyware-backup-snapshot";
/// The repository descriptor's `format` value.
pub const REPOSITORY_FORMAT: &str = "sotyware-backup-repository";
/// The only manifest and descriptor version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// The repository descriptor's name under the destination prefix.
pub const DESCRIPTOR: &str = "sotyware-backup.json";
/// Default share of changed or deleted objects that marks a snapshot
/// suspicious (the page proposes it; tests use it).
#[cfg_attr(not(test), allow(dead_code))]
pub const DEFAULT_SUSPICIOUS_PERCENT: u8 = 30;
/// Below this many objects in the previous snapshot, no snapshot is ever
/// suspicious (a handful of edits would otherwise trip it).
pub const SUSPICIOUS_MIN_OBJECTS: usize = 10;

/// Where a backup reads from or writes to: an account's configured bucket
/// and a key prefix (empty, or ending in `/`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub account: String,
    pub bucket: String,
    pub prefix: String,
}

/// Which snapshots retention keeps (all counts 0 = keep everything).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Retention {
    pub keep_last: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
}

impl Retention {
    pub fn keeps_everything(&self) -> bool {
        self.keep_last == 0 && self.keep_daily == 0 && self.keep_weekly == 0
    }
}

/// One backup job, as configured by the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobConfig {
    /// Lowercase kebab-case, at most 64 characters (a key component).
    pub id: String,
    pub name: String,
    pub source: Location,
    pub destination: Location,
    /// 0 = manual only.
    pub interval_hours: u32,
    pub retention: Retention,
    /// Share of changed or deleted objects (in percent) that marks a
    /// snapshot suspicious.
    pub suspicious_percent: u8,
}

/// Why a job configuration cannot be saved.
pub fn validate(job: &JobConfig) -> Result<(), String> {
    if !crate::cloud::account::is_kebab_case(&job.id) {
        return Err("the job id must be lowercase letters, digits and dashes".into());
    }
    let name = job.name.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err("the name must be 1 to 100 characters, without control characters".into());
    }
    for (what, l) in [("source", &job.source), ("destination", &job.destination)] {
        if l.account.is_empty() || l.bucket.is_empty() {
            return Err(format!("the {what} needs an account and a bucket"));
        }
        if !(l.prefix.is_empty() || l.prefix.ends_with('/')) || l.prefix.len() > 512 {
            return Err(format!(
                "the {what} folder must be empty or end in '/', at most 512 bytes"
            ));
        }
        if l.prefix.starts_with('/') || l.prefix.split('/').any(|s| s == "..") {
            return Err(format!("the {what} folder is not a valid key prefix"));
        }
    }
    let (s, d) = (&job.source, &job.destination);
    // A destination inside the source would back itself up; a source inside
    // the destination would back up the backup.
    if s.account == d.account
        && s.bucket == d.bucket
        && (d.prefix.starts_with(&s.prefix) || s.prefix.starts_with(&d.prefix))
    {
        return Err(
            "the destination must not be inside the source, nor the source inside the destination"
                .into(),
        );
    }
    if job.interval_hours > 24 * 31 {
        return Err("the interval must be at most 31 days".into());
    }
    if !(1..=100).contains(&job.suspicious_percent) {
        return Err("the change threshold must be 1 to 100 percent".into());
    }
    let r = job.retention;
    if r.keep_last > 10_000 || r.keep_daily > 10_000 || r.keep_weekly > 10_000 {
        return Err("retention counts must be at most 10,000".into());
    }
    Ok(())
}

/// Whether source and destination are in the same account: such a backup
/// does not survive a compromised key or a closed account.
pub fn same_account(job: &JobConfig) -> bool {
    job.source.account == job.destination.account
}

/// The blob key of `sha256` under the destination prefix.
pub fn blob_key(prefix: &str, sha256: &str) -> String {
    let head = sha256.get(..2).unwrap_or("00");
    format!("{prefix}blobs/sha256/{head}/{sha256}")
}

/// The folder of one job's manifests.
pub fn snapshots_prefix(prefix: &str, job: &str) -> String {
    format!("{prefix}snapshots/{job}/")
}

/// A manifest's key: the time first, so key order is creation order (a new
/// snapshot is never dated before the newest one, `engine::snapshot_time`).
pub fn snapshot_key(prefix: &str, job: &str, created: &str, id: &str) -> String {
    format!(
        "{}{}-{id}.json",
        snapshots_prefix(prefix, job),
        compact_time(created)
    )
}

/// `2026-11-15T10:30:00Z` → `20261115T103000Z`.
pub fn compact_time(rfc3339: &str) -> String {
    rfc3339
        .chars()
        .filter(|c| !matches!(c, '-' | ':'))
        .take_while(|c| *c != '.')
        .collect::<String>()
        .trim_end_matches('Z')
        .to_owned()
        + "Z"
}

/// The repository descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub format: String,
    pub version: u32,
    pub repository: String,
    pub created: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
}

/// One object of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub key: String,
    pub size: u64,
    pub sha256: String,
    pub etag: Option<String>,
    pub version_id: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    /// The blob's key relative to the destination prefix.
    pub blob: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub objects: u64,
    pub bytes: u64,
    pub new: u64,
    pub changed: u64,
    pub deleted: u64,
    pub transferred: u64,
    pub transferred_bytes: u64,
}

/// A snapshot manifest: what the source held at one run, and where each
/// object's content is. Contains no credential and no local path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub repository: String,
    pub snapshot: String,
    pub created: String,
    pub job: JobRef,
    pub source: SourceRef,
    pub entries: Vec<Entry>,
    pub stats: Stats,
    pub suspicious: bool,
}

impl Manifest {
    /// Refuses a manifest of another format or a newer version.
    pub fn check(&self) -> Result<(), String> {
        if self.format != SNAPSHOT_FORMAT {
            return Err(format!(
                "not a sotyware snapshot manifest ({})",
                self.format
            ));
        }
        if self.version != FORMAT_VERSION {
            return Err(format!(
                "snapshot format version {} is not supported (this build reads {FORMAT_VERSION})",
                self.version
            ));
        }
        for e in &self.entries {
            if e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("entry '{}' has no valid SHA-256", e.key));
            }
            if e.blob != blob_key("", &e.sha256) {
                return Err(format!("entry '{}' names an unexpected blob", e.key));
            }
        }
        Ok(())
    }

    /// The distinct blobs it references, with their size.
    pub fn blobs(&self) -> BTreeMap<&str, u64> {
        self.entries
            .iter()
            .map(|e| (e.blob.as_str(), e.size))
            .collect()
    }
}

/// An object as the source listing shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub key: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// What a run must do with each listed object.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Unchanged since the previous snapshot: its entry is reused as is.
    pub unchanged: Vec<Entry>,
    /// New or changed: must be read (or found by its stored SHA-256).
    pub transfer: Vec<Listed>,
    pub new: u64,
    pub changed: u64,
    /// In the previous snapshot but not listed now.
    pub deleted: u64,
}

/// Sorts the listing against the previous snapshot: same key, size, ETag
/// and modification time → unchanged. Folder markers (keys ending in `/`)
/// are skipped.
pub fn plan(listed: Vec<Listed>, previous: Option<&Manifest>) -> Plan {
    let before: BTreeMap<&str, &Entry> = previous
        .map(|m| m.entries.iter().map(|e| (e.key.as_str(), e)).collect())
        .unwrap_or_default();
    let mut plan = Plan::default();
    let mut seen = BTreeSet::new();
    for l in listed {
        if l.key.ends_with('/') {
            continue;
        }
        seen.insert(l.key.clone());
        match before.get(l.key.as_str()) {
            Some(e)
                if e.size == l.size
                    && e.etag.is_some()
                    && e.etag == l.etag
                    && e.last_modified == l.last_modified =>
            {
                plan.unchanged.push((*e).clone());
            }
            Some(_) => {
                plan.changed += 1;
                plan.transfer.push(l);
            }
            None => {
                plan.new += 1;
                plan.transfer.push(l);
            }
        }
    }
    plan.deleted = before.keys().filter(|k| !seen.contains(**k)).count() as u64;
    plan
}

/// Whether a snapshot with these changes against `previous_objects` is
/// suspicious (a possible mass deletion or modification).
pub fn suspicious(previous_objects: usize, changed: u64, deleted: u64, percent: u8) -> bool {
    if previous_objects < SUSPICIOUS_MIN_OBJECTS {
        return false;
    }
    (changed + deleted) * 100 > previous_objects as u64 * u64::from(percent)
}

/// One snapshot as retention sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub key: String,
    /// RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`), which sorts as text.
    pub created: String,
}

/// The snapshots retention keeps, by key. Always keeps the newest; keeps
/// everything when the policy keeps everything.
pub fn keep(snapshots: &[SnapshotInfo], policy: Retention) -> BTreeSet<String> {
    let mut sorted: Vec<&SnapshotInfo> = snapshots.iter().collect();
    sorted.sort_by(|a, b| b.created.cmp(&a.created).then(b.key.cmp(&a.key)));
    if policy.keeps_everything() {
        return sorted.iter().map(|s| s.key.clone()).collect();
    }
    let mut kept = BTreeSet::new();
    if let Some(newest) = sorted.first() {
        kept.insert(newest.key.clone());
    }
    for s in sorted.iter().take(policy.keep_last as usize) {
        kept.insert(s.key.clone());
    }
    // Newest snapshot of each of the last N distinct days / ISO weeks.
    let mut bucketed = |n: u32, bucket: &dyn Fn(&str) -> Option<String>| {
        let mut seen = BTreeSet::new();
        for s in &sorted {
            if seen.len() >= n as usize {
                break;
            }
            if let Some(b) = bucket(&s.created)
                && seen.insert(b)
            {
                kept.insert(s.key.clone());
            }
        }
    };
    bucketed(policy.keep_daily, &|c| c.get(..10).map(str::to_owned));
    bucketed(policy.keep_weekly, &|c| week_of(c).map(|w| w.to_string()));
    kept
}

/// Days since 1970-01-01 of a `YYYY-MM-DD…` date (proleptic Gregorian).
pub fn days_from_civil(date: &str) -> Option<i64> {
    let y: i64 = date.get(0..4)?.parse().ok()?;
    let m: i64 = date.get(5..7)?.parse().ok()?;
    let d: i64 = date.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// A week number (weeks since the Monday of 1969-12-29).
fn week_of(created: &str) -> Option<i64> {
    days_from_civil(created).map(|d| (d + 3).div_euclid(7))
}

/// How a job stands, from its local history. Never `Healthy` without a
/// passed verification recorded after the newest snapshot's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    Healthy,
    NeverRun,
    NotVerified,
    VerificationFailed,
    Partial,
    Failed,
    OutOfDate,
    LargeChange,
}

/// What the health computation needs from the newest run and verification
/// recorded on this computer. Which came first is the order of the records,
/// never the wall-clock times in them, which can step back (ADR-051).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// The newest run: its state and when it started (ms since the epoch).
    pub last_run: Option<(RunState, i64)>,
    /// The newest completed snapshot.
    pub last_snapshot: Option<SnapshotRecord>,
    /// The newest verification of that snapshot.
    pub last_verification: Option<VerificationRecord>,
}

/// A completed snapshot as health needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRecord {
    /// When its run ended (ms since the epoch), for "out of date".
    pub written_ms: i64,
    pub suspicious: bool,
    /// Its run's place in the history (later records have larger values).
    pub seq: i64,
}

/// A verification as health needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerificationRecord {
    /// Its place in the history (later records have larger values).
    pub seq: i64,
    pub passed: bool,
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "completed" => Self::Completed,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// The job's health at `now_ms`. `interval_hours` 0 (manual) never makes a
/// backup out of date.
pub fn health(history: &History, interval_hours: u32, now_ms: i64) -> Health {
    let Some((run_state, _)) = history.last_run else {
        return Health::NeverRun;
    };
    match run_state {
        RunState::Failed => return Health::Failed,
        RunState::Partial => return Health::Partial,
        RunState::Completed | RunState::Cancelled => {}
    }
    let Some(snapshot) = history.last_snapshot else {
        // A cancelled first run: nothing to restore yet.
        return Health::NeverRun;
    };
    // Only a verification recorded after the snapshot's run counts.
    match history.last_verification {
        Some(v) if v.seq > snapshot.seq && v.passed => {}
        Some(v) if v.seq > snapshot.seq => return Health::VerificationFailed,
        _ => return Health::NotVerified,
    }
    if snapshot.suspicious {
        return Health::LargeChange;
    }
    if interval_hours > 0 {
        let allowed = i64::from(interval_hours) * 2 * 3_600_000;
        if now_ms.saturating_sub(snapshot.written_ms) > allowed {
            return Health::OutOfDate;
        }
    }
    Health::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> JobConfig {
        JobConfig {
            id: "photos".into(),
            name: "Photos".into(),
            source: Location {
                account: "b2".into(),
                bucket: "photos".into(),
                prefix: String::new(),
            },
            destination: Location {
                account: "hetzner".into(),
                bucket: "backup".into(),
                prefix: "sotyware/".into(),
            },
            interval_hours: 24,
            retention: Retention {
                keep_last: 7,
                keep_daily: 14,
                keep_weekly: 8,
            },
            suspicious_percent: DEFAULT_SUSPICIOUS_PERCENT,
        }
    }

    #[test]
    fn job_configurations_are_validated() {
        assert_eq!(validate(&job()), Ok(()));
        let bad = |f: &dyn Fn(&mut JobConfig)| {
            let mut j = job();
            f(&mut j);
            validate(&j).unwrap_err()
        };
        bad(&|j| j.id = "Photos".into());
        bad(&|j| j.name = "  ".into());
        bad(&|j| j.source.prefix = "no-slash".into());
        bad(&|j| j.destination.prefix = "../x/".into());
        bad(&|j| j.destination.bucket = String::new());
        bad(&|j| j.suspicious_percent = 0);
        bad(&|j| j.interval_hours = 24 * 32);
        // Nested source and destination in one bucket.
        let e = bad(&|j| {
            j.destination = j.source.clone();
            j.destination.prefix = "backup/".into();
        });
        assert!(e.contains("inside"), "{e}");
        // Side by side in one bucket is allowed (but same account).
        let mut j = job();
        j.source.prefix = "data/".into();
        j.destination = Location {
            account: "b2".into(),
            bucket: "photos".into(),
            prefix: "backup/".into(),
        };
        assert_eq!(validate(&j), Ok(()));
        assert!(same_account(&j));
        assert!(!same_account(&job()));
    }

    #[test]
    fn repository_names_are_stable() {
        let sha = "ab".repeat(32);
        assert_eq!(blob_key("p/", &sha), format!("p/blobs/sha256/ab/{sha}"));
        assert_eq!(
            snapshot_key("p/", "photos", "2026-11-15T10:30:00Z", "x1"),
            "p/snapshots/photos/20261115T103000Z-x1.json"
        );
        assert_eq!(compact_time("2026-11-15T10:30:00.123Z"), "20261115T103000Z");
    }

    fn entry(key: &str, size: u64, etag: &str) -> Entry {
        let sha = format!("{:0>64}", format!("{:x}", key.len() as u64 * 1000 + size));
        Entry {
            key: key.into(),
            size,
            blob: blob_key("", &sha),
            sha256: sha,
            etag: Some(etag.into()),
            version_id: None,
            last_modified: Some("2026-11-01T00:00:00Z".into()),
            content_type: None,
        }
    }

    fn manifest(entries: Vec<Entry>) -> Manifest {
        Manifest {
            format: SNAPSHOT_FORMAT.into(),
            version: FORMAT_VERSION,
            repository: "r".into(),
            snapshot: "s".into(),
            created: "2026-11-15T00:00:00Z".into(),
            job: JobRef {
                id: "photos".into(),
                name: "Photos".into(),
            },
            source: SourceRef {
                provider: "s3-compatible".into(),
                endpoint: "http://x".into(),
                bucket: "b".into(),
                prefix: String::new(),
            },
            entries,
            stats: Stats::default(),
            suspicious: false,
        }
    }

    fn listed(key: &str, size: u64, etag: &str) -> Listed {
        Listed {
            key: key.into(),
            size,
            etag: Some(etag.into()),
            last_modified: Some("2026-11-01T00:00:00Z".into()),
        }
    }

    #[test]
    fn unchanged_objects_are_reused_and_only_changes_are_transferred() {
        let prev = manifest(vec![
            entry("a", 1, "e1"),
            entry("b", 2, "e2"),
            entry("gone", 3, "e3"),
        ]);
        let p = plan(
            vec![
                listed("a", 1, "e1"),
                listed("b", 2, "e2-new"),
                listed("c", 4, "e4"),
                listed("folder/", 0, "x"),
            ],
            Some(&prev),
        );
        assert_eq!(
            p.unchanged
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
        assert_eq!(
            p.transfer
                .iter()
                .map(|l| l.key.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert_eq!((p.new, p.changed, p.deleted), (1, 1, 1));
        // Without a previous snapshot everything is new.
        let p = plan(vec![listed("a", 1, "e1")], None);
        assert_eq!((p.new, p.transfer.len()), (1, 1));
        // An entry without an ETag is never assumed unchanged.
        let mut no_etag = entry("a", 1, "e1");
        no_etag.etag = None;
        let mut l = listed("a", 1, "e1");
        l.etag = None;
        let p = plan(vec![l], Some(&manifest(vec![no_etag])));
        assert_eq!(p.transfer.len(), 1);
    }

    #[test]
    fn mass_changes_mark_a_snapshot_suspicious() {
        assert!(!suspicious(9, 9, 0, 30), "too few objects to judge");
        assert!(
            !suspicious(100, 20, 10, 30),
            "exactly 30 % is not more than 30 %"
        );
        assert!(suspicious(100, 20, 11, 30));
        assert!(suspicious(1000, 0, 400, 30), "a mass deletion");
    }

    #[test]
    fn manifests_are_checked_before_use() {
        let m = manifest(vec![entry("a", 1, "e")]);
        assert_eq!(m.check(), Ok(()));
        let mut newer = m.clone();
        newer.version = 2;
        assert!(newer.check().unwrap_err().contains("not supported"));
        let mut other = m.clone();
        other.format = "x".into();
        assert!(other.check().is_err());
        let mut bad = m.clone();
        bad.entries[0].blob = "blobs/sha256/zz/evil".into();
        assert!(bad.check().is_err());
        // Round trip through JSON keeps every field.
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"lastModified\"") && json.contains("\"versionId\""));
        assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
    }

    fn snap(n: u32, created: &str) -> SnapshotInfo {
        SnapshotInfo {
            key: format!("s{n:03}"),
            created: created.into(),
        }
    }

    #[test]
    fn retention_keeps_the_newest_and_what_the_policy_asks() {
        // One snapshot a day for 30 days, two on the last day.
        let mut s: Vec<SnapshotInfo> = (1..=30)
            .map(|d| snap(d, &format!("2026-11-{d:02}T03:00:00Z")))
            .collect();
        s.push(snap(31, "2026-11-30T15:00:00Z"));
        let all = keep(&s, Retention::default());
        assert_eq!(all.len(), 31, "no policy keeps everything");
        let k = keep(
            &s,
            Retention {
                keep_last: 2,
                keep_daily: 3,
                keep_weekly: 0,
            },
        );
        // Last two (s031, s030), plus the newest of 30, 29, 28 Nov.
        assert_eq!(
            k.into_iter().collect::<Vec<_>>(),
            ["s028", "s029", "s030", "s031"]
        );
        let k = keep(
            &s,
            Retention {
                keep_last: 0,
                keep_daily: 0,
                keep_weekly: 2,
            },
        );
        assert!(k.contains("s031"), "the newest is always kept");
        // 2026-11-30 is a Monday: its week has s030/s031, the week before
        // ends on Sunday 29 Nov.
        assert_eq!(k.into_iter().collect::<Vec<_>>(), ["s029", "s031"]);
        assert_eq!(
            keep(
                &[],
                Retention {
                    keep_last: 1,
                    ..Retention::default()
                }
            )
            .len(),
            0
        );
    }

    #[test]
    fn civil_dates_and_weeks() {
        assert_eq!(days_from_civil("1970-01-01"), Some(0));
        assert_eq!(days_from_civil("2000-03-01"), Some(11_017));
        assert_eq!(days_from_civil("2026-11-30T15:00:00Z"), Some(20_787));
        // 1970-01-01 was a Thursday; 2026-11-30 is a Monday.
        assert_eq!((20_787 + 3) % 7, 0);
        assert_eq!(days_from_civil("2026-13-01"), None);
        // Monday and the following Sunday share a week; the next Monday not.
        assert_eq!(week_of("2026-11-23"), week_of("2026-11-29"));
        assert_ne!(week_of("2026-11-29"), week_of("2026-11-30"));
    }

    #[test]
    fn health_is_never_healthy_without_a_passed_verification() {
        let day = 24 * 3_600_000;
        let now = 100 * day;
        let h = |last_run, last_snapshot, last_verification| {
            health(
                &History {
                    last_run,
                    last_snapshot,
                    last_verification,
                },
                24,
                now,
            )
        };
        // A snapshot whose run is record 10, and verifications before and
        // after it.
        let written = |written_ms, suspicious| {
            Some(SnapshotRecord {
                written_ms,
                suspicious,
                seq: 10,
            })
        };
        let verified = |seq, passed| Some(VerificationRecord { seq, passed });
        assert_eq!(h(None, None, None), Health::NeverRun);
        let ok = Some((RunState::Completed, now - 1000));
        let snap = written(now - 900, false);
        assert_eq!(h(ok, snap, None), Health::NotVerified);
        assert_eq!(
            h(ok, snap, verified(9, true)),
            Health::NotVerified,
            "recorded before the snapshot's run"
        );
        assert_eq!(h(ok, snap, verified(11, false)), Health::VerificationFailed);
        assert_eq!(h(ok, snap, verified(11, true)), Health::Healthy);
        assert_eq!(
            h(ok, written(now - 900, true), verified(11, true)),
            Health::LargeChange
        );
        assert_eq!(
            h(ok, written(now - 3 * day, false), verified(11, true)),
            Health::OutOfDate
        );
        assert_eq!(
            h(Some((RunState::Failed, now)), snap, verified(11, true)),
            Health::Failed
        );
        assert_eq!(
            h(Some((RunState::Partial, now)), snap, verified(11, true)),
            Health::Partial
        );
        // Manual jobs are never out of date.
        assert_eq!(
            health(
                &History {
                    last_run: ok,
                    last_snapshot: written(0, false),
                    last_verification: verified(11, true),
                },
                0,
                now
            ),
            Health::Healthy
        );
    }
}

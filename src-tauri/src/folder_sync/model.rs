//! Pure rules of folder sync (ADR-043): how a relative path maps to an
//! object key and back, which names can be synced on this computer, which
//! files are never synced, what to do with one path given its three states
//! (synced, local, remote), conflict copy names, the deletion guard and the
//! retry schedule. No file or request here; everything is unit-tested.
//!
//! A **relative path** (`rel`) is the path below the sync folder with `/`
//! between segments and no leading or trailing `/`: `photos/2024/a.jpg`.
//! Its object key is the sync's prefix (empty for the whole bucket) plus
//! `rel`, plus `/` for a folder marker. The mapping is one to one: a key
//! whose name cannot be a file name here, or a file whose name cannot be a
//! key, is never synced (it is reported as skipped), because renaming it
//! would make it a different key that the other side then copies back.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The folder inside a sync folder that sotyware owns: the marker that
/// identifies the sync folder, the staging folder for downloads and the
/// sync trash. Never synced.
pub const OWN_FOLDER: &str = ".sotyware-sync";

/// Longest object key S3 allows, in bytes.
pub const MAX_KEY_BYTES: usize = 1024;

/// Longest file or folder name most file systems accept, in bytes (UTF-8
/// on Linux and macOS; Windows counts UTF-16 units, which for a valid
/// UTF-8 name are never more than its bytes).
const MAX_NAME_BYTES: usize = 255;

/// A file modified this recently is probably still being written: its
/// upload waits until it settles.
pub const SETTLE: Duration = Duration::from_secs(2);

/// The deletion guard (ADR-043): deletions planned in one pass need the
/// user's approval when there are at least this many of them and they are
/// more than half of the synced files, or at least [`GUARD_ALWAYS`].
pub const GUARD_MIN: u64 = 50;
/// See [`GUARD_MIN`].
pub const GUARD_ALWAYS: u64 = 1000;

/// How long the sync trash keeps a file sync removed from the sync folder.
pub const TRASH_DAYS: u64 = 30;

// ------------------------------------------------------------------ paths

/// The object key of `rel` under `prefix` (`dir`: a folder marker, ending
/// in `/`).
pub fn key_of(prefix: &str, rel: &str, dir: bool) -> String {
    if dir {
        format!("{prefix}{rel}/")
    } else {
        format!("{prefix}{rel}")
    }
}

/// What a listed key is under `prefix`: its relative path and whether it is
/// a folder marker. `None` for keys outside the prefix, the prefix's own
/// marker, keys inside [`OWN_FOLDER`] and keys with empty segments (`a//b`,
/// which no file system can hold).
pub fn rel_of_key(prefix: &str, key: &str) -> Option<(String, bool)> {
    let rest = key.strip_prefix(prefix)?;
    let (rel, dir) = match rest.strip_suffix('/') {
        Some(r) => (r, true),
        None => (rest, false),
    };
    if rel.is_empty() || rel.split('/').any(str::is_empty) {
        return None;
    }
    if rel == OWN_FOLDER || rel.starts_with(&format!("{OWN_FOLDER}/")) {
        return None;
    }
    Some((rel.to_owned(), dir))
}

/// Every folder `rel` lies in, outermost first: `a/b/c` → `a`, `a/b`.
pub fn parents(rel: &str) -> impl Iterator<Item = &str> {
    rel.match_indices('/').map(move |(i, _)| &rel[..i])
}

/// The folder `rel` lies in (`` for the top level).
pub fn parent_of(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

/// The last segment of `rel`.
pub fn name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// `rel` as a file system that ignores case sees it: two paths with the
/// same fold are one file or folder there (Windows, macOS by default).
/// Unicode lower case; neither Windows' upcase table nor APFS's folding is
/// exactly this, and Unicode normalisation is not folded.
pub fn fold(rel: &str) -> String {
    rel.to_lowercase()
}

/// How deep `rel` is (`a` → 1, `a/b` → 2), for removing folders deepest
/// first.
pub fn depth(rel: &str) -> usize {
    rel.matches('/').count() + 1
}

/// Why `segment` (one name) cannot be a file or folder name on this
/// computer, if it cannot. The same rules apply to names coming from the
/// bucket and to names found in the sync folder.
pub fn name_problem(segment: &str) -> Option<&'static str> {
    name_problem_on(segment, cfg!(windows))
}

/// [`name_problem`] for Windows (`windows`) or for Linux and macOS, so both
/// rule sets are tested on every system.
pub fn name_problem_on(segment: &str, windows: bool) -> Option<&'static str> {
    if segment.is_empty() || segment == "." || segment == ".." {
        return Some("the name is empty, '.' or '..'");
    }
    if segment.len() > MAX_NAME_BYTES {
        return Some("the name is longer than 255 bytes");
    }
    if segment.chars().any(char::is_control) {
        return Some("the name contains control characters");
    }
    if windows {
        if segment
            .chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*'))
        {
            return Some(
                "the name contains a character Windows does not allow (< > : \" \\ | ? *)",
            );
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Some("the name ends with a dot or a space, which Windows drops");
        }
        let stem = segment
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0');
        if reserved {
            return Some("the name is reserved for a device on Windows");
        }
    }
    None
}

/// Why `rel` cannot be synced between the bucket (under `prefix`) and this
/// computer, if it cannot: a name no file system here can hold, or a key
/// longer than S3 allows.
pub fn path_problem(prefix: &str, rel: &str, dir: bool) -> Option<&'static str> {
    if rel.is_empty() {
        return Some("the name is empty");
    }
    if key_of(prefix, rel, dir).len() > MAX_KEY_BYTES {
        return Some("the path is longer than an object key may be (1,024 bytes)");
    }
    rel.split('/').find_map(name_problem)
}

/// Whether a file or folder named `name` at `rel` is never synced: the
/// sync's own folder, other sotyware temporary files, and files the
/// system or an open document keeps next to the user's (`desktop.ini`,
/// `Thumbs.db`, `.DS_Store`, Office's `~$` owner files, LibreOffice's lock
/// files). The one place exclusions are decided.
pub fn ignored(rel: &str) -> bool {
    if rel == OWN_FOLDER || rel.starts_with(&format!("{OWN_FOLDER}/")) {
        return true;
    }
    let name = name_of(rel);
    let lower = name.to_lowercase();
    matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | ".ds_store")
        || name.starts_with("~$")
        || (name.starts_with(".~lock.") && name.ends_with('#'))
        || lower.ends_with(".sotyware-partial")
        || name.starts_with(".sotyware-download-")
}

// -------------------------------------------------------------- the trees

/// The state of one path when it was last synced: both sides agreed on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Synced {
    pub path: String,
    pub dir: bool,
    /// Local size (files).
    pub size: u64,
    /// Local modification time, nanoseconds since the Unix epoch.
    pub mtime_ns: Option<i64>,
    /// Local file identity (inode on Linux and macOS) where known.
    pub file_id: Option<u64>,
    pub sha256: Option<String>,
    /// The remote object's ETag (quotes removed).
    pub etag: Option<String>,
    pub remote_size: Option<u64>,
}

/// One path as the last local scan found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Local {
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub file_id: Option<u64>,
}

/// One path as the last complete remote listing found it: an object, a
/// folder marker, or a folder implied by the keys below it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub path: String,
    pub dir: bool,
    /// A folder with its own marker object (`rel/`).
    pub marker: bool,
    pub size: u64,
    pub etag: Option<String>,
    pub modified_ms: Option<i64>,
}

/// One path with its three states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Triple {
    pub path: String,
    pub synced: Option<Synced>,
    pub local: Option<Local>,
    pub remote: Option<Remote>,
}

/// How one side compares with the synced state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Not synced before (no synced state).
    New,
    /// As synced.
    Same,
    /// Different from the synced state (for a local file: size, time or
    /// identity differ, so the content may or may not).
    Changed,
    /// Gone since it was synced.
    Gone,
    /// Absent, and not synced before either.
    Absent,
}

/// The local side of `t`.
pub fn local_side(t: &Triple) -> Side {
    match (&t.synced, &t.local) {
        (None, None) => Side::Absent,
        (None, Some(_)) => Side::New,
        (Some(_), None) => Side::Gone,
        (Some(s), Some(l)) => {
            if s.dir || (l.size == s.size && l.mtime_ns == s.mtime_ns && l.file_id == s.file_id) {
                Side::Same
            } else {
                Side::Changed
            }
        }
    }
}

/// The remote side of `t`.
pub fn remote_side(t: &Triple) -> Side {
    match (&t.synced, &t.remote) {
        (None, None) => Side::Absent,
        (None, Some(_)) => Side::New,
        (Some(_), None) => Side::Gone,
        (Some(s), Some(r)) => {
            if s.dir || (r.etag == s.etag && Some(r.size) == s.remote_size) {
                Side::Same
            } else {
                Side::Changed
            }
        }
    }
}

/// What to do with one path. Every action that changes something is
/// checked again against the live file and object right before it runs
/// (the engine), so a decision taken on an older listing or scan can only
/// do less, never destroy a newer change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// In step: nothing to do.
    Nothing,
    /// Upload the local file. `replace`: the ETag the key must still have
    /// (a modified file); `None`: the key must be free. `check_content`:
    /// only the local stamp changed, so the file is hashed first and, if
    /// its content is the synced one, only the stamp is recorded.
    Upload {
        replace: Option<String>,
        check_content: bool,
    },
    /// Download the object. `replace`: the local file is replaced (it is
    /// as synced); otherwise the path must be free. `check_content`: only
    /// the ETag changed, so the object's stored SHA-256 is read first and,
    /// if it is the synced content, only the ETag is recorded.
    Download { replace: bool, check_content: bool },
    /// Both sides have a file here that differs from (or never had) a
    /// synced state: compare the contents; equal → record, different →
    /// keep both (a conflict).
    Compare,
    /// The file was deleted here and is unchanged in the bucket: delete it
    /// there (guarded).
    DeleteRemote,
    /// The object was deleted and the file here is unchanged: move the
    /// file to the sync trash (guarded).
    DeleteLocal,
    /// Gone on both sides: forget the synced state.
    Forget,
    /// A folder only here: create it in the bucket.
    MakeRemoteFolder,
    /// A folder only in the bucket: create it here.
    MakeLocalFolder,
    /// A folder on both sides: record it.
    AdoptFolder,
    /// A synced folder deleted here: remove its marker (its files are
    /// handled one by one).
    RemoveRemoteFolder,
    /// A synced folder deleted in the bucket: remove it here once empty.
    RemoveLocalFolder,
    /// A file on one side and a folder on the other: not synced, reported.
    KindClash,
}

/// The decision for one path from its three states (see [`Decision`]).
pub fn decide(t: &Triple) -> Decision {
    let kinds = [
        t.synced.as_ref().map(|s| s.dir),
        t.local.as_ref().map(|l| l.dir),
        t.remote.as_ref().map(|r| r.dir),
    ];
    let mut present = kinds.iter().flatten();
    let Some(&dir) = present.next() else {
        return Decision::Nothing;
    };
    if present.any(|&k| k != dir) {
        return Decision::KindClash;
    }
    let (local, remote) = (local_side(t), remote_side(t));
    if dir {
        return match (local, remote) {
            (Side::New, Side::Absent) => Decision::MakeRemoteFolder,
            (Side::Absent, Side::New) => Decision::MakeLocalFolder,
            (Side::New, Side::New) => Decision::AdoptFolder,
            (Side::Gone, Side::Gone) => Decision::Forget,
            (Side::Gone, _) => Decision::RemoveRemoteFolder,
            (_, Side::Gone) => Decision::RemoveLocalFolder,
            _ => Decision::Nothing,
        };
    }
    let synced_etag = || t.synced.as_ref().and_then(|s| s.etag.clone());
    match (local, remote) {
        (Side::Same, Side::Same) | (Side::Absent, Side::Absent) => Decision::Nothing,
        (Side::New, Side::Absent) => Decision::Upload {
            replace: None,
            check_content: false,
        },
        (Side::Absent, Side::New) => Decision::Download {
            replace: false,
            check_content: false,
        },
        (Side::New, Side::New) | (Side::Changed, Side::Changed) => Decision::Compare,
        (Side::Changed, Side::Same) => Decision::Upload {
            replace: synced_etag(),
            check_content: true,
        },
        (Side::Same, Side::Changed) => Decision::Download {
            replace: true,
            check_content: true,
        },
        (Side::Gone, Side::Same) => Decision::DeleteRemote,
        (Side::Same, Side::Gone) => Decision::DeleteLocal,
        (Side::Gone, Side::Gone) => Decision::Forget,
        // A change wins over a deletion: nothing edited is ever lost.
        (Side::Gone, Side::Changed) => Decision::Download {
            replace: false,
            check_content: false,
        },
        (Side::Changed, Side::Gone) => Decision::Upload {
            replace: None,
            check_content: false,
        },
        // The remaining pairs mix "never synced" with "synced", which one
        // path cannot be on both sides at once.
        _ => Decision::Nothing,
    }
}

/// Whether `decision` deletes something (and so counts for the guard).
pub fn is_deletion(decision: &Decision) -> bool {
    matches!(decision, Decision::DeleteRemote | Decision::DeleteLocal)
}

/// When a decision runs: folders and transfers first, then deletions, then
/// folder removals (deepest first), so a folder is empty when it goes and
/// a moved file is copied before its old name is deleted.
pub fn phase(decision: &Decision) -> u8 {
    match decision {
        Decision::DeleteRemote | Decision::DeleteLocal => 1,
        Decision::RemoveRemoteFolder | Decision::RemoveLocalFolder => 2,
        _ => 0,
    }
}

// --------------------------------------------------------------- the guard

/// Whether `deletions` planned at once, out of `synced_files`, need the
/// user's approval before they run.
pub fn deletions_need_approval(deletions: u64, synced_files: u64) -> bool {
    deletions >= GUARD_ALWAYS || (deletions >= GUARD_MIN && deletions * 2 > synced_files)
}

// ---------------------------------------------------------------- retries

/// How long a path that failed `attempts` times waits before the next try:
/// 30 s, doubling, at most an hour.
pub fn retry_delay(attempts: u32) -> Duration {
    let exp = attempts.saturating_sub(1).min(7);
    Duration::from_secs((30u64 << exp).min(3600))
}

/// How long the engine waits before listing again after the provider could
/// not be reached `failures` times in a row: 30 s, doubling, at most 15 min.
pub fn offline_delay(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(6);
    Duration::from_secs((30u64 << exp).min(900))
}

// -------------------------------------------------------------- conflicts

/// The name of the copy that keeps the bucket's version of `rel` when both
/// sides changed it: `report (conflicted copy 2026-11-16 1403).docx`, in
/// the same folder, numbered (` 2`, ` 3`, …) when `n > 1`. The extension
/// stays last so the copy opens with the same application.
pub fn conflict_name(rel: &str, stamp: &str, n: u32) -> String {
    let (folder, name) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let tag = if n > 1 {
        format!("conflicted copy {stamp} {n}")
    } else {
        format!("conflicted copy {stamp}")
    };
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{folder}{} ({tag}){}", &name[..i], &name[i..]),
        _ => format!("{folder}{name} ({tag})"),
    }
}

/// `YYYY-MM-DD HHMM` (UTC) for a conflict copy's name, from milliseconds
/// since the Unix epoch.
pub fn conflict_stamp(ms: i64) -> String {
    let (y, mo, d, h, mi) = civil(ms);
    format!("{y:04}-{mo:02}-{d:02} {h:02}{mi:02}")
}

/// `YYYY-MM-DD` (UTC): the sync trash's folder for one day.
pub fn day_of(ms: i64) -> String {
    let (y, mo, d, _, _) = civil(ms);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// The UTC calendar date and time of `ms` (days-from-civil, inverted).
fn civil(ms: i64) -> (i64, i64, i64, i64, i64) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, (rem % 3600) / 60)
}

/// The days a sync trash folder named `name` (`YYYY-MM-DD`) is old at
/// `now_ms`, if `name` is such a date.
pub fn trash_age_days(name: &str, now_ms: i64) -> Option<i64> {
    let mut parts = name.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // days-from-civil
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(now_ms.div_euclid(86_400_000) - days)
}

// --------------------------------------------------------------- settings

/// How often the bucket is listed for changes made elsewhere. Every check
/// lists the whole bucket (about one request per 1,000 objects).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval(pub u32);

impl Interval {
    /// The choices the page offers, in minutes.
    pub const CHOICES: [u32; 4] = [5, 15, 30, 60];

    pub fn valid(minutes: u32) -> bool {
        Self::CHOICES.contains(&minutes)
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0) * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced_file(etag: &str) -> Synced {
        Synced {
            path: "a.txt".into(),
            dir: false,
            size: 3,
            mtime_ns: Some(100),
            file_id: Some(7),
            sha256: Some("s".into()),
            etag: Some(etag.into()),
            remote_size: Some(3),
        }
    }

    fn local_file(size: u64, mtime: i64) -> Local {
        Local {
            path: "a.txt".into(),
            dir: false,
            size,
            mtime_ns: Some(mtime),
            file_id: Some(7),
        }
    }

    fn remote_file(etag: &str, size: u64) -> Remote {
        Remote {
            path: "a.txt".into(),
            dir: false,
            marker: false,
            size,
            etag: Some(etag.into()),
            modified_ms: None,
        }
    }

    fn t(s: Option<Synced>, l: Option<Local>, r: Option<Remote>) -> Triple {
        Triple {
            path: "a.txt".into(),
            synced: s,
            local: l,
            remote: r,
        }
    }

    #[test]
    fn new_files_go_to_the_side_that_lacks_them() {
        assert_eq!(
            decide(&t(None, Some(local_file(3, 1)), None)),
            Decision::Upload {
                replace: None,
                check_content: false
            }
        );
        assert_eq!(
            decide(&t(None, None, Some(remote_file("e", 3)))),
            Decision::Download {
                replace: false,
                check_content: false
            }
        );
        assert_eq!(
            decide(&t(None, Some(local_file(3, 1)), Some(remote_file("e", 3)))),
            Decision::Compare
        );
    }

    #[test]
    fn unchanged_files_need_nothing() {
        let s = synced_file("e");
        assert_eq!(
            decide(&t(
                Some(s),
                Some(local_file(3, 100)),
                Some(remote_file("e", 3))
            )),
            Decision::Nothing
        );
        assert_eq!(decide(&t(None, None, None)), Decision::Nothing);
    }

    #[test]
    fn a_change_on_one_side_goes_to_the_other() {
        let s = synced_file("e");
        assert_eq!(
            decide(&t(
                Some(s.clone()),
                Some(local_file(4, 200)),
                Some(remote_file("e", 3))
            )),
            Decision::Upload {
                replace: Some("e".into()),
                check_content: true
            }
        );
        assert_eq!(
            decide(&t(
                Some(s.clone()),
                Some(local_file(3, 100)),
                Some(remote_file("f", 5))
            )),
            Decision::Download {
                replace: true,
                check_content: true
            }
        );
        // A different file identity alone (replaced by an editor's save) is
        // a change too.
        let mut l = local_file(3, 100);
        l.file_id = Some(8);
        assert_eq!(
            decide(&t(Some(s), Some(l), Some(remote_file("e", 3)))),
            Decision::Upload {
                replace: Some("e".into()),
                check_content: true
            }
        );
    }

    #[test]
    fn changes_on_both_sides_are_compared() {
        let s = synced_file("e");
        assert_eq!(
            decide(&t(
                Some(s),
                Some(local_file(4, 200)),
                Some(remote_file("f", 5))
            )),
            Decision::Compare
        );
    }

    #[test]
    fn deletions_only_follow_an_unchanged_other_side() {
        let s = synced_file("e");
        assert_eq!(
            decide(&t(Some(s.clone()), None, Some(remote_file("e", 3)))),
            Decision::DeleteRemote
        );
        assert_eq!(
            decide(&t(Some(s.clone()), Some(local_file(3, 100)), None)),
            Decision::DeleteLocal
        );
        assert_eq!(decide(&t(Some(s.clone()), None, None)), Decision::Forget);
        // A change wins over a deletion.
        assert_eq!(
            decide(&t(Some(s.clone()), None, Some(remote_file("f", 3)))),
            Decision::Download {
                replace: false,
                check_content: false
            }
        );
        assert_eq!(
            decide(&t(Some(s), Some(local_file(9, 300)), None)),
            Decision::Upload {
                replace: None,
                check_content: false
            }
        );
    }

    #[test]
    fn a_size_change_with_the_same_etag_is_a_remote_change() {
        let s = synced_file("e");
        assert_eq!(
            decide(&t(
                Some(s),
                Some(local_file(3, 100)),
                Some(remote_file("e", 4))
            )),
            Decision::Download {
                replace: true,
                check_content: true
            }
        );
    }

    fn dir_triple(s: bool, l: bool, r: bool) -> Triple {
        Triple {
            path: "d".into(),
            synced: s.then(|| Synced {
                path: "d".into(),
                dir: true,
                ..Synced::default()
            }),
            local: l.then(|| Local {
                path: "d".into(),
                dir: true,
                ..Local::default()
            }),
            remote: r.then(|| Remote {
                path: "d".into(),
                dir: true,
                ..Remote::default()
            }),
        }
    }

    #[test]
    fn folders_follow_their_own_table() {
        assert_eq!(
            decide(&dir_triple(false, true, false)),
            Decision::MakeRemoteFolder
        );
        assert_eq!(
            decide(&dir_triple(false, false, true)),
            Decision::MakeLocalFolder
        );
        assert_eq!(
            decide(&dir_triple(false, true, true)),
            Decision::AdoptFolder
        );
        assert_eq!(decide(&dir_triple(true, true, true)), Decision::Nothing);
        assert_eq!(
            decide(&dir_triple(true, false, true)),
            Decision::RemoveRemoteFolder
        );
        assert_eq!(
            decide(&dir_triple(true, true, false)),
            Decision::RemoveLocalFolder
        );
        assert_eq!(decide(&dir_triple(true, false, false)), Decision::Forget);
    }

    #[test]
    fn a_file_against_a_folder_is_never_synced() {
        let mut tr = dir_triple(false, true, false);
        tr.remote = Some(remote_file("e", 3));
        assert_eq!(decide(&tr), Decision::KindClash);
        let mut tr = t(Some(synced_file("e")), None, Some(remote_file("e", 3)));
        tr.local = Some(Local {
            path: "a.txt".into(),
            dir: true,
            ..Local::default()
        });
        assert_eq!(decide(&tr), Decision::KindClash);
    }

    #[test]
    fn deletions_run_after_transfers_and_folders_last() {
        assert_eq!(phase(&Decision::Compare), 0);
        assert_eq!(phase(&Decision::DeleteLocal), 1);
        assert_eq!(phase(&Decision::RemoveLocalFolder), 2);
        assert!(is_deletion(&Decision::DeleteRemote));
        assert!(!is_deletion(&Decision::RemoveRemoteFolder));
    }

    #[test]
    fn keys_and_relative_paths_map_one_to_one() {
        assert_eq!(key_of("", "a/b.txt", false), "a/b.txt");
        assert_eq!(key_of("p/", "a", true), "p/a/");
        assert_eq!(rel_of_key("", "a/b.txt"), Some(("a/b.txt".into(), false)));
        assert_eq!(rel_of_key("", "a/"), Some(("a".into(), true)));
        assert_eq!(rel_of_key("p/", "p/x"), Some(("x".into(), false)));
        assert_eq!(rel_of_key("p/", "q/x"), None);
        assert_eq!(rel_of_key("p/", "p/"), None, "the prefix's own marker");
        assert_eq!(rel_of_key("", "a//b"), None);
        assert_eq!(rel_of_key("", "/a"), None);
        assert_eq!(rel_of_key("", ".sotyware-sync/root.json"), None);
        assert_eq!(rel_of_key("", ".sotyware-sync/"), None);
        assert_eq!(
            rel_of_key("", ".sotyware-syncx"),
            Some((".sotyware-syncx".into(), false))
        );
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parents("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
        assert_eq!(parents("a").count(), 0);
        assert_eq!(parent_of("a/b/c"), "a/b");
        assert_eq!(parent_of("a"), "");
        assert_eq!(name_of("a/b/c.txt"), "c.txt");
        assert_eq!(depth("a/b/c"), 3);
    }

    #[test]
    fn folding_ignores_case_and_nothing_else() {
        assert_eq!(fold("Docs/Report.TXT"), "docs/report.txt");
        assert_eq!(fold("Größe/ÄÖÜ"), "größe/äöü");
        assert_eq!(fold("Docs/Report.TXT"), fold("docs/report.txt"));
        assert_ne!(fold("a b"), fold("a_b"));
    }

    #[test]
    fn names_that_no_file_system_here_can_hold_are_refused() {
        for bad in ["", ".", "..", "a\u{1}b"] {
            assert!(name_problem_on(bad, false).is_some(), "{bad:?}");
            assert!(name_problem_on(bad, true).is_some(), "{bad:?}");
        }
        assert!(name_problem_on(&"x".repeat(256), false).is_some());
        assert!(name_problem_on(&"x".repeat(255), false).is_none());
        // Linux and macOS take these; Windows does not.
        for name in [
            "a:b", "a\\b", "what?", "star*", "pipe|", "q\"", "<x>", "dot.", "space ", "CON",
            "con.txt", "COM1", "lpt9.log", "NUL ",
        ] {
            assert!(name_problem_on(name, false).is_none(), "{name:?} on Linux");
            assert!(name_problem_on(name, true).is_some(), "{name:?} on Windows");
        }
        for fine in ["COM0", "COM10", "CONSOLE", "a.b", "größe +test.bin", "日本"] {
            assert!(name_problem_on(fine, true).is_none(), "{fine:?}");
        }
    }

    #[test]
    fn long_keys_are_refused() {
        let rel = format!("{}/{}", "a".repeat(200), "b".repeat(200));
        assert!(path_problem("", &rel, false).is_none());
        let deep = vec!["d".repeat(200); 6].join("/");
        assert!(path_problem("", &deep, false).is_some());
    }

    #[test]
    fn system_and_temporary_files_are_ignored() {
        for rel in [
            ".sotyware-sync",
            ".sotyware-sync/trash/x",
            "a/desktop.ini",
            "Thumbs.db",
            "x/.DS_Store",
            "~$report.docx",
            ".~lock.report.odt#",
            ".a.txt.123-4.sotyware-partial",
            ".sotyware-download-12-3",
        ] {
            assert!(ignored(rel), "{rel}");
        }
        for rel in [
            "a/.sotyware-sync",
            "report.docx",
            "lock#",
            "~report",
            "partial",
        ] {
            assert!(!ignored(rel), "{rel}");
        }
    }

    #[test]
    fn the_guard_holds_large_or_sweeping_deletions() {
        assert!(!deletions_need_approval(10, 12), "few files: no question");
        assert!(!deletions_need_approval(49, 50));
        assert!(deletions_need_approval(50, 60));
        assert!(
            !deletions_need_approval(50, 100),
            "half is not more than half"
        );
        assert!(deletions_need_approval(51, 100));
        assert!(!deletions_need_approval(999, 100_000));
        assert!(deletions_need_approval(1000, 100_000));
    }

    #[test]
    fn retries_back_off() {
        assert_eq!(retry_delay(1), Duration::from_secs(30));
        assert_eq!(retry_delay(2), Duration::from_secs(60));
        assert_eq!(retry_delay(7), Duration::from_secs(1920));
        assert_eq!(retry_delay(8), Duration::from_secs(3600));
        assert_eq!(retry_delay(100), Duration::from_secs(3600));
        assert_eq!(offline_delay(1), Duration::from_secs(30));
        assert_eq!(offline_delay(5), Duration::from_secs(480));
        assert_eq!(offline_delay(50), Duration::from_secs(900));
    }

    #[test]
    fn conflict_copies_keep_the_extension_last() {
        assert_eq!(
            conflict_name("docs/report.docx", "2026-11-16 1403", 1),
            "docs/report (conflicted copy 2026-11-16 1403).docx"
        );
        assert_eq!(
            conflict_name("notes", "2026-11-16 1403", 2),
            "notes (conflicted copy 2026-11-16 1403 2)"
        );
        assert_eq!(
            conflict_name(".env", "2026-11-16 1403", 1),
            ".env (conflicted copy 2026-11-16 1403)"
        );
    }

    #[test]
    fn dates_are_utc_calendar_dates() {
        // 2026-11-16T14:03:00Z
        let ms = 1_794_837_780_000;
        assert_eq!(conflict_stamp(ms), "2026-11-16 1403");
        assert_eq!(day_of(ms), "2026-11-16");
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(trash_age_days("2026-11-16", ms), Some(0));
        assert_eq!(trash_age_days("2026-10-16", ms), Some(31));
        assert_eq!(trash_age_days("2024-02-29", 1_709_164_800_000), Some(0));
        assert_eq!(trash_age_days("not-a-date", ms), None);
        assert_eq!(trash_age_days("2026-13-01", ms), None);
    }

    #[test]
    fn intervals_are_one_of_the_choices() {
        assert!(Interval::valid(15));
        assert!(!Interval::valid(1));
        assert_eq!(Interval(15).duration(), Duration::from_secs(900));
    }
}

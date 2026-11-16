//! The local side of folder sync (ADR-043): walking the sync folder,
//! stamps and hashes of files, the marker that identifies a sync folder,
//! sotyware's own folder inside it (staging and trash), free space, and
//! inspecting a folder the user chose. Blocking file system code; the
//! engine runs it on the blocking pool.
//!
//! The walk never follows a symbolic link or a junction (it is skipped and
//! reported), never reads below a folder it cannot list (that folder is
//! reported and its contents count as unknown, never as deleted), and
//! skips what `model::ignored` names and names that cannot be object keys.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::model::{self, Local, OWN_FOLDER};

/// The marker file inside [`OWN_FOLDER`].
pub const MARKER_FILE: &str = "root.json";
/// Downloads are written and verified here before they are moved into
/// place (same volume as the sync folder, so the move is a rename).
pub const STAGING: &str = "staging";
/// Files sync removed from the sync folder, kept [`model::TRASH_DAYS`].
pub const TRASH: &str = "trash";

/// Entries per batch handed to the caller during a walk.
pub const BATCH: usize = 1000;

/// A stamp: what tells a file changed without reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub file_id: Option<u64>,
}

impl Stamp {
    pub fn of(meta: &fs::Metadata) -> Self {
        Self {
            size: meta.len(),
            mtime_ns: meta.modified().ok().and_then(nanos),
            file_id: file_id(meta),
        }
    }

    /// The modification time in milliseconds (for the object's metadata).
    pub fn mtime_ms(&self) -> Option<i64> {
        self.mtime_ns.map(|ns| ns.div_euclid(1_000_000))
    }
}

fn nanos(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}

#[cfg(unix)]
fn file_id(meta: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

/// Windows has no stable file identity in `std` (the file index is
/// unstable); size and time alone tell a change there.
#[cfg(not(unix))]
fn file_id(_: &fs::Metadata) -> Option<u64> {
    None
}

/// `rel` as a path below `root`.
pub fn join(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for segment in rel.split('/') {
        p.push(segment);
    }
    p
}

/// What a walk or a targeted look found besides the entries.
#[derive(Debug, Default)]
pub struct Findings {
    /// Folders that could not be listed: what is below them is unknown.
    pub unknown: Vec<String>,
    /// Paths not synced, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// What a name in the sync folder is, for the walk.
enum Kind {
    File(Local),
    Folder(Local),
    Skip(String),
    Ignore,
}

/// Classifies `path` (at `rel`) from its metadata, without following a
/// link.
fn classify(path: &Path, rel: &str, prefix: &str) -> io::Result<Kind> {
    if model::ignored(rel) {
        return Ok(Kind::Ignore);
    }
    let meta = fs::symlink_metadata(path)?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        return Ok(Kind::Skip(
            "symbolic links and junctions are not synced".into(),
        ));
    }
    if !kind.is_file() && !kind.is_dir() {
        return Ok(Kind::Skip(
            "only files and folders are synced (this is a device, socket or pipe)".into(),
        ));
    }
    if let Some(problem) = model::path_problem(prefix, rel, kind.is_dir()) {
        return Ok(Kind::Skip(problem.into()));
    }
    let stamp = Stamp::of(&meta);
    let local = Local {
        path: rel.to_owned(),
        dir: kind.is_dir(),
        size: if kind.is_dir() { 0 } else { stamp.size },
        mtime_ns: if kind.is_dir() { None } else { stamp.mtime_ns },
        file_id: if kind.is_dir() { None } else { stamp.file_id },
    };
    Ok(if kind.is_dir() {
        Kind::Folder(local)
    } else {
        Kind::File(local)
    })
}

/// The relative path of `name` in the folder `parent` (`""`: the top), or
/// why the name cannot be synced (not UTF-8).
fn child_rel(parent: &str, name: &std::ffi::OsStr) -> Result<String, ()> {
    let name = name.to_str().ok_or(())?;
    Ok(if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    })
}

/// Walks the folder `start` (relative; `""` for the whole sync folder)
/// below `root`, depth first, handing entries to `batch` in batches of
/// [`BATCH`]. `start` itself is included when it is not the top. Stops
/// early, with what it found so far, when `stop` is set (the caller then
/// treats the walk as incomplete).
pub fn walk(
    root: &Path,
    start: &str,
    prefix: &str,
    stop: &AtomicBool,
    batch: &mut dyn FnMut(&[Local]) -> Result<(), String>,
) -> Result<Findings, String> {
    let mut findings = Findings::default();
    let mut pending: Vec<Local> = Vec::with_capacity(BATCH);
    let mut stack: Vec<String> = Vec::new();
    if start.is_empty() {
        stack.push(String::new());
    } else {
        match classify(&join(root, start), start, prefix) {
            Ok(Kind::Folder(l)) => {
                pending.push(l);
                stack.push(start.to_owned());
            }
            Ok(Kind::File(l)) => pending.push(l),
            Ok(Kind::Skip(reason)) => findings.skipped.push((start.to_owned(), reason)),
            Ok(Kind::Ignore) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => findings.unknown.push(start.to_owned()),
        }
    }
    while let Some(dir) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            return Err("the scan was stopped".into());
        }
        let entries = match fs::read_dir(join(root, &dir)) {
            Ok(entries) => entries,
            Err(e) if dir.is_empty() => {
                return Err(format!("the sync folder cannot be read: {e}"));
            }
            Err(_) => {
                findings.unknown.push(dir);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    // The listing broke off: nothing below can be trusted
                    // to be complete.
                    findings.unknown.push(dir.clone());
                    break;
                }
            };
            let Ok(rel) = child_rel(&dir, &entry.file_name()) else {
                findings.skipped.push((
                    format!(
                        "{}{}",
                        if dir.is_empty() { "" } else { "/" },
                        entry.file_name().to_string_lossy()
                    ),
                    "the name is not valid Unicode, so it cannot be an object key".into(),
                ));
                continue;
            };
            match classify(&entry.path(), &rel, prefix) {
                Ok(Kind::File(l)) => pending.push(l),
                Ok(Kind::Folder(l)) => {
                    pending.push(l);
                    stack.push(rel);
                }
                Ok(Kind::Skip(reason)) => findings.skipped.push((rel, reason)),
                Ok(Kind::Ignore) => {}
                // Vanished between the listing and the look: not there.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) => findings.unknown.push(rel),
            }
            if pending.len() >= BATCH {
                batch(&pending)?;
                pending.clear();
            }
        }
    }
    if !pending.is_empty() {
        batch(&pending)?;
    }
    Ok(findings)
}

/// How the sync folder's file system treats the case of names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Names that differ in case are different names (Linux, mostly): a
    /// path that exists is spelled as it exists.
    Sensitive,
    /// Case is ignored (Windows, macOS by default) and the canonical path
    /// names every segment as stored (`GetFinalPathNameByHandleW`, macOS's
    /// `realpath`): one call tells two spellings apart.
    IgnoredCanonical,
    /// Case is ignored and only a folder's listing tells two spellings
    /// apart (Linux's `realpath` on a FAT, exFAT or case-folding volume).
    IgnoredListed,
}

impl Case {
    /// Whether two spellings can name one file or folder here.
    pub fn ignored(self) -> bool {
        self != Case::Sensitive
    }
}

/// One path as it is now: `Ok(None)` when it does not exist (or is
/// ignored), `Err` with the reason it is not synced. Where `case` is
/// ignored, a path that exists only under another spelling (`report.txt`
/// for `Report.txt`) is not there: the scan never records one file under
/// two names.
pub fn look(
    root: &Path,
    rel: &str,
    prefix: &str,
    case: Case,
) -> io::Result<Result<Option<Local>, String>> {
    let found = match classify(&join(root, rel), rel, prefix) {
        Ok(Kind::File(l) | Kind::Folder(l)) => Ok(Some(l)),
        Ok(Kind::Ignore) => return Ok(Ok(None)),
        Ok(Kind::Skip(reason)) => Err(reason),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Ok(None)),
        Err(e) => return Err(e),
    };
    if !spelled_exactly(root, rel, case)? {
        return Ok(Ok(None));
    }
    Ok(found)
}

/// Whether something exists at `rel`, spelled exactly so (where `case` is
/// ignored, any spelling would be found otherwise).
pub fn exists(root: &Path, rel: &str, case: Case) -> io::Result<bool> {
    match fs::symlink_metadata(join(root, rel)) {
        Ok(_) => spelled_exactly(root, rel, case),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Whether every segment of `rel`, a path that exists, is spelled as the
/// file system stores it. A file system that ignores case opens
/// `report.txt` for `Report.txt`; its canonical path or its listing tells
/// them apart. A path through a link to a folder does not count (sync
/// never follows one); a link itself is judged by its own name.
pub fn spelled_exactly(root: &Path, rel: &str, case: Case) -> io::Result<bool> {
    match case {
        Case::Sensitive => Ok(true),
        Case::IgnoredCanonical => {
            let path = join(root, rel);
            // A link resolves to its target's name: its own is listed.
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return listed_exactly(root, rel);
            }
            match fs::canonicalize(&path) {
                Ok(real) => {
                    let mut stored = real.components().rev();
                    Ok(rel.rsplit('/').all(|segment| {
                        matches!(stored.next(), Some(std::path::Component::Normal(name))
                            if name == std::ffi::OsStr::new(segment))
                    }))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                // A volume that cannot name a handle's path lists folders.
                Err(_) => listed_exactly(root, rel),
            }
        }
        Case::IgnoredListed => listed_exactly(root, rel),
    }
}

/// [`spelled_exactly`] by listing each segment's folder; every segment
/// but the last must be a folder, not a link to one.
fn listed_exactly(root: &Path, rel: &str) -> io::Result<bool> {
    let mut dir = root.to_path_buf();
    let mut segments = rel.split('/').peekable();
    while let Some(segment) = segments.next() {
        let name = std::ffi::OsStr::new(segment);
        let last = segments.peek().is_none();
        let found = match fs::read_dir(&dir) {
            Ok(entries) => entries.flatten().any(|e| {
                e.file_name() == name && (last || e.file_type().is_ok_and(|t| t.is_dir()))
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        if !found {
            return Ok(false);
        }
        dir.push(segment);
    }
    Ok(true)
}

/// How the file system holding `root` treats case: a probe file in
/// sotyware's own folder, looked up and canonicalised under another
/// spelling. Where the probe cannot be made, the platform's default is
/// assumed (Windows and macOS ignore case unless set up otherwise), with
/// listings, which every file system answers as stored.
pub fn probe_case(root: &Path) -> Case {
    let default = if cfg!(any(windows, target_os = "macos")) {
        Case::IgnoredListed
    } else {
        Case::Sensitive
    };
    let Ok(own) = prepare_own_dir(root) else {
        return default;
    };
    let name = format!("case-probe-{}", std::process::id());
    let probe = own.join(&name);
    if fs::write(&probe, b"").is_err() {
        return default;
    }
    let other = own.join(name.to_uppercase());
    let case = if other.symlink_metadata().is_err() {
        Case::Sensitive
    } else if fs::canonicalize(&other)
        .is_ok_and(|real| real.file_name() == Some(std::ffi::OsStr::new(&name)))
    {
        Case::IgnoredCanonical
    } else {
        Case::IgnoredListed
    };
    let _ = fs::remove_file(&probe);
    case
}

/// The stamp of the file at `path` now, `None` if it is not there.
pub fn stamp(path: &Path) -> io::Result<Option<Stamp>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(Some(Stamp::of(&m))),
        Ok(_) => Err(io::Error::other("not a regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The SHA-256 of the file at `path` and the stamp it had: fails if the
/// file changed while it was read (its stamp before and after differ).
pub fn hash(path: &Path) -> io::Result<(String, Stamp)> {
    let mut file = fs::File::open(path)?;
    let before = Stamp::of(&file.metadata()?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let after = Stamp::of(&file.metadata()?);
    if before != after {
        return Err(io::Error::other("the file changed while it was read"));
    }
    Ok((hex(&hasher.finalize()), before))
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ----------------------------------------------------------------- marker

/// What identifies a folder as one sync's folder: `.sotyware-sync/root.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub format: u32,
    /// The sync's marker id (its configuration keeps the same).
    pub sync: String,
    pub account: String,
    pub bucket: String,
    pub created_ms: i64,
}

/// Whether a sync folder can be synced now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootState {
    /// The folder is there and is this sync's folder.
    Ready,
    /// The folder (or its drive) is not there: nothing is synced until it
    /// is back. Never a reason to delete anything.
    Missing,
    /// The folder is there, but without this sync's marker: another disk
    /// may be mounted at its place, or the marker was removed. Nothing is
    /// synced until the user decides.
    Foreign(String),
}

pub fn own_dir(root: &Path) -> PathBuf {
    root.join(OWN_FOLDER)
}

/// The state of `root` for the sync whose marker id is `marker`.
pub fn root_state(root: &Path, marker: &str) -> RootState {
    match fs::metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return RootState::Foreign("the sync folder's path is not a folder".into()),
        Err(_) => return RootState::Missing,
    }
    match read_marker(root) {
        Ok(Some(m)) if m.sync == marker => RootState::Ready,
        Ok(Some(_)) => RootState::Foreign(
            "the folder belongs to another sync (its marker names another one)".into(),
        ),
        Ok(None) => RootState::Foreign(
            "the folder is not marked as this sync's folder any more (.sotyware-sync is missing)"
                .into(),
        ),
        Err(e) => RootState::Foreign(format!("the sync folder's marker cannot be read: {e}")),
    }
}

/// The marker in `root`, `None` if there is none.
pub fn read_marker(root: &Path) -> io::Result<Option<Marker>> {
    let path = own_dir(root).join(MARKER_FILE);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Creates sotyware's folder in `root` (hidden on Windows) and writes the
/// marker, replacing it in one rename.
pub fn write_marker(root: &Path, marker: &Marker) -> io::Result<()> {
    let own = prepare_own_dir(root)?;
    let tmp = own.join(format!("{MARKER_FILE}.{}.tmp", std::process::id()));
    let json = serde_json::to_vec_pretty(marker).map_err(io::Error::other)?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, own.join(MARKER_FILE))
}

/// Removes the marker and the staging folder (sotyware's own files only).
/// The trash stays if it holds anything (the user's files); the own folder
/// goes when that left it empty. Returns whether the trash still holds
/// files.
pub fn remove_marker(root: &Path) -> io::Result<bool> {
    let own = own_dir(root);
    match fs::remove_file(own.join(MARKER_FILE)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    clear_staging(root);
    let _ = fs::remove_dir(own.join(STAGING));
    let trash = own.join(TRASH);
    let _ = remove_empty_dirs(&trash);
    let trash_kept = trash.exists();
    let _ = fs::remove_dir(&own);
    Ok(trash_kept)
}

/// Creates `.sotyware-sync` (hidden on Windows) and its staging and trash
/// folders.
pub fn prepare_own_dir(root: &Path) -> io::Result<PathBuf> {
    let own = own_dir(root);
    fs::create_dir_all(own.join(STAGING))?;
    fs::create_dir_all(own.join(TRASH))?;
    platform::hide(&own);
    Ok(own)
}

/// A new, unused path in the staging folder for one download.
pub fn staging_file(root: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    own_dir(root).join(STAGING).join(format!(
        "download-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed) + 1
    ))
}

/// Removes what earlier downloads left in the staging folder: only files
/// named like [`staging_file`]'s and download temporaries.
pub fn clear_staging(root: &Path) {
    let Ok(entries) = fs::read_dir(own_dir(root).join(STAGING)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = name.starts_with("download-") || name.ends_with(".sotyware-partial");
        if ours && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Moves the file or empty folder at `rel` into the sync trash
/// (`.sotyware-sync/trash/<day>/<rel>`), never replacing anything there.
/// Returns where it went.
pub fn move_to_trash(root: &Path, rel: &str, now_ms: i64) -> io::Result<PathBuf> {
    let day = own_dir(root).join(TRASH).join(model::day_of(now_ms));
    let mut target = join(&day, rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut n = 1;
    while target.symlink_metadata().is_ok() {
        n += 1;
        target = join(&day, model::parent_of(rel)).join(numbered(model::name_of(rel), n));
    }
    fs::rename(join(root, rel), &target)?;
    Ok(target)
}

fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

/// Removes the sync trash's day folders older than [`model::TRASH_DAYS`].
/// Returns how many it removed.
pub fn purge_trash(root: &Path, now_ms: i64) -> usize {
    let trash = own_dir(root).join(TRASH);
    let Ok(entries) = fs::read_dir(&trash) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let old = name
            .to_str()
            .and_then(|n| model::trash_age_days(n, now_ms))
            .is_some_and(|age| age > i64::try_from(model::TRASH_DAYS).unwrap_or(i64::MAX));
        if old
            && entry.file_type().is_ok_and(|t| t.is_dir())
            && fs::remove_dir_all(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Removes empty folders below and including `dir`, deepest first.
fn remove_empty_dirs(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = remove_empty_dirs(&entry.path());
        }
    }
    fs::remove_dir(dir)
}

/// The files and bytes in the sync trash.
pub fn trash_usage(root: &Path) -> (u64, u64) {
    let mut out = (0, 0);
    let mut stack = vec![own_dir(root).join(TRASH)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => {
                    out.0 += 1;
                    out.1 += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    out
}

// -------------------------------------------------------- choosing folders

/// What sotyware found in a folder the user chose.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub exists: bool,
    /// Files below it (at most [`COUNT_LIMIT`] are counted).
    pub files: u64,
    pub bytes: u64,
    /// The count stopped at the limit (or the time limit).
    pub more: bool,
    /// A folder below could not be read.
    pub unreadable: bool,
    /// It has a sync marker (`Some(marker)`): it was a sync folder.
    pub marker: Option<Marker>,
    /// A probe file could be created and removed.
    pub writable: bool,
    pub free_bytes: Option<u64>,
}

/// Files counted at most when a folder is inspected.
pub const COUNT_LIMIT: u64 = 100_000;

/// Inspects a folder the user chose, bounded (count limit and two
/// seconds).
pub fn inspect(path: &Path) -> Facts {
    let mut facts = Facts {
        exists: path.is_dir(),
        ..Facts::default()
    };
    if !facts.exists {
        return facts;
    }
    facts.marker = read_marker(path).ok().flatten();
    facts.writable = probe_writable(path);
    facts.free_bytes = free_space(path);
    let started = Instant::now();
    let mut stack = vec![path.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            facts.unreadable = true;
            continue;
        };
        for entry in entries.flatten() {
            if dir == path && entry.file_name() == OWN_FOLDER {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => {
                    facts.files += 1;
                    facts.bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
            if facts.files >= COUNT_LIMIT || started.elapsed().as_secs() >= 2 {
                facts.more = true;
                break 'walk;
            }
        }
    }
    facts
}

fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".sotyware-sync-probe-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            fs::remove_file(&probe).is_ok()
        }
        Err(_) => false,
    }
}

/// Free space for this user on the volume holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    platform::free_space(path)
}

/// Whether `inner` is `outer` or lies inside it (both canonical).
pub fn within(inner: &Path, outer: &Path) -> bool {
    inner.starts_with(outer)
}

/// The canonical form of `path` (links resolved), for comparing sync
/// folders; on Windows without the `\\?\` prefix `canonicalize` adds.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let c = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        if let Some(s) = c.to_str()
            && let Some(rest) = s.strip_prefix(r"\\?\")
            && !rest.starts_with("UNC\\")
        {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(c)
}

/// A random-looking, unique id (not secret): SHA-256 of the time, the
/// process, a counter and `salt`.
pub fn unique_id(salt: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = Sha256::new();
    h.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    h.update(std::process::id().to_le_bytes());
    h.update(N.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    h.update(salt.as_bytes());
    hex(&h.finalize())[..24].to_owned()
}

// ---------------------------------------------------------------- platform

#[cfg(unix)]
mod platform {
    use std::path::Path;

    /// Dot-names are hidden already.
    pub fn hide(_: &Path) {}

    pub fn free_space(path: &Path) -> Option<u64> {
        let s = rustix::fs::statvfs(path).ok()?;
        Some(s.f_bavail.saturating_mul(s.f_frsize))
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_HIDDEN, GetDiskFreeSpaceExW, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
        SetFileAttributesW,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// Sets the hidden attribute: a dot does not hide a folder on Windows.
    pub fn hide(path: &Path) {
        let w = wide(path);
        // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives both
        // calls; neither keeps the pointer.
        unsafe {
            let attrs = GetFileAttributesW(w.as_ptr());
            if attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_HIDDEN == 0 {
                SetFileAttributesW(w.as_ptr(), attrs | FILE_ATTRIBUTE_HIDDEN);
            }
        }
    }

    pub fn free_space(path: &Path) -> Option<u64> {
        let w = wide(path);
        let mut free: u64 = 0;
        // SAFETY: `w` is NUL-terminated and outlives the call; `free` is a
        // valid out pointer; the two other out pointers may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                w.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::path::Path;

    pub fn hide(_: &Path) {}

    pub fn free_space(_: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::TempDir;

    fn collect(root: &Path) -> (Vec<Local>, Findings) {
        let mut all = Vec::new();
        let stop = AtomicBool::new(false);
        let f = walk(root, "", "", &stop, &mut |b| {
            all.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        all.sort_by(|a, b| a.path.cmp(&b.path));
        (all, f)
    }

    #[test]
    fn the_walk_finds_files_and_folders_and_skips_its_own() {
        let t = TempDir::new("sync-walk");
        let root = t.path("root");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("a/b/c.txt"), b"abc").unwrap();
        fs::write(root.join("top.txt"), b"x").unwrap();
        fs::write(root.join("desktop.ini"), b"ignored").unwrap();
        fs::write(root.join("~$doc.docx"), b"ignored").unwrap();
        prepare_own_dir(&root).unwrap();
        fs::write(own_dir(&root).join(TRASH).join("old.txt"), b"in trash").unwrap();
        let (all, f) = collect(&root);
        let paths: Vec<(&str, bool, u64)> = all
            .iter()
            .map(|l| (l.path.as_str(), l.dir, l.size))
            .collect();
        assert_eq!(
            paths,
            [
                ("a", true, 0),
                ("a/b", true, 0),
                ("a/b/c.txt", false, 3),
                ("empty", true, 0),
                ("top.txt", false, 1)
            ]
        );
        assert!(f.unknown.is_empty() && f.skipped.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn links_and_special_files_are_skipped_and_never_followed() {
        let t = TempDir::new("sync-links");
        let root = t.path("root");
        let outside = t.path("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"s").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link-dir")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link-file")).unwrap();
        let (all, f) = collect(&root);
        assert!(all.is_empty(), "{all:?}");
        let skipped: Vec<&str> = f.skipped.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(skipped.len(), 2);
        assert!(skipped.contains(&"link-dir") && skipped.contains(&"link-file"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_unknown_not_empty() {
        use std::os::unix::fs::PermissionsExt;
        let t = TempDir::new("sync-unreadable");
        let root = t.path("root");
        fs::create_dir_all(root.join("locked")).unwrap();
        fs::write(root.join("locked/x"), b"x").unwrap();
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(root.join("locked")).is_ok(); // root ignores modes
        let (all, f) = collect(&root);
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        if readable {
            return;
        }
        assert_eq!(all.len(), 1, "the folder itself");
        assert_eq!(f.unknown, ["locked"]);
    }

    #[test]
    fn hashing_reports_the_stamp_it_read() {
        let t = TempDir::new("sync-hash");
        let p = t.file("f", 3 << 20, 7);
        let (sha, stamp) = hash(&p).unwrap();
        assert_eq!(
            sha,
            crate::cloud::test_server::sha256_hex(&fs::read(&p).unwrap())
        );
        assert_eq!(stamp.size, 3 << 20);
        assert_eq!(Some(stamp), super::stamp(&p).unwrap());
    }

    #[test]
    fn spellings_are_checked_as_the_file_system_stores_them() {
        let t = TempDir::new("sync-spelling");
        let root = t.path("root");
        fs::create_dir_all(root.join("Docs")).unwrap();
        fs::write(root.join("Docs/Report.txt"), b"x").unwrap();
        // The probe agrees with what this file system does and leaves
        // nothing behind.
        let case = probe_case(&root);
        assert_eq!(case.ignored(), root.join("docs/report.txt").exists());
        if cfg!(windows) {
            // NTFS: the final path names every segment as stored.
            assert_eq!(case, Case::IgnoredCanonical);
        }
        assert_eq!(
            fs::read_dir(own_dir(&root))
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().contains("probe"))
                .count(),
            0,
            "the probe is removed"
        );
        // Both ways agree, whatever this file system does with another
        // spelling (the canonical path only where the probe trusts it).
        let mut ways = vec![Case::IgnoredListed];
        if case != Case::IgnoredListed {
            ways.push(Case::IgnoredCanonical);
        }
        for how in ways {
            assert!(
                spelled_exactly(&root, "Docs/Report.txt", how).unwrap(),
                "{how:?}"
            );
            assert!(exists(&root, "Docs/Report.txt", how).unwrap(), "{how:?}");
            for other in ["docs/Report.txt", "Docs/report.txt", "Docs/missing.txt"] {
                assert!(!exists(&root, other, how).unwrap(), "{other} {how:?}");
            }
            let found = look(&root, "Docs/Report.txt", "", how).unwrap().unwrap();
            assert_eq!(found.map(|l| l.size), Some(1), "{how:?}");
            assert_eq!(
                look(&root, "docs/report.txt", "", how).unwrap(),
                Ok(None),
                "{how:?}"
            );
        }
        assert!(exists(&root, "Docs/Report.txt", Case::Sensitive).unwrap());
        assert!(!exists(&root, "Docs/other.txt", Case::Sensitive).unwrap());
        // Through a link to a folder a path is not spelled as stored; a
        // link is judged by its own name.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("Docs"), root.join("Link")).unwrap();
            for how in [Case::IgnoredListed, Case::IgnoredCanonical] {
                assert!(
                    !spelled_exactly(&root, "Link/Report.txt", how).unwrap(),
                    "{how:?}"
                );
                assert!(spelled_exactly(&root, "Link", how).unwrap(), "{how:?}");
            }
        }
    }

    #[test]
    fn the_marker_identifies_the_sync_folder() {
        let t = TempDir::new("sync-marker");
        let root = t.path("root");
        assert_eq!(root_state(&root, "m1"), RootState::Missing);
        fs::create_dir_all(&root).unwrap();
        assert!(matches!(root_state(&root, "m1"), RootState::Foreign(_)));
        let marker = Marker {
            format: 1,
            sync: "m1".into(),
            account: "acc".into(),
            bucket: "b".into(),
            created_ms: 1,
        };
        write_marker(&root, &marker).unwrap();
        assert_eq!(read_marker(&root).unwrap(), Some(marker));
        assert_eq!(root_state(&root, "m1"), RootState::Ready);
        assert!(matches!(root_state(&root, "m2"), RootState::Foreign(_)));
        // Removing it keeps a trash that holds files, and says so.
        fs::write(own_dir(&root).join(TRASH).join("kept.txt"), b"k").unwrap();
        fs::write(own_dir(&root).join(STAGING).join("download-1-1"), b"tmp").unwrap();
        assert!(remove_marker(&root).unwrap());
        assert!(own_dir(&root).join(TRASH).join("kept.txt").exists());
        assert!(!own_dir(&root).join(STAGING).exists());
        fs::remove_file(own_dir(&root).join(TRASH).join("kept.txt")).unwrap();
        assert!(!remove_marker(&root).unwrap());
        assert!(!own_dir(&root).exists());
    }

    #[test]
    fn the_trash_never_replaces_and_forgets_old_days() {
        let t = TempDir::new("sync-trash");
        let root = t.path("root");
        fs::create_dir_all(root.join("d")).unwrap();
        prepare_own_dir(&root).unwrap();
        let now = 1_794_837_780_000;
        fs::write(root.join("d/a.txt"), b"1").unwrap();
        let first = move_to_trash(&root, "d/a.txt", now).unwrap();
        fs::write(root.join("d/a.txt"), b"2").unwrap();
        let second = move_to_trash(&root, "d/a.txt", now).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"1");
        assert_eq!(fs::read(&second).unwrap(), b"2");
        assert!(second.ends_with("a (2).txt"));
        assert_eq!(trash_usage(&root), (2, 2));
        fs::create_dir_all(own_dir(&root).join(TRASH).join("2026-01-01")).unwrap();
        fs::create_dir_all(own_dir(&root).join(TRASH).join("keep-me")).unwrap();
        assert_eq!(purge_trash(&root, now), 1);
        assert!(own_dir(&root).join(TRASH).join("keep-me").exists());
        assert!(first.exists());
    }

    #[test]
    fn inspecting_a_folder_counts_without_its_own_folder() {
        let t = TempDir::new("sync-inspect");
        let root = t.path("root");
        fs::create_dir_all(root.join("x")).unwrap();
        fs::write(root.join("x/a"), b"12345").unwrap();
        prepare_own_dir(&root).unwrap();
        fs::write(own_dir(&root).join(TRASH).join("t"), b"t").unwrap();
        let f = inspect(&root);
        assert!(f.exists && f.writable && !f.more && !f.unreadable);
        assert_eq!((f.files, f.bytes), (1, 5));
        assert!(f.free_bytes.is_some_and(|b| b > 0));
        assert!(!inspect(&t.path("none")).exists);
    }

    #[test]
    fn ids_are_unique() {
        assert_ne!(unique_id("a"), unique_id("a"));
        assert_eq!(unique_id("x").len(), 24);
    }
}

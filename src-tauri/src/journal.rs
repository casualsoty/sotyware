//! The operation journal (ADR-030): what sotyware was doing when it last
//! stopped, so a crash, a power loss or a killed process leaves a record
//! instead of silence.
//!
//! An entry is written **before** the work it describes can change remote
//! objects or local files, updated as it goes, and removed once its outcome
//! is confirmed. What is left at startup belongs to a session that ended
//! unexpectedly (or to work that ended in a state that needs the user's
//! attention, such as a partial move):
//!
//! - **uploads**: account, bucket, key, local source path, size and SHA-256
//!   once hashed, and the multipart upload id as soon as the provider
//!   returns it;
//! - **downloads**: the hidden temporary file (`.sotyware-partial`);
//! - **staging folders** of several-item downloads;
//! - **file operations**: kind, account, bucket and every planned step
//!   (source, planned ETag, destination) with its progress.
//!
//! At startup [`Journal::clean_local`] removes only sotyware's own
//! temporary files and staging folders recorded here (checked against
//! sotyware's naming pattern); nothing remote is touched and no operation
//! is repeated. Everything else is shown in the recovery view
//! (`cloud::recovery`), where each entry is inspected against the provider
//! before the user decides.
//!
//! A journal that cannot be opened (damaged, newer schema) is left
//! untouched, like every other database: transfers and operations still
//! work, and the dashboard says that interrupted work cannot be recovered.
//! A write that fails on an open journal stops the step it was meant to
//! record, because an unrecorded step could not be recovered.
//!
//! Only `storage.rs` knows SQLite; this module holds the policy behind the
//! [`JournalStore`] trait. Nothing here is ever synced: local paths stay on
//! this device.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::features::StoreError;

/// What an entry describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Upload,
    Download,
    Staging,
    Operation,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Staging => "staging",
            Self::Operation => "operation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "upload" => Self::Upload,
            "download" => Self::Download,
            "staging" => Self::Staging,
            "operation" => Self::Operation,
            _ => return None,
        })
    }
}

/// Where an entry stands. Entries that reach a confirmed end are removed,
/// so only these remain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryState {
    /// The work started; nothing more is known.
    Started,
    /// An upload's multipart upload is open at the provider (`upload_id`).
    MultipartOpen,
    /// An upload ended after its commit point without verification: the
    /// provider may hold the object.
    NeedsInspection,
    /// An upload's multipart upload could not be aborted: its parts may
    /// remain at the provider.
    IncompleteParts,
    /// A file operation ended with some steps applied and others not (for
    /// example a move whose originals could not all be deleted).
    Partial,
}

impl EntryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::MultipartOpen => "multipart-open",
            Self::NeedsInspection => "needs-inspection",
            Self::IncompleteParts => "incomplete-parts",
            Self::Partial => "partial",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "started" => Self::Started,
            "multipart-open" => Self::MultipartOpen,
            "needs-inspection" => Self::NeedsInspection,
            "incomplete-parts" => Self::IncompleteParts,
            "partial" => Self::Partial,
            _ => return None,
        })
    }
}

/// One journal entry as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryRow {
    pub id: i64,
    pub kind: String,
    pub state: String,
    pub account_id: Option<String>,
    pub bucket: Option<String>,
    pub object_key: Option<String>,
    /// Upload source, download temporary file or staging folder. Local to
    /// this device; never shown to the page, never synced.
    pub local_path: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub multipart: bool,
    pub upload_id: Option<String>,
    /// `copy`, `move`, `rename`, `delete`, `restore` for operations.
    pub operation: Option<String>,
    pub started_ms: i64,
    pub updated_ms: i64,
}

/// One planned step of a file operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRow {
    pub seq: i64,
    pub source_key: String,
    pub source_etag: Option<String>,
    pub dest_key: Option<String>,
    /// `planned`, `copied`, `finished`, `failed`, `skipped`,
    /// `original-kept`.
    pub phase: String,
}

/// Where journal entries live. Implemented by `storage::SqliteJournal`.
pub trait JournalStore: Send {
    fn insert(&mut self, entry: &EntryRow) -> Result<i64, StoreError>;
    fn update(&mut self, entry: &EntryRow) -> Result<(), StoreError>;
    fn remove(&mut self, id: i64) -> Result<(), StoreError>;
    fn entries(&mut self) -> Result<Vec<EntryRow>, StoreError>;
    fn insert_steps(&mut self, id: i64, steps: &[StepRow]) -> Result<(), StoreError>;
    fn set_phases(&mut self, id: i64, phases: &[(i64, &str)]) -> Result<(), StoreError>;
    fn steps(&mut self, id: i64) -> Result<Vec<StepRow>, StoreError>;
}

/// The journal: managed state, shared by the transfer and operation slots.
pub struct Journal {
    inner: Mutex<Inner>,
}

enum Inner {
    Open(Box<dyn JournalStore>),
    /// Why the journal cannot be used (shown by the dashboard).
    Unavailable(String),
}

/// Milliseconds since the Unix epoch (wall clock: only for display and
/// ordering; nothing depends on it being monotonic).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Journal {
    /// The journal of `run()`: the opened store, or why it cannot be used.
    pub fn load(store: Result<Box<dyn JournalStore>, StoreError>) -> Arc<Self> {
        let inner = match store {
            Ok(store) => Inner::Open(store),
            Err(e) => {
                eprintln!("sotyware: the operation journal cannot be used: {e}");
                Inner::Unavailable(e.to_string())
            }
        };
        Arc::new(Self {
            inner: Mutex::new(inner),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Why the journal cannot be used, if it cannot.
    pub fn unavailable(&self) -> Option<String> {
        match &*self.lock() {
            Inner::Open(_) => None,
            Inner::Unavailable(reason) => Some(reason.clone()),
        }
    }

    /// Runs `f` on the open store; `Ok(None)` when the journal is
    /// unavailable (work goes on unrecorded, the dashboard says so).
    fn with<T>(
        &self,
        f: impl FnOnce(&mut dyn JournalStore) -> Result<T, StoreError>,
    ) -> Result<Option<T>, StoreError> {
        match &mut *self.lock() {
            Inner::Open(store) => f(store.as_mut()).map(Some),
            Inner::Unavailable(_) => Ok(None),
        }
    }

    /// Records the start of some work; `None` when the journal is
    /// unavailable.
    pub fn begin(&self, mut entry: EntryRow) -> Result<Option<i64>, StoreError> {
        let now = now_ms();
        entry.started_ms = now;
        entry.updated_ms = now;
        self.with(|s| s.insert(&entry))
    }

    /// Changes an entry with `f`.
    pub fn change(&self, id: i64, f: impl FnOnce(&mut EntryRow)) -> Result<(), StoreError> {
        self.with(|s| {
            let Some(mut entry) = s.entries()?.into_iter().find(|e| e.id == id) else {
                return Ok(());
            };
            f(&mut entry);
            entry.updated_ms = now_ms();
            s.update(&entry)
        })
        .map(drop)
    }

    /// Removes an entry whose outcome is confirmed.
    pub fn finish(&self, id: i64) -> Result<(), StoreError> {
        self.with(|s| s.remove(id)).map(drop)
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> Result<Vec<EntryRow>, StoreError> {
        self.with(|s| s.entries()).map(Option::unwrap_or_default)
    }

    /// One entry.
    pub fn entry(&self, id: i64) -> Result<Option<EntryRow>, StoreError> {
        Ok(self.entries()?.into_iter().find(|e| e.id == id))
    }

    pub fn add_steps(&self, id: i64, steps: &[StepRow]) -> Result<(), StoreError> {
        self.with(|s| s.insert_steps(id, steps)).map(drop)
    }

    pub fn set_phases(&self, id: i64, phases: &[(i64, &str)]) -> Result<(), StoreError> {
        if phases.is_empty() {
            return Ok(());
        }
        self.with(|s| s.set_phases(id, phases)).map(drop)
    }

    pub fn steps(&self, id: i64) -> Result<Vec<StepRow>, StoreError> {
        self.with(|s| s.steps(id)).map(Option::unwrap_or_default)
    }

    /// Removes every upload entry for this multipart upload id, once the
    /// provider confirmed it is gone (aborted later, on retry or quit).
    pub fn forget_upload_id(&self, upload_id: &str) -> Result<(), StoreError> {
        self.with(|s| {
            for e in s.entries()? {
                if e.upload_id.as_deref() == Some(upload_id) {
                    s.remove(e.id)?;
                }
            }
            Ok(())
        })
        .map(drop)
    }

    /// At startup, before anything else runs: removes sotyware's own
    /// temporary download files and staging folders that an earlier
    /// session recorded and could not remove, and their entries. Only paths
    /// whose last component has sotyware's naming pattern are touched; a
    /// path that does not exist any more is simply forgotten. Returns how
    /// many were removed.
    pub fn clean_local(&self) -> usize {
        let Ok(entries) = self.entries() else {
            return 0;
        };
        let mut removed = 0;
        for e in entries {
            let kind = EntryKind::parse(&e.kind);
            let Some(path) = e.local_path.as_deref().map(PathBuf::from) else {
                continue;
            };
            let outcome = match kind {
                Some(EntryKind::Download) if is_partial_name(&path) => remove_file(&path),
                Some(EntryKind::Staging) if is_staging_name(&path) => remove_staging(&path),
                Some(EntryKind::Download | EntryKind::Staging) => {
                    // Never written by sotyware; kept for the recovery view
                    // instead of touching a path that may be the user's.
                    eprintln!(
                        "sotyware: a journal entry names a temporary path that is not sotyware's; it was left alone"
                    );
                    continue;
                }
                _ => continue,
            };
            match outcome {
                Ok(was_there) => {
                    removed += usize::from(was_there);
                    if let Err(err) = self.finish(e.id) {
                        eprintln!("sotyware: cannot update the operation journal: {err}");
                    }
                }
                Err(err) => {
                    eprintln!(
                        "sotyware: cannot remove a temporary file left by an earlier session: {err}"
                    );
                }
            }
        }
        removed
    }
}

/// `.<name>.<pid>-<n>.sotyware-partial` (see `cloud::s3::partial_name`).
pub fn is_partial_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') && n.ends_with(".sotyware-partial") && n.len() > 18)
}

/// `.sotyware-download-<pid>-<n>` (see `cloud::bulk_download`).
pub fn is_staging_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(".sotyware-download-"))
        .is_some_and(|rest| {
            let mut parts = rest.split('-');
            matches!(
                (parts.next(), parts.next(), parts.next()),
                (Some(a), Some(b), None)
                    if !a.is_empty() && !b.is_empty()
                        && a.chars().all(|c| c.is_ascii_digit())
                        && b.chars().all(|c| c.is_ascii_digit())
            )
        })
}

fn remove_file(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => std::fs::remove_file(path).map(|()| true),
        // Not a regular file (a link or a folder someone put there): not
        // sotyware's temporary file; leave it.
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn remove_staging(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        // A real folder, not a link to one: its content is only what
        // sotyware downloaded there and had not moved into place.
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// A link between one running piece of work and its journal entry, handed
/// to the code that performs the steps.
#[derive(Clone)]
pub struct JournalLink {
    pub journal: Arc<Journal>,
    pub id: i64,
}

impl std::fmt::Debug for JournalLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalLink").field("id", &self.id).finish()
    }
}

impl JournalLink {
    /// Runs `f` on the blocking pool (SQLite does file I/O).
    pub async fn change(
        &self,
        f: impl FnOnce(&mut EntryRow) + Send + 'static,
    ) -> Result<(), StoreError> {
        let (journal, id) = (Arc::clone(&self.journal), self.id);
        tokio::task::spawn_blocking(move || journal.change(id, f))
            .await
            .map_err(|e| StoreError::new(format!("journal task failed: {e}")))?
    }

    pub async fn finish(&self) -> Result<(), StoreError> {
        let (journal, id) = (Arc::clone(&self.journal), self.id);
        tokio::task::spawn_blocking(move || journal.finish(id))
            .await
            .map_err(|e| StoreError::new(format!("journal task failed: {e}")))?
    }
}

/// Records `entry` on the blocking pool: the link, `None` when the journal
/// is unavailable, or the write error.
pub async fn begin(
    journal: &Arc<Journal>,
    entry: EntryRow,
) -> Result<Option<JournalLink>, StoreError> {
    let j = Arc::clone(journal);
    let id = tokio::task::spawn_blocking(move || j.begin(entry))
        .await
        .map_err(|e| StoreError::new(format!("journal task failed: {e}")))??;
    Ok(id.map(|id| JournalLink {
        journal: Arc::clone(journal),
        id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteJournal;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "sotyware-journal-{name}-{}-{}",
                std::process::id(),
                now_ms()
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open(dir: &Dir) -> Arc<Journal> {
        Journal::load(
            SqliteJournal::open(&dir.0.join("j.sqlite3"))
                .map(|s| Box::new(s) as Box<dyn JournalStore>),
        )
    }

    fn upload_entry() -> EntryRow {
        EntryRow {
            kind: "upload".into(),
            state: "started".into(),
            account_id: Some("photos".into()),
            bucket: Some("b".into()),
            object_key: Some("a/b.jpg".into()),
            local_path: Some("/home/u/b.jpg".into()),
            size: Some(5),
            ..EntryRow::default()
        }
    }

    #[test]
    fn entries_survive_a_restart_until_finished() {
        let dir = Dir::new("restart");
        let id = {
            let j = open(&dir);
            let id = j.begin(upload_entry()).unwrap().unwrap();
            j.change(id, |e| {
                e.state = "multipart-open".into();
                e.multipart = true;
                e.upload_id = Some("up-1".into());
                e.sha256 = Some("ab".repeat(32));
            })
            .unwrap();
            id
        };
        // "Restart": a new journal on the same file still has it.
        let j = open(&dir);
        let e = j.entry(id).unwrap().unwrap();
        assert_eq!(e.state, "multipart-open");
        assert_eq!(e.upload_id.as_deref(), Some("up-1"));
        assert_eq!(e.object_key.as_deref(), Some("a/b.jpg"));
        assert!(e.started_ms > 0 && e.updated_ms >= e.started_ms);
        j.forget_upload_id("up-1").unwrap();
        assert_eq!(j.entries().unwrap(), Vec::new());
    }

    #[test]
    fn operation_steps_are_recorded_and_updated() {
        let dir = Dir::new("steps");
        let j = open(&dir);
        let id = j
            .begin(EntryRow {
                kind: "operation".into(),
                state: "started".into(),
                operation: Some("move".into()),
                ..EntryRow::default()
            })
            .unwrap()
            .unwrap();
        let steps: Vec<StepRow> = (0..3)
            .map(|i| StepRow {
                seq: i,
                source_key: format!("a/{i}"),
                source_etag: Some(format!("\"e{i}\"")),
                dest_key: Some(format!("b/{i}")),
                phase: "planned".into(),
            })
            .collect();
        j.add_steps(id, &steps).unwrap();
        j.set_phases(id, &[(0, "finished"), (1, "copied")]).unwrap();
        let got = j.steps(id).unwrap();
        assert_eq!(
            got.iter().map(|s| s.phase.as_str()).collect::<Vec<_>>(),
            ["finished", "copied", "planned"]
        );
        j.finish(id).unwrap();
        assert!(j.steps(id).unwrap().is_empty(), "steps go with the entry");
    }

    #[test]
    fn an_unusable_journal_is_reported_and_work_goes_on_unrecorded() {
        let dir = Dir::new("bad");
        let path = dir.0.join("j.sqlite3");
        std::fs::write(&path, b"not a database at all, just text").unwrap();
        let j =
            Journal::load(SqliteJournal::open(&path).map(|s| Box::new(s) as Box<dyn JournalStore>));
        assert!(j.unavailable().is_some());
        assert_eq!(j.begin(upload_entry()).unwrap(), None);
        assert_eq!(j.entries().unwrap(), Vec::new());
        // The damaged file is left exactly as it was.
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"not a database at all, just text"
        );
    }

    #[test]
    fn startup_cleanup_removes_only_sotywares_own_temporary_paths() {
        let dir = Dir::new("clean");
        let j = open(&dir);
        let partial = dir.0.join(".photo.jpg.4242-0.sotyware-partial");
        std::fs::write(&partial, b"half").unwrap();
        let staging = dir.0.join(".sotyware-download-4242-3");
        std::fs::create_dir_all(staging.join("item")).unwrap();
        std::fs::write(staging.join("item/f"), b"x").unwrap();
        let foreign = dir.0.join("photo.jpg");
        std::fs::write(&foreign, b"user data").unwrap();
        let gone = dir.0.join(".gone.txt.1-1.sotyware-partial");
        for (kind, path) in [
            ("download", &partial),
            ("staging", &staging),
            ("download", &foreign),
            ("download", &gone),
        ] {
            j.begin(EntryRow {
                kind: kind.into(),
                state: "started".into(),
                local_path: Some(path.display().to_string()),
                ..EntryRow::default()
            })
            .unwrap();
        }
        // An upload entry is never touched by the cleanup.
        j.begin(upload_entry()).unwrap();
        assert_eq!(j.clean_local(), 2);
        assert!(!partial.exists());
        assert!(!staging.exists());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"user data");
        let left: Vec<_> = j.entries().unwrap().into_iter().map(|e| e.kind).collect();
        // The foreign path's entry stays for the user to see; the upload too.
        assert_eq!(left, ["download", "upload"]);
    }

    #[test]
    fn temporary_name_patterns_are_strict() {
        assert!(is_partial_name(Path::new(
            "/x/.a.txt.12-0.sotyware-partial"
        )));
        assert!(!is_partial_name(Path::new("/x/a.txt")));
        assert!(!is_partial_name(Path::new("/x/.sotyware-partial")));
        assert!(is_staging_name(Path::new("/x/.sotyware-download-12-0")));
        assert!(!is_staging_name(Path::new("/x/.sotyware-download-12")));
        assert!(!is_staging_name(Path::new("/x/.sotyware-download-12-0-x")));
        assert!(!is_staging_name(Path::new("/x/Downloads")));
    }
}

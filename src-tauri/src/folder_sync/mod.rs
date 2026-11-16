//! Folder sync (ADR-043): keeps a folder on this computer and a storage
//! account's bucket in step, in both directions, in the background.
//!
//! - [`model`]: the pure rules (paths and names, exclusions, the decision
//!   for one path from its synced, local and remote state, conflict copy
//!   names, the deletion guard, retries).
//! - `scan`: the local side (walking, stamps, hashes, the folder's marker,
//!   sotyware's own folder with staging and trash, free space).
//! - `engine`: one sync's task (passes, listing, planning, actions).
//! - `watch`: file change notifications (desktop).
//! - `migrate`: moving a sync folder somewhere else.
//! - This module: [`FolderSyncs`], the managed state (configurations and
//!   synced state in `folder-sync.sqlite3`, one engine per running sync,
//!   the live status the page shows), and the commands of the Sync tab.
//!
//! The page chooses a folder only through the native folder dialog Rust
//! opens; it gets the chosen path to show (and confirm) and never sends a
//! path back: every later command names the sync by its id, or the choice
//! by a token kept in Rust.
//!
//! Turning sync off forgets the configuration and the synced state and
//! removes the marker; it never deletes a file in the folder or an object
//! in the bucket. The engines run while the built-in feature `folder-sync`
//! runs (on by default).
//!
//! Design: docs/architecture/folder-sync.md.

mod engine;
mod migrate;
pub mod model;
mod scan;
#[cfg(desktop)]
mod watch;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::Notify;

use crate::cloud::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking_pub};
use crate::cloud::s3::{TransferControl, Versioning};
use crate::features::StoreError;
use crate::journal::{Journal, now_ms};
use crate::storage::SqliteSyncStore;

use engine::ConflictItem;

/// How long quitting waits for the engines to stop.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
/// How long a folder choice stays valid.
const CHOICE_TTL: Duration = Duration::from_secs(30 * 60);
/// Activity items the page gets.
const ACTIVITY_SHOWN: usize = 200;

// ------------------------------------------------------------------ types

/// One sync as stored (JSON in `folder-sync.sqlite3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    pub id: String,
    pub account: String,
    pub bucket: String,
    /// The account's endpoint when sync was set up: a sync never follows an
    /// account to another endpoint (its synced state would not fit).
    pub endpoint: String,
    /// The part of the bucket synced: `""` for all of it (a folder of the
    /// bucket would be `photos/`; not offered yet).
    #[serde(default)]
    pub prefix: String,
    /// The sync folder (absolute, UTF-8).
    pub path: String,
    /// The id written into the folder's marker.
    pub marker: String,
    #[serde(default)]
    pub paused: bool,
    pub interval_minutes: u32,
    pub created_ms: i64,
    /// The bucket's versioning when sync was set up.
    #[serde(default)]
    pub versioning: Option<String>,
    #[serde(default)]
    pub last_synced_ms: Option<i64>,
}

/// What a sync is doing, as the page shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncState {
    /// Configured, its engine about to start.
    Starting,
    /// Looking for changes (the folder or the bucket).
    Scanning,
    /// Transferring or applying changes.
    Syncing,
    /// In step.
    Synced,
    /// In step except for what waits: failures to retry, held deletions,
    /// a full drive.
    Attention,
    /// Paused by the user.
    Paused,
    /// The provider cannot be reached; tried again automatically.
    Offline,
    /// The folder (or its drive) is not there; sync goes on when it is.
    Unavailable,
    /// Something the user must fix (credentials, a changed account, a
    /// folder that is not this sync's any more).
    Error,
    /// The folder is being moved.
    Moving,
    /// Not running: folder sync is switched off in Settings › Features.
    Stopped,
}

/// Deletions the guard held (`issues`, kind `deletions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldDeletions {
    /// Files that would be removed here (deleted in the bucket).
    pub local: u64,
    /// Objects that would be deleted in the bucket (deleted here).
    pub remote: u64,
    pub synced: u64,
    pub asked_ms: i64,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub restore: bool,
}

/// A path that failed (`issues`, kind `failed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedItem {
    pub op: String,
    pub kind: ErrorKind,
    pub message: String,
    pub attempts: u32,
    pub next_ms: i64,
    pub first_ms: i64,
}

/// One completed operation, as the Activity list shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub at_ms: i64,
    pub path: String,
    /// `upload`, `download`, `delete-remote`, `delete-local`,
    /// `move-remote`, `move-local`, `conflict`, `conflict-copy`,
    /// `create-folder`.
    pub op: String,
    pub bytes: u64,
    pub message: Option<String>,
}

/// Which way a transfer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferKind {
    Upload,
    Download,
}

/// A running transfer.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInfo {
    #[serde(skip)]
    pub key: u64,
    pub path: String,
    pub kind: TransferKind,
    pub bytes: u64,
    pub total: u64,
    pub started_ms: i64,
    /// Bytes per second since it started, once it has run a second.
    pub speed: Option<u64>,
}

/// Progress of moving a sync folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationInfo {
    pub to: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Copying between drives (else renaming within one).
    pub copying: bool,
}

/// One sync as the page sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub id: String,
    pub account: String,
    pub bucket: String,
    /// The sync folder, to show (never sent back by the page).
    pub path: String,
    pub state: SyncState,
    pub message: Option<String>,
    pub error: Option<CommandError>,
    pub paused: bool,
    pub interval_minutes: u32,
    pub versioning: Option<String>,
    /// Changes in the folder are noticed as they happen.
    pub watching: bool,
    /// Synced files, their bytes on this computer, synced folders.
    pub files: u64,
    pub bytes: u64,
    pub folders: u64,
    /// What the last complete listing found in the bucket (current
    /// versions only).
    pub remote_files: u64,
    pub remote_bytes: u64,
    pub pending_uploads: u64,
    pub pending_upload_bytes: u64,
    pub pending_downloads: u64,
    pub pending_download_bytes: u64,
    pub failed: u64,
    pub skipped: u64,
    pub conflicts: u64,
    pub held_deletions: Option<HeldDeletions>,
    pub trash_files: u64,
    pub trash_bytes: u64,
    pub free_bytes: Option<u64>,
    pub disk_full: bool,
    pub last_synced_ms: Option<i64>,
    pub last_checked_ms: Option<i64>,
    pub next_check_ms: Option<i64>,
    pub transfers: Vec<TransferInfo>,
    pub migration: Option<MigrationInfo>,
    /// Grows when an activity item is added (the page reloads the list).
    pub activity_seq: u64,
}

impl SyncStatus {
    fn of(c: &SyncConfig, running: bool) -> Self {
        Self {
            id: c.id.clone(),
            account: c.account.clone(),
            bucket: c.bucket.clone(),
            path: c.path.clone(),
            state: if c.paused {
                SyncState::Paused
            } else if running {
                SyncState::Starting
            } else {
                SyncState::Stopped
            },
            message: None,
            error: None,
            paused: c.paused,
            interval_minutes: c.interval_minutes,
            versioning: c.versioning.clone(),
            watching: false,
            files: 0,
            bytes: 0,
            folders: 0,
            remote_files: 0,
            remote_bytes: 0,
            pending_uploads: 0,
            pending_upload_bytes: 0,
            pending_downloads: 0,
            pending_download_bytes: 0,
            failed: 0,
            skipped: 0,
            conflicts: 0,
            held_deletions: None,
            trash_files: 0,
            trash_bytes: 0,
            free_bytes: None,
            disk_full: false,
            last_synced_ms: c.last_synced_ms,
            last_checked_ms: None,
            next_check_ms: None,
            transfers: Vec::new(),
            migration: None,
            activity_seq: 0,
        }
    }
}

/// Every sync, for the page and the sidebar.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOverview {
    pub seq: u64,
    /// The `folder-sync` feature runs.
    pub running: bool,
    pub storage_error: Option<String>,
    pub syncs: Vec<SyncStatus>,
}

/// A folder the user chose in the dialog, waiting for confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderChoice {
    pub token: String,
    pub account: String,
    /// The sync whose folder changes, for a change.
    pub sync: Option<String>,
    /// The chosen folder, to show.
    pub path: String,
    pub files: u64,
    pub bytes: u64,
    /// The count stopped at its limit.
    pub more: bool,
    pub free_bytes: Option<u64>,
    /// The bucket's size as last measured on the Usage & costs tab.
    pub bucket_bytes: Option<u64>,
    pub bucket_objects: Option<u64>,
    pub bucket_measured_ms: Option<i64>,
    /// The folder was a sync folder before.
    pub was_sync_folder: bool,
    /// `enabled`, `suspended`, `disabled`, or `None` (could not be read).
    pub versioning: Option<String>,
    /// Why this folder cannot be used (the page cannot confirm then).
    pub problems: Vec<String>,
    /// What the user should know before confirming.
    pub warnings: Vec<String>,
    /// For a change: the files can be moved there (it is empty).
    pub can_move: bool,
}

struct Choice {
    info: FolderChoice,
    canonical: PathBuf,
    created: Instant,
}

/// Activity, problems and conflicts of one sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDetails {
    pub activity: Vec<ActivityItem>,
    pub failed: Vec<FailedEntry>,
    pub skipped: Vec<SkippedEntry>,
    pub conflicts: Vec<ConflictEntry>,
    pub held_deletions: Option<HeldDeletions>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedEntry {
    pub path: String,
    #[serde(flatten)]
    pub item: FailedItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    pub path: String,
    pub reason: String,
    /// `local` (in the folder) or `remote` (in the bucket).
    pub side: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictEntry {
    pub path: String,
    pub copy: String,
    pub at_ms: i64,
}

/// What turning sync off left behind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisableReport {
    pub path: String,
    /// The sync trash still holds files (in `.sotyware-sync/trash`).
    pub trash_kept: bool,
}

/// How a conflict is settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum ConflictChoice {
    /// This computer's version stays; the conflicted copy goes (to the
    /// sync trash here, and from the bucket).
    #[serde(rename = "keep-local")]
    Local,
    /// The bucket's version (the conflicted copy) takes the original name;
    /// this computer's version goes to the sync trash.
    #[serde(rename = "keep-remote")]
    Remote,
    /// Both stay; the conflict is dismissed.
    #[serde(rename = "keep-both")]
    Both,
}

/// What changing the folder does with the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveMode {
    /// Move everything to the new folder (renamed on one drive, copied and
    /// verified between drives, then removed from the old one), then sync
    /// on there.
    Move,
    /// Leave the old folder as it is (no longer synced) and sync the new
    /// one from scratch: its files and the bucket are merged, nothing is
    /// deleted.
    Fresh,
}

/// Where `open_sync_folder` opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    Folder,
    Trash,
}

/// `n` bytes for messages: `1.5 GB`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} bytes")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

// ---------------------------------------------------------- engine control

/// The paths the watcher named since the last pass.
#[derive(Default)]
struct Dirty {
    paths: BTreeSet<String>,
    full: bool,
    first: Option<Instant>,
    last: Option<Instant>,
}

/// One engine's switches, shared with its watcher.
pub(crate) struct Control {
    pub stop: AtomicBool,
    /// The engine's task has ended.
    ended: AtomicBool,
    pub wake: Notify,
    dirty: Mutex<Dirty>,
    force_remote: AtomicBool,
    transfers: Mutex<HashMap<u64, Arc<TransferControl>>>,
    next: AtomicU64,
}

impl Control {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stop: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            wake: Notify::new(),
            dirty: Mutex::new(Dirty::default()),
            force_remote: AtomicBool::new(false),
            transfers: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        })
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn dirty(&self) -> MutexGuard<'_, Dirty> {
        self.dirty.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The watcher saw `rel` change.
    pub fn mark_dirty(&self, rel: String) {
        let mut d = self.dirty();
        let now = Instant::now();
        d.first.get_or_insert(now);
        d.last = Some(now);
        if d.paths.len() < engine::DIRTY_MAX {
            d.paths.insert(rel);
        } else {
            d.full = true;
        }
    }

    /// The watcher may have missed changes: the next pass scans everything.
    pub fn mark_full(&self) {
        {
            let mut d = self.dirty();
            let now = Instant::now();
            d.first.get_or_insert(now);
            d.last = Some(now);
            d.full = true;
        }
        self.wake.notify_one();
    }

    /// The paths to look at again, and whether to scan everything.
    pub fn take_dirty(&self) -> (Vec<String>, bool) {
        let mut d = self.dirty();
        let out = (std::mem::take(&mut d.paths).into_iter().collect(), d.full);
        d.full = false;
        d.first = None;
        d.last = None;
        out
    }

    /// When the named changes have settled.
    pub fn dirty_deadline(&self) -> Option<Instant> {
        let d = self.dirty();
        let (first, last) = (d.first?, d.last?);
        Some((last + engine::DEBOUNCE).min(first + engine::DEBOUNCE_MAX))
    }

    pub fn take_force_remote(&self) -> bool {
        self.force_remote.swap(false, Ordering::SeqCst)
    }

    pub fn register(&self, control: Arc<TransferControl>) -> u64 {
        let key = self.next.fetch_add(1, Ordering::Relaxed);
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, control);
        key
    }

    pub fn unregister(&self, key: u64) {
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
    }

    pub fn cancel_transfers(&self) {
        for c in self
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            c.cancel();
        }
    }

    fn halt(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.cancel_transfers();
        self.wake.notify_one();
    }
}

/// What the engines and the service share.
pub(crate) struct Shared {
    store: Mutex<Result<SqliteSyncStore, String>>,
    pub accounts: Arc<CloudAccounts>,
    pub journal: OnceLock<Arc<Journal>>,
    statuses: Mutex<BTreeMap<String, SyncStatus>>,
    changed: Notify,
    seq: AtomicU64,
    #[cfg(test)]
    pub free_space_override: Mutex<Option<u64>>,
}

impl Shared {
    fn store(&self) -> MutexGuard<'_, Result<SqliteSyncStore, String>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` on the store, on this thread (blocking: call it from the
    /// blocking pool).
    pub fn with_store<T>(
        &self,
        f: impl FnOnce(&mut SqliteSyncStore) -> Result<T, StoreError>,
    ) -> Result<T, CommandError> {
        match &mut *self.store() {
            Ok(s) => f(s).map_err(store_error),
            Err(e) => Err(store_error(e)),
        }
    }

    /// Runs `f` on the store on the blocking pool.
    pub async fn db<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&mut SqliteSyncStore) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, CommandError> {
        let this = Arc::clone(self);
        blocking_pub(move || this.with_store(f)).await
    }

    /// The configuration of sync `id`, `None` if it is gone.
    pub async fn config(self: &Arc<Self>, id: &str) -> Option<SyncConfig> {
        let id = id.to_owned();
        self.db(move |s| {
            Ok(s.configs()?
                .into_iter()
                .find(|(i, _)| *i == id)
                .and_then(|(_, json)| serde_json::from_str(&json).ok()))
        })
        .await
        .ok()
        .flatten()
    }

    /// Changes the live status of sync `id` and tells the page.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut SyncStatus)) {
        if let Some(s) = self
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
        {
            f(s);
        }
        self.seq.fetch_add(1, Ordering::Relaxed);
        self.changed.notify_one();
    }

    pub fn transfer_start(&self, id: &str, path: &str, kind: TransferKind, total: u64) -> u64 {
        let key = self.seq.fetch_add(1, Ordering::Relaxed);
        self.update(id, |s| {
            s.transfers.push(TransferInfo {
                key,
                path: path.to_owned(),
                kind,
                bytes: 0,
                total,
                started_ms: now_ms(),
                speed: None,
            });
        });
        key
    }

    pub fn transfer_progress(&self, id: &str, key: u64, bytes: u64) {
        self.update(id, |s| {
            if let Some(t) = s.transfers.iter_mut().find(|t| t.key == key) {
                t.bytes = bytes;
                let secs = (now_ms() - t.started_ms) / 1000;
                t.speed = (secs >= 1).then(|| bytes / u64::try_from(secs).unwrap_or(1).max(1));
            }
        });
    }

    pub fn transfer_end(&self, id: &str, key: u64) {
        self.update(id, |s| s.transfers.retain(|t| t.key != key));
    }

    /// Free space on the drive of `path` (tests can pretend).
    pub fn free_space(&self, path: &Path) -> Option<u64> {
        #[cfg(test)]
        if let Some(n) = *self
            .free_space_override
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            return Some(n);
        }
        scan::free_space(path)
    }
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(
        ErrorKind::Storage,
        format!("the folder sync database cannot be used: {e}"),
    )
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

fn unknown_sync() -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, "this sync does not exist any more")
}

// ------------------------------------------------------------ the service

/// Receives the overview whenever it changes.
pub trait OverviewSink: Send + Sync {
    /// `false` when the receiver is gone.
    fn send(&self, overview: &SyncOverview) -> bool;
}

impl OverviewSink for Channel<SyncOverview> {
    fn send(&self, overview: &SyncOverview) -> bool {
        Channel::send(self, overview.clone()).is_ok()
    }
}

struct Engines {
    running: HashMap<String, Arc<Control>>,
    /// Engine tasks still alive (also stopped ones that are finishing).
    alive: usize,
}

/// Folder sync: managed state.
pub struct FolderSyncs {
    shared: Arc<Shared>,
    engines: Mutex<Engines>,
    ended: Condvar,
    /// The `folder-sync` feature runs.
    on: AtomicBool,
    closed: AtomicBool,
    choices: Mutex<HashMap<String, Choice>>,
    subscribers: Mutex<Vec<(u64, Arc<dyn OverviewSink>)>>,
    notifier: Mutex<bool>,
    next: AtomicU64,
    /// sotyware's data folder: never a sync folder.
    data_dir: Option<PathBuf>,
    self_ref: OnceLock<Weak<FolderSyncs>>,
}

/// The link the `folder-sync` feature and the other cloud commands use;
/// set once in `run()`.
pub static LINK: OnceLock<Arc<FolderSyncs>> = OnceLock::new();

/// Tells the syncs of `account` that sotyware itself changed its bucket
/// (an upload, a file operation): they list it again now instead of at
/// their next interval.
pub fn bucket_changed(account: &str) {
    if let Some(syncs) = LINK.get() {
        syncs.nudge(account);
    }
}

/// The storage account `account` was removed: its sync is turned off.
pub fn account_removed(account: &str) {
    if let Some(syncs) = LINK.get() {
        syncs.account_removed(account);
    }
}

impl FolderSyncs {
    /// The service of `run()`.
    pub fn load(
        store: Result<SqliteSyncStore, StoreError>,
        accounts: Arc<CloudAccounts>,
        data_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        let store = store.map_err(|e| {
            eprintln!("sotyware: the folder sync database cannot be used: {e}");
            e.to_string()
        });
        let mut statuses = BTreeMap::new();
        let this = Arc::new(Self {
            shared: Arc::new(Shared {
                store: Mutex::new(store),
                accounts,
                journal: OnceLock::new(),
                statuses: Mutex::new(BTreeMap::new()),
                changed: Notify::new(),
                seq: AtomicU64::new(1),
                #[cfg(test)]
                free_space_override: Mutex::new(None),
            }),
            engines: Mutex::new(Engines {
                running: HashMap::new(),
                alive: 0,
            }),
            ended: Condvar::new(),
            on: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            choices: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            notifier: Mutex::new(false),
            next: AtomicU64::new(1),
            data_dir,
            self_ref: OnceLock::new(),
        });
        let _ = this.self_ref.set(Arc::downgrade(&this));
        for c in this.configs().unwrap_or_default() {
            statuses.insert(c.id.clone(), SyncStatus::of(&c, false));
        }
        *this
            .shared
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = statuses;
        this
    }

    /// Makes the engines' transfers record themselves in `journal`. Once.
    pub fn record_in(&self, journal: Arc<Journal>) {
        let _ = self.shared.journal.set(journal);
    }

    fn configs(&self) -> Result<Vec<SyncConfig>, CommandError> {
        let rows = self.shared.with_store(|s| s.configs())?;
        Ok(rows
            .into_iter()
            .filter_map(
                |(id, json)| match serde_json::from_str::<SyncConfig>(&json) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        eprintln!("sotyware: folder sync '{id}' cannot be read: {e}");
                        None
                    }
                },
            )
            .collect())
    }

    fn config(&self, id: &str) -> Result<SyncConfig, CommandError> {
        self.configs()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(unknown_sync)
    }

    fn save(&self, c: &SyncConfig) -> Result<(), CommandError> {
        let json = serde_json::to_string(c)
            .map_err(|e| CommandError::new(ErrorKind::Other, e.to_string()))?;
        self.shared.with_store(|s| s.save_config(&c.id, &json))
    }

    fn engines(&self) -> MutexGuard<'_, Engines> {
        self.engines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every sync and the live status (blocking only on a short lock).
    pub fn overview(&self) -> SyncOverview {
        let storage_error = match &*self.shared.store() {
            Ok(_) => None,
            Err(e) => Some(e.clone()),
        };
        SyncOverview {
            seq: self.shared.seq.load(Ordering::Relaxed),
            running: self.on.load(Ordering::SeqCst),
            storage_error,
            syncs: self
                .shared
                .statuses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .cloned()
                .collect(),
        }
    }

    // ------------------------------------------------------- engines

    /// The feature started: every sync that is not paused runs.
    pub fn start_all(&self) {
        self.on.store(true, Ordering::SeqCst);
        for c in self.configs().unwrap_or_default() {
            if !c.paused {
                self.start_engine(&c.id);
            }
        }
        self.shared.changed.notify_one();
    }

    /// The feature stopped: every engine stops (its transfers are
    /// cancelled); the configurations stay.
    pub fn stop_all(&self) {
        self.on.store(false, Ordering::SeqCst);
        let ids: Vec<String> = self.engines().running.keys().cloned().collect();
        for id in ids {
            let _ = self.stop_engine(&id);
        }
        self.shared.changed.notify_one();
    }

    fn start_engine(&self, id: &str) {
        if !self.on.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return;
        }
        let control = {
            let mut e = self.engines();
            if e.running.contains_key(id) {
                return;
            }
            let control = Control::new();
            e.running.insert(id.to_owned(), Arc::clone(&control));
            e.alive += 1;
            control
        };
        self.shared.update(id, |s| {
            s.state = SyncState::Starting;
            s.message = None;
            s.error = None;
        });
        let shared = Arc::clone(&self.shared);
        let me = self.self_ref.get().cloned();
        let id = id.to_owned();
        tauri::async_runtime::spawn(async move {
            engine::run(shared, id, Arc::clone(&control)).await;
            control.ended.store(true, Ordering::SeqCst);
            if let Some(me) = me.and_then(|w| w.upgrade()) {
                me.engine_ended();
            }
        });
    }

    fn engine_ended(&self) {
        let mut e = self.engines();
        e.alive = e.alive.saturating_sub(1);
        self.ended.notify_all();
    }

    /// Asks the engine of `id` to stop; returns its control to wait on.
    fn stop_engine(&self, id: &str) -> Option<Arc<Control>> {
        let control = self.engines().running.remove(id)?;
        control.halt();
        Some(control)
    }

    /// Waits (bounded, blocking) until the engine of `control` has ended.
    fn wait_engine(&self, control: &Control, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut e = self.engines();
        while !control.ended.load(Ordering::SeqCst) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            e = self
                .ended
                .wait_timeout(e, left.min(Duration::from_millis(100)))
                .map_or_else(|p| p.into_inner().0, |(g, _)| g);
        }
        true
    }

    /// Waits (bounded) until no engine task is alive.
    fn wait_engines(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut e = self.engines();
        while e.alive > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            e = self
                .ended
                .wait_timeout(e, left)
                .map_or_else(|p| p.into_inner().0, |(g, _)| g);
        }
        true
    }

    /// Stops one engine and waits for it, off the async threads.
    async fn stop_and_wait(self: &Arc<Self>, id: &str) {
        let Some(control) = self.stop_engine(id) else {
            return;
        };
        let this = Arc::clone(self);
        let _ = tokio::task::spawn_blocking(move || {
            if !this.wait_engine(&control, SHUTDOWN_WAIT) {
                eprintln!("sotyware: a folder sync did not stop in time");
            }
        })
        .await;
    }

    /// The storage account `account` was removed: its sync is turned off
    /// (the folder's files and the bucket stay as they are). Blocking,
    /// bounded.
    pub fn account_removed(&self, account: &str) {
        for c in self.configs().unwrap_or_default() {
            if c.account != account {
                continue;
            }
            if let Some(control) = self.stop_engine(&c.id) {
                let _ = self.wait_engine(&control, SHUTDOWN_WAIT);
            }
            let root = PathBuf::from(&c.path);
            if matches!(scan::read_marker(&root), Ok(Some(m)) if m.sync == c.marker) {
                let _ = scan::remove_marker(&root);
            }
            if let Err(e) = self.shared.with_store(|s| s.remove_sync(&c.id)) {
                eprintln!(
                    "sotyware: the sync of a removed account could not be forgotten: {}",
                    e.message
                );
            }
            self.shared
                .statuses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&c.id);
            self.shared.changed.notify_one();
        }
    }

    /// Quitting: every engine stops; waits at most `wait`.
    pub fn shutdown(&self, wait: Duration) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop_all();
        if !self.wait_engines(wait) {
            eprintln!("sotyware: a folder sync did not stop in time while quitting");
        }
    }

    fn control(&self, id: &str) -> Option<Arc<Control>> {
        self.engines().running.get(id).cloned()
    }

    /// sotyware changed the bucket of `account`: its syncs list it now.
    /// Memory only (no database): it is called from the async threads.
    pub fn nudge(&self, account: &str) {
        let ids: Vec<String> = self
            .shared
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|s| s.account == account)
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            if let Some(control) = self.control(&id) {
                control.force_remote.store(true, Ordering::SeqCst);
                control.wake.notify_one();
            }
        }
    }

    // --------------------------------------------------- subscriptions

    /// Sends the overview to `sink` now and whenever it changes (at most
    /// four times a second). Returns the subscription id.
    pub fn subscribe(self: &Arc<Self>, sink: Arc<dyn OverviewSink>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        sink.send(&self.overview());
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, sink));
        let mut started = self.notifier.lock().unwrap_or_else(|e| e.into_inner());
        if !*started {
            *started = true;
            let weak = Arc::downgrade(self);
            let shared = Arc::clone(&self.shared);
            tauri::async_runtime::spawn(async move {
                loop {
                    shared.changed.notified().await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let Some(this) = weak.upgrade() else { return };
                    let overview = this.overview();
                    this.subscribers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .retain(|(_, s)| s.send(&overview));
                }
            });
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(i, _)| *i != id);
    }

    // ------------------------------------------------------- choosing

    /// Checks the folder `path` the user chose (in the dialog) for a new
    /// sync of `account` (`sync: None`) or as the new folder of `sync`,
    /// and keeps it behind a token until it is confirmed or discarded.
    #[allow(clippy::too_many_arguments)]
    pub fn choose(
        &self,
        account: &str,
        sync: Option<&str>,
        path: &Path,
        versioning: Option<Versioning>,
        bucket: Option<(u64, u64, i64)>,
    ) -> Result<FolderChoice, CommandError> {
        let configs = self.configs()?;
        let current = match sync {
            Some(id) => Some(
                configs
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                    .ok_or_else(unknown_sync)?,
            ),
            None => {
                if configs.iter().any(|c| c.account == account) {
                    return Err(invalid(
                        "this account's bucket is synced already; change its folder instead",
                    ));
                }
                None
            }
        };
        let shown = path.to_str().map(str::to_owned);
        let canonical = scan::canonical(path).unwrap_or_else(|_| path.to_path_buf());
        let mut problems = Vec::new();
        let mut warnings = Vec::new();
        if shown.is_none() {
            problems
                .push("The folder's path is not valid Unicode; choose another folder.".to_owned());
        }
        if canonical.parent().is_none() {
            problems.push(
                "A whole drive cannot be the sync folder; choose or create a folder on it."
                    .to_owned(),
            );
        }
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .and_then(|h| scan::canonical(&h).ok());
        if home.as_deref() == Some(canonical.as_path()) {
            problems.push(
                "Your whole user folder cannot be the sync folder; choose or create a folder in it."
                    .to_owned(),
            );
        }
        if let Some(data) = &self.data_dir {
            let data = scan::canonical(data).unwrap_or_else(|_| data.clone());
            if scan::within(&canonical, &data) || scan::within(&data, &canonical) {
                problems.push("sotyware's own data folder cannot be synced.".to_owned());
            }
        }
        for other in &configs {
            if Some(&other.id) == current.as_ref().map(|c| &c.id) {
                continue;
            }
            let o = scan::canonical(Path::new(&other.path))
                .unwrap_or_else(|_| PathBuf::from(&other.path));
            if scan::within(&canonical, &o) || scan::within(&o, &canonical) {
                problems.push(format!(
                    "This folder is (or contains, or lies inside) the sync folder of the bucket {}; two syncs cannot share files.",
                    other.bucket
                ));
            }
        }
        // Only a folder that may be used is looked into (never a whole
        // drive, nor sotyware's or another sync's files).
        let facts = if problems.is_empty() {
            scan::inspect(path)
        } else {
            scan::Facts {
                exists: path.is_dir(),
                ..scan::Facts::default()
            }
        };
        if !facts.exists {
            problems.push("The folder does not exist.".to_owned());
        } else if problems.is_empty() && !facts.writable {
            problems.push(
                "sotyware may not create files in this folder (permissions, or a read-only drive)."
                    .to_owned(),
            );
        }
        let mut can_move = false;
        if let Some(c) = &current {
            let old =
                scan::canonical(Path::new(&c.path)).unwrap_or_else(|_| PathBuf::from(&c.path));
            if old == canonical {
                if scan::root_state(&old, &c.marker) == scan::RootState::Ready {
                    problems.push("This is the sync folder already.".to_owned());
                } else {
                    // Its marker is gone (or it is another disk now): used
                    // again from scratch, never with the old synced state.
                    warnings.push("sotyware syncs this folder again from scratch: its files and the bucket are merged, and nothing is deleted because of the earlier sync.".to_owned());
                }
            } else if scan::within(&canonical, &old) {
                problems
                    .push("The new folder cannot be inside the current sync folder.".to_owned());
            } else if scan::within(&old, &canonical) {
                problems.push("The new folder cannot contain the current sync folder.".to_owned());
            }
            can_move = old != canonical && facts.exists && facts.files == 0 && !facts.more;
        }
        if facts.marker.is_some() {
            warnings.push("This folder was a sync folder before. Its files are compared with the bucket again; nothing is deleted because of that earlier sync.".to_owned());
        }
        if facts.files > 0 || facts.more {
            warnings.push(format!(
                "The folder already holds {}{} files ({}). They are merged with the bucket: files only here are uploaded, files only in the bucket are downloaded, and a file that differs on both sides is kept twice (the bucket's version as a conflicted copy).",
                facts.files,
                if facts.more { "+" } else { "" },
                bytes(facts.bytes)
            ));
        }
        if facts.unreadable {
            warnings.push("Some folders inside it cannot be read; their files are not synced until they can be.".to_owned());
        }
        match versioning {
            Some(Versioning::Enabled) => {}
            Some(Versioning::Suspended | Versioning::Disabled) => warnings.push("The bucket does not keep earlier versions: a file deleted or changed in the sync folder is deleted or replaced in the bucket for good.".to_owned()),
            None => warnings.push("The bucket's versioning could not be read; if it does not keep earlier versions, deleting or changing a file here deletes or replaces it in the bucket for good.".to_owned()),
        }
        if let (Some(free), Some((b, _, _))) = (facts.free_bytes, bucket)
            && free < b.saturating_add(256 * 1024 * 1024)
        {
            warnings.push(format!(
                "The drive has {} free, and the bucket held {} when it was last measured: not everything may fit. Downloads stop with a message when the drive is full.",
                bytes(free),
                bytes(b)
            ));
        }
        if cfg!(windows) && shown.as_deref().is_some_and(|s| s.starts_with(r"\\")) {
            warnings.push(
                "This is a network folder: changes may be noticed later than on a local drive."
                    .to_owned(),
            );
        }
        let token = format!("choice-{}", scan::unique_id(account));
        let info = FolderChoice {
            token: token.clone(),
            account: account.to_owned(),
            sync: sync.map(str::to_owned),
            path: shown.unwrap_or_else(|| path.to_string_lossy().into_owned()),
            files: facts.files,
            bytes: facts.bytes,
            more: facts.more,
            free_bytes: facts.free_bytes,
            bucket_bytes: bucket.map(|b| b.0),
            bucket_objects: bucket.map(|b| b.1),
            bucket_measured_ms: bucket.map(|b| b.2),
            was_sync_folder: facts.marker.is_some(),
            versioning: versioning.map(|v| {
                match v {
                    Versioning::Enabled => "enabled",
                    Versioning::Suspended => "suspended",
                    Versioning::Disabled => "disabled",
                }
                .to_owned()
            }),
            problems,
            warnings,
            can_move,
        };
        let mut choices = self.choices.lock().unwrap_or_else(|e| e.into_inner());
        choices.retain(|_, c| c.created.elapsed() < CHOICE_TTL);
        choices.insert(
            token,
            Choice {
                info: info.clone(),
                canonical,
                created: Instant::now(),
            },
        );
        Ok(info)
    }

    pub fn discard_choice(&self, token: &str) {
        self.choices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token);
    }

    fn take_choice(&self, token: &str) -> Result<Choice, CommandError> {
        let choice = self
            .choices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token)
            .filter(|c| c.created.elapsed() < CHOICE_TTL)
            .ok_or_else(|| invalid("this folder choice expired; choose the folder again"))?;
        if !choice.info.problems.is_empty() {
            return Err(invalid(choice.info.problems.join(" ")));
        }
        Ok(choice)
    }

    // ------------------------------------------------------- changes

    /// Starts syncing the chosen folder with its account's bucket.
    pub fn enable(&self, token: &str, interval_minutes: u32) -> Result<SyncStatus, CommandError> {
        if !model::Interval::valid(interval_minutes) {
            return Err(invalid("choose one of the offered intervals"));
        }
        let choice = self.take_choice(token)?;
        if choice.info.sync.is_some() {
            return Err(invalid("this choice is for changing a sync's folder"));
        }
        let account = choice.info.account.clone();
        if self.configs()?.iter().any(|c| c.account == account) {
            return Err(invalid("this account's bucket is synced already"));
        }
        let (_, endpoint) = self.shared.accounts.provider_and_endpoint(&account)?;
        let bucket = self
            .shared
            .accounts
            .list()
            .accounts
            .into_iter()
            .find(|a| a.id == account)
            .and_then(|a| a.bucket)
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidConfiguration,
                    "no bucket is configured for this account",
                )
            })?;
        let path = choice.canonical.clone();
        let path_text = path
            .to_str()
            .ok_or_else(|| invalid("the folder's path is not valid Unicode"))?
            .to_owned();
        let marker = scan::unique_id(&path_text);
        let config = SyncConfig {
            id: format!("{account}-{}", &scan::unique_id(&account)[..8]),
            account: account.clone(),
            bucket: bucket.clone(),
            endpoint,
            prefix: String::new(),
            path: path_text,
            marker: marker.clone(),
            paused: false,
            interval_minutes,
            created_ms: now_ms(),
            versioning: choice.info.versioning.clone(),
            last_synced_ms: None,
        };
        scan::write_marker(
            &path,
            &scan::Marker {
                format: 1,
                sync: marker,
                account,
                bucket,
                created_ms: config.created_ms,
            },
        )
        .map_err(|e| {
            CommandError::new(
                ErrorKind::LocalPermission,
                format!("cannot mark the folder as the sync folder: {e}"),
            )
        })?;
        self.save(&config)?;
        let status = SyncStatus::of(&config, self.on.load(Ordering::SeqCst));
        self.shared
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(config.id.clone(), status.clone());
        self.start_engine(&config.id);
        self.shared.changed.notify_one();
        Ok(status)
    }

    /// Pauses (every transfer is cancelled) or resumes a sync.
    pub async fn set_paused(self: &Arc<Self>, id: &str, paused: bool) -> Result<(), CommandError> {
        let mut c = self.config(id)?;
        c.paused = paused;
        self.save(&c)?;
        self.shared.update(id, |s| {
            s.paused = paused;
            s.state = if paused {
                SyncState::Paused
            } else {
                SyncState::Starting
            };
            s.message = None;
        });
        if paused {
            self.stop_and_wait(id).await;
            self.shared.update(id, |s| {
                s.state = SyncState::Paused;
                s.transfers.clear();
            });
        } else {
            self.start_engine(id);
            if !self.on.load(Ordering::SeqCst) {
                self.shared.update(id, |s| s.state = SyncState::Stopped);
            }
        }
        Ok(())
    }

    /// Checks the folder and the bucket now (and retries what failed).
    pub fn sync_now(&self, id: &str) -> Result<(), CommandError> {
        self.config(id)?;
        let Some(control) = self.control(id) else {
            return Err(CommandError::new(
                ErrorKind::Busy,
                if self.on.load(Ordering::SeqCst) {
                    "this sync is paused; resume it first"
                } else {
                    "folder sync is switched off in Settings › Features"
                },
            ));
        };
        self.retry(id)?;
        control.force_remote.store(true, Ordering::SeqCst);
        control.mark_full();
        Ok(())
    }

    /// Retries every failed item now.
    pub fn retry(&self, id: &str) -> Result<(), CommandError> {
        let now = now_ms();
        self.shared.with_store(|s| {
            for i in s.issues(id)? {
                if i.kind == "failed"
                    && let Ok(mut f) = serde_json::from_str::<FailedItem>(&i.data)
                {
                    f.next_ms = now;
                    s.put_issue(
                        id,
                        "failed",
                        &i.path,
                        &serde_json::to_string(&f).unwrap_or_default(),
                        now,
                    )?;
                }
            }
            Ok(())
        })?;
        if let Some(c) = self.control(id) {
            c.wake.notify_one();
        }
        Ok(())
    }

    /// Changes how often the bucket is checked.
    pub fn set_interval(&self, id: &str, minutes: u32) -> Result<(), CommandError> {
        if !model::Interval::valid(minutes) {
            return Err(invalid("choose one of the offered intervals"));
        }
        let mut c = self.config(id)?;
        c.interval_minutes = minutes;
        self.save(&c)?;
        self.shared.update(id, |s| s.interval_minutes = minutes);
        // The next check follows the new interval from now on.
        if let Some(control) = self.control(id) {
            control.force_remote.store(true, Ordering::SeqCst);
            control.wake.notify_one();
        }
        Ok(())
    }

    /// Turns a sync off: forgets it here and removes the folder's marker.
    /// No file and no object is deleted.
    pub async fn disable(self: &Arc<Self>, id: &str) -> Result<DisableReport, CommandError> {
        let c = self.config(id)?;
        self.stop_and_wait(id).await;
        let root = PathBuf::from(&c.path);
        let trash_kept = {
            let root = root.clone();
            let marker = c.marker.clone();
            blocking_pub(move || {
                // Only our own marker is removed (another sync's stays).
                Ok(match scan::read_marker(&root) {
                    Ok(Some(m)) if m.sync == marker => scan::remove_marker(&root).unwrap_or(false),
                    _ => false,
                })
            })
            .await?
        };
        self.shared.with_store(|s| s.remove_sync(id))?;
        self.shared
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.shared.changed.notify_one();
        Ok(DisableReport {
            path: c.path,
            trash_kept,
        })
    }

    /// Moves a sync to the folder chosen with `token`.
    pub async fn change_folder(
        self: &Arc<Self>,
        token: &str,
        mode: MoveMode,
    ) -> Result<(), CommandError> {
        let choice = self.take_choice(token)?;
        let Some(id) = choice.info.sync.clone() else {
            return Err(invalid("this choice is for a new sync"));
        };
        let config = self.config(&id)?;
        if mode == MoveMode::Move && !choice.info.can_move {
            return Err(invalid(
                "the files can only be moved into an empty folder; choose an empty one, or sync the new folder from scratch",
            ));
        }
        let was_running = self.control(&id).is_some();
        self.stop_and_wait(&id).await;
        self.shared.update(&id, |s| {
            s.state = SyncState::Moving;
            s.message = Some(match mode {
                MoveMode::Move => "Moving the files to the new folder…".into(),
                MoveMode::Fresh => "Switching to the new folder…".into(),
            });
            s.transfers.clear();
        });
        let shared = Arc::clone(&self.shared);
        let target = choice.canonical.clone();
        let result = blocking_pub(move || migrate::run(&shared, &config, &target, mode)).await;
        let config = self.config(&id)?;
        self.shared.update(&id, |s| {
            s.path = config.path.clone();
            s.migration = None;
            s.state = SyncState::Starting;
            s.message = result.as_ref().err().map(|e| e.message.clone());
        });
        if was_running || !config.paused {
            self.start_engine(&id);
        }
        result
    }

    /// Opens the sync folder (or its trash) in the system's file manager.
    pub fn open(&self, id: &str, place: Place) -> Result<(), CommandError> {
        let c = self.config(id)?;
        let root = PathBuf::from(&c.path);
        let target = match place {
            Place::Folder => root,
            Place::Trash => scan::own_dir(&root).join(scan::TRASH),
        };
        if !target.is_dir() {
            return Err(CommandError::new(
                ErrorKind::DestinationUnavailable,
                match place {
                    Place::Folder => "the sync folder is not available (is its drive connected?)",
                    Place::Trash => "the sync trash is empty",
                },
            ));
        }
        open_in_file_manager(&target)
    }

    /// Activity, problems and conflicts.
    pub fn details(&self, id: &str) -> Result<SyncDetails, CommandError> {
        self.config(id)?;
        let (activity, issues) = self
            .shared
            .with_store(|s| Ok((s.activity(id, ACTIVITY_SHOWN)?, s.issues(id)?)))?;
        let mut d = SyncDetails {
            activity: activity
                .into_iter()
                .filter_map(|(_, json)| serde_json::from_str(&json).ok())
                .collect(),
            failed: Vec::new(),
            skipped: Vec::new(),
            conflicts: Vec::new(),
            held_deletions: None,
        };
        for i in issues {
            match i.kind.as_str() {
                "failed" => {
                    if let Ok(item) = serde_json::from_str(&i.data) {
                        d.failed.push(FailedEntry { path: i.path, item });
                    }
                }
                "skipped-local" | "skipped-remote" | "skipped-case" => {
                    if let Ok(s) = serde_json::from_str::<engine::SkippedItem>(&i.data) {
                        d.skipped.push(SkippedEntry {
                            path: i.path,
                            reason: s.reason,
                            side: if i.kind == "skipped-local" {
                                "local".into()
                            } else {
                                "remote".into()
                            },
                        });
                    }
                }
                "conflict" => {
                    if let Ok(c) = serde_json::from_str::<ConflictItem>(&i.data)
                        && c.state == "kept-both"
                    {
                        d.conflicts.push(ConflictEntry {
                            path: i.path,
                            copy: c.copy,
                            at_ms: c.at_ms,
                        });
                    }
                }
                "deletions" => {
                    d.held_deletions = serde_json::from_str::<HeldDeletions>(&i.data)
                        .ok()
                        .filter(|h| !h.approved);
                }
                _ => {}
            }
        }
        Ok(d)
    }

    /// Settles a conflict the engine kept both versions of.
    pub fn resolve_conflict(
        &self,
        id: &str,
        path: &str,
        choice: ConflictChoice,
    ) -> Result<(), CommandError> {
        let c = self.config(id)?;
        let item = self
            .shared
            .with_store(|s| {
                Ok(s.issues(id)?
                    .into_iter()
                    .find(|i| i.kind == "conflict" && i.path == path)
                    .and_then(|i| serde_json::from_str::<ConflictItem>(&i.data).ok()))
            })?
            .filter(|i| i.state == "kept-both")
            .ok_or_else(|| invalid("this conflict was settled already"))?;
        let root = PathBuf::from(&c.path);
        let (original, copy) = (scan::join(&root, path), scan::join(&root, &item.copy));
        let now = now_ms();
        let local_error = |e: std::io::Error| {
            CommandError::new(
                ErrorKind::LocalFile,
                format!("the files could not be changed: {e}"),
            )
        };
        match choice {
            ConflictChoice::Both => {}
            ConflictChoice::Local => {
                if copy.symlink_metadata().is_ok() {
                    scan::prepare_own_dir(&root).map_err(local_error)?;
                    scan::move_to_trash(&root, &item.copy, now).map_err(local_error)?;
                }
            }
            ConflictChoice::Remote => {
                if copy.symlink_metadata().is_err() {
                    return Err(invalid(
                        "the conflicted copy is not in the sync folder any more",
                    ));
                }
                scan::prepare_own_dir(&root).map_err(local_error)?;
                if original.symlink_metadata().is_ok() {
                    scan::move_to_trash(&root, path, now).map_err(local_error)?;
                }
                std::fs::rename(&copy, &original).map_err(local_error)?;
            }
        }
        self.shared
            .with_store(|s| s.remove_issue(id, "conflict", path))?;
        if let Some(control) = self.control(id) {
            control.mark_dirty(path.to_owned());
            control.mark_dirty(item.copy.clone());
            control.wake.notify_one();
        }
        self.shared
            .update(id, |s| s.conflicts = s.conflicts.saturating_sub(1));
        Ok(())
    }

    /// Answers the deletion guard: `allow` runs the held deletions,
    /// otherwise the deleted files are copied back where they are missing.
    pub fn answer_deletions(&self, id: &str, allow: bool) -> Result<(), CommandError> {
        self.config(id)?;
        let now = now_ms();
        self.shared.with_store(|s| {
            let Some(row) = s.issues(id)?.into_iter().find(|i| i.kind == "deletions") else {
                return Ok(());
            };
            let Ok(mut h) = serde_json::from_str::<HeldDeletions>(&row.data) else {
                return Ok(());
            };
            h.approved = allow;
            h.restore = !allow;
            s.put_issue(
                id,
                "deletions",
                "",
                &serde_json::to_string(&h).unwrap_or_default(),
                now,
            )
        })?;
        self.shared.update(id, |s| s.held_deletions = None);
        if let Some(control) = self.control(id) {
            control.wake.notify_one();
        }
        Ok(())
    }
}

/// Opens `path` in the system's file manager (Explorer, Finder, the
/// desktop's default), through the official opener plugin's function,
/// called from Rust only.
#[cfg(desktop)]
fn open_in_file_manager(path: &Path) -> Result<(), CommandError> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| {
        CommandError::new(
            ErrorKind::Other,
            format!("the folder could not be opened: {e}"),
        )
    })
}

#[cfg(not(desktop))]
fn open_in_file_manager(_: &Path) -> Result<(), CommandError> {
    Err(CommandError::new(
        ErrorKind::Other,
        "opening folders is not available here",
    ))
}

// ---------------------------------------------------------------- commands

#[cfg(desktop)]
async fn pick_folder(window: &tauri::WebviewWindow, title: String) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(title)
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    match rx.await.ok().flatten()?.into_path() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("sotyware: the folder dialog returned no folder path: {e}");
            None
        }
    }
}

#[cfg(not(desktop))]
async fn pick_folder(_: &tauri::WebviewWindow, _: String) -> Option<PathBuf> {
    None
}

fn syncs<'a>(state: &'a State<'_, Arc<FolderSyncs>>) -> &'a Arc<FolderSyncs> {
    state.inner()
}

/// Every sync and its live status. Allowed by `allow-list-folder-syncs`.
#[tauri::command]
pub fn list_folder_syncs(state: State<'_, Arc<FolderSyncs>>) -> SyncOverview {
    syncs(&state).overview()
}

/// Sends the overview on `on_event` now and whenever it changes. Allowed
/// by `allow-watch-folder-syncs`.
#[tauri::command]
pub fn watch_folder_syncs(
    state: State<'_, Arc<FolderSyncs>>,
    on_event: Channel<SyncOverview>,
) -> u64 {
    syncs(&state).subscribe(Arc::new(on_event))
}

/// Stops the updates of a `watch_folder_syncs`. Allowed by
/// `allow-unwatch-folder-syncs`.
#[tauri::command]
pub fn unwatch_folder_syncs(state: State<'_, Arc<FolderSyncs>>, subscription: u64) {
    syncs(&state).unsubscribe(subscription);
}

/// Opens the native folder dialog for a new sync of `account` (or a new
/// folder for `sync`) and returns what sotyware found there, to confirm;
/// `null` if the dialog was cancelled. Allowed by `allow-choose-sync-folder`.
#[tauri::command]
pub async fn choose_sync_folder(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<FolderSyncs>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    usage: State<'_, Arc<crate::cloud::usage::StorageUsage>>,
    account: String,
    sync: Option<String>,
) -> Result<Option<FolderChoice>, CommandError> {
    let label = accounts
        .list()
        .accounts
        .into_iter()
        .find(|a| a.id == account)
        .ok_or_else(|| CommandError::new(ErrorKind::UnknownAccount, "no such account"))?;
    let bucket = label.bucket.clone().ok_or_else(|| {
        CommandError::new(
            ErrorKind::InvalidConfiguration,
            "no bucket is configured for this account",
        )
    })?;
    let title = if sync.is_some() {
        format!("Choose the new sync folder for {bucket}")
    } else {
        format!("Choose the folder to sync with {bucket}")
    };
    let Some(path) = pick_folder(&window, title).await else {
        return Ok(None);
    };
    choose_with(
        Arc::clone(state.inner()),
        Arc::clone(accounts.inner()),
        Some(Arc::clone(usage.inner())),
        account,
        sync,
        path,
    )
    .await
    .map(Some)
}

/// `choose_sync_folder` after the dialog (tests call it with a path).
pub(crate) async fn choose_with(
    syncs: Arc<FolderSyncs>,
    accounts: Arc<CloudAccounts>,
    usage: Option<Arc<crate::cloud::usage::StorageUsage>>,
    account: String,
    sync: Option<String>,
    path: PathBuf,
) -> Result<FolderChoice, CommandError> {
    let bucket = accounts
        .list()
        .accounts
        .into_iter()
        .find(|a| a.id == account)
        .and_then(|a| a.bucket)
        .unwrap_or_default();
    // One request: whether the bucket keeps versions (never fatal).
    let versioning = {
        let (acc, account, b) = (Arc::clone(&accounts), account.clone(), bucket.clone());
        match blocking_pub(move || acc.prepare_download(&account, &b)).await {
            Ok(store) => store.bucket_versioning(&bucket).await.ok(),
            Err(_) => None,
        }
    };
    let measured = {
        let account = account.clone();
        blocking_pub(move || {
            Ok(usage
                .and_then(|u| u.latest(&account))
                .map(|m| (m.bytes, m.objects, m.measured_ms)))
        })
        .await?
    };
    blocking_pub(move || syncs.choose(&account, sync.as_deref(), &path, versioning, measured)).await
}

/// Forgets a folder choice. Allowed by `allow-discard-sync-folder-choice`.
#[tauri::command]
pub fn discard_sync_folder_choice(state: State<'_, Arc<FolderSyncs>>, token: String) {
    syncs(&state).discard_choice(&token);
}

/// Starts syncing the chosen folder. Allowed by `allow-enable-folder-sync`.
#[tauri::command]
pub async fn enable_folder_sync(
    state: State<'_, Arc<FolderSyncs>>,
    token: String,
    interval_minutes: u32,
) -> Result<SyncStatus, CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.enable(&token, interval_minutes)).await
}

/// Pauses or resumes a sync. Allowed by `allow-set-folder-sync-paused`.
#[tauri::command]
pub async fn set_folder_sync_paused(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
    paused: bool,
) -> Result<(), CommandError> {
    Arc::clone(state.inner()).set_paused(&id, paused).await
}

/// Checks the folder and the bucket now. Allowed by `allow-sync-folder-now`.
#[tauri::command]
pub async fn sync_folder_now(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.sync_now(&id)).await
}

/// Retries what failed now. Allowed by `allow-retry-folder-sync`.
#[tauri::command]
pub async fn retry_folder_sync(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.retry(&id)).await
}

/// Changes how often the bucket is checked. Allowed by
/// `allow-set-folder-sync-interval`.
#[tauri::command]
pub async fn set_folder_sync_interval(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
    minutes: u32,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.set_interval(&id, minutes)).await
}

/// Turns a sync off; no file or object is deleted. Allowed by
/// `allow-disable-folder-sync`.
#[tauri::command]
pub async fn disable_folder_sync(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
) -> Result<DisableReport, CommandError> {
    Arc::clone(state.inner()).disable(&id).await
}

/// Moves a sync to the folder chosen with `token`: `move` its files there,
/// or sync the new folder from scratch (`fresh`). Allowed by
/// `allow-change-sync-folder`.
#[tauri::command]
pub async fn change_sync_folder(
    state: State<'_, Arc<FolderSyncs>>,
    token: String,
    mode: MoveMode,
) -> Result<(), CommandError> {
    Arc::clone(state.inner()).change_folder(&token, mode).await
}

/// Opens the sync folder or its trash in the file manager. Allowed by
/// `allow-open-sync-folder`.
#[tauri::command]
pub async fn open_sync_folder(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
    place: Place,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.open(&id, place)).await
}

/// Activity, problems and conflicts. Allowed by `allow-folder-sync-details`.
#[tauri::command]
pub async fn folder_sync_details(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
) -> Result<SyncDetails, CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.details(&id)).await
}

/// Settles a conflict. Allowed by `allow-resolve-folder-sync-conflict`.
#[tauri::command]
pub async fn resolve_folder_sync_conflict(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
    path: String,
    choice: ConflictChoice,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.resolve_conflict(&id, &path, choice)).await
}

/// Answers the deletion guard. Allowed by `allow-answer-folder-sync-deletions`.
#[tauri::command]
pub async fn answer_folder_sync_deletions(
    state: State<'_, Arc<FolderSyncs>>,
    id: String,
    allow: bool,
) -> Result<(), CommandError> {
    let s = Arc::clone(state.inner());
    blocking_pub(move || s.answer_deletions(&id, allow)).await
}

//! One sync's engine (ADR-043): a task that keeps one sync folder and its
//! bucket in step until it is stopped.
//!
//! **Triggers.** A pass runs at start, after file changes the watcher
//! reports (1.5 s after the last one, at most 10 s after the first), when
//! the remote check is due (the sync's interval), on "Sync now", when a
//! failed item's retry is due, and after sotyware itself changed the
//! bucket. While the folder is unavailable (its drive is not connected),
//! the engine looks again every 30 s and does nothing else.
//!
//! **A pass**, in order: the folder must be there and carry this sync's
//! marker; the account must still have the same bucket and endpoint; the
//! local side is scanned (all of it after a start, a watcher overflow or
//! an unavailable folder; else only the paths the watcher named); the
//! bucket is listed when due (all pages, or the listing is not used); then
//! planning cycles: candidates (the paths where synced state, scan and
//! listing differ) are decided by `model::decide` and run — transfers and
//! folders first, then deletions behind the guard, then folder removals,
//! deepest first — until a cycle changes nothing.
//!
//! **Safety.** Every action checks the live file and the live object right
//! before it changes anything, so a decision taken on an older scan or
//! listing can only do less: a replacement upload expects the ETag it
//! synced, a deletion in the bucket needs the local file still gone and the
//! object still unchanged, a local deletion needs the object still gone and
//! the file still unchanged (it then goes to the sync trash, not away), a
//! download moves into place only over the file it expects (or none).
//! Nothing below a folder that could not be read, and nothing from a
//! listing that did not finish, ever counts as deleted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use super::model::{self, Decision, Local, Remote, Synced, Triple};
use super::scan::{self, RootState, Stamp};
use super::{
    ActivityItem, Control, FailedItem, HeldDeletions, Shared, SyncConfig, SyncState, TransferKind,
};
use crate::cloud::dashboard::{CommandError, ErrorKind};
use crate::cloud::s3::{
    Destination, Existing, MAX_COPY_BYTES, S3Store, StorageError, TransferControl, UploadOptions,
    UploadStage,
};
use crate::journal::now_ms;

/// Candidates read per planning step.
const CHUNK: usize = 500;
/// Planning cycles per pass at most (each one runs what the previous made
/// possible: a conflict copy, a folder that is empty now).
const MAX_CYCLES: usize = 6;
/// Actions running at once (each transfer still waits for the account's
/// transfer queue).
const PARALLEL: usize = 3;
/// How long the watcher's events are left to settle before a pass.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(1500);
/// The longest a pass waits for events to stop.
pub(super) const DEBOUNCE_MAX: Duration = Duration::from_secs(10);
/// How often an unavailable folder is looked for.
const UNAVAILABLE_RECHECK: Duration = Duration::from_secs(30);
/// How long the engine waits after an error the user must fix.
const FATAL_RECHECK: Duration = Duration::from_secs(600);
/// Space a download leaves free on the drive.
const DISK_RESERVE: u64 = 256 * 1024 * 1024;
/// Paths named by the watcher before a full scan is cheaper.
pub(super) const DIRTY_MAX: usize = 5000;

/// Why a pass stopped before its end.
#[derive(Debug)]
pub(super) enum Stop {
    /// The provider could not be reached (or did not answer).
    Offline(String),
    /// Something the user has to fix; the engine waits for a change.
    Fatal(CommandError),
    /// The engine is stopping.
    Stopped,
}

/// What the loop keeps between passes.
pub(super) struct LoopState {
    pub full_local: bool,
    pub remote_due: Instant,
    pub offline_failures: u32,
    /// Folders the last full scan could not read.
    pub unknown: Vec<String>,
    pub retry_at: Option<Instant>,
    /// Old days of the sync trash were removed in this engine's life.
    pub purged: bool,
    /// Whether to watch the folder (tests that drive passes by hand do not).
    pub watch: bool,
    #[cfg(desktop)]
    pub watcher: Option<super::watch::Watcher>,
}

impl LoopState {
    pub fn new() -> Self {
        Self {
            full_local: true,
            remote_due: Instant::now(),
            offline_failures: 0,
            unknown: Vec::new(),
            retry_at: None,
            purged: false,
            watch: true,
            #[cfg(desktop)]
            watcher: None,
        }
    }

    fn drop_watcher(&mut self) {
        #[cfg(desktop)]
        {
            self.watcher = None;
        }
    }

    fn watching(&self) -> bool {
        #[cfg(desktop)]
        {
            self.watcher.is_some()
        }
        #[cfg(not(desktop))]
        {
            false
        }
    }
}

/// The engine's task: passes until `control.stop` is set.
pub(super) async fn run(shared: Arc<Shared>, id: String, control: Arc<Control>) {
    let mut st = LoopState::new();
    loop {
        if control.stopped() {
            break;
        }
        let wait = match step(&shared, &id, &control, &mut st).await {
            Some(wait) => wait,
            None => break,
        };
        if !wait.is_zero() {
            let _ = tokio::time::timeout(wait, control.wake.notified()).await;
        }
    }
    control.cancel_transfers();
    st.drop_watcher();
    shared.update(&id, |s| {
        if s.state != SyncState::Moving {
            s.state = if s.paused {
                SyncState::Paused
            } else {
                SyncState::Stopped
            };
        }
        s.transfers.clear();
        s.watching = false;
    });
}

/// One turn of the loop: a pass if the folder is ready, and how long to
/// wait for the next. `None`: the sync is gone.
pub(super) async fn step(
    shared: &Arc<Shared>,
    id: &str,
    control: &Arc<Control>,
    st: &mut LoopState,
) -> Option<Duration> {
    let config = shared.config(id).await?;
    let root = PathBuf::from(&config.path);
    let marker = config.marker.clone();
    let state = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || scan::root_state(&root, &marker))
            .await
            .unwrap_or(RootState::Missing)
    };
    match state {
        RootState::Ready => {}
        RootState::Missing => {
            st.drop_watcher();
            st.full_local = true;
            shared.update(id, |s| {
                s.state = SyncState::Unavailable;
                s.message = Some(
                    "The sync folder is not available: is its drive connected? Sync goes on when it is back."
                        .into(),
                );
                s.error = None;
                s.watching = false;
            });
            return Some(UNAVAILABLE_RECHECK);
        }
        RootState::Foreign(reason) => {
            st.drop_watcher();
            st.full_local = true;
            shared.update(id, |s| {
                s.state = SyncState::Error;
                s.message = Some(format!(
                    "Sync stopped: {reason}. Nothing was changed. Choose the folder again or turn sync off."
                ));
                s.error = Some(CommandError::new(ErrorKind::DestinationUnavailable, reason));
                s.watching = false;
            });
            return Some(FATAL_RECHECK);
        }
    }
    if !st.purged {
        st.purged = true;
        let root = root.clone();
        let removed = tokio::task::spawn_blocking(move || {
            // Downloads an earlier session left half-way, then trash days
            // older than the retention.
            scan::clear_staging(&root);
            scan::purge_trash(&root, now_ms())
        })
        .await
        .unwrap_or(0);
        if removed > 0 {
            eprintln!(
                "sotyware: removed {removed} day(s) of a sync trash older than {} days",
                model::TRASH_DAYS
            );
        }
    }
    #[cfg(desktop)]
    if st.watch && st.watcher.is_none() {
        st.watcher = super::watch::Watcher::start(&root, Arc::clone(control));
        // Changes while nothing watched are found by a full scan.
        st.full_local = true;
    }
    let watching = st.watching();
    shared.update(id, |s| s.watching = watching);
    match pass(shared, &config, control, st).await {
        Ok(()) => {
            st.offline_failures = 0;
        }
        Err(Stop::Stopped) => return Some(Duration::ZERO),
        Err(Stop::Offline(message)) => {
            st.offline_failures += 1;
            let delay = model::offline_delay(st.offline_failures);
            st.remote_due = Instant::now() + delay;
            let next = now_ms() + i64::try_from(delay.as_millis()).unwrap_or(0);
            shared.update(id, |s| {
                s.state = SyncState::Offline;
                s.message = Some(format!(
                    "The provider cannot be reached ({message}). Sync tries again automatically."
                ));
                s.next_check_ms = Some(next);
                s.transfers.clear();
            });
            return Some(delay);
        }
        Err(Stop::Fatal(e)) => {
            st.remote_due = Instant::now() + FATAL_RECHECK;
            shared.update(id, |s| {
                s.state = SyncState::Error;
                s.message = Some(e.message.clone());
                s.error = Some(e);
                s.transfers.clear();
            });
            return Some(FATAL_RECHECK);
        }
    }
    // The next trigger.
    let now = Instant::now();
    let mut next = st.remote_due;
    if let Some(r) = st.retry_at {
        next = next.min(r);
    }
    if let Some(d) = control.dirty_deadline() {
        next = next.min(d);
    }
    Some(next.saturating_duration_since(now))
}

/// One pass (see the module documentation).
pub(super) async fn pass(
    shared: &Arc<Shared>,
    config: &SyncConfig,
    control: &Arc<Control>,
    st: &mut LoopState,
) -> Result<(), Stop> {
    let id = config.id.as_str();
    let root = PathBuf::from(&config.path);
    let store = connect(shared, config).await?;
    if control.stopped() {
        return Err(Stop::Stopped);
    }
    // ---- local side
    let (dirty, full) = control.take_dirty();
    if st.full_local || full {
        shared.update(id, |s| {
            s.state = SyncState::Scanning;
            s.message = Some("Looking for changes in the sync folder…".into());
        });
        st.unknown = full_scan(shared, config, control).await?;
        st.full_local = false;
    } else if !dirty.is_empty() {
        targeted_scan(shared, config, &dirty, &mut st.unknown).await?;
    }
    // ---- remote side
    let complete = shared
        .db({
            let id = id.to_owned();
            move |s| s.remote_complete(&id)
        })
        .await
        .map_err(Stop::Fatal)?;
    if control.take_force_remote() || Instant::now() >= st.remote_due || !complete {
        shared.update(id, |s| {
            s.state = SyncState::Scanning;
            s.message = Some("Looking for changes in the bucket…".into());
        });
        list_remote(shared, config, &store, control).await?;
        st.remote_due = Instant::now() + model::Interval(config.interval_minutes).duration();
        let (checked, next) = (
            now_ms(),
            now_ms() + i64::from(config.interval_minutes) * 60_000,
        );
        shared.update(id, |s| {
            s.last_checked_ms = Some(checked);
            s.next_check_ms = Some(next);
        });
    }
    // ---- in step
    st.retry_at = None;
    let ctx = Arc::new(Ctx {
        shared: Arc::clone(shared),
        control: Arc::clone(control),
        config: config.clone(),
        root,
        store,
        claimed: Mutex::new(HashSet::new()),
        disk_full: AtomicBool::new(false),
        retry_at: Mutex::new(None),
        relist: AtomicBool::new(false),
    });
    let clean = reconcile(&ctx, &st.unknown).await?;
    st.retry_at = *ctx.retry_at.lock().unwrap_or_else(|e| e.into_inner());
    if ctx.relist.load(Ordering::SeqCst) {
        // An action found the bucket other than listed: list again soon.
        st.remote_due = st.remote_due.min(Instant::now() + Duration::from_secs(2));
    }
    finish_status(&ctx, clean).await;
    Ok(())
}

/// The account's store, after checking the account still has this sync's
/// bucket at the same endpoint.
async fn connect(shared: &Arc<Shared>, config: &SyncConfig) -> Result<Arc<S3Store>, Stop> {
    let accounts = Arc::clone(&shared.accounts);
    let (account, bucket, endpoint) = (
        config.account.clone(),
        config.bucket.clone(),
        config.endpoint.clone(),
    );
    crate::cloud::dashboard::blocking_pub(move || {
        let (_, now_endpoint) = accounts.provider_and_endpoint(&account)?;
        if now_endpoint != endpoint {
            return Err(CommandError::new(
                ErrorKind::InvalidConfiguration,
                "the storage account now points to another endpoint than when sync was set up; turn sync off and on again to sync with it",
            ));
        }
        accounts.prepare_download(&account, &bucket)
    })
    .await
    .map_err(|e| {
        Stop::Fatal(match e.kind {
            ErrorKind::OutdatedPage => CommandError::new(
                ErrorKind::InvalidConfiguration,
                "the storage account now has another bucket than when sync was set up; turn sync off and on again to sync with it",
            ),
            ErrorKind::UnknownAccount => CommandError::new(
                ErrorKind::UnknownAccount,
                "the storage account of this sync was removed",
            ),
            _ => e,
        })
    })
}

// ------------------------------------------------------------- scanning

async fn full_scan(
    shared: &Arc<Shared>,
    config: &SyncConfig,
    control: &Arc<Control>,
) -> Result<Vec<String>, Stop> {
    let (id, root, prefix) = (
        config.id.clone(),
        PathBuf::from(&config.path),
        config.prefix.clone(),
    );
    let generation = shared
        .db({
            let id = id.clone();
            move |s| s.local_begin(&id)
        })
        .await
        .map_err(Stop::Fatal)?;
    let sh = Arc::clone(shared);
    let ctl = Arc::clone(control);
    let walked = tokio::task::spawn_blocking(move || {
        let mut batch = |rows: &[Local]| -> Result<(), String> {
            sh.with_store(|s| s.put_local(&id, rows, generation))
                .map_err(|e| e.message)
        };
        let findings = scan::walk(&root, "", &prefix, &ctl.stop, &mut batch)?;
        sh.with_store(|s| {
            s.local_end(&id, generation, &findings.unknown)?;
            s.remove_issues(&id, "skipped-local")?;
            s.remove_issues(&id, "skipped-case")?;
            for (path, reason) in &findings.skipped {
                let data = serde_json::to_string(&SkippedItem {
                    reason: reason.clone(),
                })
                .unwrap_or_default();
                s.put_issue(&id, "skipped-local", path, &data, now_ms())?;
            }
            Ok(())
        })
        .map_err(|e| e.message)?;
        Ok::<_, String>(findings.unknown)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match walked {
        Ok(unknown) => Ok(unknown),
        Err(_) if control.stopped() => Err(Stop::Stopped),
        Err(e) => Err(Stop::Fatal(CommandError::new(
            ErrorKind::LocalFile,
            format!("the sync folder cannot be scanned: {e}"),
        ))),
    }
}

/// Looks again at the paths the watcher named (and folders that appeared
/// with them).
async fn targeted_scan(
    shared: &Arc<Shared>,
    config: &SyncConfig,
    dirty: &[String],
    unknown: &mut Vec<String>,
) -> Result<(), Stop> {
    let (id, root, prefix) = (
        config.id.clone(),
        PathBuf::from(&config.path),
        config.prefix.clone(),
    );
    let dirty = dirty.to_vec();
    let sh = Arc::clone(shared);
    let found = tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
        let mut unreadable = Vec::new();
        let mut done: HashSet<String> = HashSet::new();
        for path in dirty {
            if model::ignored(&path) {
                continue;
            }
            // The highest folder above `path` that the scan does not know:
            // it appeared with it (moved in, or created with its contents).
            let mut target = path.clone();
            for parent in model::parents(&path) {
                let known = sh
                    .with_store(|s| s.local(&id, parent))
                    .map_err(|e| e.message)?;
                if known.is_none() {
                    target = parent.to_owned();
                    break;
                }
            }
            if !done.insert(target.clone()) {
                continue;
            }
            match scan::look(&root, &target, &prefix) {
                Ok(Ok(Some(l))) if l.dir => {
                    sh.with_store(|s| s.set_local(&id, &target, None))
                        .map_err(|e| e.message)?;
                    let stop = AtomicBool::new(false);
                    let mut batch = |rows: &[Local]| -> Result<(), String> {
                        sh.with_store(|s| s.put_local(&id, rows, 0))
                            .map_err(|e| e.message)
                    };
                    let f = scan::walk(&root, &target, &prefix, &stop, &mut batch)?;
                    unreadable.extend(f.unknown);
                    sh.with_store(|s| {
                        for (p, reason) in &f.skipped {
                            let data = serde_json::to_string(&SkippedItem {
                                reason: reason.clone(),
                            })
                            .unwrap_or_default();
                            s.put_issue(&id, "skipped-local", p, &data, now_ms())?;
                        }
                        Ok(())
                    })
                    .map_err(|e| e.message)?;
                }
                Ok(Ok(Some(l))) => {
                    sh.with_store(|s| {
                        s.set_local(&id, &target, Some(&l))?;
                        s.remove_issue(&id, "skipped-local", &target)
                    })
                    .map_err(|e| e.message)?;
                }
                Ok(Ok(None)) => {
                    sh.with_store(|s| {
                        s.set_local(&id, &target, None)?;
                        s.remove_issue(&id, "skipped-local", &target)
                    })
                    .map_err(|e| e.message)?;
                }
                Ok(Err(reason)) => {
                    sh.with_store(|s| {
                        s.set_local(&id, &target, None)?;
                        let data =
                            serde_json::to_string(&SkippedItem { reason }).unwrap_or_default();
                        s.put_issue(&id, "skipped-local", &target, &data, now_ms())
                    })
                    .map_err(|e| e.message)?;
                }
                // Cannot be looked at now: unknown, never gone.
                Err(_) => unreadable.push(target),
            }
        }
        Ok(unreadable)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match found {
        Ok(more) => {
            unknown.extend(more);
            Ok(())
        }
        Err(e) => Err(Stop::Fatal(CommandError::new(
            ErrorKind::LocalFile,
            format!("the sync folder cannot be scanned: {e}"),
        ))),
    }
}

/// A skipped path's reason, as stored.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SkippedItem {
    pub reason: String,
}

/// Lists the bucket below the sync's prefix, every page, into the session
/// table. A listing that fails part way is abandoned: the previous complete
/// one stays (and is listed again before planning).
async fn list_remote(
    shared: &Arc<Shared>,
    config: &SyncConfig,
    store: &Arc<S3Store>,
    control: &Arc<Control>,
) -> Result<(), Stop> {
    let id = config.id.clone();
    let generation = shared
        .db({
            let id = id.clone();
            move |s| s.remote_begin(&id)
        })
        .await
        .map_err(Stop::Fatal)?;
    let mut token: Option<String> = None;
    let mut skipped: Vec<(String, String)> = Vec::new();
    loop {
        if control.stopped() {
            abandon(shared, &id, generation).await;
            return Err(Stop::Stopped);
        }
        let page = store
            .list_recursive_page(&config.bucket, &config.prefix, token.as_deref(), 1000)
            .await;
        let page = match page {
            Ok(p) => p,
            Err(e) => {
                abandon(shared, &id, generation).await;
                return Err(storage_stop(&e));
            }
        };
        let mut rows: Vec<Remote> = Vec::with_capacity(page.objects.len() + 16);
        let mut implied: HashSet<String> = HashSet::new();
        for o in &page.objects {
            let Some((rel, dir)) = model::rel_of_key(&config.prefix, &o.key) else {
                continue;
            };
            if let Some(problem) = model::path_problem(&config.prefix, &rel, dir) {
                skipped.push((rel, problem.to_owned()));
                continue;
            }
            for parent in model::parents(&rel) {
                if implied.insert(parent.to_owned()) {
                    rows.push(Remote {
                        path: parent.to_owned(),
                        dir: true,
                        marker: false,
                        ..Remote::default()
                    });
                }
            }
            rows.push(Remote {
                path: rel,
                dir,
                marker: dir,
                size: if dir { 0 } else { o.size },
                etag: if dir { None } else { o.e_tag.clone() },
                modified_ms: None,
            });
        }
        shared
            .db({
                let id = id.clone();
                move |s| s.put_remote(&id, &rows, generation)
            })
            .await
            .map_err(Stop::Fatal)?;
        match page.next_token {
            Some(next) => token = Some(next),
            None => break,
        }
    }
    shared
        .db({
            let id = id.clone();
            move |s| {
                s.remote_end(&id, generation)?;
                s.remove_issues(&id, "skipped-remote")?;
                for (path, reason) in &skipped {
                    let data = serde_json::to_string(&SkippedItem {
                        reason: format!("in the bucket: {reason}"),
                    })
                    .unwrap_or_default();
                    s.put_issue(&id, "skipped-remote", path, &data, now_ms())?;
                }
                Ok(())
            }
        })
        .await
        .map_err(Stop::Fatal)
}

async fn abandon(shared: &Arc<Shared>, id: &str, generation: i64) {
    let id = id.to_owned();
    if let Err(e) = shared.db(move |s| s.remote_abandon(&id, generation)).await {
        eprintln!(
            "sotyware: cannot drop an unfinished bucket listing: {}",
            e.message
        );
    }
}

/// What a provider error means for the whole pass.
fn storage_stop(e: &StorageError) -> Stop {
    match e {
        StorageError::Network(m) | StorageError::Timeout(m) => Stop::Offline(short(m)),
        e if e.is_transient() => Stop::Offline(e.to_string()),
        StorageError::InvalidCredentials(_) => Stop::Fatal(CommandError::new(
            ErrorKind::InvalidCredentials,
            "the provider does not accept this account's key any more (wrong, expired or deleted); edit the account",
        )),
        StorageError::AccessDenied(_) => Stop::Fatal(CommandError::new(
            ErrorKind::AccessDenied,
            "the provider refused to list the bucket with this account's key",
        )),
        StorageError::NotFound(_) => Stop::Fatal(CommandError::new(
            ErrorKind::BucketUnavailable,
            "the bucket was not found with this account's settings",
        )),
        other => Stop::Fatal(crate::cloud::operations::operation_error(other)),
    }
}

fn short(m: &str) -> String {
    let m = m.lines().next().unwrap_or(m);
    if m.len() > 160 {
        let mut end = 160;
        while !m.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &m[..end])
    } else {
        m.to_owned()
    }
}

// ------------------------------------------------------------- planning

/// One pass's context, shared by the actions running at once.
pub(super) struct Ctx {
    pub shared: Arc<Shared>,
    pub control: Arc<Control>,
    pub config: SyncConfig,
    pub root: PathBuf,
    pub store: Arc<S3Store>,
    /// Synced paths a move already took as its source.
    claimed: Mutex<HashSet<String>>,
    /// A download found the drive full: the other downloads wait.
    disk_full: AtomicBool,
    /// The earliest retry or settled file.
    retry_at: Mutex<Option<Instant>>,
    /// The bucket is not as listed: list again soon.
    relist: AtomicBool,
}

impl Ctx {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn key(&self, rel: &str, dir: bool) -> String {
        model::key_of(&self.config.prefix, rel, dir)
    }

    fn bucket(&self) -> &str {
        &self.config.bucket
    }

    fn retry_in(&self, d: Duration) {
        let at = Instant::now() + d;
        let mut r = self.retry_at.lock().unwrap_or_else(|e| e.into_inner());
        *r = Some(r.map_or(at, |old| old.min(at)));
    }

    async fn db<T: Send + 'static>(
        &self,
        f: impl FnOnce(
            &mut crate::storage::SqliteSyncStore,
            &str,
        ) -> Result<T, crate::features::StoreError>
        + Send
        + 'static,
    ) -> Result<T, Stop> {
        let id = self.config.id.clone();
        self.shared
            .db(move |s| f(s, &id))
            .await
            .map_err(Stop::Fatal)
    }
}

/// What one action did.
enum Done {
    /// Something changed (another cycle may find more to do).
    Changed,
    /// Nothing was needed, or it waits (settling, retry later).
    Waiting,
    /// It failed; recorded with its retry time.
    Failed,
}

/// Counts of what is left to transfer, for the page.
#[derive(Default)]
struct Pending {
    uploads: u64,
    upload_bytes: u64,
    downloads: u64,
    download_bytes: u64,
    deletions_local: u64,
    deletions_remote: u64,
}

impl Pending {
    fn total(&self) -> u64 {
        self.uploads + self.downloads + self.deletions_local + self.deletions_remote
    }

    fn add(&mut self, t: &Triple, d: &Decision) {
        match d {
            Decision::Upload { .. } => {
                self.uploads += 1;
                self.upload_bytes += t.local.as_ref().map_or(0, |l| l.size);
            }
            Decision::Download { .. } => {
                self.downloads += 1;
                self.download_bytes += t.remote.as_ref().map_or(0, |r| r.size);
            }
            Decision::Compare => {
                self.uploads += 1;
                self.upload_bytes += t.local.as_ref().map_or(0, |l| l.size);
            }
            Decision::DeleteLocal => self.deletions_local += 1,
            Decision::DeleteRemote => self.deletions_remote += 1,
            _ => {}
        }
    }
}

fn below(path: &str, folders: &[String]) -> bool {
    folders
        .iter()
        .any(|u| u.is_empty() || path == *u || path.starts_with(&format!("{u}/")))
}

/// What the issues say about paths, for one planning cycle.
struct Known {
    /// Failed paths and when they may be tried again.
    waiting: HashMap<String, i64>,
    /// Paths here that are not synced (their names, links, too large …).
    skipped_here: HashSet<String>,
    /// Paths in the bucket that cannot be stored here.
    skipped_there: HashSet<String>,
    held: Option<HeldDeletions>,
}

impl Known {
    fn from(issues: &[crate::storage::IssueRow]) -> Self {
        let mut k = Self {
            waiting: HashMap::new(),
            skipped_here: HashSet::new(),
            skipped_there: HashSet::new(),
            held: None,
        };
        for i in issues {
            match i.kind.as_str() {
                "failed" => {
                    if let Ok(f) = serde_json::from_str::<FailedItem>(&i.data) {
                        k.waiting.insert(i.path.clone(), f.next_ms);
                    }
                }
                "skipped-local" => {
                    k.skipped_here.insert(i.path.clone());
                }
                "skipped-remote" | "skipped-case" => {
                    k.skipped_there.insert(i.path.clone());
                }
                "deletions" => k.held = serde_json::from_str(&i.data).ok(),
                _ => {}
            }
        }
        k
    }

    /// Whether `t` is left out of planning: below a folder that could not
    /// be read, or only on a side where it is not synced.
    fn excluded(&self, t: &Triple, unknown: &[String]) -> bool {
        below(&t.path, unknown)
            || (t.remote.is_none() && self.skipped_here.contains(&t.path))
            || (t.local.is_none() && self.skipped_there.contains(&t.path))
    }
}

/// Planning cycles until one changes nothing. Returns whether the sync is
/// clean: nothing pending, failed or held.
async fn reconcile(ctx: &Arc<Ctx>, unknown: &[String]) -> Result<bool, Stop> {
    let mut left = u64::MAX;
    let mut failed_any = false;
    let mut held_any = false;
    for _ in 0..MAX_CYCLES {
        let issues = ctx.db(|s, id| s.issues(id)).await?;
        let mut known = Known::from(&issues);
        let now = now_ms();
        if let Some(h) = &known.held
            && h.restore
        {
            restore_held(ctx, unknown).await?;
            ctx.db(|s, id| s.remove_issue(id, "deletions", "")).await?;
            known.held = None;
        }
        // 1. Count, and collect the deletions for the guard.
        let mut pending = Pending::default();
        let mut after = String::new();
        loop {
            let chunk = {
                let a = after.clone();
                ctx.db(move |s, id| s.candidates(id, &a, CHUNK)).await?
            };
            let Some(last) = chunk.last() else { break };
            after = last.path.clone();
            for t in &chunk {
                if !known.excluded(t, unknown) {
                    pending.add(t, &model::decide(t));
                }
            }
        }
        left = pending.total();
        let deletions = pending.deletions_local + pending.deletions_remote;
        let synced_files = ctx.db(|s, id| s.synced_counts(id)).await?.0;
        let guarded = model::deletions_need_approval(deletions, synced_files);
        let approved = known
            .held
            .as_ref()
            .is_some_and(|h| h.approved && deletions <= h.local + h.remote);
        let run_deletions = deletions > 0 && (!guarded || approved);
        let mut held = known.held.clone();
        if guarded && !approved {
            let h = HeldDeletions {
                local: pending.deletions_local,
                remote: pending.deletions_remote,
                synced: synced_files,
                asked_ms: now,
                approved: false,
                restore: false,
            };
            let data = serde_json::to_string(&h).unwrap_or_default();
            ctx.db(move |s, id| s.put_issue(id, "deletions", "", &data, now))
                .await?;
            held = Some(h);
        } else if deletions == 0 && held.is_some() {
            ctx.db(|s, id| s.remove_issue(id, "deletions", "")).await?;
            held = None;
        }
        held_any = held.as_ref().is_some_and(|h| !h.approved);
        ctx.shared.update(ctx.id(), |s| {
            s.state = SyncState::Syncing;
            s.message = None;
            s.error = None;
            s.pending_uploads = pending.uploads;
            s.pending_upload_bytes = pending.upload_bytes;
            s.pending_downloads = pending.downloads;
            s.pending_download_bytes = pending.download_bytes;
            s.held_deletions = held.clone().filter(|h| !h.approved);
        });
        // 2. Run.
        let mut acted = 0usize;
        let mut failed = 0usize;
        let mut deletions_list: Vec<(Triple, Decision)> = Vec::new();
        let mut removals: Vec<(Triple, Decision)> = Vec::new();
        let mut after = String::new();
        loop {
            if ctx.control.stopped() {
                return Err(Stop::Stopped);
            }
            let chunk = {
                let a = after.clone();
                ctx.db(move |s, id| s.candidates(id, &a, CHUNK)).await?
            };
            let Some(last) = chunk.last() else { break };
            after = last.path.clone();
            let mut work = Vec::new();
            for t in chunk {
                if known.excluded(&t, unknown) {
                    continue;
                }
                if let Some(next) = known.waiting.get(&t.path)
                    && *next > now
                {
                    failed += 1;
                    ctx.retry_in(Duration::from_millis(
                        u64::try_from(next - now).unwrap_or(0),
                    ));
                    continue;
                }
                let d = model::decide(&t);
                if d == Decision::Nothing {
                    continue;
                }
                match model::phase(&d) {
                    0 => work.push((t, d)),
                    1 => deletions_list.push((t, d)),
                    _ => removals.push((t, d)),
                }
            }
            let (a, f) = run_all(ctx, work).await?;
            acted += a;
            failed += f;
        }
        if run_deletions {
            let (a, f) = run_all(ctx, deletions_list).await?;
            acted += a;
            failed += f;
            if approved {
                ctx.db(|s, id| s.remove_issue(id, "deletions", "")).await?;
                held_any = false;
            }
        }
        removals.sort_by_key(|(t, _)| std::cmp::Reverse(model::depth(&t.path)));
        for item in removals {
            match run_one(ctx, item.0, item.1).await? {
                Done::Changed => acted += 1,
                Done::Failed => failed += 1,
                Done::Waiting => {}
            }
        }
        failed_any = failed > 0;
        if acted == 0 {
            break;
        }
        // Something changed: the next cycle counts again.
        left = u64::MAX;
    }
    Ok(left == 0 && !failed_any && !held_any)
}

/// "Restore" for held deletions: forgets the synced state of each path
/// the guard held, so the next cycle copies it back to the side it was
/// deleted from (download what was deleted here, upload what was deleted
/// in the bucket).
async fn restore_held(ctx: &Arc<Ctx>, unknown: &[String]) -> Result<(), Stop> {
    let mut after = String::new();
    loop {
        let chunk = {
            let a = after.clone();
            ctx.db(move |s, id| s.candidates(id, &a, CHUNK)).await?
        };
        let Some(last) = chunk.last() else { break };
        after = last.path.clone();
        let restore: Vec<String> = chunk
            .iter()
            .filter(|t| !below(&t.path, unknown) && model::is_deletion(&model::decide(t)))
            .map(|t| t.path.clone())
            .collect();
        ctx.db(move |s, id| {
            for p in &restore {
                s.remove_synced(id, p)?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Runs `work` with at most [`PARALLEL`] actions at once. Returns how many
/// changed something and how many failed; stops the pass on an error that
/// concerns the whole sync.
async fn run_all(ctx: &Arc<Ctx>, work: Vec<(Triple, Decision)>) -> Result<(usize, usize), Stop> {
    let mut set: JoinSet<Result<Done, Stop>> = JoinSet::new();
    let mut work = work.into_iter();
    let (mut acted, mut failed) = (0, 0);
    let mut stop: Option<Stop> = None;
    loop {
        while stop.is_none() && set.len() < PARALLEL {
            let Some((t, d)) = work.next() else { break };
            let c = Arc::clone(ctx);
            set.spawn(async move { run_one(&c, t, d).await });
        }
        let Some(joined) = set.join_next().await else {
            break;
        };
        match joined {
            Ok(Ok(Done::Changed)) => acted += 1,
            Ok(Ok(Done::Failed)) => failed += 1,
            Ok(Ok(Done::Waiting)) => {}
            Ok(Err(s)) => {
                if stop.is_none() {
                    stop = Some(s);
                }
            }
            Err(e) => {
                eprintln!("sotyware: a sync action stopped unexpectedly: {e}");
                failed += 1;
            }
        }
    }
    match stop {
        Some(s) => Err(s),
        None => Ok((acted, failed)),
    }
}

// ---------------------------------------------------------------- actions

/// Runs one decision, after checking the live state; records success or
/// failure.
async fn run_one(ctx: &Arc<Ctx>, t: Triple, d: Decision) -> Result<Done, Stop> {
    if ctx.control.stopped() {
        return Err(Stop::Stopped);
    }
    let path = t.path.clone();
    let result = match &d {
        Decision::Nothing => Ok(Done::Waiting),
        Decision::Upload {
            replace,
            check_content,
        } => upload(ctx, &t, replace.clone(), *check_content).await,
        Decision::Download {
            replace,
            check_content,
        } => download(ctx, &t, *replace, *check_content).await,
        Decision::Compare => compare(ctx, &t).await,
        Decision::DeleteRemote => delete_remote(ctx, &t).await,
        Decision::DeleteLocal => delete_local(ctx, &t).await,
        Decision::Forget => {
            let p = path.clone();
            ctx.db(move |s, id| s.remove_synced(id, &p)).await?;
            Ok(Done::Changed)
        }
        Decision::MakeLocalFolder => make_local_folder(ctx, &t).await,
        Decision::MakeRemoteFolder => make_remote_folder(ctx, &t).await,
        Decision::AdoptFolder => {
            let p = path.clone();
            ctx.db(move |s, id| {
                s.put_synced(
                    id,
                    &Synced {
                        path: p,
                        dir: true,
                        ..Synced::default()
                    },
                    now_ms(),
                )
            })
            .await?;
            Ok(Done::Changed)
        }
        Decision::RemoveRemoteFolder => remove_remote_folder(ctx, &t).await,
        Decision::RemoveLocalFolder => remove_local_folder(ctx, &t).await,
        Decision::KindClash => {
            let p = path.clone();
            let data = serde_json::to_string(&SkippedItem {
                reason: "a file and a folder have this name (one here, one in the bucket); neither is synced".into(),
            })
            .unwrap_or_default();
            ctx.db(move |s, id| s.put_issue(id, "skipped-local", &p, &data, now_ms()))
                .await?;
            Ok(Done::Waiting)
        }
    };
    match result {
        Ok(Done::Failed) => Ok(Done::Failed),
        Ok(done) => {
            if matches!(done, Done::Changed) {
                let p = path.clone();
                ctx.db(move |s, id| s.remove_issue(id, "failed", &p))
                    .await?;
            }
            Ok(done)
        }
        Err(Failure::Stop(s)) => Err(s),
        Err(Failure::Item { kind, message }) => {
            record_failure(ctx, &path, op_name(&d), kind, message).await?;
            Ok(Done::Failed)
        }
    }
}

/// Why one action did not complete.
enum Failure {
    /// Only this path: recorded, retried later.
    Item { kind: ErrorKind, message: String },
    /// The whole pass.
    Stop(Stop),
}

impl From<Stop> for Failure {
    fn from(s: Stop) -> Self {
        Self::Stop(s)
    }
}

fn item(kind: ErrorKind, message: impl Into<String>) -> Failure {
    Failure::Item {
        kind,
        message: message.into(),
    }
}

/// A provider error for one path: the pass stops when it concerns the
/// whole sync (connection, key).
fn object_failure(e: &StorageError) -> Failure {
    match e {
        StorageError::Cancelled => Failure::Stop(Stop::Stopped),
        StorageError::Network(_)
        | StorageError::Timeout(_)
        | StorageError::InvalidCredentials(_) => Failure::Stop(storage_stop(e)),
        e if e.is_transient() => Failure::Stop(storage_stop(e)),
        StorageError::LocalWrite { kind, message } => local_failure(*kind, message),
        StorageError::Io(m) => item(ErrorKind::LocalFile, short(m)),
        other => {
            let c = crate::cloud::operations::operation_error(other);
            item(c.kind, c.message)
        }
    }
}

fn local_failure(kind: std::io::ErrorKind, message: &str) -> Failure {
    use std::io::ErrorKind as K;
    match kind {
        K::StorageFull | K::QuotaExceeded => {
            item(ErrorKind::DiskFull, "the drive of the sync folder is full")
        }
        K::PermissionDenied | K::ReadOnlyFilesystem => item(
            ErrorKind::LocalPermission,
            "the system refused to write in the sync folder (permissions, or the file is open in another program)",
        ),
        K::NotFound => item(
            ErrorKind::DestinationUnavailable,
            "a folder of the path is gone",
        ),
        _ => item(ErrorKind::LocalFile, short(message)),
    }
}

fn io_failure(e: &std::io::Error) -> Failure {
    local_failure(e.kind(), &e.to_string())
}

fn op_name(d: &Decision) -> &'static str {
    match d {
        Decision::Upload { .. } => "upload",
        Decision::Download { .. } => "download",
        Decision::Compare => "compare",
        Decision::DeleteRemote => "delete-remote",
        Decision::DeleteLocal => "delete-local",
        Decision::MakeLocalFolder | Decision::MakeRemoteFolder => "create-folder",
        Decision::RemoveLocalFolder | Decision::RemoveRemoteFolder => "remove-folder",
        _ => "sync",
    }
}

async fn record_failure(
    ctx: &Arc<Ctx>,
    path: &str,
    op: &str,
    kind: ErrorKind,
    message: String,
) -> Result<(), Stop> {
    let (p, o) = (path.to_owned(), op.to_owned());
    let now = now_ms();
    let next = ctx
        .db(move |s, id| {
            let attempts = s
                .issues(id)?
                .into_iter()
                .find(|i| i.kind == "failed" && i.path == p)
                .and_then(|i| serde_json::from_str::<FailedItem>(&i.data).ok())
                .map_or(0, |f| f.attempts)
                + 1;
            let next = now + i64::try_from(model::retry_delay(attempts).as_millis()).unwrap_or(0);
            let f = FailedItem {
                op: o,
                kind,
                message,
                attempts,
                next_ms: next,
                first_ms: now,
            };
            s.put_issue(
                id,
                "failed",
                &p,
                &serde_json::to_string(&f).unwrap_or_default(),
                now,
            )?;
            Ok(next)
        })
        .await?;
    ctx.retry_in(Duration::from_millis(
        u64::try_from(next - now).unwrap_or(0),
    ));
    Ok(())
}

async fn log(ctx: &Arc<Ctx>, path: &str, op: &str, bytes: u64, message: Option<String>) {
    let item = ActivityItem {
        at_ms: now_ms(),
        path: path.to_owned(),
        op: op.to_owned(),
        bytes,
        message,
    };
    let data = serde_json::to_string(&item).unwrap_or_default();
    if let Err(Stop::Fatal(e)) = ctx
        .db(move |s, id| s.add_activity(id, &data, item.at_ms))
        .await
    {
        eprintln!("sotyware: cannot record the sync activity: {}", e.message);
    }
    ctx.shared.update(ctx.id(), |s| {
        s.activity_seq += 1;
    });
}

/// How long ago a file modified at `mtime_ns` changed, if that is less
/// than [`model::SETTLE`] (it may still be being written).
fn settling(mtime_ns: Option<i64>) -> Option<Duration> {
    let ns = mtime_ns?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    let age = Duration::from_nanos(u64::try_from(now.saturating_sub(ns)).unwrap_or(0));
    (age < model::SETTLE).then_some(age)
}

/// Waits until a recently modified file has not changed for
/// [`model::SETTLE`]. `false` (and the scan updated) if it changed
/// meanwhile: the next cycle decides again.
async fn wait_settled(ctx: &Arc<Ctx>, local: &Local) -> Result<bool, Failure> {
    let Some(age) = settling(local.mtime_ns) else {
        return Ok(true);
    };
    tokio::time::sleep(model::SETTLE.saturating_sub(age) + Duration::from_millis(50)).await;
    if ctx.control.stopped() {
        return Err(Failure::Stop(Stop::Stopped));
    }
    let path = scan::join(&ctx.root, &local.path);
    let now = blocking(move || scan::stamp(&path))
        .await
        .map_err(|e| io_failure(&e))?;
    let unchanged = now.is_some_and(|n| {
        n.size == local.size && n.mtime_ns == local.mtime_ns && n.file_id == local.file_id
    });
    if !unchanged {
        refresh_local(ctx, &local.path).await?;
    }
    Ok(unchanged)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// The SHA-256 and stamp of the local file at `rel`.
async fn hash_local(ctx: &Ctx, rel: &str) -> Result<(String, Stamp), Failure> {
    let path = scan::join(&ctx.root, rel);
    blocking(move || scan::hash(&path))
        .await
        .map_err(|e| io_failure(&e))
}

fn local_of(rel: &str, stamp: &Stamp) -> Local {
    Local {
        path: rel.to_owned(),
        dir: false,
        size: stamp.size,
        mtime_ns: stamp.mtime_ns,
        file_id: stamp.file_id,
    }
}

fn synced_of(
    rel: &str,
    stamp: &Stamp,
    sha: &str,
    etag: Option<String>,
    remote_size: u64,
) -> Synced {
    Synced {
        path: rel.to_owned(),
        dir: false,
        size: stamp.size,
        mtime_ns: stamp.mtime_ns,
        file_id: stamp.file_id,
        sha256: Some(sha.to_owned()),
        etag,
        remote_size: Some(remote_size),
    }
}

/// Records the bucket's current object at `rel` (a HEAD), so the next cycle
/// decides with it.
async fn refresh_remote(ctx: &Arc<Ctx>, rel: &str) -> Result<(), Failure> {
    let key = ctx.key(rel, false);
    let head = ctx
        .store
        .head_object(ctx.bucket(), &key)
        .await
        .map_err(|e| object_failure(&e))?;
    let row = head.map(|h| Remote {
        path: rel.to_owned(),
        dir: false,
        marker: false,
        size: h.size,
        etag: h.e_tag,
        modified_ms: None,
    });
    let p = rel.to_owned();
    ctx.db(move |s, id| s.set_remote(id, &p, row.as_ref()))
        .await?;
    ctx.relist.store(true, Ordering::SeqCst);
    Ok(())
}

/// Records the local file at `rel` as it is now.
async fn refresh_local(ctx: &Arc<Ctx>, rel: &str) -> Result<(), Failure> {
    let (root, r, prefix) = (ctx.root.clone(), rel.to_owned(), ctx.config.prefix.clone());
    let found = blocking(move || scan::look(&root, &r, &prefix))
        .await
        .map_err(|e| io_failure(&e))?;
    let row = found.ok().flatten();
    let p = rel.to_owned();
    ctx.db(move |s, id| s.set_local(id, &p, row.as_ref()))
        .await?;
    Ok(())
}

/// A running transfer's entry on the page, removed when dropped.
struct Shown {
    ctx: Arc<Ctx>,
    key: u64,
}

impl Shown {
    fn new(ctx: &Arc<Ctx>, rel: &str, kind: TransferKind, total: u64) -> Self {
        let key = ctx.shared.transfer_start(ctx.id(), rel, kind, total);
        Self {
            ctx: Arc::clone(ctx),
            key,
        }
    }

    fn progress(&self) -> impl Fn(u64) + Send + Sync + 'static {
        let (shared, id, key) = (
            Arc::clone(&self.ctx.shared),
            self.ctx.config.id.clone(),
            self.key,
        );
        move |bytes| shared.transfer_progress(&id, key, bytes)
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        self.ctx.shared.transfer_end(self.ctx.id(), self.key);
    }
}

/// A transfer control that records itself in the operation journal and
/// that stopping the engine cancels.
fn transfer_control(ctx: &Ctx) -> (Arc<TransferControl>, u64) {
    let control = Arc::new(TransferControl::new());
    if let Some(journal) = ctx.shared.journal.get() {
        control.record_in(Arc::clone(journal));
    }
    let key = ctx.control.register(Arc::clone(&control));
    (control, key)
}

async fn upload(
    ctx: &Arc<Ctx>,
    t: &Triple,
    replace: Option<String>,
    check_content: bool,
) -> Result<Done, Failure> {
    let Some(local) = &t.local else {
        return Ok(Done::Waiting);
    };
    if !wait_settled(ctx, local).await? {
        return Ok(Done::Changed);
    }
    let rel = t.path.as_str();
    // Only the stamp changed? Then nothing to send.
    if check_content && let Some(s) = &t.synced {
        let (sha, stamp) = hash_local(ctx, rel).await?;
        if s.sha256.as_deref() == Some(sha.as_str()) {
            let row = Synced {
                size: stamp.size,
                mtime_ns: stamp.mtime_ns,
                file_id: stamp.file_id,
                ..s.clone()
            };
            let l = local_of(rel, &stamp);
            ctx.db(move |st, id| {
                st.put_synced(id, &row, now_ms())?;
                st.set_local(id, &row.path, Some(&l))
            })
            .await?;
            return Ok(Done::Changed);
        }
    }
    // A new file here, nothing in the bucket: maybe a synced file moved.
    if t.synced.is_none()
        && t.remote.is_none()
        && let Some(done) = try_remote_move(ctx, rel, local.size).await?
    {
        return Ok(done);
    }
    let path = scan::join(&ctx.root, rel);
    let before = {
        let p = path.clone();
        blocking(move || scan::stamp(&p))
            .await
            .map_err(|e| io_failure(&e))?
    };
    let Some(before) = before else {
        refresh_local(ctx, rel).await?;
        return Ok(Done::Waiting);
    };
    let key = ctx.key(rel, false);
    let options = UploadOptions {
        destination: match replace {
            Some(e_tag) => Destination::Replace { e_tag },
            None => Destination::New,
        },
        mtime_ms: before.mtime_ms(),
    };
    let shown = Shown::new(ctx, rel, TransferKind::Upload, before.size);
    let progress = shown.progress();
    let (control, reg) = transfer_control(ctx);
    let result = ctx
        .store
        .upload_with(ctx.bucket(), &key, &path, &options, &control, &mut |p| {
            if p.stage == UploadStage::Sending || p.stage == UploadStage::Verifying {
                progress(p.bytes);
            }
        })
        .await;
    ctx.control.unregister(reg);
    drop(shown);
    match result {
        Ok(report) => {
            let after = {
                let p = path.clone();
                blocking(move || scan::stamp(&p)).await.ok().flatten()
            };
            let row = synced_of(
                rel,
                &before,
                &report.sha256,
                report.e_tag.clone(),
                report.size,
            );
            let remote = Remote {
                path: rel.to_owned(),
                dir: false,
                marker: false,
                size: report.size,
                etag: report.e_tag.clone(),
                modified_ms: None,
            };
            // The scan gets what is there now: if the file changed right
            // after the upload, the next cycle sees the change.
            let l = after.map(|a| local_of(rel, &a));
            ctx.db(move |s, id| {
                s.put_synced(id, &row, now_ms())?;
                s.set_remote(id, &row.path, Some(&remote))?;
                s.set_local(id, &row.path, l.as_ref())
            })
            .await?;
            log(ctx, rel, "upload", report.size, None).await;
            Ok(Done::Changed)
        }
        Err(e) => match &e.error {
            // The bucket holds another object than expected (or none): the
            // next cycle decides with what is there.
            StorageError::AlreadyExists(_) | StorageError::NotFound(_) => {
                refresh_remote(ctx, rel).await?;
                Ok(Done::Changed)
            }
            StorageError::FileChanged(_) => {
                refresh_local(ctx, rel).await?;
                Ok(Done::Changed)
            }
            StorageError::TooLarge { .. } => {
                let p = rel.to_owned();
                let data = serde_json::to_string(&SkippedItem {
                    reason: "the file is larger than the provider accepts".into(),
                })
                .unwrap_or_default();
                ctx.db(move |s, id| s.put_issue(id, "skipped-local", &p, &data, now_ms()))
                    .await?;
                Ok(Done::Waiting)
            }
            other => Err(object_failure(other)),
        },
    }
}

/// A new local file whose content is a synced file that is gone here
/// while unchanged in the bucket: copied server side to the new key, the
/// old one deleted. `None` when it is not a move.
async fn try_remote_move(ctx: &Arc<Ctx>, rel: &str, size: u64) -> Result<Option<Done>, Failure> {
    if size > MAX_COPY_BYTES {
        return Ok(None);
    }
    let sources = ctx.db(move |s, id| s.moved_from_here(id, size)).await?;
    if sources.is_empty() {
        return Ok(None);
    }
    let (sha, stamp) = hash_local(ctx, rel).await?;
    let Some(source) = sources
        .into_iter()
        .find(|s| s.sha256.as_deref() == Some(sha.as_str()))
    else {
        return Ok(None);
    };
    if !ctx
        .claimed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(source.path.clone())
    {
        return Ok(None);
    }
    // The old file must still be gone here.
    let old = scan::join(&ctx.root, &source.path);
    if blocking(move || old.symlink_metadata().is_ok()).await {
        return Ok(None);
    }
    let (from, to) = (ctx.key(&source.path, false), ctx.key(rel, false));
    if ctx
        .store
        .head_object(ctx.bucket(), &to)
        .await
        .map_err(|e| object_failure(&e))?
        .is_some()
    {
        refresh_remote(ctx, rel).await?;
        return Ok(Some(Done::Changed));
    }
    match ctx
        .store
        .copy_object(ctx.bucket(), &from, None, source.etag.as_deref(), &to)
        .await
    {
        Ok(_) => {}
        // The old object changed meanwhile: an upload instead.
        Err(StorageError::Provider { status: 412, .. }) | Err(StorageError::NotFound(_)) => {
            return Ok(None);
        }
        Err(e) => return Err(object_failure(&e)),
    }
    let head = ctx
        .store
        .head_object(ctx.bucket(), &to)
        .await
        .map_err(|e| object_failure(&e))?;
    let etag = head.as_ref().and_then(|h| h.e_tag.clone());
    let row = synced_of(rel, &stamp, &sha, etag.clone(), size);
    let remote = Remote {
        path: rel.to_owned(),
        dir: false,
        marker: false,
        size,
        etag,
        modified_ms: None,
    };
    let old_path = source.path.clone();
    ctx.db(move |s, id| {
        s.put_synced(id, &row, now_ms())?;
        s.set_remote(id, &row.path, Some(&remote))?;
        s.set_local(id, &row.path, Some(&local_of(&row.path, &stamp)))
    })
    .await?;
    // The old key: deleted (on a versioned bucket it stays as an earlier
    // version). If that fails, the next cycle sees an ordinary deletion.
    if ctx
        .store
        .delete_reporting(ctx.bucket(), &from, None)
        .await
        .is_ok()
    {
        let p = old_path.clone();
        ctx.db(move |s, id| {
            s.remove_synced(id, &p)?;
            s.set_remote(id, &p, None)
        })
        .await?;
    }
    log(
        ctx,
        rel,
        "move-remote",
        0,
        Some(format!("moved in the bucket from {old_path}")),
    )
    .await;
    Ok(Some(Done::Changed))
}

async fn download(
    ctx: &Arc<Ctx>,
    t: &Triple,
    replace: bool,
    check_content: bool,
) -> Result<Done, Failure> {
    let Some(remote) = &t.remote else {
        return Ok(Done::Waiting);
    };
    let rel = t.path.as_str();
    // Only the ETag changed? The stored SHA-256 tells.
    if check_content
        && let Some(s) = &t.synced
        && s.sha256.is_some()
    {
        let props = ctx
            .store
            .object_properties(ctx.bucket(), &ctx.key(rel, false))
            .await
            .map_err(|e| object_failure(&e))?;
        let Some(props) = props else {
            refresh_remote(ctx, rel).await?;
            return Ok(Done::Changed);
        };
        if props.sha256() == s.sha256.as_deref() && props.size == s.size {
            let row = Synced {
                etag: props.e_tag.clone(),
                remote_size: Some(props.size),
                ..s.clone()
            };
            ctx.db(move |st, id| st.put_synced(id, &row, now_ms()))
                .await?;
            return Ok(Done::Changed);
        }
    }
    // A new object, nothing here: maybe a synced object moved.
    if t.synced.is_none()
        && t.local.is_none()
        && let Some(etag) = &remote.etag
        && let Some(done) = try_local_move(ctx, rel, etag, remote.size).await?
    {
        return Ok(done);
    }
    let expected = if replace {
        t.synced.as_ref().map(|s| Stamp {
            size: s.size,
            mtime_ns: s.mtime_ns,
            file_id: s.file_id,
        })
    } else {
        None
    };
    // A new file whose name is taken here: by a file the scan has not seen
    // yet (decided again), or, on Windows and macOS, by one whose name
    // differs only in case (never downloaded: it would replace it).
    if expected.is_none() {
        let target = scan::join(&ctx.root, rel);
        let taken = blocking(move || -> Option<bool> {
            target.symlink_metadata().ok()?;
            let (parent, name) = (target.parent()?, target.file_name()?);
            Some(
                std::fs::read_dir(parent)
                    .map(|d| d.flatten().any(|e| e.file_name() == name))
                    .unwrap_or(true),
            )
        })
        .await;
        match taken {
            Some(true) => {
                refresh_local(ctx, rel).await?;
                return Ok(Done::Changed);
            }
            Some(false) => {
                record_case_clash(ctx, rel).await?;
                return Ok(Done::Waiting);
            }
            None => {}
        }
    }
    match fetch(ctx, rel, remote.size).await? {
        Fetched::Staged(staged, report) => {
            place(ctx, rel, &staged, expected, &report, "download").await
        }
        Fetched::Waiting => Ok(Done::Waiting),
    }
}

/// A downloaded, verified file in the staging folder.
enum Fetched {
    Staged(PathBuf, crate::cloud::s3::DownloadReport),
    Waiting,
}

/// Downloads `rel` into the staging folder (verified: size and stored
/// SHA-256), with its modification time set.
async fn fetch(ctx: &Arc<Ctx>, rel: &str, size: u64) -> Result<Fetched, Failure> {
    if ctx.disk_full.load(Ordering::SeqCst) {
        return Err(item(
            ErrorKind::DiskFull,
            "the drive of the sync folder is full",
        ));
    }
    let free = {
        let root = ctx.root.clone();
        let shared = Arc::clone(&ctx.shared);
        blocking(move || shared.free_space(&root)).await
    };
    if let Some(free) = free
        && free < size.saturating_add(DISK_RESERVE)
    {
        ctx.disk_full.store(true, Ordering::SeqCst);
        ctx.shared.update(ctx.id(), |s| s.disk_full = true);
        return Err(item(
            ErrorKind::DiskFull,
            format!(
                "not enough space on the drive of the sync folder ({} free, {} needed with a reserve of 256 MiB)",
                crate::folder_sync::bytes(free),
                crate::folder_sync::bytes(size.saturating_add(DISK_RESERVE))
            ),
        ));
    }
    let root = ctx.root.clone();
    let staged = blocking(move || {
        scan::prepare_own_dir(&root)?;
        Ok::<_, std::io::Error>(scan::staging_file(&root))
    })
    .await
    .map_err(|e| io_failure(&e))?;
    let shown = Shown::new(ctx, rel, TransferKind::Download, size);
    let progress = shown.progress();
    let (control, reg) = transfer_control(ctx);
    let result = ctx
        .store
        .download(
            ctx.bucket(),
            &ctx.key(rel, false),
            &staged,
            Existing::Refuse,
            &control,
            &mut |bytes, _| progress(bytes),
        )
        .await;
    ctx.control.unregister(reg);
    drop(shown);
    let report = match result {
        Ok(r) => r,
        Err(StorageError::NotFound(_)) => {
            refresh_remote(ctx, rel).await?;
            return Ok(Fetched::Waiting);
        }
        Err(e) => {
            if let StorageError::LocalWrite {
                kind: std::io::ErrorKind::StorageFull,
                ..
            } = e
            {
                ctx.disk_full.store(true, Ordering::SeqCst);
                ctx.shared.update(ctx.id(), |s| s.disk_full = true);
            }
            return Err(object_failure(&e));
        }
    };
    // The file gets the time it had where it was uploaded (sotyware's
    // metadata), else the time the provider stored it.
    if let Some(ms) = report.mtime_ms.or(report.modified_ms)
        && let Ok(ms) = u64::try_from(ms)
    {
        let s = staged.clone();
        let when = SystemTime::UNIX_EPOCH + Duration::from_millis(ms);
        let _ = blocking(move || {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&s)
                .and_then(|f| f.set_modified(when))
        })
        .await;
    }
    Ok(Fetched::Staged(staged, report))
}

/// Moves a staged download to `rel`: over the file with the `expected`
/// stamp, or where nothing is (`None`). Records the synced state.
async fn place(
    ctx: &Arc<Ctx>,
    rel: &str,
    staged: &Path,
    expected: Option<Stamp>,
    report: &crate::cloud::s3::DownloadReport,
    op: &str,
) -> Result<Done, Failure> {
    let target = scan::join(&ctx.root, rel);
    let staged_c = staged.to_path_buf();
    let outcome = blocking(move || -> std::io::Result<Placed> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let now = scan::stamp(&target).ok().flatten();
        match (expected, now) {
            (Some(e), Some(n)) if e == n => {
                std::fs::rename(&staged_c, &target)?;
            }
            (None, None) if target.symlink_metadata().is_err() => {
                // Never replaces a file that appeared meanwhile.
                match std::fs::hard_link(&staged_c, &target) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&staged_c);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let _ = std::fs::remove_file(&staged_c);
                        return Ok(Placed::Changed);
                    }
                    Err(_) => {
                        if target.symlink_metadata().is_ok() {
                            let _ = std::fs::remove_file(&staged_c);
                            return Ok(Placed::Changed);
                        }
                        std::fs::rename(&staged_c, &target)?;
                    }
                }
            }
            _ => {
                let _ = std::fs::remove_file(&staged_c);
                // A file whose name differs only in case (Windows, macOS)?
                if expected.is_none()
                    && let Some(parent) = target.parent()
                    && let Some(name) = target.file_name()
                {
                    let exact = std::fs::read_dir(parent)
                        .map(|d| d.flatten().any(|e| e.file_name() == name))
                        .unwrap_or(true);
                    if !exact {
                        return Ok(Placed::CaseClash);
                    }
                }
                return Ok(Placed::Changed);
            }
        }
        Ok(Placed::Done(scan::stamp(&target)?))
    })
    .await;
    let placed = match outcome {
        Ok(p) => p,
        Err(e) => {
            let s = staged.to_path_buf();
            blocking(move || {
                let _ = std::fs::remove_file(s);
            })
            .await;
            return Err(io_failure(&e));
        }
    };
    match placed {
        Placed::Done(Some(stamp)) => {
            let row = synced_of(
                rel,
                &stamp,
                &report.sha256,
                report.e_tag.clone(),
                report.size,
            );
            let remote = Remote {
                path: rel.to_owned(),
                dir: false,
                marker: false,
                size: report.size,
                etag: report.e_tag.clone(),
                modified_ms: report.modified_ms,
            };
            ctx.db(move |s, id| {
                s.put_synced(id, &row, now_ms())?;
                s.set_remote(id, &row.path, Some(&remote))?;
                s.set_local(id, &row.path, Some(&local_of(&row.path, &stamp)))
            })
            .await?;
            log(ctx, rel, op, report.size, None).await;
            Ok(Done::Changed)
        }
        Placed::Done(None) | Placed::Changed => {
            // The local file changed (or appeared) meanwhile: the next
            // cycle compares.
            refresh_local(ctx, rel).await?;
            Ok(Done::Changed)
        }
        Placed::CaseClash => {
            record_case_clash(ctx, rel).await?;
            Ok(Done::Waiting)
        }
    }
}

/// `rel` exists in the bucket, and a file whose name differs only in
/// capitalisation exists here, on a file system that does not tell them
/// apart: the object is not synced (until a full scan looks again).
async fn record_case_clash(ctx: &Arc<Ctx>, rel: &str) -> Result<(), Failure> {
    let p = rel.to_owned();
    let data = serde_json::to_string(&SkippedItem {
        reason: "in the bucket: a file whose name differs only in capitalisation is already here, and this computer's file system does not tell them apart".into(),
    })
    .unwrap_or_default();
    ctx.db(move |s, id| s.put_issue(id, "skipped-case", &p, &data, now_ms()))
        .await?;
    Ok(())
}

enum Placed {
    Done(Option<Stamp>),
    Changed,
    CaseClash,
}

/// A new object whose ETag and size are a synced file's that is gone from
/// the bucket while unchanged here: renamed here instead of downloaded.
async fn try_local_move(
    ctx: &Arc<Ctx>,
    rel: &str,
    etag: &str,
    size: u64,
) -> Result<Option<Done>, Failure> {
    let etag_c = etag.to_owned();
    let sources = ctx
        .db(move |s, id| s.moved_from_there(id, &etag_c, size))
        .await?;
    let Some(source) = sources.into_iter().find(|s| {
        !ctx.claimed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&s.path)
    }) else {
        return Ok(None);
    };
    // The old object must still be gone.
    if ctx
        .store
        .head_object(ctx.bucket(), &ctx.key(&source.path, false))
        .await
        .map_err(|e| object_failure(&e))?
        .is_some()
    {
        return Ok(None);
    }
    if !ctx
        .claimed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(source.path.clone())
    {
        return Ok(None);
    }
    let (from, to) = (
        scan::join(&ctx.root, &source.path),
        scan::join(&ctx.root, rel),
    );
    let expected = Stamp {
        size: source.size,
        mtime_ns: source.mtime_ns,
        file_id: source.file_id,
    };
    let moved = blocking(move || -> std::io::Result<Option<Stamp>> {
        if scan::stamp(&from)? != Some(expected) || to.symlink_metadata().is_ok() {
            return Ok(None);
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&from, &to)?;
        scan::stamp(&to)
    })
    .await
    .map_err(|e| io_failure(&e))?;
    let Some(stamp) = moved else {
        return Ok(None);
    };
    let row = Synced {
        path: rel.to_owned(),
        size: stamp.size,
        mtime_ns: stamp.mtime_ns,
        file_id: stamp.file_id,
        etag: Some(etag.to_owned()),
        remote_size: Some(size),
        ..source.clone()
    };
    let old = source.path.clone();
    ctx.db(move |s, id| {
        s.remove_synced(id, &old)?;
        s.set_local(id, &old, None)?;
        s.put_synced(id, &row, now_ms())?;
        s.set_local(id, &row.path, Some(&local_of(&row.path, &stamp)))
    })
    .await?;
    log(
        ctx,
        rel,
        "move-local",
        0,
        Some(format!("moved here from {}", source.path)),
    )
    .await;
    Ok(Some(Done::Changed))
}

/// Both sides have a file at `rel` that differs from the synced state (or
/// there is none): equal contents are recorded; different ones are both
/// kept.
async fn compare(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let (Some(local), Some(_)) = (&t.local, &t.remote) else {
        return Ok(Done::Waiting);
    };
    if !wait_settled(ctx, local).await? {
        return Ok(Done::Changed);
    }
    let rel = t.path.as_str();
    let (sha, stamp) = hash_local(ctx, rel).await?;
    let props = ctx
        .store
        .object_properties(ctx.bucket(), &ctx.key(rel, false))
        .await
        .map_err(|e| object_failure(&e))?;
    let Some(props) = props else {
        refresh_remote(ctx, rel).await?;
        return Ok(Done::Changed);
    };
    let Some(e_tag) = props.e_tag.clone() else {
        return Err(item(
            ErrorKind::Provider,
            "the provider sent no ETag for this object, so a change cannot be told apart",
        ));
    };
    // The content the synced state names, on either side: only the other
    // side changed after all.
    if let Some(s) = &t.synced
        && let Some(synced_sha) = s.sha256.as_deref()
    {
        if synced_sha == sha {
            // Only the bucket changed: a download replacing this file.
            let fixed = Synced {
                size: stamp.size,
                mtime_ns: stamp.mtime_ns,
                file_id: stamp.file_id,
                ..s.clone()
            };
            ctx.db(move |st, id| st.put_synced(id, &fixed, now_ms()))
                .await?;
            return Ok(Done::Changed);
        }
        if props.sha256() == Some(synced_sha) && props.size == s.size {
            let fixed = Synced {
                etag: Some(e_tag),
                remote_size: Some(props.size),
                ..s.clone()
            };
            ctx.db(move |st, id| st.put_synced(id, &fixed, now_ms()))
                .await?;
            return Ok(Done::Changed);
        }
    }
    // Same content on both sides.
    if props.sha256() == Some(sha.as_str()) && props.size == stamp.size {
        let row = synced_of(rel, &stamp, &sha, Some(e_tag), props.size);
        ctx.db(move |s, id| s.put_synced(id, &row, now_ms()))
            .await?;
        return Ok(Done::Changed);
    }
    // No stored SHA-256 (another tool uploaded it) but the same size: only
    // the content can tell.
    let mut staged = None;
    if props.sha256().is_none() && props.size == stamp.size {
        match fetch(ctx, rel, props.size).await? {
            Fetched::Staged(path, report) => {
                if report.sha256 == sha {
                    let p = path.clone();
                    blocking(move || {
                        let _ = std::fs::remove_file(p);
                    })
                    .await;
                    let row = synced_of(rel, &stamp, &sha, report.e_tag.clone(), report.size);
                    ctx.db(move |s, id| s.put_synced(id, &row, now_ms()))
                        .await?;
                    return Ok(Done::Changed);
                }
                staged = Some((path, report));
            }
            Fetched::Waiting => return Ok(Done::Changed),
        }
    }
    keep_both(ctx, rel, &stamp, &sha, &e_tag, props.size, staged).await
}

/// A conflict as recorded (`issues`, kind `conflict`, at the original
/// path).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConflictItem {
    /// Where the bucket's version was kept (relative path).
    pub copy: String,
    /// The bucket's version that was kept (its ETag).
    pub remote_etag: String,
    pub at_ms: i64,
    /// `started` until both versions are kept, then `kept-both`.
    pub state: String,
}

/// Keeps both versions: the file here keeps its name (and is uploaded
/// over the bucket's version), the bucket's version becomes `name
/// (conflicted copy …)` on both sides. The copy's name is recorded first,
/// so an interrupted conflict goes on with the same name.
async fn keep_both(
    ctx: &Arc<Ctx>,
    rel: &str,
    stamp: &Stamp,
    sha: &str,
    remote_etag: &str,
    remote_size: u64,
    staged: Option<(PathBuf, crate::cloud::s3::DownloadReport)>,
) -> Result<Done, Failure> {
    // 1. The copy's name: the one recorded for this version, else a free one.
    let p = rel.to_owned();
    let recorded = ctx
        .db(move |s, id| {
            Ok(s.issues(id)?
                .into_iter()
                .find(|i| i.kind == "conflict" && i.path == p)
                .and_then(|i| serde_json::from_str::<ConflictItem>(&i.data).ok()))
        })
        .await?;
    let copy = match recorded {
        Some(c) if c.remote_etag == remote_etag && c.state == "started" => c.copy,
        _ => {
            let stamp_text = model::conflict_stamp(now_ms());
            let mut n = 1;
            loop {
                let candidate = model::conflict_name(rel, &stamp_text, n);
                let local_free = {
                    let path = scan::join(&ctx.root, &candidate);
                    blocking(move || path.symlink_metadata().is_err()).await
                };
                let remote_free = ctx
                    .store
                    .head_object(ctx.bucket(), &ctx.key(&candidate, false))
                    .await
                    .map_err(|e| object_failure(&e))?
                    .is_none();
                if local_free && remote_free {
                    break candidate;
                }
                n += 1;
                if n > 50 {
                    return Err(item(ErrorKind::Other, "no free name for a conflicted copy"));
                }
            }
        }
    };
    let item_row = ConflictItem {
        copy: copy.clone(),
        remote_etag: remote_etag.to_owned(),
        at_ms: now_ms(),
        state: "started".into(),
    };
    {
        let (p, data) = (
            rel.to_owned(),
            serde_json::to_string(&item_row).unwrap_or_default(),
        );
        ctx.db(move |s, id| s.put_issue(id, "conflict", &p, &data, now_ms()))
            .await?;
    }
    // 2. The bucket's version, server side, under the copy's name.
    let (from, to) = (ctx.key(rel, false), ctx.key(&copy, false));
    let copy_exists = ctx
        .store
        .head_object(ctx.bucket(), &to)
        .await
        .map_err(|e| object_failure(&e))?
        .is_some();
    // Above 5 GiB the copy is uploaded from the local copy (step 3).
    if !copy_exists && remote_size <= MAX_COPY_BYTES {
        match ctx
            .store
            .copy_object(ctx.bucket(), &from, None, Some(remote_etag), &to)
            .await
        {
            Ok(_) => {}
            Err(StorageError::Provider { status: 412, .. }) | Err(StorageError::NotFound(_)) => {
                // The bucket's version changed again: decide anew.
                if let Some((s, _)) = staged {
                    blocking(move || {
                        let _ = std::fs::remove_file(s);
                    })
                    .await;
                }
                refresh_remote(ctx, rel).await?;
                return Ok(Done::Changed);
            }
            Err(e) => return Err(object_failure(&e)),
        }
    }
    // 3. The bucket's version here, under the copy's name.
    let local_copy = scan::join(&ctx.root, &copy);
    let have_local_copy = {
        let p = local_copy.clone();
        blocking(move || p.symlink_metadata().is_ok()).await
    };
    if !have_local_copy {
        let (path, report) = match staged {
            Some(s) => s,
            None => match fetch(ctx, &copy, remote_size).await? {
                Fetched::Staged(p, r) => (p, r),
                Fetched::Waiting => {
                    // The copy is not there (yet): from the original.
                    match fetch(ctx, rel, remote_size).await? {
                        Fetched::Staged(p, r) if r.e_tag.as_deref() == Some(remote_etag) => (p, r),
                        Fetched::Staged(p, _) => {
                            blocking(move || {
                                let _ = std::fs::remove_file(p);
                            })
                            .await;
                            refresh_remote(ctx, rel).await?;
                            return Ok(Done::Changed);
                        }
                        Fetched::Waiting => return Ok(Done::Changed),
                    }
                }
            },
        };
        match place(ctx, &copy, &path, None, &report, "conflict-copy").await? {
            Done::Changed => {}
            other => return Ok(other),
        }
        if remote_size > MAX_COPY_BYTES && !copy_exists {
            // Uploaded as a new object by the next cycle (a new local file).
            ctx.relist.store(true, Ordering::SeqCst);
        } else {
            // Record the copy's object.
            let head = ctx
                .store
                .head_object(ctx.bucket(), &to)
                .await
                .map_err(|e| object_failure(&e))?;
            if let Some(h) = head {
                let c = copy.clone();
                ctx.db(move |s, id| {
                    if let Some(mut row) = s.synced(id, &c)? {
                        row.etag = h.e_tag.clone();
                        row.remote_size = Some(h.size);
                        s.put_synced(id, &row, now_ms())?;
                    }
                    s.set_remote(
                        id,
                        &c,
                        Some(&Remote {
                            path: c.clone(),
                            dir: false,
                            marker: false,
                            size: h.size,
                            etag: h.e_tag.clone(),
                            modified_ms: None,
                        }),
                    )
                })
                .await?;
            }
        }
    } else if let Some((s, _)) = staged {
        blocking(move || {
            let _ = std::fs::remove_file(s);
        })
        .await;
    }
    // 4. This computer's version over the bucket's, under the original name.
    let path = scan::join(&ctx.root, rel);
    let shown = Shown::new(ctx, rel, TransferKind::Upload, stamp.size);
    let progress = shown.progress();
    let (control, reg) = transfer_control(ctx);
    let result = ctx
        .store
        .upload_with(
            ctx.bucket(),
            &from,
            &path,
            &UploadOptions {
                destination: Destination::Replace {
                    e_tag: remote_etag.to_owned(),
                },
                mtime_ms: stamp.mtime_ms(),
            },
            &control,
            &mut |p| {
                if p.stage == UploadStage::Sending {
                    progress(p.bytes);
                }
            },
        )
        .await;
    ctx.control.unregister(reg);
    drop(shown);
    match result {
        Ok(report) if report.sha256 == sha => {
            let row = synced_of(rel, stamp, sha, report.e_tag.clone(), report.size);
            let remote = Remote {
                path: rel.to_owned(),
                dir: false,
                marker: false,
                size: report.size,
                etag: report.e_tag.clone(),
                modified_ms: None,
            };
            let done = ConflictItem {
                state: "kept-both".into(),
                ..item_row
            };
            let (p, data) = (
                rel.to_owned(),
                serde_json::to_string(&done).unwrap_or_default(),
            );
            ctx.db(move |s, id| {
                s.put_synced(id, &row, now_ms())?;
                s.set_remote(id, &row.path, Some(&remote))?;
                s.put_issue(id, "conflict", &p, &data, now_ms())
            })
            .await?;
            log(
                ctx,
                rel,
                "conflict",
                report.size,
                Some(format!(
                    "kept both versions; the bucket's version is now {copy}"
                )),
            )
            .await;
            Ok(Done::Changed)
        }
        Ok(_) => {
            // The file changed while it was uploaded: the next cycle sees
            // it as a change on top.
            refresh_local(ctx, rel).await?;
            Ok(Done::Changed)
        }
        Err(e) => match &e.error {
            StorageError::AlreadyExists(_) | StorageError::NotFound(_) => {
                refresh_remote(ctx, rel).await?;
                Ok(Done::Changed)
            }
            StorageError::FileChanged(_) => {
                refresh_local(ctx, rel).await?;
                Ok(Done::Changed)
            }
            other => Err(object_failure(other)),
        },
    }
}

async fn delete_remote(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let Some(synced) = &t.synced else {
        return Ok(Done::Waiting);
    };
    let rel = t.path.as_str();
    // Still gone here?
    let path = scan::join(&ctx.root, rel);
    if blocking(move || path.symlink_metadata().is_ok()).await {
        refresh_local(ctx, rel).await?;
        return Ok(Done::Changed);
    }
    // Still the synced object there?
    let key = ctx.key(rel, false);
    let head = ctx
        .store
        .head_object(ctx.bucket(), &key)
        .await
        .map_err(|e| object_failure(&e))?;
    match head {
        None => {
            let p = rel.to_owned();
            ctx.db(move |s, id| {
                s.remove_synced(id, &p)?;
                s.set_remote(id, &p, None)
            })
            .await?;
            return Ok(Done::Changed);
        }
        Some(h) if h.e_tag != synced.etag || Some(h.size) != synced.remote_size => {
            // Changed in the bucket: the change wins over the deletion.
            refresh_remote(ctx, rel).await?;
            return Ok(Done::Changed);
        }
        Some(_) => {}
    }
    let outcome = ctx
        .store
        .delete_reporting(ctx.bucket(), &key, None)
        .await
        .map_err(|e| object_failure(&e))?;
    let p = rel.to_owned();
    ctx.db(move |s, id| {
        s.remove_synced(id, &p)?;
        s.set_remote(id, &p, None)
    })
    .await?;
    log(
        ctx,
        rel,
        "delete-remote",
        0,
        Some(if outcome.marker_created {
            "deleted in the bucket; it keeps the earlier version, so it can be restored from Deleted items".into()
        } else {
            "deleted in the bucket".into()
        }),
    )
    .await;
    Ok(Done::Changed)
}

async fn delete_local(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let Some(synced) = &t.synced else {
        return Ok(Done::Waiting);
    };
    let rel = t.path.as_str();
    // Still gone in the bucket?
    let head = ctx
        .store
        .head_object(ctx.bucket(), &ctx.key(rel, false))
        .await
        .map_err(|e| object_failure(&e))?;
    if head.is_some() {
        refresh_remote(ctx, rel).await?;
        return Ok(Done::Changed);
    }
    // Still the synced file here? Then into the sync trash.
    let expected = Stamp {
        size: synced.size,
        mtime_ns: synced.mtime_ns,
        file_id: synced.file_id,
    };
    let (root, r) = (ctx.root.clone(), rel.to_owned());
    let moved = blocking(move || -> std::io::Result<Option<PathBuf>> {
        if scan::stamp(&scan::join(&root, &r))? != Some(expected) {
            return Ok(None);
        }
        scan::prepare_own_dir(&root)?;
        scan::move_to_trash(&root, &r, now_ms()).map(Some)
    })
    .await
    .map_err(|e| io_failure(&e))?;
    let Some(_) = moved else {
        refresh_local(ctx, rel).await?;
        return Ok(Done::Changed);
    };
    let p = rel.to_owned();
    ctx.db(move |s, id| {
        s.remove_synced(id, &p)?;
        s.set_local(id, &p, None)
    })
    .await?;
    log(
        ctx,
        rel,
        "delete-local",
        synced.size,
        Some("deleted in the bucket; moved to the sync trash here".into()),
    )
    .await;
    Ok(Done::Changed)
}

async fn make_local_folder(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let rel = t.path.clone();
    let path = scan::join(&ctx.root, &rel);
    let made = blocking(move || -> std::io::Result<bool> {
        match path.symlink_metadata() {
            Ok(m) if m.is_dir() => Ok(true),
            Ok(_) => Ok(false),
            Err(_) => std::fs::create_dir_all(&path).map(|()| true),
        }
    })
    .await
    .map_err(|e| io_failure(&e))?;
    if !made {
        refresh_local(ctx, &rel).await?;
        return Ok(Done::Changed);
    }
    ctx.db(move |s, id| {
        s.put_synced(
            id,
            &Synced {
                path: rel.clone(),
                dir: true,
                ..Synced::default()
            },
            now_ms(),
        )?;
        s.set_local(
            id,
            &rel,
            Some(&Local {
                path: rel.clone(),
                dir: true,
                ..Local::default()
            }),
        )
    })
    .await?;
    Ok(Done::Changed)
}

async fn make_remote_folder(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let rel = t.path.clone();
    // A folder with contents comes to the bucket with them.
    let r = rel.clone();
    if ctx.db(move |s, id| s.has_local_children(id, &r)).await? {
        return Ok(Done::Waiting);
    }
    ctx.store
        .put_folder_marker(ctx.bucket(), &ctx.key(&rel, true))
        .await
        .map_err(|e| object_failure(&e))?;
    ctx.db(move |s, id| {
        s.put_synced(
            id,
            &Synced {
                path: rel.clone(),
                dir: true,
                ..Synced::default()
            },
            now_ms(),
        )?;
        s.set_remote(
            id,
            &rel,
            Some(&Remote {
                path: rel.clone(),
                dir: true,
                marker: true,
                ..Remote::default()
            }),
        )
    })
    .await?;
    log(
        ctx,
        &t.path,
        "create-folder",
        0,
        Some("created in the bucket".into()),
    )
    .await;
    Ok(Done::Changed)
}

async fn remove_remote_folder(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let rel = t.path.clone();
    // Gone here still?
    let path = scan::join(&ctx.root, &rel);
    if blocking(move || path.symlink_metadata().is_ok()).await {
        refresh_local(ctx, &rel).await?;
        return Ok(Done::Changed);
    }
    let r = rel.clone();
    if ctx.db(move |s, id| s.has_remote_children(id, &r)).await? {
        // Its files go one by one (or come back here if they changed).
        return Ok(Done::Waiting);
    }
    if t.remote.as_ref().is_some_and(|r| r.marker) {
        ctx.store
            .delete_reporting(ctx.bucket(), &ctx.key(&rel, true), None)
            .await
            .map_err(|e| object_failure(&e))?;
    }
    ctx.db(move |s, id| {
        s.remove_synced(id, &rel)?;
        s.set_remote(id, &rel, None)
    })
    .await?;
    Ok(Done::Changed)
}

async fn remove_local_folder(ctx: &Arc<Ctx>, t: &Triple) -> Result<Done, Failure> {
    let rel = t.path.clone();
    let path = scan::join(&ctx.root, &rel);
    // Only an empty folder goes; anything still in it is synced first.
    let removed = blocking(move || -> std::io::Result<bool> {
        match std::fs::read_dir(&path) {
            Ok(mut d) => {
                if d.next().is_some() {
                    return Ok(false);
                }
                std::fs::remove_dir(&path).map(|()| true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e),
        }
    })
    .await
    .map_err(|e| io_failure(&e))?;
    if !removed {
        return Ok(Done::Waiting);
    }
    ctx.db(move |s, id| {
        s.remove_synced(id, &rel)?;
        s.set_local(id, &rel, None)
    })
    .await?;
    Ok(Done::Changed)
}

/// The page's figures once a pass ended.
async fn finish_status(ctx: &Arc<Ctx>, clean: bool) {
    let counts = ctx
        .db(|s, id| {
            let (files, bytes, folders) = s.synced_counts(id)?;
            let (remote_files, remote_bytes) = s.remote_counts(id)?;
            let issues = s.issues(id)?;
            Ok((files, bytes, folders, remote_files, remote_bytes, issues))
        })
        .await;
    let Ok((files, bytes, folders, remote_files, remote_bytes, issues)) = counts else {
        return;
    };
    let count = |kind: &str| issues.iter().filter(|i| i.kind == kind).count() as u64;
    let failed = count("failed");
    let skipped = count("skipped-local") + count("skipped-remote") + count("skipped-case");
    let conflicts = issues
        .iter()
        .filter(|i| i.kind == "conflict")
        .filter(|i| {
            serde_json::from_str::<ConflictItem>(&i.data).is_ok_and(|c| c.state == "kept-both")
        })
        .count() as u64;
    let held = issues
        .iter()
        .find(|i| i.kind == "deletions")
        .and_then(|i| serde_json::from_str::<HeldDeletions>(&i.data).ok())
        .filter(|h| !h.approved);
    let root = ctx.root.clone();
    let shared = Arc::clone(&ctx.shared);
    let (trash, free) =
        blocking(move || (scan::trash_usage(&root), shared.free_space(&root))).await;
    let disk_full = ctx.disk_full.load(Ordering::SeqCst);
    let now = now_ms();
    let in_step = clean && held.is_none() && failed == 0;
    ctx.shared.update(ctx.id(), |s| {
        s.files = files;
        s.bytes = bytes;
        s.folders = folders;
        s.remote_files = remote_files;
        s.remote_bytes = remote_bytes;
        s.failed = failed;
        s.skipped = skipped;
        s.conflicts = conflicts;
        s.held_deletions = held.clone();
        s.trash_files = trash.0;
        s.trash_bytes = trash.1;
        s.free_bytes = free;
        s.disk_full = disk_full;
        s.error = None;
        if in_step {
            s.state = SyncState::Synced;
            s.message = None;
            s.pending_uploads = 0;
            s.pending_upload_bytes = 0;
            s.pending_downloads = 0;
            s.pending_download_bytes = 0;
            s.last_synced_ms = Some(now);
        } else {
            s.state = SyncState::Synced;
            s.message = Some(if held.is_some() {
                "Many files would be deleted at once; sync waits for your answer.".into()
            } else if disk_full {
                "The drive of the sync folder is full; downloads wait until there is space.".into()
            } else if failed > 0 {
                format!(
                    "{failed} {} could not be synced; sync tries again automatically.",
                    if failed == 1 { "item" } else { "items" }
                )
            } else {
                "Some changes wait (a file still being written, or a retry).".into()
            });
            s.state = SyncState::Attention;
        }
    });
    if in_step {
        let mut config = ctx.config.clone();
        config.last_synced_ms = Some(now);
        let data = serde_json::to_string(&config).unwrap_or_default();
        let id = config.id.clone();
        if let Err(e) = ctx.shared.db(move |s| s.save_config(&id, &data)).await {
            eprintln!("sotyware: cannot save the folder sync: {}", e.message);
        }
    }
}

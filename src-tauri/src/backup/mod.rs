//! Backups and disaster recovery (ADR-031): jobs that keep an independent,
//! verified, versioned copy of an account's bucket (or a folder of it) in a
//! second destination, ideally another account at another provider.
//!
//! - [`model`]: job configuration, repository names, the snapshot manifest
//!   format, the change check, retention selection and health (pure).
//! - [`engine`]: the remote work: a run, verification, retention, restore.
//! - This module: [`Backups`] (managed state: the jobs and their history in
//!   `backup.sqlite3`, one task at a time in its own slot, the scheduler)
//!   and the commands of the Backup page.
//!
//! A backup task never deletes or changes a source object. The only
//! deletions are retention's, at the destination, after the newest snapshot
//! was verified and only when it is not suspicious (ADR-031). Removing a
//! job forgets its configuration and history on this computer and never
//! touches the backup data at the destination.
//!
//! Design: docs/architecture/backup.md.

pub mod engine;
pub mod model;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use crate::cloud::dashboard::{CloudAccounts, CommandError, ErrorKind};
use crate::cloud::download::CancelOutcome;
use crate::cloud::operations::operation_error;
use crate::cloud::s3::{S3Store, StorageError, Versioning};
use crate::features::StoreError;
use crate::journal::{Journal, now_ms};
use engine::{Cancel, Ctx, ObjectError, Progress, PruneReport, RestoreReport, VerifyReport};
use model::{Health, History, JobConfig, Location, Retention, RunState, Stats};

/// How long quitting waits for a cancelled task.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
/// The scheduler's first look after startup (so starting the app is not
/// slowed down by an overdue backup).
pub const SCHEDULER_DELAY: Duration = Duration::from_secs(120);
/// Runs kept per job in the history shown.
const HISTORY: usize = 50;

/// One task run as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    pub job_id: String,
    /// `backup`, `verify`, `verify-full`, `prune`, `restore`.
    pub kind: String,
    pub state: String,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub snapshot: Option<String>,
    pub suspicious: bool,
    pub passed: Option<bool>,
    /// JSON of the task's report.
    pub summary: Option<String>,
    pub error: Option<String>,
}

/// Where jobs and runs live. Implemented by `storage::SqliteBackupStore`.
pub trait BackupStore: Send {
    /// `(id, JSON configuration)` of every job.
    fn jobs(&mut self) -> Result<Vec<(String, String)>, StoreError>;
    fn save_job(&mut self, id: &str, config: &str, now_ms: i64) -> Result<(), StoreError>;
    /// Removes the job and its history (never remote data).
    fn remove_job(&mut self, id: &str) -> Result<(), StoreError>;
    fn add_run(&mut self, run: &RunRow) -> Result<i64, StoreError>;
    /// The newest `limit` runs of a job, newest first.
    fn runs(&mut self, job_id: &str, limit: usize) -> Result<Vec<RunRow>, StoreError>;
}

/// What a task does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskKind {
    /// A run, then a quick verification, then retention when allowed.
    Backup,
    Verify,
    VerifyFull,
    Prune,
    Restore,
}

/// The last run of one kind, as the page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub kind: String,
    pub state: String,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub snapshot: Option<String>,
    pub suspicious: bool,
    pub passed: Option<bool>,
    pub error: Option<String>,
    /// The task's report (JSON), for the details.
    pub summary: Option<String>,
}

impl From<&RunRow> for RunInfo {
    fn from(r: &RunRow) -> Self {
        Self {
            kind: r.kind.clone(),
            state: r.state.clone(),
            started_ms: r.started_ms,
            finished_ms: r.finished_ms,
            snapshot: r.snapshot.clone(),
            suspicious: r.suspicious,
            passed: r.passed,
            error: r.error.clone(),
            summary: r.summary.clone(),
        }
    }
}

/// One job with its health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub config: JobConfig,
    pub health: Health,
    /// Source and destination in one account: does not survive a
    /// compromised key or a closed account.
    pub same_account: bool,
    pub last_backup: Option<RunInfo>,
    pub last_verification: Option<RunInfo>,
    pub last_prune: Option<RunInfo>,
    /// The newest snapshot written, if any.
    pub last_snapshot: Option<String>,
    /// When the scheduler will run it next (interval > 0).
    pub next_due_ms: Option<i64>,
}

/// The running task, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTask {
    pub task: String,
    pub job: String,
    pub kind: TaskKind,
}

/// What `list_backups` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupList {
    pub storage_error: Option<String>,
    pub jobs: Vec<JobStatus>,
    pub active: Option<ActiveTask>,
}

/// One message of a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BackupEventBody {
    Progress { progress: Progress },
    Finished { summary: Box<TaskSummary> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEvent {
    pub task: String,
    pub seq: u64,
    #[serde(flatten)]
    pub body: BackupEventBody,
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub task: String,
    pub job: String,
    pub kind: TaskKind,
    /// `completed`, `partial`, `failed`, `cancelled`.
    pub state: String,
    pub snapshot: Option<String>,
    pub stats: Option<Stats>,
    pub suspicious: bool,
    pub verification: Option<VerifyReport>,
    pub prune: Option<PruneReport>,
    pub restore: Option<RestoreReport>,
    /// Why retention did not run after a backup, if it did not.
    pub prune_skipped: Option<String>,
    pub object_errors: Vec<ObjectError>,
    pub error: Option<CommandError>,
}

/// Receives a task's messages.
pub trait BackupSink: Send + Sync {
    fn emit(&self, event: BackupEvent);
}

impl BackupSink for Channel<BackupEvent> {
    fn emit(&self, event: BackupEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: cannot send a backup message to the dashboard: {e}");
        }
    }
}

/// A sink that drops everything (scheduled runs nobody watches).
struct Nowhere;

impl BackupSink for Nowhere {
    fn emit(&self, _: BackupEvent) {}
}

enum Slot {
    Free,
    Active {
        task: String,
        job: String,
        kind: TaskKind,
        cancel: Arc<Cancel>,
    },
}

/// Backups: managed state.
pub struct Backups {
    store: Mutex<Result<Box<dyn BackupStore>, String>>,
    slot: Mutex<Slot>,
    freed: Condvar,
    spool: PathBuf,
    journal: OnceLock<Arc<Journal>>,
    next: AtomicU64,
    closed: AtomicBool,
    /// Woken when jobs change, so the scheduler recomputes the next run.
    wake: Arc<tokio::sync::Notify>,
    scheduler: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// The link the `backup` built-in feature uses to start and stop the
/// scheduler; set once in `run()`.
pub static SCHEDULER_LINK: OnceLock<(Arc<Backups>, Arc<CloudAccounts>)> = OnceLock::new();

fn storage_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(
        ErrorKind::Storage,
        format!("the backup database cannot be used: {e}"),
    )
}

/// What an engine error means for the page.
pub fn backup_error(e: &StorageError) -> CommandError {
    match e {
        StorageError::Integrity(m) => CommandError::new(ErrorKind::Integrity, m.clone()),
        StorageError::Refused(m) => CommandError::new(ErrorKind::InvalidInput, m.clone()),
        StorageError::Network(_) | StorageError::Timeout(_) => CommandError::new(
            ErrorKind::Network,
            "the provider could not be reached or did not answer in time",
        ),
        other => operation_error(other),
    }
}

impl Backups {
    /// The service of `run()`: the opened store, and a spool folder sotyware
    /// owns (emptied now: anything there is a leftover of an earlier task).
    pub fn load(store: Result<Box<dyn BackupStore>, StoreError>, spool: PathBuf) -> Arc<Self> {
        clear_spool(&spool);
        if let Err(e) = std::fs::create_dir_all(&spool) {
            eprintln!("sotyware: cannot create the backup spool folder: {e}");
        }
        Arc::new(Self {
            store: Mutex::new(store.map_err(|e| {
                eprintln!("sotyware: the backup database cannot be used: {e}");
                e.to_string()
            })),
            slot: Mutex::new(Slot::Free),
            freed: Condvar::new(),
            spool,
            journal: OnceLock::new(),
            next: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            wake: Arc::new(tokio::sync::Notify::new()),
            scheduler: Mutex::new(None),
        })
    }

    /// Makes every transfer of a task record itself in `journal`. Once.
    pub fn record_in(&self, journal: Arc<Journal>) {
        let _ = self.journal.set(journal);
    }

    fn store(&self) -> MutexGuard<'_, Result<Box<dyn BackupStore>, String>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut dyn BackupStore) -> Result<T, StoreError>,
    ) -> Result<T, CommandError> {
        match &mut *self.store() {
            Ok(s) => f(s.as_mut()).map_err(storage_error),
            Err(e) => Err(storage_error(e)),
        }
    }

    /// Every configured job.
    pub fn jobs(&self) -> Result<Vec<JobConfig>, CommandError> {
        let rows = self.with_store(|s| s.jobs())?;
        Ok(rows
            .into_iter()
            .filter_map(
                |(id, json)| match serde_json::from_str::<JobConfig>(&json) {
                    Ok(j) => Some(j),
                    Err(e) => {
                        eprintln!("sotyware: backup job '{id}' cannot be read: {e}");
                        None
                    }
                },
            )
            .collect())
    }

    pub fn job(&self, id: &str) -> Result<JobConfig, CommandError> {
        self.jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .ok_or_else(|| CommandError::new(ErrorKind::InvalidInput, "no such backup job"))
    }

    fn runs(&self, id: &str) -> Result<Vec<RunRow>, CommandError> {
        self.with_store(|s| s.runs(id, HISTORY))
    }

    fn record(&self, run: RunRow) {
        if let Err(e) = self.with_store(|s| s.add_run(&run)) {
            eprintln!("sotyware: cannot record a backup run: {}", e.message);
        }
    }

    /// The job's health and last runs.
    pub fn status(&self, job: JobConfig, now: i64) -> Result<JobStatus, CommandError> {
        let runs = self.runs(&job.id)?;
        let last_backup = runs.iter().find(|r| r.kind == "backup");
        let last_snapshot_run = runs
            .iter()
            .find(|r| r.kind == "backup" && r.state == "completed" && r.snapshot.is_some());
        let last_verification = last_snapshot_run.and_then(|s| {
            runs.iter().find(|r| {
                (r.kind == "verify" || r.kind == "verify-full")
                    && r.snapshot == s.snapshot
                    && r.passed.is_some()
            })
        });
        let history = History {
            last_run: last_backup
                .and_then(|r| RunState::parse(&r.state).map(|s| (s, r.started_ms))),
            last_snapshot: last_snapshot_run
                .map(|r| (r.finished_ms.unwrap_or(r.started_ms), r.suspicious)),
            last_verification: last_verification.map(|r| {
                (
                    r.finished_ms.unwrap_or(r.started_ms),
                    r.passed.unwrap_or(false),
                )
            }),
        };
        Ok(JobStatus {
            health: model::health(&history, job.interval_hours, now),
            same_account: model::same_account(&job),
            last_backup: last_backup.map(RunInfo::from),
            last_verification: last_verification.map(RunInfo::from),
            last_prune: runs.iter().find(|r| r.kind == "prune").map(RunInfo::from),
            last_snapshot: last_snapshot_run.and_then(|r| r.snapshot.clone()),
            next_due_ms: next_due(&job, last_backup, now),
            config: job,
        })
    }

    /// Every job with its health, and the running task.
    pub fn list(&self) -> BackupList {
        let now = now_ms();
        let (jobs, error) = match self.jobs() {
            Ok(jobs) => {
                let mut out = Vec::new();
                let mut error = None;
                for j in jobs {
                    match self.status(j, now) {
                        Ok(s) => out.push(s),
                        Err(e) => error = Some(e.message),
                    }
                }
                (out, error)
            }
            Err(e) => (Vec::new(), Some(e.message)),
        };
        BackupList {
            storage_error: error,
            jobs,
            active: match &*self.slot() {
                Slot::Active {
                    task, job, kind, ..
                } => Some(ActiveTask {
                    task: task.clone(),
                    job: job.clone(),
                    kind: *kind,
                }),
                Slot::Free => None,
            },
        }
    }

    /// Saves a job (new or changed) after validation, and wakes the
    /// scheduler.
    pub fn save(&self, job: &JobConfig) -> Result<(), CommandError> {
        model::validate(job).map_err(|e| CommandError::new(ErrorKind::InvalidInput, e))?;
        let json = serde_json::to_string(job)
            .map_err(|e| CommandError::new(ErrorKind::Other, e.to_string()))?;
        self.with_store(|s| s.save_job(&job.id, &json, now_ms()))?;
        self.wake.notify_one();
        Ok(())
    }

    /// Forgets a job and its history here; the backup data stays.
    pub fn remove(&self, id: &str) -> Result<(), CommandError> {
        if matches!(&*self.slot(), Slot::Active { job, .. } if job == id) {
            return Err(CommandError::new(
                ErrorKind::Busy,
                "a task of this backup job is running; cancel it first",
            ));
        }
        self.with_store(|s| s.remove_job(id))?;
        self.wake.notify_one();
        Ok(())
    }

    fn reserve(&self, job: &str, kind: TaskKind) -> Result<(String, Arc<Cancel>), CommandError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CommandError::new(ErrorKind::Busy, "sotyware is quitting"));
        }
        let mut slot = self.slot();
        if let Slot::Active { job: other, .. } = &*slot {
            return Err(CommandError::new(
                ErrorKind::Busy,
                format!("a backup task of '{other}' is running; wait for it or cancel it"),
            ));
        }
        let task = format!(
            "backup-{}-{}",
            std::process::id(),
            self.next.fetch_add(1, Ordering::Relaxed) + 1
        );
        let cancel = Arc::new(Cancel::default());
        *slot = Slot::Active {
            task: task.clone(),
            job: job.to_owned(),
            kind,
            cancel: Arc::clone(&cancel),
        };
        Ok((task, cancel))
    }

    fn free(&self) {
        *self.slot() = Slot::Free;
        self.freed.notify_all();
    }

    /// Cancels task `task`; any other id is left alone.
    pub fn cancel(&self, task: &str) -> CancelOutcome {
        match &*self.slot() {
            Slot::Active {
                task: t, cancel, ..
            } if t == task => {
                cancel.cancel();
                CancelOutcome::Cancelling
            }
            _ => CancelOutcome::NotActive,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(&*self.slot(), Slot::Active { .. })
    }

    /// Cancels the running task and waits for it, bounded; stops the
    /// scheduler.
    pub fn shutdown(&self, wait: Duration) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop_scheduler();
        let deadline = std::time::Instant::now() + wait;
        let mut slot = self.slot();
        if let Slot::Active { cancel, .. } = &*slot {
            cancel.cancel();
        }
        while matches!(&*slot, Slot::Active { .. }) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                eprintln!("sotyware: a backup task did not stop in time while quitting");
                break;
            }
            slot = self
                .freed
                .wait_timeout(slot, left)
                .map_or_else(|e| e.into_inner().0, |(g, _)| g);
        }
    }

    /// Starts `kind` for `job` in the slot; the task reports to `sink`.
    pub async fn start(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        job: &str,
        kind: TaskKind,
        sink: Arc<dyn BackupSink>,
    ) -> Result<String, CommandError> {
        if kind == TaskKind::Restore {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a restore is started with its target",
            ));
        }
        let config = self.job(job)?;
        let (task, cancel) = self.reserve(job, kind)?;
        let stores = match prepare(&accounts, &config).await {
            Ok(s) => s,
            Err(e) => {
                self.free();
                return Err(e);
            }
        };
        let this = Arc::clone(self);
        let t = task.clone();
        tauri::async_runtime::spawn(async move {
            let summary = this
                .execute(&t, &config, kind, &stores, &cancel, &sink)
                .await;
            this.free();
            sink.emit(BackupEvent {
                task: t,
                seq: u64::MAX,
                body: BackupEventBody::Finished {
                    summary: Box::new(summary),
                },
            });
        });
        Ok(task)
    }

    /// Restores `snapshot` of `job` (entries under `filter`) to `target`.
    pub async fn start_restore(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        job: &str,
        snapshot: &str,
        filter: String,
        target: RestoreTo,
        sink: Arc<dyn BackupSink>,
    ) -> Result<String, CommandError> {
        let config = self.job(job)?;
        let snapshots_prefix = model::snapshots_prefix(&config.destination.prefix, &config.id);
        if !snapshot.starts_with(&snapshots_prefix) || !snapshot.ends_with(".json") {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "this is not a snapshot of this backup job",
            ));
        }
        let (task, cancel) = self.reserve(job, TaskKind::Restore)?;
        let stores = match prepare(&accounts, &config).await {
            Ok(s) => s,
            Err(e) => {
                self.free();
                return Err(e);
            }
        };
        let target_store = match &target {
            RestoreTo::Bucket { account, .. } => {
                let acc = Arc::clone(&accounts);
                let account = account.clone();
                match crate::cloud::dashboard::blocking_pub(move || {
                    acc.prepare_own_bucket(&account)
                })
                .await
                {
                    Ok(s) => Some(s),
                    Err(e) => {
                        self.free();
                        return Err(e);
                    }
                }
            }
            RestoreTo::Local(_) => None,
        };
        let this = Arc::clone(self);
        let (t, snapshot) = (task.clone(), snapshot.to_owned());
        tauri::async_runtime::spawn(async move {
            let summary = this
                .execute_restore(
                    &t,
                    &config,
                    &stores,
                    &snapshot,
                    &filter,
                    &target,
                    target_store,
                    &cancel,
                    &sink,
                )
                .await;
            this.free();
            sink.emit(BackupEvent {
                task: t,
                seq: u64::MAX,
                body: BackupEventBody::Finished {
                    summary: Box::new(summary),
                },
            });
        });
        Ok(task)
    }

    fn summary(&self, task: &str, job: &str, kind: TaskKind) -> TaskSummary {
        TaskSummary {
            task: task.into(),
            job: job.into(),
            kind,
            state: "failed".into(),
            snapshot: None,
            stats: None,
            suspicious: false,
            verification: None,
            prune: None,
            restore: None,
            prune_skipped: None,
            object_errors: Vec::new(),
            error: None,
        }
    }

    /// Runs a task to its end and records it.
    async fn execute(
        &self,
        task: &str,
        job: &JobConfig,
        kind: TaskKind,
        stores: &Stores,
        cancel: &Cancel,
        sink: &Arc<dyn BackupSink>,
    ) -> TaskSummary {
        let mut seq = 0u64;
        let mut progress = |p: Progress| {
            seq += 1;
            sink.emit(BackupEvent {
                task: task.into(),
                seq,
                body: BackupEventBody::Progress { progress: p },
            });
        };
        let spool = self.task_spool(task);
        let journal = self.journal.get().cloned();
        let mut ctx = Ctx {
            dest: &stores.dest,
            dest_bucket: &job.destination.bucket,
            dest_prefix: &job.destination.prefix,
            spool: &spool,
            cancel,
            journal: journal.as_ref(),
            progress: &mut progress,
        };
        let mut s = self.summary(task, &job.id, kind);
        let started = now_ms();
        match kind {
            TaskKind::Backup => {
                let source = engine::Source {
                    store: &stores.source,
                    bucket: &job.source.bucket,
                    prefix: &job.source.prefix,
                    provider: &stores.source_provider,
                    endpoint: &stores.source_endpoint,
                };
                let r = engine::run(&mut ctx, &source, job).await;
                s.state = r.state.as_str().into();
                s.snapshot = r.snapshot.clone();
                s.stats = Some(r.stats);
                s.suspicious = r.suspicious;
                s.object_errors = r.object_errors.clone();
                s.error = r.error.as_ref().map(backup_error);
                self.record(RunRow {
                    job_id: job.id.clone(),
                    kind: "backup".into(),
                    state: s.state.clone(),
                    started_ms: started,
                    finished_ms: Some(now_ms()),
                    snapshot: r.snapshot.clone(),
                    suspicious: r.suspicious,
                    summary: serde_json::to_string(&(r.stats, &r.object_errors)).ok(),
                    error: s.error.as_ref().map(|e| e.message.clone()),
                    ..RunRow::default()
                });
                if let Some(key) = r.snapshot {
                    let passed = self
                        .verify_and_record(&mut ctx, job, &key, false, &mut s)
                        .await;
                    if !passed {
                        s.prune_skipped = Some("the new snapshot did not pass verification".into());
                    } else if r.suspicious {
                        s.prune_skipped = Some(
                            "many objects changed or disappeared since the previous snapshot; older snapshots are kept until you review it"
                                .into(),
                        );
                    } else if !job.retention.keeps_everything() {
                        self.prune_and_record(&mut ctx, job, &mut s).await;
                    }
                }
            }
            TaskKind::Verify | TaskKind::VerifyFull => {
                match engine::snapshots(&ctx, &job.id).await {
                    Ok(list) => match list.last() {
                        Some(last) => {
                            let key = last.key.clone();
                            self.verify_and_record(
                                &mut ctx,
                                job,
                                &key,
                                kind == TaskKind::VerifyFull,
                                &mut s,
                            )
                            .await;
                            s.snapshot = Some(key);
                        }
                        None => {
                            s.error = Some(CommandError::new(
                                ErrorKind::InvalidInput,
                                "this backup has no snapshot yet",
                            ));
                        }
                    },
                    Err(e) => s.error = Some(backup_error(&e)),
                }
                if s.error.is_none() {
                    s.state = if s.verification.as_ref().is_some_and(|v| v.passed) {
                        "completed".into()
                    } else {
                        "failed".into()
                    };
                } else if matches!(s.error.as_ref().map(|e| e.kind), Some(ErrorKind::Other))
                    && cancel.is_cancelled()
                {
                    s.state = "cancelled".into();
                }
            }
            TaskKind::Prune => {
                // Retention only on a verified, unsuspicious newest snapshot.
                let status = self.status(job.clone(), now_ms());
                match status {
                    Ok(st) if st.health == Health::Healthy || st.health == Health::OutOfDate => {
                        self.prune_and_record(&mut ctx, job, &mut s).await;
                    }
                    Ok(st) => {
                        s.prune_skipped = Some(format!(
                            "retention runs only after the newest snapshot passed verification and is not marked as a large change (now: {})",
                            serde_json::to_value(st.health)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_owned))
                                .unwrap_or_default()
                        ));
                        s.state = "failed".into();
                    }
                    Err(e) => s.error = Some(e),
                }
            }
            TaskKind::Restore => {}
        }
        clear_task_spool(&spool);
        s
    }

    async fn verify_and_record(
        &self,
        ctx: &mut Ctx<'_>,
        job: &JobConfig,
        key: &str,
        deep: bool,
        s: &mut TaskSummary,
    ) -> bool {
        let started = now_ms();
        let r = engine::verify(ctx, key, deep).await;
        let (state, passed, error) = match &r {
            Ok(v) => ("completed", Some(v.passed), None),
            Err(StorageError::Cancelled) => (
                "cancelled",
                None,
                Some(backup_error(&StorageError::Cancelled)),
            ),
            Err(e) => ("failed", None, Some(backup_error(e))),
        };
        self.record(RunRow {
            job_id: job.id.clone(),
            kind: if deep { "verify-full" } else { "verify" }.into(),
            state: state.into(),
            started_ms: started,
            finished_ms: Some(now_ms()),
            snapshot: Some(key.into()),
            passed,
            summary: r.as_ref().ok().and_then(|v| serde_json::to_string(v).ok()),
            error: error.as_ref().map(|e| e.message.clone()),
            ..RunRow::default()
        });
        if let Some(e) = error
            && s.error.is_none()
        {
            s.error = Some(e);
        }
        let ok = r.as_ref().is_ok_and(|v| v.passed);
        s.verification = r.ok();
        ok
    }

    async fn prune_and_record(&self, ctx: &mut Ctx<'_>, job: &JobConfig, s: &mut TaskSummary) {
        let started = now_ms();
        let r = engine::prune(ctx, job).await;
        let (state, error) = match &r {
            Ok(_) => ("completed", None),
            Err(StorageError::Cancelled) => ("cancelled", None),
            Err(e) => ("failed", Some(backup_error(e))),
        };
        self.record(RunRow {
            job_id: job.id.clone(),
            kind: "prune".into(),
            state: state.into(),
            started_ms: started,
            finished_ms: Some(now_ms()),
            summary: r.as_ref().ok().and_then(|p| serde_json::to_string(p).ok()),
            error: error.as_ref().map(|e| e.message.clone()),
            ..RunRow::default()
        });
        if s.kind == TaskKind::Prune {
            s.state = state.into();
        }
        if let Some(e) = error
            && s.error.is_none()
        {
            s.error = Some(e);
        }
        s.prune = r.ok();
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_restore(
        &self,
        task: &str,
        job: &JobConfig,
        stores: &Stores,
        snapshot: &str,
        filter: &str,
        target: &RestoreTo,
        target_store: Option<(Arc<S3Store>, String)>,
        cancel: &Cancel,
        sink: &Arc<dyn BackupSink>,
    ) -> TaskSummary {
        let mut seq = 0u64;
        let mut progress = |p: Progress| {
            seq += 1;
            sink.emit(BackupEvent {
                task: task.into(),
                seq,
                body: BackupEventBody::Progress { progress: p },
            });
        };
        let spool = self.task_spool(task);
        let journal = self.journal.get().cloned();
        let mut ctx = Ctx {
            dest: &stores.dest,
            dest_bucket: &job.destination.bucket,
            dest_prefix: &job.destination.prefix,
            spool: &spool,
            cancel,
            journal: journal.as_ref(),
            progress: &mut progress,
        };
        let mut s = self.summary(task, &job.id, TaskKind::Restore);
        s.snapshot = Some(snapshot.into());
        let started = now_ms();
        let result = async {
            let manifest = engine::read_manifest(&ctx, snapshot).await?;
            let t = match (target, &target_store) {
                (RestoreTo::Local(dir), _) => engine::Target::Local(dir),
                (RestoreTo::Bucket { prefix, .. }, Some((store, bucket))) => {
                    engine::Target::Bucket {
                        store,
                        bucket,
                        prefix,
                    }
                }
                (RestoreTo::Bucket { .. }, None) => {
                    return Err(StorageError::Refused("no target account".into()));
                }
            };
            engine::restore(&mut ctx, &manifest, filter, &t).await
        }
        .await;
        match result {
            Ok(r) => {
                s.state = if r.failed.is_empty() {
                    "completed"
                } else {
                    "partial"
                }
                .into();
                s.object_errors = r.failed.clone();
                s.restore = Some(r);
            }
            Err(StorageError::Cancelled) => s.state = "cancelled".into(),
            Err(e) => s.error = Some(backup_error(&e)),
        }
        self.record(RunRow {
            job_id: job.id.clone(),
            kind: "restore".into(),
            state: s.state.clone(),
            started_ms: started,
            finished_ms: Some(now_ms()),
            snapshot: Some(snapshot.into()),
            summary: s
                .restore
                .as_ref()
                .and_then(|r| serde_json::to_string(r).ok()),
            error: s.error.as_ref().map(|e| e.message.clone()),
            ..RunRow::default()
        });
        clear_task_spool(&spool);
        s
    }

    fn task_spool(&self, task: &str) -> PathBuf {
        let p = self.spool.join(task);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// The scheduler: one task that sleeps until the next job is due (or
    /// until jobs change), runs it, and goes back to sleep. No polling.
    pub fn start_scheduler(self: &Arc<Self>, accounts: Arc<CloudAccounts>) {
        let this = Arc::clone(self);
        let handle = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SCHEDULER_DELAY).await;
            loop {
                if this.closed.load(Ordering::SeqCst) {
                    return;
                }
                let now = now_ms();
                let due = this.next_job(now);
                let wait = match &due {
                    Some((_, at)) if *at <= now => Duration::ZERO,
                    Some((_, at)) => Duration::from_millis(u64::try_from(*at - now).unwrap_or(0)),
                    None => Duration::from_secs(24 * 3600),
                };
                if !wait.is_zero() {
                    // Woken early when jobs change: recompute.
                    let _ = tokio::time::timeout(wait, this.wake.notified()).await;
                    continue;
                }
                let Some((job, _)) = due else { continue };
                if this.busy() {
                    // Another task runs: look again once a minute until free.
                    let _ =
                        tokio::time::timeout(Duration::from_secs(60), this.wake.notified()).await;
                    continue;
                }
                match this
                    .start(
                        Arc::clone(&accounts),
                        &job,
                        TaskKind::Backup,
                        Arc::new(Nowhere),
                    )
                    .await
                {
                    Ok(_) => {
                        // Wait for it to end before choosing the next one.
                        while this.busy() {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "sotyware: a scheduled backup of '{job}' could not start: {}",
                            e.message
                        );
                        this.record(RunRow {
                            job_id: job.clone(),
                            kind: "backup".into(),
                            state: "failed".into(),
                            started_ms: now,
                            finished_ms: Some(now_ms()),
                            error: Some(e.message),
                            ..RunRow::default()
                        });
                    }
                }
            }
        });
        if let Some(old) = self
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(handle)
        {
            old.abort();
        }
    }

    pub fn stop_scheduler(&self) {
        if let Some(h) = self
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            h.abort();
        }
    }

    /// The job due first, with its due time.
    pub fn next_job(&self, now: i64) -> Option<(String, i64)> {
        let jobs = self.jobs().ok()?;
        jobs.into_iter()
            .filter_map(|j| {
                let last = self.runs(&j.id).ok()?;
                let due = next_due(&j, last.iter().find(|r| r.kind == "backup"), now)?;
                Some((j.id, due))
            })
            .min_by_key(|(_, due)| *due)
    }
}

/// When `job` is due: its interval after the last backup run started, or
/// now if it never ran; `None` for a manual job.
pub fn next_due(job: &JobConfig, last: Option<&RunRow>, now: i64) -> Option<i64> {
    if job.interval_hours == 0 {
        return None;
    }
    Some(last.map_or(now, |r| {
        r.started_ms + i64::from(job.interval_hours) * 3_600_000
    }))
}

/// The two stores of a job, ready to send requests.
pub struct Stores {
    pub source: Arc<S3Store>,
    pub dest: Arc<S3Store>,
    pub source_provider: String,
    pub source_endpoint: String,
}

async fn prepare(accounts: &Arc<CloudAccounts>, job: &JobConfig) -> Result<Stores, CommandError> {
    let acc = Arc::clone(accounts);
    let job = job.clone();
    crate::cloud::dashboard::blocking_pub(move || {
        let source = acc.prepare_download(&job.source.account, &job.source.bucket)?;
        let dest = acc.prepare_download(&job.destination.account, &job.destination.bucket)?;
        let (provider, endpoint) = acc.provider_and_endpoint(&job.source.account)?;
        Ok(Stores {
            source,
            dest,
            source_provider: provider,
            source_endpoint: endpoint,
        })
    })
    .await
}

/// Where a restore goes, as the command receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreTo {
    Local(PathBuf),
    Bucket { account: String, prefix: String },
}

// --------------------------------------------------------------- commands

/// What the page sends to create or change a job: the buckets come from the
/// accounts' configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobInput {
    /// `None` for a new job (an id is made from the name).
    pub id: Option<String>,
    pub name: String,
    pub source_account: String,
    pub source_prefix: String,
    pub destination_account: String,
    pub destination_prefix: String,
    pub interval_hours: u32,
    pub retention: Retention,
    pub suspicious_percent: u8,
}

/// The job a [`JobInput`] describes, with the accounts' buckets.
pub fn job_from_input(
    input: JobInput,
    accounts: &CloudAccounts,
    existing: &[JobConfig],
) -> Result<JobConfig, CommandError> {
    let bucket = |account: &str| -> Result<String, CommandError> {
        accounts
            .list()
            .accounts
            .into_iter()
            .find(|a| a.id == account)
            .and_then(|a| a.bucket)
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::UnknownAccount,
                    format!("no account '{account}' with a bucket"),
                )
            })
    };
    let id = match input.id {
        Some(id) => {
            if !existing.iter().any(|j| j.id == id) {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "no such backup job",
                ));
            }
            id
        }
        None => {
            let base = crate::cloud::dashboard::id_from_label(&input.name);
            let base = if base.is_empty() {
                "backup".to_owned()
            } else {
                base
            };
            let mut id = base.clone();
            let mut n = 2;
            while existing.iter().any(|j| j.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            id
        }
    };
    Ok(JobConfig {
        id,
        name: input.name.trim().to_owned(),
        source: Location {
            bucket: bucket(&input.source_account)?,
            account: input.source_account,
            prefix: input.source_prefix,
        },
        destination: Location {
            bucket: bucket(&input.destination_account)?,
            account: input.destination_account,
            prefix: input.destination_prefix,
        },
        interval_hours: input.interval_hours,
        retention: input.retention,
        suspicious_percent: input.suspicious_percent,
    })
}

/// Every backup job with its health. Allowed by `allow-list-backups`.
#[tauri::command]
pub async fn list_backups(backups: State<'_, Arc<Backups>>) -> Result<BackupList, CommandError> {
    let b = Arc::clone(&backups);
    crate::cloud::dashboard::blocking_pub(move || Ok(b.list())).await
}

/// Creates or changes a job. Allowed by `allow-save-backup-job`.
#[tauri::command]
pub async fn save_backup_job(
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: JobInput,
) -> Result<JobConfig, CommandError> {
    let (b, a) = (Arc::clone(&backups), Arc::clone(&accounts));
    crate::cloud::dashboard::blocking_pub(move || {
        let existing = b.jobs()?;
        let config = job_from_input(job, &a, &existing)?;
        b.save(&config)?;
        Ok(config)
    })
    .await
}

/// Forgets a job here (never its backup data). Allowed by
/// `allow-remove-backup-job`.
#[tauri::command]
pub async fn remove_backup_job(
    backups: State<'_, Arc<Backups>>,
    id: String,
) -> Result<(), CommandError> {
    let b = Arc::clone(&backups);
    crate::cloud::dashboard::blocking_pub(move || b.remove(&id)).await
}

/// Starts a backup, verification or retention task. Allowed by
/// `allow-start-backup-task`.
#[tauri::command]
pub async fn start_backup_task(
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: String,
    kind: TaskKind,
    on_event: Channel<BackupEvent>,
) -> Result<String, CommandError> {
    Arc::clone(&backups)
        .start(Arc::clone(&accounts), &job, kind, Arc::new(on_event))
        .await
}

/// Cancels a running task. Allowed by `allow-cancel-backup-task`.
#[tauri::command]
pub fn cancel_backup_task(backups: State<'_, Arc<Backups>>, task: String) -> CancelOutcome {
    backups.cancel(&task)
}

/// A job's snapshots at its destination, oldest first. Allowed by
/// `allow-list-backup-snapshots`.
#[tauri::command]
pub async fn list_backup_snapshots(
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: String,
) -> Result<Vec<engine::SnapshotRef>, CommandError> {
    let config = {
        let b = Arc::clone(&backups);
        crate::cloud::dashboard::blocking_pub(move || b.job(&job)).await?
    };
    let stores = prepare(&accounts, &config).await?;
    let cancel = Cancel::default();
    let mut nothing = |_: Progress| {};
    let ctx = Ctx {
        dest: &stores.dest,
        dest_bucket: &config.destination.bucket,
        dest_prefix: &config.destination.prefix,
        spool: Path::new("."),
        cancel: &cancel,
        journal: None,
        progress: &mut nothing,
    };
    engine::snapshots(&ctx, &config.id)
        .await
        .map_err(|e| backup_error(&e))
}

/// The restore target as the page names it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RestoreTarget {
    /// A local folder chosen in a dialog Rust opens.
    Local,
    /// A folder of an account's bucket (never overwriting).
    Bucket { account: String, prefix: String },
}

/// Restores a snapshot (or a folder of it). For a local target Rust opens a
/// folder dialog; `None` when the user cancelled it. Allowed by
/// `allow-restore-backup`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn restore_backup(
    window: tauri::WebviewWindow,
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: String,
    snapshot: String,
    filter: String,
    target: RestoreTarget,
    on_event: Channel<BackupEvent>,
) -> Result<Option<String>, CommandError> {
    let to = match target {
        RestoreTarget::Local => match choose_folder(&window).await {
            Some(dir) => RestoreTo::Local(dir),
            None => return Ok(None),
        },
        RestoreTarget::Bucket { account, prefix } => {
            if !(prefix.is_empty() || prefix.ends_with('/')) {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "the target folder must be empty or end in '/'",
                ));
            }
            RestoreTo::Bucket { account, prefix }
        }
    };
    Arc::clone(&backups)
        .start_restore(
            Arc::clone(&accounts),
            &job,
            &snapshot,
            filter,
            to,
            Arc::new(on_event),
        )
        .await
        .map(Some)
}

/// The recovery export: a snapshot's manifest with the backup's location and
/// the steps to restore it with standard S3 tools. Never a credential.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryExport {
    pub format: &'static str,
    pub version: u32,
    pub exported: String,
    pub destination: ExportLocation,
    pub snapshot_key: String,
    pub instructions: Vec<&'static str>,
    pub manifest: model::Manifest,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportLocation {
    pub provider: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
}

pub const EXPORT_INSTRUCTIONS: [&str; 5] = [
    "Each entry's content is the object <prefix><blob> in the destination bucket; its SHA-256 is the last part of that key and is also stored as x-amz-meta-sotyware-sha256.",
    "With the AWS CLI: aws s3 cp s3://<bucket>/<prefix><blob> <local path> --endpoint-url <endpoint>, then compare sha256sum with the entry's sha256.",
    "With rclone, configure an S3 remote for the provider and copy the same keys.",
    "Without any tool: scripts/recovery/sotyware-restore.py in the sotyware source restores a whole snapshot with Python 3 only.",
    "You need an access key for the destination bucket; it is not in this file.",
];

/// Saves a snapshot's recovery export to a file the user picks in a save
/// dialog Rust opens. Returns the file's name, or `None` if cancelled.
/// Allowed by `allow-export-backup-manifest`.
#[tauri::command]
pub async fn export_backup_manifest(
    window: tauri::WebviewWindow,
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: String,
    snapshot: String,
) -> Result<Option<String>, CommandError> {
    let config = {
        let b = Arc::clone(&backups);
        crate::cloud::dashboard::blocking_pub(move || b.job(&job)).await?
    };
    let stores = prepare(&accounts, &config).await?;
    let (provider, endpoint, region) = {
        let a = Arc::clone(&accounts);
        let id = config.destination.account.clone();
        crate::cloud::dashboard::blocking_pub(move || a.location_of(&id)).await?
    };
    let spool = backups.task_spool(&format!("export-{}", engine::new_id()));
    let cancel = Cancel::default();
    let mut nothing = |_: Progress| {};
    let ctx = Ctx {
        dest: &stores.dest,
        dest_bucket: &config.destination.bucket,
        dest_prefix: &config.destination.prefix,
        spool: &spool,
        cancel: &cancel,
        journal: None,
        progress: &mut nothing,
    };
    let manifest = engine::read_manifest(&ctx, &snapshot).await;
    clear_task_spool(&spool);
    let manifest = manifest.map_err(|e| backup_error(&e))?;
    let export = RecoveryExport {
        format: "sotyware-recovery-export",
        version: 1,
        exported: engine::now_rfc3339(),
        destination: ExportLocation {
            provider,
            endpoint,
            region,
            bucket: config.destination.bucket.clone(),
            prefix: config.destination.prefix.clone(),
        },
        snapshot_key: snapshot,
        instructions: EXPORT_INSTRUCTIONS.to_vec(),
        manifest,
    };
    let name = format!(
        "sotyware-recovery-{}-{}.json",
        config.id,
        model::compact_time(&export.manifest.created)
    );
    let Some(path) = choose_save(&window, name).await else {
        return Ok(None);
    };
    let json = serde_json::to_vec_pretty(&export)
        .map_err(|e| CommandError::new(ErrorKind::Other, e.to_string()))?;
    tokio::fs::write(&path, json).await.map_err(|e| {
        CommandError::new(ErrorKind::LocalFile, format!("cannot write the file: {e}"))
    })?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// What the destination guarantees against deletion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Protection {
    pub same_account: bool,
    /// `enabled`, `suspended`, `disabled`, or `unknown` (could not be read).
    pub versioning: String,
    pub object_lock: bool,
    pub default_mode: Option<String>,
    pub default_days: Option<u32>,
    /// Only compliance-mode default retention makes the backup immutable.
    pub immutable: bool,
    pub summary: String,
}

/// Describes a protection result in sotyware's words.
pub fn describe_protection(p: &Protection) -> String {
    let mut parts = Vec::new();
    if p.same_account {
        parts.push("The destination is in the same account as the source: a compromised key or a closed account affects both.".to_owned());
    }
    match (p.object_lock, p.default_mode.as_deref(), p.default_days) {
        (true, Some("COMPLIANCE"), Some(d)) => parts.push(format!(
            "Object Lock in compliance mode keeps every new backup object for {d} days: nobody can delete or shorten it, not even you."
        )),
        (true, Some("GOVERNANCE"), Some(d)) => parts.push(format!(
            "Object Lock in governance mode keeps new backup objects for {d} days, but a key with the bypass permission can still delete them."
        )),
        (true, _, _) => parts.push("Object Lock is on, but without a default retention: new backup objects are not locked.".to_owned()),
        (false, _, _) => parts.push("Object Lock is off: a key that may delete in the destination bucket can delete the backup.".to_owned()),
    }
    parts.push(match p.versioning.as_str() {
        "enabled" => "Versioning is on: a plain delete leaves the data recoverable until a lifecycle rule removes it.".to_owned(),
        "suspended" => "Versioning is suspended: new deletes remove data for good.".to_owned(),
        "disabled" => "Versioning is off: deletes remove data for good.".to_owned(),
        _ => "Versioning could not be read with this key.".to_owned(),
    });
    parts.join(" ")
}

/// Reads the destination's versioning and Object Lock configuration (read
/// only). Allowed by `allow-check-backup-protection`.
#[tauri::command]
pub async fn check_backup_protection(
    backups: State<'_, Arc<Backups>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    job: String,
) -> Result<Protection, CommandError> {
    let config = {
        let b = Arc::clone(&backups);
        crate::cloud::dashboard::blocking_pub(move || b.job(&job)).await?
    };
    let stores = prepare(&accounts, &config).await?;
    protection(&stores.dest, &config).await
}

/// The destination's protection characteristics, as read from the bucket.
pub async fn protection(dest: &S3Store, job: &JobConfig) -> Result<Protection, CommandError> {
    let versioning = match dest.bucket_versioning(&job.destination.bucket).await {
        Ok(Versioning::Enabled) => "enabled",
        Ok(Versioning::Suspended) => "suspended",
        Ok(Versioning::Disabled) => "disabled",
        Err(_) => "unknown",
    };
    let lock =
        dest.object_lock(&job.destination.bucket)
            .await
            .unwrap_or(crate::cloud::s3::ObjectLock {
                enabled: false,
                default_mode: None,
                default_days: None,
            });
    let mut p = Protection {
        same_account: model::same_account(job),
        versioning: versioning.into(),
        immutable: lock.enabled
            && lock.default_mode.as_deref() == Some("COMPLIANCE")
            && lock.default_days.is_some_and(|d| d > 0),
        object_lock: lock.enabled,
        default_mode: lock.default_mode,
        default_days: lock.default_days,
        summary: String::new(),
    };
    p.summary = describe_protection(&p);
    Ok(p)
}

#[cfg(desktop)]
async fn choose_folder(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Choose a folder to restore into")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    rx.await.ok().flatten()?.into_path().ok()
}

#[cfg(not(desktop))]
async fn choose_folder(_window: &tauri::WebviewWindow) -> Option<PathBuf> {
    None
}

#[cfg(desktop)]
async fn choose_save(window: &tauri::WebviewWindow, name: String) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Save the recovery file")
        .set_file_name(name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.ok().flatten()?.into_path().ok()
}

#[cfg(not(desktop))]
async fn choose_save(_window: &tauri::WebviewWindow, _name: String) -> Option<PathBuf> {
    None
}

/// Whether `name` is one of the spool's own names: a task folder (a task id
/// or `export-<id>`) or a spool file (`object-<id>`, `engine::spool_file`).
fn spool_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|id| {
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// Removes one task's spool folder, never recursively: only regular files
/// named like spool files, then the folder itself if that left it empty.
/// Anything else (a link, a file someone else put there) stays, and so
/// does the folder.
fn clear_task_spool(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if is_file && name.to_str().is_some_and(|n| spool_name(n, "object-")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    if let Err(e) = std::fs::remove_dir(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("sotyware: left the backup spool folder in place, it holds something else: {e}");
    }
}

/// Empties the spool folder left by an earlier session: each task folder
/// as [`clear_task_spool`] does. Links and unknown names are left alone.
fn clear_spool(spool: &Path) {
    let Ok(entries) = std::fs::read_dir(spool) else {
        return;
    };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir
            && entry
                .file_name()
                .to_str()
                .is_some_and(|n| spool_name(n, ""))
        {
            clear_task_spool(&entry.path());
        }
    }
}

#[cfg(test)]
mod tests;

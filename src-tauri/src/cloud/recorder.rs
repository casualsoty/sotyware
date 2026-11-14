//! The opt-in request recorder (ADR-024): records the S3 requests the app
//! itself sends, so a controlled integration run at a real provider can
//! count them. Compiled only with the Cargo feature `request-recorder`
//! (never in a normal build) and started only when all three
//! `SOTYWARE_REQUEST_RECORD_*` variables are set and valid.
//!
//! One read-only SDK interceptor per [`S3Store`](super::s3::S3Store),
//! added in `S3Store::new`. It never returns an error, never reads a
//! credential, a header other than `content-length` and
//! `x-amz-version-id`, a body or an error message, and never blocks: each
//! event is handed to one writer thread through a bounded queue with
//! `try_send`; a full queue drops the event and leaves a gap in the
//! sequence numbers, which the writer reports. Format and redaction:
//! docs/architecture/cloud-storage.md#request-recorder.
//!
//! The release profile aborts on panic, so nothing here may panic: no
//! `unwrap`, `expect`, indexing or unchecked arithmetic.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::ffi::OsString;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_s3::config::interceptors::{
    BeforeSerializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
    FinalizerInterceptorContextRef,
};
use aws_sdk_s3::config::{ConfigBag, Intercept, RuntimeComponents};
use aws_sdk_s3::error::{BoxError, ProvideErrorMetadata};
use aws_sdk_s3::operation::abort_multipart_upload::{
    AbortMultipartUploadError, AbortMultipartUploadInput,
};
use aws_sdk_s3::operation::complete_multipart_upload::{
    CompleteMultipartUploadError, CompleteMultipartUploadInput,
};
use aws_sdk_s3::operation::copy_object::{CopyObjectError, CopyObjectInput};
use aws_sdk_s3::operation::create_multipart_upload::{
    CreateMultipartUploadError, CreateMultipartUploadInput,
};
use aws_sdk_s3::operation::delete_object::{DeleteObjectError, DeleteObjectInput};
use aws_sdk_s3::operation::get_bucket_versioning::{
    GetBucketVersioningError, GetBucketVersioningInput,
};
use aws_sdk_s3::operation::get_object::{GetObjectError, GetObjectInput};
use aws_sdk_s3::operation::head_bucket::{HeadBucketError, HeadBucketInput};
use aws_sdk_s3::operation::head_object::{HeadObjectError, HeadObjectInput};
use aws_sdk_s3::operation::list_buckets::{ListBucketsError, ListBucketsInput};
use aws_sdk_s3::operation::list_object_versions::{
    ListObjectVersionsError, ListObjectVersionsInput,
};
use aws_sdk_s3::operation::list_objects_v2::{ListObjectsV2Error, ListObjectsV2Input};
use aws_sdk_s3::operation::put_object::{PutObjectError, PutObjectInput};
use aws_sdk_s3::operation::upload_part::{UploadPartError, UploadPartInput};
use aws_smithy_runtime_api::client::interceptors::context::{Error as OpError, Input};
use aws_smithy_runtime_api::client::orchestrator::{Metadata, OrchestratorError};
use aws_smithy_runtime_api::client::retries::RequestAttempts;
use aws_smithy_types::config_bag::{Storable, StoreReplace};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::account::{ResolvedAccount, is_kebab_case};

/// The directory the recording is written to (absolute, existing, not a
/// symbolic link).
pub const ENV_DIR: &str = "SOTYWARE_REQUEST_RECORD_DIR";
/// The run id (`[a-z0-9-]`, 8 to 64 characters): the file name and the
/// salt of every hash.
pub const ENV_RUN: &str = "SOTYWARE_REQUEST_RECORD_RUN";
/// The run's key prefix: keys inside it are recorded as hashes of the part
/// after it, every other key as `outside-scope`.
pub const ENV_SCOPE: &str = "SOTYWARE_REQUEST_RECORD_SCOPE";
/// Optional: the account id recorded as itself; every other account is
/// `other-<n>`.
pub const ENV_ACCOUNT: &str = "SOTYWARE_REQUEST_RECORD_ACCOUNT";

/// The interceptor's name. Also the marker `scripts/check-release-binary.mjs`
/// looks for: a binary that contains it is a recorder build and must never
/// be packaged.
pub const MARKER: &str = "sotyware-request-recorder";

/// Version of the record format (`start` line).
pub const FORMAT: u32 = 1;

/// Events the queue holds before new ones are dropped (each a few hundred
/// bytes, so at most about a megabyte).
pub const QUEUE_CAPACITY: usize = 4096;

/// How long quitting waits for the writer to drain the queue and write the
/// `end` line.
pub const FINISH_WAIT: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

// ------------------------------------------------------------ settings

/// Validated recorder settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub dir: PathBuf,
    pub run: String,
    pub scope: String,
    pub account: Option<String>,
}

fn valid_run(run: &str) -> bool {
    (8..=64).contains(&run.len())
        && !run.starts_with('-')
        && run
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope.len() <= 512
        && scope.ends_with('/')
        && !scope.starts_with('/')
        && !scope.chars().any(char::is_control)
}

impl Settings {
    /// Reads the variables through `lookup`. `Ok(None)` when none of them
    /// is set (recording off); an error when only some are set or any is
    /// invalid (recording off too: fail closed, never half configured).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Option<Self>, String> {
        let dir = lookup(ENV_DIR);
        let run = lookup(ENV_RUN);
        let scope = lookup(ENV_SCOPE);
        let account = lookup(ENV_ACCOUNT);
        if dir.is_none() && run.is_none() && scope.is_none() && account.is_none() {
            return Ok(None);
        }
        let (Some(dir), Some(run), Some(scope)) = (dir, run, scope) else {
            return Err(format!(
                "{ENV_DIR}, {ENV_RUN} and {ENV_SCOPE} must all be set"
            ));
        };
        let dir = PathBuf::from(dir);
        if !dir.is_absolute() {
            return Err(format!("{ENV_DIR} must be an absolute path"));
        }
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "{ENV_DIR} must be a directory, not a file or symbolic link"
                ));
            }
            Err(e) => return Err(format!("{ENV_DIR} cannot be read: {}", e.kind())),
        }
        let run = run
            .into_string()
            .ok()
            .filter(|r| valid_run(r))
            .ok_or_else(|| format!("{ENV_RUN} must be 8 to 64 of a-z, 0-9 and -"))?;
        let scope = scope
            .into_string()
            .ok()
            .filter(|s| valid_scope(s))
            .ok_or_else(|| {
                format!("{ENV_SCOPE} must be a key prefix ending in / (no control characters)")
            })?;
        let account = match account {
            None => None,
            Some(a) => Some(
                a.into_string()
                    .ok()
                    .filter(|a| is_kebab_case(a))
                    .ok_or_else(|| format!("{ENV_ACCOUNT} must be an account id"))?,
            ),
        };
        Ok(Some(Self {
            dir,
            run,
            scope,
            account,
        }))
    }

    /// The recording file of process `pid`: `<dir>/<run>.<pid>.requests.jsonl`.
    /// One per process, so a run that starts the app more than once (a
    /// relaunch after quitting) gets one complete file per process.
    pub fn file(&self, pid: u32) -> PathBuf {
        self.dir.join(format!("{}.{pid}.requests.jsonl", self.run))
    }
}

// ------------------------------------------------------------- hashing

/// The first 12 bytes (hex) of SHA-256(`run` 0 `domain` 0 `value`). The
/// harness recomputes these for the keys and ids it knows; domains are
/// `key` (the part of a key after the scope), `upload-id`, `version-id`.
pub fn scoped_hash(run: &str, domain: &str, value: &str) -> String {
    let mut h = Sha256::new();
    h.update(run.as_bytes());
    h.update([0]);
    h.update(domain.as_bytes());
    h.update([0]);
    h.update(value.as_bytes());
    h.finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

// --------------------------------------------------------------- health

#[derive(Default)]
struct Stats {
    queue_full: AtomicU64,
    writer_gone: AtomicU64,
    after_finish: AtomicU64,
    written: AtomicU64,
    write_errors: AtomicU64,
    ops_started: AtomicU64,
    ops_ended: AtomicU64,
    ops_dropped: AtomicU64,
    /// Sequence numbers handed out (events submitted, dropped or not).
    next_seq: AtomicU64,
}

/// How complete a recording is. Only [`Health::complete`] means every
/// request the app's stores started was recorded from start to end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub events: u64,
    pub written: u64,
    pub queue_full: u64,
    pub writer_gone: u64,
    pub after_finish: u64,
    pub write_errors: u64,
    pub ops_started: u64,
    pub ops_ended: u64,
    pub ops_dropped: u64,
    /// The writer confirmed the `end` line within the finish wait.
    pub end_written: bool,
}

impl Health {
    pub fn open_ops(&self) -> u64 {
        self.ops_started
            .saturating_sub(self.ops_ended)
            .saturating_sub(self.ops_dropped)
    }

    /// Why the recording is incomplete; empty when it is complete.
    pub fn reasons(&self) -> Vec<&'static str> {
        let mut r = Vec::new();
        if self.queue_full > 0 {
            r.push("queue-full");
        }
        if self.writer_gone > 0 {
            r.push("writer-gone");
        }
        if self.write_errors > 0 {
            r.push("write-error");
        }
        if self.written < self.events {
            r.push("unwritten-events");
        }
        if self.open_ops() > 0 {
            r.push("open-operations");
        }
        if self.after_finish > 0 {
            r.push("events-after-end");
        }
        r
    }

    pub fn complete(&self) -> bool {
        self.end_written && self.reasons().is_empty()
    }
}

impl Stats {
    fn health(&self) -> Health {
        let get = |a: &AtomicU64| a.load(Ordering::SeqCst);
        Health {
            events: get(&self.next_seq),
            written: get(&self.written),
            queue_full: get(&self.queue_full),
            writer_gone: get(&self.writer_gone),
            after_finish: get(&self.after_finish),
            write_errors: get(&self.write_errors),
            ops_started: get(&self.ops_started),
            ops_ended: get(&self.ops_ended),
            ops_dropped: get(&self.ops_dropped),
            end_written: false,
        }
    }
}

// --------------------------------------------------------------- writer

/// Where the writer thread puts the lines: the recording file, or a test
/// double.
pub trait Sink: Write + Send + 'static {
    /// Makes written lines durable (the file: `sync_all`).
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink for std::fs::File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

enum Msg {
    Line {
        seq: u64,
        text: String,
    },
    /// Test only: the writer thread ends at once, as if it had died.
    #[cfg(test)]
    Die,
}

fn writer(rx: Receiver<Msg>, mut sink: Box<dyn Sink>, stats: Arc<Stats>, done: SyncSender<()>) {
    let mut expected = 0u64;
    let mut failed = false;
    let put = |sink: &mut Box<dyn Sink>, buf: &[u8], failed: &mut bool| -> bool {
        if *failed {
            return false;
        }
        match sink.write_all(buf).and_then(|()| sink.flush()) {
            Ok(()) => true,
            Err(e) => {
                *failed = true;
                stats.write_errors.fetch_add(1, Ordering::SeqCst);
                eprintln!(
                    "sotyware: request recorder cannot write, recording is incomplete: {}",
                    e.kind()
                );
                false
            }
        }
    };
    let gap = |from: u64, to: u64| -> String {
        format!(
            "{}\n",
            json!({"kind": "gap", "missing_from": from, "missing_to": to,
                   "count": to.saturating_sub(from).saturating_add(1)})
        )
    };
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Line { seq, text } => {
                let mut buf = String::new();
                if seq > expected {
                    buf.push_str(&gap(expected, seq.saturating_sub(1)));
                }
                buf.push_str(&text);
                buf.push('\n');
                if put(&mut sink, buf.as_bytes(), &mut failed) {
                    stats.written.fetch_add(1, Ordering::SeqCst);
                }
                expected = seq.saturating_add(1);
            }
            #[cfg(test)]
            Msg::Die => return,
        }
    }
    // The queue is closed: everything submitted before is written.
    let mut health = stats.health();
    let mut buf = String::new();
    if health.events > expected {
        buf.push_str(&gap(expected, health.events.saturating_sub(1)));
    }
    health.end_written = true;
    let reasons = health.reasons();
    buf.push_str(&format!(
        "{}\n",
        json!({
            "kind": "end",
            "events": health.events,
            "written": health.written,
            "queue_full": health.queue_full,
            "writer_gone": health.writer_gone,
            "write_errors": health.write_errors,
            "ops_started": health.ops_started,
            "ops_ended": health.ops_ended,
            "ops_dropped": health.ops_dropped,
            "ops_open": health.open_ops(),
            "accounting": if reasons.is_empty() { "complete" } else { "incomplete" },
            "reasons": reasons,
        })
    ));
    if put(&mut sink, buf.as_bytes(), &mut failed) && sink.sync().is_ok() {
        let _ = done.try_send(());
    }
}

// ------------------------------------------------------------- recorder

/// One recording: the queue to the writer thread and the counters.
pub struct Recorder {
    run: String,
    scope: String,
    account: Option<String>,
    start: Instant,
    queue: Mutex<Option<SyncSender<Msg>>>,
    done: Mutex<Option<Receiver<()>>>,
    next_op: AtomicU64,
    stats: Arc<Stats>,
    accounts: Mutex<Vec<String>>,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Recorder")
    }
}

impl Recorder {
    /// Creates `<dir>/<run>.<pid>.requests.jsonl` (never an existing file or
    /// link: exclusive creation), writes the `start` line and starts the
    /// writer thread. Any failure: no recorder, and the file is removed if
    /// it was created.
    pub fn start(settings: &Settings) -> Result<Arc<Self>, String> {
        let path = settings.file(std::process::id());
        let mut options = OpenOptions::new();
        // `create_new` is O_CREAT|O_EXCL (Windows: CREATE_NEW): it fails if
        // anything, a symbolic link included, is at the path, and never
        // follows one.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("cannot create the recording file: {}", e.kind()))?;
        Self::with_sink(settings, Box::new(file), QUEUE_CAPACITY).inspect_err(|_| {
            let _ = std::fs::remove_file(&path);
        })
    }

    /// Like [`Self::start`], writing to `sink` with a queue of `capacity`.
    pub fn with_sink(
        settings: &Settings,
        mut sink: Box<dyn Sink>,
        capacity: usize,
    ) -> Result<Arc<Self>, String> {
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let start = json!({
            "kind": "start",
            "format": FORMAT,
            "recorder": MARKER,
            "run": settings.run,
            "scope": scoped_hash(&settings.run, "scope", &settings.scope),
            "account": settings.account,
            "app_version": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "started_unix_ms": started_unix_ms,
            "queue_capacity": capacity,
        });
        sink.write_all(format!("{start}\n").as_bytes())
            .and_then(|()| sink.flush())
            .map_err(|e| format!("cannot write the recording file: {}", e.kind()))?;
        let (tx, rx) = mpsc::sync_channel(capacity);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let stats = Arc::new(Stats::default());
        let thread_stats = Arc::clone(&stats);
        std::thread::Builder::new()
            .name("sotyware-request-recorder".into())
            .spawn(move || writer(rx, sink, thread_stats, done_tx))
            .map_err(|e| format!("cannot start the recorder thread: {}", e.kind()))?;
        Ok(Arc::new(Self {
            run: settings.run.clone(),
            scope: settings.scope.clone(),
            account: settings.account.clone(),
            start: Instant::now(),
            queue: Mutex::new(Some(tx)),
            done: Mutex::new(Some(done_rx)),
            next_op: AtomicU64::new(0),
            stats,
            accounts: Mutex::new(Vec::new()),
        }))
    }

    /// Hands one event to the writer, never waiting: a full queue or a
    /// writer that is gone only counts it (its sequence number becomes a
    /// gap).
    fn submit(&self, fields: Map<String, Value>) {
        let t_us = micros(self.start.elapsed());
        let mut fields = fields;
        fields.insert("t_us".into(), t_us.into());
        let body = Value::Object(fields).to_string();
        let rest = body.strip_prefix('{').unwrap_or(&body);
        // Numbered under the lock, so the queue (and the file) is in
        // sequence order.
        let queue = lock(&self.queue);
        let Some(tx) = queue.as_ref() else {
            self.stats.after_finish.fetch_add(1, Ordering::SeqCst);
            return;
        };
        let seq = self.stats.next_seq.fetch_add(1, Ordering::SeqCst);
        let text = format!("{{\"seq\":{seq},{rest}");
        match tx.try_send(Msg::Line { seq, text }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.stats.queue_full.fetch_add(1, Ordering::SeqCst);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats.writer_gone.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// The counters so far.
    pub fn health(&self) -> Health {
        self.stats.health()
    }

    /// Closes the queue, waits at most `wait` for the writer to write
    /// everything and the `end` line, and reports how complete the
    /// recording is. Later events are counted as `after_finish`.
    pub fn finish(&self, wait: Duration) -> Health {
        drop(lock(&self.queue).take());
        let done = lock(&self.done).take();
        let end_written = done.is_some_and(|rx| rx.recv_timeout(wait).is_ok());
        let mut health = self.stats.health();
        health.end_written = end_written;
        if health.complete() {
            eprintln!(
                "sotyware: request recorder: complete, {} events",
                health.events
            );
        } else {
            eprintln!(
                "sotyware: request recorder: INCOMPLETE ({}{})",
                health.reasons().join(", "),
                if end_written { "" } else { "; no end line" }
            );
        }
        health
    }

    /// Test only: makes the writer thread end as if it had died.
    #[cfg(test)]
    pub(crate) fn kill_writer(&self) {
        if let Some(tx) = lock(&self.queue).as_ref() {
            let _ = tx.try_send(Msg::Die);
        }
    }

    fn account_label(&self, id: &str) -> String {
        if self.account.as_deref() == Some(id) {
            return id.to_owned();
        }
        let mut seen = lock(&self.accounts);
        let n = match seen.iter().position(|a| a == id) {
            Some(i) => i.saturating_add(1),
            None => {
                seen.push(id.to_owned());
                seen.len()
            }
        };
        format!("other-{n}")
    }

    /// The interceptor for one store.
    pub fn interceptor_for(self: &Arc<Self>, account: &ResolvedAccount) -> RequestRecorder {
        RequestRecorder {
            rec: Arc::clone(self),
            account: self.account_label(&account.id),
            buckets: account.buckets.clone().into(),
        }
    }

    fn key(&self, key: Option<&str>) -> Option<Value> {
        let key = key?;
        Some(match key.strip_prefix(self.scope.as_str()) {
            Some(rel) => format!("scope:{}", scoped_hash(&self.run, "key", rel)).into(),
            None => "outside-scope".into(),
        })
    }

    fn prefix(&self, prefix: Option<&str>) -> Value {
        match prefix {
            None | Some("") => "root".into(),
            Some(p) => match p.strip_prefix(self.scope.as_str()) {
                Some(rel) => format!("scope:{}", scoped_hash(&self.run, "key", rel)).into(),
                None if self.scope.starts_with(p) => "scope-ancestor".into(),
                None => "outside-scope".into(),
            },
        }
    }

    fn id(&self, domain: &str, id: Option<&str>) -> Option<Value> {
        id.map(|id| scoped_hash(&self.run, domain, id).into())
    }
}

// ---------------------------------------------------------- the process

static INSTALLED: OnceLock<Arc<Recorder>> = OnceLock::new();

/// Starts the process's recorder if the variables ask for one; called once
/// in `run()` before any store exists. Invalid or partial settings, or a
/// file that cannot be created: no recorder (and one line on stderr).
pub fn start_from_env() {
    match Settings::from_lookup(|name| std::env::var_os(name)) {
        Ok(None) => {}
        // A bundler patched this binary (deb, rpm, AppImage, MSI, NSIS):
        // a packaged recorder build never records, whatever the variables
        // say (scripts/check-release-binary.mjs refuses to package one).
        Ok(Some(_)) if tauri::utils::platform::bundle_type().is_some() => {
            eprintln!("sotyware: {MARKER} not started: this binary was packaged");
        }
        Ok(Some(settings)) => match Recorder::start(&settings) {
            Ok(rec) => {
                if INSTALLED.set(rec).is_ok() {
                    eprintln!("sotyware: {MARKER} on, run {}", settings.run);
                }
            }
            Err(e) => eprintln!("sotyware: {MARKER} not started: {e}"),
        },
        Err(e) => eprintln!("sotyware: {MARKER} not started: {e}"),
    }
}

/// Test builds only: with this variable set, every store of the test
/// process is recorded (into a sink that discards the lines), so the
/// existing suites can be run with recording on and must pass unchanged.
#[cfg(test)]
pub(crate) const TEST_ALL: &str = "SOTYWARE_RECORDER_TEST_ALL";

/// The process's recorder, if one was started.
pub fn installed() -> Option<&'static Arc<Recorder>> {
    #[cfg(test)]
    if std::env::var_os(TEST_ALL).is_some() && INSTALLED.get().is_none() {
        let settings = Settings {
            dir: PathBuf::new(),
            run: "test-all-stores".into(),
            scope: "test-all/".into(),
            account: None,
        };
        struct Discard;
        impl Write for Discard {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Sink for Discard {}
        if let Ok(rec) = Recorder::with_sink(&settings, Box::new(Discard), QUEUE_CAPACITY) {
            let _ = INSTALLED.set(rec);
        }
    }
    INSTALLED.get()
}

/// Finishes the process's recorder (in `RunEvent::Exit`, after the
/// transfers' shutdown, so their aborts are recorded).
pub fn finish_installed() {
    if let Some(rec) = installed() {
        rec.finish(FINISH_WAIT);
    }
}

// ---------------------------------------------------------- interceptor

/// The read-only interceptor of one store.
#[derive(Clone)]
pub struct RequestRecorder {
    rec: Arc<Recorder>,
    account: String,
    buckets: Arc<[String]>,
}

impl fmt::Debug for RequestRecorder {
    // Never the bucket names.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestRecorder")
    }
}

/// One operation, kept in the `ConfigBag` (`interceptor_state`). When the
/// operation's future is dropped before `read_after_execution`, the last
/// clone goes with it and `Drop` records `op-dropped`.
#[derive(Clone)]
struct OpState(Arc<OpInner>);

impl fmt::Debug for OpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpState")
    }
}

impl Storable for OpState {
    type Storer = StoreReplace<Self>;
}

struct OpInner {
    rec: Arc<Recorder>,
    op: u64,
    name: &'static str,
    account: String,
    started: Instant,
    ended: AtomicBool,
    attempts: AtomicU32,
    /// Microseconds since `started` when the current attempt was sent, plus
    /// one (0: none in flight).
    attempt_sent: AtomicU64,
    last_status: AtomicU16,
}

impl Drop for OpInner {
    fn drop(&mut self) {
        if self.ended.load(Ordering::SeqCst) {
            return;
        }
        self.rec.stats.ops_dropped.fetch_add(1, Ordering::SeqCst);
        let mut f = Map::new();
        f.insert("kind".into(), "op-dropped".into());
        f.insert("op".into(), self.op.into());
        f.insert("name".into(), self.name.into());
        f.insert("account".into(), self.account.clone().into());
        f.insert(
            "attempts".into(),
            self.attempts.load(Ordering::SeqCst).into(),
        );
        f.insert(
            "in_flight".into(),
            (self.attempt_sent.load(Ordering::SeqCst) > 0).into(),
        );
        let status = self.last_status.load(Ordering::SeqCst);
        if status > 0 {
            f.insert("last_status".into(), status.into());
        }
        f.insert("dur_us".into(), micros(self.started.elapsed()).into());
        self.rec.submit(f);
    }
}

fn kind_of<E>(e: &OrchestratorError<E>) -> &'static str {
    if e.is_operation_error() {
        "service-error"
    } else if e.is_timeout_error() {
        "timeout"
    } else if e.is_connector_error() {
        "connector"
    } else if e.is_response_error() {
        "response"
    } else if e.is_interceptor_error() {
        "interceptor"
    } else {
        "other"
    }
}

/// The provider's error code (`NoSuchKey`, `AccessDenied`, …) when the
/// typed error has one and it looks like a code; never the message.
fn error_code(e: &OpError) -> Option<String> {
    macro_rules! code {
        ($($t:ty),*) => {
            $(if let Some(err) = e.downcast_ref::<$t>() { return err.code().map(sanitize_code); })*
        };
    }
    code!(
        HeadBucketError,
        ListBucketsError,
        ListObjectsV2Error,
        HeadObjectError,
        GetObjectError,
        PutObjectError,
        CreateMultipartUploadError,
        UploadPartError,
        CompleteMultipartUploadError,
        AbortMultipartUploadError,
        DeleteObjectError,
        CopyObjectError,
        ListObjectVersionsError,
        GetBucketVersioningError
    );
    None
}

/// `bucket/key[?versionId=v]` of an `x-amz-copy-source` header, percent
/// decoded (what `s3::copy_source` encodes); `None`s when it does not have
/// that form. Only used to hash or classify, never recorded as is.
fn parse_copy_source(source: Option<&str>) -> (Option<String>, Option<String>, bool) {
    fn decode(s: &str) -> Option<String> {
        let mut out = Vec::with_capacity(s.len());
        let mut bytes = s.bytes();
        while let Some(b) = bytes.next() {
            if b == b'%' {
                let hi = char::from(bytes.next()?).to_digit(16)?;
                let lo = char::from(bytes.next()?).to_digit(16)?;
                out.push(u8::try_from(hi * 16 + lo).ok()?);
            } else {
                out.push(b);
            }
        }
        String::from_utf8(out).ok()
    }
    let Some(source) = source else {
        return (None, None, false);
    };
    let source = source.strip_prefix('/').unwrap_or(source);
    let (path, version) = match source.split_once("?versionId=") {
        Some((p, _)) => (p, true),
        None => (source, false),
    };
    match path.split_once('/') {
        Some((bucket, key)) => (decode(bucket), decode(key), version),
        None => (decode(path), None, version),
    }
}

fn sanitize_code(code: &str) -> String {
    if !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
    {
        code.to_owned()
    } else {
        "unrecognised".to_owned()
    }
}

impl RequestRecorder {
    fn bucket(&self, bucket: Option<&str>) -> Option<Value> {
        bucket.map(|b| {
            if self.buckets.iter().any(|c| c == b) {
                "configured".into()
            } else {
                "other".into()
            }
        })
    }

    /// The operation's name and its recorded fields, from the typed input
    /// only (the operation `Metadata` is not in the `ConfigBag` yet at
    /// `read_before_execution`). Unknown inputs: name `unknown`, nothing
    /// else.
    fn describe(&self, input: &Input, f: &mut Map<String, Value>) -> &'static str {
        let rec = &self.rec;
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                f.insert(k.into(), v);
            }
        };
        if let Some(i) = input.downcast_ref::<HeadBucketInput>() {
            put("bucket", self.bucket(i.bucket()));
            "HeadBucket"
        } else if input.downcast_ref::<ListBucketsInput>().is_some() {
            "ListBuckets"
        } else if let Some(i) = input.downcast_ref::<ListObjectsV2Input>() {
            put("bucket", self.bucket(i.bucket()));
            put("prefix", Some(rec.prefix(i.prefix())));
            put("delimiter", Some(i.delimiter().is_some().into()));
            put(
                "continuation",
                Some(i.continuation_token().is_some().into()),
            );
            put("max_keys", i.max_keys().map(Value::from));
            "ListObjectsV2"
        } else if let Some(i) = input.downcast_ref::<HeadObjectInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("version", Some(i.version_id().is_some().into()));
            "HeadObject"
        } else if let Some(i) = input.downcast_ref::<GetObjectInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("version", Some(i.version_id().is_some().into()));
            put("range", Some(i.range().is_some().into()));
            "GetObject"
        } else if let Some(i) = input.downcast_ref::<PutObjectInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            "PutObject"
        } else if let Some(i) = input.downcast_ref::<CreateMultipartUploadInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            "CreateMultipartUpload"
        } else if let Some(i) = input.downcast_ref::<UploadPartInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("upload", rec.id("upload-id", i.upload_id()));
            put("part", i.part_number().map(Value::from));
            "UploadPart"
        } else if let Some(i) = input.downcast_ref::<CompleteMultipartUploadInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("upload", rec.id("upload-id", i.upload_id()));
            put(
                "parts",
                i.multipart_upload().map(|m| Value::from(m.parts().len())),
            );
            "CompleteMultipartUpload"
        } else if let Some(i) = input.downcast_ref::<AbortMultipartUploadInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("upload", rec.id("upload-id", i.upload_id()));
            "AbortMultipartUpload"
        } else if let Some(i) = input.downcast_ref::<DeleteObjectInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            put("version", Some(i.version_id().is_some().into()));
            "DeleteObject"
        } else if let Some(i) = input.downcast_ref::<CopyObjectInput>() {
            // ADR-026: the destination like any key; the source from the
            // copy-source header, decoded, also hashed or "outside-scope".
            put("bucket", self.bucket(i.bucket()));
            put("key", rec.key(i.key()));
            let (src_bucket, src_key, src_version) = parse_copy_source(i.copy_source());
            put("source_bucket", self.bucket(src_bucket.as_deref()));
            put("source_key", rec.key(src_key.as_deref()));
            put("source_version", Some(src_version.into()));
            put("if_match", Some(i.copy_source_if_match().is_some().into()));
            put(
                "metadata_copied",
                Some(
                    (i.metadata_directive().map(|d| d.as_str()) == Some("COPY")
                        || i.metadata_directive().is_none())
                    .into(),
                ),
            );
            "CopyObject"
        } else if let Some(i) = input.downcast_ref::<ListObjectVersionsInput>() {
            put("bucket", self.bucket(i.bucket()));
            put("prefix", Some(rec.prefix(i.prefix())));
            put("key_marker", Some(i.key_marker().is_some().into()));
            put(
                "version_marker",
                Some(i.version_id_marker().is_some().into()),
            );
            put("max_keys", i.max_keys().map(Value::from));
            "ListObjectVersions"
        } else if let Some(i) = input.downcast_ref::<GetBucketVersioningInput>() {
            put("bucket", self.bucket(i.bucket()));
            "GetBucketVersioning"
        } else {
            "unknown"
        }
    }

    fn event(&self, kind: &str, state: &OpInner) -> Map<String, Value> {
        let mut f = Map::new();
        f.insert("kind".into(), kind.into());
        f.insert("op".into(), state.op.into());
        f.insert("name".into(), state.name.into());
        f.insert("account".into(), self.account.clone().into());
        f
    }
}

impl Intercept for RequestRecorder {
    fn name(&self) -> &'static str {
        MARKER
    }

    fn read_before_execution(
        &self,
        context: &BeforeSerializationInterceptorContextRef<'_>,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let mut f = Map::new();
        f.insert("kind".into(), "op-start".into());
        let name = self.describe(context.input(), &mut f);
        let op = self.rec.next_op.fetch_add(1, Ordering::SeqCst);
        self.rec.stats.ops_started.fetch_add(1, Ordering::SeqCst);
        f.insert("op".into(), op.into());
        f.insert("name".into(), name.into());
        f.insert("account".into(), self.account.clone().into());
        self.rec.submit(f);
        cfg.interceptor_state().store_put(OpState(Arc::new(OpInner {
            rec: Arc::clone(&self.rec),
            op,
            name,
            account: self.account.clone(),
            started: Instant::now(),
            ended: AtomicBool::new(false),
            attempts: AtomicU32::new(0),
            attempt_sent: AtomicU64::new(0),
            last_status: AtomicU16::new(0),
        })));
        Ok(())
    }

    fn read_before_transmit(
        &self,
        context: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let Some(OpState(state)) = cfg.load::<OpState>() else {
            return Ok(());
        };
        let attempt = cfg.load::<RequestAttempts>().map_or(0, |a| a.attempts());
        state.attempts.store(attempt, Ordering::SeqCst);
        state.attempt_sent.store(
            micros(state.started.elapsed()).saturating_add(1),
            Ordering::SeqCst,
        );
        let mut f = self.event("attempt-start", state);
        f.insert("attempt".into(), attempt.into());
        if let Some(m) = cfg.load::<Metadata>() {
            f.insert("sdk_op".into(), sanitize_code(m.name()).into());
        }
        if let Some(len) = context
            .request()
            .headers()
            .get("content-length")
            .and_then(|v| v.parse::<u64>().ok())
        {
            f.insert("content_length".into(), len.into());
        }
        self.rec.submit(f);
        Ok(())
    }

    fn read_after_attempt(
        &self,
        context: &FinalizerInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let Some(OpState(state)) = cfg.load::<OpState>() else {
            return Ok(());
        };
        let attempt = cfg.load::<RequestAttempts>().map_or(0, |a| a.attempts());
        let sent = state.attempt_sent.swap(0, Ordering::SeqCst);
        let mut f = self.event("attempt-end", state);
        f.insert("attempt".into(), attempt.into());
        f.insert("sent".into(), (sent > 0).into());
        if sent > 0 {
            let now = micros(state.started.elapsed());
            f.insert(
                "dur_us".into(),
                now.saturating_sub(sent.saturating_sub(1)).into(),
            );
        }
        if let Some(r) = context.response() {
            let status = r.status().as_u16();
            state.last_status.store(status, Ordering::SeqCst);
            f.insert("status".into(), status.into());
            if let Some(v) = self
                .rec
                .id("version-id", r.headers().get("x-amz-version-id"))
            {
                f.insert("version".into(), v);
            }
        }
        let outcome = match context.output_or_error() {
            Some(Ok(_)) => "ok",
            Some(Err(e)) => kind_of(e),
            None => "none",
        };
        f.insert("outcome".into(), outcome.into());
        self.rec.submit(f);
        Ok(())
    }

    fn read_after_execution(
        &self,
        context: &FinalizerInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let Some(OpState(state)) = cfg.load::<OpState>() else {
            return Ok(());
        };
        if state.ended.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.rec.stats.ops_ended.fetch_add(1, Ordering::SeqCst);
        let mut f = self.event("op-end", state);
        let (outcome, code) = match context.output_or_error() {
            Some(Ok(_)) => ("ok", None),
            Some(Err(e)) => (kind_of(e), e.as_operation_error().and_then(error_code)),
            None => ("none", None),
        };
        f.insert("outcome".into(), outcome.into());
        if let Some(code) = code {
            f.insert("code".into(), code.into());
        }
        let status = state.last_status.load(Ordering::SeqCst);
        if status > 0 {
            f.insert("status".into(), status.into());
        }
        f.insert(
            "attempts".into(),
            state.attempts.load(Ordering::SeqCst).into(),
        );
        f.insert("dur_us".into(), micros(state.started.elapsed()).into());
        self.rec.submit(f);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

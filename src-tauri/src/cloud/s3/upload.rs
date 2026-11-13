//! Uploading one file to an object key: [`S3Store::upload`] (ADR-021).
//!
//! The lifecycle, in order:
//!
//! 1. **Checks without a request:** the bucket must be configured for the
//!    account and the key valid; a transfer cancelled already stops here.
//! 2. **Queue:** a slot in the account's transfer queue (cancellable).
//! 3. **First hashing pass** (stage `hashing`): the whole file's SHA-256,
//!    which goes into the object's metadata and so must be known before the
//!    first byte is sent, and for a multipart upload the SHA-256 of every
//!    part at its exact boundaries. The file's size and modification time
//!    are noted first. Cancellable after every chunk.
//! 4. **Destination check:** `HeadObject` on the key; an existing object
//!    fails the upload with [`StorageError::AlreadyExists`] before any file
//!    data is sent. Nothing is ever overwritten on purpose; see "Conflicts".
//! 5. **Sending** (stage `sending`): one `PutObject`, or
//!    `CreateMultipartUpload`, `UploadPart`s one after another, a second
//!    `HeadObject` on the key, and `CompleteMultipartUpload`. The bytes are
//!    hashed again as the connection takes them, per request attempt, and
//!    compared with the first pass before the commit point.
//! 6. **Verification** (stage `verifying`): `HeadObject` on the stored
//!    object (the version the provider reported, if it did) must show the
//!    size sent and the SHA-256 metadata. Only then is the upload reported
//!    as complete.
//!
//! **Commit point.** A single PUT commits right before its last body chunk
//! is handed to the connection: that chunk is held back until the bytes sent
//! so far plus it match the first pass's SHA-256, the file's size and
//! modification time are unchanged, and [`TransferControl`] moves to
//! `Committed` (which fails if a cancel came first). Without its last bytes
//! the provider never receives a complete request, so a cancelled or changed
//! upload cannot become an object. (An empty file has no bytes to hold back:
//! it commits before its request.) A multipart upload commits after every
//! part was sent and checked, the destination re-checked and the file found
//! unchanged, right before `CompleteMultipartUpload`. Before the commit
//! point a cancel stops the upload (`cancel()` returns `true`); after it,
//! `cancel()` returns `false` and the upload finishes (completed, or failed
//! after the commit).
//!
//! **Cancellation and multipart cleanup.** A cancel drops the request in
//! flight (closing its connection). `CreateMultipartUpload` is never
//! dropped in flight, so no upload id can be created without sotyware
//! knowing it. An `UploadPart` whose every byte the connection already took
//! is not dropped at once either: those bytes reach the provider anyway
//! (the socket closes gracefully and its send buffer drains), and a part
//! that lands after the abort can remain stored. The cancel waits for that
//! part's answer, at most [`ClientOptions::part_settle_timeout`](super::ClientOptions),
//! so the abort comes after it (ADR-025). A multipart upload that fails or is cancelled before its
//! commit point is aborted with `AbortMultipartUpload`, in its own task
//! (so it finishes even if the caller drops the upload future) and bounded
//! by [`ClientOptions::abort_timeout`](super::ClientOptions). If the abort
//! fails or times out, the error says so ([`RemoteOutcome::IncompleteParts`]):
//! the parts may remain stored, and billed, at the provider. If the process
//! ends first, nothing aborts the upload either.
//!
//! **Timeouts.** The SDK's read timeout covers the whole HTTP request,
//! including sending the body, so a slow but steady upload taking longer
//! than it would fail. Upload requests therefore run with the read timeout
//! (and the SDK's own upload stall protection) switched off in a
//! per-operation override, and instead fail with [`StorageError::Timeout`]
//! once the connection has taken no body bytes, and no response has come,
//! for [`ClientOptions::stall_timeout`](super::ClientOptions). The connect
//! timeout stays; every other request (HEAD, create, complete, abort,
//! downloads) keeps the client's timeouts.
//!
//! **Conflicts.** The destination is checked before any data is sent and,
//! for multipart, again right before completion. Between the check and the
//! write another client can still create the object, which the upload then
//! replaces (or, on a versioned bucket, hides behind a new version): no
//! conditional write (`If-None-Match: *`) is sent, because not every
//! provider supports it (Backblaze's support is not documented).
//!
//! **After the commit point**, a failure is reported with
//! [`RemoteOutcome::NeedsInspection`]: the provider may have stored the
//! object. It is never deleted automatically (on a versioned bucket a delete
//! would only hide it, and it may be the user's only good copy).

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use aws_sdk_s3::config::StalledStreamProtectionConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::primitives::{ByteStream, Length, SdkBody};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use http_body::{Body, Frame, SizeHint};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use super::{
    MIB, S3Store, SHA256_METADATA, StorageError, TransferControl, UploadPlan, UploadReport,
    classify, hex, io_error, plan_upload,
};

/// At most one progress update per this interval, except the last one of
/// each stage.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Bytes read per step of the first hashing pass.
const HASH_CHUNK: usize = MIB as usize;

/// Bytes per body chunk handed to the connection.
const SEND_CHUNK: usize = 64 * 1024;

/// What an upload is doing, in the order it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UploadStage {
    /// Reading the file to compute its SHA-256.
    Hashing,
    /// Sending the file to the provider.
    Sending,
    /// Checking the stored object's size and SHA-256 metadata.
    Verifying,
}

/// One progress update: within a stage `bytes` never decreases, and stages
/// only move forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadProgress {
    pub stage: UploadStage,
    /// Hashed so far (`hashing`), taken by the connection so far, across all
    /// parts (`sending`), or the whole size (`verifying`).
    pub bytes: u64,
    /// The file's size.
    pub total: u64,
}

/// What a failed or cancelled upload may have left at the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOutcome {
    /// The upload stopped before its commit point: no object was created,
    /// and a multipart upload was aborted (or never started).
    NothingStored,
    /// A multipart upload could not be aborted (the abort failed or did not
    /// answer in time): its parts may remain stored, and billed, until a
    /// bucket lifecycle rule or a later cleanup removes them.
    IncompleteParts { upload_id: String },
    /// The upload passed its commit point, so the provider may have stored
    /// the object, but it could not be verified. It was not deleted.
    NeedsInspection,
}

/// Why an upload failed, and what it may have left at the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadError {
    pub error: StorageError,
    pub remote: RemoteOutcome,
}

impl UploadError {
    fn nothing(error: StorageError) -> Self {
        Self {
            error,
            remote: RemoteOutcome::NothingStored,
        }
    }
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)?;
        match &self.remote {
            RemoteOutcome::NothingStored => Ok(()),
            RemoteOutcome::IncompleteParts { .. } => {
                write!(
                    f,
                    "; parts of the incomplete upload may remain at the provider"
                )
            }
            RemoteOutcome::NeedsInspection => {
                write!(f, "; the provider may have stored the object, check it")
            }
        }
    }
}

/// A file's size and modification time, to notice a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

async fn unchanged(path: &Path, stamp: FileStamp) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|m| FileStamp::of(&m) == stamp)
}

/// What the first hashing pass found.
#[derive(Debug)]
struct FirstPass {
    stamp: FileStamp,
    sha256: String,
    plan: UploadPlan,
    /// The SHA-256 of every part, in order (for a single PUT: the file's).
    parts: Vec<[u8; 32]>,
}

/// Turns a stream of positions into throttled, monotonic progress updates.
struct Reporter<'a> {
    total: u64,
    last: Option<(UploadStage, u64)>,
    at: Option<Instant>,
    /// The newest update held back by the throttle, to be sent once the
    /// interval has passed (see [`Self::flush_at`]).
    pending: Option<(UploadStage, u64)>,
    sink: &'a mut (dyn FnMut(UploadProgress) + Send),
}

impl<'a> Reporter<'a> {
    fn new(sink: &'a mut (dyn FnMut(UploadProgress) + Send)) -> Self {
        Self {
            total: 0,
            last: None,
            at: None,
            pending: None,
            sink,
        }
    }

    fn report(&mut self, stage: UploadStage, bytes: u64, last_of_stage: bool) {
        self.report_at(Instant::now(), stage, bytes, last_of_stage);
    }

    /// Sends an update unless it would go backwards, repeat the previous
    /// one, or come sooner than [`PROGRESS_INTERVAL`] after it (then it is
    /// kept as pending); the first update of a stage and the last one always
    /// go out.
    fn report_at(&mut self, now: Instant, stage: UploadStage, bytes: u64, last_of_stage: bool) {
        let mut bytes = bytes.min(self.total);
        let mut new_stage = true;
        if let Some((last_stage, last_bytes)) = self.last {
            if stage < last_stage {
                return;
            }
            if stage == last_stage {
                // A retried request starts again from its part's beginning;
                // the update waits until it is past the earlier position.
                bytes = bytes.max(last_bytes);
                if bytes == last_bytes {
                    return;
                }
                new_stage = false;
            }
        }
        if !new_stage
            && !last_of_stage
            && self
                .at
                .is_some_and(|at| now.saturating_duration_since(at) < PROGRESS_INTERVAL)
        {
            let held = self
                .pending
                .filter(|(s, _)| *s == stage)
                .map_or(0, |(_, b)| b);
            self.pending = Some((stage, bytes.max(held)));
            return;
        }
        self.last = Some((stage, bytes));
        self.at = Some(now);
        self.pending = None;
        (self.sink)(UploadProgress {
            stage,
            bytes,
            total: self.total,
        });
    }

    /// When the held-back update is due, if there is one. The connection
    /// takes body bytes in bursts, so without this the end of a burst
    /// would only be shown with the next one.
    fn flush_at(&self) -> Option<Instant> {
        self.pending?;
        self.at.map(|at| at + PROGRESS_INTERVAL)
    }

    /// Sends the held-back update if it is due.
    fn flush(&mut self, now: Instant) {
        if self.flush_at().is_some_and(|due| now >= due)
            && let Some((stage, bytes)) = self.pending.take()
        {
            self.report_at(now, stage, bytes, false);
        }
    }
}

/// Why a request body stopped itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// The bytes read differ from the first pass, or the file's size or
    /// modification time changed.
    Changed,
    /// The transfer was cancelled before the commit point.
    Cancelled,
}

#[derive(Debug)]
struct BodyStopped(Stop);

impl fmt::Display for BodyStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Stop::Changed => write!(f, "the file changed while it was being sent"),
            Stop::Cancelled => write!(f, "the upload was cancelled"),
        }
    }
}

impl Error for BodyStopped {}

#[derive(Debug)]
struct ProbeState {
    /// Request attempts that started reading the body (an SDK retry starts
    /// a new one, with a fresh hash).
    attempts: u64,
    /// Bytes the latest attempt handed to the connection.
    sent: u64,
    /// The latest attempt handed over every byte, after its hash matched.
    complete: bool,
    stopped: Option<Stop>,
    /// When a body last handed over bytes, started or finished.
    last_activity: Instant,
}

/// Shared by every attempt's body of one request and by the upload, which
/// reads progress and activity from it.
struct SendProbe {
    /// 1-based; 1 for a single PUT.
    part: u32,
    len: u64,
    /// The first pass's SHA-256 of exactly these bytes.
    expected: [u8; 32],
    path: PathBuf,
    stamp: FileStamp,
    /// Whether handing over the last bytes is the upload's commit point
    /// (a single PUT).
    commits: bool,
    control: Arc<TransferControl>,
    state: Mutex<ProbeState>,
    /// Woken whenever a body hands over bytes, starts or finishes.
    activity: Notify,
}

impl SendProbe {
    fn new(
        part: u32,
        len: u64,
        expected: [u8; 32],
        path: &Path,
        stamp: FileStamp,
        commits: bool,
        control: &Arc<TransferControl>,
    ) -> Self {
        Self {
            part,
            len,
            expected,
            path: path.to_owned(),
            stamp,
            commits,
            control: Arc::clone(control),
            state: Mutex::new(ProbeState {
                attempts: 0,
                sent: 0,
                complete: false,
                stopped: None,
                last_activity: Instant::now(),
            }),
            activity: Notify::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ProbeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut ProbeState)) {
        let mut state = self.lock();
        f(&mut state);
        state.last_activity = Instant::now();
        drop(state);
        self.activity.notify_one();
    }

    fn start_attempt(&self) -> u64 {
        let mut attempt = 0;
        self.update(|s| {
            s.attempts += 1;
            s.sent = 0;
            s.complete = false;
            attempt = s.attempts;
        });
        attempt
    }

    fn stop(&self, stop: Stop) -> BodyStopped {
        let mut first = stop;
        self.update(|s| first = *s.stopped.get_or_insert(stop));
        BodyStopped(first)
    }

    fn stopped(&self) -> Option<Stop> {
        self.lock().stopped
    }

    fn complete(&self) -> bool {
        let s = self.lock();
        s.complete && s.stopped.is_none()
    }

    fn snapshot(&self) -> (u64, Instant) {
        let s = self.lock();
        (s.sent, s.last_activity)
    }

    /// Runs before the last bytes are handed over. `Err` keeps them back,
    /// so the provider never receives a complete request.
    fn last_chunk_check(&self, digest: &[u8]) -> Result<(), BodyStopped> {
        if digest != self.expected {
            return Err(self.stop(Stop::Changed));
        }
        if self.commits {
            // One metadata call, on the connection's task: cheap, and it
            // must happen right here, before the commit.
            let unchanged =
                std::fs::metadata(&self.path).is_ok_and(|m| FileStamp::of(&m) == self.stamp);
            if !unchanged {
                return Err(self.stop(Stop::Changed));
            }
            #[cfg(test)]
            TransferControl::run_hook(&self.control.before_commit);
            if !self.control.commit() {
                return Err(self.stop(Stop::Cancelled));
            }
            #[cfg(test)]
            TransferControl::run_hook(&self.control.after_commit);
        }
        Ok(())
    }
}

/// A request body over one attempt's file reader: hashes the bytes as the
/// connection takes them and holds back the last chunk until
/// [`SendProbe::last_chunk_check`] passes.
struct ProbedBody {
    inner: SdkBody,
    probe: Arc<SendProbe>,
    attempt: Option<u64>,
    hasher: Sha256,
    sent: u64,
}

impl Body for ProbedBody {
    type Data = <SdkBody as Body>::Data;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = &mut *self;
        let probe = Arc::clone(&this.probe);
        let attempt = *this.attempt.get_or_insert_with(|| probe.start_attempt());
        if let Some(stop) = probe.stopped() {
            return Poll::Ready(Some(Err(Box::new(BodyStopped(stop)))));
        }
        let frame = match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) if this.sent == probe.len => return Poll::Ready(None),
            // The file ended early: it was truncated.
            Poll::Ready(None) => {
                return Poll::Ready(Some(Err(Box::new(probe.stop(Stop::Changed)))));
            }
            Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
            Poll::Ready(Some(Ok(frame))) => frame,
        };
        let data = match frame.into_data() {
            Ok(data) => data,
            Err(frame) => return Poll::Ready(Some(Ok(frame))),
        };
        let sent = this.sent + data.len() as u64;
        if sent > probe.len {
            return Poll::Ready(Some(Err(Box::new(probe.stop(Stop::Changed)))));
        }
        this.hasher.update(&data);
        let last = sent == probe.len;
        if last && let Err(e) = probe.last_chunk_check(&this.hasher.clone().finalize()) {
            return Poll::Ready(Some(Err(Box::new(e))));
        }
        #[cfg(test)]
        let first = this.sent == 0;
        this.sent = sent;
        probe.update(|s| {
            if s.attempts == attempt {
                s.sent = sent;
                s.complete = last;
            }
        });
        #[cfg(test)]
        if first {
            let hook = {
                let mut hook = probe.control.during_send.lock().unwrap();
                if hook.as_ref().is_some_and(|(part, _)| *part == probe.part) {
                    hook.take()
                } else {
                    None
                }
            };
            if let Some((_, f)) = hook {
                f();
            }
        }
        Poll::Ready(Some(Ok(Frame::data(data))))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

enum Step<T> {
    Done(T),
    Cancelled,
    Activity,
}

/// How an `AbortMultipartUpload` went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortOutcome {
    /// The provider removed the upload and its parts.
    Aborted,
    /// The provider no longer knows the upload id (aborted or completed
    /// before).
    Gone,
    /// The abort failed or did not answer within the abort timeout: the
    /// parts may remain.
    Failed,
}

use AbortOutcome as Abort;

impl S3Store {
    /// Uploads `path` to `bucket/key` (the key exactly as given; single PUT
    /// or multipart per [`plan_upload`]), storing the file's SHA-256 with the
    /// object, and fails with [`StorageError::AlreadyExists`] if the key
    /// already holds an object. Waits for a slot in the account's transfer
    /// queue first. Cancellable through `control` until the commit point;
    /// `progress` gets throttled [`UploadProgress`] updates. Returns only
    /// after the stored object was verified. The lifecycle, commit point,
    /// timeouts and limits are described in this module's documentation.
    pub async fn upload(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
        control: &Arc<TransferControl>,
        progress: &mut (dyn FnMut(UploadProgress) + Send),
    ) -> Result<UploadReport, UploadError> {
        self.check_target(bucket, Some(key))
            .map_err(UploadError::nothing)?;
        if control.is_cancelled() {
            return Err(UploadError::nothing(StorageError::Cancelled));
        }
        let _slot = control
            .unless_cancelled(self.transfer_slot())
            .await
            .ok_or(StorageError::Cancelled)
            .and_then(|slot| slot)
            .map_err(UploadError::nothing)?;
        let mut reporter = Reporter::new(progress);
        let first = self
            .first_pass(path, control, &mut reporter)
            .await
            .map_err(UploadError::nothing)?;
        self.check_destination(bucket, key, control)
            .await
            .map_err(UploadError::nothing)?;
        let size = first.stamp.len;
        let (parts, version_id) = match first.plan {
            UploadPlan::Single => (
                1,
                self.put_single(bucket, key, path, &first, control, &mut reporter)
                    .await?,
            ),
            UploadPlan::Multipart { part_size, parts } => (
                parts,
                self.put_multipart(bucket, key, path, &first, part_size, control, &mut reporter)
                    .await?,
            ),
        };
        reporter.report(UploadStage::Sending, size, true);
        reporter.report(UploadStage::Verifying, size, true);
        self.verify_upload(bucket, key, version_id.as_deref(), size, &first.sha256)
            .await
            .map_err(|error| UploadError {
                error,
                remote: RemoteOutcome::NeedsInspection,
            })?;
        Ok(UploadReport {
            size,
            sha256: first.sha256,
            parts,
            version_id,
        })
    }

    /// Settings for requests that carry file data: no read timeout and no
    /// SDK stall protection (both would cover sending the body); the
    /// application's own stall detection in [`Self::drive`] replaces them.
    fn upload_config(&self) -> aws_sdk_s3::config::Builder {
        aws_sdk_s3::config::Builder::default()
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(self.options.connect_timeout)
                    .disable_read_timeout()
                    .build(),
            )
            .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
    }

    async fn first_pass(
        &self,
        path: &Path,
        control: &TransferControl,
        reporter: &mut Reporter<'_>,
    ) -> Result<FirstPass, StorageError> {
        let mut file = tokio::fs::File::open(path).await.map_err(io_error(path))?;
        let meta = file.metadata().await.map_err(io_error(path))?;
        if !meta.is_file() {
            return Err(StorageError::Io(format!("{}: not a file", path.display())));
        }
        let stamp = FileStamp::of(&meta);
        let plan = plan_upload(stamp.len, &self.account.profile.limits, &self.options)?;
        let part_size = match plan {
            UploadPlan::Single => u64::MAX,
            UploadPlan::Multipart { part_size, .. } => part_size,
        };
        reporter.total = stamp.len;
        reporter.report(UploadStage::Hashing, 0, false);
        let mut whole = Sha256::new();
        let mut part = Sha256::new();
        let mut in_part = 0u64;
        let mut parts = Vec::new();
        let mut buf = vec![0u8; HASH_CHUNK];
        let mut size = 0u64;
        loop {
            if control.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            let n = file.read(&mut buf).await.map_err(io_error(path))?;
            if n == 0 {
                break;
            }
            let mut chunk = &buf[..n];
            whole.update(chunk);
            while !chunk.is_empty() {
                let take = usize::try_from((part_size - in_part).min(chunk.len() as u64))
                    .unwrap_or(chunk.len());
                part.update(&chunk[..take]);
                in_part += take as u64;
                chunk = &chunk[take..];
                if in_part == part_size {
                    parts.push(part.finalize_reset().into());
                    in_part = 0;
                }
            }
            size += n as u64;
            reporter.report(UploadStage::Hashing, size, false);
            #[cfg(test)]
            TransferControl::run_hook(&control.during_hashing);
        }
        if size != stamp.len {
            return Err(StorageError::FileChanged(path.display().to_string()));
        }
        if in_part > 0 || parts.is_empty() {
            parts.push(part.finalize().into());
        }
        if control.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        reporter.report(UploadStage::Hashing, size, true);
        Ok(FirstPass {
            stamp,
            sha256: hex(&whole.finalize()),
            plan,
            parts,
        })
    }

    /// Fails unless `bucket/key` is free. Cancellable.
    async fn check_destination(
        &self,
        bucket: &str,
        key: &str,
        control: &TransferControl,
    ) -> Result<(), StorageError> {
        match control
            .unless_cancelled(self.head_object(bucket, key))
            .await
        {
            None => Err(StorageError::Cancelled),
            Some(Ok(None)) => Ok(()),
            Some(Ok(Some(_))) => Err(StorageError::AlreadyExists(key.to_owned())),
            Some(Err(e)) => Err(e),
        }
    }

    /// The reader for `len` bytes of `path` from `offset`, wrapped per
    /// attempt in a [`ProbedBody`]. The SDK rebuilds the body for every
    /// attempt, which reopens the file and starts a fresh hash.
    async fn probed_body(
        &self,
        offset: u64,
        probe: &Arc<SendProbe>,
    ) -> Result<ByteStream, StorageError> {
        let reader = ByteStream::read_from()
            .path(&probe.path)
            .offset(offset)
            .length(Length::Exact(probe.len))
            .buffer_size(SEND_CHUNK)
            .build()
            .await;
        let reader = match reader {
            Ok(reader) => reader,
            // Fails when the file became shorter than the part.
            Err(_) if !unchanged(&probe.path, probe.stamp).await => {
                return Err(StorageError::FileChanged(probe.path.display().to_string()));
            }
            Err(e) => return Err(StorageError::Io(format!("{}: {e}", probe.path.display()))),
        };
        let probe = Arc::clone(probe);
        Ok(ByteStream::new(reader.into_inner().map_preserve_contents(
            move |inner| {
                SdkBody::from_body_1_x(ProbedBody {
                    inner,
                    probe: Arc::clone(&probe),
                    attempt: None,
                    hasher: Sha256::new(),
                    sent: 0,
                })
            },
        )))
    }

    /// Runs one request that carries file data: its result, or `Cancelled`
    /// as soon as the transfer is cancelled (dropping the request), or
    /// `Timeout` once its body has handed over nothing, and no response came,
    /// for `stall_timeout`. Reports progress as the body moves, `base` being
    /// the bytes of the parts before this one.
    async fn drive<T>(
        &self,
        send: impl Future<Output = T>,
        probe: &SendProbe,
        control: &TransferControl,
        reporter: &mut Reporter<'_>,
        base: u64,
    ) -> Result<T, StorageError> {
        let stall = self.options.stall_timeout;
        let mut send = pin!(send);
        let mut cancelled = pin!(control.cancelled());
        let mut deadline = pin!(tokio::time::sleep(stall));
        loop {
            let mut notified = pin!(probe.activity.notified());
            let step = std::future::poll_fn(|cx| {
                if let Poll::Ready(v) = send.as_mut().poll(cx) {
                    return Poll::Ready(Step::Done(v));
                }
                if cancelled.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Step::Cancelled);
                }
                if notified.as_mut().poll(cx).is_ready() || deadline.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Step::Activity);
                }
                Poll::Pending
            })
            .await;
            match step {
                Step::Done(v) => return Ok(v),
                Step::Cancelled => {
                    // A part whose every byte the connection took reaches
                    // the provider even if this request is dropped (the
                    // socket closes gracefully, its send buffer still
                    // drains). Wait for its answer, bounded, so the abort
                    // that follows comes after it and removes it, instead
                    // of a part that lands after the abort and remains.
                    // A part not handed over completely can never be
                    // stored: dropping it is enough.
                    if !probe.commits && probe.complete() {
                        let settle = self.options.part_settle_timeout;
                        if tokio::time::timeout(settle, send.as_mut()).await.is_err() {
                            eprintln!(
                                "sotyware: part {} got no answer within {} s of cancelling; it may still be stored after the abort",
                                probe.part,
                                settle.as_secs_f32()
                            );
                        }
                    }
                    return Err(StorageError::Cancelled);
                }
                Step::Activity => {
                    let (sent, last_activity) = probe.snapshot();
                    let now = Instant::now();
                    reporter.flush(now);
                    reporter.report_at(now, UploadStage::Sending, base + sent, false);
                    let until = last_activity + stall;
                    if now >= until {
                        return Err(StorageError::Timeout(format!(
                            "upload part {}: no progress for {} s",
                            probe.part,
                            stall.as_secs_f32()
                        )));
                    }
                    // One timer: the stall deadline, or a held-back progress
                    // update if that is due sooner.
                    let wake = reporter.flush_at().map_or(until, |due| due.min(until));
                    deadline
                        .as_mut()
                        .reset(tokio::time::Instant::from_std(wake));
                }
            }
        }
    }

    /// The error of a request that failed: a change of the file or a cancel
    /// found by its body wins over what the SDK made of it.
    async fn send_failure(&self, error: StorageError, probe: &SendProbe) -> StorageError {
        match probe.stopped() {
            Some(Stop::Changed) => StorageError::FileChanged(probe.path.display().to_string()),
            Some(Stop::Cancelled) => StorageError::Cancelled,
            None if !unchanged(&probe.path, probe.stamp).await => {
                StorageError::FileChanged(probe.path.display().to_string())
            }
            None => error,
        }
    }

    async fn put_single(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
        first: &FirstPass,
        control: &Arc<TransferControl>,
        reporter: &mut Reporter<'_>,
    ) -> Result<Option<String>, UploadError> {
        let size = first.stamp.len;
        let remote = || {
            if control.is_committed() {
                RemoteOutcome::NeedsInspection
            } else {
                RemoteOutcome::NothingStored
            }
        };
        let probe = Arc::new(SendProbe::new(
            1,
            size,
            first.parts[0],
            path,
            first.stamp,
            true,
            control,
        ));
        let body = if size == 0 {
            // No last chunk to hold back: commit before the request.
            if !unchanged(path, first.stamp).await {
                return Err(UploadError::nothing(StorageError::FileChanged(
                    path.display().to_string(),
                )));
            }
            #[cfg(test)]
            TransferControl::run_hook(&control.before_commit);
            if !control.commit() {
                return Err(UploadError::nothing(StorageError::Cancelled));
            }
            #[cfg(test)]
            TransferControl::run_hook(&control.after_commit);
            probe.update(|s| s.complete = true);
            ByteStream::from_static(b"")
        } else {
            self.probed_body(0, &probe)
                .await
                .map_err(UploadError::nothing)?
        };
        let send = self
            .client
            .put_object()
            .bucket(bucket)
            .key(key)
            .content_length(i64::try_from(size).unwrap_or(i64::MAX))
            .metadata(SHA256_METADATA, &first.sha256)
            .body(body)
            .customize()
            .config_override(self.upload_config())
            .send();
        let error = match self.drive(send, &probe, control, reporter, 0).await {
            Ok(Ok(_)) if !probe.complete() => StorageError::Integrity(format!(
                "{key}: the provider answered before the whole file was sent"
            )),
            Ok(Ok(out)) => return Ok(out.version_id().map(str::to_owned)),
            Ok(Err(e)) => self.send_failure(classify("upload", e), &probe).await,
            Err(e) => e,
        };
        Err(UploadError {
            error,
            remote: remote(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn put_multipart(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
        first: &FirstPass,
        part_size: u64,
        control: &Arc<TransferControl>,
        reporter: &mut Reporter<'_>,
    ) -> Result<Option<String>, UploadError> {
        // Never dropped in flight, even when cancelled: the provider could
        // open an upload whose id sotyware would never learn, so nothing
        // could abort it.
        let created = self
            .client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .metadata(SHA256_METADATA, &first.sha256)
            .send()
            .await
            .map_err(|e| UploadError::nothing(classify("start multipart upload", e)))?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| {
                UploadError::nothing(StorageError::Provider {
                    status: 0,
                    code: String::new(),
                    message: "start multipart upload: no upload id".into(),
                })
            })?
            .to_owned();
        control.set_multipart(Some(upload_id.clone()));
        let staged = async {
            let parts = self
                .send_parts(
                    bucket, key, &upload_id, path, first, part_size, control, reporter,
                )
                .await?;
            self.check_destination(bucket, key, control).await?;
            if !unchanged(path, first.stamp).await {
                return Err(StorageError::FileChanged(path.display().to_string()));
            }
            #[cfg(test)]
            TransferControl::run_hook(&control.before_commit);
            if !control.commit() {
                return Err(StorageError::Cancelled);
            }
            #[cfg(test)]
            TransferControl::run_hook(&control.after_commit);
            Ok(parts)
        }
        .await;
        let parts = match staged {
            Ok(parts) => parts,
            Err(error) => {
                let remote = match self.abort(bucket, key, &upload_id).await {
                    Abort::Aborted | Abort::Gone => {
                        control.set_multipart(None);
                        RemoteOutcome::NothingStored
                    }
                    Abort::Failed => RemoteOutcome::IncompleteParts { upload_id },
                };
                return Err(UploadError { error, remote });
            }
        };
        // Past the commit point: not cancellable.
        let completed = self
            .client
            .complete_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(parts))
                    .build(),
            )
            .send()
            .await;
        match completed {
            Ok(out) => {
                control.set_multipart(None);
                Ok(out.version_id().map(str::to_owned))
            }
            Err(e) => {
                let error = classify("complete multipart upload", e);
                // The completion may have taken effect even though it failed
                // here; an abort that still finds the upload proves it did not.
                let remote = match self.abort(bucket, key, &upload_id).await {
                    Abort::Aborted => RemoteOutcome::NothingStored,
                    Abort::Gone => RemoteOutcome::NeedsInspection,
                    Abort::Failed => RemoteOutcome::IncompleteParts { upload_id },
                };
                if !matches!(remote, RemoteOutcome::IncompleteParts { .. }) {
                    control.set_multipart(None);
                }
                Err(UploadError { error, remote })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_parts(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        path: &Path,
        first: &FirstPass,
        part_size: u64,
        control: &Arc<TransferControl>,
        reporter: &mut Reporter<'_>,
    ) -> Result<Vec<CompletedPart>, StorageError> {
        let size = first.stamp.len;
        let mut completed = Vec::with_capacity(first.parts.len());
        let mut offset = 0u64;
        for (index, expected) in first.parts.iter().enumerate() {
            if control.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let len = part_size.min(size - offset);
            let probe = Arc::new(SendProbe::new(
                number,
                len,
                *expected,
                path,
                first.stamp,
                false,
                control,
            ));
            let body = self.probed_body(offset, &probe).await?;
            let send = self
                .client
                .upload_part()
                .bucket(bucket)
                .key(key)
                .upload_id(upload_id)
                .part_number(i32::try_from(number).unwrap_or(i32::MAX))
                .content_length(i64::try_from(len).unwrap_or(i64::MAX))
                .body(body)
                .customize()
                .config_override(self.upload_config())
                .send();
            let out = match self.drive(send, &probe, control, reporter, offset).await? {
                Ok(out) => out,
                Err(e) => {
                    let error = classify(&format!("upload part {number}"), e);
                    return Err(self.send_failure(error, &probe).await);
                }
            };
            if !probe.complete() {
                return Err(StorageError::Integrity(format!(
                    "{key}: the provider answered before part {number} was sent"
                )));
            }
            completed.push(
                CompletedPart::builder()
                    .part_number(i32::try_from(number).unwrap_or(i32::MAX))
                    .set_e_tag(out.e_tag().map(str::to_owned))
                    .build(),
            );
            offset += len;
        }
        Ok(completed)
    }

    /// Aborts a multipart upload an earlier [`Self::upload`] could not abort
    /// ([`RemoteOutcome::IncompleteParts`]): one `AbortMultipartUpload`, in
    /// its own task, bounded by `abort_timeout`. Best effort; the outcome
    /// says whether the parts are gone.
    pub async fn abort_incomplete_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> AbortOutcome {
        if self.check_target(bucket, Some(key)).is_err() {
            return Abort::Failed;
        }
        self.abort(bucket, key, upload_id).await
    }

    /// Sends `AbortMultipartUpload` in its own task, so it finishes even if
    /// the upload future is dropped, and waits at most `abort_timeout`.
    async fn abort(&self, bucket: &str, key: &str, upload_id: &str) -> Abort {
        let request = self
            .client
            .abort_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id);
        let limit = self.options.abort_timeout;
        let task = tokio::spawn(async move { tokio::time::timeout(limit, request.send()).await });
        match task.await {
            Ok(Ok(Ok(_))) => Abort::Aborted,
            Ok(Ok(Err(e))) => match classify("abort multipart upload", e) {
                StorageError::NotFound(_) => Abort::Gone,
                e => {
                    eprintln!(
                        "sotyware: cannot abort the multipart upload of '{key}'; its parts may remain: {e}"
                    );
                    Abort::Failed
                }
            },
            Ok(Err(_)) => {
                eprintln!(
                    "sotyware: aborting the multipart upload of '{key}' took longer than {} s; its parts may remain",
                    limit.as_secs_f32()
                );
                Abort::Failed
            }
            Err(e) => {
                eprintln!(
                    "sotyware: cannot abort the multipart upload of '{key}'; its parts may remain: {e}"
                );
                Abort::Failed
            }
        }
    }

    /// Checks the stored object (the version created, if the provider named
    /// one): its size and the SHA-256 metadata must be what was sent.
    async fn verify_upload(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
        size: u64,
        sha256: &str,
    ) -> Result<(), StorageError> {
        let out = self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await;
        let out = match out {
            Ok(out) => out,
            Err(e) => {
                return Err(match classify("verify upload", e) {
                    StorageError::NotFound(_) => StorageError::Integrity(format!(
                        "{key}: the object is not there after the upload"
                    )),
                    e => e,
                });
            }
        };
        let stored_size = out.content_length().and_then(|s| u64::try_from(s).ok());
        if stored_size != Some(size) {
            return Err(StorageError::Integrity(format!(
                "{key}: the provider reports {} bytes, {size} were sent",
                stored_size.map_or_else(|| "no size".to_owned(), |s| s.to_string())
            )));
        }
        let stored_sha = out.metadata().and_then(|m| m.get(SHA256_METADATA));
        if !stored_sha.is_some_and(|s| s.eq_ignore_ascii_case(sha256)) {
            return Err(StorageError::Integrity(format!(
                "{key}: the stored SHA-256 metadata is missing or differs"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

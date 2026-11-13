//! The one storage adapter: any S3-compatible provider through the official
//! AWS SDK for Rust (`aws-sdk-s3`), configured from a [`ResolvedAccount`].
//!
//! Integrity is checked end to end, independently of the provider: before an
//! upload the file's SHA-256 is computed and stored with the object as user
//! metadata (`x-amz-meta-sotyware-sha256`), the bytes are hashed again as
//! they are sent, and the stored object is checked by `HeadObject` (see
//! [`S3Store::upload`] in `upload.rs`); a download is streamed to a new
//! temporary file next to its destination, hashed while it streams, and moved
//! into place only if the size (and, when the object has one, the stored
//! SHA-256) match. Nothing relies on ETags (an ETag is not an MD5 for
//! multipart uploads and differs between providers).
//!
//! A transfer can be cancelled through its [`TransferControl`], which also
//! settles the race between cancelling and finishing: once a transfer has
//! passed its commit point (a download: the verified file is being moved
//! into place; an upload: the provider is being asked to store the object),
//! a cancel request no longer has any effect.
//!
//! Each [`S3Store`] belongs to one account and has its own HTTP connection
//! pool and its own transfer queue (a semaphore of
//! `max_concurrent_transfers` permits), so a slow or failing account never
//! holds up another one.

use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Poll;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials as SdkCredentials, Region, RequestChecksumCalculation,
    ResponseChecksumValidation,
};
use aws_sdk_s3::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_smithy_http_client::tls;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Semaphore, watch};

use super::account::ResolvedAccount;
use super::credentials::Credentials;
use super::provider::{SdkChecksums, TransferLimits};

mod upload;

#[cfg(test)]
mod hook_assumptions;

pub use upload::{AbortOutcome, RemoteOutcome, UploadError, UploadProgress, UploadStage};

/// User metadata key holding the hex SHA-256 of the object's content.
pub const SHA256_METADATA: &str = "sotyware-sha256";

/// Suffix of the temporary file a download is written to.
const PARTIAL_SUFFIX: &str = ".sotyware-partial";

/// At most this many bytes of the destination's name go into the temporary
/// file's name, so the temporary name stays within every file system's
/// 255-byte limit.
const PARTIAL_NAME_BYTES: usize = 100;

const MIB: u64 = 1024 * 1024;

/// Longest object key S3 allows, in bytes.
const MAX_KEY_BYTES: usize = 1024;

/// Client settings that are not part of the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientOptions {
    pub connect_timeout: Duration,
    /// Longest wait for the next bytes of a response.
    pub read_timeout: Duration,
    /// Attempts per request, including the first (the SDK's standard retry
    /// with exponential backoff; only transient errors are retried).
    pub max_attempts: u32,
    /// Files up to this size are sent with one PUT.
    pub multipart_threshold: u64,
    /// Preferred part size; raised when a file would need too many parts.
    pub part_size: u64,
    /// Upload requests (`PutObject`, `UploadPart`) fail with a timeout once
    /// the connection has taken no body bytes, and no response has arrived,
    /// for this long. `read_timeout` does not apply to them: the SDK measures
    /// it over the whole request, body included, so a slow but steady upload
    /// would fail (ADR-021).
    pub stall_timeout: Duration,
    /// Longest wait for `AbortMultipartUpload` after a failed or cancelled
    /// multipart upload, retries included.
    pub abort_timeout: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(60),
            max_attempts: 3,
            multipart_threshold: 64 * MIB,
            part_size: 16 * MIB,
            stall_timeout: Duration::from_secs(60),
            abort_timeout: Duration::from_secs(30),
        }
    }
}

/// Why a storage operation failed, sorted by what the user (or a retry) can do
/// about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The key id is unknown or the secret is wrong.
    InvalidCredentials(String),
    /// The credentials are valid but not allowed to do this.
    AccessDenied(String),
    /// No such bucket or object.
    NotFound(String),
    /// The provider could not be reached, or the connection broke.
    Network(String),
    /// The provider did not answer in time.
    Timeout(String),
    /// Downloaded or uploaded content does not match its checksum or size.
    Integrity(String),
    /// The local file changed while it was being uploaded.
    FileChanged(String),
    /// An upload's destination key already holds an object; nothing was
    /// overwritten.
    AlreadyExists(String),
    /// The file is larger than the provider accepts.
    TooLarge { size: u64, max: u64 },
    /// The bucket is not one of the account's configured buckets, or the key
    /// is not valid.
    Refused(String),
    /// A local file could not be read or written.
    Io(String),
    /// A download's local file could not be created, written or moved into
    /// place; `kind` tells disk full, permission denied, a vanished folder
    /// and an existing destination apart.
    LocalWrite {
        kind: io::ErrorKind,
        message: String,
    },
    /// The transfer was cancelled through its [`TransferControl`].
    Cancelled,
    /// Any other error the provider returned.
    Provider {
        status: u16,
        code: String,
        message: String,
    },
}

impl StorageError {
    /// Whether trying again later may succeed without any change.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::Timeout(_) => true,
            Self::Provider { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentials(m) => write!(f, "invalid credentials: {m}"),
            Self::AccessDenied(m) => write!(f, "access denied: {m}"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::Timeout(m) => write!(f, "timed out: {m}"),
            Self::Integrity(m) => write!(f, "integrity check failed: {m}"),
            Self::FileChanged(m) => write!(f, "file changed during upload: {m}"),
            Self::AlreadyExists(m) => write!(f, "already exists: {m}"),
            Self::TooLarge { size, max } => {
                write!(
                    f,
                    "file of {size} bytes exceeds the provider's limit of {max} bytes"
                )
            }
            Self::Refused(m) => write!(f, "refused: {m}"),
            Self::Io(m) => write!(f, "local file error: {m}"),
            Self::LocalWrite { message, .. } => write!(f, "cannot write the local file: {message}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Provider {
                status,
                code,
                message,
            } => write!(f, "provider error {status} {code}: {message}"),
        }
    }
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> StorageError + '_ {
    move |e| StorageError::Io(format!("{}: {e}", path.display()))
}

/// Maps a provider's error response to a [`StorageError`]. Pure, so the
/// mapping is unit-tested without a server.
pub fn classify_response(status: u16, code: Option<&str>, message: &str, op: &str) -> StorageError {
    let what = |m: &str| {
        if m.is_empty() {
            op.to_owned()
        } else {
            format!("{op}: {m}")
        }
    };
    match code {
        // `NotSignedUp` is what s3s (the test server) and Amazon return for an
        // unknown key id. Which codes Backblaze and Hetzner use is checked by
        // the opt-in integration tests, not documented.
        Some(
            "InvalidAccessKeyId"
            | "SignatureDoesNotMatch"
            | "InvalidToken"
            | "ExpiredToken"
            | "InvalidSecurity"
            | "AuthorizationHeaderMalformed"
            | "NotSignedUp",
        ) => StorageError::InvalidCredentials(what(message)),
        Some("AccessDenied" | "AllAccessDisabled" | "AccountProblem") => {
            StorageError::AccessDenied(what(message))
        }
        Some("NoSuchBucket" | "NoSuchKey" | "NoSuchUpload" | "NotFound") => {
            StorageError::NotFound(what(message))
        }
        Some("RequestTimeout") => StorageError::Timeout(what(message)),
        // HEAD responses have no body, so no code: only the status is known.
        None if status == 404 => StorageError::NotFound(what(message)),
        None if status == 403 => StorageError::AccessDenied(what(message)),
        None if status == 401 => StorageError::InvalidCredentials(what(message)),
        _ => StorageError::Provider {
            status,
            code: code.unwrap_or("").to_owned(),
            message: what(message),
        },
    }
}

fn classify<E>(op: &str, err: SdkError<E, aws_sdk_s3::config::http::HttpResponse>) -> StorageError
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    match &err {
        SdkError::ServiceError(service) => {
            let status = service.raw().status().as_u16();
            let e = service.err();
            classify_response(status, e.code(), e.message().unwrap_or(""), op)
        }
        SdkError::TimeoutError(_) => StorageError::Timeout(op.to_owned()),
        SdkError::DispatchFailure(d) if d.is_timeout() => {
            StorageError::Timeout(format!("{op}: {}", DisplayErrorContext(&err)))
        }
        SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            StorageError::Network(format!("{op}: {}", DisplayErrorContext(&err)))
        }
        _ => StorageError::Provider {
            status: 0,
            code: String::new(),
            message: format!("{op}: {}", DisplayErrorContext(&err)),
        },
    }
}

/// How a file will be uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadPlan {
    Single,
    Multipart { part_size: u64, parts: u64 },
}

/// Chooses single PUT or multipart for a file of `size` bytes. A multipart
/// upload always has at least two parts (Backblaze requires that).
pub fn plan_upload(
    size: u64,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Result<UploadPlan, StorageError> {
    if size > limits.max_object_size {
        return Err(StorageError::TooLarge {
            size,
            max: limits.max_object_size,
        });
    }
    let single_max = options.multipart_threshold.min(limits.max_single_put);
    let fewest_bytes_per_part = size.div_ceil(limits.max_parts);
    let part_size = options
        .part_size
        .max(limits.min_part_size)
        .max(fewest_bytes_per_part)
        .div_ceil(MIB)
        .saturating_mul(MIB)
        .min(limits.max_part_size);
    let parts = size.div_ceil(part_size);
    if size <= single_max || parts < 2 {
        if size > limits.max_single_put {
            return Err(StorageError::TooLarge {
                size,
                max: limits.max_single_put,
            });
        }
        return Ok(UploadPlan::Single);
    }
    if parts > limits.max_parts {
        return Err(StorageError::TooLarge {
            size,
            max: limits.max_part_size.saturating_mul(limits.max_parts),
        });
    }
    Ok(UploadPlan::Multipart { part_size, parts })
}

/// An object as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub key: String,
    pub size: u64,
    /// RFC 3339, as the provider reports it.
    pub last_modified: Option<String>,
}

/// What separates "folders" in object keys. S3 has no folders: a folder is
/// a key prefix ending in this character.
pub const FOLDER_DELIMITER: &str = "/";

/// One page of a folder listing, as the provider returned it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderPage {
    /// The full prefixes of immediate subfolders (each ends in `/`).
    pub prefixes: Vec<String>,
    /// Objects directly in the folder, with their full keys. May include the
    /// folder's own marker object (key equal to the prefix).
    pub objects: Vec<ObjectInfo>,
    /// The provider's token for the next page, if there is one.
    pub next_token: Option<String>,
}

/// An object's details from a HEAD request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDetails {
    pub size: u64,
    /// The SHA-256 sotyware stored at upload, if it uploaded the object.
    pub sha256: Option<String>,
    pub version_id: Option<String>,
}

/// What an upload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadReport {
    pub size: u64,
    pub sha256: String,
    /// 1 for a single PUT.
    pub parts: u64,
    /// The version created, on a versioned bucket. Deleting this exact version
    /// removes the upload completely (a plain delete only hides it there).
    pub version_id: Option<String>,
}

/// How a download was verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    /// The content matches the SHA-256 stored at upload.
    Sha256Matched,
    /// The object has no stored SHA-256 (not uploaded by sotyware); only the
    /// size was checked.
    SizeOnly,
}

/// What a download did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadReport {
    pub size: u64,
    pub sha256: String,
    pub verification: Verification,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn write_error(path: &Path) -> impl Fn(io::Error) -> StorageError + '_ {
    move |e| StorageError::LocalWrite {
        kind: e.kind(),
        message: format!("{}: {e}", path.display()),
    }
}

/// The name of the `attempt`th temporary file for `dest`: hidden on Linux,
/// next to the destination (so the final move is a rename on the same file
/// system), recognisably sotyware's, and unique per process and attempt.
fn partial_name(dest: &Path, attempt: u64) -> String {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut end = name.len().min(PARTIAL_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        ".{}.{}-{attempt}{PARTIAL_SUFFIX}",
        &name[..end],
        std::process::id()
    )
}

/// Creates a new, empty temporary file in `dest`'s folder. Never opens an
/// existing file: a name that is taken is skipped.
async fn create_partial(dest: &Path) -> Result<(PathBuf, tokio::fs::File), StorageError> {
    static ATTEMPT: AtomicU64 = AtomicU64::new(0);
    let dir = match dest.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    for _ in 0..100 {
        let path = dir.join(partial_name(dest, ATTEMPT.fetch_add(1, Ordering::Relaxed)));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(write_error(&path)(e)),
        }
    }
    Err(StorageError::LocalWrite {
        kind: io::ErrorKind::AlreadyExists,
        message: format!("{}: no free temporary file name", dir.display()),
    })
}

/// What a download does when its destination exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    /// Fail, and leave the existing file alone: also when it only appears
    /// while the download runs.
    Refuse,
    /// Replace it, in one rename, once the download is verified. Only for a
    /// file the user chose to replace.
    Replace,
}

/// Where a transfer stands. The only transitions are
/// `Running → Cancelling` ([`TransferControl::cancel`]) and
/// `Running → Committed` (the transfer's commit point); both end states are
/// final, so whichever of the two comes first wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Running,
    Cancelling,
    /// Past the commit point (a download: the verified file is being moved
    /// into place; an upload: the provider is being asked to store the
    /// object); too late to cancel.
    Committed,
}

/// Cancels one transfer, and decides the race between cancelling and
/// finishing in one place: whichever of [`Self::cancel`] and the transfer's
/// commit point comes first wins, and the other one sees that.
pub struct TransferControl {
    phase: watch::Sender<Phase>,
    /// The temporary file while it exists, so it can be removed at shutdown
    /// even if the transfer cannot clean up in time.
    partial: Mutex<Option<PathBuf>>,
    /// An upload's multipart upload id from `CreateMultipartUpload` until it
    /// is completed or aborted, so it is never lost while an abort may still
    /// be needed (ADR-021).
    multipart: Mutex<Option<String>>,
    #[cfg(test)]
    fail_write_after: Mutex<Option<u64>>,
    #[cfg(test)]
    before_commit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_commit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    during_hashing: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    during_send: Mutex<Option<(u32, Hook)>>,
}

#[cfg(test)]
type Hook = Box<dyn FnOnce() + Send>;

impl fmt::Debug for TransferControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferControl")
            .field("phase", &*self.phase.borrow())
            .finish_non_exhaustive()
    }
}

impl Default for TransferControl {
    fn default() -> Self {
        Self::new()
    }
}

impl TransferControl {
    pub fn new() -> Self {
        Self {
            phase: watch::Sender::new(Phase::Running),
            partial: Mutex::new(None),
            multipart: Mutex::new(None),
            #[cfg(test)]
            fail_write_after: Mutex::new(None),
            #[cfg(test)]
            before_commit: Mutex::new(None),
            #[cfg(test)]
            after_commit: Mutex::new(None),
            #[cfg(test)]
            during_hashing: Mutex::new(None),
            #[cfg(test)]
            during_send: Mutex::new(None),
        }
    }

    /// Asks the transfer to stop. Returns `true` if it will stop (also when
    /// it was asked before), `false` if it has passed its commit point and
    /// will finish (completed, or failed after the commit). Safe to call any
    /// number of times.
    pub fn cancel(&self) -> bool {
        let mut stops = false;
        self.phase.send_if_modified(|phase| match phase {
            Phase::Running => {
                *phase = Phase::Cancelling;
                stops = true;
                true
            }
            Phase::Cancelling => {
                stops = true;
                false
            }
            Phase::Committed => false,
        });
        stops
    }

    pub fn is_cancelled(&self) -> bool {
        *self.phase.borrow() == Phase::Cancelling
    }

    /// Moves to the final step unless cancelled first. `true` again once
    /// committed (an upload request retried by the SDK after its commit point
    /// commits again).
    fn commit(&self) -> bool {
        let mut committed = false;
        self.phase.send_if_modified(|phase| match phase {
            Phase::Running => {
                *phase = Phase::Committed;
                committed = true;
                true
            }
            Phase::Committed => {
                committed = true;
                false
            }
            Phase::Cancelling => false,
        });
        committed
    }

    fn is_committed(&self) -> bool {
        *self.phase.borrow() == Phase::Committed
    }

    /// Resolves once [`Self::cancel`] took effect.
    async fn cancelled(&self) {
        let mut rx = self.phase.subscribe();
        if rx.wait_for(|p| *p == Phase::Cancelling).await.is_err() {
            // The sender lives in `self`, so this cannot happen.
            std::future::pending::<()>().await;
        }
    }

    /// `fut`'s output, or `None` as soon as the transfer is cancelled; then
    /// `fut` is dropped, which for a request or a response body closes the
    /// connection.
    pub async fn unless_cancelled<T>(&self, fut: impl Future<Output = T>) -> Option<T> {
        let mut fut = pin!(fut);
        let mut cancelled = pin!(self.cancelled());
        std::future::poll_fn(|cx| {
            if let Poll::Ready(v) = fut.as_mut().poll(cx) {
                return Poll::Ready(Some(v));
            }
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            Poll::Pending
        })
        .await
    }

    fn lock_partial(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        self.partial.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The temporary file of the running download, if there is one now.
    pub fn partial_file(&self) -> Option<PathBuf> {
        self.lock_partial().clone()
    }

    /// The multipart upload id of the running upload while it is neither
    /// completed nor aborted (also after an abort that failed).
    pub fn open_multipart(&self) -> Option<String> {
        self.multipart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_multipart(&self, upload_id: Option<String>) {
        *self.multipart.lock().unwrap_or_else(|e| e.into_inner()) = upload_id;
    }

    /// Makes the next download fail like a full disk once it has written
    /// `bytes` bytes.
    #[cfg(test)]
    pub(crate) fn fail_writes_after(&self, bytes: u64) {
        *self.fail_write_after.lock().unwrap() = Some(bytes);
    }

    /// Runs `f` after verification, right before the commit point, so a test
    /// can cancel exactly there.
    #[cfg(test)]
    pub(crate) fn before_commit(&self, f: impl FnOnce() + Send + 'static) {
        *self.before_commit.lock().unwrap() = Some(Box::new(f));
    }

    /// Runs `f` right after the commit point, before the file is moved (an
    /// upload: before the provider is asked to store the object).
    #[cfg(test)]
    pub(crate) fn after_commit(&self, f: impl FnOnce() + Send + 'static) {
        *self.after_commit.lock().unwrap() = Some(Box::new(f));
    }

    /// Runs `f` once an upload's first hashing pass has read its first
    /// chunk.
    #[cfg(test)]
    pub(crate) fn during_hashing(&self, f: impl FnOnce() + Send + 'static) {
        *self.during_hashing.lock().unwrap() = Some(Box::new(f));
    }

    /// Runs `f` once part `part` of an upload (1 for a single PUT) has handed
    /// its first chunk to the connection.
    #[cfg(test)]
    pub(crate) fn during_send(&self, part: u32, f: impl FnOnce() + Send + 'static) {
        *self.during_send.lock().unwrap() = Some((part, Box::new(f)));
    }

    #[cfg(test)]
    fn run_hook(hook: &Mutex<Option<Box<dyn FnOnce() + Send>>>) {
        let f = hook.lock().unwrap().take();
        if let Some(f) = f {
            f();
        }
    }
}

/// Removes the temporary file and forgets it.
async fn discard_partial(control: &TransferControl, partial: &Path) {
    if let Err(e) = tokio::fs::remove_file(partial).await
        && e.kind() != io::ErrorKind::NotFound
    {
        eprintln!(
            "sotyware: cannot remove the temporary download file {}: {e}",
            partial.display()
        );
    }
    control.lock_partial().take();
}

/// Moves the verified temporary file to `dest`.
async fn finalize(partial: &Path, dest: &Path, existing: Existing) -> Result<(), StorageError> {
    match existing {
        // One rename replaces the file atomically on Linux and Windows: the
        // destination is either the old file or the complete new one.
        Existing::Replace => tokio::fs::rename(partial, dest)
            .await
            .map_err(write_error(dest)),
        // A hard link never replaces an existing file: if one appeared while
        // the download ran, it stays untouched.
        Existing::Refuse => match tokio::fs::hard_link(partial, dest).await {
            Ok(()) => {
                if let Err(e) = tokio::fs::remove_file(partial).await {
                    eprintln!(
                        "sotyware: cannot remove the temporary download file {}: {e}",
                        partial.display()
                    );
                }
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(write_error(dest)(e)),
            // File systems without hard links (FAT, exFAT, some network
            // shares): check, then rename. A file created in between would
            // be replaced; the window is this one call.
            Err(_) => {
                if tokio::fs::try_exists(dest).await.unwrap_or(true) {
                    return Err(StorageError::LocalWrite {
                        kind: io::ErrorKind::AlreadyExists,
                        message: format!("{} already exists", dest.display()),
                    });
                }
                tokio::fs::rename(partial, dest)
                    .await
                    .map_err(write_error(dest))
            }
        },
    }
}

/// One account's connection to its provider.
pub struct S3Store {
    client: Client,
    account: ResolvedAccount,
    access_key_id: String,
    options: ClientOptions,
    transfers: Semaphore,
}

impl fmt::Debug for S3Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Store")
            .field("account", &self.account.id)
            .field("endpoint", &self.account.endpoint_url)
            .finish_non_exhaustive()
    }
}

impl S3Store {
    /// Builds the client. Sends no request.
    pub fn new(account: ResolvedAccount, credentials: Credentials, options: ClientOptions) -> Self {
        let config = Self::config(&account, &credentials, &options);
        // A recorder build with recording switched on (ADR-024): one
        // read-only interceptor; it changes nothing the client does.
        #[cfg(feature = "request-recorder")]
        let config = match super::recorder::installed() {
            Some(rec) => config.interceptor(rec.interceptor_for(&account)),
            None => config,
        };
        Self::from_config(config, account, credentials, options)
    }

    /// Like [`Self::new`], recorded by `rec` instead of the process's
    /// recorder (the recorder's own tests).
    #[cfg(all(test, feature = "request-recorder"))]
    pub(crate) fn new_recorded(
        account: ResolvedAccount,
        credentials: Credentials,
        options: ClientOptions,
        rec: &std::sync::Arc<super::recorder::Recorder>,
    ) -> Self {
        let config = Self::config(&account, &credentials, &options)
            .interceptor(rec.interceptor_for(&account));
        Self::from_config(config, account, credentials, options)
    }

    fn from_config(
        config: aws_sdk_s3::config::Builder,
        account: ResolvedAccount,
        credentials: Credentials,
        options: ClientOptions,
    ) -> Self {
        Self {
            client: Client::from_conf(config.build()),
            transfers: Semaphore::new(usize::from(account.max_concurrent_transfers)),
            access_key_id: credentials.access_key_id,
            account,
            options,
        }
    }

    /// The client settings of an account.
    fn config(
        account: &ResolvedAccount,
        credentials: &Credentials,
        options: &ClientOptions,
    ) -> aws_sdk_s3::config::Builder {
        let checksums = account.profile.sdk_checksums;
        let (request_checksums, response_checksums) = match checksums {
            SdkChecksums::WhenSupported => (
                RequestChecksumCalculation::WhenSupported,
                ResponseChecksumValidation::WhenSupported,
            ),
            SdkChecksums::WhenRequired => (
                RequestChecksumCalculation::WhenRequired,
                ResponseChecksumValidation::WhenRequired,
            ),
        };
        // rustls with ring, and the operating system's root certificates.
        let http = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .http_client(http)
            .region(Region::new(account.region.clone()))
            .endpoint_url(&account.endpoint_url)
            .force_path_style(account.path_style)
            .credentials_provider(SdkCredentials::new(
                credentials.access_key_id.clone(),
                credentials.secret_access_key.expose(),
                None,
                None,
                "sotyware",
            ))
            .request_checksum_calculation(request_checksums)
            .response_checksum_validation(response_checksums)
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(options.connect_timeout)
                    .read_timeout(options.read_timeout)
                    .build(),
            )
            .retry_config(RetryConfig::standard().with_max_attempts(options.max_attempts))
    }

    pub fn account_id(&self) -> &str {
        &self.account.id
    }

    /// The key id in use (not secret), so callers can show which key it is.
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// The account's transfer queue, for tests.
    #[cfg(test)]
    pub(crate) fn transfer_queue(&self) -> &Semaphore {
        &self.transfers
    }

    fn check_target(&self, bucket: &str, key: Option<&str>) -> Result<(), StorageError> {
        if !self.account.buckets.is_empty() && !self.account.buckets.iter().any(|b| b == bucket) {
            return Err(StorageError::Refused(format!(
                "bucket '{bucket}' is not configured for account '{}'",
                self.account.id
            )));
        }
        if let Some(key) = key
            && (key.is_empty() || key.len() > MAX_KEY_BYTES)
        {
            return Err(StorageError::Refused(format!(
                "object keys must be 1 to {MAX_KEY_BYTES} bytes"
            )));
        }
        Ok(())
    }

    async fn transfer_slot(&self) -> Result<tokio::sync::SemaphorePermit<'_>, StorageError> {
        // The semaphore is never closed.
        self.transfers
            .acquire()
            .await
            .map_err(|e| StorageError::Io(format!("transfer queue closed: {e}")))
    }

    /// The SDK client, so the opt-in integration tests can check what the
    /// provider really stored (object versions, delete markers) without the
    /// adapter offering those calls to the app.
    #[cfg(test)]
    pub(super) fn sdk_client(&self) -> &Client {
        &self.client
    }

    /// The same store with `interceptor` added to its client, so a test can
    /// record the HTTP requests really sent (method, path, status), also to
    /// a real provider, where the server side cannot be observed.
    #[cfg(test)]
    pub(super) fn with_interceptor(
        mut self,
        interceptor: impl aws_sdk_s3::config::Intercept + 'static,
    ) -> Self {
        let config = self
            .client
            .config()
            .to_builder()
            .interceptor(interceptor)
            .build();
        self.client = Client::from_conf(config);
        self
    }

    /// Every bucket the credentials can see. A key restricted to one bucket
    /// may be refused this (`AccessDenied`); use the configured buckets then.
    pub async fn list_buckets(&self) -> Result<Vec<String>, StorageError> {
        let out = self
            .client
            .list_buckets()
            .send()
            .await
            .map_err(|e| classify("list buckets", e))?;
        Ok(out
            .buckets()
            .iter()
            .filter_map(|b| b.name().map(str::to_owned))
            .collect())
    }

    /// Checks that the credentials are accepted and may use `bucket`, without
    /// changing anything and without listing the account's buckets (a key
    /// restricted to one bucket may be refused `ListBuckets` while being
    /// perfectly valid for its bucket).
    ///
    /// First `HeadBucket` on the bucket: success proves both. A HEAD response
    /// has no body, so a refusal there is only a status (403 is the same for
    /// a wrong secret and a missing permission). Only then one
    /// `ListObjectsV2` of at most one key in the same bucket is sent, whose
    /// error body carries a code that tells the two apart: success means the
    /// key may read the bucket, `AccessDenied` that the key is valid but not
    /// allowed, `InvalidAccessKeyId`/`SignatureDoesNotMatch` that it is wrong.
    pub async fn check_bucket(&self, bucket: &str) -> Result<(), StorageError> {
        self.check_target(bucket, None)?;
        let head = self
            .client
            .head_bucket()
            .bucket(bucket)
            .send()
            .await
            .map(drop)
            .map_err(|e| classify("check bucket", e));
        match head {
            Err(StorageError::AccessDenied(_))
            | Err(StorageError::Provider {
                status: 400..=499, ..
            }) => self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .max_keys(1)
                .send()
                .await
                .map(drop)
                .map_err(|e| classify("check bucket", e)),
            other => other,
        }
    }

    /// Every object under `prefix`, following continuation tokens.
    pub async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<Vec<ObjectInfo>, StorageError> {
        self.check_target(bucket, None)?;
        let mut objects = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let out = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .set_continuation_token(token.take())
                .send()
                .await
                .map_err(|e| classify("list objects", e))?;
            for o in out.contents() {
                if let Some(key) = o.key() {
                    objects.push(ObjectInfo {
                        key: key.to_owned(),
                        size: o.size().and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                        last_modified: o.last_modified().map(|t| t.to_string()),
                    });
                }
            }
            match out.next_continuation_token() {
                Some(next) if out.is_truncated() == Some(true) => token = Some(next.to_owned()),
                _ => break,
            }
        }
        Ok(objects)
    }

    /// One page of one folder level: the objects directly under `prefix` and
    /// the prefixes of its immediate subfolders (`ListObjectsV2` with the
    /// delimiter `/`), at most `max_keys` entries. Never recursive: nested
    /// objects appear only as their top folder's prefix.
    ///
    /// `continuation` is the token of the previous page, exactly as the
    /// provider returned it. The returned `next_token` is `None` when the
    /// provider says the listing is complete, and also when it claims more
    /// but gives no token, or gives back the token it was sent (which would
    /// repeat the same page forever).
    pub async fn list_folder(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: i32,
    ) -> Result<FolderPage, StorageError> {
        self.check_target(bucket, None)?;
        if prefix.len() > MAX_KEY_BYTES {
            return Err(StorageError::Refused(format!(
                "prefixes must be at most {MAX_KEY_BYTES} bytes"
            )));
        }
        let out = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .delimiter(FOLDER_DELIMITER)
            .max_keys(max_keys)
            .set_continuation_token(continuation.map(str::to_owned))
            .send()
            .await
            .map_err(|e| classify("list folder", e))?;
        let objects = out
            .contents()
            .iter()
            .filter_map(|o| {
                Some(ObjectInfo {
                    key: o.key()?.to_owned(),
                    size: o.size().and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                    last_modified: o
                        .last_modified()
                        .and_then(|t| t.fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime).ok()),
                })
            })
            .collect();
        let prefixes = out
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix().map(str::to_owned))
            .collect();
        let next_token = match out.next_continuation_token() {
            Some(next) if out.is_truncated() == Some(true) && Some(next) != continuation => {
                Some(next.to_owned())
            }
            _ => None,
        };
        Ok(FolderPage {
            prefixes,
            objects,
            next_token,
        })
    }

    /// The object's size and stored checksum, or `None` if it does not exist.
    pub async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectDetails>, StorageError> {
        self.check_target(bucket, Some(key))?;
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => Ok(Some(ObjectDetails {
                size: out
                    .content_length()
                    .and_then(|s| u64::try_from(s).ok())
                    .unwrap_or(0),
                sha256: out.metadata().and_then(|m| m.get(SHA256_METADATA)).cloned(),
                version_id: out.version_id().map(str::to_owned),
            })),
            Err(e) => match classify("head object", e) {
                StorageError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// Uploads `path` to `bucket/key`, which must not hold an object yet,
    /// without cancellation or progress: see [`Self::upload`]. Kept for the
    /// tests and the opt-in provider tests; only the error is returned, not
    /// what may be left at the provider.
    pub async fn upload_file(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
    ) -> Result<UploadReport, StorageError> {
        self.upload(
            bucket,
            key,
            path,
            &std::sync::Arc::new(TransferControl::new()),
            &mut |_| {},
        )
        .await
        .map_err(|e| e.error)
    }

    /// Downloads `bucket/key` to `dest`, which must not exist yet: see
    /// [`Self::download`].
    pub async fn download_file(
        &self,
        bucket: &str,
        key: &str,
        dest: &Path,
    ) -> Result<DownloadReport, StorageError> {
        self.download(
            bucket,
            key,
            dest,
            Existing::Refuse,
            &TransferControl::new(),
            &mut |_, _| {},
        )
        .await
    }

    /// Downloads `bucket/key` (the key exactly as given) to `dest`.
    ///
    /// The response body is streamed chunk by chunk into a new temporary
    /// file in `dest`'s folder, hashed on the way; nothing holds more than
    /// one chunk in memory. `progress` gets the bytes written so far and the
    /// size the provider announced (if it did), after every chunk. Once the
    /// body has ended, the file is synced to disk and its size (and, if the
    /// object has a SHA-256 stored by sotyware, its hash) checked; only then
    /// is it moved to `dest`, per `existing`. On any failure, and on
    /// cancellation through `control`, the temporary file is removed and
    /// `dest` is left as it was.
    pub async fn download(
        &self,
        bucket: &str,
        key: &str,
        dest: &Path,
        existing: Existing,
        control: &TransferControl,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<DownloadReport, StorageError> {
        self.check_target(bucket, Some(key))?;
        if existing == Existing::Refuse && tokio::fs::try_exists(dest).await.unwrap_or(true) {
            return Err(StorageError::LocalWrite {
                kind: io::ErrorKind::AlreadyExists,
                message: format!("{} already exists", dest.display()),
            });
        }
        let _slot = control
            .unless_cancelled(self.transfer_slot())
            .await
            .ok_or(StorageError::Cancelled)??;
        let (partial, file) = create_partial(dest).await?;
        *control.lock_partial() = Some(partial.clone());
        let mut result = self
            .stream_to(bucket, key, file, &partial, control, progress)
            .await;
        if result.is_ok() {
            #[cfg(test)]
            if let Some(f) = control.before_commit.lock().unwrap().take() {
                f();
            }
            if !control.commit() {
                result = Err(StorageError::Cancelled);
            }
            #[cfg(test)]
            if result.is_ok()
                && let Some(f) = control.after_commit.lock().unwrap().take()
            {
                f();
            }
        }
        let result = match result {
            Ok(report) => finalize(&partial, dest, existing).await.map(|()| report),
            Err(e) => Err(e),
        };
        if result.is_err() {
            discard_partial(control, &partial).await;
        } else {
            control.lock_partial().take();
        }
        result
    }

    async fn stream_to(
        &self,
        bucket: &str,
        key: &str,
        mut file: tokio::fs::File,
        partial: &Path,
        control: &TransferControl,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<DownloadReport, StorageError> {
        let out = control
            .unless_cancelled(self.client.get_object().bucket(bucket).key(key).send())
            .await
            .ok_or(StorageError::Cancelled)?
            .map_err(|e| classify("download", e))?;
        let expected_sha = out.metadata().and_then(|m| m.get(SHA256_METADATA)).cloned();
        let expected_size = out.content_length().and_then(|s| u64::try_from(s).ok());
        let mut body = out.body;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        progress(0, expected_size);
        #[cfg(test)]
        let fail_after = *control.fail_write_after.lock().unwrap();
        loop {
            let chunk = control
                .unless_cancelled(body.try_next())
                .await
                .ok_or(StorageError::Cancelled)?
                .map_err(|e| {
                    StorageError::Network(format!("download: {}", DisplayErrorContext(&e)))
                })?;
            let Some(chunk) = chunk else {
                break;
            };
            #[cfg(test)]
            if fail_after.is_some_and(|limit| size + chunk.len() as u64 > limit) {
                return Err(write_error(partial)(io::Error::from(
                    io::ErrorKind::StorageFull,
                )));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(write_error(partial))?;
            size += chunk.len() as u64;
            progress(size, expected_size);
            if control.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
        }
        file.flush().await.map_err(write_error(partial))?;
        file.sync_all().await.map_err(write_error(partial))?;
        drop(file);
        let sha256 = hex(&hasher.finalize());
        if let Some(expected) = expected_size.filter(|&e| e != size) {
            return Err(StorageError::Integrity(format!(
                "{key}: received {size} bytes, expected {expected}"
            )));
        }
        let verification = match expected_sha {
            Some(expected) if expected.eq_ignore_ascii_case(&sha256) => Verification::Sha256Matched,
            Some(expected) => {
                return Err(StorageError::Integrity(format!(
                    "{key}: SHA-256 {sha256}, expected {expected}"
                )));
            }
            None => Verification::SizeOnly,
        };
        Ok(DownloadReport {
            size,
            sha256,
            verification,
        })
    }

    /// Deletes `bucket/key`. With a `version_id`, that exact version is
    /// removed for good; without one, a versioned bucket only hides the object
    /// (the data stays and is still billed).
    pub async fn delete_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> Result<(), StorageError> {
        self.check_target(bucket, Some(key))?;
        self.client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await
            .map(drop)
            .map_err(|e| classify("delete", e))
    }
}

#[cfg(test)]
mod tests {
    use super::super::provider::{BACKBLAZE_B2, HETZNER, S3_COMPATIBLE};
    use super::*;

    const GIB: u64 = 1024 * MIB;

    #[test]
    fn small_files_use_one_put() {
        let o = ClientOptions::default();
        for size in [0, 1, 64 * MIB] {
            assert_eq!(
                plan_upload(size, &BACKBLAZE_B2.limits, &o),
                Ok(UploadPlan::Single),
                "{size}"
            );
        }
    }

    #[test]
    fn large_files_use_multipart_with_the_preferred_part_size() {
        let o = ClientOptions::default();
        assert_eq!(
            plan_upload(64 * MIB + 1, &HETZNER.limits, &o),
            Ok(UploadPlan::Multipart {
                part_size: 16 * MIB,
                parts: 5
            })
        );
    }

    #[test]
    fn a_multipart_upload_never_has_a_single_part() {
        let o = ClientOptions {
            multipart_threshold: 6 * MIB,
            part_size: 16 * MIB,
            ..ClientOptions::default()
        };
        // Above the threshold but within one part: one PUT instead.
        assert_eq!(
            plan_upload(10 * MIB, &S3_COMPATIBLE.limits, &o),
            Ok(UploadPlan::Single)
        );
    }

    #[test]
    fn the_part_size_never_drops_below_the_provider_minimum() {
        let o = ClientOptions {
            multipart_threshold: MIB,
            part_size: MIB,
            ..ClientOptions::default()
        };
        // Backblaze's minimum is 5 MB (decimal); parts are whole MiB above it.
        assert_eq!(
            plan_upload(20 * MIB, &BACKBLAZE_B2.limits, &o),
            Ok(UploadPlan::Multipart {
                part_size: 5 * MIB,
                parts: 4
            })
        );
    }

    #[test]
    fn very_large_files_get_bigger_parts_to_stay_within_max_parts() {
        let o = ClientOptions::default();
        let size = 1024 * GIB; // 1 TiB: 16 MiB parts would need 65,536.
        let Ok(UploadPlan::Multipart { part_size, parts }) = plan_upload(size, &HETZNER.limits, &o)
        else {
            panic!("expected multipart");
        };
        assert!(parts <= HETZNER.limits.max_parts);
        assert!(part_size <= HETZNER.limits.max_part_size);
        assert_eq!(part_size % MIB, 0);
        assert!(part_size * parts >= size);
    }

    #[test]
    fn files_beyond_the_provider_limit_are_refused_before_any_request() {
        let o = ClientOptions::default();
        let too_big = HETZNER.limits.max_object_size + 1;
        assert_eq!(
            plan_upload(too_big, &HETZNER.limits, &o),
            Err(StorageError::TooLarge {
                size: too_big,
                max: HETZNER.limits.max_object_size
            })
        );
        // A threshold above the single-PUT limit is capped by it.
        let o = ClientOptions {
            multipart_threshold: u64::MAX,
            ..ClientOptions::default()
        };
        assert!(matches!(
            plan_upload(6 * GIB, &S3_COMPATIBLE.limits, &o),
            Ok(UploadPlan::Multipart { .. })
        ));
    }

    #[test]
    fn provider_errors_are_classified_by_code_then_status() {
        use StorageError::*;
        let c = |status, code| classify_response(status, code, "m", "op");
        assert!(matches!(
            c(403, Some("InvalidAccessKeyId")),
            InvalidCredentials(_)
        ));
        assert!(matches!(
            c(403, Some("SignatureDoesNotMatch")),
            InvalidCredentials(_)
        ));
        assert!(matches!(
            c(401, Some("InvalidToken")),
            InvalidCredentials(_)
        ));
        assert!(matches!(c(403, Some("NotSignedUp")), InvalidCredentials(_)));
        assert!(matches!(c(403, Some("AccessDenied")), AccessDenied(_)));
        assert!(matches!(c(404, Some("NoSuchBucket")), NotFound(_)));
        assert!(matches!(c(404, Some("NoSuchKey")), NotFound(_)));
        assert!(matches!(c(404, None), NotFound(_)));
        assert!(matches!(c(403, None), AccessDenied(_)));
        assert!(matches!(c(400, Some("RequestTimeout")), Timeout(_)));
        let e = c(503, Some("SlowDown"));
        assert!(matches!(e, Provider { status: 503, .. }));
        assert!(e.is_transient());
        assert!(!c(403, Some("AccessDenied")).is_transient());
        assert!(!c(400, Some("InvalidArgument")).is_transient());
    }

    #[test]
    fn hex_and_partial_names() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        let pid = std::process::id();
        assert_eq!(
            partial_name(Path::new("dir/photo.jpg"), 7),
            format!(".photo.jpg.{pid}-7.sotyware-partial")
        );
        // A long name is cut on a character boundary, never inside one.
        let long = format!("{}.bin", "ü".repeat(200));
        let name = partial_name(Path::new(&long), 1);
        assert!(name.len() <= 1 + PARTIAL_NAME_BYTES + 40, "{name}");
        assert!(name.starts_with(&format!(".{}", "ü".repeat(50))));
    }

    #[test]
    fn cancelling_and_committing_exclude_each_other() {
        let c = TransferControl::new();
        assert!(!c.is_cancelled());
        assert!(c.cancel());
        assert!(c.cancel(), "cancelling twice is the same as once");
        assert!(c.is_cancelled());
        assert!(!c.commit(), "a cancelled transfer never commits");

        let c = TransferControl::new();
        assert!(c.commit());
        assert!(c.is_committed());
        assert!(
            c.commit(),
            "committing again (an SDK retry) stays committed"
        );
        assert!(!c.cancel(), "too late once committed");
        assert!(!c.is_cancelled());
        assert!(!c.cancel());
    }

    #[tokio::test]
    async fn unless_cancelled_stops_waiting_when_cancelled() {
        let c = std::sync::Arc::new(TransferControl::new());
        assert_eq!(c.unless_cancelled(async { 5 }).await, Some(5));
        let c2 = std::sync::Arc::clone(&c);
        let waiter =
            tokio::spawn(async move { c2.unless_cancelled(std::future::pending::<()>()).await });
        tokio::task::yield_now().await;
        c.cancel();
        assert_eq!(waiter.await.unwrap(), None);
        // Already cancelled: returns at once.
        assert_eq!(c.unless_cancelled(std::future::pending::<()>()).await, None);
    }
}

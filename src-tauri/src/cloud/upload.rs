//! Uploading a batch of files into one folder of an account's configured
//! bucket (ADR-022): the `upload_cloud_files`, `cancel_cloud_upload` and
//! `retry_cloud_upload` commands, the events they send, and the one upload
//! slot behind them. Each file goes through
//! [`S3Store::upload`](super::s3::S3Store::upload) (ADR-021), which owns
//! hashing, cancellation up to the commit point, multipart, stall timeouts
//! and verification; this module only orders files and reports them.
//!
//! - **Where from:** files the user picked in the native file dialog, which
//!   Rust opens itself. Their paths stay in Rust: the page gets file names,
//!   object keys, sizes and states, never a local path (also not inside an
//!   error message).
//! - **Where to:** `prefix` + file name in the account's configured bucket.
//!   The page sends the folder it is showing when it asks for the dialog;
//!   that folder is the batch's destination whatever the page does later.
//! - **Selection:** 1 to [`MAX_FILES`] files. More are refused as a whole
//!   (nothing starts). Every file is checked before anything is sent
//!   ([`check_selection`]): a regular file, a UTF-8 name without control
//!   characters, a key of at most 1,024 bytes, a size the provider accepts,
//!   and no other selected file with the same key. A file that fails is
//!   skipped with a [`SkipReason`]; the others go ahead. Nothing is hashed
//!   here. If no file is left, no batch starts.
//! - **One file after another**, one batch at a time ([`ErrorKind::Busy`]
//!   otherwise), in a slot of its own: a download can run meanwhile. Every
//!   upload still waits for the account's transfer queue.
//! - **Never overwrite:** right before each file a `HeadObject` on its key;
//!   an existing object skips the file ([`SkipReason::DestinationExists`])
//!   before it is read. `S3Store::upload` checks again (and, for multipart,
//!   once more before completing); a conflict found there skips the file
//!   too.
//! - **Errors** ([`disposition`]): a problem of one file (it changed, cannot
//!   be read, failed verification, a non-transient provider refusal) fails
//!   that file and the batch goes on; one that would fail the next files too
//!   (credentials, access, configuration, bucket, connection, timeouts,
//!   provider 5xx/429) fails the file and stops the batch.
//! - **Events:** [`UploadEvent`]s on the Tauri channel the page passed with
//!   the command, numbered by `seq` within the batch: a `file` event when a
//!   file changes state or (throttled by `S3Store::upload`, at most one per
//!   250 ms within a stage, first and last of each stage always) makes
//!   progress, `cleanup` events on a retry, and exactly one `finished` event
//!   with the [`BatchSummary`], sent after the slot is free again.
//! - **Cancel** ([`CloudUploads::cancel`]) cancels the file being uploaded
//!   (up to its commit point) and keeps the remaining files from starting.
//!   Idempotent; it never affects another batch id.
//! - **Retry** ([`CloudUploads::retry`]) starts a new batch (new id) with
//!   the last finished batch's files that failed with nothing stored, or were
//!   cancelled or never started, checked again like a new selection; the
//!   destination is checked again before each. Completed and skipped files,
//!   and files whose remote outcome is "needs inspection" or "incomplete
//!   parts", are never retried.
//! - **Multipart cleanup:** an upload whose abort failed leaves an
//!   unresolved upload id; it is kept in memory (account, bucket, key, id)
//!   while sotyware runs, aborted again (best effort) when a batch of that
//!   account is retried and when sotyware quits, and forgotten only once the
//!   provider confirms it is gone. Nothing is stored on disk; after a
//!   restart the parts can only be removed by a bucket lifecycle rule.
//! - **Shutdown** ([`CloudUploads::shutdown`]) cancels the running batch,
//!   aborts every unresolved upload id, and waits a bounded time for both.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::download::CancelOutcome;
use super::provider::TransferLimits;
use super::s3::{
    AbortOutcome, ClientOptions, RemoteOutcome, S3Store, StorageError, TransferControl,
    UploadProgress, UploadStage, plan_upload,
};
use crate::journal::Journal;

/// Most files one batch of picked files may hold.
pub const MAX_FILES: usize = 100;

/// Most files a folder upload may hold (every file under the picked
/// folder, at any depth).
pub const MAX_FOLDER_FILES: usize = 1_000;

/// How long quitting waits for a cancelled batch and for multipart aborts:
/// the default abort timeout (30 s, `ClientOptions::abort_timeout`) plus
/// the time a download is given to clean up. Only reached when the provider
/// does not answer an abort.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(35);

/// Longest object key S3 allows, in bytes.
const MAX_KEY_BYTES: usize = 1024;

/// Where a batch stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchState {
    Active,
    /// Every file reached a final state (completed, skipped or failed).
    Completed,
    /// Stopped by the user, by leaving the browser or by quitting.
    Cancelled,
    /// Stopped by an error that would have failed the remaining files too
    /// ([`BatchSummary::error`]).
    Failed,
}

/// Where one file of a batch stands. `hashing`, `sending` and `verifying`
/// are [`UploadStage`]s; the last four are final.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileState {
    /// Waiting for its turn.
    Pending,
    Hashing,
    Sending,
    Verifying,
    /// Stored and verified.
    Completed,
    /// Not uploaded, by design ([`UploadFile::skip`]).
    Skipped,
    /// Stopped by an error ([`UploadFile::error`], [`UploadFile::remote`]).
    Failed,
    /// Cancelled while it ran, or never started because the batch was
    /// cancelled or stopped ([`UploadFile::started`] tells which).
    Cancelled,
}

impl FileState {
    pub fn is_final(self) -> bool {
        self >= Self::Completed
    }
}

/// The stage a file reached, for the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Hashing,
    Sending,
    Verifying,
}

impl From<UploadStage> for Stage {
    fn from(stage: UploadStage) -> Self {
        match stage {
            UploadStage::Hashing => Self::Hashing,
            UploadStage::Sending => Self::Sending,
            UploadStage::Verifying => Self::Verifying,
        }
    }
}

impl From<Stage> for FileState {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Hashing => Self::Hashing,
            Stage::Sending => Self::Sending,
            Stage::Verifying => Self::Verifying,
        }
    }
}

/// Why a file is not uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    /// A folder, device or other non-file (or a link to one).
    NotAFile,
    /// Its size could not be read.
    Unreadable,
    /// Its name is not valid UTF-8, so it cannot become an object key.
    InvalidName,
    /// Its name contains control characters.
    ControlCharacters,
    /// Folder plus name exceed the 1,024 bytes of an object key.
    KeyTooLong,
    /// Larger than the provider accepts.
    TooLarge,
    /// Another selected file has the same name, so the same key; none of
    /// them is uploaded.
    DuplicateKey,
    /// The bucket already holds an object with this key; it is never
    /// overwritten.
    DestinationExists,
}

/// A skipped file's reason, with a sentence for the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Skip {
    pub reason: SkipReason,
    pub message: String,
}

impl Skip {
    fn new(reason: SkipReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

/// What a failed or cancelled upload may have left at the provider (see
/// [`RemoteOutcome`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteState {
    /// Nothing: no object, no multipart upload left.
    NothingStored,
    /// A multipart upload could not be aborted; its parts may remain (and be
    /// billed). sotyware tries again on a retry of this account's batch and
    /// when quitting.
    IncompleteParts,
    /// The provider may have stored the object, but it could not be
    /// verified. Check it in the bucket; it is not retried or deleted.
    NeedsInspection,
}

impl From<&RemoteOutcome> for RemoteState {
    fn from(outcome: &RemoteOutcome) -> Self {
        match outcome {
            RemoteOutcome::NothingStored => Self::NothingStored,
            RemoteOutcome::IncompleteParts { .. } => Self::IncompleteParts,
            RemoteOutcome::NeedsInspection => Self::NeedsInspection,
        }
    }
}

/// One file of a batch, as the page sees it. Never carries a local path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFile {
    /// Position in this batch, from 0.
    pub index: u32,
    /// The file's name (not its folder); not valid UTF-8 is shown lossily.
    pub name: String,
    /// The object key, `None` if the name cannot become one.
    pub key: Option<String>,
    /// The size found when the file was picked, if it could be read.
    pub size: Option<u64>,
    pub state: FileState,
    /// The last stage the upload reached.
    pub stage: Option<Stage>,
    /// Hashed (`hashing`), sent (`sending`) or verified (`verifying`,
    /// `completed`) bytes; never decreases within a stage.
    pub bytes: u64,
    pub total: u64,
    /// Whether its upload began. True from the moment the batch takes the
    /// file, so also while its destination is checked (no event is sent
    /// then: a file's first event is its first progress or its final
    /// state); false for skipped files (also those skipped because the key
    /// turned out to exist, whenever that was found) and files the batch
    /// never reached.
    pub started: bool,
    /// Only for "skipped".
    pub skip: Option<Skip>,
    /// Only for "failed".
    pub error: Option<CommandError>,
    /// For "failed" and "cancelled".
    pub remote: Option<RemoteState>,
    /// Whether `retry_cloud_upload` would upload it again.
    pub retryable: bool,
}

/// How a batch ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub state: BatchState,
    pub count: u32,
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    /// Cancelled while running or never started.
    pub cancelled: u32,
    /// Of those, never started.
    pub not_started: u32,
    pub retryable: u32,
    /// Files whose remote outcome needs inspection or cleanup.
    pub needs_attention: u32,
    /// The error that stopped the batch (only for "failed").
    pub error: Option<CommandError>,
    /// Every file in its final state.
    pub files: Vec<UploadFile>,
}

/// How a cleanup attempt went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupOutcome {
    /// The incomplete multipart upload is gone at the provider.
    Removed,
    /// The abort failed again; the parts may remain.
    Failed,
}

/// What an event reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum EventBody {
    /// A file changed state or made progress.
    File { count: u32, file: UploadFile },
    /// An incomplete multipart upload of an earlier batch was aborted
    /// again (on a retry).
    Cleanup {
        key: String,
        outcome: CleanupOutcome,
    },
    /// The batch ended; the last event of the batch, sent once.
    Finished { summary: BatchSummary },
}

/// One event of a batch. Never carries a credential, provider text or a
/// local path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadEvent {
    pub batch: String,
    /// 1, 2, 3, … within the batch, so a page can drop anything out of order.
    pub seq: u64,
    #[serde(flatten)]
    pub body: EventBody,
}

/// What `upload_cloud_files` and `retry_cloud_upload` return.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSelection {
    /// The new batch's id; `None` if no file can be uploaded (nothing
    /// started).
    pub batch: Option<String>,
    /// The destination folder ("" for the bucket's top level).
    pub prefix: String,
    /// Every selected file: pending, or skipped with its reason.
    pub files: Vec<UploadFile>,
}

/// Where events go: the page's channel, or a test recorder.
pub trait UploadSink: Send + Sync + 'static {
    fn emit(&self, event: UploadEvent);
}

impl UploadSink for Channel<UploadEvent> {
    fn emit(&self, event: UploadEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: cannot report upload progress: {e}");
        }
    }
}

/// What happens to a batch after a file's upload failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The destination exists: the file is skipped, the batch goes on.
    Skip,
    /// Only this file failed; the batch goes on.
    FileFailed,
    /// The next files would fail too: the batch stops.
    StopBatch,
    /// Cancelled: the batch stops.
    Cancelled,
}

/// Sorts an upload error by its effect on the rest of the batch.
pub fn disposition(error: &StorageError) -> Disposition {
    match error {
        StorageError::Cancelled => Disposition::Cancelled,
        StorageError::AlreadyExists(_) => Disposition::Skip,
        StorageError::InvalidCredentials(_)
        | StorageError::AccessDenied(_)
        | StorageError::NotFound(_)
        | StorageError::Network(_)
        | StorageError::Timeout(_)
        | StorageError::Refused(_)
        | StorageError::Journal(_) => Disposition::StopBatch,
        StorageError::Provider { .. } if error.is_transient() => Disposition::StopBatch,
        StorageError::Provider { .. }
        | StorageError::Integrity(_)
        | StorageError::FileChanged(_)
        | StorageError::TooLarge { .. }
        | StorageError::Io(_)
        | StorageError::LocalWrite { .. } => Disposition::FileFailed,
    }
}

/// How a failed upload is shown. Sentences are sotyware's own: the
/// provider's text and local error text (which names the local path) are
/// only logged.
pub fn upload_error(error: &StorageError) -> CommandError {
    let (kind, message) = match error {
        StorageError::InvalidCredentials(_) => (
            ErrorKind::InvalidCredentials,
            "the provider does not accept this key ID or secret".to_owned(),
        ),
        StorageError::AccessDenied(_) => (
            ErrorKind::AccessDenied,
            "the provider refused to store files with this key".to_owned(),
        ),
        StorageError::NotFound(_) => (
            ErrorKind::BucketUnavailable,
            "the bucket was not found, or this key may not use it".to_owned(),
        ),
        StorageError::Network(_) => (
            ErrorKind::Network,
            "the connection to the provider failed or broke off".to_owned(),
        ),
        StorageError::Timeout(_) => (
            ErrorKind::Timeout,
            "the provider stopped taking data or did not answer in time".to_owned(),
        ),
        StorageError::Integrity(_) => (
            ErrorKind::Integrity,
            "the stored object does not match the file's size or SHA-256 checksum".to_owned(),
        ),
        StorageError::FileChanged(_) => (
            ErrorKind::FileChanged,
            "the file changed while it was being uploaded; try again once it is no longer being written"
                .to_owned(),
        ),
        StorageError::Io(_) | StorageError::LocalWrite { .. } => (
            ErrorKind::LocalFile,
            "the file could not be read".to_owned(),
        ),
        StorageError::TooLarge { max, .. } => (
            ErrorKind::InvalidInput,
            format!("the file is larger than the provider accepts ({max} bytes)"),
        ),
        StorageError::Refused(m) => (ErrorKind::InvalidConfiguration, m.clone()),
        StorageError::Journal(_) => (
            ErrorKind::Journal,
            "the upload was not recorded in the operation journal, so it was not started"
                .to_owned(),
        ),
        StorageError::Provider { status, code, .. } => {
            let code = if !code.is_empty()
                && code.len() <= 64
                && code.chars().all(|c| c.is_ascii_alphanumeric())
            {
                format!(" ({code})")
            } else {
                String::new()
            };
            let message = if *status == 0 {
                "the provider's answer could not be understood".to_owned()
            } else {
                format!("the provider refused the upload: HTTP {status}{code}")
            };
            (ErrorKind::Provider, message)
        }
        // Handled before: a cancel, and a conflict (skipped).
        StorageError::Cancelled | StorageError::AlreadyExists(_) => (
            ErrorKind::Other,
            "the file could not be uploaded".to_owned(),
        ),
    };
    CommandError::new(kind, message)
}

/// Refuses a destination folder no listing returns: "" (top level) or a
/// prefix ending in `/`, shorter than an object key may be.
pub fn validate_prefix(prefix: &str) -> Result<(), CommandError> {
    if !prefix.is_empty() && (!prefix.ends_with('/') || prefix.len() >= MAX_KEY_BYTES) {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "the destination folder is not valid",
        ));
    }
    Ok(())
}

/// A picked file after the checks: its path stays here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub key: Option<String>,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub skip: Option<Skip>,
}

/// Checks every picked file without reading its content (see the module
/// documentation). Keeps the order; never fails as a whole.
pub fn check_selection(
    prefix: &str,
    paths: Vec<PathBuf>,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Vec<Candidate> {
    check_named_selection(
        prefix,
        paths.into_iter().map(|p| (p, None)).collect(),
        limits,
        options,
    )
}

/// [`check_selection`] for files whose key below `prefix` is given
/// (`Some`, with `/` between folders: a folder upload, or a retry) instead
/// of being the file's own name.
pub fn check_named_selection(
    prefix: &str,
    files: Vec<(PathBuf, Option<String>)>,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = files
        .into_iter()
        .map(|(path, name)| check_file(prefix, path, name.as_deref(), limits, options))
        .collect();
    mark_duplicates(&mut candidates);
    candidates
}

/// A file found under a folder picked for upload: its key below the
/// destination, or (`Err`) the name shown when a folder or file name on
/// the way is not valid Unicode (the file is skipped).
pub type FolderFile = (PathBuf, Result<String, String>);

/// Every regular file under `root`, at any depth, with its key below the
/// destination: `<root's name>/<folders>/<file>`, `/` between them, sorted.
/// Links are not followed (neither to files nor to folders), so a link
/// cannot pull in content from elsewhere or loop. Empty folders add
/// nothing (S3 folders exist only through their objects). Refused as a
/// whole above `max` files.
pub fn folder_files(root: &std::path::Path, max: usize) -> Result<Vec<FolderFile>, CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m.to_owned());
    let name = root
        .file_name()
        .ok_or_else(|| invalid("this folder cannot be uploaded"))?
        .to_str()
        .ok_or_else(|| {
            invalid("the folder's name is not valid Unicode, so it cannot be part of an object key")
        })?
        .to_owned();
    if name.chars().any(char::is_control) {
        return Err(invalid("the folder's name contains control characters"));
    }
    let mut out = Vec::new();
    // (directory, its key below the destination, whether every name on the
    // way is valid Unicode)
    let mut stack = vec![(root.to_path_buf(), Ok::<String, String>(name))];
    while let Some((dir, rel)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            eprintln!(
                "sotyware: cannot read {} for a folder upload: {e}",
                dir.display()
            );
            CommandError::new(
                ErrorKind::LocalFile,
                "a folder inside could not be read; nothing was uploaded",
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                CommandError::new(
                    ErrorKind::LocalFile,
                    "a folder inside could not be read; nothing was uploaded",
                )
            })?;
            let kind = entry.file_type().map_err(|_| {
                CommandError::new(
                    ErrorKind::LocalFile,
                    "a file inside could not be read; nothing was uploaded",
                )
            })?;
            if kind.is_symlink() {
                continue;
            }
            let os = entry.file_name();
            let child = match (&rel, os.to_str()) {
                (Ok(r), Some(n)) => Ok(format!("{r}/{n}")),
                (Ok(r), None) | (Err(r), None) => Err(format!("{r}/{}", os.to_string_lossy())),
                (Err(r), Some(n)) => Err(format!("{r}/{n}")),
            };
            if kind.is_dir() {
                stack.push((entry.path(), child));
            } else if kind.is_file() {
                out.push((entry.path(), child));
                if out.len() > max {
                    return Err(invalid(&format!(
                        "the folder holds more than {max} files; upload it in parts"
                    )));
                }
            }
        }
    }
    out.sort_by(|a, b| {
        let key = |f: &FolderFile| match &f.1 {
            Ok(s) | Err(s) => s.clone(),
        };
        key(a).cmp(&key(b))
    });
    Ok(out)
}

/// [`check_selection`] for the files of a folder upload.
pub fn check_folder_selection(
    prefix: &str,
    files: Vec<FolderFile>,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = files
        .into_iter()
        .map(|(path, name)| match name {
            Ok(n) => check_file(prefix, path, Some(&n), limits, options),
            Err(shown) => Candidate {
                name: shown,
                key: None,
                path,
                size: None,
                skip: Some(Skip::new(
                    SkipReason::InvalidName,
                    "a folder or file name on its path is not valid Unicode, so it cannot be an object key",
                )),
            },
        })
        .collect();
    mark_duplicates(&mut candidates);
    candidates
}

/// Skips every file whose key another checked file has too.
fn mark_duplicates(candidates: &mut [Candidate]) {
    let mut keys: HashMap<String, usize> = HashMap::new();
    for c in candidates.iter().filter(|c| c.skip.is_none()) {
        if let Some(key) = &c.key {
            *keys.entry(key.clone()).or_default() += 1;
        }
    }
    for c in candidates.iter_mut() {
        if c.skip.is_none()
            && let Some(key) = &c.key
            && keys.get(key).copied().unwrap_or(0) > 1
        {
            c.skip = Some(Skip::new(
                SkipReason::DuplicateKey,
                "another selected file has the same name; none of them is uploaded",
            ));
        }
    }
}

fn check_file(
    prefix: &str,
    path: PathBuf,
    relative: Option<&str>,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Candidate {
    let raw_name = match relative {
        Some(r) => Some(std::ffi::OsString::from(r)),
        None => path.file_name().map(|n| n.to_owned()),
    };
    let name = raw_name
        .as_ref()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidate = Candidate {
        name,
        key: None,
        path,
        size: None,
        skip: None,
    };
    let skip = |c: &mut Candidate, reason, message: &str| {
        c.skip = Some(Skip::new(reason, message));
    };
    let Some(raw_name) = raw_name else {
        skip(&mut candidate, SkipReason::NotAFile, "this is not a file");
        return candidate;
    };
    let Some(utf8) = raw_name.to_str() else {
        skip(
            &mut candidate,
            SkipReason::InvalidName,
            "the file name is not valid Unicode, so it cannot be an object key",
        );
        return candidate;
    };
    if utf8.chars().any(char::is_control) {
        skip(
            &mut candidate,
            SkipReason::ControlCharacters,
            "the file name contains control characters",
        );
        return candidate;
    }
    let key = format!("{prefix}{utf8}");
    if key.len() > MAX_KEY_BYTES {
        skip(
            &mut candidate,
            SkipReason::KeyTooLong,
            "folder and file name together are longer than the 1,024 bytes an object key may have",
        );
        return candidate;
    }
    candidate.key = Some(key);
    // Follows links: a link to a regular file uploads that file, as
    // `S3Store::upload` opens it.
    match std::fs::metadata(&candidate.path) {
        Ok(meta) if !meta.is_file() => {
            skip(&mut candidate, SkipReason::NotAFile, "this is not a file");
        }
        Ok(meta) => {
            candidate.size = Some(meta.len());
            if let Err(StorageError::TooLarge { max, .. }) =
                plan_upload(meta.len(), limits, options)
            {
                candidate.skip = Some(Skip::new(
                    SkipReason::TooLarge,
                    format!("the file is larger than the provider accepts ({max} bytes)"),
                ));
            }
        }
        Err(e) => {
            eprintln!(
                "sotyware: cannot read {} for an upload: {e}",
                candidate.path.display()
            );
            skip(
                &mut candidate,
                SkipReason::Unreadable,
                "the file could not be read",
            );
        }
    }
    candidate
}

/// The batch's destination: where every file goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub account: String,
    pub bucket: String,
    pub prefix: String,
}

/// A file the batch will upload.
#[derive(Clone, Debug)]
struct Planned {
    key: String,
    path: PathBuf,
}

/// The files of a finished batch a retry would upload, and where.
struct RetryPlan {
    target: Target,
    /// Each file with its key below the target's prefix.
    files: Vec<(PathBuf, Option<String>)>,
}

/// A multipart upload whose abort failed. The store is the one the upload
/// used (same credentials), so no credential is read to abort it.
#[derive(Clone)]
struct Unresolved {
    store: Arc<S3Store>,
    account: String,
    bucket: String,
    key: String,
    upload_id: String,
}

/// Cancels a batch: the current file, and every file after it.
#[derive(Default)]
struct BatchControl {
    state: Mutex<BatchCtl>,
}

#[derive(Default)]
struct BatchCtl {
    cancelled: bool,
    current: Option<Arc<TransferControl>>,
}

impl BatchControl {
    fn lock(&self) -> MutexGuard<'_, BatchCtl> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cancel(&self) -> CancelOutcome {
        let mut state = self.lock();
        state.cancelled = true;
        match &state.current {
            Some(control) if !control.cancel() => CancelOutcome::Finishing,
            _ => CancelOutcome::Cancelling,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// A fresh control for the next step, or `None` once cancelled: checked
    /// and installed under one lock, so a cancel is never missed.
    fn begin(&self) -> Option<Arc<TransferControl>> {
        let mut state = self.lock();
        if state.cancelled {
            return None;
        }
        let control = Arc::new(TransferControl::new());
        state.current = Some(Arc::clone(&control));
        Some(control)
    }

    fn end(&self) {
        self.lock().current = None;
    }

    fn current(&self) -> Option<Arc<TransferControl>> {
        self.lock().current.clone()
    }
}

enum Slot {
    Free,
    /// The file dialog is open, or a retry is being prepared.
    Choosing,
    Active {
        id: String,
        control: Arc<BatchControl>,
    },
}

struct Inner {
    slot: Slot,
    next_id: u64,
    /// The last finished batch with something to retry, by batch id.
    retry: Option<(String, RetryPlan)>,
    /// Multipart uploads whose abort failed, until one succeeds.
    unresolved: Vec<Unresolved>,
    /// Set by [`CloudUploads::shutdown`]: no new batch starts.
    closed: bool,
}

#[cfg(test)]
type FileHook = Box<dyn Fn(usize, &Arc<TransferControl>) + Send + Sync>;
#[cfg(test)]
type IndexHook = Box<dyn Fn(usize) + Send + Sync>;

/// The one upload slot, managed as Tauri state.
pub struct CloudUploads {
    inner: Mutex<Inner>,
    /// Signalled whenever the slot becomes free.
    freed: Condvar,
    /// The operation journal every upload records itself in (ADR-030),
    /// set once in `run()`.
    journal: std::sync::OnceLock<Arc<Journal>>,
    /// Runs before a batch takes its next file (index in the batch).
    #[cfg(test)]
    before_file: Mutex<Option<IndexHook>>,
    /// Runs with each file's control once it is installed.
    #[cfg(test)]
    file_control: Mutex<Option<FileHook>>,
}

impl Default for CloudUploads {
    fn default() -> Self {
        Self::new()
    }
}

/// Holds the slot while the file dialog is open; frees it when dropped
/// unless a batch started.
pub struct Reservation {
    uploads: Arc<CloudUploads>,
    armed: bool,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            let mut inner = self.uploads.lock();
            if matches!(inner.slot, Slot::Choosing) {
                inner.slot = Slot::Free;
            }
            drop(inner);
            self.uploads.freed.notify_all();
        }
    }
}

/// One file while the batch runs.
struct Entry {
    file: UploadFile,
    /// Where to upload it from; `None` for a skipped file.
    planned: Option<Planned>,
}

/// Numbers and sends a batch's events.
struct Emitter {
    batch: String,
    seq: u64,
    sink: Arc<dyn UploadSink>,
}

impl Emitter {
    fn emit(&mut self, body: EventBody) {
        self.seq += 1;
        self.sink.emit(UploadEvent {
            batch: self.batch.clone(),
            seq: self.seq,
            body,
        });
    }

    fn file(&mut self, count: u32, file: &UploadFile) {
        self.emit(EventBody::File {
            count,
            file: file.clone(),
        });
    }
}

impl CloudUploads {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slot: Slot::Free,
                next_id: 0,
                retry: None,
                unresolved: Vec::new(),
                closed: false,
            }),
            freed: Condvar::new(),
            journal: std::sync::OnceLock::new(),
            #[cfg(test)]
            before_file: Mutex::new(None),
            #[cfg(test)]
            file_control: Mutex::new(None),
        }
    }

    /// Makes every upload record itself in `journal` (ADR-030). Once.
    pub fn record_in(&self, journal: Arc<Journal>) {
        let _ = self.journal.set(journal);
    }

    /// The provider confirmed this incomplete upload is gone: its journal
    /// entry has nothing left to inspect.
    fn forget_in_journal(&self, upload_id: &str) {
        if let Some(journal) = self.journal.get()
            && let Err(e) = journal.forget_upload_id(upload_id)
        {
            eprintln!("sotyware: cannot update the operation journal: {e}");
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the slot for a new batch, or fails with [`ErrorKind::Busy`].
    pub fn reserve(self: &Arc<Self>) -> Result<Reservation, CommandError> {
        let mut inner = self.lock();
        if inner.closed {
            return Err(CommandError::new(
                ErrorKind::Busy,
                "sotyware is quitting; no upload can start",
            ));
        }
        match inner.slot {
            Slot::Free => {
                inner.slot = Slot::Choosing;
                Ok(Reservation {
                    uploads: Arc::clone(self),
                    armed: true,
                })
            }
            Slot::Choosing => Err(CommandError::new(
                ErrorKind::Busy,
                "a file dialog for another upload is open",
            )),
            Slot::Active { .. } => Err(CommandError::new(
                ErrorKind::Busy,
                "another upload is running; wait for it to finish or cancel it",
            )),
        }
    }

    /// Starts a batch of the checked `candidates` into `target`, in the
    /// reserved slot, and returns at once. Without a file to upload (and
    /// nothing to clean up) no batch starts and the slot is freed. Fails
    /// with [`ErrorKind::Busy`] once quitting has begun (the dialog may have
    /// been open meanwhile).
    pub fn start(
        self: &Arc<Self>,
        reservation: Reservation,
        accounts: Arc<CloudAccounts>,
        target: Target,
        candidates: Vec<Candidate>,
        sink: Arc<dyn UploadSink>,
    ) -> Result<UploadSelection, CommandError> {
        self.launch(reservation, accounts, target, candidates, Vec::new(), sink)
    }

    fn launch(
        self: &Arc<Self>,
        mut reservation: Reservation,
        accounts: Arc<CloudAccounts>,
        target: Target,
        candidates: Vec<Candidate>,
        cleanup: Vec<Unresolved>,
        sink: Arc<dyn UploadSink>,
    ) -> Result<UploadSelection, CommandError> {
        let entries: Vec<Entry> = candidates
            .into_iter()
            .enumerate()
            .map(|(index, c)| {
                let planned = match (&c.skip, &c.key, c.size) {
                    (None, Some(key), Some(_)) => Some(Planned {
                        key: key.clone(),
                        path: c.path.clone(),
                    }),
                    _ => None,
                };
                Entry {
                    file: UploadFile {
                        index: u32::try_from(index).unwrap_or(u32::MAX),
                        state: if planned.is_some() {
                            FileState::Pending
                        } else {
                            FileState::Skipped
                        },
                        name: c.name,
                        key: c.key,
                        size: c.size,
                        stage: None,
                        bytes: 0,
                        total: c.size.unwrap_or(0),
                        started: false,
                        skip: c.skip,
                        error: None,
                        remote: None,
                        retryable: false,
                    },
                    planned,
                }
            })
            .collect();
        let files: Vec<UploadFile> = entries.iter().map(|e| e.file.clone()).collect();
        let prefix = target.prefix.clone();
        if entries.iter().all(|e| e.planned.is_none()) && cleanup.is_empty() {
            // Nothing to do: the reservation frees the slot when dropped.
            drop(reservation);
            return Ok(UploadSelection {
                batch: None,
                prefix,
                files,
            });
        }
        let control = Arc::new(BatchControl::default());
        let id = {
            let mut inner = self.lock();
            if inner.closed {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "sotyware is quitting; no upload can start",
                ));
            }
            inner.next_id += 1;
            let id = format!("upload-{}", inner.next_id);
            inner.slot = Slot::Active {
                id: id.clone(),
                control: Arc::clone(&control),
            };
            inner.retry = None;
            id
        };
        reservation.armed = false;
        let this = Arc::clone(self);
        let emitter = Emitter {
            batch: id.clone(),
            seq: 0,
            sink,
        };
        tauri::async_runtime::spawn(async move {
            this.run(accounts, target, entries, cleanup, control, emitter)
                .await;
        });
        Ok(UploadSelection {
            batch: Some(id),
            prefix,
            files,
        })
    }

    async fn run(
        &self,
        accounts: Arc<CloudAccounts>,
        target: Target,
        mut entries: Vec<Entry>,
        cleanup: Vec<Unresolved>,
        control: Arc<BatchControl>,
        mut emitter: Emitter,
    ) {
        let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        // Best effort, bounded by the abort timeout, never cancelled: a
        // cancel stops the files, not the cleanup.
        for u in cleanup {
            let outcome = u
                .store
                .abort_incomplete_upload(&u.bucket, &u.key, &u.upload_id)
                .await;
            let removed = !matches!(outcome, AbortOutcome::Failed);
            if removed {
                self.lock()
                    .unresolved
                    .retain(|known| known.upload_id != u.upload_id);
                if let Some(journal) = self.journal.get().cloned() {
                    let id = u.upload_id.clone();
                    let forgotten =
                        tokio::task::spawn_blocking(move || journal.forget_upload_id(&id)).await;
                    if let Ok(Err(e)) = forgotten {
                        eprintln!("sotyware: cannot update the operation journal: {e}");
                    }
                }
            }
            emitter.emit(EventBody::Cleanup {
                key: u.key.clone(),
                outcome: if removed {
                    CleanupOutcome::Removed
                } else {
                    CleanupOutcome::Failed
                },
            });
        }
        let stop = self
            .upload_all(
                &accounts,
                &target,
                &mut entries,
                &control,
                &mut emitter,
                count,
            )
            .await;
        // Whatever never started is cancelled (by the user or by the error
        // that stopped the batch).
        for entry in entries.iter_mut().filter(|e| !e.file.state.is_final()) {
            entry.file.state = FileState::Cancelled;
            entry.file.remote = Some(RemoteState::NothingStored);
            entry.file.retryable = true;
        }
        let state = match &stop {
            Some(Stop::Failed(_)) => BatchState::Failed,
            Some(Stop::Cancelled) => BatchState::Cancelled,
            None if entries.iter().any(|e| e.file.state == FileState::Cancelled) => {
                BatchState::Cancelled
            }
            None => BatchState::Completed,
        };
        debug_assert!(entries.iter().all(|e| e.file.state.is_final()));
        let files: Vec<UploadFile> = entries.iter().map(|e| e.file.clone()).collect();
        let tally = |s: FileState| {
            u32::try_from(files.iter().filter(|f| f.state == s).count()).unwrap_or(u32::MAX)
        };
        let summary = BatchSummary {
            state,
            count,
            completed: tally(FileState::Completed),
            skipped: tally(FileState::Skipped),
            failed: tally(FileState::Failed),
            cancelled: tally(FileState::Cancelled),
            not_started: u32::try_from(
                files
                    .iter()
                    .filter(|f| f.state == FileState::Cancelled && !f.started)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            retryable: u32::try_from(files.iter().filter(|f| f.retryable).count())
                .unwrap_or(u32::MAX),
            needs_attention: u32::try_from(
                files
                    .iter()
                    .filter(|f| {
                        matches!(
                            f.remote,
                            Some(RemoteState::IncompleteParts | RemoteState::NeedsInspection)
                        )
                    })
                    .count(),
            )
            .unwrap_or(u32::MAX),
            error: match stop {
                Some(Stop::Failed(e)) => Some(e),
                _ => None,
            },
            files,
        };
        let changed_account = target.account.clone();
        let retry: Vec<(PathBuf, Option<String>)> = entries
            .into_iter()
            .filter(|e| e.file.retryable)
            .filter_map(|e| e.planned)
            .map(|p| {
                let name = p
                    .key
                    .strip_prefix(target.prefix.as_str())
                    .map(str::to_owned);
                (p.path, name)
            })
            .collect();
        {
            let mut inner = self.lock();
            let has_unresolved = inner.unresolved.iter().any(|u| u.account == target.account);
            if !retry.is_empty() || has_unresolved {
                inner.retry = Some((
                    emitter.batch.clone(),
                    RetryPlan {
                        target,
                        files: retry,
                    },
                ));
            }
            if matches!(&inner.slot, Slot::Active { id, .. } if *id == emitter.batch) {
                inner.slot = Slot::Free;
            }
        }
        self.freed.notify_all();
        // A sync of this bucket lists it now (ADR-043).
        crate::folder_sync::bucket_changed(&changed_account);
        // Only now: the page may start the next batch as soon as it reads
        // this, and the slot is free.
        emitter.emit(EventBody::Finished { summary });
    }

    /// Uploads the planned files one after another. `Some` if the batch
    /// stopped early.
    async fn upload_all(
        &self,
        accounts: &Arc<CloudAccounts>,
        target: &Target,
        entries: &mut [Entry],
        control: &BatchControl,
        emitter: &mut Emitter,
        count: u32,
    ) -> Option<Stop> {
        if entries.iter().all(|e| e.planned.is_none()) {
            return None;
        }
        let store = {
            let Some(step) = control.begin() else {
                return Some(Stop::Cancelled);
            };
            let accounts = Arc::clone(accounts);
            let (account, bucket) = (target.account.clone(), target.bucket.clone());
            let prepared = step
                .unless_cancelled(blocking(move || accounts.prepare_upload(&account, &bucket)))
                .await;
            control.end();
            match prepared {
                None => return Some(Stop::Cancelled),
                Some(Err(e)) => {
                    eprintln!(
                        "sotyware: cannot start an upload to account '{}': {}",
                        target.account, e.message
                    );
                    return Some(Stop::Failed(e));
                }
                Some(Ok(store)) => store,
            }
        };
        #[cfg_attr(not(test), allow(unused_variables))]
        for (index, entry) in entries.iter_mut().enumerate() {
            let Some(planned) = entry.planned.clone() else {
                continue;
            };
            #[cfg(test)]
            if let Some(hook) = &*self.before_file.lock().unwrap() {
                hook(index);
            }
            let Some(file_control) = control.begin() else {
                return Some(Stop::Cancelled);
            };
            if let Some(journal) = self.journal.get() {
                file_control.record_in(Arc::clone(journal));
            }
            #[cfg(test)]
            if let Some(hook) = &*self.file_control.lock().unwrap() {
                hook(index, &file_control);
            }
            entry.file.started = true;
            let result = upload_one(
                &store,
                target,
                &planned,
                &file_control,
                &mut entry.file,
                emitter,
                count,
            )
            .await;
            control.end();
            let unresolved = match &result {
                Outcome::Failed {
                    upload_id: Some(id),
                    ..
                } => Some(id.clone()),
                _ => file_control.open_multipart(),
            };
            if let Some(upload_id) = unresolved {
                self.lock().unresolved.push(Unresolved {
                    store: Arc::clone(&store),
                    account: target.account.clone(),
                    bucket: target.bucket.clone(),
                    key: planned.key.clone(),
                    upload_id,
                });
            }
            emitter.file(count, &entry.file);
            match result {
                Outcome::Stop(stop) => return Some(stop),
                Outcome::Failed {
                    stop: true, error, ..
                } => return Some(Stop::Failed(error)),
                _ => {}
            }
            if control.is_cancelled() {
                return Some(Stop::Cancelled);
            }
        }
        None
    }

    /// Cancels batch `batch`: the file being uploaded and the files after
    /// it. Any other id is left alone. `Finishing` if the current file is
    /// past its commit point (it completes; no further file starts).
    pub fn cancel(&self, batch: &str) -> CancelOutcome {
        let inner = self.lock();
        match &inner.slot {
            Slot::Active { id, control } if id == batch => control.cancel(),
            _ => CancelOutcome::NotActive,
        }
    }

    /// Starts a new batch with what the finished batch `batch` left to do
    /// (see the module documentation), checking the files again. Fails if
    /// `batch` is not the last finished batch or left nothing to do.
    pub async fn retry(
        self: &Arc<Self>,
        batch: &str,
        accounts: Arc<CloudAccounts>,
        sink: Arc<dyn UploadSink>,
    ) -> Result<UploadSelection, CommandError> {
        let reservation = self.reserve()?;
        let (id, plan, cleanup) = {
            let mut inner = self.lock();
            match inner.retry.take() {
                Some((id, plan)) if id == batch => {
                    let cleanup: Vec<Unresolved> = inner
                        .unresolved
                        .iter()
                        .filter(|u| u.account == plan.target.account)
                        .cloned()
                        .collect();
                    (id, plan, cleanup)
                }
                other => {
                    inner.retry = other;
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "this upload can no longer be retried; choose the files again",
                    ));
                }
            }
        };
        if plan.files.is_empty() && cleanup.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "nothing in this upload is left to retry",
            ));
        }
        let checked = {
            let accounts = Arc::clone(&accounts);
            let target = plan.target.clone();
            let files = plan.files.clone();
            blocking(move || {
                let (limits, options) = accounts.upload_limits(&target.account, &target.bucket)?;
                Ok(check_named_selection(
                    &target.prefix,
                    files,
                    &limits,
                    &options,
                ))
            })
            .await
        };
        let candidates = match checked {
            Ok(candidates) => candidates,
            Err(e) => {
                // Kept, so the page can retry once the account is usable.
                let mut inner = self.lock();
                if inner.retry.is_none() {
                    inner.retry = Some((id, plan));
                }
                return Err(e);
            }
        };
        self.launch(
            reservation,
            accounts,
            plan.target,
            candidates,
            cleanup,
            sink,
        )
    }

    /// Whether a batch is running (or its file dialog is open).
    pub fn busy(&self) -> bool {
        !matches!(self.lock().slot, Slot::Free)
    }

    /// The multipart uploads whose abort failed, as (key, upload id).
    #[cfg(test)]
    pub(crate) fn unresolved(&self) -> Vec<(String, String)> {
        self.lock()
            .unresolved
            .iter()
            .map(|u| (u.key.clone(), u.upload_id.clone()))
            .collect()
    }

    /// Cancels whatever batch is running, for tests whose hooks do not
    /// know its id yet.
    #[cfg(test)]
    pub(crate) fn cancel_active(&self) -> CancelOutcome {
        let inner = self.lock();
        match &inner.slot {
            Slot::Active { control, .. } => control.cancel(),
            _ => CancelOutcome::NotActive,
        }
    }

    #[cfg(test)]
    pub(crate) fn before_file(&self, f: impl Fn(usize) + Send + Sync + 'static) {
        *self.before_file.lock().unwrap() = Some(Box::new(f));
    }

    #[cfg(test)]
    pub(crate) fn file_control(
        &self,
        f: impl Fn(usize, &Arc<TransferControl>) + Send + Sync + 'static,
    ) {
        *self.file_control.lock().unwrap() = Some(Box::new(f));
    }

    /// For quitting: refuses new batches, cancels the running one, aborts
    /// every unresolved multipart upload, and waits up to `wait` for all of
    /// it. What is not done in time is logged; parts may then remain at the
    /// provider.
    pub fn shutdown(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let (tx, rx) = mpsc::channel::<(String, String, bool)>();
        let mut pending = 0usize;
        let mut started: Vec<String> = Vec::new();
        let mut abort_all = |list: Vec<Unresolved>, started: &mut Vec<String>| {
            for u in list {
                if started.contains(&u.upload_id) {
                    continue;
                }
                started.push(u.upload_id.clone());
                pending += 1;
                let tx = tx.clone();
                tauri::async_runtime::spawn(async move {
                    let outcome = u
                        .store
                        .abort_incomplete_upload(&u.bucket, &u.key, &u.upload_id)
                        .await;
                    let _ = tx.send((u.key, u.upload_id, outcome != AbortOutcome::Failed));
                });
            }
        };
        let mut inner = self.lock();
        inner.closed = true;
        abort_all(inner.unresolved.clone(), &mut started);
        let control = match &inner.slot {
            Slot::Active { control, .. } => Some(Arc::clone(control)),
            _ => None,
        };
        if let Some(control) = &control {
            control.cancel();
            while matches!(inner.slot, Slot::Active { .. }) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                inner = self
                    .freed
                    .wait_timeout(inner, left)
                    .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
            }
            if matches!(inner.slot, Slot::Active { .. }) {
                eprintln!("sotyware: an upload did not stop in time while quitting");
                if let Some(upload_id) = control.current().and_then(|c| c.open_multipart()) {
                    eprintln!(
                        "sotyware: the multipart upload {upload_id} may not be aborted; its parts may remain at the provider"
                    );
                }
            }
            // The batch may have added uploads its own abort could not remove.
            abort_all(inner.unresolved.clone(), &mut started);
        }
        drop(inner);
        drop(abort_all);
        drop(tx);
        while pending > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok((key, upload_id, removed)) => {
                    pending -= 1;
                    if removed {
                        self.lock().unresolved.retain(|u| u.upload_id != upload_id);
                        self.forget_in_journal(&upload_id);
                    } else {
                        eprintln!(
                            "sotyware: cannot abort the incomplete upload of '{key}' while quitting; its parts may remain at the provider"
                        );
                    }
                }
                Err(_) => {
                    eprintln!(
                        "sotyware: {pending} incomplete multipart upload(s) were not aborted in time while quitting; their parts may remain at the provider"
                    );
                    break;
                }
            }
        }
    }
}

/// Why a batch stopped early.
enum Stop {
    Cancelled,
    Failed(CommandError),
}

/// How one file ended (its record is already updated).
enum Outcome {
    Done,
    Failed {
        stop: bool,
        error: CommandError,
        upload_id: Option<String>,
    },
    Stop(Stop),
}

/// Uploads one file: destination check, then [`S3Store::upload`]. Updates
/// `file` to its final state and sends its progress (not the final event).
async fn upload_one(
    store: &Arc<S3Store>,
    target: &Target,
    planned: &Planned,
    control: &Arc<TransferControl>,
    file: &mut UploadFile,
    emitter: &mut Emitter,
    count: u32,
) -> Outcome {
    let fail = |file: &mut UploadFile, error: &StorageError, remote: &RemoteOutcome| {
        let shown = upload_error(error);
        eprintln!(
            "sotyware: upload of '{}' to account '{}' failed: {error}",
            planned.key, target.account
        );
        file.state = FileState::Failed;
        file.error = Some(shown.clone());
        file.remote = Some(RemoteState::from(remote));
        file.retryable = matches!(remote, RemoteOutcome::NothingStored);
        shown
    };
    let exists = |file: &mut UploadFile| {
        file.state = FileState::Skipped;
        // Taken, but not uploaded: like a file skipped at selection.
        file.started = false;
        file.skip = Some(Skip::new(
            SkipReason::DestinationExists,
            "an object with this name is already in the folder; it was left as it is",
        ));
    };
    let cancelled = |file: &mut UploadFile, remote: &RemoteOutcome| {
        file.state = FileState::Cancelled;
        file.remote = Some(RemoteState::from(remote));
        file.retryable = matches!(remote, RemoteOutcome::NothingStored);
    };
    // Checked before the file is read, so an existing object costs no
    // hashing; `S3Store::upload` checks again before sending.
    match control
        .unless_cancelled(store.head_object(&target.bucket, &planned.key))
        .await
    {
        None => {
            cancelled(file, &RemoteOutcome::NothingStored);
            return Outcome::Stop(Stop::Cancelled);
        }
        Some(Ok(Some(_))) => {
            exists(file);
            return Outcome::Done;
        }
        Some(Ok(None)) => {}
        Some(Err(e)) => {
            let stop = disposition(&e) == Disposition::StopBatch;
            let error = fail(file, &e, &RemoteOutcome::NothingStored);
            return Outcome::Failed {
                stop,
                error,
                upload_id: None,
            };
        }
    }
    let result = {
        let mut progress = |p: UploadProgress| {
            let stage = Stage::from(p.stage);
            // `S3Store::upload` only moves forward; this keeps it so even
            // if that ever changed.
            if file
                .stage
                .is_some_and(|s| (s, file.bytes) >= (stage, p.bytes))
            {
                return;
            }
            file.stage = Some(stage);
            file.state = FileState::from(stage);
            file.bytes = p.bytes;
            file.total = p.total;
            emitter.file(count, file);
        };
        store
            .upload(
                &target.bucket,
                &planned.key,
                &planned.path,
                control,
                &mut progress,
            )
            .await
    };
    match result {
        Ok(report) => {
            file.state = FileState::Completed;
            file.stage = Some(Stage::Verifying);
            file.bytes = report.size;
            file.total = report.size;
            file.remote = None;
            Outcome::Done
        }
        Err(e) => match disposition(&e.error) {
            Disposition::Cancelled => {
                cancelled(file, &e.remote);
                Outcome::Stop(Stop::Cancelled)
            }
            Disposition::Skip if e.remote == RemoteOutcome::NothingStored => {
                exists(file);
                Outcome::Done
            }
            d => {
                let upload_id = match &e.remote {
                    RemoteOutcome::IncompleteParts { upload_id } => Some(upload_id.clone()),
                    _ => None,
                };
                let error = fail(file, &e.error, &e.remote);
                Outcome::Failed {
                    stop: d == Disposition::StopBatch,
                    error,
                    upload_id,
                }
            }
        },
    }
}

/// Opens the native file dialog over the dashboard for several files;
/// `None` if the user cancelled it.
#[cfg(desktop)]
async fn choose_files(window: &tauri::WebviewWindow) -> Option<Vec<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Choose files to upload")
        .pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    let paths = rx.await.ok().flatten()?;
    Some(
        paths
            .into_iter()
            .filter_map(|p| match p.into_path() {
                Ok(path) => Some(path),
                Err(e) => {
                    eprintln!("sotyware: the file dialog returned no file path: {e}");
                    None
                }
            })
            .collect(),
    )
}

#[cfg(not(desktop))]
async fn choose_files(_window: &tauri::WebviewWindow) -> Option<Vec<PathBuf>> {
    None
}

/// `upload_cloud_files` without Tauri: `pick` stands for the file dialog.
/// `Ok(None)` if the dialog was cancelled or returned nothing.
pub async fn begin_upload(
    uploads: &Arc<CloudUploads>,
    accounts: Arc<CloudAccounts>,
    target: Target,
    pick: impl Future<Output = Option<Vec<PathBuf>>>,
    sink: Arc<dyn UploadSink>,
) -> Result<Option<UploadSelection>, CommandError> {
    validate_prefix(&target.prefix)?;
    let reservation = uploads.reserve()?;
    let (limits, options) = {
        let accounts = Arc::clone(&accounts);
        let (id, bucket) = (target.account.clone(), target.bucket.clone());
        blocking(move || accounts.upload_limits(&id, &bucket)).await?
    };
    let paths = match pick.await {
        None => return Ok(None),
        Some(paths) if paths.is_empty() => return Ok(None),
        Some(paths) => paths,
    };
    if paths.len() > MAX_FILES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} files were chosen; an upload takes at most {MAX_FILES}",
                paths.len()
            ),
        ));
    }
    let prefix = target.prefix.clone();
    let candidates =
        blocking(move || Ok(check_selection(&prefix, paths, &limits, &options))).await?;
    uploads
        .start(reservation, accounts, target, candidates, sink)
        .map(Some)
}

/// Asks for files in the native file dialog and uploads them, one after
/// another, into folder `prefix` of the account's configured bucket (which
/// must still be `bucket`, the one the page browsed). Returns `null` if the
/// dialog was cancelled (nothing started), else the selection and, if any
/// file can be uploaded, the batch id; events arrive on `on_event`. Allowed
/// by `allow-upload-cloud-files`.
#[tauri::command]
pub async fn upload_cloud_files(
    window: tauri::WebviewWindow,
    accounts: State<'_, Arc<CloudAccounts>>,
    uploads: State<'_, Arc<CloudUploads>>,
    id: String,
    bucket: String,
    prefix: String,
    on_event: Channel<UploadEvent>,
) -> Result<Option<UploadSelection>, CommandError> {
    begin_upload(
        &uploads,
        Arc::clone(&accounts),
        Target {
            account: id,
            bucket,
            prefix,
        },
        choose_files(&window),
        Arc::new(on_event),
    )
    .await
}

/// `upload_cloud_folder` without Tauri: `pick` stands for the folder
/// dialog. `Ok(None)` if it was cancelled.
pub async fn begin_folder_upload(
    uploads: &Arc<CloudUploads>,
    accounts: Arc<CloudAccounts>,
    target: Target,
    pick: impl Future<Output = Option<PathBuf>>,
    sink: Arc<dyn UploadSink>,
) -> Result<Option<UploadSelection>, CommandError> {
    validate_prefix(&target.prefix)?;
    let reservation = uploads.reserve()?;
    let (limits, options) = {
        let accounts = Arc::clone(&accounts);
        let (id, bucket) = (target.account.clone(), target.bucket.clone());
        blocking(move || accounts.upload_limits(&id, &bucket)).await?
    };
    let Some(root) = pick.await else {
        return Ok(None);
    };
    let prefix = target.prefix.clone();
    let candidates = blocking(move || {
        let files = folder_files(&root, MAX_FOLDER_FILES)?;
        if files.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the folder holds no files (empty folders are not uploaded)",
            ));
        }
        Ok(check_folder_selection(&prefix, files, &limits, &options))
    })
    .await?;
    uploads
        .start(reservation, accounts, target, candidates, sink)
        .map(Some)
}

#[cfg(desktop)]
async fn choose_folder(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Choose a folder to upload")
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
async fn choose_folder(_window: &tauri::WebviewWindow) -> Option<PathBuf> {
    None
}

/// Asks for a folder in the native folder dialog and uploads every file in
/// it (any depth, at most 1,000) into `prefix` + the folder's name, as one
/// batch with the same rules as `upload_cloud_files`. The folder's path
/// stays in Rust. Allowed by `allow-upload-cloud-folder`.
#[tauri::command]
pub async fn upload_cloud_folder(
    window: tauri::WebviewWindow,
    accounts: State<'_, Arc<CloudAccounts>>,
    uploads: State<'_, Arc<CloudUploads>>,
    id: String,
    bucket: String,
    prefix: String,
    on_event: Channel<UploadEvent>,
) -> Result<Option<UploadSelection>, CommandError> {
    begin_folder_upload(
        &uploads,
        Arc::clone(&accounts),
        Target {
            account: id,
            bucket,
            prefix,
        },
        choose_folder(&window),
        Arc::new(on_event),
    )
    .await
}

/// Cancels the running batch `batch`; does nothing for any other id.
/// Allowed by `allow-cancel-cloud-upload`.
#[tauri::command]
pub fn cancel_cloud_upload(uploads: State<'_, Arc<CloudUploads>>, batch: String) -> CancelOutcome {
    uploads.cancel(&batch)
}

/// Starts what the finished batch `batch` left to do again (no dialog).
/// Allowed by `allow-retry-cloud-upload`.
#[tauri::command]
pub async fn retry_cloud_upload(
    accounts: State<'_, Arc<CloudAccounts>>,
    uploads: State<'_, Arc<CloudUploads>>,
    batch: String,
    on_event: Channel<UploadEvent>,
) -> Result<UploadSelection, CommandError> {
    uploads
        .retry(&batch, Arc::clone(&accounts), Arc::new(on_event))
        .await
}

#[cfg(test)]
mod tests;

//! Downloading one file at a time from an account's configured bucket
//! (ADR-020): the `download_cloud_file`, `cancel_cloud_download` and
//! `retry_cloud_download` commands, the progress messages they send, and the
//! one transfer slot behind them.
//!
//! - **Where from:** the account's configured bucket only. The page names the
//!   bucket it browsed so a stale view is refused, never to pick another
//!   bucket; the object key is used exactly as the listing returned it.
//! - **Where to:** a path the user chose in the native save dialog, which
//!   Rust opens itself. The page never sends a path, and the key only
//!   suggests a file name in the dialog. The dialog asks before replacing an
//!   existing file; a file chosen that way is replaced in one rename, any
//!   other existing file is never touched (see [`Existing`]).
//! - **How:** [`S3Store::download`](super::s3::S3Store::download) streams
//!   into a temporary file next to the destination, verifies it, and moves
//!   it into place. The command returns as soon as the transfer has started;
//!   the transfer runs on the async runtime, the credential store read on the
//!   blocking pool.
//! - **Progress:** [`DownloadEvent`]s on the Tauri channel the page passed
//!   with the command (so they reach only that page and that download), at
//!   most one "active" message per [`PROGRESS_INTERVAL`], then exactly one
//!   final message (completed, cancelled or failed), sent after the slot is
//!   free again and, for "completed", after the file is in place.
//! - **One at a time:** a second download (or a second save dialog) is
//!   refused with [`ErrorKind::Busy`] until the first has finished.
//! - **Cancel** stops the request and the body stream; it never affects
//!   another transfer id and is idempotent. Once the verified file is being
//!   moved into place, cancelling is too late and the download completes.
//! - **Retry** repeats the last failed download with the same account,
//!   bucket, key and destination.
//! - **Shutdown** ([`CloudDownloads::shutdown`]) cancels a running download
//!   and waits a bounded time for its temporary file to be removed.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::s3::{DownloadReport, Existing, StorageError, TransferControl, Verification};

/// At most one "active" progress message per this interval.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// How long quitting waits for a cancelled download to clean up.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// Longest object key S3 allows, in bytes.
const MAX_KEY_BYTES: usize = 1024;

/// Longest suggested file name, in bytes.
const MAX_SUGGESTED_NAME_BYTES: usize = 200;

/// Where a transfer stands, as the page shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferState {
    Active,
    /// The file is complete at its destination.
    Completed,
    /// Stopped by the user (or by quitting); nothing was written to the
    /// destination.
    Cancelled,
    /// Stopped by an error; nothing was written to the destination.
    Failed,
}

/// What a completed download was checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Integrity {
    /// The content matches the SHA-256 sotyware stored at upload.
    Sha256Verified,
    /// The object has no SHA-256 stored by sotyware: only the size the
    /// provider announced was checked.
    SizeOnly,
}

/// One progress message. Never carries a credential or provider text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvent {
    pub transfer: String,
    pub state: TransferState,
    /// Bytes written to the temporary file so far (for "completed": the
    /// file's size).
    pub bytes: u64,
    /// The size the provider announced, if it did.
    pub total: Option<u64>,
    /// Only for "completed".
    pub integrity: Option<Integrity>,
    /// Only for "failed".
    pub error: Option<CommandError>,
}

impl DownloadEvent {
    fn new(transfer: &str, state: TransferState, bytes: u64, total: Option<u64>) -> Self {
        Self {
            transfer: transfer.to_owned(),
            state,
            bytes,
            total,
            integrity: None,
            error: None,
        }
    }
}

/// What `download_cloud_file` and `retry_cloud_download` return.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStarted {
    pub transfer: String,
    /// The destination's file name (not its folder), for the page to show.
    pub file_name: String,
}

/// What a cancel request did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CancelOutcome {
    /// The transfer stops; a "cancelled" message follows.
    Cancelling,
    /// Too late: the verified file is being moved into place; a
    /// "completed" (or "failed") message follows.
    Finishing,
    /// No running transfer has this id (finished, or never existed).
    NotActive,
}

/// Where progress messages go: the page's channel, or a test recorder.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: DownloadEvent);
}

impl EventSink for Channel<DownloadEvent> {
    fn emit(&self, event: DownloadEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: cannot report download progress: {e}");
        }
    }
}

/// Lets through at most one progress message per interval (the first one
/// always).
#[derive(Debug)]
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// The file name the save dialog suggests for `key`: its last segment, with
/// characters Windows or Linux refuse in a name replaced by `_`. Only a
/// suggestion: the user's choice in the dialog is the path.
pub fn suggested_file_name(key: &str) -> String {
    let last = key.rsplit('/').next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Windows drops trailing dots and spaces; `.` and `..` are not names.
    let trimmed = cleaned.trim_end_matches(['.', ' ']).trim_start();
    if trimmed.is_empty() {
        return "download".to_owned();
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    let mut name = if reserved {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    };
    if name.len() > MAX_SUGGESTED_NAME_BYTES {
        let mut end = MAX_SUGGESTED_NAME_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// Refuses a key no object can have.
pub fn validate_key(key: &str) -> Result<(), CommandError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("an object key must be 1 to {MAX_KEY_BYTES} bytes"),
        ));
    }
    Ok(())
}

/// Checks the path the save dialog returned and decides what happens to an
/// existing file there: the dialog asked before returning an existing file,
/// so only that file may be replaced.
pub fn destination(path: PathBuf) -> Result<(PathBuf, Existing), CommandError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "the save dialog returned an unusable path: {}",
                path.display()
            ),
        ));
    }
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_dir() => Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("{} is a folder; choose a file name", path.display()),
        )),
        Ok(_) => Ok((path, Existing::Replace)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((path, Existing::Refuse)),
        Err(e) => Err(CommandError::new(
            ErrorKind::DestinationUnavailable,
            format!("{}: {e}", path.display()),
        )),
    }
}

/// How a failed download is shown. Sentences are sotyware's own; the
/// provider's error text is only logged. Local errors name the user's own
/// destination path.
pub fn download_error(error: &StorageError) -> CommandError {
    let (kind, message) = match error {
        StorageError::InvalidCredentials(_) => (
            ErrorKind::InvalidCredentials,
            "the provider does not accept this key ID or secret".to_owned(),
        ),
        StorageError::AccessDenied(_) => (
            ErrorKind::AccessDenied,
            "the provider refused to download this file with this key".to_owned(),
        ),
        StorageError::NotFound(_) => (
            ErrorKind::ObjectNotFound,
            "the file is no longer in the bucket, or this key may not read it".to_owned(),
        ),
        StorageError::Network(_) => (
            ErrorKind::Network,
            "the connection to the provider failed or broke off".to_owned(),
        ),
        StorageError::Timeout(_) => (
            ErrorKind::Timeout,
            "the provider stopped sending data".to_owned(),
        ),
        StorageError::Integrity(_) => (
            ErrorKind::Integrity,
            "the downloaded data does not match the file's size or stored SHA-256 checksum"
                .to_owned(),
        ),
        StorageError::LocalWrite { kind, message } => match kind {
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => (
                ErrorKind::DiskFull,
                format!("there is not enough free space: {message}"),
            ),
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => (
                ErrorKind::LocalPermission,
                format!("the file may not be written there: {message}"),
            ),
            io::ErrorKind::NotFound
            | io::ErrorKind::NotADirectory
            | io::ErrorKind::NotConnected
            | io::ErrorKind::BrokenPipe => (
                ErrorKind::DestinationUnavailable,
                format!("the destination folder is not available: {message}"),
            ),
            io::ErrorKind::AlreadyExists => (
                ErrorKind::DestinationExists,
                format!(
                    "a file appeared at the destination meanwhile and was left as it is: {message}"
                ),
            ),
            _ => (ErrorKind::LocalFile, message.clone()),
        },
        StorageError::Io(m) => (ErrorKind::LocalFile, m.clone()),
        // Only sotyware's own checks (bucket not configured, key length).
        StorageError::Refused(m) => (ErrorKind::InvalidConfiguration, m.clone()),
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
                format!("the provider refused the download: HTTP {status}{code}")
            };
            (ErrorKind::Provider, message)
        }
        // Not reached: a cancel is handled before, the rest are upload errors.
        StorageError::Cancelled
        | StorageError::FileChanged(_)
        | StorageError::AlreadyExists(_)
        | StorageError::TooLarge { .. } => (
            ErrorKind::Other,
            "the file could not be downloaded".to_owned(),
        ),
    };
    CommandError::new(kind, message)
}

/// What a download needs, kept for a retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub account: String,
    pub bucket: String,
    pub key: String,
    pub dest: PathBuf,
    pub existing: Existing,
}

enum Slot {
    Free,
    /// A save dialog is open for a download.
    Choosing,
    Active {
        id: String,
        control: Arc<TransferControl>,
    },
}

struct Inner {
    slot: Slot,
    next_id: u64,
    /// The last failed download, by transfer id.
    retry: Option<(String, Request)>,
    /// Set by [`CloudDownloads::shutdown`]: no new download starts.
    closed: bool,
}

/// The one download slot, managed as Tauri state.
pub struct CloudDownloads {
    inner: Mutex<Inner>,
    /// Signalled whenever the slot becomes free.
    freed: Condvar,
}

impl Default for CloudDownloads {
    fn default() -> Self {
        Self::new()
    }
}

/// Holds the slot while the save dialog is open; frees it when dropped
/// unless the download started.
pub struct Reservation {
    downloads: Arc<CloudDownloads>,
    armed: bool,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            let mut inner = self.downloads.lock();
            if matches!(inner.slot, Slot::Choosing) {
                inner.slot = Slot::Free;
            }
            drop(inner);
            self.downloads.freed.notify_all();
        }
    }
}

enum Failure {
    Cancelled,
    Failed(CommandError),
}

impl CloudDownloads {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slot: Slot::Free,
                next_id: 0,
                retry: None,
                closed: false,
            }),
            freed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the slot for a new download, or fails with
    /// [`ErrorKind::Busy`].
    pub fn reserve(self: &Arc<Self>) -> Result<Reservation, CommandError> {
        let mut inner = self.lock();
        if inner.closed {
            return Err(CommandError::new(
                ErrorKind::Busy,
                "sotyware is quitting; no download can start",
            ));
        }
        match inner.slot {
            Slot::Free => {
                inner.slot = Slot::Choosing;
                Ok(Reservation {
                    downloads: Arc::clone(self),
                    armed: true,
                })
            }
            Slot::Choosing => Err(CommandError::new(
                ErrorKind::Busy,
                "a save dialog for another download is open",
            )),
            Slot::Active { .. } => Err(CommandError::new(
                ErrorKind::Busy,
                "another download is running; wait for it to finish or cancel it",
            )),
        }
    }

    /// Starts `request` in the reserved slot and returns at once; the
    /// transfer reports to `sink`.
    pub fn start(
        self: &Arc<Self>,
        mut reservation: Reservation,
        accounts: Arc<CloudAccounts>,
        request: Request,
        sink: Arc<dyn EventSink>,
    ) -> DownloadStarted {
        let control = Arc::new(TransferControl::new());
        let id = {
            let mut inner = self.lock();
            inner.next_id += 1;
            let id = format!("download-{}", inner.next_id);
            inner.slot = Slot::Active {
                id: id.clone(),
                control: Arc::clone(&control),
            };
            inner.retry = None;
            id
        };
        reservation.armed = false;
        let started = DownloadStarted {
            transfer: id.clone(),
            file_name: request
                .dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            this.run(accounts, id, request, control, sink).await;
        });
        started
    }

    async fn run(
        &self,
        accounts: Arc<CloudAccounts>,
        id: String,
        request: Request,
        control: Arc<TransferControl>,
        sink: Arc<dyn EventSink>,
    ) {
        sink.emit(DownloadEvent::new(&id, TransferState::Active, 0, None));
        let mut seen = (0, None);
        let result = transfer(&accounts, &request, &control, &*sink, &id, &mut seen).await;
        let (bytes, total) = seen;
        let event = match &result {
            Ok(report) => DownloadEvent {
                integrity: Some(match report.verification {
                    Verification::Sha256Matched => Integrity::Sha256Verified,
                    Verification::SizeOnly => Integrity::SizeOnly,
                }),
                ..DownloadEvent::new(
                    &id,
                    TransferState::Completed,
                    report.size,
                    total.or(Some(report.size)),
                )
            },
            Err(Failure::Cancelled) => {
                DownloadEvent::new(&id, TransferState::Cancelled, bytes, total)
            }
            Err(Failure::Failed(e)) => DownloadEvent {
                error: Some(e.clone()),
                ..DownloadEvent::new(&id, TransferState::Failed, bytes, total)
            },
        };
        {
            let mut inner = self.lock();
            if matches!(&inner.slot, Slot::Active { id: active, .. } if *active == id) {
                inner.slot = Slot::Free;
            }
            if matches!(result, Err(Failure::Failed(_))) {
                inner.retry = Some((id.clone(), request));
            }
        }
        self.freed.notify_all();
        // Only now: the page may start the next download as soon as it
        // reads this, and the slot is free.
        sink.emit(event);
    }

    /// Cancels the running transfer `transfer`; any other id is left alone.
    pub fn cancel(&self, transfer: &str) -> CancelOutcome {
        let inner = self.lock();
        match &inner.slot {
            Slot::Active { id, control } if id == transfer => {
                if control.cancel() {
                    CancelOutcome::Cancelling
                } else {
                    CancelOutcome::Finishing
                }
            }
            _ => CancelOutcome::NotActive,
        }
    }

    /// Starts the last failed download `transfer` again, to the same
    /// destination.
    pub fn retry(
        self: &Arc<Self>,
        transfer: &str,
        accounts: Arc<CloudAccounts>,
        sink: Arc<dyn EventSink>,
    ) -> Result<DownloadStarted, CommandError> {
        let reservation = self.reserve()?;
        let request = {
            let mut inner = self.lock();
            match inner.retry.take() {
                Some((id, request)) if id == transfer => request,
                other => {
                    inner.retry = other;
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "this download can no longer be retried; start it again",
                    ));
                }
            }
        };
        Ok(self.start(reservation, accounts, request, sink))
    }

    /// Whether a download is running (or its save dialog is open).
    pub fn busy(&self) -> bool {
        !matches!(self.lock().slot, Slot::Free)
    }

    /// The running transfer's control, so tests can hook into it.
    #[cfg(test)]
    pub(crate) fn active_control(&self) -> Option<Arc<TransferControl>> {
        match &self.lock().slot {
            Slot::Active { control, .. } => Some(Arc::clone(control)),
            _ => None,
        }
    }

    /// For quitting: refuses new downloads, cancels the running one and
    /// waits up to `wait` for it to remove its temporary file. If it does not
    /// finish in time, the temporary file is removed here (a download that
    /// is already moving its verified file into place is waited for, never
    /// interrupted).
    pub fn shutdown(&self, wait: Duration) {
        let mut inner = self.lock();
        inner.closed = true;
        let control = match &inner.slot {
            Slot::Active { control, .. } => Arc::clone(control),
            _ => return,
        };
        control.cancel();
        let deadline = Instant::now() + wait;
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
            eprintln!("sotyware: a download did not stop in time while quitting");
            if control.is_cancelled()
                && let Some(partial) = control.partial_file()
                && let Err(e) = std::fs::remove_file(&partial)
                && e.kind() != io::ErrorKind::NotFound
            {
                eprintln!(
                    "sotyware: cannot remove the temporary download file {}: {e}",
                    partial.display()
                );
            }
        }
    }
}

async fn transfer(
    accounts: &Arc<CloudAccounts>,
    request: &Request,
    control: &TransferControl,
    sink: &dyn EventSink,
    id: &str,
    seen: &mut (u64, Option<u64>),
) -> Result<DownloadReport, Failure> {
    let store = {
        let accounts = Arc::clone(accounts);
        let (account, bucket) = (request.account.clone(), request.bucket.clone());
        control
            .unless_cancelled(blocking(move || {
                accounts.prepare_download(&account, &bucket)
            }))
            .await
            .ok_or(Failure::Cancelled)?
            .map_err(|e| {
                eprintln!(
                    "sotyware: cannot start a download from account '{}': {}",
                    request.account, e.message
                );
                Failure::Failed(e)
            })?
    };
    let mut throttle = Throttle::new(PROGRESS_INTERVAL);
    let mut progress = |bytes: u64, total: Option<u64>| {
        *seen = (bytes, total);
        if throttle.ready(Instant::now()) {
            sink.emit(DownloadEvent::new(id, TransferState::Active, bytes, total));
        }
    };
    store
        .download(
            &request.bucket,
            &request.key,
            &request.dest,
            request.existing,
            control,
            &mut progress,
        )
        .await
        .map_err(|e| match e {
            StorageError::Cancelled => Failure::Cancelled,
            e => {
                eprintln!(
                    "sotyware: download from account '{}' failed: {e}",
                    request.account
                );
                Failure::Failed(download_error(&e))
            }
        })
}

/// Opens the native save dialog over the dashboard, suggesting `name`;
/// `None` if the user cancelled it. On Linux the dialog (GTK) and on Windows
/// (common item dialog) both ask before returning an existing file.
#[cfg(desktop)]
async fn choose_destination(window: &tauri::WebviewWindow, name: String) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Save download as")
        .set_file_name(name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let path = rx.await.ok().flatten()?;
    match path.into_path() {
        Ok(path) => Some(path),
        Err(e) => {
            eprintln!("sotyware: the save dialog returned no file path: {e}");
            None
        }
    }
}

#[cfg(not(desktop))]
async fn choose_destination(_window: &tauri::WebviewWindow, _name: String) -> Option<PathBuf> {
    None
}

/// Asks for a destination in the native save dialog and starts downloading
/// `key` from the account's configured bucket (which must still be
/// `bucket`, the one the page browsed). Returns `null` if the dialog was
/// cancelled (nothing started), else the transfer id; progress arrives on
/// `on_event`. Allowed by `allow-download-cloud-file`.
#[tauri::command]
pub async fn download_cloud_file(
    window: tauri::WebviewWindow,
    accounts: State<'_, Arc<CloudAccounts>>,
    downloads: State<'_, Arc<CloudDownloads>>,
    id: String,
    bucket: String,
    key: String,
    on_event: Channel<DownloadEvent>,
) -> Result<Option<DownloadStarted>, CommandError> {
    validate_key(&key)?;
    let reservation = downloads.reserve()?;
    {
        let accounts = Arc::clone(&accounts);
        let (id, bucket) = (id.clone(), bucket.clone());
        blocking(move || accounts.check_download_source(&id, &bucket)).await?;
    }
    let Some(path) = choose_destination(&window, suggested_file_name(&key)).await else {
        return Ok(None);
    };
    let (dest, existing) = blocking(move || destination(path)).await?;
    Ok(Some(downloads.start(
        reservation,
        Arc::clone(&accounts),
        Request {
            account: id,
            bucket,
            key,
            dest,
            existing,
        },
        Arc::new(on_event),
    )))
}

/// Cancels the running download `transfer`; does nothing for any other id.
/// Allowed by `allow-cancel-cloud-download`.
#[tauri::command]
pub fn cancel_cloud_download(
    downloads: State<'_, Arc<CloudDownloads>>,
    transfer: String,
) -> CancelOutcome {
    downloads.cancel(&transfer)
}

/// Starts the failed download `transfer` again, to the same destination
/// (no dialog). Allowed by `allow-retry-cloud-download`.
#[tauri::command]
pub fn retry_cloud_download(
    accounts: State<'_, Arc<CloudAccounts>>,
    downloads: State<'_, Arc<CloudDownloads>>,
    transfer: String,
    on_event: Channel<DownloadEvent>,
) -> Result<DownloadStarted, CommandError> {
    downloads.retry(&transfer, Arc::clone(&accounts), Arc::new(on_event))
}

#[cfg(test)]
mod tests;

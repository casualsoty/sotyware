//! Downloading several files and whole folders at once (ADR-026): the
//! `download_cloud_items`, `cancel_cloud_items_download` and
//! `retry_cloud_items_download` commands, their events, and their slot.
//!
//! - **Where to:** a folder the user picks in the native folder dialog,
//!   which Rust opens; the path stays in Rust (the page gets names only,
//!   also in errors).
//! - **What:** the selected files, and every object under each selected
//!   folder (recursive listing, at most [`MAX_FILES`] files in all). A
//!   folder keeps its structure: `photos/2024/a.jpg` under the selected
//!   `photos/` is saved as `photos/2024/a.jpg` in the chosen folder.
//!   Folder markers become empty folders.
//! - **Safe local names** ([`local_segment`]): characters Windows refuses
//!   (`< > : " / \ | ? *`, control characters), trailing dots and spaces,
//!   reserved device names (`CON`, `NUL`, `COM1`, …), `.`, `..` and empty
//!   names are replaced the same way on both systems, so a key can never
//!   point outside the chosen folder. Two keys that end up with the same
//!   local name (also differing only in case) are numbered.
//! - **Staging:** everything is downloaded into a hidden folder
//!   `.sotyware-download-<pid>-<n>` inside the chosen folder, each file
//!   through `S3Store::download` (size and stored SHA-256 verified, as for
//!   a single download). An item (a selected file or folder) is moved into
//!   place only once every one of its files is verified, under its own
//!   name or, if that name exists, the first free `name (n)`: nothing is
//!   ever replaced or merged. An item with a failed file is removed from
//!   the staging folder and reported failed; nothing half-downloaded is
//!   left in the chosen folder. The staging folder is removed at the end
//!   (a killed process leaves it, hidden).
//! - **One batch at a time** in its own slot; files one after another
//!   (each still waits for the account's transfer queue).
//! - **Cancel** stops the current file (its partial data is removed) and
//!   the rest; items already moved into place stay. **Retry** downloads
//!   the failed and cancelled items again into the same folder.
//! - **Shutdown** cancels and waits at most [`SHUTDOWN_WAIT`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::download::{CancelOutcome, Integrity};
use super::operations::{
    ItemRef, MAX_KEY_BYTES, file_name, folder_name, in_folder, operation_error,
};
use super::s3::{Existing, S3Store, StorageError, TransferControl, Verification};

/// Most files one batch may download.
pub const MAX_FILES: usize = 10_000;
/// Most items (selected files and folders) one batch may name.
pub const MAX_ITEMS: usize = 1_000;
/// How long quitting waits for the cancelled batch to clean up.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
/// Least time between two progress events of one file.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

// ------------------------------------------------------------ local names

/// One key segment as a file or folder name that is valid and stays inside
/// its folder on Windows and Linux.
pub fn local_segment(segment: &str) -> String {
    let mut s: String = segment
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        return "_".into();
    }
    let stem = s
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        s.insert(0, '_');
    }
    // Long names: most file systems take 255 bytes per name.
    if s.len() > 200 {
        let mut end = 200;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// `rel` (segments joined by `/`) as safe local segments.
fn local_segments(rel: &str) -> Vec<String> {
    rel.split('/').map(local_segment).collect()
}

/// Numbers the later of two local paths that clash (also when they differ
/// only in case, for Windows and macOS file systems). A file whose path is
/// also a folder's (the keys `a/b` and `a/b/c` can both exist) is numbered
/// too, so the folder can be created. `dirs[i]` says whether `paths[i]` is
/// a folder (a marker).
fn dedupe(paths: &mut [Vec<String>], dirs: &[bool]) {
    let lower = |p: &[String]| p.join("/").to_lowercase();
    // Every folder some path needs: its parents, and markers themselves.
    let mut folders: HashSet<String> = HashSet::new();
    for (p, &dir) in paths.iter().zip(dirs) {
        let end = if dir {
            p.len()
        } else {
            p.len().saturating_sub(1)
        };
        for n in 1..=end {
            folders.insert(lower(&p[..n]));
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    for (p, &dir) in paths.iter_mut().zip(dirs) {
        if dir {
            // Folders merge; nothing to number.
            continue;
        }
        let mut candidate = p.clone();
        let mut n = 1;
        loop {
            let key = lower(&candidate);
            if !folders.contains(&key) && taken.insert(key) {
                break;
            }
            if let Some(last) = candidate.last_mut() {
                *last = super::operations::numbered_name(p.last().map_or("_", String::as_str), n);
            }
            n += 1;
        }
        *p = candidate;
    }
}

// ------------------------------------------------------------------ events

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileState {
    Pending,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkFile {
    pub index: usize,
    /// The item it belongs to.
    pub item: usize,
    /// Its local path below the chosen folder, `/` between names.
    pub name: String,
    pub key: String,
    pub size: u64,
    pub state: FileState,
    pub bytes: u64,
    pub integrity: Option<Integrity>,
    pub error: Option<CommandError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemState {
    Pending,
    Downloading,
    /// Every file verified and moved into place (`saved_as`).
    Saved,
    /// A file failed; nothing of the item was kept.
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkItem {
    pub index: usize,
    pub name: String,
    pub folder: bool,
    pub files: u64,
    pub bytes: u64,
    pub state: ItemState,
    /// The name it was saved under in the chosen folder (`name (n)` if its
    /// own name was taken).
    pub saved_as: Option<String>,
    pub error: Option<CommandError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchState {
    Completed,
    /// Some items failed.
    Partial,
    Cancelled,
    /// Stopped by an error that would fail the rest too.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkSummary {
    pub batch: String,
    pub state: BatchState,
    pub items_saved: u32,
    pub items_failed: u32,
    pub items_cancelled: u32,
    pub files_completed: u64,
    pub bytes: u64,
    pub retryable: bool,
    pub error: Option<CommandError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum BulkEventBody {
    File { file: BulkFile },
    Item { item: BulkItem },
    Finished { summary: BulkSummary },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BulkEvent {
    pub batch: String,
    pub seq: u64,
    pub body: BulkEventBody,
}

/// What a started batch is, as the page first shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkStarted {
    pub batch: String,
    /// The chosen folder's own name (not its path).
    pub folder_name: String,
    pub items: Vec<BulkItem>,
    pub files: Vec<BulkFile>,
}

pub trait BulkSink: Send + Sync + 'static {
    fn emit(&self, event: BulkEvent);
}

impl BulkSink for Channel<BulkEvent> {
    fn emit(&self, event: BulkEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: failed to send a download event to the dashboard: {e}");
        }
    }
}

/// How a failed file is shown: never a local path.
fn file_error(error: &StorageError) -> CommandError {
    match error {
        StorageError::LocalWrite { kind, .. } => {
            let (k, m) = match kind {
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => {
                    (ErrorKind::DiskFull, "there is not enough free space")
                }
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => (
                    ErrorKind::LocalPermission,
                    "the file may not be written to the chosen folder",
                ),
                std::io::ErrorKind::NotFound => (
                    ErrorKind::DestinationUnavailable,
                    "the chosen folder is not available",
                ),
                _ => (ErrorKind::LocalFile, "the file could not be written"),
            };
            CommandError::new(k, m)
        }
        StorageError::Io(_) => {
            CommandError::new(ErrorKind::LocalFile, "the file could not be written")
        }
        StorageError::Integrity(_) => CommandError::new(
            ErrorKind::Integrity,
            "the downloaded data does not match the file's size or stored SHA-256 checksum",
        ),
        StorageError::NotFound(_) => CommandError::new(
            ErrorKind::ObjectNotFound,
            "the file is no longer in the bucket",
        ),
        other => operation_error(other),
    }
}

fn stops_batch(error: &StorageError) -> bool {
    match error {
        StorageError::InvalidCredentials(_)
        | StorageError::AccessDenied(_)
        | StorageError::Network(_)
        | StorageError::Timeout(_)
        | StorageError::Refused(_) => true,
        StorageError::LocalWrite { kind, .. } => matches!(
            kind,
            std::io::ErrorKind::StorageFull
                | std::io::ErrorKind::QuotaExceeded
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::ReadOnlyFilesystem
                | std::io::ErrorKind::NotFound
        ),
        StorageError::Provider { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

// ------------------------------------------------------------------ plans

/// One file to download.
#[derive(Clone, Debug)]
struct PlannedFile {
    key: String,
    size: u64,
    /// Local path below the item's own folder (or the item's name for a
    /// file item), as segments.
    local: Vec<String>,
    /// A folder marker: only a folder is created.
    marker: bool,
}

#[derive(Clone, Debug)]
struct PlannedItem {
    name: String,
    folder: bool,
    /// The safe local name of the item.
    local_name: String,
    files: Vec<PlannedFile>,
    /// For a retry: the original item to download again.
    source: ItemRef,
}

async fn plan_items(
    store: &S3Store,
    bucket: &str,
    items: Vec<ItemRef>,
) -> Result<Vec<PlannedItem>, CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m.to_owned());
    if items.is_empty() {
        return Err(invalid("nothing is selected"));
    }
    if items.len() > MAX_ITEMS {
        return Err(invalid("at most 1,000 items at a time"));
    }
    let mut out: Vec<PlannedItem> = Vec::new();
    let mut total = 0usize;
    for item in items {
        match &item {
            ItemRef::File { key } => {
                if key.is_empty() || key.len() > MAX_KEY_BYTES || key.ends_with('/') {
                    return Err(invalid(
                        "a file key must be 1 to 1,024 bytes and not end in '/'",
                    ));
                }
                let props = store
                    .object_properties(bucket, key)
                    .await
                    .map_err(|e| operation_error(&e))?
                    .ok_or_else(|| {
                        CommandError::new(
                            ErrorKind::ObjectNotFound,
                            format!("'{}' is no longer in the bucket", file_name(key)),
                        )
                    })?;
                let local = local_segment(file_name(key));
                total += 1;
                out.push(PlannedItem {
                    name: file_name(key).to_owned(),
                    folder: false,
                    local_name: local.clone(),
                    files: vec![PlannedFile {
                        key: key.clone(),
                        size: props.size,
                        local: vec![local],
                        marker: false,
                    }],
                    source: item.clone(),
                });
            }
            ItemRef::Folder { prefix } => {
                if prefix.is_empty() || !prefix.ends_with('/') || prefix.len() > MAX_KEY_BYTES {
                    return Err(invalid("a folder must be a prefix ending in '/'"));
                }
                let local_name = local_segment(folder_name(prefix));
                let mut files = Vec::new();
                let mut token: Option<String> = None;
                loop {
                    let page = store
                        .list_recursive_page(bucket, prefix, token.as_deref(), 1000)
                        .await
                        .map_err(|e| operation_error(&e))?;
                    let before = files.len();
                    for o in page
                        .objects
                        .into_iter()
                        .filter(|o| in_folder(prefix, &o.key))
                    {
                        let rel = &o.key[prefix.len()..];
                        let marker = o.key.ends_with('/');
                        let rel = rel.strip_suffix('/').unwrap_or(rel);
                        let mut local = vec![local_name.clone()];
                        if !rel.is_empty() {
                            local.extend(local_segments(rel));
                        }
                        files.push(PlannedFile {
                            key: o.key,
                            size: o.size,
                            local,
                            marker,
                        });
                    }
                    // This page's files only: the earlier pages are counted.
                    total += files.len() - before;
                    if total > MAX_FILES {
                        return Err(invalid(
                            "more than 10,000 files are selected; download fewer at a time",
                        ));
                    }
                    match page.next_token {
                        Some(t) => token = Some(t),
                        None => break,
                    }
                }
                let mut paths: Vec<Vec<String>> = files.iter().map(|f| f.local.clone()).collect();
                let dirs: Vec<bool> = files.iter().map(|f| f.marker).collect();
                dedupe(&mut paths, &dirs);
                for (f, p) in files.iter_mut().zip(paths) {
                    f.local = p;
                }
                out.push(PlannedItem {
                    name: folder_name(prefix).to_owned(),
                    folder: true,
                    local_name,
                    files,
                    source: item.clone(),
                });
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- the slot

enum Slot {
    Free,
    Choosing,
    Active {
        id: String,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        current: Arc<Mutex<Option<Arc<TransferControl>>>>,
    },
}

struct Retry {
    batch: String,
    account: String,
    bucket: String,
    dest: PathBuf,
    items: Vec<ItemRef>,
}

struct Inner {
    slot: Slot,
    retry: Option<Retry>,
    next: u64,
    closed: bool,
}

pub struct BulkDownloads {
    inner: Mutex<Inner>,
    freed: Condvar,
}

impl Default for BulkDownloads {
    fn default() -> Self {
        Self::new()
    }
}

static STAGING: AtomicU64 = AtomicU64::new(0);

/// A hidden staging folder inside `dest`.
fn staging_dir(dest: &Path) -> std::io::Result<PathBuf> {
    for _ in 0..100 {
        let n = STAGING.fetch_add(1, Ordering::Relaxed);
        let path = dest.join(format!(".sotyware-download-{}-{n}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free staging folder name",
    ))
}

/// Moves `from` to `dest/name`, or the first free `name (n)`; never
/// replaces anything. Returns the name used.
fn move_into_place(from: &Path, dest: &Path, name: &str) -> std::io::Result<String> {
    for n in 0..1000u32 {
        let candidate = if n == 0 {
            name.to_owned()
        } else {
            super::operations::numbered_name(name, n)
        };
        let target = dest.join(&candidate);
        if target.symlink_metadata().is_ok() {
            continue;
        }
        // A hard link for a file refuses an existing target atomically;
        // folders are renamed (checked just before).
        if from.is_file() {
            match std::fs::hard_link(from, &target) {
                Ok(()) => {
                    let _ = std::fs::remove_file(from);
                    return Ok(candidate);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {}
            }
        }
        std::fs::rename(from, &target)?;
        return Ok(candidate);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free name",
    ))
}

impl BulkDownloads {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slot: Slot::Free,
                retry: None,
                next: 0,
                closed: false,
            }),
            freed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reserve(&self) -> Result<(), CommandError> {
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
                Ok(())
            }
            _ => Err(CommandError::new(
                ErrorKind::Busy,
                "another download of several files is running",
            )),
        }
    }

    fn release(&self) {
        let mut inner = self.lock();
        if matches!(inner.slot, Slot::Choosing) {
            inner.slot = Slot::Free;
        }
        drop(inner);
        self.freed.notify_all();
    }

    pub fn busy(&self) -> bool {
        !matches!(self.lock().slot, Slot::Free)
    }

    /// Starts downloading `items` into the folder `pick` returns.
    pub async fn begin(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        account: String,
        bucket: String,
        items: Vec<ItemRef>,
        pick: impl std::future::Future<Output = Option<PathBuf>>,
        sink: Arc<dyn BulkSink>,
    ) -> Result<Option<BulkStarted>, CommandError> {
        self.reserve()?;
        let result = async {
            let store = {
                let (accounts, account, bucket) =
                    (Arc::clone(&accounts), account.clone(), bucket.clone());
                blocking(move || accounts.prepare_download(&account, &bucket)).await?
            };
            let planned = plan_items(&store, &bucket, items).await?;
            let Some(dest) = pick.await else {
                return Ok(None);
            };
            Ok(Some((store, planned, dest)))
        }
        .await;
        match result {
            Ok(Some((store, planned, dest))) => self
                .launch(store, account, bucket, dest, planned, sink)
                .map(Some),
            Ok(None) => {
                self.release();
                Ok(None)
            }
            Err(e) => {
                self.release();
                Err(e)
            }
        }
    }

    /// Downloads what batch `batch` left (failed and cancelled items) again,
    /// into the same folder.
    pub async fn retry(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        batch: &str,
        sink: Arc<dyn BulkSink>,
    ) -> Result<BulkStarted, CommandError> {
        let retry = {
            let mut inner = self.lock();
            match inner.retry.take() {
                Some(r) if r.batch == batch => r,
                other => {
                    inner.retry = other;
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "this download can no longer be retried",
                    ));
                }
            }
        };
        let items = retry.items.clone();
        let (account, bucket, dest) = (
            retry.account.clone(),
            retry.bucket.clone(),
            retry.dest.clone(),
        );
        match self
            .begin(
                accounts,
                account,
                bucket,
                items,
                async move { Some(dest) },
                sink,
            )
            .await
        {
            Ok(Some(started)) => Ok(started),
            Ok(None) => Err(CommandError::new(ErrorKind::Other, "nothing started")),
            Err(e) => {
                let mut inner = self.lock();
                if inner.retry.is_none() {
                    inner.retry = Some(retry);
                }
                Err(e)
            }
        }
    }

    fn launch(
        self: &Arc<Self>,
        store: Arc<S3Store>,
        account: String,
        bucket: String,
        dest: PathBuf,
        planned: Vec<PlannedItem>,
        sink: Arc<dyn BulkSink>,
    ) -> Result<BulkStarted, CommandError> {
        let id = {
            let mut inner = self.lock();
            inner.next += 1;
            format!("bulk-{}-{}", std::process::id(), inner.next)
        };
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let current: Arc<Mutex<Option<Arc<TransferControl>>>> = Arc::default();
        {
            let mut inner = self.lock();
            if inner.closed {
                inner.slot = Slot::Free;
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "sotyware is quitting; no download can start",
                ));
            }
            inner.slot = Slot::Active {
                id: id.clone(),
                cancelled: Arc::clone(&cancelled),
                current: Arc::clone(&current),
            };
            inner.retry = None;
        }
        let mut items = Vec::new();
        let mut files = Vec::new();
        for (i, p) in planned.iter().enumerate() {
            items.push(BulkItem {
                index: i,
                name: p.name.clone(),
                folder: p.folder,
                files: p.files.len() as u64,
                bytes: p.files.iter().map(|f| f.size).sum(),
                state: ItemState::Pending,
                saved_as: None,
                error: None,
            });
            for f in &p.files {
                files.push(BulkFile {
                    index: files.len(),
                    item: i,
                    name: f.local.join("/"),
                    key: f.key.clone(),
                    size: f.size,
                    state: FileState::Pending,
                    bytes: 0,
                    integrity: None,
                    error: None,
                });
            }
        }
        let started = BulkStarted {
            batch: id.clone(),
            folder_name: dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            items: items.clone(),
            files: files.clone(),
        };
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut seq = 0u64;
            let mut emit = |body: BulkEventBody| {
                seq += 1;
                sink.emit(BulkEvent {
                    batch: id.clone(),
                    seq,
                    body,
                });
            };
            let mut stop: Option<CommandError> = None;
            let staging = match tokio::task::spawn_blocking({
                let dest = dest.clone();
                move || staging_dir(&dest)
            })
            .await
            {
                Ok(Ok(s)) => Some(s),
                _ => {
                    stop = Some(CommandError::new(
                        ErrorKind::DestinationUnavailable,
                        "the chosen folder cannot be written",
                    ));
                    None
                }
            };
            let mut file_base = 0usize;
            for (i, p) in planned.iter().enumerate() {
                let range = file_base..file_base + p.files.len();
                file_base += p.files.len();
                let Some(staging) = staging
                    .as_ref()
                    .filter(|_| stop.is_none() && !cancelled.load(Ordering::SeqCst))
                else {
                    items[i].state = ItemState::Cancelled;
                    for f in &mut files[range] {
                        f.state = FileState::Cancelled;
                        emit(BulkEventBody::File { file: f.clone() });
                    }
                    emit(BulkEventBody::Item {
                        item: items[i].clone(),
                    });
                    continue;
                };
                items[i].state = ItemState::Downloading;
                emit(BulkEventBody::Item {
                    item: items[i].clone(),
                });
                let mut failed: Option<CommandError> = None;
                for (pf, fi) in p.files.iter().zip(range.clone()) {
                    if cancelled.load(Ordering::SeqCst) || stop.is_some() || failed.is_some() {
                        files[fi].state = FileState::Cancelled;
                        emit(BulkEventBody::File {
                            file: files[fi].clone(),
                        });
                        continue;
                    }
                    let target = pf.local.iter().fold(staging.clone(), |acc, s| acc.join(s));
                    if pf.marker {
                        let ok = tokio::fs::create_dir_all(&target).await.is_ok();
                        files[fi].state = if ok {
                            FileState::Completed
                        } else {
                            FileState::Failed
                        };
                        if !ok {
                            failed = Some(CommandError::new(
                                ErrorKind::LocalFile,
                                "a folder could not be created",
                            ));
                        }
                        emit(BulkEventBody::File {
                            file: files[fi].clone(),
                        });
                        continue;
                    }
                    if let Some(parent) = target.parent()
                        && tokio::fs::create_dir_all(parent).await.is_err()
                    {
                        files[fi].state = FileState::Failed;
                        let e = CommandError::new(
                            ErrorKind::LocalFile,
                            "a folder could not be created",
                        );
                        files[fi].error = Some(e.clone());
                        failed = Some(e);
                        emit(BulkEventBody::File {
                            file: files[fi].clone(),
                        });
                        continue;
                    }
                    files[fi].state = FileState::Downloading;
                    emit(BulkEventBody::File {
                        file: files[fi].clone(),
                    });
                    let control = Arc::new(TransferControl::new());
                    *current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&control));
                    if cancelled.load(Ordering::SeqCst) {
                        control.cancel();
                    }
                    let mut last = Instant::now();
                    let mut shown = files[fi].clone();
                    let result = {
                        let emit = &mut emit;
                        let mut on_progress = |bytes: u64, _total: Option<u64>| {
                            if last.elapsed() >= PROGRESS_EVERY {
                                last = Instant::now();
                                shown.bytes = bytes;
                                emit(BulkEventBody::File {
                                    file: shown.clone(),
                                });
                            }
                        };
                        store
                            .download(
                                &bucket,
                                &pf.key,
                                &target,
                                Existing::Refuse,
                                &control,
                                &mut on_progress,
                            )
                            .await
                    };
                    *current.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    match result {
                        Ok(report) => {
                            files[fi].state = FileState::Completed;
                            files[fi].bytes = report.size;
                            files[fi].integrity = Some(match report.verification {
                                Verification::Sha256Matched => Integrity::Sha256Verified,
                                Verification::SizeOnly => Integrity::SizeOnly,
                            });
                        }
                        Err(StorageError::Cancelled) => {
                            files[fi].state = FileState::Cancelled;
                        }
                        Err(e) => {
                            let err = file_error(&e);
                            files[fi].state = FileState::Failed;
                            files[fi].error = Some(err.clone());
                            if stops_batch(&e) {
                                stop = Some(err.clone());
                            }
                            failed = Some(err);
                        }
                    }
                    emit(BulkEventBody::File {
                        file: files[fi].clone(),
                    });
                }
                let item_dir = staging.join(&p.local_name);
                let all_done = files[range.clone()]
                    .iter()
                    .all(|f| f.state == FileState::Completed);
                if all_done {
                    let (dest, from, name) = (dest.clone(), item_dir.clone(), p.local_name.clone());
                    match tokio::task::spawn_blocking(move || {
                        if !p_exists(&from) {
                            // A folder of markers only: create it.
                            std::fs::create_dir_all(&from)?;
                        }
                        move_into_place(&from, &dest, &name)
                    })
                    .await
                    {
                        Ok(Ok(saved)) => {
                            items[i].state = ItemState::Saved;
                            items[i].saved_as = Some(saved);
                        }
                        _ => {
                            items[i].state = ItemState::Failed;
                            items[i].error = Some(CommandError::new(
                                ErrorKind::LocalFile,
                                "the download could not be moved into the chosen folder",
                            ));
                        }
                    }
                } else {
                    let from = item_dir.clone();
                    let _ = tokio::task::spawn_blocking(move || remove_any(&from)).await;
                    items[i].state = if failed.is_some() {
                        ItemState::Failed
                    } else {
                        ItemState::Cancelled
                    };
                    items[i].error = failed;
                }
                emit(BulkEventBody::Item {
                    item: items[i].clone(),
                });
            }
            if let Some(s) = staging {
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&s)).await;
            }
            let count = |s: ItemState| items.iter().filter(|i| i.state == s).count() as u32;
            let (saved, failed, cancelled_items) = (
                count(ItemState::Saved),
                count(ItemState::Failed),
                count(ItemState::Cancelled),
            );
            let left: Vec<ItemRef> = planned
                .iter()
                .zip(&items)
                .filter(|(_, it)| matches!(it.state, ItemState::Failed | ItemState::Cancelled))
                .map(|(p, _)| p.source.clone())
                .collect();
            let summary = BulkSummary {
                batch: id.clone(),
                state: if stop.is_some() {
                    BatchState::Failed
                } else if cancelled.load(Ordering::SeqCst) && cancelled_items + failed > 0 {
                    // A cancel after the last item undid nothing.
                    BatchState::Cancelled
                } else if failed > 0 {
                    BatchState::Partial
                } else {
                    BatchState::Completed
                },
                items_saved: saved,
                items_failed: failed,
                items_cancelled: cancelled_items,
                files_completed: files
                    .iter()
                    .filter(|f| f.state == FileState::Completed)
                    .count() as u64,
                bytes: files
                    .iter()
                    .filter(|f| f.state == FileState::Completed)
                    .map(|f| f.bytes)
                    .sum(),
                retryable: !left.is_empty(),
                error: stop,
            };
            {
                let mut inner = this.lock();
                inner.slot = Slot::Free;
                if !left.is_empty() {
                    inner.retry = Some(Retry {
                        batch: id.clone(),
                        account,
                        bucket,
                        dest,
                        items: left,
                    });
                }
            }
            this.freed.notify_all();
            emit(BulkEventBody::Finished { summary });
        });
        Ok(started)
    }

    pub fn cancel(&self, batch: &str) -> CancelOutcome {
        let inner = self.lock();
        match &inner.slot {
            Slot::Active {
                id,
                cancelled,
                current,
            } if id == batch => {
                cancelled.store(true, Ordering::SeqCst);
                if let Some(c) = current.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    c.cancel();
                }
                CancelOutcome::Cancelling
            }
            _ => CancelOutcome::NotActive,
        }
    }

    pub fn shutdown(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut inner = self.lock();
        inner.closed = true;
        if let Slot::Active {
            cancelled, current, ..
        } = &inner.slot
        {
            cancelled.store(true, Ordering::SeqCst);
            if let Some(c) = current.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                c.cancel();
            }
        }
        while matches!(inner.slot, Slot::Active { .. }) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                eprintln!(
                    "sotyware: a download of several files was still running when sotyware quit; its hidden staging folder may remain"
                );
                return;
            }
            inner = self
                .freed
                .wait_timeout(inner, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

fn p_exists(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

fn remove_any(p: &Path) -> std::io::Result<()> {
    match p.symlink_metadata() {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(_) => Ok(()),
    }
}

// ---------------------------------------------------------------- commands

#[cfg(desktop)]
async fn choose_folder(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Choose a folder to download into")
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

/// Asks for a folder in the native folder dialog and downloads the
/// selected files and folders into it. `null` if the dialog was
/// cancelled. Allowed by `allow-download-cloud-items`.
#[tauri::command]
pub async fn download_cloud_items(
    window: tauri::WebviewWindow,
    accounts: State<'_, Arc<CloudAccounts>>,
    bulk: State<'_, Arc<BulkDownloads>>,
    id: String,
    bucket: String,
    items: Vec<ItemRef>,
    on_event: Channel<BulkEvent>,
) -> Result<Option<BulkStarted>, CommandError> {
    Arc::clone(&bulk)
        .begin(
            Arc::clone(&accounts),
            id,
            bucket,
            items,
            choose_folder(&window),
            Arc::new(on_event),
        )
        .await
}

/// Cancels a running download of several files. Allowed by
/// `allow-cancel-cloud-items-download`.
#[tauri::command]
pub fn cancel_cloud_items_download(
    bulk: State<'_, Arc<BulkDownloads>>,
    batch: String,
) -> CancelOutcome {
    bulk.cancel(&batch)
}

/// Downloads the failed and cancelled items of a finished batch again.
/// Allowed by `allow-retry-cloud-items-download`.
#[tauri::command]
pub async fn retry_cloud_items_download(
    accounts: State<'_, Arc<CloudAccounts>>,
    bulk: State<'_, Arc<BulkDownloads>>,
    batch: String,
    on_event: Channel<BulkEvent>,
) -> Result<BulkStarted, CommandError> {
    Arc::clone(&bulk)
        .retry(Arc::clone(&accounts), &batch, Arc::new(on_event))
        .await
}

#[cfg(test)]
mod tests;

//! Remote file management (ADR-026): copying, moving, renaming and
//! deleting files and folders inside an account's configured bucket, and
//! restoring deleted files, as reviewed plans that run in the background:
//! the `plan_cloud_operation`, `discard_cloud_operation_plan`,
//! `start_cloud_operation`, `cancel_cloud_operation` and
//! `retry_cloud_operation` commands, the events they send, and the one
//! operation slot behind them.
//!
//! S3 has no folders and no rename. So:
//!
//! - **Folders are prefixes** ending in `/`. A folder item is every object
//!   whose key starts with exactly that prefix (`photos/` never includes
//!   `photos-backup/x`), its marker object (key = prefix) included if it
//!   exists. They are enumerated once, at planning time (recursive
//!   `ListObjectsV2`), with their ETags; at most [`MAX_OBJECTS`] per plan.
//!   Objects that appear later are not part of the operation.
//! - **Plan, then start.** Planning sends only reads (`HeadObject`,
//!   listings, `GetBucketVersioning`, `ListObjectVersions`) and returns a
//!   [`PlanSummary`]: every item with its object count, size, destination
//!   and conflict, so the page can show them before anything changes. The
//!   plan stays in Rust (keys, ETags, version ids); the page gets its id,
//!   valid for [`PLAN_TTL`] and only until another plan replaces it.
//! - **Copy** is a server-side `CopyObject` per object, all metadata
//!   copied (content type, cache and encoding headers, user metadata
//!   including sotyware's SHA-256), conditional on the ETag seen when
//!   planning (`x-amz-copy-source-if-match`): an object changed since is
//!   not copied ([`ErrorKind::RemoteChanged`]). Right before each copy a
//!   `HeadObject` on the destination: an existing object is never
//!   overwritten (the object is skipped). Each copy is verified by a
//!   `HeadObject`: same size, and the same ETag when the source's ETag is a
//!   plain MD5 (single-part). Objects over 5 GiB are refused
//!   ([`MAX_COPY_BYTES`]).
//! - **Move and rename** are copy, verify, then delete the source, never
//!   before every object of the item is copied and verified. Each source is
//!   checked again (`HeadObject`: same ETag as planned) right before its
//!   delete; one that changed is kept. If a copy fails or the operation is
//!   cancelled before the delete phase, no source of that item is deleted.
//!   If a delete fails, the item is `partial`: the copy is complete and the
//!   original kept; retry deletes only the originals that are left, after
//!   checking their copies again.
//! - **Delete** is a `DeleteObject` without a version id for each planned
//!   key (after a `HeadObject` that its ETag is still the planned one). On
//!   a versioned bucket the provider adds a delete marker and keeps the
//!   versions (the file can be restored); otherwise the object is gone. The
//!   provider's answer says which, per object, and the report counts both.
//!   No version is ever deleted by this module except delete markers by a
//!   restore.
//! - **Restore** removes exactly the delete markers newer than a key's
//!   newest version (by version id, from `ListObjectVersions` at planning
//!   time), which makes that version current again, then checks with a
//!   `HeadObject` that the object is back.
//! - **Conflicts** (an existing destination) are shown in the plan. The
//!   page starts with [`ConflictPolicy::Skip`] (those items are not
//!   touched) or [`ConflictPolicy::Rename`] (the item goes to the first free
//!   `name (n)`, computed while planning and checked again). A folder is
//!   never merged into an existing one.
//! - **One operation at a time**, in its own slot (uploads and downloads
//!   have theirs). Within an item, at most [`CONCURRENCY`] requests at once.
//! - **Cancel** stops after the requests in flight (a server-side copy or
//!   delete that was sent is not taken back): items not started are
//!   `cancelled`, a move that was cancelled before its delete phase keeps
//!   its originals. **Shutdown** cancels and waits at most
//!   [`SHUTDOWN_WAIT`].
//! - **Errors** that would fail every further request (credentials, access,
//!   bucket, connection, timeouts, provider 5xx/429) stop the operation;
//!   others fail the object and the item goes on.
//! - **Retry** starts a new operation with what the finished one left:
//!   objects that failed with a retryable error or never started, and
//!   originals of moves that were copied but not deleted. A destination
//!   that exists by then counts as copied only if its ETag and size equal
//!   the source's (a copy whose answer was lost); otherwise it is skipped.
//!   Conflicts, changed objects and integrity failures are never retried.
//! - **Events** ([`OperationEvent`]) on the channel passed to start or
//!   retry, numbered by `seq`: `item` when an item changes, `progress`
//!   (at most every 250 ms), and exactly one `finished`, sent after the
//!   slot is free.

mod names;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

pub use names::{
    MAX_KEY_BYTES, file_name, folder_name, in_folder, name_problem, numbered as numbered_name,
    parent_of,
};

use super::browser::validate_prefix;
use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::download::CancelOutcome;
use super::s3::{MAX_COPY_BYTES, ObjectProperties, S3Store, StorageError, Versioning};

/// Most objects one plan may touch.
pub const MAX_OBJECTS: usize = 50_000;
/// Most items (selected files and folders) one plan may name.
pub const MAX_ITEMS: usize = 1_000;
/// How long a plan may be started after it was made.
pub const PLAN_TTL: Duration = Duration::from_secs(15 * 60);
/// Requests in flight at once within one item.
pub const CONCURRENCY: usize = 4;
/// How long quitting waits for a cancelled operation.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(15);
/// Page size of the recursive listings used for planning.
const LIST_PAGE: i32 = 1000;
/// Most object failures listed per item (the count is always exact).
const MAX_LISTED_FAILURES: usize = 20;
/// Least time between two progress events.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------- requests

/// A selected file or folder, as the page names it: exactly a key or a
/// prefix that a listing returned.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ItemRef {
    File { key: String },
    Folder { prefix: String },
}

/// A deleted file to restore: its key and the delete marker the deleted
/// items listing showed as its latest version.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRef {
    pub key: String,
    pub marker_version: String,
}

/// What the page asks to plan.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum OperationRequest {
    Copy {
        items: Vec<ItemRef>,
        destination: String,
    },
    Move {
        items: Vec<ItemRef>,
        destination: String,
    },
    Rename {
        item: ItemRef,
        new_name: String,
    },
    Delete {
        items: Vec<ItemRef>,
    },
    Restore {
        items: Vec<RestoreRef>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    Copy,
    Move,
    Rename,
    Delete,
    Restore,
}

impl OperationKind {
    fn moves(self) -> bool {
        matches!(self, Self::Move | Self::Rename)
    }
    fn copies(self) -> bool {
        matches!(self, Self::Copy | Self::Move | Self::Rename)
    }
}

/// What to do with items whose destination exists.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    /// Leave them alone.
    Skip,
    /// Use the free name the plan proposed (`name (n)`).
    Rename,
}

// ------------------------------------------------------------------- plans

/// An existing destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    /// The key or folder prefix that already exists.
    pub existing: String,
    /// The free name [`ConflictPolicy::Rename`] would use (its full key or
    /// prefix), if one was found.
    pub alternative: Option<String>,
}

/// One item of a plan, as the page shows it before starting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub index: usize,
    pub name: String,
    pub folder: bool,
    /// The file's key or the folder's prefix.
    pub source: String,
    /// Where it goes (copy, move, rename).
    pub destination: Option<String>,
    pub objects: u64,
    pub bytes: u64,
    pub conflict: Option<ConflictInfo>,
    /// Why the item cannot be part of the operation (it is left out).
    pub problem: Option<String>,
}

/// Whether deleting keeps versions, as far as sotyware could find out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VersioningInfo {
    Enabled,
    Suspended,
    Disabled,
    /// The key may not ask, or the provider does not say.
    Unknown,
}

/// A plan, as `plan_cloud_operation` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub plan: String,
    pub kind: OperationKind,
    pub account_id: String,
    pub bucket: String,
    pub destination: Option<String>,
    pub items: Vec<PlanItem>,
    /// Objects the operation would touch (items with a problem excluded).
    pub objects: u64,
    pub bytes: u64,
    pub conflicts: u32,
    pub versioning: VersioningInfo,
}

/// One object of an item and how far it got.
#[derive(Clone, Debug)]
struct Step {
    source: String,
    e_tag: Option<String>,
    size: u64,
    dest: Option<String>,
    /// Restore: the delete markers to remove, newest first.
    markers: Vec<String>,
    done: Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Done {
    /// Nothing done yet.
    No,
    /// Copied and verified (copy: final; move: the source is next).
    Copied,
    /// Move: copied, verified and the source deleted. Delete: deleted.
    /// Restore: restored.
    Finished {
        marker: Option<bool>,
    },
    /// Left out on purpose (an existing destination, a changed object).
    Skipped(CommandError),
    Failed(CommandError),
    /// Move: copied and verified, but the source was not deleted (the
    /// delete failed, or the source changed meanwhile). `retry` says
    /// whether a retry may try the delete again.
    OriginalKept {
        error: CommandError,
        retry: bool,
    },
}

struct ItemPlan {
    view: PlanItem,
    /// For a conflict: the steps re-targeted to the alternative.
    alternative_steps: Option<Vec<Step>>,
    steps: Vec<Step>,
}

struct StoredPlan {
    id: String,
    made: Instant,
    account: String,
    bucket: String,
    kind: OperationKind,
    items: Vec<ItemPlan>,
}

// ------------------------------------------------------------------ events

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemState {
    Pending,
    Running,
    Completed,
    /// Not touched (conflict, nothing left to do).
    Skipped,
    Failed,
    /// Part of it happened; `note` says which part.
    Partial,
    /// Not started, or stopped before anything happened.
    Cancelled,
}

impl ItemState {
    pub fn is_final(self) -> bool {
        self >= Self::Completed
    }
}

/// What a partial or finished item means for the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemNote {
    /// Skipped: its destination exists.
    DestinationExists,
    /// Move: not every object was copied; nothing was deleted, the
    /// original is complete. Copies already made stay in the destination.
    OriginalKeptCopyIncomplete,
    /// Move: every object was copied and verified, but some originals were
    /// not deleted (failed, changed meanwhile, or cancelled).
    CopiedOriginalKept,
    /// Copy or delete: some objects were done, some not.
    Incomplete,
}

/// One failed or skipped object of an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectFailure {
    pub key: String,
    pub error: CommandError,
}

/// One item while the operation runs, as the page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemReport {
    pub index: usize,
    pub name: String,
    pub folder: bool,
    pub source: String,
    /// Where it went (after a conflict rename, the new name).
    pub destination: Option<String>,
    pub state: ItemState,
    pub objects_total: u64,
    pub objects_done: u64,
    pub objects_failed: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    /// Delete: objects hidden behind a delete marker (restorable).
    pub hidden: u64,
    /// Delete: objects removed without a delete marker (not restorable).
    pub removed: u64,
    pub note: Option<ItemNote>,
    /// The first failures, at most 20 (`objects_failed` is the count).
    pub failures: Vec<ObjectFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    /// Every item reached `completed` or `skipped`.
    Completed,
    /// Some items failed or are partial; the others finished.
    Partial,
    Cancelled,
    /// Stopped by an error that would have failed the rest too.
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemCounts {
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub partial: u32,
    pub cancelled: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSummary {
    pub operation: String,
    pub kind: OperationKind,
    pub state: OperationState,
    pub items: ItemCounts,
    pub objects_done: u64,
    pub objects_failed: u64,
    pub hidden: u64,
    pub removed: u64,
    /// Whether `retry_cloud_operation` has anything to do.
    pub retryable: bool,
    /// What stopped the operation (state `failed`).
    pub error: Option<CommandError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum EventBody {
    Item {
        item: ItemReport,
    },
    Progress {
        objects_done: u64,
        objects_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    Finished {
        summary: OperationSummary,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationEvent {
    pub operation: String,
    pub seq: u64,
    pub body: EventBody,
}

/// What start and retry return.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStarted {
    pub operation: String,
    pub kind: OperationKind,
    pub items: Vec<ItemReport>,
}

/// Where events go: the page's channel, or a test recorder.
pub trait OperationSink: Send + Sync + 'static {
    fn emit(&self, event: OperationEvent);
}

impl OperationSink for Channel<OperationEvent> {
    fn emit(&self, event: OperationEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: failed to send a file operation event to the dashboard: {e}");
        }
    }
}

// ------------------------------------------------------------------ errors

/// How a failed request is shown: sotyware's own sentences, never the
/// provider's text.
pub fn operation_error(error: &StorageError) -> CommandError {
    let (kind, message) = match error {
        StorageError::InvalidCredentials(_) => (
            ErrorKind::InvalidCredentials,
            "the provider does not accept this key ID or secret".to_owned(),
        ),
        StorageError::AccessDenied(_) => (
            ErrorKind::AccessDenied,
            "the provider refused this change with this key".to_owned(),
        ),
        StorageError::NotFound(_) => (
            ErrorKind::ObjectNotFound,
            "the object no longer exists (or this key may not read it)".to_owned(),
        ),
        StorageError::Network(_) => (
            ErrorKind::Network,
            "the connection to the provider failed or broke off".to_owned(),
        ),
        StorageError::Timeout(_) => (
            ErrorKind::Timeout,
            "the provider did not answer in time".to_owned(),
        ),
        StorageError::Integrity(_) => (
            ErrorKind::Integrity,
            "the copy does not match the original's size or ETag".to_owned(),
        ),
        StorageError::AlreadyExists(_) => (
            ErrorKind::DestinationExists,
            "an object already exists at the destination; it was left as it is".to_owned(),
        ),
        StorageError::TooLarge { .. } => (
            ErrorKind::TooLarge,
            "objects larger than 5 GiB cannot be copied by sotyware".to_owned(),
        ),
        StorageError::Refused(m) => (ErrorKind::InvalidConfiguration, m.clone()),
        StorageError::Provider { status: 412, .. } => (
            ErrorKind::RemoteChanged,
            "the object changed since the operation was reviewed; it was left as it is".to_owned(),
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
                format!("the provider refused the request: HTTP {status}{code}")
            };
            (ErrorKind::Provider, message)
        }
        StorageError::Cancelled => (ErrorKind::Other, "cancelled".to_owned()),
        StorageError::Io(_) | StorageError::LocalWrite { .. } | StorageError::FileChanged(_) => {
            (ErrorKind::Other, "the request could not be made".to_owned())
        }
    };
    CommandError::new(kind, message)
}

/// Whether an error would fail every further request of the operation.
fn stops_operation(error: &StorageError) -> bool {
    match error {
        StorageError::InvalidCredentials(_)
        | StorageError::AccessDenied(_)
        | StorageError::Network(_)
        | StorageError::Timeout(_)
        | StorageError::Refused(_) => true,
        StorageError::Provider { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

/// Whether retrying may help without a change by the user.
fn retryable(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Network
            | ErrorKind::Timeout
            | ErrorKind::Provider
            | ErrorKind::InvalidCredentials
            | ErrorKind::AccessDenied
            | ErrorKind::CredentialsMissing
            | ErrorKind::CredentialStoreLocked
            | ErrorKind::Other
    )
}

fn changed(key: &str) -> CommandError {
    CommandError::new(
        ErrorKind::RemoteChanged,
        format!(
            "'{}' changed since the operation was reviewed; it was left as it is",
            file_name(key)
        ),
    )
}

fn exists_error() -> CommandError {
    operation_error(&StorageError::AlreadyExists(String::new()))
}

// --------------------------------------------------------------- planning

/// Every object under `prefix` (recursive), at most `room` of them.
async fn enumerate(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
    room: usize,
) -> Result<Vec<super::s3::ListedObject>, CommandError> {
    let mut all = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let page = store
            .list_recursive_page(bucket, prefix, token.as_deref(), LIST_PAGE)
            .await
            .map_err(|e| operation_error(&e))?;
        all.extend(
            page.objects
                .into_iter()
                .filter(|o| in_folder(prefix, &o.key)),
        );
        if all.len() > room {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "the operation would touch more than {MAX_OBJECTS} objects; choose fewer at a time"
                ),
            ));
        }
        match page.next_token {
            Some(t) => token = Some(t),
            None => return Ok(all),
        }
    }
}

/// Whether anything exists at the key or under the folder prefix `path`.
async fn occupied(store: &S3Store, bucket: &str, path: &str) -> Result<bool, CommandError> {
    if path.ends_with('/') {
        let page = store
            .list_recursive_page(bucket, path, None, 1)
            .await
            .map_err(|e| operation_error(&e))?;
        Ok(!page.objects.is_empty())
    } else {
        Ok(store
            .object_properties(bucket, path)
            .await
            .map_err(|e| operation_error(&e))?
            .is_some())
    }
}

/// The first free `name (n)` next to `taken_path` (a key or a folder
/// prefix), not among `planned`.
async fn alternative(
    store: &S3Store,
    bucket: &str,
    taken_path: &str,
    planned: &HashSet<String>,
) -> Result<Option<String>, CommandError> {
    let folder = taken_path.ends_with('/');
    let parent = parent_of(taken_path).to_owned();
    let name = if folder {
        folder_name(taken_path)
    } else {
        file_name(taken_path)
    }
    .to_owned();
    // At most 20 lookups: beyond that the user picks another name.
    for n in 1..=20u32 {
        let mut candidate = format!("{parent}{}", names::numbered(&name, n));
        if folder {
            candidate.push('/');
        }
        if candidate.len() > MAX_KEY_BYTES {
            return Ok(None);
        }
        let clashes = planned.contains(&candidate)
            || (folder && planned.iter().any(|p| p.starts_with(candidate.as_str())));
        if !clashes && !occupied(store, bucket, &candidate).await? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn steps_to(steps: &[Step], from: &str, to: &str) -> Vec<Step> {
    steps
        .iter()
        .map(|s| Step {
            dest: s.dest.as_ref().and_then(|d| names::relocate(d, from, to)),
            ..s.clone()
        })
        .collect()
}

fn single_step(props: &ObjectProperties, key: &str, dest: Option<String>) -> Step {
    Step {
        source: key.to_owned(),
        e_tag: props.e_tag.clone(),
        size: props.size,
        dest,
        markers: Vec::new(),
        done: Done::No,
    }
}

fn versioning_info(v: Result<Versioning, StorageError>) -> VersioningInfo {
    match v {
        Ok(Versioning::Enabled) => VersioningInfo::Enabled,
        Ok(Versioning::Suspended) => VersioningInfo::Suspended,
        Ok(Versioning::Disabled) => VersioningInfo::Disabled,
        Err(_) => VersioningInfo::Unknown,
    }
}

/// Plans `request` in `bucket` (reads only). Pure of state: the slot
/// stores what this returns.
async fn build_plan(
    store: &S3Store,
    bucket: &str,
    request: OperationRequest,
) -> Result<(OperationKind, Option<String>, Vec<ItemPlan>, VersioningInfo), CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m.to_owned());
    let (kind, items, destination, new_name, restores) = match request {
        OperationRequest::Copy { items, destination } => (
            OperationKind::Copy,
            items,
            Some(destination),
            None,
            Vec::new(),
        ),
        OperationRequest::Move { items, destination } => (
            OperationKind::Move,
            items,
            Some(destination),
            None,
            Vec::new(),
        ),
        OperationRequest::Rename { item, new_name } => (
            OperationKind::Rename,
            vec![item],
            None,
            Some(new_name),
            Vec::new(),
        ),
        OperationRequest::Delete { items } => {
            (OperationKind::Delete, items, None, None, Vec::new())
        }
        OperationRequest::Restore { items } => {
            (OperationKind::Restore, Vec::new(), None, None, items)
        }
    };
    if items.len() + restores.len() > MAX_ITEMS {
        return Err(invalid(&format!("at most {MAX_ITEMS} items at a time")));
    }
    if items.is_empty() && restores.is_empty() {
        return Err(invalid("nothing is selected"));
    }
    if let Some(d) = &destination {
        validate_prefix(d)?;
    }
    let mut files = Vec::new();
    let mut folders = Vec::new();
    for item in items {
        match item {
            ItemRef::File { key } => {
                if key.is_empty() || key.len() > MAX_KEY_BYTES || key.ends_with('/') {
                    return Err(invalid(
                        "a file key must be 1 to 1,024 bytes and not end in '/'",
                    ));
                }
                files.push(key);
            }
            ItemRef::Folder { prefix } => {
                if prefix.is_empty() || !prefix.ends_with('/') || prefix.len() > MAX_KEY_BYTES {
                    return Err(invalid(
                        "a folder must be a prefix ending in '/' (never the whole bucket)",
                    ));
                }
                folders.push(prefix);
            }
        }
    }
    let (files, folders) = names::without_nested(files, folders);

    let mut plans: Vec<ItemPlan> = Vec::new();
    let mut planned: HashSet<String> = HashSet::new();
    let mut total_objects = 0usize;

    // Rename: one item, the new name next to it.
    if kind == OperationKind::Rename {
        let new_name = new_name.unwrap_or_default();
        if let Some(p) = name_problem(&new_name) {
            return Err(invalid(p));
        }
        let (source, folder) = match (files.first(), folders.first()) {
            (Some(k), _) => (k.clone(), false),
            (None, Some(p)) => (p.clone(), true),
            _ => return Err(invalid("nothing is selected")),
        };
        let parent = parent_of(&source).to_owned();
        let dest = format!("{parent}{new_name}{}", if folder { "/" } else { "" });
        if dest == source {
            return Err(invalid("the new name is the same as the old one"));
        }
        if dest.len() > MAX_KEY_BYTES {
            return Err(invalid("the new key would be longer than 1,024 bytes"));
        }
        if occupied(store, bucket, &dest).await? {
            return Err(CommandError::new(
                ErrorKind::DestinationExists,
                format!("'{new_name}' already exists in this folder; choose another name"),
            ));
        }
        let steps = if folder {
            let objects = enumerate(store, bucket, &source, MAX_OBJECTS).await?;
            objects
                .into_iter()
                .map(|o| Step {
                    dest: names::relocate(&o.key, &source, &dest),
                    source: o.key,
                    e_tag: o.e_tag,
                    size: o.size,
                    markers: Vec::new(),
                    done: Done::No,
                })
                .collect::<Vec<_>>()
        } else {
            let props = store
                .object_properties(bucket, &source)
                .await
                .map_err(|e| operation_error(&e))?
                .ok_or_else(|| {
                    CommandError::new(ErrorKind::ObjectNotFound, "the file no longer exists")
                })?;
            vec![single_step(&props, &source, Some(dest.clone()))]
        };
        let problem = if steps.is_empty() {
            Some("the folder no longer holds any object".to_owned())
        } else if steps.iter().any(|s| s.size > MAX_COPY_BYTES) {
            Some("it holds an object larger than 5 GiB, which sotyware cannot copy".to_owned())
        } else {
            None
        };
        plans.push(ItemPlan {
            view: PlanItem {
                index: 0,
                name: if folder {
                    folder_name(&source)
                } else {
                    file_name(&source)
                }
                .to_owned(),
                folder,
                source,
                destination: Some(dest),
                objects: steps.len() as u64,
                bytes: steps.iter().map(|s| s.size).sum(),
                conflict: None,
                problem,
            },
            alternative_steps: None,
            steps,
        });
        let versioning = versioning_info(store.bucket_versioning(bucket).await);
        return Ok((kind, None, plans, versioning));
    }

    // Restore: per key, the delete markers newer than its newest version.
    if kind == OperationKind::Restore {
        for r in restores {
            if r.key.is_empty() || r.key.len() > MAX_KEY_BYTES {
                return Err(invalid("a file key must be 1 to 1,024 bytes"));
            }
            let mut entries = Vec::new();
            let mut marker: Option<(String, Option<String>)> = None;
            loop {
                let page = store
                    .list_versions_page(
                        bucket,
                        &r.key,
                        marker.as_ref().map(|(k, v)| (k.as_str(), v.as_deref())),
                        1000,
                    )
                    .await
                    .map_err(|e| operation_error(&e))?;
                entries.extend(page.entries.into_iter().filter(|e| e.key == r.key));
                match page.next {
                    // Past the key: the rest are other keys.
                    Some((k, v)) if k == r.key => marker = Some((k, v)),
                    _ => break,
                }
            }
            // Newest first: latest flag, then time.
            entries.sort_by(|a, b| {
                b.is_latest
                    .cmp(&a.is_latest)
                    .then(b.last_modified.cmp(&a.last_modified))
            });
            let markers: Vec<String> = entries
                .iter()
                .take_while(|e| e.delete_marker)
                .map(|e| e.version_id.clone())
                .collect();
            let newest = entries.iter().find(|e| !e.delete_marker);
            let problem = if markers.first() != Some(&r.marker_version) {
                Some(
                    "this file is not deleted any more, or was deleted again; refresh the list"
                        .to_owned(),
                )
            } else if newest.is_none() {
                Some("no earlier version of this file is kept, so it cannot be restored".to_owned())
            } else {
                None
            };
            let size = newest.map_or(0, |n| n.size);
            plans.push(ItemPlan {
                view: PlanItem {
                    index: plans.len(),
                    name: file_name(&r.key).to_owned(),
                    folder: false,
                    source: r.key.clone(),
                    destination: None,
                    objects: 1,
                    bytes: size,
                    conflict: None,
                    problem,
                },
                alternative_steps: None,
                steps: vec![Step {
                    source: r.key,
                    e_tag: None,
                    size,
                    dest: None,
                    markers,
                    done: Done::No,
                }],
            });
        }
        return Ok((kind, None, plans, VersioningInfo::Enabled));
    }

    // Copy, move and delete.
    for key in files {
        let props = store
            .object_properties(bucket, &key)
            .await
            .map_err(|e| operation_error(&e))?;
        let name = file_name(&key).to_owned();
        let Some(props) = props else {
            plans.push(ItemPlan {
                view: PlanItem {
                    index: plans.len(),
                    name,
                    folder: false,
                    source: key,
                    destination: None,
                    objects: 0,
                    bytes: 0,
                    conflict: None,
                    problem: Some("the file no longer exists".into()),
                },
                alternative_steps: None,
                steps: Vec::new(),
            });
            continue;
        };
        let dest = destination.as_ref().map(|d| format!("{d}{name}"));
        let mut problem = None;
        let mut conflict = None;
        let mut alternative_steps = None;
        let step = single_step(&props, &key, dest.clone());
        if let Some(dest) = &dest {
            if kind == OperationKind::Move && dest == &key {
                problem = Some("it is already in this folder".to_owned());
            } else if props.size > MAX_COPY_BYTES {
                problem = Some("it is larger than 5 GiB, which sotyware cannot copy".to_owned());
            } else if dest.len() > MAX_KEY_BYTES {
                problem = Some("its new key would be longer than 1,024 bytes".to_owned());
            } else if planned.contains(dest) || occupied(store, bucket, dest).await? {
                let alt = alternative(store, bucket, dest, &planned).await?;
                if let Some(a) = &alt {
                    alternative_steps = Some(vec![Step {
                        dest: Some(a.clone()),
                        ..step.clone()
                    }]);
                    planned.insert(a.clone());
                }
                conflict = Some(ConflictInfo {
                    existing: dest.clone(),
                    alternative: alt,
                });
            } else {
                planned.insert(dest.clone());
            }
        }
        total_objects += 1;
        plans.push(ItemPlan {
            view: PlanItem {
                index: plans.len(),
                name,
                folder: false,
                source: key,
                destination: dest,
                objects: 1,
                bytes: props.size,
                conflict,
                problem,
            },
            alternative_steps,
            steps: vec![step],
        });
    }
    for prefix in folders {
        let room = MAX_OBJECTS.saturating_sub(total_objects);
        let objects = enumerate(store, bucket, &prefix, room).await?;
        total_objects += objects.len();
        let name = folder_name(&prefix).to_owned();
        let dest = destination.as_ref().map(|d| format!("{d}{name}/"));
        let steps: Vec<Step> = objects
            .into_iter()
            .map(|o| Step {
                dest: dest
                    .as_ref()
                    .and_then(|d| names::relocate(&o.key, &prefix, d)),
                source: o.key,
                e_tag: o.e_tag,
                size: o.size,
                markers: Vec::new(),
                done: Done::No,
            })
            .collect();
        let mut problem = None;
        let mut conflict = None;
        let mut alternative_steps = None;
        if steps.is_empty() {
            problem = Some("the folder no longer holds any object".to_owned());
        } else if let Some(dest) = &dest {
            if names::into_itself(&prefix, dest) || (kind == OperationKind::Move && dest == &prefix)
            {
                problem = Some(if dest == &prefix && kind == OperationKind::Move {
                    "it is already in this folder".to_owned()
                } else {
                    "a folder cannot be put inside itself".to_owned()
                });
            } else if steps.iter().any(|s| s.size > MAX_COPY_BYTES) {
                problem = Some(
                    "it holds an object larger than 5 GiB, which sotyware cannot copy".to_owned(),
                );
            } else if steps
                .iter()
                .any(|s| s.dest.as_ref().is_none_or(|d| d.len() > MAX_KEY_BYTES))
            {
                problem = Some("a key inside it would become longer than 1,024 bytes".to_owned());
            } else if planned.iter().any(|p| p.starts_with(dest.as_str()))
                || occupied(store, bucket, dest).await?
            {
                let alt = alternative(store, bucket, dest, &planned).await?;
                if let Some(a) = &alt {
                    alternative_steps = Some(steps_to(&steps, dest, a));
                    planned.insert(a.clone());
                }
                conflict = Some(ConflictInfo {
                    existing: dest.clone(),
                    alternative: alt,
                });
            } else {
                planned.insert(dest.clone());
            }
        }
        plans.push(ItemPlan {
            view: PlanItem {
                index: plans.len(),
                name,
                folder: true,
                source: prefix,
                destination: dest,
                objects: steps.len() as u64,
                bytes: steps.iter().map(|s| s.size).sum(),
                conflict,
                problem,
            },
            alternative_steps,
            steps,
        });
    }
    let versioning = if kind.copies() && !kind.moves() {
        VersioningInfo::Unknown
    } else {
        versioning_info(store.bucket_versioning(bucket).await)
    };
    Ok((kind, destination, plans, versioning))
}

fn summary_of(
    plan: &StoredPlan,
    destination: Option<String>,
    versioning: VersioningInfo,
) -> PlanSummary {
    let usable = plan.items.iter().filter(|i| i.view.problem.is_none());
    PlanSummary {
        plan: plan.id.clone(),
        kind: plan.kind,
        account_id: plan.account.clone(),
        bucket: plan.bucket.clone(),
        destination,
        objects: usable.clone().map(|i| i.view.objects).sum(),
        bytes: usable.clone().map(|i| i.view.bytes).sum(),
        conflicts: usable.filter(|i| i.view.conflict.is_some()).count() as u32,
        items: plan.items.iter().map(|i| i.view.clone()).collect(),
        versioning,
    }
}

// --------------------------------------------------------------- execution

/// One item while it runs.
#[derive(Clone)]
struct Work {
    report: ItemReport,
    steps: Vec<Step>,
}

struct Control {
    cancelled: AtomicBool,
}

impl Control {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Numbers and sends events; throttles progress.
struct Emitter {
    operation: String,
    seq: u64,
    sink: Arc<dyn OperationSink>,
    last_progress: Option<Instant>,
}

impl Emitter {
    fn emit(&mut self, body: EventBody) {
        self.seq += 1;
        self.sink.emit(OperationEvent {
            operation: self.operation.clone(),
            seq: self.seq,
            body,
        });
    }

    fn item(&mut self, report: &ItemReport) {
        self.emit(EventBody::Item {
            item: report.clone(),
        });
    }

    fn progress(&mut self, totals: Totals, force: bool) {
        let now = Instant::now();
        if !force
            && self
                .last_progress
                .is_some_and(|t| now.duration_since(t) < PROGRESS_EVERY)
        {
            return;
        }
        self.last_progress = Some(now);
        self.emit(EventBody::Progress {
            objects_done: totals.0,
            objects_total: totals.1,
            bytes_done: totals.2,
            bytes_total: totals.3,
        });
    }
}

/// Objects done and total, bytes done and total.
#[derive(Clone, Copy, Default)]
struct Totals(u64, u64, u64, u64);

impl Totals {
    fn of(works: &[Work]) -> Self {
        works.iter().fold(Self::default(), |t, w| t.plus(&w.report))
    }
    fn plus(self, r: &ItemReport) -> Self {
        Self(
            self.0 + r.objects_done,
            self.1 + r.objects_total,
            self.2 + r.bytes_done,
            self.3 + r.bytes_total,
        )
    }
    fn minus(self, r: &ItemReport) -> Self {
        Self(
            self.0.saturating_sub(r.objects_done),
            self.1.saturating_sub(r.objects_total),
            self.2.saturating_sub(r.bytes_done),
            self.3.saturating_sub(r.bytes_total),
        )
    }
}

/// What one request step ended with.
enum StepResult {
    Done(Done),
    /// An error that stops the operation (the step failed too).
    Stop(CommandError),
}

fn md5_etag(e: &Option<String>) -> Option<&str> {
    e.as_deref()
        .filter(|e| e.len() == 32 && e.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Copies one object and verifies the copy (copy phase).
async fn copy_step(store: Arc<S3Store>, bucket: String, step: Step, retrying: bool) -> StepResult {
    let fail = |e: StorageError| {
        let err = operation_error(&e);
        if stops_operation(&e) {
            StepResult::Stop(err)
        } else {
            StepResult::Done(Done::Failed(err))
        }
    };
    let Some(dest) = step.dest.clone() else {
        return StepResult::Done(Done::Failed(CommandError::new(
            ErrorKind::Other,
            "no destination",
        )));
    };
    if step.size > MAX_COPY_BYTES {
        return fail(StorageError::TooLarge {
            size: step.size,
            max: MAX_COPY_BYTES,
        });
    }
    match store.object_properties(&bucket, &dest).await {
        Err(e) => return fail(e),
        Ok(Some(existing)) => {
            // On a retry, an earlier copy whose answer was lost.
            let ours = retrying
                && existing.size == step.size
                && existing.e_tag.is_some()
                && existing.e_tag == step.e_tag;
            return StepResult::Done(if ours {
                Done::Copied
            } else {
                Done::Skipped(exists_error())
            });
        }
        Ok(None) => {}
    }
    if let Err(e) = store
        .copy_object(&bucket, &step.source, None, step.e_tag.as_deref(), &dest)
        .await
    {
        return match e {
            StorageError::Provider { status: 412, .. } => {
                StepResult::Done(Done::Skipped(changed(&step.source)))
            }
            other => fail(other),
        };
    }
    match store.object_properties(&bucket, &dest).await {
        Err(e) => fail(e),
        Ok(None) => StepResult::Done(Done::Failed(operation_error(&StorageError::Integrity(
            "the copy is missing".into(),
        )))),
        Ok(Some(copy)) => {
            let etag_ok = match (md5_etag(&step.e_tag), copy.e_tag.as_deref()) {
                (Some(src), Some(dst)) => src == dst,
                _ => true,
            };
            if copy.size == step.size && etag_ok {
                StepResult::Done(Done::Copied)
            } else {
                StepResult::Done(Done::Failed(operation_error(&StorageError::Integrity(
                    String::new(),
                ))))
            }
        }
    }
}

/// Deletes one object after checking it is still the planned version
/// (delete, and a move's source phase).
async fn delete_step(
    store: Arc<S3Store>,
    bucket: String,
    step: Step,
    check_copy: bool,
) -> StepResult {
    let fail = |e: StorageError| {
        let err = operation_error(&e);
        if stops_operation(&e) {
            StepResult::Stop(err)
        } else {
            StepResult::Done(Done::Failed(err))
        }
    };
    // A move's retry: the copy must still be there and equal.
    if check_copy && let Some(dest) = &step.dest {
        match store.object_properties(&bucket, dest).await {
            Err(e) => return fail(e),
            Ok(Some(c))
                if c.size == step.size
                    && (md5_etag(&step.e_tag).is_none() || c.e_tag == step.e_tag) => {}
            Ok(_) => {
                return StepResult::Done(Done::Skipped(CommandError::new(
                    ErrorKind::RemoteChanged,
                    "the copy changed or is gone, so the original was kept",
                )));
            }
        }
    }
    match store.object_properties(&bucket, &step.source).await {
        Err(e) => return fail(e),
        Ok(None) => {
            return StepResult::Done(Done::Failed(CommandError::new(
                ErrorKind::ObjectNotFound,
                "the object was already gone",
            )));
        }
        Ok(Some(now)) => {
            if step.e_tag.is_some() && now.e_tag != step.e_tag {
                return StepResult::Done(Done::Skipped(changed(&step.source)));
            }
        }
    }
    match store.delete_reporting(&bucket, &step.source, None).await {
        Ok(outcome) => StepResult::Done(Done::Finished {
            marker: Some(outcome.marker_created),
        }),
        Err(e) => fail(e),
    }
}

/// Removes a restore's delete markers, newest first, and checks the
/// object is back.
async fn restore_step(store: Arc<S3Store>, bucket: String, step: Step) -> StepResult {
    let fail = |e: StorageError| {
        let err = operation_error(&e);
        if stops_operation(&e) {
            StepResult::Stop(err)
        } else {
            StepResult::Done(Done::Failed(err))
        }
    };
    for marker in &step.markers {
        if let Err(e) = store
            .delete_reporting(&bucket, &step.source, Some(marker))
            .await
        {
            return fail(e);
        }
    }
    match store.object_properties(&bucket, &step.source).await {
        Ok(Some(_)) => StepResult::Done(Done::Finished { marker: None }),
        Ok(None) => StepResult::Done(Done::Failed(CommandError::new(
            ErrorKind::RemoteChanged,
            "the file is still deleted (it was deleted again meanwhile)",
        ))),
        Err(e) => fail(e),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Copy,
    Delete,
    Restore,
}

/// Runs `phase` for the steps of `work` that need it, at most
/// [`CONCURRENCY`] at once. Returns the error that stops the operation.
#[allow(clippy::too_many_arguments)]
async fn run_phase(
    store: &Arc<S3Store>,
    bucket: &str,
    work: &mut Work,
    phase: Phase,
    kind: OperationKind,
    retrying: bool,
    control: &Control,
    emitter: &mut Emitter,
    others: Totals,
) -> Option<CommandError> {
    let todo: Vec<usize> = work
        .steps
        .iter()
        .enumerate()
        .filter(|(_, s)| match phase {
            Phase::Copy | Phase::Restore => s.done == Done::No,
            Phase::Delete => {
                if kind == OperationKind::Delete {
                    s.done == Done::No
                } else {
                    s.done == Done::Copied
                }
            }
        })
        .map(|(i, _)| i)
        .collect();
    let mut next = 0;
    let mut set = tokio::task::JoinSet::new();
    let mut stop: Option<CommandError> = None;
    loop {
        while stop.is_none() && !control.cancelled() && set.len() < CONCURRENCY && next < todo.len()
        {
            let i = todo[next];
            next += 1;
            let step = work.steps[i].clone();
            let (store, bucket) = (Arc::clone(store), bucket.to_owned());
            let check_copy = retrying && kind.moves();
            set.spawn(async move {
                let r = match phase {
                    Phase::Copy => copy_step(store, bucket, step, retrying).await,
                    Phase::Delete => delete_step(store, bucket, step, check_copy).await,
                    Phase::Restore => restore_step(store, bucket, step).await,
                };
                (i, r)
            });
        }
        let Some(joined) = set.join_next().await else {
            break;
        };
        let (i, result) = match joined {
            Ok(x) => x,
            Err(e) => {
                stop = Some(CommandError::new(
                    ErrorKind::Other,
                    format!("a request stopped unexpectedly: {e}"),
                ));
                continue;
            }
        };
        let done = match result {
            StepResult::Done(d) => d,
            StepResult::Stop(e) => {
                if stop.is_none() {
                    stop = Some(e.clone());
                }
                Done::Failed(e)
            }
        };
        let done = match done {
            Done::Failed(e) if phase == Phase::Delete && kind.moves() => Done::OriginalKept {
                retry: retryable(e.kind),
                error: e,
            },
            Done::Skipped(e) if phase == Phase::Delete && kind.moves() => Done::OriginalKept {
                error: e,
                retry: false,
            },
            other => other,
        };
        let step = &mut work.steps[i];
        let size = step.size;
        let source = step.source.clone();
        step.done = done.clone();
        let r = &mut work.report;
        match &done {
            Done::Copied if kind.moves() => {}
            Done::Copied | Done::Finished { .. } => {
                r.objects_done += 1;
                r.bytes_done += size;
                if let Done::Finished { marker: Some(m) } = done {
                    if m { r.hidden += 1 } else { r.removed += 1 }
                }
            }
            Done::Skipped(e) | Done::Failed(e) | Done::OriginalKept { error: e, .. } => {
                r.objects_failed += 1;
                if r.failures.len() < MAX_LISTED_FAILURES {
                    r.failures.push(ObjectFailure {
                        key: source,
                        error: e.clone(),
                    });
                }
            }
            Done::No => {}
        }
        emitter.progress(others.plus(&work.report), false);
    }
    stop
}

/// Sets the item's final state from its steps.
fn conclude(work: &mut Work, kind: OperationKind, cancelled: bool) {
    let steps = &work.steps;
    let total = steps.len();
    let finished = steps
        .iter()
        .filter(|s| matches!(s.done, Done::Finished { .. }))
        .count();
    let copied = steps
        .iter()
        .filter(|s| {
            matches!(
                s.done,
                Done::Copied | Done::Finished { .. } | Done::OriginalKept { .. }
            )
        })
        .count();
    let untouched = steps.iter().filter(|s| s.done == Done::No).count();
    let r = &mut work.report;
    r.note = None;
    if kind.moves() {
        if finished == total {
            r.state = ItemState::Completed;
        } else if copied == total {
            r.state = ItemState::Partial;
            r.note = Some(ItemNote::CopiedOriginalKept);
        } else if copied == 0 && finished == 0 && untouched == total && cancelled {
            r.state = ItemState::Cancelled;
        } else if copied == 0 && finished == 0 {
            r.state = ItemState::Failed;
            r.note = Some(ItemNote::OriginalKeptCopyIncomplete);
        } else {
            r.state = ItemState::Partial;
            r.note = Some(ItemNote::OriginalKeptCopyIncomplete);
        }
        return;
    }
    let good = if kind == OperationKind::Copy {
        copied
    } else {
        finished
    };
    if good == total {
        r.state = ItemState::Completed;
    } else if good == 0 && untouched == total && cancelled {
        r.state = ItemState::Cancelled;
    } else if good == 0 {
        r.state = ItemState::Failed;
    } else {
        r.state = ItemState::Partial;
        r.note = Some(ItemNote::Incomplete);
    }
}

// --------------------------------------------------------------------- slot

enum Slot {
    Free,
    Active { id: String, control: Arc<Control> },
}

/// What a finished operation left to retry.
struct RetrySpec {
    account: String,
    bucket: String,
    kind: OperationKind,
    works: Vec<Work>,
}

struct Inner {
    slot: Slot,
    plan: Option<StoredPlan>,
    retry: Option<(String, RetrySpec)>,
    next_id: u64,
    closed: bool,
}

#[cfg(test)]
type ItemHook = Box<dyn Fn(usize) + Send + Sync>;

/// The one file-operation slot, managed as Tauri state.
pub struct CloudOperations {
    inner: Mutex<Inner>,
    freed: Condvar,
    /// Runs before an item starts (index); tests cancel or change things here.
    #[cfg(test)]
    before_item: Mutex<Option<ItemHook>>,
}

impl Default for CloudOperations {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudOperations {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slot: Slot::Free,
                plan: None,
                retry: None,
                next_id: 0,
                closed: false,
            }),
            freed: Condvar::new(),
            #[cfg(test)]
            before_item: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn new_id(&self, what: &str) -> String {
        let mut inner = self.lock();
        inner.next_id += 1;
        format!("{what}-{}-{}", std::process::id(), inner.next_id)
    }

    #[cfg(test)]
    pub(crate) fn before_item(&self, f: impl Fn(usize) + Send + Sync + 'static) {
        *self.before_item.lock().unwrap() = Some(Box::new(f));
    }

    /// Plans `request` for account `account`, whose configured bucket must
    /// still be `bucket`, and keeps the plan (replacing any earlier one).
    pub async fn plan(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        account: String,
        bucket: String,
        request: OperationRequest,
    ) -> Result<PlanSummary, CommandError> {
        let store = {
            let (accounts, account, bucket) =
                (Arc::clone(&accounts), account.clone(), bucket.clone());
            blocking(move || accounts.prepare_download(&account, &bucket)).await?
        };
        let (kind, destination, items, versioning) = build_plan(&store, &bucket, request).await?;
        let plan = StoredPlan {
            id: self.new_id("plan"),
            made: Instant::now(),
            account,
            bucket,
            kind,
            items,
        };
        let summary = summary_of(&plan, destination, versioning);
        self.lock().plan = Some(plan);
        Ok(summary)
    }

    /// Forgets plan `plan` (the page's "Cancel" before starting).
    pub fn discard(&self, plan: &str) {
        let mut inner = self.lock();
        if inner.plan.as_ref().is_some_and(|p| p.id == plan) {
            inner.plan = None;
        }
    }

    /// Starts plan `plan` with `policy` for its conflicts.
    pub async fn start(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        plan: &str,
        policy: ConflictPolicy,
        sink: Arc<dyn OperationSink>,
    ) -> Result<OperationStarted, CommandError> {
        let stored = {
            let mut inner = self.lock();
            if inner.closed {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "sotyware is quitting; nothing can start",
                ));
            }
            if matches!(inner.slot, Slot::Active { .. }) {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "another file operation is running; wait for it to finish or cancel it",
                ));
            }
            match inner.plan.take() {
                Some(p) if p.id == plan && p.made.elapsed() <= PLAN_TTL => p,
                Some(p) if p.id != plan => {
                    inner.plan = Some(p);
                    return Err(outdated_plan());
                }
                _ => return Err(outdated_plan()),
            }
        };
        let mut works = Vec::new();
        for item in stored.items {
            if item.view.problem.is_some() {
                continue;
            }
            let (steps, destination, state) = match (&item.view.conflict, policy) {
                (Some(_), ConflictPolicy::Skip) => (
                    item.steps,
                    item.view.destination.clone(),
                    ItemState::Skipped,
                ),
                (Some(c), ConflictPolicy::Rename) => match (item.alternative_steps, &c.alternative)
                {
                    (Some(alt), Some(a)) => (alt, Some(a.clone()), ItemState::Pending),
                    _ => (
                        item.steps,
                        item.view.destination.clone(),
                        ItemState::Skipped,
                    ),
                },
                (None, _) => (
                    item.steps,
                    item.view.destination.clone(),
                    ItemState::Pending,
                ),
            };
            works.push(Work {
                report: ItemReport {
                    index: works.len(),
                    name: item.view.name,
                    folder: item.view.folder,
                    source: item.view.source,
                    destination,
                    state,
                    objects_total: steps.len() as u64,
                    objects_done: 0,
                    objects_failed: 0,
                    bytes_total: steps.iter().map(|s| s.size).sum(),
                    bytes_done: 0,
                    hidden: 0,
                    removed: 0,
                    note: (state == ItemState::Skipped).then_some(ItemNote::DestinationExists),
                    failures: Vec::new(),
                },
                steps,
            });
        }
        if works.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "no item of this plan can be done",
            ));
        }
        let store = {
            let (accounts, account, bucket) = (
                Arc::clone(&accounts),
                stored.account.clone(),
                stored.bucket.clone(),
            );
            blocking(move || accounts.prepare_download(&account, &bucket)).await?
        };
        self.launch(
            store,
            stored.account,
            stored.bucket,
            stored.kind,
            works,
            false,
            sink,
        )
    }

    /// Starts a new operation with what the finished operation `operation`
    /// left to do (module documentation).
    pub async fn retry(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        operation: &str,
        sink: Arc<dyn OperationSink>,
    ) -> Result<OperationStarted, CommandError> {
        let spec = {
            let mut inner = self.lock();
            if matches!(inner.slot, Slot::Active { .. }) {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "another file operation is running",
                ));
            }
            match inner.retry.take() {
                Some((id, spec)) if id == operation => spec,
                other => {
                    inner.retry = other;
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "this operation can no longer be retried; review it again",
                    ));
                }
            }
        };
        let store = {
            let (accounts, account, bucket) = (
                Arc::clone(&accounts),
                spec.account.clone(),
                spec.bucket.clone(),
            );
            blocking(move || accounts.prepare_download(&account, &bucket)).await
        };
        let store = match store {
            Ok(s) => s,
            Err(e) => {
                self.lock().retry = Some((operation.to_owned(), spec));
                return Err(e);
            }
        };
        self.launch(
            store,
            spec.account,
            spec.bucket,
            spec.kind,
            spec.works,
            true,
            sink,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn launch(
        self: &Arc<Self>,
        store: Arc<S3Store>,
        account: String,
        bucket: String,
        kind: OperationKind,
        works: Vec<Work>,
        retrying: bool,
        sink: Arc<dyn OperationSink>,
    ) -> Result<OperationStarted, CommandError> {
        let id = self.new_id("op");
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
        });
        {
            let mut inner = self.lock();
            if inner.closed {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "sotyware is quitting; nothing can start",
                ));
            }
            if matches!(inner.slot, Slot::Active { .. }) {
                return Err(CommandError::new(
                    ErrorKind::Busy,
                    "another file operation is running",
                ));
            }
            inner.slot = Slot::Active {
                id: id.clone(),
                control: Arc::clone(&control),
            };
            inner.retry = None;
        }
        let started = OperationStarted {
            operation: id.clone(),
            kind,
            items: works.iter().map(|w| w.report.clone()).collect(),
        };
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut emitter = Emitter {
                operation: id.clone(),
                seq: 0,
                sink,
                last_progress: None,
            };
            let (summary, retry) = this
                .run(
                    &store,
                    &bucket,
                    kind,
                    works,
                    retrying,
                    &control,
                    &mut emitter,
                    &id,
                )
                .await;
            {
                let mut inner = this.lock();
                inner.slot = Slot::Free;
                if let Some(works) = retry {
                    inner.retry = Some((
                        id.clone(),
                        RetrySpec {
                            account,
                            bucket,
                            kind,
                            works,
                        },
                    ));
                }
            }
            this.freed.notify_all();
            emitter.emit(EventBody::Finished { summary });
        });
        Ok(started)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        store: &Arc<S3Store>,
        bucket: &str,
        kind: OperationKind,
        mut works: Vec<Work>,
        retrying: bool,
        control: &Control,
        emitter: &mut Emitter,
        id: &str,
    ) -> (OperationSummary, Option<Vec<Work>>) {
        let mut stop: Option<CommandError> = None;
        emitter.progress(Totals::of(&works), true);
        for w in &works {
            if w.report.state != ItemState::Pending {
                emitter.item(&w.report);
            }
        }
        for index in 0..works.len() {
            if works[index].report.state != ItemState::Pending {
                continue;
            }
            if control.cancelled() || stop.is_some() {
                works[index].report.state = ItemState::Cancelled;
                emitter.item(&works[index].report);
                continue;
            }
            #[cfg(test)]
            if let Some(f) = self.before_item.lock().unwrap().as_ref() {
                f(index);
            }
            if control.cancelled() {
                works[index].report.state = ItemState::Cancelled;
                emitter.item(&works[index].report);
                continue;
            }
            works[index].report.state = ItemState::Running;
            emitter.item(&works[index].report);
            let mut work = works[index].clone();
            let others = Totals::of(&works).minus(&work.report);
            let phases: &[Phase] = match kind {
                OperationKind::Copy => &[Phase::Copy],
                OperationKind::Move | OperationKind::Rename => &[Phase::Copy, Phase::Delete],
                OperationKind::Delete => &[Phase::Delete],
                OperationKind::Restore => &[Phase::Restore],
            };
            for &phase in phases {
                // A move deletes nothing unless every object was copied and
                // verified, and nothing after a cancel.
                if phase == Phase::Delete && kind.moves() {
                    let all_copied = work.steps.iter().all(|s| {
                        matches!(
                            s.done,
                            Done::Copied | Done::Finished { .. } | Done::OriginalKept { .. }
                        )
                    });
                    if !all_copied || control.cancelled() || stop.is_some() {
                        break;
                    }
                }
                if control.cancelled() || stop.is_some() {
                    break;
                }
                if let Some(e) = run_phase(
                    store, bucket, &mut work, phase, kind, retrying, control, emitter, others,
                )
                .await
                {
                    stop = Some(e);
                }
            }
            conclude(&mut work, kind, control.cancelled() || stop.is_some());
            works[index] = work;
            emitter.item(&works[index].report);
            emitter.progress(Totals::of(&works), true);
        }
        let cancelled = control.cancelled();
        let mut counts = ItemCounts::default();
        let (mut od, mut of, mut hidden, mut removed) = (0, 0, 0, 0);
        for w in &works {
            match w.report.state {
                ItemState::Completed => counts.completed += 1,
                ItemState::Skipped => counts.skipped += 1,
                ItemState::Failed => counts.failed += 1,
                ItemState::Partial => counts.partial += 1,
                ItemState::Cancelled | ItemState::Pending | ItemState::Running => {
                    counts.cancelled += 1
                }
            }
            od += w.report.objects_done;
            of += w.report.objects_failed;
            hidden += w.report.hidden;
            removed += w.report.removed;
        }
        let state = if stop.is_some() {
            OperationState::Failed
        } else if cancelled {
            OperationState::Cancelled
        } else if counts.failed + counts.partial > 0 {
            OperationState::Partial
        } else {
            OperationState::Completed
        };
        let retry = retry_works(&works, kind);
        let summary = OperationSummary {
            operation: id.to_owned(),
            kind,
            state,
            items: counts,
            objects_done: od,
            objects_failed: of,
            hidden,
            removed,
            retryable: retry.is_some(),
            error: stop,
        };
        (summary, retry)
    }

    /// Cancels operation `operation`; any other id is left alone.
    pub fn cancel(&self, operation: &str) -> CancelOutcome {
        let inner = self.lock();
        match &inner.slot {
            Slot::Active { id, control } if id == operation => {
                control.cancelled.store(true, Ordering::SeqCst);
                CancelOutcome::Cancelling
            }
            _ => CancelOutcome::NotActive,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.lock().slot, Slot::Active { .. })
    }

    /// For quitting: refuses new operations, cancels the running one and
    /// waits up to `wait` for it to stop.
    pub fn shutdown(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut inner = self.lock();
        inner.closed = true;
        inner.plan = None;
        if let Slot::Active { control, .. } = &inner.slot {
            control.cancelled.store(true, Ordering::SeqCst);
        }
        while matches!(inner.slot, Slot::Active { .. }) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                eprintln!(
                    "sotyware: a file operation was still running when sotyware quit; requests already sent may complete at the provider"
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

fn outdated_plan() -> CommandError {
    CommandError::new(
        ErrorKind::OutdatedPlan,
        "this operation was reviewed too long ago or replaced by another; review it again",
    )
}

/// What a finished operation leaves for a retry (module documentation), or
/// `None` if nothing.
fn retry_works(works: &[Work], kind: OperationKind) -> Option<Vec<Work>> {
    let mut out = Vec::new();
    for w in works {
        if matches!(w.report.state, ItemState::Completed | ItemState::Skipped) {
            continue;
        }
        // A move whose copy cannot complete without a new review (a
        // conflict, a changed object, an integrity failure) is not retried
        // at all: deleting the originals that were copied would split it.
        if kind.moves()
            && w.steps.iter().any(|s| match &s.done {
                Done::Skipped(_) => true,
                Done::Failed(e) => !retryable(e.kind),
                _ => false,
            })
        {
            continue;
        }
        let mut steps = Vec::new();
        for s in &w.steps {
            let keep = match &s.done {
                Done::No => true,
                Done::Failed(e) => retryable(e.kind),
                // A move whose copy is verified: only its delete is left.
                Done::Copied => kind.moves(),
                Done::OriginalKept { retry, .. } => *retry,
                Done::Skipped(_) | Done::Finished { .. } => false,
            };
            if keep {
                let mut s = s.clone();
                match s.done {
                    Done::Failed(_) => s.done = Done::No,
                    Done::OriginalKept { .. } => s.done = Done::Copied,
                    _ => {}
                }
                steps.push(s);
            }
        }
        if steps.is_empty() {
            continue;
        }
        let report = ItemReport {
            index: out.len(),
            state: ItemState::Pending,
            objects_total: steps.len() as u64,
            objects_done: 0,
            objects_failed: 0,
            bytes_total: steps.iter().map(|s| s.size).sum(),
            bytes_done: 0,
            hidden: 0,
            removed: 0,
            note: None,
            failures: Vec::new(),
            ..w.report.clone()
        };
        out.push(Work { report, steps });
    }
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------- commands

/// Plans a copy, move, rename, delete or restore in the account's
/// configured bucket (which must still be `bucket`). Sends only reads.
/// Allowed by `allow-plan-cloud-operation`.
#[tauri::command]
pub async fn plan_cloud_operation(
    accounts: State<'_, Arc<CloudAccounts>>,
    operations: State<'_, Arc<CloudOperations>>,
    id: String,
    bucket: String,
    request: OperationRequest,
) -> Result<PlanSummary, CommandError> {
    let result = Arc::clone(&operations)
        .plan(Arc::clone(&accounts), id, bucket, request)
        .await;
    if let Err(e) = &result
        && e.kind != ErrorKind::InvalidInput
        && e.kind != ErrorKind::DestinationExists
    {
        eprintln!("sotyware: failed to plan a file operation: {}", e.message);
    }
    result
}

/// Forgets a plan that the user did not start. Allowed by
/// `allow-discard-cloud-operation-plan`.
#[tauri::command]
pub fn discard_cloud_operation_plan(operations: State<'_, Arc<CloudOperations>>, plan: String) {
    operations.discard(&plan);
}

/// Starts a reviewed plan; events go to `on_event`. Allowed by
/// `allow-start-cloud-operation`.
#[tauri::command]
pub async fn start_cloud_operation(
    accounts: State<'_, Arc<CloudAccounts>>,
    operations: State<'_, Arc<CloudOperations>>,
    plan: String,
    conflicts: ConflictPolicy,
    on_event: Channel<OperationEvent>,
) -> Result<OperationStarted, CommandError> {
    Arc::clone(&operations)
        .start(Arc::clone(&accounts), &plan, conflicts, Arc::new(on_event))
        .await
}

/// Cancels a running operation. Allowed by `allow-cancel-cloud-operation`.
#[tauri::command]
pub fn cancel_cloud_operation(
    operations: State<'_, Arc<CloudOperations>>,
    operation: String,
) -> CancelOutcome {
    operations.cancel(&operation)
}

/// Retries what a finished operation left. Allowed by
/// `allow-retry-cloud-operation`.
#[tauri::command]
pub async fn retry_cloud_operation(
    accounts: State<'_, Arc<CloudAccounts>>,
    operations: State<'_, Arc<CloudOperations>>,
    operation: String,
    on_event: Channel<OperationEvent>,
) -> Result<OperationStarted, CommandError> {
    Arc::clone(&operations)
        .retry(Arc::clone(&accounts), &operation, Arc::new(on_event))
        .await
}

#[cfg(test)]
mod tests;

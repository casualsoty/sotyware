//! The recovery view's commands (ADR-030): what the operation journal holds
//! from a session that ended unexpectedly, an inspection of each entry
//! against the provider, and the two explicit ways to resolve one —
//! dismissing it once the user has seen it, and aborting an incomplete
//! multipart upload by its exact id.
//!
//! - **Nothing is repeated.** No command re-uploads, re-copies or deletes
//!   an object. An interrupted move whose originals remain is explained
//!   (with the keys); finishing it is a normal, reviewed delete in the
//!   browser.
//! - **Inspection reads only:** `HeadObject` on the keys an entry names (at
//!   most [`MAX_INSPECTED`] per call) and `ListMultipartUploads` for an
//!   upload's key.
//! - **Aborting** a multipart upload is the only change, one id at a time,
//!   after the page asked the user, because an incomplete upload listed by
//!   the provider may belong to another device that is still uploading.
//! - The page never receives a local path: an upload entry shows the file
//!   name only.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::manage::store_for;
use super::operations::operation_error;
use super::s3::{AbortOutcome, IncompleteUpload, S3Store};
use crate::journal::{EntryKind, EntryRow, EntryState, Journal, StepRow};

/// Most keys one inspection checks (one `HeadObject` each, two for a move).
pub const MAX_INSPECTED: usize = 500;
/// Most keys listed per category of an inspection (the counts are exact).
pub const MAX_LISTED: usize = 50;
/// Most incomplete uploads one listing reads (pages of 1,000).
pub const MAX_INCOMPLETE: usize = 5_000;

/// One entry of the recovery view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryItem {
    pub id: i64,
    /// `upload`, `download`, `staging` or `operation`.
    pub kind: &'static str,
    /// `started`, `multipart-open`, `needs-inspection`,
    /// `incomplete-parts` or `partial`.
    pub state: String,
    pub account_id: Option<String>,
    /// The account's name, if it still exists.
    pub account_label: Option<String>,
    pub bucket: Option<String>,
    /// The object key (uploads, downloads).
    pub key: Option<String>,
    /// The local file's name, never its path (uploads).
    pub file_name: Option<String>,
    pub size: Option<u64>,
    /// `copy`, `move`, `rename`, `delete`, `restore` (operations).
    pub operation: Option<String>,
    /// Planned steps (operations).
    pub steps: usize,
    pub multipart: bool,
    pub started_ms: i64,
    pub updated_ms: i64,
    /// sotyware's own sentence on what is known without asking the
    /// provider.
    pub summary: String,
}

/// What `list_cloud_recovery` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryList {
    /// Why the journal cannot be used, if it cannot: interrupted work is
    /// then not recorded.
    pub unavailable: Option<String>,
    pub items: Vec<RecoveryItem>,
}

/// An incomplete multipart upload as the page sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompleteInfo {
    pub key: String,
    pub upload_id: String,
    pub initiated: Option<String>,
    /// The journal knows this upload id: sotyware started it on this
    /// device.
    pub recorded: bool,
}

/// What an inspection found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub id: i64,
    /// `complete` (nothing left to do), `nothing-stored`, `needs-attention`
    /// or `local-only` (a temporary file; nothing at the provider).
    pub verdict: &'static str,
    /// sotyware's sentence.
    pub summary: String,
    /// Counts per category (operations: `moved`, `original-kept`,
    /// `not-started`, `destination-differs`, `missing`, `copied`,
    /// `deleted`, `still-there`, `restored`, `not-restored`; uploads:
    /// `stored-verified`, `absent`, `different`).
    pub counts: BTreeMap<&'static str, usize>,
    /// Up to [`MAX_LISTED`] keys per category.
    pub keys: BTreeMap<&'static str, Vec<String>>,
    /// Steps not inspected (above [`MAX_INSPECTED`]).
    pub not_inspected: usize,
    /// Open multipart uploads for an upload's key.
    pub open_uploads: Vec<IncompleteInfo>,
}

/// What `list_cloud_incomplete_uploads` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompleteList {
    pub uploads: Vec<IncompleteInfo>,
    /// More exist than were read ([`MAX_INCOMPLETE`]).
    pub more: bool,
}

fn journal_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(
        ErrorKind::Journal,
        format!("the operation journal cannot be read: {e}"),
    )
}

fn file_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// What is known about an entry without asking the provider.
pub fn summarize(e: &EntryRow, steps: &[StepRow]) -> String {
    let state = EntryState::parse(&e.state);
    match EntryKind::parse(&e.kind) {
        Some(EntryKind::Upload) => match state {
            Some(EntryState::NeedsInspection) => "The upload passed the point where the provider may store the file, but it was never verified. Inspect it before uploading the file again.".into(),
            Some(EntryState::IncompleteParts) => "The upload failed and its incomplete multipart upload could not be removed: parts of it may remain at the provider and be billed.".into(),
            Some(EntryState::MultipartOpen) => "sotyware stopped while this file was being uploaded in parts. The incomplete upload may still be open at the provider (and be billed) until it is aborted.".into(),
            _ => "sotyware stopped while this file was being uploaded. Inspect to see whether the provider stored it.".into(),
        },
        Some(EntryKind::Download) => "A temporary download file could not be removed automatically; it was not touched because its name is not sotyware's.".into(),
        Some(EntryKind::Staging) => "A temporary download folder could not be removed automatically; it was not touched because its name is not sotyware's.".into(),
        Some(EntryKind::Operation) => {
            let count = |p: &str| steps.iter().filter(|s| s.phase == p).count();
            let op = e.operation.as_deref().unwrap_or("operation");
            let (done, copied, failed, planned) = (
                count("finished"),
                count("copied") + count("original-kept"),
                count("failed") + count("skipped"),
                count("planned"),
            );
            let what = match state {
                Some(EntryState::Partial) => "ended with only part of it applied",
                _ => "was interrupted when sotyware stopped",
            };
            format!(
                "A {op} of {} object(s) {what}. Recorded: {done} done, {copied} copied with the original kept, {failed} failed or skipped, {planned} with no recorded result. Inspect to see the current state.",
                steps.len()
            )
        }
        None => "An entry of an unknown kind.".into(),
    }
}

fn item(e: &EntryRow, steps: &[StepRow], accounts: &CloudAccounts) -> RecoveryItem {
    let kind = EntryKind::parse(&e.kind).map_or("unknown", EntryKind::as_str);
    let label = e.account_id.as_ref().and_then(|id| {
        accounts
            .list()
            .accounts
            .into_iter()
            .find(|a| &a.id == id)
            .map(|a| a.label)
    });
    RecoveryItem {
        id: e.id,
        kind,
        state: e.state.clone(),
        account_id: e.account_id.clone(),
        account_label: label,
        bucket: e.bucket.clone(),
        key: e.object_key.clone(),
        file_name: (kind == "upload")
            .then(|| e.local_path.as_deref().and_then(file_name))
            .flatten(),
        size: e.size,
        operation: e.operation.clone(),
        steps: steps.len(),
        multipart: e.multipart,
        started_ms: e.started_ms,
        updated_ms: e.updated_ms,
        summary: summarize(e, steps),
    }
}

/// Every entry of the journal, for the recovery view.
pub fn list(journal: &Journal, accounts: &CloudAccounts) -> Result<RecoveryList, CommandError> {
    let entries = journal.entries().map_err(journal_error)?;
    let mut items = Vec::with_capacity(entries.len());
    for e in &entries {
        let steps = if e.kind == EntryKind::Operation.as_str() {
            journal.steps(e.id).map_err(journal_error)?
        } else {
            Vec::new()
        };
        items.push(item(e, &steps, accounts));
    }
    Ok(RecoveryList {
        unavailable: journal.unavailable(),
        items,
    })
}

fn entry(journal: &Journal, id: i64) -> Result<EntryRow, CommandError> {
    journal.entry(id).map_err(journal_error)?.ok_or_else(|| {
        CommandError::new(
            ErrorKind::OutdatedPlan,
            "this entry is no longer in the operation journal",
        )
    })
}

/// The open multipart uploads of exactly `key` (the listing is by prefix).
async fn open_uploads_of(
    store: &S3Store,
    bucket: &str,
    key: &str,
    recorded: Option<&str>,
) -> Result<Vec<IncompleteInfo>, CommandError> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .list_incomplete_uploads(bucket, key, after)
            .await
            .map_err(|e| operation_error(&e))?;
        out.extend(
            page.uploads
                .into_iter()
                .filter(|u| u.key == key)
                .map(|u| info(u, |id| Some(id) == recorded)),
        );
        match page.next {
            Some(n) if out.len() < MAX_INCOMPLETE => after = Some(n),
            _ => break,
        }
    }
    Ok(out)
}

fn info(u: IncompleteUpload, recorded: impl Fn(&str) -> bool) -> IncompleteInfo {
    IncompleteInfo {
        recorded: recorded(&u.upload_id),
        key: u.key,
        upload_id: u.upload_id,
        initiated: u.initiated,
    }
}

/// Adds `key` to `category`.
fn note(insp: &mut Inspection, category: &'static str, key: &str) {
    *insp.counts.entry(category).or_default() += 1;
    let list = insp.keys.entry(category).or_default();
    if list.len() < MAX_LISTED {
        list.push(key.to_owned());
    }
}

/// Inspects one entry against the provider (reads only).
pub async fn inspect(
    journal: Arc<Journal>,
    accounts: Arc<CloudAccounts>,
    id: i64,
) -> Result<Inspection, CommandError> {
    let e = {
        let j = Arc::clone(&journal);
        blocking(move || entry(&j, id)).await?
    };
    let mut insp = Inspection {
        id,
        verdict: "needs-attention",
        summary: String::new(),
        counts: BTreeMap::new(),
        keys: BTreeMap::new(),
        not_inspected: 0,
        open_uploads: Vec::new(),
    };
    let kind = EntryKind::parse(&e.kind);
    if matches!(kind, Some(EntryKind::Download | EntryKind::Staging)) {
        insp.verdict = "local-only";
        insp.summary = "Only a temporary file on this computer is involved; nothing at the provider. Dismiss this entry; the file can be deleted by hand.".into();
        return Ok(insp);
    }
    let (Some(account), Some(bucket)) = (e.account_id.clone(), e.bucket.clone()) else {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "this entry names no account or bucket",
        ));
    };
    let store = store_for(&accounts, &account, &bucket).await?;
    match kind {
        Some(EntryKind::Upload) => {
            inspect_upload(&store, &bucket, &e, &mut insp).await?;
        }
        Some(EntryKind::Operation) => {
            let steps = {
                let j = Arc::clone(&journal);
                blocking(move || j.steps(id).map_err(journal_error)).await?
            };
            inspect_operation(&store, &bucket, &e, &steps, &mut insp).await?;
        }
        _ => {
            insp.summary = "This entry cannot be inspected.".into();
        }
    }
    Ok(insp)
}

async fn inspect_upload(
    store: &S3Store,
    bucket: &str,
    e: &EntryRow,
    insp: &mut Inspection,
) -> Result<(), CommandError> {
    let key = e.object_key.clone().unwrap_or_default();
    let props = store
        .object_properties(bucket, &key)
        .await
        .map_err(|err| operation_error(&err))?;
    let stored = match (&props, &e.sha256, e.size) {
        (Some(p), Some(sha), Some(size)) => {
            p.size == size && p.sha256().is_some_and(|s| s.eq_ignore_ascii_case(sha))
        }
        _ => false,
    };
    let category = match (&props, stored) {
        (None, _) => "absent",
        (Some(_), true) => "stored-verified",
        (Some(_), false) => "different",
    };
    note(insp, category, &key);
    if e.multipart {
        insp.open_uploads = open_uploads_of(store, bucket, &key, e.upload_id.as_deref()).await?;
    }
    let open = insp.open_uploads.len();
    let (verdict, summary) = match (category, open) {
        ("stored-verified", 0) => (
            "complete",
            "The provider holds this file with the expected size and SHA-256: the upload completed. Nothing is left to do.".to_owned(),
        ),
        ("absent", 0) => (
            "nothing-stored",
            "The provider does not hold this file and no incomplete upload remains: nothing was stored. Upload it again if you still want it there.".to_owned(),
        ),
        ("different", 0) => (
            "needs-attention",
            "An object with this name exists but it is not this upload's content (another device or program may have written it). It was left as it is.".to_owned(),
        ),
        (_, n) => (
            "needs-attention",
            format!(
                "{n} incomplete multipart upload(s) for this name remain at the provider and may be billed. Abort the one(s) sotyware started; an upload sotyware did not record may still be running on another device."
            ),
        ),
    };
    insp.verdict = verdict;
    insp.summary = summary;
    Ok(())
}

async fn inspect_operation(
    store: &S3Store,
    bucket: &str,
    e: &EntryRow,
    steps: &[StepRow],
    insp: &mut Inspection,
) -> Result<(), CommandError> {
    let op = e.operation.as_deref().unwrap_or("");
    let mut checked = 0usize;
    for s in steps {
        if checked >= MAX_INSPECTED {
            insp.not_inspected += 1;
            continue;
        }
        checked += 1;
        let source = store
            .object_properties(bucket, &s.source_key)
            .await
            .map_err(|err| operation_error(&err))?;
        let unchanged_source = source.as_ref().is_some_and(|p| {
            s.source_etag
                .as_deref()
                .is_none_or(|planned| p.e_tag.as_deref() == Some(planned.trim_matches('"')))
        });
        let category = match op {
            "move" | "rename" | "copy" => {
                let dest = match &s.dest_key {
                    Some(d) => store
                        .object_properties(bucket, d)
                        .await
                        .map_err(|err| operation_error(&err))?,
                    None => None,
                };
                let dest_matches = dest.as_ref().is_some_and(|d| {
                    s.source_etag.as_deref().is_some_and(|planned| {
                        d.e_tag.as_deref() == Some(planned.trim_matches('"'))
                    })
                });
                match (op, source.is_some(), dest.is_some(), dest_matches) {
                    ("copy", _, true, true) => "copied",
                    ("copy", _, false, _) => "not-copied",
                    (_, _, true, false) => "destination-differs",
                    (_, false, true, true) => "moved",
                    (_, true, true, true) => "original-kept",
                    (_, true, false, _) => "not-started",
                    (_, false, false, _) => "missing",
                }
            }
            "delete" => {
                if source.is_none() {
                    "deleted"
                } else if unchanged_source {
                    "still-there"
                } else {
                    "changed-since"
                }
            }
            "restore" => {
                if source.is_some() {
                    "restored"
                } else {
                    "not-restored"
                }
            }
            _ => "unknown",
        };
        note(insp, category, &s.source_key);
    }
    let n = |c: &str| insp.counts.get(c).copied().unwrap_or(0);
    let attention = n("original-kept")
        + n("destination-differs")
        + n("missing")
        + n("not-started")
        + n("not-copied")
        + n("still-there")
        + n("changed-since")
        + n("not-restored")
        + insp.not_inspected;
    insp.verdict = if attention == 0 {
        "complete"
    } else {
        "needs-attention"
    };
    insp.summary = match op {
        "move" | "rename" => format!(
            "{} moved; {} copied with the original still in place (delete the originals in the browser once you have checked the copies, or keep both); {} not started; {} where the destination holds something else; {} found at neither place.",
            n("moved"),
            n("original-kept"),
            n("not-started"),
            n("destination-differs"),
            n("missing")
        ),
        "copy" => format!(
            "{} copied; {} not copied (copy them again from the browser if you still want them); {} where the destination holds something else.",
            n("copied"),
            n("not-copied"),
            n("destination-differs")
        ),
        "delete" => format!(
            "{} deleted (on a versioned bucket they can be restored from Deleted items); {} still there; {} changed since the delete was reviewed.",
            n("deleted"),
            n("still-there"),
            n("changed-since")
        ),
        "restore" => format!(
            "{} restored; {} not restored (restore them again from Deleted items).",
            n("restored"),
            n("not-restored")
        ),
        _ => "This operation cannot be inspected.".into(),
    };
    if insp.not_inspected > 0 {
        insp.summary.push_str(&format!(
            " {} more object(s) were not inspected (at most {MAX_INSPECTED} per inspection).",
            insp.not_inspected
        ));
    }
    Ok(())
}

/// The incomplete multipart uploads in an account's bucket (read only).
pub async fn incomplete(
    journal: Arc<Journal>,
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
) -> Result<IncompleteList, CommandError> {
    let store = store_for(&accounts, &id, &bucket).await?;
    let known: Vec<String> = blocking(move || {
        journal
            .entries()
            .map(|es| es.into_iter().filter_map(|e| e.upload_id).collect())
            .map_err(journal_error)
    })
    .await?;
    let mut uploads = Vec::new();
    let mut after = None;
    let more = loop {
        let page = store
            .list_incomplete_uploads(&bucket, "", after)
            .await
            .map_err(|e| operation_error(&e))?;
        uploads.extend(
            page.uploads
                .into_iter()
                .map(|u| info(u, |id| known.iter().any(|k| k == id))),
        );
        match page.next {
            Some(n) if uploads.len() < MAX_INCOMPLETE => after = Some(n),
            Some(_) => break true,
            None => break false,
        }
    };
    Ok(IncompleteList { uploads, more })
}

/// How an abort the page asked for went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AbortResult {
    /// The provider removed the upload and its parts.
    Aborted,
    /// The provider no longer knows it (completed or aborted before).
    Gone,
    /// The abort failed or did not answer in time; parts may remain.
    Failed,
}

/// Aborts exactly one incomplete multipart upload, then forgets journal
/// entries that recorded this id.
pub async fn abort(
    journal: Arc<Journal>,
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    key: String,
    upload_id: String,
) -> Result<AbortResult, CommandError> {
    if key.is_empty() || upload_id.is_empty() || upload_id.len() > 1024 {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "an incomplete upload is named by its key and upload id",
        ));
    }
    let store = store_for(&accounts, &id, &bucket).await?;
    let outcome = store
        .abort_incomplete_upload(&bucket, &key, &upload_id)
        .await;
    let result = match outcome {
        AbortOutcome::Aborted => AbortResult::Aborted,
        AbortOutcome::Gone => AbortResult::Gone,
        AbortOutcome::Failed => AbortResult::Failed,
    };
    if result != AbortResult::Failed {
        blocking(move || journal.forget_upload_id(&upload_id).map_err(journal_error)).await?;
    }
    Ok(result)
}

/// Removes an entry the user has seen and resolved.
pub fn dismiss(journal: &Journal, id: i64) -> Result<(), CommandError> {
    journal.finish(id).map_err(journal_error)
}

// --------------------------------------------------------------- commands

/// Every entry of the operation journal. Allowed by
/// `allow-list-cloud-recovery`.
#[tauri::command]
pub async fn list_cloud_recovery(
    journal: State<'_, Arc<Journal>>,
    accounts: State<'_, Arc<CloudAccounts>>,
) -> Result<RecoveryList, CommandError> {
    let (journal, accounts) = (Arc::clone(&journal), Arc::clone(&accounts));
    blocking(move || list(&journal, &accounts)).await
}

/// Inspects one entry at the provider (reads only). Allowed by
/// `allow-inspect-cloud-recovery`.
#[tauri::command]
pub async fn inspect_cloud_recovery(
    journal: State<'_, Arc<Journal>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    id: i64,
) -> Result<Inspection, CommandError> {
    inspect(Arc::clone(&journal), Arc::clone(&accounts), id).await
}

/// Removes one entry the user resolved. Allowed by
/// `allow-dismiss-cloud-recovery`.
#[tauri::command]
pub async fn dismiss_cloud_recovery(
    journal: State<'_, Arc<Journal>>,
    id: i64,
) -> Result<(), CommandError> {
    let journal = Arc::clone(&journal);
    blocking(move || dismiss(&journal, id)).await
}

/// The incomplete multipart uploads in an account's bucket. Allowed by
/// `allow-list-cloud-incomplete-uploads`.
#[tauri::command]
pub async fn list_cloud_incomplete_uploads(
    journal: State<'_, Arc<Journal>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
) -> Result<IncompleteList, CommandError> {
    incomplete(Arc::clone(&journal), Arc::clone(&accounts), id, bucket).await
}

/// Aborts one incomplete multipart upload the user chose. Allowed by
/// `allow-abort-cloud-incomplete-upload`.
#[tauri::command]
pub async fn abort_cloud_incomplete_upload(
    journal: State<'_, Arc<Journal>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    key: String,
    upload_id: String,
) -> Result<AbortResult, CommandError> {
    abort(
        Arc::clone(&journal),
        Arc::clone(&accounts),
        id,
        bucket,
        key,
        upload_id,
    )
    .await
}

#[cfg(test)]
mod tests;

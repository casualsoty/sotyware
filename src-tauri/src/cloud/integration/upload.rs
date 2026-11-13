//! Opt-in: one controlled upload validation against the real Backblaze B2
//! test bucket through the app's own upload batch path
//! (docs/development/testing.md#uploads-at-backblaze). Passed against
//! Backblaze on 2026-11-13 once on WSL2 and once on native Windows 11.
//!
//! `backblaze_b2_upload_batch` uploads three generated files into the new
//! run prefix `sotyware-it/upload-validation/<run id>/` through
//! [`begin_upload`] and [`CloudUploads`] (the code behind
//! `upload_cloud_files`, with the file dialog replaced by a fixed list),
//! checks that an existing key is skipped and refused and never overwritten,
//! cancels one multipart upload during its second part, downloads the three
//! files through [`CloudDownloads`] and compares them byte for byte, then
//! removes exactly what the run created, by version id and upload id.
//!
//! Safety, enforced here:
//! - Nothing is sent unless `SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation`
//!   is set, in addition to `--ignored` and the account variables, so a
//!   `cargo test -- --ignored` run for another provider test does not write.
//! - The account must be Backblaze's own endpoint for the region.
//! - Every request passes [`RunScope`], an SDK interceptor that refuses it
//!   **before it is sent** unless it is a listing of exactly the run prefix
//!   (`ListObjectVersions`, `ListMultipartUploads`) or a request for one of
//!   this run's four keys, and refuses every `DELETE` without a version id
//!   or a multipart upload id. The app's own code (batch, download) runs
//!   with the same client ([`CloudAccounts::set_connected`]).
//! - Before the first write the run prefix must hold no object version,
//!   delete marker or incomplete multipart upload.
//! - The checks run in their own task, so a failing check cannot skip the
//!   cleanup. Cleanup lists the run prefix, aborts incomplete multipart
//!   uploads and deletes versions **only of this run's keys** (by id, never
//!   by key alone), reports anything else, and lists again: nothing may be
//!   left. Bucket settings, lifecycle rules and keys are never touched.
//! - Every client makes one attempt per request, so nothing is retried
//!   against the daily transaction caps.
//!
//! `backblaze_b2_upload_cleanup` (same confirmation, plus
//! `SOTYWARE_IT_B2_UPLOAD_RUN=<run id>`) runs the same cleanup for a run
//! that was interrupted. `upload_scenario_against_the_test_server` runs the
//! same scenario, with the same [`RunScope`], against the in-process server
//! in every `cargo test`.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_s3::config::interceptors::BeforeTransmitInterceptorContextRef;
use aws_sdk_s3::config::{ConfigBag, Intercept, RuntimeComponents};
use aws_sdk_s3::error::{BoxError, DisplayErrorContext};

use super::super::account::{MemoryAccountStore, ResolvedAccount};
use super::super::credentials::{CredentialStore, Credentials, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CloudAccounts, CredentialInput};
use super::super::download::{
    CloudDownloads, DownloadEvent, EventSink, Integrity, Request, TransferState,
};
use super::super::provider::BACKBLAZE_B2;
use super::super::s3::{
    AbortOutcome, ClientOptions, Existing, S3Store, StorageError, UploadPlan, plan_upload,
};
use super::super::test_server::{TempDir, TestServer};
use super::super::upload::{
    BatchState, BatchSummary, CloudUploads, EventBody, FileState, RemoteState, SkipReason, Target,
    UploadEvent, UploadSink, begin_upload,
};
use super::download::{assert_backblaze, fail, sha256_of};
use super::{Requests, b2_store};

/// Every run's objects live below this, inside the test key's own prefix.
const UPLOAD_ROOT: &str = "sotyware-it/upload-validation/";
/// The second, explicit opt-in for the writing tests.
const CONFIRM_VAR: &str = "SOTYWARE_IT_B2_UPLOAD_CONFIRM";
const CONFIRM_VALUE: &str = "upload-validation";
const RUN_VAR: &str = "SOTYWARE_IT_B2_UPLOAD_RUN";

const MIB: usize = 1024 * 1024;
const EMPTY: &str = "empty.bin";
/// A space, a `+` and non-ASCII letters: the key must reach the provider and
/// come back exactly as given.
const UNICODE: &str = "größe +test.bin";
const MULTIPART: &str = "multipart.bin";
/// Cancelled during its second part; must leave nothing.
const CANCELLED: &str = "cancelled.bin";
/// Every key a run may create, and so the only ones it may remove.
const NAMES: [&str; 4] = [EMPTY, UNICODE, MULTIPART, CANCELLED];
const UNICODE_SIZE: usize = 200_003;
/// Three parts of 5 MiB, 5 MiB and 3 bytes with [`options`].
const MULTIPART_SIZE: usize = 10 * MIB + 3;
const MULTIPART_PARTS: u64 = 3;
/// Longest wait for one batch or download.
const WAIT: Duration = Duration::from_secs(600);

/// One attempt per request; multipart from 6 MiB in parts of 5 MiB (Backblaze's
/// minimum is 5,000,000 bytes), so a 10 MiB file exercises multipart.
fn options() -> ClientOptions {
    ClientOptions {
        max_attempts: 1,
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        ..ClientOptions::default()
    }
}

fn writes_confirmed(value: Option<&str>) -> bool {
    value == Some(CONFIRM_VALUE)
}

/// Stops before any request unless the writes were confirmed explicitly.
fn require_confirmation() {
    let value = std::env::var(CONFIRM_VAR).ok();
    if !writes_confirmed(value.as_deref()) {
        fail(format!(
            "set {CONFIRM_VAR}={CONFIRM_VALUE} to let this test write to the Backblaze B2 test bucket; nothing was sent"
        ));
    }
}

/// The run prefix for `run`, which may not leave the upload root.
fn run_prefix(run: &str) -> String {
    assert!(
        !run.is_empty()
            && run
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "a run id is letters, digits, '-' and '_' only"
    );
    format!("{UPLOAD_ROOT}{run}/")
}

fn new_run_id() -> String {
    format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    )
}

/// Decodes `%XX` escapes; `None` if the result is not UTF-8 or an escape is
/// malformed.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The decoded value of query parameter `name`; `Some("")` for a bare
/// `name`.
fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then(|| percent_decode(v)).flatten()
    })
}

/// Whether a request may be sent during a run with prefix `prefix` in
/// `bucket` (path-style addressing): a listing of exactly the run prefix, or
/// a request for one of the run's own keys; a `DELETE` only with a version
/// id or a multipart upload id. `path` is the URI from the path on, query
/// included.
fn in_scope(method: &str, path: &str, bucket: &str, prefix: &str) -> Result<(), String> {
    let (raw_path, query) = path.split_once('?').unwrap_or((path, ""));
    let decoded = percent_decode(raw_path).ok_or("the path does not decode")?;
    let bucket_root = format!("/{bucket}");
    if decoded == bucket_root || decoded == format!("{bucket_root}/") {
        let listing =
            query_param(query, "versions").is_some() || query_param(query, "uploads").is_some();
        if method != "GET" || !listing {
            return Err(
                "only ListObjectVersions and ListMultipartUploads may address the bucket".into(),
            );
        }
        if query_param(query, "prefix").as_deref() != Some(prefix) {
            return Err("a listing must be limited to exactly the run prefix".into());
        }
        if query_param(query, "delimiter").is_some() {
            return Err("a listing must not use a delimiter".into());
        }
        return Ok(());
    }
    let key = decoded
        .strip_prefix(&format!("{bucket_root}/"))
        .ok_or("not in the test bucket")?;
    let name = key
        .strip_prefix(prefix)
        .ok_or("the key is outside the run prefix")?;
    if !NAMES.contains(&name) {
        return Err("the key is not one this run may create".into());
    }
    match method {
        "HEAD" | "GET" | "PUT" | "POST" => Ok(()),
        "DELETE" => {
            let by_version = query_param(query, "versionId").is_some_and(|v| !v.is_empty());
            let by_upload = query_param(query, "uploadId").is_some_and(|v| !v.is_empty());
            if by_version || by_upload {
                Ok(())
            } else {
                Err("a DELETE needs a version id or a multipart upload id".into())
            }
        }
        _ => Err("method not allowed".into()),
    }
}

/// Refuses, before it is sent, every request [`in_scope`] does not allow,
/// and remembers what it refused.
#[derive(Clone, Debug)]
struct RunScope {
    bucket: String,
    prefix: String,
    refused: Arc<Mutex<Vec<String>>>,
}

impl RunScope {
    fn new(bucket: &str, prefix: &str) -> Self {
        Self {
            bucket: bucket.to_owned(),
            prefix: prefix.to_owned(),
            refused: Arc::default(),
        }
    }

    fn refused(&self) -> Vec<String> {
        self.refused.lock().unwrap().clone()
    }
}

impl Intercept for RunScope {
    fn name(&self) -> &'static str {
        "sotyware-it-run-scope"
    }

    fn read_before_transmit(
        &self,
        context: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let request = context.request();
        let uri = request.uri();
        let after_scheme = uri.split_once("://").map_or(uri, |(_, rest)| rest);
        let path = after_scheme.find('/').map_or("/", |i| &after_scheme[i..]);
        let method = request.method();
        in_scope(method, path, &self.bucket, &self.prefix).map_err(|why| {
            let shown = format!(
                "{method} {}: {why}",
                path.replacen(&format!("/{}", self.bucket), "/<bucket>", 1)
            );
            self.refused.lock().unwrap().push(shown.clone());
            BoxError::from(format!("refused by the test's run scope: {shown}"))
        })
    }
}

/// One entry of a version listing.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VersionRow {
    key: String,
    version_id: String,
    size: Option<i64>,
    /// A delete marker (B2: hide marker).
    marker: bool,
}

/// One incomplete multipart upload.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenUpload {
    key: String,
    upload_id: String,
}

/// Every version and delete marker under `prefix`.
async fn list_versions(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<VersionRow>, String> {
    let mut all = Vec::new();
    let (mut key_marker, mut version_marker) = (None::<String>, None::<String>);
    loop {
        let out = store
            .sdk_client()
            .list_object_versions()
            .bucket(bucket)
            .prefix(prefix)
            .set_key_marker(key_marker.take())
            .set_version_id_marker(version_marker.take())
            .send()
            .await
            .map_err(|e| format!("list object versions: {}", DisplayErrorContext(&e)))?;
        for v in out.versions() {
            all.push(VersionRow {
                key: v.key().unwrap_or_default().to_owned(),
                version_id: v.version_id().unwrap_or_default().to_owned(),
                size: v.size(),
                marker: false,
            });
        }
        for m in out.delete_markers() {
            all.push(VersionRow {
                key: m.key().unwrap_or_default().to_owned(),
                version_id: m.version_id().unwrap_or_default().to_owned(),
                size: None,
                marker: true,
            });
        }
        if out.is_truncated() != Some(true) {
            break;
        }
        key_marker = out.next_key_marker().map(str::to_owned);
        version_marker = out.next_version_id_marker().map(str::to_owned);
        if key_marker.is_none() && version_marker.is_none() {
            return Err("list object versions: truncated without a marker".into());
        }
    }
    Ok(all)
}

/// Every incomplete multipart upload under `prefix`.
async fn list_open_uploads(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<OpenUpload>, String> {
    let mut all = Vec::new();
    let (mut key_marker, mut id_marker) = (None::<String>, None::<String>);
    loop {
        let out = store
            .sdk_client()
            .list_multipart_uploads()
            .bucket(bucket)
            .prefix(prefix)
            .set_key_marker(key_marker.take())
            .set_upload_id_marker(id_marker.take())
            .send()
            .await
            .map_err(|e| format!("list multipart uploads: {}", DisplayErrorContext(&e)))?;
        for u in out.uploads() {
            all.push(OpenUpload {
                key: u.key().unwrap_or_default().to_owned(),
                upload_id: u.upload_id().unwrap_or_default().to_owned(),
            });
        }
        if out.is_truncated() != Some(true) {
            break;
        }
        key_marker = out.next_key_marker().map(str::to_owned);
        id_marker = out.next_upload_id_marker().map(str::to_owned);
        if key_marker.is_none() && id_marker.is_none() {
            return Err("list multipart uploads: truncated without a marker".into());
        }
    }
    Ok(all)
}

/// Whether `key` is one this run (prefix `prefix`) may have created.
fn is_run_key(prefix: &str, key: &str) -> bool {
    key.strip_prefix(prefix)
        .is_some_and(|name| NAMES.contains(&name))
}

/// Splits a version listing into what cleanup deletes (this run's keys,
/// with a version id) and what it only reports.
fn split_versions<'a>(
    prefix: &str,
    rows: &'a [VersionRow],
) -> (Vec<&'a VersionRow>, Vec<&'a VersionRow>) {
    rows.iter()
        .partition(|r| is_run_key(prefix, &r.key) && !r.version_id.is_empty())
}

/// Splits incomplete multipart uploads into what cleanup aborts (this run's
/// keys, with an upload id) and what it only reports.
fn split_uploads<'a>(
    prefix: &str,
    uploads: &'a [OpenUpload],
) -> (Vec<&'a OpenUpload>, Vec<&'a OpenUpload>) {
    uploads
        .iter()
        .partition(|u| is_run_key(prefix, &u.key) && !u.upload_id.is_empty())
}

/// What cleanup found and left.
#[derive(Debug)]
struct Cleanup {
    /// The run prefix's versions before anything was deleted.
    found_versions: Option<Vec<VersionRow>>,
    /// Its incomplete multipart uploads before anything was aborted.
    found_uploads: Option<Vec<OpenUpload>>,
    left_versions: Result<Vec<VersionRow>, String>,
    left_uploads: Result<Vec<OpenUpload>, String>,
    /// Every failure and everything left alone, one line each.
    problems: Vec<String>,
}

impl Cleanup {
    fn clean(&self) -> bool {
        self.problems.is_empty()
            && self.left_versions.as_ref().is_ok_and(Vec::is_empty)
            && self.left_uploads.as_ref().is_ok_and(Vec::is_empty)
    }
}

/// Aborts this run's incomplete multipart uploads (by upload id), deletes
/// this run's versions and delete markers (by version id), then lists the
/// run prefix again. Never panics, so every step gets its attempt; nothing
/// outside this run's keys is changed.
async fn cleanup_run(store: &S3Store, bucket: &str, prefix: &str) -> Cleanup {
    let mut problems = Vec::new();
    let found_uploads = match list_open_uploads(store, bucket, prefix).await {
        Ok(u) => Some(u),
        Err(e) => {
            problems.push(format!("CLEANUP FAILED: {e}"));
            None
        }
    };
    if let Some(found) = &found_uploads {
        let (abort, foreign) = split_uploads(prefix, found);
        for f in foreign {
            problems.push(format!(
                "NOT OURS, left alone: multipart upload {} of {:?}",
                f.upload_id, f.key
            ));
        }
        for u in abort {
            match store
                .abort_incomplete_upload(bucket, &u.key, &u.upload_id)
                .await
            {
                AbortOutcome::Aborted | AbortOutcome::Gone => {}
                AbortOutcome::Failed => problems.push(format!(
                    "CLEANUP FAILED: abort of multipart upload {} of {:?}",
                    u.upload_id, u.key
                )),
            }
        }
    }
    let found_versions = match list_versions(store, bucket, prefix).await {
        Ok(v) => Some(v),
        Err(e) => {
            problems.push(format!("CLEANUP FAILED: {e}"));
            None
        }
    };
    if let Some(found) = &found_versions {
        let (delete, foreign) = split_versions(prefix, found);
        for f in foreign {
            problems.push(format!(
                "NOT OURS, left alone: version {:?} of {:?}",
                f.version_id, f.key
            ));
        }
        for v in delete {
            if let Err(e) = store
                .delete_object(bucket, &v.key, Some(&v.version_id))
                .await
            {
                problems.push(format!(
                    "CLEANUP FAILED: delete version {} of {:?}: {e}",
                    v.version_id, v.key
                ));
            }
        }
    }
    Cleanup {
        found_versions,
        found_uploads,
        left_versions: list_versions(store, bucket, prefix).await,
        left_uploads: list_open_uploads(store, bucket, prefix).await,
        problems,
    }
}

/// Records a batch's events.
#[derive(Default)]
struct Events(Mutex<Vec<UploadEvent>>);

impl UploadSink for Events {
    fn emit(&self, event: UploadEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    /// Waits for batch `batch`'s summary and checks its event numbering.
    async fn finished(&self, batch: &str) -> BatchSummary {
        let started = Instant::now();
        loop {
            let events: Vec<UploadEvent> = self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.batch == batch)
                .cloned()
                .collect();
            if let Some(EventBody::Finished { summary }) = events.last().map(|e| &e.body) {
                let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
                let expected: Vec<u64> = (1..=events.len() as u64).collect();
                assert_eq!(seqs, expected, "event numbering of {batch}");
                return summary.clone();
            }
            assert!(started.elapsed() < WAIT, "batch {batch} did not finish");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Uploads `paths` into the run prefix as `upload_cloud_files` does after
/// the dialog, and returns the summary.
async fn run_batch(
    uploads: &Arc<CloudUploads>,
    accounts: &Arc<CloudAccounts>,
    target: &Target,
    paths: Vec<PathBuf>,
) -> BatchSummary {
    let events = Arc::new(Events::default());
    let selection = begin_upload(
        uploads,
        Arc::clone(accounts),
        target.clone(),
        async move { Some(paths) },
        Arc::clone(&events) as Arc<dyn UploadSink>,
    )
    .await
    .unwrap_or_else(|e| panic!("starting the batch: {e:?}"))
    .expect("a selection");
    assert_eq!(selection.prefix, target.prefix);
    for f in &selection.files {
        assert_eq!(f.state, FileState::Pending, "{f:?}");
    }
    let batch = selection.batch.expect("a batch id");
    let summary = events.finished(&batch).await;
    assert!(!uploads.busy(), "the slot is free after the summary");
    summary
}

/// A generated source file and what it must arrive as.
struct Source {
    name: &'static str,
    path: PathBuf,
    size: u64,
    sha256: String,
}

impl Source {
    fn new(dir: &TempDir, name: &'static str, size: usize, seed: u8) -> Self {
        let path = dir.file(name, size, seed);
        Self {
            name,
            sha256: sha256_of(&path),
            size: size as u64,
            path,
        }
    }
}

/// Records download messages.
#[derive(Default)]
struct DownloadEvents(Mutex<Vec<DownloadEvent>>);

impl EventSink for DownloadEvents {
    fn emit(&self, event: DownloadEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// Downloads `key` through [`CloudDownloads`] (the code behind
/// `download_cloud_file`, a path instead of the save dialog) into `dir` and
/// checks it against `source`.
async fn download_and_compare(
    accounts: &Arc<CloudAccounts>,
    account: &str,
    bucket: &str,
    key: &str,
    dir: &TempDir,
    source: &Source,
) {
    let downloads = Arc::new(CloudDownloads::new());
    let sink = Arc::new(DownloadEvents::default());
    let dest = dir.path(source.name);
    downloads.start(
        downloads.reserve().expect("a free download slot"),
        Arc::clone(accounts),
        Request {
            account: account.to_owned(),
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            dest: dest.clone(),
            existing: Existing::Refuse,
        },
        Arc::clone(&sink) as Arc<dyn EventSink>,
    );
    let started = Instant::now();
    let last = loop {
        let last = sink.0.lock().unwrap().last().cloned();
        if let Some(e) = last.filter(|e| e.state != TransferState::Active) {
            break e;
        }
        assert!(started.elapsed() < WAIT, "the download did not finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    println!(
        "download {:?}: {:?} {:?}, {} bytes, {:.2} s",
        source.name,
        last.state,
        last.integrity,
        last.bytes,
        started.elapsed().as_secs_f64()
    );
    assert_eq!(last.state, TransferState::Completed, "{:?}", last.error);
    // Only a SHA-256 in the GetObject metadata equal to the received bytes'
    // hash gives this.
    assert_eq!(last.integrity, Some(Integrity::Sha256Verified));
    assert_eq!(last.bytes, source.size);
    assert_eq!(sha256_of(&dest), source.sha256);
    assert!(
        fs::read(&dest).unwrap() == fs::read(&source.path).unwrap(),
        "{:?} is not byte-for-byte equal to the uploaded file",
        source.name
    );
}

/// The scenario, against any account whose store `accounts` already holds:
/// one batch of three files, an existing key skipped and refused, a
/// multipart upload cancelled during its second part, three downloads.
/// Panics on the first failed check; removes nothing.
async fn scenario(
    accounts: Arc<CloudAccounts>,
    store: Arc<S3Store>,
    target: Target,
    limits: super::super::provider::TransferLimits,
) {
    let dir = TempDir::new("it-upload-source");
    let other = TempDir::new("it-upload-other");
    let empty = Source::new(&dir, EMPTY, 0, 1);
    let unicode = Source::new(&dir, UNICODE, UNICODE_SIZE, 2);
    let multipart = Source::new(&dir, MULTIPART, MULTIPART_SIZE, 3);
    let cancelled = Source::new(&dir, CANCELLED, MULTIPART_SIZE, 4);
    // Same name as `unicode`, other content: must never replace it.
    let rival = Source::new(&other, UNICODE, UNICODE_SIZE + 1, 5);
    for s in [&empty, &unicode, &multipart, &cancelled, &rival] {
        println!(
            "generated {:?}: {} bytes, SHA-256 {}",
            s.name, s.size, s.sha256
        );
    }
    assert_eq!(
        plan_upload(MULTIPART_SIZE as u64, &limits, &options()),
        Ok(UploadPlan::Multipart {
            part_size: 5 * MIB as u64,
            parts: MULTIPART_PARTS
        })
    );
    assert_eq!(
        plan_upload(UNICODE_SIZE as u64, &limits, &options()),
        Ok(UploadPlan::Single)
    );
    let key = |name: &str| format!("{}{name}", target.prefix);

    // 1. One batch: empty file, Unicode name, multipart.
    let uploads = Arc::new(CloudUploads::new());
    let t0 = Instant::now();
    let summary = run_batch(
        &uploads,
        &accounts,
        &target,
        vec![
            empty.path.clone(),
            unicode.path.clone(),
            multipart.path.clone(),
        ],
    )
    .await;
    println!(
        "batch 1: {:?}, {} completed, {:.2} s",
        summary.state,
        summary.completed,
        t0.elapsed().as_secs_f64()
    );
    assert_eq!(summary.state, BatchState::Completed, "{summary:?}");
    assert_eq!(
        (summary.completed, summary.skipped, summary.failed),
        (3, 0, 0)
    );
    assert_eq!(summary.needs_attention, 0);
    for (file, source) in summary.files.iter().zip([&empty, &unicode, &multipart]) {
        assert_eq!(file.state, FileState::Completed, "{file:?}");
        assert_eq!(file.name, source.name);
        assert_eq!(file.key.as_deref(), Some(key(source.name).as_str()));
        assert_eq!((file.bytes, file.total), (source.size, source.size));
        let head = store
            .head_object(&target.bucket, &key(source.name))
            .await
            .unwrap_or_else(|e| panic!("head {:?}: {e}", source.name))
            .expect("the uploaded object");
        println!(
            "head {:?}: {} bytes, SHA-256 metadata {:?}, version {:?}",
            source.name, head.size, head.sha256, head.version_id
        );
        assert_eq!(head.size, source.size);
        assert_eq!(head.sha256.as_deref(), Some(source.sha256.as_str()));
    }

    // 2. An existing key: the batch skips it before reading the file, and
    //    the adapter itself refuses it.
    let summary = run_batch(&uploads, &accounts, &target, vec![rival.path.clone()]).await;
    println!("batch 2 (existing key): {:?}", summary.files);
    assert_eq!(summary.state, BatchState::Completed, "{summary:?}");
    assert_eq!((summary.completed, summary.skipped), (0, 1));
    let file = &summary.files[0];
    assert_eq!(file.state, FileState::Skipped);
    assert_eq!(
        file.skip.as_ref().map(|s| s.reason),
        Some(SkipReason::DestinationExists)
    );
    assert!(!file.started);
    let refused = store
        .upload_file(&target.bucket, &key(UNICODE), &rival.path)
        .await;
    println!("direct upload over the existing key: {refused:?}");
    assert!(
        matches!(refused, Err(StorageError::AlreadyExists(_))),
        "{refused:?}"
    );

    // 3. Cancel a multipart upload during its second part: the abort must
    //    leave nothing.
    let cancel_uploads = Arc::new(CloudUploads::new());
    let weak = Arc::downgrade(&cancel_uploads);
    cancel_uploads.file_control(move |_, control| {
        let weak = weak.clone();
        control.during_send(2, move || {
            if let Some(uploads) = weak.upgrade() {
                println!("cancel during part 2: {:?}", uploads.cancel_active());
            }
        });
    });
    let summary = run_batch(
        &cancel_uploads,
        &accounts,
        &target,
        vec![cancelled.path.clone()],
    )
    .await;
    println!("batch 3 (cancelled): {:?}", summary.files);
    assert_eq!(summary.state, BatchState::Cancelled, "{summary:?}");
    let file = &summary.files[0];
    assert_eq!(file.state, FileState::Cancelled);
    assert!(file.started);
    assert_eq!(
        file.remote,
        Some(RemoteState::NothingStored),
        "the multipart upload must have been aborted"
    );
    assert!(
        cancel_uploads.unresolved().is_empty(),
        "{:?}",
        cancel_uploads.unresolved()
    );
    assert_eq!(
        store
            .head_object(&target.bucket, &key(CANCELLED))
            .await
            .unwrap_or_else(|e| panic!("head {CANCELLED:?}: {e}")),
        None
    );

    // 4. Download each uploaded file once; `unicode` must still be the
    //    first upload, not `rival`.
    let dest = TempDir::new("it-upload-dest");
    for source in [&empty, &unicode, &multipart] {
        download_and_compare(
            &accounts,
            &target.account,
            &target.bucket,
            &key(source.name),
            &dest,
            source,
        )
        .await;
    }
}

/// An in-memory `CloudAccounts` with the account added as the dashboard
/// adds it, its store replaced by `store` (same account and credentials,
/// with the recorder and the run scope).
fn accounts_with(
    input: &AccountInput,
    creds: &Credentials,
    store: Arc<S3Store>,
) -> (Arc<CloudAccounts>, String) {
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        options(),
    ));
    let id = accounts
        .add(
            input,
            CredentialInput::new(&creds.access_key_id, creds.secret_access_key.expose()),
        )
        .unwrap_or_else(|e| panic!("adding the account: {e:?}"))
        .id;
    accounts.set_connected(&id, store);
    (accounts, id)
}

/// The store every request of a run goes through: one attempt, the
/// recorder, and the run scope.
fn scoped_store(
    resolved: &ResolvedAccount,
    creds: Credentials,
    requests: &Requests,
    scope: &RunScope,
) -> Arc<S3Store> {
    Arc::new(
        S3Store::new(resolved.clone(), creds, options())
            .with_interceptor(requests.clone())
            .with_interceptor(scope.clone()),
    )
}

fn show_requests(bucket: &str, sent: &[String]) {
    let shown: Vec<String> = sent
        .iter()
        .map(|r| r.replacen(&format!("/{bucket}"), "/<bucket>", 1))
        .collect();
    println!("{} requests:", shown.len());
    for r in shown {
        println!("  {r}");
    }
}

/// Counts recorded requests by method and a path fragment.
fn count(sent: &[String], method: &str, fragment: &str) -> usize {
    sent.iter()
        .filter(|r| r.starts_with(&format!("{method} ")) && r.contains(fragment))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: writes to the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_batch() {
    require_confirmation();
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    let run = new_run_id();
    let prefix = run_prefix(&run);
    // Printed before any write, so an interrupted run can be cleaned up
    // with backblaze_b2_upload_cleanup.
    println!(
        "provider {}, region {}, endpoint {}, run id {run}, run prefix {prefix}",
        resolved.profile.id, resolved.region, resolved.endpoint_url
    );
    let requests = Requests::default();
    let scope = RunScope::new(&t.bucket, &prefix);
    let store = scoped_store(&resolved, creds.clone(), &requests, &scope);

    // Never write into anything: no version, delete marker or incomplete
    // multipart upload may exist under the new run prefix.
    let before_versions = list_versions(&store, &t.bucket, &prefix).await;
    let before_uploads = list_open_uploads(&store, &t.bucket, &prefix).await;
    match (&before_versions, &before_uploads) {
        (Ok(v), Ok(u)) if v.is_empty() && u.is_empty() => {
            println!("run prefix: no versions, delete markers or multipart uploads")
        }
        _ => {
            show_requests(&t.bucket, &requests.take());
            fail(format!(
                "nothing uploaded: the run prefix is not empty or cannot be listed: versions {} / multipart uploads {}",
                before_versions.map_or_else(|e| e, |v| v.len().to_string()),
                before_uploads.map_or_else(|e| e, |u| u.len().to_string()),
            ));
        }
    }

    let input = AccountInput {
        label: "Upload validation".into(),
        provider: resolved.profile.id.into(),
        region: resolved.region.clone(),
        endpoint: None,
        bucket: t.bucket.clone(),
    };
    let (accounts, id) = accounts_with(&input, &creds, Arc::clone(&store));
    let target = Target {
        account: id,
        bucket: t.bucket.clone(),
        prefix: prefix.clone(),
    };
    let checks = tokio::spawn(scenario(
        accounts,
        Arc::clone(&store),
        target,
        resolved.profile.limits,
    ))
    .await;

    let cleanup = cleanup_run(&store, &t.bucket, &prefix).await;
    let sent = requests.take();
    show_requests(&t.bucket, &sent);
    for line in &cleanup.problems {
        println!("{}", line.replace(&t.bucket, "<bucket>"));
    }
    println!(
        "after cleanup: versions {:?}, multipart uploads {:?}",
        cleanup.left_versions, cleanup.left_uploads
    );
    let refused = scope.refused();
    if !refused.is_empty() {
        fail(format!(
            "the run scope refused requests (nothing was sent for them): {refused:?}"
        ));
    }
    if let Err(e) = checks {
        fail(format!("the checks failed (cleanup ran first): {e}"));
    }
    if !cleanup.clean() {
        fail(format!(
            "CLEANUP INCOMPLETE under {prefix}; run backblaze_b2_upload_cleanup with {RUN_VAR}={run}"
        ));
    }

    // What the provider held before cleanup: exactly one version per
    // uploaded key (the existing key was not overwritten), no delete
    // marker, nothing for the cancelled key, and no multipart upload left
    // by the cancellation.
    let found = cleanup.found_versions.expect("listed");
    let keys: BTreeSet<&str> = found.iter().map(|v| v.key.as_str()).collect();
    let expected: BTreeSet<String> = [EMPTY, UNICODE, MULTIPART]
        .iter()
        .map(|n| format!("{prefix}{n}"))
        .collect();
    assert_eq!(
        keys,
        expected.iter().map(String::as_str).collect(),
        "keys exactly as uploaded"
    );
    assert_eq!(found.len(), 3, "one version per key: {found:?}");
    assert!(found.iter().all(|v| !v.marker), "{found:?}");
    for (name, size) in [
        (EMPTY, 0),
        (UNICODE, UNICODE_SIZE as i64),
        (MULTIPART, MULTIPART_SIZE as i64),
    ] {
        let row = found
            .iter()
            .find(|v| v.key == format!("{prefix}{name}"))
            .unwrap();
        assert_eq!(row.size, Some(size), "{name}");
    }
    assert_eq!(cleanup.found_uploads, Some(Vec::new()));

    // The requests themselves: multipart in three parts, the cancelled one
    // aborted, every DELETE by id.
    assert_eq!(
        count(&sent, "PUT", "multipart.bin?x-id=UploadPart&"),
        MULTIPART_PARTS as usize
    );
    assert_eq!(
        count(&sent, "DELETE", "cancelled.bin?x-id=AbortMultipartUpload&"),
        1
    );
    assert_eq!(count(&sent, "DELETE", "versionId="), 3);
    assert!(
        sent.len() <= 45,
        "more requests than planned: {}",
        sent.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: removes one upload-validation run from the Backblaze B2 test bucket"]
async fn backblaze_b2_upload_cleanup() {
    require_confirmation();
    let run =
        std::env::var(RUN_VAR).unwrap_or_else(|_| fail(format!("set {RUN_VAR} to the run id")));
    let prefix = run_prefix(&run);
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    let requests = Requests::default();
    let scope = RunScope::new(&t.bucket, &prefix);
    let store = scoped_store(&resolved, creds, &requests, &scope);
    println!("cleaning up run {run} under {prefix}");
    let cleanup = cleanup_run(&store, &t.bucket, &prefix).await;
    show_requests(&t.bucket, &requests.take());
    println!(
        "found: versions {:?}, multipart uploads {:?}",
        cleanup.found_versions, cleanup.found_uploads
    );
    for line in &cleanup.problems {
        println!("{}", line.replace(&t.bucket, "<bucket>"));
    }
    println!(
        "left: versions {:?}, multipart uploads {:?}",
        cleanup.left_versions, cleanup.left_uploads
    );
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    assert!(cleanup.clean(), "cleanup incomplete under {prefix}");
}

/// The same scenario, with the same run scope, against the in-process
/// server (no versioning and no `ListMultipartUploads` there, so the
/// provider-side listings are replaced by the server's files).
#[tokio::test(flavor = "multi_thread")]
async fn upload_scenario_against_the_test_server() {
    const BUCKET: &str = "it-bucket";
    let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
    let resolved = server.resolved("it-local", &[BUCKET]);
    let prefix = run_prefix(&new_run_id());
    let requests = Requests::default();
    let scope = RunScope::new(BUCKET, &prefix);
    let store = scoped_store(&resolved, server.credentials(), &requests, &scope);
    let input = AccountInput {
        label: "Local".into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(server.endpoint.clone()),
        bucket: BUCKET.into(),
    };
    let (accounts, id) = accounts_with(&input, &server.credentials(), Arc::clone(&store));
    let target = Target {
        account: id,
        bucket: BUCKET.into(),
        prefix: prefix.clone(),
    };
    // Backblaze's limits, so the plan is the one the provider run uses.
    scenario(accounts, Arc::clone(&store), target, BACKBLAZE_B2.limits).await;

    let sent = requests.take();
    show_requests(BUCKET, &sent);
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    let ops = server.ops();
    for part in 1..=MULTIPART_PARTS {
        assert!(ops.contains(&format!("UploadPart#{part}")), "{ops:?}");
    }
    assert_eq!(
        ops.iter().filter(|o| *o == "AbortMultipartUpload").count(),
        1,
        "{ops:?}"
    );
    assert!(server.open_multipart_files().is_empty());
    assert!(
        !server
            .object_path(BUCKET, &format!("{prefix}{CANCELLED}"))
            .exists()
    );
    assert_eq!(
        count(&sent, "PUT", "multipart.bin?x-id=UploadPart&"),
        MULTIPART_PARTS as usize
    );
    assert_eq!(
        count(&sent, "DELETE", "cancelled.bin?x-id=AbortMultipartUpload&"),
        1
    );
    assert_eq!(count(&sent, "DELETE", ""), 1, "no other delete");
}

/// The run scope refuses, before sending, what the run must never do.
#[tokio::test(flavor = "multi_thread")]
async fn run_scope_refuses_requests_outside_the_run() {
    const BUCKET: &str = "it-bucket";
    let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
    let resolved = server.resolved("it-local", &[BUCKET]);
    let prefix = run_prefix("scope-test");
    let requests = Requests::default();
    let scope = RunScope::new(BUCKET, &prefix);
    let store = scoped_store(&resolved, server.credentials(), &requests, &scope);
    let dir = TempDir::new("it-upload-scope");
    let file = dir.file("f", 10, 1);

    let outside = "sotyware-it/other-run/empty.bin";
    assert!(store.upload_file(BUCKET, outside, &file).await.is_err());
    let unknown = format!("{prefix}not-a-run-file.bin");
    assert!(store.upload_file(BUCKET, &unknown, &file).await.is_err());
    // A plain delete would only hide a version on B2.
    assert!(
        store
            .delete_object(BUCKET, &format!("{prefix}{EMPTY}"), None)
            .await
            .is_err()
    );
    assert!(store.list_objects(BUCKET, "sotyware-it/").await.is_err());
    assert!(list_versions(&store, BUCKET, "sotyware-it/").await.is_err());
    assert!(store.check_bucket(BUCKET).await.is_err());
    println!("refused: {:#?}", scope.refused());
    assert_eq!(scope.refused().len(), 6, "{:?}", scope.refused());
    assert!(
        server.ops().is_empty(),
        "nothing may reach the server: {:?}",
        server.ops()
    );

    // The two listings of exactly the run prefix, as the SDK really builds
    // them, pass the scope and reach the server (which does not implement
    // them, so they fail there, not in the scope).
    let _ = list_versions(&store, BUCKET, &prefix).await;
    let _ = list_open_uploads(&store, BUCKET, &prefix).await;
    assert_eq!(scope.refused().len(), 6, "{:?}", scope.refused());
    let sent = requests.take();
    println!("listings: {sent:?}");
    let listings: Vec<&String> = sent.iter().filter(|r| r.contains(" -> ")).collect();
    assert_eq!(listings.len(), 2, "both listings were sent: {sent:?}");
    assert!(listings[0].starts_with("GET ") && listings[0].contains("versions"));
    assert!(listings[1].starts_with("GET ") && listings[1].contains("uploads"));
}

#[test]
fn writes_need_the_exact_confirmation() {
    assert!(writes_confirmed(Some("upload-validation")));
    for v in [
        None,
        Some(""),
        Some("1"),
        Some("yes"),
        Some("upload-validation "),
    ] {
        assert!(!writes_confirmed(v), "{v:?}");
    }
}

#[test]
fn run_prefix_stays_under_the_upload_root() {
    assert_eq!(
        run_prefix("123-45"),
        "sotyware-it/upload-validation/123-45/"
    );
    for bad in ["", "..", "a/b", "../browse-validation", "a b", "a%2Fb"] {
        assert!(
            std::panic::catch_unwind(|| run_prefix(bad)).is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn in_scope_allows_only_the_run() {
    let b = "bkt";
    let p = "sotyware-it/upload-validation/1-2/";
    let ok = [
        (
            "GET",
            "/bkt?versions&prefix=sotyware-it%2Fupload-validation%2F1-2%2F",
        ),
        (
            "GET",
            "/bkt/?uploads&prefix=sotyware-it%2Fupload-validation%2F1-2%2F&key-marker=x",
        ),
        ("HEAD", "/bkt/sotyware-it/upload-validation/1-2/empty.bin"),
        (
            "PUT",
            "/bkt/sotyware-it/upload-validation/1-2/gr%C3%B6%C3%9Fe%20%2Btest.bin",
        ),
        (
            "POST",
            "/bkt/sotyware-it/upload-validation/1-2/multipart.bin?uploads",
        ),
        (
            "PUT",
            "/bkt/sotyware-it/upload-validation/1-2/multipart.bin?partNumber=2&uploadId=u",
        ),
        (
            "DELETE",
            "/bkt/sotyware-it/upload-validation/1-2/cancelled.bin?uploadId=u",
        ),
        (
            "DELETE",
            "/bkt/sotyware-it/upload-validation/1-2/empty.bin?versionId=v",
        ),
        (
            "GET",
            "/bkt/sotyware-it/upload-validation/1-2/multipart.bin",
        ),
    ];
    for (m, path) in ok {
        assert_eq!(in_scope(m, path, b, p), Ok(()), "{m} {path}");
    }
    let refused = [
        // Listings: other prefix, no prefix, a parent, ListObjectsV2,
        // ListBuckets, a delimiter, not a GET.
        ("GET", "/bkt?versions&prefix=sotyware-it%2F"),
        ("GET", "/bkt?versions"),
        (
            "GET",
            "/bkt?uploads&prefix=sotyware-it%2Fupload-validation%2F",
        ),
        (
            "GET",
            "/bkt?list-type=2&prefix=sotyware-it%2Fupload-validation%2F1-2%2F",
        ),
        ("GET", "/"),
        (
            "GET",
            "/bkt?versions&prefix=sotyware-it%2Fupload-validation%2F1-2%2F&delimiter=%2F",
        ),
        (
            "DELETE",
            "/bkt?versions&prefix=sotyware-it%2Fupload-validation%2F1-2%2F",
        ),
        ("POST", "/bkt?delete"),
        ("HEAD", "/bkt"),
        // Keys: other bucket, other run, a parent escape, unknown name, the
        // prefix itself.
        ("HEAD", "/other/sotyware-it/upload-validation/1-2/empty.bin"),
        ("HEAD", "/bkt/sotyware-it/upload-validation/1-3/empty.bin"),
        ("PUT", "/bkt/sotyware-it/upload-validation/1-2/../empty.bin"),
        ("PUT", "/bkt/sotyware-it/upload-validation/1-2/a/empty.bin"),
        ("PUT", "/bkt/sotyware-it/upload-validation/1-2/other.bin"),
        ("PUT", "/bkt/sotyware-it/upload-validation/1-2/"),
        // Deletes without an id, or with an empty one.
        ("DELETE", "/bkt/sotyware-it/upload-validation/1-2/empty.bin"),
        (
            "DELETE",
            "/bkt/sotyware-it/upload-validation/1-2/empty.bin?versionId=",
        ),
        (
            "DELETE",
            "/bkt/sotyware-it/upload-validation/1-2/empty.bin?uploadId=",
        ),
        // Other methods; a malformed escape.
        ("PATCH", "/bkt/sotyware-it/upload-validation/1-2/empty.bin"),
        (
            "HEAD",
            "/bkt/sotyware-it/upload-validation/1-2/empty%G0.bin",
        ),
    ];
    for (m, path) in refused {
        assert!(in_scope(m, path, b, p).is_err(), "{m} {path}");
    }
}

#[test]
fn cleanup_touches_only_this_runs_keys() {
    let p = "sotyware-it/upload-validation/1-2/";
    let row = |key: &str, id: &str, marker: bool| VersionRow {
        key: key.into(),
        version_id: id.into(),
        size: None,
        marker,
    };
    let rows = vec![
        row(&format!("{p}empty.bin"), "v1", false),
        row(&format!("{p}größe +test.bin"), "v2", false),
        row(&format!("{p}multipart.bin"), "v3", true),
        // Not ours: unknown name, deeper key, another run, no version id.
        row(&format!("{p}other.bin"), "v4", false),
        row(&format!("{p}x/empty.bin"), "v5", false),
        row("sotyware-it/upload-validation/1-3/empty.bin", "v6", false),
        row(&format!("{p}cancelled.bin"), "", false),
    ];
    let (delete, keep) = split_versions(p, &rows);
    let ids = |v: &[&VersionRow]| v.iter().map(|r| r.version_id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&delete), ["v1", "v2", "v3"]);
    assert_eq!(ids(&keep), ["v4", "v5", "v6", ""]);

    let up = |key: &str, id: &str| OpenUpload {
        key: key.into(),
        upload_id: id.into(),
    };
    let uploads = vec![
        up(&format!("{p}cancelled.bin"), "u1"),
        up(&format!("{p}other.bin"), "u2"),
        up(&format!("{p}multipart.bin"), ""),
        up("sotyware-it/x", "u3"),
    ];
    let (abort, keep) = split_uploads(p, &uploads);
    assert_eq!(
        abort
            .iter()
            .map(|u| u.upload_id.as_str())
            .collect::<Vec<_>>(),
        ["u1"]
    );
    assert_eq!(keep.len(), 3);
}

#[test]
fn query_and_percent_decoding() {
    assert_eq!(percent_decode("a%2Fb%20c"), Some("a/b c".into()));
    assert_eq!(percent_decode("gr%C3%B6%C3%9Fe"), Some("größe".into()));
    assert_eq!(percent_decode("bad%G0"), None);
    assert_eq!(percent_decode("cut%2"), None);
    assert_eq!(percent_decode("%FF"), None);
    assert_eq!(
        query_param("versions&prefix=a%2F", "versions"),
        Some(String::new())
    );
    assert_eq!(
        query_param("versions&prefix=a%2F", "prefix"),
        Some("a/".into())
    );
    assert_eq!(query_param("versions&prefix=a%2F", "delimiter"), None);
}

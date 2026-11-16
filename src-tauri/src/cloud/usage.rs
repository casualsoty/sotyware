//! Storage usage of each account's bucket, measured when the user asks, and
//! the estimated cost (ADR-041).
//!
//! No provider sotyware supports reports a bucket's size or the account's
//! bill through an API (Backblaze's `b2_list_buckets` and the S3 API have no
//! size field; Hetzner shows bucket statistics in its console only), so
//! sotyware **measures** a bucket by listing it, read-only:
//!
//! 1. every version and delete marker (`ListObjectVersions`, 1,000 per page):
//!    current versions are the files, older versions (Backblaze keeps them
//!    in every bucket; a "deleted" file's versions stay) are stored data
//!    too, delete markers store nothing. A service or key that cannot list
//!    versions falls back to the current objects (`ListObjectsV2`), and the
//!    measurement says older versions are not included;
//! 2. unfinished multipart uploads (`ListMultipartUploads`) and, for up to
//!    [`MAX_PART_LISTINGS`] of them, the parts they have stored
//!    (`ListParts`): stored data that no file listing shows.
//!
//! Each listing is one request per 1,000 entries (class C at Backblaze,
//! free like every call sotyware makes; free at Hetzner). A measurement
//! runs in its own slot, one at a time, reports progress on a per-call
//! channel and can be cancelled; it stops after [`MAX_PAGES`] pages and is
//! then marked incomplete (a lower bound). Results are kept in
//! `storage-usage.sqlite3` (the newest [`KEPT`] per account): the history
//! shown is sotyware's own measurements, never the provider's billing.
//!
//! The estimate (`pricing`) is computed per provider account: buckets of
//! one provider (Backblaze: one region) are assumed to share one account's
//! free allowance and base fee, which the page states.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use super::dashboard::{AccountInfo, CloudAccounts, CommandError, ErrorKind, blocking};
use super::download::CancelOutcome;
use super::manage::store_for;
use super::operations::operation_error;
use super::pricing::{self, AccountScope, Estimate, PricingInfo, SMALL_OBJECT_BYTES, UsageInput};
use super::s3::{S3Store, StorageError};
use crate::features::StoreError;
use crate::journal::now_ms;

/// Most listing pages one measurement reads (20 million entries).
pub const MAX_PAGES: u64 = 20_000;
/// Most unfinished uploads whose parts are listed.
pub const MAX_PART_LISTINGS: u64 = 100;
/// Measurements kept per account.
pub const KEPT: usize = 100;
/// Measurements per account in the history the page draws.
pub const HISTORY: usize = 30;

/// One measurement of one account's bucket.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub account: String,
    pub bucket: String,
    /// When it finished, milliseconds since 1970.
    pub measured_ms: i64,
    pub duration_ms: u64,
    /// Current versions (the files) and their size.
    pub objects: u64,
    pub bytes: u64,
    /// Whether versions could be listed; when not, the three fields below
    /// are zero and older versions, if any, are not included.
    pub versions_listed: bool,
    pub noncurrent_versions: u64,
    pub noncurrent_bytes: u64,
    pub delete_markers: u64,
    /// Whether unfinished multipart uploads could be listed.
    pub incomplete_listed: bool,
    pub incomplete_uploads: u64,
    /// What the listed parts of unfinished uploads store.
    pub incomplete_bytes: u64,
    /// Unfinished uploads whose parts were not listed (above the limit, or
    /// refused).
    pub incomplete_unmeasured: u64,
    /// Stored versions smaller than [`SMALL_OBJECT_BYTES`], and their size
    /// (for providers that bill a minimum object size).
    pub small_objects: u64,
    pub small_bytes: u64,
    /// False when the measurement stopped at [`MAX_PAGES`]: a lower bound.
    pub complete: bool,
    /// Listing requests sent.
    pub requests: u64,
    /// What the user should know about this measurement.
    pub notes: Vec<String>,
}

impl Measurement {
    /// Everything stored: current and older versions and the parts of
    /// unfinished uploads.
    pub fn stored_bytes(&self) -> u64 {
        self.bytes
            .saturating_add(self.noncurrent_bytes)
            .saturating_add(self.incomplete_bytes)
    }
}

/// Where measurements are kept. Implemented by `storage::SqliteUsageStore`.
pub trait UsageStore: Send {
    fn add(&mut self, m: &Measurement) -> Result<(), StoreError>;
    /// The newest `limit` measurements of `account`, newest first.
    fn history(&mut self, account: &str, limit: usize) -> Result<Vec<Measurement>, StoreError>;
}

/// What a measurement reports while it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// `versions`, `objects` (the fallback listing) or `uploads`.
    pub phase: &'static str,
    pub entries: u64,
    pub bytes: u64,
    pub requests: u64,
}

/// Whether a failed first versions page means "this service or key cannot
/// list versions" (then the current objects are listed instead), rather
/// than a failure that stops the measurement.
fn versions_unsupported(e: &StorageError) -> bool {
    match e {
        StorageError::AccessDenied(_) => true,
        StorageError::Provider { status, .. } => matches!(status, 400 | 405 | 501),
        _ => false,
    }
}

fn tally(m: &mut Measurement, size: u64, current: bool) {
    if current {
        m.objects += 1;
        m.bytes = m.bytes.saturating_add(size);
    } else {
        m.noncurrent_versions += 1;
        m.noncurrent_bytes = m.noncurrent_bytes.saturating_add(size);
    }
    if size < SMALL_OBJECT_BYTES {
        m.small_objects += 1;
        m.small_bytes = m.small_bytes.saturating_add(size);
    }
}

/// Measures `bucket` through `store`. Read-only. `cancel` is checked between
/// requests; `progress` hears about every page.
pub async fn measure(
    store: &S3Store,
    account: &str,
    bucket: &str,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<Measurement, StorageError> {
    let started = Instant::now();
    let mut m = Measurement {
        account: account.to_owned(),
        bucket: bucket.to_owned(),
        complete: true,
        ..Measurement::default()
    };
    let stopped = |cancel: &AtomicBool| {
        if cancel.load(Ordering::SeqCst) {
            Err(StorageError::Cancelled)
        } else {
            Ok(())
        }
    };
    let mut pages = 0u64;

    // 1. Every version, or the current objects where versions cannot be listed.
    let mut marker = None;
    loop {
        stopped(cancel)?;
        match store.usage_versions_page(bucket, marker.clone()).await {
            Ok(page) => {
                m.requests += 1;
                pages += 1;
                m.versions_listed = true;
                for e in &page.entries {
                    if e.delete_marker {
                        m.delete_markers += 1;
                    } else {
                        tally(&mut m, e.size, e.is_latest);
                    }
                }
                progress(Progress {
                    phase: "versions",
                    entries: m.objects + m.noncurrent_versions + m.delete_markers,
                    bytes: m.bytes + m.noncurrent_bytes,
                    requests: m.requests,
                });
                match page.next {
                    Some(next) if pages < MAX_PAGES => marker = Some(next),
                    Some(_) => {
                        m.complete = false;
                        break;
                    }
                    None => break,
                }
            }
            Err(e) if pages == 0 && versions_unsupported(&e) => {
                m.requests += 1;
                m.notes.push(
                    "This service or key does not list object versions: only the current files are counted, older versions (if any) are not."
                        .into(),
                );
                current_objects(store, bucket, cancel, progress, &mut m, &mut pages).await?;
                break;
            }
            Err(e) => return Err(e),
        }
    }
    if !m.complete {
        m.notes.push(format!(
            "The measurement stopped after {} entries; the bucket holds more, so the figures are a lower bound.",
            MAX_PAGES * 1000
        ));
    }

    // 2. Unfinished multipart uploads and the parts they store.
    let mut after = None;
    let mut part_listings = 0u64;
    'uploads: loop {
        stopped(cancel)?;
        let page = match store
            .list_incomplete_uploads(bucket, "", after.clone())
            .await
        {
            Ok(page) => page,
            Err(e @ (StorageError::Network(_) | StorageError::Timeout(_))) => return Err(e),
            Err(_) => {
                m.requests += 1;
                if !m.incomplete_listed {
                    m.notes.push(
                        "Unfinished uploads could not be listed with this key; their stored parts, if any, are not included."
                            .into(),
                    );
                }
                break;
            }
        };
        m.requests += 1;
        m.incomplete_listed = true;
        for u in &page.uploads {
            m.incomplete_uploads += 1;
            if part_listings >= MAX_PART_LISTINGS {
                m.incomplete_unmeasured += 1;
                continue;
            }
            part_listings += 1;
            let mut part_marker = None;
            loop {
                stopped(cancel)?;
                match store
                    .list_parts_page(bucket, &u.key, &u.upload_id, part_marker.clone())
                    .await
                {
                    Ok(parts) => {
                        m.requests += 1;
                        m.incomplete_bytes = m.incomplete_bytes.saturating_add(parts.bytes);
                        match parts.next {
                            Some(n) => part_marker = Some(n),
                            None => break,
                        }
                    }
                    Err(e @ (StorageError::Network(_) | StorageError::Timeout(_))) => {
                        return Err(e);
                    }
                    Err(_) => {
                        m.requests += 1;
                        m.incomplete_unmeasured += 1;
                        break;
                    }
                }
            }
            progress(Progress {
                phase: "uploads",
                entries: m.incomplete_uploads,
                bytes: m.incomplete_bytes,
                requests: m.requests,
            });
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break 'uploads,
        }
    }
    if m.incomplete_unmeasured > 0 {
        m.notes.push(format!(
            "The stored parts of {} unfinished upload(s) were not measured.",
            m.incomplete_unmeasured
        ));
    }
    m.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    m.measured_ms = now_ms();
    Ok(m)
}

/// The fallback: the current objects only (`ListObjectsV2`).
async fn current_objects(
    store: &S3Store,
    bucket: &str,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
    m: &mut Measurement,
    pages: &mut u64,
) -> Result<(), StorageError> {
    let mut token: Option<String> = None;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(StorageError::Cancelled);
        }
        let page = store
            .list_recursive_page(bucket, "", token.as_deref(), 1000)
            .await?;
        m.requests += 1;
        *pages += 1;
        for o in &page.objects {
            tally(m, o.size, true);
        }
        progress(Progress {
            phase: "objects",
            entries: m.objects,
            bytes: m.bytes,
            requests: m.requests,
        });
        match page.next_token {
            Some(next) if *pages < MAX_PAGES => token = Some(next),
            Some(_) => {
                m.complete = false;
                return Ok(());
            }
            None => return Ok(()),
        }
    }
}

/// How a failed measurement is shown: sotyware's own sentences.
pub fn usage_error(e: &StorageError) -> CommandError {
    match e {
        StorageError::AccessDenied(_) => CommandError::new(
            ErrorKind::AccessDenied,
            "the provider does not let this key list the bucket's contents",
        ),
        StorageError::NotFound(_) => CommandError::new(
            ErrorKind::BucketUnavailable,
            "the bucket was not found at this endpoint (or this key may not see it)",
        ),
        StorageError::Provider {
            status: 429 | 503, ..
        } => CommandError::new(
            ErrorKind::Busy,
            "the provider asked sotyware to slow down (too many requests); try again later",
        ),
        StorageError::Cancelled => CommandError::new(ErrorKind::Other, "cancelled"),
        other => operation_error(other),
    }
}

/// A point of an account's history, for the page's chart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub measured_ms: i64,
    pub stored_bytes: u64,
    pub objects: u64,
    pub complete: bool,
}

/// One account as the usage panel shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub account: String,
    pub label: String,
    pub provider: String,
    pub provider_name: Option<&'static str>,
    pub region: String,
    pub bucket: Option<String>,
    /// The provider account group it is estimated in.
    pub group: String,
    pub latest: Option<Measurement>,
    /// Oldest first.
    pub history: Vec<HistoryPoint>,
}

/// The buckets assumed to belong to one provider account, and their estimate.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderGroup {
    pub key: String,
    pub provider: String,
    pub provider_name: Option<&'static str>,
    /// Backblaze: the region the account lives in.
    pub region: Option<String>,
    pub accounts: Vec<String>,
    pub measured: usize,
    pub stored_bytes: u64,
    pub objects: u64,
    /// Downloads per 30 days assumed for the estimate, in GB.
    pub downloads_gb: f64,
    pub estimate: Option<Estimate>,
    pub pricing: Option<PricingInfo>,
    /// Why there is no estimate.
    pub no_estimate: Option<String>,
    /// The assumption about which buckets share one account.
    pub scope_note: String,
}

/// The estimated total in one currency (never summed across currencies).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyTotal {
    pub currency: String,
    pub total: f64,
}

/// A measurement that is running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningInfo {
    pub task: String,
    pub account: String,
    pub started_ms: i64,
}

/// What `storage_usage_overview` returns.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageOverview {
    pub accounts: Vec<AccountUsage>,
    pub groups: Vec<ProviderGroup>,
    /// Over every measured account.
    pub stored_bytes: u64,
    pub objects: u64,
    pub measured: usize,
    pub costs: Vec<CurrencyTotal>,
    pub running: Option<RunningInfo>,
    /// Why measurements could not be read or will not be kept.
    pub storage_error: Option<String>,
}

/// The group an account is estimated in.
fn group_key(info: &AccountInfo, scope: Option<AccountScope>) -> (String, Option<String>) {
    match scope {
        Some(AccountScope::Region) => (
            format!("{}/{}", info.provider, info.region),
            Some(info.region.clone()),
        ),
        _ => (info.provider.clone(), None),
    }
}

fn scope_note(name: &str, scope: Option<AccountScope>, region: Option<&str>) -> String {
    match (scope, region) {
        (Some(AccountScope::Region), Some(r)) => format!(
            "Assumes the {name} buckets in region {r} shown here belong to one {name} account, which has the free allowance once; buckets in several accounts each have their own."
        ),
        (Some(AccountScope::Provider), _) => format!(
            "Assumes every {name} bucket shown here belongs to one {name} account, which pays the base price once; buckets in several accounts each pay it."
        ),
        _ => format!("{name} has no price list in sotyware: usage only, no estimate."),
    }
}

/// The overview from the accounts, their measurements (newest first) and
/// the downloads the user assumes per group (GB per 30 days). Pure.
pub fn overview(
    accounts: &[AccountInfo],
    measurements: &HashMap<String, Vec<Measurement>>,
    downloads_gb: &HashMap<String, f64>,
) -> UsageOverview {
    let mut groups: BTreeMap<String, ProviderGroup> = BTreeMap::new();
    let mut inputs: BTreeMap<String, UsageInput> = BTreeMap::new();
    let mut out = Vec::new();
    let (mut stored, mut objects, mut measured) = (0u64, 0u64, 0usize);
    for info in accounts {
        let price = pricing::for_provider(&info.provider);
        let scope = price.as_ref().map(|p| p.account_scope);
        let (key, region) = group_key(info, scope);
        let name = info.provider_name.unwrap_or(info.provider.as_str());
        let history = measurements.get(&info.id).cloned().unwrap_or_default();
        // A measurement of another bucket (the account was changed) is history.
        let latest = history
            .iter()
            .find(|m| Some(&m.bucket) == info.bucket.as_ref())
            .cloned();
        let group = groups.entry(key.clone()).or_insert_with(|| ProviderGroup {
            key: key.clone(),
            provider: info.provider.clone(),
            provider_name: info.provider_name,
            region: region.clone(),
            accounts: Vec::new(),
            measured: 0,
            stored_bytes: 0,
            objects: 0,
            downloads_gb: downloads_gb
                .get(&key)
                .copied()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(0.0),
            estimate: None,
            pricing: price.as_ref().map(PricingInfo::from),
            no_estimate: None,
            scope_note: scope_note(name, scope, region.as_deref()),
        });
        group.accounts.push(info.id.clone());
        if let Some(m) = &latest {
            group.measured += 1;
            group.stored_bytes = group.stored_bytes.saturating_add(m.stored_bytes());
            group.objects = group.objects.saturating_add(m.objects);
            let input = inputs.entry(key.clone()).or_default();
            input.stored_bytes = input.stored_bytes.saturating_add(m.stored_bytes());
            input.small_objects = input.small_objects.saturating_add(m.small_objects);
            input.small_bytes = input.small_bytes.saturating_add(m.small_bytes);
            stored = stored.saturating_add(m.stored_bytes());
            objects = objects.saturating_add(m.objects);
            measured += 1;
        }
        out.push(AccountUsage {
            account: info.id.clone(),
            label: info.label.clone(),
            provider: info.provider.clone(),
            provider_name: info.provider_name,
            region: info.region.clone(),
            bucket: info.bucket.clone(),
            group: key,
            latest,
            history: history
                .iter()
                .take(HISTORY)
                .rev()
                .map(|m| HistoryPoint {
                    measured_ms: m.measured_ms,
                    stored_bytes: m.stored_bytes(),
                    objects: m.objects,
                    complete: m.complete,
                })
                .collect(),
        });
    }
    let mut costs: BTreeMap<String, f64> = BTreeMap::new();
    for group in groups.values_mut() {
        let Some(price) = pricing::for_provider(&group.provider) else {
            group.no_estimate = Some(format!(
                "sotyware has no price list for {}; only the usage is shown.",
                group.provider_name.unwrap_or(group.provider.as_str())
            ));
            continue;
        };
        if group.measured == 0 {
            group.no_estimate = Some("Measure a bucket to see an estimate.".into());
            continue;
        }
        let mut input = inputs.get(&group.key).copied().unwrap_or_default();
        input.egress_bytes = (group.downloads_gb * 1e9) as u64;
        let mut e = pricing::estimate(&price, &input);
        e.assumptions.push(group.scope_note.clone());
        let unmeasured = group.accounts.len() - group.measured;
        if unmeasured > 0 {
            e.assumptions.push(format!(
                "{unmeasured} of these {} bucket(s) not measured yet: not included.",
                group.accounts.len()
            ));
        }
        *costs.entry(e.currency.clone()).or_default() += e.total;
        group.estimate = Some(e);
    }
    UsageOverview {
        accounts: out,
        groups: groups.into_values().collect(),
        stored_bytes: stored,
        objects,
        measured,
        costs: costs
            .into_iter()
            .map(|(currency, total)| CurrencyTotal { currency, total })
            .collect(),
        running: None,
        storage_error: None,
    }
}

/// What a measurement sends on its channel.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub task: String,
    /// Increases by one per event; the final one is `u64::MAX`.
    pub seq: u64,
    #[serde(flatten)]
    pub body: UsageEventBody,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UsageEventBody {
    Progress {
        progress: Progress,
    },
    /// The end: a measurement (stored), or why there is none.
    Finished {
        measurement: Option<Box<Measurement>>,
        error: Option<CommandError>,
        cancelled: bool,
        /// The measurement could not be kept (shown, not stored).
        save_error: Option<String>,
    },
}

/// Where events go: the page's channel, or a test recorder.
pub trait UsageSink: Send + Sync + 'static {
    fn emit(&self, event: UsageEvent);
}

impl UsageSink for Channel<UsageEvent> {
    fn emit(&self, event: UsageEvent) {
        if let Err(e) = self.send(event) {
            eprintln!("sotyware: failed to send a usage event to the dashboard: {e}");
        }
    }
}

struct Running {
    info: RunningInfo,
    cancel: Arc<AtomicBool>,
}

/// Managed state: the measurements kept, and the one measurement running.
pub struct StorageUsage {
    store: Mutex<Result<Box<dyn UsageStore>, String>>,
    running: Mutex<Option<Running>>,
    next: AtomicU64,
}

impl StorageUsage {
    pub fn load(store: Result<Box<dyn UsageStore>, StoreError>) -> Arc<Self> {
        let store = store.map_err(|e| {
            eprintln!("sotyware: storage usage history unavailable: {e}");
            e.to_string()
        });
        Arc::new(Self {
            store: Mutex::new(store),
            running: Mutex::new(None),
            next: AtomicU64::new(1),
        })
    }

    fn running_info(&self) -> Option<RunningInfo> {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|r| r.info.clone())
    }

    /// Every account's measurements, newest first (blocking: SQLite).
    fn histories(
        &self,
        accounts: &[AccountInfo],
    ) -> (HashMap<String, Vec<Measurement>>, Option<String>) {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let store = match store.as_mut() {
            Ok(s) => s,
            Err(e) => return (HashMap::new(), Some(e.clone())),
        };
        let mut all = HashMap::new();
        for a in accounts {
            match store.history(&a.id, HISTORY.max(1)) {
                Ok(list) => {
                    all.insert(a.id.clone(), list);
                }
                Err(e) => return (all, Some(e.to_string())),
            }
        }
        (all, None)
    }

    /// The overview (blocking: SQLite).
    pub fn overview(
        &self,
        accounts: &[AccountInfo],
        downloads_gb: &HashMap<String, f64>,
    ) -> UsageOverview {
        let (histories, error) = self.histories(accounts);
        let mut o = overview(accounts, &histories, downloads_gb);
        o.storage_error = error;
        o.running = self.running_info();
        o
    }

    fn save(&self, m: &Measurement) -> Result<(), String> {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        match store.as_mut() {
            Ok(s) => s.add(m).map_err(|e| e.to_string()),
            Err(e) => Err(e.clone()),
        }
    }

    /// Takes the slot for `account`, or says which measurement holds it.
    fn reserve(&self, account: &str) -> Result<(String, Arc<AtomicBool>), CommandError> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = running.as_ref() {
            return Err(CommandError::new(
                ErrorKind::Busy,
                format!(
                    "another measurement is running (account '{}'); wait for it or cancel it",
                    r.info.account
                ),
            ));
        }
        let task = format!("usage-{}", self.next.fetch_add(1, Ordering::SeqCst));
        let cancel = Arc::new(AtomicBool::new(false));
        *running = Some(Running {
            info: RunningInfo {
                task: task.clone(),
                account: account.to_owned(),
                started_ms: now_ms(),
            },
            cancel: Arc::clone(&cancel),
        });
        Ok((task, cancel))
    }

    fn free(&self) {
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Starts measuring `account`'s bucket; events go to `sink`.
    pub async fn start(
        self: &Arc<Self>,
        accounts: Arc<CloudAccounts>,
        account: &str,
        sink: Arc<dyn UsageSink>,
    ) -> Result<String, CommandError> {
        let list = accounts.list();
        let info = list
            .accounts
            .iter()
            .find(|a| a.id == account)
            .cloned()
            .ok_or_else(|| {
                CommandError::new(ErrorKind::UnknownAccount, "this account no longer exists")
            })?;
        if let Some(e) = &info.config_error {
            return Err(CommandError::new(
                ErrorKind::InvalidConfiguration,
                e.clone(),
            ));
        }
        let bucket = info.bucket.clone().ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidConfiguration,
                "this account has no bucket",
            )
        })?;
        let (task, cancel) = self.reserve(account)?;
        let store = match store_for(&accounts, account, &bucket).await {
            Ok(s) => s,
            Err(e) => {
                self.free();
                return Err(e);
            }
        };
        let this = Arc::clone(self);
        let (t, id) = (task.clone(), account.to_owned());
        tauri::async_runtime::spawn(async move {
            let mut seq = 0u64;
            let mut progress = |p: Progress| {
                seq += 1;
                sink.emit(UsageEvent {
                    task: t.clone(),
                    seq,
                    body: UsageEventBody::Progress { progress: p },
                });
            };
            let result = measure(&store, &id, &bucket, &cancel, &mut progress).await;
            let body = match result {
                Ok(m) => {
                    let saved = {
                        let (this, m) = (Arc::clone(&this), m.clone());
                        tokio::task::spawn_blocking(move || this.save(&m))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    };
                    if let Err(e) = &saved {
                        eprintln!("sotyware: cannot keep the storage usage measurement: {e}");
                    }
                    UsageEventBody::Finished {
                        measurement: Some(Box::new(m)),
                        error: None,
                        cancelled: false,
                        save_error: saved.err(),
                    }
                }
                Err(StorageError::Cancelled) => UsageEventBody::Finished {
                    measurement: None,
                    error: None,
                    cancelled: true,
                    save_error: None,
                },
                Err(e) => {
                    let error = usage_error(&e);
                    eprintln!(
                        "sotyware: storage usage measurement failed: {}",
                        error.message
                    );
                    UsageEventBody::Finished {
                        measurement: None,
                        error: Some(error),
                        cancelled: false,
                        save_error: None,
                    }
                }
            };
            this.free();
            sink.emit(UsageEvent {
                task: t,
                seq: u64::MAX,
                body,
            });
        });
        Ok(task)
    }

    /// Cancels the measurement `task`; it stops after the request in flight.
    pub fn cancel(&self, task: &str) -> CancelOutcome {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        match running.as_ref() {
            Some(r) if r.info.task == task => {
                r.cancel.store(true, Ordering::SeqCst);
                CancelOutcome::Cancelling
            }
            _ => CancelOutcome::NotActive,
        }
    }

    /// Quitting: a running measurement stops (nothing to clean up: it only reads).
    pub fn shutdown(&self) {
        if let Some(r) = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// The usage panel's data: every account with its latest measurement and
/// history, the per-provider-account estimates and the running measurement.
/// `downloads` are the downloads per 30 days the user assumes, in GB, by
/// group key. Allowed by `allow-storage-usage-overview`.
#[tauri::command]
pub async fn storage_usage_overview(
    usage: State<'_, Arc<StorageUsage>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    downloads: Option<HashMap<String, f64>>,
) -> Result<UsageOverview, CommandError> {
    let (usage, accounts) = (Arc::clone(&usage), Arc::clone(&accounts));
    blocking(move || {
        let list = accounts.list();
        Ok(usage.overview(&list.accounts, &downloads.unwrap_or_default()))
    })
    .await
}

/// Starts measuring an account's bucket (read-only listings); returns the
/// task id. One measurement at a time. Allowed by
/// `allow-measure-storage-usage`.
#[tauri::command]
pub async fn measure_storage_usage(
    usage: State<'_, Arc<StorageUsage>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    account: String,
    on_event: Channel<UsageEvent>,
) -> Result<String, CommandError> {
    Arc::clone(&usage)
        .start(Arc::clone(&accounts), &account, Arc::new(on_event))
        .await
}

/// Cancels a running measurement. Allowed by `allow-cancel-storage-usage`.
#[tauri::command]
pub fn cancel_storage_usage(usage: State<'_, Arc<StorageUsage>>, task: String) -> CancelOutcome {
    usage.cancel(&task)
}

#[cfg(test)]
mod tests;

//! Tests of storage usage measurement (ADR-041) against sotyware's
//! in-process S3 servers (the in-memory versioned one, and s3s-fs, which
//! lists no versions but keeps unfinished multipart uploads), and of the
//! overview and its estimates. No real provider.

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInfo, AccountInput, CredentialInput};
use super::super::s3::ClientOptions;
use super::super::test_server::{TestServer, Versioned};
use super::*;

const KEY_ID: &str = "usage-key";
const SECRET: &str = "usage-Secret-0123456789";
const BUCKET: &str = "usage-bucket";

fn accounts_for(server: &TestServer, secret: &str) -> Arc<CloudAccounts> {
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        ClientOptions {
            max_attempts: 1,
            connect_timeout: Duration::from_secs(5),
            ..ClientOptions::default()
        },
    ));
    accounts
        .add(
            &AccountInput {
                label: "Local".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(server.endpoint.clone()),
                bucket: BUCKET.into(),
            },
            CredentialInput::new(KEY_ID, secret),
        )
        .unwrap();
    accounts
}

async fn versioned() -> (TestServer, Versioned, Arc<CloudAccounts>) {
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    let accounts = accounts_for(&server, SECRET);
    (server, data, accounts)
}

async fn run(accounts: &Arc<CloudAccounts>) -> (Result<Measurement, StorageError>, Vec<Progress>) {
    let store = store_for(accounts, "local", BUCKET).await.unwrap();
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let mut progress = |p: Progress| seen.push(p);
    let r = measure(&store, "local", BUCKET, &cancel, &mut progress).await;
    (r, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn versions_and_delete_markers_are_counted_apart() {
    let (_s, data, accounts) = versioned().await;
    data.seed(BUCKET, "a.txt", &[1; 1000]);
    data.seed(BUCKET, "a.txt", &[2; 1500]); // the 1,000 bytes become an older version
    data.seed(BUCKET, "big.bin", &vec![3; 100_000]);
    data.seed(BUCKET, "gone.txt", &[4; 700]);
    let store = store_for(&accounts, "local", BUCKET).await.unwrap();
    store
        .sdk_client()
        .delete_object()
        .bucket(BUCKET)
        .key("gone.txt")
        .send()
        .await
        .unwrap();

    let (m, progress) = run(&accounts).await;
    let m = m.unwrap();
    assert!(m.versions_listed && m.complete, "{m:?}");
    // Current: a.txt (1,500) and big.bin (100,000). gone.txt's 700 bytes
    // and a.txt's first 1,000 are older versions, still stored.
    assert_eq!((m.objects, m.bytes), (2, 101_500));
    assert_eq!((m.noncurrent_versions, m.noncurrent_bytes), (2, 1_700));
    assert_eq!(m.delete_markers, 1);
    assert_eq!(m.stored_bytes(), 103_200);
    // Below 64 kB: the three small versions (big.bin is not).
    assert_eq!((m.small_objects, m.small_bytes), (3, 3_200));
    assert_eq!((m.account.as_str(), m.bucket.as_str()), ("local", BUCKET));
    assert!(m.measured_ms > 0);
    // One version page and one (empty) page of unfinished uploads.
    assert!(m.incomplete_listed);
    assert_eq!((m.incomplete_uploads, m.incomplete_bytes), (0, 0));
    assert!(m.notes.is_empty(), "{:?}", m.notes);
    assert_eq!(m.requests, 2);
    assert_eq!(
        progress.last().map(|p| (p.phase, p.entries)),
        Some(("versions", 5))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_of_a_large_bucket_is_read() {
    let (_s, data, accounts) = versioned().await;
    // 2,345 objects: three pages of at most 1,000.
    for i in 0..2_345u32 {
        data.seed(BUCKET, &format!("f/{i:05}"), &i.to_le_bytes());
    }
    // And older versions spread over the pages.
    for i in (0..2_345u32).step_by(500) {
        data.seed(BUCKET, &format!("f/{i:05}"), b"newer");
    }
    let (m, progress) = run(&accounts).await;
    let m = m.unwrap();
    assert_eq!(m.objects, 2_345);
    assert_eq!(m.noncurrent_versions, 5);
    assert_eq!(m.bytes, (2_345 - 5) * 4 + 5 * 5);
    assert!(m.complete);
    let version_pages = progress.iter().filter(|p| p.phase == "versions").count();
    assert_eq!(version_pages, 3, "{progress:?}");
    // Strictly growing counts: no page read twice.
    assert!(progress.windows(2).all(|w| w[0].entries < w[1].entries));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_bucket_measures_zero() {
    let (_s, _data, accounts) = versioned().await;
    let m = run(&accounts).await.0.unwrap();
    assert_eq!((m.objects, m.bytes, m.stored_bytes()), (0, 0, 0));
    assert!(m.versions_listed && m.complete);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_version_listing_only_current_objects_are_counted_and_unfinished_uploads_are_measured()
 {
    // s3s-fs answers ListObjectVersions with 501 Not Implemented.
    let server = TestServer::start(KEY_ID, SECRET, &[BUCKET]).await;
    let accounts = accounts_for(&server, SECRET);
    let store = store_for(&accounts, "local", BUCKET).await.unwrap();
    let client = store.sdk_client();
    for (key, len) in [("one.bin", 10_000usize), ("two.bin", 70_000)] {
        client
            .put_object()
            .bucket(BUCKET)
            .key(key)
            .body(aws_sdk_s3::primitives::ByteStream::from(vec![7u8; len]))
            .send()
            .await
            .unwrap();
    }
    // An upload that was started and never completed: its part is stored.
    let started = client
        .create_multipart_upload()
        .bucket(BUCKET)
        .key("unfinished.bin")
        .send()
        .await
        .unwrap();
    let upload_id = started.upload_id().unwrap().to_owned();
    client
        .upload_part()
        .bucket(BUCKET)
        .key("unfinished.bin")
        .upload_id(&upload_id)
        .part_number(1)
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![9u8; 123_456]))
        .send()
        .await
        .unwrap();

    let m = run(&accounts).await.0.unwrap();
    assert!(!m.versions_listed);
    assert!(
        m.notes
            .iter()
            .any(|n| n.contains("does not list object versions")),
        "{:?}",
        m.notes
    );
    assert_eq!((m.objects, m.bytes), (2, 80_000));
    assert_eq!((m.noncurrent_versions, m.delete_markers), (0, 0));
    assert!(m.incomplete_listed);
    assert_eq!(
        (
            m.incomplete_uploads,
            m.incomplete_bytes,
            m.incomplete_unmeasured
        ),
        (1, 123_456, 0)
    );
    assert_eq!(m.stored_bytes(), 80_000 + 123_456);
    // The failed versions request, one objects page, the uploads page and
    // the parts page.
    assert_eq!(m.requests, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_measurement_stops_before_the_next_request() {
    let (_s, data, accounts) = versioned().await;
    data.seed(BUCKET, "x", b"x");
    let store = store_for(&accounts, "local", BUCKET).await.unwrap();
    let cancel = AtomicBool::new(true);
    let r = measure(&store, "local", BUCKET, &cancel, &mut |_| {}).await;
    assert_eq!(r, Err(StorageError::Cancelled));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_key_or_a_missing_bucket_is_reported_in_sotywares_words() {
    let (server, _data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    let accounts = accounts_for(&server, "wrong-Secret-0000000000");
    let e = run(&accounts).await.0.unwrap_err();
    let shown = usage_error(&e);
    assert_eq!(shown.kind, ErrorKind::InvalidCredentials, "{e:?}");
    assert!(!shown.message.contains("SignatureDoesNotMatch"));

    let (server, _data) =
        TestServer::start_versioned(KEY_ID, SECRET, &[("other-bucket", true)]).await;
    let accounts = accounts_for(&server, SECRET);
    let e = run(&accounts).await.0.unwrap_err();
    assert_eq!(usage_error(&e).kind, ErrorKind::BucketUnavailable, "{e:?}");

    let slow = StorageError::Provider {
        status: 503,
        code: "SlowDown".into(),
        message: "x".into(),
    };
    assert_eq!(usage_error(&slow).kind, ErrorKind::Busy);
    assert!(usage_error(&slow).message.contains("slow down"));
}

/// An in-memory [`UsageStore`].
#[derive(Clone, Default)]
struct Memory(Arc<StdMutex<Vec<Measurement>>>);

impl UsageStore for Memory {
    fn add(&mut self, m: &Measurement) -> Result<(), StoreError> {
        self.0.lock().unwrap().push(m.clone());
        Ok(())
    }
    fn history(&mut self, account: &str, limit: usize) -> Result<Vec<Measurement>, StoreError> {
        let all = self.0.lock().unwrap();
        Ok(all
            .iter()
            .rev()
            .filter(|m| m.account == account)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct Events(StdMutex<Vec<UsageEvent>>);

impl UsageSink for Events {
    fn emit(&self, event: UsageEvent) {
        self.0.lock().unwrap().push(event);
    }
}

async fn finished(events: &Arc<Events>) -> UsageEvent {
    for _ in 0..200 {
        if let Some(e) = events.0.lock().unwrap().iter().find(|e| e.seq == u64::MAX) {
            return e.clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no final event");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_measurement_runs_in_its_slot_reports_and_is_kept() {
    let (_s, data, accounts) = versioned().await;
    data.seed(BUCKET, "k", &[0; 2048]);
    let memory = Memory::default();
    let usage = StorageUsage::load(Ok(Box::new(memory.clone())));
    let events = Arc::new(Events::default());
    let task = usage
        .start(
            Arc::clone(&accounts),
            "local",
            Arc::clone(&events) as Arc<dyn UsageSink>,
        )
        .await
        .unwrap();
    // One at a time.
    let busy = usage
        .start(Arc::clone(&accounts), "local", Arc::new(Events::default()))
        .await;
    if let Err(e) = busy {
        assert_eq!(e.kind, ErrorKind::Busy);
    }
    let end = finished(&events).await;
    assert_eq!(end.task, task);
    let UsageEventBody::Finished {
        measurement,
        error,
        cancelled,
        save_error,
    } = end.body
    else {
        panic!("not finished");
    };
    assert_eq!((error, cancelled, save_error), (None, false, None));
    let m = measurement.unwrap();
    assert_eq!((m.objects, m.bytes), (1, 2048));
    assert_eq!(memory.0.lock().unwrap().as_slice(), [*m.clone()]);
    // The slot is free again, and the overview shows the measurement.
    let list = accounts.list();
    let o = usage.overview(&list.accounts, &HashMap::new());
    assert_eq!(o.running, None);
    assert_eq!(o.measured, 1);
    assert_eq!(o.stored_bytes, 2048);
    assert_eq!(o.accounts[0].latest.as_ref(), Some(&*m));
    assert_eq!(o.accounts[0].history.len(), 1);
    // A generic S3 service has no price list: usage, no estimate.
    assert_eq!(o.groups.len(), 1);
    assert!(o.groups[0].estimate.is_none());
    assert!(
        o.groups[0]
            .no_estimate
            .as_deref()
            .unwrap()
            .contains("no price list")
    );
    assert_eq!(usage.cancel(&task), CancelOutcome::NotActive);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_account_is_refused_without_taking_the_slot() {
    let (_s, _data, accounts) = versioned().await;
    let usage = StorageUsage::load(Ok(Box::new(Memory::default())));
    let e = usage
        .start(Arc::clone(&accounts), "nope", Arc::new(Events::default()))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownAccount);
    assert_eq!(usage.running_info(), None);
}

// ------------------------------------------------------------ the overview

fn info(id: &str, provider: &str, name: &'static str, region: &str, bucket: &str) -> AccountInfo {
    AccountInfo {
        id: id.into(),
        label: id.into(),
        provider: provider.into(),
        provider_name: Some(name),
        region: region.into(),
        endpoint: None,
        endpoint_url: None,
        bucket: Some(bucket.into()),
        config_error: None,
        last_check: None,
    }
}

fn measured(account: &str, bucket: &str, bytes: u64, at: i64) -> Measurement {
    Measurement {
        account: account.into(),
        bucket: bucket.into(),
        measured_ms: at,
        objects: 10,
        bytes,
        versions_listed: true,
        incomplete_listed: true,
        complete: true,
        ..Measurement::default()
    }
}

const GB: u64 = 1_000_000_000;

#[test]
fn backblaze_buckets_share_an_account_per_region_and_hetzner_per_provider() {
    let accounts = vec![
        info(
            "photos",
            "backblaze-b2",
            "Backblaze B2",
            "eu-central-003",
            "p",
        ),
        info(
            "docs",
            "backblaze-b2",
            "Backblaze B2",
            "eu-central-003",
            "d",
        ),
        info("us", "backblaze-b2", "Backblaze B2", "us-west-004", "u"),
        info("h1", "hetzner", "Hetzner Object Storage", "fsn1", "h1"),
        info("h2", "hetzner", "Hetzner Object Storage", "nbg1", "h2"),
        info("minio", "s3-compatible", "S3-compatible", "us-east-1", "m"),
    ];
    let mut ms = HashMap::new();
    ms.insert(
        "photos".to_owned(),
        vec![measured("photos", "p", 150 * GB, 2)],
    );
    ms.insert("docs".to_owned(), vec![measured("docs", "d", 60 * GB, 2)]);
    ms.insert("us".to_owned(), vec![measured("us", "u", 5 * GB, 2)]);
    ms.insert("h1".to_owned(), vec![measured("h1", "h1", 100 * GB, 2)]);
    ms.insert("minio".to_owned(), vec![measured("minio", "m", 7 * GB, 2)]);
    let o = overview(&accounts, &ms, &HashMap::new());
    let keys: Vec<&str> = o.groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "backblaze-b2/eu-central-003",
            "backblaze-b2/us-west-004",
            "hetzner",
            "s3-compatible"
        ]
    );
    let eu = &o.groups[0];
    assert_eq!(eu.accounts, ["photos", "docs"]);
    assert_eq!((eu.measured, eu.stored_bytes), (2, 210 * GB));
    // 210 GB − 10 GB free × $6.95/TB = $1.39, the free allowance counted once.
    let e = eu.estimate.as_ref().unwrap();
    assert_eq!((e.total * 100.0).round() as i64, 139);
    assert!(eu.scope_note.contains("region eu-central-003"));
    // Under 10 GB in the other region (another account): free.
    assert_eq!(o.groups[1].estimate.as_ref().unwrap().total, 0.0);
    // Hetzner: one base price for both buckets, one not measured yet.
    let h = &o.groups[2];
    assert_eq!((h.accounts.len(), h.measured), (2, 1));
    let e = h.estimate.as_ref().unwrap();
    assert_eq!((e.total * 100.0).round() as i64, 649);
    assert!(
        e.assumptions
            .iter()
            .any(|a| a.contains("1 of these 2 bucket(s) not measured yet"))
    );
    // The generic service: usage only.
    assert!(o.groups[3].estimate.is_none());
    // Totals per currency, never summed across them.
    let costs: Vec<(&str, i64)> = o
        .costs
        .iter()
        .map(|c| (c.currency.as_str(), (c.total * 100.0).round() as i64))
        .collect();
    assert_eq!(costs, [("EUR", 649), ("USD", 139)]);
    assert_eq!(o.measured, 5);
    assert_eq!(o.stored_bytes, 322 * GB);
}

#[test]
fn the_assumed_downloads_change_the_estimate_of_their_group_only() {
    let accounts = vec![info(
        "h1",
        "hetzner",
        "Hetzner Object Storage",
        "fsn1",
        "h1",
    )];
    let mut ms = HashMap::new();
    ms.insert("h1".to_owned(), vec![measured("h1", "h1", 100 * GB, 2)]);
    let mut downloads = HashMap::new();
    downloads.insert("hetzner".to_owned(), 3_000.0); // 3 TB: 2 TB above the included one
    downloads.insert("elsewhere".to_owned(), 99.0);
    let o = overview(&accounts, &ms, &downloads);
    let g = &o.groups[0];
    assert_eq!(g.downloads_gb, 3_000.0);
    let egress = g
        .estimate
        .as_ref()
        .unwrap()
        .components
        .iter()
        .find(|c| c.id == "egress")
        .unwrap();
    assert_eq!((egress.amount * 100.0).round() as i64, 200);
    // Nonsense is ignored.
    downloads.insert("hetzner".to_owned(), f64::NAN);
    let o = overview(&accounts, &ms, &downloads);
    assert_eq!(o.groups[0].downloads_gb, 0.0);
}

#[test]
fn a_measurement_of_another_bucket_is_history_not_the_latest() {
    let accounts = vec![info(
        "a",
        "backblaze-b2",
        "Backblaze B2",
        "eu-central-003",
        "new-bucket",
    )];
    let mut ms = HashMap::new();
    ms.insert(
        "a".to_owned(),
        vec![
            measured("a", "old-bucket", 50 * GB, 3),
            measured("a", "new-bucket", 20 * GB, 1),
        ],
    );
    let o = overview(&accounts, &ms, &HashMap::new());
    assert_eq!(
        o.accounts[0].latest.as_ref().map(|m| m.bucket.as_str()),
        Some("new-bucket")
    );
    assert_eq!(o.stored_bytes, 20 * GB);
    // History oldest first, both buckets.
    let times: Vec<i64> = o.accounts[0]
        .history
        .iter()
        .map(|h| h.measured_ms)
        .collect();
    assert_eq!(times, [1, 3]);
}

#[test]
fn nothing_measured_yet_means_no_estimate_and_says_so() {
    let accounts = vec![info(
        "a",
        "backblaze-b2",
        "Backblaze B2",
        "eu-central-003",
        "b",
    )];
    let o = overview(&accounts, &HashMap::new(), &HashMap::new());
    assert_eq!(o.measured, 0);
    assert!(o.groups[0].estimate.is_none());
    assert_eq!(
        o.groups[0].no_estimate.as_deref(),
        Some("Measure a bucket to see an estimate.")
    );
    assert!(o.costs.is_empty());
}

#[test]
fn the_page_receives_this_shape() {
    let accounts = vec![info("a", "hetzner", "Hetzner Object Storage", "fsn1", "b")];
    let mut ms = HashMap::new();
    ms.insert("a".to_owned(), vec![measured("a", "b", GB, 7)]);
    let o = overview(&accounts, &ms, &HashMap::new());
    let v = serde_json::to_value(&o).unwrap();
    for field in [
        "accounts",
        "groups",
        "storedBytes",
        "objects",
        "measured",
        "costs",
        "running",
        "storageError",
    ] {
        assert!(v.get(field).is_some(), "{field}");
    }
    let m = &v["accounts"][0]["latest"];
    for field in [
        "measuredMs",
        "durationMs",
        "objects",
        "bytes",
        "versionsListed",
        "noncurrentVersions",
        "noncurrentBytes",
        "deleteMarkers",
        "incompleteListed",
        "incompleteUploads",
        "incompleteBytes",
        "incompleteUnmeasured",
        "smallObjects",
        "smallBytes",
        "complete",
        "requests",
        "notes",
    ] {
        assert!(m.get(field).is_some(), "{field}");
    }
    let g = &v["groups"][0];
    assert_eq!(g["estimate"]["currency"], "EUR");
    assert_eq!(g["estimate"]["period"], "30 days");
    assert!(
        g["estimate"]["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "base-fee")
    );
    assert_eq!(g["pricing"]["checkedOn"], "2026-11-15");
    let event = UsageEvent {
        task: "usage-1".into(),
        seq: 1,
        body: UsageEventBody::Progress {
            progress: Progress {
                phase: "versions",
                entries: 3,
                bytes: 9,
                requests: 1,
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({ "task": "usage-1", "seq": 1, "type": "progress",
            "progress": { "phase": "versions", "entries": 3, "bytes": 9, "requests": 1 } })
    );
}

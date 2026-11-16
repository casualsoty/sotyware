//! The local S3 server of the storage usage desktop test (ADR-041,
//! ADR-027): `tests/desktop/cloud-ui.sh usage`. In memory (the versioned
//! backend), on 127.0.0.1 only, with a made-up key; nothing reaches a
//! provider.
//!
//! One versioned bucket, [`BUCKET`], holds current files, older versions
//! and a deleted file ([`SEED`]), so a measurement has each part to show.
//! The helper writes what a measurement must find ([`expected`]) to the
//! file named by `SOTYWARE_UI_SERVER_FILE` plus `.json`, then its endpoint
//! to that file, and serves until the endpoint file is deleted. It fails
//! at the end unless the app only read (the bucket browser's and a
//! measurement's listings), a measurement reached it, and every version is
//! still there.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::{TestServer, Version, Versioned};

pub const BUCKET: &str = "usage-versioned";
pub const KEY_ID: &str = "e2eUsageKey00000000001";
pub const SECRET: &str = "e2e-Usage-Secret-0123456789";

/// Each key with the sizes of its versions, oldest first, and whether a
/// delete marker follows them (a deleted file whose versions stay).
pub const SEED: &[(&str, &[usize], bool)] = &[
    ("docs/report.pdf", &[400_000, 450_000, 500_000], false),
    ("notes.txt", &[12], false),
    ("old/draft.txt", &[300_000], true),
    ("photos/beach.jpg", &[1_250_000], false),
];

/// Fills [`BUCKET`] with [`SEED`], straight into memory (no request).
pub fn seed_into(data: &Versioned) {
    let mut st = data.lock();
    for (key, sizes, deleted) in SEED {
        for (i, size) in sizes.iter().enumerate() {
            let fill = u8::try_from(i).unwrap_or(u8::MAX);
            st.put(BUCKET, key, vec![fill; *size], None, HashMap::new());
        }
        if *deleted {
            let (id, modified) = st.stamp();
            st.buckets
                .get_mut(BUCKET)
                .expect("the seeded bucket exists")
                .keys
                .entry((*key).to_owned())
                .or_default()
                .push(Version {
                    id,
                    marker: true,
                    data: Arc::new(Vec::new()),
                    content_type: None,
                    metadata: HashMap::new(),
                    etag: String::new(),
                    modified,
                });
        }
    }
}

/// What a measurement of the seeded bucket finds, under the names of
/// `usage::Measurement`'s fields.
pub fn expected() -> serde_json::Value {
    let (mut objects, mut bytes) = (0, 0);
    let (mut noncurrent, mut noncurrent_bytes, mut markers) = (0, 0, 0);
    for (_, sizes, deleted) in SEED {
        let (last, older) = sizes.split_last().expect("every key has a version");
        if *deleted {
            noncurrent += sizes.len();
            noncurrent_bytes += sizes.iter().sum::<usize>();
            markers += 1;
        } else {
            objects += 1;
            bytes += last;
            noncurrent += older.len();
            noncurrent_bytes += older.iter().sum::<usize>();
        }
    }
    serde_json::json!({
        "objects": objects,
        "bytes": bytes,
        "noncurrentVersions": noncurrent,
        "noncurrentBytes": noncurrent_bytes,
        "deleteMarkers": markers,
        "incompleteUploads": 0,
        "incompleteBytes": 0,
    })
}

/// Every version in the bucket: key → (id, delete marker, size).
fn versions(data: &Versioned) -> BTreeMap<String, Vec<(String, bool, usize)>> {
    data.lock().buckets[BUCKET]
        .keys
        .iter()
        .map(|(k, list)| {
            let list = list
                .iter()
                .map(|v| (v.id.clone(), v.marker, v.data.len()))
                .collect();
            (k.clone(), list)
        })
        .collect()
}

/// Not a test: the server of the storage usage desktop test (module
/// documentation). Started by the driver with `cargo test ...
/// serve_usage_for_ui -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the storage usage desktop test, not a test"]
async fn serve_usage_for_ui() {
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    seed_into(&data);
    let before = versions(&data);
    // The figures first: the driver starts once the endpoint file exists.
    fs::write(format!("{file}.json"), expected().to_string()).unwrap();
    fs::write(&file, &server.endpoint).unwrap();
    println!(
        "serving {BUCKET} (versioned) at {}: {} keys, {} versions and delete markers",
        server.endpoint,
        before.len(),
        before.values().map(Vec::len).sum::<usize>()
    );
    // A test helper, so a plain wait loop is fine here.
    while Path::new(&file).exists() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for op in server.ops_of(KEY_ID) {
        *counts.entry(op).or_default() += 1;
    }
    println!("operations: {counts:?}");
    assert!(
        counts.keys().all(|op| matches!(
            op.as_str(),
            "HeadBucket"
                | "ListObjectsV2"
                | "ListObjectVersions"
                | "ListMultipartUploads"
                | "ListParts"
        )),
        "the app sent more than reads: {counts:?}"
    );
    assert!(
        counts.contains_key("ListObjectVersions") && counts.contains_key("ListMultipartUploads"),
        "no measurement reached the server: {counts:?}"
    );
    assert_eq!(versions(&data), before, "the bucket changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_measurement_of_the_seed_finds_what_the_desktop_test_expects() {
    use crate::cloud::s3::{ClientOptions, S3Store};
    use crate::cloud::usage::measure;
    use std::sync::atomic::AtomicBool;

    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    seed_into(&data);
    let store = S3Store::new(
        server.resolved("usage", &[BUCKET]),
        server.credentials(),
        ClientOptions {
            max_attempts: 1,
            ..ClientOptions::default()
        },
    );
    let m = measure(
        &store,
        "usage",
        BUCKET,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .await
    .unwrap();
    assert!(
        m.versions_listed && m.incomplete_listed && m.complete,
        "{m:?}"
    );
    assert!(m.notes.is_empty(), "{:?}", m.notes);
    let found = serde_json::json!({
        "objects": m.objects,
        "bytes": m.bytes,
        "noncurrentVersions": m.noncurrent_versions,
        "noncurrentBytes": m.noncurrent_bytes,
        "deleteMarkers": m.delete_markers,
        "incompleteUploads": m.incomplete_uploads,
        "incompleteBytes": m.incomplete_bytes,
    });
    assert_eq!(found, expected());
    // The figures the driver checks on the page, spelled out once.
    assert_eq!(
        expected(),
        serde_json::json!({
            "objects": 3,
            "bytes": 1_750_012,
            "noncurrentVersions": 3,
            "noncurrentBytes": 1_150_000,
            "deleteMarkers": 1,
            "incompleteUploads": 0,
            "incompleteBytes": 0,
        })
    );
}

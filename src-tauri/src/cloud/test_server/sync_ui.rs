//! The local S3 server of the folder sync desktop test (ADR-043, ADR-027):
//! `tests/desktop/cloud-ui.sh sync` and `windows-cloud-ui.ps1 -Mode sync`.
//! In memory (the versioned backend), on 127.0.0.1 only, with a made-up
//! key; nothing reaches a provider.
//!
//! One versioned bucket, [`BUCKET`], seeded with [`SEED`] (files the first
//! pass must bring down). The helper writes its endpoint to the file named
//! by `SOTYWARE_UI_SERVER_FILE` and, until that file is deleted, reads
//! commands from that name plus `.ctl` (one per line, the file removed when
//! done), playing another computer that uses the same bucket:
//!
//! - `dump`: writes `.state.json`: every key with its current size and
//!   SHA-256 (or `null` when its latest entry is a delete marker), and its
//!   number of versions and delete markers; the app's operations so far;
//! - `put <key> <text>`: stores `<text>` and a newline as a new version;
//! - `delete <key>`: adds a delete marker.
//!
//! At the end it fails unless the app's requests were folder sync's and the
//! connection check's own, and **no real version was ever removed by id**:
//! whatever the sync did, every file it replaced or deleted can still be
//! restored from the bucket's versions.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{TestServer, Version, Versioned, sha256_hex};

pub const BUCKET: &str = "sync-versioned";
pub const KEY_ID: &str = "e2eSyncKey000000000001";
pub const SECRET: &str = "e2e-Sync-Secret-0123456789";

/// What the bucket holds at the start: (key, content). The first pass must
/// bring both down, the nested one into its folder.
pub const SEED: &[(&str, &str)] = &[
    ("welcome.txt", "welcome from the bucket\n"),
    ("docs/plan.txt", "the plan, written elsewhere\n"),
];

fn state(data: &Versioned, server: &TestServer, seeded: usize) -> serde_json::Value {
    let st = data.lock();
    let mut keys = serde_json::Map::new();
    for (key, list) in &st.buckets[BUCKET].keys {
        let current = list.last().filter(|v| !v.marker).map(|v| {
            serde_json::json!({
                "size": v.data.len(),
                "sha256": sha256_hex(&v.data),
            })
        });
        keys.insert(
            key.clone(),
            serde_json::json!({
                "current": current,
                "versions": list.iter().filter(|v| !v.marker).count(),
                "markers": list.iter().filter(|v| v.marker).count(),
            }),
        );
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for op in &server.ops()[seeded..] {
        *counts
            .entry(op.split('#').next().unwrap_or(op).to_owned())
            .or_default() += 1;
    }
    serde_json::json!({ "keys": keys, "operations": counts })
}

fn delete_marker(data: &Versioned, key: &str) {
    let mut st = data.lock();
    let (id, modified) = st.stamp();
    st.buckets
        .get_mut(BUCKET)
        .expect("the bucket exists")
        .keys
        .entry(key.to_owned())
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

/// Not a test: the server of the folder sync desktop test (module
/// documentation). Started by the drivers with `cargo test ...
/// serve_sync_for_ui -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the folder sync desktop test, not a test"]
async fn serve_sync_for_ui() {
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    for (key, text) in SEED {
        data.seed(BUCKET, key, text.as_bytes());
    }
    // Seeding went straight into memory: no request counted.
    let seeded = server.ops().len();
    fs::write(&file, &server.endpoint).unwrap();
    println!(
        "serving {BUCKET} (versioned) at {}, {} objects",
        server.endpoint,
        SEED.len()
    );
    let ctl = PathBuf::from(format!("{file}.ctl"));
    // A test helper, so a plain wait loop is fine here.
    while Path::new(&file).exists() {
        if let Ok(commands) = fs::read_to_string(&ctl) {
            for line in commands.lines() {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                match parts.as_slice() {
                    ["dump"] => {
                        let st = state(&data, &server, seeded);
                        let tmp = format!("{file}.state.json.tmp");
                        fs::write(&tmp, serde_json::to_string_pretty(&st).unwrap()).unwrap();
                        fs::rename(&tmp, format!("{file}.state.json")).unwrap();
                    }
                    ["put", key, text] => {
                        data.lock().put(
                            BUCKET,
                            key,
                            format!("{text}\n").into_bytes(),
                            Some("text/plain".into()),
                            HashMap::new(),
                        );
                    }
                    ["delete", key] => delete_marker(&data, key),
                    other => println!("unknown command {other:?}"),
                }
            }
            let _ = fs::remove_file(&ctl);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let final_state = state(&data, &server, seeded);
    let counts = final_state["operations"].clone();
    println!("operations: {counts}");
    let st = data.lock();
    let real_removed: Vec<_> = st
        .removed_versions
        .iter()
        .filter(|(b, _, _, marker)| b == BUCKET && !marker)
        .collect();
    println!(
        "versions removed by id: {} delete marker(s), {} real version(s) {real_removed:?}",
        st.removed_versions.iter().filter(|(_, _, _, m)| *m).count(),
        real_removed.len()
    );
    assert!(
        real_removed.is_empty(),
        "a real version was deleted for good: {real_removed:?}"
    );
    let ops = counts.as_object().expect("an object of counts");
    assert!(
        ops.keys().all(|op| matches!(
            op.as_str(),
            "HeadBucket"
                | "ListObjectsV2"
                | "ListObjectVersions"
                | "GetBucketVersioning"
                | "HeadObject"
                | "GetObject"
                | "PutObject"
                | "CopyObject"
                | "DeleteObject"
                | "CreateMultipartUpload"
                | "UploadPart"
                | "CompleteMultipartUpload"
                | "AbortMultipartUpload"
                | "ListMultipartUploads"
        )),
        "unexpected requests: {ops:?}"
    );
    assert!(
        ops.contains_key("PutObject") && ops.contains_key("GetObject"),
        "no sync transfer reached the server: {ops:?}"
    );
}

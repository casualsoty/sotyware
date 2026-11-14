//! The local S3 server of the file-management desktop test (ADR-026,
//! ADR-027): `tests/desktop/cloud-ui.sh files` and
//! `windows-cloud-ui.ps1 -Mode files`. In memory (the versioned backend),
//! on 127.0.0.1 only, with a made-up key; nothing reaches a provider.
//!
//! Two buckets: [`VERSIONED`] keeps versions (deletes add delete markers,
//! restore is possible), [`PLAIN`] does not (deletes are final). Both are
//! seeded with [`seed`]. The helper writes its endpoint to the file named
//! by `SOTYWARE_UI_SERVER_FILE` and, until that file is deleted, reads
//! commands from that name plus `.ctl` (one per line, the file removed when
//! done):
//!
//! - `dump`: writes `.state.json`: per bucket every key with its current
//!   size, ETag, content type and SHA-256 metadata, and its versions (id,
//!   delete marker or not); the app's operations so far; the versions
//!   removed by id;
//! - `fail <Operation> <n>`: the next `n` requests of that operation are
//!   refused with `AccessDenied` (a partial failure the app must report);
//! - `hang <Operation> <n>`: the next `n` requests of that operation are
//!   never answered (a transfer the driver cancels while it runs);
//! - `add <bucket> <key>`: stores a new object (another client's change
//!   between the review and the start).
//!
//! At the end it fails unless the app's requests were the file manager's
//! own and **no real version in the versioned bucket was ever removed by
//! id** (only delete markers, by a restore): whatever the UI did, nothing
//! the user could restore was deleted for good.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{TestServer, Versioned};

pub const VERSIONED: &str = "files-versioned";
pub const PLAIN: &str = "files-plain";
pub const KEY_ID: &str = "e2eFilesKey00000000001";
pub const SECRET: &str = "e2e-Files-Secret-0123456789";
/// Objects under `bulk/`: more than one folder page (1,000) and more than
/// one search call (10,000 objects looked at).
pub const BULK: usize = 10_050;

/// A 1×1 PNG.
pub const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// What both buckets hold at the start: (key, content, content type).
/// Text files say their own key, so a copy can be checked by content.
pub fn seed() -> Vec<(String, Vec<u8>, Option<&'static str>)> {
    let text = |k: &str| {
        (
            k.to_owned(),
            format!("content of {k}\n").into_bytes(),
            Some("text/plain"),
        )
    };
    let mut out = vec![
        // Folder management and the prefix boundary.
        text("photos/a.txt"),
        text("photos/b.txt"),
        text("photos/sub/c.txt"),
        text("photos-backup/keep.txt"),
        ("photos-backup/".to_owned(), Vec::new(), None),
        text("photosx"),
        // File management.
        text("files/one.txt"),
        text("files/two.txt"),
        text("files/three.txt"),
        text("conflict/src/x.txt"),
        (
            "conflict/dest/x.txt".to_owned(),
            b"already there\n".to_vec(),
            Some("text/plain"),
        ),
        // Previews.
        (
            "docs/readme.txt".to_owned(),
            b"Hello from the readme.\n".to_vec(),
            Some("text/plain"),
        ),
        (
            "docs/data.json".to_owned(),
            br#"{"name":"sotyware","list":[1,2,3]}"#.to_vec(),
            Some("application/json"),
        ),
        (
            "docs/cars.txt".to_owned(),
            b"BMW, Audi and others: a list of cars\n".to_vec(),
            Some("text/plain"),
        ),
        (
            "docs/page.html".to_owned(),
            b"<html><body><script>document.title='PWNED'</script><b>bold</b></body></html>"
                .to_vec(),
            Some("text/html"),
        ),
        (
            "docs/logo.svg".to_owned(),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"document.title='PWNED'\"></svg>"
                .to_vec(),
            Some("image/svg+xml"),
        ),
        (
            "docs/big.txt".to_owned(),
            "line of text\n".repeat(25_000).into_bytes(),
            Some("text/plain"),
        ),
        (
            "media/pixel.png".to_owned(),
            PIXEL_PNG.to_vec(),
            Some("image/png"),
        ),
        (
            "media/huge.png".to_owned(),
            [PIXEL_PNG[..8].to_vec(), vec![0u8; 11 * 1024 * 1024]].concat(),
            Some("image/png"),
        ),
        // Sorting: name order alpha < beta < gamma, size order beta (10) <
        // gamma (20) < alpha (30), date order gamma < alpha < beta.
        (
            "sort/gamma.jpg".to_owned(),
            vec![7u8; 20],
            Some("image/jpeg"),
        ),
        (
            "sort/alpha.txt".to_owned(),
            vec![b'a'; 30],
            Some("text/plain"),
        ),
        (
            "sort/beta.pdf".to_owned(),
            vec![b'%'; 10],
            Some("application/pdf"),
        ),
        // Search.
        text("search/deep/needle-1.txt"),
        text("search/deep/x/needle-2.txt"),
        text("search/other/needle-3.log"),
        text("search/other/hay.txt"),
        // Unusual names.
        text("unusual/größe +test.txt"),
        text("unusual/a?b#c%.txt"),
        text("unusual/con.txt"),
        // A folder whose delete is partly refused.
        text("del/p1.txt"),
        text("del/p2.txt"),
        // Downloads.
        text("dl/one.txt"),
        text("dl/tree/a.txt"),
        text("dl/tree/deep/b.txt"),
    ];
    for n in 0..BULK {
        out.push((format!("bulk/{n:05}.dat"), vec![1], None));
    }
    out.push(text("bulk/zz-needle.txt"));
    out
}

fn seed_into(data: &Versioned, bucket: &str) {
    let mut st = data.lock();
    for (key, content, content_type) in seed() {
        let sha = super::sha256_hex(&content);
        let meta = if key.ends_with('/') || key.starts_with("bulk/") {
            HashMap::new()
        } else {
            HashMap::from([(super::super::s3::SHA256_METADATA.to_owned(), sha)])
        };
        st.put(bucket, &key, content, content_type.map(str::to_owned), meta);
    }
}

fn dump(data: &Versioned, server: &TestServer, seeded: usize) -> serde_json::Value {
    let st = data.lock();
    let mut buckets = serde_json::Map::new();
    for name in [VERSIONED, PLAIN] {
        let b = &st.buckets[name];
        let mut keys = serde_json::Map::new();
        for (key, versions) in &b.keys {
            if key.starts_with("bulk/") && key != "bulk/zz-needle.txt" {
                continue;
            }
            let current = b.current(key).map(|v| {
                serde_json::json!({
                    "size": v.data.len(),
                    "etag": v.etag,
                    "contentType": v.content_type,
                    "sha256": super::sha256_hex(&v.data),
                    "sha256Metadata": v.metadata.get(super::super::s3::SHA256_METADATA),
                    "text": String::from_utf8(v.data.to_vec()).ok().filter(|t| t.len() < 200),
                })
            });
            let list: Vec<_> = versions
                .iter()
                .map(|v| serde_json::json!([v.id, v.marker]))
                .collect();
            keys.insert(
                key.clone(),
                serde_json::json!({"current": current, "versions": list}),
            );
        }
        let bulk = b
            .keys
            .keys()
            .filter(|k| k.starts_with("bulk/") && b.current(k).is_some())
            .count();
        buckets.insert(
            name.to_owned(),
            serde_json::json!({"keys": keys, "bulkCurrent": bulk}),
        );
    }
    serde_json::json!({
        "buckets": buckets,
        "ops": server.ops()[seeded..].to_vec(),
        "removedVersions": st.removed_versions.iter()
            .map(|(b, k, id, m)| serde_json::json!({"bucket": b, "key": k, "id": id, "marker": m}))
            .collect::<Vec<_>>(),
    })
}

/// Not a test: the server of the file-management desktop test (module
/// documentation). Started by the drivers with `cargo test ...
/// serve_files_for_ui -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the file-management desktop test, not a test"]
async fn serve_files_for_ui() {
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let (server, data) =
        TestServer::start_versioned(KEY_ID, SECRET, &[(VERSIONED, true), (PLAIN, false)]).await;
    seed_into(&data, VERSIONED);
    seed_into(&data, PLAIN);
    // Seeding went straight into memory: no request counted.
    let seeded = server.ops().len();
    fs::write(&file, &server.endpoint).unwrap();
    println!(
        "serving {VERSIONED} (versioned) and {PLAIN} (unversioned) at {}, {} objects each",
        server.endpoint,
        seed().len()
    );
    let ctl = PathBuf::from(format!("{file}.ctl"));
    // A test helper, so a plain wait loop is fine here.
    while Path::new(&file).exists() {
        if let Ok(commands) = fs::read_to_string(&ctl) {
            for line in commands.lines() {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                match parts.as_slice() {
                    ["dump"] => {
                        let state = dump(&data, &server, seeded);
                        let tmp = format!("{file}.state.json.tmp");
                        fs::write(&tmp, serde_json::to_string_pretty(&state).unwrap()).unwrap();
                        fs::rename(&tmp, format!("{file}.state.json")).unwrap();
                    }
                    [verb @ ("fail" | "hang"), op, n] => {
                        let op: &'static str = match *op {
                            "CopyObject" => "CopyObject",
                            "DeleteObject" => "DeleteObject",
                            "GetObject" => "GetObject",
                            "PutObject" => "PutObject",
                            other => panic!("{verb}: unknown operation {other}"),
                        };
                        let action = if *verb == "fail" {
                            super::Inject::Deny
                        } else {
                            super::Inject::Hang
                        };
                        server.inject(op, None, n.parse().unwrap(), action);
                    }
                    ["add", bucket, key] => {
                        data.lock().put(
                            bucket,
                            key,
                            format!("added later: {key}\n").into_bytes(),
                            Some("text/plain".into()),
                            HashMap::new(),
                        );
                    }
                    other => println!("unknown command {other:?}"),
                }
            }
            let _ = fs::remove_file(&ctl);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ops = server.ops()[seeded..].to_vec();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for op in &ops {
        *counts
            .entry(op.split('#').next().unwrap_or(op).to_owned())
            .or_default() += 1;
    }
    println!("operations: {counts:?}");
    let st = data.lock();
    let real_removed: Vec<_> = st
        .removed_versions
        .iter()
        .filter(|(b, _, _, marker)| b == VERSIONED && !marker)
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
    assert!(
        counts.keys().all(|op| matches!(
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
        )),
        "unexpected requests: {counts:?}"
    );
}

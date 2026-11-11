//! Opt-in integration tests against real provider accounts.
//!
//! Ignored by default, so `cargo test` and CI never contact a provider. Run one
//! provider with its account set up as described in
//! docs/development/testing.md#cloud-storage-integration-tests, for example:
//!
//! ```text
//! SOTYWARE_IT_B2_BUCKET=… SOTYWARE_IT_B2_REGION=eu-central-003 \
//! SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID=… SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY=… \
//! cargo test cloud::integration::backblaze_b2 -- --ignored --nocapture
//! ```
//!
//! Safety rules, enforced here:
//! - Every key is under `sotyware-it/<unique run id>/`, and the test first
//!   checks that nothing exists under that prefix.
//! - Cleanup deletes only the exact keys (and versions) this run created; it
//!   never deletes by prefix and never touches anything listed but not
//!   uploaded by the run.
//! - The test files are generated; no personal file is read.

use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::account::AccountConfig;
use super::credentials::{CredentialSource, Credentials, EnvCredentials, Secret};
use super::s3::{ClientOptions, S3Store, StorageError, Verification};
use super::test_server::TempDir;

const MIB: usize = 1024 * 1024;

struct Target {
    account_id: &'static str,
    provider: &'static str,
    bucket: String,
    region: String,
}

/// Reads `SOTYWARE_IT_<NAME>_BUCKET` and `_REGION`; panics with what to set.
fn target(name: &str, account_id: &'static str, provider: &'static str) -> Target {
    let var = |suffix: &str| {
        let key = format!("SOTYWARE_IT_{name}_{suffix}");
        std::env::var(&key).unwrap_or_else(|_| panic!("set {key} to run this opt-in test"))
    };
    Target {
        account_id,
        provider,
        bucket: var("BUCKET"),
        region: var("REGION"),
    }
}

/// Deletes exactly what the run created, newest first; reports, never panics,
/// so every object gets its attempt.
async fn cleanup(store: &S3Store, bucket: &str, created: &[(String, Option<String>)]) -> bool {
    let mut ok = true;
    for (key, version) in created.iter().rev() {
        if let Err(e) = store.delete_object(bucket, key, version.as_deref()).await {
            eprintln!("CLEANUP FAILED for {key} (version {version:?}): {e}");
            ok = false;
        }
    }
    ok
}

async fn round_trip(t: Target) {
    let mut config = AccountConfig::new(t.account_id, t.account_id, t.provider, &t.region);
    config.buckets = vec![t.bucket.clone()];
    let resolved = config.resolve().expect("account configuration");
    let creds = EnvCredentials::from_process()
        .credentials(t.account_id)
        .expect("credentials");
    let options = ClientOptions {
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        ..ClientOptions::default()
    };
    let store = S3Store::new(resolved.clone(), creds.clone(), options.clone());
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!("sotyware-it/{run}-{}/", std::process::id());
    println!(
        "{}: endpoint {}, prefix {prefix}",
        t.provider, resolved.endpoint_url
    );

    match store.list_buckets().await {
        Ok(b) => println!("list buckets: {} visible", b.len()),
        // A key restricted to one bucket may not list buckets.
        Err(StorageError::AccessDenied(e)) => {
            println!("list buckets: not allowed for this key ({e})")
        }
        Err(e) => panic!("list buckets: {e}"),
    }
    assert!(
        store
            .list_objects(&t.bucket, &prefix)
            .await
            .unwrap()
            .is_empty(),
        "the run prefix must be new"
    );

    let dir = TempDir::new("it");
    let small = dir.file("small.bin", 200_000, 1);
    let large = dir.file("large.bin", 12 * MIB + 7, 2);
    let mut created: Vec<(String, Option<String>)> = Vec::new();

    let outcome = async {
        for (name, path, parts) in [("small.bin", &small, 1), ("large.bin", &large, 3)] {
            let key = format!("{prefix}{name}");
            let t0 = Instant::now();
            let up = store.upload_file(&t.bucket, &key, path).await?;
            created.push((key.clone(), up.version_id.clone()));
            println!(
                "upload {name}: {} bytes, {} part(s), {:.2} s, version {:?}",
                up.size,
                up.parts,
                t0.elapsed().as_secs_f64(),
                up.version_id
            );
            assert_eq!(up.parts, parts);
            let head = store
                .head_object(&t.bucket, &key)
                .await?
                .expect("uploaded object");
            assert_eq!(head.sha256.as_deref(), Some(up.sha256.as_str()));
            let dest = dir.path(&format!("{name}.back"));
            let t0 = Instant::now();
            let down = store.download_file(&t.bucket, &key, &dest).await?;
            println!(
                "download {name}: {:.2} s, {:?}",
                t0.elapsed().as_secs_f64(),
                down.verification
            );
            assert_eq!(down.verification, Verification::Sha256Matched);
            assert_eq!(fs::read(&dest).unwrap(), fs::read(path).unwrap());
        }
        let listed = store.list_objects(&t.bucket, &prefix).await?;
        assert_eq!(listed.len(), 2, "{listed:?}");

        let wrong = S3Store::new(
            resolved.clone(),
            Credentials {
                access_key_id: creds.access_key_id.clone(),
                secret_access_key: Secret::new("not-the-secret"),
            },
            options.clone(),
        );
        let e = wrong.list_objects(&t.bucket, &prefix).await.unwrap_err();
        println!("wrong secret: {e}");
        assert!(matches!(e, StorageError::InvalidCredentials(_)), "{e:?}");
        Ok::<(), StorageError>(())
    }
    .await;

    let cleaned = cleanup(&store, &t.bucket, &created).await;
    let left = store.list_objects(&t.bucket, &prefix).await;
    println!("after cleanup: {left:?}");
    outcome.expect("round trip");
    assert!(cleaned, "cleanup failed; see CLEANUP FAILED lines");
    assert!(left.unwrap().is_empty(), "objects left under {prefix}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Backblaze B2 test bucket and key"]
async fn backblaze_b2_round_trip() {
    round_trip(target("B2", "it-b2", "backblaze-b2")).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Hetzner Object Storage test bucket and key"]
async fn hetzner_round_trip() {
    round_trip(target("HETZNER", "it-hetzner", "hetzner")).await;
}

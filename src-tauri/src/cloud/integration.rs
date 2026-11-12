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
//! - Before and after, the run prefix is checked for object *versions* and
//!   delete markers too, not only for current objects: on an always-versioned
//!   provider (B2) a prefix can look empty while hidden versions stay billed.
//!
//! `backblaze_b2_connection_check` and `backblaze_b2_folder_listing` write
//! nothing: they record every HTTP request the SDK sends (method, path,
//! status; never a header) and fingerprint every object version under
//! `sotyware-it/` before and after.

use std::fs;
use std::sync::{Arc, Mutex};

use aws_sdk_s3::config::interceptors::{
    BeforeDeserializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
};
use aws_sdk_s3::config::{ConfigBag, Intercept, RuntimeComponents};
use aws_sdk_s3::error::{BoxError, DisplayErrorContext};
use sha2::{Digest, Sha256};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::account::{AccountConfig, MemoryAccountStore, ResolvedAccount};
use super::browser::{self, listing_error};
use super::credentials::{
    CredentialStore, Credentials, EnvCredentials, MemoryCredentialStore, Secret,
};
use super::dashboard::{AccountInput, CheckStatus, CloudAccounts, CredentialInput, check_status};
use super::s3::{ClientOptions, FolderPage, S3Store, StorageError, Verification};
use super::test_server::{TempDir, TestServer};

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

/// One entry of a version listing: key, version id, whether it is a delete
/// marker (B2 calls it a hide marker).
type VersionEntry = (String, String, bool);

/// Every version and delete marker under `prefix`, following the markers.
async fn versions(store: &S3Store, bucket: &str, prefix: &str) -> Vec<VersionEntry> {
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
            .unwrap_or_else(|e| panic!("list object versions: {}", DisplayErrorContext(&e)));
        for v in out.versions() {
            all.push((
                v.key().unwrap_or_default().to_owned(),
                v.version_id().unwrap_or_default().to_owned(),
                false,
            ));
        }
        for m in out.delete_markers() {
            all.push((
                m.key().unwrap_or_default().to_owned(),
                m.version_id().unwrap_or_default().to_owned(),
                true,
            ));
        }
        if out.is_truncated() != Some(true) {
            break;
        }
        key_marker = out.next_key_marker().map(str::to_owned);
        version_marker = out.next_version_id_marker().map(str::to_owned);
    }
    all
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
    // The dashboard's connection check (HeadBucket, then at most a one-key
    // listing): must pass for a key restricted to this bucket, without
    // ListBuckets.
    let started = Instant::now();
    match store.check_bucket(&t.bucket).await {
        Ok(()) => println!("connection check: connected in {:?}", started.elapsed()),
        Err(e) => panic!("connection check: {e}"),
    }
    assert!(
        store
            .list_objects(&t.bucket, &prefix)
            .await
            .unwrap()
            .is_empty(),
        "the run prefix must be new"
    );
    assert!(
        versions(&store, &t.bucket, &prefix).await.is_empty(),
        "the run prefix must have no versions"
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
        for (key, version) in &created {
            assert!(
                version.is_some(),
                "{key}: no version id, so cleanup could only hide it"
            );
        }

        // A plain delete (no version id) on an always-versioned bucket only
        // hides the object: it disappears from listings and reads, a delete
        // marker appears, and the uploaded version is still stored.
        let small_key = format!("{prefix}small.bin");
        let small_version = created[0].1.clone().expect("checked above");
        store.delete_object(&t.bucket, &small_key, None).await?;
        let after = versions(&store, &t.bucket, &prefix).await;
        let markers: Vec<_> = after
            .iter()
            .filter(|(k, _, marker)| *marker && *k == small_key)
            .collect();
        // Cleanup must remove the marker too; record it before asserting.
        for (k, id, _) in &markers {
            created.push((k.clone(), Some(id.clone())));
        }
        println!(
            "plain delete: {} delete marker(s), versions now {after:?}",
            markers.len()
        );
        assert_eq!(markers.len(), 1, "{after:?}");
        assert!(
            after
                .iter()
                .any(|(k, id, marker)| !*marker && *k == small_key && *id == small_version),
            "the uploaded version must still be stored after a plain delete: {after:?}"
        );
        assert!(store.head_object(&t.bucket, &small_key).await?.is_none());
        let listed = store.list_objects(&t.bucket, &prefix).await?;
        assert_eq!(
            listed.len(),
            1,
            "a hidden object must not be listed: {listed:?}"
        );
        let dest = dir.path("hidden.back");
        let e = store
            .download_file(&t.bucket, &small_key, &dest)
            .await
            .unwrap_err();
        println!("download hidden: {e}");
        assert!(matches!(e, StorageError::NotFound(_)), "{e:?}");
        assert!(!dest.exists(), "a failed download must leave no file");
        let missing = format!("{prefix}never-uploaded.bin");
        assert!(store.head_object(&t.bucket, &missing).await?.is_none());

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
        let e = wrong.check_bucket(&t.bucket).await.unwrap_err();
        println!("connection check, wrong secret: {e}");
        assert!(matches!(e, StorageError::InvalidCredentials(_)), "{e:?}");
        Ok::<(), StorageError>(())
    }
    .await;

    let cleaned = cleanup(&store, &t.bucket, &created).await;
    let left = store.list_objects(&t.bucket, &prefix).await;
    let left_versions = versions(&store, &t.bucket, &prefix).await;
    println!("after cleanup: objects {left:?}, versions {left_versions:?}");
    outcome.expect("round trip");
    assert!(cleaned, "cleanup failed; see CLEANUP FAILED lines");
    assert!(left.unwrap().is_empty(), "objects left under {prefix}");
    assert!(
        left_versions.is_empty(),
        "versions or delete markers left under {prefix}: {left_versions:?}"
    );
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

/// Every HTTP request a client sent, as `METHOD /path?query -> status`.
/// Headers (the signature) are never read.
#[derive(Clone, Debug, Default)]
struct Requests(Arc<Mutex<Vec<String>>>);

impl Requests {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl Intercept for Requests {
    fn name(&self) -> &'static str {
        "sotyware-it-requests"
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
        let path = after_scheme
            .find('/')
            .map_or("/", |i| &after_scheme[i..])
            .to_owned();
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {path}", request.method()));
        Ok(())
    }

    fn read_after_transmit(
        &self,
        context: &BeforeDeserializationInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        if let Some(last) = self.0.lock().unwrap().last_mut() {
            last.push_str(&format!(" -> {}", context.response().status().as_u16()));
        }
        Ok(())
    }
}

/// One connection check, as the dashboard runs it, with its requests.
struct Checked {
    status: CheckStatus,
    error: Option<String>,
    requests: Vec<String>,
}

async fn check(
    account: &ResolvedAccount,
    creds: Credentials,
    bucket: &str,
    options: &ClientOptions,
) -> Checked {
    let requests = Requests::default();
    let mut account = account.clone();
    account.buckets = vec![bucket.to_owned()];
    let store = S3Store::new(account, creds, options.clone()).with_interceptor(requests.clone());
    let result = store.check_bucket(bucket).await;
    Checked {
        status: result
            .as_ref()
            .map_or_else(check_status, |()| CheckStatus::Connected),
        error: result.err().map(|e| e.to_string()),
        requests: requests.take(),
    }
}

/// The dashboard's connection check in every case that can be produced with
/// one valid key and without writing anything. Returns each case's name and
/// result; asserts what must hold at any provider.
async fn connection_check_cases(
    account: &ResolvedAccount,
    creds: &Credentials,
    bucket: &str,
    run: &str,
) -> Vec<(&'static str, Checked)> {
    assert!(
        account.path_style,
        "the ListBuckets detection below expects path-style requests"
    );
    let options = ClientOptions {
        max_attempts: 1,
        ..ClientOptions::default()
    };
    let wrong_secret = Credentials {
        access_key_id: creds.access_key_id.clone(),
        secret_access_key: Secret::new("not-the-secret-0123456789abcdefghijk"),
    };
    // Well-formed for B2 (25 hex digits) but no account's key.
    let unknown_key = Credentials {
        access_key_id: "0030000000000000000000000".into(),
        secret_access_key: creds.secret_access_key.clone(),
    };
    // Bucket names are global at B2; this one is new for every run, so it
    // exists nowhere or belongs to someone else, and the key cannot use it.
    let missing = format!("sotyware-it-missing-{run}");
    let mut unreachable = account.clone();
    unreachable.endpoint_url = "http://127.0.0.1:9".into();

    let cases = vec![
        (
            "valid key, its bucket",
            check(account, creds.clone(), bucket, &options).await,
        ),
        (
            "wrong secret",
            check(account, wrong_secret, bucket, &options).await,
        ),
        (
            "unknown key id",
            check(account, unknown_key, bucket, &options).await,
        ),
        (
            "valid key, a bucket it may not use",
            check(account, creds.clone(), &missing, &options).await,
        ),
        (
            "unreachable endpoint",
            check(&unreachable, creds.clone(), bucket, &options).await,
        ),
    ];
    for (name, c) in &cases {
        let shown: Vec<String> = c
            .requests
            .iter()
            .map(|r| r.replace(bucket, "<bucket>"))
            .collect();
        let error = c
            .error
            .as_deref()
            .map(|e| e.replace(bucket, "<bucket>"))
            .unwrap_or_default();
        println!("{name}: {:?} {shown:?} {error}", c.status);
    }

    let status = |i: usize| cases[i].1.status;
    assert_eq!(status(0), CheckStatus::Connected, "{:?}", cases[0].1.error);
    assert_eq!(status(1), CheckStatus::InvalidCredentials);
    assert_eq!(status(2), CheckStatus::InvalidCredentials);
    assert!(
        matches!(
            status(3),
            CheckStatus::AccessDenied | CheckStatus::BucketUnavailable
        ),
        "a valid key must not be reported as invalid or connected for another bucket: {:?}",
        status(3)
    );
    assert_eq!(status(4), CheckStatus::Network);

    let all: Vec<&String> = cases.iter().flat_map(|(_, c)| &c.requests).collect();
    for r in &all {
        let (method, path) = r.split_once(' ').unwrap();
        let path = path.split(" -> ").next().unwrap();
        assert!(
            method == "HEAD" || method == "GET",
            "a check may only read: {r}"
        );
        assert!(
            path != "/" && !path.starts_with("/?"),
            "ListBuckets must never be sent: {r}"
        );
        if method == "GET" {
            assert!(
                path.contains("list-type=2") && path.contains("max-keys=1"),
                "the only GET is a one-key ListObjectsV2: {r}"
            );
        }
    }
    for (name, c) in &cases[..4] {
        assert!(
            c.requests.first().is_some_and(|r| r.starts_with("HEAD ")),
            "{name}: the check starts with HeadBucket: {:?}",
            c.requests
        );
        assert!(c.requests.len() <= 2, "{name}: {:?}", c.requests);
    }
    cases
}

/// SHA-256 over every object version and delete marker under `prefix`
/// (key, version id, marker), and how many there are.
async fn fingerprint(store: &S3Store, bucket: &str, prefix: &str) -> (String, usize) {
    let mut all = versions(store, bucket, prefix).await;
    all.sort();
    let mut hash = Sha256::new();
    for (key, version, marker) in &all {
        hash.update(format!("{key}\0{version}\0{marker}\n"));
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (hex, all.len())
}

/// What the B2 test key may see: it is restricted to this file name prefix
/// (docs/development/testing.md#cloud-storage-integration-tests).
const B2_KEY_PREFIX: &str = "sotyware-it/";

fn b2_store() -> (Target, ResolvedAccount, Credentials, S3Store) {
    let t = target("B2", "it-b2", "backblaze-b2");
    let mut config = AccountConfig::new(t.account_id, t.account_id, t.provider, &t.region);
    config.buckets = vec![t.bucket.clone()];
    let resolved = config.resolve().expect("account configuration");
    let creds = EnvCredentials::from_process()
        .credentials(t.account_id)
        .expect("credentials");
    let store = S3Store::new(resolved.clone(), creds.clone(), ClientOptions::default());
    (t, resolved, creds, store)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Backblaze B2 test bucket and key"]
async fn backblaze_b2_connection_check() {
    let (t, resolved, creds, store) = b2_store();
    println!("backblaze-b2: endpoint {}", resolved.endpoint_url);
    let before = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    connection_check_cases(&resolved, &creds, &t.bucket, &run).await;
    let after = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    println!(
        "versions under {B2_KEY_PREFIX}: {} before, {} after; fingerprint {} before, {} after",
        before.1, after.1, before.0, after.0
    );
    assert_eq!(before, after, "the checks must not change the bucket");
}

/// Prints the fingerprint of everything the B2 test key can see, so a run
/// outside Rust (the Cloud Storage UI test) can show it left the bucket alone.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Backblaze B2 test bucket and key"]
async fn backblaze_b2_fingerprint() {
    let (t, _, _, store) = b2_store();
    let (hash, n) = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    println!("sotyware-it fingerprint {hash} versions {n}");
}

/// The same cases against the in-process server, with `ListBuckets` refused
/// as B2 refuses it a bucket-restricted key: shows that the request
/// recording and the assertions work before they are pointed at a provider.
#[tokio::test(flavor = "multi_thread")]
async fn connection_check_cases_against_the_test_server() {
    let server =
        TestServer::start_denying("test-key", "test-secret", &["it-bucket"], &["ListBuckets"])
            .await;
    let mut config = AccountConfig::new("it-local", "it-local", "s3-compatible", "us-east-1");
    config.endpoint = Some(server.endpoint.clone());
    let resolved = config.resolve().unwrap();
    let creds = Credentials {
        access_key_id: server.access_key_id.clone(),
        secret_access_key: Secret::new(&server.secret),
    };
    let cases = connection_check_cases(&resolved, &creds, "it-bucket", "local").await;
    assert_eq!(cases[0].1.requests, ["HEAD /it-bucket/ -> 200"]);
    assert!(
        !server
            .operations
            .lock()
            .unwrap()
            .iter()
            .any(|op| op != "HeadBucket" && op != "ListObjectsV2"),
        "{:?}",
        server.operations
    );
}

/// Checks what must hold for any page of `prefix`: only immediate children,
/// keys and prefixes inside the folder, prefixes ending in `/`.
fn assert_one_level(prefix: &str, page: &FolderPage) {
    for p in &page.prefixes {
        let rest = p
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("prefix outside the folder"));
        assert!(
            rest.ends_with('/') && rest.find('/') == Some(rest.len() - 1),
            "not an immediate subfolder"
        );
    }
    for o in &page.objects {
        let rest = o
            .key
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("key outside the folder"));
        assert!(
            !rest.contains('/'),
            "a nested key in a delimited listing: {prefix:?} {rest:?}"
        );
    }
}

/// Every entry of `prefix` in pages of `max_keys`: subfolder prefixes as
/// `(prefix, true)`, object keys (the folder's marker included) as
/// `(key, false)`; and the number of pages, at most `max_pages`.
async fn walk_pages(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
    max_keys: i32,
    max_pages: usize,
) -> (Vec<(String, bool)>, usize) {
    let mut entries = Vec::new();
    let mut token: Option<String> = None;
    for pages in 1..=max_pages {
        let page = store
            .list_folder(bucket, prefix, token.as_deref(), max_keys)
            .await
            .unwrap_or_else(|e| panic!("listing a folder: {e}"));
        assert_one_level(prefix, &page);
        assert!(
            page.prefixes.len() + page.objects.len() <= usize::try_from(max_keys).unwrap(),
            "more entries than max-keys"
        );
        entries.extend(page.prefixes.iter().map(|p| (p.clone(), true)));
        entries.extend(page.objects.iter().map(|o| (o.key.clone(), false)));
        match page.next_token {
            Some(next) => token = Some(next),
            None => return (entries, pages),
        }
    }
    (entries, max_pages)
}

/// The browser's listing in every case that can be produced with one valid
/// key and without writing anything: the root, a walk of the folders under
/// `start` (one level per request), pages of one entry against one page of
/// 1000, and a continuation token the provider never issued. Prints counts
/// only, never names. Returns the requests sent.
async fn folder_listing_cases(
    account: &ResolvedAccount,
    creds: &Credentials,
    bucket: &str,
    start: &str,
) -> Vec<String> {
    let requests = Requests::default();
    let mut account = account.clone();
    account.buckets = vec![bucket.to_owned()];
    let options = ClientOptions {
        max_attempts: 1,
        ..ClientOptions::default()
    };
    let store = S3Store::new(account.clone(), creds.clone(), options.clone())
        .with_interceptor(requests.clone());

    // The root: a key restricted to a file name prefix may be refused it.
    let root = store.list_folder(bucket, "", None, 1000).await;
    match &root {
        Ok(page) => {
            assert_one_level("", page);
            println!(
                "root: {} folders, {} objects, more: {}",
                page.prefixes.len(),
                page.objects.len(),
                page.next_token.is_some()
            );
        }
        Err(e) => println!("root: {:?} ({:?})", listing_error(e).kind, e),
    }
    assert!(
        matches!(root, Ok(_) | Err(StorageError::AccessDenied(_))),
        "the root is listed or refused, nothing else: {root:?}"
    );

    // Folder by folder below `start`, one level per request.
    let mut queue = vec![start.to_owned()];
    let (mut folders, mut objects) = (0, 0);
    let mut visited = 0;
    while let Some(prefix) = queue.pop() {
        visited += 1;
        let (entries, _) = walk_pages(&store, bucket, &prefix, 1000, 20).await;
        for (e, is_folder) in entries {
            if is_folder {
                folders += 1;
                if visited + queue.len() < 25 {
                    queue.push(e);
                }
            } else {
                objects += 1;
            }
        }
    }
    println!(
        "under the start prefix: {visited} folders opened, {folders} subfolders, {objects} objects"
    );

    // Pages of one entry give exactly what one page of 1000 gives (a page
    // lists its folders before its files, so only the set is compared).
    let (mut whole, _) = walk_pages(&store, bucket, start, 1000, 1).await;
    let (mut paged, pages) = walk_pages(&store, bucket, start, 1, 200).await;
    if whole.len() < 200 {
        whole.sort();
        paged.sort();
        let mut unique = paged.clone();
        unique.dedup();
        assert_eq!(unique.len(), paged.len(), "an entry on two pages");
        assert_eq!(paged, whole, "paged listing differs");
    }
    println!(
        "pages of 1: {pages} pages for {} entries (one page of 1000: {})",
        paged.len(),
        whole.len()
    );

    // A token the provider never issued.
    let bad = store
        .list_folder(bucket, start, Some("bm90LWEtdG9rZW4="), 1000)
        .await;
    match &bad {
        Ok(page) => println!(
            "made-up token: accepted, {} entries",
            page.prefixes.len() + page.objects.len()
        ),
        Err(e) => println!("made-up token: {:?} ({:?})", listing_error(e).kind, e),
    }

    // The whole path, as the dashboard runs it: account, credential store
    // (in memory here), cursor, folder rules.
    let keys = Arc::new(MemoryCredentialStore::default());
    let state = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        options,
    ));
    let id = state
        .add(
            &AccountInput {
                label: "Listing".into(),
                provider: account.profile.id.into(),
                region: account.region.clone(),
                endpoint: Some(account.endpoint_url.clone()),
                bucket: bucket.into(),
            },
            CredentialInput::new(&creds.access_key_id, creds.secret_access_key.expose()),
        )
        .unwrap()
        .id;
    let listing = browser::list_folder(Arc::clone(&state), id.clone(), start.into(), None)
        .await
        .unwrap_or_else(|e| panic!("list_folder: {e:?}"));
    println!(
        "dashboard listing of the start prefix: {} folders, {} files, more: {}",
        listing.folders.len(),
        listing.files.len(),
        listing.next_page.is_some()
    );
    let root = browser::list_folder(state, id, String::new(), None).await;
    println!(
        "dashboard listing of the root: {}",
        match &root {
            Ok(l) => format!("{} folders, {} files", l.folders.len(), l.files.len()),
            Err(e) => format!("{:?}", e.kind),
        }
    );

    let sent = requests.take();
    for r in &sent {
        let (method, path) = r.split_once(' ').unwrap();
        assert_eq!(method, "GET", "browsing only reads: {r}");
        assert!(
            path.contains("list-type=2") && path.contains("delimiter=%2F"),
            "only ListObjectsV2 with the delimiter: {r}"
        );
    }
    sent
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Backblaze B2 test bucket and key"]
async fn backblaze_b2_folder_listing() {
    let (t, resolved, creds, store) = b2_store();
    let before = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    let sent = folder_listing_cases(&resolved, &creds, &t.bucket, B2_KEY_PREFIX).await;
    let statuses: Vec<&str> = sent
        .iter()
        .map(|r| r.rsplit(" -> ").next().unwrap_or("?"))
        .collect();
    println!("{} requests, statuses {statuses:?}", sent.len());
    let after = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    println!(
        "versions under {B2_KEY_PREFIX}: {} before, {} after; fingerprint {} before, {} after",
        before.1, after.1, before.0, after.0
    );
    assert_eq!(before, after, "listing must not change the bucket");
}

/// The same cases against the in-process listing server, so the harness and
/// its assertions are known to work before they meet a provider.
#[tokio::test(flavor = "multi_thread")]
async fn folder_listing_cases_against_the_test_server() {
    let objects: &[(&str, &str, u64)] = &[
        ("it-bucket", "sotyware-it/", 0),
        ("it-bucket", "sotyware-it/a.txt", 1),
        ("it-bucket", "sotyware-it/b c/d.txt", 1),
        ("it-bucket", "sotyware-it/b c/e/f.txt", 1),
        ("it-bucket", "sotyware-it/x//y.txt", 1),
        ("it-bucket", "outside.txt", 1),
    ];
    let server =
        TestServer::start_listing("test-key", "test-secret", &["it-bucket"], objects, &[]).await;
    let mut config = AccountConfig::new("it-local", "it-local", "s3-compatible", "us-east-1");
    config.endpoint = Some(server.endpoint.clone());
    let resolved = config.resolve().unwrap();
    let sent = folder_listing_cases(
        &resolved,
        &server.credentials(),
        "it-bucket",
        "sotyware-it/",
    )
    .await;
    assert!(sent.len() > 8, "{sent:?}");
    assert!(
        server
            .operations
            .lock()
            .unwrap()
            .iter()
            .all(|op| op == "ListObjectsV2")
    );
}
mod browse;
mod download;

//! Opt-in: one single-file download from the real Backblaze B2 test bucket
//! through the app's own download path
//! (docs/development/testing.md#one-download-from-backblaze).
//!
//! `backblaze_b2_download_file` checks that the account is Backblaze's own
//! endpoint and that the new run prefix
//! `sotyware-it/download-validation/<run id>/` holds no version, uploads one
//! generated file there (its SHA-256 computed locally before the upload),
//! reads its metadata once with the read-only key, downloads it once through
//! [`CloudDownloads`] (the code behind `download_cloud_file`, without the
//! save dialog) with the read-only key, and deletes exactly the version it
//! created.
//!
//! Remote requests, and nothing else: one `ListObjectVersions` of the run
//! prefix and the upload's `HeadObject`, `PutObject` and verifying
//! `HeadObject` (write key, ADR-021), `HeadObject` and `GetObject`
//! (read-only key), `DeleteObject` with the version id and one more
//! `ListObjectVersions` of the run prefix (write key). Every client makes
//! one attempt per request, so a failure is never retried against the
//! provider's daily transaction cap. Cleanup runs even when a check fails
//! (the checks run in their own task).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::super::account::{MemoryAccountStore, ResolvedAccount};
use super::super::credentials::{
    CredentialStore, Credentials, EnvCredentials, MemoryCredentialStore,
};
use super::super::dashboard::{AccountInput, CloudAccounts, CredentialInput};
use super::super::download::{
    CloudDownloads, DownloadEvent, EventSink, Integrity, Request, TransferState,
};
use super::super::s3::{ClientOptions, Existing, S3Store};
use super::super::test_server::{TempDir, TestServer};
use super::{Requests, b2_store, versions};

/// Where every run's file lives, below the test key's own prefix.
const DOWNLOAD_ROOT: &str = "sotyware-it/download-validation/";
/// A space and non-ASCII letters, so the key must reach the provider and come
/// back exactly as given.
const FILE_NAME: &str = "größe test.bin";
/// 16 MiB + 3: one PUT (below the multipart threshold), long enough for more
/// than one progress message, and not a multiple of any buffer size.
const SIZE: usize = 16 * 1024 * 1024 + 3;
const PARTIAL_SUFFIX: &str = ".sotyware-partial";

fn one_attempt() -> ClientOptions {
    ClientOptions {
        max_attempts: 1,
        ..ClientOptions::default()
    }
}

/// Refuses any account but Backblaze's own S3 endpoint for the configured
/// region, before the first remote request.
fn assert_backblaze(resolved: &ResolvedAccount) {
    assert_eq!(resolved.profile.id, "backblaze-b2");
    assert_eq!(
        resolved.endpoint_url,
        format!("https://s3.{}.backblazeb2.com", resolved.region),
        "the configured endpoint must be Backblaze's own"
    );
}

/// Prints `message` on stdout, then panics with it: the Windows driver keeps
/// only a test's stdout (a panic message goes to stderr), and it must still
/// see why a provider step failed.
fn fail(message: String) -> ! {
    println!("{message}");
    panic!("{message}");
}

fn sha256_of(path: &Path) -> String {
    let digest = Sha256::digest(fs::read(path).unwrap());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Temporary download files in `dir`.
fn partials(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(PARTIAL_SUFFIX))
                .collect()
        })
        .unwrap_or_default()
}

/// The destination as it was when the final message was sent.
#[derive(Clone, Debug)]
struct AtFinal {
    dest_len: Option<u64>,
    partials: Vec<String>,
}

/// Records every message and, at the final one, the destination folder.
struct Recorder {
    dest: PathBuf,
    events: Mutex<Vec<DownloadEvent>>,
    at_final: Mutex<Option<AtFinal>>,
}

impl EventSink for Recorder {
    fn emit(&self, event: DownloadEvent) {
        if event.state != TransferState::Active {
            *self.at_final.lock().unwrap() = Some(AtFinal {
                dest_len: fs::metadata(&self.dest).ok().map(|m| m.len()),
                partials: partials(self.dest.parent().unwrap()),
            });
        }
        self.events.lock().unwrap().push(event);
    }
}

/// The file the download must produce, known before it was uploaded.
struct Expected {
    source: PathBuf,
    size: u64,
    sha256: String,
}

/// Adds the account to an in-memory `CloudAccounts` (as the dashboard does,
/// credential store in memory), downloads `key` through [`CloudDownloads`]
/// to `dest` and checks everything the page relies on. Returns the messages.
async fn download_through_app(
    input: AccountInput,
    creds: &Credentials,
    key: &str,
    dest: &Path,
    expected: &Expected,
) -> Vec<DownloadEvent> {
    let bucket = input.bucket.clone();
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        one_attempt(),
    ));
    let id = accounts
        .add(
            &input,
            CredentialInput::new(&creds.access_key_id, creds.secret_access_key.expose()),
        )
        .unwrap_or_else(|e| panic!("adding the account: {e:?}"))
        .id;
    let downloads = Arc::new(CloudDownloads::new());
    let sink = Arc::new(Recorder {
        dest: dest.to_owned(),
        events: Mutex::default(),
        at_final: Mutex::default(),
    });
    let started_at = Instant::now();
    let started = downloads.start(
        downloads.reserve().expect("a free download slot"),
        Arc::clone(&accounts),
        Request {
            account: id,
            bucket,
            key: key.to_owned(),
            dest: dest.to_owned(),
            existing: Existing::Refuse,
        },
        Arc::clone(&sink) as Arc<dyn EventSink>,
    );
    assert_eq!(
        started.file_name,
        dest.file_name().unwrap().to_string_lossy()
    );
    let at_final = loop {
        if let Some(a) = sink.at_final.lock().unwrap().clone() {
            break a;
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(600),
            "the download did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let elapsed = started_at.elapsed();
    let events = sink.events.lock().unwrap().clone();

    let (last, active) = events.split_last().expect("messages");
    println!(
        "download: {:.2} s, {} active messages, final {:?} {:?} bytes {} total {:?}",
        elapsed.as_secs_f64(),
        active.len(),
        last.state,
        last.integrity,
        last.bytes,
        last.total
    );
    println!(
        "active messages (bytes, total): {:?}",
        active
            .iter()
            .map(|e| (e.bytes, e.total))
            .collect::<Vec<_>>()
    );
    println!("at the final message: {at_final:?}");
    assert_eq!(last.state, TransferState::Completed, "{:?}", last.error);
    assert!(active.iter().all(|e| e.state == TransferState::Active));
    assert!(events.iter().all(|e| e.transfer == started.transfer));
    // Integrity "sha256-verified" only comes from a SHA-256 in the GetObject
    // response's metadata equal to the received bytes' hash.
    assert_eq!(last.integrity, Some(Integrity::Sha256Verified));
    assert_eq!(last.bytes, expected.size);
    assert_eq!(last.total, Some(expected.size));
    // The first message comes before the request; every later one carries
    // the size from the GetObject response's Content-Length.
    assert_eq!((active[0].bytes, active[0].total), (0, None));
    assert!(
        active.len() >= 2,
        "no progress after the response: {active:?}"
    );
    for pair in active.windows(2) {
        assert!(
            pair[0].bytes <= pair[1].bytes,
            "bytes went down: {active:?}"
        );
    }
    for e in &active[1..] {
        assert_eq!(e.total, Some(expected.size), "Content-Length: {e:?}");
        assert!(e.bytes <= expected.size);
    }
    // "Completed" is sent only once the file is in place, under its name.
    assert_eq!(at_final.dest_len, Some(expected.size));
    assert!(at_final.partials.is_empty(), "{:?}", at_final.partials);

    let downloaded = sha256_of(dest);
    println!(
        "downloaded file: {} bytes, SHA-256 {downloaded}",
        fs::metadata(dest).unwrap().len()
    );
    assert_eq!(downloaded, expected.sha256);
    assert!(
        fs::read(dest).unwrap() == fs::read(&expected.source).unwrap(),
        "not byte-for-byte equal to the uploaded file"
    );
    assert!(partials(dest.parent().unwrap()).is_empty());
    assert!(!downloads.busy());
    events
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: needs a Backblaze B2 test bucket, its write key and its read-only key"]
async fn backblaze_b2_download_file() {
    let (t, resolved, write_creds, _) = b2_store();
    assert_backblaze(&resolved);
    let read_creds = EnvCredentials::from_process()
        .credentials("it-b2-read")
        .expect(
            "the read-only key (SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID, ..._SECRET_ACCESS_KEY)",
        );
    assert_ne!(
        read_creds.access_key_id, write_creds.access_key_id,
        "the download must use the read-only key"
    );
    let writes = Requests::default();
    let write =
        S3Store::new(resolved.clone(), write_creds, one_attempt()).with_interceptor(writes.clone());
    let run = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    );
    let prefix = format!("{DOWNLOAD_ROOT}{run}/");
    let key = format!("{prefix}{FILE_NAME}");
    println!(
        "backblaze-b2: endpoint {}, bucket {}, key {key:?}",
        resolved.endpoint_url, t.bucket
    );
    // Never upload over anything: the new run prefix must hold no object,
    // version or delete marker.
    let before = versions(&write, &t.bucket, &prefix).await;
    assert!(
        before.is_empty(),
        "the run prefix is not empty, nothing uploaded: {before:?}"
    );

    let source_dir = TempDir::new("b2-download-source");
    let source = source_dir.file("source.bin", SIZE, 7);
    let expected = Expected {
        size: SIZE as u64,
        sha256: sha256_of(&source),
        source,
    };
    println!(
        "generated: {} bytes, SHA-256 {} (computed locally before the upload)",
        expected.size, expected.sha256
    );

    let up = write
        .upload_file(&t.bucket, &key, &expected.source)
        .await
        .unwrap_or_else(|e| panic!("upload: {e}"));
    println!(
        "upload: {} bytes, {} part(s), version {:?}, SHA-256 {}",
        up.size, up.parts, up.version_id, up.sha256
    );
    let Some(version) = up.version_id.clone() else {
        // A delete without a version id would only hide the object.
        panic!("NOT CLEANED UP: {key:?} has no version id; delete it by hand");
    };

    let reads = Requests::default();
    let checks = {
        let (reads, resolved, bucket, key) = (
            reads.clone(),
            resolved.clone(),
            t.bucket.clone(),
            key.clone(),
        );
        tokio::spawn(async move {
            assert_eq!(up.parts, 1);
            assert_eq!(
                (up.size, up.sha256.as_str()),
                (expected.size, expected.sha256.as_str())
            );
            let read = S3Store::new(resolved.clone(), read_creds.clone(), one_attempt())
                .with_interceptor(reads);
            let head = read
                .head_object(&bucket, &key)
                .await
                .unwrap_or_else(|e| panic!("head object: {e}"))
                .expect("the uploaded object");
            println!(
                "head (read-only key): size {}, sotyware SHA-256 metadata {:?}, version {:?}",
                head.size, head.sha256, head.version_id
            );
            assert_eq!(head.size, expected.size);
            assert_eq!(head.sha256.as_deref(), Some(expected.sha256.as_str()));
            let dest_dir = TempDir::new("b2-download-dest");
            let dest = dest_dir.path(FILE_NAME);
            download_through_app(
                AccountInput {
                    label: "Download validation".into(),
                    provider: resolved.profile.id.into(),
                    region: resolved.region.clone(),
                    endpoint: None,
                    bucket,
                },
                &read_creds,
                &key,
                &dest,
                &expected,
            )
            .await;
            drop(dest_dir);
            println!("local destination removed");
        })
        .await
    };

    let deleted = write.delete_object(&t.bucket, &key, Some(&version)).await;
    let left = versions(&write, &t.bucket, &prefix).await;
    for (name, sent) in [
        ("write key", writes.take()),
        ("read-only key", reads.take()),
    ] {
        let shown: Vec<String> = sent
            .iter()
            // Only the path's bucket segment: the bucket may share its name
            // with the key prefix.
            .map(|r| r.replacen(&format!("/{}/", t.bucket), "/<bucket>/", 1))
            .collect();
        println!("{name}: {} requests {shown:?}", shown.len());
    }
    println!(
        "after cleanup: {} versions/delete markers under {prefix}",
        left.len()
    );
    if let Err(e) = checks {
        panic!("the checks failed (cleanup ran first): {e}");
    }
    deleted.unwrap_or_else(|e| panic!("CLEANUP FAILED for {key:?} version {version}: {e}"));
    assert!(left.is_empty(), "left under {prefix}: {left:?}");
}

/// The same download against the in-process object server, so the checks
/// are known to work before they meet the provider.
#[tokio::test(flavor = "multi_thread")]
async fn download_through_app_against_the_test_server() {
    let server = TestServer::start("test-key", "test-secret", &["it-bucket"]).await;
    let source_dir = TempDir::new("download-source");
    let source = source_dir.file("source.bin", SIZE, 7);
    let expected = Expected {
        size: SIZE as u64,
        sha256: sha256_of(&source),
        source,
    };
    let input = AccountInput {
        label: "Local".into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(server.endpoint.clone()),
        bucket: "it-bucket".into(),
    };
    let key = format!("{DOWNLOAD_ROOT}local/{FILE_NAME}");
    let mut config = super::super::account::AccountConfig::new(
        "it-local",
        "it-local",
        "s3-compatible",
        "us-east-1",
    );
    config.endpoint = Some(server.endpoint.clone());
    config.buckets = vec!["it-bucket".into()];
    let store = S3Store::new(
        config.resolve().unwrap(),
        server.credentials(),
        one_attempt(),
    );
    store
        .upload_file("it-bucket", &key, &expected.source)
        .await
        .unwrap();
    let dest_dir = TempDir::new("download-dest");
    let dest = dest_dir.path(FILE_NAME);
    download_through_app(input, &server.credentials(), &key, &dest, &expected).await;
}

/// Where the Windows dashboard download test's objects live
/// (`tests/desktop/windows-cloud-ui.ps1 -Mode b2-download`), one run each.
const UI_ROOT: &str = "sotyware-it/download-ui-validation/";

/// The run prefix of a dashboard download run; the run id comes from the
/// environment for the cleanup, so it may not leave the download root.
fn ui_run_prefix(run: &str) -> String {
    assert!(
        !run.is_empty()
            && run
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "a run id is letters, digits, '-' and '_' only"
    );
    format!("{UI_ROOT}{run}/")
}

/// Prints the requests each key sent, the bucket segment hidden.
fn show_requests(bucket: &str, sent: &[(&str, Vec<String>)]) {
    for (name, sent) in sent {
        let shown: Vec<String> = sent
            .iter()
            .map(|r| r.replacen(&format!("/{bucket}/"), "/<bucket>/", 1))
            .collect();
        println!("{name}: {} requests {shown:?}", shown.len());
    }
}

/// First half of the Windows dashboard download test: checks the account
/// and that the new run prefix holds no version or delete marker, generates
/// the file into `SOTYWARE_IT_B2_DOWNLOAD_UI_DIR` (`source.bin`, SHA-256
/// computed before the upload), uploads it with the write key and writes
/// `manifest.json` next to it. The object is kept for the dashboard; the
/// driver deletes it with `backblaze_b2_download_ui_cleanup`.
///
/// Remote requests: one `ListObjectVersions` of the run prefix and the
/// upload's `HeadObject`, `PutObject` and verifying `HeadObject`, all with
/// the write key, one attempt each.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: uploads one file to the Backblaze B2 test bucket and keeps it"]
async fn backblaze_b2_download_ui_prepare() {
    let dir = PathBuf::from(
        std::env::var("SOTYWARE_IT_B2_DOWNLOAD_UI_DIR")
            .expect("set SOTYWARE_IT_B2_DOWNLOAD_UI_DIR to an empty local folder"),
    );
    assert!(
        fs::read_dir(&dir)
            .expect("the local folder")
            .next()
            .is_none(),
        "{dir:?} is not empty"
    );
    let (t, resolved, write_creds, _) = b2_store();
    assert_backblaze(&resolved);
    let read_creds = EnvCredentials::from_process()
        .credentials("it-b2-read")
        .expect("the read-only key (SOTYWARE_CLOUD_IT_B2_READ_*)");
    assert_ne!(
        read_creds.access_key_id, write_creds.access_key_id,
        "the dashboard must download with the read-only key, not the write key"
    );
    let writes = Requests::default();
    let write =
        S3Store::new(resolved.clone(), write_creds, one_attempt()).with_interceptor(writes.clone());
    let run = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    );
    let prefix = ui_run_prefix(&run);
    let key = format!("{prefix}{FILE_NAME}");
    println!(
        "provider {}, region {}, endpoint {}, run prefix {prefix}",
        resolved.profile.id, resolved.region, resolved.endpoint_url
    );
    let before = versions(&write, &t.bucket, &prefix).await;
    if !before.is_empty() {
        fail(format!(
            "run prefix not empty, nothing uploaded: {} versions/delete markers",
            before.len()
        ));
    }
    println!("run prefix: no objects, versions or delete markers");

    let generated = TempDir::new("b2-download-ui");
    let source = dir.join("source.bin");
    fs::copy(generated.file("source.bin", SIZE, 7), &source).expect("copy the generated file");
    drop(generated);
    let sha256 = sha256_of(&source);
    println!("generated: {SIZE} bytes, SHA-256 {sha256} (computed locally before the upload)");

    let up = write.upload_file(&t.bucket, &key, &source).await;
    show_requests(&t.bucket, &[("write key", writes.take())]);
    let up = up.unwrap_or_else(|e| {
        fail(format!(
            "upload failed: {e}; if the request reached the provider, delete what is under {prefix} by version id"
        ))
    });
    let Some(version) = up.version_id.clone() else {
        fail(format!(
            "upload: NOT CLEANED UP: {key:?} has no version id; delete it by hand"
        ));
    };
    // The driver reads the run id and version from this line even if a later
    // step fails, so the cleanup always gets them.
    println!("download-ui object kept; run id {run} version {version}");
    println!(
        "upload: {} bytes, {} part(s), SHA-256 {} (sent as the sotyware SHA-256 metadata)",
        up.size, up.parts, up.sha256
    );
    assert_eq!(up.parts, 1);
    assert_eq!(
        (up.size, up.sha256.as_str()),
        (SIZE as u64, sha256.as_str())
    );
    let manifest = serde_json::json!({
        "run": run,
        "prefix": prefix,
        "key": key,
        "fileName": FILE_NAME,
        "size": up.size,
        "sha256": sha256,
        "versionId": version,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .expect("write the manifest");
    println!("manifest written; clean up with backblaze_b2_download_ui_cleanup");
}

/// Second half: deletes exactly the version a prepare run created
/// (`SOTYWARE_IT_B2_DOWNLOAD_UI_RUN`, `SOTYWARE_IT_B2_DOWNLOAD_UI_VERSION`),
/// never without a version id, then lists the run prefix's versions once and
/// fails unless none is left. If the delete is refused (for example by the
/// daily transaction cap), nothing more is sent.
///
/// Remote requests: one `DeleteObject` with the version id and one
/// `ListObjectVersions` of the run prefix, write key, one attempt each.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: deletes one download-ui-validation version from the Backblaze B2 test bucket"]
async fn backblaze_b2_download_ui_cleanup() {
    let run = std::env::var("SOTYWARE_IT_B2_DOWNLOAD_UI_RUN")
        .expect("set SOTYWARE_IT_B2_DOWNLOAD_UI_RUN to the run id");
    let version = std::env::var("SOTYWARE_IT_B2_DOWNLOAD_UI_VERSION")
        .expect("set SOTYWARE_IT_B2_DOWNLOAD_UI_VERSION to the version id");
    assert!(!version.is_empty(), "never delete without a version id");
    let prefix = ui_run_prefix(&run);
    let key = format!("{prefix}{FILE_NAME}");
    let (t, resolved, write_creds, _) = b2_store();
    assert_backblaze(&resolved);
    let writes = Requests::default();
    let write = S3Store::new(resolved, write_creds, one_attempt()).with_interceptor(writes.clone());
    println!("cleaning up run {run}: deleting version {version} of {key:?}");
    let deleted = write.delete_object(&t.bucket, &key, Some(&version)).await;
    if let Err(e) = deleted {
        show_requests(&t.bucket, &[("write key", writes.take())]);
        fail(format!(
            "CLEANUP FAILED for {key:?} version {version}: {e}; nothing more was sent"
        ));
    }
    println!("cleanup: version {version} deleted");
    let left = versions(&write, &t.bucket, &prefix).await;
    show_requests(&t.bucket, &[("write key", writes.take())]);
    println!(
        "cleanup: {} versions/delete markers left under {prefix}",
        left.len()
    );
    assert!(left.is_empty(), "left under {prefix}: {left:?}");
}

#[test]
fn ui_run_prefix_stays_under_the_download_ui_root() {
    assert_eq!(
        ui_run_prefix("123-45"),
        "sotyware-it/download-ui-validation/123-45/"
    );
    for bad in ["", "..", "a/b", "../browse-validation", "a b"] {
        assert!(
            std::panic::catch_unwind(|| ui_run_prefix(bad)).is_err(),
            "{bad:?}"
        );
    }
}

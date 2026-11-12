//! Tests of single-file downloads: the pure rules (suggested names,
//! destinations, error mapping, throttling) and whole transfers through
//! [`CloudDownloads`] and [`S3Store::download`] against the in-process object
//! server, whose bodies can be slow, stall or break off. No real provider,
//! no save dialog (the dialog only produces the path these tests pass).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::s3::{ClientOptions, S3Store};
use super::super::test_server::{Fault, TempDir, TestObject, TestServer, pattern, sha256_hex};
use super::*;

const KEY_ID: &str = "download-test-key";
const SECRET: &str = "download-Test-Secret-0123456789";
const BUCKET: &str = "download-bucket";
const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

fn options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        // Long, so a cancelled stall is never mistaken for a timeout.
        read_timeout: Duration::from_secs(30),
        max_attempts: 1,
        ..ClientOptions::default()
    }
}

/// Records every message; the transfers run on Tauri's runtime.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<DownloadEvent>>,
}

impl EventSink for Recorder {
    fn emit(&self, event: DownloadEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn events(&self) -> Vec<DownloadEvent> {
        self.events.lock().unwrap().clone()
    }

    fn of(&self, transfer: &str) -> Vec<DownloadEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.transfer == transfer)
            .collect()
    }

    /// Waits for `transfer`'s final message (a test helper, so polling is
    /// fine here).
    async fn finished(&self, transfer: &str) -> DownloadEvent {
        let started = Instant::now();
        loop {
            if let Some(e) = self
                .of(transfer)
                .into_iter()
                .find(|e| e.state != TransferState::Active)
            {
                return e;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "{transfer} did not finish: {:?}",
                self.of(transfer)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

struct Setup {
    accounts: Arc<CloudAccounts>,
    keys: Arc<MemoryCredentialStore>,
    downloads: Arc<CloudDownloads>,
    sink: Arc<Recorder>,
    dir: TempDir,
}

fn input(label: &str, endpoint: &str, bucket: &str) -> AccountInput {
    AccountInput {
        label: label.into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint.into()),
        bucket: bucket.into(),
    }
}

/// One account (id `local`) on `server`'s `BUCKET`.
fn setup(server: &TestServer) -> Setup {
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        options(),
    ));
    let id = accounts
        .add(
            &input("Local", &server.endpoint, BUCKET),
            CredentialInput::new(&server.access_key_id, &server.secret),
        )
        .unwrap()
        .id;
    assert_eq!(id, "local");
    Setup {
        accounts,
        keys,
        downloads: Arc::new(CloudDownloads::new()),
        sink: Arc::new(Recorder::default()),
        dir: TempDir::new("download"),
    }
}

impl Setup {
    fn request(&self, key: &str, dest: &Path, existing: Existing) -> Request {
        Request {
            account: "local".into(),
            bucket: BUCKET.into(),
            key: key.into(),
            dest: dest.to_owned(),
            existing,
        }
    }

    fn start(&self, request: Request) -> Result<DownloadStarted, CommandError> {
        let reservation = self.downloads.reserve()?;
        Ok(self.downloads.start(
            reservation,
            Arc::clone(&self.accounts),
            request,
            Arc::clone(&self.sink) as Arc<dyn EventSink>,
        ))
    }

    async fn download(&self, key: &str, name: &str) -> (DownloadStarted, DownloadEvent) {
        let started = self
            .start(self.request(key, &self.dir.path(name), Existing::Refuse))
            .unwrap();
        let done = self.sink.finished(&started.transfer).await;
        (started, done)
    }

    /// Waits until the running download's temporary file holds at least
    /// `bytes` (progress messages are throttled, so they may only show it
    /// at the end).
    async fn on_disk(&self, bytes: u64) {
        let started = Instant::now();
        loop {
            let size = self
                .downloads
                .active_control()
                .and_then(|c| c.partial_file())
                .and_then(|p| fs::metadata(p).ok())
                .map_or(0, |m| m.len());
            if size >= bytes {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "{size} bytes on disk: {:?}",
                self.sink.events()
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// The names in the destination folder.
    fn listing(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

async fn serve(objects: Vec<TestObject>) -> TestServer {
    let objects: Vec<(&str, TestObject)> = objects.into_iter().map(|o| (BUCKET, o)).collect();
    TestServer::start_objects(KEY_ID, SECRET, &objects).await
}

fn kind(event: &DownloadEvent) -> Option<ErrorKind> {
    event.error.as_ref().map(|e| e.kind)
}

// ---- pure rules ----

#[test]
fn the_suggested_name_is_the_last_segment_made_safe() {
    assert_eq!(suggested_file_name("photos/2024/beach.jpg"), "beach.jpg");
    assert_eq!(suggested_file_name("readme.txt"), "readme.txt");
    assert_eq!(suggested_file_name("a//b"), "b");
    assert_eq!(
        suggested_file_name("Ferien ü/Strand (1).jpg"),
        "Strand (1).jpg"
    );
    // Never a path: separators, `..` and Windows-illegal characters.
    assert_eq!(suggested_file_name("../../etc/passwd"), "passwd");
    assert_eq!(suggested_file_name(".."), "download");
    assert_eq!(suggested_file_name("dir/.."), "download");
    assert_eq!(suggested_file_name("folder/"), "download");
    assert_eq!(suggested_file_name("a\\b:c*d?.txt"), "a_b_c_d_.txt");
    assert_eq!(suggested_file_name("x\u{0}y\n.txt"), "x_y_.txt");
    assert_eq!(suggested_file_name("trailing. . "), "trailing");
    assert_eq!(suggested_file_name("CON.txt"), "_CON.txt");
    assert_eq!(suggested_file_name("com1"), "_com1");
    assert_eq!(suggested_file_name("console.txt"), "console.txt");
    let long = suggested_file_name(&"ü".repeat(300));
    assert!(long.len() <= MAX_SUGGESTED_NAME_BYTES && long.chars().all(|c| c == 'ü'));
}

#[test]
fn keys_must_have_a_valid_length() {
    assert!(validate_key("a").is_ok());
    assert!(validate_key(&"k".repeat(1024)).is_ok());
    assert_eq!(validate_key("").unwrap_err().kind, ErrorKind::InvalidInput);
    assert_eq!(
        validate_key(&"k".repeat(1025)).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn only_the_file_the_dialog_returned_existing_may_be_replaced() {
    let dir = TempDir::new("destination");
    let new = dir.path("new.bin");
    assert_eq!(destination(new.clone()).unwrap(), (new, Existing::Refuse));
    let old = dir.file("old.bin", 3, 1);
    assert_eq!(destination(old.clone()).unwrap(), (old, Existing::Replace));
    assert_eq!(
        destination(dir.0.clone()).unwrap_err().kind,
        ErrorKind::InvalidInput,
        "a folder is never a destination"
    );
    assert_eq!(
        destination(PathBuf::from("relative.bin")).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn the_throttle_lets_one_message_through_per_interval() {
    let mut t = Throttle::new(Duration::from_millis(250));
    let t0 = Instant::now();
    assert!(t.ready(t0), "the first message always goes");
    assert!(!t.ready(t0 + Duration::from_millis(100)));
    assert!(!t.ready(t0 + Duration::from_millis(249)));
    assert!(t.ready(t0 + Duration::from_millis(250)));
    assert!(!t.ready(t0 + Duration::from_millis(400)));
    assert!(t.ready(t0 + Duration::from_secs(1)));
}

#[test]
fn errors_are_shown_with_sotywares_own_words() {
    use std::io::ErrorKind as Io;
    let local = |kind| {
        download_error(&StorageError::LocalWrite {
            kind,
            message: "/home/u/x.bin: details".into(),
        })
    };
    assert_eq!(local(Io::StorageFull).kind, ErrorKind::DiskFull);
    assert_eq!(local(Io::QuotaExceeded).kind, ErrorKind::DiskFull);
    assert_eq!(local(Io::PermissionDenied).kind, ErrorKind::LocalPermission);
    assert_eq!(
        local(Io::ReadOnlyFilesystem).kind,
        ErrorKind::LocalPermission
    );
    assert_eq!(local(Io::NotFound).kind, ErrorKind::DestinationUnavailable);
    assert_eq!(local(Io::AlreadyExists).kind, ErrorKind::DestinationExists);
    assert_eq!(local(Io::Other).kind, ErrorKind::LocalFile);
    let provider = download_error(&StorageError::Provider {
        status: 500,
        code: "InternalError".into(),
        message: "provider text with request id ABC".into(),
    });
    assert_eq!(provider.kind, ErrorKind::Provider);
    assert!(!provider.message.contains("ABC"), "{}", provider.message);
    for (e, k) in [
        (
            StorageError::InvalidCredentials("raw".into()),
            ErrorKind::InvalidCredentials,
        ),
        (
            StorageError::AccessDenied("raw".into()),
            ErrorKind::AccessDenied,
        ),
        (
            StorageError::NotFound("raw".into()),
            ErrorKind::ObjectNotFound,
        ),
        (StorageError::Network("raw".into()), ErrorKind::Network),
        (StorageError::Timeout("raw".into()), ErrorKind::Timeout),
        (StorageError::Integrity("raw".into()), ErrorKind::Integrity),
    ] {
        let shown = download_error(&e);
        assert_eq!(shown.kind, k);
        assert!(!shown.message.contains("raw"), "{}", shown.message);
    }
}

// ---- whole transfers ----

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_downloaded_verified_and_reported() {
    let data = pattern(300 * KIB, 1);
    let server = serve(vec![TestObject::new("docs/report.pdf", data.clone())]).await;
    let s = setup(&server);
    let (started, done) = s.download("docs/report.pdf", "report.pdf").await;
    assert_eq!(started.file_name, "report.pdf");
    assert_eq!(done.state, TransferState::Completed);
    assert_eq!(done.integrity, Some(Integrity::Sha256Verified));
    assert_eq!(
        (done.bytes, done.total),
        (data.len() as u64, Some(data.len() as u64))
    );
    assert_eq!(fs::read(s.dir.path("report.pdf")).unwrap(), data);
    assert_eq!(s.listing(), ["report.pdf"], "no temporary file left");

    let events = s.sink.of(&started.transfer);
    assert_eq!(events.first().unwrap().state, TransferState::Active);
    assert_eq!(events.first().unwrap().bytes, 0);
    assert_eq!(
        events.last().unwrap(),
        &done,
        "the final message comes last"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.state != TransferState::Active)
            .count(),
        1
    );
    // Bytes only grow.
    assert!(events.windows(2).all(|w| w[0].bytes <= w[1].bytes));
    assert!(!s.downloads.busy());
    assert_eq!(*server.operations.lock().unwrap(), ["GetObject"]);
    // Nothing a page receives carries the secret.
    let json = serde_json::to_string(&s.sink.events()).unwrap();
    assert!(!json.contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_file_is_downloaded() {
    let server = serve(vec![TestObject::new("empty.txt", Vec::new())]).await;
    let s = setup(&server);
    let (_, done) = s.download("empty.txt", "empty.txt").await;
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    assert_eq!((done.bytes, done.total), (0, Some(0)));
    assert_eq!(done.integrity, Some(Integrity::Sha256Verified));
    assert_eq!(fs::metadata(s.dir.path("empty.txt")).unwrap().len(), 0);
    assert_eq!(s.listing(), ["empty.txt"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_file_is_streamed_to_disk_chunk_by_chunk() {
    let size = 24 * MIB;
    let data = pattern(size, 2);
    let sha = sha256_hex(&data);
    let server = serve(vec![TestObject::new("big.bin", data)]).await;
    let store = S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    );
    let dir = TempDir::new("large");
    let dest = dir.path("big.bin");
    let control = TransferControl::new();
    let mut calls = 0usize;
    let mut largest_step = 0u64;
    let mut last = 0u64;
    let mut seen_partial_midway = false;
    let partial_dir = dir.0.clone();
    let mut progress = |bytes: u64, total: Option<u64>| {
        assert_eq!(total, Some(size as u64));
        calls += 1;
        largest_step = largest_step.max(bytes - last);
        last = bytes;
        // While it streams, the data is on disk in the temporary file, not
        // collected in memory: the file grows with the progress. (Asked of
        // the file itself: on Windows a directory listing reports the size
        // of a file that is still open as of its last close.)
        if bytes > 0 && bytes < size as u64 {
            let on_disk: u64 = fs::read_dir(&partial_dir)
                .unwrap()
                .map(|e| e.unwrap())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .ends_with(".sotyware-partial")
                })
                .map(|e| fs::metadata(e.path()).map_or(0, |m| m.len()))
                .sum();
            if on_disk > 0 && on_disk < size as u64 {
                seen_partial_midway = true;
            }
        }
    };
    let report = store
        .download(
            BUCKET,
            "big.bin",
            &dest,
            Existing::Refuse,
            &control,
            &mut progress,
        )
        .await
        .unwrap();
    assert_eq!(report.sha256, sha);
    assert!(calls > 20, "{calls} progress calls");
    assert!(
        largest_step <= MIB as u64,
        "a chunk of {largest_step} bytes"
    );
    assert!(
        seen_partial_midway,
        "the temporary file grew while streaming"
    );
    assert_eq!(fs::metadata(&dest).unwrap().len(), size as u64);
    assert_eq!(control.partial_file(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_is_throttled_and_an_unknown_size_is_reported_as_none() {
    let data = pattern(40 * 64 * KIB, 3); // 40 chunks
    let mut object = TestObject::new("slow.bin", data.clone())
        .with_fault(Fault::Slow(Duration::from_millis(25)));
    object.announce_length = false;
    let server = serve(vec![object]).await;
    let s = setup(&server);
    let started_at = Instant::now();
    let (started, done) = s.download("slow.bin", "slow.bin").await;
    let elapsed = started_at.elapsed();
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    // The provider gave no size, so neither progress nor the end claims one
    // beyond what was received.
    let active: Vec<_> = s
        .sink
        .of(&started.transfer)
        .into_iter()
        .filter(|e| e.state == TransferState::Active)
        .collect();
    assert!(active.iter().all(|e| e.total.is_none()), "{active:?}");
    assert_eq!(done.total, Some(data.len() as u64));
    // 40 chunks took about a second; at most one message per 250 ms plus
    // the initial ones.
    let allowed = (elapsed.as_millis() / PROGRESS_INTERVAL.as_millis()) as usize + 3;
    assert!(
        active.len() <= allowed && active.len() >= 3,
        "{} active messages in {elapsed:?}",
        active.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_stops_the_transfer_and_leaves_nothing() {
    let data = pattern(MIB, 4);
    let server = serve(vec![
        TestObject::new("stall.bin", data).with_fault(Fault::StallAfter(256 * KIB)),
    ])
    .await;
    let s = setup(&server);
    let started = s
        .start(s.request("stall.bin", &s.dir.path("stall.bin"), Existing::Refuse))
        .unwrap();
    s.on_disk(256 * KIB as u64).await;
    assert!(
        s.downloads
            .active_control()
            .unwrap()
            .partial_file()
            .is_some()
    );

    // Another id never touches it.
    assert_eq!(s.downloads.cancel("download-999"), CancelOutcome::NotActive);
    assert!(s.downloads.busy());

    let at = Instant::now();
    assert_eq!(
        s.downloads.cancel(&started.transfer),
        CancelOutcome::Cancelling
    );
    assert_eq!(
        s.downloads.cancel(&started.transfer),
        CancelOutcome::Cancelling,
        "cancelling twice is harmless"
    );
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(done.state, TransferState::Cancelled);
    assert!(
        at.elapsed() < Duration::from_secs(5),
        "stopped by cancelling, not by the 30 s read timeout: {:?}",
        at.elapsed()
    );
    assert_eq!(
        s.listing(),
        Vec::<String>::new(),
        "no partial or final file"
    );
    assert_eq!(
        s.downloads.cancel(&started.transfer),
        CancelOutcome::NotActive
    );
    assert!(!s.downloads.busy());
    // A cancelled download is not offered for retry.
    assert!(
        s.downloads
            .retry(
                &started.transfer,
                Arc::clone(&s.accounts),
                Arc::clone(&s.sink) as Arc<dyn EventSink>
            )
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_just_before_the_final_move_wins() {
    let server = serve(vec![
        TestObject::new("race.bin", pattern(2 * MIB, 5))
            .with_fault(Fault::Slow(Duration::from_millis(20))),
    ])
    .await;
    let s = setup(&server);
    let started = s
        .start(s.request("race.bin", &s.dir.path("race.bin"), Existing::Refuse))
        .unwrap();
    s.on_disk(64 * KIB as u64).await;
    let outcome = Arc::new(Mutex::new(None));
    {
        let (downloads, id, outcome) = (
            Arc::clone(&s.downloads),
            started.transfer.clone(),
            Arc::clone(&outcome),
        );
        s.downloads
            .active_control()
            .unwrap()
            .before_commit(move || {
                *outcome.lock().unwrap() = Some(downloads.cancel(&id));
            });
    }
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(*outcome.lock().unwrap(), Some(CancelOutcome::Cancelling));
    assert_eq!(done.state, TransferState::Cancelled);
    assert_eq!(s.listing(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_after_the_final_move_began_is_too_late_and_the_download_completes() {
    let data = pattern(2 * MIB, 6);
    let server = serve(vec![
        TestObject::new("race.bin", data.clone())
            .with_fault(Fault::Slow(Duration::from_millis(20))),
    ])
    .await;
    let s = setup(&server);
    let started = s
        .start(s.request("race.bin", &s.dir.path("race.bin"), Existing::Refuse))
        .unwrap();
    s.on_disk(64 * KIB as u64).await;
    let outcome = Arc::new(Mutex::new(None));
    {
        let (downloads, id, outcome) = (
            Arc::clone(&s.downloads),
            started.transfer.clone(),
            Arc::clone(&outcome),
        );
        s.downloads.active_control().unwrap().after_commit(move || {
            *outcome.lock().unwrap() = Some(downloads.cancel(&id));
        });
    }
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(*outcome.lock().unwrap(), Some(CancelOutcome::Finishing));
    assert_eq!(done.state, TransferState::Completed);
    assert_eq!(fs::read(s.dir.path("race.bin")).unwrap(), data);
    assert_eq!(s.listing(), ["race.bin"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checksum_mismatch_fails_and_writes_nothing() {
    let mut object = TestObject::new("tampered.bin", pattern(200 * KIB, 7));
    object.sha256 = Some(sha256_hex(b"something else"));
    let server = serve(vec![object]).await;
    let s = setup(&server);
    let (_, done) = s.download("tampered.bin", "tampered.bin").await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(kind(&done), Some(ErrorKind::Integrity));
    assert_eq!(done.integrity, None);
    assert_eq!(s.listing(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_stored_checksum_only_the_size_is_claimed() {
    let data = pattern(70 * KIB, 8);
    let server = serve(vec![TestObject::foreign("foreign.bin", data.clone())]).await;
    let s = setup(&server);
    let (_, done) = s.download("foreign.bin", "foreign.bin").await;
    assert_eq!(done.state, TransferState::Completed);
    assert_eq!(done.integrity, Some(Integrity::SizeOnly));
    assert_eq!(fs::read(s.dir.path("foreign.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_connection_fails_cleanly_and_can_be_retried() {
    let server = serve(vec![
        TestObject::new("cut.bin", pattern(MIB, 9)).with_fault(Fault::CutAfter(300 * KIB)),
    ])
    .await;
    let s = setup(&server);
    let (started, done) = s.download("cut.bin", "cut.bin").await;
    assert_eq!(done.state, TransferState::Failed, "{done:?}");
    assert!(
        matches!(kind(&done), Some(ErrorKind::Network | ErrorKind::Integrity)),
        "{done:?}"
    );
    assert_eq!(s.listing(), Vec::<String>::new());
    // Retry runs again with the same destination (and fails the same way
    // here, the server always cuts).
    let again = s
        .downloads
        .retry(
            &started.transfer,
            Arc::clone(&s.accounts),
            Arc::clone(&s.sink) as Arc<dyn EventSink>,
        )
        .unwrap();
    assert_ne!(again.transfer, started.transfer);
    assert_eq!(again.file_name, "cut.bin");
    let done = s.sink.finished(&again.transfer).await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(s.listing(), Vec::<String>::new());
    // Only the latest failure can be retried.
    assert!(
        s.downloads
            .retry(
                &started.transfer,
                Arc::clone(&s.accounts),
                Arc::clone(&s.sink) as Arc<dyn EventSink>
            )
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_after_fixing_the_key_completes_to_the_same_destination() {
    let data = pattern(50 * KIB, 10);
    let server = serve(vec![TestObject::new("k.bin", data.clone())]).await;
    let s = setup(&server);
    s.keys.delete("local").unwrap();
    // `connect` caches stores; a changed account is a new one.
    let s = Setup {
        accounts: Arc::new(CloudAccounts::load(
            Ok(Box::new(MemoryAccountStore {
                rows: Arc::new(Mutex::new(vec![server.account("local", &[BUCKET])])),
                ..MemoryAccountStore::default()
            })),
            Arc::clone(&s.keys) as Arc<dyn CredentialStore>,
            options(),
        )),
        ..s
    };
    let (started, done) = s.download("k.bin", "k.bin").await;
    assert_eq!(kind(&done), Some(ErrorKind::CredentialsMissing), "{done:?}");
    s.keys.store("local", &server.credentials()).unwrap();
    let again = s
        .downloads
        .retry(
            &started.transfer,
            Arc::clone(&s.accounts),
            Arc::clone(&s.sink) as Arc<dyn EventSink>,
        )
        .unwrap();
    let done = s.sink.finished(&again.transfer).await;
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    assert_eq!(fs::read(s.dir.path("k.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_disk_fails_and_removes_the_temporary_file() {
    let server = serve(vec![TestObject::new("big.bin", pattern(2 * MIB, 11))]).await;
    let store = S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    );
    let dir = TempDir::new("full");
    let control = TransferControl::new();
    control.fail_writes_after(300 * KIB as u64);
    let e = store
        .download(
            BUCKET,
            "big.bin",
            &dir.path("big.bin"),
            Existing::Refuse,
            &control,
            &mut |_, _| {},
        )
        .await
        .unwrap_err();
    assert_eq!(download_error(&e).kind, ErrorKind::DiskFull, "{e:?}");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    assert_eq!(control.partial_file(), None);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_without_write_permission_is_reported() {
    use std::os::unix::fs::PermissionsExt;
    let server = serve(vec![TestObject::new("a.bin", pattern(KIB, 12))]).await;
    let s = setup(&server);
    let locked = s.dir.path("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(locked.join("probe"), b"").is_ok() {
        // Running as root: permissions do not apply.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let started = s
        .start(s.request("a.bin", &locked.join("a.bin"), Existing::Refuse))
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::LocalPermission), "{done:?}");
    // A folder that vanished:
    let started = s
        .start(s.request("a.bin", &s.dir.path("gone/a.bin"), Existing::Refuse))
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(
        kind(&done),
        Some(ErrorKind::DestinationUnavailable),
        "{done:?}"
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(fs::read_dir(&locked).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_file_is_never_replaced_unless_the_user_chose_to() {
    let data = pattern(400 * KIB, 13);
    let server = serve(vec![
        TestObject::new("f.bin", data.clone()),
        TestObject::new("slow.bin", pattern(2 * MIB, 19))
            .with_fault(Fault::Slow(Duration::from_millis(20))),
    ])
    .await;
    let s = setup(&server);

    // Exists before the download: refused at once, untouched.
    let mine = s.dir.path("mine.bin");
    fs::write(&mine, b"my own file").unwrap();
    let started = s
        .start(s.request("f.bin", &mine, Existing::Refuse))
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::DestinationExists));
    assert_eq!(fs::read(&mine).unwrap(), b"my own file");

    // Appears while the download runs: still untouched.
    let late = s.dir.path("late.bin");
    let started = s
        .start(s.request("slow.bin", &late, Existing::Refuse))
        .unwrap();
    s.on_disk(64 * KIB as u64).await;
    fs::write(&late, b"created meanwhile").unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::DestinationExists), "{done:?}");
    assert_eq!(fs::read(&late).unwrap(), b"created meanwhile");

    // Chosen for replacement in the dialog: replaced in one step.
    let started = s
        .start(s.request("f.bin", &mine, Existing::Replace))
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(done.state, TransferState::Completed);
    assert_eq!(fs::read(&mine).unwrap(), data);

    // A failed replacement keeps the old file.
    let mut keep = TestObject::new("bad.bin", pattern(KIB, 14));
    keep.sha256 = Some(sha256_hex(b"other"));
    drop(server);
    let server = serve(vec![keep]).await;
    let s2 = Setup {
        dir: TempDir::new("replace-fail"),
        ..setup(&server)
    };
    let old = s2.dir.file("old.bin", 10, 15);
    let before = fs::read(&old).unwrap();
    let started = s2
        .start(s2.request("bad.bin", &old, Existing::Replace))
        .unwrap();
    let done = s2.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::Integrity));
    assert_eq!(fs::read(&old).unwrap(), before);
    assert_eq!(s2.listing(), ["old.bin"]);
    assert_eq!(s.listing(), ["late.bin", "mine.bin"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn unusual_keys_are_requested_exactly_and_never_become_paths() {
    let keys = [
        "a//b",
        "../../outside.txt",
        "./dot",
        "Ferien 2024 ü/Strand Bild (1).jpg",
        "日本語/ファイル.txt",
        "+ & ? # % = ;.txt",
        " leading and trailing ",
        "back\\slash\\name",
        "emoji 🎉.bin",
        "percent %2F encoded",
    ];
    let objects: Vec<TestObject> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| TestObject::new(k, format!("content of {i}: {k}").into_bytes()))
        .collect();
    let server = serve(objects).await;
    let s = setup(&server);
    for (i, key) in keys.iter().enumerate() {
        let name = format!("out-{i}.bin");
        let (_, done) = s.download(key, &name).await;
        assert_eq!(done.state, TransferState::Completed, "{key}: {done:?}");
        assert_eq!(
            fs::read_to_string(s.dir.path(&name)).unwrap(),
            format!("content of {i}: {key}"),
            "{key}"
        );
    }
    // Only the chosen files exist; `../../outside.txt` wrote nothing above.
    assert_eq!(s.listing().len(), keys.len());
    assert!(!s.dir.0.parent().unwrap().join("outside.txt").exists());
    // A key that differs only in a way a "normalizer" would erase is a
    // different object.
    let (_, done) = s.download("a/b", "normalized.bin").await;
    assert_eq!(kind(&done), Some(ErrorKind::ObjectNotFound));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_use_their_own_keys_and_get_distinct_transfer_ids() {
    let a = TestServer::start_objects(
        "key-a",
        "secret-a-0123456789",
        &[(BUCKET, TestObject::new("same.txt", b"from a".to_vec()))],
    )
    .await;
    let b = TestServer::start_objects(
        "key-b",
        "secret-b-0123456789",
        &[(BUCKET, TestObject::new("same.txt", b"from b".to_vec()))],
    )
    .await;
    let s = setup(&a);
    let id_b = s
        .accounts
        .add(
            &input("Other", &b.endpoint, BUCKET),
            CredentialInput::new(&b.access_key_id, &b.secret),
        )
        .unwrap()
        .id;
    let first = s
        .start(s.request("same.txt", &s.dir.path("a.txt"), Existing::Refuse))
        .unwrap();
    // One at a time: a second download is refused while the first runs, or
    // starts after it.
    s.sink.finished(&first.transfer).await;
    let second = s
        .start(Request {
            account: id_b.clone(),
            ..s.request("same.txt", &s.dir.path("b.txt"), Existing::Refuse)
        })
        .unwrap();
    s.sink.finished(&second.transfer).await;
    assert_ne!(first.transfer, second.transfer);
    assert_eq!(fs::read(s.dir.path("a.txt")).unwrap(), b"from a");
    assert_eq!(fs::read(s.dir.path("b.txt")).unwrap(), b"from b");
    // Each message names its own transfer.
    assert!(
        s.sink
            .of(&first.transfer)
            .iter()
            .all(|e| e.transfer == first.transfer)
    );
    assert_eq!(
        s.sink.events().len(),
        s.sink.of(&first.transfer).len() + s.sink.of(&second.transfer).len()
    );

    // B's key at A's server is refused as invalid credentials.
    let wrong = S3Store::new(b.resolved(&id_b, &[BUCKET]), a.credentials(), options());
    let e = wrong
        .download_file(BUCKET, "same.txt", &s.dir.path("c.txt"))
        .await
        .unwrap_err();
    assert_eq!(
        download_error(&e).kind,
        ErrorKind::InvalidCredentials,
        "{e:?}"
    );
    assert!(!s.dir.path("c.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_one_download_or_dialog_at_a_time() {
    let server = serve(vec![
        TestObject::new("stall.bin", pattern(MIB, 16)).with_fault(Fault::StallAfter(64 * KIB)),
    ])
    .await;
    let s = setup(&server);
    // A dialog that was cancelled frees the slot.
    let reservation = s.downloads.reserve().unwrap();
    assert_eq!(s.downloads.reserve().err().unwrap().kind, ErrorKind::Busy);
    drop(reservation);
    let started = s
        .start(s.request("stall.bin", &s.dir.path("x.bin"), Existing::Refuse))
        .unwrap();
    let busy = s
        .start(s.request("stall.bin", &s.dir.path("y.bin"), Existing::Refuse))
        .unwrap_err();
    assert_eq!(busy.kind, ErrorKind::Busy);
    s.downloads.cancel(&started.transfer);
    assert_eq!(
        s.sink.finished(&started.transfer).await.state,
        TransferState::Cancelled
    );
    assert!(s.downloads.reserve().is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_bucket_or_a_removed_account_is_refused() {
    let server = serve(vec![TestObject::new("a.bin", pattern(KIB, 17))]).await;
    let s = setup(&server);
    assert_eq!(
        s.accounts
            .check_download_source("local", "other-bucket")
            .unwrap_err()
            .kind,
        ErrorKind::OutdatedPage
    );
    let started = s
        .start(Request {
            bucket: "other-bucket".into(),
            ..s.request("a.bin", &s.dir.path("a.bin"), Existing::Refuse)
        })
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::OutdatedPage));
    s.accounts.remove("local").unwrap();
    let started = s
        .start(s.request("a.bin", &s.dir.path("a.bin"), Existing::Refuse))
        .unwrap();
    let done = s.sink.finished(&started.transfer).await;
    assert_eq!(kind(&done), Some(ErrorKind::UnknownAccount));
    assert!(
        server.operations.lock().unwrap().is_empty(),
        "no request was sent"
    );
    assert_eq!(s.listing(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn quitting_cancels_a_running_download_and_removes_its_temporary_file() {
    let server = serve(vec![
        TestObject::new("stall.bin", pattern(MIB, 18)).with_fault(Fault::StallAfter(128 * KIB)),
    ])
    .await;
    let s = setup(&server);
    let started = s
        .start(s.request("stall.bin", &s.dir.path("stall.bin"), Existing::Refuse))
        .unwrap();
    s.on_disk(128 * KIB as u64).await;
    assert_eq!(s.listing().len(), 1, "the temporary file exists");
    let downloads = Arc::clone(&s.downloads);
    let at = Instant::now();
    // `shutdown` blocks (it runs in the exit handler), so not on this thread.
    tokio::task::spawn_blocking(move || downloads.shutdown(SHUTDOWN_WAIT))
        .await
        .unwrap();
    assert!(at.elapsed() < Duration::from_secs(3), "{:?}", at.elapsed());
    assert_eq!(s.listing(), Vec::<String>::new());
    assert_eq!(
        s.sink.finished(&started.transfer).await.state,
        TransferState::Cancelled
    );
    assert_eq!(
        s.downloads.reserve().err().unwrap().kind,
        ErrorKind::Busy,
        "no new download"
    );
    // Idle: returns at once.
    CloudDownloads::new().shutdown(SHUTDOWN_WAIT);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_stops_sending_fails_as_a_network_error() {
    // The SDK's stalled-stream protection ends a body that delivers nothing
    // for its grace period (5 s), well before the 30 s read timeout here.
    let server = serve(vec![
        TestObject::new("stall.bin", pattern(MIB, 20)).with_fault(Fault::StallAfter(64 * KIB)),
    ])
    .await;
    let s = setup(&server);
    let at = Instant::now();
    let (_, done) = s.download("stall.bin", "stall.bin").await;
    assert_eq!(kind(&done), Some(ErrorKind::Network), "{done:?}");
    assert!(at.elapsed() < Duration::from_secs(15), "{:?}", at.elapsed());
    eprintln!("stalled body failed after {:?}", at.elapsed());
    assert_eq!(s.listing(), Vec::<String>::new());
}

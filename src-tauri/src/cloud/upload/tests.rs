//! Tests of upload batches: the pure rules (selection checks, prefixes,
//! error sorting) and whole batches through [`CloudUploads`] and
//! [`S3Store::upload`] against the in-process S3 server (s3s + s3s-fs, real
//! HTTP, SigV4 and multipart), with faults injected per operation and hooks
//! on each file's [`TransferControl`]. No real provider, no file dialog (the
//! dialog only produces the paths these tests pass to [`begin_upload`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::download::{self, CloudDownloads, DownloadEvent, TransferState};
use super::super::s3::{ClientOptions, Existing};
use super::super::test_server::{Inject, TempDir, TestServer};
use super::*;

const KEY_ID: &str = "upload-batch-key";
const SECRET: &str = "upload-batch-Secret-0123456789";
const BUCKET: &str = "upload-batch-bucket";
const MIB: usize = 1024 * 1024;

fn options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(30),
        max_attempts: 1,
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        stall_timeout: Duration::from_secs(30),
        abort_timeout: Duration::from_secs(5),
    }
}

/// Records every event; batches run on Tauri's runtime.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<UploadEvent>>,
}

impl UploadSink for Recorder {
    fn emit(&self, event: UploadEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn events(&self) -> Vec<UploadEvent> {
        self.events.lock().unwrap().clone()
    }

    fn of(&self, batch: &str) -> Vec<UploadEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.batch == batch)
            .collect()
    }

    /// The file events of `batch` for file `index`.
    fn file_events(&self, batch: &str, index: u32) -> Vec<(u64, UploadFile)> {
        self.of(batch)
            .into_iter()
            .filter_map(|e| match e.body {
                EventBody::File { file, .. } if file.index == index => Some((e.seq, file)),
                _ => None,
            })
            .collect()
    }
}

/// Opens once, wakes every waiter (a test helper for blocking hooks).
#[derive(Default)]
struct Latch {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Latch {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn wait(&self, limit: Duration) -> bool {
        let guard = self.open.lock().unwrap();
        let (guard, _) = self.cv.wait_timeout_while(guard, limit, |o| !*o).unwrap();
        *guard
    }
}

struct Setup {
    server: Arc<TestServer>,
    accounts: Arc<CloudAccounts>,
    keys: Arc<MemoryCredentialStore>,
    uploads: Arc<CloudUploads>,
    sink: Arc<Recorder>,
    dir: TempDir,
}

fn input(endpoint: &str) -> AccountInput {
    AccountInput {
        label: "Local".into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint.into()),
        bucket: BUCKET.into(),
    }
}

async fn setup_with(options: ClientOptions) -> Setup {
    let server = Arc::new(TestServer::start(KEY_ID, SECRET, &[BUCKET]).await);
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        options,
    ));
    let id = accounts
        .add(
            &input(&server.endpoint),
            CredentialInput::new(&server.access_key_id, &server.secret),
        )
        .unwrap()
        .id;
    assert_eq!(id, "local");
    Setup {
        server,
        accounts,
        keys,
        uploads: Arc::new(CloudUploads::new()),
        sink: Arc::new(Recorder::default()),
        dir: TempDir::new("upload-batch"),
    }
}

async fn setup() -> Setup {
    setup_with(options()).await
}

fn target(prefix: &str) -> Target {
    Target {
        account: "local".into(),
        bucket: BUCKET.into(),
        prefix: prefix.into(),
    }
}

impl Setup {
    /// `upload_cloud_files` with `picked` standing for the dialog's answer.
    async fn begin(
        &self,
        prefix: &str,
        picked: Option<Vec<PathBuf>>,
    ) -> Result<Option<UploadSelection>, CommandError> {
        let selection = begin_upload(
            &self.uploads,
            Arc::clone(&self.accounts),
            target(prefix),
            async move { picked },
            Arc::clone(&self.sink) as Arc<dyn UploadSink>,
        )
        .await;
        if let Ok(Some(selection)) = &selection {
            self.assert_no_paths(&serde_json::to_string(selection).unwrap());
        }
        selection
    }

    /// Starts a batch that must start; returns its id and selection.
    async fn start(&self, prefix: &str, paths: Vec<PathBuf>) -> (String, UploadSelection) {
        let selection = self.begin(prefix, Some(paths)).await.unwrap().unwrap();
        (selection.batch.clone().expect("a batch started"), selection)
    }

    async fn retry(&self, batch: &str) -> Result<UploadSelection, CommandError> {
        let selection = self
            .uploads
            .retry(
                batch,
                Arc::clone(&self.accounts),
                Arc::clone(&self.sink) as Arc<dyn UploadSink>,
            )
            .await;
        if let Ok(selection) = &selection {
            self.assert_no_paths(&serde_json::to_string(selection).unwrap());
        }
        selection
    }

    /// Waits for `batch`'s final event and checks the rules every batch
    /// keeps: the slot is free when it arrives, it comes once and last, seq
    /// counts 1, 2, 3, …, every file's progress only moves forward, files
    /// run one after another, every file is final, and nothing names a
    /// local path.
    async fn finished(&self, batch: &str) -> BatchSummary {
        let started = Instant::now();
        let summary = loop {
            let found = self.of_finished(batch);
            if let Some(summary) = found {
                // The slot was freed before the event was sent.
                assert!(!self.uploads.busy(), "the slot is still taken");
                break summary;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "{batch} did not finish: {:?}",
                self.sink.of(batch)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        // Nothing may follow the final event.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let events = self.sink.of(batch);
        let finals = events
            .iter()
            .filter(|e| matches!(e.body, EventBody::Finished { .. }))
            .count();
        assert_eq!(finals, 1, "{events:?}");
        assert!(matches!(
            events.last().unwrap().body,
            EventBody::Finished { .. }
        ));
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, i as u64 + 1, "{events:?}");
            self.assert_no_paths(&serde_json::to_string(e).unwrap());
        }
        check_files(&events);
        assert!(
            summary.files.iter().all(|f| f.state.is_final()),
            "{summary:?}"
        );
        assert_eq!(summary.count as usize, summary.files.len());
        if summary.state == BatchState::Completed {
            assert!(
                summary.files.iter().all(|f| matches!(
                    f.state,
                    FileState::Completed | FileState::Skipped | FileState::Failed
                )),
                "{summary:?}"
            );
        }
        summary
    }

    fn of_finished(&self, batch: &str) -> Option<BatchSummary> {
        self.sink.of(batch).into_iter().find_map(|e| match e.body {
            EventBody::Finished { summary } => Some(summary),
            _ => None,
        })
    }

    fn assert_no_paths(&self, json: &str) {
        let dir = self.dir.0.to_string_lossy().into_owned();
        assert!(
            !json.contains(&dir),
            "a local path reached the page: {json}"
        );
        let escaped = serde_json::to_string(&dir).unwrap();
        let escaped = escaped.trim_matches('"');
        assert!(
            !json.contains(escaped),
            "a local path reached the page: {json}"
        );
    }

    fn object(&self, key: &str) -> Option<Vec<u8>> {
        fs::read(self.server.object_path(BUCKET, key)).ok()
    }

    fn count_ops(&self, op: &str) -> usize {
        self.server.ops().iter().filter(|o| *o == op).count()
    }

    /// Stores `data` at `key` directly, as another client would.
    async fn put(&self, key: &str, data: &[u8]) {
        let path = self.dir.path(&format!("theirs-{}", key.replace('/', "_")));
        fs::write(&path, data).unwrap();
        let store = self.accounts.prepare_upload("local", BUCKET).unwrap();
        store.upload_file(BUCKET, key, &path).await.unwrap();
        fs::remove_file(path).unwrap();
    }

    fn weak(&self) -> Weak<CloudUploads> {
        Arc::downgrade(&self.uploads)
    }
}

/// Every file's events: states and stages only move forward, bytes never
/// decrease within a stage, and a file's events all come before the next
/// file's first event (one file at a time).
fn check_files(events: &[UploadEvent]) {
    let mut last: HashMap<u32, (FileState, Option<Stage>, u64)> = HashMap::new();
    let mut finished: Vec<u32> = Vec::new();
    let mut current: Option<u32> = None;
    for e in events {
        let EventBody::File { file, .. } = &e.body else {
            continue;
        };
        if let Some((state, stage, bytes)) = last.get(&file.index) {
            assert!(
                !state.is_final(),
                "an event after the final one: {events:?}"
            );
            assert!(file.state >= *state, "{events:?}");
            if file.stage == *stage && !file.state.is_final() {
                assert!(file.bytes > *bytes, "{events:?}");
            }
            assert!(file.stage >= *stage, "{events:?}");
        }
        assert!(!finished.contains(&file.index), "{events:?}");
        if current != Some(file.index) {
            if let Some(previous) = current {
                assert!(
                    last.get(&previous).is_some_and(|(s, _, _)| s.is_final()),
                    "file {} started before file {previous} ended: {events:?}",
                    file.index
                );
                finished.push(previous);
            }
            current = Some(file.index);
        }
        last.insert(file.index, (file.state, file.stage, file.bytes));
    }
}

fn file(summary: &BatchSummary, index: u32) -> &UploadFile {
    &summary.files[index as usize]
}

// ---- pure rules ----

fn limits() -> TransferLimits {
    TransferLimits {
        min_part_size: 5 * MIB as u64,
        max_part_size: 5 * MIB as u64,
        max_parts: 10,
        max_single_put: 5 * MIB as u64,
        max_object_size: 50 * MIB as u64,
    }
}

fn reasons(candidates: &[Candidate]) -> Vec<Option<SkipReason>> {
    candidates
        .iter()
        .map(|c| c.skip.as_ref().map(|s| s.reason))
        .collect()
}

#[test]
fn a_selection_is_checked_file_by_file_without_failing_as_a_whole() {
    let dir = TempDir::new("check");
    let a = dir.file("a.txt", 10, 1);
    fs::create_dir_all(dir.path("folder")).unwrap();
    let missing = dir.path("missing.bin");
    let checked = check_selection(
        "photos/",
        vec![a.clone(), dir.path("folder"), missing],
        &limits(),
        &options(),
    );
    assert_eq!(
        reasons(&checked),
        vec![
            None,
            Some(SkipReason::NotAFile),
            Some(SkipReason::Unreadable)
        ]
    );
    assert_eq!(checked[0].key.as_deref(), Some("photos/a.txt"));
    assert_eq!(checked[0].size, Some(10));
    assert_eq!(checked[0].name, "a.txt");
    assert_eq!(checked[0].path, a);
    // Unicode names are fine; the key is folder + name, unchanged. Windows
    // refuses `?` in a file name, so only there the name goes without it.
    let name = if cfg!(windows) {
        "größe test+&#%.bin"
    } else {
        "größe test+&?#%.bin"
    };
    let b = dir.file(name, 0, 2);
    let checked = check_selection("", vec![b], &limits(), &options());
    assert_eq!(reasons(&checked), vec![None]);
    assert_eq!(checked[0].key.as_deref(), Some(name));
}

#[test]
fn names_with_control_characters_are_skipped() {
    // The name is checked before the file system is asked.
    let dir = TempDir::new("control");
    for name in [
        "tab\there.txt",
        "line\nbreak.txt",
        "del\u{7f}.txt",
        "c1\u{85}.txt",
    ] {
        let checked = check_selection("", vec![dir.path(name)], &limits(), &options());
        assert_eq!(
            reasons(&checked),
            vec![Some(SkipReason::ControlCharacters)],
            "{name:?}"
        );
        assert_eq!(checked[0].key, None);
    }
}

#[cfg(unix)]
#[test]
fn names_that_are_not_utf8_are_skipped() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new("utf8");
    let path = dir.0.join(std::ffi::OsStr::from_bytes(b"bad-\xff.bin"));
    fs::write(&path, b"x").unwrap();
    let checked = check_selection("", vec![path], &limits(), &options());
    assert_eq!(reasons(&checked), vec![Some(SkipReason::InvalidName)]);
    assert_eq!(checked[0].key, None);
    assert!(
        checked[0].name.contains('\u{fffd}'),
        "{:?}",
        checked[0].name
    );
}

#[cfg(windows)]
#[test]
fn names_that_are_not_utf8_are_skipped() {
    use std::os::windows::ffi::OsStringExt;
    let dir = TempDir::new("utf8");
    // An unpaired surrogate: a valid Windows name, not valid Unicode.
    let name = std::ffi::OsString::from_wide(&[0x62, 0xD800, 0x2E, 0x62]);
    let checked = check_selection("", vec![dir.0.join(name)], &limits(), &options());
    assert_eq!(reasons(&checked), vec![Some(SkipReason::InvalidName)]);
}

#[test]
fn keys_longer_than_1024_bytes_are_skipped() {
    let dir = TempDir::new("long");
    let a = dir.file("abcd.txt", 1, 1);
    let prefix = format!("{}/", "p".repeat(1015)); // 1016 bytes
    let checked = check_selection(&prefix, vec![a.clone()], &limits(), &options());
    assert_eq!(reasons(&checked), vec![None], "1016 + 8 = 1024 bytes");
    let prefix = format!("{}/", "p".repeat(1016));
    let checked = check_selection(&prefix, vec![a], &limits(), &options());
    assert_eq!(reasons(&checked), vec![Some(SkipReason::KeyTooLong)]);
    assert_eq!(checked[0].key, None);
}

#[test]
fn files_beyond_the_provider_limits_are_skipped() {
    let dir = TempDir::new("large");
    // Fits: 10 parts of 5 MiB. Too large: more than 10 parts.
    let fits = dir.path("fits.bin");
    let large = dir.path("large.bin");
    fs::File::create(&fits)
        .unwrap()
        .set_len(50 * MIB as u64)
        .unwrap();
    fs::File::create(&large)
        .unwrap()
        .set_len(50 * MIB as u64 + 1)
        .unwrap();
    let checked = check_selection("", vec![fits, large], &limits(), &options());
    assert_eq!(reasons(&checked), vec![None, Some(SkipReason::TooLarge)]);
    let message = &checked[1].skip.as_ref().unwrap().message;
    assert!(message.contains(&(50 * MIB).to_string()), "{message}");
}

#[test]
fn files_with_the_same_destination_key_are_all_skipped() {
    let dir = TempDir::new("dup");
    fs::create_dir_all(dir.path("one")).unwrap();
    fs::create_dir_all(dir.path("two")).unwrap();
    let a1 = dir.path("one/a.txt");
    let a2 = dir.path("two/a.txt");
    fs::write(&a1, b"1").unwrap();
    fs::write(&a2, b"2").unwrap();
    // Keys are case-sensitive: "A.txt" is another key.
    let upper = dir.file("one/A.txt", 1, 3);
    let b = dir.file("b.txt", 1, 4);
    let checked = check_selection("f/", vec![a1, b, a2, upper], &limits(), &options());
    assert_eq!(
        reasons(&checked),
        vec![
            Some(SkipReason::DuplicateKey),
            None,
            Some(SkipReason::DuplicateKey),
            None
        ]
    );
}

#[test]
fn only_folders_a_listing_returns_are_destinations() {
    for ok in ["", "a/", "a/b/", "a//b/", "größe/"] {
        assert!(validate_prefix(ok).is_ok(), "{ok}");
    }
    let long = format!("{}/", "p".repeat(1023));
    for bad in ["a", "/a", "a/b", long.as_str()] {
        assert_eq!(
            validate_prefix(bad).unwrap_err().kind,
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
}

#[test]
fn errors_are_sorted_by_their_effect_on_the_batch() {
    let stop = [
        StorageError::InvalidCredentials("x".into()),
        StorageError::AccessDenied("x".into()),
        StorageError::NotFound("x".into()),
        StorageError::Network("x".into()),
        StorageError::Timeout("x".into()),
        StorageError::Refused("x".into()),
        StorageError::Provider {
            status: 503,
            code: "SlowDown".into(),
            message: "x".into(),
        },
        StorageError::Provider {
            status: 429,
            code: String::new(),
            message: "x".into(),
        },
    ];
    for e in &stop {
        assert_eq!(disposition(e), Disposition::StopBatch, "{e:?}");
    }
    let per_file = [
        StorageError::Integrity("x".into()),
        StorageError::FileChanged("/home/me/secret/a.txt".into()),
        StorageError::Io("/home/me/secret/a.txt: permission denied".into()),
        StorageError::TooLarge { size: 2, max: 1 },
        StorageError::Provider {
            status: 400,
            code: "EntityTooSmall".into(),
            message: "x".into(),
        },
    ];
    for e in &per_file {
        assert_eq!(disposition(e), Disposition::FileFailed, "{e:?}");
        // Local error text names the path: never shown.
        assert!(!upload_error(e).message.contains("/home/me"), "{e:?}");
    }
    assert_eq!(
        disposition(&StorageError::AlreadyExists("k".into())),
        Disposition::Skip
    );
    assert_eq!(
        disposition(&StorageError::Cancelled),
        Disposition::Cancelled
    );
    assert_eq!(
        upload_error(&StorageError::FileChanged("p".into())).kind,
        ErrorKind::FileChanged
    );
    assert_eq!(
        upload_error(&StorageError::AccessDenied("their text".into())),
        CommandError::new(
            ErrorKind::AccessDenied,
            "the provider refused to store files with this key"
        )
    );
    let provider = upload_error(&StorageError::Provider {
        status: 400,
        code: "Bad<script>".into(),
        message: "their text".into(),
    });
    assert_eq!(
        provider.message,
        "the provider refused the upload: HTTP 400"
    );
}

#[test]
fn events_serialize_in_the_shape_the_page_reads() {
    let event = UploadEvent {
        batch: "upload-7".into(),
        seq: 3,
        body: EventBody::File {
            count: 2,
            file: UploadFile {
                index: 1,
                name: "a.txt".into(),
                key: Some("f/a.txt".into()),
                size: Some(5),
                state: FileState::Sending,
                stage: Some(Stage::Sending),
                bytes: 4,
                total: 5,
                started: true,
                skip: None,
                error: None,
                remote: None,
                retryable: false,
            },
        },
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["kind"], "file");
    assert_eq!(json["batch"], "upload-7");
    assert_eq!(json["seq"], 3);
    assert_eq!(json["count"], 2);
    assert_eq!(json["file"]["state"], "sending");
    assert_eq!(json["file"]["stage"], "sending");
    let cleanup = serde_json::to_value(UploadEvent {
        batch: "upload-8".into(),
        seq: 1,
        body: EventBody::Cleanup {
            key: "k".into(),
            outcome: CleanupOutcome::Removed,
        },
    })
    .unwrap();
    assert_eq!(cleanup["kind"], "cleanup");
    assert_eq!(cleanup["outcome"], "removed");
    let skip = serde_json::to_value(Skip::new(SkipReason::DestinationExists, "m")).unwrap();
    assert_eq!(skip["reason"], "destination-exists");
    assert_eq!(
        serde_json::to_value(RemoteState::NeedsInspection).unwrap(),
        "needs-inspection"
    );
    let summary = serde_json::to_value(BatchSummary {
        state: BatchState::Cancelled,
        count: 0,
        completed: 0,
        skipped: 0,
        failed: 0,
        cancelled: 0,
        not_started: 0,
        retryable: 0,
        needs_attention: 0,
        error: None,
        files: Vec::new(),
    })
    .unwrap();
    assert_eq!(summary["state"], "cancelled");
    assert!(summary.get("notStarted").is_some());
    assert!(summary.get("needsAttention").is_some());
}

// ---- starting a batch ----

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_or_empty_file_dialog_starts_nothing() {
    let s = setup().await;
    assert_eq!(s.begin("", None).await, Ok(None));
    assert_eq!(s.begin("", Some(Vec::new())).await, Ok(None));
    assert!(!s.uploads.busy());
    assert!(s.sink.events().is_empty());
    assert!(s.server.ops().is_empty(), "{:?}", s.server.ops());
    // The slot is free again.
    drop(s.uploads.reserve().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn more_than_100_files_are_refused_and_100_are_uploaded() {
    let s = setup().await;
    let paths: Vec<PathBuf> = (0..=MAX_FILES)
        .map(|i| s.dir.file(&format!("f{i:03}.txt"), i % 7, i as u8))
        .collect();
    let e = s.begin("many/", Some(paths.clone())).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(
        e.message.contains("101") && e.message.contains("100"),
        "{e:?}"
    );
    assert!(!s.uploads.busy());
    assert!(s.sink.events().is_empty());
    assert!(s.server.ops().is_empty(), "nothing was sent");

    let (batch, selection) = s.start("many/", paths[..MAX_FILES].to_vec()).await;
    assert_eq!(selection.files.len(), MAX_FILES);
    assert_eq!(selection.prefix, "many/");
    assert!(
        selection
            .files
            .iter()
            .all(|f| f.state == FileState::Pending)
    );
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert_eq!(summary.completed as usize, MAX_FILES);
    assert_eq!(s.count_ops("PutObject"), MAX_FILES);
    for i in [0usize, 57, 99] {
        assert_eq!(
            s.object(&format!("many/f{i:03}.txt")).unwrap(),
            fs::read(&paths[i]).unwrap()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_valid_file_no_batch_starts() {
    let s = setup().await;
    fs::create_dir_all(s.dir.path("folder")).unwrap();
    let selection = s
        .begin("", Some(vec![s.dir.path("folder"), s.dir.path("missing")]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selection.batch, None);
    assert_eq!(
        selection
            .files
            .iter()
            .map(|f| (f.state, f.skip.as_ref().map(|s| s.reason)))
            .collect::<Vec<_>>(),
        vec![
            (FileState::Skipped, Some(SkipReason::NotAFile)),
            (FileState::Skipped, Some(SkipReason::Unreadable)),
        ]
    );
    assert!(!s.uploads.busy());
    assert!(s.sink.events().is_empty());
    assert!(s.server.ops().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_files_are_skipped_and_the_others_uploaded() {
    let s = setup().await;
    fs::create_dir_all(s.dir.path("one")).unwrap();
    let a = s.dir.file("a.txt", 100, 1);
    let dup1 = s.dir.file("d.txt", 10, 2);
    let dup2 = s.dir.file("one/d.txt", 10, 3);
    let control = s.dir.path("bad\u{1}.txt");
    let (batch, selection) = s.start("in/", vec![dup1, a.clone(), control, dup2]).await;
    assert_eq!(
        selection
            .files
            .iter()
            .map(|f| f.skip.as_ref().map(|s| s.reason))
            .collect::<Vec<_>>(),
        vec![
            Some(SkipReason::DuplicateKey),
            None,
            Some(SkipReason::ControlCharacters),
            Some(SkipReason::DuplicateKey)
        ]
    );
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert_eq!((summary.completed, summary.skipped), (1, 3));
    assert_eq!(s.object("in/a.txt").unwrap(), fs::read(&a).unwrap());
    assert_eq!(s.object("in/d.txt"), None);
    // Skipped files get no events of their own.
    for i in [0, 2, 3] {
        assert!(s.sink.file_events(&batch, i).is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_object_is_skipped_before_the_file_is_read() {
    let s = setup().await;
    s.put("dest/a.txt", b"theirs").await;
    let a = s.dir.file("a.txt", 3 * MIB, 1);
    let b = s.dir.file("b.txt", 1000, 2);
    let heads_before = s.count_ops("HeadObject");
    let (batch, _) = s.start("dest/", vec![a, b.clone()]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    let skipped = file(&summary, 0);
    assert_eq!(skipped.state, FileState::Skipped);
    assert_eq!(
        skipped.skip.as_ref().unwrap().reason,
        SkipReason::DestinationExists
    );
    assert!(!skipped.retryable);
    assert_eq!(skipped.stage, None, "never hashed");
    // Only the final event for the skipped file: no hashing progress.
    let events = s.sink.file_events(&batch, 0);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(s.object("dest/a.txt").unwrap(), b"theirs");
    assert_eq!(file(&summary, 1).state, FileState::Completed);
    assert_eq!(s.object("dest/b.txt").unwrap(), fs::read(&b).unwrap());
    // a: one HEAD; b: our HEAD, upload's HEAD, verification.
    assert_eq!(s.count_ops("HeadObject") - heads_before, 4);
    assert_eq!(s.count_ops("PutObject"), 2, "theirs + b");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflict_found_by_the_upload_itself_skips_the_file() {
    // Another client creates the object after our check, while the file is
    // being hashed: the upload's own check finds it before sending.
    let s = setup().await;
    let a = s.dir.file("race.bin", 2 * MIB, 1);
    let theirs = s.dir.path("theirs.bin");
    fs::write(&theirs, b"theirs").unwrap();
    let store = s.accounts.prepare_upload("local", BUCKET).unwrap();
    let handle = tokio::runtime::Handle::current();
    s.uploads.file_control(move |_, control| {
        let (store, theirs, handle) = (Arc::clone(&store), theirs.clone(), handle.clone());
        control.during_hashing(move || {
            std::thread::spawn(move || {
                handle
                    .block_on(store.upload_file(BUCKET, "race.bin", &theirs))
                    .unwrap();
            })
            .join()
            .unwrap();
        });
    });
    let (batch, _) = s.start("", vec![a]).await;
    let summary = s.finished(&batch).await;
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Skipped, "{f:?}");
    assert_eq!(
        f.skip.as_ref().unwrap().reason,
        SkipReason::DestinationExists
    );
    assert_eq!(s.object("race.bin").unwrap(), b"theirs");
}

// ---- one at a time ----

#[tokio::test(flavor = "multi_thread")]
async fn files_upload_one_after_another_and_batches_one_at_a_time() {
    let s = setup().await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    let small = s.dir.file("small.txt", 1000, 2);
    let empty = s.dir.file("empty.txt", 0, 3);
    let running = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Latch::default());
    let entered = Arc::new(Latch::default());
    {
        let (running, most, gate, entered) = (
            Arc::clone(&running),
            Arc::clone(&most),
            Arc::clone(&gate),
            Arc::clone(&entered),
        );
        s.uploads.file_control(move |index, control| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            let running = Arc::clone(&running);
            control.before_commit(move || {
                running.fetch_sub(1, Ordering::SeqCst);
            });
            if index == 0 {
                let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
                control.during_send(2, move || {
                    entered.open();
                    assert!(gate.wait(Duration::from_secs(20)));
                });
            }
        });
    }
    let (batch, _) = s
        .start("seq/", vec![big.clone(), small.clone(), empty])
        .await;
    assert!(entered.wait(Duration::from_secs(20)));
    // While the first file is sending: no second batch, no second dialog.
    assert_eq!(
        s.begin("other/", Some(vec![small.clone()]))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Busy
    );
    assert_eq!(s.uploads.reserve().err().unwrap().kind, ErrorKind::Busy);
    assert!(s.sink.file_events(&batch, 1).is_empty(), "file 1 waits");
    gate.open();
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert_eq!(summary.completed, 3);
    assert_eq!(most.load(Ordering::SeqCst), 1, "never two files at once");
    assert_eq!(s.object("seq/big.bin").unwrap(), fs::read(&big).unwrap());
    assert_eq!(s.object("seq/empty.txt").unwrap(), b"");
    // The operations of the files never interleave.
    let ops = s.server.ops();
    let complete = ops
        .iter()
        .position(|o| o == "CompleteMultipartUpload")
        .unwrap();
    let first_put = ops.iter().position(|o| o == "PutObject").unwrap();
    assert!(complete < first_put, "{ops:?}");
    // The next batch may start as soon as the final event arrived.
    let (next, _) = s.start("again/", vec![small]).await;
    assert_ne!(next, batch);
    assert_eq!(s.finished(&next).await.completed, 1);
}

#[derive(Default)]
struct DownloadRecorder {
    events: Mutex<Vec<DownloadEvent>>,
}

impl download::EventSink for DownloadRecorder {
    fn emit(&self, event: DownloadEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_runs_while_a_batch_is_uploading() {
    let s = setup().await;
    let original = s.dir.file("original.bin", 3 * MIB, 9);
    s.accounts
        .prepare_upload("local", BUCKET)
        .unwrap()
        .upload_file(BUCKET, "existing/original.bin", &original)
        .await
        .unwrap();
    let gate = Arc::new(Latch::default());
    let entered = Arc::new(Latch::default());
    {
        let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
        s.uploads.file_control(move |_, control| {
            let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
            control.during_send(1, move || {
                entered.open();
                assert!(gate.wait(Duration::from_secs(20)));
            });
        });
    }
    let up = s.dir.file("up.bin", 2 * MIB, 4);
    let (batch, _) = s.start("", vec![up.clone()]).await;
    assert!(entered.wait(Duration::from_secs(20)));
    assert!(s.uploads.busy());

    let downloads = Arc::new(CloudDownloads::new());
    let sink = Arc::new(DownloadRecorder::default());
    let dest = s.dir.path("downloaded.bin");
    let started = downloads.start(
        downloads.reserve().unwrap(),
        Arc::clone(&s.accounts),
        download::Request {
            account: "local".into(),
            bucket: BUCKET.into(),
            key: "existing/original.bin".into(),
            dest: dest.clone(),
            existing: Existing::Refuse,
        },
        Arc::clone(&sink) as Arc<dyn download::EventSink>,
    );
    let at = Instant::now();
    let done = loop {
        let done = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.transfer == started.transfer && e.state != TransferState::Active)
            .cloned();
        if let Some(done) = done {
            break done;
        }
        assert!(at.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&original).unwrap());
    assert!(s.uploads.busy(), "the upload is still running");
    gate.open();
    let summary = s.finished(&batch).await;
    assert_eq!(summary.completed, 1);
    assert_eq!(s.object("up.bin").unwrap(), fs::read(&up).unwrap());
}

// ---- cancelling ----

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_hashing_stops_the_batch_before_anything_is_sent() {
    let s = setup().await;
    let a = s.dir.file("a.bin", 4 * MIB, 1);
    let b = s.dir.file("b.txt", 10, 2);
    let c = s.dir.file("c.txt", 10, 3);
    let weak = s.weak();
    s.uploads.file_control(move |index, control| {
        if index == 0 {
            let weak = weak.clone();
            control.during_hashing(move || {
                assert_eq!(
                    weak.upgrade().unwrap().cancel_active(),
                    CancelOutcome::Cancelling
                );
            });
        }
    });
    let (batch, _) = s.start("", vec![a, b, c]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Cancelled);
    assert_eq!(f.stage, Some(Stage::Hashing));
    assert!(f.started && f.retryable);
    assert_eq!(f.remote, Some(RemoteState::NothingStored));
    for i in [1, 2] {
        let f = file(&summary, i);
        assert_eq!(f.state, FileState::Cancelled);
        assert!(!f.started && f.retryable);
        assert!(s.sink.file_events(&batch, i).is_empty(), "never started");
    }
    assert_eq!(
        (summary.cancelled, summary.not_started, summary.retryable),
        (3, 2, 3)
    );
    assert_eq!(s.count_ops("PutObject"), 0);
    assert_eq!(s.object("a.bin"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_single_put_stores_nothing() {
    let s = setup().await;
    let a = s.dir.file("a.bin", 3 * MIB, 1);
    let b = s.dir.file("b.txt", 10, 2);
    let weak = s.weak();
    s.uploads.file_control(move |index, control| {
        if index == 0 {
            let weak = weak.clone();
            control.during_send(1, move || {
                weak.upgrade().unwrap().cancel_active();
            });
        }
    });
    let (batch, _) = s.start("", vec![a, b]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    let f = file(&summary, 0);
    assert_eq!(
        (f.state, f.stage),
        (FileState::Cancelled, Some(Stage::Sending))
    );
    assert_eq!(f.remote, Some(RemoteState::NothingStored));
    assert!(!file(&summary, 1).started);
    assert_eq!(s.object("a.bin"), None);
    assert_eq!(s.object("b.txt"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_multipart_upload_aborts_it() {
    let s = setup().await;
    let a = s.dir.file("a.bin", 11 * MIB, 1);
    let weak = s.weak();
    s.uploads.file_control(move |_, control| {
        let weak = weak.clone();
        control.during_send(2, move || {
            weak.upgrade().unwrap().cancel_active();
        });
    });
    let (batch, _) = s.start("", vec![a]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Cancelled);
    assert_eq!(f.remote, Some(RemoteState::NothingStored));
    assert!(f.retryable);
    assert_eq!(s.count_ops("AbortMultipartUpload"), 1);
    assert_eq!(s.count_ops("CompleteMultipartUpload"), 0);
    assert_eq!(s.server.open_multipart_files(), Vec::<String>::new());
    assert!(s.uploads.unresolved().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_between_files_keeps_the_rest_from_starting() {
    let s = setup().await;
    let paths: Vec<PathBuf> = (0..3)
        .map(|i| s.dir.file(&format!("{i}.txt"), 100, i as u8))
        .collect();
    let weak = s.weak();
    s.uploads.before_file(move |index| {
        if index == 1 {
            weak.upgrade().unwrap().cancel_active();
        }
    });
    let (batch, _) = s.start("", paths).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    assert_eq!(file(&summary, 0).state, FileState::Completed);
    assert!(!file(&summary, 0).retryable);
    for i in [1, 2] {
        assert_eq!(file(&summary, i).state, FileState::Cancelled);
        assert!(!file(&summary, i).started);
    }
    assert_eq!(s.count_ops("PutObject"), 1);
    assert_eq!(summary.not_started, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_is_idempotent_and_only_for_its_own_batch() {
    let s = setup().await;
    let gate = Arc::new(Latch::default());
    let entered = Arc::new(Latch::default());
    {
        let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
        s.uploads.file_control(move |_, control| {
            let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
            control.during_send(1, move || {
                entered.open();
                gate.wait(Duration::from_secs(20));
            });
        });
    }
    let a = s.dir.file("a.bin", MIB, 1);
    let (batch, _) = s.start("", vec![a]).await;
    assert!(entered.wait(Duration::from_secs(20)));
    assert_eq!(s.uploads.cancel("upload-999"), CancelOutcome::NotActive);
    assert!(s.uploads.busy());
    assert_eq!(s.uploads.cancel(&batch), CancelOutcome::Cancelling);
    assert_eq!(s.uploads.cancel(&batch), CancelOutcome::Cancelling);
    gate.open();
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    assert_eq!(s.uploads.cancel(&batch), CancelOutcome::NotActive);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_after_the_commit_point_lets_the_file_finish_but_no_other_start() {
    let s = setup().await;
    let a = s.dir.file("a.txt", 1000, 1);
    let b = s.dir.file("b.txt", 1000, 2);
    let weak = s.weak();
    let outcome = Arc::new(Mutex::new(None));
    {
        let outcome = Arc::clone(&outcome);
        s.uploads.file_control(move |index, control| {
            if index == 0 {
                let (weak, outcome) = (weak.clone(), Arc::clone(&outcome));
                control.after_commit(move || {
                    *outcome.lock().unwrap() = Some(weak.upgrade().unwrap().cancel_active());
                });
            }
        });
    }
    let (batch, _) = s.start("", vec![a.clone(), b]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(*outcome.lock().unwrap(), Some(CancelOutcome::Finishing));
    assert_eq!(file(&summary, 0).state, FileState::Completed);
    assert_eq!(file(&summary, 1).state, FileState::Cancelled);
    assert!(!file(&summary, 1).started);
    assert_eq!(summary.state, BatchState::Cancelled);
    assert_eq!(s.object("a.txt").unwrap(), fs::read(&a).unwrap());
    assert_eq!(s.object("b.txt"), None);
}

// ---- errors ----

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_of_one_file_lets_the_batch_go_on() {
    let s = setup().await;
    let changing = s.dir.file("changing.bin", 3 * MIB, 1);
    let ok = s.dir.file("ok.txt", 500, 2);
    {
        let changing = changing.clone();
        s.uploads.file_control(move |index, control| {
            if index == 0 {
                let changing = changing.clone();
                control.during_hashing(move || {
                    let mut f = fs::OpenOptions::new().append(true).open(&changing).unwrap();
                    std::io::Write::write_all(&mut f, b"more").unwrap();
                });
            }
        });
    }
    let (batch, _) = s.start("", vec![changing, ok.clone()]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Failed);
    assert_eq!(f.error.as_ref().unwrap().kind, ErrorKind::FileChanged);
    assert_eq!(f.remote, Some(RemoteState::NothingStored));
    assert!(f.retryable);
    assert_eq!(file(&summary, 1).state, FileState::Completed);
    assert_eq!(s.object("ok.txt").unwrap(), fs::read(&ok).unwrap());
    assert_eq!((summary.failed, summary.retryable), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn access_denied_stops_the_batch_and_a_retry_rechecks_destinations() {
    let s = setup().await;
    let paths: Vec<PathBuf> = (0..3)
        .map(|i| s.dir.file(&format!("{i}.txt"), 100 + i, i as u8))
        .collect();
    // The destination check is refused: the key may not use the bucket.
    s.server.inject("HeadObject", None, 1, Inject::Deny);
    let (batch, _) = s.start("r/", paths.clone()).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Failed);
    assert_eq!(
        summary.error.as_ref().unwrap().kind,
        ErrorKind::AccessDenied
    );
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Failed);
    assert_eq!(f.error.as_ref().unwrap().kind, ErrorKind::AccessDenied);
    assert!(f.retryable);
    for i in [1, 2] {
        assert_eq!(file(&summary, i).state, FileState::Cancelled);
        assert!(!file(&summary, i).started && file(&summary, i).retryable);
    }
    assert_eq!(s.count_ops("PutObject"), 0, "the batch stopped");

    // Meanwhile someone else stores file 1's key.
    s.put("r/1.txt", b"theirs").await;
    let retried = s.retry(&batch).await.unwrap();
    let next = retried.batch.clone().unwrap();
    assert_ne!(next, batch);
    assert_eq!(retried.prefix, "r/");
    assert_eq!(retried.files.len(), 3);
    let summary = s.finished(&next).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert_eq!(file(&summary, 0).state, FileState::Completed);
    assert_eq!(
        file(&summary, 1).skip.as_ref().unwrap().reason,
        SkipReason::DestinationExists
    );
    assert_eq!(file(&summary, 2).state, FileState::Completed);
    assert_eq!(s.object("r/1.txt").unwrap(), b"theirs");
    assert_eq!(s.object("r/0.txt").unwrap(), fs::read(&paths[0]).unwrap());
    // Nothing left to retry.
    assert_eq!(
        s.retry(&next).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_uploads_only_the_eligible_files() {
    let s = setup().await;
    s.put("e/exists.txt", b"theirs").await;
    let done = s.dir.file("done.txt", 10, 1);
    let exists = s.dir.file("exists.txt", 10, 2);
    let changing = s.dir.file("changing.bin", 2 * MIB, 3);
    let never = s.dir.file("never.txt", 10, 4);
    {
        let changing = changing.clone();
        s.uploads.file_control(move |index, control| {
            if index == 2 {
                let changing = changing.clone();
                control.during_hashing(move || {
                    let mut f = fs::OpenOptions::new().append(true).open(&changing).unwrap();
                    std::io::Write::write_all(&mut f, b"more").unwrap();
                });
            }
        });
    }
    let weak = s.weak();
    s.uploads.before_file(move |index| {
        if index == 3 {
            weak.upgrade().unwrap().cancel_active();
        }
    });
    let (batch, _) = s
        .start("e/", vec![done, exists, changing.clone(), never])
        .await;
    let summary = s.finished(&batch).await;
    assert_eq!(
        summary.files.iter().map(|f| f.state).collect::<Vec<_>>(),
        vec![
            FileState::Completed,
            FileState::Skipped,
            FileState::Failed,
            FileState::Cancelled
        ]
    );
    assert_eq!(
        summary
            .files
            .iter()
            .map(|f| f.retryable)
            .collect::<Vec<_>>(),
        vec![false, false, true, true]
    );
    *s.uploads.file_control.lock().unwrap() = None;
    *s.uploads.before_file.lock().unwrap() = None;
    let puts = s.count_ops("PutObject");
    let retried = s.retry(&batch).await.unwrap();
    assert_eq!(
        retried
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["changing.bin", "never.txt"]
    );
    let summary = s.finished(retried.batch.as_ref().unwrap()).await;
    assert_eq!(summary.completed, 2);
    assert_eq!(s.count_ops("PutObject") - puts, 2);
    assert_eq!(
        s.object("e/changing.bin").unwrap(),
        fs::read(&changing).unwrap()
    );
    assert_eq!(s.object("e/exists.txt").unwrap(), b"theirs");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_result_that_needs_inspection_is_shown_and_never_retried() {
    let s = setup().await;
    let a = s.dir.file("a.txt", 1000, 1);
    let server = Arc::clone(&s.server);
    s.uploads.file_control(move |_, control| {
        let server = Arc::clone(&server);
        // After the commit point the verification HEAD is refused.
        control.after_commit(move || server.inject("HeadObject", None, 1, Inject::Deny));
    });
    let (batch, _) = s.start("", vec![a]).await;
    let summary = s.finished(&batch).await;
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Failed);
    assert_eq!(f.remote, Some(RemoteState::NeedsInspection));
    assert!(!f.retryable);
    assert_eq!(summary.needs_attention, 1);
    // The object may well be there; it is not deleted.
    assert!(s.object("a.txt").is_some());
    assert_eq!(s.count_ops("DeleteObject"), 0);
    assert_eq!(
        s.retry(&batch).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn incomplete_parts_are_kept_cleaned_up_on_retry_and_never_retried() {
    let s = setup().await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    let small = s.dir.file("small.txt", 10, 2);
    s.server
        .inject("AbortMultipartUpload", None, 1, Inject::Deny);
    let weak = s.weak();
    s.uploads.file_control(move |index, control| {
        if index == 0 {
            let weak = weak.clone();
            control.during_send(2, move || {
                weak.upgrade().unwrap().cancel_active();
            });
        }
    });
    let (batch, _) = s.start("", vec![big, small]).await;
    let summary = s.finished(&batch).await;
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Cancelled);
    assert_eq!(f.remote, Some(RemoteState::IncompleteParts));
    assert!(!f.retryable);
    assert_eq!(summary.needs_attention, 1);
    assert_eq!(s.uploads.unresolved().len(), 1);
    assert_eq!(s.uploads.unresolved()[0].0, "big.bin");
    assert!(!s.server.open_multipart_files().is_empty(), "parts remain");

    *s.uploads.file_control.lock().unwrap() = None;
    let retried = s.retry(&batch).await.unwrap();
    assert_eq!(
        retried
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["small.txt"],
        "the incomplete file is not retried"
    );
    let next = retried.batch.unwrap();
    let summary = s.finished(&next).await;
    let cleanup: Vec<_> = s
        .sink
        .of(&next)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::Cleanup { key, outcome } => Some((e.seq, key, outcome)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cleanup,
        vec![(1, "big.bin".into(), CleanupOutcome::Removed)]
    );
    assert!(s.uploads.unresolved().is_empty());
    assert_eq!(s.server.open_multipart_files(), Vec::<String>::new());
    assert_eq!(summary.completed, 1);
    assert_eq!(s.object("big.bin"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cleanup_that_fails_again_keeps_the_upload_id() {
    let s = setup().await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    s.server
        .inject("AbortMultipartUpload", None, 2, Inject::Deny);
    let weak = s.weak();
    s.uploads.file_control(move |_, control| {
        let weak = weak.clone();
        control.during_send(2, move || {
            weak.upgrade().unwrap().cancel_active();
        });
    });
    let (batch, _) = s.start("", vec![big]).await;
    s.finished(&batch).await;
    let retried = s.retry(&batch).await.unwrap();
    assert!(retried.files.is_empty());
    let next = retried.batch.unwrap();
    let summary = s.finished(&next).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert!(s.sink.of(&next).iter().any(|e| matches!(
        e.body,
        EventBody::Cleanup {
            outcome: CleanupOutcome::Failed,
            ..
        }
    )));
    assert_eq!(s.uploads.unresolved().len(), 1, "kept for the next attempt");
    // Third attempt (the injection is used up): removed.
    let third = s.retry(&next).await.unwrap().batch.unwrap();
    s.finished(&third).await;
    assert!(s.uploads.unresolved().is_empty());
    assert_eq!(s.server.open_multipart_files(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_credentials_fail_the_batch_before_any_file() {
    let s = setup().await;
    s.keys.delete("local").unwrap();
    let a = s.dir.file("a.txt", 10, 1);
    let (batch, _) = s.start("", vec![a]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(summary.state, BatchState::Failed);
    assert_eq!(
        summary.error.as_ref().unwrap().kind,
        ErrorKind::CredentialsMissing
    );
    let f = file(&summary, 0);
    assert_eq!(f.state, FileState::Cancelled);
    assert!(!f.started && f.retryable);
    assert!(s.server.ops().is_empty());
    // Once the key is back, the retry uploads it.
    s.keys.store("local", &s.server.credentials()).unwrap();
    let next = s.retry(&batch).await.unwrap().batch.unwrap();
    assert_eq!(s.finished(&next).await.completed, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_batch_id_changes_nothing() {
    let s = setup().await;
    let a = s.dir.file("a.txt", 10, 1);
    let b = s.dir.file("b.txt", 10, 2);
    s.server.inject("HeadObject", None, 1, Inject::Deny);
    let (first, _) = s.start("", vec![a]).await;
    let first_summary = s.finished(&first).await;
    assert_eq!(first_summary.state, BatchState::Failed);

    let gate = Arc::new(Latch::default());
    let entered = Arc::new(Latch::default());
    {
        let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
        s.uploads.file_control(move |_, control| {
            let (gate, entered) = (Arc::clone(&gate), Arc::clone(&entered));
            control.during_send(1, move || {
                entered.open();
                gate.wait(Duration::from_secs(20));
            });
        });
    }
    let (second, _) = s.start("", vec![b]).await;
    assert!(entered.wait(Duration::from_secs(20)));
    // The first batch's id neither cancels nor retries anything now.
    assert_eq!(s.uploads.cancel(&first), CancelOutcome::NotActive);
    assert_eq!(s.retry(&first).await.unwrap_err().kind, ErrorKind::Busy);
    gate.open();
    let summary = s.finished(&second).await;
    assert_eq!(summary.state, BatchState::Completed);
    // Starting the second batch dropped the first one's retry.
    assert_eq!(
        s.retry(&first).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    // Every event names its own batch; none of the first batch came after
    // its final event.
    let events = s.sink.events();
    let first_final = events
        .iter()
        .position(|e| e.batch == first && matches!(e.body, EventBody::Finished { .. }))
        .unwrap();
    assert!(events[first_final + 1..].iter().all(|e| e.batch == second));
}

// ---- shutdown ----

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_cancels_the_batch_and_waits_for_the_multipart_abort() {
    let s = setup().await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    let small = s.dir.file("small.txt", 10, 2);
    let entered = Arc::new(Latch::default());
    {
        let entered = Arc::clone(&entered);
        s.uploads.file_control(move |_, control| {
            let entered = Arc::clone(&entered);
            control.during_send(2, move || entered.open());
        });
    }
    // The abort takes a while: shutdown must wait for it.
    s.server.inject(
        "AbortMultipartUpload",
        None,
        1,
        Inject::Run(Arc::new(|| std::thread::sleep(Duration::from_millis(400)))),
    );
    let (batch, _) = s.start("", vec![big, small]).await;
    assert!(entered.wait(Duration::from_secs(20)));
    let uploads = Arc::clone(&s.uploads);
    let at = Instant::now();
    // `shutdown` blocks (it runs in the exit handler), so not on this thread.
    tokio::task::spawn_blocking(move || uploads.shutdown(SHUTDOWN_WAIT))
        .await
        .unwrap();
    let took = at.elapsed();
    assert!(took >= Duration::from_millis(350), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(s.server.open_multipart_files(), Vec::<String>::new());
    assert!(s.uploads.unresolved().is_empty());
    let summary = s
        .of_finished(&batch)
        .expect("finished before shutdown returned");
    assert_eq!(summary.state, BatchState::Cancelled);
    assert_eq!(file(&summary, 0).remote, Some(RemoteState::NothingStored));
    assert!(!file(&summary, 1).started);
    assert_eq!(s.object("small.txt"), None);
    assert_eq!(s.uploads.reserve().err().unwrap().kind, ErrorKind::Busy);
    // Idle: returns at once.
    let at = Instant::now();
    CloudUploads::new().shutdown(SHUTDOWN_WAIT);
    assert!(at.elapsed() < Duration::from_millis(100));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_aborts_known_incomplete_uploads() {
    let s = setup().await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    s.server
        .inject("AbortMultipartUpload", None, 1, Inject::Deny);
    let weak = s.weak();
    s.uploads.file_control(move |_, control| {
        let weak = weak.clone();
        control.during_send(2, move || {
            weak.upgrade().unwrap().cancel_active();
        });
    });
    let (batch, _) = s.start("", vec![big]).await;
    s.finished(&batch).await;
    assert_eq!(s.uploads.unresolved().len(), 1);
    assert!(!s.server.open_multipart_files().is_empty());
    let uploads = Arc::clone(&s.uploads);
    tokio::task::spawn_blocking(move || uploads.shutdown(SHUTDOWN_WAIT))
        .await
        .unwrap();
    assert!(s.uploads.unresolved().is_empty());
    assert_eq!(s.server.open_multipart_files(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_never_waits_longer_than_its_limit() {
    let s = setup_with(ClientOptions {
        abort_timeout: Duration::from_secs(1),
        ..options()
    })
    .await;
    let big = s.dir.file("big.bin", 11 * MIB, 1);
    // Every abort hangs.
    s.server
        .inject("AbortMultipartUpload", None, 100, Inject::Hang);
    let weak = s.weak();
    s.uploads.file_control(move |_, control| {
        let weak = weak.clone();
        control.during_send(2, move || {
            weak.upgrade().unwrap().cancel_active();
        });
    });
    let (batch, _) = s.start("", vec![big]).await;
    let summary = s.finished(&batch).await;
    assert_eq!(file(&summary, 0).remote, Some(RemoteState::IncompleteParts));
    let uploads = Arc::clone(&s.uploads);
    let at = Instant::now();
    tokio::task::spawn_blocking(move || uploads.shutdown(Duration::from_millis(300)))
        .await
        .unwrap();
    let took = at.elapsed();
    assert!(took < Duration::from_millis(900), "{took:?}");
    // Not removed, so not forgotten (and not claimed as removed).
    assert_eq!(s.uploads.unresolved().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shutdown_during_the_file_dialog_refuses_the_batch() {
    let s = setup().await;
    let a = s.dir.file("a.txt", 10, 1);
    let uploads = Arc::clone(&s.uploads);
    let e = begin_upload(
        &s.uploads,
        Arc::clone(&s.accounts),
        target(""),
        async move {
            // The user quits while the dialog is open.
            tokio::task::spawn_blocking(move || uploads.shutdown(SHUTDOWN_WAIT))
                .await
                .unwrap();
            Some(vec![a])
        },
        Arc::clone(&s.sink) as Arc<dyn UploadSink>,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    assert!(!s.uploads.busy(), "the reservation was released");
    assert!(s.sink.events().is_empty());
    assert!(s.server.ops().is_empty());
    assert_eq!(s.uploads.reserve().err().unwrap().kind, ErrorKind::Busy);
}

#[test]
fn local_paths_never_appear_in_a_serialized_file() {
    let path = Path::new("/home/someone/private/a.txt");
    let candidate = Candidate {
        name: "a.txt".into(),
        key: Some("a.txt".into()),
        path: path.to_owned(),
        size: Some(1),
        skip: None,
    };
    let json = serde_json::to_string(&UploadFile {
        index: 0,
        name: candidate.name.clone(),
        key: candidate.key.clone(),
        size: candidate.size,
        state: FileState::Failed,
        stage: Some(Stage::Hashing),
        bytes: 0,
        total: 1,
        started: true,
        skip: None,
        error: Some(upload_error(&StorageError::Io(format!(
            "{}: denied",
            path.display()
        )))),
        remote: Some(RemoteState::NothingStored),
        retryable: true,
    })
    .unwrap();
    assert!(!json.contains("/home/someone"), "{json}");
}

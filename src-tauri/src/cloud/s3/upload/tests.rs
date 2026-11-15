//! Tests of [`S3Store::upload`] against the in-process S3 server
//! (`test_server`: s3s + s3s-fs, real HTTP and SigV4, real multipart
//! uploads), with faults injected per operation and part, test hooks on the
//! [`TransferControl`], and a throttled TCP link for slow and stalled
//! connections. No real provider.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::super::super::test_server::{
    CONDITIONAL_PROFILE, Inject, Pace, TempDir, TestServer, ThrottledLink, multipart_files_in,
    sha256_hex,
};
use super::super::{ClientOptions, Verification};
use super::*;

const MIB: usize = 1024 * 1024;
const BUCKET: &str = "upload-bucket";

fn options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(30),
        max_attempts: 1,
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        stall_timeout: Duration::from_secs(30),
        abort_timeout: Duration::from_secs(5),
        part_settle_timeout: Duration::from_secs(5),
    }
}

async fn server() -> TestServer {
    TestServer::start("upload-key", "upload-Secret-0123456789", &[BUCKET]).await
}

fn store_with(server: &TestServer, endpoint: &str, options: ClientOptions) -> S3Store {
    let mut account = server.account("local", &[BUCKET]);
    account.endpoint = Some(endpoint.to_owned());
    S3Store::new(account.resolve().unwrap(), server.credentials(), options)
}

fn store(server: &TestServer) -> S3Store {
    store_with(server, &server.endpoint, options())
}

type Events = Vec<(Instant, UploadProgress)>;

async fn run(
    s: &S3Store,
    key: &str,
    path: &Path,
    control: &Arc<TransferControl>,
) -> (Result<UploadReport, UploadError>, Events) {
    let mut events = Vec::new();
    let result = s
        .upload(BUCKET, key, path, control, &mut |p| {
            events.push((Instant::now(), p));
        })
        .await;
    (result, events)
}

/// Stages only move forward, bytes never decrease within a stage, every
/// total is the file's size, updates within a stage are at least the
/// interval apart except a stage's last one, and verification is the last
/// stage of a completed upload only.
fn check_progress(events: &Events, total: u64, completed: bool) {
    for pair in events.windows(2) {
        let ((t0, a), (t1, b)) = (pair[0], pair[1]);
        assert!(b.stage >= a.stage, "{events:?}");
        if a.stage == b.stage {
            assert!(b.bytes > a.bytes, "{events:?}");
            if b.bytes < total {
                assert!(
                    t1.duration_since(t0) >= PROGRESS_INTERVAL - Duration::from_millis(10),
                    "throttled: {:?} apart",
                    t1.duration_since(t0)
                );
            }
        }
    }
    assert!(
        events
            .iter()
            .all(|(_, p)| p.total == total && p.bytes <= total)
    );
    let verified = events
        .iter()
        .any(|(_, p)| p.stage == UploadStage::Verifying);
    if completed {
        let (_, last) = events.last().expect("progress");
        assert_eq!(
            (last.stage, last.bytes),
            (UploadStage::Verifying, total),
            "{events:?}"
        );
    } else {
        assert!(!verified, "{events:?}");
    }
}

fn expect_err(result: Result<UploadReport, UploadError>) -> UploadError {
    match result {
        Ok(r) => panic!("expected an error, got {r:?}"),
        Err(e) => e,
    }
}

fn count(ops: &[String], op: &str) -> usize {
    ops.iter().filter(|o| *o == op).count()
}

/// Overwrites `len` bytes at `offset` with different bytes, keeping the
/// size.
fn overwrite(path: &Path, offset: u64, len: usize) {
    let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.write_all(&vec![0x5a; len]).unwrap();
    f.sync_all().unwrap();
}

/// A control whose hook can reach it without keeping it alive.
fn control_and_weak() -> (Arc<TransferControl>, Weak<TransferControl>) {
    let control = Arc::new(TransferControl::new());
    let weak = Arc::downgrade(&control);
    (control, weak)
}

// ---------------------------------------------------------------- basics

#[tokio::test(flavor = "multi_thread")]
async fn a_small_file_is_uploaded_and_verified() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-small");
    let src = dir.file("note.txt", 200_000, 1);
    let control = Arc::new(TransferControl::new());

    let (result, events) = run(&s, "docs/note.txt", &src, &control).await;
    let up = result.unwrap();
    assert_eq!((up.size, up.parts), (200_000, 1));
    assert_eq!(up.sha256, sha256_hex(&fs::read(&src).unwrap()));
    check_progress(&events, 200_000, true);
    assert_eq!(server.ops(), ["HeadObject", "PutObject", "HeadObject"]);
    assert!(server.metadata_path(BUCKET, "docs/note.txt").exists());
    let head = s
        .head_object(BUCKET, "docs/note.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (head.size, head.sha256.as_deref()),
        (200_000, Some(up.sha256.as_str()))
    );
    assert_eq!(
        fs::read(server.object_path(BUCKET, "docs/note.txt")).unwrap(),
        fs::read(&src).unwrap()
    );
    assert!(
        !control.cancel(),
        "a finished upload is past its commit point"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_file_is_uploaded_and_verified() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-empty");
    let src = dir.file("empty", 0, 2);
    let (result, events) = run(&s, "empty", &src, &Arc::new(TransferControl::new())).await;
    let up = result.unwrap();
    assert_eq!((up.size, up.parts), (0, 1));
    assert_eq!(up.sha256, sha256_hex(b""));
    check_progress(&events, 0, true);
    let head = s.head_object(BUCKET, "empty").await.unwrap().unwrap();
    assert_eq!((head.size, head.sha256), (0, Some(up.sha256)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_file_is_uploaded_in_parts_and_verified() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-multi");
    let size = 12 * MIB + 123;
    let src = dir.file("video.bin", size, 3);
    let (result, events) = run(&s, "video.bin", &src, &Arc::new(TransferControl::new())).await;
    let up = result.unwrap();
    assert_eq!((up.size, up.parts), (size as u64, 3));
    check_progress(&events, size as u64, true);
    assert_eq!(
        server.ops(),
        [
            "HeadObject",
            "CreateMultipartUpload",
            "UploadPart#1",
            "UploadPart#2",
            "UploadPart#3",
            "HeadObject",
            "CompleteMultipartUpload",
            "HeadObject"
        ]
    );
    assert!(server.open_multipart_files().is_empty());
    let back = dir.path("back");
    let down = s.download_file(BUCKET, "video.bin", &back).await.unwrap();
    assert_eq!(down.verification, Verification::Sha256Matched);
    assert_eq!(fs::read(&back).unwrap(), fs::read(&src).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_special_character_and_nested_keys_are_kept_exactly() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-keys");
    let src = dir.file("src", 70_000, 4);
    for key in [
        "Ferien 2024 ü/Strand Bild (1).jpg",
        "a+b=c&d ü.txt",
        "deep/er/still/deeper/file.bin",
        "100% sure; #1 ~test~.txt",
    ] {
        let (result, _) = run(&s, key, &src, &Arc::new(TransferControl::new())).await;
        let up = result.unwrap_or_else(|e| panic!("{key}: {e}"));
        let head = s.head_object(BUCKET, key).await.unwrap().expect(key);
        assert_eq!(head.sha256.as_deref(), Some(up.sha256.as_str()), "{key}");
        let back = dir.path("back");
        let _ = fs::remove_file(&back);
        s.download_file(BUCKET, key, &back).await.unwrap();
        assert_eq!(fs::read(&back).unwrap(), fs::read(&src).unwrap(), "{key}");
    }
    let mut keys: Vec<_> = s
        .list_objects(BUCKET, "")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "100% sure; #1 ~test~.txt",
            "Ferien 2024 ü/Strand Bild (1).jpg",
            "a+b=c&d ü.txt",
            "deep/er/still/deeper/file.bin"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_destination_is_refused_before_any_data_is_sent() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-exists");
    let first = dir.file("first", 1000, 5);
    let second = dir.file("second", 2000, 6);
    run(&s, "taken", &first, &Arc::new(TransferControl::new()))
        .await
        .0
        .unwrap();
    server.operations.lock().unwrap().clear();

    let (result, events) = run(&s, "taken", &second, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::AlreadyExists(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    check_progress(&events, 2000, false);
    assert_eq!(server.ops(), ["HeadObject"]);
    assert_eq!(
        fs::read(server.object_path(BUCKET, "taken")).unwrap(),
        fs::read(&first).unwrap()
    );
    // Through the compatibility wrapper too.
    let e = s.upload_file(BUCKET, "taken", &second).await.unwrap_err();
    assert!(matches!(e, StorageError::AlreadyExists(_)), "{e:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_multipart_upload_checks_the_destination_again_before_completing() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-recheck");
    let src = dir.file("src", 12 * MIB, 7);
    // Another client creates the object while part 2 is being sent.
    let theirs = server.object_path(BUCKET, "contested");
    let theirs2 = theirs.clone();
    server.inject(
        "UploadPart",
        Some(2),
        1,
        Inject::Run(Arc::new(move || fs::write(&theirs2, b"theirs").unwrap())),
    );
    let (result, _) = run(&s, "contested", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::AlreadyExists(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    let ops = server.ops();
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert_eq!(count(&ops, "CompleteMultipartUpload"), 0, "{ops:?}");
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(fs::read(&theirs).unwrap(), b"theirs");
}

/// A store whose profile has conditional writes established (tests only,
/// ADR-029); the test server enforces `If-None-Match: *` like Amazon.
fn conditional_store(server: &TestServer) -> S3Store {
    let mut account = server.resolved("local", &[BUCKET]);
    account.profile = &CONDITIONAL_PROFILE;
    S3Store::new(account, server.credentials(), options())
}

#[tokio::test(flavor = "multi_thread")]
async fn without_an_established_capability_no_conditional_header_is_sent() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-uncond");
    let small = dir.file("small", 10 * 1024, 1);
    let large = dir.file("large", 12 * MIB, 2);
    let control = Arc::new(TransferControl::new());
    run(&s, "small", &small, &control).await.0.unwrap();
    let control = Arc::new(TransferControl::new());
    run(&s, "large", &large, &control).await.0.unwrap();
    assert_eq!(server.conditional_ops(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conditional_upload_sends_if_none_match_and_succeeds_on_a_free_key() {
    let server = server().await;
    let s = conditional_store(&server);
    let dir = TempDir::new("up-cond-free");
    let small = dir.file("small", 10 * 1024, 1);
    let large = dir.file("large", 12 * MIB, 2);
    let control = Arc::new(TransferControl::new());
    let report = run(&s, "small", &small, &control).await.0.unwrap();
    assert_eq!(report.sha256, sha256_hex(&fs::read(&small).unwrap()));
    let control = Arc::new(TransferControl::new());
    run(&s, "large", &large, &control).await.0.unwrap();
    assert_eq!(
        server.conditional_ops(),
        ["PutObject", "CompleteMultipartUpload"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conditional_put_refused_for_another_writers_object_stores_nothing() {
    let server = server().await;
    let s = conditional_store(&server);
    let dir = TempDir::new("up-cond-race");
    let src = dir.file("src", 10 * 1024, 3);
    // Another client creates the key after the HEAD check, right before
    // this PUT reaches the provider.
    let theirs = server.object_path(BUCKET, "raced");
    let theirs2 = theirs.clone();
    server.inject(
        "PutObject",
        None,
        1,
        Inject::Run(Arc::new(move || fs::write(&theirs2, b"theirs").unwrap())),
    );
    let (result, _) = run(&s, "raced", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::AlreadyExists(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    assert_eq!(fs::read(&theirs).unwrap(), b"theirs", "never overwritten");
    assert_eq!(server.conditional_ops(), ["PutObject"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conditional_put_that_finds_its_own_content_succeeds() {
    let server = server().await;
    let s = conditional_store(&server);
    let dir = TempDir::new("up-cond-own");
    let src = dir.file("src", 10 * 1024, 4);
    let sha = sha256_hex(&fs::read(&src).unwrap());
    // As if an earlier attempt of this PUT was stored and its answer lost:
    // the same bytes with the same SHA-256 metadata are already there.
    let (object, metadata) = (
        server.object_path(BUCKET, "own"),
        server.metadata_path(BUCKET, "own"),
    );
    let (src2, sha2) = (src.clone(), sha.clone());
    server.inject(
        "PutObject",
        None,
        1,
        Inject::Run(Arc::new(move || {
            fs::copy(&src2, &object).unwrap();
            fs::write(
                &metadata,
                format!("{{\"user_metadata\":{{\"sotyware-sha256\":\"{sha2}\"}}}}"),
            )
            .unwrap();
        })),
    );
    let (result, _) = run(&s, "own", &src, &Arc::new(TransferControl::new())).await;
    let report = result.expect("the stored object is exactly this upload");
    assert_eq!(report.sha256, sha);
    assert_eq!(server.conditional_ops(), ["PutObject"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conditional_completion_refused_for_another_writer_aborts_and_stores_nothing() {
    let server = server().await;
    let s = conditional_store(&server);
    let dir = TempDir::new("up-cond-complete");
    let src = dir.file("src", 12 * MIB, 5);
    let theirs = server.object_path(BUCKET, "raced-mp");
    let theirs2 = theirs.clone();
    server.inject(
        "CompleteMultipartUpload",
        None,
        1,
        Inject::Run(Arc::new(move || fs::write(&theirs2, b"theirs").unwrap())),
    );
    let (result, _) = run(&s, "raced-mp", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::AlreadyExists(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    let ops = server.ops();
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(fs::read(&theirs).unwrap(), b"theirs", "never overwritten");
    assert_eq!(server.conditional_ops(), ["CompleteMultipartUpload"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_object_of_the_wrong_size_fails_verification_and_is_kept() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-verify-size");
    let src = dir.file("src", 100_000, 8);
    // Changes the object at rest right before the verifying HEAD (the first
    // HEAD, the destination check, finds nothing to change).
    let at_rest = server.object_path(BUCKET, "changed");
    server.inject(
        "HeadObject",
        None,
        usize::MAX,
        Inject::Run(Arc::new(move || {
            if at_rest.exists() {
                fs::write(&at_rest, b"short").unwrap();
            }
        })),
    );
    let (result, events) = run(&s, "changed", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::Integrity(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NeedsInspection);
    assert!(e.to_string().contains("check it"), "{e}");
    // Verification was reached, completion never reported, nothing deleted.
    assert_eq!(events.last().unwrap().1.stage, UploadStage::Verifying);
    assert!(server.object_path(BUCKET, "changed").exists());
    assert_eq!(count(&server.ops(), "DeleteObject"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_object_without_the_sha256_metadata_fails_verification() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-verify-sha");
    let src = dir.file("src", 100_000, 9);
    let metadata = server.metadata_path(BUCKET, "no-meta");
    server.inject(
        "HeadObject",
        None,
        usize::MAX,
        Inject::Run(Arc::new(move || {
            let _ = fs::remove_file(&metadata);
        })),
    );
    let (result, _) = run(&s, "no-meta", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(
        matches!(&e.error, StorageError::Integrity(m) if m.contains("SHA-256")),
        "{e:?}"
    );
    assert_eq!(e.remote, RemoteOutcome::NeedsInspection);
    assert!(server.object_path(BUCKET, "no-meta").exists());
}

// ---------------------------------------------------------- cancellation

#[tokio::test(flavor = "multi_thread")]
async fn a_transfer_cancelled_before_it_begins_sends_nothing() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-early");
    let src = dir.file("src", 1000, 10);
    let control = Arc::new(TransferControl::new());
    assert!(control.cancel());
    let (result, events) = run(&s, "never", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    assert!(events.is_empty());
    assert!(server.ops().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_hashing_stops_before_any_request() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-hash");
    let src = dir.file("src", 3 * MIB, 11);
    let (control, weak) = control_and_weak();
    control.during_hashing(move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, events) = run(&s, "never", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    assert!(
        events
            .iter()
            .all(|(_, p)| p.stage == UploadStage::Hashing && p.bytes < 3 * MIB as u64)
    );
    assert!(server.ops().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_single_put_stores_nothing() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-put");
    let src = dir.file("src", 3 * MIB, 12);
    let (control, weak) = control_and_weak();
    control.during_send(1, move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, events) = run(&s, "cancelled", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    check_progress(&events, 3 * MIB as u64, false);
    // Give the server time to finish whatever it received.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.object_path(BUCKET, "cancelled").exists());
    assert_eq!(s.head_object(BUCKET, "cancelled").await.unwrap(), None);
    assert_eq!(count(&server.ops(), "PutObject"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_part_aborts_the_multipart_upload() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-part");
    let src = dir.file("src", 12 * MIB, 13);
    let (control, weak) = control_and_weak();
    control.during_send(2, move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, events) = run(&s, "cancelled", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    check_progress(&events, 12 * MIB as u64, false);
    let ops = server.ops();
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert_eq!(count(&ops, "UploadPart#3"), 0, "{ops:?}");
    assert_eq!(count(&ops, "CompleteMultipartUpload"), 0, "{ops:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        server.open_multipart_files().is_empty(),
        "{:?}",
        server.open_multipart_files()
    );
    assert_eq!(s.head_object(BUCKET, "cancelled").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_right_before_completion_aborts_and_completes_nothing() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-commit");
    let src = dir.file("src", 12 * MIB, 14);
    let (control, weak) = control_and_weak();
    control.before_commit(move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, _) = run(&s, "cancelled", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    let ops = server.ops();
    assert_eq!(
        count(&ops, "UploadPart#3"),
        1,
        "all parts were sent: {ops:?}"
    );
    assert_eq!(count(&ops, "CompleteMultipartUpload"), 0, "{ops:?}");
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "cancelled").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_right_before_the_last_chunk_of_a_put_stores_nothing() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-last");
    let src = dir.file("src", 3 * MIB, 15);
    let (control, weak) = control_and_weak();
    control.before_commit(move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, _) = run(&s, "cancelled", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.object_path(BUCKET, "cancelled").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_after_the_commit_point_is_too_late() {
    for (name, size) in [("single", 3 * MIB), ("multipart", 12 * MIB)] {
        let server = server().await;
        let s = store(&server);
        let dir = TempDir::new("up-cancel-late");
        let src = dir.file("src", size, 16);
        let (control, weak) = control_and_weak();
        let refused = Arc::new(AtomicBool::new(false));
        let refused2 = Arc::clone(&refused);
        control.after_commit(move || {
            let c = weak.upgrade().unwrap();
            refused2.store(!c.cancel() && !c.cancel(), Ordering::SeqCst);
        });
        let (result, events) = run(&s, "late", &src, &control).await;
        let up = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            refused.load(Ordering::SeqCst),
            "{name}: cancel must say finishing"
        );
        assert!(!control.is_cancelled());
        check_progress(&events, size as u64, true);
        let head = s.head_object(BUCKET, "late").await.unwrap().unwrap();
        assert_eq!(head.sha256, Some(up.sha256), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_repeatedly_is_the_same_as_once() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-cancel-twice");
    let src = dir.file("src", 12 * MIB, 17);
    let (control, weak) = control_and_weak();
    control.during_send(1, move || {
        let c = weak.upgrade().unwrap();
        assert!(c.cancel());
        assert!(c.cancel());
        assert!(c.cancel());
    });
    let (result, _) = run(&s, "twice", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    assert!(control.cancel(), "still cancelled afterwards");
    assert_eq!(count(&server.ops(), "AbortMultipartUpload"), 1);
    assert!(server.open_multipart_files().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_one_upload_leaves_another_alone() {
    let server = server().await;
    let s = Arc::new(store(&server));
    let dir = TempDir::new("up-cancel-other");
    let a = dir.file("a", 3 * MIB, 18);
    let b = dir.file("b", 3 * MIB, 19);
    let (control_a, weak) = control_and_weak();
    let control_b = Arc::new(TransferControl::new());
    control_a.during_send(1, move || assert!(weak.upgrade().unwrap().cancel()));
    let ((ra, _), (rb, _)) =
        tokio::join!(run(&s, "a", &a, &control_a), run(&s, "b", &b, &control_b));
    assert_eq!(expect_err(ra).error, StorageError::Cancelled);
    rb.unwrap();
    assert!(!control_b.is_cancelled());
    assert!(server.object_path(BUCKET, "b").exists());
    assert!(!server.object_path(BUCKET, "a").exists());
}

// ------------------------------------------------------ multipart cleanup

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_part_aborts_the_multipart_upload() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-part-fails");
    let src = dir.file("src", 12 * MIB, 20);
    server.inject("UploadPart", Some(2), 1, Inject::Deny);
    let (result, _) = run(&s, "failed", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    // The server refuses before reading the body; the client sees the 403,
    // or a broken pipe if the connection closes while it is still sending.
    assert!(
        matches!(
            e.error,
            StorageError::AccessDenied(_) | StorageError::Network(_)
        ),
        "{e:?}"
    );
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    let ops = server.ops();
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert_eq!(count(&ops, "UploadPart#3"), 0, "{ops:?}");
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "failed").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_completion_aborts_the_multipart_upload() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-complete-fails");
    let src = dir.file("src", 12 * MIB, 21);
    server.inject("CompleteMultipartUpload", None, 1, Inject::Deny);
    let (result, events) = run(&s, "failed", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::AccessDenied(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    check_progress(&events, 12 * MIB as u64, false);
    assert_eq!(count(&server.ops(), "AbortMultipartUpload"), 1);
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "failed").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_abort_says_that_parts_may_remain() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-abort-fails");
    let src = dir.file("src", 12 * MIB, 22);
    server.inject("AbortMultipartUpload", None, 1, Inject::Deny);
    let (control, weak) = control_and_weak();
    control.during_send(2, move || assert!(weak.upgrade().unwrap().cancel()));
    let (result, _) = run(&s, "orphaned", &src, &control).await;
    let e = expect_err(result);
    assert_eq!(e.error, StorageError::Cancelled);
    assert!(
        matches!(&e.remote, RemoteOutcome::IncompleteParts { upload_id } if !upload_id.is_empty()),
        "{e:?}"
    );
    assert!(e.to_string().contains("may remain"), "{e}");
    // Honest: the parts really are still there.
    assert!(!server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "orphaned").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abort_that_never_answers_is_given_up_after_its_timeout() {
    let server = server().await;
    let s = store_with(
        &server,
        &server.endpoint,
        ClientOptions {
            abort_timeout: Duration::from_secs(1),
            ..options()
        },
    );
    let dir = TempDir::new("up-abort-hangs");
    let src = dir.file("src", 12 * MIB, 23);
    server.inject("UploadPart", Some(2), 1, Inject::Deny);
    server.inject("AbortMultipartUpload", None, 1, Inject::Hang);
    let started = Instant::now();
    let (result, _) = run(&s, "orphaned", &src, &Arc::new(TransferControl::new())).await;
    let e = expect_err(result);
    assert!(
        matches!(
            e.error,
            StorageError::AccessDenied(_) | StorageError::Network(_)
        ),
        "{e:?}"
    );
    assert!(
        matches!(e.remote, RemoteOutcome::IncompleteParts { .. }),
        "{e:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

// -------------------------------------------------------------- integrity

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_during_hashing_is_never_uploaded() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-change-hash");
    let src = dir.file("src", 3 * MIB, 24);
    let path = src.clone();
    let control = Arc::new(TransferControl::new());
    // The first MiB was hashed already; same size, new content.
    control.during_hashing(move || overwrite(&path, 100, 100));
    let (result, _) = run(&s, "changed", &src, &control).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::FileChanged(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.object_path(BUCKET, "changed").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_same_size_change_during_a_put_is_never_uploaded() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-change-put");
    let src = dir.file("src", 3 * MIB, 25);
    let path = src.clone();
    let control = Arc::new(TransferControl::new());
    control.during_send(1, move || overwrite(&path, 3 * MIB as u64 - 100, 50));
    let (result, events) = run(&s, "changed", &src, &control).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::FileChanged(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    check_progress(&events, 3 * MIB as u64, false);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.object_path(BUCKET, "changed").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_to_a_later_part_aborts_the_multipart_upload() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-change-part");
    let src = dir.file("src", 12 * MIB, 26);
    let path = src.clone();
    let control = Arc::new(TransferControl::new());
    // While part 2 is sent, part 3's bytes change (same size).
    control.during_send(2, move || overwrite(&path, 11 * MIB as u64, 10));
    let (result, _) = run(&s, "changed", &src, &control).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::FileChanged(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    let ops = server.ops();
    assert_eq!(count(&ops, "CompleteMultipartUpload"), 0, "{ops:?}");
    assert_eq!(count(&ops, "AbortMultipartUpload"), 1, "{ops:?}");
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "changed").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_truncated_during_the_upload_is_never_uploaded() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-truncate");
    let src = dir.file("src", 12 * MIB, 27);
    let path = src.clone();
    let control = Arc::new(TransferControl::new());
    control.during_send(2, move || {
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(7 * MIB as u64)
            .unwrap();
    });
    let (result, _) = run(&s, "truncated", &src, &control).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::FileChanged(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    assert!(server.open_multipart_files().is_empty());
    assert_eq!(s.head_object(BUCKET, "truncated").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_touched_after_its_bytes_were_sent_is_not_reported_as_uploaded() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-touch");
    let src = dir.file("src", 12 * MIB, 28);
    let path = src.clone();
    let control = Arc::new(TransferControl::new());
    // Part 1's bytes were sent unchanged; the file is then rewritten with the
    // same content, so only its modification time tells.
    control.during_send(3, move || {
        let data = fs::read(&path).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&path, data).unwrap();
    });
    let (result, _) = run(&s, "touched", &src, &control).await;
    let e = expect_err(result);
    assert!(matches!(e.error, StorageError::FileChanged(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    assert_eq!(count(&server.ops(), "CompleteMultipartUpload"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn part_digests_match_the_exact_part_boundaries() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("up-digests");
    for (size, parts) in [
        (12 * MIB + 123, 3),
        (10 * MIB, 2),
        (10 * MIB + 1, 3),
        (6 * MIB, 1),
        (1, 1),
        (0, 1),
    ] {
        let src = dir.file(&format!("f{size}"), size, 29);
        let data = fs::read(&src).unwrap();
        let control = TransferControl::new();
        let mut sink = |_| {};
        let mut reporter = Reporter::new(&mut sink);
        let first = s.first_pass(&src, &control, &mut reporter).await.unwrap();
        assert_eq!(first.sha256, sha256_hex(&data), "{size}");
        assert_eq!(first.parts.len(), parts, "{size}");
        let chunk = if parts == 1 { size.max(1) } else { 5 * MIB };
        let expected: Vec<String> = data
            .chunks(chunk)
            .map(sha256_hex)
            .chain((size == 0).then(|| sha256_hex(b"")))
            .collect();
        let got: Vec<String> = first.parts.iter().map(|d| hex(d)).collect();
        assert_eq!(got, expected, "{size}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sdk_retries_restart_the_hash_and_the_upload_still_verifies() {
    let server = server().await;
    let s = store_with(
        &server,
        &server.endpoint,
        ClientOptions {
            max_attempts: 3,
            ..options()
        },
    );
    let dir = TempDir::new("up-retry");
    // Multipart: part 2's first attempt is refused (transient), after part
    // of its body may already have been read and hashed.
    let src = dir.file("multi", 12 * MIB, 30);
    server.inject("UploadPart", Some(2), 1, Inject::InternalError);
    let (result, events) = run(&s, "multi", &src, &Arc::new(TransferControl::new())).await;
    let up = result.unwrap();
    check_progress(&events, 12 * MIB as u64, true);
    assert_eq!(
        count(&server.ops(), "UploadPart#2"),
        2,
        "{:?}",
        server.ops()
    );
    let head = s.head_object(BUCKET, "multi").await.unwrap().unwrap();
    assert_eq!(head.sha256, Some(up.sha256));
    assert_eq!(
        fs::read(server.object_path(BUCKET, "multi")).unwrap(),
        fs::read(&src).unwrap()
    );

    // Single PUT: the retried attempt passes the commit point again.
    let src = dir.file("single", 3 * MIB, 31);
    server.inject("PutObject", None, 1, Inject::InternalError);
    let (result, events) = run(&s, "single", &src, &Arc::new(TransferControl::new())).await;
    result.unwrap();
    check_progress(&events, 3 * MIB as u64, true);
    assert_eq!(count(&server.ops(), "PutObject"), 2);
    assert_eq!(
        fs::read(server.object_path(BUCKET, "single")).unwrap(),
        fs::read(&src).unwrap()
    );
}

// --------------------------------------------------------------- timeouts

#[tokio::test(flavor = "multi_thread")]
async fn without_the_override_the_sdk_read_timeout_cuts_a_slow_upload() {
    // The control experiment for the next test: through the same throttled
    // link, a plain `PutObject` with the client's own settings fails once
    // the whole request takes longer than the read timeout, though bytes
    // keep moving.
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::BytesPerSecond(MIB as u64)).await;
    let s = store_with(
        &server,
        &link.endpoint,
        ClientOptions {
            read_timeout: Duration::from_secs(1),
            ..options()
        },
    );
    let dir = TempDir::new("up-control");
    let src = dir.file("src", 4 * MIB, 32);
    let body = ByteStream::from_path(&src).await.unwrap();
    let started = Instant::now();
    let e = s
        .sdk_client()
        .put_object()
        .bucket(BUCKET)
        .key("control")
        .body(body)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(classify("put", e), StorageError::Timeout(_)),
        "after {:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_but_steady_upload_outlasts_the_read_timeout() {
    // The same kind of link and read timeout as the control experiment above.
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::BytesPerSecond(2 * MIB as u64)).await;
    let s = store_with(
        &server,
        &link.endpoint,
        ClientOptions {
            read_timeout: Duration::from_secs(1),
            // On loopback the client's socket buffer takes a few MiB at
            // once, which then drain after the last byte was handed over.
            stall_timeout: Duration::from_secs(5),
            multipart_threshold: 64 * MIB as u64,
            ..options()
        },
    );
    let dir = TempDir::new("up-slow");
    let size = 12 * MIB;
    let src = dir.file("src", size, 33);
    let started = Instant::now();
    let (result, events) = run(&s, "slow", &src, &Arc::new(TransferControl::new())).await;
    let elapsed = started.elapsed();
    let up = result.unwrap_or_else(|e| panic!("after {elapsed:?}: {e}"));
    let body_done = events
        .iter()
        .find(|(_, p)| p.stage == UploadStage::Sending && p.bytes == size as u64)
        .map(|(t, _)| t.duration_since(started));
    println!(
        "12 MiB at 2 MiB/s in one PUT: {elapsed:?} (all bytes first reported after {body_done:?}), {} progress updates",
        events.len()
    );
    assert!(elapsed > Duration::from_secs(5), "{elapsed:?}");
    assert_eq!(up.parts, 1, "one request longer than the read timeout");
    check_progress(&events, size as u64, true);
    let sending = events
        .iter()
        .filter(|(_, p)| p.stage == UploadStage::Sending)
        .count();
    assert!(sending >= 8, "{sending} updates while sending");
    assert_eq!(
        fs::read(server.object_path(BUCKET, "slow")).unwrap(),
        fs::read(&src).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_upload_times_out_and_stores_nothing() {
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::StallAfter(256 * 1024)).await;
    let s = store_with(
        &server,
        &link.endpoint,
        ClientOptions {
            // One PUT, far larger than any socket buffer.
            multipart_threshold: 64 * MIB as u64,
            stall_timeout: Duration::from_millis(1500),
            ..options()
        },
    );
    let dir = TempDir::new("up-stall");
    let src = dir.file("src", 24 * MIB, 34);
    let started = Instant::now();
    let (result, events) = run(&s, "stalled", &src, &Arc::new(TransferControl::new())).await;
    let elapsed = started.elapsed();
    let e = expect_err(result);
    println!("stalled after {elapsed:?}: {e}");
    assert!(matches!(e.error, StorageError::Timeout(_)), "{e:?}");
    assert_eq!(e.remote, RemoteOutcome::NothingStored);
    assert!(
        elapsed >= Duration::from_millis(1500) && elapsed < Duration::from_secs(10),
        "{elapsed:?}"
    );
    check_progress(&events, 24 * MIB as u64, false);
    assert!(!server.object_path(BUCKET, "stalled").exists());
}

/// The real setting: one PUT that takes more than a minute with the default
/// 60-second read timeout and stall timeout. Slow (about 70 s), so opt-in:
/// `cargo test cloud::s3::upload::tests::a_slow_upload_longer_than_a_minute -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "takes more than a minute"]
async fn a_slow_upload_longer_than_a_minute_succeeds_with_the_default_timeouts() {
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::BytesPerSecond(100 * 1024)).await;
    let s = store_with(
        &server,
        &link.endpoint,
        ClientOptions {
            max_attempts: 1,
            ..ClientOptions::default()
        },
    );
    assert_eq!(s.options.read_timeout, Duration::from_secs(60));
    assert_eq!(s.options.stall_timeout, Duration::from_secs(60));
    let dir = TempDir::new("up-minute");
    let size = 7 * MIB;
    let src = dir.file("src", size, 35);
    let started = Instant::now();
    let (result, events) = run(&s, "minute", &src, &Arc::new(TransferControl::new())).await;
    let elapsed = started.elapsed();
    let up = result.unwrap_or_else(|e| panic!("after {elapsed:?}: {e}"));
    println!(
        "7 MiB at 100 KiB/s in one PUT: {elapsed:?}, {} progress updates",
        events.len()
    );
    for (t, p) in &events {
        println!(
            "  {:>6.2} s {:?} {} bytes",
            t.duration_since(started).as_secs_f64(),
            p.stage,
            p.bytes
        );
    }
    assert!(elapsed > Duration::from_secs(65), "{elapsed:?}");
    assert_eq!(up.parts, 1);
    check_progress(&events, size as u64, true);
    assert_eq!(
        fs::read(server.object_path(BUCKET, "minute")).unwrap(),
        fs::read(&src).unwrap()
    );
}

// --------------------------------------------------------------- progress

#[test]
fn progress_is_throttled_monotonic_and_moves_forward_only() {
    let mut seen = Vec::new();
    let mut sink = |p: UploadProgress| seen.push((p.stage, p.bytes));
    let mut r = Reporter::new(&mut sink);
    r.total = 1000;
    let t0 = Instant::now();
    let ms = |n| t0 + Duration::from_millis(n);
    use UploadStage::*;
    r.report_at(ms(0), Hashing, 0, false); // first: out
    r.report_at(ms(100), Hashing, 500, false); // too soon
    r.report_at(ms(300), Hashing, 600, false); // out
    r.report_at(ms(310), Hashing, 1000, true); // last of the stage: out
    r.report_at(ms(320), Sending, 0, false); // new stage: out
    r.report_at(ms(600), Sending, 400, false); // out
    r.report_at(ms(900), Sending, 100, false); // a retry restarted: held at 400
    r.report_at(ms(1200), Sending, 700, false); // out
    r.report_at(ms(1210), Hashing, 1000, true); // backwards: dropped
    r.report_at(ms(1220), Sending, 1000, true); // out
    r.report_at(ms(1230), Sending, 1000, true); // repeat: dropped
    r.report_at(ms(1240), Verifying, 1000, true); // out
    r.report_at(ms(1250), Verifying, 5000, true); // clamped to the total: repeat
    assert_eq!(r.flush_at(), None);
    assert_eq!(
        seen,
        [
            (Hashing, 0),
            (Hashing, 600),
            (Hashing, 1000),
            (Sending, 0),
            (Sending, 400),
            (Sending, 700),
            (Sending, 1000),
            (Verifying, 1000)
        ]
    );
}

#[test]
fn an_update_held_back_by_the_throttle_is_sent_once_due() {
    let mut seen = Vec::new();
    let mut sink = |p: UploadProgress| seen.push((p.stage, p.bytes));
    let mut r = Reporter::new(&mut sink);
    r.total = 1000;
    let t0 = Instant::now();
    let ms = |n| t0 + Duration::from_millis(n);
    use UploadStage::*;
    r.report_at(ms(0), Sending, 0, false);
    // A burst: everything within the interval is held back.
    r.report_at(ms(10), Sending, 300, false);
    r.report_at(ms(20), Sending, 600, false);
    // A retried request restarts lower: the held value is kept.
    r.report_at(ms(30), Sending, 100, false);
    assert_eq!(r.flush_at(), Some(ms(250)));
    r.flush(ms(200)); // not due yet
    r.flush(ms(250));
    assert_eq!(r.flush_at(), None);
    r.flush(ms(900)); // nothing held
    assert_eq!(seen, [(Sending, 0), (Sending, 600)]);
}

#[test]
fn upload_errors_say_what_may_be_left_at_the_provider() {
    let e = UploadError::nothing(StorageError::Cancelled);
    assert_eq!(e.to_string(), "cancelled");
    let e = UploadError {
        error: StorageError::Cancelled,
        remote: RemoteOutcome::IncompleteParts {
            upload_id: "u".into(),
        },
    };
    assert!(e.to_string().contains("may remain"));
}

/// Probe, not a regression test (opt-in, prints what happens): cancels a
/// multipart upload just after the HTTP stack took the last byte of part 2,
/// behind a slow link. Dropping the request closes the socket gracefully,
/// so the bytes already in the kernel's send buffer still reach the server,
/// while the abort goes out on a new connection and can arrive first. If
/// the server stores the part after the abort (s3s-fs does; AWS documents
/// that an in-progress part "might or might not succeed"), a part of an
/// aborted upload remains. Run:
/// `cargo test --lib part_landing_after_the_abort -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "probe of a timing race; prints its findings"]
async fn probe_a_part_landing_after_the_abort() {
    let mut left = 0;
    let trials = 3;
    for trial in 0..trials {
        let server = server().await;
        let link = ThrottledLink::start(&server.endpoint, Pace::BytesPerSecond(1024 * 1024)).await;
        let s = store_with(&server, &link.endpoint, options());
        let dir = TempDir::new("up-late-part");
        let src = dir.file("src", 12 * MIB, 50 + trial as u8);
        let control = Arc::new(TransferControl::new());
        let weak = Arc::downgrade(&control);
        let started = Instant::now();
        let result = s
            .upload(BUCKET, "late.bin", &src, &control, &mut |p| {
                if p.stage == UploadStage::Sending
                    && p.bytes >= 10 * MIB as u64
                    && let Some(c) = weak.upgrade()
                {
                    c.cancel();
                }
            })
            .await;
        let returned = started.elapsed();
        let e = expect_err(result);
        // Let the link deliver whatever was already sent.
        tokio::time::sleep(Duration::from_secs(8)).await;
        let files = server.open_multipart_files();
        let ops = server.ops();
        println!(
            "trial {trial}: returned after {returned:?} with {:?}/{:?}; ops {:?}; multipart files left after 8 s: {files:?}",
            e.error, e.remote, ops
        );
        if !files.is_empty() {
            left += 1;
        }
    }
    println!("a part of an aborted upload remained in {left} of {trials} trials");
}

/// What [`cancel_after_part_2_was_handed_over`] waits for after the upload
/// returned, before it lists the multipart files left.
#[derive(Clone, Copy)]
enum Drain {
    No,
    /// As long as the link needs for a whole part at its rate, plus 1 s.
    Rate,
    /// Until a stored part 2 appears, at most 30 s: the part the race
    /// leaves behind lands later on a busy machine than at the link's rate.
    UntilPart2,
}

/// Cancels a 3-part upload the moment part 2 has been handed over
/// completely ([`TransferControl::handed_over`]), behind a link that
/// buffers it and forwards it at `rate` (at 4 MiB/s part 2 needs about
/// 1.3 s to arrive). Not from a progress update: those are throttled, and
/// on a busy machine one can come after part 2 was answered and part 3
/// began. Returns the outcome, the time from the cancel to the upload's
/// return, the operations, the multipart files the server held when
/// `AbortMultipartUpload` arrived (part 2 among them only if it was stored
/// before the abort) and the ones left after `drain`.
async fn cancel_after_part_2_was_handed_over(
    settle: Duration,
    rate: u64,
    drain: Drain,
) -> (UploadError, Duration, Vec<String>, Vec<String>, Vec<String>) {
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::Buffered(rate)).await;
    let s = store_with(
        &server,
        &link.endpoint,
        ClientOptions {
            part_settle_timeout: settle,
            ..options()
        },
    );
    let at_abort = Arc::new(Mutex::new(None::<Vec<String>>));
    let seen = Arc::clone(&at_abort);
    let root = server.root().to_path_buf();
    server.inject(
        "AbortMultipartUpload",
        None,
        1,
        Inject::Run(Arc::new(move || {
            *seen.lock().unwrap() = Some(multipart_files_in(&root));
        })),
    );
    let dir = TempDir::new("up-settle");
    let src = dir.file("src", 12 * MIB, 61);
    let control = Arc::new(TransferControl::new());
    let weak = Arc::downgrade(&control);
    let cancelled_at = Arc::new(Mutex::new(None::<Instant>));
    let at = Arc::clone(&cancelled_at);
    control.handed_over(2, move || {
        if let Some(c) = weak.upgrade()
            && c.cancel()
        {
            *at.lock().unwrap() = Some(Instant::now());
        }
    });
    let result = s
        .upload(BUCKET, "settle.bin", &src, &control, &mut |_| {})
        .await;
    let took = cancelled_at.lock().unwrap().expect("cancelled").elapsed();
    let e = expect_err(result);
    let at_abort = at_abort
        .lock()
        .unwrap()
        .take()
        .expect("the abort reached the server");
    // Let the link deliver whatever is still buffered (5 MiB at `rate`).
    match drain {
        Drain::No => {}
        Drain::Rate => {
            tokio::time::sleep(
                Duration::from_secs_f64(5.5 * MIB as f64 / rate as f64) + Duration::from_secs(1),
            )
            .await;
        }
        Drain::UntilPart2 => {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !server
                .open_multipart_files()
                .iter()
                .any(|f| f.ends_with(".part-2"))
                && Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    (
        e,
        took,
        server.ops(),
        at_abort,
        server.open_multipart_files(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_part_that_was_handed_over_is_answered_before_the_abort() {
    let (e, took, ops, at_abort, left) =
        cancel_after_part_2_was_handed_over(Duration::from_secs(5), 4 * MIB as u64, Drain::Rate)
            .await;
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    // It waited for part 2's answer: the part was stored before the abort
    // arrived, however much of it the link had forwarded before the
    // cancel (a busy machine hands it over slowly, so less is left to
    // wait for). And it returned once part 2 was answered, not at the end
    // of the bound.
    assert!(
        at_abort.iter().any(|f| f.ends_with(".part-2")),
        "part 2 was not stored when the abort arrived: {at_abort:?} {ops:?}"
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert_eq!(left, Vec::<String>::new(), "{ops:?}");
    assert_eq!(
        ops,
        [
            "HeadObject",
            "CreateMultipartUpload",
            "UploadPart#1",
            "UploadPart#2",
            "AbortMultipartUpload"
        ]
    );
}

/// The control: without the wait the abort overtakes the buffered part,
/// which the server then still stores (the race this fixes).
#[tokio::test(flavor = "multi_thread")]
async fn without_the_wait_a_handed_over_part_lands_after_the_abort() {
    let (e, took, ops, at_abort, left) =
        cancel_after_part_2_was_handed_over(Duration::ZERO, 4 * MIB as u64, Drain::UntilPart2)
            .await;
    assert_eq!(e.error, StorageError::Cancelled);
    assert!(took < Duration::from_millis(500), "{took:?}");
    assert!(
        !at_abort.iter().any(|f| f.ends_with(".part-2")),
        "part 2 was already stored when the abort arrived: {at_abort:?} {ops:?}"
    );
    assert!(
        left.iter().any(|f| f.ends_with(".part-2")),
        "expected the late part 2 to remain: {left:?} {ops:?}"
    );
}

/// The wait is bounded: a part that has not been answered when the bound
/// runs out (part 2 needs about 5 s through a 1 MiB/s link) is given up, the abort goes out, and the upload returns. (On
/// such a slow link the part may still land afterwards; the limitation is
/// documented.)
#[tokio::test(flavor = "multi_thread")]
async fn the_wait_for_a_handed_over_part_is_bounded() {
    let (e, took, _, _, _) =
        cancel_after_part_2_was_handed_over(Duration::from_secs(1), MIB as u64, Drain::No).await;
    assert_eq!(e.error, StorageError::Cancelled);
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(4),
        "{took:?}"
    );
}

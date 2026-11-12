//! Storage operations end to end against the in-process S3 server
//! (`test_server`): real HTTP, real SigV4 signing, real multipart uploads, no
//! network and no provider account.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::account::{AccountConfig, Accounts, MemoryAccountStore};
use super::credentials::{
    CredentialError, CredentialStore, Credentials, MemoryCredentialStore, Secret,
};
use super::s3::{ClientOptions, S3Store, StorageError, UploadPlan, Verification};
use super::test_server::{TempDir, TestServer};

const MIB: usize = 1024 * 1024;
const BUCKET: &str = "sotyware-test";

/// Fast failure for tests: no retries, short timeouts.
fn test_options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(2),
        read_timeout: Duration::from_secs(2),
        max_attempts: 1,
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        stall_timeout: Duration::from_secs(2),
        abort_timeout: Duration::from_secs(2),
    }
}

fn store(server: &TestServer) -> S3Store {
    S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        test_options(),
    )
}

async fn server() -> TestServer {
    TestServer::start("test-key", "test-secret", &[BUCKET, "other-bucket"]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_small_file_round_trips_with_its_checksum() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("small");
    let src = dir.file("note.txt", 100_000, 1);

    let up = s.upload_file(BUCKET, "poc/note.txt", &src).await.unwrap();
    assert_eq!((up.size, up.parts), (100_000, 1));

    let details = s
        .head_object(BUCKET, "poc/note.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(details.size, 100_000);
    assert_eq!(details.sha256.as_deref(), Some(up.sha256.as_str()));

    let listed = s.list_objects(BUCKET, "poc/").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].key.as_str(), listed[0].size),
        ("poc/note.txt", 100_000)
    );

    let dest = dir.path("back.txt");
    let down = s
        .download_file(BUCKET, "poc/note.txt", &dest)
        .await
        .unwrap();
    assert_eq!(down.verification, Verification::Sha256Matched);
    assert_eq!(down.sha256, up.sha256);
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap());

    s.delete_object(BUCKET, "poc/note.txt", up.version_id.as_deref())
        .await
        .unwrap();
    assert_eq!(s.head_object(BUCKET, "poc/note.txt").await.unwrap(), None);
    assert!(s.list_objects(BUCKET, "poc/").await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_file_is_uploaded_in_parts_and_verified() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("large");
    let size = 12 * MIB + 123;
    let src = dir.file("video.bin", size, 2);
    assert_eq!(
        super::s3::plan_upload(
            size as u64,
            &super::provider::S3_COMPATIBLE.limits,
            &test_options()
        ),
        Ok(UploadPlan::Multipart {
            part_size: 5 * MIB as u64,
            parts: 3
        })
    );

    let up = s.upload_file(BUCKET, "poc/video.bin", &src).await.unwrap();
    assert_eq!((up.size, up.parts), (size as u64, 3));

    let dest = dir.path("video-back.bin");
    let down = s
        .download_file(BUCKET, "poc/video.bin", &dest)
        .await
        .unwrap();
    assert_eq!(down.verification, Verification::Sha256Matched);
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn buckets_and_prefixes_are_listed() {
    let server = server().await;
    let s = store(&server);
    let mut buckets = s.list_buckets().await.unwrap();
    buckets.sort();
    assert_eq!(buckets, ["other-bucket", BUCKET]);

    let dir = TempDir::new("list");
    let src = dir.file("a", 10, 3);
    for key in ["a/1", "a/2", "b/1"] {
        s.upload_file(BUCKET, key, &src).await.unwrap();
    }
    let mut keys: Vec<_> = s
        .list_objects(BUCKET, "a/")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    keys.sort();
    assert_eq!(keys, ["a/1", "a/2"]);
    assert_eq!(s.list_objects(BUCKET, "").await.unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_credentials_are_reported_as_invalid_credentials() {
    let server = server().await;
    for (key, secret) in [("test-key", "wrong-secret"), ("unknown-key", "test-secret")] {
        let s = S3Store::new(
            server.resolved("local", &[BUCKET]),
            Credentials {
                access_key_id: key.into(),
                secret_access_key: Secret::new(secret),
            },
            test_options(),
        );
        let e = s.list_objects(BUCKET, "").await.unwrap_err();
        assert!(
            matches!(e, StorageError::InvalidCredentials(_)),
            "{key}: {e:?}"
        );
        assert!(!e.is_transient());
        assert!(!e.to_string().contains(secret), "{e}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_endpoint_is_a_transient_network_error() {
    // Bind and drop a listener so the port is free and nothing answers.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut c = AccountConfig::new("down", "Down", "s3-compatible", "us-east-1");
    c.endpoint = Some(format!("http://127.0.0.1:{port}"));
    // Windows retries a refused local connection for about 2 s before
    // reporting it; shorter timeouts would turn it into a timeout.
    let options = ClientOptions {
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_secs(10),
        ..test_options()
    };
    let s = S3Store::new(
        c.resolve().unwrap(),
        Credentials {
            access_key_id: "k".into(),
            secret_access_key: Secret::new("s"),
        },
        options,
    );
    let e = s.list_buckets().await.unwrap_err();
    assert!(matches!(e, StorageError::Network(_)), "{e:?}");
    assert!(e.is_transient());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_never_answers_times_out() {
    // Accepts connections and never responds.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let held = std::thread::spawn(move || {
        let conn = listener.accept();
        std::thread::sleep(Duration::from_secs(5));
        drop(conn);
    });
    let mut c = AccountConfig::new("silent", "Silent", "s3-compatible", "us-east-1");
    c.endpoint = Some(format!("http://{addr}"));
    let options = ClientOptions {
        read_timeout: Duration::from_millis(500),
        ..test_options()
    };
    let s = S3Store::new(
        c.resolve().unwrap(),
        Credentials {
            access_key_id: "k".into(),
            secret_access_key: Secret::new("s"),
        },
        options,
    );
    let started = Instant::now();
    let e = s.list_buckets().await.unwrap_err();
    assert!(
        matches!(e, StorageError::Timeout(_) | StorageError::Network(_)) && e.is_transient(),
        "{e:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    drop(held);
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupted_content_is_detected_and_never_written_to_the_destination() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("corrupt");
    let src = dir.file("doc.pdf", 50_000, 4);
    s.upload_file(BUCKET, "doc.pdf", &src).await.unwrap();

    // Flip one byte at rest, keeping the size.
    let at_rest = server.object_path(BUCKET, "doc.pdf");
    let mut data = fs::read(&at_rest).unwrap();
    data[1234] ^= 0xff;
    fs::write(&at_rest, data).unwrap();

    let dest = dir.path("doc-back.pdf");
    let e = s.download_file(BUCKET, "doc.pdf", &dest).await.unwrap_err();
    assert!(matches!(e, StorageError::Integrity(_)), "{e:?}");
    assert!(!dest.exists());
    // Only the source file: no temporary file is left.
    let names: Vec<_> = fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["doc.pdf"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_object_without_a_stored_checksum_is_checked_by_size_only() {
    let server = server().await;
    let s = store(&server);
    // Written straight into the server's folder: no sotyware metadata.
    fs::write(
        server.object_path(BUCKET, "foreign.txt"),
        b"written elsewhere",
    )
    .unwrap();
    let dir = TempDir::new("foreign");
    let dest = dir.path("foreign.txt");
    let down = s.download_file(BUCKET, "foreign.txt", &dest).await.unwrap();
    assert_eq!(down.verification, Verification::SizeOnly);
    assert_eq!(fs::read(&dest).unwrap(), b"written elsewhere");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_never_overwrites_an_existing_file() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("exists");
    let src = dir.file("a", 10, 5);
    s.upload_file(BUCKET, "a", &src).await.unwrap();
    let dest = dir.path("mine.txt");
    fs::write(&dest, b"my own file").unwrap();
    let e = s.download_file(BUCKET, "a", &dest).await.unwrap_err();
    assert!(
        matches!(
            e,
            StorageError::LocalWrite {
                kind: std::io::ErrorKind::AlreadyExists,
                ..
            }
        ),
        "{e:?}"
    );
    assert_eq!(fs::read(&dest).unwrap(), b"my own file");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_objects_and_buckets_are_not_found() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("missing");
    let e = s
        .download_file(BUCKET, "nope", &dir.path("nope"))
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::NotFound(_)), "{e:?}");
    assert!(!dir.path("nope").exists());
    assert_eq!(s.head_object(BUCKET, "nope").await.unwrap(), None);

    let any_bucket = S3Store::new(
        server.resolved("local", &[]),
        server.credentials(),
        test_options(),
    );
    let e = any_bucket
        .list_objects("no-such-bucket", "")
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::NotFound(_)), "{e:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn buckets_outside_the_account_configuration_are_refused_without_a_request() {
    let server = server().await;
    let s = store(&server); // configured for BUCKET only
    let dir = TempDir::new("refused");
    let src = dir.file("a", 10, 6);
    let e = s.upload_file("other-bucket", "a", &src).await.unwrap_err();
    assert!(matches!(e, StorageError::Refused(_)), "{e:?}");
    let e = s
        .delete_object("other-bucket", "a", None)
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::Refused(_)), "{e:?}");
    assert!(
        fs::read_dir(server.root().join("other-bucket"))
            .unwrap()
            .next()
            .is_none()
    );
    let e = s.upload_file(BUCKET, "", &src).await.unwrap_err();
    assert!(matches!(e, StorageError::Refused(_)), "{e:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_local_file_is_a_local_error() {
    let server = server().await;
    let s = store(&server);
    let dir = TempDir::new("nofile");
    let e = s
        .upload_file(BUCKET, "a", &dir.path("does-not-exist"))
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::Io(_)), "{e:?}");
}

fn accounts_for(servers: &[(&str, &TestServer)]) -> (Accounts, Arc<MemoryCredentialStore>) {
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Accounts::load(
        Box::<MemoryAccountStore>::default(),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        test_options(),
    )
    .unwrap();
    for (id, server) in servers {
        accounts
            .add(server.account(id, &[BUCKET]), &server.credentials())
            .unwrap();
    }
    (accounts, keys)
}

#[tokio::test(flavor = "multi_thread")]
async fn several_accounts_work_at_the_same_time_and_stay_separate() {
    let first = TestServer::start("key-one", "secret-one", &[BUCKET]).await;
    let second = TestServer::start("key-two", "secret-two", &[BUCKET]).await;
    let (accounts, _keys) = accounts_for(&[("first", &first), ("second", &second)]);
    let a = accounts.connect("first").unwrap();
    let b = accounts.connect("second").unwrap();

    let dir = TempDir::new("multi");
    let fa = dir.file("a.bin", 7 * MIB, 7); // multipart
    let fb = dir.file("b.bin", 300_000, 8);
    let (ra, rb) = tokio::join!(
        a.upload_file(BUCKET, "shared-key", &fa),
        b.upload_file(BUCKET, "shared-key", &fb)
    );
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    assert_eq!((ra.parts, rb.parts), (2, 1));

    // Same bucket name and key, different accounts: different objects.
    assert_eq!(
        fs::read(first.object_path(BUCKET, "shared-key")).unwrap(),
        fs::read(&fa).unwrap()
    );
    assert_eq!(
        fs::read(second.object_path(BUCKET, "shared-key")).unwrap(),
        fs::read(&fb).unwrap()
    );

    let (back_a, back_b) = (dir.path("a-back"), dir.path("b-back"));
    let (da, db) = tokio::join!(
        a.download_file(BUCKET, "shared-key", &back_a),
        b.download_file(BUCKET, "shared-key", &back_b)
    );
    assert_eq!(da.unwrap().sha256, ra.sha256);
    assert_eq!(db.unwrap().sha256, rb.sha256);

    // One account's key is not accepted by the other's provider.
    let crossed = S3Store::new(
        second.resolved("crossed", &[BUCKET]),
        first.credentials(),
        test_options(),
    );
    assert!(matches!(
        crossed.list_objects(BUCKET, "").await,
        Err(StorageError::InvalidCredentials(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn each_account_has_its_own_transfer_queue() {
    let first = TestServer::start("key-one", "secret-one", &[BUCKET]).await;
    let second = TestServer::start("key-two", "secret-two", &[BUCKET]).await;
    let (accounts, _keys) = accounts_for(&[("first", &first), ("second", &second)]);
    let mut one_slot = first.account("first", &[BUCKET]);
    one_slot.max_concurrent_transfers = 1;
    accounts.update(one_slot).unwrap();
    let a = accounts.connect("first").unwrap();
    let b = accounts.connect("second").unwrap();
    let dir = TempDir::new("queue");
    let file = dir.file("f", 1000, 9);

    // Occupy the first account's only slot.
    let held = a.transfer_queue().acquire().await.unwrap();
    let a2 = Arc::clone(&a);
    let file2 = file.clone();
    let waiting = tokio::spawn(async move { a2.upload_file(BUCKET, "queued", &file2).await });

    // The second account is not held up by the first one's full queue.
    tokio::time::timeout(Duration::from_secs(5), b.upload_file(BUCKET, "free", &file))
        .await
        .expect("the second account must not wait")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !waiting.is_finished(),
        "the queued upload must wait for the slot"
    );
    assert!(!first.object_path(BUCKET, "queued").exists());

    drop(held);
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(first.object_path(BUCKET, "queued").exists());
}

/// Local throughput, for the performance notes. Release mode:
/// `cargo test --release cloud::tests::local_throughput -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement, not a check"]
async fn local_throughput() {
    let server = server().await;
    let options = ClientOptions {
        max_attempts: 1,
        ..ClientOptions::default()
    };
    let s = S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options,
    );
    let dir = TempDir::new("perf");
    let size = 256 * MIB;
    let src = dir.file("big", size, 10);
    let t = Instant::now();
    let up = s.upload_file(BUCKET, "big", &src).await.unwrap();
    let up_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    s.download_file(BUCKET, "big", &dir.path("big-back"))
        .await
        .unwrap();
    let down_secs = t.elapsed().as_secs_f64();
    let mb = size as f64 / 1e6;
    println!(
        "upload {mb:.0} MB in {} parts: {up_secs:.2} s ({:.0} MB/s incl. hashing); download+verify: {down_secs:.2} s ({:.0} MB/s)",
        up.parts,
        mb / up_secs,
        mb / down_secs
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rotated_credentials_are_used_and_removal_leaves_remote_data() {
    let server = server().await;
    let (accounts, keys) = accounts_for(&[]);
    let wrong = Credentials {
        access_key_id: server.credentials().access_key_id,
        secret_access_key: Secret::new("not-the-secret"),
    };
    accounts
        .add(server.account("rotating", &[BUCKET]), &wrong)
        .unwrap();
    let e = accounts
        .connect("rotating")
        .unwrap()
        .list_objects(BUCKET, "")
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::InvalidCredentials(_)), "{e:?}");

    // The corrected key replaces the old one and the next connection uses it.
    accounts
        .set_credentials("rotating", &server.credentials())
        .unwrap();
    let s = accounts.connect("rotating").unwrap();
    let dir = TempDir::new("rotate");
    let file = dir.file("f", 1000, 11);
    s.upload_file(BUCKET, "kept", &file).await.unwrap();

    // Removing the account deletes its credentials, never its objects.
    drop(s);
    accounts.remove("rotating").unwrap();
    assert!(keys.ids().is_empty());
    assert!(matches!(
        keys.load("rotating"),
        Err(CredentialError::NotFound { .. })
    ));
    assert!(server.object_path(BUCKET, "kept").exists());
    let direct = S3Store::new(
        server.resolved("direct", &[BUCKET]),
        server.credentials(),
        test_options(),
    );
    assert_eq!(direct.list_objects(BUCKET, "").await.unwrap().len(), 1);
}

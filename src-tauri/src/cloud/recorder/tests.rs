//! Tests of the request recorder (ADR-024) against the in-process S3 server
//! (s3s + s3s-fs: real HTTP, SigV4, multipart), comparing what the
//! recorder wrote with the server's own request log. Run with
//! `cargo test --features request-recorder --target-dir target/recorder`.
//! No real provider.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::fs;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::s3::{
    ClientOptions, Existing, RemoteOutcome, S3Store, StorageError, TransferControl,
};
use super::super::test_server::{Inject, Pace, TempDir, TestServer, ThrottledLink};
use super::*;
use std::path::Path;

const KEY_ID: &str = "RecKeyIdSentinelQ7Z";
const SECRET: &str = "rec-Secret-Sentinel-Q7Z-0123456789";
const BUCKET: &str = "rec-bucket-sentinel-q7z";
const RUN: &str = "rec-test-run-0001";
const SCOPE: &str = "it/run-q7z/";
const ACCOUNT: &str = "rec-local";
const MIB: usize = 1024 * 1024;
/// Written into every uploaded test file, so the body would show if it
/// were ever recorded.
const BODY_SENTINEL: &[u8] = b"BODY-SENTINEL-Q7Z-";

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

// ------------------------------------------------------------- sinks

/// Collects the recording in memory; can fail after a number of bytes,
/// and can be slow.
#[derive(Clone, Default)]
struct MemSink {
    bytes: Arc<Mutex<Vec<u8>>>,
    fail_after: Option<usize>,
    delay: Duration,
}

impl Write for MemSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let mut bytes = self.bytes.lock().unwrap();
        if let Some(limit) = self.fail_after {
            let room = limit.saturating_sub(bytes.len());
            if room == 0 {
                return Err(io::Error::other("disk full (test)"));
            }
            let n = room.min(buf.len());
            bytes.extend_from_slice(&buf[..n]);
            return Ok(n);
        }
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink for MemSink {}

impl MemSink {
    fn text(&self) -> String {
        String::from_utf8(self.bytes.lock().unwrap().clone()).unwrap()
    }
}

fn settings(dir: &Path, account: Option<&str>) -> Settings {
    Settings {
        dir: dir.to_owned(),
        run: RUN.into(),
        scope: SCOPE.into(),
        account: account.map(str::to_owned),
    }
}

fn mem_recorder(sink: MemSink, capacity: usize) -> Arc<Recorder> {
    Recorder::with_sink(
        &settings(Path::new("/unused"), Some(ACCOUNT)),
        Box::new(sink),
        capacity,
    )
    .unwrap()
}

// ------------------------------------------------------- the recording

/// A parsed recording, checked for the rules every recording keeps:
/// every complete line is JSON, the first is `start`, event `seq` numbers
/// rise by one except where a `gap` line says which are missing, and
/// nothing follows `end`.
struct Recording {
    events: Vec<Value>,
    gaps: Vec<(u64, u64)>,
    end: Option<Value>,
    partial_tail: bool,
}

fn parse(text: &str) -> Recording {
    let partial_tail = !text.is_empty() && !text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.pop(); // the empty string after the last newline, or the partial tail
    let mut out = Recording {
        events: Vec::new(),
        gaps: Vec::new(),
        end: None,
        partial_tail,
    };
    let mut expected = 0u64;
    for (i, line) in lines.iter().enumerate() {
        let v: Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line {i} {line:?}: {e}"));
        assert!(out.end.is_none(), "a line after end: {line}");
        match v["kind"].as_str().unwrap() {
            "start" => {
                assert_eq!(i, 0, "start must be first");
                assert_eq!(v["run"], RUN);
                assert_eq!(v["recorder"], MARKER);
            }
            "gap" => {
                let from = v["missing_from"].as_u64().unwrap();
                let to = v["missing_to"].as_u64().unwrap();
                assert_eq!(from, expected, "{line}");
                assert!(to >= from);
                assert_eq!(v["count"].as_u64().unwrap(), to - from + 1);
                out.gaps.push((from, to));
                expected = to + 1;
            }
            "end" => out.end = Some(v),
            _ => {
                let seq = v["seq"].as_u64().unwrap();
                assert_eq!(seq, expected, "sequence numbers: {line}");
                expected = seq + 1;
                out.events.push(v);
            }
        }
    }
    if let Some(end) = &out.end {
        assert_eq!(end["events"].as_u64().unwrap(), expected, "{end}");
    }
    out
}

impl Recording {
    fn of_kind(&self, kind: &str) -> Vec<&Value> {
        self.events.iter().filter(|e| e["kind"] == kind).collect()
    }

    /// The attempts sent, as the server names them (`UploadPart#2`).
    fn sent(&self) -> Vec<String> {
        let parts: std::collections::HashMap<u64, u64> = self
            .of_kind("op-start")
            .iter()
            .filter_map(|e| Some((e["op"].as_u64()?, e["part"].as_u64()?)))
            .collect();
        self.of_kind("attempt-start")
            .iter()
            .map(|e| {
                let name = e["name"].as_str().unwrap();
                match parts.get(&e["op"].as_u64().unwrap()) {
                    Some(n) => format!("{name}#{n}"),
                    None => name.to_owned(),
                }
            })
            .collect()
    }

    /// Attempts that got an answer.
    fn answered(&self) -> Vec<String> {
        let sent = self.sent();
        let starts = self.of_kind("attempt-start");
        let ends = self.of_kind("attempt-end");
        starts
            .iter()
            .zip(sent)
            .filter(|(s, _)| {
                ends.iter().any(|e| {
                    e["op"] == s["op"] && e["attempt"] == s["attempt"] && e.get("status").is_some()
                })
            })
            .map(|(_, n)| n)
            .collect()
    }

    fn op(&self, op: u64) -> Vec<&Value> {
        self.events.iter().filter(|e| e["op"] == op).collect()
    }

    fn ops_named(&self, name: &str) -> Vec<u64> {
        self.of_kind("op-start")
            .iter()
            .filter(|e| e["name"] == name)
            .map(|e| e["op"].as_u64().unwrap())
            .collect()
    }

    /// Every operation has exactly one start and one end or drop, its
    /// attempts numbered 1, 2, … within it.
    fn check_lifecycles(&self) {
        for start in self.of_kind("op-start") {
            let op = start["op"].as_u64().unwrap();
            let lines = self.op(op);
            let finals: Vec<_> = lines
                .iter()
                .filter(|e| e["kind"] == "op-end" || e["kind"] == "op-dropped")
                .collect();
            assert_eq!(finals.len(), 1, "op {op}: {lines:#?}");
            assert_eq!(
                lines.last().unwrap()["kind"],
                finals[0]["kind"],
                "{lines:#?}"
            );
            let attempts: Vec<u64> = lines
                .iter()
                .filter(|e| e["kind"] == "attempt-start")
                .map(|e| e["attempt"].as_u64().unwrap())
                .collect();
            assert_eq!(
                attempts,
                (1..=attempts.len() as u64).collect::<Vec<_>>(),
                "{lines:#?}"
            );
        }
    }
}

/// The server's log against the recording: every attempt that got an
/// answer reached the server, and the server saw nothing the recorder did
/// not see being sent (as multisets, in order where requests were
/// sequential).
fn compare(server_ops: &[String], rec: &Recording) {
    let sent = rec.sent();
    let answered = rec.answered();
    let mut server = server_ops.to_vec();
    for a in &answered {
        let i = server
            .iter()
            .position(|s| s == a)
            .unwrap_or_else(|| panic!("{a} answered but not on the server: {server_ops:?}"));
        server.remove(i);
    }
    let mut unanswered = sent.clone();
    for a in &answered {
        if let Some(i) = unanswered.iter().position(|s| s == a) {
            unanswered.remove(i);
        }
    }
    for s in &server {
        let i = unanswered.iter().position(|u| u == s).unwrap_or_else(|| {
            panic!("the server saw {s} that the recorder did not send: {server_ops:?} vs {sent:?}")
        });
        unanswered.remove(i);
    }
}

async fn server() -> TestServer {
    TestServer::start(KEY_ID, SECRET, &[BUCKET]).await
}

fn store(server: &TestServer, rec: &Arc<Recorder>, options: ClientOptions) -> S3Store {
    store_at(server, &server.endpoint, rec, options, ACCOUNT)
}

fn store_at(
    server: &TestServer,
    endpoint: &str,
    rec: &Arc<Recorder>,
    options: ClientOptions,
    id: &str,
) -> S3Store {
    let mut account = server.account(id, &[BUCKET]);
    account.endpoint = Some(endpoint.to_owned());
    S3Store::new_recorded(
        account.resolve().unwrap(),
        server.credentials(),
        options,
        rec,
    )
}

/// A file whose bytes contain [`BODY_SENTINEL`] throughout.
fn body_file(dir: &TempDir, name: &str, size: usize) -> PathBuf {
    let data: Vec<u8> = BODY_SENTINEL.iter().copied().cycle().take(size).collect();
    let path = dir.path(name);
    fs::write(&path, data).unwrap();
    path
}

async fn upload(s: &S3Store, key: &str, path: &Path, control: &Arc<TransferControl>) -> bool {
    s.upload(BUCKET, key, path, control, &mut |_| {})
        .await
        .is_ok()
}

/// Dropped futures release their state on their own schedule.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn finish(rec: &Recorder, sink: &MemSink) -> (Health, Recording) {
    let health = rec.finish(Duration::from_secs(5));
    let text = sink.text();
    (health, parse(&text))
}

// ------------------------------------------------------------ settings

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

#[test]
fn without_any_variable_recording_is_off() {
    assert_eq!(Settings::from_lookup(lookup(&[])), Ok(None));
}

#[test]
fn partial_or_invalid_settings_fail_closed() {
    let dir = TempDir::new("rec-settings");
    let d = dir.0.to_str().unwrap().to_owned();
    let file = dir.file("f", 1, 1);
    let f = file.to_str().unwrap().to_owned();
    let ok = [(ENV_DIR, d.as_str()), (ENV_RUN, RUN), (ENV_SCOPE, SCOPE)];
    assert!(Settings::from_lookup(lookup(&ok)).unwrap().is_some());
    let bad: Vec<Vec<(&str, &str)>> = vec![
        vec![(ENV_DIR, &d)],
        vec![(ENV_RUN, RUN)],
        vec![(ENV_ACCOUNT, ACCOUNT)],
        vec![(ENV_DIR, &d), (ENV_RUN, RUN)],
        vec![
            (ENV_DIR, "relative/dir"),
            (ENV_RUN, RUN),
            (ENV_SCOPE, SCOPE),
        ],
        vec![
            (ENV_DIR, "/does/not/exist/q7z"),
            (ENV_RUN, RUN),
            (ENV_SCOPE, SCOPE),
        ],
        vec![(ENV_DIR, &f), (ENV_RUN, RUN), (ENV_SCOPE, SCOPE)],
        vec![(ENV_DIR, &d), (ENV_RUN, "short"), (ENV_SCOPE, SCOPE)],
        vec![
            (ENV_DIR, &d),
            (ENV_RUN, "Upper-Case-Run"),
            (ENV_SCOPE, SCOPE),
        ],
        vec![(ENV_DIR, &d), (ENV_RUN, "../../etc/x"), (ENV_SCOPE, SCOPE)],
        vec![(ENV_DIR, &d), (ENV_RUN, RUN), (ENV_SCOPE, "no-slash")],
        vec![(ENV_DIR, &d), (ENV_RUN, RUN), (ENV_SCOPE, "/abs/")],
        vec![(ENV_DIR, &d), (ENV_RUN, RUN), (ENV_SCOPE, "a\nb/")],
        vec![(ENV_DIR, &d), (ENV_RUN, RUN), (ENV_SCOPE, "")],
        vec![
            (ENV_DIR, &d),
            (ENV_RUN, RUN),
            (ENV_SCOPE, SCOPE),
            (ENV_ACCOUNT, "Not Kebab"),
        ],
    ];
    for pairs in bad {
        assert!(
            Settings::from_lookup(lookup(&pairs)).is_err(),
            "accepted {pairs:?}"
        );
    }
    // Nothing was created by validating.
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_as_the_directory_is_refused() {
    let dir = TempDir::new("rec-dir-link");
    let real = dir.path("real");
    fs::create_dir(&real).unwrap();
    let link = dir.path("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let pairs = [
        (ENV_DIR, link.to_str().unwrap()),
        (ENV_RUN, RUN),
        (ENV_SCOPE, SCOPE),
    ];
    assert!(Settings::from_lookup(lookup(&pairs)).is_err());
}

// --------------------------------------------------------- file handling

#[test]
fn start_creates_a_fresh_file_with_the_start_line() {
    let dir = TempDir::new("rec-start");
    let s = settings(&dir.0, None);
    let rec = Recorder::start(&s).unwrap();
    let health = rec.finish(Duration::from_secs(5));
    assert!(health.complete(), "{health:?}");
    let text = fs::read_to_string(s.file(std::process::id())).unwrap();
    let r = parse(&text);
    assert!(r.events.is_empty());
    assert_eq!(r.end.unwrap()["accounting"], "complete");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(s.file(std::process::id()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn an_existing_file_is_never_overwritten() {
    let dir = TempDir::new("rec-exists");
    let s = settings(&dir.0, None);
    fs::write(s.file(std::process::id()), b"earlier run").unwrap();
    assert!(Recorder::start(&s).is_err());
    assert_eq!(
        fs::read(s.file(std::process::id())).unwrap(),
        b"earlier run"
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_at_the_file_is_not_followed() {
    let dir = TempDir::new("rec-file-link");
    let s = settings(&dir.0, None);
    let target = dir.path("target");
    fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, s.file(std::process::id())).unwrap();
    assert!(Recorder::start(&s).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
    // A dangling link is refused as well (nothing is created through it).
    let gone = dir.path("gone");
    fs::remove_file(s.file(std::process::id())).unwrap();
    std::os::unix::fs::symlink(&gone, s.file(std::process::id())).unwrap();
    assert!(Recorder::start(&s).is_err());
    assert!(!gone.exists());
}

#[test]
fn a_missing_directory_starts_no_recorder() {
    let dir = TempDir::new("rec-missing");
    let s = settings(&dir.path("gone"), None);
    assert!(Recorder::start(&s).is_err());
    assert!(!dir.path("gone").exists());
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_starts_no_recorder() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("rec-readonly");
    let ro = dir.path("ro");
    fs::create_dir(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    let s = settings(&ro, None);
    let started = Recorder::start(&s);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    // Root may write anyway; the check means something only otherwise.
    if started.is_err() {
        assert_eq!(fs::read_dir(&ro).unwrap().count(), 0);
    }
}

#[test]
fn a_start_line_that_cannot_be_written_starts_no_recorder() {
    let sink = MemSink {
        fail_after: Some(0),
        ..MemSink::default()
    };
    assert!(Recorder::with_sink(&settings(Path::new("/x"), None), Box::new(sink), 8).is_err());
}

// ------------------------------------------------------------- disabled

#[tokio::test(flavor = "multi_thread")]
async fn a_store_without_a_recorder_has_no_recording_interceptor() {
    let server = server().await;
    if std::env::var_os(TEST_ALL).is_some() {
        // Every store is recorded in that mode: prove it is really on.
        let s = S3Store::new(
            server.resolved(ACCOUNT, &[BUCKET]),
            server.credentials(),
            options(),
        );
        assert!(
            s.sdk_client()
                .config()
                .interceptors()
                .any(|i| format!("{i:?}").contains("RequestRecorder"))
        );
        s.check_bucket(BUCKET).await.unwrap();
        assert!(installed().unwrap().health().events > 0);
        return;
    }
    let s = S3Store::new(
        server.resolved(ACCOUNT, &[BUCKET]),
        server.credentials(),
        options(),
    );
    assert!(installed().is_none());
    assert!(
        !s.sdk_client()
            .config()
            .interceptors()
            .any(|i| format!("{i:?}").contains("RequestRecorder"))
    );
    let rec = mem_recorder(MemSink::default(), 64);
    let r = store(&server, &rec, options());
    assert!(
        r.sdk_client()
            .config()
            .interceptors()
            .any(|i| format!("{i:?}").contains("RequestRecorder"))
    );
    // Same operations, same results, with and without.
    let dir = TempDir::new("rec-disabled");
    let src = body_file(&dir, "a", 1000);
    let control = Arc::new(TransferControl::new());
    assert!(upload(&s, "plain.bin", &src, &control).await);
    assert!(upload(&r, "recorded.bin", &src, &control).await);
    let ops = server.ops();
    let half = ops.len() / 2;
    assert_eq!(ops[..half], ops[half..], "{ops:?}");
}

// ---------------------------------------------- operations against s3s-fs

#[tokio::test(flavor = "multi_thread")]
async fn check_browse_uploads_and_download_match_the_server_log() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(&server, &rec, options());
    let dir = TempDir::new("rec-ops");
    let control = Arc::new(TransferControl::new());

    s.check_bucket(BUCKET).await.unwrap();
    s.list_folder(BUCKET, "", None, 1000).await.unwrap();
    s.list_folder(BUCKET, SCOPE, None, 1000).await.unwrap();
    let small = body_file(&dir, "small", 1000);
    assert!(upload(&s, &format!("{SCOPE}small.bin"), &small, &control).await);
    let big = body_file(&dir, "big", 12 * MIB);
    assert!(upload(&s, &format!("{SCOPE}big.bin"), &big, &control).await);
    let dest = dir.path("downloaded");
    s.download(
        BUCKET,
        &format!("{SCOPE}big.bin"),
        &dest,
        Existing::Refuse,
        &control,
        &mut |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&big).unwrap());
    settle().await;
    let server_ops = server.ops();
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    assert_eq!(r.end.as_ref().unwrap()["accounting"], "complete");
    assert!(r.gaps.is_empty());
    r.check_lifecycles();
    // Sequential, all answered: the very same list, in order.
    assert_eq!(r.sent(), server_ops);
    assert_eq!(r.answered(), server_ops);
    compare(&server_ops, &r);
    let names: Vec<&str> = r
        .of_kind("op-start")
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "HeadBucket",
            "ListObjectsV2",
            "ListObjectsV2",
            "HeadObject",
            "PutObject",
            "HeadObject",
            "HeadObject",
            "CreateMultipartUpload",
            "UploadPart",
            "UploadPart",
            "UploadPart",
            "HeadObject",
            "CompleteMultipartUpload",
            "HeadObject",
            "GetObject",
        ]
    );
    // Fields of each kind.
    let starts = r.of_kind("op-start");
    assert_eq!(starts[0]["bucket"], "configured");
    assert_eq!(starts[1]["prefix"], "root");
    assert_eq!(
        starts[2]["prefix"],
        format!("scope:{}", scoped_hash(RUN, "key", ""))
    );
    assert_eq!(starts[2]["delimiter"], true);
    assert_eq!(starts[2]["max_keys"], 1000);
    assert_eq!(
        starts[4]["key"],
        format!("scope:{}", scoped_hash(RUN, "key", "small.bin"))
    );
    let uploads: Vec<&Value> = starts
        .iter()
        .filter(|e| e["name"] == "UploadPart" || e["name"] == "CompleteMultipartUpload")
        .copied()
        .collect();
    assert!(uploads.iter().all(|e| e["upload"] == uploads[0]["upload"]));
    assert_eq!(uploads[0]["upload"].as_str().unwrap().len(), 24);
    assert_eq!(
        uploads
            .iter()
            .filter_map(|e| e["part"].as_u64())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(uploads[3]["parts"], 3);
    let put = r
        .of_kind("attempt-start")
        .into_iter()
        .find(|e| e["name"] == "PutObject")
        .unwrap();
    assert_eq!(put["content_length"], 1000);
    assert_eq!(put["sdk_op"], "PutObject");
    for end in r.of_kind("op-end") {
        assert_eq!(end["attempts"], 1, "{end}");
        assert_eq!(end["account"], ACCOUNT);
    }
    // The upload's first HEAD (not found) is a service error with a code.
    let head = r.of_kind("op-end")[3];
    assert_eq!(
        (&head["outcome"], &head["status"]),
        (&Value::from("service-error"), &Value::from(404))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_is_one_operation_with_two_numbered_attempts() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(
        &server,
        &rec,
        ClientOptions {
            max_attempts: 3,
            ..options()
        },
    );
    server.inject("HeadBucket", None, 1, Inject::InternalError);
    s.check_bucket(BUCKET).await.unwrap();
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    r.check_lifecycles();
    let kinds: Vec<(String, Option<u64>, Option<u64>)> = r
        .events
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_owned(),
                e["attempt"].as_u64(),
                e["status"].as_u64(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("op-start".into(), None, None),
            ("attempt-start".into(), Some(1), None),
            ("attempt-end".into(), Some(1), Some(500)),
            ("attempt-start".into(), Some(2), None),
            ("attempt-end".into(), Some(2), Some(200)),
            ("op-end".into(), None, Some(200)),
        ]
    );
    assert!(r.events.iter().all(|e| e["op"] == 0));
    assert_eq!(r.of_kind("op-end")[0]["attempts"], 2);
    assert_eq!(r.of_kind("attempt-end")[0]["outcome"], "service-error");
    assert_eq!(server.ops(), ["HeadBucket", "HeadBucket"]);
    compare(&server.ops(), &r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_request_has_its_error_code_and_no_retry() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(
        &server,
        &rec,
        ClientOptions {
            max_attempts: 3,
            ..options()
        },
    );
    server.inject("PutObject", None, 1, Inject::Deny);
    let dir = TempDir::new("rec-deny");
    let src = body_file(&dir, "a", 100);
    assert!(!upload(&s, "denied.bin", &src, &Arc::new(TransferControl::new())).await);
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    let put = r.ops_named("PutObject")[0];
    let end = r
        .op(put)
        .into_iter()
        .find(|e| e["kind"] == "op-end")
        .unwrap();
    assert_eq!(end["outcome"], "service-error");
    assert_eq!(end["code"], "AccessDenied");
    assert_eq!(end["status"], 403);
    assert_eq!(end["attempts"], 1);
    // The message ("injected by the test") is never recorded.
    assert!(!sink.text().contains("injected"));
    compare(&server.ops(), &r);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_endpoint_is_a_connector_failure_without_a_status() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store_at(
        &server,
        &format!("http://127.0.0.1:{port}"),
        &rec,
        options(),
        ACCOUNT,
    );
    assert!(s.check_bucket(BUCKET).await.is_err());
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    let end = r.of_kind("op-end")[0];
    assert_eq!(end["outcome"], "connector");
    assert!(end.get("status").is_none());
    let attempt = r.of_kind("attempt-end")[0];
    assert_eq!(attempt["sent"], true);
    assert!(attempt.get("status").is_none());
    assert!(r.answered().is_empty());
}

// --------------------------------------------------------- cancellation

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_part_records_the_dropped_part_and_the_abort() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(&server, &rec, options());
    let dir = TempDir::new("rec-cancel");
    let src = body_file(&dir, "big", 12 * MIB);
    let control = Arc::new(TransferControl::new());
    let weak = Arc::downgrade(&control);
    control.during_send(2, move || assert!(weak.upgrade().unwrap().cancel()));
    let e = s
        .upload(
            BUCKET,
            &format!("{SCOPE}big.bin"),
            &src,
            &control,
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.error, e.remote),
        (StorageError::Cancelled, RemoteOutcome::NothingStored)
    );
    settle().await;
    assert!(server.open_multipart_files().is_empty());
    let server_ops = server.ops();
    let (health, r) = finish(&rec, &sink);
    // A dropped operation ended, it is not open: the recording is complete.
    assert!(health.complete(), "{health:?}");
    assert_eq!(health.ops_dropped, 1);
    r.check_lifecycles();
    let part2 = r
        .of_kind("op-start")
        .into_iter()
        .find(|e| e["name"] == "UploadPart" && e["part"] == 2)
        .unwrap()["op"]
        .as_u64()
        .unwrap();
    let dropped = r.op(part2).last().copied().unwrap().clone();
    assert_eq!(dropped["kind"], "op-dropped");
    assert_eq!(dropped["in_flight"], true);
    assert_eq!(dropped["attempts"], 1);
    let abort = r.ops_named("AbortMultipartUpload");
    assert_eq!(abort.len(), 1);
    let abort_lines = r.op(abort[0]);
    assert_eq!(abort_lines.last().unwrap()["kind"], "op-end");
    assert_eq!(abort_lines.last().unwrap()["outcome"], "ok");
    assert_eq!(abort_lines[0]["upload"], r.op(part2)[0]["upload"]);
    // The drop is recorded before the abort starts.
    let seq_of = |e: &Value| e["seq"].as_u64().unwrap();
    assert!(seq_of(&dropped) < seq_of(abort_lines[0]));
    assert!(r.ops_named("CompleteMultipartUpload").is_empty());
    compare(&server_ops, &r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stall_timeout_shows_as_a_dropped_put() {
    let server = server().await;
    let link = ThrottledLink::start(&server.endpoint, Pace::StallAfter(256 * 1024)).await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store_at(
        &server,
        &link.endpoint,
        &rec,
        ClientOptions {
            multipart_threshold: 64 * MIB as u64,
            stall_timeout: Duration::from_millis(1500),
            ..options()
        },
        ACCOUNT,
    );
    let dir = TempDir::new("rec-stall");
    let src = body_file(&dir, "big", 24 * MIB);
    let e = s
        .upload(
            BUCKET,
            "stalled.bin",
            &src,
            &Arc::new(TransferControl::new()),
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert!(matches!(e.error, StorageError::Timeout(_)), "{e:?}");
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    let put = r.ops_named("PutObject")[0];
    let last = r.op(put).last().copied().unwrap().clone();
    assert_eq!(last["kind"], "op-dropped");
    assert_eq!(last["in_flight"], true);
    assert!(last["dur_us"].as_u64().unwrap() >= 1_400_000, "{last}");
    compare(&server.ops(), &r);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abort_past_its_timeout_shows_as_a_dropped_abort() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(
        &server,
        &rec,
        ClientOptions {
            abort_timeout: Duration::from_secs(1),
            ..options()
        },
    );
    let dir = TempDir::new("rec-abort-hangs");
    let src = body_file(&dir, "big", 12 * MIB);
    server.inject("UploadPart", Some(2), 1, Inject::Deny);
    server.inject("AbortMultipartUpload", None, 1, Inject::Hang);
    let e = s
        .upload(
            BUCKET,
            "orphaned.bin",
            &src,
            &Arc::new(TransferControl::new()),
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert!(
        matches!(e.remote, RemoteOutcome::IncompleteParts { .. }),
        "{e:?}"
    );
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    let abort = r.ops_named("AbortMultipartUpload")[0];
    let last = r.op(abort).last().copied().unwrap().clone();
    assert_eq!(last["kind"], "op-dropped");
    assert_eq!(last["in_flight"], true);
    let part2 = r
        .of_kind("op-start")
        .into_iter()
        .find(|e| e["name"] == "UploadPart" && e["part"] == 2)
        .unwrap()["op"]
        .as_u64()
        .unwrap();
    let end = r.op(part2).last().copied().unwrap().clone();
    // s3s refuses before reading the body, so the client may see the 403
    // or only the connection closing while it still sends (as in the
    // upload tests): either is recorded as such, never guessed.
    assert_eq!(end["kind"], "op-end");
    assert!(
        (end["outcome"] == "service-error" && end["code"] == "AccessDenied")
            || (end["outcome"] == "connector" && end.get("status").is_none()),
        "{end}"
    );
    compare(&server.ops(), &r);
}

// ------------------------------------------------------------ redaction

#[tokio::test(flavor = "multi_thread")]
async fn nothing_secret_or_identifying_is_recorded() {
    let server = server().await;
    let dir = TempDir::new("rec-redaction");
    let s_dir = dir.path("out");
    fs::create_dir(&s_dir).unwrap();
    let settings = settings(&s_dir, Some(ACCOUNT));
    let rec = Recorder::start(&settings).unwrap();
    let s = store(&server, &rec, options());
    let control = Arc::new(TransferControl::new());
    let inside = format!("{SCOPE}Report-QZXV9 private.bin");
    let outside = "private/Outside-QZXV9.txt";
    let src = body_file(&dir, "a", 7 * MIB);
    assert!(upload(&s, &inside, &src, &control).await);
    assert!(upload(&s, outside, &src, &control).await);
    s.list_folder(BUCKET, "it/", None, 10).await.unwrap();
    s.list_folder(BUCKET, "private/", None, 10).await.unwrap();
    // Generated keys inside the scope.
    let generated: Vec<String> = (0..20)
        .map(|i| format!("{SCOPE}GEN{i:02}X{}", "QZXWV".repeat(i % 3 + 1)))
        .collect();
    for k in &generated {
        let _ = s.head_object(BUCKET, k).await;
    }
    // A bucket outside the account's configuration is refused before any
    // request; another account's bucket name never appears either.
    settle().await;
    rec.finish(Duration::from_secs(5));
    let text = fs::read_to_string(settings.file(std::process::id())).unwrap();
    let r = parse(&text);
    let sha = super::super::test_server::sha256_hex(&fs::read(&src).unwrap());
    for needle in [
        KEY_ID,
        SECRET,
        BUCKET,
        "rec-bucket",
        SCOPE,
        "Report",
        "QZXV9",
        "Outside",
        "QZXWV",
        "GEN0",
        "private",
        "BODY-SENTINEL",
        "Q7Z",
        "uthorization",
        "AWS4-HMAC",
        "x-amz-meta",
        "sotyware-sha256",
        "Signature",
        "Credential=",
        &sha,
        &server.endpoint,
        "127.0.0.1",
    ] {
        assert!(!text.contains(needle), "{needle:?} in the recording");
    }
    // Hashes the harness can recompute, and what is outside the scope.
    let keys: Vec<&str> = r
        .of_kind("op-start")
        .iter()
        .filter_map(|e| e["key"].as_str())
        .collect();
    let inside_hash = format!(
        "scope:{}",
        scoped_hash(RUN, "key", "Report-QZXV9 private.bin")
    );
    assert!(keys.contains(&inside_hash.as_str()), "{keys:?}");
    assert!(keys.contains(&"outside-scope"));
    for k in &generated {
        let h = format!(
            "scope:{}",
            scoped_hash(RUN, "key", k.strip_prefix(SCOPE).unwrap())
        );
        assert!(keys.contains(&h.as_str()), "{h}");
    }
    let prefixes: Vec<&str> = r
        .of_kind("op-start")
        .iter()
        .filter_map(|e| e["prefix"].as_str())
        .collect();
    assert_eq!(prefixes, ["scope-ancestor", "outside-scope"]);
    // Every value is from a short allow-list of forms.
    for e in &r.events {
        for (k, v) in e.as_object().unwrap() {
            assert!(
                [
                    "seq",
                    "t_us",
                    "kind",
                    "op",
                    "name",
                    "account",
                    "bucket",
                    "key",
                    "prefix",
                    "delimiter",
                    "continuation",
                    "max_keys",
                    "version",
                    "range",
                    "upload",
                    "part",
                    "parts",
                    "attempt",
                    "sdk_op",
                    "content_length",
                    "sent",
                    "dur_us",
                    "status",
                    "outcome",
                    "code",
                    "attempts",
                    "in_flight",
                    "last_status",
                ]
                .contains(&k.as_str()),
                "unexpected field {k}: {v}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn version_ids_are_hashed_with_the_run() {
    // s3s-fs does not version; check the hashing with a made-up id the way
    // the interceptor applies it to `x-amz-version-id`.
    let rec = mem_recorder(MemSink::default(), 8);
    let v = rec.id("version-id", Some("4_z123abc_version")).unwrap();
    assert_eq!(v, scoped_hash(RUN, "version-id", "4_z123abc_version"));
    assert_ne!(
        scoped_hash(RUN, "version-id", "x"),
        scoped_hash(RUN, "upload-id", "x")
    );
    assert_ne!(
        scoped_hash(RUN, "key", "x"),
        scoped_hash("other-run-0001", "key", "x")
    );
    assert_eq!(scoped_hash(RUN, "key", "x").len(), 24);
}

// --------------------------------------------------------- concurrency

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_requests_on_two_accounts_keep_whole_ordered_lines() {
    let one = server().await;
    let two = TestServer::start(KEY_ID, SECRET, &[BUCKET]).await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let a = Arc::new(store(&one, &rec, options()));
    let b = Arc::new(store_at(
        &two,
        &two.endpoint,
        &rec,
        options(),
        "second-account",
    ));
    let mut tasks = Vec::new();
    for i in 0..64 {
        let s = Arc::clone(if i % 2 == 0 { &a } else { &b });
        tasks.push(tokio::spawn(async move {
            s.head_object(BUCKET, &format!("{SCOPE}k{i}"))
                .await
                .unwrap();
            s.check_bucket(BUCKET).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.complete(), "{health:?}");
    r.check_lifecycles();
    assert_eq!(r.of_kind("op-start").len(), 128);
    let ids: std::collections::HashSet<u64> = r
        .of_kind("op-start")
        .iter()
        .map(|e| e["op"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), 128);
    let accounts: std::collections::HashSet<&str> = r
        .events
        .iter()
        .map(|e| e["account"].as_str().unwrap())
        .collect();
    assert_eq!(accounts, [ACCOUNT, "other-1"].into_iter().collect());
    let mut all = one.ops();
    all.extend(two.ops());
    compare(&all, &r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_queue_drops_events_and_the_gaps_say_exactly_which() {
    let server = server().await;
    let sink = MemSink {
        delay: Duration::from_millis(15),
        ..MemSink::default()
    };
    let rec = mem_recorder(sink.clone(), 2);
    let s = Arc::new(store(&server, &rec, options()));
    let mut tasks = Vec::new();
    for i in 0..40 {
        let s = Arc::clone(&s);
        tasks.push(tokio::spawn(async move {
            s.head_object(BUCKET, &format!("k{i}")).await.unwrap();
        }));
    }
    let started = Instant::now();
    for t in tasks {
        t.await.unwrap();
    }
    // The slow writer never held the requests up.
    assert!(started.elapsed() < Duration::from_secs(5));
    settle().await;
    let (health, r) = finish(&rec, &sink);
    assert!(health.queue_full > 0, "{health:?}");
    assert!(!health.complete());
    let missing: u64 = r.gaps.iter().map(|(a, b)| b - a + 1).sum();
    assert_eq!(missing, health.queue_full);
    assert_eq!(r.events.len() as u64 + missing, health.events);
    let end = r.end.unwrap();
    assert_eq!(end["accounting"], "incomplete");
    assert_eq!(end["queue_full"].as_u64().unwrap(), health.queue_full);
    assert!(
        end["reasons"]
            .as_array()
            .unwrap()
            .contains(&"queue-full".into())
    );
    assert_eq!(server.ops().len(), 40);
}

// ------------------------------------------------------ recorder failure

#[tokio::test(flavor = "multi_thread")]
async fn a_writer_failing_midway_changes_nothing_for_the_transfers() {
    let server = server().await;
    let sink = MemSink {
        fail_after: Some(1500),
        ..MemSink::default()
    };
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(&server, &rec, options());
    let dir = TempDir::new("rec-write-fail");
    let src = body_file(&dir, "big", 12 * MIB);
    let control = Arc::new(TransferControl::new());
    assert!(upload(&s, "big.bin", &src, &control).await);
    let dest = dir.path("back");
    s.download(
        BUCKET,
        "big.bin",
        &dest,
        Existing::Refuse,
        &control,
        &mut |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap());
    // Cancellation and the abort still work while the recorder is broken.
    let cancel = Arc::new(TransferControl::new());
    let weak = Arc::downgrade(&cancel);
    cancel.during_send(2, move || assert!(weak.upgrade().unwrap().cancel()));
    let e = s
        .upload(BUCKET, "cancelled.bin", &src, &cancel, &mut |_| {})
        .await
        .unwrap_err();
    assert_eq!(e.error, StorageError::Cancelled);
    settle().await;
    assert!(server.open_multipart_files().is_empty());
    let health = rec.finish(Duration::from_secs(5));
    assert_eq!(health.write_errors, 1, "{health:?}");
    assert!(!health.end_written);
    assert!(!health.complete());
    let text = sink.text();
    assert_eq!(text.len(), 1500);
    let r = parse(&text);
    assert!(r.partial_tail && r.end.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_writer_thread_changes_nothing_for_the_transfers() {
    let server = server().await;
    let sink = MemSink::default();
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(&server, &rec, options());
    rec.kill_writer();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let dir = TempDir::new("rec-dead");
    let src = body_file(&dir, "big", 12 * MIB);
    let control = Arc::new(TransferControl::new());
    let weak = Arc::downgrade(&control);
    control.during_send(2, move || assert!(weak.upgrade().unwrap().cancel()));
    let e = s
        .upload(BUCKET, "cancelled.bin", &src, &control, &mut |_| {})
        .await
        .unwrap_err();
    assert_eq!(e.error, StorageError::Cancelled);
    s.check_bucket(BUCKET).await.unwrap();
    settle().await;
    assert!(server.open_multipart_files().is_empty());
    let at = Instant::now();
    let health = rec.finish(Duration::from_secs(5));
    // The writer is gone: finishing does not wait for it.
    assert!(at.elapsed() < Duration::from_secs(1));
    assert!(health.writer_gone > 0, "{health:?}");
    assert!(!health.end_written && !health.complete());
    assert!(parse(&sink.text()).end.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn finishing_with_events_still_queued_waits_only_its_limit() {
    let server = server().await;
    let sink = MemSink {
        delay: Duration::from_millis(50),
        ..MemSink::default()
    };
    let rec = mem_recorder(sink.clone(), QUEUE_CAPACITY);
    let s = store(&server, &rec, options());
    for i in 0..20 {
        s.head_object(BUCKET, &format!("k{i}")).await.unwrap();
    }
    let at = Instant::now();
    let health = rec.finish(Duration::from_millis(200));
    let took = at.elapsed();
    assert!(took < Duration::from_millis(600), "{took:?}");
    assert!(!health.end_written && !health.complete(), "{health:?}");
    // Events after the end are counted, never written, never block.
    s.check_bucket(BUCKET).await.unwrap();
    assert!(rec.health().after_finish > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn hooks_never_fail_a_request_whatever_the_recorder_state() {
    let server = server().await;
    // A queue of one and a sink that fails soon after the start line.
    let sink = MemSink {
        fail_after: Some(400),
        ..MemSink::default()
    };
    let rec = mem_recorder(sink, 1);
    let s = store(
        &server,
        &rec,
        ClientOptions {
            max_attempts: 3,
            ..options()
        },
    );
    server.inject("HeadBucket", None, 2, Inject::InternalError);
    s.check_bucket(BUCKET).await.unwrap();
    rec.kill_writer();
    for _ in 0..10 {
        s.check_bucket(BUCKET).await.unwrap();
    }
    rec.finish(Duration::from_millis(100));
    for _ in 0..10 {
        s.check_bucket(BUCKET).await.unwrap();
    }
    assert_eq!(server.ops().len(), 23);
}

#[test]
fn health_reasons_cover_every_kind_of_loss() {
    let mut h = Health {
        events: 3,
        written: 3,
        ops_started: 1,
        ops_ended: 1,
        end_written: true,
        ..Health::default()
    };
    assert!(h.complete());
    h.end_written = false;
    assert!(!h.complete());
    h.end_written = true;
    h.ops_started = 2;
    assert_eq!(h.reasons(), ["open-operations"]);
    h.ops_dropped = 1;
    assert!(h.complete());
    h.written = 2;
    assert_eq!(h.reasons(), ["unwritten-events"]);
}

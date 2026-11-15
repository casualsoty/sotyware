//! Tests of the operation journal through the real transfer and operation
//! code, and of the recovery commands, against the in-process S3 servers
//! (s3s-fs for multipart uploads, the in-memory versioned server for file
//! operations). A "crash" is simulated by dropping the running future, so
//! no cleanup of the running session takes place, exactly as when the
//! process is killed. No real provider.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::operations::{
    CloudOperations, ConflictPolicy, EventBody, ItemRef, OperationEvent, OperationRequest,
    OperationSink, OperationState, OperationSummary,
};
use super::super::s3::{ClientOptions, Existing, StorageError, TransferControl};
use super::super::test_server::{
    Fault, Inject, TempDir, TestObject, TestServer, Versioned, sha256_hex,
};
use super::*;
use crate::features::StoreError;
use crate::journal::JournalStore;
use crate::storage::SqliteJournal;

const KEY_ID: &str = "recovery-key";
const SECRET: &str = "recovery-Secret-0123456789";
const BUCKET: &str = "recovery-bucket";
const MIB: usize = 1024 * 1024;

fn options() -> ClientOptions {
    ClientOptions {
        max_attempts: 1,
        connect_timeout: Duration::from_secs(5),
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        abort_timeout: Duration::from_secs(5),
        ..ClientOptions::default()
    }
}

fn accounts_for(server: &TestServer) -> Arc<CloudAccounts> {
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        options(),
    ));
    let id = accounts
        .add(
            &AccountInput {
                label: "Local".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(server.endpoint.clone()),
                bucket: BUCKET.into(),
            },
            CredentialInput::new(KEY_ID, SECRET),
        )
        .unwrap()
        .id;
    assert_eq!(id, "local");
    accounts
}

fn journal_in(dir: &TempDir) -> Arc<Journal> {
    Journal::load(
        SqliteJournal::open(&dir.path("journal.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn JournalStore>),
    )
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(20), "waited for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_multipart_upload_is_listed_inspected_and_aborted_by_its_recorded_id() {
    let server = TestServer::start(KEY_ID, SECRET, &[BUCKET]).await;
    let accounts = accounts_for(&server);
    let dir = TempDir::new("rec-mp");
    let journal = journal_in(&dir);
    let src = dir.file("big.bin", 12 * MIB, 3);
    // Part 2 never gets an answer; the session "crashes" meanwhile.
    server.inject("UploadPart", Some(2), 1, Inject::Hang);
    let store = Arc::new(S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    ));
    let control = Arc::new(TransferControl::new());
    control.record_in(Arc::clone(&journal));
    let task = {
        let (store, control, src) = (Arc::clone(&store), Arc::clone(&control), src.clone());
        tokio::spawn(async move {
            store
                .upload(BUCKET, "big.bin", &src, &control, &mut |_| {})
                .await
        })
    };
    wait_for("the multipart id in the journal", || {
        journal
            .entries()
            .unwrap()
            .first()
            .is_some_and(|e| e.state == "multipart-open")
    })
    .await;
    task.abort(); // the process ends: no abort, no journal update
    let _ = task.await;
    assert!(
        !server.open_multipart_files().is_empty(),
        "the upload is still open"
    );

    // After "restart": the entry is there, with the file name but no path.
    let list = list(&journal, &accounts).unwrap();
    assert_eq!(list.unavailable, None);
    assert_eq!(list.items.len(), 1);
    let item = &list.items[0];
    assert_eq!(
        (item.kind, item.state.as_str(), item.key.as_deref()),
        ("upload", "multipart-open", Some("big.bin"))
    );
    assert_eq!(item.file_name.as_deref(), Some("big.bin"));
    assert_eq!(item.account_label.as_deref(), Some("Local"));
    assert!(item.multipart);
    let shown = serde_json::to_string(&list).unwrap();
    assert!(
        !shown.contains(&*dir.0.to_string_lossy()),
        "no local path reaches the page: {shown}"
    );

    let insp = inspect(Arc::clone(&journal), Arc::clone(&accounts), item.id)
        .await
        .unwrap();
    assert_eq!(insp.verdict, "needs-attention");
    assert_eq!(insp.counts.get("absent"), Some(&1));
    assert_eq!(insp.open_uploads.len(), 1);
    assert!(insp.open_uploads[0].recorded);
    let upload_id = insp.open_uploads[0].upload_id.clone();

    // The bucket-wide listing marks it as sotyware's own.
    let all = incomplete(
        Arc::clone(&journal),
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
    )
    .await
    .unwrap();
    assert_eq!(all.uploads.len(), 1);
    assert!(all.uploads[0].recorded && !all.more);

    let result = abort(
        Arc::clone(&journal),
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "big.bin".into(),
        upload_id,
    )
    .await
    .unwrap();
    assert_eq!(result, AbortResult::Aborted);
    assert!(server.open_multipart_files().is_empty());
    assert!(
        list_of(&journal, &accounts).is_empty(),
        "the entry is resolved"
    );
    // Nothing else was asked of the provider: no object was written.
    assert_eq!(server.ops().iter().filter(|o| *o == "PutObject").count(), 0);
}

fn list_of(journal: &Journal, accounts: &CloudAccounts) -> Vec<RecoveryItem> {
    list(journal, accounts).unwrap().items
}

fn upload_entry(key: &str, data: &[u8], multipart: bool) -> EntryRow {
    EntryRow {
        kind: "upload".into(),
        state: "started".into(),
        account_id: Some("local".into()),
        bucket: Some(BUCKET.into()),
        object_key: Some(key.into()),
        local_path: Some(format!("/somewhere/{key}")),
        size: Some(data.len() as u64),
        sha256: Some(sha256_hex(data)),
        multipart,
        ..EntryRow::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_are_judged_by_what_the_provider_holds() {
    let server = TestServer::start(KEY_ID, SECRET, &[BUCKET]).await;
    let accounts = accounts_for(&server);
    let dir = TempDir::new("rec-judge");
    let journal = journal_in(&dir);
    let store = S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    );
    // 1. Stored and verified before the crash.
    let ours = dir.file("ours.bin", 3000, 1);
    store.upload_file(BUCKET, "ours.bin", &ours).await.unwrap();
    let ours_id = journal
        .begin(upload_entry(
            "ours.bin",
            &std::fs::read(&ours).unwrap(),
            false,
        ))
        .unwrap()
        .unwrap();
    // 2. Never reached the provider.
    let absent_id = journal
        .begin(upload_entry("absent.bin", b"never sent", false))
        .unwrap()
        .unwrap();
    // 3. Another writer's object under the same key.
    let theirs = dir.file("theirs.bin", 100, 9);
    store
        .upload_file(BUCKET, "taken.bin", &theirs)
        .await
        .unwrap();
    let taken_id = journal
        .begin(upload_entry("taken.bin", b"what this upload had", false))
        .unwrap()
        .unwrap();
    let before = server.ops().len();

    let insp = |id| inspect(Arc::clone(&journal), Arc::clone(&accounts), id);
    let a = insp(ours_id).await.unwrap();
    assert_eq!(
        (a.verdict, a.counts.get("stored-verified")),
        ("complete", Some(&1))
    );
    let b = insp(absent_id).await.unwrap();
    assert_eq!(
        (b.verdict, b.counts.get("absent")),
        ("nothing-stored", Some(&1))
    );
    let c = insp(taken_id).await.unwrap();
    assert_eq!(
        (c.verdict, c.counts.get("different")),
        ("needs-attention", Some(&1))
    );

    // Inspection only reads: three HEADs, nothing else.
    let ops = server.ops()[before..].to_vec();
    assert_eq!(ops, ["HeadObject", "HeadObject", "HeadObject"]);
    assert_eq!(
        std::fs::read(server.object_path(BUCKET, "taken.bin")).unwrap(),
        std::fs::read(&theirs).unwrap(),
        "the other writer's object is untouched"
    );
    for id in [ours_id, absent_id, taken_id] {
        dismiss(&journal, id).unwrap();
    }
    assert!(list_of(&journal, &accounts).is_empty());
}

/// Records operation events.
#[derive(Default)]
struct Sink(Mutex<Vec<OperationEvent>>);

impl OperationSink for Sink {
    fn emit(&self, event: OperationEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Sink {
    async fn finished(&self, op: &str) -> OperationSummary {
        let t = Instant::now();
        loop {
            let found = self.0.lock().unwrap().iter().find_map(|e| match &e.body {
                EventBody::Finished { summary } if e.operation == op => Some(summary.clone()),
                _ => None,
            });
            if let Some(s) = found {
                return s;
            }
            assert!(
                t.elapsed() < Duration::from_secs(20),
                "operation {op} did not finish"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn versioned() -> (TestServer, Versioned, Arc<CloudAccounts>) {
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    let accounts = accounts_for(&server);
    (server, data, accounts)
}

async fn run_move(
    ops: &Arc<CloudOperations>,
    accounts: &Arc<CloudAccounts>,
    sink: &Arc<Sink>,
    keys: &[&str],
) -> OperationSummary {
    let plan = Arc::clone(ops)
        .plan(
            Arc::clone(accounts),
            "local".into(),
            BUCKET.into(),
            OperationRequest::Move {
                items: keys
                    .iter()
                    .map(|k| ItemRef::File { key: (*k).into() })
                    .collect(),
                destination: "to/".into(),
            },
        )
        .await
        .unwrap();
    let started = Arc::clone(ops)
        .start(
            Arc::clone(accounts),
            &plan.plan,
            ConflictPolicy::Skip,
            Arc::clone(sink) as Arc<dyn OperationSink>,
        )
        .await
        .unwrap();
    sink.finished(&started.operation).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_move_stays_in_the_journal_and_inspection_names_the_kept_originals() {
    let (server, data, accounts) = versioned().await;
    let dir = TempDir::new("rec-move");
    let journal = journal_in(&dir);
    data.seed(BUCKET, "a.txt", b"content of a");
    data.seed(BUCKET, "b.txt", b"content of b");
    let ops = Arc::new(CloudOperations::new());
    ops.record_in(Arc::clone(&journal));
    let sink = Arc::new(Sink::default());
    // The first delete is refused: a.txt is copied, its original kept, and
    // the operation stops before b.txt.
    server.inject("DeleteObject", None, 1, Inject::Deny);
    let summary = run_move(&ops, &accounts, &sink, &["a.txt", "b.txt"]).await;
    assert_eq!(summary.state, OperationState::Failed);

    let items = list_of(&journal, &accounts);
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(
        (
            item.kind,
            item.state.as_str(),
            item.operation.as_deref(),
            item.steps
        ),
        ("operation", "partial", Some("move"), 2)
    );
    let steps = journal.steps(item.id).unwrap();
    assert_eq!(
        steps.iter().map(|s| s.phase.as_str()).collect::<Vec<_>>(),
        ["original-kept", "planned"]
    );
    assert!(
        item.summary.contains("1 copied with the original kept"),
        "{}",
        item.summary
    );

    let versions_before = data.lock().buckets[BUCKET].keys.clone().len();
    let insp = inspect(Arc::clone(&journal), Arc::clone(&accounts), item.id)
        .await
        .unwrap();
    assert_eq!(insp.verdict, "needs-attention");
    assert_eq!(insp.counts.get("original-kept"), Some(&1));
    assert_eq!(insp.counts.get("not-started"), Some(&1));
    assert_eq!(insp.keys["original-kept"], ["a.txt"]);
    assert!(
        insp.summary
            .contains("1 copied with the original still in place")
    );
    // Inspection changed nothing.
    assert_eq!(data.lock().buckets[BUCKET].keys.len(), versions_before);
    assert!(data.lock().removed_versions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_completed_operation_leaves_nothing_in_the_journal() {
    let (_server, data, accounts) = versioned().await;
    let dir = TempDir::new("rec-done");
    let journal = journal_in(&dir);
    data.seed(BUCKET, "a.txt", b"content of a");
    let ops = Arc::new(CloudOperations::new());
    ops.record_in(Arc::clone(&journal));
    let sink = Arc::new(Sink::default());
    let summary = run_move(&ops, &accounts, &sink, &["a.txt"]).await;
    assert_eq!(summary.state, OperationState::Completed);
    assert!(list_of(&journal, &accounts).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_records_its_temporary_file_and_a_crash_leaves_it_for_the_next_start() {
    let data = super::super::test_server::pattern(40 * 64 * 1024, 5);
    let server = TestServer::start_objects(
        KEY_ID,
        SECRET,
        &[(
            BUCKET,
            TestObject::new("slow.bin", data.clone())
                .with_fault(Fault::Slow(Duration::from_millis(20))),
        )],
    )
    .await;
    let dir = TempDir::new("rec-dl");
    let journal = journal_in(&dir);
    let store = Arc::new(S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    ));
    // A completed download leaves no entry.
    let control = TransferControl::new();
    control.record_in(Arc::clone(&journal));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (j, s) = (Arc::clone(&journal), Arc::clone(&seen));
    store
        .download(
            BUCKET,
            "slow.bin",
            &dir.path("done.bin"),
            Existing::Refuse,
            &control,
            &mut move |_, _| {
                let e = j.entries().unwrap();
                s.lock().unwrap().push(e.len());
            },
        )
        .await
        .unwrap();
    assert!(
        seen.lock().unwrap().iter().all(|n| *n == 1),
        "recorded while it ran"
    );
    assert!(journal.entries().unwrap().is_empty());
    assert_eq!(std::fs::read(dir.path("done.bin")).unwrap(), data);

    // A download cut short by a crash leaves its file and its entry.
    let control = Arc::new(TransferControl::new());
    control.record_in(Arc::clone(&journal));
    let task = {
        let (store, control, dest) = (
            Arc::clone(&store),
            Arc::clone(&control),
            dir.path("cut.bin"),
        );
        tokio::spawn(async move {
            store
                .download(
                    BUCKET,
                    "slow.bin",
                    &dest,
                    Existing::Refuse,
                    &control,
                    &mut |_, _| {},
                )
                .await
        })
    };
    wait_for("some bytes in the temporary file", || {
        control
            .partial_file()
            .and_then(|p| std::fs::metadata(p).ok())
            .is_some_and(|m| m.len() > 0)
    })
    .await;
    let partial = control.partial_file().unwrap();
    task.abort();
    let _ = task.await;
    assert!(partial.exists());
    assert_eq!(journal.entries().unwrap().len(), 1);
    // The next start removes it, and only it.
    assert_eq!(journal.clean_local(), 1);
    assert!(!partial.exists());
    assert!(journal.entries().unwrap().is_empty());
    assert!(dir.path("done.bin").exists());
    assert!(!dir.path("cut.bin").exists());
}

/// A journal store whose writes fail (a full disk).
struct Failing;

impl JournalStore for Failing {
    fn insert(&mut self, _: &EntryRow) -> Result<i64, StoreError> {
        Err(StoreError::new("disk full"))
    }
    fn update(&mut self, _: &EntryRow) -> Result<(), StoreError> {
        Err(StoreError::new("disk full"))
    }
    fn remove(&mut self, _: i64) -> Result<(), StoreError> {
        Ok(())
    }
    fn entries(&mut self) -> Result<Vec<EntryRow>, StoreError> {
        Ok(Vec::new())
    }
    fn insert_steps(&mut self, _: i64, _: &[StepRow]) -> Result<(), StoreError> {
        Err(StoreError::new("disk full"))
    }
    fn set_phases(&mut self, _: i64, _: &[(i64, &str)]) -> Result<(), StoreError> {
        Err(StoreError::new("disk full"))
    }
    fn steps(&mut self, _: i64) -> Result<Vec<StepRow>, StoreError> {
        Ok(Vec::new())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn work_that_cannot_be_recorded_does_not_start() {
    let server = TestServer::start(KEY_ID, SECRET, &[BUCKET]).await;
    let dir = TempDir::new("rec-fail");
    let journal = Journal::load(Ok(Box::new(Failing)));
    let store = S3Store::new(
        server.resolved("local", &[BUCKET]),
        server.credentials(),
        options(),
    );
    let src = dir.file("a.bin", 1000, 2);
    let control = Arc::new(TransferControl::new());
    control.record_in(Arc::clone(&journal));
    let e = store
        .upload(BUCKET, "a.bin", &src, &control, &mut |_| {})
        .await
        .unwrap_err();
    assert!(matches!(e.error, StorageError::Journal(_)), "{e:?}");
    assert_eq!(e.remote, super::super::s3::RemoteOutcome::NothingStored);
    let ops = server.ops();
    assert!(
        !ops.iter().any(|o| o == "PutObject" || o == "HeadObject"),
        "nothing was sent: {ops:?}"
    );

    // A download refuses before writing anything, too.
    let control = TransferControl::new();
    control.record_in(Arc::clone(&journal));
    let dest = dir.path("d.bin");
    let e = store
        .download(
            BUCKET,
            "a.bin",
            &dest,
            Existing::Refuse,
            &control,
            &mut |_, _| {},
        )
        .await
        .unwrap_err();
    assert!(matches!(e, StorageError::Journal(_)), "{e:?}");
    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        1,
        "no temporary file is left"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operation_that_cannot_be_recorded_changes_nothing() {
    let (_server, data, accounts) = versioned().await;
    data.seed(BUCKET, "a.txt", b"content of a");
    let ops = Arc::new(CloudOperations::new());
    ops.record_in(Journal::load(Ok(Box::new(Failing))));
    let sink = Arc::new(Sink::default());
    let summary = run_move(&ops, &accounts, &sink, &["a.txt"]).await;
    assert_eq!(summary.state, OperationState::Failed);
    assert_eq!(summary.error.as_ref().unwrap().kind, ErrorKind::Journal);
    let current: Vec<String> = data.lock().current_objects(BUCKET).into_keys().collect();
    assert_eq!(current, ["a.txt"], "nothing was copied or deleted");
}

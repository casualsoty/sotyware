//! Tests of the file-management operations against the in-memory versioned
//! S3 server (real HTTP and SigV4 through s3s; S3 versioning semantics as
//! modelled by `test_server::Versioned`), with faults injected per
//! operation and hooks before each item. No real provider.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::s3::ClientOptions;
use super::super::test_server::{Inject, TestServer, Versioned};
use super::*;

const KEY_ID: &str = "operations-key";
const SECRET: &str = "operations-Secret-0123456789";
const BUCKET: &str = "ops-bucket";

#[derive(Default)]
struct Recorder(Mutex<Vec<OperationEvent>>);

impl OperationSink for Recorder {
    fn emit(&self, event: OperationEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn of(&self, op: &str) -> Vec<OperationEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.operation == op)
            .cloned()
            .collect()
    }
}

struct Setup {
    server: TestServer,
    data: Versioned,
    accounts: Arc<CloudAccounts>,
    ops: Arc<CloudOperations>,
    sink: Arc<Recorder>,
}

fn options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(30),
        max_attempts: 1,
        ..ClientOptions::default()
    }
}

async fn setup(versioning: bool) -> Setup {
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, versioning)]).await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
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
    Setup {
        server,
        data,
        accounts,
        ops: Arc::new(CloudOperations::new()),
        sink: Arc::new(Recorder::default()),
    }
}

fn file(key: &str) -> ItemRef {
    ItemRef::File { key: key.into() }
}

fn folder(prefix: &str) -> ItemRef {
    ItemRef::Folder {
        prefix: prefix.into(),
    }
}

impl Setup {
    fn seed(&self, keys: &[&str]) {
        for k in keys {
            self.data
                .seed(BUCKET, k, format!("content of {k}").as_bytes());
        }
    }

    fn seed_with(&self, key: &str, data: &[u8], content_type: &str, meta: &[(&str, &str)]) {
        self.data.lock().put(
            BUCKET,
            key,
            data.to_vec(),
            Some(content_type.into()),
            meta.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        );
    }

    fn current(&self) -> Vec<String> {
        self.data
            .lock()
            .current_objects(BUCKET)
            .into_keys()
            .collect()
    }

    fn versions_of(&self, key: &str) -> Vec<(String, bool)> {
        let st = self.data.lock();
        st.buckets[BUCKET]
            .keys
            .get(key)
            .map(|l| l.iter().map(|v| (v.id.clone(), v.marker)).collect())
            .unwrap_or_default()
    }

    async fn plan(&self, request: OperationRequest) -> Result<PlanSummary, CommandError> {
        Arc::clone(&self.ops)
            .plan(
                Arc::clone(&self.accounts),
                "local".into(),
                BUCKET.into(),
                request,
            )
            .await
    }

    async fn start(&self, plan: &PlanSummary, policy: ConflictPolicy) -> OperationStarted {
        Arc::clone(&self.ops)
            .start(
                Arc::clone(&self.accounts),
                &plan.plan,
                policy,
                Arc::clone(&self.sink) as Arc<dyn OperationSink>,
            )
            .await
            .unwrap()
    }

    /// Waits for the final event and checks the rules every operation keeps:
    /// the slot is free, `finished` comes once and last, `seq` counts 1, 2,
    /// 3, …, every item is final, nothing carries the key or secret.
    async fn finished(&self, op: &str) -> (OperationSummary, Vec<ItemReport>) {
        let t = Instant::now();
        let summary = loop {
            let found = self.sink.of(op).into_iter().find_map(|e| match e.body {
                EventBody::Finished { summary } => Some(summary),
                _ => None,
            });
            if let Some(s) = found {
                assert!(!self.ops.busy(), "the slot is still taken");
                break s;
            }
            assert!(t.elapsed() < Duration::from_secs(60), "{op} did not finish");
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        let events = self.sink.of(op);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.body, EventBody::Finished { .. }))
                .count(),
            1
        );
        assert!(matches!(
            events.last().unwrap().body,
            EventBody::Finished { .. }
        ));
        let mut items: HashMap<usize, ItemReport> = HashMap::new();
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, i as u64 + 1);
            let json = serde_json::to_string(e).unwrap();
            assert!(!json.contains(SECRET) && !json.contains(KEY_ID), "{json}");
            if let EventBody::Item { item } = &e.body {
                items.insert(item.index, item.clone());
            }
        }
        let mut items: Vec<ItemReport> = items.into_values().collect();
        items.sort_by_key(|i| i.index);
        for i in &items {
            assert!(i.state.is_final(), "{i:?}");
        }
        (summary, items)
    }

    async fn run(
        &self,
        request: OperationRequest,
        policy: ConflictPolicy,
    ) -> (PlanSummary, OperationSummary, Vec<ItemReport>) {
        let plan = self.plan(request).await.unwrap();
        let started = self.start(&plan, policy).await;
        let (summary, items) = self.finished(&started.operation).await;
        (plan, summary, items)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn copying_a_file_keeps_content_type_metadata_and_the_original() {
    let s = setup(true).await;
    s.seed_with(
        "docs/a.txt",
        b"hello",
        "text/plain",
        &[("sotyware-sha256", "abc"), ("x", "1")],
    );
    let (plan, summary, items) = s
        .run(
            OperationRequest::Copy {
                items: vec![file("docs/a.txt")],
                destination: "other/".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(plan.objects, 1);
    assert_eq!(summary.state, OperationState::Completed, "{summary:?}");
    assert_eq!(items[0].destination.as_deref(), Some("other/a.txt"));
    assert_eq!(s.current(), vec!["docs/a.txt", "other/a.txt"]);
    let st = s.data.lock();
    let copy = st.buckets[BUCKET].current("other/a.txt").unwrap();
    assert_eq!(copy.content_type.as_deref(), Some("text/plain"));
    assert_eq!(
        copy.metadata.get("sotyware-sha256").map(String::as_str),
        Some("abc")
    );
    assert_eq!(copy.metadata.get("x").map(String::as_str), Some("1"));
    assert_eq!(&**copy.data, b"hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflict_is_shown_skipped_or_renamed_never_overwritten() {
    let s = setup(true).await;
    s.seed(&["a.txt", "d/a.txt"]);
    let before = s.data.lock().buckets[BUCKET]
        .current("d/a.txt")
        .unwrap()
        .etag
        .clone();
    let request = OperationRequest::Copy {
        items: vec![file("a.txt")],
        destination: "d/".into(),
    };
    let plan = s.plan(request.clone()).await.unwrap();
    assert_eq!(plan.conflicts, 1);
    let c = plan.items[0].conflict.clone().unwrap();
    assert_eq!(c.existing, "d/a.txt");
    assert_eq!(c.alternative.as_deref(), Some("d/a (1).txt"));
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(summary.items.skipped, 1);
    assert_eq!(items[0].note, Some(ItemNote::DestinationExists));
    assert_eq!(
        s.data.lock().buckets[BUCKET]
            .current("d/a.txt")
            .unwrap()
            .etag,
        before
    );
    let (_, summary, items) = s.run(request, ConflictPolicy::Rename).await;
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(items[0].destination.as_deref(), Some("d/a (1).txt"));
    assert_eq!(s.current(), vec!["a.txt", "d/a (1).txt", "d/a.txt"]);
    assert_eq!(
        s.data.lock().buckets[BUCKET]
            .current("d/a.txt")
            .unwrap()
            .etag,
        before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn copying_into_the_same_folder_makes_a_numbered_copy() {
    let s = setup(false).await;
    s.seed(&["d/report.pdf", "d/report (1).pdf"]);
    let (plan, summary, _) = s
        .run(
            OperationRequest::Copy {
                items: vec![file("d/report.pdf")],
                destination: "d/".into(),
            },
            ConflictPolicy::Rename,
        )
        .await;
    assert_eq!(
        plan.items[0]
            .conflict
            .as_ref()
            .unwrap()
            .alternative
            .as_deref(),
        Some("d/report (2).pdf")
    );
    assert_eq!(summary.state, OperationState::Completed);
    assert!(s.current().contains(&"d/report (2).pdf".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_a_file_hides_the_original_on_a_versioned_bucket_and_removes_it_otherwise() {
    for versioning in [true, false] {
        let s = setup(versioning).await;
        s.seed(&["a.txt"]);
        let (plan, summary, _) = s
            .run(
                OperationRequest::Move {
                    items: vec![file("a.txt")],
                    destination: "b/".into(),
                },
                ConflictPolicy::Skip,
            )
            .await;
        assert_eq!(
            plan.versioning,
            if versioning {
                VersioningInfo::Enabled
            } else {
                VersioningInfo::Disabled
            }
        );
        assert_eq!(summary.state, OperationState::Completed);
        assert_eq!(s.current(), vec!["b/a.txt"]);
        let v = s.versions_of("a.txt");
        if versioning {
            assert_eq!(v.len(), 2, "{v:?}");
            assert!(v[1].1, "the latest is a delete marker");
        } else {
            assert!(v.is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_file_and_refusing_an_existing_name() {
    let s = setup(true).await;
    s.seed(&["f/a.txt", "f/b.txt"]);
    let e = s
        .plan(OperationRequest::Rename {
            item: file("f/a.txt"),
            new_name: "b.txt".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::DestinationExists);
    for bad in ["", "x/y", ".."] {
        let e = s
            .plan(OperationRequest::Rename {
                item: file("f/a.txt"),
                new_name: bad.into(),
            })
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput, "{bad:?}");
    }
    let (_, summary, items) = s
        .run(
            OperationRequest::Rename {
                item: file("f/a.txt"),
                new_name: "größe +c.txt".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(items[0].destination.as_deref(), Some("f/größe +c.txt"));
    assert_eq!(s.current(), vec!["f/b.txt", "f/größe +c.txt"]);
}

// No '?' in these keys: s3s (the test server) decodes `x-amz-copy-source`
// before splitting off `?versionId`, so it cannot copy a key containing '?'
// (S3 decodes after splitting; sotyware encodes '?' as %3F either way).
const TREE: [&str; 7] = [
    "photos/",
    "photos/a.jpg",
    "photos/sub/b.jpg",
    "photos/sub//odd key #%+&.txt",
    "photos/../dots",
    "photos-backup/keep.jpg",
    "photosx",
];

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_folder_moves_every_object_and_nothing_next_to_it() {
    let s = setup(true).await;
    s.seed(&TREE);
    let (plan, summary, items) = s
        .run(
            OperationRequest::Rename {
                item: folder("photos/"),
                new_name: "pics".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(
        plan.items[0].objects, 5,
        "marker and four objects, not the neighbours"
    );
    assert_eq!(summary.state, OperationState::Completed, "{items:?}");
    assert_eq!(
        s.current(),
        vec![
            "photos-backup/keep.jpg",
            "photosx",
            "pics/",
            "pics/../dots",
            "pics/a.jpg",
            "pics/sub//odd key #%+&.txt",
            "pics/sub/b.jpg",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_cannot_go_into_itself_and_moving_in_place_is_refused() {
    let s = setup(true).await;
    s.seed(&TREE);
    let plan = s
        .plan(OperationRequest::Move {
            items: vec![folder("photos/")],
            destination: "photos/sub/".into(),
        })
        .await
        .unwrap();
    assert!(
        plan.items[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("inside itself")
    );
    assert_eq!(plan.objects, 0);
    let plan = s
        .plan(OperationRequest::Move {
            items: vec![file("photosx")],
            destination: "".into(),
        })
        .await
        .unwrap();
    assert!(plan.items[0].problem.is_some());
    let e = Arc::clone(&s.ops)
        .start(
            Arc::clone(&s.accounts),
            &plan.plan,
            ConflictPolicy::Skip,
            Arc::clone(&s.sink) as Arc<dyn OperationSink>,
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_folder_hides_exactly_its_objects_and_reports_them() {
    let s = setup(true).await;
    s.seed(&TREE);
    let (plan, summary, items) = s
        .run(
            OperationRequest::Delete {
                items: vec![folder("photos/")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(plan.objects, 5);
    assert_eq!(plan.versioning, VersioningInfo::Enabled);
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!((items[0].hidden, items[0].removed), (5, 0));
    assert_eq!(s.current(), vec!["photos-backup/keep.jpg", "photosx"]);
    // Nothing permanently deleted: every version is still there.
    assert_eq!(s.versions_of("photos/a.jpg").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_without_versioning_says_the_objects_are_gone() {
    let s = setup(false).await;
    s.seed(&["x/1", "x/2", "y"]);
    let (plan, summary, items) = s
        .run(
            OperationRequest::Delete {
                items: vec![folder("x/"), file("y"), file("x/1")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(plan.items.len(), 2, "x/1 is part of x/");
    assert_eq!(plan.versioning, VersioningInfo::Disabled);
    assert_eq!(summary.removed, 3);
    assert_eq!(summary.hidden, 0);
    assert_eq!(
        items.iter().map(|i| i.state).collect::<Vec<_>>(),
        vec![ItemState::Completed; 2]
    );
    assert!(s.current().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_changed_after_review_is_not_copied_and_no_original_is_deleted() {
    let s = setup(true).await;
    s.seed(&["m/1", "m/2", "m/3"]);
    let plan = s
        .plan(OperationRequest::Move {
            items: vec![folder("m/")],
            destination: "n/".into(),
        })
        .await
        .unwrap();
    let data = s.data.clone();
    s.ops.before_item(move |_| {
        data.seed(BUCKET, "m/2", b"changed by another client");
    });
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(summary.state, OperationState::Partial, "{summary:?}");
    assert_eq!(items[0].state, ItemState::Partial);
    assert_eq!(items[0].note, Some(ItemNote::OriginalKeptCopyIncomplete));
    assert_eq!(items[0].failures[0].error.kind, ErrorKind::RemoteChanged);
    // Every original is still current; m/2 is not copied.
    let cur = s.current();
    for k in ["m/1", "m/2", "m/3"] {
        assert!(cur.contains(&k.to_owned()), "{cur:?}");
    }
    assert!(!cur.contains(&"n/2".to_owned()));
    assert!(!summary.retryable, "a changed object needs a new review");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_delete_of_a_move_keeps_the_original_and_retry_finishes_it() {
    let s = setup(true).await;
    s.seed(&["a.txt", "b.txt"]);
    s.server.inject("DeleteObject", None, 1, Inject::Deny);
    let (_, summary, items) = s
        .run(
            OperationRequest::Move {
                items: vec![file("a.txt"), file("b.txt")],
                destination: "to/".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(
        summary.state,
        OperationState::Failed,
        "access denied stops the operation"
    );
    assert_eq!(
        summary.error.as_ref().unwrap().kind,
        ErrorKind::AccessDenied
    );
    assert_eq!(items[0].state, ItemState::Partial);
    assert_eq!(items[0].note, Some(ItemNote::CopiedOriginalKept));
    assert_eq!(items[1].state, ItemState::Cancelled);
    assert!(summary.retryable);
    assert_eq!(s.current(), vec!["a.txt", "b.txt", "to/a.txt"]);
    let started = Arc::clone(&s.ops)
        .retry(
            Arc::clone(&s.accounts),
            &summary.operation,
            Arc::clone(&s.sink) as Arc<dyn OperationSink>,
        )
        .await
        .unwrap();
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(summary.state, OperationState::Completed, "{items:?}");
    assert_eq!(s.current(), vec!["to/a.txt", "to/b.txt"]);
    // The retried move deleted a.txt once only, with no second copy.
    assert_eq!(s.versions_of("to/a.txt").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_stopped_by_a_server_error_is_retried_without_copying_twice() {
    let s = setup(true).await;
    s.seed(&["c/1", "c/2", "c/3", "c/4", "c/5", "c/6"]);
    s.server
        .inject("CopyObject", None, 1, Inject::InternalError);
    let (_, summary, _) = s
        .run(
            OperationRequest::Copy {
                items: vec![folder("c/")],
                destination: "d/".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(summary.state, OperationState::Failed);
    assert!(summary.retryable);
    let started = Arc::clone(&s.ops)
        .retry(
            Arc::clone(&s.accounts),
            &summary.operation,
            Arc::clone(&s.sink) as Arc<dyn OperationSink>,
        )
        .await
        .unwrap();
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(items[0].destination.as_deref(), Some("d/c/"));
    assert_eq!(summary.state, OperationState::Completed);
    for n in 1..=6 {
        assert_eq!(s.versions_of(&format!("d/c/{n}")).len(), 1, "d/c/{n}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_stops_before_the_next_item_and_keeps_move_originals() {
    let s = setup(true).await;
    s.seed(&["1", "2", "3"]);
    let plan = s
        .plan(OperationRequest::Move {
            items: vec![file("1"), file("2"), file("3")],
            destination: "z/".into(),
        })
        .await
        .unwrap();
    let ops = Arc::clone(&s.ops);
    let id: Arc<Mutex<String>> = Arc::default();
    let seen = Arc::clone(&id);
    s.ops.before_item(move |i| {
        if i == 1 {
            ops.cancel(&seen.lock().unwrap());
        }
    });
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    *id.lock().unwrap() = started.operation.clone();
    let (summary, items) = s.finished(&started.operation).await;
    // The hook may run before the id is known: then nothing was cancelled.
    if summary.state == OperationState::Cancelled {
        assert_eq!(items[0].state, ItemState::Completed);
        assert_eq!(items[1].state, ItemState::Cancelled);
        assert_eq!(items[2].state, ItemState::Cancelled);
        assert_eq!(s.current(), vec!["2", "3", "z/1"]);
        assert!(summary.retryable);
    } else {
        assert_eq!(summary.state, OperationState::Completed);
    }
    assert_eq!(s.ops.cancel("op-unknown"), CancelOutcome::NotActive);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_after_review_is_not_deleted() {
    let s = setup(false).await;
    s.seed(&["keep.txt"]);
    let plan = s
        .plan(OperationRequest::Delete {
            items: vec![file("keep.txt")],
        })
        .await
        .unwrap();
    s.data.seed(BUCKET, "keep.txt", b"newer content");
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(items[0].state, ItemState::Failed);
    assert_eq!(items[0].failures[0].error.kind, ErrorKind::RemoteChanged);
    assert_eq!(summary.state, OperationState::Partial);
    assert_eq!(s.current(), vec!["keep.txt"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_deleted_file_removes_exactly_its_delete_markers() {
    let s = setup(true).await;
    s.seed(&["r.txt", "other.txt"]);
    let _ = s
        .run(
            OperationRequest::Delete {
                items: vec![file("r.txt")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    let _ = s
        .run(
            OperationRequest::Delete {
                items: vec![file("other.txt")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    let marker = s.versions_of("r.txt").last().unwrap().0.clone();
    // A stale marker id (deleted again since) is refused.
    let plan = s
        .plan(OperationRequest::Restore {
            items: vec![RestoreRef {
                key: "r.txt".into(),
                marker_version: "v999999".into(),
            }],
        })
        .await
        .unwrap();
    assert!(plan.items[0].problem.is_some());
    let (_, summary, _) = s
        .run(
            OperationRequest::Restore {
                items: vec![RestoreRef {
                    key: "r.txt".into(),
                    marker_version: marker,
                }],
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(s.current(), vec!["r.txt"]);
    assert_eq!(
        s.versions_of("r.txt").len(),
        1,
        "only the marker was removed"
    );
    assert!(
        s.versions_of("other.txt").last().unwrap().1,
        "other.txt stays deleted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_folder_is_enumerated_across_pages_and_deleted_completely() {
    let s = setup(false).await;
    {
        let mut st = s.data.lock();
        for n in 0..2_150 {
            st.put(
                BUCKET,
                &format!("big/{n:05}"),
                vec![1],
                None,
                HashMap::new(),
            );
        }
        st.put(BUCKET, "big-neighbour", vec![1], None, HashMap::new());
    }
    let (plan, summary, items) = s
        .run(
            OperationRequest::Delete {
                items: vec![folder("big/")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(plan.objects, 2_150);
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(items[0].objects_done, 2_150);
    assert_eq!(s.current(), vec!["big-neighbour"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn plans_expire_once_used_or_replaced() {
    let s = setup(true).await;
    s.seed(&["a", "b"]);
    let first = s
        .plan(OperationRequest::Delete {
            items: vec![file("a")],
        })
        .await
        .unwrap();
    let second = s
        .plan(OperationRequest::Delete {
            items: vec![file("b")],
        })
        .await
        .unwrap();
    let start = |p: String| {
        let (ops, accounts, sink) = (
            Arc::clone(&s.ops),
            Arc::clone(&s.accounts),
            Arc::clone(&s.sink),
        );
        async move {
            ops.start(
                accounts,
                &p,
                ConflictPolicy::Skip,
                sink as Arc<dyn OperationSink>,
            )
            .await
        }
    };
    assert_eq!(
        start(first.plan.clone()).await.unwrap_err().kind,
        ErrorKind::OutdatedPlan
    );
    let started = start(second.plan.clone()).await.unwrap();
    let _ = s.finished(&started.operation).await;
    assert_eq!(
        start(second.plan.clone()).await.unwrap_err().kind,
        ErrorKind::OutdatedPlan
    );
    assert_eq!(s.current(), vec!["a"]);
    let third = s
        .plan(OperationRequest::Delete {
            items: vec![file("a")],
        })
        .await
        .unwrap();
    s.ops.discard(&third.plan);
    assert_eq!(
        start(third.plan).await.unwrap_err().kind,
        ErrorKind::OutdatedPlan
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_requests_are_refused_before_any_request() {
    let s = setup(true).await;
    let cases = [
        OperationRequest::Delete { items: vec![] },
        OperationRequest::Delete {
            items: vec![folder("")],
        },
        OperationRequest::Delete {
            items: vec![folder("no-slash")],
        },
        OperationRequest::Delete {
            items: vec![file("ends/")],
        },
        OperationRequest::Copy {
            items: vec![file("a")],
            destination: "no-slash".into(),
        },
    ];
    for c in cases {
        assert_eq!(
            s.plan(c.clone()).await.unwrap_err().kind,
            ErrorKind::InvalidInput,
            "{c:?}"
        );
    }
    assert!(s.server.ops().is_empty(), "{:?}", s.server.ops());
    // The bucket must be the account's.
    let e = Arc::clone(&s.ops)
        .plan(
            Arc::clone(&s.accounts),
            "local".into(),
            "other-bucket".into(),
            OperationRequest::Delete {
                items: vec![file("a")],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::OutdatedPage);
}

/// Whether the running operation has been asked to stop: what `run` reads
/// right after the item hook returns.
fn stop_requested(ops: &CloudOperations) -> bool {
    matches!(&ops.lock().slot, Slot::Active { control, .. } if control.cancelled())
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_cancels_the_running_operation_and_refuses_new_ones() {
    let s = setup(true).await;
    s.seed(&["1", "2", "3", "4"]);
    let plan = s
        .plan(OperationRequest::Copy {
            items: vec![file("1"), file("2"), file("3"), file("4")],
            destination: "c/".into(),
        })
        .await
        .unwrap();
    let ops = Arc::clone(&s.ops);
    let asked: Arc<AtomicBool> = Arc::default();
    let seen = Arc::clone(&asked);
    s.ops.before_item(move |i| {
        if i == 1 {
            let quitting = Arc::clone(&ops);
            // Quitting from another thread while the operation runs. Item 1
            // goes on only once the shutdown has asked the operation to
            // stop, however late that thread gets a CPU (a fixed sleep here
            // lost that race on a busy runner).
            std::thread::spawn(move || quitting.shutdown(Duration::from_secs(10)));
            let t = Instant::now();
            while !stop_requested(&ops) && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(1));
            }
            seen.store(stop_requested(&ops), Ordering::SeqCst);
        }
    });
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert!(
        asked.load(Ordering::SeqCst),
        "the shutdown did not ask the operation to stop within 5 s"
    );
    assert_eq!(summary.state, OperationState::Cancelled);
    assert_eq!(items[0].state, ItemState::Completed);
    assert!(
        items[1..].iter().all(|i| i.state == ItemState::Cancelled),
        "{items:?}"
    );
    let e = s
        .plan(OperationRequest::Delete {
            items: vec![file("1")],
        })
        .await
        .unwrap();
    let e = Arc::clone(&s.ops)
        .start(
            Arc::clone(&s.accounts),
            &e.plan,
            ConflictPolicy::Skip,
            Arc::clone(&s.sink) as Arc<dyn OperationSink>,
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
}

#[test]
fn plans_and_events_serialize_as_the_page_expects() {
    let item = ItemReport {
        index: 0,
        name: "a".into(),
        folder: false,
        source: "a".into(),
        destination: None,
        state: ItemState::Partial,
        objects_total: 1,
        objects_done: 0,
        objects_failed: 1,
        bytes_total: 1,
        bytes_done: 0,
        hidden: 0,
        removed: 0,
        note: Some(ItemNote::CopiedOriginalKept),
        failures: vec![],
    };
    let e = OperationEvent {
        operation: "op-1".into(),
        seq: 3,
        body: EventBody::Item { item },
    };
    let json = serde_json::to_value(&e).unwrap();
    assert_eq!(json["body"]["type"], "item");
    assert_eq!(json["body"]["item"]["state"], "partial");
    assert_eq!(json["body"]["item"]["note"], "copied-original-kept");
    assert_eq!(json["body"]["item"]["objectsFailed"], 1);
    let p = OperationEvent {
        operation: "op-1".into(),
        seq: 4,
        body: EventBody::Progress {
            objects_done: 1,
            objects_total: 2,
            bytes_done: 3,
            bytes_total: 4,
        },
    };
    assert_eq!(serde_json::to_value(&p).unwrap()["body"]["objectsTotal"], 2);
    let r: OperationRequest = serde_json::from_value(serde_json::json!({
        "kind": "rename", "item": {"type": "folder", "prefix": "a/"}, "newName": "b"
    }))
    .unwrap();
    assert_eq!(
        r,
        OperationRequest::Rename {
            item: folder("a/"),
            new_name: "b".into()
        }
    );
    let r: OperationRequest = serde_json::from_value(serde_json::json!({
        "kind": "restore", "items": [{"key": "k", "markerVersion": "v1"}]
    }))
    .unwrap();
    assert!(matches!(r, OperationRequest::Restore { .. }));
}

// ------------------------------------------------- audit fixes (2026-11-14)

/// A folder deleted completely, while an object was added to it after the
/// review: the item is not reported as done, and the new object is kept.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_with_objects_added_after_review_is_not_reported_deleted() {
    let s = setup(true).await;
    s.seed(&["d/1", "d/2", "d-other/x"]);
    let plan = s
        .plan(OperationRequest::Delete {
            items: vec![folder("d/")],
        })
        .await
        .unwrap();
    assert_eq!(plan.objects, 2);
    s.seed(&["d/late"]);
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(items[0].state, ItemState::Partial);
    assert_eq!(items[0].note, Some(ItemNote::SourceNotEmpty));
    assert_eq!(items[0].hidden, 2);
    assert_eq!(summary.state, OperationState::Partial);
    assert!(
        !summary.retryable,
        "a new review is needed for the new object"
    );
    assert_eq!(s.current(), vec!["d-other/x", "d/late"]);
}

/// The same for a folder rename: the copies are made and the planned
/// originals removed, but the folder is not reported as renamed.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_renamed_while_an_object_was_added_is_reported_partial() {
    let s = setup(false).await;
    s.seed(&["photos/a", "photos/b", "photos-backup/keep"]);
    let plan = s
        .plan(OperationRequest::Rename {
            item: folder("photos/"),
            new_name: "pics".into(),
        })
        .await
        .unwrap();
    s.seed(&["photos/late"]);
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(items[0].state, ItemState::Partial);
    assert_eq!(items[0].note, Some(ItemNote::SourceNotEmpty));
    assert_eq!(summary.state, OperationState::Partial);
    assert_eq!(
        s.current(),
        vec!["photos-backup/keep", "photos/late", "pics/a", "pics/b"]
    );
}

/// A folder moved completely with nothing added: still `completed`.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_moved_completely_is_completed() {
    let s = setup(true).await;
    s.seed(&["m/1", "m/sub/2", "m-x/3"]);
    let (_, summary, items) = s
        .run(
            OperationRequest::Move {
                items: vec![folder("m/")],
                destination: "to/".into(),
            },
            ConflictPolicy::Skip,
        )
        .await;
    assert_eq!(items[0].state, ItemState::Completed);
    assert_eq!(items[0].note, None);
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(s.current(), vec!["m-x/3", "to/m/1", "to/m/sub/2"]);
}

/// A cancel that arrives after the last request: nothing was left undone,
/// so the operation is not reported as cancelled.
#[test]
fn a_late_cancel_does_not_turn_a_completed_operation_cancelled() {
    let done = ItemCounts {
        completed: 3,
        skipped: 1,
        ..ItemCounts::default()
    };
    assert_eq!(
        operation_state(false, true, &done),
        OperationState::Completed
    );
    let left = ItemCounts {
        completed: 1,
        cancelled: 2,
        ..ItemCounts::default()
    };
    assert_eq!(
        operation_state(false, true, &left),
        OperationState::Cancelled
    );
    let partial = ItemCounts {
        completed: 1,
        partial: 1,
        ..ItemCounts::default()
    };
    assert_eq!(
        operation_state(false, true, &partial),
        OperationState::Cancelled
    );
    assert_eq!(
        operation_state(false, false, &partial),
        OperationState::Partial
    );
    assert_eq!(operation_state(true, true, &done), OperationState::Failed);
}

#[test]
fn a_delete_marker_hides_only_a_kept_real_version() {
    // Versioned bucket: the marker has a real id, the version is kept.
    assert!(restorable_after_delete(true, Some("v2"), Some("v1")));
    // An object from before versioning (id "null") on an enabled bucket: the
    // marker gets a real id and the "null" version stays.
    assert!(restorable_after_delete(true, Some("v2"), Some("null")));
    // Suspended: the marker is "null" and replaces a "null" version: gone.
    assert!(!restorable_after_delete(true, Some("null"), Some("null")));
    assert!(!restorable_after_delete(true, Some("null"), None));
    // Suspended, but the current version has a real id (written while
    // versioning was on): that version is kept behind the "null" marker.
    assert!(restorable_after_delete(true, Some("null"), Some("v7")));
    // No marker: unversioned, removed.
    assert!(!restorable_after_delete(false, None, Some("v1")));
}

/// A restore whose first marker is already gone (an earlier attempt's
/// answer was lost) still removes the rest and brings the file back.
#[tokio::test(flavor = "multi_thread")]
async fn a_restore_goes_on_when_a_marker_was_already_removed() {
    let s = setup(true).await;
    s.seed(&["twice.txt"]);
    let _ = s
        .run(
            OperationRequest::Delete {
                items: vec![file("twice.txt")],
            },
            ConflictPolicy::Skip,
        )
        .await;
    // Deleted a second time (by another client): two markers in a row.
    {
        let mut st = s.data.lock();
        let (id, modified) = st.stamp();
        st.buckets
            .get_mut(BUCKET)
            .unwrap()
            .keys
            .get_mut("twice.txt")
            .unwrap()
            .push(super::super::test_server::Version {
                id,
                marker: true,
                data: Arc::new(Vec::new()),
                content_type: None,
                metadata: HashMap::new(),
                etag: String::new(),
                modified,
            });
    }
    assert_eq!(s.versions_of("twice.txt").len(), 3);
    let versions = s.versions_of("twice.txt");
    let newest_marker = versions.last().unwrap().0.clone();
    let plan = s
        .plan(OperationRequest::Restore {
            items: vec![RestoreRef {
                key: "twice.txt".into(),
                marker_version: newest_marker.clone(),
            }],
        })
        .await
        .unwrap();
    assert!(plan.items[0].problem.is_none(), "{plan:?}");
    // Remove the newest marker behind the plan's back.
    s.data
        .lock()
        .buckets
        .get_mut(BUCKET)
        .unwrap()
        .keys
        .get_mut("twice.txt")
        .unwrap()
        .retain(|v| v.id != newest_marker);
    let started = s.start(&plan, ConflictPolicy::Skip).await;
    let (summary, items) = s.finished(&started.operation).await;
    assert_eq!(items[0].state, ItemState::Completed, "{items:?}");
    assert_eq!(summary.state, OperationState::Completed);
    assert_eq!(s.current(), vec!["twice.txt"]);
    assert_eq!(
        s.versions_of("twice.txt").len(),
        1,
        "only markers were removed"
    );
}

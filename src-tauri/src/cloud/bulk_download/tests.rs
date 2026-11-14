//! Tests of downloading several files and folders against the in-memory
//! versioned S3 server, into temporary folders. No real provider, no
//! dialog (the dialog only produces the folder these tests pass).

use std::fs;
use std::time::Duration;

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::s3::ClientOptions;
use super::super::test_server::{Inject, TempDir, TestServer, Versioned};
use super::*;

const KEY_ID: &str = "bulk-key";
const SECRET: &str = "bulk-Secret-0123456789";
const BUCKET: &str = "bulk-bucket";

#[derive(Default)]
struct Recorder(Mutex<Vec<BulkEvent>>);

impl BulkSink for Recorder {
    fn emit(&self, event: BulkEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Setup {
    server: TestServer,
    data: Versioned,
    accounts: Arc<CloudAccounts>,
    bulk: Arc<BulkDownloads>,
    sink: Arc<Recorder>,
    dir: TempDir,
}

async fn setup() -> Setup {
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, true)]).await;
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        ClientOptions {
            max_attempts: 1,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            ..ClientOptions::default()
        },
    ));
    accounts
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
        .unwrap();
    Setup {
        server,
        data,
        accounts,
        bulk: Arc::new(BulkDownloads::new()),
        sink: Arc::default(),
        dir: TempDir::new("bulk-download"),
    }
}

impl Setup {
    fn seed(&self, keys: &[&str]) {
        for k in keys {
            self.data
                .seed(BUCKET, k, format!("content of {k}").as_bytes());
        }
    }

    async fn begin(
        &self,
        items: Vec<ItemRef>,
        dest: Option<PathBuf>,
    ) -> Result<Option<BulkStarted>, CommandError> {
        Arc::clone(&self.bulk)
            .begin(
                Arc::clone(&self.accounts),
                "local".into(),
                BUCKET.into(),
                items,
                async move { dest },
                Arc::clone(&self.sink) as Arc<dyn BulkSink>,
            )
            .await
    }

    async fn finished(&self, batch: &str) -> (BulkSummary, Vec<BulkItem>) {
        let t = Instant::now();
        loop {
            let events: Vec<BulkEvent> = self
                .sink
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.batch == batch)
                .cloned()
                .collect();
            if let Some(BulkEventBody::Finished { summary }) = events.last().map(|e| e.body.clone())
            {
                assert!(!self.bulk.busy());
                let dir = self.dir.0.to_string_lossy().into_owned();
                let mut items = std::collections::BTreeMap::new();
                for (i, e) in events.iter().enumerate() {
                    assert_eq!(e.seq, i as u64 + 1);
                    let json = serde_json::to_string(e).unwrap();
                    assert!(
                        !json.contains(&dir),
                        "a local path reached the page: {json}"
                    );
                    assert!(!json.contains(SECRET));
                    if let BulkEventBody::Item { item } = &e.body {
                        items.insert(item.index, item.clone());
                    }
                }
                return (summary, items.into_values().collect());
            }
            assert!(
                t.elapsed() < Duration::from_secs(60),
                "{batch} did not finish"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Every path below `dest`, `/` between names, sorted, hidden staging
    /// folders included (there must be none).
    fn tree(&self, dest: &Path) -> Vec<String> {
        fn walk(base: &Path, p: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(p).unwrap() {
                let e = e.unwrap();
                let rel = e
                    .path()
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if e.file_type().unwrap().is_dir() {
                    out.push(format!("{rel}/"));
                    walk(base, &e.path(), out);
                } else {
                    out.push(rel);
                }
            }
        }
        let mut out = Vec::new();
        walk(dest, dest, &mut out);
        out.sort();
        out
    }
}

#[test]
fn local_names_are_safe_on_windows_and_linux() {
    assert_eq!(local_segment("a:b?c*.txt"), "a_b_c_.txt");
    assert_eq!(local_segment("trailing. "), "trailing");
    assert_eq!(local_segment(".."), "_");
    assert_eq!(local_segment("."), "_");
    assert_eq!(local_segment(""), "_");
    assert_eq!(local_segment("CON"), "_CON");
    assert_eq!(local_segment("con.txt"), "_con.txt");
    assert_eq!(local_segment("COM1.log"), "_COM1.log");
    assert_eq!(local_segment("COM0"), "COM0");
    assert_eq!(local_segment("CONSOLE"), "CONSOLE");
    assert_eq!(local_segment("größe +x"), "größe +x");
    assert_eq!(local_segment("a\\b"), "a_b");
    let mut paths = vec![
        vec!["d".to_owned(), "A.txt".to_owned()],
        vec!["d".to_owned(), "a.txt".to_owned()],
    ];
    dedupe(&mut paths);
    assert_eq!(paths[1], vec!["d".to_owned(), "a (1).txt".to_owned()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_and_files_keep_their_structure_with_safe_names() {
    let s = setup().await;
    s.seed(&[
        "photos/",
        "photos/a.jpg",
        "photos/sub/b:c?.jpg",
        "photos/../up.txt",
        "photos/empty/",
        "photos-x/no.txt",
        "top.txt",
    ]);
    let dest = s.dir.path("dest");
    fs::create_dir_all(&dest).unwrap();
    let started = s
        .begin(
            vec![
                ItemRef::Folder {
                    prefix: "photos/".into(),
                },
                ItemRef::File {
                    key: "top.txt".into(),
                },
            ],
            Some(dest.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.folder_name, "dest");
    let (summary, items) = s.finished(&started.batch).await;
    assert_eq!(summary.state, BatchState::Completed, "{items:?}");
    assert_eq!(
        items
            .iter()
            .map(|i| i.saved_as.clone().unwrap())
            .collect::<Vec<_>>(),
        vec!["photos", "top.txt"]
    );
    assert_eq!(
        s.tree(&dest),
        vec![
            "photos/",
            "photos/_/",
            "photos/_/up.txt",
            "photos/a.jpg",
            "photos/empty/",
            "photos/sub/",
            "photos/sub/b_c_.jpg",
            "top.txt"
        ]
    );
    assert_eq!(
        fs::read_to_string(dest.join("photos/a.jpg")).unwrap(),
        "content of photos/a.jpg"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_name_is_never_replaced_or_merged() {
    let s = setup().await;
    s.seed(&["docs/a.txt", "b.txt"]);
    let dest = s.dir.path("dest2");
    fs::create_dir_all(dest.join("docs")).unwrap();
    fs::write(dest.join("docs/mine.txt"), "mine").unwrap();
    fs::write(dest.join("b.txt"), "mine too").unwrap();
    let started = s
        .begin(
            vec![
                ItemRef::Folder {
                    prefix: "docs/".into(),
                },
                ItemRef::File {
                    key: "b.txt".into(),
                },
            ],
            Some(dest.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, items) = s.finished(&started.batch).await;
    assert_eq!(items[0].saved_as.as_deref(), Some("docs (1)"));
    assert_eq!(items[1].saved_as.as_deref(), Some("b (1).txt"));
    assert_eq!(fs::read_to_string(dest.join("b.txt")).unwrap(), "mine too");
    assert_eq!(
        s.tree(&dest),
        vec![
            "b (1).txt",
            "b.txt",
            "docs (1)/",
            "docs (1)/a.txt",
            "docs/",
            "docs/mine.txt"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_file_leaves_nothing_of_its_item_and_retry_finishes() {
    let s = setup().await;
    s.seed(&["f/1", "f/2", "f/3", "g"]);
    let dest = s.dir.path("dest3");
    fs::create_dir_all(&dest).unwrap();
    s.server.inject("GetObject", None, 1, Inject::Deny);
    let started = s
        .begin(
            vec![
                ItemRef::Folder {
                    prefix: "f/".into(),
                },
                ItemRef::File { key: "g".into() },
            ],
            Some(dest.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    let (summary, items) = s.finished(&started.batch).await;
    assert_eq!(summary.state, BatchState::Failed);
    assert_eq!(items[0].state, ItemState::Failed);
    assert_eq!(items[1].state, ItemState::Cancelled);
    assert!(
        s.tree(&dest).is_empty(),
        "nothing half-downloaded, no staging folder: {:?}",
        s.tree(&dest)
    );
    let again = Arc::clone(&s.bulk)
        .retry(
            Arc::clone(&s.accounts),
            &started.batch,
            Arc::clone(&s.sink) as Arc<dyn BulkSink>,
        )
        .await
        .unwrap();
    let (summary, _) = s.finished(&again.batch).await;
    assert_eq!(summary.state, BatchState::Completed);
    assert_eq!(s.tree(&dest), vec!["f/", "f/1", "f/2", "f/3", "g"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_removes_the_current_item_and_keeps_saved_ones() {
    let s = setup().await;
    s.seed(&["one.txt", "two/1", "two/2"]);
    let dest = s.dir.path("dest4");
    fs::create_dir_all(&dest).unwrap();
    // The second GetObject never answers until cancelled.
    s.server
        .inject("GetObject", None, 1, Inject::Run(Arc::new(|| {})));
    s.server.inject("GetObject", None, 1, Inject::Hang);
    let started = s
        .begin(
            vec![
                ItemRef::File {
                    key: "one.txt".into(),
                },
                ItemRef::Folder {
                    prefix: "two/".into(),
                },
            ],
            Some(dest.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    // Wait for the second item to be downloading, then cancel.
    let t = Instant::now();
    loop {
        let downloading = s.sink.0.lock().unwrap().iter().any(|e| {
            matches!(&e.body, BulkEventBody::Item { item } if item.index == 1 && item.state == ItemState::Downloading)
        });
        if downloading {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.bulk.cancel(&started.batch), CancelOutcome::Cancelling);
    let (summary, items) = s.finished(&started.batch).await;
    assert_eq!(summary.state, BatchState::Cancelled);
    assert_eq!(items[0].state, ItemState::Saved);
    assert_eq!(items[1].state, ItemState::Cancelled);
    assert_eq!(s.tree(&dest), vec!["one.txt"]);
    assert_eq!(s.bulk.cancel(&started.batch), CancelOutcome::NotActive);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_dialog_or_a_missing_file_starts_nothing() {
    let s = setup().await;
    s.seed(&["a"]);
    assert!(
        s.begin(vec![ItemRef::File { key: "a".into() }], None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!s.bulk.busy());
    let e = s
        .begin(
            vec![ItemRef::File {
                key: "missing".into(),
            }],
            Some(s.dir.path("x")),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::ObjectNotFound);
    assert!(!s.bulk.busy());
    let e = s.begin(vec![], Some(s.dir.path("x"))).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
}

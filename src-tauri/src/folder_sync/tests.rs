//! Folder sync end to end (ADR-043): real sync folders in temporary
//! directories against the in-process S3 servers (the versioned in-memory
//! one; s3s-fs for multipart uploads). Passes are driven by hand
//! (`Drive::run`, one complete pass: full scan, listing, planning) so each
//! test is deterministic; one test runs the background engine with a real
//! file watcher. "Another client" changes the bucket directly through the
//! server's state. No real provider, no network.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::engine::{self, LoopState};
use super::*;
use crate::cloud::account::MemoryAccountStore;
use crate::cloud::credentials::{CredentialStore, MemoryCredentialStore};
use crate::cloud::dashboard::{AccountInput, CredentialInput};
use crate::cloud::s3::{ClientOptions, SHA256_METADATA};
use crate::cloud::test_server::{Inject, TempDir, TestServer, Versioned, sha256_hex};

const KEY: &str = "sync-key";
const SECRET: &str = "sync-Secret-0123456789";
const BUCKET: &str = "sync-bucket";
const OTHER_BUCKET: &str = "other-bucket";

fn options() -> ClientOptions {
    ClientOptions {
        max_attempts: 1,
        connect_timeout: Duration::from_secs(3),
        read_timeout: Duration::from_secs(10),
        ..ClientOptions::default()
    }
}

struct Env {
    server: TestServer,
    data: Versioned,
    accounts: Arc<CloudAccounts>,
    syncs: Arc<FolderSyncs>,
    dir: TempDir,
    account: String,
}

/// One sync, driven pass by pass.
struct Drive {
    syncs: Arc<FolderSyncs>,
    id: String,
    root: PathBuf,
    control: Arc<Control>,
    st: LoopState,
}

async fn env_with(name: &str, versioning: bool, store: Option<SqliteSyncStore>) -> Env {
    let (server, data) = TestServer::start_versioned(
        KEY,
        SECRET,
        &[(BUCKET, versioning), (OTHER_BUCKET, versioning)],
    )
    .await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        options(),
    ));
    let info = accounts
        .add(
            &AccountInput {
                label: "Synced".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(server.endpoint.clone()),
                bucket: BUCKET.into(),
            },
            CredentialInput::new(KEY, SECRET),
        )
        .unwrap();
    let dir = TempDir::new(name);
    let store = store.unwrap_or_else(|| SqliteSyncStore::in_memory().unwrap());
    let syncs = FolderSyncs::load(Ok(store), Arc::clone(&accounts), Some(dir.path("app-data")));
    Env {
        server,
        data,
        accounts,
        syncs,
        dir,
        account: info.id,
    }
}

async fn env(name: &str) -> Env {
    env_with(name, true, None).await
}

impl Env {
    async fn choose(&self, path: &Path) -> FolderChoice {
        choose_with(
            Arc::clone(&self.syncs),
            Arc::clone(&self.accounts),
            None,
            self.account.clone(),
            None,
            path.to_path_buf(),
        )
        .await
        .unwrap()
    }

    /// A sync of the account's bucket in the folder `name` (created).
    async fn enable(&self, name: &str) -> Drive {
        let root = self.dir.path(name);
        fs::create_dir_all(&root).unwrap();
        self.enable_at(&root).await
    }

    async fn enable_at(&self, root: &Path) -> Drive {
        let choice = self.choose(root).await;
        assert!(choice.problems.is_empty(), "{:?}", choice.problems);
        let status = self.syncs.enable(&choice.token, 15).unwrap();
        Drive {
            syncs: Arc::clone(&self.syncs),
            id: status.id,
            root: scan::canonical(root).unwrap(),
            control: Control::new(),
            st: LoopState {
                watch: false,
                ..LoopState::new()
            },
        }
    }

    // ---- the bucket, as another client changes it

    fn put(&self, key: &str, data: &[u8]) {
        self.data.seed(BUCKET, key, data);
    }

    /// As sotyware on another computer would store it (with its SHA-256).
    fn put_with_sha(&self, key: &str, data: &[u8]) {
        self.data.lock().put(
            BUCKET,
            key,
            data.to_vec(),
            None,
            HashMap::from([(SHA256_METADATA.to_owned(), sha256_hex(data))]),
        );
    }

    fn delete(&self, key: &str) {
        let mut st = self.data.lock();
        let (id, modified) = st.stamp();
        let b = st.buckets.get_mut(BUCKET).unwrap();
        if b.versioning {
            b.keys
                .entry(key.to_owned())
                .or_default()
                .push(crate::cloud::test_server::Version {
                    id,
                    marker: true,
                    data: Arc::new(Vec::new()),
                    content_type: None,
                    metadata: HashMap::new(),
                    etag: String::new(),
                    modified,
                });
        } else {
            b.keys.remove(key);
        }
    }

    /// A server-side rename by another client: copy, then delete.
    fn rename(&self, from: &str, to: &str) {
        let v = self.data.lock().buckets[BUCKET]
            .current(from)
            .unwrap()
            .clone();
        self.data.lock().put(
            BUCKET,
            to,
            (*v.data).clone(),
            v.content_type.clone(),
            v.metadata.clone(),
        );
        self.delete(from);
    }

    fn object(&self, key: &str) -> Option<Vec<u8>> {
        self.data.lock().buckets[BUCKET]
            .current(key)
            .map(|v| (*v.data).clone())
    }

    fn sha_of(&self, key: &str) -> Option<String> {
        self.data.lock().buckets[BUCKET]
            .current(key)
            .and_then(|v| v.metadata.get(SHA256_METADATA).cloned())
    }

    /// Every current object: key → content.
    fn objects(&self) -> BTreeMap<String, Vec<u8>> {
        self.data
            .lock()
            .current_objects(BUCKET)
            .into_iter()
            .map(|(k, v)| (k, (*v.data).clone()))
            .collect()
    }

    /// Versions of `key` (delete markers included).
    fn versions(&self, key: &str) -> usize {
        self.data.lock().buckets[BUCKET]
            .keys
            .get(key)
            .map_or(0, Vec::len)
    }

    fn ops_since(&self, mark: usize) -> Vec<String> {
        self.server.ops()[mark..].to_vec()
    }
}

impl Drive {
    fn shared(&self) -> &Arc<Shared> {
        &self.syncs.shared
    }

    /// One complete pass: everything scanned, the bucket listed.
    async fn run(&mut self) -> SyncStatus {
        self.st.full_local = true;
        self.control.force_remote.store(true, Ordering::SeqCst);
        let shared = Arc::clone(self.shared());
        let _ = engine::step(&shared, &self.id, &self.control, &mut self.st).await;
        self.status()
    }

    /// Passes until the sync is in step (or `limit` passes).
    async fn settle(&mut self) -> SyncStatus {
        for _ in 0..6 {
            let s = self.run().await;
            if s.state == SyncState::Synced && s.message.is_none() {
                return s;
            }
            // Something waits on a settling file or a retry.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.status()
    }

    fn status(&self) -> SyncStatus {
        self.syncs
            .overview()
            .syncs
            .into_iter()
            .find(|s| s.id == self.id)
            .unwrap()
    }

    fn details(&self) -> SyncDetails {
        self.syncs.details(&self.id).unwrap()
    }

    fn path(&self, rel: &str) -> PathBuf {
        scan::join(&self.root, rel)
    }

    /// Writes a file here, its time a few seconds back so it does not wait
    /// to settle.
    fn write(&self, rel: &str, data: &[u8]) {
        let p = self.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, data).unwrap();
        age(&p);
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.path(rel)).ok()
    }

    /// Every file here (outside sotyware's folder): path → content.
    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let rel = watch::relative(&self.root, &e.path()).unwrap();
                if model::ignored(&rel) {
                    continue;
                }
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.insert(rel, fs::read(e.path()).unwrap());
                }
            }
        }
        out
    }

    fn trash(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut stack = vec![scan::own_dir(&self.root).join(scan::TRASH)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.push(fs::read(e.path()).unwrap());
                }
            }
        }
        out.sort();
        out
    }
}

/// Moves a file's modification time 10 s back (an older edit: settled).
fn age(path: &Path) {
    let t = std::time::SystemTime::now() - Duration::from_secs(10);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn bytes_of(n: usize, seed: u8) -> Vec<u8> {
    crate::cloud::test_server::pattern(n, seed)
}

// --------------------------------------------------------------- set up

#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_folder_is_checked_and_marked_as_the_sync_folder() {
    let e = env("sync-choose").await;
    let root = e.dir.path("Synced Bucket");
    fs::create_dir_all(&root).unwrap();
    let choice = e.choose(&root).await;
    assert!(choice.problems.is_empty(), "{:?}", choice.problems);
    assert_eq!(choice.path, root.to_str().unwrap(), "the path to show");
    assert_eq!((choice.files, choice.bytes), (0, 0));
    assert_eq!(choice.versioning.as_deref(), Some("enabled"));
    assert!(choice.free_bytes.is_some());
    assert!(!choice.was_sync_folder);
    let status = e.syncs.enable(&choice.token, 15).unwrap();
    assert_eq!(
        status.path,
        scan::canonical(&root).unwrap().to_str().unwrap()
    );
    let marker = scan::read_marker(&root).unwrap().unwrap();
    assert_eq!(marker.bucket, BUCKET);
    // A token is used once.
    assert!(e.syncs.enable(&choice.token, 15).is_err());
    // Not a second sync of the same bucket, and never one folder twice.
    let again = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        e.account.clone(),
        None,
        root.clone(),
    )
    .await;
    assert!(again.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unsafe_folders_are_refused() {
    let e = env("sync-refuse").await;
    // A whole drive.
    let fs_root = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };
    let c = e.choose(&fs_root).await;
    assert!(
        c.problems.iter().any(|p| p.contains("whole drive")),
        "{:?}",
        c.problems
    );
    // sotyware's own data folder, and anything inside it.
    let data = e.dir.path("app-data");
    fs::create_dir_all(data.join("inner")).unwrap();
    let c = e.choose(&data.join("inner")).await;
    assert!(
        c.problems.iter().any(|p| p.contains("own data")),
        "{:?}",
        c.problems
    );
    // A folder that does not exist.
    let c = e.choose(&e.dir.path("missing")).await;
    assert!(c.problems.iter().any(|p| p.contains("does not exist")));
    // Overlap with another sync's folder (a second account, another bucket).
    let first = e.enable("first").await;
    let other = e
        .accounts
        .add(
            &AccountInput {
                label: "Other".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(e.server.endpoint.clone()),
                bucket: OTHER_BUCKET.into(),
            },
            CredentialInput::new(KEY, SECRET),
        )
        .unwrap();
    fs::create_dir_all(first.root.join("nested")).unwrap();
    for path in [
        first.root.join("nested"),
        first.root.clone(),
        e.dir.0.clone(),
    ] {
        let c = choose_with(
            Arc::clone(&e.syncs),
            Arc::clone(&e.accounts),
            None,
            other.id.clone(),
            None,
            path.clone(),
        )
        .await
        .unwrap();
        assert!(
            c.problems
                .iter()
                .any(|p| p.contains("two syncs cannot share")),
            "{path:?}: {:?}",
            c.problems
        );
        assert!(e.syncs.enable(&c.token, 15).is_err(), "a problem blocks it");
    }
    // Not an offered interval.
    let free = e.dir.path("free");
    fs::create_dir_all(&free).unwrap();
    let c = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        other.id.clone(),
        None,
        free,
    )
    .await
    .unwrap();
    assert!(e.syncs.enable(&c.token, 7).is_err());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_without_write_permission_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let e = env("sync-readonly").await;
    let root = e.dir.path("ro");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    let writable = fs::write(root.join("probe"), b"x").is_ok(); // root ignores modes
    let c = e.choose(&root).await;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return;
    }
    assert!(
        c.problems
            .iter()
            .any(|p| p.contains("may not create files")),
        "{:?}",
        c.problems
    );
}

// ------------------------------------------------------------ first sync

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_bucket_receives_the_folder() {
    let e = env("sync-upload-all").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"alpha");
    d.write("docs/report.docx", &bytes_of(70_000, 1));
    d.write("docs/deep/x.bin", &bytes_of(10, 2));
    fs::create_dir_all(d.path("empty")).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let objects = e.objects();
    assert_eq!(objects["a.txt"], b"alpha");
    assert_eq!(objects["docs/report.docx"], bytes_of(70_000, 1));
    assert_eq!(objects["docs/deep/x.bin"], bytes_of(10, 2));
    assert!(
        objects.contains_key("empty/"),
        "an empty folder gets a marker"
    );
    assert!(
        !objects.contains_key("docs/"),
        "a folder with files needs none"
    );
    assert_eq!(e.sha_of("a.txt"), Some(sha256_hex(b"alpha")));
    assert_eq!((s.files, s.bytes), (3, 5 + 70_000 + 10));
    assert!(s.last_synced_ms.is_some());
    // In step: another pass only lists.
    let mark = e.server.ops().len();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Synced);
    assert_eq!(e.ops_since(mark), ["ListObjectsV2"]);
    let activity = d.details().activity;
    assert_eq!(activity.iter().filter(|a| a.op == "upload").count(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bucket_with_files_and_folders_comes_down() {
    let e = env("sync-download-all").await;
    e.put("top.txt", b"top");
    e.put_with_sha("photos/2024/a.jpg", &bytes_of(50_000, 3));
    e.put("photos/2024/b.jpg", &bytes_of(1_000, 4));
    e.put("größe +test.bin", b"unicode");
    e.put("marker-only/", b"");
    e.put("bad//name", b"empty segment");
    e.put(".sotyware-sync/root.json", b"never into our folder");
    let mut d = e.enable("root").await;
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let files = d.files();
    assert_eq!(files["top.txt"], b"top");
    assert_eq!(files["photos/2024/a.jpg"], bytes_of(50_000, 3));
    assert_eq!(files["photos/2024/b.jpg"], bytes_of(1_000, 4));
    assert_eq!(files["größe +test.bin"], b"unicode");
    assert!(d.path("marker-only").is_dir());
    assert!(!files.contains_key("bad//name"));
    assert_eq!(files.len(), 4);
    let marker = scan::read_marker(&d.root).unwrap().unwrap();
    assert_eq!(marker.bucket, BUCKET, "our marker untouched");
    // Nothing was uploaded back.
    assert_eq!(e.versions("top.txt"), 1);
    assert_eq!(e.versions("photos/2024/b.jpg"), 1);
    assert_eq!(
        s.remote_files, 4,
        "the listing's current files (no marker, no skipped key)"
    );
    assert!(s.pending_downloads == 0 && s.pending_uploads == 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_with_files_is_merged_with_the_bucket() {
    let e = env("sync-merge").await;
    let root = e.dir.path("root");
    fs::create_dir_all(&root).unwrap();
    // Here only, there only, the same on both sides (another tool's upload,
    // without SHA-256, and sotyware's, with), different on both sides.
    for (rel, data) in [
        ("only-here.txt", b"here".as_slice()),
        ("same-plain.txt", b"same"),
        ("same-sha.txt", b"same2"),
        ("differs.txt", b"local version"),
    ] {
        let p = root.join(rel);
        fs::write(&p, data).unwrap();
        age(&p);
    }
    e.put("only-there.txt", b"there");
    e.put("same-plain.txt", b"same");
    e.put_with_sha("same-sha.txt", b"same2");
    e.put("differs.txt", b"bucket version");
    let choice = e.choose(&root).await;
    assert_eq!(choice.files, 4);
    assert!(choice.warnings.iter().any(|w| w.contains("merged")));
    let status = e.syncs.enable(&choice.token, 15).unwrap();
    let mut d = Drive {
        syncs: Arc::clone(&e.syncs),
        id: status.id,
        root: scan::canonical(&root).unwrap(),
        control: Control::new(),
        st: LoopState {
            watch: false,
            ..LoopState::new()
        },
    };
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let objects = e.objects();
    assert_eq!(objects["only-here.txt"], b"here");
    assert_eq!(d.read("only-there.txt").unwrap(), b"there");
    // The same content is recorded, never sent again.
    assert_eq!(e.versions("same-plain.txt"), 1);
    assert_eq!(e.versions("same-sha.txt"), 1);
    // Different: both kept. This computer's keeps the name, the bucket's
    // is the conflicted copy, on both sides.
    assert_eq!(objects["differs.txt"], b"local version");
    let copy = objects
        .keys()
        .find(|k| k.starts_with("differs (conflicted copy "))
        .expect("the bucket's version kept")
        .clone();
    assert_eq!(objects[&copy], b"bucket version");
    assert_eq!(d.read(&copy).unwrap(), b"bucket version");
    assert_eq!(d.read("differs.txt").unwrap(), b"local version");
    assert_eq!(s.conflicts, 1);
    let puts = e
        .ops_since(mark)
        .iter()
        .filter(|o| *o == "PutObject")
        .count();
    assert_eq!(puts, 2, "only-here.txt and the local differs.txt");
}

// --------------------------------------------------------- incremental

#[tokio::test(flavor = "multi_thread")]
async fn changes_go_both_ways() {
    let e = env("sync-changes").await;
    let mut d = e.enable("root").await;
    d.write("note.txt", b"v1");
    e.put_with_sha("remote.txt", b"r1");
    d.settle().await;
    assert_eq!(e.object("note.txt").unwrap(), b"v1");
    assert_eq!(d.read("remote.txt").unwrap(), b"r1");
    // A new local file and a local change.
    d.write("new.txt", b"new here");
    d.write("note.txt", b"v2, longer");
    // A new remote file and a remote change (another computer).
    e.put_with_sha("new-remote.txt", b"new there");
    e.put_with_sha("remote.txt", b"r2, longer");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.object("new.txt").unwrap(), b"new here");
    assert_eq!(e.object("note.txt").unwrap(), b"v2, longer");
    assert_eq!(e.versions("note.txt"), 2, "the earlier version is kept");
    assert_eq!(d.read("new-remote.txt").unwrap(), b"new there");
    assert_eq!(d.read("remote.txt").unwrap(), b"r2, longer");
    // Only a time change (touched): recorded, not sent.
    let p = d.path("note.txt");
    fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
        .unwrap();
    let mark = e.server.ops().len();
    d.settle().await;
    assert!(!e.ops_since(mark).contains(&"PutObject".to_owned()));
    assert_eq!(e.versions("note.txt"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watched_change_is_looked_at_alone() {
    let e = env("sync-targeted").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.settle().await;
    // Only the named path is looked at again; the rest of the folder is
    // not walked (a file written without telling is not noticed).
    d.write("told.txt", b"told");
    d.write("untold.txt", b"untold");
    d.st.full_local = false;
    d.control.mark_dirty("told.txt".into());
    let shared = Arc::clone(d.shared());
    let _ = engine::step(&shared, &d.id, &d.control, &mut d.st).await;
    assert_eq!(e.object("told.txt").unwrap(), b"told");
    assert!(e.object("untold.txt").is_none());
    // A new folder with contents, named by its file only.
    d.write("new/dir/x.txt", b"x");
    d.control.mark_dirty("new/dir/x.txt".into());
    let shared = Arc::clone(d.shared());
    let _ = engine::step(&shared, &d.id, &d.control, &mut d.st).await;
    assert_eq!(e.object("new/dir/x.txt").unwrap(), b"x");
    // A full scan finds the rest.
    d.settle().await;
    assert_eq!(e.object("untold.txt").unwrap(), b"untold");
}

#[tokio::test(flavor = "multi_thread")]
async fn renames_are_moves_on_the_other_side() {
    let e = env("sync-renames").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", &bytes_of(4096, 5));
    d.write("dir/x.bin", &bytes_of(3000, 6));
    d.write("dir/y.bin", &bytes_of(3001, 7));
    d.settle().await;
    // Here: a file and a folder renamed.
    fs::rename(d.path("a.txt"), d.path("renamed.txt")).unwrap();
    fs::rename(d.path("dir"), d.path("moved")).unwrap();
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let ops = e.ops_since(mark);
    assert!(
        !ops.contains(&"PutObject".to_owned()),
        "copied, not sent: {ops:?}"
    );
    assert_eq!(ops.iter().filter(|o| *o == "CopyObject").count(), 3);
    let objects = e.objects();
    assert_eq!(
        objects.keys().cloned().collect::<Vec<_>>(),
        ["moved/x.bin", "moved/y.bin", "renamed.txt"]
    );
    assert_eq!(objects["renamed.txt"], bytes_of(4096, 5));
    // In the bucket (another client): renamed; here it is renamed too,
    // not downloaded.
    e.rename("renamed.txt", "again.txt");
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert!(!e.ops_since(mark).contains(&"GetObject".to_owned()));
    assert_eq!(d.read("again.txt").unwrap(), bytes_of(4096, 5));
    assert!(d.read("renamed.txt").is_none());
    assert!(d.trash().is_empty(), "a move is not a deletion");
}

#[tokio::test(flavor = "multi_thread")]
async fn deletions_go_both_ways_and_nothing_is_lost() {
    let e = env("sync-deletions").await;
    let mut d = e.enable("root").await;
    d.write("here.txt", b"deleted here");
    d.write("there.txt", b"deleted there");
    d.write("folder/inner.txt", b"inner");
    d.settle().await;
    fs::remove_file(d.path("here.txt")).unwrap();
    fs::remove_dir_all(d.path("folder")).unwrap();
    e.delete("there.txt");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let objects = e.objects();
    assert!(objects.is_empty(), "{objects:?}");
    // Deleted in the bucket = hidden behind a delete marker: the versions
    // stay and can be restored.
    assert_eq!(e.versions("here.txt"), 2);
    assert!(
        e.data.lock().removed_versions.is_empty(),
        "no version deleted for good"
    );
    // Deleted there: moved to the sync trash here, not deleted.
    assert!(d.read("there.txt").is_none());
    assert_eq!(d.trash(), [b"deleted there".to_vec()]);
    let ops: Vec<String> = d.details().activity.into_iter().map(|a| a.op).collect();
    assert!(ops.contains(&"delete-local".to_owned()) && ops.contains(&"delete-remote".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_wins_over_a_deletion() {
    let e = env("sync-change-wins").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a1");
    d.write("b.txt", b"b1");
    d.settle().await;
    // a: deleted here, changed there → comes back with the change.
    fs::remove_file(d.path("a.txt")).unwrap();
    e.put_with_sha("a.txt", b"a2 changed there");
    // b: changed here, deleted there → goes back up.
    d.write("b.txt", b"b2 changed here");
    e.delete("b.txt");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.read("a.txt").unwrap(), b"a2 changed there");
    assert_eq!(e.object("b.txt").unwrap(), b"b2 changed here");
}

// ------------------------------------------------------------- conflicts

#[tokio::test(flavor = "multi_thread")]
async fn conflicting_changes_keep_both_and_can_be_settled() {
    let e = env("sync-conflict").await;
    let mut d = e.enable("root").await;
    for f in ["keep-local.txt", "keep-remote.txt", "keep-both.txt"] {
        d.write(f, b"original");
    }
    d.settle().await;
    for f in ["keep-local.txt", "keep-remote.txt", "keep-both.txt"] {
        d.write(f, format!("{f}: this computer").as_bytes());
        e.put_with_sha(f, format!("{f}: the other computer").as_bytes());
    }
    let s = d.settle().await;
    assert_eq!(s.conflicts, 3, "{:?}", s.message);
    let details = d.details();
    assert_eq!(details.conflicts.len(), 3);
    for c in &details.conflicts {
        assert_eq!(
            d.read(&c.path).unwrap(),
            format!("{}: this computer", c.path).as_bytes()
        );
        assert_eq!(
            d.read(&c.copy).unwrap(),
            format!("{}: the other computer", c.path).as_bytes()
        );
        assert_eq!(e.object(&c.path), d.read(&c.path));
        assert_eq!(e.object(&c.copy), d.read(&c.copy));
    }
    let copy_of = |p: &str| {
        details
            .conflicts
            .iter()
            .find(|c| c.path == p)
            .unwrap()
            .copy
            .clone()
    };
    let (cl, cr, cb) = (
        copy_of("keep-local.txt"),
        copy_of("keep-remote.txt"),
        copy_of("keep-both.txt"),
    );
    e.syncs
        .resolve_conflict(&d.id, "keep-local.txt", ConflictChoice::Local)
        .unwrap();
    e.syncs
        .resolve_conflict(&d.id, "keep-remote.txt", ConflictChoice::Remote)
        .unwrap();
    e.syncs
        .resolve_conflict(&d.id, "keep-both.txt", ConflictChoice::Both)
        .unwrap();
    assert!(
        e.syncs
            .resolve_conflict(&d.id, "keep-both.txt", ConflictChoice::Both)
            .is_err(),
        "settled once"
    );
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(s.conflicts, 0);
    // Keep local: the copy is gone on both sides (here into the trash).
    assert!(d.read(&cl).is_none() && e.object(&cl).is_none());
    assert_eq!(
        e.object("keep-local.txt").unwrap(),
        b"keep-local.txt: this computer"
    );
    // Keep remote: the other computer's version under the name.
    assert!(d.read(&cr).is_none() && e.object(&cr).is_none());
    assert_eq!(
        d.read("keep-remote.txt").unwrap(),
        b"keep-remote.txt: the other computer"
    );
    assert_eq!(
        e.object("keep-remote.txt").unwrap(),
        b"keep-remote.txt: the other computer"
    );
    // Keep both: both stay.
    assert!(d.read(&cb).is_some() && e.object(&cb).is_some());
    // Nothing was lost: what was replaced here is in the trash.
    let trash = d.trash();
    assert!(trash.contains(&b"keep-local.txt: the other computer".to_vec()));
    assert!(trash.contains(&b"keep-remote.txt: this computer".to_vec()));
}

// ---------------------------------------------------------------- safety

#[tokio::test(flavor = "multi_thread")]
async fn a_bucket_that_looks_emptied_holds_the_deletions_for_the_user() {
    let e = env("sync-guard").await;
    let mut d = e.enable("root").await;
    for i in 0..60 {
        d.write(&format!("f{i:02}.txt"), format!("file {i}").as_bytes());
    }
    d.settle().await;
    assert_eq!(e.objects().len(), 60);
    // Everything disappears from the listing at once (a wrong bucket, a
    // provider fault, another client's mistake).
    for i in 0..60 {
        e.delete(&format!("f{i:02}.txt"));
    }
    let s = d.run().await;
    let held = s.held_deletions.clone().expect("held");
    assert_eq!((held.local, held.remote), (60, 0));
    assert_eq!(s.state, SyncState::Attention);
    assert_eq!(d.files().len(), 60, "nothing removed here");
    assert!(d.trash().is_empty());
    // Asked again on the next pass: still held, still nothing removed.
    let s = d.run().await;
    assert!(s.held_deletions.is_some());
    assert_eq!(d.files().len(), 60);
    // "Restore": the files go back up.
    e.syncs.answer_deletions(&d.id, false).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.objects().len(), 60);
    assert_eq!(d.files().len(), 60);
    // A deletion the user means: allowed after the question.
    for i in 0..55 {
        fs::remove_file(d.path(&format!("f{i:02}.txt"))).unwrap();
    }
    let s = d.run().await;
    assert_eq!(s.held_deletions.as_ref().map(|h| h.remote), Some(55));
    assert_eq!(e.objects().len(), 60);
    e.syncs.answer_deletions(&d.id, true).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.objects().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_listing_never_deletes_and_the_sync_goes_offline() {
    let e = env("sync-offline").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.settle().await;
    e.delete("a.txt");
    e.server
        .inject("ListObjectsV2", None, 1, Inject::InternalError);
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Offline, "{:?}", s.message);
    assert!(
        d.read("a.txt").is_some(),
        "never deleted on a failed listing"
    );
    // Connection refused (the provider is gone for now): offline again.
    e.server.inject("ListObjectsV2", None, 1, Inject::Deny);
    let s = d.run().await;
    assert_eq!(
        s.state,
        SyncState::Error,
        "a refused listing needs the user"
    );
    assert!(d.read("a.txt").is_some());
    // Back: the deletion in the bucket now reaches here (into the trash).
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert!(d.read("a.txt").is_none());
    assert_eq!(d.trash(), [b"a".to_vec()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_transfer_is_retried_later() {
    let e = env("sync-retry").await;
    let mut d = e.enable("root").await;
    e.put_with_sha("x.txt", b"x");
    e.server.inject("GetObject", None, 1, Inject::Deny);
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Attention, "{:?}", s.message);
    assert_eq!(s.failed, 1);
    let failed = d.details().failed;
    assert_eq!(failed[0].path, "x.txt");
    assert_eq!(failed[0].item.attempts, 1);
    // Not before its time.
    let s = d.run().await;
    assert_eq!(s.failed, 1);
    assert!(d.read("x.txt").is_none());
    // "Retry now".
    e.syncs.retry(&d.id).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.read("x.txt").unwrap(), b"x");
    assert!(d.details().failed.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnected_drive_pauses_the_sync_without_deleting() {
    let e = env("sync-unplug").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.write("b/c.txt", b"c");
    d.settle().await;
    // Unplugged: the folder is gone.
    let away = e.dir.path("unplugged");
    fs::rename(&d.root, &away).unwrap();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Unavailable);
    assert_eq!(e.objects().len(), 2, "nothing deleted in the bucket");
    // An empty folder in its place (an unmounted mount point): not ours.
    fs::create_dir_all(&d.root).unwrap();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Error, "{:?}", s.message);
    assert_eq!(e.objects().len(), 2);
    // Plugged in again: in step, nothing lost.
    fs::remove_dir(&d.root).unwrap();
    fs::rename(&away, &d.root).unwrap();
    e.put_with_sha("while-away.txt", b"w");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.objects().len(), 3);
    assert_eq!(d.read("while-away.txt").unwrap(), b"w");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_drive_holds_downloads_and_lets_uploads_go() {
    let e = env("sync-disk-full").await;
    let mut d = e.enable("root").await;
    e.put_with_sha("big.bin", &bytes_of(100_000, 8));
    d.write("up.txt", b"up");
    *d.shared().free_space_override.lock().unwrap() = Some(1024);
    let s = d.run().await;
    assert!(s.disk_full, "{:?}", s.message);
    assert_eq!(s.state, SyncState::Attention);
    assert!(d.read("big.bin").is_none());
    assert_eq!(e.object("up.txt").unwrap(), b"up", "uploads go on");
    let failed = d.details().failed;
    assert_eq!(failed[0].item.kind, ErrorKind::DiskFull);
    // Space again.
    *d.shared().free_space_override.lock().unwrap() = None;
    e.syncs.retry(&d.id).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.read("big.bin").unwrap(), bytes_of(100_000, 8));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_folder_is_never_taken_as_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let e = env("sync-unreadable").await;
    let mut d = e.enable("root").await;
    d.write("locked/secret.txt", b"s");
    d.write("open.txt", b"o");
    d.settle().await;
    fs::set_permissions(d.path("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(d.path("locked")).is_ok() {
        // Running as root: modes do not apply.
        fs::set_permissions(d.path("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let s = d.run().await;
    fs::set_permissions(d.path("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(s.held_deletions.is_none());
    assert_eq!(e.object("locked/secret.txt").unwrap(), b"s", "not deleted");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.objects().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn links_system_files_and_unsupported_names_are_skipped() {
    let e = env("sync-skip").await;
    let mut d = e.enable("root").await;
    d.write("desktop.ini", b"[.ShellClassInfo]");
    d.write("~$report.docx", b"owner file");
    d.write("real.txt", b"real");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(d.path("real.txt"), d.path("link.txt")).unwrap();
        fs::write(d.path("bad\u{1}name"), b"control character").unwrap();
    }
    // A key whose name this computer cannot hold.
    e.put(
        "in/the/bucket\u{7f}.txt",
        b"control character in the bucket",
    );
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let objects = e.objects();
    assert!(objects.contains_key("real.txt"));
    assert!(!objects.contains_key("desktop.ini"));
    assert!(!objects.contains_key("~$report.docx"));
    let skipped = d.details().skipped;
    assert!(skipped.iter().any(|s| s.side == "remote"), "{skipped:?}");
    #[cfg(unix)]
    {
        assert!(!objects.contains_key("link.txt"));
        assert!(
            skipped
                .iter()
                .any(|s| s.path == "link.txt" && s.side == "local")
        );
        assert!(skipped.iter().any(|s| s.path == "bad\u{1}name"));
    }
    assert!(s.skipped >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_account_stops_the_sync_without_touching_anything() {
    let e = env("sync-account-changed").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.settle().await;
    // The account now names another (empty) bucket: syncing it with this
    // synced state would look like everything was deleted there.
    let info = e
        .accounts
        .list()
        .accounts
        .into_iter()
        .find(|a| a.id == e.account)
        .unwrap();
    e.accounts
        .update(
            &info.id,
            &AccountInput {
                label: info.label.clone(),
                provider: info.provider.clone(),
                region: info.region.clone(),
                endpoint: info.endpoint.clone(),
                bucket: OTHER_BUCKET.into(),
            },
            None,
        )
        .unwrap();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Error, "{:?}", s.message);
    assert!(s.message.unwrap().contains("another bucket"));
    assert_eq!(d.read("a.txt").unwrap(), b"a");
    assert!(d.trash().is_empty());
}

// ------------------------------------------------- big files and restarts

#[tokio::test(flavor = "multi_thread")]
async fn large_files_go_in_parts_and_come_back_whole() {
    // s3s-fs keeps multipart uploads; the versioned backend has none.
    let server = TestServer::start(KEY, SECRET, &[BUCKET]).await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        ClientOptions {
            multipart_threshold: 5 * 1024 * 1024,
            part_size: 5 * 1024 * 1024,
            ..options()
        },
    ));
    let account = accounts
        .add(
            &AccountInput {
                label: "Big".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(server.endpoint.clone()),
                bucket: BUCKET.into(),
            },
            CredentialInput::new(KEY, SECRET),
        )
        .unwrap()
        .id;
    let dir = TempDir::new("sync-large");
    let big = bytes_of(12 * 1024 * 1024 + 17, 9);
    let mut drives = Vec::new();
    for name in ["first", "second"] {
        let syncs = FolderSyncs::load(
            Ok(SqliteSyncStore::in_memory().unwrap()),
            Arc::clone(&accounts),
            None,
        );
        let root = dir.path(name);
        fs::create_dir_all(&root).unwrap();
        let c = choose_with(
            Arc::clone(&syncs),
            Arc::clone(&accounts),
            None,
            account.clone(),
            None,
            root.clone(),
        )
        .await
        .unwrap();
        let status = syncs.enable(&c.token, 15).unwrap();
        drives.push(Drive {
            syncs,
            id: status.id,
            root: scan::canonical(&root).unwrap(),
            control: Control::new(),
            st: LoopState {
                watch: false,
                ..LoopState::new()
            },
        });
    }
    drives[0].write("video.mp4", &big);
    let s = drives[0].settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let ops = server.ops();
    assert!(ops.contains(&"CreateMultipartUpload".to_owned()), "{ops:?}");
    assert_eq!(
        ops.iter().filter(|o| o.starts_with("UploadPart")).count(),
        3
    );
    assert!(server.open_multipart_files().is_empty());
    // Another computer (the second folder) gets it whole.
    let s = drives[1].settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(drives[1].read("video.mp4").unwrap(), big);
    // The file keeps the time it had on the first computer.
    let t1 = fs::metadata(drives[0].path("video.mp4"))
        .unwrap()
        .modified()
        .unwrap();
    let t2 = fs::metadata(drives[1].path("video.mp4"))
        .unwrap()
        .modified()
        .unwrap();
    let diff = t1.duration_since(t2).unwrap_or_else(|e| e.duration());
    assert!(diff < Duration::from_millis(2), "{diff:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_during_a_sync_goes_on_without_duplicates() {
    let dir = TempDir::new("sync-restart-db");
    let db = dir.path("folder-sync.sqlite3");
    let e = env_with(
        "sync-restart",
        true,
        Some(SqliteSyncStore::open(&db).unwrap()),
    )
    .await;
    let mut d = e.enable("root").await;
    for i in 0..5 {
        e.put_with_sha(&format!("down{i}.bin"), &bytes_of(20_000, i));
        d.write(&format!("up{i}.bin"), &bytes_of(20_000, 100 + i));
    }
    // The process "dies" during the transfers: the third GetObject never
    // answers and the engine is stopped while it waits.
    e.server.inject("GetObject", None, 1, Inject::Hang);
    let control = Arc::clone(&d.control);
    let stopper = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        control.halt();
    });
    let _ = d.run().await;
    stopper.await.unwrap();
    // Whatever was half done: a fresh service on the same database and
    // folder (as after a crash and a new start).
    let id = d.id.clone();
    let root = d.root.clone();
    drop(d);
    let syncs = FolderSyncs::load(
        Ok(SqliteSyncStore::open(&db).unwrap()),
        Arc::clone(&e.accounts),
        None,
    );
    let mut d = Drive {
        syncs,
        id,
        root,
        control: Control::new(),
        st: LoopState {
            watch: false,
            ..LoopState::new()
        },
    };
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    for i in 0..5u8 {
        assert_eq!(
            d.read(&format!("down{i}.bin")).unwrap(),
            bytes_of(20_000, i)
        );
        assert_eq!(
            e.object(&format!("up{i}.bin")).unwrap(),
            bytes_of(20_000, 100 + i)
        );
        assert_eq!(e.versions(&format!("up{i}.bin")), 1, "never uploaded twice");
    }
    assert_eq!(e.objects().len(), 10);
    assert_eq!(d.files().len(), 10);
    // No staged download left behind.
    let staging = scan::own_dir(&d.root).join(scan::STAGING);
    assert_eq!(fs::read_dir(staging).map(|r| r.count()).unwrap_or(0), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inconsistent_synced_state_is_repaired_without_loss() {
    let e = env("sync-inconsistent").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.write("b.txt", b"b");
    d.settle().await;
    // The database says things that are not true any more.
    d.shared()
        .with_store(|s| {
            // A synced file that exists nowhere.
            s.put_synced(
                &d.id,
                &model::Synced {
                    path: "ghost.txt".into(),
                    dir: false,
                    size: 1,
                    sha256: Some("00".into()),
                    etag: Some("x".into()),
                    remote_size: Some(1),
                    ..model::Synced::default()
                },
                0,
            )?;
            // A wrong ETag and stamp for a file that is in step.
            let mut a = s.synced(&d.id, "a.txt")?.unwrap();
            a.etag = Some("stale".into());
            a.mtime_ns = Some(1);
            s.put_synced(&d.id, &a, 0)?;
            // A wrong SHA-256: neither side matches it.
            let mut b = s.synced(&d.id, "b.txt")?.unwrap();
            b.sha256 = Some("ff".into());
            b.etag = Some("stale".into());
            b.mtime_ns = Some(1);
            s.put_synced(&d.id, &b, 0)
        })
        .unwrap();
    // A stray staged download of an earlier session.
    fs::write(scan::staging_file(&d.root), b"stray").unwrap();
    d.st.purged = false;
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let ops = e.ops_since(mark);
    assert!(!ops.contains(&"PutObject".to_owned()), "{ops:?}");
    assert!(!ops.contains(&"DeleteObject".to_owned()), "{ops:?}");
    assert_eq!(d.read("a.txt").unwrap(), b"a");
    assert_eq!(d.read("b.txt").unwrap(), b"b");
    assert_eq!(e.objects().len(), 2);
    assert!(
        d.shared()
            .with_store(|s| s.synced(&d.id, "ghost.txt"))
            .unwrap()
            .is_none()
    );
    let staging = scan::own_dir(&d.root).join(scan::STAGING);
    assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
}

// ------------------------------------------------- pause, stop, change folder

#[tokio::test(flavor = "multi_thread")]
async fn pausing_and_turning_off_keep_every_file_and_object() {
    let e = env("sync-pause-off").await;
    e.syncs.start_all();
    let root = e.dir.path("root");
    fs::create_dir_all(&root).unwrap();
    let c = e.choose(&root).await;
    let s = e.syncs.enable(&c.token, 15).unwrap();
    let id = s.id.clone();
    fs::write(root.join("a.txt"), b"a").unwrap();
    age(&root.join("a.txt"));
    wait_for(&e.syncs, &id, |s| {
        s.state == SyncState::Synced && s.files == 1
    })
    .await;
    e.syncs.set_paused(&id, true).await.unwrap();
    let s = status_of(&e.syncs, &id);
    assert_eq!(s.state, SyncState::Paused);
    assert!(e.syncs.sync_now(&id).is_err(), "paused");
    // Changes while paused wait.
    fs::write(root.join("b.txt"), b"b").unwrap();
    age(&root.join("b.txt"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(e.object("b.txt").is_none());
    e.syncs.set_paused(&id, false).await.unwrap();
    wait_for(&e.syncs, &id, |s| {
        s.state == SyncState::Synced && s.files == 2
    })
    .await;
    assert_eq!(e.object("b.txt").unwrap(), b"b");
    // Off: forgotten here, marker gone, files and objects all kept.
    let report = e.syncs.disable(&id).await.unwrap();
    assert!(!report.trash_kept);
    assert!(e.syncs.overview().syncs.is_empty());
    assert!(scan::read_marker(&root).unwrap().is_none());
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"b");
    assert_eq!(e.objects().len(), 2);
    e.syncs.shutdown(Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_running_engine_notices_changes_in_the_folder() {
    let e = env("sync-watch").await;
    e.syncs.start_all();
    let root = e.dir.path("root");
    fs::create_dir_all(&root).unwrap();
    let c = e.choose(&root).await;
    let id = e.syncs.enable(&c.token, 60).unwrap().id;
    wait_for(&e.syncs, &id, |s| s.state == SyncState::Synced).await;
    // Written while the engine runs: the watcher names it (no periodic
    // check is due for an hour).
    fs::write(root.join("watched.txt"), b"seen").unwrap();
    let t = Instant::now();
    while e.object("watched.txt").is_none() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the change was not noticed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = status_of(&e.syncs, &id);
    assert!(s.watching);
    // sotyware's own change in the bucket (an upload in the browser) is
    // picked up at once, not at the next interval.
    e.put_with_sha("from-browser.txt", b"b");
    e.syncs.nudge(&e.account);
    let t = Instant::now();
    while !root.join("from-browser.txt").exists() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the nudge was not followed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    e.syncs.shutdown(Duration::from_secs(5));
}

async fn wait_for(syncs: &Arc<FolderSyncs>, id: &str, ok: impl Fn(&SyncStatus) -> bool) {
    let t = Instant::now();
    loop {
        let s = status_of(syncs, id);
        if ok(&s) {
            return;
        }
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "waited for the sync: {:?} {:?}",
            s.state,
            s.message
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn status_of(syncs: &Arc<FolderSyncs>, id: &str) -> SyncStatus {
    syncs
        .overview()
        .syncs
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
}

async fn change(e: &Env, d: &Drive, to: &Path, mode: MoveMode) -> Result<(), CommandError> {
    let c = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        e.account.clone(),
        Some(d.id.clone()),
        to.to_path_buf(),
    )
    .await
    .unwrap();
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    e.syncs.change_folder(&c.token, mode).await
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_the_folder_keeps_syncing_without_sending_again() {
    let e = env("sync-move").await;
    let mut d = e.enable("old").await;
    d.write("a.txt", b"a");
    d.write("dir/b.txt", b"b");
    d.write("unsynced-but-mine/desktop.ini", b"kept too");
    e.put("bucket-only.txt", b"r");
    d.settle().await;
    // The trash moves along.
    e.delete("bucket-only.txt");
    d.settle().await;
    let new = e.dir.path("new home");
    fs::create_dir_all(&new).unwrap();
    // Into itself: refused.
    let inside = d.root.join("dir");
    let c = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        e.account.clone(),
        Some(d.id.clone()),
        inside,
    )
    .await
    .unwrap();
    assert!(!c.problems.is_empty());
    change(&e, &d, &new, MoveMode::Move).await.unwrap();
    let old = d.root.clone();
    d.root = scan::canonical(&new).unwrap();
    assert_eq!(status_of(&e.syncs, &d.id).path, d.root.to_str().unwrap());
    assert!(old.is_dir(), "the old folder itself stays");
    assert_eq!(fs::read_dir(&old).unwrap().count(), 0, "emptied");
    assert_eq!(d.read("a.txt").unwrap(), b"a");
    assert_eq!(
        d.read("unsynced-but-mine/desktop.ini").unwrap(),
        b"kept too"
    );
    assert_eq!(d.trash(), [b"r".to_vec()]);
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let ops = e.ops_since(mark);
    assert!(
        !ops.iter()
            .any(|o| o == "PutObject" || o == "GetObject" || o == "DeleteObject"),
        "{ops:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_to_another_drive_copies_and_verifies_first() {
    let e = env("sync-move-drive").await;
    let mut d = e.enable("old").await;
    d.write("a.txt", &bytes_of(3 << 20, 1));
    d.write("dir/b.txt", b"b");
    d.settle().await;
    // Another drive: a different file system where one exists (Linux
    // /dev/shm), else copying is forced, as across drives.
    let shm = Path::new("/dev/shm");
    let other_fs = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            shm.is_dir()
                && fs::metadata(shm).map(|m| m.dev()).ok()
                    != fs::metadata(&d.root).map(|m| m.dev()).ok()
        }
        #[cfg(not(unix))]
        {
            false
        }
    };
    let target_holder = TempDir::new("sync-move-drive-target");
    let target = if other_fs {
        let t = shm.join(format!("sotyware-sync-move-{}", std::process::id()));
        fs::create_dir_all(&t).unwrap();
        t
    } else {
        migrate::FORCE_COPY.store(true, Ordering::SeqCst);
        let t = target_holder.path("target");
        fs::create_dir_all(&t).unwrap();
        t
    };
    let result = change(&e, &d, &target, MoveMode::Move).await;
    migrate::FORCE_COPY.store(false, Ordering::SeqCst);
    result.unwrap();
    let old = d.root.clone();
    d.root = scan::canonical(&target).unwrap();
    assert_eq!(d.read("a.txt").unwrap(), bytes_of(3 << 20, 1));
    assert_eq!(d.read("dir/b.txt").unwrap(), b"b");
    assert_eq!(fs::read_dir(&old).unwrap().count(), 0);
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(
        e.versions("a.txt"),
        1,
        "copied files are hashed, not sent again"
    );
    if other_fs {
        fs::remove_dir_all(&target).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_over_in_another_folder_leaves_the_old_one_alone() {
    let e = env("sync-fresh").await;
    let mut d = e.enable("old").await;
    d.write("a.txt", b"a");
    d.settle().await;
    let new = e.dir.path("fresh");
    fs::create_dir_all(&new).unwrap();
    fs::write(new.join("already-there.txt"), b"x").unwrap();
    age(&new.join("already-there.txt"));
    // Not empty: no move, only a fresh start.
    let c = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        e.account.clone(),
        Some(d.id.clone()),
        new.clone(),
    )
    .await
    .unwrap();
    assert!(!c.can_move);
    assert!(
        e.syncs
            .change_folder(&c.token, MoveMode::Move)
            .await
            .is_err()
    );
    change(&e, &d, &new, MoveMode::Fresh).await.unwrap();
    let old = d.root.clone();
    d.root = scan::canonical(&new).unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(
        d.read("a.txt").unwrap(),
        b"a",
        "downloaded into the new folder"
    );
    assert_eq!(e.object("already-there.txt").unwrap(), b"x", "merged up");
    assert_eq!(
        fs::read(old.join("a.txt")).unwrap(),
        b"a",
        "the old folder is untouched"
    );
    assert!(scan::read_marker(&old).unwrap().is_none());
    // The old folder is not synced any more: a change there goes nowhere.
    fs::write(old.join("a.txt"), b"changed in the old folder").unwrap();
    d.settle().await;
    assert_eq!(e.object("a.txt").unwrap(), b"a");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unversioned_bucket_is_named_in_the_warnings() {
    let e = env_with("sync-unversioned", false, None).await;
    let root = e.dir.path("root");
    fs::create_dir_all(&root).unwrap();
    let c = e.choose(&root).await;
    // The in-memory server answers GetBucketVersioning only for versioned
    // buckets; either way the user is told.
    assert!(
        c.warnings.iter().any(|w| w.contains("for good")),
        "{:?}",
        c.warnings
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_account_turns_its_sync_off_and_keeps_the_files() {
    let e = env("sync-account-removed").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.settle().await;
    e.accounts.remove(&e.account).unwrap();
    e.syncs.account_removed(&e.account);
    assert!(e.syncs.overview().syncs.is_empty());
    assert!(scan::read_marker(&d.root).unwrap().is_none());
    assert_eq!(d.read("a.txt").unwrap(), b"a");
    assert_eq!(e.objects().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_lost_its_marker_is_used_again_from_scratch() {
    let e = env("sync-relink").await;
    let mut d = e.enable("root").await;
    d.write("a.txt", b"a");
    d.write("b.txt", b"b");
    d.settle().await;
    // The hidden folder was deleted (by the user, or a cleanup tool), and
    // a file with it.
    fs::remove_dir_all(scan::own_dir(&d.root)).unwrap();
    fs::remove_file(d.path("b.txt")).unwrap();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Error, "{:?}", s.message);
    // Chosen again: from scratch, so the missing file comes back instead
    // of being deleted in the bucket.
    let c = choose_with(
        Arc::clone(&e.syncs),
        Arc::clone(&e.accounts),
        None,
        e.account.clone(),
        Some(d.id.clone()),
        d.root.clone(),
    )
    .await
    .unwrap();
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    assert!(!c.can_move);
    assert!(c.warnings.iter().any(|w| w.contains("from scratch")));
    e.syncs
        .change_folder(&c.token, MoveMode::Fresh)
        .await
        .unwrap();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.read("b.txt").unwrap(), b"b");
    assert_eq!(e.objects().len(), 2);
    assert!(scan::read_marker(&d.root).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn names_differing_only_in_case_are_not_both_brought_here() {
    let e = env("sync-case").await;
    let mut d = e.enable("root").await;
    // Only meaningful where the file system ignores case (Windows, macOS).
    fs::write(d.path("Probe.txt"), b"x").unwrap();
    let insensitive = d.path("probe.txt").exists();
    fs::remove_file(d.path("Probe.txt")).unwrap();
    if !insensitive {
        return;
    }
    e.put_with_sha("Report.txt", b"upper");
    e.put_with_sha("report.txt", b"lower");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.files().len(), 1, "one of them, never one over the other");
    assert_eq!(e.objects().len(), 2, "both stay in the bucket");
    assert!(d.details().skipped.iter().any(|s| s.side == "remote"));
}

// Where the file system ignores case (Windows, macOS): forced here, so the
// rules run on every system, whatever this file system does.

fn ignore_case(d: &Drive) {
    *d.shared().case_override_value.lock().unwrap() = Some(scan::Case::IgnoredListed);
}

#[tokio::test(flavor = "multi_thread")]
async fn where_case_is_ignored_two_keys_differing_in_case_come_down_once() {
    let e = env("sync-case-forced").await;
    let mut d = e.enable("root").await;
    ignore_case(&d);
    e.put_with_sha("Report.txt", b"upper");
    e.put_with_sha("report.txt", b"lower");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let files = d.files();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["Report.txt"]);
    assert_eq!(files["Report.txt"], b"upper");
    // Both stay in the bucket, each as it was.
    assert_eq!(e.object("report.txt").unwrap(), b"lower");
    assert_eq!(e.versions("report.txt"), 1);
    assert_eq!(e.versions("Report.txt"), 1);
    let skipped = d.details().skipped;
    assert!(
        skipped.iter().any(|s| s.path == "report.txt"
            && s.side == "remote"
            && s.reason.contains("Report.txt")),
        "{skipped:?}"
    );
    // Stable: another pass changes nothing.
    let mark = e.server.ops().len();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.ops_since(mark), ["ListObjectsV2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn where_case_is_ignored_a_folder_keeps_the_spelling_it_has_here() {
    let e = env("sync-case-folder").await;
    let mut d = e.enable("root").await;
    ignore_case(&d);
    d.write("docs/b.txt", b"b");
    d.settle().await;
    // Another client writes under "Docs/": here that is the folder "docs".
    e.put_with_sha("Docs/a.txt", b"a");
    e.put_with_sha("Docs/deep/c.txt", b"c");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.files().keys().collect::<Vec<_>>(), ["docs/b.txt"]);
    let top: Vec<String> = fs::read_dir(d.path(""))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != model::OWN_FOLDER)
        .collect();
    assert_eq!(top, ["docs"], "no second spelling made here");
    assert_eq!(e.object("Docs/a.txt").unwrap(), b"a", "left as it is");
    assert_eq!(e.object("docs/b.txt").unwrap(), b"b");
    let skipped = d.details().skipped;
    assert!(
        skipped
            .iter()
            .any(|s| s.path == "Docs" && s.side == "remote"),
        "{skipped:?}"
    );
    assert!(
        !skipped.iter().any(|s| s.path.starts_with("Docs/")),
        "one entry for the folder: {skipped:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn where_case_is_ignored_a_rename_of_the_case_only_renames_the_object() {
    let e = env("sync-case-rename").await;
    let mut d = e.enable("root").await;
    ignore_case(&d);
    d.write("Report.txt", &bytes_of(5000, 3));
    d.settle().await;
    fs::rename(d.path("Report.txt"), d.path("report.txt")).unwrap();
    let mark = e.server.ops().len();
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    let ops = e.ops_since(mark);
    assert!(
        !ops.contains(&"PutObject".to_owned()),
        "copied, not sent: {ops:?}"
    );
    assert_eq!(
        e.objects().keys().collect::<Vec<_>>(),
        ["report.txt"],
        "the old spelling is deleted (a delete marker)"
    );
    assert_eq!(e.object("report.txt").unwrap(), bytes_of(5000, 3));
    assert_eq!(e.versions("Report.txt"), 2);
    assert_eq!(d.files().keys().collect::<Vec<_>>(), ["report.txt"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn where_case_is_ignored_deleting_the_file_here_deletes_its_own_spelling() {
    let e = env("sync-case-delete").await;
    let mut d = e.enable("root").await;
    ignore_case(&d);
    d.write("report.txt", b"mine");
    d.settle().await;
    e.put_with_sha("Report.txt", b"theirs");
    let s = d.settle().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(d.files().keys().collect::<Vec<_>>(), ["report.txt"]);
    // Deleted here: the object of that spelling is deleted, then the other
    // spelling, no longer sharing the name, comes down in the same pass.
    fs::remove_file(d.path("report.txt")).unwrap();
    let s = d.run().await;
    assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    assert_eq!(e.objects().keys().collect::<Vec<_>>(), ["Report.txt"]);
    assert_eq!(e.versions("report.txt"), 2, "a delete marker");
    let files = d.files();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["Report.txt"]);
    assert_eq!(files["Report.txt"], b"theirs");
    assert!(d.details().skipped.is_empty(), "{:?}", d.details().skipped);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_written_or_deleted_through_a_link_in_the_sync_folder() {
    let e = env("sync-link-parent").await;
    let mut d = e.enable("root").await;
    let outside = e.dir.path("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, d.path("link")).unwrap();
    // Keys under the link's name (a folder on another computer).
    e.put_with_sha("link/x.txt", b"x");
    e.put_with_sha("link/sub/y.txt", b"y");
    for _ in 0..3 {
        let s = d.settle().await;
        assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
        assert!(
            fs::read_dir(&outside).unwrap().next().is_none(),
            "nothing is written through the link"
        );
    }
    assert_eq!(e.objects().len(), 2, "both objects stay");
    assert_eq!(e.versions("link/x.txt"), 1, "no delete marker");
    assert_eq!(e.versions("link/sub/y.txt"), 1, "no delete marker");
    let skipped = d.details().skipped;
    assert!(
        skipped
            .iter()
            .any(|s| s.path == "link" && s.side == "local"),
        "{skipped:?}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_synced_folder_replaced_by_a_link_is_left_alone() {
    let e = env("sync-link-replaced").await;
    let mut d = e.enable("root").await;
    d.write("photos/a.jpg", b"a");
    d.write("photos/b.jpg", b"b");
    d.settle().await;
    assert_eq!(e.objects().len(), 2);
    // The folder moved elsewhere and a link left in its place; the bucket
    // gains a file in that folder.
    let elsewhere = e.dir.path("elsewhere");
    fs::rename(d.path("photos"), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, d.path("photos")).unwrap();
    e.put_with_sha("photos/c.jpg", b"c");
    for _ in 0..2 {
        let s = d.settle().await;
        assert_eq!(s.state, SyncState::Synced, "{:?}", s.message);
    }
    assert_eq!(e.objects().len(), 3, "nothing deleted in the bucket");
    assert_eq!(e.versions("photos/a.jpg"), 1);
    assert_eq!(e.versions("photos/b.jpg"), 1);
    let mut there: Vec<String> = fs::read_dir(&elsewhere)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    there.sort();
    assert_eq!(
        there,
        ["a.jpg", "b.jpg"],
        "nothing written or removed behind the link"
    );
}

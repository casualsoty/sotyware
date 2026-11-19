//! Disaster-recovery tests of the backup engine and service (ADR-031)
//! against two in-process S3 servers with different keys: a primary account
//! and a backup account ("another provider"). Every restore is compared
//! byte for byte with the original. No original object is ever deleted by
//! a backup or restore here; the tests delete or corrupt data themselves to
//! simulate the disasters. No real provider.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::*;
use crate::cloud::account::MemoryAccountStore;
use crate::cloud::credentials::{CredentialStore, Credentials, MemoryCredentialStore, Secret};
use crate::cloud::dashboard::{AccountInput, CredentialInput};
use crate::cloud::s3::{ClientOptions, S3Store};
use crate::cloud::test_server::{Inject, TempDir, TestServer, sha256_hex};
use crate::storage::SqliteBackupStore;

const PRIMARY_KEY: &str = "primary-key";
const PRIMARY_SECRET: &str = "primary-Secret-0123456789";
const VAULT_KEY: &str = "vault-key";
const VAULT_SECRET: &str = "vault-Secret-0123456789";
const PRIMARY: &str = "primary-bucket";
const VAULT: &str = "vault-bucket";

fn options() -> ClientOptions {
    ClientOptions {
        max_attempts: 1,
        connect_timeout: Duration::from_secs(3),
        read_timeout: Duration::from_secs(10),
        ..ClientOptions::default()
    }
}

struct Env {
    primary: TestServer,
    vault: TestServer,
    keys: Arc<MemoryCredentialStore>,
    accounts: Arc<CloudAccounts>,
    backups: Arc<Backups>,
    dir: TempDir,
    files: TempDir,
}

/// Records a task's events.
#[derive(Default)]
struct Sink {
    events: Mutex<Vec<BackupEvent>>,
    /// Cancels the task (through `Backups::cancel`) at this progress count.
    cancel_at: Mutex<Option<(u64, Arc<Backups>)>>,
}

impl BackupSink for Sink {
    fn emit(&self, event: BackupEvent) {
        if let BackupEventBody::Progress { progress } = &event.body
            && let Some((n, b)) = &*self.cancel_at.lock().unwrap()
            && progress.done >= *n
            && progress.phase != "listing"
        {
            b.cancel(&event.task);
        }
        self.events.lock().unwrap().push(event);
    }
}

impl Sink {
    async fn finished(&self, task: &str) -> TaskSummary {
        let t = Instant::now();
        loop {
            let found = self
                .events
                .lock()
                .unwrap()
                .iter()
                .find_map(|e| match &e.body {
                    BackupEventBody::Finished { summary } if e.task == task => {
                        Some(summary.clone())
                    }
                    _ => None,
                });
            if let Some(s) = found {
                return *s;
            }
            assert!(
                t.elapsed() < Duration::from_secs(60),
                "task {task} did not finish"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn env() -> Env {
    let primary = TestServer::start(PRIMARY_KEY, PRIMARY_SECRET, &[PRIMARY]).await;
    let vault = TestServer::start(VAULT_KEY, VAULT_SECRET, &[VAULT]).await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        options(),
    ));
    for (label, server, bucket, key, secret) in [
        ("Primary", &primary, PRIMARY, PRIMARY_KEY, PRIMARY_SECRET),
        ("Vault", &vault, VAULT, VAULT_KEY, VAULT_SECRET),
    ] {
        accounts
            .add(
                &AccountInput {
                    label: label.into(),
                    provider: "s3-compatible".into(),
                    region: "us-east-1".into(),
                    endpoint: Some(server.endpoint.clone()),
                    bucket: bucket.into(),
                },
                CredentialInput::new(key, secret),
            )
            .unwrap();
    }
    let dir = TempDir::new("backup");
    let backups = Backups::load(
        SqliteBackupStore::open(&dir.path("backup.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn BackupStore>),
        dir.path("spool"),
    );
    Env {
        primary,
        vault,
        keys,
        accounts,
        backups,
        dir,
        files: TempDir::new("backup-files"),
    }
}

fn input(retention: Retention) -> JobInput {
    JobInput {
        id: None,
        name: "Everything".into(),
        source_account: "primary".into(),
        source_prefix: String::new(),
        destination_account: "vault".into(),
        destination_prefix: "sotyware/".into(),
        interval_hours: 24,
        retention,
        suspicious_percent: model::DEFAULT_SUSPICIOUS_PERCENT,
    }
}

impl Env {
    fn store(&self, server: &TestServer, bucket: &str) -> S3Store {
        let id = if bucket == PRIMARY {
            "primary"
        } else {
            "vault"
        };
        S3Store::new(
            server.resolved(id, &[bucket]),
            server.credentials(),
            options(),
        )
    }

    /// Uploads `data` to the primary bucket as sotyware does (with its
    /// SHA-256).
    async fn put(&self, key: &str, data: &[u8]) {
        let path = self
            .files
            .path(&format!("up-{}", sha256_hex(key.as_bytes())));
        fs::write(&path, data).unwrap();
        let s = self.store(&self.primary, PRIMARY);
        if s.head_object(PRIMARY, key).await.unwrap().is_some() {
            fs::remove_file(self.primary.object_path(PRIMARY, key)).unwrap();
            let _ = fs::remove_file(self.primary.metadata_path(PRIMARY, key));
        }
        s.upload_file(PRIMARY, key, &path).await.unwrap();
    }

    /// Writes an object into the primary bucket without sotyware's SHA-256
    /// (another program's upload).
    fn put_foreign(&self, key: &str, data: &[u8]) {
        let p = self.primary.object_path(PRIMARY, key);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }

    fn delete_primary(&self, key: &str) {
        fs::remove_file(self.primary.object_path(PRIMARY, key)).unwrap();
        let _ = fs::remove_file(self.primary.metadata_path(PRIMARY, key));
    }

    fn save(&self, retention: Retention) -> JobConfig {
        let existing = self.backups.jobs().unwrap();
        let job = job_from_input(input(retention), &self.accounts, &existing).unwrap();
        self.backups.save(&job).unwrap();
        job
    }

    async fn task(&self, job: &str, kind: TaskKind) -> TaskSummary {
        let sink = Arc::new(Sink::default());
        let task = self
            .backups
            .start(
                Arc::clone(&self.accounts),
                job,
                kind,
                Arc::clone(&sink) as Arc<dyn BackupSink>,
            )
            .await
            .unwrap();
        sink.finished(&task).await
    }

    async fn restore(&self, job: &str, snapshot: &str, filter: &str, to: RestoreTo) -> TaskSummary {
        self.restore_with(job, snapshot, filter, to, Arc::new(Sink::default()))
            .await
    }

    async fn restore_with(
        &self,
        job: &str,
        snapshot: &str,
        filter: &str,
        to: RestoreTo,
        sink: Arc<Sink>,
    ) -> TaskSummary {
        let task = self
            .backups
            .start_restore(
                Arc::clone(&self.accounts),
                job,
                snapshot,
                filter.into(),
                to,
                Arc::clone(&sink) as Arc<dyn BackupSink>,
            )
            .await
            .unwrap();
        sink.finished(&task).await
    }

    fn health(&self, job: &str) -> Health {
        let list = self.backups.list();
        list.jobs
            .into_iter()
            .find(|j| j.config.id == job)
            .unwrap()
            .health
    }

    /// Every object under the vault prefix.
    fn vault_keys(&self) -> Vec<String> {
        let root = self.vault.object_path(VAULT, "");
        let mut out = Vec::new();
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.push(
                        p.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    fn snapshots(&self) -> Vec<String> {
        self.vault_keys()
            .into_iter()
            .filter(|k| k.starts_with("sotyware/snapshots/"))
            .collect()
    }
}

/// Every file under `dir`, relative path → content.
fn tree(dir: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.insert(
                    p.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

fn data(n: usize, seed: u8) -> Vec<u8> {
    crate::cloud::test_server::pattern(n, seed)
}

fn keep_last(n: u32) -> Retention {
    Retention {
        keep_last: n,
        keep_daily: 0,
        keep_weekly: 0,
    }
}

/// The whole second a snapshot is named after (`20261118T103000Z`).
fn named_second(snapshot: &str) -> &str {
    let name = snapshot.rsplit('/').next().unwrap_or(snapshot);
    name.get(..16).expect("a snapshot key")
}

/// Gives the manifest of `snapshot` the name and `created` time a run whose
/// clock ran `secs` seconds fast would have given it (moved, with its
/// SHA-256 stored again). Returns the new key and time.
async fn date_ahead(e: &Env, snapshot: &str, secs: i64) -> (String, String) {
    let old = e.vault.object_path(VAULT, snapshot);
    let mut m: model::Manifest = serde_json::from_slice(&fs::read(&old).unwrap()).unwrap();
    m.created = engine::shift_rfc3339(&m.created, secs).unwrap();
    let key = model::snapshot_key("sotyware/", &m.job.id, &m.created, &m.snapshot);
    let local = e.files.path(&format!("ahead-{}.json", m.snapshot));
    fs::write(&local, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    e.store(&e.vault, VAULT)
        .upload_file(VAULT, &key, &local)
        .await
        .unwrap();
    fs::remove_file(old).unwrap();
    let _ = fs::remove_file(e.vault.metadata_path(VAULT, snapshot));
    (key, m.created)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_backup_copies_everything_and_restores_byte_identically() {
    let e = env().await;
    let originals: BTreeMap<String, Vec<u8>> = [
        ("photos/2026/a.jpg", data(300_000, 1)),
        ("photos/2026/größe +test.bin", data(70_000, 2)),
        ("docs/empty.txt", Vec::new()),
        ("docs/same-as-a.jpg", data(300_000, 1)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    for (k, v) in &originals {
        e.put(k, v).await;
    }
    e.put_foreign("foreign/other-program.txt", b"written by another tool");
    let job = e.save(Retention::default());
    assert_eq!(e.health(&job.id), Health::NeverRun);
    let seeded = e.primary.ops().len();

    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "completed", "{s:?}");
    let stats = s.stats.unwrap();
    assert_eq!(
        (stats.objects, stats.new, stats.transferred),
        (5, 5, 4),
        "identical content is stored once"
    );
    assert!(s.verification.as_ref().unwrap().passed);
    assert_eq!(e.health(&job.id), Health::Healthy);
    // Repository layout: descriptor, 4 blobs, 1 manifest.
    let keys = e.vault_keys();
    assert!(keys.contains(&"sotyware/sotyware-backup.json".to_owned()));
    assert_eq!(
        keys.iter()
            .filter(|k| k.starts_with("sotyware/blobs/sha256/"))
            .count(),
        4
    );
    assert_eq!(e.snapshots().len(), 1);
    // Nothing was written to the source.
    let ops = e.primary.ops()[seeded..].to_vec();
    assert!(
        ops.iter()
            .all(|o| matches!(o.as_str(), "ListObjectsV2" | "HeadObject" | "GetObject")),
        "the backup only read the source: {ops:?}"
    );

    // Restore everything to a fresh folder: byte-identical.
    let target = e.files.path("restore-all");
    fs::create_dir_all(&target).unwrap();
    let r = e
        .restore(
            &job.id,
            s.snapshot.as_ref().unwrap(),
            "",
            RestoreTo::Local(target.clone()),
        )
        .await;
    assert_eq!(r.state, "completed", "{r:?}");
    let got = tree(&target);
    let mut want = originals.clone();
    want.insert(
        "foreign/other-program.txt".into(),
        b"written by another tool".to_vec(),
    );
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_incremental_backup_transfers_only_changes_and_keeps_history() {
    let e = env().await;
    for (k, v) in [
        ("a.txt", b"one".as_slice()),
        ("b.txt", b"two"),
        ("c.txt", b"three"),
    ] {
        e.put(k, v).await;
    }
    let job = e.save(Retention::default());
    let first = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(first.state, "completed");
    e.put("a.txt", b"one, changed").await;
    e.put("copy-of-b.txt", b"two").await;
    e.delete_primary("c.txt");
    let second = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(second.state, "completed", "{second:?}");
    let st = second.stats.unwrap();
    assert_eq!((st.new, st.changed, st.deleted), (1, 1, 1));
    assert_eq!(
        st.transferred, 1,
        "only the changed content; the copy reuses b's blob"
    );
    assert_eq!(e.snapshots().len(), 2);

    // The first snapshot still restores the old state, deleted file included.
    let old = e.files.path("old");
    fs::create_dir_all(&old).unwrap();
    let r = e
        .restore(
            &job.id,
            first.snapshot.as_ref().unwrap(),
            "",
            RestoreTo::Local(old.clone()),
        )
        .await;
    assert_eq!(r.state, "completed");
    assert_eq!(
        tree(&old),
        BTreeMap::from([
            ("a.txt".into(), b"one".to_vec()),
            ("b.txt".into(), b"two".to_vec()),
            ("c.txt".into(), b"three".to_vec()),
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mass_deletion_is_flagged_and_retention_keeps_the_older_snapshots() {
    let e = env().await;
    for i in 0..20 {
        e.put(&format!("f{i:02}.txt"), format!("file {i}").as_bytes())
            .await;
    }
    let job = e.save(keep_last(1));
    assert_eq!(e.task(&job.id, TaskKind::Backup).await.state, "completed");
    let before = e.snapshots();
    for i in 0..15 {
        e.delete_primary(&format!("f{i:02}.txt"));
    }
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "completed");
    assert!(s.suspicious);
    assert!(s.prune.is_none());
    assert!(
        s.prune_skipped
            .as_deref()
            .unwrap()
            .contains("older snapshots are kept")
    );
    assert_eq!(e.snapshots().len(), 2, "nothing pruned");
    assert_eq!(e.health(&job.id), Health::LargeChange);
    // A retention task is refused too until the user has reviewed it.
    let p = e.task(&job.id, TaskKind::Prune).await;
    assert!(p.prune.is_none() && p.prune_skipped.is_some(), "{p:?}");

    // Disaster recovery: put the deleted files back into the primary bucket
    // (under a new folder, never overwriting) from the older snapshot.
    let r = e
        .restore(
            &job.id,
            &format!("sotyware/{}", before[0].trim_start_matches("sotyware/")),
            "",
            RestoreTo::Bucket {
                account: "primary".into(),
                prefix: "restored/".into(),
            },
        )
        .await;
    assert_eq!(r.state, "completed", "{r:?}");
    assert_eq!(r.restore.as_ref().unwrap().restored, 20);
    let s = e.store(&e.primary, PRIMARY);
    for i in 0..20 {
        let p = e.files.path(&format!("check-{i}"));
        s.download_file(PRIMARY, &format!("restored/f{i:02}.txt"), &p)
            .await
            .unwrap();
        assert_eq!(fs::read(&p).unwrap(), format!("file {i}").as_bytes());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_deletes_only_dropped_snapshots_and_unreferenced_old_blobs() {
    let e = env().await;
    e.put("a.txt", b"version 1").await;
    e.put("b.txt", b"stays").await;
    let job = e.save(keep_last(1));
    let first = e.task(&job.id, TaskKind::Backup).await;
    let old_blob = model::blob_key("sotyware/", &sha256_hex(b"version 1"));
    e.put("a.txt", b"version 2").await;
    let second = e.task(&job.id, TaskKind::Backup).await;
    let p = second
        .prune
        .as_ref()
        .expect("retention ran after a verified backup");
    assert_eq!(p.snapshots_deleted, 1);
    assert_eq!(
        p.blobs_deleted, 0,
        "the old blob is younger than the safety margin"
    );
    assert_eq!(e.snapshots(), [second.snapshot.clone().unwrap()]);
    assert!(e.vault_keys().contains(&old_blob));

    // With the margin at zero (as after a day), the unreferenced blob goes,
    // and only it.
    let stores = prepare(&e.accounts, &job).await.unwrap();
    let cancel = Cancel::default();
    let mut nothing = |_: Progress| {};
    let spool = e.dir.path("spool-prune");
    fs::create_dir_all(&spool).unwrap();
    let mut ctx = Ctx {
        dest: &stores.dest,
        dest_bucket: VAULT,
        dest_prefix: "sotyware/",
        spool: &spool,
        cancel: &cancel,
        journal: None,
        progress: &mut nothing,
    };
    let p = engine::prune_with_margin(&mut ctx, &job, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!((p.snapshots_deleted, p.blobs_deleted), (0, 1));
    assert!(!e.vault_keys().contains(&old_blob));
    assert!(first.snapshot.is_some());
    // What is left still verifies and restores.
    let v = e.task(&job.id, TaskKind::VerifyFull).await;
    assert!(v.verification.unwrap().passed);
    let out = e.files.path("after-prune");
    fs::create_dir_all(&out).unwrap();
    e.restore(
        &job.id,
        &second.snapshot.unwrap(),
        "",
        RestoreTo::Local(out.clone()),
    )
    .await;
    assert_eq!(
        tree(&out),
        BTreeMap::from([
            ("a.txt".into(), b"version 2".to_vec()),
            ("b.txt".into(), b"stays".to_vec())
        ])
    );
}

/// Deletes `snapshot` of `job` through the service, as the page does.
async fn delete_snapshot(e: &Env, job: &str, snapshot: &str) -> TaskSummary {
    let sink = Arc::new(Sink::default());
    let task = e
        .backups
        .start_delete_snapshot(
            Arc::clone(&e.accounts),
            job,
            snapshot,
            Arc::clone(&sink) as Arc<dyn BackupSink>,
        )
        .await
        .unwrap();
    sink.finished(&task).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_snapshot_is_deleted_with_only_the_data_no_other_snapshot_uses() {
    let e = env().await;
    e.put("a.txt", b"only in the first").await;
    e.put("b.txt", b"in both").await;
    let job = e.save(Retention::default());
    let first = e.task(&job.id, TaskKind::Backup).await;
    let first_key = first.snapshot.clone().unwrap();
    e.delete_primary("a.txt");
    e.put("c.txt", b"only in the second").await;
    let second = e.task(&job.id, TaskKind::Backup).await;
    let second_key = second.snapshot.clone().unwrap();
    assert_eq!(e.snapshots(), [first_key.clone(), second_key.clone()]);
    let only_first = model::blob_key("sotyware/", &sha256_hex(b"only in the first"));
    let shared = model::blob_key("sotyware/", &sha256_hex(b"in both"));
    let only_second = model::blob_key("sotyware/", &sha256_hex(b"only in the second"));

    // Details read from the manifest, and this computer's verification.
    let runs = e.backups.runs(&job.id).unwrap();
    let stores = prepare(&e.accounts, &job).await.unwrap();
    let cancel = Cancel::default();
    let mut nothing = |_: Progress| {};
    let ctx = Ctx {
        dest: &stores.dest,
        dest_bucket: VAULT,
        dest_prefix: "sotyware/",
        spool: Path::new("."),
        cancel: &cancel,
        journal: None,
        progress: &mut nothing,
    };
    let m = engine::read_manifest(&ctx, &first_key).await.unwrap();
    let d = snapshot_details(&first_key, &m, &runs);
    assert_eq!((d.stats.objects, d.source_bucket.as_str()), (2, PRIMARY));
    assert_eq!(
        (d.verified, d.verified_full),
        (Some(true), false),
        "verified after the run"
    );
    assert!(d.verified_ms.is_some());

    // With the service's 24-hour margin, the manifest goes at once; the
    // data it alone used stays until it is older than a day.
    let s = delete_snapshot(&e, &job.id, &first_key).await;
    assert_eq!(s.state, "completed", "{:?}", s.error);
    assert_eq!(s.kind, TaskKind::DeleteSnapshot);
    let p = s.prune.unwrap();
    assert_eq!(
        (p.snapshots_deleted, p.snapshots_kept, p.blobs_deleted),
        (1, 1, 0)
    );
    assert_eq!(e.snapshots(), std::slice::from_ref(&second_key));
    assert!(
        e.vault_keys().contains(&only_first),
        "younger than the margin"
    );

    // A key that is not (or no longer) a snapshot of the job: refused,
    // nothing deleted.
    let s = delete_snapshot(&e, &job.id, &first_key).await;
    assert_eq!(s.state, "failed");
    assert_eq!(
        s.error.as_ref().map(|e| e.kind),
        Some(ErrorKind::ObjectNotFound)
    );
    let e2 = e
        .backups
        .start_delete_snapshot(
            Arc::clone(&e.accounts),
            &job.id,
            "sotyware/blobs/x.json",
            Arc::new(Sink::default()),
        )
        .await
        .unwrap_err();
    assert_eq!(e2.kind, ErrorKind::InvalidInput);
    assert_eq!(e.snapshots(), std::slice::from_ref(&second_key));

    // Once the data is old enough, only the blob no snapshot uses goes.
    let spool = e.dir.path("spool-delete");
    fs::create_dir_all(&spool).unwrap();
    let mut ctx = Ctx {
        dest: &stores.dest,
        dest_bucket: VAULT,
        dest_prefix: "sotyware/",
        spool: &spool,
        cancel: &cancel,
        journal: None,
        progress: &mut nothing,
    };
    let e3 = engine::delete_snapshot_with_margin(&mut ctx, &job, &first_key, Duration::ZERO)
        .await
        .unwrap_err();
    assert!(matches!(e3, StorageError::NotFound(_)), "{e3:?}");
    let p = engine::prune_with_margin(
        &mut ctx,
        &JobConfig {
            retention: keep_last(5),
            ..job.clone()
        },
        Duration::ZERO,
    )
    .await
    .unwrap();
    assert_eq!((p.snapshots_deleted, p.blobs_deleted), (0, 1));
    let keys = e.vault_keys();
    assert!(!keys.contains(&only_first));
    assert!(keys.contains(&shared) && keys.contains(&only_second));
    // What is left verifies.
    let v = e.task(&job.id, TaskKind::VerifyFull).await;
    assert!(v.verification.unwrap().passed);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_deleting_the_newest_snapshot_health_rests_on_the_one_before() {
    let e = env().await;
    e.put("a.txt", b"one").await;
    let job = e.save(Retention::default());
    let first = e.task(&job.id, TaskKind::Backup).await;
    e.put("a.txt", b"two").await;
    let second = e.task(&job.id, TaskKind::Backup).await;
    let status = e.backups.status(job.clone(), now_ms()).unwrap();
    assert_eq!(status.last_snapshot, second.snapshot);
    let s = delete_snapshot(&e, &job.id, second.snapshot.as_deref().unwrap()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error);
    let status = e.backups.status(job.clone(), now_ms()).unwrap();
    assert_eq!(
        status.last_snapshot, first.snapshot,
        "the deleted one no longer counts"
    );
    assert_eq!(status.health, Health::Healthy, "the first was verified");
    // Deleting the only one left: nothing vouches for the backup any more.
    let s = delete_snapshot(&e, &job.id, first.snapshot.as_deref().unwrap()).await;
    assert_eq!(s.state, "completed");
    let status = e.backups.status(job, now_ms()).unwrap();
    assert_eq!(status.last_snapshot, None);
    assert_eq!(status.health, Health::NeverRun);
}

/// A run whose clock reads before the newest snapshot dates its own a
/// second after it (ADR-051). The first snapshot here carries the date of a
/// clock that ran a day fast and was then set right: the next one is named
/// after it, so retention keeps the new snapshot, not the one dated ahead,
/// and it restores the new state.
#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_is_dated_after_one_dated_ahead_and_retention_keeps_it() {
    let e = env().await;
    e.put("a.txt", b"before").await;
    let job = e.save(keep_last(1));
    let first = e.task(&job.id, TaskKind::Backup).await;
    let (ahead, ahead_created) =
        date_ahead(&e, first.snapshot.as_deref().unwrap(), 24 * 3600).await;
    e.put("a.txt", b"after").await;
    let second = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(second.state, "completed", "{second:?}");
    let key = second.snapshot.clone().unwrap();
    let after_ahead = engine::shift_rfc3339(&ahead_created, 1).unwrap();
    assert_eq!(named_second(&key), model::compact_time(&after_ahead));
    let m: model::Manifest =
        serde_json::from_slice(&fs::read(e.vault.object_path(VAULT, &key)).unwrap()).unwrap();
    assert_eq!(m.created, after_ahead, "dated as it is named");
    assert_eq!(
        second.prune.as_ref().map(|p| p.snapshots_deleted),
        Some(1),
        "{second:?}"
    );
    assert_eq!(
        e.snapshots(),
        std::slice::from_ref(&key),
        "the one dated ahead went"
    );
    assert!(!e.vault_keys().contains(&ahead));
    assert_eq!(e.health(&job.id), Health::Healthy);
    let out = e.files.path("after-ahead");
    fs::create_dir_all(&out).unwrap();
    let r = e
        .restore(&job.id, &key, "", RestoreTo::Local(out.clone()))
        .await;
    assert_eq!(r.state, "completed", "{r:?}");
    assert_eq!(
        tree(&out),
        BTreeMap::from([("a.txt".into(), b"after".to_vec())])
    );
}

/// Health, the last backup and the newest snapshot follow the order runs
/// were recorded in, never their wall-clock times (ADR-051): here each run
/// reads an earlier time than the one before, as on a clock set back
/// between all of them.
#[test]
fn health_follows_the_order_runs_were_recorded_in_not_their_times() {
    let dir = TempDir::new("backup-order");
    let backups = Backups::load(
        SqliteBackupStore::open(&dir.path("backup.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn BackupStore>),
        dir.path("spool"),
    );
    let job = JobConfig {
        id: "photos".into(),
        name: "Photos".into(),
        source: Location {
            account: "primary".into(),
            bucket: PRIMARY.into(),
            prefix: String::new(),
        },
        destination: Location {
            account: "vault".into(),
            bucket: VAULT.into(),
            prefix: "sotyware/".into(),
        },
        interval_hours: 24,
        retention: Retention::default(),
        suspicious_percent: model::DEFAULT_SUSPICIOUS_PERCENT,
    };
    backups.save(&job).unwrap();
    let first = "sotyware/snapshots/photos/20261119T100000Z-aa.json";
    let second = "sotyware/snapshots/photos/20261119T100001Z-bb.json";
    let now = 1_000_000_000;
    let mut clock = now;
    let mut record = |kind: &str, snapshot: &str, suspicious: bool, passed: Option<bool>| {
        clock -= 1_000;
        backups.record(RunRow {
            job_id: job.id.clone(),
            kind: kind.into(),
            state: "completed".into(),
            started_ms: clock,
            finished_ms: Some(clock + 500),
            snapshot: Some(snapshot.into()),
            suspicious,
            passed,
            ..RunRow::default()
        });
        clock
    };
    let status = || backups.status(job.clone(), now).unwrap();

    record("backup", first, false, None);
    assert_eq!(status().health, Health::NotVerified);
    record("verify", first, false, Some(true));
    assert_eq!(status().health, Health::Healthy, "verified after the run");
    record("verify-full", first, false, Some(false));
    assert_eq!(
        status().health,
        Health::VerificationFailed,
        "the last verification failed"
    );
    let started = record("backup", second, true, None);
    let s = status();
    assert_eq!(s.last_snapshot.as_deref(), Some(second));
    assert_eq!(
        s.last_backup.and_then(|r| r.snapshot).as_deref(),
        Some(second)
    );
    assert_eq!(s.next_due_ms, Some(started + 24 * 3_600_000));
    assert_eq!(s.health, Health::NotVerified);
    record("verify", second, false, Some(true));
    assert_eq!(status().health, Health::LargeChange);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_deletion_waits_for_the_slot_and_is_refused_as_a_plain_task() {
    let e = env().await;
    e.put("a.txt", b"x").await;
    let job = e.save(Retention::default());
    let e1 = e
        .backups
        .start(
            Arc::clone(&e.accounts),
            &job.id,
            TaskKind::DeleteSnapshot,
            Arc::new(Sink::default()),
        )
        .await
        .unwrap_err();
    assert_eq!(e1.kind, ErrorKind::InvalidInput);
    // While another task holds the slot, a deletion is refused as busy.
    let sink = Arc::new(Sink::default());
    let task = e
        .backups
        .start(
            Arc::clone(&e.accounts),
            &job.id,
            TaskKind::Backup,
            Arc::clone(&sink) as Arc<dyn BackupSink>,
        )
        .await
        .unwrap();
    let busy = e
        .backups
        .start_delete_snapshot(
            Arc::clone(&e.accounts),
            &job.id,
            "sotyware/snapshots/everything/20261115T000000Z-x.json",
            Arc::new(Sink::default()),
        )
        .await;
    if let Err(err) = busy {
        assert_eq!(err.kind, ErrorKind::Busy);
    }
    sink.finished(&task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn damaged_or_missing_backup_data_fails_verification_and_is_never_healthy() {
    let e = env().await;
    e.put("a.bin", &data(50_000, 3)).await;
    e.put("b.bin", &data(40_000, 4)).await;
    let job = e.save(Retention::default());
    assert_eq!(e.task(&job.id, TaskKind::Backup).await.state, "completed");
    assert_eq!(e.health(&job.id), Health::Healthy);

    // Same-size corruption at the destination: the quick check (metadata)
    // cannot see it, the full check (content) does.
    let blob_a = model::blob_key("sotyware/", &sha256_hex(&data(50_000, 3)));
    fs::write(e.vault.object_path(VAULT, &blob_a), data(50_000, 99)).unwrap();
    let quick = e.task(&job.id, TaskKind::Verify).await;
    assert!(quick.verification.unwrap().passed);
    let full = e.task(&job.id, TaskKind::VerifyFull).await;
    let v = full.verification.unwrap();
    assert!(!v.passed);
    assert_eq!(v.damaged, [blob_a.trim_start_matches("sotyware/")]);
    assert_eq!(full.state, "failed");
    assert_eq!(e.health(&job.id), Health::VerificationFailed);

    // A missing blob fails even the quick check.
    let blob_b = model::blob_key("sotyware/", &sha256_hex(&data(40_000, 4)));
    fs::remove_file(e.vault.object_path(VAULT, &blob_b)).unwrap();
    let quick = e.task(&job.id, TaskKind::Verify).await;
    assert_eq!(quick.verification.unwrap().missing.len(), 1);
    assert_eq!(e.health(&job.id), Health::VerificationFailed);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_backup_writes_no_snapshot_and_the_next_run_reuses_its_blobs() {
    let e = env().await;
    for i in 0..6 {
        e.put(&format!("big{i}.bin"), &data(200_000, i)).await;
    }
    let job = e.save(Retention::default());
    let sink = Arc::new(Sink::default());
    *sink.cancel_at.lock().unwrap() = Some((3, Arc::clone(&e.backups)));
    let task = e
        .backups
        .start(
            Arc::clone(&e.accounts),
            &job.id,
            TaskKind::Backup,
            Arc::clone(&sink) as Arc<dyn BackupSink>,
        )
        .await
        .unwrap();
    let s = sink.finished(&task).await;
    assert_eq!(s.state, "cancelled", "{s:?}");
    assert!(s.snapshot.is_none());
    assert!(
        e.snapshots().is_empty(),
        "no manifest describes an incomplete state"
    );
    let stored = e
        .vault_keys()
        .iter()
        .filter(|k| k.contains("/blobs/"))
        .count();
    assert!(
        (3..6).contains(&stored),
        "{stored} blobs stored before the cancel"
    );
    assert_eq!(e.health(&job.id), Health::NeverRun);

    let again = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(again.state, "completed");
    assert_eq!(
        again.stats.unwrap().transferred as usize,
        6 - stored,
        "blobs stored by the interrupted run are reused"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_destination_or_wrong_key_fails_without_touching_anything() {
    let e = env().await;
    e.put("a.txt", b"a").await;
    let job = e.save(Retention::default());
    // Expired / replaced key at the backup provider.
    e.keys.entries.lock().unwrap().insert(
        "vault".into(),
        Credentials {
            access_key_id: VAULT_KEY.into(),
            secret_access_key: Secret::new("wrong-secret-0123456789".to_owned()),
        },
    );
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "failed");
    // A refused HEAD has no body: a wrong secret and a missing permission
    // look the same (ADR-018).
    assert!(matches!(
        s.error.unwrap().kind,
        ErrorKind::InvalidCredentials | ErrorKind::AccessDenied
    ),);
    assert_eq!(e.health(&job.id), Health::Failed);
    assert!(e.vault_keys().is_empty());

    // The destination server is gone.
    e.keys.entries.lock().unwrap().insert(
        "vault".into(),
        Credentials {
            access_key_id: VAULT_KEY.into(),
            secret_access_key: Secret::new(VAULT_SECRET.to_owned()),
        },
    );
    e.vault
        .inject("HeadObject", None, usize::MAX, Inject::InternalError);
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "failed", "{s:?}");
    assert_eq!(e.health(&job.id), Health::Failed);
    // Nothing was written to the source either way (one PutObject: the
    // test's own seeding).
    assert_eq!(
        e.primary.ops().iter().filter(|o| *o == "PutObject").count(),
        1
    );
    assert!(!e.primary.ops().iter().any(|o| o == "DeleteObject"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_local_database_loses_no_backup() {
    let e = env().await;
    e.put("keep/me.txt", b"precious").await;
    let job = e.save(Retention::default());
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "completed");
    // A new installation: empty backup database, same accounts re-entered.
    let fresh = Backups::load(
        SqliteBackupStore::open(&e.dir.path("fresh.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn BackupStore>),
        e.dir.path("spool2"),
    );
    let job2 = job_from_input(input(Retention::default()), &e.accounts, &[]).unwrap();
    assert_eq!(job2.id, job.id, "the same name gives the same job id");
    fresh.save(&job2).unwrap();
    let env2 = Env {
        backups: fresh,
        ..e
    };
    let snaps = {
        let stores = prepare(&env2.accounts, &job2).await.unwrap();
        let cancel = Cancel::default();
        let mut nothing = |_: Progress| {};
        let ctx = Ctx {
            dest: &stores.dest,
            dest_bucket: VAULT,
            dest_prefix: "sotyware/",
            spool: &env2.dir.0,
            cancel: &cancel,
            journal: None,
            progress: &mut nothing,
        };
        engine::snapshots(&ctx, &job2.id).await.unwrap()
    };
    assert_eq!(snaps.len(), 1, "found at the destination");
    let out = env2.files.path("fresh-restore");
    fs::create_dir_all(&out).unwrap();
    let r = env2
        .restore(&job2.id, &snaps[0].key, "", RestoreTo::Local(out.clone()))
        .await;
    assert_eq!(r.state, "completed");
    assert_eq!(
        tree(&out),
        BTreeMap::from([("keep/me.txt".into(), b"precious".to_vec())])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restores_never_overwrite_and_an_interrupted_restore_can_be_repeated() {
    let e = env().await;
    for i in 0..5 {
        e.put(&format!("dir/f{i}.txt"), format!("content {i}").as_bytes())
            .await;
    }
    e.put("other/x.txt", b"x").await;
    let job = e.save(Retention::default());
    let snap = e.task(&job.id, TaskKind::Backup).await.snapshot.unwrap();
    let out = e.files.path("restore");
    fs::create_dir_all(out.join("dir")).unwrap();
    fs::write(out.join("dir/f0.txt"), b"the user's own newer file").unwrap();

    // Interrupted after two files.
    let sink = Arc::new(Sink::default());
    *sink.cancel_at.lock().unwrap() = Some((2, Arc::clone(&e.backups)));
    let r = e
        .restore_with(&job.id, &snap, "dir/", RestoreTo::Local(out.clone()), sink)
        .await;
    assert_eq!(r.state, "cancelled");
    // Repeated: every file of the folder ends up there, the user's file is
    // untouched, and files restored the first time are not overwritten.
    let r = e
        .restore(&job.id, &snap, "dir/", RestoreTo::Local(out.clone()))
        .await;
    assert_eq!(r.state, "completed", "{r:?}");
    let t = tree(&out);
    assert_eq!(t["dir/f0.txt"], b"the user's own newer file");
    assert_eq!(t["dir/f0 (1).txt"], b"content 0");
    for i in 1..5 {
        assert_eq!(
            t[&format!("dir/f{i}.txt")],
            format!("content {i}").as_bytes()
        );
    }
    assert!(
        !t.contains_key("other/x.txt"),
        "the filter restores only dir/"
    );
    assert!(t.keys().all(|k| k.starts_with("dir/")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_source_object_is_never_backed_up_silently() {
    let e = env().await;
    e.put("good.txt", b"fine").await;
    e.put("rotten.txt", b"original").await;
    // Bit rot at the provider: same size, different bytes, stale checksum.
    fs::write(e.primary.object_path(PRIMARY, "rotten.txt"), b"origiNAL").unwrap();
    let job = e.save(Retention::default());
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "partial", "{s:?}");
    assert!(s.snapshot.is_none());
    assert_eq!(s.object_errors.len(), 1);
    assert_eq!(s.object_errors[0].key, "rotten.txt");
    assert!(
        s.object_errors[0].message.contains("SHA-256"),
        "{}",
        s.object_errors[0].message
    );
    assert_eq!(e.health(&job.id), Health::Partial);
}

#[test]
fn job_inputs_take_the_accounts_buckets_and_unique_ids() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let e = rt.block_on(env());
    let a = job_from_input(input(Retention::default()), &e.accounts, &[]).unwrap();
    assert_eq!(
        (
            a.id.as_str(),
            a.source.bucket.as_str(),
            a.destination.bucket.as_str()
        ),
        ("everything", PRIMARY, VAULT)
    );
    let b = job_from_input(
        input(Retention::default()),
        &e.accounts,
        std::slice::from_ref(&a),
    )
    .unwrap();
    assert_eq!(b.id, "everything-2");
    let mut bad = input(Retention::default());
    bad.destination_account = "missing".into();
    assert_eq!(
        job_from_input(bad, &e.accounts, &[]).unwrap_err().kind,
        ErrorKind::UnknownAccount
    );
    let mut same = input(Retention::default());
    same.destination_account = "primary".into();
    same.destination_prefix = "backup/".into();
    let j = job_from_input(same, &e.accounts, &[]).unwrap();
    assert!(
        e.backups.save(&j).is_err(),
        "a destination inside the source is refused"
    );
    drop(e);
}

#[test]
fn protection_is_described_as_it_is() {
    let p = |lock: bool, mode: Option<&str>, days: Option<u32>, versioning: &str| {
        let mut p = Protection {
            same_account: false,
            versioning: versioning.into(),
            object_lock: lock,
            default_mode: mode.map(str::to_owned),
            default_days: days,
            immutable: lock && mode == Some("COMPLIANCE") && days.is_some(),
            summary: String::new(),
        };
        p.summary = describe_protection(&p);
        p
    };
    let c = p(true, Some("COMPLIANCE"), Some(30), "enabled");
    assert!(c.immutable && c.summary.contains("not even you"));
    let g = p(true, Some("GOVERNANCE"), Some(30), "enabled");
    assert!(!g.immutable && g.summary.contains("bypass"));
    let off = p(false, None, None, "disabled");
    assert!(
        !off.immutable
            && off.summary.contains("Object Lock is off")
            && off.summary.contains("for good")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_destination_protection_is_read_from_the_bucket() {
    let e = env().await;
    let job = e.save(Retention::default());
    let stores = prepare(&e.accounts, &job).await.unwrap();
    let p = protection(&stores.dest, &job).await.unwrap();
    // s3s-fs has neither versioning nor Object Lock: never called immutable.
    assert!(!p.immutable);
    assert!(!p.same_account);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scheduler_runs_a_due_job_and_records_it() {
    let e = env().await;
    e.put("a.txt", b"a").await;
    let job = e.save(Retention::default());
    assert_eq!(
        e.backups.next_job(journal_now()).map(|(j, _)| j),
        Some(job.id.clone())
    );
    // A due job runs: the same task as "Back up now", without a listener.
    let sink: Arc<dyn BackupSink> = Arc::new(Nowhere);
    let task = e
        .backups
        .start(Arc::clone(&e.accounts), &job.id, TaskKind::Backup, sink)
        .await
        .unwrap();
    assert!(task.starts_with("backup-"));
    let t = Instant::now();
    while e.backups.busy() {
        assert!(t.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (_, due) = e.backups.next_job(journal_now()).unwrap();
    assert!(due > journal_now() + 23 * 3_600_000, "next run a day later");
    assert_eq!(e.health(&job.id), Health::Healthy);
    // One task at a time.
    let _hold = e.backups.reserve(&job.id, TaskKind::Verify).unwrap();
    let busy = e
        .backups
        .start(
            Arc::clone(&e.accounts),
            &job.id,
            TaskKind::Verify,
            Arc::new(Nowhere),
        )
        .await
        .unwrap_err();
    assert_eq!(busy.kind, ErrorKind::Busy);
    e.backups.free();
}

fn journal_now() -> i64 {
    crate::journal::now_ms()
}

#[allow(dead_code)]
fn unused(_: PathBuf) {}

/// The interpreter for the recovery script: `python3`, or on Windows
/// `python` / `py -3`.
fn python() -> Option<Vec<&'static str>> {
    let candidates: &[&[&str]] = &[&["python3"], &["python"], &["py", "-3"]];
    candidates.iter().find_map(|c| {
        std::process::Command::new(c[0])
            .args(&c[1..])
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| c.to_vec())
    })
}

/// Disaster recovery without sotyware: a backup made by sotyware is
/// restored by `scripts/recovery/sotyware-restore.py` alone (Python
/// standard library, SigV4 to the backup server), byte for byte. Needs
/// Python 3; CI runs it on Linux.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Python 3: cargo test --lib backup::tests::the_standalone -- --ignored"]
async fn the_standalone_recovery_script_restores_a_snapshot_byte_identically() {
    let py = python().expect("Python 3 is needed for this test");
    let e = env().await;
    let originals = [
        ("photos/a.jpg", data(150_000, 7)),
        ("photos/größe +x.bin", data(1_000, 8)),
        ("docs/empty.txt", Vec::new()),
    ];
    for (k, v) in &originals {
        e.put(k, v).await;
    }
    let job = e.save(Retention::default());
    let s = e.task(&job.id, TaskKind::Backup).await;
    assert_eq!(s.state, "completed");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts/recovery/sotyware-restore.py");
    let run = |args: &[&str]| {
        std::process::Command::new(py[0])
            .args(&py[1..])
            .arg("-I")
            .arg(&script)
            .args([
                "--endpoint",
                &e.vault.endpoint,
                "--region",
                "us-east-1",
                "--bucket",
                VAULT,
                "--prefix",
                "sotyware/",
            ])
            .args(args)
            .env("AWS_ACCESS_KEY_ID", VAULT_KEY)
            .env("AWS_SECRET_ACCESS_KEY", VAULT_SECRET)
            .output()
            .unwrap()
    };
    let list = run(&["--list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&list.stdout).trim(),
        s.snapshot.as_deref().unwrap()
    );
    let out = e.files.path("script-restore");
    let r = run(&["--latest", &job.id, "--out", out.to_str().unwrap()]);
    assert!(
        r.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
    let want: BTreeMap<String, Vec<u8>> = originals
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    assert_eq!(tree(&out), want);
    // A second run never replaces what is there.
    let again = run(&["--latest", &job.id, "--out", out.to_str().unwrap()]);
    assert!(again.status.success());
    let t = tree(&out);
    assert_eq!(t["photos/a.jpg"], data(150_000, 7));
    assert_eq!(t["photos/a (1).jpg"], data(150_000, 7));
    // A damaged blob is detected and its file removed again.
    let blob = model::blob_key("sotyware/", &sha256_hex(&data(1_000, 8)));
    fs::write(e.vault.object_path(VAULT, &blob), data(1_000, 9)).unwrap();
    let out2 = e.files.path("script-damaged");
    let bad = run(&["--latest", &job.id, "--out", out2.to_str().unwrap()]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("does not match"));
    assert!(!out2.join("photos/größe +x.bin").exists());
}

/// The spool is emptied without recursive deletion: only sotyware's own
/// spool files go, then their task folders if that emptied them. A file
/// someone else put there, a link and what the link points to stay.
#[test]
fn the_spool_cleanup_removes_only_spool_files() {
    let dir = TempDir::new("spool-cleanup");
    let spool = dir.path("backup-spool");
    let task = spool.join("0123abcd");
    let busy = spool.join("export-77aa");
    fs::create_dir_all(&task).unwrap();
    fs::create_dir_all(&busy).unwrap();
    fs::write(task.join("object-1a2b"), b"spooled").unwrap();
    fs::write(busy.join("object-3c4d"), b"spooled").unwrap();
    fs::write(busy.join("notes.txt"), b"not sotyware's").unwrap();
    fs::write(spool.join("readme"), b"not sotyware's").unwrap();
    let outside = dir.path("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("object-5e6f"), b"must stay").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, spool.join("linked")).unwrap();

    clear_spool(&spool);

    assert!(!task.exists(), "an emptied task folder is removed");
    assert!(!busy.join("object-3c4d").exists());
    assert!(busy.join("notes.txt").exists(), "a foreign file stays");
    assert!(busy.exists(), "and so does its folder");
    assert!(spool.join("readme").exists());
    assert!(
        outside.join("object-5e6f").exists(),
        "a link is never followed"
    );
    #[cfg(unix)]
    assert!(spool.join("linked").symlink_metadata().is_ok());
}

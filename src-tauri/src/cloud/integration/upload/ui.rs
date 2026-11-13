//! Opt-in: the provider side of the upload test through the Windows
//! dashboard (`tests/desktop/windows-cloud-ui.ps1 -Mode b2-upload`,
//! docs/development/testing.md#uploads-through-the-windows-dashboard-at-backblaze).
//! The dashboard itself (release binary, native file dialog, its own key)
//! uploads; these tests only prepare the run, check what arrived and remove
//! it, with the write key, through the same [`RunScope`] as the batch test.
//!
//! The driver writes the local files first, into
//! `SOTYWARE_IT_B2_UPLOAD_UI_DIR` ([`LocalFiles`]): `seed/existing.txt` and
//! `batch/` with exactly [`UI_NAMES`] (its `existing.txt` differs from the
//! seed). The run id comes from the driver (`SOTYWARE_IT_B2_UPLOAD_UI_RUN`),
//! so it is known before the first write and the cleanup can always run.
//!
//! - [`backblaze_b2_upload_ui_prepare`]: the new run prefix
//!   `sotyware-it/upload-ui-validation/<run id>/` must hold no version,
//!   delete marker or incomplete multipart upload; then `seed/existing.txt`
//!   is uploaded to `<prefix>existing.txt`, which makes the run folder
//!   visible to the browser and is the object the dashboard must not
//!   overwrite. Requests: `ListObjectVersions`, `ListMultipartUploads`,
//!   `HeadObject`, `PutObject`, `HeadObject` (5).
//! - [`backblaze_b2_upload_ui_verify`]: downloads each of the four keys once
//!   through [`CloudDownloads`] and compares it with the local file (the
//!   seed for `existing.txt`), with the SHA-256 sotyware stored checked by
//!   the download; then the run's cleanup (aborts incomplete multipart
//!   uploads by upload id, deletes this run's versions by version id, lists
//!   again) and checks on what it found: exactly the four keys, one version
//!   each, no delete marker, `existing.txt` still the seed's version, no
//!   incomplete multipart upload. Requests: 4 `GetObject`, then 2 listings,
//!   4 `DeleteObject` with a version id, 2 listings (12).
//! - [`backblaze_b2_upload_ui_cleanup`]: only the cleanup, for a run that
//!   was interrupted (or whose dashboard part never uploaded).
//!
//! Each needs `--ignored`, the B2 account variables (write key) and
//! `SOTYWARE_IT_B2_UPLOAD_CONFIRM=upload-validation`, like the batch test.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::download::{assert_backblaze, fail, sha256_of};
use super::super::{Requests, b2_store};
use super::{
    Cleanup, RunScope, Source, UNICODE, accounts_with, cleanup_run, count, download_and_compare,
    in_scope, list_open_uploads, list_versions, require_confirmation, scoped_store, show_requests,
};
use crate::cloud::dashboard::AccountInput;
use crate::cloud::provider::BACKBLAZE_B2;
use crate::cloud::s3::{ClientOptions, UploadPlan, plan_upload};
use crate::cloud::test_server::TempDir;

/// Every dashboard upload run lives below this, inside the write key's
/// prefix.
const UI_ROOT: &str = "sotyware-it/upload-ui-validation/";
const RUN_VAR: &str = "SOTYWARE_IT_B2_UPLOAD_UI_RUN";
const DIR_VAR: &str = "SOTYWARE_IT_B2_UPLOAD_UI_DIR";
const SEED_VERSION_VAR: &str = "SOTYWARE_IT_B2_UPLOAD_UI_SEED_VERSION";

/// Uploaded by the prepare test; the dashboard's file of the same name must
/// be skipped.
const SEED: &str = "existing.txt";
const SMALL: &str = "a-small.txt";
/// Large enough for the page to show "Sending" with a percentage, small
/// enough for one `PutObject` with the release binary's settings.
const PROGRESS: &str = "progress.bin";
/// Every key a run may create, and so the only ones it may remove; also the
/// exact contents of the local `batch/` folder.
const UI_NAMES: [&str; 4] = [SEED, SMALL, UNICODE, PROGRESS];

/// The run prefix for `run`, which may not leave [`UI_ROOT`].
fn ui_run_prefix(run: &str) -> String {
    assert!(
        !run.is_empty()
            && run.len() <= 64
            && run
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "a run id is 1 to 64 letters, digits, '-' and '_'"
    );
    format!("{UI_ROOT}{run}/")
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail(format!("set {name}; nothing was sent")))
}

/// The local files of a run, as the driver wrote them.
#[derive(Clone, Debug)]
struct LocalFiles {
    seed: Source,
    /// In [`UI_NAMES`] order.
    batch: Vec<Source>,
}

impl LocalFiles {
    /// Reads and checks `dir`: `seed/` holds exactly [`SEED`], `batch/`
    /// exactly [`UI_NAMES`], all regular files; each batch file is one
    /// `PutObject` for the release binary at Backblaze; the batch's
    /// `existing.txt` differs from the seed. Nothing is sent.
    fn read(dir: &Path) -> Result<Self, String> {
        fn names(dir: &Path) -> Result<BTreeSet<String>, String> {
            let mut out = BTreeSet::new();
            for e in fs::read_dir(dir).map_err(|e| format!("{dir:?}: {e}"))? {
                let e = e.map_err(|e| format!("{dir:?}: {e}"))?;
                let name = e
                    .file_name()
                    .into_string()
                    .map_err(|n| format!("{n:?} is not UTF-8"))?;
                out.insert(name);
            }
            Ok(out)
        }
        fn source(dir: &Path, name: &'static str) -> Result<Source, String> {
            let path = dir.join(name);
            let meta = fs::metadata(&path).map_err(|e| format!("{path:?}: {e}"))?;
            if !meta.is_file() {
                return Err(format!("{path:?} is not a regular file"));
            }
            Ok(Source {
                name,
                sha256: sha256_of(&path),
                size: meta.len(),
                path,
            })
        }
        let seed_dir = dir.join("seed");
        let batch_dir = dir.join("batch");
        let want_seed: BTreeSet<String> = [SEED.to_owned()].into();
        let want_batch: BTreeSet<String> = UI_NAMES.iter().map(|n| (*n).to_owned()).collect();
        if names(dir)? != ["batch".to_owned(), "seed".to_owned()].into() {
            return Err(format!("{dir:?} must hold exactly seed/ and batch/"));
        }
        if names(&seed_dir)? != want_seed {
            return Err(format!("seed/ must hold exactly {SEED}"));
        }
        if names(&batch_dir)? != want_batch {
            return Err(format!("batch/ must hold exactly {UI_NAMES:?}"));
        }
        let seed = source(&seed_dir, SEED)?;
        let batch = UI_NAMES
            .iter()
            .map(|n| source(&batch_dir, n))
            .collect::<Result<Vec<_>, _>>()?;
        for s in &batch {
            let plan = plan_upload(s.size, &BACKBLAZE_B2.limits, &ClientOptions::default());
            if plan != Ok(UploadPlan::Single) {
                return Err(format!(
                    "{} would not be one PutObject for the release binary: {plan:?}",
                    s.name
                ));
            }
        }
        if batch[0].sha256 == seed.sha256 {
            return Err(format!("batch/{SEED} must differ from the seed"));
        }
        Ok(Self { seed, batch })
    }

    /// What must be stored under each key: the seed for [`SEED`], else the
    /// batch file.
    fn expected(&self) -> Vec<&Source> {
        self.batch
            .iter()
            .map(|s| if s.name == SEED { &self.seed } else { s })
            .collect()
    }
}

fn local_files() -> LocalFiles {
    let dir = PathBuf::from(env(DIR_VAR));
    LocalFiles::read(&dir).unwrap_or_else(|e| fail(format!("local files: {e}; nothing was sent")))
}

/// Downloads every key once and compares it with what must be stored.
/// Panics on the first difference; removes nothing.
async fn verify_stored(
    accounts: Arc<crate::cloud::dashboard::CloudAccounts>,
    account: String,
    bucket: String,
    prefix: String,
    local: LocalFiles,
) {
    let dest = TempDir::new("it-upload-ui-dest");
    for source in local.expected() {
        let key = format!("{prefix}{}", source.name);
        download_and_compare(&accounts, &account, &bucket, &key, &dest, source).await;
    }
}

/// What the cleanup must have found: exactly one version per key, none a
/// delete marker, the sizes of [`LocalFiles::expected`], `existing.txt` as
/// the seed's version, and no incomplete multipart upload.
fn check_found(
    cleanup: &Cleanup,
    prefix: &str,
    local: &LocalFiles,
    seed_version: &str,
) -> Result<(), String> {
    let found = cleanup
        .found_versions
        .as_ref()
        .ok_or("the versions were not listed")?;
    let mut by_key: BTreeMap<&str, Vec<&super::VersionRow>> = BTreeMap::new();
    for v in found {
        by_key.entry(v.key.as_str()).or_default().push(v);
    }
    let want: BTreeSet<String> = UI_NAMES.iter().map(|n| format!("{prefix}{n}")).collect();
    let have: BTreeSet<String> = by_key.keys().map(|k| (*k).to_owned()).collect();
    if have != want {
        return Err(format!("keys {have:?}, expected {want:?}"));
    }
    for source in local.expected() {
        let key = format!("{prefix}{}", source.name);
        let rows = &by_key[key.as_str()];
        if rows.len() != 1 || rows[0].marker {
            return Err(format!("{key:?}: expected one version, found {rows:?}"));
        }
        if rows[0].size != i64::try_from(source.size).ok() {
            return Err(format!(
                "{key:?}: size {:?}, expected {}",
                rows[0].size, source.size
            ));
        }
        if source.name == SEED && rows[0].version_id != seed_version {
            return Err(format!(
                "{key:?}: version {:?}, expected the seed's {seed_version:?} (overwritten?)",
                rows[0].version_id
            ));
        }
    }
    if cleanup.found_uploads != Some(Vec::new()) {
        return Err(format!(
            "incomplete multipart uploads: {:?}",
            cleanup.found_uploads
        ));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: writes one seed object to the Backblaze B2 test bucket and keeps it; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_ui_prepare() {
    require_confirmation();
    let run = env(RUN_VAR);
    let prefix = ui_run_prefix(&run);
    let local = local_files();
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    println!(
        "provider {}, region {}, endpoint {}, run id {run}, run prefix {prefix}",
        resolved.profile.id, resolved.region, resolved.endpoint_url
    );
    for s in std::iter::once(&local.seed).chain(&local.batch) {
        println!("local {:?}: {} bytes, SHA-256 {}", s.name, s.size, s.sha256);
    }
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, &UI_NAMES);
    let store = scoped_store(&resolved, creds, &requests, &scope);

    let before_versions = list_versions(&store, &t.bucket, &prefix).await;
    let before_uploads = list_open_uploads(&store, &t.bucket, &prefix).await;
    match (&before_versions, &before_uploads) {
        (Ok(v), Ok(u)) if v.is_empty() && u.is_empty() => {
            println!("run prefix: no versions, delete markers or multipart uploads")
        }
        _ => {
            show_requests(&t.bucket, &requests.take());
            fail(format!(
                "nothing uploaded: the run prefix is not empty or cannot be listed: versions {} / multipart uploads {}",
                before_versions.map_or_else(|e| e, |v| v.len().to_string()),
                before_uploads.map_or_else(|e| e, |u| u.len().to_string()),
            ));
        }
    }

    let key = format!("{prefix}{SEED}");
    let up = store.upload_file(&t.bucket, &key, &local.seed.path).await;
    let sent = requests.take();
    show_requests(&t.bucket, &sent);
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    let up = up.unwrap_or_else(|e| {
        fail(format!(
            "seed upload failed: {e}; run backblaze_b2_upload_ui_cleanup with {RUN_VAR}={run}"
        ))
    });
    let Some(version) = up.version_id.clone().filter(|v| !v.is_empty()) else {
        fail(format!(
            "the seed has no version id; run backblaze_b2_upload_ui_cleanup with {RUN_VAR}={run}"
        ));
    };
    // The driver reads the version from this line.
    println!("upload-ui seed kept; run id {run} version {version}");
    assert_eq!(
        (up.size, up.sha256.as_str(), up.parts),
        (local.seed.size, local.seed.sha256.as_str(), 1)
    );
    assert_eq!(sent.len(), 5, "requests: {sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: reads and deletes one upload-ui-validation run in the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_ui_verify() {
    require_confirmation();
    let run = env(RUN_VAR);
    let prefix = ui_run_prefix(&run);
    let local = local_files();
    let seed_version = env(SEED_VERSION_VAR);
    assert!(!seed_version.is_empty(), "{SEED_VERSION_VAR} is empty");
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    println!("verifying run {run} under {prefix}");
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, &UI_NAMES);
    let store = scoped_store(&resolved, creds.clone(), &requests, &scope);
    let input = AccountInput {
        label: "Upload UI verification".into(),
        provider: resolved.profile.id.into(),
        region: resolved.region.clone(),
        endpoint: None,
        bucket: t.bucket.clone(),
    };
    let (accounts, id) = accounts_with(&input, &creds, Arc::clone(&store));
    // In their own task, so a failed check cannot skip the cleanup.
    let checks = tokio::spawn(verify_stored(
        accounts,
        id,
        t.bucket.clone(),
        prefix.clone(),
        local.clone(),
    ))
    .await;

    let cleanup = cleanup_run(&store, &t.bucket, &prefix, &UI_NAMES).await;
    let sent = requests.take();
    show_requests(&t.bucket, &sent);
    for line in &cleanup.problems {
        println!("{}", line.replace(&t.bucket, "<bucket>"));
    }
    println!(
        "found: versions {:?}, multipart uploads {:?}",
        cleanup.found_versions, cleanup.found_uploads
    );
    println!(
        "after cleanup: versions {:?}, multipart uploads {:?}",
        cleanup.left_versions, cleanup.left_uploads
    );
    let refused = scope.refused();
    if !refused.is_empty() {
        fail(format!(
            "the run scope refused requests (nothing was sent for them): {refused:?}"
        ));
    }
    // The driver reads the cleanup's outcome from these lines.
    if cleanup.clean() {
        println!("upload-ui cleanup: clean");
    } else {
        println!("upload-ui CLEANUP INCOMPLETE");
    }
    if let Err(e) = checks {
        fail(format!(
            "the download checks failed (cleanup ran first): {e}"
        ));
    }
    if !cleanup.clean() {
        fail(format!(
            "CLEANUP INCOMPLETE under {prefix}; run backblaze_b2_upload_ui_cleanup with {RUN_VAR}={run}"
        ));
    }
    if let Err(e) = check_found(&cleanup, &prefix, &local, &seed_version) {
        fail(format!("what the provider held (removed now): {e}"));
    }
    // By operation, not by path: the bucket's own name may contain
    // "sotyware-it" (it does for the test bucket), so a path fragment would
    // also match the listings.
    assert_eq!(count(&sent, "GET", "x-id=GetObject"), 4, "{sent:?}");
    assert_eq!(count(&sent, "DELETE", "versionId="), 4, "{sent:?}");
    assert_eq!(sent.len(), 12, "requests: {sent:?}");
    println!("upload-ui verify: passed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: removes one upload-ui-validation run from the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_ui_cleanup() {
    require_confirmation();
    let run = env(RUN_VAR);
    let prefix = ui_run_prefix(&run);
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, &UI_NAMES);
    let store = scoped_store(&resolved, creds, &requests, &scope);
    println!("cleaning up run {run} under {prefix}");
    let cleanup = cleanup_run(&store, &t.bucket, &prefix, &UI_NAMES).await;
    show_requests(&t.bucket, &requests.take());
    println!(
        "found: versions {:?}, multipart uploads {:?}",
        cleanup.found_versions, cleanup.found_uploads
    );
    for line in &cleanup.problems {
        println!("{}", line.replace(&t.bucket, "<bucket>"));
    }
    println!(
        "left: versions {:?}, multipart uploads {:?}",
        cleanup.left_versions, cleanup.left_uploads
    );
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    if cleanup.clean() {
        println!("upload-ui cleanup: clean");
    } else {
        println!("upload-ui CLEANUP INCOMPLETE");
    }
    assert!(cleanup.clean(), "cleanup incomplete under {prefix}");
}

/// Writes a valid local layout into `dir` (small files).
fn write_layout(dir: &TempDir) {
    let seed = dir.path("seed");
    let batch = dir.path("batch");
    fs::create_dir_all(&seed).unwrap();
    fs::create_dir_all(&batch).unwrap();
    fs::write(seed.join(SEED), b"seeded before the dashboard upload").unwrap();
    fs::write(batch.join(SEED), b"the dashboard's file, never stored").unwrap();
    fs::write(batch.join(SMALL), b"small").unwrap();
    fs::write(batch.join(UNICODE), vec![7u8; 1000]).unwrap();
    fs::write(batch.join(PROGRESS), vec![9u8; 3 * 1024 * 1024]).unwrap();
}

/// The provider side against the in-process server: the seed through the
/// run scope, the dashboard's batch through the code behind
/// `upload_cloud_files` (begin_upload, as the batch test does), then the
/// verification downloads. The server has neither versioning nor
/// `ListMultipartUploads`, so the listings are not part of it.
#[tokio::test(flavor = "multi_thread")]
async fn upload_ui_run_against_the_test_server() {
    // Ends like the real test bucket's name (this server has no listings,
    // so only object paths are exercised with it).
    const BUCKET: &str = "local-sotyware-it";
    let server =
        crate::cloud::test_server::TestServer::start("test-key", "test-secret", &[BUCKET]).await;
    let resolved = server.resolved("it-local", &[BUCKET]);
    let prefix = ui_run_prefix("local-1");
    let requests = Requests::default();
    let scope = RunScope::for_names(BUCKET, &prefix, &UI_NAMES);
    let store = scoped_store(&resolved, server.credentials(), &requests, &scope);
    let dir = TempDir::new("it-upload-ui-local");
    write_layout(&dir);
    let local = LocalFiles::read(&dir.path("")).expect("a valid layout");

    let up = store
        .upload_file(BUCKET, &format!("{prefix}{SEED}"), &local.seed.path)
        .await
        .expect("the seed");
    assert_eq!(up.sha256, local.seed.sha256);
    assert_eq!(
        requests.take().len(),
        3,
        "HeadObject, PutObject, HeadObject"
    );

    let input = AccountInput {
        label: "Local".into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(server.endpoint.clone()),
        bucket: BUCKET.into(),
    };
    let (accounts, id) = accounts_with(&input, &server.credentials(), Arc::clone(&store));
    let target = crate::cloud::upload::Target {
        account: id.clone(),
        bucket: BUCKET.into(),
        prefix: prefix.clone(),
    };
    let uploads = Arc::new(crate::cloud::upload::CloudUploads::new());
    let paths = local.batch.iter().map(|s| s.path.clone()).collect();
    let summary = super::run_batch(&uploads, &accounts, &target, paths).await;
    assert_eq!(
        (summary.completed, summary.skipped, summary.failed),
        (3, 1, 0),
        "{summary:?}"
    );
    let skipped = &summary.files[0];
    assert_eq!(skipped.name, SEED);
    assert!(!skipped.started, "{skipped:?}");
    let batch_sent = requests.take();
    // Per uploaded file: the batch's HeadObject, the adapter's HeadObject,
    // PutObject, the verifying HeadObject; the existing one: one HeadObject.
    assert_eq!(batch_sent.len(), 3 * 4 + 1, "{batch_sent:?}");
    assert_eq!(count(&batch_sent, "PUT", ""), 3, "{batch_sent:?}");

    verify_stored(accounts, id, BUCKET.into(), prefix.clone(), local).await;
    let sent = requests.take();
    assert_eq!(count(&sent, "GET", "x-id=GetObject"), 4, "{sent:?}");
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
}

#[test]
fn local_files_must_be_exactly_the_planned_layout() {
    let dir = TempDir::new("it-upload-ui-layout");
    write_layout(&dir);
    let local = LocalFiles::read(&dir.path("")).expect("valid");
    let names: Vec<&str> = local.batch.iter().map(|s| s.name).collect();
    assert_eq!(names, UI_NAMES);
    assert_eq!(local.expected()[0].sha256, local.seed.sha256);
    assert_eq!(local.expected()[1].sha256, local.batch[1].sha256);

    // An extra file in batch/.
    fs::write(dir.path("batch").join("extra.txt"), b"x").unwrap();
    assert!(LocalFiles::read(&dir.path("")).is_err());
    fs::remove_file(dir.path("batch").join("extra.txt")).unwrap();
    // A missing one.
    fs::remove_file(dir.path("batch").join(SMALL)).unwrap();
    assert!(LocalFiles::read(&dir.path("")).is_err());
    fs::write(dir.path("batch").join(SMALL), b"small").unwrap();
    // The rival equal to the seed.
    fs::copy(dir.path("seed").join(SEED), dir.path("batch").join(SEED)).unwrap();
    assert!(LocalFiles::read(&dir.path("")).is_err());
    fs::write(dir.path("batch").join(SEED), b"different").unwrap();
    assert!(LocalFiles::read(&dir.path("")).is_ok());
    // A file the release binary would send as a multipart upload.
    let big = fs::File::create(dir.path("batch").join(PROGRESS)).unwrap();
    big.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(big);
    let err = LocalFiles::read(&dir.path("")).unwrap_err();
    assert!(err.contains("PutObject"), "{err}");
}

#[test]
fn ui_run_prefix_stays_under_the_upload_ui_root() {
    assert_eq!(
        ui_run_prefix("20261113120000000"),
        "sotyware-it/upload-ui-validation/20261113120000000/"
    );
    for bad in ["", "..", "a/b", "../upload-validation", "a b", "a%2Fb"] {
        assert!(
            std::panic::catch_unwind(|| ui_run_prefix(bad)).is_err(),
            "{bad:?}"
        );
    }
    assert!(std::panic::catch_unwind(|| ui_run_prefix(&"1".repeat(65))).is_err());
}

#[test]
fn the_ui_scope_allows_only_this_runs_four_keys() {
    let b = "bkt";
    let p = "sotyware-it/upload-ui-validation/1-2/";
    for path in [
        "/bkt/sotyware-it/upload-ui-validation/1-2/existing.txt",
        "/bkt/sotyware-it/upload-ui-validation/1-2/a-small.txt",
        "/bkt/sotyware-it/upload-ui-validation/1-2/gr%C3%B6%C3%9Fe%20%2Btest.bin",
        "/bkt/sotyware-it/upload-ui-validation/1-2/progress.bin",
    ] {
        assert_eq!(in_scope("GET", path, b, p, &UI_NAMES), Ok(()), "{path}");
    }
    assert_eq!(
        in_scope(
            "GET",
            "/bkt?versions&prefix=sotyware-it%2Fupload-ui-validation%2F1-2%2F",
            b,
            p,
            &UI_NAMES
        ),
        Ok(())
    );
    for (m, path) in [
        // The batch test's names, the parent folder, another run.
        ("PUT", "/bkt/sotyware-it/upload-ui-validation/1-2/empty.bin"),
        ("PUT", "/bkt/sotyware-it/upload-ui-validation/existing.txt"),
        (
            "PUT",
            "/bkt/sotyware-it/upload-ui-validation/1-3/existing.txt",
        ),
        (
            "GET",
            "/bkt?versions&prefix=sotyware-it%2Fupload-ui-validation%2F",
        ),
        // Deletes need an id.
        (
            "DELETE",
            "/bkt/sotyware-it/upload-ui-validation/1-2/existing.txt",
        ),
    ] {
        assert!(in_scope(m, path, b, p, &UI_NAMES).is_err(), "{m} {path}");
    }
}

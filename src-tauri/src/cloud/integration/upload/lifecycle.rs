//! Opt-in: the provider side of the upload lifecycle tests through the
//! Windows dashboard (`tests/desktop/windows-cloud-ui.ps1 -Mode b2-lifecycle
//! -Scenario <scenario>`,
//! docs/development/testing.md#upload-lifecycle-through-the-windows-dashboard-at-backblaze).
//! The dashboard itself (release binary, release settings, native file
//! dialog, its own key) uploads, cancels, retries, leaves the browser or
//! quits; these tests only prepare a run, look at it while it runs, check
//! what is left and remove it, with the write key, through the same
//! [`RunScope`] as the batch test.
//!
//! One run is one [`Scenario`], under its own prefix
//! `sotyware-it/upload-lifecycle-validation/<scenario>-<timestamp>/`, with
//! its own fixed file names ([`Scenario::keys`]): nothing else may be
//! addressed, and the cleanup removes only those keys. The driver writes
//! the local files first, into `SOTYWARE_IT_B2_LIFECYCLE_DIR`
//! (`seed/anchor.txt` and `batch/` with exactly [`Scenario::batch`]);
//! [`LocalFiles::read`] checks that each one is the single `PutObject` or
//! the multipart upload the scenario needs **with the release binary's
//! settings** ([`ClientOptions::default`], Backblaze's limits), not the
//! batch test's smaller ones.
//!
//! - [`backblaze_b2_upload_lifecycle_prepare`]: the run prefix must hold no
//!   version, delete marker or incomplete multipart upload; then
//!   `anchor.txt` is uploaded, which makes the run folder visible to the
//!   browser (and is the existing object of the retry scenario). Requests:
//!   `ListObjectVersions`, `ListMultipartUploads`, `HeadObject`,
//!   `PutObject`, `HeadObject` (5).
//! - [`backblaze_b2_upload_lifecycle_probe`]: one `ListMultipartUploads` of
//!   the run prefix while the dashboard uploads; prints how many are open
//!   and for which of the run's names (1 request).
//! - [`backblaze_b2_upload_lifecycle_verify`]: downloads each object the
//!   scenario must have stored ([`Scenario::stored`]) through
//!   [`CloudDownloads`] and compares it with the local file; then the run's
//!   cleanup (aborts incomplete multipart uploads by upload id, deletes
//!   this run's versions by version id, lists again) and checks on what it
//!   found: exactly the anchor (its own version) and the stored keys, one
//!   version each, no delete marker and **no incomplete multipart upload**.
//!   Requests: one `GetObject` per stored key, then 2 listings, one
//!   `DeleteObject` with a version id per version, 2 listings.
//! - [`backblaze_b2_upload_lifecycle_cleanup`]: only the cleanup, for a run
//!   that was interrupted.
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
    Cleanup, RunScope, Source, accounts_with, cleanup_run, count, download_and_compare, in_scope,
    list_open_uploads, list_versions, require_confirmation, scoped_store, show_requests,
};
use crate::cloud::dashboard::AccountInput;
use crate::cloud::provider::BACKBLAZE_B2;
use crate::cloud::s3::{ClientOptions, UploadPlan, plan_upload};
use crate::cloud::test_server::TempDir;

/// Every lifecycle run lives below this, inside the write key's prefix.
const LC_ROOT: &str = "sotyware-it/upload-lifecycle-validation/";
const RUN_VAR: &str = "SOTYWARE_IT_B2_LIFECYCLE_RUN";
const DIR_VAR: &str = "SOTYWARE_IT_B2_LIFECYCLE_DIR";
const SCENARIO_VAR: &str = "SOTYWARE_IT_B2_LIFECYCLE_SCENARIO";
const ANCHOR_VERSION_VAR: &str = "SOTYWARE_IT_B2_LIFECYCLE_ANCHOR_VERSION";

/// Uploaded by the prepare test, so the run folder exists for the browser.
const ANCHOR: &str = "anchor.txt";
const MIB: u64 = 1024 * 1024;

/// One lifecycle behaviour, run on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scenario {
    /// A file the release binary sends as a multipart upload, completed.
    Multipart,
    /// Cancel during a multipart upload.
    Cancel,
    /// A file another process holds open fails; Try again takes only it.
    Retry,
    /// Back to accounts during a single `PutObject`.
    Leave,
    /// Tray Quit during a multipart upload.
    Quit,
}

impl Scenario {
    pub(super) const ALL: [Scenario; 5] = [
        Self::Multipart,
        Self::Cancel,
        Self::Retry,
        Self::Leave,
        Self::Quit,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Multipart => "multipart",
            Self::Cancel => "cancel",
            Self::Retry => "retry",
            Self::Leave => "leave",
            Self::Quit => "quit",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s)
    }

    /// The local `batch/` folder's files, in the order the dashboard
    /// uploads them (by name).
    pub(super) fn batch(self) -> &'static [&'static str] {
        match self {
            Self::Multipart => &["multipart.bin"],
            Self::Cancel => &["cancel.bin"],
            Self::Retry => &[ANCHOR, "a-first.txt", "b-locked.txt", "c-third.txt"],
            Self::Leave => &["a-away.bin", "b-away.txt"],
            Self::Quit => &["quit.bin"],
        }
    }

    /// Every key below the run prefix this run may address: the anchor and
    /// the batch's names.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Multipart => &[ANCHOR, "multipart.bin"],
            Self::Cancel => &[ANCHOR, "cancel.bin"],
            Self::Retry => &[ANCHOR, "a-first.txt", "b-locked.txt", "c-third.txt"],
            Self::Leave => &[ANCHOR, "a-away.bin", "b-away.txt"],
            Self::Quit => &[ANCHOR, "quit.bin"],
        }
    }

    /// The batch files the dashboard must have stored when the scenario
    /// ends (the anchor is always there and never replaced).
    fn stored(self) -> &'static [&'static str] {
        match self {
            Self::Multipart => &["multipart.bin"],
            Self::Retry => &["a-first.txt", "b-locked.txt", "c-third.txt"],
            Self::Cancel | Self::Leave | Self::Quit => &[],
        }
    }

    /// What the release binary must send for batch file `name` of `size`
    /// bytes: one `PutObject`, or a multipart upload of this many parts.
    fn plan_ok(self, name: &str, plan: &UploadPlan) -> bool {
        let parts = match (self, name) {
            (Self::Multipart, _) => 5,
            (Self::Cancel | Self::Quit, _) => 9,
            _ => 1,
        };
        match plan {
            UploadPlan::Single => parts == 1,
            UploadPlan::Multipart {
                part_size,
                parts: n,
            } => parts > 1 && *part_size == 16 * MIB && *n == parts,
        }
    }
}

/// The run prefix for `run` of `scenario`, which may not leave [`LC_ROOT`].
pub(super) fn lc_run_prefix(scenario: Scenario, run: &str) -> String {
    let stamp = run
        .strip_prefix(scenario.name())
        .and_then(|r| r.strip_prefix('-'));
    assert!(
        stamp.is_some_and(|s| {
            !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_digit())
        }),
        "a lifecycle run id is <scenario>-<digits>"
    );
    format!("{LC_ROOT}{run}/")
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail(format!("set {name}; nothing was sent")))
}

fn scenario_from_env() -> Scenario {
    let s = env(SCENARIO_VAR);
    Scenario::parse(&s).unwrap_or_else(|| fail(format!("unknown scenario {s:?}; nothing was sent")))
}

/// The local files of a run, as the driver wrote them.
#[derive(Clone, Debug)]
struct LocalFiles {
    anchor: Source,
    /// In [`Scenario::batch`] order.
    batch: Vec<Source>,
}

impl LocalFiles {
    /// Checks `dir` without reading any file: `seed/` holds exactly the
    /// anchor, `batch/` exactly the scenario's files, all regular files,
    /// and each batch file is sent as the scenario needs it by the release
    /// binary at Backblaze. Nothing is sent.
    fn check_layout(scenario: Scenario, dir: &Path) -> Result<(), String> {
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
        if names(dir)? != ["batch".to_owned(), "seed".to_owned()].into() {
            return Err(format!("{dir:?} must hold exactly seed/ and batch/"));
        }
        if names(&dir.join("seed"))? != [ANCHOR.to_owned()].into() {
            return Err(format!("seed/ must hold exactly {ANCHOR}"));
        }
        let want: BTreeSet<String> = scenario.batch().iter().map(|n| (*n).to_owned()).collect();
        if names(&dir.join("batch"))? != want {
            return Err(format!("batch/ must hold exactly {:?}", scenario.batch()));
        }
        let anchor = dir.join("seed").join(ANCHOR);
        if !fs::metadata(&anchor).is_ok_and(|m| m.is_file()) {
            return Err(format!("{anchor:?} is not a regular file"));
        }
        for name in scenario.batch() {
            let path = dir.join("batch").join(name);
            // `fs::metadata`, not the directory entry's: on Windows that
            // reports an open file's size as of its last close.
            let meta = fs::metadata(&path).map_err(|e| format!("{path:?}: {e}"))?;
            if !meta.is_file() {
                return Err(format!("{path:?} is not a regular file"));
            }
            let plan = plan_upload(meta.len(), &BACKBLAZE_B2.limits, &ClientOptions::default())
                .map_err(|e| format!("{name}: {e}"))?;
            if !scenario.plan_ok(name, &plan) {
                return Err(format!(
                    "{name} ({} bytes) is not what the {} scenario needs from the release binary: {plan:?}",
                    meta.len(),
                    scenario.name()
                ));
            }
        }
        Ok(())
    }

    /// [`Self::check_layout`], then every file's size and SHA-256; a batch
    /// `anchor.txt` must differ from the seed. Nothing is sent.
    fn read(scenario: Scenario, dir: &Path) -> Result<Self, String> {
        Self::check_layout(scenario, dir)?;
        let source = |dir: &Path, name: &'static str| {
            let path = dir.join(name);
            Source {
                name,
                sha256: sha256_of(&path),
                size: fs::metadata(&path).map_or(0, |m| m.len()),
                path,
            }
        };
        let anchor = source(&dir.join("seed"), ANCHOR);
        let batch: Vec<Source> = scenario
            .batch()
            .iter()
            .map(|n| source(&dir.join("batch"), n))
            .collect();
        if let Some(rival) = batch.iter().find(|s| s.name == ANCHOR)
            && rival.sha256 == anchor.sha256
        {
            return Err(format!("batch/{ANCHOR} must differ from the seed"));
        }
        Ok(Self { anchor, batch })
    }

    /// What must be stored when the scenario ends: the anchor, then the
    /// scenario's stored files.
    fn expected(&self, scenario: Scenario) -> Vec<&Source> {
        std::iter::once(&self.anchor)
            .chain(
                self.batch
                    .iter()
                    .filter(|s| scenario.stored().contains(&s.name)),
            )
            .collect()
    }
}

fn local_files(scenario: Scenario) -> LocalFiles {
    let dir = PathBuf::from(env(DIR_VAR));
    LocalFiles::read(scenario, &dir)
        .unwrap_or_else(|e| fail(format!("local files: {e}; nothing was sent")))
}

/// What the cleanup must have found: exactly the anchor (its own version)
/// and the scenario's stored keys, one version each, none a delete marker,
/// the local sizes, and no incomplete multipart upload.
fn check_found(
    cleanup: &Cleanup,
    scenario: Scenario,
    prefix: &str,
    local: &LocalFiles,
    anchor_version: &str,
) -> Result<(), String> {
    if cleanup.found_uploads != Some(Vec::new()) {
        return Err(format!(
            "incomplete multipart uploads were left (aborted by the cleanup now): {:?}",
            cleanup.found_uploads
        ));
    }
    let found = cleanup
        .found_versions
        .as_ref()
        .ok_or("the versions were not listed")?;
    let mut by_key: BTreeMap<&str, Vec<&super::VersionRow>> = BTreeMap::new();
    for v in found {
        by_key.entry(v.key.as_str()).or_default().push(v);
    }
    let expected = local.expected(scenario);
    let want: BTreeSet<String> = expected
        .iter()
        .map(|s| format!("{prefix}{}", s.name))
        .collect();
    let have: BTreeSet<String> = by_key.keys().map(|k| (*k).to_owned()).collect();
    if have != want {
        return Err(format!("keys {have:?}, expected {want:?}"));
    }
    for source in expected {
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
        if source.name == ANCHOR && rows[0].version_id != anchor_version {
            return Err(format!(
                "{key:?}: version {:?}, expected the anchor's {anchor_version:?} (overwritten?)",
                rows[0].version_id
            ));
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: writes one anchor object to the Backblaze B2 test bucket and keeps it; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_lifecycle_prepare() {
    require_confirmation();
    let scenario = scenario_from_env();
    let run = env(RUN_VAR);
    let prefix = lc_run_prefix(scenario, &run);
    let local = local_files(scenario);
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    println!(
        "provider {}, region {}, endpoint {}, scenario {}, run id {run}, run prefix {prefix}",
        resolved.profile.id,
        resolved.region,
        resolved.endpoint_url,
        scenario.name()
    );
    for s in std::iter::once(&local.anchor).chain(&local.batch) {
        let plan = plan_upload(s.size, &BACKBLAZE_B2.limits, &ClientOptions::default());
        println!(
            "local {:?}: {} bytes, SHA-256 {}, release plan {plan:?}",
            s.name, s.size, s.sha256
        );
    }
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, scenario.keys());
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

    let key = format!("{prefix}{ANCHOR}");
    let up = store.upload_file(&t.bucket, &key, &local.anchor.path).await;
    let sent = requests.take();
    show_requests(&t.bucket, &sent);
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    let up = up.unwrap_or_else(|e| {
        fail(format!(
            "anchor upload failed: {e}; run backblaze_b2_upload_lifecycle_cleanup with {RUN_VAR}={run}"
        ))
    });
    let Some(version) = up.version_id.clone().filter(|v| !v.is_empty()) else {
        fail(format!(
            "the anchor has no version id; run backblaze_b2_upload_lifecycle_cleanup with {RUN_VAR}={run}"
        ));
    };
    // The driver reads the version from this line.
    println!("upload-lifecycle anchor kept; run id {run} version {version}");
    assert_eq!(
        (up.size, up.sha256.as_str(), up.parts),
        (local.anchor.size, local.anchor.sha256.as_str(), 1)
    );
    assert_eq!(sent.len(), 5, "requests: {sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: lists the incomplete multipart uploads of one lifecycle run in the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_lifecycle_probe() {
    require_confirmation();
    let scenario = scenario_from_env();
    let run = env(RUN_VAR);
    let prefix = lc_run_prefix(scenario, &run);
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, scenario.keys());
    let store = scoped_store(&resolved, creds, &requests, &scope);
    let open = list_open_uploads(&store, &t.bucket, &prefix).await;
    let sent = requests.take();
    show_requests(&t.bucket, &sent);
    assert!(scope.refused().is_empty(), "{:?}", scope.refused());
    let open = open.unwrap_or_else(|e| fail(format!("probe: {e}")));
    let names: Vec<&str> = open
        .iter()
        .map(|u| u.key.strip_prefix(&prefix).unwrap_or("<outside the run>"))
        .collect();
    // The driver reads this line.
    println!(
        "upload-lifecycle probe: {} open multipart upload(s): {names:?}",
        open.len()
    );
    assert_eq!(sent.len(), 1, "requests: {sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: reads and deletes one upload-lifecycle-validation run in the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_lifecycle_verify() {
    require_confirmation();
    let scenario = scenario_from_env();
    let run = env(RUN_VAR);
    let prefix = lc_run_prefix(scenario, &run);
    let local = local_files(scenario);
    let anchor_version = env(ANCHOR_VERSION_VAR);
    assert!(!anchor_version.is_empty(), "{ANCHOR_VERSION_VAR} is empty");
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    println!("verifying {} run {run} under {prefix}", scenario.name());
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, scenario.keys());
    let store = scoped_store(&resolved, creds.clone(), &requests, &scope);
    let input = AccountInput {
        label: "Upload lifecycle verification".into(),
        provider: resolved.profile.id.into(),
        region: resolved.region.clone(),
        endpoint: None,
        bucket: t.bucket.clone(),
    };
    let (accounts, id) = accounts_with(&input, &creds, Arc::clone(&store));
    // Only the dashboard's uploads are downloaded (the anchor is the
    // prepare test's own). In their own task, so a failed check cannot skip
    // the cleanup.
    let stored: Vec<Source> = local
        .batch
        .iter()
        .filter(|s| scenario.stored().contains(&s.name))
        .cloned()
        .collect();
    let downloads = stored.len();
    let (bucket, pfx) = (t.bucket.clone(), prefix.clone());
    let checks = tokio::spawn(async move {
        let dest = TempDir::new("it-upload-lifecycle-dest");
        for source in &stored {
            let key = format!("{pfx}{}", source.name);
            download_and_compare(&accounts, &id, &bucket, &key, &dest, source).await;
        }
    })
    .await;

    let cleanup = cleanup_run(&store, &t.bucket, &prefix, scenario.keys()).await;
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
        println!("upload-lifecycle cleanup: clean");
    } else {
        println!("upload-lifecycle CLEANUP INCOMPLETE");
    }
    if let Err(e) = checks {
        fail(format!(
            "the download checks failed (cleanup ran first): {e}"
        ));
    }
    if !cleanup.clean() {
        fail(format!(
            "CLEANUP INCOMPLETE under {prefix}; run backblaze_b2_upload_lifecycle_cleanup with {RUN_VAR}={run}"
        ));
    }
    if let Err(e) = check_found(&cleanup, scenario, &prefix, &local, &anchor_version) {
        fail(format!("what the provider held (removed now): {e}"));
    }
    let versions = 1 + scenario.stored().len();
    assert_eq!(count(&sent, "GET", "x-id=GetObject"), downloads, "{sent:?}");
    assert_eq!(count(&sent, "DELETE", "versionId="), versions, "{sent:?}");
    assert_eq!(sent.len(), downloads + versions + 4, "requests: {sent:?}");
    println!("upload-lifecycle verify: passed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: removes one upload-lifecycle-validation run from the Backblaze B2 test bucket; also needs SOTYWARE_IT_B2_UPLOAD_CONFIRM"]
async fn backblaze_b2_upload_lifecycle_cleanup() {
    require_confirmation();
    let scenario = scenario_from_env();
    let run = env(RUN_VAR);
    let prefix = lc_run_prefix(scenario, &run);
    let (t, resolved, creds, _) = b2_store();
    assert_backblaze(&resolved);
    let requests = Requests::default();
    let scope = RunScope::for_names(&t.bucket, &prefix, scenario.keys());
    let store = scoped_store(&resolved, creds, &requests, &scope);
    println!("cleaning up run {run} under {prefix}");
    let cleanup = cleanup_run(&store, &t.bucket, &prefix, scenario.keys()).await;
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
        println!("upload-lifecycle cleanup: clean");
    } else {
        println!("upload-lifecycle CLEANUP INCOMPLETE");
    }
    assert!(cleanup.clean(), "cleanup incomplete under {prefix}");
}

/// Writes a valid local layout for `scenario` into `dir`; the large files
/// are sparse (only their size matters to the plan check).
fn write_layout(scenario: Scenario, dir: &TempDir) {
    let seed = dir.path("seed");
    let batch = dir.path("batch");
    fs::create_dir_all(&seed).unwrap();
    fs::create_dir_all(&batch).unwrap();
    fs::write(seed.join(ANCHOR), b"anchor").unwrap();
    for name in scenario.batch() {
        let size = match (scenario, *name) {
            (Scenario::Multipart, _) => 72 * MIB + 3,
            (Scenario::Cancel | Scenario::Quit, _) => 144 * MIB,
            (Scenario::Leave, "a-away.bin") => 60 * MIB,
            _ => 0,
        };
        let path = batch.join(name);
        if size == 0 {
            fs::write(&path, format!("{name} of the {} run", scenario.name())).unwrap();
        } else {
            fs::File::create(&path).unwrap().set_len(size).unwrap();
        }
    }
}

#[test]
fn every_scenario_layout_matches_the_release_settings() {
    for scenario in Scenario::ALL {
        let dir = TempDir::new(&format!("it-lifecycle-layout-{}", scenario.name()));
        write_layout(scenario, &dir);
        // Sizes and plans only: hashing the large sparse files would make
        // this slow in a debug build.
        LocalFiles::check_layout(scenario, &dir.path(""))
            .unwrap_or_else(|e| panic!("{}: {e}", scenario.name()));
        // Every stored name and every batch name is a key the scope allows.
        for n in scenario.batch().iter().chain(scenario.stored()) {
            assert!(scenario.keys().contains(n), "{n}");
        }
    }
    // The small one completely.
    let dir = TempDir::new("it-lifecycle-layout-read");
    write_layout(Scenario::Retry, &dir);
    let local = LocalFiles::read(Scenario::Retry, &dir.path("")).unwrap();
    let names: Vec<&str> = local.batch.iter().map(|s| s.name).collect();
    assert_eq!(names, Scenario::Retry.batch());
    let expected: Vec<&str> = local
        .expected(Scenario::Retry)
        .iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(
        expected,
        [ANCHOR, "a-first.txt", "b-locked.txt", "c-third.txt"]
    );
}

#[test]
fn a_layout_the_release_binary_would_send_otherwise_is_refused() {
    // The batch test's multipart size: one PutObject with the release
    // settings, so the multipart scenario would not exercise multipart.
    let dir = TempDir::new("it-lifecycle-layout-small");
    write_layout(Scenario::Multipart, &dir);
    fs::File::create(dir.path("batch").join("multipart.bin"))
        .unwrap()
        .set_len(10 * MIB + 3)
        .unwrap();
    let err = LocalFiles::check_layout(Scenario::Multipart, &dir.path("")).unwrap_err();
    assert!(err.contains("Single"), "{err}");

    // A leave file above the threshold would be multipart, not one PUT.
    let dir = TempDir::new("it-lifecycle-layout-leave");
    write_layout(Scenario::Leave, &dir);
    fs::File::create(dir.path("batch").join("a-away.bin"))
        .unwrap()
        .set_len(64 * MIB + 1)
        .unwrap();
    assert!(LocalFiles::check_layout(Scenario::Leave, &dir.path("")).is_err());

    // An extra file, a missing one, a retry anchor equal to the seed.
    let dir = TempDir::new("it-lifecycle-layout-retry");
    write_layout(Scenario::Retry, &dir);
    fs::write(dir.path("batch").join("extra.txt"), b"x").unwrap();
    assert!(LocalFiles::read(Scenario::Retry, &dir.path("")).is_err());
    fs::remove_file(dir.path("batch").join("extra.txt")).unwrap();
    fs::copy(
        dir.path("seed").join(ANCHOR),
        dir.path("batch").join(ANCHOR),
    )
    .unwrap();
    let err = LocalFiles::read(Scenario::Retry, &dir.path("")).unwrap_err();
    assert!(err.contains("differ"), "{err}");
    fs::remove_file(dir.path("batch").join(ANCHOR)).unwrap();
    assert!(LocalFiles::read(Scenario::Retry, &dir.path("")).is_err());
}

#[test]
fn lifecycle_run_prefix_names_the_scenario_and_stays_under_its_root() {
    assert_eq!(
        lc_run_prefix(Scenario::Cancel, "cancel-20261113120000000"),
        "sotyware-it/upload-lifecycle-validation/cancel-20261113120000000/"
    );
    for (s, bad) in [
        (Scenario::Cancel, "quit-20261113120000000"),
        (Scenario::Cancel, "cancel-"),
        (Scenario::Cancel, "cancel"),
        (Scenario::Cancel, "cancel-1/../x"),
        (Scenario::Cancel, "cancel-1a"),
        (Scenario::Quit, "../quit-1"),
        (Scenario::Quit, ""),
    ] {
        assert!(
            std::panic::catch_unwind(|| lc_run_prefix(s, bad)).is_err(),
            "{bad:?}"
        );
    }
    assert!(
        std::panic::catch_unwind(|| lc_run_prefix(
            Scenario::Quit,
            &format!("quit-{}", "1".repeat(33))
        ))
        .is_err()
    );
    for s in Scenario::ALL {
        assert_eq!(Scenario::parse(s.name()), Some(s));
    }
    assert_eq!(Scenario::parse("upload"), None);
}

#[test]
fn the_lifecycle_scope_allows_only_the_scenarios_keys() {
    let b = "bkt";
    let p = "sotyware-it/upload-lifecycle-validation/retry-1/";
    for name in Scenario::Retry.keys() {
        let path = format!("/bkt/{p}{name}");
        assert_eq!(in_scope("PUT", &path, b, p, Scenario::Retry.keys()), Ok(()));
    }
    assert_eq!(
        in_scope(
            "GET",
            "/bkt?uploads&prefix=sotyware-it%2Fupload-lifecycle-validation%2Fretry-1%2F",
            b,
            p,
            Scenario::Retry.keys()
        ),
        Ok(())
    );
    for (m, path) in [
        // Another scenario's file, another run, the parent folder.
        (
            "PUT",
            "/bkt/sotyware-it/upload-lifecycle-validation/retry-1/quit.bin",
        ),
        (
            "PUT",
            "/bkt/sotyware-it/upload-lifecycle-validation/retry-2/a-first.txt",
        ),
        (
            "PUT",
            "/bkt/sotyware-it/upload-lifecycle-validation/anchor.txt",
        ),
        (
            "GET",
            "/bkt?versions&prefix=sotyware-it%2Fupload-lifecycle-validation%2F",
        ),
        // Deletes need an id.
        (
            "DELETE",
            "/bkt/sotyware-it/upload-lifecycle-validation/retry-1/anchor.txt",
        ),
    ] {
        assert!(
            in_scope(m, path, b, p, Scenario::Retry.keys()).is_err(),
            "{m} {path}"
        );
    }
}

#[test]
fn check_found_requires_exactly_the_anchor_and_the_stored_keys_and_no_open_upload() {
    use super::{OpenUpload, VersionRow};
    let fake = |name: &'static str, size: u64| Source {
        name,
        path: PathBuf::from(name),
        size,
        sha256: String::new(),
    };
    let local = LocalFiles {
        anchor: fake(ANCHOR, 6),
        batch: vec![fake("cancel.bin", 144 * MIB)],
    };
    let p = "sotyware-it/upload-lifecycle-validation/cancel-1/";
    let anchor = VersionRow {
        key: format!("{p}{ANCHOR}"),
        version_id: "v-anchor".into(),
        size: Some(local.anchor.size as i64),
        marker: false,
    };
    let cleanup = |versions: Vec<VersionRow>, uploads: Vec<OpenUpload>| Cleanup {
        found_versions: Some(versions),
        found_uploads: Some(uploads),
        left_versions: Ok(Vec::new()),
        left_uploads: Ok(Vec::new()),
        problems: Vec::new(),
    };
    let ok = cleanup(vec![anchor.clone()], Vec::new());
    assert_eq!(
        check_found(&ok, Scenario::Cancel, p, &local, "v-anchor"),
        Ok(())
    );
    // The anchor replaced.
    assert!(check_found(&ok, Scenario::Cancel, p, &local, "v-other").is_err());
    // A cancelled file that was stored after all.
    let stored = VersionRow {
        key: format!("{p}cancel.bin"),
        version_id: "v2".into(),
        size: Some(144 * MIB as i64),
        marker: false,
    };
    let bad = cleanup(vec![anchor.clone(), stored], Vec::new());
    assert!(check_found(&bad, Scenario::Cancel, p, &local, "v-anchor").is_err());
    // Parts left behind.
    let parts = cleanup(
        vec![anchor],
        vec![OpenUpload {
            key: format!("{p}cancel.bin"),
            upload_id: "u1".into(),
        }],
    );
    let err = check_found(&parts, Scenario::Cancel, p, &local, "v-anchor").unwrap_err();
    assert!(err.contains("incomplete multipart"), "{err}");
}

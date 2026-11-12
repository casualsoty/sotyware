//! Opt-in: the bucket browser against a temporary dataset in the real
//! Backblaze B2 test bucket (docs/development/testing.md#browsing-a-temporary-dataset-at-backblaze).
//!
//! `backblaze_b2_browse_dataset` creates a small, deterministic dataset under
//! `sotyware-it/browse-validation/<run id>/`, lists it through the same code
//! the dashboard uses (`browser::list_folder` with `CloudAccounts`), compares
//! every folder with what was uploaded and with the in-process listing server
//! holding the same keys, and then deletes exactly the versions it created.
//!
//! Safety rules, enforced here:
//! - Every write is a `PutObject` of a dataset key under the run prefix;
//!   nothing else is written, and the run prefix must have no object, version
//!   or delete marker before the first write.
//! - Cleanup lists every version and delete marker under the run prefix,
//!   deletes by version id only those whose key is a dataset key of this
//!   run, and lists again. Anything else is reported, never deleted.
//! - Cleanup runs even when a check fails (the checks run in their own task,
//!   so a panic cannot skip it). `backblaze_b2_browse_cleanup` repeats it for
//!   a run id from `SOTYWARE_IT_B2_BROWSE_RUN`, after an interrupted run.
//! - With `SOTYWARE_IT_B2_BROWSE_KEEP=1` the dataset is left in place for the
//!   UI tests, and the run id is printed for the cleanup test.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::primitives::{ByteStream, DateTime, DateTimeFormat};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::super::account::{MemoryAccountStore, ResolvedAccount};
use super::super::browser::{self, FolderListing};
use super::super::credentials::{CredentialStore, Credentials, MemoryCredentialStore};
use super::super::dashboard::{
    AccountInput, CloudAccounts, CommandError, CredentialInput, ErrorKind, LIST_PAGE_SIZE,
};
use super::super::s3::{ClientOptions, S3Store, SHA256_METADATA};
use super::super::test_server::TestServer;
use super::{B2_KEY_PREFIX, Requests, VersionEntry, b2_store, fingerprint};

/// Where every run's dataset lives, below the test key's own prefix.
const BROWSE_ROOT: &str = "sotyware-it/browse-validation/";
/// Immediate files of the paginated folder; with its subfolders more than
/// one page of `LIST_PAGE_SIZE`.
const MANY_FILES: usize = 1005;
const MANY_FOLDERS: usize = 5;
const UPLOADS_AT_ONCE: usize = 8;

/// One object of the dataset.
#[derive(Clone, Debug)]
struct Item {
    /// The key below the run prefix.
    rel: String,
    body: Vec<u8>,
    /// False for keys a provider may refuse (repeated separators, `..`):
    /// a refusal is recorded, not a failure.
    required: bool,
}

/// The dataset: the same keys for every run, contents derived from the key.
fn dataset() -> Vec<Item> {
    let file = |rel: &str, copies: usize| Item {
        rel: rel.to_owned(),
        body: format!("sotyware browse validation: {rel}\n")
            .repeat(copies)
            .into_bytes(),
        required: true,
    };
    let marker = |rel: &str| Item {
        rel: rel.to_owned(),
        body: Vec::new(),
        required: true,
    };
    let optional = |mut item: Item| {
        item.required = false;
        item
    };
    let mut items = vec![
        // Files at the run's root, one with a space, Unicode in NFC and NFD.
        file("root-a.txt", 1),
        file("root b.txt", 2),
        file("caf\u{e9} nfc.txt", 1),
        file("cafe\u{301} nfd.txt", 1),
        file("\u{1f389} emoji.txt", 3),
        // Nested folders at several depths.
        file("docs/readme.txt", 1),
        file("docs/2024/report.txt", 2),
        file("docs/2024/q1/deep/leaf.txt", 1),
        file("docs/2024/q1/deep/er/leaf 2.txt", 4),
        // A folder with a marker and content; the marker is not a file.
        marker("photos/"),
        file("photos/cat.jpg", 5),
        file("photos/2023/dog.jpg", 2),
        // Folders that exist only as markers: empty when opened.
        marker("marker-only/"),
        marker("docs/empty-nested/"),
        // Unicode folder and file names.
        file(
            "\u{fc}n\u{ef}c\u{f8}d\u{e9}/\u{65e5}\u{672c}\u{8a9e} \u{30d5}\u{30a1}\u{30a4}\u{30eb}.txt",
            1,
        ),
        file(
            "\u{fc}n\u{ef}c\u{f8}d\u{e9}/\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}/\u{645}\u{631}\u{62d}\u{628}\u{627}.txt",
            2,
        ),
        // Characters with a meaning in URLs, in file and folder names.
        file("special/plus+sign.txt", 1),
        file("special/amp&ersand.txt", 1),
        file("special/question?.txt", 1),
        file("special/hash#tag.txt", 1),
        file("special/percent%.txt", 1),
        file("special/percent%20encoded.txt", 1),
        file("special/all + & ? # %.txt", 2),
        file("special/f+o&l?d#e%r/inner.txt", 1),
        // Repeated separators: an empty folder name.
        optional(file("double//inner.txt", 1)),
        optional(file("double///triple.txt", 1)),
        // `..` and `.` as ordinary names, never navigation.
        optional(file("dots/../escape.txt", 1)),
        optional(file("dots/..", 1)),
        optional(file("dots/./here.txt", 1)),
        file("dots/..name.txt", 1),
    ];
    for i in 0..MANY_FILES {
        items.push(file(&format!("many/file-{i:04}.txt"), 1));
    }
    for i in 0..MANY_FOLDERS {
        items.push(file(&format!("many/sub-{i:03}/inside.txt"), 1));
    }
    items
}

fn sha256_hex(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A run id that sorts by time and cannot repeat on one machine.
fn new_run_id() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}-{}", now.as_nanos(), std::process::id())
}

fn run_prefix(run: &str) -> String {
    assert!(
        !run.is_empty()
            && run
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "a run id is letters, digits, '-' and '_' only"
    );
    format!("{BROWSE_ROOT}{run}/")
}

/// Every key a run may create: the dataset and the two permission probes.
fn run_keys(prefix: &str) -> HashSet<String> {
    let mut keys: HashSet<String> = dataset()
        .iter()
        .map(|i| format!("{prefix}{}", i.rel))
        .collect();
    keys.insert(format!("{prefix}{PROBE}"));
    keys.insert(format!("{prefix}{READ_PROBE}"));
    keys
}

/// The write-permission probe of the test key, deleted again at once.
const PROBE: &str = "probe.txt";
/// What the read-only UI key must be refused to write.
const READ_PROBE: &str = "read-key-probe.txt";

/// The read-only key for the UI tests, if set: account id `it-b2-read`
/// (`SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID`, `..._SECRET_ACCESS_KEY`).
fn read_key() -> Option<Credentials> {
    super::super::credentials::EnvCredentials::from_process()
        .credentials("it-b2-read")
        .ok()
}

/// The manifest the UI drivers compare the page with: per folder (relative
/// to the run prefix), its subfolder names and its files with size and date,
/// as the provider listed them through the dashboard code.
fn manifest(run: &str, prefix: &str, listed: &BTreeMap<String, Opened>) -> serde_json::Value {
    // An array, not a map keyed by path: Windows PowerShell 5.1 cannot read
    // an empty property name (the run's root).
    let folders: Vec<serde_json::Value> = listed
        .iter()
        .map(|(folder, o)| {
            serde_json::json!({
                "path": &folder[prefix.len()..],
                "folders": o.folders.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                "files": o.files.iter().map(|(n, _, size, date)| serde_json::json!({
                    "name": n, "size": size, "lastModified": date,
                })).collect::<Vec<_>>(),
                "pages": o.pages.len(),
            })
        })
        .collect();
    serde_json::json!({ "run": run, "prefix": prefix, "folders": folders })
}

/// One created object: full key, size, SHA-256, the version id B2 returned.
#[derive(Clone, Debug)]
struct Created {
    key: String,
    size: u64,
    sha256: String,
    version: Option<String>,
}

/// Every version and delete marker under `prefix`; an error instead of a
/// panic, so cleanup can report it.
async fn try_versions(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<VersionEntry>, String> {
    let mut all = Vec::new();
    let (mut key_marker, mut version_marker) = (None::<String>, None::<String>);
    loop {
        let out = store
            .sdk_client()
            .list_object_versions()
            .bucket(bucket)
            .prefix(prefix)
            .set_key_marker(key_marker.take())
            .set_version_id_marker(version_marker.take())
            .send()
            .await
            .map_err(|e| format!("list object versions: {}", DisplayErrorContext(&e)))?;
        for v in out.versions() {
            all.push((
                v.key().unwrap_or_default().to_owned(),
                v.version_id().unwrap_or_default().to_owned(),
                false,
            ));
        }
        for m in out.delete_markers() {
            all.push((
                m.key().unwrap_or_default().to_owned(),
                m.version_id().unwrap_or_default().to_owned(),
                true,
            ));
        }
        if out.is_truncated() != Some(true) {
            return Ok(all);
        }
        key_marker = out.next_key_marker().map(str::to_owned);
        version_marker = out.next_version_id_marker().map(str::to_owned);
    }
}

/// Uploads the dataset under `prefix`. Returns what was created and what the
/// provider refused (key, error); every success is pushed to `created` as
/// soon as it is known, so cleanup finds it even if a later step fails.
async fn upload(
    store: &Arc<S3Store>,
    bucket: &str,
    prefix: &str,
    items: &[Item],
    created: &Arc<Mutex<Vec<Created>>>,
) -> Vec<(String, String)> {
    let slots = Arc::new(Semaphore::new(UPLOADS_AT_ONCE));
    let mut tasks = JoinSet::new();
    for item in items.iter().cloned() {
        let (store, slots, created) = (Arc::clone(store), Arc::clone(&slots), Arc::clone(created));
        let (bucket, key) = (bucket.to_owned(), format!("{prefix}{}", item.rel));
        tasks.spawn(async move {
            let _slot = slots.acquire_owned().await.expect("never closed");
            let sha256 = sha256_hex(&item.body);
            let size = item.body.len() as u64;
            let result = store
                .sdk_client()
                .put_object()
                .bucket(&bucket)
                .key(&key)
                .metadata(SHA256_METADATA, &sha256)
                .body(ByteStream::from(item.body))
                .send()
                .await;
            match result {
                Ok(out) => {
                    created.lock().unwrap().push(Created {
                        key,
                        size,
                        sha256,
                        version: out.version_id().map(str::to_owned),
                    });
                    None
                }
                Err(e) => Some((key, DisplayErrorContext(&e).to_string(), item.required)),
            }
        });
    }
    let mut refused = Vec::new();
    while let Some(done) = tasks.join_next().await {
        if let Some((key, error, required)) = done.expect("upload task") {
            println!(
                "upload refused{}: {key:?}: {}",
                if required { " (REQUIRED)" } else { "" },
                error.lines().next().unwrap_or_default()
            );
            refused.push((key, error));
        }
    }
    refused
}

/// Deletes, by version id, every version and delete marker under `prefix`
/// whose key is one of `keys`; reports anything else without touching it.
/// Lists again afterwards. `Ok` only if nothing at all is left.
async fn cleanup_run(
    store: &S3Store,
    bucket: &str,
    prefix: &str,
    keys: &HashSet<String>,
) -> Result<usize, String> {
    let found = try_versions(store, bucket, prefix).await?;
    let (ours, unknown): (Vec<_>, Vec<_>) = found
        .into_iter()
        .partition(|(key, version, _)| keys.contains(key) && !version.is_empty());
    println!(
        "cleanup: {} versions/delete markers under the run prefix belong to this run, {} do not",
        ours.len(),
        unknown.len()
    );
    for (key, version, marker) in &unknown {
        println!(
            "cleanup: NOT DELETED (not created by this run): {key:?} version {version:?} delete marker {marker}"
        );
    }
    let mut failed = Vec::new();
    for (key, version, marker) in &ours {
        if let Err(e) = store.delete_object(bucket, key, Some(version)).await {
            println!(
                "CLEANUP FAILED for {key:?} version {version:?} (delete marker {marker}): {e}"
            );
            failed.push(key.clone());
        }
    }
    let left = try_versions(store, bucket, prefix).await?;
    for (key, version, marker) in &left {
        println!("cleanup: LEFT {key:?} version {version:?} delete marker {marker}");
    }
    let markers = ours.iter().filter(|(_, _, m)| *m).count();
    println!(
        "cleanup: {} object versions and {markers} delete markers to delete by version id, {} deletes failed; listing again: {} left",
        ours.len() - markers,
        failed.len(),
        left.len()
    );
    if left.is_empty() && failed.is_empty() && unknown.is_empty() {
        Ok(ours.len())
    } else {
        Err(format!(
            "{} left under the run prefix, {} deletes failed, {} not ours",
            left.len(),
            failed.len(),
            unknown.len()
        ))
    }
}

/// What a folder must show, from the uploaded keys alone: immediate
/// subfolders `(name, prefix)` and files `(name, key, size)`; the folder's
/// own marker is neither.
type Expected = (BTreeSet<(String, String)>, BTreeSet<(String, String, u64)>);

fn expected(prefix: &str, objects: &BTreeMap<String, u64>) -> Expected {
    let (mut folders, mut files) = (BTreeSet::new(), BTreeSet::new());
    for (key, size) in objects.range(prefix.to_owned()..) {
        let Some(rest) = key.strip_prefix(prefix) else {
            break;
        };
        match rest.find('/') {
            _ if rest.is_empty() => {}
            Some(i) => {
                folders.insert((rest[..i].to_owned(), format!("{prefix}{}", &rest[..=i])));
            }
            None => {
                files.insert((rest.to_owned(), key.clone(), *size));
            }
        }
    }
    (folders, files)
}

/// Every folder prefix the keys imply, `prefix` included.
fn all_folders(prefix: &str, objects: &BTreeMap<String, u64>) -> BTreeSet<String> {
    let mut out = BTreeSet::from([prefix.to_owned()]);
    for key in objects.keys() {
        let rest = &key[prefix.len()..];
        for (i, _) in rest.match_indices('/') {
            out.insert(format!("{prefix}{}", &rest[..=i]));
        }
    }
    out
}

/// One folder as the dashboard shows it after loading every page: pages as
/// `(folders, files)` counts, entries in the order shown, and whether an
/// entry appeared twice.
#[derive(Debug, PartialEq)]
struct Opened {
    pages: Vec<(usize, usize)>,
    folders: Vec<(String, String)>,
    files: Vec<(String, String, u64, Option<String>)>,
    parent: Option<String>,
    breadcrumbs: Vec<(String, String)>,
}

async fn open_all(
    state: &Arc<CloudAccounts>,
    id: &str,
    prefix: &str,
) -> Result<Opened, CommandError> {
    let mut opened = Opened {
        pages: Vec::new(),
        folders: Vec::new(),
        files: Vec::new(),
        parent: None,
        breadcrumbs: Vec::new(),
    };
    let mut page = None;
    loop {
        let l: FolderListing =
            browser::list_folder(Arc::clone(state), id.into(), prefix.into(), page).await?;
        opened.pages.push((l.folders.len(), l.files.len()));
        opened.parent = l.parent;
        opened.breadcrumbs = l
            .breadcrumbs
            .into_iter()
            .map(|b| (b.name, b.prefix))
            .collect();
        opened
            .folders
            .extend(l.folders.into_iter().map(|f| (f.name, f.prefix)));
        opened.files.extend(
            l.files
                .into_iter()
                .map(|f| (f.name, f.key, f.size, f.last_modified)),
        );
        match l.next_page {
            Some(next) => page = Some(next),
            None => return Ok(opened),
        }
        assert!(opened.pages.len() < 50, "too many pages for {prefix:?}");
    }
}

fn dashboard_state(
    account: &ResolvedAccount,
    creds: &Credentials,
    bucket: &str,
) -> (Arc<CloudAccounts>, String) {
    let keys = Arc::new(MemoryCredentialStore::default());
    let state = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        ClientOptions::default(),
    ));
    let id = state
        .add(
            &AccountInput {
                label: "Browse validation".into(),
                provider: account.profile.id.into(),
                region: account.region.clone(),
                endpoint: Some(account.endpoint_url.clone()),
                bucket: bucket.into(),
            },
            CredentialInput::new(&creds.access_key_id, creds.secret_access_key.expose()),
        )
        .expect("account")
        .id;
    (state, id)
}

/// Collects named results; a failed check does not stop the others.
#[derive(Default)]
struct Report {
    failures: Vec<String>,
    passed: usize,
    /// Every folder as the dashboard listed it, by full prefix.
    listed: BTreeMap<String, Opened>,
}

impl Report {
    fn check(&mut self, ok: bool, what: impl Into<String>) {
        let what = what.into();
        if ok {
            self.passed += 1;
            println!("PASS {what}");
        } else {
            println!("FAIL {what}");
            self.failures.push(what);
        }
    }
}

/// Strips the run prefix for printing.
fn short<'a>(prefix: &str, s: &'a str) -> &'a str {
    s.strip_prefix(prefix).unwrap_or(s)
}

/// The checks, against the provider (`state`) and the local listing server
/// with the same keys (`local`).
/// What the checks need: the provider and local dashboard states, the raw
/// store, and what was uploaded.
struct Run {
    state: Arc<CloudAccounts>,
    id: String,
    local: Arc<CloudAccounts>,
    local_id: String,
    store: Arc<S3Store>,
    account: ResolvedAccount,
    creds: Credentials,
    bucket: String,
    prefix: String,
    objects: BTreeMap<String, u64>,
    created: Vec<Created>,
    started: String,
}

async fn validate(run: Run) -> Report {
    let Run {
        state,
        id,
        local,
        local_id,
        store,
        account,
        creds,
        bucket,
        prefix,
        objects,
        created,
        started,
    } = run;
    let mut r = Report::default();
    let p = |rel: &str| format!("{prefix}{rel}");

    // The bucket's root: this key is limited to `sotyware-it/`.
    let root = browser::list_folder(Arc::clone(&state), id.clone(), String::new(), None).await;
    println!(
        "bucket root: {}",
        match &root {
            Ok(l) => format!("{} folders, {} files", l.folders.len(), l.files.len()),
            Err(e) => format!("{:?}", e.kind),
        }
    );
    r.check(
        matches!(&root, Err(e) if e.kind == ErrorKind::AccessDenied) || root.is_ok(),
        "bucket root is listed or refused as Access denied",
    );
    let parent_of_run =
        browser::list_folder(Arc::clone(&state), id.clone(), BROWSE_ROOT.into(), None).await;
    r.check(
        matches!(&parent_of_run, Ok(l) if l.folders.iter().any(|f| f.prefix == prefix)
            && l.files.is_empty()),
        "sotyware-it/browse-validation/ shows the run folder, and no files",
    );

    // Every folder, opened the way the page opens it: only by prefixes the
    // listing returned, starting at the run prefix.
    let expected_folders = all_folders(&prefix, &objects);
    let mut seen = BTreeMap::new();
    let mut queue = vec![prefix.clone()];
    let mut local_diffs = Vec::new();
    while let Some(folder) = queue.pop() {
        let opened = match open_all(&state, &id, &folder).await {
            Ok(o) => o,
            Err(e) => {
                r.check(false, format!("open {:?}: {e:?}", short(&prefix, &folder)));
                continue;
            }
        };
        let (want_folders, want_files) = expected(&folder, &objects);
        let got_folders: BTreeSet<_> = opened.folders.iter().cloned().collect();
        let got_files: BTreeSet<_> = opened
            .files
            .iter()
            .map(|(n, k, s, _)| (n.clone(), k.clone(), *s))
            .collect();
        let name = short(&prefix, &folder).to_owned();
        let shown = if name.is_empty() { "<run root>" } else { &name };
        r.check(
            got_folders.len() == opened.folders.len() && got_files.len() == opened.files.len(),
            format!("{shown:?}: no folder or file listed twice"),
        );
        r.check(
            got_folders == want_folders,
            format!(
                "{shown:?}: exactly its immediate subfolders ({} expected, {} shown)",
                want_folders.len(),
                got_folders.len()
            ),
        );
        if got_folders != want_folders {
            println!("  expected {want_folders:?}\n  shown    {got_folders:?}");
        }
        r.check(
            got_files == want_files,
            format!(
                "{shown:?}: exactly its own files with sizes ({} expected, {} shown)",
                want_files.len(),
                got_files.len()
            ),
        );
        if got_files != want_files {
            let missing: Vec<_> = want_files.difference(&got_files).collect();
            let extra: Vec<_> = got_files.difference(&want_files).collect();
            println!("  missing {missing:?}\n  extra   {extra:?}");
        }
        let dated = opened.files.iter().all(|(_, _, _, d)| {
            d.as_deref()
                .is_some_and(|d| d.ends_with('Z') && *d >= *started.as_str())
        });
        r.check(
            dated,
            format!("{shown:?}: every file has a date from this run"),
        );
        // The same folder from the local listing server with the same keys.
        match open_all(&local, &local_id, &folder).await {
            Ok(l) => {
                let strip = |o: &Opened| {
                    (
                        o.pages.clone(),
                        o.folders.clone(),
                        o.files
                            .iter()
                            .map(|(n, k, s, _)| (n.clone(), k.clone(), *s))
                            .collect::<Vec<_>>(),
                        o.parent.clone(),
                        o.breadcrumbs.clone(),
                    )
                };
                if strip(&l) != strip(&opened) {
                    let order_only = l.pages == opened.pages
                        && l.folders.iter().collect::<BTreeSet<_>>()
                            == got_folders.iter().collect()
                        && l.files.len() == opened.files.len();
                    local_diffs.push(format!(
                        "{shown:?}: {} (pages B2 {:?}, local {:?})",
                        if order_only {
                            "order differs"
                        } else {
                            "content differs"
                        },
                        opened.pages,
                        l.pages
                    ));
                }
            }
            Err(e) => local_diffs.push(format!("{shown:?}: local server error {e:?}")),
        }
        for (_, sub) in &opened.folders {
            queue.push(sub.clone());
        }
        seen.insert(folder, opened);
    }
    let seen_set: BTreeSet<_> = seen.keys().cloned().collect();
    r.check(
        seen_set == expected_folders,
        format!(
            "every folder reachable by navigation, none extra ({} expected, {} opened)",
            expected_folders.len(),
            seen_set.len()
        ),
    );
    for missing in expected_folders.difference(&seen_set) {
        println!("  not reachable: {:?}", short(&prefix, missing));
    }
    println!(
        "local server vs Backblaze: {} folders differ{}",
        local_diffs.len(),
        if local_diffs.is_empty() { "" } else { ":" }
    );
    for d in &local_diffs {
        println!("  {d}");
    }

    // Named behaviours.
    let run_root = seen.get(&prefix);
    r.check(
        run_root.is_some_and(|o| {
            o.files.iter().all(|(n, ..)| !n.contains('/'))
                && !o.folders.iter().any(|(n, _)| n == "2024" || n == "deep")
        }),
        "run root: only immediate entries (no nested file or folder)",
    );
    r.check(
        seen.get(&p("docs/")).is_some_and(|o| {
            o.folders.iter().map(|(n, _)| n.as_str()).collect::<BTreeSet<_>>()
                == BTreeSet::from(["2024", "empty-nested"])
                && o.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>() == ["readme.txt"]
        }),
        "docs/: 2024/ and empty-nested/ as folders, readme.txt; deeper levels need separate opening",
    );
    r.check(
        seen.get(&p("photos/")).is_some_and(|o| {
            o.files
                .iter()
                .all(|(n, k, ..)| !n.is_empty() && *k != p("photos/"))
                && o.files.len() == 1
        }),
        "photos/: its marker object is not shown as a file",
    );
    for empty in ["marker-only/", "docs/empty-nested/"] {
        r.check(
            seen.get(&p(empty))
                .is_some_and(|o| o.folders.is_empty() && o.files.is_empty() && o.pages.len() == 1),
            format!("{empty}: a marker-only folder opens and is empty"),
        );
    }
    let root_names: BTreeSet<&str> = run_root
        .map(|o| o.files.iter().map(|f| f.0.as_str()).collect())
        .unwrap_or_default();
    r.check(
        root_names.contains("caf\u{e9} nfc.txt") && root_names.contains("cafe\u{301} nfd.txt"),
        "NFC and NFD names are two files, each byte-exact",
    );
    r.check(
        root_names.contains("root b.txt") && root_names.contains("\u{1f389} emoji.txt"),
        "a space and an emoji are preserved",
    );
    let special: BTreeSet<&str> = seen
        .get(&p("special/"))
        .map(|o| o.files.iter().map(|f| f.0.as_str()).collect())
        .unwrap_or_default();
    r.check(
        special
            == BTreeSet::from([
                "plus+sign.txt",
                "amp&ersand.txt",
                "question?.txt",
                "hash#tag.txt",
                "percent%.txt",
                "percent%20encoded.txt",
                "all + & ? # %.txt",
            ]),
        "special/: + & ? # % and a literal %20 preserved exactly",
    );
    r.check(
        seen.contains_key(&p("special/f+o&l?d#e%r/")),
        "a folder named f+o&l?d#e%r opens by its returned prefix",
    );
    r.check(
        seen.contains_key(&p(
            "\u{fc}n\u{ef}c\u{f8}d\u{e9}/\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}/",
        )),
        "Unicode folder names open by their returned prefix",
    );
    let dots = objects.contains_key(&p("dots/../escape.txt"));
    if dots {
        let o = seen.get(&p("dots/../"));
        r.check(
            o.is_some_and(|o| {
                o.files.iter().map(|f| f.1.as_str()).collect::<Vec<_>>()
                    == [p("dots/../escape.txt")]
                    && o.parent.as_deref() == Some(p("dots/").as_str())
                    && o.breadcrumbs.last().map(|b| b.0.as_str()) == Some("..")
            }),
            "dots/../ is a folder named '..' inside dots/, its parent is dots/",
        );
        r.check(
            seen.get(&prefix)
                .is_some_and(|o| !o.files.iter().any(|f| f.0 == "escape.txt")),
            "escape.txt does not appear at the run root",
        );
    }
    if objects.contains_key(&p("double//inner.txt")) {
        r.check(
            seen.get(&p("double/"))
                .is_some_and(|o| o.folders == [(String::new(), p("double//"))]),
            "double/ has one folder with an empty name, prefix double//",
        );
        r.check(
            seen.get(&p("double//"))
                .is_some_and(|o| o.files.iter().any(|f| f.0 == "inner.txt")),
            "double// lists inner.txt",
        );
    }

    // Pagination in many/: 1,010 immediate entries, pages of 1,000.
    let many = p("many/");
    match seen.get(&many) {
        Some(o) => {
            println!("many/: pages (folders, files) {:?}", o.pages);
            let entries: usize = o.pages.iter().map(|(f, n)| f + n).sum();
            r.check(
                o.pages.len() == 2
                    && o.pages[0].0 + o.pages[0].1 == LIST_PAGE_SIZE as usize
                    && entries == MANY_FILES + MANY_FOLDERS,
                format!(
                    "many/: {} entries in 2 pages, the first of {LIST_PAGE_SIZE}",
                    MANY_FILES + MANY_FOLDERS
                ),
            );
        }
        None => r.check(false, "many/ was opened"),
    }
    // A cursor works once.
    let first = browser::list_folder(Arc::clone(&state), id.clone(), many.clone(), None).await;
    match first {
        Ok(first) => {
            let cursor = first.next_page.clone();
            r.check(cursor.is_some(), "many/: the first page offers Load more");
            if let Some(cursor) = cursor {
                let second = browser::list_folder(
                    Arc::clone(&state),
                    id.clone(),
                    many.clone(),
                    Some(cursor.clone()),
                )
                .await;
                r.check(
                    matches!(&second, Ok(l) if l.next_page.is_none()
                        && l.folders.len() + l.files.len() == MANY_FILES + MANY_FOLDERS - LIST_PAGE_SIZE as usize),
                    "many/: Load more gives the rest and no further page",
                );
                let again = browser::list_folder(
                    Arc::clone(&state),
                    id.clone(),
                    many.clone(),
                    Some(cursor),
                )
                .await;
                r.check(
                    matches!(&again, Err(e) if e.kind == ErrorKind::OutdatedPage),
                    "many/: a used cursor is refused (outdated page)",
                );
            }
            // Refresh: a new first page, the same as before.
            let refreshed =
                browser::list_folder(Arc::clone(&state), id.clone(), many.clone(), None).await;
            r.check(
                matches!(&refreshed, Ok(l) if l.folders == first.folders && l.files.iter().map(|f| &f.key).eq(first.files.iter().map(|f| &f.key))
                    && l.next_page.is_some() && l.next_page != first.next_page),
                "many/: Refresh starts again at the same first page, with a new cursor",
            );
            // Switching folders: a cursor of many/ is not valid for docs/.
            if let Ok(l) = &refreshed
                && let Some(cursor) = l.next_page.clone()
            {
                let other = browser::list_folder(
                    Arc::clone(&state),
                    id.clone(),
                    p("docs/"),
                    Some(cursor.clone()),
                )
                .await;
                r.check(
                    matches!(&other, Err(e) if e.kind == ErrorKind::OutdatedPage),
                    "a cursor of many/ is refused for docs/",
                );
                let docs =
                    browser::list_folder(Arc::clone(&state), id.clone(), p("docs/"), None).await;
                r.check(
                    matches!(&docs, Ok(d) if d.next_page.is_none() && d.files.len() == 1),
                    "docs/ opens at its first page after many/",
                );
                // A provider token that B2 did not issue, behind a valid cursor.
                state.replace_cursor_token(&cursor, "bm90LWEtdG9rZW4=");
                let bogus = browser::list_folder(
                    Arc::clone(&state),
                    id.clone(),
                    many.clone(),
                    Some(cursor),
                )
                .await;
                println!(
                    "many/: a made-up provider token: {}",
                    match &bogus {
                        Ok(l) => format!(
                            "accepted, {} entries, more: {}",
                            l.folders.len() + l.files.len(),
                            l.next_page.is_some()
                        ),
                        Err(e) => format!("{:?}: {}", e.kind, e.message),
                    }
                );
            }
        }
        Err(e) => r.check(false, format!("many/ first page: {e:?}")),
    }

    // The provider itself, in small pages: the same set, no entry twice,
    // only immediate children, only delimited ListObjectsV2.
    let requests = Requests::default();
    let raw =
        S3Store::new(account, creds, ClientOptions::default()).with_interceptor(requests.clone());
    for (folder, size) in [(prefix.clone(), 4), (many.clone(), 7), (many.clone(), 1000)] {
        let (entries, pages) = super::walk_pages(&raw, &bucket, &folder, size, 400).await;
        let unique: BTreeSet<_> = entries.iter().collect();
        let (wf, wfiles) = expected(&folder, &objects);
        let marker = objects.contains_key(&folder) as usize;
        r.check(
            unique.len() == entries.len() && entries.len() == wf.len() + wfiles.len() + marker,
            format!(
                "provider pages of {size} in {:?}: {} entries in {pages} pages, none twice",
                short(&prefix, &folder),
                entries.len()
            ),
        );
    }
    let sent = requests.take();
    r.check(
        sent.iter().all(|q| {
            q.starts_with("GET ") && q.contains("list-type=2") && q.contains("delimiter=%2F")
        }),
        format!(
            "{} provider listings, all delimited ListObjectsV2 (never recursive)",
            sent.len()
        ),
    );

    // The stored objects are the ones uploaded: size and SHA-256 by HEAD,
    // for every key outside many/.
    let mut heads_ok = 0;
    let mut heads = 0;
    for c in created.iter().filter(|c| !c.key.starts_with(&many)) {
        heads += 1;
        match store.head_object(&bucket, &c.key).await {
            Ok(Some(d)) if d.size == c.size && d.sha256.as_deref() == Some(c.sha256.as_str()) => {
                heads_ok += 1
            }
            other => println!("  HEAD {:?}: {other:?}", short(&prefix, &c.key)),
        }
    }
    r.check(
        heads_ok == heads,
        format!("HEAD by exact key: {heads_ok}/{heads} with the uploaded size and SHA-256"),
    );
    r.listed = seen;
    r
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: writes a temporary dataset to the Backblaze B2 test bucket"]
async fn backblaze_b2_browse_dataset() {
    let (t, resolved, creds, store) = b2_store();
    let keep = std::env::var("SOTYWARE_IT_B2_BROWSE_KEEP").as_deref() == Ok("1");
    let run = new_run_id();
    let prefix = run_prefix(&run);
    println!(
        "provider {}, region {}, endpoint {}, run prefix {prefix}",
        resolved.profile.id, resolved.region, resolved.endpoint_url
    );
    assert_eq!(resolved.profile.id, "backblaze-b2");
    assert_eq!(
        resolved.endpoint_url,
        format!("https://s3.{}.backblazeb2.com", resolved.region),
        "the configured endpoint must be Backblaze's own"
    );
    assert!(prefix.starts_with(B2_KEY_PREFIX));
    store
        .check_bucket(&t.bucket)
        .await
        .expect("connection check");
    println!("connection check: connected (HeadBucket on the configured bucket)");
    let before = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
    println!("versions under {B2_KEY_PREFIX} before: {}", before.1);
    // The run prefix must be new: no object, no version, no delete marker.
    let listed = store.list_objects(&t.bucket, &prefix).await.expect("list");
    let versions = try_versions(&store, &t.bucket, &prefix)
        .await
        .expect("versions");
    assert!(
        listed.is_empty() && versions.is_empty(),
        "the run prefix is not empty: {listed:?} {versions:?}"
    );
    println!("run prefix: no objects, versions or delete markers");

    let items = dataset();
    let keys = run_keys(&prefix);
    let store = Arc::new(store);
    let created = Arc::new(Mutex::new(Vec::new()));

    // Permissions, before the dataset: one write, its read and its delete
    // by version id, all inside the run prefix.
    let probe = Item {
        rel: PROBE.into(),
        body: b"sotyware permission probe\n".to_vec(),
        required: true,
    };
    let probe_key = format!("{prefix}{PROBE}");
    let probe_refused = upload(&store, &t.bucket, &prefix, &[probe], &created).await;
    assert!(
        probe_refused.is_empty(),
        "the key may not write: {probe_refused:?}"
    );
    let probe_version = created.lock().unwrap()[0]
        .version
        .clone()
        .expect("a version id");
    assert!(
        store
            .head_object(&t.bucket, &probe_key)
            .await
            .expect("read")
            .is_some()
    );
    store
        .delete_object(&t.bucket, &probe_key, Some(&probe_version))
        .await
        .expect("the key may not delete versions");
    created.lock().unwrap().clear();
    assert!(
        try_versions(&store, &t.bucket, &prefix)
            .await
            .expect("versions")
            .is_empty(),
        "the probe left a version"
    );
    println!("permissions: write, read and delete by version id work inside the run prefix");

    let started = DateTime::from(SystemTime::now() - Duration::from_secs(300))
        .fmt(DateTimeFormat::DateTime)
        .unwrap();
    let t0 = std::time::Instant::now();
    let refused = upload(&store, &t.bucket, &prefix, &items, &created).await;
    let made = created.lock().unwrap().clone();
    let bytes: u64 = made.iter().map(|c| c.size).sum();
    println!(
        "dataset: {} of {} objects created ({bytes} bytes) in {:.1} s, {} refused",
        made.len(),
        items.len(),
        t0.elapsed().as_secs_f64(),
        refused.len()
    );
    let required_refused: Vec<_> = refused
        .iter()
        .filter(|(k, _)| {
            items
                .iter()
                .any(|i| i.required && format!("{prefix}{}", i.rel) == *k)
        })
        .collect();

    // What the provider really stored under the prefix, by version listing.
    let stored = try_versions(&store, &t.bucket, &prefix)
        .await
        .unwrap_or_default();
    let stored_keys: BTreeSet<&str> = stored.iter().map(|(k, ..)| k.as_str()).collect();
    let made_keys: BTreeSet<&str> = made.iter().map(|c| c.key.as_str()).collect();
    println!(
        "stored: {} versions, {} keys; equal to the created keys: {}",
        stored.len(),
        stored_keys.len(),
        stored_keys == made_keys
    );
    for k in stored_keys.symmetric_difference(&made_keys) {
        println!("  stored key differs from what was sent: {k:?}");
    }
    let objects: BTreeMap<String, u64> = made.iter().map(|c| (c.key.clone(), c.size)).collect();
    // In key order: uploads finish in any order.
    let mut sorted: Vec<&Created> = made.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let mut sha = Sha256::new();
    for c in sorted {
        sha.update(format!(
            "{}\0{}\0{}\n",
            &c.key[prefix.len()..],
            c.size,
            c.sha256
        ));
    }
    println!(
        "dataset manifest (relative key, size, sha-256) digest: {}",
        sha.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );

    // The same keys in the local listing server.
    let local_objects: Vec<(&str, &str, u64)> = objects
        .iter()
        .map(|(k, s)| ("it-bucket", k.as_str(), *s))
        .collect();
    let server = TestServer::start_listing(
        "test-key",
        "test-secret",
        &["it-bucket"],
        &local_objects,
        &[],
    )
    .await;
    let mut local_config = super::super::account::AccountConfig::new(
        "it-local",
        "it-local",
        "s3-compatible",
        "us-east-1",
    );
    local_config.endpoint = Some(server.endpoint.clone());
    let local_resolved = local_config.resolve().unwrap();
    let (local, local_id) = dashboard_state(&local_resolved, &server.credentials(), "it-bucket");
    let (state, id) = dashboard_state(&resolved, &creds, &t.bucket);

    let checks = tokio::spawn(validate(Run {
        state,
        id,
        local,
        local_id,
        store: Arc::clone(&store),
        account: resolved.clone(),
        creds: creds.clone(),
        bucket: t.bucket.clone(),
        prefix: prefix.clone(),
        objects,
        created: made.clone(),
        started,
    }))
    .await;

    // The UI tests' key, if set: it must see the bucket's root and the run,
    // and must not be able to write (a refused PutObject inside the run).
    let mut read_key_ok = true;
    if let Some(read_creds) = read_key() {
        let reader = Arc::new(S3Store::new(
            resolved.clone(),
            read_creds,
            ClientOptions::default(),
        ));
        let root = reader.list_folder(&t.bucket, "", None, 1000).await;
        let run_page = reader.list_folder(&t.bucket, &prefix, None, 1000).await;
        let probe = Item {
            rel: READ_PROBE.into(),
            body: b"must be refused\n".to_vec(),
            required: false,
        };
        let written = Arc::new(Mutex::new(Vec::new()));
        let refused = upload(&reader, &t.bucket, &prefix, &[probe], &written).await;
        let leaked: Vec<Created> = written.lock().unwrap().clone();
        for c in &leaked {
            // Not read-only after all: remove exactly that version now.
            if let Some(v) = &c.version {
                let _ = store.delete_object(&t.bucket, &c.key, Some(v)).await;
            }
        }
        println!(
            "read-only UI key: root {}, run prefix {}, write {}",
            match &root {
                Ok(p) => format!("listed ({} folders)", p.prefixes.len()),
                Err(e) => format!("refused ({e})"),
            },
            match &run_page {
                Ok(p) => format!("listed ({} entries)", p.prefixes.len() + p.objects.len()),
                Err(e) => format!("refused ({e})"),
            },
            if leaked.is_empty() && refused.len() == 1 {
                "refused"
            } else {
                "ACCEPTED"
            }
        );
        read_key_ok = root.is_ok() && run_page.is_ok() && leaked.is_empty() && refused.len() == 1;
    } else if keep {
        println!("no read-only UI key set (SOTYWARE_CLOUD_IT_B2_READ_*)");
    }

    let cleaned = if keep {
        if let (Ok(report), Ok(path)) = (&checks, std::env::var("SOTYWARE_IT_B2_BROWSE_MANIFEST")) {
            let m = manifest(&run, &prefix, &report.listed);
            std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).expect("write manifest");
            println!("manifest: {} folders written", report.listed.len());
        }
        println!("SOTYWARE_IT_B2_BROWSE_KEEP=1: dataset kept; run id {run}");
        println!("clean up with SOTYWARE_IT_B2_BROWSE_RUN={run} and backblaze_b2_browse_cleanup");
        None
    } else {
        let result = cleanup_run(&store, &t.bucket, &prefix, &keys).await;
        let after = fingerprint(&store, &t.bucket, B2_KEY_PREFIX).await;
        println!(
            "versions under {B2_KEY_PREFIX}: {} before, {} after; fingerprint {}",
            before.1,
            after.1,
            if before == after {
                "unchanged"
            } else {
                "CHANGED"
            }
        );
        Some((result, before == after))
    };

    let report = match checks {
        Ok(report) => report,
        Err(e) => panic!("the checks panicked (cleanup ran first): {e}"),
    };
    println!(
        "checks: {} passed, {} failed",
        report.passed,
        report.failures.len()
    );
    if let Some((result, unchanged)) = cleaned {
        let removed = result.unwrap_or_else(|e| panic!("CLEANUP INCOMPLETE: {e}"));
        println!("cleanup: verified, {removed} versions/markers removed, none left");
        assert!(unchanged, "sotyware-it/ differs from before the run");
    }
    assert!(
        required_refused.is_empty(),
        "required keys refused: {required_refused:?}"
    );
    assert!(
        read_key_ok,
        "the read-only UI key must list the root and the run, and must not write"
    );
    assert!(report.failures.is_empty(), "failed: {:#?}", report.failures);
}

/// Cleanup alone, for a run id from `SOTYWARE_IT_B2_BROWSE_RUN` (a kept or
/// interrupted run): deletes, by version id, only dataset keys of that run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: deletes a browse-validation run's dataset from the Backblaze B2 test bucket"]
async fn backblaze_b2_browse_cleanup() {
    let (t, _, _, store) = b2_store();
    let run = std::env::var("SOTYWARE_IT_B2_BROWSE_RUN")
        .expect("set SOTYWARE_IT_B2_BROWSE_RUN to the run id to clean up");
    let prefix = run_prefix(&run);
    let keys = run_keys(&prefix);
    println!("cleaning up {prefix}");
    let removed = cleanup_run(&store, &t.bucket, &prefix, &keys)
        .await
        .unwrap_or_else(|e| panic!("CLEANUP INCOMPLETE: {e}"));
    println!("cleanup: verified, {removed} versions/markers removed, none left");
}

/// Every check of `backblaze_b2_browse_dataset` against the in-process
/// listing server holding the dataset, so the checks are known to pass where
/// the listing is right before they meet the provider (no HEAD: that server
/// only lists, and `created` is empty here).
#[tokio::test(flavor = "multi_thread")]
async fn browse_checks_pass_against_the_test_server() {
    let prefix = run_prefix("local");
    let objects: BTreeMap<String, u64> = dataset()
        .iter()
        .map(|i| (format!("{prefix}{}", i.rel), i.body.len() as u64))
        .collect();
    let list: Vec<(&str, &str, u64)> = objects
        .iter()
        .map(|(k, s)| ("it-bucket", k.as_str(), *s))
        .collect();
    let server =
        TestServer::start_listing("test-key", "test-secret", &["it-bucket"], &list, &[]).await;
    let mut config = super::super::account::AccountConfig::new(
        "it-local",
        "it-local",
        "s3-compatible",
        "us-east-1",
    );
    config.endpoint = Some(server.endpoint.clone());
    let resolved = config.resolve().unwrap();
    let (state, id) = dashboard_state(&resolved, &server.credentials(), "it-bucket");
    let (local, local_id) = dashboard_state(&resolved, &server.credentials(), "it-bucket");
    let store = Arc::new(S3Store::new(
        resolved.clone(),
        server.credentials(),
        ClientOptions::default(),
    ));
    let r = validate(Run {
        state,
        id,
        local,
        local_id,
        store,
        account: resolved,
        creds: server.credentials(),
        bucket: "it-bucket".into(),
        prefix,
        objects,
        created: Vec::new(),
        started: "2000-01-01T00:00:00Z".into(),
    })
    .await;
    assert!(r.failures.is_empty(), "{:#?}", r.failures);
    assert!(r.passed > 100, "{}", r.passed);
}

//! An S3 server inside the test process: `s3s` speaks the protocol (including
//! SigV4 authentication and multipart uploads), `s3s-fs` keeps objects in a
//! temporary folder. Listens on 127.0.0.1 only; nothing leaves the machine.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::access::{S3Access, S3AccessContext};
use s3s::auth::SimpleAuth;
use s3s::dto::{HeadBucketInput, HeadBucketOutput, ListObjectsV2Input, ListObjectsV2Output};
use s3s::service::S3ServiceBuilder;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::account::{AccountConfig, ResolvedAccount};
use super::credentials::{Credentials, Secret};

/// A running test server. Stopped and its folder removed on drop.
pub struct TestServer {
    pub endpoint: String,
    pub access_key_id: String,
    pub secret: String,
    /// The S3 operation names (`HeadBucket`, `ListBuckets`, …) of every
    /// authenticated request, in order.
    pub operations: Arc<Mutex<Vec<String>>>,
    root: PathBuf,
    task: JoinHandle<()>,
}

/// Records every authenticated operation and refuses the `denied` ones with
/// `AccessDenied`, as a provider does for a key without that permission (a
/// Backblaze key restricted to one bucket is refused `ListBuckets`).
struct Access {
    denied: Vec<&'static str>,
    operations: Arc<Mutex<Vec<String>>>,
}

// Written out instead of with `#[async_trait]`, which would be a new
// dependency only for this; it is the signature that macro generates.
impl S3Access for Access {
    fn check<'a, 'b, 'c, 'f>(
        &'a self,
        cx: &'b mut S3AccessContext<'c>,
    ) -> Pin<Box<dyn Future<Output = s3s::S3Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        'c: 'f,
        Self: 'f,
    {
        let op = cx.s3_op().name().to_owned();
        let signed = cx.credentials().is_some();
        Box::pin(async move {
            if !signed {
                return Err(s3s::s3_error!(AccessDenied, "Signature is required"));
            }
            self.operations.lock().unwrap().push(op.clone());
            if self.denied.contains(&op.as_str()) {
                return Err(s3s::s3_error!(AccessDenied, "not entitled"));
            }
            Ok(())
        })
    }
}

/// A fresh, empty folder for one server.
fn fresh_root() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "sotyware-s3-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The in-memory buckets of [`TestServer::start_listing`]: key → size.
#[derive(Default)]
struct Listing {
    buckets: HashMap<String, BTreeMap<String, u64>>,
}

/// The time every listed object was "last modified".
pub const LISTING_MODIFIED_SECS: u64 = 1_762_862_400; // 2025-11-11T12:00:00Z

fn token_for(entry: &str) -> String {
    entry.bytes().map(|b| format!("{b:02x}")).collect()
}

fn entry_from_token(token: &str) -> Option<String> {
    if token.is_empty() || !token.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..token.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(token.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

impl Listing {
    fn list(&self, input: ListObjectsV2Input) -> s3s::S3Result<ListObjectsV2Output> {
        let Some(keys) = self.buckets.get(&input.bucket) else {
            return Err(s3s::s3_error!(NoSuchBucket));
        };
        let prefix = input.prefix.clone().unwrap_or_default();
        let delimiter = input.delimiter.clone().filter(|d| !d.is_empty());
        let max_keys = usize::try_from(input.max_keys.unwrap_or(1000).clamp(0, 1000)).unwrap_or(0);
        // Everything up to and including `after` was on earlier pages.
        let after = match &input.continuation_token {
            Some(token) => Some(entry_from_token(token).ok_or_else(|| {
                s3s::s3_error!(
                    InvalidArgument,
                    "The continuation token provided is incorrect"
                )
            })?),
            None => input.start_after.clone(),
        };
        let modified = s3s::dto::Timestamp::from(
            SystemTime::UNIX_EPOCH + Duration::from_secs(LISTING_MODIFIED_SECS),
        );
        let mut contents = Vec::new();
        let mut common = Vec::new();
        let mut last: Option<String> = None;
        let mut truncated = false;
        for (key, size) in keys.iter().filter(|(k, _)| k.starts_with(&prefix)) {
            let rest = &key[prefix.len()..];
            let (entry, is_folder) = match delimiter
                .as_deref()
                .and_then(|d| rest.find(d).map(|i| (d, i)))
            {
                Some((d, i)) => (format!("{prefix}{}", &rest[..i + d.len()]), true),
                None => (key.clone(), false),
            };
            if after.as_deref().is_some_and(|a| entry.as_str() <= a)
                || last.as_deref() == Some(entry.as_str())
            {
                continue;
            }
            if contents.len() + common.len() == max_keys {
                truncated = true;
                break;
            }
            if !is_folder {
                contents.push(s3s::dto::Object {
                    key: Some(key.clone()),
                    size: Some(i64::try_from(*size).unwrap_or(i64::MAX)),
                    last_modified: Some(modified.clone()),
                    ..Default::default()
                });
            } else {
                common.push(s3s::dto::CommonPrefix {
                    prefix: Some(entry.clone()),
                });
            }
            last = Some(entry);
        }
        Ok(ListObjectsV2Output {
            key_count: Some(i32::try_from(contents.len() + common.len()).unwrap_or(i32::MAX)),
            max_keys: input.max_keys,
            is_truncated: Some(truncated),
            next_continuation_token: truncated.then(|| token_for(last.as_deref().unwrap_or(""))),
            continuation_token: input.continuation_token,
            contents: (!contents.is_empty()).then_some(contents),
            common_prefixes: (!common.is_empty()).then_some(common),
            delimiter: input.delimiter,
            name: Some(input.bucket),
            prefix: input.prefix,
            ..Default::default()
        })
    }
}

// The two operations a listing needs, written out like `Access::check`.
impl s3s::S3 for Listing {
    fn list_objects_v2<'life0, 'async_trait>(
        &'life0 self,
        req: s3s::S3Request<ListObjectsV2Input>,
    ) -> Pin<
        Box<
            dyn Future<Output = s3s::S3Result<s3s::S3Response<ListObjectsV2Output>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let result = self.list(req.input).map(s3s::S3Response::new);
        Box::pin(async move { result })
    }

    fn head_bucket<'life0, 'async_trait>(
        &'life0 self,
        req: s3s::S3Request<HeadBucketInput>,
    ) -> Pin<
        Box<
            dyn Future<Output = s3s::S3Result<s3s::S3Response<HeadBucketOutput>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let result = if self.buckets.contains_key(&req.input.bucket) {
            Ok(s3s::S3Response::new(HeadBucketOutput::default()))
        } else {
            Err(s3s::s3_error!(NoSuchBucket))
        };
        Box::pin(async move { result })
    }
}

impl TestServer {
    /// Starts a server that accepts only `access_key_id`/`secret`, with the
    /// given (empty) buckets.
    pub async fn start(access_key_id: &str, secret: &str, buckets: &[&str]) -> Self {
        Self::start_denying(access_key_id, secret, buckets, &[]).await
    }

    /// Like [`Self::start`], but the key is refused the `denied` operations.
    pub async fn start_denying(
        access_key_id: &str,
        secret: &str,
        buckets: &[&str],
        denied: &[&'static str],
    ) -> Self {
        let root = fresh_root();
        // s3s-fs stores a bucket as a folder.
        for bucket in buckets {
            fs::create_dir_all(root.join(bucket)).unwrap();
        }
        let fs = s3s_fs::FileSystem::new(&root).unwrap();
        Self::serve(fs, root, access_key_id, secret, denied).await
    }

    /// A server that only lists: the buckets hold exactly the given
    /// `(bucket, key, size)` objects, keys kept as they are (`a//b`, keys
    /// ending in `/`, spaces, any Unicode), with real `ListObjectsV2`
    /// semantics: prefix, delimiter, `max-keys` and continuation tokens
    /// (s3s-fs keeps objects as files, so it cannot hold such keys, and it
    /// returns no continuation token). Every other operation is refused.
    pub async fn start_listing(
        access_key_id: &str,
        secret: &str,
        buckets: &[&str],
        objects: &[(&str, &str, u64)],
        denied: &[&'static str],
    ) -> Self {
        let mut listing = Listing::default();
        for bucket in buckets {
            listing.buckets.entry((*bucket).to_owned()).or_default();
        }
        for (bucket, key, size) in objects {
            listing
                .buckets
                .entry((*bucket).to_owned())
                .or_default()
                .insert((*key).to_owned(), *size);
        }
        Self::serve(listing, fresh_root(), access_key_id, secret, denied).await
    }

    async fn serve(
        backend: impl s3s::S3,
        root: PathBuf,
        access_key_id: &str,
        secret: &str,
        denied: &[&'static str],
    ) -> Self {
        let mut builder = S3ServiceBuilder::new(backend);
        builder.set_auth(SimpleAuth::from_single(access_key_id, secret));
        let operations = Arc::new(Mutex::new(Vec::new()));
        builder.set_access(Access {
            denied: denied.to_vec(),
            operations: Arc::clone(&operations),
        });
        let service = builder.build();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let service = service.clone();
                tokio::spawn(async move {
                    let _ = ConnBuilder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self {
            endpoint: format!("http://{addr}"),
            access_key_id: access_key_id.into(),
            secret: secret.into(),
            operations,
            root,
            task,
        }
    }

    /// The file `s3s-fs` keeps an object's content in.
    pub fn object_path(&self, bucket: &str, key: &str) -> PathBuf {
        self.root.join(bucket).join(key)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// An `s3-compatible` account pointing at this server.
    pub fn account(&self, id: &str, buckets: &[&str]) -> AccountConfig {
        let mut c = AccountConfig::new(id, id, "s3-compatible", "us-east-1");
        c.endpoint = Some(self.endpoint.clone());
        c.buckets = buckets.iter().map(|b| (*b).to_owned()).collect();
        c
    }

    pub fn resolved(&self, id: &str, buckets: &[&str]) -> ResolvedAccount {
        self.account(id, buckets).resolve().unwrap()
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: Secret::new(self.secret.clone()),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A fresh local folder, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sotyware-cloud-test-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Writes `size` bytes of a deterministic, non-repeating pattern.
    pub fn file(&self, name: &str, size: usize, seed: u8) -> PathBuf {
        let mut x = u32::from(seed).wrapping_mul(2_654_435_761) | 1;
        let data: Vec<u8> = (0..size)
            .map(|_| {
                // xorshift: cheap, and different for every byte position.
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        let path = self.path(name);
        fs::write(&path, data).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What the Cloud Storage UI test browses (tests/desktop/cloud_ui.py,
/// phase `browse`): nested folders, odd names, a folder that is only a
/// marker, and a folder of 1,005 entries (more than one page).
pub fn ui_browse_objects() -> Vec<(&'static str, String, u64)> {
    let mut objects: Vec<(&'static str, String, u64)> = [
        ("readme.txt", 12),
        ("photos/", 0),
        ("photos/cat.jpg", 2048),
        ("photos/2024/beach.jpg", 3_145_728),
        ("Ferien 2024 ü/Strand Bild (1).jpg", 1_500),
        ("Ferien 2024 ü/a+b=c&d.txt", 7),
        ("empty/", 0),
    ]
    .into_iter()
    .map(|(k, s)| (UI_BROWSE_BUCKET, k.to_owned(), s))
    .collect();
    objects.extend((0..1005).map(|i| (UI_BROWSE_BUCKET, format!("many/item-{i:04}.txt"), 1)));
    objects
}

pub const UI_BROWSE_BUCKET: &str = "browse-bucket";

/// Not a test: serves [`ui_browse_objects`] for the Cloud Storage UI test
/// until the file named by `SOTYWARE_UI_SERVER_FILE` (where it writes its
/// endpoint) is deleted, then fails if anything but `ListObjectsV2` (or the
/// connection check's `HeadBucket`) was requested. Started by
/// tests/desktop/cloud-ui.sh (phase `browse`) and windows-cloud-ui.ps1
/// (`-Mode browse`); the key and secret are made up.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the Cloud Storage UI test, not a test"]
async fn serve_listing_for_ui() {
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let objects = ui_browse_objects();
    let objects: Vec<(&str, &str, u64)> = objects
        .iter()
        .map(|(b, k, s)| (*b, k.as_str(), *s))
        .collect();
    let server = TestServer::start_listing(
        "e2eBrowseKey0000000001",
        "e2e-Browse-Secret-0123456789",
        &[UI_BROWSE_BUCKET],
        &objects,
        &[],
    )
    .await;
    fs::write(&file, &server.endpoint).unwrap();
    println!("serving {} at {}", UI_BROWSE_BUCKET, server.endpoint);
    // A test helper, so a plain wait loop is fine here.
    while Path::new(&file).exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ops = server.operations.lock().unwrap().clone();
    let lists = ops.iter().filter(|op| *op == "ListObjectsV2").count();
    println!(
        "operations: {} ListObjectsV2, others: {:?}",
        lists,
        ops.iter()
            .filter(|op| *op != "ListObjectsV2")
            .collect::<Vec<_>>()
    );
    assert!(
        ops.iter()
            .all(|op| op == "ListObjectsV2" || op == "HeadBucket"),
        "browsing must only list: {ops:?}"
    );
}

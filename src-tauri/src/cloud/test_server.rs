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
use s3s::dto::{
    GetObjectInput, GetObjectOutput, HeadBucketInput, HeadBucketOutput, ListObjectsV2Input,
    ListObjectsV2Output,
};
use s3s::service::S3ServiceBuilder;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::account::{AccountConfig, ResolvedAccount};
use super::credentials::{Credentials, Secret};

mod versioned;
pub use versioned::{Version, Versioned};

mod files_ui;

/// A running test server. Stopped and its folder removed on drop.
pub struct TestServer {
    pub endpoint: String,
    pub access_key_id: String,
    pub secret: String,
    /// The S3 operation names (`HeadBucket`, `ListBuckets`, …) of every
    /// authenticated request, in order.
    pub operations: Arc<Mutex<Vec<String>>>,
    /// The same operations with the access key that signed each, so a
    /// desktop test can tell the app's requests from the helper's own
    /// ([`HELPER_KEY_ID`]).
    keyed: Arc<Mutex<Vec<(String, String)>>>,
    injections: Arc<Mutex<Vec<Injection>>>,
    conditional: Arc<Mutex<Vec<String>>>,
    root: PathBuf,
    task: JoinHandle<()>,
}

/// The second key every [`TestServer`] accepts ([`TestServer::helper_credentials`]).
pub const HELPER_KEY_ID: &str = "e2eHelperKey0000000001";
const HELPER_SECRET: &str = "e2e-Helper-Secret-0123456789";

/// What an injected fault does to a matching request, before the server
/// handles it (and before it reads the request body).
#[derive(Clone)]
pub enum Inject {
    /// Refuse with `InternalError` (HTTP 500): transient, so the SDK retries.
    InternalError,
    /// Refuse with `AccessDenied` (HTTP 403): not retried.
    Deny,
    /// Never answer.
    Hang,
    /// Run this, then handle the request normally.
    Run(Arc<dyn Fn() + Send + Sync>),
}

struct Injection {
    op: &'static str,
    /// Only requests for this part number (`UploadPart`).
    part: Option<u32>,
    /// Matching requests left to affect.
    times: usize,
    action: Inject,
}

/// The `partNumber` query parameter of a request.
fn part_number(query: Option<&str>) -> Option<u32> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("partNumber="))
        .and_then(|n| n.parse().ok())
}

/// Records every authenticated operation (`UploadPart` with its part
/// number, as `UploadPart#2`), refuses the `denied` ones with
/// `AccessDenied`, as a provider does for a key without that permission (a
/// Backblaze key restricted to one bucket is refused `ListBuckets`), and
/// applies injected faults.
struct Access {
    denied: Vec<&'static str>,
    operations: Arc<Mutex<Vec<String>>>,
    keyed: Arc<Mutex<Vec<(String, String)>>>,
    injections: Arc<Mutex<Vec<Injection>>>,
    /// Where s3s-fs keeps objects, to enforce `If-None-Match: *` the way
    /// Amazon documents it (412 when the key exists). s3s-fs itself ignores
    /// the header. Servers without s3s-fs get a folder that never holds an
    /// object, so nothing is refused there.
    root: PathBuf,
    /// The operations that carried `If-None-Match: *`, in order.
    conditional: Arc<Mutex<Vec<String>>>,
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
        let part = part_number(cx.uri().query());
        let signed = cx.credentials().is_some();
        let key = cx
            .credentials()
            .map(|c| c.access_key.clone())
            .unwrap_or_default();
        let if_none_match = (op == "PutObject" || op == "CompleteMultipartUpload")
            && cx
                .headers()
                .get("if-none-match")
                .is_some_and(|v| v.as_bytes() == b"*");
        let target = match cx.s3_path() {
            s3s::path::S3Path::Object { bucket, key } => {
                Some(self.root.join(bucket.as_ref()).join(key.as_ref()))
            }
            _ => None,
        };
        Box::pin(async move {
            if !signed {
                return Err(s3s::s3_error!(AccessDenied, "Signature is required"));
            }
            let recorded = match part {
                Some(n) if op == "UploadPart" => format!("{op}#{n}"),
                _ => op.clone(),
            };
            self.operations.lock().unwrap().push(recorded.clone());
            self.keyed.lock().unwrap().push((key, recorded));
            if self.denied.contains(&op.as_str()) {
                return Err(s3s::s3_error!(AccessDenied, "not entitled"));
            }
            let action = {
                let mut injections = self.injections.lock().unwrap();
                injections
                    .iter_mut()
                    .find(|i| i.op == op && i.times > 0 && (i.part.is_none() || i.part == part))
                    .map(|i| {
                        i.times -= 1;
                        i.action.clone()
                    })
            };
            match action {
                None => {}
                Some(Inject::InternalError) => {
                    return Err(s3s::s3_error!(InternalError, "injected by the test"));
                }
                Some(Inject::Deny) => {
                    return Err(s3s::s3_error!(AccessDenied, "injected by the test"));
                }
                Some(Inject::Hang) => std::future::pending().await,
                Some(Inject::Run(f)) => f(),
            }
            if if_none_match {
                self.conditional.lock().unwrap().push(op.clone());
                if target.is_some_and(|p| p.is_file()) {
                    return Err(s3s::s3_error!(PreconditionFailed, "the key exists"));
                }
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

/// How the [`TestServer::start_objects`] server sends an object's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// All of it, as fast as possible.
    None,
    /// Waits this long before every chunk.
    Slow(Duration),
    /// Sends this many bytes, then nothing more (the connection stays open).
    StallAfter(usize),
    /// Sends this many bytes, then breaks the connection.
    CutAfter(usize),
}

/// One object of a [`TestServer::start_objects`] bucket.
#[derive(Clone)]
pub struct TestObject {
    pub key: String,
    pub data: Arc<Vec<u8>>,
    /// Sent as `x-amz-meta-sotyware-sha256`, as if sotyware had uploaded it.
    pub sha256: Option<String>,
    /// Whether the response has a `Content-Length`.
    pub announce_length: bool,
    pub fault: Fault,
}

/// `size` bytes of a deterministic, non-repeating pattern.
pub fn pattern(size: usize, seed: u8) -> Vec<u8> {
    let mut x = u32::from(seed).wrapping_mul(2_654_435_761) | 1;
    (0..size)
        .map(|_| {
            // xorshift: cheap, and different for every byte position.
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl TestObject {
    /// An object as sotyware uploads it: with its SHA-256 and a length.
    pub fn new(key: &str, data: Vec<u8>) -> Self {
        Self {
            key: key.to_owned(),
            sha256: Some(sha256_hex(&data)),
            data: Arc::new(data),
            announce_length: true,
            fault: Fault::None,
        }
    }

    /// An object uploaded by something else: no SHA-256 metadata.
    pub fn foreign(key: &str, data: Vec<u8>) -> Self {
        Self {
            sha256: None,
            ..Self::new(key, data)
        }
    }

    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = fault;
        self
    }
}

/// Body chunk size of the object server.
const BODY_CHUNK: usize = 64 * 1024;

/// An object body as a stream that can be slow, stall or break.
struct Body {
    data: Arc<Vec<u8>>,
    pos: usize,
    fault: Fault,
    delay: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl futures_core::Stream for Body {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        let this = &mut *self;
        if this.pos >= this.data.len() {
            return Poll::Ready(None);
        }
        let mut end = (this.pos + BODY_CHUNK).min(this.data.len());
        match this.fault {
            Fault::None => {}
            Fault::Slow(d) => {
                let sleep = this
                    .delay
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(d)));
                if sleep.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                this.delay = None;
            }
            Fault::StallAfter(n) | Fault::CutAfter(n) if this.pos >= n => {
                return if matches!(this.fault, Fault::CutAfter(_)) {
                    Poll::Ready(Some(Err(std::io::Error::other(
                        "connection cut by the test",
                    ))))
                } else {
                    Poll::Pending // never woken: the body stalls
                };
            }
            Fault::StallAfter(n) | Fault::CutAfter(n) => end = end.min(n),
        }
        let chunk = bytes::Bytes::copy_from_slice(&this.data[this.pos..end]);
        this.pos = end;
        Poll::Ready(Some(Ok(chunk)))
    }
}

/// The buckets of [`TestServer::start_objects`]: the objects, and the same
/// keys as a [`Listing`] so the browser can list them.
#[derive(Default)]
struct Objects {
    buckets: HashMap<String, HashMap<String, TestObject>>,
    listing: Listing,
}

impl s3s::S3 for Objects {
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
        let result = self.listing.list(req.input).map(s3s::S3Response::new);
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

    fn get_object<'life0, 'async_trait>(
        &'life0 self,
        req: s3s::S3Request<GetObjectInput>,
    ) -> Pin<
        Box<
            dyn Future<Output = s3s::S3Result<s3s::S3Response<GetObjectOutput>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let result = (|| {
            let bucket = self
                .buckets
                .get(&req.input.bucket)
                .ok_or_else(|| s3s::s3_error!(NoSuchBucket))?;
            let object = bucket
                .get(&req.input.key)
                .ok_or_else(|| s3s::s3_error!(NoSuchKey))?;
            let body = Body {
                data: Arc::clone(&object.data),
                pos: 0,
                fault: object.fault,
                delay: None,
            };
            Ok(s3s::S3Response::new(GetObjectOutput {
                body: Some(s3s::dto::StreamingBlob::wrap(body)),
                content_length: object
                    .announce_length
                    .then(|| i64::try_from(object.data.len()).unwrap_or(i64::MAX)),
                metadata: object.sha256.as_ref().map(|sha| {
                    HashMap::from([(crate::cloud::s3::SHA256_METADATA.to_owned(), sha.clone())])
                }),
                ..Default::default()
            }))
        })();
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

    /// A server that sends and lists objects (`GetObject`, `ListObjectsV2`,
    /// `HeadBucket`): each bucket holds exactly the given objects, keys kept
    /// as they are (s3s-fs keeps objects as files, so it cannot hold keys
    /// such as `a//b` or `..`), with the given body behaviour. Every other
    /// operation is refused.
    pub async fn start_objects(
        access_key_id: &str,
        secret: &str,
        objects: &[(&str, TestObject)],
    ) -> Self {
        let mut backend = Objects::default();
        for (bucket, object) in objects {
            backend
                .buckets
                .entry((*bucket).to_owned())
                .or_default()
                .insert(object.key.clone(), object.clone());
            backend
                .listing
                .buckets
                .entry((*bucket).to_owned())
                .or_default()
                .insert(object.key.clone(), object.data.len() as u64);
        }
        Self::serve(backend, fresh_root(), access_key_id, secret, &[]).await
    }

    /// A server on the in-memory [`Versioned`] backend: `buckets` are
    /// `(name, versioning enabled)`. The returned handle reads and changes
    /// the stored versions directly (as another client would).
    pub async fn start_versioned(
        access_key_id: &str,
        secret: &str,
        buckets: &[(&str, bool)],
    ) -> (Self, Versioned) {
        let backend = Versioned::new(buckets);
        let server = Self::serve(backend.clone(), fresh_root(), access_key_id, secret, &[]).await;
        (server, backend)
    }

    async fn serve(
        backend: impl s3s::S3,
        root: PathBuf,
        access_key_id: &str,
        secret: &str,
        denied: &[&'static str],
    ) -> Self {
        let mut builder = S3ServiceBuilder::new(backend);
        let mut auth = SimpleAuth::from_single(access_key_id, secret);
        auth.register(HELPER_KEY_ID.to_owned(), HELPER_SECRET.into());
        builder.set_auth(auth);
        let operations = Arc::new(Mutex::new(Vec::new()));
        let keyed = Arc::new(Mutex::new(Vec::new()));
        let injections = Arc::new(Mutex::new(Vec::new()));
        let conditional = Arc::new(Mutex::new(Vec::new()));
        builder.set_access(Access {
            denied: denied.to_vec(),
            operations: Arc::clone(&operations),
            keyed: Arc::clone(&keyed),
            injections: Arc::clone(&injections),
            root: root.clone(),
            conditional: Arc::clone(&conditional),
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
            keyed,
            injections,
            conditional,
            root,
            task,
        }
    }

    /// The operations that carried `If-None-Match: *` so far.
    pub fn conditional_ops(&self) -> Vec<String> {
        self.conditional.lock().unwrap().clone()
    }

    /// Applies `action` to the next `times` requests of operation `op`
    /// (`PutObject`, `UploadPart`, …), for `UploadPart` only those of part
    /// `part` if given. Rules are checked in the order they were added.
    pub fn inject(&self, op: &'static str, part: Option<u32>, times: usize, action: Inject) {
        self.injections.lock().unwrap().push(Injection {
            op,
            part,
            times,
            action,
        });
    }

    /// The operations requested so far.
    pub fn ops(&self) -> Vec<String> {
        self.operations.lock().unwrap().clone()
    }

    /// The operations signed with `access_key_id`.
    pub fn ops_of(&self, access_key_id: &str) -> Vec<String> {
        self.keyed
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k == access_key_id)
            .map(|(_, op)| op.clone())
            .collect()
    }

    /// The second key every server accepts, for a desktop test helper's own
    /// requests (they are then never counted as the app's).
    pub fn helper_credentials(&self) -> Credentials {
        Credentials {
            access_key_id: HELPER_KEY_ID.into(),
            secret_access_key: Secret::new(HELPER_SECRET.to_owned()),
        }
    }

    /// Files `s3s-fs` keeps for multipart uploads that were neither
    /// completed nor aborted (upload ids, parts, their metadata).
    pub fn open_multipart_files(&self) -> Vec<String> {
        fs::read_dir(&self.root)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.contains(".upload"))
            .collect()
    }

    /// The file `s3s-fs` keeps an object's metadata in.
    pub fn metadata_path(&self, bucket: &str, key: &str) -> PathBuf {
        fn b64(s: &str) -> String {
            const A: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let b = s.as_bytes();
            let mut out = String::new();
            for chunk in b.chunks(3) {
                let n = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |n, (i, &c)| n | u32::from(c) << (16 - 8 * i));
                for i in 0..=chunk.len() {
                    out.push(char::from(A[(n >> (18 - 6 * i) & 63) as usize]));
                }
            }
            out
        }
        self.root.join(format!(
            ".bucket-{}.object-{}.metadata.json",
            b64(bucket),
            b64(key)
        ))
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

/// A provider profile for tests only: the generic S3-compatible one with
/// conditional writes established, so the `If-None-Match` path of uploads
/// runs against this server (which enforces it like Amazon documents).
/// Says nothing about any real provider.
pub static CONDITIONAL_PROFILE: crate::cloud::provider::ProviderProfile = {
    use crate::cloud::provider::{Evidence, S3_COMPATIBLE, Support};
    let mut capabilities = S3_COMPATIBLE.capabilities;
    capabilities.conditional_put = Support::Yes(Evidence::Documented);
    crate::cloud::provider::ProviderProfile {
        id: "test-conditional",
        name: "Test service with conditional writes",
        endpoint: S3_COMPATIBLE.endpoint,
        addressing: S3_COMPATIBLE.addressing,
        sdk_checksums: S3_COMPATIBLE.sdk_checksums,
        limits: S3_COMPATIBLE.limits,
        capabilities,
        known_regions: &[],
        documentation: S3_COMPATIBLE.documentation,
    }
};

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// How a [`ThrottledLink`] forwards the bytes a client sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// At most this many bytes per second.
    BytesPerSecond(u64),
    /// This many bytes per connection, then nothing more (the connection
    /// stays open), as on a link that stopped carrying data.
    StallAfter(usize),
    /// Takes whatever the client sends at once (the client's body is
    /// handed over immediately) and forwards it at most this many bytes per
    /// second, also after the client closed the connection: like a proxy
    /// or load balancer that buffers request bodies, or a send buffer that
    /// still drains after a cancelled request.
    Buffered(u64),
}

/// A TCP relay in front of a test server whose client-to-server direction
/// is slow or stalls, so an upload meets real backpressure: the client's
/// socket buffers fill and its HTTP stack stops taking body bytes. The
/// server-to-client direction is not slowed. Its own receive buffer is
/// small, so little can pile up in front of the throttle.
pub struct ThrottledLink {
    pub endpoint: String,
    task: JoinHandle<()>,
    stop: Arc<tokio::sync::Notify>,
}

impl ThrottledLink {
    pub async fn start(upstream: &str, pace: Pace) -> Self {
        let upstream = upstream
            .strip_prefix("http://")
            .expect("an http endpoint")
            .to_owned();
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(64 * 1024).unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(16).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(tokio::sync::Notify::new());
        let stop2 = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    continue;
                };
                let upstream = upstream.clone();
                let stop = Arc::clone(&stop2);
                tokio::spawn(async move {
                    let stopped = stop.notified();
                    tokio::pin!(stopped);
                    tokio::select! {
                        () = relay(client, &upstream, pace) => {}
                        () = &mut stopped => {}
                    }
                });
            }
        });
        Self {
            endpoint: format!("http://{addr}"),
            task,
            stop,
        }
    }
}

async fn relay(client: tokio::net::TcpStream, upstream: &str, pace: Pace) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(server) = tokio::net::TcpStream::connect(upstream).await else {
        return;
    };
    let (mut client_rd, mut client_wr) = client.into_split();
    let (mut server_rd, mut server_wr) = server.into_split();
    let back = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut server_rd, &mut client_wr).await;
    });
    if let Pace::Buffered(rate) = pace {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match client_rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        // Paced by deadline, not by one sleep per piece: Windows timers
        // fire only every ~15.6 ms, so a 2.6 ms sleep per 16 KiB piece
        // would cap the link near 1 MiB/s whatever the rate. A late wake-up
        // is caught up by the next pieces; after an idle gap (more than
        // 100 ms behind) the schedule restarts instead of bursting.
        let mut due = tokio::time::Instant::now();
        while let Some(chunk) = rx.recv().await {
            for piece in chunk.chunks(16 * 1024) {
                if server_wr.write_all(piece).await.is_err() {
                    reader.abort();
                    let _ = back.await;
                    return;
                }
                let now = tokio::time::Instant::now();
                if now > due + Duration::from_millis(100) {
                    due = now;
                }
                due += Duration::from_secs_f64(piece.len() as f64 / rate as f64);
                tokio::time::sleep_until(due).await;
            }
        }
        // No half-close towards the server: like a proxy that buffers
        // bodies, the request it forwarded is finished independently of
        // its client. hyper's server (half-close off) takes an EOF as the
        // connection closing and drops a handler still running, so a part
        // whose client went away would be cut short when the server is
        // slow (debug build on a loaded machine) instead of being stored.
        // The connection ends once the answer cannot reach the client, at
        // the latest after a minute (an idle connection the client closed
        // gets no answer), or when the link is dropped.
        let mut back = back;
        if tokio::time::timeout(Duration::from_secs(60), &mut back)
            .await
            .is_err()
        {
            back.abort();
        }
        drop(server_wr);
        return;
    }
    let mut buf = vec![0u8; 16 * 1024];
    let mut total = 0usize;
    loop {
        let max = match pace {
            Pace::StallAfter(n) if total >= n => {
                // Stop reading: the client's buffers fill up.
                std::future::pending::<()>().await;
                0
            }
            Pace::StallAfter(n) => buf.len().min(n - total),
            Pace::BytesPerSecond(_) | Pace::Buffered(_) => buf.len(),
        };
        let n = match client_rd.read(&mut buf[..max]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if server_wr.write_all(&buf[..n]).await.is_err() {
            break;
        }
        total += n;
        if let Pace::BytesPerSecond(rate) = pace {
            tokio::time::sleep(Duration::from_secs_f64(n as f64 / rate as f64)).await;
        }
    }
    let _ = server_wr.shutdown().await;
    let _ = back.await;
}

impl Drop for ThrottledLink {
    fn drop(&mut self) {
        self.task.abort();
        self.stop.notify_waiters();
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

/// What the Cloud Storage UI test downloads (tests/desktop/cloud_ui.py and
/// windows-cloud-ui.ps1, phase `download`): a small and a 3 MiB file with
/// their SHA-256, odd names, a file without a stored SHA-256, a slow 16 MiB
/// file (time to cancel), one whose connection breaks and one whose content
/// does not match its SHA-256.
pub fn ui_download_objects() -> Vec<TestObject> {
    let mut tampered = TestObject::new("broken/tampered.bin", pattern(100 * 1024, 33));
    tampered.sha256 = Some(sha256_hex(b"not this content"));
    vec![
        TestObject::new("readme.txt", b"hello, cloud".to_vec()),
        TestObject::new("docs/Bericht 2024 ü & co.pdf", pattern(3 * 1024 * 1024, 31)),
        TestObject::foreign("foreign.txt", b"uploaded by another tool".to_vec()),
        TestObject::new("slow/large.bin", pattern(16 * 1024 * 1024, 32))
            .with_fault(Fault::Slow(Duration::from_millis(40))),
        TestObject::new("broken/cut.bin", pattern(1024 * 1024, 34))
            .with_fault(Fault::CutAfter(256 * 1024)),
        tampered,
    ]
}

pub const UI_DOWNLOAD_BUCKET: &str = "download-bucket";

/// Not a test: serves [`ui_download_objects`] for the Cloud Storage UI test
/// until the file named by `SOTYWARE_UI_SERVER_FILE` (where it writes its
/// endpoint; the objects' sizes and SHA-256 go to the same name plus
/// `.json`) is deleted, then fails if anything but listing, the connection
/// check and `GetObject` was requested. Started by tests/desktop/cloud-ui.sh
/// (phase `download`) and windows-cloud-ui.ps1 (`-Mode download`); the key
/// and secret are made up.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the Cloud Storage UI test, not a test"]
async fn serve_objects_for_ui() {
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let objects = ui_download_objects();
    let manifest: Vec<String> = objects
        .iter()
        .map(|o| {
            format!(
                "{{\"key\":{:?},\"size\":{},\"sha256\":{:?}}}",
                o.key,
                o.data.len(),
                sha256_hex(&o.data)
            )
        })
        .collect();
    fs::write(format!("{file}.json"), format!("[{}]", manifest.join(","))).unwrap();
    let pairs: Vec<(&str, TestObject)> = objects
        .into_iter()
        .map(|o| (UI_DOWNLOAD_BUCKET, o))
        .collect();
    let server = TestServer::start_objects(
        "e2eDownloadKey00000001",
        "e2e-Download-Secret-0123456789",
        &pairs,
    )
    .await;
    fs::write(&file, &server.endpoint).unwrap();
    println!("serving {} at {}", UI_DOWNLOAD_BUCKET, server.endpoint);
    while Path::new(&file).exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ops = server.operations.lock().unwrap().clone();
    let gets = ops.iter().filter(|op| *op == "GetObject").count();
    println!(
        "operations: {} GetObject, {} ListObjectsV2, others: {:?}",
        gets,
        ops.iter().filter(|op| *op == "ListObjectsV2").count(),
        ops.iter()
            .filter(|op| !matches!(op.as_str(), "GetObject" | "ListObjectsV2"))
            .collect::<Vec<_>>()
    );
    assert!(
        ops.iter()
            .all(|op| matches!(op.as_str(), "GetObject" | "ListObjectsV2" | "HeadBucket")),
        "downloading must only read: {ops:?}"
    );
    compare_recordings_if_asked(&server, &server.access_key_id.clone());
}

pub const UI_UPLOAD_BUCKET: &str = "upload-bucket";

/// What the upload phase of the Cloud Storage UI test finds in the bucket
/// before it uploads (tests/desktop/cloud_ui.py, phase `upload`): an object
/// the batch must skip, a folder to browse, and a file to download while an
/// upload runs. Stored through `S3Store::upload`, so with the SHA-256
/// metadata sotyware writes.
pub fn ui_upload_seed() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("inbox/existing.txt", b"already in the bucket".to_vec()),
        ("other/readme.txt", b"another folder".to_vec()),
        ("downloads/big.bin", pattern(24 * 1024 * 1024, 41)),
    ]
}

/// The objects in the s3s-fs bucket folder as the test sees them: content
/// on disk, and size and SHA-256 metadata through `HeadObject`; plus the
/// multipart files s3s-fs keeps for uploads neither completed nor aborted.
async fn ui_upload_state(server: &TestServer, store: &super::s3::S3Store) -> serde_json::Value {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                // Object keys use `/`, also on Windows (where the path
                // separator would be `\`).
                let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
                out.push(parts.join("/"));
            }
        }
    }
    let base = server.root().join(UI_UPLOAD_BUCKET);
    let mut keys = Vec::new();
    walk(&base, &base, &mut keys);
    keys.sort();
    let mut objects = Vec::new();
    for key in keys {
        let content = fs::read(base.join(&key)).unwrap_or_default();
        let head = store.head_object(UI_UPLOAD_BUCKET, &key).await;
        let (head_size, meta) = match &head {
            Ok(Some(d)) => (Some(d.size), d.sha256.clone()),
            _ => (None, None),
        };
        objects.push(serde_json::json!({
            "key": key,
            "size": content.len(),
            "sha256": sha256_hex(&content),
            "headSize": head_size,
            "sha256Metadata": meta,
            "head": format!("{:?}", head.as_ref().map(|h| h.is_some())),
        }));
    }
    let other: Vec<String> = fs::read_dir(server.root())
        .map(|r| {
            r.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n != UI_UPLOAD_BUCKET)
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "objects": objects,
        "multipart": server.open_multipart_files(),
        "otherFiles": other,
        "ops": server.ops(),
    })
}

/// Not a test: serves an empty-but-for-[`ui_upload_seed`] s3s-fs bucket for
/// the upload phase of the Cloud Storage UI test, behind a [`ThrottledLink`]
/// (`SOTYWARE_UI_UPLOAD_RATE` bytes per second towards the server, default
/// 6 MiB/s), so that cancelling during an upload and a download beside it
/// can be seen. Writes the link's endpoint to `SOTYWARE_UI_SERVER_FILE`
/// and the seed objects' sizes and SHA-256 to that name plus `.json`.
///
/// With `SOTYWARE_UI_UPLOAD_PACE=buffered` the link is a
/// [`Pace::Buffered`] one instead: it takes each request body at once, so
/// every part is handed over completely while it is still being forwarded
/// (the case ADR-025 waits for; `windows-cloud-ui.ps1 -Mode b2-lifecycle
/// -Rehearse -Scenario settle`).
///
/// Until the endpoint file is deleted it reads commands from that name
/// plus `.ctl` (one per line), deletes the file when done, and answers
/// `dump` in `.state.json`:
///
/// - `dump`: the bucket's objects, open multipart files and operations;
/// - `verify-denied`: the next `HeadObject` after the next `PutObject` is
///   refused (the upload's verification, after its commit point);
/// - `abort-denied`: the next `AbortMultipartUpload` is refused;
/// - `delete <key>`: deletes exactly that object (the test's cleanup).
///
/// With `SOTYWARE_UI_UPLOAD_SEED_KEY` and `SOTYWARE_UI_UPLOAD_SEED_FILE`
/// set, the bucket is seeded with that one file under that key instead
/// (the local rehearsal of `windows-cloud-ui.ps1 -Mode b2-upload`, which
/// lays the bucket out like its Backblaze run).
///
/// At the end it fails if the app (before the first `delete`) sent anything
/// but listing, the connection check, reads and upload requests. Started by
/// tests/desktop/cloud-ui.sh (phase `upload`); the key is made up.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "helper for the Cloud Storage UI test, not a test"]
async fn serve_uploads_for_ui() {
    use super::s3::{ClientOptions, S3Store, TransferControl};
    let file = std::env::var("SOTYWARE_UI_SERVER_FILE").expect("set SOTYWARE_UI_SERVER_FILE");
    let rate: u64 = std::env::var("SOTYWARE_UI_UPLOAD_RATE")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(6 * 1024 * 1024);
    let server = TestServer::start(
        "e2eUploadKey0000000001",
        "e2e-Upload-Secret-0123456789",
        &[UI_UPLOAD_BUCKET],
    )
    .await;
    // The helper's own requests (seeding, `dump`, `delete`) are signed with
    // the helper key, so they are never counted as the app's.
    let store = S3Store::new(
        server.resolved("helper", &[UI_UPLOAD_BUCKET]),
        server.helper_credentials(),
        ClientOptions::default(),
    );
    let dir = TempDir::new("ui-upload-seed");
    let mut manifest = Vec::new();
    let seed: Vec<(String, Vec<u8>)> = match (
        std::env::var("SOTYWARE_UI_UPLOAD_SEED_KEY"),
        std::env::var("SOTYWARE_UI_UPLOAD_SEED_FILE"),
    ) {
        (Ok(key), Ok(path)) => vec![(key, fs::read(&path).expect("the seed file"))],
        _ => ui_upload_seed()
            .into_iter()
            .map(|(k, d)| (k.to_owned(), d))
            .collect(),
    };
    for (i, (key, data)) in seed.into_iter().enumerate() {
        let path = dir.path(&format!("seed-{i}"));
        fs::write(&path, &data).unwrap();
        store
            .upload(
                UI_UPLOAD_BUCKET,
                &key,
                &path,
                &Arc::new(TransferControl::new()),
                &mut |_| {},
            )
            .await
            .unwrap();
        manifest
            .push(serde_json::json!({"key": key, "size": data.len(), "sha256": sha256_hex(&data)}));
    }
    let seeded = server.ops().len();
    fs::write(
        format!("{file}.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    let pace = match std::env::var("SOTYWARE_UI_UPLOAD_PACE").as_deref() {
        Ok("buffered") => Pace::Buffered(rate),
        Ok(other) => panic!("SOTYWARE_UI_UPLOAD_PACE: unknown pace {other:?}"),
        Err(_) => Pace::BytesPerSecond(rate),
    };
    let link = ThrottledLink::start(&server.endpoint, pace).await;
    fs::write(&file, &link.endpoint).unwrap();
    println!(
        "serving {} at {} ({} bytes/s towards the server, {pace:?}), seeded {} objects",
        UI_UPLOAD_BUCKET,
        link.endpoint,
        rate,
        manifest.len()
    );
    let ctl = PathBuf::from(format!("{file}.ctl"));
    let mut app_ops: Option<Vec<String>> = None;
    // `dump` sends one HeadObject per object; they are the test's, not the app's.
    let mut own_heads = 0usize;
    // A test helper, so a plain wait loop is fine here.
    while Path::new(&file).exists() {
        if let Ok(commands) = fs::read_to_string(&ctl) {
            for line in commands.lines() {
                match line.split_once(' ').unwrap_or((line, "")) {
                    ("dump", _) => {
                        let state = ui_upload_state(&server, &store).await;
                        if app_ops.is_none() {
                            own_heads += state["objects"].as_array().map_or(0, Vec::len);
                        }
                        let tmp = format!("{file}.state.json.tmp");
                        fs::write(&tmp, serde_json::to_string_pretty(&state).unwrap()).unwrap();
                        fs::rename(&tmp, format!("{file}.state.json")).unwrap();
                    }
                    ("verify-denied", _) => {
                        let injections = Arc::clone(&server.injections);
                        server.inject(
                            "PutObject",
                            None,
                            1,
                            Inject::Run(Arc::new(move || {
                                injections.lock().unwrap().push(Injection {
                                    op: "HeadObject",
                                    part: None,
                                    times: 1,
                                    action: Inject::Deny,
                                });
                            })),
                        );
                    }
                    ("abort-denied", _) => {
                        server.inject("AbortMultipartUpload", None, 1, Inject::Deny);
                    }
                    ("delete", key) if !key.is_empty() => {
                        if app_ops.is_none() {
                            app_ops = Some(server.ops()[seeded..].to_vec());
                        }
                        let r = store.delete_object(UI_UPLOAD_BUCKET, key, None).await;
                        println!("cleanup: deleted {key:?}: {r:?}");
                    }
                    (other, _) => println!("unknown command {other:?}"),
                }
            }
            let _ = fs::remove_file(&ctl);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ops = app_ops.unwrap_or_else(|| server.ops()[seeded..].to_vec());
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for op in &ops {
        let name = op.split('#').next().unwrap_or(op).to_owned();
        *counts.entry(name).or_default() += 1;
    }
    if let Some(heads) = counts.get_mut("HeadObject") {
        *heads -= own_heads.min(*heads);
    }
    counts.retain(|_, n| *n > 0);
    println!("operations: {counts:?} (the test's own {own_heads} HeadObject left out)");
    let left = ui_upload_state(&server, &store).await;
    println!(
        "left at the end: {} objects, multipart files {}",
        left["objects"].as_array().map_or(0, Vec::len),
        left["multipart"]
    );
    drop(link);
    compare_recordings_if_asked(&server, "e2eUploadKey0000000001");
    assert!(
        counts.keys().all(|op| matches!(
            op.as_str(),
            "HeadBucket"
                | "ListObjectsV2"
                | "HeadObject"
                | "GetObject"
                | "PutObject"
                | "CreateMultipartUpload"
                | "UploadPart"
                | "CompleteMultipartUpload"
                | "AbortMultipartUpload"
        )),
        "the app must only list, read and upload: {counts:?}"
    );
}

/// With `SOTYWARE_UI_RECORDING_DIR` set (a desktop test run with a
/// request-recorder build, ADR-024): compares the app's recordings in that
/// folder with the requests this server saw signed with the app's key, and
/// fails unless they agree ([`check_recordings`]).
fn compare_recordings_if_asked(server: &TestServer, app_key: &str) {
    let Ok(dir) = std::env::var("SOTYWARE_UI_RECORDING_DIR") else {
        return;
    };
    let app_ops = server.ops_of(app_key);
    let helper_ops = server.ops_of(HELPER_KEY_ID).len();
    match check_recordings(
        Path::new(&dir),
        &app_ops,
        &[app_key, &server.secret, HELPER_KEY_ID, HELPER_SECRET],
    ) {
        Ok(report) => {
            for line in report {
                println!("recorder: {line}");
            }
            println!(
                "recorder: the helper's own {helper_ops} requests (helper key) are not in the recording"
            );
        }
        Err(e) => panic!("recorder: MISMATCH: {e}"),
    }
}

/// Checks the request recordings (`*.requests.jsonl`) in `dir` against the
/// requests a server saw from the app (`server_ops`, as `UploadPart#2`):
/// every file is complete (start, end with `accounting: complete`, every
/// line JSON, sequence numbers without gaps), every attempt that got an
/// answer is on the server, the server saw nothing the recorder did not
/// see being sent, and none of `secrets` appears. Returns report lines.
pub fn check_recordings(
    dir: &Path,
    server_ops: &[String],
    secrets: &[&str],
) -> Result<Vec<String>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".requests.jsonl"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("no recording: the count of the app's requests is unknown".into());
    }
    let mut sent: Vec<String> = Vec::new();
    let mut answered: Vec<String> = Vec::new();
    let mut ops = 0usize;
    let mut dropped = 0usize;
    let mut report = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        for secret in secrets {
            if text.contains(secret) {
                return Err(format!("{} contains a credential", file.display()));
            }
        }
        if !text.ends_with('\n') {
            return Err(format!(
                "{}: partial last line (incomplete)",
                file.display()
            ));
        }
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}: not JSON Lines: {e}", file.display()))?;
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            return Err(format!("{}: empty", file.display()));
        };
        if first["kind"] != "start" {
            return Err(format!("{}: no start line", file.display()));
        }
        if last["kind"] != "end" || last["accounting"] != "complete" {
            return Err(format!(
                "{}: incomplete recording (count unknown): {last}",
                file.display()
            ));
        }
        let mut parts: HashMap<u64, u64> = HashMap::new();
        let mut started: Vec<(u64, u64, String)> = Vec::new();
        let mut expected = 0u64;
        for e in &lines[1..lines.len() - 1] {
            if e["seq"].as_u64() != Some(expected) {
                return Err(format!("{}: sequence gap at {e}", file.display()));
            }
            expected += 1;
            let op = e["op"].as_u64().unwrap_or(u64::MAX);
            let name = e["name"].as_str().unwrap_or("?").to_owned();
            match e["kind"].as_str().unwrap_or("") {
                "op-start" => {
                    ops += 1;
                    if let Some(n) = e["part"].as_u64() {
                        parts.insert(op, n);
                    }
                }
                "op-dropped" => dropped += 1,
                "attempt-start" => {
                    let label = match parts.get(&op) {
                        Some(n) => format!("{name}#{n}"),
                        None => name,
                    };
                    started.push((op, e["attempt"].as_u64().unwrap_or(0), label.clone()));
                    sent.push(label);
                }
                "attempt-end" if e.get("status").is_some() => {
                    let attempt = e["attempt"].as_u64().unwrap_or(0);
                    if let Some((_, _, label)) =
                        started.iter().find(|(o, a, _)| *o == op && *a == attempt)
                    {
                        answered.push(label.clone());
                    }
                }
                _ => {}
            }
        }
        report.push(format!(
            "{}: complete, {} events",
            file.file_name().unwrap_or_default().to_string_lossy(),
            expected
        ));
    }
    let mut server = server_ops.to_vec();
    for a in &answered {
        let i = server.iter().position(|s| s == a).ok_or_else(|| {
            format!("{a} was answered but is not among the server's app requests")
        })?;
        server.remove(i);
    }
    let mut unanswered = sent.clone();
    for a in &answered {
        if let Some(i) = unanswered.iter().position(|s| s == a) {
            unanswered.remove(i);
        }
    }
    for s in &server {
        let i = unanswered.iter().position(|u| u == s).ok_or_else(|| {
            format!("the server saw {s} from the app, which the recorder did not see sent")
        })?;
        unanswered.remove(i);
    }
    let count = |list: &[String]| {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for op in list {
            *m.entry(op.split('#').next().unwrap_or(op).to_owned())
                .or_default() += 1;
        }
        m
    };
    report.push(format!(
        "{} file(s), {ops} operations ({dropped} dropped), {} attempts sent, {} answered; server: {} app requests",
        files.len(),
        sent.len(),
        answered.len(),
        server_ops.len()
    ));
    report.push(format!("sent by operation: {:?}", count(&sent)));
    report.push(format!("server by operation: {:?}", count(server_ops)));
    report.push(
        "every answered attempt reached the server; the server saw nothing the recorder did not see sent"
            .into(),
    );
    Ok(report)
}

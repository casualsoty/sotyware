//! The one storage adapter: any S3-compatible provider through the official
//! AWS SDK for Rust (`aws-sdk-s3`), configured from a [`ResolvedAccount`].
//!
//! Integrity is checked end to end, independently of the provider: before an
//! upload the file's SHA-256 is computed and stored with the object as user
//! metadata (`x-amz-meta-sotyware-sha256`); a download is written to a
//! temporary file, hashed while it streams, and moved into place only if the
//! hash and the size match. Nothing relies on ETags (an ETag is not an MD5 for
//! multipart uploads and differs between providers).
//!
//! Each [`S3Store`] belongs to one account and has its own HTTP connection
//! pool and its own transfer queue (a semaphore of
//! `max_concurrent_transfers` permits), so a slow or failing account never
//! holds up another one.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use aws_sdk_s3::Client;
use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials as SdkCredentials, Region, RequestChecksumCalculation,
    ResponseChecksumValidation,
};
use aws_sdk_s3::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_smithy_http_client::tls;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use super::account::ResolvedAccount;
use super::credentials::Credentials;
use super::provider::{SdkChecksums, TransferLimits};

/// User metadata key holding the hex SHA-256 of the object's content.
pub const SHA256_METADATA: &str = "sotyware-sha256";

/// Suffix of the temporary file a download is written to.
const PARTIAL_SUFFIX: &str = ".sotyware-partial";

const MIB: u64 = 1024 * 1024;

/// Longest object key S3 allows, in bytes.
const MAX_KEY_BYTES: usize = 1024;

/// Client settings that are not part of the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientOptions {
    pub connect_timeout: Duration,
    /// Longest wait for the next bytes of a response.
    pub read_timeout: Duration,
    /// Attempts per request, including the first (the SDK's standard retry
    /// with exponential backoff; only transient errors are retried).
    pub max_attempts: u32,
    /// Files up to this size are sent with one PUT.
    pub multipart_threshold: u64,
    /// Preferred part size; raised when a file would need too many parts.
    pub part_size: u64,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(60),
            max_attempts: 3,
            multipart_threshold: 64 * MIB,
            part_size: 16 * MIB,
        }
    }
}

/// Why a storage operation failed, sorted by what the user (or a retry) can do
/// about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The key id is unknown or the secret is wrong.
    InvalidCredentials(String),
    /// The credentials are valid but not allowed to do this.
    AccessDenied(String),
    /// No such bucket or object.
    NotFound(String),
    /// The provider could not be reached, or the connection broke.
    Network(String),
    /// The provider did not answer in time.
    Timeout(String),
    /// Downloaded or uploaded content does not match its checksum or size.
    Integrity(String),
    /// The local file changed while it was being uploaded.
    FileChanged(String),
    /// The file is larger than the provider accepts.
    TooLarge { size: u64, max: u64 },
    /// The bucket is not one of the account's configured buckets, or the key
    /// is not valid.
    Refused(String),
    /// A local file could not be read or written.
    Io(String),
    /// Any other error the provider returned.
    Provider {
        status: u16,
        code: String,
        message: String,
    },
}

impl StorageError {
    /// Whether trying again later may succeed without any change.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::Timeout(_) => true,
            Self::Provider { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentials(m) => write!(f, "invalid credentials: {m}"),
            Self::AccessDenied(m) => write!(f, "access denied: {m}"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::Timeout(m) => write!(f, "timed out: {m}"),
            Self::Integrity(m) => write!(f, "integrity check failed: {m}"),
            Self::FileChanged(m) => write!(f, "file changed during upload: {m}"),
            Self::TooLarge { size, max } => {
                write!(
                    f,
                    "file of {size} bytes exceeds the provider's limit of {max} bytes"
                )
            }
            Self::Refused(m) => write!(f, "refused: {m}"),
            Self::Io(m) => write!(f, "local file error: {m}"),
            Self::Provider {
                status,
                code,
                message,
            } => write!(f, "provider error {status} {code}: {message}"),
        }
    }
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> StorageError + '_ {
    move |e| StorageError::Io(format!("{}: {e}", path.display()))
}

/// Maps a provider's error response to a [`StorageError`]. Pure, so the
/// mapping is unit-tested without a server.
pub fn classify_response(status: u16, code: Option<&str>, message: &str, op: &str) -> StorageError {
    let what = |m: &str| {
        if m.is_empty() {
            op.to_owned()
        } else {
            format!("{op}: {m}")
        }
    };
    match code {
        // `NotSignedUp` is what s3s (the test server) and Amazon return for an
        // unknown key id. Which codes Backblaze and Hetzner use is checked by
        // the opt-in integration tests, not documented.
        Some(
            "InvalidAccessKeyId"
            | "SignatureDoesNotMatch"
            | "InvalidToken"
            | "ExpiredToken"
            | "InvalidSecurity"
            | "AuthorizationHeaderMalformed"
            | "NotSignedUp",
        ) => StorageError::InvalidCredentials(what(message)),
        Some("AccessDenied" | "AllAccessDisabled" | "AccountProblem") => {
            StorageError::AccessDenied(what(message))
        }
        Some("NoSuchBucket" | "NoSuchKey" | "NoSuchUpload" | "NotFound") => {
            StorageError::NotFound(what(message))
        }
        Some("RequestTimeout") => StorageError::Timeout(what(message)),
        // HEAD responses have no body, so no code: only the status is known.
        None if status == 404 => StorageError::NotFound(what(message)),
        None if status == 403 => StorageError::AccessDenied(what(message)),
        None if status == 401 => StorageError::InvalidCredentials(what(message)),
        _ => StorageError::Provider {
            status,
            code: code.unwrap_or("").to_owned(),
            message: what(message),
        },
    }
}

fn classify<E>(op: &str, err: SdkError<E, aws_sdk_s3::config::http::HttpResponse>) -> StorageError
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    match &err {
        SdkError::ServiceError(service) => {
            let status = service.raw().status().as_u16();
            let e = service.err();
            classify_response(status, e.code(), e.message().unwrap_or(""), op)
        }
        SdkError::TimeoutError(_) => StorageError::Timeout(op.to_owned()),
        SdkError::DispatchFailure(d) if d.is_timeout() => {
            StorageError::Timeout(format!("{op}: {}", DisplayErrorContext(&err)))
        }
        SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            StorageError::Network(format!("{op}: {}", DisplayErrorContext(&err)))
        }
        _ => StorageError::Provider {
            status: 0,
            code: String::new(),
            message: format!("{op}: {}", DisplayErrorContext(&err)),
        },
    }
}

/// How a file will be uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadPlan {
    Single,
    Multipart { part_size: u64, parts: u64 },
}

/// Chooses single PUT or multipart for a file of `size` bytes. A multipart
/// upload always has at least two parts (Backblaze requires that).
pub fn plan_upload(
    size: u64,
    limits: &TransferLimits,
    options: &ClientOptions,
) -> Result<UploadPlan, StorageError> {
    if size > limits.max_object_size {
        return Err(StorageError::TooLarge {
            size,
            max: limits.max_object_size,
        });
    }
    let single_max = options.multipart_threshold.min(limits.max_single_put);
    let fewest_bytes_per_part = size.div_ceil(limits.max_parts);
    let part_size = options
        .part_size
        .max(limits.min_part_size)
        .max(fewest_bytes_per_part)
        .div_ceil(MIB)
        .saturating_mul(MIB)
        .min(limits.max_part_size);
    let parts = size.div_ceil(part_size);
    if size <= single_max || parts < 2 {
        if size > limits.max_single_put {
            return Err(StorageError::TooLarge {
                size,
                max: limits.max_single_put,
            });
        }
        return Ok(UploadPlan::Single);
    }
    if parts > limits.max_parts {
        return Err(StorageError::TooLarge {
            size,
            max: limits.max_part_size.saturating_mul(limits.max_parts),
        });
    }
    Ok(UploadPlan::Multipart { part_size, parts })
}

/// An object as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub key: String,
    pub size: u64,
    /// RFC 3339, as the provider reports it.
    pub last_modified: Option<String>,
}

/// What separates "folders" in object keys. S3 has no folders: a folder is
/// a key prefix ending in this character.
pub const FOLDER_DELIMITER: &str = "/";

/// One page of a folder listing, as the provider returned it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderPage {
    /// The full prefixes of immediate subfolders (each ends in `/`).
    pub prefixes: Vec<String>,
    /// Objects directly in the folder, with their full keys. May include the
    /// folder's own marker object (key equal to the prefix).
    pub objects: Vec<ObjectInfo>,
    /// The provider's token for the next page, if there is one.
    pub next_token: Option<String>,
}

/// An object's details from a HEAD request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDetails {
    pub size: u64,
    /// The SHA-256 sotyware stored at upload, if it uploaded the object.
    pub sha256: Option<String>,
    pub version_id: Option<String>,
}

/// What an upload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadReport {
    pub size: u64,
    pub sha256: String,
    /// 1 for a single PUT.
    pub parts: u64,
    /// The version created, on a versioned bucket. Deleting this exact version
    /// removes the upload completely (a plain delete only hides it there).
    pub version_id: Option<String>,
}

/// How a download was verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    /// The content matches the SHA-256 stored at upload.
    Sha256Matched,
    /// The object has no stored SHA-256 (not uploaded by sotyware); only the
    /// size was checked.
    SizeOnly,
}

/// What a download did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadReport {
    pub size: u64,
    pub sha256: String,
    pub verification: Verification,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Size, modification time and SHA-256 of a local file.
async fn hash_file(path: &Path) -> Result<(u64, Option<SystemTime>, String), StorageError> {
    let mut file = tokio::fs::File::open(path).await.map_err(io_error(path))?;
    let meta = file.metadata().await.map_err(io_error(path))?;
    if !meta.is_file() {
        return Err(StorageError::Io(format!("{}: not a file", path.display())));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; MIB as usize];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).await.map_err(io_error(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    if size != meta.len() {
        return Err(StorageError::FileChanged(path.display().to_string()));
    }
    Ok((size, meta.modified().ok(), hex(&hasher.finalize())))
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

/// One account's connection to its provider.
pub struct S3Store {
    client: Client,
    account: ResolvedAccount,
    access_key_id: String,
    options: ClientOptions,
    transfers: Semaphore,
}

impl fmt::Debug for S3Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Store")
            .field("account", &self.account.id)
            .field("endpoint", &self.account.endpoint_url)
            .finish_non_exhaustive()
    }
}

impl S3Store {
    /// Builds the client. Sends no request.
    pub fn new(account: ResolvedAccount, credentials: Credentials, options: ClientOptions) -> Self {
        let checksums = account.profile.sdk_checksums;
        let (request_checksums, response_checksums) = match checksums {
            SdkChecksums::WhenSupported => (
                RequestChecksumCalculation::WhenSupported,
                ResponseChecksumValidation::WhenSupported,
            ),
            SdkChecksums::WhenRequired => (
                RequestChecksumCalculation::WhenRequired,
                ResponseChecksumValidation::WhenRequired,
            ),
        };
        // rustls with ring, and the operating system's root certificates.
        let http = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .http_client(http)
            .region(Region::new(account.region.clone()))
            .endpoint_url(&account.endpoint_url)
            .force_path_style(account.path_style)
            .credentials_provider(SdkCredentials::new(
                credentials.access_key_id.clone(),
                credentials.secret_access_key.expose(),
                None,
                None,
                "sotyware",
            ))
            .request_checksum_calculation(request_checksums)
            .response_checksum_validation(response_checksums)
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(options.connect_timeout)
                    .read_timeout(options.read_timeout)
                    .build(),
            )
            .retry_config(RetryConfig::standard().with_max_attempts(options.max_attempts))
            .build();
        Self {
            client: Client::from_conf(config),
            transfers: Semaphore::new(usize::from(account.max_concurrent_transfers)),
            access_key_id: credentials.access_key_id,
            account,
            options,
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account.id
    }

    /// The key id in use (not secret), so callers can show which key it is.
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// The account's transfer queue, for tests.
    #[cfg(test)]
    pub(crate) fn transfer_queue(&self) -> &Semaphore {
        &self.transfers
    }

    fn check_target(&self, bucket: &str, key: Option<&str>) -> Result<(), StorageError> {
        if !self.account.buckets.is_empty() && !self.account.buckets.iter().any(|b| b == bucket) {
            return Err(StorageError::Refused(format!(
                "bucket '{bucket}' is not configured for account '{}'",
                self.account.id
            )));
        }
        if let Some(key) = key
            && (key.is_empty() || key.len() > MAX_KEY_BYTES)
        {
            return Err(StorageError::Refused(format!(
                "object keys must be 1 to {MAX_KEY_BYTES} bytes"
            )));
        }
        Ok(())
    }

    async fn transfer_slot(&self) -> Result<tokio::sync::SemaphorePermit<'_>, StorageError> {
        // The semaphore is never closed.
        self.transfers
            .acquire()
            .await
            .map_err(|e| StorageError::Io(format!("transfer queue closed: {e}")))
    }

    /// The SDK client, so the opt-in integration tests can check what the
    /// provider really stored (object versions, delete markers) without the
    /// adapter offering those calls to the app.
    #[cfg(test)]
    pub(super) fn sdk_client(&self) -> &Client {
        &self.client
    }

    /// The same store with `interceptor` added to its client, so a test can
    /// record the HTTP requests really sent (method, path, status), also to
    /// a real provider, where the server side cannot be observed.
    #[cfg(test)]
    pub(super) fn with_interceptor(
        mut self,
        interceptor: impl aws_sdk_s3::config::Intercept + 'static,
    ) -> Self {
        let config = self
            .client
            .config()
            .to_builder()
            .interceptor(interceptor)
            .build();
        self.client = Client::from_conf(config);
        self
    }

    /// Every bucket the credentials can see. A key restricted to one bucket
    /// may be refused this (`AccessDenied`); use the configured buckets then.
    pub async fn list_buckets(&self) -> Result<Vec<String>, StorageError> {
        let out = self
            .client
            .list_buckets()
            .send()
            .await
            .map_err(|e| classify("list buckets", e))?;
        Ok(out
            .buckets()
            .iter()
            .filter_map(|b| b.name().map(str::to_owned))
            .collect())
    }

    /// Checks that the credentials are accepted and may use `bucket`, without
    /// changing anything and without listing the account's buckets (a key
    /// restricted to one bucket may be refused `ListBuckets` while being
    /// perfectly valid for its bucket).
    ///
    /// First `HeadBucket` on the bucket: success proves both. A HEAD response
    /// has no body, so a refusal there is only a status (403 is the same for
    /// a wrong secret and a missing permission). Only then one
    /// `ListObjectsV2` of at most one key in the same bucket is sent, whose
    /// error body carries a code that tells the two apart: success means the
    /// key may read the bucket, `AccessDenied` that the key is valid but not
    /// allowed, `InvalidAccessKeyId`/`SignatureDoesNotMatch` that it is wrong.
    pub async fn check_bucket(&self, bucket: &str) -> Result<(), StorageError> {
        self.check_target(bucket, None)?;
        let head = self
            .client
            .head_bucket()
            .bucket(bucket)
            .send()
            .await
            .map(drop)
            .map_err(|e| classify("check bucket", e));
        match head {
            Err(StorageError::AccessDenied(_))
            | Err(StorageError::Provider {
                status: 400..=499, ..
            }) => self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .max_keys(1)
                .send()
                .await
                .map(drop)
                .map_err(|e| classify("check bucket", e)),
            other => other,
        }
    }

    /// Every object under `prefix`, following continuation tokens.
    pub async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<Vec<ObjectInfo>, StorageError> {
        self.check_target(bucket, None)?;
        let mut objects = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let out = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .set_continuation_token(token.take())
                .send()
                .await
                .map_err(|e| classify("list objects", e))?;
            for o in out.contents() {
                if let Some(key) = o.key() {
                    objects.push(ObjectInfo {
                        key: key.to_owned(),
                        size: o.size().and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                        last_modified: o.last_modified().map(|t| t.to_string()),
                    });
                }
            }
            match out.next_continuation_token() {
                Some(next) if out.is_truncated() == Some(true) => token = Some(next.to_owned()),
                _ => break,
            }
        }
        Ok(objects)
    }

    /// One page of one folder level: the objects directly under `prefix` and
    /// the prefixes of its immediate subfolders (`ListObjectsV2` with the
    /// delimiter `/`), at most `max_keys` entries. Never recursive: nested
    /// objects appear only as their top folder's prefix.
    ///
    /// `continuation` is the token of the previous page, exactly as the
    /// provider returned it. The returned `next_token` is `None` when the
    /// provider says the listing is complete, and also when it claims more
    /// but gives no token, or gives back the token it was sent (which would
    /// repeat the same page forever).
    pub async fn list_folder(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: i32,
    ) -> Result<FolderPage, StorageError> {
        self.check_target(bucket, None)?;
        if prefix.len() > MAX_KEY_BYTES {
            return Err(StorageError::Refused(format!(
                "prefixes must be at most {MAX_KEY_BYTES} bytes"
            )));
        }
        let out = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .delimiter(FOLDER_DELIMITER)
            .max_keys(max_keys)
            .set_continuation_token(continuation.map(str::to_owned))
            .send()
            .await
            .map_err(|e| classify("list folder", e))?;
        let objects = out
            .contents()
            .iter()
            .filter_map(|o| {
                Some(ObjectInfo {
                    key: o.key()?.to_owned(),
                    size: o.size().and_then(|s| u64::try_from(s).ok()).unwrap_or(0),
                    last_modified: o
                        .last_modified()
                        .and_then(|t| t.fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime).ok()),
                })
            })
            .collect();
        let prefixes = out
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix().map(str::to_owned))
            .collect();
        let next_token = match out.next_continuation_token() {
            Some(next) if out.is_truncated() == Some(true) && Some(next) != continuation => {
                Some(next.to_owned())
            }
            _ => None,
        };
        Ok(FolderPage {
            prefixes,
            objects,
            next_token,
        })
    }

    /// The object's size and stored checksum, or `None` if it does not exist.
    pub async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectDetails>, StorageError> {
        self.check_target(bucket, Some(key))?;
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => Ok(Some(ObjectDetails {
                size: out
                    .content_length()
                    .and_then(|s| u64::try_from(s).ok())
                    .unwrap_or(0),
                sha256: out.metadata().and_then(|m| m.get(SHA256_METADATA)).cloned(),
                version_id: out.version_id().map(str::to_owned),
            })),
            Err(e) => match classify("head object", e) {
                StorageError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// Uploads `path` to `bucket/key` (single PUT or multipart, per
    /// [`plan_upload`]), storing its SHA-256 with the object. Waits for a slot
    /// in the account's transfer queue first. A failed multipart upload is
    /// aborted, so no orphaned parts stay billed.
    pub async fn upload_file(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
    ) -> Result<UploadReport, StorageError> {
        self.check_target(bucket, Some(key))?;
        let _slot = self.transfer_slot().await?;
        let (size, modified, sha256) = hash_file(path).await?;
        let plan = plan_upload(size, &self.account.profile.limits, &self.options)?;
        let (parts, version_id) = match plan {
            UploadPlan::Single => {
                let body = ByteStream::from_path(path)
                    .await
                    .map_err(|e| StorageError::Io(format!("{}: {e}", path.display())))?;
                let out = self
                    .client
                    .put_object()
                    .bucket(bucket)
                    .key(key)
                    .content_length(i64::try_from(size).unwrap_or(i64::MAX))
                    .metadata(SHA256_METADATA, &sha256)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| classify("upload", e))?;
                (1, out.version_id().map(str::to_owned))
            }
            UploadPlan::Multipart { part_size, parts } => {
                let version = self
                    .upload_multipart(bucket, key, path, &sha256, size, part_size)
                    .await?;
                (parts, version)
            }
        };
        // The hash was taken before sending; if the file changed since, the
        // object does not match it. Remove exactly the version just created.
        let unchanged = tokio::fs::metadata(path)
            .await
            .map(|m| m.len() == size && m.modified().ok() == modified)
            .unwrap_or(false);
        if !unchanged {
            if let Err(e) = self.delete_object(bucket, key, version_id.as_deref()).await {
                eprintln!("sotyware: cannot remove the inconsistent upload of '{key}': {e}");
            }
            return Err(StorageError::FileChanged(path.display().to_string()));
        }
        Ok(UploadReport {
            size,
            sha256,
            parts,
            version_id,
        })
    }

    async fn upload_multipart(
        &self,
        bucket: &str,
        key: &str,
        path: &Path,
        sha256: &str,
        size: u64,
        part_size: u64,
    ) -> Result<Option<String>, StorageError> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .metadata(SHA256_METADATA, sha256)
            .send()
            .await
            .map_err(|e| classify("start multipart upload", e))?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| StorageError::Provider {
                status: 0,
                code: String::new(),
                message: "start multipart upload: no upload id".into(),
            })?
            .to_owned();
        let result = async {
            let mut completed = Vec::new();
            let mut offset = 0u64;
            let mut number = 1i32;
            while offset < size {
                let len = part_size.min(size - offset);
                let body = ByteStream::read_from()
                    .path(path)
                    .offset(offset)
                    .length(Length::Exact(len))
                    .build()
                    .await
                    .map_err(|e| StorageError::Io(format!("{}: {e}", path.display())))?;
                let out = self
                    .client
                    .upload_part()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .part_number(number)
                    .content_length(i64::try_from(len).unwrap_or(i64::MAX))
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| classify(&format!("upload part {number}"), e))?;
                completed.push(
                    CompletedPart::builder()
                        .part_number(number)
                        .set_e_tag(out.e_tag().map(str::to_owned))
                        .build(),
                );
                offset += len;
                number += 1;
            }
            let out = self
                .client
                .complete_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .multipart_upload(
                    CompletedMultipartUpload::builder()
                        .set_parts(Some(completed))
                        .build(),
                )
                .send()
                .await
                .map_err(|e| classify("complete multipart upload", e))?;
            Ok(out.version_id().map(str::to_owned))
        }
        .await;
        if result.is_err()
            && let Err(e) = self
                .client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await
        {
            eprintln!(
                "sotyware: cannot abort the multipart upload of '{key}': {}",
                classify("abort multipart upload", e)
            );
        }
        result
    }

    /// Downloads `bucket/key` to `dest`, which must not exist yet. The data is
    /// written to `dest` + `.sotyware-partial`, verified, and only then renamed
    /// to `dest`; on any failure the partial file is removed and `dest` is not
    /// created.
    pub async fn download_file(
        &self,
        bucket: &str,
        key: &str,
        dest: &Path,
    ) -> Result<DownloadReport, StorageError> {
        self.check_target(bucket, Some(key))?;
        if tokio::fs::try_exists(dest).await.unwrap_or(true) {
            return Err(StorageError::Refused(format!(
                "{} already exists",
                dest.display()
            )));
        }
        let _slot = self.transfer_slot().await?;
        let partial = partial_path(dest);
        let result = self.download_to(bucket, key, &partial).await;
        let result = match result {
            Ok(report) => tokio::fs::rename(&partial, dest)
                .await
                .map(|()| report)
                .map_err(io_error(dest)),
            Err(e) => Err(e),
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        result
    }

    async fn download_to(
        &self,
        bucket: &str,
        key: &str,
        partial: &Path,
    ) -> Result<DownloadReport, StorageError> {
        let out = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| classify("download", e))?;
        let expected_sha = out.metadata().and_then(|m| m.get(SHA256_METADATA)).cloned();
        let expected_size = out.content_length().and_then(|s| u64::try_from(s).ok());
        let mut body = out.body;
        let mut file = tokio::fs::File::create(partial)
            .await
            .map_err(io_error(partial))?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = body
            .try_next()
            .await
            .map_err(|e| StorageError::Network(format!("download: {}", DisplayErrorContext(&e))))?
        {
            hasher.update(&chunk);
            size += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(io_error(partial))?;
        }
        file.sync_all().await.map_err(io_error(partial))?;
        drop(file);
        let sha256 = hex(&hasher.finalize());
        if let Some(expected) = expected_size.filter(|&e| e != size) {
            return Err(StorageError::Integrity(format!(
                "{key}: received {size} bytes, expected {expected}"
            )));
        }
        let verification = match expected_sha {
            Some(expected) if expected == sha256 => Verification::Sha256Matched,
            Some(expected) => {
                return Err(StorageError::Integrity(format!(
                    "{key}: SHA-256 {sha256}, expected {expected}"
                )));
            }
            None => Verification::SizeOnly,
        };
        Ok(DownloadReport {
            size,
            sha256,
            verification,
        })
    }

    /// Deletes `bucket/key`. With a `version_id`, that exact version is
    /// removed for good; without one, a versioned bucket only hides the object
    /// (the data stays and is still billed).
    pub async fn delete_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> Result<(), StorageError> {
        self.check_target(bucket, Some(key))?;
        self.client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await
            .map(drop)
            .map_err(|e| classify("delete", e))
    }
}

#[cfg(test)]
mod tests {
    use super::super::provider::{BACKBLAZE_B2, HETZNER, S3_COMPATIBLE};
    use super::*;

    const GIB: u64 = 1024 * MIB;

    #[test]
    fn small_files_use_one_put() {
        let o = ClientOptions::default();
        for size in [0, 1, 64 * MIB] {
            assert_eq!(
                plan_upload(size, &BACKBLAZE_B2.limits, &o),
                Ok(UploadPlan::Single),
                "{size}"
            );
        }
    }

    #[test]
    fn large_files_use_multipart_with_the_preferred_part_size() {
        let o = ClientOptions::default();
        assert_eq!(
            plan_upload(64 * MIB + 1, &HETZNER.limits, &o),
            Ok(UploadPlan::Multipart {
                part_size: 16 * MIB,
                parts: 5
            })
        );
    }

    #[test]
    fn a_multipart_upload_never_has_a_single_part() {
        let o = ClientOptions {
            multipart_threshold: 6 * MIB,
            part_size: 16 * MIB,
            ..ClientOptions::default()
        };
        // Above the threshold but within one part: one PUT instead.
        assert_eq!(
            plan_upload(10 * MIB, &S3_COMPATIBLE.limits, &o),
            Ok(UploadPlan::Single)
        );
    }

    #[test]
    fn the_part_size_never_drops_below_the_provider_minimum() {
        let o = ClientOptions {
            multipart_threshold: MIB,
            part_size: MIB,
            ..ClientOptions::default()
        };
        // Backblaze's minimum is 5 MB (decimal); parts are whole MiB above it.
        assert_eq!(
            plan_upload(20 * MIB, &BACKBLAZE_B2.limits, &o),
            Ok(UploadPlan::Multipart {
                part_size: 5 * MIB,
                parts: 4
            })
        );
    }

    #[test]
    fn very_large_files_get_bigger_parts_to_stay_within_max_parts() {
        let o = ClientOptions::default();
        let size = 1024 * GIB; // 1 TiB: 16 MiB parts would need 65,536.
        let Ok(UploadPlan::Multipart { part_size, parts }) = plan_upload(size, &HETZNER.limits, &o)
        else {
            panic!("expected multipart");
        };
        assert!(parts <= HETZNER.limits.max_parts);
        assert!(part_size <= HETZNER.limits.max_part_size);
        assert_eq!(part_size % MIB, 0);
        assert!(part_size * parts >= size);
    }

    #[test]
    fn files_beyond_the_provider_limit_are_refused_before_any_request() {
        let o = ClientOptions::default();
        let too_big = HETZNER.limits.max_object_size + 1;
        assert_eq!(
            plan_upload(too_big, &HETZNER.limits, &o),
            Err(StorageError::TooLarge {
                size: too_big,
                max: HETZNER.limits.max_object_size
            })
        );
        // A threshold above the single-PUT limit is capped by it.
        let o = ClientOptions {
            multipart_threshold: u64::MAX,
            ..ClientOptions::default()
        };
        assert!(matches!(
            plan_upload(6 * GIB, &S3_COMPATIBLE.limits, &o),
            Ok(UploadPlan::Multipart { .. })
        ));
    }

    #[test]
    fn provider_errors_are_classified_by_code_then_status() {
        use StorageError::*;
        let c = |status, code| classify_response(status, code, "m", "op");
        assert!(matches!(
            c(403, Some("InvalidAccessKeyId")),
            InvalidCredentials(_)
        ));
        assert!(matches!(
            c(403, Some("SignatureDoesNotMatch")),
            InvalidCredentials(_)
        ));
        assert!(matches!(
            c(401, Some("InvalidToken")),
            InvalidCredentials(_)
        ));
        assert!(matches!(c(403, Some("NotSignedUp")), InvalidCredentials(_)));
        assert!(matches!(c(403, Some("AccessDenied")), AccessDenied(_)));
        assert!(matches!(c(404, Some("NoSuchBucket")), NotFound(_)));
        assert!(matches!(c(404, Some("NoSuchKey")), NotFound(_)));
        assert!(matches!(c(404, None), NotFound(_)));
        assert!(matches!(c(403, None), AccessDenied(_)));
        assert!(matches!(c(400, Some("RequestTimeout")), Timeout(_)));
        let e = c(503, Some("SlowDown"));
        assert!(matches!(e, Provider { status: 503, .. }));
        assert!(e.is_transient());
        assert!(!c(403, Some("AccessDenied")).is_transient());
        assert!(!c(400, Some("InvalidArgument")).is_transient());
    }

    #[test]
    fn hex_and_partial_paths() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(
            partial_path(Path::new("dir/photo.jpg")),
            PathBuf::from("dir/photo.jpg.sotyware-partial")
        );
    }
}

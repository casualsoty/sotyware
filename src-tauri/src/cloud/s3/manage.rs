//! The object-management requests of [`S3Store`] (ADR-026): copying an
//! object server side, creating a folder marker, deleting with the
//! provider's answer about delete markers, listing recursively and listing
//! versions page by page, reading an object's properties, reading the first
//! bytes of an object for a preview, and presigning a download link.
//!
//! These are single requests. Ordering, verification of a copy, conflicts
//! and "never delete before the copy is verified" belong to
//! `cloud::operations`, which is the only caller besides the tests.

use std::collections::BTreeMap;
use std::time::Duration;

use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::{ByteStream, DateTimeFormat};
use aws_sdk_s3::types::MetadataDirective;

use super::{MAX_KEY_BYTES, S3Store, SHA256_METADATA, StorageError, classify};

/// Largest object `CopyObject` copies in one request (S3, Backblaze and
/// Hetzner alike). Larger objects would need a multipart copy, which
/// sotyware does not do: they are refused before anything is sent.
pub const MAX_COPY_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Longest a presigned link may stay valid (SigV4's limit, 7 days).
pub const MAX_PRESIGN: Duration = Duration::from_secs(7 * 24 * 3600);

/// Everything a `HeadObject` says about an object that the page may show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectProperties {
    pub size: u64,
    /// RFC 3339.
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    /// As the provider sent it, quotes removed.
    pub e_tag: Option<String>,
    pub version_id: Option<String>,
    pub storage_class: Option<String>,
    pub cache_control: Option<String>,
    pub content_encoding: Option<String>,
    pub content_disposition: Option<String>,
    /// User metadata (`x-amz-meta-*`), names lowercase, sorted.
    pub metadata: BTreeMap<String, String>,
}

impl ObjectProperties {
    /// The SHA-256 sotyware stored at upload, if any.
    pub fn sha256(&self) -> Option<&str> {
        self.metadata.get(SHA256_METADATA).map(String::as_str)
    }
}

/// What a `DeleteObject` without a version id did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteOutcome {
    /// The provider created a delete marker (a versioned bucket): the
    /// object is hidden and its versions are kept, so it can be restored.
    pub marker_created: bool,
    /// The version id of the delete marker, if one was created.
    pub marker_version: Option<String>,
}

/// An object of a recursive listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedObject {
    pub key: String,
    pub size: u64,
    /// RFC 3339.
    pub last_modified: Option<String>,
    /// Quotes removed.
    pub e_tag: Option<String>,
}

/// Whether a bucket keeps versions, as `GetBucketVersioning` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Versioning {
    Enabled,
    Suspended,
    /// Never enabled: a delete removes the object for good.
    Disabled,
}

/// One page of a recursive listing (no delimiter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPage {
    pub objects: Vec<ListedObject>,
    pub next_token: Option<String>,
}

/// One entry of a version listing: an object version or a delete marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionInfo {
    pub key: String,
    pub version_id: String,
    pub is_latest: bool,
    pub delete_marker: bool,
    pub size: u64,
    /// RFC 3339.
    pub last_modified: Option<String>,
}

/// One page of a version listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionPage {
    pub entries: Vec<VersionInfo>,
    /// `(key marker, version id marker)` of the next page.
    pub next: Option<(String, Option<String>)>,
}

/// The first bytes of an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectHead {
    pub bytes: Vec<u8>,
    /// The object's whole size, if the provider said.
    pub total: Option<u64>,
    pub content_type: Option<String>,
}

/// `bucket/key` for `x-amz-copy-source`: every byte percent-encoded except
/// the unreserved characters and `/`, so any key (spaces, `+`, `?`, `#`,
/// `%`, Unicode) names exactly itself.
pub fn copy_source(bucket: &str, key: &str, version: Option<&str>) -> String {
    fn encode(s: &str, out: &mut String) {
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let mut out = String::new();
    encode(bucket, &mut out);
    out.push('/');
    encode(key, &mut out);
    if let Some(v) = version {
        out.push_str("?versionId=");
        encode(v, &mut out);
    }
    out
}

fn rfc3339(t: Option<&aws_sdk_s3::primitives::DateTime>) -> Option<String> {
    t.and_then(|t| t.fmt(DateTimeFormat::DateTime).ok())
}

fn unquote(e: Option<&str>) -> Option<String> {
    e.map(|e| e.trim_matches('"').to_owned())
}

fn size_of(n: Option<i64>) -> u64 {
    n.and_then(|s| u64::try_from(s).ok()).unwrap_or(0)
}

impl S3Store {
    /// Copies `bucket/source` (the version `source_version` if given, else
    /// the current one, which must still have the ETag `if_match` if given)
    /// to `bucket/dest`, server side, with all metadata copied
    /// (`MetadataDirective::COPY`: content type, cache and encoding headers,
    /// user metadata including sotyware's SHA-256). Does not check whether
    /// `dest` exists: the caller does, right before. Returns the new
    /// object's version id, if the bucket is versioned.
    pub async fn copy_object(
        &self,
        bucket: &str,
        source: &str,
        source_version: Option<&str>,
        if_match: Option<&str>,
        dest: &str,
    ) -> Result<Option<String>, StorageError> {
        self.check_target(bucket, Some(source))?;
        self.check_target(bucket, Some(dest))?;
        let out = self
            .client
            .copy_object()
            .bucket(bucket)
            .key(dest)
            .copy_source(copy_source(bucket, source, source_version))
            .set_copy_source_if_match(if_match.map(|e| format!("\"{e}\"")))
            .metadata_directive(MetadataDirective::Copy)
            .send()
            .await
            .map_err(|e| classify("copy object", e))?;
        Ok(out.version_id().map(str::to_owned))
    }

    /// Creates the zero-byte folder marker `bucket/key` (`key` ends in `/`).
    pub async fn put_folder_marker(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        self.check_target(bucket, Some(key))?;
        if !key.ends_with('/') {
            return Err(StorageError::Refused(
                "a folder marker's key ends in '/'".into(),
            ));
        }
        let out = self
            .client
            .put_object()
            .bucket(bucket)
            .key(key)
            .content_length(0)
            .body(ByteStream::from_static(b""))
            .send()
            .await
            .map_err(|e| classify("create folder", e))?;
        Ok(out.version_id().map(str::to_owned))
    }

    /// Deletes `bucket/key` (or, with `version_id`, that exact version) and
    /// reports whether the provider created a delete marker.
    pub async fn delete_reporting(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> Result<DeleteOutcome, StorageError> {
        self.check_target(bucket, Some(key))?;
        let out = self
            .client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await
            .map_err(|e| classify("delete", e))?;
        let marker_created = version_id.is_none() && out.delete_marker() == Some(true);
        Ok(DeleteOutcome {
            marker_created,
            marker_version: marker_created
                .then(|| out.version_id().map(str::to_owned))
                .flatten(),
        })
    }

    /// One page of every object under `prefix`, at any depth (no
    /// delimiter), at most `max_keys`.
    pub async fn list_recursive_page(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: i32,
    ) -> Result<ObjectPage, StorageError> {
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
            .max_keys(max_keys)
            .set_continuation_token(continuation.map(str::to_owned))
            .send()
            .await
            .map_err(|e| classify("list objects", e))?;
        let objects = out
            .contents()
            .iter()
            .filter_map(|o| {
                Some(ListedObject {
                    key: o.key()?.to_owned(),
                    size: size_of(o.size()),
                    last_modified: rfc3339(o.last_modified()),
                    e_tag: unquote(o.e_tag()),
                })
            })
            // A provider that returned keys outside the prefix: dropped.
            .filter(|o| o.key.starts_with(prefix))
            .collect();
        let next_token = match out.next_continuation_token() {
            Some(next) if out.is_truncated() == Some(true) && Some(next) != continuation => {
                Some(next.to_owned())
            }
            _ => None,
        };
        Ok(ObjectPage {
            objects,
            next_token,
        })
    }

    /// The bucket's versioning state. Some keys may not ask (Backblaze
    /// application keys without `readBucketInfo`): the error is returned.
    pub async fn bucket_versioning(&self, bucket: &str) -> Result<Versioning, StorageError> {
        self.check_target(bucket, None)?;
        let out = self
            .client
            .get_bucket_versioning()
            .bucket(bucket)
            .send()
            .await
            .map_err(|e| classify("bucket versioning", e))?;
        Ok(
            match out.status().map(|s| s.as_str().to_owned()).as_deref() {
                Some("Enabled") => Versioning::Enabled,
                Some("Suspended") => Versioning::Suspended,
                _ => Versioning::Disabled,
            },
        )
    }

    /// One page of the versions and delete markers under `prefix`.
    pub async fn list_versions_page(
        &self,
        bucket: &str,
        prefix: &str,
        marker: Option<(&str, Option<&str>)>,
        max_keys: i32,
    ) -> Result<VersionPage, StorageError> {
        self.check_target(bucket, None)?;
        let out = self
            .client
            .list_object_versions()
            .bucket(bucket)
            .prefix(prefix)
            .max_keys(max_keys)
            .set_key_marker(marker.map(|(k, _)| k.to_owned()))
            .set_version_id_marker(marker.and_then(|(_, v)| v.map(str::to_owned)))
            .send()
            .await
            .map_err(|e| classify("list versions", e))?;
        let mut entries = Vec::new();
        for v in out.versions() {
            if let (Some(key), Some(id)) = (v.key(), v.version_id()) {
                entries.push(VersionInfo {
                    key: key.to_owned(),
                    version_id: id.to_owned(),
                    is_latest: v.is_latest() == Some(true),
                    delete_marker: false,
                    size: size_of(v.size()),
                    last_modified: rfc3339(v.last_modified()),
                });
            }
        }
        for m in out.delete_markers() {
            if let (Some(key), Some(id)) = (m.key(), m.version_id()) {
                entries.push(VersionInfo {
                    key: key.to_owned(),
                    version_id: id.to_owned(),
                    is_latest: m.is_latest() == Some(true),
                    delete_marker: true,
                    size: 0,
                    last_modified: rfc3339(m.last_modified()),
                });
            }
        }
        entries.retain(|e| e.key.starts_with(prefix));
        let next = match (out.is_truncated(), out.next_key_marker()) {
            (Some(true), Some(k)) => {
                let v = out.next_version_id_marker().map(str::to_owned);
                // The same marker again would repeat the page forever.
                if marker.is_some_and(|(mk, mv)| mk == k && mv.map(str::to_owned) == v) {
                    None
                } else {
                    Some((k.to_owned(), v))
                }
            }
            _ => None,
        };
        Ok(VersionPage { entries, next })
    }

    /// The object's properties, or `None` if it does not exist (or the
    /// current version is a delete marker).
    pub async fn object_properties(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectProperties>, StorageError> {
        self.check_target(bucket, Some(key))?;
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => Ok(Some(ObjectProperties {
                size: size_of(out.content_length()),
                last_modified: rfc3339(out.last_modified()),
                content_type: out.content_type().map(str::to_owned),
                e_tag: unquote(out.e_tag()),
                version_id: out.version_id().map(str::to_owned),
                storage_class: out.storage_class().map(|s| s.as_str().to_owned()),
                cache_control: out.cache_control().map(str::to_owned),
                content_encoding: out.content_encoding().map(str::to_owned),
                content_disposition: out.content_disposition().map(str::to_owned),
                metadata: out
                    .metadata()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            })),
            Err(e) => match classify("head object", e) {
                StorageError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// At most the first `max` bytes of `bucket/key` (a ranged `GetObject`):
    /// never the whole object when it is larger.
    pub async fn read_head(
        &self,
        bucket: &str,
        key: &str,
        max: u64,
    ) -> Result<ObjectHead, StorageError> {
        self.check_target(bucket, Some(key))?;
        if max == 0 {
            return Err(StorageError::Refused("nothing to read".into()));
        }
        let out = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .range(format!("bytes=0-{}", max - 1))
            .send()
            .await
            .map_err(|e| classify("read object", e))?;
        let total = out
            .content_range()
            .and_then(|r| r.rsplit('/').next())
            .and_then(|t| t.parse::<u64>().ok())
            .or_else(|| out.content_length().and_then(|n| u64::try_from(n).ok()));
        let content_type = out.content_type().map(str::to_owned);
        let mut body = out.body;
        let mut bytes = Vec::new();
        while let Some(chunk) = body
            .try_next()
            .await
            .map_err(|e| StorageError::Network(format!("read object: {e}")))?
        {
            let room = usize::try_from(max)
                .unwrap_or(usize::MAX)
                .saturating_sub(bytes.len());
            // A provider that ignored the range: stop at the limit.
            bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if bytes.len() as u64 >= max {
                break;
            }
        }
        Ok(ObjectHead {
            bytes,
            total,
            content_type,
        })
    }

    /// A link that downloads `bucket/key` without credentials until it
    /// expires after `expires_in` (at most [`MAX_PRESIGN`]). Signed locally:
    /// no request is sent. The link carries the access key id and a
    /// signature, never the secret.
    pub async fn presign_get(
        &self,
        bucket: &str,
        key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        self.check_target(bucket, Some(key))?;
        if expires_in.is_zero() || expires_in > MAX_PRESIGN {
            return Err(StorageError::Refused(
                "a link is valid for 1 second to 7 days".into(),
            ));
        }
        let config = PresigningConfig::expires_in(expires_in)
            .map_err(|e| StorageError::Refused(format!("link expiry: {e}")))?;
        let req = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .presigned(config)
            .await
            .map_err(|e| classify("sign link", e))?;
        Ok(req.uri().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_source_encodes_everything_but_unreserved_and_slash() {
        assert_eq!(copy_source("b", "a/b c.txt", None), "b/a/b%20c.txt");
        assert_eq!(copy_source("b", "x+y?z#%", None), "b/x%2By%3Fz%23%25");
        assert_eq!(copy_source("b", "größe", None), "b/gr%C3%B6%C3%9Fe");
        assert_eq!(
            copy_source("b", "a//..", Some("v 1")),
            "b/a//..?versionId=v%201"
        );
    }
}

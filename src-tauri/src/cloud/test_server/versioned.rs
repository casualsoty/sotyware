//! An in-memory S3 backend with object versions (tests only): what the
//! file-management operations need (`HeadBucket`, `HeadObject`, ranged
//! `GetObject`, `PutObject`, `CopyObject`, `DeleteObject` with and without a
//! version id, `ListObjectsV2`, `ListObjectVersions`), with S3's versioning
//! semantics: on a versioned bucket a plain delete adds a delete marker and
//! keeps every version; deleting a version by id removes exactly it (a
//! delete marker removed that way makes the previous version current
//! again). An unversioned bucket keeps one version ("null") per key.
//!
//! s3s-fs has no versions and cannot hold keys such as `a//b`; this backend
//! keeps keys exactly. It does not prove how a real provider behaves.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use s3s::dto::*;
use sha2::{Digest, Sha256};

use super::Listing;

/// One version of a key (or a delete marker).
#[derive(Clone, Debug)]
pub struct Version {
    pub id: String,
    pub marker: bool,
    pub data: Arc<Vec<u8>>,
    pub content_type: Option<String>,
    pub metadata: HashMap<String, String>,
    pub etag: String,
    /// Seconds after 2025-11-11T12:00:00Z.
    pub modified: u64,
}

#[derive(Default)]
pub struct Bucket {
    pub versioning: bool,
    /// Oldest first.
    pub keys: BTreeMap<String, Vec<Version>>,
}

impl Bucket {
    /// The current version, unless the key is absent or its latest entry is
    /// a delete marker.
    pub fn current(&self, key: &str) -> Option<&Version> {
        self.keys.get(key)?.last().filter(|v| !v.marker)
    }
}

#[derive(Default)]
pub struct State {
    pub buckets: HashMap<String, Bucket>,
    next: u64,
    clock: u64,
}

impl State {
    fn stamp(&mut self) -> (String, u64) {
        self.next += 1;
        self.clock += 1;
        (format!("v{:06}", self.next), self.clock)
    }

    /// Stores a new version of `key` (what a PUT or COPY does).
    pub fn put(
        &mut self,
        bucket: &str,
        key: &str,
        data: Vec<u8>,
        content_type: Option<String>,
        metadata: HashMap<String, String>,
    ) -> Option<String> {
        let (id, modified) = self.stamp();
        let b = self.buckets.get_mut(bucket)?;
        let etag = hex(&Sha256::digest(&data))[..32].to_owned();
        let version = Version {
            id: if b.versioning { id } else { "null".into() },
            marker: false,
            data: Arc::new(data),
            content_type,
            metadata,
            etag,
            modified,
        };
        let id = version.id.clone();
        let list = b.keys.entry(key.to_owned()).or_default();
        if !b.versioning {
            list.clear();
        }
        list.push(version);
        b.versioning.then_some(id)
    }

    /// Every current object of `bucket`: key → version.
    pub fn current_objects(&self, bucket: &str) -> BTreeMap<String, Version> {
        self.buckets
            .get(bucket)
            .map(|b| {
                b.keys
                    .keys()
                    .filter_map(|k| b.current(k).map(|v| (k.clone(), v.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn timestamp(modified: u64) -> Timestamp {
    Timestamp::from(
        SystemTime::UNIX_EPOCH + Duration::from_secs(super::LISTING_MODIFIED_SECS + modified),
    )
}

/// The shared state of a running versioned server.
#[derive(Clone, Default)]
pub struct Versioned(pub Arc<Mutex<State>>);

impl Versioned {
    pub fn new(buckets: &[(&str, bool)]) -> Self {
        let mut state = State::default();
        for (name, versioning) in buckets {
            state.buckets.insert(
                (*name).to_owned(),
                Bucket {
                    versioning: *versioning,
                    keys: BTreeMap::new(),
                },
            );
        }
        Self(Arc::new(Mutex::new(state)))
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Puts an object as another client would (helper for tests).
    pub fn seed(&self, bucket: &str, key: &str, data: &[u8]) -> Option<String> {
        self.lock()
            .put(bucket, key, data.to_vec(), None, HashMap::new())
    }
}

type Fut<T> = Pin<Box<dyn Future<Output = s3s::S3Result<s3s::S3Response<T>>> + Send + 'static>>;

fn done<T: Send + 'static>(r: s3s::S3Result<T>) -> Fut<T> {
    Box::pin(async move { r.map(s3s::S3Response::new) })
}

fn no_bucket() -> s3s::S3Error {
    s3s::s3_error!(NoSuchBucket)
}

async fn read_body(body: Option<StreamingBlob>) -> s3s::S3Result<Vec<u8>> {
    let Some(mut body) = body else {
        return Ok(Vec::new());
    };
    let mut data = Vec::new();
    while let Some(chunk) =
        std::future::poll_fn(|cx| futures_core::Stream::poll_next(Pin::new(&mut body), cx)).await
    {
        let chunk = chunk.map_err(|e| s3s::s3_error!(IncompleteBody, "{e}"))?;
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// `bytes=a-b` within `len`: the half-open range.
fn range_of(range: Option<&Range>, len: usize) -> (usize, usize) {
    match range {
        Some(Range::Int { first, last }) => {
            let first = usize::try_from(*first).unwrap_or(usize::MAX).min(len);
            let end = last
                .map(|l| usize::try_from(l).unwrap_or(usize::MAX).saturating_add(1))
                .unwrap_or(len)
                .min(len);
            (first, end.max(first))
        }
        Some(Range::Suffix { length }) => {
            let n = usize::try_from(*length).unwrap_or(usize::MAX).min(len);
            (len - n, len)
        }
        None => (0, len),
    }
}

impl s3s::S3 for Versioned {
    fn head_bucket<'a, 'b>(&'a self, req: s3s::S3Request<HeadBucketInput>) -> Fut<HeadBucketOutput>
    where
        'a: 'b,
    {
        let ok = self.lock().buckets.contains_key(&req.input.bucket);
        done(if ok {
            Ok(HeadBucketOutput::default())
        } else {
            Err(no_bucket())
        })
    }

    fn get_bucket_versioning<'a, 'b>(
        &'a self,
        req: s3s::S3Request<GetBucketVersioningInput>,
    ) -> Fut<GetBucketVersioningOutput>
    where
        'a: 'b,
    {
        let st = self.lock();
        let r = match st.buckets.get(&req.input.bucket) {
            None => Err(no_bucket()),
            Some(b) => Ok(GetBucketVersioningOutput {
                status: b
                    .versioning
                    .then(|| BucketVersioningStatus::from_static(BucketVersioningStatus::ENABLED)),
                ..Default::default()
            }),
        };
        done(r)
    }

    fn head_object<'a, 'b>(&'a self, req: s3s::S3Request<HeadObjectInput>) -> Fut<HeadObjectOutput>
    where
        'a: 'b,
    {
        let st = self.lock();
        let r = (|| {
            let b = st.buckets.get(&req.input.bucket).ok_or_else(no_bucket)?;
            let v = match &req.input.version_id {
                Some(id) => b
                    .keys
                    .get(&req.input.key)
                    .and_then(|l| l.iter().find(|v| &v.id == id && !v.marker)),
                None => b.current(&req.input.key),
            }
            .ok_or_else(|| s3s::s3_error!(NoSuchKey))?;
            Ok(HeadObjectOutput {
                content_length: Some(v.data.len() as i64),
                content_type: v.content_type.clone(),
                e_tag: Some(ETag::Strong(v.etag.clone())),
                last_modified: Some(timestamp(v.modified)),
                metadata: (!v.metadata.is_empty()).then(|| v.metadata.clone()),
                version_id: b.versioning.then(|| v.id.clone()),
                ..Default::default()
            })
        })();
        done(r)
    }

    fn get_object<'a, 'b>(&'a self, req: s3s::S3Request<GetObjectInput>) -> Fut<GetObjectOutput>
    where
        'a: 'b,
    {
        let st = self.lock();
        let r = (|| {
            let b = st.buckets.get(&req.input.bucket).ok_or_else(no_bucket)?;
            let v = b
                .current(&req.input.key)
                .ok_or_else(|| s3s::s3_error!(NoSuchKey))?;
            let len = v.data.len();
            let (start, end) = range_of(req.input.range.as_ref(), len);
            let slice = v.data[start..end].to_vec();
            let body = super::Body {
                data: Arc::new(slice),
                pos: 0,
                fault: super::Fault::None,
                delay: None,
            };
            Ok(GetObjectOutput {
                body: Some(StreamingBlob::wrap(body)),
                content_length: Some((end - start) as i64),
                content_range: req
                    .input
                    .range
                    .is_some()
                    .then(|| format!("bytes {start}-{}/{len}", end.saturating_sub(1))),
                content_type: v.content_type.clone(),
                e_tag: Some(ETag::Strong(v.etag.clone())),
                metadata: (!v.metadata.is_empty()).then(|| v.metadata.clone()),
                version_id: b.versioning.then(|| v.id.clone()),
                ..Default::default()
            })
        })();
        done(r)
    }

    fn put_object<'a, 'b>(&'a self, req: s3s::S3Request<PutObjectInput>) -> Fut<PutObjectOutput>
    where
        'a: 'b,
    {
        let state = Arc::clone(&self.0);
        let input = req.input;
        Box::pin(async move {
            let data = read_body(input.body).await?;
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            if !st.buckets.contains_key(&input.bucket) {
                return Err(no_bucket());
            }
            let etag = hex(&Sha256::digest(&data))[..32].to_owned();
            let version_id = st.put(
                &input.bucket,
                &input.key,
                data,
                input.content_type,
                input.metadata.unwrap_or_default(),
            );
            Ok(s3s::S3Response::new(PutObjectOutput {
                e_tag: Some(ETag::Strong(etag)),
                version_id,
                ..Default::default()
            }))
        })
    }

    fn copy_object<'a, 'b>(&'a self, req: s3s::S3Request<CopyObjectInput>) -> Fut<CopyObjectOutput>
    where
        'a: 'b,
    {
        let mut st = self.lock();
        let r = (|| {
            let CopySource::Bucket {
                bucket: src_bucket,
                key: src_key,
                version_id,
            } = &req.input.copy_source
            else {
                return Err(s3s::s3_error!(
                    InvalidArgument,
                    "access points are not supported"
                ));
            };
            let b = st.buckets.get(&**src_bucket).ok_or_else(no_bucket)?;
            let v = match version_id {
                Some(id) => b
                    .keys
                    .get(&**src_key)
                    .and_then(|l| l.iter().find(|v| *v.id == **id && !v.marker)),
                None => b.current(src_key),
            }
            .ok_or_else(|| s3s::s3_error!(NoSuchKey))?
            .clone();
            if let Some(m) = &req.input.copy_source_if_match
                && !m.is_any()
                && m.as_etag().map(|e| e.value()) != Some(v.etag.as_str())
            {
                return Err(s3s::s3_error!(PreconditionFailed));
            }
            let replace = req
                .input
                .metadata_directive
                .as_ref()
                .is_some_and(|d| d.as_str() == MetadataDirective::REPLACE);
            let (content_type, metadata) = if replace {
                (
                    req.input.content_type.clone(),
                    req.input.metadata.clone().unwrap_or_default(),
                )
            } else {
                (v.content_type.clone(), v.metadata.clone())
            };
            if !st.buckets.contains_key(&req.input.bucket) {
                return Err(no_bucket());
            }
            let etag = v.etag.clone();
            let version = st.put(
                &req.input.bucket,
                &req.input.key,
                (*v.data).clone(),
                content_type,
                metadata,
            );
            Ok(CopyObjectOutput {
                copy_object_result: Some(CopyObjectResult {
                    e_tag: Some(ETag::Strong(etag)),
                    ..Default::default()
                }),
                version_id: version,
                ..Default::default()
            })
        })();
        done(r)
    }

    fn delete_object<'a, 'b>(
        &'a self,
        req: s3s::S3Request<DeleteObjectInput>,
    ) -> Fut<DeleteObjectOutput>
    where
        'a: 'b,
    {
        let mut st = self.lock();
        let (id, modified) = st.stamp();
        let r = (|| {
            let b = st
                .buckets
                .get_mut(&req.input.bucket)
                .ok_or_else(no_bucket)?;
            match &req.input.version_id {
                Some(vid) => {
                    let Some(list) = b.keys.get_mut(&req.input.key) else {
                        return Ok(DeleteObjectOutput::default());
                    };
                    let was_marker = list.iter().find(|v| &v.id == vid).map(|v| v.marker);
                    list.retain(|v| &v.id != vid);
                    if list.is_empty() {
                        b.keys.remove(&req.input.key);
                    }
                    Ok(DeleteObjectOutput {
                        delete_marker: was_marker,
                        version_id: Some(vid.clone()),
                        ..Default::default()
                    })
                }
                None if b.versioning => {
                    b.keys
                        .entry(req.input.key.clone())
                        .or_default()
                        .push(Version {
                            id: id.clone(),
                            marker: true,
                            data: Arc::new(Vec::new()),
                            content_type: None,
                            metadata: HashMap::new(),
                            etag: String::new(),
                            modified,
                        });
                    Ok(DeleteObjectOutput {
                        delete_marker: Some(true),
                        version_id: Some(id),
                        ..Default::default()
                    })
                }
                None => {
                    b.keys.remove(&req.input.key);
                    Ok(DeleteObjectOutput::default())
                }
            }
        })();
        done(r)
    }

    fn list_objects_v2<'a, 'b>(
        &'a self,
        req: s3s::S3Request<ListObjectsV2Input>,
    ) -> Fut<ListObjectsV2Output>
    where
        'a: 'b,
    {
        let st = self.lock();
        let mut listing = Listing::default();
        if st.buckets.contains_key(&req.input.bucket) {
            listing.buckets.insert(
                req.input.bucket.clone(),
                st.current_objects(&req.input.bucket)
                    .into_iter()
                    .map(|(k, v)| (k, v.data.len() as u64))
                    .collect(),
            );
        }
        // With the ETags a real listing has.
        let current = st.current_objects(&req.input.bucket);
        done(listing.list(req.input).map(|mut out| {
            for o in out.contents.iter_mut().flatten() {
                if let Some(v) = o.key.as_ref().and_then(|k| current.get(k)) {
                    o.e_tag = Some(ETag::Strong(v.etag.clone()));
                }
            }
            out
        }))
    }

    fn list_object_versions<'a, 'b>(
        &'a self,
        req: s3s::S3Request<ListObjectVersionsInput>,
    ) -> Fut<ListObjectVersionsOutput>
    where
        'a: 'b,
    {
        let st = self.lock();
        let r = (|| {
            let b = st.buckets.get(&req.input.bucket).ok_or_else(no_bucket)?;
            let prefix = req.input.prefix.clone().unwrap_or_default();
            let max =
                usize::try_from(req.input.max_keys.unwrap_or(1000).clamp(1, 1000)).unwrap_or(1000);
            // Newest first within a key, keys in order; the markers say where
            // the previous page stopped (exclusive).
            let mut all: Vec<(&String, &Version, bool)> = Vec::new();
            for (k, list) in b.keys.range(prefix.clone()..) {
                if !k.starts_with(&prefix) {
                    break;
                }
                for (i, v) in list.iter().enumerate().rev() {
                    all.push((k, v, i + 1 == list.len()));
                }
            }
            let start = match (&req.input.key_marker, &req.input.version_id_marker) {
                (Some(k), Some(vm)) => all
                    .iter()
                    .position(|(key, v, _)| *key == k && &v.id == vm)
                    .map_or(0, |i| i + 1),
                (Some(k), None) => all
                    .iter()
                    .position(|(key, _, _)| key.as_str() > k.as_str())
                    .unwrap_or(all.len()),
                _ => 0,
            };
            let page: Vec<_> = all.iter().skip(start).take(max).collect();
            let truncated = start + page.len() < all.len();
            let mut versions = Vec::new();
            let mut markers = Vec::new();
            for (k, v, latest) in &page {
                if v.marker {
                    markers.push(DeleteMarkerEntry {
                        key: Some((*k).clone()),
                        version_id: Some(v.id.clone()),
                        is_latest: Some(*latest),
                        last_modified: Some(timestamp(v.modified)),
                        ..Default::default()
                    });
                } else {
                    versions.push(ObjectVersion {
                        key: Some((*k).clone()),
                        version_id: Some(v.id.clone()),
                        is_latest: Some(*latest),
                        size: Some(v.data.len() as i64),
                        e_tag: Some(ETag::Strong(v.etag.clone())),
                        last_modified: Some(timestamp(v.modified)),
                        ..Default::default()
                    });
                }
            }
            let last = page.last();
            Ok(ListObjectVersionsOutput {
                is_truncated: Some(truncated),
                next_key_marker: truncated
                    .then(|| last.map(|(k, _, _)| (*k).clone()))
                    .flatten(),
                next_version_id_marker: truncated
                    .then(|| last.map(|(_, v, _)| v.id.clone()))
                    .flatten(),
                versions: (!versions.is_empty()).then_some(versions),
                delete_markers: (!markers.is_empty()).then_some(markers),
                name: Some(req.input.bucket.clone()),
                prefix: req.input.prefix.clone(),
                max_keys: req.input.max_keys,
                ..Default::default()
            })
        })();
        done(r)
    }
}

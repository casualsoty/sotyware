//! The single-request commands of the file manager (ADR-026): creating a
//! folder, an object's properties, a size-limited preview, searching a
//! folder recursively, the deleted items of a folder (versions behind a
//! delete marker), and presigned download links.
//!
//! - **Create folder** puts the zero-byte marker `prefix + name + "/"`
//!   after checking that nothing exists at or under that prefix (one
//!   `ListObjectsV2` of one key). Folders are prefixes; the marker only
//!   makes an empty one visible.
//! - **Preview** never reads more than [`TEXT_PREVIEW_BYTES`] of a text and
//!   never an image larger than [`IMAGE_PREVIEW_BYTES`] (ranged `GetObject`
//!   after a `HeadObject`). Images are recognised by their first bytes
//!   (PNG, JPEG, GIF, WebP, BMP), never by the stored content type, and SVG
//!   or HTML are never images: they are shown as text, if at all. Nothing
//!   is executed or rendered as markup.
//! - **Search** lists the folder recursively (`ListObjectsV2` without a
//!   delimiter), at most [`SEARCH_PAGES`] pages per call, and matches names
//!   in Rust (`name` mode, case-insensitive, file names and folder names),
//!   or asks the provider for keys that start with the folder plus the text
//!   (`prefix` mode, server-side). S3 cannot search file contents. The
//!   continuation token stays in Rust behind a cursor, as for browsing.
//! - **Deleted items** are keys under the folder (any depth) whose latest
//!   version is a delete marker (`ListObjectVersions`, at most
//!   [`DELETED_MAX_ENTRIES`] entries read). Each says whether an earlier
//!   version exists (restorable) or not. A key that may not list versions
//!   gets `access-denied`; storage without versions shows none.
//! - **Share link**: a presigned `GetObject` URL for 1 hour, 1 day or 7
//!   days, signed locally (no request besides a `HeadObject` that the file
//!   exists). It carries the access key id and a signature, never the
//!   secret; anyone with the link can download the file until it expires,
//!   and it cannot be revoked except by removing the key at the provider.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::State;

use super::browser::{FolderEntry, validate_prefix};
use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::operations::{
    MAX_KEY_BYTES, file_name, folder_name, name_problem, operation_error, parent_of,
};
use super::s3::{ObjectProperties, S3Store, StorageError};

/// Most bytes of a text file a preview reads.
pub const TEXT_PREVIEW_BYTES: u64 = 256 * 1024;
/// Largest image a preview reads.
pub const IMAGE_PREVIEW_BYTES: u64 = 10 * 1024 * 1024;
/// Most listing pages one search call reads.
pub const SEARCH_PAGES: usize = 10;
/// Most hits one search call returns.
pub const SEARCH_HITS: usize = 500;
/// Most version entries the deleted-items listing reads.
pub const DELETED_MAX_ENTRIES: usize = 5_000;
/// The link lifetimes the page may choose, in seconds.
pub const LINK_LIFETIMES: [u64; 3] = [3_600, 86_400, 604_800];

async fn store_for(
    accounts: &Arc<CloudAccounts>,
    id: &str,
    bucket: &str,
) -> Result<Arc<S3Store>, CommandError> {
    let (accounts, id, bucket) = (Arc::clone(accounts), id.to_owned(), bucket.to_owned());
    blocking(move || accounts.prepare_download(&id, &bucket)).await
}

fn check_key(key: &str) -> Result<(), CommandError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "an object key is 1 to 1,024 bytes",
        ));
    }
    Ok(())
}

fn logged<T>(what: &str, r: Result<T, CommandError>) -> Result<T, CommandError> {
    if let Err(e) = &r
        && !matches!(
            e.kind,
            ErrorKind::InvalidInput | ErrorKind::DestinationExists | ErrorKind::ObjectNotFound
        )
    {
        eprintln!("sotyware: failed to {what}: {}", e.message);
    }
    r
}

// ----------------------------------------------------------- create folder

/// Creates the folder `name` in `prefix`: its marker object.
pub async fn create_folder(
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    prefix: String,
    name: String,
) -> Result<FolderEntry, CommandError> {
    validate_prefix(&prefix)?;
    if let Some(p) = name_problem(&name) {
        return Err(CommandError::new(ErrorKind::InvalidInput, p));
    }
    let folder = format!("{prefix}{name}/");
    if folder.len() > MAX_KEY_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "the folder's key would be longer than 1,024 bytes",
        ));
    }
    let store = store_for(&accounts, &id, &bucket).await?;
    let page = store
        .list_recursive_page(&bucket, &folder, None, 1)
        .await
        .map_err(|e| operation_error(&e))?;
    if !page.objects.is_empty() {
        return Err(CommandError::new(
            ErrorKind::DestinationExists,
            format!("a folder named '{name}' already exists here"),
        ));
    }
    store
        .put_folder_marker(&bucket, &folder)
        .await
        .map_err(|e| operation_error(&e))?;
    Ok(FolderEntry {
        name,
        prefix: folder,
    })
}

// -------------------------------------------------------------- properties

/// An object's properties, as the page shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    pub key: String,
    pub name: String,
    /// The folder it is in ("" for the root).
    pub folder: String,
    pub bucket: String,
    pub size: u64,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub e_tag: Option<String>,
    pub version_id: Option<String>,
    pub storage_class: Option<String>,
    pub cache_control: Option<String>,
    pub content_encoding: Option<String>,
    pub content_disposition: Option<String>,
    /// The SHA-256 sotyware stored at upload.
    pub sha256: Option<String>,
    /// User metadata (names lowercase), values cut at 1,024 characters.
    pub metadata: Vec<(String, String)>,
}

fn properties_of(bucket: &str, key: &str, p: ObjectProperties) -> Properties {
    Properties {
        key: key.to_owned(),
        name: file_name(key).to_owned(),
        folder: parent_of(key).to_owned(),
        bucket: bucket.to_owned(),
        size: p.size,
        sha256: p.sha256().map(str::to_owned),
        last_modified: p.last_modified,
        content_type: p.content_type,
        e_tag: p.e_tag,
        version_id: p.version_id,
        storage_class: p.storage_class,
        cache_control: p.cache_control,
        content_encoding: p.content_encoding,
        content_disposition: p.content_disposition,
        metadata: p
            .metadata
            .into_iter()
            .map(|(k, v)| (k, v.chars().take(1024).collect()))
            .collect(),
    }
}

pub async fn properties(
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    key: String,
) -> Result<Properties, CommandError> {
    check_key(&key)?;
    let store = store_for(&accounts, &id, &bucket).await?;
    match store.object_properties(&bucket, &key).await {
        Ok(Some(p)) => Ok(properties_of(&bucket, &key, p)),
        Ok(None) => Err(CommandError::new(
            ErrorKind::ObjectNotFound,
            "the file no longer exists",
        )),
        Err(e) => Err(operation_error(&e)),
    }
}

// ----------------------------------------------------------------- preview

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewKind {
    Text,
    Image,
    /// Neither text nor a recognised image.
    Unsupported,
    /// An image larger than the preview limit.
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: PreviewKind,
    pub size: u64,
    /// For an image: its type from its first bytes (`image/png`, …).
    pub mime: Option<String>,
    /// For text: the (first part of the) content.
    pub text: Option<String>,
    /// For an image: its bytes, base64.
    pub data: Option<String>,
    /// Only the first part of the text was read.
    pub truncated: bool,
}

/// The image type of `bytes` by their signature; never SVG.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") && bytes.len() > 14 {
        Some("image/bmp")
    } else {
        None
    }
}

/// `bytes` as text, if they look like UTF-8 text (no NUL, valid UTF-8
/// except possibly a character cut off at the end by the size limit).
pub fn as_text(bytes: &[u8], cut: bool) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_owned()),
        Err(e) if cut && e.error_len().is_none() && bytes.len() - e.valid_up_to() < 4 => {
            Some(String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned())
        }
        Err(_) => None,
    }
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(T[(n >> 18) as usize & 63]));
        out.push(char::from(T[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(T[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(T[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

pub async fn preview(
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    key: String,
) -> Result<Preview, CommandError> {
    check_key(&key)?;
    let store = store_for(&accounts, &id, &bucket).await?;
    let props = store
        .object_properties(&bucket, &key)
        .await
        .map_err(|e| operation_error(&e))?
        .ok_or_else(|| CommandError::new(ErrorKind::ObjectNotFound, "the file no longer exists"))?;
    let size = props.size;
    if size == 0 {
        return Ok(Preview {
            kind: PreviewKind::Text,
            size,
            mime: None,
            text: Some(String::new()),
            data: None,
            truncated: false,
        });
    }
    // The first bytes decide; an image is then read whole if allowed.
    let head = store
        .read_head(&bucket, &key, size.min(TEXT_PREVIEW_BYTES))
        .await
        .map_err(|e| operation_error(&e))?;
    if let Some(mime) = image_type(&head.bytes) {
        if size > IMAGE_PREVIEW_BYTES {
            return Ok(Preview {
                kind: PreviewKind::TooLarge,
                size,
                mime: Some(mime.into()),
                text: None,
                data: None,
                truncated: false,
            });
        }
        let bytes = if (head.bytes.len() as u64) < size {
            store
                .read_head(&bucket, &key, size)
                .await
                .map_err(|e| operation_error(&e))?
                .bytes
        } else {
            head.bytes
        };
        return Ok(Preview {
            kind: PreviewKind::Image,
            size,
            mime: Some(mime.into()),
            text: None,
            data: Some(base64(&bytes)),
            truncated: false,
        });
    }
    let text_bytes = &head.bytes[..head.bytes.len().min(TEXT_PREVIEW_BYTES as usize)];
    let cut = (text_bytes.len() as u64) < size;
    match as_text(text_bytes, cut) {
        Some(text) => Ok(Preview {
            kind: PreviewKind::Text,
            size,
            mime: None,
            text: Some(text),
            data: None,
            truncated: cut,
        }),
        None => Ok(Preview {
            kind: PreviewKind::Unsupported,
            size,
            mime: None,
            text: None,
            data: None,
            truncated: false,
        }),
    }
}

// ------------------------------------------------------------------ search

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchMode {
    /// File and folder names containing the text (case-insensitive),
    /// matched in sotyware.
    Name,
    /// Keys starting with the folder plus the text, filtered by the
    /// provider.
    Prefix,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub folder: bool,
    pub name: String,
    /// The file's key or the folder's prefix.
    pub path: String,
    /// The folder it is in.
    pub parent: String,
    pub size: Option<u64>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Objects looked at by this call.
    pub scanned: u64,
    /// The cursor to continue, if the folder has more.
    pub next: Option<String>,
}

struct SearchCursor {
    account: String,
    bucket: String,
    prefix: String,
    query: String,
    mode: SearchMode,
    token: String,
}

/// The continuation tokens of searches in progress, behind cursor ids.
#[derive(Default)]
pub struct CloudSearches {
    cursors: Mutex<(u64, HashMap<String, SearchCursor>)>,
}

/// Most cursors kept; the oldest go first.
const MAX_CURSORS: usize = 32;

impl CloudSearches {
    fn keep(&self, c: SearchCursor) -> String {
        let mut guard = self.cursors.lock().unwrap_or_else(|e| e.into_inner());
        guard.0 += 1;
        let id = format!("search-{}", guard.0);
        if guard.1.len() >= MAX_CURSORS {
            let oldest = guard
                .1
                .keys()
                .min_by_key(|k| k.trim_start_matches("search-").parse::<u64>().unwrap_or(0))
                .cloned();
            if let Some(o) = oldest {
                guard.1.remove(&o);
            }
        }
        guard.1.insert(id.clone(), c);
        id
    }

    fn take(&self, id: &str) -> Option<SearchCursor> {
        self.cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
            .remove(id)
    }
}

/// The hits of one listed object for a name search: the file if its name
/// matches, and each folder between `prefix` and it whose name matches.
pub fn name_hits(prefix: &str, key: &str, needle: &str, seen: &mut HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(rest) = key.strip_prefix(prefix) else {
        return out;
    };
    let mut at = prefix.len();
    for segment in rest.split_inclusive('/') {
        at += segment.len();
        if let Some(name) = segment.strip_suffix('/') {
            let folder = &key[..at];
            if name.to_lowercase().contains(needle) && seen.insert(folder.to_owned()) {
                out.push(folder.to_owned());
            }
        } else if segment.to_lowercase().contains(needle) {
            out.push(key.to_owned());
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub async fn search(
    accounts: Arc<CloudAccounts>,
    searches: Arc<CloudSearches>,
    id: String,
    bucket: String,
    prefix: String,
    query: String,
    mode: SearchMode,
    cursor: Option<String>,
) -> Result<SearchResults, CommandError> {
    validate_prefix(&prefix)?;
    if query.is_empty() || query.len() > 512 || query.chars().any(char::is_control) {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "type 1 to 512 characters to search for",
        ));
    }
    let token = match &cursor {
        None => None,
        Some(c) => {
            let found = searches.take(c).ok_or_else(|| {
                CommandError::new(
                    ErrorKind::OutdatedPage,
                    "this search can no longer be continued; search again",
                )
            })?;
            if found.account != id
                || found.bucket != bucket
                || found.prefix != prefix
                || found.query != query
                || found.mode != mode
            {
                return Err(CommandError::new(
                    ErrorKind::OutdatedPage,
                    "this search can no longer be continued; search again",
                ));
            }
            Some(found.token)
        }
    };
    let store = store_for(&accounts, &id, &bucket).await?;
    let list_prefix = match mode {
        SearchMode::Name => prefix.clone(),
        SearchMode::Prefix => format!("{prefix}{query}"),
    };
    if list_prefix.len() > MAX_KEY_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "the search text is too long for a key prefix",
        ));
    }
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let mut scanned = 0u64;
    let mut token = token;
    for _ in 0..SEARCH_PAGES {
        let page = store
            .list_recursive_page(&bucket, &list_prefix, token.as_deref(), 1000)
            .await
            .map_err(|e| super::browser::listing_error(&e))?;
        for o in &page.objects {
            scanned += 1;
            let matched = match mode {
                SearchMode::Name => name_hits(&prefix, &o.key, &needle, &mut seen),
                SearchMode::Prefix if !o.key.ends_with('/') => vec![o.key.clone()],
                SearchMode::Prefix => Vec::new(),
            };
            for path in matched {
                let folder = path.ends_with('/');
                hits.push(SearchHit {
                    folder,
                    name: if folder {
                        folder_name(&path)
                    } else {
                        file_name(&path)
                    }
                    .to_owned(),
                    parent: parent_of(&path).to_owned(),
                    size: (!folder).then_some(o.size),
                    last_modified: if folder {
                        None
                    } else {
                        o.last_modified.clone()
                    },
                    path,
                });
            }
        }
        token = page.next_token;
        if token.is_none() || hits.len() >= SEARCH_HITS {
            break;
        }
    }
    let next = token.map(|t| {
        searches.keep(SearchCursor {
            account: id,
            bucket,
            prefix,
            query,
            mode,
            token: t,
        })
    });
    Ok(SearchResults {
        hits,
        scanned,
        next,
    })
}

// ----------------------------------------------------------- deleted items

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedEntry {
    pub key: String,
    pub name: String,
    pub parent: String,
    /// When the delete marker was set.
    pub deleted_at: Option<String>,
    /// The latest delete marker's version id (what a restore names).
    pub marker_version: String,
    /// An earlier version exists, so the file can be restored.
    pub restorable: bool,
    /// The size of the version a restore brings back.
    pub size: Option<u64>,
    /// Versions kept (delete markers not counted).
    pub versions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedListing {
    pub entries: Vec<DeletedEntry>,
    /// More versions exist than were read.
    pub truncated: bool,
}

/// The deleted keys among `versions` (any order): those whose latest entry
/// is a delete marker.
pub fn deleted_entries(versions: Vec<super::s3::VersionInfo>) -> Vec<DeletedEntry> {
    let mut by_key: HashMap<String, Vec<super::s3::VersionInfo>> = HashMap::new();
    for v in versions {
        by_key.entry(v.key.clone()).or_default().push(v);
    }
    let mut out: Vec<DeletedEntry> = by_key
        .into_iter()
        .filter_map(|(key, mut list)| {
            list.sort_by(|a, b| {
                b.is_latest
                    .cmp(&a.is_latest)
                    .then(b.last_modified.cmp(&a.last_modified))
            });
            let latest = list.first()?;
            if !latest.delete_marker {
                return None;
            }
            let newest = list.iter().find(|v| !v.delete_marker);
            Some(DeletedEntry {
                name: file_name(&key).to_owned(),
                parent: parent_of(&key).to_owned(),
                deleted_at: latest.last_modified.clone(),
                marker_version: latest.version_id.clone(),
                restorable: newest.is_some(),
                size: newest.map(|n| n.size),
                versions: list.iter().filter(|v| !v.delete_marker).count() as u32,
                key,
            })
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

pub async fn deleted(
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    prefix: String,
) -> Result<DeletedListing, CommandError> {
    validate_prefix(&prefix)?;
    let store = store_for(&accounts, &id, &bucket).await?;
    let mut all = Vec::new();
    let mut marker: Option<(String, Option<String>)> = None;
    let mut truncated = false;
    loop {
        let page = store
            .list_versions_page(
                &bucket,
                &prefix,
                marker.as_ref().map(|(k, v)| (k.as_str(), v.as_deref())),
                1000,
            )
            .await
            .map_err(|e| match e {
                StorageError::Provider { status: 501, .. } => CommandError::new(
                    ErrorKind::Provider,
                    "this storage does not keep versions, so deleted files cannot be listed",
                ),
                other => super::browser::listing_error(&other),
            })?;
        all.extend(page.entries);
        match page.next {
            Some(n) if all.len() < DELETED_MAX_ENTRIES => marker = Some(n),
            Some(_) => {
                truncated = true;
                break;
            }
            None => break,
        }
    }
    // A key cut off by the limit may look deleted without its versions:
    // only keys whose entries were all read count.
    if truncated && let Some(last) = all.last().map(|v| v.key.clone()) {
        all.retain(|v| v.key != last);
    }
    Ok(DeletedListing {
        entries: deleted_entries(all),
        truncated,
    })
}

// -------------------------------------------------------------- share link

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLink {
    pub url: String,
    /// RFC 3339.
    pub expires_at: String,
    pub expires_in: u64,
}

pub async fn share(
    accounts: Arc<CloudAccounts>,
    id: String,
    bucket: String,
    key: String,
    expires_in: u64,
) -> Result<ShareLink, CommandError> {
    check_key(&key)?;
    if !LINK_LIFETIMES.contains(&expires_in) {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "a link lasts 1 hour, 1 day or 7 days",
        ));
    }
    let store = store_for(&accounts, &id, &bucket).await?;
    if store
        .object_properties(&bucket, &key)
        .await
        .map_err(|e| operation_error(&e))?
        .is_none()
    {
        return Err(CommandError::new(
            ErrorKind::ObjectNotFound,
            "the file no longer exists",
        ));
    }
    let now = SystemTime::now();
    let url = store
        .presign_get(&bucket, &key, Duration::from_secs(expires_in))
        .await
        .map_err(|e| operation_error(&e))?;
    let expires_at = aws_sdk_s3::primitives::DateTime::from(now + Duration::from_secs(expires_in))
        .fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime)
        .unwrap_or_default();
    Ok(ShareLink {
        url,
        expires_at,
        expires_in,
    })
}

// ---------------------------------------------------------------- commands

/// Creates an empty folder (its marker object) in the folder shown.
/// Allowed by `allow-create-cloud-folder`.
#[tauri::command]
pub async fn create_cloud_folder(
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    prefix: String,
    name: String,
) -> Result<FolderEntry, CommandError> {
    logged(
        "create a folder",
        create_folder(Arc::clone(&accounts), id, bucket, prefix, name).await,
    )
}

/// A file's properties (`HeadObject`). Allowed by
/// `allow-cloud-object-properties`.
#[tauri::command]
pub async fn cloud_object_properties(
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    key: String,
) -> Result<Properties, CommandError> {
    logged(
        "read a file's properties",
        properties(Arc::clone(&accounts), id, bucket, key).await,
    )
}

/// A size-limited preview of a text or image file. Allowed by
/// `allow-preview-cloud-file`.
#[tauri::command]
pub async fn preview_cloud_file(
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    key: String,
) -> Result<Preview, CommandError> {
    logged(
        "preview a file",
        preview(Arc::clone(&accounts), id, bucket, key).await,
    )
}

/// Searches a folder recursively. Allowed by `allow-search-cloud-folder`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn search_cloud_folder(
    accounts: State<'_, Arc<CloudAccounts>>,
    searches: State<'_, Arc<CloudSearches>>,
    id: String,
    bucket: String,
    prefix: String,
    query: String,
    mode: SearchMode,
    cursor: Option<String>,
) -> Result<SearchResults, CommandError> {
    logged(
        "search a folder",
        search(
            Arc::clone(&accounts),
            Arc::clone(&searches),
            id,
            bucket,
            prefix,
            query,
            mode,
            cursor,
        )
        .await,
    )
}

/// The deleted files under a folder. Allowed by `allow-list-cloud-deleted`.
#[tauri::command]
pub async fn list_cloud_deleted(
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    prefix: String,
) -> Result<DeletedListing, CommandError> {
    logged(
        "list deleted files",
        deleted(Arc::clone(&accounts), id, bucket, prefix).await,
    )
}

/// A presigned download link. Allowed by `allow-share-cloud-file`.
#[tauri::command]
pub async fn share_cloud_file(
    accounts: State<'_, Arc<CloudAccounts>>,
    id: String,
    bucket: String,
    key: String,
    expires_in: u64,
) -> Result<ShareLink, CommandError> {
    logged(
        "create a share link",
        share(Arc::clone(&accounts), id, bucket, key, expires_in).await,
    )
}

#[cfg(test)]
mod tests;

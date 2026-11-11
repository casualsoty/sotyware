//! Read-only browsing of an account's bucket, one folder level at a time
//! (ADR-019): the `list_cloud_folder` command, the shapes it returns, and the
//! rules that turn S3 keys into folders and files.
//!
//! S3 has no folders. A "folder" is a key prefix ending in `/`; listing with
//! the delimiter `/` returns the objects directly under a prefix and the
//! prefixes one level deeper. So:
//!
//! - The root is the empty prefix; every other folder prefix ends in `/` and
//!   is taken **exactly** as the provider returned it. The page navigates by
//!   these prefixes and never builds one by joining names, so `a//b/` or a
//!   folder named ` ` stays what it is and no separator is added or dropped.
//! - Keys are never treated as local paths: `..`, `.`, `\` and leading `/`
//!   are ordinary characters of a key. Every request goes to the account's
//!   configured bucket, which the page cannot choose.
//! - An object whose key equals the folder's prefix is the folder's marker
//!   (what consoles create for an "empty folder"); it is not listed as a
//!   file. A folder that exists only as a marker shows up in its parent and
//!   is empty when opened.
//! - One page per request ([`LIST_PAGE_SIZE`] entries). The provider's
//!   continuation token never reaches the page: it gets a cursor id that is
//!   valid only for the same account (unchanged since), bucket and prefix,
//!   and only until the next page has been loaded with it.
//!
//! [`LIST_PAGE_SIZE`]: super::dashboard::LIST_PAGE_SIZE

use std::collections::HashSet;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::s3::{FOLDER_DELIMITER, FolderPage, StorageError};

/// Longest prefix S3 accepts (the longest key), in bytes.
const MAX_PREFIX_BYTES: usize = 1024;

/// One step of the path from the bucket's root to the current folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Breadcrumb {
    /// The folder's own name; may be empty (a key such as `a//b`).
    pub name: String,
    /// The full prefix to list to open it.
    pub prefix: String,
}

/// A subfolder of the listed folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FolderEntry {
    /// The folder's name without the trailing `/`; may be empty.
    pub name: String,
    /// Its full prefix, ending in `/`, exactly as the provider returned it.
    pub prefix: String,
}

/// An object directly in the listed folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// The part of the key after the folder's prefix.
    pub name: String,
    /// The full object key, unchanged.
    pub key: String,
    pub size: u64,
    /// RFC 3339, if the provider gave one.
    pub last_modified: Option<String>,
}

/// One page of one folder, as `list_cloud_folder` returns it. Never contains
/// a credential or a provider token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub account_id: String,
    /// The account's configured bucket that was listed.
    pub bucket: String,
    /// The listed prefix: empty for the root, else ending in `/`.
    pub prefix: String,
    /// The parent folder's prefix; `None` at the root.
    pub parent: Option<String>,
    /// From the first folder below the root to the listed one; empty at the
    /// root.
    pub breadcrumbs: Vec<Breadcrumb>,
    pub folders: Vec<FolderEntry>,
    pub files: Vec<FileEntry>,
    /// The cursor for the next page, if the provider has more.
    pub next_page: Option<String>,
}

/// Refuses a prefix that cannot be a folder: not empty and not ending in
/// `/`, or longer than any key can be.
pub fn validate_prefix(prefix: &str) -> Result<(), CommandError> {
    if prefix.len() > MAX_PREFIX_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("a folder path may be at most {MAX_PREFIX_BYTES} bytes"),
        ));
    }
    if !prefix.is_empty() && !prefix.ends_with(FOLDER_DELIMITER) {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "a folder path must be empty (the bucket's root) or end in '/'",
        ));
    }
    Ok(())
}

/// The path to `prefix`, one entry per `/`. `a//b/` gives `a` (`a/`), an
/// empty name (`a//`) and `b` (`a//b/`).
pub fn breadcrumbs(prefix: &str) -> Vec<Breadcrumb> {
    let mut crumbs = Vec::new();
    let mut start = 0;
    for (i, _) in prefix.match_indices(FOLDER_DELIMITER) {
        crumbs.push(Breadcrumb {
            name: prefix[start..i].to_owned(),
            prefix: prefix[..=i].to_owned(),
        });
        start = i + 1;
    }
    crumbs
}

/// The folder containing `prefix`: `None` for the root, `""` for a folder
/// directly under it.
pub fn parent(prefix: &str) -> Option<String> {
    let crumbs = breadcrumbs(prefix);
    match crumbs.len() {
        0 => None,
        1 => Some(String::new()),
        n => Some(crumbs[n - 2].prefix.clone()),
    }
}

/// The immediate child folder of `prefix` that `path` (a key or a deeper
/// prefix under it) lies in, if it lies in one.
fn child_folder(prefix: &str, path: &str) -> Option<String> {
    let rest = path.strip_prefix(prefix)?;
    let end = rest.find(FOLDER_DELIMITER)?;
    Some(format!("{prefix}{}", &rest[..=end]))
}

/// Folders and files of one page of `prefix`, in the provider's order.
///
/// Anything the provider returned outside `prefix` is dropped. The folder's
/// own marker object is not a file. Should a provider ignore the delimiter
/// and return deeper keys or prefixes, they are folded into their immediate
/// child folder, so nested content still appears only after opening it.
pub fn entries(prefix: &str, page: FolderPage) -> (Vec<FolderEntry>, Vec<FileEntry>) {
    let mut seen = HashSet::new();
    let mut folders = Vec::new();
    let mut add_folder = |folder: String| {
        if seen.insert(folder.clone()) {
            let name = folder[prefix.len()..folder.len() - FOLDER_DELIMITER.len()].to_owned();
            folders.push(FolderEntry {
                name,
                prefix: folder,
            });
        }
    };
    for p in &page.prefixes {
        if let Some(folder) = child_folder(prefix, p) {
            add_folder(folder);
        }
    }
    let mut files = Vec::new();
    for object in page.objects {
        let Some(rest) = object.key.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue; // the folder's marker
        }
        if let Some(folder) = child_folder(prefix, &object.key) {
            add_folder(folder);
            continue;
        }
        files.push(FileEntry {
            name: rest.to_owned(),
            size: object.size,
            last_modified: object.last_modified,
            key: object.key,
        });
    }
    (folders, files)
}

/// How a failed listing is shown. The message is sotyware's own sentence:
/// the provider's error text is logged, never shown (it is not translated,
/// and may echo request details).
pub fn listing_error(error: &StorageError) -> CommandError {
    let (kind, message) = match error {
        StorageError::InvalidCredentials(_) => (
            ErrorKind::InvalidCredentials,
            "the provider does not accept this key ID or secret".to_owned(),
        ),
        StorageError::AccessDenied(_) => (
            ErrorKind::AccessDenied,
            "the provider refused to list this folder with this key".to_owned(),
        ),
        // Backblaze answers 404 also for a bucket the key may not see, so
        // this does not claim that the bucket does not exist.
        StorageError::NotFound(_) => (
            ErrorKind::BucketUnavailable,
            "the bucket was not found at this endpoint and region, or this key may not see it"
                .to_owned(),
        ),
        StorageError::Network(_) => (
            ErrorKind::Network,
            "the provider could not be reached".to_owned(),
        ),
        StorageError::Timeout(_) => (
            ErrorKind::Timeout,
            "the provider did not answer in time".to_owned(),
        ),
        // Only sotyware's own checks (bucket not configured, prefix length).
        StorageError::Refused(m) => (ErrorKind::InvalidConfiguration, m.clone()),
        StorageError::Provider { status, code, .. } => {
            let code = if !code.is_empty()
                && code.len() <= 64
                && code.chars().all(|c| c.is_ascii_alphanumeric())
            {
                format!(" ({code})")
            } else {
                String::new()
            };
            let message = if *status == 0 {
                "the provider's answer could not be understood".to_owned()
            } else {
                format!("the provider refused the listing: HTTP {status}{code}")
            };
            (ErrorKind::Provider, message)
        }
        _ => (
            ErrorKind::Other,
            "the folder could not be listed".to_owned(),
        ),
    };
    CommandError::new(kind, message)
}

/// `list_cloud_folder` without Tauri. The account and its credentials are
/// read on the blocking pool, the request is sent on the async runtime, and
/// the result is accepted only if the account did not change meanwhile.
pub async fn list_folder(
    state: Arc<CloudAccounts>,
    id: String,
    prefix: String,
    page: Option<String>,
) -> Result<FolderListing, CommandError> {
    validate_prefix(&prefix)?;
    let prepared = {
        let state = Arc::clone(&state);
        let (id, prefix, page) = (id.clone(), prefix.clone(), page.clone());
        blocking(move || state.prepare_listing(&id, &prefix, page.as_deref())).await?
    };
    let result = prepared
        .store
        .list_folder(
            &prepared.bucket,
            &prefix,
            prepared.token.as_deref(),
            prepared.page_size,
        )
        .await;
    let folder_page = match result {
        Ok(p) => p,
        Err(e) => {
            eprintln!("sotyware: failed to list a cloud storage folder of account '{id}': {e}");
            // A 400 for a token the provider issued itself: it no longer
            // accepts it (expired, or the listing changed).
            if prepared.token.is_some() && matches!(e, StorageError::Provider { status: 400, .. }) {
                return Err(CommandError::new(
                    ErrorKind::OutdatedPage,
                    "the provider no longer accepts this page of the folder",
                ));
            }
            return Err(listing_error(&e));
        }
    };
    let next_token = folder_page.next_token.clone();
    let (folders, files) = entries(&prefix, folder_page);
    let bucket = prepared.bucket;
    let revision = prepared.revision;
    let next_page = {
        let (id, prefix, bucket) = (id.clone(), prefix.clone(), bucket.clone());
        blocking(move || {
            state.finish_listing(&id, revision, &bucket, &prefix, page.as_deref(), next_token)
        })
        .await?
    };
    Ok(FolderListing {
        account_id: id,
        bucket,
        parent: parent(&prefix),
        breadcrumbs: breadcrumbs(&prefix),
        prefix,
        folders,
        files,
        next_page,
    })
}

/// One page of one folder of the account's configured bucket: immediate
/// subfolders and files, never nested content, never credentials. `page` is
/// the `nextPage` of the previous page of the same folder, or `null` for the
/// first. Read-only: sends only `ListObjectsV2`. Allowed by
/// `allow-list-cloud-folder`.
#[tauri::command]
pub async fn list_cloud_folder(
    state: State<'_, Arc<CloudAccounts>>,
    id: String,
    prefix: String,
    page: Option<String>,
) -> Result<FolderListing, CommandError> {
    list_folder(Arc::clone(&state), id, prefix, page).await
}

#[cfg(test)]
mod tests;

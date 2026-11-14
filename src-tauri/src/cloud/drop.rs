//! Files and folders dropped on the dashboard (ADR-026). The window's
//! drag-and-drop event gives Rust their paths; they stay here. The page,
//! while it shows a bucket, subscribes with `watch_cloud_drops` and is told
//! only how many files and folders were dropped and their names; if the
//! user confirms, `upload_cloud_drop` uploads them into the folder shown as
//! one upload batch (`cloud::upload`, same rules: checked first, existing
//! objects skipped, never overwritten). A dropped folder keeps its
//! structure (as `upload_cloud_folder`). A drop is forgotten after
//! [`DROP_TTL`], when another drop replaces it, or once it was uploaded.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use super::dashboard::{CloudAccounts, CommandError, ErrorKind, blocking};
use super::upload::{
    CloudUploads, FolderFile, MAX_FOLDER_FILES, Target, UploadEvent, UploadSelection, UploadSink,
    check_folder_selection, folder_files, validate_prefix,
};

/// How long a drop may be uploaded after it happened.
pub const DROP_TTL: Duration = Duration::from_secs(120);

/// What the page is told about a drop: never a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropNotice {
    pub drop: u64,
    pub files: u32,
    pub folders: u32,
    /// The dropped items' own names (at most 20).
    pub names: Vec<String>,
}

pub trait DropSink: Send + Sync + 'static {
    fn notify(&self, notice: DropNotice) -> bool;
}

impl DropSink for Channel<DropNotice> {
    fn notify(&self, notice: DropNotice) -> bool {
        self.send(notice).is_ok()
    }
}

struct Pending {
    id: u64,
    at: Instant,
    paths: Vec<PathBuf>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    pending: Option<Pending>,
    watcher: Option<Arc<dyn DropSink>>,
}

/// The latest drop and who to tell about it.
#[derive(Default)]
pub struct CloudDrops {
    inner: Mutex<Inner>,
}

impl CloudDrops {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The page that shows a bucket wants to hear about drops (replaces any
    /// earlier subscription).
    pub fn watch(&self, sink: Arc<dyn DropSink>) {
        self.lock().watcher = Some(sink);
    }

    /// No page wants drops any more.
    pub fn unwatch(&self) {
        let mut inner = self.lock();
        inner.watcher = None;
        inner.pending = None;
    }

    /// Called for the window's drop event. Without a watching page, the
    /// drop is ignored (nothing could be uploaded).
    pub fn dropped(&self, paths: Vec<PathBuf>) {
        let mut inner = self.lock();
        let Some(watcher) = inner.watcher.clone() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        inner.next += 1;
        let id = inner.next;
        let folders = paths.iter().filter(|p| p.is_dir()).count();
        let notice = DropNotice {
            drop: id,
            files: u32::try_from(paths.len() - folders).unwrap_or(u32::MAX),
            folders: u32::try_from(folders).unwrap_or(u32::MAX),
            names: paths
                .iter()
                .take(20)
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect(),
        };
        inner.pending = Some(Pending {
            id,
            at: Instant::now(),
            paths,
        });
        if !watcher.notify(notice) {
            inner.watcher = None;
            inner.pending = None;
        }
    }

    /// Takes drop `id` if it is still the latest and not too old.
    fn take(&self, id: u64) -> Option<Vec<PathBuf>> {
        let mut inner = self.lock();
        match inner.pending.take() {
            Some(p) if p.id == id && p.at.elapsed() <= DROP_TTL => Some(p.paths),
            other => {
                inner.pending = other.filter(|p| p.id != id);
                None
            }
        }
    }
}

/// The files of a drop, each with its key below the destination: a file by
/// its name, a folder with its structure. Links are skipped.
pub fn drop_files(paths: Vec<PathBuf>) -> Result<Vec<FolderFile>, CommandError> {
    let mut out = Vec::new();
    for p in paths {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            out.extend(folder_files(
                &p,
                MAX_FOLDER_FILES.saturating_sub(out.len()),
            )?);
        } else if meta.is_file() {
            let name = p.file_name().map(|n| n.to_os_string());
            match name.as_ref().and_then(|n| n.to_str()) {
                Some(n) => {
                    let n = n.to_owned();
                    out.push((p, Ok(n)));
                }
                None => {
                    let shown = name
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    out.push((p, Err(shown)));
                }
            }
        }
        if out.len() > MAX_FOLDER_FILES {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("more than {MAX_FOLDER_FILES} files were dropped; upload them in parts"),
            ));
        }
    }
    if out.is_empty() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "nothing that can be uploaded was dropped",
        ));
    }
    Ok(out)
}

/// `upload_cloud_drop` without Tauri.
pub async fn begin_drop_upload(
    drops: &CloudDrops,
    uploads: &Arc<CloudUploads>,
    accounts: Arc<CloudAccounts>,
    target: Target,
    drop: u64,
    sink: Arc<dyn UploadSink>,
) -> Result<UploadSelection, CommandError> {
    validate_prefix(&target.prefix)?;
    let paths = drops.take(drop).ok_or_else(|| {
        CommandError::new(
            ErrorKind::OutdatedPage,
            "this drop is too old or was replaced; drop the files again",
        )
    })?;
    let reservation = uploads.reserve()?;
    let (limits, options) = {
        let accounts = Arc::clone(&accounts);
        let (id, bucket) = (target.account.clone(), target.bucket.clone());
        blocking(move || accounts.upload_limits(&id, &bucket)).await?
    };
    let prefix = target.prefix.clone();
    let candidates = blocking(move || {
        Ok(check_folder_selection(
            &prefix,
            drop_files(paths)?,
            &limits,
            &options,
        ))
    })
    .await?;
    uploads.start(reservation, accounts, target, candidates, sink)
}

/// The browser wants to hear about drops while it shows a bucket. Allowed
/// by `allow-watch-cloud-drops`.
#[tauri::command]
pub fn watch_cloud_drops(drops: State<'_, Arc<CloudDrops>>, on_drop: Channel<DropNotice>) {
    drops.watch(Arc::new(on_drop));
}

/// The browser was left. Allowed by `allow-unwatch-cloud-drops`.
#[tauri::command]
pub fn unwatch_cloud_drops(drops: State<'_, Arc<CloudDrops>>) {
    drops.unwatch();
}

/// Uploads drop `drop` into folder `prefix` of the account's bucket (which
/// must still be `bucket`). Allowed by `allow-upload-cloud-drop`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn upload_cloud_drop(
    drops: State<'_, Arc<CloudDrops>>,
    accounts: State<'_, Arc<CloudAccounts>>,
    uploads: State<'_, Arc<CloudUploads>>,
    id: String,
    bucket: String,
    prefix: String,
    drop: u64,
    on_event: Channel<UploadEvent>,
) -> Result<UploadSelection, CommandError> {
    begin_drop_upload(
        &drops,
        &uploads,
        Arc::clone(&accounts),
        Target {
            account: id,
            bucket,
            prefix,
        },
        drop,
        Arc::new(on_event),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::TempDir;

    #[derive(Default)]
    struct Seen(Mutex<Vec<DropNotice>>);

    impl DropSink for Seen {
        fn notify(&self, notice: DropNotice) -> bool {
            self.0.lock().unwrap().push(notice);
            true
        }
    }

    #[test]
    fn a_drop_is_announced_without_paths_and_taken_once() {
        let dir = TempDir::new("drops");
        let file = dir.file("a.txt", 3, 1);
        let folder = dir.path("Album");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("sub/b.txt"), "b").unwrap();
        let drops = CloudDrops::default();
        // Nobody watching: ignored.
        drops.dropped(vec![file.clone()]);
        assert!(drops.take(1).is_none());
        let seen = Arc::new(Seen::default());
        drops.watch(Arc::clone(&seen) as Arc<dyn DropSink>);
        drops.dropped(vec![file.clone(), folder.clone()]);
        let notice = seen.0.lock().unwrap()[0].clone();
        assert_eq!((notice.files, notice.folders), (1, 1));
        assert_eq!(notice.names, vec!["a.txt", "Album"]);
        let json = serde_json::to_string(&notice).unwrap();
        assert!(
            !json.contains(&dir.0.to_string_lossy().into_owned()),
            "{json}"
        );
        assert!(drops.take(notice.drop + 1).is_none(), "another id");
        let paths = drops.take(notice.drop).unwrap();
        assert!(drops.take(notice.drop).is_none(), "taken once");
        let files = drop_files(paths).unwrap();
        let keys: Vec<_> = files.iter().map(|(_, n)| n.clone().unwrap()).collect();
        assert_eq!(keys, vec!["a.txt", "Album/sub/b.txt"]);
        drops.unwatch();
        drops.dropped(vec![file]);
        assert!(drops.take(notice.drop + 1).is_none());
    }

    #[test]
    fn a_drop_with_nothing_uploadable_is_refused() {
        let dir = TempDir::new("drops-empty");
        let empty = dir.path("Empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(
            drop_files(vec![empty]).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
}

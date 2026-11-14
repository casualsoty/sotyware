//! Favorites and recent items of the file manager (ADR-026): local,
//! per-account bookmarks of object keys and folder prefixes, kept in
//! `cloud-places.sqlite3` (never as objects in the bucket). A place may
//! point at something deleted meanwhile; the page says so when it opens it.
//! Removing an account forgets its places.
//!
//! Commands: `list_cloud_places`, `set_cloud_favorite`,
//! `record_cloud_recent`, `clear_cloud_recent`.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::State;

use super::dashboard::{CommandError, ErrorKind, blocking};
use super::operations::{MAX_KEY_BYTES, file_name, folder_name, parent_of};
use crate::storage::{PlaceRow, SqlitePlacesStore};

/// Recent items kept per account.
pub const RECENT_KEEP: usize = 30;
/// Favorites per account.
pub const MAX_FAVORITES: usize = 200;

/// One favorite or recent item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// The file's key or the folder's prefix (ends in `/`).
    pub path: String,
    pub bucket: String,
    pub folder: bool,
    pub name: String,
    /// The folder it is in.
    pub parent: String,
    /// Milliseconds since 1970 (added, or last opened).
    pub touched: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Places {
    pub favorites: Vec<Place>,
    pub recent: Vec<Place>,
}

fn place(row: &PlaceRow) -> Place {
    let folder = row.path.ends_with('/');
    Place {
        name: if folder {
            folder_name(&row.path)
        } else {
            file_name(&row.path)
        }
        .to_owned(),
        parent: parent_of(&row.path).to_owned(),
        path: row.path.clone(),
        bucket: row.bucket.clone(),
        folder,
        touched: row.touched_ms,
    }
}

/// The places database, or why it cannot be used.
pub struct CloudPlaces {
    store: Mutex<Result<SqlitePlacesStore, String>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn check(path: &str, bucket: &str) -> Result<(), CommandError> {
    if path.is_empty() || path.len() > MAX_KEY_BYTES || bucket.is_empty() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "a place is a key or folder of 1 to 1,024 bytes",
        ));
    }
    Ok(())
}

impl CloudPlaces {
    pub fn load(store: Result<SqlitePlacesStore, crate::features::StoreError>) -> Self {
        Self {
            store: Mutex::new(store.map_err(|e| {
                let m = e.to_string();
                eprintln!("sotyware: favorites and recent items are not available: {m}");
                m
            })),
        }
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&mut SqlitePlacesStore) -> Result<T, crate::features::StoreError>,
    ) -> Result<T, CommandError> {
        let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Ok(store) => f(store).map_err(|e| CommandError::new(ErrorKind::Storage, e.to_string())),
            Err(m) => Err(CommandError::new(
                ErrorKind::Storage,
                format!("favorites and recent items are not available: {m}"),
            )),
        }
    }

    pub fn list(&self, account: &str) -> Result<Places, CommandError> {
        let rows = self.with(|s| s.list(account))?;
        let mut out = Places::default();
        for r in &rows {
            match r.kind.as_str() {
                "favorite" => out.favorites.push(place(r)),
                "recent" => out.recent.push(place(r)),
                _ => {}
            }
        }
        out.favorites.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.path.cmp(&b.path))
        });
        Ok(out)
    }

    pub fn set_favorite(
        &self,
        account: &str,
        bucket: &str,
        path: &str,
        favorite: bool,
    ) -> Result<Places, CommandError> {
        check(path, bucket)?;
        if favorite {
            let count = self.list(account)?.favorites.len();
            if count >= MAX_FAVORITES
                && !self.list(account)?.favorites.iter().any(|f| f.path == path)
            {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "at most 200 favorites per account",
                ));
            }
            self.with(|s| s.put(account, "favorite", path, bucket, now_ms(), usize::MAX))?;
        } else {
            self.with(|s| s.remove(account, "favorite", Some(path)))?;
        }
        self.list(account)
    }

    pub fn record_recent(
        &self,
        account: &str,
        bucket: &str,
        path: &str,
    ) -> Result<(), CommandError> {
        check(path, bucket)?;
        self.with(|s| s.put(account, "recent", path, bucket, now_ms(), RECENT_KEEP))
    }

    pub fn clear_recent(&self, account: &str) -> Result<Places, CommandError> {
        self.with(|s| s.remove(account, "recent", None))?;
        self.list(account)
    }

    /// Forgets the places of a removed account (best effort, logged).
    pub fn forget(&self, account: &str) {
        if let Err(e) = self.with(|s| s.forget(account)) {
            eprintln!(
                "sotyware: failed to forget the favorites of a removed account: {}",
                e.message
            );
        }
    }
}

/// The account's favorites and recent items. Allowed by
/// `allow-list-cloud-places`.
#[tauri::command]
pub async fn list_cloud_places(
    places: State<'_, Arc<CloudPlaces>>,
    id: String,
) -> Result<Places, CommandError> {
    let places = Arc::clone(&places);
    blocking(move || places.list(&id)).await
}

/// Marks or unmarks a file or folder as a favorite. Allowed by
/// `allow-set-cloud-favorite`.
#[tauri::command]
pub async fn set_cloud_favorite(
    places: State<'_, Arc<CloudPlaces>>,
    id: String,
    bucket: String,
    path: String,
    favorite: bool,
) -> Result<Places, CommandError> {
    let places = Arc::clone(&places);
    blocking(move || places.set_favorite(&id, &bucket, &path, favorite)).await
}

/// Remembers a file or folder the user opened. Allowed by
/// `allow-record-cloud-recent`.
#[tauri::command]
pub async fn record_cloud_recent(
    places: State<'_, Arc<CloudPlaces>>,
    id: String,
    bucket: String,
    path: String,
) -> Result<(), CommandError> {
    let places = Arc::clone(&places);
    blocking(move || places.record_recent(&id, &bucket, &path)).await
}

/// Forgets the account's recent items. Allowed by `allow-clear-cloud-recent`.
#[tauri::command]
pub async fn clear_cloud_recent(
    places: State<'_, Arc<CloudPlaces>>,
    id: String,
) -> Result<Places, CommandError> {
    let places = Arc::clone(&places);
    blocking(move || places.clear_recent(&id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::TempDir;

    fn open(dir: &TempDir) -> CloudPlaces {
        CloudPlaces::load(SqlitePlacesStore::open(&dir.path("places.sqlite3")))
    }

    #[test]
    fn favorites_and_recent_items_are_per_account_and_survive_a_restart() {
        let dir = TempDir::new("places");
        let p = open(&dir);
        p.set_favorite("a", "b", "photos/", true).unwrap();
        p.set_favorite("a", "b", "docs/x.pdf", true).unwrap();
        p.set_favorite("other", "b", "z", true).unwrap();
        for n in 0..40 {
            p.record_recent("a", "b", &format!("f{n:02}")).unwrap();
        }
        drop(p);
        let p = open(&dir);
        let l = p.list("a").unwrap();
        assert_eq!(
            l.favorites
                .iter()
                .map(|f| (f.name.as_str(), f.folder))
                .collect::<Vec<_>>(),
            vec![("photos", true), ("x.pdf", false)]
        );
        assert_eq!(l.favorites[1].parent, "docs/");
        assert_eq!(l.recent.len(), RECENT_KEEP);
        assert!(
            l.recent.iter().all(|r| r.path.as_str() >= "f10"),
            "the oldest went first"
        );
        let l = p.set_favorite("a", "b", "photos/", false).unwrap();
        assert_eq!(l.favorites.len(), 1);
        assert!(p.clear_recent("a").unwrap().recent.is_empty());
        p.forget("a");
        assert_eq!(p.list("a").unwrap(), Places::default());
        assert_eq!(p.list("other").unwrap().favorites.len(), 1);
        assert_eq!(
            p.set_favorite("a", "b", "", true).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn an_unusable_database_is_reported_not_fatal() {
        let p = CloudPlaces::load(Err(crate::features::StoreError::new("damaged")));
        assert_eq!(p.list("a").unwrap_err().kind, ErrorKind::Storage);
        p.forget("a");
    }
}

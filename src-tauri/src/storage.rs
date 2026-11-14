//! SQLite storage for feature preferences (ADR-015) and cloud storage account
//! configurations (ADR-016, ADR-018).
//!
//! One database file, `preferences.sqlite3`, in the app's local data
//! directory (`run()` resolves it). The schema version is SQLite's
//! `PRAGMA user_version`; [`MIGRATIONS`] lists the steps from each version to
//! the next. This module implements [`PreferenceStore`] and nothing else: what
//! a stored value means is decided in `features::preferences`.
//!
//! It never repairs, replaces or deletes a database it cannot use (corrupt,
//! newer schema, not a sotyware database): opening fails, sotyware runs with
//! the defaults, and the file stays as it was for the user to inspect.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::cloud::account::{AccountConfig, AccountStore};
use crate::features::{PreferenceStore, StoreError};

/// The database file inside the app's local data directory.
pub const FILE_NAME: &str = "preferences.sqlite3";

/// Schema migrations: entry `n` takes the database from version `n` to
/// `n + 1`. A new database (version 0) runs them all. Append only; never edit
/// a released entry.
pub const MIGRATIONS: &[&str] = &[
    // 1: one row per feature the user switched. A feature without a row uses
    // its `enabled_by_default`.
    "CREATE TABLE feature_preferences (
        feature_id TEXT PRIMARY KEY NOT NULL,
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
    ) STRICT;",
];

/// How long a statement waits for another connection's lock before failing
/// (only a second sotyware process could hold one, when single instance is
/// off). Bounded, so a save can never hang the switch.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

fn sql_error(context: &str) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
    move |e| StoreError::new(format!("{context}: {e}"))
}

/// Feature preferences in a SQLite database.
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens (creating it if needed) the database in `dir`, creating `dir`
    /// first if needed.
    pub fn open_in(dir: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(dir).map_err(|e| {
            StoreError::new(format!("cannot create the folder {}: {e}", dir.display()))
        })?;
        Self::open(&dir.join(FILE_NAME))
    }

    /// Opens (creating it if needed) the database at `path`, checks it and
    /// brings its schema to the current version.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_with(path, MIGRATIONS)
    }

    fn open_with(path: &Path, migrations: &[&str]) -> Result<Self, StoreError> {
        open_database(path, migrations).map(|conn| Self { conn })
    }
}

/// Opens (creating it if needed) the database at `path`, checks it and brings
/// its schema to `migrations.len()`. Shared by every store in this module.
fn open_database(path: &Path, migrations: &[&str]) -> Result<Connection, StoreError> {
    let shown = path.display().to_string();
    let opened = (|| {
        let mut conn = Connection::open(path).map_err(sql_error("cannot open"))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(sql_error("cannot configure"))?;
        check_integrity(&conn)?;
        migrate(&mut conn, migrations)?;
        Ok(conn)
    })();
    opened.map_err(|e: StoreError| StoreError::new(format!("{shown}: {e}")))
}

/// Fails for a file that is not an SQLite database or whose pages are damaged.
/// The database is tiny, so the check costs next to nothing at startup.
fn check_integrity(conn: &Connection) -> Result<(), StoreError> {
    let result: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(sql_error("cannot read"))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StoreError::new(format!(
            "the database is damaged: {result}"
        )))
    }
}

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// Brings the schema to `migrations.len()`. All pending steps and the new
/// version are one transaction: a failing step leaves the database exactly
/// as it was. A database from a newer sotyware is refused, not touched.
fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<(), StoreError> {
    let target = i64::try_from(migrations.len()).expect("few migrations");
    // Read-only when nothing is to do, so a read-only file still loads.
    let current = user_version(conn).map_err(sql_error("cannot read the schema version"))?;
    if current == target {
        return Ok(());
    }
    // IMMEDIATE: take the write lock now and read the version again under it,
    // so two processes cannot both run the same migration.
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error("cannot start the schema update"))?;
    let current = user_version(&tx).map_err(sql_error("cannot read the schema version"))?;
    let from = usize::try_from(current)
        .ok()
        .filter(|&v| v <= migrations.len())
        .ok_or_else(|| {
            StoreError::new(format!(
                "schema version {current} is not supported by this sotyware (newest: {target})"
            ))
        })?;
    for (step, sql) in migrations.iter().enumerate().skip(from) {
        tx.execute_batch(sql)
            .map_err(|e| StoreError::new(format!("schema update to version {}: {e}", step + 1)))?;
    }
    tx.pragma_update(None, "user_version", target)
        .map_err(sql_error("cannot set the schema version"))?;
    tx.commit()
        .map_err(sql_error("cannot commit the schema update"))
}

impl PreferenceStore for SqliteStore {
    fn load(&mut self) -> Result<Vec<(String, bool)>, StoreError> {
        let read = || -> rusqlite::Result<Vec<(String, bool)>> {
            let mut stmt = self.conn.prepare(
                "SELECT feature_id, enabled FROM feature_preferences ORDER BY feature_id",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        };
        read().map_err(sql_error("cannot read the feature preferences"))
    }

    /// One statement, so SQLite runs it as its own transaction: committed and
    /// durable when it returns, or not applied at all.
    fn save(&mut self, id: &str, enabled: bool) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO feature_preferences (feature_id, enabled) VALUES (?1, ?2)
                 ON CONFLICT (feature_id) DO UPDATE SET enabled = excluded.enabled",
                params![id, enabled],
            )
            .map(drop)
            .map_err(sql_error("cannot save"))
    }
}

/// The store `run()` uses: [`FILE_NAME`] in `dir`, the app's local data
/// directory, or the reason it cannot be used.
pub fn open_feature_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn PreferenceStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    Ok(Box::new(SqliteStore::open_in(&dir)?))
}

/// The cloud storage account database, separate from the preferences so the
/// two schemas evolve independently (ADR-016, kept so by ADR-018).
pub const ACCOUNTS_FILE_NAME: &str = "cloud-accounts.sqlite3";

/// Schema of [`ACCOUNTS_FILE_NAME`]; append only, like [`MIGRATIONS`]. There
/// is deliberately no column for a secret: credentials are looked up by
/// account id elsewhere.
pub const ACCOUNT_MIGRATIONS: &[&str] = &[
    // 1: one row per account, its buckets in a second table.
    "CREATE TABLE storage_accounts (
        account_id TEXT PRIMARY KEY NOT NULL,
        label TEXT NOT NULL,
        provider TEXT NOT NULL,
        region TEXT NOT NULL,
        endpoint TEXT,
        path_style INTEGER CHECK (path_style IN (0, 1)),
        max_concurrent_transfers INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE storage_account_buckets (
        account_id TEXT NOT NULL,
        position INTEGER NOT NULL,
        bucket TEXT NOT NULL,
        PRIMARY KEY (account_id, position)
    ) STRICT;",
];

/// Cloud storage account configurations in a SQLite database.
pub struct SqliteAccountStore {
    conn: Connection,
}

impl SqliteAccountStore {
    /// Opens (creating it if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, ACCOUNT_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The account store `run()` uses: [`ACCOUNTS_FILE_NAME`] in `dir`, the app's
/// local data directory (created if needed), or the reason it cannot be used.
pub fn open_account_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn AccountStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteAccountStore::open(
        &dir.join(ACCOUNTS_FILE_NAME),
    )?))
}

impl AccountStore for SqliteAccountStore {
    fn load(&mut self) -> Result<Vec<AccountConfig>, StoreError> {
        let read = || -> rusqlite::Result<Vec<AccountConfig>> {
            let mut stmt = self.conn.prepare(
                "SELECT account_id, label, provider, region, endpoint, path_style,
                        max_concurrent_transfers
                 FROM storage_accounts ORDER BY account_id",
            )?;
            let mut accounts = stmt
                .query_map([], |row| {
                    let transfers: i64 = row.get(6)?;
                    Ok(AccountConfig {
                        id: row.get(0)?,
                        label: row.get(1)?,
                        provider: row.get(2)?,
                        region: row.get(3)?,
                        endpoint: row.get(4)?,
                        path_style: row.get(5)?,
                        buckets: Vec::new(),
                        // Out-of-range values become 0, which `resolve` refuses.
                        max_concurrent_transfers: u8::try_from(transfers).unwrap_or(0),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut buckets = self.conn.prepare(
                "SELECT bucket FROM storage_account_buckets
                 WHERE account_id = ?1 ORDER BY position",
            )?;
            for account in &mut accounts {
                account.buckets = buckets
                    .query_map([&account.id], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
            }
            Ok(accounts)
        };
        read().map_err(sql_error("cannot read the storage accounts"))
    }

    /// One transaction: the account row and its buckets change together.
    fn save(&mut self, config: &AccountConfig) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            tx.execute(
                "INSERT INTO storage_accounts (account_id, label, provider, region, endpoint,
                     path_style, max_concurrent_transfers)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (account_id) DO UPDATE SET label = excluded.label,
                     provider = excluded.provider, region = excluded.region,
                     endpoint = excluded.endpoint, path_style = excluded.path_style,
                     max_concurrent_transfers = excluded.max_concurrent_transfers",
                params![
                    config.id,
                    config.label,
                    config.provider,
                    config.region,
                    config.endpoint,
                    config.path_style,
                    config.max_concurrent_transfers
                ],
            )?;
            tx.execute(
                "DELETE FROM storage_account_buckets WHERE account_id = ?1",
                [&config.id],
            )?;
            for (position, bucket) in config.buckets.iter().enumerate() {
                tx.execute(
                    "INSERT INTO storage_account_buckets (account_id, position, bucket)
                     VALUES (?1, ?2, ?3)",
                    params![
                        config.id,
                        i64::try_from(position).unwrap_or(i64::MAX),
                        bucket
                    ],
                )?;
            }
            tx.commit()
        };
        write().map_err(sql_error("cannot save the storage account"))
    }

    fn remove(&mut self, id: &str) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            tx.execute(
                "DELETE FROM storage_account_buckets WHERE account_id = ?1",
                [id],
            )?;
            tx.execute("DELETE FROM storage_accounts WHERE account_id = ?1", [id])?;
            tx.commit()
        };
        write().map_err(sql_error("cannot remove the storage account"))
    }
}

/// The file manager's local places (favorites and recent items, ADR-026),
/// separate from the accounts so the two schemas evolve independently.
pub const PLACES_FILE_NAME: &str = "cloud-places.sqlite3";

/// Schema of [`PLACES_FILE_NAME`]; append only. Only keys, prefixes and
/// times: no object content, no credential.
pub const PLACES_MIGRATIONS: &[&str] = &[
    // 1: one row per account, kind and key (or folder prefix).
    "CREATE TABLE places (
        account_id TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('favorite', 'recent')),
        path TEXT NOT NULL,
        bucket TEXT NOT NULL,
        touched_ms INTEGER NOT NULL,
        PRIMARY KEY (account_id, kind, path)
    ) STRICT;",
];

/// One stored place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRow {
    pub kind: String,
    pub path: String,
    pub bucket: String,
    pub touched_ms: i64,
}

/// Favorites and recent items in a SQLite database.
pub struct SqlitePlacesStore {
    conn: Connection,
}

impl SqlitePlacesStore {
    /// Opens (creating it if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, PLACES_MIGRATIONS).map(|conn| Self { conn })
    }

    /// Every place of `account`, newest first.
    pub fn list(&self, account: &str) -> Result<Vec<PlaceRow>, StoreError> {
        let read = || -> rusqlite::Result<Vec<PlaceRow>> {
            let mut stmt = self.conn.prepare(
                "SELECT kind, path, bucket, touched_ms FROM places
                 WHERE account_id = ?1 ORDER BY touched_ms DESC, path",
            )?;
            stmt.query_map([account], |r| {
                Ok(PlaceRow {
                    kind: r.get(0)?,
                    path: r.get(1)?,
                    bucket: r.get(2)?,
                    touched_ms: r.get(3)?,
                })
            })?
            .collect()
        };
        read().map_err(sql_error("cannot read the places"))
    }

    /// Adds or refreshes a place; for `recent`, keeps only the `keep`
    /// newest of the account.
    pub fn put(
        &mut self,
        account: &str,
        kind: &str,
        path: &str,
        bucket: &str,
        now_ms: i64,
        keep: usize,
    ) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            tx.execute(
                "INSERT INTO places (account_id, kind, path, bucket, touched_ms) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account_id, kind, path) DO UPDATE SET bucket = excluded.bucket,
                     touched_ms = excluded.touched_ms",
                params![account, kind, path, bucket, now_ms],
            )?;
            if kind == "recent" {
                tx.execute(
                    "DELETE FROM places WHERE account_id = ?1 AND kind = 'recent' AND path NOT IN (
                         SELECT path FROM places WHERE account_id = ?1 AND kind = 'recent'
                         ORDER BY touched_ms DESC, path LIMIT ?2)",
                    params![account, i64::try_from(keep).unwrap_or(i64::MAX)],
                )?;
            }
            tx.commit()
        };
        write().map_err(sql_error("cannot save the place"))
    }

    /// Removes one place (`path`) or, without one, every place of `kind`.
    pub fn remove(
        &mut self,
        account: &str,
        kind: &str,
        path: Option<&str>,
    ) -> Result<(), StoreError> {
        let r = match path {
            Some(p) => self.conn.execute(
                "DELETE FROM places WHERE account_id = ?1 AND kind = ?2 AND path = ?3",
                params![account, kind, p],
            ),
            None => self.conn.execute(
                "DELETE FROM places WHERE account_id = ?1 AND kind = ?2",
                params![account, kind],
            ),
        };
        r.map(drop).map_err(sql_error("cannot remove the place"))
    }

    /// Forgets everything of `account` (it was removed).
    pub fn forget(&mut self, account: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM places WHERE account_id = ?1", [account])
            .map(drop)
            .map_err(sql_error("cannot remove the places"))
    }
}

/// The places store `run()` uses: [`PLACES_FILE_NAME`] in `dir`.
pub fn open_places_store(dir: tauri::Result<PathBuf>) -> Result<SqlitePlacesStore, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    SqlitePlacesStore::open(&dir.join(PLACES_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::features::{self, FeaturePreferences, FeatureRegistry, FeatureState};

    /// A fresh directory under the system temp dir, removed on drop.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "sotyware-test-{}-{}-{name}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn db(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn version(path: &Path) -> i64 {
        user_version(&Connection::open(path).unwrap()).unwrap()
    }

    fn tables(path: &Path) -> Vec<String> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn first_open_creates_the_folder_the_file_and_the_current_schema() {
        let dir = TestDir::new("first");
        let nested = dir.0.join("com.sotyware.desktop");
        let mut store = SqliteStore::open_in(&nested).unwrap();
        assert_eq!(store.load().unwrap(), []);
        assert!(nested.join(FILE_NAME).is_file());
        assert_eq!(version(&nested.join(FILE_NAME)), 1);
        assert_eq!(tables(&nested.join(FILE_NAME)), ["feature_preferences"]);
    }

    #[test]
    fn saves_are_read_back_and_overwritten() {
        let dir = TestDir::new("save");
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        store.save("example", false).unwrap();
        store.save("other", true).unwrap();
        store.save("example", true).unwrap();
        assert_eq!(
            store.load().unwrap(),
            [("example".to_owned(), true), ("other".to_owned(), true)]
        );
    }

    #[test]
    fn saves_survive_closing_and_reopening() {
        let dir = TestDir::new("reopen");
        {
            let mut store = SqliteStore::open(&dir.db()).unwrap();
            store.save("example", false).unwrap();
        }
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(store.load().unwrap(), [("example".to_owned(), false)]);
    }

    #[test]
    fn unknown_ids_are_stored_and_loaded_like_any_other() {
        // Filtering is the preferences layer's job; the store keeps everything.
        let dir = TestDir::new("unknown");
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        store.save("removed-feature", false).unwrap();
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.load().unwrap(),
            [("removed-feature".to_owned(), false)]
        );
    }

    #[test]
    fn schema_rejects_values_other_than_0_and_1() {
        let dir = TestDir::new("check");
        SqliteStore::open(&dir.db()).unwrap();
        let conn = Connection::open(dir.db()).unwrap();
        for bad in ["2", "'yes'", "NULL"] {
            assert!(
                conn.execute_batch(&format!(
                    "INSERT INTO feature_preferences VALUES ('x', {bad})"
                ))
                .is_err(),
                "{bad}"
            );
        }
    }

    // --- migrations ---------------------------------------------------------

    const V2: &str = "ALTER TABLE feature_preferences ADD COLUMN note TEXT";

    #[test]
    fn reopening_a_current_database_runs_no_migration() {
        let dir = TestDir::new("current");
        SqliteStore::open(&dir.db())
            .unwrap()
            .save("a", false)
            .unwrap();
        // Migration 1 would fail on an existing table if it ran again.
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(store.load().unwrap(), [("a".to_owned(), false)]);
        assert_eq!(version(&dir.db()), 1);
    }

    #[test]
    fn an_older_database_is_migrated_and_keeps_its_rows() {
        let dir = TestDir::new("migrate");
        SqliteStore::open(&dir.db())
            .unwrap()
            .save("a", false)
            .unwrap();
        let mut store = SqliteStore::open_with(&dir.db(), &[MIGRATIONS[0], V2]).unwrap();
        assert_eq!(version(&dir.db()), 2);
        assert_eq!(store.load().unwrap(), [("a".to_owned(), false)]);
        let conn = Connection::open(dir.db()).unwrap();
        let note: Option<String> = conn
            .query_row("SELECT note FROM feature_preferences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(note, None);
    }

    #[test]
    fn a_failing_migration_changes_nothing() {
        let dir = TestDir::new("rollback");
        let e = SqliteStore::open_with(&dir.db(), &[MIGRATIONS[0], "NOT SQL"])
            .err()
            .expect("fails");
        assert!(e.to_string().contains("schema update to version 2"), "{e}");
        // Step 1 ran inside the same transaction and was rolled back with it.
        assert_eq!(version(&dir.db()), 0);
        assert!(tables(&dir.db()).is_empty());
        // The real migrations still work on the untouched file.
        SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(version(&dir.db()), 1);
    }

    #[test]
    fn a_database_from_a_newer_sotyware_is_refused_and_left_alone() {
        let dir = TestDir::new("newer");
        SqliteStore::open(&dir.db())
            .unwrap()
            .save("a", false)
            .unwrap();
        Connection::open(dir.db())
            .unwrap()
            .pragma_update(None, "user_version", 7)
            .unwrap();
        let before = fs::read(dir.db()).unwrap();
        let e = SqliteStore::open(&dir.db()).err().expect("refused");
        assert!(
            e.to_string()
                .contains("schema version 7 is not supported by this sotyware (newest: 1)"),
            "{e}"
        );
        assert_eq!(fs::read(dir.db()).unwrap(), before);
    }

    // --- broken storage -----------------------------------------------------

    #[test]
    fn a_corrupt_file_is_refused_and_left_alone() {
        let dir = TestDir::new("corrupt");
        let garbage = b"this is not an SQLite database, just some text.\n".repeat(100);
        fs::write(dir.db(), &garbage).unwrap();
        let e = SqliteStore::open(&dir.db()).err().expect("refused");
        assert!(e.to_string().contains("not a database"), "{e}");
        assert!(e.to_string().contains(FILE_NAME), "names the file: {e}");
        assert_eq!(
            fs::read(dir.db()).unwrap(),
            garbage,
            "not repaired or replaced"
        );
    }

    #[test]
    fn a_damaged_database_is_refused() {
        let dir = TestDir::new("damaged");
        {
            let mut store = SqliteStore::open(&dir.db()).unwrap();
            for i in 0..500 {
                store.save(&format!("feature-{i}"), i % 2 == 0).unwrap();
            }
        }
        // Keep the valid header (first 100 bytes), overwrite the pages after it.
        let mut bytes = fs::read(dir.db()).unwrap();
        assert!(bytes.len() > 4096, "spans several pages");
        for b in &mut bytes[100..] {
            *b = 0xA5;
        }
        fs::write(dir.db(), &bytes).unwrap();
        assert!(SqliteStore::open(&dir.db()).is_err());
        assert_eq!(fs::read(dir.db()).unwrap(), bytes);
    }

    #[test]
    fn a_folder_that_cannot_be_created_is_reported() {
        let dir = TestDir::new("nodir");
        let file = dir.0.join("a-file");
        fs::write(&file, b"").unwrap();
        let e = SqliteStore::open_in(&file.join("data"))
            .err()
            .expect("fails");
        assert!(e.to_string().starts_with("cannot create the folder"), "{e}");
        assert!(
            open_feature_store(Err(tauri::Error::UnknownPath))
                .err()
                .expect("fails")
                .to_string()
                .starts_with("no app data folder")
        );
    }

    #[test]
    fn a_read_only_database_loads_but_saving_fails() {
        let dir = TestDir::new("readonly");
        SqliteStore::open(&dir.db())
            .unwrap()
            .save("a", false)
            .unwrap();
        let mut perms = fs::metadata(dir.db()).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(dir.db(), perms.clone()).unwrap();
        let result = SqliteStore::open(&dir.db()).and_then(|mut store| {
            let loaded = store.load()?;
            Ok((loaded, store.save("a", true)))
        });
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(dir.db(), perms).unwrap();
        let (loaded, saved) = result.unwrap();
        assert_eq!(loaded, [("a".to_owned(), false)]);
        assert!(saved.unwrap_err().to_string().starts_with("cannot save"));
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(store.load().unwrap(), [("a".to_owned(), false)]);
    }

    // --- concurrency --------------------------------------------------------

    #[test]
    fn a_save_blocked_past_the_busy_timeout_fails_and_changes_nothing() {
        let dir = TestDir::new("locked");
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        store.save("a", false).unwrap();
        // Another process holding the write lock for longer than the timeout.
        let mut other = Connection::open(dir.db()).unwrap();
        let tx = other
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let started = std::time::Instant::now();
        let result = store.save("a", true);
        let waited = started.elapsed();
        tx.rollback().unwrap();
        assert_eq!(
            result.unwrap_err().to_string(),
            "cannot save: database is locked"
        );
        assert!(waited >= BUSY_TIMEOUT, "gave up after {waited:?}");
        assert_eq!(store.load().unwrap(), [("a".to_owned(), false)]);
        // Once the lock is released, saving works again.
        store.save("a", true).unwrap();
        assert_eq!(store.load().unwrap(), [("a".to_owned(), true)]);
    }

    #[test]
    fn concurrent_connections_never_corrupt_the_database() {
        // As if single instance were off and two sotyware processes saved at
        // once: separate connections to one file, each from its own thread,
        // saving back to back. SQLite's locking is not fair, so under this
        // load a connection can wait longer than `BUSY_TIMEOUT` (seen on a
        // GitHub-hosted Windows runner). Such a save must fail cleanly with
        // "database is locked" and change nothing; every other save must be
        // committed. Which saves fail depends on timing, so the test checks
        // the stored values against the saves that actually succeeded.
        let dir = TestDir::new("concurrent");
        let path = dir.db();
        // Opened up front, so only the saves compete.
        let stores: Vec<_> = (0..4).map(|_| SqliteStore::open(&path).unwrap()).collect();
        let handles: Vec<_> = stores
            .into_iter()
            .enumerate()
            .map(|(t, mut store)| {
                thread::spawn(move || {
                    let check = |result: Result<(), StoreError>| match result {
                        Ok(()) => true,
                        Err(e) => {
                            assert_eq!(e.to_string(), "cannot save: database is locked");
                            false
                        }
                    };
                    let mut last_own = None;
                    let mut saved_shared = false;
                    for i in 0..100 {
                        if check(store.save(&format!("own-{t}"), i % 2 == 0)) {
                            last_own = Some(i % 2 == 0);
                        }
                        saved_shared |= check(store.save("shared", t % 2 == 0));
                    }
                    (last_own, saved_shared)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let conn = Connection::open(&path).unwrap();
        check_integrity(&conn).unwrap();
        let mut store = SqliteStore::open(&path).unwrap();
        let rows = store.load().unwrap();
        let mut expected = Vec::new();
        for (t, &(last_own, _)) in results.iter().enumerate() {
            if let Some(value) = last_own {
                expected.push((format!("own-{t}"), value));
            }
        }
        let shared = rows.iter().find(|(id, _)| id == "shared");
        let others: Vec<_> = rows
            .iter()
            .filter(|(id, _)| id != "shared")
            .cloned()
            .collect();
        // Each thread's own row holds its last successful save, or is absent
        // if none succeeded; nothing else was written.
        assert_eq!(others, expected, "{results:?}");
        // The shared row holds a value some thread saved successfully.
        let shared_savers: Vec<bool> = (0..4)
            .filter(|&t| results[t].1)
            .map(|t| t % 2 == 0)
            .collect();
        match shared {
            Some((_, value)) => assert!(shared_savers.contains(value), "{results:?}"),
            None => assert!(shared_savers.is_empty(), "{results:?}"),
        }
        // Whoever holds the lock always commits, so some saves succeed.
        assert!(!expected.is_empty() || !shared_savers.is_empty());
    }

    // --- with the real registry and built-in features -----------------------

    /// What `run()` does at startup, with the real store and built-in features.
    fn launch(dir: &TestDir) -> (FeatureRegistry, FeaturePreferences) {
        let registry = FeatureRegistry::with_features(features::builtin());
        let ids: Vec<_> = registry.list().iter().map(|i| i.id).collect();
        let prefs = FeaturePreferences::load(open_feature_store(Ok(dir.0.clone())), &ids);
        registry.start_all(|m| prefs.enabled_at_startup(m));
        (registry, prefs)
    }

    /// What `RunEvent::Exit` does.
    fn quit(registry: FeatureRegistry, prefs: FeaturePreferences) {
        registry.stop_all();
        prefs.close();
    }

    fn example(registry: &FeatureRegistry) -> FeatureState {
        registry.info("example").unwrap().state
    }

    #[test]
    fn the_example_feature_stays_switched_across_restarts() {
        let dir = TestDir::new("restart");
        let (registry, prefs) = launch(&dir);
        assert_eq!(
            example(&registry),
            FeatureState::Running,
            "default on first launch"
        );
        let change = features::switch_feature(&registry, &prefs, "example", false).unwrap();
        assert_eq!(change.save_error, None);
        quit(registry, prefs);

        let (registry, prefs) = launch(&dir);
        assert_eq!(
            example(&registry),
            FeatureState::Disabled,
            "stayed disabled"
        );
        let change = features::switch_feature(&registry, &prefs, "example", true).unwrap();
        assert_eq!(change.save_error, None);
        quit(registry, prefs);

        let (registry, prefs) = launch(&dir);
        assert_eq!(example(&registry), FeatureState::Running, "stayed enabled");
        quit(registry, prefs);
    }

    #[test]
    fn a_corrupt_database_means_defaults_reported_and_untouched() {
        let dir = TestDir::new("corrupt-launch");
        fs::write(dir.db(), b"garbage garbage garbage garbage garbage garbage").unwrap();
        let (registry, prefs) = launch(&dir);
        assert_eq!(example(&registry), FeatureState::Running);
        let error = prefs.load_error().expect("reported").to_string();
        assert!(error.contains("not a database"), "{error}");
        let change = features::switch_feature(&registry, &prefs, "example", false).unwrap();
        assert_eq!(example(&registry), FeatureState::Disabled);
        assert!(
            change
                .save_error
                .unwrap()
                .starts_with("preferences are unavailable")
        );
        quit(registry, prefs);
        assert_eq!(
            fs::read(dir.db()).unwrap(),
            b"garbage garbage garbage garbage garbage garbage"
        );
    }

    fn b2_account() -> AccountConfig {
        let mut c = AccountConfig::new("b2-photos", "Photos", "backblaze-b2", "eu-central-003");
        c.buckets = vec!["soty-photos".into(), "soty.archive".into()];
        c.path_style = Some(true);
        c
    }

    fn custom_account() -> AccountConfig {
        let mut c = AccountConfig::new("nas", "NAS", "s3-compatible", "us-east-1");
        c.endpoint = Some("https://nas.example.org:9000".into());
        c.max_concurrent_transfers = 2;
        c
    }

    #[test]
    fn storage_accounts_survive_a_restart_with_every_field() {
        let dir = TestDir::new("accounts");
        let path = dir.0.join(ACCOUNTS_FILE_NAME);
        {
            let mut store = SqliteAccountStore::open(&path).unwrap();
            assert!(store.load().unwrap().is_empty());
            store.save(&custom_account()).unwrap();
            store.save(&b2_account()).unwrap();
        }
        let mut store = SqliteAccountStore::open(&path).unwrap();
        assert_eq!(store.load().unwrap(), [b2_account(), custom_account()]);
    }

    #[test]
    fn saving_an_account_again_replaces_it_and_its_buckets() {
        let dir = TestDir::new("accounts-update");
        let mut store = SqliteAccountStore::open(&dir.0.join(ACCOUNTS_FILE_NAME)).unwrap();
        store.save(&b2_account()).unwrap();
        let mut changed = b2_account();
        changed.label = "Family photos".into();
        changed.buckets = vec!["soty-new".into()];
        changed.path_style = None;
        store.save(&changed).unwrap();
        assert_eq!(store.load().unwrap(), [changed]);
        store.remove("b2-photos").unwrap();
        store.remove("never-existed").unwrap();
        assert!(store.load().unwrap().is_empty());
        let orphans: i64 = store
            .conn
            .query_row("SELECT count(*) FROM storage_account_buckets", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(orphans, 0);
    }

    #[test]
    fn the_accounts_database_holds_no_secret() {
        use crate::cloud::account::Accounts;
        use crate::cloud::credentials::{
            CredentialStore, Credentials, MemoryCredentialStore, Secret,
        };
        use crate::cloud::s3::ClientOptions;
        use std::sync::Arc;

        let dir = TestDir::new("accounts-secret");
        let path = dir.0.join(ACCOUNTS_FILE_NAME);
        let store = SqliteAccountStore::open(&path).unwrap();
        let keys = Arc::new(MemoryCredentialStore::default());
        let accounts = Accounts::load(
            Box::new(store),
            Arc::clone(&keys) as Arc<dyn CredentialStore>,
            ClientOptions::default(),
        )
        .unwrap();
        let creds = Credentials {
            access_key_id: "0031234567890ab0000000001".into(),
            secret_access_key: Secret::new("K003-super-secret-application-key"),
        };
        accounts.add(b2_account(), &creds).unwrap();
        accounts.connect("b2-photos").unwrap();
        assert_eq!(keys.load("b2-photos").unwrap(), creds);
        drop(accounts);
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("soty-photos"), "the account must be stored");
        assert!(!text.contains("super-secret"));
        assert!(!text.contains("0031234567890ab"));
        let conn = Connection::open(&path).unwrap();
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('storage_accounts')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !columns
                .iter()
                .any(|c| c.contains("secret") || c.contains("key")),
            "{columns:?}"
        );
    }

    #[test]
    fn a_corrupt_accounts_database_is_refused_and_left_alone() {
        let dir = TestDir::new("accounts-corrupt");
        let path = dir.0.join(ACCOUNTS_FILE_NAME);
        fs::write(&path, b"not a database, not a database, not a database").unwrap();
        assert!(SqliteAccountStore::open(&path).is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            b"not a database, not a database, not a database"
        );
    }

    #[test]
    fn an_invalid_stored_concurrency_is_loaded_and_refused_when_used() {
        let dir = TestDir::new("accounts-range");
        let path = dir.0.join(ACCOUNTS_FILE_NAME);
        let mut store = SqliteAccountStore::open(&path).unwrap();
        store.save(&custom_account()).unwrap();
        store
            .conn
            .execute(
                "UPDATE storage_accounts SET max_concurrent_transfers = 999",
                [],
            )
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded[0].max_concurrent_transfers, 0);
        assert!(loaded[0].resolve().is_err());
    }

    #[test]
    fn the_preferences_database_is_not_changed_by_the_accounts_store() {
        // The proof of concept keeps its own file; the released preferences
        // schema stays at its version.
        assert_eq!(MIGRATIONS.len(), 1);
        assert_ne!(ACCOUNTS_FILE_NAME, FILE_NAME);
    }
}

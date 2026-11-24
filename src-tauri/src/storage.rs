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

use crate::backup::{BackupStore, RunRow};

mod automation;
mod folder_sync;
mod notes;
use crate::cloud::account::{AccountConfig, AccountStore};
use crate::features::{PreferenceStore, StoreError};
use crate::journal::{EntryRow, JournalStore, StepRow};
pub use automation::{AUTOMATION_FILE_NAME, SqliteAutomationStore};
pub use folder_sync::{IssueRow, SYNC_FILE_NAME, SqliteSyncStore};
pub use notes::{NOTES_FILE_NAME, SqliteNotesStore};

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
    /// newest of the account. The place is stored as newer than every other
    /// of its kind there: at `now_ms`, or one millisecond after the newest
    /// when `now_ms` is not later than that. So the order is the order of
    /// the calls, also when several share a millisecond or the clock went
    /// back.
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
                "INSERT INTO places (account_id, kind, path, bucket, touched_ms) VALUES (?1, ?2, ?3, ?4,
                     MAX(?5, (SELECT COALESCE(MAX(touched_ms), 0) + 1 FROM places
                              WHERE account_id = ?1 AND kind = ?2)))
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

/// Backup jobs and their run history (ADR-031), separate from the other
/// databases. Configurations are JSON (the job type evolves with serde
/// defaults); no credential is ever stored here.
pub const BACKUP_FILE_NAME: &str = "backup.sqlite3";

/// Schema of [`BACKUP_FILE_NAME`]; append only.
pub const BACKUP_MIGRATIONS: &[&str] = &[
    // 1: jobs, and every task run against them.
    "CREATE TABLE backup_jobs (
        id TEXT PRIMARY KEY NOT NULL,
        config TEXT NOT NULL,
        created_ms INTEGER NOT NULL,
        updated_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE backup_runs (
        id INTEGER PRIMARY KEY,
        job_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        state TEXT NOT NULL,
        started_ms INTEGER NOT NULL,
        finished_ms INTEGER,
        snapshot TEXT,
        suspicious INTEGER NOT NULL DEFAULT 0 CHECK (suspicious IN (0, 1)),
        passed INTEGER CHECK (passed IN (0, 1)),
        summary TEXT,
        error TEXT
    ) STRICT;
    CREATE INDEX backup_runs_by_job ON backup_runs (job_id, started_ms);",
];

/// Backup jobs and runs in a SQLite database.
pub struct SqliteBackupStore {
    conn: Connection,
}

impl SqliteBackupStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, BACKUP_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The backup store `run()` uses: [`BACKUP_FILE_NAME`] in `dir`.
pub fn open_backup_store(dir: tauri::Result<PathBuf>) -> Result<Box<dyn BackupStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteBackupStore::open(
        &dir.join(BACKUP_FILE_NAME),
    )?))
}

impl BackupStore for SqliteBackupStore {
    fn jobs(&mut self) -> Result<Vec<(String, String)>, StoreError> {
        let read = || -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = self
                .conn
                .prepare("SELECT id, config FROM backup_jobs ORDER BY id")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        };
        read().map_err(sql_error("cannot read the backup jobs"))
    }

    fn save_job(&mut self, id: &str, config: &str, now_ms: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO backup_jobs (id, config, created_ms, updated_ms) VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT (id) DO UPDATE SET config = excluded.config, updated_ms = excluded.updated_ms",
                params![id, config, now_ms],
            )
            .map(drop)
            .map_err(sql_error("cannot save the backup job"))
    }

    fn remove_job(&mut self, id: &str) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            tx.execute("DELETE FROM backup_runs WHERE job_id = ?1", [id])?;
            tx.execute("DELETE FROM backup_jobs WHERE id = ?1", [id])?;
            tx.commit()
        };
        write().map_err(sql_error("cannot remove the backup job"))
    }

    fn add_run(&mut self, run: &RunRow) -> Result<i64, StoreError> {
        self.conn
            .execute(
                "INSERT INTO backup_runs (job_id, kind, state, started_ms, finished_ms, snapshot,
                     suspicious, passed, summary, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    run.job_id,
                    run.kind,
                    run.state,
                    run.started_ms,
                    run.finished_ms,
                    run.snapshot,
                    run.suspicious,
                    run.passed,
                    run.summary,
                    run.error
                ],
            )
            .map(|_| self.conn.last_insert_rowid())
            .map_err(sql_error("cannot record the backup run"))
    }

    fn runs(&mut self, job_id: &str, limit: usize) -> Result<Vec<RunRow>, StoreError> {
        let read = || -> rusqlite::Result<Vec<RunRow>> {
            // In the order recorded: the rowid grows with each insert, the
            // wall-clock times in a row can step back (ADR-051).
            let mut stmt = self.conn.prepare(
                "SELECT id, job_id, kind, state, started_ms, finished_ms, snapshot, suspicious,
                        passed, summary, error
                 FROM backup_runs WHERE job_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            stmt.query_map(
                params![job_id, i64::try_from(limit).unwrap_or(i64::MAX)],
                |r| {
                    Ok(RunRow {
                        id: r.get(0)?,
                        job_id: r.get(1)?,
                        kind: r.get(2)?,
                        state: r.get(3)?,
                        started_ms: r.get(4)?,
                        finished_ms: r.get(5)?,
                        snapshot: r.get(6)?,
                        suspicious: r.get(7)?,
                        passed: r.get(8)?,
                        summary: r.get(9)?,
                        error: r.get(10)?,
                    })
                },
            )?
            .collect()
        };
        read().map_err(sql_error("cannot read the backup runs"))
    }
}

/// The sotyware account's local state (ADR-032): which account and device
/// this is, the highest vault revision seen, the last synced vault
/// (ciphertext) and sync times. No secret: tokens and keys are in the OS
/// credential store.
pub const ACCOUNT_STATE_FILE_NAME: &str = "account.sqlite3";

/// Schema of [`ACCOUNT_STATE_FILE_NAME`]; append only.
pub const ACCOUNT_STATE_MIGRATIONS: &[&str] = &[
    // 1: one row holding the state as JSON.
    "CREATE TABLE account_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        state TEXT NOT NULL
    ) STRICT;",
];

pub struct SqliteAccountState {
    conn: Connection,
}

impl SqliteAccountState {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, ACCOUNT_STATE_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The account state store `run()` uses: [`ACCOUNT_STATE_FILE_NAME`] in `dir`.
pub fn open_account_state(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::account::StateStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteAccountState::open(
        &dir.join(ACCOUNT_STATE_FILE_NAME),
    )?))
}

impl crate::account::StateStore for SqliteAccountState {
    fn load(&mut self) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT state FROM account_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error("cannot read the account state"))
    }

    fn save(&mut self, state: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO account_state (id, state) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET state = excluded.state",
                [state],
            )
            .map(drop)
            .map_err(sql_error("cannot save the account state"))
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM account_state", [])
            .map(drop)
            .map_err(sql_error("cannot clear the account state"))
    }
}

/// The update state (ADR-033): the version floor, the last check and a
/// postponed version, as one JSON row. No secret, no path.
pub const UPDATES_FILE_NAME: &str = "updates.sqlite3";

/// Schema of [`UPDATES_FILE_NAME`]; append only.
pub const UPDATES_MIGRATIONS: &[&str] = &["CREATE TABLE update_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        state TEXT NOT NULL
    ) STRICT;"];

pub struct SqliteUpdateState {
    conn: Connection,
}

impl SqliteUpdateState {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, UPDATES_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The update state store `run()` uses: [`UPDATES_FILE_NAME`] in `dir`.
pub fn open_update_state(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::account::StateStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteUpdateState::open(
        &dir.join(UPDATES_FILE_NAME),
    )?))
}

impl crate::account::StateStore for SqliteUpdateState {
    fn load(&mut self) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT state FROM update_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error("cannot read the update state"))
    }

    fn save(&mut self, state: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO update_state (id, state) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET state = excluded.state",
                [state],
            )
            .map(drop)
            .map_err(sql_error("cannot save the update state"))
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM update_state", [])
            .map(drop)
            .map_err(sql_error("cannot clear the update state"))
    }
}

/// Where the dashboard window was (ADR-040): its normal bounds, whether it
/// was maximized and the monitor scaling, as one JSON row
/// (`window_state::Saved`). Its own file: an older sotyware never opens it,
/// so adding it changes no schema an older version knows. No secret, no path.
pub const WINDOW_STATE_FILE_NAME: &str = "window-state.sqlite3";

/// Schema of [`WINDOW_STATE_FILE_NAME`]; append only.
pub const WINDOW_STATE_MIGRATIONS: &[&str] = &["CREATE TABLE window_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        state TEXT NOT NULL
    ) STRICT;"];

pub struct SqliteWindowState {
    conn: Connection,
}

impl SqliteWindowState {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, WINDOW_STATE_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The window state store `run()` uses: [`WINDOW_STATE_FILE_NAME`] in `dir`.
pub fn open_window_state(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::account::StateStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteWindowState::open(
        &dir.join(WINDOW_STATE_FILE_NAME),
    )?))
}

impl crate::account::StateStore for SqliteWindowState {
    fn load(&mut self) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT state FROM window_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error("cannot read the window placement"))
    }

    fn save(&mut self, state: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO window_state (id, state) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET state = excluded.state",
                [state],
            )
            .map(drop)
            .map_err(sql_error("cannot save the window placement"))
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM window_state", [])
            .map(drop)
            .map_err(sql_error("cannot clear the window placement"))
    }
}

/// Storage usage measurements (ADR-041): one row per measurement of an
/// account's bucket, its figures as JSON (`cloud::usage::Measurement`). No
/// secret, no path, no object key.
pub const USAGE_FILE_NAME: &str = "storage-usage.sqlite3";

/// Schema of [`USAGE_FILE_NAME`]; append only.
pub const USAGE_MIGRATIONS: &[&str] = &["CREATE TABLE measurements (
        id INTEGER PRIMARY KEY,
        account_id TEXT NOT NULL,
        measured_ms INTEGER NOT NULL,
        data TEXT NOT NULL
    ) STRICT;
    CREATE INDEX measurements_by_account ON measurements (account_id, measured_ms);"];

pub struct SqliteUsageStore {
    conn: Connection,
}

impl SqliteUsageStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, USAGE_MIGRATIONS).map(|conn| Self { conn })
    }
}

/// The usage store `run()` uses: [`USAGE_FILE_NAME`] in `dir`.
pub fn open_usage_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::cloud::usage::UsageStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteUsageStore::open(
        &dir.join(USAGE_FILE_NAME),
    )?))
}

impl crate::cloud::usage::UsageStore for SqliteUsageStore {
    /// Adds `m` and keeps the newest `usage::KEPT` of its account, in one
    /// transaction.
    fn add(&mut self, m: &crate::cloud::usage::Measurement) -> Result<(), StoreError> {
        let data = serde_json::to_string(m).map_err(|e| StoreError::new(e.to_string()))?;
        let keep = i64::try_from(crate::cloud::usage::KEPT).unwrap_or(i64::MAX);
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot save the measurement"))?;
        tx.execute(
            "INSERT INTO measurements (account_id, measured_ms, data) VALUES (?1, ?2, ?3)",
            params![m.account, m.measured_ms, data],
        )
        .map_err(sql_error("cannot save the measurement"))?;
        tx.execute(
            "DELETE FROM measurements WHERE account_id = ?1 AND id NOT IN (
                 SELECT id FROM measurements WHERE account_id = ?1
                 ORDER BY measured_ms DESC, id DESC LIMIT ?2)",
            params![m.account, keep],
        )
        .map_err(sql_error("cannot trim the measurements"))?;
        tx.commit()
            .map_err(sql_error("cannot save the measurement"))
    }

    fn history(
        &mut self,
        account: &str,
        limit: usize,
    ) -> Result<Vec<crate::cloud::usage::Measurement>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let read = || -> rusqlite::Result<Vec<String>> {
            let mut stmt = self.conn.prepare(
                "SELECT data FROM measurements WHERE account_id = ?1
                 ORDER BY measured_ms DESC, id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![account, limit], |r| r.get(0))?;
            rows.collect()
        };
        let rows = read().map_err(sql_error("cannot read the measurements"))?;
        // A row this version cannot read (written by a newer sotyware) is
        // skipped, not an error: the others are still worth showing.
        Ok(rows
            .iter()
            .filter_map(|text| serde_json::from_str(text).ok())
            .collect())
    }
}

/// The folder sync store `run()` uses (ADR-043): [`SYNC_FILE_NAME`] in
/// `dir`.
pub fn open_sync_store(dir: tauri::Result<PathBuf>) -> Result<SqliteSyncStore, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    SqliteSyncStore::open(&dir.join(SYNC_FILE_NAME))
}

/// The autoclicker's store `run()` uses (ADR-050): [`AUTOMATION_FILE_NAME`]
/// in `dir`.
pub fn open_automation_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::automation::AutomationStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteAutomationStore::open(
        &dir.join(AUTOMATION_FILE_NAME),
    )?))
}

/// The notes overlay's store `run()` uses (ADR-052): [`NOTES_FILE_NAME`]
/// in `dir`.
pub fn open_notes_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn crate::overlay::NotesStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteNotesStore::open(
        &dir.join(NOTES_FILE_NAME),
    )?))
}

/// The operation journal (ADR-030), separate from the other databases so
/// its schema evolves on its own. Local paths are stored here and nowhere
/// else; the file never leaves the device.
pub const JOURNAL_FILE_NAME: &str = "operations-journal.sqlite3";

/// Schema of [`JOURNAL_FILE_NAME`]; append only.
pub const JOURNAL_MIGRATIONS: &[&str] = &[
    // 1: entries, and the planned steps of file operations.
    "CREATE TABLE entries (
        id INTEGER PRIMARY KEY,
        kind TEXT NOT NULL CHECK (kind IN ('upload', 'download', 'staging', 'operation')),
        state TEXT NOT NULL,
        account_id TEXT,
        bucket TEXT,
        object_key TEXT,
        local_path TEXT,
        size INTEGER,
        sha256 TEXT,
        multipart INTEGER NOT NULL DEFAULT 0 CHECK (multipart IN (0, 1)),
        upload_id TEXT,
        operation TEXT,
        started_ms INTEGER NOT NULL,
        updated_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE steps (
        entry_id INTEGER NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
        seq INTEGER NOT NULL,
        source_key TEXT NOT NULL,
        source_etag TEXT,
        dest_key TEXT,
        phase TEXT NOT NULL,
        PRIMARY KEY (entry_id, seq)
    ) STRICT;",
];

/// The operation journal in a SQLite database.
pub struct SqliteJournal {
    conn: Connection,
}

impl SqliteJournal {
    /// Opens (creating it if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = open_database(path, JOURNAL_MIGRATIONS)?;
        // Steps are removed with their entry.
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(sql_error("cannot configure"))?;
        Ok(Self { conn })
    }
}

/// The journal `run()` uses: [`JOURNAL_FILE_NAME`] in `dir`.
pub fn open_journal_store(
    dir: tauri::Result<PathBuf>,
) -> Result<Box<dyn JournalStore>, StoreError> {
    let dir = dir.map_err(|e| StoreError::new(format!("no app data folder: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| StoreError::new(format!("cannot create the folder {}: {e}", dir.display())))?;
    Ok(Box::new(SqliteJournal::open(&dir.join(JOURNAL_FILE_NAME))?))
}

fn entry_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let size: Option<i64> = r.get(7)?;
    Ok(EntryRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        state: r.get(2)?,
        account_id: r.get(3)?,
        bucket: r.get(4)?,
        object_key: r.get(5)?,
        local_path: r.get(6)?,
        size: size.and_then(|s| u64::try_from(s).ok()),
        sha256: r.get(8)?,
        multipart: r.get(9)?,
        upload_id: r.get(10)?,
        operation: r.get(11)?,
        started_ms: r.get(12)?,
        updated_ms: r.get(13)?,
    })
}

impl JournalStore for SqliteJournal {
    fn insert(&mut self, e: &EntryRow) -> Result<i64, StoreError> {
        self.conn
            .execute(
                "INSERT INTO entries (kind, state, account_id, bucket, object_key, local_path,
                     size, sha256, multipart, upload_id, operation, started_ms, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    e.kind,
                    e.state,
                    e.account_id,
                    e.bucket,
                    e.object_key,
                    e.local_path,
                    e.size.map(|s| i64::try_from(s).unwrap_or(i64::MAX)),
                    e.sha256,
                    e.multipart,
                    e.upload_id,
                    e.operation,
                    e.started_ms,
                    e.updated_ms
                ],
            )
            .map(|_| self.conn.last_insert_rowid())
            .map_err(sql_error("cannot record in the operation journal"))
    }

    fn update(&mut self, e: &EntryRow) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE entries SET state = ?2, account_id = ?3, bucket = ?4, object_key = ?5,
                     local_path = ?6, size = ?7, sha256 = ?8, multipart = ?9, upload_id = ?10,
                     operation = ?11, updated_ms = ?12
                 WHERE id = ?1",
                params![
                    e.id,
                    e.state,
                    e.account_id,
                    e.bucket,
                    e.object_key,
                    e.local_path,
                    e.size.map(|s| i64::try_from(s).unwrap_or(i64::MAX)),
                    e.sha256,
                    e.multipart,
                    e.upload_id,
                    e.operation,
                    e.updated_ms
                ],
            )
            .map(drop)
            .map_err(sql_error("cannot update the operation journal"))
    }

    fn remove(&mut self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM entries WHERE id = ?1", [id])
            .map(drop)
            .map_err(sql_error("cannot update the operation journal"))
    }

    fn entries(&mut self) -> Result<Vec<EntryRow>, StoreError> {
        let read = || -> rusqlite::Result<Vec<EntryRow>> {
            let mut stmt = self.conn.prepare(
                "SELECT id, kind, state, account_id, bucket, object_key, local_path, size,
                        sha256, multipart, upload_id, operation, started_ms, updated_ms
                 FROM entries ORDER BY id",
            )?;
            stmt.query_map([], entry_from_row)?.collect()
        };
        read().map_err(sql_error("cannot read the operation journal"))
    }

    fn insert_steps(&mut self, id: i64, steps: &[StepRow]) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO steps (entry_id, seq, source_key, source_etag, dest_key, phase)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for s in steps {
                    stmt.execute(params![
                        id,
                        s.seq,
                        s.source_key,
                        s.source_etag,
                        s.dest_key,
                        s.phase
                    ])?;
                }
            }
            tx.commit()
        };
        write().map_err(sql_error("cannot record in the operation journal"))
    }

    fn set_phases(&mut self, id: i64, phases: &[(i64, &str)]) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            {
                let mut stmt =
                    tx.prepare("UPDATE steps SET phase = ?3 WHERE entry_id = ?1 AND seq = ?2")?;
                for (seq, phase) in phases {
                    stmt.execute(params![id, seq, phase])?;
                }
            }
            tx.commit()
        };
        write().map_err(sql_error("cannot update the operation journal"))
    }

    fn steps(&mut self, id: i64) -> Result<Vec<StepRow>, StoreError> {
        let read = || -> rusqlite::Result<Vec<StepRow>> {
            let mut stmt = self.conn.prepare(
                "SELECT seq, source_key, source_etag, dest_key, phase FROM steps
                 WHERE entry_id = ?1 ORDER BY seq",
            )?;
            stmt.query_map([id], |r| {
                Ok(StepRow {
                    seq: r.get(0)?,
                    source_key: r.get(1)?,
                    source_etag: r.get(2)?,
                    dest_key: r.get(3)?,
                    phase: r.get(4)?,
                })
            })?
            .collect()
        };
        read().map_err(sql_error("cannot read the operation journal"))
    }
}

/// Creates sotyware's data folder so that only the user can open it. On
/// Linux (and other Unix systems) the folder gets mode `0700`, and an
/// existing one with wider permissions is narrowed: the databases in it hold
/// no secret, but account names, bucket names, object keys and local paths
/// are personal. Only this folder is changed, never a parent, and a link in
/// its place is left alone. Windows: `%LOCALAPPDATA%` is already private to
/// the user by its ACL; the folder inherits it.
pub fn prepare_data_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let meta = fs::symlink_metadata(dir)?;
        if meta.is_dir() && meta.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)?;
    Ok(())
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
        store.save("backup-scheduler", false).unwrap();
        store.save("other", true).unwrap();
        store.save("backup-scheduler", true).unwrap();
        assert_eq!(
            store.load().unwrap(),
            [
                ("backup-scheduler".to_owned(), true),
                ("other".to_owned(), true)
            ]
        );
    }

    #[test]
    fn saves_survive_closing_and_reopening() {
        let dir = TestDir::new("reopen");
        {
            let mut store = SqliteStore::open(&dir.db()).unwrap();
            store.save("backup-scheduler", false).unwrap();
        }
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.load().unwrap(),
            [("backup-scheduler".to_owned(), false)]
        );
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

    /// The built-in feature these tests switch: it is on by default and,
    /// without the link `run()` sets, starting and stopping it do nothing.
    const SWITCHED: &str = "backup-scheduler";

    fn scheduler(registry: &FeatureRegistry) -> FeatureState {
        registry.info(SWITCHED).unwrap().state
    }

    #[test]
    fn a_feature_stays_switched_across_restarts() {
        let dir = TestDir::new("restart");
        let (registry, prefs) = launch(&dir);
        assert_eq!(
            scheduler(&registry),
            FeatureState::Running,
            "default on first launch"
        );
        let change = features::switch_feature(&registry, &prefs, SWITCHED, false).unwrap();
        assert_eq!(change.save_error, None);
        quit(registry, prefs);

        let (registry, prefs) = launch(&dir);
        assert_eq!(
            scheduler(&registry),
            FeatureState::Disabled,
            "stayed disabled"
        );
        let change = features::switch_feature(&registry, &prefs, SWITCHED, true).unwrap();
        assert_eq!(change.save_error, None);
        quit(registry, prefs);

        let (registry, prefs) = launch(&dir);
        assert_eq!(
            scheduler(&registry),
            FeatureState::Running,
            "stayed enabled"
        );
        quit(registry, prefs);
    }

    /// sotyware 0.1.0 had an `example` feature; a database that stored a
    /// choice for it must still load: the row is ignored (no such feature
    /// any more), the other features keep their stored choices, and the row
    /// stays in the file (nothing is migrated or deleted).
    #[test]
    fn a_choice_stored_for_the_removed_example_feature_is_ignored_and_kept() {
        let dir = TestDir::new("removed-example");
        {
            let mut store = SqliteStore::open(&dir.db()).unwrap();
            store.save("example", false).unwrap();
            store.save(SWITCHED, false).unwrap();
        }
        let (registry, prefs) = launch(&dir);
        assert_eq!(prefs.load_error().map(ToString::to_string), None);
        assert!(registry.info("example").is_none(), "no example feature");
        assert_eq!(
            scheduler(&registry),
            FeatureState::Disabled,
            "stored choice kept"
        );
        assert_eq!(
            registry.info("update-check").unwrap().state,
            FeatureState::Running,
            "default"
        );
        quit(registry, prefs);
        let mut store = SqliteStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.load().unwrap(),
            [(SWITCHED.to_owned(), false), ("example".to_owned(), false)]
        );
    }

    #[test]
    fn a_corrupt_database_means_defaults_reported_and_untouched() {
        let dir = TestDir::new("corrupt-launch");
        fs::write(dir.db(), b"garbage garbage garbage garbage garbage garbage").unwrap();
        let (registry, prefs) = launch(&dir);
        assert_eq!(scheduler(&registry), FeatureState::Running);
        let error = prefs.load_error().expect("reported").to_string();
        assert!(error.contains("not a database"), "{error}");
        let change = features::switch_feature(&registry, &prefs, SWITCHED, false).unwrap();
        assert_eq!(scheduler(&registry), FeatureState::Disabled);
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

    #[cfg(unix)]
    #[test]
    fn the_data_folder_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let base = TestDir::new("private");
        // Created private.
        let fresh = base.0.join("com.sotyware.desktop");
        prepare_data_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        // An existing folder from an older version is narrowed; its files
        // stay where they are.
        let old = base.0.join("old");
        fs::create_dir_all(&old).unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(old.join(FILE_NAME), b"x").unwrap();
        prepare_data_dir(&old).unwrap();
        assert_eq!(mode(&old), 0o700);
        assert!(old.join(FILE_NAME).exists());
        // The parent is not touched.
        assert_eq!(mode(&base.0) & 0o700, 0o700);
        let parent_before = mode(&base.0);
        prepare_data_dir(&fresh).unwrap();
        assert_eq!(mode(&base.0), parent_before);
        // A link in its place is not followed.
        let target = base.0.join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let link = base.0.join("linked");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        prepare_data_dir(&link).unwrap();
        assert_eq!(mode(&target), 0o755);
    }

    #[test]
    fn the_window_placement_is_one_row_in_its_own_file() {
        let dir = TestDir::new("window-state");
        {
            let mut store = open_window_state(Ok(dir.0.clone())).unwrap();
            assert_eq!(store.load().unwrap(), None);
            store.save(r#"{"format":1}"#).unwrap();
            store.save(r#"{"format":1,"x":3}"#).unwrap();
        }
        let path = dir.0.join(WINDOW_STATE_FILE_NAME);
        assert!(path.is_file());
        assert_eq!(version(&path), 1);
        assert_eq!(tables(&path), ["window_state"]);
        let mut store = open_window_state(Ok(dir.0.clone())).unwrap();
        assert_eq!(
            store.load().unwrap().as_deref(),
            Some(r#"{"format":1,"x":3}"#)
        );
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        // Not the preferences file: an older sotyware never sees its schema.
        assert!(!dir.0.join(FILE_NAME).exists());
    }

    #[test]
    fn usage_measurements_are_kept_newest_first_and_trimmed_per_account() {
        use crate::cloud::usage::{KEPT, Measurement};
        let dir = TestDir::new("usage");
        let m = |account: &str, at: i64| Measurement {
            account: account.into(),
            bucket: "b".into(),
            measured_ms: at,
            bytes: u64::try_from(at).unwrap(),
            ..Measurement::default()
        };
        {
            let mut store = open_usage_store(Ok(dir.0.clone())).unwrap();
            for at in 1..=(KEPT as i64 + 5) {
                store.add(&m("a", at)).unwrap();
            }
            store.add(&m("other", 7)).unwrap();
        }
        let path = dir.0.join(USAGE_FILE_NAME);
        assert_eq!(version(&path), 1);
        assert_eq!(tables(&path), ["measurements"]);
        let mut store = open_usage_store(Ok(dir.0.clone())).unwrap();
        let all = store.history("a", 1_000).unwrap();
        assert_eq!(all.len(), KEPT, "trimmed to the newest {KEPT}");
        assert_eq!(all.first().map(|m| m.measured_ms), Some(KEPT as i64 + 5));
        assert_eq!(all.last().map(|m| m.measured_ms), Some(6));
        assert_eq!(
            store
                .history("a", 3)
                .unwrap()
                .iter()
                .map(|m| m.measured_ms)
                .collect::<Vec<_>>(),
            [105, 104, 103]
        );
        assert_eq!(store.history("other", 10).unwrap(), [m("other", 7)]);
        assert_eq!(store.history("nobody", 10).unwrap(), []);
        // A row a newer sotyware wrote in another shape is skipped.
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO measurements (account_id, measured_ms, data) VALUES ('other', 9, '{\"unknown\":1}')",
            [],
        )
        .unwrap();
        assert_eq!(store.history("other", 10).unwrap(), [m("other", 7)]);
    }

    #[test]
    fn backup_runs_come_back_in_the_order_recorded_not_by_their_times() {
        let dir = TestDir::new("backup-runs");
        let path = dir.0.join(BACKUP_FILE_NAME);
        let run = |job: &str, kind: &str, started_ms: i64| RunRow {
            job_id: job.into(),
            kind: kind.into(),
            state: "completed".into(),
            started_ms,
            finished_ms: Some(started_ms + 10),
            ..RunRow::default()
        };
        let mut ids = Vec::new();
        {
            let mut store = SqliteBackupStore::open(&path).unwrap();
            // Each run reads an earlier time than the one before, as on a
            // clock that was set back between them.
            for (i, kind) in ["backup", "verify", "prune", "backup"]
                .into_iter()
                .enumerate()
            {
                ids.push(
                    store
                        .add_run(&run("photos", kind, 1_000 - i as i64 * 100))
                        .unwrap(),
                );
            }
            store.add_run(&run("other", "backup", 5_000)).unwrap();
        }
        assert!(ids.is_sorted(), "{ids:?}");
        let mut store = SqliteBackupStore::open(&path).unwrap();
        let read = |runs: Vec<RunRow>| -> Vec<(i64, String, i64)> {
            runs.into_iter()
                .map(|r| (r.id, r.kind, r.started_ms))
                .collect()
        };
        assert_eq!(
            read(store.runs("photos", 10).unwrap()),
            [
                (ids[3], "backup".to_owned(), 700),
                (ids[2], "prune".to_owned(), 800),
                (ids[1], "verify".to_owned(), 900),
                (ids[0], "backup".to_owned(), 1_000),
            ],
            "the last recorded first, whatever its time"
        );
        assert_eq!(
            read(store.runs("photos", 2).unwrap()),
            [
                (ids[3], "backup".to_owned(), 700),
                (ids[2], "prune".to_owned(), 800)
            ]
        );
        assert_eq!(store.runs("other", 10).unwrap().len(), 1);
    }
}

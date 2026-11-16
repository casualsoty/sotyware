//! Folder sync's database (ADR-043): `folder-sync.sqlite3`.
//!
//! Persistent: each sync's configuration (JSON), the **synced state** of
//! every path (what both sides last agreed on: the base of the three-way
//! comparison), per-path issues (failures with their retry time, skipped
//! names, conflicts, held deletions; JSON) and recent activity (JSON).
//!
//! Per session, in TEMP tables (rebuilt by the first pass after every
//! start, never written to the file): the last local scan and the last
//! complete remote listing. Planning is one query over the three tables in
//! path order that returns only the paths where they differ, so a large
//! bucket is never held in memory.
//!
//! Paths are compared as SQLite `TEXT` with the default `BINARY` collation
//! (byte order, which for UTF-8 is code point order). A folder's subtree
//! is the range `path >= 'a/' AND path < 'a0'` (`0` follows `/`), never
//! `LIKE`, whose `_` and `%` occur in file names.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{open_database, sql_error};
use crate::features::StoreError;
use crate::folder_sync::model::{Local, Remote, Synced, Triple};

/// The database file inside the app's local data directory.
pub const SYNC_FILE_NAME: &str = "folder-sync.sqlite3";

/// Schema of [`SYNC_FILE_NAME`]; append only.
pub const SYNC_MIGRATIONS: &[&str] = &[
    // 1: configurations, synced state, issues, activity.
    "CREATE TABLE syncs (
        id TEXT PRIMARY KEY NOT NULL,
        data TEXT NOT NULL
    ) STRICT;
    CREATE TABLE synced (
        sync_id TEXT NOT NULL,
        path TEXT NOT NULL,
        dir INTEGER NOT NULL CHECK (dir IN (0, 1)),
        size INTEGER NOT NULL,
        mtime_ns INTEGER,
        file_id INTEGER,
        sha256 TEXT,
        etag TEXT,
        remote_size INTEGER,
        synced_ms INTEGER NOT NULL,
        PRIMARY KEY (sync_id, path)
    ) STRICT, WITHOUT ROWID;
    CREATE INDEX synced_by_size ON synced (sync_id, size) WHERE dir = 0;
    CREATE INDEX synced_by_etag ON synced (sync_id, etag) WHERE etag IS NOT NULL;
    CREATE TABLE issues (
        sync_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        path TEXT NOT NULL,
        data TEXT NOT NULL,
        updated_ms INTEGER NOT NULL,
        PRIMARY KEY (sync_id, kind, path)
    ) STRICT, WITHOUT ROWID;
    CREATE TABLE activity (
        id INTEGER PRIMARY KEY,
        sync_id TEXT NOT NULL,
        at_ms INTEGER NOT NULL,
        data TEXT NOT NULL
    ) STRICT;
    CREATE INDEX activity_by_sync ON activity (sync_id, id);",
];

/// The per-session tables (TEMP: in the connection's temporary store).
const SESSION_TABLES: &str = "
    CREATE TEMP TABLE IF NOT EXISTS local_scan (
        sync_id TEXT NOT NULL,
        path TEXT NOT NULL,
        dir INTEGER NOT NULL,
        size INTEGER NOT NULL,
        mtime_ns INTEGER,
        file_id INTEGER,
        generation INTEGER NOT NULL,
        PRIMARY KEY (sync_id, path)
    ) WITHOUT ROWID;
    CREATE TEMP TABLE IF NOT EXISTS remote_scan (
        sync_id TEXT NOT NULL,
        path TEXT NOT NULL,
        dir INTEGER NOT NULL,
        marker INTEGER NOT NULL,
        size INTEGER NOT NULL,
        etag TEXT,
        modified_ms INTEGER,
        generation INTEGER NOT NULL,
        PRIMARY KEY (sync_id, path)
    ) WITHOUT ROWID;
    CREATE TEMP TABLE IF NOT EXISTS scan_state (
        sync_id TEXT PRIMARY KEY NOT NULL,
        local_gen INTEGER NOT NULL DEFAULT 0,
        remote_gen INTEGER NOT NULL DEFAULT 0,
        remote_complete INTEGER NOT NULL DEFAULT 0
    );";

/// Activity items kept per sync.
pub const ACTIVITY_KEPT: i64 = 500;

/// One stored issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueRow {
    pub kind: String,
    pub path: String,
    pub data: String,
    pub updated_ms: i64,
}

/// Folder sync's database.
pub struct SqliteSyncStore {
    conn: Connection,
}

fn u64_of(v: Option<i64>) -> Option<u64> {
    v.and_then(|n| u64::try_from(n).ok())
}

fn i64_of(v: Option<u64>) -> Option<i64> {
    v.map(|n| i64::try_from(n).unwrap_or(i64::MAX))
}

fn size_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The first path after every path in the folder `dir`'s subtree (`dir`
/// without a trailing `/`: `a` → `a0`, so `a/…` lies in `["a/", "a0")`).
fn subtree_end(dir: &str) -> String {
    format!("{dir}0")
}

fn synced_from(r: &Row<'_>, at: usize) -> rusqlite::Result<Option<Synced>> {
    let path: Option<String> = r.get(at)?;
    let Some(path) = path else {
        return Ok(None);
    };
    Ok(Some(Synced {
        path,
        dir: r.get(at + 1)?,
        size: u64_of(r.get(at + 2)?).unwrap_or(0),
        mtime_ns: r.get(at + 3)?,
        file_id: u64_of(r.get(at + 4)?),
        sha256: r.get(at + 5)?,
        etag: r.get(at + 6)?,
        remote_size: u64_of(r.get(at + 7)?),
    }))
}

fn local_from(r: &Row<'_>, at: usize) -> rusqlite::Result<Option<Local>> {
    let path: Option<String> = r.get(at)?;
    let Some(path) = path else {
        return Ok(None);
    };
    Ok(Some(Local {
        path,
        dir: r.get(at + 1)?,
        size: u64_of(r.get(at + 2)?).unwrap_or(0),
        mtime_ns: r.get(at + 3)?,
        file_id: u64_of(r.get(at + 4)?),
    }))
}

fn remote_from(r: &Row<'_>, at: usize) -> rusqlite::Result<Option<Remote>> {
    let path: Option<String> = r.get(at)?;
    let Some(path) = path else {
        return Ok(None);
    };
    Ok(Some(Remote {
        path,
        dir: r.get(at + 1)?,
        marker: r.get(at + 2)?,
        size: u64_of(r.get(at + 3)?).unwrap_or(0),
        etag: r.get(at + 4)?,
        modified_ms: r.get(at + 5)?,
    }))
}

const SYNCED_COLUMNS: &str = "path, dir, size, mtime_ns, file_id, sha256, etag, remote_size";
const LOCAL_COLUMNS: &str = "path, dir, size, mtime_ns, file_id";
#[cfg(test)]
const REMOTE_COLUMNS: &str = "path, dir, marker, size, etag, modified_ms";

impl SqliteSyncStore {
    /// Opens (creating it if needed) the database at `path` and creates the
    /// session tables.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = open_database(path, SYNC_MIGRATIONS)?;
        Self::with_connection(conn)
    }

    /// A database in memory (tests).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory().map_err(sql_error("cannot open"))?;
        super::migrate(&mut conn, SYNC_MIGRATIONS)?;
        Self::with_connection(conn)
    }

    fn with_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SESSION_TABLES)
            .map_err(sql_error("cannot create the session tables"))?;
        Ok(Self { conn })
    }

    // ------------------------------------------------------------ configs

    /// `(id, JSON)` of every sync.
    pub fn configs(&mut self) -> Result<Vec<(String, String)>, StoreError> {
        let read = || -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = self
                .conn
                .prepare("SELECT id, data FROM syncs ORDER BY id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        };
        read().map_err(sql_error("cannot read the folder syncs"))
    }

    pub fn save_config(&mut self, id: &str, data: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO syncs (id, data) VALUES (?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET data = excluded.data",
                params![id, data],
            )
            .map(drop)
            .map_err(sql_error("cannot save the folder sync"))
    }

    /// Forgets a sync completely: configuration, synced state, issues,
    /// activity and session rows. Never touches a file or an object.
    pub fn remove_sync(&mut self, id: &str) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot remove the folder sync"))?;
        for table in [
            "syncs",
            "synced",
            "issues",
            "activity",
            "temp.local_scan",
            "temp.remote_scan",
            "temp.scan_state",
        ] {
            let column = if table == "syncs" { "id" } else { "sync_id" };
            tx.execute(&format!("DELETE FROM {table} WHERE {column} = ?1"), [id])
                .map_err(sql_error("cannot remove the folder sync"))?;
        }
        tx.commit()
            .map_err(sql_error("cannot remove the folder sync"))
    }

    /// Forgets what was synced (and the issues), so the next pass compares
    /// the folder and the bucket afresh, as at the first start: nothing is
    /// deleted on either side then.
    pub fn forget_synced(&mut self, id: &str) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot reset the folder sync"))?;
        for table in ["synced", "issues", "temp.local_scan", "temp.scan_state"] {
            tx.execute(&format!("DELETE FROM {table} WHERE sync_id = ?1"), [id])
                .map_err(sql_error("cannot reset the folder sync"))?;
        }
        tx.commit()
            .map_err(sql_error("cannot reset the folder sync"))
    }

    // ------------------------------------------------------- synced state

    pub fn synced(&mut self, id: &str, path: &str) -> Result<Option<Synced>, StoreError> {
        self.conn
            .query_row(
                &format!("SELECT {SYNCED_COLUMNS} FROM synced WHERE sync_id = ?1 AND path = ?2"),
                params![id, path],
                |r| synced_from(r, 0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(sql_error("cannot read the synced state"))
    }

    pub fn put_synced(&mut self, id: &str, s: &Synced, now_ms: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO synced (sync_id, path, dir, size, mtime_ns, file_id, sha256, etag,
                     remote_size, synced_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (sync_id, path) DO UPDATE SET dir = excluded.dir,
                     size = excluded.size, mtime_ns = excluded.mtime_ns,
                     file_id = excluded.file_id, sha256 = excluded.sha256,
                     etag = excluded.etag, remote_size = excluded.remote_size,
                     synced_ms = excluded.synced_ms",
                params![
                    id,
                    s.path,
                    s.dir,
                    size_i64(s.size),
                    s.mtime_ns,
                    i64_of(s.file_id),
                    s.sha256,
                    s.etag,
                    i64_of(s.remote_size),
                    now_ms
                ],
            )
            .map(drop)
            .map_err(sql_error("cannot save the synced state"))
    }

    pub fn remove_synced(&mut self, id: &str, path: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM synced WHERE sync_id = ?1 AND path = ?2",
                params![id, path],
            )
            .map(drop)
            .map_err(sql_error("cannot save the synced state"))
    }

    /// The synced files and their bytes, and the synced folders.
    pub fn synced_counts(&mut self, id: &str) -> Result<(u64, u64, u64), StoreError> {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(dir = 0), 0), COALESCE(SUM(CASE WHEN dir = 0 THEN size END), 0),
                        COALESCE(SUM(dir = 1), 0)
                 FROM synced WHERE sync_id = ?1",
                [id],
                |r| {
                    Ok((
                        u64_of(r.get(0)?).unwrap_or(0),
                        u64_of(r.get(1)?).unwrap_or(0),
                        u64_of(r.get(2)?).unwrap_or(0),
                    ))
                },
            )
            .map_err(sql_error("cannot count the synced files"))
    }

    // ---------------------------------------------------------- local scan

    fn scan_state(&mut self, id: &str) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO temp.scan_state (sync_id) VALUES (?1) ON CONFLICT DO NOTHING",
                [id],
            )
            .map(drop)
    }

    /// Starts a full local scan; returns its generation.
    pub fn local_begin(&mut self, id: &str) -> Result<i64, StoreError> {
        let mut go = || -> rusqlite::Result<i64> {
            self.scan_state(id)?;
            self.conn.query_row(
                "UPDATE temp.scan_state SET local_gen = local_gen + 1 WHERE sync_id = ?1
                 RETURNING local_gen",
                [id],
                |r| r.get(0),
            )
        };
        go().map_err(sql_error("cannot start the local scan"))
    }

    /// Records scanned entries (one transaction).
    pub fn put_local(
        &mut self,
        id: &str,
        rows: &[Local],
        generation: i64,
    ) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot record the local scan"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO temp.local_scan (sync_id, path, dir, size, mtime_ns, file_id, generation)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (sync_id, path) DO UPDATE SET dir = excluded.dir,
                         size = excluded.size, mtime_ns = excluded.mtime_ns,
                         file_id = excluded.file_id, generation = excluded.generation",
                )
                .map_err(sql_error("cannot record the local scan"))?;
            for l in rows {
                stmt.execute(params![
                    id,
                    l.path,
                    l.dir,
                    size_i64(l.size),
                    l.mtime_ns,
                    i64_of(l.file_id),
                    generation
                ])
                .map_err(sql_error("cannot record the local scan"))?;
            }
        }
        tx.commit()
            .map_err(sql_error("cannot record the local scan"))
    }

    /// Ends a full local scan: entries not seen by it are gone, except
    /// under `unknown` folders (that could not be read: what is below them
    /// is unknown, never gone).
    pub fn local_end(
        &mut self,
        id: &str,
        generation: i64,
        unknown: &[String],
    ) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot finish the local scan"))?;
        {
            let mut stmt = tx
                .prepare(
                    "SELECT path FROM temp.local_scan WHERE sync_id = ?1 AND generation < ?2 ORDER BY path",
                )
                .map_err(sql_error("cannot finish the local scan"))?;
            let stale: Vec<String> = stmt
                .query_map(params![id, generation], |r| r.get(0))
                .and_then(Iterator::collect)
                .map_err(sql_error("cannot finish the local scan"))?;
            let mut delete = tx
                .prepare("DELETE FROM temp.local_scan WHERE sync_id = ?1 AND path = ?2")
                .map_err(sql_error("cannot finish the local scan"))?;
            for path in stale {
                let below_unknown = unknown
                    .iter()
                    .any(|u| path == *u || path.starts_with(&format!("{u}/")));
                if !below_unknown {
                    delete
                        .execute(params![id, path])
                        .map_err(sql_error("cannot finish the local scan"))?;
                }
            }
        }
        tx.commit()
            .map_err(sql_error("cannot finish the local scan"))
    }

    /// Records one path as found now (`None`: it is gone, and so is
    /// everything below it).
    pub fn set_local(
        &mut self,
        id: &str,
        path: &str,
        row: Option<&Local>,
    ) -> Result<(), StoreError> {
        match row {
            Some(l) => self.put_local(id, std::slice::from_ref(l), 0),
            None => {
                let end = subtree_end(path);
                self.conn
                    .execute(
                        "DELETE FROM temp.local_scan WHERE sync_id = ?1
                         AND (path = ?2 OR (path >= ?3 AND path < ?4))",
                        params![id, path, format!("{path}/"), end],
                    )
                    .map(drop)
                    .map_err(sql_error("cannot record the local scan"))
            }
        }
    }

    pub fn local(&mut self, id: &str, path: &str) -> Result<Option<Local>, StoreError> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {LOCAL_COLUMNS} FROM temp.local_scan WHERE sync_id = ?1 AND path = ?2"
                ),
                params![id, path],
                |r| local_from(r, 0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(sql_error("cannot read the local scan"))
    }

    /// Whether the local scan has anything below the folder `path`.
    pub fn has_local_children(&mut self, id: &str, path: &str) -> Result<bool, StoreError> {
        let start = format!("{path}/");
        let end = subtree_end(path);
        self.conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM temp.local_scan WHERE sync_id = ?1
                     AND path >= ?2 AND path < ?3)",
                params![id, start, end],
                |r| r.get(0),
            )
            .map_err(sql_error("cannot read the local scan"))
    }

    // ------------------------------------------------------- remote listing

    /// Starts a remote listing; returns its generation. The previous
    /// complete listing stays in use until [`Self::remote_end`].
    pub fn remote_begin(&mut self, id: &str) -> Result<i64, StoreError> {
        let mut go = || -> rusqlite::Result<i64> {
            self.scan_state(id)?;
            self.conn.query_row(
                "UPDATE temp.scan_state SET remote_gen = remote_gen + 1 WHERE sync_id = ?1
                 RETURNING remote_gen",
                [id],
                |r| r.get(0),
            )
        };
        go().map_err(sql_error("cannot start the remote listing"))
    }

    /// Records listed entries (one transaction). A folder implied by keys
    /// below it never clears a folder's marker.
    pub fn put_remote(
        &mut self,
        id: &str,
        rows: &[Remote],
        generation: i64,
    ) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot record the remote listing"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO temp.remote_scan (sync_id, path, dir, marker, size, etag,
                         modified_ms, generation)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT (sync_id, path) DO UPDATE SET
                         dir = excluded.dir,
                         marker = CASE WHEN temp.remote_scan.generation = excluded.generation
                             THEN MAX(temp.remote_scan.marker, excluded.marker)
                             ELSE excluded.marker END,
                         size = CASE WHEN excluded.dir = 1 THEN 0 ELSE excluded.size END,
                         etag = excluded.etag, modified_ms = excluded.modified_ms,
                         generation = excluded.generation",
                )
                .map_err(sql_error("cannot record the remote listing"))?;
            for r in rows {
                stmt.execute(params![
                    id,
                    r.path,
                    r.dir,
                    r.marker,
                    size_i64(r.size),
                    r.etag,
                    r.modified_ms,
                    generation
                ])
                .map_err(sql_error("cannot record the remote listing"))?;
            }
        }
        tx.commit()
            .map_err(sql_error("cannot record the remote listing"))
    }

    /// Ends a complete listing: entries it did not return are gone. Only
    /// called after the provider returned every page.
    pub fn remote_end(&mut self, id: &str, generation: i64) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot finish the remote listing"))?;
        tx.execute(
            "DELETE FROM temp.remote_scan WHERE sync_id = ?1 AND generation < ?2",
            params![id, generation],
        )
        .map_err(sql_error("cannot finish the remote listing"))?;
        tx.execute(
            "UPDATE temp.scan_state SET remote_complete = 1 WHERE sync_id = ?1",
            [id],
        )
        .map_err(sql_error("cannot finish the remote listing"))?;
        tx.commit()
            .map_err(sql_error("cannot finish the remote listing"))
    }

    /// Drops what a listing that did not finish recorded, keeping the last
    /// complete one.
    pub fn remote_abandon(&mut self, id: &str, generation: i64) -> Result<(), StoreError> {
        // Rows of the unfinished generation replaced rows of the complete
        // one: the complete listing is not intact any more, so the next
        // pass lists again before it plans.
        self.conn
            .execute(
                "UPDATE temp.scan_state SET remote_complete = 0 WHERE sync_id = ?1 AND
                 EXISTS (SELECT 1 FROM temp.remote_scan WHERE sync_id = ?1 AND generation = ?2)",
                params![id, generation],
            )
            .map(drop)
            .map_err(sql_error("cannot abandon the remote listing"))
    }

    /// Whether a complete listing is in the session tables.
    pub fn remote_complete(&mut self, id: &str) -> Result<bool, StoreError> {
        self.conn
            .query_row(
                "SELECT COALESCE((SELECT remote_complete FROM temp.scan_state WHERE sync_id = ?1), 0)",
                [id],
                |r| r.get(0),
            )
            .map_err(sql_error("cannot read the remote listing"))
    }

    /// Records one path as it is in the bucket now (`None`: gone, with
    /// everything below it), after an action changed it.
    pub fn set_remote(
        &mut self,
        id: &str,
        path: &str,
        row: Option<&Remote>,
    ) -> Result<(), StoreError> {
        match row {
            Some(r) => {
                let generation: i64 = self
                    .conn
                    .query_row(
                        "SELECT COALESCE((SELECT remote_gen FROM temp.scan_state WHERE sync_id = ?1), 0)",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(sql_error("cannot read the remote listing"))?;
                self.put_remote(id, std::slice::from_ref(r), generation)
            }
            None => {
                let start = format!("{path}/");
                let end = subtree_end(path);
                self.conn
                    .execute(
                        "DELETE FROM temp.remote_scan WHERE sync_id = ?1
                         AND (path = ?2 OR (path >= ?3 AND path < ?4))",
                        params![id, path, start, end],
                    )
                    .map(drop)
                    .map_err(sql_error("cannot record the remote listing"))
            }
        }
    }

    #[cfg(test)]
    pub fn remote(&mut self, id: &str, path: &str) -> Result<Option<Remote>, StoreError> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {REMOTE_COLUMNS} FROM temp.remote_scan WHERE sync_id = ?1 AND path = ?2"
                ),
                params![id, path],
                |r| remote_from(r, 0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(sql_error("cannot read the remote listing"))
    }

    /// Whether the listing has anything below the folder `path`.
    pub fn has_remote_children(&mut self, id: &str, path: &str) -> Result<bool, StoreError> {
        let start = format!("{path}/");
        let end = subtree_end(path);
        self.conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM temp.remote_scan WHERE sync_id = ?1
                     AND path >= ?2 AND path < ?3)",
                params![id, start, end],
                |r| r.get(0),
            )
            .map_err(sql_error("cannot read the remote listing"))
    }

    /// The listed objects (files) and their bytes.
    pub fn remote_counts(&mut self, id: &str) -> Result<(u64, u64), StoreError> {
        self.conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM temp.remote_scan
                 WHERE sync_id = ?1 AND dir = 0",
                [id],
                |r| {
                    Ok((
                        u64_of(r.get(0)?).unwrap_or(0),
                        u64_of(r.get(1)?).unwrap_or(0),
                    ))
                },
            )
            .map_err(sql_error("cannot read the remote listing"))
    }

    // ------------------------------------------------------------ planning

    /// The next `limit` paths after `after`, in path order, where the
    /// synced state, the local scan and the remote listing do not all
    /// agree.
    pub fn candidates(
        &mut self,
        id: &str,
        after: &str,
        limit: usize,
    ) -> Result<Vec<Triple>, StoreError> {
        let read = || -> rusqlite::Result<Vec<Triple>> {
            let mut stmt = self.conn.prepare(
                "WITH p(path) AS (
                     SELECT path FROM synced WHERE sync_id = ?1 AND path > ?2
                     UNION SELECT path FROM temp.local_scan WHERE sync_id = ?1 AND path > ?2
                     UNION SELECT path FROM temp.remote_scan WHERE sync_id = ?1 AND path > ?2
                 )
                 SELECT p.path,
                     s.path, s.dir, s.size, s.mtime_ns, s.file_id, s.sha256, s.etag, s.remote_size,
                     l.path, l.dir, l.size, l.mtime_ns, l.file_id,
                     r.path, r.dir, r.marker, r.size, r.etag, r.modified_ms
                 FROM p
                 LEFT JOIN synced s ON s.sync_id = ?1 AND s.path = p.path
                 LEFT JOIN temp.local_scan l ON l.sync_id = ?1 AND l.path = p.path
                 LEFT JOIN temp.remote_scan r ON r.sync_id = ?1 AND r.path = p.path
                 WHERE NOT (s.path IS NOT NULL AND l.path IS NOT NULL AND r.path IS NOT NULL
                     AND s.dir = l.dir AND s.dir = r.dir
                     AND (s.dir = 1 OR (l.size = s.size AND l.mtime_ns IS s.mtime_ns
                         AND l.file_id IS s.file_id AND r.etag IS s.etag
                         AND r.size = s.remote_size)))
                 ORDER BY p.path
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![id, after, i64::try_from(limit).unwrap_or(i64::MAX)],
                |r| {
                    Ok(Triple {
                        path: r.get(0)?,
                        synced: synced_from(r, 1)?,
                        local: local_from(r, 9)?,
                        remote: remote_from(r, 14)?,
                    })
                },
            )?;
            rows.collect()
        };
        read().map_err(sql_error("cannot plan the sync"))
    }

    /// Synced files of `size` whose local file is gone while their object
    /// is unchanged: where a new local file of that size may have been
    /// moved from.
    pub fn moved_from_here(&mut self, id: &str, size: u64) -> Result<Vec<Synced>, StoreError> {
        let read = || -> rusqlite::Result<Vec<Synced>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT s.{} FROM synced s
                 JOIN temp.remote_scan r ON r.sync_id = s.sync_id AND r.path = s.path
                 WHERE s.sync_id = ?1 AND s.dir = 0 AND s.size = ?2 AND s.sha256 IS NOT NULL
                     AND r.dir = 0 AND r.etag IS s.etag AND r.size = s.remote_size
                     AND NOT EXISTS (SELECT 1 FROM temp.local_scan l
                         WHERE l.sync_id = s.sync_id AND l.path = s.path)
                 LIMIT 20",
                SYNCED_COLUMNS.replace(", ", ", s.")
            ))?;
            let rows = stmt.query_map(params![id, size_i64(size)], |r| synced_from(r, 0))?;
            rows.filter_map(Result::transpose).collect()
        };
        read().map_err(sql_error("cannot plan the sync"))
    }

    /// Synced files with this ETag and size whose object is gone while the
    /// local file is unchanged: where a new object may have been moved
    /// from in the bucket.
    pub fn moved_from_there(
        &mut self,
        id: &str,
        etag: &str,
        size: u64,
    ) -> Result<Vec<Synced>, StoreError> {
        let read = || -> rusqlite::Result<Vec<Synced>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT s.{} FROM synced s
                 JOIN temp.local_scan l ON l.sync_id = s.sync_id AND l.path = s.path
                 WHERE s.sync_id = ?1 AND s.dir = 0 AND s.etag = ?2 AND s.remote_size = ?3
                     AND l.dir = 0 AND l.size = s.size AND l.mtime_ns IS s.mtime_ns
                     AND l.file_id IS s.file_id
                     AND NOT EXISTS (SELECT 1 FROM temp.remote_scan r
                         WHERE r.sync_id = s.sync_id AND r.path = s.path)
                 LIMIT 20",
                SYNCED_COLUMNS.replace(", ", ", s.")
            ))?;
            let rows = stmt.query_map(params![id, etag, size_i64(size)], |r| synced_from(r, 0))?;
            rows.filter_map(Result::transpose).collect()
        };
        read().map_err(sql_error("cannot plan the sync"))
    }

    // -------------------------------------------------------------- issues

    pub fn issues(&mut self, id: &str) -> Result<Vec<IssueRow>, StoreError> {
        let read = || -> rusqlite::Result<Vec<IssueRow>> {
            let mut stmt = self.conn.prepare(
                "SELECT kind, path, data, updated_ms FROM issues WHERE sync_id = ?1
                 ORDER BY kind, path",
            )?;
            let rows = stmt.query_map([id], |r| {
                Ok(IssueRow {
                    kind: r.get(0)?,
                    path: r.get(1)?,
                    data: r.get(2)?,
                    updated_ms: r.get(3)?,
                })
            })?;
            rows.collect()
        };
        read().map_err(sql_error("cannot read the sync issues"))
    }

    pub fn put_issue(
        &mut self,
        id: &str,
        kind: &str,
        path: &str,
        data: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO issues (sync_id, kind, path, data, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (sync_id, kind, path) DO UPDATE SET data = excluded.data,
                     updated_ms = excluded.updated_ms",
                params![id, kind, path, data, now_ms],
            )
            .map(drop)
            .map_err(sql_error("cannot save a sync issue"))
    }

    pub fn remove_issue(&mut self, id: &str, kind: &str, path: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM issues WHERE sync_id = ?1 AND kind = ?2 AND path = ?3",
                params![id, kind, path],
            )
            .map(drop)
            .map_err(sql_error("cannot save a sync issue"))
    }

    pub fn remove_issues(&mut self, id: &str, kind: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM issues WHERE sync_id = ?1 AND kind = ?2",
                params![id, kind],
            )
            .map(drop)
            .map_err(sql_error("cannot save a sync issue"))
    }

    // ------------------------------------------------------------ activity

    /// Adds an activity item and keeps the newest [`ACTIVITY_KEPT`].
    pub fn add_activity(&mut self, id: &str, data: &str, at_ms: i64) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql_error("cannot record the sync activity"))?;
        tx.execute(
            "INSERT INTO activity (sync_id, at_ms, data) VALUES (?1, ?2, ?3)",
            params![id, at_ms, data],
        )
        .map_err(sql_error("cannot record the sync activity"))?;
        tx.execute(
            "DELETE FROM activity WHERE sync_id = ?1 AND id NOT IN (
                 SELECT id FROM activity WHERE sync_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![id, ACTIVITY_KEPT],
        )
        .map_err(sql_error("cannot record the sync activity"))?;
        tx.commit()
            .map_err(sql_error("cannot record the sync activity"))
    }

    /// The newest `limit` activity items, newest first: `(at_ms, JSON)`.
    pub fn activity(&mut self, id: &str, limit: usize) -> Result<Vec<(i64, String)>, StoreError> {
        let read = || -> rusqlite::Result<Vec<(i64, String)>> {
            let mut stmt = self.conn.prepare(
                "SELECT at_ms, data FROM activity WHERE sync_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![id, i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            rows.collect()
        };
        read().map_err(sql_error("cannot read the sync activity"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64, mtime: i64) -> Local {
        Local {
            path: path.into(),
            dir: false,
            size,
            mtime_ns: Some(mtime),
            file_id: None,
        }
    }

    fn object(path: &str, size: u64, etag: &str) -> Remote {
        Remote {
            path: path.into(),
            dir: false,
            marker: false,
            size,
            etag: Some(etag.into()),
            modified_ms: None,
        }
    }

    fn synced(path: &str, size: u64, mtime: i64, etag: &str) -> Synced {
        Synced {
            path: path.into(),
            dir: false,
            size,
            mtime_ns: Some(mtime),
            file_id: None,
            sha256: Some(format!("sha-{path}")),
            etag: Some(etag.into()),
            remote_size: Some(size),
        }
    }

    #[test]
    fn the_schema_is_created_and_reopened_on_disk() {
        let dir = std::env::temp_dir().join(format!("sotyware-sync-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SYNC_FILE_NAME);
        {
            let mut s = SqliteSyncStore::open(&path).unwrap();
            s.save_config("a", "{}").unwrap();
            s.put_synced("a", &synced("x", 1, 1, "e"), 5).unwrap();
            let g = s.local_begin("a").unwrap();
            s.put_local("a", &[file("x", 1, 1)], g).unwrap();
        }
        let mut s = SqliteSyncStore::open(&path).unwrap();
        assert_eq!(s.configs().unwrap(), [("a".to_owned(), "{}".to_owned())]);
        assert!(s.synced("a", "x").unwrap().is_some());
        // The session tables start empty in every session.
        assert_eq!(s.local("a", "x").unwrap(), None);
        drop(s);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_differing_paths_are_candidates() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        s.put_synced("a", &synced("same", 1, 1, "e"), 0).unwrap();
        s.put_synced("a", &synced("changed", 1, 1, "e"), 0).unwrap();
        s.put_synced("a", &synced("gone", 1, 1, "e"), 0).unwrap();
        s.put_synced("b", &synced("other-sync", 1, 1, "e"), 0)
            .unwrap();
        let g = s.local_begin("a").unwrap();
        s.put_local(
            "a",
            &[file("same", 1, 1), file("changed", 1, 2), file("new", 1, 1)],
            g,
        )
        .unwrap();
        let rg = s.remote_begin("a").unwrap();
        s.put_remote(
            "a",
            &[
                object("same", 1, "e"),
                object("changed", 1, "e"),
                object("gone", 1, "e"),
            ],
            rg,
        )
        .unwrap();
        s.remote_end("a", rg).unwrap();
        let c = s.candidates("a", "", 100).unwrap();
        let paths: Vec<&str> = c.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["changed", "gone", "new"]);
        assert!(c[1].local.is_none() && c[1].remote.is_some() && c[1].synced.is_some());
        assert!(c[2].synced.is_none() && c[2].remote.is_none());
        // Paged.
        let page = s.candidates("a", "changed", 1).unwrap();
        assert_eq!(page[0].path, "gone");
    }

    #[test]
    fn an_unfinished_listing_never_drops_entries() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        let g1 = s.remote_begin("a").unwrap();
        s.put_remote("a", &[object("x", 1, "e"), object("y", 1, "e")], g1)
            .unwrap();
        s.remote_end("a", g1).unwrap();
        assert!(s.remote_complete("a").unwrap());
        let g2 = s.remote_begin("a").unwrap();
        s.put_remote("a", &[object("x", 1, "e")], g2).unwrap();
        // The provider failed before the last page: abandoned, not ended.
        s.remote_abandon("a", g2).unwrap();
        assert!(
            s.remote("a", "y").unwrap().is_some(),
            "y is not taken as gone"
        );
        assert!(
            !s.remote_complete("a").unwrap(),
            "listed again before planning"
        );
        let g3 = s.remote_begin("a").unwrap();
        s.put_remote("a", &[object("x", 1, "e")], g3).unwrap();
        s.remote_end("a", g3).unwrap();
        assert!(s.remote("a", "y").unwrap().is_none());
    }

    #[test]
    fn an_implied_folder_never_clears_a_marker() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        let g = s.remote_begin("a").unwrap();
        let marker = Remote {
            path: "d".into(),
            dir: true,
            marker: true,
            ..Remote::default()
        };
        let implied = Remote {
            marker: false,
            ..marker.clone()
        };
        s.put_remote("a", std::slice::from_ref(&marker), g).unwrap();
        s.put_remote("a", std::slice::from_ref(&implied), g)
            .unwrap();
        assert!(s.remote("a", "d").unwrap().unwrap().marker);
        // A new listing without the marker object.
        let g2 = s.remote_begin("a").unwrap();
        s.put_remote("a", &[implied], g2).unwrap();
        s.remote_end("a", g2).unwrap();
        assert!(!s.remote("a", "d").unwrap().unwrap().marker);
    }

    #[test]
    fn a_full_scan_keeps_what_is_below_an_unreadable_folder() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        let g = s.local_begin("a").unwrap();
        s.put_local(
            "a",
            &[
                file("locked/x", 1, 1),
                file("locked2", 1, 1),
                file("gone", 1, 1),
            ],
            g,
        )
        .unwrap();
        let g2 = s.local_begin("a").unwrap();
        s.local_end("a", g2, &["locked".to_owned()]).unwrap();
        assert!(s.local("a", "locked/x").unwrap().is_some());
        assert!(
            s.local("a", "locked2").unwrap().is_none(),
            "not below 'locked'"
        );
        assert!(s.local("a", "gone").unwrap().is_none());
    }

    #[test]
    fn a_removed_folder_takes_its_subtree_but_not_its_neighbours() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        let rows = [
            file("a", 1, 1),
            file("a b", 1, 1),
            file("a/x", 1, 1),
            file("a/y/z", 1, 1),
            file("a0", 1, 1),
            file("a_%", 1, 1),
        ];
        s.put_local("s", &rows, 1).unwrap();
        assert!(s.has_local_children("s", "a").unwrap());
        s.set_local("s", "a", None).unwrap();
        for gone in ["a", "a/x", "a/y/z"] {
            assert!(s.local("s", gone).unwrap().is_none(), "{gone}");
        }
        for kept in ["a b", "a0", "a_%"] {
            assert!(s.local("s", kept).unwrap().is_some(), "{kept}");
        }
        assert!(!s.has_local_children("s", "a").unwrap());
    }

    #[test]
    fn move_sources_are_found_on_either_side() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        s.put_synced("a", &synced("old", 5, 1, "e1"), 0).unwrap();
        s.put_synced("a", &synced("kept", 5, 1, "e2"), 0).unwrap();
        let g = s.local_begin("a").unwrap();
        s.put_local("a", &[file("kept", 5, 1), file("new", 5, 9)], g)
            .unwrap();
        let rg = s.remote_begin("a").unwrap();
        s.put_remote("a", &[object("old", 5, "e1"), object("kept", 5, "e2")], rg)
            .unwrap();
        s.remote_end("a", rg).unwrap();
        let from = s.moved_from_here("a", 5).unwrap();
        assert_eq!(from.len(), 1);
        assert_eq!(from[0].path, "old");
        assert!(s.moved_from_here("a", 6).unwrap().is_empty());
        // In the bucket: "kept" was renamed to "kept2" by another client.
        let rg = s.remote_begin("a").unwrap();
        s.put_remote("a", &[object("old", 5, "e1"), object("kept2", 5, "e2")], rg)
            .unwrap();
        s.remote_end("a", rg).unwrap();
        let from = s.moved_from_there("a", "e2", 5).unwrap();
        assert_eq!(from.len(), 1);
        assert_eq!(from[0].path, "kept");
    }

    #[test]
    fn removing_a_sync_forgets_everything_of_it_only() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        for id in ["a", "b"] {
            s.save_config(id, "{}").unwrap();
            s.put_synced(id, &synced("x", 1, 1, "e"), 0).unwrap();
            s.put_issue(id, "failed", "x", "{}", 0).unwrap();
            s.add_activity(id, "{}", 0).unwrap();
        }
        s.remove_sync("a").unwrap();
        assert_eq!(s.configs().unwrap().len(), 1);
        assert!(s.synced("a", "x").unwrap().is_none());
        assert!(s.issues("a").unwrap().is_empty());
        assert!(s.activity("a", 10).unwrap().is_empty());
        assert!(s.synced("b", "x").unwrap().is_some());
        assert_eq!(s.issues("b").unwrap().len(), 1);
        s.forget_synced("b").unwrap();
        assert!(s.synced("b", "x").unwrap().is_none());
        assert_eq!(s.configs().unwrap().len(), 1, "the configuration stays");
    }

    #[test]
    fn activity_is_bounded_and_newest_first() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        for i in 0..(ACTIVITY_KEPT + 5) {
            s.add_activity("a", &i.to_string(), i).unwrap();
        }
        let all = s.activity("a", 1000).unwrap();
        assert_eq!(all.len(), usize::try_from(ACTIVITY_KEPT).unwrap());
        assert_eq!(all[0].1, (ACTIVITY_KEPT + 4).to_string());
    }

    #[test]
    fn counts_sum_files_only() {
        let mut s = SqliteSyncStore::in_memory().unwrap();
        s.put_synced("a", &synced("x", 10, 1, "e"), 0).unwrap();
        s.put_synced("a", &synced("y", 5, 1, "e"), 0).unwrap();
        s.put_synced(
            "a",
            &Synced {
                path: "d".into(),
                dir: true,
                ..Synced::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(s.synced_counts("a").unwrap(), (2, 15, 1));
        assert_eq!(s.synced_counts("none").unwrap(), (0, 0, 0));
    }
}

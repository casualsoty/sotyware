//! The autoclicker's database (ADR-050): `automation.sqlite3`.
//!
//! Three tables of JSON documents: the settings (one row), the profiles in
//! their order, and the macros. A macro row keeps its summary (name,
//! folder, counts…) beside the whole document, so the library lists
//! thousands of macros without reading their actions. What the documents
//! mean, and their validation, is `automation::model`'s; this module only
//! stores text.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{open_database, sql_error};
use crate::automation::AutomationStore;
use crate::features::StoreError;

/// The database file inside the app's local data directory.
pub const AUTOMATION_FILE_NAME: &str = "automation.sqlite3";

/// Schema of [`AUTOMATION_FILE_NAME`]; append only.
pub const AUTOMATION_MIGRATIONS: &[&str] = &[
    // 1: settings, profiles, macros.
    "CREATE TABLE settings (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        data TEXT NOT NULL
    ) STRICT;
    CREATE TABLE profiles (
        id TEXT PRIMARY KEY NOT NULL,
        position INTEGER NOT NULL,
        data TEXT NOT NULL
    ) STRICT;
    CREATE TABLE macros (
        id TEXT PRIMARY KEY NOT NULL,
        summary TEXT NOT NULL,
        data TEXT NOT NULL,
        updated_ms INTEGER NOT NULL
    ) STRICT;",
];

/// The autoclicker's documents in a SQLite database.
pub struct SqliteAutomationStore {
    conn: Connection,
}

impl SqliteAutomationStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, AUTOMATION_MIGRATIONS).map(|conn| Self { conn })
    }
}

impl AutomationStore for SqliteAutomationStore {
    fn settings(&mut self) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row("SELECT data FROM settings WHERE id = 1", [], |r| r.get(0))
            .optional()
            .map_err(sql_error("cannot read the autoclicker settings"))
    }

    fn save_settings(&mut self, data: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET data = excluded.data",
                [data],
            )
            .map(drop)
            .map_err(sql_error("cannot save the autoclicker settings"))
    }

    fn profiles(&mut self) -> Result<Vec<String>, StoreError> {
        let read = || -> rusqlite::Result<Vec<String>> {
            let mut stmt = self
                .conn
                .prepare("SELECT data FROM profiles ORDER BY position, id")?;
            stmt.query_map([], |r| r.get(0))?.collect()
        };
        read().map_err(sql_error("cannot read the profiles"))
    }

    fn save_profile(&mut self, id: &str, position: i64, data: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO profiles (id, position, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET position = excluded.position, data = excluded.data",
                params![id, position, data],
            )
            .map(drop)
            .map_err(sql_error("cannot save the profile"))
    }

    fn delete_profile(&mut self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM profiles WHERE id = ?1", [id])
            .map(drop)
            .map_err(sql_error("cannot delete the profile"))
    }

    fn macro_summaries(&mut self) -> Result<Vec<String>, StoreError> {
        let read = || -> rusqlite::Result<Vec<String>> {
            let mut stmt = self
                .conn
                .prepare("SELECT summary FROM macros ORDER BY id")?;
            stmt.query_map([], |r| r.get(0))?.collect()
        };
        read().map_err(sql_error("cannot read the macros"))
    }

    fn load_macro(&mut self, id: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row("SELECT data FROM macros WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(sql_error("cannot read the macro"))
    }

    fn save_macro(
        &mut self,
        id: &str,
        summary: &str,
        data: &str,
        updated_ms: i64,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO macros (id, summary, data, updated_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET summary = excluded.summary, data = excluded.data,
                     updated_ms = excluded.updated_ms",
                params![id, summary, data, updated_ms],
            )
            .map(drop)
            .map_err(sql_error("cannot save the macro"))
    }

    fn delete_macro(&mut self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM macros WHERE id = ?1", [id])
            .map(drop)
            .map_err(sql_error("cannot delete the macro"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "sotyware-automation-store-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn documents_survive_a_reopen() {
        let dir = Dir::new();
        let path = dir.0.join(AUTOMATION_FILE_NAME);
        {
            let mut s = SqliteAutomationStore::open(&path).unwrap();
            assert_eq!(s.settings().unwrap(), None);
            s.save_settings("{\"a\":1}").unwrap();
            s.save_settings("{\"a\":2}").unwrap();
            s.save_profile("b", 1, "B").unwrap();
            s.save_profile("a", 0, "A").unwrap();
            s.save_profile("b", 2, "B2").unwrap();
            s.save_macro("m1", "S1", "D1", 5).unwrap();
            s.save_macro("m2", "S2", "D2", 6).unwrap();
            s.delete_macro("m2").unwrap();
        }
        let mut s = SqliteAutomationStore::open(&path).unwrap();
        assert_eq!(s.settings().unwrap().as_deref(), Some("{\"a\":2}"));
        assert_eq!(s.profiles().unwrap(), ["A", "B2"]);
        assert_eq!(s.macro_summaries().unwrap(), ["S1"]);
        assert_eq!(s.load_macro("m1").unwrap().as_deref(), Some("D1"));
        assert_eq!(s.load_macro("m2").unwrap(), None);
        s.delete_profile("a").unwrap();
        assert_eq!(s.profiles().unwrap(), ["B2"]);
    }

    #[test]
    fn a_newer_schema_is_refused_and_left_alone() {
        let dir = Dir::new();
        let path = dir.0.join(AUTOMATION_FILE_NAME);
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
        }
        let e = SqliteAutomationStore::open(&path).err().unwrap();
        assert!(e.to_string().contains("schema version 99"), "{e}");
    }
}

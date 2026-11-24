//! The notes overlay's database (ADR-052): `notes.sqlite3`.
//!
//! One row per note (title, Markdown text, pin, place, times, revision)
//! and two JSON documents: the overlay's settings and what it remembers
//! between sessions (`overlay::model`). The schema version is
//! `PRAGMA user_version`; the documents carry their own format version.
//! A save of a note is one `UPDATE` of one row, a change of order one
//! transaction; what the values mean is `overlay::model`'s.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{open_database, sql_error};
use crate::features::StoreError;
use crate::overlay::NotesStore;
use crate::overlay::model::{Note, NoteOrder};

/// The database file inside the app's local data directory.
pub const NOTES_FILE_NAME: &str = "notes.sqlite3";

/// Schema of [`NOTES_FILE_NAME`]; append only.
pub const NOTES_MIGRATIONS: &[&str] = &[
    // 1: the notes, and the overlay's settings and state documents.
    "CREATE TABLE notes (
        id TEXT PRIMARY KEY NOT NULL,
        title TEXT NOT NULL,
        body TEXT NOT NULL,
        pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
        position INTEGER NOT NULL,
        created_ms INTEGER NOT NULL,
        updated_ms INTEGER NOT NULL,
        revision INTEGER NOT NULL CHECK (revision >= 0)
    ) STRICT;
    CREATE TABLE overlay_documents (
        name TEXT PRIMARY KEY NOT NULL CHECK (name IN ('settings', 'state')),
        data TEXT NOT NULL
    ) STRICT;",
];

/// The overlay's notes and documents in a SQLite database.
pub struct SqliteNotesStore {
    conn: Connection,
}

impl SqliteNotesStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        open_database(path, NOTES_MIGRATIONS).map(|conn| Self { conn })
    }
}

fn note_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let revision: i64 = r.get(7)?;
    Ok(Note {
        id: r.get(0)?,
        title: r.get(1)?,
        body: r.get(2)?,
        pinned: r.get::<_, i64>(3)? == 1,
        position: r.get(4)?,
        created_ms: r.get(5)?,
        updated_ms: r.get(6)?,
        revision: u64::try_from(revision).unwrap_or(0),
    })
}

const COLUMNS: &str = "id, title, body, pinned, position, created_ms, updated_ms, revision";

impl NotesStore for SqliteNotesStore {
    fn notes(&mut self) -> Result<Vec<Note>, StoreError> {
        let read = || -> rusqlite::Result<Vec<Note>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {COLUMNS} FROM notes ORDER BY position, id"
            ))?;
            stmt.query_map([], note_from_row)?.collect()
        };
        read().map_err(sql_error("cannot read the notes"))
    }

    fn note(&mut self, id: &str) -> Result<Option<Note>, StoreError> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM notes WHERE id = ?1"),
                [id],
                note_from_row,
            )
            .optional()
            .map_err(sql_error("cannot read the note"))
    }

    fn insert_note(&mut self, note: &Note, moved: &[NoteOrder]) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            for o in moved {
                tx.execute(
                    "UPDATE notes SET position = ?2, pinned = ?3 WHERE id = ?1",
                    params![o.id, o.position, i64::from(o.pinned)],
                )?;
            }
            tx.execute(
                &format!("INSERT INTO notes ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"),
                params![
                    note.id,
                    note.title,
                    note.body,
                    i64::from(note.pinned),
                    note.position,
                    note.created_ms,
                    note.updated_ms,
                    i64::try_from(note.revision).unwrap_or(i64::MAX),
                ],
            )?;
            tx.commit()
        };
        write().map_err(sql_error("cannot save the new note"))
    }

    fn update_content(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        updated_ms: i64,
        revision: u64,
    ) -> Result<bool, StoreError> {
        self.conn
            .execute(
                "UPDATE notes SET title = ?2, body = ?3, updated_ms = ?4, revision = ?5 WHERE id = ?1",
                params![
                    id,
                    title,
                    body,
                    updated_ms,
                    i64::try_from(revision).unwrap_or(i64::MAX)
                ],
            )
            .map(|n| n == 1)
            .map_err(sql_error("cannot save the note"))
    }

    fn update_order(&mut self, changed: &[NoteOrder]) -> Result<(), StoreError> {
        let mut write = || -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            for o in changed {
                tx.execute(
                    "UPDATE notes SET position = ?2, pinned = ?3 WHERE id = ?1",
                    params![o.id, o.position, i64::from(o.pinned)],
                )?;
            }
            tx.commit()
        };
        write().map_err(sql_error("cannot save the order of the notes"))
    }

    fn delete_note(&mut self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM notes WHERE id = ?1", [id])
            .map(drop)
            .map_err(sql_error("cannot delete the note"))
    }

    fn document(&mut self, name: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row(
                "SELECT data FROM overlay_documents WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error("cannot read the overlay's settings"))
    }

    fn save_document(&mut self, name: &str, data: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO overlay_documents (name, data) VALUES (?1, ?2)
                 ON CONFLICT (name) DO UPDATE SET data = excluded.data",
                params![name, data],
            )
            .map(drop)
            .map_err(sql_error("cannot save the overlay's settings"))
    }

    fn delete_document(&mut self, name: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM overlay_documents WHERE name = ?1", [name])
            .map(drop)
            .map_err(sql_error("cannot reset the overlay's settings"))
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
                "sotyware-notes-store-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn db(&self) -> PathBuf {
            self.0.join(NOTES_FILE_NAME)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn note(id: &str, position: i64) -> Note {
        Note {
            id: id.into(),
            title: format!("Title {id}"),
            body: format!("- [ ] {id}\n"),
            pinned: false,
            position,
            created_ms: 10,
            updated_ms: 10,
            revision: 1,
        }
    }

    #[test]
    fn notes_and_documents_survive_a_reopen() {
        let dir = Dir::new();
        {
            let mut s = SqliteNotesStore::open(&dir.db()).unwrap();
            assert!(s.notes().unwrap().is_empty());
            assert_eq!(s.document("settings").unwrap(), None);
            s.insert_note(&note("a", 0), &[]).unwrap();
            s.insert_note(&note("b", 1), &[]).unwrap();
            // A copy of "a" between them moves "b" down in the same
            // transaction.
            s.insert_note(
                &note("a2", 1),
                &[NoteOrder {
                    id: "b".into(),
                    pinned: false,
                    position: 2,
                }],
            )
            .unwrap();
            assert!(
                s.update_content("a", "Boss", "## Phase 2\n- [x] adds", 20, 2)
                    .unwrap()
            );
            assert!(!s.update_content("gone", "x", "y", 20, 2).unwrap());
            s.update_order(&[NoteOrder {
                id: "b".into(),
                pinned: true,
                position: 0,
            }])
            .unwrap();
            s.save_document("settings", "{\"a\":1}").unwrap();
            s.save_document("settings", "{\"a\":2}").unwrap();
            s.save_document("state", "{}").unwrap();
        }
        let mut s = SqliteNotesStore::open(&dir.db()).unwrap();
        let notes = s.notes().unwrap();
        let summary: Vec<(&str, i64, bool)> = notes
            .iter()
            .map(|n| (n.id.as_str(), n.position, n.pinned))
            .collect();
        // By position, then id: the display order is the model's.
        assert_eq!(summary, [("a", 0, false), ("b", 0, true), ("a2", 1, false)]);
        let a = s.note("a").unwrap().unwrap();
        assert_eq!(
            (a.title.as_str(), a.body.as_str(), a.updated_ms, a.revision),
            ("Boss", "## Phase 2\n- [x] adds", 20, 2)
        );
        assert_eq!(s.note("gone").unwrap(), None);
        assert_eq!(
            s.document("settings").unwrap().as_deref(),
            Some("{\"a\":2}")
        );
        s.delete_note("a").unwrap();
        assert_eq!(s.notes().unwrap().len(), 2);
        s.delete_document("state").unwrap();
        assert_eq!(s.document("state").unwrap(), None);
    }

    #[test]
    fn text_is_stored_exactly() {
        let dir = Dir::new();
        let mut s = SqliteNotesStore::open(&dir.db()).unwrap();
        let mut n = note("u", 0);
        n.title = "Ölmühle — “boss” ✓ 中文".into();
        n.body = "line 1\n\n  - [ ] indented ' \" ; DROP TABLE notes; --\n😀".into();
        s.insert_note(&n, &[]).unwrap();
        assert_eq!(s.note("u").unwrap().unwrap(), n);
    }

    #[test]
    fn only_the_two_documents_exist_and_duplicates_are_refused() {
        let dir = Dir::new();
        let mut s = SqliteNotesStore::open(&dir.db()).unwrap();
        assert!(s.save_document("other", "{}").is_err());
        s.insert_note(&note("a", 0), &[]).unwrap();
        assert!(s.insert_note(&note("a", 1), &[]).is_err());
        assert_eq!(s.notes().unwrap().len(), 1);
    }

    #[test]
    fn a_newer_or_corrupt_database_is_refused_and_left_alone() {
        let dir = Dir::new();
        {
            let conn = Connection::open(dir.db()).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
        }
        let e = SqliteNotesStore::open(&dir.db()).err().unwrap();
        assert!(e.to_string().contains("schema version 99"), "{e}");
        let corrupt = dir.0.join("corrupt.sqlite3");
        std::fs::write(&corrupt, b"not a database, not a database, not a database").unwrap();
        assert!(SqliteNotesStore::open(&corrupt).is_err());
        assert_eq!(
            std::fs::read(&corrupt).unwrap(),
            b"not a database, not a database, not a database"
        );
    }

    #[test]
    fn the_schema_is_at_its_version_with_its_two_tables() {
        let dir = Dir::new();
        drop(SqliteNotesStore::open(&dir.db()).unwrap());
        let conn = Connection::open(dir.db()).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, NOTES_MIGRATIONS.len() as i64);
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(tables, ["notes", "overlay_documents"]);
    }
}

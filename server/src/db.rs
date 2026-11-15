//! The service's SQLite database: schema migrations (append only, version in
//! `PRAGMA user_version`), and every query. Nothing here ever holds a
//! plaintext secret: passwords are Argon2id hashes, tokens and codes are
//! SHA-256 hashes, the vault and key wraps are client-side ciphertext.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// Schema steps: entry `n` takes the database from version `n` to `n + 1`.
/// Append only; never edit a released entry.
pub const MIGRATIONS: &[&str] = &[
    // 1: accounts, codes, devices, sessions, enrollments, audit, outbox.
    "CREATE TABLE accounts (
        id TEXT PRIMARY KEY NOT NULL,
        email TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        verified INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
        created_ms INTEGER NOT NULL,
        failed_logins INTEGER NOT NULL DEFAULT 0,
        locked_until_ms INTEGER NOT NULL DEFAULT 0,
        recovery_wrap TEXT,
        key_proof_hash TEXT,
        vault TEXT,
        vault_revision INTEGER NOT NULL DEFAULT 0,
        vault_updated_ms INTEGER,
        vault_updated_by TEXT
    ) STRICT;
    CREATE TABLE codes (
        email TEXT NOT NULL,
        purpose TEXT NOT NULL CHECK (purpose IN ('verify', 'reset')),
        code_hash TEXT NOT NULL,
        expires_ms INTEGER NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (email, purpose)
    ) STRICT;
    CREATE TABLE devices (
        id TEXT PRIMARY KEY NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        platform TEXT NOT NULL,
        public_key TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('pending', 'enrolled', 'revoked')),
        wrap TEXT,
        created_ms INTEGER NOT NULL,
        last_seen_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
        device_id TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
        family TEXT NOT NULL,
        access_hash TEXT NOT NULL UNIQUE,
        access_expires_ms INTEGER NOT NULL,
        refresh_hash TEXT NOT NULL UNIQUE,
        refresh_expires_ms INTEGER NOT NULL,
        used INTEGER NOT NULL DEFAULT 0 CHECK (used IN (0, 1)),
        revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
        created_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX sessions_by_family ON sessions (family);
    CREATE TABLE enrollments (
        id TEXT PRIMARY KEY NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
        device_id TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
        state TEXT NOT NULL CHECK (state IN ('pending', 'approved', 'rejected')),
        sealed_key TEXT,
        created_ms INTEGER NOT NULL,
        expires_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE audit (
        id INTEGER PRIMARY KEY,
        at_ms INTEGER NOT NULL,
        account_id TEXT,
        device_id TEXT,
        event TEXT NOT NULL
    ) STRICT;
    CREATE TABLE outbox (
        id INTEGER PRIMARY KEY,
        at_ms INTEGER NOT NULL,
        to_email TEXT NOT NULL,
        subject TEXT NOT NULL,
        body TEXT NOT NULL
    ) STRICT;",
];

/// Opens (creating it if needed) the database, checks it and migrates it. A
/// database from a newer service is refused, never touched.
pub fn open(path: &Path) -> Result<Connection, String> {
    let mut conn = Connection::open(path).map_err(|e| format!("cannot open the database: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    // Deleted rows (a deleted account's email, wraps and vault; used codes)
    // are overwritten on disk, not left in free pages.
    conn.pragma_update(None, "secure_delete", true)
        .map_err(|e| e.to_string())?;
    let ok: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| format!("cannot read the database: {e}"))?;
    if ok != "ok" {
        return Err(format!("the database is damaged: {ok}"));
    }
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<(), String> {
    let target = MIGRATIONS.len() as i64;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if current > target {
        return Err(format!(
            "schema version {current} is newer than this service supports ({target})"
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        tx.execute_batch(sql)
            .map_err(|e| format!("schema update to version {}: {e}", i + 1))?;
    }
    tx.pragma_update(None, "user_version", target)
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Writes a consistent copy of the live database to `to` (SQLite's online
/// backup), which must not exist yet.
pub fn backup(conn: &Connection, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("{} already exists", to.display()));
    }
    let mut dest = Connection::open(to).map_err(|e| e.to_string())?;
    let b = rusqlite::backup::Backup::new(conn, &mut dest).map_err(|e| e.to_string())?;
    b.run_to_completion(256, std::time::Duration::from_millis(10), None)
        .map_err(|e| e.to_string())?;
    drop(b);
    let ok: String = dest
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if ok != "ok" {
        return Err(format!("the backup copy failed its integrity check: {ok}"));
    }
    Ok(())
}

/// Replaces the database at `db` with the backup `from` after checking it.
/// Only while the service is stopped. The current file, if any, is kept as
/// `<db>.before-restore-<ms>`.
pub fn restore(from: &Path, db: &Path, now_ms: i64) -> Result<(), String> {
    {
        let src = Connection::open(from).map_err(|e| e.to_string())?;
        let ok: String = src
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if ok != "ok" {
            return Err(format!("the backup is damaged: {ok}"));
        }
        let version: i64 = src
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > MIGRATIONS.len() as i64 {
            return Err("the backup comes from a newer service".into());
        }
    }
    if db.exists() {
        let aside = db.with_extension(format!("before-restore-{now_ms}"));
        std::fs::rename(db, &aside).map_err(|e| e.to_string())?;
        for ext in ["-wal", "-shm"] {
            let side = std::path::PathBuf::from(format!("{}{ext}", db.display()));
            if side.exists() {
                let _ = std::fs::rename(&side, format!("{}{ext}", aside.display()));
            }
        }
    }
    std::fs::copy(from, db).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub verified: bool,
    pub created_ms: i64,
    pub failed_logins: i64,
    pub locked_until_ms: i64,
    pub recovery_wrap: Option<String>,
    pub key_proof_hash: Option<String>,
    pub vault: Option<String>,
    pub vault_revision: i64,
    pub vault_updated_ms: Option<i64>,
    pub vault_updated_by: Option<String>,
}

fn account_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        email: r.get(1)?,
        password_hash: r.get(2)?,
        verified: r.get(3)?,
        created_ms: r.get(4)?,
        failed_logins: r.get(5)?,
        locked_until_ms: r.get(6)?,
        recovery_wrap: r.get(7)?,
        key_proof_hash: r.get(8)?,
        vault: r.get(9)?,
        vault_revision: r.get(10)?,
        vault_updated_ms: r.get(11)?,
        vault_updated_by: r.get(12)?,
    })
}

const ACCOUNT_COLUMNS: &str = "id, email, password_hash, verified, created_ms, failed_logins,
    locked_until_ms, recovery_wrap, key_proof_hash, vault, vault_revision, vault_updated_ms,
    vault_updated_by";

pub fn account_by_email(c: &Connection, email: &str) -> rusqlite::Result<Option<Account>> {
    c.query_row(
        &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE email = ?1"),
        [email],
        account_row,
    )
    .optional()
}

pub fn account_by_id(c: &Connection, id: &str) -> rusqlite::Result<Option<Account>> {
    c.query_row(
        &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"),
        [id],
        account_row,
    )
    .optional()
}

pub fn insert_account(
    c: &Connection,
    id: &str,
    email: &str,
    hash: &str,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO accounts (id, email, password_hash, created_ms) VALUES (?1, ?2, ?3, ?4)",
        params![id, email, hash, now],
    )
    .map(drop)
}

pub fn set_verified(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE accounts SET verified = 1 WHERE id = ?1", [id])
        .map(drop)
}

pub fn set_password(c: &Connection, id: &str, hash: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE accounts SET password_hash = ?2, failed_logins = 0, locked_until_ms = 0 WHERE id = ?1",
        params![id, hash],
    )
    .map(drop)
}

pub fn record_failure(
    c: &Connection,
    id: &str,
    failures: i64,
    locked_until: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE accounts SET failed_logins = ?2, locked_until_ms = ?3 WHERE id = ?1",
        params![id, failures, locked_until],
    )
    .map(drop)
}

pub fn delete_account(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM accounts WHERE id = ?1", [id])
        .map(drop)
}

pub fn set_recovery(
    c: &Connection,
    id: &str,
    wrap: &str,
    proof_hash: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE accounts SET recovery_wrap = ?2, key_proof_hash = ?3 WHERE id = ?1",
        params![id, wrap, proof_hash],
    )
    .map(drop)
}

/// Stores the vault if its revision is still `expected`; the new revision,
/// or `Err(current)` when another device wrote first.
pub fn put_vault(
    c: &Connection,
    account: &str,
    expected: i64,
    envelope: &str,
    device: &str,
    now: i64,
) -> rusqlite::Result<Result<i64, i64>> {
    let changed = c.execute(
        "UPDATE accounts SET vault = ?3, vault_revision = vault_revision + 1,
             vault_updated_ms = ?4, vault_updated_by = ?5
         WHERE id = ?1 AND vault_revision = ?2",
        params![account, expected, envelope, now, device],
    )?;
    let current: i64 = c.query_row(
        "SELECT vault_revision FROM accounts WHERE id = ?1",
        [account],
        |r| r.get(0),
    )?;
    Ok(if changed == 1 {
        Ok(current)
    } else {
        Err(current)
    })
}

pub fn put_code(
    c: &Connection,
    email: &str,
    purpose: &str,
    hash: &str,
    expires: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO codes (email, purpose, code_hash, expires_ms, attempts) VALUES (?1, ?2, ?3, ?4, 0)
         ON CONFLICT (email, purpose) DO UPDATE SET code_hash = excluded.code_hash,
             expires_ms = excluded.expires_ms, attempts = 0",
        params![email, purpose, hash, expires],
    )
    .map(drop)
}

/// `(hash, expires, attempts)` of the code for `email` and `purpose`.
pub fn code(
    c: &Connection,
    email: &str,
    purpose: &str,
) -> rusqlite::Result<Option<(String, i64, i64)>> {
    c.query_row(
        "SELECT code_hash, expires_ms, attempts FROM codes WHERE email = ?1 AND purpose = ?2",
        params![email, purpose],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

pub fn code_attempt(c: &Connection, email: &str, purpose: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE codes SET attempts = attempts + 1 WHERE email = ?1 AND purpose = ?2",
        params![email, purpose],
    )
    .map(drop)
}

pub fn delete_code(c: &Connection, email: &str, purpose: &str) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM codes WHERE email = ?1 AND purpose = ?2",
        params![email, purpose],
    )
    .map(drop)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub platform: String,
    pub public_key: String,
    pub state: String,
    pub wrap: Option<String>,
    pub created_ms: i64,
    pub last_seen_ms: i64,
}

fn device_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: r.get(0)?,
        account_id: r.get(1)?,
        name: r.get(2)?,
        platform: r.get(3)?,
        public_key: r.get(4)?,
        state: r.get(5)?,
        wrap: r.get(6)?,
        created_ms: r.get(7)?,
        last_seen_ms: r.get(8)?,
    })
}

const DEVICE_COLUMNS: &str =
    "id, account_id, name, platform, public_key, state, wrap, created_ms, last_seen_ms";

pub fn insert_device(c: &Connection, d: &Device) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO devices (id, account_id, name, platform, public_key, state, wrap, created_ms, last_seen_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            d.id,
            d.account_id,
            d.name,
            d.platform,
            d.public_key,
            d.state,
            d.wrap,
            d.created_ms,
            d.last_seen_ms
        ],
    )
    .map(drop)
}

pub fn device(c: &Connection, id: &str) -> rusqlite::Result<Option<Device>> {
    c.query_row(
        &format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?1"),
        [id],
        device_row,
    )
    .optional()
}

pub fn devices(c: &Connection, account: &str) -> rusqlite::Result<Vec<Device>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices WHERE account_id = ?1 ORDER BY created_ms"
    ))?;
    stmt.query_map([account], device_row)?.collect()
}

pub fn set_device(
    c: &Connection,
    id: &str,
    state: &str,
    wrap: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE devices SET state = ?2, wrap = ?3 WHERE id = ?1",
        params![id, state, wrap],
    )
    .map(drop)
}

/// After a password reset: every device must enroll again.
pub fn unenroll_all(c: &Connection, account: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE devices SET state = 'pending', wrap = NULL WHERE account_id = ?1 AND state = 'enrolled'",
        [account],
    )
    .map(drop)
}

pub fn touch_device(c: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE devices SET last_seen_ms = ?2 WHERE id = ?1",
        params![id, now],
    )
    .map(drop)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub account_id: String,
    pub device_id: String,
    pub family: String,
    pub access_expires_ms: i64,
    pub refresh_expires_ms: i64,
    pub used: bool,
    pub revoked: bool,
}

fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        account_id: r.get(1)?,
        device_id: r.get(2)?,
        family: r.get(3)?,
        access_expires_ms: r.get(4)?,
        refresh_expires_ms: r.get(5)?,
        used: r.get(6)?,
        revoked: r.get(7)?,
    })
}

const SESSION_COLUMNS: &str =
    "id, account_id, device_id, family, access_expires_ms, refresh_expires_ms, used, revoked";

#[allow(clippy::too_many_arguments)]
pub fn insert_session(
    c: &Connection,
    id: &str,
    account: &str,
    device: &str,
    family: &str,
    access_hash: &str,
    access_expires: i64,
    refresh_hash: &str,
    refresh_expires: i64,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO sessions (id, account_id, device_id, family, access_hash, access_expires_ms,
             refresh_hash, refresh_expires_ms, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            account,
            device,
            family,
            access_hash,
            access_expires,
            refresh_hash,
            refresh_expires,
            now
        ],
    )
    .map(drop)
}

pub fn session_by_access(c: &Connection, hash: &str) -> rusqlite::Result<Option<Session>> {
    c.query_row(
        &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE access_hash = ?1"),
        [hash],
        session_row,
    )
    .optional()
}

pub fn session_by_refresh(c: &Connection, hash: &str) -> rusqlite::Result<Option<Session>> {
    c.query_row(
        &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE refresh_hash = ?1"),
        [hash],
        session_row,
    )
    .optional()
}

pub fn mark_used(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE sessions SET used = 1 WHERE id = ?1", [id])
        .map(drop)
}

pub fn revoke_family(c: &Connection, family: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE sessions SET revoked = 1 WHERE family = ?1",
        [family],
    )
    .map(drop)
}

pub fn revoke_device_sessions(c: &Connection, device: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE sessions SET revoked = 1 WHERE device_id = ?1",
        [device],
    )
    .map(drop)
}

/// Revokes every session of the account except those of `keep` (a device).
pub fn revoke_account_sessions(
    c: &Connection,
    account: &str,
    keep: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE sessions SET revoked = 1 WHERE account_id = ?1 AND (?2 IS NULL OR device_id != ?2)",
        params![account, keep],
    )
    .map(drop)
}

/// Removes sessions that can no longer be used (expired or revoked a day
/// ago); keeps the table small.
pub fn prune_sessions(c: &Connection, now: i64) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM sessions WHERE refresh_expires_ms < ?1",
        [now - 24 * 3_600_000],
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrollment {
    pub id: String,
    pub account_id: String,
    pub device_id: String,
    pub state: String,
    pub sealed_key: Option<String>,
    pub created_ms: i64,
    pub expires_ms: i64,
}

fn enrollment_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Enrollment> {
    Ok(Enrollment {
        id: r.get(0)?,
        account_id: r.get(1)?,
        device_id: r.get(2)?,
        state: r.get(3)?,
        sealed_key: r.get(4)?,
        created_ms: r.get(5)?,
        expires_ms: r.get(6)?,
    })
}

const ENROLLMENT_COLUMNS: &str =
    "id, account_id, device_id, state, sealed_key, created_ms, expires_ms";

pub fn insert_enrollment(c: &Connection, e: &Enrollment) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO enrollments (id, account_id, device_id, state, sealed_key, created_ms, expires_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![e.id, e.account_id, e.device_id, e.state, e.sealed_key, e.created_ms, e.expires_ms],
    )
    .map(drop)
}

pub fn enrollment(c: &Connection, id: &str) -> rusqlite::Result<Option<Enrollment>> {
    c.query_row(
        &format!("SELECT {ENROLLMENT_COLUMNS} FROM enrollments WHERE id = ?1"),
        [id],
        enrollment_row,
    )
    .optional()
}

pub fn pending_enrollments(
    c: &Connection,
    account: &str,
    now: i64,
) -> rusqlite::Result<Vec<Enrollment>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {ENROLLMENT_COLUMNS} FROM enrollments
         WHERE account_id = ?1 AND state = 'pending' AND expires_ms > ?2 ORDER BY created_ms"
    ))?;
    stmt.query_map(params![account, now], enrollment_row)?
        .collect()
}

pub fn set_enrollment(
    c: &Connection,
    id: &str,
    state: &str,
    sealed: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE enrollments SET state = ?2, sealed_key = ?3 WHERE id = ?1",
        params![id, state, sealed],
    )
    .map(drop)
}

pub fn audit(c: &Connection, now: i64, account: Option<&str>, device: Option<&str>, event: &str) {
    if let Err(e) = c.execute(
        "INSERT INTO audit (at_ms, account_id, device_id, event) VALUES (?1, ?2, ?3, ?4)",
        params![now, account, device, event],
    ) {
        eprintln!("sotyware-server: cannot write an audit event: {e}");
    }
}

pub fn audit_events(c: &Connection, account: &str) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = c.prepare("SELECT at_ms, event FROM audit WHERE account_id = ?1 ORDER BY id")?;
    stmt.query_map([account], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

pub fn queue_mail(
    c: &Connection,
    now: i64,
    to: &str,
    subject: &str,
    body: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO outbox (at_ms, to_email, subject, body) VALUES (?1, ?2, ?3, ?4)",
        params![now, to, subject, body],
    )
    .map(drop)
}

/// `(to, subject, body)` of every queued mail, oldest first.
pub fn outbox(c: &Connection) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = c.prepare("SELECT to_email, subject, body FROM outbox ORDER BY id")?;
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect()
}

//! sotyware's account and end-to-end encrypted sync service (ADR-032).
//!
//! The service authenticates users and devices and stores, per account:
//! an Argon2id password hash, devices with their public keys, hashed
//! session tokens, the recovery wrap and device wraps of the account key,
//! and the encrypted vault. All three of the last are client-side
//! ciphertext: the service never holds a key that decrypts them and has no
//! access to any storage provider. Design and API contract:
//! docs/architecture/accounts.md; operations: docs/development/server.md.

pub mod api;
pub mod db;
pub mod security;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Largest request body (a vault plus JSON overhead).
pub const MAX_BODY: usize = 2 * 1024 * 1024;
/// Largest vault envelope accepted.
pub const MAX_VAULT: usize = 1024 * 1024;

/// Settings of one running service.
#[derive(Clone, Debug)]
pub struct Config {
    pub cost: security::Cost,
    pub access_ttl_ms: i64,
    pub refresh_ttl_ms: i64,
    pub code_ttl_ms: i64,
    pub enrollment_ttl_ms: i64,
    /// Take the client address from `X-Forwarded-For` (only behind a proxy
    /// that overwrites it).
    pub trust_proxy: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cost: security::Cost::PRODUCTION,
            access_ttl_ms: 15 * 60_000,
            refresh_ttl_ms: 30 * 24 * 3_600_000,
            code_ttl_ms: 15 * 60_000,
            enrollment_ttl_ms: 15 * 60_000,
            trust_proxy: false,
        }
    }
}

/// Shared state of the handlers.
pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub config: Config,
    pub limits: security::Limits,
    /// Verified against for unknown emails, so they take as long as known
    /// ones.
    pub dummy_hash: String,
    /// Failed sign-ins for addresses without an account, kept in memory so
    /// lockouts look the same for them: `(failures, locked until)`.
    unknown: Mutex<HashMap<String, (i64, i64)>>,
    /// Milliseconds since the epoch (replaceable in tests).
    pub clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl AppState {
    pub fn new(conn: rusqlite::Connection, config: Config) -> Arc<Self> {
        Arc::new(Self {
            dummy_hash: security::hash_password("not-a-real-password-for-timing", config.cost),
            db: Mutex::new(conn),
            config,
            limits: security::Limits::default(),
            unknown: Mutex::new(HashMap::new()),
            clock: Box::new(now_ms),
        })
    }

    pub fn with_clock(
        mut self: Arc<Self>,
        clock: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::get_mut(&mut self).expect("not shared yet").clock = Box::new(clock);
        self
    }

    fn unknown_lock(&self, email: &str) -> i64 {
        self.unknown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(email)
            .map_or(0, |(_, until)| *until)
    }

    fn unknown_failure(&self, email: &str, now: i64) {
        let mut map = self.unknown.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 100_000 {
            map.retain(|_, (_, until)| *until > now);
        }
        let e = map.entry(email.to_owned()).or_insert((0, 0));
        e.0 += 1;
        e.1 = now + security::lockout_ms(e.0);
    }
}

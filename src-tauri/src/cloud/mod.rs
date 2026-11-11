//! Cloud storage, proof of concept (ADR-016): accounts at S3-compatible
//! providers, and storing, listing, fetching and deleting objects in them.
//!
//! - [`provider`]: what each provider supports, as data. Backblaze B2 and
//!   Hetzner Object Storage are profiles, not code.
//! - [`account`]: account configurations (no secrets), validation, the set of
//!   accounts and the [`account::AccountStore`] trait (SQLite in `storage.rs`).
//! - [`credentials`]: secrets, looked up per account by a
//!   [`credentials::CredentialSource`].
//! - [`s3`]: the one adapter, on the official AWS SDK, with end-to-end SHA-256
//!   verification and a transfer queue per account.
//!
//! Not wired into the running app yet: no feature, command or UI uses it, so
//! the dashboard and the release binary behave as before. It is public so the
//! opt-in integration tests and later the app can reach it.
//! Design: docs/architecture/cloud-storage.md.

pub mod account;
pub mod credentials;
pub mod provider;
pub mod s3;

/// The SQLite account store lives with the other SQLite code in `storage.rs`.
pub use crate::storage::{ACCOUNTS_FILE_NAME, SqliteAccountStore};

#[cfg(test)]
mod integration;
#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tests;

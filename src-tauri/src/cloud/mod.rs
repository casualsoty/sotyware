//! Cloud storage, proof of concept (ADR-016): accounts at S3-compatible
//! providers, and storing, listing, fetching and deleting objects in them.
//!
//! - [`provider`]: what each provider supports, as data. Backblaze B2 and
//!   Hetzner Object Storage are profiles, not code.
//! - [`account`]: account configurations (no secrets), validation, the set of
//!   accounts and the [`account::AccountStore`] trait (SQLite in `storage.rs`).
//! - [`credentials`]: secrets and the [`credentials::CredentialStore`] trait
//!   they are kept behind, by account id.
//! - [`keyring`]: the one real credential store, the operating system's
//!   (Windows Credential Manager, Linux Secret Service). No fallback.
//! - [`s3`]: the one adapter, on the official AWS SDK, with end-to-end SHA-256
//!   verification and a transfer queue per account.
//! - [`dashboard`]: the Tauri commands of the Cloud Storage page (ADR-018):
//!   list, add, edit and remove accounts and check their connection. No file
//!   browsing, transfers or backups are reachable from the app yet.
//!
//! Design: docs/architecture/cloud-storage.md.

pub mod account;
pub mod credentials;
pub mod dashboard;
pub mod keyring;
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

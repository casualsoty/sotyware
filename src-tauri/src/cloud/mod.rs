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
//!   list, add, edit and remove accounts and check their connection.
//! - [`browser`]: read-only browsing of an account's bucket, one folder level
//!   and one page at a time (ADR-019).
//! - [`download`]: downloading one file at a time from the browser to a path
//!   chosen in the native save dialog, with progress and cancellation
//!   (ADR-020).
//! - [`upload`]: uploading a batch of up to 100 picked files, one after
//!   another, into one folder, never overwriting, with progress,
//!   cancellation and retry (ADR-022). Backend and commands only; the
//!   dashboard has no upload control yet. Deletes and backups are not
//!   reachable from the app.
//!
//! Design: docs/architecture/cloud-storage.md.

pub mod account;
pub mod browser;
pub mod credentials;
pub mod dashboard;
pub mod download;
pub mod keyring;
pub mod provider;
/// The opt-in request recorder (ADR-024); only in builds with the
/// `request-recorder` feature, never in a normal build.
#[cfg(feature = "request-recorder")]
pub mod recorder;
pub mod s3;
pub mod upload;

/// The SQLite account store lives with the other SQLite code in `storage.rs`.
pub use crate::storage::{ACCOUNTS_FILE_NAME, SqliteAccountStore};

#[cfg(test)]
mod integration;
#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tests;

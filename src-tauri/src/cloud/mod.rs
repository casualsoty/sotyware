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
//! - [`upload`]: uploading a batch of up to 100 picked files (or a folder
//!   of up to 1,000) one after another into one folder, never overwriting,
//!   with progress, cancellation and retry (ADR-022, ADR-026).
//! - [`operations`]: copy, move, rename, delete and restore as reviewed
//!   plans in their own slot (ADR-026).
//! - [`manage`]: folders, properties, previews, search, deleted items and
//!   share links (ADR-026).
//! - [`bulk_download`]: several files and folders into a picked folder
//!   (ADR-026).
//! - [`places`]: favorites and recent items (ADR-026).
//! - [`drop`]: files dropped on the dashboard, paths kept in Rust (ADR-026).
//! - [`recovery`]: the recovery view over the operation journal: inspect
//!   interrupted work, dismiss it, abort incomplete multipart uploads
//!   (ADR-030).
//!
//! Design: docs/architecture/cloud-storage.md.

pub mod account;
pub mod browser;
pub mod bulk_download;
pub mod credentials;
pub mod dashboard;
pub mod download;
pub mod drop;
pub mod keyring;
pub mod manage;
pub mod operations;
pub mod places;
pub mod provider;
/// The opt-in request recorder (ADR-024); only in builds with the
/// `request-recorder` feature, never in a normal build.
#[cfg(feature = "request-recorder")]
pub mod recorder;
pub mod recovery;
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

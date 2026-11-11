//! The Cloud Storage page's access to accounts (ADR-018): the commands the
//! dashboard calls, the shapes they return, and the connection check.
//!
//! Everything here is a thin layer over [`Accounts`]: validation, the order of
//! credential and configuration writes and their rollback stay there. This
//! module adds what the page needs on top:
//!
//! - **Safe shapes only.** [`AccountInfo`] carries configuration and the last
//!   check, never a key id or a secret, and nothing here returns credentials.
//!   [`CredentialInput`] is the only type that holds a secret; it is turned
//!   into [`Credentials`] at once and has no readable `Debug`.
//! - **Structured errors.** [`CommandError`] has a `kind` the page switches
//!   on and a message it shows; the page never parses message text.
//! - **Off the UI thread.** Every call that can reach the OS credential store
//!   (on Linux a D-Bus round trip, possibly an unlock prompt that waits for
//!   the user) or SQLite runs in `spawn_blocking`; network requests run on the
//!   async runtime. No command blocks the event loop or an async worker.
//! - **A bucket-scoped connection check** ([`S3Store::check_bucket`]), never
//!   `ListBuckets`, which a valid key restricted to one bucket may be refused.
//!
//! Results of connection checks are kept in memory only, per account, and
//! dropped when the account changes.
//!
//! [`S3Store::check_bucket`]: super::s3::S3Store::check_bucket

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::State;

use super::account::{AccountConfig, AccountError, AccountStore, Accounts};
use super::credentials::{CredentialError, CredentialStore, Credentials, Secret};
use super::provider::{self, EndpointRule, PROVIDERS};
use super::s3::{ClientOptions, StorageError};
use crate::features::StoreError;

/// The region offered for a provider without known regions (a generic
/// S3-compatible service). Most such services accept any region in the
/// signature; this is the one the AWS tools default to.
const GENERIC_DEFAULT_REGION: &str = "us-east-1";

/// Longest account id generated from a label, leaving room for a `-<n>`
/// suffix inside the 64 characters an id may have.
const MAX_GENERATED_ID_LEN: usize = 56;

/// How many `-<n>` suffixes are tried before giving up on a label.
const MAX_ID_ATTEMPTS: usize = 1000;

/// A region a provider documents, for the form's suggestions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegionInfo {
    pub id: &'static str,
    pub location: &'static str,
}

/// A provider as the form sees it: what to ask for and what to suggest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// The account must give an endpoint (the provider has no regional rule).
    /// Otherwise an endpoint is an optional override.
    pub endpoint_required: bool,
    /// The region the form starts with.
    pub default_region: &'static str,
    pub known_regions: Vec<RegionInfo>,
}

/// Every supported provider, in [`PROVIDERS`] order.
pub fn providers() -> Vec<ProviderInfo> {
    PROVIDERS
        .iter()
        .map(|p| ProviderInfo {
            id: p.id,
            name: p.name,
            endpoint_required: p.endpoint == EndpointRule::Custom,
            default_region: p
                .known_regions
                .first()
                .map_or(GENERIC_DEFAULT_REGION, |r| r.id),
            known_regions: p
                .known_regions
                .iter()
                .map(|r| RegionInfo {
                    id: r.id,
                    location: r.location,
                })
                .collect(),
        })
        .collect()
}

/// The outcome of a connection check, sorted by what the user can do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckStatus {
    /// The provider accepted the key for the configured bucket.
    Connected,
    /// The provider does not know the key id, or the secret is wrong.
    InvalidCredentials,
    /// The provider accepted the key but refused it this bucket.
    AccessDenied,
    /// The bucket does not exist (at this endpoint/region).
    BucketUnavailable,
    /// The provider could not be reached or did not answer in time.
    Network,
    /// The OS credential store is not available (Linux without a Secret
    /// Service, Windows without a logon session).
    CredentialStoreUnavailable,
    /// The OS credential store refused: locked keyring, dismissed prompt.
    CredentialStoreLocked,
    /// Nothing is stored in the OS credential store for this account.
    CredentialsMissing,
    /// The stored configuration cannot be used (or has no bucket).
    InvalidConfiguration,
    /// Anything else; the message says what.
    Error,
}

/// The last connection check of one account. Kept in memory only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCheck {
    pub status: CheckStatus,
    /// What went wrong, for anything but `Connected`. Never contains a secret.
    pub message: Option<String>,
    /// The bucket that was checked.
    pub bucket: Option<String>,
    /// When the check finished, in milliseconds since the Unix epoch.
    pub checked_at: u64,
}

/// One account as the page sees it: configuration and status, no credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub label: String,
    pub provider: String,
    /// `None` for a provider this build does not know.
    pub provider_name: Option<&'static str>,
    pub region: String,
    /// The endpoint stored with the account (an override, or required for a
    /// generic service), for the edit form.
    pub endpoint: Option<String>,
    /// Where requests go; `None` when the configuration does not validate.
    pub endpoint_url: Option<String>,
    /// The bucket the dashboard uses (accounts created here have exactly one).
    pub bucket: Option<String>,
    /// Why the stored configuration cannot be used, if it cannot.
    pub config_error: Option<String>,
    /// The last connection check since sotyware started, if any.
    pub last_check: Option<ConnectionCheck>,
}

/// What `list_cloud_accounts` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountList {
    pub accounts: Vec<AccountInfo>,
    /// Why the account database could not be opened: then the list is empty
    /// and nothing can be added.
    pub storage_error: Option<String>,
}

/// The non-secret part of the form.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountInput {
    pub label: String,
    pub provider: String,
    pub region: String,
    /// Empty or missing means none (the provider's regional endpoint).
    #[serde(default)]
    pub endpoint: Option<String>,
    pub bucket: String,
}

/// The key pair as typed into the form. The only type here that holds a
/// secret; it is consumed by [`Self::into_credentials`].
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialInput {
    access_key_id: String,
    secret_access_key: String,
}

impl fmt::Debug for CredentialInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialInput(<redacted>)")
    }
}

impl CredentialInput {
    #[cfg(test)]
    pub fn new(access_key_id: &str, secret_access_key: &str) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
        }
    }

    /// Surrounding whitespace is dropped (a pasted key often has a trailing
    /// newline); whitespace inside is refused later by
    /// [`Credentials::validate`].
    fn into_credentials(self) -> Credentials {
        Credentials {
            access_key_id: self.access_key_id.trim().to_owned(),
            secret_access_key: Secret::new(self.secret_access_key.trim()),
        }
    }
}

/// What kind of failure a command had, for the page to choose its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// Something in the form is not valid; nothing was changed.
    InvalidInput,
    /// No account with this id (removed meanwhile).
    UnknownAccount,
    /// The account database could not be opened, read or written.
    Storage,
    /// The OS credential store is not available; nothing was stored elsewhere.
    CredentialStoreUnavailable,
    /// The OS credential store refused (locked, prompt dismissed).
    CredentialStoreLocked,
    /// Any other failure of the OS credential store.
    CredentialStore,
    /// Part of the change happened; the message says which part.
    Partial,
    /// Nothing is stored in the OS credential store for this account
    /// (browsing; the connection check reports it as a status).
    CredentialsMissing,
    /// The provider does not know the key id, or the secret is wrong.
    InvalidCredentials,
    /// The provider accepted the key but refused the request.
    AccessDenied,
    /// The bucket was not found at this endpoint (or the key may not see it).
    BucketUnavailable,
    /// The provider could not be reached, or the connection broke.
    Network,
    /// The provider did not answer in time.
    Timeout,
    /// The account's stored settings cannot be used, or it has no bucket.
    InvalidConfiguration,
    /// The provider refused for another reason (status and code in the
    /// message, never the provider's own text).
    Provider,
    /// A "load more" page that no longer fits: the account changed, the page
    /// was already loaded, or it was for another account or folder.
    OutdatedPage,
    /// Anything else.
    Other,
}

/// Why a command failed. Never carries a secret: every message comes from an
/// error type whose `Display` never shows one (tested below and in
/// `account.rs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CommandError {
    pub(super) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn unknown(id: &str) -> Self {
        Self::new(ErrorKind::UnknownAccount, format!("no account '{id}'"))
    }
}

impl From<AccountError> for CommandError {
    fn from(e: AccountError) -> Self {
        let kind = match &e {
            AccountError::Config(_) | AccountError::Duplicate(_) => ErrorKind::InvalidInput,
            AccountError::Store(_) => ErrorKind::Storage,
            AccountError::Unknown(_) => ErrorKind::UnknownAccount,
            AccountError::Credentials(c) => match c {
                CredentialError::Invalid { .. } => ErrorKind::InvalidInput,
                CredentialError::Unavailable { .. } => ErrorKind::CredentialStoreUnavailable,
                CredentialError::AccessDenied { .. } => ErrorKind::CredentialStoreLocked,
                CredentialError::NotFound { .. } | CredentialError::Backend { .. } => {
                    ErrorKind::CredentialStore
                }
            },
            AccountError::AddRollbackFailed { .. }
            | AccountError::RemovedOnlyCredentials { .. } => ErrorKind::Partial,
        };
        Self::new(kind, e.to_string())
    }
}

/// An account id from its label: lowercase ASCII letters and digits, every
/// other run of characters one `-`. "Backblaze – Photos 2026" becomes
/// `backblaze-photos-2026`; a label without any becomes `account`.
pub fn id_from_label(label: &str) -> String {
    let mut id = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            if id.len() >= MAX_GENERATED_ID_LEN {
                break;
            }
            id.push(c.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-');
    if id.is_empty() {
        "account".to_owned()
    } else {
        id.to_owned()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl AccountInput {
    /// The configuration this input describes for account `id`, starting
    /// from `base` (the stored one when editing, so settings the form does not
    /// show, such as addressing and transfer concurrency, are kept).
    fn apply(&self, base: AccountConfig) -> Result<AccountConfig, CommandError> {
        let bucket = self.bucket.trim();
        if bucket.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a bucket name is required",
            ));
        }
        let endpoint = self
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty());
        Ok(AccountConfig {
            label: self.label.trim().to_owned(),
            provider: self.provider.clone(),
            region: self.region.trim().to_owned(),
            endpoint: endpoint.map(str::to_owned),
            buckets: vec![bucket.to_owned()],
            ..base
        })
    }
}

/// What a check needs once the blocking part is done.
enum Prepared {
    Ready {
        store: Arc<super::s3::S3Store>,
        bucket: String,
        revision: u64,
    },
    Failed {
        check: ConnectionCheck,
        revision: u64,
    },
}

#[derive(Default)]
struct Checks {
    results: HashMap<String, ConnectionCheck>,
    /// Bumped whenever an account changes or goes, so a check or listing
    /// that was running meanwhile does not record a result for the old
    /// settings.
    revisions: HashMap<String, u64>,
    /// The "load more" cursors handed to the page, by id (see [`Cursor`]).
    cursors: HashMap<String, Cursor>,
    /// Cursor ids in the order they were created, for evicting the oldest.
    cursor_order: VecDeque<String>,
    next_cursor: u64,
}

/// The next page of one folder listing. The page only ever sees the id; the
/// provider's continuation token stays here, tied to the account (and its
/// revision), the bucket and the prefix it was issued for, so it can never
/// be sent with another account's credentials or for another folder.
#[derive(Clone, Debug)]
struct Cursor {
    account: String,
    revision: u64,
    bucket: String,
    prefix: String,
    token: String,
}

/// At most this many unused cursors are kept; older ones are forgotten (the
/// page then reports an outdated page and refreshes).
const MAX_CURSORS: usize = 256;

/// Entries per listing request: S3's maximum, so a large folder costs as
/// few (billed) requests as possible.
pub const LIST_PAGE_SIZE: i32 = 1000;

/// What a listing needs once the blocking part is done.
pub(super) struct PreparedListing {
    pub store: Arc<super::s3::S3Store>,
    pub bucket: String,
    pub revision: u64,
    /// The provider token of the requested page, `None` for the first page.
    pub token: Option<String>,
    pub page_size: i32,
}

/// The accounts behind the Cloud Storage page, managed as Tauri state.
pub struct CloudAccounts {
    accounts: Result<Accounts, StoreError>,
    checks: Mutex<Checks>,
    page_size: i32,
}

impl CloudAccounts {
    /// Loads the accounts from `store`. Does not touch the credential store.
    /// A store that could not be opened, or loaded, is kept as the reason the
    /// page shows; every command then fails with [`ErrorKind::Storage`].
    pub fn load(
        store: Result<Box<dyn AccountStore>, StoreError>,
        credentials: Arc<dyn CredentialStore>,
        options: ClientOptions,
    ) -> Self {
        let accounts = store.and_then(|s| Accounts::load(s, credentials, options));
        if let Err(e) = &accounts {
            eprintln!("sotyware: cannot load the cloud storage accounts: {e}");
        }
        Self {
            accounts,
            checks: Mutex::default(),
            page_size: LIST_PAGE_SIZE,
        }
    }

    /// The same accounts with smaller listing pages, so tests reach
    /// pagination with a handful of objects.
    #[cfg(test)]
    pub fn with_page_size(mut self, page_size: i32) -> Self {
        self.page_size = page_size;
        self
    }

    fn accounts(&self) -> Result<&Accounts, CommandError> {
        self.accounts.as_ref().map_err(|e| {
            CommandError::new(
                ErrorKind::Storage,
                format!("the account database is not available: {e}"),
            )
        })
    }

    fn checks(&self) -> MutexGuard<'_, Checks> {
        self.checks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn find(&self, id: &str) -> Result<AccountConfig, CommandError> {
        self.accounts()?
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| CommandError::unknown(id))
    }

    /// Forgets the account's last check and its listing cursors, and
    /// invalidates a running check or listing.
    fn changed(&self, id: &str) {
        let mut checks = self.checks();
        checks.results.remove(id);
        *checks.revisions.entry(id.to_owned()).or_default() += 1;
        checks.cursors.retain(|_, c| c.account != id);
        let Checks {
            cursors,
            cursor_order,
            ..
        } = &mut *checks;
        cursor_order.retain(|c| cursors.contains_key(c));
    }

    fn info(&self, config: AccountConfig) -> AccountInfo {
        let resolved = config.resolve();
        let last_check = self.checks().results.get(&config.id).cloned();
        AccountInfo {
            provider_name: provider::profile(&config.provider).map(|p| p.name),
            endpoint_url: resolved.as_ref().ok().map(|r| r.endpoint_url.clone()),
            config_error: resolved.err().map(|e| e.to_string()),
            bucket: config.buckets.first().cloned(),
            last_check,
            id: config.id,
            label: config.label,
            provider: config.provider,
            region: config.region,
            endpoint: config.endpoint,
        }
    }

    /// Every account, in id order. Blocks while another call holds the
    /// accounts (for example one waiting for an unlock prompt).
    pub fn list(&self) -> AccountList {
        match &self.accounts {
            Ok(accounts) => AccountList {
                accounts: accounts.list().into_iter().map(|c| self.info(c)).collect(),
                storage_error: None,
            },
            Err(e) => AccountList {
                accounts: Vec::new(),
                storage_error: Some(e.to_string()),
            },
        }
    }

    /// Adds an account with an id generated from its label (unique, so two
    /// accounts may have the same label and provider) and stores its
    /// credentials in the credential store.
    pub fn add(
        &self,
        input: &AccountInput,
        credentials: CredentialInput,
    ) -> Result<AccountInfo, CommandError> {
        let accounts = self.accounts()?;
        let credentials = credentials.into_credentials();
        let base = id_from_label(&input.label);
        let taken: HashSet<String> = accounts.list().into_iter().map(|c| c.id).collect();
        for n in 1..=MAX_ID_ATTEMPTS {
            let id = if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            if taken.contains(&id) {
                continue;
            }
            let config = input.apply(AccountConfig::new(&id, "", "", ""))?;
            match accounts.add(config.clone(), &credentials) {
                Ok(()) => {
                    self.changed(&id);
                    return Ok(self.info(config));
                }
                // Added by another call since `taken` was read: next suffix.
                Err(AccountError::Duplicate(_)) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(CommandError::new(
            ErrorKind::InvalidInput,
            "too many accounts with this name; choose another name",
        ))
    }

    /// Changes an account's configuration and, if `credentials` is given,
    /// replaces its key. The id and the provider stay; settings the form does
    /// not show are kept.
    ///
    /// Everything is validated before anything is written. A new key is
    /// stored first, in one replace: if that fails, nothing changed and the
    /// old key is still in place. Only then the configuration is saved; if
    /// that fails, the error ([`ErrorKind::Partial`]) says the new key is in
    /// use but the other changes were not saved. Without new credentials the
    /// credential store is not touched at all.
    pub fn update(
        &self,
        id: &str,
        input: &AccountInput,
        credentials: Option<CredentialInput>,
    ) -> Result<AccountInfo, CommandError> {
        let accounts = self.accounts()?;
        let current = self.find(id)?;
        if input.provider != current.provider {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the provider of an account cannot be changed; add a new account instead",
            ));
        }
        let config = input.apply(current.clone())?;
        config.resolve().map_err(AccountError::Config)?;
        if let Some(credentials) = credentials {
            accounts.set_credentials(id, &credentials.into_credentials())?;
            self.changed(id);
            if config != current
                && let Err(e) = accounts.update(config.clone())
            {
                return Err(CommandError::new(
                    ErrorKind::Partial,
                    format!("the new access key was saved, but the other changes were not: {e}"),
                ));
            }
        } else if config != current {
            accounts.update(config.clone())?;
        }
        self.changed(id);
        Ok(self.info(config))
    }

    /// Removes the account and its credentials ([`Accounts::remove`]); its
    /// buckets and objects at the provider are not touched. Fails, with the
    /// account kept, if the credentials cannot be deleted.
    pub fn remove(&self, id: &str) -> Result<(), CommandError> {
        let result = self.accounts()?.remove(id);
        // Also after a partial removal: the account's settings changed.
        self.changed(id);
        result.map_err(CommandError::from)
    }

    /// The blocking half of a check: reads the configuration and loads the
    /// credentials (OS credential store). Sends no request.
    fn prepare_check(&self, id: &str) -> Result<Prepared, CommandError> {
        let accounts = self.accounts()?;
        let revision = self.checks().revisions.get(id).copied().unwrap_or(0);
        let config = self.find(id)?;
        let failed = |status, message: String, bucket: Option<String>| {
            Ok(Prepared::Failed {
                check: ConnectionCheck {
                    status,
                    message: Some(message),
                    bucket,
                    checked_at: now_ms(),
                },
                revision,
            })
        };
        let Some(bucket) = config.buckets.first().cloned() else {
            return failed(
                CheckStatus::InvalidConfiguration,
                "no bucket is configured for this account".into(),
                None,
            );
        };
        match accounts.connect(id) {
            Ok(store) => Ok(Prepared::Ready {
                store,
                bucket,
                revision,
            }),
            Err(AccountError::Unknown(_)) => Err(CommandError::unknown(id)),
            Err(e) => {
                let status = match &e {
                    AccountError::Config(_) => CheckStatus::InvalidConfiguration,
                    AccountError::Credentials(CredentialError::NotFound { .. }) => {
                        CheckStatus::CredentialsMissing
                    }
                    AccountError::Credentials(CredentialError::Unavailable { .. }) => {
                        CheckStatus::CredentialStoreUnavailable
                    }
                    AccountError::Credentials(CredentialError::AccessDenied { .. }) => {
                        CheckStatus::CredentialStoreLocked
                    }
                    _ => CheckStatus::Error,
                };
                failed(status, e.to_string(), Some(bucket))
            }
        }
    }

    /// Records a finished check unless the account changed while it ran, and
    /// returns the account as it is now.
    fn record(
        &self,
        id: &str,
        revision: u64,
        check: ConnectionCheck,
    ) -> Result<AccountInfo, CommandError> {
        {
            let mut checks = self.checks();
            if checks.revisions.get(id).copied().unwrap_or(0) == revision {
                checks.results.insert(id.to_owned(), check);
            }
        }
        Ok(self.info(self.find(id)?))
    }

    /// The blocking half of a folder listing: finds the account's bucket,
    /// resolves the page cursor, and loads the credentials (OS credential
    /// store). Sends no request. A cursor must have been issued for this
    /// account, at its current revision, for the same bucket and prefix.
    pub(super) fn prepare_listing(
        &self,
        id: &str,
        prefix: &str,
        page: Option<&str>,
    ) -> Result<PreparedListing, CommandError> {
        let accounts = self.accounts()?;
        let config = self.find(id)?;
        let Some(bucket) = config.buckets.first().cloned() else {
            return Err(CommandError::new(
                ErrorKind::InvalidConfiguration,
                "no bucket is configured for this account",
            ));
        };
        let (revision, token) = {
            let checks = self.checks();
            let revision = checks.revisions.get(id).copied().unwrap_or(0);
            let token = match page {
                None => None,
                Some(page) => match checks.cursors.get(page) {
                    Some(c)
                        if c.account == id
                            && c.revision == revision
                            && c.bucket == bucket
                            && c.prefix == prefix =>
                    {
                        Some(c.token.clone())
                    }
                    _ => {
                        return Err(CommandError::new(
                            ErrorKind::OutdatedPage,
                            "this page of the folder is no longer available",
                        ));
                    }
                },
            };
            (revision, token)
        };
        let store = accounts.connect(id).map_err(|e| {
            let kind = match &e {
                AccountError::Unknown(_) => ErrorKind::UnknownAccount,
                AccountError::Config(_) => ErrorKind::InvalidConfiguration,
                AccountError::Credentials(CredentialError::NotFound { .. }) => {
                    ErrorKind::CredentialsMissing
                }
                AccountError::Credentials(CredentialError::Unavailable { .. }) => {
                    ErrorKind::CredentialStoreUnavailable
                }
                AccountError::Credentials(CredentialError::AccessDenied { .. }) => {
                    ErrorKind::CredentialStoreLocked
                }
                AccountError::Credentials(_) => ErrorKind::CredentialStore,
                _ => ErrorKind::Other,
            };
            CommandError::new(kind, e.to_string())
        })?;
        Ok(PreparedListing {
            store,
            bucket,
            revision,
            token,
            page_size: self.page_size,
        })
    }

    /// The blocking half after a listing: refuses a result for an account
    /// that changed or went while the request ran, retires the cursor that
    /// was used and issues one for the next page. Returns the new cursor id.
    pub(super) fn finish_listing(
        &self,
        id: &str,
        revision: u64,
        bucket: &str,
        prefix: &str,
        used: Option<&str>,
        next_token: Option<String>,
    ) -> Result<Option<String>, CommandError> {
        self.find(id)?;
        let mut checks = self.checks();
        if checks.revisions.get(id).copied().unwrap_or(0) != revision {
            return Err(CommandError::new(
                ErrorKind::OutdatedPage,
                "the account changed while the folder was loading",
            ));
        }
        if let Some(used) = used {
            checks.cursors.remove(used);
            checks.cursor_order.retain(|c| c != used);
        }
        let Some(token) = next_token else {
            return Ok(None);
        };
        checks.next_cursor += 1;
        let cursor_id = format!("p{}", checks.next_cursor);
        checks.cursors.insert(
            cursor_id.clone(),
            Cursor {
                account: id.to_owned(),
                revision,
                bucket: bucket.to_owned(),
                prefix: prefix.to_owned(),
                token,
            },
        );
        checks.cursor_order.push_back(cursor_id.clone());
        while checks.cursor_order.len() > MAX_CURSORS {
            if let Some(oldest) = checks.cursor_order.pop_front() {
                checks.cursors.remove(&oldest);
            }
        }
        Ok(Some(cursor_id))
    }

    /// How many listing cursors are kept, for tests.
    #[cfg(test)]
    pub(super) fn cursor_count(&self) -> usize {
        self.checks().cursors.len()
    }

    /// Replaces the provider token behind a cursor, so a test can send a
    /// token the provider refuses.
    #[cfg(test)]
    pub(super) fn replace_cursor_token(&self, cursor: &str, token: &str) {
        if let Some(c) = self.checks().cursors.get_mut(cursor) {
            c.token = token.to_owned();
        }
    }
}

/// How a storage error is shown as a check result.
pub fn check_status(error: &StorageError) -> CheckStatus {
    match error {
        StorageError::InvalidCredentials(_) => CheckStatus::InvalidCredentials,
        StorageError::AccessDenied(_) => CheckStatus::AccessDenied,
        StorageError::NotFound(_) => CheckStatus::BucketUnavailable,
        StorageError::Network(_) | StorageError::Timeout(_) => CheckStatus::Network,
        StorageError::Refused(_) => CheckStatus::InvalidConfiguration,
        _ => CheckStatus::Error,
    }
}

/// Runs `f` on the blocking thread pool and waits for it without blocking
/// the caller's thread: credential store and SQLite calls go through here.
pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| {
        Err(CommandError::new(
            ErrorKind::Other,
            format!("the operation stopped unexpectedly: {e}"),
        ))
    })
}

/// Logs a failed change (never a secret: see [`CommandError`]). Invalid input
/// is the user's to fix and shown on the page, so it is not logged.
fn logged<T>(what: &str, result: Result<T, CommandError>) -> Result<T, CommandError> {
    if let Err(e) = &result
        && e.kind != ErrorKind::InvalidInput
    {
        eprintln!("sotyware: failed to {what}: {}", e.message);
    }
    result
}

/// `check_cloud_account` without Tauri. The credentials are loaded on the
/// blocking pool, the request is sent on the async runtime.
pub async fn check_account(
    state: Arc<CloudAccounts>,
    id: String,
) -> Result<AccountInfo, CommandError> {
    let prepared = {
        let state = Arc::clone(&state);
        let id = id.clone();
        blocking(move || state.prepare_check(&id)).await?
    };
    let (check, revision) = match prepared {
        Prepared::Failed { check, revision } => (check, revision),
        Prepared::Ready {
            store,
            bucket,
            revision,
        } => {
            let result = store.check_bucket(&bucket).await;
            let check = ConnectionCheck {
                status: result
                    .as_ref()
                    .map_or_else(check_status, |()| CheckStatus::Connected),
                message: result.err().map(|e| e.to_string()),
                bucket: Some(bucket),
                checked_at: now_ms(),
            };
            (check, revision)
        }
    };
    blocking(move || state.record(&id, revision, check)).await
}

/// `add_cloud_account` without Tauri.
pub async fn add_account(
    state: Arc<CloudAccounts>,
    account: AccountInput,
    credentials: CredentialInput,
) -> Result<AccountInfo, CommandError> {
    let result = blocking(move || state.add(&account, credentials)).await;
    logged("add a cloud storage account", result)
}

/// `update_cloud_account` without Tauri.
pub async fn update_account(
    state: Arc<CloudAccounts>,
    id: String,
    account: AccountInput,
    credentials: Option<CredentialInput>,
) -> Result<AccountInfo, CommandError> {
    let result = blocking(move || state.update(&id, &account, credentials)).await;
    logged("change a cloud storage account", result)
}

/// `remove_cloud_account` without Tauri.
pub async fn remove_account(state: Arc<CloudAccounts>, id: String) -> Result<(), CommandError> {
    let result = blocking(move || state.remove(&id)).await;
    logged("remove a cloud storage account", result)
}

/// The supported providers with their presets. Pure data, no I/O. Allowed for
/// the dashboard by `allow-list-cloud-providers`.
#[tauri::command]
pub fn list_cloud_providers() -> Vec<ProviderInfo> {
    providers()
}

/// Every configured account with its last check; never credentials. Runs on
/// the blocking pool: the accounts may be held by a call that waits for the
/// credential store. Allowed by `allow-list-cloud-accounts`.
#[tauri::command]
pub async fn list_cloud_accounts(
    state: State<'_, Arc<CloudAccounts>>,
) -> Result<AccountList, CommandError> {
    let state = Arc::clone(&state);
    blocking(move || Ok(state.list())).await
}

/// Adds an account and stores its credentials in the OS credential store.
/// Returns the account without credentials. Allowed by
/// `allow-add-cloud-account`.
#[tauri::command]
pub async fn add_cloud_account(
    state: State<'_, Arc<CloudAccounts>>,
    account: AccountInput,
    credentials: CredentialInput,
) -> Result<AccountInfo, CommandError> {
    add_account(Arc::clone(&state), account, credentials).await
}

/// Changes an account and, with `credentials`, replaces its key. Allowed by
/// `allow-update-cloud-account`.
#[tauri::command]
pub async fn update_cloud_account(
    state: State<'_, Arc<CloudAccounts>>,
    id: String,
    account: AccountInput,
    credentials: Option<CredentialInput>,
) -> Result<AccountInfo, CommandError> {
    update_account(Arc::clone(&state), id, account, credentials).await
}

/// Removes an account from sotyware and deletes its credentials; the
/// provider's buckets and objects are never touched. Allowed by
/// `allow-remove-cloud-account`.
#[tauri::command]
pub async fn remove_cloud_account(
    state: State<'_, Arc<CloudAccounts>>,
    id: String,
) -> Result<(), CommandError> {
    remove_account(Arc::clone(&state), id).await
}

/// Checks the account's connection against its configured bucket and returns
/// the account with the result. A failed check is a result, not an error;
/// the command fails only for an unknown account or an unusable database.
/// Allowed by `allow-check-cloud-account`.
#[tauri::command]
pub async fn check_cloud_account(
    state: State<'_, Arc<CloudAccounts>>,
    id: String,
) -> Result<AccountInfo, CommandError> {
    check_account(Arc::clone(&state), id).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::super::account::MemoryAccountStore;
    use super::super::credentials::MemoryCredentialStore;
    use super::super::test_server::TestServer;
    use super::*;

    const SECRET: &str = "K003-very-secret-value";
    const KEY_ID: &str = "003a1b2c3d4e5f60000000001";
    const BUCKET: &str = "sotyware-test";

    fn test_options() -> ClientOptions {
        ClientOptions {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(5),
            max_attempts: 1,
            ..ClientOptions::default()
        }
    }

    fn state_with(
        store: &MemoryAccountStore,
        keys: &Arc<MemoryCredentialStore>,
    ) -> Arc<CloudAccounts> {
        Arc::new(CloudAccounts::load(
            Ok(Box::new(store.clone())),
            Arc::clone(keys) as Arc<dyn CredentialStore>,
            test_options(),
        ))
    }

    fn state() -> (
        Arc<CloudAccounts>,
        MemoryAccountStore,
        Arc<MemoryCredentialStore>,
    ) {
        let store = MemoryAccountStore::default();
        let keys = Arc::new(MemoryCredentialStore::default());
        (state_with(&store, &keys), store, keys)
    }

    fn b2(label: &str) -> AccountInput {
        AccountInput {
            label: label.into(),
            provider: "backblaze-b2".into(),
            region: "eu-central-003".into(),
            endpoint: None,
            bucket: "soty-photos".into(),
        }
    }

    fn creds(key: &str, secret: &str) -> CredentialInput {
        CredentialInput::new(key, secret)
    }

    fn stored(key: &str, secret: &str) -> Credentials {
        Credentials {
            access_key_id: key.into(),
            secret_access_key: Secret::new(secret),
        }
    }

    fn denied(backend_error: fn() -> CredentialError) -> Option<CredentialError> {
        Some(backend_error())
    }

    fn unavailable() -> CredentialError {
        CredentialError::Unavailable {
            backend: "test backend",
            detail: "not running".into(),
        }
    }

    fn locked() -> CredentialError {
        CredentialError::AccessDenied {
            backend: "test backend",
            detail: "prompt dismissed".into(),
        }
    }

    #[test]
    fn providers_offer_every_profile_with_its_presets() {
        let list = serde_json::to_value(providers()).unwrap();
        let ids: Vec<_> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["backblaze-b2", "hetzner", "s3-compatible"]);
        assert_eq!(list[0]["defaultRegion"], "eu-central-003");
        assert_eq!(list[0]["endpointRequired"], false);
        assert_eq!(
            list[0]["knownRegions"][0],
            serde_json::json!({"id": "eu-central-003", "location": "Amsterdam, NL"})
        );
        assert_eq!(list[1]["defaultRegion"], "fsn1");
        assert_eq!(
            list[2],
            serde_json::json!({
                "id": "s3-compatible",
                "name": "S3-compatible service",
                "endpointRequired": true,
                "defaultRegion": "us-east-1",
                "knownRegions": []
            })
        );
    }

    #[test]
    fn account_ids_come_from_the_label() {
        for (label, id) in [
            ("Photos", "photos"),
            ("Backblaze – Photos 2026", "backblaze-photos-2026"),
            ("  --Work__Backup--  ", "work-backup"),
            ("Fotos Ä", "fotos"),
            ("…", "account"),
            ("", "account"),
        ] {
            assert_eq!(id_from_label(label), id, "{label:?}");
            assert!(super::super::account::is_kebab_case(&id_from_label(label)));
        }
        let long = id_from_label(&"a".repeat(200));
        assert_eq!(long.len(), MAX_GENERATED_ID_LEN);
        assert!(super::super::account::is_kebab_case(&format!("{long}-999")));
    }

    #[test]
    fn adding_stores_the_credentials_and_returns_none_of_them() {
        let (s, store, keys) = state();
        let info = s.add(&b2("Photos"), creds(KEY_ID, SECRET)).unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "photos",
                "label": "Photos",
                "provider": "backblaze-b2",
                "providerName": "Backblaze B2",
                "region": "eu-central-003",
                "endpoint": null,
                "endpointUrl": "https://s3.eu-central-003.backblazeb2.com",
                "bucket": "soty-photos",
                "configError": null,
                "lastCheck": null
            })
        );
        let listed = serde_json::to_string(&s.list()).unwrap();
        for shown in [json.to_string(), listed, format!("{info:?}")] {
            assert!(
                !shown.contains(SECRET) && !shown.contains(KEY_ID),
                "{shown}"
            );
        }
        assert_eq!(keys.load("photos").unwrap(), stored(KEY_ID, SECRET));
        assert_eq!(store.rows.lock().unwrap()[0].buckets, ["soty-photos"]);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_from_the_form() {
        let (s, _store, keys) = state();
        let mut input = b2("  Photos ");
        input.bucket = " soty-photos\n".into();
        input.endpoint = Some("   ".into());
        let info = s.add(&input, creds(" key\n", "\tsecret \n")).unwrap();
        assert_eq!((info.label.as_str(), info.endpoint), ("Photos", None));
        assert_eq!(info.bucket.as_deref(), Some("soty-photos"));
        assert_eq!(keys.load("photos").unwrap(), stored("key", "secret"));
    }

    #[test]
    fn accounts_with_the_same_label_and_provider_stay_separate() {
        let (s, _store, keys) = state();
        let first = s.add(&b2("Photos"), creds("k1", "s1")).unwrap();
        let mut work = b2("Photos");
        work.bucket = "soty-work".into();
        let second = s.add(&work, creds("k2", "s2")).unwrap();
        assert_eq!(
            (first.id.as_str(), second.id.as_str()),
            ("photos", "photos-2")
        );
        assert_eq!(keys.load("photos").unwrap(), stored("k1", "s1"));
        assert_eq!(keys.load("photos-2").unwrap(), stored("k2", "s2"));
        let list = s.list();
        let buckets: Vec<_> = list.accounts.iter().map(|a| a.bucket.clone()).collect();
        assert_eq!(
            buckets,
            [Some("soty-photos".into()), Some("soty-work".into())]
        );

        s.remove("photos").unwrap();
        assert_eq!(keys.ids(), ["photos-2"]);
        assert_eq!(s.list().accounts.len(), 1);
    }

    #[test]
    fn invalid_input_changes_nothing() {
        let (s, store, keys) = state();
        let mut no_bucket = b2("Photos");
        no_bucket.bucket = " ".into();
        let mut bad_region = b2("Photos");
        bad_region.region = "EU".into();
        let mut no_endpoint = b2("Local");
        no_endpoint.provider = "s3-compatible".into();
        let mut http = no_endpoint.clone();
        http.endpoint = Some("http://s3.example.com".into());
        let mut no_label = b2(" ");
        no_label.label = " ".into();
        for (input, credentials) in [
            (no_bucket, creds("k", "s")),
            (bad_region, creds("k", "s")),
            (no_endpoint, creds("k", "s")),
            (http, creds("k", "s")),
            (no_label, creds("k", "s")),
            (b2("Photos"), creds("k", "with space")),
            (b2("Photos"), creds("", "s")),
        ] {
            let e = s.add(&input, credentials).unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidInput, "{e:?}");
        }
        assert!(store.rows.lock().unwrap().is_empty());
        assert!(keys.ids().is_empty());
    }

    #[test]
    fn an_unavailable_or_locked_credential_store_saves_no_account() {
        for (error, kind) in [
            (
                unavailable as fn() -> CredentialError,
                ErrorKind::CredentialStoreUnavailable,
            ),
            (locked, ErrorKind::CredentialStoreLocked),
        ] {
            let (s, store, keys) = state();
            *keys.fail_store.lock().unwrap() = denied(error);
            let e = s.add(&b2("Photos"), creds(KEY_ID, SECRET)).unwrap_err();
            assert_eq!(e.kind, kind, "{e:?}");
            assert!(store.rows.lock().unwrap().is_empty());
            assert!(s.list().accounts.is_empty());
        }
    }

    #[test]
    fn editing_without_new_credentials_never_touches_the_credential_store() {
        let (s, store, keys) = state();
        s.add(&b2("Photos"), creds(KEY_ID, SECRET)).unwrap();
        *keys.fail_store.lock().unwrap() = denied(unavailable);
        *keys.fail_load.lock().unwrap() = denied(unavailable);
        *keys.fail_delete.lock().unwrap() = denied(unavailable);
        let mut renamed = b2("Family photos");
        renamed.bucket = "soty-family".into();
        let info = s.update("photos", &renamed, None).unwrap();
        assert_eq!(
            (info.id.as_str(), info.label.as_str()),
            ("photos", "Family photos")
        );
        assert_eq!(store.rows.lock().unwrap()[0].buckets, ["soty-family"]);
        *keys.fail_load.lock().unwrap() = None;
        assert_eq!(keys.load("photos").unwrap(), stored(KEY_ID, SECRET));
    }

    #[test]
    fn editing_keeps_settings_the_form_does_not_show() {
        let (s, store, _keys) = state();
        s.add(&b2("Photos"), creds("k", "s")).unwrap();
        {
            let mut rows = store.rows.lock().unwrap();
            rows[0].path_style = Some(false);
            rows[0].max_concurrent_transfers = 7;
        }
        let keys = Arc::new(MemoryCredentialStore::with(&[("photos", "k", "s")]));
        let s = state_with(&store, &keys);
        s.update("photos", &b2("Renamed"), None).unwrap();
        let row = store.rows.lock().unwrap()[0].clone();
        assert_eq!(
            (row.path_style, row.max_concurrent_transfers),
            (Some(false), 7)
        );
    }

    #[test]
    fn new_credentials_replace_the_old_ones_only_once_stored() {
        let (s, store, keys) = state();
        s.add(&b2("Photos"), creds("old-key", "old-secret"))
            .unwrap();
        *keys.fail_store.lock().unwrap() = denied(locked);
        let e = s
            .update(
                "photos",
                &b2("Renamed"),
                Some(creds("new-key", "new-secret")),
            )
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::CredentialStoreLocked);
        assert_eq!(
            keys.load("photos").unwrap(),
            stored("old-key", "old-secret")
        );
        assert_eq!(
            store.rows.lock().unwrap()[0].label,
            "Photos",
            "nothing saved"
        );

        // Invalid new credentials or configuration: also nothing changes.
        *keys.fail_store.lock().unwrap() = None;
        let e = s
            .update("photos", &b2("Renamed"), Some(creds("new key", "s")))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        let mut bad = b2("Renamed");
        bad.region = "Bad".into();
        let e = s
            .update("photos", &bad, Some(creds("new-key", "new-secret")))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert_eq!(
            keys.load("photos").unwrap(),
            stored("old-key", "old-secret")
        );

        let info = s
            .update(
                "photos",
                &b2("Renamed"),
                Some(creds("new-key", "new-secret")),
            )
            .unwrap();
        assert_eq!(info.label, "Renamed");
        assert_eq!(
            keys.load("photos").unwrap(),
            stored("new-key", "new-secret")
        );
        assert_eq!(keys.ids(), ["photos"], "the same entry, replaced");
    }

    #[test]
    fn a_failed_save_after_new_credentials_is_reported_as_partial() {
        let (s, store, keys) = state();
        s.add(&b2("Photos"), creds("old-key", "old-secret"))
            .unwrap();
        let failing = MemoryAccountStore {
            fail_saves: true,
            ..store.clone()
        };
        let s2 = state_with(&failing, &keys);
        drop(s);
        let e = s2
            .update(
                "photos",
                &b2("Renamed"),
                Some(creds("new-key", "new-secret")),
            )
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Partial, "{e:?}");
        assert!(
            e.message.contains("new access key was saved"),
            "{}",
            e.message
        );
        assert_eq!(
            keys.load("photos").unwrap(),
            stored("new-key", "new-secret")
        );
        assert_eq!(store.rows.lock().unwrap()[0].label, "Photos");
        // Only a key change, no other change: no configuration save needed.
        s2.update("photos", &b2("Photos"), Some(creds("k3", "s3")))
            .unwrap();
        // A configuration change alone fails as a storage error.
        let e = s2.update("photos", &b2("Renamed"), None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
    }

    #[test]
    fn the_provider_of_an_account_cannot_be_changed() {
        let (s, _store, _keys) = state();
        s.add(&b2("Photos"), creds("k", "s")).unwrap();
        let mut other = b2("Photos");
        other.provider = "hetzner".into();
        other.region = "fsn1".into();
        let e = s.update("photos", &other, None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn removal_keeps_the_account_when_its_credentials_cannot_be_deleted() {
        let (s, store, keys) = state();
        s.add(&b2("Photos"), creds("k", "s")).unwrap();
        *keys.fail_delete.lock().unwrap() = denied(unavailable);
        let e = s.remove("photos").unwrap_err();
        assert_eq!(e.kind, ErrorKind::CredentialStoreUnavailable);
        assert_eq!(s.list().accounts.len(), 1, "never a silent success");
        assert_eq!(keys.ids(), ["photos"]);

        *keys.fail_delete.lock().unwrap() = None;
        let failing = MemoryAccountStore {
            fail_removes: true,
            ..store.clone()
        };
        let s = state_with(&failing, &keys);
        let e = s.remove("photos").unwrap_err();
        assert_eq!(e.kind, ErrorKind::Partial, "{e:?}");
        assert!(e.message.contains("remove it again"), "{}", e.message);
        assert_eq!(s.list().accounts.len(), 1);

        let s = state_with(&store, &keys);
        s.remove("photos").unwrap();
        assert!(s.list().accounts.is_empty() && keys.ids().is_empty());
    }

    #[test]
    fn unknown_accounts_are_reported_by_kind() {
        let (s, _store, _keys) = state();
        assert_eq!(
            s.update("nope", &b2("x"), None).unwrap_err().kind,
            ErrorKind::UnknownAccount
        );
        assert_eq!(
            s.remove("nope").unwrap_err().kind,
            ErrorKind::UnknownAccount
        );
        assert!(matches!(
            s.prepare_check("nope"),
            Err(CommandError {
                kind: ErrorKind::UnknownAccount,
                ..
            })
        ));
    }

    #[test]
    fn an_unusable_account_database_is_shown_and_refuses_changes() {
        let keys = Arc::new(MemoryCredentialStore::default());
        let s = CloudAccounts::load(
            Err(StoreError::new("the database is damaged")),
            keys.clone(),
            test_options(),
        );
        let list = s.list();
        assert!(list.accounts.is_empty());
        assert_eq!(
            list.storage_error.as_deref(),
            Some("the database is damaged")
        );
        let e = s.add(&b2("Photos"), creds("k", "s")).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(keys.ids().is_empty(), "nothing stored without a database");
    }

    #[test]
    fn an_account_that_no_longer_validates_is_listed_with_its_error() {
        let store = MemoryAccountStore::default();
        let mut old = AccountConfig::new("old", "Old", "retired-provider", "x");
        old.buckets = vec!["soty-old".into()];
        store.rows.lock().unwrap().push(old);
        let keys = Arc::new(MemoryCredentialStore::default());
        let s = state_with(&store, &keys);
        let info = &s.list().accounts[0];
        assert_eq!(
            (info.provider_name, info.endpoint_url.as_deref()),
            (None, None)
        );
        assert!(
            info.config_error
                .as_deref()
                .unwrap()
                .contains("unknown provider")
        );
    }

    #[test]
    fn credential_input_never_shows_its_secret() {
        let input: CredentialInput = serde_json::from_value(serde_json::json!({
            "accessKeyId": KEY_ID,
            "secretAccessKey": SECRET
        }))
        .unwrap();
        assert_eq!(format!("{input:?}"), "CredentialInput(<redacted>)");
        assert!(
            serde_json::from_value::<CredentialInput>(serde_json::json!({
                "accessKeyId": "k", "secretAccessKey": "s", "extra": 1
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<AccountInput>(serde_json::json!({
                "label": "x", "provider": "hetzner", "region": "fsn1", "bucket": "b",
                "secretAccessKey": "s"
            }))
            .is_err(),
            "a secret in the account part is refused, not silently stored"
        );
    }

    #[test]
    fn errors_never_contain_a_secret() {
        let (s, store, keys) = state();
        s.add(&b2("Photos"), creds(KEY_ID, SECRET)).unwrap();
        let mut shown = String::new();
        let mut push = |e: CommandError| {
            shown.push_str(&format!("{e:?} {} ", serde_json::to_string(&e).unwrap()));
        };
        push(
            s.add(&b2("Photos"), creds(KEY_ID, &format!("{SECRET} x")))
                .unwrap_err(),
        );
        *keys.fail_store.lock().unwrap() = denied(locked);
        push(s.add(&b2("Other"), creds(KEY_ID, SECRET)).unwrap_err());
        push(
            s.update("photos", &b2("x"), Some(creds(KEY_ID, SECRET)))
                .unwrap_err(),
        );
        *keys.fail_store.lock().unwrap() = None;
        let failing = MemoryAccountStore {
            fail_saves: true,
            ..store.clone()
        };
        let s2 = state_with(&failing, &keys);
        push(
            s2.update("photos", &b2("x"), Some(creds(KEY_ID, SECRET)))
                .unwrap_err(),
        );
        push(s2.add(&b2("Third"), creds(KEY_ID, SECRET)).unwrap_err());
        assert!(!shown.contains("very-secret"), "{shown}");
    }

    // ---- connection checks against the in-process S3 server ----

    fn local(server: &TestServer, label: &str, bucket: &str) -> AccountInput {
        AccountInput {
            label: label.into(),
            provider: "s3-compatible".into(),
            region: "us-east-1".into(),
            endpoint: Some(server.endpoint.clone()),
            bucket: bucket.into(),
        }
    }

    async fn checked(
        server: &TestServer,
        bucket: &str,
        key: &str,
        secret: &str,
    ) -> ConnectionCheck {
        let (s, _store, _keys) = state();
        let id = s
            .add(&local(server, "Local", bucket), creds(key, secret))
            .unwrap()
            .id;
        let info = check_account(Arc::clone(&s), id).await.unwrap();
        let check = info.last_check.clone().expect("the check is recorded");
        assert_eq!(s.list().accounts[0].last_check.as_ref(), Some(&check));
        check
    }

    fn operations(server: &TestServer) -> Vec<String> {
        server.operations.lock().unwrap().clone()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_valid_key_is_connected_with_one_head_bucket_and_no_bucket_listing() {
        let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
        let check = checked(&server, BUCKET, "test-key", "test-secret").await;
        assert_eq!(
            (check.status, check.message),
            (CheckStatus::Connected, None)
        );
        assert_eq!(check.bucket.as_deref(), Some(BUCKET));
        assert_eq!(operations(&server), ["HeadBucket"]);
    }

    /// A Backblaze key restricted to one bucket is refused `ListBuckets`
    /// ("not entitled", seen against B2 on 2026-11-11); it is still a valid
    /// key for its bucket.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_key_refused_bucket_listing_is_still_connected() {
        let server =
            TestServer::start_denying("test-key", "test-secret", &[BUCKET], &["ListBuckets"]).await;
        let check = checked(&server, BUCKET, "test-key", "test-secret").await;
        assert_eq!(check.status, CheckStatus::Connected);
        assert!(!operations(&server).contains(&"ListBuckets".to_owned()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_head_is_settled_by_listing_one_key_of_the_bucket() {
        let server = TestServer::start_denying(
            "test-key",
            "test-secret",
            &[BUCKET],
            &["ListBuckets", "HeadBucket"],
        )
        .await;
        let check = checked(&server, BUCKET, "test-key", "test-secret").await;
        assert_eq!(check.status, CheckStatus::Connected);
        assert_eq!(operations(&server), ["HeadBucket", "ListObjectsV2"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_valid_key_without_permission_is_access_denied_not_invalid() {
        let server = TestServer::start_denying(
            "test-key",
            "test-secret",
            &[BUCKET],
            &["ListBuckets", "HeadBucket", "ListObjectsV2"],
        )
        .await;
        let check = checked(&server, BUCKET, "test-key", "test-secret").await;
        assert_eq!(check.status, CheckStatus::AccessDenied, "{check:?}");
        assert_eq!(operations(&server), ["HeadBucket", "ListObjectsV2"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_secret_or_key_id_is_invalid_credentials() {
        let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
        for (key, secret) in [
            ("test-key", "wrong-secret-value"),
            ("unknown-key", "test-secret"),
        ] {
            let check = checked(&server, BUCKET, key, secret).await;
            assert_eq!(
                check.status,
                CheckStatus::InvalidCredentials,
                "{key}: {check:?}"
            );
            let shown = serde_json::to_string(&check).unwrap();
            assert!(!shown.contains("wrong-secret-value"), "{shown}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_bucket_is_bucket_unavailable() {
        let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
        let check = checked(&server, "no-such-bucket", "test-key", "test-secret").await;
        assert_eq!(check.status, CheckStatus::BucketUnavailable, "{check:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_endpoint_is_a_network_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (s, _store, _keys) = state();
        let mut input = b2("Down");
        input.provider = "s3-compatible".into();
        input.region = "us-east-1".into();
        input.endpoint = Some(format!("http://127.0.0.1:{port}"));
        let id = s.add(&input, creds("k", "s")).unwrap().id;
        let info = check_account(s, id).await.unwrap();
        assert_eq!(info.last_check.unwrap().status, CheckStatus::Network);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn credential_store_problems_are_their_own_results_and_send_nothing() {
        let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
        for (setup, want) in [
            (0, CheckStatus::CredentialsMissing),
            (1, CheckStatus::CredentialStoreUnavailable),
            (2, CheckStatus::CredentialStoreLocked),
        ] {
            let (s, _store, keys) = state();
            let id = s
                .add(
                    &local(&server, "Local", BUCKET),
                    creds("test-key", "test-secret"),
                )
                .unwrap()
                .id;
            match setup {
                0 => keys.entries.lock().unwrap().clear(),
                1 => *keys.fail_load.lock().unwrap() = denied(unavailable),
                _ => *keys.fail_load.lock().unwrap() = denied(locked),
            }
            let check = check_account(s, id).await.unwrap().last_check.unwrap();
            assert_eq!(check.status, want, "{check:?}");
            assert!(check.message.is_some());
        }
        assert!(
            operations(&server).is_empty(),
            "no request without credentials"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_check_result_is_dropped_when_the_account_changes() {
        let server = TestServer::start("test-key", "test-secret", &[BUCKET]).await;
        let (s, _store, _keys) = state();
        let id = s
            .add(
                &local(&server, "Local", BUCKET),
                creds("test-key", "test-secret"),
            )
            .unwrap()
            .id;
        let info = check_account(Arc::clone(&s), id.clone()).await.unwrap();
        assert_eq!(info.last_check.unwrap().status, CheckStatus::Connected);
        let info = s
            .update(&id, &local(&server, "Renamed", BUCKET), None)
            .unwrap();
        assert_eq!(info.last_check, None);
        // A check prepared before a change does not record its result.
        let Prepared::Ready { revision, .. } = s.prepare_check(&id).unwrap() else {
            panic!("expected a ready check");
        };
        s.update(&id, &local(&server, "Again", BUCKET), None)
            .unwrap();
        let late = ConnectionCheck {
            status: CheckStatus::Connected,
            message: None,
            bucket: None,
            checked_at: 0,
        };
        assert_eq!(s.record(&id, revision, late).unwrap().last_check, None);
        s.remove(&id).unwrap();
        assert_eq!(
            check_account(s, id).await.unwrap_err().kind,
            ErrorKind::UnknownAccount
        );
    }

    // ---- the credential store never blocks the async runtime ----

    /// A credential store whose calls wait until the test releases them, as
    /// the Secret Service does while an unlock prompt is open.
    struct WaitingStore {
        inner: MemoryCredentialStore,
        entered: AtomicBool,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl WaitingStore {
        fn wait(&self) {
            self.entered.store(true, Ordering::SeqCst);
            let _ = self.release.lock().unwrap().recv();
        }
    }

    impl CredentialStore for WaitingStore {
        fn store(&self, id: &str, c: &Credentials) -> Result<(), CredentialError> {
            self.wait();
            self.inner.store(id, c)
        }
        fn load(&self, id: &str) -> Result<Credentials, CredentialError> {
            self.wait();
            self.inner.load(id)
        }
        fn delete(&self, id: &str) -> Result<(), CredentialError> {
            self.inner.delete(id)
        }
    }

    /// One async thread only: if a credential call ran on it, nothing else
    /// could run until the call returned. A watchdog releases the call after
    /// 10 s so a regression fails instead of hanging.
    #[tokio::test(flavor = "current_thread")]
    async fn waiting_for_the_credential_store_leaves_the_async_thread_free() {
        let (release, wait) = mpsc::channel();
        let keys = Arc::new(WaitingStore {
            inner: MemoryCredentialStore::default(),
            entered: AtomicBool::new(false),
            release: Mutex::new(wait),
        });
        let s = Arc::new(CloudAccounts::load(
            Ok(Box::new(MemoryAccountStore::default())),
            Arc::clone(&keys) as Arc<dyn CredentialStore>,
            test_options(),
        ));
        let watchdog = {
            let release = release.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(10));
                let _ = release.send(());
                let _ = release.send(());
            })
        };

        let started = Instant::now();
        let adding = tokio::spawn(add_account(Arc::clone(&s), b2("Photos"), creds("k", "s")));
        while !keys.entered.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        // The add is waiting inside the credential store; this thread is not.
        assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the add blocked the runtime"
        );
        release.send(()).unwrap();
        let id = adding.await.unwrap().unwrap().id;

        keys.entered.store(false, Ordering::SeqCst);
        let started = Instant::now();
        let checking = tokio::spawn(check_account(Arc::clone(&s), id));
        while !keys.entered.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        assert_eq!(tokio::spawn(async { 8 }).await.unwrap(), 8);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the check blocked the runtime"
        );
        release.send(()).unwrap();
        // The account's endpoint is Backblaze's: no request is wanted here,
        // so only the credential part is awaited, then the task is dropped.
        checking.abort();
        drop(watchdog);
    }
}

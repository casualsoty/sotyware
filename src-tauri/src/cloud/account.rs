//! Cloud storage accounts: their configuration, validation and the set of
//! accounts sotyware knows.
//!
//! An account is one set of credentials at one provider (two Backblaze
//! accounts are two accounts). It names the buckets sotyware may use and holds
//! no secret: its credentials are kept by account id in a
//! [`CredentialStore`] (the OS credential service in the app). Configurations
//! are kept behind [`AccountStore`] (SQLite in `storage.rs`), the same split
//! as feature preferences. [`Accounts`] keeps the two consistent.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use super::credentials::{CredentialError, CredentialStore, Credentials};
use super::provider::{self, Addressing, EndpointRule, ProviderProfile};
use super::s3::{ClientOptions, S3Store};
use crate::features::StoreError;

/// Default number of transfers an account runs at once.
pub const DEFAULT_CONCURRENT_TRANSFERS: u8 = 4;
/// Upper bound for one account's concurrent transfers.
pub const MAX_CONCURRENT_TRANSFERS: u8 = 32;

/// One configured account, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountConfig {
    /// Stable, lowercase kebab-case; the key of the stored configuration and
    /// of the credentials. Never change it once saved.
    pub id: String,
    /// Shown to the user.
    pub label: String,
    /// A [`ProviderProfile::id`].
    pub provider: String,
    /// The provider's region id (`eu-central-003`, `fsn1`); free text for a
    /// custom endpoint.
    pub region: String,
    /// Required for `s3-compatible`; overrides the regional endpoint otherwise.
    pub endpoint: Option<String>,
    /// Overrides the provider's addressing style.
    pub path_style: Option<bool>,
    /// The buckets sotyware may use with this account. Empty means any bucket
    /// the credentials can reach.
    pub buckets: Vec<String>,
    /// How many uploads/downloads of this account run at once (its own queue).
    pub max_concurrent_transfers: u8,
}

impl AccountConfig {
    /// A configuration with the defaults for everything optional.
    pub fn new(id: &str, label: &str, provider: &str, region: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            provider: provider.into(),
            region: region.into(),
            endpoint: None,
            path_style: None,
            buckets: Vec::new(),
            max_concurrent_transfers: DEFAULT_CONCURRENT_TRANSFERS,
        }
    }
}

/// Why a configuration is not usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(String);

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated account: where its requests go and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAccount {
    pub id: String,
    pub profile: &'static ProviderProfile,
    pub endpoint_url: String,
    pub region: String,
    pub path_style: bool,
    pub buckets: Vec<String>,
    pub max_concurrent_transfers: u8,
}

/// Account ids are lowercase kebab-case, at most 64 characters. They become
/// database keys and OS credential names, so nothing else is allowed.
pub(crate) fn is_kebab_case(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A region goes into a host name, so only DNS label characters are allowed.
fn is_valid_region(region: &str) -> bool {
    !region.is_empty()
        && region.len() <= 63
        && region
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// S3 bucket naming rules shared by Backblaze and Hetzner: 3 to 63 characters,
/// lowercase letters, digits, `-` and `.`, starting and ending with a letter or
/// digit, no `..`.
pub fn is_valid_bucket_name(name: &str) -> bool {
    let edge_ok = |c: Option<char>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    (3..=63).contains(&name.len())
        && edge_ok(name.chars().next())
        && edge_ok(name.chars().last())
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// Checks an endpoint URL: `https://host[:port]` with nothing after the host.
/// Plain `http` only for the local machine (a test server), never across a
/// network, where it would send signed requests and data in the clear.
fn check_endpoint(url: &str) -> Result<(), ConfigError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ConfigError::new(format!("endpoint '{url}' has no scheme")))?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return Err(ConfigError::new(format!(
            "endpoint '{url}' must be scheme://host[:port] only"
        )));
    }
    let host = if let Some(v6) = rest.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        rest.split(':').next().unwrap_or_default()
    };
    match scheme {
        "https" => Ok(()),
        "http" if matches!(host, "localhost" | "127.0.0.1" | "::1") => Ok(()),
        "http" => Err(ConfigError::new(format!(
            "endpoint '{url}' must use https (plain http only on this machine)"
        ))),
        _ => Err(ConfigError::new(format!(
            "endpoint '{url}' has an unsupported scheme"
        ))),
    }
}

impl AccountConfig {
    /// Validates the configuration and works out its endpoint.
    pub fn resolve(&self) -> Result<ResolvedAccount, ConfigError> {
        if !is_kebab_case(&self.id) {
            return Err(ConfigError::new(format!(
                "account id '{}' must be lowercase kebab-case",
                self.id
            )));
        }
        if self.label.trim().is_empty() {
            return Err(ConfigError::new(format!(
                "account '{}' needs a label",
                self.id
            )));
        }
        let profile = provider::profile(&self.provider)
            .ok_or_else(|| ConfigError::new(format!("unknown provider '{}'", self.provider)))?;
        if !is_valid_region(&self.region) {
            return Err(ConfigError::new(format!(
                "region '{}' must be lowercase letters, digits and '-'",
                self.region
            )));
        }
        let endpoint_url = match (&self.endpoint, profile.endpoint) {
            (Some(url), _) => {
                check_endpoint(url)?;
                url.trim_end_matches('/').to_owned()
            }
            (None, EndpointRule::Regional { prefix, suffix }) => {
                format!("{prefix}{}{suffix}", self.region)
            }
            (None, EndpointRule::Custom) => {
                return Err(ConfigError::new(format!(
                    "provider '{}' needs an endpoint",
                    profile.id
                )));
            }
        };
        if let Some(bad) = self.buckets.iter().find(|b| !is_valid_bucket_name(b)) {
            return Err(ConfigError::new(format!("invalid bucket name '{bad}'")));
        }
        let mut seen = self.buckets.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != self.buckets.len() {
            return Err(ConfigError::new("a bucket is listed twice"));
        }
        if !(1..=MAX_CONCURRENT_TRANSFERS).contains(&self.max_concurrent_transfers) {
            return Err(ConfigError::new(format!(
                "max_concurrent_transfers must be 1 to {MAX_CONCURRENT_TRANSFERS}"
            )));
        }
        let path_style = self
            .path_style
            .unwrap_or(profile.addressing == Addressing::Path);
        // Virtual-hosted style puts the bucket into the TLS host name; a dot in
        // the bucket name would then not match the provider's certificate.
        if !path_style && self.buckets.iter().any(|b| b.contains('.')) {
            return Err(ConfigError::new(
                "bucket names with '.' need path-style addressing",
            ));
        }
        Ok(ResolvedAccount {
            id: self.id.clone(),
            profile,
            endpoint_url,
            region: self.region.clone(),
            path_style,
            buckets: self.buckets.clone(),
            max_concurrent_transfers: self.max_concurrent_transfers,
        })
    }
}

/// Where account configurations are kept. Implemented with SQLite in
/// `storage.rs`; tests use an in-memory store.
pub trait AccountStore: Send {
    /// Every stored account, in id order.
    fn load(&mut self) -> Result<Vec<AccountConfig>, StoreError>;
    /// Inserts or replaces the account with `config.id`.
    fn save(&mut self, config: &AccountConfig) -> Result<(), StoreError>;
    /// Removes the account; removing an unknown id is not an error.
    fn remove(&mut self, id: &str) -> Result<(), StoreError>;
}

/// Why an account operation failed. No variant carries a secret.
#[derive(Debug)]
pub enum AccountError {
    Config(ConfigError),
    Store(StoreError),
    Credentials(CredentialError),
    Duplicate(String),
    Unknown(String),
    /// Adding failed after the credentials were stored, and removing them
    /// again failed too: they are left in the OS credential store under the
    /// account id. Adding the account again overwrites them.
    AddRollbackFailed {
        id: String,
        save: StoreError,
        cleanup: CredentialError,
    },
    /// Removing failed after the credentials were deleted: the account is
    /// still listed but has no credentials. Removing it again finishes.
    RemovedOnlyCredentials {
        id: String,
        error: StoreError,
    },
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "invalid account: {e}"),
            Self::Store(e) => write!(f, "cannot save the account: {e}"),
            Self::Credentials(e) => e.fmt(f),
            Self::Duplicate(id) => write!(f, "an account '{id}' already exists"),
            Self::Unknown(id) => write!(f, "no account '{id}'"),
            Self::AddRollbackFailed { id, save, cleanup } => write!(
                f,
                "account '{id}' was not added ({save}), and its credentials could not \
                 be removed again ({cleanup}); adding the account again replaces them"
            ),
            Self::RemovedOnlyCredentials { id, error } => write!(
                f,
                "the credentials of account '{id}' were removed, but the account \
                 itself could not be ({error}); remove it again"
            ),
        }
    }
}

struct Inner {
    store: Box<dyn AccountStore>,
    configs: Vec<AccountConfig>,
    /// One client (and so one connection pool and one transfer queue) per
    /// account, created on first use.
    connected: HashMap<String, Arc<S3Store>>,
}

/// Every configured account, their credentials, and a connection to each
/// one in use.
///
/// Holds exactly one [`CredentialStore`]; there is no second source to fall
/// back to. Operations that touch both stores run under one lock, in an
/// order that never leaves a secret behind silently: see [`Self::add`] and
/// [`Self::remove`].
pub struct Accounts {
    inner: Mutex<Inner>,
    credentials: Arc<dyn CredentialStore>,
    options: ClientOptions,
}

impl Accounts {
    /// Loads the stored accounts. A stored account that no longer validates
    /// (for example a provider this build does not know) is kept in the list
    /// and in the store, and fails only when used. Credentials are not read
    /// here (on Linux that could open an unlock prompt at startup).
    pub fn load(
        mut store: Box<dyn AccountStore>,
        credentials: Arc<dyn CredentialStore>,
        options: ClientOptions,
    ) -> Result<Self, StoreError> {
        let configs = store.load()?;
        Ok(Self {
            inner: Mutex::new(Inner {
                store,
                configs,
                connected: HashMap::new(),
            }),
            credentials,
            options,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<AccountConfig> {
        self.lock().configs.clone()
    }

    /// Validates and saves a new account with its credentials.
    ///
    /// Order: credentials first, then the configuration. If storing the
    /// credentials fails, nothing was saved. If saving the configuration
    /// fails, the credentials just stored are deleted again; only if that
    /// delete fails too is something left behind, and the error
    /// ([`AccountError::AddRollbackFailed`]) says so.
    pub fn add(
        &self,
        config: AccountConfig,
        credentials: &Credentials,
    ) -> Result<(), AccountError> {
        config.resolve().map_err(AccountError::Config)?;
        credentials.validate().map_err(|reason| {
            AccountError::Credentials(CredentialError::Invalid {
                account_id: config.id.clone(),
                reason: reason.into(),
            })
        })?;
        let mut inner = self.lock();
        if inner.configs.iter().any(|c| c.id == config.id) {
            return Err(AccountError::Duplicate(config.id));
        }
        self.credentials
            .store(&config.id, credentials)
            .map_err(AccountError::Credentials)?;
        if let Err(save) = inner.store.save(&config) {
            return Err(match self.credentials.delete(&config.id) {
                Ok(()) => AccountError::Store(save),
                Err(cleanup) => AccountError::AddRollbackFailed {
                    id: config.id,
                    save,
                    cleanup,
                },
            });
        }
        inner.configs.push(config);
        inner.configs.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }

    /// Replaces an existing account's configuration. Its connection is dropped
    /// so the next use picks up the new settings.
    pub fn update(&self, config: AccountConfig) -> Result<(), AccountError> {
        config.resolve().map_err(AccountError::Config)?;
        let mut inner = self.lock();
        let Some(pos) = inner.configs.iter().position(|c| c.id == config.id) else {
            return Err(AccountError::Unknown(config.id));
        };
        inner.store.save(&config).map_err(AccountError::Store)?;
        inner.connected.remove(&config.id);
        inner.configs[pos] = config;
        Ok(())
    }

    /// Replaces an existing account's credentials (a new or rotated key) in
    /// one write: the old ones are never deleted first, so a failed write
    /// leaves them in place. Its connection is dropped so the next use signs
    /// with the new key.
    pub fn set_credentials(&self, id: &str, credentials: &Credentials) -> Result<(), AccountError> {
        credentials.validate().map_err(|reason| {
            AccountError::Credentials(CredentialError::Invalid {
                account_id: id.to_owned(),
                reason: reason.into(),
            })
        })?;
        let mut inner = self.lock();
        if !inner.configs.iter().any(|c| c.id == id) {
            return Err(AccountError::Unknown(id.into()));
        }
        self.credentials
            .store(id, credentials)
            .map_err(AccountError::Credentials)?;
        inner.connected.remove(id);
        Ok(())
    }

    /// Whether the account has credentials stored, without reading them into
    /// the caller.
    pub fn has_credentials(&self, id: &str) -> Result<bool, AccountError> {
        if !self.lock().configs.iter().any(|c| c.id == id) {
            return Err(AccountError::Unknown(id.into()));
        }
        self.credentials
            .exists(id)
            .map_err(AccountError::Credentials)
    }

    /// Forgets an account and deletes its credentials. **Its buckets and
    /// objects at the provider are never touched**: no request is sent.
    ///
    /// Order: credentials first, then the configuration. If deleting the
    /// credentials fails, nothing changes and the error is returned (the
    /// account stays listed, so the user can retry; never a silent success
    /// with a secret left behind). If removing the configuration then fails,
    /// the result is [`AccountError::RemovedOnlyCredentials`]. Credentials
    /// that were already missing do not stop the removal.
    pub fn remove(&self, id: &str) -> Result<(), AccountError> {
        let mut inner = self.lock();
        if !inner.configs.iter().any(|c| c.id == id) {
            return Err(AccountError::Unknown(id.into()));
        }
        self.credentials
            .delete(id)
            .map_err(AccountError::Credentials)?;
        inner.connected.remove(id);
        inner
            .store
            .remove(id)
            .map_err(|error| AccountError::RemovedOnlyCredentials {
                id: id.into(),
                error,
            })?;
        inner.configs.retain(|c| c.id != id);
        Ok(())
    }

    /// The client settings every account's store is built with.
    pub fn options(&self) -> &ClientOptions {
        &self.options
    }

    /// The account's store, connected with its own credentials from the
    /// credential store. No request is sent here: a wrong key or an
    /// unreachable endpoint shows on first use.
    pub fn connect(&self, id: &str) -> Result<Arc<S3Store>, AccountError> {
        let mut inner = self.lock();
        if let Some(store) = inner.connected.get(id) {
            return Ok(Arc::clone(store));
        }
        let config = inner
            .configs
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| AccountError::Unknown(id.into()))?;
        let resolved = config.resolve().map_err(AccountError::Config)?;
        let creds = self
            .credentials
            .load(id)
            .map_err(AccountError::Credentials)?;
        let store = Arc::new(S3Store::new(resolved, creds, self.options.clone()));
        inner.connected.insert(id.into(), Arc::clone(&store));
        Ok(store)
    }
}

/// An account store in memory, for tests.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MemoryAccountStore {
    pub rows: Arc<Mutex<Vec<AccountConfig>>>,
    pub fail_saves: bool,
    pub fail_removes: bool,
}

#[cfg(test)]
impl AccountStore for MemoryAccountStore {
    fn load(&mut self) -> Result<Vec<AccountConfig>, StoreError> {
        Ok(self.rows.lock().unwrap().clone())
    }

    fn save(&mut self, config: &AccountConfig) -> Result<(), StoreError> {
        if self.fail_saves {
            return Err(StoreError::new("disk full"));
        }
        let mut rows = self.rows.lock().unwrap();
        rows.retain(|c| c.id != config.id);
        rows.push(config.clone());
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }

    fn remove(&mut self, id: &str) -> Result<(), StoreError> {
        if self.fail_removes {
            return Err(StoreError::new("disk full"));
        }
        self.rows.lock().unwrap().retain(|c| c.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::credentials::{MemoryCredentialStore, Secret};
    use super::*;

    fn b2() -> AccountConfig {
        let mut c = AccountConfig::new("b2-photos", "Photos", "backblaze-b2", "eu-central-003");
        c.buckets = vec!["soty-photos".into()];
        c
    }

    fn hetzner() -> AccountConfig {
        let mut c = AccountConfig::new("hetzner-backup", "Backups", "hetzner", "fsn1");
        c.buckets = vec!["soty-backup".into(), "soty-projects".into()];
        c
    }

    #[test]
    fn regional_endpoints_and_addressing_come_from_the_profile() {
        let r = b2().resolve().unwrap();
        assert_eq!(r.endpoint_url, "https://s3.eu-central-003.backblazeb2.com");
        assert!(r.path_style);
        let r = hetzner().resolve().unwrap();
        assert_eq!(r.endpoint_url, "https://fsn1.your-objectstorage.com");
        assert!(!r.path_style);
        assert_eq!(r.buckets.len(), 2);
    }

    #[test]
    fn an_endpoint_and_addressing_can_be_overridden() {
        let mut c = hetzner();
        c.endpoint = Some("https://nbg1.your-objectstorage.com/".into());
        c.path_style = Some(true);
        let r = c.resolve().unwrap();
        assert_eq!(r.endpoint_url, "https://nbg1.your-objectstorage.com");
        assert!(r.path_style);
    }

    #[test]
    fn a_custom_provider_needs_an_endpoint() {
        let mut c = AccountConfig::new("local", "Local", "s3-compatible", "us-east-1");
        assert!(c.resolve().is_err());
        c.endpoint = Some("http://127.0.0.1:9000".into());
        assert_eq!(c.resolve().unwrap().endpoint_url, "http://127.0.0.1:9000");
    }

    #[test]
    fn plain_http_is_refused_except_on_this_machine() {
        let mut c = AccountConfig::new("x", "X", "s3-compatible", "r");
        for (url, ok) in [
            ("https://s3.example.com", true),
            ("https://s3.example.com:8443", true),
            ("http://localhost:9000", true),
            ("http://[::1]:9000", true),
            ("http://s3.example.com", false),
            ("http://127.0.0.1.example.com", false),
            ("ftp://s3.example.com", false),
            ("s3.example.com", false),
            ("https://s3.example.com/bucket", false),
            ("https://user@s3.example.com", false),
            ("https://", false),
        ] {
            c.endpoint = Some(url.into());
            assert_eq!(c.resolve().is_ok(), ok, "{url}");
        }
    }

    #[test]
    fn ids_regions_and_labels_are_validated() {
        for id in ["", "B2", "b2_photos", "-b2", "b2-", "b2--x", "b2 photos"] {
            let mut c = b2();
            c.id = id.into();
            assert!(c.resolve().is_err(), "id {id:?}");
        }
        for region in ["", "EU", "eu.central", "eu/central", "x.evil.com/"] {
            let mut c = b2();
            c.region = region.into();
            assert!(c.resolve().is_err(), "region {region:?}");
        }
        let mut c = b2();
        c.label = "  ".into();
        assert!(c.resolve().is_err());
        let mut c = b2();
        c.provider = "google-drive".into();
        assert!(c.resolve().is_err());
    }

    #[test]
    fn bucket_names_follow_the_s3_rules() {
        for (name, ok) in [
            ("abc", true),
            ("soty-photos-2026", true),
            ("with.dots", true),
            ("ab", false),
            (&"a".repeat(64), false),
            ("Upper", false),
            ("-start", false),
            ("end-", false),
            ("double..dot", false),
            ("under_score", false),
        ] {
            assert_eq!(is_valid_bucket_name(name), ok, "{name}");
        }
        let mut c = b2();
        c.buckets = vec!["a".into()];
        assert!(c.resolve().is_err());
        c.buckets = vec!["same-name".into(), "same-name".into()];
        assert!(c.resolve().is_err());
    }

    #[test]
    fn dotted_bucket_names_need_path_style() {
        let mut c = hetzner();
        c.buckets = vec!["soty.backup".into()];
        assert!(c.resolve().is_err());
        c.path_style = Some(true);
        assert!(c.resolve().is_ok());
    }

    #[test]
    fn transfer_concurrency_is_bounded() {
        for (n, ok) in [(0, false), (1, true), (32, true), (33, false)] {
            let mut c = b2();
            c.max_concurrent_transfers = n;
            assert_eq!(c.resolve().is_ok(), ok, "{n}");
        }
    }

    fn creds(key: &str, secret: &str) -> Credentials {
        Credentials {
            access_key_id: key.into(),
            secret_access_key: Secret::new(secret),
        }
    }

    fn accounts_with(store: &MemoryAccountStore, keys: &Arc<MemoryCredentialStore>) -> Accounts {
        Accounts::load(
            Box::new(store.clone()),
            Arc::clone(keys) as Arc<dyn CredentialStore>,
            ClientOptions::default(),
        )
        .unwrap()
    }

    fn accounts(store: &MemoryAccountStore) -> (Accounts, Arc<MemoryCredentialStore>) {
        let keys = Arc::new(MemoryCredentialStore::default());
        (accounts_with(store, &keys), keys)
    }

    #[test]
    fn accounts_are_added_listed_updated_and_removed_through_the_store() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(hetzner(), &creds("kh", "sh")).unwrap();
        a.add(b2(), &creds("kb", "sb")).unwrap();
        let ids: Vec<_> = a.list().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["b2-photos", "hetzner-backup"]);
        assert_eq!(*store.rows.lock().unwrap(), a.list());
        assert_eq!(keys.ids(), ["b2-photos", "hetzner-backup"]);

        let mut changed = b2();
        changed.label = "Family photos".into();
        a.update(changed.clone()).unwrap();
        assert!(store.rows.lock().unwrap().contains(&changed));
        // A configuration change leaves the credentials alone.
        assert_eq!(keys.load("b2-photos").unwrap(), creds("kb", "sb"));

        a.remove("hetzner-backup").unwrap();
        assert_eq!(a.list(), [changed]);
        assert_eq!(store.rows.lock().unwrap().len(), 1);
        assert_eq!(
            keys.ids(),
            ["b2-photos"],
            "removing deletes the credentials"
        );
        // A restart sees the same accounts and credentials.
        let again = accounts_with(&store, &keys);
        assert_eq!(again.list(), a.list());
        assert!(again.has_credentials("b2-photos").unwrap());
    }

    #[test]
    fn two_accounts_of_the_same_provider_are_independent() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        let mut second = b2();
        second.id = "b2-work".into();
        second.buckets = vec!["soty-work".into()];
        a.add(b2(), &creds("k1", "s1")).unwrap();
        a.add(second, &creds("k2", "s2")).unwrap();
        let photos = a.connect("b2-photos").unwrap();
        let work = a.connect("b2-work").unwrap();
        assert!(!Arc::ptr_eq(&photos, &work));
        assert_eq!(photos.access_key_id(), "k1");
        assert_eq!(work.access_key_id(), "k2");
        // The same account reuses its one connection (and transfer queue).
        assert!(Arc::ptr_eq(&photos, &a.connect("b2-photos").unwrap()));

        // Rotating one account's key changes only that account.
        a.set_credentials("b2-work", &creds("k2-new", "s2-new"))
            .unwrap();
        assert_eq!(keys.load("b2-photos").unwrap(), creds("k1", "s1"));
        assert_eq!(a.connect("b2-work").unwrap().access_key_id(), "k2-new");
        assert!(Arc::ptr_eq(&photos, &a.connect("b2-photos").unwrap()));

        // Removing one leaves the other's credentials.
        a.remove("b2-photos").unwrap();
        assert_eq!(keys.ids(), ["b2-work"]);
        assert_eq!(a.connect("b2-work").unwrap().access_key_id(), "k2-new");
    }

    #[test]
    fn invalid_duplicate_and_unknown_accounts_are_rejected_without_saving() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k", "s")).unwrap();
        assert!(matches!(
            a.add(b2(), &creds("other", "other-secret")),
            Err(AccountError::Duplicate(_))
        ));
        assert_eq!(
            keys.load("b2-photos").unwrap(),
            creds("k", "s"),
            "a duplicate add must not overwrite the existing credentials"
        );
        let mut bad = hetzner();
        bad.region = "Bad".into();
        assert!(matches!(
            a.add(bad, &creds("k", "s")),
            Err(AccountError::Config(_))
        ));
        assert!(matches!(
            a.add(hetzner(), &creds("k", "with space")),
            Err(AccountError::Credentials(CredentialError::Invalid { .. }))
        ));
        assert!(matches!(a.update(hetzner()), Err(AccountError::Unknown(_))));
        assert!(matches!(
            a.set_credentials("nope", &creds("k", "s")),
            Err(AccountError::Unknown(_))
        ));
        assert!(matches!(a.remove("nope"), Err(AccountError::Unknown(_))));
        assert!(matches!(
            a.has_credentials("nope"),
            Err(AccountError::Unknown(_))
        ));
        assert_eq!(store.rows.lock().unwrap().len(), 1);
        assert_eq!(
            keys.ids(),
            ["b2-photos"],
            "nothing stored for rejected adds"
        );
    }

    #[test]
    fn a_failed_save_stores_no_credentials() {
        let store = MemoryAccountStore {
            fail_saves: true,
            ..Default::default()
        };
        let (a, keys) = accounts(&store);
        assert!(matches!(
            a.add(b2(), &creds("k", "s")),
            Err(AccountError::Store(_))
        ));
        assert!(a.list().is_empty());
        assert!(keys.ids().is_empty(), "the credentials were rolled back");
    }

    #[test]
    fn a_failed_rollback_is_reported_not_hidden() {
        let store = MemoryAccountStore {
            fail_saves: true,
            ..Default::default()
        };
        let (a, keys) = accounts(&store);
        *keys.fail_delete.lock().unwrap() = Some(CredentialError::Backend {
            backend: "test backend",
            detail: "delete refused".into(),
        });
        let e = a.add(b2(), &creds("k", "the-secret")).unwrap_err();
        assert!(matches!(e, AccountError::AddRollbackFailed { .. }), "{e:?}");
        let text = e.to_string();
        assert!(text.contains("could not be removed again"), "{text}");
        assert!(!format!("{text} {e:?}").contains("the-secret"));
        assert!(a.list().is_empty());
    }

    #[test]
    fn a_failed_credential_store_saves_no_account() {
        let store = MemoryAccountStore::default();
        let keys = Arc::new(MemoryCredentialStore::unavailable());
        let a = accounts_with(&store, &keys);
        let e = a.add(b2(), &creds("k", "s")).unwrap_err();
        assert!(
            matches!(
                e,
                AccountError::Credentials(CredentialError::Unavailable { .. })
            ),
            "{e:?}"
        );
        assert!(a.list().is_empty());
        assert!(store.rows.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_update_keeps_the_old_credentials_and_connection() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k1", "s1")).unwrap();
        let before = a.connect("b2-photos").unwrap();
        *keys.fail_store.lock().unwrap() = Some(CredentialError::AccessDenied {
            backend: "test backend",
            detail: "locked".into(),
        });
        let e = a
            .set_credentials("b2-photos", &creds("k2", "s2"))
            .unwrap_err();
        assert!(matches!(
            e,
            AccountError::Credentials(CredentialError::AccessDenied { .. })
        ));
        assert_eq!(keys.load("b2-photos").unwrap(), creds("k1", "s1"));
        assert!(Arc::ptr_eq(&before, &a.connect("b2-photos").unwrap()));
    }

    #[test]
    fn a_failed_credential_delete_keeps_the_account() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k", "s")).unwrap();
        *keys.fail_delete.lock().unwrap() = Some(CredentialError::Unavailable {
            backend: "test backend",
            detail: "not running".into(),
        });
        let e = a.remove("b2-photos").unwrap_err();
        assert!(matches!(
            e,
            AccountError::Credentials(CredentialError::Unavailable { .. })
        ));
        assert_eq!(a.list().len(), 1, "never a silent success");
        assert_eq!(store.rows.lock().unwrap().len(), 1);
        // Once the backend works again, removing finishes.
        *keys.fail_delete.lock().unwrap() = None;
        a.remove("b2-photos").unwrap();
        assert!(a.list().is_empty() && keys.ids().is_empty());
    }

    #[test]
    fn a_failed_account_remove_after_the_credentials_is_reported() {
        let mut store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k", "s")).unwrap();
        store.fail_removes = true;
        let a = accounts_with(&store, &keys);
        let e = a.remove("b2-photos").unwrap_err();
        assert!(
            matches!(e, AccountError::RemovedOnlyCredentials { .. }),
            "{e:?}"
        );
        assert!(keys.ids().is_empty(), "the credentials are gone");
        assert_eq!(a.list().len(), 1, "the account is still listed");
        assert!(!a.has_credentials("b2-photos").unwrap());
    }

    #[test]
    fn an_account_without_credentials_can_still_be_removed() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k", "s")).unwrap();
        keys.entries.lock().unwrap().clear();
        a.remove("b2-photos").unwrap();
        assert!(a.list().is_empty());
    }

    #[test]
    fn connecting_needs_the_accounts_own_credentials() {
        let store = MemoryAccountStore::default();
        let (a, keys) = accounts(&store);
        a.add(b2(), &creds("k", "s")).unwrap();
        keys.entries.lock().unwrap().clear();
        let e = a.connect("b2-photos").unwrap_err();
        assert!(
            matches!(
                e,
                AccountError::Credentials(CredentialError::NotFound { .. })
            ),
            "{e:?}"
        );
        assert!(e.to_string().contains("b2-photos"), "{e}");
        assert!(matches!(a.connect("other"), Err(AccountError::Unknown(_))));
        assert!(!a.has_credentials("b2-photos").unwrap());
    }

    #[test]
    fn an_unavailable_backend_is_reported_on_every_path() {
        let store = MemoryAccountStore::default();
        store.rows.lock().unwrap().push(b2());
        let keys = Arc::new(MemoryCredentialStore::unavailable());
        let a = accounts_with(&store, &keys);
        // Loading never touches the credential store.
        assert_eq!(a.list().len(), 1);
        for e in [
            a.connect("b2-photos").map(|_| ()).unwrap_err(),
            a.has_credentials("b2-photos").map(|_| ()).unwrap_err(),
            a.set_credentials("b2-photos", &creds("k", "s"))
                .unwrap_err(),
            a.remove("b2-photos").unwrap_err(),
        ] {
            assert!(
                matches!(
                    e,
                    AccountError::Credentials(CredentialError::Unavailable { .. })
                ),
                "{e:?}"
            );
        }
        assert_eq!(a.list().len(), 1);
    }

    #[test]
    fn errors_never_contain_a_secret() {
        let store = MemoryAccountStore {
            fail_saves: true,
            ..Default::default()
        };
        let (a, _keys) = accounts(&store);
        let secret = "K003-very-secret-value";
        let mut shown = String::new();
        for e in [
            a.add(b2(), &creds("k", secret)).unwrap_err(),
            a.add(b2(), &creds("k", &format!("{secret} "))).unwrap_err(),
            a.set_credentials("b2-photos", &creds("k", secret))
                .unwrap_err(),
        ] {
            shown.push_str(&format!("{e} {e:?} "));
        }
        assert!(!shown.contains("very-secret"), "{shown}");
        let config = format!("{:?}", b2());
        assert!(!config.contains("very-secret"));
    }

    #[test]
    fn a_stored_account_that_no_longer_validates_is_kept_but_cannot_connect() {
        let store = MemoryAccountStore::default();
        let mut old = b2();
        old.provider = "retired-provider".into();
        store.rows.lock().unwrap().push(old);
        let (a, _keys) = accounts(&store);
        assert_eq!(a.list().len(), 1);
        assert!(matches!(
            a.connect("b2-photos"),
            Err(AccountError::Config(_))
        ));
    }
}

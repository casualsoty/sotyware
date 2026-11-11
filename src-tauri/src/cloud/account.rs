//! Cloud storage accounts: their configuration, validation and the set of
//! accounts sotyware knows.
//!
//! An account is one set of credentials at one provider (two Backblaze
//! accounts are two accounts). It names the buckets sotyware may use and holds
//! no secret: credentials are looked up by account id
//! ([`super::credentials`]). Configurations are kept behind [`AccountStore`]
//! (SQLite in `storage.rs`), the same split as feature preferences.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use super::credentials::{CredentialError, CredentialSource};
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

fn is_kebab_case(id: &str) -> bool {
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

/// Why an account operation failed.
#[derive(Debug)]
pub enum AccountError {
    Config(ConfigError),
    Store(StoreError),
    Credentials(CredentialError),
    Duplicate(String),
    Unknown(String),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "invalid account: {e}"),
            Self::Store(e) => write!(f, "cannot save the account: {e}"),
            Self::Credentials(e) => e.fmt(f),
            Self::Duplicate(id) => write!(f, "an account '{id}' already exists"),
            Self::Unknown(id) => write!(f, "no account '{id}'"),
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

/// Every configured account, and a connection to each one in use.
pub struct Accounts {
    inner: Mutex<Inner>,
    options: ClientOptions,
}

impl Accounts {
    /// Loads the stored accounts. A stored account that no longer validates
    /// (for example a provider this build does not know) is kept in the list
    /// and in the store, and fails only when used.
    pub fn load(
        mut store: Box<dyn AccountStore>,
        options: ClientOptions,
    ) -> Result<Self, StoreError> {
        let configs = store.load()?;
        Ok(Self {
            inner: Mutex::new(Inner {
                store,
                configs,
                connected: HashMap::new(),
            }),
            options,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<AccountConfig> {
        self.lock().configs.clone()
    }

    /// Validates and saves a new account. Nothing changes if either fails.
    pub fn add(&self, config: AccountConfig) -> Result<(), AccountError> {
        config.resolve().map_err(AccountError::Config)?;
        let mut inner = self.lock();
        if inner.configs.iter().any(|c| c.id == config.id) {
            return Err(AccountError::Duplicate(config.id));
        }
        inner.store.save(&config).map_err(AccountError::Store)?;
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

    /// Forgets an account. Its credentials and its data at the provider are
    /// not touched.
    pub fn remove(&self, id: &str) -> Result<(), AccountError> {
        let mut inner = self.lock();
        if !inner.configs.iter().any(|c| c.id == id) {
            return Err(AccountError::Unknown(id.into()));
        }
        inner.store.remove(id).map_err(AccountError::Store)?;
        inner.configs.retain(|c| c.id != id);
        inner.connected.remove(id);
        Ok(())
    }

    /// The account's store, connected with its own credentials. No request is
    /// sent here: a wrong key or an unreachable endpoint shows on first use.
    pub fn connect(
        &self,
        id: &str,
        credentials: &dyn CredentialSource,
    ) -> Result<Arc<S3Store>, AccountError> {
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
        let creds = credentials
            .credentials(id)
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
        self.rows.lock().unwrap().retain(|c| c.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::credentials::{Credentials, Secret, StaticCredentials};
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

    fn accounts(store: &MemoryAccountStore) -> Accounts {
        Accounts::load(Box::new(store.clone()), ClientOptions::default()).unwrap()
    }

    #[test]
    fn accounts_are_added_listed_updated_and_removed_through_the_store() {
        let store = MemoryAccountStore::default();
        let a = accounts(&store);
        a.add(hetzner()).unwrap();
        a.add(b2()).unwrap();
        let ids: Vec<_> = a.list().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["b2-photos", "hetzner-backup"]);
        assert_eq!(*store.rows.lock().unwrap(), a.list());

        let mut changed = b2();
        changed.label = "Family photos".into();
        a.update(changed.clone()).unwrap();
        assert!(store.rows.lock().unwrap().contains(&changed));

        a.remove("hetzner-backup").unwrap();
        assert_eq!(a.list(), [changed]);
        assert_eq!(store.rows.lock().unwrap().len(), 1);
        // A restart sees the same accounts.
        assert_eq!(accounts(&store).list(), a.list());
    }

    #[test]
    fn two_accounts_of_the_same_provider_are_independent() {
        let store = MemoryAccountStore::default();
        let a = accounts(&store);
        let mut second = b2();
        second.id = "b2-work".into();
        second.buckets = vec!["soty-work".into()];
        a.add(b2()).unwrap();
        a.add(second).unwrap();
        let creds = StaticCredentials(vec![
            (
                "b2-photos",
                Credentials {
                    access_key_id: "k1".into(),
                    secret_access_key: Secret::new("s1"),
                },
            ),
            (
                "b2-work",
                Credentials {
                    access_key_id: "k2".into(),
                    secret_access_key: Secret::new("s2"),
                },
            ),
        ]);
        let photos = a.connect("b2-photos", &creds).unwrap();
        let work = a.connect("b2-work", &creds).unwrap();
        assert!(!Arc::ptr_eq(&photos, &work));
        assert_eq!(photos.access_key_id(), "k1");
        assert_eq!(work.access_key_id(), "k2");
        // The same account reuses its one connection (and transfer queue).
        assert!(Arc::ptr_eq(
            &photos,
            &a.connect("b2-photos", &creds).unwrap()
        ));
    }

    #[test]
    fn invalid_duplicate_and_unknown_accounts_are_rejected_without_saving() {
        let store = MemoryAccountStore::default();
        let a = accounts(&store);
        a.add(b2()).unwrap();
        assert!(matches!(a.add(b2()), Err(AccountError::Duplicate(_))));
        let mut bad = hetzner();
        bad.region = "Bad".into();
        assert!(matches!(a.add(bad), Err(AccountError::Config(_))));
        assert!(matches!(a.update(hetzner()), Err(AccountError::Unknown(_))));
        assert!(matches!(a.remove("nope"), Err(AccountError::Unknown(_))));
        assert_eq!(store.rows.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let store = MemoryAccountStore {
            fail_saves: true,
            ..Default::default()
        };
        let a = accounts(&store);
        assert!(matches!(a.add(b2()), Err(AccountError::Store(_))));
        assert!(a.list().is_empty());
    }

    #[test]
    fn connecting_needs_the_accounts_own_credentials() {
        let store = MemoryAccountStore::default();
        let a = accounts(&store);
        a.add(b2()).unwrap();
        let none = StaticCredentials(vec![]);
        assert!(matches!(
            a.connect("b2-photos", &none),
            Err(AccountError::Credentials(_))
        ));
        assert!(matches!(
            a.connect("other", &none),
            Err(AccountError::Unknown(_))
        ));
    }

    #[test]
    fn a_stored_account_that_no_longer_validates_is_kept_but_cannot_connect() {
        let store = MemoryAccountStore::default();
        let mut old = b2();
        old.provider = "retired-provider".into();
        store.rows.lock().unwrap().push(old);
        let a = accounts(&store);
        assert_eq!(a.list().len(), 1);
        let creds = StaticCredentials(vec![]);
        assert!(matches!(
            a.connect("b2-photos", &creds),
            Err(AccountError::Config(_))
        ));
    }
}

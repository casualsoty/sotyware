//! Credentials for cloud storage accounts.
//!
//! Secrets never live in an account configuration, in the accounts database or
//! in log output. An account names no secret at all: its credentials are kept
//! by account id in a [`CredentialStore`]. The app's store is the operating
//! system's credential service ([`super::keyring::OsCredentialStore`]: Windows
//! Credential Manager, Linux Secret Service). There is no other store and no
//! fallback: if the OS service is unavailable, the operation fails.
//!
//! [`EnvCredentials`] exists only in test builds, for the opt-in integration
//! tests against real providers; the app cannot read credentials from the
//! environment.

use std::fmt;

/// A secret value. Its `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, to hand to the request signer or the OS credential
    /// store and nowhere else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// An S3 access key pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    /// Identifies the key; not secret on its own (it appears in every signed
    /// request).
    pub access_key_id: String,
    pub secret_access_key: Secret,
}

/// The longest key id or secret accepted. Real ones are far shorter (B2: 25
/// and 31 characters, Hetzner: 20 and 40); the bound keeps the stored value
/// well inside Windows' 2,560-byte credential limit.
pub const MAX_CREDENTIAL_PART_LEN: usize = 512;

impl Credentials {
    /// Checks that both parts can be stored and sent: not empty, no
    /// whitespace or control characters, not too long. The message never
    /// contains either value.
    pub fn validate(&self) -> Result<(), &'static str> {
        fn check(value: &str, what: &'static str) -> Result<(), &'static str> {
            if value.is_empty() {
                return Err(what);
            }
            if value.len() > MAX_CREDENTIAL_PART_LEN
                || value.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(what);
            }
            Ok(())
        }
        check(
            &self.access_key_id,
            "the access key id is empty, too long or contains whitespace",
        )?;
        check(
            self.secret_access_key.expose(),
            "the secret access key is empty, too long or contains whitespace",
        )
    }

    /// The value kept in the OS credential store: a version line, then the
    /// key id and the secret, one per line. One entry holds both, so replacing
    /// it replaces both at once.
    pub(crate) fn encode(&self) -> String {
        format!(
            "{STORED_FORMAT_V1}\n{}\n{}",
            self.access_key_id,
            self.secret_access_key.expose()
        )
    }

    /// Reads a value written by [`Self::encode`]. Anything else is refused
    /// without repeating it.
    pub(crate) fn decode(stored: &str) -> Option<Self> {
        let mut lines = stored.split('\n');
        let (Some(STORED_FORMAT_V1), Some(key), Some(secret), None) =
            (lines.next(), lines.next(), lines.next(), lines.next())
        else {
            return None;
        };
        let credentials = Self {
            access_key_id: key.to_owned(),
            secret_access_key: Secret::new(secret),
        };
        credentials.validate().ok()?;
        Some(credentials)
    }
}

const STORED_FORMAT_V1: &str = "sotyware-s3-credentials-v1";

/// Why an account's credentials could not be stored, read or removed. No
/// variant carries a secret; `detail` comes from the backend's error code or
/// message, never from a stored value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialError {
    /// Nothing is stored for the account. `hint` says what to do.
    NotFound { account_id: String, hint: String },
    /// The credential service cannot be reached: no Secret Service on the
    /// session bus, no session bus, no logon session on Windows.
    Unavailable {
        backend: &'static str,
        detail: String,
    },
    /// The service refused: its keyring is locked and was not unlocked, or the
    /// user dismissed the unlock prompt, or the caller has no access.
    AccessDenied {
        backend: &'static str,
        detail: String,
    },
    /// Any other failure of the service.
    Backend {
        backend: &'static str,
        detail: String,
    },
    /// The credentials given, or the value stored, cannot be used.
    Invalid { account_id: String, reason: String },
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { account_id, hint } => {
                write!(f, "no credentials for account '{account_id}' ({hint})")
            }
            Self::Unavailable { backend, detail } => write!(
                f,
                "{backend} is not available, so credentials cannot be stored or read \
                 (sotyware never falls back to storing them elsewhere): {detail}"
            ),
            Self::AccessDenied { backend, detail } => {
                write!(f, "{backend} refused access: {detail}")
            }
            Self::Backend { backend, detail } => write!(f, "{backend} failed: {detail}"),
            Self::Invalid { account_id, reason } => {
                write!(
                    f,
                    "unusable credentials for account '{account_id}': {reason}"
                )
            }
        }
    }
}

/// Where account credentials are kept, by account id. Each account has its
/// own entry, so two accounts (also at the same provider) never share or
/// overwrite each other's key. Implemented by the OS credential store; tests
/// use [`MemoryCredentialStore`].
///
/// Calls may block (D-Bus round trips, an unlock prompt on Linux): call them
/// off the UI thread.
pub trait CredentialStore: Send + Sync {
    /// Stores or replaces the account's credentials in one write. A failed
    /// replace leaves the previous credentials in place.
    fn store(&self, account_id: &str, credentials: &Credentials) -> Result<(), CredentialError>;
    /// The account's credentials, or [`CredentialError::NotFound`].
    fn load(&self, account_id: &str) -> Result<Credentials, CredentialError>;
    /// Removes the account's credentials. Nothing stored is not an error.
    fn delete(&self, account_id: &str) -> Result<(), CredentialError>;
    /// Whether credentials are stored for the account, without returning
    /// them.
    fn exists(&self, account_id: &str) -> Result<bool, CredentialError> {
        match self.load(account_id) {
            Ok(_) => Ok(true),
            Err(CredentialError::NotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Credentials from environment variables, **for the opt-in integration tests
/// only** (not compiled into the app):
/// `SOTYWARE_CLOUD_<ID>_ACCESS_KEY_ID` and `SOTYWARE_CLOUD_<ID>_SECRET_ACCESS_KEY`,
/// where `<ID>` is the account id in upper case with `-` replaced by `_`.
/// Read-only: nothing is ever copied from here into a [`CredentialStore`].
#[cfg(test)]
pub struct EnvCredentials<F = fn(&str) -> Option<String>> {
    lookup: F,
}

#[cfg(test)]
impl EnvCredentials {
    /// Reads the process environment.
    pub fn from_process() -> Self {
        Self {
            lookup: |name| std::env::var(name).ok(),
        }
    }
}

#[cfg(test)]
impl<F: Fn(&str) -> Option<String>> EnvCredentials<F> {
    /// Reads variables through `lookup` (tests pass a map).
    pub fn with_lookup(lookup: F) -> Self {
        Self { lookup }
    }

    pub fn credentials(&self, account_id: &str) -> Result<Credentials, CredentialError> {
        let (key_var, secret_var) = env_var_names(account_id);
        let read = |name: &str| (self.lookup)(name).filter(|v| !v.is_empty());
        match (read(&key_var), read(&secret_var)) {
            (Some(key), Some(secret)) => {
                if key.trim() != key || secret.trim() != secret {
                    return Err(CredentialError::Invalid {
                        account_id: account_id.to_owned(),
                        reason: "leading or trailing whitespace".into(),
                    });
                }
                Ok(Credentials {
                    access_key_id: key,
                    secret_access_key: Secret::new(secret),
                })
            }
            _ => Err(CredentialError::NotFound {
                account_id: account_id.to_owned(),
                hint: format!("set {key_var} and {secret_var}"),
            }),
        }
    }
}

/// The two variable names for `account_id`.
#[cfg(test)]
pub fn env_var_names(account_id: &str) -> (String, String) {
    let id = account_id.to_ascii_uppercase().replace('-', "_");
    (
        format!("SOTYWARE_CLOUD_{id}_ACCESS_KEY_ID"),
        format!("SOTYWARE_CLOUD_{id}_SECRET_ACCESS_KEY"),
    )
}

/// A credential store in memory, for tests. Each operation can be made to
/// fail with a chosen error, to simulate an unavailable or refusing backend.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryCredentialStore {
    pub entries: std::sync::Mutex<std::collections::HashMap<String, Credentials>>,
    pub fail_store: std::sync::Mutex<Option<CredentialError>>,
    pub fail_load: std::sync::Mutex<Option<CredentialError>>,
    pub fail_delete: std::sync::Mutex<Option<CredentialError>>,
}

#[cfg(test)]
impl MemoryCredentialStore {
    pub fn with(entries: &[(&str, &str, &str)]) -> Self {
        let store = Self::default();
        for (id, key, secret) in entries {
            store.entries.lock().unwrap().insert(
                (*id).to_owned(),
                Credentials {
                    access_key_id: (*key).to_owned(),
                    secret_access_key: Secret::new(*secret),
                },
            );
        }
        store
    }

    /// Every operation fails as if the OS service were not running.
    pub fn unavailable() -> Self {
        let store = Self::default();
        let e = || {
            Some(CredentialError::Unavailable {
                backend: "test backend",
                detail: "not running".into(),
            })
        };
        *store.fail_store.lock().unwrap() = e();
        *store.fail_load.lock().unwrap() = e();
        *store.fail_delete.lock().unwrap() = e();
        store
    }

    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.entries.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
impl CredentialStore for MemoryCredentialStore {
    fn store(&self, account_id: &str, credentials: &Credentials) -> Result<(), CredentialError> {
        if let Some(e) = self.fail_store.lock().unwrap().clone() {
            return Err(e);
        }
        self.entries
            .lock()
            .unwrap()
            .insert(account_id.to_owned(), credentials.clone());
        Ok(())
    }

    fn load(&self, account_id: &str) -> Result<Credentials, CredentialError> {
        if let Some(e) = self.fail_load.lock().unwrap().clone() {
            return Err(e);
        }
        self.entries
            .lock()
            .unwrap()
            .get(account_id)
            .cloned()
            .ok_or_else(|| CredentialError::NotFound {
                account_id: account_id.to_owned(),
                hint: "not in the test store".into(),
            })
    }

    fn delete(&self, account_id: &str) -> Result<(), CredentialError> {
        if let Some(e) = self.fail_delete.lock().unwrap().clone() {
            return Err(e);
        }
        self.entries.lock().unwrap().remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvCredentials<impl Fn(&str) -> Option<String>> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EnvCredentials::with_lookup(move |name| map.get(name).cloned())
    }

    fn creds(key: &str, secret: &str) -> Credentials {
        Credentials {
            access_key_id: key.into(),
            secret_access_key: Secret::new(secret),
        }
    }

    #[test]
    fn variable_names_are_derived_from_the_account_id() {
        assert_eq!(
            env_var_names("b2-photos"),
            (
                "SOTYWARE_CLOUD_B2_PHOTOS_ACCESS_KEY_ID".into(),
                "SOTYWARE_CLOUD_B2_PHOTOS_SECRET_ACCESS_KEY".into()
            )
        );
    }

    #[test]
    fn each_account_reads_only_its_own_variables() {
        let source = env(&[
            ("SOTYWARE_CLOUD_B2_ACCESS_KEY_ID", "key-b2"),
            ("SOTYWARE_CLOUD_B2_SECRET_ACCESS_KEY", "secret-b2"),
            ("SOTYWARE_CLOUD_HETZNER_ACCESS_KEY_ID", "key-h"),
            ("SOTYWARE_CLOUD_HETZNER_SECRET_ACCESS_KEY", "secret-h"),
        ]);
        let b2 = source.credentials("b2").unwrap();
        let h = source.credentials("hetzner").unwrap();
        assert_eq!(
            (b2.access_key_id.as_str(), b2.secret_access_key.expose()),
            ("key-b2", "secret-b2")
        );
        assert_eq!(
            (h.access_key_id.as_str(), h.secret_access_key.expose()),
            ("key-h", "secret-h")
        );
    }

    #[test]
    fn missing_or_empty_variables_are_reported_with_their_names() {
        let source = env(&[
            ("SOTYWARE_CLOUD_B2_ACCESS_KEY_ID", "key"),
            ("SOTYWARE_CLOUD_B2_SECRET_ACCESS_KEY", ""),
        ]);
        let e = source.credentials("b2").unwrap_err();
        assert!(matches!(e, CredentialError::NotFound { .. }));
        assert!(
            e.to_string()
                .contains("SOTYWARE_CLOUD_B2_SECRET_ACCESS_KEY"),
            "{e}"
        );
        assert!(source.credentials("other").is_err());
    }

    #[test]
    fn whitespace_around_a_value_is_refused() {
        let source = env(&[
            ("SOTYWARE_CLOUD_B2_ACCESS_KEY_ID", "key"),
            ("SOTYWARE_CLOUD_B2_SECRET_ACCESS_KEY", "secret\n"),
        ]);
        let e = source.credentials("b2").unwrap_err();
        assert!(matches!(e, CredentialError::Invalid { .. }));
        assert!(!e.to_string().contains("secret\n"));
    }

    #[test]
    fn secrets_never_appear_in_debug_or_display_output() {
        let c = creds("AKIDEXAMPLE", "wJalrXUtnFEMI-very-secret");
        let shown = format!("{c:?} {} {:?}", c.secret_access_key, c.secret_access_key);
        assert!(!shown.contains("very-secret"), "{shown}");
        assert!(shown.contains("AKIDEXAMPLE"));
    }

    #[test]
    fn the_stored_value_round_trips_and_holds_both_parts() {
        let c = creds("0031234567890ab0000000001", "K003abc/def+ghi=");
        let stored = c.encode();
        assert_eq!(
            stored,
            "sotyware-s3-credentials-v1\n0031234567890ab0000000001\nK003abc/def+ghi="
        );
        assert_eq!(Credentials::decode(&stored), Some(c));
    }

    #[test]
    fn a_stored_value_in_another_format_is_refused() {
        for stored in [
            "",
            "K003-the-secret",
            "sotyware-s3-credentials-v1\nkey",
            "sotyware-s3-credentials-v1\nkey\n",
            "sotyware-s3-credentials-v1\nkey\nsecret\nextra",
            "sotyware-s3-credentials-v2\nkey\nsecret",
            "sotyware-s3-credentials-v1\nkey\nsec ret",
            "sotyware-s3-credentials-v1\r\nkey\r\nsecret",
        ] {
            assert_eq!(Credentials::decode(stored), None, "{stored:?}");
        }
    }

    #[test]
    fn credentials_that_cannot_be_stored_are_refused_without_naming_them() {
        let long = "x".repeat(MAX_CREDENTIAL_PART_LEN + 1);
        for (key, secret) in [
            ("", "secret-value"),
            ("key", ""),
            ("key", "secret value"),
            ("key", "secret-value\n"),
            ("ke\ty", "secret-value"),
            ("key", long.as_str()),
        ] {
            let e = creds(key, secret).validate().unwrap_err();
            assert!(!e.contains("secret-value"), "{e}");
        }
        assert!(
            creds("key", &"x".repeat(MAX_CREDENTIAL_PART_LEN))
                .validate()
                .is_ok()
        );
        // The longest accepted value fits Windows' 2,560-byte blob, even as
        // UTF-16 (Windows stores passwords as UTF-16).
        let max = creds(
            &"k".repeat(MAX_CREDENTIAL_PART_LEN),
            &"s".repeat(MAX_CREDENTIAL_PART_LEN),
        );
        assert!(max.encode().encode_utf16().count() * 2 <= 2560);
    }

    #[test]
    fn errors_say_what_to_do_and_never_fall_back() {
        let e = CredentialError::Unavailable {
            backend: "Secret Service",
            detail: "no secret service provider or dbus session found".into(),
        };
        let text = e.to_string();
        assert!(text.contains("Secret Service is not available"), "{text}");
        assert!(text.contains("never falls back"), "{text}");
    }

    #[test]
    fn the_memory_store_keeps_accounts_apart() {
        let store = MemoryCredentialStore::default();
        assert!(!store.exists("b2-photos").unwrap());
        store.store("b2-photos", &creds("k1", "s1")).unwrap();
        store.store("b2-work", &creds("k2", "s2")).unwrap();
        assert!(store.exists("b2-photos").unwrap());
        assert_eq!(store.load("b2-work").unwrap(), creds("k2", "s2"));
        store.delete("b2-photos").unwrap();
        store.delete("b2-photos").unwrap();
        assert!(matches!(
            store.load("b2-photos"),
            Err(CredentialError::NotFound { .. })
        ));
        assert_eq!(store.ids(), ["b2-work"]);
    }
}

//! Credentials for cloud storage accounts.
//!
//! Secrets never live in an account configuration, in the accounts database or
//! in log output. An account names no secret at all: its credentials are found
//! by account id through a [`CredentialSource`]. The proof of concept has one
//! source, [`EnvCredentials`]; the OS credential store (Windows Credential
//! Manager, Secret Service) is the planned second one (ADR-016).

use std::fmt;

/// A secret value. Its `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, to hand to the request signer and nowhere else.
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

/// Why an account's credentials are not available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialError {
    /// Nothing is stored for the account. `hint` says where it was looked for.
    Missing { account_id: String, hint: String },
    /// Something is stored but cannot be used.
    Invalid { account_id: String, reason: String },
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { account_id, hint } => {
                write!(f, "no credentials for account '{account_id}' ({hint})")
            }
            Self::Invalid { account_id, reason } => {
                write!(
                    f,
                    "unusable credentials for account '{account_id}': {reason}"
                )
            }
        }
    }
}

/// Where credentials come from. Each account has its own pair, looked up by
/// account id, so two accounts never share a key by accident.
pub trait CredentialSource: Send + Sync {
    fn credentials(&self, account_id: &str) -> Result<Credentials, CredentialError>;
}

/// Credentials from environment variables, for development and the opt-in
/// integration tests:
/// `SOTYWARE_CLOUD_<ID>_ACCESS_KEY_ID` and `SOTYWARE_CLOUD_<ID>_SECRET_ACCESS_KEY`,
/// where `<ID>` is the account id in upper case with `-` replaced by `_`.
pub struct EnvCredentials<F = fn(&str) -> Option<String>> {
    lookup: F,
}

impl EnvCredentials {
    /// Reads the process environment.
    pub fn from_process() -> Self {
        Self {
            lookup: |name| std::env::var(name).ok(),
        }
    }
}

impl<F: Fn(&str) -> Option<String>> EnvCredentials<F> {
    /// Reads variables through `lookup` (tests pass a map).
    pub fn with_lookup(lookup: F) -> Self {
        Self { lookup }
    }
}

/// The two variable names for `account_id`.
pub fn env_var_names(account_id: &str) -> (String, String) {
    let id = account_id.to_ascii_uppercase().replace('-', "_");
    (
        format!("SOTYWARE_CLOUD_{id}_ACCESS_KEY_ID"),
        format!("SOTYWARE_CLOUD_{id}_SECRET_ACCESS_KEY"),
    )
}

impl<F: Fn(&str) -> Option<String> + Send + Sync> CredentialSource for EnvCredentials<F> {
    fn credentials(&self, account_id: &str) -> Result<Credentials, CredentialError> {
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
            _ => Err(CredentialError::Missing {
                account_id: account_id.to_owned(),
                hint: format!("set {key_var} and {secret_var}"),
            }),
        }
    }
}

/// Fixed credentials per account, for tests.
#[cfg(test)]
pub struct StaticCredentials(pub Vec<(&'static str, Credentials)>);

#[cfg(test)]
impl CredentialSource for StaticCredentials {
    fn credentials(&self, account_id: &str) -> Result<Credentials, CredentialError> {
        self.0
            .iter()
            .find(|(id, _)| *id == account_id)
            .map(|(_, c)| c.clone())
            .ok_or_else(|| CredentialError::Missing {
                account_id: account_id.to_owned(),
                hint: "not in the test table".into(),
            })
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
        assert!(matches!(e, CredentialError::Missing { .. }));
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
        let c = Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: Secret::new("wJalrXUtnFEMI-very-secret"),
        };
        let shown = format!("{c:?} {} {:?}", c.secret_access_key, c.secret_access_key);
        assert!(!shown.contains("very-secret"), "{shown}");
        assert!(shown.contains("AKIDEXAMPLE"));
    }
}

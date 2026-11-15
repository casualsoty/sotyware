//! The operating system's credential service as a [`CredentialStore`]:
//! Windows Credential Manager on Windows, the Secret Service (GNOME Keyring,
//! KWallet, KeePassXC, …) over D-Bus on Linux, the user's login keychain on
//! macOS. Built on `keyring-core` with one platform store crate per OS
//! (ADR-017, ADR-037).
//!
//! Naming (the same on both platforms, derived from the account id only,
//! never from the provider):
//! - service [`SERVICE`] (`com.sotyware.desktop`, the bundle identifier),
//! - user `cloud-account/<account id>`,
//! - Windows target name `com.sotyware.desktop/cloud-account/<account id>`,
//!   (a test-profile build, ADR-027, uses [`TEST_SERVICE`] and a per-profile
//!   prefix instead: [`CredentialNamespace`]),
//!   a generic credential persisted for the local machine (it does not roam),
//! - Secret Service attributes `service` and `username` as above, label
//!   `sotyware cloud account <account id>`, in the default collection,
//! - macOS: a generic password item in the login keychain with service and
//!   account as above (the legacy keychain: no entitlement or provisioning
//!   profile is needed, signed or not).
//!
//! Each account is one entry holding the key id and the secret together
//! ([`Credentials::encode`]), so an update is one replace and never a delete
//! followed by a write.
//!
//! There is no fallback: when the service is missing every call fails with
//! [`CredentialError::Unavailable`]; nothing is written to a file.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use keyring_core::{CredentialStore as KeyringStore, Entry, Error as KeyringError};

use super::account::is_kebab_case;
use super::credentials::{CredentialError, CredentialStore, Credentials};

/// The service name of every sotyware credential (the bundle identifier).
pub const SERVICE: &str = "com.sotyware.desktop";

/// The service name of credentials written by a test-profile build (ADR-027).
/// It differs from [`SERVICE`] before the first `/`, so no test target name
/// can equal or start with a normal one.
pub const TEST_SERVICE: &str = "com.sotyware.desktop.test";

/// Where in the OS credential store sotyware keeps its entries: a service
/// name and a prefix for the user name. The app's own namespace exists only
/// in builds without the `test-profile` feature, so a test-profile build
/// cannot even name it (ADR-027).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialNamespace {
    service: &'static str,
    user_prefix: String,
}

impl CredentialNamespace {
    /// The app's own credentials. Not compiled into a test-profile build.
    #[cfg(not(feature = "test-profile"))]
    pub fn normal() -> Self {
        Self {
            service: SERVICE,
            user_prefix: String::new(),
        }
    }

    /// The credentials of one test profile: service [`TEST_SERVICE`], user
    /// names prefixed with the profile's nonce. `None` unless `nonce` is 32
    /// lowercase hex digits, so a prefix can never contain `/` or `..`.
    #[cfg(any(test, feature = "test-profile"))]
    pub fn isolated(nonce: &str) -> Option<Self> {
        let valid = nonce.len() == 32
            && nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| Self {
            service: TEST_SERVICE,
            user_prefix: format!("{nonce}/"),
        })
    }

    /// The service name of every entry in this namespace.
    pub fn service(&self) -> &'static str {
        self.service
    }

    /// The user name of the entry for `account_id`.
    pub fn entry_user(&self, account_id: &str) -> String {
        format!("{}cloud-account/{account_id}", self.user_prefix)
    }

    /// The Windows target name of the entry for `account_id`.
    pub fn windows_target(&self, account_id: &str) -> String {
        format!("{}/{}", self.service, self.entry_user(account_id))
    }

    /// The start every Windows target name in this namespace has.
    pub fn windows_target_prefix(&self) -> String {
        format!("{}/{}", self.service, self.user_prefix)
    }
}

/// The Secret Service label of the entry for `account_id`, shown in Seahorse
/// or KWallet.
pub fn secret_service_label(account_id: &str) -> String {
    format!("sotyware cloud account {account_id}")
}

/// What the user sees as the backend's name in messages.
#[cfg(windows)]
pub const BACKEND: &str = "Windows Credential Manager";
#[cfg(target_os = "linux")]
pub const BACKEND: &str = "the Secret Service (desktop keyring)";
#[cfg(target_os = "macos")]
pub const BACKEND: &str = "the macOS Keychain";
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub const BACKEND: &str = "the OS credential store";

/// The OS credential store. Cheap to create: the platform store is opened on
/// first use and kept; while it cannot be opened (Linux without a Secret
/// Service) each call tries again and fails with `Unavailable`, so a keyring
/// that starts later is picked up without a restart.
pub struct OsCredentialStore {
    namespace: CredentialNamespace,
    /// Serialises every call: `keyring-core` documents that the platform
    /// stores may not handle one credential from several threads reliably.
    platform: Mutex<Option<Arc<KeyringStore>>>,
}

impl OsCredentialStore {
    /// A store for the entries of `namespace` only.
    pub fn new(namespace: CredentialNamespace) -> Self {
        Self {
            namespace,
            platform: Mutex::new(None),
        }
    }

    /// The namespace every entry of this store is in.
    pub fn namespace(&self) -> &CredentialNamespace {
        &self.namespace
    }

    fn lock(&self) -> MutexGuard<'_, Option<Arc<KeyringStore>>> {
        self.platform.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Opens the platform store if needed and builds the account's entry.
    fn with_entry<T>(
        &self,
        account_id: &str,
        op: impl FnOnce(&Entry) -> Result<T, KeyringError>,
    ) -> Result<T, CredentialError> {
        if !is_kebab_case(account_id) {
            return Err(CredentialError::Invalid {
                account_id: account_id.to_owned(),
                reason: "the account id must be lowercase kebab-case".into(),
            });
        }
        let mut platform = self.lock();
        let store = match platform.as_ref() {
            Some(store) => Arc::clone(store),
            None => {
                let store = open_platform_store().map_err(|e| classify(e, account_id))?;
                *platform = Some(Arc::clone(&store));
                store
            }
        };
        let entry = build_entry(&store, &self.namespace, account_id)
            .map_err(|e| classify(e, account_id))?;
        op(&entry).map_err(|e| classify(e, account_id))
    }
}

impl CredentialStore for OsCredentialStore {
    fn store(&self, account_id: &str, credentials: &Credentials) -> Result<(), CredentialError> {
        credentials
            .validate()
            .map_err(|reason| CredentialError::Invalid {
                account_id: account_id.to_owned(),
                reason: reason.into(),
            })?;
        let value = credentials.encode();
        self.with_entry(account_id, |entry| entry.set_password(&value))
    }

    fn load(&self, account_id: &str) -> Result<Credentials, CredentialError> {
        let value = self.with_entry(account_id, Entry::get_password)?;
        Credentials::decode(&value).ok_or_else(|| CredentialError::Invalid {
            account_id: account_id.to_owned(),
            reason: format!(
                "the entry in {BACKEND} is not in sotyware's format; enter the key again"
            ),
        })
    }

    fn delete(&self, account_id: &str) -> Result<(), CredentialError> {
        match self.with_entry(account_id, Entry::delete_credential) {
            Err(CredentialError::NotFound { .. }) => Ok(()),
            other => other,
        }
    }
}

/// Named secrets of the sotyware account (ADR-032): the refresh token, the
/// device's private key and the account key. Kept in the same OS
/// credential store and namespace as the storage credentials, but under
/// their own entry prefix (`sotyware-account/<name>`), so no storage
/// account id can ever name one of them.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<zeroize::Zeroizing<String>>, CredentialError>;
    fn set(&self, name: &str, value: &str) -> Result<(), CredentialError>;
    /// Deleting a missing secret is not an error.
    fn delete(&self, name: &str) -> Result<(), CredentialError>;
}

/// The OS credential store, for [`SecretStore`] entries.
pub struct OsSecretStore {
    namespace: CredentialNamespace,
    platform: Mutex<Option<Arc<KeyringStore>>>,
}

impl OsSecretStore {
    pub fn new(namespace: CredentialNamespace) -> Self {
        Self {
            namespace,
            platform: Mutex::new(None),
        }
    }

    fn with_entry<T>(
        &self,
        name: &str,
        op: impl FnOnce(&Entry) -> Result<T, KeyringError>,
    ) -> Result<T, CredentialError> {
        if !is_kebab_case(name) {
            return Err(CredentialError::Invalid {
                account_id: name.to_owned(),
                reason: "a secret name must be lowercase kebab-case".into(),
            });
        }
        let mut platform = self.platform.lock().unwrap_or_else(|e| e.into_inner());
        let store = match platform.as_ref() {
            Some(store) => Arc::clone(store),
            None => {
                let store = open_platform_store().map_err(|e| classify(e, name))?;
                *platform = Some(Arc::clone(&store));
                store
            }
        };
        let user = format!("{}sotyware-account/{name}", self.namespace.user_prefix);
        #[cfg(windows)]
        let entry = {
            let target = format!("{}/{user}", self.namespace.service());
            let modifiers = HashMap::from([("target", target.as_str()), ("persistence", "Local")]);
            store.build(self.namespace.service(), &user, Some(&modifiers))
        };
        #[cfg(target_os = "linux")]
        let entry = {
            let label = format!("sotyware account {name}");
            let modifiers = HashMap::from([("label", label.as_str())]);
            store.build(self.namespace.service(), &user, Some(&modifiers))
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let entry = store.build(
            self.namespace.service(),
            &user,
            None::<&HashMap<&str, &str>>,
        );
        let entry = entry.map_err(|e| classify(e, name))?;
        op(&entry).map_err(|e| classify(e, name))
    }
}

impl SecretStore for OsSecretStore {
    fn get(&self, name: &str) -> Result<Option<zeroize::Zeroizing<String>>, CredentialError> {
        match self.with_entry(name, Entry::get_password) {
            Ok(v) => Ok(Some(zeroize::Zeroizing::new(v))),
            Err(CredentialError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), CredentialError> {
        self.with_entry(name, |e| e.set_password(value))
    }

    fn delete(&self, name: &str) -> Result<(), CredentialError> {
        match self.with_entry(name, Entry::delete_credential) {
            Err(CredentialError::NotFound { .. }) => Ok(()),
            other => other,
        }
    }
}

/// A [`SecretStore`] in memory (tests).
#[cfg(test)]
#[derive(Default)]
pub struct MemorySecretStore {
    pub entries: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<zeroize::Zeroizing<String>>, CredentialError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .map(zeroize::Zeroizing::new))
    }
    fn set(&self, name: &str, value: &str) -> Result<(), CredentialError> {
        self.entries
            .lock()
            .unwrap()
            .insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), CredentialError> {
        self.entries.lock().unwrap().remove(name);
        Ok(())
    }
}

#[cfg(windows)]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "linux")]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

/// The user's login keychain (`MacKeychainDomain::User`).
#[cfg(target_os = "macos")]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Err(KeyringError::NotSupportedByStore(
        "sotyware supports the credential stores of Windows, macOS and Linux only".into(),
    ))
}

#[cfg(windows)]
fn build_entry(
    store: &Arc<KeyringStore>,
    ns: &CredentialNamespace,
    account_id: &str,
) -> Result<Entry, KeyringError> {
    let target = ns.windows_target(account_id);
    // `Local` = CRED_PERSIST_LOCAL_MACHINE: survives logoff and reboot, stays
    // on this machine. The crate's default, `Enterprise`, roams with a domain
    // roaming profile.
    let modifiers = HashMap::from([("target", target.as_str()), ("persistence", "Local")]);
    store.build(ns.service(), &ns.entry_user(account_id), Some(&modifiers))
}

#[cfg(target_os = "linux")]
fn build_entry(
    store: &Arc<KeyringStore>,
    ns: &CredentialNamespace,
    account_id: &str,
) -> Result<Entry, KeyringError> {
    let label = secret_service_label(account_id);
    let modifiers = HashMap::from([("label", label.as_str())]);
    store.build(ns.service(), &ns.entry_user(account_id), Some(&modifiers))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn build_entry(
    store: &Arc<KeyringStore>,
    ns: &CredentialNamespace,
    account_id: &str,
) -> Result<Entry, KeyringError> {
    store.build(
        ns.service(),
        &ns.entry_user(account_id),
        None::<&HashMap<&str, &str>>,
    )
}

/// Sorts a `keyring-core` error into what the caller can act on.
///
/// Never formats `BadEncoding`, `BadDataFormat` or `Ambiguous`: the first two
/// carry the stored bytes (the secret), the last carries entries. Platform
/// errors are shown by their own `Display` (a Windows error code, a D-Bus
/// error), which holds no stored value.
fn classify(error: KeyringError, account_id: &str) -> CredentialError {
    let backend = BACKEND;
    match error {
        KeyringError::NoEntry => CredentialError::NotFound {
            account_id: account_id.to_owned(),
            hint: format!("nothing stored in {backend}; enter the account's key again"),
        },
        KeyringError::NoStorageAccess(e) => {
            let detail = e.to_string();
            match platform_kind(e.as_ref(), true) {
                PlatformKind::Unavailable => CredentialError::Unavailable { backend, detail },
                PlatformKind::AccessDenied | PlatformKind::Other => {
                    CredentialError::AccessDenied { backend, detail }
                }
            }
        }
        KeyringError::PlatformFailure(e) => {
            let detail = e.to_string();
            match platform_kind(e.as_ref(), false) {
                PlatformKind::Unavailable => CredentialError::Unavailable { backend, detail },
                PlatformKind::AccessDenied => CredentialError::AccessDenied { backend, detail },
                PlatformKind::Other => CredentialError::Backend { backend, detail },
            }
        }
        KeyringError::BadEncoding(_) | KeyringError::BadDataFormat(..) => {
            CredentialError::Invalid {
                account_id: account_id.to_owned(),
                reason: format!(
                    "the entry in {backend} is not in sotyware's format; enter the key again"
                ),
            }
        }
        KeyringError::Ambiguous(entries) => CredentialError::Backend {
            backend,
            detail: format!(
                "{} entries match account '{account_id}'; remove the extra ones",
                entries.len()
            ),
        },
        KeyringError::NoDefaultStore | KeyringError::NotSupportedByStore(_) => {
            CredentialError::Unavailable {
                backend,
                detail: error.to_string(),
            }
        }
        // TooLong, Invalid, BadStoreFormat and future variants: they name an
        // attribute or a limit, never a stored value.
        other => CredentialError::Backend {
            backend,
            detail: other.to_string(),
        },
    }
}

enum PlatformKind {
    Unavailable,
    // Only the Secret Service and the macOS Keychain report a refusal the
    // code can recognise; see the Windows `platform_kind` below.
    #[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
    AccessDenied,
    Other,
}

type PlatformError = dyn std::error::Error + Send + Sync + 'static;

/// Recognises "no service here" and "refused" by the platform error's type,
/// never by its message. `no_storage_access` says which `keyring-core`
/// variant carried the error: the Secret Service store uses it for a locked
/// keyring and a dismissed prompt.
#[cfg(target_os = "linux")]
fn platform_kind(error: &PlatformError, no_storage_access: bool) -> PlatformKind {
    use secret_service::Error as Ss;
    use std::io::ErrorKind;

    // D-Bus error names are part of the protocol, not messages.
    const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
    const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";
    const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";

    fn from_zbus(error: &zbus::Error) -> Option<PlatformKind> {
        match error {
            zbus::Error::Address(_) | zbus::Error::InterfaceNotFound => {
                Some(PlatformKind::Unavailable)
            }
            // No session bus: the address names a socket that does not
            // exist or refuses (seen under WSL2), as in `tray.rs`.
            zbus::Error::Connection(io, _) | zbus::Error::InputOutput(io)
                if matches!(
                    io.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) =>
            {
                Some(PlatformKind::Unavailable)
            }
            zbus::Error::MethodError(name, _, _) => match name.as_str() {
                SERVICE_UNKNOWN | NAME_HAS_NO_OWNER => Some(PlatformKind::Unavailable),
                ACCESS_DENIED => Some(PlatformKind::AccessDenied),
                _ => None,
            },
            zbus::Error::FDO(fdo) => from_fdo(fdo),
            _ => None,
        }
    }

    fn from_fdo(error: &zbus::fdo::Error) -> Option<PlatformKind> {
        use zbus::fdo::Error as Fdo;
        match error {
            Fdo::ServiceUnknown(_) | Fdo::NameHasNoOwner(_) => Some(PlatformKind::Unavailable),
            Fdo::AccessDenied(_) => Some(PlatformKind::AccessDenied),
            Fdo::ZBus(e) => from_zbus(e),
            _ => None,
        }
    }

    let kind = match error.downcast_ref::<Ss>() {
        Some(Ss::Unavailable) => Some(PlatformKind::Unavailable),
        // A locked collection that was not unlocked, or a dismissed prompt.
        Some(Ss::Locked | Ss::Prompt) => Some(PlatformKind::AccessDenied),
        Some(Ss::Zbus(e)) => from_zbus(e),
        Some(Ss::ZbusFdo(e)) => from_fdo(e),
        _ => None,
    };
    kind.unwrap_or(if no_storage_access {
        PlatformKind::AccessDenied
    } else {
        PlatformKind::Other
    })
}

/// macOS: the store boxes the Security framework's `OSStatus` error; the
/// code (from `SecBase.h`, part of the API, not a message) says whether the
/// user dismissed the access prompt or the keychain is locked without a way
/// to ask (refused: unlock and try again), or there is no usable keychain
/// (unavailable). Other codes are backend failures; their message is the
/// system's description of the code, never a stored value.
#[cfg(target_os = "macos")]
fn platform_kind(error: &PlatformError, no_storage_access: bool) -> PlatformKind {
    match error
        .downcast_ref::<security_framework::base::Error>()
        .map(|e| e.code())
    {
        Some(code) => macos_kind(code, no_storage_access),
        None if no_storage_access => PlatformKind::AccessDenied,
        None => PlatformKind::Other,
    }
}

/// The decision of the macOS `platform_kind`, by `OSStatus` code, as a
/// plain function so every platform's unit tests check it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_kind(code: i32, no_storage_access: bool) -> PlatformKind {
    /// errSecUserCanceled: the "allow access" or unlock prompt was dismissed.
    const USER_CANCELED: i32 = -128;
    /// errSecAuthFailed: a wrong keychain password was typed.
    const AUTH_FAILED: i32 = -25293;
    /// errSecInteractionNotAllowed: locked, and no prompt may be shown.
    const INTERACTION_NOT_ALLOWED: i32 = -25308;
    /// errSecNotAvailable, errSecNoSuchKeychain, errSecInvalidKeychain.
    const NOT_AVAILABLE: i32 = -25291;
    const NO_SUCH_KEYCHAIN: i32 = -25294;
    const INVALID_KEYCHAIN: i32 = -25295;
    match code {
        USER_CANCELED | AUTH_FAILED | INTERACTION_NOT_ALLOWED => PlatformKind::AccessDenied,
        NOT_AVAILABLE | NO_SUCH_KEYCHAIN | INVALID_KEYCHAIN => PlatformKind::Unavailable,
        _ if no_storage_access => PlatformKind::AccessDenied,
        _ => PlatformKind::Other,
    }
}

/// Windows: the store crate's only `NoStorageAccess` is
/// `ERROR_NO_SUCH_LOGON_SESSION` (no credential vault for this logon, as for
/// some service or network logons), so it means unavailable. Every other
/// Win32 code arrives as `PlatformFailure` in a private error type, so no
/// code (not even `ERROR_ACCESS_DENIED`) is told apart: they are backend
/// failures whose message is the Windows error code.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_kind(_error: &PlatformError, no_storage_access: bool) -> PlatformKind {
    if no_storage_access {
        PlatformKind::Unavailable
    } else {
        PlatformKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::credentials::Secret;

    fn boxed(
        e: impl std::error::Error + Send + Sync + 'static,
    ) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(e)
    }

    /// A test namespace for the tests below that never reach a backend.
    fn test_ns() -> CredentialNamespace {
        CredentialNamespace::isolated("0123456789abcdef0123456789abcdef").unwrap()
    }

    #[cfg(not(feature = "test-profile"))]
    #[test]
    fn names_come_from_the_account_id_only() {
        let ns = CredentialNamespace::normal();
        assert_eq!(ns.service(), SERVICE);
        assert_eq!(ns.entry_user("b2-photos"), "cloud-account/b2-photos");
        assert_eq!(
            ns.windows_target("b2-photos"),
            "com.sotyware.desktop/cloud-account/b2-photos"
        );
        assert_eq!(ns.windows_target_prefix(), "com.sotyware.desktop/");
        assert_eq!(
            secret_service_label("b2-work"),
            "sotyware cloud account b2-work"
        );
        // Two accounts at the same provider get two different entries.
        assert_ne!(ns.windows_target("b2-photos"), ns.windows_target("b2-work"));
        let conf = format!("{}/tauri.conf.json", env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(conf).unwrap()).unwrap();
        assert_eq!(
            conf["identifier"], SERVICE,
            "SERVICE is the bundle identifier"
        );
    }

    /// ADR-027: an entry of a test profile can never be an entry of the app
    /// itself (same service and user name, or a Windows target with the
    /// app's prefix), whatever the account id.
    #[test]
    fn a_test_namespace_never_names_a_normal_entry() {
        let ns = test_ns();
        assert_eq!(ns.service(), TEST_SERVICE);
        assert_ne!(ns.service(), SERVICE);
        assert_eq!(
            ns.windows_target("b2-photos"),
            "com.sotyware.desktop.test/0123456789abcdef0123456789abcdef/cloud-account/b2-photos"
        );
        let normal_prefix = format!("{SERVICE}/");
        for id in ["b2-photos", "a", "sotyware-it", "cloud-account"] {
            let target = ns.windows_target(id);
            assert!(!target.starts_with(&normal_prefix), "{target}");
            assert!(target.starts_with(&ns.windows_target_prefix()));
            assert_ne!(ns.entry_user(id), format!("cloud-account/{id}"));
        }
        // Two profiles do not share entries either.
        let other = CredentialNamespace::isolated("ffffffffffffffffffffffffffffffff").unwrap();
        assert_ne!(other.windows_target("a"), ns.windows_target("a"));
        let store = OsCredentialStore::new(ns.clone());
        assert_eq!(store.namespace(), &ns);
    }

    #[test]
    fn a_test_namespace_needs_a_32_digit_lowercase_hex_nonce() {
        for bad in [
            "",
            "0123",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde/",
            "../456789abcdef0123456789abcdef0",
            "0123456789abcdef0123456789abcdef0",
            "0123456789abcdeg0123456789abcdef",
        ] {
            assert!(CredentialNamespace::isolated(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn an_invalid_account_id_is_refused_before_the_backend_is_touched() {
        let store = OsCredentialStore::new(test_ns());
        for id in ["", "B2", "a/b", "../x", "x y"] {
            assert!(
                matches!(store.load(id), Err(CredentialError::Invalid { .. })),
                "{id:?}"
            );
        }
        assert!(store.lock().is_none(), "no platform store was opened");
    }

    #[test]
    fn credentials_that_cannot_be_stored_never_reach_the_backend() {
        let store = OsCredentialStore::new(test_ns());
        let bad = Credentials {
            access_key_id: "key".into(),
            secret_access_key: Secret::new("has a space"),
        };
        let e = store.store("b2-photos", &bad).unwrap_err();
        assert!(matches!(e, CredentialError::Invalid { .. }), "{e:?}");
        assert!(!format!("{e} {e:?}").contains("has a space"));
        assert!(store.lock().is_none());
    }

    #[test]
    fn no_entry_is_not_found_with_an_actionable_hint() {
        let e = classify(KeyringError::NoEntry, "b2-photos");
        assert!(matches!(e, CredentialError::NotFound { .. }), "{e:?}");
        let text = e.to_string();
        assert!(text.contains("b2-photos") && text.contains("enter the account's key again"));
    }

    #[test]
    fn stored_bytes_never_reach_an_error() {
        let secret = b"K003-stored-secret".to_vec();
        for e in [
            KeyringError::BadEncoding(secret.clone()),
            KeyringError::BadDataFormat(secret.clone(), boxed(std::fmt::Error)),
        ] {
            let mapped = classify(e, "b2-photos");
            assert!(matches!(mapped, CredentialError::Invalid { .. }));
            let shown = format!("{mapped} {mapped:?}");
            assert!(!shown.contains("stored-secret"), "{shown}");
            assert!(!shown.contains("75, 48, 48, 51"), "{shown}"); // the bytes as a list
        }
    }

    #[test]
    fn other_errors_are_backend_failures() {
        let e = classify(KeyringError::TooLong("user".into(), 512), "x");
        assert!(matches!(e, CredentialError::Backend { .. }));
        let e = classify(KeyringError::Ambiguous(Vec::new()), "x");
        assert!(matches!(e, CredentialError::Backend { .. }));
        let e = classify(KeyringError::NotSupportedByStore("x".into()), "x");
        assert!(matches!(e, CredentialError::Unavailable { .. }));
    }

    /// The macOS rule by code, checked on every platform; the downcast of
    /// the real error type below runs on macOS only.
    #[test]
    fn keychain_codes_are_sorted_by_osstatus() {
        for code in [-128, -25293, -25308] {
            assert!(
                matches!(macos_kind(code, false), PlatformKind::AccessDenied),
                "{code}"
            );
        }
        for code in [-25291, -25294, -25295] {
            assert!(
                matches!(macos_kind(code, false), PlatformKind::Unavailable),
                "{code}"
            );
            assert!(
                matches!(macos_kind(code, true), PlatformKind::Unavailable),
                "{code}"
            );
        }
        // errSecDuplicateItem, errSecDecode: failures, not refusals…
        assert!(matches!(macos_kind(-25299, false), PlatformKind::Other));
        assert!(matches!(macos_kind(-26275, false), PlatformKind::Other));
        // …unless the store already called it "no storage access" (write
        // permission, read-only keychain).
        assert!(matches!(macos_kind(-61, true), PlatformKind::AccessDenied));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keychain_errors_are_sorted_by_type() {
        use security_framework::base::Error as Sf;
        let dismissed = classify(
            KeyringError::PlatformFailure(Box::new(Sf::from_code(-128))),
            "photos",
        );
        assert!(
            matches!(dismissed, CredentialError::AccessDenied { .. }),
            "{dismissed:?}"
        );
        let gone = classify(
            KeyringError::NoStorageAccess(Box::new(Sf::from_code(-25294))),
            "photos",
        );
        assert!(
            matches!(gone, CredentialError::Unavailable { .. }),
            "{gone:?}"
        );
        let other = classify(
            KeyringError::PlatformFailure(Box::new(Sf::from_code(-25299))),
            "photos",
        );
        assert!(
            matches!(other, CredentialError::Backend { .. }),
            "{other:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn secret_service_errors_are_sorted_by_type() {
        use secret_service::Error as Ss;
        use std::io;

        let unavailable = [
            KeyringError::PlatformFailure(boxed(Ss::Unavailable)),
            KeyringError::PlatformFailure(boxed(Ss::Zbus(zbus::Error::Address("x".into())))),
            KeyringError::PlatformFailure(boxed(Ss::Zbus(zbus::Error::InputOutput(
                io::Error::from(io::ErrorKind::ConnectionRefused).into(),
            )))),
            KeyringError::PlatformFailure(boxed(Ss::Zbus(zbus::Error::Connection(
                io::Error::from(io::ErrorKind::NotFound).into(),
                "unix:path=/run/user/1000/bus".parse().unwrap(),
            )))),
            KeyringError::PlatformFailure(boxed(Ss::ZbusFdo(zbus::fdo::Error::ServiceUnknown(
                "org.freedesktop.secrets".into(),
            )))),
            KeyringError::PlatformFailure(boxed(Ss::Zbus(zbus::Error::FDO(Box::new(
                zbus::fdo::Error::NameHasNoOwner("org.freedesktop.secrets".into()),
            ))))),
        ];
        for e in unavailable {
            let mapped = classify(e, "x");
            assert!(
                matches!(mapped, CredentialError::Unavailable { .. }),
                "{mapped:?}"
            );
        }
        let denied = [
            KeyringError::NoStorageAccess(boxed(Ss::Locked)),
            KeyringError::NoStorageAccess(boxed(Ss::Prompt)),
            KeyringError::PlatformFailure(boxed(Ss::ZbusFdo(zbus::fdo::Error::AccessDenied(
                "no".into(),
            )))),
        ];
        for e in denied {
            let mapped = classify(e, "x");
            assert!(
                matches!(mapped, CredentialError::AccessDenied { .. }),
                "{mapped:?}"
            );
        }
        let other = [
            KeyringError::PlatformFailure(boxed(Ss::Crypto("bad key"))),
            KeyringError::PlatformFailure(boxed(Ss::Zbus(zbus::Error::InputOutput(
                io::Error::from(io::ErrorKind::PermissionDenied).into(),
            )))),
            KeyringError::PlatformFailure(boxed(std::fmt::Error)),
        ];
        for e in other {
            let mapped = classify(e, "x");
            assert!(
                matches!(mapped, CredentialError::Backend { .. }),
                "{mapped:?}"
            );
        }
    }
}

/// Against the real OS credential service. Ignored by default: ordinary
/// `cargo test` and CI never touch the developer's keyring. Run:
///
/// ```text
/// cargo test cloud::keyring::os_backend -- --ignored --nocapture --test-threads=1
/// SOTYWARE_IT_KEYRING_EXPECT=unavailable cargo test cloud::keyring::os_backend -- --ignored --nocapture
/// ```
///
/// Uses dedicated account ids `it-keyring-<run>-a`/`-b` in a test namespace
/// of the run (never the app's own namespace, ADR-027), checks that they start empty, and deletes them at the end, also
/// when an assertion fails.
#[cfg(test)]
mod os_backend {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::cloud::credentials::Secret;

    fn creds(key: &str, secret: &str) -> Credentials {
        Credentials {
            access_key_id: key.into(),
            secret_access_key: Secret::new(secret),
        }
    }

    /// Independent check through Windows' own `cmdkey`: is a generic
    /// credential listed under exactly this target name? (Linux: checked with
    /// `dbus-send` from outside, see docs/development/testing.md.)
    #[cfg(windows)]
    fn listed_by_cmdkey(store: &OsCredentialStore, account_id: &str) -> bool {
        let target = store.namespace().windows_target(account_id);
        let out = std::process::Command::new("cmdkey")
            .arg(format!("/list:{target}"))
            .output()
            .expect("run cmdkey");
        // The header line repeats the name followed by ':'; an entry line
        // ("Target: LegacyGeneric:target=<name>") ends with the name.
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim_end().ends_with(&target))
    }

    #[cfg(not(windows))]
    fn listed_by_cmdkey(_store: &OsCredentialStore, _account_id: &str) -> bool {
        true
    }

    /// Deletes the test entries however the test ends.
    struct Cleanup<'a>(&'a OsCredentialStore, Vec<String>);

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            for id in &self.1 {
                match self.0.delete(id) {
                    Ok(()) => println!("cleanup: {id} deleted (or absent)"),
                    Err(e) => eprintln!("CLEANUP FAILED for {id}: {e}"),
                }
            }
        }
    }

    #[test]
    #[ignore = "opt-in: uses the real OS credential store"]
    fn round_trip() {
        let expect = std::env::var("SOTYWARE_IT_KEYRING_EXPECT").unwrap_or_default();
        let run = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let (a, b) = (format!("it-keyring-{run}-a"), format!("it-keyring-{run}-b"));
        // Never the app's own namespace (ADR-027): a namespace of this run.
        let ns = CredentialNamespace::isolated(&format!("{:032x}", run)).unwrap();
        let store = OsCredentialStore::new(ns.clone());
        println!(
            "backend: {BACKEND}; accounts {a}, {b} under {}",
            ns.windows_target_prefix()
        );

        if expect == "unavailable" {
            for result in [
                store.store(&a, &creds("key-a", "secret-a-1")).err(),
                store.load(&a).err(),
                store.exists(&a).err(),
                store.delete(&a).err(),
            ] {
                let e = result.expect("every call must fail without a service");
                println!("unavailable: {e}");
                assert!(matches!(e, CredentialError::Unavailable { .. }), "{e:?}");
                assert!(!e.to_string().contains("secret-a-1"));
            }
            return;
        }
        assert_eq!(
            expect, "",
            "SOTYWARE_IT_KEYRING_EXPECT must be empty or 'unavailable'"
        );

        let _cleanup = Cleanup(&store, vec![a.clone(), b.clone()]);
        for id in [&a, &b] {
            assert!(!store.exists(id).unwrap(), "{id} must start empty");
            let e = store.load(id).unwrap_err();
            println!("missing: {e}");
            assert!(matches!(e, CredentialError::NotFound { .. }), "{e:?}");
        }

        #[cfg(windows)]
        assert!(!listed_by_cmdkey(&store, &a));
        store.store(&a, &creds("key-a", "secret-a-1")).unwrap();
        assert!(
            listed_by_cmdkey(&store, &a),
            "cmdkey must list {}",
            ns.windows_target(&a)
        );
        assert_eq!(store.load(&a).unwrap(), creds("key-a", "secret-a-1"));
        assert!(store.exists(&a).unwrap());
        assert!(!store.exists(&b).unwrap(), "storing a must not create b");

        // A second account at the same provider: independent entry.
        store.store(&b, &creds("key-b", "secret-b-1")).unwrap();
        assert_eq!(store.load(&b).unwrap(), creds("key-b", "secret-b-1"));
        assert_eq!(store.load(&a).unwrap(), creds("key-a", "secret-a-1"));

        // Update replaces in place; the other account is untouched.
        store.store(&a, &creds("key-a2", "secret-a-2")).unwrap();
        assert_eq!(store.load(&a).unwrap(), creds("key-a2", "secret-a-2"));
        assert_eq!(store.load(&b).unwrap(), creds("key-b", "secret-b-1"));

        store.delete(&a).unwrap();
        assert!(matches!(
            store.load(&a),
            Err(CredentialError::NotFound { .. })
        ));
        assert!(!store.exists(&a).unwrap());
        assert_eq!(store.load(&b).unwrap(), creds("key-b", "secret-b-1"));
        store.delete(&a).unwrap(); // deleting again is not an error

        store.delete(&b).unwrap();
        #[cfg(windows)]
        for id in [&a, &b] {
            assert!(!listed_by_cmdkey(&store, id), "cmdkey still lists {id}");
            println!("cmdkey: {} not listed", ns.windows_target(id));
        }
        for id in [&a, &b] {
            assert!(!store.exists(id).unwrap(), "{id} must be gone");
        }
        // A fresh store (a later process) sees nothing either.
        let fresh = OsCredentialStore::new(ns);
        assert!(!fresh.exists(&a).unwrap() && !fresh.exists(&b).unwrap());
        println!("round trip passed; both entries deleted");
    }
}

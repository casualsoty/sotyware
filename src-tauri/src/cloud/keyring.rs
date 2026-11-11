//! The operating system's credential service as a [`CredentialStore`]:
//! Windows Credential Manager on Windows, the Secret Service (GNOME Keyring,
//! KWallet, KeePassXC, …) over D-Bus on Linux. Built on `keyring-core` with
//! one platform store crate per OS (ADR-017).
//!
//! Naming (the same on both platforms, derived from the account id only,
//! never from the provider):
//! - service [`SERVICE`] (`com.sotyware.desktop`, the bundle identifier),
//! - user `cloud-account/<account id>`,
//! - Windows target name `com.sotyware.desktop/cloud-account/<account id>`,
//!   a generic credential persisted for the local machine (it does not roam),
//! - Secret Service attributes `service` and `username` as above, label
//!   `sotyware cloud account <account id>`, in the default collection.
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

/// The user name of the entry for `account_id`.
pub fn entry_user(account_id: &str) -> String {
    format!("cloud-account/{account_id}")
}

/// The Windows target name of the entry for `account_id`.
pub fn windows_target(account_id: &str) -> String {
    format!("{SERVICE}/{}", entry_user(account_id))
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
#[cfg(not(any(windows, target_os = "linux")))]
pub const BACKEND: &str = "the OS credential store";

/// The OS credential store. Cheap to create: the platform store is opened on
/// first use and kept; while it cannot be opened (Linux without a Secret
/// Service) each call tries again and fails with `Unavailable`, so a keyring
/// that starts later is picked up without a restart.
pub struct OsCredentialStore {
    /// Serialises every call: `keyring-core` documents that the platform
    /// stores may not handle one credential from several threads reliably.
    platform: Mutex<Option<Arc<KeyringStore>>>,
}

impl Default for OsCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OsCredentialStore {
    pub fn new() -> Self {
        Self {
            platform: Mutex::new(None),
        }
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
        let entry = build_entry(&store, account_id).map_err(|e| classify(e, account_id))?;
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

#[cfg(windows)]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "linux")]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_platform_store() -> Result<Arc<KeyringStore>, KeyringError> {
    Err(KeyringError::NotSupportedByStore(
        "sotyware supports the credential stores of Windows and Linux only".into(),
    ))
}

#[cfg(windows)]
fn build_entry(store: &Arc<KeyringStore>, account_id: &str) -> Result<Entry, KeyringError> {
    let target = windows_target(account_id);
    // `Local` = CRED_PERSIST_LOCAL_MACHINE: survives logoff and reboot, stays
    // on this machine. The crate's default, `Enterprise`, roams with a domain
    // roaming profile.
    let modifiers = HashMap::from([("target", target.as_str()), ("persistence", "Local")]);
    store.build(SERVICE, &entry_user(account_id), Some(&modifiers))
}

#[cfg(target_os = "linux")]
fn build_entry(store: &Arc<KeyringStore>, account_id: &str) -> Result<Entry, KeyringError> {
    let label = secret_service_label(account_id);
    let modifiers = HashMap::from([("label", label.as_str())]);
    store.build(SERVICE, &entry_user(account_id), Some(&modifiers))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn build_entry(store: &Arc<KeyringStore>, account_id: &str) -> Result<Entry, KeyringError> {
    store.build(
        SERVICE,
        &entry_user(account_id),
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
    // Only the Secret Service reports a refusal the code can recognise; see
    // the Windows `platform_kind` below.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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

/// Windows: the store crate's only `NoStorageAccess` is
/// `ERROR_NO_SUCH_LOGON_SESSION` (no credential vault for this logon, as for
/// some service or network logons), so it means unavailable. Every other
/// Win32 code arrives as `PlatformFailure` in a private error type, so no
/// code (not even `ERROR_ACCESS_DENIED`) is told apart: they are backend
/// failures whose message is the Windows error code.
#[cfg(not(target_os = "linux"))]
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

    #[test]
    fn names_come_from_the_account_id_only() {
        assert_eq!(entry_user("b2-photos"), "cloud-account/b2-photos");
        assert_eq!(
            windows_target("b2-photos"),
            "com.sotyware.desktop/cloud-account/b2-photos"
        );
        assert_eq!(
            secret_service_label("b2-work"),
            "sotyware cloud account b2-work"
        );
        // Two accounts at the same provider get two different entries.
        assert_ne!(windows_target("b2-photos"), windows_target("b2-work"));
        let conf = format!("{}/tauri.conf.json", env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(conf).unwrap()).unwrap();
        assert_eq!(
            conf["identifier"], SERVICE,
            "SERVICE is the bundle identifier"
        );
    }

    #[test]
    fn an_invalid_account_id_is_refused_before_the_backend_is_touched() {
        let store = OsCredentialStore::new();
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
        let store = OsCredentialStore::new();
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
/// Uses dedicated account ids `it-keyring-<run>-a`/`-b` (never a real
/// account), checks that they start empty, and deletes them at the end, also
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
    fn listed_by_cmdkey(account_id: &str) -> bool {
        let target = windows_target(account_id);
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
    fn listed_by_cmdkey(_account_id: &str) -> bool {
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
        let store = OsCredentialStore::new();
        println!("backend: {BACKEND}; accounts {a}, {b}");

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
        assert!(!listed_by_cmdkey(&a));
        store.store(&a, &creds("key-a", "secret-a-1")).unwrap();
        assert!(
            listed_by_cmdkey(&a),
            "cmdkey must list {}",
            windows_target(&a)
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
            assert!(!listed_by_cmdkey(id), "cmdkey still lists {id}");
            println!("cmdkey: {} not listed", windows_target(id));
        }
        for id in [&a, &b] {
            assert!(!store.exists(id).unwrap(), "{id} must be gone");
        }
        // A fresh store (a later process) sees nothing either.
        let fresh = OsCredentialStore::new();
        assert!(!fresh.exists(&a).unwrap() && !fresh.exists(&b).unwrap());
        println!("round trip passed; both entries deleted");
    }
}

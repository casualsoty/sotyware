//! The optional sotyware account and end-to-end encrypted sync (ADR-032).
//!
//! - [`crypto`]: keys, envelopes, wraps, recovery key format (RustCrypto).
//! - [`vault`]: the vault document and the three-way merge.
//! - [`client`]: HTTPS access to the account service.
//! - This module: [`AccountService`] (managed state) and the commands of
//!   the Account page.
//!
//! What stays where: the refresh token, this device's X25519 private key
//! and the account key are in the OS credential store ([`SecretStore`],
//! entries `sotyware-account/<name>`); the access token only in memory; the
//! account and device ids, the highest vault revision seen and the last
//! synced vault (ciphertext) in `account.sqlite3`. The service receives
//! the vault and key wraps as ciphertext only, and a key proof (one-way).
//! The page never receives a token or a key; it receives the recovery key
//! exactly once, to show it, and sends a typed recovery key when unlocking.
//!
//! Synced: storage accounts (configuration and credentials) and backup jobs.
//! Not synced: local paths, journal entries, running work, recent items,
//! feature switches (per device).
//!
//! Without a configured service (a test-profile build whose run names
//! none) the Account page says so and nothing is sent.

pub mod client;
pub mod crypto;
pub mod vault;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use zeroize::Zeroizing;

use crate::backup::Backups;
use crate::cloud::account::AccountConfig;
use crate::cloud::credentials::{Credentials, Secret};
use crate::cloud::dashboard::{CloudAccounts, CommandError, ErrorKind};
use crate::cloud::keyring::SecretStore;
use crate::features::StoreError;
use crate::journal::now_ms;
use client::{Http, HttpError};
use crypto::{CryptoError, DeviceKey, SecretKey};
use vault::{Doc, Record, TrustedDevice};

/// The production account service, deployed 2026-11-17
/// (`docs/development/server-setup.md`). Release
/// and development builds use it; test-profile builds never do.
pub const PRODUCTION_SERVER: Option<&str> = Some("https://accounts.sotyware.com");

/// Secret names in the OS credential store.
const REFRESH: &str = "refresh-token";
const DEVICE_KEY: &str = "device-key";
const ACCOUNT_KEY: &str = "account-key";

/// The service URL of this build: in development and test-profile builds
/// `SOTYWARE_ACCOUNT_SERVER` if set; otherwise [`PRODUCTION_SERVER`],
/// except in a test-profile build, which has none: a desktop test reaches
/// only a service its run names, never the users' accounts (ADR-027).
pub fn configured_server() -> Option<String> {
    #[cfg(any(debug_assertions, feature = "test-profile"))]
    if let Ok(v) = std::env::var("SOTYWARE_ACCOUNT_SERVER")
        && !v.is_empty()
    {
        return Some(v);
    }
    if cfg!(feature = "test-profile") {
        None
    } else {
        PRODUCTION_SERVER.map(str::to_owned)
    }
}

/// Where the local, non-secret state lives (`storage::SqliteAccountState`).
pub trait StateStore: Send {
    fn load(&mut self) -> Result<Option<String>, StoreError>;
    fn save(&mut self, state: &str) -> Result<(), StoreError>;
    fn clear(&mut self) -> Result<(), StoreError>;
}

/// The local state (no secret).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalState {
    pub account_id: String,
    pub email: String,
    pub device_id: String,
    pub device_name: String,
    /// The highest vault revision this device has seen (rollback check).
    pub highest_revision: i64,
    /// The last vault this device synced, encrypted (the merge base).
    pub base_envelope: Option<String>,
    pub last_sync_ms: Option<i64>,
    pub last_sync_error: Option<String>,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    /// Whether this build has an account service at all.
    pub configured: bool,
    pub service: Option<String>,
    pub signed_in: bool,
    pub email: Option<String>,
    pub device_id: Option<String>,
    /// `enrolled`, `pending`, `none` (sync not set up) or `unknown`.
    pub vault_access: String,
    pub last_sync_ms: Option<i64>,
    pub last_sync_error: Option<String>,
    /// Records whose local change lost to another device's (kept as
    /// conflict copies in the vault).
    pub conflicts: Vec<ConflictInfo>,
    pub storage_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    pub key: String,
    /// The record it conflicts with.
    pub of: String,
    pub summary: String,
}

fn http_error(e: &HttpError) -> CommandError {
    let kind = match e.status {
        0 => ErrorKind::Network,
        401 => ErrorKind::InvalidCredentials,
        403 => ErrorKind::AccessDenied,
        409 => ErrorKind::Partial,
        429 => ErrorKind::Busy,
        400 => ErrorKind::InvalidInput,
        _ => ErrorKind::Provider,
    };
    CommandError::new(kind, e.to_string())
}

fn crypto_error(e: &CryptoError) -> CommandError {
    CommandError::new(
        match e {
            CryptoError::RecoveryKey => ErrorKind::InvalidInput,
            _ => ErrorKind::Integrity,
        },
        e.to_string(),
    )
}

fn secret_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(ErrorKind::CredentialStore, e.to_string())
}

fn not_signed_in() -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        "sign in to your sotyware account first",
    )
}

/// The account service: managed state.
pub struct AccountService {
    http: Option<Http>,
    config_error: Option<String>,
    secrets: Arc<dyn SecretStore>,
    state: Mutex<Result<Box<dyn StateStore>, String>>,
    access: Mutex<Option<String>>,
    accounts: Arc<CloudAccounts>,
    backups: OnceLock<Arc<Backups>>,
    /// One sync (or key operation) at a time.
    busy: tokio::sync::Mutex<()>,
}

impl AccountService {
    pub fn new(
        server: Option<String>,
        secrets: Arc<dyn SecretStore>,
        state: Result<Box<dyn StateStore>, StoreError>,
        accounts: Arc<CloudAccounts>,
    ) -> Arc<Self> {
        let (http, config_error) = match server.as_deref().map(Http::new) {
            Some(Ok(h)) => (Some(h), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
        };
        Arc::new(Self {
            http,
            config_error,
            secrets,
            state: Mutex::new(state.map_err(|e| e.to_string())),
            access: Mutex::new(None),
            accounts,
            backups: OnceLock::new(),
            busy: tokio::sync::Mutex::new(()),
        })
    }

    /// Lets backup jobs sync too. Once.
    pub fn with_backups(&self, backups: Arc<Backups>) {
        let _ = self.backups.set(backups);
    }

    fn http(&self) -> Result<&Http, CommandError> {
        self.http.as_ref().ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidConfiguration,
                self.config_error.clone().unwrap_or_else(|| {
                    "sotyware accounts are not available in this build: no account service is configured".into()
                }),
            )
        })
    }

    fn local(&self) -> Result<Option<LocalState>, CommandError> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let store = guard
            .as_mut()
            .map_err(|e| CommandError::new(ErrorKind::Storage, e.clone()))?;
        Ok(store
            .load()
            .map_err(|e| CommandError::new(ErrorKind::Storage, e.to_string()))?
            .and_then(|s| serde_json::from_str(&s).ok()))
    }

    fn save_local(&self, s: &LocalState) -> Result<(), CommandError> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let store = guard
            .as_mut()
            .map_err(|e| CommandError::new(ErrorKind::Storage, e.clone()))?;
        store
            .save(&serde_json::to_string(s).expect("serializes"))
            .map_err(|e| CommandError::new(ErrorKind::Storage, e.to_string()))
    }

    fn clear_local(&self) -> Result<(), CommandError> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(store) = guard.as_mut() {
            store
                .clear()
                .map_err(|e| CommandError::new(ErrorKind::Storage, e.to_string()))?;
        }
        Ok(())
    }

    fn signed_in(&self) -> Result<LocalState, CommandError> {
        self.local()?.ok_or_else(not_signed_in)
    }

    fn secret(&self, name: &str) -> Result<Option<Zeroizing<String>>, CommandError> {
        self.secrets.get(name).map_err(secret_error)
    }

    fn device_key(&self) -> Result<DeviceKey, CommandError> {
        if let Some(s) = self.secret(DEVICE_KEY)? {
            return Ok(DeviceKey::from_secret(
                &SecretKey::from_base64(&s).map_err(|e| crypto_error(&e))?,
            ));
        }
        let k = DeviceKey::generate();
        self.secrets
            .set(DEVICE_KEY, &k.secret().to_base64())
            .map_err(secret_error)?;
        Ok(k)
    }

    fn account_key(&self) -> Result<Option<SecretKey>, CommandError> {
        match self.secret(ACCOUNT_KEY)? {
            Some(s) => Ok(Some(
                SecretKey::from_base64(&s).map_err(|e| crypto_error(&e))?,
            )),
            None => Ok(None),
        }
    }

    fn store_account_key(&self, ak: &SecretKey) -> Result<(), CommandError> {
        self.secrets
            .set(ACCOUNT_KEY, &ak.to_base64())
            .map_err(secret_error)
    }

    /// One authenticated call; refreshes the access token once on 401.
    async fn api(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), CommandError> {
        let http = self.http()?;
        let token = self
            .access
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(t) = &token {
            match http.call(method, path, Some(t), body.clone()).await {
                Err(e) if e.status == 401 => {}
                other => return other.map_err(|e| http_error(&e)),
            }
        }
        self.refresh().await?;
        let token = self
            .access
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        http.call(method, path, token.as_deref(), body)
            .await
            .map_err(|e| http_error(&e))
    }

    async fn refresh(&self) -> Result<(), CommandError> {
        let http = self.http()?;
        let Some(refresh) = self.secret(REFRESH)? else {
            return Err(not_signed_in());
        };
        let (_, t) = http
            .call(
                "POST",
                "/v1/sessions/refresh",
                None,
                Some(json!({ "refreshToken": &*refresh })),
            )
            .await
            .map_err(|e| {
                if e.status == 401 {
                    CommandError::new(
                        ErrorKind::InvalidCredentials,
                        "your session ended; sign in again",
                    )
                } else {
                    http_error(&e)
                }
            })?;
        self.take_tokens(&t)?;
        Ok(())
    }

    fn take_tokens(&self, t: &Value) -> Result<(), CommandError> {
        let (Some(access), Some(refresh)) = (t["accessToken"].as_str(), t["refreshToken"].as_str())
        else {
            return Err(CommandError::new(
                ErrorKind::Provider,
                "the account service sent no tokens",
            ));
        };
        self.secrets.set(REFRESH, refresh).map_err(secret_error)?;
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = Some(access.to_owned());
        Ok(())
    }

    // ------------------------------------------------------------ status

    pub async fn status(&self) -> AccountStatus {
        let local = self.local();
        let (state, storage_error) = match local {
            Ok(s) => (s, None),
            Err(e) => (None, Some(e.message)),
        };
        let mut vault_access = "unknown".to_owned();
        if state.is_some()
            && self.http.is_some()
            && let Ok((_, a)) = self.api("GET", "/v1/account", None).await
        {
            vault_access = match (a["device"]["state"].as_str(), a["syncSetUp"].as_bool()) {
                (Some("enrolled"), _) => "enrolled".into(),
                (_, Some(true)) => "pending".into(),
                _ => "none".into(),
            };
        }
        let conflicts = self.conflicts().unwrap_or_default();
        AccountStatus {
            configured: self.http.is_some(),
            service: self.http.as_ref().map(|h| h.base().to_owned()),
            signed_in: state.is_some(),
            email: state.as_ref().map(|s| s.email.clone()),
            device_id: state.as_ref().map(|s| s.device_id.clone()),
            vault_access,
            last_sync_ms: state.as_ref().and_then(|s| s.last_sync_ms),
            last_sync_error: state.as_ref().and_then(|s| s.last_sync_error.clone()),
            conflicts,
            storage_error: storage_error.or_else(|| self.config_error.clone()),
        }
    }

    /// The conflict copies in the last synced vault.
    fn conflicts(&self) -> Result<Vec<ConflictInfo>, CommandError> {
        let Some(state) = self.local()? else {
            return Ok(Vec::new());
        };
        let Some(ak) = self.account_key()? else {
            return Ok(Vec::new());
        };
        let Some(env) = state.base_envelope.as_deref() else {
            return Ok(Vec::new());
        };
        let doc = decrypt_doc(&ak, &state.account_id, env)?;
        Ok(doc
            .records
            .iter()
            .filter(|(k, r)| k.starts_with("conflict:") && !r.deleted)
            .map(|(k, r)| ConflictInfo {
                key: k.clone(),
                of: k
                    .trim_start_matches("conflict:")
                    .rsplit_once(':')
                    .map_or(k.clone(), |(of, _)| of.to_owned()),
                summary: describe(k.trim_start_matches("conflict:"), &r.data),
            })
            .collect())
    }

    // ------------------------------------------------------- registration

    pub async fn register(&self, email: &str, password: &str) -> Result<(), CommandError> {
        self.http()?
            .call(
                "POST",
                "/v1/accounts",
                None,
                Some(json!({ "email": email, "password": password })),
            )
            .await
            .map(drop)
            .map_err(|e| http_error(&e))
    }

    pub async fn verify(
        &self,
        email: &str,
        code: &str,
        password: &str,
    ) -> Result<(), CommandError> {
        self.http()?
            .call(
                "POST",
                "/v1/accounts/verify",
                None,
                Some(json!({ "email": email, "code": code, "password": password })),
            )
            .await
            .map(drop)
            .map_err(|e| http_error(&e))
    }

    /// Signs in this device; returns its vault access.
    pub async fn sign_in(
        &self,
        email: &str,
        password: &str,
        device_name: &str,
    ) -> Result<String, CommandError> {
        let http = self.http()?;
        let device = self.device_key()?;
        let previous = self.local()?;
        let (_, t) = http
            .call(
                "POST",
                "/v1/sessions",
                None,
                Some(json!({
                    "email": email,
                    "password": password,
                    "device": {
                        "id": previous.as_ref().map(|p| p.device_id.clone()),
                        "name": device_name,
                        "platform": std::env::consts::OS,
                        "publicKey": device.public_base64(),
                    }
                })),
            )
            .await
            .map_err(|e| http_error(&e))?;
        self.take_tokens(&t)?;
        let account_id = t["accountId"].as_str().unwrap_or_default().to_owned();
        let same_account = previous
            .as_ref()
            .is_some_and(|p| p.account_id == account_id);
        if !same_account {
            // Another account than before: nothing of the old one applies.
            self.secrets.delete(ACCOUNT_KEY).map_err(secret_error)?;
        }
        let state = LocalState {
            account_id,
            email: email.trim().to_lowercase(),
            device_id: t["deviceId"].as_str().unwrap_or_default().to_owned(),
            device_name: device_name.to_owned(),
            ..if same_account {
                previous.unwrap_or_default()
            } else {
                LocalState::default()
            }
        };
        self.save_local(&state)?;
        let access = t["vaultAccess"].as_str().unwrap_or("none").to_owned();
        if access != "enrolled" {
            self.secrets.delete(ACCOUNT_KEY).map_err(secret_error)?;
        }
        Ok(access)
    }

    /// Ends the session here; local storage accounts stay. The account key
    /// and tokens are removed from this computer.
    pub async fn sign_out(&self) -> Result<(), CommandError> {
        if self.local()?.is_some() && self.http.is_some() {
            let _ = self.api("DELETE", "/v1/sessions/current", None).await;
        }
        for name in [REFRESH, ACCOUNT_KEY] {
            self.secrets.delete(name).map_err(secret_error)?;
        }
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.clear_local()
    }

    // ------------------------------------------------------------- keys

    /// First device: creates the account key and the recovery key, uploads
    /// the recovery wrap, this device's wrap and the first vault. Returns
    /// the recovery key to show once.
    pub async fn set_up_sync(&self) -> Result<Zeroizing<String>, CommandError> {
        let _busy = self.busy.lock().await;
        let state = self.signed_in()?;
        let device = self.device_key()?;
        let (ak, rk) = (SecretKey::generate(), SecretKey::generate());
        let proof = crypto::key_proof(&ak, &state.account_id);
        self.api(
            "PUT",
            "/v1/recovery",
            Some(json!({ "wrap": crypto::wrap_with_recovery(&ak, &rk, &state.account_id), "newKeyProof": proof })),
        )
        .await?;
        let sealed = crypto::seal_for_device(&ak, &state.account_id, &device.public_base64())
            .map_err(|e| crypto_error(&e))?;
        self.api(
            "PUT",
            "/v1/devices/current/wrap",
            Some(json!({ "wrap": sealed, "keyProof": proof })),
        )
        .await?;
        self.store_account_key(&ak)?;
        self.sync_locked(true).await?;
        Ok(crypto::format_recovery_key(&rk))
    }

    /// Whether `typed` is this account's recovery key (it unwraps the
    /// recovery wrap). Used to make sure the user wrote it down correctly.
    pub async fn check_recovery_key(&self, typed: &str) -> Result<bool, CommandError> {
        let state = self.signed_in()?;
        let rk = crypto::parse_recovery_key(typed).map_err(|e| crypto_error(&e))?;
        let (_, r) = self.api("GET", "/v1/recovery", None).await?;
        let wrap = r["wrap"].as_str().unwrap_or_default();
        Ok(crypto::unwrap_with_recovery(wrap, &rk, &state.account_id).is_ok())
    }

    /// A pending device unlocks with the recovery key and enrolls itself.
    pub async fn unlock(&self, typed: &str) -> Result<(), CommandError> {
        let _busy = self.busy.lock().await;
        let state = self.signed_in()?;
        let rk = crypto::parse_recovery_key(typed).map_err(|e| crypto_error(&e))?;
        let (_, r) = self.api("GET", "/v1/recovery", None).await?;
        let ak = crypto::unwrap_with_recovery(
            r["wrap"].as_str().unwrap_or_default(),
            &rk,
            &state.account_id,
        )
        .map_err(|_| {
            CommandError::new(
                ErrorKind::InvalidInput,
                "this recovery key does not unlock this account",
            )
        })?;
        self.enroll_self(&state, &ak).await?;
        self.sync_locked(true).await
    }

    async fn enroll_self(&self, state: &LocalState, ak: &SecretKey) -> Result<(), CommandError> {
        let device = self.device_key()?;
        let sealed = crypto::seal_for_device(ak, &state.account_id, &device.public_base64())
            .map_err(|e| crypto_error(&e))?;
        self.api(
            "PUT",
            "/v1/devices/current/wrap",
            Some(json!({ "wrap": sealed, "keyProof": crypto::key_proof(ak, &state.account_id) })),
        )
        .await?;
        self.store_account_key(ak)
    }

    /// A pending device asks an enrolled one for approval: the request id
    /// and the fingerprint to compare on both screens.
    pub async fn request_approval(&self) -> Result<(String, String), CommandError> {
        self.signed_in()?;
        let (_, r) = self.api("POST", "/v1/enrollments", None).await?;
        let id = r["id"].as_str().unwrap_or_default().to_owned();
        let pk = r["publicKey"].as_str().unwrap_or_default();
        Ok((id.clone(), crypto::fingerprint(pk, &id)))
    }

    /// The requesting device checks its request; on approval it opens the
    /// sealed key, keeps it, and syncs. Returns the request's state.
    pub async fn approval_status(&self, id: &str) -> Result<String, CommandError> {
        let state = self.signed_in()?;
        let (_, r) = self
            .api("GET", &format!("/v1/enrollments/{id}"), None)
            .await?;
        let s = r["state"].as_str().unwrap_or("unknown").to_owned();
        if s == "approved"
            && let Some(sealed) = r["sealedKey"].as_str()
        {
            let ak = crypto::open_for_device(&self.device_key()?, &state.account_id, sealed)
                .map_err(|e| crypto_error(&e))?;
            self.store_account_key(&ak)?;
            let _busy = self.busy.lock().await;
            self.sync_locked(true).await?;
        }
        Ok(s)
    }

    /// Pending requests with their fingerprints (enrolled devices only).
    pub async fn enrollments(&self) -> Result<Vec<Value>, CommandError> {
        self.signed_in()?;
        let (_, r) = self.api("GET", "/v1/enrollments", None).await?;
        Ok(r["enrollments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|e| {
                let fp = crypto::fingerprint(
                    e["publicKey"].as_str().unwrap_or_default(),
                    e["id"].as_str().unwrap_or_default(),
                );
                json!({
                    "id": e["id"],
                    "deviceName": e["deviceName"],
                    "platform": e["platform"],
                    "createdMs": e["createdMs"],
                    "fingerprint": fp,
                })
            })
            .collect())
    }

    /// Approves (seals the account key to the requesting device and adds it
    /// to the trusted devices in the vault) or rejects a request.
    pub async fn answer_enrollment(&self, id: &str, approve: bool) -> Result<(), CommandError> {
        let state = self.signed_in()?;
        if !approve {
            self.api("POST", &format!("/v1/enrollments/{id}/reject"), None)
                .await?;
            return Ok(());
        }
        let ak = self.account_key()?.ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                "this device cannot approve others: it has no key yet",
            )
        })?;
        let (_, list) = self.api("GET", "/v1/enrollments", None).await?;
        let req = list["enrollments"]
            .as_array()
            .and_then(|a| a.iter().find(|e| e["id"] == id))
            .cloned()
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::OutdatedPlan,
                    "this request expired or was answered",
                )
            })?;
        let pk = req["publicKey"].as_str().unwrap_or_default().to_owned();
        let sealed =
            crypto::seal_for_device(&ak, &state.account_id, &pk).map_err(|e| crypto_error(&e))?;
        self.api(
            "POST",
            &format!("/v1/enrollments/{id}/approve"),
            Some(json!({ "sealedKey": sealed, "keyProof": crypto::key_proof(&ak, &state.account_id) })),
        )
        .await?;
        let _busy = self.busy.lock().await;
        let trusted = (
            req["deviceId"].as_str().unwrap_or_default().to_owned(),
            TrustedDevice {
                public_key: pk,
                name: req["deviceName"].as_str().unwrap_or_default().to_owned(),
                added_by: state.device_id.clone(),
            },
        );
        self.sync_with(true, Some(trusted), None).await
    }

    pub async fn devices(&self) -> Result<Value, CommandError> {
        self.signed_in()?;
        Ok(self.api("GET", "/v1/devices", None).await?.1["devices"].clone())
    }

    /// Revokes a device at the service. The caller should rotate the keys
    /// next (the page asks), because the device may still hold the key.
    pub async fn revoke(&self, device: &str) -> Result<(), CommandError> {
        let state = self.signed_in()?;
        self.api("DELETE", &format!("/v1/devices/{device}"), None)
            .await?;
        if device == state.device_id {
            for name in [REFRESH, ACCOUNT_KEY] {
                self.secrets.delete(name).map_err(secret_error)?;
            }
            self.clear_local()?;
            return Ok(());
        }
        let _busy = self.busy.lock().await;
        self.sync_with(false, None, Some(device.to_owned())).await
    }

    /// Replaces the account key in one atomic service operation
    /// (`/v1/keys/rotate`): the vault re-encrypted under the new key, the
    /// new key wrapped to the recovery key (the current one, typed, or a
    /// new one, returned to show) and sealed to every remaining trusted
    /// device, all or nothing. Only devices the vault itself lists as
    /// trusted get the new key: the service cannot add one of its own.
    pub async fn rotate(
        &self,
        current_recovery: Option<&str>,
    ) -> Result<Option<Zeroizing<String>>, CommandError> {
        let _busy = self.busy.lock().await;
        self.sync_locked(true).await?;
        let (rk, new_rk) = match current_recovery {
            Some(t) => (
                crypto::parse_recovery_key(t).map_err(|e| crypto_error(&e))?,
                None,
            ),
            None => {
                let rk = SecretKey::generate();
                let shown = crypto::format_recovery_key(&rk);
                (rk, Some(shown))
            }
        };
        for _ in 0..4 {
            let mut state = self.signed_in()?;
            let old = self.current_key(&state).await?;
            if new_rk.is_none() {
                let (_, r) = self.api("GET", "/v1/recovery", None).await?;
                crypto::unwrap_with_recovery(
                    r["wrap"].as_str().unwrap_or_default(),
                    &rk,
                    &state.account_id,
                )
                .map_err(|_| {
                    CommandError::new(
                        ErrorKind::InvalidInput,
                        "this is not the account's recovery key",
                    )
                })?;
            }
            let (rev, mut doc) = self
                .remote_doc(&state, &old)
                .await?
                .unwrap_or((0, Doc::empty()));
            let ak = SecretKey::generate();
            let (_, devices) = self.api("GET", "/v1/devices", None).await?;
            let mut wraps = vec![json!({
                "deviceId": state.device_id,
                "wrap": crypto::seal_for_device(&ak, &state.account_id, &self.device_key()?.public_base64()).map_err(|e| crypto_error(&e))?,
            })];
            let enrolled: Vec<(String, String)> = devices["devices"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|d| d["state"] == "enrolled")
                .map(|d| {
                    (
                        d["id"].as_str().unwrap_or_default().to_owned(),
                        d["publicKey"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            doc.trusted_devices.retain(|id, t| {
                id == &state.device_id
                    || enrolled
                        .iter()
                        .any(|(eid, pk)| eid == id && *pk == t.public_key)
            });
            for (id, t) in &doc.trusted_devices {
                if id != &state.device_id {
                    wraps.push(json!({
                        "deviceId": id,
                        "wrap": crypto::seal_for_device(&ak, &state.account_id, &t.public_key).map_err(|e| crypto_error(&e))?,
                    }));
                }
            }
            doc.revision = rev + 1;
            doc.updated_at = now_ms();
            doc.updated_by = state.device_id.clone();
            let envelope = crypto::encrypt_vault(
                &ak,
                &state.account_id,
                &serde_json::to_vec(&doc).expect("serializes"),
            );
            let (status, _) = self
                .api(
                    "POST",
                    "/v1/keys/rotate",
                    Some(json!({
                        "recoveryWrap": crypto::wrap_with_recovery(&ak, &rk, &state.account_id),
                        "keyProof": crypto::key_proof(&old, &state.account_id),
                        "newKeyProof": crypto::key_proof(&ak, &state.account_id),
                        "expectedRevision": rev,
                        "envelope": envelope,
                        "deviceWraps": wraps,
                    })),
                )
                .await?;
            if status == 409 {
                // Another device wrote meanwhile: merge, then try again.
                self.sync_locked(false).await?;
                continue;
            }
            // Committed at the service. Should this store fail, the next
            // sync recovers the key from this device's new wrap.
            self.store_account_key(&ak)?;
            state.highest_revision = rev + 1;
            state.base_envelope = Some(envelope);
            state.last_sync_ms = Some(now_ms());
            state.last_sync_error = None;
            self.save_local(&state)?;
            return Ok(new_rk);
        }
        Err(CommandError::new(
            ErrorKind::Busy,
            "other devices kept changing the vault; try again",
        ))
    }

    // ------------------------------------------------------------ password

    pub async fn change_password(&self, current: &str, new: &str) -> Result<(), CommandError> {
        self.signed_in()?;
        self.api(
            "POST",
            "/v1/account/password",
            Some(json!({ "currentPassword": current, "newPassword": new })),
        )
        .await
        .map(drop)
    }

    pub async fn request_reset(&self, email: &str) -> Result<(), CommandError> {
        self.http()?
            .call(
                "POST",
                "/v1/password-reset/request",
                None,
                Some(json!({ "email": email })),
            )
            .await
            .map(drop)
            .map_err(|e| http_error(&e))
    }

    pub async fn confirm_reset(
        &self,
        email: &str,
        code: &str,
        new: &str,
    ) -> Result<(), CommandError> {
        self.http()?
            .call(
                "POST",
                "/v1/password-reset/confirm",
                None,
                Some(json!({ "email": email, "code": code, "newPassword": new })),
            )
            .await
            .map(drop)
            .map_err(|e| http_error(&e))?;
        // Every device lost its key copy: this one too.
        self.secrets.delete(ACCOUNT_KEY).map_err(secret_error)?;
        Ok(())
    }

    /// Deletes the account at the service. Local storage accounts stay.
    pub async fn delete(&self, password: &str) -> Result<(), CommandError> {
        self.signed_in()?;
        self.api(
            "DELETE",
            "/v1/account",
            Some(json!({ "password": password })),
        )
        .await?;
        for name in [REFRESH, ACCOUNT_KEY] {
            self.secrets.delete(name).map_err(secret_error)?;
        }
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.clear_local()
    }

    // --------------------------------------------------------------- sync

    pub async fn sync(&self) -> Result<(), CommandError> {
        let _busy = self.busy.lock().await;
        self.sync_locked(false).await
    }

    /// Resolves a conflict copy: `keep_local` puts its value back as the
    /// record; otherwise the copy is dropped.
    pub async fn resolve_conflict(&self, key: &str, keep_local: bool) -> Result<(), CommandError> {
        if !key.starts_with("conflict:") {
            return Err(CommandError::new(ErrorKind::InvalidInput, "not a conflict"));
        }
        let _busy = self.busy.lock().await;
        let state = self.signed_in()?;
        let ak = self
            .account_key()?
            .ok_or_else(|| CommandError::new(ErrorKind::InvalidInput, "this device has no key"))?;
        let Some((rev, mut doc)) = self.remote_doc(&state, &ak).await? else {
            return Ok(());
        };
        let now = now_ms();
        if let Some(copy) = doc.records.remove(key)
            && keep_local
        {
            let of = key
                .trim_start_matches("conflict:")
                .rsplit_once(':')
                .map(|(of, _)| of.to_owned())
                .unwrap_or_default();
            let next = doc.records.get(&of).map_or(0, |r| r.rev) + 1;
            doc.records.insert(
                of,
                Record {
                    rev: next,
                    updated_at: now,
                    device: state.device_id.clone(),
                    ..copy
                },
            );
        }
        if self.write_doc(&state, &ak, rev, doc).await?.is_err() {
            return Err(CommandError::new(
                ErrorKind::Busy,
                "another computer changed the vault meanwhile; sync and choose again",
            ));
        }
        self.sync_locked(false).await
    }

    async fn remote_doc(
        &self,
        state: &LocalState,
        ak: &SecretKey,
    ) -> Result<Option<(i64, Doc)>, CommandError> {
        match self.api("GET", "/v1/vault", None).await {
            Ok((_, v)) => {
                let rev = v["revision"].as_i64().unwrap_or(0);
                let env = v["envelope"].as_str().unwrap_or_default();
                let doc = decrypt_doc(ak, &state.account_id, env)?;
                if doc.revision != rev || rev < state.highest_revision {
                    return Err(CommandError::new(
                        ErrorKind::Integrity,
                        "the account service returned an older or altered vault than this device has seen; nothing was changed",
                    ));
                }
                Ok(Some((rev, doc)))
            }
            Err(e) if e.message.contains("no-vault") || e.message.contains("no vault") => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn write_doc(
        &self,
        state: &LocalState,
        ak: &SecretKey,
        expected: i64,
        mut doc: Doc,
    ) -> Result<Result<String, i64>, CommandError> {
        doc.revision = expected + 1;
        doc.updated_at = now_ms();
        doc.updated_by = state.device_id.clone();
        let env = crypto::encrypt_vault(
            ak,
            &state.account_id,
            &serde_json::to_vec(&doc).expect("serializes"),
        );
        let (status, r) = self
            .api(
                "PUT",
                "/v1/vault",
                Some(json!({ "expectedRevision": expected, "envelope": env, "keyProof": crypto::key_proof(ak, &state.account_id) })),
            )
            .await?;
        if status == 409 {
            return Ok(Err(r["revision"].as_i64().unwrap_or(expected)));
        }
        Ok(Ok(env))
    }

    async fn sync_locked(&self, add_self: bool) -> Result<(), CommandError> {
        self.sync_with(add_self, None, None).await
    }

    /// One sync: read, merge, apply, write; retried on a concurrent write.
    async fn sync_with(
        &self,
        add_self: bool,
        trust: Option<(String, TrustedDevice)>,
        untrust: Option<String>,
    ) -> Result<(), CommandError> {
        let mut state = self.signed_in()?;
        let result = async {
            for _ in 0..4 {
                let ak = self.current_key(&state).await?;
                let remote = self.remote_doc(&state, &ak).await?;
                let (rev, mut doc) = remote.unwrap_or((0, Doc::empty()));
                let base = match state.base_envelope.as_deref() {
                    Some(env) => decrypt_doc(&ak, &state.account_id, env)
                        .map(|d| d.records)
                        .unwrap_or_default(),
                    None => BTreeMap::new(),
                };
                let now = now_ms();
                let local = vault::with_tombstones(
                    self.local_records(&state, &doc.records)?,
                    &base,
                    &state.device_id,
                    now,
                );
                let merged = vault::merge(&base, &local, &doc.records, &state.device_id, now);
                self.apply(&merged.records, &local)?;
                let mut must_write = merged.changed || rev == 0;
                doc.records = merged.records;
                if add_self && !doc.trusted_devices.contains_key(&state.device_id) {
                    doc.trusted_devices.insert(
                        state.device_id.clone(),
                        TrustedDevice {
                            public_key: self.device_key()?.public_base64(),
                            name: state.device_name.clone(),
                            added_by: state.device_id.clone(),
                        },
                    );
                    must_write = true;
                }
                if let Some((id, t)) = &trust {
                    doc.trusted_devices.insert(id.clone(), t.clone());
                    must_write = true;
                }
                if let Some(id) = &untrust {
                    must_write |= doc.trusted_devices.remove(id).is_some();
                }
                // A vault encrypted under an older key is re-encrypted.
                let key_changed = self.remote_kid().await.is_some_and(|kid| kid != ak.id());
                if !must_write && !key_changed {
                    state.highest_revision = state.highest_revision.max(rev);
                    if let Some((_, v)) = self
                        .api("GET", "/v1/vault", None)
                        .await
                        .ok()
                        .filter(|(s, _)| *s == 200)
                    {
                        state.base_envelope = v["envelope"].as_str().map(str::to_owned);
                    }
                    return Ok(());
                }
                match self.write_doc(&state, &ak, rev, doc).await? {
                    Ok(env) => {
                        state.highest_revision = rev + 1;
                        state.base_envelope = Some(env);
                        return Ok(());
                    }
                    // Another device wrote meanwhile: merge again.
                    Err(_) => continue,
                }
            }
            Err(CommandError::new(
                ErrorKind::Busy,
                "other devices kept changing the vault; try again",
            ))
        }
        .await;
        state.last_sync_ms = Some(now_ms());
        state.last_sync_error = result.as_ref().err().map(|e| e.message.clone());
        self.save_local(&state)?;
        result
    }

    /// The account key, renewed from this device's wrap if the vault was
    /// re-encrypted by a rotation on another device.
    async fn current_key(&self, state: &LocalState) -> Result<SecretKey, CommandError> {
        let ak = self.account_key()?.ok_or_else(|| {
            CommandError::new(ErrorKind::InvalidInput, "this device has no key yet: unlock it with the recovery key or approve it from another device")
        })?;
        match self.remote_kid().await {
            Some(kid) if kid != ak.id() => {
                let (_, w) = self.api("GET", "/v1/devices/current/wrap", None).await?;
                let fresh = crypto::open_for_device(
                    &self.device_key()?,
                    &state.account_id,
                    w["wrap"].as_str().unwrap_or_default(),
                )
                .map_err(|e| crypto_error(&e))?;
                if fresh.id() == kid {
                    self.store_account_key(&fresh)?;
                    return Ok(fresh);
                }
                Ok(ak)
            }
            _ => Ok(ak),
        }
    }

    async fn remote_kid(&self) -> Option<String> {
        let (_, v) = self.api("GET", "/v1/vault", None).await.ok()?;
        let env: crypto::Envelope = serde_json::from_str(v["envelope"].as_str()?).ok()?;
        Some(env.kid)
    }

    /// This device's records: storage accounts with their credentials, and
    /// backup jobs. An account without local credentials takes them from
    /// the remote record (it was applied but its key could not be stored).
    fn local_records(
        &self,
        state: &LocalState,
        remote: &BTreeMap<String, Record>,
    ) -> Result<BTreeMap<String, Record>, CommandError> {
        let mut out = BTreeMap::new();
        let stamp = |key: &str, data: Value| {
            let rev = remote.get(key).map_or(0, |r| r.rev);
            (
                key.to_owned(),
                Record {
                    rev,
                    updated_at: now_ms(),
                    device: state.device_id.clone(),
                    deleted: false,
                    data,
                },
            )
        };
        for (config, creds) in self.accounts.export_for_sync()? {
            let key = format!("storage-account:{}", config.id);
            let creds = match creds {
                Some(c) => Some(
                    json!({ "accessKeyId": c.access_key_id, "secretAccessKey": c.secret_access_key.expose() }),
                ),
                None => remote
                    .get(&key)
                    .and_then(|r| r.data.get("credentials").cloned())
                    .filter(|c| !c.is_null()),
            };
            let (k, r) = stamp(
                &key,
                json!({ "config": config_json(&config), "credentials": creds }),
            );
            out.insert(k, r);
        }
        if let Some(b) = self.backups.get() {
            for job in b.jobs()? {
                let (k, r) = stamp(
                    &format!("backup-job:{}", job.id),
                    serde_json::to_value(&job).expect("serializes"),
                );
                out.insert(k, r);
            }
        }
        Ok(out)
    }

    /// Applies records that differ from this device's state.
    fn apply(
        &self,
        merged: &BTreeMap<String, Record>,
        local: &BTreeMap<String, Record>,
    ) -> Result<(), CommandError> {
        for (key, rec) in merged {
            if key.starts_with("conflict:") || local.get(key).is_some_and(|l| l.same(rec)) {
                continue;
            }
            if let Some(id) = key.strip_prefix("storage-account:") {
                if rec.deleted {
                    if local.get(key).is_some_and(|l| !l.deleted) {
                        self.accounts.remove(id)?;
                        // Its folder sync is turned off here too (ADR-043).
                        crate::folder_sync::account_removed(id);
                    }
                    continue;
                }
                let config = config_from(&rec.data["config"]).ok_or_else(|| {
                    CommandError::new(ErrorKind::Integrity, "a synced account is malformed")
                })?;
                let creds = rec.data["credentials"].as_object().map(|c| Credentials {
                    access_key_id: c
                        .get("accessKeyId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    secret_access_key: Secret::new(
                        c.get("secretAccessKey")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                });
                self.accounts.import_synced(config, creds.as_ref())?;
            } else if let (Some(id), Some(b)) =
                (key.strip_prefix("backup-job:"), self.backups.get())
            {
                if rec.deleted {
                    if local.get(key).is_some_and(|l| !l.deleted) {
                        b.remove(id)?;
                    }
                    continue;
                }
                if let Ok(job) =
                    serde_json::from_value::<crate::backup::model::JobConfig>(rec.data.clone())
                {
                    // Only once both accounts exist here (synced first).
                    let ids: Vec<String> = self
                        .accounts
                        .list()
                        .accounts
                        .into_iter()
                        .map(|a| a.id)
                        .collect();
                    if ids.contains(&job.source.account) && ids.contains(&job.destination.account) {
                        b.save(&job)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn decrypt_doc(ak: &SecretKey, account: &str, env: &str) -> Result<Doc, CommandError> {
    let plain = crypto::decrypt_vault(ak, account, env).map_err(|e| crypto_error(&e))?;
    let doc: Doc = serde_json::from_slice(&plain).map_err(|e| {
        CommandError::new(ErrorKind::Integrity, format!("the vault is malformed: {e}"))
    })?;
    doc.check()
        .map_err(|e| CommandError::new(ErrorKind::Integrity, e))?;
    Ok(doc)
}

fn config_json(c: &AccountConfig) -> Value {
    json!({
        "id": c.id,
        "label": c.label,
        "provider": c.provider,
        "region": c.region,
        "endpoint": c.endpoint,
        "pathStyle": c.path_style,
        "buckets": c.buckets,
        "maxConcurrentTransfers": c.max_concurrent_transfers,
    })
}

fn config_from(v: &Value) -> Option<AccountConfig> {
    Some(AccountConfig {
        id: v["id"].as_str()?.to_owned(),
        label: v["label"].as_str()?.to_owned(),
        provider: v["provider"].as_str()?.to_owned(),
        region: v["region"].as_str()?.to_owned(),
        endpoint: v["endpoint"].as_str().map(str::to_owned),
        path_style: v["pathStyle"].as_bool(),
        buckets: v["buckets"]
            .as_array()?
            .iter()
            .filter_map(|b| b.as_str().map(str::to_owned))
            .collect(),
        max_concurrent_transfers: u8::try_from(v["maxConcurrentTransfers"].as_u64()?).ok()?,
    })
}

/// A record in words, without any secret.
fn describe(key: &str, data: &Value) -> String {
    if key.starts_with("storage-account:") {
        format!(
            "Storage account \"{}\"",
            data["config"]["label"].as_str().unwrap_or("?")
        )
    } else if key.starts_with("backup-job:") {
        format!("Backup job \"{}\"", data["name"].as_str().unwrap_or("?"))
    } else {
        key.to_owned()
    }
}

// --------------------------------------------------------------- commands

type S<'a> = State<'a, Arc<AccountService>>;

#[tauri::command]
pub async fn account_status(svc: S<'_>) -> Result<AccountStatus, CommandError> {
    Ok(svc.status().await)
}

#[tauri::command]
pub async fn account_register(
    svc: S<'_>,
    email: String,
    password: String,
) -> Result<(), CommandError> {
    svc.register(&email, &password).await
}

#[tauri::command]
pub async fn account_verify(
    svc: S<'_>,
    email: String,
    code: String,
    password: String,
) -> Result<(), CommandError> {
    svc.verify(&email, &code, &password).await
}

#[tauri::command]
pub async fn account_sign_in(
    svc: S<'_>,
    email: String,
    password: String,
    device_name: String,
) -> Result<String, CommandError> {
    svc.sign_in(&email, &password, &device_name).await
}

#[tauri::command]
pub async fn account_sign_out(svc: S<'_>) -> Result<(), CommandError> {
    svc.sign_out().await
}

/// Returns the recovery key, once, to show.
#[tauri::command]
pub async fn account_set_up_sync(svc: S<'_>) -> Result<String, CommandError> {
    svc.set_up_sync().await.map(|k| k.to_string())
}

#[tauri::command]
pub async fn account_check_recovery_key(svc: S<'_>, key: String) -> Result<bool, CommandError> {
    svc.check_recovery_key(&key).await
}

#[tauri::command]
pub async fn account_unlock(svc: S<'_>, key: String) -> Result<(), CommandError> {
    svc.unlock(&key).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    id: String,
    fingerprint: String,
}

#[tauri::command]
pub async fn account_request_approval(svc: S<'_>) -> Result<ApprovalRequest, CommandError> {
    let (id, fingerprint) = svc.request_approval().await?;
    Ok(ApprovalRequest { id, fingerprint })
}

#[tauri::command]
pub async fn account_approval_status(svc: S<'_>, id: String) -> Result<String, CommandError> {
    svc.approval_status(&id).await
}

#[tauri::command]
pub async fn account_enrollments(svc: S<'_>) -> Result<Vec<Value>, CommandError> {
    svc.enrollments().await
}

#[tauri::command]
pub async fn account_answer_enrollment(
    svc: S<'_>,
    id: String,
    approve: bool,
) -> Result<(), CommandError> {
    svc.answer_enrollment(&id, approve).await
}

#[tauri::command]
pub async fn account_devices(svc: S<'_>) -> Result<Value, CommandError> {
    svc.devices().await
}

#[tauri::command]
pub async fn account_revoke_device(svc: S<'_>, id: String) -> Result<(), CommandError> {
    svc.revoke(&id).await
}

/// Returns a new recovery key when none was typed.
#[tauri::command]
pub async fn account_rotate_keys(
    svc: S<'_>,
    recovery_key: Option<String>,
) -> Result<Option<String>, CommandError> {
    svc.rotate(recovery_key.as_deref())
        .await
        .map(|k| k.map(|k| k.to_string()))
}

#[tauri::command]
pub async fn account_sync(svc: S<'_>) -> Result<(), CommandError> {
    svc.sync().await
}

#[tauri::command]
pub async fn account_resolve_conflict(
    svc: S<'_>,
    key: String,
    keep_local: bool,
) -> Result<(), CommandError> {
    svc.resolve_conflict(&key, keep_local).await
}

#[tauri::command]
pub async fn account_change_password(
    svc: S<'_>,
    current: String,
    new: String,
) -> Result<(), CommandError> {
    svc.change_password(&current, &new).await
}

#[derive(Deserialize)]
#[serde(
    tag = "stage",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ResetStep {
    Request {
        email: String,
    },
    Confirm {
        email: String,
        code: String,
        new_password: String,
    },
}

#[tauri::command]
pub async fn account_reset_password(svc: S<'_>, step: ResetStep) -> Result<(), CommandError> {
    match step {
        ResetStep::Request { email } => svc.request_reset(&email).await,
        ResetStep::Confirm {
            email,
            code,
            new_password,
        } => svc.confirm_reset(&email, &code, &new_password).await,
    }
}

#[tauri::command]
pub async fn account_delete(svc: S<'_>, password: String) -> Result<(), CommandError> {
    svc.delete(&password).await
}

#[cfg(test)]
mod tests;

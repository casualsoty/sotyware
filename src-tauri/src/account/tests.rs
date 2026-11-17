//! End-to-end tests of accounts and encrypted sync (ADR-032): the real
//! account service (`server/`) runs in process on a loopback port; each
//! "device" is its own profile (secret store, state database, storage
//! accounts, credential store). Everything between them goes through the
//! service over HTTP, as ciphertext. No real service, no real provider.

use std::net::SocketAddr;
use std::sync::Arc;

use super::*;
use crate::cloud::account::MemoryAccountStore;
use crate::cloud::credentials::{CredentialStore, MemoryCredentialStore};
use crate::cloud::dashboard::{AccountInput, CredentialInput};
use crate::cloud::keyring::MemorySecretStore;
use crate::cloud::s3::ClientOptions;
use crate::cloud::test_server::TempDir;
use crate::storage::SqliteAccountState;

struct Service {
    url: String,
    state: Arc<sotyware_server::AppState>,
    dir: TempDir,
}

async fn service() -> Service {
    let dir = TempDir::new("account-server");
    let conn = sotyware_server::db::open(&dir.path("server.sqlite3")).unwrap();
    let state = sotyware_server::AppState::new(
        conn,
        sotyware_server::Config {
            cost: sotyware_server::security::Cost::TEST,
            ..sotyware_server::Config::default()
        },
    );
    let app = sotyware_server::api::router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Service {
        url: format!("http://{addr}"),
        state,
        dir,
    }
}

impl Service {
    fn code(&self, email: &str) -> String {
        let mails = sotyware_server::db::outbox(&self.state.db.lock().unwrap()).unwrap();
        let (_, _, body) = mails
            .into_iter()
            .rev()
            .find(|(t, _, b)| t == email && b.contains("code is"))
            .unwrap();
        body.split("code is ").nth(1).unwrap()[..8].to_owned()
    }

    /// The service's database file, checkpointed, as text.
    fn stored_bytes(&self) -> String {
        let c = self.state.db.lock().unwrap();
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        drop(c);
        let mut b = std::fs::read(self.dir.path("server.sqlite3")).unwrap();
        b.extend(std::fs::read(self.dir.path("server.sqlite3-wal")).unwrap_or_default());
        String::from_utf8_lossy(&b).into_owned()
    }
}

/// One device's profile.
struct Device {
    svc: Arc<AccountService>,
    accounts: Arc<CloudAccounts>,
    keys: Arc<MemoryCredentialStore>,
    secrets: Arc<MemorySecretStore>,
    _dir: TempDir,
}

fn device(service: &Service, name: &str) -> Device {
    let dir = TempDir::new(&format!("account-device-{name}"));
    let keys = Arc::new(MemoryCredentialStore::default());
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::clone(&keys) as Arc<dyn CredentialStore>,
        ClientOptions::default(),
    ));
    let secrets = Arc::new(MemorySecretStore::default());
    let svc = AccountService::new(
        Some(service.url.clone()),
        Arc::clone(&secrets) as Arc<dyn SecretStore>,
        SqliteAccountState::open(&dir.path("account.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn StateStore>),
        Arc::clone(&accounts),
    );
    Device {
        svc,
        accounts,
        keys,
        secrets,
        _dir: dir,
    }
}

const EMAIL: &str = "owner@example.org";
const PW: &str = "a long enough passphrase";
const SECRET_KEY: &str = "K003-the-storage-secret-0123456789";

impl Device {
    fn add_storage_account(&self, label: &str) -> String {
        self.accounts
            .add(
                &AccountInput {
                    label: label.into(),
                    provider: "backblaze-b2".into(),
                    region: "eu-central-003".into(),
                    endpoint: None,
                    bucket: "my-photos".into(),
                },
                CredentialInput::new("003key000000000000000001", SECRET_KEY),
            )
            .unwrap()
            .id
    }

    fn labels(&self) -> Vec<String> {
        self.accounts
            .list()
            .accounts
            .into_iter()
            .map(|a| a.label)
            .collect()
    }

    fn rename(&self, id: &str, label: &str) {
        self.accounts
            .update(
                id,
                &AccountInput {
                    label: label.into(),
                    provider: "backblaze-b2".into(),
                    region: "eu-central-003".into(),
                    endpoint: None,
                    bucket: "my-photos".into(),
                },
                None,
            )
            .unwrap();
    }
}

async fn signed_up(service: &Service, d: &Device) {
    d.svc.register(EMAIL, PW).await.unwrap();
    d.svc.verify(EMAIL, &service.code(EMAIL), PW).await.unwrap();
    assert_eq!(d.svc.sign_in(EMAIL, PW, "Desktop").await.unwrap(), "none");
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_and_credentials_reach_a_second_device_only_through_ciphertext() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    let id = a.add_storage_account("Holiday Photos");
    let rk = a.svc.set_up_sync().await.unwrap();
    assert_eq!(rk.split('-').count(), 14);
    // The user retypes it: correct, and with a typo.
    assert!(a.svc.check_recovery_key(&rk).await.unwrap());
    let mut typo: Vec<char> = rk.chars().collect();
    typo[0] = if typo[0] == 'A' { 'B' } else { 'A' };
    let typo: String = typo.into_iter().collect();
    assert!(
        a.svc.check_recovery_key(&typo).await.is_err()
            || !a.svc.check_recovery_key(&typo).await.unwrap()
    );
    let st = a.svc.status().await;
    assert_eq!((st.signed_in, st.vault_access.as_str()), (true, "enrolled"));

    // The service holds no plaintext: not the storage secret, not the
    // account's name, not the account key, not the recovery key.
    let stored = s.stored_bytes();
    assert!(
        !stored.contains(SECRET_KEY),
        "the storage secret is not readable at the service"
    );
    assert!(!stored.contains("Holiday Photos"));
    assert!(!stored.contains(&*rk.replace('-', "")));
    let ak = a.svc.account_key().unwrap().unwrap();
    assert!(!stored.contains(&*ak.to_base64()));

    // A second computer: signed in, but no access without the recovery key.
    let b = device(&s, "b");
    assert_eq!(b.svc.sign_in(EMAIL, PW, "Laptop").await.unwrap(), "pending");
    assert!(
        b.svc.sync().await.is_err(),
        "the password alone opens nothing"
    );
    assert!(b.labels().is_empty());
    let wrong = crypto::format_recovery_key(&crypto::SecretKey::generate());
    assert_eq!(
        b.svc.unlock(&wrong).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    b.svc.unlock(&rk).await.unwrap();
    assert_eq!(b.labels(), ["Holiday Photos"]);
    // Same id, and the secret arrived in this device's credential store.
    let creds = b.keys.load(&id).unwrap();
    assert_eq!(creds.secret_access_key.expose(), SECRET_KEY);
    assert!(
        b.secrets
            .entries
            .lock()
            .unwrap()
            .contains_key("account-key")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_edits_keep_both_values_and_the_user_chooses() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    let id = a.add_storage_account("Photos");
    let rk = a.svc.set_up_sync().await.unwrap();
    let b = device(&s, "b");
    b.svc.sign_in(EMAIL, PW, "Laptop").await.unwrap();
    b.svc.unlock(&rk).await.unwrap();
    // Both rename while offline from each other.
    a.rename(&id, "Photos (desktop)");
    b.rename(&id, "Photos (laptop)");
    a.svc.sync().await.unwrap();
    b.svc.sync().await.unwrap();
    // The first writer's value stays; the other is kept as a conflict copy.
    assert_eq!(b.labels(), ["Photos (desktop)"]);
    let st = b.svc.status().await;
    assert_eq!(st.conflicts.len(), 1, "{st:?}");
    assert_eq!(st.conflicts[0].of, format!("storage-account:{id}"));
    assert!(st.conflicts[0].summary.contains("Photos (laptop)"));
    assert!(
        !format!("{st:?}").contains(SECRET_KEY),
        "no secret in the status"
    );
    // The user keeps the laptop's name: it wins everywhere.
    b.svc
        .resolve_conflict(&st.conflicts[0].key, true)
        .await
        .unwrap();
    assert_eq!(b.labels(), ["Photos (laptop)"]);
    a.svc.sync().await.unwrap();
    assert_eq!(a.labels(), ["Photos (laptop)"]);
    assert!(a.svc.status().await.conflicts.is_empty());
    // A stale device that changed nothing never overwrites newer values.
    let c_label = a.labels();
    b.svc.sync().await.unwrap();
    assert_eq!(a.labels(), c_label);
}

#[tokio::test(flavor = "multi_thread")]
async fn deletions_propagate_and_remote_data_is_never_touched() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    let keep = a.add_storage_account("Keep");
    let gone = a.add_storage_account("Gone");
    let rk = a.svc.set_up_sync().await.unwrap();
    let b = device(&s, "b");
    b.svc.sign_in(EMAIL, PW, "Laptop").await.unwrap();
    b.svc.unlock(&rk).await.unwrap();
    assert_eq!(b.labels().len(), 2);
    a.accounts.remove(&gone).unwrap();
    a.svc.sync().await.unwrap();
    b.svc.sync().await.unwrap();
    assert_eq!(b.labels(), ["Keep"]);
    assert!(
        b.keys.load(&gone).is_err(),
        "its local key copy is removed too"
    );
    assert!(b.keys.load(&keep).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_device_is_approved_after_comparing_fingerprints() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    a.add_storage_account("Photos");
    a.svc.set_up_sync().await.unwrap();
    let c = device(&s, "c");
    assert_eq!(
        c.svc
            .sign_in(EMAIL, PW, "Phone-sized laptop")
            .await
            .unwrap(),
        "pending"
    );
    let (req, fp_new) = c.svc.request_approval().await.unwrap();
    let list = a.svc.enrollments().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0]["fingerprint"], fp_new,
        "both screens show the same digits"
    );
    assert_eq!(c.svc.approval_status(&req).await.unwrap(), "pending");
    a.svc.answer_enrollment(&req, true).await.unwrap();
    assert_eq!(c.svc.approval_status(&req).await.unwrap(), "approved");
    assert_eq!(c.labels(), ["Photos"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_device_is_cut_off_and_rotation_moves_the_others_to_a_new_key() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    a.add_storage_account("Photos");
    let rk = a.svc.set_up_sync().await.unwrap();
    let b = device(&s, "b");
    b.svc.sign_in(EMAIL, PW, "Laptop").await.unwrap();
    b.svc.unlock(&rk).await.unwrap();
    let thief = device(&s, "thief");
    thief.svc.sign_in(EMAIL, PW, "Stolen").await.unwrap();
    thief.svc.unlock(&rk).await.unwrap();
    let stolen_id = thief.svc.local().unwrap().unwrap().device_id;
    let old_key = a.svc.account_key().unwrap().unwrap();

    a.svc.revoke(&stolen_id).await.unwrap();
    assert!(
        thief.svc.sync().await.is_err(),
        "the revoked device's session ended"
    );
    // Rotation with the existing recovery key.
    assert!(a.svc.rotate(Some(&rk)).await.unwrap().is_none());
    let new_key = a.svc.account_key().unwrap().unwrap();
    assert_ne!(new_key.id(), old_key.id());
    // The laptop picks up the new key from its new wrap and keeps syncing.
    a.add_storage_account("After rotation");
    a.svc.sync().await.unwrap();
    b.svc.sync().await.unwrap();
    assert_eq!(b.svc.account_key().unwrap().unwrap().id(), new_key.id());
    assert!(b.labels().contains(&"After rotation".to_owned()));
    // The old key no longer opens the vault.
    let (_, v) = a.svc.api("GET", "/v1/vault", None).await.unwrap();
    let acct = a.svc.local().unwrap().unwrap().account_id;
    assert!(crypto::decrypt_vault(&old_key, &acct, v["envelope"].as_str().unwrap()).is_err());
    // The recovery key still works for a fresh device after the rotation.
    let d = device(&s, "d");
    d.svc.sign_in(EMAIL, PW, "Replacement PC").await.unwrap();
    d.svc.unlock(&rk).await.unwrap();
    assert!(d.labels().contains(&"After rotation".to_owned()));
    // A rotation with a new recovery key retires the old one.
    let new_rk = a.svc.rotate(None).await.unwrap().unwrap();
    let e = device(&s, "e");
    e.svc.sign_in(EMAIL, PW, "Another").await.unwrap();
    assert!(e.svc.unlock(&rk).await.is_err());
    e.svc.unlock(&new_rk).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_vault_served_again_is_refused() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    a.add_storage_account("One");
    a.svc.set_up_sync().await.unwrap();
    let account_id = a.svc.local().unwrap().unwrap().account_id;
    let (old_env, old_rev) = {
        let c = s.state.db.lock().unwrap();
        let acc = sotyware_server::db::account_by_id(&c, &account_id)
            .unwrap()
            .unwrap();
        (acc.vault.unwrap(), acc.vault_revision)
    };
    a.add_storage_account("Two");
    a.svc.sync().await.unwrap();
    // A malicious or restored service serves the older vault again.
    s.state
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE accounts SET vault = ?2, vault_revision = ?3 WHERE id = ?1",
            rusqlite::params![account_id, old_env, old_rev],
        )
        .unwrap();
    let e = a.svc.sync().await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Integrity);
    assert!(e.message.contains("older"), "{}", e.message);
    assert_eq!(a.labels().len(), 2, "nothing local was changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_password_reset_only_the_recovery_key_unlocks_again() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    a.add_storage_account("Photos");
    let rk = a.svc.set_up_sync().await.unwrap();
    a.svc.request_reset(EMAIL).await.unwrap();
    a.svc
        .confirm_reset(EMAIL, &s.code(EMAIL), "a brand new passphrase")
        .await
        .unwrap();
    assert!(a.svc.sync().await.is_err());
    assert_eq!(
        a.svc
            .sign_in(EMAIL, "a brand new passphrase", "Desktop")
            .await
            .unwrap(),
        "pending"
    );
    a.svc.unlock(&rk).await.unwrap();
    a.svc.sync().await.unwrap();
    assert_eq!(a.labels(), ["Photos"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_out_and_deleting_the_account_keep_local_storage_accounts() {
    let s = service().await;
    let a = device(&s, "a");
    signed_up(&s, &a).await;
    a.add_storage_account("Photos");
    a.svc.set_up_sync().await.unwrap();
    a.svc.sign_out().await.unwrap();
    assert!(!a.svc.status().await.signed_in);
    assert!(
        !a.secrets
            .entries
            .lock()
            .unwrap()
            .contains_key("account-key")
    );
    assert_eq!(a.labels(), ["Photos"], "local configuration stays");
    a.svc.sign_in(EMAIL, PW, "Desktop").await.unwrap();
    a.svc.delete(PW).await.unwrap();
    assert!(!a.svc.status().await.signed_in);
    assert_eq!(a.labels(), ["Photos"]);
    assert!(a.svc.sign_in(EMAIL, PW, "Desktop").await.is_err());
}

#[tokio::test]
async fn without_a_configured_service_nothing_is_sent() {
    let dir = TempDir::new("account-none");
    let svc = AccountService::new(
        None,
        Arc::new(MemorySecretStore::default()),
        SqliteAccountState::open(&dir.path("a.sqlite3"))
            .map(|s| Box::new(s) as Box<dyn StateStore>),
        Arc::new(CloudAccounts::load(
            Ok(Box::new(MemoryAccountStore::default())),
            Arc::new(MemoryCredentialStore::default()),
            ClientOptions::default(),
        )),
    );
    let st = svc.status().await;
    assert!(!st.configured && !st.signed_in);
    let e = svc.register(EMAIL, PW).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidConfiguration);
    assert!(e.message.contains("not available in this build"));
}

#[test]
fn the_production_service_is_an_https_url_the_client_accepts() {
    let url = PRODUCTION_SERVER.expect("release builds have a service");
    assert_eq!(client::check_url(url).as_deref(), Ok(url));
    assert!(url.starts_with("https://"));
}

#[cfg(feature = "test-profile")]
#[test]
fn a_test_profile_build_never_uses_the_production_service() {
    if std::env::var_os("SOTYWARE_ACCOUNT_SERVER").is_none() {
        assert_eq!(configured_server(), None);
    }
}

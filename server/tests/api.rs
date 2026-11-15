//! The account service over HTTP (in process, through the router), with a
//! fresh SQLite database per test and a controllable clock. Wraps, proofs
//! and the vault are opaque strings here: the service never interprets
//! them (the client's cryptography is tested in the desktop app).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sotyware_server::security::Cost;
use sotyware_server::{AppState, Config, api, db};
use tower::ServiceExt;

struct Server {
    app: axum::Router,
    state: Arc<AppState>,
    clock: Arc<AtomicI64>,
    dir: std::path::PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn server(name: &str) -> Server {
    let dir = std::env::temp_dir().join(format!(
        "sotyware-server-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let conn = db::open(&dir.join("db.sqlite3")).unwrap();
    let clock = Arc::new(AtomicI64::new(1_800_000_000_000));
    let c = Arc::clone(&clock);
    let state = AppState::new(
        conn,
        Config {
            cost: Cost::TEST,
            ..Config::default()
        },
    )
    .with_clock(move || c.load(Ordering::SeqCst));
    Server {
        app: api::router(Arc::clone(&state)),
        state,
        clock,
        dir,
    }
}

const KEY_A: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const KEY_B: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";

impl Server {
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.call_from(method, path, token, body, "203.0.113.7")
            .await
    }

    async fn call_from(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
        ip: &str,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let mut req = req
            .body(Body::from(body.map_or_else(String::new, |b| b.to_string())))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(
            format!("{ip}:5555").parse().unwrap(),
        ));
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn mails(&self) -> Vec<(String, String, String)> {
        db::outbox(&self.state.db.lock().unwrap()).unwrap()
    }

    fn last_code(&self, to: &str) -> String {
        let (_, _, body) = self
            .mails()
            .into_iter()
            .rev()
            .find(|(t, _, b)| t == to && b.contains("code is"))
            .unwrap();
        body.split("code is ").nth(1).unwrap()[..8].to_owned()
    }

    async fn register_verified(&self, email: &str, password: &str) {
        let (s, _) = self
            .call(
                "POST",
                "/v1/accounts",
                None,
                Some(json!({ "email": email, "password": password })),
            )
            .await;
        assert_eq!(s, StatusCode::ACCEPTED);
        let code = self.last_code(email);
        let (s, b) = self
            .call(
                "POST",
                "/v1/accounts/verify",
                None,
                Some(json!({ "email": email, "code": code, "password": password })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }

    async fn sign_in(&self, email: &str, password: &str, name: &str, key: &str) -> Value {
        let (s, b) = self
            .call("POST", "/v1/sessions", None, Some(json!({ "email": email, "password": password, "device": { "name": name, "platform": "windows", "publicKey": key } })))
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }
}

fn tok(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap().to_owned()
}

const PW: &str = "a long enough passphrase";

#[tokio::test]
async fn health_answers() {
    let s = server("health");
    let (st, b) = s.call("GET", "/healthz", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["database"], "ok");
}

#[tokio::test]
async fn registration_never_reveals_whether_an_address_exists() {
    let s = server("register");
    let (a, ba) = s
        .call(
            "POST",
            "/v1/accounts",
            None,
            Some(json!({ "email": "Me@Example.org", "password": PW })),
        )
        .await;
    assert_eq!(
        (a, ba.clone()),
        (
            StatusCode::ACCEPTED,
            json!({ "status": "check-your-email" })
        )
    );
    let code = s.last_code("me@example.org");
    s.call(
        "POST",
        "/v1/accounts/verify",
        None,
        Some(json!({ "email": "me@example.org", "code": code, "password": PW })),
    )
    .await;
    // The same answer for an address that already has an account; its owner
    // gets a notice instead of a code.
    let (b, bb) = s
        .call(
            "POST",
            "/v1/accounts",
            None,
            Some(json!({ "email": "me@example.org", "password": "another long passphrase" })),
        )
        .await;
    assert_eq!((b, bb), (a, ba));
    assert!(s.mails().last().unwrap().1.contains("Someone tried"));
    let (st, e) = s
        .call(
            "POST",
            "/v1/accounts",
            None,
            Some(json!({ "email": "x@y.org", "password": "short" })),
        )
        .await;
    assert_eq!(
        (st, e["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid-input"))
    );
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts",
            None,
            Some(json!({ "email": "not-an-email", "password": PW })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn verification_codes_are_single_use_limited_and_need_the_password() {
    let s = server("verify");
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "a@b.org", "password": PW })),
    )
    .await;
    let code = s.last_code("a@b.org");
    let wrong = if code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts/verify",
            None,
            Some(json!({ "email": "a@b.org", "code": code, "password": "wrong but long enough" })),
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    for _ in 0..5 {
        let (st, _) = s
            .call(
                "POST",
                "/v1/accounts/verify",
                None,
                Some(json!({ "email": "a@b.org", "code": wrong, "password": PW })),
            )
            .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }
    // Exhausted: even the right code no longer works.
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts/verify",
            None,
            Some(json!({ "email": "a@b.org", "code": code, "password": PW })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Expired codes too.
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "a@b.org", "password": PW })),
    )
    .await;
    let code = s.last_code("a@b.org");
    s.advance(16 * 60_000);
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts/verify",
            None,
            Some(json!({ "email": "a@b.org", "code": code, "password": PW })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_stranger_who_registered_an_address_first_cannot_keep_it() {
    let s = server("hijack");
    let attacker = "attacker passphrase 123";
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "victim@example.org", "password": attacker })),
    )
    .await;
    // The owner registers too and verifies with the code sent to them.
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "victim@example.org", "password": PW })),
    )
    .await;
    let code = s.last_code("victim@example.org");
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts/verify",
            None,
            Some(json!({ "email": "victim@example.org", "code": code, "password": attacker })),
        )
        .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "the attacker's password no longer counts"
    );
    let (st, _) = s
        .call(
            "POST",
            "/v1/accounts/verify",
            None,
            Some(json!({ "email": "victim@example.org", "code": code, "password": PW })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = s.call("POST", "/v1/sessions", None, Some(json!({ "email": "victim@example.org", "password": attacker, "device": { "name": "x", "platform": "linux", "publicKey": KEY_A } }))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sign_in_failures_look_the_same_for_known_and_unknown_addresses() {
    let s = server("signin");
    s.register_verified("known@example.org", PW).await;
    let attempt = |email: &'static str, pw: &'static str| {
        let s = &s;
        async move {
            s.call("POST", "/v1/sessions", None, Some(json!({ "email": email, "password": pw, "device": { "name": "laptop", "platform": "windows", "publicKey": KEY_A } }))).await
        }
    };
    let known = attempt("known@example.org", "wrong password here").await;
    let unknown = attempt("nobody@example.org", "wrong password here").await;
    assert_eq!(known, unknown);
    assert_eq!(known.0, StatusCode::UNAUTHORIZED);
    // After repeated failures both are refused the same way.
    for _ in 0..4 {
        attempt("known@example.org", "wrong password here").await;
        attempt("nobody@example.org", "wrong password here").await;
    }
    let k = attempt("known@example.org", PW).await;
    let u = attempt("nobody@example.org", PW).await;
    assert_eq!(k.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(k, u);
    let a = db::account_by_email(&s.state.db.lock().unwrap(), "known@example.org")
        .unwrap()
        .unwrap();
    assert!(a.failed_logins >= 5 && a.locked_until_ms > 0);
}

#[tokio::test]
async fn an_unverified_account_cannot_sign_in() {
    let s = server("unverified");
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "new@example.org", "password": PW })),
    )
    .await;
    let (st, b) = s.call("POST", "/v1/sessions", None, Some(json!({ "email": "new@example.org", "password": PW, "device": { "name": "d", "platform": "windows", "publicKey": KEY_A } }))).await;
    assert_eq!(
        (st, b["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("email-not-verified"))
    );
}

#[tokio::test]
async fn refresh_tokens_rotate_and_a_reused_one_ends_the_session_family() {
    let s = server("refresh");
    s.register_verified("r@example.org", PW).await;
    let t = s.sign_in("r@example.org", PW, "laptop", KEY_A).await;
    assert_eq!(t["vaultAccess"], "none");
    let (st, t2) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&t, "refreshToken") })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_ne!(tok(&t2, "refreshToken"), tok(&t, "refreshToken"));
    let (st, _) = s
        .call("GET", "/v1/account", Some(&tok(&t2, "accessToken")), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    // The old refresh token again (stolen copy): the whole family ends.
    let (st, b) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&t, "refreshToken") })),
        )
        .await;
    assert_eq!(
        (st, b["error"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("session-revoked"))
    );
    let (st, _) = s
        .call("GET", "/v1/account", Some(&tok(&t2, "accessToken")), None)
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&t2, "refreshToken") })),
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn access_tokens_expire_and_sign_out_ends_the_session() {
    let s = server("expiry");
    s.register_verified("e@example.org", PW).await;
    let t = s.sign_in("e@example.org", PW, "laptop", KEY_A).await;
    let (st, _) = s.call("GET", "/v1/account", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "no token");
    s.advance(16 * 60_000);
    let (st, _) = s
        .call("GET", "/v1/account", Some(&tok(&t, "accessToken")), None)
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, t2) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&t, "refreshToken") })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = s
        .call(
            "DELETE",
            "/v1/sessions/current",
            Some(&tok(&t2, "accessToken")),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = s
        .call("GET", "/v1/account", Some(&tok(&t2, "accessToken")), None)
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

/// The first device sets up sync and writes the vault; a second device gets
/// the account key with the recovery key (here: knows the proof) and joins;
/// stale writes are refused.
#[tokio::test]
async fn sync_setup_enrollment_by_recovery_and_optimistic_concurrency() {
    let s = server("sync");
    s.register_verified("s@example.org", PW).await;
    let a = s.sign_in("s@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    let (st, b) = s.call("GET", "/v1/vault", Some(&at), None).await;
    assert_eq!(
        (st, b["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("not-enrolled"))
    );
    let (st, _) = s
        .call(
            "PUT",
            "/v1/recovery",
            Some(&at),
            Some(json!({ "wrap": "recovery-wrap-ciphertext", "newKeyProof": "proof-1" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = s
        .call(
            "PUT",
            "/v1/devices/current/wrap",
            Some(&at),
            Some(json!({ "wrap": "sealed-to-a", "keyProof": "not-the-proof" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = s
        .call(
            "PUT",
            "/v1/devices/current/wrap",
            Some(&at),
            Some(json!({ "wrap": "sealed-to-a", "keyProof": "proof-1" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, b) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&at),
            Some(
                json!({ "expectedRevision": 0, "envelope": "ciphertext-1", "keyProof": "proof-1" }),
            ),
        )
        .await;
    assert_eq!((st, b["revision"].as_i64()), (StatusCode::OK, Some(1)));
    // Setting up sync again without the current proof is refused.
    let (st, _) = s
        .call(
            "PUT",
            "/v1/recovery",
            Some(&at),
            Some(json!({ "wrap": "attacker-wrap", "newKeyProof": "attacker" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let b2 = s.sign_in("s@example.org", PW, "laptop", KEY_B).await;
    assert_eq!(b2["vaultAccess"], "pending");
    let bt = tok(&b2, "accessToken");
    let (st, _) = s.call("GET", "/v1/vault", Some(&bt), None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "no vault for a device without the key"
    );
    let (st, r) = s.call("GET", "/v1/recovery", Some(&bt), None).await;
    assert_eq!(
        (st, r["wrap"].as_str()),
        (StatusCode::OK, Some("recovery-wrap-ciphertext"))
    );
    let (st, _) = s
        .call(
            "PUT",
            "/v1/devices/current/wrap",
            Some(&bt),
            Some(json!({ "wrap": "sealed-to-b", "keyProof": "proof-1" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, v) = s.call("GET", "/v1/vault", Some(&bt), None).await;
    assert_eq!(
        (st, v["envelope"].as_str(), v["revision"].as_i64()),
        (StatusCode::OK, Some("ciphertext-1"), Some(1))
    );
    // Both write from revision 1: the second is refused with the current one.
    let (st, _) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&bt),
            Some(json!({ "expectedRevision": 1, "envelope": "from-b", "keyProof": "proof-1" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, c) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&at),
            Some(
                json!({ "expectedRevision": 1, "envelope": "stale-from-a", "keyProof": "proof-1" }),
            ),
        )
        .await;
    assert_eq!(
        (st, c["revision"].as_i64()),
        (StatusCode::CONFLICT, Some(2))
    );
    let (_, v) = s.call("GET", "/v1/vault", Some(&at), None).await;
    assert_eq!(
        v["envelope"], "from-b",
        "the stale device did not overwrite"
    );
    // A write without the key proof is refused even for an enrolled device.
    let (st, _) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&at),
            Some(json!({ "expectedRevision": 2, "envelope": "x", "keyProof": "guess" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_new_device_can_be_approved_from_an_enrolled_one() {
    let s = server("approve");
    s.register_verified("p@example.org", PW).await;
    let a = s.sign_in("p@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw", "newKeyProof": "proof" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&at),
        Some(json!({ "wrap": "wa", "keyProof": "proof" })),
    )
    .await;
    let b = s.sign_in("p@example.org", PW, "new laptop", KEY_B).await;
    let bt = tok(&b, "accessToken");
    let (st, req) = s.call("POST", "/v1/enrollments", Some(&bt), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(req["publicKey"], KEY_B);
    let id = tok(&req, "id");
    // A pending device cannot list or approve requests.
    let (st, _) = s.call("GET", "/v1/enrollments", Some(&bt), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, list) = s.call("GET", "/v1/enrollments", Some(&at), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list["enrollments"][0]["deviceName"], "new laptop");
    assert_eq!(list["enrollments"][0]["publicKey"], KEY_B);
    let (st, _) = s
        .call(
            "POST",
            &format!("/v1/enrollments/{id}/approve"),
            Some(&at),
            Some(json!({ "sealedKey": "sealed-to-b", "keyProof": "wrong" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = s
        .call(
            "POST",
            &format!("/v1/enrollments/{id}/approve"),
            Some(&at),
            Some(json!({ "sealedKey": "sealed-to-b", "keyProof": "proof" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, status) = s
        .call("GET", &format!("/v1/enrollments/{id}"), Some(&bt), None)
        .await;
    assert_eq!(
        (status["state"].as_str(), status["sealedKey"].as_str()),
        (Some("approved"), Some("sealed-to-b"))
    );
    let (st, w) = s
        .call("GET", "/v1/devices/current/wrap", Some(&bt), None)
        .await;
    assert_eq!(
        (st, w["wrap"].as_str()),
        (StatusCode::OK, Some("sealed-to-b"))
    );
    // Requests expire.
    let c = s
        .sign_in(
            "p@example.org",
            PW,
            "phone",
            "MDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk8=",
        )
        .await;
    let ct = tok(&c, "accessToken");
    let (_, r2) = s.call("POST", "/v1/enrollments", Some(&ct), None).await;
    s.advance(16 * 60_000);
    let (_, refreshed) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&c, "refreshToken") })),
        )
        .await;
    let (_, st2) = s
        .call(
            "GET",
            &format!("/v1/enrollments/{}", tok(&r2, "id")),
            Some(&tok(&refreshed, "accessToken")),
            None,
        )
        .await;
    assert_eq!(st2["state"], "expired");
}

#[tokio::test]
async fn a_revoked_device_loses_its_sessions_and_its_key_copy() {
    let s = server("revoke");
    s.register_verified("v@example.org", PW).await;
    let a = s.sign_in("v@example.org", PW, "desktop", KEY_A).await;
    let b = s.sign_in("v@example.org", PW, "stolen laptop", KEY_B).await;
    let at = tok(&a, "accessToken");
    let (_, list) = s.call("GET", "/v1/devices", Some(&at), None).await;
    assert_eq!(list["devices"].as_array().unwrap().len(), 2);
    let (st, _) = s
        .call(
            "DELETE",
            &format!("/v1/devices/{}", tok(&b, "deviceId")),
            Some(&at),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = s
        .call("GET", "/v1/account", Some(&tok(&b, "accessToken")), None)
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&b, "refreshToken") })),
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let d = db::device(&s.state.db.lock().unwrap(), &tok(&b, "deviceId"))
        .unwrap()
        .unwrap();
    assert_eq!((d.state.as_str(), d.wrap), ("revoked", None));
}

#[tokio::test]
async fn after_a_rotation_other_enrolled_devices_get_the_new_wrap_with_the_new_proof() {
    let s = server("rewrap");
    s.register_verified("w@example.org", PW).await;
    let a = s.sign_in("w@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw1", "newKeyProof": "p1" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&at),
        Some(json!({ "wrap": "wa1", "keyProof": "p1" })),
    )
    .await;
    let b = s.sign_in("w@example.org", PW, "laptop", KEY_B).await;
    let bt = tok(&b, "accessToken");
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&bt),
        Some(json!({ "wrap": "wb1", "keyProof": "p1" })),
    )
    .await;
    // A rotates: new recovery wrap and proof, then B's new wrap.
    let (st, _) = s
        .call(
            "PUT",
            "/v1/recovery",
            Some(&at),
            Some(json!({ "wrap": "rw2", "keyProof": "p1", "newKeyProof": "p2" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let path = format!("/v1/devices/{}/wrap", tok(&b, "deviceId"));
    let (st, _) = s
        .call(
            "PUT",
            &path,
            Some(&at),
            Some(json!({ "wrap": "wb2", "keyProof": "p1" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "the old proof no longer counts");
    let (st, _) = s
        .call(
            "PUT",
            &path,
            Some(&at),
            Some(json!({ "wrap": "wb2", "keyProof": "p2" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, w) = s
        .call("GET", "/v1/devices/current/wrap", Some(&bt), None)
        .await;
    assert_eq!(w["wrap"], "wb2");
}

#[tokio::test]
async fn a_key_rotation_changes_everything_at_once_or_nothing() {
    let s = server("rotate");
    s.register_verified("k@example.org", PW).await;
    let a = s.sign_in("k@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw1", "newKeyProof": "p1" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&at),
        Some(json!({ "wrap": "wa1", "keyProof": "p1" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/vault",
        Some(&at),
        Some(json!({ "expectedRevision": 0, "envelope": "v1", "keyProof": "p1" })),
    )
    .await;
    let b = s.sign_in("k@example.org", PW, "laptop", KEY_B).await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&tok(&b, "accessToken")),
        Some(json!({ "wrap": "wb1", "keyProof": "p1" })),
    )
    .await;
    let a_id = tok(&a, "deviceId");
    // A stale revision: nothing changes (old proof still valid).
    let (st, _) = s
        .call(
            "POST",
            "/v1/keys/rotate",
            Some(&at),
            Some(json!({
                "recoveryWrap": "rw2", "keyProof": "p1", "newKeyProof": "p2", "expectedRevision": 0,
                "envelope": "v2", "deviceWraps": [{ "deviceId": a_id, "wrap": "wa2" }]
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (_, r) = s.call("GET", "/v1/recovery", Some(&at), None).await;
    assert_eq!(r["wrap"], "rw1");
    // The real rotation: B is not listed (revoked meanwhile, say) and loses
    // its wrap; A's and the recovery wrap and vault change together.
    let (st, body) = s
        .call(
            "POST",
            "/v1/keys/rotate",
            Some(&at),
            Some(json!({
                "recoveryWrap": "rw2", "keyProof": "p1", "newKeyProof": "p2", "expectedRevision": 1,
                "envelope": "v2", "deviceWraps": [{ "deviceId": a_id, "wrap": "wa2" }]
            })),
        )
        .await;
    assert_eq!((st, body["revision"].as_i64()), (StatusCode::OK, Some(2)));
    let (_, r) = s.call("GET", "/v1/recovery", Some(&at), None).await;
    assert_eq!(r["wrap"], "rw2");
    let (_, w) = s
        .call("GET", "/v1/devices/current/wrap", Some(&at), None)
        .await;
    assert_eq!(w["wrap"], "wa2");
    let (st, _) = s
        .call("GET", "/v1/vault", Some(&tok(&b, "accessToken")), None)
        .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "an unlisted device must enroll again"
    );
    let (st, _) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&at),
            Some(json!({ "expectedRevision": 2, "envelope": "x", "keyProof": "p1" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "the old proof is retired");
}

#[tokio::test]
async fn a_password_change_signs_out_the_other_devices() {
    let s = server("pwchange");
    s.register_verified("c@example.org", PW).await;
    let a = s.sign_in("c@example.org", PW, "desktop", KEY_A).await;
    let b = s.sign_in("c@example.org", PW, "laptop", KEY_B).await;
    let (st, _) = s.call("POST", "/v1/account/password", Some(&tok(&a, "accessToken")), Some(json!({ "currentPassword": "wrong one entirely", "newPassword": "a new long passphrase" }))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = s
        .call(
            "POST",
            "/v1/account/password",
            Some(&tok(&a, "accessToken")),
            Some(json!({ "currentPassword": PW, "newPassword": "a new long passphrase" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(
        s.call("GET", "/v1/account", Some(&tok(&a, "accessToken")), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        s.call("GET", "/v1/account", Some(&tok(&b, "accessToken")), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (st, _) = s.call("POST", "/v1/sessions", None, Some(json!({ "email": "c@example.org", "password": PW, "device": { "name": "x", "platform": "linux", "publicKey": KEY_A } }))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_password_reset_keeps_the_vault_but_every_device_must_unlock_again() {
    let s = server("reset");
    s.register_verified("z@example.org", PW).await;
    let a = s.sign_in("z@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw", "newKeyProof": "proof" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&at),
        Some(json!({ "wrap": "wa", "keyProof": "proof" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/vault",
        Some(&at),
        Some(json!({ "expectedRevision": 0, "envelope": "ct", "keyProof": "proof" })),
    )
    .await;
    let (st, b) = s
        .call(
            "POST",
            "/v1/password-reset/request",
            None,
            Some(json!({ "email": "z@example.org" })),
        )
        .await;
    let (st2, b2) = s
        .call(
            "POST",
            "/v1/password-reset/request",
            None,
            Some(json!({ "email": "unknown@example.org" })),
        )
        .await;
    assert_eq!((st, b), (st2, b2), "no difference for an unknown address");
    let code = s.last_code("z@example.org");
    let (st, _) = s.call("POST", "/v1/password-reset/confirm", None, Some(json!({ "email": "z@example.org", "code": code, "newPassword": "the new long passphrase" }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        s.call("GET", "/v1/account", Some(&at), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let n = s
        .sign_in("z@example.org", "the new long passphrase", "desktop", KEY_A)
        .await;
    assert_eq!(
        n["vaultAccess"], "pending",
        "the reset removed every device's key copy"
    );
    let nt = tok(&n, "accessToken");
    let (_, r) = s.call("GET", "/v1/recovery", Some(&nt), None).await;
    assert_eq!(
        r["wrap"], "rw",
        "the encrypted vault and recovery wrap remain"
    );
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&nt),
        Some(json!({ "wrap": "wa2", "keyProof": "proof" })),
    )
    .await;
    let (_, v) = s.call("GET", "/v1/vault", Some(&nt), None).await;
    assert_eq!(v["envelope"], "ct");
}

#[tokio::test]
async fn deleting_an_account_removes_everything_at_once() {
    let s = server("delete");
    s.register_verified("d@example.org", PW).await;
    let a = s.sign_in("d@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw", "newKeyProof": "proof" })),
    )
    .await;
    let (st, _) = s
        .call(
            "DELETE",
            "/v1/account",
            Some(&at),
            Some(json!({ "password": "not the password!!" })),
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = s
        .call(
            "DELETE",
            "/v1/account",
            Some(&at),
            Some(json!({ "password": PW })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let c = s.state.db.lock().unwrap();
    assert!(db::account_by_email(&c, "d@example.org").unwrap().is_none());
    let n: i64 = c
        .query_row("SELECT count(*) FROM devices", [], |r| r.get(0))
        .unwrap();
    let m: i64 = c
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!((n, m), (0, 0));
}

#[tokio::test]
async fn no_password_token_or_code_is_stored_in_plaintext() {
    let s = server("plaintext");
    s.call(
        "POST",
        "/v1/accounts",
        None,
        Some(json!({ "email": "p@example.org", "password": PW })),
    )
    .await;
    let code = s.last_code("p@example.org");
    s.call(
        "POST",
        "/v1/accounts/verify",
        None,
        Some(json!({ "email": "p@example.org", "code": code, "password": PW })),
    )
    .await;
    let t = s.sign_in("p@example.org", PW, "desktop", KEY_A).await;
    let (_, t2) = s
        .call(
            "POST",
            "/v1/sessions/refresh",
            None,
            Some(json!({ "refreshToken": tok(&t, "refreshToken") })),
        )
        .await;
    // Everything in the database except the development outbox (mails
    // contain codes by nature; production sends them instead).
    let c = s.state.db.lock().unwrap();
    c.execute("DELETE FROM outbox", []).unwrap();
    c.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(c);
    let mut bytes = std::fs::read(s.dir.join("db.sqlite3")).unwrap();
    bytes.extend(std::fs::read(s.dir.join("db.sqlite3-wal")).unwrap_or_default());
    let text = String::from_utf8_lossy(&bytes);
    for secret in [
        PW.to_owned(),
        tok(&t, "accessToken"),
        tok(&t, "refreshToken"),
        tok(&t2, "accessToken"),
        tok(&t2, "refreshToken"),
        code,
    ] {
        assert!(
            !text.contains(&secret),
            "found a secret in the database: {} chars",
            secret.len()
        );
    }
}

#[tokio::test]
async fn oversized_vaults_are_refused_and_events_carry_no_secrets() {
    let s = server("limits");
    s.register_verified("l@example.org", PW).await;
    let a = s.sign_in("l@example.org", PW, "desktop", KEY_A).await;
    let at = tok(&a, "accessToken");
    s.call(
        "PUT",
        "/v1/recovery",
        Some(&at),
        Some(json!({ "wrap": "rw", "newKeyProof": "proof" })),
    )
    .await;
    s.call(
        "PUT",
        "/v1/devices/current/wrap",
        Some(&at),
        Some(json!({ "wrap": "wa", "keyProof": "proof" })),
    )
    .await;
    let big = "x".repeat(sotyware_server::MAX_VAULT + 1);
    let (st, _) = s
        .call(
            "PUT",
            "/v1/vault",
            Some(&at),
            Some(json!({ "expectedRevision": 0, "envelope": big, "keyProof": "proof" })),
        )
        .await;
    assert!(
        st == StatusCode::BAD_REQUEST || st == StatusCode::PAYLOAD_TOO_LARGE,
        "{st}"
    );
    let (_, ev) = s.call("GET", "/v1/account/events", Some(&at), None).await;
    let names: Vec<&str> = ev["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"sign-in")
            && names.contains(&"sync-set-up")
            && names.contains(&"device-enrolled"),
        "{names:?}"
    );
    let all = ev.to_string();
    assert!(!all.contains(PW) && !all.contains("proof"));
}

#[test]
fn the_database_is_backed_up_online_and_restored() {
    let dir = std::env::temp_dir().join(format!("sotyware-server-backup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("live.sqlite3");
    {
        let c = db::open(&path).unwrap();
        db::insert_account(&c, "acc1", "b@example.org", "$argon2id$hash", 1).unwrap();
        db::set_recovery(&c, "acc1", "wrap", "proofhash").unwrap();
        db::put_vault(&c, "acc1", 0, "ciphertext", "dev", 2)
            .unwrap()
            .unwrap();
        db::backup(&c, &dir.join("backup.sqlite3")).unwrap();
        assert!(
            db::backup(&c, &dir.join("backup.sqlite3")).is_err(),
            "never overwrites a backup"
        );
    }
    // Disaster: the live database is lost.
    std::fs::remove_file(&path).unwrap();
    let _ = std::fs::remove_file(dir.join("live.sqlite3-wal"));
    let _ = std::fs::remove_file(dir.join("live.sqlite3-shm"));
    db::restore(&dir.join("backup.sqlite3"), &path, 3).unwrap();
    let c = db::open(&path).unwrap();
    let a = db::account_by_id(&c, "acc1").unwrap().unwrap();
    assert_eq!(
        (
            a.vault.as_deref(),
            a.vault_revision,
            a.recovery_wrap.as_deref()
        ),
        (Some("ciphertext"), 1, Some("wrap"))
    );
    // A damaged backup is refused and the live database is left alone.
    std::fs::write(dir.join("bad.sqlite3"), b"not a database").unwrap();
    assert!(db::restore(&dir.join("bad.sqlite3"), &path, 4).is_err());
    assert!(
        db::account_by_id(&db::open(&path).unwrap(), "acc1")
            .unwrap()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn requests_are_rate_limited_per_address() {
    let s = server("rate");
    let mut last = StatusCode::OK;
    for i in 0..7 {
        let (st, _) = s
            .call_from(
                "POST",
                "/v1/accounts",
                None,
                Some(json!({ "email": format!("u{i}@example.org"), "password": PW })),
                "198.51.100.9",
            )
            .await;
        last = st;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    let (st, _) = s
        .call_from(
            "POST",
            "/v1/accounts",
            None,
            Some(json!({ "email": "other@example.org", "password": PW })),
            "198.51.100.10",
        )
        .await;
    assert_eq!(st, StatusCode::ACCEPTED, "another address is not affected");
}

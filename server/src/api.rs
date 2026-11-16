//! The versioned HTTP API (`/v1`, JSON, bearer tokens). The contract is
//! documented in docs/architecture/accounts.md; every handler here keeps
//! three rules: no response reveals whether an email is registered, no
//! secret is stored or logged in plaintext, and the vault and key wraps are
//! opaque client-side ciphertext the service never interprets.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::db;
use crate::security::{self, digest, same};

/// An error as clients see it: a stable code and sotyware's own words.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retry_after: Option<u64>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid-input", message)
    }
    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "sign in again")
    }
    fn credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid-credentials",
            "the email address or the password is not correct",
        )
    }
    fn limited(seconds: u64) -> Self {
        Self {
            retry_after: Some(seconds.max(1)),
            ..Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too-many-attempts",
                "too many attempts; wait and try again",
            )
        }
    }
    fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }
    fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }
    fn internal(e: impl std::fmt::Display) -> Self {
        eprintln!("sotyware-server: internal error: {e}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the service failed; try again later",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut r = (
            self.status,
            Json(json!({ "error": self.code, "message": self.message })),
        )
            .into_response();
        if let Some(s) = self.retry_after
            && let Ok(v) = HeaderValue::from_str(&s.to_string())
        {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
        r
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// The client's address: the socket's, or with `trust_proxy` the first
/// address of `X-Forwarded-For` (which the proxy in front must overwrite).
pub struct ClientIp(pub String);

impl FromRequestParts<Arc<AppState>> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        if state.config.trust_proxy
            && let Some(ip) = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
        {
            return Ok(Self(ip.trim().to_owned()));
        }
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map_or_else(|| "unknown".into(), |c| c.0.ip().to_string()),
        ))
    }
}

fn limit(state: &AppState, key: String, capacity: f64, per_minute: f64) -> ApiResult<()> {
    if state.limits.allow(&key, capacity, per_minute) {
        Ok(())
    } else {
        Err(ApiError::limited((60.0 / per_minute).ceil() as u64))
    }
}

/// Runs `f` with the database on the blocking pool.
async fn with_db<T: Send + 'static>(
    state: &Arc<AppState>,
    f: impl FnOnce(&rusqlite::Connection, i64) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn, (state.clock)())
    })
    .await
    .map_err(ApiError::internal)?
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/accounts", post(register))
        .route("/v1/accounts/verify", post(verify_email))
        .route("/v1/sessions", post(sign_in))
        .route("/v1/sessions/refresh", post(refresh))
        .route("/v1/sessions/current", delete(sign_out))
        .route("/v1/account", get(account).delete(delete_account))
        .route("/v1/account/password", post(change_password))
        .route("/v1/account/events", get(events))
        .route("/v1/password-reset/request", post(reset_request))
        .route("/v1/password-reset/confirm", post(reset_confirm))
        .route("/v1/recovery", get(get_recovery).put(put_recovery))
        .route("/v1/vault", get(get_vault).put(put_vault))
        .route("/v1/keys/rotate", post(rotate_keys))
        .route("/v1/devices", get(list_devices))
        .route("/v1/devices/current/wrap", get(get_wrap).put(put_wrap))
        .route("/v1/devices/{id}", delete(revoke_device))
        .route("/v1/devices/{id}/wrap", axum::routing::put(put_device_wrap))
        .route(
            "/v1/enrollments",
            post(request_enrollment).get(list_enrollments),
        )
        .route("/v1/enrollments/{id}", get(enrollment_status))
        .route("/v1/enrollments/{id}/approve", post(approve_enrollment))
        .route("/v1/enrollments/{id}/reject", post(reject_enrollment))
        .layer(axum::extract::DefaultBodyLimit::max(crate::MAX_BODY))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> ApiResult<Json<serde_json::Value>> {
    with_db(&state, |c, _| {
        let n: i64 = c.query_row("SELECT 1", [], |r| r.get(0))?;
        Ok(Json(
            json!({ "status": "ok", "database": if n == 1 { "ok" } else { "error" }, "api": "v1" }),
        ))
    })
    .await
}

// ---------------------------------------------------------------- accounts

#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
}

fn accepted() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::ACCEPTED,
        Json(json!({ "status": "check-your-email" })),
    )
}

/// Registration: always the same answer. A new address gets a verification
/// code; an unverified one gets a new code and the new password (so a
/// stranger who registered the address first cannot keep it); a verified
/// one gets a notice that someone tried.
async fn register(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<Credentials>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    limit(&state, format!("register:{ip}"), 5.0, 5.0)?;
    let email =
        security::email(&body.email).ok_or_else(|| ApiError::bad("not an email address"))?;
    if let Some(p) = security::password_problem(&body.password) {
        return Err(ApiError::bad(p));
    }
    // Hashed in every case, so the answer takes as long either way.
    let hash = security::hash_password(&body.password, state.config.cost);
    let code = security::code();
    let ttl = state.config.code_ttl_ms;
    with_db(&state, move |c, now| {
        match db::account_by_email(c, &email)? {
            Some(a) if a.verified => {
                db::queue_mail(c, now, &email, "Someone tried to create a sotyware account", "Someone tried to create a sotyware account with this address, which already has one. If it was you, sign in instead or reset your password. Otherwise you can ignore this message.")?;
                db::audit(c, now, Some(&a.id), None, "register-existing");
            }
            Some(a) => {
                db::set_password(c, &a.id, &hash)?;
                db::put_code(c, &email, "verify", &digest(&code), now + ttl)?;
                db::queue_mail(c, now, &email, "Your sotyware verification code", &format!("Your verification code is {code}. It expires in 15 minutes."))?;
                db::audit(c, now, Some(&a.id), None, "register-again");
            }
            None => {
                let id = security::id();
                db::insert_account(c, &id, &email, &hash, now)?;
                db::put_code(c, &email, "verify", &digest(&code), now + ttl)?;
                db::queue_mail(c, now, &email, "Your sotyware verification code", &format!("Your verification code is {code}. It expires in 15 minutes."))?;
                db::audit(c, now, Some(&id), None, "register");
            }
        }
        Ok(())
    })
    .await?;
    state.mail_queued.notify_one();
    Ok(accepted())
}

/// Checks a code; the same error for an unknown address, a wrong, expired
/// or exhausted code.
fn check_code(
    c: &rusqlite::Connection,
    email: &str,
    purpose: &str,
    code: &str,
    now: i64,
) -> ApiResult<()> {
    let invalid = || ApiError::bad("the code is not valid; request a new one");
    let Some((hash, expires, attempts)) = db::code(c, email, purpose)? else {
        return Err(invalid());
    };
    if expires < now || attempts >= 5 {
        db::delete_code(c, email, purpose)?;
        return Err(invalid());
    }
    if !same(&hash, &digest(code.trim())) {
        db::code_attempt(c, email, purpose)?;
        return Err(invalid());
    }
    db::delete_code(c, email, purpose)?;
    Ok(())
}

#[derive(Deserialize)]
struct Verify {
    email: String,
    code: String,
    password: String,
}

/// Activates an account; needs the code and the password, so a stranger
/// who re-registered the address meanwhile cannot hijack the owner's
/// verification.
async fn verify_email(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<Verify>,
) -> ApiResult<Json<serde_json::Value>> {
    limit(&state, format!("verify:{ip}"), 10.0, 10.0)?;
    let email =
        security::email(&body.email).ok_or_else(|| ApiError::bad("not an email address"))?;
    with_db(&state, move |c, now| {
        let Some(a) = db::account_by_email(c, &email)? else {
            return Err(ApiError::bad("the code is not valid; request a new one"));
        };
        if !security::verify_password(&body.password, &a.password_hash) {
            return Err(ApiError::credentials());
        }
        check_code(c, &email, "verify", &body.code, now)?;
        db::set_verified(c, &a.id)?;
        db::audit(c, now, Some(&a.id), None, "verified");
        Ok(Json(json!({ "status": "verified" })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInput {
    /// A device id this client got before, to keep its enrollment.
    id: Option<String>,
    name: String,
    platform: String,
    /// The device's X25519 public key, base64.
    public_key: String,
}

#[derive(Deserialize)]
struct SignIn {
    email: String,
    password: String,
    device: DeviceInput,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    access_token: String,
    refresh_token: String,
    access_expires_ms: i64,
    refresh_expires_ms: i64,
    account_id: String,
    device_id: String,
    /// `enrolled`, `pending` (sync exists, this device has no key yet) or
    /// `none` (sync not set up).
    vault_access: &'static str,
}

fn valid_key(k: &str) -> bool {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(k)
        .is_ok_and(|b| b.len() == 32)
}

fn issue(
    c: &rusqlite::Connection,
    state: &AppState,
    account: &db::Account,
    device: &db::Device,
    family: &str,
    now: i64,
) -> ApiResult<Tokens> {
    let (access, refresh) = (security::token(), security::token());
    let access_expires = now + state.config.access_ttl_ms;
    let refresh_expires = now + state.config.refresh_ttl_ms;
    db::insert_session(
        c,
        &security::id(),
        &account.id,
        &device.id,
        family,
        &digest(&access),
        access_expires,
        &digest(&refresh),
        refresh_expires,
        now,
    )?;
    Ok(Tokens {
        access_token: access,
        refresh_token: refresh,
        access_expires_ms: access_expires,
        refresh_expires_ms: refresh_expires,
        account_id: account.id.clone(),
        device_id: device.id.clone(),
        vault_access: match (device.state.as_str(), &account.key_proof_hash) {
            ("enrolled", _) => "enrolled",
            (_, Some(_)) => "pending",
            _ => "none",
        },
    })
}

async fn sign_in(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<SignIn>,
) -> ApiResult<Json<Tokens>> {
    limit(&state, format!("signin:{ip}"), 10.0, 10.0)?;
    let email = security::email(&body.email).ok_or_else(ApiError::credentials)?;
    limit(&state, format!("signin-email:{email}"), 5.0, 5.0)?;
    let d = &body.device;
    if d.name.trim().is_empty()
        || d.name.chars().count() > 64
        || d.platform.chars().count() > 32
        || !valid_key(&d.public_key)
    {
        return Err(ApiError::bad(
            "the device needs a name (≤ 64 characters), a platform and a 32-byte public key",
        ));
    }
    let state2 = Arc::clone(&state);
    let result = with_db(&state, move |c, now| {
        let account = db::account_by_email(c, &email)?;
        let locked_until = match &account {
            Some(a) => a.locked_until_ms,
            None => state2.unknown_lock(&email),
        };
        if locked_until > now {
            return Err(ApiError::limited(((locked_until - now) / 1000) as u64 + 1));
        }
        let hash = account.as_ref().map_or(state2.dummy_hash.as_str(), |a| a.password_hash.as_str());
        let ok = security::verify_password(&body.password, hash) && account.is_some();
        let Some(account) = account.filter(|_| ok) else {
            match db::account_by_email(c, &email)? {
                Some(a) => {
                    let failures = a.failed_logins + 1;
                    db::record_failure(c, &a.id, failures, now + security::lockout_ms(failures))?;
                    db::audit(c, now, Some(&a.id), None, "sign-in-failed");
                }
                None => state2.unknown_failure(&email, now),
            }
            return Err(ApiError::credentials());
        };
        if !account.verified {
            return Err(ApiError::forbidden("email-not-verified", "verify your email address first"));
        }
        db::record_failure(c, &account.id, 0, 0)?;
        let reuse = body.device.id.as_ref().and_then(|id| db::device(c, id).ok().flatten()).filter(|dev| {
            dev.account_id == account.id && dev.state != "revoked" && dev.public_key == body.device.public_key
        });
        let device = match reuse {
            Some(dev) => {
                db::touch_device(c, &dev.id, now)?;
                dev
            }
            None => {
                let dev = db::Device {
                    id: security::id(),
                    account_id: account.id.clone(),
                    name: body.device.name.trim().to_owned(),
                    platform: body.device.platform.trim().to_owned(),
                    public_key: body.device.public_key.clone(),
                    state: "pending".into(),
                    wrap: None,
                    created_ms: now,
                    last_seen_ms: now,
                };
                db::insert_device(c, &dev)?;
                db::queue_mail(c, now, &account.email, "New sign-in to your sotyware account", &format!("A device named \"{}\" ({}) signed in to your sotyware account. It cannot read your synced storage keys until you approve it from another device or enter your recovery key on it. If this was not you, change your password and revoke the device.", dev.name, dev.platform))?;
                dev
            }
        };
        db::audit(c, now, Some(&account.id), Some(&device.id), "sign-in");
        Ok(Json(issue(c, &state2, &account, &device, &security::id(), now)?))
    })
    .await;
    state.mail_queued.notify_one();
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Refresh {
    refresh_token: String,
}

/// Rotates the refresh token. A used one presented again revokes the whole
/// session family (it was stolen, or the client is broken).
async fn refresh(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<Refresh>,
) -> ApiResult<Json<Tokens>> {
    limit(&state, format!("refresh:{ip}"), 30.0, 30.0)?;
    let state2 = Arc::clone(&state);
    with_db(&state, move |c, now| {
        let Some(s) = db::session_by_refresh(c, &digest(&body.refresh_token))? else {
            return Err(ApiError::unauthorized());
        };
        if s.used || s.revoked {
            db::revoke_family(c, &s.family)?;
            db::audit(
                c,
                now,
                Some(&s.account_id),
                Some(&s.device_id),
                "refresh-token-reused",
            );
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "session-revoked",
                "this session was ended; sign in again",
            ));
        }
        if s.refresh_expires_ms < now {
            return Err(ApiError::unauthorized());
        }
        let device = db::device(c, &s.device_id)?
            .filter(|d| d.state != "revoked")
            .ok_or_else(ApiError::unauthorized)?;
        let account = db::account_by_id(c, &s.account_id)?.ok_or_else(ApiError::unauthorized)?;
        db::mark_used(c, &s.id)?;
        db::touch_device(c, &device.id, now)?;
        Ok(Json(issue(c, &state2, &account, &device, &s.family, now)?))
    })
    .await
}

/// The caller's session, account and device.
struct Caller {
    session: db::Session,
    account: db::Account,
    device: db::Device,
}

fn bearer(headers: &HeaderMap) -> ApiResult<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
        .ok_or_else(ApiError::unauthorized)
}

fn caller(c: &rusqlite::Connection, token: &str, now: i64) -> ApiResult<Caller> {
    let session = db::session_by_access(c, &digest(token))?.ok_or_else(ApiError::unauthorized)?;
    if session.revoked || session.access_expires_ms < now {
        return Err(ApiError::unauthorized());
    }
    let device = db::device(c, &session.device_id)?
        .filter(|d| d.state != "revoked")
        .ok_or_else(ApiError::unauthorized)?;
    let account = db::account_by_id(c, &session.account_id)?.ok_or_else(ApiError::unauthorized)?;
    Ok(Caller {
        session,
        account,
        device,
    })
}

async fn authed<T: Send + 'static>(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    f: impl FnOnce(&rusqlite::Connection, i64, Caller) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let token = bearer(headers)?;
    with_db(state, move |c, now| {
        let who = caller(c, &token, now)?;
        f(c, now, who)
    })
    .await
}

async fn sign_out(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<StatusCode> {
    authed(&state, &headers, |c, now, who| {
        db::revoke_family(c, &who.session.family)?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&who.device.id),
            "sign-out",
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |_, _, who| {
        let a = &who.account;
        Ok(Json(json!({
            "accountId": a.id,
            "email": a.email,
            "createdMs": a.created_ms,
            "syncSetUp": a.key_proof_hash.is_some(),
            "vault": a.vault.as_ref().map(|_| json!({
                "revision": a.vault_revision,
                "updatedMs": a.vault_updated_ms,
                "updatedBy": a.vault_updated_by,
            })),
            "device": { "id": who.device.id, "state": who.device.state },
        })))
    })
    .await
}

async fn events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |c, _, who| {
        let events: Vec<_> = db::audit_events(c, &who.account.id)?
            .into_iter()
            .rev()
            .take(100)
            .map(|(at, e)| json!({ "atMs": at, "event": e }))
            .collect();
        Ok(Json(json!({ "events": events })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordChange {
    current_password: String,
    new_password: String,
}

/// Changes the password; every other device is signed out.
async fn change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PasswordChange>,
) -> ApiResult<StatusCode> {
    if let Some(p) = security::password_problem(&body.new_password) {
        return Err(ApiError::bad(p));
    }
    let hash = security::hash_password(&body.new_password, state.config.cost);
    let result = authed(&state, &headers, move |c, now, who| {
        if !security::verify_password(&body.current_password, &who.account.password_hash) {
            return Err(ApiError::credentials());
        }
        db::set_password(c, &who.account.id, &hash)?;
        db::revoke_account_sessions(c, &who.account.id, Some(&who.device.id))?;
        db::queue_mail(c, now, &who.account.email, "Your sotyware password was changed", "The password of your sotyware account was changed, and every other device was signed out. If this was not you, reset your password now.")?;
        db::audit(c, now, Some(&who.account.id), Some(&who.device.id), "password-changed");
        Ok(StatusCode::NO_CONTENT)
    })
    .await;
    state.mail_queued.notify_one();
    result
}

#[derive(Deserialize)]
struct Password {
    password: String,
}

/// Deletes the account and everything stored with it, at once. Server
/// backups keep it until they expire (docs/development/server.md).
async fn delete_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Password>,
) -> ApiResult<StatusCode> {
    let result = authed(&state, &headers, move |c, now, who| {
        if !security::verify_password(&body.password, &who.account.password_hash) {
            return Err(ApiError::credentials());
        }
        db::delete_account(c, &who.account.id)?;
        db::queue_mail(c, now, &who.account.email, "Your sotyware account was deleted", "Your sotyware account, its devices and its encrypted vault were deleted. Storage accounts configured on your computers keep working locally.")?;
        db::audit(c, now, Some(&who.account.id), None, "account-deleted");
        Ok(StatusCode::NO_CONTENT)
    })
    .await;
    state.mail_queued.notify_one();
    result
}

#[derive(Deserialize)]
struct ResetRequest {
    email: String,
}

async fn reset_request(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<ResetRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    limit(&state, format!("reset:{ip}"), 5.0, 5.0)?;
    let Some(email) = security::email(&body.email) else {
        return Ok(accepted());
    };
    limit(&state, format!("reset-email:{email}"), 3.0, 1.0)?;
    let code = security::code();
    let ttl = state.config.code_ttl_ms;
    with_db(&state, move |c, now| {
        if let Some(a) = db::account_by_email(c, &email)?.filter(|a| a.verified) {
            db::put_code(c, &email, "reset", &digest(&code), now + ttl)?;
            db::queue_mail(c, now, &email, "Reset your sotyware password", &format!("Your password reset code is {code}. It expires in 15 minutes. Resetting the password signs out every device; each must then be unlocked again with your recovery key."))?;
            db::audit(c, now, Some(&a.id), None, "reset-requested");
        }
        Ok(())
    })
    .await?;
    state.mail_queued.notify_one();
    Ok(accepted())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetConfirm {
    email: String,
    code: String,
    new_password: String,
}

/// Sets a new password with a reset code. Every session ends and every
/// device loses its key: the vault stays encrypted and can be unlocked
/// again only with the recovery key.
async fn reset_confirm(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    Json(body): Json<ResetConfirm>,
) -> ApiResult<Json<serde_json::Value>> {
    limit(&state, format!("reset-confirm:{ip}"), 10.0, 10.0)?;
    if let Some(p) = security::password_problem(&body.new_password) {
        return Err(ApiError::bad(p));
    }
    let email = security::email(&body.email)
        .ok_or_else(|| ApiError::bad("the code is not valid; request a new one"))?;
    let hash = security::hash_password(&body.new_password, state.config.cost);
    with_db(&state, move |c, now| {
        check_code(c, &email, "reset", &body.code, now)?;
        let a = db::account_by_email(c, &email)?
            .ok_or_else(|| ApiError::bad("the code is not valid; request a new one"))?;
        db::set_password(c, &a.id, &hash)?;
        db::revoke_account_sessions(c, &a.id, None)?;
        db::unenroll_all(c, &a.id)?;
        db::audit(c, now, Some(&a.id), None, "password-reset");
        Ok(Json(json!({ "status": "password-reset" })))
    })
    .await
}

// ------------------------------------------------------------ vault, keys

/// The largest envelope or wrap accepted.
fn check_blob(what: &str, s: &str, max: usize) -> ApiResult<()> {
    if s.is_empty() || s.len() > max {
        return Err(ApiError::bad(format!(
            "the {what} must be 1 to {max} bytes"
        )));
    }
    Ok(())
}

/// The key proof must match the one recorded when sync was set up.
fn check_proof(account: &db::Account, proof: &str) -> ApiResult<()> {
    match &account.key_proof_hash {
        Some(h) if same(h, &digest(proof)) => Ok(()),
        Some(_) => Err(ApiError::forbidden(
            "wrong-key",
            "this device does not hold the account key",
        )),
        None => Err(ApiError::not_found(
            "no-sync",
            "sync is not set up for this account",
        )),
    }
}

async fn get_recovery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |_, _, who| {
        match who.account.recovery_wrap {
            Some(w) => Ok(Json(json!({ "wrap": w }))),
            None => Err(ApiError::not_found(
                "no-sync",
                "sync is not set up for this account",
            )),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryPut {
    wrap: String,
    /// Proof of the current account key (absent only for the first setup).
    key_proof: Option<String>,
    /// Proof of the account key the wrap holds (changes on rotation).
    new_key_proof: String,
}

/// Sets up sync (first device) or replaces the recovery wrap (rotation).
async fn put_recovery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RecoveryPut>,
) -> ApiResult<StatusCode> {
    check_blob("recovery wrap", &body.wrap, 4096)?;
    check_blob("key proof", &body.new_key_proof, 256)?;
    authed(&state, &headers, move |c, now, who| {
        if who.account.key_proof_hash.is_some() {
            check_proof(&who.account, body.key_proof.as_deref().unwrap_or(""))?;
        }
        let first = who.account.key_proof_hash.is_none();
        db::set_recovery(c, &who.account.id, &body.wrap, &digest(&body.new_key_proof))?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&who.device.id),
            if first { "sync-set-up" } else { "keys-rotated" },
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WrapPut {
    wrap: String,
    key_proof: String,
}

/// Stores this device's sealed copy of the account key, after it proved it
/// holds the key (unlocked with the recovery key); the device is enrolled.
async fn put_wrap(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<WrapPut>,
) -> ApiResult<StatusCode> {
    check_blob("device wrap", &body.wrap, 4096)?;
    authed(&state, &headers, move |c, now, who| {
        check_proof(&who.account, &body.key_proof)?;
        db::set_device(c, &who.device.id, "enrolled", Some(&body.wrap))?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&who.device.id),
            "device-enrolled",
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn get_wrap(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |_, _, who| {
        match (who.device.state.as_str(), who.device.wrap) {
            ("enrolled", Some(w)) => Ok(Json(json!({ "wrap": w }))),
            _ => Err(ApiError::forbidden(
                "not-enrolled",
                "this device has no key yet",
            )),
        }
    })
    .await
}

async fn get_vault(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |_, _, who| {
        if who.device.state != "enrolled" {
            return Err(ApiError::forbidden(
                "not-enrolled",
                "this device has no key yet",
            ));
        }
        match &who.account.vault {
            Some(v) => Ok(Json(json!({
                "revision": who.account.vault_revision,
                "envelope": v,
                "updatedMs": who.account.vault_updated_ms,
                "updatedBy": who.account.vault_updated_by,
            }))),
            None => Err(ApiError::not_found("no-vault", "no vault is stored yet")),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultPut {
    expected_revision: i64,
    envelope: String,
    key_proof: String,
}

/// Stores a new vault if nobody wrote since `expectedRevision`; otherwise
/// 409 with the current revision (the client merges and tries again).
async fn put_vault(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<VaultPut>,
) -> ApiResult<Response> {
    check_blob("vault", &body.envelope, crate::MAX_VAULT)?;
    authed(&state, &headers, move |c, now, who| {
        if who.device.state != "enrolled" {
            return Err(ApiError::forbidden("not-enrolled", "this device has no key yet"));
        }
        check_proof(&who.account, &body.key_proof)?;
        match db::put_vault(c, &who.account.id, body.expected_revision, &body.envelope, &who.device.id, now)? {
            Ok(revision) => {
                db::audit(c, now, Some(&who.account.id), Some(&who.device.id), "vault-written");
                Ok(Json(json!({ "revision": revision })).into_response())
            }
            Err(current) => Ok((
                StatusCode::CONFLICT,
                Json(json!({ "error": "conflict", "message": "another device changed the vault; merge and try again", "revision": current })),
            )
                .into_response()),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceWrap {
    device_id: String,
    wrap: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rotate {
    recovery_wrap: String,
    key_proof: String,
    new_key_proof: String,
    expected_revision: i64,
    envelope: String,
    device_wraps: Vec<DeviceWrap>,
}

/// Replaces the account key in one transaction: the recovery wrap, the key
/// proof, the vault (re-encrypted) and the wraps of the listed enrolled
/// devices change together or not at all, so no crash can leave them under
/// different keys. Enrolled devices not listed lose their wrap (they must
/// enroll again).
async fn rotate_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Rotate>,
) -> ApiResult<Response> {
    check_blob("recovery wrap", &body.recovery_wrap, 4096)?;
    check_blob("key proof", &body.new_key_proof, 256)?;
    check_blob("vault", &body.envelope, crate::MAX_VAULT)?;
    if body.device_wraps.len() > 1000
        || body
            .device_wraps
            .iter()
            .any(|d| d.wrap.is_empty() || d.wrap.len() > 4096)
    {
        return Err(ApiError::bad("invalid device wraps"));
    }
    authed(&state, &headers, move |c, now, who| {
        if who.device.state != "enrolled" {
            return Err(ApiError::forbidden("not-enrolled", "this device has no key"));
        }
        check_proof(&who.account, &body.key_proof)?;
        let tx = c.unchecked_transaction()?;
        let devices = db::devices(&tx, &who.account.id)?;
        for d in &body.device_wraps {
            if !devices.iter().any(|x| x.id == d.device_id && x.state == "enrolled") {
                return Err(ApiError::bad("a listed device is not an enrolled device of this account"));
            }
        }
        if let Err(current) = db::put_vault(
            &tx,
            &who.account.id,
            body.expected_revision,
            &body.envelope,
            &who.device.id,
            now,
        )? {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({ "error": "conflict", "message": "another device changed the vault; merge and try again", "revision": current })),
            )
                .into_response());
        }
        db::set_recovery(&tx, &who.account.id, &body.recovery_wrap, &digest(&body.new_key_proof))?;
        for d in devices.iter().filter(|d| d.state == "enrolled") {
            match body.device_wraps.iter().find(|w| w.device_id == d.id) {
                Some(w) => db::set_device(&tx, &d.id, "enrolled", Some(&w.wrap))?,
                None => db::set_device(&tx, &d.id, "pending", None)?,
            }
        }
        tx.commit()?;
        db::audit(c, now, Some(&who.account.id), Some(&who.device.id), "keys-rotated");
        Ok(Json(json!({ "revision": body.expected_revision + 1 })).into_response())
    })
    .await
}

// ---------------------------------------------------------------- devices

async fn list_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |c, _, who| {
        let list: Vec<_> = db::devices(c, &who.account.id)?
            .into_iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "name": d.name,
                    "platform": d.platform,
                    "publicKey": d.public_key,
                    "state": d.state,
                    "createdMs": d.created_ms,
                    "lastSeenMs": d.last_seen_ms,
                    "current": d.id == who.device.id,
                })
            })
            .collect();
        Ok(Json(json!({ "devices": list })))
    })
    .await
}

/// Stores a re-wrapped account key for another enrolled device after a key
/// rotation (the proof is the new key's).
async fn put_device_wrap(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<WrapPut>,
) -> ApiResult<StatusCode> {
    check_blob("device wrap", &body.wrap, 4096)?;
    authed(&state, &headers, move |c, now, who| {
        check_proof(&who.account, &body.key_proof)?;
        let d = db::device(c, &id)?
            .filter(|d| d.account_id == who.account.id && d.state == "enrolled")
            .ok_or_else(|| ApiError::not_found("no-device", "no such enrolled device"))?;
        db::set_device(c, &d.id, "enrolled", Some(&body.wrap))?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&d.id),
            "device-rewrapped",
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// Revokes a device: its sessions end and its key copy is deleted.
async fn revoke_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authed(&state, &headers, move |c, now, who| {
        let d = db::device(c, &id)?
            .filter(|d| d.account_id == who.account.id)
            .ok_or_else(|| ApiError::not_found("no-device", "no such device"))?;
        db::set_device(c, &d.id, "revoked", None)?;
        db::revoke_device_sessions(c, &d.id)?;
        db::audit(c, now, Some(&who.account.id), Some(&d.id), "device-revoked");
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// A pending device asks to be approved by an enrolled one.
async fn request_enrollment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let ttl = state.config.enrollment_ttl_ms;
    authed(&state, &headers, move |c, now, who| {
        if who.device.state == "enrolled" {
            return Err(ApiError::bad("this device is already enrolled"));
        }
        if who.account.key_proof_hash.is_none() {
            return Err(ApiError::not_found(
                "no-sync",
                "sync is not set up for this account",
            ));
        }
        let e = db::Enrollment {
            id: security::id(),
            account_id: who.account.id.clone(),
            device_id: who.device.id.clone(),
            state: "pending".into(),
            sealed_key: None,
            created_ms: now,
            expires_ms: now + ttl,
        };
        db::insert_enrollment(c, &e)?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&who.device.id),
            "enrollment-requested",
        );
        Ok(Json(
            json!({ "id": e.id, "expiresMs": e.expires_ms, "publicKey": who.device.public_key }),
        ))
    })
    .await
}

/// Pending requests, for an enrolled device to review.
async fn list_enrollments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, |c, now, who| {
        if who.device.state != "enrolled" {
            return Err(ApiError::forbidden(
                "not-enrolled",
                "only an enrolled device can approve another",
            ));
        }
        let mut list = Vec::new();
        for e in db::pending_enrollments(c, &who.account.id, now)? {
            if let Some(d) = db::device(c, &e.device_id)? {
                list.push(json!({
                    "id": e.id,
                    "deviceId": d.id,
                    "deviceName": d.name,
                    "platform": d.platform,
                    "publicKey": d.public_key,
                    "createdMs": e.created_ms,
                    "expiresMs": e.expires_ms,
                }));
            }
        }
        Ok(Json(json!({ "enrollments": list })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Approve {
    sealed_key: String,
    key_proof: String,
}

async fn approve_enrollment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Approve>,
) -> ApiResult<StatusCode> {
    check_blob("sealed key", &body.sealed_key, 4096)?;
    authed(&state, &headers, move |c, now, who| {
        if who.device.state != "enrolled" {
            return Err(ApiError::forbidden(
                "not-enrolled",
                "only an enrolled device can approve another",
            ));
        }
        check_proof(&who.account, &body.key_proof)?;
        let e = db::enrollment(c, &id)?
            .filter(|e| {
                e.account_id == who.account.id && e.state == "pending" && e.expires_ms > now
            })
            .ok_or_else(|| {
                ApiError::not_found("no-enrollment", "this request expired or was answered")
            })?;
        db::set_enrollment(c, &e.id, "approved", Some(&body.sealed_key))?;
        db::set_device(c, &e.device_id, "enrolled", Some(&body.sealed_key))?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&e.device_id),
            "enrollment-approved",
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn reject_enrollment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authed(&state, &headers, move |c, now, who| {
        let e = db::enrollment(c, &id)?
            .filter(|e| e.account_id == who.account.id && e.state == "pending")
            .ok_or_else(|| {
                ApiError::not_found("no-enrollment", "this request expired or was answered")
            })?;
        db::set_enrollment(c, &e.id, "rejected", None)?;
        db::audit(
            c,
            now,
            Some(&who.account.id),
            Some(&e.device_id),
            "enrollment-rejected",
        );
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// The requesting device polls its own request.
async fn enrollment_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    authed(&state, &headers, move |c, now, who| {
        let e = db::enrollment(c, &id)?
            .filter(|e| e.device_id == who.device.id)
            .ok_or_else(|| ApiError::not_found("no-enrollment", "no such request"))?;
        let state = if e.state == "pending" && e.expires_ms <= now {
            "expired"
        } else {
            e.state.as_str()
        };
        Ok(Json(json!({ "state": state, "sealedKey": e.sealed_key })))
    })
    .await
}

//! Delivery of the outbox through an SMTP relay (a transactional email
//! provider such as Brevo, Resend, Mailgun or Amazon SES, or a local mail
//! server).
//!
//! Handlers only queue mails (`db::queue_mail`), inside the transaction
//! that decided to send them, and wake the delivery task
//! (`AppState::mail_queued`). The task sends each due mail, deletes it once
//! the relay accepted it, retries a temporary failure with a growing delay
//! and gives up after [`GIVE_UP_AFTER_MS`] or on a permanent refusal.
//! Without `SOTYWARE_SMTP_HOST` nothing is sent and mails stay in the
//! outbox (development: `sotyware-server outbox` prints them).
//!
//! Log lines never contain an address, a subject or a body (they hold
//! codes): only the mail's row id and the kind of failure, never the
//! relay's reply text, which often repeats the address.

use std::sync::Arc;
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::{AppState, db};

/// A mail not sent within a day is dropped (its codes expired long ago).
pub const GIVE_UP_AFTER_MS: i64 = 24 * 3_600_000;
/// First retry delay; doubled per failed attempt up to [`MAX_RETRY_MS`].
pub const FIRST_RETRY_MS: i64 = 30_000;
pub const MAX_RETRY_MS: i64 = 15 * 60_000;
/// Even without a wake-up, the task looks at the outbox this often.
const SWEEP: Duration = Duration::from_secs(60);
/// Mails taken from the database per round.
const BATCH: i64 = 20;
/// Per SMTP command.
const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// How the connection to the relay is protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (port 465).
    Tls,
    /// Plain connection upgraded with STARTTLS, which is required: no
    /// credentials or mail are sent if the upgrade fails (port 587).
    StartTls,
    /// No encryption. Only to a relay on this machine (loopback).
    None,
}

/// The relay settings, from the environment.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// User name and password (both or neither).
    pub credentials: Option<(String, String)>,
    pub from: Mailbox,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("user", &self.credentials.as_ref().map(|(u, _)| u))
            .field("from", &self.from.to_string())
            .finish_non_exhaustive()
    }
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl SmtpConfig {
    /// Reads `SOTYWARE_SMTP_HOST`, `_PORT`, `_SECURITY` (`starttls`,
    /// `tls`, `none`), `_USERNAME`, `_PASSWORD` and `SOTYWARE_MAIL_FROM`
    /// through `var`. `Ok(None)`: no host, delivery off.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let var = |name: &str| var(name).filter(|v| !v.trim().is_empty());
        let Some(host) = var("SOTYWARE_SMTP_HOST") else {
            return Ok(None);
        };
        let host = host.trim().to_owned();
        let security = match var("SOTYWARE_SMTP_SECURITY").as_deref().map(str::trim) {
            None | Some("starttls") => Security::StartTls,
            Some("tls") => Security::Tls,
            Some("none") => Security::None,
            Some(other) => {
                return Err(format!(
                    "SOTYWARE_SMTP_SECURITY: '{other}' is not starttls, tls or none"
                ));
            }
        };
        if security == Security::None && !is_loopback(&host) {
            return Err(
                "SOTYWARE_SMTP_SECURITY=none is allowed only for a relay on this machine (localhost)"
                    .into(),
            );
        }
        let port = match var("SOTYWARE_SMTP_PORT") {
            Some(p) => p
                .trim()
                .parse()
                .map_err(|_| format!("SOTYWARE_SMTP_PORT: '{p}' is not a port number"))?,
            None => match security {
                Security::Tls => 465,
                Security::StartTls => 587,
                Security::None => 25,
            },
        };
        let credentials = match (var("SOTYWARE_SMTP_USERNAME"), var("SOTYWARE_SMTP_PASSWORD")) {
            (Some(u), Some(p)) => Some((u.trim().to_owned(), p)),
            (None, None) => None,
            _ => {
                return Err(
                    "set both SOTYWARE_SMTP_USERNAME and SOTYWARE_SMTP_PASSWORD, or neither".into(),
                );
            }
        };
        let from = var("SOTYWARE_MAIL_FROM")
            .ok_or("SOTYWARE_MAIL_FROM is required with SOTYWARE_SMTP_HOST")?
            .trim()
            .parse::<Mailbox>()
            .map_err(|e| format!("SOTYWARE_MAIL_FROM: {e}"))?;
        Ok(Some(Self {
            host,
            port,
            security,
            credentials,
            from,
        }))
    }
}

/// Why a mail was not sent, without the relay's reply text.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// Try again later: a 4xx reply, a network, TLS or timeout error.
    Temporary(String),
    /// Never deliverable: a 5xx reply or an address that cannot be used.
    Permanent(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Temporary(k) => write!(f, "temporary failure ({k})"),
            Self::Permanent(k) => write!(f, "permanent failure ({k})"),
        }
    }
}

fn classify(e: &lettre::transport::smtp::Error) -> SendError {
    if let Some(code) = e.status() {
        let kind = format!("smtp {code}");
        if e.is_permanent() {
            SendError::Permanent(kind)
        } else {
            SendError::Temporary(kind)
        }
    } else if e.is_timeout() {
        SendError::Temporary("timeout".into())
    } else if e.is_tls() {
        SendError::Temporary("tls".into())
    } else {
        SendError::Temporary("connection or protocol error".into())
    }
}

/// Sends mails through one relay. Each send opens its own connection (the
/// service sends few mails).
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl Mailer {
    pub fn new(config: &SmtpConfig) -> Result<Self, String> {
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .timeout(Some(SMTP_TIMEOUT));
        builder = match config.security {
            Security::None => builder.tls(Tls::None),
            Security::Tls | Security::StartTls => {
                let params = TlsParameters::new(config.host.clone())
                    .map_err(|e| format!("TLS for {}: {e}", config.host))?;
                builder.tls(if config.security == Security::Tls {
                    Tls::Wrapper(params)
                } else {
                    Tls::Required(params)
                })
            }
        };
        if let Some((user, password)) = &config.credentials {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from: config.from.clone(),
        })
    }

    /// Sends one plain-text mail.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), SendError> {
        let to: Mailbox = to
            .parse()
            .map_err(|_| SendError::Permanent("the address cannot be used".into()))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_owned())
            .map_err(|_| SendError::Permanent("the message cannot be built".into()))?;
        self.transport
            .send(message)
            .await
            .map(drop)
            .map_err(|e| classify(&e))
    }

    /// Sends one mail and returns the relay's full error, for the operator's
    /// `send-test-mail` command (which may print it: the address is theirs).
    pub async fn send_test(&self, to: &str) -> Result<(), String> {
        let to: Mailbox = to.parse().map_err(|e| format!("{to}: {e}"))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject("sotyware test mail")
            .header(ContentType::TEXT_PLAIN)
            .body(
                "This is a test mail from the sotyware account service. Mail delivery works."
                    .to_owned(),
            )
            .map_err(|e| e.to_string())?;
        self.transport
            .send(message)
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// What happens to a mail after a failed attempt: retried at the returned
/// time, or dropped (`None`).
pub fn after_failure(mail: &db::QueuedMail, error: &SendError, now: i64) -> Option<i64> {
    if matches!(error, SendError::Permanent(_)) || now - mail.at_ms >= GIVE_UP_AFTER_MS {
        return None;
    }
    let shift = u32::try_from(mail.attempts.clamp(0, 10)).unwrap_or(10);
    Some(now + (FIRST_RETRY_MS << shift).min(MAX_RETRY_MS))
}

async fn db_call<T: Send + 'static>(
    state: &Arc<AppState>,
    f: impl FnOnce(&rusqlite::Connection, i64) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn, (state.clock)()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sends every mail that is due now. Returns when the next one is due
/// (`None`: the outbox is empty).
pub async fn deliver_due(state: &Arc<AppState>, mailer: &Mailer) -> Result<Option<i64>, String> {
    loop {
        let due = db_call(state, |c, now| db::due_mails(c, now, BATCH)).await?;
        if due.is_empty() {
            break;
        }
        for mail in due {
            let now = (state.clock)();
            let result = if now - mail.at_ms >= GIVE_UP_AFTER_MS {
                Err(SendError::Permanent("not sent within a day".into()))
            } else {
                mailer.send(&mail.to, &mail.subject, &mail.body).await
            };
            let now = (state.clock)();
            let id = mail.id;
            match result {
                Ok(()) => db_call(state, move |c, _| db::remove_mail(c, id)).await?,
                Err(e) => match after_failure(&mail, &e, now) {
                    Some(next) => {
                        eprintln!(
                            "sotyware-server: mail {id} not sent, attempt {}: {e}; trying again in {} s",
                            mail.attempts + 1,
                            (next - now) / 1000
                        );
                        db_call(state, move |c, _| db::mail_failed(c, id, next)).await?;
                    }
                    None => {
                        eprintln!("sotyware-server: mail {id} dropped: {e}");
                        db_call(state, move |c, _| db::remove_mail(c, id)).await?;
                    }
                },
            }
        }
    }
    db_call(state, |c, _| db::next_mail_due(c)).await
}

/// The delivery task: runs until the runtime stops. A mail being sent when
/// the service stops stays queued and is sent again at the next start.
pub async fn run(state: Arc<AppState>, mailer: Mailer) {
    loop {
        let wait = match deliver_due(&state, &mailer).await {
            Ok(Some(next)) => {
                let ms = (next - (state.clock)()).clamp(0, SWEEP.as_millis() as i64);
                Duration::from_millis(u64::try_from(ms).unwrap_or(0))
            }
            Ok(None) => SWEEP,
            Err(e) => {
                eprintln!("sotyware-server: mail delivery cannot read the outbox: {e}");
                SWEEP
            }
        };
        tokio::select! {
            () = state.mail_queued.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<Option<SmtpConfig>, String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        SmtpConfig::from_env(|k| map.get(k).cloned())
    }

    const FROM: (&str, &str) = ("SOTYWARE_MAIL_FROM", "sotyware <no-reply@example.org>");

    #[test]
    fn no_host_means_no_delivery() {
        assert_eq!(config(&[]), Ok(None));
        assert_eq!(config(&[("SOTYWARE_SMTP_HOST", " "), FROM]), Ok(None));
    }

    #[test]
    fn defaults_are_starttls_on_587() {
        let c = config(&[("SOTYWARE_SMTP_HOST", "smtp.example.org"), FROM])
            .unwrap()
            .unwrap();
        assert_eq!(
            (c.security, c.port, c.credentials),
            (Security::StartTls, 587, None)
        );
        assert_eq!(c.from.email.to_string(), "no-reply@example.org");
    }

    #[test]
    fn ports_follow_the_security_unless_given() {
        let tls = config(&[
            ("SOTYWARE_SMTP_HOST", "smtp.example.org"),
            ("SOTYWARE_SMTP_SECURITY", "tls"),
            FROM,
        ])
        .unwrap()
        .unwrap();
        assert_eq!((tls.security, tls.port), (Security::Tls, 465));
        let custom = config(&[
            ("SOTYWARE_SMTP_HOST", "smtp.example.org"),
            ("SOTYWARE_SMTP_PORT", "2525"),
            FROM,
        ])
        .unwrap()
        .unwrap();
        assert_eq!(custom.port, 2525);
    }

    #[test]
    fn credentials_come_in_pairs_and_are_not_printed() {
        let c = config(&[
            ("SOTYWARE_SMTP_HOST", "smtp.example.org"),
            ("SOTYWARE_SMTP_USERNAME", "user"),
            ("SOTYWARE_SMTP_PASSWORD", "very-secret-key"),
            FROM,
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            c.credentials,
            Some(("user".into(), "very-secret-key".into()))
        );
        assert!(!format!("{c:?}").contains("very-secret-key"));
        for half in [
            ("SOTYWARE_SMTP_USERNAME", "user"),
            ("SOTYWARE_SMTP_PASSWORD", "x"),
        ] {
            assert!(config(&[("SOTYWARE_SMTP_HOST", "smtp.example.org"), half, FROM]).is_err());
        }
    }

    #[test]
    fn bad_settings_are_refused() {
        let host = ("SOTYWARE_SMTP_HOST", "smtp.example.org");
        assert!(config(&[host]).is_err(), "no sender");
        assert!(config(&[host, ("SOTYWARE_MAIL_FROM", "not an address")]).is_err());
        assert!(config(&[host, ("SOTYWARE_SMTP_PORT", "70000"), FROM]).is_err());
        assert!(config(&[host, ("SOTYWARE_SMTP_SECURITY", "ssl"), FROM]).is_err());
    }

    #[test]
    fn plaintext_only_to_this_machine() {
        let none = ("SOTYWARE_SMTP_SECURITY", "none");
        assert!(config(&[("SOTYWARE_SMTP_HOST", "smtp.example.org"), none, FROM]).is_err());
        for host in ["localhost", "127.0.0.1", "::1", "[::1]"] {
            let c = config(&[("SOTYWARE_SMTP_HOST", host), none, FROM])
                .unwrap()
                .unwrap();
            assert_eq!((c.security, c.port), (Security::None, 25), "{host}");
        }
    }

    fn mail(at_ms: i64, attempts: i64) -> db::QueuedMail {
        db::QueuedMail {
            id: 1,
            at_ms,
            to: "a@example.org".into(),
            subject: "s".into(),
            body: "b".into(),
            attempts,
        }
    }

    #[test]
    fn retries_back_off_and_end() {
        let t = SendError::Temporary("smtp 451".into());
        assert_eq!(after_failure(&mail(0, 0), &t, 1_000), Some(31_000));
        assert_eq!(after_failure(&mail(0, 1), &t, 1_000), Some(61_000));
        assert_eq!(after_failure(&mail(0, 2), &t, 1_000), Some(121_000));
        assert_eq!(
            after_failure(&mail(0, 9), &t, 1_000),
            Some(1_000 + MAX_RETRY_MS)
        );
        assert_eq!(after_failure(&mail(0, 3), &t, GIVE_UP_AFTER_MS), None);
        let p = SendError::Permanent("smtp 550".into());
        assert_eq!(after_failure(&mail(0, 0), &p, 1_000), None);
    }
}

//! Mail delivery (`mail.rs`) against a small SMTP server in the test
//! process: what is sent, what is kept for a retry, what is dropped, and a
//! registration through the API whose code arrives by mail.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use serde_json::json;
use sotyware_server::mail::{self, GIVE_UP_AFTER_MS, Mailer, Security, SmtpConfig};
use sotyware_server::security::Cost;
use sotyware_server::{AppState, Config, api, db};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// One mail as the SMTP server received it.
#[derive(Clone, Debug)]
struct Received {
    from: String,
    to: String,
    data: String,
}

impl Received {
    /// The message with quoted-printable soft line breaks undone.
    fn text(&self) -> String {
        self.data.replace("=\r\n", "").replace("=3D", "=")
    }
}

/// A minimal SMTP server: answers `rcpt_reply` to RCPT TO and records
/// every accepted message.
struct Smtp {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
    rcpt_reply: Arc<Mutex<&'static str>>,
}

async fn smtp_server() -> Smtp {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let rcpt_reply = Arc::new(Mutex::new("250 ok"));
    let (r, reply) = (Arc::clone(&received), Arc::clone(&rcpt_reply));
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (r, reply) = (Arc::clone(&r), Arc::clone(&reply));
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                send(&mut write, "220 test ESMTP").await;
                let (mut from, mut to) = (String::new(), String::new());
                while let Ok(Some(line)) = lines.next_line().await {
                    let upper = line.to_ascii_uppercase();
                    if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        send(&mut write, "250 test").await;
                    } else if upper.starts_with("MAIL FROM:") {
                        from = line[10..].trim().to_owned();
                        send(&mut write, "250 ok").await;
                    } else if upper.starts_with("RCPT TO:") {
                        to = line[8..].trim().to_owned();
                        let answer = *reply.lock().unwrap();
                        send(&mut write, answer).await;
                    } else if upper == "DATA" {
                        send(&mut write, "354 go ahead").await;
                        let mut data = String::new();
                        while let Ok(Some(l)) = lines.next_line().await {
                            if l == "." {
                                break;
                            }
                            data.push_str(&l);
                            data.push_str("\r\n");
                        }
                        r.lock().unwrap().push(Received {
                            from: from.clone(),
                            to: to.clone(),
                            data,
                        });
                        send(&mut write, "250 queued").await;
                    } else if upper == "QUIT" {
                        send(&mut write, "221 bye").await;
                        return;
                    } else if upper == "RSET" || upper == "NOOP" {
                        send(&mut write, "250 ok").await;
                    } else {
                        send(&mut write, "502 not implemented").await;
                    }
                }
            });
        }
    });
    Smtp {
        addr,
        received,
        rcpt_reply,
    }
}

async fn send(w: &mut tokio::net::tcp::OwnedWriteHalf, line: &str) {
    let _ = w.write_all(format!("{line}\r\n").as_bytes()).await;
}

fn relay(port: u16) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        security: Security::None,
        credentials: None,
        from: "sotyware <no-reply@example.org>".parse().unwrap(),
    }
}

struct Service {
    state: Arc<AppState>,
    clock: Arc<AtomicI64>,
    dir: std::path::PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn service(name: &str) -> Service {
    let dir = std::env::temp_dir().join(format!(
        "sotyware-server-mail-{name}-{}",
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
    Service { state, clock, dir }
}

impl Service {
    fn queue(&self, to: &str, subject: &str, body: &str) {
        let c = self.state.db.lock().unwrap();
        let now = self.clock.load(Ordering::SeqCst);
        db::queue_mail(&c, now, to, subject, body).unwrap();
    }

    fn outbox(&self) -> Vec<db::QueuedMail> {
        db::due_mails(&self.state.db.lock().unwrap(), i64::MAX, 100).unwrap()
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn a_queued_mail_is_sent_and_removed() {
    let smtp = smtp_server().await;
    let s = service("sent");
    s.queue(
        "Someone@Example.org",
        "Your sotyware verification code",
        "Your verification code is 123456. It expires in 15 minutes.",
    );
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    assert_eq!(mail::deliver_due(&s.state, &mailer).await, Ok(None));
    assert!(s.outbox().is_empty(), "a sent mail leaves the outbox");
    let got = smtp.received.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].from, "<no-reply@example.org>");
    assert_eq!(got[0].to, "<Someone@Example.org>");
    let text = got[0].text();
    assert!(
        text.contains("Subject: Your sotyware verification code"),
        "{text}"
    );
    assert!(
        text.contains("From: sotyware <no-reply@example.org>"),
        "{text}"
    );
    assert!(
        text.contains("Content-Type: text/plain; charset=utf-8"),
        "{text}"
    );
    assert!(text.contains("Your verification code is 123456."), "{text}");
}

#[tokio::test]
async fn a_temporary_refusal_is_retried_later() {
    let smtp = smtp_server().await;
    let s = service("retry");
    s.queue("a@example.org", "subject", "body");
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    *smtp.rcpt_reply.lock().unwrap() = "451 try again later";
    let first = s.now();
    assert_eq!(
        mail::deliver_due(&s.state, &mailer).await,
        Ok(Some(first + 30_000))
    );
    let kept = s.outbox();
    assert_eq!((kept.len(), kept[0].attempts), (1, 1));
    // Not due yet: nothing is tried.
    s.advance(10_000);
    assert_eq!(
        mail::deliver_due(&s.state, &mailer).await,
        Ok(Some(first + 30_000))
    );
    // Due again, still refused: the delay doubles.
    s.advance(20_000);
    assert_eq!(
        mail::deliver_due(&s.state, &mailer).await,
        Ok(Some(s.now() + 60_000))
    );
    // Accepted at last.
    *smtp.rcpt_reply.lock().unwrap() = "250 ok";
    s.advance(60_000);
    assert_eq!(mail::deliver_due(&s.state, &mailer).await, Ok(None));
    assert!(s.outbox().is_empty());
    assert_eq!(smtp.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unreachable_relay_is_a_temporary_failure() {
    // A port nothing listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let s = service("unreachable");
    s.queue("a@example.org", "subject", "body");
    let mailer = Mailer::new(&relay(port)).unwrap();
    let next = mail::deliver_due(&s.state, &mailer).await.unwrap();
    assert_eq!(next, Some(s.now() + 30_000));
    assert_eq!(s.outbox()[0].attempts, 1);
}

#[tokio::test]
async fn a_permanent_refusal_drops_the_mail() {
    let smtp = smtp_server().await;
    let s = service("permanent");
    s.queue("gone@example.org", "subject", "body");
    s.queue("b@example.org", "second", "body");
    *smtp.rcpt_reply.lock().unwrap() = "550 no such user";
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    assert_eq!(mail::deliver_due(&s.state, &mailer).await, Ok(None));
    assert!(s.outbox().is_empty());
    assert!(smtp.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_mail_not_sent_within_a_day_is_dropped_unsent() {
    let smtp = smtp_server().await;
    let s = service("old");
    s.queue("a@example.org", "subject", "body");
    s.advance(GIVE_UP_AFTER_MS);
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    assert_eq!(mail::deliver_due(&s.state, &mailer).await, Ok(None));
    assert!(s.outbox().is_empty());
    assert!(smtp.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unusable_address_is_dropped() {
    let smtp = smtp_server().await;
    let s = service("address");
    s.queue("not an address", "subject", "body");
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    assert_eq!(mail::deliver_due(&s.state, &mailer).await, Ok(None));
    assert!(s.outbox().is_empty());
    assert!(smtp.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_registration_code_arrives_by_mail() {
    let smtp = smtp_server().await;
    let s = service("register");
    let mailer = Mailer::new(&relay(smtp.addr.port())).unwrap();
    let task = tokio::spawn(mail::run(Arc::clone(&s.state), mailer));
    // Let the task finish its first pass over the (empty) outbox and wait,
    // so only the handler's wake-up can make it send in time.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut req = Request::builder()
        .method("POST")
        .uri("/v1/accounts")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "email": "new@example.org", "password": "a long enough passphrase" })
                .to_string(),
        ))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo::<SocketAddr>(
        "203.0.113.7:5555".parse().unwrap(),
    ));
    let res = api::router(Arc::clone(&s.state))
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    // Woken by the handler, not by the minute-long sweep.
    let mut waited = Duration::ZERO;
    while smtp.received.lock().unwrap().is_empty() {
        assert!(waited < Duration::from_secs(10), "no mail arrived");
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += Duration::from_millis(20);
    }
    let got = smtp.received.lock().unwrap()[0].clone();
    assert_eq!(got.to, "<new@example.org>");
    let text = got.text();
    let code = text
        .split("Your verification code is ")
        .nth(1)
        .and_then(|rest| rest.get(..8))
        .expect("a code in the mail");
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    // Sent, so no longer kept in the database.
    let mut waited = Duration::ZERO;
    while !s.outbox().is_empty() {
        assert!(
            waited < Duration::from_secs(10),
            "the sent mail stayed queued"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += Duration::from_millis(20);
    }
    task.abort();
}

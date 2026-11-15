//! `sotyware-server`: run the account service, or back up / restore its
//! database. Configuration through environment variables (no secret is ever
//! passed on the command line):
//!
//! - `SOTYWARE_SERVER_DB`: database file (default `sotyware-server.sqlite3`)
//! - `SOTYWARE_SERVER_ADDR`: listen address (default `127.0.0.1:8787`; TLS
//!   is terminated by a reverse proxy in front, see docs/development/server.md)
//! - `SOTYWARE_SERVER_TRUST_PROXY=1`: client addresses from `X-Forwarded-For`
//!
//! Commands: `serve` (default), `backup <file>`, `restore <file>`,
//! `outbox` (development only: prints queued mails, which contain codes).

use std::net::SocketAddr;
use std::path::PathBuf;

use sotyware_server::{AppState, Config, api, db, now_ms};

fn main() {
    let db_path = PathBuf::from(
        std::env::var("SOTYWARE_SERVER_DB").unwrap_or_else(|_| "sotyware-server.sqlite3".into()),
    );
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("serve") => serve(db_path),
        Some("backup") => args
            .get(1)
            .ok_or_else(|| "usage: backup <file>".to_owned())
            .and_then(|to| {
                let conn = db::open(&db_path)?;
                db::backup(&conn, std::path::Path::new(to))?;
                println!("backed up to {to}");
                Ok(())
            }),
        Some("restore") => args
            .get(1)
            .ok_or_else(|| "usage: restore <file> (with the service stopped)".to_owned())
            .and_then(|from| {
                db::restore(std::path::Path::new(from), &db_path, now_ms())?;
                db::open(&db_path)?;
                println!("restored {from} to {}", db_path.display());
                Ok(())
            }),
        Some("outbox") => db::open(&db_path).and_then(|conn| {
            for (to, subject, body) in db::outbox(&conn).map_err(|e| e.to_string())? {
                println!("To: {to}\nSubject: {subject}\n\n{body}\n---");
            }
            Ok(())
        }),
        Some(other) => Err(format!("unknown command '{other}'")),
    };
    if let Err(e) = result {
        eprintln!("sotyware-server: {e}");
        std::process::exit(1);
    }
}

fn serve(db_path: PathBuf) -> Result<(), String> {
    let conn = db::open(&db_path)?;
    let config = Config {
        trust_proxy: std::env::var("SOTYWARE_SERVER_TRUST_PROXY").is_ok_and(|v| v == "1"),
        ..Config::default()
    };
    let addr: SocketAddr = std::env::var("SOTYWARE_SERVER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .map_err(|e| format!("SOTYWARE_SERVER_ADDR: {e}"))?;
    let state = AppState::new(conn, config);
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("cannot listen on {addr}: {e}"))?;
        eprintln!(
            "sotyware-server: listening on {addr} (database {})",
            db_path.display()
        );
        let app = api::router(state).into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| e.to_string())
    })
}

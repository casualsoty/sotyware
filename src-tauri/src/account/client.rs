//! HTTP access to the account service (ADR-032): JSON over HTTPS with
//! rustls on ring and the operating system's root certificates (the same
//! TLS stack as the storage client). Plain HTTP only to a loopback address
//! (a development or test server). Redirects are never followed, so a
//! bearer token is never sent anywhere but the configured service.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

/// A failed call: the HTTP status (0 when the service could not be
/// reached) and the service's error code and message. Never carries a
/// token or a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status == 0 {
            write!(
                f,
                "the account service could not be reached: {}",
                self.message
            )
        } else {
            write!(f, "{} ({}, HTTP {})", self.message, self.code, self.status)
        }
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Checks a service URL: `https://host[:port]`, or `http://` to a loopback
/// address only. Returns it without a trailing slash.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "the account service URL must start with https://".to_owned())?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit_once(':')
        .map_or(rest.split('/').next().unwrap_or(""), |(h, port)| {
            if port.bytes().all(|b| b.is_ascii_digit()) {
                h
            } else {
                rest
            }
        });
    match scheme {
        "https" if !host.is_empty() => Ok(url.to_owned()),
        "http" if is_loopback(host) => Ok(url.to_owned()),
        "http" => Err("the account service must be reached over HTTPS".into()),
        _ => Err("the account service URL must start with https://".into()),
    }
}

pub struct Http {
    base: String,
    client: reqwest::Client,
}

/// The TLS configuration of sotyware's own HTTPS clients (the account
/// service, updates): rustls on ring with the operating system's root
/// certificates. Never aws-lc-rs, never a process-wide default provider.
pub fn tls_config() -> Result<rustls::ClientConfig, String> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth())
}

impl Http {
    pub fn new(base: &str) -> Result<Self, String> {
        let base = check_url(base)?;
        let tls = tls_config()?;
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .https_only(base.starts_with("https://"))
            .user_agent(concat!("sotyware/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base, client })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// One call: the JSON body of a 2xx or 409 answer (with its status), or
    /// the error.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<(u16, Value), HttpError> {
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| HttpError {
            status: 0,
            code: "client".into(),
            message: e.to_string(),
        })?;
        let mut req = self.client.request(method, format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|e| HttpError {
            status: 0,
            code: if e.is_timeout() { "timeout" } else { "network" }.into(),
            // reqwest's Display shows the URL (no token) and the cause.
            message: e.without_url().to_string(),
        })?;
        let status = res.status().as_u16();
        let value: Value = res.json().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) || status == 409 {
            return Ok((status, value));
        }
        Err(HttpError {
            status,
            code: value["error"].as_str().unwrap_or("error").to_owned(),
            message: value["message"]
                .as_str()
                .unwrap_or("the account service refused the request")
                .to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_loopback_http_is_accepted() {
        assert_eq!(
            check_url("https://accounts.example.org/").as_deref(),
            Ok("https://accounts.example.org")
        );
        assert!(check_url("https://accounts.example.org:8443").is_ok());
        assert!(check_url("http://127.0.0.1:8787").is_ok());
        assert!(check_url("http://localhost:8787").is_ok());
        assert!(check_url("http://[::1]:8787").is_ok());
        assert!(check_url("http://accounts.example.org").is_err());
        assert!(check_url("http://10.0.0.1:8787").is_err());
        assert!(check_url("ftp://x").is_err());
        assert!(check_url("accounts.example.org").is_err());
    }
}

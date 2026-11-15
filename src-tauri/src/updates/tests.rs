//! Update verification (ADR-033) with keys generated here (never a real
//! key), a local HTTP server, and the updater plugin on Tauri's mock
//! runtime. Nothing is ever installed: the tests stop after the download
//! and its checks (installing would replace the running test binary on
//! Linux and start an installer on Windows).

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State as AxState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets};

use super::*;

// ---------------------------------------------------------------- keys

struct Keys {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

fn keys() -> Keys {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        pk: kp.pk,
        sk: kp.sk,
    }
}

impl Keys {
    /// As `tauri signer generate` writes it: base64 of the key's text.
    fn pubkey(&self) -> String {
        B64.encode(self.pk.to_box().unwrap().to_string())
    }

    /// As `tauri signer sign` writes a `.sig`: base64 of the signature text.
    fn sign(&self, data: &[u8], trusted: &str) -> String {
        let sb = minisign::sign(
            None,
            &self.sk,
            Cursor::new(data),
            Some(trusted),
            Some("signature from tauri secret key"),
        )
        .unwrap();
        B64.encode(sb.to_string())
    }

    /// An artifact signature bound to `version`, as `--app-version` does.
    fn sign_artifact(&self, data: &[u8], version: &str) -> String {
        self.sign(
            data,
            &format!("timestamp:1791649051\tfile:sotyware_setup.exe\tversion:{version}"),
        )
    }
}

const TARGET: &str = "windows-x86_64-nsis";
const ARTIFACT: &[u8] = b"MZ this stands in for an installer";

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn manifest(version: &str, platforms: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "format": "sotyware-update",
        "formatVersion": 1,
        "channel": "stable",
        "version": version,
        "notes": "Fixes.",
        "pub_date": "2026-11-15T12:00:00Z",
        "platforms": platforms,
    }))
    .unwrap()
}

fn entry(k: &Keys, url: &str, version: &str) -> serde_json::Value {
    json!({
        "url": url,
        "signature": k.sign_artifact(ARTIFACT, version),
        "sha256": sha(ARTIFACT),
    })
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

fn verify(
    bytes: &[u8],
    sig: &str,
    k: &Keys,
    floor: Option<&str>,
) -> Result<Option<Offer>, UpdateError> {
    verify_manifest(
        bytes,
        sig,
        &k.pubkey(),
        "stable",
        &v("0.1.0"),
        floor.map(v).as_ref(),
        &[TARGET.to_owned(), "windows-x86_64".to_owned()],
        false,
    )
}

// ---------------------------------------------------------------- pure checks

#[test]
fn a_signed_newer_manifest_is_offered_for_this_installation() {
    let k = keys();
    let m = manifest(
        "0.2.0",
        json!({
            "windows-x86_64": entry(&k, "https://example.org/generic.exe", "0.2.0"),
            TARGET: entry(&k, "https://example.org/setup.exe", "0.2.0"),
            "windows-x86_64-msi": entry(&k, "https://example.org/x.msi", "0.2.0"),
        }),
    );
    let offer = verify(&m, &k.sign(&m, "manifest"), &k, None)
        .unwrap()
        .unwrap();
    // The installer-specific entry wins over the generic one.
    assert_eq!(offer.target, TARGET);
    assert_eq!(offer.artifact.url, "https://example.org/setup.exe");
    assert_eq!(offer.manifest.version, "0.2.0");
    assert_eq!(
        offer.raw,
        serde_json::from_slice::<serde_json::Value>(&m).unwrap()
    );
    // A signature given as text (minisign's own format) verifies too.
    let text = String::from_utf8(B64.decode(k.sign(&m, "manifest")).unwrap()).unwrap();
    assert!(verify(&m, &text, &k, None).unwrap().is_some());
    // Only the generic entry: used.
    let m = manifest(
        "0.2.0",
        json!({ "windows-x86_64": entry(&k, "https://example.org/generic.exe", "0.2.0") }),
    );
    let offer = verify(&m, &k.sign(&m, "m"), &k, None).unwrap().unwrap();
    assert_eq!(offer.target, "windows-x86_64");
}

#[test]
fn a_tampered_or_foreign_manifest_is_refused() {
    let k = keys();
    let m = manifest(
        "0.2.0",
        json!({ TARGET: entry(&k, "https://example.org/a.exe", "0.2.0") }),
    );
    let sig = k.sign(&m, "manifest");
    // One byte changed after signing.
    let mut tampered = m.clone();
    let i = tampered.iter().position(|&b| b == b'2').unwrap();
    tampered[i] = b'9';
    assert_eq!(
        verify(&tampered, &sig, &k, None),
        Err(UpdateError::Signature)
    );
    // Signed by another key.
    let other = keys();
    assert_eq!(
        verify(&m, &other.sign(&m, "manifest"), &k, None),
        Err(UpdateError::Signature)
    );
    // No or garbage signature.
    assert_eq!(verify(&m, "", &k, None), Err(UpdateError::Signature));
    assert!(verify(&m, "bm90IGEgc2lnbmF0dXJl", &k, None).is_err());
    // An artifact signature is no manifest signature.
    assert_eq!(
        verify(&m, &k.sign(ARTIFACT, "x"), &k, None),
        Err(UpdateError::Signature)
    );
    // The error never shows a key.
    let text = UpdateError::Signature.to_string();
    assert!(!text.contains(&k.pubkey()));
}

#[test]
fn format_and_channel_are_checked_after_the_signature() {
    let k = keys();
    let mut doc: serde_json::Value = serde_json::from_slice(&manifest(
        "0.2.0",
        json!({ TARGET: entry(&k, "https://example.org/a.exe", "0.2.0") }),
    ))
    .unwrap();
    doc["channel"] = json!("beta");
    let m = serde_json::to_vec(&doc).unwrap();
    assert!(matches!(
        verify(&m, &k.sign(&m, "m"), &k, None),
        Err(UpdateError::WrongChannel { .. })
    ));
    doc["channel"] = json!("stable");
    doc["formatVersion"] = json!(2);
    let m = serde_json::to_vec(&doc).unwrap();
    assert!(matches!(
        verify(&m, &k.sign(&m, "m"), &k, None),
        Err(UpdateError::Malformed(_))
    ));
    // The plugin's plain latest.json (no format) is not accepted.
    let plain = serde_json::to_vec(&json!({
        "version": "0.2.0",
        "platforms": { TARGET: { "url": "https://example.org/a.exe", "signature": "x" } }
    }))
    .unwrap();
    assert!(matches!(
        verify(&plain, &k.sign(&plain, "m"), &k, None),
        Err(UpdateError::Malformed(_))
    ));
}

#[test]
fn versions_below_the_floor_are_refused_and_not_newer_ones_ignored() {
    let k = keys();
    let signed = |version: &str| {
        let m = manifest(
            version,
            json!({ TARGET: entry(&k, "https://example.org/a.exe", version) }),
        );
        let s = k.sign(&m, "m");
        (m, s)
    };
    // Older than one offered before: a replay or rollback.
    let (m, s) = signed("0.1.5");
    assert_eq!(
        verify(&m, &s, &k, Some("0.2.0")),
        Err(UpdateError::Downgrade {
            offered: "0.1.5".into(),
            floor: "0.2.0".into()
        })
    );
    // The running version or older (and not below the floor): nothing to do.
    let (m, s) = signed("0.1.0");
    assert_eq!(verify(&m, &s, &k, Some("0.1.0")), Ok(None));
    let (m, s) = signed("0.0.9");
    assert_eq!(verify(&m, &s, &k, None), Ok(None));
    // Equal to the floor and newer than the running version: offered.
    let (m, s) = signed("0.2.0");
    assert!(verify(&m, &s, &k, Some("0.2.0")).unwrap().is_some());
    // Not a version at all.
    let (m, s) = signed("soon");
    assert!(matches!(
        verify(&m, &s, &k, None),
        Err(UpdateError::Malformed(_))
    ));
}

#[test]
fn the_package_entry_must_exist_use_https_and_carry_a_signature_and_hash() {
    let k = keys();
    let check = |platforms: serde_json::Value, loopback: bool| {
        let m = manifest("0.2.0", platforms);
        verify_manifest(
            &m,
            &k.sign(&m, "m"),
            &k.pubkey(),
            "stable",
            &v("0.1.0"),
            None,
            &[TARGET.to_owned()],
            loopback,
        )
    };
    assert!(matches!(
        check(
            json!({ "linux-x86_64": entry(&k, "https://example.org/a", "0.2.0") }),
            false
        ),
        Err(UpdateError::Platform(_))
    ));
    assert!(matches!(
        check(
            json!({ TARGET: entry(&k, "http://example.org/a.exe", "0.2.0") }),
            false
        ),
        Err(UpdateError::Platform(_))
    ));
    // Loopback HTTP only in test builds.
    assert!(matches!(
        check(
            json!({ TARGET: entry(&k, "http://127.0.0.1:9/a.exe", "0.2.0") }),
            false
        ),
        Err(UpdateError::Platform(_))
    ));
    assert!(
        check(
            json!({ TARGET: entry(&k, "http://127.0.0.1:9/a.exe", "0.2.0") }),
            true
        )
        .is_ok()
    );
    let mut e = entry(&k, "https://example.org/a.exe", "0.2.0");
    e["sha256"] = json!("abc");
    assert!(matches!(
        check(json!({ TARGET: e }), false),
        Err(UpdateError::Malformed(_))
    ));
    let mut e = entry(&k, "https://example.org/a.exe", "0.2.0");
    e["signature"] = json!("");
    assert!(matches!(
        check(json!({ TARGET: e }), false),
        Err(UpdateError::Malformed(_))
    ));
}

#[test]
fn urls_hashes_and_the_plugins_view_are_compared_exactly() {
    assert!(url_ok("https://updates.example.org/x", false));
    assert!(!url_ok("http://updates.example.org/x", true));
    assert!(!url_ok("http://127.0.0.1.example.org/x", true));
    assert!(url_ok("http://127.0.0.1:8080/x", true));
    assert!(url_ok("http://localhost/x", true));
    assert!(!url_ok("file:///C:/x.exe", true));
    // IPv6: only the loopback address, by the parsed host.
    assert!(url_ok("http://[::1]:8080/x", true));
    assert!(!url_ok("http://[2001:db8::1]/x", true));
    assert!(!url_ok("http://[::ffff:203.0.113.7]:80/x", true));
    assert!(url_ok("http://127.0.0.2/x", true));
    assert!(!url_ok("http://127.0.0.1@example.org/x", true));
    assert!(!url_ok("http://localhost.example.org/x", true));
    assert!(!url_ok("http://[::1]/x", false));

    assert_eq!(check_sha256(ARTIFACT, &sha(ARTIFACT)), Ok(()));
    assert_eq!(
        check_sha256(ARTIFACT, &sha(ARTIFACT).to_uppercase()),
        Ok(())
    );
    assert!(matches!(
        check_sha256(b"other", &sha(ARTIFACT)),
        Err(UpdateError::Integrity(_))
    ));

    let k = keys();
    let m = manifest(
        "0.2.0",
        json!({ TARGET: entry(&k, "https://example.org/a.exe", "0.2.0") }),
    );
    let offer = verify(&m, &k.sign(&m, "m"), &k, None).unwrap().unwrap();
    let a = &offer.artifact;
    assert_eq!(same_release(&offer, "0.2.0", &a.url, &a.signature), Ok(()));
    assert_eq!(same_release(&offer, "v0.2.0", &a.url, &a.signature), Ok(()));
    assert!(same_release(&offer, "0.3.0", &a.url, &a.signature).is_err());
    assert!(same_release(&offer, "0.2.0", "https://example.org/b.exe", &a.signature).is_err());
    assert!(same_release(&offer, "0.2.0", &a.url, "other").is_err());
}

#[test]
fn a_build_without_a_release_key_has_no_update_source() {
    let u = Updates::new(None, Ok(Box::new(Memory::default())));
    let s = u.status(&v("0.1.0"), Vec::new());
    assert!(!s.configured);
    assert_eq!(s.current, "0.1.0");
    assert!(s.available.is_none());
    assert_eq!(s.installed, None);
    assert_eq!(s.uses_installer, cfg!(windows));
}

#[test]
fn the_production_source_is_github_and_the_committed_key() {
    // The manifest of the release GitHub marks as latest (ADR-036), in the
    // public source repository, where release.yml publishes (ADR-039).
    assert_eq!(
        MANIFEST_URL,
        format!("https://github.com/{RELEASE_REPOSITORY}/releases/latest/download/latest.json")
    );
    assert_eq!(
        MANIFEST_URL,
        "https://github.com/casualsoty/sotyware/releases/latest/download/latest.json"
    );
    assert_eq!(RELEASE_REPOSITORY, "casualsoty/sotyware");
    // Either no key yet (updates off) or one that decodes as minisign.
    match production_pubkey() {
        None => assert!(production().is_none()),
        Some(pk) => {
            let text = text_of(pk).expect("update-key.pub holds base64 of a minisign key");
            minisign_verify::PublicKey::decode(&text)
                .expect("update-key.pub holds a minisign public key");
            let src = production().unwrap();
            assert_eq!(src.manifest_url, MANIFEST_URL);
            assert_eq!(src.channel, "stable");
        }
    }
}

/// The signing fixtures' throwaway keys are for tests only: if one of them
/// were committed as the release key, anyone holding a copy of the deleted
/// private half (or a fixture made with it) could sign "updates". The key
/// id (bytes 2..10 of the decoded key) is what a signature names.
#[test]
fn the_compiled_in_key_is_never_a_signing_fixture() {
    fn key_id(b64: &str) -> Vec<u8> {
        let text = text_of(b64).expect("base64 of a minisign key");
        let line = text.lines().nth(1).expect("a key line");
        B64.decode(line.trim()).expect("base64 key")[2..10].to_vec()
    }
    let fixtures = [
        include_str!("../../../scripts/fixtures/update/test-key.pub"),
        include_str!("../../../scripts/fixtures/update-github/test-key.pub"),
    ];
    for f in fixtures {
        assert_eq!(key_id(f).len(), 8);
    }
    if let Some(pk) = production_pubkey() {
        for f in fixtures {
            assert_ne!(
                key_id(pk),
                key_id(f),
                "src-tauri/update-key.pub holds a test fixture's key"
            );
        }
    }
}

#[test]
fn the_key_file_skips_comments_and_blank_lines() {
    assert_eq!(pubkey_in(""), None);
    assert_eq!(pubkey_in("# only\n#comments\n\n"), None);
    assert_eq!(
        pubkey_in("# key\n\n  dW50cnVzdGVk  \n# after\n"),
        Some("dW50cnVzdGVk")
    );
}

#[test]
fn every_failure_has_a_kind_the_page_knows() {
    let kinds = [
        UpdateError::Network("x".into()).kind(),
        UpdateError::Signature.kind(),
        UpdateError::Malformed("x".into()).kind(),
        UpdateError::WrongChannel {
            got: "beta".into(),
            want: "stable".into(),
        }
        .kind(),
        UpdateError::Downgrade {
            offered: "0.1.0".into(),
            floor: "0.2.0".into(),
        }
        .kind(),
        UpdateError::Platform("x".into()).kind(),
        UpdateError::Integrity("x".into()).kind(),
        UpdateError::Updater("x".into()).kind(),
    ];
    assert_eq!(
        kinds,
        [
            "network",
            "signature",
            "malformed",
            "malformed",
            "replay",
            "platform",
            "integrity",
            "installer"
        ]
    );
    // UPDATE_ERROR_KINDS in src/updates.ts holds these and "busy".
    let ts = std::fs::read_to_string(format!("{}/../src/updates.ts", env!("CARGO_MANIFEST_DIR")))
        .unwrap();
    for k in kinds.iter().chain(&["busy"]) {
        assert!(
            ts.contains(&format!("\"{k}\"")),
            "{k} missing from src/updates.ts"
        );
    }
}

#[test]
fn a_restart_needs_an_installed_update_and_nothing_running() {
    assert!(restart_allowed(None, &[]).is_err());
    let busy = restart_allowed(Some("0.2.0"), &["an upload".to_owned()]).unwrap_err();
    assert_eq!(busy.kind, ErrorKind::Busy);
    assert!(busy.message.contains("an upload"), "{busy:?}");
    assert!(restart_allowed(Some("0.2.0"), &[]).is_ok());
    let u = Updates::new(None, Ok(Box::new(Memory::default())));
    u.set_installed("0.2.0".into());
    assert_eq!(
        u.status(&v("0.1.0"), Vec::new()).installed.as_deref(),
        Some("0.2.0")
    );
}

// ---------------------------------------------------------------- service

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<String>>>);

impl StateStore for Memory {
    fn load(&mut self) -> Result<Option<String>, crate::features::StoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&mut self, state: &str) -> Result<(), crate::features::StoreError> {
        *self.0.lock().unwrap() = Some(state.to_owned());
        Ok(())
    }
    fn clear(&mut self) -> Result<(), crate::features::StoreError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

impl Memory {
    fn saved(&self) -> Saved {
        serde_json::from_str(self.0.lock().unwrap().as_deref().unwrap_or("{}")).unwrap()
    }
}

#[derive(Default)]
struct Served {
    manifest: Vec<u8>,
    sig: Option<String>,
    artifact: Vec<u8>,
    stall: bool,
    /// Serve the manifest without a length, in 64 KiB chunks.
    chunked: bool,
}

/// A body of known chunks and no announced length.
struct Chunks(std::collections::VecDeque<axum::body::Bytes>);

impl futures_core::Stream for Chunks {
    type Item = Result<axum::body::Bytes, std::convert::Infallible>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.0.pop_front().map(Ok))
    }
}

#[derive(Clone)]
struct Server {
    url: String,
    served: Arc<Mutex<Served>>,
    artifact_requested: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Shared(Arc<Mutex<Served>>, Arc<AtomicBool>);

async fn manifest_route(AxState(s): AxState<Shared>) -> axum::response::Response {
    let (chunked, manifest) = {
        let g = s.0.lock().unwrap();
        (g.chunked, g.manifest.clone())
    };
    if chunked {
        let parts = manifest
            .chunks(64 * 1024)
            .map(axum::body::Bytes::copy_from_slice)
            .collect();
        return axum::body::Body::from_stream(Chunks(parts)).into_response();
    }
    manifest.into_response()
}

async fn sig_route(AxState(s): AxState<Shared>) -> axum::response::Response {
    match s.0.lock().unwrap().sig.clone() {
        Some(sig) => sig.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn artifact_route(AxState(s): AxState<Shared>) -> impl IntoResponse {
    s.1.store(true, Ordering::SeqCst);
    let (stall, bytes) = {
        let g = s.0.lock().unwrap();
        (g.stall, g.artifact.clone())
    };
    if stall {
        std::future::pending::<()>().await;
    }
    bytes
}

async fn server() -> Server {
    let served = Arc::new(Mutex::new(Served::default()));
    let requested = Arc::new(AtomicBool::new(false));
    let app = axum::Router::new()
        .route("/sotyware-update.json", get(manifest_route))
        .route("/sotyware-update.json.sig", get(sig_route))
        .route("/sotyware_setup.exe", get(artifact_route))
        .with_state(Shared(Arc::clone(&served), Arc::clone(&requested)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Server {
        url: format!("http://{addr}"),
        served,
        artifact_requested: requested,
    }
}

impl Server {
    /// Publishes `version` for this platform, signed by `k`.
    fn publish(&self, k: &Keys, version: &str) {
        let target = targets()
            .into_iter()
            .next()
            .unwrap_or_else(|| TARGET.into());
        let m = manifest(
            version,
            json!({ target: entry(k, &format!("{}/sotyware_setup.exe", self.url), version) }),
        );
        let mut g = self.served.lock().unwrap();
        g.sig = Some(k.sign(&m, "sotyware-update.json"));
        g.manifest = m;
        g.artifact = ARTIFACT.to_vec();
    }

    fn source(&self, k: &Keys) -> Source {
        Source {
            manifest_url: format!("{}/sotyware-update.json", self.url),
            pubkey: k.pubkey(),
            channel: "stable".into(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_check_verifies_the_manifest_and_raises_the_floor() {
    let srv = server().await;
    let k = keys();
    srv.publish(&k, "0.2.0");
    let mem = Memory::default();
    let u = Updates::new(Some(srv.source(&k)), Ok(Box::new(mem.clone())));
    let offer = u.check(&v("0.1.0")).await.unwrap().unwrap();
    assert_eq!(offer.manifest.version, "0.2.0");
    assert_eq!(mem.saved().floor.as_deref(), Some("0.2.0"));
    assert!(mem.saved().last_check_ms.is_some());
    let s = u.status(&v("0.1.0"), Vec::new());
    assert!(s.configured);
    assert_eq!(
        s.available.as_ref().map(|a| a.version.as_str()),
        Some("0.2.0")
    );
    assert_eq!(s.error, None);
    // Nothing was downloaded by a check.
    assert!(!srv.artifact_requested.load(Ordering::SeqCst));

    // Postponing hides it until a newer version.
    u.postpone();
    assert!(u.status(&v("0.1.0"), Vec::new()).postponed);
    srv.publish(&k, "0.3.0");
    u.check(&v("0.1.0")).await.unwrap();
    let s = u.status(&v("0.1.0"), Vec::new());
    assert_eq!(
        s.available.as_ref().map(|a| a.version.as_str()),
        Some("0.3.0")
    );
    assert!(!s.postponed);
    assert_eq!(mem.saved().floor.as_deref(), Some("0.3.0"));

    // A validly signed but older manifest is a replay: refused, the floor
    // stays and the last good offer is kept.
    srv.publish(&k, "0.2.0");
    assert!(matches!(
        u.check(&v("0.1.0")).await,
        Err(UpdateError::Downgrade { .. })
    ));
    assert_eq!(mem.saved().floor.as_deref(), Some("0.3.0"));
    let s = u.status(&v("0.1.0"), Vec::new());
    assert!(s.error.as_deref().unwrap().contains("replay"));
    assert_eq!(s.error_kind, Some("replay"));

    // A restart keeps the floor (it is stored).
    let again = Updates::new(Some(srv.source(&k)), Ok(Box::new(mem.clone())));
    assert!(matches!(
        again.check(&v("0.1.0")).await,
        Err(UpdateError::Downgrade { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn unsigned_tampered_or_unreachable_updates_change_nothing() {
    let srv = server().await;
    let k = keys();
    srv.publish(&k, "0.2.0");
    let mem = Memory::default();
    let u = Updates::new(Some(srv.source(&k)), Ok(Box::new(mem.clone())));
    // No signature published.
    srv.served.lock().unwrap().sig = None;
    assert!(matches!(
        u.check(&v("0.1.0")).await,
        Err(UpdateError::Network(_))
    ));
    // Signed by another key.
    let other = keys();
    {
        let mut g = srv.served.lock().unwrap();
        let m = g.manifest.clone();
        g.sig = Some(other.sign(&m, "m"));
    }
    assert_eq!(u.check(&v("0.1.0")).await, Err(UpdateError::Signature));
    // Changed after signing.
    srv.publish(&k, "0.2.0");
    {
        let mut g = srv.served.lock().unwrap();
        let text = String::from_utf8(g.manifest.clone()).unwrap();
        g.manifest = text.replace("Fixes.", "Fixes!").into_bytes();
    }
    assert_eq!(u.check(&v("0.1.0")).await, Err(UpdateError::Signature));
    // A manifest larger than any real one is refused, with or without an
    // announced length, without reading it all.
    srv.publish(&k, "0.2.0");
    srv.served.lock().unwrap().manifest = vec![b' '; MAX_MANIFEST + 1];
    assert!(
        matches!(u.check(&v("0.1.0")).await, Err(UpdateError::Malformed(m)) if m.contains("too large"))
    );
    srv.served.lock().unwrap().chunked = true;
    assert!(
        matches!(u.check(&v("0.1.0")).await, Err(UpdateError::Malformed(m)) if m.contains("too large"))
    );
    // The same server streaming a normal manifest works.
    srv.publish(&k, "0.2.0");
    srv.served.lock().unwrap().chunked = true;
    let streamed = Updates::new(Some(srv.source(&k)), Ok(Box::new(Memory::default())));
    assert!(streamed.check(&v("0.1.0")).await.unwrap().is_some());
    srv.served.lock().unwrap().chunked = false;
    // Nothing was stored, nothing offered.
    assert_eq!(mem.saved(), Saved::default());
    assert!(u.status(&v("0.1.0"), Vec::new()).available.is_none());
    // An HTTP endpoint is refused outside test builds.
    let mut plain = Updates::new(
        Some(Source {
            manifest_url: "http://updates.example.org/sotyware-update.json".into(),
            pubkey: k.pubkey(),
            channel: "stable".into(),
        }),
        Ok(Box::new(Memory::default())),
    );
    Arc::get_mut(&mut plain).unwrap().allow_loopback = false;
    assert!(matches!(
        plain.check(&v("0.1.0")).await,
        Err(UpdateError::Platform(_))
    ));
}

fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
    let mut ctx = mock_context(noop_assets());
    ctx.config_mut()
        .plugins
        .0
        .insert("updater".into(), json!({ "pubkey": "" }));
    mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(ctx)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plugin_downloads_exactly_the_verified_release() {
    let srv = server().await;
    let k = keys();
    srv.publish(&k, "0.2.0");
    let u = Updates::new(Some(srv.source(&k)), Ok(Box::new(Memory::default())));
    let offer = u.check(&v("0.1.0")).await.unwrap().unwrap();
    let app = mock_app();
    let mut seen = 0;
    let (update, bytes) =
        Updates::download_verified(app.handle(), &k.pubkey(), &offer, false, |d, _| seen = d)
            .await
            .unwrap();
    assert_eq!(bytes, ARTIFACT);
    assert_eq!(seen, ARTIFACT.len() as u64);
    assert_eq!(update.version, "0.2.0");
    assert_eq!(update.download_url.as_str(), offer.artifact.url);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_package_that_does_not_match_the_signed_manifest_is_never_returned() {
    let srv = server().await;
    let k = keys();
    let app = mock_app();
    let offer_with = |artifact_sig: Option<String>, sha256: Option<String>| {
        let target = targets().into_iter().next().unwrap();
        let mut e = entry(&k, &format!("{}/sotyware_setup.exe", srv.url), "0.2.0");
        if let Some(s) = artifact_sig {
            e["signature"] = json!(s);
        }
        if let Some(h) = sha256 {
            e["sha256"] = json!(h);
        }
        let m = manifest("0.2.0", json!({ target.clone(): e }));
        verify_manifest(
            &m,
            &k.sign(&m, "m"),
            &k.pubkey(),
            "stable",
            &v("0.1.0"),
            None,
            &[target],
            true,
        )
        .unwrap()
        .unwrap()
    };
    srv.publish(&k, "0.2.0");
    let run = |offer: Offer| {
        let handle = app.handle().clone();
        let pk = k.pubkey();
        async move { Updates::download_verified(&handle, &pk, &offer, false, |_, _| {}).await }
    };

    // The artifact was signed for an older version: the plugin refuses it
    // (a manifest pairing a new number with an old package).
    let r = run(offer_with(Some(k.sign_artifact(ARTIFACT, "0.1.5")), None)).await;
    assert!(matches!(r, Err(UpdateError::Integrity(_))), "{:?}", r.err());
    // Signed without a version.
    let r = run(offer_with(
        Some(k.sign(ARTIFACT, "timestamp:1\tfile:x")),
        None,
    ))
    .await;
    assert!(matches!(r, Err(UpdateError::Integrity(_))), "{:?}", r.err());
    // Signed by another key.
    let r = run(offer_with(
        Some(keys().sign_artifact(ARTIFACT, "0.2.0")),
        None,
    ))
    .await;
    assert!(matches!(r, Err(UpdateError::Integrity(_))), "{:?}", r.err());
    // The server sends other bytes than the ones signed.
    srv.served.lock().unwrap().artifact = b"MZ something else".to_vec();
    let r = run(offer_with(None, None)).await;
    assert!(matches!(r, Err(UpdateError::Integrity(_))), "{:?}", r.err());
    // Validly signed, but not the bytes the signed manifest names.
    srv.served.lock().unwrap().artifact = ARTIFACT.to_vec();
    let r = run(offer_with(None, Some(sha(b"another build")))).await;
    assert!(matches!(r, Err(UpdateError::Integrity(_))), "{:?}", r.err());
    // The untouched offer still works.
    assert!(run(offer_with(None, None)).await.is_ok());
}

/// The installation is one blocking call that aborting the task cannot
/// stop: a cancel that arrives before the commit point stops the task from
/// installing; one that arrives after it is refused instead of reporting
/// "nothing was installed".
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_is_refused_once_the_installation_is_committed() {
    let u = Updates::new(None, Ok(Box::new(Memory::default())));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let slot = |token: &Arc<()>| {
        let log = Arc::clone(&events);
        let (alive, ended) = std::sync::mpsc::channel::<()>();
        InstallSlot {
            task: tauri::async_runtime::spawn(async move {
                let _alive = alive;
                std::future::pending::<()>().await
            }),
            sink: Channel::new(move |body| {
                if let tauri::ipc::InvokeResponseBody::Json(j) = body {
                    log.lock().unwrap().push(j);
                }
                Ok(())
            }),
            token: Arc::clone(token),
            committed: false,
            ended: Some(ended),
        }
    };
    // Cancelled before the commit point: the task finds no slot and stops.
    let first = Arc::new(());
    *u.install.lock().unwrap() = Some(slot(&first));
    assert!(u.cancel());
    assert!(!u.commit_install(&first));
    assert_eq!(*events.lock().unwrap(), [r#"{"type":"cancelled"}"#]);
    // Committed: cancel (and so quitting) leaves the installation alone.
    let second = Arc::new(());
    *u.install.lock().unwrap() = Some(slot(&second));
    assert!(
        !u.commit_install(&first),
        "another start's task never commits"
    );
    assert!(u.commit_install(&second));
    assert!(!u.cancel());
    // Quitting waits (here briefly) for an installation that never ends,
    // then gives up without touching it.
    u.shutdown_waiting(Duration::from_millis(50));
    assert_eq!(events.lock().unwrap().len(), 1, "no second cancelled event");
    let left = u.install.lock().unwrap().take().expect("the slot stays");
    assert!(left.committed && !left.task.inner().is_finished());
    left.task.abort();
}

/// Quitting while the plugin replaces the app in place (macOS, Linux)
/// waits for it to finish instead of ending the process in the middle;
/// on Windows the installation ends the process itself and calls the
/// shutdown from inside the task, so nothing waits there. A download that
/// has not reached the commit point is cancelled, not waited for.
#[tokio::test(flavor = "multi_thread")]
async fn quitting_waits_for_an_installation_that_has_begun() {
    const INSTALLING: Duration = Duration::from_millis(300);
    let u = Updates::new(None, Ok(Box::new(Memory::default())));
    let slot = |committed: bool| {
        let (alive, ended) = std::sync::mpsc::channel::<()>();
        InstallSlot {
            task: tauri::async_runtime::spawn(async move {
                let _alive = alive;
                tokio::time::sleep(INSTALLING).await;
            }),
            sink: Channel::new(|_| Ok(())),
            token: Arc::new(()),
            committed,
            ended: Some(ended),
        }
    };
    *u.install.lock().unwrap() = Some(slot(true));
    let started = std::time::Instant::now();
    // shutdown_waiting blocks: off the runtime's worker.
    tokio::task::block_in_place(|| u.shutdown_waiting(Duration::from_secs(20)));
    let waited = started.elapsed();
    let finished = u
        .install
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .task
        .inner()
        .is_finished();
    if cfg!(windows) {
        assert!(
            waited < INSTALLING && !finished,
            "no wait on Windows: {waited:?}"
        );
    } else {
        assert!(
            waited >= INSTALLING,
            "returned after {waited:?}, before the installation ended"
        );
        assert!(waited < Duration::from_secs(10), "waited {waited:?}");
        assert!(finished);
    }
    // Before the commit point: cancelled at once, never waited for.
    *u.install.lock().unwrap() = Some(slot(false));
    let started = std::time::Instant::now();
    tokio::task::block_in_place(|| u.shutdown_waiting(Duration::from_secs(20)));
    assert!(
        started.elapsed() < INSTALLING,
        "a download is cancelled, not awaited"
    );
    assert!(
        u.install.lock().unwrap().is_none(),
        "the cancelled slot is gone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_can_be_cancelled_and_one_runs_at_a_time() {
    let srv = server().await;
    let k = keys();
    srv.publish(&k, "0.2.0");
    srv.served.lock().unwrap().stall = true;
    let app = mock_app();
    let u = Updates::new(Some(srv.source(&k)), Ok(Box::new(Memory::default())));
    // Nothing checked yet: nothing to install.
    let none = |_: tauri::ipc::InvokeResponseBody| Ok(());
    let e = u
        .start_install(app.handle().clone(), Channel::new(none))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);

    u.check(&v("0.1.0")).await.unwrap().unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&events);
    let sink = Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(j) = body {
            log.lock().unwrap().push(j);
        }
        Ok(())
    });
    u.start_install(app.handle().clone(), sink).unwrap();
    // The download is waiting on the stalled server.
    for _ in 0..200 {
        if srv.artifact_requested.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(srv.artifact_requested.load(Ordering::SeqCst));
    let e = u
        .start_install(app.handle().clone(), Channel::new(none))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    assert!(u.cancel());
    assert!(!u.cancel());
    let got = events.lock().unwrap().clone();
    assert_eq!(
        got.last().map(String::as_str),
        Some(r#"{"type":"cancelled"}"#)
    );
    assert!(!got.iter().any(|e| e.contains("installing")));
    // Afterwards a new attempt may start.
    srv.served.lock().unwrap().stall = false;
}

#[test]
fn an_idle_app_blocks_nothing_and_the_state_survives_a_restart() {
    let app = mock_app();
    app.manage(Arc::new(crate::cloud::upload::CloudUploads::new()));
    app.manage(Arc::new(crate::cloud::download::CloudDownloads::new()));
    assert!(blocked_by(app.handle()).is_empty());

    let dir = crate::cloud::test_server::TempDir::new("update-state");
    let path = dir.path(crate::storage::UPDATES_FILE_NAME);
    let open = || {
        crate::storage::SqliteUpdateState::open(&path).map(|s| Box::new(s) as Box<dyn StateStore>)
    };
    let u = Updates::new(None, open());
    u.save(&Saved {
        floor: Some("0.3.0".into()),
        last_check_ms: Some(5),
        postponed: None,
    });
    drop(u);
    let u = Updates::new(None, open());
    assert_eq!(u.saved().floor.as_deref(), Some("0.3.0"));
    // An unusable store leaves updates working, without a floor.
    let broken = Updates::new(None, Err(crate::features::StoreError::new("broken")));
    assert_eq!(broken.saved(), Saved::default());
}

// ---------------------------------------------------------------- fixture

/// The manifest and installers in scripts/fixtures/update/ were signed by
/// the real `tauri signer sign --app-version 0.2.0` with a throwaway key
/// (its private half deleted); scripts/update-manifest.test.mjs verifies the
/// same files. sotyware must accept what the official signer produces.
#[test]
fn a_manifest_signed_by_the_tauri_cli_verifies() {
    const PUBKEY: &str = include_str!("../../../scripts/fixtures/update/test-key.pub");
    const MANIFEST: &[u8] = include_bytes!("../../../scripts/fixtures/update/sotyware-update.json");
    const SIG: &str = include_str!("../../../scripts/fixtures/update/sotyware-update.json.sig");
    const NSIS: &[u8] =
        include_bytes!("../../../scripts/fixtures/update/sotyware_0.2.0_x64-setup.exe");
    const MSI: &[u8] =
        include_bytes!("../../../scripts/fixtures/update/sotyware_0.2.0_x64_en-US.msi");
    let pubkey = PUBKEY.trim();
    for (target, bytes) in [("windows-x86_64-nsis", NSIS), ("windows-x86_64-msi", MSI)] {
        let offer = verify_manifest(
            MANIFEST,
            SIG,
            pubkey,
            "stable",
            &v("0.1.0"),
            None,
            &[target.to_owned(), "windows-x86_64".to_owned()],
            false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(offer.target, target);
        assert!(
            offer
                .artifact
                .url
                .starts_with("https://updates.example.invalid/0.2.0/")
        );
        // The package signature verifies and names the version, as the
        // plugin requires; the hash is the manifest's.
        verify_signature(bytes, &offer.artifact.signature, pubkey).unwrap();
        let text = String::from_utf8(B64.decode(&offer.artifact.signature).unwrap()).unwrap();
        assert!(text.contains("\tversion:0.2.0\n"), "{text}");
        assert_eq!(check_sha256(bytes, &offer.artifact.sha256), Ok(()));
    }
    // The fixture says it is not a release; it was never offered anywhere.
    let m: Manifest = serde_json::from_slice(MANIFEST).unwrap();
    assert_eq!(
        m.notes.as_deref(),
        Some("Test fixture: not a real release.")
    );
}

// ---------------------------------------------------------------- GitHub

/// What GitHub does for `releases/latest/download/<asset>` (ADR-036): a
/// redirect to the tag's download URL, then one to the asset store, where
/// the bytes are. `published` is what the latest release holds; `before`
/// is served once as the signature to imitate a release published between
/// the manifest and the signature request.
#[derive(Default)]
struct Releases {
    manifest: Vec<u8>,
    sig: String,
    before: Option<String>,
    manifest_fetches: usize,
}

#[derive(Clone)]
struct Gh(Arc<Mutex<Releases>>, String);

fn found(to: String) -> axum::response::Response {
    (StatusCode::FOUND, [(axum::http::header::LOCATION, to)]).into_response()
}

async fn gh_latest(
    AxState(g): AxState<Gh>,
    axum::extract::Path(asset): axum::extract::Path<String>,
) -> axum::response::Response {
    found(format!(
        "{}/owner/repo/releases/download/v0.2.0/{asset}",
        g.1
    ))
}

async fn gh_tag(
    AxState(g): AxState<Gh>,
    axum::extract::Path((_tag, asset)): axum::extract::Path<(String, String)>,
) -> axum::response::Response {
    // The asset store: another path (another host at GitHub), signed query.
    found(format!("{}/release-assets/{asset}?sig=x", g.1))
}

async fn gh_asset(
    AxState(g): AxState<Gh>,
    axum::extract::Path(asset): axum::extract::Path<String>,
) -> axum::response::Response {
    let mut r = g.0.lock().unwrap();
    match asset.as_str() {
        "latest.json" => {
            r.manifest_fetches += 1;
            r.manifest.clone().into_response()
        }
        "latest.json.sig" => r
            .before
            .take()
            .unwrap_or_else(|| r.sig.clone())
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn gh_loop(AxState(g): AxState<Gh>) -> axum::response::Response {
    found(format!("{}/loop/latest.json", g.1))
}

async fn gh_away() -> axum::response::Response {
    found("http://example.invalid/latest.json".into())
}

async fn github() -> (String, Arc<Mutex<Releases>>) {
    let releases = Arc::new(Mutex::new(Releases::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route(
            "/owner/repo/releases/latest/download/{asset}",
            get(gh_latest),
        )
        .route("/owner/repo/releases/download/{tag}/{asset}", get(gh_tag))
        .route("/release-assets/{asset}", get(gh_asset))
        .route("/loop/latest.json", get(gh_loop))
        .route("/loop/latest.json.sig", get(gh_loop))
        .route("/away/latest.json", get(gh_away))
        .with_state(Gh(Arc::clone(&releases), url.clone()));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, releases)
}

fn gh_source(url: &str, path: &str, k: &Keys) -> Source {
    Source {
        manifest_url: format!("{url}{path}"),
        pubkey: k.pubkey(),
        channel: "stable".into(),
    }
}

fn publish_on(releases: &Mutex<Releases>, k: &Keys, version: &str) {
    let target = targets()
        .into_iter()
        .next()
        .unwrap_or_else(|| TARGET.into());
    let m = manifest(
        version,
        json!({ target: entry(k, "https://github.com/owner/repo/releases/download/v0.2.0/sotyware_setup.exe", version) }),
    );
    let mut r = releases.lock().unwrap();
    r.sig = k.sign(&m, "latest.json");
    r.manifest = m;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_latest_release_is_found_through_githubs_redirects() {
    let (url, releases) = github().await;
    let k = keys();
    publish_on(&releases, &k, "0.2.0");
    let u = Updates::new(
        Some(gh_source(
            &url,
            "/owner/repo/releases/latest/download/latest.json",
            &k,
        )),
        Ok(Box::new(Memory::default())),
    );
    let offer = u.check(&v("0.1.0")).await.unwrap().unwrap();
    assert_eq!(offer.manifest.version, "0.2.0");
    assert_eq!(releases.lock().unwrap().manifest_fetches, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_published_between_the_two_requests_is_fetched_again_once() {
    let (url, releases) = github().await;
    let k = keys();
    publish_on(&releases, &k, "0.2.0");
    // The signature request lands on the previous release: once.
    releases.lock().unwrap().before = Some(k.sign(b"the previous manifest", "latest.json"));
    let u = Updates::new(
        Some(gh_source(
            &url,
            "/owner/repo/releases/latest/download/latest.json",
            &k,
        )),
        Ok(Box::new(Memory::default())),
    );
    assert!(u.check(&v("0.1.0")).await.unwrap().is_some());
    assert_eq!(releases.lock().unwrap().manifest_fetches, 2);
    // A signature that never matches is still refused, after one retry.
    releases.lock().unwrap().sig = keys().sign(b"x", "latest.json");
    releases.lock().unwrap().manifest_fetches = 0;
    assert_eq!(u.check(&v("0.1.0")).await, Err(UpdateError::Signature));
    assert_eq!(releases.lock().unwrap().manifest_fetches, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn redirects_are_bounded_and_never_leave_https() {
    let (url, _releases) = github().await;
    let k = keys();
    let looping = Updates::new(
        Some(gh_source(&url, "/loop/latest.json", &k)),
        Ok(Box::new(Memory::default())),
    );
    let e = looping.check(&v("0.1.0")).await.unwrap_err();
    assert!(matches!(&e, UpdateError::Network(_)), "{e:?}");
    // Loopback HTTP is allowed only because this is a test build; a
    // redirect to any other plain-HTTP address is refused even here.
    let away = Updates::new(
        Some(gh_source(&url, "/away/latest.json", &k)),
        Ok(Box::new(Memory::default())),
    );
    let e = away.check(&v("0.1.0")).await.unwrap_err();
    assert!(matches!(&e, UpdateError::Network(_)), "{e:?}");
    assert_eq!(e.kind(), "network");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_published_release_is_reported_as_such() {
    let (url, _releases) = github().await;
    let k = keys();
    // No asset at all (no release yet, or releases not public).
    let u = Updates::new(
        Some(gh_source(
            &url,
            "/owner/repo/releases/download/v0.2.0/missing.json",
            &k,
        )),
        Ok(Box::new(Memory::default())),
    );
    let e = u.check(&v("0.1.0")).await.unwrap_err();
    assert!(e.to_string().contains("HTTP 404"), "{e}");
    assert_eq!(
        u.status(&v("0.1.0"), Vec::new()).error_kind,
        Some("network")
    );
}

/// scripts/fixtures/update-github/ holds a GitHub release's `latest.json`
/// for all seven platforms, made by the release workflow's own commands with
/// the real Tauri CLI and a throwaway key (its private half deleted). Every
/// installation kind finds its own entry, which verifies, and the package's
/// SHA-256 and minisign signature (with its signed version) match.
#[test]
fn every_platform_of_a_github_release_manifest_verifies() {
    macro_rules! fixture {
        ($name:literal) => {
            include_bytes!(concat!("../../../scripts/fixtures/update-github/", $name)).as_slice()
        };
    }
    let pubkey = std::str::from_utf8(fixture!("test-key.pub"))
        .unwrap()
        .trim();
    let manifest = fixture!("latest.json");
    let sig = std::str::from_utf8(fixture!("latest.json.sig")).unwrap();
    let packages: [(&str, &[u8]); 7] = [
        (
            "windows-x86_64-nsis",
            fixture!("sotyware_0.2.0_x64-setup.exe"),
        ),
        (
            "windows-x86_64-msi",
            fixture!("sotyware_0.2.0_x64_en-US.msi"),
        ),
        (
            "darwin-aarch64-app",
            fixture!("sotyware_0.2.0_aarch64.app.tar.gz"),
        ),
        (
            "darwin-x86_64-app",
            fixture!("sotyware_0.2.0_x64.app.tar.gz"),
        ),
        (
            "linux-x86_64-appimage",
            fixture!("sotyware_0.2.0_amd64.AppImage"),
        ),
        ("linux-x86_64-deb", fixture!("sotyware_0.2.0_amd64.deb")),
        ("linux-x86_64-rpm", fixture!("sotyware-0.2.0-1.x86_64.rpm")),
    ];
    for (target, bytes) in packages {
        let offer = verify_manifest(
            manifest,
            sig,
            pubkey,
            "stable",
            &v("0.1.0"),
            None,
            &[target.to_owned()],
            false,
        )
        .unwrap()
        .unwrap_or_else(|| panic!("{target}: no offer"));
        assert_eq!(offer.target, target);
        assert!(
            offer
                .artifact
                .url
                .starts_with("https://github.com/owner/repo/releases/download/v0.2.0/"),
            "{}",
            offer.artifact.url
        );
        check_sha256(bytes, &offer.artifact.sha256).unwrap();
        verify_signature(bytes, &offer.artifact.signature, pubkey).unwrap();
    }
    // An installation kind the release does not carry gets nothing.
    assert!(matches!(
        verify_manifest(
            manifest,
            sig,
            pubkey,
            "stable",
            &v("0.1.0"),
            None,
            &["linux-aarch64-appimage".to_owned()],
            false
        ),
        Err(UpdateError::Platform(_))
    ));
}

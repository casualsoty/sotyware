//! Application updates (ADR-033), driven from Rust only.
//!
//! A release publishes `sotyware-update.json` (the manifest) and
//! `sotyware-update.json.sig` (its minisign signature, same key family as
//! the artifacts). sotyware:
//!
//! 1. fetches both (HTTPS only; loopback HTTP in test builds),
//! 2. verifies the manifest's signature with the compiled-in public key,
//! 3. checks its format, channel, and that its version is newer than the
//!    running one and not lower than the highest version ever offered to
//!    this installation (the version floor, stored locally),
//! 4. picks this installation's platform entry (`windows-x86_64-nsis` /
//!    `-msi`, the same names the updater plugin uses), which must carry an
//!    HTTPS URL, a signature and a SHA-256,
//! 5. hands exactly that verified JSON to `tauri-plugin-updater`
//!    (`restore_update`, no second fetch), which requires the artifact's
//!    signature to name the same version,
//! 6. downloads through the plugin (which verifies the artifact's minisign
//!    signature), checks the SHA-256 from the signed manifest, and only then
//!    installs, after the user clicked and while nothing else runs.
//!
//! No production key or endpoint exists yet ([`PRODUCTION`] is `None`):
//! release builds report updates as not configured. Test-profile and
//! debug builds read `SOTYWARE_UPDATE_ENDPOINT` and
//! `SOTYWARE_UPDATE_PUBKEY`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::account::StateStore;
use crate::cloud::dashboard::{CommandError, ErrorKind};
use crate::journal::now_ms;

/// Where updates come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The manifest URL; the signature is at the same URL + `.sig`.
    pub manifest_url: String,
    /// The minisign public key, base64 of its text (as `tauri signer
    /// generate` writes it).
    pub pubkey: String,
    pub channel: String,
}

/// The production update source. None exists yet: the signing key and the
/// release endpoint are external gates (docs/development/release.md).
pub const PRODUCTION: Option<(&str, &str)> = None;

/// How often a running sotyware checks.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// The first check after startup.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The whole download of one update may take this long.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// `UpdaterBuilder::build` insists on an endpoint, but sotyware never lets
/// the plugin fetch one: it hands over the verified manifest through
/// `restore_update`. A reserved `.invalid` name (RFC 2606) that can never
/// resolve.
/// The largest manifest or signature accepted.
pub const MAX_MANIFEST: usize = 256 * 1024;

const NEVER_FETCHED: &str = "https://plugin-endpoint.invalid/";

/// The link the `update-check` built-in feature uses to start and stop the
/// automatic checks; set once in `run()`.
pub static CHECK_LINK: OnceLock<(Arc<Updates>, Version)> = OnceLock::new();

pub fn configured_source() -> Option<Source> {
    #[cfg(any(debug_assertions, feature = "test-profile"))]
    if let (Ok(url), Ok(pk)) = (
        std::env::var("SOTYWARE_UPDATE_ENDPOINT"),
        std::env::var("SOTYWARE_UPDATE_PUBKEY"),
    ) && !url.is_empty()
        && !pk.is_empty()
    {
        return Some(Source {
            manifest_url: url,
            pubkey: pk,
            channel: "stable".into(),
        });
    }
    PRODUCTION.map(|(url, pk)| Source {
        manifest_url: url.into(),
        pubkey: pk.into(),
        channel: "stable".into(),
    })
}

/// Why an update was refused. Never contains a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    Network(String),
    /// The manifest's signature does not verify with sotyware's key.
    Signature,
    Malformed(String),
    WrongChannel {
        got: String,
        want: String,
    },
    /// Lower than a version offered before: a replayed or rolled-back
    /// manifest.
    Downgrade {
        offered: String,
        floor: String,
    },
    /// No entry for this installation, or not one sotyware accepts.
    Platform(String),
    /// The downloaded artifact does not match the signed manifest.
    Integrity(String),
    /// The plugin refused the update or its installation.
    Updater(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(m) => write!(f, "the update server could not be reached: {m}"),
            Self::Signature => f.write_str("the update information is not signed by sotyware's release key; nothing was installed"),
            Self::Malformed(m) => write!(f, "the update information is not valid: {m}"),
            Self::WrongChannel { got, want } => write!(f, "the update is for the {got} channel, this installation follows {want}"),
            Self::Downgrade { offered, floor } => write!(f, "the update server offers {offered}, older than {floor} which it offered before; refused as a possible replay"),
            Self::Platform(m) => write!(f, "{m}"),
            Self::Integrity(m) => write!(f, "the downloaded update does not match its signed description: {m}"),
            Self::Updater(m) => write!(f, "the update could not be installed: {m}"),
        }
    }
}

/// One platform's artifact in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub signature: String,
    pub sha256: String,
}

/// The signed manifest. A superset of the updater plugin's release format
/// (`version`, `notes`, `pub_date`, `platforms`), so the plugin reads the
/// same JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub channel: String,
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub pub_date: Option<String>,
    pub platforms: BTreeMap<String, Artifact>,
}

/// A verified offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub manifest: Manifest,
    /// The platform key chosen.
    pub target: String,
    pub artifact: Artifact,
    /// Exactly the verified JSON, for the plugin.
    pub raw: serde_json::Value,
}

fn text_of(b64_or_text: &str) -> Result<String, UpdateError> {
    let t = b64_or_text.trim();
    if t.starts_with("untrusted comment:") {
        return Ok(t.to_owned());
    }
    let bytes = B64
        .decode(t)
        .map_err(|_| UpdateError::Malformed("key or signature encoding".into()))?;
    String::from_utf8(bytes).map_err(|_| UpdateError::Malformed("key or signature encoding".into()))
}

/// Verifies a minisign signature (text or base64 of it, as Tauri writes
/// `.sig` files) over `data` with `pubkey` (base64 of the key's text).
pub fn verify_signature(data: &[u8], signature: &str, pubkey: &str) -> Result<(), UpdateError> {
    let pk = minisign_verify::PublicKey::decode(&text_of(pubkey)?)
        .map_err(|_| UpdateError::Malformed("public key".into()))?;
    let sig = minisign_verify::Signature::decode(&text_of(signature)?)
        .map_err(|_| UpdateError::Signature)?;
    pk.verify(data, &sig, false)
        .map_err(|_| UpdateError::Signature)
}

fn is_loopback_http(url: &str) -> bool {
    url.strip_prefix("http://")
        .and_then(|r| r.split(['/', ':']).next())
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[")
}

/// Whether an artifact or manifest URL is acceptable: HTTPS, or loopback
/// HTTP when `allow_loopback` (test builds only).
pub fn url_ok(url: &str, allow_loopback: bool) -> bool {
    url.starts_with("https://") || (allow_loopback && is_loopback_http(url))
}

/// Steps 2–4: the verified offer for this installation, `None` when it is
/// up to date.
#[allow(clippy::too_many_arguments)]
pub fn verify_manifest(
    bytes: &[u8],
    signature: &str,
    pubkey: &str,
    channel: &str,
    current: &Version,
    floor: Option<&Version>,
    targets: &[String],
    allow_loopback: bool,
) -> Result<Option<Offer>, UpdateError> {
    verify_signature(bytes, signature, pubkey)?;
    let raw: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| UpdateError::Malformed(e.to_string()))?;
    let manifest: Manifest =
        serde_json::from_value(raw.clone()).map_err(|e| UpdateError::Malformed(e.to_string()))?;
    if manifest.format != "sotyware-update" || manifest.format_version != 1 {
        return Err(UpdateError::Malformed(format!(
            "{} version {}",
            manifest.format, manifest.format_version
        )));
    }
    if manifest.channel != channel {
        return Err(UpdateError::WrongChannel {
            got: manifest.channel,
            want: channel.into(),
        });
    }
    let offered = Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|e| UpdateError::Malformed(format!("version: {e}")))?;
    if let Some(floor) = floor
        && offered < *floor
    {
        return Err(UpdateError::Downgrade {
            offered: offered.to_string(),
            floor: floor.to_string(),
        });
    }
    if offered <= *current {
        return Ok(None);
    }
    let (target, artifact) = targets
        .iter()
        .find_map(|t| manifest.platforms.get(t).map(|a| (t.clone(), a.clone())))
        .ok_or_else(|| {
            UpdateError::Platform(format!(
                "this release has no package for this installation ({})",
                targets.join(", ")
            ))
        })?;
    if !url_ok(&artifact.url, allow_loopback) {
        return Err(UpdateError::Platform(
            "the package is not offered over HTTPS".into(),
        ));
    }
    if artifact.sha256.len() != 64
        || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || artifact.signature.is_empty()
    {
        return Err(UpdateError::Malformed(
            "the package entry needs a signature and a SHA-256".into(),
        ));
    }
    Ok(Some(Offer {
        manifest,
        target,
        artifact,
        raw,
    }))
}

/// The platform keys this installation accepts, most specific first
/// (`{os}-{arch}-{installer}`, then `{os}-{arch}`), as the plugin does.
pub fn targets() -> Vec<String> {
    let Some(base) = tauri_plugin_updater::target() else {
        return Vec::new();
    };
    let installer = match tauri::utils::platform::bundle_type() {
        Some(tauri::utils::config::BundleType::Nsis) => Some("nsis"),
        Some(tauri::utils::config::BundleType::Msi) => Some("msi"),
        Some(tauri::utils::config::BundleType::AppImage) => Some("appimage"),
        Some(tauri::utils::config::BundleType::Deb) => Some("deb"),
        Some(tauri::utils::config::BundleType::Rpm) => Some("rpm"),
        _ => None,
    };
    match installer {
        Some(i) => vec![format!("{base}-{i}"), base],
        None => vec![base],
    }
}

/// The plugin's view of the release must be exactly the verified offer.
pub fn same_release(
    offer: &Offer,
    version: &str,
    url: &str,
    signature: &str,
) -> Result<(), UpdateError> {
    let v = Version::parse(version.trim_start_matches('v')).ok();
    let o = Version::parse(offer.manifest.version.trim_start_matches('v')).ok();
    if v.is_none() || v != o || url != offer.artifact.url || signature != offer.artifact.signature {
        return Err(UpdateError::Updater(
            "the updater's release differs from the verified manifest".into(),
        ));
    }
    Ok(())
}

/// The SHA-256 the signed manifest names must be the downloaded bytes'.
pub fn check_sha256(bytes: &[u8], expected: &str) -> Result<(), UpdateError> {
    let got: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(UpdateError::Integrity(format!(
            "SHA-256 {got}, expected {expected}"
        )))
    }
}

// ------------------------------------------------------------------ state

/// What is kept between runs (no secret).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    /// The highest version ever offered (verified) to this installation.
    pub floor: Option<String>,
    pub last_check_ms: Option<i64>,
    /// A version the user chose to postpone (not shown again until a newer
    /// one or a manual check).
    pub postponed: Option<String>,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub configured: bool,
    pub current: String,
    pub channel: String,
    pub last_check_ms: Option<i64>,
    pub available: Option<Available>,
    pub postponed: bool,
    pub error: Option<String>,
    /// Why "Update now" is not possible right now (transfers running…).
    pub blocked_by: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum InstallEvent {
    Progress {
        downloaded: u64,
        total: Option<u64>,
    },
    Verified,
    Installing,
    /// Installed without ending the process (not Windows): it takes effect
    /// when sotyware starts again.
    Installed,
    Failed {
        message: String,
    },
    Cancelled,
}

/// Updates: managed state.
pub struct Updates {
    source: Option<Source>,
    state: Mutex<Result<Box<dyn StateStore>, String>>,
    last: Mutex<(Option<Offer>, Option<String>)>,
    /// The running download-and-install task and its channel.
    install: Mutex<Option<(tauri::async_runtime::JoinHandle<()>, Channel<InstallEvent>)>>,
    timer: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    allow_loopback: bool,
}

impl Updates {
    pub fn new(
        source: Option<Source>,
        state: Result<Box<dyn StateStore>, crate::features::StoreError>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            state: Mutex::new(state.map_err(|e| e.to_string())),
            last: Mutex::new((None, None)),
            install: Mutex::new(None),
            timer: Mutex::new(None),
            allow_loopback: cfg!(any(debug_assertions, feature = "test-profile", test)),
        })
    }

    fn saved(&self) -> Saved {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.as_mut()
            .ok()
            .and_then(|s| s.load().ok().flatten())
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default()
    }

    fn save(&self, s: &Saved) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(store) = g.as_mut()
            && let Err(e) = store.save(&serde_json::to_string(s).expect("serializes"))
        {
            eprintln!("sotyware: cannot save the update state: {e}");
        }
    }

    fn last(&self) -> MutexGuard<'_, (Option<Offer>, Option<String>)> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn current(app_version: &str) -> Version {
        Version::parse(app_version).unwrap_or_else(|_| Version::new(0, 0, 0))
    }

    pub fn status(&self, current: &Version, blocked_by: Vec<String>) -> UpdateStatus {
        let saved = self.saved();
        let (offer, error) = self.last().clone();
        UpdateStatus {
            configured: self.source.is_some(),
            current: current.to_string(),
            channel: self
                .source
                .as_ref()
                .map_or_else(|| "stable".into(), |s| s.channel.clone()),
            last_check_ms: saved.last_check_ms,
            postponed: offer
                .as_ref()
                .is_some_and(|o| saved.postponed.as_deref() == Some(o.manifest.version.as_str())),
            available: offer.map(|o| Available {
                version: o.manifest.version,
                notes: o.manifest.notes,
                pub_date: o.manifest.pub_date,
            }),
            error,
            blocked_by,
        }
    }

    /// Fetches and verifies the manifest; remembers the offer.
    pub async fn check(&self, current: &Version) -> Result<Option<Offer>, UpdateError> {
        let Some(src) = &self.source else {
            return Ok(None);
        };
        if !url_ok(&src.manifest_url, self.allow_loopback) {
            return Err(UpdateError::Platform(
                "the update server must be reached over HTTPS".into(),
            ));
        }
        // The manifest is answered directly (no redirect), over HTTPS only
        // outside test builds.
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(
                crate::account::client::tls_config().map_err(UpdateError::Network)?,
            )
            .https_only(!self.allow_loopback)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("sotyware/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| UpdateError::Network(e.to_string()))?;
        let fetch = |url: String| {
            let client = &client;
            async move {
                let r = client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| UpdateError::Network(e.without_url().to_string()))?;
                if !r.status().is_success() {
                    return Err(UpdateError::Network(format!(
                        "HTTP {}",
                        r.status().as_u16()
                    )));
                }
                // A manifest is a few kilobytes; refuse anything absurd.
                let b = r
                    .bytes()
                    .await
                    .map_err(|e| UpdateError::Network(e.without_url().to_string()))?;
                if b.len() > MAX_MANIFEST {
                    return Err(UpdateError::Malformed(
                        "the update information is too large".into(),
                    ));
                }
                Ok(b.to_vec())
            }
        };
        let result: Result<Option<Offer>, UpdateError> = async {
            let manifest = fetch(src.manifest_url.clone()).await?;
            let sig = fetch(format!("{}.sig", src.manifest_url)).await?;
            let mut saved = self.saved();
            let floor = saved.floor.as_deref().and_then(|f| Version::parse(f).ok());
            let offer = verify_manifest(
                &manifest,
                std::str::from_utf8(&sig).map_err(|_| UpdateError::Signature)?,
                &src.pubkey,
                &src.channel,
                current,
                floor.as_ref(),
                &targets(),
                self.allow_loopback,
            )?;
            // The floor rises with every verified manifest, also one that
            // is not newer than the running version.
            let offered: Manifest = serde_json::from_slice(&manifest)
                .map_err(|e| UpdateError::Malformed(e.to_string()))?;
            if let Ok(v) = Version::parse(offered.version.trim_start_matches('v'))
                && floor.as_ref().is_none_or(|f| v > *f)
            {
                saved.floor = Some(v.to_string());
            }
            saved.last_check_ms = Some(now_ms());
            self.save(&saved);
            Ok(offer)
        }
        .await;
        let mut last = self.last();
        match &result {
            Ok(o) => *last = (o.clone(), None),
            Err(e) => last.1 = Some(e.to_string()),
        }
        result
    }

    pub fn postpone(&self) {
        if let Some(o) = &self.last().0 {
            let mut s = self.saved();
            s.postponed = Some(o.manifest.version.clone());
            self.save(&s);
        }
    }

    /// Stops a download in progress; nothing was installed. Too late once
    /// the installer has started (the process then ends).
    pub fn cancel(&self) -> bool {
        match self
            .install
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            Some((h, sink)) if !h.inner().is_finished() => {
                h.abort();
                let _ = sink.send(InstallEvent::Cancelled);
                true
            }
            _ => false,
        }
    }

    /// Steps 5–6 for a verified `offer`: the plugin's update from exactly
    /// that JSON, downloaded (artifact signature checked by the plugin),
    /// the SHA-256 checked, then installed. `install` false stops before
    /// installing (tests).
    pub async fn download_verified<R: Runtime>(
        app: &AppHandle<R>,
        pubkey: &str,
        offer: &Offer,
        https_only: bool,
        mut progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<(tauri_plugin_updater::Update, Vec<u8>), UpdateError> {
        use tauri_plugin_updater::UpdaterExt;
        let tls = crate::account::client::tls_config().map_err(UpdateError::Network)?;
        let exiting = app.clone();
        let updater = app
            .updater_builder()
            .pubkey(pubkey)
            .target(offer.target.clone())
            .endpoints(vec![NEVER_FETCHED.parse().expect("a valid URL")])
            .map_err(|e| UpdateError::Updater(e.to_string()))?
            .timeout(DOWNLOAD_TIMEOUT)
            // sotyware's TLS stack (ring, the system's roots), HTTPS only
            // outside test builds; redirects stay at reqwest's default.
            .configure_client(move |b| b.use_preconfigured_tls(tls.clone()).https_only(https_only))
            // Windows: the plugin starts the installer and ends the process
            // itself; release what an ordinary exit would first.
            .on_before_exit(move || crate::release_resources(&exiting))
            .build()
            .map_err(|e| UpdateError::Updater(e.to_string()))?;
        let update = updater
            .restore_update(offer.raw.clone())
            .map_err(|e| UpdateError::Updater(e.to_string()))?
            .ok_or_else(|| {
                UpdateError::Updater("the release is not newer than this version".into())
            })?;
        same_release(
            offer,
            &update.version,
            update.download_url.as_str(),
            &update.signature,
        )?;
        let mut done = 0u64;
        let bytes = update
            .download(
                |chunk, total| {
                    done += chunk as u64;
                    progress(done, total);
                },
                || {},
            )
            .await
            .map_err(|e| match e {
                tauri_plugin_updater::Error::Minisign(_)
                | tauri_plugin_updater::Error::SignedVersionMismatch { .. }
                | tauri_plugin_updater::Error::MissingSignedVersion => {
                    UpdateError::Integrity(e.to_string())
                }
                other => UpdateError::Network(other.to_string()),
            })?;
        check_sha256(&bytes, &offer.artifact.sha256)?;
        Ok((update, bytes))
    }

    /// The whole installation, in its own task (cancellable until it
    /// installs).
    pub fn start_install<R: Runtime>(
        self: &Arc<Self>,
        app: AppHandle<R>,
        sink: Channel<InstallEvent>,
    ) -> Result<(), CommandError> {
        let offer = self.last().0.clone().ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                "no update is available; check first",
            )
        })?;
        let pubkey = self
            .source
            .as_ref()
            .map(|s| s.pubkey.clone())
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidConfiguration,
                    "updates are not configured",
                )
            })?;
        let mut guard = self.install.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|(h, _)| !h.inner().is_finished())
        {
            return Err(CommandError::new(
                ErrorKind::Busy,
                "an update is already being downloaded",
            ));
        }
        let sink2 = sink.clone();
        let kept = sink.clone();
        let allow_loopback = self.allow_loopback;
        let task = tauri::async_runtime::spawn(async move {
            let mut last = std::time::Instant::now();
            let result = Self::download_verified(&app, &pubkey, &offer, !allow_loopback, |d, t| {
                if last.elapsed() >= Duration::from_millis(250) {
                    last = std::time::Instant::now();
                    let _ = sink2.send(InstallEvent::Progress {
                        downloaded: d,
                        total: t,
                    });
                }
            })
            .await;
            match result {
                Ok((update, bytes)) => {
                    let _ = sink.send(InstallEvent::Verified);
                    // Last chance: nothing may have started meanwhile.
                    if !blocked_by(&app).is_empty() {
                        let _ = sink.send(InstallEvent::Failed { message: "a transfer, file operation or backup started; the update was not installed".into() });
                        return;
                    }
                    let _ = sink.send(InstallEvent::Installing);
                    // Windows: the plugin starts the installer and ends the
                    // process (after `release_resources`); the installer
                    // starts the new version. Elsewhere it returns, and the
                    // new version runs from the next start: a restart from
                    // here would meet this process's single-instance lock.
                    match update.install(bytes) {
                        Ok(()) => {
                            let _ = sink.send(InstallEvent::Installed);
                        }
                        Err(e) => {
                            let _ = sink.send(InstallEvent::Failed {
                                message: UpdateError::Updater(e.to_string()).to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = sink.send(InstallEvent::Failed {
                        message: e.to_string(),
                    });
                }
            }
        });
        *guard = Some((task, kept));
        Ok(())
    }

    /// The automatic checks (the `update-check` feature): once after
    /// [`FIRST_CHECK`], then every [`CHECK_EVERY`] (one timer; no polling).
    /// Only when configured.
    pub fn start_checks(self: &Arc<Self>, current: Version) {
        if self.source.is_none() {
            return;
        }
        self.stop_checks();
        let this = Arc::clone(self);
        let h = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                if let Err(e) = this.check(&current).await {
                    eprintln!("sotyware: update check failed: {e}");
                }
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
        *self.timer.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    }

    pub fn stop_checks(&self) {
        if let Some(h) = self.timer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            h.abort();
        }
    }

    pub fn shutdown(&self) {
        self.stop_checks();
        self.cancel();
    }
}

/// What would be interrupted by installing now.
pub fn blocked_by<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |busy: bool, what: &str| {
        if busy {
            out.push(what.to_owned());
        }
    };
    if let Some(s) = app.try_state::<Arc<crate::cloud::upload::CloudUploads>>() {
        add(s.busy(), "an upload");
    }
    if let Some(s) = app.try_state::<Arc<crate::cloud::download::CloudDownloads>>() {
        add(s.busy(), "a download");
    }
    if let Some(s) = app.try_state::<Arc<crate::cloud::bulk_download::BulkDownloads>>() {
        add(s.busy(), "a download of several items");
    }
    if let Some(s) = app.try_state::<Arc<crate::cloud::operations::CloudOperations>>() {
        add(s.busy(), "a file operation");
    }
    if let Some(s) = app.try_state::<Arc<crate::backup::Backups>>() {
        add(s.busy(), "a backup task");
    }
    out
}

fn update_error(e: &UpdateError) -> CommandError {
    let kind = match e {
        UpdateError::Network(_) => ErrorKind::Network,
        UpdateError::Signature | UpdateError::Integrity(_) => ErrorKind::Integrity,
        _ => ErrorKind::Other,
    };
    CommandError::new(kind, e.to_string())
}

fn app_version<R: Runtime>(app: &AppHandle<R>) -> Version {
    Updates::current(&app.package_info().version.to_string())
}

#[tauri::command]
pub fn update_status(app: AppHandle, updates: State<'_, Arc<Updates>>) -> UpdateStatus {
    updates.status(&app_version(&app), blocked_by(&app))
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    updates: State<'_, Arc<Updates>>,
) -> Result<UpdateStatus, CommandError> {
    let current = app_version(&app);
    updates
        .check(&current)
        .await
        .map_err(|e| update_error(&e))?;
    Ok(updates.status(&current, blocked_by(&app)))
}

/// Downloads, verifies and installs the update the last check found.
/// Refused while a transfer, file operation or backup runs.
#[tauri::command]
pub fn install_update(
    app: AppHandle,
    updates: State<'_, Arc<Updates>>,
    on_event: Channel<InstallEvent>,
) -> Result<(), CommandError> {
    let blocked = blocked_by(&app);
    if !blocked.is_empty() {
        return Err(CommandError::new(
            ErrorKind::Busy,
            format!(
                "wait until {} has finished, or cancel it, then update",
                blocked.join(", ")
            ),
        ));
    }
    updates.start_install(app, on_event)
}

#[tauri::command]
pub fn cancel_update(updates: State<'_, Arc<Updates>>) -> bool {
    updates.cancel()
}

#[tauri::command]
pub fn postpone_update(updates: State<'_, Arc<Updates>>) {
    updates.postpone();
}

#[cfg(test)]
mod tests;

//! Test profiles (ADR-027): how a desktop test run gets an app that cannot
//! reach the user's own data.
//!
//! Compiled only into a build with the off-by-default Cargo feature
//! `test-profile` (and into unit tests). Such a build:
//!
//! - refuses to start (exit [`EXIT_REFUSED`]) unless [`ENV`] names an
//!   absolute, existing directory holding a [`MARKER_FILE`] the test driver
//!   wrote (format 1, purpose [`PURPOSE`], a run id and a 32-digit hex
//!   nonce). There is no fallback to the normal profile: a missing or
//!   invalid profile ends the process before any window, store or plugin
//!   state exists;
//! - refuses a profile directory that is, contains or lies inside one of
//!   the app's normal directories;
//! - runs under its own bundle identifier ([`IDENTIFIER`]), so it never
//!   shares a single-instance lock, a default data directory or a webview
//!   folder with the installed app;
//! - keeps its three SQLite databases in `<profile>/data` and its webview
//!   data in `<profile>/webview`;
//! - stores credentials only in the profile's [`CredentialNamespace`]
//!   (service `com.sotyware.desktop.test`, user names prefixed with the
//!   nonce); the app's own namespace is not compiled into this build;
//! - writes [`ACK_FILE`] into the profile once the stores are open: its
//!   pid, the nonce, the identifier, every path it uses and the credential
//!   prefix. The driver compares that with what it launched and set up
//!   before it touches the window.
//!
//! A normal build contains none of this and ignores [`ENV`].

// Unit tests compile this module without the feature, where only the tests
// use it.
#![cfg_attr(not(feature = "test-profile"), allow(dead_code))]

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cloud::keyring::CredentialNamespace;

/// The variable naming the profile directory.
pub const ENV: &str = "SOTYWARE_TEST_PROFILE";

/// Written by the driver into the profile directory before the launch.
pub const MARKER_FILE: &str = "sotyware-test-profile.json";

/// Written by the app into the profile directory once its stores are open.
pub const ACK_FILE: &str = "sotyware-test-profile-ack.json";

/// The `purpose` field of the marker and the acknowledgement.
pub const PURPOSE: &str = "sotyware-desktop-test-profile";

/// Present in every test-profile binary (it is written into the
/// acknowledgement), so scripts can tell such a build from a normal one.
pub const BUILD_MARKER: &str = "sotyware-test-profile-build";

/// The bundle identifier of a test-profile build.
pub const IDENTIFIER: &str = "com.sotyware.desktop.test";

/// Exit code when the profile is missing or refused.
pub const EXIT_REFUSED: i32 = 3;

/// The marker is a few dozen bytes; anything larger is not one.
const MARKER_MAX_BYTES: u64 = 4096;

/// Why a profile was refused. The message names the variable or file at
/// fault; it never holds file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse<T>(msg: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(msg.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    format: u32,
    purpose: String,
    run: String,
    nonce: String,
}

/// A validated profile.
#[derive(Debug, Clone)]
pub struct TestProfile {
    root: PathBuf,
    run: String,
    nonce: String,
    namespace: CredentialNamespace,
}

impl TestProfile {
    /// The profile directory, canonical.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the databases live.
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Where the webview keeps its data.
    pub fn webview_dir(&self) -> PathBuf {
        self.root.join("webview")
    }

    /// The only credential namespace this run may use.
    pub fn namespace(&self) -> &CredentialNamespace {
        &self.namespace
    }

    pub fn run(&self) -> &str {
        &self.run
    }

    #[cfg(test)]
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
}

fn valid_run(run: &str) -> bool {
    (1..=64).contains(&run.len())
        && run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Validates the value of [`ENV`] and the marker it points to.
pub fn from_env(value: Option<OsString>) -> Result<TestProfile, Refused> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return refuse(format!(
            "{ENV} is not set; a test-profile build never uses the normal profile"
        ));
    };
    let Some(text) = value.to_str() else {
        return refuse(format!("{ENV} is not valid UTF-8"));
    };
    let given = Path::new(text);
    if !given.is_absolute() {
        return refuse(format!("{ENV} must be an absolute path"));
    }
    let root = match fs::canonicalize(given) {
        Ok(root) => root,
        Err(e) => return refuse(format!("{ENV} cannot be resolved: {e}")),
    };
    if !root.is_dir() {
        return refuse(format!("{ENV} is not a directory"));
    }
    let marker_path = root.join(MARKER_FILE);
    let meta = match fs::symlink_metadata(&marker_path) {
        Ok(meta) => meta,
        Err(e) => return refuse(format!("no {MARKER_FILE} in the profile: {e}")),
    };
    if !meta.is_file() || meta.len() > MARKER_MAX_BYTES {
        return refuse(format!(
            "{MARKER_FILE} must be a regular file of at most {MARKER_MAX_BYTES} bytes"
        ));
    }
    let bytes = match fs::read(&marker_path) {
        Ok(bytes) => bytes,
        Err(e) => return refuse(format!("{MARKER_FILE} cannot be read: {e}")),
    };
    // A UTF-8 byte order mark is what Windows PowerShell 5.1 writes.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let marker: Marker = match serde_json::from_slice(bytes) {
        Ok(marker) => marker,
        Err(e) => return refuse(format!("{MARKER_FILE} is not a valid marker: {e}")),
    };
    if marker.format != 1 || marker.purpose != PURPOSE {
        return refuse(format!(
            "{MARKER_FILE} has the wrong format or purpose (want format 1, {PURPOSE})"
        ));
    }
    if !valid_run(&marker.run) {
        return refuse(format!(
            "{MARKER_FILE}: run must be 1 to 64 letters, digits, '-' or '_'"
        ));
    }
    let Some(namespace) = CredentialNamespace::isolated(&marker.nonce) else {
        return refuse(format!(
            "{MARKER_FILE}: nonce must be 32 lowercase hex digits"
        ));
    };
    Ok(TestProfile {
        root,
        run: marker.run,
        nonce: marker.nonce,
        namespace,
    })
}

/// `path` with every existing prefix resolved (symbolic links, `..`, case
/// on Windows), so two spellings of one place compare equal even when the
/// place does not exist yet. Below the deepest prefix that exists, a `..`
/// removes the folder before it: that folder does not exist, so it is not
/// a link whose `..` would lead elsewhere; past those folders it goes to
/// the resolved prefix's parent.
fn resolve(path: &Path) -> PathBuf {
    // Below that prefix, last first: a name, or `None` for a `..`.
    let mut rest: Vec<Option<OsString>> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(found) = fs::canonicalize(&cur) {
            let mut out = found;
            for part in rest.iter().rev() {
                match part {
                    Some(name) => out.push(name),
                    None => {
                        out.pop();
                    }
                }
            }
            return out;
        }
        let last = match cur.components().next_back() {
            Some(Component::Normal(name)) => Some(name.to_os_string()),
            Some(Component::ParentDir) => None,
            _ => return path.to_path_buf(),
        };
        let Some(parent) = cur.parent() else {
            return path.to_path_buf();
        };
        rest.push(last);
        cur = parent.to_path_buf();
    }
}

/// Component-wise prefix test; case-insensitive on Windows, where the file
/// system is.
fn is_within(path: &Path, base: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, Component::CurDir))
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) { s.to_lowercase() } else { s }
            })
            .collect()
    };
    let (p, b) = (norm(path), norm(base));
    p.len() >= b.len() && p[..b.len()] == b[..]
}

/// Refuses a profile that is, contains or lies inside any of `normal`, the
/// app's own directories.
pub fn check_separate(profile: &TestProfile, normal: &[PathBuf]) -> Result<(), Refused> {
    for dir in normal {
        let dir = resolve(dir);
        if is_within(&profile.root, &dir) || is_within(&dir, &profile.root) {
            return refuse(format!(
                "the profile {} overlaps the app's own directory {}",
                profile.root.display(),
                dir.display()
            ));
        }
    }
    Ok(())
}

/// What the app tells the driver about itself.
#[derive(Serialize)]
struct Ack<'a> {
    format: u32,
    purpose: &'static str,
    build: &'static str,
    run: &'a str,
    nonce: &'a str,
    pid: u32,
    identifier: &'static str,
    root: String,
    data_dir: String,
    webview_dir: String,
    credential_service: &'static str,
    credential_target_prefix: String,
    stores: Vec<String>,
}

fn shown(path: &Path) -> String {
    resolve(path).to_string_lossy().into_owned()
}

/// Writes [`ACK_FILE`] (to a temporary name, then renamed over any earlier
/// one, so the driver never reads half a file). `stores` are the database
/// files the app opened.
pub fn write_ack(profile: &TestProfile, stores: &[PathBuf]) -> std::io::Result<()> {
    let ack = Ack {
        format: 1,
        purpose: PURPOSE,
        build: BUILD_MARKER,
        run: &profile.run,
        nonce: &profile.nonce,
        pid: std::process::id(),
        identifier: IDENTIFIER,
        root: shown(&profile.root),
        data_dir: shown(&profile.data_dir()),
        webview_dir: shown(&profile.webview_dir()),
        credential_service: profile.namespace.service(),
        credential_target_prefix: profile.namespace.windows_target_prefix(),
        stores: stores.iter().map(|s| shown(s)).collect(),
    };
    let text = serde_json::to_vec_pretty(&ack).map_err(std::io::Error::other)?;
    let tmp = profile
        .root
        .join(format!("{ACK_FILE}.{}.tmp", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&text)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, profile.root.join(ACK_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system's temporary folder, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> std::io::Result<Self> {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "sotyware-profile-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn marker(dir: &Path, body: &str) {
        fs::write(dir.join(MARKER_FILE), body).unwrap();
    }

    fn good(dir: &Path) {
        marker(
            dir,
            &format!(r#"{{"format":1,"purpose":"{PURPOSE}","run":"r-1","nonce":"{NONCE}"}}"#),
        );
    }

    fn env(dir: &Path) -> Option<OsString> {
        Some(dir.as_os_str().to_owned())
    }

    #[test]
    fn a_valid_profile_names_its_dirs_and_an_isolated_namespace() {
        let tmp = TempDir::new().unwrap();
        good(tmp.path());
        let p = from_env(env(tmp.path())).unwrap();
        assert_eq!(p.root(), fs::canonicalize(tmp.path()).unwrap());
        assert_eq!(p.data_dir(), p.root().join("data"));
        assert_eq!(p.webview_dir(), p.root().join("webview"));
        assert_eq!(p.run(), "r-1");
        assert_eq!(p.nonce(), NONCE);
        assert_eq!(p.namespace().service(), crate::cloud::keyring::TEST_SERVICE);
        assert!(
            p.namespace()
                .windows_target("a")
                .starts_with(&format!("com.sotyware.desktop.test/{NONCE}/"))
        );
    }

    #[test]
    fn a_byte_order_mark_is_accepted() {
        let tmp = TempDir::new().unwrap();
        let body = format!(
            "\u{feff}{{\"format\":1,\"purpose\":\"{PURPOSE}\",\"run\":\"r\",\"nonce\":\"{NONCE}\"}}"
        );
        marker(tmp.path(), &body);
        assert!(from_env(env(tmp.path())).is_ok());
    }

    /// ADR-027: no value, no fallback.
    #[test]
    fn a_missing_or_empty_variable_is_refused() {
        for v in [None, Some(OsString::new())] {
            let e = from_env(v).unwrap_err();
            assert!(
                e.0.contains(ENV) && e.0.contains("never uses the normal profile"),
                "{e}"
            );
        }
    }

    #[test]
    fn a_relative_or_missing_directory_is_refused() {
        assert!(from_env(Some("relative/dir".into())).is_err());
        let tmp = TempDir::new().unwrap();
        assert!(from_env(env(&tmp.path().join("absent"))).is_err());
        let file = tmp.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(from_env(env(&file)).is_err());
    }

    #[test]
    fn a_missing_or_invalid_marker_is_refused() {
        let tmp = TempDir::new().unwrap();
        assert!(
            from_env(env(tmp.path()))
                .unwrap_err()
                .0
                .contains(MARKER_FILE)
        );
        let cases = [
            "".to_owned(),
            "not json".to_owned(),
            format!(r#"{{"format":2,"purpose":"{PURPOSE}","run":"r","nonce":"{NONCE}"}}"#),
            format!(r#"{{"format":1,"purpose":"other","run":"r","nonce":"{NONCE}"}}"#),
            format!(r#"{{"format":1,"purpose":"{PURPOSE}","run":"","nonce":"{NONCE}"}}"#),
            format!(r#"{{"format":1,"purpose":"{PURPOSE}","run":"a/b","nonce":"{NONCE}"}}"#),
            format!(r#"{{"format":1,"purpose":"{PURPOSE}","run":"r","nonce":"short"}}"#),
            format!(
                r#"{{"format":1,"purpose":"{PURPOSE}","run":"r","nonce":"{}"}}"#,
                NONCE.to_uppercase()
            ),
            format!(
                r#"{{"format":1,"purpose":"{PURPOSE}","run":"r","nonce":"{NONCE}","extra":1}}"#
            ),
            format!(r#"{{"format":1,"purpose":"{PURPOSE}","run":"r"}}"#),
        ];
        for body in cases {
            marker(tmp.path(), &body);
            assert!(from_env(env(tmp.path())).is_err(), "{body}");
        }
        // A directory in the marker's place, or an oversized file.
        fs::remove_file(tmp.path().join(MARKER_FILE)).unwrap();
        fs::create_dir(tmp.path().join(MARKER_FILE)).unwrap();
        assert!(from_env(env(tmp.path())).is_err());
        fs::remove_dir(tmp.path().join(MARKER_FILE)).unwrap();
        marker(tmp.path(), &" ".repeat(5000));
        assert!(from_env(env(tmp.path())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_marker_that_is_a_symbolic_link_is_refused() {
        let tmp = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        good(elsewhere.path());
        std::os::unix::fs::symlink(
            elsewhere.path().join(MARKER_FILE),
            tmp.path().join(MARKER_FILE),
        )
        .unwrap();
        assert!(from_env(env(tmp.path())).is_err());
    }

    /// ADR-027: the profile can never be the normal profile, inside it, or
    /// around it, however it is spelled.
    #[test]
    fn a_profile_overlapping_a_normal_directory_is_refused() {
        let base = TempDir::new().unwrap();
        let normal = base.path().join("com.sotyware.desktop");
        fs::create_dir_all(&normal).unwrap();
        // The profile is the normal directory itself.
        good(&normal);
        let p = from_env(env(&normal)).unwrap();
        assert!(check_separate(&p, std::slice::from_ref(&normal)).is_err());
        // Inside it.
        let inside = normal.join("sub");
        fs::create_dir(&inside).unwrap();
        good(&inside);
        let p = from_env(env(&inside)).unwrap();
        assert!(check_separate(&p, std::slice::from_ref(&normal)).is_err());
        // Around it (the normal dir would be inside the profile), also when
        // the normal dir does not exist yet and is spelled with `..`.
        good(base.path());
        let p = from_env(env(base.path())).unwrap();
        let spelled = base.path().join("x").join("..").join("not-yet");
        assert!(check_separate(&p, &[spelled]).is_err());
        // A sibling is fine.
        let sibling = base.path().join("profile-1");
        fs::create_dir(&sibling).unwrap();
        good(&sibling);
        let p = from_env(env(&sibling)).unwrap();
        assert!(check_separate(&p, &[normal]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_profile_reached_through_a_symbolic_link_is_compared_by_target() {
        let base = TempDir::new().unwrap();
        let normal = base.path().join("normal");
        fs::create_dir(&normal).unwrap();
        good(&normal);
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&normal, &link).unwrap();
        let p = from_env(env(&link)).unwrap();
        assert!(check_separate(&p, &[normal]).is_err());
    }

    /// A `..` after a folder that does not exist removes that folder, also
    /// behind a symbolic link. macOS's temporary folder is behind one
    /// (`/var` → `/private/var`); there the unresolved spelling shared no
    /// prefix with the profile's resolved root and the overlap was missed.
    #[cfg(unix)]
    #[test]
    fn a_missing_folder_spelled_with_dot_dot_behind_a_link_is_resolved() {
        let base = TempDir::new().unwrap();
        let real = base.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let target = fs::canonicalize(&real).unwrap();
        let spell = |parts: &[&str]| parts.iter().fold(link.clone(), |p, part| p.join(part));
        assert_eq!(
            resolve(&spell(&["x", "..", "not-yet"])),
            target.join("not-yet")
        );
        assert_eq!(
            resolve(&spell(&["x", "y", "..", "..", "z"])),
            target.join("z")
        );
        // Past the missing folders, `..` goes to the resolved parent.
        assert_eq!(
            resolve(&spell(&["x", "..", "..", "w"])),
            target.parent().unwrap().join("w")
        );
        // The profile around a normal directory spelled that way is refused,
        good(&real);
        let p = from_env(env(&link)).unwrap();
        assert!(check_separate(&p, &[spell(&["x", "..", "not-yet"])]).is_err());
        // a sibling of it is not.
        fs::create_dir(real.join("profile-1")).unwrap();
        good(&real.join("profile-1"));
        let p = from_env(env(&link.join("profile-1"))).unwrap();
        assert!(check_separate(&p, &[spell(&["x", "..", "normal"])]).is_ok());
    }

    #[test]
    fn prefix_tests_are_component_wise() {
        assert!(is_within(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(is_within(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!is_within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!is_within(Path::new("/a"), Path::new("/a/b")));
    }

    #[test]
    fn the_acknowledgement_names_the_run_the_pid_and_every_path() {
        let tmp = TempDir::new().unwrap();
        good(tmp.path());
        let p = from_env(env(tmp.path())).unwrap();
        fs::create_dir_all(p.data_dir()).unwrap();
        let db = p.data_dir().join("cloud-accounts.sqlite3");
        fs::write(&db, "").unwrap();
        write_ack(&p, std::slice::from_ref(&db)).unwrap();
        // Written again by a later launch: replaced, no temporary left.
        write_ack(&p, std::slice::from_ref(&db)).unwrap();
        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        let ack: serde_json::Value =
            serde_json::from_slice(&fs::read(tmp.path().join(ACK_FILE)).unwrap()).unwrap();
        assert_eq!(ack["format"], 1);
        assert_eq!(ack["purpose"], PURPOSE);
        assert_eq!(ack["build"], BUILD_MARKER);
        assert_eq!(ack["run"], "r-1");
        assert_eq!(ack["nonce"], NONCE);
        assert_eq!(ack["pid"], std::process::id());
        assert_eq!(ack["identifier"], IDENTIFIER);
        assert_eq!(ack["credential_service"], "com.sotyware.desktop.test");
        assert_eq!(
            ack["credential_target_prefix"],
            format!("com.sotyware.desktop.test/{NONCE}/")
        );
        let root = p.root().to_string_lossy().into_owned();
        assert_eq!(ack["root"], root);
        for key in ["data_dir", "webview_dir"] {
            assert!(ack[key].as_str().unwrap().starts_with(&root), "{key}");
        }
        assert_eq!(
            ack["stores"][0],
            fs::canonicalize(&db).unwrap().to_string_lossy().as_ref()
        );
    }

    #[test]
    fn the_test_identifier_differs_from_the_app_identifier() {
        let conf = format!("{}/tauri.conf.json", env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(conf).unwrap()).unwrap();
        assert_ne!(conf["identifier"], IDENTIFIER);
        assert_eq!(IDENTIFIER, crate::cloud::keyring::TEST_SERVICE);
    }
}

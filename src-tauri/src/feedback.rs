//! Feedback and bug reports (ADR-049): the dashboard's "Send feedback" and
//! "Report a bug" buttons open a new GitHub issue for this repository in the
//! user's browser. The page sends only which of the two it wants; Rust builds
//! the URL from fixed parts, so the page can never make sotyware open an
//! address of its choosing. A bug report is prefilled with sotyware's version
//! and the operating system's name and architecture, nothing that identifies
//! the user or the installation. Nothing is sent: the user writes and submits
//! the issue in the browser, with their own GitHub account.

use serde::Deserialize;

use crate::updates::RELEASE_REPOSITORY;

/// Which issue form to open (`.github/ISSUE_TEMPLATE/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeedbackKind {
    Feedback,
    Bug,
}

/// The issue form's file name; must match `.github/ISSUE_TEMPLATE/`.
const FEEDBACK_TEMPLATE: &str = "feedback.yml";
const BUG_TEMPLATE: &str = "bug_report.yml";

/// The operating system as a person would name it, and the architecture.
fn platform() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    format!("{os}, {}", std::env::consts::ARCH)
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The new-issue page for `kind`. GitHub's issue forms fill a field from a
/// query parameter named after its `id`.
pub fn issue_url(kind: FeedbackKind, version: &str, platform: &str) -> String {
    let base = format!("https://github.com/{RELEASE_REPOSITORY}/issues/new");
    match kind {
        FeedbackKind::Feedback => format!("{base}?template={FEEDBACK_TEMPLATE}"),
        FeedbackKind::Bug => format!(
            "{base}?template={BUG_TEMPLATE}&version={}&os={}",
            encode(version),
            encode(platform)
        ),
    }
}

/// Opens the new-issue page in the default browser. Allowed by
/// `allow-open-feedback-page`.
#[tauri::command]
pub async fn open_feedback_page(kind: FeedbackKind) -> Result<(), String> {
    let url = issue_url(kind, env!("CARGO_PKG_VERSION"), &platform());
    open_in_browser(&url)
}

#[cfg(desktop)]
fn open_in_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| {
        eprintln!("sotyware: failed to open the browser: {e}");
        format!("The browser could not be opened. The page is {url}")
    })
}

#[cfg(not(desktop))]
fn open_in_browser(_: &str) -> Result<(), String> {
    Err("opening a browser is not available here".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_opens_the_feedback_form_with_nothing_filled_in() {
        assert_eq!(
            issue_url(FeedbackKind::Feedback, "0.1.1", "Windows, x86_64"),
            "https://github.com/casualsoty/sotyware/issues/new?template=feedback.yml"
        );
    }

    #[test]
    fn a_bug_report_carries_only_the_version_and_the_platform() {
        assert_eq!(
            issue_url(FeedbackKind::Bug, "0.1.1", "Windows, x86_64"),
            "https://github.com/casualsoty/sotyware/issues/new?template=bug_report.yml\
             &version=0.1.1&os=Windows%2C%20x86_64"
        );
    }

    #[test]
    fn values_cannot_add_parameters() {
        let url = issue_url(FeedbackKind::Bug, "1.0&title=x", "a#b?c=d e");
        assert!(
            url.ends_with("&version=1.0%26title%3Dx&os=a%23b%3Fc%3Dd%20e"),
            "{url}"
        );
    }

    #[test]
    fn the_page_names_one_of_two_kinds() {
        let parse = |s: &str| serde_json::from_str::<FeedbackKind>(s);
        assert_eq!(parse("\"feedback\"").unwrap(), FeedbackKind::Feedback);
        assert_eq!(parse("\"bug\"").unwrap(), FeedbackKind::Bug);
        assert!(parse("\"https://example.com\"").is_err());
    }

    #[test]
    fn the_templates_exist_with_the_fields_the_bug_url_fills() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/ISSUE_TEMPLATE");
        let feedback = std::fs::read_to_string(dir.join(FEEDBACK_TEMPLATE)).unwrap();
        assert!(feedback.contains("name: Feedback"));
        let bug = std::fs::read_to_string(dir.join(BUG_TEMPLATE)).unwrap();
        assert!(bug.contains("id: version"));
        assert!(bug.contains("id: os"));
    }

    #[test]
    fn the_platform_names_the_system_and_architecture() {
        let p = platform();
        assert!(p.ends_with(&format!(", {}", std::env::consts::ARCH)), "{p}");
    }
}

//! Pages of this repository on GitHub, opened in the user's browser
//! (ADR-049): the dashboard's "Send feedback" and "Report a bug" buttons
//! open a new issue form, and the privacy notice links open
//! `docs/user/privacy.md`. The page sends only which page it wants; Rust
//! builds the URL from fixed parts, so the page can never make sotyware
//! open an address of its choosing. A bug report is prefilled with sotyware's version
//! and the operating system's name and architecture, nothing that identifies
//! the user or the installation. Nothing is sent: the user writes and submits
//! the issue in the browser, with their own GitHub account.

use serde::Deserialize;

use crate::updates::RELEASE_REPOSITORY;

/// Which page to open: an issue form (`.github/ISSUE_TEMPLATE/`) or the
/// privacy notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GithubPage {
    Feedback,
    Bug,
    PrivacyNotice,
}

/// The issue form's file name; must match `.github/ISSUE_TEMPLATE/`.
const FEEDBACK_TEMPLATE: &str = "feedback.yml";
const BUG_TEMPLATE: &str = "bug_report.yml";
/// The privacy notice, as it is on `main` (always the current one).
const PRIVACY_NOTICE: &str = "docs/user/privacy.md";

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

/// The address of `page`. GitHub's issue forms fill a field from a query
/// parameter named after its `id`.
pub fn page_url(page: GithubPage, version: &str, platform: &str) -> String {
    let repo = format!("https://github.com/{RELEASE_REPOSITORY}");
    let base = format!("{repo}/issues/new");
    match page {
        GithubPage::PrivacyNotice => format!("{repo}/blob/main/{PRIVACY_NOTICE}"),
        GithubPage::Feedback => format!("{base}?template={FEEDBACK_TEMPLATE}"),
        GithubPage::Bug => format!(
            "{base}?template={BUG_TEMPLATE}&version={}&os={}",
            encode(version),
            encode(platform)
        ),
    }
}

/// Opens `page` in the default browser. Allowed by
/// `allow-open-github-page`.
#[tauri::command]
pub async fn open_github_page(page: GithubPage) -> Result<(), String> {
    let url = page_url(page, env!("CARGO_PKG_VERSION"), &platform());
    open_in_browser(&url)
}

/// A test-profile build (ADR-027) never starts a browser: a desktop test
/// cannot see or close one, and it would reach GitHub. It appends the
/// address to `opened-pages.txt` in the run's profile, where the driver
/// reads it.
#[cfg(feature = "test-profile")]
fn open_in_browser(url: &str) -> Result<(), String> {
    use std::io::Write;
    let profile = crate::test_profile::from_env(std::env::var_os(crate::test_profile::ENV))
        .map_err(|e| format!("the test profile is not usable: {e}"))?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profile.root().join(OPENED_PAGES))
        .and_then(|mut f| writeln!(f, "{url}"))
        .map_err(|e| format!("the page could not be recorded: {e}"))
}

/// Where a test-profile build records the pages it would open.
#[cfg(feature = "test-profile")]
const OPENED_PAGES: &str = "opened-pages.txt";

#[cfg(all(desktop, not(feature = "test-profile")))]
fn open_in_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| {
        eprintln!("sotyware: failed to open the browser: {e}");
        format!("The browser could not be opened. The page is {url}")
    })
}

#[cfg(all(not(desktop), not(feature = "test-profile")))]
fn open_in_browser(_: &str) -> Result<(), String> {
    Err("opening a browser is not available here".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_opens_the_feedback_form_with_nothing_filled_in() {
        assert_eq!(
            page_url(GithubPage::Feedback, "0.1.1", "Windows, x86_64"),
            "https://github.com/casualsoty/sotyware/issues/new?template=feedback.yml"
        );
    }

    #[test]
    fn a_bug_report_carries_only_the_version_and_the_platform() {
        assert_eq!(
            page_url(GithubPage::Bug, "0.1.1", "Windows, x86_64"),
            "https://github.com/casualsoty/sotyware/issues/new?template=bug_report.yml\
             &version=0.1.1&os=Windows%2C%20x86_64"
        );
    }

    #[test]
    fn values_cannot_add_parameters() {
        let url = page_url(GithubPage::Bug, "1.0&title=x", "a#b?c=d e");
        assert!(
            url.ends_with("&version=1.0%26title%3Dx&os=a%23b%3Fc%3Dd%20e"),
            "{url}"
        );
    }

    #[test]
    fn the_privacy_notice_is_the_one_on_main() {
        assert_eq!(
            page_url(GithubPage::PrivacyNotice, "0.1.1", "Linux, x86_64"),
            "https://github.com/casualsoty/sotyware/blob/main/docs/user/privacy.md"
        );
        let notice = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(PRIVACY_NOTICE);
        assert!(
            std::fs::read_to_string(notice)
                .unwrap()
                .starts_with("# Privacy notice")
        );
    }

    #[test]
    fn the_page_names_one_of_three_pages() {
        let parse = |s: &str| serde_json::from_str::<GithubPage>(s);
        assert_eq!(parse("\"feedback\"").unwrap(), GithubPage::Feedback);
        assert_eq!(parse("\"bug\"").unwrap(), GithubPage::Bug);
        assert_eq!(
            parse("\"privacy-notice\"").unwrap(),
            GithubPage::PrivacyNotice
        );
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

//! The platform's input backend (ADR-050), and the file dialogs of
//! import and export.
//!
//! Windows (`windows.rs`: SendInput, low-level hooks, RegisterHotKey) and
//! Linux desktops running X11 (`x11.rs`: XTest, XRecord, XGrabKey) have a
//! backend; elsewhere `support()` says why there is none and the feature
//! cannot start.

use std::path::PathBuf;
use std::sync::Arc;

use super::input::{Backend, HotkeySink, Support};

#[cfg(windows)]
pub fn support() -> Support {
    super::windows::support()
}

#[cfg(windows)]
pub fn open(sink: HotkeySink) -> Result<Arc<dyn Backend>, String> {
    super::windows::open(sink)
}

#[cfg(target_os = "linux")]
pub fn support() -> Support {
    super::x11::support()
}

#[cfg(target_os = "linux")]
pub fn open(sink: HotkeySink) -> Result<Arc<dyn Backend>, String> {
    super::x11::open(sink)
}

/// Why global shortcuts cannot be registered on this system, for the
/// notes overlay (ADR-052), which uses the backend only for them; `None`
/// where they can.
#[cfg(windows)]
pub fn shortcuts_unavailable() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
pub fn shortcuts_unavailable() -> Option<String> {
    if super::x11::wayland_session() {
        return Some(
            "this desktop session uses Wayland, which does not let applications register global shortcuts; use the dashboard, or sign in to an X11 (Xorg) session"
                .to_owned(),
        );
    }
    let s = support();
    (!s.available).then(|| {
        s.note
            .unwrap_or_else(|| "global shortcuts are not available here".to_owned())
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn shortcuts_unavailable() -> Option<String> {
    Some("global shortcuts are available on Windows and on Linux desktops running X11; not on this system yet".to_owned())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn support() -> Support {
    Support {
        available: false,
        backend: "none",
        note: Some(
            "Input automation is available on Windows and on Linux desktops running X11; not on this system yet."
                .to_owned(),
        ),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn open(_sink: HotkeySink) -> Result<Arc<dyn Backend>, String> {
    Err(support().note.unwrap_or_default())
}

/// The save dialog of an export, opened by Rust: the page never names a
/// path.
#[cfg(desktop)]
pub async fn choose_save(window: &tauri::WebviewWindow, name: String) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Export macros and profiles")
        .set_file_name(name)
        .add_filter("sotyware macros and profiles", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.ok().flatten()?.into_path().ok()
}

/// The open dialog of an import.
#[cfg(desktop)]
pub async fn choose_open(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Import macros and profiles")
        .add_filter("sotyware macros and profiles", &["json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.ok().flatten()?.into_path().ok()
}

#[cfg(not(desktop))]
pub async fn choose_save(_window: &tauri::WebviewWindow, _name: String) -> Option<PathBuf> {
    None
}

#[cfg(not(desktop))]
pub async fn choose_open(_window: &tauri::WebviewWindow) -> Option<PathBuf> {
    None
}

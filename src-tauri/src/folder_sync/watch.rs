//! File change notifications for one sync folder (ADR-043), through the
//! `notify` crate: `ReadDirectoryChangesW` on Windows, inotify on Linux,
//! FSEvents on macOS, recursive.
//!
//! The watcher only names paths: each event marks its relative paths dirty
//! in the engine's [`Control`] and wakes the engine, which looks at them
//! again after the events settle. Events that may have lost information
//! (an overflow, a rescan request, an error) ask for a full scan. Nothing
//! relies on the watcher alone: every start, every return of the folder
//! and every failure to watch leads to a full scan, and the periodic remote
//! check is unaffected. If the folder cannot be watched (for example the
//! system's watch limit on Linux), the sync runs without it and finds local
//! changes at each periodic check.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::Control;
use super::model;

/// A running watcher; dropping it stops watching.
pub struct Watcher {
    _inner: RecommendedWatcher,
}

impl Watcher {
    /// Watches `root` recursively; `None` (logged) if the system refuses.
    pub fn start(root: &Path, control: Arc<Control>) -> Option<Self> {
        let base = root.to_path_buf();
        let handler = move |result: notify::Result<Event>| match result {
            Ok(event) => on_event(&base, &control, &event),
            Err(_) => control.mark_full(),
        };
        let mut inner = match notify::recommended_watcher(handler) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("sotyware: cannot watch a sync folder for changes: {e}");
                return None;
            }
        };
        if let Err(e) = inner.watch(root, RecursiveMode::Recursive) {
            eprintln!("sotyware: cannot watch a sync folder for changes: {e}");
            return None;
        }
        Some(Self { _inner: inner })
    }
}

fn on_event(root: &Path, control: &Control, event: &Event) {
    if event.need_rescan() {
        control.mark_full();
        return;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    let mut any = false;
    for path in &event.paths {
        match relative(root, path) {
            Some(rel) if rel.is_empty() => {
                // The folder itself: renamed, removed or its attributes.
                control.mark_full();
                return;
            }
            Some(rel) if !model::ignored(&rel) => {
                control.mark_dirty(rel);
                any = true;
            }
            _ => {}
        }
    }
    if any {
        control.wake.notify_one();
    }
}

/// `path` relative to `root` with `/` between segments; `None` outside
/// `root` or with a segment that is not Unicode.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest: PathBuf = path.strip_prefix(root).ok()?.to_path_buf();
    let mut out = String::new();
    for part in rest.components() {
        let std::path::Component::Normal(name) = part else {
            return None;
        };
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(name.to_str()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_become_relative_with_slashes() {
        let root = Path::new("/sync/root");
        assert_eq!(
            relative(root, &root.join("a").join("b.txt")),
            Some("a/b.txt".into())
        );
        assert_eq!(relative(root, root), Some(String::new()));
        assert_eq!(relative(root, Path::new("/elsewhere/x")), None);
    }
}

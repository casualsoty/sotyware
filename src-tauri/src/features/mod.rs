//! Built-in features: native Rust modules that sotyware starts, stops and reports
//! on.
//!
//! A feature implements [`Feature`] and is listed once in [`builtin`]. `run()`
//! builds a [`FeatureRegistry`] from that list and manages it as Tauri state; the
//! registry owns every feature, drives its lifecycle and isolates failures: an
//! error from one feature is recorded in that feature's state and never stops
//! another one. Whether a feature starts, and saving the user's choice, is the
//! job of [`FeaturePreferences`]; the registry never sees storage. The frontend
//! sees features only through the two commands below.
//! How to add a feature: docs/architecture/features.md.

mod backup;
mod folder_sync;
mod preferences;
mod registry;
mod updates;

use std::fmt;

use serde::Serialize;
use tauri::State;

pub use preferences::{FeaturePreferences, PreferenceStore, StoreError};
pub use registry::FeatureRegistry;

/// Every built-in feature, in registration order. Startup runs them in this
/// order and exit stops them in reverse. Adding a feature means adding one line
/// here; nothing else in the app has to change unless the feature needs its own
/// command.
pub fn builtin() -> Vec<Box<dyn Feature>> {
    vec![
        Box::new(backup::BackupScheduler),
        Box::new(updates::UpdateCheck),
        Box::new(folder_sync::FolderSync),
    ]
}

/// What a feature is. Fixed at compile time; the registry copies it once, at
/// registration, so it cannot change while the app runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureMetadata {
    /// Stable, unique, lowercase kebab-case (`backup-scheduler`, `update-check`).
    /// Never change it once released: stored preferences are keyed by it.
    pub id: &'static str,
    /// Human-readable name shown in the dashboard.
    pub name: &'static str,
    /// One sentence on what the feature does.
    pub description: &'static str,
    /// The feature's own version, independent of the app version.
    pub version: &'static str,
    /// Whether the feature starts when no preference is stored for it (see
    /// [`FeaturePreferences::enabled_at_startup`]).
    pub enabled_by_default: bool,
}

/// Why a feature could not start or stop. Shown to the user as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureError(String);

impl FeatureError {
    /// How a feature reports a failed start or stop. Neither built-in
    /// feature can fail today (both only switch work that lives elsewhere),
    /// so outside the tests nothing calls it; it stays the trait's API.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A built-in feature.
///
/// The registry calls `start` and `stop` only on valid transitions, never twice
/// in a row, and never concurrently for the same feature. Both are called
/// synchronously (at startup on the main thread, from the frontend on a Tauri
/// worker thread, at exit on the main thread), so they must return promptly:
/// long-running work belongs on a thread or task the feature spawns in `start`
/// and joins in `stop`. No polling.
pub trait Feature: Send {
    fn metadata(&self) -> FeatureMetadata;

    /// Acquires the feature's resources. On `Err` the feature must have released
    /// anything it acquired: it is then `Failed` and `stop` is not called.
    fn start(&mut self) -> Result<(), FeatureError>;

    /// Releases everything `start` acquired. Called only after a successful
    /// `start`. On `Err` the feature is still treated as stopped (`Disabled`,
    /// with the error recorded).
    fn stop(&mut self) -> Result<(), FeatureError>;
}

/// Lifecycle state of a feature. Transitions: docs/architecture/features.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureState {
    /// Registered, startup has not decided yet whether it runs.
    Registered,
    /// Not running, by choice (or default).
    Disabled,
    /// `start` is running.
    Initializing,
    /// `start` succeeded.
    Running,
    /// `start` failed; nothing of the feature runs. Can be enabled again (retry)
    /// or disabled.
    Failed,
    /// `stop` is running.
    Stopping,
}

impl FeatureState {
    /// Whether the user's intent is "on": the feature runs, is starting, or tried
    /// to start and failed.
    fn enabled(self) -> bool {
        matches!(self, Self::Initializing | Self::Running | Self::Failed)
    }
}

/// A feature as the frontend sees it: metadata plus current state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeatureInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
    pub state: FeatureState,
    pub enabled: bool,
    /// The last start or stop error, cleared by the next successful transition.
    pub error: Option<String>,
}

/// What `list_features` returns: every feature, and why stored preferences
/// could not be loaded, if they could not (the features then run with their
/// defaults and changes are not saved).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureList {
    pub features: Vec<FeatureInfo>,
    pub storage_error: Option<String>,
}

/// What `set_feature_enabled` returns: the feature's new state, and why the
/// choice could not be saved, if it could not. The switch itself has happened
/// either way; an unsaved choice lasts until sotyware quits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureChange {
    pub feature: FeatureInfo,
    pub save_error: Option<String>,
}

/// `list_features` without Tauri, for tests.
pub(crate) fn feature_list(registry: &FeatureRegistry, prefs: &FeaturePreferences) -> FeatureList {
    FeatureList {
        features: registry.list(),
        storage_error: prefs.load_error().map(ToString::to_string),
    }
}

/// `set_feature_enabled` without Tauri, for tests: switch, then save the result.
/// Only a switch the registry accepted is saved; a failed save is reported in
/// the result, not as an error, because the switch cannot be undone safely.
pub(crate) fn switch_feature(
    registry: &FeatureRegistry,
    prefs: &FeaturePreferences,
    id: &str,
    enabled: bool,
) -> Result<FeatureChange, String> {
    let feature = registry
        .set_enabled(id, enabled)
        .map_err(|e| e.to_string())?;
    let save_error = prefs.save_current(registry, id).err().map(|e| {
        eprintln!("sotyware: failed to save the preference for feature '{id}': {e}");
        e.to_string()
    });
    Ok(FeatureChange {
        feature,
        save_error,
    })
}

/// Lists every registered feature, in registration order, and whether stored
/// preferences were available. Allowed for the dashboard by the
/// `allow-list-features` permission.
#[tauri::command]
pub fn list_features(
    registry: State<'_, FeatureRegistry>,
    prefs: State<'_, FeaturePreferences>,
) -> FeatureList {
    feature_list(&registry, &prefs)
}

/// Enables (starts) or disables (stops) one feature, saves the choice, and
/// returns the feature's new state with any save error. A failing start is not
/// an error here: it is reported as state `failed` (and saved as enabled). The
/// command fails only for an unknown id or a transition the current state does
/// not allow; then nothing is saved. Runs on Tauri's thread pool (`async`), so
/// neither a feature's `start`/`stop` nor the database write blocks the event
/// loop. Allowed for the dashboard by the `allow-set-feature-enabled`
/// permission.
#[tauri::command(async)]
pub fn set_feature_enabled(
    registry: State<'_, FeatureRegistry>,
    prefs: State<'_, FeaturePreferences>,
    id: String,
    enabled: bool,
) -> Result<FeatureChange, String> {
    switch_feature(&registry, &prefs, &id, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_features_register_without_errors() {
        let mut registry = FeatureRegistry::default();
        for feature in builtin() {
            let id = feature.metadata().id;
            assert_eq!(
                registry.register(feature),
                Ok(()),
                "built-in feature '{id}'"
            );
        }
        assert!(!registry.list().is_empty());
    }

    #[test]
    fn builtin_features_all_start_and_stop_cleanly() {
        let registry = FeatureRegistry::with_features(builtin());
        registry.start_all(|_| true);
        for info in registry.list() {
            assert_eq!(info.state, FeatureState::Running, "{}", info.id);
        }
        registry.stop_all();
        for info in registry.list() {
            assert_eq!(
                (info.state, info.error),
                (FeatureState::Disabled, None),
                "{}",
                info.id
            );
        }
    }

    /// The frontend (`src/features.ts`) parses exactly this shape.
    #[test]
    fn frontend_receives_feature_info_in_this_shape() {
        let info = FeatureInfo {
            id: "test-feature",
            name: "Test feature",
            description: "Does nothing.",
            version: "0.1.0",
            state: FeatureState::Failed,
            enabled: true,
            error: Some("boom".into()),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "id": "test-feature",
                "name": "Test feature",
                "description": "Does nothing.",
                "version": "0.1.0",
                "state": "failed",
                "enabled": true,
                "error": "boom",
            })
        );
    }

    /// `src/features.ts` parses exactly these shapes too.
    #[test]
    fn frontend_receives_the_list_and_the_change_in_this_shape() {
        let info = FeatureInfo {
            id: "test-feature",
            name: "Test feature",
            description: "Does nothing.",
            version: "0.1.0",
            state: FeatureState::Disabled,
            enabled: false,
            error: None,
        };
        let list = FeatureList {
            features: vec![info.clone()],
            storage_error: Some("file is not a database".into()),
        };
        let change = FeatureChange {
            feature: info,
            save_error: None,
        };
        let info_json = serde_json::to_value(&change.feature).unwrap();
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            serde_json::json!({
                "features": [info_json.clone()],
                "storageError": "file is not a database",
            })
        );
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            serde_json::json!({ "feature": info_json, "saveError": null })
        );
    }

    #[test]
    fn frontend_receives_the_state_names() {
        let names: Vec<_> = [
            FeatureState::Registered,
            FeatureState::Disabled,
            FeatureState::Initializing,
            FeatureState::Running,
            FeatureState::Failed,
            FeatureState::Stopping,
        ]
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                "registered",
                "disabled",
                "initializing",
                "running",
                "failed",
                "stopping"
            ]
        );
    }

    #[test]
    fn enabled_means_the_user_wants_it_on() {
        use FeatureState::*;
        for (state, enabled) in [
            (Registered, false),
            (Disabled, false),
            (Initializing, true),
            (Running, true),
            (Failed, true),
            (Stopping, false),
        ] {
            assert_eq!(state.enabled(), enabled, "{state:?}");
        }
    }
}

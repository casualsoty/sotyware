//! Built-in features: native Rust modules that sotyware starts, stops and reports
//! on.
//!
//! A feature implements [`Feature`] and is listed once in [`builtin`]. `run()`
//! builds a [`FeatureRegistry`] from that list and manages it as Tauri state; the
//! registry owns every feature, drives its lifecycle and isolates failures: an
//! error from one feature is recorded in that feature's state and never stops
//! another one. The frontend sees features only through the two commands below.
//! How to add a feature: docs/architecture/features.md.

mod example;
mod registry;

use std::fmt;

use serde::Serialize;
use tauri::State;

pub use registry::FeatureRegistry;

/// Every built-in feature, in registration order. Startup runs them in this
/// order and exit stops them in reverse. Adding a feature means adding one line
/// here; nothing else in the app has to change unless the feature needs its own
/// command.
pub fn builtin() -> Vec<Box<dyn Feature>> {
    vec![Box::new(example::Example::default())]
}

/// Whether a feature starts when sotyware starts.
///
/// Nothing is persisted yet, so this is the feature's default: a feature the
/// user disabled is enabled again after a restart if its default says so. When
/// preferences are stored, the stored choice is looked up here (falling back to
/// the default); the registry itself never reads or writes storage.
pub fn enabled_at_startup(metadata: &FeatureMetadata) -> bool {
    metadata.enabled_by_default
}

/// What a feature is. Fixed at compile time; the registry copies it once, at
/// registration, so it cannot change while the app runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureMetadata {
    /// Stable, unique, lowercase kebab-case (`example`, `clipboard-history`).
    /// Never change it once released: stored preferences will be keyed by it.
    pub id: &'static str,
    /// Human-readable name shown in the dashboard.
    pub name: &'static str,
    /// One sentence on what the feature does.
    pub description: &'static str,
    /// The feature's own version, independent of the app version.
    pub version: &'static str,
    /// Whether the feature starts when there is no stored preference (today:
    /// always, see [`enabled_at_startup`]).
    pub enabled_by_default: bool,
}

/// Why a feature could not start or stop. Shown to the user as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureError(String);

impl FeatureError {
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

/// Lists every registered feature, in registration order.
/// Allowed for the dashboard by the `allow-list-features` permission.
#[tauri::command]
pub fn list_features(registry: State<'_, FeatureRegistry>) -> Vec<FeatureInfo> {
    registry.list()
}

/// Enables (starts) or disables (stops) one feature and returns its new state.
/// A failing start is not an error here: it is reported as state `failed`. The
/// command fails only for an unknown id or a transition the current state does
/// not allow. Runs on Tauri's thread pool (`async`), so a feature's `start` or
/// `stop` never blocks the event loop.
/// Allowed for the dashboard by the `allow-set-feature-enabled` permission.
#[tauri::command(async)]
pub fn set_feature_enabled(
    registry: State<'_, FeatureRegistry>,
    id: String,
    enabled: bool,
) -> Result<FeatureInfo, String> {
    registry
        .set_enabled(&id, enabled)
        .map_err(|e| e.to_string())
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

    #[test]
    fn startup_uses_the_default_while_nothing_is_persisted() {
        let mut metadata = builtin()[0].metadata();
        metadata.enabled_by_default = true;
        assert!(enabled_at_startup(&metadata));
        metadata.enabled_by_default = false;
        assert!(!enabled_at_startup(&metadata));
    }

    /// The frontend (`src/features.ts`) parses exactly this shape.
    #[test]
    fn frontend_receives_feature_info_in_this_shape() {
        let info = FeatureInfo {
            id: "example",
            name: "Example",
            description: "Does nothing.",
            version: "0.1.0",
            state: FeatureState::Failed,
            enabled: true,
            error: Some("boom".into()),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "id": "example",
                "name": "Example",
                "description": "Does nothing.",
                "version": "0.1.0",
                "state": "failed",
                "enabled": true,
                "error": "boom",
            })
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

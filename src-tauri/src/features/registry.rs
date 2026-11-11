//! The feature registry: owns every built-in feature and drives its lifecycle.
//!
//! The set of features is fixed once the registry is managed as Tauri state:
//! registering needs `&mut self`, everything afterwards takes `&self`. Each
//! feature has its own two locks: `status` (state and last error; held only
//! briefly) and `feature` (held while `start` or `stop` runs). A slow feature
//! therefore never blocks reading the list or changing another feature, and the
//! transient states `Initializing` and `Stopping` are visible while it works.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Feature, FeatureInfo, FeatureMetadata, FeatureState};

/// What the user (or startup, or exit) asks of a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Enable,
    Disable,
}

/// What the registry does for an allowed transition.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// `Initializing`, call `start`, then `Running` or `Failed`.
    Start,
    /// `Stopping`, call `stop`, then `Disabled`.
    Stop,
    /// Straight to `Disabled`: nothing of the feature runs, so `stop` is not
    /// called.
    Settle,
}

/// The lifecycle table. `None` is an invalid transition: enabling what is
/// already on or busy, disabling what is already off or busy.
fn plan(from: FeatureState, action: Action) -> Option<Step> {
    use FeatureState::*;
    match (from, action) {
        (Registered | Disabled | Failed, Action::Enable) => Some(Step::Start),
        (Running, Action::Disable) => Some(Step::Stop),
        (Registered | Failed, Action::Disable) => Some(Step::Settle),
        (Running | Initializing | Stopping, Action::Enable)
        | (Disabled | Initializing | Stopping, Action::Disable) => None,
    }
}

/// Why a feature was not registered.
#[derive(Debug, PartialEq, Eq)]
pub enum RegisterError {
    DuplicateId(&'static str),
    /// Not lowercase kebab-case.
    InvalidId(&'static str),
    EmptyField {
        id: &'static str,
        field: &'static str,
    },
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "duplicate feature id '{id}'"),
            Self::InvalidId(id) => {
                write!(
                    f,
                    "invalid feature id '{id}' (must be lowercase kebab-case)"
                )
            }
            Self::EmptyField { id, field } => write!(f, "feature '{id}' has an empty {field}"),
        }
    }
}

/// Why `set_enabled` did nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum ControlError {
    UnknownFeature(String),
    InvalidTransition {
        id: &'static str,
        state: FeatureState,
        action: Action,
    },
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFeature(id) => write!(f, "unknown feature '{id}'"),
            Self::InvalidTransition { id, state, action } => {
                let verb = match action {
                    Action::Enable => "enable",
                    Action::Disable => "disable",
                };
                write!(f, "cannot {verb} feature '{id}' while it is {state:?}")
            }
        }
    }
}

#[derive(Debug)]
struct Status {
    state: FeatureState,
    error: Option<String>,
}

struct Entry {
    metadata: FeatureMetadata,
    status: Mutex<Status>,
    feature: Mutex<Box<dyn Feature>>,
}

impl Entry {
    fn info(&self) -> FeatureInfo {
        let status = lock(&self.status);
        to_info(&self.metadata, &status)
    }
}

fn to_info(metadata: &FeatureMetadata, status: &Status) -> FeatureInfo {
    FeatureInfo {
        id: metadata.id,
        name: metadata.name,
        description: metadata.description,
        version: metadata.version,
        state: status.state,
        enabled: status.state.enabled(),
        error: status.error.clone(),
    }
}

/// A panic inside a feature aborts the process in release builds
/// (`panic = "abort"`), so a poisoned lock is seen only in debug builds and
/// tests. The guarded data is plain state and still consistent enough to read.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lowercase ASCII letters and digits in hyphen-separated words.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// All registered features, in registration order.
#[derive(Default)]
pub struct FeatureRegistry {
    entries: Vec<Entry>,
}

impl FeatureRegistry {
    /// Registers `features` in order. A feature that cannot be registered is
    /// logged and skipped; the others are still registered.
    pub fn with_features(features: Vec<Box<dyn Feature>>) -> Self {
        let mut registry = Self::default();
        for feature in features {
            if let Err(e) = registry.register(feature) {
                eprintln!("sotyware: feature not registered: {e}");
            }
        }
        registry
    }

    /// Adds `feature` in state `Registered`. Nothing of it runs yet.
    pub fn register(&mut self, feature: Box<dyn Feature>) -> Result<(), RegisterError> {
        let metadata = feature.metadata();
        let id = metadata.id;
        if !valid_id(id) {
            return Err(RegisterError::InvalidId(id));
        }
        for (field, value) in [
            ("name", metadata.name),
            ("description", metadata.description),
            ("version", metadata.version),
        ] {
            if value.trim().is_empty() {
                return Err(RegisterError::EmptyField { id, field });
            }
        }
        if self.entries.iter().any(|e| e.metadata.id == id) {
            return Err(RegisterError::DuplicateId(id));
        }
        self.entries.push(Entry {
            metadata,
            status: Mutex::new(Status {
                state: FeatureState::Registered,
                error: None,
            }),
            feature: Mutex::new(feature),
        });
        Ok(())
    }

    /// Every feature with its current state, in registration order.
    pub fn list(&self) -> Vec<FeatureInfo> {
        self.entries.iter().map(Entry::info).collect()
    }

    /// The feature `id` with its current state, if it is registered.
    pub fn info(&self, id: &str) -> Option<FeatureInfo> {
        self.entries
            .iter()
            .find(|e| e.metadata.id == id)
            .map(Entry::info)
    }

    /// Startup: starts each still-`Registered` feature for which `enabled`
    /// says so, in registration order, and marks the others `Disabled`. A
    /// failing feature ends up `Failed`; the next one is started anyway.
    pub fn start_all(&self, enabled: impl Fn(&FeatureMetadata) -> bool) {
        for entry in &self.entries {
            let action = if enabled(&entry.metadata) {
                Action::Enable
            } else {
                Action::Disable
            };
            // Only fails if the frontend already switched this feature during
            // startup; its choice stands.
            let _ = self.apply(entry, action, true);
        }
    }

    /// Enables or disables the feature `id`. A failing `start` or `stop` is
    /// recorded in the returned state, not returned as an error.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<FeatureInfo, ControlError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.metadata.id == id)
            .ok_or_else(|| ControlError::UnknownFeature(id.to_owned()))?;
        let action = if enabled {
            Action::Enable
        } else {
            Action::Disable
        };
        self.apply(entry, action, false)
    }

    /// Exit: stops every running feature, in reverse registration order. Errors
    /// are logged; every feature is still attempted.
    pub fn stop_all(&self) {
        for entry in self.entries.iter().rev() {
            if lock(&entry.status).state == FeatureState::Running {
                // Fails only if the feature changed state in between; then it is
                // not running any more.
                let _ = self.apply(entry, Action::Disable, false);
            }
        }
    }

    fn apply(
        &self,
        entry: &Entry,
        action: Action,
        only_if_registered: bool,
    ) -> Result<FeatureInfo, ControlError> {
        let id = entry.metadata.id;
        // Check and claim the transition under one lock, so two concurrent
        // requests cannot both start (or both stop) the feature.
        let step = {
            let mut status = lock(&entry.status);
            let from = status.state;
            let step = plan(from, action)
                .filter(|_| !only_if_registered || from == FeatureState::Registered)
                .ok_or(ControlError::InvalidTransition {
                    id,
                    state: from,
                    action,
                })?;
            status.state = match step {
                Step::Start => FeatureState::Initializing,
                Step::Stop => FeatureState::Stopping,
                Step::Settle => FeatureState::Disabled,
            };
            if step == Step::Settle {
                status.error = None;
                return Ok(to_info(&entry.metadata, &status));
            }
            step
        };

        // The status lock is released: the feature's code runs under its own lock.
        let (state, error) = if step == Step::Start {
            match lock(&entry.feature).start() {
                Ok(()) => (FeatureState::Running, None),
                Err(e) => {
                    eprintln!("sotyware: feature '{id}' failed to start: {e}");
                    (FeatureState::Failed, Some(e.to_string()))
                }
            }
        } else {
            match lock(&entry.feature).stop() {
                Ok(()) => (FeatureState::Disabled, None),
                Err(e) => {
                    eprintln!("sotyware: feature '{id}' failed to stop cleanly: {e}");
                    (FeatureState::Disabled, Some(format!("stop failed: {e}")))
                }
            }
        };
        let mut status = lock(&entry.status);
        *status = Status { state, error };
        Ok(to_info(&entry.metadata, &status))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::thread;

    use super::super::FeatureError;
    use super::*;
    use FeatureState::*;

    /// Records `start`/`stop` calls in a log shared with the test; fails on
    /// request.
    struct TestFeature {
        metadata: FeatureMetadata,
        log: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
        fail_stop: bool,
    }

    fn meta(id: &'static str) -> FeatureMetadata {
        FeatureMetadata {
            id,
            name: "Test",
            description: "A test feature.",
            version: "1.0.0",
            enabled_by_default: true,
        }
    }

    impl TestFeature {
        fn new(id: &'static str, log: &Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                metadata: meta(id),
                log: Arc::clone(log),
                fail_start: false,
                fail_stop: false,
            }
        }

        fn failing_start(mut self) -> Self {
            self.fail_start = true;
            self
        }

        fn failing_stop(mut self) -> Self {
            self.fail_stop = true;
            self
        }

        fn boxed(self) -> Box<dyn Feature> {
            Box::new(self)
        }
    }

    impl Feature for TestFeature {
        fn metadata(&self) -> FeatureMetadata {
            self.metadata
        }

        fn start(&mut self) -> Result<(), FeatureError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("start {}", self.metadata.id));
            if self.fail_start {
                Err(FeatureError::new("port in use"))
            } else {
                Ok(())
            }
        }

        fn stop(&mut self) -> Result<(), FeatureError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("stop {}", self.metadata.id));
            if self.fail_stop {
                Err(FeatureError::new("handle leaked"))
            } else {
                Ok(())
            }
        }
    }

    fn new_log() -> Arc<Mutex<Vec<String>>> {
        Arc::default()
    }

    fn calls(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    fn states(registry: &FeatureRegistry) -> Vec<(&'static str, FeatureState)> {
        registry.list().iter().map(|i| (i.id, i.state)).collect()
    }

    fn state_of(registry: &FeatureRegistry, id: &str) -> FeatureInfo {
        registry
            .list()
            .into_iter()
            .find(|i| i.id == id)
            .expect("feature registered")
    }

    fn registry_with(features: Vec<Box<dyn Feature>>) -> FeatureRegistry {
        let mut registry = FeatureRegistry::default();
        for f in features {
            registry.register(f).expect("registers");
        }
        registry
    }

    // --- registration -------------------------------------------------------

    #[test]
    fn registered_features_are_listed_in_order_with_metadata_and_nothing_runs() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("beta", &log).boxed(),
            TestFeature::new("alpha", &log).boxed(),
        ]);
        let list = registry.list();
        assert_eq!(
            list.iter().map(|i| i.id).collect::<Vec<_>>(),
            ["beta", "alpha"],
            "registration order, not sorted"
        );
        assert_eq!(
            list[0],
            FeatureInfo {
                id: "beta",
                name: "Test",
                description: "A test feature.",
                version: "1.0.0",
                state: Registered,
                enabled: false,
                error: None,
            }
        );
        assert!(
            calls(&log).is_empty(),
            "registering must not start anything"
        );
    }

    #[test]
    fn info_returns_one_feature_as_listed_or_none_for_an_unknown_id() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("alpha", &log).boxed(),
            TestFeature::new("beta", &log).boxed(),
        ]);
        registry.set_enabled("beta", true).unwrap();
        assert_eq!(registry.info("beta"), Some(state_of(&registry, "beta")));
        assert_eq!(registry.info("beta").unwrap().state, Running);
        assert_eq!(registry.info("gamma"), None);
    }

    #[test]
    fn duplicate_id_is_rejected_and_the_first_one_kept() {
        let log = new_log();
        let mut registry = registry_with(vec![TestFeature::new("dup", &log).boxed()]);
        assert_eq!(
            registry.register(TestFeature::new("dup", &log).failing_start().boxed()),
            Err(RegisterError::DuplicateId("dup"))
        );
        assert_eq!(registry.list().len(), 1);
        // The kept one is the first (whose start succeeds).
        registry.start_all(|_| true);
        assert_eq!(states(&registry), [("dup", Running)]);
    }

    #[test]
    fn with_features_skips_a_rejected_feature_and_keeps_the_rest() {
        let log = new_log();
        let registry = FeatureRegistry::with_features(vec![
            TestFeature::new("one", &log).boxed(),
            TestFeature::new("one", &log).boxed(),
            TestFeature::new("Bad_Id", &log).boxed(),
            TestFeature::new("two", &log).boxed(),
        ]);
        assert_eq!(
            states(&registry),
            [("one", Registered), ("two", Registered)]
        );
    }

    #[test]
    fn ids_must_be_lowercase_kebab_case() {
        for id in ["example", "clipboard-history", "v2", "a-1-b"] {
            assert!(valid_id(id), "{id}");
        }
        for id in [
            "",
            "Example",
            "with space",
            "under_score",
            "-lead",
            "trail-",
            "double--hyphen",
            "ümlaut",
        ] {
            assert!(!valid_id(id), "{id:?}");
        }
        let mut registry = FeatureRegistry::default();
        assert_eq!(
            registry.register(TestFeature::new("Nope", &new_log()).boxed()),
            Err(RegisterError::InvalidId("Nope"))
        );
    }

    #[test]
    fn name_description_and_version_must_not_be_empty() {
        for field in ["name", "description", "version"] {
            let mut feature = TestFeature::new("x", &new_log());
            match field {
                "name" => feature.metadata.name = " ",
                "description" => feature.metadata.description = "",
                _ => feature.metadata.version = "",
            }
            let mut registry = FeatureRegistry::default();
            assert_eq!(
                registry.register(feature.boxed()),
                Err(RegisterError::EmptyField { id: "x", field })
            );
        }
    }

    // --- startup ------------------------------------------------------------

    #[test]
    fn startup_starts_enabled_features_and_never_starts_disabled_ones() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("on", &log).boxed(),
            TestFeature::new("off", &log).boxed(),
        ]);
        registry.start_all(|m| m.id == "on");
        assert_eq!(states(&registry), [("on", Running), ("off", Disabled)]);
        assert_eq!(calls(&log), ["start on"]);
    }

    #[test]
    fn startup_runs_only_once_per_feature() {
        let log = new_log();
        let registry = registry_with(vec![TestFeature::new("a", &log).boxed()]);
        registry.start_all(|_| true);
        registry.start_all(|_| true);
        registry.start_all(|_| false);
        assert_eq!(states(&registry), [("a", Running)]);
        assert_eq!(calls(&log), ["start a"]);
    }

    #[test]
    fn a_failing_feature_does_not_stop_the_others() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("first", &log).boxed(),
            TestFeature::new("broken", &log).failing_start().boxed(),
            TestFeature::new("last", &log).boxed(),
        ]);
        registry.start_all(|_| true);
        assert_eq!(
            states(&registry),
            [("first", Running), ("broken", Failed), ("last", Running)]
        );
        assert_eq!(calls(&log), ["start first", "start broken", "start last"]);
        let broken = state_of(&registry, "broken");
        assert_eq!(broken.error.as_deref(), Some("port in use"));
        assert!(broken.enabled, "the user's intent is still 'on'");

        // Disabling and re-enabling the others is unaffected by the failed one.
        assert_eq!(registry.set_enabled("last", false).unwrap().state, Disabled);
        assert_eq!(registry.set_enabled("last", true).unwrap().state, Running);
        assert_eq!(state_of(&registry, "first").state, Running);
    }

    // --- enable / disable ---------------------------------------------------

    #[test]
    fn disable_stops_a_running_feature_and_enable_starts_it_again() {
        let log = new_log();
        let registry = registry_with(vec![TestFeature::new("a", &log).boxed()]);
        registry.start_all(|_| true);

        let info = registry.set_enabled("a", false).unwrap();
        assert_eq!(
            (info.state, info.enabled, info.error),
            (Disabled, false, None)
        );
        let info = registry.set_enabled("a", true).unwrap();
        assert_eq!(
            (info.state, info.enabled, info.error),
            (Running, true, None)
        );
        assert_eq!(calls(&log), ["start a", "stop a", "start a"]);
    }

    #[test]
    fn enabling_a_disabled_by_default_feature_starts_it() {
        let log = new_log();
        let registry = registry_with(vec![TestFeature::new("a", &log).boxed()]);
        registry.start_all(|_| false);
        assert!(calls(&log).is_empty());
        assert_eq!(registry.set_enabled("a", true).unwrap().state, Running);
        assert_eq!(calls(&log), ["start a"]);
    }

    #[test]
    fn invalid_transitions_are_rejected_without_calling_the_feature() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("on", &log).boxed(),
            TestFeature::new("off", &log).boxed(),
        ]);
        registry.start_all(|m| m.id == "on");
        let before = calls(&log);

        assert_eq!(
            registry.set_enabled("on", true),
            Err(ControlError::InvalidTransition {
                id: "on",
                state: Running,
                action: Action::Enable
            })
        );
        assert_eq!(
            registry.set_enabled("off", false),
            Err(ControlError::InvalidTransition {
                id: "off",
                state: Disabled,
                action: Action::Disable
            })
        );
        assert_eq!(
            calls(&log),
            before,
            "no start/stop for a rejected transition"
        );
        assert_eq!(states(&registry), [("on", Running), ("off", Disabled)]);
    }

    #[test]
    fn unknown_feature_is_an_error() {
        let registry = registry_with(vec![TestFeature::new("a", &new_log()).boxed()]);
        let err = registry.set_enabled("nope", true).unwrap_err();
        assert_eq!(err, ControlError::UnknownFeature("nope".into()));
        assert_eq!(err.to_string(), "unknown feature 'nope'");
    }

    #[test]
    fn error_messages_name_the_feature_and_the_state() {
        let e = ControlError::InvalidTransition {
            id: "a",
            state: Running,
            action: Action::Enable,
        };
        assert_eq!(
            e.to_string(),
            "cannot enable feature 'a' while it is Running"
        );
        assert_eq!(
            RegisterError::DuplicateId("a").to_string(),
            "duplicate feature id 'a'"
        );
    }

    #[test]
    fn lifecycle_table() {
        use Action::*;
        let all = [
            Registered,
            Disabled,
            Initializing,
            Running,
            Failed,
            Stopping,
        ];
        for from in all {
            let expected_enable = match from {
                Registered | Disabled | Failed => Some(Step::Start),
                _ => None,
            };
            let expected_disable = match from {
                Running => Some(Step::Stop),
                Registered | Failed => Some(Step::Settle),
                _ => None,
            };
            assert_eq!(plan(from, Enable), expected_enable, "enable from {from:?}");
            assert_eq!(
                plan(from, Disable),
                expected_disable,
                "disable from {from:?}"
            );
        }
    }

    // --- failures -----------------------------------------------------------

    #[test]
    fn a_failed_feature_can_be_retried_or_disabled_without_calling_stop() {
        let log = new_log();
        let registry = registry_with(vec![TestFeature::new("a", &log).failing_start().boxed()]);
        registry.start_all(|_| true);
        assert_eq!(state_of(&registry, "a").state, Failed);

        // Retry: start is called again, fails again.
        let info = registry.set_enabled("a", true).unwrap();
        assert_eq!(
            (info.state, info.error.as_deref()),
            (Failed, Some("port in use"))
        );

        // Disable: start failed, so nothing runs and stop is not called; the
        // stale error is cleared.
        let info = registry.set_enabled("a", false).unwrap();
        assert_eq!((info.state, info.error), (Disabled, None));
        assert_eq!(calls(&log), ["start a", "start a"]);
    }

    #[test]
    fn a_failing_stop_leaves_the_feature_disabled_with_the_error() {
        let log = new_log();
        let registry = registry_with(vec![TestFeature::new("a", &log).failing_stop().boxed()]);
        registry.start_all(|_| true);
        let info = registry.set_enabled("a", false).unwrap();
        assert_eq!(
            (info.state, info.enabled, info.error.as_deref()),
            (Disabled, false, Some("stop failed: handle leaked"))
        );
        // The next successful transition clears the error.
        let info = registry.set_enabled("a", true).unwrap();
        assert_eq!((info.state, info.error), (Running, None));
    }

    // --- shutdown -----------------------------------------------------------

    #[test]
    fn stop_all_stops_only_running_features_in_reverse_order() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("a", &log).boxed(),
            TestFeature::new("off", &log).boxed(),
            TestFeature::new("broken", &log).failing_start().boxed(),
            TestFeature::new("b", &log).boxed(),
        ]);
        registry.start_all(|m| m.id != "off");
        log.lock().unwrap().clear();

        registry.stop_all();
        assert_eq!(calls(&log), ["stop b", "stop a"]);
        assert_eq!(
            states(&registry),
            [
                ("a", Disabled),
                ("off", Disabled),
                ("broken", Failed),
                ("b", Disabled)
            ]
        );
    }

    #[test]
    fn stop_all_continues_after_a_failing_stop() {
        let log = new_log();
        let registry = registry_with(vec![
            TestFeature::new("a", &log).boxed(),
            TestFeature::new("b", &log).failing_stop().boxed(),
        ]);
        registry.start_all(|_| true);
        log.lock().unwrap().clear();
        registry.stop_all();
        assert_eq!(calls(&log), ["stop b", "stop a"]);
        assert_eq!(states(&registry), [("a", Disabled), ("b", Disabled)]);
    }

    // --- concurrency --------------------------------------------------------

    /// Blocks in `start` until the test releases it.
    struct SlowFeature {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl Feature for SlowFeature {
        fn metadata(&self) -> FeatureMetadata {
            meta("slow")
        }

        fn start(&mut self) -> Result<(), FeatureError> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(())
        }

        fn stop(&mut self) -> Result<(), FeatureError> {
            Ok(())
        }
    }

    #[test]
    fn a_feature_that_is_starting_is_visible_busy_and_blocks_nothing_else() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let log = new_log();
        let registry = Arc::new(registry_with(vec![
            Box::new(SlowFeature {
                entered: entered_tx,
                release: release_rx,
            }),
            TestFeature::new("other", &log).boxed(),
        ]));
        registry.start_all(|m| m.id == "other");

        let worker = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || registry.set_enabled("slow", true))
        };
        entered_rx.recv().unwrap();

        // While `start` runs: the list is readable and shows Initializing ...
        assert_eq!(
            states(&registry),
            [("slow", Initializing), ("other", Running)]
        );
        // ... a second request for the same feature is rejected, not queued ...
        assert_eq!(
            registry.set_enabled("slow", true),
            Err(ControlError::InvalidTransition {
                id: "slow",
                state: Initializing,
                action: Action::Enable
            })
        );
        assert!(registry.set_enabled("slow", false).is_err());
        // ... and another feature can still be changed.
        assert_eq!(
            registry.set_enabled("other", false).unwrap().state,
            Disabled
        );

        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap().unwrap().state, Running);
        assert_eq!(states(&registry), [("slow", Running), ("other", Disabled)]);
    }
}

//! Feature preferences: whether the user wants each feature on, kept across
//! restarts.
//!
//! This module is the narrow layer between the feature commands/lifecycle and
//! storage. It knows the policy (stored choice, else the feature's default;
//! what a storage failure means), but not how anything is stored: storage is
//! a [`PreferenceStore`] (the SQLite one is in `crate::storage`). The
//! [`FeatureRegistry`] knows neither; it only receives the startup decision.
//! Policy and rationale: docs/architecture/features.md#persistence, ADR-015.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{FeatureMetadata, FeatureRegistry};

/// Why preferences could not be read or written. Shown to the user as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError(String);

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where preferences live. Implemented by the SQLite store; tests use an
/// in-memory double. Calls are serialized by [`FeaturePreferences`].
pub trait PreferenceStore: Send {
    /// Every stored `(feature id, enabled)` pair, including ids no built-in
    /// feature has (any more).
    fn load(&mut self) -> Result<Vec<(String, bool)>, StoreError>;

    /// Stores one choice atomically: afterwards the old or the new value is
    /// stored, never anything else.
    fn save(&mut self, id: &str, enabled: bool) -> Result<(), StoreError>;
}

enum Slot {
    Open(Box<dyn PreferenceStore>),
    /// Could not be opened or read at startup. Never written to: a store that
    /// could not be read (corrupt, newer schema) must not be changed blindly.
    Unavailable(StoreError),
    /// sotyware is exiting; see [`FeaturePreferences::close`].
    Closed,
}

/// The stored preferences, read once at startup, and the store that saves
/// changes. Managed as Tauri state.
pub struct FeaturePreferences {
    /// What was stored at startup, for the features that exist.
    stored: HashMap<String, bool>,
    /// Why nothing could be loaded; then every feature uses its default.
    load_error: Option<StoreError>,
    /// One lock for every save: a save reads the feature's current state and
    /// writes it while holding it, so the last write always matches the last
    /// change, whatever order concurrent switches finish in.
    store: Mutex<Slot>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FeaturePreferences {
    /// Loads what `store` holds for the features in `known_ids`. If the store
    /// could not be opened or read, the reason is logged and kept: features
    /// then use their defaults and every save fails with that reason.
    /// Stored ids not in `known_ids` are ignored and left in the store, so a
    /// feature that comes back (for example after a downgrade) keeps its choice.
    pub fn load(store: Result<Box<dyn PreferenceStore>, StoreError>, known_ids: &[&str]) -> Self {
        let loaded = store.and_then(|mut store| store.load().map(|rows| (store, rows)));
        match loaded {
            Ok((store, rows)) => {
                let mut stored = HashMap::new();
                for (id, enabled) in rows {
                    if known_ids.contains(&id.as_str()) {
                        stored.insert(id, enabled);
                    } else {
                        eprintln!(
                            "sotyware: ignoring stored preference for unknown feature '{id}'"
                        );
                    }
                }
                Self {
                    stored,
                    load_error: None,
                    store: Mutex::new(Slot::Open(store)),
                }
            }
            Err(e) => {
                eprintln!("sotyware: feature preferences unavailable, using defaults: {e}");
                Self {
                    stored: HashMap::new(),
                    load_error: Some(e.clone()),
                    store: Mutex::new(Slot::Unavailable(e)),
                }
            }
        }
    }

    /// Whether a feature starts when sotyware starts: the stored choice, or the
    /// feature's `enabled_by_default` when there is none (first launch, a new
    /// feature, or storage unavailable).
    pub fn enabled_at_startup(&self, metadata: &FeatureMetadata) -> bool {
        self.stored
            .get(metadata.id)
            .copied()
            .unwrap_or(metadata.enabled_by_default)
    }

    /// Why stored preferences could not be loaded, if they could not.
    pub fn load_error(&self) -> Option<&StoreError> {
        self.load_error.as_ref()
    }

    /// Saves the user's current intent for `id` (its `enabled` in the
    /// registry, so a failed start is saved as enabled). Called after each
    /// successful switch. Reading and writing under one lock keeps concurrent
    /// switches from leaving a stale value stored.
    pub fn save_current(&self, registry: &FeatureRegistry, id: &str) -> Result<(), StoreError> {
        let mut slot = lock(&self.store);
        let store = match &mut *slot {
            Slot::Open(store) => store,
            Slot::Unavailable(e) => {
                return Err(StoreError::new(format!("preferences are unavailable: {e}")));
            }
            Slot::Closed => return Err(StoreError::new("sotyware is quitting")),
        };
        let Some(info) = registry.info(id) else {
            return Err(StoreError::new(format!("unknown feature '{id}'")));
        };
        store.save(id, info.enabled)
    }

    /// Exit: waits for a save in progress, then refuses further saves. Every
    /// completed save is already durable; this only keeps a save from being
    /// cut off halfway by the process ending (which SQLite would roll back).
    pub fn close(&self) {
        *lock(&self.store) = Slot::Closed;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::super::{Feature, FeatureError, FeatureState, switch_feature};
    use super::*;

    /// In-memory store whose rows the test can see; fails on request.
    #[derive(Clone, Default)]
    struct MemoryStore {
        pub rows: Arc<Mutex<Vec<(String, bool)>>>,
        pub saves: Arc<Mutex<usize>>,
        pub fail_load: bool,
        pub fail_save: Arc<Mutex<bool>>,
    }

    impl MemoryStore {
        pub fn with(rows: &[(&str, bool)]) -> Self {
            let store = Self::default();
            *store.rows.lock().unwrap() = rows.iter().map(|(i, e)| (i.to_string(), *e)).collect();
            store
        }

        pub fn get(&self, id: &str) -> Option<bool> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, e)| *e)
        }

        pub fn boxed(&self) -> Result<Box<dyn PreferenceStore>, StoreError> {
            Ok(Box::new(self.clone()))
        }
    }

    impl PreferenceStore for MemoryStore {
        fn load(&mut self) -> Result<Vec<(String, bool)>, StoreError> {
            if self.fail_load {
                return Err(StoreError::new("file is not a database"));
            }
            Ok(self.rows.lock().unwrap().clone())
        }

        fn save(&mut self, id: &str, enabled: bool) -> Result<(), StoreError> {
            *self.saves.lock().unwrap() += 1;
            if *self.fail_save.lock().unwrap() {
                return Err(StoreError::new("disk full"));
            }
            let mut rows = self.rows.lock().unwrap();
            match rows.iter_mut().find(|(i, _)| i == id) {
                Some(row) => row.1 = enabled,
                None => rows.push((id.to_owned(), enabled)),
            }
            Ok(())
        }
    }

    struct Flag {
        id: &'static str,
        default: bool,
        fail_start: bool,
        running: bool,
    }

    impl Flag {
        pub fn boxed(id: &'static str, default: bool) -> Box<dyn Feature> {
            Box::new(Self {
                id,
                default,
                fail_start: false,
                running: false,
            })
        }

        fn failing(id: &'static str) -> Box<dyn Feature> {
            Box::new(Self {
                id,
                default: true,
                fail_start: true,
                running: false,
            })
        }
    }

    impl Feature for Flag {
        fn metadata(&self) -> FeatureMetadata {
            FeatureMetadata {
                id: self.id,
                name: "Flag",
                description: "A test feature.",
                version: "1.0.0",
                enabled_by_default: self.default,
            }
        }

        fn start(&mut self) -> Result<(), FeatureError> {
            if self.fail_start {
                return Err(FeatureError::new("port in use"));
            }
            assert!(!self.running, "started twice");
            self.running = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), FeatureError> {
            assert!(self.running, "stopped while not running");
            self.running = false;
            Ok(())
        }
    }

    fn ids(registry: &FeatureRegistry) -> Vec<&'static str> {
        registry.list().iter().map(|i| i.id).collect()
    }

    fn state(registry: &FeatureRegistry, id: &str) -> FeatureState {
        registry.info(id).expect("registered").state
    }

    /// The startup sequence of `run()`: load, then start with the result.
    fn start(
        features: Vec<Box<dyn Feature>>,
        store: Result<Box<dyn PreferenceStore>, StoreError>,
    ) -> (FeatureRegistry, FeaturePreferences) {
        let registry = FeatureRegistry::with_features(features);
        let prefs = FeaturePreferences::load(store, &ids(&registry));
        registry.start_all(|m| prefs.enabled_at_startup(m));
        (registry, prefs)
    }

    #[test]
    fn without_a_stored_choice_each_feature_uses_its_default() {
        let (registry, prefs) = start(
            vec![Flag::boxed("on", true), Flag::boxed("off", false)],
            MemoryStore::default().boxed(),
        );
        assert_eq!(state(&registry, "on"), FeatureState::Running);
        assert_eq!(state(&registry, "off"), FeatureState::Disabled);
        assert_eq!(prefs.load_error(), None);
    }

    #[test]
    fn a_stored_choice_overrides_the_default_both_ways() {
        let store = MemoryStore::with(&[("on", false), ("off", true)]);
        let (registry, _) = start(
            vec![Flag::boxed("on", true), Flag::boxed("off", false)],
            store.boxed(),
        );
        assert_eq!(state(&registry, "on"), FeatureState::Disabled);
        assert_eq!(state(&registry, "off"), FeatureState::Running);
        assert_eq!(*store.saves.lock().unwrap(), 0, "startup writes nothing");
    }

    #[test]
    fn a_new_feature_without_a_stored_choice_uses_its_default() {
        let store = MemoryStore::with(&[("old", false)]);
        let (registry, _) = start(
            vec![Flag::boxed("old", true), Flag::boxed("new", true)],
            store.boxed(),
        );
        assert_eq!(state(&registry, "old"), FeatureState::Disabled);
        assert_eq!(state(&registry, "new"), FeatureState::Running);
    }

    #[test]
    fn unknown_stored_ids_are_ignored_and_kept() {
        let store = MemoryStore::with(&[("gone", false), ("on", false)]);
        let (registry, prefs) = start(vec![Flag::boxed("on", true)], store.boxed());
        assert_eq!(ids(&registry), ["on"], "a stored id never adds a feature");
        assert_eq!(prefs.load_error(), None);
        switch_feature(&registry, &prefs, "on", true).unwrap();
        assert_eq!(store.get("gone"), Some(false), "left in the store");
        assert_eq!(store.get("on"), Some(true));
    }

    #[test]
    fn a_successful_switch_is_saved() {
        let store = MemoryStore::default();
        let (registry, prefs) = start(vec![Flag::boxed("a", true)], store.boxed());
        let change = switch_feature(&registry, &prefs, "a", false).unwrap();
        assert_eq!(
            (change.feature.state, change.save_error),
            (FeatureState::Disabled, None)
        );
        assert_eq!(store.get("a"), Some(false));
        switch_feature(&registry, &prefs, "a", true).unwrap();
        assert_eq!(store.get("a"), Some(true));
    }

    #[test]
    fn a_rejected_switch_saves_nothing() {
        let store = MemoryStore::default();
        let (registry, prefs) = start(vec![Flag::boxed("a", true)], store.boxed());
        assert_eq!(
            switch_feature(&registry, &prefs, "a", true).unwrap_err(),
            "cannot enable feature 'a' while it is Running"
        );
        assert_eq!(
            switch_feature(&registry, &prefs, "nope", true).unwrap_err(),
            "unknown feature 'nope'"
        );
        assert_eq!(*store.saves.lock().unwrap(), 0);
    }

    #[test]
    fn a_failed_start_keeps_the_enabled_preference() {
        let store = MemoryStore::with(&[("bad", false)]);
        let (registry, prefs) = start(vec![Flag::failing("bad")], store.boxed());
        let change = switch_feature(&registry, &prefs, "bad", true).unwrap();
        assert_eq!(change.feature.state, FeatureState::Failed);
        assert_eq!(change.save_error, None);
        assert_eq!(
            store.get("bad"),
            Some(true),
            "the intent is on; next start retries"
        );
        // And the next start does retry it.
        let (registry, _) = start(vec![Flag::failing("bad")], store.boxed());
        assert_eq!(state(&registry, "bad"), FeatureState::Failed);
    }

    #[test]
    fn a_failed_save_keeps_the_switch_reports_it_and_leaves_the_store_unchanged() {
        let store = MemoryStore::with(&[("a", true)]);
        let (registry, prefs) = start(vec![Flag::boxed("a", true)], store.boxed());
        *store.fail_save.lock().unwrap() = true;
        let change = switch_feature(&registry, &prefs, "a", false).unwrap();
        assert_eq!(
            change.feature.state,
            FeatureState::Disabled,
            "switched anyway"
        );
        assert_eq!(change.save_error.as_deref(), Some("disk full"));
        assert_eq!(state(&registry, "a"), FeatureState::Disabled);
        assert_eq!(store.get("a"), Some(true), "the old choice stays stored");
        // The next start uses the stored (old) choice.
        let (registry, _) = start(vec![Flag::boxed("a", true)], store.boxed());
        assert_eq!(state(&registry, "a"), FeatureState::Running);
    }

    #[test]
    fn an_unreadable_store_means_defaults_and_no_writes() {
        let store = MemoryStore {
            fail_load: true,
            ..MemoryStore::with(&[("on", false)])
        };
        let (registry, prefs) = start(vec![Flag::boxed("on", true)], store.boxed());
        assert_eq!(state(&registry, "on"), FeatureState::Running, "default");
        assert_eq!(
            prefs.load_error().map(ToString::to_string).as_deref(),
            Some("file is not a database")
        );
        let change = switch_feature(&registry, &prefs, "on", false).unwrap();
        assert_eq!(change.feature.state, FeatureState::Disabled);
        assert_eq!(
            change.save_error.as_deref(),
            Some("preferences are unavailable: file is not a database")
        );
        assert_eq!(*store.saves.lock().unwrap(), 0, "never written");
        assert_eq!(store.get("on"), Some(false));
    }

    #[test]
    fn a_store_that_cannot_be_opened_means_defaults() {
        let (registry, prefs) = start(
            vec![Flag::boxed("on", true), Flag::boxed("off", false)],
            Err(StoreError::new("cannot create directory")),
        );
        assert_eq!(state(&registry, "on"), FeatureState::Running);
        assert_eq!(state(&registry, "off"), FeatureState::Disabled);
        let listed = super::super::feature_list(&registry, &prefs);
        assert_eq!(
            listed.storage_error.as_deref(),
            Some("cannot create directory")
        );
        let change = switch_feature(&registry, &prefs, "off", true).unwrap();
        assert_eq!(
            change.save_error.as_deref(),
            Some("preferences are unavailable: cannot create directory")
        );
    }

    #[test]
    fn stopping_at_exit_does_not_change_the_preference() {
        let store = MemoryStore::default();
        let (registry, prefs) = start(vec![Flag::boxed("a", true)], store.boxed());
        switch_feature(&registry, &prefs, "a", false).unwrap();
        switch_feature(&registry, &prefs, "a", true).unwrap();
        let saves = *store.saves.lock().unwrap();
        // The order of RunEvent::Exit in run().
        registry.stop_all();
        prefs.close();
        assert_eq!(state(&registry, "a"), FeatureState::Disabled);
        assert_eq!(store.get("a"), Some(true), "still on for the next start");
        assert_eq!(*store.saves.lock().unwrap(), saves);
    }

    #[test]
    fn after_close_saves_fail_instead_of_racing_the_exit() {
        let store = MemoryStore::default();
        let (registry, prefs) = start(vec![Flag::boxed("a", true)], store.boxed());
        prefs.close();
        let change = switch_feature(&registry, &prefs, "a", false).unwrap();
        assert_eq!(change.save_error.as_deref(), Some("sotyware is quitting"));
        assert_eq!(store.get("a"), None);
    }

    /// Checks the invariant under contention. It cannot force the one
    /// interleaving a stale save needs (a later switch's save taking the lock
    /// first); that is excluded by construction: `save_current` reads the
    /// state and writes it under one lock (ADR-015).
    #[test]
    fn concurrent_switches_leave_each_stored_value_equal_to_the_final_state() {
        const IDS: [&str; 4] = ["a", "b", "c", "d"];
        let store = MemoryStore::default();
        let (registry, prefs) = start(
            IDS.iter().map(|id| Flag::boxed(id, true)).collect(),
            store.boxed(),
        );
        let shared = Arc::new((registry, prefs));
        // Several threads per feature, all flipping it: many requests are
        // rejected as busy or already-there, the rest race to save.
        let handles: Vec<_> = (0..16)
            .map(|t| {
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let (registry, prefs) = &*shared;
                    for i in 0..200 {
                        let id = IDS[(t + i) % IDS.len()];
                        let _ = switch_feature(registry, prefs, id, (t + i) % 3 == 0);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let (registry, _) = &*shared;
        for id in IDS {
            let info = registry.info(id).unwrap();
            assert!(
                matches!(info.state, FeatureState::Running | FeatureState::Disabled),
                "{id}: {:?}",
                info.state
            );
            assert_eq!(store.get(id).unwrap_or(true), info.enabled, "{id}");
        }
    }
}

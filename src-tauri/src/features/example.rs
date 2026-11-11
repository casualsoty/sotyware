//! Example built-in feature: the smallest real use of the feature lifecycle.
//!
//! It does nothing visible. It exists to show what a feature looks like and to
//! give the dashboard and the desktop tests one feature to list, start and stop.
//! Its one "resource" is the flag it sets in `start` and clears in `stop`, which
//! lets its tests check that the registry never starts or stops it twice.

use super::{Feature, FeatureError, FeatureMetadata};

#[derive(Default)]
pub struct Example {
    running: bool,
}

impl Feature for Example {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "example",
            name: "Example",
            description: "Demonstrates the built-in feature lifecycle. It does nothing visible.",
            version: "0.1.0",
            enabled_by_default: true,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        if self.running {
            return Err(FeatureError::new("already running"));
        }
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if !self.running {
            return Err(FeatureError::new("not running"));
        }
        self.running = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FeatureRegistry, FeatureState};
    use super::*;

    #[test]
    fn starts_and_stops_and_refuses_to_do_either_twice() {
        let mut example = Example::default();
        assert_eq!(example.stop(), Err(FeatureError::new("not running")));
        assert_eq!(example.start(), Ok(()));
        assert_eq!(example.start(), Err(FeatureError::new("already running")));
        assert_eq!(example.stop(), Ok(()));
        assert!(!example.running);
    }

    #[test]
    fn through_the_registry_it_starts_when_enabled_and_stops_when_disabled() {
        let registry = FeatureRegistry::with_features(vec![Box::new(Example::default())]);
        let info = &registry.list()[0];
        assert_eq!(
            (info.id, info.name, info.version, info.state),
            ("example", "Example", "0.1.0", FeatureState::Registered)
        );
        registry.start_all(|m| m.enabled_by_default);
        assert_eq!(registry.list()[0].state, FeatureState::Running);
        // `Example` errors on a double start/stop, so `error: None` after each
        // step shows the registry called each exactly once.
        let info = registry.set_enabled("example", false).unwrap();
        assert_eq!((info.state, info.error), (FeatureState::Disabled, None));
        let info = registry.set_enabled("example", true).unwrap();
        assert_eq!((info.state, info.error), (FeatureState::Running, None));
    }
}

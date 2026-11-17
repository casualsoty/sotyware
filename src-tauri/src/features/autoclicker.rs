//! The `autoclicker` built-in feature (ADR-050): while it runs, the input
//! backend is open and the autoclicker's global shortcuts are registered,
//! so the autoclicker, the recorder and macro playback can run. Off by
//! default: until the user turns it on, sotyware registers no shortcut and
//! sends or records no input. Switched off (or quitting), any automation
//! stops, every key and button it held is released and the shortcuts are
//! unregistered.
//!
//! The service lives in `crate::automation` ([`Automation`]); this feature
//! only starts and stops it through the link `run()` sets before the event
//! loop. Without that link (unit tests), starting and stopping do nothing.
//!
//! [`Automation`]: crate::automation::Automation

use super::{Feature, FeatureError, FeatureMetadata};
use crate::automation::LINK;

#[derive(Default)]
pub struct Autoclicker;

impl Feature for Autoclicker {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "autoclicker",
            name: "Autoclicker",
            description: "Clicks, records and replays mouse and keyboard input, with global shortcuts, while sotyware is running. Off until you turn it on.",
            version: "0.1.0",
            enabled_by_default: false,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        match LINK.get() {
            Some(automation) => automation.start().map_err(FeatureError::new),
            None => Ok(()),
        }
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if let Some(automation) = LINK.get() {
            automation.stop();
        }
        Ok(())
    }
}

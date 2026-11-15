//! The `update-check` built-in feature (ADR-033): while it runs, sotyware
//! checks for a new version one minute after startup and then once a day
//! (one timer; no polling), when an update source is configured. Switched
//! off, sotyware contacts the update server only when the user clicks
//! "Check for updates". Nothing is ever installed without the user's click.
//!
//! The checks live in `crate::updates` ([`Updates`]); this feature only
//! starts and stops them through the link `run()` sets before the event
//! loop. Without that link (unit tests), starting and stopping do nothing.
//!
//! [`Updates`]: crate::updates::Updates

use super::{Feature, FeatureError, FeatureMetadata};
use crate::updates::CHECK_LINK;

#[derive(Default)]
pub struct UpdateCheck;

impl Feature for UpdateCheck {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "update-check",
            name: "Automatic update checks",
            description: "Checks for a new sotyware version shortly after startup and once a day, while sotyware is running. Updates are installed only when you click.",
            version: "0.1.0",
            enabled_by_default: true,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        if let Some((updates, current)) = CHECK_LINK.get() {
            updates.start_checks(current.clone());
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if let Some((updates, _)) = CHECK_LINK.get() {
            updates.stop_checks();
        }
        Ok(())
    }
}

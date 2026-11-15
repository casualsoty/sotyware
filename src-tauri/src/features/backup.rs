//! The `backup-scheduler` built-in feature (ADR-031): while it runs, backup
//! jobs with an interval run when they are due (one timer that sleeps until
//! the next job is due; no polling). Switched off, no backup starts by
//! itself; "Back up now" on the Backup page still works.
//!
//! The scheduler itself lives in `crate::backup` ([`Backups`]); this feature
//! only starts and stops it through the link `run()` sets before the event
//! loop. Without that link (unit tests), starting and stopping do nothing.
//!
//! [`Backups`]: crate::backup::Backups

use super::{Feature, FeatureError, FeatureMetadata};
use crate::backup::SCHEDULER_LINK;

#[derive(Default)]
pub struct BackupScheduler;

impl Feature for BackupScheduler {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "backup-scheduler",
            name: "Scheduled backups",
            description: "Runs backup jobs that have an interval when they are due, while sotyware is running.",
            version: "0.1.0",
            enabled_by_default: true,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        if let Some((backups, accounts)) = SCHEDULER_LINK.get() {
            backups.start_scheduler(std::sync::Arc::clone(accounts));
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if let Some((backups, _)) = SCHEDULER_LINK.get() {
            backups.stop_scheduler();
        }
        Ok(())
    }
}

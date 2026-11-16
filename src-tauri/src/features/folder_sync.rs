//! The `folder-sync` built-in feature (ADR-043): while it runs, every sync
//! folder that is not paused is kept in step with its bucket (watching the
//! folder, checking the bucket at its interval). Switched off, no sync
//! runs; the syncs stay configured and go on when it is switched on again.
//!
//! The engines live in `crate::folder_sync` ([`FolderSyncs`]); this
//! feature only starts and stops them through the link `run()` sets before
//! the event loop. Without that link (unit tests), starting and stopping do
//! nothing.
//!
//! [`FolderSyncs`]: crate::folder_sync::FolderSyncs

use super::{Feature, FeatureError, FeatureMetadata};
use crate::folder_sync::LINK;

#[derive(Default)]
pub struct FolderSync;

impl Feature for FolderSync {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "folder-sync",
            name: "Folder sync",
            description: "Keeps the folders you chose on this computer in step with their buckets, in both directions, while sotyware is running.",
            version: "0.1.0",
            enabled_by_default: true,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        if let Some(syncs) = LINK.get() {
            syncs.start_all();
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if let Some(syncs) = LINK.get() {
            syncs.stop_all();
        }
        Ok(())
    }
}

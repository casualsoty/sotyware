//! The `notes-overlay` built-in feature (ADR-052): while it runs, the
//! gaming notes overlay exists. Its global shortcuts are registered and
//! the overlay window can be shown (it is shown at once when the user
//! turns the feature on, and at startup only if its settings ask for it).
//! Off by default: until the user turns it on, sotyware registers none of
//! its shortcuts and creates no overlay window. Switched off (or quitting),
//! the shortcuts are unregistered and the window is destroyed; the notes
//! stay in `notes.sqlite3`.
//!
//! The service lives in `crate::overlay` ([`NotesOverlay`]); this feature
//! only starts and stops it through the link `run()` sets before the
//! event loop. Without that link (unit tests), starting and stopping do
//! nothing.
//!
//! [`NotesOverlay`]: crate::overlay::NotesOverlay

use super::{Feature, FeatureError, FeatureMetadata};
use crate::overlay::LINK;

#[derive(Default)]
pub struct NotesOverlay;

impl Feature for NotesOverlay {
    fn metadata(&self) -> FeatureMetadata {
        FeatureMetadata {
            id: "notes-overlay",
            name: "Gaming notes overlay",
            description: "Keeps your notes and checklists in a small window above games and other programs, with global shortcuts. Off until you turn it on.",
            version: "0.1.0",
            enabled_by_default: false,
        }
    }

    fn start(&mut self) -> Result<(), FeatureError> {
        match LINK.get() {
            Some(overlay) => overlay.start().map_err(FeatureError::new),
            None => Ok(()),
        }
    }

    fn stop(&mut self) -> Result<(), FeatureError> {
        if let Some(overlay) = LINK.get() {
            overlay.stop();
        }
        Ok(())
    }
}

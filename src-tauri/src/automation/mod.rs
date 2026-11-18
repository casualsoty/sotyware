//! Autoclicker and macros (ADR-050): clicks automatically, records the
//! user's mouse and keyboard input into macros, edits and replays them,
//! keeps profiles of settings and runs everything from global shortcuts.
//!
//! - [`model`]: the data and its rules (validation, shortcuts and their
//!   conflicts, screen geometry, the file format); [`keys`]: the key table.
//! - [`clicker`], [`recorder`], [`player`]: the engines, over [`input`]'s
//!   `Backend` and [`timing`]'s clock, so tests never touch real input.
//! - `native`: the platform's `Backend` (Windows: `windows.rs`; X11:
//!   `x11.rs`); none elsewhere.
//! - This module: [`Automation`], the managed state: settings, profiles
//!   and the macro library in `automation.sqlite3`, the one session that
//!   may run (autoclicker, playback or recording), the global shortcuts,
//!   the live status the page watches, and the commands.
//!
//! Automation runs only while the built-in feature `autoclicker` runs (off
//! until the user turns it on): starting it opens the platform backend and
//! registers the shortcuts; stopping it, or quitting, stops any session,
//! releases every key and button and unregisters the shortcuts. Nothing
//! restarts by itself after a restart of sotyware.
//!
//! Design: docs/architecture/automation.md.

pub mod clicker;
pub mod input;
pub mod keys;
pub mod model;
mod native;
pub mod player;
pub mod recorder;
pub mod timing;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::Notify;

use crate::features::StoreError;
use crate::journal::now_ms;
use clicker::{ClickerProgress, ClickerRun, Outcome, RunState};
use input::{Backend, HotkeySink, Output, RawEvent, Support};
use keys::{Chord, Key};
use model::{
    Action, Binding, BindingOwner, Bundle, Coordinates, HotkeyAction, Macro, MacroSummary,
    Mismatch, Monitor, Point, Positioning, Profile, RecordedOn, RecordingOptions, Settings,
    TargetApp, WindowInfo, active_bindings, conflict_with, first_conflict, unique_name,
};
use player::{PlaybackProgress, PlaybackRun};
use recorder::{Fed, Recorder};
use timing::{Clock, Control, RealClock, Rng, StopReason};

/// How long stopping waits for a session's thread to end.
pub const STOP_WAIT: Duration = Duration::from_secs(3);
/// How many recorded actions the live list shows.
const RECENT: usize = 40;
/// The status is sent at most this often.
const STATUS_EVERY: Duration = Duration::from_millis(100);
/// While a session runs, the status is sent at least this often (the
/// elapsed time moves even when nothing else does).
const HEARTBEAT: Duration = Duration::from_secs(1);

// ------------------------------------------------------------- errors

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// Something sent is not valid; nothing changed.
    InvalidInput,
    /// No macro or profile with this id (removed meanwhile).
    NotFound,
    /// Another session runs, or a recording waits to be saved.
    Busy,
    /// The `autoclicker` feature is off.
    FeatureOff,
    /// This system offers no input automation.
    Unavailable,
    /// The database cannot be used.
    Storage,
    /// The run needs the user's confirmation (`confirmed: true`).
    Confirm,
    /// The system refused input.
    Input,
    /// A file could not be read or written.
    File,
    /// A shortcut is taken.
    Conflict,
}

/// The error every automation command returns: a kind for the page's
/// logic and a sentence to show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AutomationError {
    pub kind: ErrorKind,
    pub message: String,
}

impl AutomationError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

fn invalid(message: impl Into<String>) -> AutomationError {
    AutomationError::new(ErrorKind::InvalidInput, message)
}

fn storage(e: impl std::fmt::Display) -> AutomationError {
    AutomationError::new(
        ErrorKind::Storage,
        format!("the autoclicker database cannot be used: {e}"),
    )
}

fn not_found(what: &str) -> AutomationError {
    AutomationError::new(
        ErrorKind::NotFound,
        format!("this {what} does not exist any more"),
    )
}

type Result<T, E = AutomationError> = std::result::Result<T, E>;

// -------------------------------------------------------------- store

/// What the autoclicker stores; `storage::SqliteAutomationStore` in the
/// app, an in-memory store in tests.
pub trait AutomationStore: Send {
    fn settings(&mut self) -> Result<Option<String>, StoreError>;
    fn save_settings(&mut self, data: &str) -> Result<(), StoreError>;
    fn profiles(&mut self) -> Result<Vec<String>, StoreError>;
    fn save_profile(&mut self, id: &str, position: i64, data: &str) -> Result<(), StoreError>;
    fn delete_profile(&mut self, id: &str) -> Result<(), StoreError>;
    fn macro_summaries(&mut self) -> Result<Vec<String>, StoreError>;
    fn load_macro(&mut self, id: &str) -> Result<Option<String>, StoreError>;
    fn save_macro(
        &mut self,
        id: &str,
        summary: &str,
        data: &str,
        updated_ms: i64,
    ) -> Result<(), StoreError>;
    fn delete_macro(&mut self, id: &str) -> Result<(), StoreError>;
}

// -------------------------------------------------------------- status

/// What runs, as the page shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SessionView {
    #[serde(rename_all = "camelCase")]
    Clicker {
        progress: ClickerProgress,
        target_cps: f64,
        started_ms: i64,
        profile: String,
    },
    #[serde(rename_all = "camelCase")]
    Playback {
        progress: PlaybackProgress,
        macro_id: String,
        macro_name: String,
        started_ms: i64,
    },
    #[serde(rename_all = "camelCase")]
    Recording {
        state: RunState,
        elapsed_ms: u64,
        countdown_ms: Option<u64>,
        actions: usize,
        recent: Vec<Action>,
        skipped: u64,
        started_ms: i64,
    },
}

/// How the last session ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSession {
    /// `clicker`, `playback` or `recording`.
    pub kind: &'static str,
    /// `completed`, `stopped`, `failsafe` or `failed`.
    pub outcome: &'static str,
    pub message: String,
    pub at_ms: i64,
}

/// A stopped recording waiting to be saved or discarded.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftView {
    pub actions: usize,
    pub duration_ms: u64,
    pub skipped: u64,
    pub recent: Vec<Action>,
    pub window: Option<WindowInfo>,
}

/// One shortcut in use and whether the system accepted it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyView {
    pub owner: BindingOwner,
    pub label: String,
    pub chord: Chord,
    /// The keys' names on this keyboard (`Ctrl + Alt + P`).
    pub keys: String,
    pub registered: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileRef {
    pub id: String,
    pub name: String,
}

/// Everything the page shows that changes while sotyware runs.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationStatus {
    pub seq: u64,
    /// The `autoclicker` feature runs (the backend is open).
    pub running: bool,
    pub support: Support,
    pub session: Option<SessionView>,
    pub last: Option<LastSession>,
    pub draft: Option<DraftView>,
    pub hotkeys: Vec<HotkeyView>,
    pub active_profile: ProfileRef,
    /// The program that has the keyboard, while a session runs.
    pub active_window: Option<WindowInfo>,
    /// Something a shortcut could not do ("F6: the recording runs").
    pub notice: Option<String>,
    pub storage_error: Option<String>,
}

/// The configuration the page edits, with what it needs to show it: the
/// keys sotyware knows (so the page never keeps its own table) and the
/// suggested shortcuts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationConfig {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub active_profile: String,
    pub keys: Vec<KeyView>,
    pub suggestions: std::collections::BTreeMap<HotkeyAction, Chord>,
}

/// A key as the page offers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KeyView {
    pub code: &'static str,
    /// Its name on this keyboard where the system says, else English.
    pub label: String,
    /// `modifier`, `layout`, `function`, `media`, `quiet` or `other`.
    pub kind: &'static str,
}

/// Receives the status whenever it changes.
pub trait StatusSink: Send + Sync {
    /// `false` when the receiver is gone.
    fn send(&self, status: &AutomationStatus) -> bool;
}

impl StatusSink for Channel<AutomationStatus> {
    fn send(&self, status: &AutomationStatus) -> bool {
        Channel::send(self, status.clone()).is_ok()
    }
}

// ------------------------------------------------------------ sessions

enum Live {
    Clicker(ClickerProgress),
    Playback(PlaybackProgress),
    Recording(RecordingLive),
}

#[derive(Clone)]
struct RecordingLive {
    state: RunState,
    elapsed_ms: u64,
    countdown_ms: Option<u64>,
    actions: usize,
    recent: Vec<Action>,
    skipped: u64,
}

#[derive(Clone)]
enum SessionKind {
    Clicker { target_cps: f64, profile: String },
    Playback { macro_id: String, name: String },
    Recording,
}

struct Session {
    id: u64,
    kind: SessionKind,
    control: Arc<Control>,
    thread: Option<JoinHandle<()>>,
    live: Arc<Mutex<Live>>,
    started_ms: i64,
}

struct Draft {
    actions: Vec<Action>,
    recorded_on: RecordedOn,
    coordinates: Coordinates,
    skipped: u64,
}

/// A registered (or refused) shortcut.
struct Registration {
    id: u32,
    owner: BindingOwner,
    chord: Chord,
    result: std::result::Result<(), String>,
    /// The emergency stop with other modifiers, registered only while a
    /// session runs and never listed ([`Automation::sync_hotkeys`]).
    alias: bool,
}

enum Dispatch {
    Hotkey(u32),
    Quit,
}

struct Inner {
    settings: Settings,
    profiles: Vec<Profile>,
    active: String,
    macros: Vec<MacroSummary>,
    storage_error: Option<String>,
    backend: Option<Arc<dyn Backend>>,
    registrations: Vec<Registration>,
    next_hotkey: u32,
    session: Option<Session>,
    next_session: u64,
    draft: Option<Draft>,
    last: Option<LastSession>,
    notice: Option<String>,
    dispatcher: Option<(mpsc::Sender<Dispatch>, JoinHandle<()>)>,
}

/// Opens the platform's backend; tests pass a mock.
pub type BackendFactory =
    Box<dyn Fn(HotkeySink) -> std::result::Result<Arc<dyn Backend>, String> + Send + Sync>;

/// The autoclicker: managed state.
pub struct Automation {
    inner: Mutex<Inner>,
    store: Mutex<Option<Box<dyn AutomationStore>>>,
    factory: BackendFactory,
    support: Support,
    clock: Arc<dyn Clock>,
    changed: Notify,
    subscribers: Mutex<Vec<(u64, Arc<dyn StatusSink>)>>,
    notifier: Mutex<bool>,
    next_subscriber: AtomicU64,
    seq: AtomicU64,
    self_ref: OnceLock<Weak<Automation>>,
}

/// The link the `autoclicker` feature uses; set once in `run()`.
pub static LINK: OnceLock<Arc<Automation>> = OnceLock::new();

fn parse_or<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> Option<T> {
    match serde_json::from_str(text) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("sotyware: skipping a stored {what} this version cannot read: {e}");
            None
        }
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", crate::backup::engine::new_id())
}

impl Automation {
    /// The service of `run()`: the platform's backend, the real clock.
    pub fn load(store: std::result::Result<Box<dyn AutomationStore>, StoreError>) -> Arc<Self> {
        Self::with(
            store,
            native::support(),
            Box::new(native::open),
            RealClock::new(),
        )
    }

    pub fn with(
        store: std::result::Result<Box<dyn AutomationStore>, StoreError>,
        support: Support,
        factory: BackendFactory,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        let (mut store, mut storage_error) = match store {
            Ok(s) => (Some(s), None),
            Err(e) => {
                eprintln!("sotyware: the autoclicker database cannot be used: {e}");
                (None, Some(e.to_string()))
            }
        };
        let now = now_ms();
        let mut settings: Option<Settings> = None;
        // Settings stored by a sotyware that this one cannot read are left
        // as they are (defaults are used until the user saves).
        let mut settings_stored = false;
        let mut profiles: Vec<Profile> = Vec::new();
        let mut macros: Vec<MacroSummary> = Vec::new();
        if let Some(s) = store.as_mut() {
            let loaded = (|| -> std::result::Result<(), StoreError> {
                let text = s.settings()?;
                settings_stored = text.is_some();
                settings = text.and_then(|t| parse_or(&t, "autoclicker settings"));
                profiles = s
                    .profiles()?
                    .iter()
                    .filter_map(|t| parse_or::<Profile>(t, "profile"))
                    .filter(|p| p.validate().is_ok())
                    .collect();
                macros = s
                    .macro_summaries()?
                    .iter()
                    .filter_map(|t| parse_or(t, "macro"))
                    .collect();
                Ok(())
            })();
            if let Err(e) = loaded {
                eprintln!("sotyware: the autoclicker database cannot be read: {e}");
                storage_error = Some(e.to_string());
            }
        }
        // The first start: one profile with the suggested shortcuts.
        if profiles.is_empty() {
            let p = Profile::new(new_id("profile"), "Default".to_owned(), now);
            if let Some(s) = store.as_mut()
                && storage_error.is_none()
                && let Ok(text) = serde_json::to_string(&p)
                && let Err(e) = s.save_profile(&p.id, 0, &text)
            {
                storage_error = Some(e.to_string());
            }
            profiles.push(p);
        }
        let mut settings = settings
            .filter(|s| s.validate().is_ok())
            .unwrap_or_else(|| Settings::new(profiles[0].id.clone()));
        if !profiles.iter().any(|p| p.id == settings.default_profile) {
            settings.default_profile = profiles[0].id.clone();
        }
        if let Some(s) = store.as_mut()
            && storage_error.is_none()
            && !settings_stored
            && let Ok(text) = serde_json::to_string(&settings)
            && let Err(e) = s.save_settings(&text)
        {
            storage_error = Some(e.to_string());
        }
        let active = settings.default_profile.clone();
        let this = Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                profiles,
                active,
                macros,
                storage_error,
                backend: None,
                registrations: Vec::new(),
                next_hotkey: 1,
                session: None,
                next_session: 1,
                draft: None,
                last: None,
                notice: None,
                dispatcher: None,
            }),
            store: Mutex::new(store),
            factory,
            support,
            clock,
            changed: Notify::new(),
            subscribers: Mutex::new(Vec::new()),
            notifier: Mutex::new(false),
            next_subscriber: AtomicU64::new(1),
            seq: AtomicU64::new(1),
            self_ref: OnceLock::new(),
        });
        let _ = this.self_ref.set(Arc::downgrade(&this));
        this
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Something changed: the status is sent soon.
    fn touch(&self) {
        self.seq.fetch_add(1, Ordering::Relaxed);
        self.changed.notify_one();
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut dyn AutomationStore) -> std::result::Result<T, StoreError>,
    ) -> Result<T> {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        match store.as_mut() {
            Some(s) => f(s.as_mut()).map_err(storage),
            None => Err(storage(
                self.lock()
                    .storage_error
                    .clone()
                    .unwrap_or_else(|| "it could not be opened".to_owned()),
            )),
        }
    }

    // ------------------------------------------------- feature lifecycle

    /// Starts the feature: opens the backend and registers the shortcuts.
    pub fn start(&self) -> std::result::Result<(), String> {
        if !self.support.available {
            return Err(self
                .support
                .note
                .clone()
                .unwrap_or_else(|| "input automation is not available on this system".to_owned()));
        }
        if self.lock().backend.is_some() {
            return Ok(());
        }
        input::install_panic_release();
        let (tx, rx) = mpsc::channel::<Dispatch>();
        let sink_tx = tx.clone();
        let sink: HotkeySink = Arc::new(move |id| {
            let _ = sink_tx.send(Dispatch::Hotkey(id));
        });
        let backend = (self.factory)(sink)?;
        let weak = self.self_ref.get().cloned().unwrap_or_default();
        let dispatcher = std::thread::Builder::new()
            .name("sotyware-hotkeys".into())
            .spawn(move || {
                while let Ok(Dispatch::Hotkey(id)) = rx.recv() {
                    if let Some(this) = weak.upgrade() {
                        this.on_hotkey(id);
                    }
                }
            })
            .map_err(|e| {
                backend.shutdown();
                format!("cannot start the shortcut thread: {e}")
            })?;
        let mut inner = self.lock();
        inner.backend = Some(backend);
        inner.dispatcher = Some((tx, dispatcher));
        inner.notice = None;
        self.sync_hotkeys(&mut inner);
        drop(inner);
        self.touch();
        Ok(())
    }

    /// Stops the feature: the session stops (everything held is
    /// released), the shortcuts are unregistered, the backend closes. A
    /// recording that was stopped stays as a draft to save.
    pub fn stop(&self) {
        // The backend first: no shortcut can start a session any more.
        let (backend, dispatcher) = {
            let mut inner = self.lock();
            let backend = inner.backend.take();
            if let Some(b) = &backend {
                for r in inner.registrations.drain(..) {
                    if r.result.is_ok() {
                        b.unregister_hotkey(r.id);
                    }
                }
            }
            inner.registrations.clear();
            (backend, inner.dispatcher.take())
        };
        self.stop_session(StopReason::Shutdown, true);
        if let Some(b) = backend {
            b.shutdown();
        }
        if let Some((tx, handle)) = dispatcher {
            let _ = tx.send(Dispatch::Quit);
            let _ = handle.join();
        }
        let mut inner = self.lock();
        self.sync_hotkeys(&mut inner);
        drop(inner);
        self.touch();
    }

    /// Quitting: as [`stop`](Self::stop); safe to call when stopped.
    pub fn shutdown(&self) {
        self.stop();
    }

    // ------------------------------------------------------ shortcuts

    /// Registers what should be registered and unregisters the rest. A
    /// shortcut that conflicts with an earlier one is not registered and
    /// says why.
    ///
    /// The system matches a shortcut's modifiers exactly (`RegisterHotKey`,
    /// `XGrabKey`), so while a macro holds Shift, the user's F8 arrives as
    /// Shift+F8 and the emergency stop would not fire (found by the Linux
    /// desktop test). While a session runs, the emergency stop is also
    /// registered with every other combination of modifiers; those
    /// registrations are never listed, and a combination another shortcut or
    /// program already has is left to it.
    fn sync_hotkeys(&self, inner: &mut Inner) {
        let profile = inner
            .profiles
            .iter()
            .find(|p| p.id == inner.active)
            .cloned();
        let mut wanted: Vec<(BindingOwner, Chord, bool)> =
            active_bindings(&inner.settings, profile.as_ref(), &inner.macros)
                .into_iter()
                .map(|(owner, chord)| (owner, chord, false))
                .collect();
        if inner.session.is_some()
            && let Some(stop) = inner.settings.emergency_stop.active()
        {
            for chord in stop.with_other_modifiers() {
                wanted.push((BindingOwner::Settings, chord, true));
            }
        }
        let Some(backend) = inner.backend.clone() else {
            inner.registrations = wanted
                .into_iter()
                .map(|(owner, chord, alias)| Registration {
                    id: 0,
                    owner,
                    chord,
                    result: Err("Not registered: the autoclicker is off".to_owned()),
                    alias,
                })
                .collect();
            return;
        };
        let mut keep: Vec<Registration> = Vec::new();
        for r in inner.registrations.drain(..) {
            let still = wanted
                .iter()
                .any(|(o, c, a)| *o == r.owner && *c == r.chord && *a == r.alias);
            if still && r.result.is_ok() {
                keep.push(r);
            } else if r.result.is_ok() {
                backend.unregister_hotkey(r.id);
            }
        }
        let mut out: Vec<Registration> = Vec::new();
        for (owner, chord, alias) in wanted {
            if let Some(i) = keep
                .iter()
                .position(|r| r.owner == owner && r.chord == chord && r.alias == alias)
            {
                out.push(keep.remove(i));
                continue;
            }
            let result = if let Some(other) = out.iter().find(|r| r.chord == chord) {
                Err(format!(
                    "Not registered: \"{}\" has the same shortcut",
                    other.owner.label()
                ))
            } else if let Some(p) = chord.problem() {
                Err(format!("Not registered: {p}"))
            } else {
                // Ids stay within 1…0xBFFF (Windows' range for an
                // application's hotkeys), skipping any still in use.
                let mut id = inner.next_hotkey;
                while out.iter().chain(keep.iter()).any(|r| r.id == id) {
                    id = if id >= 0xBFFF { 1 } else { id + 1 };
                }
                inner.next_hotkey = if id >= 0xBFFF { 1 } else { id + 1 };
                match backend.register_hotkey(id, chord) {
                    Ok(()) => {
                        out.push(Registration {
                            id,
                            owner,
                            chord,
                            result: Ok(()),
                            alias,
                        });
                        continue;
                    }
                    Err(e) => Err(format!("Not registered: {e}")),
                }
            };
            out.push(Registration {
                id: 0,
                owner,
                chord,
                result,
                alias,
            });
        }
        inner.registrations = out;
    }

    fn hotkey_views(&self, inner: &Inner) -> Vec<HotkeyView> {
        let backend = inner.backend.clone();
        inner
            .registrations
            .iter()
            .filter(|r| !r.alias)
            .map(|r| HotkeyView {
                owner: r.owner.clone(),
                label: r.owner.label(),
                chord: r.chord,
                keys: r.chord.label_with(|k| {
                    backend
                        .as_ref()
                        .and_then(|b| b.key_name(k))
                        .unwrap_or_else(|| k.label().to_owned())
                }),
                registered: r.result.is_ok(),
                error: r.result.clone().err(),
            })
            .collect()
    }

    /// A registered shortcut was pressed (on the dispatcher thread).
    fn on_hotkey(&self, id: u32) {
        let (owner, keys) = {
            let inner = self.lock();
            let Some(r) = inner
                .registrations
                .iter()
                .find(|r| r.id == id && r.result.is_ok())
            else {
                return;
            };
            (
                r.owner.clone(),
                r.chord.label_with(|k| k.label().to_owned()),
            )
        };
        let result = match &owner {
            BindingOwner::Settings => {
                self.stop_session(StopReason::User(format!("Emergency stop ({keys})")), false);
                Ok(())
            }
            BindingOwner::Profile { action } => self.profile_hotkey(*action, &keys),
            BindingOwner::Macro { id, .. } => {
                let playing = matches!(
                    &self.lock().session,
                    Some(Session { kind: SessionKind::Playback { macro_id, .. }, .. }) if macro_id == id
                );
                if playing {
                    self.stop_session(StopReason::User(format!("Stopped with {keys}")), false);
                    Ok(())
                } else {
                    self.play(Some(id.clone()), false, true)
                }
            }
        };
        let notice = result.err().map(|e| format!("{keys}: {}", e.message));
        if let Some(n) = &notice {
            eprintln!("sotyware: shortcut {n}");
        }
        self.lock().notice = notice;
        self.touch();
    }

    fn profile_hotkey(&self, action: HotkeyAction, keys: &str) -> Result<()> {
        let running = self.lock().session.as_ref().map(|s| s.kind.clone());
        let stop = |s: &Self| {
            s.stop_session(StopReason::User(format!("Stopped with {keys}")), false);
            Ok(())
        };
        match action {
            HotkeyAction::ToggleClicker => match running {
                Some(SessionKind::Clicker { .. }) => stop(self),
                _ => self.start_clicker(false, true),
            },
            HotkeyAction::StartClicker => match running {
                Some(SessionKind::Clicker { .. }) => Ok(()),
                _ => self.start_clicker(false, true),
            },
            HotkeyAction::PauseResume => self.toggle_pause(),
            HotkeyAction::StartRecording => match running {
                Some(SessionKind::Recording) => Ok(()),
                _ => self.start_recording(false),
            },
            HotkeyAction::StopRecording => match running {
                Some(SessionKind::Recording) => stop(self),
                _ => Ok(()),
            },
            HotkeyAction::PlayMacro => match running {
                Some(SessionKind::Playback { .. }) => Ok(()),
                _ => self.play(None, false, true),
            },
            HotkeyAction::StopPlayback => match running {
                Some(SessionKind::Playback { .. }) => stop(self),
                _ => Ok(()),
            },
            HotkeyAction::StopAll => stop(self),
        }
    }

    // ---------------------------------------------------------- status

    pub fn status(&self) -> AutomationStatus {
        let inner = self.lock();
        let session = inner.session.as_ref().map(|s| {
            let live = s.live.lock().unwrap_or_else(|e| e.into_inner());
            match &*live {
                Live::Clicker(p) => {
                    let (target_cps, profile) = match &s.kind {
                        SessionKind::Clicker {
                            target_cps,
                            profile,
                        } => (*target_cps, profile.clone()),
                        _ => (0.0, String::new()),
                    };
                    SessionView::Clicker {
                        progress: p.clone(),
                        target_cps,
                        started_ms: s.started_ms,
                        profile,
                    }
                }
                Live::Playback(p) => {
                    let (macro_id, macro_name) = match &s.kind {
                        SessionKind::Playback { macro_id, name } => {
                            (macro_id.clone(), name.clone())
                        }
                        _ => (String::new(), String::new()),
                    };
                    SessionView::Playback {
                        progress: p.clone(),
                        macro_id,
                        macro_name,
                        started_ms: s.started_ms,
                    }
                }
                Live::Recording(r) => SessionView::Recording {
                    state: r.state,
                    elapsed_ms: r.elapsed_ms,
                    countdown_ms: r.countdown_ms,
                    actions: r.actions,
                    recent: r.recent.clone(),
                    skipped: r.skipped,
                    started_ms: s.started_ms,
                },
            }
        });
        let draft = inner.draft.as_ref().map(|d| DraftView {
            actions: d.actions.len(),
            duration_ms: d.actions.iter().map(|a| u64::from(a.delay_ms)).sum(),
            skipped: d.skipped,
            recent: d.actions[d.actions.len().saturating_sub(RECENT)..].to_vec(),
            window: d.recorded_on.window.clone(),
        });
        let active_profile = inner
            .profiles
            .iter()
            .find(|p| p.id == inner.active)
            .map_or_else(
                || ProfileRef {
                    id: inner.active.clone(),
                    name: String::new(),
                },
                |p| ProfileRef {
                    id: p.id.clone(),
                    name: p.name.clone(),
                },
            );
        let backend = inner.backend.clone();
        let running_session = inner.session.is_some();
        let status = AutomationStatus {
            seq: self.seq.load(Ordering::Relaxed),
            running: backend.is_some(),
            support: self.support.clone(),
            session,
            last: inner.last.clone(),
            draft,
            hotkeys: self.hotkey_views(&inner),
            active_profile,
            active_window: None,
            notice: inner.notice.clone(),
            storage_error: inner.storage_error.clone(),
        };
        drop(inner);
        AutomationStatus {
            active_window: if running_session {
                backend.and_then(|b| b.foreground())
            } else {
                None
            },
            ..status
        }
    }

    /// Sends the status on `sink` now and whenever it changes.
    pub fn subscribe(self: &Arc<Self>, sink: Arc<dyn StatusSink>) -> u64 {
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        sink.send(&self.status());
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, sink));
        let mut started = self.notifier.lock().unwrap_or_else(|e| e.into_inner());
        if !*started {
            *started = true;
            let weak = Arc::downgrade(self);
            tauri::async_runtime::spawn(async move {
                loop {
                    let active = match weak.upgrade() {
                        Some(this) => this.lock().session.is_some(),
                        None => return,
                    };
                    let notified = async {
                        let Some(this) = weak.upgrade() else { return };
                        this.changed.notified().await;
                    };
                    if active {
                        let _ = tokio::time::timeout(HEARTBEAT, notified).await;
                    } else {
                        notified.await;
                    }
                    tokio::time::sleep(STATUS_EVERY).await;
                    let Some(this) = weak.upgrade() else { return };
                    let status = this.status();
                    this.subscribers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .retain(|(_, s)| s.send(&status));
                }
            });
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(i, _)| *i != id);
    }

    pub fn config(&self) -> AutomationConfig {
        let inner = self.lock();
        let backend = inner.backend.clone();
        let keys = keys::KEYS
            .iter()
            .filter_map(|info| Key::from_code(info.code))
            .map(|k| KeyView {
                code: k.code(),
                label: backend
                    .as_ref()
                    .and_then(|b| b.key_name(k))
                    .unwrap_or_else(|| k.label().to_owned()),
                kind: match k.class() {
                    keys::KeyClass::Modifier(_) => "modifier",
                    keys::KeyClass::Layout => "layout",
                    keys::KeyClass::Function => "function",
                    keys::KeyClass::Media => "media",
                    keys::KeyClass::Quiet => "quiet",
                    keys::KeyClass::Other => "other",
                },
            })
            .collect();
        let suggestions = HotkeyAction::PROFILE
            .iter()
            .chain([&HotkeyAction::StopAll])
            .filter_map(|&a| {
                a.suggestion()
                    .and_then(|c| Chord::parse(c).ok())
                    .map(|c| (a, c))
            })
            .collect();
        AutomationConfig {
            settings: inner.settings.clone(),
            profiles: inner.profiles.clone(),
            active_profile: inner.active.clone(),
            keys,
            suggestions,
        }
    }

    // --------------------------------------------------------- sessions

    fn backend_for_session(&self, inner: &Inner) -> Result<Arc<dyn Backend>> {
        if !self.support.available {
            return Err(AutomationError::new(
                ErrorKind::Unavailable,
                self.support
                    .note
                    .clone()
                    .unwrap_or_else(|| "input automation is not available here".to_owned()),
            ));
        }
        let backend = inner.backend.clone().ok_or_else(|| {
            AutomationError::new(
                ErrorKind::FeatureOff,
                "Turn on the autoclicker first (Autoclicker page or Settings › Features)",
            )
        })?;
        if let Some(s) = &inner.session {
            return Err(AutomationError::new(
                ErrorKind::Busy,
                match s.kind {
                    SessionKind::Clicker { .. } => "The autoclicker is running; stop it first",
                    SessionKind::Playback { .. } => "A macro is playing; stop it first",
                    SessionKind::Recording => "A recording is running; stop it first",
                },
            ));
        }
        Ok(backend)
    }

    /// Whether an unbounded run from the dashboard needs confirming: it
    /// does when no emergency stop shortcut is registered.
    fn needs_confirmation(inner: &Inner) -> bool {
        !inner
            .registrations
            .iter()
            .any(|r| r.owner == BindingOwner::Settings && !r.alias && r.result.is_ok())
    }

    fn delay(inner: &Inner, configured: u32, from_dashboard: bool) -> Duration {
        let ms = if from_dashboard {
            configured.max(inner.settings.dashboard_countdown_ms)
        } else {
            configured
        };
        Duration::from_millis(u64::from(ms))
    }

    /// Starts the active profile's autoclicker.
    pub fn start_clicker(&self, from_dashboard: bool, confirmed: bool) -> Result<()> {
        let mut inner = self.lock();
        let backend = self.backend_for_session(&inner)?;
        let profile = inner
            .profiles
            .iter()
            .find(|p| p.id == inner.active)
            .cloned()
            .ok_or_else(|| not_found("profile"))?;
        let config = profile.clicker.clone();
        config.validate().map_err(invalid)?;
        if from_dashboard && !confirmed && config.unbounded() && Self::needs_confirmation(&inner) {
            return Err(AutomationError::new(
                ErrorKind::Confirm,
                "The autoclicker will click until you stop it, and no emergency stop shortcut is active. Stop it from this window or by quitting sotyware.",
            ));
        }
        let monitors = backend
            .monitors()
            .map_err(|e| AutomationError::new(ErrorKind::Input, e))?;
        let delay = Self::delay(&inner, config.start_delay_ms, from_dashboard);
        let failsafe = inner.settings.failsafe_corners;
        let target_cps = 1000.0 / config.mean_interval_ms();
        let live = Arc::new(Mutex::new(Live::Clicker(ClickerProgress {
            state: if delay.is_zero() {
                RunState::Running
            } else {
                RunState::Countdown
            },
            clicks: 0,
            elapsed_ms: 0,
            countdown_ms: Some(delay.as_millis() as u64),
            cps: 0.0,
            skipped: 0,
            remaining: config.max_clicks,
        })));
        let control = Control::new();
        let id = inner.next_session;
        inner.next_session += 1;
        let (clock, weak) = (
            Arc::clone(&self.clock),
            self.self_ref.get().cloned().unwrap_or_default(),
        );
        let (c2, l2) = (Arc::clone(&control), Arc::clone(&live));
        let thread = std::thread::Builder::new()
            .name("sotyware-clicker".into())
            .spawn(move || {
                let touch = weak.clone();
                let report = move |p: &ClickerProgress| {
                    *l2.lock().unwrap_or_else(|e| e.into_inner()) = Live::Clicker(p.clone());
                    if let Some(a) = touch.upgrade() {
                        a.touch();
                    }
                };
                let mut out = Output::new(backend);
                let outcome = ClickerRun {
                    config: &config,
                    start_delay: delay,
                    clock: clock.as_ref(),
                    control: &c2,
                    rng: Rng::from_time(),
                    failsafe,
                    monitors,
                    report: &report,
                }
                .run(&mut out);
                drop(out);
                if let Some(a) = weak.upgrade() {
                    a.session_ended(id, "clicker", outcome);
                }
            })
            .map_err(|e| AutomationError::new(ErrorKind::Input, format!("cannot start: {e}")))?;
        inner.session = Some(Session {
            id,
            kind: SessionKind::Clicker {
                target_cps,
                profile: profile.name.clone(),
            },
            control,
            thread: Some(thread),
            live,
            started_ms: now_ms(),
        });
        // The emergency stop with other modifiers held, while it runs.
        self.sync_hotkeys(&mut inner);
        inner.notice = None;
        drop(inner);
        self.touch();
        Ok(())
    }

    /// Plays macro `id` with its own settings, or (`None`) the active
    /// profile's macro with the profile's settings.
    pub fn play(&self, id: Option<String>, from_dashboard: bool, confirmed: bool) -> Result<()> {
        let (macro_id, override_settings) = {
            let inner = self.lock();
            self.backend_for_session(&inner)?;
            match id {
                Some(id) => (id, None),
                None => {
                    let p = inner
                        .profiles
                        .iter()
                        .find(|p| p.id == inner.active)
                        .ok_or_else(|| not_found("profile"))?;
                    let id = p.macro_id.clone().ok_or_else(|| {
                        invalid(format!("The profile \"{}\" has no macro to play; choose one on its Profiles card", p.name))
                    })?;
                    (id, p.playback.clone())
                }
            }
        };
        let m = self.load_macro(&macro_id)?;
        let settings = override_settings.unwrap_or_else(|| m.playback.clone());
        settings.validate(Some(m.actions.len())).map_err(invalid)?;
        if m.actions.is_empty() {
            return Err(invalid("This macro has no actions yet"));
        }
        let mut inner = self.lock();
        let backend = self.backend_for_session(&inner)?;
        if from_dashboard && !confirmed && settings.unbounded() && Self::needs_confirmation(&inner)
        {
            return Err(AutomationError::new(
                ErrorKind::Confirm,
                "The macro will repeat until you stop it, and no emergency stop shortcut is active. Stop it from this window or by quitting sotyware.",
            ));
        }
        let monitors = backend
            .monitors()
            .map_err(|e| AutomationError::new(ErrorKind::Input, e))?;
        let delay = Self::delay(&inner, settings.start_delay_ms, from_dashboard);
        let failsafe = inner.settings.failsafe_corners;
        let live = Arc::new(Mutex::new(Live::Playback(PlaybackProgress {
            state: RunState::Countdown,
            repetition: 1,
            repetitions: settings.repeat,
            action: 0,
            actions: m.actions.len(),
            label: String::new(),
            progress: None,
            elapsed_ms: 0,
            remaining_ms: None,
            countdown_ms: Some(delay.as_millis() as u64),
            skipped: 0,
            warning: None,
            at_pause_point: false,
        })));
        let control = Control::new();
        let id = inner.next_session;
        inner.next_session += 1;
        let name = m.name.clone();
        let (clock, weak) = (
            Arc::clone(&self.clock),
            self.self_ref.get().cloned().unwrap_or_default(),
        );
        let (c2, l2) = (Arc::clone(&control), Arc::clone(&live));
        let thread = std::thread::Builder::new()
            .name("sotyware-playback".into())
            .spawn(move || {
                let touch = weak.clone();
                let report = move |p: &PlaybackProgress| {
                    *l2.lock().unwrap_or_else(|e| e.into_inner()) = Live::Playback(p.clone());
                    if let Some(a) = touch.upgrade() {
                        a.touch();
                    }
                };
                let mut out = Output::new(backend);
                let outcome = PlaybackRun {
                    macro_: &m,
                    settings: &settings,
                    start_delay: delay,
                    clock: clock.as_ref(),
                    control: &c2,
                    rng: Rng::from_time(),
                    failsafe,
                    monitors,
                    report: &report,
                }
                .run(&mut out);
                drop(out);
                if let Some(a) = weak.upgrade() {
                    a.session_ended(id, "playback", outcome);
                }
            })
            .map_err(|e| AutomationError::new(ErrorKind::Input, format!("cannot start: {e}")))?;
        inner.session = Some(Session {
            id,
            kind: SessionKind::Playback { macro_id, name },
            control,
            thread: Some(thread),
            live,
            started_ms: now_ms(),
        });
        // The emergency stop with other modifiers held, while it runs.
        self.sync_hotkeys(&mut inner);
        inner.notice = None;
        drop(inner);
        self.touch();
        Ok(())
    }

    /// Starts recording with the settings' recording options.
    pub fn start_recording(&self, from_dashboard: bool) -> Result<()> {
        let mut inner = self.lock();
        let backend = self.backend_for_session(&inner)?;
        if inner.draft.is_some() {
            return Err(AutomationError::new(
                ErrorKind::Busy,
                "Save or discard the last recording first",
            ));
        }
        let options = inner.settings.recording.clone();
        options.validate().map_err(invalid)?;
        let mut shortcuts: Vec<Chord> = inner
            .registrations
            .iter()
            .filter(|r| r.result.is_ok())
            .map(|r| r.chord)
            .collect();
        // The emergency stop pressed with other modifiers held: it stops
        // the recording too (see `sync_hotkeys`) and is never recorded.
        if let Some(stop) = inner.settings.emergency_stop.active() {
            shortcuts.extend(stop.with_other_modifiers());
        }
        let delay = if from_dashboard {
            Duration::from_millis(u64::from(inner.settings.dashboard_countdown_ms))
        } else {
            Duration::ZERO
        };
        let live = Arc::new(Mutex::new(Live::Recording(RecordingLive {
            // Running only once the capture is on (`record`).
            state: RunState::Countdown,
            elapsed_ms: 0,
            countdown_ms: Some(delay.as_millis() as u64),
            actions: 0,
            recent: Vec::new(),
            skipped: 0,
        })));
        let control = Control::new();
        let id = inner.next_session;
        inner.next_session += 1;
        let (clock, weak) = (
            Arc::clone(&self.clock),
            self.self_ref.get().cloned().unwrap_or_default(),
        );
        let (c2, l2) = (Arc::clone(&control), Arc::clone(&live));
        let thread = std::thread::Builder::new()
            .name("sotyware-recorder".into())
            .spawn(move || {
                let result = record(RecordRun {
                    backend: backend.as_ref(),
                    options,
                    shortcuts,
                    delay,
                    clock: clock.as_ref(),
                    control: &c2,
                    live: &l2,
                    touch: &|| {
                        if let Some(a) = weak.upgrade() {
                            a.touch();
                        }
                    },
                });
                if let Some(a) = weak.upgrade() {
                    let outcome = match result {
                        Ok(mut draft) => {
                            let n = draft.actions.len();
                            let message = format!("Recorded {n} actions");
                            if n > 0 {
                                draft.actions.shrink_to_fit();
                                a.lock().draft = Some(draft);
                            }
                            match c2.stopped() {
                                Some(StopReason::Rule(m)) => Outcome::Completed(m),
                                _ => Outcome::Completed(message),
                            }
                        }
                        Err(e) => Outcome::Stopped(StopReason::Failed(e)),
                    };
                    a.session_ended(id, "recording", outcome);
                }
            })
            .map_err(|e| AutomationError::new(ErrorKind::Input, format!("cannot start: {e}")))?;
        inner.session = Some(Session {
            id,
            kind: SessionKind::Recording,
            control,
            thread: Some(thread),
            live,
            started_ms: now_ms(),
        });
        // The emergency stop with other modifiers held, while it runs.
        self.sync_hotkeys(&mut inner);
        inner.notice = None;
        drop(inner);
        self.touch();
        Ok(())
    }

    fn session_ended(&self, id: u64, kind: &'static str, outcome: Outcome) {
        let mut inner = self.lock();
        if inner.session.as_ref().is_some_and(|s| s.id == id) {
            inner.session = None;
            self.sync_hotkeys(&mut inner);
        }
        let (outcome_name, message) = match outcome {
            Outcome::Completed(m) => ("completed", m),
            Outcome::Stopped(StopReason::User(m)) => ("stopped", m),
            Outcome::Stopped(StopReason::Shutdown) => (
                "stopped",
                "Stopped: the autoclicker was turned off".to_owned(),
            ),
            Outcome::Stopped(StopReason::Failsafe) => (
                "failsafe",
                "Stopped: the pointer was pushed into a corner of the screen".to_owned(),
            ),
            Outcome::Stopped(StopReason::Rule(m)) => ("stopped", m),
            Outcome::Stopped(StopReason::Failed(m)) => ("failed", m),
        };
        if outcome_name == "failed" {
            eprintln!("sotyware: {kind} failed: {message}");
        }
        inner.last = Some(LastSession {
            kind,
            outcome: outcome_name,
            message,
            at_ms: now_ms(),
        });
        drop(inner);
        self.touch();
    }

    /// Stops the session, if any. `wait`: until its thread has released
    /// everything (bounded). A recording stopped with [`DASHBOARD_STOP`]
    /// drops its last movements (they lead to the Stop button).
    pub fn stop_session(&self, reason: StopReason, wait: bool) {
        let handle = {
            let mut inner = self.lock();
            let Some(s) = inner.session.as_mut() else {
                return;
            };
            s.control.stop(reason);
            if wait { s.thread.take() } else { None }
        };
        if let Some(h) = handle {
            let until = Instant::now() + STOP_WAIT;
            while !h.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            } else {
                eprintln!("sotyware: an automation session did not stop in time");
            }
        }
    }

    fn toggle_pause(&self) -> Result<()> {
        let inner = self.lock();
        let s = inner
            .session
            .as_ref()
            .ok_or_else(|| invalid("Nothing is running"))?;
        if s.control.paused() {
            s.control.resume();
        } else {
            s.control.pause();
        }
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        let inner = self.lock();
        let s = inner
            .session
            .as_ref()
            .ok_or_else(|| invalid("Nothing is running"))?;
        if paused {
            s.control.pause();
        } else {
            s.control.resume();
        }
        drop(inner);
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------- recordings

    pub fn save_recording(
        &self,
        name: &str,
        description: &str,
        folder: &str,
    ) -> Result<MacroSummary> {
        let draft = self
            .lock()
            .draft
            .take()
            .ok_or_else(|| invalid("There is no recording to save"))?;
        let names: Vec<String> = self.lock().macros.iter().map(|m| m.name.clone()).collect();
        let now = now_ms();
        let mut m = Macro::new(
            new_id("macro"),
            unique_name(name.trim(), names.iter().map(String::as_str)),
            now,
        );
        m.description = description.trim().to_owned();
        m.folder = folder.trim().to_owned();
        m.playback.positioning = match draft.coordinates {
            Coordinates::Absolute => Positioning::Absolute,
            Coordinates::Relative => Positioning::Cursor,
            Coordinates::Window => Positioning::Window,
        };
        if draft.coordinates == Coordinates::Window
            && let Some(w) = &draft.recorded_on.window
        {
            m.target = Some(TargetApp {
                process: w.process.clone(),
                title: None,
                on_mismatch: Mismatch::Warn,
            });
        }
        m.recorded_on = Some(draft.recorded_on.clone());
        m.actions = draft.actions.clone();
        if let Err(e) = m.validate() {
            self.lock().draft = Some(draft);
            return Err(invalid(e));
        }
        match self.store_macro(&m) {
            Ok(s) => {
                self.touch();
                Ok(s)
            }
            Err(e) => {
                // Not lost: still there to save again.
                self.lock().draft = Some(draft);
                Err(e)
            }
        }
    }

    pub fn discard_recording(&self) {
        self.lock().draft = None;
        self.touch();
    }

    // ---------------------------------------------------------- library

    pub fn macros(&self) -> Vec<MacroSummary> {
        self.lock().macros.clone()
    }

    pub fn load_macro(&self, id: &str) -> Result<Macro> {
        let text = self
            .with_store(|s| s.load_macro(id))?
            .ok_or_else(|| not_found("macro"))?;
        serde_json::from_str(&text).map_err(|e| {
            storage(format!(
                "the macro cannot be read by this version of sotyware: {e}"
            ))
        })
    }

    /// Saves `m` (new or changed) and updates the library and shortcuts.
    fn store_macro(&self, m: &Macro) -> Result<MacroSummary> {
        let summary = m.summary();
        let data = serde_json::to_string(m).map_err(|e| invalid(e.to_string()))?;
        let summary_text = serde_json::to_string(&summary).map_err(|e| invalid(e.to_string()))?;
        self.with_store(|s| s.save_macro(&m.id, &summary_text, &data, m.modified_ms))?;
        let mut inner = self.lock();
        match inner.macros.iter_mut().find(|x| x.id == m.id) {
            Some(x) => *x = summary.clone(),
            None => inner.macros.push(summary.clone()),
        }
        self.sync_hotkeys(&mut inner);
        Ok(summary)
    }

    fn check_macro_shortcut(&self, m: &Macro) -> Result<()> {
        let Some(chord) = m.shortcut.active() else {
            return Ok(());
        };
        let inner = self.lock();
        let profile = inner.profiles.iter().find(|p| p.id == inner.active);
        let others: Vec<MacroSummary> = inner
            .macros
            .iter()
            .filter(|x| x.id != m.id)
            .cloned()
            .collect();
        let all = active_bindings(&inner.settings, profile, &others);
        let owner = BindingOwner::Macro {
            id: m.id.clone(),
            name: m.name.clone(),
        };
        match conflict_with(&all, &owner, chord) {
            Some(c) => Err(AutomationError::new(ErrorKind::Conflict, c)),
            None => Ok(()),
        }
    }

    pub fn create_macro(&self, name: &str, description: &str, folder: &str) -> Result<Macro> {
        let names: Vec<String> = self.lock().macros.iter().map(|m| m.name.clone()).collect();
        let mut m = Macro::new(
            new_id("macro"),
            unique_name(name.trim(), names.iter().map(String::as_str)),
            now_ms(),
        );
        m.description = description.trim().to_owned();
        m.folder = folder.trim().to_owned();
        m.validate().map_err(invalid)?;
        self.store_macro(&m)?;
        self.touch();
        Ok(m)
    }

    /// Saves an edited macro; returns its summary and warnings (keys that
    /// are also shortcuts).
    pub fn save_macro(&self, mut m: Macro) -> Result<SavedMacro> {
        let old = self.load_macro(&m.id)?;
        m.created_ms = old.created_ms;
        m.modified_ms = now_ms().max(old.modified_ms);
        m.format_version = model::MACRO_FORMAT_VERSION;
        m.app_version = env!("CARGO_PKG_VERSION").to_owned();
        m.name = m.name.trim().to_owned();
        m.validate().map_err(invalid)?;
        if self
            .lock()
            .macros
            .iter()
            .any(|x| x.id != m.id && x.name.eq_ignore_ascii_case(&m.name))
        {
            return Err(invalid(format!("Another macro is called \"{}\"", m.name)));
        }
        self.check_macro_shortcut(&m)?;
        let warnings = self.shortcut_warnings(&m);
        let summary = self.store_macro(&m)?;
        self.touch();
        Ok(SavedMacro { summary, warnings })
    }

    fn shortcut_warnings(&self, m: &Macro) -> Vec<String> {
        let inner = self.lock();
        let mut out: Vec<String> = Vec::new();
        for r in inner
            .registrations
            .iter()
            .filter(|r| r.result.is_ok() && !r.alias)
        {
            let presses = m.actions.iter().any(|a| match &a.kind {
                model::ActionKind::Key { key, .. } => *key == r.chord.key,
                model::ActionKind::Keys { chord } => *chord == r.chord,
                _ => false,
            });
            if presses {
                out.push(format!(
                    "This macro presses {}, the shortcut of \"{}\": playing it may trigger that shortcut",
                    r.chord.label_with(|k| k.label().to_owned()),
                    r.owner.label()
                ));
            }
        }
        out
    }

    pub fn update_macro_info(&self, id: &str, changes: MacroChanges) -> Result<MacroSummary> {
        let mut m = self.load_macro(id)?;
        if let Some(name) = changes.name {
            let name = name.trim().to_owned();
            if self
                .lock()
                .macros
                .iter()
                .any(|x| x.id != m.id && x.name.eq_ignore_ascii_case(&name))
            {
                return Err(invalid(format!("Another macro is called \"{name}\"")));
            }
            m.name = name;
        }
        if let Some(d) = changes.description {
            m.description = d.trim().to_owned();
        }
        if let Some(f) = changes.folder {
            m.folder = f.trim().to_owned();
        }
        if let Some(f) = changes.favorite {
            m.favorite = f;
        }
        if let Some(s) = changes.shortcut {
            m.shortcut = s;
        }
        m.modified_ms = now_ms().max(m.modified_ms);
        m.validate().map_err(invalid)?;
        self.check_macro_shortcut(&m)?;
        let s = self.store_macro(&m)?;
        self.touch();
        Ok(s)
    }

    pub fn duplicate_macro(&self, id: &str) -> Result<MacroSummary> {
        let mut m = self.load_macro(id)?;
        let names: Vec<String> = self.lock().macros.iter().map(|m| m.name.clone()).collect();
        m.id = new_id("macro");
        m.name = unique_name(&m.name, names.iter().map(String::as_str));
        m.shortcut = Binding {
            chord: m.shortcut.chord,
            enabled: false,
        };
        m.favorite = false;
        let now = now_ms();
        m.created_ms = now;
        m.modified_ms = now;
        m.validate().map_err(invalid)?;
        let s = self.store_macro(&m)?;
        self.touch();
        Ok(s)
    }

    pub fn delete_macro(&self, id: &str) -> Result<()> {
        if matches!(
            &self.lock().session,
            Some(Session { kind: SessionKind::Playback { macro_id, .. }, .. }) if macro_id == id
        ) {
            return Err(AutomationError::new(
                ErrorKind::Busy,
                "This macro is playing; stop it first",
            ));
        }
        self.with_store(|s| s.delete_macro(id))?;
        // Profiles that played it play nothing now.
        let changed: Vec<Profile> = {
            let mut inner = self.lock();
            inner.macros.retain(|m| m.id != id);
            let mut changed = Vec::new();
            for p in inner.profiles.iter_mut() {
                if p.macro_id.as_deref() == Some(id) {
                    p.macro_id = None;
                    changed.push(p.clone());
                }
            }
            self.sync_hotkeys(&mut inner);
            changed
        };
        for p in &changed {
            self.persist_profile(p)?;
        }
        self.touch();
        Ok(())
    }

    // --------------------------------------------------------- profiles

    fn persist_profile(&self, p: &Profile) -> Result<()> {
        let position = self
            .lock()
            .profiles
            .iter()
            .position(|x| x.id == p.id)
            .unwrap_or(0) as i64;
        let text = serde_json::to_string(p).map_err(|e| invalid(e.to_string()))?;
        self.with_store(|s| s.save_profile(&p.id, position, &text))
    }

    fn persist_settings(&self, settings: &Settings) -> Result<()> {
        let text = serde_json::to_string(settings).map_err(|e| invalid(e.to_string()))?;
        self.with_store(|s| s.save_settings(&text))
    }

    pub fn create_profile(&self, name: &str, from: Option<&str>) -> Result<Profile> {
        let mut inner = self.lock();
        if inner.profiles.len() >= model::MAX_PROFILES {
            return Err(invalid(format!("At most {} profiles", model::MAX_PROFILES)));
        }
        let now = now_ms();
        let name = unique_name(name.trim(), inner.profiles.iter().map(|p| p.name.as_str()));
        let mut p = Profile::new(new_id("profile"), name, now);
        if let Some(src) = from {
            let s = inner
                .profiles
                .iter()
                .find(|p| p.id == src)
                .ok_or_else(|| not_found("profile"))?;
            p.clicker = s.clicker.clone();
            p.hotkeys = s.hotkeys.clone();
            p.macro_id = s.macro_id.clone();
            p.playback = s.playback.clone();
        }
        p.validate().map_err(invalid)?;
        inner.profiles.push(p.clone());
        drop(inner);
        if let Err(e) = self.persist_profile(&p) {
            self.lock().profiles.retain(|x| x.id != p.id);
            return Err(e);
        }
        self.touch();
        Ok(p)
    }

    /// Saves an edited profile (its clicker, shortcuts, macro and
    /// playback settings); the active profile's shortcuts are registered
    /// again at once.
    pub fn save_profile(&self, mut p: Profile) -> Result<Profile> {
        let mut inner = self.lock();
        let Some(i) = inner.profiles.iter().position(|x| x.id == p.id) else {
            return Err(not_found("profile"));
        };
        p.name = p.name.trim().to_owned();
        p.created_ms = inner.profiles[i].created_ms;
        p.modified_ms = now_ms().max(inner.profiles[i].modified_ms);
        p.validate().map_err(invalid)?;
        if inner
            .profiles
            .iter()
            .any(|x| x.id != p.id && x.name.eq_ignore_ascii_case(&p.name))
        {
            return Err(invalid(format!("Another profile is called \"{}\"", p.name)));
        }
        if let Some(m) = &p.macro_id
            && !inner.macros.iter().any(|x| &x.id == m)
        {
            return Err(not_found("macro"));
        }
        // Its own shortcuts must differ from each other, and (active or
        // not) from the emergency stop and the macros'.
        let mut mine: Vec<(BindingOwner, Chord)> = Vec::new();
        for (&action, b) in &p.hotkeys {
            if let Some(c) = b.active() {
                mine.push((BindingOwner::Profile { action }, c));
            }
        }
        if let Some((a, b, c)) = first_conflict(&mine) {
            return Err(AutomationError::new(
                ErrorKind::Conflict,
                format!(
                    "{} is the shortcut of both \"{}\" and \"{}\"",
                    c.label_with(|k| k.label().to_owned()),
                    a.label(),
                    b.label()
                ),
            ));
        }
        let shared = active_bindings(&inner.settings, None, &inner.macros);
        for (owner, chord) in &mine {
            if let Some(c) = conflict_with(&shared, owner, *chord) {
                return Err(AutomationError::new(ErrorKind::Conflict, c));
            }
        }
        let old = std::mem::replace(&mut inner.profiles[i], p.clone());
        if inner.active == p.id {
            self.sync_hotkeys(&mut inner);
        }
        drop(inner);
        if let Err(e) = self.persist_profile(&p) {
            let mut inner = self.lock();
            if let Some(x) = inner.profiles.iter_mut().find(|x| x.id == old.id) {
                *x = old;
            }
            self.sync_hotkeys(&mut inner);
            return Err(e);
        }
        self.touch();
        Ok(p)
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        let mut inner = self.lock();
        if !inner.profiles.iter().any(|p| p.id == id) {
            return Err(not_found("profile"));
        }
        if inner.profiles.len() == 1 {
            return Err(invalid("The last profile cannot be deleted"));
        }
        drop(inner);
        self.with_store(|s| s.delete_profile(id))?;
        inner = self.lock();
        inner.profiles.retain(|p| p.id != id);
        let mut settings_changed = false;
        if inner.settings.default_profile == id {
            inner.settings.default_profile = inner.profiles[0].id.clone();
            settings_changed = true;
        }
        if inner.active == id {
            inner.active = inner.settings.default_profile.clone();
            self.sync_hotkeys(&mut inner);
        }
        let settings = inner.settings.clone();
        drop(inner);
        if settings_changed {
            self.persist_settings(&settings)?;
        }
        self.touch();
        Ok(())
    }

    /// Switches profiles: its shortcuts replace the old ones at once; a
    /// running session goes on with the settings it started with.
    pub fn activate_profile(&self, id: &str) -> Result<()> {
        let mut inner = self.lock();
        if !inner.profiles.iter().any(|p| p.id == id) {
            return Err(not_found("profile"));
        }
        inner.active = id.to_owned();
        self.sync_hotkeys(&mut inner);
        drop(inner);
        self.touch();
        Ok(())
    }

    pub fn set_default_profile(&self, id: &str) -> Result<()> {
        let settings = {
            let mut inner = self.lock();
            if !inner.profiles.iter().any(|p| p.id == id) {
                return Err(not_found("profile"));
            }
            inner.settings.default_profile = id.to_owned();
            inner.settings.clone()
        };
        self.persist_settings(&settings)?;
        self.touch();
        Ok(())
    }

    pub fn save_settings(&self, settings: Settings) -> Result<Settings> {
        settings.validate().map_err(invalid)?;
        let mut inner = self.lock();
        if !inner
            .profiles
            .iter()
            .any(|p| p.id == settings.default_profile)
        {
            return Err(not_found("profile"));
        }
        if let Some(c) = settings.emergency_stop.active() {
            let profile = inner.profiles.iter().find(|p| p.id == inner.active);
            let others = active_bindings(
                &Settings {
                    emergency_stop: Binding::NONE,
                    ..settings.clone()
                },
                profile,
                &inner.macros,
            );
            if let Some(msg) = conflict_with(&others, &BindingOwner::Settings, c) {
                return Err(AutomationError::new(ErrorKind::Conflict, msg));
            }
        }
        let old = std::mem::replace(&mut inner.settings, settings.clone());
        self.sync_hotkeys(&mut inner);
        drop(inner);
        if let Err(e) = self.persist_settings(&settings) {
            let mut inner = self.lock();
            inner.settings = old;
            self.sync_hotkeys(&mut inner);
            return Err(e);
        }
        self.touch();
        Ok(settings)
    }

    // ---------------------------------------------------- import/export

    /// The export file's text for these macros and profiles.
    pub fn export_text(&self, macros: &[String], profiles: &[String]) -> Result<String> {
        if macros.is_empty() && profiles.is_empty() {
            return Err(invalid("Choose at least one macro or profile to export"));
        }
        let mut bundle = Bundle {
            format: model::BUNDLE_FORMAT.to_owned(),
            format_version: model::BUNDLE_FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            exported_at: crate::backup::engine::now_rfc3339(),
            macros: Vec::new(),
            profiles: Vec::new(),
        };
        for id in macros {
            bundle.macros.push(self.load_macro(id)?);
        }
        {
            let inner = self.lock();
            for id in profiles {
                bundle.profiles.push(
                    inner
                        .profiles
                        .iter()
                        .find(|p| &p.id == id)
                        .cloned()
                        .ok_or_else(|| not_found("profile"))?,
                );
            }
        }
        serde_json::to_string_pretty(&bundle).map_err(|e| invalid(e.to_string()))
    }

    /// Imports a file's macros and profiles, each under a new id and a free
    /// name; an item that is not valid is reported and left out. Imported
    /// macro shortcuts are switched off (they may clash).
    pub fn import_text(&self, text: &str) -> Result<ImportReport> {
        let bundle = Bundle::parse(text).map_err(invalid)?;
        let mut report = ImportReport {
            macros: Vec::new(),
            profiles: Vec::new(),
            problems: Vec::new(),
        };
        let mut new_ids: Vec<(String, String)> = Vec::new();
        let now = now_ms();
        for mut m in bundle.macros {
            let old_id = m.id.clone();
            let shown = m.name.clone();
            m.id = new_id("macro");
            let names: Vec<String> = self.lock().macros.iter().map(|x| x.name.clone()).collect();
            m.name = unique_name(m.name.trim(), names.iter().map(String::as_str));
            m.shortcut.enabled = false;
            m.app_version = env!("CARGO_PKG_VERSION").to_owned();
            m.modified_ms = now;
            if let Err(e) = m.validate() {
                report.problems.push(format!("Macro \"{shown}\": {e}"));
                continue;
            }
            match self.store_macro(&m) {
                Ok(s) => {
                    new_ids.push((old_id, m.id.clone()));
                    report.macros.push(s);
                }
                Err(e) => report
                    .problems
                    .push(format!("Macro \"{shown}\": {}", e.message)),
            }
        }
        for mut p in bundle.profiles {
            let shown = p.name.clone();
            if self.lock().profiles.len() >= model::MAX_PROFILES {
                report.problems.push(format!(
                    "Profile \"{shown}\": at most {} profiles",
                    model::MAX_PROFILES
                ));
                continue;
            }
            p.id = new_id("profile");
            {
                let inner = self.lock();
                p.name = unique_name(
                    p.name.trim(),
                    inner.profiles.iter().map(|x| x.name.as_str()),
                );
                p.macro_id = p.macro_id.and_then(|old| {
                    new_ids
                        .iter()
                        .find(|(o, _)| *o == old)
                        .map(|(_, n)| n.clone())
                        .or_else(|| inner.macros.iter().any(|x| x.id == old).then_some(old))
                });
            }
            p.created_ms = now;
            p.modified_ms = now;
            if let Err(e) = p.validate() {
                report.problems.push(format!("Profile \"{shown}\": {e}"));
                continue;
            }
            self.lock().profiles.push(p.clone());
            if let Err(e) = self.persist_profile(&p) {
                self.lock().profiles.retain(|x| x.id != p.id);
                report
                    .problems
                    .push(format!("Profile \"{shown}\": {}", e.message));
                continue;
            }
            report.profiles.push(ProfileRef {
                id: p.id.clone(),
                name: p.name.clone(),
            });
        }
        self.touch();
        Ok(report)
    }

    // ---------------------------------------------------------- helpers

    fn backend(&self) -> Result<Arc<dyn Backend>> {
        if !self.support.available {
            return Err(AutomationError::new(
                ErrorKind::Unavailable,
                self.support.note.clone().unwrap_or_default(),
            ));
        }
        self.lock().backend.clone().ok_or_else(|| {
            AutomationError::new(ErrorKind::FeatureOff, "Turn on the autoclicker first")
        })
    }

    pub fn screen(&self) -> Result<ScreenLayout> {
        let monitors = self
            .backend()?
            .monitors()
            .map_err(|e| AutomationError::new(ErrorKind::Input, e))?;
        Ok(ScreenLayout {
            bounds: model::virtual_bounds(&monitors),
            monitors,
        })
    }

    pub fn cursor_now(&self) -> Result<CapturedPoint> {
        let b = self.backend()?;
        let point = b
            .cursor()
            .map_err(|e| AutomationError::new(ErrorKind::Input, e))?;
        let monitors = b.monitors().unwrap_or_default();
        Ok(CapturedPoint {
            point,
            monitor: model::monitor_at(&monitors, point),
            on_sotyware: b.own_window_at(point),
        })
    }

    pub fn window_now(&self) -> Result<WindowInfo> {
        let b = self.backend()?;
        if b.own_window_active() {
            return Err(invalid(
                "sotyware's own window was active; switch to the program during the countdown",
            ));
        }
        b.foreground()
            .ok_or_else(|| invalid("No window was active"))
    }
}

/// Everything a recording session needs.
struct RecordRun<'a> {
    backend: &'a dyn Backend,
    options: RecordingOptions,
    shortcuts: Vec<Chord>,
    delay: Duration,
    clock: &'a dyn Clock,
    control: &'a Control,
    live: &'a Mutex<Live>,
    touch: &'a dyn Fn(),
}

/// The recording session's thread: the countdown, then the user's input
/// until a stop, with the live list updated at most 20 times a second.
fn record(run: RecordRun<'_>) -> std::result::Result<Draft, String> {
    let set_live = |f: &dyn Fn(&mut RecordingLive)| {
        if let Live::Recording(r) = &mut *run.live.lock().unwrap_or_else(|e| e.into_inner()) {
            f(r);
        }
        (run.touch)();
    };
    // Countdown (pauses stretch it).
    let begin = run.clock.now();
    let mut shift = Duration::ZERO;
    loop {
        match run.clock.wait_until(begin + run.delay + shift, run.control) {
            timing::Wake::Reached => break,
            timing::Wake::Stop => {
                return Ok(Draft {
                    actions: Vec::new(),
                    recorded_on: RecordedOn {
                        cursor: Point::new(0, 0),
                        window: None,
                        monitors: Vec::new(),
                    },
                    coordinates: run.options.coordinates,
                    skipped: 0,
                });
            }
            timing::Wake::Pause => match run.clock.wait_resumed(run.control) {
                Some(d) => shift += d,
                None => continue,
            },
        }
    }
    let monitors = run.backend.monitors().unwrap_or_default();
    let cursor = run.backend.cursor().unwrap_or(Point::new(0, 0));
    let (tx, rx) = mpsc::channel::<RawEvent>();
    run.backend.start_capture(tx)?;
    let started = Instant::now();
    let mut recorder = Recorder::new(run.options.clone(), started, run.shortcuts.clone());
    let mut window: Option<WindowInfo> = None;
    let mut paused = false;
    let mut last_live = Instant::now() - Duration::from_secs(1);
    let mut full = false;
    set_live(&|r| {
        r.state = RunState::Running;
        r.countdown_ms = None;
    });
    loop {
        if run.control.stopped().is_some() {
            break;
        }
        let want_pause = run.control.paused();
        if want_pause != paused {
            paused = want_pause;
            if paused {
                recorder.pause(Instant::now());
            } else {
                recorder.resume(Instant::now());
            }
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(ev) => {
                let own = match ev.kind {
                    input::RawKind::Move(p)
                    | input::RawKind::Button(_, _, p)
                    | input::RawKind::Wheel { at: p, .. } => run.backend.own_window_at(p),
                    input::RawKind::Key(..) | input::RawKind::OtherKey => {
                        run.backend.own_window_active()
                    }
                };
                match recorder.feed(ev, own) {
                    Fed::Added { first_input: true } => {
                        window = run.backend.foreground();
                    }
                    Fed::Full => {
                        full = true;
                        run.control.stop(StopReason::Rule(format!(
                            "Recording stopped: a macro holds at most {} actions",
                            model::MAX_ACTIONS
                        )));
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                run.control
                    .stop(StopReason::Failed("the input capture ended".to_owned()));
            }
        }
        if last_live.elapsed() >= Duration::from_millis(50) {
            last_live = Instant::now();
            let elapsed = started.elapsed();
            let (n, recent, skipped) = (recorder.len(), recorder.recent(RECENT), recorder.skipped);
            set_live(&|r| {
                r.state = if paused {
                    RunState::Paused
                } else {
                    RunState::Running
                };
                r.elapsed_ms = elapsed.as_millis() as u64;
                r.actions = n;
                r.recent = recent.clone();
                r.skipped = skipped;
            });
        }
    }
    run.backend.stop_capture();
    // Events still queued arrived before the stop.
    while let Ok(ev) = rx.try_recv() {
        if full {
            break;
        }
        let own = match ev.kind {
            input::RawKind::Move(p)
            | input::RawKind::Button(_, _, p)
            | input::RawKind::Wheel { at: p, .. } => run.backend.own_window_at(p),
            _ => run.backend.own_window_active(),
        };
        let _ = recorder.feed(ev, own);
    }
    let trim =
        matches!(run.control.stopped(), Some(StopReason::User(ref r)) if r == DASHBOARD_STOP);
    let skipped = recorder.skipped;
    let actions = recorder.finish(Instant::now(), trim);
    Ok(Draft {
        actions,
        recorded_on: RecordedOn {
            cursor,
            window,
            monitors,
        },
        coordinates: run.options.coordinates,
        skipped,
    })
}

/// The stop reason of the dashboard's Stop button (a recording stopped
/// with it drops its last pointer movements).
const DASHBOARD_STOP: &str = "Stopped from sotyware";

// ----------------------------------------------------------- results

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMacro {
    pub summary: MacroSummary,
    pub warnings: Vec<String>,
}

/// What `update_macro_info` changes (absent: unchanged).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MacroChanges {
    pub name: Option<String>,
    pub description: Option<String>,
    pub folder: Option<String>,
    pub favorite: Option<bool>,
    pub shortcut: Option<Binding>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub macros: Vec<MacroSummary>,
    pub profiles: Vec<ProfileRef>,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenLayout {
    pub monitors: Vec<Monitor>,
    pub bounds: Option<model::Rect>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedPoint {
    pub point: Point,
    pub monitor: Option<usize>,
    pub on_sotyware: bool,
}

// ------------------------------------------------------------ commands

fn automation<'a>(state: &'a State<'_, Arc<Automation>>) -> &'a Arc<Automation> {
    state.inner()
}

async fn blocking<T: Send + 'static>(
    state: &State<'_, Arc<Automation>>,
    f: impl FnOnce(Arc<Automation>) -> Result<T> + Send + 'static,
) -> Result<T> {
    let a = Arc::clone(automation(state));
    tokio::task::spawn_blocking(move || f(a))
        .await
        .unwrap_or_else(|e| {
            Err(AutomationError::new(
                ErrorKind::Storage,
                format!("the operation stopped unexpectedly: {e}"),
            ))
        })
}

/// The live status. Allowed by `allow-automation-status`.
#[tauri::command(async)]
pub fn automation_status(state: State<'_, Arc<Automation>>) -> AutomationStatus {
    automation(&state).status()
}

/// Sends the status on `on_event` now and whenever it changes (at most
/// ten times a second; once a second while a session runs). Allowed by
/// `allow-watch-automation`.
#[tauri::command]
pub fn watch_automation(
    state: State<'_, Arc<Automation>>,
    on_event: Channel<AutomationStatus>,
) -> u64 {
    automation(&state).subscribe(Arc::new(on_event))
}

/// Stops a `watch_automation`. Allowed by `allow-unwatch-automation`.
#[tauri::command]
pub fn unwatch_automation(state: State<'_, Arc<Automation>>, subscription: u64) {
    automation(&state).unsubscribe(subscription);
}

/// The settings and profiles. Allowed by `allow-automation-config`.
#[tauri::command(async)]
pub fn automation_config(state: State<'_, Arc<Automation>>) -> AutomationConfig {
    automation(&state).config()
}

/// Starts the active profile's autoclicker. Allowed by
/// `allow-start-clicker`.
#[tauri::command]
pub async fn start_clicker(
    state: State<'_, Arc<Automation>>,
    from_dashboard: bool,
    confirmed: bool,
) -> Result<()> {
    blocking(&state, move |a| a.start_clicker(from_dashboard, confirmed)).await
}

/// Plays a macro (`id`), or the active profile's (`null`). Allowed by
/// `allow-play-macro`.
#[tauri::command]
pub async fn play_macro(
    state: State<'_, Arc<Automation>>,
    id: Option<String>,
    from_dashboard: bool,
    confirmed: bool,
) -> Result<()> {
    blocking(&state, move |a| a.play(id, from_dashboard, confirmed)).await
}

/// Starts recording. Allowed by `allow-start-recording`.
#[tauri::command]
pub async fn start_recording(
    state: State<'_, Arc<Automation>>,
    from_dashboard: bool,
) -> Result<()> {
    blocking(&state, move |a| a.start_recording(from_dashboard)).await
}

/// Pauses or resumes what runs. Allowed by `allow-set-automation-paused`.
#[tauri::command(async)]
pub fn set_automation_paused(state: State<'_, Arc<Automation>>, paused: bool) -> Result<()> {
    automation(&state).set_paused(paused)
}

/// Stops what runs and waits until every key and button is released.
/// Allowed by `allow-stop-automation`.
#[tauri::command]
pub async fn stop_automation(state: State<'_, Arc<Automation>>) -> Result<()> {
    blocking(&state, |a| {
        a.stop_session(StopReason::User(DASHBOARD_STOP.to_owned()), true);
        Ok(())
    })
    .await
}

/// Saves the stopped recording as a macro. Allowed by
/// `allow-save-recording`.
#[tauri::command]
pub async fn save_recording(
    state: State<'_, Arc<Automation>>,
    name: String,
    description: String,
    folder: String,
) -> Result<MacroSummary> {
    blocking(&state, move |a| {
        a.save_recording(&name, &description, &folder)
    })
    .await
}

/// Forgets the stopped recording. Allowed by `allow-discard-recording`.
#[tauri::command(async)]
pub fn discard_recording(state: State<'_, Arc<Automation>>) {
    automation(&state).discard_recording();
}

/// The macro library (without actions). Allowed by `allow-list-macros`.
#[tauri::command(async)]
pub fn list_macros(state: State<'_, Arc<Automation>>) -> Vec<MacroSummary> {
    automation(&state).macros()
}

/// One macro with its actions. Allowed by `allow-get-macro`.
#[tauri::command]
pub async fn get_macro(state: State<'_, Arc<Automation>>, id: String) -> Result<Macro> {
    blocking(&state, move |a| a.load_macro(&id)).await
}

/// A new empty macro. Allowed by `allow-create-macro`.
#[tauri::command]
pub async fn create_macro(
    state: State<'_, Arc<Automation>>,
    name: String,
    description: String,
    folder: String,
) -> Result<Macro> {
    blocking(&state, move |a| {
        a.create_macro(&name, &description, &folder)
    })
    .await
}

/// Saves an edited macro. Allowed by `allow-save-macro`.
#[tauri::command]
pub async fn save_macro(state: State<'_, Arc<Automation>>, document: Macro) -> Result<SavedMacro> {
    blocking(&state, move |a| a.save_macro(document)).await
}

/// Renames, moves, (un)favourites a macro or sets its shortcut. Allowed by
/// `allow-update-macro-info`.
#[tauri::command]
pub async fn update_macro_info(
    state: State<'_, Arc<Automation>>,
    id: String,
    changes: MacroChanges,
) -> Result<MacroSummary> {
    blocking(&state, move |a| a.update_macro_info(&id, changes)).await
}

/// Copies a macro. Allowed by `allow-duplicate-macro`.
#[tauri::command]
pub async fn duplicate_macro(
    state: State<'_, Arc<Automation>>,
    id: String,
) -> Result<MacroSummary> {
    blocking(&state, move |a| a.duplicate_macro(&id)).await
}

/// Deletes a macro. Allowed by `allow-delete-macro`.
#[tauri::command]
pub async fn delete_macro(state: State<'_, Arc<Automation>>, id: String) -> Result<()> {
    blocking(&state, move |a| a.delete_macro(&id)).await
}

/// A new profile, empty or copied from `from`. Allowed by
/// `allow-create-profile`.
#[tauri::command]
pub async fn create_profile(
    state: State<'_, Arc<Automation>>,
    name: String,
    from: Option<String>,
) -> Result<Profile> {
    blocking(&state, move |a| a.create_profile(&name, from.as_deref())).await
}

/// Saves an edited profile. Allowed by `allow-save-profile`.
#[tauri::command]
pub async fn save_profile(state: State<'_, Arc<Automation>>, profile: Profile) -> Result<Profile> {
    blocking(&state, move |a| a.save_profile(profile)).await
}

/// Deletes a profile. Allowed by `allow-delete-profile`.
#[tauri::command]
pub async fn delete_profile(state: State<'_, Arc<Automation>>, id: String) -> Result<()> {
    blocking(&state, move |a| a.delete_profile(&id)).await
}

/// Makes a profile the active one. Allowed by `allow-activate-profile`.
#[tauri::command]
pub async fn activate_profile(state: State<'_, Arc<Automation>>, id: String) -> Result<()> {
    blocking(&state, move |a| a.activate_profile(&id)).await
}

/// The profile sotyware starts with. Allowed by
/// `allow-set-default-profile`.
#[tauri::command]
pub async fn set_default_profile(state: State<'_, Arc<Automation>>, id: String) -> Result<()> {
    blocking(&state, move |a| a.set_default_profile(&id)).await
}

/// Saves the shared settings. Allowed by `allow-save-automation-settings`.
#[tauri::command]
pub async fn save_automation_settings(
    state: State<'_, Arc<Automation>>,
    settings: Settings,
) -> Result<Settings> {
    blocking(&state, move |a| a.save_settings(settings)).await
}

/// Writes the chosen macros and profiles to a file the user picks in a
/// save dialog Rust opens; the file's name, or `null` if cancelled.
/// Allowed by `allow-export-automation`.
#[tauri::command]
pub async fn export_automation(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<Automation>>,
    macros: Vec<String>,
    profiles: Vec<String>,
) -> Result<Option<String>> {
    let suggested = {
        let a = automation(&state);
        let name = match (macros.as_slice(), profiles.as_slice()) {
            ([one], []) => a
                .macros()
                .into_iter()
                .find(|m| &m.id == one)
                .map(|m| m.name),
            ([], [one]) => a
                .config()
                .profiles
                .into_iter()
                .find(|p| &p.id == one)
                .map(|p| p.name),
            _ => None,
        };
        file_name(name.as_deref().unwrap_or("sotyware automation"))
    };
    let text = blocking(&state, move |a| a.export_text(&macros, &profiles)).await?;
    let Some(path) = native::choose_save(&window, suggested).await else {
        return Ok(None);
    };
    let shown = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || write_file(&path, &text))
        .await
        .map_err(|e| AutomationError::new(ErrorKind::File, e.to_string()))??;
    Ok(Some(shown))
}

/// Reads macros and profiles from a file the user picks in an open dialog
/// Rust opens; `null` if cancelled. Allowed by `allow-import-automation`.
#[tauri::command]
pub async fn import_automation(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<Automation>>,
) -> Result<Option<ImportReport>> {
    let Some(path) = native::choose_open(&window).await else {
        return Ok(None);
    };
    blocking(&state, move |a| {
        let text = read_file(&path)?;
        a.import_text(&text)
    })
    .await
    .map(Some)
}

/// The pointer's position after `delay_ms` (to move it where it should
/// click meanwhile). Allowed by `allow-capture-cursor-position`.
#[tauri::command]
pub async fn capture_cursor_position(
    state: State<'_, Arc<Automation>>,
    delay_ms: u32,
) -> Result<CapturedPoint> {
    automation(&state).backend()?;
    tokio::time::sleep(Duration::from_millis(u64::from(delay_ms.min(10_000)))).await;
    automation(&state).cursor_now()
}

/// The active window after `delay_ms` (to switch to it meanwhile).
/// Allowed by `allow-capture-active-window`.
#[tauri::command]
pub async fn capture_active_window(
    state: State<'_, Arc<Automation>>,
    delay_ms: u32,
) -> Result<WindowInfo> {
    automation(&state).backend()?;
    tokio::time::sleep(Duration::from_millis(u64::from(delay_ms.min(10_000)))).await;
    automation(&state).window_now()
}

/// The monitors. Allowed by `allow-screen-layout`.
#[tauri::command(async)]
pub fn screen_layout(state: State<'_, Arc<Automation>>) -> Result<ScreenLayout> {
    automation(&state).screen()
}

/// A file name from a macro or profile name (characters no file system
/// takes become `-`).
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '-'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim();
    format!(
        "{}.sotyware.json",
        if cleaned.is_empty() {
            "automation"
        } else {
            cleaned
        }
    )
}

fn write_file(path: &std::path::Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            AutomationError::new(
                ErrorKind::File,
                format!("the file could not be written: {e}"),
            )
        })
}

fn read_file(path: &std::path::Path) -> Result<String> {
    let meta = std::fs::metadata(path).map_err(|e| {
        AutomationError::new(ErrorKind::File, format!("the file cannot be read: {e}"))
    })?;
    if meta.len() > model::MAX_BUNDLE_BYTES {
        return Err(AutomationError::new(
            ErrorKind::File,
            "the file is larger than 64 MB; it is not a sotyware macro file",
        ));
    }
    std::fs::read_to_string(path)
        .map_err(|e| AutomationError::new(ErrorKind::File, format!("the file cannot be read: {e}")))
}

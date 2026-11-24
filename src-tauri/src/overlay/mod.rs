//! The gaming notes overlay (ADR-052): a small always-on-top window of
//! notes over a game or any other program, with its own global shortcuts.
//!
//! - [`model`]: notes, their order, the settings, the shortcuts' actions;
//!   [`placement`]: where the window opens. Both pure.
//! - [`NotesStore`]: `notes.sqlite3` (`storage::SqliteNotesStore`).
//! - [`OverlayHost`]: the window itself, behind a trait so that every rule
//!   here is tested without a display (`host::TauriHost` in the app).
//! - This module: [`NotesOverlay`], the managed state: the notes' order
//!   and the settings in memory, the window's visibility, mode and
//!   placement, the shortcuts (registered through a second instance of the
//!   autoclicker's platform backend, which only ever registers shortcuts
//!   here), the status the pages watch, and the commands.
//!
//! The overlay exists only while the built-in feature `notes-overlay` runs
//! (off until the user turns it on): starting it registers the shortcuts
//! and, when the user turned it on now or `showAtStartup` asks for it at
//! startup, shows the window; stopping it, or quitting, unregisters them
//! and destroys the window. The notes stay in the database either way.
//!
//! **Interactive and passive.** Interactive, the window takes clicks and,
//! when clicked or switched to by shortcut, the keyboard. Passive, clicks
//! go through it (`set_ignore_cursor_events`: `WS_EX_TRANSPARENT` and
//! `WS_EX_LAYERED` on Windows, an empty input shape on X11) and it never
//! takes the keyboard (`set_focusable(false)`); if it had the keyboard, it
//! gives it back to the program that had it before. Showing the window
//! never takes the keyboard from the game: only switching to interactive by
//! shortcut, creating a note by shortcut, or a click does.
//!
//! **Threads.** The window is driven from worker threads only (commands
//! are `async`, shortcuts arrive on their own thread), one operation at a
//! time (`window_ops`), and never while `inner` is held: a window call can
//! wait for the event loop, whose handlers take `inner` or `track` only.
//!
//! Design: docs/architecture/notes-overlay.md.

pub mod model;
pub mod placement;

#[cfg(windows)]
mod focus;
mod host;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{State, WebviewWindow};
use tokio::sync::Notify;

use crate::automation::claims::{self, Claims, Owner};
use crate::automation::input::{Backend, HotkeySink};
use crate::automation::keys::Chord;
use crate::automation::{KeyView, key_views};
use crate::features::StoreError;
use crate::journal::now_ms;
use crate::window_state::{Placement, Rect, Saved, Screen};
pub use host::TauriHost;
use model::{
    DEFAULT_TITLE, MAX_NOTES, MAX_SIZE, MIN_SIZE, Mode, Note, NoteMeta, NoteOrder, OPACITY_RANGE,
    OverlayAction, OverlaySettings, OverlayState, TextSize,
};

/// The overlay window's label. Lowercase, like every label (AGENTS.md).
pub const OVERLAY_LABEL: &str = "overlay";
/// The page the window loads (a second entry of the Vite build).
pub const OVERLAY_PAGE: &str = "overlay.html";
/// Status sends are gathered for this long, so a burst is one message.
const STATUS_EVERY: Duration = Duration::from_millis(40);
/// How long after a show a position elsewhere than asked is the window
/// manager's placement or not settled yet (see `NotesOverlay::expect`).
const PLACED_WITHIN: Duration = Duration::from_millis(1500);
/// Where the overlay must be right after a show, on X11. A window mapped
/// again may be placed anew by the window manager (openbox put it at 0, 0
/// even when it was moved back before it was shown), GTK skips a move to
/// where it last asked to go if the window manager's answer has not
/// arrived yet, and the position tao reports just after a window is
/// mapped can be one the window never had (0, 0, from the frame before
/// the window manager placed it; the window stayed at its corner and no
/// later event said so). So for a moment after each show (set before it,
/// since the event loop may report the placement while `show` runs, and
/// again after it), a position elsewhere is never remembered, and the
/// first one is answered with a move back (a no-op where the window is in
/// fact there). Each report comes twice (moved, resized), the same.
#[derive(Clone, Copy, Debug)]
struct Expected {
    x: i32,
    y: i32,
    at: Instant,
    corrected: bool,
}

/// The names of the two documents in `notes.sqlite3`.
const SETTINGS_DOC: &str = "settings";
const STATE_DOC: &str = "state";

// ------------------------------------------------------------- errors

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// Something sent is not valid; nothing changed.
    InvalidInput,
    /// No note with this id (deleted meanwhile).
    NotFound,
    /// The note changed since the page read it.
    Conflict,
    /// There are as many notes as there can be.
    Limit,
    /// The `notes-overlay` feature is off.
    FeatureOff,
    /// The database cannot be used.
    Storage,
    /// The window could not be created or changed.
    Window,
}

/// The error every overlay command returns: a kind for the page's logic
/// and a sentence to show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverlayError {
    pub kind: ErrorKind,
    pub message: String,
}

impl OverlayError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

fn invalid(message: impl Into<String>) -> OverlayError {
    OverlayError::new(ErrorKind::InvalidInput, message)
}

fn storage(e: impl std::fmt::Display) -> OverlayError {
    OverlayError::new(
        ErrorKind::Storage,
        format!("the notes database cannot be used: {e}"),
    )
}

fn not_found() -> OverlayError {
    OverlayError::new(ErrorKind::NotFound, "this note does not exist any more")
}

fn feature_off() -> OverlayError {
    OverlayError::new(
        ErrorKind::FeatureOff,
        "Turn on the notes overlay first (Settings › Notes overlay)",
    )
}

fn window_error(what: &str, e: impl std::fmt::Display) -> OverlayError {
    OverlayError::new(
        ErrorKind::Window,
        format!("The overlay could not be {what}: {e}"),
    )
}

type Result<T, E = OverlayError> = std::result::Result<T, E>;

// -------------------------------------------------------------- store

/// What the overlay stores; `storage::SqliteNotesStore` in the app, an
/// in-memory store in tests.
pub trait NotesStore: Send {
    fn notes(&mut self) -> Result<Vec<Note>, StoreError>;
    fn note(&mut self, id: &str) -> Result<Option<Note>, StoreError>;
    /// Adds `note`; `moved` are notes whose place changed to make room.
    fn insert_note(&mut self, note: &Note, moved: &[NoteOrder]) -> Result<(), StoreError>;
    /// `false` when no such note exists.
    fn update_content(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        updated_ms: i64,
        revision: u64,
    ) -> Result<bool, StoreError>;
    fn update_order(&mut self, changed: &[NoteOrder]) -> Result<(), StoreError>;
    fn delete_note(&mut self, id: &str) -> Result<(), StoreError>;
    fn document(&mut self, name: &str) -> Result<Option<String>, StoreError>;
    fn save_document(&mut self, name: &str, data: &str) -> Result<(), StoreError>;
    fn delete_document(&mut self, name: &str) -> Result<(), StoreError>;
}

// --------------------------------------------------------------- host

/// The monitors and the pointer, as placing a new window needs them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    pub screens: Vec<Screen>,
    pub primary: Option<Screen>,
    /// Physical pixels of the virtual desktop.
    pub cursor: Option<(f64, f64)>,
}

/// How a new window starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowInit {
    /// Expanded; a collapsed window is as high as its title bar.
    pub placement: Placement,
    pub collapsed: bool,
    pub mode: Mode,
}

/// The overlay window. Every method may wait for the event loop: never
/// called from it, nor while `NotesOverlay::inner` is held.
pub trait OverlayHost: Send + Sync {
    fn geometry(&self) -> Geometry;
    fn exists(&self) -> bool;
    /// Creates the hidden window in its place and mode.
    fn create(&self, init: &WindowInit) -> std::result::Result<(), String>;
    /// The outer position and inner size now (physical), and the scale.
    fn bounds(&self) -> Option<(Rect, f64)>;
    fn place(&self, placement: Placement, collapsed: bool) -> std::result::Result<(), String>;
    /// Shows the window above every other one it can be above, taking the
    /// keyboard only if `activate`.
    fn show(&self, mode: Mode, activate: bool) -> std::result::Result<(), String>;
    fn hide(&self) -> std::result::Result<(), String>;
    fn destroy(&self);
    /// Click-through and focusability.
    fn set_mode(&self, mode: Mode) -> std::result::Result<(), String>;
    /// Takes the keyboard, remembering the window that had it.
    fn take_focus(&self) -> std::result::Result<(), String>;
    /// If the overlay has the keyboard, gives it back.
    fn give_focus_back(&self);
    /// Shrinks to the title bar, or back to `expanded_height` (physical).
    fn set_collapsed(
        &self,
        collapsed: bool,
        expanded_height: u32,
    ) -> std::result::Result<(), String>;
    /// The dashboard, shown and focused.
    fn show_dashboard(&self);
    /// `url` in the default browser.
    fn open_link(&self, url: &str) -> std::result::Result<(), String>;
}

// ------------------------------------------------------------- status

/// What the page should give the keyboard to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusTarget {
    /// The note's title, selected (a note just created by shortcut).
    Title,
    /// The note itself (switched to interactive by shortcut).
    #[default]
    Content,
}

/// One shortcut of the overlay and whether the system accepted it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHotkeyView {
    pub action: OverlayAction,
    pub label: &'static str,
    pub chord: Chord,
    /// The keys' names on this keyboard (`Ctrl + Shift + F1`).
    pub keys: String,
    pub registered: bool,
    pub error: Option<String>,
}

/// Everything the pages show that changes while sotyware runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatus {
    pub seq: u64,
    /// The `notes-overlay` feature runs.
    pub running: bool,
    pub visible: bool,
    pub mode: Mode,
    pub collapsed: bool,
    pub opacity: u8,
    pub text_size: TextSize,
    pub active_note: Option<String>,
    /// Bumped when Rust chose another note on its own (a shortcut); the
    /// page then shows `active_note`.
    pub active_seq: u64,
    /// Bumped when Rust changed the notes on its own (a note created by
    /// shortcut); the page then reads them again.
    pub notes_seq: u64,
    /// Bumped when the page should give the keyboard to `focus_target`.
    pub focus_seq: u64,
    pub focus_target: FocusTarget,
    /// Bumped when the overlay asked for its settings; the dashboard then
    /// opens Settings › Notes overlay.
    pub settings_seq: u64,
    pub note_count: usize,
    pub hotkeys: Vec<OverlayHotkeyView>,
    /// Why no global shortcut can work on this system, if none can.
    pub shortcuts_note: Option<String>,
    /// Something a shortcut or the window could not do.
    pub notice: Option<String>,
    pub storage_error: Option<String>,
}

/// Receives the status whenever it changes.
pub trait StatusSink: Send + Sync {
    /// `false` when the receiver is gone.
    fn send(&self, status: &OverlayStatus) -> bool;
}

impl StatusSink for Channel<OverlayStatus> {
    fn send(&self, status: &OverlayStatus) -> bool {
        Channel::send(self, status.clone()).is_ok()
    }
}

/// The limits the settings page enforces before Rust does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayLimits {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub min_opacity: u8,
    pub max_opacity: u8,
    pub max_notes: usize,
    pub max_title_chars: usize,
    pub max_body_chars: usize,
}

/// The settings with what the settings page needs to show them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayConfig {
    pub settings: OverlaySettings,
    pub suggestions: BTreeMap<OverlayAction, Chord>,
    pub keys: Vec<KeyView>,
    pub limits: OverlayLimits,
}

/// What `list_notes` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesList {
    /// In display order.
    pub notes: Vec<Note>,
    pub active_note: Option<String>,
}

/// What `save_note` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedNote {
    pub revision: u64,
    pub updated_ms: i64,
}

/// What `duplicate_note` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Duplicated {
    pub note: Note,
    pub order: Vec<NoteOrder>,
}

/// What `delete_note` returns: the note shown now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deleted {
    pub active_note: Option<String>,
}

/// Who asked for a change: it decides what happens to the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Dashboard,
    Overlay,
    Shortcut,
}

impl Origin {
    fn of(window: &WebviewWindow) -> Self {
        if window.label() == OVERLAY_LABEL {
            Self::Overlay
        } else {
            Self::Dashboard
        }
    }
}

// ------------------------------------------------------------ service

struct Registration {
    id: u32,
    action: OverlayAction,
    chord: Chord,
    result: std::result::Result<(), String>,
}

enum Dispatch {
    Hotkey(u32),
    Quit,
}

/// The shortcut thread and the channel to it.
type Dispatcher = (mpsc::Sender<Dispatch>, JoinHandle<()>);

struct Inner {
    settings: OverlaySettings,
    collapsed: bool,
    active: Option<String>,
    notes: Vec<NoteMeta>,
    storage_error: Option<String>,
    running: bool,
    /// `setup` has finished: a start now is the user's doing.
    startup_done: bool,
    /// sotyware is quitting: the window goes with the process.
    exiting: bool,
    visible: bool,
    mode: Mode,
    backend: Option<Arc<dyn Backend>>,
    backend_error: Option<String>,
    registrations: Vec<Registration>,
    next_hotkey: u32,
    dispatcher: Option<Dispatcher>,
    active_seq: u64,
    notes_seq: u64,
    focus_seq: u64,
    focus_target: FocusTarget,
    settings_seq: u64,
    notice: Option<String>,
    /// The state document as last written, to write only changes.
    state_written: Option<String>,
}

/// Where the window is, tracked from its own events (the event loop's
/// handlers take only this lock).
#[derive(Default)]
struct Track {
    /// Expanded: while collapsed, the height stays the expanded one.
    latest: Option<Saved>,
    collapsed: bool,
}

/// Opens the platform's backend for the shortcuts; tests pass a mock.
pub type BackendFactory =
    Box<dyn Fn(HotkeySink) -> std::result::Result<Arc<dyn Backend>, String> + Send + Sync>;

/// The notes overlay: managed state.
pub struct NotesOverlay {
    inner: Mutex<Inner>,
    store: Mutex<Option<Box<dyn NotesStore>>>,
    /// Why the database could not be opened, if it could not (fixed at
    /// load: read without `inner`, which callers of the store may hold).
    open_error: Option<String>,
    track: Mutex<Track>,
    /// Where the window must be right after it was shown, until the
    /// window manager placed it ([`window_changed`](Self::window_changed)).
    expect: Mutex<Option<Expected>>,
    /// One window operation at a time.
    window_ops: Mutex<()>,
    host: OnceLock<Box<dyn OverlayHost>>,
    factory: BackendFactory,
    /// Why global shortcuts cannot work here (Wayland, macOS), if so.
    shortcuts_note: Option<String>,
    claims: OnceLock<Arc<Claims>>,
    changed: Notify,
    subscribers: Mutex<Vec<(u64, Arc<dyn StatusSink>)>>,
    notifier: Mutex<bool>,
    next_subscriber: AtomicU64,
    seq: AtomicU64,
    self_ref: OnceLock<Weak<NotesOverlay>>,
}

/// The link the `notes-overlay` feature uses; set once in `run()`.
pub static LINK: OnceLock<Arc<NotesOverlay>> = OnceLock::new();

fn new_id() -> String {
    format!("note-{}", crate::backup::engine::new_id())
}

fn lock_or<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl NotesOverlay {
    /// The service of `run()`: shortcuts through the platform's backend.
    pub fn load(store: std::result::Result<Box<dyn NotesStore>, StoreError>) -> Arc<Self> {
        Self::with(
            store,
            crate::automation::native::shortcuts_unavailable(),
            Box::new(crate::automation::native::open),
        )
    }

    pub fn with(
        store: std::result::Result<Box<dyn NotesStore>, StoreError>,
        shortcuts_note: Option<String>,
        factory: BackendFactory,
    ) -> Arc<Self> {
        let (mut store, mut storage_error) = match store {
            Ok(s) => (Some(s), None),
            Err(e) => {
                eprintln!("sotyware: the notes database cannot be used: {e}");
                (None, Some(e.to_string()))
            }
        };
        let open_error = storage_error.clone();
        let mut settings = OverlaySettings::default();
        let mut state = OverlayState::new();
        let mut state_written = None;
        let mut notes: Vec<NoteMeta> = Vec::new();
        if let Some(s) = store.as_mut() {
            let loaded = (|| -> std::result::Result<(), StoreError> {
                // A document this version cannot read is left alone: the
                // defaults are used until the user saves.
                if let Some(text) = s.document(SETTINGS_DOC)? {
                    match serde_json::from_str::<OverlaySettings>(&text)
                        .map_err(|e| e.to_string())
                        .and_then(OverlaySettings::validated)
                    {
                        Ok(v) => settings = v,
                        Err(e) => eprintln!(
                            "sotyware: skipping stored overlay settings this version cannot use: {e}"
                        ),
                    }
                }
                if let Some(text) = s.document(STATE_DOC)? {
                    if let Some(st) = OverlayState::parse(&text) {
                        state = st;
                    }
                    state_written = Some(text);
                }
                notes = s.notes()?.iter().map(Note::meta).collect();
                Ok(())
            })();
            if let Err(e) = loaded {
                eprintln!("sotyware: the notes database cannot be read: {e}");
                storage_error = Some(e.to_string());
            }
        }
        let active = model::opening_note(&notes, state.active_note.as_deref(), settings.open_note);
        let mode = settings.default_mode;
        let this = Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                collapsed: state.collapsed,
                active,
                notes,
                storage_error,
                running: false,
                startup_done: false,
                exiting: false,
                visible: false,
                mode,
                backend: None,
                backend_error: None,
                registrations: Vec::new(),
                next_hotkey: 1,
                dispatcher: None,
                active_seq: 0,
                notes_seq: 0,
                focus_seq: 0,
                focus_target: FocusTarget::Content,
                settings_seq: 0,
                notice: None,
                state_written,
            }),
            store: Mutex::new(store),
            open_error,
            track: Mutex::new(Track {
                latest: state.placement,
                collapsed: state.collapsed,
            }),
            expect: Mutex::new(None),
            window_ops: Mutex::new(()),
            host: OnceLock::new(),
            factory,
            shortcuts_note,
            claims: OnceLock::new(),
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

    /// The window it drives; set once in `setup`.
    pub fn attach(&self, host: Box<dyn OverlayHost>) {
        if self.host.set(host).is_err() {
            eprintln!("sotyware: the overlay window host was attached twice");
        }
    }

    /// Shares the record of which feature holds which shortcut with the
    /// autoclicker (ADR-052).
    pub fn share_claims(self: &Arc<Self>, shared: Arc<Claims>) {
        shared.on_release(
            Owner::NotesOverlay,
            Arc::downgrade(self) as Weak<dyn claims::Retry>,
        );
        let _ = self.claims.set(shared);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock_or(&self.inner)
    }

    /// Something changed: the status is sent soon.
    fn touch(&self) {
        self.seq.fetch_add(1, Ordering::Relaxed);
        self.changed.notify_one();
    }

    /// Runs `f` on the store. Never takes `inner`: callers may hold it.
    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut dyn NotesStore) -> std::result::Result<T, StoreError>,
    ) -> Result<T> {
        let mut store = lock_or(&self.store);
        match store.as_mut() {
            Some(s) => f(s.as_mut()).map_err(storage),
            None => Err(storage(
                self.open_error
                    .as_deref()
                    .unwrap_or("it could not be opened"),
            )),
        }
    }

    // ------------------------------------------------ feature lifecycle

    /// Starts the feature: the shortcuts are registered (where the system
    /// allows it; their refusals are shown, the overlay works without
    /// them) and, when the user turned it on now, the window is shown.
    pub fn start(&self) -> std::result::Result<(), String> {
        if self.lock().running {
            return Ok(());
        }
        let opened = if self.shortcuts_note.is_none() {
            Some(self.open_backend())
        } else {
            None
        };
        let show = {
            let mut inner = self.lock();
            inner.running = true;
            inner.visible = false;
            inner.mode = inner.settings.default_mode;
            inner.notice = None;
            match opened {
                Some(Ok((backend, dispatcher))) => {
                    inner.backend = Some(backend);
                    inner.dispatcher = Some(dispatcher);
                    inner.backend_error = None;
                }
                Some(Err(e)) => {
                    eprintln!("sotyware: the overlay's shortcuts are not available: {e}");
                    inner.backend_error = Some(e);
                }
                None => {}
            }
            self.sync_hotkeys(&mut inner);
            inner.startup_done
        };
        self.touch();
        if show && let Err(e) = self.show_now(false) {
            self.notice(e.message);
        }
        Ok(())
    }

    fn open_backend(&self) -> std::result::Result<(Arc<dyn Backend>, Dispatcher), String> {
        let (tx, rx) = mpsc::channel::<Dispatch>();
        let sink_tx = tx.clone();
        let sink: HotkeySink = Arc::new(move |id| {
            let _ = sink_tx.send(Dispatch::Hotkey(id));
        });
        let backend = (self.factory)(sink)?;
        let weak = self.self_ref.get().cloned().unwrap_or_default();
        let dispatcher = std::thread::Builder::new()
            .name("sotyware-overlay-hotkeys".into())
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
        Ok((backend, (tx, dispatcher)))
    }

    /// Stops the feature: the shortcuts are unregistered, the window is
    /// destroyed (its place and the note shown are remembered first). When
    /// sotyware quits the window is left to go with the process: the exit
    /// runs on the event loop's thread, which must never wait for
    /// `window_ops` (a worker holding it may be waiting for that thread).
    pub fn stop(&self) {
        let (backend, dispatcher, exiting) = {
            let mut inner = self.lock();
            let exiting = inner.exiting;
            inner.running = false;
            inner.visible = false;
            let backend = inner.backend.take();
            if let Some(b) = &backend {
                for r in inner.registrations.drain(..) {
                    if r.result.is_ok() {
                        b.unregister_hotkey(r.id);
                    }
                }
            }
            inner.registrations.clear();
            self.sync_hotkeys(&mut inner);
            (backend, inner.dispatcher.take(), exiting)
        };
        if let Some(b) = backend {
            b.shutdown();
        }
        if let Some((tx, handle)) = dispatcher {
            let _ = tx.send(Dispatch::Quit);
            // The dispatcher may be the caller (a shortcut that ended in a
            // stop never does, but a join on itself would hang).
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
        if !exiting {
            let _ops = lock_or(&self.window_ops);
            if let Some(host) = self.host.get() {
                host.destroy();
            }
        }
        self.persist_state();
        self.touch();
    }

    /// sotyware is quitting (`release_resources`, before the features
    /// stop): from now on nothing drives the window.
    pub fn exiting(&self) {
        self.lock().exiting = true;
    }

    /// `setup` has finished: from now on a start is the user's doing.
    /// Shows the overlay if the feature runs and the settings ask for it,
    /// on a worker thread: `setup` runs on the event loop's thread, which
    /// must never wait for `window_ops`.
    pub fn startup_finished(self: &Arc<Self>) {
        let show = {
            let mut inner = self.lock();
            inner.startup_done = true;
            inner.running && inner.settings.show_at_startup
        };
        if show {
            let this = Arc::clone(self);
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = this.show_now(false) {
                    this.notice(e.message);
                }
            });
        }
    }

    /// Quitting: as [`stop`](Self::stop) when running (without touching
    /// the window), else only the state is saved.
    pub fn shutdown(&self) {
        self.exiting();
        if self.lock().running {
            self.stop();
        } else {
            self.persist_state();
        }
    }

    fn notice(&self, message: String) {
        eprintln!("sotyware: overlay: {message}");
        self.lock().notice = Some(message);
        self.touch();
    }

    // ------------------------------------------------------ shortcuts

    /// Registers what should be registered and unregisters the rest. A
    /// shortcut the autoclicker holds is refused with its name.
    fn sync_hotkeys(&self, inner: &mut Inner) {
        let wanted = if inner.running {
            inner.settings.active_shortcuts()
        } else {
            Vec::new()
        };
        let Some(backend) = inner.backend.clone() else {
            let why = if !inner.running {
                "Not registered: the notes overlay is off".to_owned()
            } else if let Some(n) = self
                .shortcuts_note
                .as_ref()
                .or(inner.backend_error.as_ref())
            {
                format!("Not registered: {n}")
            } else {
                "Not registered".to_owned()
            };
            inner.registrations = wanted
                .into_iter()
                .map(|(action, chord)| Registration {
                    id: 0,
                    action,
                    chord,
                    result: Err(why.clone()),
                })
                .collect();
            if let Some(c) = self.claims.get() {
                c.publish(Owner::NotesOverlay, &[]);
            }
            return;
        };
        let mut keep: Vec<Registration> = Vec::new();
        for r in inner.registrations.drain(..) {
            let still = wanted.iter().any(|(a, c)| *a == r.action && *c == r.chord);
            if still && r.result.is_ok() {
                keep.push(r);
            } else if r.result.is_ok() {
                backend.unregister_hotkey(r.id);
            }
        }
        let mut out: Vec<Registration> = Vec::new();
        for (action, chord) in wanted {
            if let Some(i) = keep
                .iter()
                .position(|r| r.action == action && r.chord == chord)
            {
                out.push(keep.remove(i));
                continue;
            }
            let held_by = self
                .claims
                .get()
                .and_then(|c| c.holder(chord, Owner::NotesOverlay));
            let result = if let Some(feature) = held_by {
                Err(format!(
                    "Not registered: {} uses this shortcut",
                    feature.label()
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
                            action,
                            chord,
                            result: Ok(()),
                        });
                        continue;
                    }
                    Err(e) => Err(format!("Not registered: {e}")),
                }
            };
            out.push(Registration {
                id: 0,
                action,
                chord,
                result,
            });
        }
        inner.registrations = out;
        if let Some(c) = self.claims.get() {
            let held: Vec<Chord> = inner
                .registrations
                .iter()
                .filter(|r| r.result.is_ok())
                .map(|r| r.chord)
                .collect();
            c.publish(Owner::NotesOverlay, &held);
        }
    }

    fn hotkey_views(&self, inner: &Inner) -> Vec<OverlayHotkeyView> {
        let backend = inner.backend.clone();
        inner
            .registrations
            .iter()
            .map(|r| OverlayHotkeyView {
                action: r.action,
                label: r.action.label(),
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
        let (action, keys) = {
            let inner = self.lock();
            if !inner.running {
                return;
            }
            let Some(r) = inner
                .registrations
                .iter()
                .find(|r| r.id == id && r.result.is_ok())
            else {
                return;
            };
            (r.action, r.chord.label_with(|k| k.label().to_owned()))
        };
        let result = self.shortcut(action);
        let notice = result.err().map(|e| format!("{keys}: {}", e.message));
        if let Some(n) = &notice {
            eprintln!("sotyware: overlay shortcut {n}");
        }
        self.lock().notice = notice;
        self.touch();
    }

    /// What a shortcut does.
    fn shortcut(&self, action: OverlayAction) -> Result<()> {
        let (visible, mode) = {
            let inner = self.lock();
            (inner.visible, inner.mode)
        };
        match action {
            OverlayAction::ToggleVisible => {
                if visible {
                    self.hide_now()
                } else {
                    self.show_now(false)
                }
            }
            OverlayAction::ToggleMode => {
                if !visible {
                    self.show_now(false)?;
                    self.set_mode(Mode::Interactive, Origin::Shortcut).map(drop)
                } else {
                    let next = match mode {
                        Mode::Interactive => Mode::Passive,
                        Mode::Passive => Mode::Interactive,
                    };
                    self.set_mode(next, Origin::Shortcut).map(drop)
                }
            }
            OverlayAction::NewNote => {
                self.create_note(None, Origin::Shortcut)?;
                if !visible {
                    self.show_now(false)?;
                }
                // Typed into at once: its title, selected.
                self.switch_mode(
                    Mode::Interactive,
                    Origin::Shortcut,
                    Some(FocusTarget::Title),
                )
                .map(drop)
            }
            OverlayAction::NextNote | OverlayAction::PreviousNote => {
                let step = if action == OverlayAction::NextNote {
                    1
                } else {
                    -1
                };
                {
                    let mut inner = self.lock();
                    let next = model::step_note(&inner.notes, inner.active.as_deref(), step);
                    if next.is_some() {
                        inner.active = next;
                        inner.active_seq += 1;
                    }
                }
                if !visible {
                    self.show_now(false)?;
                }
                Ok(())
            }
        }
    }

    // -------------------------------------------------------- window

    fn host(&self) -> Result<&dyn OverlayHost> {
        self.host
            .get()
            .map(|h| h.as_ref())
            .ok_or_else(|| window_error("shown", "no window system"))
    }

    /// Where a new window goes: where it was (if remembered and still on
    /// a monitor), else the settings' place on the pointer's monitor.
    fn initial_placement(&self, settings: &OverlaySettings, geometry: &Geometry) -> Placement {
        let saved = if settings.remember_placement {
            lock_or(&self.track).latest
        } else {
            None
        };
        saved
            .and_then(|s| placement::restore(&s, &geometry.screens))
            .or_else(|| self.default_placement(settings, geometry))
            .unwrap_or(Placement {
                x: 100,
                y: 100,
                width: settings.width,
                height: settings.height,
                maximized: false,
            })
    }

    fn default_placement(
        &self,
        settings: &OverlaySettings,
        geometry: &Geometry,
    ) -> Option<Placement> {
        placement::screen_at(&geometry.screens, geometry.cursor, geometry.primary).map(|s| {
            placement::default_placement(&s, (settings.width, settings.height), settings.anchor)
        })
    }

    fn remember(&self, p: Placement, scale: f64) {
        lock_or(&self.track).latest = Some(Saved::new(
            Rect {
                x: p.x,
                y: p.y,
                width: p.width,
                height: p.height,
            },
            false,
            scale,
        ));
    }

    /// Shows the window (creating it first if needed), above the others,
    /// on a connected monitor.
    fn show_now(&self, activate: bool) -> Result<()> {
        let host = self.host()?;
        let _ops = lock_or(&self.window_ops);
        let (running, mode, collapsed, settings) = {
            let inner = self.lock();
            (
                inner.running,
                inner.mode,
                inner.collapsed,
                inner.settings.clone(),
            )
        };
        if !running {
            return Err(feature_off());
        }
        let geometry = host.geometry();
        // Where a window shown again goes, applied again once it is shown.
        let mut again = None;
        // Where it must be once shown (X11 only: `Expected`).
        let expected = if !host.exists() {
            let placement = self.initial_placement(&settings, &geometry);
            let scale = placement::screen_at(
                &geometry.screens,
                Some((f64::from(placement.x), f64::from(placement.y))),
                geometry.primary,
            )
            .map_or(1.0, |s| s.scale);
            host.create(&WindowInit {
                placement,
                collapsed,
                mode,
            })
            .map_err(|e| window_error("created", e))?;
            self.remember(placement, scale);
            Some(placement)
        } else {
            // Shown again: where it was, since a window manager may place a
            // window it maps again anew (openbox put it at 0, 0 even when it
            // was moved back before it was shown), and on a connected
            // monitor (one may have been unplugged while it was hidden).
            let tracked = lock_or(&self.track).latest;
            again = tracked
                .and_then(|s| placement::restore(&s, &geometry.screens))
                .or_else(|| self.default_placement(&settings, &geometry));
            if let Some(p) = again {
                host.place(p, collapsed)
                    .map_err(|e| window_error("placed", e))?;
            }
            again
        };
        // Before the show (the event loop may report the placement while
        // `show` runs) and again after it, counted from then (a busy event
        // loop reports late).
        if let Some(p) = expected {
            self.expect_at(p);
        }
        host.show(mode, activate)
            .map_err(|e| window_error("shown", e))?;
        if let Some(p) = again {
            host.place(p, collapsed)
                .map_err(|e| window_error("placed", e))?;
        }
        if let Some(p) = expected {
            self.expect_at(p);
        }
        self.lock().visible = true;
        self.touch();
        Ok(())
    }

    fn hide_now(&self) -> Result<()> {
        {
            let _ops = lock_or(&self.window_ops);
            if let Some(host) = self.host.get()
                && host.exists()
            {
                host.give_focus_back();
                host.hide().map_err(|e| window_error("hidden", e))?;
            }
        }
        self.lock().visible = false;
        self.persist_state();
        self.touch();
        Ok(())
    }

    /// Shows or hides the overlay.
    pub fn set_visible(&self, visible: bool) -> Result<OverlayStatus> {
        if visible {
            self.show_now(false)?;
        } else {
            self.hide_now()?;
        }
        Ok(self.status())
    }

    /// Switches between interactive and passive. A shortcut that makes it
    /// interactive gives it the keyboard (the page focuses the note);
    /// passive always gives the keyboard back.
    pub fn set_mode(&self, mode: Mode, origin: Origin) -> Result<OverlayStatus> {
        let focus = (mode == Mode::Interactive && origin == Origin::Shortcut)
            .then_some(FocusTarget::Content);
        self.switch_mode(mode, origin, focus)
    }

    /// [`set_mode`](Self::set_mode), telling the page what to focus.
    fn switch_mode(
        &self,
        mode: Mode,
        origin: Origin,
        focus: Option<FocusTarget>,
    ) -> Result<OverlayStatus> {
        {
            let _ops = lock_or(&self.window_ops);
            let visible = {
                let mut inner = self.lock();
                if !inner.running {
                    return Err(feature_off());
                }
                inner.mode = mode;
                if let Some(target) = focus {
                    inner.focus_seq += 1;
                    inner.focus_target = target;
                }
                inner.visible
            };
            if let Some(host) = self.host.get()
                && host.exists()
            {
                host.set_mode(mode)
                    .map_err(|e| window_error("switched", e))?;
                match mode {
                    Mode::Interactive if origin == Origin::Shortcut && visible => {
                        host.take_focus().map_err(|e| window_error("focused", e))?;
                    }
                    Mode::Passive => host.give_focus_back(),
                    Mode::Interactive => {}
                }
            }
        }
        self.touch();
        Ok(self.status())
    }

    /// Collapses the overlay to its title bar, or expands it.
    pub fn set_collapsed(&self, collapsed: bool) -> Result<OverlayStatus> {
        {
            let _ops = lock_or(&self.window_ops);
            let height = {
                let mut track = lock_or(&self.track);
                track.collapsed = collapsed;
                track.latest.map(|s| s.height)
            };
            self.lock().collapsed = collapsed;
            if let Some(host) = self.host.get()
                && host.exists()
            {
                let expanded = height
                    .or_else(|| host.bounds().map(|(r, _)| r.height))
                    .unwrap_or(MIN_SIZE.1);
                host.set_collapsed(collapsed, expanded)
                    .map_err(|e| window_error("resized", e))?;
            }
        }
        self.persist_state();
        self.touch();
        Ok(self.status())
    }

    /// The background's opacity, from the overlay's own control; saved
    /// in the settings.
    pub fn set_opacity(&self, opacity: u8) -> Result<OverlayStatus> {
        let (lo, hi) = OPACITY_RANGE;
        if !(lo..=hi).contains(&opacity) {
            return Err(invalid(format!("The opacity must be from {lo} to {hi} %")));
        }
        let mut settings = self.lock().settings.clone();
        settings.opacity = opacity;
        self.store_settings(&settings)?;
        self.lock().settings.opacity = opacity;
        self.touch();
        Ok(self.status())
    }

    /// The overlay asked for its settings: the dashboard opens them.
    pub fn request_settings(&self) {
        self.lock().settings_seq += 1;
        self.touch();
        if let Some(host) = self.host.get() {
            host.show_dashboard();
        }
    }

    /// Opens `url` from note `id` in the browser, if the note holds it.
    pub fn open_link(&self, id: &str, url: &str) -> Result<()> {
        let note = self.with_store(|s| s.note(id))?.ok_or_else(not_found)?;
        if !model::link_allowed(&note.body, url) {
            return Err(invalid(
                "Only web addresses written in the note can be opened",
            ));
        }
        self.host()?
            .open_link(url)
            .map_err(|e| OverlayError::new(ErrorKind::Window, e))
    }

    /// The window must be at `p` once shown ([`Expected`]). Only on X11:
    /// Windows shows a window where it was and reports where it is.
    fn expect_at(&self, p: Placement) {
        if cfg!(target_os = "linux") {
            *lock_or(&self.expect) = Some(Expected {
                x: p.x,
                y: p.y,
                at: Instant::now(),
                corrected: false,
            });
        }
    }

    /// Ages the expectation, as if `by` had passed since the show.
    #[cfg(test)]
    fn age_expectation(&self, by: Duration) {
        if let Some(e) = lock_or(&self.expect).as_mut()
            && let Some(at) = e.at.checked_sub(by)
        {
            e.at = at;
        }
    }

    // ------------------------------------------- window events (loop)

    /// The window moved or was resized (from the event loop: only the
    /// tracking and expectation locks). A collapsed window keeps its
    /// expanded height. Right after a show, a position elsewhere than
    /// asked is not remembered ([`Expected`]); for the first one this
    /// returns where to put the window back (the caller moves it).
    pub fn window_changed(&self, bounds: Rect, scale: f64) -> Option<(i32, i32)> {
        if bounds.width == 0 || bounds.height == 0 || !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        {
            let mut expect = lock_or(&self.expect);
            if let Some(e) = expect.as_mut() {
                if e.at.elapsed() > PLACED_WITHIN {
                    *expect = None;
                } else if (bounds.x, bounds.y) != (e.x, e.y) {
                    if e.corrected {
                        return None;
                    }
                    e.corrected = true;
                    return Some((e.x, e.y));
                }
            }
        }
        let mut track = lock_or(&self.track);
        let height = match (track.collapsed, track.latest) {
            (true, Some(previous)) => {
                let h = f64::from(previous.height) * scale / previous.scale;
                if h.is_finite() && h >= 1.0 {
                    h.round().min(f64::from(u32::MAX)) as u32
                } else {
                    previous.height
                }
            }
            _ => bounds.height,
        };
        track.latest = Some(Saved::new(Rect { height, ..bounds }, false, scale));
        None
    }

    /// The window was destroyed (by `stop`, or by the system): the next
    /// show creates it again.
    pub fn window_destroyed(&self) {
        let mut inner = self.lock();
        if inner.visible {
            inner.visible = false;
            drop(inner);
            self.touch();
        }
    }

    /// Saves the state document if it changed: the note shown, collapsed,
    /// the place. On hide, focus loss, stop and exit; never per pixel.
    pub fn persist_state(&self) {
        let latest = lock_or(&self.track).latest;
        let (text, written) = {
            let inner = self.lock();
            let state = OverlayState {
                collapsed: inner.collapsed,
                active_note: inner.active.clone(),
                placement: latest,
                ..OverlayState::new()
            };
            (
                serde_json::to_string(&state).expect("serializes"),
                inner.state_written.clone(),
            )
        };
        if written.as_deref() == Some(text.as_str()) {
            return;
        }
        match self.with_store(|s| s.save_document(STATE_DOC, &text)) {
            Ok(()) => self.lock().state_written = Some(text),
            Err(e) => eprintln!("sotyware: cannot save the overlay's state: {}", e.message),
        }
    }

    // ---------------------------------------------------------- notes

    /// Every note in display order, with their text.
    pub fn list_notes(&self) -> Result<NotesList> {
        let mut notes = self.with_store(|s| s.notes())?;
        let mut inner = self.lock();
        // What the database holds is what the overlay works with.
        inner.notes = notes.iter().map(Note::meta).collect();
        let order = model::display_order(&inner.notes);
        notes.sort_by_key(|n| order.iter().position(|id| *id == n.id));
        if !inner
            .active
            .as_ref()
            .is_some_and(|a| order.iter().any(|id| id == a))
        {
            inner.active = order.first().cloned();
        }
        Ok(NotesList {
            notes,
            active_note: inner.active.clone(),
        })
    }

    /// A new note at the end of the list, shown at once. From a shortcut
    /// the page is told to read the notes again.
    pub fn create_note(&self, title: Option<String>, origin: Origin) -> Result<Note> {
        let title = match title {
            Some(t) if !t.trim().is_empty() => model::clean_title(&t).map_err(invalid)?,
            _ => DEFAULT_TITLE.to_owned(),
        };
        let mut inner = self.lock();
        if inner.notes.len() >= MAX_NOTES {
            return Err(OverlayError::new(
                ErrorKind::Limit,
                format!("There can be at most {MAX_NOTES} notes; delete one first"),
            ));
        }
        let title = model::unique_title(&title, inner.notes.iter().map(|n| n.title.as_str()));
        let now = now_ms();
        let note = Note {
            id: new_id(),
            title,
            body: inner.settings.new_note.body().to_owned(),
            pinned: false,
            position: model::end_position(&inner.notes),
            created_ms: now,
            updated_ms: now,
            revision: 1,
        };
        self.with_store(|s| s.insert_note(&note, &[]))?;
        inner.notes.push(note.meta());
        inner.active = Some(note.id.clone());
        if origin == Origin::Shortcut {
            // The page reads the notes again and shows this one.
            inner.notes_seq += 1;
            inner.active_seq += 1;
        }
        drop(inner);
        self.touch();
        Ok(note)
    }

    /// Saves a note's title and text (the page's autosave). `revision` is
    /// the one the page edited; another one means the note changed
    /// meanwhile, and nothing is saved.
    pub fn save_note(&self, id: &str, title: &str, body: &str, revision: u64) -> Result<SavedNote> {
        let title = model::clean_title(title).map_err(invalid)?;
        let body = model::clean_body(body).map_err(invalid)?;
        let mut inner = self.lock();
        let Some(i) = inner.notes.iter().position(|n| n.id == id) else {
            return Err(not_found());
        };
        let current = inner.notes[i].revision;
        if current != revision {
            return Err(OverlayError::new(
                ErrorKind::Conflict,
                format!("this note changed meanwhile (revision {current})"),
            ));
        }
        let next = current + 1;
        let now = now_ms();
        if !self.with_store(|s| s.update_content(id, &title, &body, now, next))? {
            inner.notes.remove(i);
            return Err(not_found());
        }
        let meta = &mut inner.notes[i];
        meta.title = title;
        meta.revision = next;
        Ok(SavedNote {
            revision: next,
            updated_ms: now,
        })
    }

    /// Deletes a note; the note shown becomes the one after it (else the
    /// one before).
    pub fn delete_note(&self, id: &str) -> Result<Deleted> {
        let mut inner = self.lock();
        let order = model::display_order(&inner.notes);
        let Some(at) = order.iter().position(|x| x == id) else {
            return Err(not_found());
        };
        self.with_store(|s| s.delete_note(id))?;
        inner.notes.retain(|n| n.id != id);
        if inner.active.as_deref() == Some(id) || inner.active.is_none() {
            inner.active = order
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|p| order.get(p)))
                .cloned();
        }
        let active = inner.active.clone();
        drop(inner);
        self.touch();
        Ok(Deleted {
            active_note: active,
        })
    }

    /// A copy of a note right behind it, shown at once.
    pub fn duplicate_note(&self, id: &str) -> Result<Duplicated> {
        let original = self.with_store(|s| s.note(id))?.ok_or_else(not_found)?;
        let mut inner = self.lock();
        if inner.notes.len() >= MAX_NOTES {
            return Err(OverlayError::new(
                ErrorKind::Limit,
                format!("There can be at most {MAX_NOTES} notes; delete one first"),
            ));
        }
        let mut notes = inner.notes.clone();
        let (position, pinned, moved) =
            model::insert_after(&mut notes, id).ok_or_else(not_found)?;
        let now = now_ms();
        let note = Note {
            id: new_id(),
            title: model::unique_title(&original.title, notes.iter().map(|n| n.title.as_str())),
            body: original.body,
            pinned,
            position,
            created_ms: now,
            updated_ms: now,
            revision: 1,
        };
        self.with_store(|s| s.insert_note(&note, &moved))?;
        notes.push(note.meta());
        inner.notes = notes;
        inner.active = Some(note.id.clone());
        let order = model::orders(&inner.notes);
        drop(inner);
        self.touch();
        Ok(Duplicated { note, order })
    }

    /// Pins or unpins a note; every note's place afterwards.
    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<Vec<NoteOrder>> {
        self.reorder(|notes| model::set_pinned(notes, id, pinned))
    }

    /// Moves a note to `index` of the display order (inside its group);
    /// every note's place afterwards.
    pub fn move_note(&self, id: &str, index: usize) -> Result<Vec<NoteOrder>> {
        self.reorder(|notes| model::move_note(notes, id, index))
    }

    fn reorder(
        &self,
        f: impl FnOnce(&mut [NoteMeta]) -> Option<Vec<NoteOrder>>,
    ) -> Result<Vec<NoteOrder>> {
        let mut inner = self.lock();
        let mut notes = inner.notes.clone();
        let changed = f(&mut notes).ok_or_else(not_found)?;
        if !changed.is_empty() {
            self.with_store(|s| s.update_order(&changed))?;
        }
        inner.notes = notes;
        Ok(model::orders(&inner.notes))
    }

    /// The page shows note `id` now (remembered for the next start and as
    /// the start of the next/previous shortcuts).
    pub fn select_note(&self, id: &str) -> Result<()> {
        let mut inner = self.lock();
        if !inner.notes.iter().any(|n| n.id == id) {
            return Err(not_found());
        }
        inner.active = Some(id.to_owned());
        Ok(())
    }

    // ------------------------------------------------------- settings

    fn store_settings(&self, settings: &OverlaySettings) -> Result<()> {
        let text = serde_json::to_string(settings).expect("serializes");
        self.with_store(|s| s.save_document(SETTINGS_DOC, &text))
    }

    pub fn config(&self) -> OverlayConfig {
        let inner = self.lock();
        OverlayConfig {
            settings: inner.settings.clone(),
            suggestions: OverlayAction::ALL
                .iter()
                .filter_map(|&a| Chord::parse(a.suggestion()).ok().map(|c| (a, c)))
                .collect(),
            keys: key_views(inner.backend.as_ref()),
            limits: OverlayLimits {
                min_width: MIN_SIZE.0,
                min_height: MIN_SIZE.1,
                max_width: MAX_SIZE.0,
                max_height: MAX_SIZE.1,
                min_opacity: OPACITY_RANGE.0,
                max_opacity: OPACITY_RANGE.1,
                max_notes: MAX_NOTES,
                max_title_chars: model::MAX_TITLE_CHARS,
                max_body_chars: model::MAX_BODY_CHARS,
            },
        }
    }

    /// Validates, stores and applies new settings: the shortcuts are
    /// registered again, and an overlay that is open moves to its new
    /// default place when the size or the corner changed.
    pub fn save_settings(&self, settings: OverlaySettings) -> Result<OverlaySettings> {
        let settings = settings.validated().map_err(invalid)?;
        self.store_settings(&settings)?;
        let moved = {
            let mut inner = self.lock();
            let old = std::mem::replace(&mut inner.settings, settings.clone());
            self.sync_hotkeys(&mut inner);
            (old.width, old.height, old.anchor)
                != (settings.width, settings.height, settings.anchor)
        };
        if moved {
            self.replace_window(&settings);
        }
        self.touch();
        Ok(settings)
    }

    /// Puts an open window at the default place of `settings`, expanded.
    fn replace_window(&self, settings: &OverlaySettings) {
        let _ops = lock_or(&self.window_ops);
        let Some(host) = self.host.get().filter(|h| h.exists()) else {
            return;
        };
        let geometry = host.geometry();
        if let Some(p) = self.default_placement(settings, &geometry) {
            let collapsed = self.lock().collapsed;
            match host.place(p, collapsed) {
                Ok(()) => {
                    let scale = placement::screen_at(
                        &geometry.screens,
                        Some((f64::from(p.x), f64::from(p.y))),
                        geometry.primary,
                    )
                    .map_or(1.0, |s| s.scale);
                    self.remember(p, scale);
                }
                Err(e) => eprintln!("sotyware: cannot place the overlay: {e}"),
            }
        }
    }

    /// Back to the default settings (the notes stay): the shortcuts are
    /// the suggested ones again, the remembered place is forgotten and an
    /// open overlay goes to the default place, expanded, in the default
    /// mode.
    pub fn reset(&self) -> Result<OverlaySettings> {
        self.with_store(|s| s.delete_document(SETTINGS_DOC))?;
        let settings = OverlaySettings::default();
        {
            let mut track = lock_or(&self.track);
            track.latest = None;
            track.collapsed = false;
        }
        let (mode, running) = {
            let mut inner = self.lock();
            inner.settings = settings.clone();
            inner.collapsed = false;
            if inner.running {
                inner.mode = settings.default_mode;
            }
            self.sync_hotkeys(&mut inner);
            (inner.mode, inner.running)
        };
        {
            let _ops = lock_or(&self.window_ops);
            if let Some(host) = self.host.get().filter(|h| h.exists()) {
                let expanded = host.bounds().map_or(MIN_SIZE.1, |(r, _)| r.height);
                if let Err(e) = host.set_collapsed(false, expanded) {
                    eprintln!("sotyware: cannot expand the overlay: {e}");
                }
                if running && let Err(e) = host.set_mode(mode) {
                    eprintln!("sotyware: cannot switch the overlay's mode: {e}");
                }
            }
        }
        self.replace_window(&settings);
        self.persist_state();
        self.touch();
        Ok(settings)
    }

    // --------------------------------------------------------- status

    pub fn status(&self) -> OverlayStatus {
        let inner = self.lock();
        OverlayStatus {
            seq: self.seq.load(Ordering::Relaxed),
            running: inner.running,
            visible: inner.visible,
            mode: inner.mode,
            collapsed: inner.collapsed,
            opacity: inner.settings.opacity,
            text_size: inner.settings.text_size,
            active_note: inner.active.clone(),
            active_seq: inner.active_seq,
            notes_seq: inner.notes_seq,
            focus_seq: inner.focus_seq,
            focus_target: inner.focus_target,
            settings_seq: inner.settings_seq,
            note_count: inner.notes.len(),
            hotkeys: self.hotkey_views(&inner),
            shortcuts_note: self
                .shortcuts_note
                .clone()
                .or_else(|| inner.backend_error.clone()),
            notice: inner.notice.clone(),
            storage_error: inner.storage_error.clone(),
        }
    }

    /// Sends the status on `sink` now and whenever it changes.
    pub fn subscribe(self: &Arc<Self>, sink: Arc<dyn StatusSink>) -> u64 {
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        sink.send(&self.status());
        lock_or(&self.subscribers).push((id, sink));
        let mut started = lock_or(&self.notifier);
        if !*started {
            *started = true;
            let weak = Arc::downgrade(self);
            tauri::async_runtime::spawn(async move {
                loop {
                    let notified = async {
                        let Some(this) = weak.upgrade() else { return };
                        this.changed.notified().await;
                    };
                    notified.await;
                    tokio::time::sleep(STATUS_EVERY).await;
                    let Some(this) = weak.upgrade() else { return };
                    let status = this.status();
                    lock_or(&this.subscribers).retain(|(_, s)| s.send(&status));
                }
            });
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        lock_or(&self.subscribers).retain(|(i, _)| *i != id);
    }
}

impl claims::Retry for NotesOverlay {
    /// The autoclicker gave a chord up: refused shortcuts are tried again.
    fn retry_shortcuts(&self) {
        let mut inner = self.lock();
        self.sync_hotkeys(&mut inner);
        drop(inner);
        self.touch();
    }
}

// ------------------------------------------------------------ commands

fn overlay<'a>(state: &'a State<'_, Arc<NotesOverlay>>) -> &'a Arc<NotesOverlay> {
    state.inner()
}

/// Runs `f` on a blocking thread: the database, and window calls that
/// wait for the event loop, never run on it.
async fn blocking<T: Send + 'static>(
    state: &State<'_, Arc<NotesOverlay>>,
    f: impl FnOnce(Arc<NotesOverlay>) -> Result<T> + Send + 'static,
) -> Result<T> {
    let o = Arc::clone(overlay(state));
    tokio::task::spawn_blocking(move || f(o))
        .await
        .unwrap_or_else(|e| {
            Err(OverlayError::new(
                ErrorKind::Storage,
                format!("the operation stopped unexpectedly: {e}"),
            ))
        })
}

/// Sends the status on `on_event` now and whenever it changes. Allowed
/// for both windows by `allow-watch-overlay`.
#[tauri::command(async)]
pub fn watch_overlay(state: State<'_, Arc<NotesOverlay>>, on_event: Channel<OverlayStatus>) -> u64 {
    overlay(&state).subscribe(Arc::new(on_event))
}

/// Stops a `watch_overlay`. Allowed by `allow-unwatch-overlay`.
#[tauri::command(async)]
pub fn unwatch_overlay(state: State<'_, Arc<NotesOverlay>>, subscription: u64) {
    overlay(&state).unsubscribe(subscription);
}

/// The settings, the suggested shortcuts, the keys and the limits.
/// Allowed for the dashboard by `allow-overlay-config`.
#[tauri::command(async)]
pub fn overlay_config(state: State<'_, Arc<NotesOverlay>>) -> OverlayConfig {
    overlay(&state).config()
}

/// Saves the settings. Allowed for the dashboard by
/// `allow-save-overlay-settings`.
#[tauri::command]
pub async fn save_overlay_settings(
    state: State<'_, Arc<NotesOverlay>>,
    settings: OverlaySettings,
) -> Result<OverlaySettings> {
    blocking(&state, move |o| o.save_settings(settings)).await
}

/// Back to the default settings; the notes stay. Allowed for the
/// dashboard by `allow-reset-overlay`.
#[tauri::command]
pub async fn reset_overlay(state: State<'_, Arc<NotesOverlay>>) -> Result<OverlaySettings> {
    blocking(&state, |o| o.reset()).await
}

/// Shows or hides the overlay. Allowed for both windows by
/// `allow-set-overlay-visible`.
#[tauri::command]
pub async fn set_overlay_visible(
    state: State<'_, Arc<NotesOverlay>>,
    visible: bool,
) -> Result<OverlayStatus> {
    blocking(&state, move |o| o.set_visible(visible)).await
}

/// Switches between interactive and passive. Allowed for both windows by
/// `allow-set-overlay-mode`.
#[tauri::command]
pub async fn set_overlay_mode(
    window: WebviewWindow,
    state: State<'_, Arc<NotesOverlay>>,
    mode: Mode,
) -> Result<OverlayStatus> {
    let origin = Origin::of(&window);
    blocking(&state, move |o| o.set_mode(mode, origin)).await
}

/// Collapses or expands the overlay. Allowed for the overlay by
/// `allow-set-overlay-collapsed`.
#[tauri::command]
pub async fn set_overlay_collapsed(
    state: State<'_, Arc<NotesOverlay>>,
    collapsed: bool,
) -> Result<OverlayStatus> {
    blocking(&state, move |o| o.set_collapsed(collapsed)).await
}

/// The background's opacity. Allowed for the overlay by
/// `allow-set-overlay-opacity`.
#[tauri::command]
pub async fn set_overlay_opacity(
    state: State<'_, Arc<NotesOverlay>>,
    opacity: u8,
) -> Result<OverlayStatus> {
    blocking(&state, move |o| o.set_opacity(opacity)).await
}

/// Shows the dashboard at Settings › Notes overlay. Allowed for the
/// overlay by `allow-open-overlay-settings`.
#[tauri::command]
pub async fn open_overlay_settings(state: State<'_, Arc<NotesOverlay>>) -> Result<()> {
    blocking(&state, |o| {
        o.request_settings();
        Ok(())
    })
    .await
}

/// Opens a web address written in note `id` in the browser. Allowed for
/// the overlay by `allow-open-note-link`.
#[tauri::command]
pub async fn open_note_link(
    state: State<'_, Arc<NotesOverlay>>,
    id: String,
    url: String,
) -> Result<()> {
    blocking(&state, move |o| o.open_link(&id, &url)).await
}

/// Every note with its text, in display order. Allowed for the overlay
/// by `allow-list-notes`.
#[tauri::command]
pub async fn list_notes(state: State<'_, Arc<NotesOverlay>>) -> Result<NotesList> {
    blocking(&state, |o| o.list_notes()).await
}

/// A new note. Allowed for the overlay by `allow-create-note`.
#[tauri::command]
pub async fn create_note(
    state: State<'_, Arc<NotesOverlay>>,
    title: Option<String>,
) -> Result<Note> {
    blocking(&state, move |o| o.create_note(title, Origin::Overlay)).await
}

/// Saves a note's title and text. Allowed for the overlay by
/// `allow-save-note`.
#[tauri::command]
pub async fn save_note(
    state: State<'_, Arc<NotesOverlay>>,
    id: String,
    title: String,
    body: String,
    revision: u64,
) -> Result<SavedNote> {
    blocking(&state, move |o| o.save_note(&id, &title, &body, revision)).await
}

/// Deletes a note (the page asked the user first). Allowed for the
/// overlay by `allow-delete-note`.
#[tauri::command]
pub async fn delete_note(state: State<'_, Arc<NotesOverlay>>, id: String) -> Result<Deleted> {
    blocking(&state, move |o| o.delete_note(&id)).await
}

/// A copy of a note. Allowed for the overlay by `allow-duplicate-note`.
#[tauri::command]
pub async fn duplicate_note(state: State<'_, Arc<NotesOverlay>>, id: String) -> Result<Duplicated> {
    blocking(&state, move |o| o.duplicate_note(&id)).await
}

/// Pins or unpins a note. Allowed for the overlay by
/// `allow-set-note-pinned`.
#[tauri::command]
pub async fn set_note_pinned(
    state: State<'_, Arc<NotesOverlay>>,
    id: String,
    pinned: bool,
) -> Result<Vec<NoteOrder>> {
    blocking(&state, move |o| o.set_pinned(&id, pinned)).await
}

/// Moves a note. Allowed for the overlay by `allow-move-note`.
#[tauri::command]
pub async fn move_note(
    state: State<'_, Arc<NotesOverlay>>,
    id: String,
    index: usize,
) -> Result<Vec<NoteOrder>> {
    blocking(&state, move |o| o.move_note(&id, index)).await
}

/// The note the page shows. Allowed for the overlay by
/// `allow-select-note`.
#[tauri::command]
pub async fn select_note(state: State<'_, Arc<NotesOverlay>>, id: String) -> Result<()> {
    blocking(&state, move |o| o.select_note(&id)).await
}

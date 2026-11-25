//! The notes overlay's service (ADR-052) over an in-memory store, a fake
//! window and the autoclicker's mock backend: notes, their order and
//! their persistence, the window's lifecycle, interactive and passive,
//! placement, and the shortcuts' registration and presses. No display,
//! no real shortcut.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::model::*;
use super::*;
use crate::automation::claims::{Claims, Owner};
use crate::automation::input::mock::MockBackend;
use crate::automation::input::{Backend, HotkeySink};
use crate::automation::keys::Chord;
use crate::automation::model::Binding;
use crate::features::StoreError;
use crate::window_state::{Placement, Rect, Screen};

// ------------------------------------------------------------ doubles

#[derive(Default)]
struct Data {
    notes: Vec<Note>,
    docs: BTreeMap<String, String>,
    writes: usize,
}

/// An in-memory `notes.sqlite3`, shared with the test; `fail` makes
/// every call fail.
#[derive(Clone, Default)]
struct MemoryStore {
    data: Arc<Mutex<Data>>,
    fail: Arc<AtomicBool>,
}

impl MemoryStore {
    fn check(&self) -> Result<(), StoreError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(StoreError::new("disk I/O error"))
        } else {
            Ok(())
        }
    }

    fn note(&self, id: &str) -> Option<Note> {
        self.data
            .lock()
            .unwrap()
            .notes
            .iter()
            .find(|n| n.id == id)
            .cloned()
    }

    fn doc(&self, name: &str) -> Option<String> {
        self.data.lock().unwrap().docs.get(name).cloned()
    }

    fn writes(&self) -> usize {
        self.data.lock().unwrap().writes
    }
}

impl NotesStore for MemoryStore {
    fn notes(&mut self) -> Result<Vec<Note>, StoreError> {
        self.check()?;
        let mut v = self.data.lock().unwrap().notes.clone();
        v.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
        Ok(v)
    }

    fn note(&mut self, id: &str) -> Result<Option<Note>, StoreError> {
        self.check()?;
        Ok(MemoryStore::note(self, id))
    }

    fn insert_note(&mut self, note: &Note, moved: &[NoteOrder]) -> Result<(), StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        if d.notes.iter().any(|n| n.id == note.id) {
            return Err(StoreError::new("UNIQUE constraint failed"));
        }
        for o in moved {
            if let Some(n) = d.notes.iter_mut().find(|n| n.id == o.id) {
                n.position = o.position;
                n.pinned = o.pinned;
            }
        }
        d.notes.push(note.clone());
        d.writes += 1;
        Ok(())
    }

    fn update_content(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        updated_ms: i64,
        revision: u64,
    ) -> Result<bool, StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        d.writes += 1;
        Ok(match d.notes.iter_mut().find(|n| n.id == id) {
            Some(n) => {
                n.title = title.into();
                n.body = body.into();
                n.updated_ms = updated_ms;
                n.revision = revision;
                true
            }
            None => false,
        })
    }

    fn update_order(&mut self, changed: &[NoteOrder]) -> Result<(), StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        d.writes += 1;
        for o in changed {
            if let Some(n) = d.notes.iter_mut().find(|n| n.id == o.id) {
                n.position = o.position;
                n.pinned = o.pinned;
            }
        }
        Ok(())
    }

    fn delete_note(&mut self, id: &str) -> Result<(), StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        d.writes += 1;
        d.notes.retain(|n| n.id != id);
        Ok(())
    }

    fn document(&mut self, name: &str) -> Result<Option<String>, StoreError> {
        self.check()?;
        Ok(self.doc(name))
    }

    fn save_document(&mut self, name: &str, data: &str) -> Result<(), StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        d.writes += 1;
        d.docs.insert(name.into(), data.into());
        Ok(())
    }

    fn delete_document(&mut self, name: &str) -> Result<(), StoreError> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        d.writes += 1;
        d.docs.remove(name);
        Ok(())
    }
}

/// What the fake window was asked, and its state.
#[derive(Default)]
struct HostState {
    calls: Mutex<Vec<String>>,
    exists: AtomicBool,
    visible: AtomicBool,
    focused: AtomicBool,
    bounds: Mutex<Option<(Rect, f64)>>,
    geometry: Mutex<Geometry>,
    fail_create: AtomicBool,
    opened: Mutex<Vec<String>>,
    /// Run once by the next `show`, as the event loop may report the
    /// window manager's placement while `show` runs.
    on_show: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl HostState {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    fn log(&self, s: impl Into<String>) {
        self.calls.lock().unwrap().push(s.into());
    }
}

struct FakeHost(Arc<HostState>);

fn placed(p: Placement) -> Rect {
    Rect {
        x: p.x,
        y: p.y,
        width: p.width,
        height: p.height,
    }
}

impl OverlayHost for FakeHost {
    fn geometry(&self) -> Geometry {
        self.0.geometry.lock().unwrap().clone()
    }

    fn exists(&self) -> bool {
        self.0.exists.load(Ordering::SeqCst)
    }

    fn create(&self, init: &WindowInit) -> std::result::Result<(), String> {
        if self.0.fail_create.load(Ordering::SeqCst) {
            return Err("WebView2 is not installed".into());
        }
        let p = init.placement;
        self.0.log(format!(
            "create {} {} {}x{} {:?}{}",
            p.x,
            p.y,
            p.width,
            p.height,
            init.mode,
            if init.collapsed { " collapsed" } else { "" }
        ));
        self.0.exists.store(true, Ordering::SeqCst);
        *self.0.bounds.lock().unwrap() = Some((placed(p), 1.0));
        Ok(())
    }

    fn bounds(&self) -> Option<(Rect, f64)> {
        *self.0.bounds.lock().unwrap()
    }

    fn place(&self, p: Placement, collapsed: bool) -> std::result::Result<(), String> {
        self.0.log(format!(
            "place {} {} {}x{}{}",
            p.x,
            p.y,
            p.width,
            p.height,
            if collapsed { " collapsed" } else { "" }
        ));
        *self.0.bounds.lock().unwrap() = Some((placed(p), 1.0));
        Ok(())
    }

    fn show(&self, mode: Mode, activate: bool) -> std::result::Result<(), String> {
        self.0.log(format!("show {mode:?} activate={activate}"));
        self.0.visible.store(true, Ordering::SeqCst);
        let during = self.0.on_show.lock().unwrap().take();
        if let Some(event) = during {
            event();
        }
        if activate {
            self.0.focused.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn hide(&self) -> std::result::Result<(), String> {
        self.0.log("hide");
        self.0.visible.store(false, Ordering::SeqCst);
        self.0.focused.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn destroy(&self) {
        self.0.log("destroy");
        self.0.exists.store(false, Ordering::SeqCst);
        self.0.visible.store(false, Ordering::SeqCst);
    }

    fn set_mode(&self, mode: Mode) -> std::result::Result<(), String> {
        self.0.log(format!("mode {mode:?}"));
        Ok(())
    }

    fn take_focus(&self) -> std::result::Result<(), String> {
        self.0.log("take focus");
        self.0.focused.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn give_focus_back(&self) {
        if self.0.focused.swap(false, Ordering::SeqCst) {
            self.0.log("give focus back");
        }
    }

    fn set_collapsed(&self, collapsed: bool, expanded: u32) -> std::result::Result<(), String> {
        self.0
            .log(format!("collapsed={collapsed} expanded={expanded}"));
        Ok(())
    }

    fn show_dashboard(&self) {
        self.0.log("show dashboard");
    }

    fn open_link(&self, url: &str) -> std::result::Result<(), String> {
        self.0.opened.lock().unwrap().push(url.into());
        Ok(())
    }
}

fn screen(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Screen {
    Screen {
        work: Rect {
            x,
            y,
            width,
            height,
        },
        scale,
    }
}

/// One 1920 × 1040 monitor at 100 %, the pointer in its middle.
fn one_monitor() -> Geometry {
    let s = screen(0, 0, 1920, 1040, 1.0);
    Geometry {
        screens: vec![s],
        primary: Some(s),
        cursor: Some((960.0, 520.0)),
    }
}

/// The user moves or resizes the overlay, a while after it was shown
/// (right after a show, a position elsewhere than asked is the window
/// manager's on X11 and is not remembered: `Expected`).
fn user_moves(o: &NotesOverlay, bounds: Rect, scale: f64) -> Option<(i32, i32)> {
    o.age_expectation(Duration::from_secs(2));
    o.window_changed(bounds, scale)
}

struct Rig {
    overlay: Arc<NotesOverlay>,
    store: MemoryStore,
    host: Arc<HostState>,
    backend: Arc<MockBackend>,
    sink: Arc<Mutex<Option<HotkeySink>>>,
}

impl Rig {
    fn new() -> Self {
        Self::with_store(MemoryStore::default())
    }

    fn with_store(store: MemoryStore) -> Self {
        Self::build(store, None)
    }

    fn build(store: MemoryStore, shortcuts_note: Option<String>) -> Self {
        let backend = MockBackend::new();
        let sink: Arc<Mutex<Option<HotkeySink>>> = Arc::default();
        let (b, s) = (Arc::clone(&backend), Arc::clone(&sink));
        let overlay = NotesOverlay::with(
            Ok(Box::new(store.clone())),
            shortcuts_note,
            Box::new(move |sink| {
                *s.lock().unwrap() = Some(sink);
                Ok(Arc::clone(&b) as Arc<dyn Backend>)
            }),
        );
        let host = Arc::new(HostState::default());
        *host.geometry.lock().unwrap() = one_monitor();
        overlay.attach(Box::new(FakeHost(Arc::clone(&host))));
        Self {
            overlay,
            store,
            host,
            backend,
            sink,
        }
    }

    /// Started by the user (after startup): the window shows at once.
    fn running(self) -> Self {
        self.overlay.startup_finished();
        self.overlay.start().unwrap();
        self
    }

    fn registered(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .backend
            .hotkeys
            .lock()
            .unwrap()
            .values()
            .map(|c| c.to_string())
            .collect();
        v.sort();
        v
    }

    /// The user presses `chord` while it is registered.
    fn press(&self, chord: &str) {
        let chord = Chord::parse(chord).unwrap();
        let id = self
            .backend
            .hotkeys
            .lock()
            .unwrap()
            .iter()
            .find(|(_, c)| **c == chord)
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("{chord} is not registered"));
        let sink = self.sink.lock().unwrap().clone().expect("a backend");
        sink(id);
    }

    fn status(&self) -> OverlayStatus {
        self.overlay.status()
    }
}

/// Waits (up to 5 s) for `f`; shortcuts arrive on their own thread.
fn eventually(f: impl Fn() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    f()
}

fn suggested() -> Vec<String> {
    let mut v: Vec<String> = OverlayAction::ALL
        .iter()
        .map(|a| a.suggestion().to_owned())
        .collect();
    v.sort();
    v
}

// --------------------------------------------------------------- notes

#[test]
fn notes_are_created_edited_duplicated_and_deleted() {
    let rig = Rig::new();
    let o = &rig.overlay;
    let a = o.create_note(None, Origin::Overlay).unwrap();
    assert_eq!(
        (a.title.as_str(), a.body.as_str(), a.revision),
        ("New note", "", 1)
    );
    let b = o.create_note(Some("Boss".into()), Origin::Overlay).unwrap();
    let c = o.create_note(Some("  ".into()), Origin::Overlay).unwrap();
    assert_eq!(
        c.title, "New note (2)",
        "blank titles get the default, made unique"
    );
    assert_eq!(o.status().active_note.as_deref(), Some(c.id.as_str()));

    let saved = o
        .save_note(&b.id, "Boss strategy", "- [ ] heal\n- [x] gear", 1)
        .unwrap();
    assert_eq!(saved.revision, 2);
    let stored = rig.store.note(&b.id).unwrap();
    assert_eq!(
        (stored.title.as_str(), stored.body.as_str(), stored.revision),
        ("Boss strategy", "- [ ] heal\n- [x] gear", 2)
    );
    // A save of an older revision is refused and changes nothing.
    let e = o.save_note(&b.id, "stale", "stale", 1).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert_eq!(rig.store.note(&b.id).unwrap().title, "Boss strategy");

    let d = o.duplicate_note(&b.id).unwrap();
    assert_eq!(d.note.title, "Boss strategy (2)");
    assert_eq!(d.note.body, "- [ ] heal\n- [x] gear");
    let ids: Vec<&str> = d.order.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            a.id.as_str(),
            b.id.as_str(),
            d.note.id.as_str(),
            c.id.as_str()
        ]
    );
    assert_eq!(o.status().active_note.as_deref(), Some(d.note.id.as_str()));

    // Deleting the note shown shows the one after it, else the one before.
    let gone = o.delete_note(&d.note.id).unwrap();
    assert_eq!(gone.active_note.as_deref(), Some(c.id.as_str()));
    assert!(rig.store.note(&d.note.id).is_none());
    let gone = o.delete_note(&c.id).unwrap();
    assert_eq!(gone.active_note.as_deref(), Some(b.id.as_str()));
    assert_eq!(o.delete_note(&c.id).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(
        o.save_note(&c.id, "x", "y", 1).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        o.duplicate_note(&c.id).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(o.status().note_count, 2);
}

#[test]
fn new_notes_start_from_the_settings_template() {
    let rig = Rig::new();
    let s = OverlaySettings {
        new_note: NewNoteTemplate::Checklist,
        ..OverlaySettings::default()
    };
    rig.overlay.save_settings(s).unwrap();
    let n = rig.overlay.create_note(None, Origin::Overlay).unwrap();
    assert_eq!(n.body, "- [ ] ");
}

#[test]
fn titles_and_text_are_checked_before_anything_is_stored() {
    let rig = Rig::new();
    let o = &rig.overlay;
    let n = o.create_note(None, Origin::Overlay).unwrap();
    let writes = rig.store.writes();
    let long = "x".repeat(MAX_TITLE_CHARS + 1);
    assert_eq!(
        o.save_note(&n.id, &long, "", 1).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        o.save_note(&n.id, "t", &"y".repeat(MAX_BODY_CHARS + 1), 1)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        o.create_note(Some(long), Origin::Overlay).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(rig.store.writes(), writes);
    // A title's line break becomes a space; Windows line ends become \n.
    o.save_note(&n.id, "a\nb", "1\r\n2", 1).unwrap();
    let stored = rig.store.note(&n.id).unwrap();
    assert_eq!(
        (stored.title.as_str(), stored.body.as_str()),
        ("a b", "1\n2")
    );
}

#[test]
fn there_are_at_most_max_notes() {
    let store = MemoryStore::default();
    for i in 0..MAX_NOTES {
        store.data.lock().unwrap().notes.push(Note {
            id: format!("n{i}"),
            title: format!("N{i}"),
            body: String::new(),
            pinned: false,
            position: i as i64,
            created_ms: 0,
            updated_ms: 0,
            revision: 1,
        });
    }
    let rig = Rig::with_store(store);
    let e = rig.overlay.create_note(None, Origin::Overlay).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert_eq!(
        rig.overlay.duplicate_note("n1").unwrap_err().kind,
        ErrorKind::Limit
    );
    rig.overlay.delete_note("n0").unwrap();
    assert!(rig.overlay.create_note(None, Origin::Overlay).is_ok());
}

#[test]
fn pinning_and_moving_change_the_order_in_the_database() {
    let rig = Rig::new();
    let o = &rig.overlay;
    let ids: Vec<String> = ["A", "B", "C"]
        .iter()
        .map(|t| {
            o.create_note(Some((*t).into()), Origin::Overlay)
                .unwrap()
                .id
        })
        .collect();
    let order = o.set_pinned(&ids[2], true).unwrap();
    assert_eq!(
        order
            .iter()
            .map(|x| (x.id.as_str(), x.pinned))
            .collect::<Vec<_>>(),
        [
            (ids[2].as_str(), true),
            (ids[0].as_str(), false),
            (ids[1].as_str(), false)
        ]
    );
    let order = o.move_note(&ids[1], 1).unwrap();
    assert_eq!(
        order.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
        [ids[2].as_str(), ids[1].as_str(), ids[0].as_str()]
    );
    // The database holds the same order: a new service reads it back.
    let again = Rig::with_store(rig.store.clone());
    let list = again.overlay.list_notes().unwrap();
    let titles: Vec<&str> = list.notes.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["C", "B", "A"]);
    assert!(list.notes[0].pinned);
    assert_eq!(
        o.move_note("nope", 0).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        o.set_pinned("nope", true).unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn selecting_a_note_is_remembered_without_telling_the_page() {
    let rig = Rig::new();
    let o = &rig.overlay;
    let a = o.create_note(None, Origin::Overlay).unwrap();
    o.create_note(None, Origin::Overlay).unwrap();
    let before = o.status().active_seq;
    o.select_note(&a.id).unwrap();
    let s = o.status();
    assert_eq!(s.active_note.as_deref(), Some(a.id.as_str()));
    assert_eq!(s.active_seq, before, "the page chose it; nothing to follow");
    assert_eq!(o.select_note("nope").unwrap_err().kind, ErrorKind::NotFound);
}

// --------------------------------------------------------- persistence

#[test]
fn notes_settings_and_state_survive_a_restart() {
    let rig = Rig::new().running();
    let o = &rig.overlay;
    let a = o
        .create_note(Some("Route".into()), Origin::Overlay)
        .unwrap();
    let b = o.create_note(Some("Loot".into()), Origin::Overlay).unwrap();
    o.save_note(&a.id, "Route", "1. bridge\n2. cave", 1)
        .unwrap();
    o.select_note(&a.id).unwrap();
    let mut s = OverlaySettings {
        opacity: 60,
        text_size: TextSize::Large,
        ..OverlaySettings::default()
    };
    s.shortcuts
        .insert(OverlayAction::NewNote, Binding::on("Ctrl+Alt+F9"));
    o.save_settings(s.clone()).unwrap();
    o.set_collapsed(true).unwrap();
    user_moves(
        o,
        Rect {
            x: 50,
            y: 60,
            width: 400,
            height: 38,
        },
        1.0,
    );
    o.shutdown();

    let again = Rig::with_store(rig.store.clone());
    let list = again.overlay.list_notes().unwrap();
    assert_eq!(list.notes.len(), 2);
    assert_eq!(list.active_note.as_deref(), Some(a.id.as_str()));
    assert_eq!(list.notes[0].body, "1. bridge\n2. cave");
    assert_eq!(list.notes[1].id, b.id);
    let st = again.status();
    assert_eq!(
        (st.opacity, st.text_size, st.collapsed),
        (60, TextSize::Large, true)
    );
    assert_eq!(again.overlay.config().settings, s.validated().unwrap());
    // The window opens where it was, at its expanded height.
    let again = again.running();
    let created = again.host.calls();
    assert!(
        created
            .iter()
            .any(|c| c.starts_with("create 50 60 400x") && c.ends_with("collapsed")),
        "{created:?}"
    );
}

#[test]
fn the_state_is_written_only_when_it_changed() {
    let rig = Rig::new();
    let o = &rig.overlay;
    o.create_note(None, Origin::Overlay).unwrap();
    o.persist_state();
    let writes = rig.store.writes();
    o.persist_state();
    o.persist_state();
    assert_eq!(rig.store.writes(), writes, "unchanged: no write");
    user_moves(
        o,
        Rect {
            x: 1,
            y: 2,
            width: 300,
            height: 300,
        },
        1.0,
    );
    // Moving is tracked in memory only.
    assert_eq!(rig.store.writes(), writes);
    o.persist_state();
    assert_eq!(rig.store.writes(), writes + 1);
}

#[test]
fn settings_a_newer_sotyware_wrote_are_used_as_defaults_and_left_alone() {
    let store = MemoryStore::default();
    let newer = r#"{"formatVersion":2,"opacity":50}"#;
    store
        .data
        .lock()
        .unwrap()
        .docs
        .insert("settings".into(), newer.into());
    store
        .data
        .lock()
        .unwrap()
        .docs
        .insert("state".into(), "{garbled".into());
    let rig = Rig::with_store(store.clone());
    assert_eq!(rig.overlay.config().settings, OverlaySettings::default());
    rig.overlay.create_note(None, Origin::Overlay).unwrap();
    assert_eq!(
        store.doc("settings").as_deref(),
        Some(newer),
        "not overwritten"
    );
    // Saving is the user's choice: then it is replaced.
    rig.overlay
        .save_settings(OverlaySettings::default())
        .unwrap();
    assert_ne!(store.doc("settings").as_deref(), Some(newer));
}

#[test]
fn an_unusable_database_is_reported_and_nothing_pretends_to_save() {
    let rig = Rig::build(MemoryStore::default(), None);
    rig.store.fail.store(true, Ordering::SeqCst);
    let e = rig.overlay.create_note(None, Origin::Overlay).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Storage);
    assert!(e.message.contains("disk I/O error"), "{}", e.message);
    assert_eq!(
        rig.overlay.list_notes().unwrap_err().kind,
        ErrorKind::Storage
    );
    assert_eq!(
        rig.overlay
            .save_settings(OverlaySettings::default())
            .unwrap_err()
            .kind,
        ErrorKind::Storage
    );
    // A database that cannot be opened at all.
    let broken = NotesOverlay::with(
        Err(StoreError::new("file is not a database")),
        None,
        Box::new(|_| Err("no backend".into())),
    );
    assert_eq!(
        broken.status().storage_error.as_deref(),
        Some("file is not a database")
    );
    let e = broken.create_note(None, Origin::Overlay).unwrap_err();
    assert!(
        e.message.contains("file is not a database"),
        "{}",
        e.message
    );
}

#[test]
fn settings_are_validated_and_a_reset_keeps_the_notes() {
    let rig = Rig::new().running();
    let o = &rig.overlay;
    let n = o.create_note(None, Origin::Overlay).unwrap();
    let bad = OverlaySettings {
        opacity: 5,
        ..OverlaySettings::default()
    };
    assert_eq!(
        o.save_settings(bad).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(rig.store.doc("settings"), None, "nothing stored");
    let mut s = OverlaySettings {
        opacity: 40,
        ..OverlaySettings::default()
    };
    s.shortcuts
        .insert(OverlayAction::ToggleVisible, Binding::on("Ctrl+Alt+F11"));
    o.save_settings(s).unwrap();
    assert!(rig.registered().contains(&"Ctrl+Alt+F11".to_owned()));
    o.set_collapsed(true).unwrap();
    let reset = o.reset().unwrap();
    assert_eq!(reset, OverlaySettings::default());
    assert_eq!(rig.store.doc("settings"), None);
    assert_eq!(
        rig.registered(),
        suggested(),
        "the suggested shortcuts again"
    );
    assert!(!o.status().collapsed);
    assert!(rig.store.note(&n.id).is_some(), "the notes stay");
    let calls = rig.host.calls();
    assert!(
        calls.iter().any(|c| c.starts_with("collapsed=false")),
        "{calls:?}"
    );
    assert!(calls.iter().any(|c| c.starts_with("place ")), "{calls:?}");
}

#[test]
fn the_opacity_from_the_overlay_is_saved_in_the_settings() {
    let rig = Rig::new().running();
    rig.overlay.set_opacity(45).unwrap();
    assert_eq!(rig.status().opacity, 45);
    let again = Rig::with_store(rig.store.clone());
    assert_eq!(again.overlay.config().settings.opacity, 45);
    assert_eq!(
        rig.overlay.set_opacity(10).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        rig.overlay.set_opacity(101).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

// ----------------------------------------------------- window lifecycle

#[test]
fn turned_on_by_the_user_it_shows_without_taking_the_keyboard() {
    let rig = Rig::new().running();
    let calls = rig.host.calls();
    // Top right of the monitor, 24 px in, at the default size.
    assert_eq!(
        calls,
        [
            "create 1536 24 360x440 Interactive",
            "show Interactive activate=false"
        ]
    );
    let s = rig.status();
    assert!(s.running && s.visible);
    assert!(!rig.host.focused.load(Ordering::SeqCst));
}

#[test]
fn at_startup_it_shows_only_when_the_settings_ask() {
    // Started by startup (the feature was on): nothing shows.
    let rig = Rig::new();
    rig.overlay.start().unwrap();
    rig.overlay.startup_finished();
    assert!(rig.host.calls().is_empty());
    assert!(!rig.status().visible);

    let store = MemoryStore::default();
    let s = OverlaySettings {
        show_at_startup: true,
        default_mode: Mode::Passive,
        ..OverlaySettings::default()
    };
    store
        .data
        .lock()
        .unwrap()
        .docs
        .insert("settings".into(), serde_json::to_string(&s).unwrap());
    let rig = Rig::with_store(store);
    rig.overlay.start().unwrap();
    assert!(rig.host.calls().is_empty(), "not before setup has finished");
    rig.overlay.startup_finished();
    // On a worker thread: setup runs on the event loop's.
    assert!(eventually(|| rig.status().visible));
    assert_eq!(
        rig.host.calls(),
        [
            "create 1536 24 360x440 Passive",
            "show Passive activate=false"
        ]
    );
    assert_eq!(rig.status().mode, Mode::Passive);
}

#[test]
fn quitting_leaves_the_window_alone_and_still_unregisters_and_remembers() {
    let rig = Rig::new().running();
    user_moves(
        &rig.overlay,
        Rect {
            x: 7,
            y: 8,
            width: 300,
            height: 300,
        },
        1.0,
    );
    rig.host.clear();
    // As release_resources does: exiting first, then the feature stops.
    rig.overlay.exiting();
    rig.overlay.stop();
    rig.overlay.shutdown();
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
    assert!(rig.registered().is_empty());
    let state = rig.store.doc("state").unwrap();
    assert!(state.contains(r#""x":7"#), "{state}");
}

#[test]
fn hiding_keeps_the_window_and_stopping_destroys_it() {
    let rig = Rig::new().running();
    rig.host.focused.store(true, Ordering::SeqCst);
    rig.host.clear();
    rig.overlay.set_visible(false).unwrap();
    assert_eq!(rig.host.calls(), ["give focus back", "hide"]);
    assert!(!rig.status().visible);
    rig.overlay.set_visible(true).unwrap();
    // Put back where it was (a window manager may place a window it maps
    // again anew), then shown: not created again.
    assert_eq!(
        rig.host.calls()[2..],
        [
            "place 1536 24 360x440".to_owned(),
            "show Interactive activate=false".to_owned(),
            "place 1536 24 360x440".to_owned()
        ]
    );
    rig.host.clear();
    rig.overlay.stop();
    assert_eq!(rig.host.calls(), ["destroy"]);
    let s = rig.status();
    assert!(!s.running && !s.visible);
    // Off, it cannot be shown nor switched.
    assert_eq!(
        rig.overlay.set_visible(true).unwrap_err().kind,
        ErrorKind::FeatureOff
    );
    assert_eq!(
        rig.overlay
            .set_mode(Mode::Passive, Origin::Dashboard)
            .unwrap_err()
            .kind,
        ErrorKind::FeatureOff
    );
}

#[test]
fn a_window_that_could_not_be_created_is_reported_and_tried_again() {
    let rig = Rig::new();
    rig.host.fail_create.store(true, Ordering::SeqCst);
    let rig = rig.running();
    let s = rig.status();
    assert!(s.running && !s.visible);
    assert!(
        s.notice.as_deref().is_some_and(|n| n.contains("WebView2")),
        "{:?}",
        s.notice
    );
    rig.host.fail_create.store(false, Ordering::SeqCst);
    rig.overlay.set_visible(true).unwrap();
    assert!(rig.status().visible);
}

#[test]
fn a_window_destroyed_by_the_system_is_created_again_when_shown() {
    let rig = Rig::new().running();
    rig.host.exists.store(false, Ordering::SeqCst);
    rig.overlay.window_destroyed();
    assert!(!rig.status().visible);
    rig.host.clear();
    rig.overlay.set_visible(true).unwrap();
    assert!(
        rig.host.calls()[0].starts_with("create "),
        "{:?}",
        rig.host.calls()
    );
}

#[test]
fn shown_again_it_is_put_back_where_the_user_left_it() {
    let rig = Rig::new().running();
    user_moves(
        &rig.overlay,
        Rect {
            x: 300,
            y: 400,
            width: 500,
            height: 300,
        },
        1.0,
    );
    rig.overlay.set_visible(false).unwrap();
    rig.host.clear();
    rig.overlay.set_visible(true).unwrap();
    assert_eq!(
        rig.host.calls(),
        [
            "place 300 400 500x300",
            "show Interactive activate=false",
            "place 300 400 500x300"
        ]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn right_after_a_show_a_position_elsewhere_is_put_right_once_and_never_remembered() {
    let at = |x: i32, y: i32| Rect {
        x,
        y,
        width: 360,
        height: 440,
    };
    for again in [false, true] {
        // The first show, which creates the window, and a show again.
        let rig = Rig::new().running();
        if again {
            rig.overlay.set_visible(false).unwrap();
            rig.overlay.set_visible(true).unwrap();
        }
        // The window manager mapped it at 0, 0, or tao reported 0, 0
        // before the window manager placed it at its corner (found by the
        // Linux desktop test): put back once, never remembered, also when
        // the same report comes again (moved, then resized).
        assert_eq!(rig.overlay.window_changed(at(0, 0), 1.0), Some((1536, 24)));
        assert_eq!(rig.overlay.window_changed(at(0, 0), 1.0), None);
        assert_eq!(rig.overlay.window_changed(at(1536, 24), 1.0), None);
        assert_eq!(rig.overlay.window_changed(at(5, 5), 1.0), None);
        rig.overlay.shutdown();
        let state = rig.store.doc("state").unwrap();
        assert!(state.contains(r#""x":1536,"y":24"#), "{state}");
    }
    // Reported at its place first and elsewhere after (a reparenting
    // window manager): still put right.
    let rig = Rig::new().running();
    assert_eq!(rig.overlay.window_changed(at(1536, 24), 1.0), None);
    assert_eq!(rig.overlay.window_changed(at(0, 0), 1.0), Some((1536, 24)));
    // Later the user's moves are remembered.
    assert_eq!(user_moves(&rig.overlay, at(700, 300), 1.0), None);
    rig.overlay.shutdown();
    let state = rig.store.doc("state").unwrap();
    assert!(state.contains(r#""x":700,"y":300"#), "{state}");
}

#[cfg(target_os = "linux")]
#[test]
fn the_window_managers_placement_reported_while_showing_is_put_right_too() {
    // Found by the Linux desktop test under load: the event loop reported
    // the window manager's 0, 0 before `show` returned.
    let rig = Rig::new().running();
    rig.overlay.set_visible(false).unwrap();
    // What `window_changed` answered while `show` ran.
    type Answers = Mutex<Vec<Option<(i32, i32)>>>;
    let seen: Arc<Answers> = Arc::default();
    let (overlay, got) = (Arc::downgrade(&rig.overlay), Arc::clone(&seen));
    *rig.host.on_show.lock().unwrap() = Some(Box::new(move || {
        if let Some(o) = overlay.upgrade() {
            let mapped = Rect {
                x: 0,
                y: 0,
                width: 360,
                height: 440,
            };
            got.lock().unwrap().push(o.window_changed(mapped, 1.0));
        }
    }));
    rig.overlay.set_visible(true).unwrap();
    assert_eq!(*seen.lock().unwrap(), [Some((1536, 24))]);
    rig.overlay.shutdown();
    let state = rig.store.doc("state").unwrap();
    assert!(state.contains(r#""x":1536"#), "{state}");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn elsewhere_a_move_after_showing_is_always_the_users() {
    let rig = Rig::new().running();
    rig.overlay.set_visible(false).unwrap();
    rig.overlay.set_visible(true).unwrap();
    let moved = Rect {
        x: 0,
        y: 0,
        width: 360,
        height: 440,
    };
    assert_eq!(rig.overlay.window_changed(moved, 1.0), None);
}

#[test]
fn shown_again_after_its_monitor_was_unplugged_it_moves_onto_one() {
    let rig = Rig::new().running();
    // The user had put it on a monitor above the primary, unplugged while
    // the overlay was hidden.
    user_moves(
        &rig.overlay,
        Rect {
            x: 500,
            y: -1000,
            width: 360,
            height: 440,
        },
        1.0,
    );
    rig.overlay.set_visible(false).unwrap();
    rig.host.clear();
    rig.overlay.set_visible(true).unwrap();
    assert_eq!(
        rig.host.calls(),
        [
            "place 1536 24 360x440",
            "show Interactive activate=false",
            "place 1536 24 360x440"
        ]
    );
}

// ------------------------------------------------- interactive/passive

#[test]
fn passive_gives_the_keyboard_back_and_interactive_by_shortcut_takes_it() {
    let rig = Rig::new().running();
    rig.host.focused.store(true, Ordering::SeqCst);
    rig.host.clear();
    let s = rig
        .overlay
        .set_mode(Mode::Passive, Origin::Overlay)
        .unwrap();
    assert_eq!(s.mode, Mode::Passive);
    assert_eq!(rig.host.calls(), ["mode Passive", "give focus back"]);
    // From the dashboard: interactive, but the keyboard stays where it is.
    rig.host.clear();
    rig.overlay
        .set_mode(Mode::Interactive, Origin::Dashboard)
        .unwrap();
    assert_eq!(rig.host.calls(), ["mode Interactive"]);
    // By shortcut: it takes the keyboard, and the page focuses the note.
    let before = rig.status().focus_seq;
    rig.overlay
        .set_mode(Mode::Passive, Origin::Shortcut)
        .unwrap();
    rig.host.clear();
    let s = rig
        .overlay
        .set_mode(Mode::Interactive, Origin::Shortcut)
        .unwrap();
    assert_eq!(rig.host.calls(), ["mode Interactive", "take focus"]);
    assert_eq!(
        (s.focus_seq, s.focus_target),
        (before + 1, FocusTarget::Content)
    );
}

#[test]
fn the_mode_shortcut_always_brings_back_an_overlay_that_ignores_the_mouse() {
    let rig = Rig::new().running();
    rig.overlay
        .set_mode(Mode::Passive, Origin::Overlay)
        .unwrap();
    rig.overlay.set_visible(false).unwrap();
    rig.host.clear();
    // Hidden and passive: the mode shortcut shows it, interactive, with
    // the keyboard.
    rig.press("Ctrl+Shift+F2");
    assert!(eventually(
        || rig.status().mode == Mode::Interactive && rig.status().visible
    ));
    assert_eq!(
        rig.host.calls(),
        [
            "place 1536 24 360x440",
            "show Passive activate=false",
            "place 1536 24 360x440",
            "mode Interactive",
            "take focus"
        ]
    );
    // Visible: it switches back and forth.
    rig.press("Ctrl+Shift+F2");
    assert!(eventually(|| rig.status().mode == Mode::Passive));
    rig.press("Ctrl+Shift+F2");
    assert!(eventually(|| rig.status().mode == Mode::Interactive));
}

#[test]
fn the_mode_at_start_is_the_default_one() {
    let rig = Rig::new();
    let s = OverlaySettings {
        default_mode: Mode::Passive,
        ..OverlaySettings::default()
    };
    rig.overlay.save_settings(s).unwrap();
    let rig = rig.running();
    assert_eq!(rig.status().mode, Mode::Passive);
    rig.overlay
        .set_mode(Mode::Interactive, Origin::Dashboard)
        .unwrap();
    rig.overlay.stop();
    rig.overlay.start().unwrap();
    assert_eq!(rig.status().mode, Mode::Passive, "every start begins in it");
}

// ------------------------------------------------------------ placement

#[test]
fn a_new_overlay_opens_on_the_monitor_with_the_pointer() {
    let rig = Rig::new();
    let primary = screen(0, 0, 1920, 1040, 1.0);
    let upper = screen(406, -1440, 2560, 1400, 1.5);
    *rig.host.geometry.lock().unwrap() = Geometry {
        screens: vec![primary, upper],
        primary: Some(primary),
        cursor: Some((1000.0, -700.0)),
    };
    let s = OverlaySettings {
        anchor: Anchor::BottomLeft,
        remember_placement: false,
        ..OverlaySettings::default()
    };
    rig.overlay.save_settings(s).unwrap();
    let rig = rig.running();
    // 150 %: 540 × 660 physical, 36 px in from the bottom left.
    assert_eq!(rig.host.calls()[0], "create 442 -736 540x660 Interactive");
}

#[test]
fn with_remembering_off_it_opens_at_its_default_place_every_time() {
    let store = MemoryStore::default();
    let rig = Rig::with_store(store.clone()).running();
    user_moves(
        &rig.overlay,
        Rect {
            x: 100,
            y: 200,
            width: 500,
            height: 600,
        },
        1.0,
    );
    rig.overlay.shutdown();
    // Remembered: it comes back where it was.
    let again = Rig::with_store(store.clone()).running();
    assert_eq!(again.host.calls()[0], "create 100 200 500x600 Interactive");
    let s = OverlaySettings {
        remember_placement: false,
        ..OverlaySettings::default()
    };
    again.overlay.save_settings(s).unwrap();
    again.overlay.shutdown();
    let third = Rig::with_store(store).running();
    assert_eq!(third.host.calls()[0], "create 1536 24 360x440 Interactive");
}

#[test]
fn collapsing_keeps_the_expanded_height_for_later() {
    let rig = Rig::new().running();
    rig.host.clear();
    rig.overlay.set_collapsed(true).unwrap();
    assert_eq!(rig.host.calls(), ["collapsed=true expanded=440"]);
    // While collapsed the window is 38 px high; moving it keeps 440, and a
    // move to a 150 % monitor makes that 660.
    user_moves(
        &rig.overlay,
        Rect {
            x: 10,
            y: 20,
            width: 360,
            height: 38,
        },
        1.0,
    );
    user_moves(
        &rig.overlay,
        Rect {
            x: 3000,
            y: 20,
            width: 540,
            height: 57,
        },
        1.5,
    );
    rig.host.clear();
    rig.overlay.set_collapsed(false).unwrap();
    assert_eq!(rig.host.calls(), ["collapsed=false expanded=660"]);
    assert!(!rig.status().collapsed);
}

#[test]
fn a_new_size_or_corner_moves_an_open_overlay_there() {
    let rig = Rig::new().running();
    rig.host.clear();
    let mut s = OverlaySettings {
        width: 300,
        height: 200,
        anchor: Anchor::TopLeft,
        ..OverlaySettings::default()
    };
    rig.overlay.save_settings(s.clone()).unwrap();
    assert_eq!(rig.host.calls(), ["place 24 24 300x200"]);
    // A place of the middle row: centred vertically, here a third of the
    // way across from the right (the overlay's centre at 1280 of 1920).
    rig.host.clear();
    s.anchor = Anchor::MidCenterRight;
    rig.overlay.save_settings(s.clone()).unwrap();
    assert_eq!(rig.host.calls(), ["place 1130 420 300x200"]);
    // Other settings leave it where the user put it.
    rig.host.clear();
    s.opacity = 70;
    rig.overlay.save_settings(s).unwrap();
    assert!(rig.host.calls().is_empty());
}

// ------------------------------------------------------------ shortcuts

#[test]
fn shortcuts_are_registered_while_running_and_never_twice() {
    let rig = Rig::new();
    assert!(
        rig.registered().is_empty(),
        "off by default: nothing registered"
    );
    rig.overlay.start().unwrap();
    assert_eq!(rig.registered(), suggested());
    let views = rig.status().hotkeys;
    assert_eq!(views.len(), 5);
    assert!(views.iter().all(|v| v.registered && v.error.is_none()));
    // A second start registers nothing more.
    rig.overlay.start().unwrap();
    assert_eq!(rig.backend.hotkeys.lock().unwrap().len(), 5);
    rig.overlay.stop();
    assert!(rig.registered().is_empty());
    assert!(rig.backend.shut.load(Ordering::SeqCst));
    assert!(rig.status().hotkeys.is_empty());
    rig.overlay.start().unwrap();
    assert_eq!(rig.registered(), suggested());
    rig.overlay.shutdown();
    assert!(rig.registered().is_empty());
}

#[test]
fn changing_a_shortcut_unregisters_the_old_chord() {
    let rig = Rig::new().running();
    let mut s = OverlaySettings::default();
    s.shortcuts
        .insert(OverlayAction::NextNote, Binding::on("Ctrl+Alt+PageDown"));
    s.shortcuts.insert(
        OverlayAction::PreviousNote,
        Binding {
            chord: Chord::parse("Ctrl+Shift+F4").ok(),
            enabled: false,
        },
    );
    rig.overlay.save_settings(s).unwrap();
    assert_eq!(
        rig.registered(),
        [
            "Ctrl+Alt+PageDown",
            "Ctrl+Shift+F1",
            "Ctrl+Shift+F2",
            "Ctrl+Shift+F3"
        ]
    );
    assert_eq!(rig.status().hotkeys.len(), 4);
}

#[test]
fn a_shortcut_another_program_holds_is_shown_as_not_registered() {
    let rig = Rig::new();
    rig.backend
        .taken
        .lock()
        .unwrap()
        .push(Chord::parse("Ctrl+Shift+F3").unwrap());
    let rig = rig.running();
    let views = rig.status().hotkeys;
    let refused: Vec<_> = views.iter().filter(|v| !v.registered).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].action, OverlayAction::NewNote);
    assert_eq!(
        refused[0].error.as_deref(),
        Some("Not registered: another program uses this shortcut")
    );
    // The others work; the overlay runs.
    assert_eq!(rig.registered().len(), 4);
    assert!(rig.status().running);
}

#[test]
fn a_shortcut_the_autoclicker_holds_is_refused_with_its_name_and_retried_when_freed() {
    let rig = Rig::new();
    let claims = Claims::new();
    rig.overlay.share_claims(Arc::clone(&claims));
    claims.publish(
        Owner::Autoclicker,
        &[Chord::parse("Ctrl+Shift+F1").unwrap()],
    );
    let rig = rig.running();
    let v = rig
        .status()
        .hotkeys
        .into_iter()
        .find(|v| v.action == OverlayAction::ToggleVisible)
        .unwrap();
    assert_eq!(
        (v.registered, v.error.as_deref()),
        (
            false,
            Some("Not registered: the autoclicker uses this shortcut")
        )
    );
    assert_eq!(
        claims.holder(Chord::parse("Ctrl+Shift+F2").unwrap(), Owner::Autoclicker),
        Some(Owner::NotesOverlay),
        "the overlay publishes what it holds"
    );
    // The autoclicker lets it go: the overlay registers it by itself.
    claims.publish(Owner::Autoclicker, &[]);
    assert!(eventually(|| rig.registered() == suggested()));
    assert!(rig.status().hotkeys.iter().all(|v| v.registered));
    rig.overlay.stop();
    assert_eq!(
        claims.holder(Chord::parse("Ctrl+Shift+F2").unwrap(), Owner::Autoclicker),
        None,
        "off: it holds nothing"
    );
}

#[test]
fn the_autoclicker_and_the_overlay_name_each_other() {
    // The real autoclicker service against the overlay's claims: a chord
    // the overlay holds is refused by the autoclicker with its name.
    let claims = Claims::new();
    let rig = Rig::new();
    rig.overlay.share_claims(Arc::clone(&claims));
    let rig = rig.running();
    let clicker_backend = MockBackend::new();
    let b = Arc::clone(&clicker_backend);
    let dir = std::env::temp_dir().join(format!(
        "sotyware-overlay-claims-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let store =
        crate::storage::SqliteAutomationStore::open(&dir.join("automation.sqlite3")).unwrap();
    let automation = crate::automation::Automation::with(
        Ok(Box::new(store)),
        crate::automation::input::Support {
            available: true,
            backend: "mock",
            note: None,
        },
        Box::new(move |_| Ok(Arc::clone(&b) as Arc<dyn Backend>)),
        crate::automation::timing::RealClock::new(),
    );
    automation.share_claims(Arc::clone(&claims));
    let mut settings = automation.config().settings;
    settings.emergency_stop = Binding::on("Ctrl+Shift+F2");
    automation.save_settings(settings).unwrap();
    automation.start().unwrap();
    let stop = automation
        .status()
        .hotkeys
        .into_iter()
        .find(|h| h.chord == Chord::parse("Ctrl+Shift+F2").unwrap())
        .unwrap();
    assert_eq!(
        (stop.registered, stop.error.as_deref()),
        (
            false,
            Some("Not registered: the notes overlay uses this shortcut")
        )
    );
    // The overlay is turned off: the autoclicker registers it by itself.
    rig.overlay.stop();
    assert!(eventually(|| automation
        .status()
        .hotkeys
        .iter()
        .any(
            |h| h.chord == Chord::parse("Ctrl+Shift+F2").unwrap() && h.registered
        )));
    automation.stop();
}

#[test]
fn without_global_shortcuts_the_overlay_still_runs_and_says_why() {
    let note = "this desktop session uses Wayland, which does not let applications register global shortcuts";
    let rig = Rig::build(MemoryStore::default(), Some(note.into())).running();
    let s = rig.status();
    assert!(s.running && s.visible);
    assert!(rig.sink.lock().unwrap().is_none(), "no backend was opened");
    assert_eq!(s.shortcuts_note.as_deref(), Some(note));
    assert_eq!(s.hotkeys.len(), 5);
    assert!(s.hotkeys.iter().all(
        |h| !h.registered && h.error.as_deref() == Some(&format!("Not registered: {note}")[..])
    ));
}

#[test]
fn a_backend_that_cannot_open_leaves_the_overlay_running_without_shortcuts() {
    let overlay = NotesOverlay::with(
        Ok(Box::new(MemoryStore::default())),
        None,
        Box::new(|_| Err("the X11 display cannot be opened".into())),
    );
    let host = Arc::new(HostState::default());
    *host.geometry.lock().unwrap() = one_monitor();
    overlay.attach(Box::new(FakeHost(Arc::clone(&host))));
    overlay.start().unwrap();
    let s = overlay.status();
    assert!(s.running);
    assert_eq!(
        s.shortcuts_note.as_deref(),
        Some("the X11 display cannot be opened")
    );
    assert!(s.hotkeys.iter().all(|h| !h.registered));
}

#[test]
fn the_visibility_shortcut_shows_and_hides_it() {
    let rig = Rig::new().running();
    rig.press("Ctrl+Shift+F1");
    assert!(eventually(|| !rig.status().visible));
    rig.press("Ctrl+Shift+F1");
    assert!(eventually(|| rig.status().visible));
    assert!(
        !rig.host.focused.load(Ordering::SeqCst),
        "never takes the keyboard"
    );
}

#[test]
fn the_new_note_shortcut_creates_one_and_gives_it_the_keyboard() {
    let rig = Rig::new().running();
    rig.overlay
        .set_mode(Mode::Passive, Origin::Overlay)
        .unwrap();
    rig.overlay.set_visible(false).unwrap();
    let before = rig.status();
    rig.press("Ctrl+Shift+F3");
    assert!(eventually(
        || rig.status().note_count == 1 && rig.status().visible
    ));
    assert!(eventually(|| rig.status().mode == Mode::Interactive));
    let s = rig.status();
    assert_eq!(
        s.notes_seq,
        before.notes_seq + 1,
        "the page reads the notes again"
    );
    assert_eq!(s.active_seq, before.active_seq + 1);
    assert_eq!(s.focus_target, FocusTarget::Title);
    assert!(s.focus_seq > before.focus_seq);
    assert!(rig.host.focused.load(Ordering::SeqCst));
    let id = s.active_note.unwrap();
    assert_eq!(rig.store.note(&id).unwrap().title, "New note");
}

#[test]
fn next_and_previous_go_round_the_notes_in_order() {
    let rig = Rig::new().running();
    let o = &rig.overlay;
    let ids: Vec<String> = ["A", "B", "C"]
        .iter()
        .map(|t| {
            o.create_note(Some((*t).into()), Origin::Overlay)
                .unwrap()
                .id
        })
        .collect();
    o.set_pinned(&ids[2], true).unwrap();
    o.select_note(&ids[0]).unwrap();
    // Order: C (pinned), A, B.
    let seq = rig.status().active_seq;
    rig.press("Ctrl+Shift+F5");
    assert!(eventually(|| rig.status().active_seq == seq + 1));
    assert_eq!(rig.status().active_note.as_deref(), Some(ids[1].as_str()));
    rig.press("Ctrl+Shift+F5");
    assert!(eventually(|| rig.status().active_seq == seq + 2));
    assert_eq!(rig.status().active_note.as_deref(), Some(ids[2].as_str()));
    rig.press("Ctrl+Shift+F4");
    assert!(eventually(|| rig.status().active_seq == seq + 3));
    assert_eq!(rig.status().active_note.as_deref(), Some(ids[1].as_str()));
}

#[test]
fn a_shortcut_that_fails_says_so_in_the_status() {
    let rig = Rig::new();
    rig.host.fail_create.store(true, Ordering::SeqCst);
    rig.overlay.start().unwrap();
    rig.press("Ctrl+Shift+F1");
    assert!(eventually(|| rig.status().notice.is_some_and(|n| n
        .starts_with("Ctrl + Shift + F1: ")
        && n.contains("WebView2"))));
}

// ----------------------------------------------------------- the rest

#[test]
fn links_open_only_when_the_note_holds_them() {
    let rig = Rig::new().running();
    let n = rig.overlay.create_note(None, Origin::Overlay).unwrap();
    rig.overlay
        .save_note(&n.id, "Guide", "[map](https://example.com/map)", 1)
        .unwrap();
    rig.overlay
        .open_link(&n.id, "https://example.com/map")
        .unwrap();
    assert_eq!(
        rig.overlay
            .open_link(&n.id, "https://elsewhere.example/")
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        rig.overlay
            .open_link("nope", "https://example.com/map")
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        *rig.host.opened.lock().unwrap(),
        ["https://example.com/map"]
    );
}

#[test]
fn asking_for_the_settings_shows_the_dashboard_and_tells_it() {
    let rig = Rig::new().running();
    let before = rig.status().settings_seq;
    rig.host.clear();
    rig.overlay.request_settings();
    assert_eq!(rig.status().settings_seq, before + 1);
    assert_eq!(rig.host.calls(), ["show dashboard"]);
}

#[test]
fn the_status_is_sent_to_subscribers_when_it_changes() {
    struct Sink(Mutex<Vec<OverlayStatus>>);
    impl StatusSink for Sink {
        fn send(&self, status: &OverlayStatus) -> bool {
            self.0.lock().unwrap().push(status.clone());
            true
        }
    }
    let rig = Rig::new();
    let sink = Arc::new(Sink(Mutex::new(Vec::new())));
    let id = tauri::async_runtime::block_on(async {
        rig.overlay
            .subscribe(Arc::clone(&sink) as Arc<dyn StatusSink>)
    });
    assert_eq!(sink.0.lock().unwrap().len(), 1, "at once");
    rig.overlay.startup_finished();
    rig.overlay.start().unwrap();
    assert!(eventually(|| sink
        .0
        .lock()
        .unwrap()
        .last()
        .is_some_and(|s| s.running && s.visible)));
    rig.overlay.unsubscribe(id);
    let sent = sink.0.lock().unwrap().len();
    rig.overlay.set_visible(false).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(sink.0.lock().unwrap().len(), sent, "unsubscribed");
}

#[test]
fn the_pages_receive_the_status_in_this_shape() {
    let rig = Rig::new().running();
    let json = serde_json::to_value(rig.status()).unwrap();
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "activeNote",
            "activeSeq",
            "collapsed",
            "focusSeq",
            "focusTarget",
            "hotkeys",
            "mode",
            "noteCount",
            "notesSeq",
            "notice",
            "opacity",
            "running",
            "seq",
            "settingsSeq",
            "shortcutsNote",
            "storageError",
            "textSize",
            "visible",
        ]
    );
    assert_eq!(json["mode"], "interactive");
    assert_eq!(json["textSize"], "medium");
    assert_eq!(json["focusTarget"], "content");
    assert_eq!(
        json["hotkeys"][0],
        serde_json::json!({
            "action": "toggle-visible",
            "label": "Show or hide the overlay",
            "chord": "Ctrl+Shift+F1",
            "keys": "Ctrl + Shift + F1",
            "registered": true,
            "error": null,
        })
    );
    let config = serde_json::to_value(rig.overlay.config()).unwrap();
    assert_eq!(config["limits"]["minWidth"], 240);
    assert_eq!(config["limits"]["maxNotes"], MAX_NOTES);
    assert_eq!(config["suggestions"]["new-note"], "Ctrl+Shift+F3");
    assert!(
        config["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["code"] == "F1")
    );
    let e = serde_json::to_value(OverlayError::new(ErrorKind::FeatureOff, "x")).unwrap();
    assert_eq!(
        e,
        serde_json::json!({ "kind": "feature-off", "message": "x" })
    );
}

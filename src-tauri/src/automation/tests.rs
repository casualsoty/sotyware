//! Tests of the automation service (ADR-050) with an in-memory store and
//! the mock backend: no real input is ever sent or recorded.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::input::mock::MockBackend;
use super::input::{Backend, HotkeySink, RawEvent, RawKind, Support, Synth};
use super::keys::{Chord, Key};
use super::model::*;
use super::timing::{Clock, FakeClock, RealClock};
use super::*;

// ------------------------------------------------------------- doubles

#[derive(Default)]
struct MemData {
    settings: Option<String>,
    profiles: BTreeMap<String, (i64, String)>,
    macros: BTreeMap<String, (String, String)>,
    fail: bool,
}

#[derive(Clone, Default)]
struct MemoryStore(Arc<Mutex<MemData>>);

impl MemoryStore {
    fn check(&self) -> std::result::Result<(), StoreError> {
        if self.0.lock().unwrap().fail {
            Err(StoreError::new("disk is gone"))
        } else {
            Ok(())
        }
    }
}

impl AutomationStore for MemoryStore {
    fn settings(&mut self) -> std::result::Result<Option<String>, StoreError> {
        self.check()?;
        Ok(self.0.lock().unwrap().settings.clone())
    }
    fn save_settings(&mut self, data: &str) -> std::result::Result<(), StoreError> {
        self.check()?;
        self.0.lock().unwrap().settings = Some(data.to_owned());
        Ok(())
    }
    fn profiles(&mut self) -> std::result::Result<Vec<String>, StoreError> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let mut v: Vec<(i64, String, String)> = d
            .profiles
            .iter()
            .map(|(id, (pos, data))| (*pos, id.clone(), data.clone()))
            .collect();
        v.sort();
        Ok(v.into_iter().map(|(_, _, d)| d).collect())
    }
    fn save_profile(
        &mut self,
        id: &str,
        position: i64,
        data: &str,
    ) -> std::result::Result<(), StoreError> {
        self.check()?;
        self.0
            .lock()
            .unwrap()
            .profiles
            .insert(id.to_owned(), (position, data.to_owned()));
        Ok(())
    }
    fn delete_profile(&mut self, id: &str) -> std::result::Result<(), StoreError> {
        self.check()?;
        self.0.lock().unwrap().profiles.remove(id);
        Ok(())
    }
    fn macro_summaries(&mut self) -> std::result::Result<Vec<String>, StoreError> {
        self.check()?;
        Ok(self
            .0
            .lock()
            .unwrap()
            .macros
            .values()
            .map(|(s, _)| s.clone())
            .collect())
    }
    fn load_macro(&mut self, id: &str) -> std::result::Result<Option<String>, StoreError> {
        self.check()?;
        Ok(self
            .0
            .lock()
            .unwrap()
            .macros
            .get(id)
            .map(|(_, d)| d.clone()))
    }
    fn save_macro(
        &mut self,
        id: &str,
        summary: &str,
        data: &str,
        _: i64,
    ) -> std::result::Result<(), StoreError> {
        self.check()?;
        self.0
            .lock()
            .unwrap()
            .macros
            .insert(id.to_owned(), (summary.to_owned(), data.to_owned()));
        Ok(())
    }
    fn delete_macro(&mut self, id: &str) -> std::result::Result<(), StoreError> {
        self.check()?;
        self.0.lock().unwrap().macros.remove(id);
        Ok(())
    }
}

struct Harness {
    a: Arc<Automation>,
    backend: Arc<MockBackend>,
    store: MemoryStore,
    sink: Arc<Mutex<Option<HotkeySink>>>,
}

fn support() -> Support {
    Support {
        available: true,
        backend: "mock",
        note: None,
    }
}

fn harness_with(store: MemoryStore, clock: Arc<dyn Clock>) -> Harness {
    let backend = MockBackend::new().with_clock(Arc::clone(&clock));
    let sink: Arc<Mutex<Option<HotkeySink>>> = Arc::new(Mutex::new(None));
    let (b2, s2) = (Arc::clone(&backend), Arc::clone(&sink));
    let a = Automation::with(
        Ok(Box::new(store.clone())),
        support(),
        Box::new(move |s| {
            *s2.lock().unwrap() = Some(s);
            Ok(Arc::clone(&b2) as Arc<dyn Backend>)
        }),
        clock,
    );
    Harness {
        a,
        backend,
        store,
        sink,
    }
}

fn harness() -> Harness {
    harness_with(MemoryStore::default(), RealClock::new())
}

impl Harness {
    fn on(&self) -> &Self {
        self.a.start().unwrap();
        self
    }

    /// The user presses a registered shortcut.
    fn press(&self, chord: &str) {
        let chord = Chord::parse(chord).unwrap();
        let id = *self
            .backend
            .hotkeys
            .lock()
            .unwrap()
            .iter()
            .find(|(_, c)| **c == chord)
            .unwrap_or_else(|| panic!("{chord} is not registered"))
            .0;
        let sink = self.sink.lock().unwrap().clone().unwrap();
        sink(id);
    }

    fn registered(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .backend
            .hotkeys
            .lock()
            .unwrap()
            .values()
            .map(ToString::to_string)
            .collect();
        v.sort();
        v
    }

    fn wait_for(&self, what: &str, f: impl Fn(&AutomationStatus) -> bool) -> AutomationStatus {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let s = self.a.status();
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < until,
                "timed out waiting for {what}: {s:#?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn idle(&self) -> AutomationStatus {
        self.wait_for("the session to end", |s| s.session.is_none())
    }

    fn profile(&self) -> Profile {
        let c = self.a.config();
        c.profiles
            .into_iter()
            .find(|p| p.id == c.active_profile)
            .unwrap()
    }

    fn set_clicker(&self, f: impl FnOnce(&mut ClickerConfig)) {
        let mut p = self.profile();
        f(&mut p.clicker);
        self.a.save_profile(p).unwrap();
    }

    fn macro_with(&self, name: &str, actions: Vec<Action>) -> Macro {
        let mut m = self.a.create_macro(name, "", "").unwrap();
        m.actions = actions;
        self.a.save_macro(m.clone()).unwrap();
        self.a.load_macro(&m.id).unwrap()
    }
}

fn key(code: &str) -> Key {
    Key::from_code(code).unwrap()
}

fn tap(code: &str) -> Vec<Action> {
    vec![
        Action::new(
            0,
            ActionKind::Key {
                key: key(code),
                state: Press::Down,
            },
        ),
        Action::new(
            10,
            ActionKind::Key {
                key: key(code),
                state: Press::Up,
            },
        ),
    ]
}

// --------------------------------------------------------------- tests

#[test]
fn the_first_start_makes_a_default_profile_and_settings() {
    let h = harness();
    let c = h.a.config();
    assert_eq!(c.profiles.len(), 1);
    assert_eq!(c.profiles[0].name, "Default");
    assert_eq!(c.settings.default_profile, c.profiles[0].id);
    assert_eq!(c.active_profile, c.profiles[0].id);
    assert_eq!(c.settings.emergency_stop, Binding::on("F8"));
    // Stored, so a restart finds the same profile.
    let again = harness_with(h.store.clone(), RealClock::new());
    assert_eq!(again.a.config().profiles, c.profiles);
}

#[test]
fn turning_on_registers_the_shortcuts_and_off_releases_them() {
    let h = harness();
    let s = h.a.status();
    assert!(!s.running);
    assert!(
        s.hotkeys.iter().all(|k| !k.registered),
        "nothing before the feature runs"
    );
    h.on();
    assert_eq!(h.registered(), ["F10", "F6", "F7", "F8", "F9", "Shift+F11"]);
    let s = h.a.status();
    assert!(s.running);
    assert!(s.hotkeys.iter().all(|k| k.registered && k.error.is_none()));
    h.a.stop();
    assert!(h.registered().is_empty());
    assert!(h.backend.shut.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.a.status().running);
}

#[test]
fn a_shortcut_another_program_holds_is_reported_and_the_others_still_work() {
    let h = harness();
    h.backend
        .taken
        .lock()
        .unwrap()
        .push(Chord::parse("F7").unwrap());
    h.on();
    let s = h.a.status();
    let f7 = s
        .hotkeys
        .iter()
        .find(|k| k.chord.to_string() == "F7")
        .unwrap();
    assert!(!f7.registered);
    assert!(f7.error.as_deref().unwrap().contains("another program"));
    assert!(h.registered().contains(&"F8".to_owned()));
}

#[test]
fn shortcut_conflicts_are_refused_with_the_other_owner_named() {
    let h = harness();
    let mut p = h.profile();
    p.hotkeys
        .insert(HotkeyAction::PauseResume, Binding::on("F8"));
    let e = h.a.save_profile(p).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert!(e.message.contains("Emergency stop"), "{}", e.message);

    let mut p = h.profile();
    p.hotkeys
        .insert(HotkeyAction::StartClicker, Binding::on("F6"));
    let e = h.a.save_profile(p).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);

    let m = h.a.create_macro("Login", "", "").unwrap();
    let e =
        h.a.update_macro_info(
            &m.id,
            MacroChanges {
                shortcut: Some(Binding::on("F9")),
                ..MacroChanges::default()
            },
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert!(e.message.contains("Start recording"), "{}", e.message);
    // A free key works, and registers at once when the feature runs.
    h.on();
    h.a.update_macro_info(
        &m.id,
        MacroChanges {
            shortcut: Some(Binding::on("Ctrl+Alt+KeyL")),
            ..MacroChanges::default()
        },
    )
    .unwrap();
    assert!(h.registered().contains(&"Ctrl+Alt+KeyL".to_owned()));
    // And the emergency stop cannot take it.
    let mut settings = h.a.config().settings;
    settings.emergency_stop = Binding::on("Ctrl+Alt+KeyL");
    assert_eq!(
        h.a.save_settings(settings).unwrap_err().kind,
        ErrorKind::Conflict
    );
}

#[test]
fn nothing_runs_while_the_feature_is_off() {
    let h = harness();
    assert_eq!(
        h.a.start_clicker(true, true).unwrap_err().kind,
        ErrorKind::FeatureOff
    );
    assert_eq!(
        h.a.start_recording(true).unwrap_err().kind,
        ErrorKind::FeatureOff
    );
    assert_eq!(h.a.screen().unwrap_err().kind, ErrorKind::FeatureOff);
}

#[test]
fn a_system_without_input_automation_cannot_turn_it_on() {
    let a = Automation::with(
        Ok(Box::new(MemoryStore::default())),
        Support {
            available: false,
            backend: "none",
            note: Some("Wayland".into()),
        },
        Box::new(|_| Err("never".into())),
        RealClock::new(),
    );
    assert_eq!(a.start(), Err("Wayland".to_owned()));
    assert_eq!(
        a.start_clicker(true, true).unwrap_err().kind,
        ErrorKind::Unavailable
    );
}

#[test]
fn the_toggle_shortcut_starts_and_stops_the_clicker_and_releases_its_button() {
    let h = harness();
    h.on();
    h.set_clicker(|c| {
        c.interval_ms = 5.0;
        c.hold_ms = 3;
    });
    h.press("F6");
    h.wait_for("clicks", |s| {
        matches!(&s.session, Some(SessionView::Clicker { progress, .. }) if progress.clicks >= 3)
    });
    h.press("F6");
    let s = h.idle();
    let last = s.last.unwrap();
    assert_eq!((last.kind, last.outcome), ("clicker", "stopped"));
    assert!(last.message.contains("F6"), "{}", last.message);
    assert!(h.backend.still_held().is_empty());
}

#[test]
fn the_emergency_stop_stops_playback_and_releases_held_keys() {
    let h = harness();
    h.on();
    let m = h.macro_with(
        "Hold",
        vec![
            Action::new(
                0,
                ActionKind::Key {
                    key: key("ShiftLeft"),
                    state: Press::Down,
                },
            ),
            Action::new(60_000, ActionKind::Wait),
        ],
    );
    h.a.play(Some(m.id), false, false).unwrap();
    h.wait_for("Shift held", |_| !h.backend.still_held().is_empty());
    h.press("F8");
    let s = h.idle();
    assert_eq!(s.last.unwrap().message, "Emergency stop (F8)");
    assert!(h.backend.still_held().is_empty());
}

#[test]
fn the_emergency_stop_works_while_a_macro_holds_modifiers() {
    // A system matches a shortcut's modifiers exactly (RegisterHotKey,
    // XGrabKey): with Shift held by the macro, the user's F8 arrives as
    // Shift+F8 (found by the Linux desktop test, where it stopped nothing).
    let h = harness();
    h.on();
    let idle = h.registered();
    let listed = h.a.status().hotkeys.len();
    let m = h.macro_with(
        "Hold",
        vec![
            Action::new(
                0,
                ActionKind::Key {
                    key: key("ShiftLeft"),
                    state: Press::Down,
                },
            ),
            Action::new(60_000, ActionKind::Wait),
        ],
    );
    h.a.play(Some(m.id), false, false).unwrap();
    h.wait_for("Shift held", |_| !h.backend.still_held().is_empty());
    assert_eq!(
        h.registered().len(),
        idle.len() + 15,
        "{:?}",
        h.registered()
    );
    assert_eq!(
        h.a.status().hotkeys.len(),
        listed,
        "the other combinations are never listed"
    );
    h.press("Shift+F8");
    let s = h.idle();
    assert_eq!(s.last.unwrap().message, "Emergency stop (Shift + F8)");
    assert!(h.backend.still_held().is_empty());
    assert_eq!(h.registered(), idle, "gone once the session ended");
}

#[test]
fn the_emergency_stop_with_modifiers_held_stops_a_recording_and_is_not_recorded() {
    let h = harness();
    h.on();
    h.press("F9");
    h.wait_for("the recording", |s| {
        matches!(
            &s.session,
            Some(SessionView::Recording {
                state: RunState::Running,
                ..
            })
        )
    });
    let sink = h.backend.capture_sink().unwrap();
    let t0 = Instant::now();
    let ev = |ms: u64, kind| RawEvent {
        at: t0 + Duration::from_millis(ms),
        kind,
    };
    for e in [
        ev(0, RawKind::Key(key("F9"), Press::Up)),
        ev(100, RawKind::Key(key("KeyH"), Press::Down)),
        ev(140, RawKind::Key(key("KeyH"), Press::Up)),
        ev(500, RawKind::Key(key("ShiftLeft"), Press::Down)),
    ] {
        sink.send(e).unwrap();
    }
    h.wait_for(
        "H and Shift",
        |s| matches!(&s.session, Some(SessionView::Recording { actions, .. }) if *actions == 3),
    );
    // F8 with Shift held: the emergency stop, so neither key is recorded.
    sink.send(ev(600, RawKind::Key(key("F8"), Press::Down)))
        .unwrap();
    h.wait_for(
        "Shift taken back out",
        |s| matches!(&s.session, Some(SessionView::Recording { actions, .. }) if *actions == 2),
    );
    h.press("Shift+F8");
    let s = h.idle();
    assert_eq!(s.last.unwrap().message, "Recorded 2 actions");
    let saved = h.a.save_recording("Typed", "", "").unwrap();
    let m = h.a.load_macro(&saved.id).unwrap();
    let described: Vec<String> = m.actions.iter().map(Action::describe).collect();
    assert_eq!(described, ["H down", "H up"]);
}

#[test]
fn only_one_session_runs_at_a_time() {
    let h = harness();
    h.on();
    h.a.start_clicker(false, true).unwrap();
    assert_eq!(
        h.a.start_recording(false).unwrap_err().kind,
        ErrorKind::Busy
    );
    assert_eq!(
        h.a.start_clicker(false, true).unwrap_err().kind,
        ErrorKind::Busy
    );
    h.a.stop_session(StopReason::User("test".into()), true);
    assert!(h.a.status().session.is_none());
}

#[test]
fn an_endless_run_from_the_dashboard_needs_confirming_without_an_emergency_stop() {
    let h = harness();
    h.on();
    // F8 is registered: no question.
    h.a.start_clicker(true, false).unwrap();
    h.a.stop_session(StopReason::User("test".into()), true);
    let mut settings = h.a.config().settings;
    settings.emergency_stop.enabled = false;
    h.a.save_settings(settings).unwrap();
    assert_eq!(
        h.a.start_clicker(true, false).unwrap_err().kind,
        ErrorKind::Confirm
    );
    // A bounded run needs no confirmation, and a confirmed one starts.
    h.set_clicker(|c| c.max_clicks = Some(2));
    h.a.start_clicker(true, false).unwrap();
    h.a.stop_session(StopReason::User("test".into()), true);
    h.set_clicker(|c| c.max_clicks = None);
    h.a.start_clicker(true, true).unwrap();
    h.a.stop_session(StopReason::User("test".into()), true);
}

#[test]
fn a_start_from_the_dashboard_counts_down_first() {
    let h = harness();
    h.on();
    let mut settings = h.a.config().settings;
    settings.dashboard_countdown_ms = 5_000;
    h.a.save_settings(settings).unwrap();
    h.a.start_clicker(true, false).unwrap();
    let s = h.a.status();
    match s.session {
        Some(SessionView::Clicker { progress, .. }) => {
            assert_eq!(progress.state, RunState::Countdown);
            assert_eq!(progress.countdown_ms, Some(5_000));
        }
        other => panic!("{other:?}"),
    }
    h.a.stop_session(StopReason::User("test".into()), true);
    assert!(h.backend.presses(MouseButton::Left).is_empty());
}

#[test]
fn a_recording_becomes_a_macro_without_its_stop_shortcut() {
    let h = harness();
    h.on();
    *h.backend.foreground.lock().unwrap() = Some(WindowInfo {
        title: "Notes".into(),
        process: "notes.exe".into(),
        client: Rect::new(100, 100, 800, 600),
    });
    h.press("F9");
    h.wait_for("the recording", |s| {
        matches!(
            &s.session,
            Some(SessionView::Recording {
                state: RunState::Running,
                ..
            })
        )
    });
    let sink = h.backend.capture_sink().unwrap();
    let t0 = Instant::now();
    let ev = |ms: u64, kind| RawEvent {
        at: t0 + Duration::from_millis(ms),
        kind,
    };
    for e in [
        ev(0, RawKind::Key(key("F9"), Press::Up)),
        ev(
            10,
            RawKind::Button(MouseButton::Left, Press::Down, Point::new(300, 300)),
        ),
        ev(
            90,
            RawKind::Button(MouseButton::Left, Press::Up, Point::new(300, 300)),
        ),
        ev(500, RawKind::Key(key("KeyH"), Press::Down)),
        ev(540, RawKind::Key(key("KeyH"), Press::Up)),
        ev(900, RawKind::Key(key("F10"), Press::Down)),
    ] {
        sink.send(e).unwrap();
    }
    h.wait_for(
        "four actions",
        |s| matches!(&s.session, Some(SessionView::Recording { actions, .. }) if *actions >= 5),
    );
    h.press("F10");
    let s = h.idle();
    let draft = s.draft.unwrap();
    assert_eq!(draft.actions, 5, "{:?}", draft.recent);
    assert_eq!(draft.window.unwrap().process, "notes.exe");
    let saved = h.a.save_recording("  Note  ", "a test", "Work").unwrap();
    assert_eq!(
        (saved.name.as_str(), saved.folder.as_str()),
        ("Note", "Work")
    );
    assert!(h.a.status().draft.is_none());
    let m = h.a.load_macro(&saved.id).unwrap();
    let described: Vec<String> = m.actions.iter().map(Action::describe).collect();
    assert_eq!(
        described,
        [
            "Move to 300, 300",
            "Left button down at 300, 300",
            "Left button up at 300, 300",
            "H down",
            "H up"
        ]
    );
    assert_eq!(m.recorded_on.unwrap().monitors.len(), 1);
    // Recording needs the draft saved or discarded first.
    h.a.start_recording(false).unwrap();
    h.a.stop_session(StopReason::User("test".into()), true);
}

#[test]
fn an_unsaved_recording_blocks_the_next_one_until_discarded() {
    let h = harness();
    h.on();
    h.a.start_recording(false).unwrap();
    h.wait_for("the recording", |s| {
        matches!(
            &s.session,
            Some(SessionView::Recording {
                state: RunState::Running,
                ..
            })
        )
    });
    let sink = h.backend.capture_sink().unwrap();
    sink.send(RawEvent {
        at: Instant::now(),
        kind: RawKind::Key(key("KeyQ"), Press::Down),
    })
    .unwrap();
    h.wait_for(
        "an action",
        |s| matches!(&s.session, Some(SessionView::Recording { actions, .. }) if *actions >= 1),
    );
    h.a.stop_session(StopReason::User(DASHBOARD_STOP.into()), true);
    assert!(h.a.status().draft.is_some());
    assert_eq!(
        h.a.start_recording(false).unwrap_err().kind,
        ErrorKind::Busy
    );
    h.a.discard_recording();
    h.a.start_recording(false).unwrap();
    h.a.stop_session(StopReason::User("test".into()), true);
}

#[test]
fn playing_the_profiles_macro_uses_the_profiles_settings() {
    let h = harness_with(MemoryStore::default(), FakeClock::new());
    h.on();
    let m = h.macro_with("Tap", tap("KeyA"));
    let mut p = h.profile();
    p.macro_id = Some(m.id.clone());
    p.playback = Some(PlaybackSettings {
        repeat: Some(3),
        ..PlaybackSettings::default()
    });
    h.a.save_profile(p).unwrap();
    h.press("Shift+F11");
    h.wait_for("the playback to end", |s| s.last.is_some());
    let s = h.idle();
    assert_eq!(s.last.unwrap().message, "Finished: 3 repetitions");
    let downs = h
        .backend
        .sent()
        .into_iter()
        .filter(|s| *s == Synth::Key(key("KeyA"), Press::Down))
        .count();
    assert_eq!(downs, 3);
    // The macro played alone uses its own settings: once.
    h.a.play(Some(m.id), false, false).unwrap();
    h.wait_for("the second playback", |s| {
        s.session.is_none() && h.backend.sent().len() == 8
    });
}

#[test]
fn a_profile_without_a_macro_says_so() {
    let h = harness();
    h.on();
    let e = h.a.play(None, true, false).unwrap_err();
    assert!(e.message.contains("has no macro"), "{}", e.message);
    // From a shortcut, the reason becomes the page's notice.
    h.press("Shift+F11");
    let s = h.wait_for("the notice", |s| s.notice.is_some());
    assert!(s.notice.unwrap().starts_with("Shift + F11:"));
}

#[test]
fn macros_are_created_edited_duplicated_and_deleted() {
    let h = harness();
    let m = h.a.create_macro("Daily", "", "Work").unwrap();
    assert_eq!(h.a.create_macro("daily", "", "").unwrap().name, "daily (2)");
    let mut edited = m.clone();
    edited.actions = tap("KeyD");
    edited.description = "Opens the report".into();
    let saved = h.a.save_macro(edited).unwrap();
    assert_eq!(saved.summary.actions, 2);
    assert_eq!(saved.summary.duration_ms, 10);
    // A bad edit is refused and nothing changes.
    let mut bad = h.a.load_macro(&m.id).unwrap();
    bad.playback.speed = 100.0;
    assert_eq!(
        h.a.save_macro(bad).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(h.a.load_macro(&m.id).unwrap().actions.len(), 2);
    // A duplicate keeps the actions, not the shortcut.
    h.a.update_macro_info(
        &m.id,
        MacroChanges {
            shortcut: Some(Binding::on("Ctrl+Alt+KeyD")),
            favorite: Some(true),
            ..MacroChanges::default()
        },
    )
    .unwrap();
    let copy = h.a.duplicate_macro(&m.id).unwrap();
    // "daily (2)" is taken already (names compare without case).
    assert_eq!(copy.name, "Daily (3)");
    assert_eq!(copy.actions, 2);
    assert!(!copy.shortcut.enabled && !copy.favorite);
    // Deleting it clears the profiles that played it.
    let mut p = h.profile();
    p.macro_id = Some(m.id.clone());
    h.a.save_profile(p).unwrap();
    h.a.delete_macro(&m.id).unwrap();
    assert_eq!(h.profile().macro_id, None);
    assert_eq!(h.a.load_macro(&m.id).unwrap_err().kind, ErrorKind::NotFound);
    assert!(!h.a.macros().iter().any(|x| x.id == m.id));
}

#[test]
fn saving_a_macro_warns_about_keys_that_are_shortcuts() {
    let h = harness();
    h.on();
    let m = h.a.create_macro("Presses F6", "", "").unwrap();
    let mut edited = m.clone();
    edited.actions = tap("F6");
    let saved = h.a.save_macro(edited).unwrap();
    assert_eq!(saved.warnings.len(), 1);
    assert!(saved.warnings[0].contains("F6"));
}

#[test]
fn profiles_switch_their_shortcuts_at_once_and_persist() {
    let h = harness();
    h.on();
    let first = h.profile();
    let second = h.a.create_profile("Gaming", Some(&first.id)).unwrap();
    assert_eq!(second.clicker, first.clicker);
    let mut s2 = second.clone();
    s2.hotkeys
        .insert(HotkeyAction::ToggleClicker, Binding::on("Ctrl+F6"));
    s2.clicker.interval_ms = 20.0;
    h.a.save_profile(s2).unwrap();
    // Not active yet: its shortcut is not registered.
    assert!(!h.registered().contains(&"Ctrl+F6".to_owned()));
    h.a.activate_profile(&second.id).unwrap();
    assert!(h.registered().contains(&"Ctrl+F6".to_owned()));
    assert!(!h.registered().contains(&"F6".to_owned()));
    assert_eq!(h.a.status().active_profile.name, "Gaming");
    // The default profile is what sotyware starts with.
    h.a.set_default_profile(&second.id).unwrap();
    let again = harness_with(h.store.clone(), RealClock::new());
    let c = again.a.config();
    assert_eq!(c.active_profile, second.id);
    assert_eq!(c.profiles.len(), 2);
    assert_eq!(
        c.profiles
            .iter()
            .find(|p| p.id == second.id)
            .unwrap()
            .clicker
            .interval_ms,
        20.0
    );
    // Deleting the active default profile activates the remaining one.
    h.a.delete_profile(&second.id).unwrap();
    let c = h.a.config();
    assert_eq!(
        (
            c.active_profile.as_str(),
            c.settings.default_profile.as_str()
        ),
        (first.id.as_str(), first.id.as_str())
    );
    assert!(h.registered().contains(&"F6".to_owned()));
    assert_eq!(
        h.a.delete_profile(&first.id).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn invalid_profiles_and_settings_are_refused() {
    let h = harness();
    let mut p = h.profile();
    p.clicker.interval_ms = 0.0;
    assert_eq!(
        h.a.save_profile(p).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let mut p = h.profile();
    p.macro_id = Some("macro-gone".into());
    assert_eq!(h.a.save_profile(p).unwrap_err().kind, ErrorKind::NotFound);
    let mut s = h.a.config().settings;
    s.dashboard_countdown_ms = 60_000;
    assert_eq!(
        h.a.save_settings(s).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let mut s = h.a.config().settings;
    s.recording.mouse_moves = false;
    s.recording.mouse_buttons = false;
    s.recording.keyboard = false;
    s.recording.scroll = false;
    assert!(h.a.save_settings(s).is_err());
}

#[test]
fn export_and_import_give_new_ids_free_names_and_switched_off_shortcuts() {
    let h = harness();
    let m = h.macro_with("Report", tap("KeyR"));
    h.a.update_macro_info(
        &m.id,
        MacroChanges {
            shortcut: Some(Binding::on("Ctrl+Alt+KeyR")),
            ..MacroChanges::default()
        },
    )
    .unwrap();
    let mut p = h.profile();
    p.macro_id = Some(m.id.clone());
    h.a.save_profile(p.clone()).unwrap();
    let text =
        h.a.export_text(std::slice::from_ref(&m.id), std::slice::from_ref(&p.id))
            .unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["format"], "sotyware-automation");
    assert_eq!(json["formatVersion"], 1);
    assert_eq!(json["macros"][0]["formatVersion"], 1);
    assert_eq!(json["macros"][0]["appVersion"], env!("CARGO_PKG_VERSION"));

    let report = h.a.import_text(&text).unwrap();
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    let imported = &report.macros[0];
    assert_ne!(imported.id, m.id);
    assert_eq!(imported.name, "Report (2)");
    assert!(!imported.shortcut.enabled);
    let profile =
        h.a.config()
            .profiles
            .into_iter()
            .find(|x| x.id == report.profiles[0].id)
            .unwrap();
    assert_eq!(profile.name, "Default (2)");
    assert_eq!(profile.macro_id.as_deref(), Some(imported.id.as_str()));
}

#[test]
fn an_import_reports_bad_items_and_refuses_foreign_files() {
    let h = harness();
    let m = h.macro_with("Good", tap("KeyG"));
    let text = h.a.export_text(std::slice::from_ref(&m.id), &[]).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut bad = json["macros"][0].clone();
    bad["name"] = "Bad".into();
    bad["actions"][0]["delayMs"] = serde_json::json!(u64::from(MAX_DELAY_MS) + 1);
    json["macros"].as_array_mut().unwrap().push(bad);
    let report = h.a.import_text(&json.to_string()).unwrap();
    assert_eq!(report.macros.len(), 1);
    assert_eq!(report.problems.len(), 1);
    assert!(
        report.problems[0].starts_with("Macro \"Bad\""),
        "{:?}",
        report.problems
    );
    let e = h.a.import_text("{\"hello\": 1}").unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    // A macro with an action sotyware does not know (code to run) is
    // never imported.
    let mut evil = serde_json::from_str::<serde_json::Value>(&text).unwrap();
    evil["macros"][0]["actions"][0] =
        serde_json::json!({"delayMs": 0, "type": "run", "command": "calc.exe"});
    assert!(h.a.import_text(&evil.to_string()).is_err());
}

#[test]
fn a_broken_database_keeps_defaults_in_memory_and_reports_it() {
    let store = MemoryStore::default();
    store.0.lock().unwrap().fail = true;
    let h = harness_with(store.clone(), RealClock::new());
    let s = h.a.status();
    assert!(s.storage_error.unwrap().contains("disk is gone"));
    assert_eq!(h.a.config().profiles.len(), 1);
    assert_eq!(
        h.a.create_macro("X", "", "").unwrap_err().kind,
        ErrorKind::Storage
    );
    // A failed profile save changes nothing.
    let mut p = h.profile();
    p.name = "Renamed".into();
    assert_eq!(h.a.save_profile(p).unwrap_err().kind, ErrorKind::Storage);
    assert_eq!(h.profile().name, "Default");
}

#[test]
fn settings_this_version_cannot_read_are_left_as_they_are() {
    let store = MemoryStore::default();
    store.0.lock().unwrap().settings = Some("{\"fromTheFuture\": true}".into());
    let h = harness_with(store.clone(), RealClock::new());
    assert_eq!(h.a.config().settings.emergency_stop, Binding::on("F8"));
    assert_eq!(
        store.0.lock().unwrap().settings.as_deref(),
        Some("{\"fromTheFuture\": true}")
    );
}

#[test]
fn unreadable_stored_documents_are_skipped() {
    let store = MemoryStore::default();
    store
        .0
        .lock()
        .unwrap()
        .profiles
        .insert("profile-x".into(), (0, "{not json".into()));
    let h = harness_with(store, RealClock::new());
    assert_eq!(h.a.config().profiles.len(), 1);
    assert_eq!(h.profile().name, "Default");
}

#[test]
fn turning_the_feature_off_stops_a_session_and_releases_everything() {
    let h = harness();
    h.on();
    let m = h.macro_with(
        "Drag",
        vec![
            Action::new(
                0,
                ActionKind::Button {
                    button: MouseButton::Left,
                    state: Press::Down,
                    x: 10,
                    y: 10,
                },
            ),
            Action::new(60_000, ActionKind::Wait),
        ],
    );
    h.a.play(Some(m.id), false, false).unwrap();
    h.wait_for("the button held", |_| !h.backend.still_held().is_empty());
    h.a.stop();
    assert!(h.a.status().session.is_none());
    assert!(h.backend.still_held().is_empty());
    assert_eq!(
        h.a.status().last.unwrap().message,
        "Stopped: the autoclicker was turned off"
    );
}

#[test]
fn pause_and_resume_reach_the_running_session() {
    let h = harness();
    h.on();
    h.set_clicker(|c| c.interval_ms = 5.0);
    h.a.start_clicker(false, true).unwrap();
    h.a.set_paused(true).unwrap();
    h.wait_for("the pause", |s| {
        matches!(&s.session, Some(SessionView::Clicker { progress, .. }) if progress.state == RunState::Paused)
    });
    h.press("F7");
    h.wait_for("the resume", |s| {
        matches!(&s.session, Some(SessionView::Clicker { progress, .. }) if progress.state == RunState::Running)
    });
    h.a.stop_session(StopReason::User("test".into()), true);
    assert_eq!(
        h.a.set_paused(true).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn captures_read_the_pointer_and_refuse_sotyware_as_the_target_window() {
    let h = harness();
    h.on();
    *h.backend.cursor.lock().unwrap() = Point::new(42, 24);
    let p = h.a.cursor_now().unwrap();
    assert_eq!(
        (p.point, p.monitor, p.on_sotyware),
        (Point::new(42, 24), Some(0), false)
    );
    h.backend
        .own_active
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(h.a.window_now().unwrap_err().kind, ErrorKind::InvalidInput);
    let screen = h.a.screen().unwrap();
    assert_eq!(screen.bounds, Some(Rect::new(0, 0, 1920, 1080)));
}

/// `src/automation.ts` parses exactly these shapes.
#[test]
fn the_page_receives_the_status_in_this_shape() {
    let h = harness();
    h.on();
    let json = serde_json::to_value(h.a.status()).unwrap();
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "activeProfile",
            "activeWindow",
            "draft",
            "hotkeys",
            "last",
            "notice",
            "running",
            "seq",
            "session",
            "storageError",
            "support"
        ]
    );
    assert_eq!(
        json["support"],
        serde_json::json!({"available": true, "backend": "mock", "note": null})
    );
    let f6 = json["hotkeys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["chord"] == "F6")
        .unwrap();
    assert_eq!(
        f6,
        &serde_json::json!({
            "owner": {"kind": "profile", "action": "toggle-clicker"},
            "label": "Start or stop the autoclicker",
            "chord": "F6",
            "keys": "F6",
            "registered": true,
            "error": null
        })
    );
    h.set_clicker(|c| c.interval_ms = 5.0);
    h.a.start_clicker(false, true).unwrap();
    let s = serde_json::to_value(h.a.status()).unwrap();
    assert_eq!(s["session"]["kind"], "clicker");
    let progress = s["session"]["progress"].as_object().unwrap();
    let mut pk: Vec<&str> = progress.keys().map(String::as_str).collect();
    pk.sort_unstable();
    assert_eq!(
        pk,
        [
            "clicks",
            "countdownMs",
            "cps",
            "elapsedMs",
            "remaining",
            "skipped",
            "state"
        ]
    );
    h.a.stop_session(StopReason::User("test".into()), true);
}

#[test]
fn the_configuration_carries_the_keys_and_the_suggested_shortcuts() {
    let h = harness();
    let json = serde_json::to_value(h.a.config()).unwrap();
    let keys = json["keys"].as_array().unwrap();
    assert_eq!(keys.len(), super::keys::KEYS.len());
    assert_eq!(
        keys[0],
        serde_json::json!({"code": "KeyA", "label": "A", "kind": "layout"})
    );
    assert!(
        keys.iter()
            .any(|k| k["code"] == "ShiftLeft" && k["kind"] == "modifier")
    );
    assert_eq!(json["suggestions"]["toggle-clicker"], "F6");
    assert_eq!(json["suggestions"]["stop-all"], "F8");
    assert_eq!(json["suggestions"]["play-macro"], "Shift+F11");
    assert!(json["suggestions"].get("start-clicker").is_none());
}

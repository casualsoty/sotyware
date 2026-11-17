//! The boundary between the automation engines and the operating system
//! (ADR-050).
//!
//! [`Backend`] is everything the engines need from the platform: send
//! input, read the pointer, the monitors and the active window, register
//! global shortcuts and capture the user's input for the recorder. The
//! Windows implementation is `windows.rs` (SendInput, low-level hooks,
//! RegisterHotKey), the X11 one `x11.rs` (XTest, XRecord, XGrabKey); every
//! other system has none. Tests use [`mock::MockBackend`], so no test of
//! the engines ever moves the real pointer or presses a real key.
//!
//! [`Output`] wraps a backend for one session and remembers every key and
//! button the session holds down, so that a stop, a pause, an error, the
//! end of a macro or even a panic releases them: automation never leaves a
//! key or button stuck.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Once};
use std::time::Instant;

use super::keys::{Chord, Key};
use super::model::{Monitor, MouseButton, Point, Press, WindowInfo};

/// One piece of input to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Synth {
    /// Move the pointer to a point of the virtual desktop.
    Move(Point),
    Button(MouseButton, Press),
    /// Turn the wheel; 120 is one notch.
    Wheel {
        dx: i32,
        dy: i32,
    },
    Key(Key, Press),
}

/// One piece of input the user made, as the recorder receives it. Only a
/// platform backend makes them (none on macOS yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct RawEvent {
    pub at: Instant,
    pub kind: RawKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub enum RawKind {
    Move(Point),
    Button(MouseButton, Press, Point),
    Wheel {
        dx: i32,
        dy: i32,
        at: Point,
    },
    Key(Key, Press),
    /// A key sotyware cannot replay (not in the key table).
    OtherKey,
}

/// Which system input the platform offers.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub available: bool,
    /// `windows`, `x11`, or `none`.
    pub backend: &'static str,
    /// Why automation is not available here, or what is limited.
    pub note: Option<String>,
}

/// Called with a registered shortcut's id when it is pressed. Runs on the
/// backend's own thread: it must return at once.
pub type HotkeySink = Arc<dyn Fn(u32) + Send + Sync>;

/// What the automation engines need from the platform.
pub trait Backend: Send + Sync {
    /// Sends `batch` as one uninterrupted piece of input where the system
    /// allows it.
    fn send(&self, batch: &[Synth]) -> Result<(), String>;
    fn cursor(&self) -> Result<Point, String>;
    fn monitors(&self) -> Result<Vec<Monitor>, String>;
    /// The window that receives keyboard input.
    fn foreground(&self) -> Option<WindowInfo>;
    /// Whether one of sotyware's own windows is under `p`.
    fn own_window_at(&self, p: Point) -> bool;
    /// Whether one of sotyware's own windows has the keyboard.
    fn own_window_active(&self) -> bool;
    /// The key's name on the current keyboard layout.
    fn key_name(&self, key: Key) -> Option<String>;
    /// Makes `chord` a global shortcut; its presses call the backend's
    /// sink with `id`. Fails when another program has it.
    fn register_hotkey(&self, id: u32, chord: Chord) -> Result<(), String>;
    fn unregister_hotkey(&self, id: u32);
    /// Starts sending the user's input to `sink` (never sotyware's own
    /// injected input).
    fn start_capture(&self, sink: Sender<RawEvent>) -> Result<(), String>;
    fn stop_capture(&self);
    /// Releases everything: shortcuts, capture, threads.
    fn shutdown(&self);
}

/// The keys and buttons a session holds down, in the order pressed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    buttons: Vec<MouseButton>,
    keys: Vec<Key>,
}

impl Held {
    pub fn note(&mut self, s: &Synth) {
        match *s {
            Synth::Button(b, Press::Down) => {
                if !self.buttons.contains(&b) {
                    self.buttons.push(b);
                }
            }
            Synth::Button(b, Press::Up) => self.buttons.retain(|&x| x != b),
            Synth::Key(k, Press::Down) => {
                if !self.keys.contains(&k) {
                    self.keys.push(k);
                }
            }
            Synth::Key(k, Press::Up) => self.keys.retain(|&x| x != k),
            Synth::Move(_) | Synth::Wheel { .. } => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buttons.is_empty() && self.keys.is_empty()
    }

    /// Releases in reverse order: buttons first, then keys last pressed
    /// first (so a held Ctrl is let go after the key it modified).
    pub fn release(&self) -> Vec<Synth> {
        let mut out: Vec<Synth> = self
            .buttons
            .iter()
            .rev()
            .map(|&b| Synth::Button(b, Press::Up))
            .collect();
        out.extend(self.keys.iter().rev().map(|&k| Synth::Key(k, Press::Up)));
        out
    }

    /// Presses again what [`release`](Self::release) let go of.
    pub fn press(&self) -> Vec<Synth> {
        let mut out: Vec<Synth> = self
            .keys
            .iter()
            .map(|&k| Synth::Key(k, Press::Down))
            .collect();
        out.extend(self.buttons.iter().map(|&b| Synth::Button(b, Press::Down)));
        out
    }
}

/// What must be released if the process panics (the hook below sends it).
type Pending = Option<(Arc<dyn Backend>, Vec<Synth>)>;
static PANIC_RELEASE: Mutex<Pending> = Mutex::new(None);
static HOOK: Once = Once::new();

/// Installs, once per process, a panic hook that releases what a session
/// holds before the panic goes on (release builds abort on a panic; the
/// hook still runs first). Chains to the hook that was there.
pub fn install_panic_release() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut pending) = PANIC_RELEASE.try_lock()
                && let Some((backend, release)) = pending.take()
            {
                let _ = backend.send(&release);
            }
            previous(info);
        }));
    });
}

/// One session's way to send input: the backend plus what it holds.
pub struct Output {
    backend: Arc<dyn Backend>,
    held: Held,
}

impl Output {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            held: Held::default(),
        }
    }

    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    pub fn held(&self) -> &Held {
        &self.held
    }

    /// Sends `batch` and remembers what it leaves held.
    pub fn send(&mut self, batch: &[Synth]) -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        let result = self.backend.send(batch);
        // Noted even on an error: part of the batch may have gone out, and
        // releasing a key that is not held does no harm.
        for s in batch {
            self.held.note(s);
        }
        self.publish();
        result
    }

    /// Lets go of everything held; returns what it was, so a pause can
    /// press it again.
    pub fn release_all(&mut self) -> Held {
        let held = std::mem::take(&mut self.held);
        if !held.is_empty() {
            if let Err(e) = self.backend.send(&held.release()) {
                eprintln!("sotyware: failed to release held keys and buttons: {e}");
            }
            self.publish();
        }
        held
    }

    /// Presses again what a pause released.
    pub fn press_again(&mut self, held: &Held) -> Result<(), String> {
        self.send(&held.press())
    }

    fn publish(&self) {
        let mut pending = PANIC_RELEASE.lock().unwrap_or_else(|e| e.into_inner());
        *pending = if self.held.is_empty() {
            None
        } else {
            Some((Arc::clone(&self.backend), self.held.release()))
        };
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // Whatever ended the session, nothing stays held.
        self.release_all();
    }
}

#[cfg(test)]
pub mod mock {
    //! A backend that records what it is asked to send, with the time of
    //! the session's clock, and plays the user's side on request.

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::automation::model::Rect;
    use crate::automation::timing::Clock;

    type SendHook = Box<dyn Fn(&MockBackend, &Synth) + Send + Sync>;
    type WindowHook = Box<dyn Fn(&MockBackend, Point) + Send + Sync>;

    pub struct MockBackend {
        clock: Mutex<Option<Arc<dyn Clock>>>,
        pub log: Mutex<Vec<(Duration, Synth)>>,
        pub cursor: Mutex<Point>,
        pub monitors: Mutex<Vec<Monitor>>,
        pub foreground: Mutex<Option<WindowInfo>>,
        /// sotyware's window, for `own_window_at`.
        pub own: Mutex<Option<Rect>>,
        pub own_active: AtomicBool,
        pub hotkeys: Mutex<BTreeMap<u32, Chord>>,
        /// Shortcuts "another program" holds: registering them fails.
        pub taken: Mutex<Vec<Chord>>,
        pub capture: Mutex<Option<Sender<RawEvent>>>,
        pub fail_sends: AtomicBool,
        pub shut: AtomicBool,
        on_send: Mutex<Option<SendHook>>,
        on_window_check: Mutex<Option<WindowHook>>,
    }

    impl MockBackend {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                clock: Mutex::new(None),
                log: Mutex::new(Vec::new()),
                cursor: Mutex::new(Point::new(500, 500)),
                monitors: Mutex::new(vec![Monitor {
                    bounds: Rect::new(0, 0, 1920, 1080),
                    primary: true,
                }]),
                foreground: Mutex::new(None),
                own: Mutex::new(None),
                own_active: AtomicBool::new(false),
                hotkeys: Mutex::new(BTreeMap::new()),
                taken: Mutex::new(Vec::new()),
                capture: Mutex::new(None),
                fail_sends: AtomicBool::new(false),
                shut: AtomicBool::new(false),
                on_send: Mutex::new(None),
                on_window_check: Mutex::new(None),
            })
        }

        pub fn with_clock(self: &Arc<Self>, clock: Arc<dyn Clock>) -> Arc<Self> {
            *self.clock.lock().unwrap() = Some(clock);
            Arc::clone(self)
        }

        /// Runs `f` after each synthesized input (the "user" reacting).
        pub fn on_send(&self, f: impl Fn(&MockBackend, &Synth) + Send + Sync + 'static) {
            *self.on_send.lock().unwrap() = Some(Box::new(f));
        }

        /// Runs `f` on each `own_window_at` (the "user" acting while an
        /// engine checks where its input would land).
        pub fn on_window_check(&self, f: impl Fn(&MockBackend, Point) + Send + Sync + 'static) {
            *self.on_window_check.lock().unwrap() = Some(Box::new(f));
        }

        pub fn sent(&self) -> Vec<Synth> {
            self.log.lock().unwrap().iter().map(|(_, s)| *s).collect()
        }

        /// The time of every button press of `button`.
        pub fn presses(&self, button: MouseButton) -> Vec<Duration> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, s)| *s == Synth::Button(button, Press::Down))
                .map(|(t, _)| *t)
                .collect()
        }

        /// What is still held at the end (should be nothing).
        pub fn still_held(&self) -> Held {
            let mut held = Held::default();
            for (_, s) in self.log.lock().unwrap().iter() {
                held.note(s);
            }
            held
        }

        /// The user presses a registered shortcut.
        pub fn capture_sink(&self) -> Option<Sender<RawEvent>> {
            self.capture.lock().unwrap().clone()
        }
    }

    impl Backend for MockBackend {
        fn send(&self, batch: &[Synth]) -> Result<(), String> {
            if self.fail_sends.load(Ordering::SeqCst) {
                return Err("input refused".into());
            }
            let now = self
                .clock
                .lock()
                .unwrap()
                .as_ref()
                .map_or(Duration::ZERO, |c| c.now());
            for s in batch {
                self.log.lock().unwrap().push((now, *s));
                if let Synth::Move(p) = s {
                    *self.cursor.lock().unwrap() = *p;
                }
                if let Some(f) = self.on_send.lock().unwrap().as_ref() {
                    f(self, s);
                }
            }
            Ok(())
        }

        fn cursor(&self) -> Result<Point, String> {
            Ok(*self.cursor.lock().unwrap())
        }

        fn monitors(&self) -> Result<Vec<Monitor>, String> {
            Ok(self.monitors.lock().unwrap().clone())
        }

        fn foreground(&self) -> Option<WindowInfo> {
            self.foreground.lock().unwrap().clone()
        }

        fn own_window_at(&self, p: Point) -> bool {
            if let Some(f) = self.on_window_check.lock().unwrap().as_ref() {
                f(self, p);
            }
            self.own.lock().unwrap().is_some_and(|r| r.contains(p))
        }

        fn own_window_active(&self) -> bool {
            self.own_active.load(Ordering::SeqCst)
        }

        fn key_name(&self, key: Key) -> Option<String> {
            Some(key.label().to_owned())
        }

        fn register_hotkey(&self, id: u32, chord: Chord) -> Result<(), String> {
            if self.taken.lock().unwrap().contains(&chord) {
                return Err("another program uses this shortcut".into());
            }
            self.hotkeys.lock().unwrap().insert(id, chord);
            Ok(())
        }

        fn unregister_hotkey(&self, id: u32) {
            self.hotkeys.lock().unwrap().remove(&id);
        }

        fn start_capture(&self, sink: Sender<RawEvent>) -> Result<(), String> {
            *self.capture.lock().unwrap() = Some(sink);
            Ok(())
        }

        fn stop_capture(&self) {
            *self.capture.lock().unwrap() = None;
        }

        fn shutdown(&self) {
            self.shut.store(true, Ordering::SeqCst);
            self.hotkeys.lock().unwrap().clear();
            self.stop_capture();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;

    fn key(code: &str) -> Key {
        Key::from_code(code).unwrap()
    }

    #[test]
    fn held_input_is_released_in_reverse_and_pressed_again_in_order() {
        let mut held = Held::default();
        for s in [
            Synth::Key(key("ControlLeft"), Press::Down),
            Synth::Key(key("ShiftLeft"), Press::Down),
            Synth::Button(MouseButton::Left, Press::Down),
            Synth::Key(key("KeyA"), Press::Down),
            Synth::Key(key("KeyA"), Press::Up),
        ] {
            held.note(&s);
        }
        assert_eq!(
            held.release(),
            [
                Synth::Button(MouseButton::Left, Press::Up),
                Synth::Key(key("ShiftLeft"), Press::Up),
                Synth::Key(key("ControlLeft"), Press::Up),
            ]
        );
        assert_eq!(
            held.press(),
            [
                Synth::Key(key("ControlLeft"), Press::Down),
                Synth::Key(key("ShiftLeft"), Press::Down),
                Synth::Button(MouseButton::Left, Press::Down),
            ]
        );
    }

    #[test]
    fn dropping_an_output_releases_what_it_holds() {
        let backend = MockBackend::new();
        {
            let mut out = Output::new(backend.clone());
            out.send(&[
                Synth::Key(key("ShiftLeft"), Press::Down),
                Synth::Button(MouseButton::Right, Press::Down),
            ])
            .unwrap();
            assert!(!backend.still_held().is_empty());
        }
        assert!(backend.still_held().is_empty());
    }

    #[test]
    fn a_failed_send_still_counts_what_may_have_gone_out() {
        let backend = MockBackend::new();
        let mut out = Output::new(backend.clone());
        backend
            .fail_sends
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(out.send(&[Synth::Key(key("KeyQ"), Press::Down)]).is_err());
        assert!(!out.held().is_empty());
    }
}

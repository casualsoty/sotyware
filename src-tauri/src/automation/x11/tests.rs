//! Integration tests of the X11 backend against a **private Xvfb display**
//! the test starts and stops: the input goes to a window the test owns on
//! a display nobody looks at, never to the user's desktop. Opt-in
//! (`--ignored`), since they need `Xvfb`:
//!
//! `cargo test --lib automation::x11 -- --ignored --test-threads=1`

use std::ffi::CString;
use std::os::raw::{c_long, c_uchar};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::automation::clicker::{ClickerRun, Outcome};
use crate::automation::input::Output;
use crate::automation::model::{
    Action, ActionKind, ClickTarget, ClickerConfig, Macro, PlaybackSettings, PointOrder, TargetMode,
};
use crate::automation::player::PlaybackRun;
use crate::automation::timing::{Control, RealClock, Rng, StopReason};

/// A private Xvfb, killed when dropped.
struct Xvfb {
    child: Child,
    name: CString,
}

impl Xvfb {
    /// Like tests/desktop/virtual.sh: the abstract socket only (no TCP, no
    /// file in /tmp/.X11-unix, which is read-only under WSLg), and
    /// `-displayfd` to know when it accepts clients. Taking the number is
    /// the claim: if Xvfb exits first, the next number is tried.
    fn start() -> Self {
        use std::io::{BufRead, BufReader};
        for n in (40..100).rev() {
            let mut child = Command::new("Xvfb")
                .args([
                    &format!(":{n}"),
                    "-screen",
                    "0",
                    "1280x800x24",
                    "-nolisten",
                    "tcp",
                    "-nolisten",
                    "unix",
                    "-displayfd",
                    "1",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("Xvfb must be installed for these tests");
            let mut line = String::new();
            let out = child.stdout.take().unwrap();
            let read = BufReader::new(out).read_line(&mut line);
            if read.is_ok_and(|n| n > 0) && line.trim() == n.to_string() {
                return Self {
                    child,
                    name: CString::new(format!(":{n}")).unwrap(),
                };
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("Xvfb did not start on any display in :40-:99");
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A window on the private display that records the events it receives.
struct Target {
    conn: Conn,
    window: c_ulong,
}

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Press(c_uint, i32, i32),
    Release(c_uint),
    Key(c_uint),
}

impl Target {
    fn new(name: &CStr) -> Self {
        let conn = Conn::open(Some(name)).unwrap();
        let x = &conn.l.x;
        let root = conn.root();
        // SAFETY: an open display.
        let window = unsafe {
            let w = (x.XCreateSimpleWindow)(conn.d, root, 100, 100, 400, 300, 0, 0, 0);
            let title = CString::new("sotyware test target").unwrap();
            (x.XStoreName)(conn.d, w, title.as_ptr());
            (x.XSelectInput)(
                conn.d,
                w,
                xlib::ButtonPressMask
                    | xlib::ButtonReleaseMask
                    | xlib::KeyPressMask
                    | xlib::KeyReleaseMask,
            );
            (x.XMapRaised)(conn.d, w);
            (x.XSync)(conn.d, xlib::False);
            w
        };
        std::thread::sleep(Duration::from_millis(100));
        // SAFETY: an open display and a mapped window.
        unsafe {
            (x.XSetInputFocus)(conn.d, window, xlib::RevertToParent, 0);
            (x.XSync)(conn.d, xlib::False);
        }
        Self { conn, window }
    }

    fn set_pid(&mut self, pid: Option<u32>) {
        let atom = self.conn.atom("_NET_WM_PID");
        let x = &self.conn.l.x;
        // SAFETY: an open display and the test's own window.
        unsafe {
            match pid {
                Some(pid) => {
                    let value: [c_long; 1] = [c_long::from(pid)];
                    (x.XChangeProperty)(
                        self.conn.d,
                        self.window,
                        atom,
                        xlib::XA_CARDINAL,
                        32,
                        xlib::PropModeReplace,
                        value.as_ptr().cast::<c_uchar>(),
                        1,
                    );
                }
                None => {
                    (x.XDeleteProperty)(self.conn.d, self.window, atom);
                }
            }
            (x.XSync)(self.conn.d, xlib::False);
        }
    }

    /// Every event received within `wait`.
    fn events(&self, wait: Duration) -> Vec<Seen> {
        let x = &self.conn.l.x;
        let until = Instant::now() + wait;
        let mut out = Vec::new();
        while Instant::now() < until {
            // SAFETY: an open display; zeroed union filled by XNextEvent.
            while unsafe { (x.XPending)(self.conn.d) } > 0 {
                let mut ev: xlib::XEvent = unsafe { std::mem::zeroed() };
                unsafe { (x.XNextEvent)(self.conn.d, &mut ev) };
                match ev.get_type() {
                    xlib::ButtonPress => {
                        let b = xlib::XButtonEvent::from(ev);
                        out.push(Seen::Press(b.button, b.x, b.y));
                    }
                    xlib::ButtonRelease => {
                        out.push(Seen::Release(xlib::XButtonEvent::from(ev).button));
                    }
                    xlib::KeyPress => out.push(Seen::Key(xlib::XKeyEvent::from(ev).keycode)),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }
}

fn key(code: &str) -> Key {
    Key::from_code(code).unwrap()
}

/// What the X server considers held (XQueryKeymap).
fn key_down(conn: &Conn, k: Key) -> bool {
    let mut map = [0 as c_char; 32];
    // SAFETY: a 32-byte keymap buffer.
    unsafe { (conn.l.x.XQueryKeymap)(conn.d, map.as_mut_ptr()) };
    let code = usize::from(k.info().x11);
    map[code / 8] as u8 & (1 << (code % 8)) != 0
}

#[test]
#[ignore = "starts a private Xvfb display; run with --ignored"]
fn the_x11_backend_sends_queries_records_and_grabs_on_a_private_display() {
    let xvfb = Xvfb::start();
    let mut target = Target::new(&xvfb.name);
    let pressed = Arc::new(Mutex::new(Vec::<u32>::new()));
    let p2 = Arc::clone(&pressed);
    let backend = X11Backend::open(
        Some(xvfb.name.clone()),
        Arc::new(move |id| p2.lock().unwrap().push(id)),
    )
    .unwrap();

    // Monitors: one Xvfb screen.
    let monitors = backend.monitors().unwrap();
    assert_eq!(monitors.len(), 1);
    assert_eq!(monitors[0].bounds, Rect::new(0, 0, 1280, 800));

    // A click at a point of the target window.
    backend
        .send(&[
            Synth::Move(Point::new(300, 250)),
            Synth::Button(MouseButton::Left, Press::Down),
            Synth::Button(MouseButton::Left, Press::Up),
        ])
        .unwrap();
    assert_eq!(backend.cursor().unwrap(), Point::new(300, 250));
    assert_eq!(
        target.events(Duration::from_millis(200)),
        [Seen::Press(1, 200, 150), Seen::Release(1)]
    );

    // Keys go to the focused window, by physical key.
    backend
        .send(&[
            Synth::Key(key("KeyA"), Press::Down),
            Synth::Key(key("KeyA"), Press::Up),
        ])
        .unwrap();
    assert_eq!(target.events(Duration::from_millis(200)), [Seen::Key(38)]);
    assert_eq!(backend.key_name(key("KeyA")).as_deref(), Some("A"));

    // Two notches up, one right.
    backend
        .send(&[
            Synth::Wheel { dx: 0, dy: 240 },
            Synth::Wheel { dx: 120, dy: 0 },
        ])
        .unwrap();
    let wheel = target.events(Duration::from_millis(200));
    assert_eq!(
        wheel
            .iter()
            .filter(|e| matches!(e, Seen::Press(4, ..)))
            .count(),
        2,
        "{wheel:?}"
    );
    assert_eq!(
        wheel
            .iter()
            .filter(|e| matches!(e, Seen::Press(7, ..)))
            .count(),
        1
    );

    // The active window (no window manager: the input focus).
    let fg = backend.foreground().unwrap();
    assert_eq!(fg.title, "sotyware test target");
    assert_eq!(fg.client, Rect::new(100, 100, 400, 300));

    // sotyware's own window is told by its _NET_WM_PID.
    assert!(!backend.own_window_at(Point::new(300, 250)));
    target.set_pid(Some(std::process::id()));
    assert!(backend.own_window_at(Point::new(300, 250)));
    assert!(!backend.own_window_at(Point::new(1000, 700)));
    assert!(backend.own_window_active());
    assert_eq!(
        backend.foreground().unwrap().process,
        process_name(std::process::id())
    );
    target.set_pid(None);
    assert!(!backend.own_window_at(Point::new(300, 250)));

    // Shortcuts: F6 fires once per press; a key another client holds is
    // refused.
    backend
        .register_hotkey(7, Chord::parse("F6").unwrap())
        .unwrap();
    let user = Conn::open(Some(&xvfb.name)).unwrap();
    let press = |k: &str, down: bool| {
        // SAFETY: the test's own connection.
        unsafe {
            (user.l.tst.XTestFakeKeyEvent)(
                user.d,
                c_uint::from(key(k).info().x11),
                c_int::from(down),
                0,
            );
            (user.l.x.XSync)(user.d, xlib::False);
        }
    };
    press("F6", true);
    press("F6", true); // auto-repeat
    press("F6", false);
    let until = Instant::now() + Duration::from_secs(2);
    while pressed.lock().unwrap().is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(*pressed.lock().unwrap(), [7]);
    // With NumLock on (Mod2), the shortcut still fires.
    press("NumLock", true);
    press("NumLock", false);
    press("F6", true);
    press("F6", false);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(*pressed.lock().unwrap(), [7, 7]);
    press("NumLock", true);
    press("NumLock", false);
    // Another client's grab.
    // SAFETY: the test's own connection.
    unsafe {
        let root = user.root();
        (user.l.x.XGrabKey)(
            user.d,
            c_int::from(key("F7").info().x11),
            xlib::ControlMask,
            root,
            xlib::False,
            xlib::GrabModeAsync,
            xlib::GrabModeAsync,
        );
        (user.l.x.XSync)(user.d, xlib::False);
    }
    let refused = backend.register_hotkey(8, Chord::parse("Ctrl+F7").unwrap());
    assert_eq!(
        refused,
        Err("another program uses this shortcut".to_owned())
    );
    backend.unregister_hotkey(7);
    std::thread::sleep(Duration::from_millis(100));
    press("F6", true);
    press("F6", false);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(pressed.lock().unwrap().len(), 2, "unregistered");

    // Recording: what "the user" does arrives as raw events.
    let (tx, rx) = std::sync::mpsc::channel();
    backend.start_capture(tx).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: the test's own connection.
    unsafe {
        (user.l.tst.XTestFakeMotionEvent)(user.d, -1, 50, 60, 0);
        (user.l.tst.XTestFakeButtonEvent)(user.d, 3, 1, 0);
        (user.l.tst.XTestFakeButtonEvent)(user.d, 3, 0, 0);
        (user.l.tst.XTestFakeKeyEvent)(user.d, c_uint::from(key("KeyB").info().x11), 1, 0);
        (user.l.tst.XTestFakeKeyEvent)(user.d, c_uint::from(key("KeyB").info().x11), 0, 0);
        (user.l.x.XSync)(user.d, xlib::False);
    }
    std::thread::sleep(Duration::from_millis(300));
    backend.stop_capture();
    let got: Vec<RawKind> = rx.try_iter().map(|e| e.kind).collect();
    let at = Point::new(50, 60);
    for want in [
        RawKind::Move(at),
        RawKind::Button(MouseButton::Right, Press::Down, at),
        RawKind::Button(MouseButton::Right, Press::Up, at),
        RawKind::Key(key("KeyB"), Press::Down),
        RawKind::Key(key("KeyB"), Press::Up),
    ] {
        assert!(got.contains(&want), "{want:?} not in {got:?}");
    }
    // Stopped: nothing more arrives (the sender is gone).
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    let _ = target.events(Duration::from_millis(100));

    // The autoclicker engine, for real: 25 clicks at 4 ms into the window.
    let backend: Arc<dyn Backend> = Arc::new(backend);
    let clock = RealClock::new();
    let control = Control::new();
    let config = ClickerConfig {
        interval_ms: 4.0,
        max_clicks: Some(25),
        target: ClickTarget {
            mode: TargetMode::Points,
            points: vec![Point::new(150, 120)],
            order: PointOrder::Sequential,
            area: None,
            spread: 0,
        },
        ..ClickerConfig::default()
    };
    let started = Instant::now();
    let mut out = Output::new(Arc::clone(&backend));
    let outcome = ClickerRun {
        config: &config,
        start_delay: Duration::ZERO,
        clock: clock.as_ref(),
        control: &control,
        rng: Rng::new(1),
        failsafe: true,
        monitors: monitors.clone(),
        report: &|_| {},
    }
    .run(&mut out);
    let took = started.elapsed();
    drop(out);
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");
    let seen = target.events(Duration::from_millis(300));
    assert_eq!(
        seen.iter()
            .filter(|e| **e == Seen::Press(1, 50, 20))
            .count(),
        25,
        "{seen:?}"
    );
    // 24 intervals of 4 ms: about 96 ms, generously bounded.
    assert!(
        took >= Duration::from_millis(90) && took < Duration::from_secs(2),
        "{took:?}"
    );

    // Playback holding Shift, stopped from another thread: Shift is up
    // afterwards.
    let mut m = Macro::new("m-x11".into(), "Hold".into(), 0);
    m.actions = vec![
        Action::new(
            0,
            ActionKind::Key {
                key: key("ShiftLeft"),
                state: Press::Down,
            },
        ),
        Action::new(60_000, ActionKind::Wait),
    ];
    let control = Control::new();
    let stopper = Arc::clone(&control);
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        stopper.stop(StopReason::User("test".into()));
    });
    let observer = Conn::open(Some(&xvfb.name)).unwrap();
    let settings = PlaybackSettings::default();
    let mut out = Output::new(Arc::clone(&backend));
    let (checker, observer) = std::thread::scope(|s| {
        let h = s.spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            (key_down(&observer, key("ShiftLeft")), observer)
        });
        let outcome = PlaybackRun {
            macro_: &m,
            settings: &settings,
            start_delay: Duration::ZERO,
            clock: clock.as_ref(),
            control: &control,
            rng: Rng::new(2),
            failsafe: false,
            monitors: monitors.clone(),
            report: &|_| {},
        }
        .run(&mut out);
        assert!(matches!(outcome, Outcome::Stopped(StopReason::User(_))));
        h.join().unwrap()
    });
    drop(out);
    let _ = t.join();
    assert!(checker, "Shift was held during playback");
    assert!(!key_down(&observer, key("ShiftLeft")), "Shift was released");
    backend.shutdown();
}

#[test]
#[ignore = "starts a private Xvfb display; run with --ignored"]
fn the_x11_shortcut_thread_ends_cleanly() {
    let xvfb = Xvfb::start();
    let count = Arc::new(AtomicU32::new(0));
    let c2 = Arc::clone(&count);
    let backend = X11Backend::open(
        Some(xvfb.name.clone()),
        Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    for i in 0..20 {
        backend
            .register_hotkey(
                i,
                Chord::parse(&format!("Ctrl+Alt+F{}", i % 12 + 1)).unwrap(),
            )
            .ok();
    }
    let started = Instant::now();
    backend.shutdown();
    assert!(started.elapsed() < Duration::from_secs(2));
    // Registering after shutdown fails instead of hanging.
    assert!(
        backend
            .register_hotkey(99, Chord::parse("F9").unwrap())
            .is_err()
    );
}

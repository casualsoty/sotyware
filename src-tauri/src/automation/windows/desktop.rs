//! Integration test of the Windows backend on the **real desktop**:
//! opt-in (`--ignored`), never in CI, because it moves the pointer, clicks
//! and presses keys. Everything goes to a topmost window the test creates:
//! before each click the test checks that this window is under the point
//! (otherwise it fails without sending anything), keys are sent only while
//! this window has the keyboard, and the pointer is put back at the end.
//! Run it on a desktop nobody is using at that moment:
//!
//! `cargo test --lib automation::windows::desktop -- --ignored --test-threads=1`

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, TranslateMessage, WM_CLOSE, WM_DESTROY,
    WM_KEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use super::*;
use crate::automation::clicker::{ClickerRun, Outcome};
use crate::automation::input::Output;
use crate::automation::model::{
    Action, ActionKind, ClickTarget, ClickerConfig, Macro, PlaybackSettings, PointOrder, TargetMode,
};
use crate::automation::player::PlaybackRun;
use crate::automation::timing::{Control, RealClock, Rng, StopReason};

static LEFT: AtomicU32 = AtomicU32::new(0);
static RIGHT: AtomicU32 = AtomicU32::new(0);
static KEY_A: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            LEFT.fetch_add(1, Ordering::SeqCst);
        }
        WM_RBUTTONDOWN => {
            RIGHT.fetch_add(1, Ordering::SeqCst);
        }
        WM_KEYDOWN if wparam == 0x41 => {
            KEY_A.fetch_add(1, Ordering::SeqCst);
        }
        WM_CLOSE => {
            // SAFETY: the test's own window.
            unsafe { DestroyWindow(hwnd) };
            return 0;
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        _ => {}
    }
    // SAFETY: the default handling of every other message.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A topmost 400×300 window at (200, 200) on its own thread.
struct TestWindow {
    hwnd: HWND,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestWindow {
    fn open() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let class: Vec<u16> = "sotyware-automation-test\0".encode_utf16().collect();
            // SAFETY: registers and creates the test's own window class and
            // window, then runs its message loop.
            unsafe {
                let module = GetModuleHandleW(std::ptr::null());
                let wc = WNDCLASSW {
                    style: 0,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: module,
                    hIcon: std::ptr::null_mut(),
                    hCursor: std::ptr::null_mut(),
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: class.as_ptr(),
                };
                RegisterClassW(&wc);
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_POPUP | WS_VISIBLE,
                    200,
                    200,
                    400,
                    300,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    module,
                    std::ptr::null(),
                );
                SetForegroundWindow(hwnd);
                let _ = tx.send(hwnd as isize);
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
        let hwnd = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("test window") as HWND;
        std::thread::sleep(Duration::from_millis(400));
        Self {
            hwnd,
            thread: Some(thread),
        }
    }

    fn under(&self, p: Point) -> bool {
        // SAFETY: a plain query.
        let w = unsafe { WindowFromPoint(POINT { x: p.x, y: p.y }) };
        w == self.hwnd
    }

    fn has_keyboard(&self) -> bool {
        // SAFETY: a plain query.
        unsafe { GetForegroundWindow() == self.hwnd }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: closes the test's own window, which ends its thread.
        unsafe { PostMessageW(self.hwnd, WM_CLOSE, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !f() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Input as "the user" (no sotyware marker), for the hooks to record.
fn as_user(inputs: &[INPUT]) {
    let mine: Vec<INPUT> = inputs
        .iter()
        .map(|i| {
            let mut i = *i;
            // Writing a union field is safe; the member matches the type.
            if i.r#type == INPUT_MOUSE {
                i.Anonymous.mi.dwExtraInfo = 0;
            } else {
                i.Anonymous.ki.dwExtraInfo = 0;
            }
            i
        })
        .collect();
    send_inputs(&mine).unwrap();
}

#[test]
#[ignore = "moves the real pointer and clicks into its own window; run with --ignored on an idle desktop"]
fn the_windows_backend_sends_records_and_grabs_on_the_real_desktop() {
    let saved = {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: an out pointer.
        unsafe { GetCursorPos(&mut p) };
        p
    };
    let window = TestWindow::open();
    let center = Point::new(400, 350);
    assert!(
        window.under(center),
        "the test window is not under {center:?}; nothing was sent"
    );
    let pressed = Arc::new(Mutex::new(Vec::<u32>::new()));
    let p2 = Arc::clone(&pressed);
    let backend = WindowsBackend::open(Arc::new(move |id| p2.lock().unwrap().push(id))).unwrap();

    let monitors = backend.monitors().unwrap();
    assert!(monitors.iter().any(|m| m.primary));
    assert!(
        backend.own_window_at(center),
        "the test window belongs to this process"
    );

    // Moves land on the exact pixel; clicks reach the window.
    backend.send(&[Synth::Move(center)]).unwrap();
    assert_eq!(backend.cursor().unwrap(), center);
    let (l0, r0) = (LEFT.load(Ordering::SeqCst), RIGHT.load(Ordering::SeqCst));
    backend
        .send(&[
            Synth::Button(MouseButton::Left, Press::Down),
            Synth::Button(MouseButton::Left, Press::Up),
            Synth::Button(MouseButton::Right, Press::Down),
            Synth::Button(MouseButton::Right, Press::Up),
        ])
        .unwrap();
    wait_for("the clicks", || {
        LEFT.load(Ordering::SeqCst) == l0 + 1 && RIGHT.load(Ordering::SeqCst) == r0 + 1
    });

    // The hooks record the user's input, never sotyware's own.
    let (tx, rx) = mpsc::channel();
    backend.start_capture(tx).unwrap();
    let user_point = Point::new(450, 360);
    assert!(window.under(user_point));
    let virt = crate::automation::model::virtual_bounds(&monitors).unwrap();
    as_user(&[
        mouse(
            normalise(user_point.x, virt.x, virt.width),
            normalise(user_point.y, virt.y, virt.height),
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        button(MouseButton::Right, Press::Down),
        button(MouseButton::Right, Press::Up),
    ]);
    backend.send(&[Synth::Move(center)]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    backend.stop_capture();
    let got: Vec<RawKind> = rx.try_iter().map(|e| e.kind).collect();
    assert!(
        got.contains(&RawKind::Button(
            MouseButton::Right,
            Press::Down,
            user_point
        )),
        "{got:?}"
    );
    assert!(got.contains(&RawKind::Button(MouseButton::Right, Press::Up, user_point)));
    assert!(
        !got.contains(&RawKind::Move(center)),
        "sotyware's own move was recorded: {got:?}"
    );

    // The autoclicker engine, for real: 20 clicks at 5 ms into the window.
    let backend: Arc<dyn Backend> = Arc::new(backend);
    let clock = RealClock::new();
    let config = ClickerConfig {
        interval_ms: 5.0,
        max_clicks: Some(20),
        target: ClickTarget {
            mode: TargetMode::Points,
            points: vec![center],
            order: PointOrder::Sequential,
            area: None,
            spread: 0,
        },
        ..ClickerConfig::default()
    };
    let before = LEFT.load(Ordering::SeqCst);
    let control = Control::new();
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
    drop(out);
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");
    wait_for("20 clicks", || LEFT.load(Ordering::SeqCst) == before + 20);

    // Keyboard, shortcuts and released keys: only while the test window
    // has the keyboard (Windows may refuse it the focus).
    if window.has_keyboard() {
        let a0 = KEY_A.load(Ordering::SeqCst);
        let k = |c: &str| Key::from_code(c).unwrap();
        backend
            .send(&[
                Synth::Key(k("KeyA"), Press::Down),
                Synth::Key(k("KeyA"), Press::Up),
            ])
            .unwrap();
        wait_for("the key", || KEY_A.load(Ordering::SeqCst) == a0 + 1);

        let chord = Chord::parse("Ctrl+Alt+Shift+F24").unwrap();
        backend.register_hotkey(9, chord).unwrap();
        as_user(&[
            keyboard(k("ControlLeft"), Press::Down),
            keyboard(k("AltLeft"), Press::Down),
            keyboard(k("ShiftLeft"), Press::Down),
            keyboard(k("F24"), Press::Down),
            keyboard(k("F24"), Press::Up),
            keyboard(k("ShiftLeft"), Press::Up),
            keyboard(k("AltLeft"), Press::Up),
            keyboard(k("ControlLeft"), Press::Up),
        ]);
        wait_for("the shortcut", || pressed.lock().unwrap().contains(&9));
        // Taken by another registration: refused with the reason.
        let taken = Chord::parse("Ctrl+Alt+Shift+F23").unwrap();
        let (held_tx, held_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let _ = held_tx.send(register(777, taken));
            let _ = done_rx.recv();
            // SAFETY: this thread's own hotkey.
            unsafe { UnregisterHotKey(std::ptr::null_mut(), 777) };
        });
        held_rx.recv().unwrap().unwrap();
        assert_eq!(
            backend.register_hotkey(10, taken),
            Err("another program uses this shortcut".to_owned())
        );
        let _ = done_tx.send(());
        holder.join().unwrap();
        backend.unregister_hotkey(9);

        // A playback holding Shift, stopped: Shift is up afterwards.
        let mut m = Macro::new("m-win".into(), "Hold".into(), 0);
        m.actions = vec![
            Action::new(
                0,
                ActionKind::Key {
                    key: k("ShiftLeft"),
                    state: Press::Down,
                },
            ),
            Action::new(60_000, ActionKind::Wait),
        ];
        let control = Control::new();
        let stopper = Arc::clone(&control);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.stop(StopReason::User("test".into()));
        });
        let settings = PlaybackSettings::default();
        let mut out = Output::new(Arc::clone(&backend));
        PlaybackRun {
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
        drop(out);
        t.join().unwrap();
        // SAFETY: a plain query of the key state.
        let shift = unsafe { GetAsyncKeyState(0xA0) } as u16;
        assert_eq!(shift & 0x8000, 0, "Left Shift was released");
    } else {
        eprintln!("keyboard checks skipped: Windows did not give the test window the keyboard");
    }

    backend.shutdown();
    drop(window);
    // SAFETY: puts the pointer back where it was.
    unsafe { SetCursorPos(saved.x, saved.y) };
}

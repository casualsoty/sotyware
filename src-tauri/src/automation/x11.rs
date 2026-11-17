//! The X11 backend (ADR-050): XTest sends input, the RECORD extension
//! captures the user's input for the recorder, XGrabKey makes the global
//! shortcuts, RandR reports the monitors and the EWMH properties
//! (`_NET_ACTIVE_WINDOW`, `_NET_WM_PID`, `_NET_WM_NAME`) the active
//! window.
//!
//! Through `x11-dl` (already in the tree through tao and wry), which loads
//! libX11, libXtst and libXrandr when the feature starts: nothing is
//! linked at build time. Each X connection is used by one thread at a
//! time: the input connection behind a mutex, the shortcut connection only
//! by its own thread (which waits in `poll` on the connection and a wake
//! pipe), and the two RECORD connections by the capture thread and its
//! stopper.
//!
//! **Errors.** Xlib reports request errors (a window that closed meanwhile,
//! a shortcut another program grabbed) to one process-wide handler, whose
//! default ends the process. Every call on sotyware's own connections
//! that can fail runs inside [`Conn::trapped`], which puts sotyware's
//! handler in place first (GTK may have replaced it) and syncs, so the
//! error is collected for that call; errors of other connections go to
//! the handler that was there before.
//!
//! **Wayland.** A Wayland desktop does not let applications send or record
//! other programs' input, so on a Wayland session the backend is not
//! offered at all (XWayland would reach X11 programs only).

use std::ffi::{CStr, CString};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_ulong};
use std::ptr;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11_dl::{xlib, xrandr, xrecord, xtest};

use super::input::{Backend, HotkeySink, RawEvent, RawKind, Support, Synth};
use super::keys::{Chord, Key, KeyClass};
use super::model::{Monitor, MouseButton, Point, Press, Rect, WindowInfo};

struct Libs {
    x: xlib::Xlib,
    tst: xtest::Xf86vmode,
    rec: Option<xrecord::Xf86vmode>,
    randr: Option<xrandr::Xrandr>,
}

fn libs() -> Result<&'static Libs, String> {
    static LIBS: OnceLock<Result<Libs, String>> = OnceLock::new();
    LIBS.get_or_init(|| {
        let x = xlib::Xlib::open().map_err(|e| format!("libX11 cannot be loaded: {e}"))?;
        let tst = xtest::Xf86vmode::open()
            .map_err(|e| format!("libXtst cannot be loaded (install libxtst6): {e}"))?;
        Ok(Libs {
            x,
            tst,
            rec: xrecord::Xf86vmode::open().ok(),
            randr: xrandr::Xrandr::open().ok(),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn unavailable(note: &str) -> Support {
    Support {
        available: false,
        backend: "none",
        note: Some(note.to_owned()),
    }
}

/// Whether this session can use the X11 backend.
pub fn support() -> Support {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"));
    if wayland {
        return unavailable(
            "This desktop session uses Wayland, which does not let applications send or record input for other programs. Sign in to an X11 (Xorg) session to use the autoclicker.",
        );
    }
    if std::env::var_os("DISPLAY").is_none_or(|v| v.is_empty()) {
        return unavailable("No X11 display is available.");
    }
    match libs() {
        Ok(_) => Support {
            available: true,
            backend: "x11",
            note: None,
        },
        Err(e) => unavailable(&e),
    }
}

pub fn open(sink: HotkeySink) -> Result<Arc<dyn Backend>, String> {
    X11Backend::open(None, sink).map(|b| Arc::new(b) as Arc<dyn Backend>)
}

// ----------------------------------------------------------- errors

type ErrorHandler = unsafe extern "C" fn(*mut xlib::Display, *mut xlib::XErrorEvent) -> c_int;

/// sotyware's connections and the first error each got since it was last
/// asked.
static OURS: Mutex<Vec<(usize, Option<u8>)>> = Mutex::new(Vec::new());
/// The handler that was in place before sotyware's.
static PREVIOUS: Mutex<Option<ErrorHandler>> = Mutex::new(None);

unsafe extern "C" fn on_error(d: *mut xlib::Display, e: *mut xlib::XErrorEvent) -> c_int {
    // SAFETY: Xlib passes a valid event for the duration of the call.
    let code = unsafe { (*e).error_code };
    if let Ok(mut ours) = OURS.lock()
        && let Some(slot) = ours.iter_mut().find(|(p, _)| *p == d as usize)
    {
        slot.1.get_or_insert(code);
        return 0;
    }
    let previous = PREVIOUS.lock().ok().and_then(|p| *p);
    match previous {
        // SAFETY: forwarding Xlib's own arguments to the handler Xlib had.
        Some(f) => unsafe { f(d, e) },
        None => 0,
    }
}

fn assert_handler(l: &Libs) {
    // SAFETY: plain function pointer exchange in Xlib.
    let previous = unsafe { (l.x.XSetErrorHandler)(Some(on_error)) };
    let ours: ErrorHandler = on_error;
    if let Some(p) = previous
        && !std::ptr::fn_addr_eq(p, ours)
        && let Ok(mut slot) = PREVIOUS.lock()
    {
        *slot = Some(p);
    }
}

// -------------------------------------------------------- connection

/// One X connection of sotyware's.
struct Conn {
    l: &'static Libs,
    d: *mut xlib::Display,
    atoms: Vec<(&'static str, c_ulong)>,
}

// SAFETY: a connection is used by one thread at a time (behind a mutex or
// owned by one thread); Xlib needs no more without XInitThreads.
unsafe impl Send for Conn {}

impl Conn {
    fn open(name: Option<&CStr>) -> Result<Self, String> {
        let l = libs()?;
        assert_handler(l);
        // SAFETY: a null name means $DISPLAY.
        let d = unsafe { (l.x.XOpenDisplay)(name.map_or(ptr::null(), CStr::as_ptr)) };
        if d.is_null() {
            return Err("the X11 display cannot be opened".to_owned());
        }
        if let Ok(mut ours) = OURS.lock() {
            ours.push((d as usize, None));
        }
        Ok(Self {
            l,
            d,
            atoms: Vec::new(),
        })
    }

    fn root(&self) -> c_ulong {
        // SAFETY: an open display.
        unsafe { (self.l.x.XDefaultRootWindow)(self.d) }
    }

    /// Runs `f`, syncs, and returns the first X error it caused.
    fn trapped<T>(&self, f: impl FnOnce() -> T) -> (T, Option<u8>) {
        assert_handler(self.l);
        let value = f();
        // SAFETY: an open display.
        unsafe { (self.l.x.XSync)(self.d, xlib::False) };
        let error = OURS.lock().ok().and_then(|mut ours| {
            ours.iter_mut()
                .find(|(p, _)| *p == self.d as usize)
                .and_then(|slot| slot.1.take())
        });
        (value, error)
    }

    fn atom(&mut self, name: &'static str) -> c_ulong {
        if let Some((_, a)) = self.atoms.iter().find(|(n, _)| *n == name) {
            return *a;
        }
        let c = CString::new(name).unwrap_or_default();
        // SAFETY: an open display and a NUL-terminated name.
        let a = unsafe { (self.l.x.XInternAtom)(self.d, c.as_ptr(), xlib::False) };
        self.atoms.push((name, a));
        a
    }

    /// A window property's raw items (format 8 as bytes, 32 as longs).
    fn property(
        &mut self,
        w: c_ulong,
        name: &'static str,
    ) -> Option<(c_int, Vec<c_ulong>, Vec<u8>)> {
        let atom = self.atom(name);
        let (mut kind, mut format, mut n, mut after) = (0, 0, 0, 0);
        let mut data: *mut c_uchar = ptr::null_mut();
        let (status, error) = self.trapped(|| {
            // SAFETY: valid out pointers; the data is freed below.
            unsafe {
                (self.l.x.XGetWindowProperty)(
                    self.d,
                    w,
                    atom,
                    0,
                    4096,
                    xlib::False,
                    xlib::AnyPropertyType as c_ulong,
                    &mut kind,
                    &mut format,
                    &mut n,
                    &mut after,
                    &mut data,
                )
            }
        });
        if status != 0 || error.is_some() || data.is_null() {
            return None;
        }
        let out = match format {
            // SAFETY: Xlib returns `n` items of the given format.
            32 => {
                let items =
                    unsafe { std::slice::from_raw_parts(data.cast::<c_ulong>(), n as usize) };
                (32, items.to_vec(), Vec::new())
            }
            8 => {
                let bytes = unsafe { std::slice::from_raw_parts(data, n as usize) };
                (8, Vec::new(), bytes.to_vec())
            }
            f => (f, Vec::new(), Vec::new()),
        };
        // SAFETY: allocated by XGetWindowProperty.
        unsafe { (self.l.x.XFree)(data.cast()) };
        Some(out)
    }

    fn window_pid(&mut self, w: c_ulong) -> Option<u32> {
        self.property(w, "_NET_WM_PID")
            .and_then(|(_, longs, _)| longs.first().copied())
            .and_then(|p| u32::try_from(p).ok())
    }

    fn window_title(&mut self, w: c_ulong) -> String {
        for name in ["_NET_WM_NAME", "WM_NAME"] {
            if let Some((8, _, bytes)) = self.property(w, name) {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
        }
        String::new()
    }

    /// The window that has the keyboard: `_NET_ACTIVE_WINDOW`, else the
    /// input focus.
    fn active(&mut self) -> Option<c_ulong> {
        let root = self.root();
        if let Some((32, longs, _)) = self.property(root, "_NET_ACTIVE_WINDOW")
            && let Some(&w) = longs.first()
            && w != 0
        {
            return Some(w);
        }
        let (mut focus, mut revert) = (0, 0);
        // SAFETY: valid out pointers.
        unsafe { (self.l.x.XGetInputFocus)(self.d, &mut focus, &mut revert) };
        (focus > 1).then_some(focus)
    }

    /// `w` or one of its descendants (a window manager's frame holds the
    /// client) carries `_NET_WM_PID` = `pid`; `w` itself if it is the client.
    fn client_of(&mut self, w: c_ulong, depth: u32) -> Option<(c_ulong, u32)> {
        if let Some(pid) = self.window_pid(w) {
            return Some((w, pid));
        }
        if depth == 0 {
            return None;
        }
        for child in self.children(w).into_iter().rev() {
            if let Some(found) = self.client_of(child, depth - 1) {
                return Some(found);
            }
        }
        None
    }

    fn children(&mut self, w: c_ulong) -> Vec<c_ulong> {
        let (mut root, mut parent) = (0, 0);
        let mut list: *mut c_ulong = ptr::null_mut();
        let mut n: c_uint = 0;
        let (ok, error) = self.trapped(|| {
            // SAFETY: valid out pointers; the list is freed below.
            unsafe { (self.l.x.XQueryTree)(self.d, w, &mut root, &mut parent, &mut list, &mut n) }
        });
        if ok == 0 || error.is_some() || list.is_null() {
            return Vec::new();
        }
        // SAFETY: `n` windows from XQueryTree.
        let out = unsafe { std::slice::from_raw_parts(list, n as usize) }.to_vec();
        unsafe { (self.l.x.XFree)(list.cast()) };
        out
    }

    fn client_rect(&mut self, w: c_ulong) -> Option<Rect> {
        let root = self.root();
        let (mut r, mut x, mut y, mut width, mut height, mut border, mut depth) =
            (0, 0, 0, 0, 0, 0, 0);
        let (mut ax, mut ay, mut child) = (0, 0, 0);
        let ((g, t), error) = self.trapped(|| {
            // SAFETY: valid out pointers.
            unsafe {
                (
                    (self.l.x.XGetGeometry)(
                        self.d,
                        w,
                        &mut r,
                        &mut x,
                        &mut y,
                        &mut width,
                        &mut height,
                        &mut border,
                        &mut depth,
                    ),
                    (self.l.x.XTranslateCoordinates)(
                        self.d, w, root, 0, 0, &mut ax, &mut ay, &mut child,
                    ),
                )
            }
        });
        (g != 0 && t != 0 && error.is_none() && width > 0 && height > 0)
            .then(|| Rect::new(ax, ay, width, height))
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Ok(mut ours) = OURS.lock() {
            ours.retain(|(p, _)| *p != self.d as usize);
        }
        // SAFETY: closing the display this value opened.
        unsafe { (self.l.x.XCloseDisplay)(self.d) };
    }
}

fn process_name(pid: u32) -> String {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_default()
}

fn x_button(b: MouseButton) -> c_uint {
    match b {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 8,
        MouseButton::Forward => 9,
    }
}

/// The modifiers a shortcut is made of; NumLock and CapsLock are ignored.
const MOD_MASK: c_uint = xlib::ShiftMask | xlib::ControlMask | xlib::Mod1Mask | xlib::Mod4Mask;
const IGNORED_MODS: [c_uint; 4] = [
    0,
    xlib::LockMask,
    xlib::Mod2Mask,
    xlib::LockMask | xlib::Mod2Mask,
];

fn x_mods(c: Chord) -> c_uint {
    let mut m = 0;
    if c.shift {
        m |= xlib::ShiftMask;
    }
    if c.ctrl {
        m |= xlib::ControlMask;
    }
    if c.alt {
        m |= xlib::Mod1Mask;
    }
    if c.meta {
        m |= xlib::Mod4Mask;
    }
    m
}

// ----------------------------------------------------------- shortcuts

enum HkRequest {
    Register(u32, Chord, Sender<Result<(), String>>),
    Unregister(u32),
    Quit,
}

struct HotkeyThread {
    tx: Sender<HkRequest>,
    wake: OwnedFd,
    handle: Option<JoinHandle<()>>,
}

impl HotkeyThread {
    fn start(name: Option<CString>, sink: HotkeySink) -> Result<Self, String> {
        let (read, write) =
            rustix::pipe::pipe().map_err(|e| format!("cannot create the shortcut pipe: {e}"))?;
        let (tx, rx) = mpsc::channel::<HkRequest>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let handle = std::thread::Builder::new()
            .name("sotyware-x11-hotkeys".into())
            .spawn(move || {
                let conn = match Conn::open(name.as_deref()) {
                    Ok(c) => {
                        let _ = ready_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                hotkey_loop(&conn, &rx, &read, &sink);
            })
            .map_err(|e| format!("cannot start the shortcut thread: {e}"))?;
        ready_rx
            .recv()
            .map_err(|_| "the shortcut thread ended".to_owned())??;
        Ok(Self {
            tx,
            wake: write,
            handle: Some(handle),
        })
    }

    fn request(&self, r: HkRequest) {
        let _ = self.tx.send(r);
        let _ = rustix::io::write(&self.wake, &[1]);
    }

    fn stop(&mut self) {
        self.request(HkRequest::Quit);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn hotkey_loop(conn: &Conn, rx: &mpsc::Receiver<HkRequest>, wake: &OwnedFd, sink: &HotkeySink) {
    let x = &conn.l.x;
    let root = conn.root();
    // Repeated presses of a held key come as presses only, so a held
    // shortcut fires once.
    let mut supported = 0;
    // SAFETY: an open display.
    unsafe { (x.XkbSetDetectableAutoRepeat)(conn.d, xlib::True, &mut supported) };
    // id, keycode, modifiers, held
    let mut grabs: Vec<(u32, c_int, c_uint, bool)> = Vec::new();
    // SAFETY: the connection's file descriptor lives as long as `conn`.
    let xfd = unsafe { BorrowedFd::borrow_raw((x.XConnectionNumber)(conn.d)) };
    let ungrab = |keycode: c_int, mods: c_uint| {
        for extra in IGNORED_MODS {
            // SAFETY: an open display.
            unsafe { (x.XUngrabKey)(conn.d, keycode, mods | extra, root) };
        }
    };
    loop {
        let mut fds = [
            rustix::event::PollFd::new(&xfd, rustix::event::PollFlags::IN),
            rustix::event::PollFd::new(wake, rustix::event::PollFlags::IN),
        ];
        // SAFETY: Xlib may have queued events already: check before waiting.
        let queued = unsafe { (x.XPending)(conn.d) } > 0;
        if !queued && rustix::event::poll(&mut fds, None).is_err() {
            std::thread::sleep(Duration::from_millis(5));
        }
        if fds[1].revents().contains(rustix::event::PollFlags::IN) {
            let mut buf = [0u8; 64];
            let _ = rustix::io::read(wake, &mut buf);
        }
        while let Ok(r) = rx.try_recv() {
            match r {
                HkRequest::Register(id, chord, reply) => {
                    let keycode = c_int::from(chord.key.info().x11);
                    let mods = x_mods(chord);
                    let (_, error) = conn.trapped(|| {
                        for extra in IGNORED_MODS {
                            // SAFETY: an open display; errors are trapped.
                            unsafe {
                                (x.XGrabKey)(
                                    conn.d,
                                    keycode,
                                    mods | extra,
                                    root,
                                    xlib::False,
                                    xlib::GrabModeAsync,
                                    xlib::GrabModeAsync,
                                )
                            };
                        }
                    });
                    let result = match error {
                        None => {
                            grabs.push((id, keycode, mods, false));
                            Ok(())
                        }
                        Some(code) => {
                            // Release the variants that did succeed.
                            let _ = conn.trapped(|| ungrab(keycode, mods));
                            Err(if code == xlib::BadAccess {
                                "another program uses this shortcut".to_owned()
                            } else {
                                format!("the X server refused the shortcut (error {code})")
                            })
                        }
                    };
                    let _ = reply.send(result);
                }
                HkRequest::Unregister(id) => {
                    if let Some(i) = grabs.iter().position(|g| g.0 == id) {
                        let (_, keycode, mods, _) = grabs.remove(i);
                        let still = grabs.iter().any(|g| g.1 == keycode && g.2 == mods);
                        if !still {
                            let _ = conn.trapped(|| ungrab(keycode, mods));
                        }
                    }
                }
                HkRequest::Quit => {
                    for (_, keycode, mods, _) in grabs.drain(..) {
                        let _ = conn.trapped(|| ungrab(keycode, mods));
                    }
                    return;
                }
            }
        }
        // SAFETY: an open display; events are read into a zeroed union.
        while unsafe { (x.XPending)(conn.d) } > 0 {
            let mut ev: xlib::XEvent = unsafe { std::mem::zeroed() };
            unsafe { (x.XNextEvent)(conn.d, &mut ev) };
            let kind = ev.get_type();
            if kind != xlib::KeyPress && kind != xlib::KeyRelease {
                continue;
            }
            let key = xlib::XKeyEvent::from(ev);
            let keycode = key.keycode as c_int;
            let mods = key.state & MOD_MASK;
            for g in grabs.iter_mut().filter(|g| g.1 == keycode) {
                if kind == xlib::KeyRelease {
                    g.3 = false;
                } else if g.2 == mods && !g.3 {
                    g.3 = true;
                    sink(g.0);
                }
            }
        }
    }
}

// ------------------------------------------------------------ capture

struct CaptureState {
    sink: Sender<RawEvent>,
}

unsafe extern "C" fn on_record(closure: *mut c_char, data: *mut xrecord::XRecordInterceptData) {
    let Ok(l) = libs() else { return };
    let Some(rec) = &l.rec else { return };
    // SAFETY: the closure is the capture thread's `CaptureState`, alive
    // while the context is enabled; `data` is valid until freed below.
    let (state, d) = unsafe { (&*(closure as *const CaptureState), &*data) };
    if d.category == xrecord::XRecordFromServer && !d.data.is_null() && d.data_len >= 8 {
        // SAFETY: `data_len` 4-byte units of one wire event.
        let bytes = unsafe { std::slice::from_raw_parts(d.data, d.data_len as usize * 4) };
        let kind = c_int::from(bytes[0] & 0x7F);
        let detail = bytes[1];
        let at = Point::new(
            i32::from(i16::from_ne_bytes([bytes[20], bytes[21]])),
            i32::from(i16::from_ne_bytes([bytes[22], bytes[23]])),
        );
        let raw = match kind {
            xlib::MotionNotify => Some(RawKind::Move(at)),
            xlib::ButtonPress | xlib::ButtonRelease => {
                let press = if kind == xlib::ButtonPress {
                    Press::Down
                } else {
                    Press::Up
                };
                match detail {
                    1 => Some(RawKind::Button(MouseButton::Left, press, at)),
                    2 => Some(RawKind::Button(MouseButton::Middle, press, at)),
                    3 => Some(RawKind::Button(MouseButton::Right, press, at)),
                    8 => Some(RawKind::Button(MouseButton::Back, press, at)),
                    9 => Some(RawKind::Button(MouseButton::Forward, press, at)),
                    4..=7 if press == Press::Down => Some(match detail {
                        4 => RawKind::Wheel { dx: 0, dy: 120, at },
                        5 => RawKind::Wheel {
                            dx: 0,
                            dy: -120,
                            at,
                        },
                        6 => RawKind::Wheel {
                            dx: -120,
                            dy: 0,
                            at,
                        },
                        _ => RawKind::Wheel { dx: 120, dy: 0, at },
                    }),
                    _ => None,
                }
            }
            xlib::KeyPress | xlib::KeyRelease => {
                let press = if kind == xlib::KeyPress {
                    Press::Down
                } else {
                    Press::Up
                };
                Some(match Key::from_x11(u32::from(detail)) {
                    Some(k) => RawKind::Key(k, press),
                    None => RawKind::OtherKey,
                })
            }
            _ => None,
        };
        if let Some(kind) = raw {
            let _ = state.sink.send(RawEvent {
                at: Instant::now(),
                kind,
            });
        }
    }
    // SAFETY: handed to the callback for it to free.
    unsafe { (rec.XRecordFreeData)(data) };
}

struct Capture {
    ctrl: Conn,
    context: c_ulong,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    fn start(name: Option<&CStr>, sink: Sender<RawEvent>) -> Result<Self, String> {
        let l = libs()?;
        let rec = l
            .rec
            .as_ref()
            .ok_or("recording needs the X RECORD extension's library (libXtst)")?;
        let ctrl = Conn::open(name)?;
        let data = Conn::open(name)?;
        let (mut major, mut minor) = (0, 0);
        // SAFETY: an open display.
        if unsafe { (rec.XRecordQueryVersion)(ctrl.d, &mut major, &mut minor) } == 0 {
            return Err(
                "this X server has no RECORD extension; recording is not possible".to_owned(),
            );
        }
        // SAFETY: the range is freed after the context holds a copy.
        let context = unsafe {
            let range = (rec.XRecordAllocRange)();
            if range.is_null() {
                return Err("cannot set up recording".to_owned());
            }
            (*range).device_events.first = xlib::KeyPress as c_uchar;
            (*range).device_events.last = xlib::MotionNotify as c_uchar;
            let mut clients = xrecord::XRecordAllClients;
            let mut ranges = [range];
            let ctx =
                (rec.XRecordCreateContext)(ctrl.d, 0, &mut clients, 1, ranges.as_mut_ptr(), 1);
            (l.x.XFree)(range.cast());
            (l.x.XSync)(ctrl.d, xlib::False);
            ctx
        };
        if context == 0 {
            return Err("the X server refused to record input".to_owned());
        }
        let thread = std::thread::Builder::new()
            .name("sotyware-x11-record".into())
            .spawn(move || {
                let state = Box::new(CaptureState { sink });
                let closure = Box::into_raw(state);
                // SAFETY: blocks until the context is disabled; the closure
                // outlives the call.
                unsafe {
                    (rec.XRecordEnableContext)(data.d, context, Some(on_record), closure.cast());
                    drop(Box::from_raw(closure));
                }
                drop(data);
            })
            .map_err(|e| format!("cannot start the recording thread: {e}"))?;
        Ok(Self {
            ctrl,
            context,
            thread: Some(thread),
        })
    }

    fn stop(mut self) {
        let Ok(l) = libs() else { return };
        let Some(rec) = &l.rec else { return };
        // SAFETY: the control connection created the context.
        unsafe {
            (rec.XRecordDisableContext)(self.ctrl.d, self.context);
            (l.x.XSync)(self.ctrl.d, xlib::False);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // SAFETY: disabled above, so no longer in use.
        unsafe { (rec.XRecordFreeContext)(self.ctrl.d, self.context) };
    }
}

// ------------------------------------------------------------ backend

pub struct X11Backend {
    name: Option<CString>,
    input: Mutex<Conn>,
    hotkeys: Mutex<Option<HotkeyThread>>,
    capture: Mutex<Option<Capture>>,
    pid: u32,
}

impl X11Backend {
    /// Opens display `name` (`None`: `$DISPLAY`); hotkeys call `sink`.
    pub fn open(name: Option<CString>, sink: HotkeySink) -> Result<Self, String> {
        let conn = Conn::open(name.as_deref())?;
        let (mut ev, mut err, mut major, mut minor) = (0, 0, 0, 0);
        // SAFETY: an open display.
        if unsafe {
            (conn.l.tst.XTestQueryExtension)(conn.d, &mut ev, &mut err, &mut major, &mut minor)
        } == 0
        {
            return Err("this X server has no XTEST extension; input cannot be sent".to_owned());
        }
        let hotkeys = HotkeyThread::start(name.clone(), sink)?;
        Ok(Self {
            name,
            input: Mutex::new(conn),
            hotkeys: Mutex::new(Some(hotkeys)),
            capture: Mutex::new(None),
            pid: std::process::id(),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Conn> {
        self.input.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Backend for X11Backend {
    fn send(&self, batch: &[Synth]) -> Result<(), String> {
        let c = self.conn();
        let (l, d) = (c.l, c.d);
        let (_, error) = c.trapped(|| {
            for s in batch {
                // SAFETY: an open display; errors are trapped.
                unsafe {
                    match *s {
                        Synth::Move(p) => {
                            (l.tst.XTestFakeMotionEvent)(d, -1, p.x, p.y, 0);
                        }
                        Synth::Button(b, press) => {
                            (l.tst.XTestFakeButtonEvent)(
                                d,
                                x_button(b),
                                c_int::from(press == Press::Down),
                                0,
                            );
                        }
                        Synth::Wheel { dx, dy } => {
                            let (button, amount) = if dy != 0 {
                                (if dy > 0 { 4 } else { 5 }, dy)
                            } else {
                                (if dx > 0 { 7 } else { 6 }, dx)
                            };
                            let notches = (amount.unsigned_abs() / 120).max(1);
                            for _ in 0..notches {
                                (l.tst.XTestFakeButtonEvent)(d, button, 1, 0);
                                (l.tst.XTestFakeButtonEvent)(d, button, 0, 0);
                            }
                        }
                        Synth::Key(k, press) => {
                            (l.tst.XTestFakeKeyEvent)(
                                d,
                                c_uint::from(k.info().x11),
                                c_int::from(press == Press::Down),
                                0,
                            );
                        }
                    }
                }
            }
        });
        match error {
            None => Ok(()),
            Some(code) => Err(format!("the X server refused the input (error {code})")),
        }
    }

    fn cursor(&self) -> Result<Point, String> {
        let c = self.conn();
        let root = c.root();
        let (mut r, mut child, mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0, 0, 0);
        // SAFETY: valid out pointers.
        let ok = unsafe {
            (c.l.x.XQueryPointer)(
                c.d, root, &mut r, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask,
            )
        };
        if ok == 0 {
            return Err("the pointer is on another screen".to_owned());
        }
        Ok(Point::new(rx, ry))
    }

    fn monitors(&self) -> Result<Vec<Monitor>, String> {
        let c = self.conn();
        let root = c.root();
        let mut out = Vec::new();
        if let Some(randr) = &c.l.randr {
            let mut n: c_int = 0;
            // SAFETY: valid out pointer; the list is freed below.
            let list = unsafe { (randr.XRRGetMonitors)(c.d, root, xlib::True, &mut n) };
            if !list.is_null() {
                // SAFETY: `n` monitors from XRRGetMonitors.
                let items =
                    unsafe { std::slice::from_raw_parts(list, usize::try_from(n).unwrap_or(0)) };
                for m in items {
                    if m.width > 0 && m.height > 0 {
                        out.push(Monitor {
                            bounds: Rect::new(m.x, m.y, m.width as u32, m.height as u32),
                            primary: m.primary != 0,
                        });
                    }
                }
                // SAFETY: allocated by XRRGetMonitors.
                unsafe { (randr.XRRFreeMonitors)(list) };
            }
        }
        if out.is_empty() {
            // SAFETY: an open display.
            let (w, h) = unsafe {
                let s = (c.l.x.XDefaultScreen)(c.d);
                (
                    (c.l.x.XDisplayWidth)(c.d, s),
                    (c.l.x.XDisplayHeight)(c.d, s),
                )
            };
            out.push(Monitor {
                bounds: Rect::new(0, 0, w.max(1) as u32, h.max(1) as u32),
                primary: true,
            });
        }
        if !out.iter().any(|m| m.primary) {
            out[0].primary = true;
        }
        Ok(out)
    }

    fn foreground(&self) -> Option<WindowInfo> {
        let mut c = self.conn();
        let w = c.active()?;
        let (client, pid) = c.client_of(w, 2).unwrap_or((w, 0));
        let title = c.window_title(client);
        let client_rect = c.client_rect(client)?;
        Some(WindowInfo {
            title,
            process: if pid == 0 {
                String::new()
            } else {
                process_name(pid)
            },
            client: client_rect,
        })
    }

    fn own_window_at(&self, p: Point) -> bool {
        let mut c = self.conn();
        let root = c.root();
        let (mut x, mut y, mut child) = (0, 0, 0);
        let (ok, error) = c.trapped(|| {
            // SAFETY: valid out pointers.
            unsafe {
                (c.l.x.XTranslateCoordinates)(c.d, root, root, p.x, p.y, &mut x, &mut y, &mut child)
            }
        });
        if ok == 0 || error.is_some() || child == 0 {
            return false;
        }
        c.client_of(child, 3)
            .is_some_and(|(_, pid)| pid == self.pid)
    }

    fn own_window_active(&self) -> bool {
        let mut c = self.conn();
        c.active()
            .and_then(|w| c.client_of(w, 2))
            .is_some_and(|(_, pid)| pid == self.pid)
    }

    fn key_name(&self, key: Key) -> Option<String> {
        if key.class() != KeyClass::Layout {
            return None;
        }
        let c = self.conn();
        // SAFETY: an open display.
        let sym = unsafe { (c.l.x.XkbKeycodeToKeysym)(c.d, key.info().x11, 0, 0) };
        let ch = match sym {
            0x20..=0x7E | 0xA0..=0xFF => char::from_u32(sym as u32),
            s if s & 0xFF00_0000 == 0x0100_0000 => char::from_u32((s & 0x00FF_FFFF) as u32),
            _ => None,
        }?;
        Some(ch.to_uppercase().collect())
    }

    fn register_hotkey(&self, id: u32, chord: Chord) -> Result<(), String> {
        let guard = self.hotkeys.lock().unwrap_or_else(|e| e.into_inner());
        let h = guard.as_ref().ok_or("the shortcuts are closed")?;
        let (tx, rx) = mpsc::channel();
        h.request(HkRequest::Register(id, chord, tx));
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "the X server did not answer".to_owned())?
    }

    fn unregister_hotkey(&self, id: u32) {
        if let Some(h) = self
            .hotkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            h.request(HkRequest::Unregister(id));
        }
    }

    fn start_capture(&self, sink: Sender<RawEvent>) -> Result<(), String> {
        let mut slot = self.capture.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Some(Capture::start(self.name.as_deref(), sink)?);
        Ok(())
    }

    fn stop_capture(&self) {
        if let Some(c) = self
            .capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            c.stop();
        }
    }

    fn shutdown(&self) {
        self.stop_capture();
        if let Some(mut h) = self
            .hotkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            h.stop();
        }
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;

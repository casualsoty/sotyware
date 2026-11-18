//! The Windows backend (ADR-050): `SendInput` sends input, low-level hooks
//! (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`) capture the user's input for the
//! recorder, `RegisterHotKey` makes the global shortcuts;
//! `EnumDisplayMonitors` and `GetForegroundWindow` report the screens and
//! the active window. Through `windows-sys` (already a dependency; every
//! function used is in features the tree compiles already).
//!
//! **One input thread** owns the shortcuts and the hooks: Windows delivers
//! `WM_HOTKEY` and calls low-level hooks on the thread that registered
//! them, inside its message loop. The thread only moves events on (the
//! hooks into the recorder's channel, the shortcuts into the service's
//! sink) and returns at once, so Windows never removes a hook for taking
//! too long. The hooks are installed only while a recording runs.
//!
//! **sotyware's own input** carries a marker in `dwExtraInfo`; the hooks
//! skip it, so playback is never recorded. Input other programs inject
//! (remote desktop tools) is recorded like the user's.
//!
//! **Coordinates** are physical pixels of the virtual desktop: sotyware is
//! per-monitor DPI aware (v2, set by tao), so `GetCursorPos`, the hooks'
//! points and monitor rectangles all agree. An absolute `SendInput` move
//! is normalised to 0…65535 over the virtual desktop; the pointer is then
//! checked and put on the exact pixel with `SetCursorPos` if rounding
//! missed it, before any button of the same batch.
//!
//! **Keys** are sent by scan code (`KEYEVENTF_SCANCODE`), as the keyboard
//! would: the active layout of the program that receives them decides the
//! character, and games reading scan codes see them. Pause and the media
//! keys go by virtual-key code, which is how Windows identifies them.
//!
//! **Limits** (Windows' own): input cannot reach a program running as
//! administrator unless sotyware does too (UIPI), nor the secure desktop
//! (UAC prompts, the lock screen); `SendInput` reports neither.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VSC_TO_VK_EX, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MapVirtualKeyW,
    RegisterHotKey, SendInput, UnregisterHotKey, VK_PAUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GA_ROOT, GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetMessageW, GetWindowTextW, GetWindowThreadProcessId, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, LLKHF_UP, MONITORINFOF_PRIMARY, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetCursorPos, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_APP, WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_USER,
    WM_XBUTTONDOWN, WM_XBUTTONUP, WindowFromPoint, XBUTTON1,
};

use super::input::{Backend, HotkeySink, RawEvent, RawKind, Support, Synth};
use super::keys::{Chord, Key, KeyClass};
use super::model::{Monitor, MouseButton, Point, Press, Rect, WindowInfo};

/// `dwExtraInfo` of sotyware's own input ("SOTY").
const MARKER: usize = 0x534F_5459;
/// Asks the input thread to read its request channel.
const WM_REQUEST: u32 = WM_APP + 0x51;

pub fn support() -> Support {
    Support {
        available: true,
        backend: "windows",
        note: None,
    }
}

pub fn open(sink: HotkeySink) -> Result<Arc<dyn Backend>, String> {
    WindowsBackend::open(sink).map(|b| Arc::new(b) as Arc<dyn Backend>)
}

enum Request {
    Register(u32, Chord, Sender<Result<(), String>>),
    Unregister(u32),
    StartCapture(Sender<RawEvent>, Sender<Result<(), String>>),
    StopCapture(Sender<()>),
    Quit,
}

pub struct WindowsBackend {
    thread_id: u32,
    tx: Mutex<Sender<Request>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    pid: u32,
}

thread_local! {
    /// Where the hooks send events (set on the input thread while
    /// recording; the hooks run on that thread).
    static CAPTURE: RefCell<Option<Sender<RawEvent>>> = const { RefCell::new(None) };
}

fn last_error() -> String {
    // SAFETY: reads the calling thread's last error.
    let code = unsafe { GetLastError() };
    std::io::Error::from_raw_os_error(code as i32).to_string()
}

impl WindowsBackend {
    fn open(sink: HotkeySink) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel::<u32>();
        let handle = std::thread::Builder::new()
            .name("sotyware-input".into())
            .spawn(move || input_thread(&rx, &sink, &ready_tx))
            .map_err(|e| format!("cannot start the input thread: {e}"))?;
        let thread_id = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "the input thread did not start".to_owned())?;
        Ok(Self {
            thread_id,
            tx: Mutex::new(tx),
            handle: Mutex::new(Some(handle)),
            pid: std::process::id(),
        })
    }

    fn request(&self, r: Request) -> bool {
        let sent = self
            .tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(r)
            .is_ok();
        // SAFETY: posting to a thread that has a message queue.
        sent && unsafe { PostThreadMessageW(self.thread_id, WM_REQUEST, 0, 0) } != 0
    }
}

fn input_thread(rx: &Receiver<Request>, sink: &HotkeySink, ready: &Sender<u32>) {
    let mut msg: MSG = MSG::default();
    // SAFETY: creates this thread's message queue before its id is given
    // out, so no posted message is lost.
    unsafe {
        PeekMessageW(
            &mut msg,
            std::ptr::null_mut(),
            WM_USER,
            WM_USER,
            PM_NOREMOVE,
        );
        let _ = ready.send(GetCurrentThreadId());
    }
    let mut hotkeys: Vec<i32> = Vec::new();
    let mut hooks: Option<(HHOOK, HHOOK)> = None;
    let unhook = |hooks: &mut Option<(HHOOK, HHOOK)>| {
        if let Some((m, k)) = hooks.take() {
            // SAFETY: hooks this thread installed.
            unsafe {
                UnhookWindowsHookEx(m);
                UnhookWindowsHookEx(k);
            }
        }
        CAPTURE.with(|c| *c.borrow_mut() = None);
    };
    'run: loop {
        // SAFETY: a valid MSG; the hooks are called from inside GetMessageW.
        let got = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if got == 0 || got == -1 || msg.message == WM_QUIT {
            break;
        }
        match msg.message {
            WM_HOTKEY => sink(msg.wParam as u32),
            WM_REQUEST => {
                while let Ok(r) = rx.try_recv() {
                    match r {
                        Request::Register(id, chord, reply) => {
                            let result = register(id, chord);
                            if result.is_ok() {
                                hotkeys.push(id as i32);
                            }
                            let _ = reply.send(result);
                        }
                        Request::Unregister(id) => {
                            // SAFETY: a hotkey of this thread's queue.
                            unsafe { UnregisterHotKey(std::ptr::null_mut(), id as i32) };
                            hotkeys.retain(|&h| h != id as i32);
                        }
                        Request::StartCapture(events, reply) => {
                            unhook(&mut hooks);
                            CAPTURE.with(|c| *c.borrow_mut() = Some(events));
                            // SAFETY: global low-level hooks of this module,
                            // removed by `unhook`.
                            let installed = unsafe {
                                let module = GetModuleHandleW(std::ptr::null());
                                let m = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
                                let k = SetWindowsHookExW(
                                    WH_KEYBOARD_LL,
                                    Some(keyboard_hook),
                                    module,
                                    0,
                                );
                                if m.is_null() || k.is_null() {
                                    let e = last_error();
                                    if !m.is_null() {
                                        UnhookWindowsHookEx(m);
                                    }
                                    if !k.is_null() {
                                        UnhookWindowsHookEx(k);
                                    }
                                    Err(format!(
                                        "Windows refused to let sotyware record input: {e}"
                                    ))
                                } else {
                                    Ok((m, k))
                                }
                            };
                            let _ = reply.send(installed.map(|h| {
                                hooks = Some(h);
                            }));
                            if hooks.is_none() {
                                CAPTURE.with(|c| *c.borrow_mut() = None);
                            }
                        }
                        Request::StopCapture(done) => {
                            unhook(&mut hooks);
                            let _ = done.send(());
                        }
                        Request::Quit => break 'run,
                    }
                }
            }
            _ => {}
        }
    }
    unhook(&mut hooks);
    for id in hotkeys {
        // SAFETY: hotkeys of this thread's queue.
        unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
    }
}

/// The virtual-key code of `key` on the current layout (what a press of
/// that physical key gives), for `RegisterHotKey`.
fn layout_vk(key: Key) -> u32 {
    let info = key.info();
    if info.class == KeyClass::Layout {
        // SAFETY: a pure table lookup in the current keyboard layout.
        let vk = unsafe { MapVirtualKeyW(u32::from(info.win_scan), MAPVK_VSC_TO_VK_EX) };
        if vk != 0 {
            return vk;
        }
    }
    u32::from(info.win_vk)
}

fn register(id: u32, chord: Chord) -> Result<(), String> {
    let mut mods = MOD_NOREPEAT;
    if chord.ctrl {
        mods |= MOD_CONTROL;
    }
    if chord.alt {
        mods |= MOD_ALT;
    }
    if chord.shift {
        mods |= MOD_SHIFT;
    }
    if chord.meta {
        mods |= MOD_WIN;
    }
    // SAFETY: a thread hotkey (no window): WM_HOTKEY comes to this queue.
    if unsafe { RegisterHotKey(std::ptr::null_mut(), id as i32, mods, layout_vk(chord.key)) } != 0 {
        return Ok(());
    }
    // SAFETY: reads this thread's last error.
    let code = unsafe { GetLastError() };
    Err(if code == ERROR_HOTKEY_ALREADY_REGISTERED {
        "another program uses this shortcut".to_owned()
    } else {
        format!("Windows refused the shortcut: {}", last_error())
    })
}

fn deliver(kind: RawKind) {
    CAPTURE.with(|c| {
        if let Some(s) = c.borrow().as_ref() {
            let _ = s.send(RawEvent {
                at: Instant::now(),
                kind,
            });
        }
    });
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: Windows passes an MSLLHOOKSTRUCT for HC_ACTION.
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if info.dwExtraInfo != MARKER {
            let at = Point::new(info.pt.x, info.pt.y);
            let wheel = (info.mouseData >> 16) as u16 as i16 as i32;
            let x = |down| {
                if (info.mouseData >> 16) as u16 == XBUTTON1 {
                    RawKind::Button(MouseButton::Back, down, at)
                } else {
                    RawKind::Button(MouseButton::Forward, down, at)
                }
            };
            let kind = match wparam as u32 {
                WM_MOUSEMOVE => Some(RawKind::Move(at)),
                WM_LBUTTONDOWN => Some(RawKind::Button(MouseButton::Left, Press::Down, at)),
                WM_LBUTTONUP => Some(RawKind::Button(MouseButton::Left, Press::Up, at)),
                WM_RBUTTONDOWN => Some(RawKind::Button(MouseButton::Right, Press::Down, at)),
                WM_RBUTTONUP => Some(RawKind::Button(MouseButton::Right, Press::Up, at)),
                WM_MBUTTONDOWN => Some(RawKind::Button(MouseButton::Middle, Press::Down, at)),
                WM_MBUTTONUP => Some(RawKind::Button(MouseButton::Middle, Press::Up, at)),
                WM_XBUTTONDOWN => Some(x(Press::Down)),
                WM_XBUTTONUP => Some(x(Press::Up)),
                WM_MOUSEWHEEL if wheel != 0 => Some(RawKind::Wheel {
                    dx: 0,
                    dy: wheel,
                    at,
                }),
                WM_MOUSEHWHEEL if wheel != 0 => Some(RawKind::Wheel {
                    dx: wheel,
                    dy: 0,
                    at,
                }),
                _ => None,
            };
            if let Some(kind) = kind {
                deliver(kind);
            }
        }
    }
    // SAFETY: passing the call on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: Windows passes a KBDLLHOOKSTRUCT for HC_ACTION.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if info.dwExtraInfo != MARKER {
            let press = if info.flags & LLKHF_UP != 0 {
                Press::Up
            } else {
                Press::Down
            };
            let extended = info.flags & LLKHF_EXTENDED != 0;
            deliver(
                match Key::from_windows(info.vkCode, info.scanCode, extended) {
                    Some(k) => RawKind::Key(k, press),
                    None => RawKind::OtherKey,
                },
            );
        }
    }
    // SAFETY: passing the call on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// The 0…65535 coordinate Windows maps back to pixel `v` of a span of
/// `len` pixels starting at `origin`. Windows maps 0 to the first pixel and
/// 65535 to the last (`MOUSEINPUT`), rounding down: origin + n × (len − 1)
/// / 65535. Seen on the real desktop (Windows 11 26200, two monitors): the
/// earlier n × len / 65536 form landed a pixel short of (450, 360). The
/// value aims at the middle of the pixel's range of n, so a rounding
/// difference cannot reach a neighbour.
pub(crate) fn normalise(v: i32, origin: i32, len: u32) -> i32 {
    let len = i64::from(len.max(1));
    if len == 1 {
        return 0;
    }
    let rel = (i64::from(v) - i64::from(origin)).clamp(0, len - 1);
    ((2 * rel + 1) * 65_535 / (2 * (len - 1))).min(65_535) as i32
}

fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MARKER,
            },
        },
    }
}

fn keyboard(key: Key, press: Press) -> INPUT {
    let info = key.info();
    let up = if press == Press::Up {
        KEYEVENTF_KEYUP
    } else {
        0
    };
    let ext = if info.win_scan & 0xE000 != 0 {
        KEYEVENTF_EXTENDEDKEY
    } else {
        0
    };
    let by_vk = info.class == KeyClass::Media || info.win_vk == VK_PAUSE as u8;
    let ki = if by_vk {
        KEYBDINPUT {
            wVk: u16::from(info.win_vk),
            wScan: info.win_scan & 0xFF,
            dwFlags: up | ext,
            time: 0,
            dwExtraInfo: MARKER,
        }
    } else {
        KEYBDINPUT {
            wVk: 0,
            wScan: info.win_scan & 0xFF,
            dwFlags: up | ext | KEYEVENTF_SCANCODE,
            time: 0,
            dwExtraInfo: MARKER,
        }
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki },
    }
}

fn button(b: MouseButton, press: Press) -> INPUT {
    let down = press == Press::Down;
    let (flags, data) = match b {
        MouseButton::Left => (
            if down {
                MOUSEEVENTF_LEFTDOWN
            } else {
                MOUSEEVENTF_LEFTUP
            },
            0,
        ),
        MouseButton::Right => (
            if down {
                MOUSEEVENTF_RIGHTDOWN
            } else {
                MOUSEEVENTF_RIGHTUP
            },
            0,
        ),
        MouseButton::Middle => (
            if down {
                MOUSEEVENTF_MIDDLEDOWN
            } else {
                MOUSEEVENTF_MIDDLEUP
            },
            0,
        ),
        MouseButton::Back => (
            if down {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            1,
        ),
        MouseButton::Forward => (
            if down {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            2,
        ),
    };
    mouse(0, 0, data, flags)
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: a slice of initialised INPUTs.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!("Windows refused the input: {}", last_error()))
    }
}

fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        // SAFETY: `data` is the Vec below, alive during the enumeration.
        let out = unsafe { &mut *(data as *mut Vec<Monitor>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..MONITORINFO::default()
        };
        // SAFETY: a monitor handle from the enumeration.
        if unsafe { GetMonitorInfoW(m, &mut info) } != 0 {
            let r = info.rcMonitor;
            if r.right > r.left && r.bottom > r.top {
                out.push(Monitor {
                    bounds: Rect::new(
                        r.left,
                        r.top,
                        (r.right - r.left) as u32,
                        (r.bottom - r.top) as u32,
                    ),
                    primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
                });
            }
        }
        1
    }
    let mut out: Vec<Monitor> = Vec::new();
    // SAFETY: enumerates every monitor into `out`.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut out as *mut Vec<Monitor> as LPARAM,
        );
    }
    out
}

fn window_pid(w: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: a window handle (any) and an out pointer.
    unsafe { GetWindowThreadProcessId(w, &mut pid) };
    pid
}

fn process_name(pid: u32) -> String {
    // SAFETY: a limited query handle, closed below.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or("").to_owned()
    }
}

impl Backend for WindowsBackend {
    fn send(&self, batch: &[Synth]) -> Result<(), String> {
        let screens = monitors();
        let virt = super::model::virtual_bounds(&screens).unwrap_or(Rect::new(0, 0, 1, 1));
        let mut pending: Vec<INPUT> = Vec::with_capacity(batch.len());
        for s in batch {
            match *s {
                Synth::Move(p) => {
                    send_inputs(&pending)?;
                    pending.clear();
                    send_inputs(&[mouse(
                        normalise(p.x, virt.x, virt.width),
                        normalise(p.y, virt.y, virt.height),
                        0,
                        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    )])?;
                    // The exact pixel, before any button of this batch.
                    let mut at = POINT { x: 0, y: 0 };
                    // SAFETY: an out pointer; positions in physical pixels.
                    unsafe {
                        if GetCursorPos(&mut at) != 0 && (at.x != p.x || at.y != p.y) {
                            SetCursorPos(p.x, p.y);
                        }
                    }
                }
                Synth::Button(b, press) => pending.push(button(b, press)),
                Synth::Wheel { dx, dy } => {
                    if dy != 0 {
                        pending.push(mouse(0, 0, dy as u32, MOUSEEVENTF_WHEEL));
                    }
                    if dx != 0 {
                        pending.push(mouse(0, 0, dx as u32, MOUSEEVENTF_HWHEEL));
                    }
                }
                Synth::Key(k, press) => pending.push(keyboard(k, press)),
            }
        }
        send_inputs(&pending)
    }

    fn cursor(&self) -> Result<Point, String> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: an out pointer.
        if unsafe { GetCursorPos(&mut p) } == 0 {
            return Err(format!(
                "the pointer position cannot be read: {}",
                last_error()
            ));
        }
        Ok(Point::new(p.x, p.y))
    }

    fn monitors(&self) -> Result<Vec<Monitor>, String> {
        let m = monitors();
        if m.is_empty() {
            return Err("no monitor was found".to_owned());
        }
        Ok(m)
    }

    fn foreground(&self) -> Option<WindowInfo> {
        // SAFETY: plain queries of the foreground window.
        unsafe {
            let w = GetForegroundWindow();
            if w.is_null() {
                return None;
            }
            let mut title = [0u16; 512];
            let n = GetWindowTextW(w, title.as_mut_ptr(), title.len() as i32);
            let mut r = RECT::default();
            let mut origin = POINT { x: 0, y: 0 };
            if GetClientRect(w, &mut r) == 0 || ClientToScreen(w, &mut origin) == 0 {
                return None;
            }
            Some(WindowInfo {
                title: String::from_utf16_lossy(&title[..n.max(0) as usize]),
                process: process_name(window_pid(w)),
                client: Rect::new(
                    origin.x,
                    origin.y,
                    (r.right - r.left).max(1) as u32,
                    (r.bottom - r.top).max(1) as u32,
                ),
            })
        }
    }

    fn own_window_at(&self, p: Point) -> bool {
        // SAFETY: plain window queries.
        unsafe {
            let w = WindowFromPoint(POINT { x: p.x, y: p.y });
            if w.is_null() {
                return false;
            }
            // The top-level window: WebView2's content belongs to its own
            // process, its frame to sotyware.
            let root = GetAncestor(w, GA_ROOT);
            window_pid(if root.is_null() { w } else { root }) == self.pid
        }
    }

    fn own_window_active(&self) -> bool {
        // SAFETY: a plain query.
        let w = unsafe { GetForegroundWindow() };
        !w.is_null() && window_pid(w) == self.pid
    }

    fn key_name(&self, key: Key) -> Option<String> {
        let info = key.info();
        if info.class != KeyClass::Layout {
            return None;
        }
        let lparam = (i32::from(info.win_scan & 0xFF) << 16)
            | if info.win_scan & 0xE000 != 0 {
                1 << 24
            } else {
                0
            };
        let mut buf = [0u16; 64];
        // SAFETY: a buffer and its length.
        let n = unsafe { GetKeyNameTextW(lparam, buf.as_mut_ptr(), buf.len() as i32) };
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]).to_uppercase())
    }

    fn register_hotkey(&self, id: u32, chord: Chord) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        if !self.request(Request::Register(id, chord, tx)) {
            return Err("the input thread has ended".to_owned());
        }
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "the input thread did not answer".to_owned())?
    }

    fn unregister_hotkey(&self, id: u32) {
        self.request(Request::Unregister(id));
    }

    fn start_capture(&self, sink: Sender<RawEvent>) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        if !self.request(Request::StartCapture(sink, tx)) {
            return Err("the input thread has ended".to_owned());
        }
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "the input thread did not answer".to_owned())?
    }

    fn stop_capture(&self) {
        let (tx, rx) = mpsc::channel();
        if self.request(Request::StopCapture(tx)) {
            let _ = rx.recv_timeout(Duration::from_secs(2));
        }
    }

    fn shutdown(&self) {
        self.request(Request::Quit);
        if let Some(h) = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !h.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The opt-in real-desktop test (moves the pointer; never in CI).
#[cfg(test)]
#[path = "windows/desktop.rs"]
mod desktop;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalised_coordinates_map_back_to_their_pixel() {
        // Windows maps n back to origin + n * (len - 1) / 65535, rounded
        // down: 0 is the first pixel, 65535 the last. The real desktop test
        // found it (3440 x 2880 virtual desktop from (0, -1440)): the
        // earlier n * len / 65536 form sent (450, 360) to (449, 359).
        let back = |n: i32, origin: i32, len: u32| {
            origin + (i64::from(n) * (i64::from(len) - 1) / 65_535) as i32
        };
        for (origin, len) in [
            (0, 1920u32),
            (-1280, 3200),
            (0, 3840),
            (0, 3440),
            (-1440, 2880),
            (0, 1),
            (100, 7),
        ] {
            let last = origin + len as i32 - 1;
            for v in [
                origin,
                (origin + 1).min(last),
                origin + len as i32 / 2,
                last,
                (origin + 450).min(last),
                (origin + 1800).min(last),
            ] {
                let n = normalise(v, origin, len);
                assert!((0..=65_535).contains(&n));
                assert_eq!(back(n, origin, len), v, "{v} in {origin}+{len}");
            }
        }
        assert_eq!(back(normalise(450, 0, 3440), 0, 3440), 450);
        assert_eq!(back(normalise(360, -1440, 2880), -1440, 2880), 360);
        // Every pixel of a span, both ends included.
        for len in [2u32, 3, 7, 1366, 2560] {
            for v in 0..len as i32 {
                assert_eq!(back(normalise(v, 0, len), 0, len), v, "{v} of {len}");
            }
        }
        // Points off the desktop go to its edge pixel.
        assert_eq!(normalise(-5, 0, 100), normalise(0, 0, 100));
        assert_eq!(back(normalise(-5, 0, 100), 0, 100), 0);
        assert_eq!(normalise(500, 0, 100), 65_535);
        assert_eq!(back(normalise(500, 0, 100), 0, 100), 99);
    }

    #[test]
    fn keys_go_by_scan_code_except_pause_and_media_keys() {
        let k = |c: &str| Key::from_code(c).unwrap();
        // SAFETY: reading the union member just written.
        let ki = |i: INPUT| unsafe { i.Anonymous.ki };
        let a = ki(keyboard(k("KeyA"), Press::Down));
        assert_eq!((a.wVk, a.wScan, a.dwFlags), (0, 0x1E, KEYEVENTF_SCANCODE));
        let up = ki(keyboard(k("ArrowUp"), Press::Up));
        assert_eq!(
            (up.wScan, up.dwFlags),
            (
                0x48,
                KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
            )
        );
        let pause = ki(keyboard(k("Pause"), Press::Down));
        assert_eq!(
            (pause.wVk, pause.dwFlags & KEYEVENTF_SCANCODE),
            (VK_PAUSE, 0)
        );
        let play = ki(keyboard(k("MediaPlayPause"), Press::Down));
        assert_eq!((play.wVk, play.dwFlags), (0xB3, KEYEVENTF_EXTENDEDKEY));
        assert_eq!(a.dwExtraInfo, MARKER);
    }
}

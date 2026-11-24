//! The notes overlay's window on Windows (ADR-052): shown and hidden
//! without being activated, its extended style for each mode, and the
//! foreground window it gives the keyboard back to.
//!
//! tao shows a window with `ShowWindow(SW_SHOW)` (its "don't focus" marker
//! lasts only until the window is created), and `SW_SHOW` activates a
//! window even with `WS_EX_NOACTIVATE` (seen on Windows 11 25H2,
//! 2026-11-23), which after a shortcut press would take the keyboard from
//! the game. And tao rewrites the extended style from its own flags on
//! every change of them (`set_focusable`, `set_ignore_cursor_events`,
//! `set_visible`), so styles set here would not last. So after the window
//! is created, sotyware changes none of tao's flags: it shows and hides
//! the window with `SetWindowPos(SWP_SHOWWINDOW | SWP_NOACTIVATE)` and
//! `SWP_HIDEWINDOW`, and sets its extended style itself. tao's own idea of
//! its visibility then stays "hidden", which only its `set_focus` reads,
//! so the overlay takes the keyboard with `SetForegroundWindow` (wry moves
//! the focus into the webview on `WM_SETFOCUS`). Through `windows-sys`
//! (already a dependency; `Win32_UI_WindowsAndMessaging` and
//! `Win32_Foundation` are compiled in the tree already).

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GW_HWNDNEXT, GWL_EXSTYLE, GetForegroundWindow, GetWindow, GetWindowLongW,
    GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

fn hwnd(h: isize) -> HWND {
    h as HWND
}

/// The window that has the keyboard, if any.
pub fn foreground() -> Option<isize> {
    // SAFETY: a plain query.
    let w = unsafe { GetForegroundWindow() };
    (!w.is_null()).then_some(w as isize)
}

pub fn is_window(h: isize) -> bool {
    // SAFETY: any value is accepted; a stale handle gives false.
    unsafe { IsWindow(hwnd(h)) != 0 }
}

/// Whether window `h` belongs to this process (the dashboard, the
/// overlay): the keyboard is never "given back" to sotyware itself.
pub fn is_own_process(h: isize) -> bool {
    let mut pid = 0;
    // SAFETY: a window handle (any) and an out pointer.
    unsafe { GetWindowThreadProcessId(hwnd(h), &mut pid) };
    pid == std::process::id()
}

/// Gives window `h` the keyboard; false if Windows refused (sotyware may
/// take the foreground only right after the user's input reached it: a
/// shortcut press, a click on its window). Never makes up a key press to
/// get round that, as tao's `set_focus` does.
pub fn activate(h: isize) -> bool {
    // SAFETY: a window handle; Windows checks the foreground rules.
    unsafe { SetForegroundWindow(hwnd(h)) != 0 }
}

/// The overlay's extended style in a mode, from its current one: a tool
/// window (no taskbar button, not in Alt+Tab; tao gives every window
/// without an owner `WS_EX_APPWINDOW`), and when passive click-through
/// (`WS_EX_TRANSPARENT | WS_EX_LAYERED`, as tao's `set_ignore_cursor_events`)
/// and never activated by a click (`WS_EX_NOACTIVATE`).
pub fn ex_style(current: u32, passive: bool) -> u32 {
    let passive_bits = WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE;
    let base = (current & !(WS_EX_APPWINDOW | passive_bits)) | WS_EX_TOOLWINDOW;
    if passive { base | passive_bits } else { base }
}

/// Gives window `h` the [`ex_style`] of a mode, shown or hidden (it stays
/// so).
pub fn set_mode(h: isize, passive: bool) {
    // SAFETY: a window handle of this process; the style is read, changed
    // and applied (`SWP_FRAMECHANGED`) without showing or activating it.
    unsafe {
        let current = GetWindowLongW(hwnd(h), GWL_EXSTYLE) as u32;
        SetWindowLongW(hwnd(h), GWL_EXSTYLE, ex_style(current, passive) as i32);
        SetWindowPos(
            hwnd(h),
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Shows window `h` at the top of the topmost windows (above a game that
/// made itself topmost after it) without activating it; a minimized one is
/// restored first, without activation too.
pub fn show(h: isize) {
    // SAFETY: a window handle of this process.
    unsafe {
        if IsIconic(hwnd(h)) != 0 {
            ShowWindow(hwnd(h), SW_SHOWNOACTIVATE);
        }
        SetWindowPos(
            hwnd(h),
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Hides window `h` without activating another one: the keyboard was
/// given back before, when the overlay had it.
pub fn hide(h: isize) {
    // SAFETY: a window handle of this process.
    unsafe {
        SetWindowPos(
            hwnd(h),
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// The first window below `h` in the z-order that a user would have been
/// using: visible, not minimized, not a tool or click-through window,
/// not sotyware's. When the overlay took the keyboard by a click, the
/// program used before it is usually that one.
pub fn next_below(h: isize) -> Option<isize> {
    let mut w = hwnd(h);
    for _ in 0..512 {
        // SAFETY: walking the z-order with window handles.
        w = unsafe { GetWindow(w, GW_HWNDNEXT) };
        if w.is_null() {
            return None;
        }
        // SAFETY: plain queries of a window handle.
        let (visible, minimized, ex) = unsafe {
            (
                IsWindowVisible(w) != 0,
                IsIconic(w) != 0,
                GetWindowLongW(w, GWL_EXSTYLE) as u32,
            )
        };
        let skipped = ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) != 0;
        if visible && !minimized && !skipped && !is_own_process(w as isize) {
            return Some(w as isize);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_is_a_tool_window_and_passive_lets_clicks_through() {
        // What tao gives a window without an owner, interactive: a
        // taskbar entry, topmost.
        let tao = WS_EX_APPWINDOW | 0x8 | 0x100;
        let interactive = ex_style(tao, false);
        assert_eq!(interactive, WS_EX_TOOLWINDOW | 0x8 | 0x100);
        let passive = ex_style(interactive, true);
        assert_eq!(
            passive,
            interactive | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE
        );
        // And back, whatever it was.
        assert_eq!(ex_style(passive, false), interactive);
        assert_eq!(ex_style(tao | WS_EX_NOACTIVATE, false), interactive);
        assert_eq!(ex_style(passive, true), passive);
    }

    #[test]
    fn a_handle_that_is_no_window_is_refused() {
        assert!(!is_window(0));
        assert!(!is_window(0x7FFF_FFF0));
        // Never panics on what is not a window.
        assert!(!is_own_process(0) || std::process::id() == 0);
        assert_eq!(next_below(0x7FFF_FFF0), None);
    }
}

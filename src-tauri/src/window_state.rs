//! Where the dashboard window opens, how big it is, and remembering where the
//! user put it.
//!
//! - **First launch** (nothing saved): half the width and half the height of
//!   the primary monitor's *work area* (the screen without the taskbar, Dock
//!   or panels), but at least [`DEFAULT_MIN`] (900 × 540 logical pixels: the
//!   dashboard's sidebar layout needs 760 px; half of a small laptop screen
//!   would collapse it into an overlay) and never more than the work area,
//!   centred in that work area. On a 1920 × 1080 work area at 100 % scaling
//!   that is 960 × 540.
//! - **Later launches:** the size and place the window had when it was last
//!   hidden, unfocused or quit, maximized again if it was. A saved place is
//!   used only where it still overlaps a connected monitor's work area; the
//!   window is then moved (and if need be shrunk) to lie fully inside that
//!   work area, and resized for that monitor's scaling if it changed. A place
//!   on no connected monitor (a monitor was unplugged) gives the first-launch
//!   default.
//! - **Showing a hidden window** (tray "Open Dashboard", a second launch)
//!   first checks that it is still on a connected monitor and moves it back
//!   onto one if not ([`ensure_visible`]).
//!
//! The geometry is plain functions on physical pixels ([`default_placement`],
//! [`restore_placement`]), unit-tested without a display. What is saved is
//! one JSON row in `window-state.sqlite3` (`storage.rs`), its own file so that
//! an older sotyware never meets a schema it does not know. The bounds are
//! tracked in memory as the window moves and written only when the window is
//! hidden or loses focus and when the process exits: no timer, and no write
//! per pixel of a drag.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{PhysicalPosition, PhysicalSize, Runtime, Window};

use crate::account::StateStore;
use crate::features::StoreError;

/// The format of the saved JSON; another value is ignored.
const FORMAT: u32 = 1;

/// The smallest first-launch size, in logical pixels: wide enough for the
/// dashboard's sidebar layout (its breakpoint is 760 px) with room for the
/// page. The window itself can still be made as small as the configured
/// minimum (640 × 480).
pub const DEFAULT_MIN: (f64, f64) = (900.0, 540.0);

/// A rectangle in physical pixels of the virtual desktop (which may start
/// at negative coordinates when a monitor is left of or above the primary).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// The area this rectangle shares with `other`, in square pixels.
    fn overlap(self, other: Rect) -> i64 {
        let w = self.right().min(other.right()) - i64::from(self.x.max(other.x));
        let h = self.bottom().min(other.bottom()) - i64::from(self.y.max(other.y));
        if w <= 0 || h <= 0 { 0 } else { w * h }
    }
}

/// One connected monitor as placement needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// The work area: the monitor without taskbar, Dock or panels.
    pub work: Rect,
    /// Physical pixels per logical pixel (1.5 at 150 % scaling).
    pub scale: f64,
}

/// What is saved: the window's normal (not maximized) bounds and whether it
/// was maximized. `x`, `y` are the outer position; `width`, `height` the
/// inner size, as Tauri sets them (`set_position`, `set_size`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub format: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// The scale factor of the monitor the window was on, so its logical
    /// size is kept on a monitor whose scaling changed.
    pub scale: f64,
}

impl Saved {
    pub fn new(bounds: Rect, maximized: bool, scale: f64) -> Self {
        Self {
            format: FORMAT,
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            maximized,
            scale,
        }
    }

    fn bounds(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    /// Reads a saved row; `None` for anything this version cannot use.
    pub fn parse(text: &str) -> Option<Self> {
        let s: Saved = serde_json::from_str(text).ok()?;
        (s.format == FORMAT && s.width > 0 && s.height > 0 && s.scale.is_finite() && s.scale > 0.0)
            .then_some(s)
    }
}

/// Where and how the window opens: the outer position and the inner size
/// (physical pixels), and whether to maximize it afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// The window's minimum inner size in physical pixels on `screen`.
fn minimum(screen: &Screen, min_logical: (f64, f64)) -> (u32, u32) {
    let px = |v: f64| {
        let p = (v * screen.scale).round();
        if p.is_finite() && p > 0.0 {
            p.min(f64::from(u32::MAX)) as u32
        } else {
            0
        }
    };
    (px(min_logical.0), px(min_logical.1))
}

/// `wanted`, at least `min` and at most `max`; `max` wins when the screen is
/// smaller than the minimum (the window then fills the work area).
fn fit(wanted: u32, min: u32, max: u32) -> u32 {
    wanted.max(min).min(max).max(1)
}

/// The first-launch placement on `screen`: half its work area's width and
/// height (outer size), at least `min_logical` and [`DEFAULT_MIN`] (inner
/// size, logical pixels) and at most the work area, centred. `frame` is what
/// the window's decorations add to the inner size (zero for an undecorated
/// window).
pub fn default_placement(screen: &Screen, min_logical: (f64, f64), frame: (u32, u32)) -> Placement {
    let work = screen.work;
    let (min_w, min_h) = minimum(
        screen,
        (
            min_logical.0.max(DEFAULT_MIN.0),
            min_logical.1.max(DEFAULT_MIN.1),
        ),
    );
    let max_w = work.width.saturating_sub(frame.0);
    let max_h = work.height.saturating_sub(frame.1);
    let width = fit((work.width / 2).saturating_sub(frame.0), min_w, max_w);
    let height = fit((work.height / 2).saturating_sub(frame.1), min_h, max_h);
    Placement {
        x: centred(work.x, work.width, width + frame.0),
        y: centred(work.y, work.height, height + frame.1),
        width,
        height,
        maximized: false,
    }
}

fn centred(start: i32, span: u32, size: u32) -> i32 {
    let offset = i64::from(span.saturating_sub(size) / 2);
    i32::try_from(i64::from(start) + offset).unwrap_or(start)
}

/// `pos` moved so that `size` fits inside `[start, start + span)`.
fn clamp_into(pos: i32, size: u32, start: i32, span: u32) -> i32 {
    let last = i64::from(start) + i64::from(span) - i64::from(size);
    let p = i64::from(pos).min(last).max(i64::from(start));
    i32::try_from(p).unwrap_or(start)
}

/// The saved placement made visible on the connected `screens`, or `None`
/// when it is on none of them (the caller then uses [`default_placement`]).
///
/// The window goes to the monitor whose work area it overlaps most; its
/// inner size is rescaled when that monitor's scaling differs from the one
/// it was saved on, kept at least the minimum and at most the work area,
/// and it is moved to lie fully inside the work area.
pub fn restore_placement(
    saved: &Saved,
    screens: &[Screen],
    min_logical: (f64, f64),
    frame: (u32, u32),
) -> Option<Placement> {
    let outer = Rect {
        width: saved.width.saturating_add(frame.0),
        height: saved.height.saturating_add(frame.1),
        ..saved.bounds()
    };
    let screen = screens
        .iter()
        .map(|s| (s, outer.overlap(s.work)))
        .filter(|(_, area)| *area > 0)
        .max_by_key(|(_, area)| *area)
        .map(|(s, _)| *s)?;
    let ratio = if (screen.scale - saved.scale).abs() > 0.01 {
        screen.scale / saved.scale
    } else {
        1.0
    };
    let rescale = |v: u32| {
        let r = (f64::from(v) * ratio).round();
        if r.is_finite() && r >= 1.0 {
            r.min(f64::from(u32::MAX)) as u32
        } else {
            1
        }
    };
    let (min_w, min_h) = minimum(&screen, min_logical);
    let work = screen.work;
    let width = fit(
        rescale(saved.width),
        min_w,
        work.width.saturating_sub(frame.0),
    );
    let height = fit(
        rescale(saved.height),
        min_h,
        work.height.saturating_sub(frame.1),
    );
    Some(Placement {
        x: clamp_into(saved.x, width + frame.0, work.x, work.width),
        y: clamp_into(saved.y, height + frame.1, work.y, work.height),
        width,
        height,
        maximized: saved.maximized,
    })
}

/// Whether `bounds` (outer) overlaps any work area of `screens`.
pub fn on_some_screen(bounds: Rect, screens: &[Screen]) -> bool {
    screens.iter().any(|s| bounds.overlap(s.work) > 0)
}

/// Managed state: the placement saved by an earlier session, and the
/// window's normal bounds as it moves now.
pub struct WindowStates {
    store: Mutex<Result<Box<dyn StateStore>, String>>,
    /// Read once when loading.
    initial: Option<Saved>,
    /// The latest bounds seen (from the window's own events).
    latest: Mutex<Option<Saved>>,
    /// What the store holds, to write only changes.
    written: Mutex<Option<Saved>>,
}

impl WindowStates {
    /// Reads the saved placement. A store that cannot be opened or read only
    /// means the default placement and no saving; the error is logged.
    pub fn load(store: Result<Box<dyn StateStore>, StoreError>) -> Self {
        let mut store = store.map_err(|e| e.to_string());
        let initial = match &mut store {
            Ok(s) => match s.load() {
                Ok(text) => text.as_deref().and_then(Saved::parse),
                Err(e) => {
                    eprintln!("sotyware: window placement unavailable: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("sotyware: window placement unavailable: {e}");
                None
            }
        };
        Self {
            store: Mutex::new(store),
            initial,
            latest: Mutex::new(initial),
            written: Mutex::new(initial),
        }
    }

    /// The placement saved by an earlier session, if any.
    pub fn initial(&self) -> Option<Saved> {
        self.initial
    }

    /// Records the window's current normal bounds (or that it is maximized,
    /// keeping the normal bounds it had before).
    pub fn track(&self, bounds: Rect, maximized: bool, scale: f64) {
        let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        *latest = Some(match (*latest, maximized) {
            (Some(previous), true) => Saved {
                maximized: true,
                ..previous
            },
            _ => Saved::new(bounds, maximized, scale),
        });
    }

    /// Writes the latest bounds if they changed since the last write. Called
    /// off the event loop, and once more at exit.
    pub fn persist(&self) {
        let Some(latest) = *self.latest.lock().unwrap_or_else(|e| e.into_inner()) else {
            return;
        };
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if *written == Some(latest) {
            return;
        }
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(store) = store.as_mut() else {
            return;
        };
        let text = serde_json::to_string(&latest).expect("serializes");
        match store.save(&text) {
            Ok(()) => *written = Some(latest),
            Err(e) => eprintln!("sotyware: cannot save the window placement: {e}"),
        }
    }
}

/// The connected monitors as placement sees them.
fn screens<R: Runtime>(window: &Window<R>) -> Vec<Screen> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let w = m.work_area();
            Screen {
                work: Rect {
                    x: w.position.x,
                    y: w.position.y,
                    width: w.size.width,
                    height: w.size.height,
                },
                scale: m.scale_factor(),
            }
        })
        .filter(|s| s.work.width > 0 && s.work.height > 0)
        .collect()
}

/// The primary monitor (else the first connected one).
fn primary<R: Runtime>(window: &Window<R>, all: &[Screen]) -> Option<Screen> {
    let primary = window.primary_monitor().ok().flatten().map(|m| {
        let w = m.work_area();
        Screen {
            work: Rect {
                x: w.position.x,
                y: w.position.y,
                width: w.size.width,
                height: w.size.height,
            },
            scale: m.scale_factor(),
        }
    });
    primary
        .filter(|s| s.work.width > 0 && s.work.height > 0)
        .or_else(|| all.first().copied())
}

/// What the window's decorations add to its inner size (zero when it has
/// none, or when the platform does not know yet).
fn frame<R: Runtime>(window: &Window<R>) -> (u32, u32) {
    match (window.outer_size(), window.inner_size()) {
        (Ok(o), Ok(i)) => (
            o.width.saturating_sub(i.width),
            o.height.saturating_sub(i.height),
        ),
        _ => (0, 0),
    }
}

fn apply<R: Runtime>(window: &Window<R>, p: Placement) {
    // Position first (it may move the window to a monitor with another
    // scaling, which resizes it), then the exact size, then the position
    // again in case the scaling change moved it.
    let position = PhysicalPosition::new(p.x, p.y);
    let steps = [
        window.set_position(position),
        window.set_size(PhysicalSize::new(p.width, p.height)),
        window.set_position(position),
    ];
    for e in steps.into_iter().filter_map(Result::err) {
        eprintln!("sotyware: cannot place the dashboard: {e}");
    }
    if p.maximized
        && let Err(e) = window.maximize()
    {
        eprintln!("sotyware: cannot maximize the dashboard: {e}");
    }
}

/// Places the (still hidden) dashboard where it was saved, or at the
/// first-launch default, and records that as its current placement.
/// Without monitor information the window keeps its configured size,
/// centred.
pub fn place<R: Runtime>(window: &Window<R>, states: &WindowStates, min_logical: (f64, f64)) {
    let all = screens(window);
    let frame = frame(window);
    let placement = states
        .initial()
        .and_then(|saved| restore_placement(&saved, &all, min_logical, frame))
        .or_else(|| primary(window, &all).map(|s| default_placement(&s, min_logical, frame)));
    match placement {
        Some(p) => {
            apply(window, p);
            let scale = window.scale_factor().unwrap_or(1.0);
            states.track(
                Rect {
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                },
                false,
                scale,
            );
            if p.maximized {
                states.track(
                    Rect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    true,
                    scale,
                );
            }
        }
        None => {
            if let Err(e) = window.center() {
                eprintln!("sotyware: cannot centre the dashboard: {e}");
            }
        }
    }
}

/// Records the window's bounds after it moved or was resized. A minimized
/// or hidden window says nothing about where the user wants it.
pub fn track<R: Runtime>(window: &Window<R>, states: &WindowStates) {
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    if !visible || minimized {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return;
    }
    states.track(
        Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        maximized,
        window.scale_factor().unwrap_or(1.0),
    );
}

/// Before a hidden dashboard is shown again: if it lies on no connected
/// monitor (one was unplugged or rearranged while it was hidden), moves it
/// onto the nearest one, or to the default place.
pub fn ensure_visible<R: Runtime>(window: &Window<R>, min_logical: (f64, f64)) {
    let all = screens(window);
    if all.is_empty() || window.is_maximized().unwrap_or(false) {
        return;
    }
    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let frame = frame(window);
    let outer = Rect {
        x: position.x,
        y: position.y,
        width: size.width.saturating_add(frame.0),
        height: size.height.saturating_add(frame.1),
    };
    if on_some_screen(outer, &all) {
        return;
    }
    if let Some(screen) = primary(window, &all) {
        apply(window, default_placement(&screen, min_logical, frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: (f64, f64) = (640.0, 480.0);

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

    fn saved(x: i32, y: i32, width: u32, height: u32) -> Saved {
        Saved::new(
            Rect {
                x,
                y,
                width,
                height,
            },
            false,
            1.0,
        )
    }

    fn placement(x: i32, y: i32, width: u32, height: u32) -> Placement {
        Placement {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    #[test]
    fn the_default_is_half_the_work_area_centred() {
        // 1920 × 1080 usable desktop: 960 × 540, centred.
        let s = screen(0, 0, 1920, 1080, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(480, 270, 960, 540)
        );
    }

    #[test]
    fn the_default_uses_the_work_area_not_the_whole_screen() {
        // A 1920 × 1080 monitor with a 48 px taskbar at the bottom: half the
        // height (516) is below the first-launch minimum of 540.
        let s = screen(0, 0, 1920, 1032, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(480, 246, 960, 540)
        );
        // A taskbar on the left: the work area starts at x = 62.
        let s = screen(62, 0, 1858, 1080, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(62 + 464, 270, 929, 540)
        );
    }

    #[test]
    fn the_default_is_never_below_the_first_launch_minimum() {
        // A 1366 × 768 laptop (728 px work area): half is 683 × 364, which
        // would collapse the sidebar; the first window is 900 × 540.
        let s = screen(0, 0, 1366, 728, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(233, 94, 900, 540)
        );
        // 1280 × 800 (the virtual display of the desktop tests).
        let s = screen(0, 0, 1280, 800, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(190, 130, 900, 540)
        );
        // A larger configured minimum still wins.
        assert_eq!(
            default_placement(&s, (1000.0, 600.0), (0, 0)),
            placement(140, 100, 1000, 600)
        );
        assert_eq!(DEFAULT_MIN, (900.0, 540.0));
    }

    #[test]
    fn the_minimum_is_in_logical_pixels() {
        // 150 % scaling: the 900 × 540 first-launch minimum is 1350 × 810
        // physical pixels; half of 2560 × 1392 is 1280 × 696, below it.
        let s = screen(0, 0, 2560, 1392, 1.5);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(605, 291, 1350, 810)
        );
        // A 3840 × 2088 work area at 150 %: half (1920 × 1044) is above it.
        let s = screen(0, 0, 3840, 2088, 1.5);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(960, 522, 1920, 1044)
        );
    }

    #[test]
    fn a_screen_smaller_than_the_minimum_gets_the_whole_work_area() {
        let s = screen(0, 0, 600, 400, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(0, 0, 600, 400)
        );
    }

    #[test]
    fn decorations_count_towards_half_the_work_area() {
        // A native title bar of 32 px and 2 px borders: the outer window is
        // 1440 × 800 on a 2880 × 1600 work area, the inner size smaller.
        let s = screen(0, 0, 2880, 1600, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (4, 34)),
            placement(720, 400, 1436, 766)
        );
        // Where the minimum applies, it is the inner size's.
        let s = screen(0, 0, 1920, 1080, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (4, 34)),
            placement(480, 253, 956, 540)
        );
    }

    #[test]
    fn the_default_on_a_secondary_primary_monitor_with_negative_coordinates() {
        let s = screen(-1920, -200, 1920, 1080, 1.0);
        assert_eq!(
            default_placement(&s, MIN, (0, 0)),
            placement(-1920 + 480, -200 + 270, 960, 540)
        );
    }

    #[test]
    fn a_saved_placement_on_a_connected_monitor_is_used_unchanged() {
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        let s = saved(100, 80, 1200, 800);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(100, 80, 1200, 800))
        );
    }

    #[test]
    fn a_maximized_window_is_maximized_again() {
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        let s = Saved {
            maximized: true,
            ..saved(100, 80, 1200, 800)
        };
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(Placement {
                maximized: true,
                ..placement(100, 80, 1200, 800)
            })
        );
    }

    #[test]
    fn a_placement_on_an_unplugged_monitor_falls_back_to_the_default() {
        // Saved on a second monitor right of the primary, which is gone.
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        let s = saved(2200, 100, 1000, 700);
        assert_eq!(restore_placement(&s, &screens, MIN, (0, 0)), None);
        assert!(!on_some_screen(s.bounds(), &screens));
    }

    #[test]
    fn a_partly_visible_window_is_moved_fully_inside() {
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        // Hanging off the right and bottom edges.
        let s = saved(1500, 900, 1000, 700);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(920, 340, 1000, 700))
        );
        // Above and left of the work area.
        let s = saved(-300, -50, 1000, 700);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(0, 0, 1000, 700))
        );
    }

    #[test]
    fn a_window_larger_than_a_smaller_screen_is_shrunk_to_fit() {
        // Saved at 2400 × 1300 on a big monitor; now only 1366 × 728.
        let screens = [screen(0, 0, 1366, 728, 1.0)];
        let s = saved(50, 40, 2400, 1300);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(0, 0, 1366, 728))
        );
    }

    #[test]
    fn the_window_goes_to_the_monitor_it_overlaps_most() {
        let screens = [
            screen(0, 0, 1920, 1040, 1.0),
            screen(1920, 0, 2560, 1400, 1.0),
        ];
        // Mostly on the second monitor, a little on the first.
        let s = saved(1800, 100, 1200, 800);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(1920, 100, 1200, 800))
        );
    }

    #[test]
    fn a_monitor_left_of_the_primary_with_negative_coordinates() {
        let screens = [
            screen(0, 0, 1920, 1040, 1.0),
            screen(-1680, 0, 1680, 1010, 1.0),
        ];
        let s = saved(-1500, 120, 1100, 750);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(-1500, 120, 1100, 750))
        );
    }

    #[test]
    fn a_changed_scaling_keeps_the_logical_size() {
        // Saved at 100 %: 1000 × 700. The monitor is now at 150 %.
        let screens = [screen(0, 0, 3840, 2088, 1.5)];
        let s = saved(200, 100, 1000, 700);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(200, 100, 1500, 1050))
        );
    }

    #[test]
    fn a_restored_window_keeps_the_minimum_size() {
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        let s = saved(10, 10, 300, 200);
        assert_eq!(
            restore_placement(&s, &screens, MIN, (0, 0)),
            Some(placement(10, 10, 640, 480))
        );
    }

    #[test]
    fn decorations_are_kept_inside_the_work_area_too() {
        let screens = [screen(0, 0, 1920, 1040, 1.0)];
        let s = saved(1000, 500, 900, 520);
        // Outer 904 × 554 must fit: x ≤ 1016, y ≤ 486.
        assert_eq!(
            restore_placement(&s, &screens, MIN, (4, 34)),
            Some(placement(1000, 486, 900, 520))
        );
    }

    #[test]
    fn no_screen_information_means_no_restored_placement() {
        assert_eq!(
            restore_placement(&saved(0, 0, 800, 600), &[], MIN, (0, 0)),
            None
        );
    }

    #[test]
    fn saved_rows_are_parsed_strictly() {
        let s = Saved {
            maximized: true,
            ..saved(-20, 30, 1200, 700)
        };
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(
            text,
            r#"{"format":1,"x":-20,"y":30,"width":1200,"height":700,"maximized":true,"scale":1.0}"#
        );
        assert_eq!(Saved::parse(&text), Some(s));
        for bad in [
            "",
            "not json",
            r#"{"format":2,"x":0,"y":0,"width":10,"height":10,"maximized":false,"scale":1.0}"#,
            r#"{"format":1,"x":0,"y":0,"width":0,"height":10,"maximized":false,"scale":1.0}"#,
            r#"{"format":1,"x":0,"y":0,"width":10,"height":10,"maximized":false,"scale":0.0}"#,
            r#"{"format":1,"x":0,"y":0,"width":10,"maximized":false,"scale":1.0}"#,
        ] {
            assert_eq!(Saved::parse(bad), None, "{bad}");
        }
    }

    /// An in-memory [`StateStore`] that counts writes.
    #[derive(Default)]
    struct Memory {
        row: std::sync::Arc<Mutex<Option<String>>>,
        writes: std::sync::Arc<Mutex<usize>>,
        fail: bool,
    }

    impl StateStore for Memory {
        fn load(&mut self) -> Result<Option<String>, StoreError> {
            Ok(self.row.lock().unwrap().clone())
        }
        fn save(&mut self, state: &str) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::new("disk full"));
            }
            *self.writes.lock().unwrap() += 1;
            *self.row.lock().unwrap() = Some(state.to_owned());
            Ok(())
        }
        fn clear(&mut self) -> Result<(), StoreError> {
            *self.row.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn bounds_are_written_only_when_they_changed() {
        let store = Memory::default();
        let (row, writes) = (store.row.clone(), store.writes.clone());
        let states = WindowStates::load(Ok(Box::new(store)));
        assert_eq!(states.initial(), None);
        states.persist();
        assert_eq!(*writes.lock().unwrap(), 0, "nothing tracked yet");

        let r = Rect {
            x: 10,
            y: 20,
            width: 900,
            height: 600,
        };
        states.track(r, false, 1.25);
        states.persist();
        states.persist();
        assert_eq!(*writes.lock().unwrap(), 1, "one write for one change");
        assert_eq!(
            Saved::parse(row.lock().unwrap().as_deref().unwrap()),
            Some(Saved::new(r, false, 1.25))
        );

        // Maximizing keeps the normal bounds and remembers the maximized state.
        states.track(
            Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            true,
            1.25,
        );
        states.persist();
        assert_eq!(
            Saved::parse(row.lock().unwrap().as_deref().unwrap()),
            Some(Saved::new(r, true, 1.25))
        );
        assert_eq!(*writes.lock().unwrap(), 2);
    }

    #[test]
    fn a_saved_placement_is_read_back_at_the_next_start() {
        let store = Memory::default();
        let row = store.row.clone();
        let s = Saved::new(
            Rect {
                x: 5,
                y: 6,
                width: 700,
                height: 500,
            },
            false,
            1.0,
        );
        *row.lock().unwrap() = Some(serde_json::to_string(&s).unwrap());
        let states = WindowStates::load(Ok(Box::new(store)));
        assert_eq!(states.initial(), Some(s));
    }

    #[test]
    fn an_unusable_store_means_the_default_and_no_saving() {
        let states = WindowStates::load(Err(StoreError::new("cannot open")));
        assert_eq!(states.initial(), None);
        states.track(
            Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            false,
            1.0,
        );
        states.persist(); // logged, nothing else

        let store = Memory {
            fail: true,
            ..Memory::default()
        };
        let states = WindowStates::load(Ok(Box::new(store)));
        states.track(
            Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            false,
            1.0,
        );
        states.persist(); // logged; tried again next time
        states.persist();
    }

    #[test]
    fn a_garbled_saved_row_is_ignored() {
        let store = Memory::default();
        *store.row.lock().unwrap() = Some("{\"format\":1".into());
        let states = WindowStates::load(Ok(Box::new(store)));
        assert_eq!(states.initial(), None);
    }
}

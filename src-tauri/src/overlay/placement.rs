//! Where the notes overlay opens and how big it is (ADR-052).
//!
//! - **A new overlay** (nothing remembered, remembering off, or after a
//!   reset) opens at the size of the settings, in logical pixels (so it is
//!   as big on a 150 % monitor as on a 100 % one), at the chosen place on
//!   the work area of the monitor the pointer is on (else the primary
//!   monitor): a corner, the middle of the left or right edge, centred
//!   vertically a third of the way across from the left or the right, or
//!   the centre; [`MARGIN`] away from its edges, never larger than that
//!   work area.
//! - **A remembered overlay** opens where it was, through the dashboard's
//!   own rules (`window_state::restore_placement`): only where it still
//!   overlaps a connected monitor, moved fully inside that monitor's work
//!   area, resized for a monitor whose scaling changed.
//! - **Collapsed**, only its title bar shows: the same place and width,
//!   [`COLLAPSED_HEIGHT`] logical pixels high.
//!
//! Plain functions on physical pixels, unit-tested without a display.

use crate::window_state::{Placement, Saved, Screen, restore_placement};

use super::model::{Anchor, COLLAPSED_HEIGHT, MIN_SIZE};

/// Between a new overlay and the work area's edges, logical pixels.
pub const MARGIN: f64 = 24.0;

/// Logical pixels on `scale` as physical ones, at least 1.
fn physical(logical: f64, scale: f64) -> u32 {
    let p = (logical * scale).round();
    if p.is_finite() && p >= 1.0 {
        p.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

/// The smallest overlay, logical pixels.
pub fn min_logical() -> (f64, f64) {
    (f64::from(MIN_SIZE.0), f64::from(MIN_SIZE.1))
}

/// A new overlay on `screen`: `size` (logical) at `anchor`.
pub fn default_placement(screen: &Screen, size: (u32, u32), anchor: Anchor) -> Placement {
    let work = screen.work;
    let margin = physical(MARGIN, screen.scale);
    let width =
        physical(f64::from(size.0), screen.scale).min(work.width.saturating_sub(2 * margin).max(1));
    let height = physical(f64::from(size.1), screen.scale)
        .min(work.height.saturating_sub(2 * margin).max(1));
    let left = i64::from(work.x) + i64::from(margin);
    let top = i64::from(work.y) + i64::from(margin);
    let right = i64::from(work.x) + i64::from(work.width) - i64::from(margin) - i64::from(width);
    let bottom = i64::from(work.y) + i64::from(work.height) - i64::from(margin) - i64::from(height);
    let centre_x = i64::from(work.x) + (i64::from(work.width) - i64::from(width)) / 2;
    let centre_y = i64::from(work.y) + (i64::from(work.height) - i64::from(height)) / 2;
    // Centred on the line a third of the way across: its near edge
    // `work / 3 - width / 2` in from the work area's edge (the same from
    // the right as from the left), never nearer than the margin, which
    // also keeps a wide overlay inside the far margin.
    let third = (2 * i64::from(work.width) - 3 * i64::from(width))
        .div_euclid(6)
        .max(i64::from(margin));
    let (x, y) = match anchor {
        Anchor::TopLeft => (left, top),
        Anchor::TopRight => (right, top),
        Anchor::BottomLeft => (left, bottom),
        Anchor::BottomRight => (right, bottom),
        Anchor::Center => (centre_x, centre_y),
        Anchor::CenterLeft => (left, centre_y),
        Anchor::CenterRight => (right, centre_y),
        Anchor::MidCenterLeft => (i64::from(work.x) + third, centre_y),
        Anchor::MidCenterRight => (
            i64::from(work.x) + i64::from(work.width) - i64::from(width) - third,
            centre_y,
        ),
    };
    Placement {
        x: i32::try_from(x).unwrap_or(work.x),
        y: i32::try_from(y).unwrap_or(work.y),
        width,
        height,
        maximized: false,
    }
}

/// A remembered overlay made visible on the connected `screens`, or
/// `None` when it is on none of them.
pub fn restore(saved: &Saved, screens: &[Screen]) -> Option<Placement> {
    restore_placement(saved, screens, min_logical(), (0, 0)).map(|p| Placement {
        maximized: false,
        ..p
    })
}

/// The monitor whose work area holds `point` (physical pixels), else the
/// one nearest to it, else `fallback`.
pub fn screen_at(
    screens: &[Screen],
    point: Option<(f64, f64)>,
    fallback: Option<Screen>,
) -> Option<Screen> {
    let Some((px, py)) = point.filter(|(x, y)| x.is_finite() && y.is_finite()) else {
        return fallback.or_else(|| screens.first().copied());
    };
    let distance = |s: &Screen| {
        let w = s.work;
        let (l, t) = (f64::from(w.x), f64::from(w.y));
        let (r, b) = (l + f64::from(w.width), t + f64::from(w.height));
        let dx = if px < l {
            l - px
        } else if px >= r {
            px - r + 1.0
        } else {
            0.0
        };
        let dy = if py < t {
            t - py
        } else if py >= b {
            py - b + 1.0
        } else {
            0.0
        };
        dx * dx + dy * dy
    };
    screens
        .iter()
        .copied()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .or(fallback)
}

/// The height of the collapsed overlay (its title bar) on a monitor of
/// `scale`, physical pixels: it keeps its place and width.
pub fn collapsed_height(scale: f64) -> u32 {
    physical(f64::from(COLLAPSED_HEIGHT), scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_state::Rect;

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

    fn at(p: Placement) -> (i32, i32, u32, u32) {
        (p.x, p.y, p.width, p.height)
    }

    #[test]
    fn a_new_overlay_opens_at_its_corner_with_a_margin() {
        // 1920 × 1080 with a 40 px taskbar at the bottom, 100 %.
        let s = screen(0, 0, 1920, 1040, 1.0);
        let size = (360, 440);
        assert_eq!(
            at(default_placement(&s, size, Anchor::TopRight)),
            (1920 - 24 - 360, 24, 360, 440)
        );
        assert_eq!(
            at(default_placement(&s, size, Anchor::TopLeft)),
            (24, 24, 360, 440)
        );
        assert_eq!(
            at(default_placement(&s, size, Anchor::BottomLeft)),
            (24, 1040 - 24 - 440, 360, 440)
        );
        assert_eq!(
            at(default_placement(&s, size, Anchor::BottomRight)),
            (1536, 576, 360, 440)
        );
        assert_eq!(
            at(default_placement(&s, size, Anchor::Center)),
            (780, 300, 360, 440)
        );
        assert!(!default_placement(&s, size, Anchor::Center).maximized);
    }

    #[test]
    fn the_middle_places_are_centred_vertically() {
        // 1920 × 1040 at 100 %: 300 px from the top; a third of the way
        // across puts the overlay's centre at 640 or 1280.
        let s = screen(0, 0, 1920, 1040, 1.0);
        let size = (360, 440);
        let place = |anchor| at(default_placement(&s, size, anchor));
        assert_eq!(place(Anchor::CenterLeft), (24, 300, 360, 440));
        assert_eq!(place(Anchor::MidCenterLeft), (640 - 180, 300, 360, 440));
        assert_eq!(place(Anchor::Center), (960 - 180, 300, 360, 440));
        assert_eq!(place(Anchor::MidCenterRight), (1280 - 180, 300, 360, 440));
        assert_eq!(place(Anchor::CenterRight), (1920 - 24 - 360, 300, 360, 440));
        // 150 %: 540 × 660 physical, the margin 36, the thirds at 1280 and
        // 2560 of 3840.
        let hi = screen(0, 0, 3840, 2100, 1.5);
        let place = |anchor| at(default_placement(&hi, size, anchor));
        assert_eq!(place(Anchor::CenterLeft), (36, 720, 540, 660));
        assert_eq!(place(Anchor::MidCenterLeft), (1280 - 270, 720, 540, 660));
        assert_eq!(place(Anchor::MidCenterRight), (2560 - 270, 720, 540, 660));
        assert_eq!(place(Anchor::CenterRight), (3840 - 36 - 540, 720, 540, 660));
    }

    #[test]
    fn a_third_across_is_the_same_from_either_side_and_inside_the_margins() {
        // Left of the primary, 1280 wide: a third is 426.67, so either
        // overlay's near edge is 246 px in from its side.
        let left = screen(-1280, 0, 1280, 984, 1.0);
        assert_eq!(
            at(default_placement(&left, (360, 440), Anchor::MidCenterLeft)),
            (-1280 + 246, 272, 360, 440)
        );
        assert_eq!(
            at(default_placement(&left, (360, 440), Anchor::MidCenterRight)),
            (-246 - 360, 272, 360, 440)
        );
        // Too wide to be centred there: at the margin of its own side.
        let s = screen(0, 0, 1920, 1040, 1.0);
        assert_eq!(
            at(default_placement(&s, (1600, 440), Anchor::MidCenterLeft)),
            (24, 300, 1600, 440)
        );
        assert_eq!(
            at(default_placement(&s, (1600, 440), Anchor::MidCenterRight)),
            (1920 - 24 - 1600, 300, 1600, 440)
        );
    }

    #[test]
    fn its_size_and_margin_are_logical_pixels() {
        // 150 %: 360 × 440 logical is 540 × 660 physical, the margin 36.
        let s = screen(0, 0, 3840, 2100, 1.5);
        assert_eq!(
            at(default_placement(&s, (360, 440), Anchor::TopRight)),
            (3840 - 36 - 540, 36, 540, 660)
        );
    }

    #[test]
    fn a_monitor_left_of_or_above_the_primary_has_negative_coordinates() {
        // The maintainer's layout: 2560 × 1440 above the primary at
        // (406, -1440).
        let s = screen(406, -1440, 2560, 1400, 1.0);
        assert_eq!(
            at(default_placement(&s, (360, 440), Anchor::TopLeft)),
            (430, -1416, 360, 440)
        );
        let left = screen(-1280, 0, 1280, 984, 1.0);
        assert_eq!(
            at(default_placement(&left, (360, 440), Anchor::BottomRight)),
            (-24 - 360, 984 - 24 - 440, 360, 440)
        );
    }

    #[test]
    fn it_is_never_larger_than_the_work_area() {
        let small = screen(0, 0, 800, 560, 1.0);
        let p = default_placement(&small, (1600, 1600), Anchor::TopRight);
        assert_eq!(at(p), (24, 24, 800 - 48, 560 - 48));
    }

    #[test]
    fn a_remembered_overlay_is_restored_where_it_still_fits() {
        let screens = [
            screen(0, 0, 1920, 1040, 1.0),
            screen(1920, 0, 2560, 1400, 1.0),
        ];
        let saved = Saved::new(
            Rect {
                x: 2000,
                y: 100,
                width: 400,
                height: 500,
            },
            false,
            1.0,
        );
        assert_eq!(
            at(restore(&saved, &screens).unwrap()),
            (2000, 100, 400, 500)
        );
        // The second monitor was unplugged: nothing to restore.
        assert_eq!(restore(&saved, &screens[..1]), None);
        // Partly off its monitor: moved inside.
        let edge = Saved::new(
            Rect {
                x: 1800,
                y: 900,
                width: 400,
                height: 500,
            },
            false,
            1.0,
        );
        assert_eq!(
            at(restore(&edge, &screens[..1]).unwrap()),
            (1520, 540, 400, 500)
        );
    }

    #[test]
    fn a_changed_scaling_keeps_the_logical_size_and_the_minimum_holds() {
        let screens = [screen(0, 0, 3840, 2100, 2.0)];
        let saved = Saved::new(
            Rect {
                x: 10,
                y: 10,
                width: 360,
                height: 440,
            },
            false,
            1.0,
        );
        assert_eq!(at(restore(&saved, &screens).unwrap()), (10, 10, 720, 880));
        let tiny = Saved::new(
            Rect {
                x: 10,
                y: 10,
                width: 50,
                height: 50,
            },
            true,
            1.0,
        );
        let p = restore(&tiny, &[screen(0, 0, 1920, 1040, 1.0)]).unwrap();
        assert_eq!((p.width, p.height), MIN_SIZE);
        assert!(!p.maximized, "an overlay is never maximized");
    }

    #[test]
    fn the_monitor_is_the_one_with_the_pointer() {
        let a = screen(0, 0, 1920, 1040, 1.0);
        let b = screen(406, -1440, 2560, 1400, 1.25);
        let screens = [a, b];
        assert_eq!(screen_at(&screens, Some((500.0, 500.0)), None), Some(a));
        assert_eq!(screen_at(&screens, Some((1000.0, -700.0)), None), Some(b));
        // On the taskbar (outside every work area): the nearest.
        assert_eq!(screen_at(&screens, Some((500.0, 1060.0)), None), Some(a));
        // No pointer: the fallback (the primary), else the first.
        assert_eq!(screen_at(&screens, None, Some(b)), Some(b));
        assert_eq!(screen_at(&screens, None, None), Some(a));
        assert_eq!(screen_at(&[], Some((1.0, 1.0)), Some(b)), Some(b));
        assert_eq!(screen_at(&screens, Some((f64::NAN, 1.0)), Some(b)), Some(b));
    }

    #[test]
    fn collapsed_it_is_as_high_as_its_title_bar() {
        assert_eq!(collapsed_height(1.0), COLLAPSED_HEIGHT);
        assert_eq!(collapsed_height(1.5), 57);
        assert_eq!(collapsed_height(2.0), 2 * COLLAPSED_HEIGHT);
        assert_eq!(collapsed_height(f64::NAN), 1);
    }
}

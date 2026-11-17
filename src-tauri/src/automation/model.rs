//! The autoclicker's data and pure rules (ADR-050): the clicker
//! configuration, macros and their actions, playback settings, profiles,
//! settings, shortcuts and their conflicts, screen geometry and the macro
//! file format. No I/O and no clock: everything here is checked by unit
//! tests.
//!
//! Every type the page or a file can send is **validated** before use
//! ([`Macro::validate`], [`Profile::validate`], [`Settings::validate`]):
//! values out of range are refused with a sentence the page shows, never
//! clamped silently. Macro and profile documents are JSON (camelCase
//! fields, kebab-case values), read with unknown fields refused, so a file
//! from a newer sotyware is never half-understood.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::keys::{Chord, Key};

/// The macro document format (`formatVersion`).
pub const MACRO_FORMAT_VERSION: u32 = 1;
/// The export file's format name and version.
pub const BUNDLE_FORMAT: &str = "sotyware-automation";
pub const BUNDLE_FORMAT_VERSION: u32 = 1;

/// At most this many actions in one macro (a recording stops there).
pub const MAX_ACTIONS: usize = 100_000;
/// The longest delay before one action, and the longest configured wait.
pub const MAX_DELAY_MS: u32 = 86_400_000;
pub const MAX_NAME: usize = 120;
pub const MAX_DESCRIPTION: usize = 2_000;
pub const MAX_FOLDER: usize = 80;
pub const MAX_POINTS: usize = 100;
pub const MAX_PROFILES: usize = 200;
pub const MAX_IGNORED: usize = 64;
/// Coordinates are kept within ±2^20 px: far beyond any screen, and safe
/// for every sum and product below.
pub const COORD_LIMIT: i32 = 1 << 20;
/// The fastest click rate: one click per millisecond.
pub const MIN_INTERVAL_MS: f64 = 1.0;
pub const MIN_SPEED: f64 = 0.1;
pub const MAX_SPEED: f64 = 20.0;
pub const MAX_REPEAT: u32 = 1_000_000;
pub const MAX_CLICKS: u64 = 100_000_000;

// ------------------------------------------------------------ geometry

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    fn check(self, what: &str) -> Result<(), String> {
        if self.x.abs() > COORD_LIMIT || self.y.abs() > COORD_LIMIT {
            return Err(format!(
                "{what} ({}, {}) is far outside any screen",
                self.x, self.y
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    pub fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    pub fn contains(self, p: Point) -> bool {
        i64::from(p.x) >= i64::from(self.x)
            && i64::from(p.x) < self.right()
            && i64::from(p.y) >= i64::from(self.y)
            && i64::from(p.y) < self.bottom()
    }

    /// `p` moved inside (on the nearest edge pixel).
    pub fn clamp(self, p: Point) -> Point {
        let clamp = |v: i32, lo: i32, len: u32| -> i32 {
            let hi = i64::from(lo) + i64::from(len.max(1)) - 1;
            i32::try_from(i64::from(v).clamp(i64::from(lo), hi)).unwrap_or(lo)
        };
        Point::new(
            clamp(p.x, self.x, self.width),
            clamp(p.y, self.y, self.height),
        )
    }

    fn check(self, what: &str) -> Result<(), String> {
        Point::new(self.x, self.y).check(what)?;
        if self.width == 0 || self.height == 0 {
            return Err(format!("{what} has no area"));
        }
        if self.width > COORD_LIMIT as u32 || self.height > COORD_LIMIT as u32 {
            return Err(format!("{what} is larger than any screen"));
        }
        Ok(())
    }
}

/// One display, in physical pixels of the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Monitor {
    pub bounds: Rect,
    pub primary: bool,
}

/// The rectangle around every monitor.
pub fn virtual_bounds(monitors: &[Monitor]) -> Option<Rect> {
    let first = monitors.first()?;
    let (mut l, mut t) = (i64::from(first.bounds.x), i64::from(first.bounds.y));
    let (mut r, mut b) = (first.bounds.right(), first.bounds.bottom());
    for m in &monitors[1..] {
        l = l.min(i64::from(m.bounds.x));
        t = t.min(i64::from(m.bounds.y));
        r = r.max(m.bounds.right());
        b = b.max(m.bounds.bottom());
    }
    Some(Rect::new(
        i32::try_from(l).ok()?,
        i32::try_from(t).ok()?,
        u32::try_from(r - l).ok()?,
        u32::try_from(b - t).ok()?,
    ))
}

/// The monitor that shows `p`, else the nearest one.
pub fn monitor_at(monitors: &[Monitor], p: Point) -> Option<usize> {
    if let Some(i) = monitors.iter().position(|m| m.bounds.contains(p)) {
        return Some(i);
    }
    monitors
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| {
            let c = m.bounds.clamp(p);
            (i64::from(c.x) - i64::from(p.x)).pow(2) + (i64::from(c.y) - i64::from(p.y)).pow(2)
        })
        .map(|(i, _)| i)
}

/// `p` on the current screens: inside a monitor (the nearest one's edge
/// if it fell between monitors or off every screen).
pub fn onto_screens(monitors: &[Monitor], p: Point) -> Point {
    match monitor_at(monitors, p) {
        Some(i) => monitors[i].bounds.clamp(p),
        None => p,
    }
}

/// Where a point recorded on the monitors `from` belongs on the monitors
/// `to` (resolution or arrangement changed): the same relative place on
/// the corresponding monitor. Monitors correspond by role: the primary to
/// the primary, the others in their order from left to right, top to
/// bottom; a monitor that is gone maps to the primary.
pub fn map_point(p: Point, from: &[Monitor], to: &[Monitor]) -> Point {
    let (Some(src), false) = (monitor_at(from, p), to.is_empty()) else {
        return p;
    };
    let order = |ms: &[Monitor]| -> Vec<usize> {
        let mut idx: Vec<usize> = (0..ms.len()).collect();
        idx.sort_by_key(|&i| (!ms[i].primary, ms[i].bounds.x, ms[i].bounds.y));
        idx
    };
    let (of, ot) = (order(from), order(to));
    let rank = of.iter().position(|&i| i == src).unwrap_or(0);
    let dst = ot.get(rank).copied().unwrap_or(ot[0]);
    let (a, b) = (from[src].bounds, to[dst].bounds);
    let scale = |v: i32, origin: i32, len_a: u32, origin_b: i32, len_b: u32| -> i32 {
        let rel = (f64::from(v) - f64::from(origin)) / f64::from(len_a.max(1));
        let mapped = f64::from(origin_b) + rel * f64::from(len_b);
        mapped.floor() as i32
    };
    let mapped = Point::new(
        scale(p.x, a.x, a.width, b.x, b.width),
        scale(p.y, a.y, a.height, b.y, b.height),
    );
    b.clamp(mapped)
}

/// Whether `p` is in one of the screen corners that stop automation (the
/// fail-safe): within `margin` px of a corner of the rectangle around every
/// monitor, where that corner lies on a monitor (a corner between monitors
/// of different sizes cannot be reached).
pub fn in_failsafe_corner(monitors: &[Monitor], p: Point, margin: u32) -> bool {
    let Some(v) = virtual_bounds(monitors) else {
        return false;
    };
    let m = i64::from(margin);
    let corners = [
        (i64::from(v.x), i64::from(v.y)),
        (v.right() - 1, i64::from(v.y)),
        (i64::from(v.x), v.bottom() - 1),
        (v.right() - 1, v.bottom() - 1),
    ];
    corners.iter().any(|&(cx, cy)| {
        let corner = Point::new(cx as i32, cy as i32);
        monitors.iter().any(|mon| mon.bounds.contains(corner))
            && (i64::from(p.x) - cx).abs() <= m
            && (i64::from(p.y) - cy).abs() <= m
    })
}

/// A window, as the native layer reports it: its title, the program's
/// file name and its client area on the virtual desktop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowInfo {
    pub title: String,
    pub process: String,
    pub client: Rect,
}

// ------------------------------------------------------------- inputs

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// The first extra button (XButton1, "back").
    Back,
    /// The second extra button (XButton2, "forward").
    Forward,
}

impl MouseButton {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Middle => "Middle",
            Self::Back => "Back",
            Self::Forward => "Forward",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Press {
    Down,
    Up,
}

// ------------------------------------------------------------- clicker

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClickType {
    Single,
    Double,
}

/// How the page shows the rate: clicks per second or the interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RateUnit {
    Cps,
    Interval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RandomMode {
    /// Every interval is exactly `intervalMs`.
    Off,
    /// Every interval is drawn evenly from `minMs`…`maxMs`.
    Range,
    /// Every interval is `intervalMs` ± `variationPercent`.
    Variation,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RandomInterval {
    pub mode: RandomMode,
    pub min_ms: f64,
    pub max_ms: f64,
    pub variation_percent: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetMode {
    /// Wherever the pointer is.
    Cursor,
    /// The listed points (one point: a fixed position).
    Points,
    /// A random point inside `area`.
    Area,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PointOrder {
    Sequential,
    Random,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClickTarget {
    pub mode: TargetMode,
    pub points: Vec<Point>,
    pub order: PointOrder,
    pub area: Option<Rect>,
    /// Each click lands within this many pixels of its point (0: exactly).
    pub spread: u32,
}

/// A pause after every `clicks` clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Batch {
    pub clicks: u32,
    pub pause_ms: u32,
}

/// The autoclicker's configuration (one per profile).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClickerConfig {
    pub button: MouseButton,
    pub click_type: ClickType,
    /// From one click's start to the next one's.
    pub interval_ms: f64,
    pub rate_unit: RateUnit,
    pub random: RandomInterval,
    /// Stop after this many clicks (a double-click counts once); `None`:
    /// until stopped.
    pub max_clicks: Option<u64>,
    /// Stop after this long (paused time not counted).
    pub max_duration_ms: Option<u64>,
    pub start_delay_ms: u32,
    pub batch: Option<Batch>,
    pub target: ClickTarget,
    /// How long the button is held for each click (0: pressed and released
    /// at once).
    pub hold_ms: u32,
    /// Between the two clicks of a double-click.
    pub double_gap_ms: u32,
    /// The page shows the advanced settings.
    pub advanced: bool,
}

impl Default for ClickerConfig {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            click_type: ClickType::Single,
            interval_ms: 100.0,
            rate_unit: RateUnit::Cps,
            random: RandomInterval {
                mode: RandomMode::Off,
                min_ms: 80.0,
                max_ms: 120.0,
                variation_percent: 20,
            },
            max_clicks: None,
            max_duration_ms: None,
            start_delay_ms: 0,
            batch: None,
            target: ClickTarget {
                mode: TargetMode::Cursor,
                points: Vec::new(),
                order: PointOrder::Sequential,
                area: None,
                spread: 0,
            },
            hold_ms: 0,
            double_gap_ms: 50,
            advanced: false,
        }
    }
}

fn check_ms(what: &str, v: f64) -> Result<(), String> {
    if !v.is_finite() || v < MIN_INTERVAL_MS || v > f64::from(MAX_DELAY_MS) {
        return Err(format!(
            "{what} must be between 1 ms and 24 hours (it is {v} ms)"
        ));
    }
    Ok(())
}

impl ClickerConfig {
    /// The shortest interval this configuration can produce.
    pub fn shortest_interval_ms(&self) -> f64 {
        match self.random.mode {
            RandomMode::Off => self.interval_ms,
            RandomMode::Range => self.random.min_ms,
            RandomMode::Variation => {
                self.interval_ms * (1.0 - f64::from(self.random.variation_percent) / 100.0)
            }
        }
    }

    /// The average interval (for the page's "about N clicks per second").
    pub fn mean_interval_ms(&self) -> f64 {
        match self.random.mode {
            RandomMode::Range => (self.random.min_ms + self.random.max_ms) / 2.0,
            _ => self.interval_ms,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        check_ms("The click interval", self.interval_ms)?;
        match self.random.mode {
            RandomMode::Off => {}
            RandomMode::Range => {
                check_ms("The shortest random interval", self.random.min_ms)?;
                check_ms("The longest random interval", self.random.max_ms)?;
                if self.random.min_ms > self.random.max_ms {
                    return Err(
                        "The shortest random interval is longer than the longest one".to_owned(),
                    );
                }
            }
            RandomMode::Variation => {
                if self.random.variation_percent == 0 || self.random.variation_percent > 90 {
                    return Err("The random variation must be between 1 % and 90 %".to_owned());
                }
            }
        }
        // Any random mode keeps its fields in range, so switching back is safe.
        if !self.random.min_ms.is_finite()
            || !self.random.max_ms.is_finite()
            || self.random.min_ms < 0.0
            || self.random.max_ms < 0.0
            || self.random.variation_percent > 90
        {
            return Err("The random interval settings are out of range".to_owned());
        }
        if self.max_clicks == Some(0) || self.max_clicks.is_some_and(|n| n > MAX_CLICKS) {
            return Err("The number of clicks must be between 1 and 100,000,000".to_owned());
        }
        if self
            .max_duration_ms
            .is_some_and(|d| d == 0 || d > u64::from(MAX_DELAY_MS) * 7)
        {
            return Err("The time limit must be between 1 ms and 7 days".to_owned());
        }
        if self.start_delay_ms > MAX_DELAY_MS {
            return Err("The start delay must be at most 24 hours".to_owned());
        }
        if let Some(b) = self.batch
            && (b.clicks == 0 || b.pause_ms > MAX_DELAY_MS)
        {
            return Err(
                "A batch needs at least one click and a pause of at most 24 hours".to_owned(),
            );
        }
        if self.hold_ms > 10_000 || self.double_gap_ms > 10_000 {
            return Err("Hold time and double-click gap must be at most 10 seconds".to_owned());
        }
        let per_click = match self.click_type {
            ClickType::Single => f64::from(self.hold_ms),
            ClickType::Double => 2.0 * f64::from(self.hold_ms) + f64::from(self.double_gap_ms),
        };
        if per_click >= self.shortest_interval_ms() {
            return Err(format!(
                "A click takes {per_click} ms (hold time{}), longer than the shortest interval ({} ms)",
                if self.click_type == ClickType::Double {
                    " and double-click gap"
                } else {
                    ""
                },
                self.shortest_interval_ms()
            ));
        }
        let t = &self.target;
        if t.points.len() > MAX_POINTS {
            return Err(format!("At most {MAX_POINTS} click positions"));
        }
        for (i, p) in t.points.iter().enumerate() {
            p.check(&format!("Position {}", i + 1))?;
        }
        if let Some(a) = t.area {
            a.check("The click area")?;
        }
        if t.spread > 10_000 {
            return Err("The spread must be at most 10,000 px".to_owned());
        }
        match t.mode {
            TargetMode::Cursor => {}
            TargetMode::Points if t.points.is_empty() => {
                return Err("Add at least one click position".to_owned());
            }
            TargetMode::Points => {}
            TargetMode::Area if t.area.is_none() => {
                return Err("Set the click area".to_owned());
            }
            TargetMode::Area => {}
        }
        Ok(())
    }

    /// Whether this configuration runs until it is stopped.
    pub fn unbounded(&self) -> bool {
        self.max_clicks.is_none() && self.max_duration_ms.is_none()
    }
}

// ------------------------------------------------------------ actions

/// What one macro step does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ActionKind {
    /// Move the pointer.
    Move { x: i32, y: i32 },
    /// Press or release a mouse button where the pointer was.
    Button {
        button: MouseButton,
        state: Press,
        x: i32,
        y: i32,
    },
    /// A whole click (press and release), `count` times, at a position or
    /// where the pointer is.
    Click {
        button: MouseButton,
        count: u8,
        x: Option<i32>,
        y: Option<i32>,
    },
    /// Turn the wheel: 120 is one notch; positive `dy` away from the user,
    /// positive `dx` to the right.
    Scroll {
        dx: i32,
        dy: i32,
        x: Option<i32>,
        y: Option<i32>,
    },
    /// Press or release one key.
    Key { key: Key, state: Press },
    /// Press a shortcut: the modifiers, the key, then release them all.
    Keys { chord: Chord },
    /// Only the delay.
    Wait,
    /// Playback pauses here until the user resumes it.
    Pause,
}

/// One macro step: wait `delay_ms` after the previous step, then act.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub delay_ms: u32,
    pub kind: ActionKind,
}

/// [`Action`] in JSON: the delay beside the kind's own fields.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActionWire {
    delay_ms: u32,
    #[serde(flatten)]
    kind: serde_json::Value,
}

impl Serialize for Action {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let kind = serde_json::to_value(&self.kind).map_err(serde::ser::Error::custom)?;
        ActionWire {
            delay_ms: self.delay_ms,
            kind,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let delay = map
            .remove("delayMs")
            .ok_or_else(|| serde::de::Error::custom("an action needs delayMs"))?;
        let delay_ms: u32 = serde_json::from_value(delay).map_err(serde::de::Error::custom)?;
        let keys: Vec<String> = map.keys().cloned().collect();
        let kind: ActionKind = serde_json::from_value(serde_json::Value::Object(map))
            .map_err(serde::de::Error::custom)?;
        // serde lets unit variants (`wait`, `pause`) of a tagged enum take
        // any field: every field must be one the kind has.
        let known = serde_json::to_value(&kind).map_err(serde::de::Error::custom)?;
        if let Some(extra) = keys.iter().find(|k| known.get(k.as_str()).is_none()) {
            return Err(serde::de::Error::custom(format!("unknown field '{extra}'")));
        }
        Ok(Self { delay_ms, kind })
    }
}

impl Action {
    pub fn new(delay_ms: u32, kind: ActionKind) -> Self {
        Self { delay_ms, kind }
    }

    /// Where the action puts the pointer first, if it does.
    pub fn position(&self) -> Option<Point> {
        match self.kind {
            ActionKind::Move { x, y } | ActionKind::Button { x, y, .. } => Some(Point::new(x, y)),
            ActionKind::Click {
                x: Some(x),
                y: Some(y),
                ..
            }
            | ActionKind::Scroll {
                x: Some(x),
                y: Some(y),
                ..
            } => Some(Point::new(x, y)),
            _ => None,
        }
    }

    /// A short English description (status line, logs).
    pub fn describe(&self) -> String {
        match &self.kind {
            ActionKind::Move { x, y } => format!("Move to {x}, {y}"),
            ActionKind::Button {
                button,
                state,
                x,
                y,
                ..
            } => format!(
                "{} button {} at {x}, {y}",
                button.label(),
                if *state == Press::Down { "down" } else { "up" }
            ),
            ActionKind::Click { button, count, .. } => match count {
                2 => format!("{} double-click", button.label()),
                _ => format!("{} click", button.label()),
            },
            ActionKind::Scroll { dx, dy, .. } => {
                if *dx == 0 {
                    format!("Scroll {}", if *dy > 0 { "up" } else { "down" })
                } else {
                    format!("Scroll {}", if *dx > 0 { "right" } else { "left" })
                }
            }
            ActionKind::Key { key, state } => format!(
                "{} {}",
                key.label(),
                if *state == Press::Down { "down" } else { "up" }
            ),
            ActionKind::Keys { chord } => {
                format!("Press {}", chord.label_with(|k| k.label().to_owned()))
            }
            ActionKind::Wait => format!("Wait {}", format_ms(u64::from(self.delay_ms))),
            ActionKind::Pause => "Pause until resumed".to_owned(),
        }
    }

    fn validate(&self, n: usize) -> Result<(), String> {
        let at = |m: String| format!("Action {n}: {m}");
        if self.delay_ms > MAX_DELAY_MS {
            return Err(at("the delay must be at most 24 hours".to_owned()));
        }
        if let Some(p) = self.position() {
            p.check("the position").map_err(at)?;
        }
        match &self.kind {
            ActionKind::Click { count, x, y, .. } => {
                if !(1..=2).contains(count) {
                    return Err(at("a click is single or double".to_owned()));
                }
                if x.is_some() != y.is_some() {
                    return Err(at("a position needs both x and y".to_owned()));
                }
            }
            ActionKind::Scroll { dx, dy, x, y } => {
                if (*dx == 0) == (*dy == 0) {
                    return Err(at("a scroll goes one way: up/down or left/right".to_owned()));
                }
                if dx.abs() > 120 * 100 || dy.abs() > 120 * 100 {
                    return Err(at("at most 100 notches in one scroll".to_owned()));
                }
                if x.is_some() != y.is_some() {
                    return Err(at("a position needs both x and y".to_owned()));
                }
            }
            ActionKind::Key { key, .. } => {
                let _ = key;
            }
            ActionKind::Keys { chord } => {
                let _ = chord;
            }
            _ => {}
        }
        Ok(())
    }
}

/// `1 h 2 min`, `3.5 s`, `250 ms`.
pub fn format_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        let s = ms as f64 / 1000.0;
        if ms.is_multiple_of(1000) {
            format!("{} s", ms / 1000)
        } else {
            format!("{s:.1} s")
        }
    } else if ms < 3_600_000 {
        format!("{} min {} s", ms / 60_000, (ms % 60_000) / 1000)
    } else {
        format!("{} h {} min", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}

// ------------------------------------------------------------ playback

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Positioning {
    /// The recorded screen positions.
    Absolute,
    /// Moved with the pointer: the recording's start becomes the pointer's
    /// position when playback starts.
    Cursor,
    /// Moved with the active window: positions keep their place in it.
    Window,
}

/// Actions `first`…`last` (indexes, inclusive) are the part repeated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Section {
    pub first: u32,
    pub last: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaybackSettings {
    /// How many times; `None`: until stopped.
    pub repeat: Option<u32>,
    /// Stop after this long (paused time not counted).
    pub stop_after_ms: Option<u64>,
    /// 2.0 plays twice as fast.
    pub speed: f64,
    pub start_delay_ms: u32,
    /// Between repetitions (not scaled by the speed).
    pub repeat_delay_ms: u32,
    /// Every delay varies by up to this much, at random (0–50 %).
    pub jitter_percent: u8,
    pub positioning: Positioning,
    /// Map positions to the current screens when they changed since the
    /// recording (absolute) or to the window's size (window).
    pub scale_to_screen: bool,
    /// Only this part repeats; what comes before and after plays once.
    pub section: Option<Section>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            repeat: Some(1),
            stop_after_ms: None,
            speed: 1.0,
            start_delay_ms: 0,
            repeat_delay_ms: 0,
            jitter_percent: 0,
            positioning: Positioning::Absolute,
            scale_to_screen: true,
            section: None,
        }
    }
}

impl PlaybackSettings {
    pub fn validate(&self, actions: Option<usize>) -> Result<(), String> {
        if self.repeat == Some(0) || self.repeat.is_some_and(|n| n > MAX_REPEAT) {
            return Err("Repetitions must be between 1 and 1,000,000".to_owned());
        }
        if self
            .stop_after_ms
            .is_some_and(|d| d == 0 || d > u64::from(MAX_DELAY_MS) * 7)
        {
            return Err("The time limit must be between 1 ms and 7 days".to_owned());
        }
        if !self.speed.is_finite() || self.speed < MIN_SPEED || self.speed > MAX_SPEED {
            return Err(format!(
                "The speed must be between {MIN_SPEED}× and {MAX_SPEED}×"
            ));
        }
        if self.start_delay_ms > MAX_DELAY_MS || self.repeat_delay_ms > MAX_DELAY_MS {
            return Err("Delays must be at most 24 hours".to_owned());
        }
        if self.jitter_percent > 50 {
            return Err("Random delay variation must be at most 50 %".to_owned());
        }
        if let Some(s) = self.section {
            if s.first > s.last {
                return Err("The repeated part ends before it starts".to_owned());
            }
            if let Some(n) = actions
                && s.last as usize >= n
            {
                return Err("The repeated part goes past the last action".to_owned());
            }
        }
        Ok(())
    }

    pub fn unbounded(&self) -> bool {
        self.repeat.is_none() && self.stop_after_ms.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mismatch {
    /// Go on, and say so.
    Warn,
    /// Stop playback.
    Stop,
}

/// The program a macro is meant for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetApp {
    /// The program's file name (`notepad.exe`), compared without case.
    pub process: String,
    /// Part of the window title, if it must match too.
    pub title: Option<String>,
    pub on_mismatch: Mismatch,
}

impl TargetApp {
    pub fn matches(&self, w: &WindowInfo) -> bool {
        w.process.eq_ignore_ascii_case(&self.process)
            && self
                .title
                .as_ref()
                .is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
    }

    fn validate(&self) -> Result<(), String> {
        if self.process.trim().is_empty() || self.process.len() > 260 {
            return Err("The target program needs a name".to_owned());
        }
        if self.title.as_ref().is_some_and(|t| t.len() > 260) {
            return Err("The target window title is too long".to_owned());
        }
        Ok(())
    }
}

/// Where and on what the macro was recorded: the reference for relative
/// and scaled playback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedOn {
    /// The pointer when recording started.
    pub cursor: Point,
    /// The window of the first click or key, if any.
    pub window: Option<WindowInfo>,
    pub monitors: Vec<Monitor>,
}

// --------------------------------------------------------------- macros

/// A shortcut that can be switched off without forgetting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub chord: Option<Chord>,
    pub enabled: bool,
}

impl Binding {
    pub const NONE: Self = Self {
        chord: None,
        enabled: false,
    };

    pub fn on(chord: &str) -> Self {
        Self {
            chord: Chord::parse(chord).ok(),
            enabled: true,
        }
    }

    /// The chord, if the binding is in use.
    pub fn active(self) -> Option<Chord> {
        self.chord.filter(|_| self.enabled)
    }
}

/// A recorded or written sequence of actions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Macro {
    pub format_version: u32,
    /// The sotyware version that last saved it.
    pub app_version: String,
    pub id: String,
    pub name: String,
    pub description: String,
    /// `""`: no folder.
    pub folder: String,
    pub favorite: bool,
    pub shortcut: Binding,
    pub created_ms: i64,
    pub modified_ms: i64,
    pub recorded_on: Option<RecordedOn>,
    pub target: Option<TargetApp>,
    pub playback: PlaybackSettings,
    pub actions: Vec<Action>,
}

fn check_text(what: &str, value: &str, max: usize, required: bool) -> Result<(), String> {
    if required && value.trim().is_empty() {
        return Err(format!("{what} cannot be empty"));
    }
    if value.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(format!("{what} contains control characters"));
    }
    Ok(())
}

/// A sotyware id: 1–64 lowercase letters, digits and `-`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Macro {
    pub fn new(id: String, name: String, now_ms: i64) -> Self {
        Self {
            format_version: MACRO_FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            id,
            name,
            description: String::new(),
            folder: String::new(),
            favorite: false,
            shortcut: Binding::NONE,
            created_ms: now_ms,
            modified_ms: now_ms,
            recorded_on: None,
            target: None,
            playback: PlaybackSettings::default(),
            actions: Vec::new(),
        }
    }

    /// Refuses anything a macro must not contain. A macro is data only:
    /// it is never executed as code, whatever a file says.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != MACRO_FORMAT_VERSION {
            return Err(format!(
                "This macro has format version {}; this sotyware reads version {MACRO_FORMAT_VERSION}",
                self.format_version
            ));
        }
        if !valid_id(&self.id) {
            return Err("The macro's id is not valid".to_owned());
        }
        check_text("The name", &self.name, MAX_NAME, true)?;
        check_text("The description", &self.description, MAX_DESCRIPTION, false)?;
        check_text("The folder", &self.folder, MAX_FOLDER, false)?;
        if self.folder.contains('\n') || self.name.contains('\n') {
            return Err("Names and folders are one line".to_owned());
        }
        if self.app_version.len() > 32 {
            return Err("The app version is not valid".to_owned());
        }
        if let Some(c) = self.shortcut.chord
            && let Some(p) = c.problem()
        {
            return Err(format!("Shortcut: {p}"));
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(format!("A macro has at most {MAX_ACTIONS} actions"));
        }
        for (i, a) in self.actions.iter().enumerate() {
            a.validate(i + 1)?;
        }
        if let Some(t) = &self.target {
            t.validate()?;
        }
        if let Some(r) = &self.recorded_on {
            r.cursor.check("The recording's start")?;
            if r.monitors.len() > 32 {
                return Err("Too many monitors".to_owned());
            }
            for m in &r.monitors {
                m.bounds.check("A recorded monitor")?;
            }
            if let Some(w) = &r.window {
                check_text("The recorded window title", &w.title, 1024, false)?;
                check_text("The recorded program", &w.process, 260, false)?;
            }
        }
        self.playback.validate(Some(self.actions.len()))
    }

    /// One pass at normal speed, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.actions.iter().map(|a| u64::from(a.delay_ms)).sum()
    }

    pub fn summary(&self) -> MacroSummary {
        MacroSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            folder: self.folder.clone(),
            favorite: self.favorite,
            shortcut: self.shortcut,
            actions: self.actions.len(),
            duration_ms: self.duration_ms(),
            created_ms: self.created_ms,
            modified_ms: self.modified_ms,
            target: self.target.as_ref().map(|t| t.process.clone()),
            has_pause: self
                .actions
                .iter()
                .any(|a| matches!(a.kind, ActionKind::Pause)),
        }
    }

    /// The first position an action moves to: the reference of relative
    /// playback when the macro has no recording information.
    pub fn first_position(&self) -> Option<Point> {
        self.actions.iter().find_map(Action::position)
    }
}

/// What the library lists for a macro (without its actions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub folder: String,
    pub favorite: bool,
    pub shortcut: Binding,
    pub actions: usize,
    pub duration_ms: u64,
    pub created_ms: i64,
    pub modified_ms: i64,
    pub target: Option<String>,
    pub has_pause: bool,
}

/// `name`, or `name (2)`, `name (3)`… whichever no name in `taken` has.
pub fn unique_name<'a>(name: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let free = |n: &str| !taken.clone().any(|t| t.eq_ignore_ascii_case(n));
    if free(name) {
        return name.to_owned();
    }
    let base = name.trim_end();
    (2..)
        .map(|i| {
            let suffix = format!(" ({i})");
            let room = MAX_NAME.saturating_sub(suffix.chars().count());
            let cut: String = base.chars().take(room).collect();
            format!("{cut}{suffix}")
        })
        .find(|n| free(n))
        .unwrap_or_else(|| name.to_owned())
}

// ------------------------------------------------------------ hotkeys

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HotkeyAction {
    ToggleClicker,
    StartClicker,
    PauseResume,
    StartRecording,
    StopRecording,
    PlayMacro,
    StopPlayback,
    /// The emergency stop: everything stops and every key and button is
    /// released. Shared by every profile.
    StopAll,
}

impl HotkeyAction {
    pub const PROFILE: [Self; 7] = [
        Self::ToggleClicker,
        Self::StartClicker,
        Self::PauseResume,
        Self::StartRecording,
        Self::StopRecording,
        Self::PlayMacro,
        Self::StopPlayback,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleClicker => "Start or stop the autoclicker",
            Self::StartClicker => "Start the autoclicker",
            Self::PauseResume => "Pause or resume",
            Self::StartRecording => "Start recording",
            Self::StopRecording => "Stop recording",
            Self::PlayMacro => "Play the profile's macro",
            Self::StopPlayback => "Stop playback",
            Self::StopAll => "Emergency stop",
        }
    }

    /// The suggested key (the default profile starts with it).
    pub fn suggestion(self) -> Option<&'static str> {
        match self {
            Self::ToggleClicker => Some("F6"),
            Self::PauseResume => Some("F7"),
            Self::StopAll => Some("F8"),
            Self::StartRecording => Some("F9"),
            Self::StopRecording => Some("F10"),
            Self::PlayMacro => Some("Shift+F11"),
            Self::StartClicker | Self::StopPlayback => None,
        }
    }
}

// ------------------------------------------------------------ profiles

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub clicker: ClickerConfig,
    /// The profile's shortcuts (not the emergency stop, which is shared).
    pub hotkeys: BTreeMap<HotkeyAction, Binding>,
    /// The macro "Play the profile's macro" plays.
    pub macro_id: Option<String>,
    /// Playback settings for that macro; `None`: the macro's own.
    pub playback: Option<PlaybackSettings>,
    pub created_ms: i64,
    pub modified_ms: i64,
}

impl Profile {
    /// A profile with the suggested shortcuts.
    pub fn new(id: String, name: String, now_ms: i64) -> Self {
        let hotkeys = HotkeyAction::PROFILE
            .iter()
            .map(|&a| (a, a.suggestion().map_or(Binding::NONE, Binding::on)))
            .collect();
        Self {
            id,
            name,
            clicker: ClickerConfig::default(),
            hotkeys,
            macro_id: None,
            playback: None,
            created_ms: now_ms,
            modified_ms: now_ms,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err("The profile's id is not valid".to_owned());
        }
        check_text("The profile name", &self.name, MAX_NAME, true)?;
        if self.name.contains('\n') {
            return Err("The profile name is one line".to_owned());
        }
        self.clicker.validate()?;
        if self.hotkeys.contains_key(&HotkeyAction::StopAll) {
            return Err("The emergency stop is set in Settings, for every profile".to_owned());
        }
        for (a, b) in &self.hotkeys {
            if let Some(c) = b.chord
                && let Some(p) = c.problem()
            {
                return Err(format!("{}: {p}", a.label()));
            }
        }
        if let Some(id) = &self.macro_id
            && !valid_id(id)
        {
            return Err("The profile's macro is not valid".to_owned());
        }
        if let Some(p) = &self.playback {
            p.validate(None)?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------ settings

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Coordinates {
    /// Positions as recorded; plays back at the same place on the screen.
    Absolute,
    /// Plays back relative to where the pointer is at the start.
    Relative,
    /// Plays back relative to the active window.
    Window,
}

/// What the recorder captures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingOptions {
    pub mouse_moves: bool,
    pub mouse_buttons: bool,
    pub keyboard: bool,
    pub scroll: bool,
    pub coordinates: Coordinates,
    /// At most this many pointer movements per second.
    pub move_rate_hz: u32,
    /// Movements shorter than this many pixels are not kept (the last
    /// position before a click or key always is).
    pub min_move_px: u32,
    pub ignored_keys: Vec<Key>,
    pub ignored_buttons: Vec<MouseButton>,
}

impl Default for RecordingOptions {
    fn default() -> Self {
        Self {
            mouse_moves: true,
            mouse_buttons: true,
            keyboard: true,
            scroll: true,
            coordinates: Coordinates::Absolute,
            move_rate_hz: 60,
            min_move_px: 3,
            ignored_keys: Vec::new(),
            ignored_buttons: Vec::new(),
        }
    }
}

impl RecordingOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.mouse_moves || self.mouse_buttons || self.keyboard || self.scroll) {
            return Err("Choose at least one kind of input to record".to_owned());
        }
        if !(1..=1000).contains(&self.move_rate_hz) {
            return Err("Pointer movements: between 1 and 1,000 per second".to_owned());
        }
        if self.min_move_px > 500 {
            return Err("The smallest movement kept must be at most 500 px".to_owned());
        }
        if self.ignored_keys.len() > MAX_IGNORED || self.ignored_buttons.len() > MAX_IGNORED {
            return Err("Too many ignored keys or buttons".to_owned());
        }
        Ok(())
    }
}

/// The autoclicker's settings, shared by every profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub default_profile: String,
    pub emergency_stop: Binding,
    /// Pushing the pointer into a corner of the screen stops automation.
    pub failsafe_corners: bool,
    /// A start from sotyware's own window waits at least this long, so the
    /// pointer can be moved where it should click.
    pub dashboard_countdown_ms: u32,
    pub recording: RecordingOptions,
}

impl Settings {
    pub fn new(default_profile: String) -> Self {
        Self {
            default_profile,
            emergency_stop: Binding::on("F8"),
            failsafe_corners: true,
            dashboard_countdown_ms: 3_000,
            recording: RecordingOptions::default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.default_profile) {
            return Err("The default profile is not valid".to_owned());
        }
        if let Some(c) = self.emergency_stop.chord
            && let Some(p) = c.problem()
        {
            return Err(format!("Emergency stop: {p}"));
        }
        if self.dashboard_countdown_ms > 10_000 {
            return Err("The countdown must be at most 10 seconds".to_owned());
        }
        self.recording.validate()
    }
}

// ------------------------------------------------------------ bindings

/// Who owns a shortcut.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BindingOwner {
    /// The emergency stop (Settings).
    Settings,
    /// An action of the active profile.
    Profile { action: HotkeyAction },
    /// A macro's own shortcut.
    Macro { id: String, name: String },
}

impl BindingOwner {
    pub fn label(&self) -> String {
        match self {
            Self::Settings => HotkeyAction::StopAll.label().to_owned(),
            Self::Profile { action } => action.label().to_owned(),
            Self::Macro { name, .. } => format!("Play \"{name}\""),
        }
    }
}

/// Every shortcut in use: the emergency stop, the active profile's and
/// each macro's (enabled ones only).
pub fn active_bindings(
    settings: &Settings,
    profile: Option<&Profile>,
    macros: &[MacroSummary],
) -> Vec<(BindingOwner, Chord)> {
    let mut out = Vec::new();
    if let Some(c) = settings.emergency_stop.active() {
        out.push((BindingOwner::Settings, c));
    }
    if let Some(p) = profile {
        for (&action, b) in &p.hotkeys {
            if let Some(c) = b.active() {
                out.push((BindingOwner::Profile { action }, c));
            }
        }
    }
    for m in macros {
        if let Some(c) = m.shortcut.active() {
            out.push((
                BindingOwner::Macro {
                    id: m.id.clone(),
                    name: m.name.clone(),
                },
                c,
            ));
        }
    }
    out
}

/// The first two bindings that share a chord, if any.
pub fn first_conflict(
    bindings: &[(BindingOwner, Chord)],
) -> Option<(&BindingOwner, &BindingOwner, Chord)> {
    for (i, (a, ca)) in bindings.iter().enumerate() {
        if let Some((b, _)) = bindings[i + 1..].iter().find(|(_, cb)| cb == ca) {
            return Some((a, b, *ca));
        }
    }
    None
}

/// Why `candidate` (owned by `owner`) cannot join `bindings`: the binding
/// that has its chord already.
pub fn conflict_with(
    bindings: &[(BindingOwner, Chord)],
    owner: &BindingOwner,
    candidate: Chord,
) -> Option<String> {
    bindings
        .iter()
        .find(|(o, c)| o != owner && *c == candidate)
        .map(|(o, _)| {
            format!(
                "{} is already the shortcut of \"{}\"",
                candidate.label_with(|k| k.label().to_owned()),
                o.label()
            )
        })
}

// --------------------------------------------------------- the file

/// The export file: macros and profiles, with the format and the version
/// of sotyware that wrote it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    #[serde(default)]
    pub macros: Vec<Macro>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

/// The largest file `import_automation` reads.
pub const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

impl Bundle {
    /// Reads a file's text: the envelope must be sotyware's, of a version
    /// this build reads; each macro and profile is checked separately by
    /// the caller, so one bad item does not refuse the others.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("This is not a JSON file: {e}"))?;
        let format = value.get("format").and_then(serde_json::Value::as_str);
        if format != Some(BUNDLE_FORMAT) {
            return Err("This is not a sotyware macro or profile file".to_owned());
        }
        let version = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(BUNDLE_FORMAT_VERSION)) {
            return Err(format!(
                "This file has format version {}; this sotyware reads version {BUNDLE_FORMAT_VERSION}",
                version.map_or_else(|| "?".to_owned(), |v| v.to_string())
            ));
        }
        serde_json::from_value(value).map_err(|e| format!("The file is damaged: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, y: i32, w: u32, h: u32, primary: bool) -> Monitor {
        Monitor {
            bounds: Rect::new(x, y, w, h),
            primary,
        }
    }

    #[test]
    fn the_virtual_screen_spans_every_monitor_including_negative_ones() {
        let ms = [
            mon(0, 0, 1920, 1080, true),
            mon(-1280, 200, 1280, 1024, false),
        ];
        assert_eq!(virtual_bounds(&ms), Some(Rect::new(-1280, 0, 3200, 1224)));
        assert_eq!(virtual_bounds(&[]), None);
    }

    #[test]
    fn points_between_monitors_go_onto_the_nearest_one() {
        let ms = [mon(0, 0, 1920, 1080, true), mon(1920, 0, 1280, 720, false)];
        assert_eq!(monitor_at(&ms, Point::new(2000, 100)), Some(1));
        // Below the smaller monitor: a dead zone of the virtual screen.
        assert_eq!(
            onto_screens(&ms, Point::new(2500, 900)),
            Point::new(2500, 719)
        );
        assert_eq!(
            onto_screens(&ms, Point::new(2000, 900)),
            Point::new(1919, 900)
        );
        assert_eq!(onto_screens(&ms, Point::new(-50, -50)), Point::new(0, 0));
        assert_eq!(onto_screens(&ms, Point::new(10, 10)), Point::new(10, 10));
    }

    #[test]
    fn positions_scale_with_their_monitor_when_the_resolution_changes() {
        let before = [mon(0, 0, 1920, 1080, true)];
        let after = [mon(0, 0, 2560, 1440, true)];
        assert_eq!(
            map_point(Point::new(960, 540), &before, &after),
            Point::new(1280, 720)
        );
        assert_eq!(
            map_point(Point::new(0, 0), &before, &after),
            Point::new(0, 0)
        );
        assert_eq!(
            map_point(Point::new(1919, 1079), &before, &after),
            Point::new(2558, 1438)
        );
    }

    #[test]
    fn monitors_correspond_by_role_when_the_arrangement_changes() {
        // Recorded: primary at 0, second monitor to its right.
        let before = [mon(0, 0, 1920, 1080, true), mon(1920, 0, 1920, 1080, false)];
        // Now: the second monitor moved to the left, and is smaller.
        let after = [mon(0, 0, 1920, 1080, true), mon(-1280, 0, 1280, 720, false)];
        let p = map_point(Point::new(1920 + 960, 540), &before, &after);
        assert_eq!(p, Point::new(-640, 360));
        // The primary stays the primary.
        assert_eq!(
            map_point(Point::new(100, 100), &before, &after),
            Point::new(100, 100)
        );
        // A monitor that is gone maps to the primary.
        let one = [mon(0, 0, 1280, 720, true)];
        assert_eq!(
            map_point(Point::new(1920 + 960, 540), &before, &one),
            Point::new(640, 360)
        );
    }

    #[test]
    fn only_reachable_corners_are_failsafe_corners() {
        let ms = [mon(0, 0, 1920, 1080, true), mon(1920, 0, 1280, 720, false)];
        assert!(in_failsafe_corner(&ms, Point::new(0, 0), 2));
        assert!(in_failsafe_corner(&ms, Point::new(1, 1079), 2));
        assert!(in_failsafe_corner(&ms, Point::new(3199, 0), 2));
        // (3199, 1079) is below the smaller monitor: not reachable.
        assert!(!in_failsafe_corner(&ms, Point::new(3199, 719), 2));
        assert!(!in_failsafe_corner(&ms, Point::new(500, 500), 2));
        assert!(!in_failsafe_corner(&ms, Point::new(4, 4), 2));
    }

    #[test]
    fn the_default_clicker_is_valid_and_bad_values_are_refused() {
        let ok = ClickerConfig::default();
        assert_eq!(ok.validate(), Ok(()));
        let bad = |f: &dyn Fn(&mut ClickerConfig)| {
            let mut c = ClickerConfig::default();
            f(&mut c);
            c.validate().unwrap_err()
        };
        bad(&|c| c.interval_ms = 0.5);
        bad(&|c| c.interval_ms = f64::NAN);
        bad(&|c| c.max_clicks = Some(0));
        bad(&|c| {
            c.random.mode = RandomMode::Range;
            c.random.min_ms = 200.0;
            c.random.max_ms = 100.0;
        });
        bad(&|c| c.target.mode = TargetMode::Points);
        bad(&|c| c.target.mode = TargetMode::Area);
        bad(&|c| c.target.points = vec![Point::new(COORD_LIMIT + 1, 0)]);
        assert!(
            bad(&|c| {
                c.click_type = ClickType::Double;
                c.interval_ms = 50.0;
                c.hold_ms = 10;
                c.double_gap_ms = 30;
            })
            .contains("longer than the shortest interval")
        );
    }

    #[test]
    fn actions_keep_their_delay_beside_their_own_fields_in_json() {
        let a = Action::new(
            120,
            ActionKind::Button {
                button: MouseButton::Left,
                state: Press::Down,
                x: 10,
                y: -20,
            },
        );
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"delayMs": 120, "type": "button", "button": "left", "state": "down", "x": 10, "y": -20})
        );
        assert_eq!(serde_json::from_value::<Action>(json).unwrap(), a);
        let keys = serde_json::json!({"delayMs": 0, "type": "keys", "chord": "Ctrl+KeyS"});
        assert!(serde_json::from_value::<Action>(keys).is_ok());
        for bad in [
            serde_json::json!({"type": "wait"}),
            serde_json::json!({"delayMs": 1, "type": "run", "command": "calc.exe"}),
            serde_json::json!({"delayMs": 1, "type": "wait", "extra": true}),
            serde_json::json!({"delayMs": -1, "type": "wait"}),
            serde_json::json!({"delayMs": 1, "type": "key", "key": "Power", "state": "down"}),
        ] {
            assert!(
                serde_json::from_value::<Action>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    fn sample_macro() -> Macro {
        let mut m = Macro::new("m-1".into(), "Login".into(), 1);
        m.actions = vec![
            Action::new(0, ActionKind::Move { x: 5, y: 5 }),
            Action::new(
                500,
                ActionKind::Click {
                    button: MouseButton::Left,
                    count: 1,
                    x: Some(5),
                    y: Some(5),
                },
            ),
            Action::new(1000, ActionKind::Wait),
        ];
        m
    }

    #[test]
    fn a_macro_round_trips_through_json_and_validates() {
        let m = sample_macro();
        assert_eq!(m.validate(), Ok(()));
        let text = serde_json::to_string_pretty(&m).unwrap();
        let back: Macro = serde_json::from_str(&text).unwrap();
        assert_eq!(back, m);
        assert_eq!(m.duration_ms(), 1500);
        let s = m.summary();
        assert_eq!((s.actions, s.duration_ms, s.has_pause), (3, 1500, false));
    }

    #[test]
    fn invalid_macros_are_refused_with_a_reason() {
        let check = |f: &dyn Fn(&mut Macro)| {
            let mut m = sample_macro();
            f(&mut m);
            m.validate().unwrap_err()
        };
        assert!(check(&|m| m.format_version = 2).contains("format version 2"));
        check(&|m| m.name = "  ".into());
        check(&|m| m.name = "a\u{7}".into());
        check(&|m| m.id = "../x".into());
        check(&|m| m.actions[1].delay_ms = MAX_DELAY_MS + 1);
        check(&|m| {
            m.actions.push(Action::new(
                0,
                ActionKind::Scroll {
                    dx: 120,
                    dy: 120,
                    x: None,
                    y: None,
                },
            ));
        });
        check(&|m| m.playback.speed = 0.0);
        check(&|m| m.playback.section = Some(Section { first: 1, last: 3 }));
        check(&|m| m.shortcut = Binding::on("Ctrl+F1").with_chord("KeyA"));
        let mut long = sample_macro();
        long.actions = vec![Action::new(0, ActionKind::Wait); MAX_ACTIONS + 1];
        assert!(long.validate().is_err());
    }

    impl Binding {
        fn with_chord(mut self, chord: &str) -> Self {
            self.chord = Some(Chord::parse(chord).unwrap());
            self
        }
    }

    #[test]
    fn names_are_made_unique_without_case() {
        let taken = ["Login", "login (2)"];
        assert_eq!(unique_name("Login", taken.iter().copied()), "Login (3)");
        assert_eq!(unique_name("Other", taken.iter().copied()), "Other");
    }

    #[test]
    fn a_new_profile_suggests_shortcuts_and_refuses_the_emergency_stop() {
        let p = Profile::new("p-1".into(), "Default".into(), 0);
        assert_eq!(p.validate(), Ok(()));
        assert_eq!(
            p.hotkeys[&HotkeyAction::ToggleClicker]
                .active()
                .map(|c| c.to_string()),
            Some("F6".into())
        );
        assert_eq!(p.hotkeys[&HotkeyAction::StartClicker], Binding::NONE);
        let mut bad = p.clone();
        bad.hotkeys
            .insert(HotkeyAction::StopAll, Binding::on("F12"));
        assert!(bad.validate().is_err());
    }

    #[test]
    fn conflicts_name_both_owners_and_ignore_disabled_bindings() {
        let settings = Settings::new("p-1".into());
        let mut p = Profile::new("p-1".into(), "Default".into(), 0);
        let mut m = sample_macro().summary();
        m.shortcut = Binding::on("F6");
        let all = active_bindings(&settings, Some(&p), std::slice::from_ref(&m));
        let (a, b, c) = first_conflict(&all).unwrap();
        assert_eq!(c.to_string(), "F6");
        assert_eq!(
            a,
            &BindingOwner::Profile {
                action: HotkeyAction::ToggleClicker
            }
        );
        assert!(matches!(b, BindingOwner::Macro { .. }));
        // Switched off, F6 is free.
        p.hotkeys
            .get_mut(&HotkeyAction::ToggleClicker)
            .unwrap()
            .enabled = false;
        let all = active_bindings(&settings, Some(&p), std::slice::from_ref(&m));
        assert_eq!(first_conflict(&all), None);
        let f8 = Chord::parse("F8").unwrap();
        let msg = conflict_with(
            &all,
            &BindingOwner::Profile {
                action: HotkeyAction::PauseResume,
            },
            f8,
        )
        .unwrap();
        assert!(msg.contains("Emergency stop"), "{msg}");
        assert_eq!(conflict_with(&all, &BindingOwner::Settings, f8), None);
    }

    #[test]
    fn only_sotyware_files_of_a_known_version_are_read() {
        assert!(Bundle::parse("not json").is_err());
        assert!(Bundle::parse(r#"{"format":"other"}"#).is_err());
        let newer = format!(r#"{{"format":"{BUNDLE_FORMAT}","formatVersion":2}}"#);
        assert!(Bundle::parse(&newer).unwrap_err().contains("version 2"));
        let ok = format!(
            r#"{{"format":"{BUNDLE_FORMAT}","formatVersion":1,"appVersion":"0.1.2","exportedAt":"x","macros":[]}}"#
        );
        assert_eq!(Bundle::parse(&ok).unwrap().macros.len(), 0);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_ms(250), "250 ms");
        assert_eq!(format_ms(3000), "3 s");
        assert_eq!(format_ms(3500), "3.5 s");
        assert_eq!(format_ms(125_000), "2 min 5 s");
        assert_eq!(format_ms(3_720_000), "1 h 2 min");
    }
}

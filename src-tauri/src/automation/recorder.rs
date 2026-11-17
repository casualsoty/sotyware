//! The macro recorder (ADR-050): turns the user's input, as the backend
//! captures it, into macro actions with their timing.
//!
//! Pure: the session feeds it [`RawEvent`]s with their time and whether
//! they happened on sotyware's own window; it keeps what the options ask
//! for. Rules:
//!
//! - **Timing:** each action keeps its time since the start (paused time
//!   not counted); delays are the differences of times rounded to the
//!   millisecond, so rounding never adds up. The idle time before the
//!   first action is dropped (playback has its own start delay).
//! - **Pointer movements** are kept at most `moveRateHz` times a second
//!   and only when they moved at least `minMovePx`; the last position
//!   before a click, a key or a scroll is always kept.
//! - **Shortcuts:** a key that, with the modifiers held, is one of the
//!   active shortcuts (stop recording, emergency stop…) is not recorded,
//!   and neither are the modifier presses just before it.
//! - **sotyware's own window:** clicks and scrolls on it, and keys typed
//!   while it is active, are not recorded (the click on Stop recording
//!   never ends up in the macro).
//! - A release whose press was not recorded (the shortcut that started the
//!   recording, a click that began before it) is dropped, and anything
//!   still held at the end is released by the macro's last actions.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::input::{RawEvent, RawKind};
use super::keys::{Chord, Key, Mods};
use super::model::{Action, ActionKind, MAX_ACTIONS, MouseButton, Point, Press, RecordingOptions};

/// What feeding one event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fed {
    Ignored,
    /// Added an action; `first_input` for the recording's first click or
    /// key (the session notes the window it went to).
    Added {
        first_input: bool,
    },
    /// The macro is full ([`MAX_ACTIONS`]); recording must stop.
    Full,
}

pub struct Recorder {
    options: RecordingOptions,
    start: Instant,
    paused_since: Option<Instant>,
    paused_total: Duration,
    shortcuts: Vec<Chord>,
    mods: Mods,
    /// Every action with its time since the start.
    actions: Vec<(Duration, ActionKind)>,
    keys_down: Vec<Key>,
    buttons_down: Vec<MouseButton>,
    /// Presses not recorded: their releases are dropped too.
    dropped_keys: BTreeSet<Key>,
    dropped_buttons: BTreeSet<MouseButton>,
    last_move: Option<(Duration, Point)>,
    pending_move: Option<(Duration, Point)>,
    had_input: bool,
    /// Keys and buttons sotyware cannot replay, or input it left out.
    pub skipped: u64,
}

impl Recorder {
    pub fn new(options: RecordingOptions, start: Instant, shortcuts: Vec<Chord>) -> Self {
        Self {
            options,
            start,
            paused_since: None,
            paused_total: Duration::ZERO,
            shortcuts,
            mods: Mods::default(),
            actions: Vec::new(),
            keys_down: Vec::new(),
            buttons_down: Vec::new(),
            dropped_keys: BTreeSet::new(),
            dropped_buttons: BTreeSet::new(),
            last_move: None,
            pending_move: None,
            had_input: false,
            skipped: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn paused(&self) -> bool {
        self.paused_since.is_some()
    }

    pub fn pause(&mut self, at: Instant) {
        if self.paused_since.is_none() {
            self.flush_move();
            self.paused_since = Some(at);
        }
    }

    pub fn resume(&mut self, at: Instant) {
        if let Some(since) = self.paused_since.take() {
            self.paused_total += at.saturating_duration_since(since);
            // A movement after the pause starts afresh: no jump recorded
            // as a slow glide.
            self.last_move = None;
        }
    }

    /// Recording time of `at`: since the start, without pauses (frozen at
    /// the pause's start while paused).
    fn time(&self, at: Instant) -> Duration {
        let at = self.paused_since.map_or(at, |p| p.min(at));
        at.saturating_duration_since(self.start)
            .saturating_sub(self.paused_total)
    }

    fn push(&mut self, at: Duration, kind: ActionKind) -> Fed {
        if self.actions.len() >= MAX_ACTIONS {
            return Fed::Full;
        }
        let input = matches!(
            kind,
            ActionKind::Button { .. } | ActionKind::Key { .. } | ActionKind::Scroll { .. }
        );
        let first_input = input && !self.had_input;
        self.had_input |= input;
        // Times never go backwards (events can arrive a little out of order
        // between the mouse and keyboard hooks).
        let at = self.actions.last().map_or(at, |(t, _)| at.max(*t));
        self.actions.push((at, kind));
        Fed::Added { first_input }
    }

    fn flush_move(&mut self) {
        if let Some((t, p)) = self.pending_move.take()
            && self.options.mouse_moves
            && self.last_move.is_none_or(|(_, lp)| lp != p)
        {
            self.last_move = Some((t, p));
            let _ = self.push(t, ActionKind::Move { x: p.x, y: p.y });
        }
    }

    /// Feeds one event; `own` is whether it happened on sotyware's window
    /// (under the pointer for mouse input, the active window for keys).
    pub fn feed(&mut self, ev: RawEvent, own: bool) -> Fed {
        if self.actions.len() >= MAX_ACTIONS {
            return Fed::Full;
        }
        let at = self.time(ev.at);
        match ev.kind {
            RawKind::Move(p) => {
                if self.paused() || !self.options.mouse_moves || own {
                    return Fed::Ignored;
                }
                if let Some((lt, lp)) = self.last_move {
                    let dx = i64::from(p.x) - i64::from(lp.x);
                    let dy = i64::from(p.y) - i64::from(lp.y);
                    let far = dx * dx + dy * dy >= i64::from(self.options.min_move_px).pow(2);
                    let late = at.saturating_sub(lt)
                        >= Duration::from_secs_f64(1.0 / f64::from(self.options.move_rate_hz));
                    if !(far && late) {
                        self.pending_move = Some((at, p));
                        return Fed::Ignored;
                    }
                }
                self.pending_move = None;
                self.last_move = Some((at, p));
                self.push(at, ActionKind::Move { x: p.x, y: p.y })
            }
            RawKind::Button(b, Press::Down, p) => {
                if self.paused()
                    || !self.options.mouse_buttons
                    || own
                    || self.options.ignored_buttons.contains(&b)
                {
                    self.dropped_buttons.insert(b);
                    if !self.paused() {
                        self.skipped += 1;
                    }
                    return Fed::Ignored;
                }
                self.dropped_buttons.remove(&b);
                if !self.buttons_down.contains(&b) {
                    self.buttons_down.push(b);
                }
                self.pending_move = Some((at, p));
                self.flush_move();
                self.push(
                    at,
                    ActionKind::Button {
                        button: b,
                        state: Press::Down,
                        x: p.x,
                        y: p.y,
                    },
                )
            }
            RawKind::Button(b, Press::Up, p) => {
                if self.dropped_buttons.remove(&b) || !self.buttons_down.contains(&b) {
                    return Fed::Ignored;
                }
                self.buttons_down.retain(|&x| x != b);
                if !self.paused() {
                    self.pending_move = Some((at, p));
                    self.flush_move();
                }
                self.push(
                    at,
                    ActionKind::Button {
                        button: b,
                        state: Press::Up,
                        x: p.x,
                        y: p.y,
                    },
                )
            }
            RawKind::Wheel { dx, dy, at: p } => {
                if self.paused() || !self.options.scroll || own {
                    return Fed::Ignored;
                }
                self.pending_move = Some((at, p));
                self.flush_move();
                self.push(
                    at,
                    ActionKind::Scroll {
                        dx,
                        dy,
                        x: Some(p.x),
                        y: Some(p.y),
                    },
                )
            }
            RawKind::Key(k, Press::Down) => self.key_down(k, at, own),
            RawKind::Key(k, Press::Up) => {
                if let Some(m) = k.modifier() {
                    self.mods.set(m, false);
                }
                if self.dropped_keys.remove(&k) || !self.keys_down.contains(&k) {
                    return Fed::Ignored;
                }
                self.keys_down.retain(|&x| x != k);
                self.push(
                    at,
                    ActionKind::Key {
                        key: k,
                        state: Press::Up,
                    },
                )
            }
            RawKind::OtherKey => {
                if !self.paused() && self.options.keyboard {
                    self.skipped += 1;
                }
                Fed::Ignored
            }
        }
    }

    fn key_down(&mut self, k: Key, at: Duration, own: bool) -> Fed {
        let held = self.mods;
        if let Some(m) = k.modifier() {
            self.mods.set(m, true);
        }
        let shortcut = k.modifier().is_none()
            && self
                .shortcuts
                .iter()
                .any(|c| c.key == k && c.mods() == held);
        if shortcut {
            self.dropped_keys.insert(k);
            self.strip_modifiers(held);
            return Fed::Ignored;
        }
        if self.paused() || !self.options.keyboard || own || self.options.ignored_keys.contains(&k)
        {
            self.dropped_keys.insert(k);
            if !self.paused() {
                self.skipped += 1;
            }
            return Fed::Ignored;
        }
        if self.dropped_keys.contains(&k) {
            // The auto-repeat of a key whose press was not recorded.
            return Fed::Ignored;
        }
        if k.modifier().is_some() && self.keys_down.contains(&k) {
            // A held modifier repeats; once is enough.
            return Fed::Ignored;
        }
        if !self.keys_down.contains(&k) {
            self.keys_down.push(k);
        }
        self.flush_move();
        self.push(
            at,
            ActionKind::Key {
                key: k,
                state: Press::Down,
            },
        )
    }

    /// Removes the modifier presses that were part of a shortcut: the
    /// trailing presses (only movements after them) of the modifiers in
    /// `mods`.
    fn strip_modifiers(&mut self, mods: Mods) {
        let mut i = self.actions.len();
        while i > 0 {
            i -= 1;
            match &self.actions[i].1 {
                ActionKind::Move { .. } => {}
                ActionKind::Key {
                    key,
                    state: Press::Down,
                } if key.modifier().is_some_and(|m| mods.has(m)) => {
                    let key = *key;
                    self.actions.remove(i);
                    self.keys_down.retain(|&x| x != key);
                    self.dropped_keys.insert(key);
                }
                _ => break,
            }
        }
    }

    /// The last `n` actions with their delays (the live list).
    pub fn recent(&self, n: usize) -> Vec<Action> {
        let from = self.actions.len().saturating_sub(n);
        let before = from.checked_sub(1).map(|i| self.actions[i].0);
        to_actions(&self.actions[from..], before)
    }

    /// The finished macro. `trim_moves`: drop the movements after the last
    /// click, key or scroll (stopped from the dashboard: they only lead to
    /// its Stop button).
    pub fn finish(mut self, end: Instant, trim_moves: bool) -> Vec<Action> {
        if !trim_moves {
            self.flush_move();
        }
        if trim_moves {
            while matches!(self.actions.last(), Some((_, ActionKind::Move { .. }))) {
                self.actions.pop();
            }
        }
        let end = self
            .actions
            .last()
            .map_or_else(|| self.time(end), |(t, _)| *t);
        for b in std::mem::take(&mut self.buttons_down).into_iter().rev() {
            let at = self
                .actions
                .iter()
                .rev()
                .find_map(|(_, a)| match a {
                    ActionKind::Button { button, x, y, .. } if *button == b => {
                        Some(Point::new(*x, *y))
                    }
                    _ => None,
                })
                .unwrap_or(Point::new(0, 0));
            self.actions.push((
                end,
                ActionKind::Button {
                    button: b,
                    state: Press::Up,
                    x: at.x,
                    y: at.y,
                },
            ));
        }
        for k in std::mem::take(&mut self.keys_down).into_iter().rev() {
            self.actions.push((
                end,
                ActionKind::Key {
                    key: k,
                    state: Press::Up,
                },
            ));
        }
        self.actions.truncate(MAX_ACTIONS);
        let first = self.actions.first().map(|(t, _)| *t);
        to_actions(&self.actions, first)
    }
}

/// Delays from times: each is the difference of the times rounded to the
/// millisecond, so rounding does not add up over a long macro. `before`
/// is the time the first delay counts from.
fn to_actions(timed: &[(Duration, ActionKind)], before: Option<Duration>) -> Vec<Action> {
    let round = |d: Duration| (d.as_micros() + 500) / 1000;
    let mut prev = before.map_or(0, round);
    timed
        .iter()
        .map(|(t, kind)| {
            let now = round(*t).max(prev);
            let delay = u32::try_from(now - prev).unwrap_or(super::model::MAX_DELAY_MS);
            prev = now;
            Action::new(delay.min(super::model::MAX_DELAY_MS), kind.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str) -> Key {
        Key::from_code(code).unwrap()
    }

    struct Rec {
        t0: Instant,
        r: Recorder,
    }

    impl Rec {
        fn new(options: RecordingOptions, shortcuts: &[&str]) -> Self {
            let t0 = Instant::now();
            Self {
                t0,
                r: Recorder::new(
                    options,
                    t0,
                    shortcuts.iter().map(|s| Chord::parse(s).unwrap()).collect(),
                ),
            }
        }

        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }

        fn feed(&mut self, ms: u64, kind: RawKind) -> Fed {
            let at = self.at(ms);
            self.r.feed(RawEvent { at, kind }, false)
        }

        fn feed_own(&mut self, ms: u64, kind: RawKind) -> Fed {
            let at = self.at(ms);
            self.r.feed(RawEvent { at, kind }, true)
        }

        fn finish(self, ms: u64, trim: bool) -> Vec<Action> {
            let end = self.at(ms);
            self.r.finish(end, trim)
        }
    }

    fn mv(x: i32, y: i32) -> RawKind {
        RawKind::Move(Point::new(x, y))
    }

    fn btn(b: MouseButton, s: Press, x: i32, y: i32) -> RawKind {
        RawKind::Button(b, s, Point::new(x, y))
    }

    fn k(code: &str, s: Press) -> RawKind {
        RawKind::Key(key(code), s)
    }

    fn kinds(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .map(|a| format!("{}:{}", a.delay_ms, a.describe()))
            .collect()
    }

    #[test]
    fn the_example_sequence_keeps_its_order_and_timing() {
        // Move to A, left-click, wait 500 ms, press a shortcut, wait 1 s,
        // move to B, right-click.
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(100, mv(10, 10));
        r.feed(150, btn(MouseButton::Left, Press::Down, 10, 10));
        r.feed(230, btn(MouseButton::Left, Press::Up, 10, 10));
        r.feed(730, k("ControlLeft", Press::Down));
        r.feed(760, k("KeyS", Press::Down));
        r.feed(820, k("KeyS", Press::Up));
        r.feed(850, k("ControlLeft", Press::Up));
        r.feed(1850, mv(300, 400));
        r.feed(1900, btn(MouseButton::Right, Press::Down, 300, 400));
        r.feed(1980, btn(MouseButton::Right, Press::Up, 300, 400));
        let actions = r.finish(2500, false);
        assert_eq!(
            kinds(&actions),
            [
                "0:Move to 10, 10",
                "50:Left button down at 10, 10",
                "80:Left button up at 10, 10",
                "500:Left Ctrl down",
                "30:S down",
                "60:S up",
                "30:Left Ctrl up",
                "1000:Move to 300, 400",
                "50:Right button down at 300, 400",
                "80:Right button up at 300, 400",
            ]
        );
    }

    #[test]
    fn delays_add_up_to_the_real_time_without_rounding_drift() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        let t0 = r.t0;
        // 1,000 key presses 1.4 ms apart: rounding each delay alone would
        // lose 0.4 ms every time.
        for i in 0..1000u64 {
            let at = t0 + Duration::from_micros(1_400 * i);
            r.r.feed(
                RawEvent {
                    at,
                    kind: k(if i % 2 == 0 { "KeyA" } else { "KeyB" }, Press::Down),
                },
                false,
            );
        }
        let actions = r.finish(5_000, false);
        let total: u64 = actions.iter().map(|a| u64::from(a.delay_ms)).sum();
        assert_eq!(total, 1399, "999 gaps of 1.4 ms, rounded once");
    }

    #[test]
    fn movements_are_sampled_but_the_position_before_a_click_is_exact() {
        let options = RecordingOptions {
            move_rate_hz: 10,
            min_move_px: 5,
            ..RecordingOptions::default()
        };
        let mut r = Rec::new(options, &[]);
        r.feed(0, mv(0, 0));
        r.feed(10, mv(1, 1)); // too close and too soon
        r.feed(50, mv(50, 50)); // far but too soon
        r.feed(120, mv(60, 60)); // kept (100 ms after the last kept)
        r.feed(130, mv(70, 70)); // pending
        r.feed(140, btn(MouseButton::Left, Press::Down, 71, 71));
        r.feed(150, btn(MouseButton::Left, Press::Up, 71, 71));
        let actions = r.finish(200, false);
        assert_eq!(
            kinds(&actions),
            [
                "0:Move to 0, 0",
                "120:Move to 60, 60",
                "20:Move to 71, 71",
                "0:Left button down at 71, 71",
                "10:Left button up at 71, 71",
            ]
        );
    }

    #[test]
    fn the_stop_shortcut_and_its_modifiers_are_not_recorded() {
        let mut r = Rec::new(RecordingOptions::default(), &["Ctrl+Shift+F10", "F8"]);
        r.feed(0, k("KeyA", Press::Down));
        r.feed(10, k("KeyA", Press::Up));
        r.feed(100, k("ControlLeft", Press::Down));
        r.feed(110, mv(5, 5));
        r.feed(120, k("ShiftLeft", Press::Down));
        r.feed(130, k("F10", Press::Down));
        // The modifiers' releases come after the shortcut ended recording
        // here, but are dropped in any case.
        r.feed(140, k("F10", Press::Up));
        r.feed(150, k("ShiftLeft", Press::Up));
        r.feed(160, k("ControlLeft", Press::Up));
        // F8 alone (the emergency stop) is not recorded either.
        r.feed(200, k("F8", Press::Down));
        r.feed(210, k("F8", Press::Up));
        let actions = r.finish(300, false);
        assert_eq!(kinds(&actions), ["0:A down", "10:A up", "100:Move to 5, 5"]);
    }

    #[test]
    fn a_key_typed_with_other_modifiers_than_a_shortcut_is_recorded() {
        let mut r = Rec::new(RecordingOptions::default(), &["Ctrl+F10"]);
        r.feed(0, k("F10", Press::Down));
        r.feed(10, k("F10", Press::Up));
        let actions = r.finish(20, false);
        assert_eq!(kinds(&actions), ["0:F10 down", "10:F10 up"]);
    }

    #[test]
    fn input_on_sotyware_is_not_recorded_and_unmatched_releases_are_dropped() {
        let mut r = Rec::new(RecordingOptions::default(), &["F9"]);
        // The release of the click on "Start recording" and of the F9 that
        // started it.
        r.feed(5, btn(MouseButton::Left, Press::Up, 900, 900));
        r.feed(6, k("F9", Press::Up));
        r.feed(100, btn(MouseButton::Left, Press::Down, 10, 10));
        r.feed(150, btn(MouseButton::Left, Press::Up, 10, 10));
        // Then the click on Stop recording, on sotyware's window.
        r.feed_own(900, mv(900, 900));
        assert_eq!(
            r.feed_own(950, btn(MouseButton::Left, Press::Down, 900, 900)),
            Fed::Ignored
        );
        r.feed_own(960, k("KeyX", Press::Down));
        let skipped = r.r.skipped;
        let actions = r.finish(1000, true);
        assert_eq!(
            kinds(&actions),
            [
                "0:Move to 10, 10",
                "0:Left button down at 10, 10",
                "50:Left button up at 10, 10"
            ]
        );
        assert_eq!(skipped, 2);
    }

    #[test]
    fn trailing_movement_is_trimmed_only_when_asked() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(0, k("KeyA", Press::Down));
        r.feed(10, k("KeyA", Press::Up));
        r.feed(500, mv(100, 100));
        r.feed(600, mv(200, 200));
        let kept = Rec {
            t0: r.t0,
            r: Recorder {
                actions: r.r.actions.clone(),
                ..Recorder::new(RecordingOptions::default(), r.t0, vec![])
            },
        };
        assert_eq!(r.finish(700, true).len(), 2);
        assert_eq!(kept.finish(700, false).len(), 4);
    }

    #[test]
    fn pauses_are_not_recorded_and_not_counted_as_time() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(0, k("KeyA", Press::Down));
        r.feed(100, k("KeyA", Press::Up));
        let at = r.at(200);
        r.r.pause(at);
        r.feed(300, k("KeyB", Press::Down));
        r.feed(400, k("KeyB", Press::Up));
        r.feed(500, btn(MouseButton::Left, Press::Down, 1, 1));
        let at = r.at(5_200);
        r.r.resume(at);
        // The release of a click pressed during the pause is not recorded.
        r.feed(5_250, btn(MouseButton::Left, Press::Up, 1, 1));
        r.feed(5_300, k("KeyC", Press::Down));
        r.feed(5_350, k("KeyC", Press::Up));
        let actions = r.finish(6_000, false);
        assert_eq!(
            kinds(&actions),
            ["0:A down", "100:A up", "200:C down", "50:C up"]
        );
    }

    #[test]
    fn a_key_released_during_a_pause_is_released_at_the_pause() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(0, k("ShiftLeft", Press::Down));
        let at = r.at(300);
        r.r.pause(at);
        r.feed(800, k("ShiftLeft", Press::Up));
        let actions = r.finish(1_000, false);
        assert_eq!(kinds(&actions), ["0:Left Shift down", "300:Left Shift up"]);
    }

    #[test]
    fn options_choose_what_is_recorded() {
        let options = RecordingOptions {
            mouse_moves: false,
            keyboard: false,
            ignored_buttons: vec![MouseButton::Right],
            ..RecordingOptions::default()
        };
        let mut r = Rec::new(options, &[]);
        r.feed(0, mv(1, 1));
        r.feed(10, k("KeyA", Press::Down));
        r.feed(20, btn(MouseButton::Right, Press::Down, 5, 5));
        r.feed(30, btn(MouseButton::Right, Press::Up, 5, 5));
        r.feed(40, btn(MouseButton::Left, Press::Down, 5, 5));
        r.feed(50, btn(MouseButton::Left, Press::Up, 5, 5));
        r.feed(
            60,
            RawKind::Wheel {
                dx: 0,
                dy: -120,
                at: Point::new(5, 5),
            },
        );
        r.feed(70, RawKind::OtherKey);
        let skipped = r.r.skipped;
        let actions = r.finish(100, false);
        assert_eq!(
            kinds(&actions),
            [
                "0:Left button down at 5, 5",
                "10:Left button up at 5, 5",
                "10:Scroll down",
            ]
        );
        // The A press and the right click were left out (the unknown key
        // too, since the keyboard is not recorded at all).
        assert_eq!(skipped, 2);
    }

    #[test]
    fn auto_repeat_is_kept_for_keys_but_not_for_modifiers() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(0, k("ShiftLeft", Press::Down));
        r.feed(500, k("ShiftLeft", Press::Down));
        r.feed(530, k("ShiftLeft", Press::Down));
        r.feed(600, k("Backspace", Press::Down));
        r.feed(1100, k("Backspace", Press::Down));
        r.feed(1130, k("Backspace", Press::Up));
        r.feed(1200, k("ShiftLeft", Press::Up));
        let actions = r.finish(1300, false);
        assert_eq!(
            kinds(&actions),
            [
                "0:Left Shift down",
                "600:Backspace down",
                "500:Backspace down",
                "30:Backspace up",
                "70:Left Shift up",
            ]
        );
    }

    #[test]
    fn what_is_still_held_at_the_end_is_released() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        r.feed(0, btn(MouseButton::Left, Press::Down, 4, 4));
        r.feed(10, k("AltLeft", Press::Down));
        let actions = r.finish(100, true);
        assert_eq!(
            kinds(&actions),
            [
                "0:Move to 4, 4",
                "0:Left button down at 4, 4",
                "10:Left Alt down",
                "0:Left button up at 4, 4",
                "0:Left Alt up",
            ]
        );
    }

    #[test]
    fn the_first_click_or_key_is_reported_once() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        assert_eq!(r.feed(0, mv(1, 1)), Fed::Added { first_input: false });
        assert_eq!(
            r.feed(5, k("KeyA", Press::Down)),
            Fed::Added { first_input: true }
        );
        assert_eq!(
            r.feed(6, k("KeyA", Press::Up)),
            Fed::Added { first_input: false }
        );
    }

    #[test]
    fn the_live_list_shows_the_last_actions_with_their_delays() {
        let mut r = Rec::new(RecordingOptions::default(), &[]);
        for i in 0..10u64 {
            r.feed(
                i * 100,
                k("KeyA", if i % 2 == 0 { Press::Down } else { Press::Up }),
            );
        }
        let recent = r.r.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent.iter().all(|a| a.delay_ms == 100));
    }
}

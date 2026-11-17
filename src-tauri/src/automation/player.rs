//! The macro playback engine (ADR-050): replays a macro's actions with
//! their timing on the session's own thread.
//!
//! - **Timing:** each action happens its delay after the previous one,
//!   divided by the speed (and varied by the jitter); times are absolute
//!   from the start, so a long macro does not drift. The delay between
//!   repetitions and the start delay are not scaled by the speed.
//! - **Repetition:** the whole macro, or only its section: what comes
//!   before the section plays once, the section repeats, what comes after
//!   plays once.
//! - **Positions** are placed when playback starts (after the start delay,
//!   so the pointer and the window can be put in place meanwhile):
//!   absolute (mapped to the current monitors if they changed and scaling
//!   is on), relative to the pointer, or relative to the active window.
//!   Every point ends up on a screen.
//! - **Safety:** the fail-safe corners are checked before every action; a
//!   press on sotyware's own window, or a key while sotyware is active, is
//!   skipped with its release; a target program that is not active stops
//!   or warns; a pause releases everything held and presses it again on
//!   resume; the end, a stop or an error releases everything.

use std::collections::BTreeSet;
use std::time::Duration;

use super::clicker::{Outcome, RunState, pushed_into_corner};
use super::input::{Backend, Output, Synth};
use super::keys::Key;
use super::model::{
    ActionKind, Macro, Mismatch, Monitor, MouseButton, PlaybackSettings, Point, Positioning, Press,
    Rect, onto_screens,
};
use super::timing::{Clock, Control, Rng, StopReason, Wake};

/// What the page shows of a running playback.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackProgress {
    pub state: RunState,
    /// 1-based.
    pub repetition: u32,
    pub repetitions: Option<u32>,
    /// The action played last (0-based index into the macro).
    pub action: usize,
    pub actions: usize,
    pub label: String,
    /// 0…1, when the end is known.
    pub progress: Option<f64>,
    pub elapsed_ms: u64,
    pub remaining_ms: Option<u64>,
    pub countdown_ms: Option<u64>,
    /// Actions skipped because sotyware's window was under them or active.
    pub skipped: u64,
    pub warning: Option<String>,
    /// Paused by a pause action (resume to go on).
    pub at_pause_point: bool,
}

/// Where recorded positions go now.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    Same,
    Offset { dx: i64, dy: i64 },
    Screens { from: Vec<Monitor> },
    Window { from: Rect, to: Rect, scale: bool },
}

impl Placement {
    /// Decides the placement once, at the start. Fails when it cannot be
    /// decided (a window-relative macro without its window).
    pub fn decide(
        m: &Macro,
        s: &PlaybackSettings,
        backend: &dyn Backend,
        monitors: &[Monitor],
    ) -> Result<Self, String> {
        let recorded = m.recorded_on.as_ref();
        Ok(match s.positioning {
            Positioning::Absolute => match recorded {
                Some(r)
                    if s.scale_to_screen && !r.monitors.is_empty() && r.monitors != monitors =>
                {
                    Self::Screens {
                        from: r.monitors.clone(),
                    }
                }
                _ => Self::Same,
            },
            Positioning::Cursor => {
                let reference = recorded.map(|r| r.cursor).or_else(|| m.first_position());
                match reference {
                    None => Self::Same,
                    Some(r) => {
                        let now = backend.cursor()?;
                        Self::Offset {
                            dx: i64::from(now.x) - i64::from(r.x),
                            dy: i64::from(now.y) - i64::from(r.y),
                        }
                    }
                }
            }
            Positioning::Window => {
                let from = recorded.and_then(|r| r.window.as_ref()).map(|w| w.client).ok_or(
                    "This macro has no recorded window to play relative to; record it again with window positions, or play it with absolute positions",
                )?;
                let to = backend
                    .foreground()
                    .map(|w| w.client)
                    .ok_or("No window is active to play relative to")?;
                Self::Window {
                    from,
                    to,
                    scale: s.scale_to_screen,
                }
            }
        })
    }

    pub fn place(&self, p: Point, monitors: &[Monitor]) -> Point {
        let placed = match self {
            Self::Same => p,
            Self::Offset { dx, dy } => Point::new(
                (i64::from(p.x) + dx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                (i64::from(p.y) + dy).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            ),
            Self::Screens { from } => super::model::map_point(p, from, monitors),
            Self::Window { from, to, scale } => {
                let axis = |v: i32, f0: i32, flen: u32, t0: i32, tlen: u32| -> i32 {
                    let rel = f64::from(v) - f64::from(f0);
                    let rel = if *scale {
                        rel * f64::from(tlen) / f64::from(flen.max(1))
                    } else {
                        rel
                    };
                    (f64::from(t0) + rel).round() as i32
                };
                Point::new(
                    axis(p.x, from.x, from.width, to.x, to.width),
                    axis(p.y, from.y, from.height, to.y, to.height),
                )
            }
        };
        onto_screens(monitors, placed)
    }
}

pub struct PlaybackRun<'a> {
    pub macro_: &'a Macro,
    pub settings: &'a PlaybackSettings,
    pub start_delay: Duration,
    pub clock: &'a dyn Clock,
    pub control: &'a Control,
    pub rng: Rng,
    pub failsafe: bool,
    pub monitors: Vec<Monitor>,
    pub report: &'a (dyn Fn(&PlaybackProgress) + Sync),
}

struct State {
    progress: PlaybackProgress,
    /// Active time of the next action, in ms since the start.
    t_ms: f64,
    shift: Duration,
    origin: Duration,
    placement: Placement,
    /// Where playback last moved the pointer: a recorded position in a
    /// corner is not the user pushing the pointer there.
    placed: Option<Point>,
    skipped_buttons: BTreeSet<MouseButton>,
    skipped_keys: BTreeSet<Key>,
    /// The planned length of the whole playback (when bounded).
    planned_ms: Option<f64>,
    planned_actions: Option<u64>,
    played: u64,
}

/// Why playback cannot go on.
enum Halt {
    Stop,
    Done(String),
}

impl PlaybackRun<'_> {
    fn active(&self, st: &State) -> Duration {
        self.clock.now().saturating_sub(st.origin + st.shift)
    }

    /// Waits until active time `at_ms`; pauses release what is held and
    /// press it again on resume.
    fn wait(&self, at_ms: f64, st: &mut State, out: &mut Output) -> Result<(), Halt> {
        loop {
            let deadline = st.origin + st.shift + Duration::from_secs_f64(at_ms.max(0.0) / 1000.0);
            match self.clock.wait_until(deadline, self.control) {
                Wake::Reached => return Ok(()),
                Wake::Stop => return Err(Halt::Stop),
                Wake::Pause => self.pause(st, out)?,
            }
        }
    }

    fn pause(&self, st: &mut State, out: &mut Output) -> Result<(), Halt> {
        let held = out.release_all();
        st.progress.state = RunState::Paused;
        (self.report)(&st.progress);
        let Some(paused) = self.clock.wait_resumed(self.control) else {
            return Err(Halt::Stop);
        };
        st.shift += paused;
        st.progress.state = RunState::Running;
        st.progress.at_pause_point = false;
        if let Err(e) = out.press_again(&held) {
            self.control
                .stop(StopReason::Failed(format!("input could not be sent: {e}")));
            return Err(Halt::Stop);
        }
        (self.report)(&st.progress);
        Ok(())
    }

    fn jitter(&mut self) -> f64 {
        let j = f64::from(self.settings.jitter_percent) / 100.0;
        if j == 0.0 {
            1.0
        } else {
            self.rng.range_f64(1.0 - j, 1.0 + j)
        }
    }

    fn send(&self, out: &mut Output, batch: &[Synth]) -> Result<(), Halt> {
        out.send(batch).map_err(|e| {
            self.control
                .stop(StopReason::Failed(format!("input could not be sent: {e}")));
            Halt::Stop
        })
    }

    /// Stops when the user pushed the pointer into a fail-safe corner.
    fn failsafe(&self, out: &Output, st: &State) -> Result<(), Halt> {
        if self.failsafe && pushed_into_corner(out.backend(), &self.monitors, st.placed) {
            self.control.stop(StopReason::Failsafe);
            return Err(Halt::Stop);
        }
        Ok(())
    }

    /// Moves the pointer to `p`, then sends `then`. The fail-safe is
    /// checked once more right before: a push into a corner after the
    /// action's first check would be undone by the move.
    fn send_at(
        &self,
        out: &mut Output,
        st: &mut State,
        p: Point,
        then: &[Synth],
    ) -> Result<(), Halt> {
        self.failsafe(out, st)?;
        st.placed = Some(p);
        let mut batch = Vec::with_capacity(then.len() + 1);
        batch.push(Synth::Move(p));
        batch.extend_from_slice(then);
        self.send(out, &batch)
    }

    /// Whether keyboard input may go on (the target program check).
    fn target_ok(&self, out: &Output, st: &mut State) -> Result<(), Halt> {
        let Some(t) = &self.macro_.target else {
            return Ok(());
        };
        let fg = out.backend().foreground();
        if fg.as_ref().is_some_and(|w| t.matches(w)) {
            return Ok(());
        }
        let now = fg.map_or_else(|| "no program".to_owned(), |w| w.process.clone());
        let message = format!("{now} is active, not {}", t.process);
        match t.on_mismatch {
            Mismatch::Stop => {
                self.control
                    .stop(StopReason::Rule(format!("Stopped: {message}")));
                Err(Halt::Stop)
            }
            Mismatch::Warn => {
                st.progress.warning = Some(format!("Playing although {message}"));
                Ok(())
            }
        }
    }

    fn play(&mut self, i: usize, st: &mut State, out: &mut Output) -> Result<(), Halt> {
        let m = self.macro_;
        let action = &m.actions[i];
        let delay = f64::from(action.delay_ms) * self.jitter() / self.settings.speed;
        st.t_ms += delay;
        if let Some(limit) = self.settings.stop_after_ms {
            let limit = limit as f64;
            if st.t_ms > limit {
                self.wait(limit, st, out)?;
                return Err(Halt::Done("Finished: time limit reached".to_owned()));
            }
        }
        self.wait(st.t_ms, st, out)?;
        self.failsafe(out, st)?;
        let place = |x: i32, y: i32| st.placement.place(Point::new(x, y), &self.monitors);
        let mut skipped = false;
        match action.kind.clone() {
            ActionKind::Move { x, y } => {
                let p = place(x, y);
                self.send_at(out, st, p, &[])?;
            }
            ActionKind::Button {
                button,
                state: Press::Down,
                x,
                y,
            } => {
                let p = place(x, y);
                if out.backend().own_window_at(p) {
                    st.skipped_buttons.insert(button);
                    skipped = true;
                } else {
                    self.send_at(out, st, p, &[Synth::Button(button, Press::Down)])?;
                }
            }
            ActionKind::Button {
                button,
                state: Press::Up,
                x,
                y,
            } => {
                if st.skipped_buttons.remove(&button) {
                    skipped = true;
                } else {
                    let p = place(x, y);
                    self.send_at(out, st, p, &[Synth::Button(button, Press::Up)])?;
                }
            }
            ActionKind::Click {
                button,
                count,
                x,
                y,
            } => {
                let p = x.zip(y).map(|(x, y)| place(x, y));
                let lands = p.or_else(|| out.backend().cursor().ok());
                if lands.is_some_and(|p| out.backend().own_window_at(p)) {
                    skipped = true;
                } else {
                    let mut clicks = Vec::new();
                    for _ in 0..count.max(1) {
                        clicks.push(Synth::Button(button, Press::Down));
                        clicks.push(Synth::Button(button, Press::Up));
                    }
                    match p {
                        Some(p) => self.send_at(out, st, p, &clicks)?,
                        None => self.send(out, &clicks)?,
                    }
                }
            }
            ActionKind::Scroll { dx, dy, x, y } => {
                let p = x.zip(y).map(|(x, y)| place(x, y));
                let lands = p.or_else(|| out.backend().cursor().ok());
                if lands.is_some_and(|p| out.backend().own_window_at(p)) {
                    skipped = true;
                } else {
                    let wheel = [Synth::Wheel { dx, dy }];
                    match p {
                        Some(p) => self.send_at(out, st, p, &wheel)?,
                        None => self.send(out, &wheel)?,
                    }
                }
            }
            ActionKind::Key {
                key,
                state: Press::Down,
            } => {
                if out.backend().own_window_active() {
                    st.skipped_keys.insert(key);
                    skipped = true;
                } else {
                    self.target_ok(out, st)?;
                    self.send(out, &[Synth::Key(key, Press::Down)])?;
                }
            }
            ActionKind::Key {
                key,
                state: Press::Up,
            } => {
                if st.skipped_keys.remove(&key) {
                    skipped = true;
                } else {
                    self.send(out, &[Synth::Key(key, Press::Up)])?;
                }
            }
            ActionKind::Keys { chord } => {
                if out.backend().own_window_active() {
                    skipped = true;
                } else {
                    self.target_ok(out, st)?;
                    let mods: Vec<Key> = [
                        (chord.ctrl, "ControlLeft"),
                        (chord.alt, "AltLeft"),
                        (chord.shift, "ShiftLeft"),
                        (chord.meta, "MetaLeft"),
                    ]
                    .into_iter()
                    .filter(|(on, _)| *on)
                    .filter_map(|(_, code)| Key::from_code(code))
                    .collect();
                    let mut batch: Vec<Synth> =
                        mods.iter().map(|&k| Synth::Key(k, Press::Down)).collect();
                    batch.push(Synth::Key(chord.key, Press::Down));
                    batch.push(Synth::Key(chord.key, Press::Up));
                    batch.extend(mods.iter().rev().map(|&k| Synth::Key(k, Press::Up)));
                    self.send(out, &batch)?;
                }
            }
            ActionKind::Wait => {}
            ActionKind::Pause => {
                st.progress.at_pause_point = true;
                st.progress.action = i;
                st.progress.label = action.describe();
                self.control.pause();
                self.pause(st, out)?;
            }
        }
        if skipped {
            st.progress.skipped += 1;
        }
        st.played += 1;
        let elapsed = self.active(st);
        st.progress.action = i;
        st.progress.label = action.describe();
        st.progress.elapsed_ms = elapsed.as_millis() as u64;
        let by_time = self
            .settings
            .stop_after_ms
            .map(|limit| (limit as f64 - elapsed.as_secs_f64() * 1000.0).max(0.0));
        let by_plan = st.planned_ms.map(|p| (p - st.t_ms).max(0.0));
        st.progress.remaining_ms = match (by_time, by_plan) {
            (Some(a), Some(b)) => Some(a.min(b) as u64),
            (a, b) => a.or(b).map(|v| v as u64),
        };
        st.progress.progress = match (st.planned_actions, self.settings.stop_after_ms) {
            (Some(total), _) if total > 0 => Some((st.played as f64 / total as f64).min(1.0)),
            (_, Some(limit)) => Some((elapsed.as_secs_f64() * 1000.0 / limit as f64).min(1.0)),
            _ => None,
        };
        (self.report)(&st.progress);
        Ok(())
    }

    /// Plays the macro. Everything pressed is released before it returns.
    pub fn run(mut self, out: &mut Output) -> Outcome {
        let m = self.macro_;
        let s = self.settings;
        let n = m.actions.len();
        let (intro, body, outro) = match s.section {
            Some(sec) if (sec.last as usize) < n => (
                0..sec.first as usize,
                sec.first as usize..sec.last as usize + 1,
                sec.last as usize + 1..n,
            ),
            _ => (0..0, 0..n, n..n),
        };
        let ms_of = |r: &std::ops::Range<usize>| -> f64 {
            m.actions[r.clone()]
                .iter()
                .map(|a| f64::from(a.delay_ms))
                .sum::<f64>()
                / s.speed
        };
        let (planned_ms, planned_actions) = match s.repeat {
            Some(reps) => (
                Some(
                    ms_of(&intro)
                        + ms_of(&body) * f64::from(reps)
                        + f64::from(s.repeat_delay_ms) * f64::from(reps.saturating_sub(1))
                        + ms_of(&outro),
                ),
                Some(intro.len() as u64 + body.len() as u64 * u64::from(reps) + outro.len() as u64),
            ),
            None => (None, None),
        };
        let mut st = State {
            progress: PlaybackProgress {
                state: RunState::Countdown,
                repetition: 1,
                repetitions: s.repeat,
                action: 0,
                actions: n,
                label: String::new(),
                progress: planned_actions.map(|_| 0.0),
                elapsed_ms: 0,
                remaining_ms: planned_ms.map(|v| v as u64),
                countdown_ms: Some(self.start_delay.as_millis() as u64),
                skipped: 0,
                warning: None,
                at_pause_point: false,
            },
            t_ms: 0.0,
            shift: Duration::ZERO,
            origin: self.clock.now(),
            placement: Placement::Same,
            placed: None,
            skipped_buttons: BTreeSet::new(),
            skipped_keys: BTreeSet::new(),
            planned_ms,
            planned_actions,
            played: 0,
        };
        if n == 0 {
            return Outcome::Completed("The macro has no actions".to_owned());
        }
        if !self.start_delay.is_zero() {
            (self.report)(&st.progress);
            let delay = self.start_delay.as_secs_f64() * 1000.0;
            if self.wait(delay, &mut st, out).is_err() {
                return self.stopped(out);
            }
        }
        st.origin = self.clock.now() - st.shift;
        st.progress.state = RunState::Running;
        st.progress.countdown_ms = None;
        st.placement = match Placement::decide(m, s, out.backend(), &self.monitors) {
            Ok(p) => p,
            Err(e) => {
                self.control.stop(StopReason::Failed(e));
                return self.stopped(out);
            }
        };
        // Checked once at the start for a macro with a target program; key
        // actions check again.
        if m.target.is_some() && self.target_ok(out, &mut st).is_err() {
            return self.stopped(out);
        }
        (self.report)(&st.progress);
        let result = (|| -> Result<(), Halt> {
            for i in intro.clone() {
                self.play(i, &mut st, out)?;
            }
            let mut rep: u32 = 1;
            loop {
                st.progress.repetition = rep;
                for i in body.clone() {
                    self.play(i, &mut st, out)?;
                }
                if s.repeat.is_some_and(|r| rep >= r) {
                    break;
                }
                rep = rep.saturating_add(1);
                // An endless macro without any delay still yields: at
                // least 1 ms between repetitions.
                let pass: u64 = m.actions[body.clone()]
                    .iter()
                    .map(|a| u64::from(a.delay_ms))
                    .sum();
                let gap = if pass == 0 && s.repeat_delay_ms == 0 {
                    1.0
                } else {
                    f64::from(s.repeat_delay_ms)
                };
                st.t_ms += gap;
            }
            for i in outro.clone() {
                self.play(i, &mut st, out)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                out.release_all();
                Outcome::Completed(match s.repeat {
                    Some(1) => "Finished".to_owned(),
                    Some(r) => format!("Finished: {r} repetitions"),
                    None => "Finished".to_owned(),
                })
            }
            Err(Halt::Done(message)) => {
                out.release_all();
                Outcome::Completed(message)
            }
            Err(Halt::Stop) => self.stopped(out),
        }
    }

    fn stopped(&self, out: &mut Output) -> Outcome {
        out.release_all();
        Outcome::Stopped(
            self.control
                .stopped()
                .unwrap_or_else(|| StopReason::User("Stopped".into())),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::automation::input::Backend;
    use crate::automation::input::mock::MockBackend;
    use crate::automation::keys::Chord;
    use crate::automation::model::{Action, RecordedOn, Section, TargetApp, WindowInfo};
    use crate::automation::timing::{FakeClock, Step};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn key(code: &str) -> Key {
        Key::from_code(code).unwrap()
    }

    struct Harness {
        clock: Arc<FakeClock>,
        backend: Arc<MockBackend>,
        control: Arc<Control>,
        reports: Arc<Mutex<Vec<PlaybackProgress>>>,
    }

    fn harness() -> Harness {
        let clock = FakeClock::new();
        let backend = MockBackend::new().with_clock(clock.clone());
        Harness {
            clock,
            backend,
            control: Control::new(),
            reports: Arc::new(Mutex::new(Vec::new())),
        }
    }

    impl Harness {
        fn play(&self, m: &Macro) -> Outcome {
            self.play_with(m, &m.playback, Duration::ZERO)
        }

        fn play_with(&self, m: &Macro, s: &PlaybackSettings, delay: Duration) -> Outcome {
            assert_eq!(m.validate(), Ok(()));
            let reports = Arc::clone(&self.reports);
            let report = move |p: &PlaybackProgress| reports.lock().unwrap().push(p.clone());
            let mut out = Output::new(self.backend.clone());
            let outcome = PlaybackRun {
                macro_: m,
                settings: s,
                start_delay: delay,
                clock: self.clock.as_ref(),
                control: &self.control,
                rng: Rng::new(9),
                failsafe: true,
                monitors: self.backend.monitors().unwrap(),
                report: &report,
            }
            .run(&mut out);
            drop(out);
            assert!(self.backend.still_held().is_empty(), "input stayed held");
            outcome
        }

        fn timed(&self) -> Vec<(u64, Synth)> {
            self.backend
                .log
                .lock()
                .unwrap()
                .iter()
                .map(|(t, s)| (t.as_millis() as u64, *s))
                .collect()
        }
    }

    fn mac(actions: Vec<Action>) -> Macro {
        let mut m = Macro::new("m-1".into(), "Test".into(), 0);
        m.actions = actions;
        m
    }

    fn click_at(delay: u32, x: i32, y: i32) -> [Action; 2] {
        [
            Action::new(
                delay,
                ActionKind::Button {
                    button: MouseButton::Left,
                    state: Press::Down,
                    x,
                    y,
                },
            ),
            Action::new(
                50,
                ActionKind::Button {
                    button: MouseButton::Left,
                    state: Press::Up,
                    x,
                    y,
                },
            ),
        ]
    }

    fn key_tap(delay: u32, code: &str) -> [Action; 2] {
        [
            Action::new(
                delay,
                ActionKind::Key {
                    key: key(code),
                    state: Press::Down,
                },
            ),
            Action::new(
                30,
                ActionKind::Key {
                    key: key(code),
                    state: Press::Up,
                },
            ),
        ]
    }

    /// Move to A, left-click, wait 500 ms, Ctrl+S, wait 1 s, move to B,
    /// right-click.
    fn example() -> Macro {
        let mut a = vec![Action::new(0, ActionKind::Move { x: 100, y: 100 })];
        a.extend(click_at(20, 100, 100));
        a.push(Action::new(
            500,
            ActionKind::Keys {
                chord: Chord::parse("Ctrl+KeyS").unwrap(),
            },
        ));
        a.push(Action::new(1000, ActionKind::Move { x: 800, y: 600 }));
        a.push(Action::new(
            20,
            ActionKind::Click {
                button: MouseButton::Right,
                count: 1,
                x: Some(800),
                y: Some(600),
            },
        ));
        mac(a)
    }

    #[test]
    fn a_macro_replays_with_its_recorded_timing() {
        let h = harness();
        assert_eq!(h.play(&example()), Outcome::Completed("Finished".into()));
        let ctrl = key("ControlLeft");
        let s = key("KeyS");
        assert_eq!(
            h.timed(),
            [
                (0, Synth::Move(Point::new(100, 100))),
                (20, Synth::Move(Point::new(100, 100))),
                (20, Synth::Button(MouseButton::Left, Press::Down)),
                (70, Synth::Move(Point::new(100, 100))),
                (70, Synth::Button(MouseButton::Left, Press::Up)),
                (570, Synth::Key(ctrl, Press::Down)),
                (570, Synth::Key(s, Press::Down)),
                (570, Synth::Key(s, Press::Up)),
                (570, Synth::Key(ctrl, Press::Up)),
                (1570, Synth::Move(Point::new(800, 600))),
                (1590, Synth::Move(Point::new(800, 600))),
                (1590, Synth::Button(MouseButton::Right, Press::Down)),
                (1590, Synth::Button(MouseButton::Right, Press::Up)),
            ]
        );
    }

    #[test]
    fn speed_scales_delays_but_not_the_start_or_the_repeat_delay() {
        let h = harness();
        let mut m = mac(key_tap(100, "KeyA").to_vec());
        let mut s = m.playback.clone();
        s.speed = 2.0;
        s.repeat = Some(3);
        s.repeat_delay_ms = 1000;
        m.playback = s.clone();
        h.play_with(&m, &s, ms(500));
        let downs: Vec<u64> = h
            .timed()
            .into_iter()
            .filter(|(_, x)| *x == Synth::Key(key("KeyA"), Press::Down))
            .map(|(t, _)| t)
            .collect();
        // Start 500; each pass is 50 + 15 ms; 1000 ms between passes.
        assert_eq!(downs, [550, 1615, 2680]);
        let h = harness();
        s.speed = 0.5;
        h.play_with(&m, &s, Duration::ZERO);
        let first_up = h
            .timed()
            .into_iter()
            .find(|(_, x)| *x == Synth::Key(key("KeyA"), Press::Up));
        assert_eq!(first_up.map(|(t, _)| t), Some(260));
    }

    #[test]
    fn repetition_counts_and_reports_its_progress() {
        let h = harness();
        let mut m = mac(key_tap(10, "KeyB").to_vec());
        m.playback.repeat = Some(4);
        assert_eq!(
            h.play(&m),
            Outcome::Completed("Finished: 4 repetitions".into())
        );
        let downs = h
            .backend
            .sent()
            .into_iter()
            .filter(|x| *x == Synth::Key(key("KeyB"), Press::Down))
            .count();
        assert_eq!(downs, 4);
        let reports = h.reports.lock().unwrap();
        let last = reports.last().unwrap();
        assert_eq!((last.repetition, last.repetitions), (4, Some(4)));
        assert_eq!(last.progress, Some(1.0));
        assert_eq!(last.remaining_ms, Some(0));
    }

    #[test]
    fn endless_playback_runs_until_stopped_and_releases_held_keys() {
        let h = harness();
        // Holds Shift across a long wait: stopping must release it.
        let mut m = mac(vec![
            Action::new(
                0,
                ActionKind::Key {
                    key: key("ShiftLeft"),
                    state: Press::Down,
                },
            ),
            Action::new(5_000, ActionKind::Wait),
            Action::new(
                0,
                ActionKind::Key {
                    key: key("ShiftLeft"),
                    state: Press::Up,
                },
            ),
        ]);
        m.playback.repeat = None;
        h.clock
            .at(ms(12_000), Step::Stop(StopReason::User("F8".into())));
        assert_eq!(h.play(&m), Outcome::Stopped(StopReason::User("F8".into())));
        let log = h.timed();
        assert_eq!(
            log.last(),
            Some(&(12_000, Synth::Key(key("ShiftLeft"), Press::Up)))
        );
    }

    #[test]
    fn an_endless_macro_without_delays_still_yields_between_repetitions() {
        let h = harness();
        let mut m = mac(vec![Action::new(0, ActionKind::Move { x: 300, y: 300 })]);
        m.playback.repeat = None;
        h.clock
            .at(ms(5), Step::Stop(StopReason::User("Stop".into())));
        h.play(&m);
        assert_eq!(h.backend.sent().len(), 5, "one repetition per millisecond");
    }

    #[test]
    fn a_pause_releases_held_input_and_presses_it_again_on_resume() {
        let h = harness();
        let m = mac(vec![
            Action::new(
                0,
                ActionKind::Button {
                    button: MouseButton::Left,
                    state: Press::Down,
                    x: 10,
                    y: 10,
                },
            ),
            Action::new(1_000, ActionKind::Move { x: 300, y: 300 }),
            Action::new(
                0,
                ActionKind::Button {
                    button: MouseButton::Left,
                    state: Press::Up,
                    x: 300,
                    y: 300,
                },
            ),
        ]);
        h.clock.at(ms(400), Step::Pause).at(ms(2_400), Step::Resume);
        h.play(&m);
        let left = |p| Synth::Button(MouseButton::Left, p);
        assert_eq!(
            h.timed(),
            [
                (0, Synth::Move(Point::new(10, 10))),
                (0, left(Press::Down)),
                (400, left(Press::Up)),
                (2_400, left(Press::Down)),
                (3_000, Synth::Move(Point::new(300, 300))),
                (3_000, Synth::Move(Point::new(300, 300))),
                (3_000, left(Press::Up)),
            ]
        );
        assert!(
            h.reports
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.state == RunState::Paused)
        );
    }

    #[test]
    fn a_pause_action_waits_for_the_user() {
        let h = harness();
        let mut a = key_tap(0, "KeyA").to_vec();
        a.push(Action::new(100, ActionKind::Pause));
        a.extend(key_tap(100, "KeyB"));
        h.clock.at(ms(10_000), Step::Resume);
        h.play(&mac(a));
        let b_down = h
            .timed()
            .into_iter()
            .find(|(_, x)| *x == Synth::Key(key("KeyB"), Press::Down))
            .unwrap();
        // Paused at 130 until 10,000; B comes 100 ms later.
        assert_eq!(b_down.0, 10_000 + 100);
        assert!(h.reports.lock().unwrap().iter().any(|r| r.at_pause_point));
    }

    #[test]
    fn only_the_section_repeats() {
        let h = harness();
        let mut a = key_tap(0, "KeyA").to_vec(); // 0, 1: intro
        a.extend(key_tap(100, "KeyB")); // 2, 3: section
        a.extend(key_tap(100, "KeyC")); // 4, 5: outro
        let mut m = mac(a);
        m.playback.section = Some(Section { first: 2, last: 3 });
        m.playback.repeat = Some(3);
        h.play(&m);
        let downs: Vec<String> = h
            .backend
            .sent()
            .into_iter()
            .filter_map(|x| match x {
                Synth::Key(k, Press::Down) => Some(k.code().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(downs, ["KeyA", "KeyB", "KeyB", "KeyB", "KeyC"]);
    }

    #[test]
    fn jitter_varies_delays_within_its_range() {
        let h = harness();
        let mut a = Vec::new();
        for _ in 0..200 {
            a.push(Action::new(100, ActionKind::Move { x: 5, y: 5 }));
        }
        let mut m = mac(a);
        m.playback.jitter_percent = 20;
        h.play(&m);
        let t: Vec<u64> = h.timed().iter().map(|(t, _)| *t).collect();
        let gaps: Vec<u64> = t.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| (79..=121).contains(g)), "{gaps:?}");
        assert!(gaps.iter().any(|&g| g < 90) && gaps.iter().any(|&g| g > 110));
    }

    #[test]
    fn positions_follow_the_pointer_in_relative_mode() {
        let h = harness();
        let mut m = example();
        m.recorded_on = Some(RecordedOn {
            cursor: Point::new(50, 50),
            window: None,
            monitors: vec![],
        });
        m.playback.positioning = Positioning::Cursor;
        *h.backend.cursor.lock().unwrap() = Point::new(250, 150);
        h.play(&m);
        assert_eq!(h.backend.sent()[0], Synth::Move(Point::new(300, 200)));
    }

    #[test]
    fn positions_follow_the_window_and_its_size() {
        let h = harness();
        let mut m = mac(vec![Action::new(
            0,
            ActionKind::Click {
                button: MouseButton::Left,
                count: 1,
                x: Some(150),
                y: Some(250),
            },
        )]);
        let window = |x, y, w, h| WindowInfo {
            title: "Editor".into(),
            process: "editor.exe".into(),
            client: Rect::new(x, y, w, h),
        };
        m.recorded_on = Some(RecordedOn {
            cursor: Point::new(0, 0),
            window: Some(window(100, 200, 400, 300)),
            monitors: vec![],
        });
        m.playback.positioning = Positioning::Window;
        m.playback.scale_to_screen = false;
        *h.backend.foreground.lock().unwrap() = Some(window(1000, 500, 800, 600));
        h.play(&m);
        assert_eq!(h.backend.sent()[0], Synth::Move(Point::new(1050, 550)));
        let h2 = harness();
        m.playback.scale_to_screen = true;
        *h2.backend.foreground.lock().unwrap() = Some(window(1000, 500, 800, 600));
        h2.play(&m);
        assert_eq!(h2.backend.sent()[0], Synth::Move(Point::new(1100, 600)));
        // Without the recorded window, window positions cannot be placed.
        let h3 = harness();
        m.recorded_on = None;
        assert!(
            matches!(h3.play(&m), Outcome::Stopped(StopReason::Failed(e)) if e.contains("no recorded window"))
        );
        assert!(h3.backend.sent().is_empty());
    }

    #[test]
    fn absolute_positions_are_mapped_to_changed_screens() {
        let h = harness();
        let mut m = mac(vec![Action::new(0, ActionKind::Move { x: 960, y: 540 })]);
        m.recorded_on = Some(RecordedOn {
            cursor: Point::new(0, 0),
            window: None,
            monitors: vec![Monitor {
                bounds: Rect::new(0, 0, 1920, 1080),
                primary: true,
            }],
        });
        *h.backend.monitors.lock().unwrap() = vec![Monitor {
            bounds: Rect::new(0, 0, 3840, 2160),
            primary: true,
        }];
        h.play(&m);
        assert_eq!(h.backend.sent(), [Synth::Move(Point::new(1920, 1080))]);
        // Without scaling, the point stays where it was.
        let h = harness();
        m.playback.scale_to_screen = false;
        *h.backend.monitors.lock().unwrap() = vec![Monitor {
            bounds: Rect::new(0, 0, 3840, 2160),
            primary: true,
        }];
        h.play(&m);
        assert_eq!(h.backend.sent(), [Synth::Move(Point::new(960, 540))]);
    }

    #[test]
    fn points_off_every_screen_are_brought_onto_one() {
        let h = harness();
        let m = mac(vec![Action::new(0, ActionKind::Move { x: 5000, y: -40 })]);
        h.play(&m);
        assert_eq!(h.backend.sent(), [Synth::Move(Point::new(1919, 0))]);
    }

    #[test]
    fn the_target_program_is_checked_before_typing() {
        let h = harness();
        let mut m = mac(key_tap(0, "KeyA").to_vec());
        m.target = Some(TargetApp {
            process: "notepad.exe".into(),
            title: None,
            on_mismatch: Mismatch::Stop,
        });
        *h.backend.foreground.lock().unwrap() = Some(WindowInfo {
            title: "Browser".into(),
            process: "browser.exe".into(),
            client: Rect::new(0, 0, 10, 10),
        });
        let outcome = h.play(&m);
        assert!(
            matches!(&outcome, Outcome::Stopped(StopReason::Rule(m)) if m.contains("browser.exe is active, not notepad.exe")),
            "{outcome:?}"
        );
        assert!(h.backend.sent().is_empty());
        // Warn: plays and says so.
        let h = harness();
        m.target.as_mut().unwrap().on_mismatch = Mismatch::Warn;
        h.play(&m);
        assert_eq!(h.backend.sent().len(), 2);
        assert!(h.reports.lock().unwrap().last().unwrap().warning.is_some());
        // The right program: no warning.
        let h = harness();
        *h.backend.foreground.lock().unwrap() = Some(WindowInfo {
            title: "Untitled".into(),
            process: "NOTEPAD.EXE".into(),
            client: Rect::new(0, 0, 10, 10),
        });
        h.play(&m);
        assert!(h.reports.lock().unwrap().last().unwrap().warning.is_none());
    }

    #[test]
    fn input_aimed_at_sotyware_is_skipped_with_its_release() {
        let h = harness();
        *h.backend.own.lock().unwrap() = Some(Rect::new(0, 0, 200, 200));
        let mut a = click_at(0, 100, 100).to_vec();
        a.extend(click_at(10, 500, 500));
        h.play(&mac(a));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 1);
        assert_eq!(h.reports.lock().unwrap().last().unwrap().skipped, 2);
        // Keys while sotyware is active.
        let h = harness();
        h.backend
            .own_active
            .store(true, std::sync::atomic::Ordering::SeqCst);
        h.play(&mac(key_tap(0, "KeyZ").to_vec()));
        assert!(h.backend.sent().is_empty());
    }

    #[test]
    fn the_time_limit_ends_playback_on_time() {
        let h = harness();
        let mut m = mac(key_tap(100, "KeyA").to_vec());
        m.playback.repeat = None;
        m.playback.stop_after_ms = Some(1_000);
        let outcome = h.play(&m);
        assert_eq!(
            outcome,
            Outcome::Completed("Finished: time limit reached".into())
        );
        let last = h.timed().last().copied().unwrap();
        assert!(last.0 <= 1_000);
    }

    #[test]
    fn the_fail_safe_corner_stops_playback() {
        let h = harness();
        let mut a = Vec::new();
        for i in 0..10 {
            a.push(Action::new(100, ActionKind::Move { x: 500 + i, y: 500 }));
        }
        // The user pushes the pointer into the top-left corner after the
        // fifth movement.
        h.backend.on_send(|b, s| {
            if *s == Synth::Move(Point::new(504, 500)) {
                *b.cursor.lock().unwrap() = Point::new(0, 1);
            }
        });
        let outcome = h.play(&mac(a));
        assert_eq!(outcome, Outcome::Stopped(StopReason::Failsafe));
        assert_eq!(
            h.backend.sent().len(),
            5,
            "stopped before the action after the push"
        );
    }

    #[test]
    fn a_push_during_the_window_check_is_not_undone_by_the_next_move() {
        // The race the Linux desktop test hit with the clicker: the user
        // pushes the pointer into a corner after the action's check, while
        // playback looks where the click would land; the move to the click
        // would put it back.
        let h = harness();
        let button = |state, x, y| ActionKind::Button {
            button: MouseButton::Left,
            state,
            x,
            y,
        };
        let mut a = Vec::new();
        for _ in 0..5 {
            a.push(Action::new(100, button(Press::Down, 800, 600)));
            a.push(Action::new(50, button(Press::Up, 800, 600)));
        }
        h.backend.on_window_check(|b, _| {
            if b.presses(MouseButton::Left).len() == 2 {
                *b.cursor.lock().unwrap() = Point::new(1919, 1079);
            }
        });
        assert_eq!(h.play(&mac(a)), Outcome::Stopped(StopReason::Failsafe));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 2);
    }

    #[test]
    fn a_recorded_position_in_a_corner_is_not_the_user_pushing_the_pointer_there() {
        let h = harness();
        let button = |state, x, y| ActionKind::Button {
            button: MouseButton::Left,
            state,
            x,
            y,
        };
        let a = vec![
            Action::new(100, ActionKind::Move { x: 0, y: 0 }),
            Action::new(100, button(Press::Down, 1919, 1079)),
            Action::new(100, button(Press::Up, 1919, 1079)),
            Action::new(100, ActionKind::Move { x: 700, y: 700 }),
        ];
        assert_eq!(h.play(&mac(a)), Outcome::Completed("Finished".into()));
        assert_eq!(
            h.backend.sent().last(),
            Some(&Synth::Move(Point::new(700, 700)))
        );
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 1);
    }

    #[test]
    fn the_start_delay_counts_down_before_positions_are_placed() {
        let h = harness();
        let mut m = example();
        m.recorded_on = Some(RecordedOn {
            cursor: Point::new(100, 100),
            window: None,
            monitors: vec![],
        });
        m.playback.positioning = Positioning::Cursor;
        // The user moves the pointer during the countdown.
        let b = h.backend.clone();
        h.clock
            .at(ms(1_000), Step::Pause)
            .at(ms(1_000), Step::Resume);
        *b.cursor.lock().unwrap() = Point::new(400, 400);
        h.play_with(&m, &m.playback.clone(), ms(3_000));
        assert_eq!(h.timed()[0], (3_000, Synth::Move(Point::new(400, 400))));
        assert_eq!(h.reports.lock().unwrap()[0].state, RunState::Countdown);
    }
}

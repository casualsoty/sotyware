//! The autoclicker engine (ADR-050): clicks by a [`ClickerConfig`] on the
//! session's own thread until a limit is reached or it is stopped.
//!
//! Clicks are scheduled on absolute deadlines from the start (the interval
//! is from one click's start to the next one's), so the rate does not
//! drift. A session that falls behind (a busy computer) does not catch up
//! in a burst: the next click is then scheduled from now. Before every
//! click the engine checks the fail-safe corners and skips a click that
//! would land on sotyware's own window, so the dashboard's Stop button can
//! always be clicked. A pause releases a button held for a click.

use std::collections::VecDeque;
use std::time::Duration;

use super::input::{Backend, Output, Synth};
use super::model::{
    ClickType, ClickerConfig, Monitor, Point, PointOrder, Press, RandomMode, TargetMode,
    in_failsafe_corner, onto_screens,
};
use super::timing::{Clock, Control, Rng, StopReason, Wake};

/// How far from a corner the fail-safe triggers.
pub const FAILSAFE_MARGIN: u32 = 2;

/// Whether the user pushed the pointer into a fail-safe corner: it is in
/// one, and not at the spot the run itself last moved it to (a position in
/// a corner is clicked like any other).
pub fn pushed_into_corner(
    backend: &dyn Backend,
    monitors: &[Monitor],
    placed: Option<Point>,
) -> bool {
    backend
        .cursor()
        .is_ok_and(|p| Some(p) != placed && in_failsafe_corner(monitors, p, FAILSAFE_MARGIN))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// Waiting for the start delay.
    Countdown,
    Running,
    Paused,
}

/// What the page shows of a running autoclicker.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickerProgress {
    pub state: RunState,
    pub clicks: u64,
    /// Time clicking, without the countdown and pauses.
    pub elapsed_ms: u64,
    /// Until the start, while counting down.
    pub countdown_ms: Option<u64>,
    /// Clicks in the last second.
    pub cps: f64,
    /// Clicks skipped because sotyware's window was under them.
    pub skipped: u64,
    pub remaining: Option<u64>,
}

/// How a session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It did what it was set to do (a limit was reached).
    Completed(String),
    Stopped(StopReason),
}

pub struct ClickerRun<'a> {
    pub config: &'a ClickerConfig,
    /// The configured start delay, or longer for a start from the
    /// dashboard.
    pub start_delay: Duration,
    pub clock: &'a dyn Clock,
    pub control: &'a Control,
    pub rng: Rng,
    pub failsafe: bool,
    pub monitors: Vec<Monitor>,
    pub report: &'a (dyn Fn(&ClickerProgress) + Sync),
}

struct Rate {
    recent: VecDeque<Duration>,
}

impl Rate {
    fn add(&mut self, at: Duration) {
        self.recent.push_back(at);
        while self
            .recent
            .front()
            .is_some_and(|&t| at.saturating_sub(t) >= Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
    }

    fn per_second(&self, now: Duration) -> f64 {
        self.recent
            .iter()
            .filter(|&&t| now.saturating_sub(t) < Duration::from_secs(1))
            .count() as f64
    }
}

impl ClickerRun<'_> {
    /// The next interval, in milliseconds.
    fn interval(&mut self) -> f64 {
        let c = self.config;
        let ms = match c.random.mode {
            RandomMode::Off => c.interval_ms,
            RandomMode::Range => self.rng.range_f64(c.random.min_ms, c.random.max_ms),
            RandomMode::Variation => {
                let v = f64::from(c.random.variation_percent) / 100.0;
                c.interval_ms * self.rng.range_f64(1.0 - v, 1.0 + v)
            }
        };
        ms.max(super::model::MIN_INTERVAL_MS)
    }

    fn target(&mut self, n: u64) -> Option<Point> {
        let t = &self.config.target;
        let base = match t.mode {
            TargetMode::Cursor => return None,
            TargetMode::Points => {
                let i = match t.order {
                    PointOrder::Sequential => (n % t.points.len() as u64) as usize,
                    PointOrder::Random => self.rng.range_i64(0, t.points.len() as i64 - 1) as usize,
                };
                t.points[i]
            }
            TargetMode::Area => {
                let a = t.area?;
                Point::new(
                    self.rng
                        .range_i64(i64::from(a.x), a.right() - 1)
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                    self.rng
                        .range_i64(i64::from(a.y), a.bottom() - 1)
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                )
            }
        };
        let s = i64::from(t.spread);
        let p = if s == 0 {
            base
        } else {
            Point::new(
                (i64::from(base.x) + self.rng.range_i64(-s, s)) as i32,
                (i64::from(base.y) + self.rng.range_i64(-s, s)) as i32,
            )
        };
        Some(onto_screens(&self.monitors, p))
    }

    /// Waits for `deadline`, handling pauses (which push `shift` back by
    /// their length and release anything held). `false`: stopped.
    fn wait(
        &self,
        deadline: Duration,
        shift: &mut Duration,
        out: &mut Output,
        progress: &mut ClickerProgress,
    ) -> bool {
        loop {
            match self.clock.wait_until(deadline + *shift, self.control) {
                Wake::Reached => return true,
                Wake::Stop => return false,
                Wake::Pause => {
                    out.release_all();
                    let before = progress.state;
                    progress.state = RunState::Paused;
                    (self.report)(progress);
                    match self.clock.wait_resumed(self.control) {
                        None => return false,
                        Some(d) => {
                            *shift += d;
                            progress.state = before;
                            (self.report)(progress);
                        }
                    }
                }
            }
        }
    }

    /// Runs the autoclicker until a limit or a stop. Everything it pressed
    /// is released before it returns (`Output` also does on drop).
    pub fn run(mut self, out: &mut Output) -> Outcome {
        let c = self.config.clone();
        let mut shift = Duration::ZERO;
        let mut progress = ClickerProgress {
            state: RunState::Countdown,
            clicks: 0,
            elapsed_ms: 0,
            countdown_ms: Some(self.start_delay.as_millis() as u64),
            cps: 0.0,
            skipped: 0,
            remaining: c.max_clicks,
        };
        let begin = self.clock.now();
        if !self.start_delay.is_zero() {
            (self.report)(&progress);
            if !self.wait(begin + self.start_delay, &mut shift, out, &mut progress) {
                return self.stopped(out);
            }
        }
        progress.state = RunState::Running;
        progress.countdown_ms = None;
        (self.report)(&progress);
        // From here, `started + shift` is the clicking clock's zero.
        let started = self.clock.now() - shift;
        // The next click's time in milliseconds of clicking: kept as a
        // float so fractional intervals (3⅓ ms) never drift.
        let mut next_ms = 0.0_f64;
        let at_ms = |ms: f64| Duration::from_secs_f64(ms.max(0.0) / 1000.0);
        let mut rate = Rate {
            recent: VecDeque::new(),
        };
        let hold = Duration::from_millis(u64::from(c.hold_ms));
        let gap = Duration::from_millis(u64::from(c.double_gap_ms));
        // Where this run last moved the pointer: a position in a corner
        // (a fixed one, or a random one in an area) is not the user pushing
        // the pointer there.
        let mut placed: Option<Point> = None;
        loop {
            let active = |s: &Self, shift: Duration| s.clock.now().saturating_sub(started + shift);
            if let Some(max) = c.max_clicks
                && progress.clicks >= max
            {
                return self.completed(out, format!("Finished: {} clicks", progress.clicks));
            }
            if let Some(max) = c.max_duration_ms
                && active(&self, shift) >= Duration::from_millis(max)
            {
                return self.completed(
                    out,
                    format!(
                        "Finished: time limit reached after {} clicks",
                        progress.clicks
                    ),
                );
            }
            let mut deadline = started + at_ms(next_ms);
            if let Some(max) = c.max_duration_ms {
                deadline = deadline.min(started + Duration::from_millis(max));
            }
            if !self.wait(deadline, &mut shift, out, &mut progress) {
                return self.stopped(out);
            }
            if let Some(max) = c.max_duration_ms
                && active(&self, shift) >= Duration::from_millis(max)
            {
                continue;
            }
            let now = active(&self, shift);
            let cursor = out.backend().cursor().ok();
            if self.failsafe
                && cursor.is_some_and(|p| {
                    Some(p) != placed && in_failsafe_corner(&self.monitors, p, FAILSAFE_MARGIN)
                })
            {
                self.control.stop(StopReason::Failsafe);
                return self.stopped(out);
            }
            let target = self.target(progress.clicks + progress.skipped);
            let backend = out.backend();
            let lands = target.or(cursor);
            let clicked = !lands.is_some_and(|p| backend.own_window_at(p));
            // Once more right before moving to the target: a push into a
            // corner during the window check would be undone by the move
            // (seen in the Linux desktop test).
            if clicked
                && target.is_some()
                && self.failsafe
                && pushed_into_corner(backend, &self.monitors, placed)
            {
                self.control.stop(StopReason::Failsafe);
                return self.stopped(out);
            }
            if !clicked {
                // Never click sotyware's own window: its Stop must work.
                progress.skipped += 1;
            } else {
                let mut first: Vec<Synth> = target.map(Synth::Move).into_iter().collect();
                let presses = if c.click_type == ClickType::Double {
                    2
                } else {
                    1
                };
                let mut ok = true;
                for i in 0..presses {
                    if i == 1 && !gap.is_zero() {
                        let at = self.clock.now().saturating_sub(started + shift) + gap;
                        if !self.wait(started + at, &mut shift, out, &mut progress) {
                            return self.stopped(out);
                        }
                    }
                    let mut batch = std::mem::take(&mut first);
                    if let Some(Synth::Move(p)) = batch.first() {
                        placed = Some(*p);
                    }
                    batch.push(Synth::Button(c.button, Press::Down));
                    if hold.is_zero() {
                        batch.push(Synth::Button(c.button, Press::Up));
                    }
                    if let Err(e) = out.send(&batch) {
                        ok = false;
                        self.control.stop(StopReason::Failed(format!(
                            "the click could not be sent: {e}"
                        )));
                        break;
                    }
                    if !hold.is_zero() {
                        let at = self.clock.now().saturating_sub(started + shift) + hold;
                        let waited = self.wait(started + at, &mut shift, out, &mut progress);
                        // Released whether the hold ended or was cut short.
                        if out.held().is_empty() {
                            // A pause during the hold already released it.
                        } else if let Err(e) = out.send(&[Synth::Button(c.button, Press::Up)]) {
                            self.control.stop(StopReason::Failed(format!(
                                "the button could not be released: {e}"
                            )));
                        }
                        if !waited {
                            return self.stopped(out);
                        }
                    }
                }
                if !ok {
                    return self.stopped(out);
                }
                progress.clicks += 1;
                rate.add(now);
            }
            progress.elapsed_ms = now.as_millis() as u64;
            progress.cps = rate.per_second(now);
            progress.remaining = c.max_clicks.map(|m| m.saturating_sub(progress.clicks));
            (self.report)(&progress);
            let mut step = self.interval();
            if let Some(b) = c.batch
                && clicked
                && progress.clicks.is_multiple_of(u64::from(b.clicks))
            {
                step += f64::from(b.pause_ms);
            }
            next_ms += step;
            // Fell more than an interval behind: no burst to catch up.
            let now_ms = active(&self, shift).as_secs_f64() * 1000.0;
            if now_ms > next_ms + step {
                next_ms = now_ms;
            }
        }
    }

    fn completed(&self, out: &mut Output, message: String) -> Outcome {
        out.release_all();
        Outcome::Completed(message)
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
    use crate::automation::model::{Batch, ClickTarget, MouseButton, RandomInterval, Rect};
    use crate::automation::timing::{FakeClock, Step};

    struct Harness {
        clock: Arc<FakeClock>,
        backend: Arc<MockBackend>,
        control: Arc<Control>,
        reports: Arc<Mutex<Vec<ClickerProgress>>>,
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

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    impl Harness {
        fn run(&self, config: &ClickerConfig) -> Outcome {
            self.run_with(config, Duration::ZERO, true)
        }

        fn run_with(&self, config: &ClickerConfig, delay: Duration, failsafe: bool) -> Outcome {
            assert_eq!(config.validate(), Ok(()));
            let reports = Arc::clone(&self.reports);
            let report = move |p: &ClickerProgress| reports.lock().unwrap().push(p.clone());
            let mut out = Output::new(self.backend.clone());
            let outcome = ClickerRun {
                config,
                start_delay: delay,
                clock: self.clock.as_ref(),
                control: &self.control,
                rng: Rng::new(42),
                failsafe,
                monitors: self.backend.monitors().unwrap(),
                report: &report,
            }
            .run(&mut out);
            drop(out);
            assert!(self.backend.still_held().is_empty(), "a button stayed held");
            outcome
        }
    }

    fn config(interval: f64, clicks: Option<u64>) -> ClickerConfig {
        ClickerConfig {
            interval_ms: interval,
            max_clicks: clicks,
            ..ClickerConfig::default()
        }
    }

    #[test]
    fn clicks_land_exactly_on_their_interval_and_stop_at_the_count() {
        let h = harness();
        let outcome = h.run(&config(100.0, Some(50)));
        assert_eq!(outcome, Outcome::Completed("Finished: 50 clicks".into()));
        let presses = h.backend.presses(MouseButton::Left);
        assert_eq!(presses.len(), 50);
        for (i, t) in presses.iter().enumerate() {
            assert_eq!(*t, ms(100 * i as u64), "click {i}");
        }
        // Every press has its release, in the same instant.
        let sent = h.backend.sent();
        assert_eq!(sent.len(), 100);
        assert!(sent.chunks(2).all(|c| c
            == [
                Synth::Button(MouseButton::Left, Press::Down),
                Synth::Button(MouseButton::Left, Press::Up)
            ]));
        let last = h.reports.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.clicks, last.remaining), (50, Some(0)));
    }

    #[test]
    fn a_fast_rate_keeps_its_count_without_drift() {
        let h = harness();
        h.run(&config(1.0, Some(10_000)));
        let presses = h.backend.presses(MouseButton::Left);
        assert_eq!(presses.len(), 10_000);
        assert_eq!(presses.last(), Some(&ms(9_999)));
        // Fractional intervals are kept on absolute deadlines: 3 clicks per
        // 10 ms never drifts.
        let h = harness();
        h.run(&config(10.0 / 3.0, Some(301)));
        assert_eq!(h.backend.presses(MouseButton::Left).last(), Some(&ms(1000)));
    }

    #[test]
    fn infinite_clicking_runs_until_stopped_and_releases_a_held_button() {
        let h = harness();
        h.clock
            .at(ms(1_000), Step::Stop(StopReason::User("F8".into())));
        let mut c = config(100.0, None);
        c.hold_ms = 40;
        let outcome = h.run(&c);
        assert_eq!(outcome, Outcome::Stopped(StopReason::User("F8".into())));
        // Clicks at 0, 100, …, 1000 (stopped while waiting for 1000).
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 10);
    }

    #[test]
    fn a_stop_during_a_hold_releases_the_button_at_once() {
        let h = harness();
        h.clock.at(ms(120), Step::Stop(StopReason::Shutdown));
        let mut c = config(100.0, None);
        c.hold_ms = 50;
        h.run(&c);
        let log = h.backend.log.lock().unwrap().clone();
        let last = log.last().unwrap();
        assert_eq!(last.1, Synth::Button(MouseButton::Left, Press::Up));
        assert_eq!(
            last.0,
            ms(120),
            "released when stopped, not at the hold's end"
        );
    }

    #[test]
    fn a_pause_shifts_the_schedule_and_is_not_counted_as_clicking_time() {
        let h = harness();
        h.clock.at(ms(250), Step::Pause).at(ms(1_250), Step::Resume);
        h.run(&config(100.0, Some(6)));
        // 0, 100, 200, (pause 250–1250 shifts by 1000), 1300, 1400, 1500.
        assert_eq!(
            h.backend.presses(MouseButton::Left),
            [0, 100, 200, 1300, 1400, 1500].map(ms)
        );
        let reports = h.reports.lock().unwrap();
        assert!(reports.iter().any(|r| r.state == RunState::Paused));
        assert_eq!(reports.last().unwrap().elapsed_ms, 500);
    }

    #[test]
    fn the_start_delay_counts_down_first() {
        let h = harness();
        h.run_with(&config(100.0, Some(2)), ms(3_000), true);
        assert_eq!(h.backend.presses(MouseButton::Left), [ms(3_000), ms(3_100)]);
        assert_eq!(h.reports.lock().unwrap()[0].state, RunState::Countdown);
    }

    #[test]
    fn random_intervals_stay_in_their_range() {
        let h = harness();
        let mut c = config(100.0, Some(500));
        c.random = RandomInterval {
            mode: RandomMode::Range,
            min_ms: 50.0,
            max_ms: 150.0,
            variation_percent: 20,
        };
        h.run(&c);
        let p = h.backend.presses(MouseButton::Left);
        let gaps: Vec<f64> = p
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        assert!(
            gaps.iter().all(|g| (49.99..=150.01).contains(g)),
            "{gaps:?}"
        );
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        assert!((90.0..110.0).contains(&mean), "mean {mean}");
        assert!(gaps.iter().any(|g| *g < 70.0) && gaps.iter().any(|g| *g > 130.0));
    }

    #[test]
    fn double_clicks_press_twice_with_their_gap_and_count_once() {
        let h = harness();
        let mut c = config(500.0, Some(3));
        c.click_type = ClickType::Double;
        c.double_gap_ms = 40;
        c.button = MouseButton::Right;
        h.run(&c);
        assert_eq!(
            h.backend.presses(MouseButton::Right),
            [0, 40, 500, 540, 1000, 1040].map(ms)
        );
    }

    #[test]
    fn positions_are_visited_in_order_and_batches_pause() {
        let h = harness();
        let mut c = config(100.0, Some(5));
        c.target = ClickTarget {
            mode: TargetMode::Points,
            points: vec![Point::new(10, 10), Point::new(20, 20)],
            order: PointOrder::Sequential,
            area: None,
            spread: 0,
        };
        c.batch = Some(Batch {
            clicks: 2,
            pause_ms: 1_000,
        });
        h.run(&c);
        let moves: Vec<Synth> = h
            .backend
            .sent()
            .into_iter()
            .filter(|s| matches!(s, Synth::Move(_)))
            .collect();
        assert_eq!(
            moves,
            [10, 20, 10, 20, 10].map(|v| Synth::Move(Point::new(v, v)))
        );
        assert_eq!(
            h.backend.presses(MouseButton::Left),
            [0, 100, 1200, 1300, 2400].map(ms)
        );
    }

    #[test]
    fn area_clicks_stay_inside_the_area_and_on_the_screen() {
        let h = harness();
        let mut c = config(10.0, Some(300));
        c.target = ClickTarget {
            mode: TargetMode::Area,
            points: vec![],
            order: PointOrder::Random,
            area: Some(Rect::new(1900, 100, 100, 50)),
            spread: 0,
        };
        h.run(&c);
        for s in h.backend.sent() {
            if let Synth::Move(p) = s {
                assert!(
                    (1900..1920).contains(&p.x) && (100..150).contains(&p.y),
                    "{p:?}"
                );
            }
        }
    }

    #[test]
    fn the_fail_safe_corner_stops_before_clicking() {
        let h = harness();
        // The user pushes the pointer into the top-left corner after the
        // third click.
        h.backend.on_send(|b, s| {
            if *s == Synth::Button(MouseButton::Left, Press::Up)
                && b.presses(MouseButton::Left).len() == 3
            {
                *b.cursor.lock().unwrap() = Point::new(1, 0);
            }
        });
        let outcome = h.run(&config(100.0, None));
        assert_eq!(outcome, Outcome::Stopped(StopReason::Failsafe));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 3);
        // Switched off, the corner does nothing.
        let h = harness();
        *h.backend.cursor.lock().unwrap() = Point::new(0, 0);
        assert!(matches!(
            h.run_with(&config(100.0, Some(4)), Duration::ZERO, false),
            Outcome::Completed(_)
        ));
    }

    #[test]
    fn a_position_in_a_corner_is_not_the_user_pushing_the_pointer_there() {
        // Found by the Linux desktop test: a fixed position left at (0, 0)
        // clicked once, then the clicker took its own move for the user's
        // push and stopped.
        let h = harness();
        let mut c = config(100.0, Some(4));
        c.target = ClickTarget {
            mode: TargetMode::Points,
            points: vec![Point::new(0, 0), Point::new(1919, 1079)],
            order: PointOrder::Sequential,
            area: None,
            spread: 0,
        };
        assert_eq!(h.run(&c), Outcome::Completed("Finished: 4 clicks".into()));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 4);
        // The user pushing the pointer into another corner still stops it,
        // and so does a push into the same corner by a pixel.
        for push in [Point::new(1919, 0), Point::new(1, 1)] {
            let h = harness();
            c.max_clicks = None;
            h.backend.on_send(move |b, s| {
                if *s == Synth::Button(MouseButton::Left, Press::Up)
                    && b.presses(MouseButton::Left).len() == 3
                {
                    *b.cursor.lock().unwrap() = push;
                }
            });
            assert_eq!(
                h.run(&c),
                Outcome::Stopped(StopReason::Failsafe),
                "{push:?}"
            );
            assert_eq!(h.backend.presses(MouseButton::Left).len(), 3);
        }
    }

    #[test]
    fn a_push_during_the_window_check_is_not_undone_by_the_next_move() {
        // Seen in the Linux desktop test: the user pushed the pointer into
        // a corner after the clicker's check, while it looked where the
        // click would land, and its move to the target put it back.
        let h = harness();
        // Bounded, so a clicker that misses the push finishes instead of
        // clicking forever.
        let mut c = config(100.0, Some(10));
        c.target = ClickTarget {
            mode: TargetMode::Points,
            points: vec![Point::new(800, 600)],
            order: PointOrder::Sequential,
            area: None,
            spread: 0,
        };
        h.backend.on_window_check(|b, _| {
            if b.presses(MouseButton::Left).len() == 3 {
                *b.cursor.lock().unwrap() = Point::new(0, 0);
            }
        });
        assert_eq!(h.run(&c), Outcome::Stopped(StopReason::Failsafe));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 3);
    }

    #[test]
    fn clicks_over_sotyware_are_skipped_so_its_stop_button_works() {
        let h = harness();
        *h.backend.own.lock().unwrap() = Some(Rect::new(400, 400, 200, 200));
        *h.backend.cursor.lock().unwrap() = Point::new(500, 500);
        h.clock
            .at(ms(1_000), Step::Stop(StopReason::User("Stop".into())));
        h.run(&config(100.0, None));
        assert!(h.backend.presses(MouseButton::Left).is_empty());
        let last = h.reports.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.clicks, last.skipped), (0, 10));
    }

    #[test]
    fn a_failed_send_ends_the_session_with_the_reason() {
        let h = harness();
        h.backend
            .fail_sends
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let outcome = h.run(&config(100.0, Some(5)));
        assert!(
            matches!(outcome, Outcome::Stopped(StopReason::Failed(m)) if m.contains("could not be sent"))
        );
    }

    #[test]
    fn a_time_limit_ends_the_session() {
        let h = harness();
        let mut c = config(100.0, None);
        c.max_duration_ms = Some(1_050);
        let outcome = h.run(&c);
        assert!(matches!(outcome, Outcome::Completed(m) if m.contains("time limit")));
        assert_eq!(h.backend.presses(MouseButton::Left).len(), 11);
    }

    #[test]
    fn the_rate_counts_clicks_of_the_last_second() {
        let h = harness();
        h.run(&config(50.0, Some(60)));
        let last = h.reports.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.cps, 20.0);
    }
}

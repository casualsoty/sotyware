//! Time for the automation engines (ADR-050): a [`Control`] that stops or
//! pauses a session from any thread, a [`Clock`] the engines wait on, and
//! a small random number generator.
//!
//! The engines never sleep by themselves: they wait for a deadline on the
//! clock, which wakes at once when the session is stopped or paused. The
//! real clock ([`RealClock`]) waits on a condition variable while the
//! deadline is far and in short precise sleeps near it, so a stop takes
//! effect within a few milliseconds and clicks land within about a
//! millisecond of their time (the operating system's timer decides the
//! rest). Deadlines are absolute, counted from the session's start, so
//! errors never add up. Tests use [`FakeClock`], whose time only moves
//! when an engine waits, and which can pause, resume or stop the session
//! at given times: every timing test is exact and instant.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Why a session ended before finishing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The dashboard's Stop or a stop shortcut.
    User(String),
    /// The pointer was pushed into a screen corner.
    Failsafe,
    /// The application quits or the feature was switched off.
    Shutdown,
    /// A rule of the session ended it (the target program is not active).
    Rule(String),
    /// Something failed; the message says what.
    Failed(String),
}

#[derive(Default)]
struct Flags {
    stop: Option<StopReason>,
    pause: bool,
}

/// Stops or pauses one session. Shared between the engine's thread and
/// whoever controls it.
#[derive(Default)]
pub struct Control {
    flags: Mutex<Flags>,
    changed: Condvar,
}

impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Flags> {
        self.flags.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks the session to stop; the first reason given wins.
    pub fn stop(&self, reason: StopReason) {
        let mut f = self.lock();
        if f.stop.is_none() {
            f.stop = Some(reason);
        }
        self.changed.notify_all();
    }

    pub fn pause(&self) {
        self.lock().pause = true;
        self.changed.notify_all();
    }

    pub fn resume(&self) {
        self.lock().pause = false;
        self.changed.notify_all();
    }

    pub fn stopped(&self) -> Option<StopReason> {
        self.lock().stop.clone()
    }

    pub fn paused(&self) -> bool {
        self.lock().pause
    }
}

/// What woke a wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// The deadline came.
    Reached,
    /// A stop was asked for.
    Stop,
    /// A pause was asked for; call [`Clock::wait_resumed`].
    Pause,
}

/// The time an engine runs on.
pub trait Clock: Send + Sync {
    /// Time since the clock's start.
    fn now(&self) -> Duration;

    /// Waits until `deadline` (from the clock's start), or less if the
    /// session is stopped or paused meanwhile.
    fn wait_until(&self, deadline: Duration, control: &Control) -> Wake;

    /// Waits while the session is paused: `Some(how long it was paused)`
    /// once it is resumed, `None` if it was stopped instead.
    fn wait_resumed(&self, control: &Control) -> Option<Duration>;
}

/// The real clock.
pub struct RealClock {
    start: Instant,
}

impl RealClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
        })
    }
}

/// Above this, a wait sleeps on the condition variable (woken by a stop or
/// pause); below it, in short sleeps, which are precise where the
/// condition variable's timeout is not (Windows rounds it to its 15.6 ms
/// timer tick, while `thread::sleep` uses a high-resolution timer).
const COARSE: Duration = Duration::from_millis(25);
const FINE_STEP: Duration = Duration::from_millis(2);

impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn wait_until(&self, deadline: Duration, control: &Control) -> Wake {
        loop {
            let flags = control.lock();
            if flags.stop.is_some() {
                return Wake::Stop;
            }
            if flags.pause {
                return Wake::Pause;
            }
            let now = self.now();
            if now >= deadline {
                return Wake::Reached;
            }
            let left = deadline - now;
            if left > COARSE {
                // Wakes early on a stop or pause; the rest is waited below.
                let _ = control
                    .changed
                    .wait_timeout(flags, left - COARSE + FINE_STEP)
                    .unwrap_or_else(|e| e.into_inner());
            } else {
                drop(flags);
                std::thread::sleep(left.min(FINE_STEP));
            }
        }
    }

    fn wait_resumed(&self, control: &Control) -> Option<Duration> {
        let since = self.now();
        let mut flags = control.lock();
        while flags.pause && flags.stop.is_none() {
            flags = control
                .changed
                .wait(flags)
                .unwrap_or_else(|e| e.into_inner());
        }
        if flags.stop.is_some() {
            None
        } else {
            Some(self.now().saturating_sub(since))
        }
    }
}

/// A clock for tests: time moves only when an engine waits, straight to
/// the deadline, unless a scripted step comes first. Steps pause, resume
/// or stop the session at a given time, so pause and stop are tested
/// exactly.
#[cfg(test)]
pub struct FakeClock {
    now: Mutex<Duration>,
    script: Mutex<std::collections::VecDeque<(Duration, Step)>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub enum Step {
    Pause,
    Resume,
    Stop(StopReason),
}

#[cfg(test)]
impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Duration::ZERO),
            script: Mutex::new(std::collections::VecDeque::new()),
        })
    }

    /// Adds a step at `at` (steps must be added in time order).
    pub fn at(&self, at: Duration, step: Step) -> &Self {
        self.script.lock().unwrap().push_back((at, step));
        self
    }

    pub fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// Applies the next step if it is due by `limit`.
    fn next_step(&self, limit: Option<Duration>, control: &Control) -> bool {
        let mut script = self.script.lock().unwrap();
        let Some((at, _)) = script.front() else {
            return false;
        };
        if limit.is_some_and(|l| *at > l) {
            return false;
        }
        let (at, step) = script.pop_front().unwrap();
        drop(script);
        let mut now = self.now.lock().unwrap();
        *now = (*now).max(at);
        drop(now);
        match step {
            Step::Pause => control.pause(),
            Step::Resume => control.resume(),
            Step::Stop(r) => control.stop(r),
        }
        true
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }

    fn wait_until(&self, deadline: Duration, control: &Control) -> Wake {
        loop {
            if control.stopped().is_some() {
                return Wake::Stop;
            }
            if control.paused() {
                return Wake::Pause;
            }
            if !self.next_step(Some(deadline), control) {
                let mut now = self.now.lock().unwrap();
                *now = (*now).max(deadline);
                return Wake::Reached;
            }
        }
    }

    fn wait_resumed(&self, control: &Control) -> Option<Duration> {
        let since = self.now();
        loop {
            if control.stopped().is_some() {
                return None;
            }
            if !control.paused() {
                return Some(self.now() - since);
            }
            assert!(
                self.next_step(None, control),
                "a test paused a session and never resumed or stopped it"
            );
        }
    }
}

/// SplitMix64: small, fast and good enough to vary intervals and
/// positions; seeded from the time in production and fixed in tests.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    #[cfg(test)]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn from_time() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Self(t.as_nanos() as u64 ^ u64::from(std::process::id()).rotate_left(32))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Evenly in `[lo, hi]`.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
    }

    /// Evenly in `[lo, hi]` (inclusive).
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % span) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_clock_reaches_its_deadline_and_wakes_at_once_on_stop() {
        let clock = RealClock::new();
        let control = Control::new();
        let deadline = clock.now() + Duration::from_millis(30);
        assert_eq!(clock.wait_until(deadline, &control), Wake::Reached);
        assert!(clock.now() >= deadline);

        let stopper = Arc::clone(&control);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            stopper.stop(StopReason::User("test".into()));
        });
        let started = Instant::now();
        let far = clock.now() + Duration::from_secs(60);
        assert_eq!(clock.wait_until(far, &control), Wake::Stop);
        assert!(started.elapsed() < Duration::from_secs(5));
        t.join().unwrap();
        assert_eq!(control.stopped(), Some(StopReason::User("test".into())));
    }

    #[test]
    fn the_real_clock_measures_a_pause() {
        let clock = RealClock::new();
        let control = Control::new();
        control.pause();
        assert_eq!(
            clock.wait_until(Duration::from_secs(60), &control),
            Wake::Pause
        );
        let resumer = Arc::clone(&control);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            resumer.resume();
        });
        let paused = clock.wait_resumed(&control).unwrap();
        assert!(paused >= Duration::from_millis(25), "{paused:?}");
        t.join().unwrap();
    }

    #[test]
    fn the_first_stop_reason_is_kept() {
        let c = Control::new();
        c.stop(StopReason::Failsafe);
        c.stop(StopReason::Shutdown);
        assert_eq!(c.stopped(), Some(StopReason::Failsafe));
    }

    #[test]
    fn the_fake_clock_runs_its_script_in_time_order() {
        let clock = FakeClock::new();
        clock
            .at(FakeClock::ms(100), Step::Pause)
            .at(FakeClock::ms(400), Step::Resume);
        let control = Control::new();
        assert_eq!(clock.wait_until(FakeClock::ms(50), &control), Wake::Reached);
        assert_eq!(clock.now(), FakeClock::ms(50));
        assert_eq!(clock.wait_until(FakeClock::ms(200), &control), Wake::Pause);
        assert_eq!(clock.now(), FakeClock::ms(100));
        assert_eq!(clock.wait_resumed(&control), Some(FakeClock::ms(300)));
        assert_eq!(
            clock.wait_until(FakeClock::ms(500), &control),
            Wake::Reached
        );
    }

    #[test]
    fn random_numbers_stay_in_range_and_repeat_with_their_seed() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let x = a.range_f64(80.0, 120.0);
            assert!((80.0..=120.0).contains(&x));
            assert_eq!(x, b.range_f64(80.0, 120.0));
            let i = a.range_i64(-3, 3);
            assert!((-3..=3).contains(&i));
            b.range_i64(-3, 3);
        }
        assert_eq!(Rng::new(1).range_i64(5, 5), 5);
    }
}

//! Which of sotyware's own features holds which global shortcut (ADR-052).
//!
//! The autoclicker (ADR-050) and the notes overlay (ADR-052) register
//! their shortcuts each through its own instance of the platform's input
//! backend, so the system alone would refuse the second of two features
//! that want one chord as if "another program" had it. Each feature
//! publishes here the chords it holds; before registering a chord, a
//! feature asks who holds it, so its refusal can name the feature; and when
//! a feature gives chords up, every other feature is asked to try its
//! refused shortcuts again, on a thread of its own (never inside the
//! caller's lock). `run()` shares one instance; tests make their own.

use std::sync::{Arc, Mutex, Weak};

use super::keys::Chord;

/// A feature of sotyware that registers global shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Autoclicker,
    NotesOverlay,
}

impl Owner {
    /// How a refusal names it: "the autoclicker uses this shortcut".
    pub fn label(self) -> &'static str {
        match self {
            Self::Autoclicker => "the autoclicker",
            Self::NotesOverlay => "the notes overlay",
        }
    }
}

/// A feature that can try its refused shortcuts again.
pub trait Retry: Send + Sync {
    fn retry_shortcuts(&self);
}

type Listener = (Owner, Weak<dyn Retry>);

/// The chords each feature holds now.
#[derive(Default)]
pub struct Claims {
    held: Mutex<Vec<(Owner, Chord)>>,
    listeners: Mutex<Vec<Listener>>,
}

impl Claims {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The feature other than `asking` that holds `chord`, if any.
    pub fn holder(&self, chord: Chord, asking: Owner) -> Option<Owner> {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(o, c)| *o != asking && *c == chord)
            .map(|(o, _)| *o)
    }

    /// Replaces what `owner` holds with `chords`. When that gives up a
    /// chord, the other features retry their refused shortcuts.
    pub fn publish(&self, owner: Owner, chords: &[Chord]) {
        let released = {
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            let released = held.iter().any(|(o, c)| *o == owner && !chords.contains(c));
            held.retain(|(o, _)| *o != owner);
            held.extend(chords.iter().map(|c| (owner, *c)));
            released
        };
        if !released {
            return;
        }
        let others: Vec<Arc<dyn Retry>> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(o, _)| *o != owner)
            .filter_map(|(_, w)| w.upgrade())
            .collect();
        for other in others {
            // Its own thread: the publisher may hold its own lock, and the
            // other feature's retry takes the other's.
            let spawned = std::thread::Builder::new()
                .name("sotyware-shortcut-retry".into())
                .spawn(move || other.retry_shortcuts());
            if let Err(e) = spawned {
                eprintln!("sotyware: cannot retry the refused shortcuts: {e}");
            }
        }
    }

    /// `who` (belonging to `owner`) retries its refused shortcuts whenever
    /// another feature gives chords up.
    pub fn on_release(&self, owner: Owner, who: Weak<dyn Retry>) {
        let mut listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        listeners.retain(|(o, w)| *o != owner && w.strong_count() > 0);
        listeners.push((owner, who));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl Retry for Counter {
        fn retry_shortcuts(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn wait_for(f: impl Fn() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    #[test]
    fn a_chord_is_held_by_the_feature_that_published_it_only() {
        let claims = Claims::new();
        claims.publish(Owner::Autoclicker, &[chord("F6"), chord("Ctrl+Shift+F1")]);
        assert_eq!(
            claims.holder(chord("Ctrl+Shift+F1"), Owner::NotesOverlay),
            Some(Owner::Autoclicker)
        );
        // A feature never conflicts with itself.
        assert_eq!(claims.holder(chord("F6"), Owner::Autoclicker), None);
        assert_eq!(claims.holder(chord("F7"), Owner::NotesOverlay), None);
        // Publishing replaces what it held.
        claims.publish(Owner::Autoclicker, &[chord("F7")]);
        assert_eq!(claims.holder(chord("F6"), Owner::NotesOverlay), None);
        assert_eq!(
            claims.holder(chord("F7"), Owner::NotesOverlay),
            Some(Owner::Autoclicker)
        );
        assert_eq!(Owner::Autoclicker.label(), "the autoclicker");
        assert_eq!(Owner::NotesOverlay.label(), "the notes overlay");
    }

    #[test]
    fn giving_a_chord_up_asks_the_other_feature_to_retry_once() {
        let claims = Claims::new();
        let overlay: Arc<Counter> = Arc::default();
        let clicker: Arc<Counter> = Arc::default();
        claims.on_release(
            Owner::NotesOverlay,
            Arc::downgrade(&overlay) as Weak<dyn Retry>,
        );
        claims.on_release(
            Owner::Autoclicker,
            Arc::downgrade(&clicker) as Weak<dyn Retry>,
        );
        claims.publish(Owner::Autoclicker, &[chord("F6")]);
        // Holding more releases nothing.
        claims.publish(Owner::Autoclicker, &[chord("F6"), chord("F7")]);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(overlay.0.load(Ordering::SeqCst), 0);
        claims.publish(Owner::Autoclicker, &[chord("F7")]);
        assert!(wait_for(|| overlay.0.load(Ordering::SeqCst) == 1));
        // Never the publisher itself.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(clicker.0.load(Ordering::SeqCst), 0);
        assert_eq!(overlay.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_listener_that_is_gone_is_skipped_and_replaced() {
        let claims = Claims::new();
        let first: Arc<Counter> = Arc::default();
        claims.on_release(
            Owner::NotesOverlay,
            Arc::downgrade(&first) as Weak<dyn Retry>,
        );
        drop(first);
        let second: Arc<Counter> = Arc::default();
        claims.on_release(
            Owner::NotesOverlay,
            Arc::downgrade(&second) as Weak<dyn Retry>,
        );
        claims.publish(Owner::Autoclicker, &[chord("F6")]);
        claims.publish(Owner::Autoclicker, &[]);
        assert!(wait_for(|| second.0.load(Ordering::SeqCst) == 1));
        assert_eq!(claims.listeners.lock().unwrap().len(), 1);
    }
}

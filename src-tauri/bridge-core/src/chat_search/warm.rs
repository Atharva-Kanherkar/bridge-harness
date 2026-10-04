//! One pre-started model session, so Enter pays for model turns, not for a
//! provider cold start.
//!
//! Measured on the Claude adapter, starting a session took 12 to 21 seconds
//! and a model turn one to three. A search that started its own session on
//! Enter could not finish inside any reasonable budget, so the index's
//! "unsure" answer to a typed query starts one in the background instead.
//!
//! The slot holds at most one session, is single-use (a search takes it, and
//! nothing carries from one search's context into the next), and empties
//! itself after [`IDLE`] unused. A search that arrives while a start is in
//! flight waits for that start rather than beginning a second one.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a started, unused session is kept.
pub const IDLE: Duration = Duration::from_secs(180);
/// How long a search waits for a start already in flight.
pub const WAIT_FOR_START: Duration = Duration::from_secs(30);

struct Slot<T> {
    /// Which model the warm session runs, so a settings change is not served
    /// a session on the old model.
    key: Option<String>,
    ready: Option<(Instant, T)>,
    starting: bool,
    /// Bumped whenever the slot is filled, so an expiry timer only clears
    /// the session it was set for.
    generation: u64,
}

pub struct Pool<T> {
    slot: Mutex<Slot<T>>,
    changed: Condvar,
}

impl<T: Send + 'static> Pool<T> {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(Slot {
                key: None,
                ready: None,
                starting: false,
                generation: 0,
            }),
            changed: Condvar::new(),
        }
    }

    /// Start a session for `key` in the background unless one is ready or
    /// starting. Returns whether a start began.
    pub fn prewarm(
        self: &Arc<Self>,
        key: &str,
        idle: Duration,
        start: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> bool {
        let replaced = {
            let mut slot = self.slot.lock().unwrap();
            if slot.starting {
                return false;
            }
            if slot.key.as_deref() == Some(key)
                && slot.ready.as_ref().is_some_and(|(expires, _)| Instant::now() < *expires)
            {
                return false;
            }
            slot.starting = true;
            slot.key = Some(key.to_owned());
            // Stopping a provider can block; never do it under the pool lock.
            slot.ready.take()
        };
        let pool = Arc::clone(self);
        std::thread::spawn(move || {
            drop(replaced);
            let started = start();
            let generation = {
                let mut slot = pool.slot.lock().unwrap();
                slot.starting = false;
                slot.generation += 1;
                if let Ok(session) = started {
                    slot.ready = Some((Instant::now() + idle, session));
                }
                pool.changed.notify_all();
                if slot.ready.is_none() {
                    return;
                }
                slot.generation
            };
            std::thread::sleep(idle);
            let expired = {
                let mut slot = pool.slot.lock().unwrap();
                if slot.generation == generation {
                    slot.ready.take()
                } else {
                    None
                }
            };
            drop(expired);
        });
        true
    }

    /// Take the warm session for `key`, waiting up to `wait` for a start in
    /// flight. `None` means the caller starts its own.
    pub fn take(&self, key: &str, wait: Duration) -> Option<T> {
        let deadline = Instant::now() + wait;
        let mut slot = self.slot.lock().unwrap();
        while slot.starting && slot.key.as_deref() == Some(key) {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            slot = self.changed.wait_timeout(slot, deadline - now).unwrap().0;
        }
        if slot.key.as_deref() != Some(key) {
            return None;
        }
        let ready = slot.ready.take();
        drop(slot);
        ready.and_then(|(expires, session)| (Instant::now() < expires).then_some(session))
    }
}

impl<T: Send + 'static> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_warm_session_is_taken_once() {
        let pool = Arc::new(Pool::new());
        assert!(pool.prewarm("haiku", IDLE, || Ok(7)));
        assert_eq!(pool.take("haiku", WAIT_FOR_START), Some(7));
        assert_eq!(pool.take("haiku", Duration::ZERO), None, "single use");
    }

    #[test]
    fn a_search_waits_for_a_start_in_flight_instead_of_starting_twice() {
        let pool = Arc::new(Pool::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&starts);
        pool.prewarm("haiku", IDLE, move || {
            counted.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(80));
            Ok("session")
        });
        let again = Arc::clone(&starts);
        assert!(!pool.prewarm("haiku", IDLE, move || {
            again.fetch_add(1, Ordering::SeqCst);
            Ok("second")
        }));
        assert_eq!(pool.take("haiku", WAIT_FOR_START), Some("session"));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn another_model_is_never_served_the_warm_session() {
        let pool = Arc::new(Pool::new());
        pool.prewarm("haiku", IDLE, || Ok(1));
        assert_eq!(pool.take("sonnet", WAIT_FOR_START), None);
        assert_eq!(pool.take("haiku", WAIT_FOR_START), Some(1));
        pool.prewarm("sonnet", IDLE, || Ok(2));
        assert_eq!(pool.take("sonnet", WAIT_FOR_START), Some(2));
    }

    #[test]
    fn take_rejects_an_expired_session_before_the_timer_runs() {
        let pool = Pool::new();
        {
            let mut slot = pool.slot.lock().unwrap();
            slot.key = Some("haiku".into());
            slot.ready = Some((Instant::now() - Duration::from_secs(1), 1));
        }
        assert_eq!(pool.take("haiku", Duration::ZERO), None);
    }

    #[test]
    fn replacing_a_session_stops_it_outside_the_pool_lock() {
        struct Session(std::sync::Weak<Pool<Session>>);
        impl Drop for Session {
            fn drop(&mut self) {
                if let Some(pool) = self.0.upgrade() {
                    assert!(pool.slot.try_lock().is_ok(), "provider shutdown must not hold the slot lock");
                }
            }
        }
        let pool = Arc::new(Pool::new());
        {
            let mut slot = pool.slot.lock().unwrap();
            slot.key = Some("haiku".into());
            slot.ready = Some((Instant::now() + IDLE, Session(Arc::downgrade(&pool))));
        }
        let weak = Arc::downgrade(&pool);
        assert!(pool.prewarm("sonnet", IDLE, move || Ok(Session(weak))));
        drop(pool.take("sonnet", WAIT_FOR_START).expect("replacement started"));
    }

    #[test]
    fn an_unused_session_expires() {
        let pool = Arc::new(Pool::new());
        pool.prewarm("haiku", Duration::from_millis(30), || Ok(1));
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(pool.take("haiku", Duration::ZERO), None);
    }

    #[test]
    fn a_failed_start_leaves_the_slot_empty() {
        let pool: Arc<Pool<u8>> = Arc::new(Pool::new());
        pool.prewarm("haiku", IDLE, || Err("not signed in".into()));
        assert_eq!(pool.take("haiku", WAIT_FOR_START), None);
        assert!(pool.prewarm("haiku", IDLE, || Ok(3)), "a failure does not block the next start");
    }
}

//! Bounded transport for provider event frames.
//!
//! The OpenCode SSE reader used to feed an unbounded `std::sync::mpsc`
//! channel; whenever normalization or the store fell behind, every serialized
//! frame in between stayed resident with no cap, no counter, and no policy.
//! This queue enforces an item capacity and a byte budget with a documented
//! overload order: transient frames (streaming deltas whose terminal frame
//! carries the full content) are evicted first and counted, durable frames
//! (lifecycle, approvals, errors, completions, usage) are never dropped —
//! once the backlog is all-durable and at capacity the producer blocks, which
//! back-pressures the provider socket instead of buffering without bound.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueBudget {
    pub max_items: usize,
    pub max_bytes: usize,
}

impl Default for QueueBudget {
    fn default() -> Self {
        Self {
            max_items: 2_048,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueMetricsSnapshot {
    pub depth: usize,
    pub bytes: usize,
    pub high_water_bytes: usize,
    pub dropped_transient: u64,
    /// Frames the producer read from the provider socket but refused to
    /// queue because they belonged to no session this runtime owns. Counted
    /// here so a filter that used to `continue` silently leaves evidence.
    pub dropped_foreign: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Disconnected;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameClass {
    Durable,
    Transient,
}

struct QueueState {
    frames: VecDeque<(FrameClass, String)>,
    bytes: usize,
    high_water_bytes: usize,
    dropped_transient: u64,
    dropped_foreign: u64,
    sender_alive: bool,
    receiver_alive: bool,
}

struct Shared {
    budget: QueueBudget,
    state: Mutex<QueueState>,
    not_empty: Condvar,
    not_full: Condvar,
}

impl Shared {
    fn over_budget(&self, state: &QueueState, incoming_bytes: usize) -> bool {
        state.frames.len() >= self.budget.max_items
            || state.bytes + incoming_bytes > self.budget.max_bytes
    }

    /// Make room by discarding the oldest transient frames. Durable frames
    /// are never touched; relative durable order is preserved.
    fn evict_transients(&self, state: &mut QueueState, incoming_bytes: usize) {
        while self.over_budget(state, incoming_bytes) {
            let Some(position) = state
                .frames
                .iter()
                .position(|(class, _)| *class == FrameClass::Transient)
            else {
                return;
            };
            let (_, frame) = state.frames.remove(position).expect("position is in range");
            state.bytes -= frame.len();
            state.dropped_transient += 1;
        }
    }

    fn push(&self, state: &mut QueueState, class: FrameClass, frame: String) {
        state.bytes += frame.len();
        state.high_water_bytes = state.high_water_bytes.max(state.bytes);
        state.frames.push_back((class, frame));
        self.not_empty.notify_one();
    }

    fn snapshot(&self) -> QueueMetricsSnapshot {
        let state = self.state.lock().expect("frame queue lock is never poisoned");
        QueueMetricsSnapshot {
            depth: state.frames.len(),
            bytes: state.bytes,
            high_water_bytes: state.high_water_bytes,
            dropped_transient: state.dropped_transient,
            dropped_foreign: state.dropped_foreign,
        }
    }
}

pub struct FrameSender {
    shared: Arc<Shared>,
}

pub struct FrameReceiver {
    shared: Arc<Shared>,
}

/// Read-only view of the queue for diagnostics.
#[derive(Clone)]
pub struct QueueMetrics {
    shared: Arc<Shared>,
}

impl QueueMetrics {
    pub fn snapshot(&self) -> QueueMetricsSnapshot {
        self.shared.snapshot()
    }
}

pub fn bounded_frame_queue(budget: QueueBudget) -> (FrameSender, FrameReceiver, QueueMetrics) {
    let shared = Arc::new(Shared {
        budget,
        state: Mutex::new(QueueState {
            frames: VecDeque::new(),
            bytes: 0,
            high_water_bytes: 0,
            dropped_transient: 0,
            dropped_foreign: 0,
            sender_alive: true,
            receiver_alive: true,
        }),
        not_empty: Condvar::new(),
        not_full: Condvar::new(),
    });
    (
        FrameSender {
            shared: shared.clone(),
        },
        FrameReceiver {
            shared: shared.clone(),
        },
        QueueMetrics { shared },
    )
}

impl FrameSender {
    /// Enqueue a frame that must not be lost. Evicts transient backlog first;
    /// when the backlog is all-durable and at capacity this blocks until the
    /// consumer drains or disconnects. A frame larger than the whole budget
    /// is admitted alone rather than deadlocking.
    pub fn send_durable(&self, frame: String) -> Result<(), Disconnected> {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        loop {
            if !state.receiver_alive {
                return Err(Disconnected);
            }
            self.shared.evict_transients(&mut state, frame.len());
            if !self.shared.over_budget(&state, frame.len()) || state.frames.is_empty() {
                self.shared.push(&mut state, FrameClass::Durable, frame);
                return Ok(());
            }
            state = self
                .shared
                .not_full
                .wait(state)
                .expect("frame queue lock is never poisoned");
        }
    }

    /// Enqueue a frame the pipeline may shed under pressure. Returns whether
    /// the frame was queued; a `false` is counted, not an error, because the
    /// terminal frame for the same item carries the complete content.
    pub fn send_transient(&self, frame: String) -> Result<bool, Disconnected> {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        if !state.receiver_alive {
            return Err(Disconnected);
        }
        if self.shared.over_budget(&state, frame.len()) {
            state.dropped_transient += 1;
            return Ok(false);
        }
        self.shared.push(&mut state, FrameClass::Transient, frame);
        Ok(true)
    }
}

impl FrameSender {
    /// Record a frame the producer declined to queue because it was not this
    /// runtime's to forward. Nothing is enqueued; only the count moves.
    pub fn record_foreign_drop(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        state.dropped_foreign += 1;
    }
}

impl Drop for FrameSender {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        state.sender_alive = false;
        self.shared.not_empty.notify_all();
    }
}

impl FrameReceiver {
    /// Next frame in order, blocking on an empty queue. Errors only once the
    /// sender is gone and the backlog is fully drained.
    pub fn recv(&self) -> Result<String, Disconnected> {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        loop {
            if let Some((_, frame)) = state.frames.pop_front() {
                state.bytes -= frame.len();
                self.shared.not_full.notify_one();
                return Ok(frame);
            }
            if !state.sender_alive {
                return Err(Disconnected);
            }
            state = self
                .shared
                .not_empty
                .wait(state)
                .expect("frame queue lock is never poisoned");
        }
    }
}

impl Drop for FrameReceiver {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("frame queue lock is never poisoned");
        state.receiver_alive = false;
        self.shared.not_full.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn small_budget() -> QueueBudget {
        QueueBudget {
            max_items: 8,
            max_bytes: 256,
        }
    }

    #[test]
    fn transient_flood_stays_within_budget_and_counts_drops() {
        let (sender, _receiver, metrics) = bounded_frame_queue(small_budget());
        for index in 0..1_000 {
            let queued = sender
                .send_transient(format!("transient frame {index:0>32}"))
                .expect("receiver is alive");
            let snapshot = metrics.snapshot();
            assert!(snapshot.bytes <= 256, "byte budget held: {}", snapshot.bytes);
            assert!(snapshot.depth <= 8, "item budget held: {}", snapshot.depth);
            if !queued {
                break;
            }
        }
        for index in 0..1_000 {
            let _ = sender.send_transient(format!("transient frame {index:0>32}"));
        }
        let snapshot = metrics.snapshot();
        assert!(snapshot.bytes <= 256);
        assert!(snapshot.depth <= 8);
        assert!(snapshot.dropped_transient > 0, "drops are counted");
        assert!(snapshot.high_water_bytes <= 256);
        assert!(snapshot.high_water_bytes > 0);
    }

    #[test]
    fn foreign_drops_are_counted() {
        let (sender, receiver, metrics) = bounded_frame_queue(small_budget());
        sender.record_foreign_drop();
        sender.record_foreign_drop();
        sender.send_durable("kept".into()).expect("receiver is alive");
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.dropped_foreign, 2);
        assert_eq!(snapshot.dropped_transient, 0, "a foreign drop is not a shed delta");
        assert_eq!(snapshot.depth, 1, "nothing foreign was enqueued");
        assert_eq!(receiver.recv().unwrap(), "kept");
    }

    #[test]
    fn durable_frames_evict_transients_and_survive_in_order() {
        let (sender, receiver, metrics) = bounded_frame_queue(small_budget());
        while sender
            .send_transient("x".repeat(30))
            .expect("receiver is alive")
        {}
        for index in 0..6 {
            sender
                .send_durable(format!("durable {index}"))
                .expect("receiver is alive");
        }
        let snapshot = metrics.snapshot();
        assert!(snapshot.bytes <= 256);
        let mut durable_seen = Vec::new();
        for _ in 0..snapshot.depth {
            let frame = receiver.recv().expect("frames remain");
            if frame.starts_with("durable") {
                durable_seen.push(frame);
            }
        }
        assert_eq!(
            durable_seen,
            (0..6).map(|i| format!("durable {i}")).collect::<Vec<_>>(),
            "every durable frame arrives, in order"
        );
    }

    #[test]
    fn durable_producer_blocks_at_capacity_and_unblocks_on_drain() {
        let (sender, receiver, _metrics) = bounded_frame_queue(QueueBudget {
            max_items: 2,
            max_bytes: 1024,
        });
        sender.send_durable("one".into()).expect("space");
        sender.send_durable("two".into()).expect("space");
        let producer = std::thread::spawn(move || {
            sender.send_durable("three".into()).expect("unblocked by drain");
            std::time::Instant::now()
        });
        let before_drain = std::time::Instant::now();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(receiver.recv().expect("first frame"), "one");
        let unblocked_at = producer.join().expect("producer finishes");
        assert!(
            unblocked_at.duration_since(before_drain) >= Duration::from_millis(100),
            "the producer waited for the drain instead of buffering"
        );
        assert_eq!(receiver.recv().expect("second frame"), "two");
        assert_eq!(receiver.recv().expect("third frame"), "three");
    }

    #[test]
    fn oversized_durable_frame_is_admitted_alone() {
        let (sender, receiver, _metrics) = bounded_frame_queue(small_budget());
        sender
            .send_durable("y".repeat(4_096))
            .expect("oversized durable admitted rather than deadlocking");
        assert_eq!(receiver.recv().expect("frame arrives").len(), 4_096);
    }

    #[test]
    fn receiver_drain_survives_sender_drop_then_disconnects() {
        let (sender, receiver, _metrics) = bounded_frame_queue(small_budget());
        sender.send_durable("last words".into()).expect("space");
        drop(sender);
        assert_eq!(receiver.recv().expect("backlog drains"), "last words");
        assert_eq!(receiver.recv(), Err(Disconnected));
    }

    #[test]
    fn sends_fail_once_receiver_is_gone() {
        let (sender, receiver, _metrics) = bounded_frame_queue(small_budget());
        drop(receiver);
        assert_eq!(sender.send_durable("late".into()), Err(Disconnected));
        assert_eq!(sender.send_transient("late".into()), Err(Disconnected));
    }

    #[test]
    fn blocked_durable_producer_errors_when_receiver_drops() {
        let (sender, receiver, _metrics) = bounded_frame_queue(QueueBudget {
            max_items: 1,
            max_bytes: 1024,
        });
        sender.send_durable("plug".into()).expect("space");
        let producer =
            std::thread::spawn(move || sender.send_durable("stuck".into()));
        std::thread::sleep(Duration::from_millis(100));
        drop(receiver);
        assert_eq!(
            producer.join().expect("producer finishes"),
            Err(Disconnected)
        );
    }
}

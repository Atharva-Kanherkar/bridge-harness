//! Coalescing the live agent stream on its way to the webview.
//!
//! A single tool call is roughly four frames — started, an output delta or
//! two, completed — and a hundred-step turn is four hundred of them, each one
//! its own IPC message, each one serialized, posted and dispatched
//! individually. The frontend already re-batches on arrival (a 50 ms timer in
//! `App.tsx`), so the per-frame delivery buys nothing and costs the webview a
//! wake-up per frame at the exact moment it is trying to draw.
//!
//! This sits at the shell's emit boundary and nowhere else. Nothing about
//! durability changes: the frames were persisted before the event bus ever
//! published them, and this is downstream of both. It is not a bypass of the
//! bus either — every event still arrives from the bus, and the only thing
//! that changes is how many messages carry them across the process boundary.
//!
//! Two rules make it safe to put here:
//!
//! 1. **Order is preserved.** Any other event flushes the pending agent frames
//!    ahead of itself, so a `state-changed` never overtakes the frames that
//!    caused it.
//! 2. **Nothing is held long.** The batch flushes at the window (a frame's
//!    worth of time), at a size cap, or the moment anything else is emitted.

use serde_json::Value;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The webview event carrying a batch of agent frames.
///
/// Shell-to-webview transport, deliberately *not* a protocol notification:
/// `bridged` still speaks `agent-event`, one frame per notification, and the
/// daemon contract is unchanged. `bridgeApi.onAgentEvent` unpacks this and
/// calls its handler once per frame, so nothing above `src/api.ts` can tell.
pub const AGENT_EVENT_BATCH: &str = "agent-event-batch";

/// About one display frame. Long enough to collect a burst, short enough that
/// a lone frame still reads as immediate.
pub const FLUSH_WINDOW: Duration = Duration::from_millis(16);

/// A cap so a burst cannot grow one message without bound.
pub const MAX_BATCH: usize = 64;

fn agent_event_name() -> &'static str {
    bridge_protocol::notifications::NotificationName::AgentEvent.as_str()
}

/// One message to send: an event name and its payload.
pub type Emission = (String, Value);

/// What the shell does with a message once the batcher has decided on it.
type Sink = Box<dyn Fn(&str, Value) + Send + Sync>;

/// The decision half, with no threads in it.
#[derive(Default)]
struct Pending {
    events: Vec<Value>,
    /// When the held frames must go out. `None` means nothing is held.
    deadline: Option<Instant>,
    stopped: bool,
}

impl Pending {
    /// What to send, in order, given one incoming event.
    fn offer(
        &mut self,
        kind: &str,
        payload: Value,
        now: Instant,
        window: Duration,
        cap: usize,
    ) -> Vec<Emission> {
        if kind != agent_event_name() {
            // Anything else goes out behind whatever is already held, never
            // ahead of it.
            let mut out = Vec::new();
            out.extend(self.take());
            out.push((kind.to_string(), payload));
            return out;
        }
        self.events.push(payload);
        if self.events.len() >= cap {
            return self.take().into_iter().collect();
        }
        match self.deadline {
            // Late — the flush thread has not got to it yet, so send now.
            Some(deadline) if now >= deadline => self.take().into_iter().collect(),
            Some(_) => Vec::new(),
            None => {
                self.deadline = Some(now + window);
                Vec::new()
            }
        }
    }

    /// The batch, if its window has closed.
    fn due(&mut self, now: Instant) -> Option<Emission> {
        match self.deadline {
            Some(deadline) if now >= deadline => self.take(),
            _ => None,
        }
    }

    /// The batch, whatever the clock says.
    fn take(&mut self) -> Option<Emission> {
        self.deadline = None;
        if self.events.is_empty() {
            return None;
        }
        Some((
            AGENT_EVENT_BATCH.to_string(),
            Value::Array(std::mem::take(&mut self.events)),
        ))
    }
}

/// The batcher, with the thread that closes its windows.
pub struct AgentEventBatcher {
    pending: Mutex<Pending>,
    woken: Condvar,
    window: Duration,
    cap: usize,
    emit: Sink,
}

impl AgentEventBatcher {
    /// Start one, with a thread that flushes what the window leaves behind.
    ///
    /// The thread sleeps on the condition variable while nothing is held, so
    /// an idle app pays nothing for this.
    pub fn spawn<E>(window: Duration, cap: usize, emit: E) -> Arc<Self>
    where
        E: Fn(&str, Value) + Send + Sync + 'static,
    {
        let batcher = Arc::new(Self {
            pending: Mutex::new(Pending::default()),
            woken: Condvar::new(),
            window,
            cap,
            emit: Box::new(emit),
        });
        let worker = batcher.clone();
        // A failed spawn is not fatal: without the closing thread every batch
        // still goes out, just on the next frame or the size cap instead of on
        // the window. Better a coarser transcript than no app.
        let _ = std::thread::Builder::new()
            .name("agent-event-batcher".into())
            .spawn(move || worker.run());
        batcher
    }

    /// Offer one event. The emit closure is called under the same lock the
    /// decision was made under, so two threads can never reorder what they
    /// send.
    pub fn emit(&self, kind: &str, payload: Value) {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        let emissions = pending.offer(kind, payload, Instant::now(), self.window, self.cap);
        for (name, value) in &emissions {
            (self.emit)(name, value.clone());
        }
        let armed = pending.deadline.is_some();
        drop(pending);
        if armed {
            self.woken.notify_all();
        }
    }

    /// Send whatever is held and stop the thread. For tests and shutdown.
    pub fn stop(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((name, value)) = pending.take() {
            (self.emit)(&name, value);
        }
        pending.stopped = true;
        drop(pending);
        self.woken.notify_all();
    }

    fn run(self: Arc<Self>) {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if pending.stopped {
                return;
            }
            let Some(deadline) = pending.deadline else {
                pending = self
                    .woken
                    .wait(pending)
                    .unwrap_or_else(|error| error.into_inner());
                continue;
            };
            let now = Instant::now();
            if now < deadline {
                let (guard, _) = self
                    .woken
                    .wait_timeout(pending, deadline - now)
                    .unwrap_or_else(|error| error.into_inner());
                pending = guard;
                continue;
            }
            if let Some((name, value)) = pending.due(now) {
                (self.emit)(&name, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const WINDOW: Duration = Duration::from_millis(16);

    fn frame(id: i64) -> Value {
        serde_json::json!({ "id": id })
    }

    fn ids(emission: &Emission) -> Vec<i64> {
        emission
            .1
            .as_array()
            .expect("a batch is an array")
            .iter()
            .map(|value| value["id"].as_i64().expect("an id"))
            .collect()
    }

    #[test]
    fn holds_frames_inside_the_window() {
        let mut pending = Pending::default();
        let now = Instant::now();
        assert!(pending
            .offer(agent_event_name(), frame(1), now, WINDOW, 64)
            .is_empty());
        assert!(pending
            .offer(agent_event_name(), frame(2), now, WINDOW, 64)
            .is_empty());
        let flushed = pending.due(now + WINDOW).expect("the window closed");
        assert_eq!(flushed.0, AGENT_EVENT_BATCH);
        assert_eq!(ids(&flushed), vec![1, 2]);
    }

    #[test]
    fn sends_nothing_before_the_window_closes() {
        let mut pending = Pending::default();
        let now = Instant::now();
        pending.offer(agent_event_name(), frame(1), now, WINDOW, 64);
        assert!(pending.due(now + Duration::from_millis(1)).is_none());
    }

    #[test]
    fn flushes_early_at_the_size_cap() {
        let mut pending = Pending::default();
        let now = Instant::now();
        pending.offer(agent_event_name(), frame(1), now, WINDOW, 2);
        let flushed = pending.offer(agent_event_name(), frame(2), now, WINDOW, 2);
        assert_eq!(flushed.len(), 1);
        assert_eq!(ids(&flushed[0]), vec![1, 2]);
        // And it starts over rather than re-sending what it just sent.
        assert!(pending.due(now + WINDOW).is_none());
    }

    #[test]
    fn puts_another_event_behind_the_frames_it_follows() {
        let mut pending = Pending::default();
        let now = Instant::now();
        pending.offer(agent_event_name(), frame(1), now, WINDOW, 64);
        let sent = pending.offer("state-changed", Value::Null, now, WINDOW, 64);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, AGENT_EVENT_BATCH);
        assert_eq!(ids(&sent[0]), vec![1]);
        assert_eq!(sent[1].0, "state-changed");
    }

    #[test]
    fn passes_an_unrelated_event_straight_through() {
        let mut pending = Pending::default();
        let sent = pending.offer("state-changed", Value::Null, Instant::now(), WINDOW, 64);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "state-changed");
    }

    #[test]
    fn sends_late_frames_rather_than_holding_them_for_another_window() {
        let mut pending = Pending::default();
        let now = Instant::now();
        pending.offer(agent_event_name(), frame(1), now, WINDOW, 64);
        // The flush thread never woke; the next frame carries the batch out.
        let sent = pending.offer(agent_event_name(), frame(2), now + WINDOW, WINDOW, 64);
        assert_eq!(sent.len(), 1);
        assert_eq!(ids(&sent[0]), vec![1, 2]);
    }

    #[test]
    fn the_thread_closes_a_window_nothing_else_would() {
        let (sender, receiver) = mpsc::channel();
        let batcher = AgentEventBatcher::spawn(Duration::from_millis(5), 64, move |kind, value| {
            let _ = sender.send((kind.to_string(), value));
        });
        batcher.emit(agent_event_name(), frame(1));
        batcher.emit(agent_event_name(), frame(2));
        let (kind, value) = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the window closed on its own");
        assert_eq!(kind, AGENT_EVENT_BATCH);
        assert_eq!(ids(&(kind, value)), vec![1, 2]);
        batcher.stop();
    }

    #[test]
    fn stopping_sends_what_is_still_held() {
        let (sender, receiver) = mpsc::channel();
        // A window long enough that only `stop` can end it.
        let batcher = AgentEventBatcher::spawn(Duration::from_secs(30), 64, move |kind, value| {
            let _ = sender.send((kind.to_string(), value));
        });
        batcher.emit(agent_event_name(), frame(7));
        batcher.stop();
        let held = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("stop flushed");
        assert_eq!(ids(&held), vec![7]);
    }
}

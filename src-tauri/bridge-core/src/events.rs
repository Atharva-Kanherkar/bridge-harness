//! The event-publisher seam: typed core events on a bounded, lossy broadcast
//! channel.
//!
//! Rules (they are the contract, see `docs/protocol/README.md`):
//! - The session forest / SQLite is the **authoritative history**. The
//!   broadcast channel is a low-latency notification layer only — bounded,
//!   and it drops the oldest events when a receiver lags, so it must never be
//!   treated as a source of truth.
//! - **Publish only after the corresponding DB transaction commits.** A
//!   rolled-back mutation must emit nothing.
//! - Durable events ([`CoreEvent::Agent`]) carry their session-forest
//!   sequence; after a disconnect or `Lagged`, a client replays from its last
//!   cursor via the durable store (no gaps, no duplicates). Transient events
//!   are never replayed.
//!
//! Hosts subscribe and forward: the Tauri shell forwards each event to
//! `app.emit` with unchanged names and payloads, so the frontend needs no
//! changes while the migration is in flight.

use crate::model::AgentEvent;
use bridge_protocol::notifications::NotificationName;
use serde_json::Value;
pub use tokio::sync::broadcast;

/// Bounded capacity of the live channel. Deliberately generous — lagging is
/// legal (receivers replay durable events from the store) but should be rare.
const EVENT_BUS_CAPACITY: usize = 1024;

/// A typed core event. Wire kind and payload match today's Tauri emits
/// exactly; the exhaustive [`CoreEvent::kind`] mapping keeps this enum and
/// the protocol notification registry from drifting.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Refetch hint: application state changed; clients re-read the snapshot.
    StateChanged,
    /// Refetch hint: adapter availability changed.
    AdaptersChanged,
    /// Durable conversation-history event, backed by a session-forest entry.
    Agent(AgentEvent),
    /// Refetch hint carrying the changed learning run/state payload.
    LearningJobChanged(Value),
    /// Transient terminal bytes; worthless once stale, never replayed.
    SessionOutput { session_id: String, data: String },
    /// Transient provider usage tick for the ambient meter.
    AccountUsage { provider: String, rate_limits: Value },
}

impl CoreEvent {
    /// The protocol notification this event is delivered as. Exhaustive on
    /// purpose: a new event variant fails to compile until the protocol
    /// registry names it and assigns a delivery class.
    pub fn kind(&self) -> NotificationName {
        match self {
            CoreEvent::StateChanged => NotificationName::StateChanged,
            CoreEvent::AdaptersChanged => NotificationName::AdaptersChanged,
            CoreEvent::Agent(_) => NotificationName::AgentEvent,
            CoreEvent::LearningJobChanged(_) => NotificationName::LearningJobChanged,
            CoreEvent::SessionOutput { .. } => NotificationName::SessionOutput,
            CoreEvent::AccountUsage { .. } => NotificationName::AccountUsage,
        }
    }

    /// The wire payload, exactly as today's `app.emit` serializes it.
    pub fn payload(&self) -> Value {
        match self {
            CoreEvent::StateChanged | CoreEvent::AdaptersChanged => Value::Null,
            CoreEvent::Agent(event) => serde_json::to_value(event).expect("agent event serializes"),
            CoreEvent::LearningJobChanged(payload) => payload.clone(),
            CoreEvent::SessionOutput { session_id, data } => serde_json::json!({
                "sessionId": session_id,
                "data": data,
            }),
            CoreEvent::AccountUsage { provider, rate_limits } => serde_json::json!({
                "provider": provider,
                "rateLimits": rate_limits,
            }),
        }
    }

    /// The durable replay cursor: the session-forest sequence for durable
    /// events, `None` for transient ones.
    pub fn durable_cursor(&self) -> Option<i64> {
        match self {
            CoreEvent::Agent(event) => Some(event.sequence),
            _ => None,
        }
    }
}

/// The notify-only live channel. Cloning yields another handle to the same
/// channel, so boot-time publishers (adapter discovery) can capture one.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<CoreEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.sender.subscribe()
    }

    /// Publish an event. Call sites must sit **after** the corresponding DB
    /// commit. Delivery is best-effort by design: no subscribers (or lagged
    /// subscribers) are not errors, because durable history lives in SQLite.
    pub fn publish(&self, event: CoreEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::notifications::DeliveryClass;

    fn agent_event(sequence: i64) -> AgentEvent {
        AgentEvent {
            id: sequence,
            session_id: "s".into(),
            sequence,
            protocol_version: 1,
            kind: "message.completed".into(),
            item_id: None,
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some(format!("event {sequence}")),
            data: serde_json::json!({}),
            provider_meta: serde_json::json!({}),
            created_at: "now".into(),
        }
    }

    #[test]
    fn every_event_kind_maps_into_the_protocol_registry() {
        let events = [
            CoreEvent::StateChanged,
            CoreEvent::AdaptersChanged,
            CoreEvent::Agent(agent_event(1)),
            CoreEvent::LearningJobChanged(serde_json::json!({"id":"run"})),
            CoreEvent::SessionOutput { session_id: "s".into(), data: "$ ls".into() },
            CoreEvent::AccountUsage { provider: "codex".into(), rate_limits: serde_json::json!({}) },
        ];
        for event in &events {
            // Durable events — and only durable events — expose a cursor.
            assert_eq!(
                event.durable_cursor().is_some(),
                event.kind().delivery() == DeliveryClass::Durable,
                "{}",
                event.kind().as_str()
            );
        }
        let kinds: std::collections::HashSet<_> =
            events.iter().map(|event| event.kind()).collect();
        assert_eq!(kinds.len(), NotificationName::ALL.len(), "every notification kind is covered");
    }

    #[test]
    fn payloads_match_the_legacy_emit_shapes() {
        assert_eq!(CoreEvent::StateChanged.payload(), Value::Null);
        let output = CoreEvent::SessionOutput { session_id: "s".into(), data: "hi".into() };
        assert_eq!(output.payload(), serde_json::json!({"sessionId":"s","data":"hi"}));
        let usage = CoreEvent::AccountUsage {
            provider: "claude".into(),
            rate_limits: serde_json::json!({"remaining": 10}),
        };
        assert_eq!(
            usage.payload(),
            serde_json::json!({"provider":"claude","rateLimits":{"remaining":10}})
        );
        let agent = CoreEvent::Agent(agent_event(7));
        assert_eq!(agent.payload()["sequence"], serde_json::json!(7));
    }

    #[test]
    fn lagged_receivers_recover_by_replaying_durable_history_from_the_cursor() {
        // The kill-and-reconnect contract in miniature: the live channel may
        // drop events under lag, but durable history replays them from the
        // last seen cursor with no gaps and no duplicates.
        let (sender, mut receiver) = broadcast::channel::<CoreEvent>(2);
        let durable_store: Vec<AgentEvent> = (1..=6).map(agent_event).collect();

        // The subscriber sees the first event, then stalls while five more
        // land in a channel with room for two.
        sender.send(CoreEvent::Agent(durable_store[0].clone())).unwrap();
        let first = receiver.try_recv().unwrap();
        let mut last_cursor = first.durable_cursor().unwrap();
        assert_eq!(last_cursor, 1);
        for event in &durable_store[1..] {
            sender.send(CoreEvent::Agent(event.clone())).unwrap();
        }

        let mut delivered = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(event) => delivered.push(event.durable_cursor().unwrap()),
                Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                    assert!(missed > 0, "the bounded channel dropped events");
                    // Recover from the authoritative store, not the channel.
                    let replayed: Vec<i64> = durable_store
                        .iter()
                        .filter(|event| event.sequence > last_cursor)
                        .map(|event| event.sequence)
                        .collect();
                    assert_eq!(replayed, vec![2, 3, 4, 5, 6], "no gaps, no duplicates");
                    last_cursor = *replayed.last().unwrap();
                    delivered.clear();
                    // Live delivery resumes; anything still buffered is a
                    // duplicate of the replay and is skipped by cursor.
                    while let Ok(event) = receiver.try_recv() {
                        let cursor = event.durable_cursor().unwrap();
                        assert!(cursor <= last_cursor, "buffered events never exceed the replayed cursor");
                    }
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Closed) => unreachable!(),
            }
        }
        assert_eq!(last_cursor, 6);
    }
}

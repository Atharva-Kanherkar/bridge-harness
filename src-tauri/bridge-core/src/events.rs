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
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

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
    /// Conversation event. Positive sequences are durable session-forest
    /// entries; sequence zero is a transient streaming frame.
    Agent(AgentEvent),
    /// Refetch hint carrying the changed learning run/state payload.
    LearningJobChanged(Value),
    /// Refetch hint: explicit memory pins changed in this scope. The payload
    /// names the scope only, because a client that rebuilt a record from a
    /// notification would keep showing a pin the tombstone already removed.
    MemoryChanged { scope_key: String },
    /// Transient terminal bytes; worthless once stale, never replayed. The
    /// terminal id addresses one shell of the workspace's several.
    SessionOutput { session_id: String, terminal_id: String, data: String },
    /// One shell ended — exit, kill, or explicit close. Clients re-list the
    /// workspace's terminals rather than trusting a rebuilt roster.
    TerminalExited { session_id: String, terminal_id: String },
    /// Transient provider usage tick for the ambient meter.
    AccountUsage {
        provider: String,
        rate_limits: Value,
    },
    /// Refetch hint: a managed agent's installation or readiness changed. The
    /// payload carries the agent id only — authoritative state is refetched
    /// through `agents/list_managed_agents`, never reconstructed from this.
    ManagedAgentChanged { agent_id: String },
    /// A cold-start phase observed at a real adapter launch boundary.
    /// Transient and best-effort: worthless once stale, and a client that
    /// missed one simply never shows that phase rather than catching up —
    /// unlike every other variant here, there is no "last known state" to
    /// resend after a lag.
    SessionStartup {
        session_id: String,
        phase: crate::adapters::StartupPhase,
    },
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
            CoreEvent::MemoryChanged { .. } => NotificationName::MemoryChanged,
            CoreEvent::SessionOutput { .. } => NotificationName::SessionOutput,
            CoreEvent::TerminalExited { .. } => NotificationName::TerminalExited,
            CoreEvent::AccountUsage { .. } => NotificationName::AccountUsage,
            CoreEvent::ManagedAgentChanged { .. } => NotificationName::ManagedAgentChanged,
            CoreEvent::SessionStartup { .. } => NotificationName::SessionStartup,
        }
    }

    /// The wire payload, exactly as today's `app.emit` serializes it.
    pub fn payload(&self) -> Value {
        match self {
            CoreEvent::StateChanged | CoreEvent::AdaptersChanged => Value::Null,
            CoreEvent::Agent(event) => serde_json::to_value(event).expect("agent event serializes"),
            CoreEvent::LearningJobChanged(payload) => payload.clone(),
            // The scope only. Rebuilding a record from the hint would leave a
            // pin on screen that its tombstone already removed.
            CoreEvent::MemoryChanged { scope_key } => serde_json::json!({
                "scopeKey": scope_key,
            }),
            CoreEvent::SessionOutput { session_id, terminal_id, data } => serde_json::json!({
                "sessionId": session_id,
                "terminalId": terminal_id,
                "data": data,
            }),
            CoreEvent::TerminalExited { session_id, terminal_id } => serde_json::json!({
                "sessionId": session_id,
                "terminalId": terminal_id,
            }),
            // The agent id only: the client refetches authoritative state rather
            // than rebuilding it from a notification.
            CoreEvent::ManagedAgentChanged { agent_id } => serde_json::json!({
                "agentId": agent_id,
            }),
            CoreEvent::SessionStartup { session_id, phase } => serde_json::json!({
                "sessionId": session_id,
                "phase": phase.as_str(),
            }),
            CoreEvent::AccountUsage {
                provider,
                rate_limits,
            } => serde_json::json!({
                "provider": provider,
                "rateLimits": rate_limits,
            }),
        }
    }

    /// The durable replay cursor: a positive session-forest sequence for
    /// persisted events, `None` for transient sequence-zero frames and other
    /// live-only notifications.
    pub fn durable_cursor(&self) -> Option<i64> {
        match self {
            CoreEvent::Agent(event) if event.sequence > 0 => Some(event.sequence),
            _ => None,
        }
    }
}

#[derive(Default)]
struct ReconciliationState {
    state_changed: bool,
    adapters_changed: bool,
    learning_job_changed: Option<Value>,
    /// Which agents changed while a subscriber was lagging. A set rather than a
    /// flag because the hint names its agent, and ordered so replay is
    /// deterministic.
    managed_agents_changed: std::collections::BTreeSet<String>,
    /// Which memory scopes changed while a subscriber was lagging. Same shape
    /// as the agent set above, for the same reason: the hint names its scope.
    memory_scopes_changed: std::collections::BTreeSet<String>,
}

/// Receive failures exposed without coupling hosts to Tokio's channel types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    Lagged(u64),
    Closed,
}

/// A host subscription to the live event stream.
pub struct EventReceiver {
    receiver: broadcast::Receiver<CoreEvent>,
    reconciliation: Arc<Mutex<ReconciliationState>>,
}

impl EventReceiver {
    pub fn blocking_recv(&mut self) -> Result<CoreEvent, ReceiveError> {
        self.receiver.blocking_recv().map_err(|error| match error {
            broadcast::error::RecvError::Lagged(missed) => ReceiveError::Lagged(missed),
            broadcast::error::RecvError::Closed => ReceiveError::Closed,
        })
    }

    /// Idempotent refetch hints that restore convergence after a lag. It is
    /// safe to resend a hint seen earlier; clients simply refetch the latest
    /// authoritative state.
    pub fn reconciliation_events(&self) -> Vec<CoreEvent> {
        let state = self.reconciliation.lock().unwrap();
        let mut events = Vec::new();
        if state.state_changed {
            events.push(CoreEvent::StateChanged);
        }
        if state.adapters_changed {
            events.push(CoreEvent::AdaptersChanged);
        }
        if let Some(payload) = &state.learning_job_changed {
            events.push(CoreEvent::LearningJobChanged(payload.clone()));
        }
        for agent_id in &state.managed_agents_changed {
            events.push(CoreEvent::ManagedAgentChanged {
                agent_id: agent_id.clone(),
            });
        }
        for scope_key in &state.memory_scopes_changed {
            events.push(CoreEvent::MemoryChanged {
                scope_key: scope_key.clone(),
            });
        }
        events
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<CoreEvent, broadcast::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

/// The notify-only live channel. Cloning yields another handle to the same
/// channel, so boot-time publishers (adapter discovery) can capture one.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<CoreEvent>,
    reconciliation: Arc<Mutex<ReconciliationState>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self {
            sender,
            reconciliation: Arc::new(Mutex::new(ReconciliationState::default())),
        }
    }

    pub fn subscribe(&self) -> EventReceiver {
        EventReceiver {
            receiver: self.sender.subscribe(),
            reconciliation: self.reconciliation.clone(),
        }
    }

    /// Publish an event. Call sites must sit **after** the corresponding DB
    /// commit. Delivery is best-effort by design: no subscribers (or lagged
    /// subscribers) are not errors, because durable history lives in SQLite.
    pub fn publish(&self, event: CoreEvent) {
        {
            let mut state = self.reconciliation.lock().unwrap();
            match &event {
                CoreEvent::StateChanged => state.state_changed = true,
                CoreEvent::AdaptersChanged => state.adapters_changed = true,
                CoreEvent::LearningJobChanged(payload) => {
                    state.learning_job_changed = Some(payload.clone())
                }
                // A refetch hint, like StateChanged and AdaptersChanged above: a
                // client that missed it while lagging still has to learn that it
                // must re-read, so it is reconciled rather than dropped.
                CoreEvent::ManagedAgentChanged { agent_id } => {
                    state.managed_agents_changed.insert(agent_id.clone());
                }
                CoreEvent::MemoryChanged { scope_key } => {
                    state.memory_scopes_changed.insert(scope_key.clone());
                }
                CoreEvent::Agent(_)
                | CoreEvent::SessionOutput { .. }
                | CoreEvent::TerminalExited { .. }
                | CoreEvent::AccountUsage { .. }
                | CoreEvent::SessionStartup { .. } => {}
            }
        }
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::notifications::DeliveryClass;

    #[test]
    fn terminal_payloads_carry_the_shell_identity() {
        let output = CoreEvent::SessionOutput {
            session_id: "w".into(),
            terminal_id: "t1".into(),
            data: "ok".into(),
        };
        assert_eq!(output.kind(), NotificationName::SessionOutput);
        assert_eq!(
            output.payload(),
            serde_json::json!({"sessionId": "w", "terminalId": "t1", "data": "ok"})
        );

        let exited = CoreEvent::TerminalExited { session_id: "w".into(), terminal_id: "t1".into() };
        assert_eq!(exited.kind(), NotificationName::TerminalExited);
        assert_eq!(
            exited.payload(),
            serde_json::json!({"sessionId": "w", "terminalId": "t1"})
        );
    }

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
            CoreEvent::MemoryChanged {
                scope_key: "account:local".into(),
            },
            CoreEvent::SessionOutput {
                session_id: "s".into(),
                terminal_id: "t1".into(),
                data: "$ ls".into(),
            },
            CoreEvent::AccountUsage {
                provider: "codex".into(),
                rate_limits: serde_json::json!({}),
            },
            CoreEvent::ManagedAgentChanged {
                agent_id: "codex".into(),
            },
            CoreEvent::TerminalExited {
                session_id: "s".into(),
                terminal_id: "t1".into(),
            },
            CoreEvent::SessionStartup {
                session_id: "s".into(),
                phase: crate::adapters::StartupPhase::Spawning,
            },
        ];
        for event in &events {
            assert_eq!(
                event.durable_cursor().is_some(),
                matches!(event, CoreEvent::Agent(_))
            );
        }
        let kinds: std::collections::HashSet<_> = events.iter().map(|event| event.kind()).collect();
        // Every registry entry is either produced by a CoreEvent variant or
        // synthesized by a host about its own delivery channel. Listing the
        // host-synthesized set here keeps the two exhaustive together: a new
        // registry entry fails this test until it is claimed by one side.
        let host_synthesized = [NotificationName::StreamLagged];
        assert_eq!(
            kinds.len() + host_synthesized.len(),
            NotificationName::ALL.len(),
            "every notification kind is covered"
        );
        for synthesized in host_synthesized {
            assert!(
                !kinds.contains(&synthesized),
                "{} is host-synthesized, never published on the core bus",
                synthesized.as_str()
            );
        }
        let transient_agent = CoreEvent::Agent(agent_event(0));
        assert_eq!(transient_agent.kind().delivery(), DeliveryClass::Mixed);
        assert_eq!(transient_agent.durable_cursor(), None);
    }

    #[test]
    fn payloads_match_the_legacy_emit_shapes() {
        assert_eq!(CoreEvent::StateChanged.payload(), Value::Null);
        let output = CoreEvent::SessionOutput {
            session_id: "s".into(),
            terminal_id: "t1".into(),
            data: "hi".into(),
        };
        assert_eq!(
            output.payload(),
            serde_json::json!({"sessionId":"s","terminalId":"t1","data":"hi"})
        );
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
        let memory = CoreEvent::MemoryChanged {
            scope_key: "account:local".into(),
        };
        assert_eq!(
            memory.payload(),
            serde_json::json!({"scopeKey":"account:local"}),
            "the hint names the scope and carries no record"
        );
        let startup = CoreEvent::SessionStartup {
            session_id: "s".into(),
            phase: crate::adapters::StartupPhase::Handshake,
        };
        assert_eq!(startup.kind(), NotificationName::SessionStartup);
        assert_eq!(
            startup.payload(),
            serde_json::json!({"sessionId": "s", "phase": "handshake"})
        );
        assert_eq!(startup.durable_cursor(), None);
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
        sender
            .send(CoreEvent::Agent(durable_store[0].clone()))
            .unwrap();
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
                        assert!(
                            cursor <= last_cursor,
                            "buffered events never exceed the replayed cursor"
                        );
                    }
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Closed) => unreachable!(),
            }
        }
        assert_eq!(last_cursor, 6);
    }

    #[test]
    fn lagged_hosts_can_reemit_every_refetch_hint() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish(CoreEvent::StateChanged);
        bus.publish(CoreEvent::AdaptersChanged);
        bus.publish(CoreEvent::LearningJobChanged(
            serde_json::json!({"id":"latest"}),
        ));
        bus.publish(CoreEvent::MemoryChanged {
            scope_key: "account:local".into(),
        });
        for index in 0..=EVENT_BUS_CAPACITY {
            bus.publish(CoreEvent::SessionOutput {
                session_id: "s".into(),
                terminal_id: "t1".into(),
                data: index.to_string(),
            });
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        let recovered = receiver.reconciliation_events();
        assert!(recovered
            .iter()
            .any(|event| matches!(event, CoreEvent::StateChanged)));
        assert!(recovered
            .iter()
            .any(|event| matches!(event, CoreEvent::AdaptersChanged)));
        assert!(recovered.iter().any(|event| matches!(
            event,
            CoreEvent::LearningJobChanged(payload) if payload["id"] == "latest"
        )));
        assert!(recovered.iter().any(|event| matches!(
            event,
            CoreEvent::MemoryChanged { scope_key } if scope_key == "account:local"
        )));
    }
}

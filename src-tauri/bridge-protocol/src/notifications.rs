//! The notification namespace: every event a Bridge host pushes to clients,
//! with its delivery class.
//!
//! **Durable** notifications are backed by the session forest (SQLite), carry
//! a per-session sequence cursor, and are replayable after a disconnect or a
//! lagged live channel. **Mixed** notifications share a wire name between
//! durable records and transient streaming frames; the positive cursor is the
//! discriminator. **Transient** notifications are delivered live only.
//!
//! Wire names are the Tauri event names, so the migration compatibility
//! adapter forwards them unchanged.

macro_rules! notifications {
    ($(($variant:ident, $name:literal, $class:ident)),* $(,)?) => {
        /// A notification kind in the registry.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum NotificationName {
            $($variant),*
        }

        impl NotificationName {
            pub const ALL: &'static [NotificationName] = &[$(NotificationName::$variant),*];

            /// The wire name (today: the Tauri event name).
            pub const fn as_str(self) -> &'static str {
                match self { $(NotificationName::$variant => $name),* }
            }

            pub const fn delivery(self) -> DeliveryClass {
                match self { $(NotificationName::$variant => DeliveryClass::$class),* }
            }
        }
    };
}

/// How a notification is delivered and recovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryClass {
    /// Backed by durable history with a sequence cursor; replayable with no
    /// gaps or duplicates after a disconnect or a lagged live channel.
    Durable,
    /// A wire name carries both persisted events (positive cursor) and
    /// transient frames (cursor zero). Clients replay only persisted events.
    Mixed,
    /// Live-only; never replayed. Either a refetch hint or data that is
    /// worthless once stale (terminal bytes, usage ticks).
    Transient,
}

impl DeliveryClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeliveryClass::Durable => "durable",
            DeliveryClass::Mixed => "mixed",
            DeliveryClass::Transient => "transient",
        }
    }
}

notifications![
    // Persisted conversation history has a positive session-forest cursor;
    // streaming deltas/progress use sequence zero and are transient.
    (AgentEvent, "agent-event", Mixed),
    // Refetch hints: the payload carries no state; clients re-read snapshots.
    (StateChanged, "state-changed", Transient),
    (AdaptersChanged, "adapters-changed", Transient),
    // Managed-runtime lifecycle: a refetch hint, like its neighbours above. The
    // payload carries the agent id only, and authoritative state comes from
    // `agents/list_managed_agents`. There is no progress stream, because the
    // operations complete before their method returns.
    (ManagedAgentChanged, "managed-agent-changed", Transient),
    (LearningJobChanged, "learning-job-changed", Transient),
    // Explicit memory pins changed. A refetch hint like the ones above: the
    // payload names the affected scope only, and the records themselves come
    // from `memory/list_memory_records`. Never the record — a pin the client
    // rebuilt from a notification would outlive the tombstone that removed it.
    (MemoryChanged, "memory-changed", Transient),
    // Explicitly transient streams: worthless once stale, never replayed.
    (SessionOutput, "session-output", Transient),
    // One shell ended — by exit, by kill, or by close. Transient like the
    // bytes: a client that missed it re-lists the workspace's terminals.
    (TerminalExited, "terminal-exited", Transient),
    (AccountUsage, "account-usage", Transient),
    // Host-synthesized: the live channel dropped events for this connection.
    // Durable history is intact — replay every watched session from its last
    // cursor via `sessions/replay_session_events`; refetch hints are resent
    // alongside this marker. The payload carries `{"missed": n}` when the
    // host knows how many frames were dropped, `{}` when it does not.
    (StreamLagged, "stream-lagged", Transient),
];

impl NotificationName {
    /// Parse a wire name.
    pub fn parse(name: &str) -> Option<NotificationName> {
        NotificationName::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn wire_names_are_unique_and_parse_back() {
        let mut seen = HashSet::new();
        for notification in NotificationName::ALL.iter().copied() {
            assert!(
                seen.insert(notification.as_str()),
                "duplicate {}",
                notification.as_str()
            );
            assert_eq!(
                NotificationName::parse(notification.as_str()),
                Some(notification)
            );
        }
        assert_eq!(NotificationName::parse("no-such-event"), None);
    }

    #[test]
    fn delivery_classes_are_pinned() {
        // Delivery class is contract: durable events promise replay, and
        // transient streams promise they will never be replayed. Changing a
        // class is a breaking protocol change — this test makes it loud.
        assert_eq!(
            NotificationName::AgentEvent.delivery(),
            DeliveryClass::Mixed
        );
        for transient in [
            NotificationName::StateChanged,
            NotificationName::AdaptersChanged,
            NotificationName::LearningJobChanged,
            NotificationName::MemoryChanged,
            NotificationName::SessionOutput,
            NotificationName::AccountUsage,
            NotificationName::StreamLagged,
        ] {
            assert_eq!(
                transient.delivery(),
                DeliveryClass::Transient,
                "{}",
                transient.as_str()
            );
        }
    }
}

//! The notification namespace: every event a Bridge host pushes to clients,
//! with its delivery class.
//!
//! **Durable** notifications are backed by the session forest (SQLite), carry
//! a per-session sequence cursor, and are replayable after a disconnect or a
//! lagged live channel — the live channel is notify-only and lossy, never the
//! source of truth. **Transient** notifications (terminal bytes, usage ticks,
//! refetch hints) are delivered live only and never replayed.
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
    /// Live-only; never replayed. Either a refetch hint or data that is
    /// worthless once stale (terminal bytes, usage ticks).
    Transient,
}

impl DeliveryClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeliveryClass::Durable => "durable",
            DeliveryClass::Transient => "transient",
        }
    }
}

notifications![
    // Durable conversation history: each payload is a session-forest entry
    // with `sessionId` and a monotonic per-session `sequence` cursor.
    (AgentEvent, "agent-event", Durable),
    // Refetch hints: the payload carries no state; clients re-read snapshots.
    (StateChanged, "state-changed", Transient),
    (AdaptersChanged, "adapters-changed", Transient),
    (LearningJobChanged, "learning-job-changed", Transient),
    // Explicitly transient streams: worthless once stale, never replayed.
    (SessionOutput, "session-output", Transient),
    (AccountUsage, "account-usage", Transient),
];

impl NotificationName {
    /// Parse a wire name.
    pub fn parse(name: &str) -> Option<NotificationName> {
        NotificationName::ALL.iter().copied().find(|candidate| candidate.as_str() == name)
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
            assert!(seen.insert(notification.as_str()), "duplicate {}", notification.as_str());
            assert_eq!(NotificationName::parse(notification.as_str()), Some(notification));
        }
        assert_eq!(NotificationName::parse("no-such-event"), None);
    }

    #[test]
    fn delivery_classes_are_pinned() {
        // Delivery class is contract: durable events promise replay, and
        // transient streams promise they will never be replayed. Changing a
        // class is a breaking protocol change — this test makes it loud.
        assert_eq!(NotificationName::AgentEvent.delivery(), DeliveryClass::Durable);
        for transient in [
            NotificationName::StateChanged,
            NotificationName::AdaptersChanged,
            NotificationName::LearningJobChanged,
            NotificationName::SessionOutput,
            NotificationName::AccountUsage,
        ] {
            assert_eq!(transient.delivery(), DeliveryClass::Transient, "{}", transient.as_str());
        }
    }
}

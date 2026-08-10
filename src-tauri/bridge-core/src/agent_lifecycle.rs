//! Installation readiness and process lifecycle for managed agents.
//!
//! Three facts about an agent are independent and this module never collapses
//! them into one boolean:
//!
//!   * Bridge owns a receipt-bound payload (`installed`).
//!   * The integration's own non-destructive readiness check passes (`ready`).
//!   * A process is alive right now (`running`).
//!
//! Like [`crate::worker_lifecycle`], the state machine owns transition
//! validation only — callers persist an accepted transition before publishing
//! events. The coordinator above it owns the ordering that keeps the filesystem
//! and the process table honest with each other.
//!
//! # Authentication boundary
//!
//! Vendor authentication stays vendor-owned. A vendor reporting a missing login
//! or API key is not a Bridge failure and is never represented as Bridge
//! credential state: readiness returns [`ReadinessOutcome::VendorBlocked`], the
//! vendor's own message is preserved verbatim, and the agent stays `installed`
//! rather than being recast as `broken` or promoted to `ready`. This module has
//! no credential store, no OAuth client, and no logout path.

use std::{error::Error, fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentLifecycleState {
    /// No Bridge-managed payload and no discoverable user-managed runtime.
    NotInstalled,
    /// A managed install is in flight and may still be cancelled.
    Installing,
    /// Bridge owns a receipt-bound payload; launch readiness is not yet proven.
    Installed,
    /// The integration's own readiness check passed. Spawn-on-use starts here.
    Ready,
    /// A live process exists.
    Running,
    /// A stop was requested and the process has not been reaped yet.
    Stopping,
    /// A managed payload is being removed.
    Uninstalling,
    /// A user-managed runtime is discoverable. Bridge holds no receipt for it
    /// and must never remove it.
    External,
    /// Unusable for a reason repair cannot be expected to fix on its own.
    Broken,
    /// Bridge owns the payload but it drifted from its receipt.
    Repairable,
}

impl AgentLifecycleState {
    pub const ALL: [Self; 10] = [
        Self::NotInstalled,
        Self::Installing,
        Self::Installed,
        Self::Ready,
        Self::Running,
        Self::Stopping,
        Self::Uninstalling,
        Self::External,
        Self::Broken,
        Self::Repairable,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Uninstalling => "uninstalling",
            Self::External => "external",
            Self::Broken => "broken",
            Self::Repairable => "repairable",
        }
    }

    /// Does Bridge hold, or expect to hold, a receipt for this agent's payload?
    ///
    /// False for `external`, where a discoverable user-managed runtime is usable
    /// but not Bridge's to remove, and false for `installing`, where staging has
    /// not been promoted yet so there is nothing to own. `broken` is owned: it is
    /// only reachable from states that already had a managed payload, which is
    /// why removing a broken install is allowed.
    pub const fn is_bridge_owned(self) -> bool {
        matches!(
            self,
            Self::Installed
                | Self::Ready
                | Self::Running
                | Self::Stopping
                | Self::Uninstalling
                | Self::Repairable
                | Self::Broken
        )
    }

    /// Might a process still be alive in this state?
    pub const fn may_have_process(self) -> bool {
        matches!(self, Self::Running | Self::Stopping)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        LEGAL_TRANSITIONS.contains(&(self, next))
    }
}

impl fmt::Display for AgentLifecycleState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AgentLifecycleState {
    type Err = ParseAgentLifecycleStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| ParseAgentLifecycleStateError(value.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseAgentLifecycleStateError(pub String);

impl fmt::Display for ParseAgentLifecycleStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown agent lifecycle state: {}", self.0)
    }
}

impl Error for ParseAgentLifecycleStateError {}

/// The complete transition relation.
///
/// Two absences are load-bearing rather than oversights, and both are asserted
/// in the tests below:
///
///   * `running` has no edge to `uninstalling`. Removing a payload underneath a
///     live process, or handing a fresh launch a path that is being deleted, is
///     only prevented if stopping is a state the machine must pass through.
///   * `external` has no edge to `uninstalling`. Bridge holds no receipt for a
///     user-managed runtime, so there is no state from which it could begin
///     removing one.
pub const LEGAL_TRANSITIONS: [(AgentLifecycleState, AgentLifecycleState); 32] = [
    // Discovery.
    (
        AgentLifecycleState::NotInstalled,
        AgentLifecycleState::Installing,
    ),
    (
        AgentLifecycleState::NotInstalled,
        AgentLifecycleState::External,
    ),
    (
        AgentLifecycleState::External,
        AgentLifecycleState::NotInstalled,
    ),
    // Opting into a managed payload alongside a user-managed runtime.
    (
        AgentLifecycleState::External,
        AgentLifecycleState::Installing,
    ),
    // Install, including cancellation back to where it started.
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::NotInstalled,
    ),
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::Repairable,
    ),
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::Broken,
    ),
    // Readiness is a separate promotion from ownership.
    (AgentLifecycleState::Installed, AgentLifecycleState::Ready),
    (
        AgentLifecycleState::Installed,
        AgentLifecycleState::Repairable,
    ),
    (AgentLifecycleState::Installed, AgentLifecycleState::Broken),
    (
        AgentLifecycleState::Installed,
        AgentLifecycleState::Uninstalling,
    ),
    // Spawn-on-use.
    (AgentLifecycleState::Ready, AgentLifecycleState::Running),
    (AgentLifecycleState::Ready, AgentLifecycleState::Installed),
    (AgentLifecycleState::Ready, AgentLifecycleState::Repairable),
    (AgentLifecycleState::Ready, AgentLifecycleState::Broken),
    (
        AgentLifecycleState::Ready,
        AgentLifecycleState::Uninstalling,
    ),
    // Run and stop.
    (AgentLifecycleState::Running, AgentLifecycleState::Stopping),
    (AgentLifecycleState::Running, AgentLifecycleState::Ready),
    (AgentLifecycleState::Running, AgentLifecycleState::Broken),
    (AgentLifecycleState::Stopping, AgentLifecycleState::Ready),
    (
        AgentLifecycleState::Stopping,
        AgentLifecycleState::Uninstalling,
    ),
    (AgentLifecycleState::Stopping, AgentLifecycleState::Broken),
    // Uninstall, including a refusal that leaves the payload intact.
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::NotInstalled,
    ),
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::Broken,
    ),
    // Repair.
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Installing,
    ),
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Uninstalling,
    ),
    // Retry and removal out of a broken install.
    (AgentLifecycleState::Broken, AgentLifecycleState::Installing),
    (
        AgentLifecycleState::Broken,
        AgentLifecycleState::Uninstalling,
    ),
    (
        AgentLifecycleState::Broken,
        AgentLifecycleState::NotInstalled,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAgentLifecycleTransition {
    pub from: AgentLifecycleState,
    pub to: AgentLifecycleState,
}

impl fmt::Display for InvalidAgentLifecycleTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "illegal agent lifecycle transition: {} -> {}",
            self.from, self.to
        )
    }
}

impl Error for InvalidAgentLifecycleTransition {}

pub fn validate_transition(
    from: AgentLifecycleState,
    to: AgentLifecycleState,
) -> Result<(), InvalidAgentLifecycleTransition> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(InvalidAgentLifecycleTransition { from, to })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLifecycle {
    state: AgentLifecycleState,
}

impl AgentLifecycle {
    pub const fn new(state: AgentLifecycleState) -> Self {
        Self { state }
    }

    pub const fn state(self) -> AgentLifecycleState {
        self.state
    }

    pub fn transition_to(
        &mut self,
        next: AgentLifecycleState,
    ) -> Result<(), InvalidAgentLifecycleTransition> {
        validate_transition(self.state, next)?;
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_state_pair_matches_the_locked_transition_matrix() {
        let expected = LEGAL_TRANSITIONS.into_iter().collect::<HashSet<_>>();
        assert_eq!(
            expected.len(),
            LEGAL_TRANSITIONS.len(),
            "legal transitions must be unique"
        );

        let mut accepted = 0;
        for from in AgentLifecycleState::ALL {
            for to in AgentLifecycleState::ALL {
                let should_accept = expected.contains(&(from, to));
                assert_eq!(
                    validate_transition(from, to).is_ok(),
                    should_accept,
                    "unexpected transition verdict for {from} -> {to}"
                );
                if should_accept {
                    accepted += 1;
                }
            }
        }
        assert_eq!(accepted, LEGAL_TRANSITIONS.len());
        assert_eq!(
            AgentLifecycleState::ALL.len() * AgentLifecycleState::ALL.len(),
            100,
            "the matrix must cover every ordered pair of the ten states"
        );
    }

    #[test]
    fn wire_strings_round_trip_and_reject_unknown_states() {
        let mut seen = HashSet::new();
        for state in AgentLifecycleState::ALL {
            assert!(seen.insert(state.as_str()), "duplicate wire string");
            assert_eq!(state.as_str().parse::<AgentLifecycleState>().unwrap(), state);
            assert_eq!(state.to_string(), state.as_str());
        }
        assert_eq!(
            "ready".parse::<AgentLifecycleState>().unwrap(),
            AgentLifecycleState::Ready
        );
        let error = "Ready".parse::<AgentLifecycleState>().unwrap_err();
        assert_eq!(error, ParseAgentLifecycleStateError("Ready".into()));
        assert!(error.to_string().contains("unknown agent lifecycle state"));
        assert!("".parse::<AgentLifecycleState>().is_err());
    }

    #[test]
    fn running_cannot_reach_uninstalling_without_stopping() {
        let direct = validate_transition(
            AgentLifecycleState::Running,
            AgentLifecycleState::Uninstalling,
        )
        .unwrap_err();
        assert_eq!(direct.from, AgentLifecycleState::Running);
        assert_eq!(direct.to, AgentLifecycleState::Uninstalling);
        assert!(direct
            .to_string()
            .contains("illegal agent lifecycle transition: running -> uninstalling"));

        let mut lifecycle = AgentLifecycle::new(AgentLifecycleState::Running);
        assert!(lifecycle
            .transition_to(AgentLifecycleState::Uninstalling)
            .is_err());
        assert_eq!(
            lifecycle.state(),
            AgentLifecycleState::Running,
            "a rejected transition must not mutate state"
        );
        lifecycle
            .transition_to(AgentLifecycleState::Stopping)
            .unwrap();
        lifecycle
            .transition_to(AgentLifecycleState::Uninstalling)
            .unwrap();
    }

    #[test]
    fn no_state_that_may_hold_a_process_reaches_uninstalling_directly() {
        // `stopping` is the one exception: reaching it is the act of reaping.
        for from in AgentLifecycleState::ALL.into_iter().filter(|state| {
            state.may_have_process() && *state != AgentLifecycleState::Stopping
        }) {
            assert!(
                validate_transition(from, AgentLifecycleState::Uninstalling).is_err(),
                "{from} must stop before uninstalling"
            );
        }
    }

    #[test]
    fn external_runtimes_have_no_path_into_uninstalling() {
        assert!(!AgentLifecycleState::External.is_bridge_owned());
        assert!(validate_transition(
            AgentLifecycleState::External,
            AgentLifecycleState::Uninstalling
        )
        .is_err());
        // Nor by way of any state Bridge does not own the payload in.
        for from in AgentLifecycleState::ALL
            .into_iter()
            .filter(|state| !state.is_bridge_owned())
        {
            assert!(
                validate_transition(from, AgentLifecycleState::Uninstalling).is_err(),
                "{from} does not own a payload and must not begin uninstalling"
            );
        }
    }

    #[test]
    fn repair_and_retry_paths_are_explicit() {
        let mut lifecycle = AgentLifecycle::new(AgentLifecycleState::Repairable);
        lifecycle
            .transition_to(AgentLifecycleState::Installing)
            .unwrap();
        lifecycle
            .transition_to(AgentLifecycleState::Installed)
            .unwrap();

        let mut broken = AgentLifecycle::new(AgentLifecycleState::Broken);
        assert!(
            broken.transition_to(AgentLifecycleState::Ready).is_err(),
            "a broken agent must be reinstalled or repaired, never promoted straight to ready"
        );
        broken.transition_to(AgentLifecycleState::Installing).unwrap();
    }

    #[test]
    fn nothing_reaches_installed_ready_or_running_without_passing_through_install() {
        for target in [
            AgentLifecycleState::Ready,
            AgentLifecycleState::Running,
            AgentLifecycleState::Stopping,
        ] {
            assert!(
                validate_transition(AgentLifecycleState::NotInstalled, target).is_err(),
                "not_installed must not jump to {target}"
            );
        }
        assert!(validate_transition(
            AgentLifecycleState::NotInstalled,
            AgentLifecycleState::Installed
        )
        .is_err());
    }
}

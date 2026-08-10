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

use crate::managed_payload::{ManagedPayloadStatus, RepairReason};
use std::{error::Error, fmt, path::PathBuf, str::FromStr};

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

/// What the #165 payload engine says about the payload, reduced to what the
/// lifecycle needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadCondition {
    /// No Bridge-managed payload.
    Absent,
    /// Bridge owns a receipt-bound payload with a verified entrypoint.
    Installed { entrypoint: PathBuf },
    /// Bridge owns the payload but it no longer matches its receipt.
    Repairable { reason: RepairReason },
}

impl PayloadCondition {
    /// Reduce a [`ManagedPayloadStatus`] without reinterpreting it.
    ///
    /// Every repair reason stays a repair reason, including
    /// `EntrypointNotExecutable`: an installed payload that cannot be executed
    /// is drift to be repaired, never a `ready` agent.
    pub fn from_status(status: ManagedPayloadStatus) -> Self {
        match status {
            ManagedPayloadStatus::NotInstalled => Self::Absent,
            ManagedPayloadStatus::Installed { entrypoint, .. } => Self::Installed { entrypoint },
            ManagedPayloadStatus::Repairable { reason } => Self::Repairable { reason },
        }
    }
}

/// A user-managed runtime Bridge can see but does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRuntime {
    pub candidate: PathBuf,
}

/// The result of an integration's own non-destructive readiness check.
///
/// Bridge does not implement these checks — Claude's is a Node and sidecar
/// probe, Codex's is a `codex --version`, OpenCode's is its provider catalog —
/// and this enum exists so their verdicts can be carried without being
/// rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessOutcome {
    /// The integration can launch.
    Ready { version: Option<String> },
    /// The integration cannot launch for a reason reinstalling will not fix — a
    /// missing runtime prerequisite, say.
    Unavailable { reason: String },
    /// The vendor reported that it needs a login or an API key.
    ///
    /// Deliberately distinct from [`Self::Unavailable`]: nothing is broken, the
    /// user simply has not authenticated with the vendor. `vendor_message` is
    /// the vendor's own text, carried verbatim, and Bridge stores no credential
    /// state of its own for it.
    VendorBlocked { vendor_message: String },
    /// Not probed — there was no payload to probe.
    NotProbed,
}

impl ReadinessOutcome {
    /// Is this verdict the vendor's rather than Bridge's?
    ///
    /// Callers use this to keep a vendor auth prompt recognizable as a vendor
    /// concern instead of reporting it as a Bridge failure.
    pub const fn is_vendor_owned(&self) -> bool {
        matches!(self, Self::VendorBlocked { .. })
    }

    /// The message to show, if any. Vendor text is never reworded.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Ready { .. } | Self::NotProbed => None,
            Self::Unavailable { reason } => Some(reason),
            Self::VendorBlocked { vendor_message } => Some(vendor_message),
        }
    }
}

/// Whether a process is alive right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessCondition {
    None,
    Running { pid: u32 },
}

/// The three independent facts, gathered from sources this module does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleObservation {
    pub payload: PayloadCondition,
    pub external: Option<ExternalRuntime>,
    pub readiness: ReadinessOutcome,
    pub process: ProcessCondition,
}

impl LifecycleObservation {
    pub fn absent() -> Self {
        Self {
            payload: PayloadCondition::Absent,
            external: None,
            readiness: ReadinessOutcome::NotProbed,
            process: ProcessCondition::None,
        }
    }

    /// Resolve the settled state these facts describe.
    ///
    /// "Settled" means no operation is in flight: `installing`, `stopping`, and
    /// `uninstalling` are owned by the coordinator driving an operation and are
    /// never inferred from an observation.
    ///
    /// Precedence, in order, and each rule is load-bearing:
    ///
    /// 1. **A live process resolves to `running`, whatever the payload says.**
    ///    `stopping` is only reachable from `running`, so any other answer would
    ///    leave a live process in a state Bridge cannot legally stop it from. A
    ///    payload that drifted or vanished underneath a running process becomes
    ///    actionable once the process is reaped, and until then the observation
    ///    still carries the payload condition for callers that want to warn.
    /// 2. Drift outranks readiness. A `repairable` payload is never `ready`.
    /// 3. A Bridge-managed payload outranks a discoverable external runtime,
    ///    because the managed payload is the one Bridge would launch. `external`
    ///    describes the case where there is nothing managed to launch.
    /// 4. Readiness promotes `installed` to `ready`. A vendor auth block leaves
    ///    the agent `installed` — owned, not launch-ready, nothing broken.
    pub fn settled_state(&self) -> AgentLifecycleState {
        if matches!(self.process, ProcessCondition::Running { .. }) {
            return AgentLifecycleState::Running;
        }
        match &self.payload {
            PayloadCondition::Repairable { .. } => AgentLifecycleState::Repairable,
            PayloadCondition::Absent => {
                if self.external.is_some() {
                    AgentLifecycleState::External
                } else {
                    AgentLifecycleState::NotInstalled
                }
            }
            PayloadCondition::Installed { .. } => match &self.readiness {
                ReadinessOutcome::Ready { .. } => AgentLifecycleState::Ready,
                ReadinessOutcome::Unavailable { .. } => AgentLifecycleState::Broken,
                ReadinessOutcome::VendorBlocked { .. } | ReadinessOutcome::NotProbed => {
                    AgentLifecycleState::Installed
                }
            },
        }
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

    fn installed_payload() -> PayloadCondition {
        PayloadCondition::Installed {
            entrypoint: PathBuf::from("/managed/agents/a/installations/i/payload/bin/agent"),
        }
    }

    fn external() -> Option<ExternalRuntime> {
        Some(ExternalRuntime {
            candidate: PathBuf::from("/usr/local/bin/agent"),
        })
    }

    #[test]
    fn settled_state_separates_payload_readiness_and_process() {
        let cases: [(LifecycleObservation, AgentLifecycleState); 9] = [
            (LifecycleObservation::absent(), AgentLifecycleState::NotInstalled),
            (
                LifecycleObservation {
                    external: external(),
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::External,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Installed,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Ready {
                        version: Some("1.2.3".into()),
                    },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Ready,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Unavailable {
                        reason: "Node.js 18+ is required to run Claude models".into(),
                    },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Broken,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Ready { version: None },
                    process: ProcessCondition::Running { pid: 4321 },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Running,
            ),
            // Drift outranks readiness: an installed-but-unexecutable payload is
            // repairable, never ready. This is the #165 EntrypointNotExecutable
            // reason arriving through PayloadCondition::from_status.
            (
                LifecycleObservation {
                    payload: PayloadCondition::Repairable {
                        reason: RepairReason::EntrypointNotExecutable,
                    },
                    readiness: ReadinessOutcome::Ready { version: None },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Repairable,
            ),
            // A managed payload outranks a discoverable external runtime.
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    external: external(),
                    readiness: ReadinessOutcome::Ready { version: None },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Ready,
            ),
            // Liveness outranks the payload condition, because `stopping` is only
            // reachable from `running`: resolving a live process to anything else
            // would leave it in a state Bridge cannot legally stop it from.
            (
                LifecycleObservation {
                    payload: PayloadCondition::Absent,
                    process: ProcessCondition::Running { pid: 99 },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Running,
            ),
        ];

        for (observation, expected) in cases {
            assert_eq!(
                observation.settled_state(),
                expected,
                "unexpected settled state for {observation:?}"
            );
        }

        // And that last case really is recoverable rather than a dead end.
        let mut stranded = AgentLifecycle::new(AgentLifecycleState::Running);
        stranded.transition_to(AgentLifecycleState::Stopping).unwrap();
        stranded
            .transition_to(AgentLifecycleState::Uninstalling)
            .unwrap();
    }

    #[test]
    fn settled_state_never_infers_an_in_flight_operation() {
        // installing/stopping/uninstalling belong to the coordinator driving an
        // operation and must never be produced by observing the world.
        for payload in [
            PayloadCondition::Absent,
            installed_payload(),
            PayloadCondition::Repairable {
                reason: RepairReason::IntegrityDrift,
            },
        ] {
            for readiness in [
                ReadinessOutcome::NotProbed,
                ReadinessOutcome::Ready { version: None },
                ReadinessOutcome::Unavailable { reason: "x".into() },
                ReadinessOutcome::VendorBlocked {
                    vendor_message: "y".into(),
                },
            ] {
                for process in [ProcessCondition::None, ProcessCondition::Running { pid: 1 }] {
                    for external in [None, external()] {
                        let state = LifecycleObservation {
                            payload: payload.clone(),
                            external,
                            readiness: readiness.clone(),
                            process,
                        }
                        .settled_state();
                        assert!(
                            !matches!(
                                state,
                                AgentLifecycleState::Installing
                                    | AgentLifecycleState::Stopping
                                    | AgentLifecycleState::Uninstalling
                            ),
                            "observation must not infer the in-flight state {state}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn vendor_auth_block_keeps_the_agent_installed_and_the_message_intact() {
        let vendor_message =
            "Not logged in. Run `codex login` to authenticate with your ChatGPT account.";
        let readiness = ReadinessOutcome::VendorBlocked {
            vendor_message: vendor_message.into(),
        };
        let observation = LifecycleObservation {
            payload: installed_payload(),
            readiness: readiness.clone(),
            ..LifecycleObservation::absent()
        };

        assert_eq!(
            observation.settled_state(),
            AgentLifecycleState::Installed,
            "a vendor auth prompt is not a Bridge failure and not readiness"
        );
        assert_ne!(observation.settled_state(), AgentLifecycleState::Broken);
        assert!(readiness.is_vendor_owned());
        assert_eq!(readiness.message(), Some(vendor_message));

        // A Bridge-side unavailability is the opposite: not vendor-owned.
        let bridge_side = ReadinessOutcome::Unavailable {
            reason: "Node.js 18+ is required".into(),
        };
        assert!(!bridge_side.is_vendor_owned());
        assert!(!ReadinessOutcome::Ready { version: None }.is_vendor_owned());
        assert_eq!(ReadinessOutcome::Ready { version: None }.message(), None);
    }

    #[test]
    fn payload_conditions_map_from_the_engine_without_reinterpretation() {
        assert_eq!(
            PayloadCondition::from_status(ManagedPayloadStatus::NotInstalled),
            PayloadCondition::Absent
        );
        for reason in [
            RepairReason::CorruptActiveReceipt,
            RepairReason::ActiveReceiptMismatch,
            RepairReason::MissingInstallation,
            RepairReason::CorruptEmbeddedReceipt,
            RepairReason::ReceiptChainMismatch,
            RepairReason::MissingPayload,
            RepairReason::MissingEntrypoint,
            RepairReason::EntrypointNotExecutable,
            RepairReason::IntegrityDrift,
            RepairReason::UnsafeManagedPath,
        ] {
            let condition =
                PayloadCondition::from_status(ManagedPayloadStatus::Repairable { reason });
            assert_eq!(
                condition,
                PayloadCondition::Repairable { reason },
                "repair reasons must survive the reduction unchanged"
            );
            assert_eq!(
                LifecycleObservation {
                    payload: condition,
                    ..LifecycleObservation::absent()
                }
                .settled_state(),
                AgentLifecycleState::Repairable
            );
        }
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

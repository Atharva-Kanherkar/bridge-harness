//! Pure worker lifecycle state machine.
//!
//! This module owns transition validation only. Callers remain responsible for
//! persisting an accepted transition before publishing session-forest or UI events.

use std::{error::Error, fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerLifecycleState {
    Starting,
    Working,
    Waiting,
    Warm,
    Checkpointing,
    Stopped,
    Resuming,
    Restored,
    Failed,
    Completed,
    Cancelled,
}

impl WorkerLifecycleState {
    pub const ALL: [Self; 11] = [
        Self::Starting,
        Self::Working,
        Self::Waiting,
        Self::Warm,
        Self::Checkpointing,
        Self::Stopped,
        Self::Resuming,
        Self::Restored,
        Self::Failed,
        Self::Completed,
        Self::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Warm => "warm",
            Self::Checkpointing => "checkpointing",
            Self::Stopped => "stopped",
            Self::Resuming => "resuming",
            Self::Restored => "restored",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        LEGAL_TRANSITIONS.contains(&(self, next))
    }
}

impl fmt::Display for WorkerLifecycleState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for WorkerLifecycleState {
    type Err = ParseWorkerLifecycleStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| ParseWorkerLifecycleStateError(value.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWorkerLifecycleStateError(pub String);

impl fmt::Display for ParseWorkerLifecycleStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown worker lifecycle state: {}", self.0)
    }
}

impl Error for ParseWorkerLifecycleStateError {}

/// The complete transition relation locked by the issue #1 state diagram.
pub const LEGAL_TRANSITIONS: [(WorkerLifecycleState, WorkerLifecycleState); 25] = [
    (
        WorkerLifecycleState::Starting,
        WorkerLifecycleState::Working,
    ),
    (WorkerLifecycleState::Working, WorkerLifecycleState::Waiting),
    (WorkerLifecycleState::Waiting, WorkerLifecycleState::Working),
    (WorkerLifecycleState::Working, WorkerLifecycleState::Warm),
    (
        WorkerLifecycleState::Working,
        WorkerLifecycleState::Completed,
    ),
    (
        WorkerLifecycleState::Working,
        WorkerLifecycleState::Checkpointing,
    ),
    (WorkerLifecycleState::Warm, WorkerLifecycleState::Working),
    (
        WorkerLifecycleState::Warm,
        WorkerLifecycleState::Checkpointing,
    ),
    (
        WorkerLifecycleState::Checkpointing,
        WorkerLifecycleState::Stopped,
    ),
    (
        WorkerLifecycleState::Stopped,
        WorkerLifecycleState::Resuming,
    ),
    (
        WorkerLifecycleState::Resuming,
        WorkerLifecycleState::Working,
    ),
    (
        WorkerLifecycleState::Resuming,
        WorkerLifecycleState::Restored,
    ),
    (
        WorkerLifecycleState::Restored,
        WorkerLifecycleState::Working,
    ),
    (WorkerLifecycleState::Working, WorkerLifecycleState::Failed),
    (WorkerLifecycleState::Failed, WorkerLifecycleState::Resuming),
    (
        WorkerLifecycleState::Failed,
        WorkerLifecycleState::Completed,
    ),
    (
        WorkerLifecycleState::Working,
        WorkerLifecycleState::Cancelled,
    ),
    (
        WorkerLifecycleState::Waiting,
        WorkerLifecycleState::Cancelled,
    ),
    // A stop has to be able to land wherever a worker actually is. When
    // `Cancelled` was reachable only from `working` and `waiting`, stopping a
    // `warm` worker tore its process down and told the parent it had ended
    // while its lifecycle stayed `warm` — leaving a session with no process
    // still eligible for reuse, so the next compatible objective resumed a
    // corpse. Every non-terminal state can be cancelled; the terminal ones
    // are already over.
    (
        WorkerLifecycleState::Starting,
        WorkerLifecycleState::Cancelled,
    ),
    (WorkerLifecycleState::Warm, WorkerLifecycleState::Cancelled),
    (
        WorkerLifecycleState::Checkpointing,
        WorkerLifecycleState::Cancelled,
    ),
    (
        WorkerLifecycleState::Resuming,
        WorkerLifecycleState::Cancelled,
    ),
    (
        WorkerLifecycleState::Restored,
        WorkerLifecycleState::Cancelled,
    ),
    (WorkerLifecycleState::Failed, WorkerLifecycleState::Cancelled),
    (
        WorkerLifecycleState::Stopped,
        WorkerLifecycleState::Cancelled,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWorkerLifecycleTransition {
    pub from: WorkerLifecycleState,
    pub to: WorkerLifecycleState,
}

impl fmt::Display for InvalidWorkerLifecycleTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "illegal worker lifecycle transition: {} -> {}",
            self.from, self.to
        )
    }
}

impl Error for InvalidWorkerLifecycleTransition {}

pub fn validate_transition(
    from: WorkerLifecycleState,
    to: WorkerLifecycleState,
) -> Result<(), InvalidWorkerLifecycleTransition> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(InvalidWorkerLifecycleTransition { from, to })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerLifecycle {
    state: WorkerLifecycleState,
}

impl WorkerLifecycle {
    pub const fn new(state: WorkerLifecycleState) -> Self {
        Self { state }
    }

    pub const fn state(self) -> WorkerLifecycleState {
        self.state
    }

    pub fn transition_to(
        &mut self,
        next: WorkerLifecycleState,
    ) -> Result<(), InvalidWorkerLifecycleTransition> {
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
    fn all_state_pairs_match_the_locked_transition_matrix() {
        let expected = LEGAL_TRANSITIONS.into_iter().collect::<HashSet<_>>();
        assert_eq!(expected.len(), 25, "legal transitions must be unique");

        let mut accepted = 0;
        let mut rejected = 0;
        for from in WorkerLifecycleState::ALL {
            for to in WorkerLifecycleState::ALL {
                let should_accept = expected.contains(&(from, to));
                assert_eq!(
                    validate_transition(from, to).is_ok(),
                    should_accept,
                    "unexpected transition verdict for {from} -> {to}"
                );
                if should_accept {
                    accepted += 1;
                } else {
                    rejected += 1;
                }
            }
        }

        assert_eq!(accepted, 25);
        assert_eq!(rejected, 96);
    }

    #[test]
    fn legal_transitions_update_the_machine_state() {
        for (from, to) in LEGAL_TRANSITIONS {
            let mut lifecycle = WorkerLifecycle::new(from);
            assert_eq!(lifecycle.transition_to(to), Ok(()));
            assert_eq!(lifecycle.state(), to);
        }
    }

    #[test]
    fn illegal_transitions_do_not_mutate_machine_state() {
        let legal = LEGAL_TRANSITIONS.into_iter().collect::<HashSet<_>>();
        for from in WorkerLifecycleState::ALL {
            for to in WorkerLifecycleState::ALL {
                if legal.contains(&(from, to)) {
                    continue;
                }

                let mut lifecycle = WorkerLifecycle::new(from);
                assert_eq!(
                    lifecycle.transition_to(to),
                    Err(InvalidWorkerLifecycleTransition { from, to })
                );
                assert_eq!(lifecycle.state(), from);
            }
        }
    }

    #[test]
    fn terminal_states_have_no_outgoing_transitions() {
        for terminal in [
            WorkerLifecycleState::Completed,
            WorkerLifecycleState::Cancelled,
        ] {
            assert!(terminal.is_terminal());
            for target in WorkerLifecycleState::ALL {
                assert!(validate_transition(terminal, target).is_err());
            }
        }

        for non_terminal in WorkerLifecycleState::ALL
            .into_iter()
            .filter(|state| !state.is_terminal())
        {
            assert!(!non_terminal.is_terminal());
        }
    }

    #[test]
    fn state_names_are_stable_snake_case() {
        let expected = [
            "starting",
            "working",
            "waiting",
            "warm",
            "checkpointing",
            "stopped",
            "resuming",
            "restored",
            "failed",
            "completed",
            "cancelled",
        ];

        let actual = WorkerLifecycleState::ALL.map(WorkerLifecycleState::as_str);
        assert_eq!(actual, expected);
        assert_eq!(actual.into_iter().collect::<HashSet<_>>().len(), 11);
        for state in WorkerLifecycleState::ALL {
            assert_eq!(state.as_str().parse(), Ok(state));
            assert_eq!(state.to_string(), state.as_str());
        }
        assert!("unknown".parse::<WorkerLifecycleState>().is_err());
    }
}

//! Early native compaction: Bridge asks a top-level direct Claude or Codex
//! chat to compact its own window at an idle turn boundary.
//!
//! This file holds the pure rule and the memory of the trigger. The rule
//! reads no file, no clock and no database. Each input comes in as a value,
//! so the same inputs give the same decision.
//!
//! The frame path hands the [`Tracker`] a [`Receipt`] for a turn that can
//! lead to a compaction. A sweep checks [`rule::precheck`] from memory, then
//! reads one [`Snapshot`] of the chat and asks [`rule::decide`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::adapters::NativeCompaction;

/// A bound window reading at or above this count fires. Provisional: the
/// live eval and a ledger query set the final value.
pub const THRESHOLD_TOKENS: i64 = 100_000;
/// A mark younger than this waits. The Claude reading comes after the turn
/// end, and a person who types again soon wins.
pub const IDLE_DELAY: Duration = Duration::from_secs(20);
/// A mark older than this goes. After it, the prompt cache can be cold and
/// the idle release can stop the process.
pub const WARM_LIMIT: Duration = Duration::from_secs(100);
/// An open attempt with no terminal signal for this long settles as
/// `unsettled`.
pub const SETTLE_LIMIT: Duration = Duration::from_secs(300);
/// The search for the next reading of a compacted attempt stops after this.
pub const NEXT_READING_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The open tool scan reads at most this many entries, newest first.
pub const TOOL_SCAN_CAP: usize = 2_000;
/// The harnesses of version 1. OpenCode and each other harness get a
/// suppression, never a dispatch.
pub const HARNESSES: [&str; 2] = ["claude", "codex"];

// The idle release must not stop the process of a chat while its mark is
// still warm.
const _: () = assert!(WARM_LIMIT.as_secs() < crate::runtime_budget::IDLE_TTL.as_secs());

const DIRECT_KIND: &str = "direct";
/// A new chat is `idle`. A chat after a turn or a compaction is `ready`.
const IDLE_STATUSES: [&str; 2] = ["ready", "idle"];
/// The sidecar reads the Claude window after the `result` frame, when the
/// turn is over, so this row has no turn id.
const CLAUDE_READING_SOURCE: &str = "claude.context_usage";

/// One launch of a harness process. The pair equals the `started_at` and
/// `provider_session_id` columns of the session row while the launch is
/// current.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Launch {
    pub started_at: String,
    pub provider_session_id: String,
}

/// A completed turn of a chat that can lead to a compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueMark {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub completed_at: DateTime<Utc>,
    /// `None` when the frame path did not know the launch.
    pub launch: Option<Launch>,
    pub harness: String,
}

/// How an attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A native boundary arrived while the attempt was open.
    Compacted,
    EndedWithoutBoundary,
    Failed,
    Unsupported,
    Interrupted,
    Unsettled,
}

impl Outcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compacted => "compacted",
            Self::EndedWithoutBoundary => "ended_without_boundary",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
            Self::Interrupted => "interrupted",
            Self::Unsettled => "unsettled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::Compacted,
            Self::EndedWithoutBoundary,
            Self::Failed,
            Self::Unsupported,
            Self::Interrupted,
            Self::Unsettled,
        ]
        .into_iter()
        .find(|outcome| outcome.as_str() == value)
    }

    /// These outcomes can repeat on the same process, so no other attempt
    /// runs on the launch.
    pub const fn stops_the_launch(self) -> bool {
        matches!(self, Self::Failed | Self::Unsupported | Self::Unsettled)
    }
}

/// A terminal signal for the open attempt of a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub session_id: String,
    pub attempt_id: String,
    pub outcome: Outcome,
    /// The Claude boundary report as given. Codex reports no figures.
    pub boundary_pre_tokens: Option<i64>,
    /// Not on the base of the readings: compare the trigger reading with the
    /// next reading instead.
    pub boundary_post_tokens: Option<i64>,
    /// The active turn at the boundary. The next reading never has this turn
    /// id. `None` on Claude.
    pub compaction_turn_id: Option<String>,
}

/// What the frame path hands over after its lock. It builds the receipt from
/// values that it holds already, so the receipt adds no SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receipt {
    Due(DueMark),
    Settle(Settlement),
}

/// An attempt from its insert to its terminal signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAttempt {
    pub attempt_id: String,
    pub launch: Launch,
}

/// Why no other attempt runs on a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStop {
    /// An attempt ended as failed, unsupported or unsettled.
    Failed,
    /// The next reading of a compacted attempt was at or above
    /// [`THRESHOLD_TOKENS`].
    Ineffective,
}

/// The inputs that need only memory. The sweep checks them before it takes
/// a lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInputs {
    /// The switch in memory.
    pub enabled: bool,
    pub mark: DueMark,
    pub now: DateTime<Utc>,
    pub attempt_open: bool,
    /// The stop of the launch of the mark, if any.
    pub launch_stop: Option<LaunchStop>,
}

/// The columns of the session row that the rule reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub kind: String,
    pub parent_session_id: Option<String>,
    pub depth: i64,
    pub harness: String,
    pub model: Option<String>,
    pub status: String,
    pub active_turn_id: Option<String>,
    pub started_at: Option<String>,
    pub provider_session_id: Option<String>,
}

/// The answer of one state query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Probe {
    #[default]
    Clear,
    Present,
    /// The query failed or hit its cap.
    Unknown,
}

/// The activity that holds a compaction back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatState {
    /// An open `needs_you` attention row, or the status `waiting`.
    pub approval: Probe,
    /// A started tool with no completion after the last `turn.started`.
    pub tool: Probe,
    /// A worker result or delivery that is not done, a live harness
    /// subagent, or a background task in progress.
    pub children: Probe,
    /// Queued input in the state `queued` or `claiming`.
    pub queue: Probe,
}

impl ChatState {
    fn any_unknown(self) -> bool {
        [self.approval, self.tool, self.children, self.queue].contains(&Probe::Unknown)
    }
}

/// The latest `context_readings` row of a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub id: i64,
    pub turn_id: Option<String>,
    pub harness: String,
    pub model: Option<String>,
    pub provider_session_id: Option<String>,
    pub used_tokens: i64,
    pub window_tokens: i64,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// The state that the sweep reads under its reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The switch row, read again, so a stale switch in memory cannot fire.
    pub enabled: bool,
    pub opted_out: bool,
    pub session: SessionRow,
    pub native_compaction: NativeCompaction,
    /// The adapter map has a runtime for the chat. Bridge never starts a
    /// process to compact.
    pub has_runtime: bool,
    pub state: ChatState,
    pub reading: Option<Reading>,
}

/// Each input of [`rule::decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    pub memory: MemoryInputs,
    pub snapshot: Snapshot,
}

fn harness_allowed(harness: &str) -> bool {
    HARNESSES.contains(&harness)
}

fn span(limit: Duration) -> TimeDelta {
    TimeDelta::from_std(limit).unwrap_or(TimeDelta::MAX)
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn launch_is_current(mark: &DueMark, session: &SessionRow) -> bool {
    mark.launch.as_ref().is_some_and(|launch| {
        session.started_at.as_deref() == Some(launch.started_at.as_str())
            && session.provider_session_id.as_deref() == Some(launch.provider_session_id.as_str())
    })
}

/// The pure rule. The order of [`rule::ORDER`] is the order of the checks,
/// and the first match wins.
pub mod rule {
    use super::{
        harness_allowed, launch_is_current, parse_time, span, DueMark, Inputs, LaunchStop,
        MemoryInputs, Probe, Reading, SessionRow, CLAUDE_READING_SOURCE, DIRECT_KIND,
        IDLE_DELAY, IDLE_STATUSES, THRESHOLD_TOKENS, WARM_LIMIT,
    };

    /// Why the rule did not fire.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Suppression {
        Disabled,
        OptedOut,
        HarnessNotAllowed,
        NotDirect,
        HasParent,
        Unsupported,
        NoRuntime,
        LaunchMismatch,
        LaunchAttemptFailed,
        LaunchAttemptIneffective,
        AttemptOpen,
        TooSoon,
        CacheCold,
        TurnActive,
        ApprovalOpen,
        ToolOpen,
        ChildrenLive,
        QueuedInput,
        StateUnknown,
        NoReading,
        ReadingMismatch,
        BelowThreshold,
    }

    /// The suppressions in the order of the checks.
    pub const ORDER: [Suppression; 22] = [
        Suppression::Disabled,
        Suppression::OptedOut,
        Suppression::HarnessNotAllowed,
        Suppression::NotDirect,
        Suppression::HasParent,
        Suppression::Unsupported,
        Suppression::NoRuntime,
        Suppression::LaunchMismatch,
        Suppression::LaunchAttemptFailed,
        Suppression::LaunchAttemptIneffective,
        Suppression::AttemptOpen,
        Suppression::TooSoon,
        Suppression::CacheCold,
        Suppression::TurnActive,
        Suppression::ApprovalOpen,
        Suppression::ToolOpen,
        Suppression::ChildrenLive,
        Suppression::QueuedInput,
        Suppression::StateUnknown,
        Suppression::NoReading,
        Suppression::ReadingMismatch,
        Suppression::BelowThreshold,
    ];

    /// What a decision does to the due mark.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MarkEffect {
        /// The mark stays until `WARM_LIMIT`.
        Keep,
        /// The mark goes.
        Drop,
    }

    impl Suppression {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Disabled => "disabled",
                Self::OptedOut => "opted_out",
                Self::HarnessNotAllowed => "harness_not_allowed",
                Self::NotDirect => "not_direct",
                Self::HasParent => "has_parent",
                Self::Unsupported => "unsupported",
                Self::NoRuntime => "no_runtime",
                Self::LaunchMismatch => "launch_mismatch",
                Self::LaunchAttemptFailed => "launch_attempt_failed",
                Self::LaunchAttemptIneffective => "launch_attempt_ineffective",
                Self::AttemptOpen => "attempt_open",
                Self::TooSoon => "too_soon",
                Self::CacheCold => "cache_cold",
                Self::TurnActive => "turn_active",
                Self::ApprovalOpen => "approval_open",
                Self::ToolOpen => "tool_open",
                Self::ChildrenLive => "children_live",
                Self::QueuedInput => "queued_input",
                Self::StateUnknown => "state_unknown",
                Self::NoReading => "no_reading",
                Self::ReadingMismatch => "reading_mismatch",
                Self::BelowThreshold => "below_threshold",
            }
        }

        /// A suppression that can clear while the mark is warm keeps it.
        pub const fn mark(self) -> MarkEffect {
            match self {
                Self::AttemptOpen
                | Self::TooSoon
                | Self::TurnActive
                | Self::ApprovalOpen
                | Self::ToolOpen
                | Self::ChildrenLive
                | Self::QueuedInput
                | Self::StateUnknown
                | Self::NoReading => MarkEffect::Keep,
                Self::Disabled
                | Self::OptedOut
                | Self::HarnessNotAllowed
                | Self::NotDirect
                | Self::HasParent
                | Self::Unsupported
                | Self::NoRuntime
                | Self::LaunchMismatch
                | Self::LaunchAttemptFailed
                | Self::LaunchAttemptIneffective
                | Self::CacheCold
                | Self::ReadingMismatch
                | Self::BelowThreshold => MarkEffect::Drop,
            }
        }
    }

    /// Why the rule fired. The attempt row keeps it in `reason`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FireReason {
        WindowOverThreshold,
    }

    impl FireReason {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::WindowOverThreshold => "window_over_threshold",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Decision {
        Fire(FireReason),
        Suppress(Suppression),
    }

    impl Decision {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Fire(reason) => reason.as_str(),
                Self::Suppress(suppression) => suppression.as_str(),
            }
        }

        /// A fire removes the mark: a new mark needs a new completed turn.
        pub const fn mark(self) -> MarkEffect {
            match self {
                Self::Fire(_) => MarkEffect::Drop,
                Self::Suppress(suppression) => suppression.mark(),
            }
        }
    }

    /// How the latest reading relates to the mark.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Binding {
        /// The reading is the window after the turn of the mark.
        Bound,
        /// No reading of this launch came after the turn end yet.
        NoReading,
        /// The reading belongs to another launch, model, harness or turn.
        Mismatch,
    }

    /// The suppressions that need only memory, in the order of the checks.
    pub fn precheck(memory: &MemoryInputs) -> Option<Suppression> {
        ORDER
            .into_iter()
            .find(|suppression| memory_holds(*suppression, memory))
    }

    /// Fire or suppress. The precheck answer stands unless a suppression of
    /// the snapshot comes before it in the order.
    pub fn decide(inputs: &Inputs) -> Decision {
        let from_memory = precheck(&inputs.memory);
        let first = ORDER
            .into_iter()
            .take_while(|suppression| Some(*suppression) != from_memory)
            .find(|suppression| snapshot_holds(*suppression, inputs))
            .or(from_memory);
        match first {
            Some(suppression) => Decision::Suppress(suppression),
            None => Decision::Fire(FireReason::WindowOverThreshold),
        }
    }

    /// Bind the latest reading to the mark. Its thread, harness and model
    /// equal the current values of the session and of the mark. It is not
    /// older than the launch, and its turn matches. Codex matches by turn
    /// id. A Claude reading has no turn id, so it matches by its source and
    /// a time at or after the turn end.
    pub fn bind(mark: &DueMark, session: &SessionRow, reading: Option<&Reading>) -> Binding {
        let Some(reading) = reading else {
            return Binding::NoReading;
        };
        let Some(launch) = mark.launch.as_ref() else {
            return Binding::Mismatch;
        };
        let same_window = reading.provider_session_id.as_deref()
            == Some(launch.provider_session_id.as_str())
            && reading.provider_session_id == session.provider_session_id
            && reading.harness == mark.harness
            && reading.harness == session.harness
            && reading.model == session.model;
        let in_launch =
            parse_time(&launch.started_at).is_some_and(|started| reading.created_at >= started);
        if !same_window || !in_launch {
            return Binding::Mismatch;
        }
        let after_turn_end = reading.created_at >= mark.completed_at;
        let turn_matches = match mark.harness.as_str() {
            "codex" => reading.turn_id.is_some() && reading.turn_id == mark.turn_id,
            "claude" => {
                reading.source == CLAUDE_READING_SOURCE
                    && reading.turn_id.is_none()
                    && after_turn_end
            }
            _ => false,
        };
        if turn_matches {
            Binding::Bound
        } else if after_turn_end {
            Binding::Mismatch
        } else {
            Binding::NoReading
        }
    }

    fn memory_holds(suppression: Suppression, memory: &MemoryInputs) -> bool {
        let age = memory.now.signed_duration_since(memory.mark.completed_at);
        match suppression {
            Suppression::Disabled => !memory.enabled,
            Suppression::HarnessNotAllowed => !harness_allowed(&memory.mark.harness),
            Suppression::LaunchMismatch => memory.mark.launch.is_none(),
            Suppression::LaunchAttemptFailed => memory.launch_stop == Some(LaunchStop::Failed),
            Suppression::LaunchAttemptIneffective => {
                memory.launch_stop == Some(LaunchStop::Ineffective)
            }
            Suppression::AttemptOpen => memory.attempt_open,
            Suppression::TooSoon => age < span(IDLE_DELAY),
            Suppression::CacheCold => age > span(WARM_LIMIT),
            Suppression::OptedOut
            | Suppression::NotDirect
            | Suppression::HasParent
            | Suppression::Unsupported
            | Suppression::NoRuntime
            | Suppression::TurnActive
            | Suppression::ApprovalOpen
            | Suppression::ToolOpen
            | Suppression::ChildrenLive
            | Suppression::QueuedInput
            | Suppression::StateUnknown
            | Suppression::NoReading
            | Suppression::ReadingMismatch
            | Suppression::BelowThreshold => false,
        }
    }

    fn snapshot_holds(suppression: Suppression, inputs: &Inputs) -> bool {
        let snapshot = &inputs.snapshot;
        let session = &snapshot.session;
        let mark = &inputs.memory.mark;
        let binding = || bind(mark, session, snapshot.reading.as_ref());
        match suppression {
            Suppression::Disabled => !snapshot.enabled,
            Suppression::OptedOut => snapshot.opted_out,
            Suppression::HarnessNotAllowed => !harness_allowed(&session.harness),
            Suppression::NotDirect => session.kind != DIRECT_KIND,
            Suppression::HasParent => session.parent_session_id.is_some() || session.depth > 0,
            Suppression::Unsupported => !snapshot.native_compaction.is_supported(),
            Suppression::NoRuntime => !snapshot.has_runtime,
            Suppression::LaunchMismatch => !launch_is_current(mark, session),
            Suppression::TurnActive => {
                !IDLE_STATUSES.contains(&session.status.as_str()) || session.active_turn_id.is_some()
            }
            Suppression::ApprovalOpen => snapshot.state.approval == Probe::Present,
            Suppression::ToolOpen => snapshot.state.tool == Probe::Present,
            Suppression::ChildrenLive => snapshot.state.children == Probe::Present,
            Suppression::QueuedInput => snapshot.state.queue == Probe::Present,
            Suppression::StateUnknown => snapshot.state.any_unknown(),
            Suppression::NoReading => binding() == Binding::NoReading,
            Suppression::ReadingMismatch => binding() == Binding::Mismatch,
            Suppression::BelowThreshold => {
                binding() == Binding::Bound
                    && snapshot
                        .reading
                        .as_ref()
                        .is_some_and(|reading| reading.used_tokens < THRESHOLD_TOKENS)
            }
            Suppression::LaunchAttemptFailed
            | Suppression::LaunchAttemptIneffective
            | Suppression::AttemptOpen
            | Suppression::TooSoon
            | Suppression::CacheCold => false,
        }
    }
}

/// The memory of the trigger. Each method holds the mutex for one map
/// operation and takes no other lock. So the frame path and the sweep can
/// call it under their own locks.
#[derive(Debug, Default)]
pub struct Tracker {
    enabled: AtomicBool,
    memory: Mutex<Memory>,
}

#[derive(Debug, Default)]
struct Memory {
    due: HashMap<String, DueMark>,
    /// Chats with a native boundary after their last turn end.
    compacted: HashSet<String>,
    open: HashMap<String, OpenAttempt>,
    /// One entry for each chat, because a chat has one current launch.
    stopped: HashMap<String, (Launch, LaunchStop)>,
}

impl Tracker {
    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// A due mark replaces the mark of the same chat. A settlement closes
    /// the open attempt that it names.
    pub fn accept(&self, receipt: Receipt) {
        match receipt {
            Receipt::Due(mark) => {
                self.memory().due.insert(mark.session_id.clone(), mark);
            }
            Receipt::Settle(settlement) => {
                self.settle(
                    &settlement.session_id,
                    &settlement.attempt_id,
                    settlement.outcome,
                );
            }
        }
    }

    /// The frame path calls this when it handles a native boundary of the
    /// chat. Then the next turn end of the chat makes no due mark. A mark
    /// from before the boundary no longer tells the size of the window, so
    /// it goes too.
    pub fn note_boundary(&self, session_id: &str) {
        let mut memory = self.memory();
        memory.compacted.insert(session_id.to_owned());
        memory.due.remove(session_id);
    }

    /// The frame path calls this for each completed turn of the chat. True
    /// means that the turn closed a compaction, so it gives no due mark. The
    /// flag holds no turn id: Claude sends no turn start for a compaction,
    /// and its turn end has no turn id.
    pub fn take_compaction_end(&self, session_id: &str) -> bool {
        self.memory().compacted.remove(session_id)
    }

    pub fn due_mark(&self, session_id: &str) -> Option<DueMark> {
        self.memory().due.get(session_id).cloned()
    }

    /// The inputs of [`rule::precheck`] for the mark of the chat, or `None`
    /// when the chat has no mark.
    pub fn memory_inputs(&self, session_id: &str, now: DateTime<Utc>) -> Option<MemoryInputs> {
        let enabled = self.enabled();
        let memory = self.memory();
        let mark = memory.due.get(session_id)?.clone();
        let launch_stop = memory
            .stopped
            .get(session_id)
            .filter(|(launch, _)| mark.launch.as_ref() == Some(launch))
            .map(|(_, stop)| *stop);
        Some(MemoryInputs {
            enabled,
            attempt_open: memory.open.contains_key(session_id),
            launch_stop,
            mark,
            now,
        })
    }

    /// Apply a decision to the mark that it belongs to. A newer mark of the
    /// chat stays. Returns whether the mark went.
    pub fn apply(&self, mark: &DueMark, decision: rule::Decision) -> bool {
        if decision.mark() == rule::MarkEffect::Keep {
            return false;
        }
        let mut memory = self.memory();
        if memory.due.get(&mark.session_id) != Some(mark) {
            return false;
        }
        memory.due.remove(&mark.session_id);
        true
    }

    /// The sweep calls this after it writes the attempt row and before it
    /// dispatches, so a boundary that comes first finds the attempt.
    pub fn open(&self, session_id: &str, attempt: OpenAttempt) {
        self.memory().open.insert(session_id.to_owned(), attempt);
    }

    pub fn open_attempt(&self, session_id: &str) -> Option<OpenAttempt> {
        self.memory().open.get(session_id).cloned()
    }

    /// Close the open attempt of the chat when its id matches. An outcome
    /// that can repeat stops the launch of the attempt. Returns the closed
    /// attempt.
    pub fn settle(&self, session_id: &str, attempt_id: &str, outcome: Outcome) -> Option<OpenAttempt> {
        let mut memory = self.memory();
        if memory
            .open
            .get(session_id)
            .is_none_or(|open| open.attempt_id != attempt_id)
        {
            return None;
        }
        let open = memory.open.remove(session_id)?;
        if outcome.stops_the_launch() {
            memory.stopped.insert(
                session_id.to_owned(),
                (open.launch.clone(), LaunchStop::Failed),
            );
        }
        Some(open)
    }

    /// The settle step calls this with the next reading of a compacted
    /// attempt. A window still at or above the threshold stops the launch.
    /// Without this stop, a chat with a large fixed part compacts after each
    /// turn.
    pub fn record_next_reading(&self, session_id: &str, launch: &Launch, used_tokens: i64) {
        if used_tokens >= THRESHOLD_TOKENS {
            self.memory().stopped.insert(
                session_id.to_owned(),
                (launch.clone(), LaunchStop::Ineffective),
            );
        }
    }
}

#[cfg(test)]
mod tests;

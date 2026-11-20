//! Evals for early native compaction.
//!
//! `offline` needs no model and runs with the suite. Each case of
//! `testing/evals/early-compaction/rule-cases.json` is a list of steps for one
//! chat. The steps are turn starts and ends, receipts, native boundaries,
//! readings, state changes, relaunches, settlements, clock steps and sweeps.
//! Each sweep names the decision that it must give: `fire`, a suppression, or
//! `none` when the chat has no due mark.
//!
//! [`World`] plays the frame path and the sweep with the real `Tracker` and
//! the real `rule`. It keeps the session row, the state and the latest reading
//! in memory, so it needs no database. It makes no claim about tokens.

use std::collections::BTreeSet;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;

use crate::adapters::NativeCompaction;
use crate::early_compaction::rule::{self, Decision, MarkEffect, ORDER};
use crate::early_compaction::{
    ChatState, DueMark, Inputs, Launch, OpenAttempt, Outcome, Probe, Reading, Receipt,
    SessionRow, Settlement, Snapshot, Tracker,
};

const CASES: &str = include_str!("../../../testing/evals/early-compaction/rule-cases.json");
const SESSION: &str = "chat-1";
/// The label of a sweep that fires.
const FIRE: &str = "fire";
/// The label of a sweep that finds no due mark.
const NO_MARK: &str = "none";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cases {
    start: DateTime<Utc>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    name: String,
    harness: String,
    #[serde(default = "direct")]
    kind: String,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    depth: i64,
    steps: Vec<Step>,
}

fn direct() -> String {
    "direct".to_owned()
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Step {
    TurnStart(String),
    TurnEnd(TurnEnd),
    /// A due mark that the frame path built in its lock before the sweep
    /// opened an attempt, and that `accept` stores after the lock.
    Receipt(String),
    Boundary,
    Reading(ReadingStep),
    State(StateStep),
    Relaunch(RelaunchStep),
    Settle(String),
    /// Seconds.
    Advance(i64),
    Sweep(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TurnEnd {
    /// False for a turn that the frame path never marks: a failed turn, a
    /// provider error, a prompt-control turn or a checkpoint turn.
    #[serde(default = "yes")]
    marks: bool,
    #[serde(default)]
    no_launch: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadingStep {
    used: i64,
    /// Absent: the active or last turn on Codex, null on Claude.
    #[serde(default)]
    turn: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    thread: Option<String>,
    #[serde(default)]
    source: Option<String>,
    /// The next reading of the last compacted attempt.
    #[serde(default)]
    next: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateStep {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    switch: Option<bool>,
    #[serde(default)]
    opted_out: Option<bool>,
    #[serde(default)]
    runtime: Option<bool>,
    #[serde(default)]
    support: Option<Support>,
    #[serde(default)]
    approval: Option<ProbeName>,
    #[serde(default)]
    tool: Option<ProbeName>,
    #[serde(default)]
    children: Option<ProbeName>,
    #[serde(default)]
    queue: Option<ProbeName>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RelaunchStep {
    /// Absent: native resume, which keeps the thread id.
    #[serde(default)]
    thread: Option<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum Support {
    Unsupported,
    WithFocus,
    WholeConversation,
}

impl From<Support> for NativeCompaction {
    fn from(support: Support) -> Self {
        match support {
            Support::Unsupported => Self::Unsupported,
            Support::WithFocus => Self::WithFocus,
            Support::WholeConversation => Self::WholeConversation,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum ProbeName {
    Clear,
    Present,
    Unknown,
}

impl From<ProbeName> for Probe {
    fn from(name: ProbeName) -> Self {
        match name {
            ProbeName::Clear => Self::Clear,
            ProbeName::Present => Self::Present,
            ProbeName::Unknown => Self::Unknown,
        }
    }
}

/// One chat as the frame path and the sweep see it.
struct World {
    tracker: Tracker,
    now: DateTime<Utc>,
    session: SessionRow,
    launch: Launch,
    switch_row: bool,
    opted_out: bool,
    has_runtime: bool,
    native: NativeCompaction,
    state: ChatState,
    reading: Option<Reading>,
    readings: i64,
    last_turn: Option<String>,
    attempts: usize,
    compacted_launch: Option<Launch>,
}

impl World {
    fn new(case: &Case, start: DateTime<Utc>) -> Self {
        let launch = Launch {
            started_at: (start - TimeDelta::hours(1)).to_rfc3339(),
            provider_session_id: "thread-1".to_owned(),
        };
        let (model, native) = match case.harness.as_str() {
            "claude" => ("claude-haiku-5-5", NativeCompaction::WithFocus),
            "codex" => ("gpt-6.2-sol", NativeCompaction::WholeConversation),
            _ => ("opencode-model", NativeCompaction::Unsupported),
        };
        let tracker = Tracker::default();
        tracker.set_enabled(true);
        Self {
            tracker,
            now: start,
            session: SessionRow {
                kind: case.kind.clone(),
                parent_session_id: case.parent.clone(),
                depth: case.depth,
                harness: case.harness.clone(),
                model: Some(model.to_owned()),
                status: "idle".to_owned(),
                active_turn_id: None,
                started_at: Some(launch.started_at.clone()),
                provider_session_id: Some(launch.provider_session_id.clone()),
            },
            launch,
            switch_row: true,
            opted_out: false,
            has_runtime: true,
            native,
            state: ChatState::default(),
            reading: None,
            readings: 0,
            last_turn: None,
            attempts: 0,
            compacted_launch: None,
        }
    }

    fn claude(&self) -> bool {
        self.session.harness == "claude"
    }

    fn apply(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::TurnStart(turn) => {
                self.session.status = "working".to_owned();
                self.session.active_turn_id = Some(turn.clone());
                self.last_turn = Some(turn.clone());
            }
            Step::TurnEnd(end) => self.turn_end(end),
            Step::Receipt(turn) => self.tracker.accept(Receipt::Due(self.mark(Some(turn.clone()), true))),
            Step::Boundary => self.boundary(),
            Step::Reading(reading) => self.read(reading)?,
            Step::State(state) => self.change(state),
            Step::Relaunch(relaunch) => self.relaunch(relaunch),
            Step::Settle(outcome) => self.settle(outcome)?,
            Step::Advance(seconds) => self.now += TimeDelta::seconds(*seconds),
            Step::Sweep(expected) => self.sweep(expected)?,
        }
        Ok(())
    }

    fn mark(&self, turn_id: Option<String>, with_launch: bool) -> DueMark {
        DueMark {
            session_id: SESSION.to_owned(),
            turn_id,
            completed_at: self.now,
            launch: with_launch.then(|| self.launch.clone()),
            harness: self.session.harness.clone(),
        }
    }

    /// The frame path: no mark with the switch off, for the turn end of a
    /// compaction, or for the turn of an open attempt. The flag clears on
    /// each completed turn, also on a failed one.
    fn turn_end(&mut self, end: &TurnEnd) {
        let turn_id = self.session.active_turn_id.take();
        self.session.status = "ready".to_owned();
        let compaction_end = self.tracker.take_compaction_end(SESSION);
        let attempt_open = self.tracker.open_attempt(SESSION).is_some();
        if end.marks && self.tracker.enabled() && !compaction_end && !attempt_open {
            self.tracker.accept(Receipt::Due(self.mark(turn_id, !end.no_launch)));
        }
    }

    /// The frame path: a boundary sets the flag and settles the open attempt.
    fn boundary(&mut self) {
        self.tracker.note_boundary(SESSION);
        if let Some(open) = self.tracker.open_attempt(SESSION) {
            self.tracker.accept(Receipt::Settle(settlement(&open, Outcome::Compacted)));
            self.compacted_launch = Some(open.launch);
        }
    }

    fn read(&mut self, step: &ReadingStep) -> Result<(), String> {
        let default_turn = if self.claude() {
            None
        } else {
            self.session.active_turn_id.clone().or_else(|| self.last_turn.clone())
        };
        let default_source = if self.claude() { "claude.context_usage" } else { "codex.token_usage" };
        self.readings += 1;
        self.reading = Some(Reading {
            id: self.readings,
            turn_id: step.turn.clone().or(default_turn),
            harness: self.session.harness.clone(),
            model: step.model.clone().or_else(|| self.session.model.clone()),
            provider_session_id: Some(step.thread.clone().unwrap_or_else(|| self.launch.provider_session_id.clone())),
            used_tokens: step.used,
            window_tokens: 200_000,
            source: step.source.clone().unwrap_or_else(|| default_source.to_owned()),
            created_at: self.now,
        });
        if step.next {
            let launch = self.compacted_launch.as_ref().ok_or("a next reading with no compacted attempt")?;
            self.tracker.record_next_reading(SESSION, launch, step.used);
        }
        Ok(())
    }

    fn change(&mut self, step: &StateStep) {
        if let Some(status) = &step.status {
            self.session.status = status.clone();
        }
        if let Some(model) = &step.model {
            self.session.model = Some(model.clone());
        }
        if let Some(enabled) = step.switch {
            self.switch_row = enabled;
            self.tracker.set_enabled(enabled);
        }
        self.opted_out = step.opted_out.unwrap_or(self.opted_out);
        self.has_runtime = step.runtime.unwrap_or(self.has_runtime);
        self.native = step.support.map_or(self.native, NativeCompaction::from);
        let state = &mut self.state;
        state.approval = step.approval.map_or(state.approval, Probe::from);
        state.tool = step.tool.map_or(state.tool, Probe::from);
        state.children = step.children.map_or(state.children, Probe::from);
        state.queue = step.queue.map_or(state.queue, Probe::from);
    }

    fn relaunch(&mut self, step: &RelaunchStep) {
        self.launch = Launch {
            started_at: self.now.to_rfc3339(),
            provider_session_id: step.thread.clone().unwrap_or_else(|| self.launch.provider_session_id.clone()),
        };
        self.session.started_at = Some(self.launch.started_at.clone());
        self.session.provider_session_id = Some(self.launch.provider_session_id.clone());
        self.has_runtime = true;
    }

    fn settle(&mut self, name: &str) -> Result<(), String> {
        let outcome = Outcome::parse(name).ok_or_else(|| format!("unknown outcome {name}"))?;
        let open = self.tracker.open_attempt(SESSION).ok_or("no open attempt to settle")?;
        self.tracker.accept(Receipt::Settle(settlement(&open, outcome)));
        if outcome == Outcome::Compacted {
            self.compacted_launch = Some(open.launch);
        }
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            enabled: self.switch_row,
            opted_out: self.opted_out,
            session: self.session.clone(),
            native_compaction: self.native,
            has_runtime: self.has_runtime,
            state: self.state,
            reading: self.reading.clone(),
        }
    }

    /// The sweep: the memory checks first, the snapshot only when they pass.
    /// A fire opens an attempt and the dispatch sets the status `working`.
    fn sweep(&mut self, expected: &str) -> Result<(), String> {
        let Some(memory) = self.tracker.memory_inputs(SESSION, self.now) else {
            return same(expected, NO_MARK);
        };
        let mark = memory.mark.clone();
        let decision = match rule::precheck(&memory) {
            Some(suppression) => Decision::Suppress(suppression),
            None => rule::decide(&Inputs { memory, snapshot: self.snapshot() }),
        };
        let dropped = self.tracker.apply(&mark, decision);
        if dropped != (decision.mark() == MarkEffect::Drop) {
            return Err(format!("{} left the mark in the wrong state", decision.as_str()));
        }
        let label = match decision {
            Decision::Fire(_) => FIRE,
            Decision::Suppress(suppression) => return same(expected, suppression.as_str()),
        };
        self.attempts += 1;
        let launch = mark.launch.ok_or("a fire for a mark with no launch")?;
        let attempt_id = format!("attempt-{}", self.attempts);
        self.tracker.open(SESSION, OpenAttempt { attempt_id, launch });
        self.session.status = "working".to_owned();
        same(expected, label)
    }
}

fn settlement(open: &OpenAttempt, outcome: Outcome) -> Settlement {
    Settlement {
        session_id: SESSION.to_owned(),
        attempt_id: open.attempt_id.clone(),
        outcome,
        boundary_pre_tokens: None,
        boundary_post_tokens: None,
        compaction_turn_id: None,
    }
}

fn same(expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!("expected {expected}, got {actual}"))
    }
}

fn run(case: &Case, start: DateTime<Utc>) -> Result<(), String> {
    let mut world = World::new(case, start);
    for (index, step) in case.steps.iter().enumerate() {
        world
            .apply(step)
            .map_err(|error| format!("{}: step {index}: {error}", case.name))?;
    }
    Ok(())
}

#[test]
fn offline() -> Result<(), String> {
    let corpus: Cases = serde_json::from_str(CASES).map_err(|error| error.to_string())?;
    let mut failures: Vec<String> = corpus
        .cases
        .iter()
        .filter_map(|case| run(case, corpus.start).err())
        .collect();
    let expected: BTreeSet<&str> = corpus
        .cases
        .iter()
        .flat_map(|case| &case.steps)
        .filter_map(|step| match step {
            Step::Sweep(label) => Some(label.as_str()),
            _ => None,
        })
        .collect();
    let labels = ORDER
        .iter()
        .map(|suppression| suppression.as_str())
        .chain([FIRE, NO_MARK]);
    failures.extend(
        labels
            .filter(|label| !expected.contains(label))
            .map(|label| format!("no sweep expects {label}")),
    );
    println!("early compaction offline: {} cases", corpus.cases.len());
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

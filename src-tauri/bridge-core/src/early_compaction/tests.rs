use chrono::{DateTime, TimeDelta, Utc};

use super::rule::{self, Binding, Decision, FireReason, MarkEffect, Suppression, ORDER};
use super::*;

const CLAUDE_MODEL: &str = "claude-haiku-5-5";
const CODEX_MODEL: &str = "gpt-6.2-sol";
const BOTH: [&str; 2] = ["claude", "codex"];
const FIRE: Decision = Decision::Fire(FireReason::WindowOverThreshold);
/// 2026-11-20T10:00:00Z, the turn end of each mark in these tests.
const TURN_END_SECONDS: i64 = 1_795_168_800;

fn at_ms(milliseconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
        + TimeDelta::seconds(TURN_END_SECONDS)
        + TimeDelta::milliseconds(milliseconds)
}

fn at(seconds: i64) -> DateTime<Utc> {
    at_ms(seconds * 1_000)
}

fn model_of(harness: &str) -> Option<String> {
    Some(match harness {
        "claude" => CLAUDE_MODEL.to_owned(),
        "codex" => CODEX_MODEL.to_owned(),
        other => format!("{other}-model"),
    })
}

fn native(harness: &str) -> NativeCompaction {
    match harness {
        "claude" => NativeCompaction::WithFocus,
        "codex" => NativeCompaction::WholeConversation,
        _ => NativeCompaction::Unsupported,
    }
}

fn launch() -> Launch {
    Launch {
        started_at: at(-3_600).to_rfc3339(),
        provider_session_id: "thread-1".to_owned(),
    }
}

fn mark(harness: &str) -> DueMark {
    DueMark {
        session_id: "chat-1".to_owned(),
        turn_id: Some("turn-1".to_owned()),
        completed_at: at(0),
        launch: Some(launch()),
        harness: harness.to_owned(),
        generation: 0,
    }
}

fn session(harness: &str) -> SessionRow {
    let launch = launch();
    SessionRow {
        kind: "direct".to_owned(),
        parent_session_id: None,
        depth: 0,
        harness: harness.to_owned(),
        model: model_of(harness),
        status: "ready".to_owned(),
        active_turn_id: None,
        started_at: Some(launch.started_at),
        provider_session_id: Some(launch.provider_session_id),
    }
}

/// The reading of the turn of [`mark`]: Codex writes it before the turn end
/// with the turn id, Claude writes it after the turn end with no turn id.
fn reading(harness: &str, used_tokens: i64) -> Reading {
    let claude = harness == "claude";
    Reading {
        id: 7,
        turn_id: (!claude).then(|| "turn-1".to_owned()),
        harness: harness.to_owned(),
        model: model_of(harness),
        provider_session_id: Some("thread-1".to_owned()),
        used_tokens,
        window_tokens: 200_000,
        source: if claude { "claude.context_usage" } else { "codex.token_usage" }.to_owned(),
        created_at: if claude { at(1) } else { at(-1) },
    }
}

fn snapshot(harness: &str) -> Snapshot {
    Snapshot {
        enabled: true,
        opted_out: false,
        session: session(harness),
        native_compaction: native(harness),
        has_runtime: true,
        state: ChatState::default(),
        reading: Some(reading(harness, 150_000)),
    }
}

/// Inputs that fire: 30 s after the turn end, 150,000 tokens.
fn firing(harness: &str) -> Inputs {
    Inputs {
        memory: MemoryInputs {
            enabled: true,
            mark: mark(harness),
            now: at(30),
            attempt_open: false,
            launch_stop: None,
        },
        snapshot: snapshot(harness),
    }
}

fn decide_with(harness: &str, memory: MemoryInputs) -> Decision {
    rule::decide(&Inputs { memory, snapshot: snapshot(harness) })
}

fn assert_suppressed(harness: &str, expected: Suppression, change: impl Fn(&mut Inputs)) {
    let mut inputs = firing(harness);
    change(&mut inputs);
    assert_eq!(rule::decide(&inputs), Decision::Suppress(expected), "{harness}");
}

fn assert_fires(harness: &str, change: impl Fn(&mut Inputs)) {
    let mut inputs = firing(harness);
    change(&mut inputs);
    assert_eq!(rule::decide(&inputs), FIRE, "{harness}");
}

/// A Tracker that holds the mark of `harness` with the switch on.
fn tracker_with_mark(harness: &str) -> Tracker {
    let tracker = Tracker::default();
    tracker.set_enabled(true);
    tracker.accept(Receipt::Due(mark(harness)));
    tracker
}

/// Change the inputs so that this one suppression holds. No change makes a
/// suppression hold that comes earlier in the order.
fn make_hold(suppression: Suppression, inputs: &mut Inputs) {
    let memory = &mut inputs.memory;
    let snapshot = &mut inputs.snapshot;
    match suppression {
        Suppression::Disabled => snapshot.enabled = false,
        Suppression::OptedOut => snapshot.opted_out = true,
        Suppression::HarnessNotAllowed => {
            memory.mark.harness = "opencode".to_owned();
            snapshot.session.harness = "opencode".to_owned();
        }
        Suppression::NotDirect => snapshot.session.kind = "orchestrator".to_owned(),
        Suppression::HasParent => {
            snapshot.session.parent_session_id = Some("parent-1".to_owned());
            snapshot.session.depth = 1;
        }
        Suppression::Unsupported => snapshot.native_compaction = NativeCompaction::Unsupported,
        Suppression::NoRuntime => snapshot.has_runtime = false,
        Suppression::LaunchMismatch => memory.mark.launch = None,
        Suppression::LaunchAttemptFailed => memory.launch_stop = Some(LaunchStop::Failed),
        Suppression::LaunchAttemptIneffective => memory.launch_stop = Some(LaunchStop::Ineffective),
        Suppression::AttemptOpen => memory.attempt_open = true,
        Suppression::TooSoon => memory.now = at(10),
        Suppression::CacheCold => memory.now = at(200),
        Suppression::TurnActive => {
            snapshot.session.status = "working".to_owned();
            snapshot.session.active_turn_id = Some("turn-2".to_owned());
        }
        Suppression::ApprovalOpen => snapshot.state.approval = Probe::Present,
        Suppression::ToolOpen => snapshot.state.tool = Probe::Present,
        Suppression::ChildrenLive => snapshot.state.children = Probe::Present,
        Suppression::QueuedInput => snapshot.state.queue = Probe::Present,
        Suppression::StateUnknown => snapshot.state.tool = Probe::Unknown,
        Suppression::NoReading => snapshot.reading = None,
        Suppression::ReadingMismatch => {
            if let Some(reading) = snapshot.reading.as_mut() {
                reading.model = Some("another-model".to_owned());
            }
        }
        Suppression::BelowThreshold => {
            if let Some(reading) = snapshot.reading.as_mut() {
                reading.used_tokens = THRESHOLD_TOKENS - 1;
            }
        }
    }
}

#[test]
fn suppresses_disabled() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::Disabled, |inputs| inputs.memory.enabled = false);
        // A stale switch in memory cannot fire: the sweep reads the row again.
        assert_suppressed(harness, Suppression::Disabled, |inputs| inputs.snapshot.enabled = false);
        let tracker = tracker_with_mark(harness);
        tracker.set_enabled(false);
        let memory = tracker.memory_inputs("chat-1", at(30));
        assert_eq!(memory.as_ref().and_then(rule::precheck), Some(Suppression::Disabled));
        assert_eq!(memory.map(|memory| decide_with(harness, memory)), Some(Decision::Suppress(Suppression::Disabled)));
    }
    assert!(!Tracker::default().enabled());
}

#[test]
fn suppresses_opted_out() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::OptedOut, |inputs| inputs.snapshot.opted_out = true);
    }
}

#[test]
fn suppresses_harness_not_allowed_for_opencode() {
    let mut inputs = firing("opencode");
    inputs.snapshot.native_compaction = NativeCompaction::WholeConversation;
    assert_eq!(rule::precheck(&inputs.memory), Some(Suppression::HarnessNotAllowed));
    assert_eq!(rule::decide(&inputs), Decision::Suppress(Suppression::HarnessNotAllowed));
    // The session row decides too, also when the mark names an allowed harness.
    assert_suppressed("claude", Suppression::HarnessNotAllowed, |inputs| {
        inputs.snapshot.session.harness = "opencode".to_owned();
    });
}

#[test]
fn suppresses_not_direct_for_an_orchestrator() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::NotDirect, |inputs| {
            inputs.snapshot.session.kind = "orchestrator".to_owned();
        });
    }
}

#[test]
fn suppresses_not_direct_for_a_worker() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::NotDirect, |inputs| {
            let session = &mut inputs.snapshot.session;
            session.kind = "orchestrator".to_owned();
            session.parent_session_id = Some("parent-1".to_owned());
            session.depth = 1;
        });
    }
}

#[test]
fn suppresses_not_direct_for_a_workflow_session() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::NotDirect, |inputs| {
            inputs.snapshot.session.kind = crate::workflow_run::WORKFLOW_SESSION_KIND.to_owned();
        });
    }
}

#[test]
fn suppresses_has_parent() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::HasParent, |inputs| {
            inputs.snapshot.session.parent_session_id = Some("parent-1".to_owned());
        });
        assert_suppressed(harness, Suppression::HasParent, |inputs| inputs.snapshot.session.depth = 1);
    }
}

#[test]
fn suppresses_unsupported() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::Unsupported, |inputs| {
            inputs.snapshot.native_compaction = NativeCompaction::Unsupported;
        });
    }
}

#[test]
fn suppresses_no_runtime() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::NoRuntime, |inputs| inputs.snapshot.has_runtime = false);
    }
}

#[test]
fn suppresses_launch_mismatch_when_the_mark_has_no_launch() {
    for harness in BOTH {
        let mut inputs = firing(harness);
        inputs.memory.mark.launch = None;
        assert_eq!(rule::precheck(&inputs.memory), Some(Suppression::LaunchMismatch));
        assert_eq!(rule::decide(&inputs), Decision::Suppress(Suppression::LaunchMismatch));
        let binding = rule::bind(&inputs.memory.mark, &inputs.snapshot.session, inputs.snapshot.reading.as_ref());
        assert_eq!(binding, Binding::Mismatch);
    }
}

#[test]
fn suppresses_launch_mismatch_after_a_relaunch() {
    for harness in BOTH {
        // Native resume keeps the thread, so only the start time changes.
        assert_suppressed(harness, Suppression::LaunchMismatch, |inputs| {
            inputs.snapshot.session.started_at = Some(at(5).to_rfc3339());
        });
        assert_suppressed(harness, Suppression::LaunchMismatch, |inputs| {
            inputs.snapshot.session.provider_session_id = Some("thread-2".to_owned());
        });
        assert_suppressed(harness, Suppression::LaunchMismatch, |inputs| {
            inputs.snapshot.session.started_at = None;
        });
    }
}

#[test]
fn suppresses_launch_attempt_failed() {
    let stops = [Outcome::Failed, Outcome::Unsupported, Outcome::Unsettled];
    let others = [Outcome::Compacted, Outcome::EndedWithoutBoundary, Outcome::Interrupted];
    for harness in BOTH {
        for outcome in stops.into_iter().chain(others) {
            let tracker = tracker_with_mark(harness);
            tracker.open("chat-1", OpenAttempt { attempt_id: "attempt-1".to_owned(), launch: launch() });
            assert!(tracker.apply(&mark(harness), FIRE));
            let settled = tracker.settle("chat-1", "attempt-1", outcome);
            assert_eq!(settled.map(|open| open.attempt_id), Some("attempt-1".to_owned()));
            assert_eq!(tracker.open_attempt("chat-1"), None);
            tracker.accept(Receipt::Due(mark(harness)));
            let expected = if outcome.stops_the_launch() {
                Decision::Suppress(Suppression::LaunchAttemptFailed)
            } else {
                FIRE
            };
            let memory = tracker.memory_inputs("chat-1", at(30));
            assert_eq!(memory.map(|memory| decide_with(harness, memory)), Some(expected), "{harness} {}", outcome.as_str());
        }
        // The stop holds for its launch only.
        let tracker = tracker_with_mark(harness);
        tracker.open("chat-1", OpenAttempt { attempt_id: "attempt-1".to_owned(), launch: launch() });
        tracker.accept(Receipt::Settle(Settlement {
            session_id: "chat-1".to_owned(),
            attempt_id: "attempt-1".to_owned(),
            outcome: Outcome::Failed,
            boundary_pre_tokens: None,
            boundary_post_tokens: None,
            compaction_turn_id: None,
            error: None,
        }));
        let mut relaunched = mark(harness);
        relaunched.launch = Some(Launch { started_at: at(-100).to_rfc3339(), provider_session_id: "thread-1".to_owned() });
        tracker.accept(Receipt::Due(relaunched));
        assert_eq!(tracker.memory_inputs("chat-1", at(30)).map(|memory| memory.launch_stop), Some(None));
    }
}

#[test]
fn suppresses_launch_attempt_ineffective() {
    for harness in BOTH {
        for (next_used, expected) in [
            (THRESHOLD_TOKENS, Decision::Suppress(Suppression::LaunchAttemptIneffective)),
            (THRESHOLD_TOKENS - 1, FIRE),
        ] {
            let tracker = tracker_with_mark(harness);
            tracker.open("chat-1", OpenAttempt { attempt_id: "attempt-1".to_owned(), launch: launch() });
            assert!(tracker.settle("chat-1", "attempt-1", Outcome::Compacted).is_some());
            tracker.record_next_reading("chat-1", &launch(), next_used);
            let memory = tracker.memory_inputs("chat-1", at(30));
            assert_eq!(memory.map(|memory| decide_with(harness, memory)), Some(expected), "{harness} {next_used}");
        }
        // A failed attempt on the launch comes first in the order.
        assert_suppressed(harness, Suppression::LaunchAttemptFailed, |inputs| {
            inputs.memory.launch_stop = Some(LaunchStop::Failed);
        });
        assert_suppressed(harness, Suppression::LaunchAttemptIneffective, |inputs| {
            inputs.memory.launch_stop = Some(LaunchStop::Ineffective);
        });
    }
}

#[test]
fn suppresses_attempt_open() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::AttemptOpen, |inputs| inputs.memory.attempt_open = true);
        let tracker = tracker_with_mark(harness);
        tracker.open("chat-1", OpenAttempt { attempt_id: "attempt-1".to_owned(), launch: launch() });
        let memory = tracker.memory_inputs("chat-1", at(30));
        assert_eq!(memory.as_ref().and_then(rule::precheck), Some(Suppression::AttemptOpen));
        // A settlement for another attempt leaves the open one.
        assert_eq!(tracker.settle("chat-1", "attempt-0", Outcome::Failed), None);
        assert!(tracker.open_attempt("chat-1").is_some());
    }
}

#[test]
fn suppresses_too_soon() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::TooSoon, |inputs| inputs.memory.now = at_ms(19_999));
        assert_suppressed(harness, Suppression::TooSoon, |inputs| inputs.memory.now = at(-1));
        assert_fires(harness, |inputs| inputs.memory.now = at(20));
    }
}

#[test]
fn suppresses_cache_cold() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::CacheCold, |inputs| inputs.memory.now = at_ms(100_001));
        assert_fires(harness, |inputs| inputs.memory.now = at(100));
    }
}

#[test]
fn suppresses_turn_active() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::TurnActive, |inputs| {
            inputs.snapshot.session.status = "working".to_owned();
        });
        assert_suppressed(harness, Suppression::TurnActive, |inputs| {
            inputs.snapshot.session.active_turn_id = Some("turn-2".to_owned());
        });
        for status in ["waiting", "checkpointing", "stopped"] {
            assert_suppressed(harness, Suppression::TurnActive, |inputs| {
                inputs.snapshot.session.status = status.to_owned();
            });
        }
        assert_fires(harness, |inputs| inputs.snapshot.session.status = "idle".to_owned());
    }
}

#[test]
fn suppresses_approval_open() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ApprovalOpen, |inputs| {
            inputs.snapshot.state.approval = Probe::Present;
        });
    }
}

#[test]
fn suppresses_tool_open() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ToolOpen, |inputs| inputs.snapshot.state.tool = Probe::Present);
    }
}

#[test]
fn suppresses_children_live() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ChildrenLive, |inputs| {
            inputs.snapshot.state.children = Probe::Present;
        });
    }
}

#[test]
fn suppresses_queued_input() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::QueuedInput, |inputs| inputs.snapshot.state.queue = Probe::Present);
    }
}

#[test]
fn suppresses_state_unknown() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::StateUnknown, |inputs| inputs.snapshot.state.approval = Probe::Unknown);
        assert_suppressed(harness, Suppression::StateUnknown, |inputs| inputs.snapshot.state.tool = Probe::Unknown);
        assert_suppressed(harness, Suppression::StateUnknown, |inputs| inputs.snapshot.state.children = Probe::Unknown);
        assert_suppressed(harness, Suppression::StateUnknown, |inputs| inputs.snapshot.state.queue = Probe::Unknown);
        // A known open item comes first in the order.
        assert_suppressed(harness, Suppression::QueuedInput, |inputs| {
            inputs.snapshot.state.tool = Probe::Unknown;
            inputs.snapshot.state.queue = Probe::Present;
        });
    }
}

#[test]
fn suppresses_no_reading() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::NoReading, |inputs| inputs.snapshot.reading = None);
    }
}

#[test]
fn suppresses_reading_mismatch_for_another_model() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ReadingMismatch, |inputs| {
            if let Some(reading) = inputs.snapshot.reading.as_mut() {
                reading.model = Some("another-model".to_owned());
            }
        });
        assert_suppressed(harness, Suppression::ReadingMismatch, |inputs| {
            if let Some(reading) = inputs.snapshot.reading.as_mut() {
                reading.model = None;
            }
        });
    }
}

#[test]
fn suppresses_reading_mismatch_for_another_thread() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ReadingMismatch, |inputs| {
            if let Some(reading) = inputs.snapshot.reading.as_mut() {
                reading.provider_session_id = Some("thread-0".to_owned());
            }
        });
        assert_suppressed(harness, Suppression::ReadingMismatch, |inputs| {
            if let Some(reading) = inputs.snapshot.reading.as_mut() {
                reading.harness = if harness == "claude" { "codex" } else { "claude" }.to_owned();
            }
        });
    }
}

#[test]
fn suppresses_below_threshold() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::BelowThreshold, |inputs| {
            if let Some(reading) = inputs.snapshot.reading.as_mut() {
                reading.used_tokens = THRESHOLD_TOKENS - 1;
            }
        });
    }
}

#[test]
fn fires_at_the_threshold() {
    for harness in BOTH {
        let mut inputs = firing(harness);
        if let Some(reading) = inputs.snapshot.reading.as_mut() {
            reading.used_tokens = THRESHOLD_TOKENS;
        }
        assert_eq!(rule::precheck(&inputs.memory), None);
        let decision = rule::decide(&inputs);
        assert_eq!(decision, FIRE, "{harness}");
        assert_eq!(decision.as_str(), "window_over_threshold");
        // The same inputs give the same decision.
        assert_eq!(rule::decide(&inputs), decision);
    }
}

#[test]
fn suppressions_are_checked_in_the_stated_order() {
    for harness in BOTH {
        assert_eq!(rule::decide(&firing(harness)), FIRE, "{harness}");
        for suppression in ORDER {
            let mut inputs = firing(harness);
            make_hold(suppression, &mut inputs);
            assert_eq!(rule::decide(&inputs), Decision::Suppress(suppression), "{harness} alone");
        }
        // With each later suppression true as well, the first one still wins.
        for first in 0..ORDER.len() {
            let mut inputs = firing(harness);
            for suppression in ORDER[first..].iter().rev() {
                make_hold(*suppression, &mut inputs);
            }
            let name = ORDER[first].as_str();
            assert_eq!(rule::decide(&inputs), Decision::Suppress(ORDER[first]), "{harness} {name}");
        }
    }
}

const STATED: [(&str, MarkEffect); 22] = [
    ("disabled", MarkEffect::Drop),
    ("opted_out", MarkEffect::Drop),
    ("harness_not_allowed", MarkEffect::Drop),
    ("not_direct", MarkEffect::Drop),
    ("has_parent", MarkEffect::Drop),
    ("unsupported", MarkEffect::Drop),
    ("no_runtime", MarkEffect::Drop),
    ("launch_mismatch", MarkEffect::Drop),
    ("launch_attempt_failed", MarkEffect::Drop),
    ("launch_attempt_ineffective", MarkEffect::Drop),
    ("attempt_open", MarkEffect::Keep),
    ("too_soon", MarkEffect::Keep),
    ("cache_cold", MarkEffect::Drop),
    ("turn_active", MarkEffect::Keep),
    ("approval_open", MarkEffect::Keep),
    ("tool_open", MarkEffect::Keep),
    ("children_live", MarkEffect::Keep),
    ("queued_input", MarkEffect::Keep),
    ("state_unknown", MarkEffect::Keep),
    ("no_reading", MarkEffect::Keep),
    ("reading_mismatch", MarkEffect::Drop),
    ("below_threshold", MarkEffect::Drop),
];

#[test]
fn keep_and_drop_follow_the_stated_list() {
    let names: Vec<&str> = ORDER.iter().map(|suppression| suppression.as_str()).collect();
    let stated: Vec<&str> = STATED.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, stated);
    for (suppression, (name, effect)) in ORDER.into_iter().zip(STATED) {
        assert_eq!(suppression.mark(), effect, "{name}");
        let mut inputs = firing("claude");
        make_hold(suppression, &mut inputs);
        let decision = rule::decide(&inputs);
        assert_eq!(decision.as_str(), name);
        assert_eq!(decision.mark(), effect, "{name}");
        let tracker = tracker_with_mark("claude");
        assert_eq!(tracker.apply(&mark("claude"), decision), effect == MarkEffect::Drop, "{name}");
        assert_eq!(tracker.due_mark("chat-1").is_some(), effect == MarkEffect::Keep, "{name}");
    }
    // A fire removes the mark: a new mark needs a new completed turn.
    let tracker = tracker_with_mark("codex");
    assert_eq!(FIRE.mark(), MarkEffect::Drop);
    assert!(tracker.apply(&mark("codex"), FIRE));
    assert_eq!(tracker.due_mark("chat-1"), None);
    assert_eq!(tracker.memory_inputs("chat-1", at(30)), None);
}

#[test]
fn a_new_due_mark_replaces_the_old_one() {
    let tracker = tracker_with_mark("codex");
    let mut newer = mark("codex");
    newer.turn_id = Some("turn-2".to_owned());
    newer.completed_at = at(40);
    tracker.accept(Receipt::Due(newer.clone()));
    assert_eq!(tracker.due_mark("chat-1"), Some(newer.clone()));
    // A decision for the old mark leaves the newer mark.
    assert!(!tracker.apply(&mark("codex"), Decision::Suppress(Suppression::CacheCold)));
    assert_eq!(tracker.due_mark("chat-1"), Some(newer.clone()));
    // The mark of another chat is its own.
    let mut other = mark("claude");
    other.session_id = "chat-2".to_owned();
    tracker.accept(Receipt::Due(other.clone()));
    assert_eq!(tracker.due_mark("chat-1"), Some(newer.clone()));
    assert_eq!(tracker.due_mark("chat-2"), Some(other));
    assert!(tracker.apply(&newer, FIRE));
    assert_eq!(tracker.due_mark("chat-1"), None);
}

#[test]
fn a_codex_reading_binds_by_turn_id() {
    let due = mark("codex");
    let row = session("codex");
    let bind = |change: &dyn Fn(&mut Reading)| {
        let mut latest = reading("codex", 150_000);
        change(&mut latest);
        rule::bind(&due, &row, Some(&latest))
    };
    // Codex writes the reading of the turn before the turn end.
    assert_eq!(bind(&|_| {}), Binding::Bound);
    assert_eq!(bind(&|latest| latest.created_at = at(3)), Binding::Bound);
    // An older turn of the same launch, from before the turn end.
    assert_eq!(bind(&|latest| latest.turn_id = Some("turn-0".to_owned())), Binding::NoReading);
    // Another turn after the turn end.
    assert_eq!(
        bind(&|latest| {
            latest.turn_id = Some("turn-2".to_owned());
            latest.created_at = at(3);
        }),
        Binding::Mismatch
    );
    assert_eq!(
        bind(&|latest| {
            latest.turn_id = None;
            latest.created_at = at(3);
        }),
        Binding::Mismatch
    );
    assert_suppressed("codex", Suppression::ReadingMismatch, |inputs| {
        if let Some(latest) = inputs.snapshot.reading.as_mut() {
            latest.turn_id = Some("turn-2".to_owned());
            latest.created_at = at(3);
        }
    });
}

#[test]
fn a_claude_reading_binds_by_launch_and_time_after_the_turn_end() {
    let due = mark("claude");
    let row = session("claude");
    let bind = |change: &dyn Fn(&mut Reading)| {
        let mut latest = reading("claude", 150_000);
        change(&mut latest);
        rule::bind(&due, &row, Some(&latest))
    };
    assert_eq!(bind(&|_| {}), Binding::Bound);
    assert_eq!(bind(&|latest| latest.created_at = at(0)), Binding::Bound);
    assert_eq!(bind(&|latest| latest.created_at = at_ms(1_600)), Binding::Bound);
    // A row of the same thread from an earlier launch: native resume keeps
    // the thread id.
    assert_eq!(bind(&|latest| latest.created_at = at(-4_000)), Binding::Mismatch);
    assert_eq!(bind(&|latest| latest.turn_id = Some("turn-1".to_owned())), Binding::Mismatch);
    assert_eq!(bind(&|latest| latest.source = "codex.token_usage".to_owned()), Binding::Mismatch);
    // A launch start that does not parse binds no reading.
    let mut odd = due.clone();
    odd.launch = Some(Launch { started_at: "launch-1".to_owned(), provider_session_id: "thread-1".to_owned() });
    let mut odd_row = row.clone();
    odd_row.started_at = Some("launch-1".to_owned());
    assert_eq!(rule::bind(&odd, &odd_row, Some(&reading("claude", 150_000))), Binding::Mismatch);
}

#[test]
fn a_claude_reading_from_before_the_turn_end_is_no_reading() {
    let early = |inputs: &mut Inputs| {
        if let Some(latest) = inputs.snapshot.reading.as_mut() {
            latest.created_at = at_ms(-5);
        }
    };
    assert_suppressed("claude", Suppression::NoReading, early);
    let mut inputs = firing("claude");
    early(&mut inputs);
    let decision = rule::decide(&inputs);
    assert_eq!(decision.mark(), MarkEffect::Keep);
    let tracker = tracker_with_mark("claude");
    assert!(!tracker.apply(&mark("claude"), decision));
    // The late reading arrives, and the next sweep fires.
    let memory = tracker.memory_inputs("chat-1", at(40));
    assert_eq!(memory.map(|memory| decide_with("claude", memory)), Some(FIRE));
}

#[test]
fn a_model_switch_in_the_idle_delay_is_a_reading_mismatch() {
    for harness in BOTH {
        assert_suppressed(harness, Suppression::ReadingMismatch, |inputs| {
            inputs.snapshot.session.model = Some("another-model".to_owned());
        });
    }
}

// The suppression state against a real store.

type StoreResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A store with the chat of [`session`] on Claude. Its forest starts with a
/// turn start, so the tool scan has a floor.
struct Store {
    _scratch: tempfile::TempDir,
    core: crate::runtime::BridgeCore,
}

impl Store {
    fn new() -> StoreResult<Self> {
        let scratch = tempfile::tempdir()?;
        let core = crate::runtime::BridgeCore::for_tests(scratch.path());
        let store = Self { _scratch: scratch, core };
        let chat = session("claude");
        store.sql(
            "INSERT INTO sessions(id,harness,model,label,title,status,metric_source,kind,depth,provider_session_id,started_at)
             VALUES('chat-1','claude',?1,'Chat','Notes chat','ready','reported','direct',0,?2,?3)",
            rusqlite::params![chat.model, chat.provider_session_id, chat.started_at],
        )?;
        store.entry("turn.started", serde_json::json!({}), None)?;
        Ok(store)
    }

    fn db(&self) -> MutexGuard<'_, rusqlite::Connection> {
        self.core.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sql(&self, sql: &str, values: impl rusqlite::Params) -> rusqlite::Result<usize> {
        self.db().execute(sql, values)
    }

    /// Append one forest entry of the chat. Returns its sequence.
    fn entry(&self, kind: &str, payload: serde_json::Value, item: Option<&str>) -> StoreResult<i64> {
        let entry = crate::store::append_session_entry(&self.db(), "chat-1", None, kind, &payload, item, "visible", None)?;
        Ok(entry.sequence)
    }

    fn state(&self) -> ChatState {
        attempts::chat_state(&self.db(), "chat-1", "ready")
    }

    /// The decision for inputs that fire, with the state of this store.
    fn decide(&self) -> Decision {
        let mut inputs = firing("claude");
        inputs.snapshot.state = self.state();
        rule::decide(&inputs)
    }
}

#[test]
fn an_open_approval_among_resolved_ones_suppresses() -> StoreResult {
    let store = Store::new()?;
    assert_eq!(store.decide(), FIRE, "no request yet");
    let first = store.entry("approval.requested", serde_json::json!({"approvalId": "approval-a"}), None)?;
    let second = store.entry("approval.requested", serde_json::json!({"approvalId": "approval-b"}), None)?;
    store.entry("approval.resolved", serde_json::json!({"requestEventId": second}), None)?;
    assert_eq!(store.state().approval, Probe::Present, "request A is still open");
    assert_eq!(store.decide(), Decision::Suppress(Suppression::ApprovalOpen));
    store.entry("approval.resolved", serde_json::json!({"requestEventId": first}), None)?;
    assert_eq!(store.state().approval, Probe::Clear);
    assert_eq!(store.decide(), FIRE);
    Ok(())
}

#[test]
fn a_write_scope_approval_suppresses_until_its_approval_resolves() -> StoreResult {
    let store = Store::new()?;
    let approval_id = "delegation-path-scope:turn-1:src/**";
    let request = store.entry(
        "approval.requested",
        serde_json::json!({"approvalId": approval_id, "approvalType": "delegation_path_scope", "status": "pending"}),
        None,
    )?;
    assert_eq!(store.decide(), Decision::Suppress(Suppression::ApprovalOpen));
    // Policy writes this for ordinary routing, with no request id.
    store.entry("delegation.approved", serde_json::json!({"requestId": "turn-1"}), None)?;
    assert_eq!(store.decide(), Decision::Suppress(Suppression::ApprovalOpen), "a routing decision answered the card");
    store.entry("approval.resolved", serde_json::json!({"approvalId": approval_id, "requestEventId": request}), None)?;
    assert_eq!(store.decide(), FIRE);
    Ok(())
}

#[test]
fn a_foreground_tool_with_no_completion_suppresses() -> StoreResult {
    let kinds = [
        ("tool.started", "tool.completed"),
        ("command.started", "command.completed"),
        ("file_change.started", "file_change.completed"),
    ];
    for (start, end) in kinds {
        let store = Store::new()?;
        store.entry(start, serde_json::json!({"status": "inProgress"}), Some("item-1"))?;
        assert_eq!(store.state().tool, Probe::Present, "{start}");
        assert_eq!(store.decide(), Decision::Suppress(Suppression::ToolOpen), "{start}");
        store.entry(end, serde_json::json!({"status": "completed"}), Some("item-2"))?;
        assert_eq!(store.decide(), Decision::Suppress(Suppression::ToolOpen), "{start}: another item closed it");
        store.entry(end, serde_json::json!({"status": "completed"}), Some("item-1"))?;
        assert_eq!(store.decide(), FIRE, "{start}");
    }
    // A start from before the last turn start is not open.
    let store = Store::new()?;
    store.entry("tool.started", serde_json::json!({}), Some("item-old"))?;
    store.entry("turn.started", serde_json::json!({}), None)?;
    assert_eq!(store.state().tool, Probe::Clear);
    Ok(())
}

#[test]
fn a_worker_result_or_outbox_delivery_not_done_suppresses() -> StoreResult {
    let store = Store::new()?;
    store.sql("INSERT INTO sessions(id,harness,label,status,metric_source,parent_session_id) VALUES('worker-1','codex','Worker','working','reported','chat-1')", [])?;
    store.sql(
        "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at)
         VALUES('worker-1','chat-1','working','research','key','pending','now')",
        [],
    )?;
    assert_eq!(store.state().children, Probe::Present);
    assert_eq!(store.decide(), Decision::Suppress(Suppression::ChildrenLive), "worker result");
    store.sql("UPDATE worker_runtime SET result_status='reported' WHERE session_id='worker-1'", [])?;
    assert_eq!(store.decide(), FIRE);
    store.sql(
        "INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,next_attempt_at,created_at)
         VALUES('result','parent','worker.result','{\"report\":{\"parent_session_id\":\"chat-1\"}}','result','pending','now','now')",
        [],
    )?;
    assert_eq!(store.decide(), Decision::Suppress(Suppression::ChildrenLive), "outbox delivery");
    store.sql("UPDATE durable_outbox SET status='delivered' WHERE id='result'", [])?;
    assert_eq!(store.decide(), FIRE);
    Ok(())
}

#[test]
fn a_tool_scan_past_the_cap_is_state_unknown() -> StoreResult {
    let store = Store::new()?;
    let fill = |count: usize| -> StoreResult {
        let base: i64 = store.db().query_row("SELECT MAX(sequence) FROM session_entries WHERE session_id='chat-1'", [], |row| row.get(0))?;
        store.sql(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?2)
             INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             SELECT 'filler-'||(?1+i),'chat-1',?1+i,'assistant.message','{}','2026-11-20T10:00:00+00:00' FROM n",
            rusqlite::params![base, i64::try_from(count)?],
        )?;
        Ok(())
    };
    // The turn start is the last row inside the cap.
    fill(TOOL_SCAN_CAP - 1)?;
    assert_eq!(store.state().tool, Probe::Clear);
    assert_eq!(store.decide(), FIRE);
    fill(1)?;
    assert_eq!(store.state().tool, Probe::Unknown);
    assert_eq!(store.decide(), Decision::Suppress(Suppression::StateUnknown));
    Ok(())
}

#[test]
fn an_open_tool_with_no_turn_start_inside_the_cap_is_unknown() -> StoreResult {
    let store = Store::new()?;
    store.entry("tool.started", serde_json::json!({"status": "inProgress"}), Some("item-1"))?;
    assert_eq!(store.state().tool, Probe::Present, "the turn start is inside the cap");
    // Newer rows push the turn start out of the cap. The open tool stays inside it.
    let base: i64 = store.db().query_row("SELECT MAX(sequence) FROM session_entries WHERE session_id='chat-1'", [], |row| row.get(0))?;
    store.sql(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?2)
         INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
         SELECT 'filler-'||(?1+i),'chat-1',?1+i,'assistant.message','{}','2026-11-20T10:00:00+00:00' FROM n",
        rusqlite::params![base, i64::try_from(TOOL_SCAN_CAP - 1)?],
    )?;
    assert_eq!(store.state().tool, Probe::Unknown, "an open tool with no turn start proves nothing");
    assert_eq!(store.decide(), Decision::Suppress(Suppression::StateUnknown));
    Ok(())
}

#[test]
fn a_failed_state_query_is_state_unknown() -> StoreResult {
    let probes: [(&str, fn(ChatState) -> Probe); 3] = [
        ("queued_session_input", |state| state.queue),
        ("attention_events", |state| state.approval),
        ("session_entries", |state| state.tool),
    ];
    for (table, probe) in probes {
        let store = Store::new()?;
        assert_eq!(store.decide(), FIRE, "{table}");
        store.sql(&format!("ALTER TABLE {table} RENAME TO {table}_gone"), [])?;
        assert_eq!(probe(store.state()), Probe::Unknown, "{table}");
        assert_eq!(store.decide(), Decision::Suppress(Suppression::StateUnknown), "{table}");
    }
    Ok(())
}

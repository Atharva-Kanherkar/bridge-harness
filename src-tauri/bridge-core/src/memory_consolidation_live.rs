//! The production dream: one hidden bounded session per batch on the memory
//! model.
//!
//! Same shape as the live extraction and evaluation runs, and bounded the same
//! way: each session is `kind = 'consolidation'` so no surface lists it; the
//! briefing policy is compiled with an empty scope so every tool call is
//! denied; each batch is exactly one turn with no repair; and every exit is a
//! settled run row with observed tokens, spend and the report. The plan is
//! read and each answer gated under short database locks — a model call holds
//! none. The learning router is never consulted.
//!
//! The expiry sweep runs on this thread rather than the model's, because it is
//! deterministic and has to happen whether or not a scope ever configures a
//! consolidation profile: a record with an expiry stops applying on time in
//! every install, and only the reorganising half is opt-in.

use std::io::BufRead;
use std::sync::{mpsc, Arc};
use std::time::{Duration as StdDuration, Instant};

use bridge_protocol::messages as wire;
use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::events::CoreEvent;
use crate::memory_consolidation::{
    self, ClaimedConsolidation, ConsolidationModel, ConsolidationOutput, CONSOLIDATION_SESSION_KIND,
    STATUS_COMPLETED, STATUS_FAILED,
};
use crate::memory_dream::{self, DreamDb, DreamOutcome};
use crate::BridgeError;
use crate::memory_ledger;
use crate::runtime::BridgeCore;

const MAX_WALL_SECONDS: i64 = 240;
const POLL_SECONDS: u64 = 60;

/// The cadence loop, started once per host beside the other maintenance
/// threads. Every tick sweeps expiries and stale proposals, queues the nightly
/// dream once its hour has passed (or a dream for a scope that reached its
/// budget), and then claims at most one due run — so the deterministic half
/// keeps working in a scope that turns dreaming off.
pub fn start_consolidation_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(POLL_SECONDS));
        let now = Utc::now();
        let swept = {
            let db = core.db.lock().unwrap();
            memory_ledger::sweep_expired(&db, now).and_then(|mut expired| {
                expired.extend(memory_ledger::sweep_stale_proposals(&db, now)?);
                expired.extend(crate::memory_retention::sweep_unused(&db, now)?);
                Ok(expired)
            })
        };
        if let Err(error) = crate::taste::signals::sweep(&core.db.lock().unwrap(), now) {
            eprintln!("[taste] signal retention sweep failed: {error}");
        }
        if matches!(&swept, Ok(expired) if !expired.is_empty()) {
            core.events.publish(CoreEvent::MemoryChanged {
                scope_key: memory_ledger::account_memory_scope().to_string(),
            });
        }
        let claimed = {
            let db = core.db.lock().unwrap();
            crate::memory_model::resolve(&db, &core.adapter_registry).and_then(|memory| {
                // The local clock decides the night; a failed check is logged
                // and the tick still claims whatever is already due.
                if let Err(error) = memory_consolidation::enqueue_nightly(&db, &chrono::Local::now(), &memory)
                    .and_then(|_| memory_consolidation::enqueue_when_full(&db, now, &memory))
                {
                    eprintln!("memory dream: scheduling failed: {error}");
                }
                memory_consolidation::claim_due(&db, now, &memory)
            })
        };
        if let Ok(Some(run)) = claimed {
            execute(&core, run);
        }
    });
}

/// The core's store, locked only for each step of a dream.
struct Locked<'a>(&'a Arc<BridgeCore>);

impl DreamDb for Locked<'_> {
    fn with<R>(&self, work: impl FnOnce(&rusqlite::Connection) -> R) -> R {
        let db = self.0.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        work(&db)
    }
}

/// One hidden bounded turn per batch on the memory model.
struct LiveDream<'a> {
    core: &'a Arc<BridgeCore>,
    harness: &'a str,
    model: Option<&'a str>,
}

impl ConsolidationModel for LiveDream<'_> {
    fn consolidate(&mut self, prompt: &str) -> Result<ConsolidationOutput, BridgeError> {
        one_bounded_turn(self.core, self.harness, self.model, prompt).map_err(BridgeError::Invalid)
    }
}

pub fn execute(core: &Arc<BridgeCore>, claimed: ClaimedConsolidation) {
    let ClaimedConsolidation { run_id, scope_key, lease_owner, harness, model, .. } = claimed;
    let mut live = LiveDream { core, harness: &harness, model: model.as_deref() };
    // The report is stored after every batch, which also renews the lease;
    // a worker that lost its lease stops instead of writing over the next.
    let outcome = memory_dream::drive(
        &Locked(core),
        &mut live,
        &scope_key,
        Utc::now(),
        |db, report| {
            let stored = serde_json::to_string(report)
                .map_err(|error| BridgeError::Invalid(error.to_string()))
                .and_then(|json| memory_consolidation::record_report(db, &run_id, &lease_owner, &json, Utc::now()));
            stored.unwrap_or_else(|error| {
                eprintln!("memory dream: the report was not stored: {error}");
                false
            })
        },
        || core.events.publish(CoreEvent::MemoryChanged { scope_key: scope_key.clone() }),
    );
    let finish = |status: &str, detail: &str, outcome: Option<&DreamOutcome>| {
        settle(core, &run_id, &lease_owner, status, detail, &harness, model.as_deref(), outcome);
    };
    match outcome {
        Ok(outcome) if outcome.sha256.is_none() => finish(STATUS_COMPLETED, "no_candidates", None),
        // Every batch answered with nothing the gate could read: a failure,
        // not a quiet night.
        Ok(outcome) if outcome.report.failed_batches == outcome.report.batches => {
            finish(STATUS_FAILED, &outcome.report.detail(), Some(&outcome))
        }
        Ok(outcome) => finish(STATUS_COMPLETED, &outcome.report.detail(), Some(&outcome)),
        Err(error) => finish(STATUS_FAILED, &error.to_string(), None),
    }
}

#[allow(clippy::too_many_arguments)]
fn settle(
    core: &Arc<BridgeCore>,
    run_id: &str,
    lease_owner: &str,
    status: &str,
    detail: &str,
    harness: &str,
    model: Option<&str>,
    outcome: Option<&DreamOutcome>,
) {
    let (tokens, spend, applied, refused) = match outcome {
        Some(outcome) => (
            outcome.output.observed_tokens,
            outcome.output.spend_microusd,
            outcome.report.applied() as i64,
            (outcome.report.refused_total() + outcome.report.invalid) as i64,
        ),
        None => (0, 0, 0, 0),
    };
    let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stored = outcome
        .map(|outcome| {
            serde_json::to_string(&outcome.report)
                .map_err(|error| BridgeError::Invalid(error.to_string()))
                .and_then(|json| memory_consolidation::record_report(&db, run_id, lease_owner, &json, Utc::now()))
        })
        .unwrap_or(Ok(true));
    if let Err(error) = stored.and_then(|_| {
        memory_consolidation::settle(
            &db,
            run_id,
            lease_owner,
            status,
            Some(detail),
            Some(harness),
            model,
            outcome.and_then(|outcome| outcome.sha256.as_deref()),
            tokens,
            spend,
            applied,
            refused,
            Utc::now(),
        )
    }) {
        eprintln!("memory dream: the run was not settled: {error}");
    }
}

/// Spawn the hidden session, send the prompt as its one turn, and read the
/// stream to the result marker under a wall-clock deadline.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    harness: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<ConsolidationOutput, String> {
    let limits = wire::WorkBriefLimits {
        max_wall_seconds: MAX_WALL_SECONDS,
        max_turns: 1,
        max_tool_calls: 1,
        max_output_tokens: None,
        cost_ceiling_microusd: None,
    };
    let policy = BriefingRuntimePolicy::compile_scoped(Vec::new(), limits)
        .map_err(|unsupported| format!("policy: {}", unsupported.reason()))?;

    let session_id = Uuid::new_v4().to_string();
    let scratch = core.chat_scratch_dir(&session_id);
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let cwd = scratch.to_string_lossy().to_string();
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
             VALUES(?1,NULL,?2,'Consolidation','working','reported',?3,?4,'Memory consolidation',?5,0)",
            params![session_id, harness, model, CONSOLIDATION_SESSION_KIND, cwd],
        )
        .map_err(|error| error.to_string())?;
    }

    let started = core.adapter_registry.start(
        harness,
        StartRequest {
            session_id: None,
            cwd: &cwd,
            model,
            effort: None,
            speed: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: Some(&policy),
            interactive: false,
            account: None,
            on_progress: None,
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            settle_session(core, &session_id, "failed");
            return Err(format!("provider start: {error}"));
        }
    };
    let mut runtime = started.runtime;

    let (lines, receiver) = mpsc::channel::<String>();
    let mut reader = started.reader;
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if lines.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    if let Err(error) = runtime.send_turn(prompt) {
        runtime.stop(ShutdownReason::Failed);
        settle_session(core, &session_id, "failed");
        return Err(format!("send: {error}"));
    }

    let deadline = Instant::now() + StdDuration::from_secs(MAX_WALL_SECONDS as u64);
    let mut text = String::new();
    let mut tokens: i64 = 0;
    let mut spend: i64 = 0;
    let outcome = loop {
        if Instant::now() >= deadline {
            break Err("deadline exceeded".to_string());
        }
        match receiver.recv_timeout(StdDuration::from_secs(1)) {
            Ok(line) => {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                match message.get("type").and_then(Value::as_str) {
                    Some("assistant") => {
                        let blocks = message
                            .get("message")
                            .and_then(|inner| inner.get("content"))
                            .or_else(|| message.get("content"))
                            .and_then(Value::as_array);
                        for block in blocks.into_iter().flatten() {
                            if block.get("type").and_then(Value::as_str) == Some("text") {
                                if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                                    if !text.is_empty() {
                                        text.push('\n');
                                    }
                                    text.push_str(chunk);
                                }
                            }
                        }
                    }
                    Some("result") => {
                        if let Some(usage) = message.get("usage") {
                            tokens += usage.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                            tokens +=
                                usage.get("output_tokens").and_then(Value::as_i64).unwrap_or(0);
                        }
                        if let Some(cost) = message.get("total_cost_usd").and_then(Value::as_f64) {
                            spend += (cost * 1_000_000.0).round() as i64;
                        }
                        break Ok(());
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("provider ended before a result".to_string());
            }
        }
    };

    match outcome {
        Ok(()) => {
            runtime.stop(ShutdownReason::Completed);
            settle_session(core, &session_id, "ended");
            Ok(ConsolidationOutput { text, observed_tokens: tokens, spend_microusd: spend })
        }
        Err(detail) => {
            runtime.stop(ShutdownReason::Failed);
            settle_session(core, &session_id, "failed");
            Err(detail)
        }
    }
}

fn settle_session(core: &Arc<BridgeCore>, session_id: &str, status: &str) {
    let db = core.db.lock().unwrap();
    let _ = db.execute(
        "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
        params![session_id, status, Utc::now().to_rfc3339()],
    );
}

//! The production consolidation run: a hidden bounded session on the pinned
//! harness and model.
//!
//! Same shape as the live extraction and evaluation runs, and bounded the same
//! way: the session is `kind = 'consolidation'` so no surface lists it; the
//! briefing policy is compiled with an empty scope so every tool call is
//! denied; there is exactly one turn and no repair; and every exit is a settled
//! run row with observed tokens and spend. The candidate list is assembled and
//! the gate runs under short database locks — the model call itself holds none.
//! The learning router is never consulted.
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
    self, ClaimedConsolidation, ConsolidationOutput, CONSOLIDATION_SESSION_KIND, STATUS_COMPLETED,
    STATUS_FAILED,
};
use crate::memory_ledger;
use crate::runtime::BridgeCore;

const MAX_WALL_SECONDS: i64 = 240;
const POLL_SECONDS: u64 = 60;

/// The cadence loop, started once per host beside the other maintenance
/// threads. Every tick sweeps expiries, schedules a run for a scope that has
/// reached its budget, and then claims at most one due run — so the
/// deterministic half keeps working in a scope that never turns the bounded
/// half on, and a full scope that nobody is talking to still gets consolidated.
pub fn start_consolidation_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(POLL_SECONDS));
        let now = Utc::now();
        let swept = {
            let db = core.db.lock().unwrap();
            memory_ledger::sweep_expired(&db, now)
        };
        if matches!(&swept, Ok(expired) if !expired.is_empty()) {
            core.events.publish(CoreEvent::MemoryChanged {
                scope_key: memory_ledger::account_memory_scope().to_string(),
            });
        }
        {
            let db = core.db.lock().unwrap();
            let _ = memory_consolidation::enqueue_when_full(&db, now);
        }
        let claimed = {
            let db = core.db.lock().unwrap();
            memory_consolidation::claim_due(&db, now)
        };
        if let Ok(Some(run)) = claimed {
            execute(&core, run);
        }
    });
}

pub fn execute(core: &Arc<BridgeCore>, claimed: ClaimedConsolidation) {
    let ClaimedConsolidation { run_id, scope_key, lease_owner, harness, model, .. } = claimed;

    let prepared = {
        let db = core.db.lock().unwrap();
        memory_consolidation::settings(&db, &scope_key).and_then(|settings| {
            memory_ledger::sweep_expired(&db, Utc::now())?;
            Ok((
                settings.allow_removal,
                memory_consolidation::build_candidates(&db, &scope_key)?,
            ))
        })
    };
    let (allow_removal, candidates) = match prepared {
        Ok((allow_removal, Some(candidates))) => (allow_removal, candidates),
        Ok((_, None)) => {
            settle(core, &run_id, &lease_owner, STATUS_COMPLETED, "no_candidates", &harness, &model, None, None);
            return;
        }
        Err(error) => {
            settle(core, &run_id, &lease_owner, STATUS_FAILED, &error.to_string(), &harness, &model, None, None);
            return;
        }
    };

    let prompt = memory_consolidation::compose_prompt(&candidates, allow_removal);
    let output = match one_bounded_turn(core, &harness, &model, &prompt) {
        Ok(output) => output,
        Err(detail) => {
            settle(core, &run_id, &lease_owner, STATUS_FAILED, &detail, &harness, &model, Some(&candidates.sha256), None);
            return;
        }
    };

    let report = {
        let db = core.db.lock().unwrap();
        memory_consolidation::gate_and_apply(&db, &scope_key, &output.text, Utc::now())
    };
    match report {
        Ok(report) => {
            if report.applied > 0 {
                core.events.publish(CoreEvent::MemoryChanged { scope_key: scope_key.clone() });
            }
            settle(
                core,
                &run_id,
                &lease_owner,
                STATUS_COMPLETED,
                &report.detail(),
                &harness,
                &model,
                Some(&candidates.sha256),
                Some((&output, report.applied as i64, report.refused as i64)),
            );
        }
        Err(error) => settle(
            core,
            &run_id,
            &lease_owner,
            STATUS_FAILED,
            &error.to_string(),
            &harness,
            &model,
            Some(&candidates.sha256),
            Some((&output, 0, 0)),
        ),
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
    model: &str,
    prompt_digest: Option<&str>,
    output: Option<(&ConsolidationOutput, i64, i64)>,
) {
    let (tokens, spend, applied, refused) = match output {
        Some((output, applied, refused)) => {
            (output.observed_tokens, output.spend_microusd, applied, refused)
        }
        None => (0, 0, 0, 0),
    };
    let db = core.db.lock().unwrap();
    let _ = memory_consolidation::settle(
        &db,
        run_id,
        lease_owner,
        status,
        Some(detail),
        Some(harness),
        Some(model),
        prompt_digest,
        tokens,
        spend,
        applied,
        refused,
        Utc::now(),
    );
}

/// Spawn the hidden session, send the prompt as its one turn, and read the
/// stream to the result marker under a wall-clock deadline.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    harness: &str,
    model: &str,
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
            cwd: &cwd,
            model: Some(model),
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: Some(&policy),
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

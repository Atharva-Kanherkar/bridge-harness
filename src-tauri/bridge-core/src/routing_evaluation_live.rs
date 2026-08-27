//! The production evaluation run: a hidden bounded session on the profile the
//! deterministic core chose.
//!
//! Same shape as the live extraction run, and bounded the same way: the session
//! is `kind = 'outcome_evaluation'` so no surface lists it; the briefing policy
//! is compiled with an empty scope so every tool call is denied; there is
//! exactly one turn and no repair; and every exit is a settled run row with
//! observed tokens and spend. The bundle is built and the gate runs under short
//! database locks — the model call itself holds none.
//!
//! Two exits deserve naming, because getting them wrong is how a judge poisons
//! a learner. A provider that never answers, or answers something the gate
//! refuses, settles `failed` with no score. A judge that says the evidence does
//! not settle the rubric settles `skipped`, also with no score. Neither is
//! written as a zero, because a zero is a statement about the work and neither
//! of those is one.

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
use crate::learning_job;
use crate::routing_evaluation::{
    self, ClaimedEvaluation, EvaluationOutput, EVALUATION_SESSION_KIND, STATUS_COMPLETED,
    STATUS_FAILED,
};
use crate::runtime::BridgeCore;

const MAX_WALL_SECONDS: i64 = 180;
const POLL_SECONDS: u64 = 60;

/// The cadence loop, started once per host beside the other maintenance
/// threads. Every due run funnels through the same lease.
pub fn start_evaluation_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(POLL_SECONDS));
        let claimed = {
            let db = core.db.lock().unwrap();
            routing_evaluation::claim_due(&db, Utc::now())
        };
        if let Ok(Some(run)) = claimed {
            execute(&core, run);
        }
    });
}

pub fn execute(core: &Arc<BridgeCore>, claimed: ClaimedEvaluation) {
    let evidence = {
        let db = core.db.lock().unwrap();
        routing_evaluation::build_evidence(&db, &claimed.decision_id)
    };
    let evidence = match evidence {
        Ok(Some(evidence)) => evidence,
        Ok(None) => {
            settle(core, &claimed, STATUS_FAILED, "no recorded outcome to evaluate", None, None, None, (0, 0));
            return;
        }
        Err(error) => {
            settle(core, &claimed, STATUS_FAILED, &error.to_string(), None, None, None, (0, 0));
            return;
        }
    };

    let prompt = routing_evaluation::compose_prompt(&evidence);
    let output = match one_bounded_turn(
        core,
        &claimed.harness,
        &claimed.model,
        claimed.effort.as_deref(),
        &prompt,
    ) {
        Ok(output) => output,
        Err(detail) => {
            settle(core, &claimed, STATUS_FAILED, &detail, Some(&evidence.sha256), None, None, (0, 0));
            return;
        }
    };
    let usage = (output.observed_tokens, output.spend_microusd);

    let verdict = {
        let db = core.db.lock().unwrap();
        routing_evaluation::gate_and_record(&db, &claimed, &evidence, &output.text, Utc::now())
    };
    match verdict {
        Ok(verdict) => {
            let status = verdict.settled_status();
            let scored = status == STATUS_COMPLETED;
            settle(
                core,
                &claimed,
                status,
                &verdict.detail,
                Some(&evidence.sha256),
                scored.then_some(verdict.score_bps).flatten(),
                scored.then_some(verdict.confidence_bps),
                usage,
            );
        }
        Err(error) => settle(
            core,
            &claimed,
            STATUS_FAILED,
            &error.to_string(),
            Some(&evidence.sha256),
            None,
            None,
            usage,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn settle(
    core: &Arc<BridgeCore>,
    claimed: &ClaimedEvaluation,
    status: &str,
    detail: &str,
    evidence_digest: Option<&str>,
    score_bps: Option<i64>,
    confidence_bps: Option<i64>,
    usage: (i64, i64),
) {
    let settled = {
        let db = core.db.lock().unwrap();
        routing_evaluation::settle(
            &db,
            &claimed.run_id,
            &claimed.lease_owner,
            status,
            Some(detail),
            evidence_digest,
            score_bps,
            confidence_bps,
            usage.0,
            usage.1,
            Utc::now(),
        )
    };
    if !matches!(settled, Ok(true)) {
        return;
    }
    let Some(learning_run_id) = claimed.learning_run_id.as_deref() else {
        return;
    };
    let run = {
        let db = core.db.lock().unwrap();
        learning_job::refresh_evaluated_usage(&db, learning_run_id)
            .and_then(|()| learning_job::load_run(&db, learning_run_id))
    };
    if let Ok(Some(run)) = run {
        core.events.publish(CoreEvent::LearningJobChanged(
            serde_json::to_value(&run).unwrap_or_default(),
        ));
    }
}

/// Spawn the hidden session, send the prompt as its one turn, and read the
/// stream to the result marker under a wall-clock deadline.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    harness: &str,
    model: &str,
    effort: Option<&str>,
    prompt: &str,
) -> Result<EvaluationOutput, String> {
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
             VALUES(?1,NULL,?2,'Evaluation','working','reported',?3,?4,'Outcome evaluation',?5,0)",
            params![session_id, harness, model, EVALUATION_SESSION_KIND, cwd],
        )
        .map_err(|error| error.to_string())?;
    }

    let started = core.adapter_registry.start(
        harness,
        StartRequest {
            cwd: &cwd,
            model: Some(model),
            effort,
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
            Ok(EvaluationOutput { text, observed_tokens: tokens, spend_microusd: spend })
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

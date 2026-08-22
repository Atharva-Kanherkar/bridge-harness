//! The production extraction run: a hidden bounded session on the pinned
//! harness and model.
//!
//! Same shape as the live briefing run, smaller in every dimension: the
//! session is `kind = 'extraction'` so no surface lists it; the briefing
//! policy is compiled with an empty scope so every tool call is denied; there
//! is exactly one turn and no repair; and every exit is a settled run row with
//! observed tokens and spend. The digest is built and the gate runs under
//! short database locks — the model call itself holds none. The learning
//! router is never consulted here.

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
use crate::memory_extraction::{
    self, ClaimedExtraction, ExtractionOutput, EXTRACTION_SESSION_KIND,
};
use crate::runtime::BridgeCore;

const MAX_WALL_SECONDS: i64 = 240;
const POLL_SECONDS: u64 = 60;

/// The cadence loop, started once per host beside the other maintenance
/// threads. Every due run funnels through the same lease as every trigger.
pub fn start_extraction_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(POLL_SECONDS));
        let claimed = {
            let db = core.db.lock().unwrap();
            memory_extraction::claim_due(&db, Utc::now())
        };
        if let Ok(Some(run)) = claimed {
            execute(&core, run);
        }
    });
}

pub fn execute(core: &Arc<BridgeCore>, claimed: ClaimedExtraction) {
    let ClaimedExtraction { run_id, scope_key, session_id, lease_owner, harness, model } = claimed;

    let digest = {
        let db = core.db.lock().unwrap();
        memory_extraction::build_digest(&db, &scope_key, &session_id)
    };
    let digest = match digest {
        Ok(Some(digest)) => digest,
        Ok(None) => {
            settle(core, &run_id, &lease_owner, "completed", Some("empty_digest"), &harness, &model, None, None);
            return;
        }
        Err(error) => {
            settle(core, &run_id, &lease_owner, "failed", Some(&error.to_string()), &harness, &model, None, None);
            return;
        }
    };

    let output = match one_bounded_turn(core, &harness, &model, &digest.text) {
        Ok(output) => output,
        Err(detail) => {
            settle(core, &run_id, &lease_owner, "failed", Some(&detail), &harness, &model, Some(&digest.sha256), None);
            return;
        }
    };

    let report = {
        let db = core.db.lock().unwrap();
        memory_extraction::gate_and_insert(&db, &scope_key, &session_id, &output.text)
    };
    match report {
        Ok(report) => {
            if report.written > 0 {
                core.events.publish(CoreEvent::MemoryChanged { scope_key: scope_key.clone() });
            }
            let detail = format!(
                "written {} invalid {} duplicates {} refused {}",
                report.written, report.invalid, report.duplicates, report.refused
            );
            settle(
                core, &run_id, &lease_owner, "completed", Some(&detail), &harness, &model,
                Some(&digest.sha256), Some((&output, report.written as i64)),
            );
        }
        Err(error) => {
            settle(
                core, &run_id, &lease_owner, "failed", Some(&error.to_string()), &harness, &model,
                Some(&digest.sha256), Some((&output, 0)),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn settle(
    core: &Arc<BridgeCore>,
    run_id: &str,
    lease_owner: &str,
    status: &str,
    detail: Option<&str>,
    harness: &str,
    model: &str,
    prompt_digest: Option<&str>,
    output: Option<(&ExtractionOutput, i64)>,
) {
    let (tokens, spend, proposals) = match output {
        Some((output, written)) => (output.observed_tokens, output.spend_microusd, written),
        None => (0, 0, 0),
    };
    let db = core.db.lock().unwrap();
    let _ = memory_extraction::settle(
        &db, run_id, lease_owner, status, detail, Some(harness), Some(model),
        prompt_digest, tokens, spend, proposals,
    );
}

/// Spawn the hidden session, send the digest as its one turn, and read the
/// stream to the result marker under a wall-clock deadline.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    harness: &str,
    model: &str,
    digest: &str,
) -> Result<ExtractionOutput, String> {
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
             VALUES(?1,NULL,?2,'Extraction','working','reported',?3,?4,'Memory extraction',?5,0)",
            params![session_id, harness, model, EXTRACTION_SESSION_KIND, cwd],
        )
        .map_err(|error| error.to_string())?;
    }

    let instructions = memory_extraction::extraction_instructions();
    let started = core.adapter_registry.start(
        harness,
        StartRequest {
            cwd: &cwd,
            model: Some(model),
            effort: None,
            instructions: Some(&instructions),
            write_mode: None,
            read_only_sandbox: None,
            briefing: Some(&policy),
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

    if let Err(error) = runtime.send_turn(digest) {
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
                            tokens += usage.get("output_tokens").and_then(Value::as_i64).unwrap_or(0);
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
            Ok(ExtractionOutput { text, observed_tokens: tokens, spend_microusd: spend })
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

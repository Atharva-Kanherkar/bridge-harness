//! The production [`SearchModel`]: one hidden, tool-free Claude session per
//! deep search.
//!
//! Same shape as the extraction and insights runs: a `chat_search`-kind
//! session no list shows, started under a toolless briefing policy, so the
//! provider is sent no tool definitions and not the coding preset either.
//! Measured on the harness's default preset, one search cost about 19k
//! tokens a turn, nearly all of it tool schemas and preset text. Each model
//! turn records its usage on that session, so the spend shows up wherever
//! provider usage does. The
//! session is stopped and settled when the search is dropped, whichever way
//! it ended. No turn is written to any forest.

use std::io::BufRead;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use bridge_protocol::messages as wire;
use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use super::agent::{ModelTurn, SearchModel, INSTRUCTIONS, MAX_TOOL_CALLS};
use super::CHAT_SEARCH_SESSION_KIND;
use crate::adapters::{AdapterRuntime, ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::runtime::BridgeCore;

/// The only harness that can enforce a turn with no tools.
pub const HARNESS: &str = "claude";
/// Ceiling the provider itself is told; the loop's own budget is far tighter.
const PROVIDER_WALL_SECONDS: i64 = 60;
/// Marks this search's rows in the usage ledger.
pub const TASK_FAMILY: &str = "chat_search";

pub struct ClaudeSearchModel {
    core: Arc<BridgeCore>,
    session_id: String,
    runtime: Option<Box<dyn AdapterRuntime>>,
    lines: mpsc::Receiver<String>,
    failed: bool,
}

impl ClaudeSearchModel {
    pub fn start(core: &Arc<BridgeCore>, model: &str) -> Result<Self, String> {
        let limits = wire::WorkBriefLimits {
            max_wall_seconds: PROVIDER_WALL_SECONDS,
            // The first turn plus one per lookup.
            max_turns: MAX_TOOL_CALLS as i64 + 1,
            max_tool_calls: 1,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        };
        let policy = BriefingRuntimePolicy::compile_toolless(limits)
            .map_err(|unsupported| format!("policy: {}", unsupported.reason()))?;

        let session_id = Uuid::new_v4().to_string();
        let scratch = core.chat_scratch_dir(&session_id);
        std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
        let cwd = scratch.to_string_lossy().to_string();
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
                 VALUES(?1,NULL,?2,'Search','working','reported',?3,?4,'Chat search',?5,0)",
                params![session_id, HARNESS, model, CHAT_SEARCH_SESSION_KIND, cwd],
            )
            .map_err(|error| error.to_string())?;
        }
        let started = core.adapter_registry.start(
            HARNESS,
            StartRequest {
                cwd: &cwd,
                model: Some(model),
                effort: None,
                instructions: Some(INSTRUCTIONS),
                write_mode: None,
                read_only_sandbox: None,
                briefing: Some(&policy),
                on_progress: None,
            },
        );
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                settle(core, &session_id, "failed");
                return Err(format!("provider start: {error}"));
            }
        };
        let (sender, lines) = mpsc::channel::<String>();
        let mut reader = started.reader;
        std::thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender.send(line.trim_end().to_owned()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            core: core.clone(),
            session_id,
            runtime: Some(started.runtime),
            lines,
            failed: false,
        })
    }

    fn record_usage(&self, result: &Value) -> u64 {
        let usage = result.get("usage");
        let tokens = ["input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
            .iter()
            .filter_map(|key| usage.and_then(|usage| usage.get(key)).and_then(Value::as_u64))
            .sum();
        let db = self.core.db.lock().unwrap();
        let _ = crate::policy::record_provider_usage(
            &db,
            &self.session_id,
            &self.session_id,
            None,
            &format!("provider.{HARNESS}"),
            result,
        );
        let _ = db.execute(
            "UPDATE usage_ledger SET task_family=?2 WHERE session_id=?1 AND task_family IS NULL",
            params![self.session_id, TASK_FAMILY],
        );
        tokens
    }
}

impl SearchModel for ClaudeSearchModel {
    fn turn(&mut self, text: &str, deadline: Instant) -> Result<ModelTurn, String> {
        let runtime = self.runtime.as_ref().ok_or("the search session has stopped")?;
        if let Err(error) = runtime.send_turn(text) {
            self.failed = true;
            return Err(format!("send: {error}"));
        }
        let mut reply = String::new();
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.failed = true;
                return Err("deadline exceeded".into());
            }
            let wait = (deadline - now).min(Duration::from_millis(250));
            match self.lines.recv_timeout(wait) {
                Ok(line) => {
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    match message.get("type").and_then(Value::as_str) {
                        Some("assistant") => {
                            let blocks = message
                                .get("message")
                                .and_then(|inner| inner.get("content"))
                                .or_else(|| message.get("content"))
                                .and_then(Value::as_array);
                            for block in blocks.into_iter().flatten() {
                                if block.get("type").and_then(Value::as_str) == Some("text") {
                                    if let Some(part) = block.get("text").and_then(Value::as_str) {
                                        reply.push_str(part);
                                    }
                                }
                            }
                        }
                        Some("result") => {
                            let tokens = self.record_usage(&message);
                            if let Some(error) = result_error(&message) {
                                self.failed = true;
                                return Err(error);
                            }
                            if reply.trim().is_empty() {
                                if let Some(result) = message.get("result").and_then(Value::as_str) {
                                    reply.push_str(result);
                                }
                            }
                            return Ok(ModelTurn { text: reply, tokens });
                        }
                        _ => {}
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.failed = true;
                    return Err("the provider ended before answering".into());
                }
            }
        }
    }
}

// The SDK can emit subtype="success" for an API failure (including 429).
// Its error flag and terminal reason are authoritative, not the subtype.
fn result_error(message: &Value) -> Option<String> {
    if message.get("is_error").and_then(Value::as_bool) != Some(true)
        && message.get("terminal_reason").and_then(Value::as_str) != Some("api_error")
    {
        return None;
    }
    let error = message.get("result").and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| message.get("errors").and_then(Value::as_array).map(|errors| {
            errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")
        }))
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "the provider returned an error".into());
    Some(super::tools::redact(&error))
}

impl Drop for ClaudeSearchModel {
    fn drop(&mut self) {
        if let Some(mut runtime) = self.runtime.take() {
            runtime.stop(if self.failed { ShutdownReason::Failed } else { ShutdownReason::Completed });
        }
        settle(&self.core, &self.session_id, if self.failed { "failed" } else { "ended" });
    }
}

static WARM: std::sync::LazyLock<Arc<super::warm::Pool<ClaudeSearchModel>>> =
    std::sync::LazyLock::new(|| Arc::new(super::warm::Pool::new()));

fn warm_key(core: &Arc<BridgeCore>, model: &str) -> String {
    format!("{:p}:{model}", Arc::as_ptr(core))
}

/// Start a search session in the background, for a query the index was
/// unsure about, so a following Enter does not wait for a cold start.
pub fn prewarm(core: &Arc<BridgeCore>, model: &str) {
    let owned = Arc::clone(core);
    let model_name = model.to_owned();
    WARM.prewarm(&warm_key(core, model), super::warm::IDLE, move || {
        ClaudeSearchModel::start(&owned, &model_name)
    });
}

/// The warm session when there is one for this model, else a fresh start.
pub fn take_or_start(core: &Arc<BridgeCore>, model: &str) -> Result<ClaudeSearchModel, String> {
    match WARM.take(&warm_key(core, model), super::warm::WAIT_FOR_START) {
        Some(session) => Ok(session),
        None => ClaudeSearchModel::start(core, model),
    }
}

fn settle(core: &Arc<BridgeCore>, session_id: &str, status: &str) {
    let db = core.db.lock().unwrap();
    let _ = db.execute(
        "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
        params![session_id, status, Utc::now().to_rfc3339()],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_sdk_quota_error_is_reported_even_with_a_success_subtype() {
        let message = json!({
            "type": "result", "subtype": "success", "is_error": true,
            "terminal_reason": "api_error", "api_error_status": 429,
            "result": "You've hit your session limit"
        });
        assert_eq!(result_error(&message).as_deref(), Some("You've hit your session limit"));
    }

    #[test]
    fn a_normal_result_is_not_an_error_and_error_arrays_are_supported() {
        assert_eq!(result_error(&json!({"is_error": false, "result": "{\"answer\":[]}"})), None);
        assert_eq!(result_error(&json!({"is_error": true, "errors": ["Not signed in"]})).as_deref(), Some("Not signed in"));
        assert_eq!(result_error(&json!({"terminal_reason": "api_error"})).as_deref(), Some("the provider returned an error"));
    }
}

//! Codex rollouts: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//!
//! A rollout is a stream of typed events. `token_count` events carry the
//! per-turn delta in `last_token_usage` and a cumulative `total_token_usage`
//! that must never be summed. They carry no model: that arrives on the
//! preceding `turn_context`, so the reducer carries it forward. A forked or
//! subagent rollout opens with the parent's history copied in and re-stamped
//! to the fork instant; that burst was already counted from the parent's own
//! file and is dropped.

use super::scan::LineOutcome;
use super::{json_count, location_fingerprint, parse_rfc3339, ParsedUsage, SourceEnv};
use crate::analytics::{
    CoverageState, DiscoveredAnalyticsSource, ExactTotalFormula, ImporterCapability,
    NumericUsagePayload, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const AGENT: &str = "codex";
pub const PROVIDER: &str = "openai";

/// Copies of parent history land in one synchronous burst (observed gaps of
/// 0–40 ms), while the child's first real turn takes seconds. One second
/// separates them; ccusage uses the same threshold.
const FORK_COPY_MAX_GAP_MS: i64 = 1_000;

pub fn discover(env: &SourceEnv) -> DiscoveredAnalyticsSource {
    let location = env.codex_sessions_dir();
    let (coverage, reason) = jsonl_root_state(&location, "Codex");
    DiscoveredAnalyticsSource {
        agent: AGENT.into(),
        provider: PROVIDER.into(),
        location_fingerprint: location_fingerprint(AGENT, &location),
        location,
        detected_version: None,
        capability: ImporterCapability::Supported,
        coverage,
        reason,
    }
}

/// Pre-scan coverage for a directory of transcripts: `partial` when at least
/// one `.jsonl` exists (nothing imported yet), `empty` otherwise, `unreadable`
/// when the directory exists but cannot be listed.
pub(crate) fn jsonl_root_state(root: &Path, label: &str) -> (CoverageState, Option<String>) {
    if !root.exists() {
        return (
            CoverageState::Empty,
            Some(format!("{label} history directory not found")),
        );
    }
    if std::fs::read_dir(root).is_err() {
        return (
            CoverageState::Unreadable,
            Some(format!("{label} history directory cannot be read")),
        );
    }
    let has_transcripts = super::scan::list_jsonl_files(root)
        .into_iter()
        .next()
        .is_some();
    if has_transcripts {
        (CoverageState::Partial, None)
    } else {
        (
            CoverageState::Empty,
            Some(format!("{label} history directory holds no transcripts")),
        )
    }
}

/// Reducer state for one rollout file, persisted in the file cursor so a
/// resumed read keeps the model, session and duplicate signature.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexScanState {
    pub model: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage_signature: Option<String>,
    pub saw_session_meta: bool,
    pub suppressing_fork_copies: bool,
    pub fork_copy_anchor_ms: i64,
}

fn epoch_ms(value: Option<&Value>) -> Option<i64> {
    let text = value?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn parent_of(payload: &Value) -> Option<(String, &'static str)> {
    if let Some(parent) = payload.get("forked_from_id").and_then(Value::as_str) {
        return Some((parent.to_string(), "fork"));
    }
    let spawn = payload
        .get("source")
        .and_then(|s| s.get("subagent"))
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(Value::as_str);
    if let Some(parent) = spawn {
        return Some((parent.to_string(), "subagent"));
    }
    None
}

/// Feeds one rollout line into `state`. `file_stem` names the session when
/// the rollout has no `session_meta`; `line_no` addresses the record.
pub(crate) fn parse_line(
    line: &str,
    state: &mut CodexScanState,
    file_fingerprint: &str,
    file_stem: &str,
    line_no: u64,
) -> LineOutcome {
    if !line.contains("\"token_count\"")
        && !line.contains("\"turn_context\"")
        && !line.contains("\"session_meta\"")
    {
        return LineOutcome::Ignored;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return LineOutcome::Ignored;
    };
    let Some(payload) = record.get("payload").filter(|p| p.is_object()) else {
        return LineOutcome::Ignored;
    };
    match record.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            // Only the first meta describes this file's own session; a fork
            // repeats its ancestors' metas right after it.
            if state.saw_session_meta {
                return LineOutcome::Ignored;
            }
            state.saw_session_meta = true;
            let id = payload
                .get("id")
                .or_else(|| payload.get("session_id"))
                .and_then(Value::as_str);
            if let Some(id) = id {
                state.session_id = id.to_string();
            }
            state.cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some((parent, kind)) = parent_of(payload) {
                state.parent_session_id = Some(parent);
                state.session_type = Some(kind.to_string());
                if let Some(anchor) = epoch_ms(record.get("timestamp")) {
                    state.suppressing_fork_copies = true;
                    state.fork_copy_anchor_ms = anchor;
                }
            }
            return LineOutcome::Ignored;
        }
        Some("turn_context") => {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                state.model = model.to_string();
            }
            return LineOutcome::Ignored;
        }
        _ => {}
    }
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return LineOutcome::Ignored;
    }
    let Some(info) = payload.get("info").filter(|i| i.is_object()) else {
        return LineOutcome::Ignored;
    };
    let Some(last) = info.get("last_token_usage").filter(|l| l.is_object()) else {
        return LineOutcome::Ignored;
    };
    let Some(occurred_at) = parse_rfc3339(record.get("timestamp")) else {
        return LineOutcome::Skipped;
    };
    let Some(timestamp_ms) = epoch_ms(record.get("timestamp")) else {
        return LineOutcome::Skipped;
    };
    // A token_count before its turn_context has no model. It must not consume
    // the duplicate signature, or the re-emitted copy after the model is
    // known would be dropped and those tokens never counted.
    if state.model.is_empty() {
        return LineOutcome::Skipped;
    }
    let signature = last.to_string();
    if state.last_usage_signature.as_deref() == Some(signature.as_str()) {
        return LineOutcome::Skipped;
    }
    state.last_usage_signature = Some(signature);
    if state.suppressing_fork_copies {
        if timestamp_ms - state.fork_copy_anchor_ms < FORK_COPY_MAX_GAP_MS {
            state.fork_copy_anchor_ms = timestamp_ms;
            return LineOutcome::Skipped;
        }
        state.suppressing_fork_copies = false;
    }

    let input = json_count(last.get("input_tokens"));
    let cached = json_count(last.get("cached_input_tokens"));
    let cache_write = json_count(last.get("cache_write_input_tokens"));
    let output = json_count(last.get("output_tokens"));
    let reasoning_raw = json_count(last.get("reasoning_output_tokens"));
    let total = json_count(last.get("total_tokens"));
    let uncached =
        input.map(|input| (input - cached.unwrap_or(0) - cache_write.unwrap_or(0)).max(0));
    let reasoning = match (reasoning_raw, output) {
        (Some(r), Some(o)) => Some(r.min(o)),
        (r, None) => r,
        (None, Some(_)) => None,
    };
    let context_window = json_count(info.get("model_context_window"));

    let mut numeric = BTreeMap::new();
    for (key, value) in [
        ("input_tokens", input),
        ("cached_input_tokens", cached),
        ("cache_creation_input_tokens", cache_write),
        ("output_tokens", output),
        ("reasoning_tokens", reasoning_raw),
        ("total_tokens", total),
        ("model_context_window", context_window),
    ] {
        if let Some(value) = value {
            numeric.insert(key.to_string(), value);
        }
    }
    let Ok(numeric_usage) = NumericUsagePayload::new(numeric) else {
        return LineOutcome::Skipped;
    };

    let native_session_id = if state.session_id.is_empty() {
        file_stem.to_string()
    } else {
        state.session_id.clone()
    };
    let parsed = ParsedUsage {
        native_record_id: format!("{file_fingerprint}:{line_no}"),
        native_session_id: Some(native_session_id),
        parent_native_session_id: state.parent_session_id.clone(),
        session_type: state.session_type.clone(),
        occurred_at,
        model: Some(state.model.clone()),
        provider: PROVIDER.into(),
        input_semantics: "inclusive",
        output_semantics: "delta",
        usage: TokenUsage {
            total_input_tokens: input,
            uncached_input_tokens: uncached,
            cache_read_tokens: cached,
            cache_write_tokens: cache_write,
            output_tokens: output,
            reasoning_tokens: reasoning,
            tool_use_tokens: None,
            provider_reported_total_tokens: total,
            exact_total_formula: ExactTotalFormula::InputIncludesCachePlusOutput,
        },
        numeric_usage,
        reported_cost_microusd: None,
        project_path: state.cwd.clone(),
    };
    if !parsed.has_tokens() {
        return LineOutcome::Skipped;
    }
    LineOutcome::Record(parsed)
}

#[cfg(test)]
pub(crate) mod fixtures {
    pub fn session_meta(ts: &str, id: &str, extra: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{ts}","cwd":"/Users/x/proj","originator":"codex-tui","cli_version":"0.131.0",{extra}"source":"cli","base_instructions":{{"text":"SECRET INSTRUCTIONS"}}}}}}"#
        )
    }

    pub fn forked_session_meta(ts: &str, id: &str, parent: &str) -> String {
        session_meta(
            ts,
            id,
            &format!(r#""forked_from_id":"{parent}","session_id":"{parent}","#),
        )
    }

    pub fn turn_context(ts: &str, model: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"turn_context","payload":{{"turn_id":"t","cwd":"/Users/x/proj","model":"{model}","approval_policy":"on-request"}}}}"#
        )
    }

    pub fn token_count(ts: &str, input: i64, cached: i64, output: i64, reasoning: i64) -> String {
        let total = input + output;
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":999999,"cached_input_tokens":0,"output_tokens":999,"reasoning_output_tokens":0,"total_tokens":1000998}},"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":{reasoning},"total_tokens":{total}}},"model_context_window":258400}},"rate_limits":{{"limit_id":"codex"}}}}}}"#
        )
    }

    pub fn null_token_count(ts: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{{"limit_id":"codex"}}}}}}"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn feed(lines: &[String]) -> (Vec<ParsedUsage>, usize, CodexScanState) {
        let mut state = CodexScanState::default();
        let mut records = Vec::new();
        let mut skipped = 0;
        for (index, line) in lines.iter().enumerate() {
            match parse_line(line, &mut state, "fp", "rollout-stem", index as u64 + 1) {
                LineOutcome::Record(record) => records.push(record),
                LineOutcome::Skipped => skipped += 1,
                LineOutcome::Ignored => {}
            }
        }
        (records, skipped, state)
    }

    #[test]
    fn model_is_carried_from_turn_context_and_input_is_cache_inclusive() {
        let (records, _, _) = feed(&[
            session_meta("2026-05-27T07:09:16.540Z", "sess-a", ""),
            null_token_count("2026-05-27T07:09:16.752Z"),
            turn_context("2026-05-27T07:09:16.542Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:23.814Z", 19707, 3456, 312, 40),
        ]);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(record.native_session_id.as_deref(), Some("sess-a"));
        assert_eq!(record.native_record_id, "fp:4");
        assert_eq!(record.usage.total_input_tokens, Some(19707));
        assert_eq!(record.usage.uncached_input_tokens, Some(19707 - 3456));
        assert_eq!(record.usage.cache_read_tokens, Some(3456));
        assert_eq!(record.usage.output_tokens, Some(312));
        assert_eq!(record.usage.reasoning_tokens, Some(40));
        assert_eq!(record.usage.provider_reported_total_tokens, Some(20019));
        assert_eq!(record.usage.exact_total(), Some(20019));
        let json = record.numeric_usage.to_json().unwrap();
        assert!(json.contains("\"model_context_window\":258400"));
        assert!(!json.contains("SECRET"));
        assert_eq!(record.project_path.as_deref(), Some("/Users/x/proj"));
    }

    #[test]
    fn uncached_input_never_goes_negative_and_reasoning_is_clamped() {
        let (records, _, _) = feed(&[
            turn_context("2026-05-27T07:09:16Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:23Z", 100, 150, 10, 50),
        ]);
        assert_eq!(records[0].usage.uncached_input_tokens, Some(0));
        assert_eq!(records[0].usage.reasoning_tokens, Some(10));
        // The raw payload keeps the provider's number.
        assert!(records[0]
            .numeric_usage
            .to_json()
            .unwrap()
            .contains("\"reasoning_tokens\":50"));
    }

    #[test]
    fn token_count_before_turn_context_is_skipped_without_consuming_signature() {
        let (records, skipped, _) = feed(&[
            token_count("2026-05-27T07:09:23Z", 100, 0, 10, 0),
            turn_context("2026-05-27T07:09:24Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:25Z", 100, 0, 10, 0),
        ]);
        assert_eq!(records.len(), 1, "the re-emitted copy must be counted");
        assert_eq!(skipped, 1);
    }

    #[test]
    fn identical_consecutive_last_usage_is_dropped() {
        let (records, skipped, _) = feed(&[
            turn_context("2026-05-27T07:09:16Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:23Z", 100, 0, 10, 0),
            token_count("2026-05-27T07:09:24Z", 100, 0, 10, 0),
            token_count("2026-05-27T07:09:25Z", 200, 0, 10, 0),
            token_count("2026-05-27T07:09:26Z", 100, 0, 10, 0),
        ]);
        assert_eq!(records.len(), 3);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn fork_burst_is_suppressed_until_a_real_turn() {
        let (records, skipped, state) = feed(&[
            forked_session_meta("2026-07-27T06:25:22.527Z", "child", "parent"),
            turn_context("2026-07-27T06:25:22.530Z", "gpt-5.5"),
            token_count("2026-07-27T06:25:22.540Z", 100, 0, 10, 0),
            token_count("2026-07-27T06:25:22.560Z", 200, 0, 20, 0),
            token_count("2026-07-27T06:25:22.900Z", 300, 0, 30, 0),
            token_count("2026-07-27T06:25:29.000Z", 400, 0, 40, 0),
        ]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].usage.total_input_tokens, Some(400));
        assert_eq!(
            records[0].parent_native_session_id.as_deref(),
            Some("parent")
        );
        assert_eq!(records[0].session_type.as_deref(), Some("fork"));
        assert_eq!(records[0].native_session_id.as_deref(), Some("child"));
        assert_eq!(skipped, 3);
        assert!(!state.suppressing_fork_copies);
    }

    #[test]
    fn subagent_spawn_is_treated_as_a_fork() {
        let meta = session_meta(
            "2026-07-27T06:25:22.527Z",
            "child",
            r#""source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent","depth":1}}},"thread_source":"subagent","#,
        )
        .replace(r#","source":"cli""#, "");
        let (records, _, _) = feed(&[
            meta,
            turn_context("2026-07-27T06:25:22.530Z", "gpt-5.5"),
            token_count("2026-07-27T06:25:22.540Z", 100, 0, 10, 0),
            token_count("2026-07-27T06:25:29.000Z", 400, 0, 40, 0),
        ]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_type.as_deref(), Some("subagent"));
        assert_eq!(
            records[0].parent_native_session_id.as_deref(),
            Some("parent")
        );
    }

    #[test]
    fn session_meta_after_the_first_is_ignored() {
        let (records, _, _) = feed(&[
            session_meta("2026-05-27T07:09:16Z", "mine", ""),
            session_meta("2026-05-27T07:09:16Z", "ancestor", ""),
            turn_context("2026-05-27T07:09:17Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:23Z", 100, 0, 10, 0),
        ]);
        assert_eq!(records[0].native_session_id.as_deref(), Some("mine"));
    }

    #[test]
    fn session_falls_back_to_file_stem_without_session_meta() {
        let (records, _, _) = feed(&[
            turn_context("2026-05-27T07:09:17Z", "gpt-5.5"),
            token_count("2026-05-27T07:09:23Z", 100, 0, 10, 0),
        ]);
        assert_eq!(
            records[0].native_session_id.as_deref(),
            Some("rollout-stem")
        );
    }

    #[test]
    fn total_token_usage_alone_never_yields_a_record() {
        let line = r#"{"timestamp":"2026-05-27T07:09:23Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"output_tokens":5,"total_tokens":10}}}}"#;
        let (records, _, _) = feed(&[
            turn_context("2026-05-27T07:09:17Z", "gpt-5.5"),
            line.to_string(),
        ]);
        assert!(records.is_empty());
    }
}

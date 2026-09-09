//! Claude Code transcripts: `~/.claude/projects/**/*.jsonl`.
//!
//! Every assistant content block is written as its own record and each one
//! repeats the parent message's complete `usage`, so the same `message.id`
//! recurs several times per turn. Summing those overcounts by more than 2x;
//! the first record per `message.id:requestId` wins and the rest are skipped.

use super::scan::LineOutcome;
use super::{
    json_count, location_fingerprint, microusd_from_usd, parse_rfc3339, ParsedUsage, SourceEnv,
};
use crate::analytics::{
    DiscoveredAnalyticsSource, ExactTotalFormula, ImporterCapability, NumericUsagePayload,
    TokenUsage,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const AGENT: &str = "claude";
pub const PROVIDER: &str = "anthropic";

pub fn discover(env: &SourceEnv) -> DiscoveredAnalyticsSource {
    let location = env.claude_projects_dir();
    let (coverage, reason) = super::codex::jsonl_root_state(&location, "Claude Code");
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

/// Parses one transcript line. Only `type: "assistant"` records with a
/// `message.usage` object carry usage; everything else is ignored.
pub(crate) fn parse_line(line: &str) -> LineOutcome {
    if !line.contains("\"usage\"") {
        return LineOutcome::Ignored;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return LineOutcome::Ignored;
    };
    if record.get("type").and_then(Value::as_str) != Some("assistant") {
        return LineOutcome::Ignored;
    }
    let Some(message) = record.get("message").filter(|m| m.is_object()) else {
        return LineOutcome::Ignored;
    };
    let Some(usage) = message.get("usage").filter(|u| u.is_object()) else {
        return LineOutcome::Ignored;
    };
    let Some(occurred_at) = parse_rfc3339(record.get("timestamp")) else {
        return LineOutcome::Skipped;
    };
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty());
    let Some(model) = model else {
        return LineOutcome::Skipped;
    };
    if model == "<synthetic>" {
        return LineOutcome::Skipped;
    }

    let message_id = message.get("id").and_then(Value::as_str);
    let request_id = record.get("requestId").and_then(Value::as_str);
    let native_record_id = match (message_id, request_id) {
        (None, None) => match record.get("uuid").and_then(Value::as_str) {
            Some(uuid) if !uuid.is_empty() => uuid.to_string(),
            _ => return LineOutcome::Skipped,
        },
        (message_id, request_id) => format!(
            "{}:{}",
            message_id.unwrap_or_default(),
            request_id.unwrap_or_default()
        ),
    };

    let uncached = json_count(usage.get("input_tokens"));
    let cache_read = json_count(usage.get("cache_read_input_tokens"));
    let cache_write = json_count(usage.get("cache_creation_input_tokens"));
    let output = json_count(usage.get("output_tokens"));
    let thinking = usage
        .get("output_tokens_details")
        .and_then(|details| json_count(details.get("thinking_tokens")));
    let reasoning = match (thinking, output) {
        (Some(thinking), Some(output)) => Some(thinking.min(output)),
        (Some(thinking), None) => Some(thinking),
        (None, _) => None,
    };
    let total_input = match (uncached, cache_read, cache_write) {
        (Some(u), r, w) => Some(u + r.unwrap_or(0) + w.unwrap_or(0)),
        _ => None,
    };

    let mut numeric = BTreeMap::new();
    for (key, value) in [
        ("input_tokens", uncached),
        ("cache_read_input_tokens", cache_read),
        ("cache_creation_input_tokens", cache_write),
        ("output_tokens", output),
        ("reasoning_tokens", reasoning),
    ] {
        if let Some(value) = value {
            numeric.insert(key.to_string(), value);
        }
    }
    let Ok(numeric_usage) = NumericUsagePayload::new(numeric) else {
        return LineOutcome::Skipped;
    };

    let is_sidechain = record
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // A sidechain record's `sessionId` is already the parent session; the
    // subagent has no session of its own, so its usage lands on the parent.
    let native_session_id = record
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let parsed = ParsedUsage {
        native_record_id,
        native_session_id,
        parent_native_session_id: None,
        session_type: is_sidechain.then(|| "sidechain".to_string()),
        occurred_at,
        model: Some(model.to_string()),
        provider: PROVIDER.into(),
        input_semantics: "exclusive",
        output_semantics: "delta",
        usage: TokenUsage {
            total_input_tokens: total_input,
            uncached_input_tokens: uncached,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            output_tokens: output,
            reasoning_tokens: reasoning,
            tool_use_tokens: None,
            provider_reported_total_tokens: None,
            exact_total_formula: ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput,
        },
        numeric_usage,
        reported_cost_microusd: record
            .get("costUSD")
            .and_then(Value::as_f64)
            .and_then(microusd_from_usd),
        project_path: record
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    if !parsed.has_tokens() {
        return LineOutcome::Skipped;
    }
    LineOutcome::Record(parsed)
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// One assistant record in the shape Claude Code 2.x writes. `content` is
    /// deliberately a placeholder: the importer must never read it.
    pub fn assistant_line(
        session_id: &str,
        message_id: &str,
        request_id: &str,
        model: &str,
        timestamp: &str,
        input: i64,
        cache_write: i64,
        cache_read: i64,
        output: i64,
        extra: &str,
    ) -> String {
        format!(
            concat!(
                r#"{{"parentUuid":"p","isSidechain":false,"cwd":"/Users/x/proj","sessionId":"{session}","version":"2.1.261","type":"assistant","uuid":"u-{message}-{request}","timestamp":"{ts}","requestId":"{request}",{extra}"#,
                r#""message":{{"id":"{message}","type":"message","role":"assistant","model":"{model}","content":[{{"type":"text","text":"SECRET PROMPT TEXT"}}],"usage":{{"input_tokens":{input},"cache_creation_input_tokens":{cw},"cache_read_input_tokens":{cr},"output_tokens":{output},"output_tokens_details":{{"thinking_tokens":5}},"service_tier":"standard"}}}}}}"#
            ),
            session = session_id,
            message = message_id,
            request = request_id,
            model = model,
            ts = timestamp,
            input = input,
            cw = cache_write,
            cr = cache_read,
            output = output,
            extra = extra,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::assistant_line;
    use super::*;

    fn record(line: &str) -> ParsedUsage {
        match parse_line(line) {
            LineOutcome::Record(record) => record,
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn assistant_records_yield_exclusive_input_buckets() {
        let line = assistant_line(
            "sess-1",
            "msg_1",
            "req_1",
            "claude-sonnet-5",
            "2026-08-24T11:32:03.531Z",
            2,
            22203,
            26742,
            725,
            "",
        );
        let parsed = record(&line);
        assert_eq!(parsed.native_record_id, "msg_1:req_1");
        assert_eq!(parsed.native_session_id.as_deref(), Some("sess-1"));
        assert_eq!(parsed.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(parsed.usage.uncached_input_tokens, Some(2));
        assert_eq!(parsed.usage.cache_write_tokens, Some(22203));
        assert_eq!(parsed.usage.cache_read_tokens, Some(26742));
        assert_eq!(parsed.usage.output_tokens, Some(725));
        assert_eq!(parsed.usage.reasoning_tokens, Some(5));
        assert_eq!(parsed.usage.total_input_tokens, Some(2 + 22203 + 26742));
        assert_eq!(parsed.usage.exact_total(), Some(2 + 22203 + 26742 + 725));
        assert_eq!(parsed.reported_cost_microusd, None);
        assert_eq!(parsed.occurred_at, "2026-08-24T11:32:03.531+00:00");
        let json = parsed.numeric_usage.to_json().unwrap();
        assert!(!json.contains("SECRET"));
    }

    #[test]
    fn non_assistant_lines_and_synthetic_models_are_not_records() {
        assert!(matches!(
            parse_line(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#),
            LineOutcome::Ignored
        ));
        assert!(matches!(
            parse_line(r#"{"type":"user","message":{"usage":{"input_tokens":1}}}"#),
            LineOutcome::Ignored
        ));
        let synthetic = assistant_line(
            "s",
            "m",
            "r",
            "<synthetic>",
            "2026-08-24T11:32:03Z",
            1,
            0,
            0,
            1,
            "",
        );
        assert!(matches!(parse_line(&synthetic), LineOutcome::Skipped));
        assert!(matches!(parse_line("not json"), LineOutcome::Ignored));
    }

    #[test]
    fn cost_usd_when_present_becomes_provider_reported_microusd() {
        let line = assistant_line(
            "s",
            "m",
            "r",
            "claude-opus-4-1",
            "2026-08-24T11:32:03Z",
            10,
            0,
            0,
            20,
            r#""costUSD":0.0123456,"#,
        );
        assert_eq!(record(&line).reported_cost_microusd, Some(12_346));
    }

    #[test]
    fn sidechain_records_attribute_to_the_parent_session() {
        let line = assistant_line(
            "parent-sess",
            "m",
            "r",
            "claude-haiku-4-5",
            "2026-08-07T19:37:09.055Z",
            10,
            0,
            0,
            20,
            r#""agentId":"aa5ebfe","attributionAgent":"workflow-subagent","#,
        )
        .replace(r#""isSidechain":false"#, r#""isSidechain":true"#);
        let parsed = record(&line);
        assert_eq!(parsed.native_session_id.as_deref(), Some("parent-sess"));
        assert_eq!(parsed.session_type.as_deref(), Some("sidechain"));
    }

    #[test]
    fn falls_back_to_uuid_when_neither_id_exists() {
        let line = assistant_line(
            "s",
            "m",
            "r",
            "claude-sonnet-5",
            "2026-08-24T11:32:03Z",
            10,
            0,
            0,
            20,
            "",
        )
        .replace(r#""requestId":"r","#, "")
        .replace(r#""id":"m","#, "");
        assert_eq!(record(&line).native_record_id, "u-m-r");
    }

    #[test]
    fn zero_token_records_are_skipped() {
        let line = assistant_line(
            "s",
            "m",
            "r",
            "claude-sonnet-5",
            "2026-08-24T11:32:03Z",
            0,
            0,
            0,
            0,
            "",
        );
        assert!(matches!(parse_line(&line), LineOutcome::Skipped));
    }
}

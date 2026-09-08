//! Per-request usage records, normalized from each provider's `usage.updated`
//! event.
//!
//! Every harness describes a request differently — Codex reports a cumulative
//! thread counter beside the last request, Claude folds a multi-model turn into
//! one `result`, OpenCode puts tokens and cost on a `step-finish` part, ACP
//! agents send a context gauge — and the ledger wants one row per request per
//! model with mutually exclusive token buckets. This module is that mapping.
//! It never invents a figure: a bucket a provider did not report stays `None`,
//! and the documented cache-inclusion formula is applied per provider through
//! [`ExactTotalFormula`], never a generic one.

use crate::analytics::{ExactTotalFormula, TokenUsage};
use crate::policy::UsageReport;
use crate::usage_pricing::usd_to_microusd;
use serde_json::Value;

/// Which normalizer wrote the event, read off the ledger `source`
/// (`provider.<adapter id>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageProvider {
    Codex,
    Claude,
    OpenCode,
    /// Cursor, Grok, and any other Agent Client Protocol agent.
    Acp,
    Other,
}

impl UsageProvider {
    pub fn from_source(source: &str) -> Self {
        match source.strip_prefix("provider.").unwrap_or(source) {
            "codex" => Self::Codex,
            "claude" => Self::Claude,
            "opencode" => Self::OpenCode,
            "cursor" | "grok" => Self::Acp,
            _ => Self::Other,
        }
    }
}

/// One request's usage for one model. Tokens follow the provider's own
/// semantics, recorded in `tokens.exact_total_formula`; `uncached_input` is
/// always the exclusive figure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageRecord {
    /// The model this record is for when the provider names one; `None`
    /// means the session's model of record applies.
    pub model: Option<String>,
    /// The model that actually served the request when it differed.
    pub serving_model: Option<String>,
    pub tokens: TokenUsage,
    pub context_window_tokens: Option<i64>,
    pub context_used_tokens: Option<i64>,
    pub context_percent: Option<i64>,
    pub runtime_ms: Option<i64>,
    /// A cost the provider itself reported, in micro-USD.
    pub reported_cost_microusd: Option<i64>,
    pub provider_record_id: Option<String>,
}

/// Normalize a `usage.updated` payload into ledger-ready records. An empty
/// vector means the event carried nothing worth a row.
pub fn normalize(source: &str, data: &Value) -> Vec<UsageRecord> {
    match UsageProvider::from_source(source) {
        UsageProvider::Codex => normalize_codex(data).into_iter().collect(),
        UsageProvider::Claude => normalize_claude(data),
        UsageProvider::OpenCode => normalize_opencode(data).into_iter().collect(),
        UsageProvider::Acp => normalize_acp(data).into_iter().collect(),
        UsageProvider::Other => normalize_generic(data).into_iter().collect(),
    }
}

fn integer(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| value.get(*key)?.as_i64())
}

fn decimal(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| value.get(*key)?.as_f64())
}

fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Input that already includes the cached portion (Codex, OpenAI-style,
/// OpenCode): the exclusive figure is what is left after both cache buckets,
/// never negative. Reasoning is a subset of output and is clamped to it.
fn cache_inclusive_tokens(
    input: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    output: Option<i64>,
    reasoning: Option<i64>,
) -> TokenUsage {
    TokenUsage {
        total_input_tokens: input,
        uncached_input_tokens: input.map(|input| {
            input
                .saturating_sub(cache_read.unwrap_or(0))
                .saturating_sub(cache_write.unwrap_or(0))
                .max(0)
        }),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        output_tokens: output,
        reasoning_tokens: clamp_reasoning(reasoning, output),
        tool_use_tokens: None,
        provider_reported_total_tokens: None,
        exact_total_formula: ExactTotalFormula::InputIncludesCachePlusOutput,
    }
}

/// Anthropic input excludes both cache buckets, so it *is* the exclusive
/// figure and the total is the sum of all three.
fn anthropic_tokens(
    input: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    output: Option<i64>,
) -> TokenUsage {
    let uncached = input.map(|input| input.max(0));
    TokenUsage {
        total_input_tokens: uncached.map(|uncached| {
            uncached + cache_read.unwrap_or(0).max(0) + cache_write.unwrap_or(0).max(0)
        }),
        uncached_input_tokens: uncached,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        output_tokens: output,
        reasoning_tokens: None,
        tool_use_tokens: None,
        provider_reported_total_tokens: None,
        exact_total_formula: ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput,
    }
}

fn clamp_reasoning(reasoning: Option<i64>, output: Option<i64>) -> Option<i64> {
    let reasoning = reasoning?.max(0);
    Some(match output {
        Some(output) => reasoning.min(output.max(0)),
        None => reasoning,
    })
}

fn has_token_figures(tokens: &TokenUsage) -> bool {
    tokens.total_input_tokens.is_some()
        || tokens.uncached_input_tokens.is_some()
        || tokens.cache_read_tokens.is_some()
        || tokens.cache_write_tokens.is_some()
        || tokens.output_tokens.is_some()
}

/// Codex `thread/tokenUsage/updated`, already reduced by the normalizer to the
/// `last` slice under `usage`. A frame with only the cumulative `total` has an
/// empty `usage` and yields nothing.
fn normalize_codex(data: &Value) -> Option<UsageRecord> {
    let usage = data.get("usage")?.as_object()?;
    let usage = Value::Object(usage.clone());
    let tokens = cache_inclusive_tokens(
        integer(&usage, &["input_tokens"]),
        integer(&usage, &["cache_read_tokens"]),
        integer(&usage, &["cache_write_tokens"]),
        integer(&usage, &["output_tokens"]),
        integer(&usage, &["reasoning_tokens"]),
    );
    if !has_token_figures(&tokens) {
        return None;
    }
    Some(UsageRecord {
        model: None,
        serving_model: string(data, &["servingModel"]),
        tokens,
        context_window_tokens: data
            .pointer("/tokenUsage/modelContextWindow")
            .and_then(Value::as_i64),
        context_used_tokens: None,
        context_percent: None,
        runtime_ms: None,
        reported_cost_microusd: None,
        provider_record_id: None,
    })
}

/// Claude `result`: one record per `modelUsage` entry, or one from the
/// aggregate `usage` when the per-model split is absent. The turn cost is
/// attributed exactly once — through each entry's own `costUSD` when every
/// entry carries one, otherwise on the first entry with the others marked as
/// covered by that figure (a reported zero rather than an unpriced gap, so
/// the table never prices what the provider already billed).
fn normalize_claude(data: &Value) -> Vec<UsageRecord> {
    let turn_cost = decimal(data, &["totalCostUsd", "total_cost_usd"]).and_then(usd_to_microusd);
    let runtime_ms = integer(data, &["durationMs", "duration_ms", "runtime_ms", "runtimeMs"]);
    let provider_record_id = match (
        string(data, &["messageId", "message_id"]),
        string(data, &["requestId", "request_id"]),
    ) {
        (Some(message_id), Some(request_id)) => Some(format!("{message_id}:{request_id}")),
        _ => None,
    };

    let per_model: Vec<(String, &Value)> = data
        .get("modelUsage")
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter(|(model, entry)| !model.trim().is_empty() && entry.is_object())
                .map(|(model, entry)| (model.trim().to_owned(), entry))
                .collect()
        })
        .unwrap_or_default();

    if per_model.is_empty() {
        let Some(usage) = data.get("usage").filter(|usage| usage.is_object()) else {
            return Vec::new();
        };
        let tokens = anthropic_tokens(
            integer(usage, &["input_tokens", "inputTokens"]),
            integer(usage, &["cache_read_input_tokens", "cacheReadInputTokens", "cache_read_tokens"]),
            integer(usage, &["cache_creation_input_tokens", "cacheCreationInputTokens", "cache_write_tokens"]),
            integer(usage, &["output_tokens", "outputTokens"]),
        );
        if !has_token_figures(&tokens) && turn_cost.is_none() {
            return Vec::new();
        }
        return vec![UsageRecord {
            model: None,
            serving_model: None,
            tokens,
            context_window_tokens: None,
            context_used_tokens: None,
            context_percent: None,
            runtime_ms,
            reported_cost_microusd: turn_cost,
            provider_record_id,
        }];
    }

    let every_entry_priced = per_model
        .iter()
        .all(|(_, entry)| decimal(entry, &["costUSD", "costUsd", "cost_usd"]).is_some());
    per_model
        .into_iter()
        .enumerate()
        .map(|(index, (model, entry))| {
            let reported_cost_microusd = if every_entry_priced {
                decimal(entry, &["costUSD", "costUsd", "cost_usd"]).and_then(usd_to_microusd)
            } else if index == 0 {
                turn_cost
            } else {
                turn_cost.map(|_| 0)
            };
            UsageRecord {
                model: Some(model),
                serving_model: None,
                tokens: anthropic_tokens(
                    integer(entry, &["inputTokens", "input_tokens"]),
                    integer(entry, &["cacheReadInputTokens", "cache_read_input_tokens"]),
                    integer(entry, &["cacheCreationInputTokens", "cache_creation_input_tokens"]),
                    integer(entry, &["outputTokens", "output_tokens"]),
                ),
                context_window_tokens: integer(entry, &["contextWindow", "context_window"]),
                context_used_tokens: None,
                context_percent: None,
                runtime_ms,
                reported_cost_microusd,
                provider_record_id: provider_record_id.clone(),
            }
        })
        .collect()
}

/// OpenCode `step-finish`: `tokens` plus the step's `cost`, with the model the
/// owning message named. Input is cache-inclusive.
fn normalize_opencode(data: &Value) -> Option<UsageRecord> {
    let usage = data.get("usage").filter(|usage| usage.is_object())?;
    let tokens = cache_inclusive_tokens(
        integer(usage, &["input_tokens", "input"]),
        integer(usage, &["cache_read_tokens", "cached_input_tokens"]),
        integer(usage, &["cache_write_tokens"]),
        integer(usage, &["output_tokens", "output"]),
        integer(usage, &["reasoning_tokens", "reasoning"]),
    );
    let reported_cost_microusd = decimal(data, &["cost", "costUsd", "cost_usd"]).and_then(usd_to_microusd);
    if !has_token_figures(&tokens) && reported_cost_microusd.is_none() {
        return None;
    }
    let model = match (string(data, &["model"]), string(data, &["provider"])) {
        (Some(model), Some(provider)) if !model.contains('/') => Some(format!("{provider}/{model}")),
        (model, _) => model,
    };
    Some(UsageRecord {
        model,
        serving_model: None,
        tokens,
        context_window_tokens: None,
        context_used_tokens: None,
        context_percent: None,
        runtime_ms: None,
        reported_cost_microusd,
        provider_record_id: None,
    })
}

/// ACP `usage_update`: a context gauge, never an input/output split. The
/// normalizer has already kept `total_cost_usd` only when the agent priced in
/// USD, so any other currency arrives as `null` here and is ignored.
fn normalize_acp(data: &Value) -> Option<UsageRecord> {
    let used = data.pointer("/usage/used_tokens").and_then(Value::as_i64);
    let window = data.pointer("/usage/context_window").and_then(Value::as_i64);
    let context_percent = integer(data, &["context_percent", "contextPercent"]);
    let reported_cost_microusd = decimal(data, &["total_cost_usd", "totalCostUsd"]).and_then(usd_to_microusd);
    if used.is_none() && window.is_none() && context_percent.is_none() && reported_cost_microusd.is_none() {
        return None;
    }
    Some(UsageRecord {
        model: None,
        serving_model: None,
        tokens: TokenUsage::default(),
        context_window_tokens: window,
        context_used_tokens: used,
        context_percent,
        runtime_ms: None,
        reported_cost_microusd,
        provider_record_id: None,
    })
}

/// A harness this module does not know: the alias search the ledger has
/// always used, with the cache-inclusive formula only when a provider gave
/// an explicit uncached figure or none of the cache buckets.
fn normalize_generic(data: &Value) -> Option<UsageRecord> {
    let report = UsageReport::from_normalized(data)?;
    let tokens = match report.uncached_input_tokens {
        Some(uncached) => TokenUsage {
            total_input_tokens: report.input_tokens,
            uncached_input_tokens: Some(uncached.max(0)),
            cache_read_tokens: report.cache_read_tokens,
            cache_write_tokens: report.cache_write_tokens,
            output_tokens: report.output_tokens,
            exact_total_formula: ExactTotalFormula::NoExactFormula,
            ..TokenUsage::default()
        },
        None => TokenUsage {
            exact_total_formula: ExactTotalFormula::NoExactFormula,
            ..cache_inclusive_tokens(
                report.input_tokens,
                report.cache_read_tokens,
                report.cache_write_tokens,
                report.output_tokens,
                None,
            )
        },
    };
    Some(UsageRecord {
        model: None,
        serving_model: None,
        tokens,
        context_window_tokens: None,
        context_used_tokens: None,
        context_percent: report.context_percent,
        runtime_ms: report.runtime_ms,
        reported_cost_microusd: report.cost_microusd,
        provider_record_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent;
    use serde_json::json;

    fn usage_event(events: Vec<agent::NormalizedEvent>) -> Vec<Value> {
        events
            .into_iter()
            .filter(|event| event.kind == "usage.updated")
            .map(|event| event.data)
            .collect()
    }

    #[test]
    fn codex_last_slice_nets_out_both_cache_buckets_and_clamps_reasoning() {
        let data = usage_event(agent::normalize_codex_message(&json!({
            "method":"thread/tokenUsage/updated",
            "params":{"threadId":"t","turnId":"turn-1","tokenUsage":{
                "total":{"totalTokens":900,"inputTokens":800,"cachedInputTokens":700,"cacheWriteInputTokens":50,"outputTokens":100,"reasoningOutputTokens":40},
                "last":{"totalTokens":90,"inputTokens":80,"cachedInputTokens":70,"cacheWriteInputTokens":5,"outputTokens":10,"reasoningOutputTokens":40},
                "modelContextWindow":272000}}
        })));
        let records = normalize("provider.codex", &data[0]);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.tokens.total_input_tokens, Some(80));
        assert_eq!(record.tokens.uncached_input_tokens, Some(5));
        assert_eq!(record.tokens.cache_read_tokens, Some(70));
        assert_eq!(record.tokens.cache_write_tokens, Some(5));
        assert_eq!(record.tokens.output_tokens, Some(10));
        assert_eq!(record.tokens.reasoning_tokens, Some(10), "reasoning is clamped to output");
        assert_eq!(record.tokens.exact_total_formula, ExactTotalFormula::InputIncludesCachePlusOutput);
        assert_eq!(record.tokens.exact_total(), Some(90));
        assert_eq!(record.context_window_tokens, Some(272_000));
        assert_eq!(record.reported_cost_microusd, None);

        // Cached figures larger than the input never go negative.
        let over = normalize(
            "provider.codex",
            &json!({"usage":{"input_tokens":3,"cache_read_tokens":4,"cache_write_tokens":2,"output_tokens":1}}),
        );
        assert_eq!(over[0].tokens.uncached_input_tokens, Some(0));
    }

    #[test]
    fn codex_frames_with_only_a_total_produce_no_record() {
        let data = usage_event(agent::normalize_codex_message(&json!({
            "method":"thread/tokenUsage/updated",
            "params":{"threadId":"t","turnId":"turn-1","tokenUsage":{
                "total":{"totalTokens":900,"inputTokens":800,"cachedInputTokens":700,"outputTokens":100},
                "modelContextWindow":272000}}
        })));
        assert!(normalize("provider.codex", &data[0]).is_empty());
    }

    #[test]
    fn a_rerouted_codex_model_serves_the_rest_of_that_turn_only() {
        let mut state = agent::CodexStreamState::default();
        fn usage(state: &mut agent::CodexStreamState, turn: &str) -> Value {
            usage_event(agent::normalize_codex_message_with_state(
                &json!({"method":"thread/tokenUsage/updated","params":{"threadId":"t","turnId":turn,"tokenUsage":{"last":{"inputTokens":10,"outputTokens":2}}}}),
                state,
            ))
            .remove(0)
        }
        assert_eq!(normalize("provider.codex", &usage(&mut state, "turn-1"))[0].serving_model, None);
        agent::normalize_codex_message_with_state(
            &json!({"method":"model/rerouted","params":{"threadId":"t","turnId":"turn-1","fromModel":"gpt-5","toModel":"gpt-5-mini","reason":"capacity"}}),
            &mut state,
        );
        assert_eq!(
            normalize("provider.codex", &usage(&mut state, "turn-1"))[0].serving_model.as_deref(),
            Some("gpt-5-mini")
        );
        agent::normalize_codex_message_with_state(
            &json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"turn-1","status":"completed"}}}),
            &mut state,
        );
        agent::normalize_codex_message_with_state(
            &json!({"method":"turn/started","params":{"threadId":"t","turn":{"id":"turn-2"}}}),
            &mut state,
        );
        assert_eq!(normalize("provider.codex", &usage(&mut state, "turn-2"))[0].serving_model, None);
    }

    #[test]
    fn claude_model_usage_yields_one_record_per_model_with_the_turn_cost_once() {
        let data = usage_event(agent::normalize_claude_message(&json!({
            "type":"result","subtype":"success","result":"done","duration_ms":4200,
            "usage":{"input_tokens":30,"output_tokens":15,"cache_read_input_tokens":300,"cache_creation_input_tokens":40},
            "modelUsage":{
                "claude-opus-4-6":{"inputTokens":20,"outputTokens":10,"cacheReadInputTokens":200,"cacheCreationInputTokens":30,"costUSD":0.05,"contextWindow":200000},
                "claude-haiku-4-5":{"inputTokens":10,"outputTokens":5,"cacheReadInputTokens":100,"cacheCreationInputTokens":10,"costUSD":0.01,"contextWindow":200000}
            },
            "total_cost_usd":0.06
        })));
        assert_eq!(data.len(), 1, "the aggregate usage is not a separate event");
        let records = normalize("provider.claude", &data[0]);
        assert_eq!(records.len(), 2);
        let opus = records.iter().find(|record| record.model.as_deref() == Some("claude-opus-4-6")).unwrap();
        assert_eq!(opus.tokens.uncached_input_tokens, Some(20));
        assert_eq!(opus.tokens.cache_read_tokens, Some(200));
        assert_eq!(opus.tokens.cache_write_tokens, Some(30));
        assert_eq!(opus.tokens.output_tokens, Some(10));
        assert_eq!(opus.tokens.total_input_tokens, Some(250));
        assert_eq!(opus.tokens.exact_total_formula, ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput);
        assert_eq!(opus.context_window_tokens, Some(200_000));
        assert_eq!(opus.runtime_ms, Some(4200));
        let total: i64 = records.iter().filter_map(|record| record.reported_cost_microusd).sum();
        assert_eq!(total, 60_000, "per-model costs sum to the turn cost, not twice it");

        // Without per-model cost the turn total lands on one record and the
        // rest carry a reported zero — covered, not unpriced.
        let fallback = normalize(
            "provider.claude",
            &json!({"usage":{"input_tokens":30,"output_tokens":15},"totalCostUsd":0.06,
                "modelUsage":{"claude-opus-4-6":{"inputTokens":20,"outputTokens":10},"claude-haiku-4-5":{"inputTokens":10,"outputTokens":5}}}),
        );
        let costs: Vec<Option<i64>> = fallback.iter().map(|record| record.reported_cost_microusd).collect();
        assert_eq!(costs, vec![Some(60_000), Some(0)]);
    }

    #[test]
    fn claude_without_model_usage_yields_one_record_from_usage() {
        let data = usage_event(agent::normalize_claude_message(&json!({
            "type":"result","subtype":"success","result":"done",
            "usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":2,"cache_creation_input_tokens":1},
            "total_cost_usd":0.012345
        })));
        let records = normalize("provider.claude", &data[0]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, None);
        assert_eq!(records[0].tokens.uncached_input_tokens, Some(11));
        assert_eq!(records[0].tokens.total_input_tokens, Some(14));
        assert_eq!(records[0].tokens.exact_total(), Some(19));
        assert_eq!(records[0].reported_cost_microusd, Some(12_345));
        assert_eq!(records[0].provider_record_id, None);

        let with_ids = normalize(
            "provider.claude",
            &json!({"usage":{"input_tokens":1},"messageId":"msg_1","requestId":"req_1"}),
        );
        assert_eq!(with_ids[0].provider_record_id.as_deref(), Some("msg_1:req_1"));
    }

    #[test]
    fn opencode_step_finish_carries_tokens_cost_and_the_owning_messages_model() {
        let mut state = agent::OpenCodeStreamState::default();
        agent::normalize_opencode_message_with_state(
            &json!({"type":"message.updated","properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant","modelID":"claude-opus-4-6","providerID":"anthropic"}}}),
            &mut state,
        );
        let data = usage_event(agent::normalize_opencode_message_with_state(
            &json!({"type":"message.part.updated","properties":{"part":{"id":"prt_1","messageID":"msg_1","sessionID":"ses_1","type":"step-finish",
                "tokens":{"input":40,"output":8,"reasoning":3,"cache":{"read":30,"write":5}},"cost":0.0123}}}),
            &mut state,
        ));
        assert_eq!(data.len(), 1);
        let records = normalize("provider.opencode", &data[0]);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.model.as_deref(), Some("anthropic/claude-opus-4-6"));
        assert_eq!(record.tokens.total_input_tokens, Some(40));
        assert_eq!(record.tokens.uncached_input_tokens, Some(5));
        assert_eq!(record.tokens.cache_read_tokens, Some(30));
        assert_eq!(record.tokens.cache_write_tokens, Some(5));
        assert_eq!(record.tokens.output_tokens, Some(8));
        assert_eq!(record.tokens.reasoning_tokens, Some(3));
        assert_eq!(record.reported_cost_microusd, Some(12_300));
    }

    #[test]
    fn acp_usage_updates_are_a_context_gauge_and_ignore_non_usd_costs() {
        let usd = normalize(
            "provider.cursor",
            &json!({"usage":{"used_tokens":12,"context_window":100},"context_percent":12,"total_cost_usd":0.5,"cost":{"amount":0.5,"currency":"USD"}}),
        );
        assert_eq!(usd.len(), 1);
        assert_eq!(usd[0].tokens, TokenUsage::default());
        assert_eq!(usd[0].context_used_tokens, Some(12));
        assert_eq!(usd[0].context_window_tokens, Some(100));
        assert_eq!(usd[0].context_percent, Some(12));
        assert_eq!(usd[0].reported_cost_microusd, Some(500_000));

        // The ACP normalizer nulls `total_cost_usd` for any other currency.
        let update: agent_client_protocol::schema::v1::SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate":"usage_update","used":12,"size":100,"cost":{"amount":3.0,"currency":"EUR"}
        }))
        .unwrap();
        let event = crate::acp_events::session_update_event(&update);
        assert_eq!(event.kind, "usage.updated");
        let eur = normalize("provider.grok", &event.data);
        assert_eq!(eur[0].reported_cost_microusd, None);
        assert_eq!(eur[0].context_percent, Some(12));
    }
}

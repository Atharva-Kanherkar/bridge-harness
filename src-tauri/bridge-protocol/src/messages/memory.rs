use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Named account scope. Never SQL NULL; never inferred from a missing workspace.
pub const ACCOUNT_MEMORY_SCOPE: &str = "account:local";
pub const MAX_MEMORY_BODY_CHARS: usize = 4000;
pub const MAX_MEMORY_LIST_LIMIT: u32 = 50;
/// The kind vocabulary, in the order surfaces present it.
pub const MEMORY_KINDS: [&str; 4] = ["preference", "fact", "decision", "constraint"];

/// Explicit pin. Scope is always written as `account:local` by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMemoryRecordParams {
    pub body: String,
    /// `preference` (default), `fact`, `decision`, or `constraint`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional session that originated the slash/command. Not a scope key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMemoryRecordsParams {
    pub scope_key: String,
    /// `active` (default) or `proposed`. Nothing else lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteMemoryRecordParams {
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub scope_key: String,
    pub kind: String,
    pub body: String,
    pub provenance: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    /// Written only by producers that can mean it (the extractor). An explicit
    /// save keeps it NULL, and surfaces render unknown — never a fake number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_bps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Set when this record replaced an earlier one through supersede.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The instant this record's claim began to hold. Validity is an interval,
    /// not a flag: a record that never reached active carries an empty one,
    /// where `validTo` equals `validFrom`.
    pub valid_from: String,
    /// The instant the claim stopped holding. Absent while the record is
    /// active; a supersession, a tombstone, or an expiry closes it, and a
    /// closed record stays queryable as history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    /// When set, the instant after which the record stops applying. The sweep
    /// closes the interval at this instant rather than at the moment it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Records making competing claims about one subject share this key. At
    /// most one member of a group is active, so a packet can never carry two.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_group: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Read a scope as it stood at an instant: exactly the records whose validity
/// interval contains it. As of now this is the active set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMemoryRecordsAsOfParams {
    pub scope_key: String,
    /// RFC 3339. The instant to read the scope at.
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryRecordsResult {
    pub scope_key: String,
    pub records: Vec<MemoryRecord>,
}

/// Edit is supersession: a new record replaces the old, which stays history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupersedeMemoryRecordParams {
    pub record_id: String,
    pub body: String,
    /// Inherited from the superseded record when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveMemoryRecordParams {
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectMemoryRecordParams {
    pub record_id: String,
}

/// Extraction has three account-local modes. `propose` is the safe default and
/// queues every gated candidate for review. `remember` stops future extraction;
/// new memory is then saved only explicitly, while existing active extracted
/// records remain active. `auto_apply` runs the same extractor but promotes
/// only candidates at or above 90% confidence grounded in a cited, visible user
/// message and cleared by the deterministic stability, tombstone, and conflict
/// guards. Other validated candidates that fit the scope budget remain proposed
/// for review; invalid, unsafe, duplicate, or over-budget output is refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateExtractionSettingsParams {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractionRun {
    pub status: String,
    pub proposal_count: i64,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractionSettings {
    pub scope_key: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The newest run in this scope, spend observed — never a simulated zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<MemoryExtractionRun>,
}

/// Consolidation is opt-in per scope. `off` is the default and means nothing
/// automatic; `propose` runs the pinned profile once a scope has been quiet
/// for the debounce window. The job is bookkeeping rather than judgement, so
/// the profile it points at is deliberately the user's choice of a cheap one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConsolidationSettingsParams {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How many records the scope may hold before every write is refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_records: Option<i64>,
    /// Whether a proposal may ask for removal. Off by default; with it off an
    /// operation asking for removal is refused rather than downgraded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_removal: Option<bool>,
    /// How long a scope must stay quiet before a queued run becomes due.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConsolidationRun {
    /// One of `queued`, `running`, `completed`, `failed`, or `cancelled`.
    pub status: String,
    pub applied_count: i64,
    pub refused_count: i64,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConsolidationSettings {
    pub scope_key: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub max_records: i64,
    pub allow_removal: bool,
    pub debounce_seconds: i64,
    /// What the scope currently holds against `maxRecords`. A write that would
    /// take this past the budget is refused; nothing is ever evicted for it.
    pub held_records: i64,
    /// The newest settled run in this scope, spend observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<MemoryConsolidationRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInjectionSettings {
    pub scope_key: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMemoryInjectionParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPacketAuditParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPacketItem {
    pub record_id: String,
    pub body: String,
    pub kind: String,
    pub reason: String,
}

/// The newest retrieval audit for a session. An empty selection means the
/// session started without a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPacketAudit {
    pub session_id: String,
    pub selected: Vec<MemoryPacketItem>,
    pub token_estimate: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// One record's recall history over the last 14 days of packet audits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallStat {
    pub id: String,
    pub recalls: i64,
    /// Day bucket of the last recall (0 = 13 days ago, 13 = today); -1 if none.
    pub last_recalled_day: i64,
    /// recalls / packets built in the window.
    pub in_packet_ratio: f64,
    /// 14-day recall series, oldest first.
    pub daily: Vec<i64>,
}

/// How many packets left a record out for one reason, as the audit coded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExclusionCount {
    pub code: String,
    pub count: i64,
}

/// Active records of one kind and how many of them reached a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryKindUse {
    pub kind: String,
    pub active: i64,
    /// Active records of this kind recalled at least once in the window.
    pub recalled: i64,
    pub recalls: i64,
}

/// Recall analytics for the account scope, folded from the retrieval audits.
/// It measures delivery (what reached a prompt), not whether the model used it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallStats {
    pub per_record: Vec<MemoryRecallStat>,
    /// Packets built per day bucket, oldest first, 14 entries.
    pub injections_per_day: Vec<i64>,
    pub budget_chars_used: i64,
    pub budget_chars_max: i64,
    /// Packets built in the window, and how many carried at least one memory.
    pub packets: i64,
    pub packets_with_memories: i64,
    pub active_records: i64,
    /// Active records recalled at least once in the window.
    pub recalled_records: i64,
    pub by_kind: Vec<MemoryKindUse>,
    pub exclusions: Vec<MemoryExclusionCount>,
}

/// One settled extraction or consolidation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryActivityEntry {
    /// `extraction` or `consolidation`.
    pub source: String,
    pub status: String,
    /// Proposals written (extraction) or changes applied (consolidation).
    pub applied: i64,
    pub refused: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub at: String,
}

/// Settled runs, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryActivityLog {
    pub entries: Vec<MemoryActivityEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetInsightsParams {
    /// Run the analysis again. Without it only the stored report is read, so
    /// opening the tab never sends memory bodies to a provider.
    #[serde(default)]
    pub refresh: bool,
}

/// The Memory Insights report. `stats` and `kinds` are Bridge's own figures;
/// the prose is the model's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInsightsReport {
    pub headline: String,
    pub summary: String,
    pub highlights: Vec<crate::messages::UsageInsightHighlight>,
    pub themes: Vec<crate::messages::UsageInsightTheme>,
    pub recommendations: Vec<String>,
    pub stats: MemoryRecallStats,
    pub memories_analysed: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInsightsResult {
    pub status: crate::messages::UsageInsightsStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<MemoryInsightsReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What memory exists, so surfaces can be honest about what they do not own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCapabilities {
    pub ledger: MemoryLedgerCapability,
    /// Provider-owned memory commands currently reachable through the slash
    /// catalog. An unavailable adapter contributes nothing.
    pub provider_native: Vec<ProviderMemoryCommand>,
}

/// The Bridge half: the local ledger's contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryLedgerCapability {
    pub exists: bool,
    pub scope_key: String,
    pub max_body_chars: u32,
    pub kinds: Vec<String>,
}

/// A provider-owned memory command in the catalog. It stays on that provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMemoryCommand {
    pub harness: String,
    pub command: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn save_and_list_refuse_unknown_fields_and_null_scope() {
        assert!(
            serde_json::from_value::<SaveMemoryRecordParams>(json!({})).is_err(),
            "body is required"
        );
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({
            "body": "pin",
            "scopeKey": "account:local"
        }))
        .is_err());
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({
            "body": "pin",
            "workspaceId": "w"
        }))
        .is_err());
        assert!(
            serde_json::from_value::<ListMemoryRecordsParams>(json!({})).is_err(),
            "scopeKey is required"
        );
        assert!(serde_json::from_value::<ListMemoryRecordsParams>(json!({
            "scopeKey": "account:local",
            "includeDeleted": true
        }))
        .is_err());
        assert!(serde_json::from_value::<SaveMemoryRecordParams>(json!({ "body": "pin" })).is_ok());
        assert!(serde_json::from_value::<ListMemoryRecordsParams>(json!({
            "scopeKey": "account:local"
        }))
        .is_ok());
    }
}

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

/// Extraction is opt-in per scope. `remember` is the default and means nothing
/// automatic; `propose` runs the pinned profile after turns. `auto_apply` does
/// not exist until a replay bench can justify it.
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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Named account scope. Never SQL NULL; never inferred from a missing workspace.
pub const ACCOUNT_MEMORY_SCOPE: &str = "account:local";
pub const MAX_MEMORY_BODY_CHARS: usize = 4000;
pub const MAX_MEMORY_LIST_LIMIT: u32 = 50;

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
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryRecordsResult {
    pub scope_key: String,
    pub records: Vec<MemoryRecord>,
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

//! External harness import: local discovery, review, and atomic commit.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportCandidateKind {
    Conversation,
    Message,
    Attachment,
    Memory,
    ProjectHint,
    Instruction,
    Rule,
    Prompt,
    Command,
    Skill,
    Agent,
    Hook,
    McpServer,
    Plugin,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportStability {
    Stable,
    VersionGated,
    Experimental,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportSourceClassification {
    Documented,
    VersionGatedPrivate,
    UnsupportedPrivate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportSource {
    pub provider: String,
    pub adapter_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub canonical_source_ref: String,
    pub source_path_fingerprint: String,
    pub discovered_at: String,
    pub source_metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportDiagnostic {
    pub code: String,
    pub severity: String,
    pub classification: ExternalImportSourceClassification,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportRedactionSummary {
    pub structured_fields_excluded: u32,
    pub text_values_redacted: u32,
    pub categories: Vec<String>,
    pub safely_representable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportArtifact {
    pub artifact_id: String,
    pub canonical_source_ref: String,
    pub source_label: String,
    pub kind: ExternalImportCandidateKind,
    pub classification: ExternalImportSourceClassification,
    pub stability: ExternalImportStability,
    pub estimated_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_schema_gate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportDiscovery {
    pub discovery_id: String,
    pub provider: String,
    pub approved_roots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    pub format_versions: BTreeMap<String, String>,
    pub artifacts: Vec<ExternalImportArtifact>,
    pub diagnostics: Vec<ExternalImportDiagnostic>,
    pub discovered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportCandidate {
    pub candidate_id: String,
    pub source: ExternalImportSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_native_id: Option<String>,
    pub kind: ExternalImportCandidateKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_hint: Option<String>,
    pub content_hash: String,
    pub stability: ExternalImportStability,
    pub confidence_bps: u16,
    pub selected_by_default: bool,
    pub redaction_summary: ExternalImportRedactionSummary,
    pub diagnostics: Vec<ExternalImportDiagnostic>,
    pub normalized_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportConflictPolicy {
    Skip,
    ImportAsNewHistoricalRevision,
    KeepExisting,
    ImportAlongside,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportSetupActivationPolicy {
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportPlan {
    pub selected_candidate_ids: Vec<String>,
    pub conflict_policy: ExternalImportConflictPolicy,
    pub setup_activation_policy: ExternalImportSetupActivationPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_scope: Option<String>,
    pub dry_run: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportCandidateStatus {
    Imported,
    SkippedDuplicate,
    ChangedSource,
    Conflicted,
    Rejected,
    Unsupported,
    DryRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportCandidateResult {
    pub candidate_id: String,
    pub status: ExternalImportCandidateStatus,
    pub created_bridge_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_of: Option<String>,
    pub diagnostics: Vec<ExternalImportDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportCommit {
    pub import_id: String,
    pub candidate_results: Vec<ExternalImportCandidateResult>,
    pub created_bridge_ids: Vec<String>,
    pub imported: u32,
    pub skipped: u32,
    pub changed: u32,
    pub conflicted: u32,
    pub rejected: u32,
    pub unsupported: u32,
    pub rollback_state: String,
    pub diagnostics: Vec<ExternalImportDiagnostic>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoverExternalImportParams {
    pub provider: String,
    pub approved_roots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_export: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub format_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewExternalImportParams {
    pub discovery: ExternalImportDiscovery,
    pub artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImportPreview {
    pub candidates: Vec<ExternalImportCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitExternalImportParams {
    pub candidates: Vec<ExternalImportCandidate>,
    pub plan: ExternalImportPlan,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn setup_activation_has_no_enabled_wire_value() {
        assert!(
            serde_json::from_value::<ExternalImportSetupActivationPolicy>(json!("enabled"))
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(ExternalImportSetupActivationPolicy::Disabled).unwrap(),
            json!("disabled")
        );
    }

    #[test]
    fn import_params_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<DiscoverExternalImportParams>(json!({
                "provider": "claude_code",
                "approvedRoots": [],
                "formatVersions": {},
                "backgroundScan": true
            }))
            .is_err()
        );
    }
}

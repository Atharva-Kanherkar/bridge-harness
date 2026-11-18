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
    Script,
    Preference,
    ProviderInstance,
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
    /// For a chat: its title, read from the first and last few KiB only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// For a chat: the folder it ran in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
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
    /// References a discovery this daemon actually walked — a client cannot
    /// substitute its own `approvedRoots`/`artifacts` by hand-building one.
    pub discovery_id: String,
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
    /// Same discovery `preview` reviewed. Commit re-derives every candidate
    /// from disk against it rather than trusting client-supplied
    /// `normalizedPayload`/`contentHash`/`candidateId` values — the source
    /// file is re-read and re-hashed at commit time, closing the
    /// discover/preview/commit TOCTOU window.
    pub discovery_id: String,
    pub plan: ExternalImportPlan,
    /// The artifacts `preview` was run on. Commit re-derives only these, so a
    /// store of 80,000 transcripts is not re-read to import ten. Absent means
    /// every artifact in the discovery, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ids: Option<Vec<String>>,
}

/// What `imports/detect_import_sources` found for one tool. Detection takes no
/// parameters and is read-only: it lists folders and counts files, and never
/// opens a database or a transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceCounts {
    pub chats: u64,
    pub projects: u64,
    pub workspaces: u64,
    pub memories: u64,
    pub setup: u64,
    /// Sessions that ran in a temporary folder: SDK probes and background
    /// runs, not chats a person had. Listed, but not selected by default.
    #[serde(default)]
    pub background: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetectedImportSource {
    /// Bundle tool id: `claude_code`, `codex`, `opencode`, `conductor`, `t3code`, `orca`, `superset`.
    pub tool: String,
    pub label: String,
    pub installed: bool,
    /// Bridge can import this tool's own data today.
    pub supported: bool,
    pub roots: Vec<String>,
    pub counts: ImportSourceCounts,
    /// One line for the person: what comes over, or why not yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourcesDetection {
    pub sources: Vec<DetectedImportSource>,
    pub detected_at: String,
}

/// `imports/start_import`: import everything Bridge can bring over from one
/// tool, in the background. Returns at once with the job; a job already
/// running is returned instead of starting a second one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartImportParams {
    /// Bundle tool id, as in `DetectedImportSource::tool`.
    pub tool: String,
}

/// Something that did not come over, with how many times.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportNotCarried {
    pub text: String,
    pub count: u64,
}

/// A background import: how far it has got and what came over so far. Sent
/// by `imports/import_status` and live as `imports/progress`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub job_id: String,
    pub tool: String,
    /// `running`, `done` or `failed`.
    pub status: String,
    /// Files the job reads: transcripts, memories and setup items.
    pub total: u64,
    pub processed: u64,
    /// Chats, memories and setup items written to Bridge.
    pub imported: u64,
    /// Memories among `imported`.
    pub memories: u64,
    pub already_in_bridge: u64,
    /// Chats imported before whose source has changed since; kept as they were.
    pub changed: u64,
    pub failed: u64,
    /// Background runs from the OS temp folder, not imported.
    pub left_out: u64,
    /// The folder of the chat being read now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_project: Option<String>,
    #[serde(default)]
    pub not_carried: Vec<ImportNotCarried>,
    #[serde(default)]
    pub errors: Vec<String>,
    pub started_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

/// `imports/import_status`: the latest import, running or finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportJobStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<ImportJob>,
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

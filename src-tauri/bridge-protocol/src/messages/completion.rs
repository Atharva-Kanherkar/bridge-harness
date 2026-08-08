//! The completion domain: the acceptance contract a session is verified
//! against, the checks that prove it, and the verifier manifests that supply
//! them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a check earns its verdict. Mirrors `bridge_core::completion::EvalKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvalKind {
    Deterministic,
    Scrutiny,
    UserTesting,
}

/// Where a check stands. Mirrors `bridge_core::completion::CheckStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
    Blocked,
    Stale,
}

/// One executed check reported back into a completion attempt. Mirrors
/// `bridge_core::completion::CheckRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub check_id: String,
    pub kind: EvalKind,
    pub required: bool,
    pub status: CheckStatus,
    pub executor: String,
    pub command: Option<String>,
    pub verifier_family: Option<String>,
    pub detail: Option<String>,
    /// Digest of the check's output; the output itself never crosses the wire.
    pub output_digest: Option<String>,
    pub artifact_refs: Vec<String>,
}

/// A registered verifier: what it checks, when it applies, and what it needs.
/// Mirrors `bridge_core::completion::VerifierManifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifierManifest {
    pub id: String,
    pub kind: EvalKind,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub different_model_family: bool,
    pub checks: Vec<String>,
    #[serde(default)]
    pub evidence_required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCompletionPlanParams {
    pub session_id: String,
    pub acceptance_criteria: Vec<String>,
    pub changed_paths: Vec<String>,
    /// Commands the repository itself declares as its verification entry
    /// points (test, lint, build).
    pub repository_commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_projection: Option<String>,
    pub markdown_committed: bool,
}

/// How far a workspace has drifted from the branch it builds on. Mirrors
/// `bridge_core::git::BaseBranchDivergence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaseBranchDivergence {
    pub base_ref: Option<String>,
    pub base_commit: Option<String>,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub ref_age_seconds: Option<i64>,
    pub fetch_attempted: bool,
    pub fetched: bool,
    pub dirty: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceBaseDivergenceParams {
    pub session_id: String,
    /// Consult the network for a fresh base ref. False measures against the last
    /// fetched ref, which the result reports.
    pub fetch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshWorkspaceBaseParams {
    pub session_id: String,
}

/// Where a worker's repository output stands. Mirrors
/// `bridge_core::worker_adoption::WorkerRepositoryBinding`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRepositoryBinding {
    pub session_id: String,
    pub parent_session_id: String,
    pub workspace_id: String,
    pub worktree_path: String,
    pub worktree_branch: String,
    pub task_worktree_path: String,
    /// `in_place`, `pending_adoption`, `adopted`, `discarded`, or `empty`.
    pub state: String,
    pub head: Option<String>,
    pub base_commit: Option<String>,
    pub base_branch: Option<String>,
    pub baseline_dirty_paths: Vec<String>,
    pub changed_paths: Vec<String>,
    pub diffstat: Option<String>,
    pub dirty: bool,
    pub detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingWorkerAdoptionsParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PendingWorkerAdoptionsResult(pub Vec<WorkerRepositoryBinding>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdoptWorkerWorktreeParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscardWorkerWorktreeParams {
    pub session_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordCompletionCheckParams {
    pub attempt_id: String,
    pub run: CheckRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaiveCompletionParams {
    pub attempt_id: String,
    /// Every unresolved required check the waiver covers; a partial list is
    /// rejected by the command.
    pub check_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterVerifierManifestParams {
    /// Where the manifest came from, for provenance in the audit trail.
    pub source: String,
    pub manifest: VerifierManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifierCandidatesParams {
    pub change_labels: Vec<String>,
    pub available_capabilities: Vec<String>,
}

/// Mirrors `bridge_core::completion::VerifierCandidate` — a registered
/// verifier's eligibility for a change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifierCandidate {
    pub manifest: VerifierManifest,
    pub eligible: bool,
    pub exclusion_reasons: Vec<String>,
}

/// `completion/verifier_candidates`' result: a bare array on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct VerifierCandidatesResult(pub Vec<VerifierCandidate>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    fn check_run() -> CheckRun {
        CheckRun {
            check_id: "cargo-test".into(),
            kind: EvalKind::Deterministic,
            required: true,
            status: CheckStatus::Passed,
            executor: "shell".into(),
            command: Some("cargo test".into()),
            verifier_family: None,
            detail: None,
            output_digest: Some("sha256:abc".into()),
            artifact_refs: vec!["artifact-1".into()],
        }
    }

    #[test]
    fn completion_plan_params_round_trip() {
        let plan = CreateCompletionPlanParams {
            session_id: "s-1".into(),
            acceptance_criteria: vec!["tests pass".into()],
            changed_paths: vec!["src/lib.rs".into()],
            repository_commands: vec!["cargo test".into()],
            markdown_projection: None,
            markdown_committed: false,
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            json!({
                "sessionId": "s-1",
                "acceptanceCriteria": ["tests pass"],
                "changedPaths": ["src/lib.rs"],
                "repositoryCommands": ["cargo test"],
                "markdownCommitted": false,
            }),
            "absent options stay off the wire"
        );
        assert_eq!(round_trip(&plan), plan);
    }

    #[test]
    fn check_runs_and_manifests_round_trip_with_snake_case_verdicts() {
        let record = RecordCompletionCheckParams { attempt_id: "a-1".into(), run: check_run() };
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["attemptId"], json!("a-1"));
        assert_eq!(wire["run"]["checkId"], json!("cargo-test"));
        assert_eq!(wire["run"]["kind"], json!("deterministic"));
        assert_eq!(wire["run"]["status"], json!("passed"));
        assert_eq!(wire["run"]["outputDigest"], json!("sha256:abc"));
        assert_eq!(round_trip(&record), record);

        let register = RegisterVerifierManifestParams {
            source: "repository".into(),
            manifest: VerifierManifest {
                id: "rust-tests".into(),
                kind: EvalKind::UserTesting,
                triggers: vec!["rust".into()],
                required_capabilities: Vec::new(),
                different_model_family: true,
                checks: vec!["cargo-test".into()],
                evidence_required: Vec::new(),
            },
        };
        let wire = serde_json::to_value(&register).unwrap();
        assert_eq!(wire["manifest"]["kind"], json!("user_testing"));
        assert_eq!(wire["manifest"]["differentModelFamily"], json!(true));
        assert_eq!(round_trip(&register), register);
    }

    #[test]
    fn manifest_list_fields_default_to_empty() {
        let manifest: VerifierManifest = serde_json::from_value(json!({
            "id": "minimal",
            "kind": "scrutiny",
            "differentModelFamily": false,
            "checks": ["one"],
        }))
        .unwrap();
        assert!(manifest.triggers.is_empty());
        assert!(manifest.required_capabilities.is_empty());
        assert!(manifest.evidence_required.is_empty());
    }

    #[test]
    fn waiver_and_candidate_params_round_trip() {
        let waive = WaiveCompletionParams {
            attempt_id: "a-1".into(),
            check_ids: vec!["cargo-test".into()],
            reason: "flaky infrastructure".into(),
        };
        assert_eq!(
            serde_json::to_value(&waive).unwrap(),
            json!({
                "attemptId": "a-1",
                "checkIds": ["cargo-test"],
                "reason": "flaky infrastructure",
            })
        );
        assert_eq!(round_trip(&waive), waive);

        let candidates = VerifierCandidatesParams {
            change_labels: vec!["rust".into()],
            available_capabilities: vec!["shell".into()],
        };
        assert_eq!(
            serde_json::to_value(&candidates).unwrap(),
            json!({"changeLabels": ["rust"], "availableCapabilities": ["shell"]})
        );
        assert_eq!(round_trip(&candidates), candidates);
    }

    #[test]
    fn completion_params_reject_incomplete_payloads() {
        assert!(serde_json::from_value::<CreateCompletionPlanParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<CreateCompletionPlanParams>(json!({
                "sessionId": "s",
                "acceptanceCriteria": [],
                "changedPaths": [],
                "repositoryCommands": [],
            }))
            .is_err(),
            "markdownCommitted is required"
        );
        assert!(serde_json::from_value::<RecordCompletionCheckParams>(json!({"attemptId": "a"}))
            .is_err());
        assert!(
            serde_json::from_value::<RecordCompletionCheckParams>(json!({
                "attemptId": "a",
                "run": {"check_id": "c", "kind": "deterministic", "required": true,
                        "status": "passed", "executor": "shell", "artifactRefs": []},
            }))
            .is_err(),
            "nested payload fields are camelCase too"
        );
        assert!(
            serde_json::from_value::<CheckRun>(json!({
                "checkId": "c", "kind": "deterministic", "required": true,
                "status": "aborted", "executor": "shell", "artifactRefs": [],
            }))
            .is_err(),
            "unknown check statuses must be rejected"
        );
        assert!(serde_json::from_value::<WaiveCompletionParams>(
            json!({"attemptId": "a", "checkIds": []})
        )
        .is_err());
        assert!(serde_json::from_value::<RegisterVerifierManifestParams>(
            json!({"source": "repository"})
        )
        .is_err());
        assert!(serde_json::from_value::<VerifierCandidatesParams>(json!({"changeLabels": []}))
            .is_err());
    }
}

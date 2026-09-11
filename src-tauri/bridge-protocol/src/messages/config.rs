//! The configuration domain: harness defaults, agent definitions, and the
//! OpenCode provider catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSettings {
    pub default_harness: Option<String>,
    pub max_concurrent_workers: usize,
    pub max_workers_per_turn: usize,
    pub stall_timeout_seconds: u64,
    pub warm_retention_minutes: i64,
    pub automatic_retry: bool,
    pub provider_failover: bool,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self { default_harness: None, max_concurrent_workers: 2, max_workers_per_turn: 3,
            stall_timeout_seconds: 600, warm_retention_minutes: 5, automatic_retry: true, provider_failover: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetWorkerSettingsParams { pub workspace_id: String }

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveWorkerSettingsParams { pub workspace_id: String, pub settings: WorkerSettings }
use serde_json::Value;

use super::common::Effort;

/// Per-harness defaults. Mirrors `bridge_core::agent_config::HarnessConfig`,
/// including its refusal of unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessConfig {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub system_prompt: String,
    /// Harness-specific settings passed through untouched.
    #[serde(default = "empty_object")]
    pub advanced: Value,
    /// Set when this config overrides a built-in default.
    #[serde(default)]
    pub is_override: bool,
}

/// A configured agent. Mirrors `bridge_core::agent_config::AgentDefinition`,
/// including its refusal of unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDefinition {
    /// Empty when creating; the server assigns the id.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub role: String,
    pub harness: String,
    #[serde(default)]
    pub model: Option<String>,
    pub effort: Effort,
    #[serde(default)]
    pub system_prompt: String,
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub is_built_in: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveHarnessConfigParams {
    pub config: HarnessConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetHarnessConfigParams {
    /// The harness id to restore to its built-in defaults.
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshOpencodeCatalogParams {
    /// Working directory whose OpenCode configuration to read; omitted uses
    /// the user-level configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetOpencodeProviderApiKeyParams {
    pub provider_id: String,
    /// Stored by OpenCode's own auth file, never persisted by Bridge.
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveOpencodeProviderAuthParams {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAgentConfigParams {
    pub agent: AgentDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAgentConfigParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetDefaultAgentParams {
    pub id: String,
}

/// How much Bridge asks before an agent acts. Mirrors
/// `bridge_core::agent_config::PermissionPolicy`.
///
/// `default` on the container, not just the fields: a policy payload written by
/// an older build must still read once slice 3 adds the graduated modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionPolicy {
    /// Auto-accept every provider approval, for every agent. Worker write scope
    /// and browser outward effects are unaffected — those are authorization.
    #[serde(alias = "bypassAll")]
    pub auto_approve_provider_permissions: bool,
    /// Allows these worker roles to propose guidance; each edit still requires
    /// a separate human approval of its exact text.
    pub worker_prompt_proposal_roles: Vec<PromptProposalWorkerRole>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptProposalWorkerRole {
    Research,
    Implementation,
    Verification,
    Planning,
    Documentation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavePermissionPolicyParams {
    pub policy: PermissionPolicy,
}

/// The configuration snapshot every config mutation returns. Mirrors
/// `bridge_core::agent_config::ConfigState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigState {
    pub harnesses: Vec<HarnessConfig>,
    pub agents: Vec<AgentDefinition>,
    pub default_agent_id: String,
    pub permission_policy: PermissionPolicy,
}

// --- Prompt Studio (#241) -----------------------------------------------------

/// Which compiled prompt stack a Prompt Studio method operates on. Wire values
/// equal the storage keys the core prompt model uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PromptTargetChoice {
    #[serde(rename = "orchestrator")]
    Orchestrator,
    #[serde(rename = "worker:research")]
    WorkerResearch,
    #[serde(rename = "worker:implementation")]
    WorkerImplementation,
    #[serde(rename = "worker:verification")]
    WorkerVerification,
    #[serde(rename = "worker:planning")]
    WorkerPlanning,
    #[serde(rename = "worker:documentation")]
    WorkerDocumentation,
    #[serde(rename = "direct_session")]
    DirectSession,
}

/// Every Prompt Studio method addresses one target; `depth` selects the worker
/// topology depth for depth-sensitive worker contracts, is ignored by
/// non-worker targets, and defaults to 0 when omitted. Each method repeats
/// these fields because params types are named after their method.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPromptStackParams {
    pub target: PromptTargetChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavePromptSectionParams {
    pub target: PromptTargetChoice,
    pub section_id: String,
    /// The replacement text; an empty override is refused — delete instead.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetPromptSectionParams {
    pub target: PromptTargetChoice,
    pub section_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestorePromptRevisionParams {
    pub target: PromptTargetChoice,
    pub section_id: String,
    /// The revision to restore; it must belong to this exact target and
    /// section, and restoring appends a new revision rather than rewinding.
    pub revision_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewCompiledPromptParams {
    pub target: PromptTargetChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
}

/// Mirrors `bridge_core::prompt_sections::PromptSectionState` exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromptSectionStatePayload {
    Default,
    Overridden { text: String },
    Deleted,
}

/// The closed mutation vocabulary of a revision. Mirrors
/// `bridge_core::prompt_sections::PromptSectionOperation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptRevisionOperation {
    Override,
    Delete,
    Reset,
    Restore,
}

/// How a provider-owned layer's byte count was obtained. Closed vocabulary —
/// mirrors `bridge_core::prompt_studio::PromptLayerSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptLayerSource {
    /// The provider reported the number itself.
    Reported,
    /// Bridge observed the exact bytes.
    Measured,
    /// A labelled approximation.
    Estimated,
    /// Nothing defensible exists.
    Unavailable,
}

/// One append-only revision of a prompt section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptRevisionView {
    pub id: i64,
    pub operation: PromptRevisionOperation,
    pub state: PromptSectionStatePayload,
    pub restored_from_revision_id: Option<i64>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<PromptRevisionAttribution>,
}

/// Host-stamped origin of an agent-proposed, human-approved prompt revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptRevisionAttribution {
    pub actor_session_id: String,
    pub actor_turn_id: String,
    pub actor_role: String,
    pub proposal_id: String,
    pub rationale: String,
}

/// A required-vocabulary warning over a section's effective text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptLintWarningView {
    pub marker: String,
    pub message: String,
}

/// One section of a resolved prompt stack. Mirrors
/// `bridge_core::prompt_studio::PromptSectionView`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptSectionView {
    pub id: String,
    pub state: PromptSectionStatePayload,
    /// The built-in text this section falls back to when its state is default.
    pub default_text: String,
    /// The text the live compiler resolves to, or null when deleted.
    pub effective_text: Option<String>,
    pub bytes: u64,
    /// Byte-derived estimate (`ceil(bytes / 4)`), labelled as an estimate.
    pub token_estimate: u64,
    pub lint_warnings: Vec<PromptLintWarningView>,
    /// Append-only history, oldest first.
    pub revisions: Vec<PromptRevisionView>,
}

/// The full studio view of one target's stack. Mirrors
/// `bridge_core::prompt_studio::PromptStackView`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptStackView {
    /// Storage key of the target (`orchestrator`, `worker:<role>`,
    /// `direct_session`) — same vocabulary the params enum accepts.
    pub target: String,
    /// Worker topology depth the stack was resolved at.
    pub depth: i64,
    pub sections: Vec<PromptSectionView>,
}

/// What a mutation changed: the revision it appended plus the fresh stack for
/// the target, so a client never mutates against a stale view. Mirrors
/// `bridge_core::prompt_studio::PromptSectionMutation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptSectionMutationResult {
    pub revision: PromptRevisionView,
    pub stack: PromptStackView,
}

/// One provider-owned layer's honest standing inside a preview. Mirrors
/// `bridge_core::prompt_studio::PromptProviderLayerStatus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptProviderLayerStatus {
    pub layer: String,
    pub adapter: String,
    pub source: PromptLayerSource,
    /// Exact byte size when actually readable; never invented.
    pub bytes: Option<u64>,
    pub detail: Option<String>,
}

/// The exact Bridge-authored envelopes for one target's resolved stack.
/// Mirrors `bridge_core::prompt_studio::CompiledPromptPreview`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompiledPromptPreviewResult {
    pub target: String,
    pub depth: i64,
    pub stack: PromptStackView,
    /// Exact `<bridge-stable-prompt …>` bytes compiled from the resolved
    /// sections alone — no project rules, tool schemas, or runtime variables.
    pub stable_prefix: String,
    /// Exact `<bridge-variable-context>` bytes with an empty sections list;
    /// runtime task/session/restoration content is never fabricated here.
    pub variable_suffix: String,
    pub schema_version: u32,
    pub prefix_id: String,
    pub prefix_hash: String,
    pub prefix_bytes: u64,
    pub prefix_token_estimate: u64,
    pub provider_layers: Vec<PromptProviderLayerStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn prompt_proposal_permissions_default_closed_and_reject_unknown_roles() {
        let legacy: PermissionPolicy = serde_json::from_value(json!({
            "autoApproveProviderPermissions": true,
            "updatedAt": "before-prompt-proposals"
        })).unwrap();
        assert!(legacy.worker_prompt_proposal_roles.is_empty());
        let policy = PermissionPolicy {
            worker_prompt_proposal_roles: vec![PromptProposalWorkerRole::Research],
            ..PermissionPolicy::default()
        };
        assert_eq!(round_trip(&policy), policy);
        assert_eq!(serde_json::to_value(&policy).unwrap()["workerPromptProposalRoles"], json!(["research"]));
        assert!(serde_json::from_value::<SavePermissionPolicyParams>(json!({
            "policy": {"workerPromptProposalRoles": ["orchestrator"]}
        })).is_err());
    }

    #[test]
    fn prompt_revision_attribution_round_trips_and_legacy_history_still_reads() {
        let mut revision: PromptRevisionView = serde_json::from_value(json!({
            "id": 1, "operation": "override", "state": {"state": "overridden", "text": "Cite sources."},
            "restoredFromRevisionId": null, "createdAt": "now"
        })).unwrap();
        assert!(revision.attribution.is_none());
        revision.attribution = Some(PromptRevisionAttribution {
            actor_session_id: "worker-session".into(),
            actor_turn_id: "turn-1".into(),
            actor_role: "research".into(),
            proposal_id: "proposal-1".into(),
            rationale: "Keep research auditable.".into(),
        });
        assert_eq!(round_trip(&revision), revision);
        assert_eq!(serde_json::to_value(&revision).unwrap()["attribution"]["actorSessionId"], "worker-session");
    }

    #[test]
    fn harness_config_round_trips_and_defaults_its_optional_fields() {
        let save = SaveHarnessConfigParams {
            config: HarnessConfig {
                id: "codex".into(),
                label: "Codex".into(),
                enabled: true,
                default_model: Some("gpt-5".into()),
                effort: Some(Effort::Xhigh),
                system_prompt: String::new(),
                advanced: json!({"sandbox": "workspace-write"}),
                is_override: true,
            },
        };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(wire["config"]["defaultModel"], json!("gpt-5"));
        assert_eq!(wire["config"]["effort"], json!("xhigh"));
        assert_eq!(wire["config"]["isOverride"], json!(true));
        assert_eq!(wire["config"]["advanced"]["sandbox"], json!("workspace-write"));
        assert_eq!(round_trip(&save), save);

        let minimal: HarnessConfig =
            serde_json::from_value(json!({"id": "shell", "label": "Shell", "enabled": false}))
                .unwrap();
        assert_eq!(minimal.default_model, None);
        assert_eq!(minimal.effort, None);
        assert_eq!(minimal.advanced, json!({}), "advanced defaults to an empty object");
        assert!(!minimal.is_override);
    }

    #[test]
    fn agent_definitions_round_trip() {
        let save = SaveAgentConfigParams {
            agent: AgentDefinition {
                id: String::new(),
                name: "Reviewer".into(),
                description: "Reviews diffs".into(),
                role: "review".into(),
                harness: "claude".into(),
                model: None,
                effort: Effort::Medium,
                system_prompt: "Be exacting.".into(),
                enabled: true,
                is_default: false,
                is_built_in: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
        };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(wire["agent"]["systemPrompt"], json!("Be exacting."));
        assert_eq!(wire["agent"]["isBuiltIn"], json!(false));
        assert_eq!(round_trip(&save), save);

        let created: AgentDefinition = serde_json::from_value(json!({
            "name": "Planner", "role": "plan", "harness": "codex", "effort": "low",
            "enabled": true,
        }))
        .unwrap();
        assert!(created.id.is_empty(), "the server assigns ids");
        assert!(created.system_prompt.is_empty());
    }

    #[test]
    fn opencode_and_id_params_round_trip() {
        let key = SetOpencodeProviderApiKeyParams {
            provider_id: "openrouter".into(),
            api_key: "sk-test".into(),
            directory: None,
        };
        assert_eq!(
            serde_json::to_value(&key).unwrap(),
            json!({"providerId": "openrouter", "apiKey": "sk-test"}),
            "absent options stay off the wire"
        );
        assert_eq!(round_trip(&key), key);

        let remove = RemoveOpencodeProviderAuthParams {
            provider_id: "openrouter".into(),
            directory: Some("/repos/demo".into()),
        };
        assert_eq!(
            serde_json::to_value(&remove).unwrap(),
            json!({"providerId": "openrouter", "directory": "/repos/demo"})
        );
        assert_eq!(round_trip(&remove), remove);

        let refresh = RefreshOpencodeCatalogParams { directory: None };
        assert_eq!(serde_json::to_value(&refresh).unwrap(), json!({}));
        assert_eq!(round_trip(&refresh), refresh);

        for wire in [
            serde_json::to_value(ResetHarnessConfigParams { id: "codex".into() }).unwrap(),
            serde_json::to_value(DeleteAgentConfigParams { id: "codex".into() }).unwrap(),
            serde_json::to_value(SetDefaultAgentParams { id: "codex".into() }).unwrap(),
        ] {
            assert_eq!(wire, json!({"id": "codex"}));
        }
    }

    #[test]
    fn config_params_reject_incomplete_and_misspelled_payloads() {
        assert!(serde_json::from_value::<SaveHarnessConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SaveAgentConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ResetHarnessConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<DeleteAgentConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SetDefaultAgentParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SetOpencodeProviderApiKeyParams>(
            json!({"providerId": "openrouter"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<SetOpencodeProviderApiKeyParams>(
                json!({"provider_id": "openrouter", "api_key": "sk"})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<HarnessConfig>(json!({
                "id": "codex", "label": "Codex", "enabled": true, "defaultEffort": "high",
            }))
            .is_err(),
            "a harness config refuses fields it does not define"
        );
        assert!(
            serde_json::from_value::<AgentDefinition>(json!({
                "name": "Planner", "role": "plan", "harness": "codex", "effort": "extreme",
                "enabled": true,
            }))
            .is_err(),
            "unknown effort levels must be rejected"
        );
        assert!(
            serde_json::from_value::<RefreshOpencodeCatalogParams>(json!({"cwd": "/x"})).is_err(),
            "params reject arguments the contract does not name"
        );
    }

    #[test]
    fn prompt_target_wire_values_are_the_storage_keys() {
        for (wire, expected) in [
            ("orchestrator", PromptTargetChoice::Orchestrator),
            ("worker:research", PromptTargetChoice::WorkerResearch),
            ("worker:implementation", PromptTargetChoice::WorkerImplementation),
            ("worker:verification", PromptTargetChoice::WorkerVerification),
            ("worker:planning", PromptTargetChoice::WorkerPlanning),
            ("worker:documentation", PromptTargetChoice::WorkerDocumentation),
            ("direct_session", PromptTargetChoice::DirectSession),
        ] {
            assert_eq!(
                serde_json::from_value::<PromptTargetChoice>(json!(wire)).unwrap(),
                expected
            );
            assert_eq!(serde_json::to_value(&expected).unwrap(), json!(wire));
        }
        assert!(serde_json::from_value::<PromptTargetChoice>(json!("orchestrator:extra")).is_err());
        assert!(serde_json::from_value::<PromptTargetChoice>(json!("worker")).is_err());
        assert!(serde_json::from_value::<PromptTargetChoice>(json!("Orchestrator")).is_err());
    }

    #[test]
    fn prompt_studio_params_round_trip_and_default_depth() {
        let save = SavePromptSectionParams {
            target: PromptTargetChoice::WorkerResearch,
            section_id: "worker_contract".into(),
            text: "Research only.".into(),
            depth: Some(1),
        };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(
            wire,
            json!({
                "target": "worker:research",
                "sectionId": "worker_contract",
                "text": "Research only.",
                "depth": 1,
            })
        );
        assert_eq!(round_trip(&save), save);

        // Absent depth stays off the wire and defaults to None; core applies 0.
        let minimal: PreviewCompiledPromptParams = serde_json::from_value(json!({
            "target": "direct_session",
        }))
        .unwrap();
        assert_eq!(minimal.depth, None);
        assert_eq!(
            serde_json::to_value(&minimal).unwrap(),
            json!({"target": "direct_session"}),
            "absent options stay off the wire"
        );

        // Every method's params round-trips independently; sibling param types
        // must refuse each other's shapes (deny_unknown_fields + required fields).
        let stack_params = GetPromptStackParams {
            target: PromptTargetChoice::Orchestrator,
            depth: None,
        };
        assert_eq!(round_trip(&stack_params), stack_params);
        let reset_params = ResetPromptSectionParams {
            target: PromptTargetChoice::DirectSession,
            section_id: "bridge_role".into(),
            depth: None,
        };
        assert_eq!(round_trip(&reset_params), reset_params);
        let restore_params = RestorePromptRevisionParams {
            target: PromptTargetChoice::Orchestrator,
            section_id: "bridge_role".into(),
            revision_id: 7,
            depth: Some(0),
        };
        assert_eq!(round_trip(&restore_params), restore_params);
        assert!(
            serde_json::from_value::<GetPromptStackParams>(
                serde_json::to_value(&reset_params).unwrap()
            )
            .is_err(),
            "a stack read refuses a reset's shape"
        );
    }

    #[test]
    fn prompt_studio_params_reject_unknown_and_misspelled_fields() {
        assert!(serde_json::from_value::<GetPromptStackParams>(json!({})).is_err());
        assert!(serde_json::from_value::<GetPromptStackParams>(json!({"target": "nope"})).is_err());
        assert!(
            serde_json::from_value::<GetPromptStackParams>(
                json!({"target": "orchestrator", "workerDepth": 1})
            )
            .is_err(),
            "params deny fields the contract does not name"
        );
        assert!(
            serde_json::from_value::<SavePromptSectionParams>(json!({
                "target": "orchestrator", "sectionId": "bridge_role",
            }))
            .is_err(),
            "save requires text"
        );
        assert!(
            serde_json::from_value::<RestorePromptRevisionParams>(json!({
                "target": "orchestrator", "sectionId": "bridge_role",
            }))
            .is_err(),
            "restore requires revisionId"
        );
        assert!(
            serde_json::from_value::<PreviewCompiledPromptParams>(
                json!({"target": "orchestrator", "revisionId": 1})
            )
            .is_err()
        );
    }

    #[test]
    fn prompt_studio_results_round_trip() {
        let stack = PromptStackView {
            target: "orchestrator".into(),
            depth: 0,
            sections: vec![PromptSectionView {
                id: "bridge_role".into(),
                state: PromptSectionStatePayload::Overridden { text: "Custom.".into() },
                default_text: "Default role text.".into(),
                effective_text: Some("Custom.".into()),
                bytes: 7,
                token_estimate: 2,
                lint_warnings: vec![PromptLintWarningView {
                    marker: "bridge-delegate".into(),
                    message: "Typed delegation may stop working.".into(),
                }],
                revisions: vec![PromptRevisionView {
                    id: 4,
                    operation: PromptRevisionOperation::Override,
                    state: PromptSectionStatePayload::Overridden { text: "Custom.".into() },
                    restored_from_revision_id: None,
                    attribution: None,
                    created_at: "2026-08-23T00:00:00Z".into(),
                }],
            }],
        };
        let result = stack;
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["target"], json!("orchestrator"));
        assert_eq!(wire["sections"][0]["state"]["state"], json!("overridden"));
        assert_eq!(wire["sections"][0]["tokenEstimate"], json!(2));
        assert_eq!(
            wire["sections"][0]["revisions"][0]["restoredFromRevisionId"],
            json!(null)
        );
        assert_eq!(round_trip(&result), result);

        let mutation = PromptSectionMutationResult {
            revision: PromptRevisionView {
                id: 5,
                operation: PromptRevisionOperation::Restore,
                state: PromptSectionStatePayload::Deleted,
                restored_from_revision_id: Some(2),
                attribution: None,
                created_at: "2026-08-23T00:00:00Z".into(),
            },
            stack: result.clone(),
        };
        let wire = serde_json::to_value(&mutation).unwrap();
        assert_eq!(wire["revision"]["restoredFromRevisionId"], json!(2));
        assert_eq!(wire["revision"]["operation"], json!("restore"));
        assert_eq!(round_trip(&mutation), mutation);

        // Typed vocabularies serialize as their closed string sets.
        for (value, wire) in [
            (PromptRevisionOperation::Override, "override"),
            (PromptRevisionOperation::Delete, "delete"),
            (PromptRevisionOperation::Reset, "reset"),
            (PromptRevisionOperation::Restore, "restore"),
        ] {
            assert_eq!(serde_json::to_value(&value).unwrap(), json!(wire));
        }
        for (value, wire) in [
            (PromptLayerSource::Reported, "reported"),
            (PromptLayerSource::Measured, "measured"),
            (PromptLayerSource::Estimated, "estimated"),
            (PromptLayerSource::Unavailable, "unavailable"),
        ] {
            assert_eq!(serde_json::to_value(&value).unwrap(), json!(wire));
        }
        assert!(serde_json::from_value::<PromptLayerSource>(json!("fabricated")).is_err());

        let deleted_state = PromptSectionStatePayload::Deleted;
        assert_eq!(
            serde_json::to_value(&deleted_state).unwrap(),
            json!({"state": "deleted"})
        );
    }
}

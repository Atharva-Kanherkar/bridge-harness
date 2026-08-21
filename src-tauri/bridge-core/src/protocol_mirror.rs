//! The drift gate between core DTOs and their `bridge-protocol` mirrors.
//!
//! The protocol crate deliberately does not depend on core: a wire contract
//! that imports the runtime it describes is not a contract. The cost is that
//! several payload types exist twice, and nothing makes the compiler compare
//! them. This module is that comparison, and it is test-only.
//!
//! Two mechanisms, because two things can drift:
//!
//! * **Enums** get an exhaustive `match` from the core variant to the protocol
//!   one. Adding a variant to core stops this module from compiling until the
//!   contract names it, and the assertions then check the two agree on the wire
//!   spelling — which is where `snake_case` and `lowercase` mirrors diverge.
//! * **Structs** get a JSON round-trip: serialize the core value, deserialize
//!   it *as the mirror*, serialize that, and compare documents. A field added,
//!   renamed, or removed on either side changes exactly one of the two.

use serde::{de::DeserializeOwned, Serialize};

use bridge_protocol::messages as wire;

use crate::{
    agent_config, automations, browser_bridge, completion, delegation, learning_job,
    learning_router, marketplace, model, model_profiles, skill_marketplace, suggestion_engine,
};

/// Assert a core DTO and its protocol mirror describe the same document.
fn assert_mirrors<Mirror>(core: &impl Serialize)
where
    Mirror: Serialize + DeserializeOwned,
{
    let name = std::any::type_name::<Mirror>();
    let core_json = serde_json::to_value(core).unwrap();
    let mirror: Mirror = serde_json::from_value(core_json.clone())
        .unwrap_or_else(|error| panic!("{name} rejects what core emits: {error}\n{core_json:#}"));
    assert_eq!(
        serde_json::to_value(&mirror).unwrap(),
        core_json,
        "{name} and its core DTO disagree"
    );
}

/// Assert a core enum variant and its protocol mirror share a wire value.
fn assert_same_wire_value(core: &impl Serialize, mirror: &impl Serialize) {
    assert_eq!(
        serde_json::to_value(core).unwrap(),
        serde_json::to_value(mirror).unwrap(),
        "protocol and core disagree on a wire value"
    );
}

fn mirror_harness(harness: &model::Harness) -> wire::HarnessId {
    wire::HarnessId::try_from(harness).expect("harness has a wire id")
}

fn mirror_effort(effort: delegation::Effort) -> wire::Effort {
    match effort {
        delegation::Effort::Low => wire::Effort::Low,
        delegation::Effort::Medium => wire::Effort::Medium,
        delegation::Effort::High => wire::Effort::High,
        delegation::Effort::Xhigh => wire::Effort::Xhigh,
    }
}

fn mirror_eval_kind(kind: completion::EvalKind) -> wire::EvalKind {
    match kind {
        completion::EvalKind::Deterministic => wire::EvalKind::Deterministic,
        completion::EvalKind::Scrutiny => wire::EvalKind::Scrutiny,
        completion::EvalKind::UserTesting => wire::EvalKind::UserTesting,
    }
}

fn mirror_check_status(status: completion::CheckStatus) -> wire::CheckStatus {
    match status {
        completion::CheckStatus::Pending => wire::CheckStatus::Pending,
        completion::CheckStatus::Running => wire::CheckStatus::Running,
        completion::CheckStatus::Passed => wire::CheckStatus::Passed,
        completion::CheckStatus::Failed => wire::CheckStatus::Failed,
        completion::CheckStatus::Skipped => wire::CheckStatus::Skipped,
        completion::CheckStatus::Blocked => wire::CheckStatus::Blocked,
        completion::CheckStatus::Stale => wire::CheckStatus::Stale,
    }
}

fn mirror_router_mode(mode: learning_router::RouterMode) -> wire::RouterMode {
    match mode {
        learning_router::RouterMode::Disabled => wire::RouterMode::Disabled,
        learning_router::RouterMode::Shadow => wire::RouterMode::Shadow,
        learning_router::RouterMode::Autonomous => wire::RouterMode::Autonomous,
    }
}

fn mirror_profile_purpose(purpose: model_profiles::ProfilePurpose) -> wire::ProfilePurpose {
    match purpose {
        model_profiles::ProfilePurpose::StandardOrchestrator => {
            wire::ProfilePurpose::StandardOrchestrator
        }
        model_profiles::ProfilePurpose::PremiumOrchestrator => {
            wire::ProfilePurpose::PremiumOrchestrator
        }
        model_profiles::ProfilePurpose::Planner => wire::ProfilePurpose::Planner,
        model_profiles::ProfilePurpose::Implementer => wire::ProfilePurpose::Implementer,
        model_profiles::ProfilePurpose::Verifier => wire::ProfilePurpose::Verifier,
        model_profiles::ProfilePurpose::Reviewer => wire::ProfilePurpose::Reviewer,
        model_profiles::ProfilePurpose::Research => wire::ProfilePurpose::Research,
        model_profiles::ProfilePurpose::Documentation => wire::ProfilePurpose::Documentation,
        model_profiles::ProfilePurpose::Evaluator => wire::ProfilePurpose::Evaluator,
    }
}

fn mirror_trigger_kind(kind: learning_job::LearningTriggerKind) -> wire::LearningTriggerKind {
    match kind {
        learning_job::LearningTriggerKind::Manual => wire::LearningTriggerKind::Manual,
        learning_job::LearningTriggerKind::InApp => wire::LearningTriggerKind::InApp,
        learning_job::LearningTriggerKind::Codex => wire::LearningTriggerKind::Codex,
        learning_job::LearningTriggerKind::Claude => wire::LearningTriggerKind::Claude,
        learning_job::LearningTriggerKind::OpenCode => wire::LearningTriggerKind::OpenCode,
    }
}

fn mirror_marketplace_provider(
    provider: marketplace::MarketplaceProvider,
) -> wire::MarketplaceProvider {
    match provider {
        marketplace::MarketplaceProvider::Codex => wire::MarketplaceProvider::Codex,
        marketplace::MarketplaceProvider::Claude => wire::MarketplaceProvider::Claude,
    }
}

fn mirror_marketplace_action(action: marketplace::MarketplaceAction) -> wire::MarketplaceAction {
    match action {
        marketplace::MarketplaceAction::Install => wire::MarketplaceAction::Install,
        marketplace::MarketplaceAction::Enable => wire::MarketplaceAction::Enable,
        marketplace::MarketplaceAction::Disable => wire::MarketplaceAction::Disable,
        marketplace::MarketplaceAction::Update => wire::MarketplaceAction::Update,
        marketplace::MarketplaceAction::Uninstall => wire::MarketplaceAction::Uninstall,
        marketplace::MarketplaceAction::Authenticate => wire::MarketplaceAction::Authenticate,
    }
}

fn mirror_skill_provider(provider: skill_marketplace::SkillProvider) -> wire::SkillProvider {
    match provider {
        skill_marketplace::SkillProvider::Codex => wire::SkillProvider::Codex,
        skill_marketplace::SkillProvider::Claude => wire::SkillProvider::Claude,
        skill_marketplace::SkillProvider::OpenCode => wire::SkillProvider::OpenCode,
    }
}

fn mirror_suggestion_fallback_reason(
    reason: suggestion_engine::FallbackReason,
) -> wire::SuggestionFallbackReason {
    match reason {
        suggestion_engine::FallbackReason::UnknownModel => wire::SuggestionFallbackReason::UnknownModel,
        suggestion_engine::FallbackReason::Unauthorized => wire::SuggestionFallbackReason::Unauthorized,
        suggestion_engine::FallbackReason::RateLimited => wire::SuggestionFallbackReason::RateLimited,
    }
}

fn mirror_skill_action(action: skill_marketplace::SkillAction) -> wire::SkillAction {
    match action {
        skill_marketplace::SkillAction::Install => wire::SkillAction::Install,
        skill_marketplace::SkillAction::Rollback => wire::SkillAction::Rollback,
        skill_marketplace::SkillAction::Uninstall => wire::SkillAction::Uninstall,
    }
}

fn mirror_automation_provider(
    provider: automations::AutomationProvider,
) -> wire::AutomationProvider {
    match provider {
        automations::AutomationProvider::Claude => wire::AutomationProvider::Claude,
        automations::AutomationProvider::Codex => wire::AutomationProvider::Codex,
        automations::AutomationProvider::OpenCode => wire::AutomationProvider::OpenCode,
    }
}

fn mirror_automation_action(action: automations::AutomationAction) -> wire::AutomationAction {
    match action {
        automations::AutomationAction::Pause => wire::AutomationAction::Pause,
        automations::AutomationAction::Resume => wire::AutomationAction::Resume,
        automations::AutomationAction::Delete => wire::AutomationAction::Delete,
    }
}

#[test]
fn harness_ids_round_trip_with_identical_wire_values() {
    for harness in [
        model::Harness::Claude,
        model::Harness::Codex,
        model::Harness::OpenCode,
        model::Harness::Shell,
        model::Harness::from_stored("gemini"),
    ] {
        let id = mirror_harness(&harness);
        assert_same_wire_value(&harness, &id);
        assert_eq!(model::Harness::from(id), harness);
    }
}

#[test]
fn builtin_harnesses_keep_the_wire_values_protocol_0_8_published() {
    // These four strings are persisted in `sessions.harness` and keyed on in
    // the adapter registry. Opening the identifier must not have moved one.
    for (harness, expected) in [
        (model::Harness::Claude, "claude"),
        (model::Harness::Codex, "codex"),
        (model::Harness::OpenCode, "opencode"),
        (model::Harness::Shell, "shell"),
    ] {
        assert_eq!(
            serde_json::to_value(&harness).unwrap(),
            serde_json::json!(expected)
        );
    }
}

#[test]
fn a_registry_agent_sharing_a_builtin_name_is_one_identity_not_two() {
    // The live ACP registry ships an entry whose id is `opencode`, and Bridge
    // ships an OpenCode adapter. They are the same agent reached two ways, so
    // they are one harness with one id and one history. An earlier draft
    // spelled the registry one `acp:opencode`, which made a single agent look
    // like two competing products and leaked the transport into identity.
    let from_registry = model::Harness::from_stored("opencode");
    assert_eq!(from_registry, model::Harness::OpenCode);
    assert_eq!(
        serde_json::to_value(&from_registry).unwrap(),
        serde_json::json!("opencode")
    );
    assert!(model::Harness::parse("acp:opencode").is_err());

    // An agent with no bespoke adapter is named by its own id, not by how it
    // is run — so writing one later changes nothing about its sessions.
    let gemini = model::Harness::from_stored("gemini");
    assert_eq!(
        gemini,
        model::Harness::Agent(wire::HarnessId::parse("gemini").unwrap())
    );
    assert_eq!(gemini.id(), "gemini");
    assert_eq!(mirror_harness(&gemini).as_str(), "gemini");
}

#[test]
fn the_state_snapshot_mirrors_a_session_whose_harness_cannot_be_interpreted() {
    // `assert_mirrors` deserializes what core emits into the wire type, so this
    // is the gate that catches a result violating its own published schema.
    // `Session.harness` is deliberately the tolerant `StoredHarnessId`: a
    // session persisted by a newer Bridge, or one whose agent was uninstalled,
    // must still appear in the snapshot. Were it the strict `HarnessId`, this
    // panics — which is exactly the bug this test exists to prevent.
    for harness in [
        model::Harness::from_stored("gemini"),
        model::Harness::from_stored("acp:gemini"),
        model::Harness::from_stored(""),
        model::Harness::Codex,
    ] {
        let expected = harness.id().into_owned();
        let state = model::BridgeState {
            projects: Vec::new(),
            workspaces: Vec::new(),
            sessions: vec![model::Session {
                harness,
                ..populated_session()
            }],
            events: Vec::new(),
        };
        assert_mirrors::<wire::BridgeState>(&state);

        let mirrored: wire::BridgeState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(mirrored.sessions[0].harness.as_str(), expected);
        // The strict id is available only when the value actually satisfies
        // the grammar, so a client can tell "actionable" from "readable only".
        assert_eq!(
            mirrored.sessions[0].harness.interpreted().is_some(),
            model::Harness::parse(&expected).is_ok(),
            "{expected:?}"
        );
    }
}

#[test]
fn unknown_harnesses_serialize_under_their_own_id_but_are_not_valid_parameters() {
    // The deliberate asymmetry: outbound tolerant so a session whose harness
    // this build cannot interpret still lists and replays under its own name;
    // inbound strict, because nothing can be done with such an id.
    let unknown = model::Harness::from_stored("acp:gemini");
    assert_eq!(unknown, model::Harness::Unknown("acp:gemini".into()));
    assert_eq!(
        serde_json::to_value(&unknown).unwrap(),
        serde_json::json!("acp:gemini")
    );
    assert_eq!(unknown.label(), "acp:gemini");
    assert!(wire::HarnessId::try_from(&unknown).is_err());
    assert!(model::Harness::parse("acp:gemini").is_err());
    assert!(serde_json::from_value::<model::Harness>(serde_json::json!("acp:gemini")).is_err());
}

#[test]
fn a_stored_harness_id_is_idempotent_through_its_canonical_form() {
    // Reading a row and writing it back must be a fixed point, and a value
    // `from_stored` produces must never alias a different variant: two
    // harnesses that serialize the same are the same harness.
    let ids = [
        "claude",
        "codex",
        "opencode",
        "shell",
        "gemini",
        "github-copilot-cli",
        "",
        "acp:gemini",
        "Gemini",
    ];
    let mut seen: Vec<(String, model::Harness)> = Vec::new();
    for raw in ids {
        let harness = model::Harness::from_stored(raw);
        let canonical = harness.id().into_owned();
        assert_eq!(canonical, raw, "{raw:?} is not its own canonical form");
        assert_eq!(
            model::Harness::from_stored(&canonical),
            harness,
            "{raw:?} is not a fixed point"
        );
        if let Some((_, other)) = seen.iter().find(|(id, _)| id == &canonical) {
            assert_eq!(
                other, &harness,
                "{canonical:?} names two different harnesses"
            );
        }
        seen.push((canonical, harness));
    }
}

#[test]
fn a_stored_harness_id_is_never_read_as_a_different_harness() {
    // Regression for the removed `_ => Harness::Shell` fallthrough, which
    // turned a corrupt or forward-dated row into a runnable shell session.
    for raw in ["", "acp:gemini", "Gemini", "SHELL", "gem ini", "claude "] {
        let restored = model::Harness::from_stored(raw);
        assert_eq!(
            restored,
            model::Harness::Unknown(raw.to_owned()),
            "{raw:?} was interpreted as {restored:?}"
        );
        // Whatever it is, it round-trips back to the same stored bytes.
        assert_eq!(restored.id(), raw);
    }
}

#[test]
fn effort_levels_share_their_wire_values() {
    for effort in [
        delegation::Effort::Low,
        delegation::Effort::Medium,
        delegation::Effort::High,
        delegation::Effort::Xhigh,
    ] {
        assert_same_wire_value(&effort, &mirror_effort(effort));
    }
}

#[test]
fn completion_verdicts_share_their_wire_values() {
    for kind in [
        completion::EvalKind::Deterministic,
        completion::EvalKind::Scrutiny,
        completion::EvalKind::UserTesting,
    ] {
        assert_same_wire_value(&kind, &mirror_eval_kind(kind));
    }
    for status in [
        completion::CheckStatus::Pending,
        completion::CheckStatus::Running,
        completion::CheckStatus::Passed,
        completion::CheckStatus::Failed,
        completion::CheckStatus::Skipped,
        completion::CheckStatus::Blocked,
        completion::CheckStatus::Stale,
    ] {
        assert_same_wire_value(&status, &mirror_check_status(status));
    }
}

#[test]
fn routing_and_profile_enums_share_their_wire_values() {
    for mode in [
        learning_router::RouterMode::Disabled,
        learning_router::RouterMode::Shadow,
        learning_router::RouterMode::Autonomous,
    ] {
        assert_same_wire_value(&mode, &mirror_router_mode(mode));
    }
    for purpose in model_profiles::ProfilePurpose::ALL {
        assert_same_wire_value(&purpose, &mirror_profile_purpose(purpose));
    }
}

#[test]
fn suggestion_fallback_reasons_share_their_wire_values() {
    for reason in [
        suggestion_engine::FallbackReason::UnknownModel,
        suggestion_engine::FallbackReason::Unauthorized,
        suggestion_engine::FallbackReason::RateLimited,
    ] {
        assert_same_wire_value(&reason, &mirror_suggestion_fallback_reason(reason));
    }
}

#[test]
fn learning_trigger_kinds_share_their_wire_values() {
    for kind in [
        learning_job::LearningTriggerKind::Manual,
        learning_job::LearningTriggerKind::InApp,
        learning_job::LearningTriggerKind::Codex,
        learning_job::LearningTriggerKind::Claude,
        learning_job::LearningTriggerKind::OpenCode,
    ] {
        assert_same_wire_value(&kind, &mirror_trigger_kind(kind));
    }
    for (core, external) in [
        (
            learning_job::LearningTriggerKind::Codex,
            wire::ExternalLearningTriggerKind::Codex,
        ),
        (
            learning_job::LearningTriggerKind::Claude,
            wire::ExternalLearningTriggerKind::Claude,
        ),
        (
            learning_job::LearningTriggerKind::OpenCode,
            wire::ExternalLearningTriggerKind::OpenCode,
        ),
    ] {
        assert_same_wire_value(&core, &external);
    }
    for (core, local) in [
        (
            learning_job::LearningTriggerKind::Manual,
            wire::LocalLearningTriggerKind::Manual,
        ),
        (
            learning_job::LearningTriggerKind::InApp,
            wire::LocalLearningTriggerKind::InApp,
        ),
    ] {
        assert_same_wire_value(&core, &local);
    }
}

#[test]
fn marketplace_and_skill_enums_share_their_wire_values() {
    for provider in [
        marketplace::MarketplaceProvider::Codex,
        marketplace::MarketplaceProvider::Claude,
    ] {
        assert_same_wire_value(&provider, &mirror_marketplace_provider(provider));
    }
    for action in [
        marketplace::MarketplaceAction::Install,
        marketplace::MarketplaceAction::Enable,
        marketplace::MarketplaceAction::Disable,
        marketplace::MarketplaceAction::Update,
        marketplace::MarketplaceAction::Uninstall,
        marketplace::MarketplaceAction::Authenticate,
    ] {
        assert_same_wire_value(&action, &mirror_marketplace_action(action));
    }
    for provider in [
        skill_marketplace::SkillProvider::Codex,
        skill_marketplace::SkillProvider::Claude,
        skill_marketplace::SkillProvider::OpenCode,
    ] {
        assert_same_wire_value(&provider, &mirror_skill_provider(provider));
    }
    for action in [
        skill_marketplace::SkillAction::Install,
        skill_marketplace::SkillAction::Rollback,
        skill_marketplace::SkillAction::Uninstall,
    ] {
        assert_same_wire_value(&action, &mirror_skill_action(action));
    }
    for provider in [
        automations::AutomationProvider::Claude,
        automations::AutomationProvider::Codex,
        automations::AutomationProvider::OpenCode,
    ] {
        assert_same_wire_value(&provider, &mirror_automation_provider(provider));
    }
    for action in [
        automations::AutomationAction::Pause,
        automations::AutomationAction::Resume,
        automations::AutomationAction::Delete,
    ] {
        assert_same_wire_value(&action, &mirror_automation_action(action));
    }
}

#[test]
fn base_branch_divergence_mirrors_core() {
    assert_mirrors::<wire::ReadWorkspaceFileResult>(&crate::workspace_files::FileContents {
        path: "src/main.rs".into(),
        content: "fn main() {}\n".into(),
        sha256: "e3b0c442".into(),
        too_large: false,
        binary: false,
        size_bytes: 13,
    });
    assert_mirrors::<wire::WriteWorkspaceFileResult>(&crate::workspace_files::WriteOutcome {
        sha256: "e3b0c442".into(),
    });
    assert_mirrors::<wire::BaseBranchDivergence>(&crate::git::BaseBranchDivergence {
        base_ref: Some("origin/main".into()),
        base_commit: Some("90ce51c".into()),
        head: Some("2b43aaad9b36".into()),
        branch: Some("bridge/task".into()),
        ahead: 1,
        behind: 67,
        ref_age_seconds: Some(3_600),
        fetch_attempted: true,
        fetched: false,
        dirty: true,
        unavailable_reason: None,
    });
}

#[test]
fn worker_repository_binding_mirrors_core() {
    assert_mirrors::<wire::WorkerRepositoryBinding>(
        &crate::worker_adoption::WorkerRepositoryBinding {
            session_id: "child".into(),
            parent_session_id: "parent".into(),
            workspace_id: "w".into(),
            worktree_path: "/repos/demo/worktrees/workers/oslo/child".into(),
            worktree_branch: "bridge/task-worker-child".into(),
            task_worktree_path: "/repos/demo/worktrees/oslo".into(),
            state: crate::worker_adoption::STATE_PENDING.into(),
            head: Some("2b43aaad9b36".into()),
            base_commit: Some("90ce51c".into()),
            base_branch: Some("bridge/task".into()),
            baseline_dirty_paths: vec![],
            changed_paths: vec!["src/components/Markdown.tsx".into()],
            diffstat: Some("1 file(s) changed, 12 insertion(s), 3 deletion(s)".into()),
            dirty: false,
            detail: None,
            created_at: "now".into(),
            updated_at: "now".into(),
        },
    );
}

#[test]
fn completion_payloads_mirror_core() {
    assert_mirrors::<wire::CheckRun>(&completion::CheckRun {
        check_id: "cargo-test".into(),
        kind: completion::EvalKind::Deterministic,
        required: true,
        status: completion::CheckStatus::Passed,
        executor: "shell".into(),
        command: Some("cargo test".into()),
        verifier_family: Some("rust".into()),
        detail: Some("184 passed".into()),
        output_digest: Some("sha256:abc".into()),
        artifact_refs: vec!["artifact-1".into()],
    });
    assert_mirrors::<wire::VerifierManifest>(&completion::VerifierManifest {
        id: "rust-tests".into(),
        kind: completion::EvalKind::UserTesting,
        triggers: vec!["rust".into()],
        required_capabilities: vec!["shell".into()],
        different_model_family: true,
        checks: vec!["cargo-test".into()],
        evidence_required: vec!["digest".into()],
    });
}

#[test]
fn routing_and_profile_payloads_mirror_core() {
    assert_mirrors::<wire::RouterPreferences>(&learning_router::RouterPreferences {
        mode: learning_router::RouterMode::Autonomous,
        minimum_pass_bps: 6_500,
        pinned_harness: Some("codex".into()),
        pinned_model: Some("gpt-5".into()),
        excluded_harnesses: vec!["shell".into()],
        excluded_models: vec!["haiku".into()],
    });
    assert_mirrors::<wire::ModelProfileDraft>(&model_profiles::ModelProfileDraft {
        purpose: model_profiles::ProfilePurpose::StandardOrchestrator,
        provider: "codex".into(),
        model: "gpt-5".into(),
        effort: delegation::Effort::High,
        fallback_purpose: Some(model_profiles::ProfilePurpose::PremiumOrchestrator),
        pinned: true,
        learning_enabled: false,
        budget_preference: Some("balanced".into()),
        latency_preference: Some("interactive".into()),
    });
}

#[test]
fn configuration_payloads_mirror_core() {
    assert_mirrors::<wire::HarnessConfig>(&agent_config::HarnessConfig {
        id: "codex".into(),
        label: "Codex".into(),
        enabled: true,
        default_model: Some("gpt-5".into()),
        effort: Some(delegation::Effort::Xhigh),
        system_prompt: "Be exacting.".into(),
        advanced: serde_json::json!({"sandbox": "workspace-write"}),
        is_override: true,
    });
    assert_mirrors::<wire::AgentDefinition>(&agent_config::AgentDefinition {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        description: "Reviews diffs".into(),
        role: "review".into(),
        harness: "claude".into(),
        model: Some("sonnet".into()),
        effort: delegation::Effort::Medium,
        system_prompt: "Be exacting.".into(),
        enabled: true,
        is_default: false,
        is_built_in: true,
        created_at: "now".into(),
        updated_at: "now".into(),
    });
}

#[test]
fn learning_schedules_mirror_core() {
    assert_mirrors::<wire::LearningSchedule>(&learning_job::LearningSchedule {
        job_id: "default".into(),
        enabled: true,
        cadence_minutes: 720,
        next_run_at: Some("later".into()),
        run_budget_microusd: 250_000,
        run_budget_tokens: 400_000,
        mode: "ask".into(),
    });
}

#[test]
fn browser_payloads_mirror_core() {
    assert_mirrors::<wire::BrowserActionRequest>(&browser_bridge::BrowserActionRequest {
        kind: "click".into(),
        element_id: Some("submit".into()),
        text: Some("hello".into()),
        url: Some("https://example.test".into()),
        x: Some(12.5),
        y: Some(48.0),
        tab_id: Some(3),
        sensitive_kind: Some("password".into()),
        expected_domain: Some("example.test".into()),
        actor: Some("worker-1".into()),
    });
    assert_mirrors::<wire::BrowserRouteRequest>(&browser_bridge::BrowserRouteRequest {
        structured_api_available: false,
        needs_user_auth: true,
        needs_isolation: false,
        needs_parallelism: true,
        needs_geo_or_proxy: false,
        unattended: true,
        dom_control_available: true,
        remote_provider_configured: false,
        task_class: Some("checkout".into()),
    });
    assert_mirrors::<wire::RemoteBrowserConfig>(&browser_bridge::RemoteBrowserConfig {
        endpoint: "wss://remote.test".into(),
        bearer_token_env: "REMOTE_BROWSER_TOKEN".into(),
        enabled: true,
    });
}

// --- snapshot DTOs (#125) ----------------------------------------------------

fn mirror_session_status(status: &model::SessionStatus) -> wire::SessionStatus {
    match status {
        model::SessionStatus::Idle => wire::SessionStatus::Idle,
        model::SessionStatus::Starting => wire::SessionStatus::Starting,
        model::SessionStatus::Working => wire::SessionStatus::Working,
        model::SessionStatus::Waiting => wire::SessionStatus::Waiting,
        model::SessionStatus::Warm => wire::SessionStatus::Warm,
        model::SessionStatus::Checkpointing => wire::SessionStatus::Checkpointing,
        model::SessionStatus::Ready => wire::SessionStatus::Ready,
        model::SessionStatus::Stopped => wire::SessionStatus::Stopped,
        model::SessionStatus::Resuming => wire::SessionStatus::Resuming,
        model::SessionStatus::Restored => wire::SessionStatus::Restored,
        model::SessionStatus::Failed => wire::SessionStatus::Failed,
        model::SessionStatus::Completed => wire::SessionStatus::Completed,
        model::SessionStatus::Cancelled => wire::SessionStatus::Cancelled,
    }
}

#[test]
fn snapshot_enums_share_their_wire_values() {
    for status in [
        model::SessionStatus::Idle,
        model::SessionStatus::Starting,
        model::SessionStatus::Working,
        model::SessionStatus::Waiting,
        model::SessionStatus::Warm,
        model::SessionStatus::Checkpointing,
        model::SessionStatus::Ready,
        model::SessionStatus::Stopped,
        model::SessionStatus::Resuming,
        model::SessionStatus::Restored,
        model::SessionStatus::Failed,
        model::SessionStatus::Completed,
        model::SessionStatus::Cancelled,
    ] {
        let mirrored = mirror_session_status(&status);
        assert_same_wire_value(&status, &mirrored);
    }
    for tier in [
        model::CapabilityTier::Fast,
        model::CapabilityTier::Standard,
        model::CapabilityTier::Strong,
    ] {
        let mirrored = match tier {
            model::CapabilityTier::Fast => wire::CapabilityTier::Fast,
            model::CapabilityTier::Standard => wire::CapabilityTier::Standard,
            model::CapabilityTier::Strong => wire::CapabilityTier::Strong,
        };
        assert_same_wire_value(&tier, &mirrored);
    }
    for mode in [
        model::RestorationMode::Hot,
        model::RestorationMode::Native,
        model::RestorationMode::CheckpointRestored,
        model::RestorationMode::Fresh,
    ] {
        let mirrored = match mode {
            model::RestorationMode::Hot => wire::RestorationMode::Hot,
            model::RestorationMode::Native => wire::RestorationMode::Native,
            model::RestorationMode::CheckpointRestored => wire::RestorationMode::CheckpointRestored,
            model::RestorationMode::Fresh => wire::RestorationMode::Fresh,
        };
        assert_same_wire_value(&mode, &mirrored);
    }
    for fidelity in [
        model::ContinuationFidelity::Native,
        model::ContinuationFidelity::ProjectedAtBoundary,
        model::ContinuationFidelity::ProjectedMidTurn,
    ] {
        let mirrored = match fidelity {
            model::ContinuationFidelity::Native => wire::ContinuationFidelity::Native,
            model::ContinuationFidelity::ProjectedAtBoundary => {
                wire::ContinuationFidelity::ProjectedAtBoundary
            }
            model::ContinuationFidelity::ProjectedMidTurn => {
                wire::ContinuationFidelity::ProjectedMidTurn
            }
        };
        assert_same_wire_value(&fidelity, &mirrored);
    }
    for eligibility in [
        model::ResumeEligibility::Native,
        model::ResumeEligibility::CheckpointRestored,
        model::ResumeEligibility::Fresh,
    ] {
        let mirrored = match eligibility {
            model::ResumeEligibility::Native => wire::ResumeEligibility::Native,
            model::ResumeEligibility::CheckpointRestored => {
                wire::ResumeEligibility::CheckpointRestored
            }
            model::ResumeEligibility::Fresh => wire::ResumeEligibility::Fresh,
        };
        assert_same_wire_value(&eligibility, &mirrored);
    }
    for verdict in [
        completion::CompletionVerdict::Verifying,
        completion::CompletionVerdict::ChangesRequested,
        completion::CompletionVerdict::Verified,
        completion::CompletionVerdict::Waived,
        completion::CompletionVerdict::Failed,
        completion::CompletionVerdict::Superseded,
    ] {
        let mirrored = match verdict {
            completion::CompletionVerdict::Verifying => wire::CompletionVerdict::Verifying,
            completion::CompletionVerdict::ChangesRequested => {
                wire::CompletionVerdict::ChangesRequested
            }
            completion::CompletionVerdict::Verified => wire::CompletionVerdict::Verified,
            completion::CompletionVerdict::Waived => wire::CompletionVerdict::Waived,
            completion::CompletionVerdict::Failed => wire::CompletionVerdict::Failed,
            completion::CompletionVerdict::Superseded => wire::CompletionVerdict::Superseded,
        };
        assert_same_wire_value(&verdict, &mirrored);
    }
}

fn populated_session() -> model::Session {
    model::Session {
        id: "s-1".into(),
        workspace_id: Some("w-1".into()),
        harness: model::Harness::Codex,
        label: "Orchestrator".into(),
        status: model::SessionStatus::Waiting,
        started_at: Some("now".into()),
        ended_at: Some("later".into()),
        context_percent: Some(41),
        usage_percent: Some(12),
        metric_source: "reported".into(),
        provider_session_id: Some("prov-1".into()),
        active_turn_id: Some("turn-1".into()),
        model: Some("gpt-5".into()),
        requested_tier: Some(model::CapabilityTier::Standard),
        effort: Some("high".into()),
        parent_session_id: Some("parent".into()),
        depth: Some(1),
        restoration_mode: model::RestorationMode::CheckpointRestored,
        continuation_fidelity: model::ContinuationFidelity::ProjectedAtBoundary,
        title: Some("Fix tests".into()),
        kind: "orchestrator".into(),
        cwd: Some("/repos/demo".into()),
    }
}

#[test]
fn the_bridge_state_snapshot_mirrors_core() {
    // Every field populated with Some(...) so a renamed, retyped, or removed
    // field on either side breaks JSON equality.
    assert_mirrors::<wire::BridgeState>(&model::BridgeState {
        projects: vec![model::Project {
            id: "p-1".into(),
            name: "Demo".into(),
            path: "/repos/demo".into(),
            created_at: "now".into(),
        }],
        workspaces: vec![model::Workspace {
            id: "w-1".into(),
            project_id: Some("p-1".into()),
            city: Some("Kyoto".into()),
            title: "Payments".into(),
            branch: Some("bridge/payments".into()),
            path: Some("/repos/demo".into()),
            status: model::SessionStatus::Working,
            dirty_files: 2,
            additions: 40,
            deletions: 3,
            created_at: "now".into(),
        }],
        sessions: vec![populated_session()],
        events: vec![model::BridgeEvent {
            id: 9,
            source: "supervisor".into(),
            kind: "workspace.created".into(),
            entity_id: "w-1".into(),
            body: "Created workspace".into(),
            created_at: "now".into(),
        }],
    });
}

#[test]
fn the_session_forest_snapshot_mirrors_core() {
    let entry = model::SessionEntry {
        id: "e-1".into(),
        session_id: "s-1".into(),
        parent_entry_id: Some("e-0".into()),
        sequence: 1,
        semantic_schema_version: 2,
        kind: "assistant.message".into(),
        payload: serde_json::json!({"text": "hello"}),
        provider_event_id: Some("prov-e".into()),
        context_visibility: "visible".into(),
        token_estimate: Some(12),
        created_at: "now".into(),
    };
    assert_mirrors::<wire::SessionForestSnapshot>(&model::SessionForestSnapshot {
        session_id: "s-1".into(),
        entries: vec![entry.clone()],
        head: Some(model::SessionHead {
            session_id: "s-1".into(),
            active_entry_id: Some("e-1".into()),
            native_provider_session_id: Some("prov-1".into()),
            restoration_mode: model::RestorationMode::Native,
            resume_eligibility: model::ResumeEligibility::CheckpointRestored,
            latest_checkpoint_entry_id: Some("e-0".into()),
            updated_at: "now".into(),
        }),
        leaves: vec![entry],
        worker_leases: vec![model::WorkerLease {
            session_id: "worker-1".into(),
            workspace_id: "w-1".into(),
            role: "implementation".into(),
            capability_tier: "standard".into(),
            task_family: "rust".into(),
            owned_paths: serde_json::json!(["src/"]),
            write_mode: "exclusive".into(),
            lease_status: "active".into(),
            expires_at: Some("later".into()),
            created_at: "now".into(),
            updated_at: "now".into(),
        }],
        worker_runtimes: vec![model::WorkerRuntimeRecord {
            session_id: "worker-1".into(),
            parent_session_id: "s-1".into(),
            lifecycle_state: "working".into(),
            task_family: "rust".into(),
            compatibility_key: "codex:gpt-5".into(),
            result_status: "pending".into(),
            retry_count: 1,
            warm_until: Some("later".into()),
            worktree_path: Some("/worktrees/w".into()),
            worktree_branch: Some("bridge/w".into()),
            last_result: Some(serde_json::json!({"ok": true})),
            last_activity_at: Some("now".into()),
            waiting_since: Some("now".into()),
            waiting_reason: Some("approval_requested".into()),
            progress_summary: Some("Running: cargo test".into()),
            updated_at: "now".into(),
        }],
        worker_queue: vec![model::QueuedWorkerRequest {
            id: "q-1".into(),
            parent_session_id: "s-1".into(),
            workspace_id: "w-1".into(),
            turn_id: "turn-1".into(),
            request: serde_json::json!({"objective": "fix"}),
            actual_model: "gpt-5".into(),
            queue_status: "queued".into(),
            sequence: 1,
            dispatched_session_id: Some("worker-1".into()),
            attempt_count: 1,
            expires_at: "later".into(),
            blocked_at: Some("now".into()),
            claimed_at: Some("now".into()),
            last_error: Some("busy".into()),
            created_at: "now".into(),
            updated_at: "now".into(),
        }],
        usage: vec![model::UsageLedgerRow {
            id: 1,
            workspace_id: "w-1".into(),
            session_id: Some("s-1".into()),
            turn_id: Some("turn-1".into()),
            input_tokens: Some(1000),
            output_tokens: Some(200),
            cache_read_tokens: Some(800),
            cache_write_tokens: Some(10),
            uncached_input_tokens: Some(200),
            context_percent: Some(30),
            capability_units: 2,
            runtime_ms: Some(1200),
            cost_microusd: Some(310),
            cost_source: Some("reported".into()),
            stable_prefix_id: Some("prefix-1".into()),
            stable_prefix_hash: Some("hash".into()),
            prompt_schema_version: Some(1),
            prefix_token_estimate: Some(700),
            harness: Some("codex".into()),
            model: Some("gpt-5".into()),
            role: Some("orchestrator".into()),
            task_family: Some("rust".into()),
            restoration_mode: Some("fresh".into()),
            cross_harness_reuse: Some("same_harness".into()),
            source: "provider".into(),
            created_at: "now".into(),
        }],
        reasons: vec![model::BridgeEvent {
            id: 4,
            source: "policy".into(),
            kind: "delegation.approved".into(),
            entity_id: "s-1".into(),
            body: "approved".into(),
            created_at: "now".into(),
        }],
        policy_limits: model::PolicyLimits {
            max_workers_per_turn: 4,
            max_strong_workers_per_turn: 1,
            max_capability_units_per_turn: 8,
        },
        repository_divergence: model::RepositoryDivergence {
            status: "diverged".into(),
            selected_state: Some(serde_json::json!({"head": "old"})),
            current_state: serde_json::json!({"head": "new"}),
        },
        completion: Some(completion::CompletionSummary {
            attempt_id: "a-1".into(),
            contract_id: "c-1".into(),
            verdict: completion::CompletionVerdict::ChangesRequested,
            repository: completion::RepositoryStamp {
                head: "abc".into(),
                dirty_digest: "sha256:d".into(),
            },
            passed_required: 1,
            total_required: 3,
            checks: vec![completion::CheckRun {
                check_id: "cargo-test".into(),
                kind: completion::EvalKind::Deterministic,
                required: true,
                status: completion::CheckStatus::Failed,
                executor: "shell".into(),
                command: Some("cargo test".into()),
                verifier_family: Some("rust".into()),
                detail: Some("2 failed".into()),
                output_digest: Some("sha256:o".into()),
                artifact_refs: vec!["artifact-1".into()],
            }],
            markdown_committed: true,
            waiver_reason: Some("flake".into()),
        }),
    });
}

#[test]
fn result_payloads_mirror_core() {
    assert_mirrors::<wire::HealthResult>(&crate::api::Health {
        ok: true,
        version: "0.1.0",
        harnesses: std::collections::HashMap::from([("claude", true), ("shell", true)]),
        database: "/data/bridge.db".into(),
        telemetry_database: "/data/bridge-telemetry.db".into(),
        snapshot_directory: "/data/history-snapshots".into(),
        snapshot_count: 9,
        snapshot_total_bytes: 4_096,
        adapters: vec![model::AdapterDescriptor {
            // Deliberately a partial declaration so the mirror proves the wire
            // shape carries the exact list rather than a defaulted one.
            sandbox_modes: vec![
                model::SandboxMode::ReadOnly,
                model::SandboxMode::WorkspaceWrite,
            ],
            id: "codex".into(),
            label: "Codex".into(),
            available: true,
            version: Some("1.0".into()),
            capabilities: vec!["shell".into()],
            unavailable_reason: Some("offline".into()),
            models: vec![model::ModelOption {
                id: "gpt-5".into(),
                label: "GPT-5".into(),
                tier: model::CapabilityTier::Strong,
                default_for_tier: true,
            }],
            default_model: Some("gpt-5".into()),
        }],
    });
    assert_mirrors::<wire::SessionForestDigestResult>(&crate::api::ForestDigest {
        digest: "v1:42:2026-08-20T00:00:00Z".into(),
    });
    assert_mirrors::<wire::SanitizedTurn>(&crate::secret_interception::SanitizedTurn {
        text: "use {{bridge:secret:ref-1}}".into(),
        interceptions: vec![crate::secret_interception::SecretInterception {
            reference: "ref-1".into(),
            detector: "openai_api_key".into(),
        }],
    });
    assert_mirrors::<wire::SlashCommand>(&crate::slash::SlashCommand {
        name: "review".into(),
        description: "Review the diff".into(),
        harness: "claude".into(),
        kind: "skill".into(),
    });
    assert_mirrors::<wire::SlashCommandResolve>(&crate::api::SlashCommandResolve {
        name: "review".into(),
        harness: "claude".into(),
        kind: "skill".into(),
        switch_harness: true,
    });
    assert_mirrors::<wire::VerifierCandidate>(&completion::VerifierCandidate {
        manifest: completion::VerifierManifest {
            id: "rust-tests".into(),
            kind: completion::EvalKind::Deterministic,
            triggers: vec!["rust".into()],
            required_capabilities: vec!["shell".into()],
            different_model_family: false,
            checks: vec!["cargo-test".into()],
            evidence_required: vec!["digest".into()],
        },
        eligible: false,
        exclusion_reasons: vec!["missing capabilities: browser".into()],
    });
    assert_mirrors::<wire::ConfigState>(&agent_config::ConfigState {
        harnesses: vec![agent_config::HarnessConfig {
            id: "codex".into(),
            label: "Codex".into(),
            enabled: true,
            default_model: Some("gpt-5".into()),
            effort: Some(delegation::Effort::High),
            system_prompt: "Be exacting.".into(),
            advanced: serde_json::json!({"sandbox": "workspace-write"}),
            is_override: true,
        }],
        agents: vec![agent_config::AgentDefinition {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            description: "Reviews diffs".into(),
            role: "review".into(),
            harness: "claude".into(),
            model: Some("sonnet".into()),
            effort: delegation::Effort::Medium,
            system_prompt: "Be exacting.".into(),
            enabled: true,
            is_default: true,
            is_built_in: false,
            created_at: "now".into(),
            updated_at: "now".into(),
        }],
        default_agent_id: "reviewer".into(),
        permission_policy: agent_config::PermissionPolicy {
            bypass_all: true,
            updated_at: "now".into(),
        },
    });
    assert_mirrors::<wire::BrowserRouteDecision>(&browser_bridge::route_browser(
        browser_bridge::BrowserRouteRequest {
            structured_api_available: false,
            needs_user_auth: true,
            needs_isolation: false,
            needs_parallelism: false,
            needs_geo_or_proxy: false,
            unattended: false,
            dom_control_available: true,
            remote_provider_configured: false,
            task_class: Some("checkout".into()),
        },
    ));
    for skill in browser_bridge::bundled_skills() {
        assert_mirrors::<wire::BrowserSkill>(&skill);
    }
}

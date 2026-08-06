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
    agent_config, browser_bridge, completion, delegation, learning_job, learning_router,
    marketplace, model, model_profiles, skill_marketplace,
};

/// Assert a core DTO and its protocol mirror describe the same document.
fn assert_mirrors<Mirror>(core: &impl Serialize)
where
    Mirror: Serialize + DeserializeOwned,
{
    let name = std::any::type_name::<Mirror>();
    let core_json = serde_json::to_value(core).unwrap();
    let mirror: Mirror = serde_json::from_value(core_json.clone()).unwrap_or_else(|error| {
        panic!("{name} rejects what core emits: {error}\n{core_json:#}")
    });
    assert_eq!(
        serde_json::to_value(&mirror).unwrap(),
        core_json,
        "{name} and its core DTO disagree"
    );
}

/// Assert core accepts every document its protocol mirror emits.
///
/// Used where the core type is a command *input* and so derives only
/// `Deserialize`: there is no core document to compare against. Weaker than
/// [`assert_mirrors`] — it catches a renamed, retyped, or removed core field,
/// but an *optional* field added to core and not to the mirror slips through.
fn assert_core_accepts_mirror<Core>(mirror: &impl Serialize)
where
    Core: DeserializeOwned,
{
    let wire_json = serde_json::to_value(mirror).unwrap();
    serde_json::from_value::<Core>(wire_json.clone()).unwrap_or_else(|error| {
        panic!(
            "core rejects what {} emits: {error}\n{wire_json:#}",
            std::any::type_name::<Core>()
        )
    });
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
    wire::HarnessId::from(harness)
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

fn mirror_skill_action(action: skill_marketplace::SkillAction) -> wire::SkillAction {
    match action {
        skill_marketplace::SkillAction::Install => wire::SkillAction::Install,
        skill_marketplace::SkillAction::Rollback => wire::SkillAction::Rollback,
        skill_marketplace::SkillAction::Uninstall => wire::SkillAction::Uninstall,
    }
}

#[test]
fn harness_ids_round_trip_with_identical_wire_values() {
    for harness in [
        model::Harness::Claude,
        model::Harness::Codex,
        model::Harness::OpenCode,
        model::Harness::Shell,
    ] {
        let id = mirror_harness(&harness);
        assert_same_wire_value(&harness, &id);
        assert_eq!(model::Harness::from(id), harness);
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
    // Core's action request is input-only, so compare in the one direction
    // that exists: everything the contract emits, core must accept.
    assert_core_accepts_mirror::<browser_bridge::BrowserActionRequest>(
        &wire::BrowserActionRequest {
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
        },
    );
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

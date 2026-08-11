//! Request dispatch: every method in the registry, validated against its
//! contracted params type and routed to its `bridge_core::api` body.
//!
//! The `match` below is exhaustive over [`MethodName`], so adding a method to
//! the registry stops this crate from compiling until the daemon serves it.
//! Params are deserialized as the **wire** types from `bridge_protocol` — the
//! same schemas clients generate against — so casing mistakes, unknown fields,
//! and out-of-set enum values all fail here with `invalid_params` instead of
//! deep inside the runtime.

use bridge_core::{api, learning_job, BridgeCore, BridgeError};
use bridge_protocol::messages as wire;
use bridge_protocol::{ErrorCode, MethodName, RpcError, TypedMethod};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Handle one request. `params` is the request's raw params value, if any.
pub fn dispatch(
    core: &Arc<BridgeCore>,
    method: MethodName,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    // The contract knows which methods are parameterless; enforce the absence
    // here so a stray payload is a client error, not silently dropped data.
    if TypedMethod::for_method(method).params.is_none() && params.is_some() {
        return Err(RpcError::new(
            ErrorCode::InvalidParams,
            format!("{} takes no parameters", method.as_str()),
        ));
    }

    match method {
        MethodName::Health => reply(api::health(core)),
        MethodName::GetState => reply(api::get_state(core)),

        MethodName::AddProject => {
            let p: wire::AddProjectParams = decode(method, params)?;
            reply(api::add_project(core, &p.path))
        }

        MethodName::CreateWorkspace => {
            let p: wire::CreateWorkspaceParams = decode(method, params)?;
            reply(api::create_workspace(core, &p.title))
        }
        MethodName::ConnectWorkspaceFolder => {
            let p: wire::ConnectWorkspaceFolderParams = decode(method, params)?;
            reply(api::connect_workspace_folder(core, &p.workspace_id, &p.path))
        }
        MethodName::ListWorkspaceFiles => {
            let p: wire::ListWorkspaceFilesParams = decode(method, params)?;
            reply(api::list_workspace_files(core, &p.session_id))
        }
        MethodName::RefreshWorkspace => {
            let p: wire::RefreshWorkspaceParams = decode(method, params)?;
            reply(api::refresh_workspace(core, &p.workspace_id))
        }
        MethodName::ArchiveWorkspace => {
            let p: wire::ArchiveWorkspaceParams = decode(method, params)?;
            reply(api::archive_workspace(core, &p.workspace_id))
        }

        MethodName::GetSessionForest => {
            let p: wire::GetSessionForestParams = decode(method, params)?;
            reply(api::get_session_forest(core, &p.session_id))
        }
        MethodName::ReplaySessionEvents => {
            let p: wire::ReplaySessionEventsParams = decode(method, params)?;
            reply(api::replay_session_events(core, &p.session_id, p.after_sequence, p.limit))
        }
        MethodName::ActivateSessionEntry => {
            let p: wire::ActivateSessionEntryParams = decode(method, params)?;
            reply(api::activate_session_entry(core, &p.session_id, &p.entry_id))
        }
        MethodName::CreateChat => {
            let p: wire::CreateChatParams = decode(method, params)?;
            reply(api::create_chat(core, &p.harness.into(), p.model.as_deref(), p.title.as_deref()))
        }
        MethodName::CreateWorkspaceSession => {
            let p: wire::CreateWorkspaceSessionParams = decode(method, params)?;
            reply(api::create_workspace_session(
                core,
                &p.workspace_id,
                p.create_worktree.unwrap_or(false),
            ))
        }
        MethodName::StartSession => {
            let p: wire::StartSessionParams = decode(method, params)?;
            reply(api::start_session(core, p.workspace_id, p.harness.map(Into::into), p.model))
        }
        MethodName::StartChat => {
            let p: wire::StartChatParams = decode(method, params)?;
            reply(api::start_chat(core, p.session_id))
        }
        MethodName::UpdateChatModel => {
            let p: wire::UpdateChatModelParams = decode(method, params)?;
            reply(api::update_chat_model(core, &p.session_id, &p.harness.into(), p.model.as_deref()))
        }
        MethodName::PrepareTurn => {
            let p: wire::PrepareTurnParams = decode(method, params)?;
            reply(api::prepare_turn(core, p.session_id, p.text))
        }
        MethodName::SendTurn => {
            let p: wire::SendTurnParams = decode(method, params)?;
            reply(api::send_turn(core, p.session_id, p.text))
        }
        MethodName::CompactSession => {
            let p: wire::CompactSessionParams = decode(method, params)?;
            reply(api::compact_session(core, &p.session_id))
        }
        MethodName::InterruptTurn => {
            let p: wire::InterruptTurnParams = decode(method, params)?;
            reply(api::interrupt_turn(core, &p.session_id))
        }
        MethodName::RefreshAccountUsage => reply(api::refresh_account_usage(core)),
        MethodName::StopSession => {
            let p: wire::StopSessionParams = decode(method, params)?;
            reply(api::stop_session(core, p.session_id))
        }

        MethodName::ResolveApproval => {
            let p: wire::ResolveApprovalParams = decode(method, params)?;
            reply(api::resolve_approval(
                core,
                &p.session_id,
                p.event_id,
                &unit_variant_wire_value(&p.decision),
            ))
        }

        MethodName::OpenTerminal => {
            let p: wire::OpenTerminalParams = decode(method, params)?;
            reply(api::open_terminal(core, &p.workspace_id))
        }
        MethodName::WriteTerminal => {
            let p: wire::WriteTerminalParams = decode(method, params)?;
            reply(api::write_terminal(core, &p.workspace_id, &p.data))
        }
        MethodName::ResizeTerminal => {
            let p: wire::ResizeTerminalParams = decode(method, params)?;
            reply(api::resize_terminal(core, &p.workspace_id, p.rows, p.cols))
        }

        MethodName::ListSlashCommands => reply(api::list_slash_commands(core)),
        MethodName::ResolveSlashCommand => {
            let p: wire::ResolveSlashCommandParams = decode(method, params)?;
            reply(api::resolve_slash_command(core, &p.text, &p.session_id))
        }

        MethodName::CreateCompletionPlan => {
            let p: wire::CreateCompletionPlanParams = decode(method, params)?;
            reply(api::create_completion_plan(
                core,
                &p.session_id,
                p.acceptance_criteria,
                p.changed_paths,
                p.repository_commands,
                p.markdown_projection,
                p.markdown_committed,
            ))
        }
        MethodName::RecordCompletionCheck => {
            let p: wire::RecordCompletionCheckParams = decode(method, params)?;
            let run = into_core(method, &p.run)?;
            reply(api::record_completion_check(core, &p.attempt_id, &run))
        }
        MethodName::WaiveCompletion => {
            let p: wire::WaiveCompletionParams = decode(method, params)?;
            reply(api::waive_completion(core, &p.attempt_id, &p.check_ids, &p.reason))
        }
        MethodName::RegisterVerifierManifest => {
            let p: wire::RegisterVerifierManifestParams = decode(method, params)?;
            let manifest = into_core(method, &p.manifest)?;
            reply(api::register_verifier_manifest(core, &p.source, &manifest))
        }
        MethodName::WorkspaceBaseDivergence => {
            let p: wire::WorkspaceBaseDivergenceParams = decode(method, params)?;
            reply(api::workspace_base_divergence(core, &p.session_id, p.fetch))
        }
        MethodName::RefreshWorkspaceBase => {
            let p: wire::RefreshWorkspaceBaseParams = decode(method, params)?;
            reply(api::refresh_workspace_base(core, &p.session_id))
        }
        MethodName::PendingWorkerAdoptions => {
            let p: wire::PendingWorkerAdoptionsParams = decode(method, params)?;
            reply(api::pending_worker_adoptions(core, &p.session_id))
        }
        MethodName::AdoptWorkerWorktree => {
            let p: wire::AdoptWorkerWorktreeParams = decode(method, params)?;
            reply(api::adopt_worker_worktree(core, &p.session_id))
        }
        MethodName::DiscardWorkerWorktree => {
            let p: wire::DiscardWorkerWorktreeParams = decode(method, params)?;
            reply(api::discard_worker_worktree(core, &p.session_id, &p.reason))
        }
        MethodName::VerifierCandidates => {
            let p: wire::VerifierCandidatesParams = decode(method, params)?;
            reply(api::verifier_candidates(core, &p.change_labels, p.available_capabilities))
        }

        MethodName::GetRouterPreferences => {
            let p: wire::GetRouterPreferencesParams = decode(method, params)?;
            reply(api::get_router_preferences(core, &p.workspace_id))
        }
        MethodName::UpdateRouterPreferences => {
            let p: wire::UpdateRouterPreferencesParams = decode(method, params)?;
            let preferences = into_core(method, &p.preferences)?;
            reply(api::update_router_preferences(core, &p.workspace_id, &preferences))
        }
        MethodName::RollbackRoutingPolicy => {
            let p: wire::RollbackRoutingPolicyParams = decode(method, params)?;
            reply(api::rollback_routing_policy(core, p.target_version, &p.explanation))
        }

        MethodName::GetModelSetup => reply(api::get_model_setup(core)),
        MethodName::RecommendedModelProfiles => reply(api::recommended_model_profiles(core)),
        MethodName::SaveModelProfiles => {
            let p: wire::SaveModelProfilesParams = decode(method, params)?;
            let profiles: Vec<_> = into_core(method, &p.profiles)?;
            reply(api::save_model_profiles(core, &profiles))
        }
        MethodName::ResetModelProfiles => reply(api::reset_model_profiles(core)),

        MethodName::GetConfigState => reply(api::get_config_state(core)),
        MethodName::SaveHarnessConfig => {
            let p: wire::SaveHarnessConfigParams = decode(method, params)?;
            reply(api::save_harness_config(core, into_core(method, &p.config)?))
        }
        MethodName::ResetHarnessConfig => {
            let p: wire::ResetHarnessConfigParams = decode(method, params)?;
            reply(api::reset_harness_config(core, &p.id))
        }
        MethodName::RefreshOpencodeCatalog => {
            let p: wire::RefreshOpencodeCatalogParams = decode(method, params)?;
            reply(api::refresh_opencode_catalog(core, p.directory))
        }
        MethodName::SetOpencodeProviderApiKey => {
            let p: wire::SetOpencodeProviderApiKeyParams = decode(method, params)?;
            reply(api::set_opencode_provider_api_key(core, &p.provider_id, &p.api_key, p.directory))
        }
        MethodName::RemoveOpencodeProviderAuth => {
            let p: wire::RemoveOpencodeProviderAuthParams = decode(method, params)?;
            reply(api::remove_opencode_provider_auth(core, &p.provider_id, p.directory))
        }
        MethodName::SaveAgentConfig => {
            let p: wire::SaveAgentConfigParams = decode(method, params)?;
            reply(api::save_agent_config(core, into_core(method, &p.agent)?))
        }
        MethodName::DeleteAgentConfig => {
            let p: wire::DeleteAgentConfigParams = decode(method, params)?;
            reply(api::delete_agent_config(core, &p.id))
        }
        MethodName::SetDefaultAgent => {
            let p: wire::SetDefaultAgentParams = decode(method, params)?;
            reply(api::set_default_agent(core, &p.id))
        }
        MethodName::ResetAllConfig => reply(api::reset_all_config(core)),

        MethodName::GetLearningState => reply(api::get_learning_state(core)),
        MethodName::RunLearning => {
            let p: wire::RunLearningParams = decode(method, params)?;
            let kind = match p.trigger_kind {
                wire::LocalLearningTriggerKind::Manual => learning_job::LearningTriggerKind::Manual,
                wire::LocalLearningTriggerKind::InApp => learning_job::LearningTriggerKind::InApp,
            };
            reply(api::run_learning(core, kind))
        }
        MethodName::CancelLearningRun => {
            let p: wire::CancelLearningRunParams = decode(method, params)?;
            reply(api::cancel_learning_run(core, &p.run_id))
        }
        MethodName::UpdateLearningSchedule => {
            let p: wire::UpdateLearningScheduleParams = decode(method, params)?;
            let schedule = into_core(method, &p.schedule)?;
            reply(api::update_learning_schedule(core, &schedule))
        }
        MethodName::RegisterLearningTrigger => {
            let p: wire::RegisterLearningTriggerParams = decode(method, params)?;
            reply(api::register_learning_trigger(
                core,
                external_trigger(p.kind),
                &p.registration_id,
                p.credential_ref.as_deref(),
                p.expires_at.as_deref(),
            ))
        }
        MethodName::GetLearningTriggerInstructions => {
            let p: wire::GetLearningTriggerInstructionsParams = decode(method, params)?;
            reply(api::get_learning_trigger_instructions(
                external_trigger(p.kind),
                &p.database_path,
                &p.registration_id,
            ))
        }
        MethodName::EnableLearningTrigger => {
            let p: wire::EnableLearningTriggerParams = decode(method, params)?;
            reply(api::enable_learning_trigger(core, external_trigger(p.kind), &p.registration_id))
        }
        MethodName::ApproveLearningRun => {
            let p: wire::ApproveLearningRunParams = decode(method, params)?;
            reply(api::approve_learning_run(core, &p.run_id))
        }

        MethodName::BrowserBridgeState => reply(api::browser_bridge_state(core)),
        MethodName::InstallBrowserNativeHost => reply(api::install_browser_native_host(core)),
        MethodName::BrowserAction => {
            let p: wire::BrowserActionParams = decode(method, params)?;
            reply(api::browser_action(core, into_core(method, &p.request)?))
        }
        MethodName::SetBrowserPermission => {
            let p: wire::SetBrowserPermissionParams = decode(method, params)?;
            reply(api::set_browser_permission(core, &unit_variant_wire_value(&p.permission)))
        }
        MethodName::ResolveBrowserApproval => {
            let p: wire::ResolveBrowserApprovalParams = decode(method, params)?;
            reply(api::resolve_browser_approval(core, &p.approval_id, p.allow))
        }
        MethodName::TakeoverBrowser => reply(api::takeover_browser(core)),
        MethodName::DetachBrowser => reply(api::detach_browser(core)),
        MethodName::RouteBrowser => {
            let p: wire::RouteBrowserParams = decode(method, params)?;
            encode(api::route_browser(into_core(method, &p.request)?))
        }
        MethodName::BrowserSkills => encode(api::browser_skills()),
        MethodName::ConfigureRemoteBrowser => {
            let p: wire::ConfigureRemoteBrowserParams = decode(method, params)?;
            let config = match &p.config {
                Some(config) => Some(into_core(method, config)?),
                None => None,
            };
            reply(api::configure_remote_browser(core, config))
        }
        MethodName::StartRemoteBrowser => {
            let p: wire::StartRemoteBrowserParams = decode(method, params)?;
            reply(api::start_remote_browser(core, &p.initial_url))
        }

        MethodName::MarketplaceCatalog => encode(api::marketplace_catalog()),
        MethodName::MarketplaceAppAuthStates => reply(api::marketplace_app_auth_states()),
        MethodName::ListManagedAgents => {
            reply_managed(api::list_managed_agents())
        }
        MethodName::InspectManagedAgent => {
            let p: wire::InspectManagedAgentParams = decode(method, params)?;
            reply_managed(api::inspect_managed_agent(&p.agent_id))
        }
        MethodName::InstallManagedAgent => {
            let p: wire::InstallManagedAgentParams = decode(method, params)?;
            reply_managed(api::install_managed_agent(&p.agent_id))
        }
        MethodName::RepairManagedAgent => {
            let p: wire::RepairManagedAgentParams = decode(method, params)?;
            reply_managed(api::repair_managed_agent(&p.agent_id))
        }
        MethodName::UninstallManagedAgent => {
            let p: wire::UninstallManagedAgentParams = decode(method, params)?;
            reply_managed(api::uninstall_managed_agent(core, &p.agent_id))
        }
        MethodName::MarketplaceAction => {
            let p: wire::MarketplaceActionParams = decode(method, params)?;
            reply(api::marketplace_action(
                into_core(method, &p.provider)?,
                &p.plugin_id,
                p.marketplace.as_deref(),
                into_core(method, &p.action)?,
            ))
        }

        MethodName::SkillCatalog => reply(api::skill_catalog(core)),
        MethodName::SkillSuggestions => {
            let p: wire::SkillSuggestionsParams = decode(method, params)?;
            reply(api::skill_suggestions(core, &p.query, into_core(method, &p.provider)?))
        }
        MethodName::PreviewSkillChange => {
            let p: wire::PreviewSkillChangeParams = decode(method, params)?;
            let targets: Vec<_> = into_core(method, &p.targets)?;
            reply(api::preview_skill_change(
                core,
                &p.skill_id,
                into_core(method, &p.action)?,
                &targets,
            ))
        }
        MethodName::ExecuteSkillChange => {
            let p: wire::ExecuteSkillChangeParams = decode(method, params)?;
            reply(api::execute_skill_change(core, &p.confirmation_id))
        }
    }
}

/// Deserialize a method's contracted params type. A method whose params are
/// all optional accepts an omitted `params` key as the empty object.
fn decode<T: DeserializeOwned>(method: MethodName, params: Option<Value>) -> Result<T, RpcError> {
    let value = params.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    serde_json::from_value(value).map_err(|error| {
        RpcError::new(
            ErrorCode::InvalidParams,
            format!("invalid params for {}: {error}", method.as_str()),
        )
    })
}

/// Convert a wire mirror into its core DTO through their (test-pinned)
/// identical JSON representation. A failure here means the mirror drifted —
/// an internal contract bug, never a client error.
fn into_core<W: Serialize, T: DeserializeOwned>(method: MethodName, value: &W) -> Result<T, RpcError> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            RpcError::new(
                ErrorCode::InternalError,
                format!(
                    "protocol mirror for {} no longer matches the runtime type: {error}",
                    method.as_str()
                ),
            )
        })
}

/// The wire string of a unit enum variant (e.g. `ApprovalDecision::Accept`
/// → `"accept"`), for api functions that take the validated set as `&str`.
fn unit_variant_wire_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("unit enum variants serialize as strings")
}

fn external_trigger(kind: wire::ExternalLearningTriggerKind) -> learning_job::LearningTriggerKind {
    match kind {
        wire::ExternalLearningTriggerKind::Codex => learning_job::LearningTriggerKind::Codex,
        wire::ExternalLearningTriggerKind::Claude => learning_job::LearningTriggerKind::Claude,
        wire::ExternalLearningTriggerKind::OpenCode => learning_job::LearningTriggerKind::OpenCode,
    }
}

/// Serialize a fallible api result into the response value.
fn reply<T: Serialize>(result: Result<T, BridgeError>) -> Result<Value, RpcError> {
    match result {
        Ok(value) => encode(value),
        Err(error) => Err(RpcError::new(ErrorCode::from(&error), error.to_string())),
    }
}

/// Serialize a managed-agent result, preserving its stable domain code.
///
/// A parallel of [`reply`] rather than a reuse of it: the seven managed-agent
/// conditions live in their own error type precisely so they keep their own
/// 3000-range codes instead of being flattened into `BridgeError::Invalid`.
fn reply_managed<T: Serialize>(
    result: Result<T, bridge_core::managed_agents::ManagedAgentError>,
) -> Result<Value, RpcError> {
    match result {
        Ok(value) => encode(value),
        Err(error) => Err(RpcError::new(ErrorCode::from(&error), error.to_string())),
    }
}

/// Serialize an infallible api result into the response value.
fn encode<T: Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|error| {
        RpcError::new(ErrorCode::InternalError, format!("result failed to serialize: {error}"))
    })
}

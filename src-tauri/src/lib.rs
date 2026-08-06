pub use bridge_core::{
    completion, learning_job, learning_router, model_profiles, policy_replay, router_replay,
    routing_policy,
};

use bridge_core::api;
use bridge_core::live_turn;
use bridge_core::model::*;
use bridge_core::{
    agent_config, browser_bridge, marketplace, opencode_adapter, secret_interception,
    skill_marketplace, slash,
};
use bridge_core::{start_health_server, BootConfig, BridgeCore, BridgeError};
use std::{
    path::PathBuf,
    sync::Arc,
};
use tauri::{AppHandle, Emitter, Manager, State};

// Every command below delegates to `bridge_core::api` — the host-agnostic body
// of each protocol method, shared with the `bridged` daemon. The shell's only
// concerns are Tauri argument decoding and blocking-pool placement: anything
// that can touch Git, processes, PTYs, or the network runs via
// `spawn_blocking` so native work never lands on the macOS UI thread.

/// Run a blocking api call on the blocking pool with a labeled failure.
async fn blocking<T, F>(task_label: &'static str, work: F) -> Result<T, BridgeError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, BridgeError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| BridgeError::Invalid(format!("{task_label} task failed: {error}")))?
}

#[tauri::command]
async fn health(state: State<'_, Arc<BridgeCore>>) -> Result<api::Health, BridgeError> {
    api::health(state.inner())
}

#[tauri::command]
async fn browser_bridge_state(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<browser_bridge::BrowserBridgeSnapshot, BridgeError> {
    api::browser_bridge_state(state.inner())
}

#[tauri::command]
async fn install_browser_native_host(state: State<'_, Arc<BridgeCore>>) -> Result<String, BridgeError> {
    let core = state.inner().clone();
    blocking("Native host registration", move || api::install_browser_native_host(&core)).await
}

#[tauri::command]
async fn browser_action(
    request: browser_bridge::BrowserActionRequest,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<String, BridgeError> {
    api::browser_action(state.inner(), request)
}

#[tauri::command]
async fn set_browser_permission(
    permission: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::set_browser_permission(state.inner(), &permission)
}

#[tauri::command]
async fn resolve_browser_approval(
    approval_id: String,
    allow: bool,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::resolve_browser_approval(state.inner(), &approval_id, allow)
}

#[tauri::command]
async fn takeover_browser(state: State<'_, Arc<BridgeCore>>) -> Result<(), BridgeError> {
    api::takeover_browser(state.inner())
}

#[tauri::command]
async fn detach_browser(state: State<'_, Arc<BridgeCore>>) -> Result<String, BridgeError> {
    api::detach_browser(state.inner())
}

#[tauri::command]
async fn route_browser(
    request: browser_bridge::BrowserRouteRequest,
) -> browser_bridge::BrowserRouteDecision {
    api::route_browser(request)
}

#[tauri::command]
async fn browser_skills() -> Vec<browser_bridge::BrowserSkill> {
    api::browser_skills()
}

#[tauri::command]
async fn configure_remote_browser(
    config: Option<browser_bridge::RemoteBrowserConfig>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    let core = state.inner().clone();
    blocking("Remote browser configuration", move || {
        api::configure_remote_browser(&core, config)
    })
    .await
}

#[tauri::command]
async fn start_remote_browser(
    initial_url: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<serde_json::Value, BridgeError> {
    let core = state.inner().clone();
    blocking("Remote browser", move || api::start_remote_browser(&core, &initial_url)).await
}

#[tauri::command]
async fn marketplace_catalog() -> marketplace::MarketplaceCatalog {
    api::marketplace_catalog()
}

#[tauri::command]
async fn marketplace_app_auth_states(
) -> Result<Vec<marketplace::MarketplaceAppAuthState>, BridgeError> {
    api::marketplace_app_auth_states()
}

#[tauri::command]
async fn skill_catalog(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<skill_marketplace::SkillCatalog, BridgeError> {
    let core = state.inner().clone();
    blocking("Skill discovery", move || api::skill_catalog(&core)).await
}

#[tauri::command]
async fn skill_suggestions(
    query: String,
    provider: skill_marketplace::SkillProvider,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<skill_marketplace::CapabilitySuggestion>, BridgeError> {
    let core = state.inner().clone();
    blocking("Skill suggestion", move || api::skill_suggestions(&core, &query, provider)).await
}

#[tauri::command]
async fn preview_skill_change(
    skill_id: String,
    action: skill_marketplace::SkillAction,
    targets: Vec<skill_marketplace::SkillProvider>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<skill_marketplace::SkillPreview, BridgeError> {
    let core = state.inner().clone();
    blocking("Skill preview", move || {
        api::preview_skill_change(&core, &skill_id, action, &targets)
    })
    .await
}

#[tauri::command]
async fn execute_skill_change(
    confirmation_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<skill_marketplace::SkillActionResult>, BridgeError> {
    let core = state.inner().clone();
    blocking("Skill installer", move || api::execute_skill_change(&core, &confirmation_id)).await
}

#[tauri::command]
async fn marketplace_action(
    provider: marketplace::MarketplaceProvider,
    plugin_id: String,
    marketplace: Option<String>,
    action: marketplace::MarketplaceAction,
) -> Result<marketplace::MarketplaceActionResult, BridgeError> {
    api::marketplace_action(provider, &plugin_id, marketplace.as_deref(), action)
}

#[tauri::command]
async fn get_state(state: State<'_, Arc<BridgeCore>>) -> Result<BridgeState, BridgeError> {
    api::get_state(state.inner())
}

#[tauri::command]
async fn get_session_forest(
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<SessionForestSnapshot, BridgeError> {
    // Git may be slow on large repositories or during index contention. Never
    // run it on the macOS event loop or while holding the global SQLite lock.
    let core = state.inner().clone();
    blocking("Repository refresh", move || api::get_session_forest(&core, &session_id)).await
}

#[tauri::command]
async fn create_completion_plan(
    session_id: String,
    acceptance_criteria: Vec<String>,
    changed_paths: Vec<String>,
    repository_commands: Vec<String>,
    markdown_projection: Option<String>,
    markdown_committed: bool,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<completion::CompletionSummary, BridgeError> {
    let core = state.inner().clone();
    blocking("Completion planning", move || {
        api::create_completion_plan(
            &core,
            &session_id,
            acceptance_criteria,
            changed_paths,
            repository_commands,
            markdown_projection,
            markdown_committed,
        )
    })
    .await
}

#[tauri::command]
async fn record_completion_check(
    attempt_id: String,
    run: completion::CheckRun,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<completion::CompletionSummary, BridgeError> {
    api::record_completion_check(state.inner(), &attempt_id, &run)
}

#[tauri::command]
async fn waive_completion(
    attempt_id: String,
    check_ids: Vec<String>,
    reason: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<completion::CompletionSummary, BridgeError> {
    api::waive_completion(state.inner(), &attempt_id, &check_ids, &reason)
}

#[tauri::command]
async fn register_verifier_manifest(
    source: String,
    manifest: completion::VerifierManifest,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::register_verifier_manifest(state.inner(), &source, &manifest)
}

#[tauri::command]
async fn verifier_candidates(
    change_labels: Vec<String>,
    available_capabilities: Vec<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<completion::VerifierCandidate>, BridgeError> {
    api::verifier_candidates(state.inner(), &change_labels, available_capabilities)
}

#[tauri::command]
async fn get_router_preferences(
    workspace_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    api::get_router_preferences(state.inner(), &workspace_id)
}

#[tauri::command]
async fn update_router_preferences(
    workspace_id: String,
    preferences: learning_router::RouterPreferences,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    api::update_router_preferences(state.inner(), &workspace_id, &preferences)
}

#[tauri::command]
async fn get_model_setup(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    api::get_model_setup(state.inner())
}

#[tauri::command]
async fn recommended_model_profiles(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<model_profiles::ModelProfileDraft>, BridgeError> {
    api::recommended_model_profiles(state.inner())
}

#[tauri::command]
async fn save_model_profiles(
    profiles: Vec<model_profiles::ModelProfileDraft>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    api::save_model_profiles(state.inner(), &profiles)
}

#[tauri::command]
async fn reset_model_profiles(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    api::reset_model_profiles(state.inner())
}

#[tauri::command]
async fn get_config_state(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    api::get_config_state(state.inner())
}

#[tauri::command]
async fn save_harness_config(
    config: agent_config::HarnessConfig,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let core = state.inner().clone();
    blocking("Harness configuration", move || api::save_harness_config(&core, config)).await
}

#[tauri::command]
async fn reset_harness_config(
    id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let core = state.inner().clone();
    blocking("Harness reset", move || api::reset_harness_config(&core, &id)).await
}

#[tauri::command]
async fn refresh_opencode_catalog(
    directory: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let core = state.inner().clone();
    blocking("OpenCode discovery", move || api::refresh_opencode_catalog(&core, directory)).await
}

#[tauri::command]
async fn set_opencode_provider_api_key(
    provider_id: String,
    api_key: String,
    directory: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let core = state.inner().clone();
    blocking("OpenCode authentication", move || {
        api::set_opencode_provider_api_key(&core, &provider_id, &api_key, directory)
    })
    .await
}

#[tauri::command]
async fn remove_opencode_provider_auth(
    provider_id: String,
    directory: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let core = state.inner().clone();
    blocking("OpenCode authentication", move || {
        api::remove_opencode_provider_auth(&core, &provider_id, directory)
    })
    .await
}

#[tauri::command]
async fn save_agent_config(
    agent: agent_config::AgentDefinition,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    api::save_agent_config(state.inner(), agent)
}

#[tauri::command]
async fn delete_agent_config(
    id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    api::delete_agent_config(state.inner(), &id)
}

#[tauri::command]
async fn set_default_agent(
    id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    api::set_default_agent(state.inner(), &id)
}

#[tauri::command]
async fn reset_all_config(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let core = state.inner().clone();
    blocking("Configuration reset", move || api::reset_all_config(&core)).await
}

#[tauri::command]
async fn get_learning_state(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningState, BridgeError> {
    api::get_learning_state(state.inner())
}

#[tauri::command]
async fn run_learning(
    trigger_kind: learning_job::LearningTriggerKind,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningRun, BridgeError> {
    let core = state.inner().clone();
    blocking("Learning", move || api::run_learning(&core, trigger_kind)).await
}

#[tauri::command]
async fn cancel_learning_run(
    run_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningRun, BridgeError> {
    api::cancel_learning_run(state.inner(), &run_id)
}

#[tauri::command]
async fn update_learning_schedule(
    schedule: learning_job::LearningSchedule,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningSchedule, BridgeError> {
    api::update_learning_schedule(state.inner(), &schedule)
}

#[tauri::command]
async fn register_learning_trigger(
    kind: learning_job::LearningTriggerKind,
    registration_id: String,
    credential_ref: Option<String>,
    expires_at: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::register_learning_trigger(
        state.inner(),
        kind,
        &registration_id,
        credential_ref.as_deref(),
        expires_at.as_deref(),
    )
}

#[tauri::command]
async fn get_learning_trigger_instructions(
    kind: learning_job::LearningTriggerKind,
    database_path: String,
    registration_id: String,
) -> Result<String, BridgeError> {
    api::get_learning_trigger_instructions(kind, &database_path, &registration_id)
}

#[tauri::command]
async fn enable_learning_trigger(
    kind: learning_job::LearningTriggerKind,
    registration_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::enable_learning_trigger(state.inner(), kind, &registration_id)
}

#[tauri::command]
async fn approve_learning_run(
    run_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningRun, BridgeError> {
    api::approve_learning_run(state.inner(), &run_id)
}

#[tauri::command]
async fn rollback_routing_policy(
    target_version: i64,
    explanation: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<learning_job::LearningState, BridgeError> {
    api::rollback_routing_policy(state.inner(), target_version, &explanation)
}

#[tauri::command]
async fn activate_session_entry(
    session_id: String,
    entry_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<SessionForestSnapshot, BridgeError> {
    api::activate_session_entry(state.inner(), &session_id, &entry_id)
}

#[tauri::command]
async fn add_project(path: String, state: State<'_, Arc<BridgeCore>>) -> Result<BridgeState, BridgeError> {
    api::add_project(state.inner(), &path)
}

/// Create a repo-less workspace. A folder/git repo can be connected later.
#[tauri::command]
async fn create_workspace(
    title: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    api::create_workspace(state.inner(), &title)
}

/// Create a standalone direct chat (no workspace). Runs in a private scratch dir.
#[tauri::command]
async fn create_chat(
    harness: Harness,
    model: Option<String>,
    title: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    api::create_chat(state.inner(), &harness, model.as_deref(), title.as_deref())
}

/// Create an orchestrator session inside a workspace (the classic Bridge agent
/// that plans and delegates to workers). Multiple are allowed per workspace.
#[tauri::command]
async fn create_workspace_session(
    workspace_id: String,
    create_worktree: Option<bool>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    // Worktree creation shells out to Git; keep it on the blocking pool.
    let core = state.inner().clone();
    blocking("Worktree creation", move || {
        api::create_workspace_session(&core, &workspace_id, create_worktree.unwrap_or(false))
    })
    .await
}

/// Change a root chat's provider/model. Stops any running adapter so the next
/// message starts a fresh provider session with the explicit user selection.
#[tauri::command]
async fn update_chat_model(
    session_id: String,
    harness: Harness,
    model: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    // Stopping the old adapter can block on process teardown; run the whole
    // claim -> teardown -> commit window on the blocking pool.
    let core = state.inner().clone();
    blocking("Adapter shutdown", move || {
        api::update_chat_model(&core, &session_id, &harness, model.as_deref())
    })
    .await
}

/// Enumerate slash commands + skills from every signed-in provider, so the UI
/// can offer a labeled `/` menu.
#[tauri::command]
async fn list_slash_commands(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<slash::SlashCommand>, BridgeError> {
    api::list_slash_commands(state.inner())
}

/// Resolve a composer `/command` against the catalog so the UI can auto-switch
/// harness before sending.
#[tauri::command]
async fn resolve_slash_command(
    text: String,
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Option<api::SlashCommandResolve>, BridgeError> {
    api::resolve_slash_command(state.inner(), &text, &session_id)
}

/// Attach a folder (optionally a git repo) to a workspace as its working directory.
#[tauri::command]
async fn connect_workspace_folder(
    workspace_id: String,
    path: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    api::connect_workspace_folder(state.inner(), &workspace_id, &path)
}

#[tauri::command]
async fn start_session(
    workspace_id: String,
    harness: Option<Harness>,
    model: Option<String>,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    let core = state.inner().clone();
    blocking("Session start", move || api::start_session(&core, workspace_id, harness, model))
        .await
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with the
/// routing briefing + delegation protocol (workers enabled via the reader gate).
#[tauri::command]
async fn start_chat(
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    let core = state.inner().clone();
    blocking("Chat start", move || api::start_chat(&core, session_id)).await
}

#[tauri::command]
async fn open_terminal(
    workspace_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    let core = state.inner().clone();
    blocking("Terminal open", move || api::open_terminal(&core, &workspace_id)).await
}

#[tauri::command]
async fn write_terminal(
    workspace_id: String,
    data: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::write_terminal(state.inner(), &workspace_id, &data)
}

#[tauri::command]
async fn prepare_turn(
    session_id: String,
    text: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<secret_interception::SanitizedTurn, BridgeError> {
    let core = state.inner().clone();
    blocking("Turn preparation", move || api::prepare_turn(&core, session_id, text)).await
}

#[tauri::command]
async fn send_turn(
    session_id: String,
    text: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    let core = state.inner().clone();
    blocking("Turn delivery", move || api::send_turn(&core, session_id, text)).await
}

/// List the current chat's workspace files for the composer's `@file`
/// autocomplete. Returns an empty list for chats with no connected folder.
#[tauri::command]
async fn list_workspace_files(
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<Vec<String>, BridgeError> {
    // Listing is pure filesystem work; keep it off the async runtime.
    let core = state.inner().clone();
    blocking("Workspace file listing", move || api::list_workspace_files(&core, &session_id))
        .await
}

#[tauri::command]
async fn compact_session(
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::compact_session(state.inner(), &session_id)
}

/// Replay durable session events after a cursor — the recovery half of the
/// notify-then-replay event contract.
#[tauri::command]
async fn replay_session_events(
    session_id: String,
    after_sequence: i64,
    limit: Option<u32>,
    app: AppHandle,
) -> Result<Vec<AgentEvent>, BridgeError> {
    blocking("Session replay", move || {
        api::replay_session_events(
            &app.state::<Arc<BridgeCore>>(),
            &session_id,
            after_sequence,
            limit,
        )
    })
    .await
}

#[tauri::command]
async fn interrupt_turn(session_id: String, state: State<'_, Arc<BridgeCore>>) -> Result<(), BridgeError> {
    api::interrupt_turn(state.inner(), &session_id)
}

/// Refresh subscription usage for every provider, independent of which session
/// is on screen. Claude is queried out-of-band via its headless `/usage`
/// command; Codex is asked on a live session and answers on its event stream.
/// Both results are broadcast on the `account-usage` channel.
#[tauri::command]
async fn refresh_account_usage(
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::refresh_account_usage(state.inner())
}

#[tauri::command]
async fn resolve_approval(
    session_id: String,
    event_id: i64,
    decision: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::resolve_approval(state.inner(), &session_id, event_id, &decision)
}

#[tauri::command]
async fn resize_terminal(
    workspace_id: String,
    rows: u16,
    cols: u16,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<(), BridgeError> {
    api::resize_terminal(state.inner(), &workspace_id, rows, cols)
}

#[tauri::command]
async fn stop_session(
    session_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    let core = state.inner().clone();
    blocking("Session stop", move || api::stop_session(&core, session_id)).await
}

#[tauri::command]
async fn refresh_workspace(
    workspace_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    // Git status scans run entirely off the async runtime so a slow scan
    // cannot delay message submission or streaming writes.
    let core = state.inner().clone();
    blocking("Workspace refresh", move || api::refresh_workspace(&core, &workspace_id)).await
}

#[tauri::command]
async fn archive_workspace(
    workspace_id: String,
    state: State<'_, Arc<BridgeCore>>,
) -> Result<BridgeState, BridgeError> {
    api::archive_workspace(state.inner(), &workspace_id)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            // Embedded mode is one of the two allowed owners of a data
            // directory (the other is the bridged daemon), never both at
            // once. Acquire the exclusive lease before touching any store;
            // the lease lives as managed state until the process exits.
            let lease = bridge_core::ownership::DataDirLease::acquire(
                &data,
                bridge_core::ownership::OwnerKind::Embedded,
            )
            .map_err(|error| {
                eprintln!("bridge: {error}");
                Box::<dyn std::error::Error>::from(error.to_string())
            })?;
            app.manage(lease);
            let bundled_extension = app.path().resource_dir()?.join("browser-extension");
            let extension_path = if bundled_extension.exists() {
                bundled_extension
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension")
            };
            // The Tauri compatibility adapter: subscribe BEFORE boot so
            // boot-time events (adapter discovery) cannot be missed, then
            // forward every core event to the webview with unchanged names
            // and payloads.
            let events = bridge_core::events::EventBus::new();
            let mut receiver = events.subscribe();
            let forwarder = app.handle().clone();
            std::thread::Builder::new()
                .name("core-event-forwarder".into())
                .spawn(move || loop {
                    match receiver.blocking_recv() {
                        Ok(event) => {
                            let _ = forwarder.emit(event.kind().as_str(), event.payload());
                        }
                        // The compatibility UI already reconciles durable
                        // history from the session forest. Skip stale live
                        // frames here; daemon clients use cursor replay.
                        Err(bridge_core::events::ReceiveError::Lagged(_)) => {
                            for event in receiver.reconciliation_events() {
                                let _ = forwarder.emit(event.kind().as_str(), event.payload());
                            }
                            continue;
                        }
                        Err(bridge_core::events::ReceiveError::Closed) => break,
                    }
                })?;
            let core = BridgeCore::boot(BootConfig {
                data_dir: data,
                browser_extension_path: extension_path,
                events: Some(events),
            })
            .map_err(Box::<dyn std::error::Error>::from)?;
            start_health_server(
                core.database_path.clone(),
                core.adapter_registry.descriptors(),
                core.credential_broker.clone(),
            );
            let core = Arc::new(core);
            app.manage(core.clone());
            live_turn::start_worker_maintenance(core.clone());
            live_turn::start_learning_maintenance(core.clone());
            live_turn::start_history_snapshot_maintenance(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            browser_bridge_state,
            install_browser_native_host,
            browser_action,
            set_browser_permission,
            resolve_browser_approval,
            takeover_browser,
            detach_browser,
            route_browser,
            browser_skills,
            configure_remote_browser,
            start_remote_browser,
            marketplace_catalog,
            marketplace_app_auth_states,
            marketplace_action,
            skill_catalog,
            skill_suggestions,
            preview_skill_change,
            execute_skill_change,
            get_state,
            get_session_forest,
            replay_session_events,
            create_completion_plan,
            record_completion_check,
            waive_completion,
            register_verifier_manifest,
            verifier_candidates,
            get_router_preferences,
            update_router_preferences,
            get_model_setup,
            recommended_model_profiles,
            save_model_profiles,
            reset_model_profiles,
            get_config_state,
            save_harness_config,
            reset_harness_config,
            refresh_opencode_catalog,
            set_opencode_provider_api_key,
            remove_opencode_provider_auth,
            save_agent_config,
            delete_agent_config,
            set_default_agent,
            reset_all_config,
            get_learning_state,
            run_learning,
            cancel_learning_run,
            update_learning_schedule,
            register_learning_trigger,
            get_learning_trigger_instructions,
            enable_learning_trigger,
            approve_learning_run,
            rollback_routing_policy,
            activate_session_entry,
            add_project,
            create_workspace,
            create_chat,
            create_workspace_session,
            connect_workspace_folder,
            update_chat_model,
            list_slash_commands,
            resolve_slash_command,
            start_session,
            start_chat,
            open_terminal,
            write_terminal,
            resize_terminal,
            prepare_turn,
            send_turn,
            list_workspace_files,
            compact_session,
            interrupt_turn,
            refresh_account_usage,
            resolve_approval,
            stop_session,
            refresh_workspace,
            archive_workspace
        ])
        .run(tauri::generate_context!())
        .expect("Bridge failed to start")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_core::live_turn::{
        agent_event_changes_bridge_state, begin_pressure_compaction, cross_harness_reuse_marker,
        deliver_sanitized_turn, deliver_worker_objective, persist_prompt_compilation,
        persist_submitted_user_turn, prepare_worker_failure_settlement,
        process_worker_result_output, record_actual_execution_best_effort,
        record_model_resolution_warning, reserve_worker_launch, reserve_worker_launch_outcome,
        resolve_policy_delegation_approval, WorkerReservationOutcome, HISTORY_SNAPSHOT_INTERVAL,
    };
    use bridge_core::workspaces;
    use bridge_core::{
        adapters, agent, compaction_controller, delegation, git, policy, prompt_compiler,
        session_forest, session_supervisor, sessions, store, worker_lifecycle,
    };
    use rusqlite::{params, Connection};
    use std::collections::HashMap;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::Duration;

    #[test]
    fn orchestrator_start_uses_the_persisted_standard_profile() {
        let registry = adapters::AdapterRegistry::built_in().unwrap();
        let descriptors = registry.descriptors();
        let Ok(mut profiles) = model_profiles::recommended_profiles(&descriptors) else {
            // Provider-binary availability is environment-owned. Catalog/profile
            // resolution itself is covered with a deterministic fake catalog.
            return;
        };
        let expected = profiles
            .iter_mut()
            .find(|profile| profile.purpose == model_profiles::ProfilePurpose::StandardOrchestrator)
            .unwrap();
        expected.effort = delegation::Effort::High;
        let expected_provider = expected.provider.clone();
        let expected_model = expected.model.clone();
        let db = store::open(Path::new(":memory:")).unwrap();
        model_profiles::save_profiles(&db, &descriptors, &profiles).unwrap();
        let selected = sessions::resolve_orchestrator_selection(&db, &registry).unwrap();
        assert_eq!(selected.adapter_id, expected_provider);
        assert_eq!(selected.model, expected_model);
        assert_eq!(selected.effort, Some(delegation::Effort::High));
        assert_eq!(selected.tier, CapabilityTier::Standard);
    }

    #[test]
    fn checkpoint_prompt_records_cross_harness_compatibility_without_prompt_contents() {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/cache-test','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Cache','bridge/cache','/tmp/cache-test','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('child','w','claude','Child','working','reported','parent')", []).unwrap();
        assert_eq!(
            cross_harness_reuse_marker(&db, "parent", "codex"),
            "same_harness"
        );
        assert_eq!(
            cross_harness_reuse_marker(&db, "parent", "claude"),
            "incompatible"
        );
        assert_eq!(
            cross_harness_reuse_marker(&db, "missing", "claude"),
            "not_applicable"
        );

        let prompt = prompt_compiler::PromptCompiler::new("worker:verification")
            .stable_section("contract", "Verify the task")
            .variable_section("restoration_context", "checkpoint evidence")
            .compile()
            .unwrap();
        persist_prompt_compilation(
            &db,
            "child",
            "claude",
            Some("sonnet"),
            "worker:verification",
            "verification",
            RestorationMode::CheckpointRestored,
            cross_harness_reuse_marker(&db, "parent", "claude"),
            &prompt,
        )
        .unwrap();
        let stored = store::latest_prompt_compilation(&db, "child")
            .unwrap()
            .unwrap();
        assert_eq!(stored.restoration_mode, "checkpoint_restored");
        assert_eq!(stored.cross_harness_reuse, "incompatible");
        assert_eq!(stored.prefix_hash, prompt.metadata.prefix_hash);
        assert!(!serde_json::to_string(&stored)
            .unwrap()
            .contains("Verify the task"));
        assert!(!serde_json::to_string(&stored)
            .unwrap()
            .contains("checkpoint evidence"));
    }

    #[test]
    fn orchestrator_worktree_is_created_from_the_connected_repository_head() {
        let fixture = tempfile::tempdir().unwrap();
        let repo = fixture.path().join("repository");
        std::fs::create_dir(&repo).unwrap();
        let run_git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "bridge-test@example.invalid"]);
        run_git(&["config", "user.name", "Bridge Test"]);
        std::fs::write(repo.join("README.md"), "base\n").unwrap();
        run_git(&["add", "."]);
        run_git(&["commit", "-m", "fixture", "-q"]);

        let created = sessions::prepare_orchestrator_worktree(
            &fixture.path().join("managed-worktrees"),
            "Payments / API",
            &repo,
            "12345678-abcd",
        )
        .unwrap();

        assert_eq!(created.branch, "bridge/payments-api-12345678");
        assert_eq!(
            std::fs::read_to_string(created.path.join("README.md")).unwrap(),
            "base\n"
        );
        assert_eq!(
            git::current_branch(&created.path).as_deref(),
            Some(created.branch.as_str())
        );
    }

    #[test]
    fn tauri_commands_never_block_the_ui_thread() {
        let source = include_str!("lib.rs");
        assert!(
            !source.contains("#[tauri::command]\nfn "),
            "Tauri commands must be async so native work never runs on the macOS UI thread"
        );
        // Method bodies live in bridge_core::api (shared with the daemon);
        // the shell adds transport wiring only. A command that does not call
        // through the api seam means logic leaked back into the shell.
        let command_region = source
            .split("#[cfg(test)]")
            .next()
            .expect("lib.rs has a test module");
        let attribute = format!("#[tauri::{}]", "command"); // dodge this literal
        let stray: Vec<&str> = command_region
            .split(&attribute)
            .skip(1)
            .filter(|body| !body.contains("api::"))
            .map(|body| body.trim_start().lines().next().unwrap_or_default())
            .collect();
        assert!(stray.is_empty(), "commands not delegating to bridge_core::api: {stray:?}");
    }

    #[test]
    fn the_shell_never_emits_a_literal_event_name() {
        // Every notification flows through the core event bus; the setup
        // forwarder (which emits `event.kind().as_str()`) is the only code
        // that touches Tauri's event system. A literal event name in an
        // emit call means someone bypassed the bus — and broke the durable
        // replay contract for that event.
        let source = include_str!("lib.rs");
        assert_eq!(
            source.matches(".emit(\"").count(),
            0,
            "publish CoreEvent on state.events instead of emitting directly"
        );
    }

    #[test]
    fn the_protocol_contract_matches_the_registered_command_surface() {
        let source = include_str!("lib.rs");
        let start = source.find("generate_handler![").expect("command registry")
            + "generate_handler![".len();
        let end = start + source[start..].find(']').expect("registry end");
        let commands: Vec<&str> = source[start..end]
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        assert!(!commands.is_empty());
        for command in &commands {
            assert!(
                bridge_protocol::MethodName::from_command(command).is_some(),
                "command {command} is registered with Tauri but missing from the \
                 bridge-protocol method registry"
            );
        }
        assert_eq!(
            commands.len(),
            bridge_protocol::MethodName::ALL.len(),
            "bridge-protocol declares methods for commands that are not registered; \
             the registry and generate_handler![...] must stay 1:1"
        );
    }

    #[test]
    fn every_command_signature_matches_its_contracted_params() {
        // The contract's params structs are hand-written mirrors of these
        // signatures. Compare both wire names and JSON-relevant Rust types so
        // a rename or retype fails here rather than in daemon dispatch.
        let source = include_str!("lib.rs");
        for method in bridge_protocol::MethodName::ALL.iter().copied() {
            let command = command_arguments(source, method.command_name());
            let contract = bridge_protocol::TypedMethod::params_schema_fields(method);
            match (command, contract) {
                (None, None) => {}
                (Some(command), Some(contract)) => {
                    let command_names: Vec<&str> = command
                        .iter()
                        .map(|argument| argument.name.as_str())
                        .collect();
                    let contract_names: Vec<&str> =
                        contract.iter().map(|(name, _)| name.as_str()).collect();
                    assert_eq!(
                        command_names,
                        contract_names,
                        "{} takes different arguments than its contract names",
                        method.as_str()
                    );
                    for (argument, (_, schema)) in command.iter().zip(contract.iter()) {
                        assert_eq!(
                            rust_parameter_shape(method, &argument.name, &argument.kind),
                            schema_parameter_shape(schema),
                            "{} parameter {} has a different type from its contract",
                            method.as_str(),
                            argument.name
                        );
                    }
                }
                (command, contract) => panic!(
                    "{} parameterlessness drifted: command={command:?}, contract={contract:?}",
                    method.as_str()
                ),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CommandArgument {
        name: String,
        kind: String,
    }

    /// The non-injected arguments a Tauri command accepts, sorted by their
    /// camelCase wire names. Whitespace is removed from Rust types so multiline
    /// signatures compare consistently.
    fn command_arguments(source: &str, command: &str) -> Option<Vec<CommandArgument>> {
        let needle = format!("async fn {command}(");
        let start = source
            .find(&needle)
            .unwrap_or_else(|| panic!("no async fn named {command} in the shell"))
            + needle.len();
        // Split the parameter list on top-level commas: generic arguments
        // (`State<'_, Arc<BridgeCore>>`) carry commas of their own.
        let mut depth = 0usize;
        let mut parameters: Vec<String> = Vec::new();
        let mut current = String::new();
        for character in source[start..].chars() {
            match character {
                ')' if depth == 0 => break,
                ',' if depth == 0 => parameters.push(std::mem::take(&mut current)),
                _ => {
                    match character {
                        '(' | '<' => depth += 1,
                        ')' | '>' => depth -= 1,
                        _ => {}
                    }
                    current.push(character);
                }
            }
        }
        parameters.push(current);

        let mut arguments: Vec<CommandArgument> = parameters
            .iter()
            .filter_map(|parameter| {
                let (name, kind) = parameter.split_once(':')?;
                let kind: String = kind
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect();
                // Tauri injects these; a client never sends them.
                if kind.contains("State<") || kind.contains("AppHandle") {
                    return None;
                }
                Some(CommandArgument {
                    name: camel_case(name.trim()),
                    kind,
                })
            })
            .collect();
        arguments.sort_by(|left, right| left.name.cmp(&right.name));
        (!arguments.is_empty()).then_some(arguments)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ParameterShape {
        String,
        Boolean,
        Integer(String),
        Number,
        Reference(String),
        Array(Box<ParameterShape>),
        Optional(Box<ParameterShape>),
    }

    fn rust_parameter_shape(
        method: bridge_protocol::MethodName,
        field: &str,
        kind: &str,
    ) -> ParameterShape {
        if let Some(inner) = generic_inner(kind, "Option") {
            return ParameterShape::Optional(Box::new(rust_parameter_shape(method, field, inner)));
        }
        if let Some(inner) = generic_inner(kind, "Vec") {
            return ParameterShape::Array(Box::new(rust_parameter_shape(method, field, inner)));
        }

        let leaf = kind.rsplit("::").next().unwrap_or(kind);
        match leaf {
            "String" => match (method, field) {
                (bridge_protocol::MethodName::ResolveApproval, "decision") => {
                    ParameterShape::Reference("ApprovalDecision".into())
                }
                (bridge_protocol::MethodName::SetBrowserPermission, "permission") => {
                    ParameterShape::Reference("BrowserPermission".into())
                }
                _ => ParameterShape::String,
            },
            "bool" => ParameterShape::Boolean,
            "i64" | "u16" | "u32" => ParameterShape::Integer(
                match leaf {
                    "i64" => "int64",
                    "u16" => "uint16",
                    "u32" => "uint32",
                    _ => unreachable!(),
                }
                .into(),
            ),
            "f64" => ParameterShape::Number,
            "Harness" => ParameterShape::Reference("HarnessId".into()),
            "LearningTriggerKind"
                if field == "kind"
                    && matches!(
                        method,
                        bridge_protocol::MethodName::RegisterLearningTrigger
                            | bridge_protocol::MethodName::GetLearningTriggerInstructions
                            | bridge_protocol::MethodName::EnableLearningTrigger
                    ) =>
            {
                ParameterShape::Reference("ExternalLearningTriggerKind".into())
            }
            "LearningTriggerKind" if method == bridge_protocol::MethodName::RunLearning => {
                ParameterShape::Reference("LocalLearningTriggerKind".into())
            }
            reference => ParameterShape::Reference(reference.into()),
        }
    }

    fn generic_inner<'a>(kind: &'a str, container: &str) -> Option<&'a str> {
        kind.strip_prefix(container)?
            .strip_prefix('<')?
            .strip_suffix('>')
    }

    fn schema_parameter_shape(schema: &serde_json::Value) -> ParameterShape {
        if let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) {
            return ParameterShape::Reference(reference.rsplit('/').next().unwrap().into());
        }
        if let Some(parts) = schema.get("allOf").and_then(serde_json::Value::as_array) {
            assert_eq!(parts.len(), 1, "unsupported allOf params schema: {schema}");
            return schema_parameter_shape(&parts[0]);
        }
        if let Some(options) = schema.get("anyOf").and_then(serde_json::Value::as_array) {
            let non_null: Vec<&serde_json::Value> = options
                .iter()
                .filter(|option| {
                    option.get("type").and_then(serde_json::Value::as_str) != Some("null")
                })
                .collect();
            assert_eq!(
                non_null.len(),
                1,
                "unsupported anyOf params schema: {schema}"
            );
            return ParameterShape::Optional(Box::new(schema_parameter_shape(non_null[0])));
        }

        match schema.get("type") {
            Some(serde_json::Value::String(kind)) => schema_type_shape(kind, schema),
            Some(serde_json::Value::Array(kinds)) => {
                let non_null: Vec<&str> = kinds
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|kind| *kind != "null")
                    .collect();
                assert_eq!(
                    non_null.len(),
                    1,
                    "unsupported union params schema: {schema}"
                );
                ParameterShape::Optional(Box::new(schema_type_shape(non_null[0], schema)))
            }
            _ => panic!("unsupported params schema: {schema}"),
        }
    }

    fn schema_type_shape(kind: &str, schema: &serde_json::Value) -> ParameterShape {
        match kind {
            "string" => ParameterShape::String,
            "boolean" => ParameterShape::Boolean,
            "integer" => ParameterShape::Integer(
                schema
                    .get("format")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("integer")
                    .into(),
            ),
            "number" => ParameterShape::Number,
            "array" => ParameterShape::Array(Box::new(schema_parameter_shape(
                schema
                    .get("items")
                    .expect("array params schemas declare items"),
            ))),
            _ => panic!("unsupported params type {kind}: {schema}"),
        }
    }

    #[test]
    fn signature_type_comparison_covers_scalars_collections_and_narrowed_enums() {
        use bridge_protocol::MethodName;

        assert_eq!(
            rust_parameter_shape(MethodName::ReplaySessionEvents, "limit", "Option<u32>"),
            ParameterShape::Optional(Box::new(ParameterShape::Integer("uint32".into())))
        );
        assert_eq!(
            rust_parameter_shape(MethodName::ResizeTerminal, "rows", "u16"),
            ParameterShape::Integer("uint16".into())
        );
        assert_eq!(
            rust_parameter_shape(MethodName::ReplaySessionEvents, "after", "i64"),
            ParameterShape::Integer("int64".into())
        );
        assert_eq!(
            rust_parameter_shape(MethodName::SaveAgentConfig, "args", "Vec<String>"),
            ParameterShape::Array(Box::new(ParameterShape::String))
        );
        assert_eq!(
            rust_parameter_shape(
                MethodName::SaveModelProfiles,
                "profiles",
                "Vec<model_profiles::ModelProfileDraft>"
            ),
            ParameterShape::Array(Box::new(ParameterShape::Reference(
                "ModelProfileDraft".into()
            )))
        );
        assert_eq!(
            rust_parameter_shape(MethodName::ResolveApproval, "decision", "String"),
            ParameterShape::Reference("ApprovalDecision".into())
        );
        assert_eq!(
            rust_parameter_shape(
                MethodName::RegisterLearningTrigger,
                "kind",
                "learning_job::LearningTriggerKind"
            ),
            ParameterShape::Reference("ExternalLearningTriggerKind".into())
        );
        assert_eq!(
            rust_parameter_shape(
                MethodName::RunLearning,
                "triggerKind",
                "learning_job::LearningTriggerKind"
            ),
            ParameterShape::Reference("LocalLearningTriggerKind".into())
        );
    }

    fn camel_case(snake: &str) -> String {
        let mut out = String::with_capacity(snake.len());
        let mut capitalize = false;
        for character in snake.chars() {
            if character == '_' {
                capitalize = true;
            } else if capitalize {
                out.push(character.to_ascii_uppercase());
                capitalize = false;
            } else {
                out.push(character);
            }
        }
        out
    }

    #[test]
    fn evidence_recording_failure_is_not_load_bearing_for_worker_launch() {
        let db = Connection::open_in_memory().unwrap();
        record_actual_execution_best_effort(
            &db,
            "missing-decision",
            "codex",
            "model",
            delegation::Effort::Medium,
            "missing-parent",
        );
    }

    struct RecordingRuntime {
        sent: Arc<Mutex<Vec<String>>>,
    }

    impl adapters::AdapterRuntime for RecordingRuntime {
        fn process_id(&self) -> u32 {
            0
        }
        fn provider_session_id(&self) -> &str {
            "recording"
        }
        fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
            Arc::new(Mutex::new(None))
        }
        fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
            self.sent.lock().unwrap().push(text.into());
            Ok(())
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn respond(
            &self,
            _request_id: serde_json::Value,
            _decision: &str,
        ) -> Result<(), BridgeError> {
            Ok(())
        }
        fn stop(&mut self, _reason: adapters::ShutdownReason) {}
    }

    struct RejectingRuntime;

    impl adapters::AdapterRuntime for RejectingRuntime {
        fn process_id(&self) -> u32 {
            0
        }
        fn provider_session_id(&self) -> &str {
            "rejecting"
        }
        fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
            Arc::new(Mutex::new(None))
        }
        fn send_turn(&self, _text: &str) -> Result<(), BridgeError> {
            Err(BridgeError::Invalid("delivery rejected".into()))
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn respond(
            &self,
            _request_id: serde_json::Value,
            _decision: &str,
        ) -> Result<(), BridgeError> {
            Ok(())
        }
        fn stop(&mut self, _reason: adapters::ShutdownReason) {}
    }

    fn policy_request(paths: &[&str]) -> delegation::DelegationRequest {
        delegation::DelegationRequest {
            schema_version: 1,
            role: delegation::WorkerRole::Implementation,
            objective: "Implement auth".into(),
            acceptance_criteria: vec!["Tests pass".into()],
            known_facts: Vec::new(),
            decisions: Vec::new(),
            evidence_ids: Vec::new(),
            relevant_files: Vec::new(),
            owned_paths: paths.iter().map(|path| (*path).into()).collect(),
            write_mode: delegation::WriteMode::Isolated,
            capability_tier: delegation::CapabilityTier::Standard,
            effort: delegation::Effort::Medium,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec!["cargo test".into()],
            output_contract: delegation::OutputContract::ImplementationResult,
            harness: Some("codex".into()),
            model: None,
        }
    }

    #[test]
    fn worker_objective_delivery_failure_is_not_reported_as_launched() {
        let adapters = Mutex::new(HashMap::from([(
            "worker".into(),
            Box::new(RejectingRuntime) as Box<dyn adapters::AdapterRuntime>,
        )]));
        assert!(deliver_worker_objective(&adapters, "worker", "do work").is_err());
        assert!(deliver_worker_objective(&adapters, "missing", "do work").is_err());
    }

    #[test]
    fn chat_secret_is_sanitized_before_harness_delivery() {
        let canary = "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ";
        let prepared = secret_interception::sanitize(&format!("review issue 42 with {canary}"));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let runtime = RecordingRuntime { sent: sent.clone() };

        deliver_sanitized_turn(&runtime, &prepared.text, None).unwrap();

        let delivered = sent.lock().unwrap().first().cloned().unwrap();
        assert!(!delivered.contains(canary));
        assert!(delivered.contains("[secret:sec_"));
    }

    #[test]
    fn only_global_state_mutations_request_a_full_state_reload() {
        let event = |kind: &str, status: Option<&str>| agent::NormalizedEvent {
            kind: kind.into(),
            item_id: None,
            role: None,
            status: status.map(str::to_owned),
            title: None,
            text: None,
            data: serde_json::json!({}),
        };
        for kind in [
            "turn.started",
            "turn.completed",
            "approval.requested",
            "usage.updated",
        ] {
            assert!(agent_event_changes_bridge_state(&event(kind, None)));
        }
        assert!(agent_event_changes_bridge_state(&event(
            "error",
            Some("failed")
        )));
        assert!(!agent_event_changes_bridge_state(&event(
            "message.delta",
            Some("streaming")
        )));
        assert!(!agent_event_changes_bridge_state(&event(
            "tool.completed",
            Some("completed")
        )));
        assert!(!agent_event_changes_bridge_state(&event(
            "provider.unknown",
            None
        )));
    }

    #[test]
    fn claude_history_persists_only_the_sanitized_user_turn() {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('secret-chat',NULL,'claude','Secret chat','working','reported','direct')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('secret-chat','fresh','now')",
            [],
        )
        .unwrap();
        let canary = "xoxb-123456789012-abcdefghijklmnop";
        let prepared = secret_interception::sanitize(&format!("post using {canary}"));

        persist_submitted_user_turn(&db, "secret-chat", "claude", &prepared.text)
            .unwrap()
            .unwrap();

        let serialized =
            serde_json::to_string(&store::session_entries(&db, "secret-chat").unwrap()).unwrap();
        assert!(!serialized.contains(canary));
        assert!(serialized.contains("[secret:sec_"));
    }

    #[test]
    fn post_start_delivery_failure_settles_and_releases_its_lease() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        let reservation = reserve_worker_launch(
            &db,
            "parent",
            "turn-delivery-failure",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap()
        .unwrap();
        prepare_worker_failure_settlement(&db, &reservation.session_id).unwrap();
        prepare_worker_failure_settlement(&db, &reservation.session_id).unwrap();
        session_supervisor::SessionSupervisor::transition(
            &db,
            &reservation.session_id,
            worker_lifecycle::WorkerLifecycleState::Failed,
            Some("objective_delivery_failed"),
        )
        .unwrap();
        session_supervisor::SessionSupervisor::transition(
            &db,
            &reservation.session_id,
            worker_lifecycle::WorkerLifecycleState::Completed,
            Some("terminal_failure_reported"),
        )
        .unwrap();
        let result = delegation::WorkerResult {
            schema_version: delegation::SCHEMA_VERSION,
            status: delegation::WorkerResultStatus::Failed,
            summary: "Objective delivery failed".into(),
            files_changed: vec![],
            tests: vec![],
            decisions: vec![],
            risks: vec!["Worker received no objective".into()],
            remaining_work: vec!["Retry the delegation".into()],
            suggested_next_action: delegation::SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        session_supervisor::SessionSupervisor::record_result(&db, &reservation.session_id, &result)
            .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT lease_status FROM worker_leases WHERE session_id=?1",
                params![reservation.session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "released"
        );
    }

    fn policy_fixture() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        let workspace_path = Path::new(env!("CARGO_MANIFEST_DIR"));
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![workspace_path.to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task',?1,'idle','now')", params![workspace_path.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth) VALUES('parent','w','codex','Parent','working','reported',0)", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('parent','fresh','now')", []).unwrap();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Write scope: src/**"}),
            )
            .unwrap();
        db
    }

    fn archive_fixture() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/archive-demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/archive-workspace','stopped','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Codex','stopped','reported')", []).unwrap();
        db.execute("INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at) VALUES('e1','s',NULL,1,'user.message','{\"text\":\"one\"}','now'),('e2','s','e1',2,'assistant.message','{\"text\":\"two\"}','now')", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,latest_checkpoint_entry_id,updated_at) VALUES('s','e2','fresh','e1','now')", []).unwrap();
        db.execute("INSERT INTO task_knowledge(id,workspace_id,session_id,kind,body,source_entry_id,created_at) VALUES('k','w','s','decision','Keep history','e1','now')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,write_mode,lease_status,created_at,updated_at) VALUES('s','w','implementation','standard','shared','expired','now','now')", []).unwrap();
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,turn_id,capability_units,source,created_at) VALUES('w','s','turn',3,'test','now')", []).unwrap();
        db
    }

    fn count(db: &Connection, table: &str) -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn session_status_round_trip() {
        for (value, expected) in [
            ("starting", SessionStatus::Starting),
            ("working", SessionStatus::Working),
            ("waiting", SessionStatus::Waiting),
            ("warm", SessionStatus::Warm),
            ("checkpointing", SessionStatus::Checkpointing),
            ("stopped", SessionStatus::Stopped),
            ("resuming", SessionStatus::Resuming),
            ("restored", SessionStatus::Restored),
            ("failed", SessionStatus::Failed),
            ("completed", SessionStatus::Completed),
            ("cancelled", SessionStatus::Cancelled),
        ] {
            assert_eq!(store::status(value), expected);
        }
    }

    #[test]
    fn local_history_snapshot_schedule_is_periodic() {
        assert_eq!(HISTORY_SNAPSHOT_INTERVAL, Duration::from_secs(15 * 60));
    }

    #[test]
    fn pressure_compaction_starts_only_at_seventy_five_percent_with_new_work() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"meaningful work"}),
            )
            .unwrap();
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,context_percent,capability_units,source,created_at) VALUES('w','parent',74,0,'test','now')", []).unwrap();
        assert!(begin_pressure_compaction(&db, "parent").unwrap().is_none());
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,context_percent,capability_units,source,created_at) VALUES('w','parent',75,0,'test','later')", []).unwrap();
        assert!(begin_pressure_compaction(&db, "parent").unwrap().is_some());
        assert_eq!(
            compaction_controller::CompactionController::pending(&db, "parent")
                .unwrap()
                .unwrap()
                .reason,
            compaction_controller::CompactionReason::ContextPressure
        );
    }

    #[test]
    fn archive_workspace_records_cleans_every_dependent_table() {
        let db = archive_fixture();
        workspaces::archive_workspace_records(&db, "w", || Ok(())).unwrap();
        for table in [
            "task_knowledge",
            "worker_leases",
            "session_heads",
            "session_entries",
            "usage_ledger",
            "sessions",
            "workspaces",
        ] {
            assert_eq!(count(&db, table), 0, "{table} retained archive rows");
        }
        assert_eq!(count(&db, "projects"), 1);
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn sqlite_snapshot_replays_forest_and_rewind_changes_only_active_head() {
        let db = archive_fixture();
        let before_entries = store::session_entries(&db, "s").unwrap();
        let before_workspace_path: String = db
            .query_row("SELECT path FROM workspaces WHERE id='w'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let initial = sessions::session_forest_snapshot(&db, "s").unwrap();
        assert_eq!(initial.repository_divergence.status, "unknown");
        assert_eq!(initial.head.unwrap().active_entry_id.as_deref(), Some("e2"));
        assert_eq!(initial.entries.len(), 2);
        assert_eq!(
            initial
                .leaves
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["e2"]
        );
        assert_eq!(initial.worker_leases.len(), 1);
        assert_eq!(initial.usage.len(), 1);

        let rewound = sessions::activate_session_entry_records(&db, "s", "e1").unwrap();
        assert_eq!(rewound.head.unwrap().active_entry_id.as_deref(), Some("e1"));
        assert_eq!(store::session_entries(&db, "s").unwrap(), before_entries);
        assert_eq!(
            db.query_row("SELECT path FROM workspaces WHERE id='w'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            before_workspace_path
        );
        assert!(rewound.reasons.iter().any(|event| {
            event.kind == "session.head_moved" && event.body.contains("files were not changed")
        }));
        assert_eq!(rewound.repository_divergence.status, "unknown");
    }

    #[test]
    fn repository_stamps_detect_clean_dirty_and_conversation_rewind_divergence() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path();
        let git = |arguments: &[&str]| {
            let output = std::process::Command::new("git")
                .args(arguments)
                .current_dir(repository)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "bridge@example.invalid"]);
        git(&["config", "user.name", "Bridge Test"]);
        std::fs::write(repository.join("tracked.txt"), "first\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "--quiet", "-m", "initial"]);

        let db = store::open(Path::new(":memory:")).unwrap();
        let path = repository.to_string_lossy();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![path.as_ref()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task',?1,'idle','now')",
            params![path.as_ref()],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Codex','working','reported')", []).unwrap();

        let clean = session_forest::SessionForest::new(&db)
            .append(
                "s",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"clean"}),
            )
            .unwrap();
        assert_eq!(clean.payload["_bridgeRepoState"]["status"], "clean");
        assert_eq!(
            sessions::session_forest_snapshot(&db, "s")
                .unwrap()
                .repository_divergence
                .status,
            "aligned"
        );

        std::fs::write(repository.join("tracked.txt"), "changed\n").unwrap();
        let dirty = session_forest::SessionForest::new(&db)
            .append(
                "s",
                session_forest::EntryKind::AssistantMessage,
                serde_json::json!({"text":"dirty"}),
            )
            .unwrap();
        assert_eq!(dirty.payload["_bridgeRepoState"]["status"], "dirty");
        assert_ne!(
            clean.payload["_bridgeRepoState"],
            dirty.payload["_bridgeRepoState"]
        );
        assert_eq!(
            sessions::session_forest_snapshot(&db, "s")
                .unwrap()
                .repository_divergence
                .status,
            "aligned"
        );

        let rewound = sessions::activate_session_entry_records(&db, "s", &clean.id).unwrap();
        assert_eq!(rewound.repository_divergence.status, "diverged");
        assert_eq!(
            std::fs::read_to_string(repository.join("tracked.txt")).unwrap(),
            "changed\n"
        );
    }

    #[test]
    fn archive_workspace_records_rolls_back_when_worktree_removal_fails() {
        let db = archive_fixture();
        let result = workspaces::archive_workspace_records(&db, "w", || {
            Err(BridgeError::Git("injected removal failure".into()))
        });
        assert!(matches!(result, Err(BridgeError::Git(_))));
        for table in [
            "task_knowledge",
            "worker_leases",
            "session_heads",
            "session_entries",
            "usage_ledger",
            "sessions",
            "workspaces",
        ] {
            assert!(count(&db, table) > 0, "{table} was not rolled back");
        }
    }

    #[test]
    fn repair_and_fallback_store_audit_events() {
        let db = store::open(Path::new(":memory:")).unwrap();
        let mut tracker = delegation::ResultRepairTracker::default();
        let first = process_worker_result_output(
            &db,
            &mut tracker,
            "worker",
            "invalid first output",
            |prompt| {
                assert!(prompt.contains("one repair turn"));
                true
            },
        )
        .unwrap();
        assert_eq!(first, None);
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "worker.result.repair_requested"
        );

        let fallback = process_worker_result_output(
            &db,
            &mut tracker,
            "worker",
            "invalid repair output",
            |_| panic!("a second repair must not be sent"),
        )
        .unwrap()
        .unwrap();
        assert!(fallback.summary.contains("Unstructured worker result"));
        assert!(!fallback.summary.contains("invalid first output"));
        assert!(!fallback.summary.contains("invalid repair output"));
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "worker.result.unstructured"
        );
    }

    #[test]
    fn policy_reservation_precedes_spawn_and_queues_overlapping_writer() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        let first = reserve_worker_launch(&db, "parent", "turn-1", &request, "gpt-5.6-terra", true)
            .unwrap()
            .expect("first writer should reserve");
        assert!(matches!(
            first.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM worker_leases WHERE workspace_id='w' AND lease_status='active'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT turn_id FROM usage_ledger WHERE session_id=?1",
                params![first.session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "turn-1"
        );
        assert_eq!(
            db.query_row(
                "SELECT requested_tier || ':' || model FROM sessions WHERE id=?1",
                params![first.session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "standard:gpt-5.6-terra"
        );

        let second = reserve_worker_launch_outcome(
            &db,
            "parent",
            "turn-1",
            &request,
            "gpt-5.6-terra",
            true,
            None,
        )
        .unwrap();
        assert!(matches!(second, WorkerReservationOutcome::Queued));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sessions WHERE workspace_id='w'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            2
        );
        let entries = store::session_entries(&db, "parent").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2].kind, "delegation.requested");
        assert_eq!(entries[2].payload["decision"], "queue");
        assert_eq!(entries[2].payload["reason"], "writer_conflict");
    }

    #[test]
    fn policy_defers_cross_harness_reservation_until_phase_boundary() {
        let db = policy_fixture();
        db.execute(
            "UPDATE sessions SET active_turn_id='turn-cross' WHERE id='parent'",
            [],
        )
        .unwrap();
        let mut request = policy_request(&["src/auth/**"]);
        request.harness = Some("claude".into());
        let outcome = reserve_worker_launch_outcome(
            &db,
            "parent",
            "turn-cross",
            &request,
            "fable",
            true,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, WorkerReservationOutcome::Queued));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sessions WHERE parent_session_id='parent'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT queue_status FROM worker_queue", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "queued"
        );
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "handoff.deferred_for_phase_boundary"
        );
    }

    #[test]
    fn policy_approval_blocks_launch_side_effects_until_accepted() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Please implement the auth change"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        assert!(reserve_worker_launch(
            &db,
            "parent",
            "turn-approval",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap()
        .is_none());
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='parent'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "waiting"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM usage_ledger WHERE source LIKE 'policy.spawn.%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        let approval = store::session_entries(&db, "parent")
            .unwrap()
            .into_iter()
            .last()
            .unwrap();
        assert_eq!(approval.kind, "approval.requested");
        let (turn_id, approved_request) = resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='parent'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "working"
        );
        assert_eq!(turn_id, "turn-approval");
        assert!(reserve_worker_launch(
            &db,
            "parent",
            &turn_id,
            &approved_request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap()
        .is_some());
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .is_err());
    }

    #[test]
    fn declined_policy_approval_never_launches() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Explain the auth module only"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        reserve_worker_launch(
            &db,
            "parent",
            "turn-decline",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        let approval = store::session_entries(&db, "parent")
            .unwrap()
            .into_iter()
            .last()
            .unwrap();
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "decline",
            &approval.payload,
        )
        .unwrap()
        .is_none());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        reserve_worker_launch(
            &db,
            "parent",
            "turn-decline",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        assert_eq!(
            store::session_entries(&db, "parent")
                .unwrap()
                .iter()
                .filter(|entry| entry.kind == "approval.requested")
                .count(),
            1,
            "a declined turn/scope must not create a dead follow-up card"
        );
    }

    #[test]
    fn policy_approval_is_idempotent_and_stale_branches_cannot_resolve() {
        let db = policy_fixture();
        let forest = session_forest::SessionForest::new(&db);
        let branch_point = forest
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Explain the auth module only"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        reserve_worker_launch(&db, "parent", "turn-stale", &request, "gpt-5.6-terra", true)
            .unwrap();
        reserve_worker_launch(&db, "parent", "turn-stale", &request, "gpt-5.6-terra", true)
            .unwrap();
        let entries = store::session_entries(&db, "parent").unwrap();
        let approvals = entries
            .iter()
            .filter(|entry| entry.kind == "approval.requested")
            .collect::<Vec<_>>();
        assert_eq!(approvals.len(), 1);
        let approval = approvals[0];
        forest.move_head("parent", Some(&branch_point.id)).unwrap();
        forest
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Do not make any changes"}),
            )
            .unwrap();
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .is_err());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn approved_launch_failure_is_durable_and_retryable_for_the_turn() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        live_turn::record_approved_launch_failure(&db, "parent", "turn-retry", &request).unwrap();
        let entries = session_forest::SessionForest::new(&db)
            .active_branch("parent")
            .unwrap();
        let failure = entries.last().unwrap();
        assert_eq!(failure.kind, "delegation.rejected");
        assert_eq!(failure.payload["reason"], "approved_launch_failed");
        assert_eq!(failure.payload["turnId"], "turn-retry");
        assert!(failure.payload["text"]
            .as_str()
            .unwrap()
            .contains("retried"));
    }

    #[test]
    fn unknown_model_hint_falls_back_and_records_warning_event() {
        let db = policy_fixture();
        let registry = adapters::AdapterRegistry::built_in().unwrap();
        let resolution = registry
            .resolve_model(
                "codex",
                CapabilityTier::Standard,
                Some("not-an-advertised-model"),
            )
            .unwrap();
        assert_eq!(resolution.actual_model, "gpt-5.6-terra");
        record_model_resolution_warning(&db, "parent", &resolution).unwrap();
        let (kind, body): (String, String) = db
            .query_row(
                "SELECT kind,body FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "capability.model_fallback");
        assert!(body.contains("not-an-advertised-model"));
        assert!(body.contains("gpt-5.6-terra"));
    }

    #[test]
    fn policy_budget_is_scoped_to_parent_turn() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        let outcome = policy::PolicyEngine::default().decide(&policy::PolicyInput {
            workspace_id: "w".into(),
            worktree_id: "w".into(),
            parent_session_id: "parent".into(),
            turn_id: "turn-1".into(),
            parent_depth: 0,
            request: request.clone(),
            owned_path_provenance: policy::OwnedPathProvenance {
                trusted_paths: request.owned_paths.clone(),
                source_entry_ids: vec!["test-user-entry".into()],
            },
            requested_harness: "codex".into(),
            task_family: "implementation".into(),
            active_workers: Vec::new(),
            warm_workers: Vec::new(),
            budget: policy::RequestBudget::default(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available: false,
        });
        for index in 0..3 {
            policy::record_spawn_usage(
                &db,
                "w",
                "parent",
                "turn-1",
                &outcome,
                delegation::CapabilityTier::Standard,
            )
            .unwrap();
            assert!(index < 3);
        }
        assert!(
            reserve_worker_launch(&db, "parent", "turn-1", &request, "gpt-5.6-terra", true)
                .unwrap()
                .is_none()
        );
        let next_turn =
            reserve_worker_launch(&db, "parent", "turn-2", &request, "gpt-5.6-terra", true)
                .unwrap()
                .expect("new turn should reset request counters");
        assert!(matches!(
            next_turn.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
    }
}

mod adapters;
mod agent;
mod agent_config;
mod binary;
mod browser_bridge;
mod claude_adapter;
mod codex_adapter;
mod compaction_controller;
pub mod completion;
mod context;
mod credential_broker;
mod delegation;
mod git;
mod handoff;
pub mod learning_job;
pub mod learning_router;
mod marketplace;
mod model;
pub mod model_profiles;
mod opencode_adapter;
mod orchestrator;
mod policy;
mod policy_coordinator;
pub mod policy_replay;
mod restoration;
pub mod router_replay;
pub mod routing_policy;
mod secret_interception;
mod session_forest;
mod session_supervisor;
mod skill_marketplace;
mod slash;
mod store;
mod worker_guard;
mod worker_lifecycle;
mod worker_pool;
mod worker_sandbox;
mod worktree_coordinator;

use chrono::Utc;
use model::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("{0}")]
    Invalid(String),
    #[error("Git: {0}")]
    Git(String),
    #[error("Database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Adapter: {0}")]
    Adapter(String),
    #[error("PTY: {0}")]
    Pty(String),
}
impl Serialize for BridgeError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct RuntimeSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}
struct AppState {
    db: Mutex<Connection>,
    telemetry_db: Mutex<Connection>,
    runtimes: Mutex<HashMap<String, RuntimeSession>>,
    adapters: Mutex<HashMap<String, Box<dyn adapters::AdapterRuntime>>>,
    adapter_registry: Arc<adapters::AdapterRegistry>,
    delegations: Mutex<DelegationState>,
    worktrees: PathBuf,
    database_path: PathBuf,
    telemetry_database_path: PathBuf,
    snapshot_dir: PathBuf,
    skill_store: PathBuf,
    skill_consents: Arc<Mutex<HashMap<String, skill_marketplace::SkillConsent>>>,
    credential_broker: Arc<credential_broker::CredentialBroker>,
    browser_bridge: Arc<browser_bridge::BrowserBridgeSupervisor>,
}

/// Bookkeeping for the multi-agent delegation tree.
#[derive(Default)]
struct DelegationState {
    /// Tracks the single same-session repair allowed for malformed worker output.
    result_repairs: delegation::ResultRepairTracker,
    /// Last observed provider turn per session, retained until the next turn
    /// so late usage events keep the originating user-request budget key.
    last_turn_by_session: HashMap<String, String>,
    /// Read-only worker session → tracked Git state captured before process start.
    read_only_baselines: HashMap<String, worker_guard::ReadOnlyBaseline>,
    /// OS-level boundary and output directory retained until the worker exits.
    read_only_sandboxes: HashMap<String, worker_sandbox::ReadOnlySandbox>,
}

#[derive(Serialize)]
struct Health {
    ok: bool,
    version: &'static str,
    harnesses: HashMap<&'static str, bool>,
    database: String,
    telemetry_database: String,
    snapshot_directory: String,
    adapters: Vec<AdapterDescriptor>,
}

#[tauri::command]
async fn health(state: State<'_, AppState>) -> Result<Health, BridgeError> {
    let adapters = state.adapter_registry.descriptors();
    let opencode_available = adapters
        .iter()
        .find(|adapter| adapter.id == "opencode")
        .is_some_and(|adapter| adapter.available);
    Ok(Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        harnesses: HashMap::from([
            ("claude", binary::resolve("claude").is_some()),
            ("codex", binary::resolve("codex").is_some()),
            ("opencode", opencode_available),
            ("shell", true),
        ]),
        database: state.database_path.to_string_lossy().into(),
        telemetry_database: state.telemetry_database_path.to_string_lossy().into(),
        snapshot_directory: state.snapshot_dir.to_string_lossy().into(),
        adapters,
    })
}

#[tauri::command]
async fn browser_bridge_state(
    state: State<'_, AppState>,
) -> Result<browser_bridge::BrowserBridgeSnapshot, BridgeError> {
    Ok(state.browser_bridge.snapshot())
}

#[tauri::command]
async fn install_browser_native_host(state: State<'_, AppState>) -> Result<String, BridgeError> {
    let supervisor = Arc::clone(&state.browser_bridge);
    tauri::async_runtime::spawn_blocking(move || {
        let executable = std::env::var_os("BRIDGE_BROWSER_HOST")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok().and_then(|path| path.parent().and_then(find_browser_host)))
            .ok_or_else(|| BridgeError::Invalid("Could not locate bridge-browser-host".into()))?;
        if !executable.exists() {
            return Err(BridgeError::Invalid(format!("Native host executable is missing at {}. Build the bridge-browser-host binary first.", executable.display())));
        }
        supervisor.install_native_host(&executable).map(|path| path.to_string_lossy().into_owned())
    }).await.map_err(|error| BridgeError::Invalid(format!("Native host registration task failed: {error}")))?
}

fn find_browser_host(directory: &Path) -> Option<PathBuf> {
    let direct = directory.join("bridge-browser-host");
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bridge-browser-host-"))
        })
}

#[tauri::command]
async fn browser_action(
    request: browser_bridge::BrowserActionRequest,
    state: State<'_, AppState>,
) -> Result<String, BridgeError> {
    state.browser_bridge.issue(request)
}

#[tauri::command]
async fn set_browser_permission(
    permission: String,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    state.browser_bridge.set_permission(&permission)
}

#[tauri::command]
async fn resolve_browser_approval(
    approval_id: String,
    allow: bool,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    state.browser_bridge.resolve_approval(&approval_id, allow)
}

#[tauri::command]
async fn takeover_browser(state: State<'_, AppState>) -> Result<(), BridgeError> {
    state.browser_bridge.takeover()
}

#[tauri::command]
async fn detach_browser(state: State<'_, AppState>) -> Result<String, BridgeError> {
    state.browser_bridge.detach()
}

#[tauri::command]
async fn route_browser(
    request: browser_bridge::BrowserRouteRequest,
) -> browser_bridge::BrowserRouteDecision {
    browser_bridge::route_browser(request)
}

#[tauri::command]
async fn browser_skills() -> Vec<browser_bridge::BrowserSkill> {
    browser_bridge::bundled_skills()
}

#[tauri::command]
async fn configure_remote_browser(
    config: Option<browser_bridge::RemoteBrowserConfig>,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    let supervisor = Arc::clone(&state.browser_bridge);
    tauri::async_runtime::spawn_blocking(move || supervisor.configure_remote(config))
        .await
        .map_err(|error| {
            BridgeError::Invalid(format!("Remote browser configuration task failed: {error}"))
        })?
}

#[tauri::command]
async fn start_remote_browser(
    initial_url: String,
    state: State<'_, AppState>,
) -> Result<Value, BridgeError> {
    let supervisor = Arc::clone(&state.browser_bridge);
    tauri::async_runtime::spawn_blocking(move || supervisor.start_remote_session(&initial_url))
        .await
        .map_err(|error| BridgeError::Invalid(format!("Remote browser task failed: {error}")))?
}

#[tauri::command]
async fn marketplace_catalog() -> marketplace::MarketplaceCatalog {
    marketplace::catalog()
}

#[tauri::command]
async fn marketplace_app_auth_states(
) -> Result<Vec<marketplace::MarketplaceAppAuthState>, BridgeError> {
    marketplace::app_auth_states()
}

fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

async fn live_available_capabilities(state: &AppState) -> std::collections::HashSet<String> {
    let mut capabilities = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .flat_map(|descriptor| descriptor.capabilities)
        .collect::<std::collections::HashSet<_>>();
    let home = user_home();
    let store = state.skill_store.clone();
    if let Ok(Ok(skills)) = tauri::async_runtime::spawn_blocking(move || {
        skill_marketplace::available_capabilities(&home, &store)
    })
    .await
    {
        capabilities.extend(skills);
    }
    capabilities
}

#[tauri::command]
async fn skill_catalog(
    state: State<'_, AppState>,
) -> Result<skill_marketplace::SkillCatalog, BridgeError> {
    let home = user_home();
    let store = state.skill_store.clone();
    tauri::async_runtime::spawn_blocking(move || skill_marketplace::catalog(&home, &store))
        .await
        .map_err(|error| BridgeError::Invalid(format!("Skill discovery task failed: {error}")))?
}

#[tauri::command]
async fn skill_suggestions(
    query: String,
    provider: skill_marketplace::SkillProvider,
    state: State<'_, AppState>,
) -> Result<Vec<skill_marketplace::CapabilitySuggestion>, BridgeError> {
    let home = user_home();
    let store = state.skill_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        skill_marketplace::suggestions(&query, provider, &home, &store)
    })
    .await
    .map_err(|error| BridgeError::Invalid(format!("Skill suggestion task failed: {error}")))?
}

#[tauri::command]
async fn preview_skill_change(
    skill_id: String,
    action: skill_marketplace::SkillAction,
    targets: Vec<skill_marketplace::SkillProvider>,
    state: State<'_, AppState>,
) -> Result<skill_marketplace::SkillPreview, BridgeError> {
    let home = user_home();
    let store = state.skill_store.clone();
    let consents = Arc::clone(&state.skill_consents);
    tauri::async_runtime::spawn_blocking(move || {
        skill_marketplace::preview(
            &skill_id,
            action,
            &targets,
            &home,
            &store,
            consents.as_ref(),
        )
    })
    .await
    .map_err(|error| BridgeError::Invalid(format!("Skill preview task failed: {error}")))?
}

#[tauri::command]
async fn execute_skill_change(
    confirmation_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<skill_marketplace::SkillActionResult>, BridgeError> {
    let home = user_home();
    let store = state.skill_store.clone();
    let consents = Arc::clone(&state.skill_consents);
    let results = tauri::async_runtime::spawn_blocking(move || {
        skill_marketplace::execute(&confirmation_id, &home, &store, consents.as_ref())
    })
    .await
    .map_err(|error| BridgeError::Invalid(format!("Skill installer task failed: {error}")))??;
    let _ = app.emit("state-changed", ());
    Ok(results)
}

#[tauri::command]
async fn marketplace_action(
    provider: marketplace::MarketplaceProvider,
    plugin_id: String,
    marketplace: Option<String>,
    action: marketplace::MarketplaceAction,
) -> Result<marketplace::MarketplaceActionResult, BridgeError> {
    marketplace::execute_action(provider, &plugin_id, marketplace.as_deref(), action)
}
#[tauri::command]
async fn get_state(state: State<'_, AppState>) -> Result<BridgeState, BridgeError> {
    store::state(&state.db.lock().unwrap())
}

fn session_forest_snapshot(
    db: &Connection,
    session_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    let current_state = store::repository_state_for_session(db, session_id)?;
    session_forest_snapshot_with_repository_state(db, session_id, current_state)
}

fn session_forest_snapshot_with_repository_state(
    db: &Connection,
    session_id: &str,
    current_state: serde_json::Value,
) -> Result<SessionForestSnapshot, BridgeError> {
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let config = policy::PolicyConfig::default();
    let entries = store::session_entries(db, session_id)?;
    let head = store::session_head(db, session_id)?;
    let selected_state = head
        .as_ref()
        .and_then(|head| head.active_entry_id.as_deref())
        .and_then(|id| entries.iter().find(|entry| entry.id == id))
        .and_then(|entry| entry.payload.get("_bridgeRepoState"))
        .cloned();
    let comparable = |value: &serde_json::Value| {
        value.get("status").and_then(serde_json::Value::as_str) != Some("unavailable")
    };
    let divergence_status = match selected_state.as_ref() {
        Some(selected)
            if comparable(selected) && comparable(&current_state) && selected == &current_state =>
        {
            "aligned"
        }
        Some(selected) if comparable(selected) && comparable(&current_state) => "diverged",
        _ => "unknown",
    };
    Ok(SessionForestSnapshot {
        session_id: session_id.to_owned(),
        entries,
        head,
        leaves: session_forest::SessionForest::new(db)
            .branch_leaves(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?,
        worker_leases: store::worker_leases(db, &workspace_id)?,
        worker_runtimes: store::worker_runtimes(db, &workspace_id)?,
        worker_queue: store::worker_queue_requests(db, &workspace_id)?,
        usage: store::usage_ledger(db, &workspace_id, None)?,
        reasons: store::workspace_reason_events(db, &workspace_id)?,
        policy_limits: PolicyLimits {
            max_workers_per_turn: config.max_workers_per_turn as i64,
            max_strong_workers_per_turn: config.max_strong_workers_per_turn as i64,
            max_capability_units_per_turn: config.max_capability_units_per_turn,
        },
        repository_divergence: RepositoryDivergence {
            status: divergence_status.into(),
            selected_state,
            current_state,
        },
        completion: completion::latest_summary(db, session_id)?,
    })
}

#[tauri::command]
async fn get_session_forest(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<SessionForestSnapshot, BridgeError> {
    // Git may be slow on large repositories or during index contention. Never
    // run it on the macOS event loop or while holding the global SQLite lock.
    let repository_path = {
        let db = state.db.lock().unwrap();
        store::repository_path_for_session(&db, &session_id)?
    };
    let repository_state = match repository_path {
        Some(path) => {
            tauri::async_runtime::spawn_blocking(move || store::repository_state_for_path(&path))
                .await
                .map_err(|error| {
                    BridgeError::Invalid(format!("Repository refresh task failed: {error}"))
                })?
        }
        None => serde_json::json!({"status":"unavailable"}),
    };
    let db = state.db.lock().unwrap();
    session_forest_snapshot_with_repository_state(&db, &session_id, repository_state)
}

fn completion_repository_stamp(
    db: &Connection,
    session_id: &str,
) -> Result<completion::RepositoryStamp, BridgeError> {
    let state = store::repository_state_for_session(db, session_id)?;
    let head = state
        .get("head")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid("completion proof requires a Git repository HEAD".into())
        })?;
    let dirty = state
        .get("dirtyHash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(
                "completion proof requires a deterministic dirty-tree digest".into(),
            )
        })?;
    Ok(completion::RepositoryStamp {
        head: head.into(),
        dirty_digest: dirty.into(),
    })
}

fn completion_attempt_repository(
    db: &Connection,
    attempt_id: &str,
) -> Result<(String, completion::RepositoryStamp), BridgeError> {
    let (session_id, repository_path, stored_head, stored_dirty): (String, String, String, String) = db.query_row(
        "SELECT session_id,repository_path,repository_head,dirty_digest FROM eval_attempts WHERE id=?1",
        params![attempt_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let state = store::repository_state_for_path(std::path::Path::new(&repository_path));
    let head = state
        .get("head")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&stored_head);
    let dirty = state
        .get("dirtyHash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&stored_dirty);
    Ok((
        session_id,
        completion::RepositoryStamp {
            head: head.into(),
            dirty_digest: dirty.into(),
        },
    ))
}

#[tauri::command]
async fn create_completion_plan(
    session_id: String,
    acceptance_criteria: Vec<String>,
    changed_paths: Vec<String>,
    repository_commands: Vec<String>,
    markdown_projection: Option<String>,
    markdown_committed: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<completion::CompletionSummary, BridgeError> {
    let (workspace_id, implementer_family): (String, Option<String>) = {
        let db = state.db.lock().unwrap();
        let workspace_id = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let implementer_family = db.query_row(
            "SELECT s.harness FROM worker_runtime r JOIN worker_leases l ON l.session_id=r.session_id JOIN sessions s ON s.id=r.session_id WHERE r.parent_session_id=?1 AND l.role='implementation' ORDER BY r.updated_at DESC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        ).optional()?;
        (workspace_id, implementer_family)
    };
    let contract = completion::CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id,
        session_id: session_id.clone(),
        schema_version: completion::COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: acceptance_criteria.clone(),
        markdown_projection,
        markdown_committed,
    };
    let available_capabilities = live_available_capabilities(&state).await;
    let change_labels = completion::labels_for_paths(&changed_paths);
    let db = state.db.lock().unwrap();
    let plan = completion::plan_with_registered_manifests(
        &db,
        completion::PlanInput {
            contract_id: contract.id.clone(),
            acceptance_criteria,
            changed_paths,
            repository_commands,
        },
        &change_labels,
        &available_capabilities,
    )?;
    let repository_path: String = db.query_row("SELECT COALESCE(s.cwd,w.path) FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1", params![session_id], |row| row.get(0))?;
    let repository = completion_repository_stamp(&db, &session_id)?;
    completion::create_flow(
        &db,
        &contract,
        &plan,
        &session_id,
        &repository_path,
        &repository,
        implementer_family.as_deref(),
    )?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion plan was not persisted".into()))?;
    let _ = app.emit("state-changed", ());
    Ok(summary)
}

#[tauri::command]
async fn record_completion_check(
    attempt_id: String,
    run: completion::CheckRun,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = state.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, &attempt_id)?;
    completion::record_check(&db, &attempt_id, &run)?;
    completion::finalize(&db, &attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    let _ = app.emit("state-changed", ());
    Ok(summary)
}

#[tauri::command]
async fn waive_completion(
    attempt_id: String,
    check_ids: Vec<String>,
    reason: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = state.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, &attempt_id)?;
    completion::waive(
        &db,
        &attempt_id,
        &check_ids,
        &reason,
        "local_user",
        &repository,
    )?;
    completion::finalize(&db, &attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    let _ = app.emit("state-changed", ());
    Ok(summary)
}

#[tauri::command]
async fn register_verifier_manifest(
    source: String,
    manifest: completion::VerifierManifest,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    completion::register_verifier_manifest(&state.db.lock().unwrap(), &source, &manifest)
}

#[tauri::command]
async fn verifier_candidates(
    change_labels: Vec<String>,
    available_capabilities: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<completion::VerifierCandidate>, BridgeError> {
    completion::verifier_candidates(
        &state.db.lock().unwrap(),
        &change_labels,
        &available_capabilities.into_iter().collect(),
    )
}

#[tauri::command]
async fn get_router_preferences(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    learning_router::load_preferences(&state.db.lock().unwrap(), &workspace_id)
}

#[tauri::command]
async fn update_router_preferences(
    workspace_id: String,
    preferences: learning_router::RouterPreferences,
    state: State<'_, AppState>,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    let db = state.db.lock().unwrap();
    learning_router::save_preferences(&db, &workspace_id, &preferences)?;
    learning_router::load_preferences(&db, &workspace_id)
}

#[tauri::command]
async fn get_model_setup(
    state: State<'_, AppState>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::setup_state(&state.db.lock().unwrap())
}

#[tauri::command]
async fn recommended_model_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<model_profiles::ModelProfileDraft>, BridgeError> {
    model_profiles::recommended_profiles(&state.adapter_registry.descriptors())
}

#[tauri::command]
async fn save_model_profiles(
    profiles: Vec<model_profiles::ModelProfileDraft>,
    state: State<'_, AppState>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::save_profiles(
        &state.db.lock().unwrap(),
        &state.adapter_registry.descriptors(),
        &profiles,
    )
}

#[tauri::command]
async fn reset_model_profiles(
    state: State<'_, AppState>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::reset_profiles(
        &state.db.lock().unwrap(),
        &state.adapter_registry.descriptors(),
    )
}

#[tauri::command]
async fn get_config_state(
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::state(&state.db.lock().unwrap())
}

#[tauri::command]
async fn save_harness_config(
    config: agent_config::HarnessConfig,
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let opencode_settings = (config.id == "opencode")
        .then(|| agent_config::opencode_settings(Some(&config)))
        .transpose()?;
    let next = agent_config::save_harness(&state.db.lock().unwrap(), config)?;
    if let Some(settings) = opencode_settings {
        let registry = state.adapter_registry.clone();
        let directory = opencode_directory(None)?;
        let _ = tauri::async_runtime::spawn_blocking(move || {
            registry.refresh_opencode(settings, &directory)
        })
        .await;
    }
    Ok(next)
}

#[tauri::command]
async fn reset_harness_config(
    id: String,
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_harness(&state.db.lock().unwrap(), &id)?;
    if id == "opencode" {
        let registry = state.adapter_registry.clone();
        let directory = opencode_directory(None)?;
        let _ = tauri::async_runtime::spawn_blocking(move || {
            registry.refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory)
        })
        .await;
    }
    Ok(next)
}

fn opencode_directory(directory: Option<String>) -> Result<String, BridgeError> {
    let path = directory
        .map(|value| PathBuf::from(value.trim()))
        .filter(|path| !path.as_os_str().is_empty())
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    if !path.is_dir() {
        return Err(BridgeError::Invalid(format!(
            "OpenCode directory does not exist: {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn refresh_opencode_catalog(
    directory: Option<String>,
    state: State<'_, AppState>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    let registry = state.adapter_registry.clone();
    let settings = registry.opencode_settings()?;
    tauri::async_runtime::spawn_blocking(move || registry.refresh_opencode(settings, &directory))
        .await
        .map_err(|error| BridgeError::Adapter(format!("OpenCode discovery task failed: {error}")))?
}

#[tauri::command]
async fn set_opencode_provider_api_key(
    provider_id: String,
    api_key: String,
    directory: Option<String>,
    state: State<'_, AppState>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    let registry = state.adapter_registry.clone();
    tauri::async_runtime::spawn_blocking(move || {
        registry.set_opencode_provider_api_key(&directory, &provider_id, &api_key)
    })
    .await
    .map_err(|error| {
        BridgeError::Adapter(format!("OpenCode authentication task failed: {error}"))
    })?
}

#[tauri::command]
async fn remove_opencode_provider_auth(
    provider_id: String,
    directory: Option<String>,
    state: State<'_, AppState>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    let registry = state.adapter_registry.clone();
    tauri::async_runtime::spawn_blocking(move || {
        registry.remove_opencode_provider_auth(&directory, &provider_id)
    })
    .await
    .map_err(|error| {
        BridgeError::Adapter(format!("OpenCode authentication task failed: {error}"))
    })?
}

#[tauri::command]
async fn save_agent_config(
    agent: agent_config::AgentDefinition,
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::save_agent(&state.db.lock().unwrap(), agent)
}

#[tauri::command]
async fn delete_agent_config(
    id: String,
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::delete_agent(&state.db.lock().unwrap(), &id)
}

#[tauri::command]
async fn set_default_agent(
    id: String,
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::set_default(&state.db.lock().unwrap(), &id)
}

#[tauri::command]
async fn reset_all_config(
    state: State<'_, AppState>,
) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_all(&state.db.lock().unwrap())?;
    let registry = state.adapter_registry.clone();
    let directory = opencode_directory(None)?;
    let _ = tauri::async_runtime::spawn_blocking(move || {
        registry.refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory)
    })
    .await;
    Ok(next)
}

#[tauri::command]
async fn get_learning_state(
    state: State<'_, AppState>,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::learning_state(&state.db.lock().unwrap())
}

#[tauri::command]
async fn run_learning(
    trigger_kind: learning_job::LearningTriggerKind,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<learning_job::LearningRun, BridgeError> {
    if matches!(
        trigger_kind,
        learning_job::LearningTriggerKind::Codex
            | learning_job::LearningTriggerKind::Claude
            | learning_job::LearningTriggerKind::OpenCode
    ) {
        return Err(BridgeError::Invalid(
            "external learning triggers must use a registered narrow command".into(),
        ));
    }
    let database_path = state.database_path.clone();
    let run = tauri::async_runtime::spawn_blocking(move || {
        learning_job::run_local_database(&database_path, trigger_kind)
    })
    .await
    .map_err(|error| BridgeError::Invalid(format!("Learning task failed: {error}")))??;
    let _ = app.emit("learning-job-changed", &run);
    Ok(run)
}

#[tauri::command]
async fn cancel_learning_run(
    run_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::cancel_run(&state.db.lock().unwrap(), &run_id)?;
    let _ = app.emit("learning-job-changed", &run);
    Ok(run)
}

#[tauri::command]
async fn update_learning_schedule(
    schedule: learning_job::LearningSchedule,
    state: State<'_, AppState>,
) -> Result<learning_job::LearningSchedule, BridgeError> {
    learning_job::update_schedule(&state.db.lock().unwrap(), &schedule)
}

#[tauri::command]
async fn register_learning_trigger(
    kind: learning_job::LearningTriggerKind,
    registration_id: String,
    credential_ref: Option<String>,
    expires_at: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    learning_job::register_trigger_with_expiry(
        &state.db.lock().unwrap(),
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
    learning_job::trigger_instructions(kind, &database_path, &registration_id)
}

#[tauri::command]
async fn enable_learning_trigger(
    kind: learning_job::LearningTriggerKind,
    registration_id: String,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    learning_job::enable_trigger(&state.db.lock().unwrap(), kind, &registration_id)
}

#[tauri::command]
async fn approve_learning_run(
    run_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::approve_run(&state.db.lock().unwrap(), &run_id)?;
    let _ = app.emit("learning-job-changed", &run);
    Ok(run)
}

#[tauri::command]
async fn rollback_routing_policy(
    target_version: i64,
    explanation: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::rollback_policy(&state.db.lock().unwrap(), target_version, &explanation)?;
    let result = learning_job::learning_state(&state.db.lock().unwrap())?;
    let _ = app.emit("learning-job-changed", &result);
    Ok(result)
}

#[tauri::command]
async fn activate_session_entry(
    session_id: String,
    entry_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SessionForestSnapshot, BridgeError> {
    let db = state.db.lock().unwrap();
    let snapshot = activate_session_entry_records(&db, &session_id, &entry_id)?;
    let _ = app.emit("state-changed", ());
    Ok(snapshot)
}

fn activate_session_entry_records(
    db: &Connection,
    session_id: &str,
    entry_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    session_forest::SessionForest::new(db)
        .move_head(session_id, Some(entry_id))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "session-forest",
        "session.head_moved",
        session_id,
        &format!("Conversation head moved to {entry_id}; files were not changed"),
    )?;
    session_forest_snapshot(db, session_id)
}

#[tauri::command]
async fn add_project(path: String, state: State<'_, AppState>) -> Result<BridgeState, BridgeError> {
    let clean = git::validate_repo(Path::new(&path))?;
    let name = Path::new(&clean)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("Repository")
        .to_string();
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
        params![id, name, clean, Utc::now().to_rfc3339()],
    )?;
    store::event(
        &db,
        "project",
        "project.added",
        &id,
        &format!("Added {name}"),
    )?;
    store::state(&db)
}

/// Scratch working directory for a chat that has no connected folder/repo.
fn chat_scratch_dir(state: &AppState, session_id: &str) -> PathBuf {
    state
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chats")
        .join(session_id)
}

fn chat_label(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New chat")
        .to_string()
}

/// Create a repo-less workspace. A folder/git repo can be connected later.
#[tauri::command]
async fn create_workspace(
    title: String,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let name = title.trim();
    if name.is_empty() {
        return Err(BridgeError::Invalid("Workspace name is required".into()));
    }
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO workspaces(id,title,status,created_at) VALUES(?1,?2,'idle',?3)",
        params![id, name, Utc::now().to_rfc3339()],
    )?;
    store::event(
        &db,
        "supervisor",
        "workspace.created",
        &id,
        &format!("Created workspace {name}"),
    )?;
    store::state(&db)
}

/// Create a standalone direct chat (no workspace). Runs in a private scratch dir.
#[tauri::command]
async fn create_chat(
    harness: Harness,
    model: Option<String>,
    title: Option<String>,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let adapter_id = store::harness_name(&harness);
    let id = Uuid::new_v4().to_string();
    let cwd = chat_scratch_dir(state.inner(), &id);
    let label = chat_label(title.as_deref());
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth) VALUES(?1,NULL,?2,?3,'idle','estimated',?4,'direct',?5,?6,0)",
        params![id, adapter_id, label, model, title, cwd.to_string_lossy()],
    )?;
    store::event(
        &db,
        "chat",
        "chat.created",
        &id,
        &format!("Created chat {label}"),
    )?;
    store::state(&db)
}

/// Create an orchestrator session inside a workspace (the classic Bridge agent
/// that plans and delegates to workers). Multiple are allowed per workspace.
#[derive(Debug, Clone)]
struct OrchestratorSelection {
    adapter_id: String,
    model: String,
    tier: CapabilityTier,
    effort: Option<delegation::Effort>,
    label: String,
}

fn resolve_orchestrator_selection(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
) -> Result<OrchestratorSelection, BridgeError> {
    let descriptors = registry.descriptors();
    let configured_agent = agent_config::default_orchestrator(db);
    if let Some(agent) = configured_agent.as_ref() {
        if agent.enabled && matches!(agent.harness.as_str(), "codex" | "claude" | "opencode") {
            let harness_config = agent_config::harness_config(db, &agent.harness);
            let preferred_model = agent
                .model
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    harness_config
                        .as_ref()
                        .and_then(|config| config.default_model.as_deref())
                        .filter(|value| !value.trim().is_empty())
                });
            if harness_config.is_some() {
                if let Ok(resolution) = registry.resolve_model(
                    &agent.harness,
                    CapabilityTier::Standard,
                    preferred_model,
                ) {
                    return Ok(OrchestratorSelection {
                        adapter_id: agent.harness.clone(),
                        model: resolution.actual_model,
                        tier: CapabilityTier::Standard,
                        effort: harness_config
                            .and_then(|config| config.effort)
                            .or(Some(agent.effort)),
                        label: agent.name.clone(),
                    });
                }
            }
        }
    }
    if let Some(profile) = model_profiles::resolve_profile(
        db,
        &descriptors,
        model_profiles::ProfilePurpose::StandardOrchestrator,
    )?
    .filter(|profile| agent_config::is_harness_enabled(db, &profile.provider))
    {
        let resolution =
            registry.resolve_model(&profile.provider, profile.tier, Some(&profile.model))?;
        return Ok(OrchestratorSelection {
            adapter_id: profile.provider,
            model: resolution.actual_model,
            tier: profile.tier,
            effort: configured_agent
                .as_ref()
                .filter(|agent| !agent.is_built_in || !agent.updated_at.is_empty())
                .map(|agent| agent.effort)
                .or(Some(profile.effort)),
            label: configured_agent
                .as_ref()
                .map(|agent| agent.name.clone())
                .unwrap_or_else(|| orchestrator::SESSION_LABEL.into()),
        });
    }
    for descriptor in descriptors.iter().filter(|descriptor| {
        descriptor.available && agent_config::is_harness_enabled(db, &descriptor.id)
    }) {
        if let Ok(resolution) = registry.resolve_model(&descriptor.id, orchestrator::TIER, None) {
            return Ok(OrchestratorSelection {
                adapter_id: descriptor.id.clone(),
                model: resolution.actual_model,
                tier: orchestrator::TIER,
                effort: None,
                label: orchestrator::SESSION_LABEL.into(),
            });
        }
    }
    Err(BridgeError::Invalid(
        "no available adapter can resolve the Standard orchestrator profile".into(),
    ))
}

#[tauri::command]
async fn create_workspace_session(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    let selection = resolve_orchestrator_selection(&db, &state.adapter_registry)?;
    let ws_path: Option<String> = db
        .query_row(
            "SELECT path FROM workspaces WHERE id=?1",
            params![workspace_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    let cwd = ws_path.unwrap_or_else(|| {
        chat_scratch_dir(state.inner(), &id)
            .to_string_lossy()
            .to_string()
    });
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,requested_tier,effort,kind,cwd,depth) VALUES(?1,?2,?3,?4,'idle','estimated',?5,?6,?7,'orchestrator',?8,0)",
        params![id, workspace_id, selection.adapter_id, selection.label, selection.model, selection.tier.as_str(), selection.effort.map(|effort| effort.as_str()), cwd],
    )?;
    store::event(
        &db,
        "supervisor",
        "session.created",
        &id,
        "New agent session",
    )?;
    store::state(&db)
}

/// Change a direct chat's harness/model. Stops any running adapter so the next
/// message starts a fresh provider session with the new model.
#[tauri::command]
async fn update_chat_model(
    session_id: String,
    harness: Harness,
    model: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let adapter_id = store::harness_name(&harness);
    let stop_session_id = session_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&stop_session_id) {
            runtime.stop(adapters::ShutdownReason::Replaced);
        };
    })
    .await
    .map_err(|error| BridgeError::Adapter(format!("Adapter shutdown task failed: {error}")))?;
    session_supervisor::SessionSupervisor::clear_adapter_process(
        &state.db.lock().unwrap(),
        &session_id,
    )?;
    let db = state.db.lock().unwrap();
    db.execute(
        "UPDATE sessions SET harness=?2,model=?3,provider_session_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1 AND kind='direct'",
        params![session_id, adapter_id, model],
    )?;
    store::state(&db)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SlashCommandResolve {
    name: String,
    harness: String,
    kind: String,
    /// When true, the frontend should switch the direct chat to `harness` before sending.
    switch_harness: bool,
}

/// Enumerate slash commands + skills from every signed-in provider, so the UI
/// can offer a labeled `/` menu.
#[tauri::command]
async fn list_slash_commands(
    state: State<'_, AppState>,
) -> Result<Vec<slash::SlashCommand>, BridgeError> {
    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    Ok(slash::list_commands(&available))
}

/// Resolve a composer `/command` against the catalog so the UI can auto-switch
/// harness before sending.
#[tauri::command]
async fn resolve_slash_command(
    text: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<SlashCommandResolve>, BridgeError> {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Ok(None);
    };
    let name = rest
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BridgeError::Invalid("Empty slash command".into()))?;
    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    let (kind, session_harness): (String, String) = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT kind, harness FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };
    let catalog = slash::list_commands(&available);
    let matches: Vec<_> = catalog
        .iter()
        .filter(|command| command.name.eq_ignore_ascii_case(name))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    let chosen = matches
        .iter()
        .find(|command| command.harness == session_harness)
        .or_else(|| {
            // Prefer the command's own harness when the name is unique to one provider.
            if matches.len() == 1 {
                matches.first()
            } else {
                None
            }
        })
        .or_else(|| matches.first())
        .map(|command| (*command).clone())
        .expect("matches non-empty");
    let switch_harness = kind == "direct" && chosen.harness != session_harness;
    Ok(Some(SlashCommandResolve {
        name: chosen.name.clone(),
        harness: chosen.harness.clone(),
        kind: chosen.kind.clone(),
        switch_harness,
    }))
}

/// Attach a folder (optionally a git repo) to a workspace as its working directory.
#[tauri::command]
async fn connect_workspace_folder(
    workspace_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let folder = Path::new(&path);
    if !folder.is_dir() {
        return Err(BridgeError::Invalid("That folder no longer exists".into()));
    }
    let db = state.db.lock().unwrap();
    let (resolved_path, project_id, branch) = match git::validate_repo(folder) {
        Ok(root) => {
            let name = Path::new(&root)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Repository")
                .to_string();
            db.execute(
                "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
                params![
                    Uuid::new_v4().to_string(),
                    name,
                    root,
                    Utc::now().to_rfc3339()
                ],
            )?;
            let project_id: Option<String> = db
                .query_row(
                    "SELECT id FROM projects WHERE path=?1",
                    params![root],
                    |r| r.get(0),
                )
                .ok();
            (root.clone(), project_id, git::current_branch(folder))
        }
        Err(_) => (folder.to_string_lossy().to_string(), None, None),
    };
    db.execute(
        "UPDATE workspaces SET path=?2,project_id=?3,branch=?4 WHERE id=?1",
        params![workspace_id, resolved_path, project_id, branch],
    )?;
    store::event(
        &db,
        "supervisor",
        "workspace.connected",
        &workspace_id,
        &format!("Connected {resolved_path}"),
    )?;
    store::state(&db)
}

#[tauri::command]
async fn start_session(
    workspace_id: String,
    _harness: Option<Harness>,
    _model: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    // The persisted Standard orchestrator profile owns the default provider,
    // model, tier, and effort. Resolution still happens against live inventory.
    let selection = {
        let db = state.db.lock().unwrap();
        resolve_orchestrator_selection(&db, &state.adapter_registry)?
    };
    let adapter_id = selection.adapter_id.as_str();
    let session_label = selection.label.as_str();
    let chosen_model = Some(selection.model.clone());
    let chosen_effort = selection.effort;
    let chosen_effort_name = chosen_effort.map(|effort| effort.as_str());
    let db = state.db.lock().unwrap();
    let path: Option<String> = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get::<_, Option<String>>(0),
    )?;
    let existing: Option<(String, Option<String>)> = db.query_row(
        "SELECT id,provider_session_id FROM sessions WHERE workspace_id=?1 AND harness=?2 AND status IN ('idle','stopped','failed','ready','working','waiting') ORDER BY rowid DESC LIMIT 1",
        params![workspace_id, adapter_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).ok();
    let session_id = existing
        .as_ref()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let stored_provider_id = existing
        .as_ref()
        .and_then(|(_, provider_id)| provider_id.clone());
    let checkpoint_context = if existing.is_some() {
        restoration::checkpoint_context(&db, &session_id)?
    } else {
        None
    };
    drop(db);
    let path = path.filter(|value| !value.is_empty()).unwrap_or_else(|| {
        chat_scratch_dir(state.inner(), &session_id)
            .to_string_lossy()
            .to_string()
    });
    std::fs::create_dir_all(&path)?;
    let process_is_hot = state.adapters.lock().unwrap().contains_key(&session_id);
    if process_is_hot {
        let current_model: Option<String> = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT model FROM sessions WHERE id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if current_model.as_deref() == chosen_model.as_deref() {
            let db = state.db.lock().unwrap();
            restoration::set_head_state(
                &db,
                &session_id,
                RestorationMode::Hot,
                if stored_provider_id.is_some() {
                    ResumeEligibility::Native
                } else {
                    ResumeEligibility::CheckpointRestored
                },
                stored_provider_id.as_deref(),
            )?;
            handoff::record_fidelity(&db, &session_id, ContinuationFidelity::Native)?;
            return store::state(&db);
        }
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::Replaced);
        }
        record_shutdown_reason(
            &state.db.lock().unwrap(),
            &session_id,
            adapters::ShutdownReason::Replaced,
        )?;
    }

    // The orchestrator is depth 0. It gets the routing briefing plus the shared
    // delegation protocol so it can spawn workers itself.
    let configured_prompt =
        agent_config::orchestrator_prompt(&state.db.lock().unwrap(), adapter_id);
    let orchestrator_instructions = format!(
        "{}\n\n{}{}\n\n{}",
        orchestrator::briefing(),
        delegation::protocol(0),
        if configured_prompt.is_empty() {
            String::new()
        } else {
            format!("\n\n{configured_prompt}")
        },
        state.credential_broker.instructions(&session_id),
    );
    let plan = restoration::select_plan(
        false,
        stored_provider_id.as_deref(),
        state.adapter_registry.supports_native_resume(adapter_id),
        checkpoint_context.is_some(),
    );
    let start_fresh = |instructions: &str| {
        state.adapter_registry.start(
            adapter_id,
            adapters::StartRequest {
                cwd: &path,
                model: chosen_model.as_deref(),
                effort: chosen_effort_name,
                instructions: Some(instructions),
                write_mode: None,
                read_only_sandbox: None,
            },
        )
    };
    let checkpoint_instructions = checkpoint_context
        .as_ref()
        .map(|context| format!("{orchestrator_instructions}\n\n{context}"));
    let (mut started, restoration_mode, resume_eligibility) = match plan {
        restoration::RestorationPlan::Native => {
            let provider_id = stored_provider_id
                .as_deref()
                .expect("native plan has provider id");
            match state.adapter_registry.resume(
                adapter_id,
                adapters::ResumeRequest {
                    provider_session_id: provider_id,
                    cwd: &path,
                    model: chosen_model.as_deref(),
                    effort: chosen_effort_name,
                    instructions: Some(orchestrator_instructions.as_str()),
                    write_mode: None,
                    read_only_sandbox: None,
                },
            ) {
                Ok(started) => (started, RestorationMode::Native, ResumeEligibility::Native),
                Err(error) => {
                    let db = state.db.lock().unwrap();
                    restoration::record_resume_failed(&db, &session_id, &error.to_string())?;
                    drop(db);
                    match restoration::fallback_after_failure(
                        restoration::RestorationPlan::Native,
                        checkpoint_instructions.is_some(),
                    ) {
                        Some(restoration::RestorationPlan::CheckpointRestored) => {
                            match start_fresh(
                                checkpoint_instructions
                                    .as_deref()
                                    .expect("checkpoint fallback has stored context"),
                            ) {
                                Ok(started) => (
                                    started,
                                    RestorationMode::CheckpointRestored,
                                    ResumeEligibility::CheckpointRestored,
                                ),
                                Err(error) => {
                                    let db = state.db.lock().unwrap();
                                    restoration::record_checkpoint_restore_failed(
                                        &db,
                                        &session_id,
                                        &error.to_string(),
                                    )?;
                                    drop(db);
                                    (
                                        start_fresh(&orchestrator_instructions)?,
                                        RestorationMode::Fresh,
                                        ResumeEligibility::Fresh,
                                    )
                                }
                            }
                        }
                        Some(restoration::RestorationPlan::Fresh) => (
                            start_fresh(&orchestrator_instructions)?,
                            RestorationMode::Fresh,
                            ResumeEligibility::Fresh,
                        ),
                        _ => unreachable!("native failure has a deterministic fallback"),
                    }
                }
            }
        }
        restoration::RestorationPlan::CheckpointRestored => {
            match start_fresh(
                checkpoint_instructions
                    .as_deref()
                    .expect("checkpoint plan has stored context"),
            ) {
                Ok(started) => (
                    started,
                    RestorationMode::CheckpointRestored,
                    ResumeEligibility::CheckpointRestored,
                ),
                Err(error) => {
                    debug_assert_eq!(
                        restoration::fallback_after_failure(
                            restoration::RestorationPlan::CheckpointRestored,
                            true,
                        ),
                        Some(restoration::RestorationPlan::Fresh)
                    );
                    let db = state.db.lock().unwrap();
                    restoration::record_checkpoint_restore_failed(
                        &db,
                        &session_id,
                        &error.to_string(),
                    )?;
                    drop(db);
                    (
                        start_fresh(&orchestrator_instructions)?,
                        RestorationMode::Fresh,
                        ResumeEligibility::Fresh,
                    )
                }
            }
        }
        restoration::RestorationPlan::Fresh => (
            start_fresh(&orchestrator_instructions)?,
            RestorationMode::Fresh,
            ResumeEligibility::Fresh,
        ),
        restoration::RestorationPlan::Hot => unreachable!("hot sessions returned above"),
    };
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let process_id = started.runtime.process_id();
    let reader = started.reader;
    let started_at = Utc::now().to_rfc3339();
    let db = state.db.lock().unwrap();
    if existing.is_some() {
        db.execute(
            "UPDATE sessions SET harness=?2,status='working',started_at=?3,ended_at=NULL,provider_session_id=?4,active_turn_id=NULL,metric_source='reported',model=?5,requested_tier=?6,effort=?7,label=?8,depth=0,parent_session_id=NULL,trace_id=COALESCE(trace_id,lower(hex(randomblob(16)))) WHERE id=?1",
            params![
                session_id,
                adapter_id,
                started_at,
                thread_id,
                chosen_model,
                selection.tier.as_str(),
                chosen_effort_name,
                session_label
            ],
        )?;
    } else {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,provider_session_id,model,requested_tier,effort,depth,trace_id) VALUES(?1,?2,?3,?4,'working',?5,'reported',?6,?7,?8,?9,0,?10)",
            params![
                session_id,
                workspace_id,
                adapter_id,
                session_label,
                started_at,
                thread_id,
                chosen_model,
                selection.tier.as_str(),
                chosen_effort_name,
                Uuid::new_v4().simple().to_string()
            ],
        )?;
    }
    if let Err(error) =
        session_supervisor::SessionSupervisor::track_adapter_process(&db, &session_id, process_id)
    {
        drop(db);
        started.runtime.stop(adapters::ShutdownReason::Failed);
        return Err(error);
    }
    restoration::set_head_state(
        &db,
        &session_id,
        restoration_mode,
        resume_eligibility,
        Some(&thread_id),
    )?;
    let continuation_fidelity = match restoration_mode {
        RestorationMode::Hot | RestorationMode::Native => ContinuationFidelity::Native,
        RestorationMode::CheckpointRestored => ContinuationFidelity::ProjectedAtBoundary,
        RestorationMode::Fresh if existing.is_some() => ContinuationFidelity::ProjectedMidTurn,
        RestorationMode::Fresh => ContinuationFidelity::Native,
    };
    handoff::record_fidelity(&db, &session_id, continuation_fidelity)?;
    db.execute(
        "UPDATE workspaces SET status='working' WHERE id=?1",
        params![workspace_id],
    )?;
    store::event(
        &db,
        "adapter",
        "session.started",
        &session_id,
        &format!(
            "Started {session_label} on {} with {} restoration",
            chosen_model.as_deref().unwrap_or("default"),
            restoration_mode.as_str()
        ),
    )?;
    if adapter_id == orchestrator::HARNESS {
        let context = agent::NormalizedEvent {
            kind: "session.context".into(),
            item_id: Some("orchestrator-briefing".into()),
            role: Some("system".into()),
            status: Some("ready".into()),
            title: Some("Orchestrator routing policy".into()),
            text: Some(orchestrator::briefing()),
            data: serde_json::json!({
                "source": "capability-policy",
                "requestedTier": orchestrator::TIER,
                "runtimeModel": chosen_model
            }),
        };
        let _ = store::session_event(
            &db,
            &session_id,
            &context,
            &serde_json::json!({"adapter": adapter_id, "hidden": true}),
        );
    }
    for message in &started.startup_messages {
        persist_agent_value(
            &db,
            &state.adapter_registry,
            adapter_id,
            &session_id,
            message,
        )?;
    }
    drop(db);
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), started.runtime);

    spawn_reader_thread(
        app.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with the
/// routing briefing + delegation protocol (workers enabled via the reader gate).
#[tauri::command]
async fn start_chat(
    session_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let (harness, kind, model, cwd_col, workspace_id, provider_id, effort): (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT harness,kind,model,cwd,workspace_id,provider_session_id,effort FROM sessions WHERE id=?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )?
    };
    if state.adapters.lock().unwrap().contains_key(&session_id) {
        return store::state(&state.db.lock().unwrap());
    }
    let is_orchestrator = kind == "orchestrator";
    let cwd = match cwd_col.filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => {
            let workspace_path = workspace_id.as_ref().and_then(|workspace| {
                state
                    .db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT path FROM workspaces WHERE id=?1",
                        params![workspace],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten()
            });
            workspace_path.unwrap_or_else(|| {
                chat_scratch_dir(state.inner(), &session_id)
                    .to_string_lossy()
                    .to_string()
            })
        }
    };
    std::fs::create_dir_all(&cwd)?;
    let adapter_id: &str = harness.as_str();
    if !agent_config::is_harness_enabled(&state.db.lock().unwrap(), adapter_id) {
        return Err(BridgeError::Invalid(format!(
            "{adapter_id} is disabled in Settings"
        )));
    }
    let tier = if is_orchestrator {
        orchestrator::TIER
    } else {
        CapabilityTier::Fast
    };
    let configured_harness = agent_config::harness_config(&state.db.lock().unwrap(), adapter_id);
    let chosen_model = model
        .as_ref()
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| {
            configured_harness
                .as_ref()
                .and_then(|config| config.default_model.clone())
        })
        .or_else(|| {
            state
                .adapter_registry
                .resolve_model(adapter_id, tier, None)
                .ok()
                .map(|resolution| resolution.actual_model)
        });
    let proxy_instructions = state.credential_broker.instructions(&session_id);
    let configured_prompt = if is_orchestrator {
        agent_config::orchestrator_prompt(&state.db.lock().unwrap(), adapter_id)
    } else {
        agent_config::session_prompt(&state.db.lock().unwrap(), adapter_id)
    };
    let orchestrator_instructions = if is_orchestrator {
        format!(
            "{}\n\n{}{}\n\n{}",
            orchestrator::briefing(),
            delegation::protocol(0),
            if configured_prompt.is_empty() {
                String::new()
            } else {
                format!("\n\n{configured_prompt}")
            },
            proxy_instructions
        )
    } else {
        format!(
            "{}{}",
            if configured_prompt.is_empty() {
                String::new()
            } else {
                format!("{configured_prompt}\n\n")
            },
            proxy_instructions
        )
    };
    let configured_effort = configured_harness
        .and_then(|config| config.effort)
        .map(|value| value.as_str().to_owned());
    let chosen_effort = effort
        .filter(|value| !value.is_empty())
        .or(configured_effort);
    let resumable = provider_id
        .filter(|value| !value.is_empty())
        .filter(|_| state.adapter_registry.supports_native_resume(adapter_id));
    let registry = state.adapter_registry.clone();
    let launch_adapter_id = adapter_id.to_owned();
    let launch_cwd = cwd.clone();
    let launch_model = chosen_model.clone();
    let (mut started, mode, eligibility) =
        tauri::async_runtime::spawn_blocking(move || match resumable {
            Some(provider) => match registry.resume(
                &launch_adapter_id,
                adapters::ResumeRequest {
                    provider_session_id: &provider,
                    cwd: &launch_cwd,
                    model: launch_model.as_deref(),
                    effort: chosen_effort.as_deref(),
                    instructions: Some(&orchestrator_instructions),
                    write_mode: None,
                    read_only_sandbox: None,
                },
            ) {
                Ok(started) => Ok((started, RestorationMode::Native, ResumeEligibility::Native)),
                Err(_) => registry
                    .start(
                        &launch_adapter_id,
                        adapters::StartRequest {
                            cwd: &launch_cwd,
                            model: launch_model.as_deref(),
                            effort: chosen_effort.as_deref(),
                            instructions: Some(&orchestrator_instructions),
                            write_mode: None,
                            read_only_sandbox: None,
                        },
                    )
                    .map(|started| (started, RestorationMode::Fresh, ResumeEligibility::Fresh)),
            },
            None => registry
                .start(
                    &launch_adapter_id,
                    adapters::StartRequest {
                        cwd: &launch_cwd,
                        model: launch_model.as_deref(),
                        effort: chosen_effort.as_deref(),
                        instructions: Some(&orchestrator_instructions),
                        write_mode: None,
                        read_only_sandbox: None,
                    },
                )
                .map(|started| (started, RestorationMode::Fresh, ResumeEligibility::Fresh)),
        })
        .await
        .map_err(|error| BridgeError::Adapter(format!("Adapter startup task failed: {error}")))??;
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let process_id = started.runtime.process_id();
    let reader = started.reader;
    let started_at = Utc::now().to_rfc3339();
    {
        let db = state.db.lock().unwrap();
        if is_orchestrator {
            db.execute(
                "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,cwd=?5,harness=?6,requested_tier=?7,label=?8,depth=0 WHERE id=?1",
                params![session_id, started_at, thread_id, chosen_model, cwd, adapter_id, tier.as_str(), orchestrator::SESSION_LABEL],
            )?;
        } else {
            db.execute(
                "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,cwd=?5 WHERE id=?1",
                params![session_id, started_at, thread_id, chosen_model, cwd],
            )?;
        }
        if let Err(error) = session_supervisor::SessionSupervisor::track_adapter_process(
            &db,
            &session_id,
            process_id,
        ) {
            drop(db);
            started.runtime.stop(adapters::ShutdownReason::Failed);
            return Err(error);
        }
        restoration::set_head_state(&db, &session_id, mode, eligibility, Some(&thread_id))?;
        if let Some(workspace) = &workspace_id {
            let _ = db.execute(
                "UPDATE workspaces SET status='working' WHERE id=?1",
                params![workspace],
            );
        }
        store::event(
            &db,
            "adapter",
            "session.started",
            &session_id,
            &format!(
                "Started {} on {}",
                if is_orchestrator {
                    "orchestrator"
                } else {
                    "chat"
                },
                chosen_model.as_deref().unwrap_or("default")
            ),
        )?;
        if is_orchestrator {
            let context = agent::NormalizedEvent {
                kind: "session.context".into(),
                item_id: Some("orchestrator-briefing".into()),
                role: Some("system".into()),
                status: Some("ready".into()),
                title: Some("Orchestrator routing policy".into()),
                text: Some(orchestrator::briefing()),
                data: serde_json::json!({"source": "capability-policy", "requestedTier": tier, "runtimeModel": chosen_model}),
            };
            let _ = store::session_event(
                &db,
                &session_id,
                &context,
                &serde_json::json!({"adapter": adapter_id, "hidden": true}),
            );
        }
        for message in &started.startup_messages {
            persist_agent_value(
                &db,
                &state.adapter_registry,
                adapter_id,
                &session_id,
                message,
            )?;
        }
    }
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), started.runtime);
    spawn_reader_thread(
        app.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
}

/// Drive one structured session's stdout: normalize every frame, then on exit
/// mark the session stopped and unblock any parent that was waiting on it.
fn spawn_reader_thread(
    app: AppHandle,
    session_id: String,
    launch_started_at: String,
    current_turn: Arc<Mutex<Option<String>>>,
    mut reader: Box<dyn BufRead + Send>,
) {
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                        handle_agent_value(&app, &session_id, &current_turn, &value);
                    }
                }
            }
        }
        let state = app.state::<AppState>();
        let is_current_launch = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT started_at=?2 FROM sessions WHERE id=?1",
                params![session_id, launch_started_at],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if !is_current_launch {
            return;
        }
        state.adapters.lock().unwrap().remove(&session_id);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        notify_parent_on_worker_exit(&app, &session_id);
        let db = state.db.lock().unwrap();
        let is_worker = store::worker_runtime(&db, &session_id)
            .ok()
            .flatten()
            .is_some();
        let workspace: Option<String> = db
            .query_row(
                "SELECT workspace_id FROM sessions WHERE id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        if !is_worker {
            let _ = db.execute("UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1 AND status IN ('working','waiting')", params![session_id,Utc::now().to_rfc3339()]);
        }
        if let Some(workspace) = workspace {
            let _=db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'stopped' END WHERE id=?1",params![workspace]);
        }
        drop(db);
        let _ = app.emit("state-changed", ());
    });
}

fn persist_agent_value(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
    adapter_id: &str,
    session_id: &str,
    value: &serde_json::Value,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let normalized = registry.normalize(adapter_id, value);
    normalized
        .iter()
        .map(|event| {
            store::session_event(
                db,
                session_id,
                event,
                &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
            )
        })
        .collect()
}

fn agent_event_changes_bridge_state(event: &agent::NormalizedEvent) -> bool {
    matches!(
        event.kind.as_str(),
        "turn.started" | "turn.completed" | "approval.requested" | "usage.updated"
    ) || (event.kind == "error" && event.status.as_deref() == Some("failed"))
}

fn handle_agent_value(
    app: &AppHandle,
    session_id: &str,
    current_turn: &Arc<Mutex<Option<String>>>,
    value: &serde_json::Value,
) {
    // Codex account rate-limit frames (the reply to `account/rateLimits/read`
    // and its rolling push) are subscription telemetry, not conversation. Route
    // them straight to the ambient usage channel without persisting.
    if let Some(rate_limits) = codex_rate_limits_from_frame(value) {
        emit_account_usage(app, "codex", rate_limits);
        return;
    }
    let state = app.state::<AppState>();
    let mut pending_directives: Vec<(delegation::DelegationRequest, String)> = Vec::new();
    let mut pending_ui_events: Vec<AgentEvent> = Vec::new();
    let mut pending_telemetry: Vec<store::TelemetrySpan> = Vec::new();
    let mut turn_completed = false;
    let mut checkpoint_prompt_after_turn: Option<String> = None;
    let mut checkpoint_response_seen = false;
    let mut checkpoint_turn_handled = false;
    let mut finish_checkpointing = false;
    let mut finish_requested_shutdown = false;
    let mut recover_compaction = false;
    let bridge_state_changed;

    {
        let db = state.db.lock().unwrap();
        let session_context: Option<(Option<String>, String, i64, Option<String>, String, String)> = db
            .query_row(
                "SELECT workspace_id,harness,COALESCE(depth,0),active_turn_id,kind,COALESCE(trace_id,id) FROM sessions WHERE id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .ok();
        let Some((workspace_id, adapter_id, own_depth, stored_turn_id, session_kind, trace_id)) =
            session_context
        else {
            return;
        };
        // Direct chats are single-agent: no worker delegation and no auto-compaction.
        let is_direct = session_kind == "direct";
        let observed_turn_id = current_turn
            .lock()
            .unwrap()
            .clone()
            .or(stored_turn_id)
            .or_else(|| {
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .last_turn_by_session
                    .get(session_id)
                    .cloned()
            });
        let normalized = state.adapter_registry.normalize(&adapter_id, value);
        bridge_state_changed = normalized.iter().any(agent_event_changes_bridge_state);
        for event in &normalized {
            match event.kind.as_str() {
                "turn.started" => {
                    let turn_id = event
                        .data
                        .pointer("/turn/id")
                        .or_else(|| event.data.get("turnId"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    *current_turn.lock().unwrap() = turn_id.clone();
                    if let Some(turn_id) = &turn_id {
                        state
                            .delegations
                            .lock()
                            .unwrap()
                            .last_turn_by_session
                            .insert(session_id.into(), turn_id.clone());
                    }
                    let checkpointing = db
                        .query_row(
                            "SELECT status='checkpointing' FROM sessions WHERE id=?1",
                            params![session_id],
                            |row| row.get::<_, bool>(0),
                        )
                        .unwrap_or(false);
                    let _ = if checkpointing {
                        db.execute(
                            "UPDATE sessions SET active_turn_id=?2 WHERE id=?1",
                            params![session_id, turn_id],
                        )
                    } else {
                        db.execute(
                            "UPDATE sessions SET status='working',active_turn_id=?2 WHERE id=?1",
                            params![session_id, turn_id],
                        )
                    };
                }
                "turn.completed" => {
                    turn_completed = true;
                    *current_turn.lock().unwrap() = None;
                    let checkpointing_worker = own_depth > 0
                        && store::worker_runtime(&db, session_id)
                            .ok()
                            .flatten()
                            .is_some_and(|runtime| runtime.lifecycle_state == "checkpointing");
                    if !checkpointing_worker {
                        let _ = db.execute(
                            "UPDATE sessions SET status='ready',active_turn_id=NULL WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id]);
                    }
                }
                "approval.requested" => {
                    if own_depth > 0 {
                        let _ = session_supervisor::SessionSupervisor::transition(
                            &db,
                            session_id,
                            worker_lifecycle::WorkerLifecycleState::Waiting,
                            Some("approval_requested"),
                        );
                    } else {
                        let _ = db.execute(
                            "UPDATE sessions SET status='waiting' WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute(
                            "UPDATE workspaces SET status='waiting' WHERE id=?1",
                            params![workspace_id],
                        );
                    }
                }
                "usage.updated" => {
                    let scope = workspace_id.as_deref().unwrap_or(session_id);
                    let _ = policy::record_provider_usage(
                        &db,
                        scope,
                        session_id,
                        observed_turn_id.as_deref(),
                        &format!("provider.{adapter_id}"),
                        &event.data,
                    );
                }
                "error" if event.status.as_deref() == Some("failed") => {
                    if own_depth > 0 {
                        let lifecycle = store::worker_runtime(&db, session_id)
                            .ok()
                            .flatten()
                            .map(|runtime| runtime.lifecycle_state);
                        if lifecycle.as_deref() == Some("waiting") {
                            let _ = session_supervisor::SessionSupervisor::transition(
                                &db,
                                session_id,
                                worker_lifecycle::WorkerLifecycleState::Working,
                                Some("approval_aborted_by_error"),
                            );
                        }
                        let _ = session_supervisor::SessionSupervisor::transition(
                            &db,
                            session_id,
                            worker_lifecycle::WorkerLifecycleState::Failed,
                            Some("provider_error"),
                        );
                    } else {
                        let _ = db.execute(
                            "UPDATE sessions SET status='failed' WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute(
                            "UPDATE workspaces SET status='failed' WHERE id=?1",
                            params![workspace_id],
                        );
                    }
                }
                _ => {}
            }
        }
        for mut normalized_event in normalized {
            // A completed assistant message may carry delegation directives.
            // Spawn the workers (after the lock is released) and strip the raw
            // directive block so the conversation shows prose, not machine JSON.
            if !is_direct
                && normalized_event.kind == "message.completed"
                && normalized_event.role.as_deref() == Some("assistant")
            {
                if let Some(text) = normalized_event.text.clone() {
                    match delegation::parse_delegation_requests(&text) {
                        delegation::ParseOutcome::Parsed(requests) => {
                            let item_id = normalized_event.item_id.clone().unwrap_or_default();
                            let is_new = store::claim_delegation_receipt(&db, session_id, &item_id)
                                .unwrap_or(false);
                            let mut accepted_count = 0;
                            if is_new {
                                accepted_count = requests.len();
                                let turn_id = observed_turn_id
                                    .clone()
                                    .or_else(|| normalized_event.item_id.clone())
                                    .unwrap_or_else(|| format!("turn-{}", Uuid::new_v4()));
                                pending_directives.extend(
                                    requests
                                        .into_iter()
                                        .map(|request| (request, turn_id.clone())),
                                );
                            }
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                if accepted_count > 0 {
                                    "_Delegating to a worker…_".to_owned()
                                } else {
                                    "_Delegation request already processed._".to_owned()
                                }
                            } else {
                                stripped
                            });
                        }
                        delegation::ParseOutcome::Invalid { reason, .. } => {
                            let _ = store::event(
                                &db,
                                "delegation",
                                "delegation.request.invalid",
                                session_id,
                                &reason,
                            );
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                format!("_Invalid delegation request: {reason}_")
                            } else {
                                stripped
                            });
                        }
                        delegation::ParseOutcome::Absent => {}
                    }
                }
            }
            if let Ok(event) = store::session_event(
                &db,
                session_id,
                &normalized_event,
                &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
            ) {
                pending_telemetry.push(store::telemetry_span(
                    &trace_id,
                    session_id,
                    &adapter_id,
                    &normalized_event,
                    &event.created_at,
                ));
                pending_ui_events.push(event);
            }
            let pending_compaction =
                compaction_controller::CompactionController::pending(&db, session_id)
                    .ok()
                    .flatten();
            if normalized_event.kind == "message.completed"
                && normalized_event.role.as_deref() == Some("assistant")
                && pending_compaction.is_some()
            {
                checkpoint_response_seen = true;
                checkpoint_turn_handled = true;
                let output = normalized_event.text.as_deref().unwrap_or_default();
                match compaction_controller::CompactionController::handle_output(
                    &db, session_id, output,
                ) {
                    Ok(compaction_controller::CheckpointOutcome::Repair { prompt }) => {
                        checkpoint_prompt_after_turn = Some(prompt);
                    }
                    Ok(compaction_controller::CheckpointOutcome::Completed { .. }) => {
                        finish_checkpointing = own_depth > 0;
                        finish_requested_shutdown = pending_compaction.is_some_and(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        });
                    }
                    Ok(compaction_controller::CheckpointOutcome::Failed) => {
                        recover_compaction = true;
                        finish_checkpointing = own_depth > 0;
                        finish_requested_shutdown = pending_compaction.is_some_and(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        });
                    }
                    Ok(compaction_controller::CheckpointOutcome::NotPending) | Err(_) => {}
                }
            }
        }
        if turn_completed && checkpoint_prompt_after_turn.is_none() {
            if let Ok(Some(pending)) =
                compaction_controller::CompactionController::pending(&db, session_id)
            {
                if pending.attempt == 1 {
                    checkpoint_turn_handled = true;
                    checkpoint_prompt_after_turn = Some(
                        compaction_controller::CompactionController::checkpoint_prompt(
                            session_id,
                            &pending,
                            Some("repair the invalid checkpoint response"),
                        ),
                    );
                }
            }
        }
        if turn_completed
            && checkpoint_prompt_after_turn.is_none()
            && !checkpoint_response_seen
            && own_depth == 0
            && !is_direct
        {
            if let Ok(Some(prompt)) = begin_pressure_compaction(&db, session_id) {
                checkpoint_prompt_after_turn = Some(prompt);
            }
        }
    }

    // Telemetry is deliberately flushed only after the correctness database
    // lock and all semantic transactions are complete. A telemetry failure is
    // best-effort and cannot roll back durable local history.
    if !pending_telemetry.is_empty() {
        if let Ok(telemetry) = state.telemetry_db.try_lock() {
            let _ = store::append_telemetry_batch(&telemetry, &pending_telemetry);
        }
    }

    if turn_completed {
        if let Some(prompt) = checkpoint_prompt_after_turn {
            if let Err(error) = send_internal_checkpoint_turn(app, session_id, &prompt) {
                let db = state.db.lock().unwrap();
                let pending = compaction_controller::CompactionController::pending(&db, session_id)
                    .ok()
                    .flatten();
                let attempt = pending.as_ref().map_or(0, |pending| pending.attempt);
                let shutdown = pending.is_some_and(|pending| {
                    pending.reason == compaction_controller::CompactionReason::BeforeShutdown
                });
                let _ = compaction_controller::CompactionController::record_failure(
                    &db,
                    session_id,
                    &format!("checkpoint turn could not start: {error}"),
                    attempt,
                );
                finish_checkpointing = true;
                finish_requested_shutdown = shutdown;
            }
        }
    }
    if finish_checkpointing {
        finish_worker_checkpoint(app, session_id, adapters::ShutdownReason::Completed);
    }
    if recover_compaction {
        let _ = run_compaction_recovery(app, session_id);
    }
    if finish_requested_shutdown {
        finish_orchestrator_shutdown(app, session_id, adapters::ShutdownReason::UserStopped);
    }

    for (directive, turn_id) in &pending_directives {
        let _ = launch_worker(app, session_id, turn_id, directive, true);
    }
    // When this session's own turn ends and it is not waiting on any child
    // worker, hand its result up to its parent (no-op if it has no parent).
    if turn_completed && !checkpoint_response_seen && !checkpoint_turn_handled {
        let idle =
            store::outstanding_children(&state.db.lock().unwrap(), session_id).unwrap_or(0) == 0;
        if idle {
            forward_turn_result(app, session_id);
        }
    }
    for event in pending_ui_events {
        let _ = app.emit("agent-event", event);
    }
    if bridge_state_changed {
        let _ = app.emit("state-changed", ());
    }
}

fn begin_pressure_compaction(
    db: &Connection,
    session_id: &str,
) -> Result<Option<String>, BridgeError> {
    let context_percent = db
        .query_row(
            "SELECT CAST(context_percent AS REAL) FROM usage_ledger WHERE session_id=?1 AND context_percent IS NOT NULL ORDER BY id DESC LIMIT 1",
            params![session_id],
            |row| row.get::<_, f64>(0),
        )
        .ok();
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let last_boundary = branch
        .iter()
        .rposition(|entry| entry.kind == "compaction")
        .map_or(0, |index| index + 1);
    let meaningful = branch[last_boundary..].iter().any(|entry| {
        matches!(
            entry.kind.as_str(),
            "user.message" | "assistant.message" | "worker.result" | "tool.completed"
        )
    });
    let trigger = compaction_controller::TriggerState {
        reason: compaction_controller::CompactionReason::ContextPressure,
        context_percent,
        projected_tokens_with_reserve: None,
        context_window_tokens: None,
        has_valid_typed_result: false,
        one_shot_worker: false,
        tool_call_active: false,
        approval_active: false,
        has_meaningful_new_work: meaningful,
        wall_clock_only: false,
    };
    let Ok(reason) = compaction_controller::decide(&trigger) else {
        return Ok(None);
    };
    let tokens = compaction_controller::active_token_estimate(db, session_id)?;
    compaction_controller::CompactionController::begin(db, session_id, reason, tokens)
}

fn send_internal_checkpoint_turn(
    app: &AppHandle,
    session_id: &str,
    prompt: &str,
) -> Result<(), BridgeError> {
    let state = app.state::<AppState>();
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(session_id)
        .ok_or_else(|| BridgeError::Invalid("checkpoint agent process is not running".into()))?;
    runtime.send_turn(prompt)?;
    drop(adapters);
    store::event(
        &state.db.lock().unwrap(),
        "compaction",
        "checkpoint.turn_started",
        session_id,
        "Checkpoint-only structured turn started",
    )?;
    Ok(())
}

fn finish_worker_checkpoint(app: &AppHandle, session_id: &str, reason: adapters::ShutdownReason) {
    let state = app.state::<AppState>();
    let should_stop = {
        let db = state.db.lock().unwrap();
        let checkpointing = store::worker_runtime(&db, session_id)
            .ok()
            .flatten()
            .is_some_and(|runtime| runtime.lifecycle_state == "checkpointing");
        if checkpointing {
            let _ = session_supervisor::SessionSupervisor::transition(
                &db,
                session_id,
                worker_lifecycle::WorkerLifecycleState::Stopped,
                Some("checkpoint_turn_finished"),
            );
            let _ = db.execute(
                "UPDATE worker_leases SET lease_status='checkpointed',updated_at=?2 WHERE session_id=?1",
                params![session_id, Utc::now().to_rfc3339()],
            );
        }
        checkpointing
    };
    if should_stop {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(session_id) {
            runtime.stop(reason);
        }
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            session_id,
        );
    }
}

fn finish_orchestrator_shutdown(
    app: &AppHandle,
    session_id: &str,
    reason: adapters::ShutdownReason,
) {
    let state = app.state::<AppState>();
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(session_id) {
        runtime.stop(reason);
    }
    let db = state.db.lock().unwrap();
    let workspace_id = db
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let _ = record_shutdown_reason(&db, session_id, reason);
    let _ = db.execute(
        "UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1",
        params![session_id, Utc::now().to_rfc3339()],
    );
    if let Some(workspace_id) = workspace_id {
        let _ = db.execute(
            "UPDATE workspaces SET status='stopped' WHERE id=?1",
            params![workspace_id],
        );
    }
    let _ = app.emit("state-changed", ());
}

fn run_compaction_recovery(app: &AppHandle, session_id: &str) -> Result<(), BridgeError> {
    let state = app.state::<AppState>();
    let workspace_path: String = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT w.path FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
            params![session_id],
            |row| row.get(0),
        )?
    };
    let git_status = worker_guard::tracked_status(Path::new(&workspace_path)).unwrap_or_default();
    compaction_controller::CompactionController::reconstruct_from_normalized_events_and_git(
        &state.db.lock().unwrap(),
        session_id,
        &git_status,
    )?;
    Ok(())
}

/// Spawn a child worker session in the parent's workspace and hand it its task.
struct WorkerLaunchReservation {
    session_id: String,
    workspace_id: String,
    depth: i64,
    path: String,
    branch: String,
    actual_model: String,
    outcome: policy::PolicyOutcome,
    reuse_existing: bool,
}

enum WorkerReservationOutcome {
    Reserved(WorkerLaunchReservation),
    Queued,
    Blocked,
}

enum WorkerLaunchOutcome {
    Launched(String),
    Queued,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerActivation {
    Fresh,
    Native,
    CheckpointRestored,
}

fn record_model_resolution_warning(
    db: &Connection,
    parent_session_id: &str,
    resolution: &adapters::ModelResolution,
) -> Result<(), BridgeError> {
    if let Some(warning) = &resolution.warning {
        store::event(
            db,
            "capability",
            "capability.model_fallback",
            parent_session_id,
            warning,
        )?;
    }
    Ok(())
}

fn reserve_worker_launch_outcome(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    actual_model: &str,
    queue_on_block: bool,
    router_decision_id: Option<&str>,
) -> Result<WorkerReservationOutcome, BridgeError> {
    let route = policy_coordinator::PolicyCoordinator::decide_worker_route(
        db,
        parent_session_id,
        turn_id,
        directive,
        true,
    )?;
    let policy_coordinator::WorkerRouteContext {
        workspace_id,
        parent_depth,
        path,
        branch,
        outcome,
    } = route;
    if let Some(decision_id) = router_decision_id {
        learning_router::record_policy_result(db, decision_id, &outcome)?;
    }
    let handoff = handoff::assess(db, parent_session_id, &directive.runtime_harness())?;
    if queue_on_block && handoff.cross_harness && !handoff.at_phase_boundary {
        worker_pool::WorkerPool::enqueue(
            db,
            parent_session_id,
            &workspace_id,
            turn_id,
            directive,
            actual_model,
        )?;
        store::event(
            db,
            "handoff",
            "handoff.deferred_for_phase_boundary",
            parent_session_id,
            &format!(
                "Deferred {} to {} until the active turn reaches a phase boundary",
                handoff.source_harness, handoff.target_harness
            ),
        )?;
        return Ok(WorkerReservationOutcome::Queued);
    }
    match &outcome.decision {
        policy::RouteDecision::Queue => {
            if queue_on_block {
                worker_pool::WorkerPool::enqueue(
                    db,
                    parent_session_id,
                    &workspace_id,
                    turn_id,
                    directive,
                    actual_model,
                )?;
            }
            return Ok(if queue_on_block {
                WorkerReservationOutcome::Queued
            } else {
                WorkerReservationOutcome::Blocked
            });
        }
        policy::RouteDecision::ResumeWorker { session_id } => {
            let runtime = store::worker_runtime(db, session_id)?.ok_or_else(|| {
                BridgeError::Invalid(format!("warm worker {session_id} has no runtime record"))
            })?;
            let worker_path = runtime.worktree_path.unwrap_or_else(|| path.clone());
            let worker_branch = runtime.worktree_branch.unwrap_or_else(|| branch.clone());
            return Ok(WorkerReservationOutcome::Reserved(
                WorkerLaunchReservation {
                    session_id: session_id.clone(),
                    workspace_id,
                    depth: parent_depth + 1,
                    path: worker_path,
                    branch: worker_branch,
                    actual_model: actual_model.into(),
                    outcome,
                    reuse_existing: true,
                },
            ));
        }
        policy::RouteDecision::SpawnWorker(_) => {}
        policy::RouteDecision::RequireUserApproval => {
            db.execute(
                "UPDATE sessions SET status='waiting' WHERE id=?1",
                params![parent_session_id],
            )?;
            db.execute(
                "UPDATE workspaces SET status='waiting' WHERE id=?1",
                params![workspace_id],
            )?;
            return Ok(WorkerReservationOutcome::Blocked);
        }
        _ => return Ok(WorkerReservationOutcome::Blocked),
    }

    let session_id = Uuid::new_v4().to_string();
    let depth = parent_depth + 1;
    let harness = directive.runtime_harness();
    let effort = directive.effort.as_str();
    let now = Utc::now().to_rfc3339();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,model,requested_tier,effort,parent_session_id,depth) VALUES(?1,?2,?3,?4,'starting',?5,'reported',?6,?7,?8,?9,?10)",
        params![
            session_id,
            workspace_id,
            harness,
            directive.label(),
            now,
            actual_model,
            directive.capability_tier.as_str(),
            effort,
            parent_session_id,
            depth,
        ],
    )?;
    let compatibility_key =
        worker_pool::WorkerCompatibilityKey::for_request(&workspace_id, directive)?.encode()?;
    store::upsert_worker_lease(
        &transaction,
        &WorkerLease {
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            role: policy::role_name(directive.role).into(),
            capability_tier: policy::tier_name(directive.capability_tier).into(),
            task_family: policy::role_name(directive.role).into(),
            owned_paths: serde_json::json!(directive.owned_paths),
            write_mode: policy::write_mode_name(directive.write_mode).into(),
            lease_status: "active".into(),
            expires_at: None,
            created_at: now.clone(),
            updated_at: now,
        },
    )?;
    store::upsert_worker_runtime(
        &transaction,
        &WorkerRuntimeRecord {
            session_id: session_id.clone(),
            parent_session_id: parent_session_id.to_owned(),
            lifecycle_state: "starting".into(),
            task_family: worker_pool::task_family(directive),
            compatibility_key,
            result_status: "pending".into(),
            retry_count: 0,
            warm_until: None,
            worktree_path: None,
            worktree_branch: None,
            last_result: None,
            updated_at: Utc::now().to_rfc3339(),
        },
    )?;
    policy::record_spawn_usage(
        &transaction,
        &workspace_id,
        &session_id,
        turn_id,
        &outcome,
        directive.capability_tier,
    )?;
    let outbox_created_at = Utc::now().to_rfc3339();
    store::enqueue_outbox(
        &transaction,
        &OutboxMessage {
            id: Uuid::new_v4().to_string(),
            destination: "integration".into(),
            event_type: "worker.spawned".into(),
            payload: serde_json::json!({
                "sessionId": session_id,
                "parentSessionId": parent_session_id,
                "turnId": turn_id,
            }),
            idempotency_key: format!("worker-spawn:{parent_session_id}:{turn_id}:{session_id}"),
            status: "pending".into(),
            attempt_count: 0,
            next_attempt_at: outbox_created_at.clone(),
            last_error: None,
            created_at: outbox_created_at,
            delivered_at: None,
        },
    )?;
    transaction.commit()?;
    Ok(WorkerReservationOutcome::Reserved(
        WorkerLaunchReservation {
            session_id,
            workspace_id,
            depth,
            path,
            branch,
            actual_model: actual_model.into(),
            outcome,
            reuse_existing: false,
        },
    ))
}

#[cfg(test)]
fn reserve_worker_launch(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    actual_model: &str,
    queue_on_block: bool,
) -> Result<Option<WorkerLaunchReservation>, BridgeError> {
    Ok(
        match reserve_worker_launch_outcome(
            db,
            parent_session_id,
            turn_id,
            directive,
            actual_model,
            queue_on_block,
            None,
        )? {
            WorkerReservationOutcome::Reserved(reservation) => Some(reservation),
            WorkerReservationOutcome::Queued | WorkerReservationOutcome::Blocked => None,
        },
    )
}

fn launch_worker_outcome(
    app: &AppHandle,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> WorkerLaunchOutcome {
    let state = app.state::<AppState>();
    let routed = {
        let db = state.db.lock().unwrap();
        learning_router::route(
            &db,
            parent_session_id,
            turn_id,
            directive,
            &state.adapter_registry.descriptors(),
        )
    };
    let routed = match routed {
        Ok(routed) => routed,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "router",
                "router.no_eligible_route",
                parent_session_id,
                &error.to_string(),
            );
            return WorkerLaunchOutcome::Failed;
        }
    };
    let directive = &routed.request;
    let harness = directive.runtime_harness();
    if !agent_config::is_harness_enabled(&state.db.lock().unwrap(), &harness) {
        let db = state.db.lock().unwrap();
        let _ = learning_router::record_route_status(&db, &routed.decision.id, "harness_disabled");
        let _ = store::event(
            &db,
            "capability",
            "capability.harness_disabled",
            parent_session_id,
            &format!("{harness} is disabled in Settings"),
        );
        drop(db);
        let _ = app.emit("state-changed", ());
        return WorkerLaunchOutcome::Failed;
    }
    let resolution = match state.adapter_registry.resolve_model(
        &harness,
        directive.capability_tier,
        directive.model.as_deref(),
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = learning_router::record_route_status(
                &db,
                &routed.decision.id,
                "model_resolution_failed",
            );
            let _ = store::event(
                &db,
                "capability",
                "capability.resolution_failed",
                parent_session_id,
                &error.to_string(),
            );
            drop(db);
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
    };
    let reservation = {
        let db = state.db.lock().unwrap();
        let _ = record_model_resolution_warning(&db, parent_session_id, &resolution);
        record_actual_execution_best_effort(
            &db,
            &routed.decision.id,
            &harness,
            &resolution.actual_model,
            directive.effort,
            parent_session_id,
        );
        reserve_worker_launch_outcome(
            &db,
            parent_session_id,
            turn_id,
            directive,
            &resolution.actual_model,
            queue_on_block,
            Some(&routed.decision.id),
        )
    };
    let mut reservation = match reservation {
        Ok(WorkerReservationOutcome::Reserved(reservation)) => reservation,
        Ok(WorkerReservationOutcome::Queued) => {
            let _ = learning_router::record_route_status(
                &state.db.lock().unwrap(),
                &routed.decision.id,
                "queued",
            );
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Queued;
        }
        Ok(WorkerReservationOutcome::Blocked) => {
            let _ = learning_router::record_route_status(
                &state.db.lock().unwrap(),
                &routed.decision.id,
                "policy_blocked",
            );
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = learning_router::record_route_status(&db, &routed.decision.id, "policy_failed");
            let _ = store::event(
                &db,
                "policy",
                "policy.decision_failed",
                parent_session_id,
                &error.to_string(),
            );
            drop(db);
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
    };
    if let Err(error) = learning_router::bind_worker(
        &state.db.lock().unwrap(),
        &routed.decision.id,
        &reservation.session_id,
    ) {
        fail_reserved_worker(
            app,
            &reservation.session_id,
            &directive.label(),
            &format!("Could not bind learning-router outcome: {error}"),
        );
        return WorkerLaunchOutcome::Failed;
    }
    let _ = learning_router::record_route_status(
        &state.db.lock().unwrap(),
        &routed.decision.id,
        "reserved",
    );
    let completion_input = serde_json::to_string(directive)
        .map_err(|error| BridgeError::Invalid(format!("Could not serialize worker completion input: {error}")))
        .and_then(|serialized| state.db.lock().unwrap().execute(
            "INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES(?1,?2,?3) ON CONFLICT(child_session_id) DO UPDATE SET request=excluded.request,updated_at=excluded.updated_at",
            params![reservation.session_id, serialized, Utc::now().to_rfc3339()],
        ).map(|_| ()).map_err(BridgeError::from));
    if let Err(error) = completion_input {
        fail_reserved_worker(
            app,
            &reservation.session_id,
            &directive.label(),
            &error.to_string(),
        );
        return WorkerLaunchOutcome::Failed;
    }
    if directive.role == delegation::WorkerRole::Verification {
        let verification_path: Result<String, BridgeError> = state.db.lock().unwrap().query_row(
            "SELECT repository_path FROM eval_attempts WHERE session_id=?1 AND status IN ('verifying','changes_requested','failed') ORDER BY started_at DESC,rowid DESC LIMIT 1",
            params![parent_session_id],
            |row| row.get(0),
        ).map_err(BridgeError::from);
        match verification_path {
            Ok(path) => {
                reservation.path = path.clone();
                let _ = state.db.lock().unwrap().execute(
                    "UPDATE worker_runtime SET worktree_path=?2,updated_at=?3 WHERE session_id=?1",
                    params![reservation.session_id, path, Utc::now().to_rfc3339()],
                );
            }
            Err(error) => {
                fail_reserved_worker(
                    app,
                    &reservation.session_id,
                    &directive.label(),
                    &format!("Could not bind verifier to the implementation revision: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
            }
        }
    }
    let requires_child_worktree = matches!(
        &reservation.outcome.decision,
        policy::RouteDecision::SpawnWorker(spec) if spec.requires_child_worktree
    );
    if requires_child_worktree {
        match worktree_coordinator::WorktreeCoordinator::prepare_isolated_worker(
            &state.db.lock().unwrap(),
            &state.worktrees.join("workers"),
            &reservation.workspace_id,
            Path::new(&reservation.path),
            &reservation.branch,
            &reservation.session_id,
            &directive.owned_paths,
        ) {
            Ok((path, branch)) => {
                reservation.path = path.to_string_lossy().into_owned();
                reservation.branch = branch;
            }
            Err(error) => {
                let db = state.db.lock().unwrap();
                let transaction = match db.unchecked_transaction() {
                    Ok(transaction) => transaction,
                    Err(_) => return WorkerLaunchOutcome::Failed,
                };
                let _ = transaction.execute(
                    "DELETE FROM worker_runtime WHERE session_id=?1",
                    params![reservation.session_id],
                );
                let _ = transaction.execute(
                    "DELETE FROM worker_leases WHERE session_id=?1",
                    params![reservation.session_id],
                );
                let _ = transaction.execute(
                    "DELETE FROM sessions WHERE id=?1",
                    params![reservation.session_id],
                );
                let _ = transaction.commit();
                let queued = queue_on_block
                    && worker_pool::WorkerPool::enqueue(
                        &db,
                        parent_session_id,
                        &reservation.workspace_id,
                        turn_id,
                        directive,
                        &reservation.actual_model,
                    )
                    .is_ok();
                let _ = store::event(
                    &db,
                    "worktree",
                    "worker.worktree_queued",
                    parent_session_id,
                    &error.to_string(),
                );
                let _ = learning_router::record_route_status(
                    &db,
                    &routed.decision.id,
                    if queued { "queued" } else { "worktree_failed" },
                );
                let _ = app.emit("state-changed", ());
                return if queued {
                    WorkerLaunchOutcome::Queued
                } else {
                    WorkerLaunchOutcome::Failed
                };
            }
        }
    }
    let model = reservation.actual_model.clone();
    let effort = directive.effort.as_str().to_owned();
    let label = directive.label();
    let evidence = match session_supervisor::SessionSupervisor::worker_evidence(
        &state.db.lock().unwrap(),
        parent_session_id,
        &directive.evidence_ids,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            fail_reserved_worker(
                app,
                &reservation.session_id,
                &label,
                &format!("Could not resolve worker evidence: {error}"),
            );
            return WorkerLaunchOutcome::Failed;
        }
    };
    let role = directive.role.as_str();
    let configured_prompt = agent_config::prompt_suffix(&state.db.lock().unwrap(), &harness, role);
    let mut instructions = format!(
        "{}{}\n\n{}",
        delegation::worker_briefing(directive, reservation.depth, &reservation.branch, &evidence),
        if configured_prompt.is_empty() {
            String::new()
        } else {
            format!("\n\n{configured_prompt}")
        },
        state
            .credential_broker
            .instructions(&reservation.session_id)
    );

    if reservation.reuse_existing
        && state
            .adapters
            .lock()
            .unwrap()
            .contains_key(&reservation.session_id)
    {
        let current = store::worker_runtime(&state.db.lock().unwrap(), &reservation.session_id)
            .ok()
            .flatten()
            .map(|runtime| runtime.lifecycle_state);
        let transition_result = match current.as_deref() {
            Some("warm") => session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("compatible_hot_task"),
            ),
            Some("stopped") => session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                worker_lifecycle::WorkerLifecycleState::Resuming,
                Some("compatible_hot_task"),
            )
            .and_then(|_| {
                session_supervisor::SessionSupervisor::transition(
                    &state.db.lock().unwrap(),
                    &reservation.session_id,
                    worker_lifecycle::WorkerLifecycleState::Working,
                    Some("hot_process_reused"),
                )
            }),
            _ => Err(BridgeError::Invalid(
                "compatible hot worker is not reusable".into(),
            )),
        };
        if transition_result.is_ok()
            && worker_pool::WorkerPool::activate_reused_worker(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                &reservation.workspace_id,
                directive,
            )
            .is_ok()
        {
            let provider_session_id = state
                .adapters
                .lock()
                .unwrap()
                .get(&reservation.session_id)
                .map(|runtime| runtime.provider_session_id().to_owned());
            let _ = restoration::set_head_state(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                RestorationMode::Hot,
                ResumeEligibility::Native,
                provider_session_id.as_deref(),
            );
            let _ = handoff::record_fidelity(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                ContinuationFidelity::Native,
            );
            if let Some(runtime) = state.adapters.lock().unwrap().get(&reservation.session_id) {
                if runtime.send_turn(&instructions).is_ok() {
                    let _ = learning_router::record_route_status(
                        &state.db.lock().unwrap(),
                        &routed.decision.id,
                        "launched",
                    );
                    let _ = app.emit("state-changed", ());
                    return WorkerLaunchOutcome::Launched(reservation.session_id);
                }
            }
        }
        let _ = store::event(
            &state.db.lock().unwrap(),
            "worker-pool",
            "worker.hot_resume_failed",
            &reservation.session_id,
            "Could not reactivate compatible hot worker",
        );
        return WorkerLaunchOutcome::Failed;
    }

    let mut read_only_sandbox = None;
    if directive.write_mode == delegation::WriteMode::ReadOnly {
        match worker_guard::ReadOnlyBaseline::capture(&reservation.path) {
            Ok(baseline) => {
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .read_only_baselines
                    .insert(reservation.session_id.clone(), baseline);
            }
            Err(error) => {
                fail_reserved_worker(
                    app,
                    &reservation.session_id,
                    &label,
                    &format!("Could not capture tracked-file baseline: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
            }
        }
        match worker_sandbox::ReadOnlySandbox::create(
            &reservation.session_id,
            Path::new(&reservation.path),
            directive,
        ) {
            Ok(sandbox) => {
                let output = sandbox.output_dir.display().to_string();
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .read_only_sandboxes
                    .insert(reservation.session_id.clone(), sandbox.clone());
                read_only_sandbox = Some(sandbox);
                instructions.push_str(&format!("\n\nRead-only OS isolation is active. The workspace is immutable and network access is {}. Write artifacts only under BRIDGE_WORKER_OUTPUT_DIR: {output}", if directive.network_access { "authorized" } else { "denied" }));
                let _ = store::event(
                    &state.db.lock().unwrap(),
                    "sandbox",
                    "worker.read_only_isolation_prepared",
                    &reservation.session_id,
                    &format!(
                        "mode=seatbelt network_allowed={} output_dir={output}",
                        directive.network_access
                    ),
                );
            }
            Err(error) => {
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .read_only_baselines
                    .remove(&reservation.session_id);
                fail_reserved_worker(
                    app,
                    &reservation.session_id,
                    &label,
                    &format!("Could not establish read-only OS isolation: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
            }
        }
    }

    let activation = if reservation.reuse_existing {
        if session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            &reservation.session_id,
            worker_lifecycle::WorkerLifecycleState::Resuming,
            Some("compatible_cold_task"),
        )
        .is_err()
        {
            return WorkerLaunchOutcome::Failed;
        }
        let provider_id: Option<String> = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT provider_session_id FROM sessions WHERE id=?1",
                params![reservation.session_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let checkpoint =
            restoration::checkpoint_context(&state.db.lock().unwrap(), &reservation.session_id)
                .ok()
                .flatten();
        let resumed = provider_id
            .as_deref()
            .filter(|_| state.adapter_registry.supports_native_resume(&harness))
            .map(|provider_session_id| {
                state.adapter_registry.resume(
                    &harness,
                    adapters::ResumeRequest {
                        provider_session_id,
                        cwd: &reservation.path,
                        model: Some(model.as_str()),
                        effort: Some(&effort),
                        instructions: Some(instructions.as_str()),
                        write_mode: Some(directive.write_mode),
                        read_only_sandbox: read_only_sandbox.as_ref(),
                    },
                )
            })
            .transpose();
        match resumed {
            Ok(Some(started)) => Ok((started, WorkerActivation::Native)),
            Err(error) => {
                let _ = restoration::record_resume_failed(
                    &state.db.lock().unwrap(),
                    &reservation.session_id,
                    &error.to_string(),
                );
                let restored_instructions = format!("{instructions}\n\n{}", checkpoint.unwrap_or_else(|| "Bridge checkpoint-restoration context: prior typed worker result is stored in the session forest.".into()));
                state
                    .adapter_registry
                    .start(
                        &harness,
                        adapters::StartRequest {
                            cwd: &reservation.path,
                            model: Some(model.as_str()),
                            effort: Some(&effort),
                            instructions: Some(restored_instructions.as_str()),
                            write_mode: Some(directive.write_mode),
                            read_only_sandbox: read_only_sandbox.as_ref(),
                        },
                    )
                    .map(|started| (started, WorkerActivation::CheckpointRestored))
            }
            Ok(None) => {
                let restored_instructions = format!("{instructions}\n\n{}", checkpoint.unwrap_or_else(|| "Bridge checkpoint-restoration context: prior typed worker result is stored in the session forest.".into()));
                state
                    .adapter_registry
                    .start(
                        &harness,
                        adapters::StartRequest {
                            cwd: &reservation.path,
                            model: Some(model.as_str()),
                            effort: Some(&effort),
                            instructions: Some(restored_instructions.as_str()),
                            write_mode: Some(directive.write_mode),
                            read_only_sandbox: read_only_sandbox.as_ref(),
                        },
                    )
                    .map(|started| (started, WorkerActivation::CheckpointRestored))
            }
        }
    } else {
        state
            .adapter_registry
            .start(
                &harness,
                adapters::StartRequest {
                    cwd: &reservation.path,
                    model: Some(model.as_str()),
                    effort: Some(&effort),
                    instructions: Some(instructions.as_str()),
                    write_mode: Some(directive.write_mode),
                    read_only_sandbox: read_only_sandbox.as_ref(),
                },
            )
            .map(|started| (started, WorkerActivation::Fresh))
    };
    let (started, activation) = match activation {
        Ok(started) => started,
        Err(error) => {
            state
                .delegations
                .lock()
                .unwrap()
                .read_only_baselines
                .remove(&reservation.session_id);
            if let Some(sandbox) = state
                .delegations
                .lock()
                .unwrap()
                .read_only_sandboxes
                .remove(&reservation.session_id)
            {
                sandbox.cleanup();
            }
            fail_reserved_worker(
                app,
                &reservation.session_id,
                &label,
                &format!("Could not start provider process: {error}"),
            );
            return WorkerLaunchOutcome::Failed;
        }
    };
    let session_id = reservation.session_id;
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;
    let startup_messages = started.startup_messages;
    let mut runtime = started.runtime;
    let started_at = Utc::now().to_rfc3339();

    if let Err(error) = session_supervisor::SessionSupervisor::track_adapter_process(
        &state.db.lock().unwrap(),
        &session_id,
        runtime.process_id(),
    ) {
        runtime.stop(adapters::ShutdownReason::Failed);
        fail_reserved_worker(
            app,
            &session_id,
            &label,
            &format!("Could not track provider process: {error}"),
        );
        return WorkerLaunchOutcome::Failed;
    }

    let transition_result = match activation {
        WorkerActivation::Fresh | WorkerActivation::Native => {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some(if activation == WorkerActivation::Native {
                    "native_resumed"
                } else {
                    "provider_started"
                }),
            )
            .map(|_| ())
        }
        WorkerActivation::CheckpointRestored => session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            &session_id,
            worker_lifecycle::WorkerLifecycleState::Restored,
            Some("checkpoint_fallback"),
        )
        .and_then(|_| {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("checkpoint_restored"),
            )
        })
        .map(|_| ()),
    };
    if let Err(error) = transition_result {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        let _ = store::event(
            &state.db.lock().unwrap(),
            "supervisor",
            "worker.transition_failed",
            &session_id,
            &error.to_string(),
        );
        return WorkerLaunchOutcome::Failed;
    }
    let (restoration_mode, resume_eligibility) = match activation {
        WorkerActivation::Fresh => (RestorationMode::Fresh, ResumeEligibility::Fresh),
        WorkerActivation::Native => (RestorationMode::Native, ResumeEligibility::Native),
        WorkerActivation::CheckpointRestored => (
            RestorationMode::CheckpointRestored,
            ResumeEligibility::CheckpointRestored,
        ),
    };
    let continuation_fidelity = match activation {
        WorkerActivation::Native => ContinuationFidelity::Native,
        WorkerActivation::CheckpointRestored => ContinuationFidelity::ProjectedAtBoundary,
        WorkerActivation::Fresh => {
            handoff::assess(&state.db.lock().unwrap(), parent_session_id, &harness)
                .map(|assessment| handoff::fidelity_for_projection(&assessment))
                .unwrap_or(ContinuationFidelity::ProjectedMidTurn)
        }
    };
    if restoration::set_head_state(
        &state.db.lock().unwrap(),
        &session_id,
        restoration_mode,
        resume_eligibility,
        Some(&thread_id),
    )
    .is_err()
    {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        return WorkerLaunchOutcome::Failed;
    }
    if handoff::record_fidelity(
        &state.db.lock().unwrap(),
        &session_id,
        continuation_fidelity,
    )
    .is_err()
    {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        return WorkerLaunchOutcome::Failed;
    }

    {
        let db = state.db.lock().unwrap();
        let _ = db.execute(
            "UPDATE sessions SET status='working',started_at=?2,provider_session_id=?3,label=?4,model=?5,effort=?6 WHERE id=?1",
            params![
                session_id,
                started_at,
                thread_id,
                label,
                model,
                effort,
            ],
        );
        let _ = db.execute(
            "UPDATE workspaces SET status='working' WHERE id=?1",
            params![reservation.workspace_id],
        );
        for message in &startup_messages {
            let _ =
                persist_agent_value(&db, &state.adapter_registry, &harness, &session_id, message);
        }
        let spawn_event = agent::NormalizedEvent {
            kind: if reservation.reuse_existing {
                "delegation.resumed".into()
            } else {
                "delegation.spawned".into()
            },
            item_id: Some(format!("spawn-{session_id}")),
            role: Some("system".into()),
            status: Some("working".into()),
            title: Some(if reservation.reuse_existing {
                format!("Resumed {label}")
            } else {
                format!("Delegated to {label}")
            }),
            text: Some(directive.objective.clone()),
            data: serde_json::json!({
                "childSessionId": session_id,
                "request": directive,
                "harness": harness,
                "requestedTier": directive.capability_tier,
                "model": model,
                "modelLabel": delegation::model_display(&model),
                "effort": effort,
                "depth": reservation.depth,
                "turnId": turn_id,
                "policy": reservation.outcome,
                "restorationMode": restoration_mode,
                "continuationFidelity": continuation_fidelity,
            }),
        };
        if let Ok(stored) = store::session_event(
            &db,
            parent_session_id,
            &spawn_event,
            &serde_json::json!({"delegation": true}),
        ) {
            let _ = app.emit("agent-event", stored);
        }
        let _ = store::event(
            &db,
            "delegation",
            if reservation.reuse_existing {
                "worker.resumed"
            } else {
                "worker.spawned"
            },
            parent_session_id,
            &format!(
                "{} {label} (effort {})",
                if reservation.reuse_existing {
                    "Resumed"
                } else {
                    "Spawned"
                },
                effort
            ),
        );
    }

    if reservation.reuse_existing
        && worker_pool::WorkerPool::activate_reused_worker(
            &state.db.lock().unwrap(),
            &session_id,
            &reservation.workspace_id,
            directive,
        )
        .is_err()
    {
        return WorkerLaunchOutcome::Failed;
    }
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), runtime);
    spawn_reader_thread(
        app.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    if let Err(error) = deliver_worker_objective(&state.adapters, &session_id, &directive.objective)
    {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::Failed);
        }
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        fail_reserved_worker(
            app,
            &session_id,
            &label,
            &format!("Could not deliver worker objective: {error}"),
        );
        return WorkerLaunchOutcome::Failed;
    }
    let _ = app.emit("state-changed", ());
    let _ = learning_router::record_route_status(
        &state.db.lock().unwrap(),
        &routed.decision.id,
        "launched",
    );
    WorkerLaunchOutcome::Launched(session_id)
}

fn record_actual_execution_best_effort(
    db: &Connection,
    decision_id: &str,
    harness: &str,
    model: &str,
    effort: delegation::Effort,
    parent_session_id: &str,
) {
    if let Err(error) =
        learning_router::record_actual_execution(db, decision_id, harness, model, effort)
    {
        let _ = learning_router::record_route_status(
            db,
            decision_id,
            "actual_resolution_record_failed",
        );
        let _ = store::event(
            db,
            "router",
            "router.actual_resolution_record_failed",
            parent_session_id,
            &error.to_string(),
        );
    }
}

fn deliver_worker_objective(
    adapters: &Mutex<HashMap<String, Box<dyn adapters::AdapterRuntime>>>,
    session_id: &str,
    objective: &str,
) -> Result<(), BridgeError> {
    adapters
        .lock()
        .unwrap()
        .get(session_id)
        .ok_or_else(|| {
            BridgeError::Invalid("Worker runtime disappeared before objective delivery".into())
        })?
        .send_turn(objective)
}

fn launch_worker(
    app: &AppHandle,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> Option<String> {
    match launch_worker_outcome(app, parent_session_id, turn_id, directive, queue_on_block) {
        WorkerLaunchOutcome::Launched(session_id) => Some(session_id),
        WorkerLaunchOutcome::Queued | WorkerLaunchOutcome::Failed => None,
    }
}

fn fail_reserved_worker(app: &AppHandle, session_id: &str, label: &str, reason: &str) {
    let state = app.state::<AppState>();
    if prepare_worker_failure_settlement(&state.db.lock().unwrap(), session_id).is_err() {
        return;
    }
    let result = delegation::WorkerResult {
        schema_version: delegation::SCHEMA_VERSION,
        status: delegation::WorkerResultStatus::Failed,
        summary: format!("{label} could not start: {reason}"),
        files_changed: vec![],
        tests: vec![],
        decisions: vec![],
        risks: vec![reason.to_owned()],
        remaining_work: vec!["Retry or delegate the task differently".into()],
        suggested_next_action: delegation::SuggestedNextAction::Finish,
        suggested_role: None,
        suggested_task: None,
    };
    if settle_worker_after_result(app, session_id, &result).unwrap_or(false) {
        report_to_parent(app, session_id, &result);
    }
}

fn prepare_worker_failure_settlement(db: &Connection, session_id: &str) -> Result<(), BridgeError> {
    let current: String = db.query_row(
        "SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    if current == "working" {
        return Ok(());
    }
    session_supervisor::SessionSupervisor::transition(
        db,
        session_id,
        worker_lifecycle::WorkerLifecycleState::Working,
        Some("startup_failed_before_process"),
    )
    .map(|_| ())
}

/// Frame a finished worker's final message and send it up to its parent.
fn forward_turn_result(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    let meta: Option<(
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
    )> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT parent_session_id,label,harness,model,effort FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok()
    };
    let Some((parent, label, harness, model, effort)) = meta else {
        return;
    };
    if parent.is_none() {
        return;
    }
    let raw_output: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT json_extract(payload,'$.text') FROM session_entries
             WHERE session_id=?1 AND kind='message.completed'
               AND json_extract(payload,'$.role')='assistant'
               AND COALESCE(json_extract(payload,'$.text'),'')<>''
             ORDER BY sequence DESC LIMIT 1",
            params![child_session_id],
            |r| r.get(0),
        )
        .ok()
    };
    let raw_output =
        raw_output.unwrap_or_else(|| "(worker finished without a text summary)".to_owned());
    let result = match {
        let db = state.db.lock().unwrap();
        let mut delegations = state.delegations.lock().unwrap();
        process_worker_result_output(
            &db,
            &mut delegations.result_repairs,
            child_session_id,
            &raw_output,
            |prompt| {
                state
                    .adapters
                    .lock()
                    .unwrap()
                    .get(child_session_id)
                    .is_some_and(|runtime| runtime.send_turn(prompt).is_ok())
            },
        )
    } {
        Ok(result) => result,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "delegation",
                "worker.result.processing_failed",
                child_session_id,
                &error.to_string(),
            );
            return;
        }
    };
    let Some(result) = result else {
        let _ = app.emit("state-changed", ());
        return;
    };
    verify_read_only_worker(app, child_session_id);
    let _ = (label, harness, model, effort);
    match settle_worker_after_result(app, child_session_id, &result) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = store::event(
                &state.db.lock().unwrap(),
                "supervisor",
                "worker.settle_failed",
                child_session_id,
                &error.to_string(),
            );
            return;
        }
    }
    report_to_parent(app, child_session_id, &result);
    let terminal = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT lifecycle_state IN ('completed','cancelled') FROM worker_runtime WHERE session_id=?1",
            params![child_session_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if terminal {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(child_session_id) {
            runtime.stop(adapters::ShutdownReason::Completed);
        }
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            child_session_id,
        );
    }
}

fn process_worker_result_output(
    db: &Connection,
    tracker: &mut delegation::ResultRepairTracker,
    child_session_id: &str,
    raw_output: &str,
    send_same_session_repair: impl FnOnce(&str) -> bool,
) -> Result<Option<delegation::WorkerResult>, BridgeError> {
    match tracker.process(child_session_id, raw_output, send_same_session_repair) {
        delegation::WorkerOutputAction::Structured(result) => Ok(Some(result)),
        delegation::WorkerOutputAction::AwaitingRepair { reason } => {
            store::event(
                db,
                "delegation",
                "worker.result.repair_requested",
                child_session_id,
                &reason,
            )?;
            Ok(None)
        }
        delegation::WorkerOutputAction::Unstructured { raw: _, reason } => {
            store::event(
                db,
                "delegation",
                "worker.result.unstructured",
                child_session_id,
                &reason,
            )?;
            Ok(Some(delegation::WorkerResult {
                schema_version: delegation::SCHEMA_VERSION,
                status: delegation::WorkerResultStatus::Failed,
                summary: format!("Unstructured worker result after repair failure: {reason}"),
                files_changed: vec![],
                tests: vec![],
                decisions: vec![],
                risks: vec!["The raw worker response was excluded from parent context".into()],
                remaining_work: vec!["Review the worker transcript manually".into()],
                suggested_next_action: delegation::SuggestedNextAction::Finish,
                suggested_role: None,
                suggested_task: None,
            }))
        }
    }
}

fn settle_worker_after_result(
    app: &AppHandle,
    child_session_id: &str,
    result: &delegation::WorkerResult,
) -> Result<bool, BridgeError> {
    let state = app.state::<AppState>();
    let current = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1",
            params![child_session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    if current.as_deref() == Some("failed")
        && matches!(
            result.status,
            delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked
        )
    {
        session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            child_session_id,
            worker_lifecycle::WorkerLifecycleState::Completed,
            Some("terminal_failure_reported"),
        )?;
        return Ok(true);
    }
    if current.as_deref() != Some("working") {
        return Ok(true);
    }
    if result.is_retryable() {
        let retry_count = store::worker_runtime(&state.db.lock().unwrap(), child_session_id)?
            .map(|runtime| runtime.retry_count)
            .unwrap_or(1);
        let can_retry_hot = worker_pool::should_retry(
            result,
            retry_count,
            state
                .adapters
                .lock()
                .unwrap()
                .contains_key(child_session_id),
        );
        if can_retry_hot {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Failed,
                Some("typed_failure"),
            )?;
            state.db.lock().unwrap().execute(
                "UPDATE worker_runtime SET retry_count=1,updated_at=?2 WHERE session_id=?1",
                params![child_session_id, Utc::now().to_rfc3339()],
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Resuming,
                Some("automatic_retry"),
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("same_process_retry"),
            )?;
            let sent = state.adapters.lock().unwrap().get(child_session_id).is_some_and(|runtime| {
                runtime.send_turn("Retry the same assigned task once. Address the failure, rerun verification, and return a typed worker result.").is_ok()
            });
            if sent {
                return Ok(false);
            }
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Failed,
                Some("retry_delivery_failed"),
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Completed,
                Some("terminal_failure_reported"),
            )?;
            return Ok(true);
        }
    }
    let (next, warm_until) = match result.status {
        delegation::WorkerResultStatus::Completed
        | delegation::WorkerResultStatus::NeedsDelegation => {
            let attributes: Option<(String, String, String)> = state
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT role,capability_tier,write_mode FROM worker_leases WHERE session_id=?1",
                    params![child_session_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();
            match attributes.map(|(role, tier, mode)| {
                worker_pool::retention_action_for_attributes(&role, &tier, &mode, Utc::now())
            }) {
                Some(worker_pool::RetentionAction::KeepWarmUntil(until)) => (
                    worker_lifecycle::WorkerLifecycleState::Warm,
                    Some(until.to_rfc3339()),
                ),
                _ => (worker_lifecycle::WorkerLifecycleState::Completed, None),
            }
        }
        delegation::WorkerResultStatus::Cancelled => {
            (worker_lifecycle::WorkerLifecycleState::Cancelled, None)
        }
        delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked => {
            (worker_lifecycle::WorkerLifecycleState::Failed, None)
        }
    };
    session_supervisor::SessionSupervisor::transition(
        &state.db.lock().unwrap(),
        child_session_id,
        next,
        Some("typed_result"),
    )?;
    if matches!(
        result.status,
        delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked
    ) {
        session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            child_session_id,
            worker_lifecycle::WorkerLifecycleState::Completed,
            Some("terminal_failure_reported"),
        )?;
    }
    if let Some(warm_until) = warm_until {
        state.db.lock().unwrap().execute(
            "UPDATE worker_runtime SET warm_until=?2 WHERE session_id=?1",
            params![child_session_id, warm_until],
        )?;
    }
    Ok(true)
}

/// If a worker process exits before ever reporting, tell its parent so the
/// parent is not left waiting on a child that will never answer.
fn notify_parent_on_worker_exit(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    verify_read_only_worker(app, child_session_id);
    let already = store::worker_runtime(&state.db.lock().unwrap(), child_session_id)
        .ok()
        .flatten()
        .is_some_and(|runtime| runtime.result_status == "reported");
    if already {
        return;
    }
    let meta: Option<(Option<String>, String)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT parent_session_id,label FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    };
    let Some((Some(_parent), label)) = meta else {
        return;
    };
    let result = delegation::WorkerResult {
        schema_version: delegation::SCHEMA_VERSION,
        status: delegation::WorkerResultStatus::Failed,
        summary: format!("{label} ended without reporting a result"),
        files_changed: vec![],
        tests: vec![],
        decisions: vec![],
        risks: vec!["Worker process exited before a typed result was produced".into()],
        remaining_work: vec!["Retry or delegate the task differently".into()],
        suggested_next_action: delegation::SuggestedNextAction::Finish,
        suggested_role: None,
        suggested_task: None,
    };
    match settle_worker_after_result(app, child_session_id, &result) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = store::event(
                &state.db.lock().unwrap(),
                "supervisor",
                "worker.settle_failed",
                child_session_id,
                &error.to_string(),
            );
            return;
        }
    }
    report_to_parent(app, child_session_id, &result);
}

fn verify_read_only_worker(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    let baseline = state
        .delegations
        .lock()
        .unwrap()
        .read_only_baselines
        .remove(child_session_id);
    let Some(baseline) = baseline else {
        return;
    };
    let db = state.db.lock().unwrap();
    if let Err(error) = worker_guard::verify_and_record(&db, child_session_id, &baseline) {
        let _ = store::event(
            &db,
            "sandbox",
            "worker.read_only_verification_failed",
            child_session_id,
            &error.to_string(),
        );
    }
    drop(db);
    let sandbox = state
        .delegations
        .lock()
        .unwrap()
        .read_only_sandboxes
        .remove(child_session_id);
    if let Some(sandbox) = sandbox {
        let _ = store::event(
            &state.db.lock().unwrap(),
            "sandbox",
            "worker.read_only_isolation_cleaned",
            child_session_id,
            &format!("output_dir={}", sandbox.output_dir.display()),
        );
        sandbox.cleanup();
    }
}

/// Deliver a framed message from a child to its parent session: send it into the
/// parent's live turn stream and drop a marker card into the parent's transcript.
fn report_to_parent(app: &AppHandle, child_session_id: &str, result: &delegation::WorkerResult) {
    let state = app.state::<AppState>();
    let report = {
        let db = state.db.lock().unwrap();
        session_supervisor::SessionSupervisor::record_result(&db, child_session_id, result)
            .ok()
            .flatten()
    };
    let Some(report) = report else {
        return;
    };
    let app = app.clone();
    let child_session_id = child_session_id.to_owned();
    let result = result.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let available_capabilities = live_available_capabilities(&state).await;
        let completion_result = {
            let db = state.db.lock().unwrap();
            completion::create_from_worker_result(
                &db,
                &child_session_id,
                &result,
                &available_capabilities,
            )
        };
        let completion = match completion_result {
            Ok(summary) => summary,
            Err(error) => {
                let db = state.db.lock().unwrap();
                let _ = store::event(
                    &db,
                    "completion",
                    "completion.plan_failed",
                    &child_session_id,
                    &error.to_string(),
                );
                completion::record_gate_error(&db, &child_session_id, &error.to_string())
                    .ok()
                    .flatten()
            }
        };
        {
            let db = state.db.lock().unwrap();
            let _ = completion::reconcile_parent_readiness(&db, &report.parent_session_id);
        }
        let routing_notice = serde_json::json!({
        "type": "bridge-worker-evidence",
        "evidenceId": report.evidence_id,
        "status": result.status.as_str(),
        "summary": result.summary,
        "completion": completion,
        "instruction": "Treat this as routing metadata. The referenced SQLite worker.result entry is canonical. If completion is verifying or changes_requested, route the next required verification sequentially; do not claim the task is done."
    })
    .to_string();
        let delivered = match state
            .adapters
            .lock()
            .unwrap()
            .get(&report.parent_session_id)
        {
            Some(runtime) => runtime.send_turn(&routing_notice).is_ok(),
            None => false,
        };
        {
            let db = state.db.lock().unwrap();
            let result_event = agent::NormalizedEvent {
                kind: "delegation.result".into(),
                item_id: Some(format!("result-{}", Uuid::new_v4())),
                role: Some("system".into()),
                status: Some("completed".into()),
                title: Some("Worker result".into()),
                text: Some(result.summary.clone()),
                data: serde_json::json!({"childSessionId": child_session_id, "evidenceId": report.evidence_id, "delivered": delivered, "status": result.status.as_str()}),
            };
            if let Ok(stored) = store::session_event(
                &db,
                &report.parent_session_id,
                &result_event,
                &serde_json::json!({"delegation": true}),
            ) {
                let _ = app.emit("agent-event", stored);
            }
            if delivered {
                let _ = db.execute(
                    "UPDATE sessions SET status='working' WHERE id=?1 AND ended_at IS NULL",
                    params![report.parent_session_id],
                );
            }
        }
        let workspace_id = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT workspace_id FROM sessions WHERE id=?1",
                params![child_session_id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        let _ = app.emit("state-changed", ());
        if let Some(workspace_id) = workspace_id {
            dispatch_next_queued_worker(&app, &workspace_id);
        }
    });
}

fn dispatch_next_queued_worker(app: &AppHandle, workspace_id: &str) {
    let state = app.state::<AppState>();
    let queued =
        worker_pool::WorkerPool::claim_next_queued(&state.db.lock().unwrap(), workspace_id)
            .ok()
            .flatten();
    let Some(queued) = queued else {
        return;
    };
    let directive: delegation::DelegationRequest = match serde_json::from_value(queued.request) {
        Ok(directive) => directive,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::update_worker_queue(&db, &queued.id, "rejected", None);
            let _ = store::event(
                &db,
                "worker-pool",
                "worker.queue.invalid",
                &queued.id,
                &error.to_string(),
            );
            return;
        }
    };
    let launched = launch_worker(
        app,
        &queued.parent_session_id,
        &queued.turn_id,
        &directive,
        false,
    );
    let db = state.db.lock().unwrap();
    let _ = if let Some(session_id) = launched.as_deref() {
        store::update_worker_queue(&db, &queued.id, "dispatched", Some(session_id))
    } else {
        store::update_worker_queue(&db, &queued.id, "rejected", None)
    };
}

fn maintain_worker_pool(app: &AppHandle) {
    let state = app.state::<AppState>();
    let expired = worker_pool::WorkerPool::warm_workers_due(&state.db.lock().unwrap(), Utc::now())
        .unwrap_or_default();
    for session_id in expired {
        let prompt = {
            let db = state.db.lock().unwrap();
            let tokens =
                compaction_controller::active_token_estimate(&db, &session_id).unwrap_or_default();
            let prompt = compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeSuspend,
                tokens,
            )
            .ok()
            .flatten();
            if prompt.is_some() {
                let _ = session_supervisor::SessionSupervisor::transition(
                    &db,
                    &session_id,
                    worker_lifecycle::WorkerLifecycleState::Checkpointing,
                    Some("warm_idle_timeout"),
                );
                let _ = db.execute(
                    "UPDATE worker_runtime SET warm_until=NULL WHERE session_id=?1",
                    params![session_id],
                );
            }
            prompt
        };
        if let Some(prompt) = prompt {
            if let Err(error) = send_internal_checkpoint_turn(app, &session_id, &prompt) {
                let _ = compaction_controller::CompactionController::record_failure(
                    &state.db.lock().unwrap(),
                    &session_id,
                    &format!("checkpoint turn could not start: {error}"),
                    0,
                );
                finish_worker_checkpoint(app, &session_id, adapters::ShutdownReason::Failed);
            }
        }
    }
    let timed_out = {
        let db = state.db.lock().unwrap();
        let mut statement = match db.prepare(
            "SELECT session_id FROM worker_runtime WHERE lifecycle_state='checkpointing' ORDER BY session_id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let result = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows
                .filter_map(Result::ok)
                .filter(|session_id| {
                    compaction_controller::CompactionController::pending(&db, session_id)
                        .ok()
                        .flatten()
                        .and_then(|pending| {
                            chrono::DateTime::parse_from_rfc3339(&pending.requested_at).ok()
                        })
                        .is_some_and(|requested| {
                            Utc::now()
                                .signed_duration_since(requested.with_timezone(&Utc))
                                .num_seconds()
                                >= compaction_controller::CHECKPOINT_TIMEOUT_SECONDS
                        })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        result
    };
    for session_id in timed_out {
        let _ = compaction_controller::CompactionController::record_failure(
            &state.db.lock().unwrap(),
            &session_id,
            "checkpoint turn timed out; suspension continued",
            1,
        );
        finish_worker_checkpoint(app, &session_id, adapters::ShutdownReason::Failed);
    }
    let shutdown_timeouts = {
        let db = state.db.lock().unwrap();
        let mut statement = match db.prepare(
            "SELECT id FROM sessions WHERE status='checkpointing' AND parent_session_id IS NULL ORDER BY id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let result = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows
                .filter_map(Result::ok)
                .filter(|session_id| {
                    compaction_controller::CompactionController::pending(&db, session_id)
                        .ok()
                        .flatten()
                        .filter(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        })
                        .and_then(|pending| {
                            chrono::DateTime::parse_from_rfc3339(&pending.requested_at).ok()
                        })
                        .is_some_and(|requested| {
                            Utc::now()
                                .signed_duration_since(requested.with_timezone(&Utc))
                                .num_seconds()
                                >= compaction_controller::CHECKPOINT_TIMEOUT_SECONDS
                        })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        result
    };
    for session_id in shutdown_timeouts {
        let _ = compaction_controller::CompactionController::record_failure(
            &state.db.lock().unwrap(),
            &session_id,
            "shutdown checkpoint timed out; termination continued",
            1,
        );
        finish_orchestrator_shutdown(app, &session_id, adapters::ShutdownReason::UserStopped);
    }
    let workspaces = {
        let db = state.db.lock().unwrap();
        if worker_pool::WorkerPool::maintain_queue(&db, Utc::now()).is_err() {
            return;
        }
        let mut statement = match db.prepare(
            "SELECT workspace_id FROM worker_queue WHERE queue_status='queued' GROUP BY workspace_id ORDER BY MIN(sequence),workspace_id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let workspaces = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows.filter_map(Result::ok).collect::<Vec<_>>(),
            Err(_) => return,
        };
        workspaces
    };
    for workspace_id in workspaces {
        dispatch_next_queued_worker(app, &workspace_id);
    }
}

fn start_worker_maintenance(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        maintain_worker_pool(&app);
    });
}

fn start_learning_maintenance(app: AppHandle) {
    thread::spawn(move || loop {
        let ran = {
            let state = app.state::<AppState>();
            let database_path = state.database_path.clone();
            let result = learning_job::run_due_database(&database_path, Utc::now())
                .ok()
                .flatten();
            result
        };
        if let Some(run) = ran {
            let _ = app.emit("learning-job-changed", run);
        }
        thread::sleep(Duration::from_secs(60));
    });
}

const HISTORY_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15 * 60);

fn start_history_snapshot_maintenance(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(HISTORY_SNAPSHOT_INTERVAL);
        let state = app.state::<AppState>();
        if let Ok(db) = Connection::open_with_flags(
            &state.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            let _ = store::export_history_snapshot(&db, &state.snapshot_dir);
        }
    });
}

#[tauri::command]
async fn open_terminal(
    workspace_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    let runtime_id = format!("terminal:{workspace_id}");
    if state.runtimes.lock().unwrap().contains_key(&runtime_id) {
        return Ok(());
    }
    let db = state.db.lock().unwrap();
    let path: String = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get(0),
    )?;
    drop(db);
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let mut command = CommandBuilder::new("zsh");
    command.args(["-l"]);
    command.cwd(&path);
    command.env("TERM", "xterm-256color");
    command.env("BRIDGE_WORKSPACE_ID", &workspace_id);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    state.runtimes.lock().unwrap().insert(
        runtime_id.clone(),
        RuntimeSession {
            writer,
            master: pair.master,
            child,
        },
    );
    let app_reader = app.clone();
    let workspace_reader = workspace_id.clone();
    let runtime_reader = runtime_id.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = app_reader.emit(
                        "session-output",
                        TerminalChunk {
                            session_id: workspace_reader.clone(),
                            data,
                        },
                    );
                }
            }
        }
        let state = app_reader.state::<AppState>();
        state.runtimes.lock().unwrap().remove(&runtime_reader);
    });
    Ok(())
}

#[tauri::command]
async fn write_terminal(
    workspace_id: String,
    data: String,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    let mut sessions = state.runtimes.lock().unwrap();
    let runtime = sessions
        .get_mut(&format!("terminal:{workspace_id}"))
        .ok_or_else(|| BridgeError::Invalid("Workspace terminal is not open".into()))?;
    runtime.writer.write_all(data.as_bytes())?;
    runtime.writer.flush()?;
    Ok(())
}

#[tauri::command]
async fn prepare_turn(
    session_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<secret_interception::SanitizedTurn, BridgeError> {
    if text.trim().is_empty() {
        return Err(BridgeError::Invalid("Message cannot be empty".into()));
    }
    let exists: bool = state.db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
        params![session_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(BridgeError::Invalid("Chat session does not exist".into()));
    }
    let intercepted = secret_interception::intercept(&text);
    state
        .credential_broker
        .register(&session_id, intercepted.captured);
    Ok(intercepted.sanitized)
}

fn deliver_sanitized_turn(
    runtime: &dyn adapters::AdapterRuntime,
    text: &str,
    application_context: Option<&str>,
) -> Result<(), BridgeError> {
    match application_context {
        Some(context) => runtime.send_turn_with_context(text, context),
        None => runtime.send_turn(text),
    }
}

fn persist_submitted_user_turn(
    db: &Connection,
    session_id: &str,
    adapter_id: &str,
    display_text: &str,
) -> Result<Option<AgentEvent>, BridgeError> {
    if adapter_id == "codex" {
        return Ok(None);
    }
    let user_event = agent::NormalizedEvent {
        kind: "message.completed".into(),
        item_id: Some(format!("user-{}", Uuid::new_v4())),
        role: Some("user".into()),
        status: Some("completed".into()),
        title: None,
        text: Some(display_text.into()),
        data: serde_json::json!({}),
    };
    store::session_event(
        db,
        session_id,
        &user_event,
        &serde_json::json!({"adapter": adapter_id}),
    )
    .map(Some)
}

#[tauri::command]
async fn send_turn(
    session_id: String,
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    if text.trim().is_empty() {
        return Err(BridgeError::Invalid("Message cannot be empty".into()));
    }
    if store::worker_runtime(&state.db.lock().unwrap(), &session_id)?.is_some() {
        return Err(BridgeError::Invalid(
            "Worker turns are scheduled through the policy-controlled worker pool".into(),
        ));
    }

    // Sanitize the user-authored text before slash expansion, adapter transport,
    // optimistic UI projection, or durable conversation history can observe it.
    let intercepted = secret_interception::intercept(&text);
    state
        .credential_broker
        .register(&session_id, intercepted.captured);
    let sanitized_input = intercepted.sanitized;
    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    let session_harness: String = state.db.lock().unwrap().query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;

    let outbound = match slash::dispatch(&sanitized_input.text, &session_harness, &available) {
        slash::SlashDispatch::Usage => {
            refresh_account_usage(app.clone(), state.clone()).await?;
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                "Refreshed account usage. Check the meter in the title bar.",
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Compact { .. } => {
            compact_session(session_id.clone(), app.clone(), state.clone()).await?;
            return Ok(());
        }
        slash::SlashDispatch::Clear => {
            state.credential_broker.clear_session(&session_id);
            if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
                runtime.stop(adapters::ShutdownReason::UserStopped);
            }
            let db = state.db.lock().unwrap();
            session_supervisor::SessionSupervisor::clear_adapter_process(&db, &session_id)?;
            db.execute(
                "UPDATE sessions SET provider_session_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1",
                params![session_id],
            )?;
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                "Cleared this chat’s provider session. Send a message to start fresh.",
            )?;
            let _ = app.emit("state-changed", ());
            return Ok(());
        }
        slash::SlashDispatch::Unsupported { name, harness } => {
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                &format!("`/{name}` is a {harness} terminal UI command and isn’t available inside Bridge yet."),
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Expand { text } => text,
        slash::SlashDispatch::Forward { text } => text,
    };

    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    let credential_context = state.credential_broker.turn_context(&session_id, &outbound);
    if let Err(error) =
        deliver_sanitized_turn(runtime.as_ref(), &outbound, credential_context.as_deref())
    {
        drop(adapters);
        record_recoverable_adapter_failure(&state, &session_id, &error)?;
        return Err(error);
    }
    drop(adapters);
    let db = state.db.lock().unwrap();
    let adapter_id: String = db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    // Claude stream-json does not reliably echo the submitted user turn; persist it locally.
    // Prefer the original slash text for the transcript when we expanded a skill/prompt.
    let display_text = if outbound != sanitized_input.text {
        sanitized_input.text
    } else {
        outbound.clone()
    };
    if let Some(event) = persist_submitted_user_turn(&db, &session_id, &adapter_id, &display_text)?
    {
        let _ = app.emit("agent-event", event);
    }
    let _ = db.execute(
        "UPDATE sessions SET status='working' WHERE id=?1",
        params![session_id],
    );
    let _ = app.emit("state-changed", ());
    Ok(())
}

fn record_recoverable_adapter_failure(
    state: &State<'_, AppState>,
    session_id: &str,
    error: &BridgeError,
) -> Result<(), BridgeError> {
    let db = state.db.lock().unwrap();
    db.execute(
        "UPDATE sessions SET status='failed',active_turn_id=NULL,ended_at=?2 WHERE id=?1",
        params![session_id, Utc::now().to_rfc3339()],
    )?;
    store::event(
        &db,
        "adapter",
        "adapter.request_failed",
        session_id,
        &error.to_string(),
    )?;
    Ok(())
}

fn emit_local_assistant(
    app: &AppHandle,
    state: &State<'_, AppState>,
    session_id: &str,
    adapter_id: &str,
    text: &str,
) -> Result<(), BridgeError> {
    let db = state.db.lock().unwrap();
    let event = store::session_event(
        &db,
        session_id,
        &agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some(format!("bridge-{}", Uuid::new_v4())),
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some(text.into()),
            data: serde_json::json!({ "bridgeLocal": true }),
        },
        &serde_json::json!({ "adapter": adapter_id }),
    )?;
    let _ = app.emit("agent-event", event);
    Ok(())
}

#[tauri::command]
async fn compact_session(
    session_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    let prompt = {
        let db = state.db.lock().unwrap();
        let status: String = db.query_row(
            "SELECT status FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if matches!(status.as_str(), "working" | "waiting" | "checkpointing") {
            return Err(BridgeError::Invalid(
                "Compaction waits until the active tool, approval, or turn finishes".into(),
            ));
        }
        let branch = session_forest::SessionForest::new(&db)
            .active_branch(&session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let meaningful = branch.iter().any(|entry| {
            matches!(
                entry.kind.as_str(),
                "user.message" | "assistant.message" | "worker.result" | "tool.completed"
            )
        });
        compaction_controller::decide(&compaction_controller::TriggerState {
            reason: compaction_controller::CompactionReason::Manual,
            context_percent: None,
            projected_tokens_with_reserve: None,
            context_window_tokens: None,
            has_valid_typed_result: false,
            one_shot_worker: false,
            tool_call_active: false,
            approval_active: false,
            has_meaningful_new_work: meaningful,
            wall_clock_only: false,
        })
        .map_err(|reason| BridgeError::Invalid(format!("Compaction suppressed: {reason:?}")))?;
        let tokens = compaction_controller::active_token_estimate(&db, &session_id)?;
        compaction_controller::CompactionController::begin(
            &db,
            &session_id,
            compaction_controller::CompactionReason::Manual,
            tokens,
        )?
        .ok_or_else(|| BridgeError::Invalid("Compaction is already pending".into()))?
    };
    send_internal_checkpoint_turn(&app, &session_id, &prompt)
}

#[tauri::command]
async fn interrupt_turn(session_id: String, state: State<'_, AppState>) -> Result<(), BridgeError> {
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.interrupt()
}

/// Refresh subscription usage for every provider, independent of which session
/// is on screen. Claude is queried out-of-band via its headless `/usage`
/// command; Codex is asked on a live session and answers on its event stream.
/// Both results are broadcast on the `account-usage` channel.
#[tauri::command]
async fn refresh_account_usage(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    // Claude: a global, read-only account query — no running session required.
    if binary::resolve("claude").is_some() {
        let app = app.clone();
        thread::spawn(move || {
            let cwd = std::env::temp_dir();
            let cwd = cwd.to_string_lossy();
            if let Some(data) = claude_adapter::read_usage_snapshot(cwd.as_ref()) {
                if let Some(rate_limits) = data.get("rateLimits") {
                    emit_account_usage(&app, "claude", rate_limits.clone());
                }
            }
        });
    }
    // Codex: rate limits are account-wide, so a single running session answers
    // for the whole account. Its reply routes back through handle_agent_value.
    let codex_sessions: Vec<String> = {
        let db = state.db.lock().unwrap();
        let mut statement =
            db.prepare("SELECT id FROM sessions WHERE harness='codex' AND ended_at IS NULL")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        ids
    };
    let adapters = state.adapters.lock().unwrap();
    for session_id in codex_sessions {
        if let Some(runtime) = adapters.get(&session_id) {
            let _ = runtime.read_usage();
            break;
        }
    }
    Ok(())
}

/// Rate-limit snapshot carried by a Codex account frame, if this is one.
fn codex_rate_limits_from_frame(value: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(rate_limits) = value.pointer("/result/rateLimits") {
        return Some(rate_limits.clone());
    }
    if value.get("method").and_then(|m| m.as_str()) == Some("account/rateLimits/updated") {
        if let Some(rate_limits) = value.pointer("/params/rateLimits") {
            return Some(rate_limits.clone());
        }
    }
    None
}

/// Broadcast a provider's subscription usage to the UI's ambient meter.
fn emit_account_usage(app: &AppHandle, provider: &str, rate_limits: serde_json::Value) {
    let _ = app.emit(
        "account-usage",
        serde_json::json!({ "provider": provider, "rateLimits": rate_limits }),
    );
}

#[tauri::command]
async fn resolve_approval(
    session_id: String,
    event_id: i64,
    decision: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    if !matches!(
        decision.as_str(),
        "accept" | "acceptForSession" | "decline" | "cancel"
    ) {
        return Err(BridgeError::Invalid("Unsupported approval decision".into()));
    }
    let db = state.db.lock().unwrap();
    let (data, adapter_id): (String, String) = db.query_row(
        "SELECT e.payload,s.harness FROM session_entries e
         JOIN sessions s ON s.id=e.session_id
         WHERE e.session_id=?1 AND e.sequence=?2 AND e.kind='approval.requested'",
        params![session_id, event_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let data: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| BridgeError::Invalid(format!("Approval metadata is invalid: {e}")))?;
    if data.get("approvalType").and_then(serde_json::Value::as_str) == Some("delegation_path_scope")
    {
        let launch =
            resolve_policy_delegation_approval(&db, &session_id, event_id, &decision, &data)?;
        drop(db);
        if let Some((turn_id, request)) = launch {
            match launch_worker_outcome(&app, &session_id, &turn_id, &request, true) {
                WorkerLaunchOutcome::Launched(_) | WorkerLaunchOutcome::Queued => {}
                WorkerLaunchOutcome::Failed => {
                    let db = state.db.lock().unwrap();
                    record_approved_launch_failure(&db, &session_id, &turn_id, &request)?;
                    let _ = app.emit("state-changed", ());
                    return Err(BridgeError::Invalid(
                        "Write scope was approved, but the worker could not launch; the delegation may be retried for this turn".into(),
                    ));
                }
            }
        }
        let _ = app.emit("state-changed", ());
        return Ok(());
    }
    let request_id = data
        .get("requestId")
        .cloned()
        .ok_or_else(|| BridgeError::Invalid("Approval has no adapter request id".into()))?;
    let is_worker = store::worker_runtime(&db, &session_id)?.is_some();
    if is_worker {
        session_supervisor::SessionSupervisor::transition(
            &db,
            &session_id,
            worker_lifecycle::WorkerLifecycleState::Working,
            Some("approval_resolved"),
        )?;
    }
    drop(db);
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    if let Err(error) = runtime.respond(request_id, &decision) {
        drop(adapters);
        if is_worker {
            let _ = session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Waiting,
                Some("approval_delivery_failed"),
            );
        }
        return Err(error);
    }
    drop(adapters);
    let mut normalized = agent::NormalizedEvent {
        kind: "approval.resolved".into(),
        item_id: None,
        role: None,
        status: Some(decision.clone()),
        title: Some("Approval resolved".into()),
        text: None,
        data: serde_json::json!({"requestEventId":event_id,"decision":decision}),
    };
    normalized.item_id = data
        .get("itemId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let db = state.db.lock().unwrap();
    let event = store::session_event(
        &db,
        &session_id,
        &normalized,
        &serde_json::json!({"adapter":adapter_id}),
    )?;
    if !is_worker {
        db.execute(
            "UPDATE sessions SET status='working' WHERE id=?1",
            params![session_id],
        )?;
    }
    db.execute(
        "UPDATE workspaces SET status=CASE
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='working') THEN 'working'
            ELSE 'ready' END
         WHERE id=(SELECT workspace_id FROM sessions WHERE id=?1)",
        params![session_id],
    )?;
    let _ = app.emit("agent-event", event);
    let _ = app.emit("state-changed", ());
    Ok(())
}

fn record_approved_launch_failure(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
    request: &delegation::DelegationRequest,
) -> Result<(), BridgeError> {
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::DelegationRejected,
            serde_json::json!({
                "requestId": turn_id,
                "turnId": turn_id,
                "status": "failed",
                "reason": "approved_launch_failed",
                "title": "Approved delegation could not launch",
                "text": "The approved same-turn scope remains available if the delegation is retried.",
                "request": request,
            }),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "policy",
        "policy.approved_launch_failed",
        session_id,
        turn_id,
    )?;
    Ok(())
}

fn resolve_policy_delegation_approval(
    db: &Connection,
    session_id: &str,
    event_id: i64,
    decision: &str,
    payload: &serde_json::Value,
) -> Result<Option<(String, delegation::DelegationRequest)>, BridgeError> {
    if decision == "acceptForSession" {
        return Err(BridgeError::Invalid(
            "Delegation path scope can only be approved for this turn".into(),
        ));
    }
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let approval_id = payload
        .get("approvalId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BridgeError::Invalid("Policy approval has no approval id".into()))?;
    let request_entry = branch
        .iter()
        .find(|entry| {
            entry.sequence == event_id
                && entry.kind == "approval.requested"
                && entry.payload["approvalId"] == approval_id
        })
        .ok_or_else(|| {
            BridgeError::Invalid("Approval is no longer on the active conversation branch".into())
        })?;
    if branch.iter().any(|entry| {
        entry.kind == "approval.resolved" && entry.payload["approvalId"] == approval_id
    }) {
        return Err(BridgeError::Invalid("Approval was already resolved".into()));
    }
    let request: delegation::DelegationRequest =
        serde_json::from_value(payload.get("request").cloned().ok_or_else(|| {
            BridgeError::Invalid("Policy approval has no delegation request".into())
        })?)
        .map_err(|error| {
            BridgeError::Invalid(format!("Policy approval request is invalid: {error}"))
        })?;
    request.validate().map_err(BridgeError::Invalid)?;
    let turn_id = payload
        .get("turnId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| BridgeError::Invalid("Policy approval has no parent turn".into()))?
        .to_owned();
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::ApprovalResolved,
            serde_json::json!({
                "approvalId": approval_id,
                "approvalType": "delegation_path_scope",
                "requestEventId": event_id,
                "requestEntryId": request_entry.id,
                "turnId": turn_id,
                "decision": decision,
                "approvedOwnedPaths": request.owned_paths,
            }),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    db.execute(
        "UPDATE sessions SET status='working' WHERE id=?1 AND status='waiting'",
        params![session_id],
    )?;
    db.execute(
        "UPDATE workspaces SET status=CASE
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='working') THEN 'working'
            ELSE 'ready' END
         WHERE id=(SELECT workspace_id FROM sessions WHERE id=?1)",
        params![session_id],
    )?;
    Ok(matches!(decision, "accept" | "acceptForSession").then_some((turn_id, request)))
}

#[tauri::command]
async fn resize_terminal(
    workspace_id: String,
    rows: u16,
    cols: u16,
    state: State<'_, AppState>,
) -> Result<(), BridgeError> {
    if let Some(runtime) = state
        .runtimes
        .lock()
        .unwrap()
        .get_mut(&format!("terminal:{workspace_id}"))
    {
        runtime
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| BridgeError::Pty(e.to_string()))?
    }
    Ok(())
}
#[tauri::command]
async fn stop_session(
    session_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let is_worker = state.db.lock().unwrap().query_row(
        "SELECT parent_session_id IS NOT NULL FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if is_worker {
        if let Some(runtime) = state.adapters.lock().unwrap().get(&session_id) {
            let _ = runtime.interrupt();
        }
        let result = delegation::WorkerResult {
            schema_version: delegation::SCHEMA_VERSION,
            status: delegation::WorkerResultStatus::Cancelled,
            summary: "Worker cancelled by user".into(),
            files_changed: vec![],
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec!["Cancelled work was not completed".into()],
            suggested_next_action: delegation::SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        if !settle_worker_after_result(&app, &session_id, &result)? {
            return Err(BridgeError::Invalid(
                "cancelled worker cannot be retried".into(),
            ));
        }
        report_to_parent(&app, &session_id, &result);
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::UserCancelled);
        }
        let db = state.db.lock().unwrap();
        session_supervisor::SessionSupervisor::clear_adapter_process(&db, &session_id)?;
        let workspace_id: String = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('starting','working','waiting','warm','checkpointing','resuming','restored')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id])?;
        let _ = app.emit("state-changed", ());
        return store::state(&db);
    }
    let has_process = state.adapters.lock().unwrap().contains_key(&session_id);
    let shutdown_prompt = {
        let db = state.db.lock().unwrap();
        let status: String = db.query_row(
            "SELECT status FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let branch = session_forest::SessionForest::new(&db)
            .active_branch(&session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let meaningful = branch.iter().any(|entry| {
            matches!(
                entry.kind.as_str(),
                "user.message" | "assistant.message" | "worker.result" | "tool.completed"
            )
        });
        if has_process
            && meaningful
            && !matches!(status.as_str(), "working" | "waiting" | "checkpointing")
        {
            let tokens = compaction_controller::active_token_estimate(&db, &session_id)?;
            compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeShutdown,
                tokens,
            )?
        } else {
            None
        }
    };
    if let Some(prompt) = shutdown_prompt {
        match send_internal_checkpoint_turn(&app, &session_id, &prompt) {
            Ok(()) => {
                let db = state.db.lock().unwrap();
                db.execute(
                    "UPDATE sessions SET status='checkpointing' WHERE id=?1",
                    params![session_id],
                )?;
                let _ = app.emit("state-changed", ());
                return store::state(&db);
            }
            Err(error) => {
                let _ = compaction_controller::CompactionController::record_failure(
                    &state.db.lock().unwrap(),
                    &session_id,
                    &format!("shutdown checkpoint could not start: {error}"),
                    0,
                );
            }
        }
    }
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
        runtime.stop(adapters::ShutdownReason::UserStopped);
    }
    record_shutdown_reason(
        &state.db.lock().unwrap(),
        &session_id,
        adapters::ShutdownReason::UserStopped,
    )?;
    if let Some(mut runtime) = state.runtimes.lock().unwrap().remove(&session_id) {
        runtime
            .child
            .kill()
            .map_err(|e| BridgeError::Pty(e.to_string()))?;
        let _ = runtime.child.wait();
    }
    let db = state.db.lock().unwrap();
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    db.execute(
        "UPDATE sessions SET status='stopped',ended_at=?2 WHERE id=?1",
        params![session_id, Utc::now().to_rfc3339()],
    )?;
    db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'stopped' END WHERE id=?1",params![workspace_id])?;
    store::event(
        &db,
        "supervisor",
        "session.stopped",
        &session_id,
        "Session stopped by user",
    )?;
    let _ = app.emit("state-changed", ());
    store::state(&db)
}

fn record_shutdown_reason(
    db: &Connection,
    session_id: &str,
    reason: adapters::ShutdownReason,
) -> Result<(), BridgeError> {
    session_supervisor::SessionSupervisor::clear_adapter_process(db, session_id)?;
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::SessionStatus,
            serde_json::json!({"status":"stopped","reason":reason.as_str()}),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "adapter",
        "session.shutdown",
        session_id,
        reason.as_str(),
    )
}
#[tauri::command]
async fn refresh_workspace(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    // Resolve the path under the lock, but leave Git entirely outside it so a
    // slow status scan cannot delay message submission or streaming writes.
    let path: String = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT path FROM workspaces WHERE id=?1",
            params![workspace_id],
            |r| r.get(0),
        )?
    };
    let (dirty, adds, dels) =
        tauri::async_runtime::spawn_blocking(move || git::stats(Path::new(&path)))
            .await
            .map_err(|error| {
                BridgeError::Invalid(format!("Workspace refresh task failed: {error}"))
            })??;
    let db = state.db.lock().unwrap();
    db.execute(
        "UPDATE workspaces SET dirty_files=?2,additions=?3,deletions=?4 WHERE id=?1",
        params![workspace_id, dirty, adds, dels],
    )?;
    store::state(&db)
}

#[tauri::command]
async fn archive_workspace(
    workspace_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BridgeState, BridgeError> {
    let db = state.db.lock().unwrap();
    let (path, repo): (String, String) = db.query_row(
        "SELECT w.path,p.path FROM workspaces w JOIN projects p ON p.id=w.project_id WHERE w.id=?1",
        params![workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let active: i64 = db.query_row(
        "SELECT COUNT(*) FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting','ready') AND ended_at IS NULL",
        params![workspace_id],
        |r| r.get(0),
    )?;
    if active > 0 {
        return Err(BridgeError::Invalid(
            "Stop every running session before archiving this workspace".into(),
        ));
    }
    let (dirty, _, _) = git::stats(Path::new(&path))?;
    if dirty > 0 {
        return Err(BridgeError::Invalid(format!(
            "Workspace has {dirty} uncommitted file(s). Commit or discard them before archiving"
        )));
    }
    archive_workspace_records(&db, &workspace_id, || {
        git::remove_worktree(Path::new(&repo), Path::new(&path))
    })?;
    store::event(
        &db,
        "supervisor",
        "workspace.archived",
        &workspace_id,
        "Archived clean workspace; branch preserved",
    )?;
    let _ = app.emit("state-changed", ());
    store::state(&db)
}

fn archive_workspace_records(
    db: &Connection,
    workspace_id: &str,
    remove_worktree: impl FnOnce() -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM task_knowledge WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM worker_leases WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM session_heads WHERE session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM session_entries WHERE session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM usage_ledger WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM sessions WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute("DELETE FROM workspaces WHERE id=?1", params![workspace_id])?;
    remove_worktree()?;
    transaction.commit()?;
    Ok(())
}

fn start_health_server(
    database: PathBuf,
    adapters: Vec<AdapterDescriptor>,
    credential_broker: Arc<credential_broker::CredentialBroker>,
) {
    thread::spawn(move || {
        let Ok(server) = tiny_http::Server::http("127.0.0.1:4317") else {
            return;
        };
        for request in server.incoming_requests() {
            if request.url() == "/health" {
                let body = serde_json::json!({
                    "ok": true,
                    "version": env!("CARGO_PKG_VERSION"),
                    "database": database,
                    "adapters": adapters,
                    "harnesses": {
                        "claude": binary::resolve("claude").is_some(),
                        "codex": binary::resolve("codex").is_some(),
                        "opencode": binary::resolve("opencode").is_some(),
                        "shell": true
                    }
                })
                .to_string();
                let mut response = tiny_http::Response::from_string(body).with_status_code(200);
                if let Ok(header) =
                    tiny_http::Header::from_bytes("Content-Type", "application/json")
                {
                    response.add_header(header);
                }
                let _ = request.respond(response);
                continue;
            }
            if let Some(route) = request.url().strip_prefix(credential_broker::PROXY_PREFIX) {
                // Handle each proxy call on its own thread so a slow (or
                // deliberately slow-drip) upstream request cannot block /health
                // liveness or serialize other agents behind the single accept loop.
                let route = route.to_owned();
                let method = request.method().as_str().to_owned();
                let token = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv(credential_broker::PROXY_AUTH_HEADER))
                    .map(|header| header.value.as_str().to_owned())
                    .unwrap_or_default();
                let headers: Vec<(String, String)> = request
                    .headers()
                    .iter()
                    .map(|header| (header.field.to_string(), header.value.as_str().to_owned()))
                    .collect();
                let broker = credential_broker.clone();
                thread::spawn(move || {
                    let mut request = request;
                    let mut parts = route.splitn(3, '/');
                    let session_id = parts.next().unwrap_or_default().to_owned();
                    let reference = parts.next().unwrap_or_default().to_owned();
                    let path_and_query = format!("/{}", parts.next().unwrap_or_default());
                    let mut body = Vec::new();
                    let result = request
                        .as_reader()
                        .take((credential_broker::MAX_BODY_BYTES + 1) as u64)
                        .read_to_end(&mut body)
                        .map_err(BridgeError::Io)
                        .and_then(|_| {
                            broker.proxy(credential_broker::ProxyRequest {
                                session_id,
                                reference,
                                method,
                                path_and_query,
                                headers,
                                token,
                                body,
                            })
                        });
                    let response = match result {
                        Ok(proxied) => {
                            let mut response = tiny_http::Response::from_data(proxied.body)
                                .with_status_code(proxied.status);
                            if let Some(header) = proxied.content_type.and_then(|value| {
                                tiny_http::Header::from_bytes("Content-Type", value).ok()
                            }) {
                                response.add_header(header);
                            }
                            response
                        }
                        Err(error) => {
                            let body = serde_json::json!({"ok": false, "error": error.to_string()})
                                .to_string();
                            tiny_http::Response::from_string(body).with_status_code(400)
                        }
                    };
                    let _ = request.respond(response);
                });
                continue;
            }
            let body = serde_json::json!({"ok": false, "error": "not found"}).to_string();
            let mut response = tiny_http::Response::from_string(body).with_status_code(404);
            if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
                response.add_header(header);
            }
            let _ = request.respond(response);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let db_path = data.join("bridge.db");
            let telemetry_db_path = data.join("bridge-telemetry.db");
            let snapshot_dir = data.join("history-snapshots");
            let connection =
                store::open(&db_path).map_err(|e| Box::<dyn std::error::Error>::from(e))?;
            let telemetry_connection = store::open_telemetry(&telemetry_db_path)
                .map_err(|e| Box::<dyn std::error::Error>::from(e))?;
            session_supervisor::SessionSupervisor::recover_tracked_adapter_processes(&connection)
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            session_supervisor::SessionSupervisor::recover_orphaned_workers(&connection)
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            session_supervisor::SessionSupervisor::reconcile_workspace_statuses(&connection)
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let _ = store::export_history_snapshot(&connection, &snapshot_dir);
            let opencode_config = agent_config::state(&connection)?
                .harnesses
                .into_iter()
                .find(|config| config.id == "opencode");
            let opencode_settings = agent_config::opencode_settings(opencode_config.as_ref())?;
            let discovery_handle = app.handle().clone();
            let adapter_registry = Arc::new(
                adapters::AdapterRegistry::built_in_with_opencode_notify(
                    opencode_settings,
                    Some(Box::new(move || {
                        // OpenCode discovery finishes after the frontend's initial
                        // health fetch; tell it to re-read adapter availability.
                        let _ = discovery_handle.emit("adapters-changed", ());
                    })),
                )
                .map_err(Box::<dyn std::error::Error>::from)?,
            );
            let credential_broker = Arc::new(
                credential_broker::CredentialBroker::openai()
                    .map_err(|error| Box::<dyn std::error::Error>::from(error))?,
            );
            let bundled_extension = app.path().resource_dir()?.join("browser-extension");
            let extension_path = if bundled_extension.exists() {
                bundled_extension
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension")
            };
            let browser_bridge = browser_bridge::BrowserBridgeSupervisor::start(
                extension_path,
                data.join("browser-site-metrics.json"),
            );
            start_health_server(
                db_path.clone(),
                adapter_registry.descriptors(),
                credential_broker.clone(),
            );
            app.manage(AppState {
                db: Mutex::new(connection),
                telemetry_db: Mutex::new(telemetry_connection),
                runtimes: Mutex::new(HashMap::new()),
                adapters: Mutex::new(HashMap::new()),
                adapter_registry,
                delegations: Mutex::new(DelegationState::default()),
                worktrees: data.join("worktrees"),
                database_path: db_path,
                telemetry_database_path: telemetry_db_path,
                snapshot_dir,
                skill_store: data.join("skills"),
                skill_consents: Arc::new(Mutex::new(HashMap::new())),
                credential_broker,
                browser_bridge,
            });
            start_worker_maintenance(app.handle().clone());
            start_learning_maintenance(app.handle().clone());
            start_history_snapshot_maintenance(app.handle().clone());
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
        let selected = resolve_orchestrator_selection(&db, &registry).unwrap();
        assert_eq!(selected.adapter_id, expected_provider);
        assert_eq!(selected.model, expected_model);
        assert_eq!(selected.effort, Some(delegation::Effort::High));
        assert_eq!(selected.tier, CapabilityTier::Standard);
    }

    #[test]
    fn tauri_commands_never_block_the_ui_thread() {
        let source = include_str!("lib.rs");
        assert!(
            !source.contains("#[tauri::command]\nfn "),
            "Tauri commands must be async so native work never runs on the macOS UI thread"
        );
        assert!(source.contains("learning_job::run_local_database(&database_path, trigger_kind)"));
        let locked_learning_call = [
            "learning_job::run_learning(",
            "&state.db.lock().unwrap()",
            ", trigger_kind)",
        ]
        .concat();
        assert!(!source.contains(&locked_learning_call));
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
        archive_workspace_records(&db, "w", || Ok(())).unwrap();
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
        let initial = session_forest_snapshot(&db, "s").unwrap();
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

        let rewound = activate_session_entry_records(&db, "s", "e1").unwrap();
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
            session_forest_snapshot(&db, "s")
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
            session_forest_snapshot(&db, "s")
                .unwrap()
                .repository_divergence
                .status,
            "aligned"
        );

        let rewound = activate_session_entry_records(&db, "s", &clean.id).unwrap();
        assert_eq!(rewound.repository_divergence.status, "diverged");
        assert_eq!(
            std::fs::read_to_string(repository.join("tracked.txt")).unwrap(),
            "changed\n"
        );
    }

    #[test]
    fn archive_workspace_records_rolls_back_when_worktree_removal_fails() {
        let db = archive_fixture();
        let result = archive_workspace_records(&db, "w", || {
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
        record_approved_launch_failure(&db, "parent", "turn-retry", &request).unwrap();
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

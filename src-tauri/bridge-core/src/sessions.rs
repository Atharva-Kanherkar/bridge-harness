//! Sessions domain, management and control surfaces: creating chats and
//! workspace sessions, switching models, reading/rewinding the session forest,
//! interrupting turns, starting manual compaction, and refreshing account usage
//! as [`BridgeCore`] methods.
//!
//! Host shells keep only transport concerns: blocking-pool placement for Git
//! scans and worktree creation, and event emission after mutations. Methods
//! that surround a host-run blocking step are split into a `plan_*` /
//! `persist_*`(or `commit_*`) pair; everything in between is the host's
//! scheduling choice, not domain logic.
//!
//! Starting adapters, sending turns, and delivering checkpoint prompts remain
//! host-run live-turn orchestration until that slice moves behind the core seam.

use crate::model::*;
use crate::runtime::BridgeCore;
use crate::{
    adapters, agent, agent_config, binary, claude_adapter, compaction_controller, completion,
    git, model_profiles, orchestrator, policy, restoration, session_forest, session_supervisor,
    store, BridgeError,
};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct OrchestratorSelection {
    pub adapter_id: String,
    pub model: String,
    pub tier: CapabilityTier,
    pub effort: Option<crate::delegation::Effort>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorWorktree {
    pub path: PathBuf,
    pub branch: String,
}

/// Everything resolved up front for a new workspace session, so the host can
/// run worktree creation on its blocking pool between planning and
/// persistence.
/// Fields are private on purpose: a plan is an opaque, single-use token
/// bound to the workspace it was planned for, consumed by
/// [`BridgeCore::persist_workspace_session`]. Hosts read what they need for
/// the blocking step through the accessors.
#[derive(Debug)]
pub struct WorkspaceSessionPlan {
    workspace_id: String,
    session_id: String,
    selection: OrchestratorSelection,
    workspace_title: String,
    workspace_path: Option<String>,
    worktree_source: Option<String>,
}

impl WorkspaceSessionPlan {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn workspace_title(&self) -> &str {
        &self.workspace_title
    }

    /// Repository to create the isolated worktree from; `Some` exactly when
    /// isolation was requested (validated to have a connected repository).
    pub fn worktree_source(&self) -> Option<&str> {
        self.worktree_source.as_deref()
    }
}

/// A validated, not-yet-applied chat model switch: an opaque, single-use
/// token bound to the session it was planned for and carrying the planned
/// revision (the previous harness/model), which the commit re-verifies.
#[derive(Debug)]
pub struct ChatModelChange {
    session_id: String,
    adapter_id: String,
    kind: String,
    previous_harness: String,
    previous_model: Option<String>,
    selected: ModelOption,
}

impl ChatModelChange {
    /// The model the plan selected (visible for logging and tests).
    pub fn selected_model(&self) -> &str {
        &self.selected.id
    }
}

impl BridgeCore {
    /// Scratch working directory for a chat that has no connected folder/repo.
    pub fn chat_scratch_dir(&self, session_id: &str) -> PathBuf {
        self.database_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chats")
            .join(session_id)
    }

    /// Create a standalone direct chat (no workspace). Runs in a private
    /// scratch dir.
    pub fn create_chat(
        &self,
        harness: &Harness,
        model: Option<&str>,
        title: Option<&str>,
    ) -> Result<BridgeState, BridgeError> {
        let adapter_id = store::harness_name(harness);
        let id = Uuid::new_v4().to_string();
        let cwd = self.chat_scratch_dir(&id);
        let label = chat_label(title);
        let db = self.db.lock().unwrap();
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

    /// Move a session's conversation head. Publishes the state-changed
    /// refetch hint once the move is recorded.
    pub fn activate_session_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<SessionForestSnapshot, BridgeError> {
        let db = self.db.lock().unwrap();
        let snapshot = activate_session_entry_records(&db, session_id, entry_id)?;
        self.events.publish(crate::events::CoreEvent::StateChanged);
        Ok(snapshot)
    }

    /// The session's repository path, if any. Hosts resolve this under the
    /// lock, then compute the repository state outside it — Git may be slow
    /// on large repositories or during index contention.
    pub fn session_repository_path(
        &self,
        session_id: &str,
    ) -> Result<Option<PathBuf>, BridgeError> {
        let db = self.db.lock().unwrap();
        store::repository_path_for_session(&db, session_id)
    }

    pub fn session_forest_snapshot_with_repository_state(
        &self,
        session_id: &str,
        repository_state: serde_json::Value,
    ) -> Result<SessionForestSnapshot, BridgeError> {
        let db = self.db.lock().unwrap();
        session_forest_snapshot_with_repository_state(&db, session_id, repository_state)
    }

    /// Resolve the orchestrator selection and workspace facts for a new
    /// session; validates isolation requirements when `isolated` is set.
    pub fn plan_workspace_session(
        &self,
        workspace_id: &str,
        isolated: bool,
    ) -> Result<WorkspaceSessionPlan, BridgeError> {
        let (selection, workspace_title, workspace_path, project_id) = {
            let db = self.db.lock().unwrap();
            let selection = resolve_orchestrator_selection(&db, &self.adapter_registry)?;
            let (title, path, project_id): (String, Option<String>, Option<String>) = db
                .query_row(
                    "SELECT title,path,project_id FROM workspaces WHERE id=?1",
                    params![workspace_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            (selection, title, path, project_id)
        };
        let worktree_source = if isolated {
            if project_id.is_none() {
                return Err(BridgeError::Invalid(
                    "Connect a Git repository before creating an isolated worktree".into(),
                ));
            }
            Some(
                workspace_path
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        BridgeError::Invalid(
                            "Connect a Git repository before creating an isolated worktree".into(),
                        )
                    })?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(WorkspaceSessionPlan {
            workspace_id: workspace_id.to_owned(),
            session_id: Uuid::new_v4().to_string(),
            selection,
            workspace_title,
            workspace_path,
            worktree_source,
        })
    }

    /// Persist the planned session (and its worktree evidence) in one
    /// transaction. A persistence failure removes the just-created worktree
    /// so a retry starts clean.
    pub fn persist_workspace_session(
        &self,
        plan: WorkspaceSessionPlan,
        worktree: Option<OrchestratorWorktree>,
    ) -> Result<BridgeState, BridgeError> {
        let cwd = worktree
            .as_ref()
            .map(|value| value.path.to_string_lossy().into_owned())
            .or_else(|| plan.workspace_path.clone())
            .unwrap_or_else(|| {
                self.chat_scratch_dir(&plan.session_id)
                    .to_string_lossy()
                    .to_string()
            });
        let persisted = (|| -> Result<BridgeState, BridgeError> {
            let db = self.db.lock().unwrap();
            let transaction = db.unchecked_transaction()?;
            transaction.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,requested_tier,effort,kind,cwd,depth) VALUES(?1,?2,?3,?4,'idle','estimated',?5,?6,?7,'orchestrator',?8,0)",
                params![plan.session_id, plan.workspace_id, plan.selection.adapter_id, plan.selection.label, plan.selection.model, plan.selection.tier.as_str(), plan.selection.effort.map(|effort| effort.as_str()), cwd],
            )?;
            store::event(
                &transaction,
                "supervisor",
                "session.created",
                &plan.session_id,
                "New agent session",
            )?;
            if let Some(created) = &worktree {
                store::event(
                    &transaction,
                    "worktree",
                    "session.worktree_created",
                    &plan.session_id,
                    &format!(
                        "Created isolated worktree {} on branch {}",
                        created.path.display(),
                        created.branch
                    ),
                )?;
            }
            let next = store::state(&transaction)?;
            transaction.commit()?;
            Ok(next)
        })();
        if persisted.is_err() {
            if let Some(created) = &worktree {
                let _ = git::remove_worktree(
                    Path::new(plan.workspace_path.as_deref().unwrap_or("")),
                    &created.path,
                );
            }
        }
        persisted
    }

    /// Validate a chat model switch and select the concrete model. Returns
    /// `None` when the chat already runs the requested harness/model.
    pub fn plan_chat_model_change(
        &self,
        session_id: &str,
        harness: &Harness,
        model: Option<&str>,
    ) -> Result<Option<ChatModelChange>, BridgeError> {
        let adapter_id = store::harness_name(harness);
        if !agent_config::is_harness_enabled(&self.db.lock().unwrap(), adapter_id) {
            return Err(BridgeError::Invalid(format!(
                "{} is disabled in Settings",
                harness.label()
            )));
        }
        let (kind, previous_harness, previous_model, active_turn_id, parent_session_id): (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self.db.lock().unwrap().query_row(
            "SELECT kind,harness,model,active_turn_id,parent_session_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        if parent_session_id.is_some() || !matches!(kind.as_str(), "direct" | "orchestrator") {
            return Err(BridgeError::Invalid(
                "Only root chats and orchestrators can change models".into(),
            ));
        }
        if active_turn_id.is_some() {
            return Err(BridgeError::Invalid(
                "Wait for the current response before switching models".into(),
            ));
        }
        let descriptor = self
            .adapter_registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == adapter_id)
            .ok_or_else(|| {
                BridgeError::Invalid(format!("No model adapter is registered for {adapter_id}"))
            })?;
        if !descriptor.available {
            return Err(BridgeError::Invalid(
                descriptor
                    .unavailable_reason
                    .unwrap_or_else(|| format!("{} is unavailable", descriptor.label)),
            ));
        }
        let default_tier = if kind == "orchestrator" {
            CapabilityTier::Standard
        } else {
            CapabilityTier::Fast
        };
        let selected = if let Some(requested) =
            model.filter(|value| !value.trim().is_empty())
        {
            descriptor
                .models
                .iter()
                .find(|option| option.id.eq_ignore_ascii_case(requested.trim()))
                .cloned()
                .ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "{} does not offer model {requested}",
                        descriptor.label
                    ))
                })?
        } else {
            descriptor
                .models
                .iter()
                .find(|option| option.tier == default_tier && option.default_for_tier)
                .or_else(|| {
                    descriptor
                        .models
                        .iter()
                        .find(|option| option.tier == default_tier)
                })
                .cloned()
                .ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "{} has no {} model",
                        descriptor.label,
                        default_tier.as_str()
                    ))
                })?
        };
        if previous_harness == adapter_id && previous_model.as_deref() == Some(selected.id.as_str())
        {
            return Ok(None);
        }
        Ok(Some(ChatModelChange {
            session_id: session_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            kind,
            previous_harness,
            previous_model,
            selected,
        }))
    }

    /// Remove and stop a session's live adapter runtime, if any. Blocking —
    /// hosts place this on their blocking pool.
    pub fn stop_session_adapter(&self, session_id: &str, reason: adapters::ShutdownReason) {
        if let Some(mut runtime) = self.adapters.lock().unwrap().remove(session_id) {
            runtime.stop(reason);
        }
    }

    /// Replay durable session events with a sequence greater than the
    /// cursor. This is the recovery path of the notify-then-replay contract:
    /// after a disconnect or a lagged live channel, a client calls this with
    /// its last seen cursor and receives the missed durable history with no
    /// gaps and no duplicates. Transient frames are never replayed.
    pub fn replay_session_events(
        &self,
        session_id: &str,
        after_sequence: i64,
        limit: Option<u32>,
    ) -> Result<Vec<AgentEvent>, BridgeError> {
        if after_sequence < 0 {
            return Err(BridgeError::Invalid(
                "afterSequence must be non-negative".into(),
            ));
        }
        let limit = limit.unwrap_or(bridge_protocol::messages::DEFAULT_REPLAY_EVENT_LIMIT);
        if !(1..=bridge_protocol::messages::MAX_REPLAY_EVENT_LIMIT).contains(&limit) {
            return Err(BridgeError::Invalid(format!(
                "limit must be between 1 and {}",
                bridge_protocol::messages::MAX_REPLAY_EVENT_LIMIT
            )));
        }
        let db = self.db.lock().unwrap();
        store::session_events_after(&db, session_id, after_sequence, limit)
    }

    /// Interrupt the session's active turn on its live adapter runtime.
    pub fn interrupt_turn(&self, session_id: &str) -> Result<(), BridgeError> {
        let adapters = self.adapters.lock().unwrap();
        let runtime = adapters
            .get(session_id)
            .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
        runtime.interrupt()
    }

    /// Validate and begin a manual compaction, returning the checkpoint
    /// prompt. Delivering that prompt as an internal turn is still shell
    /// machinery until the live-turn slice lands.
    pub fn begin_manual_compaction(&self, session_id: &str) -> Result<String, BridgeError> {
        let db = self.db.lock().unwrap();
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
            .active_branch(session_id)
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
        let tokens = compaction_controller::active_token_estimate(&db, session_id)?;
        compaction_controller::CompactionController::begin(
            &db,
            session_id,
            compaction_controller::CompactionReason::Manual,
            tokens,
        )?
        .ok_or_else(|| BridgeError::Invalid("Compaction is already pending".into()))
    }

    /// Publish one provider's subscription usage tick to the ambient meter.
    pub fn publish_account_usage(&self, provider: &str, rate_limits: serde_json::Value) {
        self.events.publish(crate::events::CoreEvent::AccountUsage {
            provider: provider.to_owned(),
            rate_limits,
        });
    }

    /// Ask one live Codex session for account-wide rate limits; the reply
    /// arrives asynchronously on its event stream.
    pub fn request_codex_usage(&self) -> Result<(), BridgeError> {
        let codex_sessions: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut statement =
                db.prepare("SELECT id FROM sessions WHERE harness='codex' AND ended_at IS NULL")?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            ids
        };
        let adapters = self.adapters.lock().unwrap();
        for session_id in codex_sessions {
            if let Some(runtime) = adapters.get(&session_id) {
                let _ = runtime.read_usage();
                break;
            }
        }
        Ok(())
    }

    /// Refresh subscription usage for every provider, independent of which
    /// session is on screen. Claude is queried out-of-band via its headless
    /// `/usage` command; Codex is asked on a live session and answers on its
    /// event stream. Both results ride the `account-usage` channel.
    pub fn refresh_account_usage(&self) -> Result<(), BridgeError> {
        // Claude: a global, read-only account query — no running session
        // required. The probe shells out and can be slow; it runs on its own
        // thread and publishes when it returns.
        if binary::resolve("claude").is_some() {
            let events = self.events.clone();
            std::thread::spawn(move || {
                let cwd = std::env::temp_dir();
                let cwd = cwd.to_string_lossy();
                if let Some(data) = claude_adapter::read_usage_snapshot(cwd.as_ref()) {
                    if let Some(rate_limits) = data.get("rateLimits") {
                        events.publish(crate::events::CoreEvent::AccountUsage {
                            provider: "claude".into(),
                            rate_limits: rate_limits.clone(),
                        });
                    }
                }
            });
        }
        // Codex: rate limits are account-wide, so a single running session
        // answers for the whole account.
        self.request_codex_usage()
    }

    /// Apply a planned model change: clear the tracked provider process,
    /// persist the selection, reset restoration state, record both audit
    /// events, and publish the durable agent event. Also returns it.
    pub fn commit_chat_model_change(
        &self,
        change: ChatModelChange,
    ) -> Result<AgentEvent, BridgeError> {
        let session_id = change.session_id.as_str();
        let db = self.db.lock().unwrap();
        let transaction = db.unchecked_transaction()?;
        session_supervisor::SessionSupervisor::clear_adapter_process(&transaction, session_id)?;
        if persist_chat_model_selection(
            &transaction,
            session_id,
            &change.adapter_id,
            &change.selected.id,
            change.selected.tier,
            (&change.previous_harness, change.previous_model.as_deref()),
        )? != 1
        {
            return Err(BridgeError::Invalid(
                "The chat could not be updated because it changed while the switch was in flight"
                    .into(),
            ));
        }
        restoration::set_head_state(
            &transaction,
            session_id,
            RestorationMode::Fresh,
            ResumeEligibility::Fresh,
            None,
        )?;
        let subject = if change.kind == "orchestrator" {
            "Orchestrator"
        } else {
            "Chat"
        };
        let detail = format!(
            "{subject} runtime changed from {}/{} to {}/{}. The next message starts a fresh provider session.",
            change.previous_harness,
            change.previous_model.as_deref().unwrap_or("automatic"),
            change.adapter_id,
            change.selected.id,
        );
        store::event(&transaction, "chat", "session.model_changed", session_id, &detail)?;
        let event = store::session_event_in_transaction(
            &transaction,
            session_id,
            &agent::NormalizedEvent {
                kind: "session.model_changed".into(),
                item_id: Some(format!("model-change-{}", Uuid::new_v4())),
                role: Some("system".into()),
                status: Some("ready".into()),
                title: Some(format!("{subject} model changed")),
                text: Some(detail),
                data: serde_json::json!({
                    "previousHarness": change.previous_harness,
                    "previousModel": change.previous_model,
                    "harness": change.adapter_id,
                    "model": change.selected.id,
                    "modelLabel": change.selected.label,
                    "tier": change.selected.tier,
                    "freshProviderSession": true,
                }),
            },
            &serde_json::json!({"source": "user-selection"}),
        )?;
        transaction.commit()?;
        self.events.publish(crate::events::CoreEvent::Agent(event.clone()));
        Ok(event)
    }
}

fn chat_label(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New chat")
        .to_string()
}

pub fn session_forest_snapshot(
    db: &Connection,
    session_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    let current_state = store::repository_state_for_session(db, session_id)?;
    session_forest_snapshot_with_repository_state(db, session_id, current_state)
}

pub fn session_forest_snapshot_with_repository_state(
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

pub fn activate_session_entry_records(
    db: &Connection,
    session_id: &str,
    entry_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    session_forest::SessionForest::new(&transaction)
        .move_head(session_id, Some(entry_id))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        &transaction,
        "session-forest",
        "session.head_moved",
        session_id,
        &format!("Conversation head moved to {entry_id}; files were not changed"),
    )?;
    let snapshot = session_forest_snapshot(&transaction, session_id)?;
    transaction.commit()?;
    Ok(snapshot)
}

/// Crate-private on purpose: callers must go through the plan/commit pair so
/// validation cannot be bypassed. The WHERE clause re-verifies the planned
/// revision (previous harness/model) and that no turn started meanwhile, so a
/// stale plan updates zero rows instead of clobbering a changed session.
pub(crate) fn persist_chat_model_selection(
    db: &Connection,
    session_id: &str,
    adapter_id: &str,
    model: &str,
    tier: CapabilityTier,
    (previous_harness, previous_model): (&str, Option<&str>),
) -> Result<usize, BridgeError> {
    Ok(db.execute(
        "UPDATE sessions SET harness=?2,model=?3,requested_tier=?4,provider_session_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1 AND parent_session_id IS NULL AND kind IN ('direct','orchestrator') AND harness=?5 AND model IS ?6 AND active_turn_id IS NULL",
        params![session_id, adapter_id, model, tier.as_str(), previous_harness, previous_model],
    )?)
}

pub fn prepare_orchestrator_worktree(
    namespace_root: &Path,
    workspace_title: &str,
    workspace_path: &Path,
    session_id: &str,
) -> Result<OrchestratorWorktree, BridgeError> {
    git::validate_repo(workspace_path).map_err(|_| {
        BridgeError::Invalid(
            "Connect a Git repository before creating an isolated worktree".into(),
        )
    })?;
    let workspace_slug = {
        let value = git::slug(workspace_title);
        if value.is_empty() {
            "workspace".to_owned()
        } else {
            value
        }
    };
    let session_slug = git::slug(session_id);
    let short_session = session_slug.chars().take(8).collect::<String>();
    let branch = format!("bridge/{workspace_slug}-{short_session}");
    let path = namespace_root
        .join("orchestrators")
        .join(&workspace_slug)
        .join(session_id);
    git::create_worktree(workspace_path, &path, &branch)?;
    Ok(OrchestratorWorktree { path, branch })
}

pub fn resolve_orchestrator_selection(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// A registered, available harness with Standard and Fast models, so
    /// selection logic can run without real provider binaries.
    struct StubAdapter;
    impl adapters::HarnessAdapter for StubAdapter {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "codex".into(),
                label: "Codex".into(),
                available: true,
                version: None,
                capabilities: Vec::new(),
                unavailable_reason: None,
                models: vec![
                    ModelOption {
                        id: "stub-standard".into(),
                        label: "Stub Standard".into(),
                        tier: CapabilityTier::Standard,
                        default_for_tier: true,
                    },
                    ModelOption {
                        id: "stub-fast".into(),
                        label: "Stub Fast".into(),
                        tier: CapabilityTier::Fast,
                        default_for_tier: true,
                    },
                ],
                default_model: None,
            }
        }
        fn start(&self, _: adapters::StartRequest<'_>) -> Result<adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("stub adapter cannot start".into()))
        }
        fn resume(&self, _: adapters::ResumeRequest<'_>) -> Result<adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("stub adapter cannot resume".into()))
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _: &Value) -> Vec<agent::NormalizedEvent> {
            Vec::new()
        }
    }

    fn fixture() -> (tempfile::TempDir, BridgeCore) {
        let scratch = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        let mut registry = adapters::AdapterRegistry::empty();
        registry.register(Box::new(StubAdapter)).unwrap();
        core.adapter_registry = std::sync::Arc::new(registry);
        (scratch, core)
    }

    fn seed_workspace(core: &BridgeCore, with_project: bool) {
        let db = core.db.lock().unwrap();
        if with_project {
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/sessions-demo','now')", []).unwrap();
        }
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES('w',?1,'Payments API','/tmp/sessions-demo','idle','now')",
            params![with_project.then_some("p")],
        )
        .unwrap();
    }

    #[test]
    fn create_chat_persists_a_scratch_dir_direct_session() {
        let (_scratch, core) = fixture();
        let snapshot = core.create_chat(&Harness::Codex, Some("stub-fast"), Some("  Billing  ")).unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        let db = core.db.lock().unwrap();
        let (kind, label, model, cwd): (String, String, Option<String>, String) = db
            .query_row("SELECT kind,label,model,cwd FROM sessions", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        assert_eq!(kind, "direct");
        assert_eq!(label, "Billing");
        assert_eq!(model.as_deref(), Some("stub-fast"));
        assert!(cwd.contains("chats"), "direct chats run in a private scratch dir: {cwd}");
    }

    #[test]
    fn create_chat_defaults_the_label_when_the_title_is_blank() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, Some("   ")).unwrap();
        let db = core.db.lock().unwrap();
        let label: String = db.query_row("SELECT label FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(label, "New chat");
    }

    #[test]
    fn activate_session_entry_moves_the_head_and_records_the_event() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','S','idle','reported')", []).unwrap();
            db.execute("INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at) VALUES('e1','s',NULL,1,'user.message','{}','now'),('e2','s','e1',2,'assistant.message','{}','now')", []).unwrap();
            db.execute("INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,updated_at) VALUES('s','e2','fresh','now')", []).unwrap();
        }
        let mut events = core.events.subscribe();
        let snapshot = core.activate_session_entry("s", "e1").unwrap();
        assert_eq!(snapshot.head.unwrap().active_entry_id.as_deref(), Some("e1"));
        assert!(matches!(events.try_recv().unwrap(), crate::events::CoreEvent::StateChanged));
        let db = core.db.lock().unwrap();
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE kind='session.head_moved' AND entity_id='s')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
    }

    #[test]
    fn activate_session_entry_rolls_back_and_publishes_nothing_when_audit_fails() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','S','idle','reported')", []).unwrap();
            db.execute("INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at) VALUES('e1','s',NULL,1,'user.message','{}','now'),('e2','s','e1',2,'assistant.message','{}','now')", []).unwrap();
            db.execute("INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,updated_at) VALUES('s','e2','fresh','now')", []).unwrap();
            db.execute_batch(
                "CREATE TRIGGER fail_head_audit BEFORE INSERT ON events
                 WHEN NEW.kind='session.head_moved'
                 BEGIN SELECT RAISE(FAIL, 'injected audit failure'); END;",
            )
            .unwrap();
        }
        let mut events = core.events.subscribe();
        assert!(core.activate_session_entry("s", "e1").is_err());
        assert!(events.try_recv().is_err(), "a rolled-back head move must publish nothing");
        let active: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT active_entry_id FROM session_heads WHERE session_id='s'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(active, "e2", "the head move and audit must roll back together");
    }

    #[test]
    fn plan_workspace_session_validates_isolation_and_resolves_a_selection() {
        let (_scratch, core) = fixture();
        assert!(matches!(core.plan_workspace_session("missing", false), Err(BridgeError::Db(_))));

        seed_workspace(&core, false);
        let error = core.plan_workspace_session("w", true).unwrap_err();
        assert!(error.to_string().contains("Connect a Git repository"), "{error}");

        let plan = core.plan_workspace_session("w", false).unwrap();
        assert_eq!(plan.selection.adapter_id, "codex");
        assert!(
            plan.selection.model.starts_with("stub-"),
            "selection must come from the registered adapter, got {}",
            plan.selection.model
        );
        assert_eq!(plan.workspace_title, "Payments API");
        assert!(plan.worktree_source.is_none());
    }

    #[test]
    fn plan_workspace_session_requires_a_registered_adapter() {
        let scratch = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(scratch.path());
        seed_workspace(&core, false);
        let error = core.plan_workspace_session("w", false).unwrap_err();
        assert!(error.to_string().contains("no available adapter"), "{error}");
    }

    #[test]
    fn persist_workspace_session_records_the_orchestrator_row() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        let plan = core.plan_workspace_session("w", false).unwrap();
        let snapshot = core.persist_workspace_session(plan, None).unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        let db = core.db.lock().unwrap();
        let (kind, cwd, harness): (String, String, String) = db
            .query_row("SELECT kind,cwd,harness FROM sessions", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(kind, "orchestrator");
        assert_eq!(cwd, "/tmp/sessions-demo", "cwd falls back to the workspace path");
        assert_eq!(harness, "codex");
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE kind='session.created')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
    }

    #[test]
    fn failed_persistence_removes_the_created_worktree() {
        let (scratch, core) = fixture();
        // Real repository + worktree so the compensating removal is real.
        let repo = scratch.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "bridge-test@example.invalid"],
            vec!["config", "user.name", "Bridge Test"],
        ] {
            assert!(std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap().success());
        }
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        for args in [vec!["add", "."], vec!["commit", "-m", "fixture", "-q"]] {
            assert!(std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap().success());
        }
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", params![repo.to_string_lossy()]).unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES('w','p','Payments API',?1,'idle','now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
        }
        let plan = core.plan_workspace_session("w", true).unwrap();
        let worktree = prepare_orchestrator_worktree(
            &core.worktrees,
            plan.workspace_title(),
            Path::new(plan.worktree_source().unwrap()),
            plan.session_id(),
        )
        .unwrap();
        assert!(worktree.path.exists());
        // Force the insert to fail: a session with the planned id already exists.
        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES(?1,'w','codex','占','idle','reported')",
                params![plan.session_id()],
            )
            .unwrap();
        let result = core.persist_workspace_session(plan, Some(worktree.clone()));
        assert!(result.is_err());
        assert!(!worktree.path.exists(), "failed persistence must remove the worktree");
    }

    #[test]
    fn chat_model_changes_are_validated_planned_and_committed() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();

        // Unknown adapter (claude has no stub registered).
        let error = core.plan_chat_model_change(&session_id, &Harness::Claude, None).unwrap_err();
        assert!(error.to_string().contains("No model adapter"), "{error}");
        // Unknown model on a registered adapter.
        let error = core
            .plan_chat_model_change(&session_id, &Harness::Codex, Some("no-such-model"))
            .unwrap_err();
        assert!(error.to_string().contains("does not offer model"), "{error}");
        // Busy chats cannot switch.
        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET active_turn_id='turn' WHERE id=?1", params![session_id])
            .unwrap();
        let error = core.plan_chat_model_change(&session_id, &Harness::Codex, None).unwrap_err();
        assert!(error.to_string().contains("Wait for the current response"), "{error}");
        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET active_turn_id=NULL WHERE id=?1", params![session_id])
            .unwrap();

        // Plan + commit: direct chats default to the Fast tier.
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .expect("switching claude -> codex is a real change");
        assert_eq!(change.selected_model(), "stub-fast");
        let mut events = core.events.subscribe();
        let event = core.commit_chat_model_change(change).unwrap();
        assert_eq!(event.kind, "session.model_changed");
        // The durable agent event rides the bus with its replay cursor.
        match events.try_recv().unwrap() {
            crate::events::CoreEvent::Agent(published) => {
                assert_eq!(published.id, event.id);
                assert_eq!(published.sequence, event.sequence);
            }
            other => panic!("expected the agent event, got {:?}", other.kind()),
        }
        let db = core.db.lock().unwrap();
        let (harness, model): (String, Option<String>) = db
            .query_row("SELECT harness,model FROM sessions WHERE id=?1", params![session_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(harness, "codex");
        assert_eq!(model.as_deref(), Some("stub-fast"));
        drop(db);

        // Re-planning the same harness/model is a no-op.
        assert!(core
            .plan_chat_model_change(&session_id, &Harness::Codex, Some("stub-fast"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn chat_model_change_rolls_back_and_publishes_nothing_when_history_fails() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        core.db
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_model_history BEFORE INSERT ON session_entries
                 WHEN NEW.kind='session.model_changed'
                 BEGIN SELECT RAISE(FAIL, 'injected history failure'); END;",
            )
            .unwrap();

        let mut events = core.events.subscribe();
        assert!(core.commit_chat_model_change(change).is_err());
        assert!(events.try_recv().is_err(), "a rolled-back model change must publish nothing");
        let (harness, model): (String, Option<String>) = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT harness,model FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(harness, "claude");
        assert_eq!(model, None, "session state and durable history must commit together");
    }

    #[test]
    fn user_model_selection_updates_an_orchestrator_session() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,requested_tier,kind,depth) VALUES('orchestrator',NULL,'codex','Orchestrator','ready','reported','old-model','standard','orchestrator',0)",
            [],
        )
        .unwrap();
        let changed = persist_chat_model_selection(
            &db,
            "orchestrator",
            "claude",
            "opus",
            CapabilityTier::Strong,
            ("codex", Some("old-model")),
        )
        .unwrap();
        assert_eq!(changed, 1);
        let actual: (String, String, String, String, Option<String>) = db
            .query_row(
                "SELECT harness,model,requested_tier,status,provider_session_id FROM sessions WHERE id='orchestrator'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(actual, ("claude".into(), "opus".into(), "strong".into(), "idle".into(), None));

        // A stale revision (the session changed since planning) updates nothing.
        let stale = persist_chat_model_selection(
            &db,
            "orchestrator",
            "codex",
            "other",
            CapabilityTier::Standard,
            ("codex", Some("old-model")),
        )
        .unwrap();
        assert_eq!(stale, 0, "a stale plan must not clobber a changed session");
    }

    #[test]
    fn lifecycle_claims_serialize_starts_against_model_switches() {
        let (_scratch, core) = fixture();
        let claim = core.claim_session_lifecycle("s", "model switch").unwrap();
        let error = core.claim_session_lifecycle("s", "session start").unwrap_err();
        assert!(
            error.to_string().contains("model switch"),
            "the conflict names the operation in flight: {error}"
        );
        // Other sessions are unaffected; releasing the claim reopens the session.
        core.claim_session_lifecycle("other", "session start").unwrap();
        drop(claim);
        core.claim_session_lifecycle("s", "session start").unwrap();
    }

    #[test]
    fn a_session_that_changed_after_planning_cannot_be_clobbered_by_the_commit() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        // Simulate an interleaved switch landing first: the stored model no
        // longer matches the plan's revision.
        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET model='switched-elsewhere' WHERE id=?1", params![session_id])
            .unwrap();
        let mut events = core.events.subscribe();
        let error = core.commit_chat_model_change(change).unwrap_err();
        assert!(error.to_string().contains("changed while the switch was in flight"), "{error}");
        assert!(events.try_recv().is_err(), "a stale model-change plan must publish nothing");
        let model: Option<String> = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT model FROM sessions WHERE id=?1", params![session_id], |row| row.get(0))
            .unwrap();
        assert_eq!(model.as_deref(), Some("switched-elsewhere"), "the interleaved state survives");
    }

    #[test]
    fn commit_refuses_when_the_session_stopped_being_a_root_chat() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET parent_session_id='parent' WHERE id=?1", params![session_id])
            .unwrap();
        let error = core.commit_chat_model_change(change).unwrap_err();
        assert!(error.to_string().contains("changed while the switch was in flight"), "{error}");
    }

    /// A live adapter runtime that records control calls.
    struct RecordingRuntime {
        interrupted: std::sync::Arc<std::sync::atomic::AtomicBool>,
        usage_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl adapters::AdapterRuntime for RecordingRuntime {
        fn process_id(&self) -> u32 {
            0
        }
        fn provider_session_id(&self) -> &str {
            "recording"
        }
        fn current_turn(&self) -> std::sync::Arc<std::sync::Mutex<Option<String>>> {
            std::sync::Arc::new(std::sync::Mutex::new(None))
        }
        fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            self.interrupted.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn respond(&self, _: serde_json::Value, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn read_usage(&self) -> Result<(), BridgeError> {
            self.usage_requested.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn stop(&mut self, _: adapters::ShutdownReason) {}
    }

    #[test]
    fn interrupt_turn_requires_and_reaches_the_live_runtime() {
        let (_scratch, core) = fixture();
        let error = core.interrupt_turn("absent").unwrap_err();
        assert!(error.to_string().contains("not running"), "{error}");

        let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        core.adapters.lock().unwrap().insert(
            "live".into(),
            Box::new(RecordingRuntime {
                interrupted: interrupted.clone(),
                usage_requested: Default::default(),
            }),
        );
        core.interrupt_turn("live").unwrap();
        assert!(interrupted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn manual_compaction_validates_status_and_meaningful_work() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','S','working','reported')", []).unwrap();
        }
        let error = core.begin_manual_compaction("s").unwrap_err();
        assert!(error.to_string().contains("waits until"), "{error}");

        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='idle' WHERE id='s'", [])
            .unwrap();
        // No meaningful conversation yet: the controller suppresses it.
        let error = core.begin_manual_compaction("s").unwrap_err();
        assert!(error.to_string().contains("Compaction suppressed"), "{error}");

        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at) VALUES('e1','s',NULL,1,'assistant.message','{\"text\":\"work\"}','now')",
                [],
            )
            .unwrap();
        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,updated_at) VALUES('s','e1','fresh','now')",
                [],
            )
            .unwrap();
        let prompt = core.begin_manual_compaction("s").unwrap();
        assert!(!prompt.is_empty());
        let error = core.begin_manual_compaction("s").unwrap_err();
        assert!(error.to_string().contains("already pending"), "{error}");
    }

    #[test]
    fn codex_usage_is_requested_on_one_live_session() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('c1','w','codex','S','idle','reported')", []).unwrap();
        }
        // No live runtime: the request is a quiet no-op.
        core.request_codex_usage().unwrap();

        let usage_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        core.adapters.lock().unwrap().insert(
            "c1".into(),
            Box::new(RecordingRuntime {
                interrupted: Default::default(),
                usage_requested: usage_requested.clone(),
            }),
        );
        core.request_codex_usage().unwrap();
        assert!(usage_requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn account_usage_ticks_ride_the_bus_with_the_legacy_payload() {
        let (_scratch, core) = fixture();
        let mut events = core.events.subscribe();
        core.publish_account_usage("codex", serde_json::json!({"remaining": 5}));
        match events.try_recv().unwrap() {
            crate::events::CoreEvent::AccountUsage { provider, rate_limits } => {
                assert_eq!(provider, "codex");
                assert_eq!(rate_limits, serde_json::json!({"remaining": 5}));
            }
            other => panic!("expected account usage, got {:?}", other.kind()),
        }
    }

    #[test]
    fn kill_and_reconnect_replays_missed_events_by_cursor_with_no_gaps_or_duplicates() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        let persist = |text: &str| -> i64 {
            let db = core.db.lock().unwrap();
            let event = store::session_event(
                &db,
                &session_id,
                &agent::NormalizedEvent {
                    kind: "message.completed".into(),
                    item_id: Some(format!("item-{text}")),
                    role: Some("assistant".into()),
                    status: Some("completed".into()),
                    title: None,
                    text: Some(text.into()),
                    data: serde_json::json!({}),
                },
                &serde_json::json!({"adapter": "codex"}),
            )
            .unwrap();
            drop(db);
            core.events.publish(crate::events::CoreEvent::Agent(event.clone()));
            event.sequence
        };

        // The client is connected for the first two events...
        let mut live = core.events.subscribe();
        persist("one");
        let second = persist("two");
        let mut last_seen = 0;
        for _ in 0..2 {
            if let crate::events::CoreEvent::Agent(event) = live.try_recv().unwrap() {
                last_seen = event.sequence;
            }
        }
        assert_eq!(last_seen, second);
        // ...then dies. Three more events land while it is gone.
        drop(live);
        let third = persist("three");
        persist("four");
        let fifth = persist("five");

        // Reconnect: subscribe first, then replay from the last seen cursor.
        // An event committed in that window appears in both streams; the
        // client drops live cursors at or below the replay high-water mark.
        let mut reconnected = core.events.subscribe();
        let raced = persist("raced");
        let replayed = core.replay_session_events(&session_id, last_seen, None).unwrap();
        let sequences: Vec<i64> = replayed.iter().map(|event| event.sequence).collect();
        assert_eq!(
            sequences,
            vec![third, third + 1, fifth, raced],
            "replay itself has no gaps or duplicates"
        );
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(replayed[0].text.as_deref(), Some("three"));
        assert_eq!(replayed[0].kind, "assistant.message", "replay carries the durable forest kind");
        assert!(replayed[0].sequence > 0, "durable events always carry a positive cursor");
        let replay_high_water = *sequences.last().unwrap();
        let raced_live = match reconnected.try_recv().unwrap() {
            crate::events::CoreEvent::Agent(event) => event,
            other => panic!("expected raced agent event, got {:?}", other.kind()),
        };
        assert_eq!(raced_live.sequence, replay_high_water);
        let mut delivered = sequences.clone();
        if raced_live.sequence > replay_high_water {
            delivered.push(raced_live.sequence);
        }
        let after_replay = persist("after-replay");
        let newer_live = match reconnected.try_recv().unwrap() {
            crate::events::CoreEvent::Agent(event) => event,
            other => panic!("expected newer agent event, got {:?}", other.kind()),
        };
        if newer_live.sequence > replay_high_water {
            delivered.push(newer_live.sequence);
        }
        assert_eq!(delivered, vec![third, third + 1, fifth, raced, after_replay]);
        // Replaying from the newest cursor is empty; from zero is everything durable.
        assert!(core
            .replay_session_events(&session_id, after_replay, None)
            .unwrap()
            .is_empty());
        assert!(core.replay_session_events(&session_id, 0, None).unwrap().len() >= 5);
        // Unknown sessions replay nothing rather than erroring.
        assert!(core
            .replay_session_events("no-such-session", 0, None)
            .unwrap()
            .is_empty());
        assert!(core.replay_session_events(&session_id, -1, None).is_err());
        assert!(core.replay_session_events(&session_id, 0, Some(0)).is_err());
        assert!(core
            .replay_session_events(
                &session_id,
                0,
                Some(bridge_protocol::messages::MAX_REPLAY_EVENT_LIMIT + 1),
            )
            .is_err());
        assert_eq!(
            core.replay_session_events(&session_id, 0, Some(2))
                .unwrap()
                .len(),
            2,
            "replay pages are bounded by the requested limit"
        );
    }

    #[test]
    fn replay_preserves_legacy_and_typed_forest_payloads() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, None, None).unwrap();
        let session_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        let (stored, typed_payload) = {
            let db = core.db.lock().unwrap();
            let stored = store::session_event(
                &db,
                &session_id,
                &agent::NormalizedEvent {
                    kind: "message.completed".into(),
                    item_id: Some("item-1".into()),
                    role: Some("assistant".into()),
                    status: Some("completed".into()),
                    title: Some("Title".into()),
                    text: Some("Body".into()),
                    data: serde_json::json!([{"nested": 7}]),
                },
                &serde_json::json!({"adapter": "codex", "requestId": "r1"}),
            )
            .unwrap();
            let typed = session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::ArtifactCreated,
                    serde_json::json!({"path": "result.json", "protocolVersion": 99}),
                )
                .unwrap();
            (stored, typed.payload)
        };

        let replayed = core.replay_session_events(&session_id, 0, None).unwrap();
        assert_eq!(replayed[0].item_id.as_deref(), Some("item-1"));
        assert_eq!(replayed[0].title.as_deref(), Some("Title"));
        assert_eq!(replayed[0].data, serde_json::json!([{"nested": 7}]));
        assert_eq!(
            replayed[0].provider_meta,
            serde_json::json!({"adapter": "codex", "requestId": "r1"})
        );
        assert_eq!(replayed[0].created_at, stored.created_at);
        assert_eq!(replayed[1].kind, "artifact.created");
        assert_eq!(replayed[1].protocol_version, 1);
        assert_eq!(replayed[1].data, typed_payload);
    }

    #[test]
    fn replay_rejects_corrupt_or_future_schema_history() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, None, None).unwrap();
        let db = core.db.lock().unwrap();
        let session_id: String = db
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        session_forest::SessionForest::new(&db)
            .append(
                &session_id,
                session_forest::EntryKind::ArtifactCreated,
                serde_json::json!({"path": "result.json"}),
            )
            .unwrap();
        db.execute(
            "UPDATE session_entries SET payload='not-json' WHERE session_id=?1",
            params![session_id],
        )
        .unwrap();
        drop(db);
        assert!(core.replay_session_events(&session_id, 0, None).is_err());

        let db = core.db.lock().unwrap();
        db.execute(
            "UPDATE session_entries SET payload='{\"path\":\"result.json\"}',semantic_schema_version=?2 WHERE session_id=?1",
            params![
                session_id,
                SEMANTIC_EVENT_SCHEMA_VERSION + 1,
            ],
        )
        .unwrap();
        drop(db);
        assert!(core.replay_session_events(&session_id, 0, None).is_err());
    }

    #[test]
    fn stopping_a_session_without_a_live_adapter_is_a_no_op() {
        let (_scratch, core) = fixture();
        core.stop_session_adapter("nothing-running", adapters::ShutdownReason::Replaced);
        assert!(core.adapters.lock().unwrap().is_empty());
    }
}

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
    context::ContextProjector, git, handoff, model_profiles, orchestrator, policy, restoration,
    session_forest, session_supervisor, store, BridgeError,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Whether a new aside may open as a native Codex thread fork of its source:
/// same harness (the fork verb belongs to the thread's owner), a stored
/// non-empty provider thread id to fork from, no turn in flight on the source
/// (forking a live turn is a race), and an installed Codex that exposes
/// `thread/fork`. Pure so every caller — and every test — gets the same answer.
fn aside_fork_eligible(
    aside_harness: &str,
    source_harness: &str,
    source_thread: Option<&str>,
    source_turn_active: bool,
    codex_supports_fork: bool,
) -> bool {
    aside_harness == "codex"
        && source_harness == "codex"
        && source_thread.is_some_and(|thread| !thread.trim().is_empty())
        && !source_turn_active
        && codex_supports_fork
}

/// Below this many tokens on the active branch, a model switch asks for no
/// handoff summary.
///
/// The summary exists to *compress* context the incoming model could not
/// otherwise carry. Under the floor there is nothing to compress —
/// `start_chat`'s mechanical projection replays the whole branch verbatim — so
/// the round trip buys nothing and can only cost: a visible pause, and an agent
/// asked to checkpoint a conversation that has barely started.
pub const SWITCH_SUMMARY_MIN_TOKENS: i64 = 1_500;

/// A pending request for the outgoing provider to summarise the conversation
/// before a model switch tears it down. Delivery and settlement are host-side
/// live-turn orchestration; this is the durable, validated part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchSummaryRequest {
    pub session_id: String,
    pub prompt: String,
    /// The sequence of this request's own `compaction.requested` entry.
    /// Outcome reads only consider terminal entries with a greater sequence,
    /// so a stale terminal from an earlier checkpoint can never answer for
    /// this request.
    pub after_sequence: i64,
}

/// What became of a [`SwitchSummaryRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchSummaryOutcome {
    /// A valid typed summary landed in the forest (`compaction` entry).
    Summarised,
    /// The pipeline recorded a failure (`compaction.failed`) — parse errors,
    /// repair exhaustion, or a cancelled request.
    Failed,
    /// Still awaiting the provider (including its single repair retry).
    Pending,
}

/// What a switched chat will actually inherit from its previous turns, as
/// reported by the `session.model_changed` transcript event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarriedContext {
    pub summary: bool,
    pub decisions: usize,
    pub files_touched: usize,
    pub recent_entries: usize,
}

impl CarriedContext {
    fn describe(&self) -> String {
        if self.summary {
            format!(
                "carried forward: summary + {} decisions + {} files",
                self.decisions, self.files_touched
            )
        } else {
            format!(
                "carried forward: {} recent entries (no summary available)",
                self.recent_entries
            )
        }
    }
}

/// Project what the next provider would receive from stored history. `None`
/// means nothing can be carried: an empty branch projects to nothing.
fn carried_context(db: &Connection, session_id: &str) -> Option<CarriedContext> {
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .ok()?;
    let projection = ContextProjector::project(&branch, 128_000).ok()?;
    let recent_entries = projection
        .render_entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind.as_str(),
                "user.message" | "assistant.message" | "worker.result" | "handoff.brief"
            )
        })
        .count();
    if recent_entries == 0 && projection.restoration_context.is_none() {
        return None;
    }
    Some(CarriedContext {
        summary: projection.restoration_context.is_some(),
        decisions: projection
            .restoration_context
            .as_ref()
            .map(|context| context.decisions.len())
            .unwrap_or(0),
        files_touched: projection
            .restoration_context
            .as_ref()
            .map(|context| context.files_touched.len())
            .unwrap_or(0),
        recent_entries,
    })
}

#[derive(Debug, Clone)]
pub struct OrchestratorSelection {
    pub adapter_id: String,
    /// None leaves model selection to the provider when its catalog is not yet available.
    pub model: Option<String>,
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
    selected: Option<ModelOption>,
    tier: CapabilityTier,
    /// Whether the next turn can resume the stored provider thread under the
    /// new model. Decided at plan time, when the session row and the adapter
    /// registry are both in hand: the harness must be unchanged, the adapter
    /// must support native resume, and a thread must actually be stored.
    /// Harness equality alone is not enough — an adapter without native
    /// resume, or a chat that never started, still needs the summary and the
    /// projection path, or the switch would promise a continuation the next
    /// start cannot deliver.
    native_continuation: bool,
}

impl ChatModelChange {
    /// The model the plan selected (visible for logging and tests).
    pub fn selected_model(&self) -> Option<&str> {
        self.selected.as_ref().map(|model| model.id.as_str())
    }

    /// Whether this change resumes the stored provider thread under the new
    /// model instead of handing over a summary and starting fresh.
    pub fn resumes_natively(&self) -> bool {
        self.native_continuation
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
        self.create_chat_id(harness, model, title)?;
        self.state_snapshot()
    }

    /// Create a direct chat and return the UUID used for that exact insert.
    pub fn create_chat_id(
        &self,
        harness: &Harness,
        model: Option<&str>,
        title: Option<&str>,
    ) -> Result<String, BridgeError> {
        let adapter_id = store::harness_name(harness);
        let id = Uuid::new_v4().to_string();
        let cwd = self.chat_scratch_dir(&id);
        let label = chat_label(title);
        let mut db = self.db.lock().unwrap();
        let transaction = db.transaction()?;
        transaction.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth) VALUES(?1,NULL,?2,?3,'idle','estimated',?4,'direct',?5,?6,0)",
            params![id, adapter_id, label, model, title, cwd.to_string_lossy()],
        )?;
        store::event(
            &transaction,
            "chat",
            "chat.created",
            &id,
            &format!("Created chat {label}"),
        )?;
        transaction.commit()?;
        Ok(id)
    }

    /// Create a source-scoped aside and carry its handoff in the same
    /// transaction. The exact inserted id is returned to prevent races.
    ///
    /// Context rides the best channel available. When the aside targets the
    /// same Codex harness as its source, the source has a stored provider
    /// thread, that thread is not mid-turn, and the installed Codex exposes
    /// `thread/fork`, the aside is created already pointed at a NATIVE FORK of
    /// the source thread: its first cold start forks the source into a new
    /// thread, so the aside reads the parent conversation's full provider
    /// history and every write lands on the fork — the parent conversation is
    /// never appended to, provider-side or forest-side. Anything else falls
    /// back to the projected handoff brief (stored checkpoint context), which
    /// is also the fallback ladder's first stop when a fork fails at start
    /// time.
    pub fn create_aside_chat_id(
        &self,
        source_session_id: &str,
        harness: &Harness,
        model: Option<&str>,
        title: Option<&str>,
    ) -> Result<(String, bool, bool), BridgeError> {
        let adapter_id = store::harness_name(harness);
        let id = Uuid::new_v4().to_string();
        let label = chat_label(title);
        // Discovered before the database lock: the first call may shell out to
        // the Codex binary to read its schema, and holding the core lock under
        // a subprocess would stall every other session operation.
        let codex_can_fork = adapter_id.as_ref() == "codex" && crate::codex_adapter::supports_native_fork();
        let mut db = self.db.lock().unwrap();
        let transaction = db.transaction()?;
        let (workspace_id, source_cwd, source_harness, source_thread, source_turn): (
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
        ) = transaction.query_row(
            "SELECT workspace_id,cwd,harness,provider_session_id,active_turn_id FROM sessions WHERE id=?1",
            params![source_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?.ok_or_else(|| BridgeError::Invalid("Aside source session does not exist".into()))?;
        let cwd = source_cwd.unwrap_or_else(|| self.chat_scratch_dir(&id).to_string_lossy().into_owned());
        // A fork must target the same harness that owns the thread, on a
        // source with a resumable thread id and no turn in flight.
        let native_fork = aside_fork_eligible(
            adapter_id.as_ref(),
            &source_harness,
            source_thread.as_deref(),
            source_turn.is_some(),
            codex_can_fork,
        );
        let insert_thread = native_fork.then(|| source_thread.clone()).flatten();
        match insert_thread.as_deref() {
            Some(thread) => {
                transaction.execute(
                    "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth,provider_session_id,continuation_fidelity) VALUES(?1,?2,?3,?4,'idle','estimated',?5,'direct',?6,?7,0,?8,'native')",
                    params![id, workspace_id, adapter_id, label, model, title, cwd, thread],
                )?;
            }
            None => {
                transaction.execute(
                    "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth) VALUES(?1,?2,?3,?4,'idle','estimated',?5,'direct',?6,?7,0)",
                    params![id, workspace_id, adapter_id, label, model, title, cwd],
                )?;
            }
        }
        store::event(&transaction, "chat", "aside.created", &id, &format!("Created aside {label} from {source_session_id}"))?;
        if native_fork {
            restoration::set_head_state(
                &transaction,
                &id,
                RestorationMode::NativeFork,
                ResumeEligibility::Native,
                source_thread.as_deref(),
            )?;
            store::event(
                &transaction,
                "chat",
                "aside.native_fork",
                &id,
                &format!("Aside {label} will fork Codex thread {thread} on its first turn", thread = source_thread.as_deref().unwrap_or_default()),
            )?;
        }
        let carried = handoff::carry_brief_in_transaction(&transaction, &id, source_session_id)?;
        // The brief is carried either way: it is the fork's checkpoint
        // fallback if the native fork fails at start time, and the whole
        // context channel when no fork applies.
        if !native_fork {
            transaction.execute(
                "UPDATE sessions SET continuation_fidelity=?2 WHERE id=?1",
                params![id, if carried { "projected_at_boundary" } else { "native" }],
            )?;
        }
        transaction.commit()?;
        Ok((id, carried, native_fork))
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
                // The class that used to leak permanently: an orchestrator
                // checkout was named only by `sessions.cwd`, which no cleanup
                // path consulted, so nothing could find it — let alone reclaim
                // it. It gets an owned inventory row here, in the same
                // transaction as the session that owns it.
                crate::worktree_registry::register(
                    &transaction,
                    &crate::worktree_registry::NewWorktree {
                        kind: crate::worktree_registry::KIND_ORCHESTRATOR.to_owned(),
                        repo_root: plan.workspace_path.clone().unwrap_or_default(),
                        path: created.path.to_string_lossy().to_string(),
                        branch: Some(created.branch.clone()),
                        owner_session_id: Some(plan.session_id.clone()),
                        owner_workspace_id: Some(plan.workspace_id.clone()),
                        base_commit: None,
                    },
                )?;
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

    /// Validate a chat model switch, allowing provider defaults before discovery. Returns
    /// `None` when the chat already runs the requested harness/model.
    pub fn plan_chat_model_change(
        &self,
        session_id: &str,
        harness: &Harness,
        model: Option<&str>,
    ) -> Result<Option<ChatModelChange>, BridgeError> {
        let adapter_id = store::harness_name(harness);
        if !agent_config::is_harness_enabled(&self.db.lock().unwrap(), &adapter_id) {
            return Err(BridgeError::Invalid(format!(
                "{} is disabled in Settings",
                harness.label()
            )));
        }
        let (kind, previous_harness, previous_model, provider_session_id, active_turn_id, parent_session_id): (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self.db.lock().unwrap().query_row(
            "SELECT kind,harness,model,provider_session_id,active_turn_id,parent_session_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
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
        let selected = if let Some(requested) = model.filter(|value| !value.trim().is_empty()) {
            descriptor
                .models
                .iter()
                .find(|option| option.id.eq_ignore_ascii_case(requested.trim()))
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "{} does not offer model {requested}",
                        descriptor.label
                    ))
                })?
        } else if descriptor.models.is_empty() {
            // The first user-owned session may be what publishes the catalog.
            None
        } else {
            descriptor
                .models
                .iter()
                .find(|option| Some(&option.id) == descriptor.default_model.as_ref())
                .or_else(|| descriptor.models.iter().find(|option| option.tier == default_tier && option.default_for_tier))
                .or_else(|| {
                    descriptor
                        .models
                        .iter()
                        .find(|option| option.tier == default_tier)
                })
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "{} has no {} model",
                        descriptor.label,
                        default_tier.as_str()
                    ))
                })?
        };
        if let Some(selected) = &selected {
            if !selected.available || !selected.compatible {
                return Err(BridgeError::Invalid(format!("{} is not available for this session", selected.label)));
            }
        }
        if previous_harness == adapter_id && previous_model.as_deref() == selected.as_ref().map(|model| model.id.as_str())
        {
            return Ok(None);
        }
        // Native continuation is an eligibility check, not a harness
        // comparison: `start_chat` gates `RestorationPlan::Native` on the
        // adapter's resume support and a stored thread id, so the switch must
        // apply the same gate before skipping the summary and claiming the
        // conversation continues. Anything else takes the handover path.
        let native_continuation = previous_harness == adapter_id
            && self.adapter_registry.supports_native_resume(adapter_id.as_ref())
            && provider_session_id.is_some();
        Ok(Some(ChatModelChange {
            session_id: session_id.to_owned(),
            adapter_id: adapter_id.into_owned(),
            kind,
            previous_harness,
            previous_model,
            tier: selected.as_ref().map_or(default_tier, |model| model.tier),
            selected,
            native_continuation,
        }))
    }

    /// Remove and stop a session's live adapter runtime, if any. Blocking —
    /// hosts place this on their blocking pool.
    pub fn stop_session_adapter(&self, session_id: &str, reason: adapters::ShutdownReason) {
        self.deactivate_reader_launch(session_id);
        if let Some(mut runtime) = self.adapters.lock().unwrap().remove(session_id) {
            runtime.stop(reason);
        }
        // A model switch's outgoing runtime lives outside the adapter map while
        // it summarises; stopping the session must not leave it running.
        crate::switch_summary::stop_for_session(self, session_id, reason);
    }

    /// Stop an adapter for a clean host exit and durably retire its process
    /// claim. A reader whose runtime has been removed skips its usual exit
    /// cleanup, so the host must finish it here; otherwise an idle, completed
    /// turn is misreported as an orphan failure at the next launch.
    pub fn shutdown_session_adapter(&self, session_id: &str) -> Result<(), BridgeError> {
        let _lifecycle = self.claim_session_lifecycle(session_id, "app shutdown")?;
        self.deactivate_reader_launch(session_id);
        // A model switch can leave only an outgoing summary, with no attached
        // runtime or durable process claim. Cancel and stop it before any early
        // return, while retaining its launch identity until the reader exits.
        let had_detached_summary = crate::switch_summary::is_detached(self, session_id);
        crate::switch_summary::stop_for_session(
            self,
            session_id,
            adapters::ShutdownReason::AppShutdown,
        );
        let runtime = self.adapters.lock().unwrap().remove(session_id);
        // Provider shutdown can block and its reader may need the adapter map.
        // Hold neither the map nor the database while waiting for the process.
        if let Some(mut runtime) = runtime {
            runtime.stop(adapters::ShutdownReason::AppShutdown);
        } else {
            // A reader removes its runtime before retiring the durable claim.
            // Let that exit cleanup finish if its process is still alive; if
            // the process has already exited, the host can settle the claim.
            // Never discard ownership of a process still known to be running.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            loop {
                let claim: Option<(u32, Option<String>)> = self.db.lock().unwrap().query_row(
                    "SELECT adapter_pid,adapter_process_identity FROM sessions WHERE id=?1 AND adapter_pid IS NOT NULL",
                    params![session_id], |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional()?;
                let Some((pid, expected_identity)) = claim else {
                    if had_detached_summary {
                        break;
                    }
                    return Ok(());
                };
                let live_identity = adapters::process_identity(pid);
                if live_identity.is_none() || (expected_identity.is_some() && live_identity != expected_identity) {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    return Err(BridgeError::Adapter(format!(
                        "provider {pid} still has a live process claim after its runtime exited"
                    )));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        let db = self.db.lock().unwrap();
        let transaction = db.unchecked_transaction()?;
        let (previous_status, active_turn): (String, bool) = transaction.query_row(
            "SELECT status,active_turn_id IS NOT NULL FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let status = match previous_status.as_str() {
            "completed" | "cancelled" | "failed" | "stopped" => previous_status.as_str(),
            _ => "stopped",
        };
        session_supervisor::SessionSupervisor::clear_adapter_process(&transaction, session_id)?;
        transaction.execute(
            "UPDATE sessions SET status=?2,active_turn_id=NULL,
             ended_at=CASE WHEN status=?2 THEN ended_at ELSE ?3 END WHERE id=?1",
            params![session_id, status, chrono::Utc::now().to_rfc3339()],
        )?;
        session_forest::append_in_transaction(
            &transaction,
            session_id,
            session_forest::EntryKind::SessionStatus,
            serde_json::json!({
                "status": status,
                "reason": adapters::ShutdownReason::AppShutdown.as_str(),
                "interrupted": active_turn,
            }),
        ).map_err(|error| BridgeError::Invalid(error.to_string()))?;
        store::event(
            &transaction,
            "adapter",
            "session.shutdown",
            session_id,
            adapters::ShutdownReason::AppShutdown.as_str(),
        )?;
        // Worker lifecycle/result recovery remains separate: shutting down an
        // unfinished worker must never manufacture a successful result.
        session_supervisor::SessionSupervisor::reconcile_workspace_statuses(&transaction)?;
        transaction.commit()?;
        self.events.publish(crate::events::CoreEvent::StateChanged);
        Ok(())
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
        tail: Option<bool>,
    ) -> Result<Vec<AgentEvent>, BridgeError> {
        if after_sequence < 0 {
            return Err(BridgeError::Invalid(
                "afterSequence must be non-negative".into(),
            ));
        }
        if tail == Some(true) && after_sequence != 0 {
            return Err(BridgeError::Invalid(
                "tail replay requires afterSequence=0".into(),
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
        if tail == Some(true) {
            store::session_events_tail(&db, session_id, limit)
        } else {
            store::session_events_after(&db, session_id, after_sequence, limit)
        }
    }

    /// The change token for one session's forest, holding the store lock only
    /// for a handful of indexed lookups.
    pub fn session_forest_digest(&self, session_id: &str) -> Result<String, BridgeError> {
        let db = self.db.lock().unwrap();
        session_forest_digest(&db, session_id)
    }

    /// The bounded, source-labelled context breakdown for one session. Live
    /// adapter observations are gathered under the adapter lock, then merged
    /// with store-derived sources without holding the store lock.
    pub fn context_breakdown(
        &self,
        session_id: &str,
    ) -> Result<bridge_protocol::messages::ContextBreakdownResult, BridgeError> {
        let inventories = self.session_context_inventories(session_id);
        let db = self.db.lock().unwrap();
        crate::context_breakdown::context_breakdown(&db, session_id, &inventories)
    }

    /// The cheap half of breakdown polling; same inputs as
    /// [`BridgeCore::context_breakdown`], including the live observations —
    /// the digest reads this session's own inventories so a turn elsewhere
    /// never invalidates it.
    pub fn context_breakdown_digest(
        &self,
        session_id: &str,
    ) -> Result<bridge_protocol::messages::ContextBreakdownDigestResult, BridgeError> {
        let inventories = self.session_context_inventories(session_id);
        let db = self.db.lock().unwrap();
        crate::context_breakdown::context_breakdown_digest_result(
            &db,
            session_id,
            &inventories,
        )
    }

    /// One session's live adapter observations, read under the adapter lock
    /// and released before any store work.
    fn session_context_inventories(
        &self,
        session_id: &str,
    ) -> Vec<crate::context_inventory::AdapterContextInventory> {
        let adapters = self.adapters.lock().unwrap();
        adapters
            .get(session_id)
            .map(|runtime| runtime.context_inventory())
            .unwrap_or_default()
    }

    /// Interrupt the session's active turn on its live adapter runtime.
    pub fn interrupt_turn(&self, session_id: &str) -> Result<(), BridgeError> {
        let adapters = self.adapters.lock().unwrap();
        let runtime = adapters.get(session_id).ok_or_else(|| {
            BridgeError::Invalid("Structured adapter session is not running".into())
        })?;
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
            compaction_controller::CONVERSATION_KINDS.contains(&entry.kind.as_str())
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
        match compaction_controller::CompactionController::begin(
            &db,
            session_id,
            compaction_controller::CompactionReason::Manual,
            tokens,
        )? {
            compaction_controller::CompactionStart::Ready(prompt) => Ok(prompt),
            compaction_controller::CompactionStart::AlreadyPending => Err(BridgeError::Invalid(
                "Compaction is already pending".into(),
            )),
            // Reporting this as "already pending" sent the user looking for a
            // checkpoint that was never requested.
            compaction_controller::CompactionStart::ProviderLimited { harness, until } => {
                Err(BridgeError::Invalid(format!(
                    "{harness} is out of quota until {until}. Compaction needs a provider turn, so it cannot run until then; your conversation history is intact."
                )))
            }
        }
    }

    /// Ask the outgoing provider to summarise the conversation before a model
    /// switch tears it down. The request rides the existing validated
    /// compaction pipeline: `compaction.requested` entry, one internal turn,
    /// typed parse/validate with a single repair retry. Only a hot, idle chat
    /// with meaningful history *and* enough of it to be worth compressing can be
    /// asked; anything else — cold process, active turn, in-flight compaction,
    /// empty conversation, or a branch under
    /// [`SWITCH_SUMMARY_MIN_TOKENS`] — returns `None` and the switch falls back
    /// to the mechanical projection, which loses nothing at that size.
    pub fn plan_switch_summary(
        &self,
        session_id: &str,
    ) -> Result<Option<SwitchSummaryRequest>, BridgeError> {
        if self.adapters.lock().unwrap().get(session_id).is_none() {
            return Ok(None);
        }
        let db = self.db.lock().unwrap();
        let status: String = db.query_row(
            "SELECT status FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if matches!(status.as_str(), "working" | "waiting" | "checkpointing") {
            return Ok(None);
        }
        let branch = session_forest::SessionForest::new(&db)
            .active_branch(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let meaningful = branch.iter().any(|entry| {
            compaction_controller::CONVERSATION_KINDS.contains(&entry.kind.as_str())
        });
        if !meaningful {
            return Ok(None);
        }
        // Never disturb an in-flight compaction; its outcome is already owned.
        if compaction_controller::CompactionController::pending(&db, session_id)?.is_some() {
            return Ok(None);
        }
        // The floor reads the *conversation* estimate, not the full branch: a
        // fresh chat's compiled prompt alone measures thousands of tokens, and
        // a greeting was clearing the floor on the strength of context it did
        // not write. The checkpoint request below still records the full
        // estimate, because `tokensBefore` describes context pressure, not
        // conversation size. A floor under the "meaningful work" gate above,
        // not a replacement for it.
        if compaction_controller::conversation_token_estimate(&db, session_id)?
            < SWITCH_SUMMARY_MIN_TOKENS
        {
            return Ok(None);
        }
        let tokens =
            compaction_controller::active_token_estimate(&db, session_id)?;
        // Background: the request is answered by the outgoing runtime *after*
        // the switch commits, on a reader that shares this session id with the
        // incoming model, so the live reader must never treat it as its own.
        let Some(prompt) = compaction_controller::CompactionController::begin_background(
            &db,
            session_id,
            compaction_controller::CompactionReason::BeforeDowngrade,
            tokens,
        )?
        .prompt()
        else {
            return Ok(None);
        };
        // The request's own sequence scopes outcome reads to THIS request, so
        // a terminal from an older checkpoint is never mistaken for its answer.
        let after_sequence: i64 = db.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id=?1 AND kind='compaction.requested'",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(Some(SwitchSummaryRequest {
            session_id: session_id.to_owned(),
            prompt,
            after_sequence,
        }))
    }

    /// Cancel an outstanding summary request so normal replies are never
    /// misparsed as checkpoint output. This is the timeout path's cleanup and
    /// the delivery-failure path's cleanup.
    pub fn cancel_switch_summary(
        &self,
        session_id: &str,
        reason: &str,
        attempt: u8,
    ) -> Result<(), BridgeError> {
        compaction_controller::CompactionController::record_failure(
            &self.db.lock().unwrap(),
            session_id,
            reason,
            attempt,
        )
    }

    /// Read what became of a summary request without blocking. Terminal
    /// entries are scoped to sequences after this request's own
    /// `compaction.requested` row, so the verdict always describes *this*
    /// request — never a previous checkpoint's leftover.
    pub fn switch_summary_outcome(
        &self,
        request: &SwitchSummaryRequest,
    ) -> Result<SwitchSummaryOutcome, BridgeError> {
        let db = self.db.lock().unwrap();
        if compaction_controller::CompactionController::pending(&db, &request.session_id)?.is_some()
        {
            return Ok(SwitchSummaryOutcome::Pending);
        }
        let kind: Option<String> = db
            .query_row(
                "SELECT kind FROM session_entries WHERE session_id=?1 AND sequence > ?2 AND kind IN ('compaction','compaction.failed','checkpoint') ORDER BY sequence DESC LIMIT 1",
                params![request.session_id, request.after_sequence],
                |row| row.get(0),
            )
            .optional()?;
        match kind.as_deref() {
            // `checkpoint` alone is a late background landing (the new model had
            // already spoken, so no boundary moved); paired, it precedes the
            // `compaction` this query finds first. Either way the request was
            // answered.
            Some("compaction") | Some("checkpoint") => Ok(SwitchSummaryOutcome::Summarised),
            _ => Ok(SwitchSummaryOutcome::Failed),
        }
    }

    /// Wait for a torn-down session's turn bookkeeping to settle, clearing it
    /// if the dead reader never will. The switch's own summary turn writes
    /// `active_turn_id` asynchronously when the provider acknowledges the
    /// turn; once the adapter is stopped, `turn.completed` can never arrive,
    /// and the reader's exit sweep races the commit. Left alone, that residue
    /// trips the commit's `active_turn_id IS NULL` revision check and the
    /// switch fails against its own machinery.
    ///
    /// Only a session with **no live runtime** is ever touched: with the
    /// process alive a turn may still finish on its own, and the commit guard
    /// keeps its full pessimism. With the process dead, any recorded turn is
    /// unfinishable by construction, so clearing it restores the idle row the
    /// plan verified.
    pub fn settle_adapterless_turn_state(
        &self,
        session_id: &str,
        budget: std::time::Duration,
    ) -> Result<(), BridgeError> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            let active: Option<String> = self.db.lock().unwrap().query_row(
                "SELECT active_turn_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )?;
            if active.is_none() {
                return Ok(());
            }
            if self.adapters.lock().unwrap().contains_key(session_id) {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        self.db.lock().unwrap().execute(
            "UPDATE sessions SET active_turn_id=NULL WHERE id=?1",
            params![session_id],
        )?;
        Ok(())
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
    ///
    /// Returns whether a live session was asked. When none was, the caller
    /// falls back to Codex's own on-disk record, because "no chat is open" is
    /// not the same as "no limits exist".
    pub fn request_codex_usage(&self) -> Result<bool, BridgeError> {
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
                // Reaching a registered runtime is not the same as it
                // answering: a session whose request writer has closed rejects
                // here. Reporting that as asked would suppress the disk
                // fallback in exactly the case that needs it, so only a
                // successful request counts, and anything else tries the next
                // session before falling through.
                if runtime.read_usage().is_ok() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Publish Codex's rate limits from its rollout history, off-thread. Used
    /// when no live session can be asked: the numbers are account-wide, so the
    /// last ones Codex wrote are still the current ones until they reset.
    fn spawn_codex_usage_from_disk(&self) {
        let events = self.events.clone();
        std::thread::spawn(move || {
            let sessions_dir = crate::usage_import::SourceEnv::from_process().codex_sessions_dir();
            publish_codex_usage_from_disk(&events, &sessions_dir, chrono::Utc::now().timestamp());
        });
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
        // answers for the whole account. With no session running, read the
        // same numbers from Codex's rollouts rather than showing nothing —
        // waiting for the user to open a chat before admitting a limit exists
        // is what made the meter look broken on a cold start.
        if !self.request_codex_usage()? {
            self.spawn_codex_usage_from_disk();
        }
        Ok(())
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
            change.selected_model(),
            change.tier,
            (&change.previous_harness, change.previous_model.as_deref()),
            change.resumes_natively(),
        )? != 1
        {
            return Err(BridgeError::Invalid(
                "The chat could not be updated because it changed while the switch was in flight"
                    .into(),
            ));
        }
        let resumes_natively = change.resumes_natively();
        if resumes_natively {
            // The stored thread is still this agent's own, so the next turn
            // resumes it under the new model. `set_head_state` coalesces a
            // `None` id, which is what keeps the identity here.
            restoration::set_head_state(
                &transaction,
                session_id,
                RestorationMode::Native,
                ResumeEligibility::Native,
                None,
            )?;
        } else {
            // A different agent cannot resume this thread, and leaving the id
            // on the head surfaced a dead thread in the forest snapshot long
            // after `sessions.provider_session_id` was cleared.
            restoration::clear_head_state_for_new_provider(
                &transaction,
                session_id,
                RestorationMode::Fresh,
                ResumeEligibility::Fresh,
            )?;
        }
        // Say what the next provider will actually inherit. The projection is
        // read before any switch bookkeeping appends, so it describes exactly
        // what start_chat's cold path will inject. A natively resumed change
        // inherits everything through the provider thread, so there is nothing
        // to project and nothing to claim.
        let carried = (!resumes_natively)
            .then(|| carried_context(&transaction, session_id))
            .flatten();
        let subject = if change.kind == "orchestrator" {
            "Orchestrator"
        } else {
            "Chat"
        };
        let detail = if resumes_natively {
            format!(
                "{subject} model changed from {} to {}. The conversation continues on the same {} session.",
                change.previous_model.as_deref().unwrap_or("automatic"),
                change.selected_model().unwrap_or("default"),
                change.adapter_id,
            )
        } else {
            let mut carry_note = carried
                .as_ref()
                .map(CarriedContext::describe)
                .unwrap_or_else(|| "no context carried (summary unavailable)".to_owned());
            // `is_detached` asks for a *live* summary: a retired entry (its
            // runtime already stopped, only its reader still draining) is no
            // longer preparing anything and must not be claimed as such.
            if crate::switch_summary::is_detached(self, session_id) {
                // A background summary from the outgoing model is on its way; the
                // mechanical projection above is what the new model inherits now.
                carry_note.push_str("; a handoff summary from the previous model is being prepared in the background");
            }
            format!(
                "{subject} runtime changed from {}/{} to {}/{}. The next message starts a fresh provider session; {}.",
                change.previous_harness,
                change.previous_model.as_deref().unwrap_or("automatic"),
                change.adapter_id,
                change.selected_model().unwrap_or("default"),
                carry_note,
            )
        };
        store::event(
            &transaction,
            "chat",
            "session.model_changed",
            session_id,
            &detail,
        )?;
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
                data: {
                    let mut obj = serde_json::json!({
                        "previousHarness": change.previous_harness,
                        "previousModel": change.previous_model,
                        "harness": change.adapter_id,
                        "model": change.selected_model(),
                        "modelLabel": change.selected.as_ref().map(|model| model.label.as_str()).unwrap_or("Provider default"),
                        "tier": change.tier,
                        // The milestone marker every model change carries, and
                        // the separate claim about whether the provider session
                        // survived it. They used to be the same field, so making
                        // the claim truthful would have hidden the row.
                        "modelChanged": true,
                        "freshProviderSession": !resumes_natively,
                    });
                    if let Some(carried) = carried {
                        obj["carriedContext"] = serde_json::json!({
                            "summary": carried.summary,
                            "decisions": carried.decisions,
                            "filesTouched": carried.files_touched,
                            "recentEntries": carried.recent_entries,
                        });
                    }
                    obj
                },
            },
            &serde_json::json!({"source": "user-selection"}),
        )?;
        transaction.commit()?;
        self.events
            .publish(crate::events::CoreEvent::Agent(event.clone()));
        Ok(event)
    }
}

/// Read Codex's rate limits from `sessions_dir` and publish them on the
/// account-usage channel. Returns whether any live window was found.
///
/// A frame is published either way. When nothing current is on disk the
/// payload is empty, which clears the provider rather than leaving whatever
/// was last shown in place: a session that reported 95% and then ended would
/// otherwise keep that 95% on screen forever once its window reset, which is
/// the same stale reading the expiry pruning exists to prevent.
///
/// Split out from the spawning caller so the fallback is testable against a
/// fixture directory rather than the real `CODEX_HOME`.
pub(crate) fn publish_codex_usage_from_disk(
    events: &crate::events::EventBus,
    sessions_dir: &Path,
    now_unix: i64,
) -> bool {
    let found = crate::meter_sources::codex_rate_limits(sessions_dir, now_unix);
    let live = found.is_some();
    events.publish(crate::events::CoreEvent::AccountUsage {
        provider: "codex".into(),
        rate_limits: found.unwrap_or_else(|| serde_json::json!({})),
    });
    live
}

fn chat_label(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New chat")
        .to_string()
}

/// An opaque change token composed from the monotonic columns behind every
/// store-derived field of [`SessionForestSnapshot`]. Equal tokens mean the
/// snapshot would be byte-identical except for repository divergence, which
/// lives outside the store; a token may change without a visible snapshot
/// change (worker tables are folded in globally), and that costs one full
/// fetch — the same work every poll used to do. Entries never need loading:
/// the forest is append-only, so the max sequence plus the head row cover it.
pub fn session_forest_digest(db: &Connection, session_id: &str) -> Result<String, BridgeError> {
    // The same existence check the snapshot performs, so both surfaces agree
    // on unknown sessions. Direct chats carry no workspace.
    let _workspace: Option<String> = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let digest: String = db.query_row(
        "SELECT 'v1'
            ||':'||COALESCE((SELECT MAX(sequence) FROM session_entries WHERE session_id=?1),0)
            ||':'||COALESCE((SELECT updated_at FROM session_heads WHERE session_id=?1),'')
            ||':'||COALESCE((SELECT active_entry_id FROM session_heads WHERE session_id=?1),'')
            ||':'||(SELECT status||'/'||COALESCE(active_turn_id,'') FROM sessions WHERE id=?1)
            ||':'||COALESCE((SELECT MAX(id) FROM usage_ledger),0)
            ||':'||(SELECT COUNT(*)||'/'||COALESCE(MAX(updated_at),'') FROM worker_leases)
            ||':'||(SELECT COUNT(*)||'/'||COALESCE(MAX(updated_at),'') FROM worker_runtime)
            ||':'||(SELECT COUNT(*)||'/'||COALESCE(MAX(sequence),0)||'/'||COALESCE(MAX(updated_at),'') FROM worker_queue)
            ||':'||COALESCE((SELECT MAX(id) FROM events),0)
            ||':'||(SELECT COUNT(*)||'/'||COALESCE(MAX(updated_at),'') FROM worker_worktree_adoptions)
            ||':'||COALESCE((SELECT rowid||'/'||status FROM eval_attempts WHERE session_id=?1 ORDER BY started_at DESC,rowid DESC LIMIT 1),'')",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(digest)
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
    // Imported and other workspace-less direct chats carry a NULL
    // workspace_id; the workspace-scoped queries below simply return empty
    // results for a key that matches no real workspace row.
    let workspace_id: Option<String> = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let workspace_id = workspace_id.unwrap_or_default();
    let config = policy::PolicyConfig::default();
    // Bounded on purpose. The untrimmed read of a long chat reached 129 MB of
    // payload, which is both slow to parse twice (here and in the renderer)
    // and past the daemon's frame ceiling, so it failed the open outright.
    let window = store::session_entry_window(db, session_id, store::SNAPSHOT_ENTRY_WINDOW)?;
    let entries = window.entries;
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
    let entries_returned = entries.len() as i64;
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
        reasons: store::workspace_reason_events(db, &workspace_id, store::SNAPSHOT_REASON_WINDOW)?,
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
        entry_window: crate::model::SessionEntryWindowSummary {
            returned: entries_returned,
            total: window.total,
            trimmed_payloads: window.trimmed_payloads,
        },
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
    model: Option<&str>,
    tier: CapabilityTier,
    (previous_harness, previous_model): (&str, Option<&str>),
    resumes_natively: bool,
) -> Result<usize, BridgeError> {
    // A harness change is a different agent, so the backend binding goes the
    // way of the provider session id: `read_binding` composes the binding's
    // agent from the harness column, and a stale backend id left under the new
    // harness reads as "codex was served by claude.agent-sdk" and refuses
    // every later launch. A same-harness model change keeps the binding — the
    // same agent resumes under the same backend, and the recorded version and
    // installation stay true.
    if previous_harness != adapter_id {
        return Ok(db.execute(
            "UPDATE sessions SET harness=?2,model=?3,requested_tier=?4,provider_session_id=NULL,backend_id=NULL,backend_version=NULL,backend_installation_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1 AND parent_session_id IS NULL AND kind IN ('direct','orchestrator') AND harness=?5 AND model IS ?6 AND active_turn_id IS NULL",
            params![session_id, adapter_id, model, tier.as_str(), previous_harness, previous_model],
        )?);
    }
    if resumes_natively {
        // The provider session survives a model change the next turn can
        // actually resume. The cache miss is unavoidable — provider caches are
        // per model — but the conversation is not: the Claude Agent SDK sets
        // `resume` and `model` independently, and Codex `thread/resume`
        // carries `model`. Clearing the id here is what forced a Sonnet→Opus
        // switch to restart from an 8 KB projection as if it had crossed
        // harnesses.
        return Ok(db.execute(
            "UPDATE sessions SET harness=?2,model=?3,requested_tier=?4,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1 AND parent_session_id IS NULL AND kind IN ('direct','orchestrator') AND harness=?5 AND model IS ?6 AND active_turn_id IS NULL",
            params![session_id, adapter_id, model, tier.as_str(), previous_harness, previous_model],
        )?);
    }
    // Same harness, but nothing the next turn can resume — an adapter without
    // native resume, or a chat that never started a thread. The dead id is
    // cleared exactly as a harness change clears it, so nothing later mistakes
    // it for a resumable session; the backend binding stays, since the same
    // agent still serves the session.
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
        BridgeError::Invalid("Connect a Git repository before creating an isolated worktree".into())
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
        // Any harness the registry actually runs may be the configured
        // orchestrator, rather than a list that has to be edited every time an
        // adapter is added — a named harness with no adapter behind it (the
        // `bridge` pseudo-harness) has nothing to resolve a model against, and
        // an adapter that is registered but unavailable falls through on
        // `resolve_model` below exactly as it did before.
        if agent.enabled
            && descriptors
                .iter()
                .any(|descriptor| descriptor.id == agent.harness)
        {
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
                        model: Some(resolution.actual_model),
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
            model: Some(resolution.actual_model),
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
                model: Some(resolution.actual_model),
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
    struct StubAdapter {
        catalog_empty: bool,
        expected_model: Option<&'static str>,
    }
    impl adapters::HarnessAdapter for StubAdapter {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            let mut descriptor = AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "codex".into(),
                label: "Codex".into(),
                available: true,
                auth_state: crate::model::AuthState::Unknown,
                version: None,
                capabilities: Vec::new(),
                unavailable_reason: None,
                models: vec![
                    ModelOption {
                        id: "stub-standard".into(),
                        label: "Stub Standard".into(),
                        tier: CapabilityTier::Standard,
                        available: true,
                        compatible: true,
                        lifecycle: crate::model::ModelLifecycle::Stable,
                        source: crate::model::ModelCatalogSource::CuratedFallback,
                        supported_effort_levels: vec!["high".into(), "ultra".into()],
                        default_for_tier: true,
                    },
                    ModelOption {
                        id: "stub-fast".into(),
                        label: "Stub Fast".into(),
                        tier: CapabilityTier::Fast,
                        available: true,
                        compatible: true,
                        lifecycle: crate::model::ModelLifecycle::Stable,
                        source: crate::model::ModelCatalogSource::CuratedFallback,
                        supported_effort_levels: Vec::new(),
                        default_for_tier: true,
                    },
                ],
                default_model: None,
                model_catalog: crate::model::ModelCatalogDiagnostics::curated(),
            };
            if self.catalog_empty {
                descriptor.id = "cursor".into();
                descriptor.label = "Cursor".into();
                descriptor.models.clear();
            }
            descriptor
        }
        fn start(
            &self,
            request: adapters::StartRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            if self.catalog_empty {
                assert_eq!(request.model, None, "the provider must choose its own default");
                assert_eq!(request.effort, None, "do not carry another provider's effort");
            } else if let Some(expected) = self.expected_model {
                assert_eq!(request.model, Some(expected));
            }
            Err(BridgeError::Adapter("stub adapter cannot start".into()))
        }
        fn resume(
            &self,
            _: adapters::ResumeRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
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
        registry
            .register(Box::new(StubAdapter {
                catalog_empty: false,
                expected_model: None,
            }))
            .unwrap();
        registry
            .register(Box::new(StubAdapter {
                catalog_empty: true,
                expected_model: None,
            }))
            .unwrap();
        core.adapter_registry = std::sync::Arc::new(registry);
        (scratch, core)
    }

    /// The codex stub, plus a resume-capable harness, so a switch that can
    /// natively resume has somewhere to run. `StubAdapter` deliberately
    /// reports no resume support — production Cursor and Grok do the same —
    /// which is what makes the fallback path testable beside the native one.
    struct ResumableStubAdapter;
    impl adapters::HarnessAdapter for ResumableStubAdapter {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "claude".into(),
                label: "Claude".into(),
                available: true,
                auth_state: crate::model::AuthState::Unknown,
                version: None,
                capabilities: Vec::new(),
                unavailable_reason: None,
                models: vec![
                    ModelOption {
                        id: "stub-sonnet".into(),
                        label: "Stub Sonnet".into(),
                        tier: CapabilityTier::Fast,
                        available: true,
                        compatible: true,
                        lifecycle: crate::model::ModelLifecycle::Stable,
                        source: crate::model::ModelCatalogSource::CuratedFallback,
                        supported_effort_levels: Vec::new(),
                        default_for_tier: true,
                    },
                    ModelOption {
                        id: "stub-opus".into(),
                        label: "Stub Opus".into(),
                        tier: CapabilityTier::Standard,
                        available: true,
                        compatible: true,
                        lifecycle: crate::model::ModelLifecycle::Stable,
                        source: crate::model::ModelCatalogSource::CuratedFallback,
                        supported_effort_levels: Vec::new(),
                        default_for_tier: true,
                    },
                ],
                default_model: None,
                model_catalog: crate::model::ModelCatalogDiagnostics::curated(),
            }
        }
        fn start(
            &self,
            _: adapters::StartRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("stub adapter cannot start".into()))
        }
        fn resume(
            &self,
            _: adapters::ResumeRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("stub adapter cannot resume".into()))
        }
        fn supports_native_resume(&self) -> bool {
            true
        }
        fn normalize(&self, _: &Value) -> Vec<agent::NormalizedEvent> {
            Vec::new()
        }
    }

    fn resume_fixture() -> (tempfile::TempDir, BridgeCore) {
        let scratch = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        let mut registry = adapters::AdapterRegistry::empty();
        registry
            .register(Box::new(StubAdapter {
                catalog_empty: false,
                expected_model: None,
            }))
            .unwrap();
        registry.register(Box::new(ResumableStubAdapter)).unwrap();
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

    fn only_session_id(core: &BridgeCore) -> String {
        core.db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn welcome_chat_can_switch_to_an_undiscovered_provider_default_then_start() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        let plan = core.plan_workspace_session("w", false).unwrap();
        core.persist_workspace_session(plan, None).unwrap();
        let id = only_session_id(&core);
        let core = std::sync::Arc::new(core);

        // The welcome composer creates the configured orchestrator first, then
        // applies the user's Cursor/Default choice before starting any provider.
        crate::api::update_chat_model(&core, &id, &Harness::Cursor, None, None).unwrap();
        let stored: (String, Option<String>, Option<String>) = core.db.lock().unwrap()
            .query_row("SELECT harness,model,effort FROM sessions WHERE id=?1", params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(stored, ("cursor".into(), None, None));
        // Re-selecting Default is a no-op even before the catalog is discovered.
        crate::api::update_chat_model(&core, &id, &Harness::Cursor, None, None).unwrap();
        let error = crate::live_turn::start_chat(&core, id).unwrap_err();
        assert!(error.to_string().contains("stub adapter cannot start"), "{error}");
    }

    #[test]
    fn configured_model_does_not_override_an_undiscovered_provider_default() {
        for orchestrator in [false, true] {
            let (_scratch, core) = fixture();
            let id = if orchestrator {
                seed_workspace(&core, false);
                let plan = core.plan_workspace_session("w", false).unwrap();
                core.persist_workspace_session(plan, None).unwrap();
                only_session_id(&core)
            } else {
                core.create_chat_id(&Harness::Codex, Some("stub-fast"), None)
                    .unwrap()
            };
            {
                let db = core.db.lock().unwrap();
                let mut config = agent_config::harness_config(&db, "cursor").unwrap();
                config.default_model = Some("retired-cursor-model".into());
                agent_config::save_harness(&db, config).unwrap();
            }
            let core = std::sync::Arc::new(core);
            crate::api::update_chat_model(&core, &id, &Harness::Cursor, None, None).unwrap();
            // The stub asserts that the adapter receives None, despite the
            // saved setting, for both a direct chat and the welcome composer.
            let error = crate::live_turn::start_chat(&core, id).unwrap_err();
            assert!(
                error.to_string().contains("stub adapter cannot start"),
                "{error}"
            );
        }
    }

    #[test]
    fn configured_model_resolves_against_a_known_catalog_unless_the_session_has_a_pin() {
        for (stored, configured, expected) in [
            (None, "stub-fast", "stub-fast"),
            (Some("stub-standard"), "stub-fast", "stub-standard"),
            (None, "retired-model", "stub-standard"),
        ] {
            let (_scratch, mut core) = fixture();
            let mut registry = adapters::AdapterRegistry::empty();
            registry
                .register(Box::new(StubAdapter {
                    catalog_empty: false,
                    expected_model: Some(expected),
                }))
                .unwrap();
            core.adapter_registry = std::sync::Arc::new(registry);
            {
                let db = core.db.lock().unwrap();
                let mut config = agent_config::harness_config(&db, "codex").unwrap();
                config.default_model = Some(configured.into());
                agent_config::save_harness(&db, config).unwrap();
            }
            let id = core.create_chat_id(&Harness::Codex, stored, None).unwrap();
            let error = crate::live_turn::start_chat(&std::sync::Arc::new(core), id).unwrap_err();
            assert!(
                error.to_string().contains("stub adapter cannot start"),
                "{error}"
            );
        }
    }

    #[test]
    fn undiscovered_catalog_does_not_accept_unverified_model_or_effort_choices() {
        let (_scratch, core) = fixture();
        let id = core.create_chat_id(&Harness::Claude, None, None).unwrap();
        let core = std::sync::Arc::new(core);
        assert!(crate::api::update_chat_model(&core, &id, &Harness::Cursor, Some("invented"), None).is_err());
        assert!(crate::api::update_chat_model(&core, &id, &Harness::Cursor, None, Some("high")).is_err());
        let harness: String = core.db.lock().unwrap()
            .query_row("SELECT harness FROM sessions WHERE id=?1", params![id], |row| row.get(0)).unwrap();
        assert_eq!(harness, "claude", "a refused switch must leave the chat intact");
    }

    #[test]
    fn aside_creation_returns_exact_id_and_inherits_source_scope_atomically() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, true);
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,kind,depth) VALUES('source','w','codex','Source','idle','estimated','/tmp/sessions-demo','orchestrator',0)", [],
            ).unwrap();
            session_forest::SessionForest::new(&db).append(
                "source", session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Keep the repository context"}),
            ).unwrap();
        }
        let (aside_id, carried, native_fork) = core.create_aside_chat_id(
            "source", &Harness::Codex, Some("stub-standard"), Some("Check"),
        ).unwrap();
        // The stub codex adapter cannot fork (no app-server schema on the test
        // path), so the aside falls back to the projected handoff brief.
        assert!(carried);
        assert!(!native_fork);
        let db = core.db.lock().unwrap();
        let (workspace_id, cwd): (Option<String>, Option<String>) = db.query_row(
            "SELECT workspace_id,cwd FROM sessions WHERE id=?1", params![aside_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(workspace_id.as_deref(), Some("w"));
        assert_eq!(cwd.as_deref(), Some("/tmp/sessions-demo"));
        let handoffs: i64 = db.query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='handoff.brief'",
            params![aside_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(handoffs, 1);
    }

    #[test]
    fn fork_eligibility_belongs_to_codex_on_a_stored_idle_thread() {
        assert!(aside_fork_eligible(
            "codex", "codex", Some("parent-thread"), false, true,
        ));
        // The fork verb belongs to the thread's owner: a Claude aside from a
        // Codex chat reads the projected brief instead.
        assert!(!aside_fork_eligible(
            "claude", "codex", Some("parent-thread"), false, true,
        ));
        assert!(!aside_fork_eligible(
            "codex", "claude", Some("parent-thread"), false, true,
        ));
        // No stored thread, nothing to fork.
        assert!(!aside_fork_eligible("codex", "codex", None, false, true));
        assert!(!aside_fork_eligible("codex", "codex", Some("  "), false, true));
        // A turn in flight is a fork race: wait for the boundary.
        assert!(!aside_fork_eligible(
            "codex", "codex", Some("parent-thread"), true, true,
        ));
        // An installed Codex without thread/fork cannot fork.
        assert!(!aside_fork_eligible(
            "codex", "codex", Some("parent-thread"), false, false,
        ));
    }

    #[test]
    fn aside_fork_points_the_head_at_the_source_thread_for_a_native_start() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, true);
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,kind,depth,provider_session_id) VALUES('forksource','w','codex','Source','idle','estimated','/tmp/sessions-demo','orchestrator',0,'parent-thread')",
                [],
            ).unwrap();
            session_forest::SessionForest::new(&db).append(
                "forksource", session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Keep the repository context"}),
            ).unwrap();
        }
        let (aside_id, _carried, native_fork) = core.create_aside_chat_id(
            "forksource", &Harness::Codex, Some("stub-standard"), Some("Check"),
        ).unwrap();
        let can_fork_here = aside_fork_eligible(
            "codex", "codex", Some("parent-thread"), false,
            crate::codex_adapter::supports_native_fork(),
        );
        assert_eq!(native_fork, can_fork_here, "eligibility is decided by the installed codex, not the test");
        let db = core.db.lock().unwrap();
        let (provider_id, head_mode, fidelity): (Option<String>, String, String) = db.query_row(
            "SELECT s.provider_session_id, COALESCE(h.restoration_mode,'fresh'), s.continuation_fidelity FROM sessions s LEFT JOIN session_heads h ON h.session_id=s.id WHERE s.id=?1",
            params![aside_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        if native_fork {
            assert_eq!(provider_id.as_deref(), Some("parent-thread"));
            assert_eq!(head_mode, "native_fork");
            assert_eq!(fidelity, "native");
        } else {
            // No fork support on this machine: the aside still carries the
            // projected brief and nothing claims a native thread.
            assert_eq!(provider_id, None);
            assert_ne!(head_mode, "native_fork");
        }
        // Either way the parent session was never written to.
        let source_entries: i64 = db.query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id='forksource'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(source_entries, 1, "the source conversation must not gain entries from an aside");
    }

    #[test]
    fn forest_digest_is_stable_until_a_store_input_changes() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, Some("stub-standard"), Some("Digest"))
            .unwrap();
        let session_id = only_session_id(&core);
        let baseline = core.session_forest_digest(&session_id).unwrap();
        assert_eq!(
            baseline,
            core.session_forest_digest(&session_id).unwrap(),
            "an unchanged session yields an unchanged digest"
        );

        // An appended forest entry.
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::SessionStatus,
                    serde_json::json!({"status":"working"}),
                )
                .unwrap();
        }
        let after_entry = core.session_forest_digest(&session_id).unwrap();
        assert_ne!(baseline, after_entry, "entry appends invalidate");

        // A head move.
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO session_heads(session_id,restoration_mode,resume_eligibility,updated_at)
                 VALUES(?1,'fresh','fresh','2026-08-20T10:00:00Z')
                 ON CONFLICT(session_id) DO UPDATE SET updated_at='2026-08-20T10:00:00Z'",
                params![session_id],
            )
            .unwrap();
        }
        let after_head = core.session_forest_digest(&session_id).unwrap();
        assert_ne!(after_entry, after_head, "head updates invalidate");

        // A usage row.
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO usage_ledger(workspace_id,session_id,source,created_at) VALUES('w',?1,'test','now')",
                params![session_id],
            )
            .unwrap();
        }
        let after_usage = core.session_forest_digest(&session_id).unwrap();
        assert_ne!(after_head, after_usage, "usage appends invalidate");

        // A reason event.
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor','test.reason','w','because','now')",
                [],
            )
            .unwrap();
        }
        let after_event = core.session_forest_digest(&session_id).unwrap();
        assert_ne!(after_usage, after_event, "workspace events invalidate");

        assert_eq!(
            after_event,
            core.session_forest_digest(&session_id).unwrap(),
            "quiescence is stable again"
        );
    }

    #[test]
    fn forest_digest_rejects_unknown_sessions_like_the_snapshot_does() {
        let (_scratch, core) = fixture();
        assert!(core.session_forest_digest("no-such-session").is_err());
    }

    /// The measurable half of the polling fix: per-poll cost of the digest
    /// versus building and serializing the full snapshot for a 5k-entry
    /// session. Run with:
    /// `cargo test -p bridge-core forest_digest_poll_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark: prints poll-cost numbers, no assertions beyond sanity"]
    fn forest_digest_poll_cost() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('bench-s','w','codex','Bench','working','reported')",
                [],
            )
            .unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            let forest = session_forest::SessionForest::new(&db);
            for index in 0..5_000 {
                forest
                    .append(
                        &session_id,
                        session_forest::EntryKind::SessionStatus,
                        serde_json::json!({"status":"working","detail":format!("turn {index} of a long conversation with realistic payload text")}),
                    )
                    .unwrap();
            }
        }
        let iterations = 100u32;
        let db = core.db.lock().unwrap();

        let start = std::time::Instant::now();
        let mut snapshot_bytes = 0usize;
        for _ in 0..iterations {
            let snapshot = session_forest_snapshot_with_repository_state(
                &db,
                &session_id,
                serde_json::json!({"status":"unavailable"}),
            )
            .unwrap();
            snapshot_bytes = serde_json::to_string(&snapshot).unwrap().len();
        }
        let full_elapsed = start.elapsed();

        let start = std::time::Instant::now();
        let mut digest_bytes = 0usize;
        for _ in 0..iterations {
            digest_bytes = session_forest_digest(&db, &session_id).unwrap().len();
        }
        let digest_elapsed = start.elapsed();

        println!(
            "full-snapshot poll: {:?}/iter, {snapshot_bytes} bytes/iter",
            full_elapsed / iterations
        );
        println!(
            "digest poll:        {:?}/iter, {digest_bytes} bytes/iter",
            digest_elapsed / iterations
        );
        assert!(digest_bytes < 512);
        assert!(snapshot_bytes > 100_000);
    }

    #[test]
    fn create_chat_persists_a_scratch_dir_direct_session() {
        let (_scratch, core) = fixture();
        let snapshot = core
            .create_chat(&Harness::Codex, Some("stub-fast"), Some("  Billing  "))
            .unwrap();
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
        assert!(
            cwd.contains("chats"),
            "direct chats run in a private scratch dir: {cwd}"
        );
    }

    #[test]
    fn create_chat_defaults_the_label_when_the_title_is_blank() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, Some("   "))
            .unwrap();
        let db = core.db.lock().unwrap();
        let label: String = db
            .query_row("SELECT label FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(label, "New chat");
    }

    #[test]
    fn create_chat_rolls_back_the_session_when_its_event_cannot_be_recorded() {
        let (_scratch, core) = fixture();
        core.db
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_chat_created
                 BEFORE INSERT ON events
                 WHEN NEW.kind='chat.created'
                 BEGIN SELECT RAISE(ABORT, 'event rejected'); END;",
            )
            .unwrap();

        assert!(core.create_chat_id(&Harness::Codex, None, None).is_err());
        let db = core.db.lock().unwrap();
        let sessions: i64 = db
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 0, "the failed audit event must not leave a ghost chat");
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
        assert_eq!(
            snapshot.head.unwrap().active_entry_id.as_deref(),
            Some("e1")
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::StateChanged
        ));
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
        assert!(
            events.try_recv().is_err(),
            "a rolled-back head move must publish nothing"
        );
        let active: String = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT active_entry_id FROM session_heads WHERE session_id='s'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            active, "e2",
            "the head move and audit must roll back together"
        );
    }

    #[test]
    fn plan_workspace_session_validates_isolation_and_resolves_a_selection() {
        let (_scratch, core) = fixture();
        assert!(matches!(
            core.plan_workspace_session("missing", false),
            Err(BridgeError::Db(_))
        ));

        seed_workspace(&core, false);
        let error = core.plan_workspace_session("w", true).unwrap_err();
        assert!(
            error.to_string().contains("Connect a Git repository"),
            "{error}"
        );

        let plan = core.plan_workspace_session("w", false).unwrap();
        assert_eq!(plan.selection.adapter_id, "codex");
        assert!(
            plan.selection.model.as_deref()
                .is_some_and(|model| model.starts_with("stub-")),
            "selection must come from the registered adapter, got {}",
            plan.selection.model.as_deref().unwrap_or("default")
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
        assert!(
            error.to_string().contains("no available adapter"),
            "{error}"
        );
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
        assert_eq!(
            cwd, "/tmp/sessions-demo",
            "cwd falls back to the workspace path"
        );
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
            vec!["config", "commit.gpgsign", "false"],
        ] {
            assert!(std::process::Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        for args in [vec!["add", "."], vec!["commit", "-m", "fixture", "-q"]] {
            assert!(std::process::Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
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
        assert!(
            !worktree.path.exists(),
            "failed persistence must remove the worktree"
        );
    }

    /// A harness change clears the backend binding with the provider session
    /// id; a same-harness model change keeps it. The binding's agent is read
    /// from the harness column, so a stale backend id under a new harness
    /// bricks every later launch.
    #[test]
    fn switching_harness_clears_the_backend_binding_and_model_alone_keeps_it() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        let read_backend = || -> Option<String> {
            core.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT backend_id FROM sessions WHERE id=?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let set_backend = |value: &str| {
            core.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE sessions SET backend_id=?2,backend_version='1.0.0' WHERE id=?1",
                    params![session_id, value],
                )
                .unwrap();
        };
        set_backend("claude.agent-sdk");

        // Same harness, different model: the binding survives.
        persist_chat_model_selection(
            &core.db.lock().unwrap(),
            &session_id,
            "claude",
            Some("opus"),
            CapabilityTier::Fast,
            ("claude", None),
            true,
        )
        .unwrap();
        assert_eq!(read_backend().as_deref(), Some("claude.agent-sdk"));

        // Different harness: binding and provider session id both go.
        persist_chat_model_selection(
            &core.db.lock().unwrap(),
            &session_id,
            "codex",
            Some("gpt-5.3-codex"),
            CapabilityTier::Fast,
            ("claude", Some("opus")),
            false,
        )
        .unwrap();
        assert_eq!(read_backend(), None, "a different agent has nothing to continue");
    }

    #[test]
    fn switch_summary_is_skipped_without_a_hot_provider() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"we decided to change src/app.ts"}),
                )
                .unwrap();
        }
        // No adapter runtime is registered for this chat, so there is nobody
        // to ask: the plan must decline without touching the forest.
        assert!(core.plan_switch_summary(&session_id).unwrap().is_none());
        let db = core.db.lock().unwrap();
        let kinds: Vec<String> = db
            .prepare("SELECT kind FROM session_entries WHERE session_id=?1 AND kind LIKE 'compaction%'")
            .unwrap()
            .query_map(params![session_id], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(kinds.is_empty(), "no compaction traffic was appended");
    }

    /// The floor is a magic number, so what needs pinning is that it lands
    /// between the two cases it exists to separate: the "hi" that produced a
    /// refusal in the transcript, and a session with real work in it.
    #[test]
    fn the_switch_summary_floor_separates_a_greeting_from_real_work() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        core.adapters.lock().unwrap().insert(
            session_id.clone(),
            Box::new(RecordingRuntime {
                interrupted: Default::default(),
                usage_requested: Default::default(),
            }),
        );
        let append = |kind: session_forest::EntryKind, payload: serde_json::Value| {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(&session_id, kind, payload)
                .unwrap();
        };

        // The field failure this floor missed the first time: a greeting-only
        // chat measured 13k tokens because the *branch* estimate counts the
        // injected instructions. Reproduce that shape — one bulky machine
        // entry dwarfing a two-line conversation — and require the floor to
        // read only the conversation.
        append(
            session_forest::EntryKind::SessionStatus,
            serde_json::json!({"status": "ready", "detail": "Compiled orchestration prompt. ".repeat(700)}),
        );
        append(session_forest::EntryKind::UserMessage, serde_json::json!({"text":"hi"}));
        append(
            session_forest::EntryKind::AssistantMessage,
            serde_json::json!({"role":"assistant","text":"Hey! What are we building?"}),
        );
        let db_estimates = || {
            let db = core.db.lock().unwrap();
            (
                compaction_controller::active_token_estimate(&db, &session_id).unwrap(),
                compaction_controller::conversation_token_estimate(&db, &session_id).unwrap(),
            )
        };
        let (branch_tokens, conversation_tokens) = db_estimates();
        assert!(
            branch_tokens >= SWITCH_SUMMARY_MIN_TOKENS,
            "the trap: the full branch already clears the floor, estimated {branch_tokens}"
        );
        assert!(
            conversation_tokens < SWITCH_SUMMARY_MIN_TOKENS,
            "the conversation itself sits under it, estimated {conversation_tokens}"
        );
        assert!(
            core.plan_switch_summary(&session_id).unwrap().is_none(),
            "the hot-path planner must apply the floor, not only compute an estimate below it"
        );
        let compaction_requests = || {
            core.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='compaction.requested'",
                    params![session_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        };
        assert_eq!(compaction_requests(), 0, "the skipped summary must not touch the forest");

        // Now the same session with actual work in it. Sizes are the point, so
        // the payloads carry real bulk rather than a token_estimate override.
        for index in 0..8 {
            append(
                session_forest::EntryKind::AssistantMessage,
                serde_json::json!({
                    "role": "assistant",
                    "text": format!("Refactored the token store for the {index}th time. {}", "Details of what changed and why, at the length a real turn runs to. ".repeat(12)),
                }),
            );
        }
        let (_, working_tokens) = db_estimates();
        assert!(
            working_tokens >= SWITCH_SUMMARY_MIN_TOKENS,
            "a session with real history must clear the floor, estimated {working_tokens}"
        );
        assert!(
            core.plan_switch_summary(&session_id).unwrap().is_some(),
            "real history must still produce a checkpoint request"
        );
        assert_eq!(compaction_requests(), 1);
    }

    #[test]
    fn switch_summary_outcome_tracks_the_pipeline_and_timeout_cancels_it() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"history worth summarising"}),
                )
                .unwrap();
        }
        // A cold session declines to plan a summary at all.
        assert!(core.plan_switch_summary(&session_id).unwrap().is_none());

        // Simulate what a hot session's pipeline does: begin() appends the
        // request; the reader resolves it with a terminal entry.
        let request = {
            let db = core.db.lock().unwrap();
            let prompt = compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeDowngrade,
                100,
            )
            .unwrap().prompt()
            .expect("no compaction is in flight");
            let after_sequence: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id=?1 AND kind='compaction.requested'",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap();
            super::SwitchSummaryRequest {
                session_id: session_id.clone(),
                prompt,
                after_sequence,
            }
        };
        assert_eq!(
            core.switch_summary_outcome(&request).unwrap(),
            super::SwitchSummaryOutcome::Pending
        );
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::Compaction,
                    serde_json::json!({"schemaVersion":1,"summary":"the typed summary"}),
                )
                .unwrap();
        }
        assert_eq!(
            core.switch_summary_outcome(&request).unwrap(),
            super::SwitchSummaryOutcome::Summarised
        );

        // A second request that times out must be cancelled, so later normal
        // replies are never misread as checkpoint output.
        {
            let db = core.db.lock().unwrap();
            compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeDowngrade,
                120,
            )
            .unwrap().prompt()
            .expect("previous terminal cleared the way");
        }
        core.cancel_switch_summary(
            &session_id,
            "model-switch summary timed out; switch continued",
            1,
        )
        .unwrap();
        assert_eq!(
            core.switch_summary_outcome(&request).unwrap(),
            super::SwitchSummaryOutcome::Failed
        );
        let db = core.db.lock().unwrap();
        assert!(compaction_controller::CompactionController::pending(&db, &session_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn switch_summary_outcome_ignores_terminals_older_than_the_request() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"history worth summarising"}),
                )
                .unwrap();
        }
        // A terminal from a PREVIOUS checkpoint exists before this request.
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::Compaction,
                    serde_json::json!({"schemaVersion":1,"summary":"an older summary"}),
                )
                .unwrap();
        }
        let request = {
            let db = core.db.lock().unwrap();
            compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeDowngrade,
                100,
            )
            .unwrap().prompt()
            .expect("the older terminal cleared the way");
            let after_sequence: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id=?1 AND kind='compaction.requested'",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap();
            super::SwitchSummaryRequest {
                session_id: session_id.clone(),
                prompt: "summarise".into(),
                after_sequence,
            }
        };
        // The stale terminal must not answer for this request.
        assert_eq!(
            core.switch_summary_outcome(&request).unwrap(),
            super::SwitchSummaryOutcome::Pending
        );
    }

    #[test]
    fn commit_reports_what_the_next_provider_will_inherit() {
        let (_scratch, core) = fixture();

        // An empty chat carries nothing, and says so plainly.
        let empty_id = core.create_chat_id(&Harness::Claude, None, None).unwrap();
        let change = core
            .plan_chat_model_change(&empty_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        let event = core.commit_chat_model_change(change).unwrap();
        assert_eq!(event.data["carriedContext"], serde_json::Value::Null);
        assert!(event.text.as_deref().unwrap().contains("no context carried"));

        // A chat whose branch holds a validated summary reports it. Seeded
        // through the real reconstruction path so the boundary trio
        // (checkpoint + compaction + branch.summary) matches production.
        let rich_id = core.create_chat_id(&Harness::Claude, None, None).unwrap();
        {
            let db = core.db.lock().unwrap();
            let forest = session_forest::SessionForest::new(&db);
            forest
                .append(
                    &rich_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"we decided to change src/app.ts"}),
                )
                .unwrap();
            compaction_controller::CompactionController::record_reconstructed(
                &db,
                &rich_id,
                "chose src/app.ts".to_owned(),
                vec!["change src/app.ts".to_owned()],
                Vec::new(),
                compaction_controller::CompactionReason::BeforeDowngrade,
            )
            .unwrap();
        }
        let change = core
            .plan_chat_model_change(&rich_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        let event = core.commit_chat_model_change(change).unwrap();
        assert_eq!(event.data["carriedContext"]["summary"], true);
        assert_eq!(event.data["carriedContext"]["decisions"], 1);
        assert_eq!(event.data["carriedContext"]["filesTouched"], 0);
        let text = event.text.as_deref().unwrap();
        assert!(text.contains("carried forward: summary + 1 decisions"), "{text}");
    }

    #[test]
    fn failed_switch_summary_commits_new_model_with_mechanical_history_only() {
        let (_scratch, core) = fixture();
        let session_id = core.create_chat_id(&Harness::Claude, None, None).unwrap();
        {
            let db = core.db.lock().unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"Keep this original history"}),
                )
                .unwrap();
        }
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .expect("claude to codex changes the runtime");
        {
            let db = core.db.lock().unwrap();
            compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeDowngrade,
                120,
            )
            .unwrap().prompt()
            .unwrap();
            compaction_controller::CompactionController::record_failure(
                &db,
                &session_id,
                "checkpoint metadata does not match its controller request",
                1,
            )
            .unwrap();
        }

        let event = core.commit_chat_model_change(change).unwrap();
        assert_eq!(event.data["harness"], "codex");
        assert_eq!(event.data["model"], "stub-fast");
        assert_eq!(event.data["carriedContext"]["summary"], false);
        assert_eq!(event.data["carriedContext"]["recentEntries"], 1);
        let entries = store::session_entries(&core.db.lock().unwrap(), &session_id).unwrap();
        let failure = entries
            .iter()
            .find(|entry| entry.kind == "compaction.failed")
            .unwrap();
        assert_eq!(failure.payload["trigger"], "before_downgrade");
        assert!(failure.payload["message"]
            .as_str()
            .unwrap()
            .contains("model switch continued"));
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == "compaction")
                .count(),
            0,
            "a failed outgoing summary cannot create a reconstructed boundary"
        );
    }

    #[test]
    fn a_resumable_same_harness_change_keeps_the_provider_session() {
        let (_scratch, core) = resume_fixture();
        core.create_chat(&Harness::Claude, Some("stub-sonnet"), None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "UPDATE sessions SET provider_session_id='thread-1',backend_id='claude.agent-sdk',backend_version='1.2.3' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
            restoration::set_head_state(
                &db,
                &session_id,
                RestorationMode::Native,
                ResumeEligibility::Native,
                Some("thread-1"),
            )
            .unwrap();
            // Something worth summarising, so a skipped summary is a decision
            // rather than an empty-conversation no-op.
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"we decided to change src/app.ts"}),
                )
                .unwrap();
        }

        let change = core
            .plan_chat_model_change(&session_id, &Harness::Claude, Some("stub-opus"))
            .unwrap()
            .expect("stub-sonnet -> stub-opus is a real change");
        assert!(change.resumes_natively());
        let event = core.commit_chat_model_change(change).unwrap();

        let db = core.db.lock().unwrap();
        let (provider, backend, model): (Option<String>, Option<String>, Option<String>) = db
            .query_row(
                "SELECT provider_session_id,backend_id,model FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        // The whole point: the thread survives so the next turn resumes it.
        assert_eq!(provider.as_deref(), Some("thread-1"));
        assert_eq!(backend.as_deref(), Some("claude.agent-sdk"));
        assert_eq!(model.as_deref(), Some("stub-opus"));

        let (mode, eligibility, native): (String, String, Option<String>) = db
            .query_row(
                "SELECT restoration_mode,resume_eligibility,native_provider_session_id FROM session_heads WHERE session_id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(mode, "native");
        assert_eq!(eligibility, "native");
        assert_eq!(native.as_deref(), Some("thread-1"));

        // And a plan built on that state resumes rather than projecting.
        assert_eq!(
            restoration::select_plan(false, provider.as_deref(), true, false, true, false),
            restoration::RestorationPlan::Native
        );

        // The milestone still renders, and it no longer claims a fresh session.
        assert_eq!(event.data["modelChanged"], true);
        assert_eq!(event.data["freshProviderSession"], false);
        assert!(event.data["carriedContext"].is_null());
        let text = event.text.unwrap_or_default();
        assert!(text.contains("continues on the same claude session"), "{text}");
        assert!(!text.contains("fresh provider session"), "{text}");
    }

    #[test]
    fn a_same_harness_change_without_resume_support_takes_the_handover_path() {
        // The codex stub reports no native resume — production Cursor and
        // Grok do the same — so a model change there must summarise and start
        // fresh, exactly as a harness change does, rather than promise a
        // continuation the next start cannot deliver.
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, Some("stub-fast"), None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "UPDATE sessions SET provider_session_id='thread-1',backend_id='codex.app-server' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
            restoration::set_head_state(
                &db,
                &session_id,
                RestorationMode::Native,
                ResumeEligibility::Native,
                Some("thread-1"),
            )
            .unwrap();
            session_forest::SessionForest::new(&db)
                .append(
                    &session_id,
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text":"we decided to change src/app.ts"}),
                )
                .unwrap();
        }

        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, Some("stub-standard"))
            .unwrap()
            .expect("stub-fast -> stub-standard is a real change");
        assert!(!change.resumes_natively());
        let event = core.commit_chat_model_change(change).unwrap();

        let db = core.db.lock().unwrap();
        let (provider, backend): (Option<String>, Option<String>) = db
            .query_row(
                "SELECT provider_session_id,backend_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        // Nobody can resume this thread, so it is cleared; the backend
        // binding stays, since the same agent still serves the session.
        assert_eq!(provider, None);
        assert_eq!(backend.as_deref(), Some("codex.app-server"));

        let (mode, native): (String, Option<String>) = db
            .query_row(
                "SELECT restoration_mode,native_provider_session_id FROM session_heads WHERE session_id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(mode, "fresh");
        assert_eq!(native, None);

        // And the transcript says fresh, with the projection to show for it.
        assert_eq!(event.data["modelChanged"], true);
        assert_eq!(event.data["freshProviderSession"], true);
        assert!(event.data["carriedContext"].is_object());
        let text = event.text.unwrap_or_default();
        assert!(text.contains("fresh provider session"), "{text}");
    }

    #[test]
    fn a_same_harness_change_with_no_stored_thread_takes_the_handover_path() {
        // A chat that never started has no thread to resume, even on a
        // resume-capable harness: the switch still hands over the projection.
        let (_scratch, core) = resume_fixture();
        core.create_chat(&Harness::Claude, Some("stub-sonnet"), None).unwrap();
        let session_id = only_session_id(&core);

        let change = core
            .plan_chat_model_change(&session_id, &Harness::Claude, Some("stub-opus"))
            .unwrap()
            .expect("stub-sonnet -> stub-opus is a real change");
        assert!(!change.resumes_natively());
        let event = core.commit_chat_model_change(change).unwrap();
        assert_eq!(event.data["freshProviderSession"], true);
        let text = event.text.unwrap_or_default();
        assert!(text.contains("fresh provider session"), "{text}");
    }

    #[test]
    fn a_cross_harness_change_clears_the_provider_session_and_the_stale_native_id() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "UPDATE sessions SET provider_session_id='claude-session',backend_id='claude.agent-sdk' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
            restoration::set_head_state(
                &db,
                &session_id,
                RestorationMode::Native,
                ResumeEligibility::Native,
                Some("claude-session"),
            )
            .unwrap();
        }

        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .expect("claude -> codex is a real change");
        assert!(!change.resumes_natively());
        let event = core.commit_chat_model_change(change).unwrap();

        let db = core.db.lock().unwrap();
        let (provider, backend): (Option<String>, Option<String>) = db
            .query_row(
                "SELECT provider_session_id,backend_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(provider, None);
        assert_eq!(backend, None);

        let (mode, native): (String, Option<String>) = db
            .query_row(
                "SELECT restoration_mode,native_provider_session_id FROM session_heads WHERE session_id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(mode, "fresh");
        // G10: this used to keep pointing at a thread nothing would ever resume.
        assert_eq!(native, None);
        assert_eq!(event.data["modelChanged"], true);
        assert_eq!(event.data["freshProviderSession"], true);
    }

    #[test]
    fn persist_chat_model_selection_still_guards_against_a_row_that_moved() {
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Codex, Some("stub-fast"), None).unwrap();
        let session_id = only_session_id(&core);
        let db = core.db.lock().unwrap();
        db.execute(
            "UPDATE sessions SET provider_session_id='thread-1' WHERE id=?1",
            params![session_id],
        )
        .unwrap();
        // The guard reads the model the plan saw; a different one means the row
        // changed underneath and the switch must not clobber it.
        assert_eq!(
            persist_chat_model_selection(
                &db,
                &session_id,
                "codex",
                Some("stub-standard"),
                CapabilityTier::Standard,
                ("codex", Some("someone-else-switched")),
                true,
            )
            .unwrap(),
            0
        );
        let provider: Option<String> = db
            .query_row(
                "SELECT provider_session_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provider.as_deref(), Some("thread-1"));
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
        let error = core
            .plan_chat_model_change(&session_id, &Harness::Claude, None)
            .unwrap_err();
        assert!(error.to_string().contains("No model adapter"), "{error}");
        // Unknown model on a registered adapter.
        let error = core
            .plan_chat_model_change(&session_id, &Harness::Codex, Some("no-such-model"))
            .unwrap_err();
        assert!(
            error.to_string().contains("does not offer model"),
            "{error}"
        );
        // Busy chats cannot switch.
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET active_turn_id='turn' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        let error = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap_err();
        assert!(
            error.to_string().contains("Wait for the current response"),
            "{error}"
        );
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET active_turn_id=NULL WHERE id=?1",
                params![session_id],
            )
            .unwrap();

        // Plan + commit: direct chats default to the Fast tier.
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .expect("switching claude -> codex is a real change");
        assert_eq!(change.selected_model(), Some("stub-fast"));
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
            .query_row(
                "SELECT harness,model FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
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
        assert!(
            events.try_recv().is_err(),
            "a rolled-back model change must publish nothing"
        );
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
        assert_eq!(
            model, None,
            "session state and durable history must commit together"
        );
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
            Some("opus"),
            CapabilityTier::Strong,
            ("codex", Some("old-model")),
            false,
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
        assert_eq!(
            actual,
            (
                "claude".into(),
                "opus".into(),
                "strong".into(),
                "idle".into(),
                None
            )
        );

        // A stale revision (the session changed since planning) updates nothing.
        let stale = persist_chat_model_selection(
            &db,
            "orchestrator",
            "codex",
            Some("other"),
            CapabilityTier::Standard,
            ("codex", Some("old-model")),
            false,
        )
        .unwrap();
        assert_eq!(stale, 0, "a stale plan must not clobber a changed session");
    }

    #[test]
    fn lifecycle_claims_serialize_starts_against_model_switches() {
        let (_scratch, core) = fixture();
        let claim = core.claim_session_lifecycle("s", "model switch").unwrap();
        let error = core
            .claim_session_lifecycle("s", "session start")
            .unwrap_err();
        assert!(
            error.to_string().contains("model switch"),
            "the conflict names the operation in flight: {error}"
        );
        // Other sessions are unaffected; releasing the claim reopens the session.
        core.claim_session_lifecycle("other", "session start")
            .unwrap();
        drop(claim);
        core.claim_session_lifecycle("s", "session start").unwrap();
    }

    #[test]
    fn app_shutdown_settles_a_detached_summary_with_or_without_an_incoming_runtime() {
        for incoming_runtime in [false, true] {
            let (_scratch, core) = fixture();
            let session_id = core
                .create_chat_id(&Harness::Codex, Some("stub-fast"), None)
                .unwrap();
            core.adapters.lock().unwrap().insert(
                session_id.clone(),
                Box::new(RecordingRuntime {
                    interrupted: Default::default(),
                    usage_requested: Default::default(),
                }),
            );
            let request = {
                let db = core.db.lock().unwrap();
                let prompt = compaction_controller::CompactionController::begin_background(
                    &db,
                    &session_id,
                    compaction_controller::CompactionReason::BeforeDowngrade,
                    100,
                )
                .unwrap()
                .prompt()
                .unwrap();
                SwitchSummaryRequest {
                    session_id: session_id.clone(),
                    prompt,
                    after_sequence: 0,
                }
            };
            assert!(crate::switch_summary::detach(&core, &session_id, request));
            if incoming_runtime {
                core.adapters.lock().unwrap().insert(
                    session_id.clone(),
                    Box::new(RecordingRuntime {
                        interrupted: Default::default(),
                        usage_requested: Default::default(),
                    }),
                );
            }
            core.db.lock().unwrap().execute(
                "UPDATE sessions SET status=?2,active_turn_id=?3,
                 provider_session_id='incoming-thread',adapter_pid=?4,
                 adapter_process_identity=?5 WHERE id=?1",
                params![
                    session_id,
                    if incoming_runtime { "working" } else { "ready" },
                    incoming_runtime.then_some("incoming-turn"),
                    incoming_runtime.then_some(0),
                    incoming_runtime.then_some("fixture"),
                ],
            ).unwrap();
            let mut events = core.events.subscribe();

            core.shutdown_session_adapter(&session_id).unwrap();

            assert!(!core.adapters.lock().unwrap().contains_key(&session_id));
            assert!(!crate::switch_summary::is_detached(&core, &session_id));
            assert!(crate::switch_summary::is_detached_launch(&core, &session_id, 0, "recording"),
                "the outgoing reader must remain recognisable while its final frames drain");
            let db = core.db.lock().unwrap();
            assert!(compaction_controller::CompactionController::pending(&db, &session_id)
                .unwrap().is_none());
            let saved: (String, Option<String>, Option<i64>, Option<String>, String) = db.query_row(
                "SELECT status,active_turn_id,adapter_pid,adapter_process_identity,provider_session_id
                 FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).unwrap();
            assert_eq!(saved, ("stopped".into(), None, None, None, "incoming-thread".into()));
            let entries = store::session_entries(&db, &session_id).unwrap();
            assert!(entries.iter().any(|entry| entry.kind == "compaction.failed"
                && entry.payload["reason"].as_str().is_some_and(|reason| reason.contains("app_shutdown"))));
            assert_eq!(entries.last().unwrap().payload["interrupted"], incoming_runtime);
            assert!(matches!(events.try_recv().unwrap(), crate::events::CoreEvent::StateChanged));
        }
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
            .execute(
                "UPDATE sessions SET model='switched-elsewhere' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        let mut events = core.events.subscribe();
        let error = core.commit_chat_model_change(change).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed while the switch was in flight"),
            "{error}"
        );
        assert!(
            events.try_recv().is_err(),
            "a stale model-change plan must publish nothing"
        );
        let model: Option<String> = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT model FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            model.as_deref(),
            Some("switched-elsewhere"),
            "the interleaved state survives"
        );
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
            .execute(
                "UPDATE sessions SET parent_session_id='parent' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        let error = core.commit_chat_model_change(change).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed while the switch was in flight"),
            "{error}"
        );
    }

    #[test]
    fn switch_commits_after_its_own_summary_turn_left_turn_state_behind() {
        // Regression for the "changed while the switch was in flight" failure:
        // the switch's own summary turn wrote `active_turn_id`, teardown killed
        // the adapter before `turn.completed`, and the commit refused. Settling
        // the adapterless residue must let the commit land.
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        let change = core
            .plan_chat_model_change(&session_id, &Harness::Codex, None)
            .unwrap()
            .unwrap();
        // What the reader thread does when the summary turn starts — and no
        // adapter survives to ever complete it.
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET active_turn_id='summary-turn',status='working' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        core.settle_adapterless_turn_state(&session_id, std::time::Duration::ZERO)
            .unwrap();
        core.commit_chat_model_change(change).unwrap();
        let (harness, active_turn_id, status): (String, Option<String>, String) = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT harness,active_turn_id,status FROM sessions WHERE id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(harness, "codex", "the switch landed");
        assert!(active_turn_id.is_none(), "no dangling turn survives");
        assert_eq!(status, "idle", "the session is usable again");
    }

    #[test]
    fn settle_waits_for_a_natural_clear_before_touching_the_row() {
        // The reader's own cleanup (turn.completed or the exit sweep) may land
        // during the budget; settle must observe it rather than rewrite it.
        let (_scratch, core) = fixture();
        let core = std::sync::Arc::new(core);
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET active_turn_id='summary-turn',status='working' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        let clearer = {
            let core = std::sync::Arc::clone(&core);
            let session_id = session_id.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(60));
                core.db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE sessions SET active_turn_id=NULL,status='stopped' WHERE id=?1",
                        params![session_id],
                    )
                    .unwrap();
            })
        };
        core.settle_adapterless_turn_state(&session_id, std::time::Duration::from_secs(5))
            .unwrap();
        clearer.join().unwrap();
        let status: String = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "stopped", "the natural cleanup's write survives");
    }

    #[test]
    fn settle_leaves_a_live_runtime_alone() {
        // With the process alive a turn may still finish on its own; the
        // commit guard keeps its pessimism and settle must not interfere.
        let (_scratch, core) = fixture();
        core.create_chat(&Harness::Claude, None, None).unwrap();
        let session_id = only_session_id(&core);
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET active_turn_id='user-turn',status='working' WHERE id=?1",
                params![session_id],
            )
            .unwrap();
        core.adapters.lock().unwrap().insert(
            session_id.clone(),
            Box::new(RecordingRuntime {
                interrupted: Default::default(),
                usage_requested: Default::default(),
            }),
        );
        core.settle_adapterless_turn_state(&session_id, std::time::Duration::ZERO)
            .unwrap();
        let active: Option<String> = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT active_turn_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active.as_deref(), Some("user-turn"), "a live turn is not cleared");
    }

    #[test]
    fn a_natively_resumable_switch_appends_no_handover_summary_through_the_api() {
        // The plan/commit halves are covered above; this drives the real
        // `update_chat_model` path, where skipping the summary is what keeps a
        // 20-second budget and a `compaction.failed` row out of every
        // same-harness switch.
        let (_scratch, core) = resume_fixture();
        seed_workspace(&core, false);
        let core = std::sync::Arc::new(core);
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,model,provider_session_id) VALUES('switch-chat','w','claude','Chat','idle','estimated','direct','stub-sonnet','thread-1')", [],
        ).unwrap();
        core.adapters.lock().unwrap().insert("switch-chat".into(), Box::new(RecordingRuntime {
            interrupted: Default::default(), usage_requested: Default::default(),
        }));
        {
            let db = core.db.lock().unwrap();
            let forest = session_forest::SessionForest::new(&db);
            for index in 0..8 {
                forest.append(
                    "switch-chat",
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text": format!("{index}: {}", "we decided to change src/app.ts. ".repeat(40))}),
                ).unwrap();
            }
        }
        // Control: every gate `plan_switch_summary` checks is satisfied, so a
        // zero below is a decision and not an empty-conversation no-op.
        assert!(core.plan_switch_summary("switch-chat").unwrap().is_some());
        core.cancel_switch_summary("switch-chat", "control", 1).unwrap();
        let before: i64 = core.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id='switch-chat' AND kind='compaction.requested'",
            [], |row| row.get(0),
        ).unwrap();

        crate::api::update_chat_model(&core, "switch-chat", &Harness::Claude, Some("stub-opus"), None).unwrap();

        let db = core.db.lock().unwrap();
        let after: i64 = db.query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id='switch-chat' AND kind='compaction.requested'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(after, before, "a natively resumable switch must not ask for a handover summary");
        let (model, provider): (String, Option<String>) = db.query_row(
            "SELECT model,provider_session_id FROM sessions WHERE id='switch-chat'", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(model, "stub-opus");
        assert_eq!(provider.as_deref(), Some("thread-1"), "the thread survives the switch");
    }

    #[test]
    fn a_cross_harness_switch_commits_before_the_summary_turn_starts() {
        // The whole point of #529: a cross-harness switch must return without
        // waiting on the outgoing model's checkpoint. It commits on the
        // mechanical projection and detaches the old runtime to summarise in
        // the background.
        let (_scratch, core) = resume_fixture();
        seed_workspace(&core, false);
        let core = std::sync::Arc::new(core);
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,model,provider_session_id) VALUES('x','w','claude','Chat','idle','estimated','direct','stub-sonnet','thread-1')", [],
        ).unwrap();
        core.adapters.lock().unwrap().insert("x".into(), Box::new(RecordingRuntime {
            interrupted: Default::default(), usage_requested: Default::default(),
        }));
        {
            let db = core.db.lock().unwrap();
            let forest = session_forest::SessionForest::new(&db);
            for index in 0..8 {
                forest.append(
                    "x",
                    session_forest::EntryKind::UserMessage,
                    serde_json::json!({"text": format!("{index}: {}", "we decided to change src/app.ts. ".repeat(40))}),
                ).unwrap();
            }
        }

        crate::api::update_chat_model(&core, "x", &Harness::Codex, Some("stub-standard"), None).unwrap();

        // The switch committed: the row is the incoming model, and the outgoing
        // runtime is detached (out of the adapter slot, in the summary map),
        // with a BACKGROUND pending request the incoming reader will ignore.
        let (harness, model): (String, String) = core.db.lock().unwrap().query_row(
            "SELECT harness,model FROM sessions WHERE id='x'", [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(harness, "codex");
        assert_eq!(model, "stub-standard");
        assert!(crate::switch_summary::is_detached(&core, "x"), "the outgoing runtime is detached");
        assert!(!core.adapters.lock().unwrap().contains_key("x"), "the adapter slot is free for the incoming model");
        let pending = compaction_controller::CompactionController::pending(&core.db.lock().unwrap(), "x")
            .unwrap()
            .expect("a summary request is pending");
        assert!(pending.background, "the request is a background one");
        assert_eq!(pending.reason, compaction_controller::CompactionReason::BeforeDowngrade);

        // The switch's own audit event is recorded; any checkpoint.turn_started
        // can only come after it, from the background thread post-commit.
        let events = store::state(&core.db.lock().unwrap()).unwrap().events;
        let model_changed = events.iter().find(|event| event.kind == "session.model_changed").expect("the switch committed its milestone");
        let model_changed_id = Some(model_changed.id);
        assert!(
            model_changed.body.contains("being prepared in the background"),
            "the milestone names the live background summary: {}",
            model_changed.body
        );
        for event in &events {
            if event.kind == "checkpoint.turn_started" {
                assert!(event.id > model_changed_id.unwrap(), "a summary turn never precedes the commit");
            }
        }

        // Settle the request so the background waiter exits promptly instead of
        // polling for its full budget.
        core.cancel_switch_summary("x", "test teardown", 1).unwrap();
        crate::switch_summary::abort(&core, "x");
    }

    #[test]
    fn chat_effort_changes_restart_warm_runtime_and_validate_before_teardown() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        let core = std::sync::Arc::new(core);
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,model,effort,provider_session_id) VALUES('effort-chat','w','codex','Chat','idle','estimated','direct','stub-standard','high','native-thread')", [],
        ).unwrap();
        core.adapters.lock().unwrap().insert("effort-chat".into(), Box::new(RecordingRuntime {
            interrupted: Default::default(), usage_requested: Default::default(),
        }));
        assert!(crate::api::update_chat_model(&core, "effort-chat", &Harness::Codex, Some("stub-standard"), Some("invalid")).is_err());
        assert!(core.adapters.lock().unwrap().contains_key("effort-chat"));
        crate::api::update_chat_model(&core, "effort-chat", &Harness::Codex, Some("stub-standard"), Some("high")).unwrap();
        assert!(core.adapters.lock().unwrap().contains_key("effort-chat"), "same effort must not restart");
        crate::api::update_chat_model(&core, "effort-chat", &Harness::Codex, Some("stub-standard"), Some("ultra")).unwrap();
        assert!(!core.adapters.lock().unwrap().contains_key("effort-chat"));
        let (effort, provider): (String, String) = core.db.lock().unwrap().query_row(
            "SELECT effort,provider_session_id FROM sessions WHERE id='effort-chat'", [], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(effort, "ultra");
        assert_eq!(provider, "native-thread");
        crate::api::update_chat_model(&core, "effort-chat", &Harness::Codex, Some("stub-fast"), None).unwrap();
        let effort: Option<String> = core.db.lock().unwrap().query_row("SELECT effort FROM sessions WHERE id='effort-chat'", [], |row| row.get(0)).unwrap();
        assert!(effort.is_none(), "switching to a model without thinking support clears effort");
    }

    /// A runtime that is registered but can no longer answer — the shape of a
    /// session whose request writer has closed under it.
    struct RejectingUsageRuntime;
    impl adapters::AdapterRuntime for RejectingUsageRuntime {
        fn process_id(&self) -> u32 {
            0
        }
        fn provider_session_id(&self) -> &str {
            "rejecting"
        }
        fn current_turn(&self) -> std::sync::Arc<std::sync::Mutex<Option<String>>> {
            std::sync::Arc::new(std::sync::Mutex::new(None))
        }
        fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn respond(&self, _: serde_json::Value, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn read_usage(&self) -> Result<(), BridgeError> {
            Err(BridgeError::Invalid("the request writer is closed".into()))
        }
        fn stop(&mut self, _: adapters::ShutdownReason) {}
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
            self.interrupted
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn respond(&self, _: serde_json::Value, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn read_usage(&self) -> Result<(), BridgeError> {
            self.usage_requested
                .store(true, std::sync::atomic::Ordering::SeqCst);
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
        assert!(
            error.to_string().contains("Compaction suppressed"),
            "{error}"
        );

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
        // No live runtime: nothing is asked, and saying so is what lets the
        // caller fall back to Codex's on-disk limits instead of showing blank.
        assert!(!core.request_codex_usage().unwrap());

        let usage_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        core.adapters.lock().unwrap().insert(
            "c1".into(),
            Box::new(RecordingRuntime {
                interrupted: Default::default(),
                usage_requested: usage_requested.clone(),
            }),
        );
        assert!(core.request_codex_usage().unwrap());
        assert!(usage_requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Writes a rollout carrying one live and one expired window.
    fn seed_codex_rollout(sessions_dir: &Path, now: i64) {
        let day = sessions_dir.join("2026").join("09").join("09");
        std::fs::create_dir_all(&day).unwrap();
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "limit_id": "codex",
                    "primary": { "used_percent": 44.0, "window_minutes": 300, "resets_at": now + 900 },
                    "secondary": { "used_percent": 91.0, "window_minutes": 10_080, "resets_at": now - 60 },
                    "plan_type": "plus"
                }
            }
        })
        .to_string();
        std::fs::write(day.join("rollout-a.jsonl"), format!("{line}\n")).unwrap();
    }

    #[test]
    fn codex_usage_falls_back_to_disk_when_no_session_is_live() {
        let (_scratch, core) = fixture();
        let mut events = core.events.subscribe();
        let temp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp();
        seed_codex_rollout(temp.path(), now);

        assert!(super::publish_codex_usage_from_disk(
            &core.events,
            temp.path(),
            now
        ));

        match events.try_recv().unwrap() {
            crate::events::CoreEvent::AccountUsage {
                provider,
                rate_limits,
            } => {
                assert_eq!(provider, "codex");
                // The live 5h window is reported...
                assert_eq!(rate_limits["primary"]["used_percent"], serde_json::json!(44.0));
                // ...and the window that already reset is not, however alarming
                // its last recorded percentage was.
                assert!(rate_limits.get("secondary").is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
        // Exactly one frame, not one per rollout examined.
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn codex_disk_fallback_clears_the_provider_without_usable_limits() {
        let (_scratch, core) = fixture();
        let mut events = core.events.subscribe();
        let temp = tempfile::tempdir().unwrap();
        // Every window already reset, so there is nothing current to say.
        seed_codex_rollout(temp.path(), 0);

        assert!(!super::publish_codex_usage_from_disk(
            &core.events,
            temp.path(),
            chrono::Utc::now().timestamp()
        ));
        // Silence would strand whatever a since-ended session last reported.
        // An empty payload is how the tray and the panel are told to drop it.
        match events.try_recv().unwrap() {
            crate::events::CoreEvent::AccountUsage {
                provider,
                rate_limits,
            } => {
                assert_eq!(provider, "codex");
                assert_eq!(rate_limits, serde_json::json!({}));
                assert_eq!(crate::meter_sources::tray_title(&rate_limits), "");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn codex_usage_falls_through_to_disk_when_a_live_session_rejects() {
        let (_scratch, core) = fixture();
        seed_workspace(&core, false);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('c1','w','codex','S','idle','reported')", []).unwrap();
        }
        core.adapters
            .lock()
            .unwrap()
            .insert("c1".into(), Box::new(RejectingUsageRuntime));

        // The runtime is registered but cannot answer. Claiming it was asked
        // would suppress the fallback in the one case that most needs it.
        assert!(!core.request_codex_usage().unwrap());
    }

    #[test]
    fn account_usage_ticks_ride_the_bus_with_the_legacy_payload() {
        let (_scratch, core) = fixture();
        let mut events = core.events.subscribe();
        core.publish_account_usage("codex", serde_json::json!({"remaining": 5}));
        match events.try_recv().unwrap() {
            crate::events::CoreEvent::AccountUsage {
                provider,
                rate_limits,
            } => {
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
            core.events
                .publish(crate::events::CoreEvent::Agent(event.clone()));
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
        let replayed = core
            .replay_session_events(&session_id, last_seen, None, None)
            .unwrap();
        let sequences: Vec<i64> = replayed.iter().map(|event| event.sequence).collect();
        assert_eq!(
            sequences,
            vec![third, third + 1, fifth, raced],
            "replay itself has no gaps or duplicates"
        );
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(replayed[0].text.as_deref(), Some("three"));
        assert_eq!(
            replayed[0].kind, "assistant.message",
            "replay carries the durable forest kind"
        );
        assert!(
            replayed[0].sequence > 0,
            "durable events always carry a positive cursor"
        );
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
        assert_eq!(
            delivered,
            vec![third, third + 1, fifth, raced, after_replay]
        );
        // Replaying from the newest cursor is empty; from zero is everything durable.
        assert!(core
            .replay_session_events(&session_id, after_replay, None, None)
            .unwrap()
            .is_empty());
        assert!(
            core.replay_session_events(&session_id, 0, None, None)
                .unwrap()
                .len()
                >= 5
        );
        // Unknown sessions replay nothing rather than erroring.
        assert!(core
            .replay_session_events("no-such-session", 0, None, None)
            .unwrap()
            .is_empty());
        assert!(core.replay_session_events(&session_id, -1, None, None).is_err());
        assert!(core.replay_session_events(&session_id, 0, Some(0), None).is_err());
        assert!(core
            .replay_session_events(
                &session_id,
                0,
                Some(bridge_protocol::messages::MAX_REPLAY_EVENT_LIMIT + 1),
                None,
            )
            .is_err());
        assert_eq!(
            core.replay_session_events(&session_id, 0, Some(2), None)
                .unwrap()
                .len(),
            2,
            "replay pages are bounded by the requested limit"
        );
        let tail = core
            .replay_session_events(&session_id, 0, Some(2), Some(true))
            .unwrap();
        assert_eq!(
            tail.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            vec![raced, after_replay]
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

        let replayed = core.replay_session_events(&session_id, 0, None, None).unwrap();
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
        assert!(core.replay_session_events(&session_id, 0, None, None).is_err());

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
        assert!(core.replay_session_events(&session_id, 0, None, None).is_err());
    }

    #[test]
    fn stopping_a_session_without_a_live_adapter_is_a_no_op() {
        let (_scratch, core) = fixture();
        core.stop_session_adapter("nothing-running", adapters::ShutdownReason::Replaced);
        assert!(core.adapters.lock().unwrap().is_empty());
    }
}

//! The live-turn kernel: starting adapter sessions, delivering turns,
//! stopping sessions, the adapter reader threads, and the worker/delegation
//! settlement they drive — transplanted verbatim from the Tauri shell.
//!
//! Every function here takes (or captures) an `Arc<BridgeCore>` instead of an
//! `AppHandle`: threads own a runtime handle, and every notification is a
//! typed publish on the core event bus. Hosts call these from their blocking
//! pools; nothing here assumes an async runtime.

use crate::events::CoreEvent;
use crate::model::*;
use crate::runtime::BridgeCore;
use crate::sessions;
use crate::{
    adapters, agent, agent_config, compaction_controller, completion, delegation, handoff,
    learning_job, learning_router, orchestrator, policy, policy_coordinator, prompt_compiler, restoration,
    secret_interception, session_forest, session_supervisor, skill_marketplace, slash, store,
    worker_guard, worker_lifecycle, worker_pool, worker_sandbox, workspace_files,
    worktree_coordinator, BridgeError, WORKER_STALL_TIMEOUT_SECONDS,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::{
    collections::HashMap,
    io::BufRead,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use uuid::Uuid;

/// The invoking user's home directory, for skill-store scans.
pub fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Capabilities from available adapters plus installed skills. Blocking (it
/// scans the skill store); callers already run on worker threads.
pub fn live_available_capabilities(state: &BridgeCore) -> std::collections::HashSet<String> {
    let mut capabilities = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .flat_map(|descriptor| descriptor.capabilities)
        .collect::<std::collections::HashSet<_>>();
    if let Ok(skills) =
        skill_marketplace::available_capabilities(&user_home(), &state.skill_store)
    {
        capabilities.extend(skills);
    }
    capabilities
}

fn compile_orchestrator_prompt(
    configured_prompt: &str,
    credential_context: &str,
    checkpoint_context: Option<&str>,
) -> Result<prompt_compiler::CompiledPrompt, BridgeError> {
    let mut compiler = prompt_compiler::PromptCompiler::new("orchestrator")
        .stable_section("bridge_role", orchestrator::briefing())
        .stable_section("delegation_protocol", delegation::protocol(0))
        .project_rule("configured_project_rules", configured_prompt)
        .variable_section("session_capabilities", credential_context);
    if let Some(context) = checkpoint_context {
        compiler = compiler.variable_section("restoration_context", context);
    }
    compiler.compile()
}

fn compile_session_prompt(
    configured_prompt: &str,
    credential_context: &str,
) -> Result<prompt_compiler::CompiledPrompt, BridgeError> {
    prompt_compiler::PromptCompiler::new("session")
        .project_rule("configured_project_rules", configured_prompt)
        .variable_section("session_capabilities", credential_context)
        .compile()
}

fn compile_worker_prompt(
    directive: &delegation::DelegationRequest,
    depth: i64,
    branch: &str,
    evidence: &[delegation::WorkerEvidence],
    configured_prompt: &str,
    credential_context: &str,
    checkpoint_context: Option<&str>,
) -> Result<prompt_compiler::CompiledPrompt, BridgeError> {
    let mut compiler = prompt_compiler::PromptCompiler::new(format!("worker:{}", directive.role.as_str()))
        .stable_section("worker_contract", delegation::worker_contract(directive.role, depth))
        .project_rule("configured_project_rules", configured_prompt)
        .variable_section("task_context", delegation::worker_task_context(directive, branch, evidence))
        .variable_section("session_capabilities", credential_context);
    if let Some(context) = checkpoint_context {
        compiler = compiler.variable_section("restoration_context", context);
    }
    compiler.compile()
}

pub fn persist_prompt_compilation(
    db: &Connection,
    session_id: &str,
    harness: &str,
    model: Option<&str>,
    role: &str,
    task_family: &str,
    restoration_mode: RestorationMode,
    cross_harness_reuse: &str,
    prompt: &prompt_compiler::CompiledPrompt,
) -> Result<i64, BridgeError> {
    store::record_prompt_compilation(
        db,
        &PromptCompilationRecord {
            id: 0,
            session_id: session_id.into(),
            turn_id: None,
            prefix_id: prompt.metadata.prefix_id.clone(),
            prefix_hash: prompt.metadata.prefix_hash.clone(),
            schema_version: i64::from(prompt.metadata.schema_version),
            prefix_bytes: prompt.metadata.prefix_bytes as i64,
            prefix_token_estimate: prompt.metadata.prefix_token_estimate as i64,
            harness: harness.into(),
            model: model.map(str::to_owned),
            role: role.into(),
            task_family: task_family.into(),
            restoration_mode: restoration_mode.as_str().into(),
            cross_harness_reuse: cross_harness_reuse.into(),
            created_at: Utc::now().to_rfc3339(),
        },
    )
}

pub fn cross_harness_reuse_marker(
    db: &Connection,
    parent_session_id: &str,
    child_harness: &str,
) -> &'static str {
    match db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![parent_session_id],
        |row| row.get::<_, String>(0),
    ).ok().as_deref() {
        Some(parent) if parent == child_harness => "same_harness",
        Some(_) => "incompatible",
        None => "not_applicable",
    }
}

pub fn start_session(
    core: &Arc<BridgeCore>,
    workspace_id: String,
    harness: Option<Harness>,
    model: Option<String>,
) -> Result<BridgeState, BridgeError> {
    let state = core;
    // An explicit chat choice wins. Without one, the persisted Standard
    // orchestrator profile remains the default.
    let selection = if let Some(harness) = harness {
        let adapter_id = store::harness_name(&harness).to_owned();
        let db = state.db.lock().unwrap();
        if !agent_config::is_harness_enabled(&db, &adapter_id) {
            return Err(BridgeError::Invalid(format!(
                "{} is disabled in Settings",
                harness.label()
            )));
        }
        let descriptor = state
            .adapter_registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == adapter_id)
            .ok_or_else(|| BridgeError::Invalid(format!("No model adapter is registered for {adapter_id}")))?;
        if !descriptor.available {
            return Err(BridgeError::Invalid(
                descriptor
                    .unavailable_reason
                    .unwrap_or_else(|| format!("{} is unavailable", descriptor.label)),
            ));
        }
        let selected = if let Some(requested) = model.as_deref().filter(|value| !value.trim().is_empty()) {
            descriptor
                .models
                .iter()
                .find(|option| option.id.eq_ignore_ascii_case(requested.trim()))
                .cloned()
                .ok_or_else(|| BridgeError::Invalid(format!("{} does not offer model {requested}", descriptor.label)))?
        } else {
            descriptor
                .models
                .iter()
                .find(|option| option.tier == CapabilityTier::Standard && option.default_for_tier)
                .or_else(|| descriptor.models.iter().find(|option| option.tier == CapabilityTier::Standard))
                .cloned()
                .ok_or_else(|| BridgeError::Invalid(format!("{} has no standard model", descriptor.label)))?
        };
        sessions::OrchestratorSelection {
            adapter_id,
            model: selected.id,
            tier: selected.tier,
            effort: agent_config::harness_config(&db, store::harness_name(&harness))
                .and_then(|config| config.effort),
            label: agent_config::default_orchestrator(&db)
                .map(|agent| agent.name)
                .unwrap_or_else(|| orchestrator::SESSION_LABEL.into()),
        }
    } else {
        let db = state.db.lock().unwrap();
        sessions::resolve_orchestrator_selection(&db, &state.adapter_registry)?
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
    // Exclusive with model switches (and other starts) on this session for
    // the rest of the launch flow.
    let _lifecycle = state.claim_session_lifecycle(&session_id, "session start")?;
    let path = path.filter(|value| !value.is_empty()).unwrap_or_else(|| {
        state
            .chat_scratch_dir(&session_id)
            .to_string_lossy()
            .to_string()
    });
    std::fs::create_dir_all(&path)?;
    let configured_prompt = agent_config::orchestrator_prompt(&state.db.lock().unwrap(), adapter_id);
    let credential_context = state.credential_broker.instructions(&session_id);
    let orchestrator_prompt = compile_orchestrator_prompt(&configured_prompt, &credential_context, None)?;
    let orchestrator_instructions = orchestrator_prompt.instructions().to_owned();
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
        let hot_prompt_compatible = store::latest_prompt_compilation(
            &state.db.lock().unwrap(),
            &session_id,
        )?.is_some_and(|previous| {
            previous.harness == adapter_id
                && previous.prefix_hash == orchestrator_prompt.metadata.prefix_hash
                && previous.schema_version == i64::from(orchestrator_prompt.metadata.schema_version)
        });
        if current_model.as_deref() == chosen_model.as_deref() && hot_prompt_compatible {
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
            persist_prompt_compilation(
                &db,
                &session_id,
                adapter_id,
                chosen_model.as_deref(),
                "orchestrator",
                "orchestration",
                RestorationMode::Hot,
                "not_applicable",
                &orchestrator_prompt,
            )?;
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
    let checkpoint_instructions = checkpoint_context.as_deref().map(|context| {
        compile_orchestrator_prompt(&configured_prompt, &credential_context, Some(context))
            .map(|prompt| prompt.instructions().to_owned())
    }).transpose()?;
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
    if let Err(error) = persist_prompt_compilation(
        &db,
        &session_id,
        adapter_id,
        chosen_model.as_deref(),
        "orchestrator",
        "orchestration",
        restoration_mode,
        "not_applicable",
        &orchestrator_prompt,
    ) {
        let _ = db.execute("UPDATE sessions SET status='failed' WHERE id=?1", params![session_id]);
        drop(db);
        started.runtime.stop(adapters::ShutdownReason::Failed);
        return Err(error);
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
        core.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    core.events.publish(CoreEvent::StateChanged);
    store::state(&state.db.lock().unwrap())
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with the
/// routing briefing + delegation protocol (workers enabled via the reader gate).
pub fn start_chat(
    core: &Arc<BridgeCore>,
    session_id: String,
) -> Result<BridgeState, BridgeError> {
    let state = core;
    // Exclusive with model switches (and other starts) on this session: the
    // switch flow tears the adapter down across an await, and a start
    // interleaving into that window would be orphaned by its commit.
    let _lifecycle = state.claim_session_lifecycle(&session_id, "session start")?;
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
                state
                    .chat_scratch_dir(&session_id)
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
    let compiled_prompt = if is_orchestrator {
        compile_orchestrator_prompt(&configured_prompt, &proxy_instructions, None)?
    } else {
        compile_session_prompt(&configured_prompt, &proxy_instructions)?
    };
    let runtime_instructions = compiled_prompt.instructions().to_owned();
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
    let (mut started, mode, eligibility) = (match resumable {
            Some(provider) => match registry.resume(
                &launch_adapter_id,
                adapters::ResumeRequest {
                    provider_session_id: &provider,
                    cwd: &launch_cwd,
                    model: launch_model.as_deref(),
                    effort: chosen_effort.as_deref(),
                    instructions: Some(&runtime_instructions),
                    write_mode: None,
                    read_only_sandbox: None,
                },
            ) {
                Ok(started) => Ok((
                    started,
                    RestorationMode::Native,
                    ResumeEligibility::Native,
                )),
                Err(_) => registry.start(
                    &launch_adapter_id,
                    adapters::StartRequest {
                        cwd: &launch_cwd,
                        model: launch_model.as_deref(),
                        effort: chosen_effort.as_deref(),
                        instructions: Some(&runtime_instructions),
                        write_mode: None,
                        read_only_sandbox: None,
                    },
                )
                .map(|started| (
                    started,
                    RestorationMode::Fresh,
                    ResumeEligibility::Fresh,
                )),
            },
            None => registry
                .start(
                    &launch_adapter_id,
                    adapters::StartRequest {
                        cwd: &launch_cwd,
                        model: launch_model.as_deref(),
                        effort: chosen_effort.as_deref(),
                        instructions: Some(&runtime_instructions),
                        write_mode: None,
                        read_only_sandbox: None,
                    },
                )
                .map(|started| (started, RestorationMode::Fresh, ResumeEligibility::Fresh)),
        })?;
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
        if let Err(error) = persist_prompt_compilation(
            &db,
            &session_id,
            adapter_id,
            chosen_model.as_deref(),
            if is_orchestrator { "orchestrator" } else { "session" },
            if is_orchestrator { "orchestration" } else { "direct" },
            mode,
            "not_applicable",
            &compiled_prompt,
        ) {
            let _ = db.execute("UPDATE sessions SET status='failed' WHERE id=?1", params![session_id]);
            drop(db);
            started.runtime.stop(adapters::ShutdownReason::Failed);
            return Err(error);
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
        core.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    core.events.publish(CoreEvent::StateChanged);
    store::state(&state.db.lock().unwrap())
}

/// Drive one structured session's stdout: normalize every frame, then on exit
/// mark the session stopped and unblock any parent that was waiting on it.
fn spawn_reader_thread(
    core: Arc<BridgeCore>,
    session_id: String,
    launch_started_at: String,
    current_turn: Arc<Mutex<Option<String>>>,
    mut reader: Box<dyn BufRead + Send>,
) {
    thread::spawn(move || {
        let tracks_worker = store::worker_runtime(
            &core.clone().db.lock().unwrap(),
            &session_id,
        )
        .ok()
        .flatten()
        .is_some();
        // Seed a heartbeat so a worker that never emits a single line still has
        // a baseline the stall watchdog can measure from.
        if tracks_worker {
            record_worker_activity(&core, &session_id);
        }
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // Every line proves liveness — refresh the heartbeat before
                    // normalization so tool-run and reasoning frames all count.
                    if tracks_worker {
                        record_worker_activity(&core, &session_id);
                    }
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                        handle_agent_value(&core, &session_id, &current_turn, &value);
                    }
                }
            }
        }
        if tracks_worker {
            core.clone()
                .worker_activity
                .lock()
                .unwrap()
                .remove(&session_id);
            core.clone()
                .worker_activity_persisted
                .lock()
                .unwrap()
                .remove(&session_id);
        }
        let state = core.clone();
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
        notify_parent_on_worker_exit(&core, &session_id);
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
        core.events.publish(CoreEvent::StateChanged);
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

pub fn agent_event_changes_bridge_state(event: &agent::NormalizedEvent) -> bool {
    matches!(
        event.kind.as_str(),
        "turn.started" | "turn.completed" | "approval.requested" | "usage.updated"
    ) || (event.kind == "error" && event.status.as_deref() == Some("failed"))
}

fn handle_agent_value(
    core: &Arc<BridgeCore>,
    session_id: &str,
    current_turn: &Arc<Mutex<Option<String>>>,
    value: &serde_json::Value,
) {
    // Codex account rate-limit frames (the reply to `account/rateLimits/read`
    // and its rolling push) are subscription telemetry, not conversation. Route
    // them straight to the ambient usage channel without persisting.
    if let Some(rate_limits) = codex_rate_limits_from_frame(value) {
        core.clone().publish_account_usage("codex", rate_limits);
        return;
    }
    let state = core.clone();
    let mut pending_directives: Vec<(delegation::DelegationRequest, String)> = Vec::new();
    let mut pending_invalid_delegations: Vec<String> = Vec::new();
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
        let mut observed_turn_id = current_turn
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
        // The exact composer text is persisted locally at submission time.
        // Provider echoes may include hidden user-role file context, so do not
        // duplicate them into the visible conversation.
        let normalized = state
            .adapter_registry
            .normalize(&adapter_id, value)
            .into_iter()
            .filter(|event| {
                event.role.as_deref() != Some("user") || !event.kind.starts_with("message.")
            })
            .collect::<Vec<_>>();
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
                        observed_turn_id = Some(turn_id.clone());
                        let _ = store::bind_latest_prompt_compilation_to_turn(
                            &db,
                            session_id,
                            turn_id,
                        );
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
                            // Deferred: feed the reason back to the orchestrator
                            // (after the lock) so it re-emits a valid request
                            // instead of silently going idle with no result.
                            pending_invalid_delegations.push(reason.clone());
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                format!("_Delegation request rejected: {reason}. Correcting and retrying…_")
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
                // Publish while the database mutex is still held. This keeps
                // durable live delivery in commit/sequence order: another
                // thread cannot persist and publish sequence N+1 before N.
                state.events.publish(CoreEvent::Agent(event));
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
            if let Err(error) = send_internal_checkpoint_turn(core, session_id, &prompt) {
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
        finish_worker_checkpoint(core, session_id, adapters::ShutdownReason::Completed);
    }
    if recover_compaction {
        let _ = run_compaction_recovery(core, session_id);
    }
    if finish_requested_shutdown {
        finish_orchestrator_shutdown(core, session_id, adapters::ShutdownReason::UserStopped);
    }

    for (directive, turn_id) in &pending_directives {
        let _ = launch_worker(core, session_id, turn_id, directive, true);
    }
    if !pending_directives.is_empty() {
        // A valid request cleared the backlog; reset the correction budget.
        state
            .delegations
            .lock()
            .unwrap()
            .invalid_request_corrections
            .remove(session_id);
    }
    // A rejected `bridge-delegate` request never launched a worker. Surface it
    // as a distinct row and feed the reason back so the orchestrator re-emits a
    // valid request, rather than going idle with no result the user can see.
    for reason in &pending_invalid_delegations {
        const MAX_INVALID_REQUEST_CORRECTIONS: u32 = 3;
        let attempts = {
            let mut delegations = state.delegations.lock().unwrap();
            let counter = delegations
                .invalid_request_corrections
                .entry(session_id.to_owned())
                .or_insert(0);
            *counter += 1;
            *counter
        };
        let will_retry = attempts <= MAX_INVALID_REQUEST_CORRECTIONS;
        {
            let db = state.db.lock().unwrap();
            let rejection = agent::NormalizedEvent {
                kind: "delegation.rejected".into(),
                item_id: Some(format!("rejected-{}", Uuid::new_v4())),
                role: Some("system".into()),
                status: Some("failed".into()),
                title: Some("Delegation rejected".into()),
                text: Some(reason.clone()),
                data: serde_json::json!({
                    "reason": reason,
                    "willRetry": will_retry,
                    "attempt": attempts,
                }),
            };
            if let Ok(stored) = store::session_event(
                &db,
                session_id,
                &rejection,
                &serde_json::json!({"delegation": true}),
            ) {
                core.events.publish(CoreEvent::Agent(stored));
            }
        }
        if will_retry {
            let prompt = delegation::invalid_request_feedback(reason);
            let delivered = state
                .adapters
                .lock()
                .unwrap()
                .get(session_id)
                .is_some_and(|runtime| runtime.send_turn(&prompt).is_ok());
            let db = state.db.lock().unwrap();
            if delivered {
                let _ = db.execute(
                    "UPDATE sessions SET status='working' WHERE id=?1 AND ended_at IS NULL",
                    params![session_id],
                );
            } else {
                let _ = store::event(
                    &db,
                    "delegation",
                    "delegation.correction.undeliverable",
                    session_id,
                    reason,
                );
            }
        } else {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "delegation",
                "delegation.correction.exhausted",
                session_id,
                reason,
            );
        }
    }
    // When this session's own turn ends and it is not waiting on any child
    // worker, hand its result up to its parent (no-op if it has no parent).
    if turn_completed && !checkpoint_response_seen && !checkpoint_turn_handled {
        let idle =
            store::outstanding_children(&state.db.lock().unwrap(), session_id).unwrap_or(0) == 0;
        if idle {
            forward_turn_result(core, session_id);
        }
    }
    if bridge_state_changed {
        core.events.publish(CoreEvent::StateChanged);
    }
}

pub fn begin_pressure_compaction(
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

pub fn send_internal_checkpoint_turn(
    core: &Arc<BridgeCore>,
    session_id: &str,
    prompt: &str,
) -> Result<(), BridgeError> {
    let state = core.clone();
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

fn finish_worker_checkpoint(core: &Arc<BridgeCore>, session_id: &str, reason: adapters::ShutdownReason) {
    let state = core.clone();
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
    core: &Arc<BridgeCore>,
    session_id: &str,
    reason: adapters::ShutdownReason,
) {
    let state = core.clone();
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
    core.events.publish(CoreEvent::StateChanged);
}

fn run_compaction_recovery(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    let state = core.clone();
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
pub struct WorkerLaunchReservation {
    pub session_id: String,
    pub workspace_id: String,
    pub depth: i64,
    pub path: String,
    pub branch: String,
    pub actual_model: String,
    pub outcome: policy::PolicyOutcome,
    pub reuse_existing: bool,
}

pub enum WorkerReservationOutcome {
    Reserved(WorkerLaunchReservation),
    Queued,
    Blocked,
}

pub enum WorkerLaunchOutcome {
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

pub fn record_model_resolution_warning(
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

pub fn reserve_worker_launch_outcome(
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
            last_activity_at: None,
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

/// Test support: reserve a worker launch without spawning it. Kept public
/// (not cfg(test)) so downstream-crate tests can exercise reservations.
pub fn reserve_worker_launch(
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

pub fn launch_worker_outcome(
    core: &Arc<BridgeCore>,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> WorkerLaunchOutcome {
    let state = core.clone();
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
            drop(db);
            report_worker_launch_failure(core, parent_session_id, "routing", &error.to_string());
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
        report_worker_launch_failure(
            core,
            parent_session_id,
            "capability",
            &format!("{harness} is disabled in Settings"),
        );
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
            report_worker_launch_failure(
                core,
                parent_session_id,
                "model_resolution",
                &error.to_string(),
            );
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
            core.events.publish(CoreEvent::StateChanged);
            return WorkerLaunchOutcome::Queued;
        }
        Ok(WorkerReservationOutcome::Blocked) => {
            let _ = learning_router::record_route_status(
                &state.db.lock().unwrap(),
                &routed.decision.id,
                "policy_blocked",
            );
            report_worker_launch_failure(
                core,
                parent_session_id,
                "policy",
                "Worker launch was blocked by delegation policy",
            );
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
            report_worker_launch_failure(core, parent_session_id, "policy", &error.to_string());
            return WorkerLaunchOutcome::Failed;
        }
    };
    if let Err(error) = learning_router::bind_worker(
        &state.db.lock().unwrap(),
        &routed.decision.id,
        &reservation.session_id,
    ) {
        fail_reserved_worker(
            core,
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
            core,
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
                    core,
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
                if !queue_on_block {
                    let _ = learning_router::record_route_status(
                        &state.db.lock().unwrap(),
                        &routed.decision.id,
                        "worktree_failed",
                    );
                    fail_reserved_worker(
                        core,
                        &reservation.session_id,
                        &directive.label(),
                        &format!("Could not prepare isolated worker worktree: {error}"),
                    );
                    return WorkerLaunchOutcome::Failed;
                }
                let db = state.db.lock().unwrap();
                if let Err(cleanup_error) = delete_reserved_worker(&db, &reservation.session_id) {
                    drop(db);
                    fail_reserved_worker(
                        core,
                        &reservation.session_id,
                        &directive.label(),
                        &format!(
                            "Could not prepare isolated worker worktree ({error}) or clean up its reservation: {cleanup_error}"
                        ),
                    );
                    return WorkerLaunchOutcome::Failed;
                }
                let queued = worker_pool::WorkerPool::enqueue(
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
                    if queued {
                        "worker.worktree_queued"
                    } else {
                        "worker.worktree_failed"
                    },
                    parent_session_id,
                    &error.to_string(),
                );
                let _ = learning_router::record_route_status(
                    &db,
                    &routed.decision.id,
                    if queued { "queued" } else { "worktree_failed" },
                );
                drop(db);
                if queued {
                    core.events.publish(CoreEvent::StateChanged);
                    return WorkerLaunchOutcome::Queued;
                }
                report_worker_launch_failure(
                    core,
                    parent_session_id,
                    "worktree",
                    &format!("Could not prepare or queue isolated worker worktree: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
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
                core,
                &reservation.session_id,
                &label,
                &format!("Could not resolve worker evidence: {error}"),
            );
            return WorkerLaunchOutcome::Failed;
        }
    };
    let role = directive.role.as_str();
    let configured_prompt = agent_config::prompt_suffix(&state.db.lock().unwrap(), &harness, role);
    let credential_context = state.credential_broker.instructions(&reservation.session_id);
    let compiled_prompt = match compile_worker_prompt(
        directive,
        reservation.depth,
        &reservation.branch,
        &evidence,
        &configured_prompt,
        &credential_context,
        None,
    ) {
        Ok(prompt) => prompt,
        Err(error) => {
            fail_reserved_worker(core, &reservation.session_id, &label, &format!("Could not compile worker prompt: {error}"));
            return WorkerLaunchOutcome::Failed;
        }
    };
    let mut instructions = compiled_prompt.instructions().to_owned();
    let hot_prompt_compatible = store::latest_prompt_compilation(
        &state.db.lock().unwrap(),
        &reservation.session_id,
    ).ok().flatten().is_some_and(|previous| {
        previous.harness == harness
            && previous.model.as_deref() == Some(model.as_str())
            && previous.prefix_hash == compiled_prompt.metadata.prefix_hash
            && previous.schema_version == i64::from(compiled_prompt.metadata.schema_version)
    });

    if reservation.reuse_existing
        && state.adapters.lock().unwrap().contains_key(&reservation.session_id)
        && hot_prompt_compatible
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
        let activation_result = transition_result.and_then(|_| {
            worker_pool::WorkerPool::activate_reused_worker(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                &reservation.workspace_id,
                directive,
            )
        });
        if activation_result.is_ok() {
            // Reused warm workers keep their previous heartbeat; reset it so the
            // stall watchdog measures from the start of this task, not the last.
            reset_worker_heartbeat(&state, &reservation.session_id);
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
            let prompt_record_id = {
                let db = state.db.lock().unwrap();
                let marker = cross_harness_reuse_marker(&db, parent_session_id, &harness);
                persist_prompt_compilation(
                    &db,
                    &reservation.session_id,
                    &harness,
                    Some(&model),
                    &format!("worker:{}", directive.role.as_str()),
                    directive.role.as_str(),
                    RestorationMode::Hot,
                    marker,
                    &compiled_prompt,
                )
            };
            let prompt_record_id = match prompt_record_id {
                Ok(id) => id,
                Err(error) => {
                    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&reservation.session_id) {
                        runtime.stop(adapters::ShutdownReason::Failed);
                    }
                    let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
                        &state.db.lock().unwrap(),
                        &reservation.session_id,
                    );
                    fail_reserved_worker(core, &reservation.session_id, &label, &format!("Could not persist hot prompt compilation: {error}"));
                    return WorkerLaunchOutcome::Failed;
                }
            };
            let delivery = state
                .adapters
                .lock()
                .unwrap()
                .get(&reservation.session_id)
                .ok_or_else(|| BridgeError::Invalid("Hot worker runtime disappeared before prompt delivery".into()))
                .and_then(|runtime| runtime.send_turn(&instructions));
            if let Err(error) = delivery {
                if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&reservation.session_id) {
                    runtime.stop(adapters::ShutdownReason::Failed);
                }
                let db = state.db.lock().unwrap();
                let _ = session_supervisor::SessionSupervisor::clear_adapter_process(&db, &reservation.session_id);
                let _ = store::delete_prompt_compilation(&db, prompt_record_id);
                drop(db);
                fail_reserved_worker(core, &reservation.session_id, &label, &format!("Could not deliver hot worker prompt: {error}"));
                let _ = store::event(&state.db.lock().unwrap(), "worker-pool", "worker.hot_resume_failed", &reservation.session_id, &error.to_string());
                return WorkerLaunchOutcome::Failed;
            }
            let _ = learning_router::record_route_status(
                &state.db.lock().unwrap(),
                &routed.decision.id,
                "launched",
            );
            core.events.publish(CoreEvent::StateChanged);
            return WorkerLaunchOutcome::Launched(reservation.session_id);
        }
        let error = activation_result.unwrap_err();
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&reservation.session_id) {
            runtime.stop(adapters::ShutdownReason::Failed);
        }
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &reservation.session_id,
        );
        fail_reserved_worker(core, &reservation.session_id, &label, &format!("Could not reactivate compatible hot worker: {error}"));
        let _ = store::event(&state.db.lock().unwrap(), "worker-pool", "worker.hot_resume_failed", &reservation.session_id, &error.to_string());
        return WorkerLaunchOutcome::Failed;
    }

    if reservation.reuse_existing
        && state.adapters.lock().unwrap().contains_key(&reservation.session_id)
        && !hot_prompt_compatible
    {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&reservation.session_id) {
            runtime.stop(adapters::ShutdownReason::Replaced);
        }
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &reservation.session_id,
        );
        let _ = store::event(
            &state.db.lock().unwrap(),
            "prompt-cache",
            "worker.prompt_prefix_changed",
            &reservation.session_id,
            "Restarting worker because the stable prompt prefix changed",
        );
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
                    core,
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
                let output = sandbox.output_dir().display().to_string();
                let network_allowed = sandbox.network_allowed();
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
                        network_allowed
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
                    core,
                    &reservation.session_id,
                    &label,
                    &format!("Could not establish read-only OS isolation: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
            }
        }
    }

    let compile_restored_prompt = |checkpoint: Option<String>| {
        let restoration_context = checkpoint.unwrap_or_else(|| "Bridge checkpoint-restoration context: prior typed worker result is stored in the session forest.".into());
        compile_worker_prompt(
            directive,
            reservation.depth,
            &reservation.branch,
            &evidence,
            &configured_prompt,
            &credential_context,
            Some(&restoration_context),
        ).map(|prompt| prompt.instructions().to_owned())
    };
    let activation = if reservation.reuse_existing {
        if let Err(error) = session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            &reservation.session_id,
            worker_lifecycle::WorkerLifecycleState::Resuming,
            Some("compatible_cold_task"),
        ) {
            verify_read_only_worker(core, &reservation.session_id);
            fail_reserved_worker(
                core,
                &reservation.session_id,
                &label,
                &format!("Could not transition reused worker to resuming: {error}"),
            );
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
                let _ = restoration::record_resume_failed(&state.db.lock().unwrap(), &reservation.session_id, &error.to_string());
                compile_restored_prompt(checkpoint).and_then(|restored_instructions| state.adapter_registry.start(&harness, adapters::StartRequest {
                    cwd: &reservation.path,
                    model: Some(model.as_str()),
                    effort: Some(&effort),
                    instructions: Some(restored_instructions.as_str()),
                    write_mode: Some(directive.write_mode),
                    read_only_sandbox: read_only_sandbox.as_ref(),
                })).map(|started| (started, WorkerActivation::CheckpointRestored))
            }
            Ok(None) => {
                compile_restored_prompt(checkpoint).and_then(|restored_instructions| state.adapter_registry.start(&harness, adapters::StartRequest {
                    cwd: &reservation.path,
                    model: Some(model.as_str()),
                    effort: Some(&effort),
                    instructions: Some(restored_instructions.as_str()),
                    write_mode: Some(directive.write_mode),
                    read_only_sandbox: read_only_sandbox.as_ref(),
                })).map(|started| (started, WorkerActivation::CheckpointRestored))
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
                core,
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
        verify_read_only_worker(core, &session_id);
        fail_reserved_worker(
            core,
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
        verify_read_only_worker(core, &session_id);
        fail_reserved_worker(
            core,
            &session_id,
            &label,
            &format!("Could not transition worker to working: {error}"),
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
    if let Err(error) = restoration::set_head_state(
        &state.db.lock().unwrap(),
        &session_id,
        restoration_mode,
        resume_eligibility,
        Some(&thread_id),
    )
    {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        verify_read_only_worker(core, &session_id);
        fail_reserved_worker(
            core,
            &session_id,
            &label,
            &format!("Could not persist worker restoration state: {error}"),
        );
        return WorkerLaunchOutcome::Failed;
    }
    if let Err(error) = handoff::record_fidelity(
        &state.db.lock().unwrap(),
        &session_id,
        continuation_fidelity,
    )
    {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
            &state.db.lock().unwrap(),
            &session_id,
        );
        verify_read_only_worker(core, &session_id);
        fail_reserved_worker(
            core,
            &session_id,
            &label,
            &format!("Could not persist worker continuation fidelity: {error}"),
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
        let cross_harness_reuse = cross_harness_reuse_marker(&db, parent_session_id, &harness);
        if let Err(error) = persist_prompt_compilation(
            &db,
            &session_id,
            &harness,
            Some(&model),
            &format!("worker:{}", directive.role.as_str()),
            directive.role.as_str(),
            restoration_mode,
            cross_harness_reuse,
            &compiled_prompt,
        ) {
            drop(db);
            runtime.stop(adapters::ShutdownReason::Failed);
            let _ = session_supervisor::SessionSupervisor::clear_adapter_process(&state.db.lock().unwrap(), &session_id);
            fail_reserved_worker(core, &session_id, &label, &format!("Could not persist prompt compilation: {error}"));
            return WorkerLaunchOutcome::Failed;
        }
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
            core.events.publish(CoreEvent::Agent(stored));
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

    if reservation.reuse_existing {
        if let Err(error) = worker_pool::WorkerPool::activate_reused_worker(
            &state.db.lock().unwrap(),
            &session_id,
            &reservation.workspace_id,
            directive,
        ) {
            runtime.stop(adapters::ShutdownReason::Failed);
            let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
                &state.db.lock().unwrap(),
                &session_id,
            );
            verify_read_only_worker(core, &session_id);
            fail_reserved_worker(
                core,
                &session_id,
                &label,
                &format!("Could not activate reused worker: {error}"),
            );
            return WorkerLaunchOutcome::Failed;
        }
    }
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), runtime);
    spawn_reader_thread(
        core.clone(),
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
            core,
            &session_id,
            &label,
            &format!("Could not deliver worker objective: {error}"),
        );
        verify_read_only_worker(core, &session_id);
        return WorkerLaunchOutcome::Failed;
    }
    core.events.publish(CoreEvent::StateChanged);
    let _ = learning_router::record_route_status(
        &state.db.lock().unwrap(),
        &routed.decision.id,
        "launched",
    );
    WorkerLaunchOutcome::Launched(session_id)
}

fn report_worker_launch_failure(
    core: &Arc<BridgeCore>,
    parent_session_id: &str,
    phase: &str,
    reason: &str,
) {
    let state = core.clone();
    let routing_notice = serde_json::json!({
        "type": "bridge-worker-launch-failed",
        "phase": phase,
        "reason": reason,
        "instruction": "No worker started. Do not wait for a result. Tell the user what failed, then retry only if a different route can address the failure."
    })
    .to_string();
    let delivered = state
        .adapters
        .lock()
        .unwrap()
        .get(parent_session_id)
        .is_some_and(|runtime| runtime.send_turn(&routing_notice).is_ok());
    let event = agent::NormalizedEvent {
        kind: "delegation.rejected".into(),
        item_id: Some(format!("launch-failed-{}", Uuid::new_v4())),
        role: Some("system".into()),
        status: Some("failed".into()),
        title: Some("Worker failed to start".into()),
        text: Some(reason.to_owned()),
        data: serde_json::json!({
            "launchFailed": true,
            "phase": phase,
            "reason": reason,
            "willRetry": false,
            "orchestratorNotified": delivered,
        }),
    };
    if let Ok(stored) = store::session_event(
        &state.db.lock().unwrap(),
        parent_session_id,
        &event,
        &serde_json::json!({"delegation": true}),
    ) {
        core.events.publish(CoreEvent::Agent(stored));
    }
    core.events.publish(CoreEvent::StateChanged);
}

pub fn record_actual_execution_best_effort(
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

pub fn deliver_worker_objective(
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
    core: &Arc<BridgeCore>,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> Option<String> {
    match launch_worker_outcome(core, parent_session_id, turn_id, directive, queue_on_block) {
        WorkerLaunchOutcome::Launched(session_id) => Some(session_id),
        WorkerLaunchOutcome::Queued | WorkerLaunchOutcome::Failed => None,
    }
}

fn fail_reserved_worker(core: &Arc<BridgeCore>, session_id: &str, label: &str, reason: &str) {
    let state = core.clone();
    if prepare_worker_failure_settlement(&state.db.lock().unwrap(), session_id).is_err() {
        let parent_session_id = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT parent_session_id FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        if let Some(parent_session_id) = parent_session_id {
            report_worker_launch_failure(core, &parent_session_id, "settlement", reason);
        }
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
    match settle_worker_after_result(core, session_id, &result) {
        Ok(true) => report_to_parent(core, session_id, &result),
        Ok(false) | Err(_) => {
            let parent_session_id = state
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT parent_session_id FROM sessions WHERE id=?1",
                    params![session_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            if let Some(parent_session_id) = parent_session_id {
                report_worker_launch_failure(core, &parent_session_id, "settlement", reason);
            }
        }
    }
}

fn delete_reserved_worker(db: &Connection, session_id: &str) -> Result<(), BridgeError> {
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM worker_runtime WHERE session_id=?1",
        params![session_id],
    )?;
    transaction.execute(
        "DELETE FROM worker_leases WHERE session_id=?1",
        params![session_id],
    )?;
    transaction.execute("DELETE FROM sessions WHERE id=?1", params![session_id])?;
    transaction.commit()?;
    Ok(())
}

pub fn prepare_worker_failure_settlement(db: &Connection, session_id: &str) -> Result<(), BridgeError> {
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
fn forward_turn_result(core: &Arc<BridgeCore>, child_session_id: &str) {
    let state = core.clone();
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
        core.events.publish(CoreEvent::StateChanged);
        return;
    };
    let _ = (label, harness, model, effort);
    match settle_worker_after_result(core, child_session_id, &result) {
        Ok(true) => verify_read_only_worker(core, child_session_id),
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
    report_to_parent(core, child_session_id, &result);
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

pub fn process_worker_result_output(
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
    core: &Arc<BridgeCore>,
    child_session_id: &str,
    result: &delegation::WorkerResult,
) -> Result<bool, BridgeError> {
    let state = core.clone();
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
/// Refresh a session's liveness heartbeat for the stall watchdog.
fn reset_worker_heartbeat(state: &BridgeCore, session_id: &str) {
    state
        .worker_activity
        .lock()
        .unwrap()
        .insert(session_id.to_string(), std::time::Instant::now());
}

fn record_worker_activity(core: &Arc<BridgeCore>, session_id: &str) {
    let state = core.clone();
    reset_worker_heartbeat(&state, session_id);
    let should_persist = {
        let mut persisted = state.worker_activity_persisted.lock().unwrap();
        let should_persist = persisted
            .get(session_id)
            .is_none_or(|seen| seen.elapsed() >= Duration::from_secs(2));
        if should_persist {
            persisted.insert(session_id.to_owned(), std::time::Instant::now());
        }
        should_persist
    };
    if should_persist {
        let _ = state.db.lock().unwrap().execute(
            "UPDATE worker_runtime SET last_activity_at=?2 WHERE session_id=?1 AND result_status='pending'",
            params![session_id, Utc::now().to_rfc3339()],
        );
    }
}

/// Seconds since a session last produced output, if it is being tracked.
fn worker_silence_secs(state: &BridgeCore, session_id: &str) -> Option<u64> {
    state
        .worker_activity
        .lock()
        .unwrap()
        .get(session_id)
        .map(|seen| seen.elapsed().as_secs())
}

/// Return an unreported worker's label (with a parent) or None. Shared guard for
/// the process-exit and stall failure paths; `record_result` is idempotent on
/// `result_status="reported"`, so a later real EOF won't double-report.
fn unreported_worker_meta(core: &Arc<BridgeCore>, child_session_id: &str) -> Option<String> {
    let state = core.clone();
    let reported = store::worker_runtime(&state.db.lock().unwrap(), child_session_id)
        .ok()
        .flatten()
        .is_some_and(|runtime| runtime.result_status == "reported");
    if reported {
        return None;
    }
    let (parent, label): (Option<String>, String) = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT parent_session_id,label FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    parent.map(|_| label)
}

/// Route a synthesized failure result through the same settle + report seam the
/// happy path uses, releasing the parent's outstanding-child count and emitting
/// a `delegation.result` to the UI.
fn report_synthetic_worker_failure(
    core: &Arc<BridgeCore>,
    child_session_id: &str,
    result: &delegation::WorkerResult,
) {
    match settle_worker_after_result(core, child_session_id, result) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = store::event(
                &core.clone().db.lock().unwrap(),
                "supervisor",
                "worker.settle_failed",
                child_session_id,
                &error.to_string(),
            );
            return;
        }
    }
    report_to_parent(core, child_session_id, result);
}

fn notify_parent_on_worker_exit(core: &Arc<BridgeCore>, child_session_id: &str) {
    verify_read_only_worker(core, child_session_id);
    let Some(label) = unreported_worker_meta(core, child_session_id) else {
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
    report_synthetic_worker_failure(core, child_session_id, &result);
}

/// Stall watchdog action: a worker that has been silent past the timeout.
///
/// Ordering matters for a clean signal:
///   1. Re-check silence under the lock — output may have arrived between the
///      watchdog's snapshot and now, in which case the worker is not stalled.
///   2. Remove the adapter from the live map *without stopping it yet*, so the
///      failure settles as terminal (no retry into a hung process) and the
///      still-running reader thread produces no premature EOF result.
///   3. Report the distinct stall failure (claims `result_status=reported`).
///   4. Only then stop the retained process; its EOF now finds the worker
///      already reported and is a no-op, so the UI shows STALLED, not the
///      generic "ended without reporting".
fn notify_parent_on_worker_stalled(core: &Arc<BridgeCore>, child_session_id: &str) {
    let state = core.clone();
    // (1) Confirm the worker is still silent — closes the snapshot→act race.
    match worker_silence_secs(&state, child_session_id) {
        Some(silent) if silent >= WORKER_STALL_TIMEOUT_SECONDS => {}
        _ => return,
    }
    let Some(label) = unreported_worker_meta(core, child_session_id) else {
        return;
    };
    // (2) Detach the live runtime but keep it alive until the result is claimed.
    let runtime = state.adapters.lock().unwrap().remove(child_session_id);
    let _ = session_supervisor::SessionSupervisor::clear_adapter_process(
        &state.db.lock().unwrap(),
        child_session_id,
    );
    state
        .worker_activity
        .lock()
        .unwrap()
        .remove(child_session_id);
    state
        .worker_activity_persisted
        .lock()
        .unwrap()
        .remove(child_session_id);
    verify_read_only_worker(core, child_session_id);
    let result = delegation::WorkerResult {
        schema_version: delegation::SCHEMA_VERSION,
        status: delegation::WorkerResultStatus::Failed,
        summary: format!(
            "{label} stopped responding (no output for {WORKER_STALL_TIMEOUT_SECONDS}s) and was stopped"
        ),
        files_changed: vec![],
        tests: vec![],
        decisions: vec![],
        risks: vec![
            "Worker went silent past the stall timeout and was stopped mid-task; it may have left \
             uncommitted filesystem changes that are not reflected in files_changed"
                .into(),
        ],
        remaining_work: vec!["Inspect the worktree for partial changes, then retry or re-delegate".into()],
        suggested_next_action: delegation::SuggestedNextAction::Finish,
        suggested_role: None,
        suggested_task: None,
    };
    // (3) Claim the result before the process can die and race us.
    report_synthetic_worker_failure(core, child_session_id, &result);
    // (4) Now stop the hung process; its EOF handler will find it reported.
    if let Some(mut runtime) = runtime {
        runtime.stop(adapters::ShutdownReason::Failed);
    }
}

fn verify_read_only_worker(core: &Arc<BridgeCore>, child_session_id: &str) {
    let state = core.clone();
    let baseline = state
        .delegations
        .lock()
        .unwrap()
        .read_only_baselines
        .remove(child_session_id);
    if let Some(baseline) = baseline {
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
    }
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
            &format!("output_dir={}", sandbox.output_dir().display()),
        );
        sandbox.cleanup();
    }
}

/// Deliver a framed message from a child to its parent session: send it into the
/// parent's live turn stream and drop a marker card into the parent's transcript.
fn report_to_parent(core: &Arc<BridgeCore>, child_session_id: &str, result: &delegation::WorkerResult) {
    let state = core.clone();
    let report = {
        let db = state.db.lock().unwrap();
        session_supervisor::SessionSupervisor::record_result(&db, child_session_id, result)
            .ok()
            .flatten()
    };
    let Some(report) = report else {
        return;
    };
    let core = core.clone();
    let child_session_id = child_session_id.to_owned();
    let result = result.clone();
    thread::spawn(move || {
        let state = core.clone();
        let available_capabilities = live_available_capabilities(&state);
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
                core.events.publish(CoreEvent::Agent(stored));
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
        core.events.publish(CoreEvent::StateChanged);
        if let Some(workspace_id) = workspace_id {
            dispatch_next_queued_worker(&core, &workspace_id);
        }
    });
}

fn dispatch_next_queued_worker(core: &Arc<BridgeCore>, workspace_id: &str) {
    let state = core.clone();
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
        core,
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

fn maintain_worker_pool(core: &Arc<BridgeCore>) {
    let state = core.clone();
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
            if let Err(error) = send_internal_checkpoint_turn(core, &session_id, &prompt) {
                let _ = compaction_controller::CompactionController::record_failure(
                    &state.db.lock().unwrap(),
                    &session_id,
                    &format!("checkpoint turn could not start: {error}"),
                    0,
                );
                finish_worker_checkpoint(core, &session_id, adapters::ShutdownReason::Failed);
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
        finish_worker_checkpoint(core, &session_id, adapters::ShutdownReason::Failed);
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
        finish_orchestrator_shutdown(core, &session_id, adapters::ShutdownReason::UserStopped);
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
        dispatch_next_queued_worker(core, &workspace_id);
    }

    // Stall watchdog. Detection is driven off the in-memory heartbeat map, so
    // the common case (no silent sessions) touches neither the adapter map nor
    // the DB. Only sessions already silent past the timeout are confirmed — via
    // a primary-key lookup on worker_runtime, never a table scan — to be an
    // alive, unreported, actively-`working` worker. A process that has exited is
    // handled by the reader-thread EOF path; `waiting` (awaiting human approval),
    // `warm`, and `checkpointing` are intentionally idle and excluded. Each
    // candidate is re-checked for silence inside the handler before it acts, so
    // output arriving after this snapshot cannot be replaced by a synthetic
    // failure.
    let silent_ids: Vec<String> = {
        let activity = state.worker_activity.lock().unwrap();
        activity
            .iter()
            .filter(|(_, seen)| seen.elapsed().as_secs() >= WORKER_STALL_TIMEOUT_SECONDS)
            .map(|(session_id, _)| session_id.clone())
            .collect()
    };
    if silent_ids.is_empty() {
        return;
    }
    let alive: Vec<String> = {
        let adapters = state.adapters.lock().unwrap();
        silent_ids
            .into_iter()
            .filter(|session_id| adapters.contains_key(session_id))
            .collect()
    };
    let stalled: Vec<String> = {
        let db = state.db.lock().unwrap();
        alive
            .into_iter()
            .filter(|session_id| {
                db.query_row(
                    "SELECT 1 FROM worker_runtime WHERE session_id=?1 AND lifecycle_state='working' AND result_status='pending'",
                    params![session_id],
                    |_| Ok(()),
                )
                .is_ok()
            })
            .collect()
    };
    for session_id in stalled {
        notify_parent_on_worker_stalled(core, &session_id);
    }
}

pub fn start_worker_maintenance(core: Arc<BridgeCore>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        maintain_worker_pool(&core);
    });
}

pub fn start_learning_maintenance(core: Arc<BridgeCore>) {
    thread::spawn(move || loop {
        let ran = {
            let state = core.clone();
            let database_path = state.database_path.clone();
            let result = learning_job::run_due_database(&database_path, Utc::now())
                .ok()
                .flatten();
            result
        };
        if let Some(run) = ran {
            core.events.publish(CoreEvent::LearningJobChanged(serde_json::to_value(run).unwrap_or_default()));
        }
        thread::sleep(Duration::from_secs(60));
    });
}

pub const HISTORY_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub fn start_history_snapshot_maintenance(core: Arc<BridgeCore>) {
    thread::spawn(move || loop {
        thread::sleep(HISTORY_SNAPSHOT_INTERVAL);
        let state = core.clone();
        if let Ok(db) = Connection::open_with_flags(
            &state.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            let _ = store::export_history_snapshot(&db, &state.snapshot_dir);
        }
    });
}

pub fn prepare_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<secret_interception::SanitizedTurn, BridgeError> {
    let state = core;
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

pub fn deliver_sanitized_turn(
    runtime: &dyn adapters::AdapterRuntime,
    text: &str,
    application_context: Option<&str>,
) -> Result<(), BridgeError> {
    match application_context {
        Some(context) => runtime.send_turn_with_context(text, context),
        None => runtime.send_turn(text),
    }
}

pub fn persist_submitted_user_turn(
    db: &Connection,
    session_id: &str,
    adapter_id: &str,
    display_text: &str,
) -> Result<Option<AgentEvent>, BridgeError> {
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

pub fn send_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<(), BridgeError> {
    let state = core;
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
            state.refresh_account_usage()?;
            emit_local_assistant(core,
                &session_id,
                &session_harness,
                "Refreshed account usage. Check the meter in the title bar.",
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Compact { .. } => {
            let prompt = state.begin_manual_compaction(&session_id)?;
            send_internal_checkpoint_turn(core, &session_id, &prompt)?;
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
            emit_local_assistant(core,
                &session_id,
                &session_harness,
                "Cleared this chat’s provider session. Send a message to start fresh.",
            )?;
            core.events.publish(CoreEvent::StateChanged);
            return Ok(());
        }
        slash::SlashDispatch::Unsupported { name, harness } => {
            emit_local_assistant(core,
                &session_id,
                &session_harness,
                &format!("`/{name}` is a {harness} terminal UI command and isn’t available inside Bridge yet."),
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Expand { text } => text,
        slash::SlashDispatch::Forward { text } => text,
    };

    // Read any @file mentions before locking the adapter map so the referenced
    // file contents ride along as trusted application context, not user text.
    let file_context = if let Some(root) = state.session_workspace_root(&session_id) {
        workspace_files::mention_context(&root, &outbound)
    } else {
        None
    };
    let provider_text = workspace_files::append_to_user_text(&outbound, file_context.as_deref());

    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    let credential_context = state.credential_broker.turn_context(&session_id, &outbound);
    if let Err(error) = deliver_sanitized_turn(
        runtime.as_ref(),
        &provider_text,
        credential_context.as_deref(),
    ) {
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
        core.events.publish(CoreEvent::Agent(event));
    }
    let _ = db.execute(
        "UPDATE sessions SET status='working' WHERE id=?1",
        params![session_id],
    );
    core.events.publish(CoreEvent::StateChanged);
    Ok(())
}

fn record_recoverable_adapter_failure(
    state: &Arc<BridgeCore>,
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
    core: &Arc<BridgeCore>,
    session_id: &str,
    adapter_id: &str,
    text: &str,
) -> Result<(), BridgeError> {
    let db = core.db.lock().unwrap();
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
    core.events.publish(CoreEvent::Agent(event));
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

pub fn record_approved_launch_failure(
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

pub fn resolve_policy_delegation_approval(
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

pub fn stop_session(
    core: &Arc<BridgeCore>,
    session_id: String,
) -> Result<BridgeState, BridgeError> {
    let state = core;
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
        if !settle_worker_after_result(&core, &session_id, &result)? {
            return Err(BridgeError::Invalid(
                "cancelled worker cannot be retried".into(),
            ));
        }
        report_to_parent(&core, &session_id, &result);
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::UserCancelled);
        }
        verify_read_only_worker(&core, &session_id);
        let db = state.db.lock().unwrap();
        session_supervisor::SessionSupervisor::clear_adapter_process(&db, &session_id)?;
        let workspace_id: String = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('starting','working','waiting','warm','checkpointing','resuming','restored')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id])?;
        core.events.publish(CoreEvent::StateChanged);
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
        match send_internal_checkpoint_turn(&core, &session_id, &prompt) {
            Ok(()) => {
                let db = state.db.lock().unwrap();
                db.execute(
                    "UPDATE sessions SET status='checkpointing' WHERE id=?1",
                    params![session_id],
                )?;
                core.events.publish(CoreEvent::StateChanged);
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
    core.events.publish(CoreEvent::StateChanged);
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

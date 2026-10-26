//! Host-owned personal defaults and immutable-at-creation session settings.
//! No project files, provider calls, or credentials are read by this module.
use crate::{
    adapters, agent_config,
    delegation::WriteMode,
    model::{AdapterDescriptor, SandboxMode},
    sessions, store, BridgeCore, BridgeError,
};
use bridge_protocol::messages::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::sync::Arc;

const DEFAULTS_KIND: &str = "chat_defaults_v1";
const SESSION_KIND: &str = "chat_session_settings_v1";
const AGENT_KIND: &str = "chat_agent_snapshot_v1";

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}
fn bounded(value: &str, max: usize, name: &str) -> Result<(), BridgeError> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(invalid(format!("Invalid {name}")));
    }
    Ok(())
}
fn project_key(db: &Connection, project_id: Option<&str>) -> Result<String, BridgeError> {
    match project_id {
        None => Ok("application".into()),
        Some(id) => {
            bounded(id, 128, "project id")?;
            if !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(invalid("Project no longer exists"));
            }
            Ok(format!("project:{id}"))
        }
    }
}
fn read<T: serde::de::DeserializeOwned>(
    db: &Connection,
    kind: &str,
    id: &str,
) -> Result<Option<T>, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )
        .optional()?;
    payload
        .map(|text| {
            if text.len() > 128 * 1024 {
                return Err(invalid("Chat settings exceed the storage limit"));
            }
            serde_json::from_str(&text)
                .map_err(|_| invalid("Stored chat settings are invalid; reset them in Settings"))
        })
        .transpose()
}
fn write<T: serde::Serialize>(
    db: &Connection,
    kind: &str,
    id: &str,
    value: &T,
) -> Result<(), BridgeError> {
    let payload = serde_json::to_string(value).map_err(|e| invalid(e.to_string()))?;
    db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,datetime('now'),datetime('now')) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at", params![kind,id,payload])?;
    Ok(())
}
fn record(db: &Connection, key: &str) -> Result<ChatDefaultsRecord, BridgeError> {
    Ok(read(db, DEFAULTS_KIND, key)?.unwrap_or_default())
}
pub fn view(db: &Connection, project_id: Option<&str>) -> Result<ChatDefaultsView, BridgeError> {
    let key = project_key(db, project_id)?;
    Ok(ChatDefaultsView {
        application: record(db, "application")?,
        project: project_id.map(|_| record(db, &key)).transpose()?,
    })
}
fn validate_shape(settings: &ChatDefaults) -> Result<(), BridgeError> {
    if let Some(ChatLaunchTarget::Agent { agent_id }) = &settings.target {
        bounded(agent_id, 128, "agent id")?;
    }
    if let Some(model) = &settings.model {
        bounded(&model.harness, 128, "harness")?;
        if let Some(id) = &model.model {
            bounded(id, 512, "model id")?;
        }
    }
    if let Some(effort) = settings.effort.as_ref().and_then(|e| e.value.as_deref()) {
        if effort.len() > 32
            || effort.is_empty()
            || !effort
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'-')
        {
            return Err(invalid("Unsupported thinking effort"));
        }
    }
    Ok(())
}
fn validate_model(
    choice: &ChatModelChoice,
    effort: Option<&str>,
    descriptors: &[AdapterDescriptor],
    db: &Connection,
) -> Result<(), BridgeError> {
    let descriptor = descriptors
        .iter()
        .find(|d| d.id == choice.harness)
        .ok_or_else(|| invalid("Selected harness is not registered"))?;
    if !descriptor.available || !agent_config::is_harness_enabled(db, &choice.harness) {
        return Err(invalid(format!(
            "{} is unavailable or disabled",
            descriptor.label
        )));
    }
    let model = choice
        .model
        .as_ref()
        .map(|id| {
            descriptor
                .models
                .iter()
                .find(|m| m.id == *id && m.available && m.compatible)
                .ok_or_else(|| {
                    invalid(
                        "Selected model is unavailable or incompatible; choose a model in Settings",
                    )
                })
        })
        .transpose()?;
    if let Some(effort) = effort {
        let model =
            model.ok_or_else(|| invalid("Choose a model before setting thinking effort"))?;
        if !model
            .supported_effort_levels
            .iter()
            .any(|value| value == effort)
        {
            return Err(invalid(
                "Selected model does not support this thinking effort",
            ));
        }
    }
    Ok(())
}
fn agent_by_id(db: &Connection, id: &str) -> Result<agent_config::AgentDefinition, BridgeError> {
    agent_config::state(db)?
        .agents
        .into_iter()
        .find(|a| a.id == id && a.enabled)
        .ok_or_else(|| {
            invalid("Selected agent was removed or disabled; choose an agent in Settings")
        })
}
pub fn save(
    db: &Connection,
    descriptors: &[AdapterDescriptor],
    params: &SaveChatDefaultsParams,
) -> Result<ChatDefaultsView, BridgeError> {
    let key = project_key(db, params.project_id.as_deref())?;
    validate_shape(&params.defaults)?;
    if let Some(ChatLaunchTarget::Agent { agent_id }) = &params.defaults.target {
        agent_by_id(db, agent_id)?;
    }
    if let Some(model) = &params.defaults.model {
        validate_model(
            model,
            params
                .defaults
                .effort
                .as_ref()
                .and_then(|e| e.value.as_deref()),
            descriptors,
            db,
        )?;
    }
    let transaction = db.unchecked_transaction()?;
    let current = record(&transaction, &key)?;
    if current.revision != params.expected_revision {
        return Err(invalid(
            "Defaults changed in another window. Reload and review before saving",
        ));
    }
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Defaults revision exhausted"))?;
    write(
        &transaction,
        DEFAULTS_KIND,
        &key,
        &ChatDefaultsRecord {
            revision,
            defaults: params.defaults.clone(),
        },
    )?;
    let result = view(&transaction, params.project_id.as_deref())?;
    transaction.commit()?;
    Ok(result)
}

/// Layer fields without treating false or an explicit provider default as absent.
fn overlay(
    base: &mut ChatDefaults,
    layer: &ChatDefaults,
    source: &str,
    sources: &mut BTreeMap<String, String>,
) {
    macro_rules! field {
        ($name:ident, $wire:literal) => {
            if let Some(value) = &layer.$name {
                base.$name = Some(value.clone());
                sources.insert($wire.into(), source.into());
            }
        };
    }
    field!(target, "target");
    field!(model, "model");
    field!(effort, "effort");
    field!(access, "access");
    field!(create_worktree, "createWorktree");
    field!(active_turn_input, "activeTurnInput");
}
fn workspace_project(
    db: &Connection,
    workspace_id: Option<&str>,
) -> Result<Option<String>, BridgeError> {
    match workspace_id {
        None => Ok(None),
        Some(id) => {
            bounded(id, 128, "workspace id")?;
            db.query_row("SELECT project_id FROM workspaces WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| invalid("Workspace no longer exists"))
        }
    }
}
pub fn resolve(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
    params: &ResolveChatSettingsParams,
) -> Result<ResolvedChatSettings, BridgeError> {
    validate_shape(&params.overrides)?;
    let project_id = workspace_project(db, params.workspace_id.as_deref())?;
    let stored = view(db, project_id.as_deref())?;
    let revision = ChatDefaultsRevision {
        application: stored.application.revision,
        project: stored.project.as_ref().map_or(0, |p| p.revision),
    };
    if params
        .expected_revision
        .as_ref()
        .is_some_and(|expected| *expected != revision)
    {
        return Err(invalid(
            "New chat defaults changed. Review the refreshed choices before sending",
        ));
    }
    let mut sources = BTreeMap::new();
    let mut combined = ChatDefaults::default();
    overlay(
        &mut combined,
        &stored.application.defaults,
        "application",
        &mut sources,
    );
    if let Some(project) = stored.project {
        overlay(&mut combined, &project.defaults, "project", &mut sources);
    }
    overlay(&mut combined, &params.overrides, "chat", &mut sources);
    let target = combined.target.unwrap_or_else(|| {
        if params.workspace_id.is_some() {
            ChatLaunchTarget::Orchestrator
        } else {
            ChatLaunchTarget::Direct
        }
    });
    if params.workspace_id.is_none() && target != ChatLaunchTarget::Direct {
        return Err(invalid(
            "This default needs a project. Choose a project or Direct chat",
        ));
    }
    let preset = match &target {
        ChatLaunchTarget::Agent { agent_id } => Some(agent_by_id(db, agent_id)?),
        _ => None,
    };
    if preset.as_ref().is_some_and(|a| a.role == "orchestrator") {
        return Err(invalid("Choose Orchestrator for an orchestrator preset"));
    }
    let selection =
        if combined.model.is_none() && preset.as_ref().is_none_or(|a| a.harness == "bridge") {
            Some(sessions::resolve_orchestrator_selection(db, registry)?)
        } else {
            None
        };
    let model = combined.model.unwrap_or_else(|| match &preset {
        Some(a) if a.harness != "bridge" => ChatModelChoice {
            harness: a.harness.clone(),
            model: a.model.clone(),
        },
        _ => selection
            .as_ref()
            .map(|s| ChatModelChoice {
                harness: s.adapter_id.clone(),
                model: s.model.clone(),
            })
            .unwrap_or_else(|| ChatModelChoice {
                harness: "codex".into(),
                model: None,
            }),
    });
    let effort = combined.effort.map(|e| e.value).unwrap_or_else(|| {
        // A different explicit model never inherits an incompatible legacy effort.
        if sources.contains_key("model") {
            None
        } else {
            preset
                .as_ref()
                .map(|a| a.effort.as_str().to_owned())
                .or_else(|| {
                    selection
                        .as_ref()
                        .and_then(|s| s.effort.map(|e| e.as_str().to_owned()))
                })
        }
    });
    let descriptors = registry.descriptors();
    validate_model(&model, effort.as_deref(), &descriptors, db)?;
    if let (Some(_), Some(value)) = (&preset, effort.as_deref()) {
        // A specialist runs as a worker. Its delegation contract and cost model
        // carry low..xhigh, so reject anything else here rather than at create.
        if serde_json::from_value::<crate::delegation::Effort>(serde_json::json!(value)).is_err() {
            return Err(invalid(format!(
                "Specialist agents support low, medium, high or xhigh thinking effort, not {value}. Choose one of those or Provider default"
            )));
        }
    }
    let access = combined.access.unwrap_or(ChatAccess::Ask);
    validate_access(&model.harness, access, &descriptors)?;
    let create_worktree = combined.create_worktree.unwrap_or(false);
    if create_worktree && project_id.is_none() {
        return Err(invalid(
            "Connect a Git project before choosing an isolated worktree",
        ));
    }
    for field in [
        "target",
        "model",
        "effort",
        "access",
        "createWorktree",
        "activeTurnInput",
    ] {
        sources
            .entry(field.into())
            .or_insert_with(|| "default".into());
    }
    if preset.is_some() {
        for field in ["model", "effort"] {
            if sources[field] == "default" {
                sources.insert(field.into(), "agent".into());
            }
        }
    }
    Ok(ResolvedChatSettings {
        workspace_id: params.workspace_id.clone(),
        project_id,
        revision,
        target,
        model,
        effort,
        access,
        create_worktree,
        active_turn_input: combined.active_turn_input.unwrap_or(ActiveTurnInput::Steer),
        sources,
    })
}
fn validate_access(
    harness: &str,
    access: ChatAccess,
    descriptors: &[AdapterDescriptor],
) -> Result<(), BridgeError> {
    let mode = match access {
        ChatAccess::Ask => SandboxMode::WorkspaceWrite,
        ChatAccess::Full => SandboxMode::DangerFullAccess,
    };
    let descriptor = descriptors
        .iter()
        .find(|d| d.id == harness)
        .ok_or_else(|| invalid("Harness is not registered"))?;
    if !descriptor.sandbox_modes.contains(&mode) {
        return Err(invalid(format!(
            "{} cannot enforce this access choice. Choose a supported harness",
            descriptor.label
        )));
    }
    Ok(())
}
pub fn snapshot(
    db: &Connection,
    session_id: &str,
) -> Result<Option<ChatSessionSettings>, BridgeError> {
    read(db, SESSION_KIND, session_id)
}
pub fn session_settings(
    db: &Connection,
    session_id: &str,
) -> Result<ChatSessionSettings, BridgeError> {
    let (harness, model, effort, workspace_id, kind): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ) = db.query_row(
        "SELECT harness,model,effort,workspace_id,kind FROM sessions WHERE id=?1",
        [session_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let mut snapshot = snapshot(db, session_id)?.unwrap_or(ChatSessionSettings {
        revision: 0,
        settings: ResolvedChatSettings {
            workspace_id: workspace_id.clone(),
            project_id: workspace_project(db, workspace_id.as_deref())?,
            revision: ChatDefaultsRevision::default(),
            // A legacy chat has no snapshot; its stored kind is the only record
            // of how it started, and it is what save-as-default exports.
            target: if kind == "orchestrator" {
                ChatLaunchTarget::Orchestrator
            } else {
                ChatLaunchTarget::Direct
            },
            model: ChatModelChoice {
                harness: harness.clone(),
                model: model.clone(),
            },
            effort: effort.clone(),
            access: effective_access(db, session_id)?,
            create_worktree: false,
            active_turn_input: ActiveTurnInput::Steer,
            sources: BTreeMap::new(),
        },
    });
    snapshot.settings.model = ChatModelChoice { harness, model };
    snapshot.settings.effort = effort;
    Ok(snapshot)
}
/// Follow only real parent rows. New root sessions always have an explicit
/// snapshot; older roots keep the legacy global policy until edited by a user.
pub fn scoped_access(db: &Connection, session_id: &str) -> Result<Option<ChatAccess>, BridgeError> {
    Ok(scoped_snapshot(db, session_id)?.map(|stored| stored.settings.access))
}

/// The nearest snapshot on the session's real parent chain: its own for a
/// root, its root's for a worker.
fn scoped_snapshot(
    db: &Connection,
    session_id: &str,
) -> Result<Option<ChatSessionSettings>, BridgeError> {
    let mut current = Some(session_id.to_owned());
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) || seen.len() > 64 {
            return Err(invalid("Invalid session ancestry"));
        }
        let parent: Option<String> = db.query_row(
            "SELECT parent_session_id FROM sessions WHERE id=?1",
            [&id],
            |r| r.get(0),
        )?;
        if let Some(stored) = snapshot(db, &id)? {
            return Ok(Some(stored));
        }
        current = parent;
    }
    Ok(None)
}

pub fn effective_access(db: &Connection, session_id: &str) -> Result<ChatAccess, BridgeError> {
    if let Some(access) = scoped_access(db, session_id)? {
        return Ok(access);
    }
    Ok(
        if agent_config::permission_policy(db)?.auto_approve_provider_permissions {
            ChatAccess::Full
        } else {
            ChatAccess::Ask
        },
    )
}

/// A top-level chat a person reads: no parent and no worker runtime.
pub fn is_top_level_chat(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    Ok(db.query_row(
        "SELECT parent_session_id IS NULL AND NOT EXISTS(SELECT 1 FROM worker_runtime WHERE session_id=?1) FROM sessions WHERE id=?1",
        [session_id],
        |r| r.get(0),
    )?)
}

pub fn root_write_mode(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WriteMode>, BridgeError> {
    Ok(snapshot(db, session_id)?.map(|s| match s.settings.access {
        ChatAccess::Ask => WriteMode::Shared,
        ChatAccess::Full => WriteMode::Full,
    }))
}

pub fn create(
    core: &Arc<BridgeCore>,
    params: &CreateConfiguredChatParams,
) -> Result<ConfiguredChatResult, BridgeError> {
    if params.title.as_ref().is_some_and(|s| s.len() > 512) {
        return Err(invalid("Chat title is too long"));
    }
    let operation = params
        .workspace_id
        .as_deref()
        .map(|id| core.workspace_operation(id));
    let _operation = operation
        .as_ref()
        .map(|o| crate::runtime::lock_operation(o));
    let resolved = resolve(
        &core.db.lock().unwrap(),
        &core.adapter_registry,
        &ResolveChatSettingsParams {
            workspace_id: params.workspace_id.clone(),
            overrides: params.overrides.clone(),
            expected_revision: Some(params.expected_revision.clone()),
        },
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    let worktree = if resolved.create_worktree {
        let db = core.db.lock().unwrap();
        let (title, path): (String, String) = db.query_row(
            "SELECT title,path FROM workspaces WHERE id=?1",
            [params.workspace_id.as_deref().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        drop(db);
        Some((
            path.clone(),
            sessions::prepare_workspace_worktree(
                &core.worktrees,
                &title,
                std::path::Path::new(&path),
                &id,
                WorkspaceSessionKind::Orchestrator,
            )?,
        ))
    } else {
        None
    };
    let result = (|| {
        let db = core.db.lock().unwrap();
        let tx = db.unchecked_transaction()?;
        // Revalidate the same reviewed revision inside the commit, after Git work.
        let settings = resolve(
            &tx,
            &core.adapter_registry,
            &ResolveChatSettingsParams {
                workspace_id: params.workspace_id.clone(),
                overrides: params.overrides.clone(),
                expected_revision: Some(params.expected_revision.clone()),
            },
        )?;
        let cwd = if let Some((_, w)) = &worktree {
            w.path.to_string_lossy().into_owned()
        } else if let Some(w) = &params.workspace_id {
            tx.query_row("SELECT path FROM workspaces WHERE id=?1", [w], |r| {
                r.get::<_, Option<String>>(0)
            })?
            .unwrap_or_else(|| core.chat_scratch_dir(&id).to_string_lossy().into_owned())
        } else {
            core.chat_scratch_dir(&id).to_string_lossy().into_owned()
        };
        let kind = if settings.target == ChatLaunchTarget::Direct {
            "direct"
        } else {
            "orchestrator"
        };
        tx.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,effort,kind,title,cwd,depth) VALUES(?1,?2,?3,?4,'idle','estimated',?5,?6,?7,?8,?9,0)",params![id,params.workspace_id,settings.model.harness,if kind=="direct" {"Chat"} else {"Orchestrator"},settings.model.model,settings.effort,kind,params.title,cwd])?;
        let configuration = ChatSessionSettings {
            revision: 1,
            settings,
        };
        write(&tx, SESSION_KIND, &id, &configuration)?;
        if let ChatLaunchTarget::Agent { agent_id } = &configuration.settings.target {
            let mut agent = agent_by_id(&tx, agent_id)?;
            agent.harness = configuration.settings.model.harness.clone();
            agent.model = configuration.settings.model.model.clone();
            if let Some(effort) = configuration.settings.effort.as_deref() {
                agent.effort = serde_json::from_value(serde_json::json!(effort))
                    .map_err(|_| invalid("Invalid effort"))?;
            }
            write(&tx, AGENT_KIND, &id, &agent)?;
        }
        if let Some((source, w)) = &worktree {
            crate::worktree_registry::register(
                &tx,
                &crate::worktree_registry::NewWorktree {
                    kind: if kind == "direct" {
                        crate::worktree_registry::KIND_DIRECT
                    } else {
                        crate::worktree_registry::KIND_ORCHESTRATOR
                    }
                    .into(),
                    repo_root: source.clone(),
                    path: w.path.to_string_lossy().into_owned(),
                    branch: Some(w.branch.clone()),
                    owner_session_id: Some(id.clone()),
                    owner_workspace_id: params.workspace_id.clone(),
                    base_commit: None,
                },
            )?;
        }
        store::event(
            &tx,
            "chat",
            "chat.created",
            &id,
            "Created chat with reviewed defaults",
        )?;
        let state = crate::api::protocol_wire(store::state(&tx)?)?;
        tx.commit()?;
        Ok(ConfiguredChatResult {
            session_id: id.clone(),
            state,
            configuration,
        })
    })();
    if result.is_err() {
        if let Some((source, w)) = worktree {
            let _ = crate::git::remove_worktree(std::path::Path::new(&source), &w.path);
        }
    }
    if result.is_ok() {
        core.events.publish(crate::events::CoreEvent::StateChanged);
    }
    result
}

pub fn selected_agent(
    db: &Connection,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<agent_config::AgentDefinition>, BridgeError> {
    Ok(
        read::<agent_config::AgentDefinition>(db, AGENT_KIND, session_id)?
            .filter(|a| a.id == agent_id),
    )
}

pub fn update_options(
    core: &Arc<BridgeCore>,
    params: &UpdateChatOptionsParams,
) -> Result<ChatSessionSettings, BridgeError> {
    let (current, family) = {
        let db = core.db.lock().unwrap();
        let root: bool = db.query_row(
            "SELECT parent_session_id IS NULL AND archived_at IS NULL FROM sessions WHERE id=?1",
            [&params.session_id],
            |r| r.get(0),
        )?;
        if !root {
            return Err(invalid("Change access from an active top-level chat"));
        }
        let current = session_settings(&db, &params.session_id)?;
        if current.revision != params.expected_revision {
            return Err(invalid(
                "Chat settings changed. Reload before changing access",
            ));
        }
        validate_access(
            &current.settings.model.harness,
            params.access,
            &core.adapter_registry.descriptors(),
        )?;
        let mut q=db.prepare("WITH RECURSIVE family(id) AS (SELECT id FROM sessions WHERE id=?1 UNION SELECT s.id FROM sessions s JOIN family f ON s.parent_session_id=f.id) SELECT id FROM family ORDER BY id")?;
        let family = q
            .query_map([&params.session_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        (current, family)
    };
    let _claims = family
        .iter()
        .map(|id| core.claim_session_lifecycle(id, "chat options change"))
        .collect::<Result<Vec<_>, _>>()?;
    let access_changed = current.settings.access != params.access || current.revision == 0;
    if access_changed {
        // Block queued dispatch and stale permission replies before retiring any process.
        let db = core.db.lock().unwrap();
        let tx = db.unchecked_transaction()?;
        for id in &family {
            core.deactivate_reader_launch(id);
            tx.execute("UPDATE worker_queue SET queue_status='cancelled',last_error='chat access changed' WHERE parent_session_id=?1 AND queue_status IN ('queued','dispatching','blocked_on_human')",[id])?;
            cancel_permissions(&tx, id)?;
        }
        tx.commit()?;
        drop(db);
        // Root first, as archive does, so a worker's cancellation cannot start
        // a new turn on it under the old posture.
        let root = std::iter::once(&params.session_id);
        for id in root.chain(family.iter().filter(|id| **id != params.session_id)) {
            crate::live_turn::stop_session_for_archive(
                core,
                id,
                crate::live_turn::StopCause::AccessChanged,
            )?;
        }
    }
    let db = core.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let mut next = session_settings(&tx, &params.session_id)?;
    if next.revision != params.expected_revision {
        return Err(invalid("Chat settings changed. Reload before saving"));
    }
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Chat revision exhausted"))?;
    next.settings.access = params.access;
    next.settings.active_turn_input = params.active_turn_input;
    next.settings.sources.insert("access".into(), "chat".into());
    next.settings
        .sources
        .insert("activeTurnInput".into(), "chat".into());
    if access_changed {
        for id in &family {
            tx.execute(
                "UPDATE sessions SET provider_session_id=NULL WHERE id=?1",
                [id],
            )?;
            tx.execute(
                "UPDATE session_heads SET native_provider_session_id=NULL WHERE session_id=?1",
                [id],
            )?;
        }
    }
    write(&tx, SESSION_KIND, &params.session_id, &next)?;
    store::event(
        &tx,
        "user",
        "session.options_changed",
        &params.session_id,
        "Updated this chat's options",
    )?;
    tx.commit()?;
    core.events.publish(crate::events::CoreEvent::StateChanged);
    Ok(next)
}
fn cancel_permissions(db: &rusqlite::Transaction<'_>, id: &str) -> Result<(), BridgeError> {
    let mut q=db.prepare("SELECT e.sequence,e.kind FROM session_entries e WHERE e.session_id=?1 AND e.kind IN ('permission.requested','approval.requested') AND NOT EXISTS(SELECT 1 FROM session_entries r WHERE r.session_id=e.session_id AND r.kind=replace(e.kind,'.requested','.resolved') AND json_extract(r.payload,'$.data.requestEventId')=e.sequence)")?;
    let pending = q
        .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(q);
    for (sequence, kind) in pending {
        let interaction = kind.split('.').next().unwrap();
        db.execute("INSERT INTO interaction_resolutions(session_id,request_sequence,interaction_kind,status,decision,resolved_by,reason,created_at,updated_at) VALUES(?1,?2,?3,'declined','decline','user','chat access changed',datetime('now'),datetime('now')) ON CONFLICT(session_id,request_sequence) DO UPDATE SET status='declined',decision='decline',reason='chat access changed'",params![id,sequence,interaction])?;
        let mut event = crate::agent::NormalizedEvent::new(&format!("{interaction}.resolved"));
        event.status = Some("declined".into());
        event.data = serde_json::json!({"requestEventId":sequence,"decision":"decline","resolvedBy":"user","reason":"chat access changed"});
        store::session_event_in_transaction(db, id, &event, &serde_json::Value::Null)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    struct Stub;
    fn descriptor() -> AdapterDescriptor {
        serde_json::from_value(json!({"id":"codex","label":"Codex","available":true,"authState":"unknown","capabilities":["briefings"],"sandboxModes":["read_only","workspace_write","danger_full_access"],"defaultModel":"standard","models":[{"id":"standard","label":"Standard","tier":"standard","defaultForTier":true,"supportedEffortLevels":["low","medium","high","xhigh"]},{"id":"fast","label":"Fast","tier":"fast","defaultForTier":true,"supportedEffortLevels":[]}]})).unwrap()
    }
    impl adapters::HarnessAdapter for Stub {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            descriptor()
        }
        fn start(
            &self,
            _: adapters::StartRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            panic!("defaults must not start a provider")
        }
        fn resume(
            &self,
            _: adapters::ResumeRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            panic!("defaults must not resume a provider")
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _: &Value) -> Vec<crate::agent::NormalizedEvent> {
            vec![]
        }
    }
    fn fixture() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(dir.path());
        let mut registry = adapters::AdapterRegistry::empty();
        registry.register(Box::new(Stub)).unwrap();
        core.adapter_registry = Arc::new(registry);
        {
            let db = core.db.lock().unwrap();
            for id in ["a", "b"] {
                db.execute(
                    "INSERT INTO projects(id,name,path,created_at) VALUES(?1,?1,?2,'now')",
                    params![id, dir.path().join(id).to_string_lossy()],
                )
                .unwrap();
                db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,?1,'Kyoto',?1,'main',?2,'idle','now')",params![id,dir.path().join(id).to_string_lossy()]).unwrap();
            }
        }
        (dir, Arc::new(core))
    }
    fn defaults(value: Value) -> ChatDefaults {
        serde_json::from_value(value).unwrap()
    }
    fn save_value(
        core: &BridgeCore,
        project: Option<&str>,
        revision: u32,
        value: Value,
    ) -> ChatDefaultsView {
        save(
            &core.db.lock().unwrap(),
            &core.adapter_registry.descriptors(),
            &SaveChatDefaultsParams {
                project_id: project.map(str::to_owned),
                expected_revision: revision,
                defaults: defaults(value),
            },
        )
        .unwrap()
    }
    fn resolve_value(
        core: &BridgeCore,
        workspace: Option<&str>,
        value: Value,
    ) -> ResolvedChatSettings {
        resolve(
            &core.db.lock().unwrap(),
            &core.adapter_registry,
            &ResolveChatSettingsParams {
                workspace_id: workspace.map(str::to_owned),
                overrides: defaults(value),
                expected_revision: None,
            },
        )
        .unwrap()
    }
    fn create_value(core: &Arc<BridgeCore>, workspace: &str, value: Value) -> ConfiguredChatResult {
        let settings = resolve_value(core, Some(workspace), value.clone());
        create(
            core,
            &CreateConfiguredChatParams {
                workspace_id: Some(workspace.into()),
                overrides: defaults(value),
                expected_revision: settings.revision,
                title: None,
            },
        )
        .unwrap()
    }
    #[test]
    fn precedence_and_explicit_false_and_null_are_preserved() {
        let (_dir, core) = fixture();
        save_value(
            &core,
            None,
            0,
            json!({"target":{"kind":"direct"},"model":{"harness":"codex","model":"standard"},"effort":{"value":"high"},"access":"full","createWorktree":true,"activeTurnInput":"queue"}),
        );
        save_value(
            &core,
            Some("a"),
            0,
            json!({"access":"ask","createWorktree":false,"effort":{"value":null}}),
        );
        let a = resolve_value(&core, Some("a"), json!({}));
        assert_eq!(a.access, ChatAccess::Ask);
        assert!(!a.create_worktree);
        assert_eq!(a.effort, None);
        assert_eq!(a.sources["access"], "project");
        let b = resolve_value(&core, Some("b"), json!({}));
        assert_eq!(b.access, ChatAccess::Full);
        assert!(b.create_worktree);
        assert_eq!(b.effort.as_deref(), Some("high"));
        let draft = resolve_value(
            &core,
            Some("a"),
            json!({"access":"full","model":{"harness":"codex","model":null},"effort":{"value":null}}),
        );
        assert_eq!(draft.access, ChatAccess::Full);
        assert_eq!(draft.model.model, None);
        assert_eq!(draft.sources["model"], "chat");
    }
    #[test]
    fn save_conflict_does_not_change_stored_defaults_and_reset_keeps_revision() {
        let (_dir, core) = fixture();
        save_value(&core, Some("a"), 0, json!({"access":"full"}));
        let result = save(
            &core.db.lock().unwrap(),
            &core.adapter_registry.descriptors(),
            &SaveChatDefaultsParams {
                project_id: Some("a".into()),
                expected_revision: 0,
                defaults: defaults(json!({"access":"ask"})),
            },
        );
        assert!(result.is_err());
        assert_eq!(
            resolve_value(&core, Some("a"), json!({})).access,
            ChatAccess::Full
        );
        let reset = save_value(&core, Some("a"), 1, json!({}));
        assert_eq!(reset.project.unwrap().revision, 2);
        assert_eq!(
            resolve_value(&core, Some("a"), json!({})).access,
            ChatAccess::Ask
        );
    }
    #[test]
    fn project_scope_is_validated_and_never_attached_by_name() {
        let (_dir, core) = fixture();
        let db = core.db.lock().unwrap();
        assert!(view(&db, Some("missing")).is_err());
        assert!(view(&db, Some("application")).is_err());
        assert!(resolve(
            &db,
            &core.adapter_registry,
            &ResolveChatSettingsParams {
                workspace_id: Some("missing".into()),
                overrides: ChatDefaults::default(),
                expected_revision: None
            }
        )
        .is_err());
    }
    #[test]
    fn strict_decoding_and_bounded_values_reject_untrusted_payloads() {
        for value in [
            json!({"access":"yolo"}),
            json!({"env":{"TOKEN":"x"}}),
            json!({"target":{"kind":"agent","agentId":"a","command":"sh"}}),
            json!({"effort":{"value":"invalid effort!"}}),
        ] {
            let result = serde_json::from_value::<ChatDefaults>(value);
            assert!(result.as_ref().is_err() || validate_shape(result.as_ref().unwrap()).is_err());
        }
        assert!(validate_shape(&defaults(
            json!({"model":{"harness":"codex","model":"a".repeat(513)}})
        ))
        .is_err());
        assert!(
            validate_shape(&defaults(json!({"target":{"kind":"agent","agentId":"\n"}}))).is_err()
        );
    }
    #[test]
    fn incompatible_model_effort_and_unknown_presets_are_rejected_without_effects() {
        let (_dir, core) = fixture();
        for value in [
            json!({"model":{"harness":"codex","model":"removed"}}),
            json!({"model":{"harness":"codex","model":"fast"},"effort":{"value":"high"}}),
            json!({"target":{"kind":"agent","agentId":"deleted"}}),
        ] {
            assert!(resolve(
                &core.db.lock().unwrap(),
                &core.adapter_registry,
                &ResolveChatSettingsParams {
                    workspace_id: Some("a".into()),
                    overrides: defaults(value),
                    expected_revision: None
                }
            )
            .is_err());
        }
        assert!(core.adapters.lock().unwrap().is_empty());
        assert_eq!(
            core.db
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn unsupported_access_never_broadens_and_unknown_capabilities_fail_closed() {
        let mut descriptor = descriptor();
        descriptor.id = "pi".into();
        descriptor.sandbox_modes = vec![SandboxMode::DangerFullAccess];
        assert!(validate_access("pi", ChatAccess::Ask, &[descriptor.clone()]).is_err());
        assert!(validate_access("pi", ChatAccess::Full, &[descriptor.clone()]).is_ok());
        descriptor.sandbox_modes.clear();
        assert!(validate_access("pi", ChatAccess::Ask, &[descriptor]).is_err());
    }
    #[test]
    fn creation_is_exact_atomic_and_snapshots_do_not_follow_later_defaults() {
        let (_dir, core) = fixture();
        save_value(
            &core,
            Some("a"),
            0,
            json!({"target":{"kind":"direct"},"access":"full"}),
        );
        let first = create_value(&core, "a", json!({}));
        let second = create_value(&core, "a", json!({"access":"ask"}));
        assert_ne!(first.session_id, second.session_id);
        assert!(first
            .state
            .sessions
            .iter()
            .any(|s| s.id == first.session_id));
        save_value(&core, Some("a"), 1, json!({"access":"ask"}));
        let db = core.db.lock().unwrap();
        assert_eq!(
            effective_access(&db, &first.session_id).unwrap(),
            ChatAccess::Full
        );
        assert_eq!(
            effective_access(&db, &second.session_id).unwrap(),
            ChatAccess::Ask
        );
        assert_eq!(
            snapshot(&db, &first.session_id)
                .unwrap()
                .unwrap()
                .settings
                .revision
                .project,
            1
        );
        assert!(core.adapters.lock().unwrap().is_empty());
        assert!(!core.chat_scratch_dir(&first.session_id).exists());
    }
    #[test]
    fn stale_reviewed_revision_creates_no_session() {
        let (_dir, core) = fixture();
        let reviewed = resolve_value(&core, Some("a"), json!({}));
        save_value(&core, Some("a"), 0, json!({"access":"full"}));
        assert!(create(
            &core,
            &CreateConfiguredChatParams {
                workspace_id: Some("a".into()),
                overrides: ChatDefaults::default(),
                expected_revision: reviewed.revision,
                title: None
            }
        )
        .is_err());
        assert_eq!(
            core.db
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn corrupt_records_fail_closed_and_legacy_policy_does_not_leak_to_new_chats() {
        let (_dir, core) = fixture();
        crate::api::save_permission_policy(
            &core,
            agent_config::PermissionPolicy {
                auto_approve_provider_permissions: true,
                ..Default::default()
            },
        )
        .unwrap();
        let chat = create_value(&core, "a", json!({}));
        let db = core.db.lock().unwrap();
        assert_eq!(
            effective_access(&db, &chat.session_id).unwrap(),
            ChatAccess::Ask
        );
        db.execute(
            "UPDATE configuration_entries SET payload='{broken' WHERE kind=?1 AND id=?2",
            params![SESSION_KIND, chat.session_id],
        )
        .unwrap();
        assert!(effective_access(&db, &chat.session_id).is_err());
    }
    #[test]
    fn worker_access_follows_its_actual_parent_and_unknown_ids_fail() {
        let (_dir, core) = fixture();
        let ask = create_value(&core, "a", json!({"access":"ask"}));
        let full = create_value(&core, "b", json!({"access":"full"}));
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,kind,parent_session_id) VALUES('child','a','codex','child','idle','worker',?1)",[&ask.session_id]).unwrap();
        assert_eq!(effective_access(&db, "child").unwrap(), ChatAccess::Ask);
        assert_eq!(
            effective_access(&db, &full.session_id).unwrap(),
            ChatAccess::Full
        );
        assert!(effective_access(&db, "missing").is_err());
    }
    #[test]
    fn options_change_is_scoped_and_invalidates_native_permission_state() {
        let (_dir, core) = fixture();
        let a = create_value(&core, "a", json!({"access":"full"}));
        let b = create_value(&core, "b", json!({"access":"full"}));
        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET provider_session_id='standing-grant-thread' WHERE id=?1",
                [&a.session_id],
            )
            .unwrap();
        let updated = update_options(
            &core,
            &UpdateChatOptionsParams {
                session_id: a.session_id.clone(),
                expected_revision: 1,
                access: ChatAccess::Ask,
                active_turn_input: ActiveTurnInput::Queue,
            },
        )
        .unwrap();
        assert_eq!(updated.revision, 2);
        let db = core.db.lock().unwrap();
        assert_eq!(
            effective_access(&db, &a.session_id).unwrap(),
            ChatAccess::Ask
        );
        assert_eq!(
            effective_access(&db, &b.session_id).unwrap(),
            ChatAccess::Full
        );
        assert_eq!(
            db.query_row(
                "SELECT provider_session_id FROM sessions WHERE id=?1",
                [&a.session_id],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
    }
    #[test]
    fn persisted_defaults_survive_database_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.db");
        let db = store::open(&path).unwrap();
        save(
            &db,
            &[descriptor()],
            &SaveChatDefaultsParams {
                project_id: None,
                expected_revision: 0,
                defaults: defaults(json!({"access":"full"})),
            },
        )
        .unwrap();
        drop(db);
        let reopened = store::open(&path).unwrap();
        assert_eq!(
            view(&reopened, None).unwrap().application.defaults.access,
            Some(ChatAccess::Full)
        );
    }
    #[test]
    fn resetting_preferences_keeps_existing_session_authority() {
        let (_dir, core) = fixture();
        let chat = create_value(&core, "a", json!({"access":"ask"}));
        agent_config::reset_all(&core.db.lock().unwrap()).unwrap();
        let db = core.db.lock().unwrap();
        assert!(snapshot(&db, &chat.session_id).unwrap().is_some());
        assert!(matches!(
            root_write_mode(&db, &chat.session_id).unwrap(),
            Some(WriteMode::Shared)
        ));
    }
    #[test]
    fn failed_session_snapshot_rolls_back_the_insert() {
        let (_dir, core) = fixture();
        let resolved = resolve_value(&core, Some("a"), json!({}));
        core.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_snapshot BEFORE INSERT ON configuration_entries WHEN NEW.kind='chat_session_settings_v1' BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        assert!(create(
            &core,
            &CreateConfiguredChatParams {
                workspace_id: Some("a".into()),
                overrides: ChatDefaults::default(),
                expected_revision: resolved.revision,
                title: None
            }
        )
        .is_err());
        assert_eq!(
            core.db
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    struct LaunchProbe(
        std::sync::Arc<std::sync::Mutex<Vec<(Option<WriteMode>, Option<String>, Option<String>)>>>,
    );
    impl adapters::HarnessAdapter for LaunchProbe {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            descriptor()
        }
        fn start(
            &self,
            r: adapters::StartRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            assert!(
                r.interactive
                    && crate::mcp_apps::attaches_to(
                        r.interactive,
                        r.write_mode,
                        r.read_only_sandbox,
                        r.briefing
                    ),
                "a configured interactive chat keeps the visualize tool"
            );
            self.0.lock().unwrap().push((
                r.write_mode,
                r.model.map(str::to_owned),
                r.effort.map(str::to_owned),
            ));
            Err(invalid("probe: launch boundary reached"))
        }
        fn resume(
            &self,
            r: adapters::ResumeRequest<'_>,
        ) -> Result<adapters::StartedAdapter, BridgeError> {
            self.0.lock().unwrap().push((
                r.write_mode,
                r.model.map(str::to_owned),
                r.effort.map(str::to_owned),
            ));
            Err(invalid("probe: resume boundary reached"))
        }
        fn supports_native_resume(&self) -> bool {
            true
        }
        fn normalize(&self, _: &Value) -> Vec<crate::agent::NormalizedEvent> {
            vec![]
        }
    }
    #[test]
    fn configured_root_launch_passes_concrete_access_and_provider_defaults() {
        for (access, expected) in [("ask", WriteMode::Shared), ("full", WriteMode::Full)] {
            let (_dir, mut core) = fixture();
            let calls = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
            let mut registry = adapters::AdapterRegistry::empty();
            registry
                .register(Box::new(LaunchProbe(calls.clone())))
                .unwrap();
            Arc::get_mut(&mut core).unwrap().adapter_registry = Arc::new(registry);
            let chat = create_value(
                &core,
                "a",
                json!({"target":{"kind":"direct"},"model":{"harness":"codex","model":null},"effort":{"value":null},"access":access}),
            );
            let error = crate::live_turn::start_chat(&core, chat.session_id).unwrap_err();
            assert!(error.to_string().contains("probe:"), "{error}");
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0], (Some(expected), None, None));
        }
    }
    #[test]
    fn inherited_aside_and_fork_authority_does_not_follow_global_full_access() {
        let (_dir, core) = fixture();
        let source = create_value(&core, "a", json!({"access":"ask"}));
        let db = core.db.lock().unwrap();
        agent_config::save_permission_policy(
            &db,
            agent_config::PermissionPolicy {
                auto_approve_provider_permissions: true,
                ..Default::default()
            },
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,kind) VALUES('fork','codex','fork','idle','direct')",[]).unwrap();
        inherit_session_settings(&db, &source.session_id, "fork").unwrap();
        assert_eq!(effective_access(&db, "fork").unwrap(), ChatAccess::Ask);
        assert!(matches!(
            root_write_mode(&db, "fork").unwrap(),
            Some(WriteMode::Shared)
        ));
    }
    #[test]
    fn changing_access_declines_old_permissions_and_stale_auto_grants() {
        let (_dir, core) = fixture();
        let chat = create_value(&core, "a", json!({"access":"full"}));
        let sequence = {
            let db = core.db.lock().unwrap();
            let mut event = crate::agent::NormalizedEvent::new("permission.requested");
            event.data = json!({"requestId":1});
            store::session_event(&db, &chat.session_id, &event, &Value::Null)
                .unwrap()
                .sequence
        };
        update_options(
            &core,
            &UpdateChatOptionsParams {
                session_id: chat.session_id.clone(),
                expected_revision: 1,
                access: ChatAccess::Ask,
                active_turn_input: ActiveTurnInput::Steer,
            },
        )
        .unwrap();
        assert!(
            crate::api::auto_resolve_provider_permission(&core, &chat.session_id, sequence)
                .is_err()
        );
        let db = core.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT decision FROM interaction_resolutions WHERE session_id=?1 AND request_sequence=?2",params![chat.session_id,sequence],|r|r.get::<_,String>(0)).unwrap(),"decline");
    }
    #[test]
    fn removed_models_and_unenforceable_switches_fail_before_provider_start() {
        let (_dir, core) = fixture();
        let chat = create_value(&core, "a", json!({}));
        let db = core.db.lock().unwrap();
        assert!(validate_launch(
            &db,
            &core.adapter_registry,
            &chat.session_id,
            "codex",
            Some("removed"),
            None
        )
        .is_err());
        assert!(validate_model_switch(
            &db,
            &core.adapter_registry,
            &chat.session_id,
            "unregistered"
        )
        .is_err());
    }
    #[test]
    fn a_scoped_approval_chat_cannot_launch_an_unrestricted_worker() {
        use crate::delegation::*;
        let (_dir, core) = fixture();
        let chat = create_value(&core, "a", json!({"access":"ask"}));
        let request = DelegationRequest {
            schema_version: SCHEMA_VERSION,
            role: WorkerRole::Implementation,
            objective: "change files".into(),
            acceptance_criteria: vec!["tests".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec![],
            owned_paths: vec!["**".into()],
            write_mode: WriteMode::Full,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec![],
            output_contract: OutputContract::ImplementationResult,
            harness: Some("codex".into()),
            model: None,
        };
        let error = crate::policy_coordinator::PolicyCoordinator::decide_worker_route(
            &core.db.lock().unwrap(),
            &chat.session_id,
            "turn",
            &request,
            true,
        )
        .err()
        .expect("unrestricted worker must be refused");
        assert!(error.to_string().contains("requires user approval"));
    }
    fn real_worker(db: &Connection, id: &str, parent: &str, lifecycle: &str) {
        // Shaped like `reserve_worker_launch_outcome`: default `kind`, a
        // runtime row and an active lease.
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES(?1,'a','codex','Worker','working','reported',?2,1)",params![id,parent]).unwrap();
        store::upsert_worker_runtime(
            db,
            &crate::model::WorkerRuntimeRecord {
                session_id: id.into(),
                parent_session_id: parent.into(),
                lifecycle_state: lifecycle.into(),
                task_family: "implementation".into(),
                compatibility_key: "key".into(),
                result_status: "pending".into(),
                retry_count: 0,
                warm_until: None,
                worktree_path: None,
                worktree_branch: None,
                last_result: None,
                last_activity_at: None,
                waiting_since: None,
                waiting_reason: None,
                progress_summary: None,
                updated_at: "now".into(),
                failure_class: None,
            },
        )
        .unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,write_mode,lease_status,created_at,updated_at) VALUES(?1,'a','implementation','standard','implementation','shared','active','now','now')",[id]).unwrap();
    }
    #[test]
    fn only_top_level_chats_are_interactive_launches() {
        let (_dir, core) = fixture();
        let root = create_value(&core, "a", json!({"access":"ask"}));
        let db = core.db.lock().unwrap();
        real_worker(&db, "worker", &root.session_id, "working");
        assert!(is_top_level_chat(&db, &root.session_id).unwrap());
        assert!(!is_top_level_chat(&db, "worker").unwrap());
        // The worker carries an explicit write mode and is not interactive.
        assert!(!crate::mcp_apps::attaches_to(
            false,
            Some(WriteMode::Shared),
            None,
            None
        ));
        assert!(is_top_level_chat(&db, "missing").is_err());
    }
    #[test]
    fn an_aside_from_a_worker_keeps_the_scoped_restriction_of_its_root() {
        let (_dir, core) = fixture();
        let root = create_value(&core, "a", json!({"access":"ask"}));
        {
            let db = core.db.lock().unwrap();
            agent_config::save_permission_policy(
                &db,
                agent_config::PermissionPolicy {
                    auto_approve_provider_permissions: true,
                    ..Default::default()
                },
            )
            .unwrap();
            real_worker(&db, "worker", &root.session_id, "working");
            assert_eq!(effective_access(&db, "worker").unwrap(), ChatAccess::Ask);
        }
        let (aside, _, _) = core
            .create_aside_chat_id("worker", &crate::model::Harness::Codex, None, None)
            .unwrap();
        let db = core.db.lock().unwrap();
        assert_eq!(effective_access(&db, &aside).unwrap(), ChatAccess::Ask);
        assert!(matches!(
            root_write_mode(&db, &aside).unwrap(),
            Some(WriteMode::Shared)
        ));
    }
    #[test]
    fn an_access_change_settles_real_workers_and_releases_their_leases() {
        let (_dir, core) = fixture();
        let root = create_value(&core, "a", json!({"access":"full"}));
        real_worker(
            &core.db.lock().unwrap(),
            "worker",
            &root.session_id,
            "working",
        );
        update_options(
            &core,
            &UpdateChatOptionsParams {
                session_id: root.session_id.clone(),
                expected_revision: 1,
                access: ChatAccess::Ask,
                active_turn_input: ActiveTurnInput::Steer,
            },
        )
        .unwrap();
        let db = core.db.lock().unwrap();
        let lifecycle: String = db
            .query_row(
                "SELECT lifecycle_state FROM worker_runtime WHERE session_id='worker'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let lease: String = db
            .query_row(
                "SELECT lease_status FROM worker_leases WHERE session_id='worker'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            (lifecycle.as_str(), lease.as_str()),
            ("cancelled", "released")
        );
    }
    #[test]
    fn specialist_efforts_offered_are_the_ones_a_worker_can_start() {
        let (_dir, mut core) = fixture();
        let mut wide = descriptor();
        wide.models[0].supported_effort_levels = vec!["high".into(), "max".into(), "ultra".into()];
        struct Wide(AdapterDescriptor);
        impl adapters::HarnessAdapter for Wide {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn descriptor(&self) -> AdapterDescriptor {
                self.0.clone()
            }
            fn start(
                &self,
                _: adapters::StartRequest<'_>,
            ) -> Result<adapters::StartedAdapter, BridgeError> {
                panic!("no start")
            }
            fn resume(
                &self,
                _: adapters::ResumeRequest<'_>,
            ) -> Result<adapters::StartedAdapter, BridgeError> {
                panic!("no resume")
            }
            fn supports_native_resume(&self) -> bool {
                false
            }
            fn normalize(&self, _: &Value) -> Vec<crate::agent::NormalizedEvent> {
                vec![]
            }
        }
        let mut registry = adapters::AdapterRegistry::empty();
        registry.register(Box::new(Wide(wide))).unwrap();
        Arc::get_mut(&mut core).unwrap().adapter_registry = Arc::new(registry);
        let agent = {
            let db = core.db.lock().unwrap();
            agent_config::state(&db)
                .unwrap()
                .agents
                .into_iter()
                .find(|a| a.enabled && a.role == "research")
                .unwrap()
                .id
        };
        for (effort, starts) in [("high", true), ("max", false), ("ultra", false)] {
            let value = json!({"target":{"kind":"agent","agentId":agent},"model":{"harness":"codex","model":"standard"},"effort":{"value":effort}});
            let resolved = resolve(
                &core.db.lock().unwrap(),
                &core.adapter_registry,
                &ResolveChatSettingsParams {
                    workspace_id: Some("a".into()),
                    overrides: defaults(value.clone()),
                    expected_revision: None,
                },
            );
            // Whatever resolve accepts must also create; nothing fails later.
            assert_eq!(resolved.is_ok(), starts, "{effort}: {resolved:?}");
            if let Ok(settings) = resolved {
                assert!(
                    create(
                        &core,
                        &CreateConfiguredChatParams {
                            workspace_id: Some("a".into()),
                            overrides: defaults(value),
                            expected_revision: settings.revision,
                            title: None
                        }
                    )
                    .is_ok(),
                    "{effort}"
                );
            }
        }
    }
    #[test]
    fn a_legacy_orchestrator_reports_and_exports_its_orchestrator_target() {
        let (_dir, core) = fixture();
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('legacy-o','a','codex','Orchestrator','idle','orchestrator')",[]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('legacy-d','a','codex','Chat','idle','direct')",[]).unwrap();
        assert_eq!(
            session_settings(&db, "legacy-o").unwrap().settings.target,
            ChatLaunchTarget::Orchestrator
        );
        assert_eq!(
            session_settings(&db, "legacy-d").unwrap().settings.target,
            ChatLaunchTarget::Direct
        );
    }
    #[test]
    fn seeded_fuzz_scope_ids_and_model_effort_access_combinations() {
        let (_dir, core) = fixture();
        let db = core.db.lock().unwrap();
        let mut seed = 0x840_5c0e_e0f0u64;
        let (mut accepted, mut rejected) = (0, 0);
        for _ in 0..5_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Arbitrary scope ids, including path-like and name-like ones, never bind.
            let id = (0..1 + seed % 24)
                .map(|i| char::from(b' ' + ((seed >> ((i * 5) % 60)) % 95) as u8))
                .collect::<String>();
            let known = id == "a" || id == "b";
            assert_eq!(
                view(&db, Some(&id)).is_ok(),
                known,
                "seed {seed} scope {id:?}"
            );
            // The catalog is the oracle: standard advertises low..xhigh, fast none.
            let model = ["standard", "fast", "removed"][(seed >> 8) as usize % 3];
            let effort = [
                None,
                Some(None),
                Some(Some("low")),
                Some(Some("xhigh")),
                Some(Some("max")),
            ][(seed >> 12) as usize % 5];
            let access = [None, Some("ask"), Some("full")][(seed >> 16) as usize % 3];
            let target = [
                json!({"kind":"direct"}),
                json!({"kind":"orchestrator"}),
                json!({"kind":"agent","agentId":format!("missing-{:x}", seed >> 20)}),
            ][(seed >> 20) as usize % 3]
                .clone();
            let mut value = json!({"model":{"harness":"codex","model":model},"target":target});
            if let Some(effort) = effort {
                value["effort"] = json!({"value": effort});
            }
            if let Some(access) = access {
                value["access"] = json!(access);
            }
            let expected = model != "removed"
                && !matches!(effort, Some(Some("max")))
                && !(model == "fast" && matches!(effort, Some(Some(_))))
                && target["kind"] != "agent";
            let workspace = if seed & (1 << 40) != 0 {
                id.clone()
            } else {
                "a".into()
            };
            let result = resolve(
                &db,
                &core.adapter_registry,
                &ResolveChatSettingsParams {
                    workspace_id: Some(workspace.clone()),
                    overrides: defaults(value.clone()),
                    expected_revision: None,
                },
            );
            let scoped = workspace == "a" || workspace == "b";
            assert_eq!(
                result.is_ok(),
                expected && scoped,
                "seed {seed} {value} in {workspace:?}: {result:?}"
            );
            if result.is_ok() {
                accepted += 1
            } else {
                rejected += 1
            }
            if let Ok(settings) = result {
                assert_eq!(
                    settings.access,
                    if access == Some("full") {
                        ChatAccess::Full
                    } else {
                        ChatAccess::Ask
                    },
                    "seed {seed}"
                );
                assert_eq!(settings.model.model.as_deref(), Some(model), "seed {seed}");
            }
        }
        // Both outcomes are exercised, so the oracle is not vacuous.
        assert!(
            accepted > 500 && rejected > 500,
            "accepted {accepted}, rejected {rejected}"
        );
        assert!(core.adapters.lock().unwrap().is_empty());
        assert_eq!(
            db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn seeded_fuzz_layering_and_strict_payloads() {
        // xorshift64, fixed seed: replayable without OS randomness or extra dependencies.
        let mut seed = 0x840_20261026u64;
        for _ in 0..10_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut out = ChatDefaults::default();
            let mut sources = BTreeMap::new();
            let mut expected = None;
            for (bit, source) in [(0, "application"), (2, "project"), (4, "chat")] {
                let access = if seed & (1 << bit) != 0 {
                    Some(if seed & (1 << (bit + 1)) != 0 {
                        ChatAccess::Full
                    } else {
                        ChatAccess::Ask
                    })
                } else {
                    None
                };
                if let Some(value) = access {
                    expected = Some(value);
                }
                overlay(
                    &mut out,
                    &ChatDefaults {
                        access,
                        ..Default::default()
                    },
                    source,
                    &mut sources,
                );
            }
            assert_eq!(out.access, expected, "seed {seed}");
            let payload = json!({format!("untrusted_{seed:x}"):seed,"access":"full"});
            assert!(serde_json::from_value::<ChatDefaults>(payload).is_err());
            let bytes = seed
                .to_le_bytes()
                .iter()
                .map(|n| char::from(b' ' + n % 95))
                .collect::<String>();
            let _ = serde_json::from_str::<ChatDefaults>(&bytes);
        }
    }
}

/// Provider flags are checked on every launch/resume, including after a model
/// change. An explicit provider default must not become an unrelated tier pin.
pub fn validate_launch(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
    session_id: &str,
    harness: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<bool, BridgeError> {
    let Some(settings) = snapshot(db, session_id)? else {
        return Ok(false);
    };
    let descriptors = registry.descriptors();
    validate_model(
        &ChatModelChoice {
            harness: harness.into(),
            model: model.map(str::to_owned),
        },
        effort,
        &descriptors,
        db,
    )?;
    validate_access(harness, settings.settings.access, &descriptors)?;
    Ok(true)
}
/// Forks/asides are independent roots, so authority must travel explicitly in
/// the same insertion transaction rather than falling back to global policy.
pub fn inherit_session_settings(
    db: &Connection,
    source: &str,
    target: &str,
) -> Result<(), BridgeError> {
    // A worker has no snapshot of its own; it holds its root's authority. The
    // detached copy must carry that authority, not fall back to the legacy
    // global policy.
    if let Some(mut stored) = scoped_snapshot(db, source)? {
        let (harness, model, effort, workspace): (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = db.query_row(
            "SELECT harness,model,effort,workspace_id FROM sessions WHERE id=?1",
            [target],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        stored.revision = 1;
        stored.settings.target = ChatLaunchTarget::Direct;
        stored.settings.workspace_id = workspace.clone();
        stored.settings.project_id = workspace_project(db, workspace.as_deref())?;
        stored.settings.model = ChatModelChoice { harness, model };
        stored.settings.effort = effort;
        stored
            .settings
            .sources
            .insert("access".into(), "source chat".into());
        write(db, SESSION_KIND, target, &stored)?;
    }
    Ok(())
}

pub fn validate_model_switch(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
    session_id: &str,
    harness: &str,
) -> Result<(), BridgeError> {
    if let Some(settings) = snapshot(db, session_id)? {
        validate_access(harness, settings.settings.access, &registry.descriptors())?;
    }
    Ok(())
}

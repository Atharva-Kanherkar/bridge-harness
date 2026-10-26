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
    let (harness, model, effort, workspace_id): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = db.query_row(
        "SELECT harness,model,effort,workspace_id FROM sessions WHERE id=?1",
        [session_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let mut snapshot = snapshot(db, session_id)?.unwrap_or(ChatSessionSettings {
        revision: 0,
        settings: ResolvedChatSettings {
            workspace_id: workspace_id.clone(),
            project_id: workspace_project(db, workspace_id.as_deref())?,
            revision: ChatDefaultsRevision::default(),
            target: ChatLaunchTarget::Direct,
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
pub fn effective_access(db: &Connection, session_id: &str) -> Result<ChatAccess, BridgeError> {
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
            return Ok(stored.settings.access);
        }
        current = parent;
    }
    Ok(
        if agent_config::permission_policy(db)?.auto_approve_provider_permissions {
            ChatAccess::Full
        } else {
            ChatAccess::Ask
        },
    )
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
        for id in &family {
            crate::live_turn::stop_session_for_archive(core, id)?;
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

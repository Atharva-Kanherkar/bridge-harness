//! The read side of the tools of a chat. Each source is free: the connector
//! catalog that Bridge cached, the last Claude init, the latest context
//! reading and the warm runtime of the chat. Nothing here starts a process.
use super::{
    claude_units, codex_units, invalid, opencode_not_applied_reason, opencode_units, read, save, status,
    stored_revision, ClaudeSource, CodexSource, OpenCodeSource, SaveOutcome, TokenFigures, ToolSelectionRow,
};
use crate::{chat_defaults, provider_accounts, runtime::BridgeCore, BridgeError};
use bridge_protocol::messages::{
    ChatToolItem, ChatToolSelection, ChatToolSelectionStatus, ChatToolSource, ChatToolSourceKind, ChatToolsDraft,
    GetChatToolsParams, GetChatToolsResult, UpdateChatToolsParams, UpdateChatToolsResult,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

/// The note of a source that only a warm runtime answers.
pub const COLD_NOTE: &str = "Available after the first turn";
/// The label of the Claude token figures. The sidecar reads them with
/// `getContextUsage`.
pub const CLAUDE_FIGURE_SOURCE: &str = "claude.getContextUsage";
const NO_CATALOG_NOTE: &str = "Bridge has not read the connector catalog yet. Open Connectors to read it";
const NO_UNITS_REASON: &str = "This harness has no tool choice";
const DONOR_NOTE: &str = "The newest chat of this account that started. A new chat can list other items";
/// How many recent Claude chats a draft looks at for an init to borrow.
const DONOR_CANDIDATES: i64 = 50;

/// The facts of a chat that the list needs.
struct Chat {
    harness: String,
    account: Option<String>,
    workspace_id: Option<String>,
}

/// The saved choice as the list sees it.
enum Saved {
    None,
    Row(ToolSelectionRow),
    /// A row that fails its checks. A save at its revision replaces it.
    Unreadable { revision: u32, reason: String },
}

impl Saved {
    fn row(&self) -> Option<&ToolSelectionRow> {
        match self {
            Self::Row(row) => Some(row),
            Self::None | Self::Unreadable { .. } => None,
        }
    }
}

/// The last Claude init that the list uses. `from` names the chat that
/// gave it, when that chat is not this chat.
struct InitRead {
    data: Value,
    at: String,
    from: Option<String>,
}

#[derive(Default)]
struct ClaudeReads {
    init: Option<InitRead>,
    figures: Option<TokenFigures>,
    failure: Option<String>,
}

type Listed = (Vec<ChatToolItem>, Vec<ChatToolSource>);

/// The items of a chat or a draft, its saved choice and the sources read.
pub fn get(core: &Arc<BridgeCore>, params: &GetChatToolsParams) -> Result<GetChatToolsResult, BridgeError> {
    match (params.session_id.as_deref(), params.draft.as_ref()) {
        (Some(session_id), None) => for_session(core, session_id),
        (None, Some(draft)) => for_draft(core, draft),
        _ => Err(invalid("Give exactly one of sessionId and draft")),
    }
}

/// Save a new choice through the shared busy guard of a send.
pub fn update(core: &BridgeCore, params: &UpdateChatToolsParams) -> Result<UpdateChatToolsResult, BridgeError> {
    Ok(match save(core, &params.session_id, params.expected_revision, &params.selection)? {
        SaveOutcome::Saved(row) => UpdateChatToolsResult { revision: Some(row.revision), refusal: None },
        SaveOutcome::Busy(reason) => UpdateChatToolsResult { revision: None, refusal: Some(reason.to_owned()) },
    })
}

fn for_session(core: &Arc<BridgeCore>, session_id: &str) -> Result<GetChatToolsResult, BridgeError> {
    let (chat, saved, claude) = {
        let db = core.try_db()?;
        let chat = chat_facts(&db, session_id)?.ok_or_else(|| invalid("This chat does not exist"))?;
        if !chat_defaults::is_top_level_chat(&db, session_id)? {
            return Err(invalid("Only a top-level chat can have a tool choice"));
        }
        let saved = saved_choice(&db, session_id)?;
        let claude = if chat.harness == "claude" { claude_reads(&db, Some(session_id), &chat) } else { ClaudeReads::default() };
        (chat, saved, claude)
    };
    let (items, sources) = match chat.harness.as_str() {
        "claude" => claude_list(claude, saved.row()),
        "codex" => codex_list(core, Some(session_id), saved.row()),
        "opencode" => opencode_list(core, Some(session_id), saved.row()),
        _ => (Vec::new(), Vec::new()),
    };
    Ok(result(chat, saved, items, sources))
}

/// A draft has no row and no runtime. Claude borrows the last init of a
/// recent chat of the same account. Codex and OpenCode wait for a turn.
fn for_draft(core: &Arc<BridgeCore>, draft: &ChatToolsDraft) -> Result<GetChatToolsResult, BridgeError> {
    let chat = Chat {
        account: provider_accounts::normalize_choice(&draft.harness, draft.account.as_deref()),
        harness: draft.harness.clone(),
        workspace_id: draft.workspace_id.clone(),
    };
    let (items, sources) = match chat.harness.as_str() {
        "claude" => {
            let reads = claude_reads(&*core.try_db()?, None, &chat);
            claude_list(reads, None)
        }
        "codex" => codex_list(core, None, None),
        "opencode" => opencode_list(core, None, None),
        _ => (Vec::new(), Vec::new()),
    };
    Ok(result(chat, Saved::None, items, sources))
}

fn result(chat: Chat, saved: Saved, items: Vec<ChatToolItem>, sources: Vec<ChatToolSource>) -> GetChatToolsResult {
    let (choice, selection) = match saved {
        Saved::Row(row) => (Some(row.choice()), status(&row, &chat.harness, chat.account.as_deref())),
        Saved::None => (None, selection_status(&chat, 0, None)),
        Saved::Unreadable { revision, reason } => (None, selection_status(&chat, revision, Some(reason))),
    };
    let selection = match chat.harness.as_str() {
        "claude" | "codex" => selection,
        "opencode" => opencode_guard(selection, choice.as_ref()),
        _ => not_applied(selection, NO_UNITS_REASON),
    };
    GetChatToolsResult { harness: chat.harness, account: chat.account, items, choice, selection, sources }
}

fn selection_status(chat: &Chat, revision: u32, reason: Option<String>) -> ChatToolSelectionStatus {
    ChatToolSelectionStatus {
        revision,
        harness: chat.harness.clone(),
        account: chat.account.clone(),
        applies: reason.is_none(),
        not_applied_reason: reason,
    }
}

fn not_applied(selection: ChatToolSelectionStatus, reason: &str) -> ChatToolSelectionStatus {
    if !selection.applies {
        return selection;
    }
    ChatToolSelectionStatus { applies: false, not_applied_reason: Some(reason.to_owned()), ..selection }
}

/// An inherited `OPENCODE_CONFIG_CONTENT` that does not parse stops each
/// OpenCode choice. The picker says so before the person makes one.
fn opencode_guard(selection: ChatToolSelectionStatus, choice: Option<&ChatToolSelection>) -> ChatToolSelectionStatus {
    let off = choice.map_or(&[][..], |choice| choice.servers_off.as_slice());
    match opencode_not_applied_reason(std::env::var("OPENCODE_CONFIG_CONTENT").ok().as_deref(), off) {
        Some(reason) => not_applied(selection, reason),
        None => selection,
    }
}

fn chat_facts(db: &Connection, session_id: &str) -> Result<Option<Chat>, BridgeError> {
    let row: Option<(String, Option<String>, Option<String>)> = db
        .query_row("SELECT harness,account,workspace_id FROM sessions WHERE id=?1", [session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()?;
    Ok(row.map(|(harness, account, workspace_id)| Chat {
        account: provider_accounts::normalize_choice(&harness, account.as_deref()),
        harness,
        workspace_id,
    }))
}

fn saved_choice(db: &Connection, session_id: &str) -> Result<Saved, BridgeError> {
    match read(db, session_id) {
        Ok(Some(row)) => Ok(Saved::Row(row)),
        Ok(None) => Ok(Saved::None),
        Err(BridgeError::Invalid(reason)) => Ok(Saved::Unreadable { revision: stored_revision(db, session_id)?, reason }),
        Err(other) => Err(other),
    }
}

// --- Claude -----------------------------------------------------------------------

fn last_init(db: &Connection, session_id: &str) -> Result<Option<(Value, String)>, BridgeError> {
    let row: Option<(Option<String>, String)> = db
        .query_row(
            "SELECT json_extract(payload,'$.data'),created_at FROM session_entries WHERE session_id=?1 AND kind='session.started' AND json_extract(payload,'$.data.subtype')='init' ORDER BY sequence DESC LIMIT 1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(data, at)| Some((serde_json::from_str(&data?).ok()?, at))))
}

/// The newest top-level Claude chat of the same account that has an init.
/// A chat of the same workspace comes first.
fn donor(db: &Connection, chat: &Chat, skip: Option<&str>) -> Result<Option<(String, InitRead)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id,COALESCE(NULLIF(title,''),label),account FROM sessions s WHERE harness='claude' AND parent_session_id IS NULL AND id IS NOT ?1 AND NOT EXISTS(SELECT 1 FROM worker_runtime w WHERE w.session_id=s.id) ORDER BY (workspace_id IS ?2) DESC, rowid DESC LIMIT ?3",
    )?;
    let candidates = statement
        .query_map(params![skip, chat.workspace_id, DONOR_CANDIDATES], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, title, account) in candidates {
        if provider_accounts::normalize_choice("claude", account.as_deref()) != chat.account {
            continue;
        }
        if let Some((data, at)) = last_init(db, &id)? {
            return Ok(Some((id, InitRead { data, at, from: Some(title) })));
        }
    }
    Ok(None)
}

/// The per-item figures of the latest Claude context reading of a chat.
fn latest_figures(db: &Connection, session_id: &str) -> Result<Option<TokenFigures>, BridgeError> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT breakdown_json,created_at FROM context_readings WHERE session_id=?1 AND source='claude.context_usage' AND breakdown_json IS NOT NULL ORDER BY id DESC LIMIT 1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((breakdown, at)) = row else { return Ok(None) };
    let Ok(breakdown) = serde_json::from_str::<Value>(&breakdown) else { return Ok(None) };
    let named = |rows: Option<&Value>| -> BTreeMap<String, u64> {
        rows.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row.get("name")?.as_str()?.to_owned(), row.get("tokens")?.as_u64()?)))
            .collect()
    };
    Ok(Some(TokenFigures {
        source: CLAUDE_FIGURE_SOURCE.to_owned(),
        at: Some(at),
        servers: named(breakdown.get("mcpServers")),
        skills: named(breakdown.pointer("/skills/items")),
    }))
}

/// The init of this chat, or a borrowed one, and the figures of the chat
/// that gave it. A failed read becomes a note and drops nothing.
fn claude_reads(db: &Connection, own: Option<&str>, chat: &Chat) -> ClaudeReads {
    let mut failures = Vec::new();
    let mut init = None;
    if let Some(id) = own {
        match last_init(db, id) {
            Ok(found) => init = found.map(|(data, at)| (id.to_owned(), InitRead { data, at, from: None })),
            Err(error) => failures.push(error.to_string()),
        }
    }
    if init.is_none() {
        match donor(db, chat, own) {
            Ok(found) => init = found,
            Err(error) => failures.push(error.to_string()),
        }
    }
    let figures = match init.as_ref().map(|(id, _)| latest_figures(db, id)).transpose() {
        Ok(figures) => figures.flatten(),
        Err(error) => {
            failures.push(error.to_string());
            None
        }
    };
    ClaudeReads {
        init: init.map(|(_, read)| read),
        figures,
        failure: (!failures.is_empty()).then(|| failures.join(". ")),
    }
}

/// The connectors of the cached catalog, and when Bridge read them.
fn peek_catalog() -> Option<(Vec<bridge_protocol::messages::ConnectorDescriptor>, Option<String>)> {
    let (configuration, age) = crate::marketplace::peek_claude_connectors()?;
    let harnesses =
        [crate::connector_surface::HarnessConnectors { harness: "claude".to_owned(), health: configuration.connector_health }];
    let at = chrono::Duration::from_std(age).ok().map(|age| (chrono::Utc::now() - age).to_rfc3339());
    Some((crate::api::connector_descriptors(&harnesses), at))
}

fn source(kind: ChatToolSourceKind, label: &str, at: Option<String>, note: Option<String>) -> ChatToolSource {
    ChatToolSource { kind, label: label.to_owned(), at, available: true, note }
}

fn missing(kind: ChatToolSourceKind, label: &str, note: String) -> ChatToolSource {
    ChatToolSource { kind, label: label.to_owned(), at: None, available: false, note: Some(note) }
}

fn claude_list(reads: ClaudeReads, row: Option<&ToolSelectionRow>) -> Listed {
    let mut claude = reads.init.as_ref().map(|init| ClaudeSource::from_init(&init.data)).unwrap_or_default();
    let catalog = peek_catalog();
    let mut sources = Vec::new();
    match catalog {
        Some((connectors, at)) => {
            claude.catalog = connectors;
            sources.push(source(ChatToolSourceKind::Catalog, "Connector catalog", at, None));
        }
        None => sources.push(missing(ChatToolSourceKind::Catalog, "Connector catalog", NO_CATALOG_NOTE.to_owned())),
    }
    let cold = || reads.failure.clone().unwrap_or_else(|| COLD_NOTE.to_owned());
    sources.push(match &reads.init {
        Some(InitRead { at, from: Some(title), .. }) => source(
            ChatToolSourceKind::ClaudeInit,
            &format!("Claude init of {title}"),
            Some(at.clone()),
            Some(DONOR_NOTE.to_owned()),
        ),
        Some(InitRead { at, from: None, .. }) => {
            source(ChatToolSourceKind::ClaudeInit, "Claude init of this chat", Some(at.clone()), None)
        }
        None => missing(ChatToolSourceKind::ClaudeInit, "Claude init", cold()),
    });
    sources.push(match &reads.figures {
        Some(figures) => source(ChatToolSourceKind::ContextReading, CLAUDE_FIGURE_SOURCE, figures.at.clone(), None),
        None => missing(ChatToolSourceKind::ContextReading, CLAUDE_FIGURE_SOURCE, cold()),
    });
    claude.figures = reads.figures;
    (claude_units(&claude, row), sources)
}

// --- Codex and OpenCode -----------------------------------------------------------

fn has_runtime(core: &BridgeCore, session_id: &str) -> bool {
    core.adapters.lock().unwrap_or_else(PoisonError::into_inner).contains_key(session_id)
}

fn runtime_source(label: &str, at: Option<&str>, read: &Result<Value, BridgeError>) -> ChatToolSource {
    match read {
        Ok(_) => source(ChatToolSourceKind::Runtime, label, at.map(str::to_owned), None),
        Err(error) => missing(ChatToolSourceKind::Runtime, label, error.to_string()),
    }
}

fn joined(handle: std::thread::ScopedJoinHandle<'_, Result<Value, BridgeError>>) -> Result<Value, BridgeError> {
    handle.join().unwrap_or_else(|_| Err(BridgeError::Adapter("The read of the Codex runtime stopped".into())))
}

/// Three reads on the warm app-server of the chat, at the same time. Each
/// is a JSON-RPC request on the open connection, so no process starts.
fn codex_list(core: &Arc<BridgeCore>, session_id: Option<&str>, row: Option<&ToolSelectionRow>) -> Listed {
    let Some(session_id) = session_id.filter(|id| has_runtime(core, id)) else {
        return (
            codex_units(&CodexSource::default(), row),
            vec![missing(ChatToolSourceKind::Runtime, "Codex app-server", COLD_NOTE.to_owned())],
        );
    };
    let (servers, skills, config) = std::thread::scope(|scope| {
        let ask = |kind: &'static str| {
            scope.spawn(move || crate::apps::control::request(core, session_id, &json!({ "kind": kind })))
        };
        let (servers, skills, config) = (ask("servers"), ask("skills"), ask("config"));
        (joined(servers), joined(skills), joined(config))
    });
    let at = chrono::Utc::now().to_rfc3339();
    let sources = vec![
        runtime_source("Codex mcpServerStatus/list", Some(&at), &servers),
        runtime_source("Codex skills/list", Some(&at), &skills),
        runtime_source("Codex config/read", Some(&at), &config),
    ];
    let warm = CodexSource { status: servers.ok(), skills: skills.ok(), config: config.ok(), at: Some(at) };
    (codex_units(&warm, row), sources)
}

/// One list read on the warm server of the chat, after the adapter map is
/// free again.
fn opencode_list(core: &BridgeCore, session_id: Option<&str>, row: Option<&ToolSelectionRow>) -> Listed {
    let request = session_id.and_then(|id| {
        core.adapters.lock().unwrap_or_else(PoisonError::into_inner).get(id).and_then(|runtime| runtime.mcp_list_request())
    });
    let Some(request) = request else {
        return (
            opencode_units(&OpenCodeSource::default(), row),
            vec![missing(ChatToolSourceKind::Runtime, "OpenCode server", COLD_NOTE.to_owned())],
        );
    };
    let listed = request();
    let at = chrono::Utc::now().to_rfc3339();
    let sources = vec![runtime_source("OpenCode MCP list", Some(&at), &listed)];
    (opencode_units(&OpenCodeSource { mcp: listed.ok() }, row), sources)
}

#[cfg(test)]
mod tests;

//! OpenCode's store (`<data home>/opencode/opencode.db`) read into
//! transfer-bundle chats, at the fidelity the person saw them.
//!
//! Shapes were taken from a real store (opencode 2.1.1). Chats are rows in one
//! SQLite database, in two generations of tables: 2.x writes `session_v2` and
//! `session_message`; 1.x wrote `session`, `message` and `part`, and 2.x copies
//! those sessions into `session_v2` when it upgrades a store. A session in both
//! is read once, from `session_v2`.
//!
//! The store is opened read-only, so OpenCode keeps running while it is read.
//! Only the session, message and part tables are queried. The same database
//! holds `account`, `credential` and `control_account` with tokens; nothing
//! here reads them.

use crate::import_bundle::{
    AgentRole, BundleAttachment, BundleChat, BundleEntry, BundleEntryKind, BundleTool, EntriesSource, NativeRef,
    TitleSource, ToolStatus,
};
use chrono::DateTime;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

/// Which generation of tables a session lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    /// `session_v2` with `session_message` (opencode 2.x).
    V2,
    /// `session` with `message` and `part` (opencode 1.x).
    V1,
}

/// One session as the listing sees it, without its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub directory: String,
    pub title: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub archived: bool,
    pub version: Option<String>,
    /// The bytes its messages take in the store, for batching.
    pub bytes: u64,
    pub generation: Generation,
}

/// Open a store read-only. A running OpenCode keeps writing; a reader waits
/// briefly for its lock instead of failing.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn has_table(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![table],
        |row| row.get(0),
    )
}

fn sizes(conn: &Connection, sql: &str) -> rusqlite::Result<HashMap<String, u64>> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, bytes) = row?;
        *out.entry(id).or_insert(0) += u64::try_from(bytes).unwrap_or(0);
    }
    Ok(out)
}

/// Every session in the store, 2.x first, then 1.x sessions 2.x does not
/// have. Reads the session tables and message sizes, never a message.
pub fn list_sessions(conn: &Connection) -> rusqlite::Result<Vec<SessionRow>> {
    let v2 = has_table(conn, "session_v2")? && has_table(conn, "session_message")?;
    let v1 = has_table(conn, "session")? && has_table(conn, "message")?;
    let mut out = Vec::new();
    if v2 {
        let bytes = sizes(conn, "SELECT session_id, SUM(LENGTH(CAST(data AS BLOB))) FROM session_message GROUP BY session_id")?;
        read_sessions(conn, "session_v2", "", Generation::V2, &bytes, &mut out)?;
    }
    if v1 {
        let mut bytes = sizes(conn, "SELECT session_id, SUM(LENGTH(CAST(data AS BLOB))) FROM message GROUP BY session_id")?;
        if has_table(conn, "part")? {
            for (id, size) in sizes(conn, "SELECT session_id, SUM(LENGTH(CAST(data AS BLOB))) FROM part GROUP BY session_id")? {
                *bytes.entry(id).or_insert(0) += size;
            }
        }
        let skip = if v2 { "WHERE NOT EXISTS (SELECT 1 FROM session_v2 v WHERE v.id = s.id)" } else { "" };
        read_sessions(conn, "session", skip, Generation::V1, &bytes, &mut out)?;
    }
    Ok(out)
}

/// Every session's folder and whether it is a subagent's, for counting.
/// Reads the session tables only.
pub fn list_session_heads(conn: &Connection) -> rusqlite::Result<Vec<(String, bool)>> {
    let v2 = has_table(conn, "session_v2")?;
    let v1 = has_table(conn, "session")?;
    let mut sql = Vec::new();
    if v2 {
        sql.push("SELECT directory, parent_id FROM session_v2".to_owned());
    }
    if v1 {
        let skip = if v2 { " s WHERE NOT EXISTS (SELECT 1 FROM session_v2 v WHERE v.id = s.id)" } else { "" };
        sql.push(format!("SELECT directory, parent_id FROM session{skip}"));
    }
    if sql.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(&sql.join(" UNION ALL "))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            row.get::<_, Option<String>>(1)?.is_some_and(|parent| !parent.is_empty()),
        ))
    })?;
    rows.collect()
}

fn read_sessions(
    conn: &Connection,
    table: &str,
    filter: &str,
    generation: Generation,
    bytes: &HashMap<String, u64>,
    out: &mut Vec<SessionRow>,
) -> rusqlite::Result<()> {
    let sql = format!(
        "SELECT s.id, s.parent_id, s.directory, s.title, s.time_created, s.time_updated, s.time_archived, s.version
         FROM {table} s {filter}"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        Ok(SessionRow {
            bytes: bytes.get(&id).copied().unwrap_or(0),
            id,
            parent_id: row.get::<_, Option<String>>(1)?.filter(|parent| !parent.is_empty()),
            directory: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            title: row.get(3)?,
            created_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            updated_ms: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            archived: row.get::<_, Option<i64>>(6)?.is_some(),
            version: row.get(7)?,
            generation,
        })
    })?;
    for row in rows {
        out.push(row?);
    }
    Ok(())
}

/// One session by id, from whichever generation holds it.
pub fn find_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<SessionRow>> {
    for (table, generation) in [("session_v2", Generation::V2), ("session", Generation::V1)] {
        if !has_table(conn, table)? {
            continue;
        }
        let sql = format!(
            "SELECT s.id, s.parent_id, s.directory, s.title, s.time_created, s.time_updated, s.time_archived, s.version
             FROM {table} s WHERE s.id=?1"
        );
        let found = conn
            .query_row(&sql, params![id], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    parent_id: row.get::<_, Option<String>>(1)?.filter(|parent| !parent.is_empty()),
                    directory: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    title: row.get(3)?,
                    created_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    updated_ms: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    archived: row.get::<_, Option<i64>>(6)?.is_some(),
                    version: row.get(7)?,
                    bytes: 0,
                    generation,
                })
            })
            .optional()?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

/// A session read into a chat, with what Bridge did not carry.
#[derive(Debug)]
pub struct ParsedSession {
    pub chat: BundleChat,
    /// Rows and parts Bridge does not carry, by reason.
    pub skipped: BTreeMap<String, u32>,
    /// Messages the person typed in OpenCode.
    pub typed_messages: u32,
}

/// Where a session is read from, besides the row itself.
pub struct SessionSource<'a> {
    /// The store file, recorded on the native reference.
    pub store: &'a Path,
    /// The data folder holding the store.
    pub home: &'a Path,
    /// The source reference the chat is filed under.
    pub source_ref: String,
    pub label: String,
}

#[derive(Debug, Default)]
struct Reading {
    entries: Vec<BundleEntry>,
    skipped: BTreeMap<String, u32>,
    typed_messages: u32,
    model: Option<String>,
    effort: Option<String>,
}

impl Reading {
    fn skip(&mut self, reason: &str) {
        *self.skipped.entry(reason.to_owned()).or_insert(0) += 1;
    }

    fn push(&mut self, id: String, at: Option<String>, kind: BundleEntryKind, text: Option<String>, tool: Option<BundleTool>) {
        self.entries.push(BundleEntry { source_id: id, at: at.unwrap_or_default(), kind, text, tool, attachments: Vec::new() });
    }

    /// The model and its variant, from an assistant message. Bridge names an
    /// OpenCode model `<provider>/<model>`, as OpenCode's own `--model` does.
    fn note_model(&mut self, provider: Option<&str>, model: Option<&str>, variant: Option<&str>) {
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            self.model = Some(match provider.filter(|provider| !provider.is_empty()) {
                Some(provider) => format!("{provider}/{model}"),
                None => model.to_owned(),
            });
        }
        if let Some(variant) = variant.filter(|variant| !variant.is_empty() && *variant != "default") {
            self.effort = Some(variant.to_owned());
        }
    }
}

/// Read one session into a chat.
pub fn read_session(conn: &Connection, row: &SessionRow, source: &SessionSource<'_>) -> rusqlite::Result<ParsedSession> {
    let mut reading = Reading::default();
    match row.generation {
        Generation::V2 => read_v2(conn, &row.id, &mut reading)?,
        Generation::V1 => read_v1(conn, &row.id, &mut reading)?,
    }
    let title = row
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty() && !is_placeholder_title(title))
        .map(str::to_owned);
    let is_subagent = row.parent_id.is_some();
    let native_key = |id: &str| format!("opencode:{id}");
    let chat = BundleChat {
        key: native_key(&row.id),
        cwd: (!row.directory.is_empty()).then(|| row.directory.clone()),
        title_source: if title.is_some() { TitleSource::Native } else { TitleSource::Derived },
        title,
        workspace_key: None,
        pinned: false,
        pull_requests: Vec::new(),
        legacy_native_id: None,
        legacy_content_hash: None,
        created_at: millis(row.created_ms),
        updated_at: millis(row.updated_ms),
        archived: row.archived,
        harness: "opencode".into(),
        model: reading.model.take(),
        effort: reading.effort.take(),
        native_ref: Some(NativeRef {
            harness: "opencode".into(),
            session_id: row.id.clone(),
            config_root: Some(source.home.to_string_lossy().into_owned()),
            transcript_path: Some(source.store.to_string_lossy().into_owned()),
        }),
        parent_chat_key: row.parent_id.as_deref().map(native_key),
        parent_native_key: row.parent_id.as_deref().map(native_key),
        agent_role: is_subagent.then_some(AgentRole::Subagent),
        entries_source: EntriesSource::Native,
        source_ref: source.source_ref.clone(),
        source_label: source.label.clone(),
        entries: reading.entries,
    };
    Ok(ParsedSession { chat, skipped: reading.skipped, typed_messages: reading.typed_messages })
}

/// OpenCode names a session it has not titled yet "New session - <time>"
/// (a subagent's, "Child session - <time>").
fn is_placeholder_title(title: &str) -> bool {
    title.starts_with("New session - ") || title.starts_with("Child session - ")
}

fn millis(ms: i64) -> Option<String> {
    (ms > 0).then(|| DateTime::from_timestamp_millis(ms).map(|at| at.to_rfc3339())).flatten()
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

/// Row types that are OpenCode's own bookkeeping.
const V2_BOOKKEEPING: &[&str] = &["system", "idle", "location-switched"];

fn read_v2(conn: &Connection, session: &str, reading: &mut Reading) -> rusqlite::Result<()> {
    let mut statement =
        conn.prepare("SELECT id, type, time_created, data FROM session_message WHERE session_id=?1 ORDER BY seq")?;
    let mut rows = statement.query(params![session])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let at = millis(row.get::<_, Option<i64>>(2)?.unwrap_or(0));
        let Ok(data) = serde_json::from_str::<Value>(&row.get::<_, String>(3)?) else {
            reading.skip("unreadable_row");
            continue;
        };
        match kind.as_str() {
            "user" => {
                let text = str_at(&data, "/text").unwrap_or("").to_owned();
                let attachments = v2_attachments(&data);
                if text.trim().is_empty() && attachments.is_empty() {
                    continue;
                }
                reading.typed_messages += 1;
                reading.push(id, at, BundleEntryKind::UserMessage, Some(text), None);
                if let Some(entry) = reading.entries.last_mut() {
                    entry.attachments = attachments;
                }
            }
            "assistant" => {
                reading.note_model(str_at(&data, "/model/providerID"), str_at(&data, "/model/id"), str_at(&data, "/model/variant"));
                v2_assistant(&id, at, &data, reading);
            }
            "synthetic" => v2_synthetic(&id, at, &data, reading),
            "compaction" => {
                if str_at(&data, "/status").is_some_and(|status| status != "completed") {
                    continue;
                }
                let summary = str_at(&data, "/summary").unwrap_or("").trim();
                let text = if summary.is_empty() { "OpenCode compacted the conversation" } else { summary };
                reading.push(id, at, BundleEntryKind::CompactionBoundary, Some(text.to_owned()), None);
            }
            kind if V2_BOOKKEEPING.contains(&kind) => {}
            "" => reading.skip("untyped_record"),
            kind => reading.skip(&format!("row:{kind}")),
        }
    }
    Ok(())
}

/// A user message's inline images (`files: [{mime, name, source, data}]`).
fn v2_attachments(data: &Value) -> Vec<BundleAttachment> {
    data.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|file| {
                    let media_type = file.get("mime").and_then(Value::as_str)?.to_owned();
                    let inline = file.pointer("/source/type").and_then(Value::as_str).is_none_or(|kind| kind == "inline");
                    let data = file.get("data").and_then(Value::as_str).filter(|_| inline);
                    Some(BundleAttachment {
                        media_type,
                        name: file.get("name").and_then(Value::as_str).map(str::to_owned),
                        bytes: data.map_or(0, |data| (data.len() as u64) * 3 / 4),
                        data_base64: data.map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn v2_assistant(id: &str, at: Option<String>, data: &Value, reading: &mut Reading) {
    let Some(content) = data.get("content").and_then(Value::as_array) else {
        return;
    };
    for (index, item) in content.iter().enumerate() {
        let item_id = format!("{id}:{index}");
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    reading.push(item_id, at.clone(), BundleEntryKind::AssistantMessage, Some(text.to_owned()), None);
                }
            }
            "reasoning" => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    reading.push(item_id, at.clone(), BundleEntryKind::Reasoning, Some(text.to_owned()), None);
                }
            }
            "tool" => {
                let state = item.get("state").unwrap_or(&Value::Null);
                let call = Call {
                    id: item.get("id").and_then(Value::as_str).unwrap_or(&item_id),
                    name: item.get("name").and_then(Value::as_str).unwrap_or(""),
                    status: state.get("status").and_then(Value::as_str).unwrap_or(""),
                    input: state.get("input").unwrap_or(&Value::Null),
                    output: content_text(state.get("content").unwrap_or(&Value::Null)),
                    metadata: state.get("metadata").unwrap_or(&Value::Null),
                    error: str_at(state, "/error/message").map(str::to_owned),
                };
                let tool = tool_row(&call);
                reading.push(item_id, at.clone(), BundleEntryKind::ToolCall, None, Some(tool));
            }
            "" => reading.skip("untyped_content_block"),
            kind => reading.skip(&format!("content:{kind}")),
        }
    }
}

/// A row OpenCode wrote into the chat itself: a background subagent's or
/// command's result, shown as the row it finished; or its own nudge to carry
/// on, which nobody typed.
fn v2_synthetic(id: &str, at: Option<String>, data: &Value, reading: &mut Reading) {
    let text = str_at(data, "/text").unwrap_or("").to_owned();
    let description = str_at(data, "/description").unwrap_or("").to_owned();
    let state = str_at(data, "/metadata/state").unwrap_or("completed");
    let failed = matches!(state, "failed" | "error") || data.pointer("/metadata/exit").and_then(Value::as_i64).is_some_and(|code| code != 0);
    let tool = match str_at(data, "/metadata/source") {
        Some("subagent") => BundleTool {
            call_id: id.to_owned(),
            name: "task".into(),
            input: json!({ "description": description, "subagent_type": str_at(data, "/metadata/agent") }),
            output: (!text.is_empty()).then_some(text),
            status: if failed { ToolStatus::Failed } else { ToolStatus::Completed },
        },
        Some("shell") => BundleTool {
            call_id: id.to_owned(),
            name: "bash".into(),
            input: json!({ "command": description }),
            output: (!text.is_empty()).then_some(text),
            status: if failed { ToolStatus::Failed } else { ToolStatus::Completed },
        },
        None => return,
        Some(other) => {
            reading.skip(&format!("row:synthetic {other}"));
            return;
        }
    };
    reading.push(id.to_owned(), at, BundleEntryKind::ToolCall, None, Some(tool));
}

/// The text of a content list: `[{type: "text", text}, ...]`.
fn content_text(content: &Value) -> Option<String> {
    let text = content
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str).is_none_or(|kind| kind == "text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

/// One tool call, from either generation.
struct Call<'a> {
    id: &'a str,
    name: &'a str,
    status: &'a str,
    input: &'a Value,
    output: Option<String>,
    metadata: &'a Value,
    error: Option<String>,
}

/// A tool call as the row Bridge draws for the same tool run live.
fn tool_row(call: &Call<'_>) -> BundleTool {
    let key = call.name.to_ascii_lowercase();
    let exit_failed = call.metadata.get("exit").and_then(Value::as_i64).is_some_and(|code| code != 0);
    let status = match call.status {
        "error" => ToolStatus::Failed,
        _ if exit_failed => ToolStatus::Failed,
        "completed" => ToolStatus::Completed,
        _ => ToolStatus::Unknown,
    };
    let output = match (call.output.clone(), call.error.clone()) {
        (Some(output), Some(error)) => Some(format!("{output}\n{error}")),
        (output, error) => output.or(error),
    };
    let with_file = |input: &Value| {
        let mut input = input.clone();
        let file = ["filePath", "path", "file_path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str).map(str::to_owned));
        if let (Some(map), Some(file)) = (input.as_object_mut(), file) {
            map.insert("file_path".into(), Value::String(file));
        }
        input
    };
    let (name, input, output) = match key.as_str() {
        "shell" | "bash" => ("bash".to_owned(), call.input.clone(), output),
        "read" => ("read".to_owned(), with_file(call.input), output),
        "edit" | "multiedit" => {
            // OpenCode keeps the edit's diff beside it; the diff is what the
            // person saw.
            let diff = call.metadata.get("diff").and_then(Value::as_str).filter(|diff| !diff.is_empty());
            ("edit".to_owned(), with_file(call.input), diff.map(str::to_owned).or(output))
        }
        "write" => ("write".to_owned(), with_file(call.input), output),
        "patch" | "apply_patch" => ("apply_patch".to_owned(), call.input.clone(), output),
        "task" => ("task".to_owned(), call.input.clone(), output),
        "subagent" => {
            let mut input = call.input.clone();
            if let Some(map) = input.as_object_mut() {
                if let Some(agent) = map.get("agent").cloned() {
                    map.insert("subagent_type".into(), agent);
                }
            }
            ("task".to_owned(), input, output)
        }
        _ => (call.name.to_owned(), call.input.clone(), output),
    };
    BundleTool { call_id: call.id.to_owned(), name, input, output, status }
}

/// 1.x part types that are OpenCode's own bookkeeping.
const V1_BOOKKEEPING: &[&str] = &["step-start", "step-finish", "snapshot", "patch", "agent", "retry", "subtask"];

fn read_v1(conn: &Connection, session: &str, reading: &mut Reading) -> rusqlite::Result<()> {
    let parts_table = has_table(conn, "part")?;
    let mut messages = conn.prepare("SELECT id, time_created, data FROM message WHERE session_id=?1 ORDER BY time_created, id")?;
    let mut parts = if parts_table { Some(conn.prepare("SELECT id, data FROM part WHERE message_id=?1 ORDER BY id")?) } else { None };
    let mut rows = messages.query(params![session])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let at = millis(row.get::<_, Option<i64>>(1)?.unwrap_or(0));
        let Ok(message) = serde_json::from_str::<Value>(&row.get::<_, String>(2)?) else {
            reading.skip("unreadable_row");
            continue;
        };
        let role = str_at(&message, "/role").unwrap_or("");
        let summary = message.get("summary").and_then(Value::as_bool) == Some(true);
        // A compaction's summary may run on another model; the chat's is
        // the one its replies used.
        if role == "assistant" && !summary {
            reading.note_model(str_at(&message, "/providerID"), str_at(&message, "/modelID"), str_at(&message, "/variant"));
        }
        let mut message_parts = Vec::new();
        if let Some(parts) = parts.as_mut() {
            let mut part_rows = parts.query(params![id])?;
            while let Some(part) = part_rows.next()? {
                let part_id: String = part.get(0)?;
                match serde_json::from_str::<Value>(&part.get::<_, String>(1)?) {
                    Ok(value) => message_parts.push((part_id, value)),
                    Err(_) => reading.skip("unreadable_row"),
                }
            }
        }
        match role {
            "user" => v1_user(&id, at, &message_parts, reading),
            "assistant" if summary => {
                v1_summary(&id, at, &message_parts, reading);
            }
            "assistant" => v1_assistant(at, &message_parts, reading),
            "" => reading.skip("untyped_record"),
            other => reading.skip(&format!("row:{other}")),
        }
    }
    Ok(())
}

fn v1_user(id: &str, at: Option<String>, parts: &[(String, Value)], reading: &mut Reading) {
    let mut text = Vec::new();
    let mut attachments = Vec::new();
    let mut compacted = false;
    for (_, part) in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            // OpenCode's own words in the user's turn (file contents it read
            // in, reminders) are not what the person typed.
            "text" if part.get("synthetic").and_then(Value::as_bool) == Some(true) => {}
            "text" => text.extend(part.get("text").and_then(Value::as_str).filter(|text| !text.trim().is_empty())),
            "file" => {
                let media_type = part.get("mime").and_then(Value::as_str).unwrap_or("").to_owned();
                let data = part
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(|url| url.strip_prefix("data:"))
                    .and_then(|rest| rest.split_once(";base64,"))
                    .map(|(_, data)| data.to_owned());
                attachments.push(BundleAttachment {
                    media_type,
                    name: part.get("filename").and_then(Value::as_str).map(str::to_owned),
                    bytes: data.as_ref().map_or(0, |data| (data.len() as u64) * 3 / 4),
                    data_base64: data,
                });
            }
            "compaction" => compacted = true,
            kind if V1_BOOKKEEPING.contains(&kind) => {}
            "" => reading.skip("untyped_content_block"),
            kind => reading.skip(&format!("part:{kind}")),
        }
    }
    if compacted {
        reading.push(format!("{id}:compaction"), at.clone(), BundleEntryKind::CompactionBoundary, Some("OpenCode compacted the conversation".into()), None);
    }
    if text.is_empty() && attachments.is_empty() {
        return;
    }
    reading.typed_messages += 1;
    reading.push(id.to_owned(), at, BundleEntryKind::UserMessage, Some(text.join("\n")), None);
    if let Some(entry) = reading.entries.last_mut() {
        entry.attachments = attachments;
    }
}

/// The summary a 1.x compaction wrote: the text of the compaction point.
fn v1_summary(id: &str, at: Option<String>, parts: &[(String, Value)], reading: &mut Reading) {
    let summary = parts
        .iter()
        .filter(|(_, part)| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|(_, part)| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if summary.trim().is_empty() {
        return;
    }
    match reading.entries.last_mut() {
        Some(last) if last.kind == BundleEntryKind::CompactionBoundary => last.text = Some(summary),
        _ => reading.push(format!("{id}:summary"), at, BundleEntryKind::CompactionBoundary, Some(summary), None),
    }
}

fn v1_assistant(at: Option<String>, parts: &[(String, Value)], reading: &mut Reading) {
    for (part_id, part) in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" if part.get("synthetic").and_then(Value::as_bool) == Some(true) => {}
            "text" => {
                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    reading.push(part_id.clone(), at.clone(), BundleEntryKind::AssistantMessage, Some(text.to_owned()), None);
                }
            }
            "reasoning" => {
                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    reading.push(part_id.clone(), at.clone(), BundleEntryKind::Reasoning, Some(text.to_owned()), None);
                }
            }
            "tool" => {
                let state = part.get("state").unwrap_or(&Value::Null);
                let call = Call {
                    id: part.get("callID").and_then(Value::as_str).unwrap_or(part_id),
                    name: part.get("tool").and_then(Value::as_str).unwrap_or(""),
                    status: state.get("status").and_then(Value::as_str).unwrap_or(""),
                    input: state.get("input").unwrap_or(&Value::Null),
                    output: state.get("output").and_then(Value::as_str).filter(|text| !text.is_empty()).map(str::to_owned),
                    metadata: state.get("metadata").unwrap_or(&Value::Null),
                    error: state.get("error").and_then(Value::as_str).map(str::to_owned),
                };
                let tool = tool_row(&call);
                reading.push(part_id.clone(), at.clone(), BundleEntryKind::ToolCall, None, Some(tool));
            }
            kind if V1_BOOKKEEPING.contains(&kind) => {}
            "" => reading.skip("untyped_content_block"),
            kind => reading.skip(&format!("part:{kind}")),
        }
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;

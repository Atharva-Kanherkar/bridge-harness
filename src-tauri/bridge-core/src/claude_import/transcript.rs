//! Claude Code transcripts (`<config>/projects/<slug>/<session>.jsonl`) read
//! into transfer-bundle chats, at the fidelity the person saw them: text,
//! thinking, tool calls with their results, images, compaction points, and
//! each subagent's own transcript as a child chat.
//!
//! Shapes were taken from a real store of 81,567 transcripts (1.57 M lines):
//! Claude Code writes one content block per `assistant` record, results come
//! back as `tool_result` blocks inside `user` records, subagents live in
//! `<session>/subagents/agent-<id>.jsonl` next to an `agent-<id>.meta.json`,
//! and a compaction is a `system` record with subtype `compact_boundary`
//! followed by a `user` record flagged `isCompactSummary`.

use crate::import_bundle::{
    AgentRole, BundleAttachment, BundleChat, BundleEntry, BundleEntryKind, BundleTool,
    EntriesSource, NativeRef, TitleSource, ToolStatus,
};
use chrono::DateTime;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// Transcripts larger than this are reported, not read. Real stores reach
/// 141 MB; the reader streams, so this is a guard, not the working limit.
pub const MAX_TRANSCRIPT_BYTES: u64 = 512 * 1024 * 1024;

/// One line larger than this is skipped and counted (a pasted binary blob).
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Record types that are Claude Code's own bookkeeping, not conversation.
/// Expected in every real transcript, so they are skipped without a count.
const BOOKKEEPING: &[&str] = &[
    "attachment",
    "queue-operation",
    "last-prompt",
    "atis-latch",
    "cost-state",
    "session-owner",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "mode",
    "pr-link",
    "tools-used",
    "frame-link",
    "worktree-state",
    "relocated",
    "started",
    "result",
    "file-history-delta",
    "artifact-comment-monitor",
    "summary",
    "file-history-snapshot",
    "artifact-autoreact-ledger",
    "permission-mode",
];

/// `system` subtypes that are UI plumbing rather than conversation.
const SYSTEM_BOOKKEEPING: &[&str] = &[
    "local_command",
    "stop_hook_summary",
    "api_error",
    "informational",
    "model_refusal_fallback",
    "turn_duration",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Skipped(pub BTreeMap<String, u32>);

impl Skipped {
    fn bump(&mut self, reason: impl Into<String>) {
        *self.0.entry(reason.into()).or_insert(0) += 1;
    }
}

#[derive(Debug, Clone)]
pub struct ParsedTranscript {
    pub chat: BundleChat,
    pub skipped: Skipped,
    /// Raw count of user and assistant content the person would have seen,
    /// for the parity check: every one is either an entry or counted in `skipped`.
    pub source_items: u32,
    /// Older transcripts keep subagent turns inline (`isSidechain` with an
    /// `agentId`) instead of in `subagents/`. One reader per agent, by id.
    pub inline_subagents: Vec<(String, ParsedTranscript)>,
}

#[derive(Debug, Default)]
struct Reader {
    /// Set on a session's own file: sidechain records go to `sidechains`.
    split_sidechains: bool,
    sidechains: BTreeMap<String, Reader>,
    entries: Vec<BundleEntry>,
    tool_index: HashMap<String, usize>,
    seen: HashSet<String>,
    skipped: Skipped,
    source_items: u32,
    session_id: Option<String>,
    cwd: Option<String>,
    first_at: Option<String>,
    last_at: Option<String>,
    model: Option<String>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    pending_compaction: Option<usize>,
    user_said: String,
}

fn valid_timestamp(object: &Map<String, Value>) -> Option<String> {
    let at = object.get("timestamp")?.as_str()?;
    DateTime::parse_from_rfc3339(at).ok()?;
    Some(at.to_owned())
}

/// `<command-name>/model</command-name><command-args>opus</command-args>` is
/// how Claude Code stores a slash command; show it as the person typed it.
fn tidy_user_text(text: &str) -> String {
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&close)? + start;
        Some(text[start..end].trim().to_owned())
    };
    if let Some(command) = tag("command-name") {
        let args = tag("command-args").unwrap_or_default();
        let command = if command.starts_with('/') { command } else { format!("/{command}") };
        return if args.is_empty() { command } else { format!("{command} {args}") };
    }
    text.to_owned()
}

fn tool_result_text(content: Option<&Value>) -> (String, u32) {
    match content {
        Some(Value::String(text)) => (text.clone(), 0),
        Some(Value::Array(items)) => {
            let mut parts = Vec::new();
            let mut images = 0;
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            parts.push(text.to_owned());
                        }
                    }
                    Some("image") => {
                        images += 1;
                        parts.push("[image]".into());
                    }
                    // A tool search's reference list: names, not output.
                    Some("tool_reference") => {
                        if let Some(name) = item.get("tool_name").and_then(Value::as_str) {
                            parts.push(name.to_owned());
                        }
                    }
                    _ => {}
                }
            }
            (parts.join("\n"), images)
        }
        _ => (String::new(), 0),
    }
}

impl Reader {
    fn push(&mut self, entry: BundleEntry) {
        self.entries.push(entry);
    }

    fn note_time(&mut self, at: &str) {
        if self.first_at.as_deref().is_none_or(|first| at < first) {
            self.first_at = Some(at.to_owned());
        }
        if self.last_at.as_deref().is_none_or(|last| at > last) {
            self.last_at = Some(at.to_owned());
        }
    }

    fn record(&mut self, record: &Value) {
        let Some(object) = record.as_object() else {
            self.skipped.bump("non_object_line");
            return;
        };
        if self.split_sidechains && object.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            let agent = object
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .unwrap_or("sidechain")
                .to_owned();
            self.sidechains.entry(agent).or_default().record(record);
            return;
        }
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            self.skipped.bump("untyped_record");
            return;
        };
        if self.session_id.is_none() {
            self.session_id = object.get("sessionId").and_then(Value::as_str).map(str::to_owned);
        }
        match kind {
            "custom-title" => {
                self.custom_title = object.get("customTitle").and_then(Value::as_str).map(str::to_owned);
            }
            "ai-title" => {
                self.ai_title = object.get("aiTitle").and_then(Value::as_str).map(str::to_owned);
            }
            "agent-name" => {}
            "system" => self.system(object),
            "user" => self.user(object),
            "assistant" => self.assistant(object),
            other if BOOKKEEPING.contains(&other) => {}
            other => self.skipped.bump(format!("record:{other}")),
        }
    }

    fn system(&mut self, object: &Map<String, Value>) {
        let subtype = object.get("subtype").and_then(Value::as_str).unwrap_or("");
        if subtype != "compact_boundary" {
            if !SYSTEM_BOOKKEEPING.contains(&subtype) {
                self.skipped.bump(format!("system:{subtype}"));
            }
            return;
        }
        let Some(at) = valid_timestamp(object) else {
            self.skipped.bump("invalid_timestamp");
            return;
        };
        let id = object.get("uuid").and_then(Value::as_str).unwrap_or("compact").to_owned();
        if !self.seen.insert(format!("{id}:compact")) {
            return;
        }
        self.note_time(&at);
        self.pending_compaction = Some(self.entries.len());
        self.push(BundleEntry {
            source_id: id,
            at,
            kind: BundleEntryKind::CompactionBoundary,
            text: None,
            tool: None,
            attachments: Vec::new(),
        });
    }

    fn user(&mut self, object: &Map<String, Value>) {
        if object.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let Some(uuid) = object.get("uuid").and_then(Value::as_str).map(str::to_owned) else {
            self.skipped.bump("missing_uuid");
            return;
        };
        let Some(at) = valid_timestamp(object) else {
            self.skipped.bump("invalid_timestamp");
            return;
        };
        if self.cwd.is_none() {
            self.cwd = object.get("cwd").and_then(Value::as_str).map(str::to_owned);
        }
        let content = object.get("message").and_then(|message| message.get("content"));
        if object.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
            self.compact_summary(uuid, at, content);
            return;
        }
        if !self.seen.insert(uuid.clone()) {
            self.skipped.bump("duplicate_source_id");
            return;
        }
        self.note_time(&at);
        match content {
            Some(Value::String(text)) => {
                self.source_items += 1;
                let text = tidy_user_text(text);
                self.said(&text);
                self.push(BundleEntry {
                    source_id: uuid,
                    at,
                    kind: BundleEntryKind::UserMessage,
                    text: Some(text),
                    tool: None,
                    attachments: Vec::new(),
                });
            }
            Some(Value::Array(blocks)) => self.user_blocks(uuid, at, blocks),
            _ => self.skipped.bump("unsupported_content_shape"),
        }
    }

    /// A compaction summary fills the boundary just before it, or stands
    /// alone as a boundary when the boundary record is missing.
    fn compact_summary(&mut self, uuid: String, at: String, content: Option<&Value>) {
        let (summary, _) = tool_result_text(content);
        let summary = match content {
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => summary,
        };
        match self.pending_compaction.take() {
            Some(index) => self.entries[index].text = Some(summary),
            None => {
                self.note_time(&at);
                self.push(BundleEntry {
                    source_id: uuid,
                    at,
                    kind: BundleEntryKind::CompactionBoundary,
                    text: Some(summary),
                    tool: None,
                    attachments: Vec::new(),
                });
            }
        }
    }

    /// A user record made of blocks: text and images become one message,
    /// tool results fill in the calls they answer.
    fn user_blocks(&mut self, uuid: String, at: String, blocks: &[Value]) {
        let mut texts = Vec::new();
        let mut attachments = Vec::new();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        texts.push(tidy_user_text(text));
                    }
                }
                Some("image") => {
                    let source = block.get("source");
                    let data = source
                        .and_then(|source| source.get("data"))
                        .and_then(Value::as_str);
                    let media_type = source
                        .and_then(|source| source.get("media_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    attachments.push(BundleAttachment {
                        media_type: media_type.to_owned(),
                        name: None,
                        bytes: data.map(|data| data.len() as u64 * 3 / 4).unwrap_or(0),
                        data_base64: data.map(str::to_owned),
                    });
                }
                Some("tool_result") => self.tool_result(block),
                Some("document") => self.skipped.bump("content:document"),
                Some(other) => self.skipped.bump(format!("content:{other}")),
                None => self.skipped.bump("untyped_content_block"),
            }
        }
        if !texts.is_empty() || !attachments.is_empty() {
            self.source_items += 1;
            let text = texts.join("\n\n");
            self.said(&text);
            self.push(BundleEntry {
                source_id: uuid,
                at,
                kind: BundleEntryKind::UserMessage,
                text: Some(text),
                tool: None,
                attachments,
            });
        }
    }

    fn said(&mut self, text: &str) {
        if self.user_said.len() < 4_000 {
            self.user_said.push_str(text);
            self.user_said.push('\n');
        }
    }

    fn tool_result(&mut self, block: &Value) {
        let Some(call_id) = block.get("tool_use_id").and_then(Value::as_str) else {
            self.skipped.bump("tool_result_without_call");
            return;
        };
        let (output, _) = tool_result_text(block.get("content"));
        let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
        match self.tool_index.get(call_id).copied() {
            Some(index) => {
                if let Some(tool) = self.entries[index].tool.as_mut() {
                    tool.output = Some(output);
                    tool.status = if failed { ToolStatus::Failed } else { ToolStatus::Completed };
                }
            }
            None => self.skipped.bump("tool_result_without_call"),
        }
    }

    fn assistant(&mut self, object: &Map<String, Value>) {
        let Some(uuid) = object.get("uuid").and_then(Value::as_str).map(str::to_owned) else {
            self.skipped.bump("missing_uuid");
            return;
        };
        let Some(at) = valid_timestamp(object) else {
            self.skipped.bump("invalid_timestamp");
            return;
        };
        let Some(message) = object.get("message").and_then(Value::as_object) else {
            self.skipped.bump("missing_message_payload");
            return;
        };
        // Each assistant record has its own uuid even when several share one
        // API message, so a repeated uuid is a repeated record.
        if !self.seen.insert(uuid.clone()) {
            self.skipped.bump("duplicate_source_id");
            return;
        }
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            if !model.starts_with('<') {
                self.model = Some(model.to_owned());
            }
        }
        self.note_time(&at);
        let blocks: Vec<Value> = match message.get("content") {
            Some(Value::Array(blocks)) => blocks.clone(),
            Some(Value::String(text)) => vec![serde_json::json!({ "type": "text", "text": text })],
            _ => {
                self.skipped.bump("unsupported_content_shape");
                return;
            }
        };
        for (index, block) in blocks.iter().enumerate() {
            let source_id = format!("{uuid}:{index}");
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    self.source_items += 1;
                    self.push(BundleEntry {
                        source_id,
                        at: at.clone(),
                        kind: BundleEntryKind::AssistantMessage,
                        text: block.get("text").and_then(Value::as_str).map(str::to_owned),
                        tool: None,
                        attachments: Vec::new(),
                    });
                }
                Some("thinking") => {
                    let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                    // Claude Code keeps only a signature for thinking it was not
                    // allowed to show; an empty block has nothing to import.
                    if text.trim().is_empty() {
                        continue;
                    }
                    self.source_items += 1;
                    self.push(BundleEntry {
                        source_id,
                        at: at.clone(),
                        kind: BundleEntryKind::Reasoning,
                        text: Some(text.to_owned()),
                        tool: None,
                        attachments: Vec::new(),
                    });
                }
                Some("tool_use") => {
                    self.source_items += 1;
                    let call_id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or(&source_id)
                        .to_owned();
                    self.tool_index.insert(call_id.clone(), self.entries.len());
                    self.push(BundleEntry {
                        source_id,
                        at: at.clone(),
                        kind: BundleEntryKind::ToolCall,
                        text: None,
                        tool: Some(BundleTool {
                            call_id,
                            name: block.get("name").and_then(Value::as_str).unwrap_or("tool").to_owned(),
                            input: block.get("input").cloned().unwrap_or(Value::Null),
                            output: None,
                            status: ToolStatus::Unknown,
                        }),
                        attachments: Vec::new(),
                    });
                }
                Some("redacted_thinking") => self.skipped.bump("content:redacted_thinking"),
                Some(other) => self.skipped.bump(format!("content:{other}")),
                None => self.skipped.bump("untyped_content_block"),
            }
        }
    }
}

/// Skip the rest of the current line through the reader's own buffer,
/// without collecting it: a rejected line never costs more than that buffer.
fn discard_line<R: BufRead>(reader: &mut R) -> std::io::Result<u64> {
    let mut skipped = 0u64;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(skipped);
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => {
                reader.consume(end + 1);
                return Ok(skipped + end as u64 + 1);
            }
            None => {
                let length = available.len();
                reader.consume(length);
                skipped += length as u64;
            }
        }
    }
}

/// Read one transcript file. `key` and `label` name it inside the bundle.
pub fn read_transcript(path: &Path, key: &str, label: &str) -> std::io::Result<ParsedTranscript> {
    read_transcript_file(path, key, label, false)
}

fn read_transcript_file(
    path: &Path,
    key: &str,
    label: &str,
    split_sidechains: bool,
) -> std::io::Result<ParsedTranscript> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_TRANSCRIPT_BYTES {
        return Err(std::io::Error::other(format!(
            "transcript is {size} bytes, over the {MAX_TRANSCRIPT_BYTES} byte import limit"
        )));
    }
    let mut reader = Reader {
        split_sidechains,
        ..Reader::default()
    };
    let mut lines = BufReader::new(File::open(path)?);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = lines.by_ref().take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        if buffer.len() > MAX_LINE_BYTES && !buffer.ends_with(b"\n") {
            reader.skipped.bump("oversized_line");
            discard_line(&mut lines)?;
            continue;
        }
        let line = String::from_utf8_lossy(&buffer);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(record) => reader.record(&record),
            Err(_) => reader.skipped.bump("malformed_line"),
        }
    }
    Ok(finish(reader, path, key, label))
}

/// Turn a finished reader into a chat. `path` is the file it was read from.
fn finish(mut reader: Reader, path: &Path, key: &str, label: &str) -> ParsedTranscript {
    let inline_subagents = std::mem::take(&mut reader.sidechains)
        .into_iter()
        .map(|(agent, child)| {
            let child_key = format!("{key}#agent-{agent}");
            let child_label = format!("{label} > agent-{agent}");
            (agent, finish(child, path, &child_key, &child_label))
        })
        .collect();
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(key)
        .to_owned();
    let session_id = reader.session_id.clone().unwrap_or(stem);
    let (title, title_source) = match (&reader.custom_title, &reader.ai_title) {
        (Some(title), _) if !title.trim().is_empty() => (Some(title.clone()), TitleSource::User),
        (_, Some(title))
            if !title.trim().is_empty()
                && crate::session_titles::grounded(title, &reader.user_said) =>
        {
            (Some(title.clone()), TitleSource::Native)
        }
        _ => (None, TitleSource::Derived),
    };
    let chat = BundleChat {
        key: key.to_owned(),
        workspace_key: None,
        cwd: reader.cwd.clone(),
        title,
        title_source,
        created_at: reader.first_at.clone(),
        updated_at: reader.last_at.clone(),
        archived: false,
        pinned: false,
        harness: "claude".into(),
        model: reader.model.clone(),
        effort: None,
        native_ref: Some(NativeRef {
            harness: "claude".into(),
            session_id,
            config_root: None,
            transcript_path: Some(path.to_string_lossy().into_owned()),
        }),
        parent_chat_key: None,
        agent_role: None,
        entries_source: if reader.entries.is_empty() {
            EntriesSource::None
        } else {
            EntriesSource::Native
        },
        entries: reader.entries,
        pull_requests: Vec::new(),
        source_ref: path.to_string_lossy().into_owned(),
        source_label: label.to_owned(),
    };
    ParsedTranscript {
        chat,
        skipped: reader.skipped,
        source_items: reader.source_items,
        inline_subagents,
    }
}

/// A session's subagent transcripts: `<dir>/<session>/subagents/agent-*.jsonl`.
pub fn subagent_files(main: &Path) -> Vec<PathBuf> {
    let Some(stem) = main.file_stem() else {
        return Vec::new();
    };
    let dir = main.with_file_name(stem).join("subagents");
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("agent-"))
        })
        .collect();
    files.sort();
    files
}

/// A subagent transcript that was not brought over, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildFailure {
    pub label: String,
    pub reason: String,
}

/// A session and its subagents, plus every subagent that could not be read.
#[derive(Debug, Clone)]
pub struct SessionRead {
    pub chats: Vec<ParsedTranscript>,
    pub failures: Vec<ChildFailure>,
}

/// Read a session and its subagents: the main chat first, then one child chat
/// per subagent, linked by `parent_chat_key`. Subagent files and their
/// `.meta.json` are opened only when they resolve inside `approved_roots`.
pub fn read_session(
    main: &Path,
    label: &str,
    approved_roots: &[PathBuf],
) -> std::io::Result<SessionRead> {
    let key = main.to_string_lossy().into_owned();
    let mut parsed = read_transcript_file(main, &key, label, true)?;
    let inline = std::mem::take(&mut parsed.inline_subagents);
    let parent_session = parsed
        .chat
        .native_ref
        .as_ref()
        .map(|native| native.session_id.clone())
        .unwrap_or_default();
    let mut read = SessionRead {
        chats: vec![parsed],
        failures: Vec::new(),
    };
    for found in subagent_files(main) {
        let agent = found
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("agent")
            .to_owned();
        let child_label = format!("{label} > {agent}");
        let Ok(file) = crate::external_import::canonicalize_approved_path(approved_roots, &found)
        else {
            read.failures.push(ChildFailure {
                label: child_label,
                reason: "the subagent transcript is outside the approved folders and was not read"
                    .into(),
            });
            continue;
        };
        let mut child = match read_transcript(&file, &found.to_string_lossy(), &child_label) {
            Ok(child) => child,
            Err(error) => {
                read.failures.push(ChildFailure {
                    label: child_label,
                    reason: format!("the subagent transcript could not be read: {error}"),
                });
                continue;
            }
        };
        let meta = read_agent_meta(&found, approved_roots, &child_label, &mut read.failures);
        name_subagent(&mut child.chat, &key, &parent_session, &agent, meta.as_ref());
        read.chats.push(child);
    }
    // Inline subagents that also have their own file appear once: the file.
    let from_files: HashSet<String> = subagent_files(main)
        .iter()
        .filter_map(|file| file.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned))
        .collect();
    for (agent, mut child) in inline {
        let agent = format!("agent-{agent}");
        if from_files.contains(&agent) {
            continue;
        }
        name_subagent(&mut child.chat, &key, &parent_session, &agent, None);
        read.chats.push(child);
    }
    Ok(read)
}

/// A subagent's `agent-<id>.meta.json`, when it exists inside the approved folders.
fn read_agent_meta(
    transcript: &Path,
    approved_roots: &[PathBuf],
    label: &str,
    failures: &mut Vec<ChildFailure>,
) -> Option<Value> {
    let path = transcript.with_extension("meta.json");
    if std::fs::symlink_metadata(&path).is_err() {
        return None;
    }
    let Ok(path) = crate::external_import::canonicalize_approved_path(approved_roots, &path) else {
        failures.push(ChildFailure {
            label: label.to_owned(),
            reason: "the subagent's details are outside the approved folders and were not read"
                .into(),
        });
        return None;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
}

/// Link a subagent chat to its session and name it from its type and task.
fn name_subagent(
    chat: &mut BundleChat,
    parent_key: &str,
    parent_session: &str,
    agent: &str,
    meta: Option<&Value>,
) {
    let agent_type = meta
        .and_then(|meta| meta.get("agentType"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    chat.parent_chat_key = Some(parent_key.to_owned());
    chat.agent_role = Some(AgentRole::Subagent);
    if chat.model.is_none() {
        chat.model = meta
            .and_then(|meta| meta.get("model"))
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(native) = chat.native_ref.as_mut() {
        native.session_id = format!("{parent_session}/{agent}");
    }
    if chat.title_source == TitleSource::Derived {
        let task = chat
            .entries
            .iter()
            .find(|entry| entry.kind == BundleEntryKind::UserMessage)
            .and_then(|entry| entry.text.as_deref())
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
            .map(|line| line.trim().chars().take(70).collect::<String>());
        chat.title = Some(match (agent_type.as_deref(), task) {
            (Some(kind), Some(task)) => format!("{kind}: {task}"),
            (Some(kind), None) => format!("{kind} subagent"),
            (None, Some(task)) => task,
            (None, None) => "Subagent".into(),
        });
        chat.title_source = TitleSource::Tool;
    }
}

/// What the review list shows for a session without reading all of it: the
/// title (custom over grounded AI title over first message), the folder, and
/// when it started. Reads at most the first and last 64 KiB.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPeek {
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
}

pub fn peek_session(path: &Path) -> SessionPeek {
    const WINDOW: u64 = 64 * 1024;
    let Ok(mut file) = File::open(path) else {
        return SessionPeek::default();
    };
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut head = Vec::new();
    if (&mut file).take(WINDOW).read_to_end(&mut head).is_err() {
        return SessionPeek::default();
    }
    let mut tail = Vec::new();
    if size > WINDOW {
        use std::io::{Seek, SeekFrom};
        if file.seek(SeekFrom::Start(size.saturating_sub(WINDOW))).is_ok() {
            if file.take(WINDOW).read_to_end(&mut tail).is_err() {
                tail.clear();
            }
        }
    }
    let mut peek = SessionPeek::default();
    let mut first_user: Option<String> = None;
    let mut custom = None;
    let mut ai = None;
    for chunk in [&head, &tail] {
        for line in String::from_utf8_lossy(chunk).lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match record.get("type").and_then(Value::as_str) {
                Some("custom-title") => {
                    custom = record.get("customTitle").and_then(Value::as_str).map(str::to_owned)
                }
                Some("ai-title") => ai = record.get("aiTitle").and_then(Value::as_str).map(str::to_owned),
                Some("user") if record.get("isMeta").and_then(Value::as_bool) != Some(true) => {
                    if peek.cwd.is_none() {
                        peek.cwd = record.get("cwd").and_then(Value::as_str).map(str::to_owned);
                    }
                    if peek.started_at.is_none() {
                        peek.started_at = record.get("timestamp").and_then(Value::as_str).map(str::to_owned);
                    }
                    if first_user.is_none() && record.get("isCompactSummary").is_none() {
                        let content = record.pointer("/message/content");
                        let text = match content {
                            Some(Value::String(text)) => Some(tidy_user_text(text)),
                            Some(Value::Array(blocks)) => blocks
                                .iter()
                                .find_map(|block| block.get("text").and_then(Value::as_str))
                                .map(tidy_user_text),
                            _ => None,
                        };
                        first_user = text.filter(|text| !text.trim().is_empty());
                    }
                }
                _ => {}
            }
        }
    }
    let said = first_user.clone().unwrap_or_default();
    peek.title = custom
        .filter(|title| !title.trim().is_empty())
        .or_else(|| ai.filter(|title| crate::session_titles::grounded(title, &said)))
        .or_else(|| {
            first_user.map(|text| text.lines().next().unwrap_or_default().chars().take(80).collect())
        });
    peek
}

#[cfg(test)]
mod tests;

//! The transfer bundle: one format every import source converts into.
//!
//! Adapters (Claude Code, Codex, OpenCode, and the ADEs that wrap them) read
//! their own storage and emit a [`TransferBundle`]. Nothing else. The
//! [`resolve`] step merges bundles from several tools so one native session
//! becomes one Bridge chat however many tools point at it, and [`lower`]
//! turns the merged result into the existing [`ImportCandidate`] pipeline,
//! which owns consent, the ledger, provenance and the atomic write.
//!
//! The three Bridge hierarchies stay separate here as everywhere else:
//! projects and workspaces (the workspace tree), `parent_chat_key` and
//! `agent_role` (the agent tree, always historical), and `entries` (the
//! conversation tree of one chat).

use crate::BridgeError;
use crate::external_import::{
    content_hash, deterministic_identity, path_fingerprint, sanitize_import_payload,
    CandidateKind, ImportCandidate, ImportDiagnostic, ImportSource, SourceClassification,
    Stability,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// The only bundle version this build reads. A newer bundle fails closed.
pub const BUNDLE_VERSION: u32 = 1;

/// Images bigger than this stay out of the stored chat and are reported.
pub const MAX_INLINE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;

/// Tool output kept per call. The rest is cut with a visible marker so one
/// enormous `cat` does not make a chat unloadable.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferBundle {
    pub bundle_version: u32,
    pub source: BundleSource,
    #[serde(default)]
    pub projects: Vec<BundleProject>,
    #[serde(default)]
    pub workspaces: Vec<BundleWorkspace>,
    #[serde(default)]
    pub chats: Vec<BundleChat>,
    #[serde(default)]
    pub memories: Vec<BundleMemory>,
    #[serde(default)]
    pub setup: Vec<BundleSetup>,
    #[serde(default)]
    pub preferences: Vec<BundlePreference>,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
    #[serde(default)]
    pub excluded: Vec<BundleExclusion>,
}

impl TransferBundle {
    pub fn new(source: BundleSource) -> Self {
        Self {
            bundle_version: BUNDLE_VERSION,
            source,
            projects: Vec::new(),
            workspaces: Vec::new(),
            chats: Vec::new(),
            memories: Vec::new(),
            setup: Vec::new(),
            preferences: Vec::new(),
            diagnostics: Vec::new(),
            excluded: Vec::new(),
        }
    }

    /// Record something that did not come over, merging repeat categories.
    pub fn exclude(&mut self, category: &str, source_label: &str, count: u32, reason: &str) {
        if count == 0 {
            return;
        }
        if let Some(existing) = self.excluded.iter_mut().find(|item| {
            item.category == category && item.source_label == source_label && item.reason == reason
        }) {
            existing.count += count;
            return;
        }
        self.excluded.push(BundleExclusion {
            category: category.into(),
            source_label: source_label.into(),
            count,
            reason: reason.into(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleSource {
    /// Stable tool id: `claude_code`, `codex`, `opencode`, `conductor`, `t3code`, `orca`, `superset`.
    pub tool: String,
    pub tool_version: Option<String>,
    pub schema_version: Option<String>,
    pub adapter_version: String,
    #[serde(default)]
    pub roots: Vec<String>,
    pub detected_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleProject {
    pub key: String,
    pub name: String,
    pub repo_root: String,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleWorkspace {
    pub key: String,
    pub project_key: Option<String>,
    pub path: String,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeRef {
    /// The harness that wrote the transcript: `claude`, `codex`, `opencode`, `cursor`.
    pub harness: String,
    pub session_id: String,
    pub config_root: Option<String>,
    pub transcript_path: Option<String>,
}

impl NativeRef {
    pub fn key(&self) -> String {
        format!("{}:{}", self.harness, self.session_id)
    }
}

/// Where a chat's title came from, strongest first. A title the person typed
/// in any tool beats one a tool generated, which beats the harness's own
/// guess, which beats one Bridge derived from the first message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    User,
    Tool,
    Native,
    Derived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntriesSource {
    /// Read from the harness that wrote the conversation.
    Native,
    /// The wrapping tool's own copy, used when the native transcript is gone.
    ToolCopy,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Orchestrator,
    Worker,
    Subagent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleChat {
    /// Unique inside one bundle.
    pub key: String,
    pub workspace_key: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub title_source: TitleSource,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pinned: bool,
    /// `claude`, `codex`, `opencode`, `cursor` or `other`.
    pub harness: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub native_ref: Option<NativeRef>,
    pub parent_chat_key: Option<String>,
    pub agent_role: Option<AgentRole>,
    pub entries_source: EntriesSource,
    #[serde(default)]
    pub entries: Vec<BundleEntry>,
    #[serde(default)]
    pub pull_requests: Vec<String>,
    /// The file or row the chat was read from; provenance and identity key off it.
    pub source_ref: String,
    /// A short, safe label for the report (never raw content).
    pub source_label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum BundleEntryKind {
    #[serde(rename = "user.message")]
    UserMessage,
    #[serde(rename = "assistant.message")]
    AssistantMessage,
    #[serde(rename = "reasoning")]
    Reasoning,
    /// A tool call with its result, merged into one row.
    #[serde(rename = "tool.call")]
    ToolCall,
    /// The harness compacted the conversation here; `text` is its summary.
    #[serde(rename = "compaction.boundary")]
    CompactionBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub source_id: String,
    pub at: String,
    pub kind: BundleEntryKind,
    pub text: Option<String>,
    pub tool: Option<BundleTool>,
    #[serde(default)]
    pub attachments: Vec<BundleAttachment>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Completed,
    Failed,
    /// The call has no recorded result (the session ended mid-call).
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleTool {
    pub call_id: String,
    pub name: String,
    #[serde(default)]
    pub input: Value,
    pub output: Option<String>,
    pub status: ToolStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleAttachment {
    pub media_type: String,
    pub name: Option<String>,
    /// Base64 bytes for images small enough to keep inline.
    pub data_base64: Option<String>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleMemory {
    pub key: String,
    /// `global` or `project`.
    pub scope: String,
    pub project_key: Option<String>,
    pub kind: String,
    pub body: String,
    pub source_ref: String,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleSetup {
    pub key: String,
    pub kind: CandidateKind,
    /// `global` or `project`.
    pub scope: String,
    pub project_key: Option<String>,
    pub name: String,
    pub body: Option<String>,
    pub config: Option<Value>,
    pub source_ref: String,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreference {
    pub key: String,
    pub name: String,
    pub value: Value,
    pub bridge_setting: Option<String>,
    pub source_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleExclusion {
    pub category: String,
    pub source_label: String,
    pub count: u32,
    pub reason: String,
}

/// One chat after merging, with every source that pointed at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChat {
    /// `harness:sessionId` when the chat has a native session, else `tool:key`.
    pub identity: String,
    pub chat: BundleChat,
    pub primary: BundleSource,
    /// Every `(tool, source_ref)` that contributed, primary first.
    pub contributors: Vec<(String, String)>,
    pub parent_identity: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedImport {
    pub sources: Vec<BundleSource>,
    pub projects: Vec<BundleProject>,
    pub workspaces: Vec<BundleWorkspace>,
    /// Parents always come before their children.
    pub chats: Vec<ResolvedChat>,
    pub memories: Vec<(BundleSource, BundleMemory)>,
    pub setup: Vec<(BundleSource, BundleSetup)>,
    pub preferences: Vec<(BundleSource, BundlePreference)>,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub excluded: Vec<BundleExclusion>,
}

/// Lexical normalization for paths that may no longer exist: resolves `.`
/// and `..` and trailing slashes, and uses the real path when it exists.
pub fn normalize_path(raw: &str) -> String {
    let path = Path::new(raw);
    if let Ok(real) = path.canonicalize() {
        return real.to_string_lossy().into_owned();
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    let text = out.to_string_lossy().into_owned();
    if text.is_empty() {
        raw.to_owned()
    } else {
        text
    }
}

fn info(code: &str, message: String) -> ImportDiagnostic {
    ImportDiagnostic {
        code: code.into(),
        severity: "info".into(),
        classification: SourceClassification::Documented,
        message,
        recovery: None,
        source_label: None,
    }
}

fn entry_count_for_merge(chat: &BundleChat) -> (u8, usize) {
    let rank = match chat.entries_source {
        EntriesSource::Native if !chat.entries.is_empty() => 2,
        EntriesSource::ToolCopy if !chat.entries.is_empty() => 1,
        _ => 0,
    };
    (rank, chat.entries.len())
}

/// Merge `incoming` into `existing`. Entries are never interleaved: the whole
/// list comes from one source, native over a tool's copy, longer on a tie.
fn merge_chat(existing: &mut ResolvedChat, incoming: BundleChat, incoming_source: &BundleSource) {
    existing
        .contributors
        .push((incoming_source.tool.clone(), incoming.source_ref.clone()));
    let take_entries = entry_count_for_merge(&incoming) > entry_count_for_merge(&existing.chat);
    let chat = &mut existing.chat;
    if take_entries {
        chat.entries = incoming.entries.clone();
        chat.entries_source = incoming.entries_source;
    }
    let incoming_title = incoming.title.as_ref().filter(|title| !title.trim().is_empty());
    if let Some(title) = incoming_title {
        let existing_has_title = chat.title.as_ref().is_some_and(|t| !t.trim().is_empty());
        if !existing_has_title || incoming.title_source < chat.title_source {
            chat.title = Some(title.clone());
            chat.title_source = incoming.title_source;
        }
    }
    chat.created_at = match (chat.created_at.take(), incoming.created_at) {
        (Some(a), Some(b)) => Some(if b < a { b } else { a }),
        (a, b) => a.or(b),
    };
    chat.updated_at = match (chat.updated_at.take(), incoming.updated_at) {
        (Some(a), Some(b)) => Some(if b > a { b } else { a }),
        (a, b) => a.or(b),
    };
    chat.archived |= incoming.archived;
    chat.pinned |= incoming.pinned;
    if chat.cwd.is_none() {
        chat.cwd = incoming.cwd;
    }
    if chat.workspace_key.is_none() {
        chat.workspace_key = incoming.workspace_key;
    }
    if chat.model.is_none() {
        chat.model = incoming.model;
    }
    if chat.effort.is_none() {
        chat.effort = incoming.effort;
    }
    if chat.agent_role.is_none() {
        chat.agent_role = incoming.agent_role;
    }
    for pr in incoming.pull_requests {
        if !chat.pull_requests.contains(&pr) {
            chat.pull_requests.push(pr);
        }
    }
}

/// Merge every bundle into one import. Fails closed on an unknown bundle version.
pub fn resolve(bundles: Vec<TransferBundle>) -> Result<ResolvedImport, BridgeError> {
    let mut resolver = Resolver::default();
    for bundle in bundles {
        resolver.add(bundle)?;
    }
    Ok(resolver.finish())
}

/// The state `resolve` keeps while it folds bundles together.
#[derive(Default)]
struct Resolver {
    out: ResolvedImport,
    chat_index: HashMap<String, usize>,
    /// (tool, bundle chat key) -> identity, for parent links.
    local_identity: HashMap<(String, String), String>,
    /// (tool, child identity, parent key in that tool's bundle).
    pending_parent: Vec<(String, String, String)>,
    workspace_by_path: HashMap<String, usize>,
    workspace_key_map: HashMap<(String, String), String>,
    project_by_root: HashMap<String, usize>,
    project_key_map: HashMap<(String, String), String>,
    memory_seen: HashSet<(String, Option<String>, String)>,
    setup_seen: HashSet<(String, String, String)>,
    preference_seen: HashSet<String>,
}

impl Resolver {
    fn add(&mut self, bundle: TransferBundle) -> Result<(), BridgeError> {
        if bundle.bundle_version != BUNDLE_VERSION {
            return Err(BridgeError::Invalid(format!(
                "Import bundle version {} is not supported by this Bridge (expects {BUNDLE_VERSION}); update Bridge and try again",
                bundle.bundle_version
            )));
        }
        let tool = bundle.source.tool.clone();
        if tool.trim().is_empty() {
            return Err(BridgeError::Invalid("Import bundle has no source tool".into()));
        }
        self.out.sources.push(bundle.source.clone());
        self.out.diagnostics.extend(bundle.diagnostics);
        for exclusion in bundle.excluded {
            push_exclusion(
                &mut self.out.excluded,
                &exclusion.category,
                &exclusion.source_label,
                exclusion.count,
                &exclusion.reason,
            );
        }
        for project in bundle.projects {
            self.add_project(&tool, project);
        }
        for workspace in bundle.workspaces {
            self.add_workspace(&tool, workspace);
        }
        for chat in bundle.chats {
            self.add_chat(&tool, &bundle.source, chat);
        }
        self.add_items(&tool, &bundle.source, bundle.memories, bundle.setup, bundle.preferences);
        Ok(())
    }

    fn add_project(&mut self, tool: &str, project: BundleProject) {
        let root = normalize_path(&project.repo_root);
        let merged_key = match self.project_by_root.get(&root) {
            Some(index) => self.out.projects[*index].key.clone(),
            None => {
                let key = format!("project:{root}");
                self.project_by_root.insert(root.clone(), self.out.projects.len());
                self.out.projects.push(BundleProject {
                    key: key.clone(),
                    repo_root: root,
                    ..project.clone()
                });
                key
            }
        };
        self.project_key_map.insert((tool.to_owned(), project.key), merged_key);
    }

    fn add_workspace(&mut self, tool: &str, workspace: BundleWorkspace) {
        let path = normalize_path(&workspace.path);
        let project_key = workspace
            .project_key
            .as_ref()
            .and_then(|key| self.project_key_map.get(&(tool.to_owned(), key.clone())).cloned());
        let merged_key = match self.workspace_by_path.get(&path) {
            Some(index) => {
                let existing = &mut self.out.workspaces[*index];
                if existing.title.is_none() {
                    existing.title = workspace.title.clone();
                }
                if existing.branch.is_none() {
                    existing.branch = workspace.branch.clone();
                }
                if existing.project_key.is_none() {
                    existing.project_key = project_key.clone();
                }
                existing.key.clone()
            }
            None => {
                let key = format!("workspace:{path}");
                self.workspace_by_path.insert(path.clone(), self.out.workspaces.len());
                self.out.workspaces.push(BundleWorkspace {
                    key: key.clone(),
                    project_key,
                    path,
                    ..workspace.clone()
                });
                key
            }
        };
        self.workspace_key_map.insert((tool.to_owned(), workspace.key), merged_key);
    }

    fn add_chat(&mut self, tool: &str, source: &BundleSource, mut chat: BundleChat) {
        if chat.key.trim().is_empty() {
            self.out.diagnostics.push(info(
                "chat_without_key",
                format!("A chat from {tool} had no key and was skipped"),
            ));
            return;
        }
        chat.workspace_key = chat
            .workspace_key
            .as_ref()
            .and_then(|key| self.workspace_key_map.get(&(tool.to_owned(), key.clone())).cloned());
        if let Some(cwd) = chat.cwd.as_deref() {
            chat.cwd = Some(normalize_path(cwd));
        }
        let identity = chat
            .native_ref
            .as_ref()
            .filter(|native| !native.session_id.trim().is_empty() && !native.harness.trim().is_empty())
            .map(NativeRef::key)
            .unwrap_or_else(|| format!("{tool}:{}", chat.key));
        if self
            .local_identity
            .insert((tool.to_owned(), chat.key.clone()), identity.clone())
            .is_some()
        {
            self.out.diagnostics.push(info(
                "duplicate_chat_key",
                format!("Two chats from {tool} shared one key; the later one was merged"),
            ));
        }
        if let Some(parent) = chat.parent_chat_key.clone() {
            self.pending_parent.push((tool.to_owned(), identity.clone(), parent));
        }
        match self.chat_index.get(&identity) {
            Some(index) => merge_chat(&mut self.out.chats[*index], chat, source),
            None => {
                self.chat_index.insert(identity.clone(), self.out.chats.len());
                self.out.chats.push(ResolvedChat {
                    identity,
                    contributors: vec![(tool.to_owned(), chat.source_ref.clone())],
                    primary: source.clone(),
                    parent_identity: None,
                    chat,
                });
            }
        }
    }

    fn add_items(
        &mut self,
        tool: &str,
        source: &BundleSource,
        memories: Vec<BundleMemory>,
        setup: Vec<BundleSetup>,
        preferences: Vec<BundlePreference>,
    ) {
        for mut memory in memories {
            let key = (
                memory.scope.clone(),
                memory.project_key.clone(),
                crate::external_import::sha256_hex(memory.body.trim().as_bytes()),
            );
            if self.memory_seen.insert(key) {
                memory.project_key = memory
                    .project_key
                    .and_then(|key| self.project_key_map.get(&(tool.to_owned(), key)).cloned());
                self.out.memories.push((source.clone(), memory));
            }
        }
        for item in setup {
            let body = item
                .body
                .clone()
                .or_else(|| item.config.as_ref().map(Value::to_string))
                .unwrap_or_default();
            let key = (
                item.kind.as_str().to_owned(),
                item.name.clone(),
                crate::external_import::sha256_hex(body.as_bytes()),
            );
            if self.setup_seen.insert(key) {
                self.out.setup.push((source.clone(), item));
            }
        }
        for preference in preferences {
            if self
                .preference_seen
                .insert(format!("{}={}", preference.name, preference.value))
            {
                self.out.preferences.push((source.clone(), preference));
            }
        }
    }

    fn finish(mut self) -> ResolvedImport {
        for (tool, child_identity, parent_key) in std::mem::take(&mut self.pending_parent) {
            let Some(parent_identity) = self.local_identity.get(&(tool.clone(), parent_key)).cloned()
            else {
                self.out.diagnostics.push(info(
                    "parent_chat_missing",
                    format!("A {tool} chat named a parent that was not in the import; it was kept as a top-level chat"),
                ));
                continue;
            };
            if parent_identity == child_identity {
                continue;
            }
            if let Some(index) = self.chat_index.get(&child_identity) {
                if self.out.chats[*index].parent_identity.is_none() {
                    self.out.chats[*index].parent_identity = Some(parent_identity);
                }
            }
        }
        break_parent_cycles(&mut self.out);
        order_parents_first(&mut self.out.chats);
        self.out
    }
}

fn break_parent_cycles(out: &mut ResolvedImport) {
    let index: HashMap<String, usize> = out
        .chats
        .iter()
        .enumerate()
        .map(|(position, chat)| (chat.identity.clone(), position))
        .collect();
    for start in 0..out.chats.len() {
        let mut seen = HashSet::new();
        let mut cursor = Some(start);
        while let Some(position) = cursor {
            if !seen.insert(position) {
                out.chats[start].parent_identity = None;
                out.diagnostics.push(info(
                    "parent_chat_cycle",
                    "Chats named each other as parents; one was kept as a top-level chat".into(),
                ));
                break;
            }
            cursor = out.chats[position]
                .parent_identity
                .as_ref()
                .and_then(|parent| index.get(parent).copied());
        }
    }
}

fn order_parents_first(chats: &mut Vec<ResolvedChat>) {
    let identities: HashSet<String> = chats.iter().map(|chat| chat.identity.clone()).collect();
    let mut placed: HashSet<String> = HashSet::new();
    let mut ordered = Vec::with_capacity(chats.len());
    let mut remaining: Vec<ResolvedChat> = std::mem::take(chats);
    while !remaining.is_empty() {
        let before = remaining.len();
        let (ready, rest): (Vec<_>, Vec<_>) = remaining.into_iter().partition(|chat| {
            chat.parent_identity
                .as_ref()
                .map(|parent| !identities.contains(parent) || placed.contains(parent))
                .unwrap_or(true)
        });
        for chat in ready {
            placed.insert(chat.identity.clone());
            ordered.push(chat);
        }
        remaining = rest;
        if remaining.len() == before {
            // Unreachable after cycle breaking; keep everything rather than loop.
            ordered.append(&mut remaining);
        }
    }
    *chats = ordered;
}

/// Human name for a source tool, for badges and the report.
pub fn tool_label(tool: &str) -> &str {
    match tool {
        "claude_code" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "conductor" => "Conductor",
        "t3code" => "T3 Code",
        "orca" => "Orca",
        "superset" => "Superset",
        "cursor" => "Cursor",
        other => other,
    }
}

fn truncate_chars(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_owned(), false);
    }
    let kept: String = text.chars().take(limit).collect();
    (kept, true)
}

/// Lower one bundle entry to the message shape `external_import` stores.
/// Returns `None` (and counts it) when the entry has nothing to show.
fn lower_entry(
    entry: &BundleEntry,
    sequence: usize,
    excluded: &mut BTreeMap<&'static str, u32>,
) -> Option<Value> {
    match entry.kind {
        BundleEntryKind::UserMessage => lower_user(entry, sequence, excluded),
        BundleEntryKind::AssistantMessage | BundleEntryKind::Reasoning => {
            lower_text(entry, sequence, excluded)
        }
        BundleEntryKind::ToolCall => lower_tool(entry, sequence, excluded),
        BundleEntryKind::CompactionBoundary => Some(lower_compaction(entry, sequence)),
    }
}

fn lower_user(
    entry: &BundleEntry,
    sequence: usize,
    excluded: &mut BTreeMap<&'static str, u32>,
) -> Option<Value> {
    let mut attachments = Vec::new();
    for attachment in &entry.attachments {
        match attachment.data_base64.as_deref() {
            Some(data)
                if attachment.media_type.starts_with("image/")
                    && data.len() <= MAX_INLINE_ATTACHMENT_BYTES =>
            {
                attachments.push(json!({
                    "mediaType": attachment.media_type,
                    "dataUri": format!("data:{};base64,{data}", attachment.media_type),
                }));
            }
            _ => *excluded.entry("attachment").or_insert(0) += 1,
        }
    }
    let text = entry.text.clone().unwrap_or_default();
    let text = if text.trim().is_empty() {
        if attachments.is_empty() {
            *excluded.entry("empty_message").or_insert(0) += 1;
            return None;
        }
        "Attached image".to_owned()
    } else {
        text
    };
    let mut message = json!({
        "sourceId": entry.source_id,
        "role": "user",
        "kind": "user.message",
        "text": text,
        "timestamp": entry.at,
        "sequence": sequence,
    });
    if !attachments.is_empty() {
        message["data"] = json!({ "attachments": attachments });
    }
    Some(message)
}

fn lower_text(
    entry: &BundleEntry,
    sequence: usize,
    excluded: &mut BTreeMap<&'static str, u32>,
) -> Option<Value> {
    let text = entry.text.clone().unwrap_or_default();
    if text.trim().is_empty() {
        *excluded.entry("empty_message").or_insert(0) += 1;
        return None;
    }
    let kind = if entry.kind == BundleEntryKind::Reasoning {
        "reasoning"
    } else {
        "assistant.message"
    };
    Some(json!({
        "sourceId": entry.source_id,
        "role": "assistant",
        "kind": kind,
        "text": text,
        "timestamp": entry.at,
        "sequence": sequence,
    }))
}

fn lower_tool(
    entry: &BundleEntry,
    sequence: usize,
    excluded: &mut BTreeMap<&'static str, u32>,
) -> Option<Value> {
    let Some(tool) = entry.tool.as_ref() else {
        *excluded.entry("tool_without_call").or_insert(0) += 1;
        return None;
    };
    let call_id = if tool.call_id.trim().is_empty() {
        entry.source_id.clone()
    } else {
        tool.call_id.clone()
    };
    let name = if tool.name.trim().is_empty() {
        "tool".to_owned()
    } else {
        tool.name.clone()
    };
    let mut data = json!({
        "type": "tool_use",
        "name": name,
        "input": tool.input,
    });
    if let Some(output) = tool.output.as_deref() {
        let (kept, cut) = truncate_chars(output, MAX_TOOL_OUTPUT_CHARS);
        data["aggregatedOutput"] = Value::String(if cut {
            format!("{kept}\n\n[output cut at {MAX_TOOL_OUTPUT_CHARS} characters on import]")
        } else {
            kept
        });
    }
    Some(json!({
        "sourceId": entry.source_id,
        "role": "assistant",
        "kind": "tool.completed",
        "itemId": call_id,
        "title": name,
        "status": match tool.status {
            ToolStatus::Completed => "completed",
            ToolStatus::Failed => "failed",
            ToolStatus::Unknown => "interrupted",
        },
        "data": data,
        "timestamp": entry.at,
        "sequence": sequence,
    }))
}

fn lower_compaction(entry: &BundleEntry, sequence: usize) -> Value {
    let text = entry.text.clone().unwrap_or_default();
    json!({
        "sourceId": entry.source_id,
        "role": "system",
        "kind": "compaction",
        "schemaVersion": 1,
        "summary": if text.trim().is_empty() { "Conversation compacted".to_owned() } else { text },
        "timestamp": entry.at,
        "sequence": sequence,
    })
}

fn derived_title(chat: &BundleChat) -> String {
    chat.entries
        .iter()
        .find(|entry| entry.kind == BundleEntryKind::UserMessage)
        .and_then(|entry| entry.text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| {
            text.lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(80)
                .collect::<String>()
        })
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Imported chat".into())
}

fn import_source(source: &BundleSource, source_ref: &str) -> ImportSource {
    ImportSource {
        provider: source.tool.clone(),
        adapter_version: source.adapter_version.clone(),
        source_version: source.tool_version.clone(),
        schema_version: source.schema_version.clone(),
        canonical_source_ref: source_ref.to_owned(),
        source_path_fingerprint: path_fingerprint(Path::new(source_ref)),
        discovered_at: source.detected_at.clone(),
        source_metadata: json!({ "roots": source.roots }),
    }
}

fn build_candidate(
    source: ImportSource,
    kind: CandidateKind,
    title: String,
    native_id: Option<String>,
    payload: Value,
    stability: Stability,
    selected_by_default: bool,
    diagnostics: Vec<ImportDiagnostic>,
) -> ImportCandidate {
    let (safe_payload, redaction_summary) = sanitize_import_payload(&payload);
    let hash = content_hash(&safe_payload);
    let candidate_id = deterministic_identity(
        &source.provider,
        &source.canonical_source_ref,
        native_id.as_deref(),
        &hash,
        &kind,
    );
    let (safe_title, _) = sanitize_import_payload(&Value::String(title));
    ImportCandidate {
        candidate_id,
        source,
        source_native_id: native_id,
        kind,
        title: safe_title.as_str().unwrap_or("Imported item").to_owned(),
        created_at: None,
        updated_at: None,
        project_hint: None,
        content_hash: hash,
        stability: stability.clone(),
        confidence_bps: if matches!(stability, Stability::Stable) { 10_000 } else { 8_000 },
        selected_by_default,
        redaction_summary,
        diagnostics,
        normalized_payload: safe_payload,
    }
}

/// What lowering produced: candidates for the existing pipeline, plus what did
/// not come over, for the report.
#[derive(Debug, Clone, Default)]
pub struct LoweredImport {
    pub candidates: Vec<ImportCandidate>,
    pub excluded: Vec<BundleExclusion>,
    pub diagnostics: Vec<ImportDiagnostic>,
}

/// Turn a resolved import into candidates. Chats come out parents first, and a
/// child carries its parent's candidate id so the writer can link them.
/// A chat's entries as stored messages, numbered from 1, with anything left
/// out counted against the chat's label.
fn lower_messages(chat: &BundleChat, out: &mut LoweredImport) -> Vec<Value> {
    let mut excluded: BTreeMap<&'static str, u32> = BTreeMap::new();
    let messages: Vec<Value> = chat
        .entries
        .iter()
        .filter_map(|entry| lower_entry(entry, 0, &mut excluded))
        .enumerate()
        .map(|(index, mut message)| {
            message["sequence"] = json!(index + 1);
            message
        })
        .collect();
    for (category, count) in &excluded {
        let reason = match *category {
            "attachment" => "image too large or not an image; the message text was kept",
            "empty_message" => "the message had no text",
            _ => "the entry had no content Bridge can show",
        };
        push_exclusion(&mut out.excluded, category, &chat.source_label, *count, reason);
    }
    messages
}

/// Lower one resolved chat into a conversation candidate, or count it as
/// excluded when nothing in it can be shown.
fn lower_chat(
    resolved_chat: &ResolvedChat,
    workspace_by_key: &HashMap<&str, &BundleWorkspace>,
    candidate_by_identity: &mut HashMap<String, String>,
    out: &mut LoweredImport,
) {
    let chat = &resolved_chat.chat;
    let messages = lower_messages(chat, out);
    if messages.is_empty() {
        push_exclusion(
            &mut out.excluded,
            "chat",
            &chat.source_label,
            1,
            match chat.entries_source {
                EntriesSource::None => "no transcript was found for this chat",
                _ => "the chat had no messages Bridge can show",
            },
        );
        return;
    }
    let title_source = if chat.title.as_ref().is_some_and(|t| !t.trim().is_empty()) {
        chat.title_source
    } else {
        TitleSource::Derived
    };
    let title = chat
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| derived_title(chat));
    let workspace_path = chat
        .workspace_key
        .as_deref()
        .and_then(|key| workspace_by_key.get(key))
        .map(|workspace| workspace.path.clone());
    let parent_candidate = resolved_chat
        .parent_identity
        .as_ref()
        .and_then(|parent| candidate_by_identity.get(parent).cloned());
    // What the conversation says, independent of which tool delivered it:
    // the same session through Conductor and through Claude Code compares equal.
    let transcript_hash = content_hash(&Value::Array(messages.clone()));
    let payload = json!({
        "messages": messages,
        "transcriptHash": transcript_hash,
        "projectHint": chat.cwd,
        "historical": true,
        "resumable": false,
        "harness": chat.harness,
        "sourceTool": resolved_chat.primary.tool,
        "sourceBadge": format!("Imported from {}", tool_label(&resolved_chat.primary.tool)),
        "nativeRef": chat.native_ref,
        "nativeKey": chat.native_ref.as_ref().map(NativeRef::key),
        "cwd": chat.cwd,
        "workspacePath": workspace_path,
        "titleSource": title_source,
        "entriesSource": chat.entries_source,
        "archived": chat.archived,
        "pinned": chat.pinned,
        "model": chat.model,
        "effort": chat.effort,
        "agentRole": chat.agent_role,
        "parentCandidateId": parent_candidate,
        "pullRequests": chat.pull_requests,
        "alsoFrom": resolved_chat.contributors.iter().skip(1).map(|(tool, source_ref)| json!({
            "tool": tool,
            "sourceRef": source_ref,
        })).collect::<Vec<_>>(),
    });
    let native_id = Some(
        chat.native_ref
            .as_ref()
            .map(|native| native.session_id.clone())
            .unwrap_or_else(|| chat.key.clone()),
    );
    let mut candidate = build_candidate(
        import_source(&resolved_chat.primary, &chat.source_ref),
        CandidateKind::Conversation,
        title,
        native_id,
        payload,
        Stability::VersionGated,
        false,
        Vec::new(),
    );
    candidate.created_at = chat.created_at.clone();
    candidate.updated_at = chat.updated_at.clone();
    // The payload was redacted above; the hint must be the same redacted value.
    candidate.project_hint = candidate.normalized_payload["cwd"].as_str().map(str::to_owned);
    candidate_by_identity.insert(resolved_chat.identity.clone(), candidate.candidate_id.clone());
    out.candidates.push(candidate);
}

/// Lower memories, setup and preferences: each becomes one inert candidate.
fn lower_items(resolved: &ResolvedImport, out: &mut LoweredImport) {
    for (source, memory) in &resolved.memories {
        if memory.body.trim().is_empty() {
            push_exclusion(&mut out.excluded, "memory", &memory.source_label, 1, "the memory was empty");
            continue;
        }
        let payload = json!({
            "body": memory.body,
            "memoryType": memory.kind,
            "scope": memory.scope,
            "sourceTool": source.tool,
        });
        out.candidates.push(build_candidate(
            import_source(source, &memory.source_ref),
            CandidateKind::Memory,
            memory.source_label.clone(),
            Some(memory.key.clone()),
            payload,
            Stability::VersionGated,
            false,
            Vec::new(),
        ));
    }
    for (source, setup) in &resolved.setup {
        if !setup.kind.is_setup() {
            push_exclusion(&mut out.excluded, "setup", &setup.source_label, 1, "not a setup kind Bridge stores");
            continue;
        }
        let payload = json!({
            "name": setup.name,
            "scope": setup.scope,
            "body": setup.body,
            "config": setup.config,
            "sourceTool": source.tool,
            "inert": true,
        });
        out.candidates.push(build_candidate(
            import_source(source, &setup.source_ref),
            setup.kind.clone(),
            setup.name.clone(),
            Some(setup.key.clone()),
            payload,
            Stability::Stable,
            false,
            Vec::new(),
        ));
    }
    for (source, preference) in &resolved.preferences {
        let payload = json!({
            "name": preference.name,
            "value": preference.value,
            "bridgeSetting": preference.bridge_setting,
            "sourceTool": source.tool,
            "inert": true,
        });
        out.candidates.push(build_candidate(
            import_source(source, &preference.source_ref),
            CandidateKind::Preference,
            format!("{}: {}", tool_label(&source.tool), preference.name),
            Some(preference.key.clone()),
            payload,
            Stability::Stable,
            false,
            Vec::new(),
        ));
    }
}

pub fn lower(resolved: &ResolvedImport) -> LoweredImport {
    let mut out = LoweredImport {
        excluded: resolved.excluded.clone(),
        diagnostics: resolved.diagnostics.clone(),
        ..LoweredImport::default()
    };
    let mut candidate_by_identity: HashMap<String, String> = HashMap::new();
    let workspace_by_key: HashMap<&str, &BundleWorkspace> = resolved
        .workspaces
        .iter()
        .map(|workspace| (workspace.key.as_str(), workspace))
        .collect();
    for resolved_chat in &resolved.chats {
        lower_chat(resolved_chat, &workspace_by_key, &mut candidate_by_identity, &mut out);
    }
    lower_items(resolved, &mut out);
    out
}

fn push_exclusion(
    list: &mut Vec<BundleExclusion>,
    category: &str,
    source_label: &str,
    count: u32,
    reason: &str,
) {
    if count == 0 {
        return;
    }
    if let Some(existing) = list.iter_mut().find(|item| {
        item.category == category && item.source_label == source_label && item.reason == reason
    }) {
        existing.count += count;
        return;
    }
    list.push(BundleExclusion {
        category: category.into(),
        source_label: source_label.into(),
        count,
        reason: reason.into(),
    });
}

#[cfg(test)]
pub(crate) mod tests;

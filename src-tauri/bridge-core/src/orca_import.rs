//! Orca into Bridge (#1059): its worktrees as Bridge workspaces adopted in
//! place ([`crate::import_worktrees`]), its chats, and its multi-agent runs as
//! orchestrator chats with their workers under them.
//!
//! Orca runs Claude Code, Codex, OpenCode and other agents. Each chat records
//! the session it ran as ([`store::NativeSession`]). When that session's file
//! is in Bridge's folders for the agent (Claude Code's include the account
//! folder Orca launched it with), the chat is read by the agent's own reader,
//! so it is the same chat the agent's import makes. When it is gone, the chat
//! is Orca's own copy, keeping the native identity.
//!
//! Built from Orca's source (`stablyai/orca@1f4e0b30`) and its released
//! 1.4.271, run headless in a sandbox through its own CLI; the schemas it
//! wrote are the fixtures (`testing/fixtures/import/orca/1.4.271/`). Only
//! databases are opened, read-only, and only named columns of the allowed
//! tables and named profile documents; never [`store::DENIED_FILES`].

pub mod store;

#[cfg(test)]
pub(crate) mod fixture;

use crate::external_import::{
    content_hash, deterministic_identity, source_now, validate_schema_version, CandidateKind, DiscoveredArtifact,
    DiscoveryRequest, DiscoveryResult, DiscoverySelection, ExternalHarnessImporter, FixtureFormat, FixtureManifest,
    ImportCandidate, ImportDiagnostic, NormalizedImportCandidate, RedactionSummary, SchemaGate, SourceClassification,
    Stability, ValidationResult,
};
use crate::import_bundle::{
    AgentRole, BundleChat, BundleEntry, BundleEntryKind, BundleSource, BundleTool, EntriesSource, NativeRef, TitleSource,
    ToolStatus, TransferBundle,
};
use crate::import_worktrees::Candidate;
use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use store::{Body, Chat, Item, NativeSession, Place, Store, Worker};

/// The import id the Import page and the job use.
pub const PROVIDER: &str = "orca";
pub const ADAPTER_VERSION: &str = "1";
pub const STORE_GATE: &str = "orca-sqlite";

#[derive(Debug, Default, Clone, Copy)]
pub struct OrcaImporter;

/// Where Orca keeps things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$ORCA_USER_DATA_PATH`, else `~/Library/Application Support/orca`.
    pub base: PathBuf,
}

impl Paths {
    pub fn at(base: &Path) -> Self {
        Self { base: base.to_path_buf() }
    }

    /// For a home folder, honouring `ORCA_USER_DATA_PATH` as Orca does.
    pub fn for_home(home: &Path, data_path: Option<&Path>) -> Self {
        Self::at(&data_path.map_or_else(|| home.join("Library/Application Support/orca"), Path::to_path_buf))
    }

    /// This machine's, from the environment.
    pub fn from_env(home: &Path) -> Self {
        let data_path = std::env::var_os("ORCA_USER_DATA_PATH").filter(|value| !value.is_empty()).map(PathBuf::from);
        Self::for_home(home, data_path.as_deref())
    }

    pub fn chats(&self) -> PathBuf {
        self.base.join(store::CHAT_DB)
    }

    pub fn installed(&self) -> bool {
        [store::CHAT_DB, store::OLD_CHAT_DB, store::PROFILE_INDEX].iter().any(|file| self.base.join(file).is_file())
    }

    /// Every profile: those the index lists and those with a folder.
    pub fn profiles(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        if let Some(index) = std::fs::read_to_string(self.base.join(store::PROFILE_INDEX))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        {
            for profile in index.get("profiles").and_then(Value::as_array).into_iter().flatten() {
                if let Some(id) = profile.get("id").and_then(Value::as_str).and_then(store::plain_id) {
                    out.insert(id);
                }
            }
        }
        for entry in std::fs::read_dir(self.base.join("profiles")).into_iter().flatten().flatten() {
            if let Some(id) = entry.file_name().to_str().and_then(store::plain_id) {
                if entry.path().join(store::PROFILE_DB).is_file() {
                    out.insert(id);
                }
            }
        }
        out.into_iter().collect()
    }
}

/// Read Orca's data, or say why not.
pub fn read(paths: &Paths) -> Result<(Store, PathBuf), String> {
    if !paths.installed() {
        return Err("Orca was not found on this Mac".into());
    }
    let chat_path = paths.chats();
    let (chat_version, chats) = if chat_path.is_file() {
        let conn = crate::import_sqlite::open(&chat_path).map_err(|error| format!("Bridge could not open Orca's chats: {error}"))?;
        store::read_chats(&conn)?
    } else if paths.base.join(store::OLD_CHAT_DB).is_file() {
        return Err("Orca keeps these chats in an older format. Open Orca once so it updates them, then import again".into());
    } else {
        (0, Vec::new())
    };
    let mut notes = Vec::new();
    let mut profiles = Vec::new();
    for id in paths.profiles() {
        let folder = paths.base.join("profiles").join(&id);
        let database = folder.join(store::PROFILE_DB);
        if database.is_file() {
            let conn = crate::import_sqlite::open(&database).map_err(|error| format!("Bridge could not open Orca's profile '{id}': {error}"))?;
            profiles.push(store::read_profile(&conn, &id)?);
        }
    }
    if profiles.is_empty() && paths.base.join("profiles").is_dir() {
        notes.push("worktrees: open Orca once so it updates its data, then import again".to_owned());
    }
    let orchestration = paths.base.join(store::ORCHESTRATION_DB);
    let runs = if orchestration.is_file() {
        match crate::import_sqlite::open(&orchestration).map_err(|error| error.to_string()).and_then(|conn| store::read_runs(&conn)) {
            Ok(runs) => Some(runs),
            Err(reason) => {
                notes.push(format!("links between multi-agent runs' chats: {reason}"));
                None
            }
        }
    } else {
        Some(Vec::new())
    };
    Ok((Store { chat_version, chats, profiles, runs, notes }, chat_path))
}

/// A chat Bridge can bring over: on this Mac, with an agent Bridge runs.
pub fn is_live(chat: &Chat) -> bool {
    chat.local && store::harness_of(&chat.agent).is_some()
}

pub fn chats(store: &Store) -> impl Iterator<Item = &Chat> {
    store.chats.iter().filter(|chat| is_live(chat))
}

/// Every repository, by id, across profiles.
fn repos(store: &Store) -> HashMap<&str, &store::Repo> {
    store.profiles.iter().flat_map(|profile| &profile.repos).map(|repo| (repo.id.as_str(), repo)).collect()
}

/// The folder a chat ran in: its worktree, its folder workspace's folder,
/// else where Orca launched it.
pub fn folder(store: &Store, chat: &Chat) -> Option<PathBuf> {
    match &chat.place {
        Place::Worktree { path, .. } => Some(path.clone()),
        Place::Folder(id) => store.profiles.iter().find_map(|profile| profile.folders.get(id).cloned()),
        Place::None => None,
    }
    .or_else(|| chat.launch_directory.clone())
    .filter(|path| path.is_absolute())
}

/// Archived in Orca: the chat's worktree is.
pub fn is_archived(store: &Store, chat: &Chat) -> bool {
    let Place::Worktree { path, .. } = &chat.place else { return false };
    store.profiles.iter().flat_map(|profile| &profile.worktrees).any(|worktree| &worktree.path == path && worktree.archived)
}

/// Setup and archive scripts from the repository's `orca.yaml`.
fn scripts_of(repo: Option<&&store::Repo>) -> Option<crate::import_worktrees::Scripts> {
    let raw = std::fs::read_to_string(repo?.path.join("orca.yaml")).ok()?;
    (raw.len() <= 256 * 1024).then(|| store::yaml_scripts(&raw)).flatten()
}

/// Every worktree a profile lists or a live chat ran in, once each, named
/// by Orca's display name. Main checkouts and SSH repositories' worktrees
/// are left out; a worktree is archived when Orca archived it.
pub fn worktree_candidates(store: &Store) -> Vec<Candidate> {
    let repos = repos(store);
    let mut by_path: BTreeMap<PathBuf, (String, Option<String>, bool, String)> = BTreeMap::new();
    let mut add = |repo: &str, path: &Path, name: Option<String>, archived: bool, source: String| {
        let repo_entry = repos.get(repo);
        if repo_entry.is_some_and(|known| known.remote || known.path == path) {
            return;
        }
        let entry = by_path.entry(path.to_path_buf()).or_insert_with(|| (repo.to_owned(), None, true, source));
        if entry.1.is_none() {
            entry.1 = name.filter(|name| !name.trim().is_empty());
        }
        entry.2 &= archived;
    };
    for worktree in store.profiles.iter().flat_map(|profile| &profile.worktrees) {
        add(&worktree.repo, &worktree.path, worktree.name.clone(), worktree.archived, format!("orca:{}", worktree.id));
    }
    for chat in chats(store) {
        if let Place::Worktree { repo, path } = &chat.place {
            let archived = is_archived(store, chat);
            add(repo, path, None, archived, format!("orca:{repo}::{}", path.to_string_lossy()));
        }
    }
    by_path
        .into_iter()
        .map(|(path, (repo, title, archived, source_ref))| Candidate {
            tool: PROVIDER,
            name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            title,
            archived,
            source_ref,
            scripts: scripts_of(repos.get(repo.as_str())),
            path,
        })
        .collect()
}

/// What detection shows for Orca.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub chats: u64,
    pub projects: u64,
    pub workspaces: u64,
}

pub fn counts(store: &Store) -> Counts {
    let projects: HashSet<&Path> = store
        .profiles
        .iter()
        .flat_map(|profile| &profile.repos)
        .filter(|repo| !repo.remote)
        .map(|repo| repo.path.as_path())
        .collect();
    let repos = repos(store);
    let workspaces = store
        .profiles
        .iter()
        .flat_map(|profile| &profile.worktrees)
        .filter(|worktree| !worktree.archived)
        .filter(|worktree| repos.get(worktree.repo.as_str()).is_none_or(|repo| !repo.remote && repo.path != worktree.path))
        .map(|worktree| worktree.path.as_path())
        .collect::<HashSet<_>>()
        .len();
    Counts { chats: chats(store).count() as u64, projects: projects.len() as u64, workspaces: workspaces as u64 }
}

/// How each live chat comes over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatPlan {
    /// Read by the agent's own importer, from these files.
    pub native: BTreeSet<NativeSession>,
    /// Orca's own copy, by Orca session id.
    pub copies: Vec<String>,
}

/// Decide, for every live chat, between its native session (when the file
/// was found) and Orca's copy.
pub fn plan_chats(store: &Store, found: &HashSet<NativeSession>) -> ChatPlan {
    let mut plan = ChatPlan::default();
    for chat in chats(store) {
        match chat.native.as_ref().filter(|native| found.contains(*native)) {
            Some(native) => {
                plan.native.insert(native.clone());
            }
            None => plan.copies.push(chat.id.clone()),
        }
    }
    plan
}

/// The account folder Orca launched a chat's agent with, as the folder that
/// agent's import reads (`<CLAUDE_CONFIG_DIR>`, `<HOME>/.claude`, ...).
pub fn account_root(chat: &Chat) -> Option<(&'static str, PathBuf)> {
    let home = chat.account_home.as_ref()?;
    let harness = store::harness_of(&chat.agent)?;
    let root = match (harness, home.variable.as_str()) {
        ("claude", "CLAUDE_CONFIG_DIR") | ("codex", "CODEX_HOME") => home.path.clone(),
        ("claude", "HOME") => home.path.join(".claude"),
        ("codex", "HOME") => home.path.join(".codex"),
        _ => return None,
    };
    root.is_absolute().then_some((harness, root))
}

/// The chat that leads each run's workers, by worker: an Orca chat id, or
/// a native session's key for a terminal worker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// Worker chat id to its run's coordinator chat id.
    pub chats: BTreeMap<String, String>,
    /// Worker native key to its run's coordinator chat id.
    pub sessions: BTreeMap<String, String>,
    /// Coordinators, by chat id.
    pub coordinators: BTreeSet<String>,
    /// Workers left top-level: a terminal led their run, or Orca captured
    /// no session for them.
    pub unlinked: usize,
}

pub fn links(store: &Store) -> Links {
    let mut out = Links::default();
    let live: HashSet<&str> = chats(store).map(|chat| chat.id.as_str()).collect();
    for run in store.runs.iter().flatten() {
        let coordinator = run.coordinator.as_deref().filter(|id| live.contains(id));
        for worker in &run.workers {
            match (coordinator, worker) {
                (Some(lead), Worker::Chat(chat)) if chat != lead => {
                    out.chats.entry(chat.clone()).or_insert_with(|| lead.to_owned());
                    out.coordinators.insert(lead.to_owned());
                }
                (Some(lead), Worker::Session(native)) => {
                    out.sessions.entry(native.key()).or_insert_with(|| lead.to_owned());
                    out.coordinators.insert(lead.to_owned());
                }
                (_, Worker::Chat(chat)) if coordinator.is_some_and(|lead| lead == chat) => {}
                _ => out.unlinked += 1,
            }
        }
    }
    out
}

/// `<chat database>#<session id>`, the source of an Orca copy.
pub fn source_ref(store: &Path, chat: &str) -> String {
    format!("{}#{chat}", store.to_string_lossy())
}

pub fn parse_source_ref(source_ref: &str) -> Option<(PathBuf, String)> {
    let (store, chat) = source_ref.rsplit_once('#')?;
    (!store.is_empty() && !chat.is_empty()).then(|| (PathBuf::from(store), chat.to_owned()))
}

fn supported_format_versions() -> BTreeMap<String, String> {
    BTreeMap::from([(STORE_GATE.to_owned(), STORE_GATE.to_owned())])
}

/// The job's work item for one copied chat.
pub fn copy_artifact(store_path: &Path, store: &Store, chat: &Chat) -> DiscoveredArtifact {
    let reference = source_ref(store_path, &chat.id);
    let label = format!("Orca/{}", chat.id);
    DiscoveredArtifact {
        artifact_id: deterministic_identity(PROVIDER, &reference, Some(&label), chat.updated_at.as_deref().unwrap_or_default(), &CandidateKind::Conversation),
        canonical_source_ref: reference,
        source_label: label,
        kind: CandidateKind::Conversation,
        classification: SourceClassification::VersionGatedPrivate,
        stability: Stability::VersionGated,
        estimated_bytes: 4096,
        modified_at: chat.updated_at.clone(),
        required_schema_gate: Some(STORE_GATE.into()),
        title: chat.title.clone(),
        project_path: folder(store, chat).map(|path| path.to_string_lossy().into_owned()),
        started_at: chat.created_at.clone(),
    }
}

/// A discovery holding one batch of copies, for preview and commit.
pub fn job_discovery(roots: &[PathBuf], artifacts: Vec<DiscoveredArtifact>) -> DiscoveryResult {
    DiscoveryResult {
        discovery_id: format!("job_{}", uuid::Uuid::new_v4().simple()),
        provider: PROVIDER.into(),
        approved_roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        source_version: None,
        format_versions: supported_format_versions(),
        artifacts,
        diagnostics: Vec::new(),
        discovered_at: Utc::now().to_rfc3339(),
    }
}

/// Orca's copy of a chat: its conversation, title, archived state, folder,
/// the native identity it ran as, and its place in a multi-agent run.
pub fn copy_chat(store: &Store, chat: &Chat, items: &[Item], reference: &str, links: &Links) -> BundleChat {
    let entries = items
        .iter()
        .map(|item| {
            let at = item.at.clone().unwrap_or_default();
            let (kind, text, tool) = match &item.body {
                Body::User(text) => (BundleEntryKind::UserMessage, Some(text.clone()), None),
                Body::Assistant(text) => (BundleEntryKind::AssistantMessage, Some(text.clone()), None),
                Body::Reasoning(text) => (BundleEntryKind::Reasoning, Some(text.clone()), None),
                Body::Tool { call_id, name, input, output, failed } => (
                    BundleEntryKind::ToolCall,
                    None,
                    Some(BundleTool {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        output: output.clone(),
                        status: match failed {
                            Some(false) => ToolStatus::Completed,
                            Some(true) => ToolStatus::Failed,
                            None => ToolStatus::Unknown,
                        },
                    }),
                ),
            };
            BundleEntry { source_id: item.id.clone(), at, kind, text, tool, attachments: Vec::new() }
        })
        .collect();
    let harness = chat.native.as_ref().map(|native| native.harness).or_else(|| store::harness_of(&chat.agent)).unwrap_or("other");
    let lead = links
        .chats
        .get(&chat.id)
        .or_else(|| chat.native.as_ref().and_then(|native| links.sessions.get(&native.key())));
    let lead_chat = lead.and_then(|lead| store.chats.iter().find(|candidate| &candidate.id == lead));
    let agent_role = if lead.is_some() {
        Some(AgentRole::Worker)
    } else if links.coordinators.contains(&chat.id) {
        Some(AgentRole::Orchestrator)
    } else {
        None
    };
    BundleChat {
        key: chat.id.clone(),
        workspace_key: None,
        cwd: folder(store, chat).map(|path| path.to_string_lossy().into_owned()),
        title: chat.title.clone(),
        title_source: TitleSource::Tool,
        created_at: chat.created_at.clone(),
        updated_at: chat.updated_at.clone(),
        archived: is_archived(store, chat),
        pinned: false,
        harness: harness.to_owned(),
        model: chat.model.clone(),
        effort: None,
        native_ref: chat.native.as_ref().map(|native| NativeRef {
            harness: native.harness.to_owned(),
            session_id: native.id.clone(),
            config_root: None,
            transcript_path: None,
        }),
        parent_chat_key: lead.cloned(),
        parent_native_key: lead_chat.and_then(|lead| lead.native.as_ref()).map(NativeSession::key),
        agent_role,
        entries_source: EntriesSource::ToolCopy,
        entries,
        pull_requests: Vec::new(),
        legacy_native_id: None,
        legacy_content_hash: None,
        source_ref: reference.to_owned(),
        source_label: format!("Orca/{}", chat.title.as_deref().unwrap_or(&chat.id)),
    }
}

fn unsupported_candidate(artifact: &DiscoveredArtifact, discovery: &DiscoveryResult, reason: &str) -> ImportCandidate {
    let source = source_now(
        PROVIDER,
        ADAPTER_VERSION,
        Path::new(&artifact.canonical_source_ref),
        discovery.source_version.clone(),
        discovery.format_versions.get(STORE_GATE).cloned(),
    );
    ImportCandidate {
        candidate_id: deterministic_identity(
            PROVIDER,
            &source.canonical_source_ref,
            Some("unsupported"),
            &content_hash(&Value::String(reason.to_owned())),
            &CandidateKind::Unsupported,
        ),
        source,
        source_native_id: None,
        kind: CandidateKind::Unsupported,
        title: artifact.title.clone().unwrap_or_else(|| artifact.source_label.clone()),
        created_at: artifact.started_at.clone(),
        updated_at: artifact.modified_at.clone(),
        project_hint: None,
        content_hash: content_hash(&Value::Null),
        stability: Stability::Unavailable,
        confidence_bps: 0,
        selected_by_default: false,
        // Not safely representable: the default.
        redaction_summary: RedactionSummary::default(),
        diagnostics: vec![ImportDiagnostic {
            code: "artifact_preview_failed".into(),
            severity: "warning".into(),
            classification: SourceClassification::UnsupportedPrivate,
            message: reason.into(),
            recovery: None,
            source_label: Some(artifact.source_label.clone()),
        }],
        normalized_payload: Value::Object(Map::new()),
    }
}

/// The chat database a reference names, if it is under an approved root.
fn approved_store(approved: &[PathBuf], store: &Path) -> Result<PathBuf, String> {
    let real = store.canonicalize().map_err(|error| format!("Orca's chats are unavailable: {error}"))?;
    let named = real.file_name().is_some_and(|name| name == store::CHAT_DB);
    if named && approved.iter().filter_map(|root| root.canonicalize().ok()).any(|root| real.starts_with(root)) {
        Ok(real)
    } else {
        Err("That Orca database is outside the approved folders".into())
    }
}

/// Every selected chat's copy, read into one bundle and lowered to
/// candidates. Each data folder is read once.
fn preview_copies(approved: &[PathBuf], artifacts: &[&DiscoveredArtifact], discovery: &DiscoveryResult) -> Result<Vec<ImportCandidate>, BridgeError> {
    let mut out = Vec::new();
    if artifacts.is_empty() {
        return Ok(out);
    }
    let mut bundle = TransferBundle::new(BundleSource {
        tool: PROVIDER.into(),
        tool_version: discovery.source_version.clone(),
        schema_version: Some(STORE_GATE.into()),
        adapter_version: ADAPTER_VERSION.into(),
        roots: discovery.approved_roots.clone(),
        detected_at: discovery.discovered_at.clone(),
    });
    let mut stores: HashMap<PathBuf, Result<(Connection, Store, Links), String>> = HashMap::new();
    for artifact in artifacts {
        let outcome = parse_source_ref(&artifact.canonical_source_ref)
            .ok_or_else(|| "Not an Orca chat reference".to_owned())
            .and_then(|(path, chat_id)| {
                let path = approved_store(approved, &path)?;
                let entry = stores.entry(path.clone()).or_insert_with(|| {
                    let base = path.parent().ok_or_else(|| "Orca's data folder is unavailable".to_owned())?;
                    let (store, _) = read(&Paths::at(base))?;
                    let conn = crate::import_sqlite::open(&path).map_err(|error| format!("Bridge could not open Orca's chats: {error}"))?;
                    let links = links(&store);
                    Ok((conn, store, links))
                });
                let (conn, store, links) = entry.as_ref().map_err(Clone::clone)?;
                let chat = chats(store).find(|chat| chat.id == chat_id).ok_or_else(|| "Orca no longer has this chat".to_owned())?;
                let items = store::items(conn, &chat.id)?;
                Ok(copy_chat(store, chat, &items, &artifact.canonical_source_ref, links))
            });
        match outcome {
            Ok(chat) if chat.entries.is_empty() => out.push(unsupported_candidate(artifact, discovery, "1 chat: no messages Bridge can show")),
            Ok(chat) => bundle.chats.push(chat),
            Err(reason) => out.push(unsupported_candidate(artifact, discovery, &reason)),
        }
    }
    let lowered = crate::import_bundle::lower(&crate::import_bundle::resolve(vec![bundle])?);
    out.extend(lowered.candidates);
    Ok(out)
}

impl ExternalHarnessImporter for OrcaImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!("Orca importer cannot discover provider '{}'", request.provider)));
        }
        let mut artifacts = Vec::new();
        let mut roots = Vec::new();
        for root in &request.approved_roots {
            let paths = Paths::at(Path::new(root));
            let (store, path) = read(&paths).map_err(BridgeError::Invalid)?;
            artifacts.extend(chats(&store).map(|chat| copy_artifact(&path, &store, chat)));
            roots.push(paths.base);
        }
        Ok(job_discovery(&roots, artifacts))
    }

    fn preview(&self, discovery: &DiscoveryResult, selection: &DiscoverySelection) -> Result<Vec<ImportCandidate>, BridgeError> {
        if discovery.provider != PROVIDER {
            return Err(BridgeError::Invalid("Discovery result belongs to another provider".into()));
        }
        let gate = validate_schema_version(
            &SchemaGate { format: STORE_GATE.into(), allowed_versions: vec![STORE_GATE.into()] },
            discovery.format_versions.get(STORE_GATE).map(String::as_str),
        );
        if !gate.accepted {
            return Err(BridgeError::Invalid(gate.diagnostics[0].message.clone()));
        }
        let approved: Vec<PathBuf> = discovery.approved_roots.iter().map(PathBuf::from).collect();
        let selected: HashSet<&str> = selection.artifact_ids.iter().map(String::as_str).collect();
        let artifacts: Vec<&DiscoveredArtifact> = discovery
            .artifacts
            .iter()
            .filter(|artifact| selected.contains(artifact.artifact_id.as_str()) && artifact.kind == CandidateKind::Conversation)
            .collect();
        preview_copies(&approved, &artifacts, discovery)
    }

    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult {
        if candidate.source.provider != PROVIDER {
            return ValidationResult {
                accepted: false,
                diagnostics: vec![ImportDiagnostic {
                    code: "provider_mismatch".into(),
                    severity: "error".into(),
                    classification: SourceClassification::UnsupportedPrivate,
                    message: "Candidate does not belong to Orca".into(),
                    recovery: None,
                    source_label: None,
                }],
            };
        }
        if matches!(candidate.stability, Stability::VersionGated) {
            return validate_schema_version(schema_gate, candidate.source.schema_version.as_deref());
        }
        ValidationResult { accepted: candidate.redaction_summary.safely_representable, diagnostics: candidate.diagnostics.clone() }
    }

    fn normalize(&self, candidate: ImportCandidate) -> Result<NormalizedImportCandidate, BridgeError> {
        if candidate.source.provider != PROVIDER || !candidate.redaction_summary.safely_representable {
            return Err(BridgeError::Invalid("Orca candidate is not safe to normalize".into()));
        }
        let deterministic_bridge_id = deterministic_identity(
            PROVIDER,
            &candidate.source.canonical_source_ref,
            candidate.source_native_id.as_deref(),
            &candidate.content_hash,
            &candidate.kind,
        );
        Ok(NormalizedImportCandidate { candidate, deterministic_bridge_id })
    }

    fn fixture_manifest(&self) -> FixtureManifest {
        FixtureManifest {
            provider: PROVIDER.into(),
            adapter_version: ADAPTER_VERSION.into(),
            formats: vec![FixtureFormat {
                source: "Orca data folder (agent-session-domain.db: agent_session_records, journal_items; profiles/*/profile-state.db: repos, worktreeMeta; orchestration.db: runs, dispatch_contexts)".into(),
                classification: SourceClassification::VersionGatedPrivate,
                versions: vec![STORE_GATE.into()],
            }],
        }
    }
}

/// What applying Orca's titles, archived state and run links changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    pub titled: usize,
    pub archived: usize,
    pub linked: usize,
}

/// The Bridge chat a native session became, if any import made one.
fn session_for(transaction: &rusqlite::Transaction<'_>, native_key: &str) -> Result<Option<String>, BridgeError> {
    transaction
        .query_row("SELECT session_id FROM import_native_sessions WHERE native_key=?1", params![native_key], |row| row.get(0))
        .optional()
        .or_else(|error| match error {
            // No import has run yet: the table does not exist.
            rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("no such table") => Ok(None),
            other => Err(other),
        })
        .map_err(BridgeError::from)
}

/// Give chats Orca's title (unless renamed in Bridge since) and archived
/// state, and put each run's workers under the chat that led it. Never
/// unarchives, and never moves a chat that already has a parent.
pub fn overlay(db: &Connection, store: &Store) -> Result<Overlay, BridgeError> {
    let mut out = Overlay::default();
    let links = links(store);
    let transaction = db.unchecked_transaction()?;
    let by_id: HashMap<&str, &Chat> = chats(store).map(|chat| (chat.id.as_str(), chat)).collect();
    for chat in by_id.values() {
        let Some(native) = &chat.native else { continue };
        let Some(session) = session_for(&transaction, &native.key())? else { continue };
        if let Some(title) = chat.title.as_deref().filter(|title| !title.is_empty()) {
            out.titled += transaction.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2 AND kind='imported' AND title_source='imported' AND COALESCE(title,'') <> ?1",
                params![title, session],
            )?;
        }
        if is_archived(store, chat) {
            out.archived += transaction.execute(
                "UPDATE sessions SET archived_at=?1 WHERE id=?2 AND kind='imported' AND archived_at IS NULL",
                params![Utc::now().to_rfc3339(), session],
            )?;
        }
    }
    let workers = links
        .chats
        .iter()
        .filter_map(|(worker, lead)| by_id.get(worker.as_str()).and_then(|chat| chat.native.as_ref()).map(|native| (native.key(), lead)))
        .chain(links.sessions.iter().map(|(key, lead)| (key.clone(), lead)));
    for (worker_key, lead) in workers {
        let Some(lead_key) = by_id.get(lead.as_str()).and_then(|chat| chat.native.as_ref()).map(NativeSession::key) else { continue };
        let (Some(worker), Some(parent)) = (session_for(&transaction, &worker_key)?, session_for(&transaction, &lead_key)?) else { continue };
        if worker == parent {
            continue;
        }
        let depth: i64 = transaction.query_row("SELECT COALESCE(depth,0) FROM sessions WHERE id=?1", params![parent], |row| row.get(0))?;
        let moved = transaction.execute(
            "UPDATE sessions SET parent_session_id=?1, depth=?2 WHERE id=?3 AND kind='imported' AND parent_session_id IS NULL",
            params![parent, depth + 1, worker],
        )?;
        if moved > 0 {
            // The worker's own subagents move down with it.
            transaction.execute(
                "WITH RECURSIVE below(id, depth) AS (
                    SELECT id, ?2 FROM sessions WHERE id=?1
                    UNION ALL SELECT sessions.id, below.depth + 1 FROM sessions JOIN below ON sessions.parent_session_id = below.id
                 )
                 UPDATE sessions SET depth = (SELECT depth FROM below WHERE below.id = sessions.id)
                 WHERE id IN (SELECT id FROM below) AND id <> ?1",
                params![worker, depth + 1],
            )?;
            out.linked += moved;
        }
    }
    transaction.commit()?;
    Ok(out)
}

#[cfg(test)]
mod fuzz;

#[cfg(test)]
#[path = "orca_import/tests.rs"]
mod tests;

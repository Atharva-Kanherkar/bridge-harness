//! T3 Code into Bridge (#1058): its threads as chats, its worktrees as Bridge
//! workspaces adopted in place ([`crate::import_worktrees`]).
//!
//! T3 Code runs Claude Code, Codex and OpenCode. Each thread records the
//! session it ran as ([`store::NativeSession`]). When that session's file is
//! still in Bridge's folders for the agent, the chat is read by the agent's
//! own reader, so it is the same chat the agent's import makes. When it is
//! gone (Claude Code deletes sessions after 30 days by default), the chat is
//! T3 Code's own copy of the messages, keeping the native identity.
//!
//! Built from T3 Code's source (`pingdotgg/t3code@ccc1feb15`, migration
//! 102), its installed CLI run headless in a sandbox, and schema fixtures
//! rebuilt with its own migrations (`testing/fixtures/import/t3code/`). Only
//! the database is opened, read-only, and only named columns of
//! [`store::ALLOWED_TABLES`]; never `secrets/`, `settings.json` or an
//! `auth_*` table.

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
    AgentRole, BundleChat, BundleEntry, BundleEntryKind, BundleSource, EntriesSource, NativeRef, TitleSource, TransferBundle,
};
use crate::import_worktrees::Candidate;
use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use store::{Message, NativeSession, Role, Store, Thread};

/// The import id the Import page and the job use.
pub const PROVIDER: &str = "t3code";
pub const ADAPTER_VERSION: &str = "1";
pub const STORE_GATE: &str = "t3code-sqlite";

#[derive(Debug, Default, Clone, Copy)]
pub struct T3CodeImporter;

/// Where T3 Code keeps things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$T3CODE_HOME`, else `~/.t3`.
    pub base: PathBuf,
    /// `userdata/statev2.sqlite`: the database since the v2 cutover.
    pub store_v2: PathBuf,
    /// `userdata/state.sqlite`: the database before it.
    pub store_v1: PathBuf,
}

impl Paths {
    pub fn at(base: &Path) -> Self {
        let userdata = base.join("userdata");
        Self { base: base.to_path_buf(), store_v2: userdata.join("statev2.sqlite"), store_v1: userdata.join("state.sqlite") }
    }

    /// For a home folder, honouring `T3CODE_HOME` as T3 Code does.
    pub fn for_home(home: &Path, t3code_home: Option<&Path>) -> Self {
        Self::at(&t3code_home.map_or_else(|| home.join(".t3"), Path::to_path_buf))
    }

    /// This machine's, from the environment.
    pub fn from_env(home: &Path) -> Self {
        let t3code_home = std::env::var_os("T3CODE_HOME").filter(|value| !value.is_empty()).map(PathBuf::from);
        Self::for_home(home, t3code_home.as_deref())
    }

    /// The one database to read: the v2 file when there is one (it holds
    /// everything the old one did), else the old one.
    pub fn store(&self) -> Option<&Path> {
        [&self.store_v2, &self.store_v1].into_iter().find(|path| path.is_file()).map(PathBuf::as_path)
    }

    pub fn installed(&self) -> bool {
        self.store().is_some()
    }
}

/// Read T3 Code's database, or say why not.
pub fn read(paths: &Paths) -> Result<(Store, PathBuf), String> {
    let path = paths.store().ok_or_else(|| "T3 Code was not found on this Mac".to_owned())?;
    let conn = crate::import_sqlite::open(path).map_err(|error| format!("Bridge could not open T3 Code's database: {error}"))?;
    Ok((store::read(&conn)?, path.to_path_buf()))
}

/// A thread that comes over: not deleted, in a project that is not deleted.
pub fn is_live(store: &Store, thread: &Thread) -> bool {
    !thread.deleted && store.projects.get(&thread.project_id).is_some_and(|project| !project.deleted)
}

/// Live threads that are chats of their own (not a subagent's).
pub fn chats(store: &Store) -> impl Iterator<Item = &Thread> {
    store.threads.iter().filter(|thread| is_live(store, thread) && thread.parent.is_none())
}

/// Archived in T3 Code: the thread, or its whole project.
pub fn is_archived(store: &Store, thread: &Thread) -> bool {
    thread.archived || store.projects.get(&thread.project_id).is_some_and(|project| project.archived)
}

/// The folder a thread ran in: its worktree, else its project's folder.
pub fn folder(store: &Store, thread: &Thread) -> Option<PathBuf> {
    thread
        .worktree
        .clone()
        .or_else(|| store.projects.get(&thread.project_id).map(|project| project.root.clone()))
        .filter(|path| path.is_absolute())
}

/// Every worktree a live thread recorded, once each, named for its most
/// recent thread. Left out as archived when all its threads are.
pub fn worktree_candidates(store: &Store) -> Vec<Candidate> {
    let mut by_path: BTreeMap<PathBuf, Vec<&Thread>> = BTreeMap::new();
    for thread in store.threads.iter().filter(|thread| is_live(store, thread) && thread.parent.is_none()) {
        if let Some(path) = thread.worktree.clone().filter(|path| path.is_absolute()) {
            by_path.entry(path).or_default().push(thread);
        }
    }
    by_path
        .into_iter()
        .map(|(path, threads)| {
            let newest = threads
                .iter()
                .max_by(|a, b| (&a.updated_at, &a.id).cmp(&(&b.updated_at, &b.id)))
                .copied()
                .unwrap_or(threads[0]);
            Candidate {
                tool: PROVIDER,
                name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                title: newest.title.clone(),
                archived: threads.iter().all(|thread| is_archived(store, thread)),
                source_ref: format!("t3code:{}", newest.id),
                scripts: store.projects.get(&newest.project_id).and_then(|project| project.scripts.clone()),
                path,
            }
        })
        .collect()
}

/// What detection shows for T3 Code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub chats: u64,
    pub projects: u64,
    pub workspaces: u64,
}

pub fn counts(store: &Store) -> Counts {
    let projects: HashSet<&str> = chats(store).map(|thread| thread.project_id.as_str()).collect();
    Counts {
        chats: chats(store).count() as u64,
        projects: projects.len() as u64,
        workspaces: worktree_candidates(store).iter().filter(|candidate| !candidate.archived).count() as u64,
    }
}

/// How each live thread's chat comes over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatPlan {
    /// Read by the agent's own importer, from these files.
    pub native: BTreeSet<NativeSession>,
    /// T3 Code's own copy, by thread id.
    pub copies: Vec<String>,
}

/// Decide, for every live thread, between its native session (when the
/// file was found) and T3 Code's copy. A subagent of a Claude Code thread
/// read natively comes with its parent's transcript, so it is neither.
pub fn plan_chats(store: &Store, found: &HashSet<NativeSession>) -> ChatPlan {
    let mut plan = ChatPlan::default();
    let by_id: HashMap<&str, &Thread> = store.threads.iter().map(|thread| (thread.id.as_str(), thread)).collect();
    fn native_found<'a>(thread: &'a Thread, found: &HashSet<NativeSession>) -> Option<&'a NativeSession> {
        thread.native.as_ref().filter(|native| found.contains(*native))
    }
    for thread in store.threads.iter().filter(|thread| is_live(store, thread)) {
        if let Some(native) = native_found(thread, found) {
            plan.native.insert(native.clone());
            continue;
        }
        let parent_in_claude = thread
            .parent
            .as_deref()
            .and_then(|parent| by_id.get(parent))
            .and_then(|parent| native_found(parent, found))
            .is_some_and(|native| native.harness == "claude");
        if !parent_in_claude {
            plan.copies.push(thread.id.clone());
        }
    }
    plan
}

/// `<store>#<thread id>`, the source of a T3 Code copy.
pub fn source_ref(store: &Path, thread: &str) -> String {
    format!("{}#{thread}", store.to_string_lossy())
}

pub fn parse_source_ref(source_ref: &str) -> Option<(PathBuf, String)> {
    let (store, thread) = source_ref.rsplit_once('#')?;
    (!store.is_empty() && !thread.is_empty()).then(|| (PathBuf::from(store), thread.to_owned()))
}

fn supported_format_versions() -> BTreeMap<String, String> {
    BTreeMap::from([(STORE_GATE.to_owned(), STORE_GATE.to_owned())])
}

/// The job's work item for one copied thread.
pub fn copy_artifact(store_path: &Path, store: &Store, thread: &Thread) -> DiscoveredArtifact {
    let reference = source_ref(store_path, &thread.id);
    let label = format!("T3 Code/{}", thread.id);
    DiscoveredArtifact {
        artifact_id: deterministic_identity(
            PROVIDER,
            &reference,
            Some(&label),
            thread.updated_at.as_deref().unwrap_or_default(),
            &CandidateKind::Conversation,
        ),
        canonical_source_ref: reference,
        source_label: label,
        kind: CandidateKind::Conversation,
        classification: SourceClassification::VersionGatedPrivate,
        stability: Stability::VersionGated,
        estimated_bytes: 4096,
        modified_at: thread.updated_at.clone(),
        required_schema_gate: Some(STORE_GATE.into()),
        title: thread.title.clone(),
        project_path: folder(store, thread).map(|path| path.to_string_lossy().into_owned()),
        started_at: thread.created_at.clone(),
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

/// T3 Code's copy of a thread as a chat: its messages, its title, archived
/// state and folder, and the native identity it ran as.
pub fn copy_chat(store: &Store, thread: &Thread, messages: &[Message], reference: &str) -> BundleChat {
    let entries = messages
        .iter()
        .map(|message| BundleEntry {
            source_id: message.id.clone(),
            at: message.at.clone().unwrap_or_default(),
            kind: match message.role {
                Role::User => BundleEntryKind::UserMessage,
                Role::Assistant => BundleEntryKind::AssistantMessage,
            },
            text: Some(message.text.clone()),
            tool: None,
            attachments: Vec::new(),
        })
        .collect();
    let harness = thread
        .native
        .as_ref()
        .map(|native| native.harness)
        .or_else(|| thread.driver.as_deref().and_then(store::harness_of))
        .unwrap_or("other");
    BundleChat {
        key: thread.id.clone(),
        workspace_key: None,
        cwd: folder(store, thread).map(|path| path.to_string_lossy().into_owned()),
        title: thread.title.clone(),
        title_source: TitleSource::Tool,
        created_at: thread.created_at.clone(),
        updated_at: thread.updated_at.clone(),
        archived: is_archived(store, thread),
        pinned: thread.pinned,
        harness: harness.to_owned(),
        model: thread.model.clone(),
        effort: None,
        native_ref: thread.native.as_ref().map(|native| NativeRef {
            harness: native.harness.to_owned(),
            session_id: native.id.clone(),
            config_root: None,
            transcript_path: None,
        }),
        parent_chat_key: thread.parent.clone(),
        parent_native_key: None,
        agent_role: thread.parent.as_ref().map(|_| AgentRole::Subagent),
        entries_source: EntriesSource::ToolCopy,
        entries,
        pull_requests: Vec::new(),
        legacy_native_id: None,
        legacy_content_hash: None,
        source_ref: reference.to_owned(),
        source_label: format!("T3 Code/{}", thread.title.as_deref().unwrap_or(&thread.id)),
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
        redaction_summary: RedactionSummary { safely_representable: false, ..RedactionSummary::default() },
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

/// The store a reference names, if it is under an approved root.
fn approved_store(approved: &[PathBuf], store: &Path) -> Result<PathBuf, String> {
    let real = store.canonicalize().map_err(|error| format!("T3 Code's database is unavailable: {error}"))?;
    if approved.iter().filter_map(|root| root.canonicalize().ok()).any(|root| real.starts_with(root)) {
        Ok(real)
    } else {
        Err("That T3 Code database is outside the approved folders".into())
    }
}

/// Every selected thread's copy, read into one bundle (subagents under their
/// parents) and lowered to candidates. Each database is read once.
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
    let mut stores: HashMap<PathBuf, Result<(Connection, Store), String>> = HashMap::new();
    for artifact in artifacts {
        let read = parse_source_ref(&artifact.canonical_source_ref)
            .ok_or_else(|| "Not a T3 Code thread reference".to_owned())
            .and_then(|(path, thread_id)| {
                let path = approved_store(approved, &path)?;
                let entry = stores.entry(path.clone()).or_insert_with(|| {
                    let conn = crate::import_sqlite::open(&path).map_err(|error| format!("Bridge could not open T3 Code's database: {error}"))?;
                    let store = store::read(&conn)?;
                    Ok((conn, store))
                });
                let (conn, store) = entry.as_ref().map_err(Clone::clone)?;
                let thread = store
                    .threads
                    .iter()
                    .find(|thread| thread.id == thread_id && is_live(store, thread))
                    .ok_or_else(|| "T3 Code no longer has this thread".to_owned())?;
                let messages = store::messages(conn, store.era, thread)?;
                // A subagent links to its parent by key when the parent is in
                // this batch, else by the parent's native session (imported by
                // its agent, or in an earlier batch), else stands alone.
                let mut chat = copy_chat(store, thread, &messages, &artifact.canonical_source_ref);
                chat.parent_native_key = thread
                    .parent
                    .as_deref()
                    .and_then(|parent| store.threads.iter().find(|candidate| candidate.id == parent))
                    .and_then(|parent| parent.native.as_ref())
                    .map(NativeSession::key);
                Ok(chat)
            });
        match read {
            Ok(chat) if chat.entries.is_empty() => out.push(unsupported_candidate(artifact, discovery, "1 chat: no messages Bridge can show")),
            Ok(chat) => bundle.chats.push(chat),
            Err(reason) => out.push(unsupported_candidate(artifact, discovery, &reason)),
        }
    }
    let lowered = crate::import_bundle::lower(&crate::import_bundle::resolve(vec![bundle])?);
    out.extend(lowered.candidates);
    Ok(out)
}

impl ExternalHarnessImporter for T3CodeImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!("T3 Code importer cannot discover provider '{}'", request.provider)));
        }
        let mut artifacts = Vec::new();
        let mut roots = Vec::new();
        for root in &request.approved_roots {
            let paths = Paths::at(Path::new(root));
            let (store, path) = read(&paths).map_err(BridgeError::Invalid)?;
            artifacts.extend(chats(&store).map(|thread| copy_artifact(&path, &store, thread)));
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
                    message: "Candidate does not belong to T3 Code".into(),
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
            return Err(BridgeError::Invalid("T3 Code candidate is not safe to normalize".into()));
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
                source: "T3 Code database (statev2.sqlite or state.sqlite: projection_projects, projection_threads, orchestration_v2_projection_threads, provider threads and messages)".into(),
                classification: SourceClassification::VersionGatedPrivate,
                versions: vec![STORE_GATE.into()],
            }],
        }
    }
}

/// What applying T3 Code's titles and archived state changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    pub titled: usize,
    pub archived: usize,
}

/// Give chats that came over through their agent's import T3 Code's title
/// (unless renamed in Bridge since) and archived state. Never unarchives.
pub fn overlay(db: &Connection, store: &Store) -> Result<Overlay, BridgeError> {
    let mut out = Overlay::default();
    let transaction = db.unchecked_transaction()?;
    for thread in chats(store) {
        let Some(native) = &thread.native else { continue };
        let session: Option<String> = transaction
            .query_row("SELECT session_id FROM import_native_sessions WHERE native_key=?1", params![native.key()], |row| row.get(0))
            .optional()
            .or_else(|error| match error {
                // No import has run yet: the table does not exist.
                rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("no such table") => Ok(None),
                other => Err(other),
            })?;
        let Some(session) = session else { continue };
        if let Some(title) = thread.title.as_deref().map(str::trim).filter(|title| !title.is_empty()) {
            out.titled += transaction.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2 AND kind='imported' AND title_source='imported' AND COALESCE(title,'') <> ?1",
                params![title, session],
            )?;
        }
        if is_archived(store, thread) {
            out.archived += transaction.execute(
                "UPDATE sessions SET archived_at=?1 WHERE id=?2 AND kind='imported' AND archived_at IS NULL",
                params![Utc::now().to_rfc3339(), session],
            )?;
        }
    }
    transaction.commit()?;
    Ok(out)
}

#[cfg(test)]
mod fuzz;

#[cfg(test)]
#[path = "t3code_import/tests.rs"]
mod tests;
